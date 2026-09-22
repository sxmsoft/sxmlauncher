//! Secure credential storage.
//!
//! Refresh tokens are the most sensitive thing SXMLauncher touches: a stolen
//! refresh token can impersonate the player for months. They therefore live in
//! the OS credential vault (Keychain / Credential Manager / Secret Service) and
//! never in SQLite or a JSON file.
//!
//! [`KeyringVault`] falls back to [`MemoryVault`] only when the platform store
//! is unavailable (headless CI, minimal Linux containers) — and in that case
//! the account is marked `has_stored_credentials = false` so the UI prompts for
//! a fresh sign-in instead of silently losing the session.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::account::{AccountProvider, TokenSet};

/// Keyring service name; entries are grouped under it in every OS vault.
pub const DEFAULT_VAULT_SERVICE: &str = "dev.sxmlauncher.app";

/// Vault key for an account: stable across renames and re-logins.
pub fn account_key(provider: AccountProvider, uuid: Uuid) -> String {
    format!("{provider}:{uuid}")
}

/// Store for provider sessions.
#[async_trait]
pub trait CredentialVault: Send + Sync + 'static {
    async fn store(&self, key: &str, tokens: &TokenSet) -> AppResult<()>;
    async fn load(&self, key: &str) -> AppResult<Option<TokenSet>>;
    async fn delete(&self, key: &str) -> AppResult<()>;
    /// Human-readable backend name, surfaced in Settings > Accounts.
    fn backend(&self) -> &'static str;
}

/// OS-native vault backed by the `keyring` crate.
#[derive(Debug, Clone)]
pub struct KeyringVault {
    service: String,
}

impl Default for KeyringVault {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringVault {
    pub fn new() -> Self {
        Self {
            service: DEFAULT_VAULT_SERVICE.to_string(),
        }
    }

    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// `keyring` is a blocking API (Windows DPAPI, dbus, Security framework), so
    /// every call hops onto the blocking pool.
    async fn blocking<T, F>(f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> AppResult<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(f).await?
    }
}

#[async_trait]
impl CredentialVault for KeyringVault {
    async fn store(&self, key: &str, tokens: &TokenSet) -> AppResult<()> {
        let service = self.service.clone();
        let key = key.to_string();
        let secret = serde_json::to_string(tokens)?;
        Self::blocking(move || {
            let entry = keyring::Entry::new(&service, &key)?;
            entry.set_password(&secret)?;
            Ok(())
        })
        .await
    }

    async fn load(&self, key: &str) -> AppResult<Option<TokenSet>> {
        let service = self.service.clone();
        let key = key.to_string();
        Self::blocking(move || {
            let entry = keyring::Entry::new(&service, &key)?;
            match entry.get_password() {
                Ok(secret) => Ok(Some(serde_json::from_str(&secret)?)),
                // Missing entry is a normal "not signed in" state, not an error.
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(err) => Err(AppError::from(err)),
            }
        })
        .await
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let service = self.service.clone();
        let key = key.to_string();
        Self::blocking(move || {
            let entry = keyring::Entry::new(&service, &key)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(AppError::from(err)),
            }
        })
        .await
    }

    fn backend(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            "Windows Credential Manager"
        } else if cfg!(target_os = "macos") {
            "macOS Keychain"
        } else {
            "Secret Service / keyutils"
        }
    }
}

/// Process-lifetime vault used for tests and as a degraded fallback.
#[derive(Debug, Clone, Default)]
pub struct MemoryVault {
    entries: Arc<Mutex<HashMap<String, TokenSet>>>,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

#[async_trait]
impl CredentialVault for MemoryVault {
    async fn store(&self, key: &str, tokens: &TokenSet) -> AppResult<()> {
        self.entries.lock().insert(key.to_string(), tokens.clone());
        Ok(())
    }

    async fn load(&self, key: &str) -> AppResult<Option<TokenSet>> {
        Ok(self.entries.lock().get(key).cloned())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.entries.lock().remove(key);
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "in-memory (not persisted)"
    }
}

/// Probe the platform vault once at startup.
///
/// Returns the keyring vault when a round-trip write/read/delete works,
/// otherwise the in-memory vault so the app still runs.
pub async fn select_vault() -> Arc<dyn CredentialVault> {
    let probe = KeyringVault::new();
    let key = "sxmlauncher:probe";
    let tokens = TokenSet::new(
        "probe",
        None,
        chrono::Utc::now() + chrono::Duration::seconds(30),
    );

    if probe.store(key, &tokens).await.is_ok() && probe.load(key).await.is_ok() {
        let _ = probe.delete(key).await;
        Arc::new(probe)
    } else {
        eprintln!(
            "[auth] OS credential vault unavailable; falling back to an in-memory vault. \
             Sessions will need to be re-authenticated after restart."
        );
        Arc::new(MemoryVault::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_vault_round_trips_tokens() {
        let vault = MemoryVault::new();
        let key = account_key(AccountProvider::Offline, Uuid::new_v4());
        assert!(vault.load(&key).await.expect("load").is_none());

        let tokens = TokenSet::new("access", Some("refresh".into()), chrono::Utc::now());
        vault.store(&key, &tokens).await.expect("store");
        assert_eq!(vault.len(), 1);

        let loaded = vault.load(&key).await.expect("load").expect("present");
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh"));

        vault.delete(&key).await.expect("delete");
        assert!(vault.is_empty());
    }

    #[test]
    fn account_keys_are_provider_scoped() {
        let uuid = Uuid::new_v4();
        assert_ne!(
            account_key(AccountProvider::Microsoft, uuid),
            account_key(AccountProvider::ElyBy, uuid)
        );
    }
}
