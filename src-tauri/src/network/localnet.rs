//! LAN world discovery.
//!
//! The global browser needs a directory service (Redis) that somebody has to
//! run, which is exactly why "the launcher is always offline" was a fair
//! complaint. This module is the zero-infrastructure answer: a launcher
//! announces a world it is serving with a UDP broadcast, and every other
//! launcher on the same network sees it within a second. Nothing to install,
//! nothing to configure, no port forwarding.
//!
//! ```text
//!   host:  Open to LAN in game ─▶ log watcher sees "Started serving on 51234"
//!          └─▶ LanManager::publish ─▶ UDP broadcast every 2s
//!   guest: lan_browse ─▶ query broadcast ─▶ hosts reply ─▶ listed in the UI
//!          └─▶ launch with --server <host ip> --port <world port>
//! ```
//!
//! The beacon is deliberately tiny: it carries only what a server list shows,
//! and the game's own handshake remains the real gate.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket as StdUdpSocket};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::cancel::CancelFlag;
use crate::error::{AppError, AppResult};

/// Magic string every beacon starts with (cheap filter for stray UDP traffic).
pub const MAGIC: &str = "SXML-LAN1";
/// How long a world stays listed after its last beacon.
pub const WORLD_TTL: Duration = Duration::from_secs(9);
/// Beacon interval while hosting.
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(2);
/// Hard cap on a single datagram we are willing to parse.
const MAX_DATAGRAM: usize = 8 * 1024;

/// One broadcast payload: either an announcement or a "who is out there" probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanBeacon {
    pub magic: String,
    /// `true` for a browse request, `false` for an announcement.
    #[serde(default)]
    pub query: bool,
    pub id: Uuid,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub motd: String,
    #[serde(default)]
    pub host: String,
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
    /// Port the world listens on (the game's "Open to LAN" port).
    pub port: u16,
    #[serde(default)]
    pub password_protected: bool,
    #[serde(default)]
    pub protocol_version: i32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub ttl_secs: u64,
}

impl LanBeacon {
    /// A probe sent by `lan_browse`.
    pub fn probe() -> Self {
        Self {
            magic: MAGIC.to_string(),
            query: true,
            id: Uuid::nil(),
            name: String::new(),
            motd: String::new(),
            host: String::new(),
            instance_id: None,
            world_name: None,
            game_version: String::new(),
            loader: String::new(),
            players: 0,
            max_players: 0,
            port: 0,
            password_protected: false,
            protocol_version: 0,
            tags: Vec::new(),
            ttl_secs: 0,
        }
    }

    /// Reject anything that is not ours or is obviously malformed.
    pub fn validate(&self) -> AppResult<()> {
        if self.magic != MAGIC {
            return Err(AppError::Transport("not a SXMLAUNCHER LAN beacon".into()));
        }
        if !self.query && self.port == 0 {
            return Err(AppError::Transport("LAN beacon without a port".into()));
        }
        Ok(())
    }
}

/// A world discovered on the local network, as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanWorld {
    pub id: Uuid,
    pub name: String,
    pub motd: String,
    pub host: String,
    /// Address the game connects to (`--server`).
    pub address: String,
    /// Port the game connects to (`--port`).
    pub port: u16,
    pub game_version: String,
    pub loader: String,
    pub players: u32,
    pub max_players: u32,
    pub password_protected: bool,
    pub protocol_version: i32,
    pub tags: Vec<String>,
    pub instance_id: Option<Uuid>,
    pub world_name: Option<String>,
    /// Seconds since the last beacon — the UI greys out stale entries.
    pub last_seen_secs: f64,
}

/// A world this launcher is announcing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanHost {
    pub id: Uuid,
    pub name: String,
    pub port: u16,
    /// How friends on the same network connect.
    pub address: String,
    pub instance_id: Option<Uuid>,
    pub world_name: Option<String>,
    pub game_version: String,
    pub players: u32,
    pub max_players: u32,
    /// `true` when the port came from the game log rather than a manual entry.
    pub auto_detected: bool,
}

/// LAN broadcast socket, shared by the announcer and the browser.
pub struct LanDiscovery {
    socket: Arc<UdpSocket>,
    port: u16,
    broadcast: Vec<SocketAddr>,
}

impl LanDiscovery {
    /// Bind the beacon port. `port == 0` asks the OS for a free one (tests).
    pub async fn bind(port: u16) -> AppResult<Self> {
        let socket = tokio::task::spawn_blocking(move || -> AppResult<StdUdpSocket> {
            let address = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
            let socket = std::net::UdpSocket::bind(address).map_err(|err| {
                AppError::Transport(format!("cannot bind the LAN beacon port {port}: {err}"))
            })?;
            socket.set_broadcast(true).map_err(|err| {
                AppError::Transport(format!("cannot enable broadcast on the LAN socket: {err}"))
            })?;
            socket.set_nonblocking(true)?;
            Ok(socket)
        })
        .await??;

        let actual_port = socket
            .local_addr()
            .map_err(|err| AppError::Transport(format!("LAN socket has no address: {err}")))?
            .port();
        let socket = UdpSocket::from_std(socket)
            .map_err(|err| AppError::Transport(format!("LAN socket setup failed: {err}")))?;

        Ok(Self {
            socket: Arc::new(socket),
            port: actual_port,
            broadcast: broadcast_targets(actual_port),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Send one payload to every broadcast address we care about.
    pub async fn send(&self, beacon: &LanBeacon) -> AppResult<()> {
        let payload = serde_json::to_vec(beacon)?;
        if payload.len() > MAX_DATAGRAM {
            return Err(AppError::Transport(
                "LAN beacon is too large to broadcast".into(),
            ));
        }
        for target in &self.broadcast {
            // One unreachable interface must not abort the others.
            let _ = self.socket.send_to(&payload, target).await;
        }
        Ok(())
    }

    /// Ask every launcher on the network to announce itself again.
    pub async fn probe(&self) -> AppResult<()> {
        self.send(&LanBeacon::probe()).await
    }

    /// Receive announcements until `idle_timeout` passes without traffic.
    pub async fn collect(&self, idle_timeout: Duration) -> Vec<LanBeacon> {
        let mut found = Vec::new();
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        let deadline = Instant::now() + idle_timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, self.socket.recv_from(&mut buffer)).await {
                Ok(Ok((read, _from))) => {
                    if let Ok(beacon) = serde_json::from_slice::<LanBeacon>(&buffer[..read]) {
                        if beacon.validate().is_ok() && !beacon.query {
                            found.push(beacon);
                        }
                    }
                }
                // A socket error on a broadcast read is not fatal.
                Ok(Err(_)) => continue,
                Err(_) => break,
            }
        }
        found
    }

    /// Probe + collect: the whole "what is on my network?" operation.
    pub async fn browse(&self, idle_timeout: Duration) -> AppResult<Vec<LanBeacon>> {
        self.probe().await?;
        Ok(self.collect(idle_timeout).await)
    }
}

/// Where a broadcast should be sent.
///
/// The limited broadcast address works everywhere; the per-interface /24 address
/// is added because some Windows adapters drop 255.255.255.255.
fn broadcast_targets(port: u16) -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, port))];
    if let Some(ip) = primary_local_ipv4() {
        let octets = ip.octets();
        targets.push(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(octets[0], octets[1], octets[2], 255),
            port,
        )));
    }
    targets
}

/// Best-effort "which LAN address am I?" lookup.
///
/// Connecting a UDP socket to a public address never sends a packet, but it makes
/// the OS pick the interface it would use — exactly the address friends on the
/// same network need.
pub fn primary_local_ipv4() -> Option<Ipv4Addr> {
    let socket = StdUdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

/// Announces local worlds and keeps a live map of the ones it hears about.
///
/// One instance lives in [`crate::state::AppState`]; the maps are shared with the
/// receive loop through `Arc<DashMap>` so the listener and the commands see the
/// same data without any locking.
pub struct LanManager {
    port: AtomicU16,
    discovery: tokio::sync::Mutex<Option<Arc<LanDiscovery>>>,
    /// Beacons we are broadcasting, keyed by host id.
    hosts: Arc<DashMap<Uuid, LanBeacon>>,
    /// Cancellation flag per announcement (stops the announce loop).
    announcers: DashMap<Uuid, CancelFlag>,
    /// Worlds heard from other machines, with the time of the last beacon.
    worlds: Arc<DashMap<Uuid, (LanWorld, Instant)>>,
    listener: tokio::sync::Mutex<Option<CancelFlag>>,
}

impl Default for LanManager {
    fn default() -> Self {
        Self::new(crate::config::DEFAULT_LAN_PORT)
    }
}

impl LanManager {
    pub fn new(port: u16) -> Self {
        Self {
            port: AtomicU16::new(port),
            discovery: tokio::sync::Mutex::new(None),
            hosts: Arc::new(DashMap::new()),
            announcers: DashMap::new(),
            worlds: Arc::new(DashMap::new()),
            listener: tokio::sync::Mutex::new(None),
        }
    }

    /// UDP port the beacon socket is (or will be) bound to.
    ///
    /// Atomic rather than `u16` because the OS resolves `port 0` to a real port
    /// during binding, and `&self` mutators like `rebind` need to update it.
    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    /// Bind (once) and make sure the receive loop is running.
    pub async fn discovery(&self) -> AppResult<Arc<LanDiscovery>> {
        {
            let slot = self.discovery.lock().await;
            if let Some(existing) = slot.as_ref() {
                return Ok(existing.clone());
            }
        }

        let discovery = Arc::new(LanDiscovery::bind(self.port()).await?);
        self.port.store(discovery.port(), Ordering::Relaxed);
        {
            let mut slot = self.discovery.lock().await;
            *slot = Some(discovery.clone());
        }

        let mut listener = self.listener.lock().await;
        let needs_listener = listener.as_ref().is_none_or(|flag| flag.is_cancelled());
        if needs_listener {
            let flag = CancelFlag::new();
            *listener = Some(flag.clone());
            start_listener(
                discovery.clone(),
                self.hosts.clone(),
                self.worlds.clone(),
                flag,
            );
        }
        Ok(discovery)
    }

    /// Start announcing a world; returns what friends on the LAN will see.
    ///
    /// Calling it twice for the same id replaces the previous announcement, which
    /// is what the log watcher relies on when a world's player count changes.
    pub async fn publish(&self, mut beacon: LanBeacon, auto_detected: bool) -> AppResult<LanHost> {
        let discovery = self.discovery().await?;
        if beacon.id == Uuid::nil() {
            beacon.id = Uuid::new_v4();
        }
        beacon.query = false;
        beacon.validate()?;

        // Replace any previous announcer for this host id.
        if let Some((_, flag)) = self.announcers.remove(&beacon.id) {
            flag.cancel();
        }
        self.hosts.insert(beacon.id, beacon.clone());

        let flag = CancelFlag::new();
        self.announcers.insert(beacon.id, flag.clone());

        // Announce immediately (so a browse right after hosting finds it) and
        // then keep refreshing the listing.
        let _ = discovery.send(&beacon).await;
        let announce = beacon.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(ANNOUNCE_INTERVAL) => {
                        // One failed broadcast is normal on flaky Wi-Fi.
                        let _ = discovery.send(&announce).await;
                    }
                    _ = flag.cancelled() => return,
                }
            }
        });

        Ok(self.host_status(beacon.id, auto_detected).unwrap_or(LanHost {
            id: beacon.id,
            name: beacon.name.clone(),
            port: beacon.port,
            address: local_address(),
            instance_id: beacon.instance_id,
            world_name: beacon.world_name.clone(),
            game_version: beacon.game_version.clone(),
            players: beacon.players,
            max_players: beacon.max_players,
            auto_detected,
        }))
    }

    /// Update a published world in place (player counts change over time).
    pub async fn update(&self, id: Uuid, players: u32) -> Option<LanHost> {
        {
            let mut beacon = self.hosts.get_mut(&id)?;
            beacon.players = players;
        }
        // Push the change immediately instead of waiting for the next tick.
        if let Ok(discovery) = self.discovery().await {
            if let Some(beacon) = self.hosts.get(&id) {
                let _ = discovery.send(beacon.value()).await;
            }
        }
        self.host_status(id, false)
    }

    /// Stop announcing a world.
    pub fn unpublish(&self, id: Uuid) -> bool {
        let removed = self.hosts.remove(&id).is_some();
        if let Some((_, flag)) = self.announcers.remove(&id) {
            flag.cancel();
        }
        removed
    }

    /// Worlds this machine is currently announcing.
    pub fn hosts(&self) -> Vec<LanHost> {
        let mut list: Vec<LanHost> = self
            .hosts
            .iter()
            .map(|entry| self.to_status(entry.key(), entry.value()))
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Status of one announced world.
    pub fn host_status(&self, id: Uuid, auto_detected: bool) -> Option<LanHost> {
        let entry = self.hosts.get(&id)?;
        let mut status = self.to_status(&id, entry.value());
        status.auto_detected = auto_detected;
        Some(status)
    }

    /// Announcement for an instance, if one is being hosted.
    pub fn host_for_instance(&self, instance_id: Uuid) -> Option<LanHost> {
        self.hosts
            .iter()
            .find(|entry| entry.value().instance_id == Some(instance_id))
            .map(|entry| self.to_status(entry.key(), entry.value()))
    }

    fn to_status(&self, id: &Uuid, beacon: &LanBeacon) -> LanHost {
        LanHost {
            id: *id,
            name: beacon.name.clone(),
            port: beacon.port,
            address: local_address(),
            instance_id: beacon.instance_id,
            world_name: beacon.world_name.clone(),
            game_version: beacon.game_version.clone(),
            players: beacon.players,
            max_players: beacon.max_players,
            auto_detected: false,
        }
    }

    /// Worlds heard from other machines, freshest first.
    pub fn worlds(&self) -> Vec<LanWorld> {
        let now = Instant::now();
        let mut list: Vec<LanWorld> = self
            .worlds
            .iter()
            .map(|entry| {
                let (mut world, seen) = entry.value().clone();
                world.last_seen_secs = now.duration_since(seen).as_secs_f64();
                world
            })
            .collect();
        list.sort_by(|a, b| a.last_seen_secs.total_cmp(&b.last_seen_secs));
        list
    }

    /// Ask the network and merge the answers into the live map.
    pub async fn browse(&self, idle_timeout: Duration) -> AppResult<Vec<LanWorld>> {
        let discovery = self.discovery().await?;
        for beacon in discovery.browse(idle_timeout).await? {
            // Our own worlds are reported through `hosts()`, never as remote.
            if self.hosts.contains_key(&beacon.id) {
                continue;
            }
            let world = LanWorld {
                id: beacon.id,
                name: beacon.name,
                motd: beacon.motd,
                host: beacon.host,
                address: local_address(),
                port: beacon.port,
                game_version: beacon.game_version,
                loader: beacon.loader,
                players: beacon.players,
                max_players: beacon.max_players,
                password_protected: beacon.password_protected,
                protocol_version: beacon.protocol_version,
                tags: beacon.tags,
                instance_id: beacon.instance_id,
                world_name: beacon.world_name,
                last_seen_secs: 0.0,
            };
            self.worlds.insert(beacon.id, (world, Instant::now()));
        }
        Ok(self.worlds())
    }

    /// Stop every announcement (app shutdown).
    pub fn shutdown(&self) {
        for entry in self.announcers.iter() {
            entry.value().cancel();
        }
        self.announcers.clear();
        self.hosts.clear();
    }

    /// Move the beacon socket to a new port.
    ///
    /// Called when the user edits the LAN port in Settings: announcements are
    /// dropped, the old socket is closed and the next operation binds again.
    pub async fn rebind(&self, port: u16) {
        if port == self.port() {
            return;
        }
        self.shutdown();
        if let Some(flag) = self.listener.lock().await.take() {
            flag.cancel();
        }
        *self.discovery.lock().await = None;
        self.port.store(port, Ordering::Relaxed);
    }
}

/// This machine's LAN address, as friends should type it.
pub fn local_address() -> String {
    primary_local_ipv4()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Receive loop: answers probes and records announcements.
fn start_listener(
    discovery: Arc<LanDiscovery>,
    hosts: Arc<DashMap<Uuid, LanBeacon>>,
    worlds: Arc<DashMap<Uuid, (LanWorld, Instant)>>,
    flag: CancelFlag,
) {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        loop {
            if flag.is_cancelled() {
                return;
            }
            let received = tokio::time::timeout(
                Duration::from_millis(400),
                discovery.socket.recv_from(&mut buffer),
            )
            .await;

            if let Ok(Ok((read, from))) = received {
                if let Ok(beacon) = serde_json::from_slice::<LanBeacon>(&buffer[..read]) {
                    if beacon.validate().is_ok() {
                        if beacon.query {
                            // Answer probes with every world we announce, so a
                            // browse finds us without waiting for the next tick.
                            for entry in hosts.iter() {
                                let _ = discovery.send(entry.value()).await;
                            }
                        } else if !hosts.contains_key(&beacon.id) {
                            let address = match from.ip() {
                                IpAddr::V4(v4) => v4.to_string(),
                                IpAddr::V6(v6) => v6.to_string(),
                            };
                            let world = LanWorld {
                                id: beacon.id,
                                name: beacon.name.clone(),
                                motd: beacon.motd.clone(),
                                host: beacon.host.clone(),
                                address,
                                port: beacon.port,
                                game_version: beacon.game_version.clone(),
                                loader: beacon.loader.clone(),
                                players: beacon.players,
                                max_players: beacon.max_players,
                                password_protected: beacon.password_protected,
                                protocol_version: beacon.protocol_version,
                                tags: beacon.tags.clone(),
                                instance_id: beacon.instance_id,
                                world_name: beacon.world_name.clone(),
                                last_seen_secs: 0.0,
                            };
                            worlds.insert(beacon.id, (world, Instant::now()));
                        }
                    }
                }
            }

            // Expire worlds whose host stopped announcing.
            let now = Instant::now();
            worlds.retain(|_, (_, seen)| now.duration_since(*seen) <= WORLD_TTL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_recognised_and_announcements_need_a_port() {
        assert!(LanBeacon::probe().validate().is_ok());

        let mut announcement = LanBeacon::probe();
        announcement.query = false;
        assert!(announcement.validate().is_err());
        announcement.port = 25565;
        assert!(announcement.validate().is_ok());
    }

    #[test]
    fn foreign_packets_are_rejected() {
        let mut beacon = LanBeacon::probe();
        beacon.magic = "SOME-OTHER-LAUNCHER".into();
        assert!(beacon.validate().is_err());
    }

    #[test]
    fn beacons_round_trip_through_json() {
        let mut beacon = LanBeacon::probe();
        beacon.query = false;
        beacon.id = Uuid::new_v4();
        beacon.name = "Survival".into();
        beacon.port = 51234;
        beacon.players = 2;
        beacon.max_players = 8;
        beacon.game_version = "1.20.1".into();

        let encoded = serde_json::to_vec(&beacon).expect("encode");
        let decoded: LanBeacon = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded.magic, MAGIC);
        assert_eq!(decoded.port, 51234);
        assert_eq!(decoded.name, "Survival");
        assert!(!decoded.query);
    }

    #[tokio::test]
    async fn publishing_registers_updates_and_stops_a_world() {
        let manager = LanManager::new(0);
        let instance_id = Uuid::new_v4();
        let mut beacon = LanBeacon::probe();
        beacon.query = false;
        beacon.name = "Friends Only".into();
        beacon.port = 25570;
        beacon.instance_id = Some(instance_id);

        let host = manager.publish(beacon.clone(), true).await.expect("publish");
        assert_eq!(host.port, 25570);
        assert!(host.auto_detected);
        assert_eq!(manager.hosts().len(), 1);
        assert!(manager.host_for_instance(instance_id).is_some());

        assert!(manager.update(host.id, 3).await.is_some());
        assert_eq!(manager.hosts()[0].players, 3);

        assert!(manager.unpublish(host.id));
        assert!(manager.hosts().is_empty());
        assert!(!manager.unpublish(host.id));
    }

    #[tokio::test]
    async fn browsing_never_reports_our_own_worlds() {
        let manager = LanManager::new(0);
        let mut beacon = LanBeacon::probe();
        beacon.query = false;
        beacon.name = "Mine".into();
        beacon.port = 25571;
        let host = manager.publish(beacon, false).await.expect("publish");

        let worlds = manager
            .browse(Duration::from_millis(120))
            .await
            .expect("browse");
        assert!(worlds.iter().all(|world| world.id != host.id));
    }

    #[test]
    fn world_ttl_is_short_enough_to_not_show_ghosts() {
        assert!(WORLD_TTL.as_secs() <= 15);
    }

    #[test]
    fn local_address_is_always_usable() {
        assert!(!local_address().is_empty());
    }
}


