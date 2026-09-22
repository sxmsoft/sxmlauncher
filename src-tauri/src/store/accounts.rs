//! Account rows in SQLite.
//!
//! Only *identity* lives here: provider, username, Minecraft UUID, skin
//! metadata and the bookkeeping flags the UI shows. Tokens are never stored in
//! this table — they belong to the OS credential vault (`auth::vault`), so a
//! leaked `sxmlauncher.db` never leaks a session.

use chrono::{DateTime, Utc};
use rusqlite::{params, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::account::{AccountProvider, SkinModel, SkinProfile, UserAccount};
use crate::store::{bool_to_int, datetime_to_millis, int_to_bool, millis_to_datetime, Database};

/// Column list shared by every read so the row mapper stays in sync with the
/// `SELECT` statements below.
const ACCOUNT_COLUMNS: &str = "id, provider, username, uuid, skin_model, skin_url, cape_url, \
                               has_stored_credentials, expires_at, created_at, last_used_at";

/// Raw SQLite values for one account row, before they are parsed into a
/// [`UserAccount`]. Parsing happens outside the `rusqlite` closure because a
/// malformed UUID is an [`AppError`], not a `rusqlite::Error`.
struct RawAccount {
    id: String,
    provider: String,
    username: String,
    uuid: String,
    skin_model: String,
    skin_url: Option<String>,
    cape_url: Option<String>,
    has_stored_credentials: i64,
    expires_at: Option<i64>,
    created_at: i64,
    last_used_at: i64,
}

impl RawAccount {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            provider: row.get(1)?,
            username: row.get(2)?,
            uuid: row.get(3)?,
            skin_model: row.get(4)?,
            skin_url: row.get(5)?,
            cape_url: row.get(6)?,
            has_stored_credentials: row.get(7)?,
            expires_at: row.get(8)?,
            created_at: row.get(9)?,
            last_used_at: row.get(10)?,
        })
    }

    fn into_account(self) -> AppResult<UserAccount> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|err| AppError::Account(format!("stored account id is invalid: {err}")))?;
        let provider = AccountProvider::from_str_opt(&self.provider).ok_or_else(|| {
            AppError::Account(format!("stored account provider is unknown: {}", self.provider))
        })?;
        let uuid = Uuid::parse_str(&self.uuid).map_err(|err| {
            AppError::Account(format!("stored Minecraft uuid is invalid: {err}"))
        })?;

        Ok(UserAccount {
            id,
            provider,
            username: self.username,
            uuid,
            skin: SkinProfile {
                model: SkinModel::from_str_opt(&self.skin_model).unwrap_or_default(),
                skin_url: self.skin_url,
                cape_url: self.cape_url,
            },
            has_stored_credentials: int_to_bool(self.has_stored_credentials),
            expires_at: self.expires_at.and_then(millis_to_datetime_opt),
            created_at: millis_to_datetime(self.created_at),
            last_used_at: millis_to_datetime(self.last_used_at),
        })
    }
}

/// `Option<i64>` -> `Option<DateTime<Utc>>` without the `?` noise in the mapper.
fn millis_to_datetime_opt(millis: i64) -> Option<DateTime<Utc>> {
    chrono::DateTime::from_timestamp_millis(millis)
}

impl Database {
    /// Every account, most recently used first (the account picker order).
    pub fn list_accounts(&self) -> AppResult<Vec<UserAccount>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY last_used_at DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], RawAccount::read)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?.into_account()?);
            }
            Ok(out)
        })
    }

    pub fn get_account(&self, id: Uuid) -> AppResult<Option<UserAccount>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1");
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query_map(params![id.to_string()], RawAccount::read)?;
            match rows.next() {
                Some(row) => Ok(Some(row?.into_account()?)),
                None => Ok(None),
            }
        })
    }

    /// Look an account up by its provider identity, so signing in again reuses
    /// the existing row (and therefore keeps instance/history references).
    pub fn find_account(
        &self,
        provider: AccountProvider,
        uuid: Uuid,
    ) -> AppResult<Option<UserAccount>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE provider = ?1 AND uuid = ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query_map(params![provider.as_str(), uuid.to_string()], RawAccount::read)?;
            match rows.next() {
                Some(row) => Ok(Some(row?.into_account()?)),
                None => Ok(None),
            }
        })
    }

    /// Insert or update an account row.
    pub fn upsert_account(&self, account: &UserAccount) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (
                    id, provider, username, uuid, skin_model, skin_url, cape_url,
                    has_stored_credentials, expires_at, created_at, last_used_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (id) DO UPDATE SET
                    provider = excluded.provider,
                    username = excluded.username,
                    uuid = excluded.uuid,
                    skin_model = excluded.skin_model,
                    skin_url = excluded.skin_url,
                    cape_url = excluded.cape_url,
                    has_stored_credentials = excluded.has_stored_credentials,
                    expires_at = excluded.expires_at,
                    last_used_at = excluded.last_used_at",
                params![
                    account.id.to_string(),
                    account.provider.as_str(),
                    account.username,
                    account.uuid.to_string(),
                    account.skin.model.as_str(),
                    account.skin.skin_url,
                    account.skin.cape_url,
                    bool_to_int(account.has_stored_credentials),
                    account.expires_at.map(datetime_to_millis),
                    datetime_to_millis(account.created_at),
                    datetime_to_millis(account.last_used_at),
                ],
            )?;
            Ok(())
        })
    }

    /// Track when the stored session stops being valid (used for the account
    /// badge and to decide whether a refresh is due on startup).
    pub fn set_account_expiry(
        &self,
        id: Uuid,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET expires_at = ?2 WHERE id = ?1",
                params![id.to_string(), expires_at.map(datetime_to_millis)],
            )?;
            Ok(())
        })
    }

    /// Flip the "vault holds a refresh token" flag without touching tokens.
    pub fn set_credentials_stored(&self, id: Uuid, stored: bool) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET has_stored_credentials = ?2 WHERE id = ?1",
                params![id.to_string(), bool_to_int(stored)],
            )?;
            Ok(())
        })
    }

    pub fn delete_account(&self, id: Uuid) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", params![id.to_string()])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(provider: AccountProvider, username: &str, uuid: Uuid) -> UserAccount {
        UserAccount::new(provider, username.to_string(), uuid)
    }

    #[test]
    fn upsert_then_read_round_trips_every_field() {
        let db = Database::open_in_memory().expect("db");
        let mut account = sample(AccountProvider::Microsoft, "Steve", Uuid::new_v4());
        account.skin.model = SkinModel::Slim;
        account.skin.skin_url = Some("https://example.invalid/skin.png".into());
        account.skin.cape_url = Some("https://example.invalid/cape.png".into());
        account.has_stored_credentials = true;
        account.expires_at = Some(Utc::now() + chrono::Duration::hours(1));

        db.upsert_account(&account).expect("insert");
        let loaded = db
            .get_account(account.id)
            .expect("read")
            .expect("row exists");

        assert_eq!(loaded.id, account.id);
        assert_eq!(loaded.provider, AccountProvider::Microsoft);
        assert_eq!(loaded.username, "Steve");
        assert_eq!(loaded.uuid, account.uuid);
        assert_eq!(loaded.skin.model, SkinModel::Slim);
        assert_eq!(loaded.skin.skin_url.as_deref(), Some("https://example.invalid/skin.png"));
        assert!(loaded.has_stored_credentials);
        assert!(loaded.expires_at.is_some());
    }

    #[test]
    fn upserting_the_same_row_updates_in_place() {
        let db = Database::open_in_memory().expect("db");
        let mut account = sample(AccountProvider::Offline, "Guest", Uuid::new_v4());
        db.upsert_account(&account).expect("insert");

        account.username = "Renamed".into();
        account.last_used_at = Utc::now() + chrono::Duration::seconds(5);
        db.upsert_account(&account).expect("update");

        let all = db.list_accounts().expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].username, "Renamed");
    }

    #[test]
    fn duplicate_provider_uuid_is_rejected_by_the_unique_index() {
        let db = Database::open_in_memory().expect("db");
        let uuid = Uuid::new_v4();
        db.upsert_account(&sample(AccountProvider::ElyBy, "One", uuid))
            .expect("first");

        // A *different* row id with the same (provider, uuid) must not create a
        // second identity — the unique index is what keeps history consistent.
        let duplicate = sample(AccountProvider::ElyBy, "Two", uuid);
        assert!(db.upsert_account(&duplicate).is_err());
    }

    #[test]
    fn find_account_matches_on_provider_and_uuid() {
        let db = Database::open_in_memory().expect("db");
        let uuid = Uuid::new_v4();
        let account = sample(AccountProvider::ElyBy, "ElyUser", uuid);
        db.upsert_account(&account).expect("insert");

        let found = db
            .find_account(AccountProvider::ElyBy, uuid)
            .expect("find")
            .expect("exists");
        assert_eq!(found.id, account.id);

        // Same uuid, different provider is a different identity.
        assert!(db
            .find_account(AccountProvider::Microsoft, uuid)
            .expect("find")
            .is_none());
    }

    #[test]
    fn flags_and_expiry_can_be_updated_without_a_full_upsert() {
        let db = Database::open_in_memory().expect("db");
        let account = sample(AccountProvider::Microsoft, "Steve", Uuid::new_v4());
        db.upsert_account(&account).expect("insert");

        let expiry = Utc::now() + chrono::Duration::minutes(30);
        db.set_account_expiry(account.id, Some(expiry)).expect("expiry");
        db.set_credentials_stored(account.id, true).expect("flag");

        let loaded = db.get_account(account.id).expect("read").expect("row");
        assert!(loaded.has_stored_credentials);
        assert_eq!(
            loaded.expires_at.map(|value| value.timestamp_millis()),
            Some(expiry.timestamp_millis())
        );

        db.set_account_expiry(account.id, None).expect("clear expiry");
        db.set_credentials_stored(account.id, false).expect("clear flag");
        let loaded = db.get_account(account.id).expect("read").expect("row");
        assert!(!loaded.has_stored_credentials);
        assert!(loaded.expires_at.is_none());
    }

    #[test]
    fn delete_removes_the_row() {
        let db = Database::open_in_memory().expect("db");
        let account = sample(AccountProvider::Offline, "Gone", Uuid::new_v4());
        db.upsert_account(&account).expect("insert");
        db.delete_account(account.id).expect("delete");

        assert!(db.get_account(account.id).expect("read").is_none());
        assert!(db.list_accounts().expect("list").is_empty());
    }

    #[test]
    fn list_orders_by_last_used_descending() {
        let db = Database::open_in_memory().expect("db");
        let mut old = sample(AccountProvider::Offline, "Old", Uuid::new_v4());
        old.last_used_at = Utc::now() - chrono::Duration::days(2);
        let fresh = sample(AccountProvider::Offline, "Fresh", Uuid::new_v4());

        db.upsert_account(&old).expect("insert old");
        db.upsert_account(&fresh).expect("insert fresh");

        let ordered = db.list_accounts().expect("list");
        assert_eq!(ordered[0].username, "Fresh");
        assert_eq!(ordered[1].username, "Old");
    }
}
