//! Mod & modpack engine.
//!
//! Layering (each layer only knows the one below it):
//!
//! ```text
//!   ModEngine      orchestration: plan -> download -> verify -> extract
//!   ├── resolver   registry-agnostic dependency graph -> plan
//!   ├── mrpack     .mrpack manifests + overrides
//!   ├── downloader parallel + hash-verified downloads
//!   └── java       runtime detection/provisioning
//! ```
//!
//! `ModEngine` never mutates instance config: it writes files and returns the
//! plan it executed, and the instance manager persists the outcome.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::{AppPaths, AppSettings};
use crate::error::{AppError, AppResult};
use crate::jobs::JobTracker;
use crate::models::modpack::{PackTarget, ResolvedPackPlan};
use crate::models::progress::{JobKind, JobStage, ProgressSink};
use crate::store::Database;

/// Official authlib-injector metadata (sha256-verified download).
///
/// Never use the broken `github.com/elyby/authlib-injector/.../authlib-injector.jar`
/// URL — that release asset does not exist and fails every Ely.by launch.
pub const AUTHLIB_INJECTOR_META: &str = "https://authlib-injector.yushi.moe/artifact/latest.json";
/// BMCLAPI mirror of the same metadata JSON (China / CDN fallback).
pub const AUTHLIB_INJECTOR_META_BMCLAPI: &str =
    "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/artifact/latest.json";

#[derive(Debug, Clone, serde::Deserialize)]
struct AuthlibInjectorMeta {
    version: String,
    download_url: String,
    checksums: AuthlibChecksums,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct AuthlibChecksums {
    sha256: String,
}

pub mod curseforge;
pub mod curseforge_pack;
pub mod downloader;
pub mod java_runtime;
pub mod modrinth;
pub mod mrpack;
pub mod resolver;

pub use crate::models::modpack::MrpackManifest;
pub use curseforge::{fingerprint, fingerprint_file, CurseForgeClient};
pub use downloader::{
    sha1_bytes, sha1_file, sha256_file, DownloadOutcome, DownloadTask, Downloader,
};
pub use java_runtime::{required_java_major, required_major_for, JavaRegistry, JavaRuntime};
pub use modrinth::ModrinthClient;
pub use mrpack::InstalledJar;
pub use resolver::{ModRegistry, ModRequest};

/// The single entry point for everything that downloads game content.
#[derive(Clone)]
pub struct ModEngine {
    paths: AppPaths,
    db: Database,
    http: reqwest::Client,
    downloader: Downloader,
    modrinth: ModrinthClient,
    curseforge: CurseForgeClient,
    java: JavaRegistry,
}

impl ModEngine {
    pub fn new(paths: AppPaths, db: Database, settings: &AppSettings) -> AppResult<Self> {
        let http = crate::mods::modrinth::http_client()?;
        let downloader = Downloader::new(
            http.clone(),
            paths.downloads.clone(),
            settings.max_concurrent_downloads,
        );

        Ok(Self {
            modrinth: ModrinthClient::new(http.clone(), Some(db.clone())),
            curseforge: CurseForgeClient::new(http.clone(), settings.curseforge_api_key.clone()),
            java: JavaRegistry::from_settings(paths.clone(), http.clone(), settings),
            downloader,
            paths,
            db,
            http,
        })
    }

    pub fn modrinth(&self) -> &ModrinthClient {
        &self.modrinth
    }

    pub fn curseforge(&self) -> &CurseForgeClient {
        &self.curseforge
    }

    pub fn java(&self) -> &JavaRegistry {
        &self.java
    }

    pub fn downloader(&self) -> &Downloader {
        &self.downloader
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// Registry pair used for dependency resolution.
    pub fn registry(&self) -> ModRegistry {
        ModRegistry::new(self.modrinth.clone(), self.curseforge.clone())
    }

    /// Resolve a selection of mods into a complete plan.
    pub async fn resolve(
        &self,
        requests: &[ModRequest],
        target: &PackTarget,
    ) -> AppResult<ResolvedPackPlan> {
        self.registry().resolve(requests, target).await
    }

    /// Execute a plan: download every file with hash verification.
    ///
    /// Instances are only considered installed once every file verified, so a
    /// half-finished download never presents as a playable instance.
    pub async fn install_plan(
        &self,
        plan: &ResolvedPackPlan,
        instance_root: &Path,
        label: impl Into<String>,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Vec<DownloadOutcome>> {
        if plan.files.is_empty() {
            return Ok(Vec::new());
        }

        let tasks: Vec<DownloadTask> = plan
            .files
            .iter()
            .map(|file| {
                let mut task = DownloadTask::new(
                    file.file_name.clone(),
                    file.url.clone(),
                    instance_root.join(&file.destination),
                );
                if let Some(sha1) = &file.sha1 {
                    task = task.with_sha1(sha1.clone());
                }
                task.with_size(file.size)
            })
            .collect();

        // Files with no download URL can only have come from a pack manifest
        // that uses an unsupported CDN; fail loudly rather than installing a
        // partial pack.
        for (index, task) in tasks.iter().enumerate() {
            if task.url.is_empty() {
                return Err(AppError::ModResolution(format!(
                    "{} has no download URL in this pack's manifest",
                    plan.files[index].title
                )));
            }
        }

        self.downloader
            .fetch_all(JobKind::ModpackInstall, label, tasks, sink)
            .await
    }

    /// Install an `.mrpack`: extract overrides, then download every manifest file.
    pub async fn install_mrpack(
        &self,
        archive: PathBuf,
        instance_root: PathBuf,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ResolvedPackPlan> {
        let manifest_archive = archive.clone();
        let manifest =
            tokio::task::spawn_blocking(move || mrpack::read_manifest(&manifest_archive)).await??;

        let pack = mrpack::pack_reference(
            "unknown",
            &manifest.version_id,
            &manifest.name,
            &manifest.version_id,
            None,
        );
        let plan = mrpack::plan_from_manifest(&manifest, &pack)?;

        // Overrides first: a manifest file must be able to replace one.
        mrpack::extract_overrides(archive, instance_root.clone()).await?;
        self.install_plan(
            &plan,
            &instance_root,
            format!("Installing {}", manifest.name),
            sink,
        )
        .await?;

        Ok(plan)
    }

    /// List the jars currently installed in an instance.
    pub async fn scan_mods(&self, instance_root: &Path) -> AppResult<Vec<InstalledJar>> {
        mrpack::scan_mods(instance_root.to_path_buf()).await
    }

    /// Ensure a Java runtime for the required major version.
    ///
    /// A compatible runtime that is already installed (a system JDK, or a JDK the
    /// launcher downloaded earlier) is reused as-is. A managed Temurin download
    /// only happens when the machine has nothing that can run the game — which is
    /// what keeps instance installs fast.
    pub async fn ensure_java(
        &self,
        major: u8,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<JavaRuntime> {
        self.java.ensure(major, sink.as_ref()).await
    }

    /// Ensure the authlib-injector agent jar is present (Ely.by accounts).
    ///
    /// Resolves the latest build from the official metadata JSON (with a BMCLAPI
    /// fallback), verifies the published sha256, and stores one copy under the
    /// app root. The download is a normal cancelable job in the Activity panel.
    pub async fn ensure_authlib_injector(&self, sink: Arc<dyn ProgressSink>) -> AppResult<PathBuf> {
        let destination = self.paths.authlib_injector();
        let meta = self.fetch_authlib_meta().await?;
        let expected = meta.checksums.sha256.to_lowercase();

        if destination.is_file() {
            if let Ok(actual) = sha256_file(&destination) {
                if actual.eq_ignore_ascii_case(&expected) {
                    return Ok(destination);
                }
            }
            // Stale or corrupt jar — replace it.
            let _ = tokio::fs::remove_file(&destination).await;
        }

        let task = DownloadTask::new(
            format!("authlib-injector {}", meta.version),
            meta.download_url,
            destination.clone(),
        )
        .with_sha256(expected)
        .without_cache();
        let tracker = Arc::new(JobTracker::start(
            JobKind::Launch,
            "authlib-injector (Ely.by agent)",
            JobStage::Downloading,
            sink,
        ));
        let fetched = self.downloader.fetch(task, tracker.clone()).await;
        Downloader::settle(&tracker, fetched).await?;
        Ok(destination)
    }

    /// Fetch authlib-injector metadata, preferring the official host then BMCLAPI.
    async fn fetch_authlib_meta(&self) -> AppResult<AuthlibInjectorMeta> {
        let mut last_err = None;
        for url in [AUTHLIB_INJECTOR_META, AUTHLIB_INJECTOR_META_BMCLAPI] {
            match self.http.get(url).send().await {
                Ok(response) if response.status().is_success() => {
                    return response.json().await.map_err(|err| {
                        AppError::Network(format!(
                            "authlib-injector metadata from {url} was not valid JSON: {err}"
                        ))
                    });
                }
                Ok(response) => {
                    last_err = Some(AppError::Network(format!(
                        "authlib-injector metadata {url} returned HTTP {}",
                        response.status()
                    )));
                }
                Err(err) => {
                    last_err = Some(AppError::Network(format!(
                        "authlib-injector metadata {url} unreachable: {err}"
                    )));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            AppError::Network("authlib-injector metadata could not be fetched".into())
        }))
    }

    /// Resolve a runtime without downloading anything.
    pub async fn resolve_java(
        &self,
        major: u8,
        allow_download: bool,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Option<JavaRuntime>> {
        self.java
            .ensure_with(major, sink.as_ref(), allow_download)
            .await
    }

    /// Adopt a fresh API key without rebuilding the whole engine.
    pub fn with_settings(&self, settings: &AppSettings) -> AppResult<Self> {
        let mut engine = self.clone();
        engine.curseforge =
            CurseForgeClient::new(engine.http.clone(), settings.curseforge_api_key.clone());
        engine.downloader = Downloader::new(
            engine.http.clone(),
            engine.paths.downloads.clone(),
            settings.max_concurrent_downloads,
        );
        // The Java roots list lives in settings, so the registry is rebuilt too.
        engine.java =
            JavaRegistry::from_settings(engine.paths.clone(), engine.http.clone(), settings);
        Ok(engine)
    }

    /// Refresh the CurseForge key in place.
    pub fn set_curseforge_key(&mut self, api_key: Option<String>) {
        self.curseforge = CurseForgeClient::new(self.http.clone(), api_key);
    }

    /// Total size of the shared download cache.
    pub async fn download_cache_size(&self) -> u64 {
        self.downloader.cache_size().await
    }

    /// Persist a resolved plan's files as "installed" records.
    pub fn record_installed(
        &self,
        instance_id: uuid::Uuid,
        plan: &ResolvedPackPlan,
    ) -> AppResult<()> {
        for file in &plan.files {
            self.db
                .upsert_installed_mod(&crate::store::instances::InstalledModRow {
                    instance_id,
                    source: file.source,
                    project_id: file.project_id.clone(),
                    version_id: file.version_id.clone(),
                    title: file.title.clone(),
                    file_name: file.file_name.clone(),
                    sha1: file.sha1.clone(),
                    enabled: true,
                    installed_at: chrono::Utc::now(),
                })?;
        }
        Ok(())
    }
}
