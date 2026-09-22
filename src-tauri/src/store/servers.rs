//! Server browser cache, favourites, join history and P2P session diagnostics.
//!
//! Redis is the live directory; this module is what the browser shows while
//! offline and what makes "recently played with friends" work.
//!
//! **Time storage format.** Everything in this module is stored as unix
//! milliseconds, matching the convention documented on [`crate::store`]. Redis
//! publishes ISO-8601 timestamps, but the directory parses those into
//! `DateTime<Utc>` before they ever reach SQLite, so there is exactly one
//! on-disk time representation.

use chrono::{DateTime, Utc};
use rusqlite::params;
use uuid::Uuid;

use crate::error::AppResult;
use crate::models::server::{ConnectionMode, ServerListing};
use crate::store::{
    bool_to_int, datetime_to_millis, int_to_bool, millis_to_datetime, Database,
};

/// A cached listing plus its local annotations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedServer {
    pub listing: ServerListing,
    pub favorite: bool,
    /// Unix milliseconds since epoch (UTC).
    pub last_seen_at: DateTime<Utc>,
}

/// One row of `join_history`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinHistoryEntry {
    pub server_id: Option<Uuid>,
    pub server_name: String,
    pub instance_id: Option<Uuid>,
    pub mode: Option<ConnectionMode>,
    /// Unix milliseconds since epoch (UTC).
    pub joined_at: DateTime<Utc>,
    /// `success` | `refused` | `failed` | `timeout`
    pub outcome: String,
    pub detail: Option<String>,
}

impl JoinHistoryEntry {
    /// A successful join of `server_name` performed `now`.
    pub fn success(
        server_id: Option<Uuid>,
        server_name: impl Into<String>,
        instance_id: Option<Uuid>,
        mode: Option<ConnectionMode>,
        detail: Option<String>,
    ) -> Self {
        Self {
            server_id,
            server_name: server_name.into(),
            instance_id,
            mode,
            joined_at: Utc::now(),
            outcome: "success".to_string(),
            detail,
        }
    }

    /// A join that did not complete. `outcome` is one of `refused`, `failed`,
    /// `timeout` — the values the diagnostics panel understands.
    pub fn failure(
        server_id: Option<Uuid>,
        server_name: impl Into<String>,
        outcome: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            server_id,
            server_name: server_name.into(),
            instance_id: None,
            mode: None,
            joined_at: Utc::now(),
            outcome: outcome.to_string(),
            detail: Some(detail.into()),
        }
    }
}

/// One row of `p2p_sessions` — the data behind the connection diagnostics panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pSessionRecord {
    pub id: Uuid,
    /// `host` | `guest`
    pub role: String,
    pub peer_id: String,
    pub mode: ConnectionMode,
    pub local_port: Option<u16>,
    /// Unix milliseconds since epoch (UTC).
    pub started_at: DateTime<Utc>,
    /// Unix milliseconds since epoch (UTC) when present.
    pub ended_at: Option<DateTime<Utc>>,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub rtt_ms: Option<u32>,
    pub detail: Option<String>,
}

/// SQLite has no unsigned integers; `FromSql` is only implemented for `i64`.
/// A negative value would mean corruption, so clamp instead of wrapping.
fn to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

impl Database {
    /// Cache a listing we saw in the browser (called on browse and after a join).
    pub fn cache_server_listing(&self, listing: &ServerListing) -> AppResult<()> {
        let payload = serde_json::to_string(listing)?;
        let last_seen = datetime_to_millis(Utc::now());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO server_cache (id, listing_json, favorite, last_seen_at)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT (id) DO UPDATE SET
                    listing_json = excluded.listing_json,
                    last_seen_at = excluded.last_seen_at",
                params![listing.id.to_string(), payload, last_seen],
            )?;
            Ok(())
        })
    }

    pub fn cached_servers(&self) -> AppResult<Vec<CachedServer>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT listing_json, favorite, last_seen_at
                 FROM server_cache ORDER BY favorite DESC, last_seen_at DESC",
            )?;
            // Deserialize outside the `query_map` closure: a JSON error is an
            // `AppError`, and closures handed to rusqlite must return
            // `rusqlite::Result`.
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            let mut out = Vec::new();
            for row in rows {
                let (payload, favorite, last_seen_at) = row?;
                if let Ok(listing) = serde_json::from_str::<ServerListing>(&payload) {
                    out.push(CachedServer {
                        listing,
                        favorite: int_to_bool(favorite),
                        last_seen_at: millis_to_datetime(last_seen_at),
                    });
                }
            }
            Ok(out)
        })
    }

    /// Mark/unmark a cached listing as a favourite.
    ///
    /// Returns `false` when the server is not cached — the caller can then cache
    /// it first, which keeps favourites usable straight from a browse result.
    pub fn set_server_favorite(&self, id: Uuid, favorite: bool) -> AppResult<bool> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE server_cache SET favorite = ?2 WHERE id = ?1",
                params![id.to_string(), bool_to_int(favorite)],
            )?;
            Ok(updated > 0)
        })
    }

    pub fn recent_joins(&self, limit: u32) -> AppResult<Vec<JoinHistoryEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT server_id, server_name, instance_id, mode, joined_at, outcome, detail
                 FROM join_history ORDER BY joined_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit as i64], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?;

            let mut out = Vec::new();
            for row in rows {
                let (server_id, server_name, instance_id, mode, joined_at, outcome, detail) = row?;
                out.push(JoinHistoryEntry {
                    server_id: server_id.and_then(|raw| Uuid::parse_str(&raw).ok()),
                    server_name,
                    instance_id: instance_id.and_then(|raw| Uuid::parse_str(&raw).ok()),
                    mode: mode.as_deref().and_then(ConnectionMode::from_str_opt),
                    joined_at: millis_to_datetime(joined_at),
                    outcome,
                    detail,
                });
            }
            Ok(out)
        })
    }

    /// Append a join (success or failure) to the history.
    pub fn record_join(&self, entry: &JoinHistoryEntry) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO join_history
                    (server_id, server_name, instance_id, mode, joined_at, outcome, detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.server_id.map(|id| id.to_string()),
                    entry.server_name,
                    entry.instance_id.map(|id| id.to_string()),
                    entry.mode.map(|mode| mode.as_str()),
                    datetime_to_millis(entry.joined_at),
                    entry.outcome,
                    entry.detail,
                ],
            )?;
            Ok(())
        })
    }

    /// Recent P2P sessions, used by the diagnostics panel and `session_history`.
    pub fn p2p_session_history(&self, limit: u32) -> AppResult<Vec<P2pSessionRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, peer_id, mode, local_port, started_at, ended_at,
                        bytes_up, bytes_down, rtt_ms, detail
                 FROM p2p_sessions ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })?;

            let mut out = Vec::new();
            for row in rows {
                let (
                    id,
                    role,
                    peer_id,
                    mode,
                    local_port,
                    started_at,
                    ended_at,
                    bytes_up,
                    bytes_down,
                    rtt_ms,
                    detail,
                ) = row?;
                out.push(P2pSessionRecord {
                    id: Uuid::parse_str(&id).unwrap_or(Uuid::nil()),
                    role,
                    peer_id,
                    mode: ConnectionMode::from_str_opt(&mode)
                        .unwrap_or(ConnectionMode::Dedicated),
                    local_port: local_port.and_then(|port| u16::try_from(port).ok()),
                    started_at: millis_to_datetime(started_at),
                    ended_at: ended_at.map(millis_to_datetime),
                    bytes_up: to_u64(bytes_up),
                    bytes_down: to_u64(bytes_down),
                    rtt_ms: rtt_ms.and_then(|value| u32::try_from(value).ok()),
                    detail,
                });
            }
            Ok(out)
        })
    }

    /// Record a P2P session as soon as it is established (`ended_at` is `None`
    /// until [`Database::finish_p2p_session`] runs).
    pub fn record_p2p_session(&self, record: &P2pSessionRecord) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO p2p_sessions
                    (id, role, peer_id, mode, local_port, started_at, ended_at,
                     bytes_up, bytes_down, rtt_ms, detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (id) DO UPDATE SET
                    ended_at = excluded.ended_at,
                    bytes_up = excluded.bytes_up,
                    bytes_down = excluded.bytes_down,
                    rtt_ms = excluded.rtt_ms,
                    detail = excluded.detail",
                params![
                    record.id.to_string(),
                    record.role,
                    record.peer_id,
                    record.mode.as_str(),
                    record.local_port.map(i64::from),
                    datetime_to_millis(record.started_at),
                    record.ended_at.map(datetime_to_millis),
                    record.bytes_up as i64,
                    record.bytes_down as i64,
                    record.rtt_ms.map(i64::from),
                    record.detail,
                ],
            )?;
            Ok(())
        })
    }

    /// Close a session row with its final counters.
    pub fn finish_p2p_session(
        &self,
        id: Uuid,
        ended_at: DateTime<Utc>,
        bytes_up: u64,
        bytes_down: u64,
        rtt_ms: Option<u32>,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE p2p_sessions
                 SET ended_at = ?2, bytes_up = ?3, bytes_down = ?4, rtt_ms = ?5
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    datetime_to_millis(ended_at),
                    bytes_up as i64,
                    bytes_down as i64,
                    rtt_ms.map(i64::from),
                ],
            )?;
            Ok(())
        })
    }

    /// Share of the most recent `limit` sessions that had to use the relay.
    ///
    /// This is the health metric behind the diagnostics panel: a rising ratio
    /// means hole punching is failing for more players.
    pub fn relay_fallback_ratio(&self, limit: u32) -> AppResult<f32> {
        self.with_conn(|conn| {
            let (total, relayed) = conn.query_row(
                "WITH recent AS (
                     SELECT mode FROM p2p_sessions ORDER BY started_at DESC LIMIT ?1
                 )
                 SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN mode = 'relay' THEN 1 ELSE 0 END), 0)
                 FROM recent",
                [limit as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )?;
            if total == 0 {
                Ok(0.0)
            } else {
                Ok((relayed as f32) / (total as f32))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::server::{
        ConnectionDescriptor, ConnectionMode, PeerEndpoint, PlayerCount, ServerListing,
        ServerOwner,
    };

    fn listing(name: &str) -> ServerListing {
        ServerListing {
            schema_version: crate::models::server::SERVER_LISTING_SCHEMA,
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "desc".into(),
            motd: "motd".into(),
            icon_base64: None,
            owner: ServerOwner {
                name: "host".into(),
                uuid: Uuid::new_v4(),
                provider: crate::models::account::AccountProvider::Offline,
            },
            game_version: "1.20.1".into(),
            loader: crate::models::instance::LoaderKind::Vanilla,
            loader_version: None,
            modpack: None,
            required_mod_ids: Vec::new(),
            players: PlayerCount { online: 1, max: 8 },
            connection: ConnectionDescriptor {
                mode: ConnectionMode::DirectP2p,
                peer_id: "peer".into(),
                public_key: "key".into(),
                endpoints: vec![PeerEndpoint::local("127.0.0.1:25565".parse().unwrap())],
                relay: None,
                session_token: "token".into(),
                protocol_version: 763,
            },
            region: None,
            tags: vec!["vanilla".into()],
            whitelist: Default::default(),
            password_protected: false,
            world_name: Some("world".into()),
            created_at: Utc::now(),
            heartbeat_at: Utc::now(),
            ttl_secs: 30,
            world_playtime_secs: 60,
        }
    }

    #[test]
    fn cached_listing_round_trips_through_json() {
        let db = Database::open_in_memory().expect("db");
        let original = listing("Survival");
        db.cache_server_listing(&original).expect("cache");

        let cached = db.cached_servers().expect("read");
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].listing.name, "Survival");
        assert_eq!(cached[0].listing.id, original.id);
        assert!(!cached[0].favorite);
    }

    #[test]
    fn caching_the_same_server_twice_does_not_duplicate_it() {
        let db = Database::open_in_memory().expect("db");
        let original = listing("Survival");
        db.cache_server_listing(&original).expect("first");
        db.cache_server_listing(&original).expect("second");
        assert_eq!(db.cached_servers().expect("read").len(), 1);
    }

    #[test]
    fn favourites_sort_first_and_report_whether_they_matched() {
        let db = Database::open_in_memory().expect("db");
        let plain = listing("Plain");
        let starred = listing("Starred");
        db.cache_server_listing(&plain).expect("cache plain");
        db.cache_server_listing(&starred).expect("cache starred");

        assert!(db.set_server_favorite(starred.id, true).expect("mark"));
        // An unknown id is not an error, just a no-op.
        assert!(!db.set_server_favorite(Uuid::new_v4(), true).expect("no-op"));

        let cached = db.cached_servers().expect("read");
        assert_eq!(cached[0].listing.name, "Starred");
        assert!(cached[0].favorite);
    }

    #[test]
    fn join_history_round_trips_mode_and_ids() {
        let db = Database::open_in_memory().expect("db");
        let server_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let entry = JoinHistoryEntry::success(
            Some(server_id),
            "Friends' World",
            Some(instance_id),
            Some(ConnectionMode::Relay),
            Some("relayed".into()),
        );
        db.record_join(&entry).expect("record");

        let joins = db.recent_joins(10).expect("read");
        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].server_id, Some(server_id));
        assert_eq!(joins[0].instance_id, Some(instance_id));
        assert_eq!(joins[0].mode, Some(ConnectionMode::Relay));
        assert_eq!(joins[0].outcome, "success");
    }

    #[test]
    fn failed_joins_are_recorded_without_a_server_id() {
        let db = Database::open_in_memory().expect("db");
        db.record_join(&JoinHistoryEntry::failure(
            None,
            "Expired Code",
            "failed",
            "code expired",
        ))
        .expect("record");

        let joins = db.recent_joins(10).expect("read");
        assert_eq!(joins[0].server_id, None);
        assert_eq!(joins[0].outcome, "failed");
        assert_eq!(joins[0].mode, None);
    }

    #[test]
    fn p2p_sessions_can_be_opened_and_closed() {
        let db = Database::open_in_memory().expect("db");
        let id = Uuid::new_v4();
        db.record_p2p_session(&P2pSessionRecord {
            id,
            role: "guest".into(),
            peer_id: "peer-1".into(),
            mode: ConnectionMode::DirectP2p,
            local_port: Some(25566),
            started_at: Utc::now(),
            ended_at: None,
            bytes_up: 0,
            bytes_down: 0,
            rtt_ms: None,
            detail: Some("protocol 763".into()),
        })
        .expect("open");

        db.finish_p2p_session(id, Utc::now(), 1024, 2048, Some(23))
            .expect("close");

        let history = db.p2p_session_history(10).expect("read");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, "guest");
        assert_eq!(history[0].local_port, Some(25566));
        assert_eq!(history[0].bytes_up, 1024);
        assert_eq!(history[0].bytes_down, 2048);
        assert_eq!(history[0].rtt_ms, Some(23));
        assert!(history[0].ended_at.is_some());
    }

    #[test]
    fn relay_ratio_counts_only_relayed_recent_sessions() {
        let db = Database::open_in_memory().expect("db");
        let base = Utc::now();

        for (index, mode) in [
            ConnectionMode::DirectP2p,
            ConnectionMode::Relay,
            ConnectionMode::DirectP2p,
            ConnectionMode::Relay,
        ]
        .into_iter()
        .enumerate()
        {
            db.record_p2p_session(&P2pSessionRecord {
                id: Uuid::new_v4(),
                role: "guest".into(),
                peer_id: format!("peer-{index}"),
                mode,
                local_port: None,
                started_at: base + chrono::Duration::seconds(index as i64),
                ended_at: None,
                bytes_up: 0,
                bytes_down: 0,
                rtt_ms: None,
                detail: None,
            })
            .expect("record");
        }

        let ratio = db.relay_fallback_ratio(10).expect("ratio");
        assert!((ratio - 0.5).abs() < f32::EPSILON);

        // An empty window is 0.0 rather than a division by zero.
        assert_eq!(db.relay_fallback_ratio(0).expect("ratio"), 0.0);
    }
}
