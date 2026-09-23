//! Redis global server browser, signaling bus and share-code directory.
//!
//! Key layout (also documented in [`crate::models::server`]):
//!
//! ```text
//! sxml:servers:index        ZSET    server_id -> heartbeat unix ms (pruned on read)
//! sxml:servers:{id}         STRING  ServerListing JSON, TTL = ttl_secs
//! sxml:heartbeat:{id}       PUBSUB  ServerHeartbeat JSON
//! sxml:signal:{peer_id}     PUBSUB  wakeup only (payload lives in the inbox)
//! sxml:inbox:{peer_id}      LIST    SignalingEnvelope JSON, drained by the peer
//! sxml:code:{CODE}          STRING  server_id, TTL refreshed on each host heartbeat
//! sxml:stats:online         STRING  global concurrent-player counter
//! ```
//!
//! Everything is namespaced by [`KEY_PREFIX`] so one Redis instance can host
//! several environments (dev/staging/prod) without collisions.
//!
//! **Expiry is the feature.** A host that crashes stops refreshing and simply
//! disappears from the browser: no cleanup job, no ghost servers.

use std::time::Duration;

use futures::StreamExt;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, RedisError};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::models::server::{
    JoinRejection, ServerFilter, ServerHeartbeat, ServerListing, ServerListingSummary,
    SignalingEnvelope, DEFAULT_HEARTBEAT_TTL_SECS,
};
use crate::network::code::normalize_share_code;
use async_trait::async_trait;

/// Namespace for every key this app owns.
pub const KEY_PREFIX: &str = "sxml";
/// Redis expiry for a share code between heartbeats.
///
/// The host calls [`RedisDirectory::put_code`] again on every heartbeat, so a
/// live session stays joinable for as long as it is hosted. A crashed host
/// stops refreshing and the code disappears with this delay.
pub const CODE_TTL_SECS: u64 = 300;
/// Cap on listings returned by one browse (protects the UI and the network).
pub const BROWSE_LIMIT: u32 = 200;

/// Redis-backed directory.
///
/// Commands use a `ConnectionManager` (auto-reconnecting, multiplexed); Pub/Sub
/// needs a dedicated connection, which is why the client is kept around too.
#[derive(Clone)]
pub struct RedisDirectory {
    client: redis::Client,
    manager: ConnectionManager,
    prefix: String,
    /// Password from the URL, used only to scrub it out of driver errors.
    /// Never log this field.
    secret: Option<String>,
}

impl RedisDirectory {
    /// Connect and verify the endpoint answers `PING`.
    ///
    /// `rediss://` (Upstash) needs the rustls crypto provider installed before
    /// the client builds its TLS config. Installing twice is an error we ignore.
    pub async fn connect(url: &str) -> AppResult<Self> {
        let _ = redis_rustls::crypto::ring::default_provider().install_default();
        let secret = redis_url_secret(url);
        let client = redis::Client::open(url).map_err(|err| {
            AppError::Directory(format!(
                "DIRECTORY_UNREACHABLE: the Redis URL in Settings is invalid ({})",
                scrub_secret(&err.to_string(), secret.as_deref())
            ))
        })?;
        let manager = tokio::time::timeout(
            Duration::from_secs(10),
            ConnectionManager::new(client.clone()),
        )
        .await
        .map_err(|_| {
            AppError::Directory(format!(
                "DIRECTORY_UNREACHABLE: Redis did not answer in time ({})",
                redact_redis_url(url)
            ))
        })?
        .map_err(|err| {
            AppError::Directory(format!(
                "DIRECTORY_UNREACHABLE: {}",
                scrub_secret(&err.to_string(), secret.as_deref())
            ))
        })?;

        let directory = Self {
            client,
            manager,
            prefix: KEY_PREFIX.to_string(),
            secret,
        };
        directory.ping().await?;
        Ok(directory)
    }

    /// Build a client with a custom namespace (used by tests).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    fn scrub(&self, err: RedisError) -> AppError {
        hint_redis(scrub_secret(&err.to_string(), self.secret.as_deref()))
    }

    fn inbox_key(&self, peer_id: &str) -> String {
        signal_inbox_key(&self.prefix, peer_id)
    }

    /// `PING` — used by the Settings page "Test connection" button.
    pub async fn ping(&self) -> AppResult<()> {
        let mut conn = self.manager.clone();
        let response: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|err| self.scrub(err))?;
        if response != "PONG" {
            return Err(AppError::Directory(format!(
                "unexpected Redis reply to PING: {response}"
            )));
        }
        Ok(())
    }

    /// Concurrent-player counter shown on the dashboard.
    pub async fn online_players(&self) -> AppResult<u64> {
        let mut conn = self.manager.clone();
        let value: Option<String> = conn
            .get(self.key("stats:online"))
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(value.and_then(|raw| raw.parse().ok()).unwrap_or(0))
    }

    fn key(&self, suffix: &str) -> String {
        namespaced(&self.prefix, suffix)
    }

    fn listing_key(&self, id: uuid::Uuid) -> String {
        listing_key(&self.prefix, id)
    }

    fn index_key(&self) -> String {
        index_key(&self.prefix)
    }

    fn signal_channel(&self, peer_id: &str) -> String {
        signal_channel(&self.prefix, peer_id)
    }

    fn heartbeat_channel(&self, id: uuid::Uuid) -> String {
        heartbeat_channel(&self.prefix, id)
    }

    fn code_key(&self, code: &str) -> String {
        code_key(&self.prefix, code)
    }

    /// Publish (or refresh) a listing and index it for browsing.
    pub async fn publish_listing(&self, listing: &ServerListing) -> AppResult<()> {
        let payload = serde_json::to_string(listing)?;
        let mut conn = self.manager.clone();
        let ttl = listing.ttl_secs.max(1);

        // Atomic: heartbeats are frequent and must never leave a listing
        // indexed without a body (or vice versa).
        let script = redis::Script::new(
            r"
            redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
            redis.call('ZADD', KEYS[2], ARGV[3], ARGV[4])
            return 1
            ",
        );
        script
            .key(self.listing_key(listing.id))
            .key(self.index_key())
            .arg(payload)
            .arg(ttl)
            .arg(listing.heartbeat_at.timestamp_millis())
            .arg(listing.id.to_string())
            .invoke_async::<i64>(&mut conn)
            .await
            .map_err(|err| self.scrub(err))?;

        Ok(())
    }

    /// Refresh the TTL without rewriting the whole document.
    pub async fn touch_listing(&self, id: uuid::Uuid, ttl_secs: u32) -> AppResult<bool> {
        let mut conn = self.manager.clone();
        let refreshed: bool = conn
            .expire(self.listing_key(id), i64::from(ttl_secs.max(1)))
            .await
            .map_err(|err| self.scrub(err))?;
        if refreshed {
            let _: () = conn
                .zadd(
                    self.index_key(),
                    id.to_string(),
                    chrono::Utc::now().timestamp_millis(),
                )
                .await
                .map_err(|err| self.scrub(err))?;
        }
        Ok(refreshed)
    }

    /// Remove a listing immediately (graceful host shutdown).
    pub async fn remove_listing(&self, id: uuid::Uuid, peer_id: &str) -> AppResult<()> {
        let mut conn = self.manager.clone();
        let _: () = conn
            .del(self.listing_key(id))
            .await
            .map_err(|err| self.scrub(err))?;
        let _: () = conn
            .zrem(self.index_key(), id.to_string())
            .await
            .map_err(|err| self.scrub(err))?;

        // Best effort: tell guests the world closed.
        let goodbye = SignalingEnvelope::Reject {
            from_peer_id: peer_id.to_string(),
            session_id: id,
            reason: JoinRejection::HostClosed,
            sent_at: chrono::Utc::now(),
        };
        let _ = self.publish_signal(peer_id, &goodbye).await;
        Ok(())
    }

    /// Browse the directory, pruning stale index entries on the way through.
    pub async fn browse(&self, filter: &ServerFilter) -> AppResult<Vec<ServerListingSummary>> {
        let limit = filter.limit.unwrap_or(BROWSE_LIMIT).min(BROWSE_LIMIT);
        let mut conn = self.manager.clone();

        // Alive = heartbeated within twice the TTL (tolerates one missed beat).
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::from(DEFAULT_HEARTBEAT_TTL_SECS) * 2 * 1000;

        // Drop the dead entries so the index does not grow forever. (`zrembyscore`
        // requires both bounds to share a type; the raw command keeps the
        // flexible `-inf` sentinel.)
        let _: i64 = redis::cmd("ZREMRANGEBYSCORE")
            .arg(self.index_key())
            .arg("-inf")
            .arg(cutoff)
            .query_async(&mut conn)
            .await
            .map_err(|err| self.scrub(err))?;

        // The zset range bounds are `isize` in the redis API.
        let stop = isize::try_from(limit.saturating_sub(1)).unwrap_or(isize::MAX);
        let ids: Vec<String> = conn
            .zrevrange(self.index_key(), 0isize, stop)
            .await
            .map_err(|err| self.scrub(err))?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let listing_keys: Vec<String> = ids
            .iter()
            .filter_map(|id| uuid::Uuid::parse_str(id).ok())
            .map(|id| self.listing_key(id))
            .collect();
        let payloads: Vec<Option<String>> = conn
            .mget(listing_keys)
            .await
            .map_err(|err| self.scrub(err))?;

        let mut summaries = Vec::new();
        for payload in payloads.into_iter().flatten() {
            let Ok(listing) = serde_json::from_str::<ServerListing>(&payload) else {
                // A listing written by a newer schema is skipped, not fatal.
                continue;
            };
            if listing.is_stale() || !filter.matches(&listing) {
                continue;
            }
            summaries.push(ServerListingSummary::from(&listing));
        }

        Ok(summaries)
    }

    /// Fetch one listing by id.
    pub async fn listing(&self, id: uuid::Uuid) -> AppResult<Option<ServerListing>> {
        let mut conn = self.manager.clone();
        let payload: Option<String> = conn
            .get(self.listing_key(id))
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(payload.and_then(|raw| serde_json::from_str(&raw).ok()))
    }

    /// Broadcast a heartbeat so watching UIs update counts without refetching.
    pub async fn publish_heartbeat(&self, heartbeat: &ServerHeartbeat) -> AppResult<()> {
        let payload = serde_json::to_string(heartbeat)?;
        let mut conn = self.manager.clone();
        let _: () = conn
            .publish(self.heartbeat_channel(heartbeat.server_id), payload)
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Subscribe to a listing's heartbeat (server detail pane).
    ///
    /// Returns a channel rather than a `Stream`: pub/sub needs its own dedicated
    /// connection, which a background task owns and tears down with the channel.
    pub async fn subscribe_heartbeat(
        &self,
        id: uuid::Uuid,
    ) -> AppResult<mpsc::Receiver<ServerHeartbeat>> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|err| self.scrub(err))?;
        pubsub
            .subscribe(self.heartbeat_channel(id))
            .await
            .map_err(|err| self.scrub(err))?;
        // `OnMessage` is the stream type redis exposes for pub/sub; `PubSub`
        // itself is not a `Stream`.
        let mut stream = pubsub.into_on_message();

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let Ok(payload) = message.get_payload::<String>() else {
                    continue;
                };
                let Ok(heartbeat) = serde_json::from_str::<ServerHeartbeat>(&payload) else {
                    continue;
                };
                // A dropped receiver means nobody is watching: stop reading.
                if tx.send(heartbeat).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    /// Publish a signaling envelope.
    ///
    /// The inbox list is written first and is the source of truth. Pub/sub is
    /// only a wakeup, so an offer published before the peer subscribes — or
    /// during a dropped Upstash connection — is still drained later.
    pub async fn publish_signal(
        &self,
        peer_id: &str,
        envelope: &SignalingEnvelope,
    ) -> AppResult<()> {
        let payload = serde_json::to_string(envelope)?;
        let mut conn = self.manager.clone();
        let inbox = self.inbox_key(peer_id);
        let _: () = conn
            .lpush(&inbox, &payload)
            .await
            .map_err(|err| self.scrub(err))?;
        let _: () = conn
            .ltrim(&inbox, 0, 31)
            .await
            .map_err(|err| self.scrub(err))?;
        let _: () = conn
            .expire(&inbox, 120)
            .await
            .map_err(|err| self.scrub(err))?;
        // Wake a live subscriber. Zero receivers is fine: the inbox holds it.
        let _: i64 = conn
            .publish(self.signal_channel(peer_id), "1")
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Take every pending envelope for `peer_id` (oldest first).
    ///
    /// Redis runs the pops inside one script, so a concurrent `LPUSH` cannot
    /// land between the read and the delete.
    pub async fn drain_signals(&self, peer_id: &str) -> AppResult<Vec<SignalingEnvelope>> {
        let mut conn = self.manager.clone();
        let script = redis::Script::new(
            r"
            local out = {}
            for _ = 1, 32 do
              local item = redis.call('RPOP', KEYS[1])
              if not item then break end
              table.insert(out, item)
            end
            return out
            ",
        );
        let payloads: Vec<String> = script
            .key(self.inbox_key(peer_id))
            .invoke_async(&mut conn)
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(payloads
            .into_iter()
            .filter_map(|payload| serde_json::from_str(&payload).ok())
            .collect())
    }

    /// Subscribe to our own signaling channel, reconnecting if the socket drops.
    ///
    /// Messages are taken from the inbox (see [`Self::publish_signal`]). The
    /// pub/sub stream is a wakeup plus a reconnect trigger, not the payload.
    pub async fn subscribe_signals(
        &self,
        peer_id: &str,
    ) -> AppResult<mpsc::Receiver<SignalingEnvelope>> {
        let (tx, rx) = mpsc::channel(128);
        let directory = self.clone();
        let peer = peer_id.to_string();
        let channel = self.signal_channel(peer_id);
        tokio::spawn(async move {
            if forward_inbox(&directory, &peer, &tx).await.is_err() {
                return;
            }
            loop {
                if tx.is_closed() {
                    break;
                }
                let subscribed = async {
                    let mut pubsub = directory.client.get_async_pubsub().await?;
                    pubsub.subscribe(&channel).await?;
                    Ok::<_, RedisError>(pubsub.into_on_message())
                };
                let mut stream = match subscribed.await {
                    Ok(stream) => stream,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                loop {
                    let message = tokio::select! {
                        _ = tx.closed() => return,
                        message = stream.next() => message,
                    };
                    if message.is_none() {
                        break;
                    }
                    if forward_inbox(&directory, &peer, &tx).await.is_err() {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });
        Ok(rx)
    }

    /// Store the listing body without adding it to the browse index.
    ///
    /// Private ("share-code only") sessions still have to be resolvable by id
    /// after `put_code`. Heartbeats call this again to refresh the TTL.
    pub async fn save_listing(&self, listing: &ServerListing) -> AppResult<()> {
        let payload = serde_json::to_string(listing)?;
        let mut conn = self.manager.clone();
        let ttl = u64::from(listing.ttl_secs.max(1));
        let _: () = conn
            .set_ex(self.listing_key(listing.id), payload, ttl)
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Store a share code so a friend can type it instead of a raw address.
    ///
    /// Calling this again resets the TTL. The host heartbeat does that for the
    /// whole session; `CODE_TTL_SECS` is only the gap after the last refresh.
    pub async fn put_code(&self, code: &str, server_id: uuid::Uuid) -> AppResult<()> {
        let mut conn = self.manager.clone();
        let _: () = conn
            .set_ex(self.code_key(code), server_id.to_string(), CODE_TTL_SECS)
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Drop a share code immediately (host stopped).
    pub async fn delete_code(&self, code: &str) -> AppResult<()> {
        let mut conn = self.manager.clone();
        let _: () = conn
            .del(self.code_key(code))
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Resolve a share code into its listing.
    pub async fn resolve_code(&self, code: &str) -> AppResult<Option<ServerListing>> {
        Ok(match self.lookup_invite(code).await? {
            InviteLookup::Found(listing) => Some(listing),
            InviteLookup::Unknown | InviteLookup::ListingMissing { .. } => None,
        })
    }

    /// Distinguish "no such code" from "code maps at a listing that already expired".
    pub async fn lookup_invite(&self, code: &str) -> AppResult<InviteLookup> {
        let mut conn = self.manager.clone();
        let id: Option<String> = conn
            .get(self.code_key(code))
            .await
            .map_err(|err| self.scrub(err))?;
        let Some(id) = id.and_then(|raw| uuid::Uuid::parse_str(&raw).ok()) else {
            return Ok(InviteLookup::Unknown);
        };
        match self.listing(id).await? {
            Some(listing) => Ok(InviteLookup::Found(listing)),
            None => Ok(InviteLookup::ListingMissing { server_id: id }),
        }
    }

    /// Write the listing body and the share-code mapping in one script.
    ///
    /// A crash between the two writes used to leave a code that resolved to
    /// nothing (or a listing with no code). Heartbeats call this every tick.
    pub async fn sync_session(
        &self,
        listing: &ServerListing,
        code: &str,
        public: bool,
    ) -> AppResult<()> {
        let payload = serde_json::to_string(listing)?;
        let mut conn = self.manager.clone();
        let script = redis::Script::new(
            r"
            redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
            redis.call('SET', KEYS[2], ARGV[3], 'EX', ARGV[4])
            if ARGV[5] == '1' then
              redis.call('ZADD', KEYS[3], ARGV[6], ARGV[7])
            else
              redis.call('ZREM', KEYS[3], ARGV[7])
            end
            return 1
            ",
        );
        script
            .key(self.listing_key(listing.id))
            .key(self.code_key(code))
            .key(self.index_key())
            .arg(payload)
            .arg(listing.ttl_secs.max(1))
            .arg(listing.id.to_string())
            .arg(CODE_TTL_SECS)
            .arg(if public { "1" } else { "0" })
            .arg(listing.heartbeat_at.timestamp_millis())
            .arg(listing.id.to_string())
            .invoke_async::<i64>(&mut conn)
            .await
            .map_err(|err| self.scrub(err))?;
        Ok(())
    }

    /// Increment the global concurrent-player counter.
    pub async fn add_players(&self, delta: i64) -> AppResult<i64> {
        let mut conn = self.manager.clone();
        conn.incr(self.key("stats:online"), delta)
            .await
            .map_err(|err| self.scrub(err))
    }
}

// --- key builders (pure functions so the layout can be tested without Redis) --

/// `sxml:<suffix>`
pub fn namespaced(prefix: &str, suffix: &str) -> String {
    format!("{prefix}:{suffix}")
}

pub fn listing_key(prefix: &str, id: uuid::Uuid) -> String {
    namespaced(prefix, &format!("servers:{id}"))
}

pub fn index_key(prefix: &str) -> String {
    namespaced(prefix, "servers:index")
}

pub fn signal_channel(prefix: &str, peer_id: &str) -> String {
    namespaced(prefix, &format!("signal:{peer_id}"))
}

pub fn heartbeat_channel(prefix: &str, id: uuid::Uuid) -> String {
    namespaced(prefix, &format!("heartbeat:{id}"))
}

/// Share codes are case-insensitive. Full codes use the canonical grouped form
/// so a paste without dashes hits the same key the host wrote.
pub fn code_key(prefix: &str, code: &str) -> String {
    namespaced(prefix, &format!("code:{}", normalize_share_code(code)))
}

pub fn signal_inbox_key(prefix: &str, peer_id: &str) -> String {
    namespaced(prefix, &format!("inbox:{peer_id}"))
}

/// `true` when `redisUrl` points at a shared server (Upstash, a VPS) rather
/// than the localhost default. A shared URL is the directory both launchers
/// must use; the public MQTT broker is only the fallback for the local default.
pub fn redis_endpoint_is_shared(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']);
    !matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1")
}

/// Drop userinfo so a status pill or log line cannot show the password.
pub fn redact_redis_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return "redis://invalid".to_string();
    };
    let _ = parsed.set_password(None);
    let _ = parsed.set_username("");
    parsed.to_string()
}

fn redis_url_secret(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.password().map(str::to_string))
        .filter(|secret| !secret.is_empty())
}

fn scrub_secret(message: &str, secret: Option<&str>) -> String {
    match secret {
        Some(secret) if !secret.is_empty() => message.replace(secret, "***"),
        _ => message.to_string(),
    }
}

async fn forward_inbox(
    directory: &RedisDirectory,
    peer_id: &str,
    tx: &mpsc::Sender<SignalingEnvelope>,
) -> Result<(), ()> {
    let pending = directory.drain_signals(peer_id).await.map_err(|_| ())?;
    for envelope in pending {
        if tx.send(envelope).await.is_err() {
            return Err(());
        }
    }
    Ok(())
}

/// Publish anything serializable to a channel (used by tests and ad-hoc events).
pub async fn publish_json<T: Serialize>(
    directory: &RedisDirectory,
    channel: &str,
    value: &T,
) -> AppResult<()> {
    let payload = serde_json::to_string(value)?;
    let mut conn = directory.manager.clone();
    let _: () = conn
        .publish(channel, payload)
        .await
        .map_err(|err| directory.scrub(err))?;
    Ok(())
}

/// Translate a Redis error into a domain error with a useful hint.
fn map_redis(err: RedisError) -> AppError {
    hint_redis(err.to_string())
}

fn hint_redis(message: String) -> AppError {
    let hint = if message.contains("Connection refused") || message.contains("os error 61") {
        " — the Redis directory server is not reachable. Check the URL in Settings."
    } else if message.contains("NOAUTH") || message.contains("WRONGPASS") {
        " — the Redis server requires a password."
    } else {
        ""
    };
    AppError::Directory(format!("{message}{hint}"))
}

/// Result of looking up a share code.
#[derive(Debug, Clone)]
pub enum InviteLookup {
    Found(ServerListing),
    /// No mapping. The code expired, was never published, or the host stopped.
    Unknown,
    /// The code still maps, but the listing body is gone (heartbeat missed).
    ListingMissing {
        server_id: uuid::Uuid,
    },
}

/// What the session manager needs from a directory.
///
/// [`RedisDirectory`] is the production implementation (the global server
/// browser); [`MemoryDirectory`] is an in-process one so the session lifecycle
/// can be tested end to end without any infrastructure.
#[async_trait]
pub trait Directory: Send + Sync + 'static {
    async fn ping(&self) -> AppResult<()>;
    async fn online_players(&self) -> AppResult<u64>;
    async fn publish_listing(&self, listing: &ServerListing) -> AppResult<()>;
    /// Persist the listing body without indexing it for browse.
    async fn save_listing(&self, listing: &ServerListing) -> AppResult<()>;
    async fn touch_listing(&self, id: uuid::Uuid, ttl_secs: u32) -> AppResult<bool>;
    async fn remove_listing(&self, id: uuid::Uuid, peer_id: &str) -> AppResult<()>;
    async fn browse(&self, filter: &ServerFilter) -> AppResult<Vec<ServerListingSummary>>;
    async fn listing(&self, id: uuid::Uuid) -> AppResult<Option<ServerListing>>;
    async fn publish_heartbeat(&self, heartbeat: &ServerHeartbeat) -> AppResult<()>;
    async fn publish_signal(&self, peer_id: &str, envelope: &SignalingEnvelope) -> AppResult<()>;
    /// Pending envelopes for `peer_id`, oldest first. Empty when nothing is waiting.
    async fn drain_signals(&self, peer_id: &str) -> AppResult<Vec<SignalingEnvelope>>;
    async fn put_code(&self, code: &str, server_id: uuid::Uuid) -> AppResult<()>;
    async fn delete_code(&self, code: &str) -> AppResult<()>;
    async fn resolve_code(&self, code: &str) -> AppResult<Option<ServerListing>>;
    async fn lookup_invite(&self, code: &str) -> AppResult<InviteLookup>;
    /// Refresh the listing and the share code together.
    async fn sync_session(
        &self,
        listing: &ServerListing,
        code: &str,
        public: bool,
    ) -> AppResult<()> {
        if public {
            self.publish_listing(listing).await?;
        } else {
            self.save_listing(listing).await?;
        }
        self.put_code(code, listing.id).await
    }
    async fn add_players(&self, delta: i64) -> AppResult<i64>;
}

#[async_trait]
impl Directory for RedisDirectory {
    async fn ping(&self) -> AppResult<()> {
        RedisDirectory::ping(self).await
    }

    async fn online_players(&self) -> AppResult<u64> {
        RedisDirectory::online_players(self).await
    }

    async fn publish_listing(&self, listing: &ServerListing) -> AppResult<()> {
        RedisDirectory::publish_listing(self, listing).await
    }

    async fn save_listing(&self, listing: &ServerListing) -> AppResult<()> {
        RedisDirectory::save_listing(self, listing).await
    }

    async fn touch_listing(&self, id: uuid::Uuid, ttl_secs: u32) -> AppResult<bool> {
        RedisDirectory::touch_listing(self, id, ttl_secs).await
    }

    async fn remove_listing(&self, id: uuid::Uuid, peer_id: &str) -> AppResult<()> {
        RedisDirectory::remove_listing(self, id, peer_id).await
    }

    async fn browse(&self, filter: &ServerFilter) -> AppResult<Vec<ServerListingSummary>> {
        RedisDirectory::browse(self, filter).await
    }

    async fn listing(&self, id: uuid::Uuid) -> AppResult<Option<ServerListing>> {
        RedisDirectory::listing(self, id).await
    }

    async fn publish_heartbeat(&self, heartbeat: &ServerHeartbeat) -> AppResult<()> {
        RedisDirectory::publish_heartbeat(self, heartbeat).await
    }

    async fn publish_signal(&self, peer_id: &str, envelope: &SignalingEnvelope) -> AppResult<()> {
        RedisDirectory::publish_signal(self, peer_id, envelope).await
    }

    async fn drain_signals(&self, peer_id: &str) -> AppResult<Vec<SignalingEnvelope>> {
        RedisDirectory::drain_signals(self, peer_id).await
    }

    async fn put_code(&self, code: &str, server_id: uuid::Uuid) -> AppResult<()> {
        RedisDirectory::put_code(self, code, server_id).await
    }

    async fn delete_code(&self, code: &str) -> AppResult<()> {
        RedisDirectory::delete_code(self, code).await
    }

    async fn resolve_code(&self, code: &str) -> AppResult<Option<ServerListing>> {
        RedisDirectory::resolve_code(self, code).await
    }

    async fn lookup_invite(&self, code: &str) -> AppResult<InviteLookup> {
        RedisDirectory::lookup_invite(self, code).await
    }

    async fn sync_session(
        &self,
        listing: &ServerListing,
        code: &str,
        public: bool,
    ) -> AppResult<()> {
        RedisDirectory::sync_session(self, listing, code, public).await
    }

    async fn add_players(&self, delta: i64) -> AppResult<i64> {
        RedisDirectory::add_players(self, delta).await
    }
}

/// In-process directory: same semantics, no Redis.
///
/// Browse only returns listings passed through [`Directory::publish_listing`].
/// [`Directory::save_listing`] keeps a body for share-code lookup without
/// indexing it. Share codes expire `CODE_TTL_SECS` after the last `put_code`
/// on the virtual clock ([`MemoryDirectory::advance_secs`]).
#[derive(Clone)]
pub struct MemoryDirectory {
    listings:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<uuid::Uuid, ServerListing>>>,
    /// Ids that belong in the public browser.
    indexed: std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<uuid::Uuid>>>,
    /// code → (server id, expiry unix seconds on the virtual clock).
    codes: std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<String, (uuid::Uuid, i64)>>>,
    /// peer id → pending signaling envelopes (oldest at the front).
    signals: std::sync::Arc<
        parking_lot::Mutex<
            std::collections::HashMap<String, std::collections::VecDeque<SignalingEnvelope>>,
        >,
    >,
    now_secs: std::sync::Arc<parking_lot::Mutex<i64>>,
    players: std::sync::Arc<std::sync::atomic::AtomicI64>,
}

impl Default for MemoryDirectory {
    fn default() -> Self {
        Self {
            listings: std::sync::Arc::new(
                parking_lot::Mutex::new(std::collections::HashMap::new()),
            ),
            indexed: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
            codes: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            signals: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            now_secs: std::sync::Arc::new(parking_lot::Mutex::new(chrono::Utc::now().timestamp())),
            players: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }
}

impl MemoryDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Move the directory clock forward. Used to prove a refreshed share code
    /// still resolves after more than [`CODE_TTL_SECS`] without sleeping.
    pub fn advance_secs(&self, secs: i64) {
        *self.now_secs.lock() += secs;
    }

    fn now_secs(&self) -> i64 {
        *self.now_secs.lock()
    }
}

#[async_trait]
impl Directory for MemoryDirectory {
    async fn ping(&self) -> AppResult<()> {
        Ok(())
    }

    async fn online_players(&self) -> AppResult<u64> {
        Ok(self
            .players
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(0) as u64)
    }

    async fn publish_listing(&self, listing: &ServerListing) -> AppResult<()> {
        self.listings.lock().insert(listing.id, listing.clone());
        self.indexed.lock().insert(listing.id);
        Ok(())
    }

    async fn save_listing(&self, listing: &ServerListing) -> AppResult<()> {
        self.listings.lock().insert(listing.id, listing.clone());
        Ok(())
    }

    async fn touch_listing(&self, id: uuid::Uuid, _ttl_secs: u32) -> AppResult<bool> {
        Ok(self.listings.lock().contains_key(&id))
    }

    async fn remove_listing(&self, id: uuid::Uuid, _peer_id: &str) -> AppResult<()> {
        self.listings.lock().remove(&id);
        self.indexed.lock().remove(&id);
        Ok(())
    }

    async fn browse(&self, filter: &ServerFilter) -> AppResult<Vec<ServerListingSummary>> {
        let indexed = self.indexed.lock().clone();
        let mut out = Vec::new();
        for listing in self.listings.lock().values() {
            if !indexed.contains(&listing.id) || !filter.matches(listing) {
                continue;
            }
            out.push(ServerListingSummary::from(listing));
        }
        out.sort_by(|a, b| b.heartbeat_at.cmp(&a.heartbeat_at));
        Ok(out)
    }

    async fn listing(&self, id: uuid::Uuid) -> AppResult<Option<ServerListing>> {
        Ok(self.listings.lock().get(&id).cloned())
    }

    async fn publish_heartbeat(&self, _heartbeat: &ServerHeartbeat) -> AppResult<()> {
        Ok(())
    }

    async fn publish_signal(&self, peer_id: &str, envelope: &SignalingEnvelope) -> AppResult<()> {
        let mut signals = self.signals.lock();
        let inbox = signals.entry(peer_id.to_string()).or_default();
        inbox.push_back(envelope.clone());
        while inbox.len() > 32 {
            inbox.pop_front();
        }
        Ok(())
    }

    async fn drain_signals(&self, peer_id: &str) -> AppResult<Vec<SignalingEnvelope>> {
        let mut signals = self.signals.lock();
        Ok(signals
            .remove(peer_id)
            .map(|inbox| inbox.into_iter().collect())
            .unwrap_or_default())
    }

    async fn put_code(&self, code: &str, server_id: uuid::Uuid) -> AppResult<()> {
        let expires_at = self.now_secs() + i64::try_from(CODE_TTL_SECS).unwrap_or(i64::MAX);
        self.codes
            .lock()
            .insert(normalize_share_code(code), (server_id, expires_at));
        Ok(())
    }

    async fn delete_code(&self, code: &str) -> AppResult<()> {
        self.codes.lock().remove(&normalize_share_code(code));
        Ok(())
    }

    async fn lookup_invite(&self, code: &str) -> AppResult<InviteLookup> {
        let now = self.now_secs();
        let key = normalize_share_code(code);
        let found = self.codes.lock().get(&key).copied();
        match found {
            Some((id, expires_at)) if expires_at > now => match self.listing(id).await? {
                Some(listing) => Ok(InviteLookup::Found(listing)),
                None => Ok(InviteLookup::ListingMissing { server_id: id }),
            },
            _ => Ok(InviteLookup::Unknown),
        }
    }

    async fn resolve_code(&self, code: &str) -> AppResult<Option<ServerListing>> {
        Ok(match self.lookup_invite(code).await? {
            InviteLookup::Found(listing) => Some(listing),
            InviteLookup::Unknown | InviteLookup::ListingMissing { .. } => None,
        })
    }

    async fn add_players(&self, delta: i64) -> AppResult<i64> {
        Ok(self
            .players
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed)
            + delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::account::AccountProvider;
    use crate::models::instance::LoaderKind;
    use crate::models::server::{
        ConnectionDescriptor, ConnectionMode, PlayerCount, ServerFilter, ServerOwner,
        WhitelistPolicy, SERVER_LISTING_SCHEMA,
    };

    fn listing(name: &str, online: u32) -> ServerListing {
        ServerListing {
            schema_version: SERVER_LISTING_SCHEMA,
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            description: "test world".into(),
            motd: "welcome".into(),
            icon_base64: None,
            owner: ServerOwner {
                name: "Steve".into(),
                uuid: uuid::Uuid::new_v4(),
                provider: AccountProvider::Offline,
            },
            game_version: "1.20.1".into(),
            loader: LoaderKind::Fabric,
            loader_version: Some("0.15.11".into()),
            modpack: None,
            required_mod_ids: vec![],
            players: PlayerCount { online, max: 8 },
            connection: ConnectionDescriptor {
                mode: ConnectionMode::DirectP2p,
                peer_id: "peer".into(),
                public_key: "key".into(),
                endpoints: vec![],
                relay: None,
                session_token: "token".into(),
                protocol_version: 763,
            },
            region: Some("eu".into()),
            tags: vec!["survival".into()],
            whitelist: WhitelistPolicy::default(),
            password_protected: false,
            world_name: Some("Overworld".into()),
            created_at: chrono::Utc::now(),
            heartbeat_at: chrono::Utc::now(),
            ttl_secs: DEFAULT_HEARTBEAT_TTL_SECS,
            world_playtime_secs: 0,
        }
    }

    #[test]
    fn keys_are_namespaced_and_stable() {
        let id = uuid::Uuid::nil();
        assert_eq!(index_key(KEY_PREFIX), "sxml:servers:index");
        assert_eq!(listing_key(KEY_PREFIX, id), format!("sxml:servers:{id}"));
        assert_eq!(signal_channel(KEY_PREFIX, "abc"), "sxml:signal:abc");
        assert_eq!(
            heartbeat_channel(KEY_PREFIX, id),
            format!("sxml:heartbeat:{id}")
        );
        // Share codes are case-insensitive.
        assert_eq!(code_key(KEY_PREFIX, "sxm1-abcd"), "sxml:code:SXM1-ABCD");
        assert_eq!(code_key(KEY_PREFIX, "SXM1-ABCD"), "sxml:code:SXM1-ABCD");
    }

    #[test]
    fn custom_prefixes_isolate_environments() {
        let id = uuid::Uuid::nil();
        assert!(listing_key("sxml-dev", id).starts_with("sxml-dev:"));
        assert_ne!(listing_key("sxml-dev", id), listing_key("sxml", id));
    }

    #[test]
    fn rediss_urls_parse_when_tls_is_compiled_in() {
        // This is the exact failure mode without `tokio-rustls-comp`:
        // `Client::open` rejects `rediss://` before any socket is opened.
        let opened = redis::Client::open("rediss://default:token@example.upstash.io:6379");
        assert!(
            opened.is_ok(),
            "rediss:// was rejected: {}",
            opened
                .as_ref()
                .err()
                .map(|err| err.to_string())
                .unwrap_or_default()
        );
    }

    #[tokio::test]
    async fn rediss_connect_is_not_rejected_for_missing_tls() {
        // Nothing listens here. A refused connection means the TLS feature
        // compiled in and the client tried the socket.
        let err = match RedisDirectory::connect("rediss://default:token@127.0.0.1:1").await {
            Ok(_) => panic!("port 1 accepted a Redis connection"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            !message.contains("feature is not enabled"),
            "TLS client was not compiled in: {message}"
        );
    }

    #[test]
    fn redis_errors_get_actionable_hints() {
        let refused = RedisError::from((redis::ErrorKind::IoError, "Connection refused"));
        let mapped = map_redis(refused);
        assert!(mapped.to_string().contains("not reachable"));

        let auth = RedisError::from((redis::ErrorKind::AuthenticationFailed, "NOAUTH"));
        assert!(map_redis(auth).to_string().contains("password"));
    }

    #[tokio::test]
    async fn refreshed_share_code_survives_past_one_ttl() {
        let directory = MemoryDirectory::new();
        let world = listing("Invite", 0);
        directory.save_listing(&world).await.expect("save");
        directory
            .put_code("SXM1-TEST", world.id)
            .await
            .expect("code");

        let step = i64::try_from(CODE_TTL_SECS).unwrap() - 10;
        directory.advance_secs(step);
        assert!(
            directory
                .resolve_code("sxm1-test")
                .await
                .expect("resolve")
                .is_some(),
            "code should still be inside the first TTL window"
        );

        // Heartbeat refreshes the mapping. Wall time since the first put is
        // now longer than CODE_TTL_SECS.
        directory
            .put_code("SXM1-TEST", world.id)
            .await
            .expect("refresh");
        directory.advance_secs(step);
        let refreshed = directory
            .resolve_code("SXM1-TEST")
            .await
            .expect("resolve")
            .expect("invite stays live while the host heartbeats");
        assert_eq!(refreshed.id, world.id);

        directory.advance_secs(20);
        assert!(
            directory
                .resolve_code("SXM1-TEST")
                .await
                .expect("resolve")
                .is_none(),
            "a code that is not refreshed must expire"
        );
    }

    #[tokio::test]
    async fn private_listing_resolves_by_code_and_stays_out_of_browse() {
        let directory = MemoryDirectory::new();
        let world = listing("Private", 1);
        directory.save_listing(&world).await.expect("save");
        directory
            .put_code("SXM1-PRIV", world.id)
            .await
            .expect("code");

        let resolved = directory
            .resolve_code("SXM1-PRIV")
            .await
            .expect("resolve")
            .expect("private invite");
        assert_eq!(resolved.id, world.id);
        let by_id = directory
            .listing(world.id)
            .await
            .expect("listing")
            .expect("body");
        assert_eq!(by_id.name, "Private");

        let browse = directory
            .browse(&ServerFilter::default())
            .await
            .expect("browse");
        assert!(browse.iter().all(|row| row.id != world.id));
    }

    #[tokio::test]
    async fn sloppy_share_code_resolves_and_missing_listing_is_distinct() {
        use crate::network::code::{CodeFlags, ConnectCode};

        let directory = MemoryDirectory::new();
        let world = listing("Invite", 0);
        let code = ConnectCode::relay(
            world.id,
            3,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        );
        let canonical = code.to_share_string();
        directory.save_listing(&world).await.expect("save");
        directory
            .put_code(&canonical, world.id)
            .await
            .expect("code");

        let sloppy = canonical.to_lowercase().replace('-', "");
        let resolved = directory
            .resolve_code(&sloppy)
            .await
            .expect("resolve")
            .expect("canonical key");
        assert_eq!(resolved.id, world.id);
        assert_eq!(
            code_key(KEY_PREFIX, &sloppy),
            code_key(KEY_PREFIX, &canonical)
        );

        directory.listings.lock().remove(&world.id);
        match directory.lookup_invite(&canonical).await.expect("lookup") {
            InviteLookup::ListingMissing { server_id } => assert_eq!(server_id, world.id),
            other => panic!("expected a missing listing, got {other:?}"),
        }
        directory.delete_code(&sloppy).await.expect("delete");
        assert!(matches!(
            directory.lookup_invite(&canonical).await.expect("lookup"),
            InviteLookup::Unknown
        ));
    }

    #[tokio::test]
    async fn signal_inbox_keeps_an_offer_published_before_drain() {
        use crate::models::server::SignalingEnvelope;

        let directory = MemoryDirectory::new();
        let envelope = SignalingEnvelope::KeepAlive {
            session_id: uuid::Uuid::nil(),
            sent_at: chrono::Utc::now(),
        };
        directory
            .publish_signal("host-peer", &envelope)
            .await
            .expect("publish");
        // Nothing is subscribed yet. The inbox must still yield the envelope.
        let drained = directory.drain_signals("host-peer").await.expect("drain");
        assert_eq!(drained.len(), 1);
        assert!(directory
            .drain_signals("host-peer")
            .await
            .expect("second")
            .is_empty());
    }

    #[test]
    fn shared_redis_urls_are_detected_and_redacted() {
        assert!(!redis_endpoint_is_shared("redis://127.0.0.1:6379/0"));
        assert!(!redis_endpoint_is_shared("redis://localhost:6379/0"));
        assert!(redis_endpoint_is_shared(
            "rediss://default:s3cret-not-real@example.upstash.io:6379"
        ));
        let redacted = redact_redis_url("rediss://default:s3cret-not-real@example.upstash.io:6379");
        assert!(!redacted.contains("s3cret-not-real"));
        assert!(redacted.contains("example.upstash.io"));
        let scrubbed = scrub_secret("NOAUTH s3cret-not-real", Some("s3cret-not-real"));
        assert!(!scrubbed.contains("s3cret-not-real"));
        assert_eq!(redact_redis_url("not a url"), "redis://invalid");
    }

    #[test]
    fn listings_serialize_with_the_documented_schema_version() {
        let value = serde_json::to_value(listing("World", 2)).expect("serialize");
        assert_eq!(value["schemaVersion"], SERVER_LISTING_SCHEMA);
        assert_eq!(value["players"]["online"], 2);
        assert!(value["connection"]["endpoints"].is_array());
    }
}
