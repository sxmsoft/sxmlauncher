//! Application-level commands: info, settings, caches, diagnostics, updater.
//!
//! Rules every command here follows:
//! * Thin: argument validation + delegation, nothing else.
//! * No `unwrap()`: failures become [`crate::error::AppError`], which
//!   serializes to `{ code, message, retryable }` for the frontend.
//!
//! The updater commands wrap `tauri_plugin_updater`: the check talks to the
//! signed release feed configured in `tauri.conf.json`, the download streams
//! bytes (relayed as `updater://progress` events), and the install restarts the
//! launcher so the platform installer can swap the binary.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

use crate::config::{AppSettings, MAX_ICON_BYTES};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
/// Tauri event carrying update download progress.
pub const UPDATE_PROGRESS_EVENT: &str = "updater://progress";

/// Build/runtime information for the About pane and bug reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub tauri_version: String,
    pub rust_version: String,
    pub os: String,
    pub arch: String,
    pub vault_backend: String,
    pub curseforge_configured: bool,
    pub max_icon_bytes: usize,
}

#[tauri::command]
pub async fn app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    Ok(AppInfo {
        name: "SXMLauncher".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        tauri_version: tauri::VERSION.to_string(),
        rust_version: option_env!("CARGO_PKG_RUST_VERSION")
            .unwrap_or("unknown")
            .to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        vault_backend: state.accounts().vault_backend().to_string(),
        curseforge_configured: state.mods().curseforge().is_configured(),
        max_icon_bytes: MAX_ICON_BYTES,
    })
}

/// Where every piece of launcher data lives (About pane → "open folder").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsReport {
    pub root: String,
    pub instances: String,
    pub shared: String,
    pub java: String,
    pub cache: String,
    pub downloads: String,
    pub logs: String,
    pub database: String,
}

#[tauri::command]
pub async fn app_paths(state: State<'_, AppState>) -> AppResult<PathsReport> {
    let paths = &state.paths;
    Ok(PathsReport {
        root: paths.root.to_string_lossy().into_owned(),
        instances: paths.instances.to_string_lossy().into_owned(),
        shared: paths.shared.to_string_lossy().into_owned(),
        java: paths.java.to_string_lossy().into_owned(),
        cache: paths.cache.to_string_lossy().into_owned(),
        downloads: paths.downloads.to_string_lossy().into_owned(),
        logs: paths.logs.to_string_lossy().into_owned(),
        database: paths.database_file.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> AppResult<AppSettings> {
    Ok(state.settings())
}

/// Persist settings and rebuild dependent managers.
#[tauri::command]
pub async fn settings_update(
    settings: AppSettings,
    state: State<'_, AppState>,
) -> AppResult<AppSettings> {
    state.apply_settings(settings).await
}

/// What a Redis probe learned, for the Settings → Network test button.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedisProbe {
    pub ok: bool,
    pub online_players: u64,
    pub message: Option<String>,
}

/// Verify the directory endpoint before the user commits to it.
///
/// `mqtt://host:port` URLs (the embedded directory default) are probed with a
/// real MQTT connect; anything else goes to the legacy Redis path.
#[tauri::command]
pub async fn settings_test_redis(url: String) -> AppResult<RedisProbe> {
    if let Some(rest) = url.strip_prefix("mqtt://") {
        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse::<u16>()
                    .unwrap_or(crate::network::mqtt::DEFAULT_MQTT_PORT),
            ),
            None => (rest.to_string(), crate::network::mqtt::DEFAULT_MQTT_PORT),
        };
        return match crate::network::mqtt::MqttDirectory::connect(&host, port).await {
            Ok(directory) => {
                let players = crate::network::Directory::online_players(&directory)
                    .await
                    .unwrap_or_default();
                Ok(RedisProbe {
                    ok: true,
                    online_players: players,
                    message: Some(format!("connected to {host}:{port}")),
                })
            }
            Err(err) => Ok(RedisProbe {
                ok: false,
                online_players: 0,
                message: Some(err.to_string()),
            }),
        };
    }

    match crate::network::RedisDirectory::connect(&url).await {
        Ok(directory) => {
            let players = directory.online_players().await.unwrap_or_default();
            Ok(RedisProbe {
                ok: true,
                online_players: players,
                message: None,
            })
        }
        Err(err) => Ok(RedisProbe {
            ok: false,
            online_players: 0,
            message: Some(err.to_string()),
        }),
    }
}

/// Byte counts for the Storage section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub download_bytes: u64,
    pub metadata_rows: usize,
    pub instance_bytes: u64,
}

#[tauri::command]
pub async fn cache_stats(state: State<'_, AppState>) -> AppResult<CacheStats> {
    let metadata_rows = state
        .db
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM metadata_cache", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|err| crate::error::AppError::Database(err.to_string()))
        })
        .unwrap_or(0) as usize;
    Ok(CacheStats {
        download_bytes: crate::mods::downloader::directory_size(&state.paths.downloads),
        metadata_rows,
        instance_bytes: crate::mods::downloader::directory_size(&state.paths.instances),
    })
}

/// Clear caches. `downloads` removes cached mod/asset files, `metadata` drops
/// the SQLite key/value cache rows.
#[tauri::command]
pub async fn cache_clear(
    downloads: bool,
    metadata: bool,
    state: State<'_, AppState>,
) -> AppResult<CacheStats> {
    if downloads {
        // Best-effort: remove the cached download files but keep the folder.
        let mut entries = tokio::fs::read_dir(&state.paths.downloads).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                let _ = tokio::fs::remove_dir_all(&path).await;
            } else {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }
    if metadata {
        state.db.cache_clear()?;
    }
    cache_stats(state).await
}

/// Last N lines of a named log file (diagnostics pane).
#[tauri::command]
pub async fn log_tail(
    name: String,
    lines: Option<usize>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let path = state.paths.log_file(&name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = tokio::fs::read_to_string(&path).await?;
    let wanted = lines.unwrap_or(200);
    let collected: Vec<String> = content
        .lines()
        .rev()
        .take(wanted)
        .map(|line| line.to_string())
        .collect();
    let mut lines_out = collected;
    lines_out.reverse();
    Ok(lines_out)
}

// --- updater -------------------------------------------------------------

/// What a feed check found, as the Updates section renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Semver advertised by the release feed.
    pub version: String,
    /// Release notes from the feed (may be empty).
    pub notes: String,
    /// `true` when the feed version is newer than the running app.
    pub update_available: bool,
    /// Semver of the running launcher, echoed for display.
    pub current_version: String,
}

impl UpdateInfo {
    fn from_update(update: &tauri_plugin_updater::Update, current: &str) -> Self {
        Self {
            version: update.version.clone(),
            notes: update.body.clone().unwrap_or_default(),
            update_available: true,
            current_version: current.to_string(),
        }
    }
}

/// Check the release feed. Returns "no update" as data (not an error) so the UI
/// can show an up-to-date badge without handling an exception path.
#[tauri::command]
pub async fn updater_check(app: AppHandle) -> AppResult<UpdateInfo> {
    let current = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let updater = app
        .updater()
        .map_err(|err| AppError::Other(format!("updater unavailable: {err}")))?;
    match updater.check().await {
        Ok(Some(update)) => {
            // Park the descriptor so `updater_download` can pick it up.
            {
                let mut pending = pending_update(&app);
                pending.checked = Some(update.clone());
                pending.downloaded = None;
            }
            Ok(UpdateInfo::from_update(&update, &current))
        }
        Ok(None) => Ok(UpdateInfo {
            version: current.clone(),
            notes: String::new(),
            update_available: false,
            current_version: current,
        }),
        // Feed unreachable / offline: a check failing is normal (air-gapped
        // machines, firewalls), so surface it as data instead of a toast-level
        // error the user must dismiss.
        Err(err) => Ok(UpdateInfo {
            version: current.clone(),
            notes: err.to_string(),
            update_available: false,
            current_version: current,
        }),
    }
}

/// Download the previously found update, emitting `updater://progress` events.
///
/// The plugin verifies the minisign signature against the bundled public key
/// before the bytes are accepted; a bad signature aborts here as an error.
#[tauri::command]
pub async fn updater_download(app: AppHandle) -> AppResult<()> {
    let update = take_pending_check(&app).ok_or_else(|| {
        AppError::Other("run a check first — there is no update pending".to_string())
    })?;

    let handle = app.clone();
    let bytes = update
        .download(
            &mut |chunk, total| {
                let _ = handle.emit(
                    UPDATE_PROGRESS_EVENT,
                    serde_json::json!({ "chunkBytes": chunk, "totalBytes": total, "done": false }),
                );
            },
            || {
                // Called when the download finished and the signature verified.
                let _ = handle.emit(
                    UPDATE_PROGRESS_EVENT,
                    serde_json::json!({ "chunkBytes": 0, "totalBytes": 0, "done": true }),
                );
            },
        )
        .await
        .map_err(|err| AppError::Other(format!("update download failed: {err}")))?;

    // Park the verified package for `updater_install`.
    {
        let mut pending = pending_update(&app);
        pending.checked = None;
        pending.downloaded = Some((update, bytes));
    }
    Ok(())
}

/// Restart the launcher to apply the downloaded update.
///
/// On Windows (NSIS/MSI) the plugin runs the downloaded installer and exits
/// this process; the new launcher comes up in its place. On macOS/Linux the
/// caller relaunches manually, so this returns normally there.
#[tauri::command]
pub async fn updater_install(app: AppHandle) -> AppResult<()> {
    let Some((update, bytes)) = pending_update(&app).downloaded.take() else {
        return Err(AppError::Other(
            "no downloaded update to install — download it first".to_string(),
        ));
    };
    update
        .install(&bytes)
        .map_err(|err| AppError::Other(format!("update install failed: {err}")))?;
    Ok(())
}

/// Parked updater state between the three IPC steps: the descriptor found by
/// `updater_check`, then the signature-verified package bytes.
#[derive(Default)]
struct PendingUpdate {
    checked: Option<tauri_plugin_updater::Update>,
    downloaded: Option<(tauri_plugin_updater::Update, Vec<u8>)>,
}

/// One lazily-initialized slot per process: updater state outlives any single
/// command, and `State<>` injection would need it managed at setup anyway.
fn pending_update(
    app: &AppHandle,
) -> std::sync::MutexGuard<'static, PendingUpdate> {
    // Silence the unused-app warning; the slot is process-global by design.
    let _ = app;
    static SLOT: std::sync::OnceLock<std::sync::Mutex<PendingUpdate>> = std::sync::OnceLock::new();
    SLOT.get_or_init(Default::default).lock().unwrap()
}

fn take_pending_check(app: &AppHandle) -> Option<tauri_plugin_updater::Update> {
    pending_update(app).checked.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_info_serializes_camel_case_for_the_frontend() {
        let info = UpdateInfo {
            version: "0.2.0".to_string(),
            notes: "changes".to_string(),
            update_available: true,
            current_version: "0.1.0".to_string(),
        };
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["updateAvailable"], true);
        assert_eq!(json["currentVersion"], "0.1.0");
        assert_eq!(json["notes"], "changes");
    }

    #[test]
    fn the_progress_event_name_is_stable() {
        // The frontend subscribes to exactly this name.
        assert_eq!(UPDATE_PROGRESS_EVENT, "updater://progress");
    }
}
