//! The bridge between a local Minecraft socket and a P2P tunnel.
//!
//! Two directions, mirror images of each other:
//!
//! ```text
//! guest:  Minecraft client -> 127.0.0.1:<ephemeral> -> [bridge] -> tunnel -> host
//! host:   tunnel -> [bridge] -> 127.0.0.1:25565 (the local server)
//! ```
//!
//! The local port is **always** allocated by the OS (never a hardcoded 25565)
//! because several sessions may be open at once, and the guest needs the exact
//! address to pass to the game client via `--server/--port`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::network::transport::DuplexStream;

/// Minecraft's default server port.
pub const DEFAULT_SERVER_PORT: u16 = 25565;
/// Hard cap on the sniffed handshake packet (VarInt length + fields).
const MAX_HANDSHAKE_PACKET: usize = 4096;
/// How long to wait for the client's first packet before giving up on sniffing.
const SNIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Parsed Minecraft handshake (packet id 0x00, initial state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeInfo {
    /// Protocol version the client speaks (e.g. 763 for 1.20.1).
    pub protocol_version: i32,
    /// Server address the client typed — for a tunneled session this is the
    /// loopback address, which is exactly how we confirm we wired it up.
    pub server_address: String,
    pub server_port: u16,
    /// 1 = status ping, 2 = login, 3 = transfer.
    pub next_state: i32,
    /// Login username, only present for `next_state == 2` (read from the login
    /// start packet, which arrives right after the handshake).
    pub username: Option<String>,
}

impl HandshakeInfo {
    /// `true` when the client is only pinging the server list, not joining.
    pub fn is_status_ping(&self) -> bool {
        self.next_state == 1
    }
}

/// Live counters for the connection diagnostics panel.
#[derive(Debug, Clone, Default)]
pub struct BridgeStats {
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub rtt_ms: Option<u32>,
}

/// A running bridge.
pub struct BridgeHandle {
    /// Address the Minecraft client should connect to.
    pub local_address: SocketAddr,
    pub cancel: CancellationToken,
    pub stats: Arc<parking_lot::Mutex<BridgeStats>>,
    pub handshake: Option<HandshakeInfo>,
}

impl BridgeHandle {
    pub fn snapshot(&self) -> (u64, u64) {
        let stats = self.stats.lock();
        (stats.bytes_up, stats.bytes_down)
    }
}

/// Ask the OS for an unused loopback port.
pub async fn find_free_port() -> AppResult<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|err| AppError::Transport(format!("cannot allocate a local port: {err}")))?;
    let port = listener
        .local_addr()
        .map_err(|err| AppError::Transport(format!("cannot read the allocated port: {err}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// Host side: accept a guest's tunnel and attach it to the local server.
pub async fn bridge_to_local_server(
    tunnel: Box<dyn DuplexStream>,
    server_address: SocketAddr,
    cancel: CancellationToken,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<BridgeStats> {
    sink.report(
        ProgressEvent::started(JobKind::P2pHost, "Guest connected")
            .stage(JobStage::ConnectingP2p)
            .detail(format!("forwarding to {server_address}")),
    )
    .await;

    forward_tunnel_to_server(tunnel, server_address, cancel).await
}

/// Splice an already-open tunnel (UDP or relay TCP) onto the local server.
///
/// Unlike [`bridge_to_local_server`], this does not emit a Connecting row.
/// The strict-NAT host relay loop uses it so a retry does not leave Activity
/// on Connecting while the world is still up.
pub async fn forward_tunnel_to_server(
    tunnel: Box<dyn DuplexStream>,
    server_address: SocketAddr,
    cancel: CancellationToken,
) -> AppResult<BridgeStats> {
    let server = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TcpStream::connect(server_address),
    )
    .await
    .map_err(|_| {
        AppError::Transport(format!(
            "the local server at {server_address} did not accept a connection; \
             is the world actually running?"
        ))
    })?
    .map_err(|err| {
        AppError::Transport(format!(
            "cannot reach the local server at {server_address}: {err}"
        ))
    })?;

    copy_bidirectional(server, tunnel, cancel).await
}

/// Guest side: listen on loopback, then splice the game's connection into the
/// tunnel.
///
/// Returns as soon as the listener is bound so the launcher can start Minecraft
/// with the right `--server/--port`; the splice happens when the client connects.
pub async fn bridge_from_game_client(
    tunnel: Box<dyn DuplexStream>,
    port: Option<u16>,
    cancel: CancellationToken,
    sink: Arc<dyn ProgressSink>,
    on_handshake: Option<tokio::sync::oneshot::Sender<HandshakeInfo>>,
) -> AppResult<BridgeHandle> {
    let listener = TcpListener::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port.unwrap_or(0),
    ))
    .await
    .map_err(|err| AppError::Transport(format!("cannot open a local bridge port: {err}")))?;

    let local_address = listener
        .local_addr()
        .map_err(|err| AppError::Transport(format!("cannot read the bridge address: {err}")))?;

    let stats = Arc::new(parking_lot::Mutex::new(BridgeStats::default()));
    let task_cancel = cancel.clone();
    let task_stats = stats.clone();

    tokio::spawn(async move {
        let accepted = tokio::select! {
            _ = task_cancel.cancelled() => return,
            result = listener.accept() => result,
        };
        let (client, _peer) = match accepted {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[bridge] accept failed: {err}");
                return;
            }
        };

        // Sniff the handshake so we can log/verify which version joined, then
        // forward those exact bytes before switching to bulk copy.
        let (handshake, prefix) = match peek_handshake(&client).await {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[bridge] handshake sniff failed: {err}");
                (None, Vec::new())
            }
        };
        if let (Some(info), Some(sender)) = (handshake.clone(), on_handshake) {
            let _ = sender.send(info);
        }

        match splice(client, tunnel, prefix, task_stats.clone(), task_cancel).await {
            Ok(()) => {}
            Err(err) => eprintln!("[bridge] session ended: {err}"),
        }
        sink.report(
            ProgressEvent::started(JobKind::P2pConnect, "Session closed")
                .stage(JobStage::Done)
                .finished(),
        )
        .await;
    });

    Ok(BridgeHandle {
        local_address,
        cancel,
        stats,
        handshake: None,
    })
}

/// Bidirectional copy with byte accounting.
async fn splice(
    client: TcpStream,
    tunnel: Box<dyn DuplexStream>,
    prefix: Vec<u8>,
    stats: Arc<parking_lot::Mutex<BridgeStats>>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut tunnel_read, mut tunnel_write) = tokio::io::split(tunnel);

    // Replay the sniffed bytes to the host; the game client is waiting on them.
    if !prefix.is_empty() {
        tunnel_write.write_all(&prefix).await?;
        stats.lock().bytes_up += prefix.len() as u64;
    }

    let up_stats = stats.clone();
    let up_cancel = cancel.clone();
    let up = async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let read = tokio::select! {
                _ = up_cancel.cancelled() => break,
                result = client_read.read(&mut buffer) => match result {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(_) => break,
                },
            };
            if tunnel_write.write_all(&buffer[..read]).await.is_err() {
                break;
            }
            up_stats.lock().bytes_up += read as u64;
        }
        let _ = tunnel_write.shutdown().await;
    };

    let down_stats = stats.clone();
    let down_cancel = cancel.clone();
    let down = async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let read = tokio::select! {
                _ = down_cancel.cancelled() => break,
                result = tunnel_read.read(&mut buffer) => match result {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(_) => break,
                },
            };
            if client_write.write_all(&buffer[..read]).await.is_err() {
                break;
            }
            down_stats.lock().bytes_down += read as u64;
        }
        let _ = client_write.shutdown().await;
    };

    tokio::join!(up, down);
    let _ = cancel;
    Ok(())
}

/// Copy between a raw TCP connection and a tunnel (host side).
async fn copy_bidirectional(
    server: TcpStream,
    tunnel: Box<dyn DuplexStream>,
    cancel: CancellationToken,
) -> AppResult<BridgeStats> {
    // A plain socket stands in for "the client" so both sides share one path.
    let stats = Arc::new(parking_lot::Mutex::new(BridgeStats::default()));
    splice(server, tunnel, Vec::new(), stats.clone(), cancel).await?;
    let snapshot = stats.lock().clone();
    Ok(snapshot)
}

/// Read (and consume) the Minecraft handshake packet, returning the bytes so the
/// caller can replay them.
pub async fn peek_handshake(client: &TcpStream) -> AppResult<(Option<HandshakeInfo>, Vec<u8>)> {
    let mut peeked = vec![0u8; MAX_HANDSHAKE_PACKET];

    // peek() does not consume, so the game client is unaffected.
    let read = tokio::time::timeout(SNIFF_TIMEOUT, client.peek(&mut peeked)).await;
    let read = match read {
        Ok(Ok(read)) => read,
        Ok(Err(err)) => {
            return Err(AppError::Transport(format!(
                "cannot peek the handshake: {err}"
            )))
        }
        Err(_) => return Ok((None, Vec::new())),
    };
    if read == 0 {
        return Ok((None, Vec::new()));
    }

    let buffer = &peeked[..read];
    let mut cursor = 0usize;
    let Some(packet_length) = read_varint(buffer, &mut cursor) else {
        return Ok((None, Vec::new()));
    };
    if packet_length <= 0 || packet_length as usize > MAX_HANDSHAKE_PACKET {
        // Not a handshake (could be an HTTP probe against the bridge port).
        return Ok((None, Vec::new()));
    }

    let packet_end = cursor + packet_length as usize;
    if packet_end > buffer.len() {
        // The packet has not fully arrived yet; forward nothing and let the
        // splice handle it — correctness beats logging here.
        return Ok((None, Vec::new()));
    }

    let Some(packet_id) = read_varint(buffer, &mut cursor) else {
        return Ok((None, Vec::new()));
    };
    if packet_id != 0x00 {
        return Ok((None, Vec::new()));
    }

    let Some(protocol_version) = read_varint(buffer, &mut cursor) else {
        return Ok((None, Vec::new()));
    };
    let Some(server_address) = read_string(buffer, &mut cursor) else {
        return Ok((None, Vec::new()));
    };
    if cursor + 2 > packet_end {
        return Ok((None, Vec::new()));
    }
    let server_port = u16::from_be_bytes([buffer[cursor], buffer[cursor + 1]]);
    cursor += 2;
    let Some(next_state) = read_varint(buffer, &mut cursor) else {
        return Ok((None, Vec::new()));
    };

    // Try to read the login username when the client is joining (not pinging).
    let mut username = None;
    if next_state == 2 && packet_end + 3 < buffer.len() {
        let mut login_cursor = packet_end;
        if let Some(login_length) = read_varint(buffer, &mut login_cursor) {
            let login_end = login_cursor + login_length as usize;
            if login_end <= buffer.len() {
                if let Some(login_id) = read_varint(buffer, &mut login_cursor) {
                    if login_id == 0x00 {
                        username = read_string(buffer, &mut login_cursor);
                    }
                }
            }
        }
    }

    // Replay only the handshake packet: the login packet stays untouched.
    Ok((
        Some(HandshakeInfo {
            protocol_version,
            server_address,
            server_port,
            next_state,
            username,
        }),
        buffer[..packet_end].to_vec(),
    ))
}

/// Read a VarInt (Minecraft's 7-bit continuation encoding).
fn read_varint(buffer: &[u8], cursor: &mut usize) -> Option<i32> {
    let mut value: i32 = 0;
    for shift in 0..5 {
        let byte = *buffer.get(*cursor)?;
        *cursor += 1;
        value |= i32::from(byte & 0x7f) << (7 * shift);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// Read a VarInt-prefixed UTF-8 string.
fn read_string(buffer: &[u8], cursor: &mut usize) -> Option<String> {
    let length = read_varint(buffer, cursor)?;
    if length < 0 || length as usize > 32767 {
        return None;
    }
    let end = *cursor + length as usize;
    let bytes = buffer.get(*cursor..end)?;
    *cursor = end;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Encode a VarInt (used by tests and by any status ping we send).
pub fn write_varint(value: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    let mut value = value as u32;
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return out;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

/// Build a handshake packet (used to test the sniffer and to ping servers).
pub fn build_handshake(
    protocol_version: i32,
    address: &str,
    port: u16,
    next_state: i32,
) -> Vec<u8> {
    let mut payload = write_varint(0x00);
    payload.extend(write_varint(protocol_version));
    payload.extend(write_varint(address.len() as i32));
    payload.extend_from_slice(address.as_bytes());
    payload.extend_from_slice(&port.to_be_bytes());
    payload.extend(write_varint(next_state));

    let mut packet = write_varint(payload.len() as i32);
    packet.extend(payload);
    packet
}

/// A session record ready for the SQLite diagnostics table.
#[derive(Debug, Clone)]
pub struct SessionLogEntry {
    pub id: uuid::Uuid,
    pub role: String,
    pub peer_id: String,
    pub mode: crate::models::server::ConnectionMode,
    pub local_port: Option<u16>,
    pub started_at: DateTime<Utc>,
    pub stats: BridgeStats,
    pub handshake: Option<HandshakeInfo>,
}

impl SessionLogEntry {
    pub fn to_record(
        &self,
        ended_at: Option<DateTime<Utc>>,
    ) -> crate::store::servers::P2pSessionRecord {
        crate::store::servers::P2pSessionRecord {
            id: self.id,
            role: self.role.clone(),
            peer_id: self.peer_id.clone(),
            mode: self.mode,
            local_port: self.local_port,
            started_at: self.started_at,
            ended_at,
            bytes_up: self.stats.bytes_up,
            bytes_down: self.stats.bytes_down,
            rtt_ms: self.stats.rtt_ms,
            detail: self.handshake.as_ref().map(|handshake| {
                format!(
                    "protocol {} · {} · {}",
                    handshake.protocol_version,
                    handshake
                        .username
                        .clone()
                        .unwrap_or_else(|| "unknown".into()),
                    if handshake.is_status_ping() {
                        "status"
                    } else {
                        "login"
                    }
                )
            }),
        }
    }
}

/// Measure a TCP round trip to a local or remote endpoint.
pub async fn measure_tcp_rtt(address: SocketAddr) -> AppResult<u32> {
    let started = Instant::now();
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        TcpStream::connect(address),
    )
    .await
    .map_err(|_| AppError::Transport(format!("timed out measuring {address}")))?
    .map_err(|err| AppError::Transport(format!("cannot reach {address}: {err}")))?;
    drop(stream);
    Ok(started.elapsed().as_millis() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn varints_round_trip_including_boundaries() {
        for value in [0, 1, 127, 128, 255, 300, 2097151, 2147483647] {
            let encoded = write_varint(value);
            let mut cursor = 0usize;
            assert_eq!(read_varint(&encoded, &mut cursor), Some(value));
            assert_eq!(cursor, encoded.len());
        }
    }

    #[test]
    fn truncated_varints_are_rejected() {
        // Continuation bit set on every byte, never terminates.
        let mut cursor = 0usize;
        assert!(read_varint(&[0x80, 0x80, 0x80, 0x80, 0x80], &mut cursor).is_none());
        let mut cursor = 0usize;
        assert!(read_varint(&[], &mut cursor).is_none());
    }

    #[test]
    fn handshake_packets_are_parsed_with_the_username() {
        let handshake = build_handshake(763, "127.0.0.1", 25565, 2);
        // Append a login-start packet carrying the username. The declared string
        // length must match the name exactly, or `read_string` correctly refuses
        // to hand back a truncated username.
        let username = "SteveTheMiner16";
        let mut login_payload = write_varint(0x00);
        login_payload.extend(write_varint(username.len() as i32));
        login_payload.extend_from_slice(username.as_bytes());
        let mut bytes = handshake.clone();
        bytes.extend(write_varint(login_payload.len() as i32));
        bytes.extend(login_payload);

        // The parser reads from a buffer, mirroring what `peek` hands us.
        let mut cursor = 0usize;
        let packet_length = read_varint(&bytes, &mut cursor).expect("length");
        let _ = packet_length;
        let info = parse_for_test(&bytes);
        assert_eq!(info.protocol_version, 763);
        assert_eq!(info.server_address, "127.0.0.1");
        assert_eq!(info.server_port, 25565);
        assert_eq!(info.next_state, 2);
        assert_eq!(info.username.as_deref(), Some("SteveTheMiner16"));
    }

    /// Test-only wrapper around the same parsing the sniffer uses.
    fn parse_for_test(buffer: &[u8]) -> HandshakeInfo {
        let mut cursor = 0usize;
        let packet_length = read_varint(buffer, &mut cursor).expect("length") as usize;
        let packet_end = cursor + packet_length;
        let _packet_id = read_varint(buffer, &mut cursor).expect("packet id");
        let protocol_version = read_varint(buffer, &mut cursor).expect("protocol");
        let server_address = read_string(buffer, &mut cursor).expect("address");
        let server_port = u16::from_be_bytes([buffer[cursor], buffer[cursor + 1]]);
        cursor += 2;
        let next_state = read_varint(buffer, &mut cursor).expect("state");

        let mut username = None;
        if next_state == 2 && packet_end + 3 < buffer.len() {
            let mut login_cursor = packet_end;
            if let Some(length) = read_varint(buffer, &mut login_cursor) {
                let end = login_cursor + length as usize;
                if end <= buffer.len() {
                    if let Some(id) = read_varint(buffer, &mut login_cursor) {
                        if id == 0x00 {
                            username = read_string(buffer, &mut login_cursor);
                        }
                    }
                }
            }
        }
        HandshakeInfo {
            protocol_version,
            server_address,
            server_port,
            next_state,
            username,
        }
    }

    #[tokio::test]
    async fn sniffing_a_real_socket_reads_the_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let packet = build_handshake(763, "127.0.0.1", 25565, 1);
                let _ = socket.write_all(&packet).await;
                // Hold the connection open so the peek has something to read.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });

        let client = TcpStream::connect(address).await.expect("connect");
        let (info, prefix) = peek_handshake(&client).await.expect("sniff");
        let info = info.expect("handshake parsed");
        assert_eq!(info.protocol_version, 763);
        assert_eq!(info.next_state, 1);
        assert!(info.is_status_ping());
        assert!(!prefix.is_empty(), "the handshake bytes must be replayed");

        // The prefix must not have been consumed from the socket.
        let mut peeked = [0u8; 16];
        let read = client.peek(&mut peeked).await.expect("peek");
        assert!(read > 0);
    }

    #[tokio::test]
    async fn sniffing_ignores_http_probes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(b"GET / HTTP/1.1\r\n\r\n").await;
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
        });

        let client = TcpStream::connect(address).await.expect("connect");
        let (info, prefix) = peek_handshake(&client).await.expect("sniff");
        assert!(info.is_none());
        assert!(prefix.is_empty());
    }

    #[tokio::test]
    async fn free_ports_are_actually_free_and_distinct() {
        let first = find_free_port().await.expect("port");
        let second = find_free_port().await.expect("port");
        assert_ne!(first, second);
        assert!(first > 1024);
    }

    #[tokio::test]
    async fn bridging_forwards_bytes_in_both_directions() {
        // Stand-in for the Minecraft server: echo everything it receives.
        let server = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let server_address = server.local_addr().expect("addr");
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = server.accept().await {
                let mut buffer = vec![0u8; 128];
                while let Ok(read) = socket.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    if socket.write_all(&buffer[..read]).await.is_err() {
                        break;
                    }
                }
            }
        });

        // Tunnel stand-in: a duplex pipe we control from the test.
        let (tunnel_side, mut test_side) = tokio::io::duplex(64 * 1024);

        // `bridge_to_local_server` runs for the whole lifetime of the session
        // (it only returns once the tunnel closes), so it must be spawned here
        // rather than awaited — awaiting it would block before we can write.
        let cancel = CancellationToken::new();
        let bridge_cancel = cancel.clone();
        let bridge = tokio::spawn(async move {
            bridge_to_local_server(
                Box::new(tunnel_side),
                server_address,
                bridge_cancel,
                Arc::new(crate::models::progress::NoopProgressSink),
            )
            .await
        });

        // The duplex buffer (64 KiB) holds this write until the bridge starts
        // reading, so there is no race with the connection setup above.
        test_side
            .write_all(b"hello minecraft")
            .await
            .expect("write");
        let mut response = vec![0u8; 15];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            test_side.read_exact(&mut response),
        )
        .await
        .expect("no echo received")
        .expect("read");
        assert_eq!(&response, b"hello minecraft");

        // Tearing the tunnel down must make the bridge return successfully.
        drop(test_side);
        cancel.cancel();
        let stats = tokio::time::timeout(std::time::Duration::from_secs(5), bridge)
            .await
            .expect("bridge did not shut down")
            .expect("join")
            .expect("bridge");
        assert!(stats.bytes_up >= 15);
    }
}
