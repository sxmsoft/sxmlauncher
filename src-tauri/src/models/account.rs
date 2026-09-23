//! Accounts, sessions and the launch-time identity handed to the JVM.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which identity provider backs an account.
///
/// Providers share the same downstream shape (username + UUID + access token)
/// but differ completely in how that material is obtained and refreshed, so
/// the provider tag travels with the account everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountProvider {
    /// Microsoft/Xbox Live OAuth2 + PKCE, Mojang services.
    Microsoft,
    /// Ely.by (Authlib-compatible) account, adds custom skins/capes.
    ElyBy,
    /// Local-only profile, `OfflinePlayer:<name>` UUID, no network session.
    Offline,
    /// sx.acc account API (`{BASE}/v1`) plus authlib at `{BASE}/authlib/`.
    SxAcc,
}

impl AccountProvider {
    /// Stable string used in keyring entries and the SQLite `accounts` table.
    pub fn as_str(self) -> &'static str {
        match self {
            AccountProvider::Microsoft => "microsoft",
            AccountProvider::ElyBy => "ely_by",
            AccountProvider::Offline => "offline",
            AccountProvider::SxAcc => "sx_acc",
        }
    }

    pub fn from_str_opt(raw: &str) -> Option<Self> {
        match raw {
            "microsoft" => Some(AccountProvider::Microsoft),
            "ely_by" => Some(AccountProvider::ElyBy),
            "offline" => Some(AccountProvider::Offline),
            "sx_acc" => Some(AccountProvider::SxAcc),
            _ => None,
        }
    }

    /// Provider that has a live network session (i.e. can be refreshed).
    pub fn is_online(self) -> bool {
        !matches!(self, AccountProvider::Offline)
    }
}

impl std::fmt::Display for AccountProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Skin/cape metadata resolved from the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinModel {
    Classic,
    Slim,
}

impl Default for SkinModel {
    fn default() -> Self {
        SkinModel::Classic
    }
}

impl SkinModel {
    /// Stable string used in the SQLite `skin_model` column.
    pub fn as_str(self) -> &'static str {
        match self {
            SkinModel::Classic => "classic",
            SkinModel::Slim => "slim",
        }
    }

    /// Parse the persisted form; `None` for anything unrecognised so a future
    /// model name never breaks an older launcher build.
    pub fn from_str_opt(raw: &str) -> Option<Self> {
        match raw {
            "classic" => Some(SkinModel::Classic),
            "slim" => Some(SkinModel::Slim),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinProfile {
    pub model: SkinModel,
    pub skin_url: Option<String>,
    pub cape_url: Option<String>,
}

impl Default for SkinProfile {
    fn default() -> Self {
        Self {
            model: SkinModel::Classic,
            skin_url: None,
            cape_url: None,
        }
    }
}

/// Full account record. `tokens` are **never** part of this struct — they live
/// in the OS credential vault and are loaded on demand by `auth::session`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAccount {
    /// Local row id (not the Minecraft UUID).
    pub id: Uuid,
    pub provider: AccountProvider,
    pub username: String,
    /// Minecraft UUID (dashed form for display).
    pub uuid: Uuid,
    pub skin: SkinProfile,
    /// True when the vault holds a refresh token for this account.
    pub has_stored_credentials: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl UserAccount {
    /// Newly created local account row.
    pub fn new(provider: AccountProvider, username: String, uuid: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            provider,
            username,
            uuid,
            skin: SkinProfile::default(),
            has_stored_credentials: false,
            created_at: now,
            last_used_at: now,
            expires_at: None,
        }
    }

    /// UUID in the undashed form Minecraft launch arguments expect.
    pub fn uuid_undashed(&self) -> String {
        self.uuid.simple().to_string()
    }

    pub fn summary(&self) -> AccountSummary {
        AccountSummary {
            id: self.id,
            provider: self.provider,
            username: self.username.clone(),
            uuid: self.uuid,
            skin: self.skin.clone(),
            has_stored_credentials: self.has_stored_credentials,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
        }
    }
}

/// Trimmed projection sent to the UI — deliberately excludes anything secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: Uuid,
    pub provider: AccountProvider,
    pub username: String,
    pub uuid: Uuid,
    pub skin: SkinProfile,
    pub has_stored_credentials: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: DateTime<Utc>,
}

/// Secret material for a provider session. Serialized only inside the keyring
/// entry backed by Keychain / Credential Manager / Secret Service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    /// MSA only: XUID required by the Mojang launch argument `--xuid`.
    #[serde(default)]
    pub xuid: Option<String>,
    /// MSA only: the client id the token was issued to.
    #[serde(default)]
    pub client_id: Option<String>,
}

impl TokenSet {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token,
            expires_at,
            xuid: None,
            client_id: None,
        }
    }

    /// Treat tokens as expired slightly early so a refresh never races a launch.
    pub fn is_expired(&self, skew_secs: i64) -> bool {
        Utc::now() + Duration::seconds(skew_secs) >= self.expires_at
    }

    pub fn can_refresh(&self) -> bool {
        self.refresh_token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
    }
}

/// `userType` values Mojang's session server understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    Msa,
    Legacy,
    Mojang,
}

impl UserType {
    pub fn as_str(self) -> &'static str {
        match self {
            UserType::Msa => "msa",
            UserType::Legacy => "legacy",
            UserType::Mojang => "mojang",
        }
    }
}

/// Everything the launcher needs to build the `--accessToken/--uuid/...` args.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchIdentity {
    pub username: String,
    /// Undashed UUID.
    pub uuid: String,
    pub access_token: String,
    pub user_type: UserType,
    #[serde(default)]
    pub xuid: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    /// Offline sessions log in as `player<random>` to the local server.
    pub offline: bool,
    /// Authlib-injector endpoint (Ely.by, sx.acc, and other Yggdrasil servers).
    /// When set the launcher must add `-javaagent:authlib-injector.jar=<url>`
    /// to the JVM arguments, otherwise skins and the session are wrong.
    #[serde(default)]
    pub authlib_url: Option<String>,
    /// Appended to Minecraft's `--versionType` (`release` → `release/sx.acc`).
    #[serde(default)]
    pub version_type_suffix: Option<String>,
}

impl LaunchIdentity {
    pub fn offline(username: &str, uuid: Uuid) -> Self {
        Self {
            username: username.to_string(),
            uuid: uuid.simple().to_string(),
            // Mojang's client sends a synthetic token for offline mode.
            access_token: "0".to_string(),
            user_type: UserType::Legacy,
            xuid: None,
            client_id: None,
            offline: true,
            authlib_url: None,
            version_type_suffix: None,
        }
    }

    /// `--versionType` value. sx.acc sessions read `release/sx.acc`.
    pub fn version_type(&self, release_type: Option<&str>) -> String {
        let base = release_type
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("release");
        match self
            .version_type_suffix
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(suffix) => format!("{base}/{suffix}"),
            None => base.to_string(),
        }
    }
}

/// Marker alias documenting that a `Uuid` is a Minecraft account UUID.
pub type MinecraftUuid = Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sx_acc_version_type_keeps_the_release_prefix() {
        let mut identity = LaunchIdentity::offline("Steve", Uuid::nil());
        assert_eq!(identity.version_type(Some("release")), "release");
        assert_eq!(identity.version_type(None), "release");
        identity.version_type_suffix = Some("sx.acc".into());
        assert_eq!(identity.version_type(Some("release")), "release/sx.acc");
        assert_eq!(identity.version_type(Some("snapshot")), "snapshot/sx.acc");
        assert_eq!(identity.version_type(Some("  ")), "release/sx.acc");
    }

    #[test]
    fn provider_wire_name_for_sx_acc_is_snake_case() {
        assert_eq!(AccountProvider::SxAcc.as_str(), "sx_acc");
        assert_eq!(
            AccountProvider::from_str_opt("sx_acc"),
            Some(AccountProvider::SxAcc)
        );
        let json = serde_json::to_string(&AccountProvider::SxAcc).expect("json");
        assert_eq!(json, "\"sx_acc\"");
    }
}
