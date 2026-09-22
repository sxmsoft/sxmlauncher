//! Dependency resolution.
//!
//! Given "I want Sodium and Iris for 1.20.1 Fabric", produce the complete,
//! verified file list — including required dependencies, detected conflicts and
//! files that must be removed. Resolution is a bounded BFS over the dependency
//! graph with a visited set, so a cyclic or self-referential mod cannot hang the
//! launcher.
//!
//! Resolution is intentionally *pure with respect to I/O ordering*: it only
//! talks to the registries, never to the instance folder, so the resulting plan
//! can be reviewed (and unit tested) before anything touches disk.

use std::collections::{HashSet, VecDeque};

use crate::error::{AppError, AppResult};
use crate::models::instance::{LoaderKind, ModLoader};
use crate::models::modpack::{
    ModConflict, ModInclusionReason, ModSource, ModVersion, PackDependencyKind, PackTarget,
    ResolvedMod, ResolvedPackPlan,
};
use crate::mods::curseforge::CurseForgeClient;
use crate::mods::modrinth::ModrinthClient;

/// Safety valve: a plan never grows past this many mods.
pub const MAX_NODES: usize = 400;
/// How deep we follow `required` dependencies.
pub const MAX_DEPTH: usize = 12;

/// A mod the user asked for.
#[derive(Debug, Clone)]
pub struct ModRequest {
    pub source: ModSource,
    pub project_id: String,
    /// Pin a specific version; `None` means "latest compatible".
    pub version_id: Option<String>,
    pub required: bool,
}

impl ModRequest {
    pub fn modrinth(project_id: impl Into<String>) -> Self {
        Self {
            source: ModSource::Modrinth,
            project_id: project_id.into(),
            version_id: None,
            required: true,
        }
    }

    pub fn curseforge(mod_id: u32) -> Self {
        Self {
            source: ModSource::CurseForge,
            project_id: mod_id.to_string(),
            version_id: None,
            required: true,
        }
    }
}

/// Registries the resolver can query.
#[derive(Clone)]
pub struct ModRegistry {
    pub modrinth: ModrinthClient,
    pub curseforge: CurseForgeClient,
}

impl ModRegistry {
    pub fn new(modrinth: ModrinthClient, curseforge: CurseForgeClient) -> Self {
        Self {
            modrinth,
            curseforge,
        }
    }

    /// Resolve a set of requests into a complete install plan.
    pub async fn resolve(
        &self,
        requests: &[ModRequest],
        target: &PackTarget,
    ) -> AppResult<ResolvedPackPlan> {
        let mut queue: VecDeque<(ModRequest, usize, ModInclusionReason)> = requests
            .iter()
            .cloned()
            .map(|request| {
                let reason = if request.required {
                    ModInclusionReason::Requested
                } else {
                    ModInclusionReason::OptionalDependency
                };
                (request, 0usize, reason)
            })
            .collect();

        let mut visited: HashSet<String> = HashSet::new();
        // file name (lowercased) -> (project id, title), to spot two mods that
        // provide the same file and would overwrite each other.
        let mut occupied: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        let mut files: Vec<ResolvedMod> = Vec::new();
        let mut conflicts: Vec<ModConflict> = Vec::new();

        while let Some((request, depth, reason)) = queue.pop_front() {
            if files.len() >= MAX_NODES {
                return Err(AppError::ModResolution(format!(
                    "this selection pulls in more than {MAX_NODES} mods — narrow it down"
                )));
            }
            let visit_key = format!("{}:{}", request.source.as_str(), request.project_id);
            if !visited.insert(visit_key.clone()) {
                continue;
            }

            let version = match self.resolve_version(&request, target).await {
                Ok(version) => version,
                // An unresolvable *optional* dependency must not fail the plan.
                Err(_) if !request.required => continue,
                Err(err) => return Err(err),
            };

            let destination = format!("mods/{}", version.file_name);
            let normalized = destination.to_lowercase();
            if let Some((other_project, other_title)) = occupied.get(&normalized) {
                if other_project != &version.project_id {
                    conflicts.push(ModConflict {
                        project_id: version.project_id.clone(),
                        title: version.name.clone(),
                        with_project_id: other_project.clone(),
                        with_title: other_title.clone(),
                        reason: format!(
                            "both mods provide {}; only one can be installed",
                            version.file_name
                        ),
                    });
                }
            } else {
                occupied.insert(
                    normalized,
                    (version.project_id.clone(), version.name.clone()),
                );
            }

            if let Some(dependency_conflict) = self.detect_hard_conflict(&version, &files) {
                conflicts.push(dependency_conflict);
            }

            // Follow dependencies before we drop the version.
            if depth < MAX_DEPTH {
                for dependency in &version.dependencies {
                    let Some(project_id) = dependency.project_id.clone() else {
                        continue;
                    };
                    match dependency.kind {
                        PackDependencyKind::Required => {
                            if !visited.contains(&format!("modrinth:{project_id}")) {
                                queue.push_back((
                                    ModRequest {
                                        source: ModSource::Modrinth,
                                        project_id,
                                        version_id: dependency.version_id.clone(),
                                        required: true,
                                    },
                                    depth + 1,
                                    ModInclusionReason::RequiredDependency,
                                ));
                            }
                        }
                        PackDependencyKind::Incompatible => {
                            conflicts.push(ModConflict {
                                project_id: project_id.clone(),
                                title: dependency
                                    .file_name
                                    .clone()
                                    .unwrap_or_else(|| project_id.clone()),
                                with_project_id: version.project_id.clone(),
                                with_title: version.name.clone(),
                                reason: "declared incompatible with another selected mod".into(),
                            });
                        }
                        // Optional deps stay untouched: installing them silently
                        // would upgrade a "nice to have" into a surprise.
                        PackDependencyKind::Optional | PackDependencyKind::Embedded => {}
                    }
                }
            }

            files.push(ResolvedMod {
                project_id: version.project_id.clone(),
                version_id: version.id.clone(),
                title: version.name.clone(),
                file_name: version.file_name.clone(),
                url: version.download_url.clone(),
                sha1: version.hashes.sha1.clone(),
                size: version.file_size,
                destination,
                source: version.source,
                required: request.required,
                reason,
            });
        }

        Ok(ResolvedPackPlan {
            java_major: crate::mods::java_runtime::required_major_for(
                &target.game_version,
                target.loader.kind,
            ),
            total_bytes: files.iter().map(|file| file.size).sum(),
            target: target.clone(),
            files,
            removals: Vec::new(),
            conflicts,
        })
    }

    /// Fetch the pinned version, or the newest compatible one.
    async fn resolve_version(
        &self,
        request: &ModRequest,
        target: &PackTarget,
    ) -> AppResult<ModVersion> {
        match request.source {
            ModSource::Modrinth => {
                if let Some(version_id) = &request.version_id {
                    let version = self.modrinth.version(version_id).await?;
                    if version.download_url.is_empty() {
                        return Err(AppError::ModResolution(format!(
                            "{} has no downloadable file for this version",
                            version.name
                        )));
                    }
                    return Ok(version);
                }
                let loader = loader_name(&target.loader);
                self.modrinth
                    .latest_compatible(&request.project_id, &target.game_version, loader)
                    .await?
                    .ok_or_else(|| {
                        AppError::ModResolution(format!(
                            "{} has no build for Minecraft {} ({})",
                            request.project_id, target.game_version, loader.unwrap_or("vanilla")
                        ))
                    })
            }
            ModSource::CurseForge => {
                let mod_id: u32 = request.project_id.parse().map_err(|_| {
                    AppError::ModResolution(format!(
                        "`{}` is not a CurseForge mod id",
                        request.project_id
                    ))
                })?;
                let files = self
                    .curseforge
                    .files(mod_id, Some(&target.game_version))
                    .await?;
                let file = files
                    .into_iter()
                    .find(|file| {
                        file.is_downloadable()
                            && loader_name(&target.loader)
                                .is_none_or(|loader| file.supports_loader(loader))
                    })
                    .ok_or_else(|| {
                        AppError::ModResolution(
                            "that CurseForge project has no file the API will let us download \
                             (the author may have disabled third-party downloads)"
                                .to_string(),
                        )
                    })?;
                Ok(self.curseforge.file_to_version(&file))
            }
        }
    }

    /// Detect a mod that declares an already-selected mod incompatible.
    fn detect_hard_conflict(
        &self,
        version: &ModVersion,
        installed: &[ResolvedMod],
    ) -> Option<ModConflict> {
        for dependency in &version.dependencies {
            if dependency.kind != PackDependencyKind::Incompatible {
                continue;
            }
            let Some(project_id) = &dependency.project_id else {
                continue;
            };
            if let Some(existing) = installed
                .iter()
                .find(|file| &file.project_id == project_id || &file.version_id == project_id)
            {
                return Some(ModConflict {
                    project_id: version.project_id.clone(),
                    title: version.name.clone(),
                    with_project_id: existing.project_id.clone(),
                    with_title: existing.title.clone(),
                    reason: "these two mods cannot be installed together".to_string(),
                });
            }
        }
        None
    }
}

/// Loader name used in registry queries.
pub fn loader_name(loader: &ModLoader) -> Option<&'static str> {
    match loader.kind {
        LoaderKind::Fabric => Some("fabric"),
        LoaderKind::Quilt => Some("quilt"),
        LoaderKind::Forge => Some("forge"),
        LoaderKind::NeoForge => Some("neoforge"),
        LoaderKind::Vanilla => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PackTarget {
        PackTarget {
            game_version: "1.20.1".into(),
            loader: ModLoader::new(LoaderKind::Fabric, "0.15.11"),
        }
    }

    #[test]
    fn loader_names_match_registry_expectations() {
        assert_eq!(loader_name(&ModLoader::new(LoaderKind::Fabric, "1")), Some("fabric"));
        assert_eq!(
            loader_name(&ModLoader::new(LoaderKind::NeoForge, "1")),
            Some("neoforge")
        );
        assert_eq!(loader_name(&ModLoader::vanilla()), None);
    }

    #[test]
    fn registry_is_cloneable_and_shares_clients() {
        let registry = ModRegistry::new(
            ModrinthClient::new(reqwest::Client::new(), None),
            CurseForgeClient::new(reqwest::Client::new(), None),
        );
        let clone = registry.clone();
        assert!(!clone.curseforge.is_configured());
        assert_eq!(target().game_version, "1.20.1");
    }

    #[test]
    fn conflicting_projects_are_reported_against_installed_files() {
        let registry = ModRegistry::new(
            ModrinthClient::new(reqwest::Client::new(), None),
            CurseForgeClient::new(reqwest::Client::new(), None),
        );
        let installed = vec![ResolvedMod {
            project_id: "sodium".into(),
            version_id: "v1".into(),
            title: "Sodium".into(),
            file_name: "sodium.jar".into(),
            url: "https://cdn".into(),
            sha1: None,
            size: 1,
            destination: "mods/sodium.jar".into(),
            source: ModSource::Modrinth,
            required: true,
            reason: ModInclusionReason::Requested,
        }];

        let version = ModVersion {
            id: "v2".into(),
            project_id: "optifine".into(),
            name: "OptiFine".into(),
            version_number: "1.0".into(),
            version_type: "release".into(),
            source: ModSource::Modrinth,
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            file_name: "optifine.jar".into(),
            download_url: "https://cdn/optifine.jar".into(),
            hashes: Default::default(),
            file_size: 1,
            published_at: None,
            dependencies: vec![crate::models::modpack::ModDependency {
                kind: PackDependencyKind::Incompatible,
                project_id: Some("sodium".into()),
                version_id: None,
                file_name: None,
            }],
        };

        let conflict = registry
            .detect_hard_conflict(&version, &installed)
            .expect("conflict detected");
        assert_eq!(conflict.with_project_id, "sodium");
        assert_eq!(conflict.project_id, "optifine");
    }

    #[test]
    fn requests_default_to_required() {
        assert!(ModRequest::modrinth("abc").required);
        assert_eq!(ModRequest::curseforge(1234).project_id, "1234");
    }
}
