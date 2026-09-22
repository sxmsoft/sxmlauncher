//! Instance commands: CRUD, install, launch, mod toggles.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::commands::{GameState, GameStateEvent, LaunchReport, SessionEventPayload};
use crate::error::{AppError, AppResult};
use crate::instances::{launch, LaunchExtras, LaunchPlanner};
use crate::models::instance::{CreateInstanceRequest, Instance, UpdateInstanceRequest};
use crate::state::{sink_for, AppState, RunningGameHandle, SESSION_EVENT, GAME_EVENT};
use crate::store::instances::InstalledModRow;

/// Optional P2P wiring for a launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    /// `host:port` of a local bridge to join straight into a world.
    #[serde(default)]
    pub connect: Option<String>,
    /// Launch a hosted world and publish it (handled by `host_start`).
    #[serde(default)]
    pub start_host: bool,
    /// Quick-play a specific singleplayer world.
    #[serde(default)]
    pub quick_play_world: Option<String>,
}

#[tauri::command]
pub async fn instance_list(state: State<'_, AppState>) -> AppResult<Vec<Instance>> {
    state.instances().list().await
}

#[tauri::command]
pub async fn instance_get(id: Uuid, state: State<'_, AppState>) -> AppResult<Instance> {
    state.instances().get(id).await
}

#[tauri::command]
pub async fn instance_create(
    request: CreateInstanceRequest,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    state.instances().create(request, sink_for(&app)).await
}

#[tauri::command]
pub async fn instance_update(
    request: UpdateInstanceRequest,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    state.instances().update(request).await
}

#[tauri::command]
pub async fn instance_delete(
    id: Uuid,
    delete_files: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.instances().delete(id, delete_files).await
}

#[tauri::command]
pub async fn instance_duplicate(
    id: Uuid,
    new_name: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    state.instances().duplicate(id, new_name).await
}

#[tauri::command]
pub async fn instance_install(
    id: Uuid,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    let manager = state.instances();
    let instance = manager.get(id).await?;
    manager.install(instance, sink_for(&app)).await
}

#[tauri::command]
pub async fn instance_refresh(id: Uuid, state: State<'_, AppState>) -> AppResult<Instance> {
    state.instances().refresh(id).await
}

#[tauri::command]
pub async fn instance_mods(
    id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<Vec<InstalledModRow>> {
    state.db.list_installed_mods(id)
}

#[tauri::command]
pub async fn instance_toggle_mod(
    id: Uuid,
    project_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.db.set_mod_enabled(id, &project_id, enabled)
}

#[tauri::command]
pub async fn instance_running(state: State<'_, AppState>) -> AppResult<Vec<GameStateEvent>> {
    Ok(state
        .running_games()
        .into_iter()
        .map(|(instance_id, handle)| GameStateEvent {
            instance_id,
            state: GameState::Running,
            pid: Some(handle.game.pid),
            connect_address: handle
                .game
                .local_port
                .map(|port| format!("127.0.0.1:{port}")),
            message: None,
        })
        .collect())
}

/// Install if needed, then start the game.
#[tauri::command]
pub async fn instance_launch(
    id: Uuid,
    options: Option<LaunchOptions>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<LaunchReport> {
    let options = options.unwrap_or_default();
    let manager = state.instances();

    // 1. Refuse to launch twice.
    if state.running.contains_key(&id) {
        return Err(AppError::Config(
            "that instance is already running".to_string(),
        ));
    }

    // 2. Make sure the game files exist (install is idempotent and cached).
    let instance = manager.get(id).await?;
    let instance = if instance.status == crate::models::instance::InstanceStatus::Ready {
        instance
    } else {
        manager.install(instance, sink_for(&app)).await?
    };

    // 3. Identity.
    let accounts = state.accounts();
    let identity = match accounts.active_identity().await {
        Ok(identity) => identity,
        Err(AppError::Unauthorized) => {
            return Err(AppError::Account(
                "sign in before launching, or pick an offline profile".to_string(),
            ))
        }
        Err(err) => return Err(err),
    };

    // 4. Version metadata + Java runtime.
    let version = state.resolve_local_version(&instance.config.resolved_version_id())?;
    // Ely.by (and any Yggdrasil) session needs the authlib-injector agent, which
    // is fetched once on demand. Doing it here means the launch fails with a
    // readable error instead of a session error inside the game.
    if identity.authlib_url.is_some() {
        state
            .mods()
            .ensure_authlib_injector(sink_for(&app))
            .await?;
    }
    let runtimes = state.mods().java().detect().await.unwrap_or_default();
    let planner = LaunchPlanner::new(state.paths.clone(), runtimes);

    let connect: Option<SocketAddr> = match options.connect.as_deref() {
        Some(address) => Some(address.parse().map_err(|err| {
            AppError::Config(format!("`{address}` is not a valid host:port: {err}"))
        })?),
        None => None,
    };

    let extras = LaunchExtras {
        connect,
        quick_play_world: options.quick_play_world.clone(),
        ..LaunchExtras::default()
    };

    let plan = planner.build(&instance, &version, &identity, &extras)?;
    let local_port = connect.map(|address| address.port());

    let _ = app.emit(
        GAME_EVENT,
        GameStateEvent {
            instance_id: id,
            state: GameState::Starting,
            pid: None,
            connect_address: options.connect.clone(),
            message: Some(format!("launching {}", instance.config.name)),
        },
    );

    // 5. Spawn.
    let (game, child) = launch::spawn(&plan, id, local_port).await?;
    state.db.record_launch(id)?;

    let handle = Arc::new(RunningGameHandle {
        game: game.clone(),
        plan: plan.clone(),
        child: Arc::new(AsyncMutex::new(child)),
        session_id: None,
    });
    state.running.insert(id, handle.clone());

    let _ = app.emit(
        GAME_EVENT,
        GameStateEvent {
            instance_id: id,
            state: GameState::Running,
            pid: Some(game.pid),
            connect_address: options.connect.clone(),
            message: None,
        },
    );

    // 6. Watch for exit on a background task: playtime accounting must survive
    //    the UI being closed. The same loop tails the game log, because that is
    //    how "Open to LAN" ports are discovered: Minecraft prints
    //    `Started serving on <port>` and the launcher publishes the world on the
    //    network without any user configuration.
    let watcher_state = app.clone();
    let log_file = plan.log_file.clone();
    let settings = state.settings();
    let instance_name = instance.config.name.clone();
    let instance_version = instance.config.game_version.clone();
    let instance_loader = instance.config.loader.kind.as_str().to_string();
    let lan_watch = settings.lan_discovery && settings.lan_auto_detect;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        // Only the bytes written after the launch matter, so a line from a
        // previous run can never publish a stale port.
        let mut log_offset = tokio::fs::metadata(&log_file)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        let mut published_lan: Option<Uuid> = None;
        loop {
            ticker.tick().await;
            let exited = {
                let mut child = handle.child.lock().await;
                child.try_wait().ok().flatten()
            };

            if lan_watch && exited.is_none() {
                if let Ok(tail) = read_log_tail(&log_file, &mut log_offset).await {
                    if let Some(port) = open_lan_port(&tail) {
                        if let Some(host) = announce_lan_world(
                            &watcher_state,
                            id,
                            &instance_name,
                            &instance_version,
                            &instance_loader,
                            port,
                        )
                        .await
                        {
                            published_lan = Some(host.id);
                        }
                    }
                }
            }

            if let Some(status) = exited {
                if let Some(state) = watcher_state.try_state::<AppState>() {
                    if let Some(lan_id) = published_lan {
                        state.lan().unpublish(lan_id);
                    }
                }
                if let Some(state) = watcher_state.try_state::<AppState>() {
                    let seconds = (chrono::Utc::now() - game.started_at)
                        .num_seconds()
                        .max(0) as u64;
                    let _ = state.db.record_playtime(id, seconds);
                    state.running.remove(&id);
                    let _ = watcher_state.emit(
                        GAME_EVENT,
                        GameStateEvent {
                            instance_id: id,
                            state: if status.success() {
                                GameState::Exited
                            } else {
                                GameState::Crashed
                            },
                            pid: Some(game.pid),
                            connect_address: None,
                            message: status.code().map(|code| format!("exit code {code}")),
                        },
                    );
                }
                break;
            }
        }
    });

    Ok(LaunchReport {
        instance,
        pid: game.pid,
        connect_address: options.connect,
        session_id: None,
        command_preview: plan.redacted_command_line(),
    })
}

/// Stop a running game (graceful, then forced after a short grace period).
#[tauri::command]
pub async fn instance_kill(
    id: Uuid,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let Some((_, handle)) = state.running.remove(&id) else {
        return Ok(false);
    };

    {
        let mut child = handle.child.lock().await;
        let _ = child.start_kill();
    }

    // Escalate if the JVM ignores the first signal.
    let killer = handle.clone();
    tokio::spawn(async move {
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut child = killer.child.lock().await;
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    let _ = child.start_kill();
                }
                Err(_) => return,
            }
        }
    });

    let _ = app.emit(
        GAME_EVENT,
        GameStateEvent {
            instance_id: id,
            state: GameState::Exited,
            pid: Some(handle.game.pid),
            connect_address: None,
            message: Some("stopped by the player".to_string()),
        },
    );
    Ok(true)
}

/// Import an instance folder (drag & drop or file picker).
#[tauri::command]
pub async fn instance_import(
    folder: String,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    state.instances().import(std::path::PathBuf::from(folder)).await
}

/// Release versions available for a new instance.
#[tauri::command]
pub async fn version_list(
    releases_only: Option<bool>,
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::models::version::ManifestEntry>> {
    let client = crate::instances::MojangClient::new(
        crate::mods::modrinth::http_client()?,
        Some(state.db.clone()),
    );
    let mut versions = client.version_manifest().await?.versions;
    if releases_only.unwrap_or(true) {
        versions.retain(|entry| entry.release_type == "release");
    }
    Ok(versions)
}

/// Bytes appended to the game log since the previous poll.
///
/// Reading forward from a stored offset keeps the tail scan O(new bytes) no
/// matter how long the session runs.
async fn read_log_tail(path: &Path, offset: &mut u64) -> AppResult<String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let Ok(metadata) = tokio::fs::metadata(path).await else {
        // The JVM creates the file lazily; nothing to read yet.
        return Ok(String::new());
    };
    let length = metadata.len();
    if length <= *offset {
        return Ok(String::new());
    }
    let start = *offset;
    *offset = length;

    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut buffer = String::new();
    file.take(length - start).read_to_string(&mut buffer).await?;
    Ok(buffer)
}

/// The port Minecraft printed after "Open to LAN".
///
/// Vanilla logs `Started serving on 51234` for a singleplayer LAN world and
/// `Starting Minecraft server on *:25565` for dedicated servers; both are
/// recognised so hosting an external server works the same way.
fn open_lan_port(tail: &str) -> Option<u16> {
    static LAN_PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = LAN_PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?:Started serving on\s*|Starting Minecraft server on \*?:)(\d{2,5})",
        )
        .expect("the LAN port pattern is a valid regex")
    });

    // Scan from the end: the newest line wins if the player opens twice.
    for line in tail.lines().rev() {
        if let Some(capture) = pattern.captures(line) {
            if let Some(port) = capture
                .get(1)
                .and_then(|value| value.as_str().parse::<u16>().ok())
            {
                if port > 0 {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// Publish a world discovered in the game log on the local network.
///
/// Re-publishing is idempotent: an announcement already exists for the instance,
/// so this returns it instead of spawning a second beacon loop.
async fn announce_lan_world(
    app: &tauri::AppHandle,
    instance_id: Uuid,
    name: &str,
    game_version: &str,
    loader: &str,
    port: u16,
) -> Option<crate::network::localnet::LanHost> {
    use tauri::Manager;

    let state = app.try_state::<crate::state::AppState>()?;
    if let Some(existing) = state.lan().host_for_instance(instance_id) {
        return Some(existing);
    }

    let beacon = crate::network::localnet::LanBeacon {
        magic: crate::network::localnet::MAGIC.to_string(),
        query: false,
        id: Uuid::new_v5(&Uuid::NAMESPACE_OID, instance_id.as_bytes()),
        name: name.to_string(),
        motd: format!("{name} · Open to LAN"),
        host: state
            .accounts()
            .active_account()
            .await
            .ok()
            .flatten()
            .map(|account| account.username)
            .unwrap_or_default(),
        instance_id: Some(instance_id),
        world_name: None,
        game_version: game_version.to_string(),
        loader: loader.to_string(),
        players: 1,
        max_players: state.settings().max_hosted_players.max(2),
        port,
        password_protected: false,
        protocol_version: crate::network::session::protocol_version_for(game_version),
        tags: vec!["lan".to_string()],
        ttl_secs: 9,
    };

    match state.lan().publish(beacon, true).await {
        Ok(host) => {
            let _ = app.emit(
                SESSION_EVENT,
                SessionEventPayload {
                    id: host.id,
                    kind: "lan_open".to_string(),
                    peer_id: None,
                    username: None,
                    players: None,
                    message: Some(format!("{}:{}", host.address, host.port)),
                },
            );
            Some(host)
        }
        Err(err) => {
            eprintln!("[lan] could not announce the world: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_lan_port_reads_the_vanilla_log_line() {
        let log = "[12:00:01] [Server thread/INFO]: Started serving on 51234";
        assert_eq!(open_lan_port(log), Some(51234));
    }

    #[test]
    fn open_lan_port_reads_dedicated_server_lines() {
        assert_eq!(
            open_lan_port("Starting Minecraft server on *:25565"),
            Some(25565)
        );
    }

    #[test]
    fn open_lan_port_ignores_ordinary_lines() {
        assert_eq!(open_lan_port("Player joined the game\nPreparing level \"world\""), None);
        assert_eq!(open_lan_port("Started serving on 0"), None);
        // The newest match wins.
        let two = "Started serving on 1000\nStarted serving on 2000";
        assert_eq!(open_lan_port(two), Some(2000));
    }
}
