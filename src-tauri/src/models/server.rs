//! The **Redis Global Server Browser** payload.
//!
//! Wire format contract (shared with any third-party browser UI):
//!
//! ```text
//! sxml:servers:index       ZSET    server_id -> heartbeat unix ms
//! sxml:servers:{id}        STRING  ServerListing JSON, EXPIRE = ttl_secs
//! sxml:signal:{peer_id}    PUBSUB  SignalingEnvelope JSON (hole punch / WebRTC)
//! sxml:heartbeat:{id}      PUBSUB  ServerHeartbeat JSON (lobby liveness hints)
//! sxml:code:{code}         STRING  server_id, EXPIRE = 300s (direct connect codes)
//! ```
//!
//! A server disappears from the browser automatically: the listing key expires
//! after `ttl_secs`, and the index entry is pruned on every browse by comparing
//! heartbeat timestamps against the current time.

use std::net::SocketAddr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::instance::LoaderKind;

/// Payload schema version — bump when a field changes meaning.
pub const SERVER_LISTING_SCHEMA: u8 = 1;
/// Aggressive TTL: a host must refresh every 30s to stay visible.
pub const DEFAULT_HEARTBEAT_TTL_SECS: u32 = 30;
/// How often the host node refreshes its listing.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// How the guest will reach the host's Minecraft server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    /// UDP hole punching succeeded, traffic is peer-to-peer.
    DirectP2p,
    /// Hole punching failed (symmetric NAT); traffic is relayed.
    Relay,
    /// Reachable at a public address (self-hosted with port forwarding).
    Dedicated,
}

impl ConnectionMode {
    /// Stable string used in SQLite and in the connect code.
    ///
    /// Matches the serde `snake_case` representation exactly, so a value written
    /// to the database and the same value on the wire are byte-identical.
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectionMode::DirectP2p => "direct_p2p",
            ConnectionMode::Relay => "relay",
            ConnectionMode::Dedicated => "dedicated",
        }
    }

    /// Parse the persisted/Wire form.
    pub fn from_str_opt(raw: &str) -> Option<Self> {
        match raw {
            "direct_p2p" => Some(ConnectionMode::DirectP2p),
            "relay" => Some(ConnectionMode::Relay),
            "dedicated" => Some(ConnectionMode::Dedicated),
            _ => None,
        }
    }

    /// `true` when the session is exposed at a public address rather than
    /// tunnelled through the relay.
    pub fn is_direct(self) -> bool {
        !matches!(self, ConnectionMode::Relay)
    }
}

impl std::fmt::Display for ConnectionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ConnectionMode {
    type Err = crate::error::AppError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        ConnectionMode::from_str_opt(raw).ok_or_else(|| {
            crate::error::AppError::Config(format!("unknown connection mode: {raw}"))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    /// Public address observed by a STUN server.
    Public,
    /// LAN address, used for same-network joins.
    Local,
    /// Relay ingress address.
    Relay,
}

impl EndpointKind {
    /// Lower is better; the connector tries endpoints in this order.
    pub fn priority(self) -> u8 {
        match self {
            EndpointKind::Local => 0,
            EndpointKind::Public => 1,
            EndpointKind::Relay => 2,
        }
    }
}

/// One candidate address for the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEndpoint {
    pub kind: EndpointKind,
    pub addr: SocketAddr,
    /// Only usable for a limited time (hole-punched mappings expire).
    pub expires_at: Option<DateTime<Utc>>,
}

impl PeerEndpoint {
    pub fn public(addr: SocketAddr) -> Self {
        Self {
            kind: EndpointKind::Public,
            addr,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(120)),
        }
    }

    pub fn local(addr: SocketAddr) -> Self {
        Self {
            kind: EndpointKind::Local,
            addr,
            expires_at: None,
        }
    }

    pub fn relay(addr: SocketAddr) -> Self {
        Self {
            kind: EndpointKind::Relay,
            addr,
            expires_at: None,
        }
    }
}

/// Relay/tunnel fallback descriptor (e4mc-style or a self-hosted TURN node).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayDescriptor {
    pub url: String,
    /// Short-lived room token minted by the relay; opaque to the client.
    pub room_token: String,
    pub region: Option<String>,
    /// Certificate pin for self-hosted relays (`sha256` of the leaf cert).
    pub cert_fingerprint: Option<String>,
}

/// Everything a peer needs to attempt a connection, without any secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDescriptor {
    pub mode: ConnectionMode,
    /// libp2p-style peer id (base58) / UUID of the host node.
    pub peer_id: String,
    /// Ed25519 public key (base64) used to verify signaling envelopes.
    pub public_key: String,
    /// Ordered candidate addresses.
    pub endpoints: Vec<PeerEndpoint>,
    pub relay: Option<RelayDescriptor>,
    /// Shared secret-less session token required by the tunnel handshake.
    pub session_token: String,
    /// Minecraft protocol version the host speaks (for the version badge).
    pub protocol_version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCount {
    pub online: u32,
    pub max: u32,
}

impl PlayerCount {
    pub fn is_full(&self) -> bool {
        self.max > 0 && self.online >= self.max
    }

    pub fn free_slots(&self) -> u32 {
        self.max.saturating_sub(self.online)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerOwner {
    pub name: String,
    /// Minecraft UUID of the account hosting the session.
    pub uuid: Uuid,
    pub provider: crate::models::account::AccountProvider,
}

/// Modpack reference shown as a badge and used for compatibility checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackRef {
    pub source: crate::models::modpack::ModSource,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version_number: String,
}

/// Who may join.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhitelistPolicy {
    pub enabled: bool,
    /// Minecraft UUIDs (undashed) allowed to join.
    pub allowed_uuids: Vec<String>,
}

/// The document published to Redis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerListing {
    pub schema_version: u8,
    /// Deterministic id derived from the host peer id + world, so a restart
    /// replaces the old entry instead of leaking a second one.
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Raw MOTD line as shown in the Minecraft multiplayer list.
    pub motd: String,
    /// PNG/JPEG icon, already base64 encoded and size-capped (~64 KiB).
    pub icon_base64: Option<String>,

    pub owner: ServerOwner,

    pub game_version: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    pub modpack: Option<ModpackRef>,
    /// Mods the guest must have for a clean join (Modrinth project ids).
    pub required_mod_ids: Vec<String>,

    pub players: PlayerCount,
    pub connection: ConnectionDescriptor,
    pub region: Option<String>,
    pub tags: Vec<String>,
    pub whitelist: WhitelistPolicy,
    pub password_protected: bool,
    /// World name shown in the detail pane.
    pub world_name: Option<String>,

    pub created_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
    pub ttl_secs: u32,
    /// Aggregated play time of the world in seconds (nice-to-have stat).
    pub world_playtime_secs: u64,
}

impl ServerListing {
    /// Assign a fresh WAN-facing endpoint list and heartbeat timestamp.
    pub fn refresh_heartbeat(&mut self, endpoints: Vec<PeerEndpoint>) {
        self.heartbeat_at = Utc::now();
        if !endpoints.is_empty() {
            self.connection.endpoints = endpoints;
        }
    }

    /// `true` when the listing should no longer be trusted.
    pub fn is_stale(&self) -> bool {
        let age = Utc::now() - self.heartbeat_at;
        age.num_seconds() > i64::from(self.ttl_secs) * 2
    }

    /// Latency-free compatibility verdict used by the browse cards.
    pub fn is_compatible_with(&self, game_version: &str, loader: LoaderKind) -> bool {
        self.game_version == game_version && self.loader == loader
    }
}

/// Lightweight card payload sent to the browser grid (`browse` command).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerListingSummary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub icon_base64: Option<String>,
    pub owner_name: String,
    pub game_version: String,
    pub loader: LoaderKind,
    pub modpack_name: Option<String>,
    pub players: PlayerCount,
    pub mode: ConnectionMode,
    pub region: Option<String>,
    pub tags: Vec<String>,
    pub password_protected: bool,
    pub heartbeat_at: DateTime<Utc>,
    /// Milliseconds since the last heartbeat (freshness badge).
    pub age_ms: i64,
    /// Measured round-trip time in ms, filled in by the ping probe.
    pub ping_ms: Option<u32>,
    /// Filled in client-side by comparing against the active instance.
    pub version_mismatch: bool,
}

impl From<&ServerListing> for ServerListingSummary {
    fn from(listing: &ServerListing) -> Self {
        Self {
            id: listing.id,
            name: listing.name.clone(),
            description: listing.description.clone(),
            icon_base64: listing.icon_base64.clone(),
            owner_name: listing.owner.name.clone(),
            game_version: listing.game_version.clone(),
            loader: listing.loader,
            modpack_name: listing.modpack.as_ref().map(|pack| pack.name.clone()),
            players: listing.players.clone(),
            mode: listing.connection.mode,
            region: listing.region.clone(),
            tags: listing.tags.clone(),
            password_protected: listing.password_protected,
            heartbeat_at: listing.heartbeat_at,
            age_ms: (Utc::now() - listing.heartbeat_at).num_milliseconds(),
            ping_ms: None,
            version_mismatch: false,
        }
    }
}

/// Published on every heartbeat; lets watchers update counts without refetching
/// the whole listing document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerHeartbeat {
    pub server_id: Uuid,
    pub players: PlayerCount,
    pub ttl_secs: u32,
    pub heartbeat_at: DateTime<Utc>,
}

/// Query options for the browser.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerFilter {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub loader: Option<LoaderKind>,
    #[serde(default)]
    pub modpack_project_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub hide_full: bool,
    #[serde(default)]
    pub hide_password_protected: bool,
    /// Only servers with `ping_ms <= threshold`.
    #[serde(default)]
    pub max_ping_ms: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl ServerFilter {
    pub fn matches(&self, listing: &ServerListing) -> bool {
        if let Some(query) = self.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let needle = query.to_lowercase();
            let haystacks = [
                listing.name.to_lowercase(),
                listing.description.to_lowercase(),
                listing.owner.name.to_lowercase(),
            ];
            if !haystacks.iter().any(|value| value.contains(&needle)) {
                return false;
            }
        }
        if let Some(version) = &self.game_version {
            if &listing.game_version != version {
                return false;
            }
        }
        if let Some(loader) = self.loader {
            if listing.loader != loader {
                return false;
            }
        }
        if let Some(project_id) = &self.modpack_project_id {
            let matches_pack = listing
                .modpack
                .as_ref()
                .is_some_and(|pack| &pack.project_id == project_id);
            if !matches_pack {
                return false;
            }
        }
        if !self.tags.is_empty()
            && !self
                .tags
                .iter()
                .all(|tag| listing.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
        {
            return false;
        }
        if self.hide_full && listing.players.is_full() {
            return false;
        }
        if self.hide_password_protected && listing.password_protected {
            return false;
        }
        true
    }
}

/// Signaling envelope exchanged over `sxml:signal:{peer_id}`.
///
/// `signature` is the Ed25519 signature over the canonical JSON body, enabling
/// peer authentication without a central authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SignalingEnvelope {
    /// Guest -> Host: "I want to connect, here are my candidates".
    Offer {
        from_peer_id: String,
        to_peer_id: String,
        session_id: Uuid,
        candidates: Vec<PeerEndpoint>,
        public_key: String,
        signature: String,
        sent_at: DateTime<Utc>,
    },
    /// Host -> Guest: "Accepted, punch towards this address".
    Answer {
        from_peer_id: String,
        to_peer_id: String,
        session_id: Uuid,
        candidates: Vec<PeerEndpoint>,
        public_key: String,
        signature: String,
        sent_at: DateTime<Utc>,
    },
    /// Both sides probe each other simultaneously to open their NAT mappings.
    HolePunchProbe {
        from_peer_id: String,
        session_id: Uuid,
        token: [u8; 16],
        sent_at: DateTime<Utc>,
    },
    /// Host -> Guest: direct connection failed, switch to the relay.
    RelayFallback {
        from_peer_id: String,
        session_id: Uuid,
        relay: RelayDescriptor,
        sent_at: DateTime<Utc>,
    },
    /// Refusals with a reason the UI can display.
    Reject {
        from_peer_id: String,
        session_id: Uuid,
        reason: JoinRejection,
        sent_at: DateTime<Utc>,
    },
    /// Periodic "still punching" keep-alive while NAT traversal is in flight.
    KeepAlive {
        session_id: Uuid,
        sent_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinRejection {
    ServerFull,
    NotWhitelisted,
    WrongPassword,
    VersionMismatch,
    MissingMods,
    HostClosed,
    Banned,
}

impl JoinRejection {
    pub fn user_message(self) -> &'static str {
        match self {
            JoinRejection::ServerFull => "This server is full.",
            JoinRejection::NotWhitelisted => "You are not on this server's whitelist.",
            JoinRejection::WrongPassword => "Incorrect password.",
            JoinRejection::VersionMismatch => "Your game version does not match the host.",
            JoinRejection::MissingMods => "You are missing mods required by the host.",
            JoinRejection::HostClosed => "The host closed the world.",
            JoinRejection::Banned => "You were removed from this session.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::account::AccountProvider;

    fn listing(online: u32, max: u32, version: &str) -> ServerListing {
        ServerListing {
            schema_version: SERVER_LISTING_SCHEMA,
            id: Uuid::new_v4(),
            name: "Survival".into(),
            description: "Vanilla+ world".into(),
            motd: "Welcome".into(),
            icon_base64: None,
            owner: ServerOwner {
                name: "Steve".into(),
                uuid: Uuid::new_v4(),
                provider: AccountProvider::Offline,
            },
            game_version: version.into(),
            loader: LoaderKind::Fabric,
            loader_version: Some("0.15.11".into()),
            modpack: None,
            required_mod_ids: vec![],
            players: PlayerCount { online, max },
            connection: ConnectionDescriptor {
                mode: ConnectionMode::DirectP2p,
                peer_id: "peer".into(),
                public_key: "key".into(),
                endpoints: vec![],
                relay: None,
                session_token: "token".into(),
                protocol_version: 763,
            },
            region: None,
            tags: vec!["survival".into()],
            whitelist: WhitelistPolicy::default(),
            password_protected: false,
            world_name: None,
            created_at: Utc::now(),
            heartbeat_at: Utc::now(),
            ttl_secs: DEFAULT_HEARTBEAT_TTL_SECS,
            world_playtime_secs: 0,
        }
    }

    #[test]
    fn filter_hides_full_servers() {
        let full = listing(20, 20, "1.20.1");
        let filter = ServerFilter {
            hide_full: true,
            ..ServerFilter::default()
        };
        assert!(!filter.matches(&full));
    }

    #[test]
    fn filter_matches_version_and_tag() {
        let server = listing(1, 10, "1.20.1");
        let filter = ServerFilter {
            game_version: Some("1.20.1".into()),
            tags: vec!["SURVIVAL".into()],
            ..ServerFilter::default()
        };
        assert!(filter.matches(&server));
    }

    #[test]
    fn summary_reports_version_mismatch_flag_when_asked() {
        let server = listing(3, 8, "1.21.1");
        let summary = ServerListingSummary::from(&server);
        assert_eq!(summary.players.online, 3);
        assert_eq!(summary.game_version, "1.21.1");
    }
}
