//! Schema definition and versioned migrations.
//!
//! `PRAGMA user_version` is the schema version. To evolve the schema, append a
//! new `Migration` with the next version; never edit a shipped migration.

use crate::error::AppResult;
use crate::store::Database;

/// Current schema version — must equal the highest migration version.
pub const SCHEMA_VERSION: i64 = 2;

/// One forward migration.
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Ordered list of migrations. Append only.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial_schema",
    sql: r#"
-- Accounts ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS accounts (
    id                      TEXT    PRIMARY KEY,
    provider                TEXT    NOT NULL,
    username                TEXT    NOT NULL,
    uuid                    TEXT    NOT NULL,
    skin_model              TEXT    NOT NULL DEFAULT 'classic',
    skin_url                TEXT,
    cape_url                TEXT,
    has_stored_credentials  INTEGER NOT NULL DEFAULT 0,
    expires_at              INTEGER,
    created_at              INTEGER NOT NULL,
    last_used_at            INTEGER NOT NULL
);
-- One row per (provider, minecraft uuid): re-login updates instead of duplicating.
CREATE UNIQUE INDEX IF NOT EXISTS accounts_provider_uuid
    ON accounts (provider, uuid);

-- Settings ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS app_settings (
    key         TEXT    PRIMARY KEY,
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Instances --------------------------------------------------------------
CREATE TABLE IF NOT EXISTS instances (
    id                    TEXT    PRIMARY KEY,
    name                  TEXT    NOT NULL,
    description           TEXT    NOT NULL DEFAULT '',
    icon                  TEXT,
    game_version          TEXT    NOT NULL,
    loader                TEXT    NOT NULL,
    loader_version        TEXT,
    config_json           TEXT    NOT NULL,
    status                TEXT    NOT NULL DEFAULT 'not_installed',
    last_played_at        INTEGER,
    total_playtime_secs   INTEGER NOT NULL DEFAULT 0,
    launch_count          INTEGER NOT NULL DEFAULT 0,
    mod_count             INTEGER NOT NULL DEFAULT 0,
    size_bytes            INTEGER NOT NULL DEFAULT 0,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS instances_updated_idx ON instances (updated_at DESC);

-- Installed mods (per instance) -----------------------------------------
CREATE TABLE IF NOT EXISTS installed_mods (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id  TEXT    NOT NULL,
    source       TEXT    NOT NULL,
    project_id   TEXT    NOT NULL,
    version_id   TEXT    NOT NULL,
    title        TEXT    NOT NULL DEFAULT '',
    file_name    TEXT    NOT NULL,
    sha1         TEXT,
    enabled      INTEGER NOT NULL DEFAULT 1,
    installed_at INTEGER NOT NULL,
    UNIQUE (instance_id, project_id)
);
CREATE INDEX IF NOT EXISTS installed_mods_instance_idx
    ON installed_mods (instance_id);

-- Shared metadata cache (Modrinth/CurseForge projects, version manifests) --
CREATE TABLE IF NOT EXISTS metadata_cache (
    cache_key   TEXT    PRIMARY KEY,
    payload     TEXT    NOT NULL,
    expires_at  INTEGER NOT NULL
);

-- Server browser cache ---------------------------------------------------
CREATE TABLE IF NOT EXISTS server_cache (
    id            TEXT    PRIMARY KEY,
    listing_json  TEXT    NOT NULL,
    favorite      INTEGER NOT NULL DEFAULT 0,
    last_seen_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS server_cache_seen_idx ON server_cache (last_seen_at DESC);

CREATE TABLE IF NOT EXISTS join_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id    TEXT,
    server_name  TEXT NOT NULL DEFAULT '',
    instance_id  TEXT,
    mode         TEXT,
    joined_at    INTEGER NOT NULL,
    outcome      TEXT    NOT NULL DEFAULT 'unknown',
    detail       TEXT
);

-- P2P session log (diagnostics for hole punching) -------------------------
CREATE TABLE IF NOT EXISTS p2p_sessions (
    id            TEXT    PRIMARY KEY,
    role          TEXT    NOT NULL,
    peer_id       TEXT    NOT NULL,
    mode          TEXT    NOT NULL,
    local_port    INTEGER,
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER,
    bytes_up      INTEGER NOT NULL DEFAULT 0,
    bytes_down    INTEGER NOT NULL DEFAULT 0,
    rtt_ms        INTEGER,
    detail        TEXT
);
"#,
},
// Custom modpacks (v2): a named set of pinned registry projects the player
// assembles themselves, CurseForge-style. `custom_pack_items` is the
// manifest; instances created from a pack reference it by id.
Migration {
    version: 2,
    name: "custom_modpacks",
    sql: r#"
CREATE TABLE IF NOT EXISTS custom_packs (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    description  TEXT,
    icon_url     TEXT,
    game_version TEXT,
    loader       TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS custom_pack_items (
    pack_id    TEXT NOT NULL REFERENCES custom_packs(id) ON DELETE CASCADE,
    source     TEXT NOT NULL,
    project_id TEXT NOT NULL,
    version_id TEXT NOT NULL DEFAULT '',
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (pack_id, project_id)
);
"#,
}];

/// Apply every migration newer than the database's current `user_version`.
pub fn apply(db: &Database) -> AppResult<()> {
    let current: i64 = db.with_conn(|conn| {
        Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
    })?;

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        db.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(migration.sql)?;
            // `user_version` cannot be parameterized, and the value is a
            // compile-time constant from MIGRATIONS, so formatting is safe.
            tx.execute_batch(&format!("PRAGMA user_version = {}", migration.version))?;
            tx.commit()?;
            Ok(())
        })?;
        tracing_info(migration.version, migration.name);
    }
    Ok(())
}

/// Kept as a tiny indirection so the store module has no logging dependency.
fn tracing_info(version: i64, name: &str) {
    #[cfg(debug_assertions)]
    eprintln!("[store] applied migration {version} ({name})");
    #[cfg(not(debug_assertions))]
    let _ = (version, name);
}
