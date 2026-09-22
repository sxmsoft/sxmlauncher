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
use crate::models::progress::{InstanceBoundSink, JobKind, JobStage, ProgressEvent, ProgressSink};
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
    pub fn new(paths: AppPaths, db: Database, mods: Arc<ModEngine>, settings: AppSettings) -> Self {
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

    /// Install (or repair) the vanilla game plus, when needed, its loader profile.
    ///
    /// The Mojang manifest is only queried with [`InstanceConfig::mojang_version_id`].
    /// Fabric/Quilt/Forge/NeoForge ids are profiles that inherit that version.
    pub async fn install(
        &self,
        mut instance: Instance,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Instance> {
        let id = instance.config.id;
        self.db
            .set_instance_status(id, InstanceStatus::Installing)?;
        let sink: Arc<dyn ProgressSink> = Arc::new(InstanceBoundSink::new(sink, id));

        let game_version = instance.config.mojang_version_id().to_string();
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
                "Minecraft {} · {}",
                instance.config.game_version,
                instance.config.loader.kind.as_str()
            )),
        )
        .await;

        // Provision Java *before* touching game files: an unlaunchable instance
        // is worse than a failed install.
        let required = crate::mods::required_major_for(
            &instance.config.game_version,
            instance.config.loader.kind,
        );
        let mut java_bin = instance
            .config
            .java
            .override_path
            .clone()
            .filter(|path| path.is_file());
        if java_bin.is_none() {
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
                && resolved.as_ref().is_none_or(|runtime| !runtime.is_managed);
            if wants_managed {
                let managed = self.mods.java().install(required, sink.as_ref()).await?;
                java_bin = Some(managed.path.clone());
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
                java_bin = Some(runtime.path.clone());
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
        let result: AppResult<()> = async {
            installer
                .install_version(&game_version, version_sink.clone())
                .await?;
            if instance.config.loader.kind.is_modded() {
                let installed = crate::instances::loaders::install_loader(
                    &installer,
                    &instance.config,
                    java_bin.as_deref(),
                    version_sink.clone(),
                )
                .await?;
                instance.config.loader.version = Some(installed.version);
                instance.config.touch();
                self.write_config(&instance.config)?;
                self.db.upsert_instance(&instance)?;
            }
            Ok(())
        }
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
            let _ = self
                .db
                .set_instance_status(id, InstanceStatus::NotInstalled);
        }

        result?;

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
        let game_version = instance.config.mojang_version_id();
        let profile_id = instance.config.resolved_version_id();
        // The client jar always belongs to the plain Minecraft version. Loader
        // profiles only add `versions/<profile>/<profile>.json`.
        let client_jar = self
            .paths
            .version_dir(game_version)
            .join(format!("{game_version}.jar"));
        let vanilla_json = self.paths.version_json(game_version);
        let profile_json = self.paths.version_json(&profile_id);

        instance.status = if !layout.root().exists() {
            InstanceStatus::NotInstalled
        } else if vanilla_json.is_file() && client_jar.is_file() && profile_json.is_file() {
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

        // Shared libraries and the client jar live outside the instance folder.
        // What the card calls "size" is this folder: mods, configs, saves, logs.
        let size_root = layout.root();
        instance.size_bytes = tokio::task::spawn_blocking(move || {
            if size_root.exists() {
                directory_size(&size_root)
            } else {
                0
            }
        })
        .await
        .unwrap_or(0);

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

    struct LiveInstallSink;

    #[async_trait::async_trait]
    impl ProgressSink for LiveInstallSink {
        async fn report(&self, event: ProgressEvent) {
            if event.error.is_some()
                || event.finished
                || matches!(
                    event.stage,
                    JobStage::Resolving
                        | JobStage::ProvisioningJava
                        | JobStage::Extracting
                        | JobStage::Failed
                )
            {
                eprintln!(
                    "[install] {} {:?} {} {}",
                    event.label,
                    event.stage,
                    event.detail.as_deref().unwrap_or(""),
                    event.error.as_deref().unwrap_or("")
                );
            }
        }
    }

    fn manager(root: &std::path::Path) -> InstanceManager {
        let paths = AppPaths::from_root(root);
        paths.ensure().expect("layout");
        let db = Database::open_in_memory().expect("db");
        let settings = AppSettings::default();
        let engine =
            Arc::new(ModEngine::new(paths.clone(), db.clone(), &settings).expect("engine"));
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
        std::fs::write(
            manager.layout(instance.config.id).mods().join("a.jar"),
            b"jar",
        )
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
        manager
            .delete(instance.config.id, true)
            .await
            .expect("delete");

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

        let marker = manager.layout(instance.config.id).mods();
        std::fs::create_dir_all(&marker).expect("mods dir");
        std::fs::write(marker.join("pack.jar"), vec![0u8; 128]).expect("mod jar");

        let refreshed = manager.refresh(instance.config.id).await.expect("refresh");
        assert_eq!(refreshed.status, InstanceStatus::NotInstalled);
        assert!(
            refreshed.size_bytes >= 128,
            "refresh should record the instance folder size, got {}",
            refreshed.size_bytes
        );
    }

    #[tokio::test]
    async fn refresh_detects_ready_when_version_and_jar_exist() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        let instance = manager
            .create(request("Installed"), Arc::new(NoopProgressSink))
            .await
            .expect("create");

        let game_version = instance.config.mojang_version_id().to_string();
        let profile_id = instance.config.resolved_version_id();
        std::fs::create_dir_all(manager.paths().version_dir(&game_version)).expect("mkdir");
        std::fs::create_dir_all(manager.paths().version_dir(&profile_id)).expect("mkdir");
        std::fs::write(manager.paths().version_json(&game_version), b"{}").expect("vanilla json");
        std::fs::write(manager.paths().version_json(&profile_id), b"{}").expect("profile json");
        std::fs::write(
            manager
                .paths()
                .version_dir(&game_version)
                .join(format!("{game_version}.jar")),
            b"jar",
        )
        .expect("jar");

        let refreshed = manager.refresh(instance.config.id).await.expect("refresh");
        assert_eq!(refreshed.status, InstanceStatus::Ready);
    }

    /// End-to-end install against the public meta APIs. Ignored by default
    /// because it downloads the Minecraft client, libraries and assets.
    #[tokio::test]
    #[ignore = "downloads Minecraft 1.21.1 plus Fabric, Quilt, Forge and NeoForge"]
    async fn live_modded_install_uses_the_plain_minecraft_version() {
        let temp = tempdir();
        let manager = manager(&temp.0);
        for (name, kind) in [
            ("Fabric", LoaderKind::Fabric),
            ("Quilt", LoaderKind::Quilt),
            ("Forge", LoaderKind::Forge),
            ("NeoForge", LoaderKind::NeoForge),
        ] {
            let mut created = request(name);
            created.game_version = "1.21.1".into();
            created.loader = Some(ModLoader {
                kind,
                version: None,
                build: None,
            });
            let instance = manager
                .create(created, Arc::new(NoopProgressSink))
                .await
                .unwrap_or_else(|err| panic!("{name} create: {err}"));
            let installed = manager
                .install(instance, Arc::new(LiveInstallSink))
                .await
                .unwrap_or_else(|err| panic!("{name} install: {err}"));
            assert_eq!(installed.status, InstanceStatus::Ready, "{name} status");
            assert_eq!(installed.config.mojang_version_id(), "1.21.1");
            assert_ne!(installed.config.resolved_version_id(), "1.21.1");
            assert!(installed.config.loader.version.is_some(), "{name} version");

            let profile_raw = std::fs::read_to_string(
                manager
                    .paths()
                    .version_json(&installed.config.resolved_version_id()),
            )
            .unwrap_or_else(|err| panic!("{name} profile: {err}"));
            let profile: crate::models::version::VersionJson =
                serde_json::from_str(&profile_raw).expect("profile json");
            assert_eq!(profile.inherits_from.as_deref(), Some("1.21.1"));
            assert!(manager
                .paths()
                .version_dir("1.21.1")
                .join("1.21.1.jar")
                .is_file());

            let vanilla_raw =
                std::fs::read_to_string(manager.paths().version_json("1.21.1")).expect("vanilla");
            let vanilla: crate::models::version::VersionJson =
                serde_json::from_str(&vanilla_raw).expect("vanilla json");
            let merged = crate::models::version::merge_profiles(&vanilla, &profile);
            let planner =
                crate::instances::launch::LaunchPlanner::new(manager.paths().clone(), Vec::new());
            planner
                .build_classpath(&merged, &manager.layout(installed.config.id).root())
                .unwrap_or_else(|err| panic!("{name} classpath: {err}"));

            let runtime = crate::mods::java_runtime::JavaRuntime {
                path: std::path::PathBuf::from("/usr/bin/java"),
                major: 21,
                version: "21".into(),
                vendor: "OpenJDK".into(),
                is_managed: false,
                architecture: std::env::consts::ARCH.to_string(),
            };
            let launching = crate::instances::launch::LaunchPlanner::new(
                manager.paths().clone(),
                vec![runtime],
            );
            let identity =
                crate::models::account::LaunchIdentity::offline("Steve", uuid::Uuid::new_v4());
            let plan = launching
                .build(
                    &installed,
                    &merged,
                    &identity,
                    &crate::instances::launch::LaunchExtras::default(),
                )
                .unwrap_or_else(|err| panic!("{name} launch plan: {err}"));
            assert!(
                plan.jvm_args.iter().any(|arg| arg == "-cp"),
                "{name} launch is missing -cp: {:?}",
                plan.jvm_args
            );
            assert!(
                plan.game_args
                    .windows(2)
                    .any(|pair| pair[0] == "--username" && pair[1] == "Steve"),
                "{name} launch is missing the vanilla username arg: {:?}",
                plan.game_args
            );
            let expected_main = match kind {
                LoaderKind::Fabric | LoaderKind::Quilt => "KnotClient",
                LoaderKind::Forge => "ForgeBootstrap",
                LoaderKind::NeoForge => "BootstrapLauncher",
                LoaderKind::Vanilla => "Main",
            };
            assert!(
                plan.main_class.contains(expected_main),
                "{name} main class was {}",
                plan.main_class
            );

            let (_game, mut child) =
                crate::instances::launch::spawn(&plan, installed.config.id, None)
                    .await
                    .unwrap_or_else(|err| panic!("{name} spawn: {err}"));
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let _ = child.kill().await;
            let log = std::fs::read_to_string(&plan.log_file).unwrap_or_default();
            assert!(
                log.len() > 40,
                "{name} produced no launch log (main {})",
                plan.main_class
            );
            let tail: String = log
                .chars()
                .rev()
                .take(240)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            eprintln!("[launch] {name} log tail: {tail}");
            if log.contains("Exception in thread") {
                eprintln!("----- {name} launch log -----\n{log}\n----- end -----");
                panic!("{name} crashed during launch");
            }
            for needle in [
                "NoClassDefFoundError",
                "ClassNotFoundException",
                "Could not find or load main class",
                "UnsupportedClassVersionError",
            ] {
                assert!(
                    !log.contains(needle),
                    "{name} failed to launch ({needle}):\n{log}"
                );
            }

            let loader_version = installed.config.loader.version.as_deref().unwrap();
            let patched = match kind {
                LoaderKind::Forge => format!(
                    "net/minecraftforge/forge/1.21.1-{loader_version}/forge-1.21.1-{loader_version}-client.jar"
                ),
                LoaderKind::NeoForge => format!(
                    "net/neoforged/neoforge/{loader_version}/neoforge-{loader_version}-client.jar"
                ),
                _ => String::new(),
            };
            if !patched.is_empty() {
                let path = manager.paths().libraries().join(&patched);
                assert!(
                    path.is_file(),
                    "{name} installer did not produce {}",
                    path.display()
                );
            }
        }
    }
}
