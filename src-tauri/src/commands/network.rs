//! P2P hosting / joining / browsing commands.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

use crate::commands::{parse_uuid, GuestStatus, HostStatus, JoinStatus, NetworkStatus};
use crate::error::{AppError, AppResult};
use crate::models::server::{
    JoinRejection, ServerFilter, ServerListingSummary, WhitelistPolicy,
};
use crate::network::session::JoinTarget;
use crate::network::{bridge, code::ConnectCode, holepunch, CodeFlags, HostOptions};
use crate::state::{sink_for, AppState, SESSION_EVENT};

/// Request body for `host_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRequest {
    pub instance_id: Option<Uuid>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub motd: Option<String>,
    #[serde(default)]
    pub icon_base64: Option<String>,
    #[serde(default)]
    pub max_players: Option<u32>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub whitelist: Option<WhitelistPolicy>,
    #[serde(default)]
    pub public: Option<bool>,
    #[serde(default)]
    pub world_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Local Minecraft server to expose (defaults to 127.0.0.1:25565).
    #[serde(default)]
    pub local_port: Option<u16>,
    #[serde(default)]
    pub force_relay: bool,
}

/// Browse the Redis directory.
#[tauri::command]
pub async fn server_browse(
    filter: Option<ServerFilter>,
    state: State<'_, AppState>,
) -> AppResult<Vec<ServerListingSummary>> {
    let manager = state.network()?;
    let filter = filter.unwrap_or_default();
    let results = manager.browse(&filter).await?;

    // Cache what we saw so the browser works offline.
    for summary in &results {
        if let Ok(Some(listing)) = manager.directory().listing(summary.id).await {
            let _ = state.db.cache_server_listing(&listing);
        }
    }
    Ok(results)
}

/// Measure latency to a listing.
///
/// Relayed sessions return `None`: their latency is a property of the relay, not
/// of the host, so a per-host badge would be misleading.
#[tauri::command]
pub async fn server_ping(id: Uuid, state: State<'_, AppState>) -> AppResult<Option<u32>> {
    let manager = state.network()?;
    let Some(listing) = manager.directory().listing(id).await? else {
        return Ok(None);
    };
    Ok(manager.probe(&listing.connection).await)
}

/// Cached (offline) servers, favourites first.
#[tauri::command]
pub async fn server_favorites(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::store::servers::CachedServer>> {
    state.db.cached_servers()
}

#[tauri::command]
pub async fn server_set_favorite(
    id: Uuid,
    favorite: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    // `false` means the listing was never cached (favouriting straight from a
    // browse result before the cache write landed). That is not an error.
    state.db.set_server_favorite(id, favorite)?;
    Ok(())
}

/// What the header pill / Settings show as the directory endpoint.
fn directory_label(settings: &crate::config::AppSettings) -> String {
    if settings.mqtt_broker.trim().is_empty() {
        settings.redis_url.clone()
    } else {
        format!("mqtt://{}/{}", settings.mqtt_broker.trim(), settings.mqtt_port)
    }
}

/// Directory connection status for the header pill.
#[tauri::command]
pub async fn network_status(state: State<'_, AppState>) -> AppResult<NetworkStatus> {
    let settings = state.settings();
    let lan = state.lan();
    let lan_worlds = lan.worlds().len();
    let lan_hosts = lan.hosts().len();
    let Some(manager) = state.network_optional() else {
        return Ok(NetworkStatus {
            directory_connected: false,
            directory_url: directory_label(&settings),
            online_players: 0,
            active_hosts: lan_hosts,
            active_guests: 0,
            relay_configured: settings.relay_url.is_some(),
            message: Some(
                "LAN mode: local worlds work with no server. Enable the global directory in \
                 Settings → Network for friends outside your network."
                    .to_string(),
            ),
            lan_enabled: settings.lan_discovery,
            lan_port: settings.lan_port,
            lan_worlds,
        });
    };

    let online_players = manager
        .directory()
        .online_players()
        .await
        .unwrap_or_default();

    Ok(NetworkStatus {
        directory_connected: true,
        directory_url: directory_label(&settings),
        online_players,
        active_hosts: manager.active_hosts().len(),
        active_guests: manager.active_guests().len(),
        relay_configured: settings.relay_url.is_some(),
        message: None,
        lan_enabled: settings.lan_discovery,
        lan_port: settings.lan_port,
        lan_worlds,
    })
}

/// Request body for `lan_host_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanHostRequest {
    /// Display name friends see (defaults to the instance name).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub motd: String,
    /// Port the world listens on. The launcher fills this in automatically when
    /// the player uses "Open to LAN" in game.
    pub port: u16,
    #[serde(default)]
    pub instance_id: Option<Uuid>,
    #[serde(default)]
    pub world_name: Option<String>,
    #[serde(default)]
    pub game_version: String,
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub players: u32,
    #[serde(default)]
    pub max_players: u32,
    #[serde(default)]
    pub password_protected: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Everything the LAN tab needs in one round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanReport {
    /// `false` when the user turned LAN discovery off in Settings.
    pub enabled: bool,
    /// UDP port used for beacons.
    pub port: u16,
    /// Worlds other machines on this network are serving.
    pub worlds: Vec<crate::network::localnet::LanWorld>,
    /// Worlds this machine is announcing.
    pub hosting: Vec<crate::network::localnet::LanHost>,
    /// Ready-to-share instruction line for the UI.
    pub hint: String,
}

/// Browse the local network.
///
/// This is the "no server required" path: it broadcasts a probe and collects the
/// answers, so the list is live without any directory service being configured.
#[tauri::command]
pub async fn lan_browse(
    timeout_ms: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<LanReport> {
    let settings = state.settings();
    let lan = state.lan();
    if !settings.lan_discovery {
        return Ok(LanReport {
            enabled: false,
            port: settings.lan_port,
            worlds: Vec::new(),
            hosting: lan.hosts(),
            hint: "LAN discovery is turned off in Settings → Network.".to_string(),
        });
    }

    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(900).clamp(150, 4000));
    let worlds = lan.browse(timeout).await?;
    let hosting = lan.hosts();

    Ok(LanReport {
        enabled: true,
        port: lan.port(),
        worlds,
        hosting,
        hint: format!(
            "Beacons on UDP {}. Same Wi-Fi only — no port forwarding and no server needed.",
            lan.port()
        ),
    })
}

/// Worlds already heard, without sending a new probe (used for polling).
#[tauri::command]
pub async fn lan_worlds(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::network::localnet::LanWorld>> {
    Ok(state.lan().worlds())
}

/// Announce a world on the local network.
#[tauri::command]
pub async fn lan_host_start(
    request: LanHostRequest,
    state: State<'_, AppState>,
) -> AppResult<crate::network::localnet::LanHost> {
    if request.port == 0 {
        return Err(AppError::Config(
            "the world's port is unknown. Open the world to LAN in game — the launcher detects \
             the port automatically — or type it by hand."
                .to_string(),
        ));
    }
    let settings = state.settings();
    if !settings.lan_discovery {
        return Err(AppError::Config(
            "LAN discovery is turned off in Settings → Network.".to_string(),
        ));
    }

    let lan = state.lan();
    let beacon = crate::network::localnet::LanBeacon {
        magic: crate::network::localnet::MAGIC.to_string(),
        query: false,
        id: request
            .instance_id
            .map(|id| Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes()))
            .unwrap_or_else(Uuid::new_v4),
        name: if request.name.trim().is_empty() {
            "Minecraft world".to_string()
        } else {
            request.name.clone()
        },
        motd: request.motd.clone(),
        host: state
            .accounts()
            .active_account()
            .await
            .ok()
            .flatten()
            .map(|account| account.username)
            .unwrap_or_default(),
        instance_id: request.instance_id,
        world_name: request.world_name.clone(),
        game_version: request.game_version.clone(),
        loader: request.loader.clone(),
        players: request.players,
        max_players: request.max_players,
        port: request.port,
        password_protected: request.password_protected,
        protocol_version: crate::network::session::protocol_version_for(&request.game_version),
        tags: request.tags.clone(),
        ttl_secs: 9,
    };

    lan.publish(beacon, false).await
}

/// Stop announcing a world.
#[tauri::command]
pub async fn lan_host_stop(id: Uuid, state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.lan().unpublish(id))
}

/// Announcement attached to an instance, if any.
#[tauri::command]
pub async fn lan_host_for_instance(
    instance_id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<Option<crate::network::localnet::LanHost>> {
    Ok(state.lan().host_for_instance(instance_id))
}

/// Address friends on the same network should use for a hosted world.
#[tauri::command]
pub async fn lan_address(state: State<'_, AppState>) -> AppResult<String> {
    Ok(format!(
        "{}:{}",
        crate::network::localnet::local_address(),
        state.lan().port()
    ))
}

/// One-click hosting: pick an instance → Host → launcher starts the game
/// server, waits until a port is listening, then exposes a join code / P2P.
#[tauri::command]
pub async fn host_start(
    request: HostRequest,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<HostStatus> {
    let instance_id = request.instance_id.ok_or_else(|| {
        AppError::Config(
            "pick an instance before hosting — the launcher starts that \
             instance's server and publishes a join code"
                .to_string(),
        )
    })?;

    let manager = state.network_for_hosting().await?;
    let settings = state.settings();
    let sink = sink_for(&app);

    // 1. Ensure the instance is installed and a local Minecraft server is up.
    let mut instance = state.instances().get(instance_id).await?;
    if instance.status != crate::models::instance::InstanceStatus::Ready {
        instance = state
            .instances()
            .install(instance, sink.clone())
            .await?;
    }
    let version = state.resolve_local_version(&instance.config.resolved_version_id())?;
    let java = state
        .mods()
        .ensure_java(instance.required_java_major, sink.clone())
        .await?;

    let (local_server, hosted) = crate::instances::resolve_or_start_host(
        &instance,
        &version,
        &state.mods(),
        &java,
        request.local_port,
        sink.clone(),
    )
    .await?;

    // 2. Publish P2P session against the live local port.
    let mut options = HostOptions {
        name: request.name.clone(),
        description: request.description.clone(),
        motd: request
            .motd
            .clone()
            .unwrap_or_else(|| format!("{} · hosted with SXMLAUNCHER", request.name)),
        icon_bytes: match &request.icon_base64 {
            Some(encoded) => Some(crate::network::icon::decode_icon(encoded)?),
            None => None,
        },
        max_players: request.max_players.unwrap_or(settings.max_hosted_players),
        password: request.password.clone(),
        whitelist: request.whitelist.clone().unwrap_or_default(),
        public: request.public.unwrap_or(settings.share_by_default),
        world_name: request.world_name.clone(),
        tags: request.tags.clone(),
        force_relay: request.force_relay,
        local_server,
        game_version: instance.config.game_version.clone(),
        loader: instance.config.loader.clone(),
        modpack: instance.config.source_pack.as_ref().map(|pack| {
            crate::models::server::ModpackRef {
                source: pack.source,
                project_id: pack.project_id.clone(),
                version_id: pack.version_id.clone(),
                name: pack.name.clone(),
                version_number: pack.version_number.clone(),
            }
        }),
        required_mod_ids: state
            .db
            .list_installed_mods(instance_id)?
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.project_id.clone())
            .collect(),
        ..HostOptions::default()
    };
    // Keep tags informative for the browser.
    if !options.tags.iter().any(|tag| tag == "sxm-host") {
        options.tags.push("sxm-host".into());
    }

    let session = match manager.host_world(options, sink).await {
        Ok(session) => session,
        Err(err) => {
            if let Some(hosted) = hosted {
                let mut child = hosted.child.lock().await;
                let _ = child.start_kill();
            }
            return Err(err);
        }
    };

    if let Some(hosted) = hosted {
        state
            .hosted_servers
            .insert(session.id, std::sync::Arc::new(hosted));
    }

    // 3. Launch the host's own client into the local server so they can play.
    let connect = format!("{}:{}", local_server.ip(), local_server.port());
    if let Err(err) = crate::commands::instance::instance_launch(
        instance_id,
        Some(crate::commands::instance::LaunchOptions {
            connect: Some(connect),
            ..Default::default()
        }),
        app.clone(),
        state,
    )
    .await
    {
        // Hosting still works if the client launch fails — friends can join.
        eprintln!("[host] client launch after host_start failed: {err}");
    }

    if !session.nat_behavior.is_punchable() {
        let _ = app.emit(
            SESSION_EVENT,
            crate::commands::SessionEventPayload {
                id: session.id,
                kind: "relay_fallback".to_string(),
                peer_id: None,
                username: None,
                players: None,
                message: Some(session.nat_behavior.describe().to_string()),
            },
        );
    }

    status_of(&session)
}

/// Stop hosting and remove the listing immediately.
#[tauri::command]
pub async fn host_stop(id: Uuid, state: State<'_, AppState>) -> AppResult<()> {
    if let Some((_, hosted)) = state.hosted_servers.remove(&id) {
        let mut child = hosted.child.lock().await;
        let _ = child.start_kill();
    }
    if let Ok(manager) = state.network() {
        manager.stop_hosting(id).await?;
    } else if let Some(manager) = state.network_optional() {
        manager.stop_hosting(id).await?;
    }
    Ok(())
}

/// Live status for a hosted world.
#[tauri::command]
pub async fn host_status(id: Uuid, state: State<'_, AppState>) -> AppResult<HostStatus> {
    let manager = state.network()?;
    let session = manager
        .host_session(id)
        .ok_or_else(|| AppError::Directory("that world is not being hosted".to_string()))?;
    status_of(&session)
}

/// Kick a guest.
#[tauri::command]
pub async fn host_kick(
    id: Uuid,
    peer_id: String,
    reason: Option<JoinRejection>,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let manager = state.network()?;
    let session = manager
        .host_session(id)
        .ok_or_else(|| AppError::Directory("that world is not being hosted".to_string()))?;
    Ok(session.kick(&peer_id, reason.unwrap_or(JoinRejection::Banned)))
}

/// Join by typing a share code.
#[tauri::command]
pub async fn join_code(
    code: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<JoinStatus> {
    // Validate locally first so a typo fails instantly with a clear message.
    let _ = ConnectCode::parse(&code)?;
    let manager = state.network()?;
    let session = manager
        .join_world(JoinTarget::Code { code }, sink_for(&app))
        .await?;
    Ok(join_status(&session))
}

/// Join from the server browser.
#[tauri::command]
pub async fn join_server(
    id: Uuid,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<JoinStatus> {
    let manager = state.network()?;
    let session = manager
        .join_world(JoinTarget::ListingId { id }, sink_for(&app))
        .await?;
    Ok(join_status(&session))
}

/// Leave a joined session.
#[tauri::command]
pub async fn leave_session(id: Uuid, state: State<'_, AppState>) -> AppResult<()> {
    state.network()?.leave(id).await
}

/// Share code for an active hosted world.
#[tauri::command]
pub async fn connection_code(id: Uuid, state: State<'_, AppState>) -> AppResult<String> {
    let manager = state.network()?;
    let session = manager
        .host_session(id)
        .ok_or_else(|| AppError::Directory("that world is not being hosted".to_string()))?;
    Ok(session.share_code.clone())
}

/// Local round trip to a bridge address (diagnostics pane).
#[tauri::command]
pub async fn local_rtt(address: String) -> AppResult<u32> {
    let parsed = address.parse().map_err(|err| {
        AppError::Config(format!("`{address}` is not a valid address: {err}"))
    })?;
    bridge::measure_tcp_rtt(parsed).await
}

/// Recent join history + relay ratio (connection diagnostics).
#[tauri::command]
pub async fn session_history(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<SessionHistory> {
    let limit = limit.unwrap_or(25);
    Ok(SessionHistory {
        joins: state.db.recent_joins(limit)?,
        p2p: state.db.p2p_session_history(limit)?,
        relay_ratio: state.db.relay_fallback_ratio(limit)?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistory {
    pub joins: Vec<crate::store::servers::JoinHistoryEntry>,
    pub p2p: Vec<crate::store::servers::P2pSessionRecord>,
    /// Share of recent sessions that needed the relay.
    pub relay_ratio: f32,
}

/// NAT behaviour report for the hosting panel ("will hosting work for me?").
#[tauri::command]
pub async fn nat_probe(state: State<'_, AppState>) -> AppResult<NatAdvice> {
    let settings = state.settings();
    let socket = holepunch::bind_punch_socket(0).await?;
    let config = holepunch::PunchConfig::from_servers(&settings.stun_servers);

    match holepunch::StunClient::classify(&socket, &config.stun_servers, config.stun_timeout).await {
        Ok(mapping) => Ok(NatAdvice {
            behavior: mapping.behavior,
            public_address: Some(mapping.address.to_string()),
            can_host_direct: mapping.behavior.is_punchable(),
            message: mapping.behavior.describe().to_string(),
        }),
        Err(err) => Ok(NatAdvice {
            behavior: holepunch::NatBehavior::Unknown,
            public_address: None,
            can_host_direct: false,
            message: format!("could not determine your NAT type: {err}"),
        }),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NatAdvice {
    pub behavior: holepunch::NatBehavior,
    pub public_address: Option<String>,
    pub can_host_direct: bool,
    pub message: String,
}

/// Build the UI-facing status for a host session.
fn status_of(session: &Arc<crate::network::HostSession>) -> AppResult<HostStatus> {
    let summary = session.summary();
    Ok(HostStatus {
        id: session.id,
        share_code: session.share_code.clone(),
        mode: summary.mode,
        nat_advice: session.nat_behavior.describe().to_string(),
        public_endpoint: session.public_endpoint.map(|address| address.to_string()),
        guests: session
            .guests
            .iter()
            .filter(|entry| entry.connected)
            .map(|entry| GuestStatus {
                peer_id: entry.peer_id.clone(),
                address: entry.address.to_string(),
                username: entry.username.clone(),
                protocol_version: entry.protocol_version,
                joined_at: entry.joined_at,
            })
            .collect(),
        summary,
    })
}

/// Build the UI-facing status for a guest session.
fn join_status(session: &Arc<crate::network::GuestSession>) -> JoinStatus {
    let (ip, port) = session.connect_target();
    JoinStatus {
        id: session.id,
        server_name: session.server_name.clone(),
        mode: session.mode,
        local_address: ip,
        local_port: port,
        remote: session.remote.map(|address| address.to_string()),
        rtt_ms: session.rtt_ms,
    }
}

/// Re-exported for the frontend: flags carried by a share code.
pub fn code_flags(code: &str) -> AppResult<CodeFlags> {
    Ok(ConnectCode::parse(code)?.flags)
}

/// Convenience used by tests and by `parse_uuid` consumers.
pub fn parse_listing_id(raw: &str) -> AppResult<Uuid> {
    parse_uuid(raw)
}
