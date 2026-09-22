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

use serde::Deserialize;

use crate::config::{AppPaths, AppSettings};
use crate::error::{AppError, AppResult};
use crate::jobs::JobTracker;
use crate::models::progress::{JobKind, JobStage, ProgressSink};
use crate::models::modpack::{PackTarget, ResolvedPackPlan};
use crate::store::Database;

/// Primary metadata endpoint for the official authlib-injector agent jar.
///
/// Returns JSON with `download_url` + `checksums.sha256`. Do **not** confuse this
/// with Ely.by's authlib *API* endpoint (`https://authserver.ely.by/api/authlib-injector`),
/// which is passed to the JVM as `-javaagent:…jar=<api-url>` after the jar is on disk.
pub const AUTHLIB_INJECTOR_METADATA_URL: &str =
    "https://authlib-injector.yushi.moe/artifact/latest.json";
/// BMCLAPI mirror of the same latest.json metadata (China-friendly fallback).
pub const AUTHLIB_INJECTOR_METADATA_MIRROR_URL: &str =
    "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/artifact/latest.json";

pub mod curseforge;
pub mod downloader;
pub mod java_runtime;
pub mod modrinth;
pub mod mrpack;
pub mod resolver;

pub use curseforge::{fingerprint, fingerprint_file, CurseForgeClient};
pub use downloader::{sha1_bytes, sha1_file, DownloadOutcome, DownloadTask, Downloader};
pub use java_runtime::{required_java_major, required_major_for, JavaRegistry, JavaRuntime};
pub use modrinth::ModrinthClient;
pub use crate::models::modpack::MrpackManifest;
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
        let manifest = tokio::task::spawn_blocking(move || mrpack::read_manifest(&manifest_archive))
            .await??;

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
    /// Resolves the concrete download URL from yushi.moe / BMCLAPI `latest.json`
    /// (the old GitHub `elyby/authlib-injector` asset path 404s), verifies the
    /// published SHA-256, and keeps exactly one copy in the app root.
    pub async fn ensure_authlib_injector(
        &self,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<PathBuf> {
        let destination = self.paths.authlib_injector();
        let existing_size = std::fs::metadata(&destination).map(|meta| meta.len()).unwrap_or(0);
        if destination.is_file() && existing_size > 64 * 1024 {
            return Ok(destination);
        }

        let artifact = resolve_authlib_injector_artifact(&self.http).await?;
        let mut task = DownloadTask::new(
            crate::config::AUTHLIB_INJECTOR_JAR,
            artifact.download_url,
            destination.clone(),
        )
        .without_cache();
        task = task.with_sha256(artifact.sha256);
        let tracker = Arc::new(JobTracker::start(
            JobKind::Launch,
            format!("authlib-injector {}", artifact.version),
            JobStage::Downloading,
            sink,
        ));
        self.downloader.fetch(task, tracker).await?;
        Ok(destination)
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
        engine.curseforge = CurseForgeClient::new(
            engine.http.clone(),
            settings.curseforge_api_key.clone(),
        );
        engine.downloader = Downloader::new(
            engine.http.clone(),
            engine.paths.downloads.clone(),
            settings.max_concurrent_downloads,
        );
        // The Java roots list lives in settings, so the registry is rebuilt too.
        engine.java = JavaRegistry::from_settings(engine.paths.clone(), engine.http.clone(), settings);
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
            self.db.upsert_installed_mod(&crate::store::instances::InstalledModRow {
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

/// Concrete authlib-injector jar chosen from a latest.json metadata document.
#[derive(Debug, Clone)]
struct AuthlibInjectorArtifact {
    version: String,
    download_url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct AuthlibLatestJson {
    version: String,
    download_url: String,
    checksums: AuthlibChecksums,
}

#[derive(Debug, Deserialize)]
struct AuthlibChecksums {
    sha256: String,
}

/// Fetch latest.json from the primary host, falling back to the BMCLAPI mirror.
async fn resolve_authlib_injector_artifact(
    http: &reqwest::Client,
) -> AppResult<AuthlibInjectorArtifact> {
    let mut last_err = None;
    for url in [
        AUTHLIB_INJECTOR_METADATA_URL,
        AUTHLIB_INJECTOR_METADATA_MIRROR_URL,
    ] {
        match fetch_authlib_latest_json(http, url).await {
            Ok(artifact) => return Ok(artifact),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        AppError::Network("authlib-injector metadata could not be resolved".into())
    }))
}

async fn fetch_authlib_latest_json(
    http: &reqwest::Client,
    url: &str,
) -> AppResult<AuthlibInjectorArtifact> {
    let response = http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| AppError::Network(format!("authlib-injector metadata ({url}): {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "authlib-injector metadata: HTTP {} from {url}",
            response.status()
        )));
    }
    let body = response.text().await.map_err(|err| {
        AppError::Network(format!("authlib-injector metadata body ({url}): {err}"))
    })?;
    parse_authlib_latest_json(&body)
}

fn parse_authlib_latest_json(body: &str) -> AppResult<AuthlibInjectorArtifact> {
    let parsed: AuthlibLatestJson = serde_json::from_str(body).map_err(|err| {
        AppError::Network(format!("authlib-injector metadata is not valid JSON: {err}"))
    })?;
    if parsed.download_url.trim().is_empty() {
        return Err(AppError::Network(
            "authlib-injector metadata has an empty download_url".into(),
        ));
    }
    let sha256 = parsed.checksums.sha256.trim().to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::Network(format!(
            "authlib-injector metadata has an invalid sha256 `{sha256}`"
        )));
    }
    Ok(AuthlibInjectorArtifact {
        version: parsed.version,
        download_url: parsed.download_url,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yushi_latest_json() {
        let body = r#"{
          "build_number": 56,
          "version": "1.2.8",
          "download_url": "https://authlib-injector.yushi.moe/artifact/56/authlib-injector-1.2.8.jar",
          "checksums": {
            "sha256": "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10"
          }
        }"#;
        let artifact = parse_authlib_latest_json(body).expect("parse");
        assert_eq!(artifact.version, "1.2.8");
        assert!(artifact.download_url.contains("authlib-injector-1.2.8.jar"));
        assert_eq!(
            artifact.sha256,
            "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10"
        );
    }

    #[test]
    fn rejects_missing_checksum() {
        let body = r#"{"version":"1.0","download_url":"https://x/a.jar","checksums":{"sha256":"nope"}}"#;
        assert!(parse_authlib_latest_json(body).is_err());
    }
}
