//! Settings storage + the shared metadata cache (Modrinth/CurseForge payloads
//! and the Mojang version manifest, so the UI opens instantly and offline).

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::params;

use crate::config::AppSettings;
use crate::error::AppResult;
use crate::store::{datetime_to_millis, Database};

/// Key holding the serialized [`AppSettings`] blob.
pub const SETTINGS_BLOB_KEY: &str = "app.settings";

impl Database {
    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT value FROM app_settings WHERE key = ?1")?;
            let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
            match rows.next() {
                Some(value) => Ok(Some(value?)),
                None => Ok(None),
            }
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                                                updated_at = excluded.updated_at",
                params![key, value, datetime_to_millis(Utc::now())],
            )?;
            Ok(())
        })
    }

    pub fn delete_setting(&self, key: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
            Ok(())
        })
    }

    pub fn all_settings(&self) -> AppResult<HashMap<String, String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT key, value FROM app_settings")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut map = HashMap::new();
            for row in rows {
                let (key, value) = row?;
                map.insert(key, value);
            }
            Ok(map)
        })
    }

    /// Load settings, falling back to defaults for missing/corrupt blobs.
    ///
    /// A malformed blob is intentionally *not* an error: a bad settings row must
    /// never prevent the launcher from starting. The corrupt value is preserved
    /// under a `.bak` key so the user can recover it.
    pub fn load_app_settings(&self) -> AppResult<AppSettings> {
        let Some(raw) = self.get_setting(SETTINGS_BLOB_KEY)? else {
            return Ok(AppSettings::default());
        };
        match serde_json::from_str::<AppSettings>(&raw) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                self.set_setting(&format!("{SETTINGS_BLOB_KEY}.bak"), &raw)?;
                Ok(AppSettings::default())
            }
        }
    }

    pub fn save_app_settings(&self, settings: &AppSettings) -> AppResult<()> {
        let blob = serde_json::to_string(settings)?;
        self.set_setting(SETTINGS_BLOB_KEY, &blob)
    }

    // --- metadata cache --------------------------------------------------

    /// Read a cached payload if it has not expired.
    pub fn cache_get(&self, key: &str) -> AppResult<Option<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT payload, expires_at FROM metadata_cache WHERE cache_key = ?1",
            )?;
            let mut rows = stmt.query_map(params![key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            match rows.next() {
                Some(row) => {
                    let (payload, expires_at) = row?;
                    if expires_at <= datetime_to_millis(Utc::now()) {
                        return Ok(None);
                    }
                    Ok(Some(payload))
                }
                None => Ok(None),
            }
        })
    }

    /// Upsert a cached payload. `ttl_secs == 0` means "never expires"; a
    /// negative TTL stores an already-expired entry, which is how callers seed
    /// a value they want [`Database::cache_purge_expired`] to clean up.
    pub fn cache_put(&self, key: &str, payload: &str, ttl_secs: i64) -> AppResult<()> {
        let expires_at = if ttl_secs == 0 {
            i64::MAX
        } else {
            datetime_to_millis(Utc::now() + chrono::Duration::seconds(ttl_secs))
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO metadata_cache (cache_key, payload, expires_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (cache_key) DO UPDATE SET payload = excluded.payload,
                                                       expires_at = excluded.expires_at",
                params![key, payload, expires_at],
            )?;
            Ok(())
        })
    }

    pub fn cache_purge_expired(&self) -> AppResult<usize> {
        self.with_conn(|conn| {
            let removed = conn.execute(
                "DELETE FROM metadata_cache WHERE expires_at <= ?1",
                params![datetime_to_millis(Utc::now())],
            )?;
            Ok(removed)
        })
    }

    pub fn cache_clear(&self) -> AppResult<usize> {
        self.with_conn(|conn| {
            let removed = conn.execute("DELETE FROM metadata_cache", [])?;
            Ok(removed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let db = Database::open_in_memory().expect("db");
        let mut settings = AppSettings::default();
        settings.max_concurrent_downloads = 20;
        db.save_app_settings(&settings).expect("save");

        let loaded = db.load_app_settings().expect("load");
        assert_eq!(loaded.max_concurrent_downloads, 20);
    }

    #[test]
    fn corrupt_settings_blob_falls_back_and_is_backed_up() {
        let db = Database::open_in_memory().expect("db");
        db.set_setting(SETTINGS_BLOB_KEY, "{ not json")
            .expect("seed corrupt blob");

        let loaded = db.load_app_settings().expect("load");
        assert_eq!(loaded.max_concurrent_downloads, AppSettings::default().max_concurrent_downloads);
        assert!(db
            .get_setting(&format!("{SETTINGS_BLOB_KEY}.bak"))
            .expect("bak readable")
            .is_some());
    }

    #[test]
    fn expired_cache_entries_are_not_returned_but_survive_purge() {
        let db = Database::open_in_memory().expect("db");
        db.cache_put("modrinth:project:sodium", "{\"title\":\"Sodium\"}", -1)
            .expect("put expired");
        assert!(db.cache_get("modrinth:project:sodium").expect("get").is_none());

        db.cache_put("modrinth:search:sodium", "[]", 3600)
            .expect("put fresh");
        assert!(db.cache_get("modrinth:search:sodium").expect("get").is_some());

        let purged = db.cache_purge_expired().expect("purge");
        assert_eq!(purged, 1);
    }

    #[test]
    fn ttl_zero_means_never_expires() {
        let db = Database::open_in_memory().expect("db");
        db.cache_put("mojang:version_manifest", "{}", 0)
            .expect("put");
        assert!(db
            .cache_get("mojang:version_manifest")
            .expect("get")
            .is_some());
    }
}
