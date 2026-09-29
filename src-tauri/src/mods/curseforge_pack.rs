//! CurseForge zip modpack install (manifest.json + overrides/).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::models::instance::{LoaderKind, ModLoader};
use crate::models::modpack::{
    ModInclusionReason, ModSource, PackTarget, ResolvedMod, ResolvedPackPlan,
};
use crate::models::progress::ProgressSink;
use crate::mods::ModEngine;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfManifest {
    name: Option<String>,
    minecraft: CfMinecraft,
    files: Vec<CfManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CfModLoader>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfModLoader {
    id: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfManifestFile {
    #[serde(rename = "projectID")]
    project_id: u32,
    #[serde(rename = "fileID")]
    file_id: u32,
    #[serde(default = "default_required")]
    required: bool,
}

fn default_required() -> bool {
    true
}

impl ModEngine {
    /// Install a CurseForge pack zip: parse manifest, download every file, extract overrides.
    pub async fn install_curseforge_pack(
        &self,
        archive: PathBuf,
        instance_root: PathBuf,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ResolvedPackPlan> {
        let archive_for_manifest = archive.clone();
        let root_for_overrides = instance_root.clone();
        let (manifest, _copied) = tokio::task::spawn_blocking(move || {
            read_cf_manifest_and_overrides(&archive_for_manifest, &root_for_overrides)
        })
        .await??;

        let target = target_from_cf_manifest(&manifest)?;
        let plan = self.plan_curseforge_manifest(&manifest, &target).await?;
        let label = manifest
            .name
            .clone()
            .unwrap_or_else(|| "CurseForge pack".into());
        self.install_plan(&plan, &instance_root, format!("Installing {label}"), sink)
            .await?;
        Ok(plan)
    }

    async fn plan_curseforge_manifest(
        &self,
        manifest: &CfManifest,
        target: &PackTarget,
    ) -> AppResult<ResolvedPackPlan> {
        let mut files = Vec::new();
        let mut total_bytes = 0u64;

        for entry in &manifest.files {
            if !entry.required {
                continue;
            }
            let file = self.curseforge().file(entry.project_id, entry.file_id).await?;
            let version = self.curseforge().file_to_version(&file);
            if version.download_url.is_empty() {
                return Err(AppError::ModResolution(format!(
                    "CurseForge file {} (project {}) has no download URL — set SXML_CURSEFORGE_API_KEY",
                    entry.file_id, entry.project_id
                )));
            }
            total_bytes = total_bytes.saturating_add(version.file_size);
            files.push(ResolvedMod {
                source: ModSource::CurseForge,
                project_id: entry.project_id.to_string(),
                version_id: entry.file_id.to_string(),
                title: version.name.clone(),
                file_name: version.file_name.clone(),
                url: version.download_url.clone(),
                sha1: version.hashes.sha1.clone(),
                size: version.file_size,
                destination: PathBuf::from("mods")
                    .join(&version.file_name)
                    .to_string_lossy()
                    .into_owned(),
                required: entry.required,
                reason: ModInclusionReason::Requested,
            });
        }

        Ok(ResolvedPackPlan {
            target: target.clone(),
            files,
            removals: Vec::new(),
            conflicts: Vec::new(),
            total_bytes,
            java_major: 0,
        })
    }
}

fn target_from_cf_manifest(manifest: &CfManifest) -> AppResult<PackTarget> {
    let loader_entry = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first());

    let loader = match loader_entry {
        Some(entry) => parse_cf_loader(&entry.id),
        None => ModLoader::vanilla(),
    };

    Ok(PackTarget {
        game_version: manifest.minecraft.version.clone(),
        loader,
    })
}

fn parse_cf_loader(id: &str) -> ModLoader {
    let lower = id.to_ascii_lowercase();
    let (kind, version) = if let Some(rest) = lower.strip_prefix("forge-") {
        (LoaderKind::Forge, rest.to_string())
    } else if let Some(rest) = lower.strip_prefix("neoforge-") {
        (LoaderKind::NeoForge, rest.to_string())
    } else if let Some(rest) = lower.strip_prefix("fabric-") {
        (LoaderKind::Fabric, rest.to_string())
    } else if let Some(rest) = lower.strip_prefix("quilt-") {
        (LoaderKind::Quilt, rest.to_string())
    } else {
        return ModLoader::vanilla();
    };
    ModLoader::new(kind, version)
}

fn read_cf_manifest_and_overrides(
    archive: &Path,
    instance_root: &Path,
) -> AppResult<(CfManifest, usize)> {
    let file = std::fs::File::open(archive).map_err(|err| {
        AppError::ModResolution(format!("cannot open {}: {err}", archive.display()))
    })?;
    let mut zip = zip::ZipArchive::new(file)?;

    let manifest = {
        let mut entry = zip.by_name("manifest.json").map_err(|_| {
            AppError::ModResolution(
                "this CurseForge zip has no manifest.json — is it a modpack archive?".into(),
            )
        })?;
        let mut raw = String::new();
        entry.read_to_string(&mut raw).map_err(|err| {
            AppError::ModResolution(format!("cannot read manifest.json: {err}")
            )
        })?;
        serde_json::from_str::<CfManifest>(&raw)?
    };

    let mut copied = 0usize;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        let relative = if let Some(rest) = name.strip_prefix("overrides/") {
            rest
        } else if let Some(rest) = name.strip_prefix("overrides\\") {
            rest
        } else {
            continue;
        };
        if relative.is_empty() || entry.is_dir() {
            continue;
        }
        let dest = instance_root.join(relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
        out.flush()?;
        copied += 1;
    }

    Ok((manifest, copied))
}
