//! UDP hole punching + NAT behaviour detection.
//!
//! The algorithm, in order:
//!
//! 1. **Discover** our public mapping by sending STUN Binding Requests to two
//!    independent servers. Comparing the two answers tells us whether the NAT is
//!    endpoint-independent (hole punching will work) or symmetric (it will not,
//!    go straight to the relay).
//! 2. **Punch**: send `PUNCH` probes to every candidate endpoint of the peer.
//!    The peer does the same. Both NATs only create the inbound mapping once
//!    each side has sent *outbound* to the other, which is why the probes must
//!    overlap in time — that window is what [`PunchConfig::probe_interval`]
//!    controls.
//! 3. **Confirm** by receiving an `ACK` carrying the session token. From then on
//!    the mapping is open and the datagrams are the tunnel.
//!
//! STUN is implemented inline (RFC 5389 binding request/response) instead of
//! pulling a crate: we need exactly one message type, and the parser is the only
//! thing standing between us and a silent NAT failure, so it is worth having it
//! readable and unit-tested.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use rand::RngCore;
use tokio::net::UdpSocket;

use crate::error::{AppError, AppResult};

/// STUN magic cookie (RFC 5389).
const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const STUN_ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Punch probe magic: `SXPT`.
const PUNCH_MAGIC: [u8; 4] = [0x53, 0x58, 0x50, 0x54];
/// Datagram kinds exchanged during punching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PunchKind {
    Probe = 1,
    Ack = 2,
}

/// Tuning knobs for a punch attempt.
#[derive(Debug, Clone)]
pub struct PunchConfig {
    /// Total time budget for one endpoint.
    pub timeout: Duration,
    /// Delay between probes (kept short: the window is small).
    pub probe_interval: Duration,
    /// STUN servers used for discovery.
    pub stun_servers: Vec<SocketAddr>,
    /// Timeout per STUN request.
    pub stun_timeout: Duration,
}

/// STUN servers used when the player has not overridden them.
///
/// These are hostnames, and `SocketAddr` can only hold a numeric address, so
/// they have to be resolved. Resolution is deferred to first use and cached for
/// the life of the process: building a [`PunchConfig`] must never perform I/O on
/// the startup path, and a second lookup would only repeat the same answer.
const DEFAULT_STUN_HOSTS: [&str; 2] = ["stun.l.google.com:19302", "stun.cloudflare.com:3478"];

static DEFAULT_STUN_SERVERS: std::sync::LazyLock<Vec<SocketAddr>> =
    std::sync::LazyLock::new(|| {
        DEFAULT_STUN_HOSTS
            .iter()
            .filter_map(|host| resolve_blocking(host))
            .collect()
    });

impl Default for PunchConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(8),
            probe_interval: Duration::from_millis(200),
            stun_servers: DEFAULT_STUN_SERVERS.clone(),
            stun_timeout: Duration::from_secs(3),
        }
    }
}

impl PunchConfig {
    /// Build a config from a comma-separated server list (Settings / env).
    pub fn from_servers(servers: &[String]) -> Self {
        let mut config = Self::default();
        let parsed: Vec<SocketAddr> = servers
            .iter()
            .filter_map(|server| {
                // Accept `host:port`: resolve only if it is already an IP.
                server
                    .parse::<SocketAddr>()
                    .ok()
                    .or_else(|| resolve_blocking(server))
            })
            .collect();
        if !parsed.is_empty() {
            config.stun_servers = parsed;
        }
        config
    }
}

/// Blocking DNS for a `host:port` STUN server (bounded, startup-only cost).
fn resolve_blocking(server: &str) -> Option<SocketAddr> {
    use std::net::ToSocketAddrs;
    server.to_socket_addrs().ok()?.next()
}

/// What our NAT does with outbound UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NatBehavior {
    /// Same public port regardless of destination — punching works.
    EndpointIndependent,
    /// Public port depends on the destination — punching usually fails.
    AddressDependent,
    /// Unknown (only one STUN server answered).
    Unknown,
}

impl NatBehavior {
    /// Should we even bother punching?
    pub fn is_punchable(self) -> bool {
        matches!(
            self,
            NatBehavior::EndpointIndependent | NatBehavior::Unknown
        )
    }

    pub fn describe(self) -> &'static str {
        match self {
            NatBehavior::EndpointIndependent => {
                "your router maps UDP consistently; direct connections will work"
            }
            NatBehavior::AddressDependent => {
                "your router uses a strict/symmetric mapping; hosting will fall back to the relay"
            }
            NatBehavior::Unknown => "NAT behaviour could not be fully determined",
        }
    }
}

/// Result of STUN discovery.
#[derive(Debug, Clone, Copy)]
pub struct PublicMapping {
    pub address: SocketAddr,
    pub behavior: NatBehavior,
    pub rtt_ms: u32,
}

/// A punched, ready-to-tunnel path.
#[derive(Debug, Clone, Copy)]
pub struct PunchedChannel {
    pub peer: SocketAddr,
    pub rtt_ms: u32,
    /// Number of probes it took (diagnostics for the connection panel).
    pub attempts: u32,
}

/// Minimal STUN client.
pub struct StunClient;

impl StunClient {
    /// Send a Binding Request and parse the XOR-MAPPED-ADDRESS.
    pub async fn discover(
        socket: &UdpSocket,
        server: SocketAddr,
        timeout: Duration,
    ) -> AppResult<SocketAddr> {
        let transaction_id = random_transaction_id();
        let request = encode_binding_request(&transaction_id);

        let started = Instant::now();
        socket.send_to(&request, server).await.map_err(|err| {
            AppError::Transport(format!("cannot reach STUN server {server}: {err}"))
        })?;

        let mut buffer = [0u8; 512];
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::Transport(format!(
                    "STUN server {server} did not answer within {:?}",
                    timeout
                )));
            }
            let received = tokio::time::timeout(remaining, socket.recv_from(&mut buffer)).await;
            let (read, from) = match received {
                Ok(Ok(value)) => value,
                Ok(Err(err)) => {
                    return Err(AppError::Transport(format!("STUN receive failed: {err}")))
                }
                Err(_) => {
                    return Err(AppError::Transport(format!(
                        "STUN server {server} did not answer within {:?}",
                        timeout
                    )))
                }
            };
            if from != server {
                continue;
            }
            if let Some(mapped) = parse_binding_response(&buffer[..read], &transaction_id) {
                let _ = started;
                return Ok(mapped);
            }
        }
    }

    /// Query two servers to classify the NAT.
    pub async fn classify(
        socket: &UdpSocket,
        servers: &[SocketAddr],
        timeout: Duration,
    ) -> AppResult<PublicMapping> {
        let mut mapped: Vec<SocketAddr> = Vec::new();

        for server in servers.iter().take(2) {
            if let Ok(address) = Self::discover(socket, *server, timeout).await {
                mapped.push(address);
            }
        }

        match mapped.first() {
            None => Err(AppError::Transport(
                "no STUN server was reachable; the launcher cannot discover your public address"
                    .to_string(),
            )),
            Some(first) => {
                let behavior = if mapped.len() < 2 {
                    NatBehavior::Unknown
                } else if mapped.iter().all(|address| address.port() == first.port()) {
                    NatBehavior::EndpointIndependent
                } else {
                    NatBehavior::AddressDependent
                };
                Ok(PublicMapping {
                    address: *first,
                    behavior,
                    rtt_ms: 0,
                })
            }
        }
    }
}

/// Encode a RFC 5389 Binding Request.
fn encode_binding_request(transaction_id: &[u8; 12]) -> [u8; 20] {
    let mut message = [0u8; 20];
    message[0..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    // Message length: no attributes.
    message[2..4].copy_from_slice(&0u16.to_be_bytes());
    message[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    message[8..20].copy_from_slice(transaction_id);
    message
}

/// Parse a Binding Response, extracting XOR-MAPPED-ADDRESS (or MAPPED-ADDRESS).
pub fn parse_binding_response(message: &[u8], transaction_id: &[u8; 12]) -> Option<SocketAddr> {
    if message.len() < 20 {
        return None;
    }
    let message_type = u16::from_be_bytes([message[0], message[1]]);
    if message_type != STUN_BINDING_RESPONSE {
        return None;
    }
    let cookie = u32::from_be_bytes([message[4], message[5], message[6], message[7]]);
    if cookie != STUN_MAGIC_COOKIE {
        return None;
    }
    // A response for a different transaction must be ignored.
    if &message[8..20] != transaction_id {
        return None;
    }

    // The header's length field is authoritative (RFC 5389 §6): if the message
    // claims more attribute bytes than it actually carries, it is truncated and
    // must be discarded. Clamping to `message.len()` here instead would let a
    // partially-received datagram be parsed as a valid answer.
    let attributes_len = u16::from_be_bytes([message[2], message[3]]) as usize;
    let end = 20 + attributes_len;
    if end > message.len() {
        return None;
    }

    let mut offset = 20usize;
    while offset + 4 <= end {
        let attribute_type = u16::from_be_bytes([message[offset], message[offset + 1]]);
        let length = u16::from_be_bytes([message[offset + 2], message[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start + length;
        // An attribute that spills past the declared block is malformed too.
        if value_end > end {
            return None;
        }
        let value = &message[value_start..value_end];

        let address = match attribute_type {
            STUN_ATTR_XOR_MAPPED_ADDRESS => decode_xor_mapped_address(value),
            STUN_ATTR_MAPPED_ADDRESS => decode_mapped_address(value),
            _ => None,
        };
        if address.is_some() {
            return address;
        }

        // Attributes are padded to a 4-byte boundary.
        offset = value_end + ((4 - (length % 4)) % 4);
    }
    None
}

fn decode_xor_mapped_address(value: &[u8]) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let family = value[1];
    let port = u16::from_be_bytes([value[2], value[3]]) ^ (STUN_MAGIC_COOKIE >> 16) as u16;
    match family {
        0x01 => {
            let raw = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
            let ip = Ipv4Addr::from(raw ^ STUN_MAGIC_COOKIE);
            Some(SocketAddr::new(IpAddr::V4(ip), port))
        }
        // IPv6 XOR uses the transaction id in the mask; we only need IPv4 for
        // Minecraft, so an IPv6 answer is reported as unsupported.
        _ => None,
    }
}

fn decode_mapped_address(value: &[u8]) -> Option<SocketAddr> {
    if value.len() < 8 || value[1] != 0x01 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]);
    let ip = Ipv4Addr::new(value[4], value[5], value[6], value[7]);
    Some(SocketAddr::new(IpAddr::V4(ip), port))
}

fn random_transaction_id() -> [u8; 12] {
    let mut id = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut id);
    id
}

/// Exact wire size of a punch datagram: magic(4) + kind(1) + token(32).
pub const PUNCH_DATAGRAM_LEN: usize = 37;

/// Encode a punch datagram: `SXPT | kind(1) | token(32)`.
pub fn encode_punch(kind: PunchKind, token_hash: &[u8; 32]) -> Vec<u8> {
    let mut datagram = Vec::with_capacity(PUNCH_DATAGRAM_LEN);
    datagram.extend_from_slice(&PUNCH_MAGIC);
    datagram.push(kind as u8);
    datagram.extend_from_slice(token_hash);
    datagram
}

/// Decode a punch datagram.
pub fn decode_punch(datagram: &[u8]) -> Option<(PunchKind, [u8; 32])> {
    if datagram.len() != PUNCH_DATAGRAM_LEN || datagram[..4] != PUNCH_MAGIC {
        return None;
    }
    let kind = match datagram[4] {
        1 => PunchKind::Probe,
        2 => PunchKind::Ack,
        _ => return None,
    };
    let mut token = [0u8; 32];
    token.copy_from_slice(&datagram[5..37]);
    Some((kind, token))
}

/// Hash the session token so the raw token never travels inside punch probes.
pub fn token_fingerprint(token: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Punch towards a peer address until it answers.
pub async fn punch(
    socket: &UdpSocket,
    peer: SocketAddr,
    session_token: &str,
) -> AppResult<PunchedChannel> {
    punch_with_config(socket, peer, session_token, &PunchConfig::default()).await
}

/// Punch with explicit tuning (tests use a short timeout).
pub async fn punch_with_config(
    socket: &UdpSocket,
    peer: SocketAddr,
    session_token: &str,
    config: &PunchConfig,
) -> AppResult<PunchedChannel> {
    let peers = std::sync::Arc::new(parking_lot::Mutex::new(vec![peer]));
    punch_candidates(socket, &peers, session_token, config).await
}

/// Punch every candidate in `peers` until one answers.
///
/// The list may grow while the punch is in flight (a signaling answer arrived).
/// A datagram is accepted from whichever address actually replied, as long as
/// it carries this session's token: NAT often rewrites the source port, so
/// requiring `from == peer` drops a punch that otherwise succeeded.
pub async fn punch_candidates(
    socket: &UdpSocket,
    peers: &std::sync::Arc<parking_lot::Mutex<Vec<SocketAddr>>>,
    session_token: &str,
    config: &PunchConfig,
) -> AppResult<PunchedChannel> {
    let token = token_fingerprint(session_token);
    let probe = encode_punch(PunchKind::Probe, &token);
    let ack = encode_punch(PunchKind::Ack, &token);

    let deadline = Instant::now() + config.timeout;
    let started = Instant::now();
    let mut attempts: u32 = 0;
    let mut buffer = [0u8; 512];

    loop {
        if Instant::now() >= deadline {
            let targets = peers.lock().clone();
            let described = if targets.len() == 1 {
                targets[0].to_string()
            } else {
                format!("{} candidates", targets.len())
            };
            return Err(AppError::Transport(format!(
                "JOIN_UNREACHABLE: no answer from {described} after {attempts} hole-punch probes"
            )));
        }

        let targets = peers.lock().clone();
        attempts += 1;
        let mut sent = false;
        let mut send_error: Option<std::io::Error> = None;
        for peer in &targets {
            match socket.send_to(&probe, *peer).await {
                Ok(_) => sent = true,
                // One dead LAN candidate must not abort the loopback or public ones.
                Err(err) => send_error = Some(err),
            }
        }
        if !targets.is_empty() && !sent {
            let err = send_error
                .map(|err| err.to_string())
                .unwrap_or_else(|| "send failed".into());
            return Err(AppError::Transport(format!(
                "cannot send punch probe: {err}"
            )));
        }

        // Wait for either a PROBE (simultaneous open) or an ACK (we were first).
        let received =
            tokio::time::timeout(config.probe_interval, socket.recv_from(&mut buffer)).await;
        match received {
            Ok(Ok((read, from))) => {
                let Some((kind, peer_token)) = decode_punch(&buffer[..read]) else {
                    continue;
                };
                if peer_token != token {
                    // Someone else is probing the same port: not our session.
                    continue;
                }
                if kind == PunchKind::Probe {
                    // Simultaneous open succeeded: confirm so the peer stops.
                    let _ = socket.send_to(&ack, from).await;
                }
                return Ok(PunchedChannel {
                    peer: from,
                    rtt_ms: started.elapsed().as_millis() as u32,
                    attempts,
                });
            }
            // Jitter our own probes a little so two peers do not stay in
            // lockstep. The RNG must be dropped before the await: `ThreadRng` is
            // not `Send`, and holding it across an await point would make this
            // whole future non-`Send` (and therefore unusable from `tokio::spawn`).
            _ => {
                let jitter = u64::from(rand::thread_rng().next_u32() % 40);
                tokio::time::sleep(Duration::from_millis(jitter)).await;
            }
        }
    }
}

/// Bind the socket used for discovery and punching.
pub async fn bind_punch_socket(port: u16) -> AppResult<UdpSocket> {
    bind_punch_socket_on(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)).await
}

/// Bind the punch socket on an explicit address (tests use loopback so two
/// peers in one process can reach each other's published endpoints).
pub async fn bind_punch_socket_on(address: SocketAddr) -> AppResult<UdpSocket> {
    UdpSocket::bind(address)
        .await
        .map_err(|err| AppError::Transport(format!("cannot bind UDP address {address}: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `bind_punch_socket` deliberately binds `0.0.0.0` so it can receive from
    /// any NAT mapping, which makes `local_addr()` report `0.0.0.0:port`.
    /// A datagram cannot be sent *to* `0.0.0.0` (Windows rejects it with
    /// WSAEADDRNOTAVAIL), so the tests point the peers at loopback explicitly.
    fn loopback(address: SocketAddr) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), address.port())
    }

    #[test]
    fn binding_request_is_rfc5389_shaped() {
        let id = [7u8; 12];
        let request = encode_binding_request(&id);
        assert_eq!(request.len(), 20);
        assert_eq!(
            u16::from_be_bytes([request[0], request[1]]),
            STUN_BINDING_REQUEST
        );
        assert_eq!(
            u32::from_be_bytes([request[4], request[5], request[6], request[7]]),
            STUN_MAGIC_COOKIE
        );
        assert_eq!(&request[8..20], &id);
    }

    /// Build a binding response with an XOR-MAPPED-ADDRESS attribute.
    fn response(transaction_id: &[u8; 12], ip: Ipv4Addr, port: u16) -> Vec<u8> {
        let mut message = vec![0u8; 20];
        message[0..2].copy_from_slice(&STUN_BINDING_RESPONSE.to_be_bytes());
        message[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
        message[8..20].copy_from_slice(transaction_id);

        let cookie = STUN_MAGIC_COOKIE;
        let xor_port = port ^ (cookie >> 16) as u16;
        let xor_ip = u32::from(ip) ^ cookie;

        let mut attribute = vec![0u8; 12];
        attribute[0..2].copy_from_slice(&STUN_ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        attribute[2..4].copy_from_slice(&8u16.to_be_bytes());
        attribute[4] = 0;
        attribute[5] = 0x01;
        attribute[6..8].copy_from_slice(&xor_port.to_be_bytes());
        attribute[8..12].copy_from_slice(&xor_ip.to_be_bytes());

        message[2..4].copy_from_slice(&(attribute.len() as u16).to_be_bytes());
        message.extend_from_slice(&attribute);
        message
    }

    #[test]
    fn parses_xor_mapped_address() {
        let id = [3u8; 12];
        let message = response(&id, Ipv4Addr::new(203, 0, 113, 9), 40123);
        let parsed = parse_binding_response(&message, &id).expect("mapped address");
        assert_eq!(parsed.ip().to_string(), "203.0.113.9");
        assert_eq!(parsed.port(), 40123);
    }

    #[test]
    fn ignores_responses_for_other_transactions_or_types() {
        let id = [3u8; 12];
        let message = response(&id, Ipv4Addr::new(203, 0, 113, 9), 40123);

        // Wrong transaction id.
        assert!(parse_binding_response(&message, &[9u8; 12]).is_none());

        // Wrong message type.
        let mut wrong_type = message.clone();
        wrong_type[0] = 0x01;
        wrong_type[1] = 0x11;
        assert!(parse_binding_response(&wrong_type, &id).is_none());

        // Too short / garbage.
        assert!(parse_binding_response(&[], &id).is_none());
        assert!(parse_binding_response(&[1, 2, 3], &id).is_none());
    }

    #[test]
    fn tolerates_a_truncated_attribute_block() {
        let id = [3u8; 12];
        let mut message = response(&id, Ipv4Addr::new(198, 51, 100, 4), 1000);
        // Claim a huge attribute length so the parser must stop instead of panicking.
        message[2..4].copy_from_slice(&60000u16.to_be_bytes());
        assert!(parse_binding_response(&message, &id).is_none());
    }

    #[test]
    fn default_config_never_panics_and_stays_usable() {
        // Regression guard: the defaults were once built with
        // `"host:port".parse::<SocketAddr>()`, which cannot parse a hostname
        // and therefore panicked on *every* `PunchConfig::default()` call —
        // including the one on the app's startup path.
        let config = PunchConfig::default();
        assert!(config.timeout >= config.probe_interval);
        assert!(config.stun_timeout > Duration::ZERO);
        assert!(config
            .stun_servers
            .iter()
            .all(|server| server.port() > 0 && !server.ip().is_unspecified()));
    }

    #[test]
    fn unusable_server_lists_fall_back_to_the_built_ins() {
        let config = PunchConfig::from_servers(&["not a server".to_string()]);
        assert_eq!(config.stun_servers, PunchConfig::default().stun_servers);

        // A numeric override is accepted verbatim.
        let explicit = PunchConfig::from_servers(&["203.0.113.1:3478".to_string()]);
        assert_eq!(explicit.stun_servers.len(), 1);
        assert_eq!(explicit.stun_servers[0].port(), 3478);
    }

    #[test]
    fn nat_behaviour_describes_itself() {
        assert!(NatBehavior::EndpointIndependent.is_punchable());
        assert!(!NatBehavior::AddressDependent.is_punchable());
        assert!(NatBehavior::Unknown.is_punchable());
        assert!(NatBehavior::AddressDependent.describe().contains("relay"));
    }

    #[test]
    fn punch_datagrams_round_trip_and_reject_junk() {
        let token = token_fingerprint("session-token");
        let probe = encode_punch(PunchKind::Probe, &token);
        assert_eq!(decode_punch(&probe), Some((PunchKind::Probe, token)));

        let ack = encode_punch(PunchKind::Ack, &token);
        assert_eq!(
            decode_punch(&ack).map(|(kind, _)| kind),
            Some(PunchKind::Ack)
        );

        assert!(decode_punch(b"not a punch").is_none());
        assert!(decode_punch(&[]).is_none());
        let mut bad_kind = probe.clone();
        bad_kind[4] = 9;
        assert!(decode_punch(&bad_kind).is_none());
    }

    #[test]
    fn token_fingerprint_is_stable_and_token_specific() {
        assert_eq!(token_fingerprint("a"), token_fingerprint("a"));
        assert_ne!(token_fingerprint("a"), token_fingerprint("b"));
        assert_eq!(token_fingerprint("abc").len(), 32);
    }

    #[tokio::test]
    async fn two_peers_punch_through_each_other() {
        // Both sockets live on loopback, which stands in for two NAT'd peers
        // that have already learned each other's public mapping.
        let server = bind_punch_socket(0).await.expect("bind server");
        let client = bind_punch_socket(0).await.expect("bind client");
        let server_addr = loopback(server.local_addr().expect("addr"));
        let client_addr = loopback(client.local_addr().expect("addr"));

        let token = "shared-session-token";
        let server_task = tokio::spawn(async move {
            punch_with_config(
                &server,
                client_addr,
                token,
                &PunchConfig {
                    timeout: Duration::from_secs(3),
                    probe_interval: Duration::from_millis(50),
                    ..PunchConfig::default()
                },
            )
            .await
        });

        // Give the server a head start so we exercise the PROBE -> ACK path.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let client_result = punch_with_config(
            &client,
            server_addr,
            token,
            &PunchConfig {
                timeout: Duration::from_secs(3),
                probe_interval: Duration::from_millis(50),
                ..PunchConfig::default()
            },
        )
        .await;

        assert!(
            client_result.is_ok(),
            "client punch failed: {client_result:?}"
        );
        let server_result = server_task.await.expect("join");
        assert!(
            server_result.is_ok(),
            "server punch failed: {server_result:?}"
        );
    }

    #[tokio::test]
    async fn punching_a_dead_endpoint_times_out() {
        let socket = bind_punch_socket(0).await.expect("bind");
        // 203.0.113.0/24 is TEST-NET-3: guaranteed unroutable.
        let error = punch_with_config(
            &socket,
            "203.0.113.1:25565".parse().expect("addr"),
            "token",
            &PunchConfig {
                timeout: Duration::from_millis(150),
                probe_interval: Duration::from_millis(20),
                ..PunchConfig::default()
            },
        )
        .await
        .expect_err("must time out");
        assert!(error.to_string().contains("no answer"));
    }

    #[tokio::test]
    async fn a_peer_with_a_different_token_is_ignored() {
        let server = bind_punch_socket(0).await.expect("bind server");
        let client = bind_punch_socket(0).await.expect("bind client");
        let server_addr = loopback(server.local_addr().expect("addr"));
        // Read the client address *before* the socket is moved into the task.
        let client_addr = loopback(client.local_addr().expect("addr"));

        // The client finishes its punch, but with a token the server does not know.
        let client_task = tokio::spawn(async move {
            punch_with_config(
                &client,
                server_addr,
                "attacker-token",
                &PunchConfig {
                    timeout: Duration::from_millis(200),
                    probe_interval: Duration::from_millis(30),
                    ..PunchConfig::default()
                },
            )
            .await
        });

        let server_result = punch_with_config(
            &server,
            client_addr,
            "real-token",
            &PunchConfig {
                timeout: Duration::from_millis(200),
                probe_interval: Duration::from_millis(30),
                ..PunchConfig::default()
            },
        )
        .await;

        // Both must fail: the tokens never match.
        assert!(server_result.is_err());
        assert!(client_task.await.expect("join").is_err());
    }
}
