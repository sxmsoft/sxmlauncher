//! Vanilla installation: version manifest, per-version metadata, client jar,
//! libraries, natives extraction and the asset object store.
//!
//! All heavy content is stored **once** under `shared/` and referenced by every
//! instance, so ten 1.20.1 instances cost one copy of the assets and libraries.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressSink};
use crate::models::version::{
    merge_profiles, AssetIndexRef, DownloadArtifact, FeatureSet, ManifestEntry, VersionJson,
    VersionManifest,
};
use crate::mods::downloader::{DownloadTask, Downloader};
use crate::store::Database;

/// Mojang's public metadata endpoints.
pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";
/// The manifest only changes when Mojang releases something.
const MANIFEST_TTL_SECS: i64 = 6 * 60 * 60;

/// Client-facing Mojang metadata client.
#[derive(Clone)]
pub struct MojangClient {
    http: reqwest::Client,
    cache: Option<Database>,
}

impl MojangClient {
    pub fn new(http: reqwest::Client, cache: Option<Database>) -> Self {
        Self { http, cache }
    }

    /// The version manifest (cached for 6 hours).
    pub async fn version_manifest(&self) -> AppResult<VersionManifest> {
        if let Some(cache) = &self.cache {
            if let Some(payload) = cache.cache_get("mojang:version_manifest")? {
                if let Ok(manifest) = serde_json::from_str::<VersionManifest>(&payload) {
                    return Ok(manifest);
                }
            }
        }

        let manifest = self
            .http
            .get(VERSION_MANIFEST_URL)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("version manifest request failed: {err}")))?
            .json::<VersionManifest>()
            .await?;

        if let Some(cache) = &self.cache {
            let payload = serde_json::to_string(&manifest)?;
            cache.cache_put("mojang:version_manifest", &payload, MANIFEST_TTL_SECS)?;
        }
        Ok(manifest)
    }

    /// Release versions only, newest first.
    pub async fn release_versions(&self) -> AppResult<Vec<ManifestEntry>> {
        Ok(self
            .version_manifest()
            .await?
            .versions
            .into_iter()
            .filter(|entry| entry.release_type == "release")
            .collect())
    }

    /// Metadata URL for a version id.
    pub async fn version_url(&self, version_id: &str) -> AppResult<String> {
        let manifest = self.version_manifest().await?;
        manifest
            .versions
            .iter()
            .find(|entry| entry.id == version_id)
            .map(|entry| entry.url.clone())
            .ok_or_else(|| {
                AppError::Config(format!(
                    "Minecraft {version_id} is not in the version manifest"
                ))
            })
    }

    /// Raw version json (no inheritance resolution).
    pub async fn version_json(&self, version_id: &str) -> AppResult<VersionJson> {
        let cache_key = format!("mojang:version_json:{version_id}");
        if let Some(cache) = &self.cache {
            if let Some(payload) = cache.cache_get(&cache_key)? {
                if let Ok(json) = serde_json::from_str::<VersionJson>(&payload) {
                    return Ok(json);
                }
            }
        }

        let url = self.version_url(version_id).await?;
        let json = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("version json request failed: {err}")))?
            .json::<VersionJson>()
            .await?;

        if let Some(cache) = &self.cache {
            let payload = serde_json::to_string(&json)?;
            // Version metadata is immutable once published.
            cache.cache_put(&cache_key, &payload, 0)?;
        }
        Ok(json)
    }

    /// Version json with `inheritsFrom` already merged in.
    pub async fn resolved_version_json(&self, version_id: &str) -> AppResult<VersionJson> {
        let child = self.version_json(version_id).await?;
        match child.inherits_from.clone() {
            None => Ok(child),
            Some(parent_id) => {
                let parent = self.version_json(&parent_id).await?;
                Ok(merge_profiles(&parent, &child))
            }
        }
    }
}

/// One file an instance install must fetch.
#[derive(Debug, Clone)]
pub struct InstallFile {
    pub label: String,
    pub url: String,
    pub destination: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

/// A native jar plus the extraction rules that apply to it.
#[derive(Debug, Clone)]
pub struct NativeEntry {
    pub url: String,
    pub destination: PathBuf,
    pub excludes: Vec<String>,
}

/// Everything needed to make an instance launchable.
#[derive(Debug, Default)]
pub struct InstallPlan {
    pub java_major: u8,
    pub files: Vec<InstallFile>,
    pub natives: Vec<NativeEntry>,
    pub total_bytes: u64,
}

/// Build the install plan for a resolved version json.
///
/// Mirrors the official launcher order: client jar, libraries, natives, assets.
pub fn build_install_plan(
    version: &VersionJson,
    paths: &AppPaths,
    features: &FeatureSet,
) -> AppResult<InstallPlan> {
    let mut plan = InstallPlan {
        java_major: version
            .java_version
            .as_ref()
            .map(|java| java.major_version)
            .unwrap_or(8),
        ..Default::default()
    };

    // 1. Client jar. Inherited profiles keep this jar beside the vanilla id.
    if let Some(client) = version.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        if !client.url.is_empty() {
            let client_id = crate::models::version::client_jar_version_id(version);
            let destination = paths
                .version_dir(client_id)
                .join(format!("{client_id}.jar"));
            plan.total_bytes += client.size.unwrap_or(0);
            plan.files.push(InstallFile {
                label: format!("{client_id}.jar"),
                url: client.url.clone(),
                destination,
                sha1: client.sha1.clone(),
                size: client.size,
            });
        }
    }

    // 2. Libraries + natives.
    for library in &version.libraries {
        if !library.is_applicable(features) {
            continue;
        }
        if let Some(relative) = library.artifact_path(features) {
            if let Some(url) = library.artifact_url() {
                let size = library
                    .downloads
                    .as_ref()
                    .and_then(|d| d.artifact.as_ref())
                    .and_then(|artifact| artifact.size);
                let sha1 = library
                    .downloads
                    .as_ref()
                    .and_then(|d| d.artifact.as_ref())
                    .and_then(|artifact| artifact.sha1.clone())
                    .or_else(|| library.sha1.clone());
                plan.total_bytes += size.unwrap_or(0);
                plan.files.push(InstallFile {
                    // Maven coordinates are `group:artifact:version`, so the
                    // *middle* segment names the file. Taking the last segment
                    // would label every download with its version number.
                    label: library
                        .name
                        .split(':')
                        .nth(1)
                        .unwrap_or(&library.name)
                        .to_string(),
                    url,
                    destination: paths.libraries().join(&relative),
                    sha1,
                    size,
                });
            }
        }

        // Native-only libraries (pre-1.19 LWJGL layout).
        if let Some(native) = library.native_artifact() {
            plan.natives.push(NativeEntry {
                url: native.url.clone(),
                destination: paths.natives().join(&version.id).join(
                    native
                        .relative_path()
                        .and_then(|path| path.file_name().map(PathBuf::from))
                        .unwrap_or_else(|| PathBuf::from("natives.jar")),
                ),
                excludes: library.extract_excludes(),
            });
        }
    }

    // 3. Asset objects come from the index, which we only fetch on demand.
    Ok(plan)
}

/// Turn an asset index's `objects` map into download tasks.
///
/// Minecraft stores objects content-addressed as `objects/<first2>/<hash>` and
/// serves them from `resources.download.minecraft.net`.
pub fn asset_tasks(index: &serde_json::Value, paths: &AppPaths) -> AppResult<Vec<InstallFile>> {
    let objects = index
        .get("objects")
        .and_then(|value| value.as_object())
        .ok_or_else(|| AppError::Config("asset index has no objects map".into()))?;

    let mut files = Vec::with_capacity(objects.len());
    for (name, entry) in objects {
        let Some(hash) = entry.get("hash").and_then(|value| value.as_str()) else {
            continue;
        };
        let size = entry.get("size").and_then(|value| value.as_u64());
        let shard = hash.get(0..2).unwrap_or("00");
        files.push(InstallFile {
            label: name.clone(),
            url: format!("{RESOURCES_BASE}/{shard}/{hash}"),
            destination: paths.asset_objects().join(shard).join(hash),
            sha1: Some(hash.to_string()),
            size,
        });
    }
    Ok(files)
}

/// Runtime install context: where things go and how to download them.
pub struct Installer<'a> {
    pub paths: &'a AppPaths,
    pub mojang: MojangClient,
    pub downloader: &'a Downloader,
    http: reqwest::Client,
}

impl<'a> Installer<'a> {
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Download libraries from a profile. Client jars and asset objects are
    /// left to [`Self::install_version`], which is only called with a Mojang
    /// version id.
    pub async fn install_libraries(
        &self,
        version: &VersionJson,
        label: &str,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<()> {
        let features = FeatureSet::default();
        let plan = build_install_plan(version, self.paths, &features)?;
        let libraries_root = self.paths.libraries();
        let mut seen = std::collections::HashSet::new();
        let files: Vec<InstallFile> = plan
            .files
            .into_iter()
            .filter(|file| file.destination.starts_with(&libraries_root))
            .filter(|file| seen.insert(file.destination.clone()))
            .collect();
        self.download_files(label, files, sink).await
    }

    pub async fn download_files(
        &self,
        label: &str,
        files: Vec<InstallFile>,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<()> {
        if files.is_empty() {
            return Ok(());
        }
        let tasks: Vec<DownloadTask> = files
            .iter()
            .map(|file| {
                let mut task = DownloadTask::new(
                    file.label.clone(),
                    file.url.clone(),
                    file.destination.clone(),
                );
                if let Some(sha1) = &file.sha1 {
                    task = task.with_sha1(sha1.clone());
                }
                if let Some(size) = file.size {
                    task = task.with_size(size);
                }
                task
            })
            .collect();
        self.downloader
            .fetch_all(JobKind::InstanceInstall, label, tasks, sink)
            .await?;
        Ok(())
    }

    pub fn new(
        paths: &'a AppPaths,
        cache: Option<Database>,
        downloader: &'a Downloader,
        http: reqwest::Client,
    ) -> Self {
        Self {
            paths,
            mojang: MojangClient::new(http.clone(), cache),
            downloader,
            http,
        }
    }

    /// Download the client jar, libraries and natives for a version.
    pub async fn install_version(
        &self,
        version_id: &str,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<VersionJson> {
        let version = self.mojang.resolved_version_json(version_id).await?;

        // Persist the version metadata *before* fetching files. The launcher
        // reads `versions/<id>/<id>.json` from disk when building the launch
        // command (deliberately: launching then works offline and reflects
        // what was verified), so an install that skipped this write produced
        // a "Ready" instance the Play button could never start.
        let version_dir = self.paths.version_dir(version_id);
        tokio::fs::create_dir_all(&version_dir)
            .await
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;
        let json_path = self.paths.version_json(version_id);
        let serialized = serde_json::to_vec_pretty(&version)?;
        tokio::fs::write(&json_path, serialized)
            .await
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;

        let features = FeatureSet::default();
        let plan = build_install_plan(&version, self.paths, &features)?;

        let tasks: Vec<DownloadTask> = plan
            .files
            .iter()
            .map(|file| {
                let mut task = DownloadTask::new(
                    file.label.clone(),
                    file.url.clone(),
                    file.destination.clone(),
                );
                if let Some(sha1) = &file.sha1 {
                    task = task.with_sha1(sha1.clone());
                }
                if let Some(size) = file.size {
                    task = task.with_size(size);
                }
                task
            })
            .collect();

        self.downloader
            .fetch_all(
                JobKind::InstanceInstall,
                format!("Installing Minecraft {version_id}"),
                tasks,
                sink.clone(),
            )
            .await?;

        self.install_natives(&plan.natives, sink.clone()).await?;
        self.install_assets(&version, sink).await?;
        Ok(version)
    }

    /// Extract native jars into `shared/natives/<version>`.
    ///
    /// Modern versions (1.19+) ship natives as regular libraries, so this is a
    /// no-op there; older versions genuinely need the `.dll`/`.so` files.
    async fn install_natives(
        &self,
        natives: &[NativeEntry],
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<()> {
        if natives.is_empty() {
            return Ok(());
        }
        let tasks: Vec<DownloadTask> = natives
            .iter()
            .map(|native| {
                DownloadTask::new("natives", native.url.clone(), native.destination.clone())
            })
            .collect();
        self.downloader
            .fetch_all(JobKind::InstanceInstall, "Natives", tasks, sink.clone())
            .await?;

        for native in natives {
            let archive = native.destination.clone();
            let output = archive
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.paths.natives());
            let excludes = native.excludes.clone();
            tokio::task::spawn_blocking(move || extract_natives(&archive, &output, &excludes))
                .await??;
        }
        Ok(())
    }

    /// Download the asset index and every object it references.
    pub async fn install_assets(
        &self,
        version: &VersionJson,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<()> {
        let Some(index_ref) = &version.asset_index else {
            // Pre-1.6 versions used a flat `resources/` folder; the client jar
            // ships those, so there is nothing to hydrate.
            return Ok(());
        };

        sink.report(
            crate::models::progress::ProgressEvent::started(
                JobKind::AssetHydration,
                "Hydrating assets",
            )
            .stage(JobStage::Resolving)
            .detail(&index_ref.id),
        )
        .await;

        let index_path = self
            .paths
            .assets_indexes()
            .join(format!("{}.json", index_ref.id));
        if !index_path.is_file() {
            download_artifact(&self.paths.downloads, &self.http, index_ref, &index_path).await?;
        }

        let raw = tokio::fs::read_to_string(&index_path).await?;
        let index: serde_json::Value = serde_json::from_str(&raw)?;
        let files = asset_tasks(&index, self.paths)?;

        let tasks: Vec<DownloadTask> = files
            .into_iter()
            .map(|file| {
                let mut task = DownloadTask::new(file.label, file.url, file.destination);
                if let Some(sha1) = file.sha1 {
                    task = task.with_sha1(sha1);
                }
                task
            })
            .collect();

        self.downloader
            .fetch_all(JobKind::AssetHydration, "Assets", tasks, sink)
            .await?;
        Ok(())
    }
}

/// Fetch a single artifact (asset index, logging config, ...).
async fn download_artifact(
    cache_dir: &Path,
    client: &reqwest::Client,
    artifact: &AssetIndexRef,
    destination: &Path,
) -> AppResult<()> {
    let response = client
        .get(&artifact.url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("asset index download failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "asset index download failed with HTTP {}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AppError::Network(format!("asset index body failed: {err}")))?;

    let observed = crate::mods::downloader::sha1_bytes(&bytes);
    if !observed.eq_ignore_ascii_case(&artifact.sha1) {
        return Err(AppError::HashMismatch {
            file: format!("assets/indexes/{}.json", artifact.id),
            expected: artifact.sha1.clone(),
            actual: observed,
        });
    }

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(destination, &bytes).await?;
    let _ = cache_dir;
    Ok(())
}

/// Extract a native jar, skipping the `extract.exclude` patterns.
pub fn extract_natives(archive: &Path, output: &Path, excludes: &[String]) -> AppResult<usize> {
    std::fs::create_dir_all(output)?;
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut extracted = 0usize;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();

        // Library natives are jar-relative; `META-INF/` is always excluded.
        if excludes.iter().any(|pattern| name.starts_with(pattern)) || name.starts_with("META-INF/")
        {
            continue;
        }
        // Only ship actual native binaries, never nested jars or metadata.
        let is_binary = name.ends_with(".dll")
            || name.ends_with(".so")
            || name.ends_with(".dylib")
            || name.ends_with(".jnilib");
        if !is_binary {
            continue;
        }

        let destination = output.join(
            Path::new(&name)
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("native")),
        );
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer)?;
        std::fs::write(&destination, &buffer)?;
        extracted += 1;
    }
    Ok(extracted)
}

/// Map a Mojang artifact into a download task.
pub fn artifact_task(artifact: &DownloadArtifact, destination: PathBuf) -> DownloadTask {
    let mut task = DownloadTask::new(
        destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "artifact".into()),
        artifact.url.clone(),
        destination,
    );
    if let Some(sha1) = &artifact.sha1 {
        task = task.with_sha1(sha1.clone());
    }
    if let Some(size) = artifact.size {
        task = task.with_size(size);
    }
    task
}

/// Group install files by their parent directory (useful for diagnostics).
pub fn group_by_directory(files: &[InstallFile]) -> HashMap<String, usize> {
    let mut groups = HashMap::new();
    for file in files {
        let key = file
            .destination
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default();
        *groups.entry(key).or_insert(0) += 1;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_json() -> VersionJson {
        serde_json::from_value(serde_json::json!({
            "id": "1.20.1",
            "mainClass": "net.minecraft.client.main.Main",
            "assets": "5",
            "javaVersion": { "component": "java-runtime-gamma", "majorVersion": 17 },
            "downloads": {
                "client": { "sha1": "client-sha", "size": 100, "url": "https://piston/client.jar" }
            },
            "libraries": [
                {
                    "name": "com.mojang:brigadier:1.0.18",
                    "downloads": {
                        "artifact": {
                            "path": "com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar",
                            "sha1": "brig-sha",
                            "size": 77,
                            "url": "https://libraries/brigadier.jar"
                        }
                    }
                },
                {
                    "name": "org.lwjgl:lwjgl:3.3.1",
                    "downloads": {
                        "artifact": {
                            "path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar",
                            "sha1": "lwjgl-sha",
                            "size": 12,
                            "url": "https://libraries/lwjgl.jar"
                        },
                        "classifiers": {
                            "natives-windows": {
                                "path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar",
                                "sha1": "native-sha",
                                "url": "https://libraries/lwjgl-natives.jar"
                            }
                        }
                    },
                    "natives": { "windows": "natives-windows" },
                    "extract": { "exclude": ["META-INF/"] }
                }
            ],
            "assetIndex": {
                "id": "5",
                "sha1": "index-sha",
                "size": 5,
                "totalSize": 1000,
                "url": "https://piston/5.json"
            }
        }))
        .expect("version fixture")
    }

    #[test]
    fn plan_includes_client_jar_and_libraries() {
        let paths = AppPaths::from_root("/tmp/sxml-installer");
        let plan =
            build_install_plan(&version_json(), &paths, &FeatureSet::default()).expect("plan");

        assert_eq!(plan.java_major, 17);
        let labels: Vec<&str> = plan.files.iter().map(|file| file.label.as_str()).collect();
        assert!(labels.contains(&"1.20.1.jar"));
        assert!(labels.contains(&"brigadier"));
        assert!(labels.contains(&"lwjgl"));
        // sha1 must travel with the task so the download is verified.
        assert!(plan
            .files
            .iter()
            .any(|file| file.sha1.as_deref() == Some("client-sha")));
    }

    #[test]
    fn natives_are_planned_for_this_platform_only() {
        let paths = AppPaths::from_root("/tmp/sxml-installer");
        let plan =
            build_install_plan(&version_json(), &paths, &FeatureSet::default()).expect("plan");

        if cfg!(target_os = "windows") {
            assert_eq!(plan.natives.len(), 1);
            assert!(plan.natives[0].url.ends_with("lwjgl-natives.jar"));
            assert_eq!(plan.natives[0].excludes, vec!["META-INF/".to_string()]);
        } else {
            // The fixture only declares a windows classifier.
            assert!(plan.natives.is_empty());
        }
    }

    #[test]
    fn asset_tasks_are_content_addressed() {
        let paths = AppPaths::from_root("/tmp/sxml-installer");
        let index = serde_json::json!({
            "objects": {
                "minecraft/sounds/click.ogg": { "hash": "abcdef1234567890", "size": 42 }
            }
        });
        let files = asset_tasks(&index, &paths).expect("tasks");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].sha1.as_deref(), Some("abcdef1234567890"));
        assert!(files[0]
            .url
            .starts_with("https://resources.download.minecraft.net/ab/"));
        assert!(files[0]
            .destination
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("assets/objects/ab/abcdef1234567890"));
    }

    #[test]
    fn asset_index_without_objects_is_an_error() {
        let paths = AppPaths::from_root("/tmp/sxml-installer");
        assert!(asset_tasks(&serde_json::json!({}), &paths).is_err());
    }

    #[test]
    fn native_extraction_keeps_only_binaries() {
        let temp = std::env::temp_dir().join(format!("sxml-natives-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("temp");
        let archive = temp.join("natives.jar");

        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            use std::io::Write;
            for (name, bytes) in [
                ("lwjgl.dll", &b"binary"[..]),
                ("META-INF/MANIFEST.MF", &b"manifest"[..]),
                ("readme.txt", &b"text"[..]),
            ] {
                writer.start_file(name, options).expect("start");
                writer.write_all(bytes).expect("write");
            }
            writer.finish().expect("finish");
        }

        let output = temp.join("out");
        let extracted =
            extract_natives(&archive, &output, &["META-INF/".to_string()]).expect("extract");
        assert_eq!(extracted, 1);
        assert!(output.join("lwjgl.dll").is_file());
        assert!(!output.join("readme.txt").exists());

        let _ = std::fs::remove_dir_all(&temp);
    }
}
