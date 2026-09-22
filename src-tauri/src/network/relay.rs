//! Relay (tunnel) fallback transport.
//!
//! Used when UDP hole punching cannot succeed: symmetric NATs, carrier-grade
//! NAT, corporate firewalls, or a host that asked to be relayed on purpose.
//!
//! Protocol (deliberately tiny so any relay implementation — including an
//! e4mc-style one — can speak it):
//!
//! ```text
//! client -> relay   u32 length | JSON RelayHello
//! relay  -> client  u32 length | JSON RelayResponse
//! after "ok"        the connection is an opaque byte pipe
//! ```
//!
//! After the handshake the relay copies bytes to the matching peer, so this
//! module hands back a plain `TcpStream` as the session's [`DuplexStream`] and
//! the bridge needs no relay-specific knowledge.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::server::{ConnectionDescriptor, ConnectionMode, RelayDescriptor};
use crate::network::transport::{ConnectedPeer, Transport};

/// Wire protocol version.
pub const RELAY_PROTOCOL: u8 = 1;
/// Hard cap on the handshake frame we will read.
const MAX_HANDSHAKE_BYTES: usize = 8 * 1024;
/// How long to wait for the relay's response.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Handshake sent to the relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayHello {
    pub protocol: u8,
    /// Host or guest.
    pub role: RelayRole,
    pub peer_id: String,
    /// Room the two peers share.
    pub room_token: String,
    pub session_token: String,
    /// Asserted Minecraft protocol version, so the relay can reject mismatches
    /// early instead of shuttling bytes that will never parse.
    pub protocol_version: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayRole {
    Host,
    Guest,
}

/// Relay response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Relay-observed address, useful diagnostics for the connection panel.
    #[serde(default)]
    pub observed_address: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
}

/// TCP relay transport.
pub struct RelayTransport {
    /// Override used by tests / self-hosted relays (`SXML_RELAY_URL`).
    default_relay: Option<String>,
}

impl RelayTransport {
    pub fn new(default_relay: Option<String>) -> Self {
        Self { default_relay }
    }
}

#[async_trait]
impl Transport for RelayTransport {
    fn kind(&self) -> &'static str {
        "relay-tcp"
    }

    fn mode(&self) -> ConnectionMode {
        ConnectionMode::Relay
    }

    async fn connect(
        &self,
        descriptor: &ConnectionDescriptor,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<ConnectedPeer> {
        let relay = descriptor
            .relay
            .clone()
            .or_else(|| {
                self.default_relay.as_ref().map(|url| RelayDescriptor {
                    url: url.clone(),
                    room_token: descriptor.session_token.clone(),
                    region: None,
                    cert_fingerprint: None,
                })
            })
            .ok_or_else(|| {
                AppError::Transport(
                    "this session has no relay available, and the direct connection failed"
                        .to_string(),
                )
            })?;

        sink.report(
            ProgressEvent::started(JobKind::P2pConnect, "Connecting through the relay")
                .stage(JobStage::ConnectingP2p)
                .detail(&relay.url),
        )
        .await;

        let address = parse_relay_address(&relay.url)?;
        let started = Instant::now();
        let mut socket = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(address))
            .await
            .map_err(|_| {
                AppError::Transport(format!("the relay at {address} did not accept a connection"))
            })?
            .map_err(|err| AppError::Transport(format!("cannot reach the relay: {err}")))?;

        // Keep-alive so a NAT middlebox does not silently drop an idle session.
        let _ = socket.set_nodelay(true);

        let hello = RelayHello {
            protocol: RELAY_PROTOCOL,
            role: RelayRole::Guest,
            peer_id: descriptor.peer_id.clone(),
            room_token: relay.room_token.clone(),
            session_token: descriptor.session_token.clone(),
            protocol_version: descriptor.protocol_version,
        };

        let response = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            handshake(&mut socket, &hello),
        )
        .await
        .map_err(|_| AppError::Transport("the relay did not answer the handshake".to_string()))??;

        if !response.ok {
            return Err(AppError::Transport(response.error.unwrap_or_else(|| {
                "the relay refused this session".to_string()
            })));
        }

        let rtt_ms = Some(started.elapsed().as_millis() as u32);
        Ok(ConnectedPeer {
            peer_id: descriptor.peer_id.clone(),
            mode: ConnectionMode::Relay,
            remote: Some(address),
            rtt_ms,
            stream: Box::new(socket),
            cancel: CancellationToken::new(),
        })
    }

    async fn probe(&self, descriptor: &ConnectionDescriptor) -> AppResult<Option<u32>> {
        // Build an owned descriptor: borrowing a temporary here would not live
        // long enough to use it below.
        let relay: Option<RelayDescriptor> =
            descriptor.relay.clone().or_else(|| {
                self.default_relay.as_ref().map(|url| RelayDescriptor {
                    url: url.clone(),
                    room_token: String::new(),
                    region: None,
                    cert_fingerprint: None,
                })
            });
        let Some(relay) = relay else {
            return Ok(None);
        };
        let address = parse_relay_address(&relay.url)?;
        let started = Instant::now();
        let socket = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(address))
            .await
            .map_err(|_| AppError::Transport("relay connect timed out".to_string()))??;
        drop(socket);
        Ok(Some(started.elapsed().as_millis() as u32))
    }
}

/// Perform the length-prefixed JSON handshake.
pub async fn handshake(socket: &mut TcpStream, hello: &RelayHello) -> AppResult<RelayResponse> {
    let payload = serde_json::to_vec(hello)?;
    if payload.len() > MAX_HANDSHAKE_BYTES {
        return Err(AppError::Transport("relay handshake too large".to_string()));
    }

    socket
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|err| AppError::Transport(format!("relay write failed: {err}")))?;
    socket
        .write_all(&payload)
        .await
        .map_err(|err| AppError::Transport(format!("relay write failed: {err}")))?;
    socket.flush().await.ok();

    let mut length_bytes = [0u8; 4];
    socket
        .read_exact(&mut length_bytes)
        .await
        .map_err(|err| AppError::Transport(format!("relay closed the connection: {err}")))?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_HANDSHAKE_BYTES {
        return Err(AppError::Transport(
            "the relay sent a malformed handshake response".to_string(),
        ));
    }

    let mut buffer = vec![0u8; length];
    socket
        .read_exact(&mut buffer)
        .await
        .map_err(|err| AppError::Transport(format!("relay response truncated: {err}")))?;

    serde_json::from_slice(&buffer)
        .map_err(|err| AppError::Transport(format!("unreadable relay response: {err}")))
}

/// Extract a dialable `SocketAddr` from a relay URL.
///
/// Accepts `ws://`, `wss://`, `tcp://` and bare `host:port`. `wss` defaults to
/// 443 because the production relay terminates TLS in front of the raw tunnel.
pub fn parse_relay_address(url: &str) -> AppResult<std::net::SocketAddr> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);

    let is_tls = trimmed.starts_with("wss://") || trimmed.starts_with("https://");
    let with_port = if authority.contains(':') {
        authority.to_string()
    } else if is_tls {
        format!("{authority}:443")
    } else {
        format!("{authority}:80")
    };

    use std::net::ToSocketAddrs;
    with_port
        .to_socket_addrs()
        .map_err(|err| AppError::Transport(format!("cannot resolve relay `{url}`: {err}")))?
        .next()
        .ok_or_else(|| AppError::Transport(format!("the relay `{url}` did not resolve to an IP")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn relay_urls_parse_with_and_without_schemes() {
        assert_eq!(
            parse_relay_address("tcp://127.0.0.1:9000").expect("addr").to_string(),
            "127.0.0.1:9000"
        );
        assert_eq!(
            parse_relay_address("127.0.0.1:9000").expect("addr").to_string(),
            "127.0.0.1:9000"
        );
        // wss defaults to 443, ws to 80.
        assert!(parse_relay_address("wss://127.0.0.1").is_ok());
        assert_eq!(
            parse_relay_address("wss://127.0.0.1").expect("addr").port(),
            443
        );
        assert_eq!(parse_relay_address("ws://127.0.0.1").expect("addr").port(), 80);
        // A path must be ignored.
        assert_eq!(
            parse_relay_address("wss://127.0.0.1:9443/session")
                .expect("addr")
                .port(),
            9443
        );
    }

    #[tokio::test]
    async fn handshake_exchanges_the_hello_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut length_bytes = [0u8; 4];
            socket.read_exact(&mut length_bytes).await.expect("length");
            let mut payload = vec![0u8; u32::from_be_bytes(length_bytes) as usize];
            socket.read_exact(&mut payload).await.expect("payload");
            let hello: RelayHello = serde_json::from_slice(&payload).expect("hello");

            let response = RelayResponse {
                ok: hello.session_token == "good-token",
                error: Some("bad token".into()),
                observed_address: Some("198.51.100.7:54321".into()),
                region: Some("eu-central".into()),
            };
            let body = serde_json::to_vec(&response).expect("encode");
            socket
                .write_all(&(body.len() as u32).to_be_bytes())
                .await
                .expect("write length");
            socket.write_all(&body).await.expect("write body");
            hello
        });

        let mut client = TcpStream::connect(address).await.expect("connect");
        let hello = RelayHello {
            protocol: RELAY_PROTOCOL,
            role: RelayRole::Guest,
            peer_id: "peer".into(),
            room_token: "room".into(),
            session_token: "good-token".into(),
            protocol_version: 763,
        };
        let response = handshake(&mut client, &hello).await.expect("handshake");

        assert!(response.ok);
        assert_eq!(response.region.as_deref(), Some("eu-central"));
        let received = server.await.expect("join");
        assert_eq!(received.protocol, RELAY_PROTOCOL);
        assert_eq!(received.protocol_version, 763);
    }

    #[tokio::test]
    async fn refused_sessions_return_the_relay_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut length_bytes = [0u8; 4];
            let _ = socket.read_exact(&mut length_bytes).await;
            let mut payload = vec![0u8; u32::from_be_bytes(length_bytes) as usize];
            let _ = socket.read_exact(&mut payload).await;

            let body = serde_json::to_vec(&RelayResponse {
                ok: false,
                error: Some("room is full".into()),
                observed_address: None,
                region: None,
            })
            .expect("encode");
            let _ = socket.write_all(&(body.len() as u32).to_be_bytes()).await;
            let _ = socket.write_all(&body).await;
        });

        let transport = RelayTransport::new(Some(format!("tcp://{address}")));
        let descriptor = ConnectionDescriptor {
            mode: ConnectionMode::Relay,
            peer_id: "peer".into(),
            public_key: "key".into(),
            endpoints: vec![],
            relay: None,
            session_token: "token".into(),
            protocol_version: 763,
        };
        let error = transport
            .connect(
                &descriptor,
                std::sync::Arc::new(crate::models::progress::NoopProgressSink),
            )
            .await
            .expect_err("must refuse");
        assert!(error.to_string().contains("room is full"));
    }

    #[tokio::test]
    async fn missing_relay_and_unreachable_relay_fail_clearly() {
        let transport = RelayTransport::new(None);
        let descriptor = ConnectionDescriptor {
            mode: ConnectionMode::Relay,
            peer_id: "peer".into(),
            public_key: "key".into(),
            endpoints: vec![],
            relay: None,
            session_token: "token".into(),
            protocol_version: 763,
        };
        let error = transport
            .connect(
                &descriptor,
                std::sync::Arc::new(crate::models::progress::NoopProgressSink),
            )
            .await
            .expect_err("must fail");
        assert!(error.to_string().contains("no relay available"));

        // Port 1 on loopback refuses immediately.
        let unreachable = RelayTransport::new(Some("tcp://127.0.0.1:1".into()));
        assert!(unreachable
            .connect(
                &descriptor,
                std::sync::Arc::new(crate::models::progress::NoopProgressSink)
            )
            .await
            .is_err());
    }
}
