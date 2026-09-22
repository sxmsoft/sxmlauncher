//! Offline ("guest") accounts.
//!
//! These are the accounts players use for LAN worlds, testing mods, and servers
//! running in `online-mode=false`. The UUID is derived the same way the vanilla
//! server derives it, so a world that has seen this player before keeps the same
//! player data:
//!
//! ```text
//! UUID = MD5("OfflinePlayer:" + name)   (RFC 4122 version 3, name-based)
//! ```

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::account::{AccountProvider, LaunchIdentity, UserAccount};

/// Vanilla's rule: 3-16 characters, `[A-Za-z0-9_]` only.
pub const MIN_USERNAME_LEN: usize = 3;
pub const MAX_USERNAME_LEN: usize = 16;

/// Deterministic offline UUID for a username.
pub fn offline_uuid(username: &str) -> Uuid {
    // `new_v3` is MD5-based, matching Java's `UUID.nameUUIDFromBytes`.
    Uuid::new_v3(&Uuid::NAMESPACE_OID, format!("OfflinePlayer:{username}").as_bytes())
}

/// Validate a nickname against the vanilla rules.
pub fn validate_username(username: &str) -> AppResult<()> {
    let length = username.chars().count();
    if !(MIN_USERNAME_LEN..=MAX_USERNAME_LEN).contains(&length) {
        return Err(AppError::Account(format!(
            "usernames must be between {MIN_USERNAME_LEN} and {MAX_USERNAME_LEN} characters"
        )));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::Account(
            "usernames may only contain letters, numbers and underscores".to_string(),
        ));
    }
    Ok(())
}

/// Build an offline account (no vault entry, no network session).
pub fn create_offline_account(username: &str) -> AppResult<UserAccount> {
    let username = username.trim();
    validate_username(username)?;

    let mut account =
        UserAccount::new(AccountProvider::Offline, username.to_string(), offline_uuid(username));
    account.has_stored_credentials = false;
    // Offline sessions never expire; there is nothing to refresh.
    account.expires_at = None;
    account.skin.skin_url = None;
    Ok(account)
}

/// Launch identity for an offline session.
pub fn offline_identity(username: &str) -> LaunchIdentity {
    LaunchIdentity::offline(username, offline_uuid(username))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_is_stable_and_matches_vanilla() {
        let first = offline_uuid("Notch");
        let second = offline_uuid("Notch");
        assert_eq!(first, second);
        // Version 3 (MD5 name-based) as vanilla expects.
        assert_eq!(first.get_version_num(), 3);
        assert_ne!(offline_uuid("Notch"), offline_uuid("notch"));
    }

    #[test]
    fn username_validation_enforces_vanilla_rules() {
        assert!(validate_username("Steve_99").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username(&"a".repeat(17)).is_err());
        assert!(validate_username("bad name").is_err());
        assert!(validate_username("emoji🎉").is_err());
    }

    #[test]
    fn created_account_is_offline_and_unexpiring() {
        let account = create_offline_account("  Guest  ").expect("account");
        assert_eq!(account.username, "Guest");
        assert_eq!(account.provider, AccountProvider::Offline);
        assert!(!account.has_stored_credentials);
        assert!(account.expires_at.is_none());
    }

    #[test]
    fn offline_identity_uses_the_undashed_uuid_and_never_claims_online() {
        let identity = offline_identity("Guest");
        assert!(identity.offline);
        assert_eq!(identity.uuid.len(), 32);
        assert!(!identity.uuid.contains('-'));
    }
}
