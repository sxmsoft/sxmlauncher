//! `.mrpack` (Modrinth modpack) support.
//!
//! An `.mrpack` is a zip containing:
//! * `modrinth.index.json` — the machine-readable plan (see [`MrpackManifest`]).
//! * `overrides/` — files copied into the instance root verbatim (configs, packs).
//! * `client-overrides/` — client-only overrides, which is what we want.
//! * `server-overrides/` — ignored by the launcher.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::models::instance::LoaderKind;
use crate::models::modpack::{
    ModInclusionReason, ModSource, ModpackRefSource, MrpackManifest, PackTarget, ResolvedMod,
    ResolvedPackPlan,
};

/// Manifest file name inside the archive.
pub const MANIFEST_ENTRY: &str = "modrinth.index.json";
/// Highest `.mrpack` format version this launcher understands.
pub const SUPPORTED_FORMAT: u32 = 1;

/// Read and validate the manifest from an `.mrpack`.
pub fn read_manifest(archive: &Path) -> AppResult<MrpackManifest> {
    let file = std::fs::File::open(archive)
        .map_err(|err| AppError::ModResolution(format!("cannot open {}: {err}", archive.display())))?;
    let mut zip = zip::ZipArchive::new(file)?;

    let manifest = {
        let mut entry = zip.by_name(MANIFEST_ENTRY).map_err(|_| {
            AppError::ModResolution(format!(
                "{} does not contain {MANIFEST_ENTRY} — is this really an .mrpack?",
                archive
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))
        })?;
        let mut raw = String::new();
        entry.read_to_string(&mut raw).map_err(|err| {
            AppError::ModResolution(format!("cannot read {MANIFEST_ENTRY}: {err}"))
        })?;
        serde_json::from_str::<MrpackManifest>(&raw)?
    };

    if manifest.format_version > SUPPORTED_FORMAT {
        return Err(AppError::Unsupported(format!(
            "this pack uses .mrpack format {} but this launcher supports {}",
            manifest.format_version, SUPPORTED_FORMAT
        )));
    }
    Ok(manifest)
}

/// Minecraft version + loader described by the manifest's `dependencies` map.
pub fn target_from_manifest(manifest: &MrpackManifest) -> AppResult<PackTarget> {
    let game_version = manifest.dependencies.get("minecraft").cloned().ok_or_else(|| {
        AppError::ModResolution("the pack manifest does not declare a Minecraft version".into())
    })?;

    let loader = [
        ("fabric-loader", LoaderKind::Fabric),
        ("quilt-loader", LoaderKind::Quilt),
        ("forge", LoaderKind::Forge),
        ("neoforge", LoaderKind::NeoForge),
    ]
    .iter()
    .find_map(|(key, kind)| {
        manifest
            .dependencies
            .get(*key)
            .map(|version| crate::models::instance::ModLoader::new(*kind, version.clone()))
    })
    .unwrap_or_else(crate::models::instance::ModLoader::vanilla);

    Ok(PackTarget {
        game_version,
        loader,
    })
}

/// Turn a manifest into a downloadable plan.
///
/// `env.client == "unsupported"` entries are skipped: they are server-only mods
/// that would crash the client.
pub fn plan_from_manifest(manifest: &MrpackManifest, pack: &ModpackRefSource) -> AppResult<ResolvedPackPlan> {
    let target = target_from_manifest(manifest)?;
    // A loop (not `map`) because path validation is fallible and a malicious
    // manifest must abort the whole plan, not be silently skipped.
    let mut files: Vec<ResolvedMod> = Vec::with_capacity(manifest.files.len());
    for file in manifest.files.iter().filter(|file| file.supports_client()) {
        {
            let url = file.downloads.first().cloned().unwrap_or_default();
            files.push(ResolvedMod {
                // A manifest file is not tied to a project id we can query; the
                // destination path is the stable identity.
                project_id: format!("{}#{}", pack.project_id, file.path),
                version_id: file.hashes.sha1.clone().unwrap_or_else(|| file.path.clone()),
                title: file
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&file.path)
                    .to_string(),
                file_name: file
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&file.path)
                    .to_string(),
                url,
                sha1: file.hashes.sha1.clone(),
                size: file.file_size,
                destination: normalize_relative_path(&file.path)?,
                source: ModSource::Modrinth,
                required: true,
                reason: ModInclusionReason::FromPackManifest,
            });
        }
    }

    Ok(ResolvedPackPlan {
        total_bytes: files.iter().map(|file| file.size).sum(),
        java_major: crate::mods::java_runtime::required_major_for(
            &target.game_version,
            target.loader.kind,
        ),
        target,
        files,
        removals: Vec::new(),
        conflicts: Vec::new(),
    })
}

/// Synthetic project id used for manifest files (kept stable across installs).
pub fn manifest_file_id(pack_project_id: &str, path: &str) -> String {
    format!("{pack_project_id}#{path}")
}

/// Reject absolute paths and `..` escapes inside a pack manifest.
///
/// A hostile `.mrpack` could otherwise write anywhere on disk.
pub fn normalize_relative_path(path: &str) -> AppResult<String> {
    let cleaned = path.replace('\\', "/");
    let candidate = Path::new(&cleaned);
    if candidate.is_absolute() {
        return Err(AppError::ModResolution(format!(
            "the pack tries to write to an absolute path: {path}"
        )));
    }
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => {
                return Err(AppError::ModResolution(format!(
                    "the pack contains an unsafe path: {path}"
                )))
            }
        }
    }
    Ok(cleaned)
}

/// Extract `overrides/` and `client-overrides/` into an instance root.
pub async fn extract_overrides(archive: PathBuf, instance_root: PathBuf) -> AppResult<usize> {
    tokio::task::spawn_blocking(move || -> AppResult<usize> {
        let file = std::fs::File::open(&archive)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut written = 0usize;

        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            // `client-overrides` wins over `overrides` when both define a file.
            let relative = name
                .strip_prefix("client-overrides/")
                .or_else(|| name.strip_prefix("overrides/"));
            let Some(relative) = relative else {
                continue;
            };
            let safe = normalize_relative_path(relative)?;
            let destination = instance_root.join(&safe);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if destination.exists() && name.starts_with("overrides/") {
                // A client-override already wrote this file.
                continue;
            }
            let mut buffer = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buffer)?;
            std::fs::write(&destination, &buffer)?;
            written += 1;
        }
        Ok(written)
    })
    .await?
}

/// Hash + file name of a jar sitting in an instance's `mods/` folder.
#[derive(Debug, Clone)]
pub struct InstalledJar {
    pub path: PathBuf,
    pub file_name: String,
    pub sha1: String,
    pub size: u64,
}

/// Enumerate installed jars so the resolver can detect what is already there.
pub async fn scan_mods(instance_root: PathBuf) -> AppResult<Vec<InstalledJar>> {
    tokio::task::spawn_blocking(move || -> AppResult<Vec<InstalledJar>> {
        let mods = instance_root.join("mods");
        if !mods.exists() {
            return Ok(Vec::new());
        }
        let mut jars = Vec::new();
        for entry in std::fs::read_dir(&mods)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let disabled = path
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("disabled"))
                .unwrap_or(false);
            let is_jar = !disabled
                && path
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false);
            if !is_jar {
                continue;
            }
            let sha1 = crate::mods::downloader::sha1_file(&path)?;
            jars.push(InstalledJar {
                file_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size: entry.metadata()?.len(),
                sha1,
                path,
            });
        }
        Ok(jars)
    })
    .await?
}

/// Build the `ModpackRefSource` recorded on an instance created from a pack.
pub fn pack_reference(
    project_id: &str,
    version_id: &str,
    name: &str,
    version_number: &str,
    icon_url: Option<String>,
) -> ModpackRefSource {
    ModpackRefSource {
        source: ModSource::Modrinth,
        project_id: project_id.to_string(),
        version_id: version_id.to_string(),
        name: name.to_string(),
        version_number: version_number.to_string(),
        icon_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> MrpackManifest {
        serde_json::from_value(serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "abc",
            "name": "Test Pack",
            "files": [
                {
                    "path": "mods/sodium.jar",
                    "hashes": { "sha1": "aa", "sha512": "bb" },
                    "downloads": ["https://cdn.modrinth.com/data/x/versions/y/sodium.jar"],
                    "fileSize": 100
                },
                {
                    "path": "mods/server-only.jar",
                    "hashes": { "sha1": "cc" },
                    "env": { "client": "unsupported", "server": "required" },
                    "downloads": ["https://cdn.modrinth.com/data/x/versions/z/server-only.jar"],
                    "fileSize": 50
                }
            ],
            "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.11" }
        }))
        .expect("manifest fixture")
    }

    #[test]
    fn target_detects_version_and_loader() {
        let target = target_from_manifest(&manifest()).expect("target");
        assert_eq!(target.game_version, "1.20.1");
        assert_eq!(target.loader.kind, LoaderKind::Fabric);
        assert_eq!(target.loader.version.as_deref(), Some("0.15.11"));
    }

    #[test]
    fn plan_skips_server_only_files() {
        let manifest = manifest();
        let pack = pack_reference("proj", "ver", "Test Pack", "1.0.0", None);
        let plan = plan_from_manifest(&manifest, &pack).expect("plan");

        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].destination, "mods/sodium.jar");
        assert_eq!(plan.files[0].sha1.as_deref(), Some("aa"));
        assert_eq!(plan.total_bytes, 100);
        assert_eq!(plan.java_major, 17);
    }

    #[test]
    fn manifest_without_minecraft_version_is_rejected() {
        let mut manifest = manifest();
        manifest.dependencies.clear();
        assert!(target_from_manifest(&manifest).is_err());
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(normalize_relative_path("mods/ok.jar").is_ok());
        assert!(normalize_relative_path("mods/./ok.jar").is_ok());
        assert!(normalize_relative_path("../../etc/passwd").is_err());
        assert!(normalize_relative_path("/etc/passwd").is_err());
        assert!(normalize_relative_path("C:\\Windows\\system32\\evil.dll").is_err());
    }

    #[test]
    fn hashes_default_when_absent() {
        let file: crate::models::modpack::MrpackFile = serde_json::from_value(serde_json::json!({
            "path": "x.jar",
            "hashes": {}
        }))
        .expect("file");
        // Check the borrow-based accessor *before* moving the hashes out.
        assert!(file.supports_client());
        let hashes: crate::models::modpack::ModHashes = file.hashes;
        assert!(hashes.sha1.is_none());
    }
}
