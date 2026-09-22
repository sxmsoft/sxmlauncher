//! Microsoft authentication: MSA (OAuth2 + PKCE) → Xbox Live → XSTS → Mojang.
//!
//! ```text
//!  browser ──code──▶ MSA token endpoint ──access_token──▶ XBL user.auth
//!        ──xbl token──▶ XSTS authorize ──xsts token──▶ api.minecraftservices
//!        ──mc access_token──▶ /minecraft/profile
//! ```
//!
//! Two interactive entry points are supported:
//! * [`MicrosoftAuth::authorize_url`] + [`LoopbackServer`](super::oauth::LoopbackServer)
//!   — system browser with a loopback redirect (preferred).
//! * [`MicrosoftAuth::begin_device_code`] — device authorization grant, used
//!   when no browser can be opened (headless, remote session, locked-down box).

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::account::{MinecraftUuid, SkinModel, SkinProfile, TokenSet};

/// Public client id used by every third-party Minecraft launcher.
pub const MSA_CLIENT_ID: &str = "00000000402b5328";
/// `XboxLive.signin` grants the Xbox scopes; `offline_access` yields a refresh
/// token (which is what lets us keep the player signed in).
pub const MSA_SCOPE: &str = "XboxLive.signin offline_access";

const AUTHORIZE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const XBL_AUTHORIZE_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const MC_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const XBL_RELYING_PARTY: &str = "http://auth.xboxlive.com";
const MC_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";

/// Tokens returned by the MSA token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsaTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Lifetime in seconds (MSA always sends this).
    #[serde(default)]
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl MsaTokenResponse {
    /// Convert to the vault shape, with a safety margin on the expiry.
    pub fn into_token_set(self) -> TokenSet {
        // Refresh 60s early so a launch never straddles the expiry boundary.
        let lifetime = Duration::seconds(self.expires_in.max(0) - 60);
        TokenSet::new(
            self.access_token,
            self.refresh_token,
            Utc::now() + lifetime,
        )
    }
}

/// Device-code grant prompt shown to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    /// Seconds until the device code itself expires.
    pub expires_in: i64,
    /// Minimum seconds between polls, as required by the spec.
    pub interval: i64,
    /// Opaque code used by [`MicrosoftAuth::poll_device_code`].
    pub device_code: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    message: String,
    expires_in: i64,
    #[serde(default = "default_poll_interval")]
    interval: i64,
}

fn default_poll_interval() -> i64 {
    5
}

/// Xbox Live user token exchange result.
#[derive(Debug, Clone)]
pub struct XblToken {
    pub token: String,
    pub user_hash: String,
}

/// XSTS token + the identity material the Mojang login needs.
#[derive(Debug, Clone)]
pub struct XstsToken {
    pub token: String,
    pub user_hash: String,
    pub xuid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XblResponse {
    token: String,
    #[serde(default)]
    display_claims: Option<XblClaims>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XblClaims {
    #[serde(default)]
    xui: Vec<XuiClaim>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct XuiClaim {
    #[serde(default)]
    uhs: Option<String>,
    #[serde(default)]
    xid: Option<String>,
}

/// Mojang session granted by `login_with_xbox`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftSession {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: i64,
    pub username: String,
}

/// `/minecraft/profile` payload.
#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<MinecraftSkin>,
    #[serde(default)]
    pub capes: Vec<MinecraftCape>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftSkin {
    pub id: String,
    pub state: String,
    pub url: String,
    /// `classic` (wide) or `slim` (Alex).
    #[serde(default, rename = "variant")]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftCape {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    pub url: String,
}

impl MinecraftProfile {
    pub fn uuid(&self) -> AppResult<MinecraftUuid> {
        dashed_uuid_from_hex(&self.id)
    }

    /// Only the *active* skin counts; Mojang returns inactive variants too.
    pub fn active_skin(&self) -> SkinProfile {
        let skin = self.skins.iter().find(|skin| skin.state == "ACTIVE");
        SkinProfile {
            model: match skin.and_then(|skin| skin.variant.as_deref()) {
                Some("slim") => SkinModel::Slim,
                _ => SkinModel::Classic,
            },
            skin_url: skin.map(|skin| skin.url.clone()),
            cape_url: self.capes.first().map(|cape| cape.url.clone()),
        }
    }
}

/// Convert Mojang's 32-char hex id (no dashes) into a UUID.
pub fn dashed_uuid_from_hex(raw: &str) -> AppResult<MinecraftUuid> {
    uuid::Uuid::parse_str(raw.trim()).map_err(|err| {
        AppError::Account(format!("Minecraft returned an invalid profile id `{raw}`: {err}"))
    })
}

/// The Minecraft/Xbox Live authentication client.
#[derive(Debug, Clone)]
pub struct MicrosoftAuth {
    http: reqwest::Client,
    client_id: String,
}

impl MicrosoftAuth {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: MSA_CLIENT_ID.to_string(),
        }
    }

    /// Build the client with a user-supplied Azure application id.
    ///
    /// Microsoft occasionally restricts the shared Minecraft client id for
    /// third-party launchers; when that happens a user registers their own app
    /// (public, `XboxLive.signin` scope, `http://localhost` redirect) and pastes
    /// its id into Settings → Fixes.
    pub fn with_client_id(http: reqwest::Client, client_id: impl Into<String>) -> Self {
        Self {
            http,
            client_id: client_id.into(),
        }
    }

    /// Build the authorize URL. `challenge` = PKCE S256 challenge.
    pub fn authorize_url(
        &self,
        redirect_uri: &str,
        challenge: &str,
        state: &str,
    ) -> AppResult<String> {
        let mut url = url::Url::parse(AUTHORIZE_URL)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &self.client_id);
            query.append_pair("response_type", "code");
            query.append_pair("redirect_uri", redirect_uri);
            query.append_pair("response_mode", "query");
            query.append_pair("scope", MSA_SCOPE);
            query.append_pair("prompt", "select_account");
            query.append_pair("code_challenge", challenge);
            query.append_pair("code_challenge_method", super::oauth::PkceCode::method());
            query.append_pair("state", state);
        }
        Ok(url.to_string())
    }

    /// Exchange an authorization code (with its PKCE verifier) for tokens.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> AppResult<MsaTokenResponse> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("scope", MSA_SCOPE),
        ];
        let response = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("MSA token request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Microsoft rejected the sign-in code ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }
        Ok(response.json::<MsaTokenResponse>().await?)
    }

    /// Start the device-code grant.
    pub async fn begin_device_code(&self) -> AppResult<DeviceCodePrompt> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("scope", MSA_SCOPE),
        ];
        let response = self
            .http
            .post(DEVICE_CODE_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("device code request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Microsoft refused the device code request ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }

        let device: DeviceCodeResponse = response.json().await?;
        Ok(DeviceCodePrompt {
            user_code: device.user_code,
            verification_uri: device.verification_uri,
            message: device.message,
            expires_in: device.expires_in,
            interval: device.interval.max(1),
            device_code: device.device_code,
        })
    }

    /// Poll the token endpoint until the user finishes, or the code expires.
    ///
    /// Honours `authorization_pending` / `slow_down` exactly as the spec asks,
    /// and gives up at `expires_in` so a forgotten prompt cannot leak a task.
    pub async fn poll_device_code(&self, prompt: &DeviceCodePrompt) -> AppResult<MsaTokenResponse> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(
            u64::try_from(prompt.expires_in).unwrap_or(900),
        );
        let mut interval = std::time::Duration::from_secs(
            u64::try_from(prompt.interval).unwrap_or(5),
        );

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::Account(
                    "the sign-in code expired before it was used".to_string(),
                ));
            }
            tokio::time::sleep(interval).await;

            let params = [
                ("client_id", self.client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", prompt.device_code.as_str()),
            ];
            let response = self
                .http
                .post(TOKEN_URL)
                .form(&params)
                .send()
                .await
                .map_err(|err| AppError::Network(format!("device token poll failed: {err}")))?;

            if response.status().is_success() {
                return Ok(response.json::<MsaTokenResponse>().await?);
            }

            let body = response.text().await.unwrap_or_default();
            match oauth_error_code(&body).as_deref() {
                // Normal while the user is still typing the code.
                Some("authorization_pending") => continue,
                // We are polling too fast; back off as instructed.
                Some("slow_down") => {
                    interval += std::time::Duration::from_secs(5);
                    continue;
                }
                Some("expired_token") => {
                    return Err(AppError::Account(
                        "the sign-in code expired before it was used".to_string(),
                    ))
                }
                Some("authorization_declined") => {
                    return Err(AppError::Account("sign-in was declined".to_string()))
                }
                _ => {
                    return Err(AppError::Account(format!(
                        "device sign-in failed: {}",
                        summarize_oauth_error(&body)
                    )))
                }
            }
        }
    }

    /// Refresh an MSA session using a stored refresh token.
    pub async fn refresh(&self, refresh_token: &str) -> AppResult<MsaTokenResponse> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", MSA_SCOPE),
        ];
        let response = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("MSA refresh failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let code = oauth_error_code(&body);
            // An invalid_grant means the player revoked access: surface it as a
            // sign-in requirement so the UI can prompt instead of retrying.
            if code.as_deref() == Some("invalid_grant") {
                return Err(AppError::Unauthorized);
            }
            return Err(AppError::Account(format!(
                "Microsoft refresh failed ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }
        Ok(response.json::<MsaTokenResponse>().await?)
    }

    /// Step 1: MSA access token → Xbox Live user token.
    async fn authenticate_xbox(&self, msa_access_token: &str) -> AppResult<XblToken> {
        let payload = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={msa_access_token}")
            },
            "RelyingParty": XBL_RELYING_PARTY,
            "TokenType": "JWT"
        });

        let response = self
            .http
            .post(XBL_AUTHORIZE_URL)
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Xbox Live auth failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Xbox Live rejected the Microsoft token ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }

        let xbl: XblResponse = response.json().await?;
        let user_hash = xbl
            .display_claims
            .and_then(|claims| claims.xui.into_iter().next())
            .and_then(|claim| claim.uhs)
            .ok_or_else(|| {
                AppError::Account("Xbox Live response did not include a user hash".to_string())
            })?;

        Ok(XblToken {
            token: xbl.token,
            user_hash,
        })
    }

    /// Step 2: XBL token → XSTS token (this is where account problems surface).
    async fn authorize_xsts(&self, xbl_token: &str) -> AppResult<XstsToken> {
        let payload = serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": MC_RELYING_PARTY,
            "TokenType": "JWT"
        });

        let response = self
            .http
            .post(XSTS_AUTHORIZE_URL)
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("XSTS authorize failed: {err}")))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            // XSTS reports *why* a player cannot play, and the codes are far
            // more useful to the user than the HTTP status.
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
                let xerr = parsed
                    .get("XErr")
                    .and_then(|value| value.as_u64())
                    .unwrap_or_default();
                if let Some(message) = xsts_error_message(xerr) {
                    return Err(AppError::Account(message.to_string()));
                }
            }
            return Err(AppError::Account(format!(
                "Xbox Live authorization failed ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }

        let xsts: XblResponse = serde_json::from_str(&body)?;
        let claim = xsts
            .display_claims
            .and_then(|claims| claims.xui.into_iter().next())
            .ok_or_else(|| {
                AppError::Account("Xbox Live token had no identity claims".to_string())
            })?;

        Ok(XstsToken {
            token: xsts.token,
            user_hash: claim.uhs.unwrap_or_default(),
            xuid: claim.xid.unwrap_or_default(),
        })
    }

    /// Step 3: XSTS token → Mojang access token.
    pub async fn login_with_xbox(&self, xsts: &XstsToken) -> AppResult<MinecraftSession> {
        let payload = serde_json::json!({
            "identityToken": format!("XBL3.0 x={};{}", xsts.user_hash, xsts.token)
        });

        let response = self
            .http
            .post(MC_LOGIN_URL)
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Minecraft login failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "Minecraft login failed ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }
        Ok(response.json::<MinecraftSession>().await?)
    }

    /// Step 4: read the profile (name, uuid, skins, capes).
    pub async fn fetch_profile(&self, mc_access_token: &str) -> AppResult<MinecraftProfile> {
        let response = self
            .http
            .get(MC_PROFILE_URL)
            .bearer_auth(mc_access_token)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("profile request failed: {err}")))?;

        if response.status().as_u16() == 401 {
            return Err(AppError::Account(
                "the Minecraft session token was rejected".to_string(),
            ));
        }
        if response.status().as_u16() == 404 {
            // 404 here means "no Java Edition profile on this account".
            return Err(AppError::Account(
                "this Microsoft account does not own Minecraft: Java Edition".to_string(),
            ));
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Account(format!(
                "could not load the Minecraft profile ({status}): {}",
                summarize_oauth_error(&body)
            )));
        }
        Ok(response.json::<MinecraftProfile>().await?)
    }

    /// `true` when the account owns the Java Edition entitlement.
    pub async fn has_java_entitlement(&self, mc_access_token: &str) -> AppResult<bool> {
        let response = self
            .http
            .get(MC_ENTITLEMENTS_URL)
            .bearer_auth(mc_access_token)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("entitlements request failed: {err}")))?;

        if !response.status().is_success() {
            return Ok(false);
        }
        let payload: serde_json::Value = response.json().await?;
        let owned = payload
            .get("items")
            .and_then(|items| items.as_array())
            .map(|items| {
                items.iter().any(|item| {
                    item.get("name")
                        .and_then(|name| name.as_str())
                        .is_some_and(|name| {
                            name.starts_with("product_minecraft")
                                || name.starts_with("game_minecraft")
                        })
                })
            })
            .unwrap_or(false);
        Ok(owned)
    }

    /// Full exchange: MSA tokens → verified Mojang identity.
    pub async fn complete_login(
        &self,
        msa: &MsaTokenResponse,
    ) -> AppResult<MsaLoginOutcome> {
        let xbl = self.authenticate_xbox(&msa.access_token).await?;
        let xsts = self.authorize_xsts(&xbl.token).await?;
        let session = self.login_with_xbox(&xsts).await?;
        let profile = self.fetch_profile(&session.access_token).await?;
        let uuid = profile.uuid()?;

        let mut tokens = TokenSet::new(
            session.access_token.clone(),
            msa.refresh_token.clone(),
            Utc::now() + Duration::seconds(session.expires_in.max(0) - 60),
        );
        tokens.xuid = Some(xsts.xuid);
        tokens.client_id = Some(self.client_id.clone());

        Ok(MsaLoginOutcome {
            uuid,
            username: profile.name.clone(),
            skin: profile.active_skin(),
            tokens,
        })
    }
}

/// Everything a successful MSA sign-in produces.
#[derive(Debug, Clone)]
pub struct MsaLoginOutcome {
    pub uuid: MinecraftUuid,
    pub username: String,
    pub skin: SkinProfile,
    pub tokens: TokenSet,
}

/// Human-readable explanation for the XSTS `XErr` codes. Returns `None` for
/// codes we do not know, so the caller can fall back to the raw body.
pub fn xsts_error_message(code: u64) -> Option<&'static str> {
    Some(match code {
        2148916227 => "This Xbox account has been banned from Xbox Live.",
        2148916233 => {
            "This Microsoft account has no Xbox profile yet. Sign in to xbox.com once to \
             create one, then try again."
        }
        2148916235 => "Xbox Live is not available in this account's country or region.",
        2148916236 | 2148916237 => {
            "This account needs adult verification (a child account must be added to a \
             Microsoft family by an adult)."
        }
        2148916238 => {
            "This is a child account. An adult must add it to a Microsoft family before it \
             can play."
        }
        _ => return None,
    })
}

/// Extract `error` from an OAuth error body.
fn oauth_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

/// Extract `error_description` when present, else the raw body (truncated).
fn summarize_oauth_error(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(description) = value.get("error_description").and_then(|d| d.as_str()) {
            return match msa_error_hint(value.get("error").and_then(|e| e.as_str())) {
                Some(hint) => format!("{description}\n\n{hint}"),
                None => description.to_string(),
            };
        }
        if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
            return error.to_string();
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "no details returned".to_string()
    } else {
        trimmed.chars().take(300).collect()
    }
}

/// Skin/cape profile read straight from Mojang's public session server.
///
/// This endpoint needs no authentication and reflects the *current* skin, so a
/// Microsoft account that signed in weeks ago can still show the skin the player
/// changed on minecraft.net afterwards. It is also what lets the launcher refresh
/// skins without a full session refresh.
pub async fn fetch_skin_from_session_server(
    http: &reqwest::Client,
    uuid: MinecraftUuid,
) -> AppResult<SkinProfile> {
    const SESSION_SERVER: &str = "https://sessionserver.mojang.com";

    let response = http
        .get(format!(
            "{SESSION_SERVER}/session/minecraft/profile/{}",
            uuid.simple()
        ))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| AppError::Network(format!("session server request failed: {err}")))?;

    // 404 = the profile has no skin the server knows about; not an error.
    if !response.status().is_success() {
        return Ok(SkinProfile::default());
    }

    let payload: serde_json::Value = response.json().await?;
    let mut encoded = None;
    if let Some(properties) = payload.get("properties").and_then(|properties| properties.as_array())
    {
        for property in properties {
            if property.get("name").and_then(|name| name.as_str()) == Some("textures") {
                encoded = property
                    .get("value")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
            }
        }
    }

    let Some(encoded) = encoded else {
        return Ok(SkinProfile::default());
    };

    let decoded = decode_texture_payload(&encoded);
    let skin = decoded
        .pointer("/textures/SKIN")
        .or_else(|| decoded.pointer("/textures/skin"));
    let cape = decoded
        .pointer("/textures/CAPE")
        .or_else(|| decoded.pointer("/textures/cape"));

    Ok(SkinProfile {
        model: skin
            .and_then(|skin| skin.pointer("/metadata/model").and_then(|model| model.as_str()))
            .map(|model| {
                if model.eq_ignore_ascii_case("slim") {
                    SkinModel::Slim
                } else {
                    SkinModel::Classic
                }
            })
            .unwrap_or(SkinModel::Classic),
        skin_url: skin
            .and_then(|skin| skin.get("url").and_then(|url| url.as_str()))
            .map(str::to_string),
        cape_url: cape
            .and_then(|cape| cape.get("url").and_then(|url| url.as_str()))
            .map(str::to_string),
    })
}

/// Base64-decode a `textures` property value.
fn decode_texture_payload(value: &str) -> serde_json::Value {
    use base64::Engine;
    serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .unwrap_or_default(),
    )
    .unwrap_or(serde_json::Value::Null)
}

/// Actionable wording for the Microsoft error codes players actually hit.
///
/// A bare `AADSTS50011` tells nobody anything; these hints turn the sign-in
/// screen into something a user can act on (including the Settings → Fixes
/// remedy of registering their own client id).
fn msa_error_hint(code: Option<&str>) -> Option<&'static str> {
    Some(match code? {
        "invalid_client" => {
            "Microsoft does not accept this application id. Register your own public Azure \
             application (type “Mobile and desktop applications”, redirect URI \
             `http://localhost`, delegated permission `XboxLive.signin`) and paste its client \
             id into Settings → Fixes → Microsoft client id."
        }
        "invalid_request" | "unsupported_response_type" => {
            "The sign-in request was rejected as malformed. Make sure the client id supports a \
             `http://localhost` redirect and the `XboxLive.signin` scope."
        }
        "unauthorized_client" => {
            "This application id is not allowed to use this sign-in method. Device-code sign-in \
             is the usual casualty; browser sign-in or your own Azure app id will still work."
        }
        "invalid_grant" => "The session expired or access was revoked; sign in again.",
        "access_denied" => {
            "Access was denied before any account was selected. If a corporate policy blocks \
             it, try again from a personal account or use a device code."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> MicrosoftAuth {
        MicrosoftAuth::new(reqwest::Client::new())
    }

    #[test]
    fn authorize_url_contains_pkce_and_state() {
        let pkce = super::super::oauth::PkceCode::generate().expect("pkce");
        let url = auth()
            .authorize_url("http://127.0.0.1:1234/callback", &pkce.challenge, "state-1")
            .expect("url");
        let parsed = url::Url::parse(&url).expect("parses");

        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params.get("client_id").map(|v| v.as_ref()), Some(MSA_CLIENT_ID));
        assert_eq!(params.get("code_challenge_method").map(|v| v.as_ref()), Some("S256"));
        assert_eq!(
            params.get("code_challenge").map(|v| v.as_ref()),
            Some(pkce.challenge.as_str())
        );
        assert_eq!(params.get("state").map(|v| v.as_ref()), Some("state-1"));
        assert_eq!(
            params.get("redirect_uri").map(|v| v.as_ref()),
            Some("http://127.0.0.1:1234/callback")
        );
    }

    #[test]
    fn token_set_expires_early() {
        let response = MsaTokenResponse {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_in: 3600,
            token_type: Some("Bearer".into()),
            scope: None,
        };
        let tokens = response.into_token_set();
        assert!(tokens.can_refresh());
        // 3600s lifetime minus the 60s safety margin.
        let lifetime = (tokens.expires_at - Utc::now()).num_seconds();
        assert!((3480..=3540).contains(&lifetime), "lifetime was {lifetime}");
    }

    #[test]
    fn profile_maps_active_skin_and_cape() {
        let profile: MinecraftProfile = serde_json::from_value(serde_json::json!({
            "id": "069a79f444e94726a5befca90e38aaf5",
            "name": "Notch",
            "skins": [
                { "id": "a", "state": "INACTIVE", "url": "https://old", "variant": "classic" },
                { "id": "b", "state": "ACTIVE", "url": "https://skin", "variant": "slim" }
            ],
            "capes": [ { "id": "c", "alias": "Migrator", "url": "https://cape" } ]
        }))
        .expect("profile fixture");

        assert_eq!(profile.uuid().expect("uuid").to_string(), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        let skin = profile.active_skin();
        assert_eq!(skin.model, SkinModel::Slim);
        assert_eq!(skin.skin_url.as_deref(), Some("https://skin"));
        assert_eq!(skin.cape_url.as_deref(), Some("https://cape"));
    }

    #[test]
    fn xsts_codes_map_to_actionable_messages() {
        assert!(xsts_error_message(2148916233)
            .expect("known code")
            .contains("no Xbox profile"));
        assert!(xsts_error_message(999).is_none());
    }

    #[test]
    fn invalid_profile_id_is_an_error_not_a_panic() {
        let profile = MinecraftProfile {
            id: "not-a-uuid".into(),
            name: "X".into(),
            skins: vec![],
            capes: vec![],
        };
        assert!(profile.uuid().is_err());
    }

    #[test]
    fn oauth_errors_are_summarized() {
        // A known but self-explanatory code stays verbatim plus its hint.
        assert!(
            summarize_oauth_error(r#"{"error":"invalid_grant","error_description":"expired"}"#)
                .starts_with("expired")
        );
        assert_eq!(
            oauth_error_code(r#"{"error":"authorization_pending"}"#).as_deref(),
            Some("authorization_pending")
        );
        // Known error codes get an actionable hint appended.
        let invalid_client =
            summarize_oauth_error(r#"{"error":"invalid_client","error_description":"nope"}"#);
        assert!(invalid_client.contains("nope"));
        assert!(invalid_client.contains("Azure"));
    }
}
