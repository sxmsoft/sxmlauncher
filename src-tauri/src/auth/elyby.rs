//! Ely.by authentication.
//!
//! Two interchangeable front doors (documented at docs.ely.by):
//!
//! 1. **Authlib / Yggdrasil** (`authserver.ely.by/auth/*`) — the same protocol
//!    Mojang used pre-migration. A launch must then attach authlib-injector
//!    (`-javaagent:authlib-injector.jar=https://authserver.ely.by/api/authlib-injector`)
//!    so the client talks to Ely.by instead of Mojang.
//! 2. **OAuth2** (`account.ely.by`) — browser based, no password ever touches the
//!    launcher. The public desktop client `sxmlauncher3` has no registered
//!    redirect, so its browser flow is device code (`/code?user_code=`). A
//!    custom web application exchanges an authorization code and must send the
//!    exact trio `client_id`, `client_secret`, `redirect_uri`; a mismatched
//!    redirect fails with `invalid_client`.
//!
//! Both produce a Minecraft-shaped session (`accessToken` + profile) so the rest
//! of the launcher never needs to know which one was used.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::account::{MinecraftUuid, SkinModel, SkinProfile, TokenSet};

/// Authlib-injector endpoint advertised to the JVM.
pub const AUTHLIB_INJECTOR_URL: &str = "https://authserver.ely.by/api/authlib-injector";
/// Public OAuth client id used for the browser sign-in flow.
pub const ELYBY_CLIENT_ID: &str = "sxmlauncher3";
const AUTHSERVER: &str = "https://authserver.ely.by";
/// Browser entry point. **No** `/auth` suffix: Ely.by's OAuth2 server serves the
/// consent page straight from `oauth2/v1`.
const OAUTH_AUTHORIZE_URL: &str = "https://account.ely.by/oauth2/v1";
/// Token endpoint (note the `/api` prefix — the authlib one is different).
const OAUTH_TOKEN_URL: &str = "https://account.ely.by/api/oauth2/v1/token";
/// User info endpoint (needs the `account_info` scope).
const ACCOUNT_INFO_URL: &str = "https://account.ely.by/api/account/v1/info";
const SKIN_SYSTEM: &str = "https://skinsystem.ely.by";

/// Ely.by texture URLs are often `http://`. The webview CSP allows `https:`
/// images and blocks plain `http:` (except the asset host), so a stored http
/// URL never paints and the preview falls through to Steve.
pub fn https_texture_url(url: &str) -> String {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let host = lower
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if host == "ely.by" || host.ends_with(".ely.by") {
        return format!("https://{}", &trimmed["http://".len()..]);
    }
    trimmed.to_string()
}

/// Scope set needed to read the profile and keep a refresh token.
pub const ELYBY_SCOPE: &str = "account_info offline_access minecraft_server_session";
/// Client token Ely.by expects when the launcher owns the session lifecycle.
pub const ELYBY_LOCAL_CLIENT: &str = "sxmlauncher";

/// Result of an Authlib authenticate/refresh call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElySession {
    pub access_token: String,
    pub client_token: String,
    #[serde(default)]
    pub selected_profile: Option<ElyProfileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElyProfileRef {
    /// Undashed UUID.
    pub id: String,
    pub name: String,
}

impl ElySession {
    pub fn profile(&self) -> AppResult<&ElyProfileRef> {
        self.selected_profile.as_ref().ok_or_else(|| {
            AppError::Account("Ely.by session did not select a Minecraft profile".to_string())
        })
    }

    pub fn uuid(&self) -> AppResult<MinecraftUuid> {
        let id = &self.profile()?.id;
        MinecraftUuid::parse_str(id).map_err(|err| {
            AppError::Account(format!("Ely.by returned an invalid uuid `{id}`: {err}"))
        })
    }

    pub fn username(&self) -> AppResult<String> {
        Ok(self.profile()?.name.clone())
    }

    /// Ely.by sessions are long lived; treat the token as valid for a day so the
    /// UI can show a meaningful "refresh" affordance.
    pub fn into_token_set(&self) -> TokenSet {
        TokenSet::new(
            self.access_token.clone(),
            Some(self.client_token.clone()),
            Utc::now() + Duration::hours(24),
        )
    }
}

#[derive(Debug, Deserialize)]
struct ElyAuthErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    /// Device-code and token errors use `message` rather than `error_message`.
    #[serde(default)]
    message: Option<String>,
}

/// OAuth2 token response from Ely.by.
#[derive(Debug, Clone, Deserialize)]
pub struct ElyOAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    /// Seconds the access token is valid for (86400 = one day).
    #[serde(default)]
    pub expires_in: i64,
}

impl ElyOAuthTokens {
    /// Expiry with a small safety margin, used for the vault entry.
    ///
    /// Ely.by's desktop clients sometimes omit `expires_in` (or send 0) for a
    /// token the docs describe as valid for a day. Treat that as 24 hours so
    /// the launcher does not immediately refresh a brand-new session.
    pub fn expires_at(&self) -> chrono::DateTime<Utc> {
        let seconds = if self.expires_in <= 0 {
            86_400
        } else {
            self.expires_in
        };
        Utc::now() + Duration::seconds(seconds.max(60) - 60)
    }
}

/// Device-code grant issued for the public desktop client.
#[derive(Debug, Clone)]
pub struct ElyDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: i64,
}

/// Browser page that starts the Ely.by sign-in for a device code.
///
/// The account site reads `user_code` from the query and continues into the
/// real login form. The value Ely.by returns as `verification_uri` is `http://`,
/// so this always uses `https://account.ely.by`.
pub fn device_browser_url(user_code: &str) -> String {
    let mut url = url::Url::parse("https://account.ely.by/code").expect("ely.by code page");
    url.query_pairs_mut().append_pair("user_code", user_code);
    url.to_string()
}

/// `GET account/v1/info` response.
#[derive(Debug, Clone, Deserialize)]
pub struct ElyAccountInfo {
    /// Minecraft profile UUID (dashed) — this is what the game gets.
    pub uuid: String,
    pub username: String,
}

/// Textures returned by the skin system.
///
/// Only `properties` is consumed; the profile's own `id`/`name` are redundant
/// with the UUID we already asked for, so they are deliberately not modelled.
#[derive(Debug, Deserialize)]
struct TextureProfile {
    #[serde(default)]
    properties: Vec<TextureProperty>,
}

#[derive(Debug, Deserialize)]
struct TextureProperty {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

/// Mojang-format texture payload stored base64 inside `properties[].value`.
#[derive(Debug, Deserialize)]
struct DecodedTextures {
    #[serde(default)]
    textures: DecodedTextureMap,
}

#[derive(Debug, Default, Deserialize)]
struct DecodedTextureMap {
    // Mojang's format upper-cases these keys; Ely.by mirrors Mojang, so accept
    // both spellings instead of guessing which server we are talking to.
    #[serde(default, alias = "SKIN")]
    skin: Option<DecodedTexture>,
    #[serde(default, alias = "CAPE")]
    cape: Option<DecodedTexture>,
}

#[derive(Debug, Deserialize)]
struct DecodedTexture {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    metadata: Option<DecodedTextureMetadata>,
}

#[derive(Debug, Deserialize)]
struct DecodedTextureMetadata {
    #[serde(default)]
    model: Option<String>,
}

/// Ely.by authentication client.
#[derive(Debug, Clone)]
pub struct ElyByAuth {
    http: reqwest::Client,
    client_id: String,
    /// Required by the token endpoint; without it every exchange fails with
    /// `invalid_client` ("the trio client_id + client_secret + redirect_uri did
    /// not match any registered application").
    client_secret: Option<String>,
}

impl ElyByAuth {
    pub fn new(http: reqwest::Client, client_id: impl Into<String>) -> Self {
        Self {
            http,
            client_id: client_id.into(),
            client_secret: None,
        }
    }

    /// Build the client with its application secret.
    pub fn with_secret(
        http: reqwest::Client,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            http,
            client_id: client_id.into(),
            client_secret: Some(client_secret.into()),
        }
    }

    /// Update the application credentials in place (Settings → Fixes).
    pub fn set_credentials(&mut self, client_id: String, client_secret: Option<String>) {
        self.client_id = client_id;
        self.client_secret = client_secret.filter(|secret| !secret.trim().is_empty());
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn client_secret(&self) -> Option<&str> {
        self.client_secret.as_deref()
    }

    /// The historical `sxmlauncher3` application is a **desktop** client.
    ///
    /// Ely.by's validate API returns `invalid_client` ("Can not find application
    /// you are trying to authorize") for every `redirect_uri`, including
    /// `http://localhost:25564/elyby/callback`, and accepts the request only
    /// when the redirect is omitted. Device code is the browser flow that
    /// client allows, and it does not need a client secret. A configured secret
    /// means the user registered their own web application, which uses the
    /// authorization-code redirect instead.
    pub fn browser_flow_is_device_code(&self) -> bool {
        self.client_id == ELYBY_CLIENT_ID || self.client_secret.is_none()
    }

    /// Browser-based authorize URL.
    ///
    /// Ely.by's OAuth2 does not implement PKCE; a `code_challenge` would simply
    /// be ignored, so this flow is authenticated by the `client_secret` instead.
    pub fn authorize_url(&self, redirect_uri: &str, state: &str) -> AppResult<String> {
        let mut url = url::Url::parse(OAUTH_AUTHORIZE_URL)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &self.client_id);
            query.append_pair("response_type", "code");
            query.append_pair("redirect_uri", redirect_uri);
            query.append_pair("scope", ELYBY_SCOPE);
            query.append_pair("state", state);
            query.append_pair("prompt", "select_account");
        }
        Ok(url.to_string())
    }

    /// Exchange an authorization code for the OAuth2 tokens.
    pub async fn exchange_code(&self, code: &str, redirect_uri: &str) -> AppResult<ElyOAuthTokens> {
        let mut params = vec![
            ("client_id", self.client_id.clone()),
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
        ];
        if let Some(secret) = &self.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        self.post_token(params, "Ely.by token request failed").await
    }

    /// Start the device-code grant used by the public desktop client.
    pub async fn begin_device_code(&self) -> AppResult<ElyDeviceCode> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("scope", ELYBY_SCOPE),
        ];
        let response = self
            .http
            .post("https://account.ely.by/api/oauth2/v1/devicecode")
            .form(&params)
            .send()
            .await
            .map_err(|err| {
                AppError::Network(format!("Ely.by device code request failed: {err}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Ely.by refused the device code request ({status}): {}",
                summarize(&body)
            )));
        }

        #[derive(Deserialize)]
        struct Issued {
            device_code: String,
            user_code: String,
            expires_in: i64,
            #[serde(default = "default_interval")]
            interval: i64,
        }
        fn default_interval() -> i64 {
            5
        }

        let issued: Issued = response.json().await?;
        Ok(ElyDeviceCode {
            device_code: issued.device_code,
            user_code: issued.user_code,
            expires_in: issued.expires_in,
            interval: issued.interval.max(1),
        })
    }

    /// Poll until the player finishes the device-code sign-in on Ely.by.
    pub async fn poll_device_code(&self, pending: &ElyDeviceCode) -> AppResult<ElyOAuthTokens> {
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(u64::try_from(pending.expires_in).unwrap_or(600));
        let mut interval =
            std::time::Duration::from_secs(u64::try_from(pending.interval).unwrap_or(5));

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::Account(
                    "the Ely.by sign-in code expired before it was used".to_string(),
                ));
            }

            let mut params = vec![
                ("client_id", self.client_id.clone()),
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:device_code".to_string(),
                ),
                ("device_code", pending.device_code.clone()),
            ];
            if let Some(secret) = &self.client_secret {
                params.push(("client_secret", secret.clone()));
            }

            let response = self
                .http
                .post(OAUTH_TOKEN_URL)
                .form(&params)
                .send()
                .await
                .map_err(|err| AppError::Network(format!("Ely.by device poll failed: {err}")))?;

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status.is_success() {
                return serde_json::from_str::<ElyOAuthTokens>(&body).map_err(|err| {
                    AppError::Account(format!("unexpected Ely.by device response: {err}"))
                });
            }

            let code = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(|error| error.as_str())
                        .map(str::to_string)
                });
            match code.as_deref() {
                Some("authorization_pending") => {
                    tokio::time::sleep(interval).await;
                    continue;
                }
                Some("slow_down") => {
                    interval += std::time::Duration::from_secs(5);
                    tokio::time::sleep(interval).await;
                    continue;
                }
                Some("expired_token") | Some("expired_user_code") => {
                    return Err(AppError::Account(
                        "the Ely.by sign-in code expired before it was used".to_string(),
                    ))
                }
                Some("access_denied") => {
                    return Err(AppError::Account("Ely.by sign-in was declined".to_string()))
                }
                _ => {
                    return Err(AppError::Account(format!(
                        "Ely.by device sign-in failed ({status}): {}",
                        summarize(&body)
                    )))
                }
            }
        }
    }

    async fn post_token(
        &self,
        params: Vec<(&str, String)>,
        context: &str,
    ) -> AppResult<ElyOAuthTokens> {
        let response = self
            .http
            .post(OAUTH_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("{context}: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Ely.by rejected the sign-in ({status}): {}. All three of client id, client \
                 secret and redirect URI must match the application registered on ely.by \
                 (see Settings → Fixes).",
                summarize(&body)
            )));
        }
        Ok(response.json::<ElyOAuthTokens>().await?)
    }

    /// Refresh an OAuth2 session (`grant_type=refresh_token`).
    pub async fn refresh_oauth(&self, refresh_token: &str) -> AppResult<ElyOAuthTokens> {
        let mut params = vec![
            ("client_id", self.client_id.clone()),
            ("grant_type", "refresh_token".to_string()),
            ("scope", ELYBY_SCOPE.to_string()),
            ("refresh_token", refresh_token.to_string()),
        ];
        if let Some(secret) = &self.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        let response = self
            .http
            .post(OAUTH_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by refresh failed: {err}")))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            // An expired/invalidated token must force a fresh sign-in.
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(AppError::Unauthorized);
            }
            return Err(AppError::Account(format!(
                "Ely.by refresh failed ({status}): {}",
                summarize(&body)
            )));
        }
        Ok(
            serde_json::from_str::<ElyOAuthTokens>(&body).map_err(|err| {
                AppError::Account(format!("unexpected Ely.by refresh response: {err}"))
            })?,
        )
    }

    /// Read the Minecraft profile attached to an OAuth2 access token.
    pub async fn account_info(&self, access_token: &str) -> AppResult<ElyAccountInfo> {
        let response = self
            .http
            .get(ACCOUNT_INFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by account lookup failed: {err}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(AppError::Unauthorized);
            }
            return Err(AppError::Account(format!(
                "Ely.by profile lookup failed ({status}): {}",
                summarize(&body)
            )));
        }

        response
            .json::<ElyAccountInfo>()
            .await
            .map_err(|err| AppError::Account(format!("unexpected Ely.by profile response: {err}")))
    }

    /// Turn OAuth2 tokens into the Minecraft-shaped session the rest of the
    /// launcher expects. With the `minecraft_server_session` scope the OAuth
    /// access token *is* the Minecraft session token.
    pub async fn session_from_oauth(&self, tokens: &ElyOAuthTokens) -> AppResult<ElySession> {
        let info = self.account_info(&tokens.access_token).await?;
        Ok(ElySession {
            access_token: tokens.access_token.clone(),
            client_token: ELYBY_LOCAL_CLIENT.to_string(),
            selected_profile: Some(ElyProfileRef {
                id: info.uuid,
                name: info.username,
            }),
        })
    }

    /// Build the vault token set from OAuth2 tokens (kept in sync with expiry).
    pub fn tokens_from_oauth(&self, tokens: &ElyOAuthTokens) -> TokenSet {
        TokenSet::new(
            tokens.access_token.clone(),
            tokens.refresh_token.clone(),
            tokens.expires_at(),
        )
    }

    /// Authlib username/password sign-in (used by the password tab in the UI).
    pub async fn authenticate(&self, username: &str, password: &str) -> AppResult<ElySession> {
        let client_token = uuid::Uuid::new_v4().simple().to_string();
        let payload = serde_json::json!({
            "username": username,
            "password": password,
            "clientToken": client_token,
            "requestUser": true
        });

        let response = self
            .http
            .post(format!("{AUTHSERVER}/auth/authenticate"))
            .header("User-Agent", "SXMLauncher/0.1")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by authenticate failed: {err}")))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            // 403 + ForbiddenOperationException is Ely.by's "wrong credentials".
            if status.as_u16() == 403 {
                return Err(AppError::Account(
                    "Ely.by rejected those credentials. Check your username/email and password."
                        .to_string(),
                ));
            }
            return Err(AppError::Account(format!(
                "Ely.by sign-in failed ({status}): {}",
                summarize(&body)
            )));
        }

        serde_json::from_str::<ElySession>(&body)
            .map_err(|err| AppError::Account(format!("unexpected Ely.by response: {err}")))
    }

    /// Refresh an Authlib session.
    pub async fn refresh(&self, access_token: &str, client_token: &str) -> AppResult<ElySession> {
        let payload = serde_json::json!({
            "accessToken": access_token,
            "clientToken": client_token,
            "requestUser": true
        });

        let response = self
            .http
            .post(format!("{AUTHSERVER}/auth/refresh"))
            .header("User-Agent", "SXMLauncher/0.1")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by refresh failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // An expired/invalidated token must force a fresh sign-in.
            if status.as_u16() == 403 {
                return Err(AppError::Unauthorized);
            }
            return Err(AppError::Account(format!(
                "Ely.by refresh failed ({status}): {}",
                summarize(&body)
            )));
        }
        Ok(response.json::<ElySession>().await?)
    }

    /// `true` when the session is still valid server-side.
    pub async fn validate(&self, access_token: &str) -> AppResult<bool> {
        let response = self
            .http
            .post(format!("{AUTHSERVER}/auth/validate"))
            .json(&serde_json::json!({ "accessToken": access_token }))
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by validate failed: {err}")))?;
        Ok(response.status().is_success())
    }

    /// Invalidate a session on sign-out so the token cannot be replayed.
    pub async fn invalidate(&self, access_token: &str, client_token: &str) -> AppResult<()> {
        let response = self
            .http
            .post(format!("{AUTHSERVER}/auth/invalidate"))
            .json(&serde_json::json!({
                "accessToken": access_token,
                "clientToken": client_token
            }))
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by sign-out failed: {err}")))?;

        if !response.status().is_success() {
            // Sign-out must never fail loudly: the local session is cleared
            // regardless, and a stale server session expires on its own.
            eprintln!("[auth] Ely.by invalidate returned {}", response.status());
        }
        Ok(())
    }

    /// Resolve skin/cape URLs. Falls back to the deterministic texture URLs when
    /// the profile endpoint gives us nothing useful.
    pub async fn fetch_textures(&self, uuid: MinecraftUuid) -> AppResult<SkinProfile> {
        let plain_id = uuid.simple().to_string();
        let response = self
            .http
            .get(format!("{SKIN_SYSTEM}/profile/{plain_id}"))
            .header("User-Agent", "SXMLauncher/0.1")
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Ely.by texture lookup failed: {err}")))?;

        if !response.status().is_success() {
            // Not fatal: the player simply keeps the default skin.
            return Ok(SkinProfile {
                model: SkinModel::Classic,
                skin_url: Some(https_texture_url(&format!(
                    "{SKIN_SYSTEM}/textures/{plain_id}"
                ))),
                cape_url: None,
            });
        }

        let profile: TextureProfile = response.json().await?;
        let decoded = profile
            .properties
            .iter()
            .find(|property| property.name.as_deref() == Some("textures"))
            .and_then(|property| property.value.as_deref())
            .and_then(decode_texture_property);

        Ok(SkinProfile {
            model: decoded
                .as_ref()
                .and_then(|textures| textures.textures.skin.as_ref())
                .and_then(|skin| skin.metadata.as_ref())
                .and_then(|metadata| metadata.model.as_deref())
                .map(|model| {
                    if model.eq_ignore_ascii_case("slim") {
                        SkinModel::Slim
                    } else {
                        SkinModel::Classic
                    }
                })
                .unwrap_or(SkinModel::Classic),
            skin_url: decoded
                .as_ref()
                .and_then(|textures| textures.textures.skin.as_ref())
                .and_then(|skin| skin.url.clone())
                .or_else(|| Some(format!("{SKIN_SYSTEM}/textures/{plain_id}")))
                .map(|url| https_texture_url(&url)),
            cape_url: decoded
                .and_then(|textures| textures.textures.cape)
                .and_then(|cape| cape.url)
                .map(|url| https_texture_url(&url)),
        })
    }
}

/// Base64-decode the `textures` property (Mojang's format, Ely.by reuses it).
fn decode_texture_property(value: &str) -> Option<DecodedTextures> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn summarize(body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<ElyAuthErrorBody>(body) {
        if let Some(message) = parsed.error_message.or(parsed.message) {
            return match parsed.error {
                Some(error) if !message.contains(&error) => format!("{error}: {message}"),
                _ => message,
            };
        }
        if let Some(error) = parsed.error {
            return error;
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "no details returned".to_string()
    } else {
        trimmed.chars().take(300).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn session_json() -> String {
        serde_json::json!({
            "accessToken": "ely-token",
            "clientToken": "client-token",
            "selectedProfile": { "id": "069a79f444e94726a5befca90e38aaf5", "name": "ElyUser" }
        })
        .to_string()
    }

    #[test]
    fn parses_authlib_session() {
        let session: ElySession = serde_json::from_str(&session_json()).expect("session");
        assert_eq!(session.username().expect("name"), "ElyUser");
        assert_eq!(
            session.uuid().expect("uuid").to_string(),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
        assert_eq!(
            session.into_token_set().refresh_token.as_deref(),
            Some("client-token")
        );
    }

    #[test]
    fn session_without_profile_is_an_error() {
        let session: ElySession = serde_json::from_str(
            &serde_json::json!({ "accessToken": "t", "clientToken": "c" }).to_string(),
        )
        .expect("session");
        assert!(session.username().is_err());
    }

    fn encode_textures(payload: serde_json::Value) -> String {
        base64::engine::general_purpose::STANDARD.encode(payload.to_string())
    }

    #[test]
    fn ely_texture_urls_are_upgraded_to_https() {
        assert_eq!(
            https_texture_url("http://ely.by/storage/skins/abc.png"),
            "https://ely.by/storage/skins/abc.png"
        );
        assert_eq!(
            https_texture_url("http://skinsystem.ely.by/textures/uuid"),
            "https://skinsystem.ely.by/textures/uuid"
        );
        assert_eq!(
            https_texture_url("https://textures.minecraft.net/texture/abc"),
            "https://textures.minecraft.net/texture/abc"
        );
    }

    #[test]
    fn decodes_mojang_cased_texture_properties() {
        let encoded = encode_textures(serde_json::json!({
            "textures": {
                "SKIN": { "url": "https://skinsystem.ely.by/skin", "metadata": { "model": "slim" } },
                "CAPE": { "url": "https://skinsystem.ely.by/cape" }
            }
        }));

        let decoded = decode_texture_property(&encoded).expect("decodes");
        let skin = decoded.textures.skin.expect("skin decoded");
        assert_eq!(skin.url.as_deref(), Some("https://skinsystem.ely.by/skin"));
        assert_eq!(
            skin.metadata.and_then(|metadata| metadata.model).as_deref(),
            Some("slim")
        );
        assert_eq!(
            decoded.textures.cape.and_then(|cape| cape.url).as_deref(),
            Some("https://skinsystem.ely.by/cape")
        );
    }

    #[test]
    fn decodes_lowercase_texture_keys_too() {
        let encoded = encode_textures(serde_json::json!({
            "textures": { "skin": { "url": "https://s/s" } }
        }));
        let decoded = decode_texture_property(&encoded).expect("decodes");
        assert_eq!(
            decoded.textures.skin.and_then(|skin| skin.url).as_deref(),
            Some("https://s/s")
        );
        assert!(decoded.textures.cape.is_none());
    }

    #[test]
    fn garbage_texture_property_decodes_to_none() {
        assert!(decode_texture_property("!!!not-base64!!!").is_none());
        assert!(decode_texture_property("").is_none());
    }

    #[test]
    fn authorize_url_matches_the_documented_oauth_flow() {
        let auth =
            ElyByAuth::with_secret(reqwest::Client::new(), "sxmlauncher3", "not-a-real-secret");
        let url = auth
            .authorize_url("http://localhost:25564/elyby/callback", "state-1")
            .expect("url");

        // Ely.by's endpoint is `oauth2/v1` (no `/auth` suffix) and the flow is
        // authenticated with the client secret, not with PKCE.
        assert!(url.starts_with("https://account.ely.by/oauth2/v1?"));
        assert!(url.contains("client_id=sxmlauncher3"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
        assert!(url.contains("state=state-1"));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn device_browser_url_is_the_real_elyby_account_host() {
        let url = device_browser_url("RCHPBBTK");
        let parsed = url::Url::parse(&url).expect("url");
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("account.ely.by"));
        assert_eq!(parsed.path(), "/code");
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "user_code")
                .map(|(_, value)| value.into_owned())
                .as_deref(),
            Some("RCHPBBTK")
        );
        // The public desktop client rejects every redirect_uri. The page URL
        // must not carry one or Ely.by shows "can not find application".
        assert!(!url.contains("redirect_uri"));
        assert!(!url.contains("sxmlauncher"));
    }

    #[test]
    fn public_client_without_a_secret_uses_device_code() {
        let auth = ElyByAuth::new(reqwest::Client::new(), "sxmlauncher3");
        assert!(auth.browser_flow_is_device_code());
        assert!(auth.client_secret().is_none());

        let custom = ElyByAuth::with_secret(reqwest::Client::new(), "my-web-app", "real-secret");
        assert!(!custom.browser_flow_is_device_code());
    }

    #[test]
    fn missing_oauth_expiry_lasts_a_day() {
        let tokens = ElyOAuthTokens {
            access_token: "a".into(),
            refresh_token: None,
            token_type: None,
            expires_in: 0,
        };
        let lifetime = (tokens.expires_at() - Utc::now()).num_seconds();
        assert!((86_280..=86_400).contains(&lifetime), "lifetime {lifetime}");
    }

    #[test]
    fn token_refresh_carries_the_client_secret() {
        let auth = ElyByAuth::with_secret(reqwest::Client::new(), "sxmlauncher3", "secret-1");
        let debug = format!("{auth:?}");
        assert!(debug.contains("sxmlauncher3"));
        assert!(debug.contains("secret-1"));
    }

    #[test]
    fn oauth_token_expiry_has_a_safety_margin() {
        let tokens = ElyOAuthTokens {
            access_token: "a".into(),
            refresh_token: None,
            token_type: None,
            expires_in: 86400,
        };
        let lifetime = (tokens.expires_at() - Utc::now()).num_seconds();
        assert!((86280..=86400).contains(&lifetime), "lifetime {lifetime}");
    }
}
