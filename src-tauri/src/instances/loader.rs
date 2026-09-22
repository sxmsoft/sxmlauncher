//! Mod-loader profile installation (Fabric, Quilt, Forge, NeoForge).
//!
//! Vanilla Minecraft is installed by [`super::installer`]; this module layers
//! the loader profile on top so `versions/<loader-id>/<loader-id>.json` exists
//! and the launch planner can resolve a real main class + classpath.
//!
//! * **Fabric / Quilt** — fetch the official meta profile JSON and install its
//!   libraries through the same download pipeline as vanilla.
//! * **Forge / NeoForge** — run the official installer jar with
//!   `--installClient` against the shared Minecraft root (the layout matches
//!   `.minecraft`). A stub `launcher_profiles.json` is written first because
//!   the installer refuses to run without one.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::models::instance::{LoaderKind, ModLoader};
use crate::models::progress::{JobKind, JobStage, ProgressSink};
use crate::models::version::{FeatureSet, VersionJson};
use crate::mods::downloader::{DownloadTask, Downloader};
use crate::process;

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";

/// Install (or repair) the loader profile for an instance.
///
/// Vanilla instances are a no-op. Modded instances write
/// `versions/<id>/<id>.json` and download every library the profile declares.
pub async fn install_loader(
    paths: &AppPaths,
    game_version: &str,
    loader: &ModLoader,
    downloader: &Downloader,
    http: &reqwest::Client,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<Option<VersionJson>> {
    if !loader.kind.is_modded() {
        return Ok(None);
    }

    let loader_version = loader.version.as_deref().ok_or_else(|| {
        AppError::Config(format!(
            "this {} instance has no loader version pinned — reinstall the pack \
             or pick a loader version in instance settings",
            loader.kind.as_str()
        ))
    })?;

    sink.report(
        crate::models::progress::ProgressEvent::started(
            JobKind::InstanceInstall,
            format!("Installing {} {loader_version}", loader.kind.as_str()),
        )
        .stage(JobStage::Resolving)
        .detail(game_version),
    )
    .await;

    match loader.kind {
        LoaderKind::Fabric => {
            install_fabric_like(
                paths,
                FABRIC_META,
                game_version,
                loader_version,
                downloader,
                http,
                sink,
            )
            .await
            .map(Some)
        }
        LoaderKind::Quilt => {
            install_fabric_like(
                paths,
                QUILT_META,
                game_version,
                loader_version,
                downloader,
                http,
                sink,
            )
            .await
            .map(Some)
        }
        LoaderKind::Forge => {
            install_via_official_installer(
                paths,
                forge_installer_url(game_version, loader_version),
                format!("forge-{game_version}-{loader_version}-installer.jar"),
                downloader,
                sink,
            )
            .await?;
            let id = loader.version_id(game_version);
            read_installed_profile(paths, &id).map(Some)
        }
        LoaderKind::NeoForge => {
            install_via_official_installer(
                paths,
                neoforge_installer_url(loader_version),
                format!("neoforge-{loader_version}-installer.jar"),
                downloader,
                sink,
            )
            .await?;
            let id = loader.version_id(game_version);
            // NeoForge ids are `neoforge-<ver>`; the installer may also write
            // `{mc}-neoforge-<ver>`. Try both.
            match read_installed_profile(paths, &id) {
                Ok(profile) => Ok(Some(profile)),
                Err(_) => {
                    let alt = format!("{game_version}-neoforge-{loader_version}");
                    read_installed_profile(paths, &alt).map(Some)
                }
            }
        }
        LoaderKind::Vanilla => Ok(None),
    }
}

/// Fabric and Quilt share the same meta profile shape.
async fn install_fabric_like(
    paths: &AppPaths,
    meta_base: &str,
    game_version: &str,
    loader_version: &str,
    downloader: &Downloader,
    http: &reqwest::Client,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<VersionJson> {
    let url = format!(
        "{meta_base}/versions/loader/{game_version}/{loader_version}/profile/json"
    );
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("loader profile request failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "loader profile download failed with HTTP {} for {url}",
            response.status()
        )));
    }
    let profile: VersionJson = response.json().await.map_err(|err| {
        AppError::Config(format!("loader profile JSON was unreadable: {err}"))
    })?;

    // Persist before downloading libraries so a mid-install crash still leaves
    // a repairable profile on disk.
    let version_dir = paths.version_dir(&profile.id);
    tokio::fs::create_dir_all(&version_dir).await?;
    let json_path = paths.version_json(&profile.id);
    tokio::fs::write(&json_path, serde_json::to_vec_pretty(&profile)?).await?;

    // Fabric/Quilt ship a dummy empty jar next to the profile; the official
    // launcher expects it. An empty file is enough.
    let dummy_jar = version_dir.join(format!("{}.jar", profile.id));
    if !dummy_jar.is_file() {
        tokio::fs::write(&dummy_jar, b"").await?;
    }

    let features = FeatureSet::default();
    let plan = crate::instances::installer::build_install_plan(&profile, paths, &features)?;
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

    if !tasks.is_empty() {
        downloader
            .fetch_all(
                JobKind::InstanceInstall,
                format!("Loader libraries ({})", profile.id),
                tasks,
                sink,
            )
            .await?;
    }

    Ok(profile)
}

fn forge_installer_url(game_version: &str, loader_version: &str) -> String {
    format!(
        "{FORGE_MAVEN}/net/minecraftforge/forge/{game_version}-{loader_version}/forge-{game_version}-{loader_version}-installer.jar"
    )
}

fn neoforge_installer_url(loader_version: &str) -> String {
    format!(
        "{NEOFORGE_MAVEN}/net/neoforged/neoforge/{loader_version}/neoforge-{loader_version}-installer.jar"
    )
}

/// Run the official Forge/NeoForge installer against the shared Minecraft root.
async fn install_via_official_installer(
    paths: &AppPaths,
    installer_url: String,
    installer_name: String,
    downloader: &Downloader,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<()> {
    // The installer insists on a launcher_profiles.json existing in the target.
    ensure_launcher_profiles(paths).await?;

    let installer_path = paths.downloads.join(&installer_name);
    let task = DownloadTask::new(installer_name, installer_url, installer_path.clone());
    // Installer jars are large; skip sha (Forge does not publish one here).
    let tracker = Arc::new(crate::jobs::JobTracker::start(
        JobKind::InstanceInstall,
        "Downloading loader installer",
        JobStage::Downloading,
        sink.clone(),
    ));
    downloader.fetch(task, tracker).await?;

    // Locate a Java runtime to run the installer with.
    let java = find_java_for_installer(paths).await?;

    sink.report(
        crate::models::progress::ProgressEvent::started(
            JobKind::InstanceInstall,
            "Running loader installer",
        )
        .stage(JobStage::Extracting)
        .detail(installer_path.display().to_string()),
    )
    .await;

    let shared = paths.shared.clone();
    let output = process::command(&java)
        .arg("-jar")
        .arg(&installer_path)
        .arg("--installClient")
        .arg(&shared)
        .output()
        .await
        .map_err(|err| {
            AppError::Java(format!(
                "could not run the loader installer ({}): {err}",
                installer_path.display()
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().chars().take(500).collect()
        };
        return Err(AppError::Config(format!(
            "the loader installer failed: {detail}"
        )));
    }

    Ok(())
}

async fn ensure_launcher_profiles(paths: &AppPaths) -> AppResult<()> {
    let profiles = paths.shared.join("launcher_profiles.json");
    if profiles.is_file() {
        return Ok(());
    }
    let stub = serde_json::json!({
        "profiles": {},
        "clientToken": "00000000000000000000000000000000",
        "launcherVersion": { "name": "sxmlauncher", "format": 21 }
    });
    tokio::fs::create_dir_all(&paths.shared).await?;
    tokio::fs::write(&profiles, serde_json::to_vec_pretty(&stub)?).await?;
    Ok(())
}

async fn find_java_for_installer(paths: &AppPaths) -> AppResult<PathBuf> {
    // Prefer a managed JDK 17+; fall back to PATH.
    let managed = paths.java.clone();
    if managed.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&managed) {
            for entry in entries.flatten() {
                let candidate = entry
                    .path()
                    .join("bin")
                    .join(if cfg!(windows) { "java.exe" } else { "java" });
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    // `java` on PATH — the installer itself needs a recent enough JDK.
    Ok(PathBuf::from("java"))
}

fn read_installed_profile(paths: &AppPaths, version_id: &str) -> AppResult<VersionJson> {
    let path = paths.version_json(version_id);
    let raw = std::fs::read_to_string(&path).map_err(|err| {
        AppError::Config(format!(
            "loader install finished but {} is missing: {err}",
            path.display()
        ))
    })?;
    Ok(serde_json::from_str(&raw)?)
}

/// Resolve the latest stable Fabric loader for a game version (when a pack
/// forgets to pin one).
pub async fn latest_fabric_loader(
    http: &reqwest::Client,
    game_version: &str,
) -> AppResult<String> {
    let url = format!("{FABRIC_META}/versions/loader/{game_version}");
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("Fabric loader list failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "Fabric loader list failed with HTTP {}",
            response.status()
        )));
    }
    let list: Vec<serde_json::Value> = response.json().await?;
    list.iter()
        .find_map(|entry| {
            let stable = entry
                .pointer("/loader/stable")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let version = entry.pointer("/loader/version")?.as_str()?;
            if stable {
                Some(version.to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            list.first()
                .and_then(|entry| entry.pointer("/loader/version"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            AppError::ModResolution(format!(
                "no Fabric loader is published for Minecraft {game_version}"
            ))
        })
}

/// Build the profile URL for diagnostics / tests.
pub fn fabric_profile_url(game_version: &str, loader_version: &str) -> String {
    format!(
        "{FABRIC_META}/versions/loader/{game_version}/{loader_version}/profile/json"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_installer_url_matches_maven_layout() {
        assert_eq!(
            forge_installer_url("1.20.1", "47.2.0"),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"
        );
    }

    #[test]
    fn neoforge_installer_url_matches_maven_layout() {
        assert_eq!(
            neoforge_installer_url("21.1.72"),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.72/neoforge-21.1.72-installer.jar"
        );
    }

    #[test]
    fn fabric_profile_url_is_stable() {
        assert_eq!(
            fabric_profile_url("1.20.1", "0.15.11"),
            "https://meta.fabricmc.net/v2/versions/loader/1.20.1/0.15.11/profile/json"
        );
    }

    #[test]
    fn stub_profiles_path_lives_under_shared() {
        let paths = AppPaths::from_root("/tmp/sxml-loader");
        assert_eq!(
            paths.shared.join("launcher_profiles.json"),
            PathBuf::from("/tmp/sxml-loader/shared/launcher_profiles.json")
        );
    }
}
