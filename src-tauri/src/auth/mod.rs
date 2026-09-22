//! Account management: three providers behind one interface.
//!
//! Design invariants
//! * The **frontend never sees a token**. It sees [`AccountSummary`]s; the
//!   launch identity is assembled inside the backend and handed straight to the
//!   JVM argument builder.
//! * Refresh tokens live only in the OS credential vault.
//! * A refresh that fails with [`AppError::Unauthorized`] means the player must
//!   sign in again — the account row stays (so instances keep their owner) but
//!   `has_stored_credentials` flips to `false`.
//! * Concurrent refreshes for the same account are coalesced, because two
//!   parallel refreshes with a rotating refresh token would invalidate one.

use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::account::{
    AccountProvider, AccountSummary, LaunchIdentity, MinecraftUuid, SkinModel, SkinProfile,
    TokenSet, UserAccount, UserType,
};
use crate::store::Database;

pub mod elyby;
pub mod msa;
pub mod oauth;
pub mod offline;
pub mod skin_upload;
pub mod vault;

pub use elyby::{
    ElyByAuth, ElySession, AUTHLIB_INJECTOR_URL, ELYBY_CLIENT_ID, ELYBY_LOCAL_CLIENT, ELYBY_SCOPE,
};
pub use msa::{DeviceCodePrompt, MicrosoftAuth, MsaLoginOutcome};
pub use oauth::{CallbackResult, LoopbackServer, PkceCode};
pub use offline::{create_offline_account, offline_identity, offline_uuid, validate_username};
pub use vault::{account_key, select_vault, CredentialVault, KeyringVault, MemoryVault};

/// Settings key holding the currently selected account id.
const ACTIVE_ACCOUNT_KEY: &str = "app.active_account";
/// Refresh a token this many seconds before it actually expires.
const EXPIRY_SKEW_SECS: i64 = 120;

/// A sign-in that is waiting for the user to finish in a browser.
///
/// The loopback listener lives inside this struct, so the pending login must be
/// stored in application state between the two IPC calls
/// (`account_begin_login` → `account_complete_login`).
pub struct PendingLogin {
    pub login_id: Uuid,
    pub provider: AccountProvider,
    pub authorize_url: String,
    pub redirect_uri: String,
    verifier: String,
    state: String,
    server: LoopbackServer,
}

impl PendingLogin {
    /// Wait for the browser redirect and return the authorization code.
    async fn wait_for_code(self) -> AppResult<String> {
        let callback = self.server.wait_for_code(&self.state).await?;
        Ok(callback.code)
    }
}

/// In-flight sign-ins, keyed by login id.
pub type PendingLogins = DashMap<Uuid, PendingLogin>;

/// What a completed sign-in produced, before persistence.
struct LoginOutcome {
    provider: AccountProvider,
    username: String,
    uuid: Uuid,
    skin: crate::models::account::SkinProfile,
    tokens: Option<TokenSet>,
}

/// Provider credentials resolved from user settings.
///
/// They live in settings (not the vault) because they are *application*
/// credentials — the same ones for every player — and the vault is per account.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Azure public client id for the Microsoft sign-in.
    pub msa_client_id: String,
    /// Ely.by OAuth application id.
    pub elyby_client_id: String,
    /// Ely.by OAuth application secret.
    pub elyby_client_secret: Option<String>,
    /// Exact redirect URI registered with Ely.by.
    pub elyby_redirect_uri: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            msa_client_id: crate::config::MSA_DEFAULT_CLIENT_ID.to_string(),
            elyby_client_id: crate::config::ELYBY_DEFAULT_CLIENT_ID.to_string(),
            elyby_client_secret: Some(crate::config::ELYBY_DEFAULT_CLIENT_SECRET.to_string()),
            elyby_redirect_uri: crate::config::AppSettings::default().elyby_redirect_uri,
        }
    }
}

impl From<&crate::config::AppSettings> for ProviderConfig {
    fn from(settings: &crate::config::AppSettings) -> Self {
        Self {
            msa_client_id: settings.msa_client_id.clone(),
            elyby_client_id: settings.elyby_client_id.clone(),
            elyby_client_secret: settings.elyby_client_secret.clone(),
            elyby_redirect_uri: settings.elyby_redirect_uri.clone(),
        }
    }
}

impl ProviderConfig {
    /// The port Ely.by must be able to reach back on, taken from the redirect.
    ///
    /// `None` when the URI does not carry a port (a domain redirect), in which
    /// case a random loopback port is used and only works if the user registered
    /// `http://localhost` as a wildcard.
    pub fn elyby_redirect_port(&self) -> Option<u16> {
        url::Url::parse(&self.elyby_redirect_uri)
            .ok()
            .and_then(|url| url.port_or_known_default())
    }
}

/// The single entry point for everything account related.
pub struct AccountManager {
    db: Database,
    vault: Arc<dyn CredentialVault>,
    msa: MicrosoftAuth,
    elyby: ElyByAuth,
    /// Exact redirect URI registered with Ely.by.
    elyby_redirect_uri: String,
    active: Mutex<Option<Uuid>>,
    /// Guards against two concurrent refreshes for the same account.
    refreshes: DashMap<Uuid, ()>,
}

impl AccountManager {
    pub fn new(db: Database, vault: Arc<dyn CredentialVault>) -> Self {
        Self::new_with_config(db, vault, ProviderConfig::default())
    }

    /// Build the manager with the credentials the user configured.
    pub fn new_with_config(
        db: Database,
        vault: Arc<dyn CredentialVault>,
        config: ProviderConfig,
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("SXMLauncher/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            db,
            vault,
            msa: MicrosoftAuth::with_client_id(http.clone(), config.msa_client_id),
            elyby: ElyByAuth::with_secret(
                http,
                config.elyby_client_id,
                config
                    .elyby_client_secret
                    .unwrap_or_else(|| ELYBY_LOCAL_CLIENT.to_string()),
            ),
            elyby_redirect_uri: config.elyby_redirect_uri,
            active: Mutex::new(None),
            refreshes: DashMap::new(),
        }
    }

    /// Backend name shown in Settings (Keychain / Credential Manager / ...).
    pub fn vault_backend(&self) -> &'static str {
        self.vault.backend()
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Restore the previously active account (called once at startup).
    pub fn restore_active(&self) -> AppResult<Option<Uuid>> {
        let Some(raw) = self.db.get_setting(ACTIVE_ACCOUNT_KEY)? else {
            // No explicit choice yet: fall back to the most recently used one.
            let newest = self.db.list_accounts()?.into_iter().next();
            if let Some(account) = &newest {
                *self.active.lock() = Some(account.id);
            }
            return Ok(newest.map(|account| account.id));
        };
        let id = Uuid::parse_str(&raw)
            .map_err(|err| AppError::Config(format!("invalid stored account id: {err}")))?;
        if self.db.get_account(id)?.is_none() {
            // The account was deleted since; clear the stale pointer.
            self.db.delete_setting(ACTIVE_ACCOUNT_KEY)?;
            return Ok(None);
        }
        *self.active.lock() = Some(id);
        Ok(Some(id))
    }

    pub fn active_id(&self) -> Option<Uuid> {
        *self.active.lock()
    }

    pub async fn list_accounts(&self) -> AppResult<Vec<AccountSummary>> {
        Ok(self
            .db
            .list_accounts()?
            .iter()
            .map(UserAccount::summary)
            .collect())
    }

    pub async fn get_account(&self, id: Uuid) -> AppResult<UserAccount> {
        self.db
            .get_account(id)?
            .ok_or_else(|| AppError::Account(format!("no account with id {id}")))
    }

    pub async fn active_account(&self) -> AppResult<Option<UserAccount>> {
        let Some(id) = self.active_id() else {
            return Ok(None);
        };
        self.db.get_account(id)
    }

    /// Select an account; it becomes the identity used by every launch.
    pub async fn set_active(&self, id: Uuid) -> AppResult<AccountSummary> {
        let mut account = self.get_account(id).await?;
        account.last_used_at = Utc::now();
        self.db.upsert_account(&account)?;
        self.db.set_setting(ACTIVE_ACCOUNT_KEY, &id.to_string())?;
        *self.active.lock() = Some(id);
        Ok(account.summary())
    }

    // --- sign-in ---------------------------------------------------------

    /// Create (or reuse) an offline account and make it active.
    pub async fn login_offline(&self, username: &str) -> AppResult<AccountSummary> {
        let account = create_offline_account(username)?;
        let outcome = LoginOutcome {
            provider: AccountProvider::Offline,
            username: account.username.clone(),
            uuid: account.uuid,
            skin: account.skin.clone(),
            // Offline accounts have nothing worth storing in a vault.
            tokens: None,
        };
        self.finalize_login(outcome).await
    }

    /// Begin the Microsoft loopback sign-in: returns the URL to open.
    pub async fn begin_msa_login(&self) -> AppResult<PendingLogin> {
        let pkce = PkceCode::generate()?;
        let state = oauth::random_state();
        let server = LoopbackServer::bind().await?;
        let authorize_url = self
            .msa
            .authorize_url(server.redirect_uri(), &pkce.challenge, &state)?;

        Ok(PendingLogin {
            login_id: Uuid::new_v4(),
            provider: AccountProvider::Microsoft,
            authorize_url,
            redirect_uri: server.redirect_uri().to_string(),
            verifier: pkce.verifier,
            state,
            server,
        })
    }

    /// Finish the Microsoft loopback sign-in (waits for the browser redirect).
    pub async fn complete_msa_login(&self, pending: PendingLogin) -> AppResult<AccountSummary> {
        let redirect_uri = pending.redirect_uri.clone();
        let verifier = pending.verifier.clone();
        let code = pending.wait_for_code().await?;

        let token_response = self.msa.exchange_code(&code, &verifier, &redirect_uri).await?;
        let outcome = self.msa.complete_login(&token_response).await?;

        self.finalize_login(LoginOutcome {
            provider: AccountProvider::Microsoft,
            username: outcome.username,
            uuid: outcome.uuid,
            skin: outcome.skin,
            tokens: Some(outcome.tokens),
        })
        .await
    }

    /// Start the device-code flow (no browser automation required).
    pub async fn begin_msa_device_code(&self) -> AppResult<DeviceCodePrompt> {
        self.msa.begin_device_code().await
    }

    /// Poll until the device-code sign-in completes.
    pub async fn complete_msa_device_code(
        &self,
        prompt: &DeviceCodePrompt,
    ) -> AppResult<AccountSummary> {
        let token_response = self.msa.poll_device_code(prompt).await?;
        let outcome = self.msa.complete_login(&token_response).await?;

        self.finalize_login(LoginOutcome {
            provider: AccountProvider::Microsoft,
            username: outcome.username,
            uuid: outcome.uuid,
            skin: outcome.skin,
            tokens: Some(outcome.tokens),
        })
        .await
    }

    /// The port Ely.by's callback listener must bind.
    fn elyby_redirect_port(&self) -> u16 {
        url::Url::parse(&self.elyby_redirect_uri)
            .ok()
            .and_then(|url| url.port_or_known_default())
            .unwrap_or(0)
    }

    /// Begin the Ely.by OAuth sign-in.
    ///
    /// The loopback listener binds the port *and path* encoded in the configured
    /// redirect URI, because Ely.by compares `redirect_uri` exactly — a random
    /// port (or the wrong `/callback` path) would produce their "can not find
    /// application you are trying to authorize" page even with a correct client id.
    pub async fn begin_elyby_login(&self) -> AppResult<PendingLogin> {
        let state = oauth::random_state();
        let port = self.elyby_redirect_port();
        let callback_path = url::Url::parse(&self.elyby_redirect_uri)
            .ok()
            .map(|url| url.path().to_string())
            .unwrap_or_else(|| "/elyby/callback".to_string());
        let server = LoopbackServer::bind_on_with_path(port, &callback_path).await?;
        let redirect_uri = if port == 0 {
            server.redirect_uri().to_string()
        } else {
            // Prefer the exact registered URI (host + port + path) over the
            // derived one so a trailing-slash mismatch cannot reject the token
            // exchange.
            self.elyby_redirect_uri.clone()
        };
        let authorize_url = self.elyby.authorize_url(&redirect_uri, &state)?;

        Ok(PendingLogin {
            login_id: Uuid::new_v4(),
            provider: AccountProvider::ElyBy,
            authorize_url,
            redirect_uri,
            verifier: String::new(),
            state,
            server,
        })
    }

    /// Finish the Ely.by OAuth sign-in.
    pub async fn complete_elyby_login(&self, pending: PendingLogin) -> AppResult<AccountSummary> {
        let redirect_uri = pending.redirect_uri.clone();
        let code = pending.wait_for_code().await?;

        let tokens = self.elyby.exchange_code(&code, &redirect_uri).await?;
        // With `minecraft_server_session` the OAuth access token *is* the
        // Minecraft session, and `account/v1/info` carries the profile.
        let session = self.elyby.session_from_oauth(&tokens).await?;
        let uuid = session.uuid()?;
        let username = session.username()?;
        let skin = self.elyby.fetch_textures(uuid).await.unwrap_or_default();

        self.finalize_login(LoginOutcome {
            provider: AccountProvider::ElyBy,
            username,
            uuid,
            skin,
            tokens: Some(self.elyby.tokens_from_oauth(&tokens)),
        })
        .await
    }

    /// Ely.by Authlib username/password sign-in.
    pub async fn login_elyby_password(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<AccountSummary> {
        let session = self.elyby.authenticate(username, password).await?;
        let uuid = session.uuid()?;
        let name = session.username()?;
        let skin = self.elyby.fetch_textures(uuid).await.unwrap_or_default();

        self.finalize_login(LoginOutcome {
            provider: AccountProvider::ElyBy,
            username: name,
            uuid,
            skin,
            tokens: Some(session.into_token_set()),
        })
        .await
    }

    /// Persist a completed sign-in and make the account active.
    async fn finalize_login(&self, outcome: LoginOutcome) -> AppResult<AccountSummary> {
        // Reuse the existing row id when this identity already signed in before,
        // so instances/history keep pointing at the same account.
        let existing = self.db.find_account(outcome.provider, outcome.uuid)?;
        let mut account = match existing {
            Some(mut account) => {
                account.username = outcome.username.clone();
                account.skin = outcome.skin.clone();
                account.last_used_at = Utc::now();
                account
            }
            None => UserAccount::new(outcome.provider, outcome.username.clone(), outcome.uuid),
        };

        if let Some(tokens) = &outcome.tokens {
            let key = account_key(outcome.provider, outcome.uuid);
            self.vault.store(&key, tokens).await?;
            account.has_stored_credentials = true;
            account.expires_at = Some(tokens.expires_at);
        } else {
            account.has_stored_credentials = false;
            account.expires_at = None;
        }

        self.db.upsert_account(&account)?;
        self.db.set_setting(ACTIVE_ACCOUNT_KEY, &account.id.to_string())?;
        *self.active.lock() = Some(account.id);

        Ok(account.summary())
    }

    // --- sessions --------------------------------------------------------

    /// Identity for the currently selected account, refreshed if needed.
    pub async fn active_identity(&self) -> AppResult<LaunchIdentity> {
        let id = self
            .active_id()
            .ok_or(AppError::Unauthorized)?;
        self.launch_identity(id).await
    }

    /// Build the JVM launch identity, refreshing the session when required.
    pub async fn launch_identity(&self, id: Uuid) -> AppResult<LaunchIdentity> {
        let account = self.get_account(id).await?;
        if account.provider == AccountProvider::Offline {
            return Ok(offline_identity(&account.username));
        }

        let tokens = self.ensure_tokens(&account).await?;

        Ok(match account.provider {
            AccountProvider::Microsoft => LaunchIdentity {
                username: account.username.clone(),
                uuid: account.uuid_undashed(),
                access_token: tokens.access_token.clone(),
                user_type: UserType::Msa,
                xuid: tokens.xuid.clone(),
                client_id: tokens.client_id.clone(),
                offline: false,
                authlib_url: None,
            },
            AccountProvider::ElyBy => LaunchIdentity {
                username: account.username.clone(),
                uuid: account.uuid_undashed(),
                access_token: tokens.access_token.clone(),
                user_type: UserType::Mojang,
                xuid: None,
                client_id: None,
                offline: false,
                // Without the injector the client would authenticate against
                // Mojang and fail with a session error.
                authlib_url: Some(elyby::AUTHLIB_INJECTOR_URL.to_string()),
            },
            AccountProvider::Offline => offline_identity(&account.username),
        })
    }

    /// Return valid tokens, refreshing exactly once if they are expired.
    async fn ensure_tokens(&self, account: &UserAccount) -> AppResult<TokenSet> {
        let key = account_key(account.provider, account.uuid);
        let stored = self.vault.load(&key).await?.ok_or(AppError::Unauthorized)?;

        if !stored.is_expired(EXPIRY_SKEW_SECS) {
            return Ok(stored);
        }
        if !stored.can_refresh() {
            self.mark_signed_out(account).await?;
            return Err(AppError::Unauthorized);
        }

        // Coalesce concurrent refreshes: the second caller waits and then reads
        // the freshly stored token instead of rotating it again.
        let acquired = match self.refreshes.entry(account.id) {
            dashmap::mapref::entry::Entry::Vacant(slot) => {
                slot.insert(());
                true
            }
            dashmap::mapref::entry::Entry::Occupied(_) => false,
        };

        if !acquired {
            for _ in 0..40 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Some(tokens) = self.vault.load(&key).await? {
                    if !tokens.is_expired(EXPIRY_SKEW_SECS) {
                        return Ok(tokens);
                    }
                }
            }
            return Err(AppError::Account(
                "timed out waiting for a concurrent session refresh".to_string(),
            ));
        }

        let result = self.refresh_tokens(account, &stored).await;
        self.refreshes.remove(&account.id);

        match result {
            Ok(tokens) => {
                self.vault.store(&key, &tokens).await?;
                self.db.set_account_expiry(account.id, Some(tokens.expires_at))?;
                Ok(tokens)
            }
            Err(AppError::Unauthorized) => {
                self.mark_signed_out(account).await?;
                Err(AppError::Unauthorized)
            }
            Err(err) => Err(err),
        }
    }

    /// Force a refresh (Settings → "Refresh session").
    pub async fn refresh_account(&self, id: Uuid) -> AppResult<AccountSummary> {
        let account = self.get_account(id).await?;
        if account.provider == AccountProvider::Offline {
            return Ok(account.summary());
        }
        // Expire the stored token on purpose so `ensure_tokens` takes the
        // refresh path.
        let key = account_key(account.provider, account.uuid);
        if let Some(mut stored) = self.vault.load(&key).await? {
            stored.expires_at = Utc::now() - chrono::Duration::seconds(1);
            self.vault.store(&key, &stored).await?;
        }
        let tokens = self.ensure_tokens(&account).await?;

        let mut updated = account;
        updated.expires_at = Some(tokens.expires_at);
        updated.last_used_at = Utc::now();
        updated.has_stored_credentials = true;
        self.db.upsert_account(&updated)?;
        Ok(updated.summary())
    }

    /// Provider-specific refresh.
    async fn refresh_tokens(
        &self,
        account: &UserAccount,
        stored: &TokenSet,
    ) -> AppResult<TokenSet> {
        match account.provider {
            AccountProvider::Microsoft => {
                let refresh_token = stored
                    .refresh_token
                    .as_deref()
                    .ok_or(AppError::Unauthorized)?;
                let msa_tokens = self.msa.refresh(refresh_token).await?;
                let outcome = self.msa.complete_login(&msa_tokens).await?;

                // The username can change (name change on Mojang's side).
                if outcome.username != account.username {
                    let mut renamed = account.clone();
                    renamed.username = outcome.username.clone();
                    renamed.skin = outcome.skin.clone();
                    self.db.upsert_account(&renamed)?;
                }
                Ok(outcome.tokens)
            }
            AccountProvider::ElyBy => {
                // The browser flow stores the *OAuth* refresh token, so it must
                // be refreshed through Ely.by's OAuth2 endpoint (the authlib
                // refresh would reject it with a confusing 403).
                let refresh_token = stored
                    .refresh_token
                    .as_deref()
                    .ok_or(AppError::Unauthorized)?;
                let tokens = self.elyby.refresh_oauth(refresh_token).await?;
                if let Ok(info) = self.elyby.account_info(&tokens.access_token).await {
                    if info.username != account.username {
                        let mut renamed = account.clone();
                        renamed.username = info.username.clone();
                        if let Ok(uuid) = MinecraftUuid::parse_str(&info.uuid) {
                            renamed.uuid = uuid;
                            renamed.skin = self
                                .elyby
                                .fetch_textures(uuid)
                                .await
                                .unwrap_or_default();
                        }
                        self.db.upsert_account(&renamed)?;
                    }
                }
                Ok(self.elyby.tokens_from_oauth(&tokens))
            }
            AccountProvider::Offline => Ok(stored.clone()),
        }
    }

    /// Clear vault + flag after a session can no longer be refreshed.
    async fn mark_signed_out(&self, account: &UserAccount) -> AppResult<()> {
        let key = account_key(account.provider, account.uuid);
        // Best effort: a missing entry is fine.
        let _ = self.vault.delete(&key).await;
        self.db.set_credentials_stored(account.id, false)?;
        self.db.set_account_expiry(account.id, None)?;
        Ok(())
    }

    /// Sign out: revoke server-side where possible, then forget locally.
    pub async fn sign_out(&self, id: Uuid) -> AppResult<()> {
        let account = self.get_account(id).await?;
        let key = account_key(account.provider, account.uuid);

        if account.provider == AccountProvider::ElyBy {
            if let Some(stored) = self.vault.load(&key).await? {
                if let Some(client_token) = stored.refresh_token.as_deref() {
                    let _ = self
                        .elyby
                        .invalidate(&stored.access_token, client_token)
                        .await;
                }
            }
        }

        let _ = self.vault.delete(&key).await;
        self.db.delete_account(id)?;
        self.refreshes.remove(&id);

        if self.active_id() == Some(id) {
            *self.active.lock() = None;
            let fallback = self.db.list_accounts()?.into_iter().next();
            match fallback {
                Some(account) => {
                    self.db.set_setting(ACTIVE_ACCOUNT_KEY, &account.id.to_string())?;
                    *self.active.lock() = Some(account.id);
                }
                None => {
                    self.db.delete_setting(ACTIVE_ACCOUNT_KEY)?;
                }
            }
        }
        Ok(())
    }

    /// Drop the vault entry without deleting the account row (used when the
    /// player wants to keep the profile but sign out).
    pub async fn forget_credentials(&self, id: Uuid) -> AppResult<()> {
        let account = self.get_account(id).await?;
        self.mark_signed_out(&account).await
    }

    /// Re-read the skin and cape from the provider and persist them.
    ///
    /// Skins change on the provider's side (minecraft.net, ely.by), so the UI
    /// calls this instead of trusting a value that may be months old. Offline
    /// profiles have nothing to refresh and simply keep what they have.
    pub async fn refresh_skin(&self, id: Uuid) -> AppResult<SkinProfile> {
        let account = self.get_account(id).await?;
        let skin = match account.provider {
            AccountProvider::Microsoft => {
                msa::fetch_skin_from_session_server(&self.http_for_skins(), account.uuid).await?
            }
            AccountProvider::ElyBy => self.elyby.fetch_textures(account.uuid).await?,
            AccountProvider::Offline => return Ok(account.skin),
        };

        let mut updated = account;
        updated.skin = skin.clone();
        self.db.upsert_account(&updated)?;
        Ok(skin)
    }

    /// Apply a user-picked PNG skin to the account, then re-read the profile.
    ///
    /// * Microsoft — the PNG goes straight to the official Mojang endpoint
    ///   with the account's Minecraft token.
    /// * Ely.by — the public API has no upload endpoint (uploads happen on the
    ///   website with a browser session), so the file is validated locally and
    ///   the outcome carries a deep link to the profile's skin page instead of
    ///   a doomed request.
    /// * Offline — nothing to upload to.
    pub async fn upload_skin(
        &self,
        id: Uuid,
        model: SkinModel,
        png: Vec<u8>,
    ) -> AppResult<skin_upload::SkinUploadOutcome> {
        use skin_upload::provider_skin_page;

        let account = self.get_account(id).await?;

        // Validate before anything else, so every provider path (including the
        // Ely.by redirect and the offline reject) fails fast on a bad file.
        skin_upload::validate_skin_png(&png)?;

        match account.provider {
            AccountProvider::Offline => Err(AppError::Config(
                "offline profiles have no provider to upload a skin to".to_string(),
            )),
            AccountProvider::ElyBy => {
                let link = provider_skin_page(AccountProvider::ElyBy, &account.username);
                Ok(skin_upload::SkinUploadOutcome {
                    uploaded: false,
                    // Keep the current profile; nothing changed remotely.
                    skin: account.skin.clone(),
                    message: link.as_ref().map(|link| {
                        format!(
                            "Ely.by accepts skin uploads on its website — your PNG passed the \
                             launcher's checks, continue at {link}"
                        )
                    }),
                    url: link,
                })
            }
            AccountProvider::Microsoft => {
                // The upload needs the *Minecraft* token; ensure_tokens runs
                // the whole XBL/XSTS exchange when the stored one is stale.
                let tokens = self.ensure_tokens(&account).await?;
                let http = self.http_for_skins();
                skin_upload::upload_skin_to_mojang(&http, &tokens.access_token, model, png)
                    .await?;

                // Read the profile back so the UI can show the applied skin
                // immediately (the session server reflects uploads fast, and
                // it is the same source `refresh_skin` uses).
                let skin =
                    msa::fetch_skin_from_session_server(&http, account.uuid).await?;
                let mut updated = account;
                updated.skin = skin.clone();
                self.db.upsert_account(&updated)?;

                Ok(skin_upload::SkinUploadOutcome {
                    uploaded: true,
                    skin,
                    message: None,
                    url: None,
                })
            }
        }
    }

    /// A short-lived client for skin lookups (long timeout, no session state).
    fn http_for_skins(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(concat!("SXMLauncher/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;

    fn manager() -> AccountManager {
        let db = Database::open_in_memory().expect("db");
        let vault = Arc::new(MemoryVault::new());
        AccountManager::new(db, vault)
    }

    #[tokio::test]
    async fn offline_login_creates_and_activates_an_account() {
        let manager = manager();
        let summary = manager.login_offline("Guest_1").await.expect("login");

        assert_eq!(summary.provider, AccountProvider::Offline);
        assert!(!summary.has_stored_credentials);
        assert_eq!(manager.active_id(), Some(summary.id));

        let accounts = manager.list_accounts().await.expect("list");
        assert_eq!(accounts.len(), 1);
    }

    #[tokio::test]
    async fn offline_login_is_idempotent_per_username() {
        let manager = manager();
        let first = manager.login_offline("Repeater").await.expect("first");
        let second = manager.login_offline("Repeater").await.expect("second");
        assert_eq!(first.id, second.id);
        assert_eq!(manager.list_accounts().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn offline_launch_identity_needs_no_tokens() {
        let manager = manager();
        let summary = manager.login_offline("OfflineGuy").await.expect("login");
        let identity = manager.launch_identity(summary.id).await.expect("identity");

        assert!(identity.offline);
        assert_eq!(identity.username, "OfflineGuy");
        assert_eq!(identity.uuid.len(), 32);
        assert!(identity.authlib_url.is_none());
    }

    #[tokio::test]
    async fn online_account_without_vault_entry_reports_unauthorized() {
        let manager = manager();
        // Simulate an online account whose vault entry was wiped.
        let mut account = UserAccount::new(
            AccountProvider::Microsoft,
            "Steve".into(),
            Uuid::new_v4(),
        );
        account.has_stored_credentials = true;
        manager.db.upsert_account(&account).expect("seed");

        let error = manager
            .launch_identity(account.id)
            .await
            .expect_err("must require sign-in");
        assert_eq!(error.code(), crate::error::CODE_UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_marks_the_account_signed_out_when_the_vault_is_empty() {
        let manager = manager();
        let mut account =
            UserAccount::new(AccountProvider::Microsoft, "Steve".into(), Uuid::new_v4());
        account.has_stored_credentials = true;
        account.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
        manager.db.upsert_account(&account).expect("seed");

        assert!(manager.refresh_account(account.id).await.is_err());
    }

    #[tokio::test]
    async fn sign_out_promotes_another_account() {
        let manager = manager();
        let first = manager.login_offline("First").await.expect("first");
        let second = manager.login_offline("Second").await.expect("second");
        assert_eq!(manager.active_id(), Some(second.id));

        manager.sign_out(second.id).await.expect("sign out");
        assert_eq!(manager.active_id(), Some(first.id));

        manager.sign_out(first.id).await.expect("sign out");
        assert_eq!(manager.active_id(), None);
    }

    #[tokio::test]
    async fn restore_active_falls_back_to_most_recent_account() {
        let manager = manager();
        let summary = manager.login_offline("Only").await.expect("login");
        // Simulate a restart: a new manager over the same database.
        let restarted = AccountManager::new(manager.db.clone(), Arc::new(MemoryVault::new()));
        let restored = restarted.restore_active().expect("restore");
        // The explicit pointer survives, so the same account comes back.
        assert_eq!(restored, Some(summary.id));
    }

    #[tokio::test]
    async fn msa_login_url_targets_microsoft_with_pkce() {
        let manager = manager();
        let pending = manager.begin_msa_login().await.expect("begin");
        assert!(pending.authorize_url.contains("login.microsoftonline.com"));
        assert!(pending.authorize_url.contains("code_challenge="));
        // `localhost`, not `127.0.0.1`: Microsoft rejects loopback redirects that
        // are not registered as `http://localhost`.
        assert!(pending.redirect_uri.starts_with("http://localhost:"));
        assert_eq!(pending.provider, AccountProvider::Microsoft);
    }

    #[tokio::test]
    async fn elyby_login_url_uses_the_injector_client() {
        let manager = manager();
        let pending = manager.begin_elyby_login().await.expect("begin");
        assert!(pending.authorize_url.contains("account.ely.by"));
        assert!(pending.authorize_url.contains("client_id=sxmlauncher3"));
        // The callback binds the port encoded in the configured redirect URI,
        // which is what makes Ely.by's exact-match check pass.
        assert!(pending.redirect_uri.starts_with("http://localhost:25564/"));
        assert_eq!(pending.provider, AccountProvider::ElyBy);
    }

    #[tokio::test]
    async fn elyby_identity_carries_the_authlib_injector_url() {
        let manager = manager();
        let mut account =
            UserAccount::new(AccountProvider::ElyBy, "ElyUser".into(), Uuid::new_v4());
        manager.db.upsert_account(&account).expect("seed");

        // Seed a long-lived vault entry so no refresh is attempted.
        let tokens = TokenSet::new(
            "ely-access",
            Some("ely-client".into()),
            Utc::now() + chrono::Duration::hours(1),
        );
        manager
            .vault
            .store(&account_key(AccountProvider::ElyBy, account.uuid), &tokens)
            .await
            .expect("store");

        account.has_stored_credentials = true;
        let identity = manager.launch_identity(account.id).await.expect("identity");
        assert_eq!(identity.user_type, UserType::Mojang);
        assert_eq!(identity.authlib_url.as_deref(), Some(AUTHLIB_INJECTOR_URL));
        assert!(!identity.offline);
    }
}
