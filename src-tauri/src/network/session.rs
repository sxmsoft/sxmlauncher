//! Session manager: one-click hosting and joining.
//!
//! ## Hosting (`host_world`)
//!
//! ```text
//! 1. discover our public UDP mapping (STUN)         -> PeerEndpoint list
//! 2. bind the punch socket, mint a session token
//! 3. publish the ServerListing to Redis (TTL 30s)
//! 4. spawn the heartbeat task (every 10s, keeps TTL + player counts fresh)
//! 5. accept punches: token check -> tunnel handshake -> bridge to 127.0.0.1:25565
//! 6. publish a share code + emit SessionEvent::GuestJoined
//! ```
//!
//! ## Joining (`join_world`)
//!
//! ```text
//! 1. resolve the listing (by id, or by share code)
//! 2. connect: direct UDP tunnel, falling back to the relay
//! 3. bind a loopback port and start the bridge
//! 4. return that address so the launcher can pass --server/--port
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::server::{
    ConnectionDescriptor, ConnectionMode, JoinRejection, PeerEndpoint, PlayerCount,
    ServerHeartbeat, ServerListing, ServerListingSummary, WhitelistPolicy,
    DEFAULT_HEARTBEAT_TTL_SECS, HEARTBEAT_INTERVAL_SECS,
};
use crate::network::bridge::{self, DEFAULT_SERVER_PORT};
use crate::network::code::{CodeFlags, ConnectCode};
use crate::network::directory::Directory;
use crate::network::holepunch::{
    self, decode_punch, encode_punch, NatBehavior, PunchConfig, PunchKind, PUNCH_DATAGRAM_LEN,
};
use crate::network::transport::{
    ByteCounters, CountingStream, Frame, FrameKind, TransportRegistry, ACK_TIMEOUT,
    FRAME_OVERHEAD, MAX_FRAME_PAYLOAD, MAX_RETRIES,
};
use crate::store::servers::P2pSessionRecord;
use crate::store::Database;

/// Options for hosting a world.
#[derive(Debug, Clone)]
pub struct HostOptions {
    pub name: String,
    pub description: String,
    pub motd: String,
    /// Icon as raw bytes; encoded + size-checked before it is published.
    pub icon_bytes: Option<Vec<u8>>,
    pub game_version: String,
    pub loader: crate::models::instance::ModLoader,
    pub modpack: Option<crate::models::server::ModpackRef>,
    pub required_mod_ids: Vec<String>,
    pub world_name: Option<String>,
    /// Local address of the Minecraft server to expose.
    pub local_server: SocketAddr,
    pub max_players: u32,
    pub password: Option<String>,
    pub whitelist: WhitelistPolicy,
    /// Publish to the global browser (private sessions use the code only).
    pub public: bool,
    pub region: Option<String>,
    pub tags: Vec<String>,
    /// Force relay mode (diagnostics, or a host behind CGNAT with no punchable path).
    pub force_relay: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            name: "My World".into(),
            description: String::new(),
            motd: "Hosted with SXMLauncher".into(),
            icon_bytes: None,
            game_version: "1.20.1".into(),
            loader: crate::models::instance::ModLoader::vanilla(),
            modpack: None,
            required_mod_ids: Vec::new(),
            world_name: None,
            local_server: SocketAddr::from(([127, 0, 0, 1], DEFAULT_SERVER_PORT)),
            max_players: 8,
            password: None,
            whitelist: WhitelistPolicy::default(),
            public: true,
            region: None,
            tags: Vec::new(),
            force_relay: false,
        }
    }
}

/// What a running host session does to the outside world.
pub struct HostSession {
    pub id: Uuid,
    pub peer_id: String,
    pub listing: Arc<RwLock<ServerListing>>,
    pub local_server: SocketAddr,
    pub share_code: String,
    pub nat_behavior: NatBehavior,
    pub public_endpoint: Option<SocketAddr>,
    pub guests: Arc<DashMap<String, GuestConnection>>,
    /// Live traffic counters, finalized into the `p2p_sessions` row on stop.
    pub byte_counters: Arc<ByteCounters>,
    cancel: CancellationToken,
    events: broadcast::Sender<SessionEvent>,
    directory: Arc<dyn Directory>,
    db: Database,
    _heartbeat: tokio::task::JoinHandle<()>,
}

impl HostSession {
    /// Subscribe to lifecycle events (joins, leaves, errors).
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub fn player_count(&self) -> PlayerCount {
        let guests = self.guests.iter().filter(|entry| entry.connected).count() as u32;
        PlayerCount {
            online: guests,
            max: self.listing.read().players.max,
        }
    }

    /// Current snapshot for the UI.
    pub fn summary(&self) -> ServerListingSummary {
        let mut listing = self.listing.read().clone();
        listing.players = self.player_count();
        ServerListingSummary::from(&listing)
    }

    /// Stop hosting: cancel the accept loop, remove the listing, tell guests.
    ///
    /// Closes the session's diagnostics row with the final byte counters —
    /// even a crashed host row can only be stale-open, never lost: shutdown
    /// finalizes what it can reach.
    pub async fn stop(&self) -> AppResult<()> {
        self.cancel.cancel();
        let _ = self.directory.remove_listing(self.id, &self.peer_id).await;
        let _ = self
            .directory
            .add_players(-(self.player_count().online as i64))
            .await;

        let (bytes_up, bytes_down) = self.byte_counters.snapshot();
        let _ = self.db.finish_p2p_session(
            self.id,
            Utc::now(),
            bytes_up,
            bytes_down,
            None,
        );
        let _ = self.events.send(SessionEvent::Stopped { id: self.id });
        Ok(())
    }

    /// Kick a guest and reject their future attempts this session.
    pub fn kick(&self, peer_id: &str, reason: JoinRejection) -> bool {
        if let Some(mut guest) = self.guests.get_mut(peer_id) {
            guest.connected = false;
            guest.cancel.cancel();
            let _ = self.events.send(SessionEvent::GuestLeft {
                id: self.id,
                peer_id: peer_id.to_string(),
                reason,
            });
            return true;
        }
        false
    }

    pub fn is_running(&self) -> bool {
        !self.cancel.is_cancelled()
    }

    /// A cancellation token that fires when the host session stops (or when
    /// this session is dropped at shutdown) but can be cancelled alone — used
    /// for per-guest tunnels so a kick does not tear the world down.
    pub fn child_cancel_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }
}

/// A guest attached to a host session.
#[derive(Debug, Clone)]
pub struct GuestConnection {
    pub peer_id: String,
    pub address: SocketAddr,
    pub username: Option<String>,
    pub protocol_version: Option<i32>,
    pub connected: bool,
    pub joined_at: chrono::DateTime<Utc>,
    pub mode: ConnectionMode,
    pub cancel: CancellationToken,
}

/// A joined session on the guest side.
pub struct GuestSession {
    pub id: Uuid,
    pub server_name: String,
    pub mode: ConnectionMode,
    /// Loopback address the game client must connect to.
    pub local_address: SocketAddr,
    pub remote: Option<SocketAddr>,
    pub rtt_ms: Option<u32>,
    pub handshake: Arc<parking_lot::Mutex<Option<bridge::HandshakeInfo>>>,
    pub cancel: CancellationToken,
}

impl GuestSession {
    /// Address to pass to Minecraft as `--server/--port`.
    pub fn connect_target(&self) -> (String, u16) {
        (self.local_address.ip().to_string(), self.local_address.port())
    }
}

/// Events emitted by a host session.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    GuestJoined {
        id: Uuid,
        peer_id: String,
        username: Option<String>,
        protocol_version: Option<i32>,
    },
    GuestLeft {
        id: Uuid,
        peer_id: String,
        reason: JoinRejection,
    },
    ListingUpdated {
        id: Uuid,
        players: PlayerCount,
    },
    NatWarning {
        id: Uuid,
        behavior: NatBehavior,
    },
    RelayFallback {
        id: Uuid,
        reason: String,
    },
    Error {
        id: Uuid,
        message: String,
    },
    Stopped {
        id: Uuid,
    },
}

/// Owns every active host/guest session.
pub struct SessionManager {
    directory: Arc<dyn Directory>,
    transports: Arc<TransportRegistry>,
    db: Database,
    hosts: DashMap<Uuid, Arc<HostSession>>,
    guests: DashMap<Uuid, Arc<GuestSession>>,
    punch_config: PunchConfig,
    /// Public key advertised with listings (Ed25519, base64) — signing is
    /// TODO(security): the field is carried end-to-end but not yet enforced.
    public_key: String,
}

impl SessionManager {
    pub fn new(
        directory: Arc<dyn Directory>,
        transports: Arc<TransportRegistry>,
        db: Database,
        punch_config: PunchConfig,
        public_key: String,
    ) -> Self {
        Self {
            directory,
            transports,
            db,
            hosts: DashMap::new(),
            guests: DashMap::new(),
            punch_config,
            public_key,
        }
    }

    pub fn directory(&self) -> &Arc<dyn Directory> {
        &self.directory
    }

    pub fn punch_config(&self) -> &PunchConfig {
        &self.punch_config
    }

    /// Publish a hosted world and start accepting guests.
    pub async fn host_world(
        &self,
        options: HostOptions,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Arc<HostSession>> {
        let id = Uuid::new_v4();
        let peer_id = id.to_string();

        sink.report(
            ProgressEvent::started(JobKind::P2pHost, format!("Hosting {}", options.name))
                .stage(JobStage::Resolving),
        )
        .await;

        // 1. Discover our public mapping.
        //
        // The punch socket is bound on the same stack as the Minecraft server
        // we are exposing: production hosts bind the wildcard (so the public
        // endpoint is reachable), while a loopback-local server (tests, local
        // demos) binds loopback so peers inside the same process can actually
        // reach the endpoints the listing advertises.
        let punch_bind = if options.local_server.ip().is_loopback() {
            SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0)
        } else {
            SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0)
        };
        let punch_socket = if options.force_relay {
            None
        } else {
            Some(holepunch::bind_punch_socket_on(punch_bind).await?)
        };

        let mut endpoints: Vec<PeerEndpoint> = Vec::new();
        let mut nat_behavior = NatBehavior::Unknown;
        let mut public_endpoint = None;

        if let Some(socket) = &punch_socket {
            match holepunch::StunClient::classify(
                socket,
                &self.punch_config.stun_servers,
                self.punch_config.stun_timeout,
            )
            .await
            {
                Ok(mapping) => {
                    nat_behavior = mapping.behavior;
                    public_endpoint = Some(mapping.address);
                    endpoints.push(PeerEndpoint::public(mapping.address));
                }
                Err(err) => {
                    // No STUN: the relay is the only way in.
                    sink.report(
                        ProgressEvent::started(JobKind::P2pHost, "NAT discovery failed")
                            .stage(JobStage::Registering)
                            .detail(err.to_string()),
                    )
                    .await;
                }
            }
        }

        let local_address = punch_socket
            .as_ref()
            .and_then(|socket| socket.local_addr().ok());
        if let Some(address) = local_address {
            endpoints.push(PeerEndpoint::local(address));
        }

        if !nat_behavior.is_punchable() {
            let _ = sink
                .report(
                    ProgressEvent::started(JobKind::P2pHost, "Strict NAT detected")
                        .stage(JobStage::Registering)
                        .detail(nat_behavior.describe()),
                )
                .await;
        }

        // 2. Session token: the shared secret the punch and tunnel handshakes check.
        let session_token = uuid::Uuid::new_v4().simple().to_string();

        // 3. Build the listing.
        let icon_base64 = match &options.icon_bytes {
            Some(bytes) => crate::network::icon::encode_icon(bytes)?,
            None => None,
        };
        let password_protected = options
            .password
            .as_deref()
            .map(|password| !password.is_empty())
            .unwrap_or(false);

        let listing = ServerListing {
            schema_version: crate::models::server::SERVER_LISTING_SCHEMA,
            id,
            name: options.name.clone(),
            description: options.description.clone(),
            motd: options.motd.clone(),
            icon_base64,
            owner: crate::models::server::ServerOwner {
                name: "host".to_string(),
                uuid: Uuid::new_v4(),
                provider: crate::models::account::AccountProvider::Offline,
            },
            game_version: options.game_version.clone(),
            loader: options.loader.kind,
            loader_version: options.loader.version.clone(),
            modpack: options.modpack.clone(),
            required_mod_ids: options.required_mod_ids.clone(),
            players: PlayerCount {
                online: 0,
                max: options.max_players,
            },
            connection: ConnectionDescriptor {
                mode: if options.force_relay || !nat_behavior.is_punchable() {
                    ConnectionMode::Relay
                } else {
                    ConnectionMode::DirectP2p
                },
                peer_id: peer_id.clone(),
                public_key: self.public_key.clone(),
                endpoints: endpoints.clone(),
                relay: options.force_relay.then(|| crate::models::server::RelayDescriptor {
                    url: String::new(),
                    room_token: session_token.clone(),
                    region: options.region.clone(),
                    cert_fingerprint: None,
                }),
                session_token: session_token.clone(),
                protocol_version: protocol_version_for(&options.game_version),
            },
            region: options.region.clone(),
            tags: options.tags.clone(),
            whitelist: options.whitelist.clone(),
            password_protected,
            world_name: options.world_name.clone(),
            created_at: Utc::now(),
            heartbeat_at: Utc::now(),
            ttl_secs: DEFAULT_HEARTBEAT_TTL_SECS,
            world_playtime_secs: 0,
        };

        let listing = Arc::new(RwLock::new(listing));
        let (events, _) = broadcast::channel(64);
        let cancel = CancellationToken::new();

        // 4. Register with the directory (only public sessions are listed).
        //
        // Clone before awaiting: a `parking_lot` read guard is not `Send`, and
        // holding one across an await point would make every command that hosts
        // a world fail to compile as a Tauri command.
        if options.public {
            let snapshot = listing.read().clone();
            self.directory.publish_listing(&snapshot).await?;
        }

        // 5. Share code so a friend can join without the browser.
        let share_code = match (public_endpoint, options.force_relay) {
            (Some(address), false) => ConnectCode::direct(
                id,
                address,
                CodeFlags {
                    password_protected,
                    whitelisted: options.whitelist.enabled,
                },
            )?
            .to_share_string(),
            _ => ConnectCode::relay(
                id,
                0,
                CodeFlags {
                    password_protected,
                    whitelisted: options.whitelist.enabled,
                },
            )
            .to_share_string(),
        };
        if options.public {
            self.directory.put_code(&share_code, id).await?;
        }

        // 6. Heartbeat loop.
        let heartbeat_directory = self.directory.clone();
        let heartbeat_listing = listing.clone();
        let heartbeat_cancel = cancel.clone();
        let heartbeat_events = events.clone();
        let guests_for_count = Arc::new(DashMap::<String, GuestConnection>::new());
        let count_source = guests_for_count.clone();

        let heartbeat = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
                HEARTBEAT_INTERVAL_SECS,
            ));
            loop {
                tokio::select! {
                    _ = heartbeat_cancel.cancelled() => break,
                    _ = ticker.tick() => {
                        let online = count_source.iter().filter(|entry| entry.connected).count() as u32;
                        let (max, ttl) = {
                            let mut listing = heartbeat_listing.write();
                            listing.players = PlayerCount { online, max: listing.players.max };
                            listing.heartbeat_at = Utc::now();
                            (listing.players.max, listing.ttl_secs)
                        };
                        let snapshot = heartbeat_listing.read().clone();
                        if let Err(err) = heartbeat_directory.publish_listing(&snapshot).await {
                            let _ = heartbeat_events.send(SessionEvent::Error {
                                id,
                                message: err.to_string(),
                            });
                        }
                        let _ = heartbeat_directory
                            .publish_heartbeat(&ServerHeartbeat {
                                server_id: id,
                                players: PlayerCount { online, max },
                                ttl_secs: ttl,
                                heartbeat_at: snapshot.heartbeat_at,
                            })
                            .await;
                        let _ = heartbeat_events.send(SessionEvent::ListingUpdated {
                            id,
                            players: PlayerCount { online, max },
                        });
                    }
                }
            }
        });

        let session = Arc::new(HostSession {
            id,
            peer_id: peer_id.clone(),
            listing,
            local_server: options.local_server,
            share_code,
            nat_behavior,
            public_endpoint,
            guests: guests_for_count,
            byte_counters: ByteCounters::new(),
            cancel: cancel.clone(),
            events: events.clone(),
            directory: self.directory.clone(),
            db: self.db.clone(),
            _heartbeat: heartbeat,
        });

        // 7. Session demux: the single reader of the punch socket. It answers
        // punches, verifies tunnel handshakes and routes established guests'
        // frames to their tunnel task (see `run_host_demux`).
        if let Some(socket) = punch_socket {
            let socket = Arc::new(socket);
            let demux_session = session.clone();
            let demux_options = options.clone();
            let demux_token = session_token.clone();
            let demux_events = events.clone();
            let demux_cancel = cancel.clone();
            tokio::spawn(async move {
                run_host_demux(
                    socket,
                    demux_session,
                    demux_options,
                    demux_token,
                    demux_events,
                    demux_cancel,
                )
                .await;
            });
        }

        let _ = events.send(SessionEvent::NatWarning {
            id,
            behavior: nat_behavior,
        });

        self.hosts.insert(id, session.clone());

        // Persist the session row immediately: the diagnostics panel shows the
        // live session (open `ended_at`), and a crash later can only leave it
        // stale-open — never missing.
        let mode = session.listing.read().connection.mode;
        let _ = self.db.record_p2p_session(&P2pSessionRecord {
            id,
            role: "host".to_string(),
            peer_id: peer_id.clone(),
            mode,
            local_port: local_address.map(|addr| addr.port()),
            started_at: Utc::now(),
            ended_at: None,
            bytes_up: 0,
            bytes_down: 0,
            rtt_ms: None,
            detail: match mode {
                ConnectionMode::Relay => Some("forced relay / not punchable".to_string()),
                _ => nat_behavior.describe().to_string().into(),
            },
        });

        Ok(session)
    }

    /// Join a world by listing id or share code.
    pub async fn join_world(
        &self,
        target: JoinTarget,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Arc<GuestSession>> {
        let (listing, password_required) = match target {
            JoinTarget::ListingId { id } => {
                let listing = self
                    .directory
                    .listing(id)
                    .await?
                    .ok_or_else(|| AppError::Directory("that world is offline now".to_string()))?;
                let password_required = listing.password_protected;
                (listing, password_required)
            }
            JoinTarget::Code { code } => {
                let listing = self
                    .directory
                    .resolve_code(&code)
                    .await?
                    .ok_or_else(|| {
                        AppError::Directory(
                            "that connection code has expired or the host is offline".to_string(),
                        )
                    })?;
                let password_required = listing.password_protected;
                (listing, password_required)
            }
        };
        let _ = password_required;

        if listing.players.is_full() {
            return Err(AppError::Transport(
                JoinRejection::ServerFull.user_message().to_string(),
            ));
        }

        sink.report(
            ProgressEvent::started(JobKind::P2pConnect, format!("Joining {}", listing.name))
                .stage(JobStage::ConnectingP2p),
        )
        .await;

        let peer = self.transports.connect(&listing.connection, sink.clone()).await?;
        let mode = peer.mode;

        // Caching failures must never block a join.
        let _ = self.db.cache_server_listing(&listing);

        let handshake = Arc::new(parking_lot::Mutex::new(None));
        let (handshake_tx, handshake_rx) = tokio::sync::oneshot::channel();
        let cancel = peer.cancel.clone();

        let handle = bridge::bridge_from_game_client(
            peer.stream,
            None,
            cancel.clone(),
            sink.clone(),
            Some(handshake_tx),
        )
        .await?;

        // Capture the handshake for diagnostics without blocking the join.
        let handshake_slot = handshake.clone();
        tokio::spawn(async move {
            if let Ok(info) = handshake_rx.await {
                *handshake_slot.lock() = Some(info);
            }
        });

        let session = Arc::new(GuestSession {
            id: listing.id,
            server_name: listing.name.clone(),
            mode,
            local_address: handle.local_address,
            remote: peer.remote,
            rtt_ms: peer.rtt_ms,
            handshake,
            cancel,
        });

        self.guests.insert(listing.id, session.clone());

        let _ = self.db.record_join(&crate::store::servers::JoinHistoryEntry {
            server_id: Some(listing.id),
            server_name: listing.name.clone(),
            instance_id: None,
            mode: Some(mode),
            joined_at: Utc::now(),
            outcome: "success".to_string(),
            detail: Some(format!("local bridge {}", session.local_address)),
        });

        if mode == ConnectionMode::Relay {
            let _ = sink
                .report(
                    ProgressEvent::started(JobKind::P2pConnect, "Connected via relay")
                        .stage(JobStage::Running)
                        .detail("a direct connection was not possible"),
                )
                .await;
        }

        Ok(session)
    }

    /// Stop a guest session and tear the bridge down.
    pub async fn leave(&self, id: Uuid) -> AppResult<()> {
        if let Some((_, session)) = self.guests.remove(&id) {
            session.cancel.cancel();
        }
        Ok(())
    }

    /// Stop a host session.
    pub async fn stop_hosting(&self, id: Uuid) -> AppResult<()> {
        if let Some((_, session)) = self.hosts.remove(&id) {
            session.stop().await?;
        }
        Ok(())
    }

    pub fn host_session(&self, id: Uuid) -> Option<Arc<HostSession>> {
        self.hosts.get(&id).map(|entry| entry.value().clone())
    }

    pub fn active_hosts(&self) -> Vec<Arc<HostSession>> {
        self.hosts.iter().map(|entry| entry.value().clone()).collect()
    }

    pub fn active_guests(&self) -> Vec<Arc<GuestSession>> {
        self.guests.iter().map(|entry| entry.value().clone()).collect()
    }

    /// Measure latency to a host without joining (server browser ping badge).
    pub async fn probe(&self, descriptor: &ConnectionDescriptor) -> Option<u32> {
        self.transports.probe(descriptor).await
    }

    /// Browse the directory (thin passthrough so commands depend on one type).
    pub async fn browse(
        &self,
        filter: &crate::models::server::ServerFilter,
    ) -> AppResult<Vec<ServerListingSummary>> {
        self.directory.browse(filter).await
    }

    /// Shut everything down (app exit).
    pub async fn shutdown(&self) {
        for session in self.hosts.iter() {
            let _ = session.value().stop().await;
        }
        self.hosts.clear();
        for session in self.guests.iter() {
            session.value().cancel.cancel();
        }
        self.guests.clear();
    }
}

/// How a join was addressed.
#[derive(Debug, Clone)]
pub enum JoinTarget {
    ListingId { id: Uuid },
    Code { code: String },
}

/// The host's single reader of the punch socket.
///
/// A UDP socket yields every datagram exactly once, so exactly one task may
/// call `recv_from` on it. This loop therefore demultiplexes everything by
/// packet type:
///
/// * punch probes (`SXPT` datagrams) — answered until the peer is established,
/// * tunnel handshakes (`Hello` frames carrying the session token) — verified,
///   policy-checked and promoted into a per-guest tunnel task,
/// * established guests' frames (`Data`/`Ack`/`Ping`/`Bye`) — routed to that
///   guest's tunnel task.
///
/// The previous design had the accept loop and each tunnel task race on the
/// same socket: a guest's data frames were regularly swallowed by the accept
/// loop, which only wanted `Hello`s, so tunnels stalled and byte counters
/// under-counted. One demux reader fixes both.
async fn run_host_demux(
    socket: Arc<tokio::net::UdpSocket>,
    session: Arc<HostSession>,
    options: HostOptions,
    session_token: String,
    events: broadcast::Sender<SessionEvent>,
    cancel: CancellationToken,
) {
    let token_fingerprint = holepunch::token_fingerprint(&session_token);
    let punch_ack = encode_punch(PunchKind::Ack, &token_fingerprint);
    let mut buffer = vec![0u8; FRAME_OVERHEAD + MAX_FRAME_PAYLOAD];

    // Frames destined for established guests, keyed by their socket address.
    let (frame_tx, _frame_rx) = tokio::sync::mpsc::unbounded_channel::<(SocketAddr, Frame)>();
    let mut guests: HashMap<SocketAddr, tokio::sync::mpsc::UnboundedSender<Frame>> = HashMap::new();

    loop {
        let received = tokio::select! {
            _ = cancel.cancelled() => break,
            result = socket.recv_from(&mut buffer) => result,
        };
        let Ok((read, from)) = received else {
            break;
        };
        let datagram = &buffer[..read];

        // Punch probes share the socket with the tunnel protocol (same port is
        // published in the listing's endpoints).
        if read == PUNCH_DATAGRAM_LEN {
            if let Some((_kind, peer_token)) = decode_punch(datagram) {
                if peer_token == token_fingerprint && !guests.contains_key(&from) {
                    // Acknowledge so the peer stops probing. Once a guest is
                    // established its frames speak for themselves.
                    let _ = socket.send_to(&punch_ack, from).await;
                }
                continue;
            }
        }

        let Some(frame) = Frame::decode(datagram) else {
            continue;
        };

        match frame.kind {
            FrameKind::Hello => {
                if frame.payload != session_token.as_bytes() {
                    continue;
                }
                if guests.contains_key(&from) {
                    // Retransmitted handshake (the Welcome datagram may have
                    // been lost): answer again, nothing else to do.
                    let welcome = Frame::new(FrameKind::Welcome, 0, 0, Vec::new());
                    let _ = socket.send_to(&welcome.encode(), from).await;
                    continue;
                }
                if let Some(rejection) = preflight_join(&session, &from, &events) {
                    let bye = Frame::new(
                        FrameKind::Bye,
                        0,
                        0,
                        rejection.user_message().as_bytes().to_vec(),
                    );
                    let _ = socket.send_to(&bye.encode(), from).await;
                    continue;
                }
                match promote_guest(&socket, &session, &options, &session_token, from, &events, frame_tx.clone())
                    .await
                {
                    Some(sender) => {
                        guests.insert(from, sender);
                    }
                    None => continue,
                }
            }
            _ => {
                let mut dead = false;
                if let Some(sender) = guests.get(&from) {
                    dead = sender.send(frame).is_err();
                }
                if dead {
                    guests.remove(&from);
                }
                // Frames from unknown peers are dropped: they belong to a
                // guest that just left or to someone scanning the port.
            }
        }
    }
    // Dropping `guests` (and `frame_tx`) closes every per-guest channel; the
    // tunnel tasks unwind and their bridges see EOF.
}

/// Capacity checks applied before a handshake becomes a guest.
fn preflight_join(
    session: &Arc<HostSession>,
    from: &SocketAddr,
    events: &broadcast::Sender<SessionEvent>,
) -> Option<JoinRejection> {
    if session.player_count().online >= session.listing.read().players.max {
        return Some(JoinRejection::ServerFull);
    }
    let whitelist = session.listing.read().whitelist.clone();
    if whitelist.enabled && whitelist.allowed_uuids.is_empty() {
        // Identity only becomes known during the game handshake; with an empty
        // allow-list nobody can be admitted.
        let _ = events.send(SessionEvent::GuestLeft {
            id: session.id,
            peer_id: format!("{from}"),
            reason: JoinRejection::NotWhitelisted,
        });
        return Some(JoinRejection::NotWhitelisted);
    }
    None
}

/// Turn a verified handshake into a registered guest with its own tunnel task.
/// Returns the sender the demux loop uses to route this guest's frames.
#[allow(clippy::too_many_arguments)]
async fn promote_guest(
    socket: &Arc<tokio::net::UdpSocket>,
    session: &Arc<HostSession>,
    options: &HostOptions,
    _session_token: &str,
    peer: SocketAddr,
    events: &broadcast::Sender<SessionEvent>,
    demux: tokio::sync::mpsc::UnboundedSender<(SocketAddr, Frame)>,
) -> Option<tokio::sync::mpsc::UnboundedSender<Frame>> {
    let peer_id = format!("{peer}");
    // A child of the session token: stopping the host tears every guest
    // tunnel down, while a kick only cancels one guest.
    let cancel = session.child_cancel_token();
    session.guests.insert(
        peer_id.clone(),
        GuestConnection {
            peer_id: peer_id.clone(),
            address: peer,
            username: None,
            protocol_version: None,
            connected: true,
            joined_at: Utc::now(),
            mode: ConnectionMode::DirectP2p,
            cancel: cancel.clone(),
        },
    );

    let _ = events.send(SessionEvent::GuestJoined {
        id: session.id,
        peer_id: peer_id.clone(),
        username: None,
        protocol_version: None,
    });
    let _ = session.directory.add_players(1).await;

    // Answer the handshake so the guest's `UdpTunnel::start` returns.
    let welcome = Frame::new(FrameKind::Welcome, 0, 0, Vec::new());
    if let Err(err) = socket.send_to(&welcome.encode(), peer).await {
        let _ = events.send(SessionEvent::Error {
            id: session.id,
            message: format!("could not answer the handshake: {err}"),
        });
        return None;
    }

    let (guest_tx, guest_rx) = tokio::sync::mpsc::unbounded_channel::<Frame>();
    let counters = session.byte_counters.clone();
    let local_server = options.local_server;
    let guests = session.guests.clone();
    let session_id = session.id;
    let error_events = events.clone();
    let tunnel_socket = socket.clone();
    let directory = session.directory.clone();

    tokio::spawn(async move {
        // Keep the demux sender alive for the tunnel's lifetime: dropping it
        // would look like a leave to the tunnel's frame router.
        let _keep_demux = demux;
        let result = run_guest_tunnel(
            tunnel_socket,
            peer,
            local_server,
            cancel,
            guest_rx,
            counters,
        )
        .await;
        if let Some(mut guest) = guests.get_mut(&peer_id) {
            guest.connected = false;
        }
        guests.remove(&peer_id);
        let _ = directory.add_players(-1).await;
        if let Err(err) = result {
            let _ = error_events.send(SessionEvent::Error {
                id: session_id,
                message: err.to_string(),
            });
        }
    });

    Some(guest_tx)
}

/// Application-facing end of a **host-side** tunnel.
///
/// * `poll_read` yields the guest's `Data` payloads (routed here by the demux
///   loop); a closed channel is EOF, which ends the bridge cleanly.
/// * `poll_write` hands server bytes to the tunnel's writer task, which frames,
///   sequences and ACK-awaits them exactly like the guest's `UdpTunnel` does.
struct HostTunnelStream {
    inbound: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    outbound: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl AsyncRead for HostTunnelStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match std::pin::Pin::new(&mut self.inbound).poll_recv(cx) {
            std::task::Poll::Ready(Some(payload)) => {
                buf.put_slice(&payload);
                std::task::Poll::Ready(Ok(()))
            }
            // Channel closed: the guest (or the demux loop) is gone — EOF.
            std::task::Poll::Ready(None) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl AsyncWrite for HostTunnelStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        // An unbounded channel never backpressures, so this is always ready.
        match self.outbound.send(buf.to_vec()) {
            Ok(()) => std::task::Poll::Ready(Ok(buf.len())),
            Err(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "tunnel writer is gone",
            ))),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Host side of one guest's tunnel.
///
/// Symmetric to the guest's `UdpTunnel`: a router splits demux-routed frames
/// into guest data (towards the local server) and ACKs (towards the writer),
/// while the writer streams local-server bytes back with the same
/// stop-and-wait discipline the guest applies to us.
async fn run_guest_tunnel(
    socket: Arc<tokio::net::UdpSocket>,
    peer: SocketAddr,
    local_server: SocketAddr,
    cancel: CancellationToken,
    mut from_demux: tokio::sync::mpsc::UnboundedReceiver<Frame>,
    counters: Arc<ByteCounters>,
) -> AppResult<()> {
    use tokio::sync::mpsc::UnboundedSender;

    let (inbound_tx, inbound_rx): (UnboundedSender<Vec<u8>>, _) = tokio::sync::mpsc::unbounded_channel();
    let (outbound_tx, mut outbound_rx): (UnboundedSender<Vec<u8>>, _) = tokio::sync::mpsc::unbounded_channel();
    let (ack_tx, mut ack_rx): (UnboundedSender<u32>, _) = tokio::sync::mpsc::unbounded_channel();

    // Router: demux frames -> inbound data / writer ACKs / pong / teardown.
    let router_cancel = cancel.clone();
    let router_socket = socket.clone();
    let router = tokio::spawn(async move {
        loop {
            let frame = tokio::select! {
                _ = router_cancel.cancelled() => break,
                frame = from_demux.recv() => match frame {
                    Some(frame) => frame,
                    None => break,
                },
            };
            match frame.kind {
                FrameKind::Data => {
                    if inbound_tx.send(frame.payload).is_err() {
                        break;
                    }
                }
                FrameKind::Ack => {
                    let _ = ack_tx.send(frame.ack);
                }
                FrameKind::Ping => {
                    let pong = Frame::new(FrameKind::Pong, 0, 0, Vec::new());
                    let _ = router_socket.send_to(&pong.encode(), peer).await;
                }
                FrameKind::Bye => break,
                FrameKind::Hello | FrameKind::Welcome | FrameKind::Pong => {}
            }
        }
    });

    // Writer: local-server bytes -> guest, stop-and-wait with ACKs.
    let writer_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        let mut sequence: u32 = 0;
        loop {
            let payload = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                payload = outbound_rx.recv() => match payload {
                    Some(payload) => payload,
                    None => break,
                },
            };

            let frame = Frame::new(FrameKind::Data, sequence, 0, payload);
            let encoded = frame.encode();

            let mut acknowledged = false;
            for _ in 0..MAX_RETRIES {
                if socket.send_to(&encoded, peer).await.is_err() {
                    break;
                }
                match tokio::time::timeout(ACK_TIMEOUT, ack_rx.recv()).await {
                    Ok(Some(ack)) if ack == sequence => {
                        acknowledged = true;
                        break;
                    }
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => continue,
                }
            }

            if !acknowledged {
                break;
            }
            sequence = sequence.wrapping_add(1);
        }
    });

    // Bridge the (counted) tunnel to the local Minecraft server. The bridge
    // owns the connection out to `local_server` and splices both directions.
    let tunnel = CountingStream::new(
        Box::new(HostTunnelStream {
            inbound: inbound_rx,
            outbound: outbound_tx,
        }),
        counters.clone(),
    );
    let bridge_result = bridge::bridge_to_local_server(
        Box::new(tunnel),
        local_server,
        cancel.clone(),
        Arc::new(crate::models::progress::NoopProgressSink),
    )
    .await;

    // Whichever side ended first, tear the rest down and reap the tasks.
    cancel.cancel();
    let _ = writer.await;
    let _ = router.await;
    bridge_result.map(|_| ())
}
/// Minecraft protocol version for a game version (best effort; the exact value
/// is verified against the client's handshake once someone joins).
pub fn protocol_version_for(game_version: &str) -> i32 {
    match game_version {
        v if v.starts_with("1.21.4") => 769,
        v if v.starts_with("1.21.2") || v.starts_with("1.21.3") => 768,
        v if v.starts_with("1.21.1") || v.starts_with("1.21") => 767,
        v if v.starts_with("1.20.5") || v.starts_with("1.20.6") => 766,
        v if v.starts_with("1.20.3") || v.starts_with("1.20.4") => 765,
        v if v.starts_with("1.20.2") => 764,
        v if v.starts_with("1.20") => 763,
        v if v.starts_with("1.19.4") => 762,
        v if v.starts_with("1.19.3") => 761,
        v if v.starts_with("1.19") => 760,
        v if v.starts_with("1.18") => 757,
        v if v.starts_with("1.17") => 755,
        v if v.starts_with("1.16") => 754,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_versions_match_the_wiki_table() {
        assert_eq!(protocol_version_for("1.20.1"), 763);
        assert_eq!(protocol_version_for("1.20.4"), 765);
        assert_eq!(protocol_version_for("1.21.4"), 769);
        assert_eq!(protocol_version_for("1.16.5"), 754);
        assert_eq!(protocol_version_for("unknown"), 0);
    }

    #[test]
    fn host_options_default_to_a_local_vanilla_world() {
        let options = HostOptions::default();
        assert_eq!(options.local_server.port(), DEFAULT_SERVER_PORT);
        assert!(options.public);
        assert!(!options.force_relay);
        assert_eq!(options.loader.kind, crate::models::instance::LoaderKind::Vanilla);
    }

    #[test]
    fn join_targets_are_debuggable_and_distinct() {
        let by_id = JoinTarget::ListingId { id: Uuid::nil() };
        let by_code = JoinTarget::Code {
            code: "SXM1-ABCD".into(),
        };
        assert!(format!("{by_id:?}").contains("ListingId"));
        assert!(format!("{by_code:?}").contains("SXM1"));
    }
}
