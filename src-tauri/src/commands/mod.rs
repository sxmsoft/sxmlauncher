//! Tauri command surface.
//!
//! Rules every command follows:
//! * Thin: argument validation + delegation to a manager, nothing else.
//! * No `unwrap()`: all failures become [`crate::error::AppError`], which
//!   serializes to `{ code, message, retryable }` for the frontend.
//! * Long work reports progress through the `job://progress` event and returns
//!   the final result; the UI never polls.

pub mod account;
pub mod custom_packs;
pub mod instance;
pub mod jobs;
pub mod mods;
pub mod network;
pub mod system;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::instance::Instance;
use crate::models::server::{ConnectionMode, PlayerCount, ServerListingSummary};

/// Payload emitted on `game://state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStateEvent {
    pub instance_id: Uuid,
    pub state: GameState,
    pub pid: Option<u32>,
    /// Local bridge address when a P2P session is attached.
    pub connect_address: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameState {
    Starting,
    Running,
    Exited,
    Crashed,
}

/// Payload emitted on `session://event`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventPayload {
    pub id: Uuid,
    pub kind: String,
    pub peer_id: Option<String>,
    pub username: Option<String>,
    pub players: Option<PlayerCount>,
    pub message: Option<String>,
}

/// A running host session as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub id: Uuid,
    pub share_code: String,
    pub summary: ServerListingSummary,
    /// `direct_p2p` when a public UDP mapping was found, else `relay`.
    pub mode: ConnectionMode,
    /// Plain-English NAT verdict for the hosting panel.
    pub nat_advice: String,
    pub public_endpoint: Option<String>,
    pub guests: Vec<GuestStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestStatus {
    pub peer_id: String,
    pub address: String,
    pub username: Option<String>,
    pub protocol_version: Option<i32>,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

/// A joined session as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinStatus {
    pub id: Uuid,
    pub server_name: String,
    pub mode: ConnectionMode,
    /// Loopback bridge. 1.20+ launches with `--quickPlayMultiplayer`; older
    /// versions still use `--server` / `--port`.
    pub local_address: String,
    pub local_port: u16,
    pub remote: Option<String>,
    pub rtt_ms: Option<u32>,
    /// `true` when a ready instance was started into the bridge.
    pub launched: bool,
    pub launch_error: Option<String>,
}

/// Result of a launch request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchReport {
    pub instance: Instance,
    pub pid: u32,
    pub connect_address: Option<String>,
    pub session_id: Option<Uuid>,
    /// Redacted command line, safe to show in a debug pane.
    pub command_preview: String,
}

/// Aggregated directory connection state for the status pill.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    pub directory_connected: bool,
    pub directory_url: String,
    pub online_players: u64,
    pub active_hosts: usize,
    pub active_guests: usize,
    pub relay_configured: bool,
    pub message: Option<String>,
    /// LAN discovery is always available and needs no server of any kind.
    #[serde(default)]
    pub lan_enabled: bool,
    /// UDP port used for LAN beacons.
    #[serde(default)]
    pub lan_port: u16,
    /// Worlds announced on the local network right now.
    #[serde(default)]
    pub lan_worlds: usize,
}

/// Validate a UUID-ish string coming from the UI.
pub fn parse_uuid(raw: &str) -> Result<Uuid, crate::error::AppError> {
    Uuid::parse_str(raw.trim())
        .map_err(|err| crate::error::AppError::Config(format!("invalid id `{raw}`: {err}")))
}
