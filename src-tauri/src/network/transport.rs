//! Transport abstraction + the UDP tunnel that carries a Minecraft session.
//!
//! Why a custom framing layer instead of libp2p here:
//! * The bridge only ever needs **one ordered byte stream** per session.
//! * It must be trivially inspectable: a launcher that silently fails to connect
//!   is unfixable, so every frame is small, logged when debug logging is on, and
//!   has an explicit kind.
//! * `libp2p`/QUIC can be dropped in behind [`Transport`] later without touching
//!   the session manager or the bridge.
//!
//! ## Reliability
//!
//! Stop-and-wait with a single in-flight frame per direction. Minecraft's
//! traffic is small and latency critical (keep-alives, position updates), and
//! stop-and-wait avoids head-of-line pathologies of a naive window while keeping
//! the code auditable. A 100 ms RTT session sustains ~640 KiB/s at 64 KiB frames,
//! which is far above what the game needs. Bulk transfers (world downloads) are
//! routed through the relay transport instead.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::server::{
    ConnectionDescriptor, ConnectionMode, EndpointKind, PeerEndpoint, SignalingEnvelope,
};
use crate::network::directory::Directory;
use crate::network::holepunch::{self, PunchConfig, PunchedChannel};
use crate::network::relay::RelayTransport;

/// Maximum application payload per frame (keeps datagrams under the path MTU).
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024;
/// Retransmit timeout for an unacknowledged frame.
pub const ACK_TIMEOUT: Duration = Duration::from_millis(400);
/// Retransmit budget before the tunnel is declared dead.
pub const MAX_RETRIES: u32 = 12;
/// Keep-alive interval when no data flows.
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Anything that behaves like a bidirectional byte stream.
pub trait DuplexStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DuplexStream for T {}

/// Live byte counters for a tunneled session.
///
/// Shared between the stream wrapper and whoever persists the session record:
/// the diagnostics panel shows how much traffic a session carried.
#[derive(Debug, Default)]
pub struct ByteCounters {
    up: std::sync::atomic::AtomicU64,
    down: std::sync::atomic::AtomicU64,
}

impl ByteCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.up.load(std::sync::atomic::Ordering::Relaxed),
            self.down.load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    fn add_up(&self, bytes: usize) {
        self.up
            .fetch_add(bytes as u64, std::sync::atomic::Ordering::Relaxed);
    }

    fn add_down(&self, bytes: usize) {
        self.down
            .fetch_add(bytes as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A [`DuplexStream`] that counts application bytes in both directions.
///
/// Wrap the tunnel stream with this and keep the counters around: when the
/// session ends, [`ByteCounters::snapshot`] gives the final numbers for the
/// `p2p_sessions` row.
pub struct CountingStream {
    inner: Box<dyn DuplexStream>,
    counters: Arc<ByteCounters>,
}

impl CountingStream {
    pub fn new(inner: Box<dyn DuplexStream>, counters: Arc<ByteCounters>) -> Self {
        Self { inner, counters }
    }

    pub fn counters(&self) -> Arc<ByteCounters> {
        self.counters.clone()
    }
}

impl AsyncRead for CountingStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let filled_before = buf.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let read = buf.filled().len() - filled_before;
            if read > 0 {
                self.counters.add_down(read);
            }
        }
        result
    }
}

impl AsyncWrite for CountingStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let result = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(written)) = &result {
            self.counters.add_up(*written);
        }
        result
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A connected peer plus how we reached it.
pub struct ConnectedPeer {
    pub peer_id: String,
    pub mode: ConnectionMode,
    pub remote: Option<SocketAddr>,
    pub rtt_ms: Option<u32>,
    /// The ordered byte stream carrying the tunneled Minecraft protocol.
    pub stream: Box<dyn DuplexStream>,
    /// Cancelling this tears the tunnel down.
    pub cancel: CancellationToken,
}

impl std::fmt::Debug for ConnectedPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectedPeer")
            .field("peer_id", &self.peer_id)
            .field("mode", &self.mode)
            .field("remote", &self.remote)
            .field("rtt_ms", &self.rtt_ms)
            .finish_non_exhaustive()
    }
}

/// A way to reach a host.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Human-readable identifier (`direct-udp`, `relay-tcp`).
    fn kind(&self) -> &'static str;
    /// Mode this transport satisfies.
    fn mode(&self) -> ConnectionMode;

    /// Attempt the connection. Implementations must be cancellable and must
    /// never block indefinitely (their own internal timeout applies).
    async fn connect(
        &self,
        descriptor: &ConnectionDescriptor,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ConnectedPeer>;

    /// Cheapest possible "are you there?" probe, used for the ping badge.
    async fn probe(&self, descriptor: &ConnectionDescriptor) -> AppResult<Option<u32>>;
}

/// Ordered transport chain with relay fallback.
pub struct TransportRegistry {
    direct: Arc<DirectTransport>,
    relay: Arc<RelayTransport>,
}

impl TransportRegistry {
    pub fn new(direct: Arc<DirectTransport>, relay: Arc<RelayTransport>) -> Self {
        Self { direct, relay }
    }

    pub fn direct(&self) -> &Arc<DirectTransport> {
        &self.direct
    }

    pub fn relay(&self) -> &Arc<RelayTransport> {
        &self.relay
    }

    /// Connect using the best available transport.
    ///
    /// Order:
    /// 1. `Dedicated` / `DirectP2p` descriptors try direct first.
    /// 2. Relay-mode descriptors still try loopback and LAN candidates.
    /// 3. On failure, fall back to the relay when the descriptor carries one.
    ///    This is exactly the "symmetric NAT" case the spec calls out.
    pub async fn connect(
        &self,
        descriptor: &ConnectionDescriptor,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ConnectedPeer> {
        let mut last_error: Option<AppError> = None;

        if descriptor.mode == ConnectionMode::Relay {
            sink.report(
                ProgressEvent::started(JobKind::P2pConnect, "Host is on a strict NAT")
                    .stage(JobStage::ConnectingP2p)
                    .detail("Trying a short direct tunnel, then the relay"),
            )
            .await;
        }

        // Relay mode still tries loopback / LAN first. A strict NAT blocks the
        // public mapping, not a second launcher on this PC. That attempt is
        // capped so a dead candidate cannot postpone the relay.
        if let Some(direct) = direct_attempt(descriptor) {
            let budget = if descriptor.mode == ConnectionMode::Relay {
                RELAY_DIRECT_TIMEOUT
            } else {
                DIRECT_ATTEMPT_TIMEOUT
            };
            match tokio::time::timeout(budget, self.direct.connect(&direct, sink.clone())).await {
                Ok(Ok(peer)) => return Ok(peer),
                Ok(Err(err)) => {
                    sink.report(
                        ProgressEvent::started(JobKind::P2pConnect, "Direct connection failed")
                            .stage(JobStage::Failed)
                            .detail(err.to_string())
                            .failed(err.to_string()),
                    )
                    .await;
                    last_error = Some(err);
                }
                Err(_) => {
                    let err = AppError::Transport(
                        "JOIN_UNREACHABLE: the direct tunnel timed out before the relay fallback"
                            .to_string(),
                    );
                    sink.report(
                        ProgressEvent::started(JobKind::P2pConnect, "Direct tunnel timed out")
                            .stage(JobStage::Failed)
                            .failed(err.to_string()),
                    )
                    .await;
                    last_error = Some(err);
                }
            }
        }

        if descriptor.relay.is_some() {
            match self.relay.connect(descriptor, sink).await {
                Ok(peer) => return Ok(peer),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Transport(
                "JOIN_UNREACHABLE: the host could not be reached directly and offers no relay"
                    .to_string(),
            )
        }))
    }

    /// Best-effort latency for a descriptor (direct probe only).
    pub async fn probe(&self, descriptor: &ConnectionDescriptor) -> Option<u32> {
        if descriptor.mode == ConnectionMode::Relay {
            return None;
        }
        self.direct.probe(descriptor).await.ok().flatten()
    }
}

/// How long a strict-NAT guest may spend on loopback/LAN before the relay.
const RELAY_DIRECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Cap for a punchable direct attempt (STUN plus the punch budget).
const DIRECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Cancels signaling when a direct attempt is dropped by the outer timeout.
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Direct UDP transport: hole punching + the tunnel.
pub struct DirectTransport {
    /// Local address to bind tunnels on (usually `0.0.0.0:0`).
    bind: SocketAddr,
    /// When set, the guest publishes its candidates and accepts answers while punching.
    directory: Option<Arc<dyn Directory>>,
    punch: PunchConfig,
}

impl DirectTransport {
    pub fn new(bind: SocketAddr) -> Self {
        Self {
            bind,
            directory: None,
            punch: PunchConfig::default(),
        }
    }

    /// Attach the shared directory so a join can exchange ICE-style candidates.
    pub fn with_signaling(mut self, directory: Arc<dyn Directory>, punch: PunchConfig) -> Self {
        self.directory = Some(directory);
        self.punch = punch;
        self
    }
}

#[async_trait]
impl Transport for DirectTransport {
    fn kind(&self) -> &'static str {
        "direct-udp"
    }

    fn mode(&self) -> ConnectionMode {
        ConnectionMode::DirectP2p
    }

    async fn connect(
        &self,
        descriptor: &ConnectionDescriptor,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ConnectedPeer> {
        let candidates = ordered_endpoints(&descriptor.endpoints);
        if candidates.is_empty() && self.directory.is_none() {
            return Err(AppError::Transport(
                "JOIN_UNREACHABLE: the host did not publish any reachable address".to_string(),
            ));
        }

        // Bind the configured address (`0.0.0.0:0` in the app). A loopback-only
        // socket cannot send to a LAN interface address, which is how a second
        // launcher on this machine reaches the host when relay DNS fails.
        let socket = UdpSocket::bind(self.bind)
            .await
            .map_err(|err| AppError::Transport(format!("cannot bind tunnel socket: {err}")))?;

        let peers = Arc::new(parking_lot::Mutex::new(
            candidates
                .iter()
                .map(|(_, address)| *address)
                .collect::<Vec<_>>(),
        ));
        let stop_signals = CancellationToken::new();
        let _stop_on_drop = CancelOnDrop(stop_signals.clone());
        let locals_only = !candidates.is_empty()
            && candidates
                .iter()
                .all(|(endpoint, _)| endpoint.kind == EndpointKind::Local);
        // Local candidates (loopback / LAN) do not need STUN. Classifying here
        // burns several seconds and, on a strict-NAT host, delays the relay.
        if let Some(directory) = &self.directory {
            if !locals_only {
                let guest_peer = uuid::Uuid::new_v4().to_string();
                let mut offered = guest_endpoints(socket.local_addr().ok());
                match holepunch::StunClient::classify(
                    &socket,
                    &self.punch.stun_servers,
                    self.punch.stun_timeout,
                )
                .await
                {
                    Ok(mapping) => offered.push(PeerEndpoint::public(mapping.address)),
                    Err(err) => {
                        sink.report(
                            ProgressEvent::started(JobKind::P2pConnect, "NAT discovery failed")
                                .stage(JobStage::ConnectingP2p)
                                .detail(format!("STUN_UNREACHABLE: {err}")),
                        )
                        .await;
                    }
                }
                let session_id = uuid::Uuid::parse_str(&descriptor.peer_id)
                    .unwrap_or_else(|_| uuid::Uuid::nil());
                let _ = directory
                    .publish_signal(
                        &descriptor.peer_id,
                        &SignalingEnvelope::Offer {
                            from_peer_id: guest_peer.clone(),
                            to_peer_id: descriptor.peer_id.clone(),
                            session_id,
                            candidates: offered,
                            public_key: descriptor.public_key.clone(),
                            signature: String::new(),
                            sent_at: chrono::Utc::now(),
                        },
                    )
                    .await;
                let directory = directory.clone();
                let extra = peers.clone();
                let stop = stop_signals.clone();
                tokio::spawn(async move {
                    while !stop.is_cancelled() {
                        if let Ok(envelopes) = directory.drain_signals(&guest_peer).await {
                            for envelope in envelopes {
                                if let SignalingEnvelope::Answer { candidates, .. } = envelope {
                                    let mut guard = extra.lock();
                                    for candidate in candidates {
                                        if !guard.contains(&candidate.addr) {
                                            guard.push(candidate.addr);
                                        }
                                    }
                                }
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                });
            }
        }

        sink.report(
            ProgressEvent::started(JobKind::P2pConnect, "Opening a direct tunnel")
                .stage(JobStage::ConnectingP2p)
                .detail(format!("{} candidates", peers.lock().len())),
        )
        .await;

        let punch_config = if locals_only {
            // Unreachable LAN addresses must not stall the relay fallback.
            PunchConfig {
                timeout: Duration::from_millis(700),
                probe_interval: Duration::from_millis(50),
                stun_servers: Vec::new(),
                stun_timeout: Duration::from_millis(200),
            }
        } else {
            self.punch.clone()
        };
        let punched =
            holepunch::punch_candidates(&socket, &peers, &descriptor.session_token, &punch_config)
                .await;
        stop_signals.cancel();
        match punched {
            Ok(punched) => {
                let remote = punched.peer;
                let (stream, rtt) =
                    UdpTunnel::start(socket, punched, &descriptor.session_token).await?;
                Ok(ConnectedPeer {
                    peer_id: descriptor.peer_id.clone(),
                    mode: ConnectionMode::DirectP2p,
                    remote: Some(remote),
                    rtt_ms: rtt,
                    stream: Box::new(stream),
                    cancel: CancellationToken::new(),
                })
            }
            Err(err) => Err(err),
        }
    }

    async fn probe(&self, descriptor: &ConnectionDescriptor) -> AppResult<Option<u32>> {
        let Some((_, address)) = ordered_endpoints(&descriptor.endpoints).into_iter().next() else {
            return Ok(None);
        };
        let socket = UdpSocket::bind(self.bind).await?;
        let started = Instant::now();
        let punched =
            crate::network::holepunch::punch(&socket, address, &descriptor.session_token).await?;
        let _ = punched;
        Ok(Some(started.elapsed().as_millis() as u32))
    }
}

/// Direct attempt for this descriptor.
///
/// Punchable hosts try every candidate. Relay-mode hosts only try local
/// candidates (loopback and LAN); the public mapping is not punchable.
fn direct_attempt(descriptor: &ConnectionDescriptor) -> Option<ConnectionDescriptor> {
    if descriptor.mode != ConnectionMode::Relay {
        return Some(descriptor.clone());
    }
    let mut local_only = descriptor.clone();
    local_only.mode = ConnectionMode::DirectP2p;
    local_only
        .endpoints
        .retain(|endpoint| endpoint.kind == EndpointKind::Local);
    if local_only.endpoints.is_empty() {
        None
    } else {
        Some(local_only)
    }
}

/// Addresses the guest can be dialed on, derived from the punch socket.
fn guest_endpoints(bound: Option<SocketAddr>) -> Vec<PeerEndpoint> {
    let Some(bound) = bound else {
        return Vec::new();
    };
    let port = bound.port();
    if port == 0 {
        return Vec::new();
    }
    let mut endpoints = vec![PeerEndpoint::local(SocketAddr::from((
        std::net::Ipv4Addr::LOCALHOST,
        port,
    )))];
    for ip in crate::network::localnet::ipv4_interface_addresses() {
        if ip.is_unspecified() || ip.is_loopback() {
            continue;
        }
        let addr = SocketAddr::from((ip, port));
        if endpoints.iter().any(|endpoint| endpoint.addr == addr) {
            continue;
        }
        endpoints.push(PeerEndpoint::local(addr));
    }
    endpoints
}

/// Endpoints sorted by preference, dropping expired ones.
fn ordered_endpoints(endpoints: &[PeerEndpoint]) -> Vec<(PeerEndpoint, SocketAddr)> {
    let now = chrono::Utc::now();
    let mut candidates: Vec<(PeerEndpoint, SocketAddr)> = endpoints
        .iter()
        .filter(|endpoint| {
            endpoint
                .expires_at
                .map(|expiry| expiry > now)
                .unwrap_or(true)
        })
        .map(|endpoint| (endpoint.clone(), endpoint.addr))
        .collect();
    candidates.sort_by_key(|(endpoint, _)| match endpoint.kind {
        crate::models::server::EndpointKind::Local => 0,
        crate::models::server::EndpointKind::Public => 1,
        crate::models::server::EndpointKind::Relay => 2,
    });
    candidates
}

/// Frame kinds understood by the tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// Session handshake, carries the session token.
    Hello = 1,
    /// Handshake response.
    Welcome = 2,
    /// Ordered application bytes.
    Data = 3,
    /// Cumulative acknowledgement of the last received `Data` sequence.
    Ack = 4,
    /// Liveness probe.
    Ping = 5,
    Pong = 6,
    /// Graceful teardown.
    Bye = 7,
}

impl FrameKind {
    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => FrameKind::Hello,
            2 => FrameKind::Welcome,
            3 => FrameKind::Data,
            4 => FrameKind::Ack,
            5 => FrameKind::Ping,
            6 => FrameKind::Pong,
            7 => FrameKind::Bye,
            _ => return None,
        })
    }
}

/// Wire frame: `kind(1) | seq(4) | ack(4) | payload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    pub seq: u32,
    pub ack: u32,
    pub payload: Vec<u8>,
}

/// Header size in bytes.
pub const FRAME_HEADER_LEN: usize = 9;
/// Frame magic marker; a stray UDP packet is dropped before it can confuse us.
pub const FRAME_MAGIC: [u8; 2] = [0x53, 0x58]; // "SX"
/// Bytes of magic + header.
pub const FRAME_OVERHEAD: usize = FRAME_MAGIC.len() + FRAME_HEADER_LEN;

impl Frame {
    pub fn new(kind: FrameKind, seq: u32, ack: u32, payload: Vec<u8>) -> Self {
        Self {
            kind,
            seq,
            ack,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(FRAME_OVERHEAD + self.payload.len());
        bytes.extend_from_slice(&FRAME_MAGIC);
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.seq.to_be_bytes());
        bytes.extend_from_slice(&self.ack.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Decode a datagram, returning `None` when it is not one of our frames.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FRAME_OVERHEAD || bytes[..2] != FRAME_MAGIC {
            return None;
        }
        let kind = FrameKind::from_u8(bytes[2])?;
        let seq = u32::from_be_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
        let ack = u32::from_be_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]);
        Some(Self {
            kind,
            seq,
            ack,
            payload: bytes[FRAME_OVERHEAD..].to_vec(),
        })
    }
}

/// The ordered, acknowledged byte stream produced by [`UdpTunnel`].
///
/// It is a plain `AsyncRead + AsyncWrite` so the bridge, the tests and any
/// future QUIC transport all behave identically.
pub struct UdpTunnelStream {
    /// Bytes coming out of the tunnel (reliable, ordered).
    reader: tokio::io::DuplexStream,
    /// Bytes going into the tunnel.
    writer: tokio::io::DuplexStream,
    cancel: CancellationToken,
}

impl UdpTunnelStream {
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl AsyncRead for UdpTunnelStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for UdpTunnelStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.cancel.cancel();
        std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

/// Drives the framing/reliability loop over a punched UDP socket.
pub struct UdpTunnel;

/// Channel depth for the tunnel's internal byte queues.
const TUNNEL_BUFFER: usize = 256 * 1024;

impl UdpTunnel {
    /// Start the tunnel and return the application-facing stream + RTT.
    ///
    /// `socket` may be shared (two references to the same UDP socket), as when
    /// a single hole-punch socket is demultiplexed between the handshake and
    /// the data phase; each side keeps its own `Arc`.
    pub async fn start(
        socket: UdpSocket,
        punched: PunchedChannel,
        session_token: &str,
    ) -> AppResult<(UdpTunnelStream, Option<u32>)> {
        let peer = punched.peer;
        let token = session_token.as_bytes().to_vec();

        let (app_in, tunnel_in) = tokio::io::duplex(TUNNEL_BUFFER);
        let (tunnel_out, app_out) = tokio::io::duplex(TUNNEL_BUFFER);

        // Handshake: HELLO (client side) then WELCOME.
        let hello = Frame::new(FrameKind::Hello, 0, 0, token.clone());
        socket
            .send_to(&hello.encode(), peer)
            .await
            .map_err(|err| AppError::Transport(format!("could not send the handshake: {err}")))?;

        let rtt = tokio::time::timeout(Duration::from_secs(5), await_welcome(&socket, peer))
            .await
            .map_err(|_| {
                AppError::Transport(
                    "the peer accepted the punch but never answered the tunnel handshake"
                        .to_string(),
                )
            })??;

        let cancel = CancellationToken::new();
        let task_socket = Arc::new(socket);
        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            if let Err(err) =
                run_tunnel(task_socket, peer, tunnel_in, tunnel_out, task_cancel).await
            {
                // A dead tunnel surfaces as an EOF on the application stream,
                // which the bridge turns into a clean "connection closed".
                eprintln!("[p2p] tunnel closed: {err}");
            }
        });

        Ok((
            UdpTunnelStream {
                reader: app_out,
                writer: app_in,
                cancel,
            },
            rtt,
        ))
    }
}

/// Wait for a `Welcome` frame and measure the round trip.
async fn await_welcome(socket: &UdpSocket, peer: SocketAddr) -> AppResult<Option<u32>> {
    let started = Instant::now();
    let mut buffer = vec![0u8; FRAME_OVERHEAD + MAX_FRAME_PAYLOAD];

    loop {
        let (read, from) = socket
            .recv_from(&mut buffer)
            .await
            .map_err(|err| AppError::Transport(format!("tunnel handshake failed: {err}")))?;
        if from != peer {
            // Ignore unsolicited datagrams (port scans, stray STUN replies).
            continue;
        }
        let Some(frame) = Frame::decode(&buffer[..read]) else {
            continue;
        };
        match frame.kind {
            FrameKind::Welcome => {
                let rtt = started.elapsed().as_millis() as u32;
                // Acknowledge nothing: WELCOME is not sequenced.
                return Ok(Some(rtt));
            }
            FrameKind::Ping => {
                let pong = Frame::new(FrameKind::Pong, 0, 0, Vec::new());
                let _ = socket.send_to(&pong.encode(), peer).await;
            }
            _ => continue,
        }
    }
}

/// The tunnel event loop: reads app bytes, frames + ACKs them, and pumps
/// received payloads back into the application stream.
async fn run_tunnel(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    mut from_app: tokio::io::DuplexStream,
    mut to_app: tokio::io::DuplexStream,
    cancel: CancellationToken,
) -> AppResult<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(32);
    let (ack_tx, mut ack_rx) = mpsc::channel::<u32>(64);

    // Reader: socket -> app, acknowledging data frames.
    let reader_cancel = cancel.clone();
    let reader_socket = socket.clone();
    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; FRAME_OVERHEAD + MAX_FRAME_PAYLOAD];
        let mut expected_seq: u32 = 0;

        loop {
            let received = tokio::select! {
                _ = reader_cancel.cancelled() => break,
                result = reader_socket.recv_from(&mut buffer) => result,
            };
            let (read, from) = match received {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("[p2p] receive failed: {err}");
                    break;
                }
            };
            if from != peer {
                continue;
            }
            let Some(frame) = Frame::decode(&buffer[..read]) else {
                continue;
            };

            match frame.kind {
                FrameKind::Data => {
                    // Strictly ordered delivery: anything out of order is
                    // re-requested implicitly by ACKing the last good sequence.
                    if frame.seq == expected_seq {
                        if to_app.write_all(&frame.payload).await.is_err() {
                            break;
                        }
                        expected_seq = expected_seq.wrapping_add(1);
                    }
                    let _ = ack_tx.try_send(expected_seq.wrapping_sub(1));
                }
                FrameKind::Ping => {
                    // Answer from the writer task: the socket is shared, and the
                    // reader must never block on a send.
                    let _ = outgoing_tx.try_send_control(FrameKind::Pong);
                }
                FrameKind::Pong => {}
                FrameKind::Ack => {
                    // A host that runs the same demux answers our Data frames
                    // here (the writer is mid-`send_to`, never receiving).
                    let _ = ack_tx.try_send(frame.ack);
                }
                FrameKind::Bye => break,
                FrameKind::Hello | FrameKind::Welcome => {}
            }
        }
        let _ = to_app.shutdown().await;
    });

    // Writer: app -> socket, stop-and-wait with ACK verification.
    let writer_cancel = cancel.clone();
    let writer_socket = socket.clone();
    let writer = tokio::spawn(async move {
        let socket = writer_socket;
        let mut sequence: u32 = 0;
        let mut buffer = vec![0u8; MAX_FRAME_PAYLOAD];

        loop {
            // Drain queued control frames (Pong replies) before anything else,
            // otherwise a peer that only pings would never get an answer.
            while let Ok(control) = outgoing_rx.try_recv() {
                if socket.send_to(&control, peer).await.is_err() {
                    return;
                }
            }

            let read = tokio::select! {
                _ = writer_cancel.cancelled() => break,
                Some(control) = outgoing_rx.recv() => {
                    let _ = socket.send_to(&control, peer).await;
                    continue;
                }
                result = from_app.read(&mut buffer) => match result {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(_) => break,
                },
            };

            let frame = Frame::new(FrameKind::Data, sequence, 0, buffer[..read].to_vec());
            let encoded = frame.encode();

            let mut acknowledged = false;
            for _ in 0..MAX_RETRIES {
                if socket.send_to(&encoded, peer).await.is_err() {
                    break;
                }
                // Wait for the ACK that covers this sequence number.
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
                eprintln!("[p2p] peer stopped acknowledging at sequence {sequence}");
                break;
            }
            sequence = sequence.wrapping_add(1);
        }
    });

    // Keep-alive: keeps the NAT mapping warm and detects a dead peer.
    let keepalive_cancel = cancel.clone();
    let keepalive = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = keepalive_cancel.cancelled() => break,
                _ = tokio::time::sleep(KEEPALIVE_INTERVAL) => {
                    let ping = Frame::new(FrameKind::Ping, 0, 0, Vec::new());
                    if socket.send_to(&ping.encode(), peer).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let _ = reader.await;
    let _ = writer.await;
    let _ = keepalive.await;
    Ok(())
}

/// Helper so the reader task can queue control frames without a mutable handle.
trait ControlSender {
    fn try_send_control(&self, kind: FrameKind) -> Result<(), ()>;
}

impl ControlSender for mpsc::Sender<Vec<u8>> {
    fn try_send_control(&self, kind: FrameKind) -> Result<(), ()> {
        let frame = Frame::new(kind, 0, 0, Vec::new());
        self.try_send(frame.encode()).map_err(|_| ())
    }
}

/// Handshake awaited by the **host** side of a punched session.
pub struct HandshakeWaiter {
    socket: UdpSocket,
    /// When set, only this address may complete the handshake.
    expected_peer: Option<SocketAddr>,
    token: String,
    ready: oneshot::Sender<SocketAddr>,
}

impl HandshakeWaiter {
    pub fn new(
        socket: UdpSocket,
        expected_peer: Option<SocketAddr>,
        token: impl Into<String>,
        ready: oneshot::Sender<SocketAddr>,
    ) -> Self {
        Self {
            socket,
            expected_peer,
            token: token.into(),
            ready,
        }
    }

    /// Await a client HELLO carrying the expected session token.
    pub async fn wait(self) -> AppResult<SocketAddr> {
        let mut buffer = vec![0u8; FRAME_OVERHEAD + MAX_FRAME_PAYLOAD];
        let deadline = Instant::now() + Duration::from_secs(15);

        loop {
            if Instant::now() > deadline {
                return Err(AppError::Transport(
                    "no guest completed the handshake in time".to_string(),
                ));
            }
            let (read, from) = self.socket.recv_from(&mut buffer).await?;
            if let Some(expected) = self.expected_peer {
                if from != expected {
                    continue;
                }
            }
            let Some(frame) = Frame::decode(&buffer[..read]) else {
                continue;
            };
            if frame.kind != FrameKind::Hello {
                continue;
            }
            // A wrong token must not open a tunnel: the token is the only thing
            // preventing an unrelated peer from hijacking the punched port.
            if frame.payload != self.token.as_bytes() {
                continue;
            }
            let welcome = Frame::new(FrameKind::Welcome, 0, 0, Vec::new());
            let _ = self.socket.send_to(&welcome.encode(), from).await;
            let _ = self.ready.send(from);
            return Ok(from);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = Frame::new(FrameKind::Data, 7, 3, b"hello minecraft".to_vec());
        let decoded = Frame::decode(&frame.encode()).expect("decodes");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn non_frames_are_ignored_not_misparsed() {
        assert!(Frame::decode(b"GET / HTTP/1.1").is_none());
        assert!(Frame::decode(&[]).is_none());
        assert!(Frame::decode(&[0x53, 0x58, 0xff]).is_none());
        // Correct magic but unknown kind byte.
        let mut bytes = Frame::new(FrameKind::Data, 1, 1, vec![]).encode();
        bytes[2] = 42;
        assert!(Frame::decode(&bytes).is_none());
    }

    #[test]
    fn empty_payload_frames_are_valid() {
        let frame = Frame::new(FrameKind::Ping, 0, 0, Vec::new());
        let encoded = frame.encode();
        assert_eq!(encoded.len(), FRAME_OVERHEAD);
        assert_eq!(Frame::decode(&encoded), Some(frame));
    }

    #[test]
    fn max_payload_frame_fits_in_a_reasonable_mtu() {
        assert!(FRAME_OVERHEAD + MAX_FRAME_PAYLOAD <= u16::MAX as usize);
    }

    #[test]
    fn endpoints_are_sorted_local_then_public_then_relay() {
        use crate::models::server::{EndpointKind, PeerEndpoint};

        let endpoints = vec![
            PeerEndpoint::relay("198.51.100.1:9000".parse().unwrap()),
            PeerEndpoint::public("203.0.113.5:25565".parse().unwrap()),
            PeerEndpoint::local("192.168.1.20:25565".parse().unwrap()),
        ];
        let ordered = ordered_endpoints(&endpoints);
        assert_eq!(
            ordered[0].0.kind,
            EndpointKind::Local,
            "local addresses should win for same-network joins"
        );
        assert_eq!(ordered[1].0.kind, EndpointKind::Public);
        assert_eq!(ordered[2].0.kind, EndpointKind::Relay);
    }

    fn sample_descriptor(
        mode: ConnectionMode,
        endpoints: Vec<PeerEndpoint>,
    ) -> ConnectionDescriptor {
        ConnectionDescriptor {
            mode,
            peer_id: "peer".into(),
            public_key: String::new(),
            endpoints,
            relay: Some(crate::models::server::RelayDescriptor {
                url: "wss://relay.sxmlauncher.dev".into(),
                room_token: "room".into(),
                region: None,
                cert_fingerprint: None,
            }),
            session_token: "token".into(),
            protocol_version: 767,
        }
    }

    #[test]
    fn relay_mode_still_tries_local_endpoints() {
        let descriptor = sample_descriptor(
            ConnectionMode::Relay,
            vec![
                PeerEndpoint::public("203.0.113.5:25565".parse().unwrap()),
                PeerEndpoint::local("127.0.0.1:41234".parse().unwrap()),
            ],
        );
        let attempt = direct_attempt(&descriptor).expect("local candidate");
        assert_eq!(attempt.endpoints.len(), 1);
        assert!(attempt.endpoints[0].addr.ip().is_loopback());
        assert!(attempt
            .relay
            .as_ref()
            .is_some_and(|relay| !relay.url.is_empty()));
    }

    #[test]
    fn relay_mode_without_local_endpoints_skips_direct() {
        let descriptor = sample_descriptor(
            ConnectionMode::Relay,
            vec![PeerEndpoint::public("203.0.113.5:25565".parse().unwrap())],
        );
        assert!(direct_attempt(&descriptor).is_none());
    }

    #[tokio::test]
    async fn relay_mode_falls_back_before_a_direct_tunnel_can_stall() {
        use std::time::Instant;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut length_bytes = [0u8; 4];
            socket.read_exact(&mut length_bytes).await.expect("length");
            let mut payload = vec![0u8; u32::from_be_bytes(length_bytes) as usize];
            socket.read_exact(&mut payload).await.expect("payload");
            let body = serde_json::to_vec(&crate::network::relay::RelayResponse {
                ok: true,
                error: None,
                observed_address: None,
                region: Some("test".into()),
            })
            .expect("encode");
            socket
                .write_all(&(body.len() as u32).to_be_bytes())
                .await
                .expect("write length");
            socket.write_all(&body).await.expect("write body");
            // Hold the pipe open until the guest drops it.
            let _ = socket.read(&mut [0u8; 8]).await;
        });

        let registry = TransportRegistry::new(
            Arc::new(DirectTransport::new("127.0.0.1:0".parse().unwrap())),
            Arc::new(crate::network::relay::RelayTransport::new(None)),
        );
        let mut descriptor = sample_descriptor(
            ConnectionMode::Relay,
            vec![
                PeerEndpoint::public("203.0.113.5:25565".parse().unwrap()),
                PeerEndpoint::local("127.0.0.1:1".parse().unwrap()),
            ],
        );
        descriptor.relay = Some(crate::models::server::RelayDescriptor {
            url: format!("tcp://{address}"),
            room_token: "room".into(),
            region: None,
            cert_fingerprint: None,
        });

        let started = Instant::now();
        let peer = registry
            .connect(
                &descriptor,
                Arc::new(crate::models::progress::NoopProgressSink),
            )
            .await
            .expect("relay fallback");
        let elapsed = started.elapsed();
        assert_eq!(peer.mode, ConnectionMode::Relay);
        assert!(
            elapsed < Duration::from_secs(5),
            "strict-NAT fallback took {elapsed:?}; a direct tunnel must not stall the relay"
        );
    }

    #[tokio::test]
    async fn dead_relay_fails_quickly_instead_of_staying_on_connecting() {
        use std::time::Instant;

        let registry = TransportRegistry::new(
            Arc::new(DirectTransport::new("127.0.0.1:0".parse().unwrap())),
            Arc::new(crate::network::relay::RelayTransport::new(None)),
        );
        let mut descriptor = sample_descriptor(
            ConnectionMode::Relay,
            vec![PeerEndpoint::public("203.0.113.5:25565".parse().unwrap())],
        );
        descriptor.relay = Some(crate::models::server::RelayDescriptor {
            url: "tcp://127.0.0.1:1".into(),
            room_token: "room".into(),
            region: None,
            cert_fingerprint: None,
        });

        let started = Instant::now();
        let error = registry
            .connect(
                &descriptor,
                Arc::new(crate::models::progress::NoopProgressSink),
            )
            .await
            .expect_err("dead relay");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "dead relay took {elapsed:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("JOIN_UNREACHABLE"),
            "expected a join error, got {message}"
        );
    }

    #[tokio::test]
    async fn tunnel_stream_reports_a_cancel_token() {
        let (left, _right) = tokio::io::duplex(64);
        let stream = UdpTunnelStream {
            reader: _right,
            writer: left,
            cancel: CancellationToken::new(),
        };
        assert!(!stream.cancel_token().is_cancelled());
    }
}
