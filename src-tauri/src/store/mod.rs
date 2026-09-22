//! Local SQLite persistence (app settings, cached metadata, instance index).
//!
//! Design notes
//! * `rusqlite` is synchronous; every public method therefore either runs
//!   directly inside a short critical section or is wrapped in
//!   `spawn_blocking` so a long query can never stall the async runtime.
//! * Timestamps are stored as **unix milliseconds** (`INTEGER`), never as
//!   strings: it keeps ordering/indexing cheap and avoids timezone drift.
//! * The shared `Connection` sits behind a `parking_lot::Mutex`. Writes are
//!   small and rare compared to downloads, so a single connection with WAL mode
//!   is the right trade-off. If this ever becomes a bottleneck the module can
//!   move to `r2d2_sqlite` without touching call sites.

pub mod accounts;
pub mod instances;
pub mod migrations;
pub mod servers;
pub mod settings;

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// Thin, thread-safe wrapper around a single SQLite connection.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database file at `path`.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(|err| {
            AppError::Database(format!("cannot open {}: {err}", path.display()))
        })?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.configure()?;
        db.migrate()?;
        Ok(db)
    }

    /// In-memory database, used by unit tests.
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|err| AppError::Database(format!("cannot open in-memory db: {err}")))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.configure()?;
        db.migrate()?;
        Ok(db)
    }

    /// Pragmas tuned for a desktop app: WAL for concurrent reads, foreign keys
    /// on, and a busy timeout so a concurrent writer waits instead of erroring.
    fn configure(&self) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA temp_store = MEMORY;",
            )?;
            Ok(())
        })
    }

    /// Run the schema migrations in `migrations.rs`.
    pub fn migrate(&self) -> AppResult<()> {
        migrations::apply(self)
    }

    /// Run `f` inside the connection lock.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let conn = self.conn.lock();
        f(&conn)
    }

    /// Run `f` inside the lock but on a blocking thread pool, for queries that
    /// may take longer than a frame (sizes, batch upserts).
    pub async fn with_conn_async<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> AppResult<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock();
            f(&guard)
        })
        .await?
    }

    /// Wrap an already-running transaction.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }
}

/// Convert a stored unix-millis integer into a `DateTime<Utc>`.
pub fn millis_to_datetime(millis: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(millis).unwrap_or_else(chrono::Utc::now)
}

/// Inverse of [`millis_to_datetime`].
pub fn datetime_to_millis(value: chrono::DateTime<chrono::Utc>) -> i64 {
    value.timestamp_millis()
}

/// Map an optional millis column.
pub fn opt_millis_to_datetime(millis: Option<i64>) -> Option<chrono::DateTime<chrono::Utc>> {
    millis.and_then(chrono::DateTime::from_timestamp_millis)
}

/// SQLite has no boolean type; normalize the `0/1` round trip in one place.
pub fn int_to_bool(value: i64) -> bool {
    value != 0
}

pub fn bool_to_int(value: bool) -> i64 {
    i64::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates_in_memory() {
        let db = Database::open_in_memory().expect("in-memory db opens");
        let version: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
            })
            .expect("user_version readable");
        assert_eq!(version, migrations::SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let db = Database::open_in_memory().expect("db");
        db.migrate().expect("second migrate succeeds");
        let tables: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .expect("table count");
        assert!(tables >= 5);
    }

    #[test]
    fn bool_round_trip() {
        assert!(int_to_bool(bool_to_int(true)));
        assert!(!int_to_bool(bool_to_int(false)));
    }
}
