//! Embedded MQTT-backed directory — the client-side global server browser.
//!
//! The old design needed a Redis server we do not operate, so every launcher
//! sat in "offline" mode forever. This implementation needs **no server of our
//! own**: clients meet on a public MQTT broker and the broker's *retained
//! messages* carry all shared state:
//!
//! ```text
//! sxml2/listing/{server_id}   retained ServerListing JSON (republished per heartbeat)
//! sxml2/code/{CODE}          retained "{server_id}" (share codes)
//! sxml2/signal/{peer_id}     non-retained SignalingEnvelope JSON (hole punch)
//! ```
//!
//! Design notes:
//!
//! * **Retained = the database.** A fresh subscriber receives every live
//!   listing the moment it subscribes, so "browse" is: subscribe, collect the
//!   retained snapshot, filter. Expiry is client-side — a listing whose
//!   heartbeat is older than twice its TTL is dead (the Redis implementation
//!   achieved the same with `ZREMRANGEBYSCORE` index pruning).
//! * **One connection, cheap handles.** rumqttc's event loop owns the socket;
//!   `AsyncClient` handles are clones into a bounded command queue, so
//!   publishes never block on the network. The loop reconnects on its own.
//! * **Signals** fan out from `signal/{peer_id}` topics to in-process mpsc
//!   receivers, the same contract the Redis pub/sub implementation exposes.
//! * **Online counter** is derived from the cached listings (sum of reported
//!   players) instead of a racy shared counter — it is a badge, not a ledger.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::{RwLock};
use rumqttc::tokio_rustls::rustls::{ClientConfig, RootCertStore};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use tokio::sync::{broadcast, mpsc};

use crate::error::{AppError, AppResult};
use crate::models::server::{
    ServerFilter, ServerHeartbeat, ServerListing, ServerListingSummary, SignalingEnvelope,
    DEFAULT_HEARTBEAT_TTL_SECS,
};

/// Topic namespace. Bumped from the Redis-era `sxml` so stale broker state is
/// ignored cleanly.
pub const MQTT_KEY_PREFIX: &str = "sxml2";

/// Default public broker (TLS port). Anonymous; no account or setup needed.
pub const DEFAULT_MQTT_BROKER: &str = "broker.emqx.io";
pub const DEFAULT_MQTT_PORT: u16 = 8883;

/// A listing is dead when its heartbeat is older than 2× its TTL.
const STALE_MULTIPLIER: i64 = 2;

/// Retained deletion marker. MQTT's native "delete a retained message" is an
/// empty payload — which existing subscribers never receive — so removals are
/// announced with this one-byte tombstone instead (and the empty payload is
/// ALSO published to clean the retained copy for future subscribers).
const TOMBSTONE: &[u8] = b"x";

/// How long `connect` waits for CONNECT+SUBACK before reporting failure.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long lookups wait for the retained flood after a cold start.
const COLD_START_WAIT: Duration = Duration::from_secs(2);
const COLD_START_TICK: Duration = Duration::from_millis(100);

/// A live message from the broker.
#[derive(Debug, Clone)]
pub struct BrokerMessage {
    /// Topic *suffix* under the namespace, e.g. `signal/{peer_id}`.
    pub suffix: String,
    pub payload: Vec<u8>,
    pub retained: bool,
}

/// Shared view of everything the broker has told us.
#[derive(Default)]
struct MqttCache {
    listings: HashMap<uuid::Uuid, ServerListing>,
    codes: HashMap<String, uuid::Uuid>,
}

impl MqttCache {
    fn fresh_listings(&self) -> Vec<ServerListing> {
        let cutoff = stale_cutoff();
        self.listings
            .values()
            .filter(|listing| listing.heartbeat_at.timestamp_millis() >= cutoff)
            .cloned()
            .collect()
    }
}

fn stale_cutoff() -> i64 {
    Utc::now().timestamp_millis() - STALE_MULTIPLIER * i64::from(DEFAULT_HEARTBEAT_TTL_SECS) * 1000
}

/// The embedded directory handle given to [`super::SessionManager`].
#[derive(Clone)]
pub struct MqttDirectory {
    client: AsyncClient,
    #[allow(dead_code)]
    events: broadcast::Sender<BrokerMessage>,
    broker: String,
    signal_subscribers: Arc<RwLock<HashMap<String, Vec<mpsc::Sender<SignalingEnvelope>>>>>,
    cache: Arc<RwLock<MqttCache>>,
    /// Running value for `add_players` (the trait's counter contract); the
    /// displayed global number is derived from listings instead.
    local_players: Arc<AtomicI64>,
}

impl MqttDirectory {
    /// Connect, subscribe to the namespace and start the event loop.
    pub async fn connect(broker_host: &str, port: u16) -> AppResult<Self> {
        let session = uuid::Uuid::new_v4().simple().to_string();
        let mut opts = MqttOptions::new(format!("sxml-{session}"), broker_host, port);
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_clean_session(true);
        // Listings carry base64 icons; stay under the broker's usual 1 MiB
        // cap while allowing rich worlds. (outgoing, incoming)
        opts.set_max_packet_size(512 * 1024, 512 * 1024);

        if port == 8883 {
            let tls_config = Self::tls_config().ok_or_else(|| {
                AppError::Directory("no TLS root certificates found on this system".to_string())
            })?;
            opts.set_transport(Transport::tls_with_config(
                rumqttc::TlsConfiguration::Rustls(Arc::new(tls_config)),
            ));
        }

        let (client, mut eventloop) = rumqttc::AsyncClient::new(opts, 64);
        let (events_tx, _) = broadcast::channel::<BrokerMessage>(1024);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<AppResult<()>>();

        let cache: Arc<RwLock<MqttCache>> = Arc::new(RwLock::new(MqttCache::default()));
        let signal_subscribers: Arc<RwLock<HashMap<String, Vec<mpsc::Sender<SignalingEnvelope>>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let broker_label = format!("{broker_host}:{port}");
        // Clones for the spawned tasks (the originals move into `Self`).
        let loop_client = client.clone();
        let loop_events = events_tx.clone();
        let loop_broker = broker_label.clone();

        // The event loop: owns the socket forever, feeds the cache, reports
        // readiness once the subscription is confirmed.
        let loop_cache = cache.clone();
        let ready_task = tokio::spawn(async move {
            let mut ready: Option<tokio::sync::oneshot::Sender<AppResult<()>>> = Some(ready_tx);
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        if let Err(err) = loop_client
                            .subscribe(format!("{MQTT_KEY_PREFIX}/#"), QoS::AtLeastOnce)
                            .await
                        {
                            if let Some(tx) = ready.take() {
                                let _ = tx.send(Err(AppError::Directory(format!(
                                    "broker subscription failed: {err}"
                                ))));
                            }
                            break;
                        }
                        // SubAck arrives next; readiness is sent there so we
                        // know the broker accepted the wildcard subscription.
                    }
                    Ok(Event::Incoming(Packet::SubAck(_))) => {
                        if let Some(tx) = ready.take() {
                            let _ = tx.send(Ok(()));
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        let Some(suffix) = publish
                            .topic
                            .strip_prefix(&format!("{MQTT_KEY_PREFIX}/"))
                            .map(str::to_string)
                        else {
                            continue;
                        };

                        if suffix.starts_with("listing/") {
                            // Two payload shapes: a full listing JSON, or the
                            // tombstone `{"tombstone":true}` (an empty retained
                            // payload deletes the message broker-side but is
                            // NOT delivered to live subscribers — the tombstone
                            // is how removals propagate to clients that are
                            // currently connected).
                            match serde_json::from_slice::<ServerListing>(&publish.payload) {
                                Ok(listing) => {
                                    let mut guard = loop_cache.write();
                                    if is_fresh(&listing) {
                                        guard.listings.insert(listing.id, listing);
                                    } else {
                                        // A retained corpse: expired but the
                                        // host never said goodbye. Drop it.
                                        guard.listings.remove(&listing.id);
                                    }
                                }
                                Err(_) => {
                                    if publish.payload == TOMBSTONE {
                                        let id = suffix["listing/".len()..].parse().ok();
                                        if let Some(id) = id {
                                            loop_cache.write().listings.remove(&id);
                                        }
                                    } else {
                                        eprintln!(
                                            "[mqtt] bad listing payload on {suffix}"
                                        );
                                    }
                                }
                            }
                        } else if suffix.starts_with("code/") {
                            if let Ok(id) = serde_json::from_slice::<uuid::Uuid>(&publish.payload)
                            {
                                let code = suffix["code/".len()..].to_uppercase();
                                loop_cache.write().codes.insert(code, id);
                            }
                        } else if suffix.starts_with("signal/") {
                            // Signals reach subscribers through the fan-out
                            // task below (broadcast -> mpsc); nothing to do in
                            // the cache.
                        }

                        let _ = loop_events.send(BrokerMessage {
                            suffix,
                            payload: publish.payload.to_vec(),
                            retained: publish.retain,
                        });
                    }
                    Ok(_) => {}
                    Err(err) => {
                        // rumqttc retries the connection on the next poll;
                        // only the very first window can fail startup.
                        if let Some(tx) = ready.take() {
                            let _ = tx.send(Err(AppError::Directory(format!(
                                "MQTT broker {loop_broker} unreachable: {err}"
                            ))));
                            break;
                        }
                        eprintln!("[mqtt] {loop_broker}: {err}; reconnecting");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });

        // Signal fan-out task: route incoming non-retained signal messages to
        // the in-process receivers registered by `subscribe_signals`.
        let mut fanout_rx = events_tx.subscribe();
        let fanout_signals = signal_subscribers.clone();
        tokio::spawn(async move {
            loop {
                let Ok(message) = fanout_rx.recv().await else {
                    break;
                };
                if message.retained || !message.suffix.starts_with("signal/") {
                    continue;
                }
                let Ok(envelope) = serde_json::from_slice::<SignalingEnvelope>(&message.payload)
                else {
                    continue;
                };
                let peer_id = message.suffix["signal/".len()..].to_string();
                if let Some(senders) = fanout_signals.write().get_mut(&peer_id) {
                    senders.retain(|sender| !sender.is_closed());
                    for sender in senders.iter() {
                        let _ = sender.try_send(envelope.clone());
                    }
                }
            }
        });

        match tokio::time::timeout(CONNECT_TIMEOUT, ready_rx).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(err))) => return Err(err),
            Ok(Err(_)) | Err(_) => {
                return Err(AppError::Directory(format!(
                    "MQTT broker {broker_host}:{port} did not answer in time"
                )))
            }
        }

        drop(ready_task);

        Ok(Self {
            client,
            events: events_tx,
            broker: broker_label,
            signal_subscribers,
            cache,
            local_players: Arc::new(AtomicI64::new(0)),
        })
    }

    /// rustls config with the OS certificate store — rumqttc ships no roots.
    fn tls_config() -> Option<ClientConfig> {
        let certs = rustls_native_certs::load_native_certs()
            .map_err(|err| {
                eprintln!("[mqtt] loading OS certificates failed: {err}");
                err
            })
            .unwrap_or_default();
        let mut roots = RootCertStore::empty();
        // `add_parsable_certificates` skips individual bad encodings instead
        // of failing the whole store.
        let (added, _ignored) = roots.add_parsable_certificates(certs);
        if added == 0 {
            return None;
        }
        Some(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    pub fn broker_label(&self) -> &str {
        &self.broker
    }

    /// Subscribe to signalling for `peer_id` (same contract as the Redis
    /// pub/sub path).
    pub async fn subscribe_signals(
        &self,
        peer_id: &str,
    ) -> AppResult<mpsc::Receiver<SignalingEnvelope>> {
        let (tx, rx) = mpsc::channel(128);
        self.signal_subscribers
            .write()
            .entry(peer_id.to_string())
            .or_default()
            .push(tx);
        Ok(rx)
    }

    /// Wait (bounded) for the retained flood after a cold start.
    async fn wait_for_snapshot(&self) -> Vec<ServerListing> {
        let deadline = tokio::time::Instant::now() + COLD_START_WAIT;
        loop {
            let listings = self.cache.read().fresh_listings();
            if !listings.is_empty() || tokio::time::Instant::now() >= deadline {
                return listings;
            }
            tokio::time::sleep(COLD_START_TICK).await;
        }
    }

    async fn publish_retained(&self, suffix: String, payload: Vec<u8>) -> AppResult<()> {
        self.client
            .publish(format!("{MQTT_KEY_PREFIX}/{suffix}"), QoS::AtLeastOnce, true, payload)
            .await
            .map_err(|err| AppError::Directory(format!("broker publish failed: {err}")))
    }
}

fn is_fresh(listing: &ServerListing) -> bool {
    let ttl = if listing.ttl_secs > 0 {
        listing.ttl_secs
    } else {
        DEFAULT_HEARTBEAT_TTL_SECS
    };
    listing.heartbeat_at.timestamp_millis()
        >= Utc::now().timestamp_millis() - STALE_MULTIPLIER * i64::from(ttl) * 1000
}

#[async_trait]
impl super::Directory for MqttDirectory {
    async fn ping(&self) -> AppResult<()> {
        // The handle only exists after CONNECT+SUBACK succeeded; a later
        // disconnect re-heals inside the event loop.
        Ok(())
    }

    async fn online_players(&self) -> AppResult<u64> {
        let total: u64 = self
            .cache
            .read()
            .fresh_listings()
            .iter()
            .map(|listing| u64::from(listing.players.online.min(listing.players.max.max(1))))
            .sum();
        Ok(total)
    }

    async fn publish_listing(&self, listing: &ServerListing) -> AppResult<()> {
        let payload = serde_json::to_vec(listing)
            .map_err(|err| AppError::Directory(format!("listing encode failed: {err}")))?;
        // Retained publish: replaces the snapshot for this id on every broker
        // client and seeds every future subscriber. The heartbeat loop calls
        // this every few seconds — this IS the TTL mechanism.
        self.publish_retained(format!("listing/{}", listing.id), payload)
            .await
    }

    async fn touch_listing(&self, id: uuid::Uuid, _ttl_secs: u32) -> AppResult<bool> {
        Ok(self.cache.read().listings.contains_key(&id))
    }

    async fn remove_listing(&self, id: uuid::Uuid, _peer_id: &str) -> AppResult<()> {
        self.cache.write().listings.remove(&id);
        // 1. Tombstone: live subscribers drop the listing immediately.
        let _ = self
            .publish_retained(format!("listing/{id}"), TOMBSTONE.to_vec())
            .await;
        // 2. Empty payload: deletes the retained message broker-side so
        // future subscribers start clean.
        let _ = self.publish_retained(format!("listing/{id}"), Vec::new()).await;
        Ok(())
    }

    async fn browse(&self, filter: &ServerFilter) -> AppResult<Vec<ServerListingSummary>> {
        let listings = self.wait_for_snapshot().await;
        Ok(listings
            .iter()
            .filter(|listing| filter.matches(listing))
            .map(ServerListingSummary::from)
            .collect())
    }

    async fn listing(&self, id: uuid::Uuid) -> AppResult<Option<ServerListing>> {
        if let Some(listing) = self.cache.read().listings.get(&id).cloned() {
            return Ok(Some(listing));
        }
        // Cold start: the retained snapshot may not have landed yet.
        let deadline = tokio::time::Instant::now() + COLD_START_WAIT;
        loop {
            tokio::time::sleep(COLD_START_TICK).await;
            if let Some(listing) = self.cache.read().listings.get(&id).cloned() {
                return Ok(Some(listing));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    async fn publish_heartbeat(&self, _heartbeat: &ServerHeartbeat) -> AppResult<()> {
        // Heartbeats re-publish the full listing (retained), which IS the
        // heartbeat; nothing extra to send.
        Ok(())
    }

    async fn publish_signal(&self, peer_id: &str, envelope: &SignalingEnvelope) -> AppResult<()> {
        let payload = serde_json::to_vec(envelope)
            .map_err(|err| AppError::Directory(format!("signal encode failed: {err}")))?;
        self.client
            .publish(
                format!("{MQTT_KEY_PREFIX}/signal/{peer_id}"),
                QoS::AtLeastOnce,
                false, // signals are ephemeral by definition
                payload,
            )
            .await
            .map_err(|err| AppError::Directory(format!("broker publish failed: {err}")))
    }

    async fn put_code(&self, code: &str, server_id: uuid::Uuid) -> AppResult<()> {
        self.cache
            .write()
            .codes
            .insert(code.to_uppercase(), server_id);
        self.publish_retained(
            format!("code/{}", code.to_uppercase()),
            serde_json::to_vec(&server_id).unwrap_or_default(),
        )
        .await
    }

    async fn resolve_code(&self, code: &str) -> AppResult<Option<ServerListing>> {
        let lookup = |cache: &MqttCache| {
            cache
                .codes
                .get(&code.to_uppercase())
                .copied()
                .and_then(|id| cache.listings.get(&id).cloned())
        };
        if let Some(listing) = lookup(&self.cache.read()) {
            return Ok(Some(listing));
        }
        // The code may predate our subscription (typed right after launch).
        let deadline = tokio::time::Instant::now() + COLD_START_WAIT;
        loop {
            tokio::time::sleep(COLD_START_TICK).await;
            if let Some(listing) = lookup(&self.cache.read()) {
                return Ok(Some(listing));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    async fn add_players(&self, delta: i64) -> AppResult<i64> {
        // Global counter is derived from listings (`online_players`); the
        // session manager still calls this, so keep a local running value to
        // stay trait-compatible.
        Ok(self
            .local_players
            .fetch_add(delta, Ordering::Relaxed)
            + delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::Directory as _;

    /// The namespace separator must stay consistent: the event loop strips it
    /// with a string prefix, a typo there would silently drop every message.
    #[test]
    fn topic_prefix_is_consistent() {
        let topic = format!("{MQTT_KEY_PREFIX}/listing/abc");
        assert_eq!(
            topic.strip_prefix(&format!("{MQTT_KEY_PREFIX}/")),
            Some("listing/abc")
        );
    }

    #[test]
    fn stale_cutoff_uses_double_ttl() {
        let cutoff = stale_cutoff();
        let expected = Utc::now().timestamp_millis()
            - STALE_MULTIPLIER * i64::from(DEFAULT_HEARTBEAT_TTL_SECS) * 1000;
        assert!((cutoff - expected).abs() < 1000);
    }

    #[test]
    fn is_fresh_rejects_old_heartbeats() {
        let mut listing = test_listing();
        listing.heartbeat_at = Utc::now() - chrono::Duration::seconds(
            3 * i64::from(DEFAULT_HEARTBEAT_TTL_SECS),
        );
        assert!(!is_fresh(&listing));
        listing.heartbeat_at = Utc::now();
        assert!(is_fresh(&listing));
    }

    /// Live round-trip against the real public broker: two independent
    /// directory handles meet through retained messages — one publishes a
    /// listing, the other (subscribed afterwards) must see it in browse, and
    /// deleting it must remove it for everyone.
    ///
    /// Ignored by default: it needs internet and talks to a shared public
    /// broker; CI runs it with `--ignored` on a schedule, and it is exactly
    /// the test to run after touching the retained-message protocol.
    #[tokio::test]
    #[ignore = "requires internet + the public emqx broker"]
    async fn retained_listing_round_trip_between_two_clients() {
        let host_a = MqttDirectory::connect(DEFAULT_MQTT_BROKER, DEFAULT_MQTT_PORT)
            .await
            .expect("connect A");
        let host_b = MqttDirectory::connect(DEFAULT_MQTT_BROKER, DEFAULT_MQTT_PORT)
            .await
            .expect("connect B");

        let mut listing = test_listing();
        // Unique per run so a concurrent CI job on the shared broker cannot
        // collide with us.
        listing.name = format!("probe-{}", uuid::Uuid::new_v4());
        host_a.publish_listing(&listing).await.expect("publish");
        // AtLeastOnce + the loop's command queue: give the broker a moment.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let found = host_b
            .browse(&ServerFilter::default())
            .await
            .expect("browse");
        assert!(
            found.iter().any(|summary| summary.id == listing.id),
            "B must see A's retained listing"
        );
        let fetched = host_b.listing(listing.id).await.expect("listing");
        assert_eq!(fetched.map(|l| l.name), Some(listing.name.clone()));

        // Removal: empty retained payload deletes it for future subscribers.
        host_a.remove_listing(listing.id, "peer").await.expect("remove");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let after = host_b.listing(listing.id).await.expect("listing after");
        assert!(after.is_none(), "removed listing must vanish from the cache");
    }

    fn test_listing() -> ServerListing {
        use crate::models::account::AccountProvider;
        use crate::models::instance::LoaderKind;
        use crate::models::server::{
            ConnectionDescriptor, ConnectionMode, PlayerCount, ServerOwner,
            SERVER_LISTING_SCHEMA,
        };
        ServerListing {
            // NOTE: fields are re-listed here rather than `..Default::default()`
            // so a new ServerListing field forces this helper to be revisited.
            schema_version: SERVER_LISTING_SCHEMA,
            id: uuid::Uuid::new_v4(),
            name: "t".into(),
            description: String::new(),
            motd: String::new(),
            icon_base64: None,
            owner: ServerOwner {
                name: "Steve".into(),
                uuid: uuid::Uuid::new_v4(),
                provider: AccountProvider::Offline,
            },
            game_version: "1.20.1".into(),
            loader: LoaderKind::Vanilla,
            loader_version: None,
            modpack: None,
            required_mod_ids: vec![],
            players: PlayerCount { online: 1, max: 8 },
            connection: ConnectionDescriptor {
                mode: ConnectionMode::DirectP2p,
                peer_id: "peer".into(),
                public_key: String::new(),
                endpoints: vec![],
                relay: None,
                session_token: "t".into(),
                protocol_version: 0,
            },
            region: None,
            tags: vec![],
            whitelist: Default::default(),
            password_protected: false,
            world_name: None,
            created_at: Utc::now(),
            heartbeat_at: Utc::now(),
            ttl_secs: DEFAULT_HEARTBEAT_TTL_SECS,
            world_playtime_secs: 0,
        }
    }
}
