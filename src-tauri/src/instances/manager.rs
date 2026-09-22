//! Instance lifecycle: create, install, update, duplicate, delete.
//!
//! Persistence is deliberately two-layered:
//! * `<instance>/instance.json` — the **source of truth**, so an instance folder
//!   can be zipped, moved to another machine, or copied by hand and still work.
//! * the SQLite `instances` table — an index for fast listing/sorting plus the
//!   counters (playtime, launch count) that are not part of the config.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::config::{AppPaths, AppSettings};
use crate::error::{AppError, AppResult};
use crate::models::instance::{
    CreateInstanceRequest, Instance, InstanceConfig, InstancePaths, InstanceStatus, JavaSettings,
    LoaderKind, MemorySettings, ModLoader, PathExt, ResolutionSettings, UpdateInstanceRequest,
};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::version::FeatureSet;
use crate::mods::downloader::directory_size;
use crate::mods::ModEngine;
use crate::store::Database;

/// Owns every filesystem mutation an instance undergoes.
pub struct InstanceManager {
    paths: AppPaths,
    db: Database,
    mods: Arc<ModEngine>,
    settings: AppSettings,
}

impl InstanceManager {
    pub fn new(
        paths: AppPaths,
        db: Database,
        mods: Arc<ModEngine>,
        settings: AppSettings,
    ) -> Self {
        Self {
            paths,
            db,
            mods,
            settings,
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn engine(&self) -> &Arc<ModEngine> {
        &self.mods
    }

    /// Per-instance layout helper.
    pub fn layout(&self, id: Uuid) -> InstancePaths {
        InstancePaths::new(self.paths.instances.clone(), id)
    }

    pub fn apply_settings(&mut self, settings: AppSettings) {
        self.settings = settings;
    }

    /// All instances, newest first.
    pub async fn list(&self) -> AppResult<Vec<Instance>> {
        self.db.list_instances()
    }

    pub async fn get(&self, id: Uuid) -> AppResult<Instance> {
        self.db
            .get_instance(id)?
            .ok_or_else(|| AppError::InstanceNotFound(id.to_string()))
    }

    pub async fn exists(&self, id: Uuid) -> bool {
        self.db.get_instance(id).ok().flatten().is_some()
    }

    /// Create the instance directory, write `instance.json`, index it.
    pub async fn create(
        &self,
        request: CreateInstanceRequest,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Instance> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(AppError::Config("an instance needs a name".into()));
        }
        if request.game_version.trim().is_empty() {
            return Err(AppError::Config("pick a Minecraft version".into()));
        }

        let now = Utc::now();
        let id = Uuid::new_v4();
        let layout = self.layout(id);
        layout.root().ensure_dir()?;
        for dir in layout.required_dirs() {
            dir.ensure_dir()?;
        }

        let loader = request.loader.unwrap_or_else(ModLoader::vanilla);
        let config = InstanceConfig {
            id,
            name: name.to_string(),
            description: request.description.unwrap_or_default(),
            icon: request.icon.or_else(|| Some("📦".to_string())),
            game_version: request.game_version.trim().to_string(),
            loader,
            java: JavaSettings {
                auto_download: self.settings.auto_provision_java,
                ..JavaSettings::default()
            },
            memory: request.memory.unwrap_or(self.settings.default_memory),
            resolution: ResolutionSettings::default(),
            game_args: Vec::new(),
            source_pack: None,
            created_at: now,
            updated_at: now,
        };
        self.write_config(&config)?;

        let instance = Instance {
            required_java_major: crate::mods::required_major_for(
                &config.game_version,
                config.loader.kind,
            ),
            config,
            status: InstanceStatus::NotInstalled,
            mod_count: 0,
            last_played_at: None,
            total_playtime_secs: 0,
            launch_count: 0,
            size_bytes: 0,
        };
        self.db.upsert_instance(&instance)?;

        if request.install_now {
            return self.install(instance, sink).await;
        }
        Ok(instance)
    }

    /// Sparse update; only provided fields change.
    pub async fn update(&self, request: UpdateInstanceRequest) -> AppResult<Instance> {
        let mut instance = self.get(request.id).await?;
        let config = &mut instance.config;

        if let Some(name) = request.name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(AppError::Config("an instance needs a name".into()));
            }
            config.name = trimmed.to_string();
        }
        if let Some(description) = request.description {
            config.description = description;
        }
        if let Some(icon) = request.icon {
            config.icon = Some(icon);
        }
        if let Some(version) = request.game_version {
            config.game_version = version;
        }
        if let Some(loader) = request.loader {
            config.loader = loader;
        }
        if let Some(java) = request.java {
            config.java = java;
        }
        if let Some(memory) = request.memory {
            config.memory = memory.sanitized();
        }
        if let Some(resolution) = request.resolution {
            config.resolution = resolution;
        }
        if let Some(args) = request.game_args {
            config.game_args = args;
        }
        config.touch();

        instance.required_java_major =
            crate::mods::required_major_for(&config.game_version, config.loader.kind);

        self.write_config(&instance.config)?;
        self.db.upsert_instance(&instance)?;
        Ok(instance)
    }

    /// Install (or repair) the vanilla profile this instance needs.
    pub async fn install(
        &self,
        instance: Instance,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Instance> {
        let id = instance.config.id;
        self.db.set_instance_status(id, InstanceStatus::Installing)?;

        // One stable job id for the whole install: every report below (Java
        // provisioning, downloads, natives, assets, done) carries it, so the
        // activity panel shows ONE job progressing instead of a new
        // "working…" row per stage that never goes away.
        let job_id = uuid::Uuid::new_v4();
        sink.report(
            ProgressEvent::started(
                JobKind::InstanceInstall,
                format!("Installing {}", instance.config.name),
            )
            .for_job(job_id)
            .stage(JobStage::Resolving)
            .detail(format!(
                "Minecraft {} · {} ({})",
                instance.config.game_version,
                instance.config.loader.kind.as_str(),
                instance.config.resolved_version_id()
            )),
        )
        .await;

        // Provision Java *before* touching game files: an unlaunchable instance
        // is worse than a failed install.
        let required = crate::mods::required_major_for(
            &instance.config.game_version,
            instance.config.loader.kind,
        );
        if instance.config.java.override_path.is_none() {
            let allow_download =
                self.settings.auto_provision_java && instance.config.java.auto_download;
            // `resolve_java` reuses any compatible runtime (system or managed):
            // that is what stops every install from fetching another 200 MB JDK.
            let resolved = self
                .mods
                .resolve_java(required, allow_download, sink.clone())
                .await
                .inspect_err(|err| {
                    // Mirror the failure into our job so it does not linger
                    // as "working…" in the activity panel forever.
                    let failure = ProgressEvent::started(
                        JobKind::InstanceInstall,
                        format!("Installing {} failed", instance.config.name),
                    )
                    .for_job(job_id)
                    .failed(err.to_string());
                    futures::executor::block_on(sink.report(failure));
                })?;

            // A user who explicitly asked for "launcher-managed Java" gets one
            // even when a system JDK would have worked.
            let wants_managed = !self.settings.prefer_system_java
                && allow_download
                && resolved
                    .as_ref()
                    .is_none_or(|runtime| !runtime.is_managed);
            if wants_managed {
                let managed = self.mods.java().install(required, sink.as_ref()).await?;
                sink.report(
                    ProgressEvent::started(
                        JobKind::InstanceInstall,
                        format!("Using {}", managed.describe()),
                    )
                    .for_job(job_id)
                    .stage(JobStage::ProvisioningJava)
                    .detail(managed.path.to_string_lossy().to_string()),
                )
                .await;
            } else if let Some(runtime) = resolved {
                sink.report(
                    ProgressEvent::started(
                        JobKind::InstanceInstall,
                        format!("Using {}", runtime.describe()),
                    )
                    .for_job(job_id)
                    .stage(JobStage::ProvisioningJava)
                    .detail(runtime.path.to_string_lossy().to_string()),
                )
                .await;
            }
        }

        let downloader = self.mods.downloader();
        // Route the downloader's own (correctly finished) job through us:
        // install_version reports under our stable job id, so its completion
        // event closes THE job rather than adding a new row.
        let version_sink: Arc<dyn ProgressSink> =
            Arc::new(ForwardingSink::new(sink.clone(), job_id));
        let installer = crate::instances::installer::Installer::new(
            &self.paths,
            Some(self.db.clone()),
            downloader,
            crate::mods::modrinth::http_client()?,
        );

        // Always install the vanilla parent first. Modded profiles inherit from
        // it; installing only the loader leaves `inheritsFrom` unresolved.
        let vanilla_id = instance.config.game_version.clone();
        let result = installer
            .install_version(&vanilla_id, version_sink.clone())
            .await;

        if let Err(err) = &result {
            sink.report(
                ProgressEvent::started(
                    JobKind::InstanceInstall,
                    format!("Installing {} failed", instance.config.name),
                )
                .for_job(job_id)
                .failed(err.to_string()),
            )
            .await;
            // A failed install must not keep claiming `Installing`: the Play
            // button gates on status, and a stuck status is how the launcher
            // ends up in an endless "preparing" state.
            let _ = self.db.set_instance_status(id, InstanceStatus::NotInstalled);
            return Err(result.err().unwrap());
        }

        // Layer the loader profile (Fabric/Quilt/Forge/NeoForge) on top.
        if instance.config.loader.kind.is_modded() {
            let loader_result = crate::instances::loader::install_loader(
                &self.paths,
                &instance.config.game_version,
                &instance.config.loader,
                downloader,
                &crate::mods::modrinth::http_client()?,
                version_sink.clone(),
            )
            .await;

            if let Err(err) = &loader_result {
                sink.report(
                    ProgressEvent::started(
                        JobKind::InstanceInstall,
                        format!("Installing {} failed", instance.config.name),
                    )
                    .for_job(job_id)
                    .failed(err.to_string()),
                )
                .await;
                let _ = self.db.set_instance_status(id, InstanceStatus::NotInstalled);
                return Err(loader_result.err().unwrap());
            }
        }

        // Refetch the row the installer may have touched, then flip to Ready:
        // status is derived from real files via `refresh`, so the Play button
        // unlocks exactly when the version JSON + client jar exist on disk.
        let instance = self.refresh(id).await?;

        sink.report(
            ProgressEvent::started(
                JobKind::InstanceInstall,
                format!("{} is ready", instance.config.name),
            )
            .for_job(job_id)
            .stage(JobStage::Done)
            .finished(),
        )
        .await;
        Ok(instance)
    }

    /// Recompute the derived status from what is actually on disk.
    pub async fn refresh(&self, id: Uuid) -> AppResult<Instance> {
        let mut instance = self.get(id).await?;
        let layout = self.layout(id);
        let version_id = instance.config.resolved_version_id();

        let client_jar = self
            .paths
            .version_dir(&version_id)
            .join(format!("{version_id}.jar"));
        let version_json = self.paths.version_json(&version_id);

        instance.status = if !layout.root().exists() {
            InstanceStatus::NotInstalled
        } else if version_json.is_file() && client_jar.is_file() {
            InstanceStatus::Ready
        } else {
            InstanceStatus::NotInstalled
        };

        let mods = layout.mods();
        instance.mod_count = if mods.exists() {
            std::fs::read_dir(&mods)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            entry
                                .path()
                                .extension()
                                .map(|ext| ext.eq_ignore_ascii_case("jar"))
                                .unwrap_or(false)
                        })
                        .count() as u32
                })
                .unwrap_or(0)
        } else {
            0
        };

        self.db.upsert_instance(&instance)?;
        Ok(instance)
    }

    /// Copy an instance (config gets a new id; the folder is copied verbatim).
    pub async fn duplicate(&self, id: Uuid, new_name: Option<String>) -> AppResult<Instance> {
        let source = self.get(id).await?;
        let source_root = self.layout(id).root();

        let new_id = Uuid::new_v4();
        let target_root = self.layout(new_id).root();
        target_root.ensure_dir()?;

        let from = source_root.clone();
        let to = target_root.clone();
        tokio::task::spawn_blocking(move || copy_tree(&from, &to)).await??;

        let mut config = source.config.clone();
        config.id = new_id;
        config.name = new_name.unwrap_or_else(|| format!("{} (copy)", source.config.name));
        config.created_at = Utc::now();
        config.updated_at = Utc::now();
        self.write_config(&config)?;

        let mut instance = source.clone();
        instance.config = config;
        instance.status = InstanceStatus::Ready;
        instance.launch_count = 0;
        instance.total_playtime_secs = 0;
        instance.last_played_at = None;
        self.db.upsert_instance(&instance)?;
        Ok(instance)
    }

    /// Delete the index row and (optionally) the files.
    pub async fn delete(&self, id: Uuid, delete_files: bool) -> AppResult<()> {
        // Make sure the instance exists before removing anything.
        let _ = self.get(id).await?;
        if delete_files {
            let root = self.layout(id).root();
            if root.exists() {
                tokio::task::spawn_blocking(move || std::fs::remove_dir_all(root)).await??;
            }
        }
        self.db.delete_instance(id)
    }

    /// Refresh the cached size on disk.
    pub async fn measure(&self, id: Uuid) -> AppResult<u64> {
        let root = self.layout(id).root();
        let size = tokio::task::spawn_blocking(move || directory_size(&root)).await?;
        Ok(size)
    }

    /// Absolute path of the instance folder (for "Open folder").
    pub async fn folder(&self, id: Uuid) -> AppResult<PathBuf> {
        let _ = self.get(id).await?;
        let root = self.layout(id).root();
        root.ensure_dir()?;
        Ok(root)
    }

    /// Read `instance.json`, tolerating a hand-edited file with a mismatched id.
    fn read_config(&self, config_path: PathBuf) -> AppResult<InstanceConfig> {
        let raw = std::fs::read_to_string(&config_path)?;
        let config: InstanceConfig = serde_json::from_str(&raw)?;
        Ok(config)
    }

    /// Atomically write `instance.json`.
    fn write_config(&self, config: &InstanceConfig) -> AppResult<()> {
        let layout = self.layout(config.id);
        layout.root().ensure_dir()?;
        let payload = serde_json::to_string_pretty(config)?;
        crate::config::write_atomic(&layout.config_file(), payload.as_bytes())
    }

    /// Load a config purely from disk (used when importing an instance folder).
    pub fn import_from_disk(&self, folder: &std::path::Path) -> AppResult<InstanceConfig> {
        self.read_config(folder.join("instance.json"))
    }

    /// Import an instance folder dropped into the launcher.
    pub async fn import(&self, folder: PathBuf) -> AppResult<Instance> {
        let mut config = self.import_from_disk(&folder)?;

        // Re-home the folder under the instances root if it came from elsewhere.
        let expected_root = self.layout(config.id).root();
        if folder != expected_root {
            if expected_root.exists() {
                return Err(AppError::Config(format!(
                    "an instance with id {} already exists",
                    config.id
                )));
            }
            let from = folder.clone();
            let to = expected_root.clone();
            tokio::task::spawn_blocking(move || copy_tree(&from, &to)).await??;
        }

        config.touch();
        self.write_config(&config)?;

        let instance = Instance {
            required_java_major: crate::mods::required_major_for(
                &config.game_version,
                config.loader.kind,
            ),
            config,
            status: InstanceStatus::NotInstalled,
            mod_count: 0,
            last_played_at: None,
            total_playtime_secs: 0,
            launch_count: 0,
            size_bytes: 0,
        };
        self.db.upsert_instance(&instance)?;
        self.refresh(instance.config.id).await
    }

    /// Feature flags for argument rule evaluation (custom resolution etc.).
    pub fn feature_set(&self, instance: &Instance) -> FeatureSet {
        FeatureSet::default().with_custom_resolution(!instance.config.resolution.fullscreen)
    }

    /// Snapshot of settings as the manager sees them.
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Recommended defaults for a new instance of a given version/loader.
    pub fn suggest_memory(&self, loader: LoaderKind) -> MemorySettings {
        match loader {
            LoaderKind::Vanilla => MemorySettings {
                min_mb: 1024,
                max_mb: 4096,
            },
            _ => MemorySettings {
                min_mb: 2048,
                max_mb: 6144,
            },
        }
    }
}

/// Recursive copy that preserves the directory shape.
pub fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> AppResult<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if file_type.is_symlink() {
            // Never follow symlinks while copying: a loop would fill the disk.
            continue;
        } else {
            std::fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

/// Re-labels every event from an inner sink with OUR stable job id.
///
/// `install_version` emits its own job (and finishes it correctly); without
/// this wrapper those events appear in the activity panel as a second, third,
/// … row while the outer install job never closes. Forwarding under one id
/// makes the whole install ONE job in the UI.
struct ForwardingSink {
    inner: Arc<dyn ProgressSink>,
    job_id: Uuid,
}

impl ForwardingSink {
    fn new(inner: Arc<dyn ProgressSink>, job_id: Uuid) -> Self {
        Self { inner, job_id }
    }
}

#[async_trait::async_trait]
impl ProgressSink for ForwardingSink {
    async fn report(&self, mut event: ProgressEvent) {
        event.job_id = self.job_id;
        // Keep the outer title so the row does not flicker between names; the
        // inner detail/current-item still flow through untouched.
        self.inner.report(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::progress::NoopProgressSink;

    fn manager(root: &std::path::Path) -> InstanceManager {
        let paths = AppPaths::from_root(root);
        paths.ensure().expect("layout");
        let db = Database::open_in_memory().expect("db");
        let settings = AppSettings::default();
        let engine = Arc::new(ModEngine::new(paths.clone(), db.clone(), &settings).expect("engine"));
        InstanceManager::new(paths, db, engine, settings)
    }

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        let path = std::env::temp_dir().join(format!("sxml-instances-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("temp");
        TempDir(path)
    }

    fn request(name: &str) -> CreateInstanceRequest {
        CreateInstanceRequest {
            name: name.into(),
            description: Some("test".into()),
            game_version: "1.20.1".into(),
            loader: Some(ModLoader::new(LoaderKind::Fabric, "0.15.11")),
            memory: None,
            icon: None,
            install_now: false,
        }
    }

    #[tokio::test]
    async fn create_lays_out_directories_and_persists_config() {
        let temp = tempdir();
        let manager = manager(&temp.0);

        let instance = manager
            .create(request("Fabulously Optimized"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let layout = manager.layout(instance.config.id);
        assert!(layout.config_file().is_file());
        assert!(layout.mods().is_dir());
        assert!(layout.saves().is_dir());
        assert_eq!(instance.status, InstanceStatus::NotInstalled);
        assert_eq!(instance.required_java_major, 17);

        // Loading back through the index yields the same config.
        let loaded = manager.get(instance.config.id).await.expect("get");
        assert_eq!(loaded.config.name, "Fabulously Optimized");
        assert_eq!(loaded.config.loader.kind, LoaderKind::Fabric);
    }

    #[tokio::test]
    async fn create_rejects_blank_names_and_versions() {
        let temp = tempdir();
        let manager = manager(&temp.0);

        let mut blank = request("   ");
        assert!(manager
            .create(blank.clone(), Arc::new(NoopProgressSink))
            .await
            .is_err());

        blank.name = "Fine".into();
        blank.game_version = "  ".into();
        assert!(manager
            .create(blank, Arc::new(NoopProgressSink))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn update_is_sparse_and_sanitizes_memory() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Vanilla"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let updated = manager
            .update(UpdateInstanceRequest {
                id: instance.config.id,
                memory: Some(MemorySettings {
                    min_mb: 8000,
                    max_mb: 2048,
                }),
                ..Default::default()
            })
            .await
            .expect("update");

        assert_eq!(updated.config.name, "Vanilla");
        assert!(updated.config.memory.max_mb >= updated.config.memory.min_mb);
    }

    #[tokio::test]
    async fn duplicate_copies_files_and_resets_counters() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Original"), Arc::new(NoopProgressSink))
            .await
            .expect("create");
        std::fs::write(manager.layout(instance.config.id).mods().join("a.jar"), b"jar")
            .expect("write mod");

        let copy = manager
            .duplicate(instance.config.id, None)
            .await
            .expect("duplicate");

        assert_ne!(copy.config.id, instance.config.id);
        assert_eq!(copy.config.name, "Original (copy)");
        assert!(manager
            .layout(copy.config.id)
            .mods()
            .join("a.jar")
            .is_file());
        assert_eq!(copy.launch_count, 0);
    }

    #[tokio::test]
    async fn delete_can_keep_or_remove_files() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Disposable"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let root = manager.layout(instance.config.id).root();
        manager.delete(instance.config.id, true).await.expect("delete");

        assert!(!root.exists());
        assert!(manager.get(instance.config.id).await.is_err());
    }

    #[tokio::test]
    async fn refresh_reports_not_installed_without_a_client_jar() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Uninstalled"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let refreshed = manager.refresh(instance.config.id).await.expect("refresh");
        assert_eq!(refreshed.status, InstanceStatus::NotInstalled);
    }

    #[tokio::test]
    async fn refresh_detects_ready_when_version_and_jar_exist() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Installed"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let version_id = instance.config.resolved_version_id();
        std::fs::create_dir_all(manager.paths().version_dir(&version_id)).expect("mkdir");
        std::fs::write(manager.paths().version_json(&version_id), b"{}").expect("json");
        std::fs::write(
            manager
                .paths()
                .version_dir(&version_id)
                .join(format!("{version_id}.jar")),
            b"jar",
        )
        .expect("jar");

        let refreshed = manager.refresh(instance.config.id).await.expect("refresh");
        assert_eq!(refreshed.status, InstanceStatus::Ready);
    }
}
