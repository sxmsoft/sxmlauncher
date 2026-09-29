//! Account management: Microsoft, Ely.by, sx.acc, and offline behind one interface.
//!
//! Design invariants
//! * The **frontend never sees a token**. It sees [`AccountSummary`]s; the
//!   launch identity is assembled inside the backend and handed straight to the
//!   JVM argument builder.
//! * Refresh tokens live only in the OS credential vault.
//! * A refresh that fails with [`AppError::Unauthorized`] (HTTP 401,
//!   `invalid_grant`, or `invalid_token`) means the player must sign in again.
//!   The account row stays, and `has_stored_credentials` flips to `false`.
//! * A timeout or other transport failure does not delete the saved login.
//!   sx.acc skips refresh entirely while the access token is still valid, and
//!   a refresh that does run is capped at a few seconds.
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
pub mod sxacc;
pub mod vault;

pub use elyby::{
    ElyByAuth, ElyDeviceCode, ElySession, AUTHLIB_INJECTOR_URL, ELYBY_CLIENT_ID, ELYBY_SCOPE,
};
pub use msa::{
    DeviceCodePrompt, MicrosoftAuth, MsaLoginOutcome, MSA_LEGACY_REDIRECT_URI, MSA_LEGACY_SCHEME,
};
pub use oauth::{CallbackQuery, CallbackResult, LoopbackServer, PkceCode};
pub use offline::{create_offline_account, offline_identity, offline_uuid, validate_username};
pub use vault::{account_key, select_vault, CredentialVault, KeyringVault, MemoryVault};

/// Settings key holding the currently selected account id.
const ACTIVE_ACCOUNT_KEY: &str = "app.active_account";
/// Refresh a token this many seconds before it actually expires.
const EXPIRY_SKEW_SECS: i64 = 120;

/// How early `ensure_tokens` treats a session as expired.
///
/// sx.acc's refresh route can hang on a token the server still accepts, so
/// Play, profile, and skin use the access token until `expires_at`. Other
/// providers keep the two-minute skew.
fn refresh_skew(provider: AccountProvider) -> i64 {
    match provider {
        AccountProvider::SxAcc => 0,
        _ => EXPIRY_SKEW_SECS,
    }
}

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
    source: CallbackSource,
}

/// Where the authorization result comes back from.
enum CallbackSource {
    /// Custom Azure app, or an Ely.by web app with a registered redirect.
    Loopback(LoopbackServer),
    /// Public Minecraft client: the browser returns `ms-xal-...://auth`.
    Protocol(ProtocolInbox),
    /// Public Ely.by desktop client (`sxmlauncher3`): no redirect is registered.
    ElyDevice(ElyDeviceCode),
}

/// In-process waiter for a protocol callback delivered by the OS.
///
/// The receiver sits behind a mutex because `PendingLogin` is stored in a
/// `DashMap` (which requires `Sync`) between the begin and complete commands.
/// `oneshot::Receiver` itself is not `Sync`.
struct ProtocolInbox {
    state: String,
    rx: Mutex<Option<tokio::sync::oneshot::Receiver<AppResult<CallbackResult>>>>,
}

impl Drop for ProtocolInbox {
    fn drop(&mut self) {
        PROTOCOL_CALLBACKS.remove(&self.state);
    }
}

/// Pending `ms-xal-` callbacks, keyed by the OAuth `state`.
static PROTOCOL_CALLBACKS: std::sync::LazyLock<
    DashMap<String, tokio::sync::oneshot::Sender<AppResult<CallbackResult>>>,
> = std::sync::LazyLock::new(DashMap::new);

/// Hand a protocol URL from the OS (`ms-xal-00000000402b5328://auth?code=...`)
/// to the sign-in that is waiting for it.
///
/// Called from the single-instance / deep-link hooks. URLs that are not this
/// sign-in are ignored.
pub fn deliver_oauth_callback(raw: &str) {
    let Ok(url) = url::Url::parse(raw.trim()) else {
        return;
    };
    if !url.scheme().eq_ignore_ascii_case(MSA_LEGACY_SCHEME)
        && !url.scheme().eq_ignore_ascii_case(sxacc::SXACC_SCHEME)
    {
        return;
    }
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned());
    let Some(state) = state else {
        return;
    };
    let Some((_, sender)) = PROTOCOL_CALLBACKS.remove(&state) else {
        return;
    };
    let parsed = oauth::parse_callback_query(raw);
    let _ = sender.send(match parsed {
        Ok(CallbackQuery::Success(callback)) => Ok(callback),
        Ok(CallbackQuery::ProviderError(error)) => Err(AppError::Account(format!(
            "authorization denied by provider: {error}"
        ))),
        Ok(CallbackQuery::Incomplete) => Err(AppError::Account(
            "the sign-in callback was missing its authorization code".to_string(),
        )),
        Err(err) => Err(err),
    });
}

fn register_protocol_waiter(state: String) -> ProtocolInbox {
    let (tx, rx) = tokio::sync::oneshot::channel();
    PROTOCOL_CALLBACKS.insert(state.clone(), tx);
    ProtocolInbox {
        state,
        rx: Mutex::new(Some(rx)),
    }
}

async fn await_protocol_code(
    inbox: ProtocolInbox,
    expected: &str,
    redirect_uri: &str,
) -> AppResult<String> {
    let rx = inbox.rx.lock().take().ok_or_else(|| {
        AppError::Account("the sign-in callback was already consumed".to_string())
    })?;
    let callback = tokio::time::timeout(oauth::CALLBACK_TIMEOUT, rx)
        .await
        .map_err(|_| {
            let extra = if redirect_uri.starts_with("ms-xal") {
                " If the official Minecraft launcher is also installed it may have \
                 claimed the ms-xal callback; use the device-code sign-in instead."
            } else {
                ""
            };
            AppError::Account(format!(
                "timed out waiting for the sign-in to return to SXMLAUNCHER.{extra}"
            ))
        })?
        .map_err(|_| {
            AppError::Account("the sign-in was cancelled before it finished".to_string())
        })??;
    if callback.state != expected {
        return Err(AppError::Account(
            "OAuth state mismatch; the callback did not match this sign-in attempt".to_string(),
        ));
    }
    Ok(callback.code)
}

impl PendingLogin {
    /// Wait for the browser redirect and return the authorization code.
    async fn wait_for_code(self) -> AppResult<String> {
        let expected = self.state.clone();
        let redirect_uri = self.redirect_uri.clone();
        match self.source {
            CallbackSource::Loopback(server) => {
                let callback = server.wait_for_code(&expected).await?;
                Ok(callback.code)
            }
            CallbackSource::Protocol(inbox) => {
                await_protocol_code(inbox, &expected, &redirect_uri).await
            }
            CallbackSource::ElyDevice(_) => Err(AppError::Account(
                "this Ely.by sign-in does not use an authorization code".to_string(),
            )),
        }
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
    /// sx.acc origin. Empty until the user sets `SXACC_BASE_URL` or Settings.
    pub sxacc_base_url: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            msa_client_id: crate::config::MSA_DEFAULT_CLIENT_ID.to_string(),
            elyby_client_id: crate::config::ELYBY_DEFAULT_CLIENT_ID.to_string(),
            elyby_client_secret: None,
            elyby_redirect_uri: crate::config::AppSettings::default().elyby_redirect_uri,
            sxacc_base_url: String::new(),
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
            sxacc_base_url: settings.sxacc_base_url.clone(),
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
    sxacc: sxacc::SxAccAuth,
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
        let http = crate::http::client();

        Self {
            db,
            vault,
            msa: MicrosoftAuth::with_client_id(http.clone(), config.msa_client_id),
            // `ELYBY_LOCAL_CLIENT` is the Authlib client token, not an OAuth
            // secret. Substituting it makes every token exchange fail with
            // `invalid_client`. A missing secret stays missing.
            elyby: match config
                .elyby_client_secret
                .filter(|secret| !secret.trim().is_empty())
            {
                Some(secret) => {
                    ElyByAuth::with_secret(http.clone(), config.elyby_client_id, secret)
                }
                None => ElyByAuth::new(http.clone(), config.elyby_client_id),
            },
            elyby_redirect_uri: config.elyby_redirect_uri,
            sxacc: sxacc::SxAccAuth::new(http, &config.sxacc_base_url),
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

    #[cfg(test)]
    pub(crate) fn vault(&self) -> &Arc<dyn CredentialVault> {
        &self.vault
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

    /// Begin the Microsoft browser sign-in: returns the URL to open.
    pub async fn begin_msa_login(&self) -> AppResult<PendingLogin> {
        let state = oauth::random_state();
        if self.msa.uses_minecraft_public_client() {
            let authorize_url = self.msa.legacy_authorize_url(&state)?;
            return Ok(PendingLogin {
                login_id: Uuid::new_v4(),
                provider: AccountProvider::Microsoft,
                authorize_url,
                redirect_uri: MSA_LEGACY_REDIRECT_URI.to_string(),
                verifier: String::new(),
                state: state.clone(),
                source: CallbackSource::Protocol(register_protocol_waiter(state)),
            });
        }

        let pkce = PkceCode::generate()?;
        let server = LoopbackServer::bind().await?;
        let authorize_url =
            self.msa
                .authorize_url(server.redirect_uri(), &pkce.challenge, &state)?;

        Ok(PendingLogin {
            login_id: Uuid::new_v4(),
            provider: AccountProvider::Microsoft,
            authorize_url,
            redirect_uri: server.redirect_uri().to_string(),
            verifier: pkce.verifier,
            state,
            source: CallbackSource::Loopback(server),
        })
    }

    /// Finish the Microsoft loopback sign-in (waits for the browser redirect).
    pub async fn complete_msa_login(&self, pending: PendingLogin) -> AppResult<AccountSummary> {
        let redirect_uri = pending.redirect_uri.clone();
        let verifier = pending.verifier.clone();
        let code = pending.wait_for_code().await?;

        let token_response = self
            .msa
            .exchange_code(&code, &verifier, &redirect_uri)
            .await?;
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

    /// Begin the Ely.by browser sign-in.
    ///
    /// The public desktop client `sxmlauncher3` has no registered redirect, so
    /// an authorize URL that includes `redirect_uri` is rejected before the
    /// login form ("Can not find application you are trying to authorize").
    /// That client uses the device-code page on `account.ely.by`, which is the
    /// real account login. A user-supplied web application (client id + secret
    /// + exact redirect) keeps the loopback authorization-code flow.
    pub async fn begin_elyby_login(&self) -> AppResult<PendingLogin> {
        if self.elyby.browser_flow_is_device_code() {
            let issued = self.elyby.begin_device_code().await?;
            let authorize_url = elyby::device_browser_url(&issued.user_code);
            return Ok(PendingLogin {
                login_id: Uuid::new_v4(),
                provider: AccountProvider::ElyBy,
                authorize_url,
                redirect_uri: String::new(),
                verifier: String::new(),
                state: String::new(),
                source: CallbackSource::ElyDevice(issued),
            });
        }

        let state = oauth::random_state();
        let server = LoopbackServer::bind_for_redirect(&self.elyby_redirect_uri).await?;
        let redirect_uri = self.elyby_redirect_uri.clone();
        let authorize_url = self.elyby.authorize_url(&redirect_uri, &state)?;

        Ok(PendingLogin {
            login_id: Uuid::new_v4(),
            provider: AccountProvider::ElyBy,
            authorize_url,
            redirect_uri,
            verifier: String::new(),
            state,
            source: CallbackSource::Loopback(server),
        })
    }

    /// Finish the Ely.by OAuth sign-in.
    pub async fn complete_elyby_login(&self, pending: PendingLogin) -> AppResult<AccountSummary> {
        let PendingLogin {
            redirect_uri,
            state,
            source,
            ..
        } = pending;
        let tokens = match source {
            CallbackSource::ElyDevice(device) => self.elyby.poll_device_code(&device).await?,
            CallbackSource::Loopback(server) => {
                let callback = server.wait_for_code(&state).await?;
                self.elyby
                    .exchange_code(&callback.code, &redirect_uri)
                    .await?
            }
            CallbackSource::Protocol(_) => {
                return Err(AppError::Account(
                    "Ely.by sign-in does not use the Microsoft protocol callback".to_string(),
                ))
            }
        };
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

    /// Flows the configured sx.acc server currently advertises.
    pub async fn sxacc_capabilities(&self) -> sxacc::SxAccCapabilities {
        self.sxacc.capabilities().await
    }

    /// Browser sign-in for sx.acc: authorization code + PKCE, returning through
    /// `sxmlauncher://auth/callback`.
    pub async fn begin_sxacc_login(&self) -> AppResult<PendingLogin> {
        let pkce = PkceCode::generate()?;
        let state = oauth::random_state();
        let authorize_url = self.sxacc.begin_oauth(&pkce.challenge, &state).await?;
        Ok(PendingLogin {
            login_id: Uuid::new_v4(),
            provider: AccountProvider::SxAcc,
            authorize_url,
            redirect_uri: sxacc::SXACC_REDIRECT_URI.to_string(),
            verifier: pkce.verifier,
            state: state.clone(),
            source: CallbackSource::Protocol(register_protocol_waiter(state)),
        })
    }

    /// Finish an sx.acc browser sign-in started by [`Self::begin_sxacc_login`].
    pub async fn complete_sxacc_login(&self, pending: PendingLogin) -> AppResult<AccountSummary> {
        let PendingLogin {
            verifier,
            state,
            redirect_uri,
            source,
            ..
        } = pending;
        let session = match source {
            CallbackSource::Protocol(inbox) => {
                let code = await_protocol_code(inbox, &state, &redirect_uri).await?;
                self.sxacc.exchange_code(&code, &verifier).await?
            }
            CallbackSource::Loopback(_) | CallbackSource::ElyDevice(_) => {
                return Err(AppError::Account(
                    "this sx.acc sign-in does not use that callback".into(),
                ))
            }
        };
        self.finalize_sxacc(session).await
    }

    /// Device-code sign-in, for machines where the custom protocol cannot return.
    pub async fn begin_sxacc_device(&self) -> AppResult<sxacc::SxAccDevicePrompt> {
        let (prompt, _) = self.sxacc.begin_device().await?;
        Ok(prompt)
    }

    pub async fn complete_sxacc_device(
        &self,
        prompt: &sxacc::SxAccDevicePrompt,
    ) -> AppResult<AccountSummary> {
        if prompt.token_url.trim().is_empty() {
            return Err(AppError::Account(
                "this sx.acc device sign-in is missing its token endpoint".into(),
            ));
        }
        let grant = sxacc::SxAccDeviceGrant {
            device_code: prompt.device_code.clone(),
            interval_secs: prompt.interval.max(1),
            expires_at: Utc::now() + chrono::Duration::seconds(prompt.expires_in.max(30)),
            token_url: prompt.token_url.clone(),
        };
        self.finalize_sxacc(self.sxacc.poll_device(&grant).await?)
            .await
    }

    /// Username and password against `{BASE}/v1/auth/login`.
    pub async fn login_sxacc_password(
        &self,
        username: &str,
        password: &str,
    ) -> AppResult<AccountSummary> {
        self.finalize_sxacc(self.sxacc.login_password(username, password).await?)
            .await
    }

    /// Create an sx.acc account from the launcher, then sign in with it.
    pub async fn register_sxacc(
        &self,
        email: &str,
        password: &str,
        username: &str,
    ) -> AppResult<AccountSummary> {
        self.finalize_sxacc(self.sxacc.register(email, password, username).await?)
            .await
    }

    async fn finalize_sxacc(&self, session: sxacc::SxAccSession) -> AppResult<AccountSummary> {
        self.finalize_login(LoginOutcome {
            provider: AccountProvider::SxAcc,
            username: session.username,
            uuid: session.uuid,
            skin: session.skin,
            tokens: Some(session.tokens),
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
        self.db
            .set_setting(ACTIVE_ACCOUNT_KEY, &account.id.to_string())?;
        *self.active.lock() = Some(account.id);

        Ok(account.summary())
    }

    // --- sessions --------------------------------------------------------

    /// Identity for the currently selected account, refreshed if needed.
    pub async fn active_identity(&self) -> AppResult<LaunchIdentity> {
        let id = self.active_id().ok_or(AppError::Unauthorized)?;
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
                version_type_suffix: None,
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
                version_type_suffix: None,
            },
            AccountProvider::SxAcc => LaunchIdentity {
                username: account.username.clone(),
                uuid: account.uuid_undashed(),
                access_token: tokens.access_token.clone(),
                user_type: UserType::Mojang,
                xuid: None,
                client_id: None,
                offline: false,
                authlib_url: Some(self.sxacc.authlib_url()?),
                version_type_suffix: Some(sxacc::SXACC_VERSION_LABEL.to_string()),
            },
            AccountProvider::Offline => offline_identity(&account.username),
        })
    }

    /// Return valid tokens, refreshing exactly once if they are expired.
    async fn ensure_tokens(&self, account: &UserAccount) -> AppResult<TokenSet> {
        let key = account_key(account.provider, account.uuid);
        let stored = self.vault.load(&key).await?.ok_or(AppError::Unauthorized)?;
        let skew = refresh_skew(account.provider);

        // Still accepted by the server: do not call refresh. For sx.acc this
        // is the whole access-token lifetime, so a hung refresh cannot block
        // Play or a skin upload.
        if !stored.is_expired(skew) {
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
                    if !tokens.is_expired(skew) {
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
                self.db
                    .set_account_expiry(account.id, Some(tokens.expires_at))?;
                Ok(tokens)
            }
            // Only a real rejection signs the player out. Timeouts and
            // connection errors leave the vault entry in place.
            Err(AppError::Unauthorized) => {
                self.mark_signed_out(account).await?;
                Err(AppError::Unauthorized)
            }
            Err(AppError::Network(_)) if Utc::now() < stored.expires_at => Ok(stored),
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
                            renamed.skin =
                                self.elyby.fetch_textures(uuid).await.unwrap_or_default();
                        }
                        self.db.upsert_account(&renamed)?;
                    }
                }
                Ok(self.elyby.tokens_from_oauth(&tokens))
            }
            AccountProvider::SxAcc => {
                let refresh_token = stored
                    .refresh_token
                    .as_deref()
                    .ok_or(AppError::Unauthorized)?;
                let session = self.sxacc.refresh(refresh_token).await?;
                if session.username != account.username || session.uuid != account.uuid {
                    let mut renamed = account.clone();
                    renamed.username = session.username.clone();
                    renamed.uuid = session.uuid;
                    renamed.skin = session.skin.clone();
                    self.db.upsert_account(&renamed)?;
                }
                Ok(session.tokens)
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
        if account.provider == AccountProvider::SxAcc {
            if let Some(stored) = self.vault.load(&key).await? {
                let _ = self
                    .sxacc
                    .logout(&stored.access_token, stored.refresh_token.as_deref())
                    .await;
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
                    self.db
                        .set_setting(ACTIVE_ACCOUNT_KEY, &account.id.to_string())?;
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
            AccountProvider::SxAcc => {
                let tokens = self.ensure_tokens(&account).await?;
                self.sxacc.fetch_profile(&tokens.access_token).await?.skin
            }
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
            AccountProvider::SxAcc => {
                let tokens = self.ensure_tokens(&account).await?;
                match self
                    .sxacc
                    .upload_skin(&tokens.access_token, model, png)
                    .await?
                {
                    Some(skin) => {
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
                    None => Ok(skin_upload::SkinUploadOutcome {
                        uploaded: false,
                        skin: account.skin.clone(),
                        message: Some(
                            "This sx.acc server does not accept skin uploads from the launcher. \
                             The PNG passed the local checks."
                                .into(),
                        ),
                        url: None,
                    }),
                }
            }
            AccountProvider::Microsoft => {
                // The upload needs the *Minecraft* token; ensure_tokens runs
                // the whole XBL/XSTS exchange when the stored one is stale.
                let tokens = self.ensure_tokens(&account).await?;
                let http = self.http_for_skins();
                skin_upload::upload_skin_to_mojang(&http, &tokens.access_token, model, png).await?;

                // Read the profile back so the UI can show the applied skin
                // immediately (the session server reflects uploads fast, and
                // it is the same source `refresh_skin` uses).
                let skin = msa::fetch_skin_from_session_server(&http, account.uuid).await?;
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
        crate::http::client()
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
        let mut account =
            UserAccount::new(AccountProvider::Microsoft, "Steve".into(), Uuid::new_v4());
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
    async fn msa_public_client_opens_the_live_login_page() {
        let manager = manager();
        let pending = manager.begin_msa_login().await.expect("begin");
        assert!(pending
            .authorize_url
            .starts_with("https://login.live.com/oauth20_authorize.srf?"));
        assert!(pending.authorize_url.contains("client_id=00000000402b5328"));
        assert!(pending
            .authorize_url
            .contains("redirect_uri=ms-xal-00000000402b5328"));
        assert!(!pending.authorize_url.contains("code_challenge"));
        assert!(!pending.authorize_url.contains("localhost"));
        assert_eq!(pending.redirect_uri, MSA_LEGACY_REDIRECT_URI);
        assert_eq!(pending.provider, AccountProvider::Microsoft);
    }

    #[tokio::test]
    async fn custom_msa_client_keeps_the_pkce_loopback() {
        let db = Database::open_in_memory().expect("db");
        let mut config = ProviderConfig::default();
        config.msa_client_id = "11111111-2222-3333-4444-555555555555".into();
        let manager = AccountManager::new_with_config(db, Arc::new(MemoryVault::new()), config);
        let pending = manager.begin_msa_login().await.expect("begin");
        assert!(pending.authorize_url.contains("login.microsoftonline.com"));
        assert!(pending.authorize_url.contains("code_challenge="));
        assert!(pending.redirect_uri.starts_with("http://localhost:"));
        assert!(pending.redirect_uri.ends_with("/callback"));
    }

    #[tokio::test]
    async fn protocol_callback_returns_only_the_matching_state() {
        let manager = manager();
        let pending = manager.begin_msa_login().await.expect("begin");
        let state = pending.state.clone();

        deliver_oauth_callback("ms-xal-00000000402b5328://auth?code=attacker&state=other");
        deliver_oauth_callback(&format!(
            "ms-xal-00000000402b5328://auth?code=good-code&state={state}"
        ));

        let code = pending.wait_for_code().await.expect("code");
        assert_eq!(code, "good-code");
    }

    #[tokio::test]
    async fn protocol_callback_surfaces_a_provider_error() {
        let manager = manager();
        let pending = manager.begin_msa_login().await.expect("begin");
        let state = pending.state.clone();
        deliver_oauth_callback(&format!(
            "ms-xal-00000000402b5328://auth?error=access_denied&error_description=nope&state={state}"
        ));
        let error = pending.wait_for_code().await.expect_err("denied");
        assert!(error.to_string().contains("access_denied"));
        assert!(error.to_string().contains("nope"));
    }

    #[tokio::test]
    async fn elyby_web_app_uses_the_exact_registered_redirect() {
        let db = Database::open_in_memory().expect("db");
        let mut config = ProviderConfig::default();
        config.elyby_client_id = "my-web-app".into();
        config.elyby_client_secret = Some("not-a-real-secret".into());
        config.elyby_redirect_uri = "http://localhost:25564/elyby/callback".into();
        let manager = AccountManager::new_with_config(db, Arc::new(MemoryVault::new()), config);
        let pending = manager.begin_elyby_login().await.expect("begin");
        assert!(pending
            .authorize_url
            .starts_with("https://account.ely.by/oauth2/v1?"));
        assert!(pending.authorize_url.contains("client_id=my-web-app"));
        assert!(pending.authorize_url.contains("25564"));
        assert!(pending.authorize_url.contains("elyby"));
        assert_eq!(
            pending.redirect_uri,
            "http://localhost:25564/elyby/callback"
        );
        assert_eq!(pending.provider, AccountProvider::ElyBy);
        assert!(!pending.authorize_url.contains("code_challenge"));
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
