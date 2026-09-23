//! sx.acc account client.
//!
//! The launcher never ships a host. `SXACC_BASE_URL` (or the Settings field)
//! is the only origin:
//!
//! * account API — `{BASE}/v1` (register, login, refresh, profile, skin)
//! * authlib root — `{BASE}/authlib/` (passed to authlib-injector)
//! * OAuth public client `sxmlauncher`, redirect `sxmlauncher://auth/callback`
//!
//! v1 is the contract. A discovery document at `GET {BASE}/v1` may name
//! different paths or turn flows off; unknown fields are ignored so a later
//! v2 document can add keys without breaking this client. When that document
//! is missing, the standard paths above are used.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::oauth::PkceCode;
use crate::error::{AppError, AppResult};
use crate::models::account::{SkinModel, SkinProfile, TokenSet};

/// Public OAuth client id registered for SXMLAUNCHER.
pub const SXACC_OAUTH_CLIENT_ID: &str = "sxmlauncher";
/// Custom-protocol redirect. Registered with the desktop deep-link plugin.
pub const SXACC_REDIRECT_URI: &str = "sxmlauncher://auth/callback";
/// Scheme of [`SXACC_REDIRECT_URI`].
pub const SXACC_SCHEME: &str = "sxmlauncher";
/// Suffix appended to Minecraft's `--versionType` (`release/sx.acc`).
pub const SXACC_VERSION_LABEL: &str = "sx.acc";

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);

/// What the sign-in UI should offer for the configured base URL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SxAccCapabilities {
    pub configured: bool,
    pub reachable: bool,
    pub password: bool,
    pub register: bool,
    pub oauth: bool,
    pub device: bool,
    pub message: Option<String>,
}

/// A finished sx.acc session, ready to store in the vault.
#[derive(Debug, Clone)]
pub struct SxAccSession {
    pub username: String,
    pub uuid: Uuid,
    pub skin: SkinProfile,
    pub tokens: TokenSet,
}

/// Device-code grant the poller needs. Not sent to the UI.
#[derive(Debug, Clone)]
pub struct SxAccDeviceGrant {
    pub device_code: String,
    pub interval_secs: i64,
    pub expires_at: DateTime<Utc>,
    pub token_url: String,
}

/// Prompt returned to the UI for a device-code sign-in.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SxAccDevicePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub expires_in: i64,
    pub interval: i64,
    pub device_code: String,
    /// Token endpoint for this grant. Round-tripped so a refreshed settings
    /// origin is not required to finish a sign-in already in progress.
    #[serde(default)]
    pub token_url: String,
}

#[derive(Debug, Clone)]
struct Endpoints {
    register: String,
    login: String,
    refresh: String,
    profile: String,
    skin: String,
    logout: String,
    authorize: String,
    token: String,
    device: Option<String>,
    scopes: String,
    password: bool,
    register_enabled: bool,
    oauth: bool,
    device_enabled: bool,
}

impl Endpoints {
    fn standard(base: &str) -> Self {
        Self {
            register: format!("{base}/v1/auth/register"),
            login: format!("{base}/v1/auth/login"),
            refresh: format!("{base}/v1/auth/refresh"),
            profile: format!("{base}/v1/profile"),
            skin: format!("{base}/v1/profile/skin"),
            logout: format!("{base}/v1/logout"),
            authorize: format!("{base}/v1/oauth/authorize"),
            token: format!("{base}/v1/oauth/token"),
            device: Some(format!("{base}/v1/oauth/device")),
            scopes: String::new(),
            password: true,
            register_enabled: true,
            oauth: true,
            device_enabled: true,
        }
    }

    fn authorize_url(&self, challenge: &str, state: &str) -> AppResult<String> {
        let mut pairs = vec![
            ("response_type", "code"),
            ("client_id", SXACC_OAUTH_CLIENT_ID),
            ("redirect_uri", SXACC_REDIRECT_URI),
            ("code_challenge", challenge),
            ("code_challenge_method", PkceCode::method()),
            ("state", state),
        ];
        if !self.scopes.is_empty() {
            pairs.push(("scope", self.scopes.as_str()));
        }
        append_query(&self.authorize, &pairs)
    }
}

/// HTTP client bound to one sx.acc origin.
#[derive(Debug, Clone)]
pub struct SxAccAuth {
    http: reqwest::Client,
    base_url: String,
}

impl SxAccAuth {
    pub fn new(http: reqwest::Client, base_url: &str) -> Self {
        Self {
            http,
            base_url: normalize_base_url(base_url).unwrap_or_default(),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `{BASE}/authlib/` — the value authlib-injector receives.
    pub fn authlib_url(&self) -> AppResult<String> {
        self.require_base()?;
        Ok(authlib_root(&self.base_url))
    }

    pub async fn capabilities(&self) -> SxAccCapabilities {
        if self.base_url.is_empty() {
            return SxAccCapabilities {
                configured: false,
                reachable: false,
                password: false,
                register: false,
                oauth: false,
                device: false,
                message: Some(
                    "Set the sx.acc base URL in Settings, or export SXACC_BASE_URL. \
                     SXMLAUNCHER does not assume a host."
                        .into(),
                ),
            };
        }
        match self.discover().await {
            Ok(endpoints) => SxAccCapabilities {
                configured: true,
                reachable: true,
                password: endpoints.password,
                register: endpoints.register_enabled,
                oauth: endpoints.oauth,
                device: endpoints.device_enabled,
                message: None,
            },
            Err(err) => SxAccCapabilities {
                configured: true,
                reachable: false,
                // The probe failed, but the standard flows may still answer.
                // The sign-in buttons stay available and report the real error.
                password: true,
                register: true,
                oauth: true,
                device: true,
                message: Some(err.to_string()),
            },
        }
    }

    pub async fn login_password(&self, username: &str, password: &str) -> AppResult<SxAccSession> {
        // Live sx.acc v2 accepts `username` only, and rejects anything longer
        // than 16 characters. Validate before discovery so an email is never
        // sent as that field.
        let username = username.trim();
        crate::auth::offline::validate_username(username)?;
        if password.is_empty() {
            return Err(AppError::Account(
                "sx.acc sign-in needs a username and a password".into(),
            ));
        }
        let endpoints = self.discover().await?;
        if !endpoints.password {
            return Err(AppError::Account(
                "this sx.acc server does not expose password sign-in".into(),
            ));
        }
        self.login_with(&endpoints, username, password).await
    }

    pub async fn register(
        &self,
        email: &str,
        password: &str,
        username: &str,
    ) -> AppResult<SxAccSession> {
        let endpoints = self.discover().await?;
        if !endpoints.register_enabled {
            return Err(AppError::Account(
                "this sx.acc server does not expose account registration".into(),
            ));
        }
        let email = email.trim();
        let username = username.trim();
        if !email.is_empty() {
            validate_email(email)?;
        }
        crate::auth::offline::validate_username(username)?;
        if password.chars().count() < 8 {
            return Err(AppError::Account(
                "sx.acc passwords must be at least 8 characters".into(),
            ));
        }

        let mut body = serde_json::Map::new();
        body.insert("username".into(), json!(username));
        body.insert("password".into(), json!(password));
        if !email.is_empty() {
            body.insert("email".into(), json!(email));
        }
        let created = self
            .post_json(&endpoints.register, &Value::Object(body))
            .await?;

        if find_string(&created, &["accessToken", "access_token", "token"]).is_some() {
            return self.finish_session(&endpoints, created).await;
        }
        // Older servers may create the account without starting a session.
        self.login_with(&endpoints, username, password).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> AppResult<SxAccSession> {
        let endpoints = self.discover().await?;
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(AppError::Unauthorized);
        }

        let json_body = json!({ "refresh_token": refresh_token });
        let refreshed = match self.post_json(&endpoints.refresh, &json_body).await {
            Ok(value) => value,
            Err(AppError::Account(message)) if message_is_missing_route(&message) => {
                self.post_token(
                    &endpoints.token,
                    &[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", refresh_token),
                        ("client_id", SXACC_OAUTH_CLIENT_ID),
                    ],
                )
                .await?
            }
            Err(AppError::Account(message)) if message_is_unauthorized(&message) => {
                return Err(AppError::Unauthorized);
            }
            Err(err) => return Err(err),
        };
        self.finish_session(&endpoints, refreshed).await
    }

    pub async fn exchange_code(&self, code: &str, verifier: &str) -> AppResult<SxAccSession> {
        let endpoints = self.discover().await?;
        let value = self
            .post_token(
                &endpoints.token,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("redirect_uri", SXACC_REDIRECT_URI),
                    ("client_id", SXACC_OAUTH_CLIENT_ID),
                    ("code_verifier", verifier),
                ],
            )
            .await?;
        self.finish_session(&endpoints, value).await
    }

    pub async fn begin_oauth(&self, challenge: &str, state: &str) -> AppResult<String> {
        let endpoints = self.discover().await?;
        if !endpoints.oauth {
            return Err(AppError::Account(
                "this sx.acc server does not expose browser sign-in".into(),
            ));
        }
        endpoints.authorize_url(challenge, state)
    }

    pub async fn begin_device(&self) -> AppResult<(SxAccDevicePrompt, SxAccDeviceGrant)> {
        let endpoints = self.discover().await?;
        let Some(device_url) = endpoints.device.clone() else {
            return Err(AppError::Account(
                "this sx.acc server does not expose device-code sign-in".into(),
            ));
        };
        if !endpoints.device_enabled {
            return Err(AppError::Account(
                "this sx.acc server does not expose device-code sign-in".into(),
            ));
        }

        let mut fields = vec![("client_id", SXACC_OAUTH_CLIENT_ID)];
        if !endpoints.scopes.is_empty() {
            fields.push(("scope", endpoints.scopes.as_str()));
        }
        let value = match self.post_token(&device_url, &fields).await {
            Ok(value) => value,
            Err(AppError::Account(message)) if message_is_missing_route(&message) => {
                // Some v1 servers publish the device grant one path deeper.
                let fallback = format!("{device_url}/code");
                self.post_token(&fallback, &fields).await?
            }
            Err(err) => return Err(err),
        };

        let device_code = find_string(&value, &["device_code", "deviceCode"])
            .ok_or_else(|| AppError::Account("sx.acc device response had no device_code".into()))?
            .to_string();
        let user_code = find_string(&value, &["user_code", "userCode"])
            .unwrap_or("------")
            .to_string();
        let verification_uri = find_string(
            &value,
            &[
                "verification_uri_complete",
                "verificationUriComplete",
                "verification_uri",
                "verificationUri",
            ],
        )
        .unwrap_or(device_url.as_str())
        .to_string();
        let expires_in = find_i64(&value, &["expires_in", "expiresIn"]).unwrap_or(900).max(30);
        let interval = find_i64(&value, &["interval"]).unwrap_or(5).clamp(1, 60);
        let message = find_string(&value, &["message"])
            .unwrap_or("Enter the code in your browser to finish sx.acc sign-in")
            .to_string();

        let prompt = SxAccDevicePrompt {
            user_code,
            verification_uri,
            message,
            expires_in,
            interval,
            device_code: device_code.clone(),
            token_url: endpoints.token.clone(),
        };
        let grant = SxAccDeviceGrant {
            device_code,
            interval_secs: interval,
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in),
            token_url: endpoints.token.clone(),
        };
        Ok((prompt, grant))
    }

    pub async fn poll_device(&self, grant: &SxAccDeviceGrant) -> AppResult<SxAccSession> {
        let endpoints = self.discover().await?;
        let mut interval = Duration::from_secs(grant.interval_secs.max(1) as u64);
        loop {
            if Utc::now() >= grant.expires_at {
                return Err(AppError::Account(
                    "the sx.acc device code expired before it was confirmed".into(),
                ));
            }
            tokio::time::sleep(interval).await;
            let result = self
                .post_token(
                    &grant.token_url,
                    &[
                        (
                            "grant_type",
                            "urn:ietf:params:oauth:grant-type:device_code",
                        ),
                        ("device_code", grant.device_code.as_str()),
                        ("client_id", SXACC_OAUTH_CLIENT_ID),
                    ],
                )
                .await;

            match result {
                Ok(value) => {
                    if let Some(error) = find_string(&value, &["error"]) {
                        match error {
                            "authorization_pending" => continue,
                            "slow_down" => {
                                interval += Duration::from_secs(5);
                                continue;
                            }
                            "expired_token" | "access_denied" => {
                                return Err(AppError::Account(format!(
                                    "sx.acc device sign-in failed: {error}"
                                )));
                            }
                            other => {
                                let detail = find_string(&value, &["error_description", "errorDescription"])
                                    .unwrap_or(other);
                                return Err(AppError::Account(format!(
                                    "sx.acc device sign-in failed: {detail}"
                                )));
                            }
                        }
                    }
                    return self.finish_session(&endpoints, value).await;
                }
                Err(AppError::Account(message))
                    if message.contains("authorization_pending") =>
                {
                    continue;
                }
                Err(AppError::Account(message)) if message.contains("slow_down") => {
                    interval += Duration::from_secs(5);
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Re-read the profile (and skin, when the server has one).
    pub async fn fetch_profile(&self, access_token: &str) -> AppResult<SxAccSession> {
        let endpoints = self.discover().await?;
        let profile = self.get_bearer(&endpoints.profile, access_token).await?;
        // Profile responses carry the player, not a fresh access token.
        let mut wrapped = profile;
        if find_string(&wrapped, &["accessToken", "access_token", "token"]).is_none() {
            if let Some(obj) = wrapped.as_object_mut() {
                obj.insert("accessToken".into(), json!(access_token));
            }
        }
        self.finish_session(&endpoints, wrapped).await
    }

    /// Best-effort skin upload. `Ok(None)` means the server has no upload route.
    pub async fn upload_skin(
        &self,
        access_token: &str,
        model: SkinModel,
        png: Vec<u8>,
    ) -> AppResult<Option<SkinProfile>> {
        let endpoints = self.discover().await?;
        let _ = model;
        let part = reqwest::multipart::Part::bytes(png)
            .file_name("skin.png")
            .mime_str("image/png")
            .map_err(|err| AppError::Account(format!("could not build the skin upload: {err}")))?;
        // Live sx.acc v2 accepts a single multipart field named `file` on PUT.
        let form = reqwest::multipart::Form::new().part("file", part);
        let response = self
            .http
            .put(&endpoints.skin)
            .bearer_auth(access_token)
            .multipart(form)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("sx.acc skin upload failed: {err}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(None);
        }
        let value = read_json(response).await?;
        let skin = extract_skin(&self.base_url, &value);
        if skin.skin_url.is_some() {
            return Ok(Some(skin));
        }
        let session = self.fetch_profile(access_token).await?;
        Ok(Some(session.skin))
    }

    /// Tell the server the refresh token is done. Failures are ignored by the caller.
    pub async fn logout(&self, access_token: &str, refresh_token: Option<&str>) -> AppResult<()> {
        let endpoints = self.discover().await?;
        let mut body = serde_json::Map::new();
        if let Some(refresh) = refresh_token {
            body.insert("refreshToken".into(), json!(refresh));
        }
        let _ = self
            .http
            .post(&endpoints.logout)
            .bearer_auth(access_token)
            .json(&Value::Object(body))
            .send()
            .await;
        Ok(())
    }

    async fn login_with(
        &self,
        endpoints: &Endpoints,
        identifier: &str,
        password: &str,
    ) -> AppResult<SxAccSession> {
        let body = json!({
            "username": identifier,
            "password": password,
        });
        let value = self.post_json(&endpoints.login, &body).await?;
        self.finish_session(endpoints, value).await
    }

    async fn finish_session(&self, endpoints: &Endpoints, value: Value) -> AppResult<SxAccSession> {
        let mut session = parse_session(&self.base_url, &value)?;
        if session.skin.skin_url.is_none() {
            if let Ok(profile) = self
                .get_bearer(&endpoints.profile, &session.tokens.access_token)
                .await
            {
                let skin = extract_skin(&self.base_url, &profile);
                if skin.skin_url.is_some() || skin.cape_url.is_some() {
                    session.skin = skin;
                }
                if session.username.is_empty() {
                    if let Some(name) = find_string(&profile, &["username", "name"]) {
                        session.username = name.to_string();
                    }
                }
            }
        }
        if session.skin.skin_url.is_none() {
            if let Ok(skin_doc) = self
                .get_bearer(&endpoints.skin, &session.tokens.access_token)
                .await
            {
                let skin = extract_skin(&self.base_url, &skin_doc);
                if skin.skin_url.is_some() || skin.cape_url.is_some() {
                    session.skin = skin;
                }
            }
        }
        Ok(session)
    }

    async fn discover(&self) -> AppResult<Endpoints> {
        self.require_base()?;
        let url = format!("{}/v1", self.base_url);
        let response = match self.http.get(&url).timeout(DISCOVERY_TIMEOUT).send().await {
            Ok(response) => response,
            Err(err) => {
                return Err(AppError::Network(format!(
                    "could not reach sx.acc at {}: {err}",
                    self.base_url
                )))
            }
        };
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Endpoints::standard(&self.base_url));
        }
        if !response.status().is_success() {
            // A v1 API with no index document sometimes answers 401/405.
            // The named routes are still the contract.
            return Ok(Endpoints::standard(&self.base_url));
        }
        let text = response.text().await.unwrap_or_default();
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Ok(Endpoints::standard(&self.base_url));
        };
        if !value.is_object() {
            return Ok(Endpoints::standard(&self.base_url));
        }
        Ok(Endpoints::from_discovery(&self.base_url, &value))
    }

    async fn post_json(&self, url: &str, body: &Value) -> AppResult<Value> {
        let response = self
            .http
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("sx.acc request to {url} failed: {err}")))?;
        read_json(response).await
    }

    async fn post_token(&self, url: &str, fields: &[(&str, &str)]) -> AppResult<Value> {
        let response = self
            .http
            .post(url)
            .form(fields)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("sx.acc token request failed: {err}")))?;
        if response.status() == reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE {
            let mut map = serde_json::Map::new();
            for (key, value) in fields {
                map.insert((*key).to_string(), json!(value));
            }
            return self.post_json(url, &Value::Object(map)).await;
        }
        read_json(response).await
    }

    async fn get_bearer(&self, url: &str, access_token: &str) -> AppResult<Value> {
        let response = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("sx.acc profile request failed: {err}")))?;
        read_json(response).await
    }

    fn require_base(&self) -> AppResult<()> {
        if self.base_url.is_empty() {
            return Err(AppError::Config(
                "sx.acc base URL is not set. Add it in Settings → Accounts, or export SXACC_BASE_URL. \
                 SXMLAUNCHER does not assume a host."
                    .into(),
            ));
        }
        Ok(())
    }
}

impl Endpoints {
    fn from_discovery(base: &str, value: &Value) -> Self {
        let mut endpoints = Self::standard(base);
        let (password, register, oauth, device) = flow_flags(value);
        endpoints.password = password;
        endpoints.register_enabled = register;
        endpoints.oauth = oauth;
        endpoints.device_enabled = device;
        endpoints.register = resolve_path(base, value, &["register", "registration"], &endpoints.register);
        endpoints.login = resolve_path(base, value, &["login", "signIn", "sign_in"], &endpoints.login);
        endpoints.refresh = resolve_path(base, value, &["refresh", "token"], &endpoints.refresh);
        endpoints.profile = resolve_path(base, value, &["profile", "me"], &endpoints.profile);
        endpoints.skin = resolve_path(base, value, &["skin"], &endpoints.skin);
        endpoints.logout = resolve_path(base, value, &["logout", "revoke"], &endpoints.logout);
        endpoints.authorize = resolve_oauth(
            base,
            value,
            &["authorizationEndpoint", "authorization_endpoint", "authorize"],
            &endpoints.authorize,
        );
        endpoints.token = resolve_oauth(
            base,
            value,
            &["tokenEndpoint", "token_endpoint", "token"],
            &endpoints.token,
        );
        if device {
            endpoints.device = Some(resolve_oauth(
                base,
                value,
                &[
                    "deviceAuthorizationEndpoint",
                    "device_authorization_endpoint",
                    "device",
                ],
                endpoints.device.as_deref().unwrap_or(""),
            ));
        } else {
            endpoints.device = None;
        }
        if let Some(scopes) = value.get("scopes").and_then(|v| v.as_array()) {
            let joined = scopes
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if !joined.is_empty() {
                endpoints.scopes = joined;
            }
        } else if let Some(scope) = find_in(value, &["scope", "scopes"]) {
            endpoints.scopes = scope.to_string();
        }
        endpoints
    }
}

/// Normalize a user-entered origin.
///
/// Empty stays empty. `localhost` without a scheme becomes `http://`. Any
/// other host without a scheme becomes `https://`. Trailing slashes are
/// removed. There is no default host.
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else if is_loopback_host(trimmed) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    let url = url::Url::parse(with_scheme.trim()).map_err(|err| err.to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!(
            "sx.acc base URL must be http or https (got {})",
            url.scheme()
        ));
    }
    if url.host_str().is_none() {
        return Err("sx.acc base URL is missing a host".into());
    }
    let mut serialized = url.to_string();
    while serialized.ends_with('/') {
        serialized.pop();
    }
    Ok(serialized)
}

/// Authlib-injector root. The trailing slash is part of the contract.
pub fn authlib_root(base: &str) -> String {
    format!("{}/authlib/", base.trim_end_matches('/'))
}

fn is_loopback_host(raw: &str) -> bool {
    let host = raw.split('/').next().unwrap_or(raw);
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1"
    )
}

fn validate_email(email: &str) -> AppResult<()> {
    let Some((local, domain)) = email.split_once('@') else {
        return Err(AppError::Account(
            "sx.acc registration needs an email address".into(),
        ));
    };
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || email.chars().any(char::is_whitespace)
    {
        return Err(AppError::Account(
            "that email address does not look usable".into(),
        ));
    }
    Ok(())
}

fn append_query(url: &str, pairs: &[(&str, &str)]) -> AppResult<String> {
    let mut url = url::Url::parse(url).map_err(|err| {
        AppError::Account(format!("sx.acc authorization endpoint is not a URL: {err}"))
    })?;
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

fn join_base(base: &str, path: &str) -> String {
    let path = path.trim();
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.trim_end_matches('/').to_string();
    }
    format!("{base}/{}", path.trim_start_matches('/'))
}

fn resolve_path(base: &str, value: &Value, keys: &[&str], fallback: &str) -> String {
    if let Some(raw) = nested_string(value, &["endpoints", "api"], keys) {
        return join_base(base, raw);
    }
    if let Some(raw) = string_field(value, keys) {
        return join_base(base, raw);
    }
    fallback.to_string()
}

fn resolve_oauth(base: &str, value: &Value, keys: &[&str], fallback: &str) -> String {
    if let Some(raw) = nested_string(value, &["oauth", "openid"], keys) {
        return join_base(base, raw);
    }
    resolve_path(base, value, keys, fallback)
}

fn nested_string<'a>(value: &'a Value, containers: &[&str], keys: &[&str]) -> Option<&'a str> {
    let obj = value.as_object()?;
    for container in containers {
        if let Some(child) = obj.get(*container) {
            if let Some(found) = string_field(child, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn flow_flags(value: &Value) -> (bool, bool, bool, bool) {
    let flows = value.get("flows").or_else(|| value.get("capabilities"));
    match flows {
        Some(Value::Array(items)) => {
            let has = |name: &str| {
                items.iter().any(|item| {
                    item.as_str().is_some_and(|raw| {
                        raw.eq_ignore_ascii_case(name)
                            || (name == "oauth" && raw.eq_ignore_ascii_case("pkce"))
                    })
                })
            };
            (has("password"), has("register"), has("oauth"), has("device"))
        }
        Some(Value::Object(map)) => {
            let flag = |name: &str| map.get(name).and_then(|v| v.as_bool()).unwrap_or(true);
            let oauth = map
                .get("oauth")
                .or_else(|| map.get("pkce"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            (flag("password"), flag("register"), oauth, flag("device"))
        }
        _ => (true, true, true, true),
    }
}

async fn read_json(response: reqwest::Response) -> AppResult<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let message = api_error(status, &body);
        if status == reqwest::StatusCode::UNAUTHORIZED || message_is_unauthorized(&message) {
            // Login failures stay as account errors (wrong password). Refresh
            // maps this text to Unauthorized at the call site.
            return Err(AppError::Account(message));
        }
        return Err(AppError::Account(message));
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body).map_err(|err| {
        AppError::Account(format!("sx.acc returned a response that is not JSON ({err})"))
    })
}

fn api_error(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(message) = find_string(
            &value,
            &[
                "error_description",
                "errorDescription",
                "message",
                "error_message",
                "errorMessage",
            ],
        ) {
            return format!("sx.acc returned HTTP {status}: {message}");
        }
        if let Some(error) = value.get("error") {
            if let Some(message) = error.as_str() {
                return format!("sx.acc returned HTTP {status}: {message}");
            }
            if let Some(message) = find_string(error, &["message", "description"]) {
                return format!("sx.acc returned HTTP {status}: {message}");
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        format!("sx.acc returned HTTP {status}")
    } else {
        let mut snippet: String = trimmed.chars().take(180).collect();
        if trimmed.chars().count() > 180 {
            snippet.push('…');
        }
        format!("sx.acc returned HTTP {status}: {snippet}")
    }
}

fn message_is_missing_route(message: &str) -> bool {
    message.contains("HTTP 404") || message.contains("HTTP 405")
}

fn message_is_unauthorized(message: &str) -> bool {
    message.contains("HTTP 401")
        || message.contains("invalid_grant")
        || message.contains("invalid_token")
}

fn parse_session(base: &str, value: &Value) -> AppResult<SxAccSession> {
    let access = find_string(value, &["accessToken", "access_token", "token", "sessionToken"])
        .ok_or_else(|| {
            AppError::Account("sx.acc response did not include an access token".into())
        })?
        .to_string();
    let refresh = find_string(value, &["refreshToken", "refresh_token"]).map(str::to_string);
    let username = find_username(value)
        .ok_or_else(|| AppError::Account("sx.acc response did not include a username".into()))?
        .to_string();
    let uuid = find_uuid(value).ok_or_else(|| {
        AppError::Account("sx.acc response did not include a player uuid".into())
    })?;
    let expires_at = expires_from(value);
    Ok(SxAccSession {
        username,
        uuid,
        skin: extract_skin(base, value),
        tokens: TokenSet::new(access, refresh, expires_at),
    })
}

fn find_uuid(value: &Value) -> Option<Uuid> {
    for key in ["uuid", "profileId", "profile_id"] {
        if let Some(raw) = find_string(value, &[key]) {
            if let Some(uuid) = parse_uuid(raw) {
                return Some(uuid);
            }
        }
    }
    // `id` is only a player uuid when it actually parses as one. Account-row
    // ids and OAuth client ids must not win.
    find_string_parsed(value, "id")
}

fn find_string_parsed(value: &Value, key: &str) -> Option<Uuid> {
    let mut found = None;
    visit(value, 0, &mut |node| {
        if found.is_some() {
            return;
        }
        if let Some(raw) = string_field(node, &[key]) {
            if let Some(uuid) = parse_uuid(raw) {
                found = Some(uuid);
            }
        }
    });
    found
}

fn parse_uuid(raw: &str) -> Option<Uuid> {
    let raw = raw.trim();
    if let Ok(uuid) = Uuid::parse_str(raw) {
        return Some(uuid);
    }
    if raw.len() == 32 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        let dashed = format!(
            "{}-{}-{}-{}-{}",
            &raw[0..8],
            &raw[8..12],
            &raw[12..16],
            &raw[16..20],
            &raw[20..32]
        );
        return Uuid::parse_str(&dashed).ok();
    }
    None
}

fn expires_from(value: &Value) -> DateTime<Utc> {
    if let Some(raw) = find_string(value, &["expiresAt", "expires_at"]) {
        if let Ok(when) = DateTime::parse_from_rfc3339(raw) {
            return when.with_timezone(&Utc);
        }
    }
    let seconds = find_i64(value, &["expiresIn", "expires_in"]).filter(|secs| *secs > 0);
    Utc::now() + chrono::Duration::seconds(seconds.unwrap_or(3600))
}

fn extract_skin(base: &str, value: &Value) -> SkinProfile {
    let mut skin = SkinProfile::default();
    visit(value, 0, &mut |node| apply_skin_node(base, node, &mut skin));
    skin
}

fn apply_skin_node(base: &str, node: &Value, skin: &mut SkinProfile) {
    if let Some(url) = string_field(node, &["skinUrl", "skin_url"]) {
        skin.skin_url = Some(absolute_url(base, url));
    }
    if let Some(url) = string_field(node, &["capeUrl", "cape_url"]) {
        skin.cape_url = Some(absolute_url(base, url));
    }
    if let Some(model) = string_field(node, &["skinModel", "skin_model", "model", "variant"]) {
        skin.model = skin_model(model);
    }
    if let Some(url) = node
        .get("skin")
        .and_then(|skin| string_field(skin, &["url", "skinUrl", "skin_url"]))
    {
        skin.skin_url = Some(absolute_url(base, url));
        if let Some(model) = node
            .get("skin")
            .and_then(|skin| string_field(skin, &["model", "variant"]))
        {
            skin.model = skin_model(model);
        }
    }
    if let Some(url) = node
        .get("cape")
        .and_then(|cape| string_field(cape, &["url", "capeUrl", "cape_url"]))
    {
        skin.cape_url = Some(absolute_url(base, url));
    }
    if let Some(skins) = node.get("skins").and_then(|v| v.as_array()) {
        if let Some(first) = skins.first() {
            if let Some(url) = string_field(first, &["url"]) {
                skin.skin_url = Some(absolute_url(base, url));
            }
            if let Some(model) = string_field(first, &["variant", "model"]) {
                skin.model = skin_model(model);
            }
        }
    }
    if let Some(textures) = node.get("textures").filter(|v| v.is_object()) {
        apply_mojang_textures(base, textures, skin);
    }
    if let Some(properties) = node.get("properties").and_then(|v| v.as_array()) {
        for property in properties {
            let name = string_field(property, &["name"]).unwrap_or("");
            if !name.eq_ignore_ascii_case("textures") {
                continue;
            }
            let Some(encoded) = string_field(property, &["value"]) else {
                continue;
            };
            let Ok(bytes) = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                encoded,
            ) else {
                continue;
            };
            let Ok(decoded) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if let Some(textures) = decoded.get("textures") {
                apply_mojang_textures(base, textures, skin);
            }
        }
    }
}

fn apply_mojang_textures(base: &str, textures: &Value, skin: &mut SkinProfile) {
    if let Some(url) = textures
        .get("SKIN")
        .or_else(|| textures.get("skin"))
        .and_then(|skin| string_field(skin, &["url"]))
    {
        skin.skin_url = Some(absolute_url(base, url));
    }
    if let Some(model) = textures
        .get("SKIN")
        .and_then(|skin| skin.get("metadata"))
        .and_then(|meta| string_field(meta, &["model"]))
    {
        skin.model = skin_model(model);
    }
    if let Some(url) = textures
        .get("CAPE")
        .or_else(|| textures.get("cape"))
        .and_then(|cape| string_field(cape, &["url"]))
    {
        skin.cape_url = Some(absolute_url(base, url));
    }
}

fn skin_model(raw: &str) -> SkinModel {
    match raw.trim().to_ascii_lowercase().as_str() {
        "slim" | "alex" | "thin" => SkinModel::Slim,
        _ => SkinModel::Classic,
    }
}

fn absolute_url(base: &str, url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:") {
        return url.to_string();
    }
    if let Ok(base) = url::Url::parse(&format!("{base}/")) {
        if let Ok(joined) = base.join(url) {
            return joined.to_string();
        }
    }
    if url.starts_with('/') {
        format!("{base}{url}")
    } else {
        format!("{base}/{url}")
    }
}

/// Player name, skipping provider labels and texture property names that
/// appear higher in the document than the real profile.
fn find_username(value: &Value) -> Option<&str> {
    let keys = ["username", "playerName", "player_name", "name"];
    let mut found = None;
    visit(value, 0, &mut |node| {
        if found.is_some() {
            return;
        }
        if let Some(raw) = string_field(node, &keys) {
            if raw != "sx.acc" && !raw.eq_ignore_ascii_case("textures") {
                found = Some(raw);
            }
        }
    });
    found
}

fn find_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let mut found = None;
    visit(value, 0, &mut |node| {
        if found.is_some() {
            return;
        }
        if let Some(raw) = string_field(node, keys) {
            found = Some(raw);
        }
    });
    found
}

fn find_in<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    find_string(value, keys)
}

fn find_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    let mut found = None;
    visit(value, 0, &mut |node| {
        if found.is_some() {
            return;
        }
        let Some(obj) = node.as_object() else {
            return;
        };
        for key in keys {
            if let Some(number) = obj.get(*key).and_then(|v| v.as_i64()) {
                found = Some(number);
                return;
            }
            if let Some(number) = obj.get(*key).and_then(|v| v.as_u64()) {
                found = Some(number as i64);
                return;
            }
        }
    });
    found
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let obj = value.as_object()?;
    for key in keys {
        if let Some(raw) = obj.get(*key).and_then(|v| v.as_str()) {
            if !raw.trim().is_empty() {
                return Some(raw.trim());
            }
        }
    }
    None
}

fn visit<'a>(value: &'a Value, depth: u8, f: &mut dyn FnMut(&'a Value)) {
    if depth > 6 {
        return;
    }
    f(value);
    match value {
        Value::Object(map) => {
            for child in map.values() {
                if child.is_object() || child.is_array() {
                    visit(child, depth + 1, f);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                if child.is_object() || child.is_array() {
                    visit(child, depth + 1, f);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[test]
    fn base_url_stays_empty_until_the_user_sets_one() {
        assert_eq!(normalize_base_url("  ").unwrap(), "");
        assert!(normalize_base_url("ftp://files.example").is_err());
    }

    #[test]
    fn base_url_strips_the_trailing_slash_and_accepts_localhost() {
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8787/").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            normalize_base_url("localhost:3000").unwrap(),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_base_url("https://accounts.example/sx/").unwrap(),
            "https://accounts.example/sx"
        );
        assert_eq!(
            authlib_root("https://accounts.example/sx"),
            "https://accounts.example/sx/authlib/"
        );
    }

    #[test]
    fn standard_routes_match_live_sxacc_v2() {
        let endpoints = Endpoints::standard("https://sx-acc.vercel.app");
        assert_eq!(
            endpoints.register,
            "https://sx-acc.vercel.app/v1/auth/register"
        );
        assert_eq!(endpoints.login, "https://sx-acc.vercel.app/v1/auth/login");
        assert_eq!(
            endpoints.refresh,
            "https://sx-acc.vercel.app/v1/auth/refresh"
        );
        assert_eq!(endpoints.profile, "https://sx-acc.vercel.app/v1/profile");
        assert_eq!(endpoints.skin, "https://sx-acc.vercel.app/v1/profile/skin");
    }

    #[test]
    fn authorize_url_is_the_public_client_with_pkce() {
        let endpoints = Endpoints::standard("http://127.0.0.1:9");
        let url = endpoints.authorize_url("chal", "st").unwrap();
        assert!(url.starts_with("http://127.0.0.1:9/v1/oauth/authorize?"));
        assert!(url.contains("client_id=sxmlauncher"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st"));
        assert!(url.contains("redirect_uri=sxmlauncher%3A%2F%2Fauth%2Fcallback"));
        assert!(!url.contains("client_secret"));
    }

    #[test]
    fn discovery_can_turn_flows_off_and_move_paths() {
        let doc = json!({
            "flows": ["password", "register"],
            "endpoints": { "login": "/v1/custom/login" },
            "oauth": { "authorizationEndpoint": "https://accounts.example/oauth/authorize" }
        });
        let endpoints = Endpoints::from_discovery("https://accounts.example", &doc);
        assert!(endpoints.password);
        assert!(endpoints.register_enabled);
        assert!(!endpoints.oauth);
        assert!(!endpoints.device_enabled);
        assert_eq!(endpoints.login, "https://accounts.example/v1/custom/login");
        assert_eq!(
            endpoints.authorize,
            "https://accounts.example/oauth/authorize"
        );
    }

    #[test]
    fn session_parser_accepts_camel_case_and_a_relative_skin() {
        let value = json!({
            "accessToken": "acc",
            "refreshToken": "ref",
            "expiresIn": 120,
            "user": {
                "username": "Steve",
                "uuid": "11111111-2222-3333-4444-555555555555",
                "skinUrl": "/textures/steve.png",
                "skinModel": "slim"
            }
        });
        let session = parse_session("http://127.0.0.1:8787", &value).unwrap();
        assert_eq!(session.username, "Steve");

        let labeled = json!({
            "name": "sx.acc",
            "access_token": "acc",
            "profile": {
                "id": "11111111-2222-3333-4444-555555555555",
                "username": "Steve"
            },
            "properties": [{ "name": "textures", "value": "e30=" }]
        });
        let labeled = parse_session("http://127.0.0.1:8787", &labeled).unwrap();
        assert_eq!(labeled.username, "Steve");
        assert_eq!(session.tokens.access_token, "acc");
        assert_eq!(session.tokens.refresh_token.as_deref(), Some("ref"));
        assert_eq!(
            session.skin.skin_url.as_deref(),
            Some("http://127.0.0.1:8787/textures/steve.png")
        );
        assert_eq!(session.skin.model, SkinModel::Slim);
    }

    #[test]
    fn session_parser_accepts_the_live_v2_profile_document() {
        let value = json!({
            "access_token": "acc",
            "refresh_token": "ref",
            "token_type": "Bearer",
            "expires_in": 900,
            "profile": {
                "id": "9130a0bb-1e4f-421b-b1f3-b6023c513f30",
                "uuid": "9130a0bb1e4f421bb1f3b6023c513f30",
                "username": "Newbie",
                "email": "new@example.com",
                "skinModel": "steve",
                "skinUrl": null
            }
        });
        let session = parse_session("https://sx-acc.vercel.app", &value).unwrap();
        assert_eq!(session.username, "Newbie");
        assert_eq!(session.tokens.access_token, "acc");
        assert_eq!(session.tokens.refresh_token.as_deref(), Some("ref"));
        assert_eq!(
            session.uuid.to_string(),
            "9130a0bb-1e4f-421b-b1f3-b6023c513f30"
        );
        assert!(session.skin.skin_url.is_none());
        assert_eq!(session.skin.model, SkinModel::Classic);
    }

    #[test]
    fn session_parser_accepts_yggdrasil_profiles_and_texture_properties() {
        let textures = json!({
            "textures": {
                "SKIN": {
                    "url": "https://textures.example/skin.png",
                    "metadata": { "model": "slim" }
                },
                "CAPE": { "url": "https://textures.example/cape.png" }
            }
        });
        let encoded = base64::engine::general_purpose::STANDARD.encode(textures.to_string());
        let value = json!({
            "accessToken": "yg",
            "selectedProfile": {
                "id": "11111111222233334444555555555555",
                "name": "Alex"
            },
            "user": {
                "properties": [{ "name": "textures", "value": encoded }]
            }
        });
        let session = parse_session("http://127.0.0.1:9", &value).unwrap();
        assert_eq!(session.username, "Alex");
        assert_eq!(
            session.uuid.to_string(),
            "11111111-2222-3333-4444-555555555555"
        );
        assert_eq!(
            session.skin.skin_url.as_deref(),
            Some("https://textures.example/skin.png")
        );
        assert_eq!(
            session.skin.cape_url.as_deref(),
            Some("https://textures.example/cape.png")
        );
        assert_eq!(session.skin.model, SkinModel::Slim);
    }

    #[tokio::test]
    async fn password_login_and_register_talk_to_v1() {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_v1(hits.clone()).await;
        let auth = SxAccAuth::new(test_http(), &base);

        let session = auth.login_password("Ada", "correct-horse").await.unwrap();
        assert_eq!(session.username, "Ada");
        assert_eq!(
            session.skin.skin_url.as_deref(),
            Some("https://textures.example/ada.png")
        );
        assert_eq!(auth.authlib_url().unwrap(), format!("{base}/authlib/"));

        let rejected = auth
            .login_password("ada@example.com", "correct-horse")
            .await
            .unwrap_err();
        assert!(
            rejected.to_string().contains("16")
                || rejected.to_string().contains("underscores"),
            "{rejected}"
        );

        let created = auth
            .register("new@example.com", "long-enough", "Newbie")
            .await
            .unwrap();
        assert_eq!(created.username, "Newbie");

        let png = vec![0x89, 0x50, 0x4e, 0x47];
        auth.upload_skin("acc-token", SkinModel::Classic, png)
            .await
            .unwrap()
            .expect("skin upload");

        let recorded = hits.lock().await.clone();
        assert!(recorded.iter().any(|(method, path, body)| {
            method == "POST"
                && path == "/v1/auth/login"
                && body.contains("\"username\":\"Ada\"")
                && body.contains("password")
                && !body.contains("\"login\"")
                && !body.contains("ada@example.com")
        }));
        assert!(recorded.iter().any(|(method, path, body)| {
            method == "POST"
                && path == "/v1/auth/register"
                && body.contains("\"username\":\"Newbie\"")
                && body.contains("new@example.com")
                && !body.contains("clientId")
        }));
        assert!(recorded.iter().any(|(method, path, body)| {
            method == "PUT" && path == "/v1/profile/skin" && body.contains("name=\"file\"")
        }));
        assert!(!recorded
            .iter()
            .any(|(_, path, _)| path == "/v1/login" || path == "/v1/register"));
    }

    #[tokio::test]
    async fn oauth_callback_uses_the_public_client_and_pkce() {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_v1(hits.clone()).await;
        let db = crate::store::Database::open_in_memory().unwrap();
        let mut config = crate::auth::ProviderConfig::default();
        config.sxacc_base_url = base.clone();
        let manager = crate::auth::AccountManager::new_with_config(
            db,
            std::sync::Arc::new(crate::auth::vault::MemoryVault::new()),
            config,
        );

        let pending = manager.begin_sxacc_login().await.unwrap();
        assert!(pending.authorize_url.contains("client_id=sxmlauncher"));
        assert!(pending
            .authorize_url
            .contains("redirect_uri=sxmlauncher%3A%2F%2Fauth%2Fcallback"));
        assert!(pending.authorize_url.contains("code_challenge="));
        assert_eq!(pending.redirect_uri, SXACC_REDIRECT_URI);
        let state = pending.state.clone();
        crate::auth::deliver_oauth_callback(&format!(
            "sxmlauncher://auth/callback?code=auth-code&state={state}"
        ));
        // A mismatched scheme must not complete this sign-in.
        crate::auth::deliver_oauth_callback(&format!(
            "https://evil.example/callback?code=nope&state={state}"
        ));

        let summary = manager.complete_sxacc_login(pending).await.unwrap();
        assert_eq!(summary.username, "Ada");
        assert_eq!(summary.provider, crate::models::account::AccountProvider::SxAcc);

        let identity = manager.launch_identity(summary.id).await.unwrap();
        let authlib = format!("{base}/authlib/");
        assert_eq!(identity.authlib_url.as_deref(), Some(authlib.as_str()));
        assert_eq!(identity.version_type(Some("release")), "release/sx.acc");
        assert_eq!(identity.user_type, crate::models::account::UserType::Mojang);
        assert!(!identity.offline);

        let recorded = hits.lock().await.clone();
        let token = recorded
            .iter()
            .find(|(_, path, _)| path == "/v1/oauth/token")
            .expect("token exchange");
        assert!(token.2.contains("client_id=sxmlauncher"));
        assert!(token.2.contains("code_verifier="));
        assert!(token.2.contains("code=auth-code"));
        assert!(token.2.contains("sxmlauncher"));
        assert!(!token.2.contains("client_secret"));
    }

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    async fn spawn_v1(hits: Arc<Mutex<Vec<(String, String, String)>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let hits = hits.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    loop {
                        let n = socket.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if request_is_complete(&buf) || buf.len() > 64 * 1024 {
                            break;
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let request_line = req.lines().next().unwrap_or("");
                    let mut parts = request_line.split_whitespace();
                    let method = parts.next().unwrap_or("");
                    let path = parts
                        .next()
                        .unwrap_or("/")
                        .split('?')
                        .next()
                        .unwrap_or("/");
                    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                    hits.lock()
                        .await
                        .push((method.to_string(), path.to_string(), body.clone()));
                    let (status, content_type, payload) = route(method, path, &body);
                    let response = format!(
                        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        reason(status),
                        payload.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        tokio::task::yield_now().await;
        format!("http://127.0.0.1:{port}")
    }

    fn route(method: &str, path: &str, _body: &str) -> (u16, &'static str, String) {
        let player = |name: &str| {
            json!({
                "access_token": "acc-token",
                "refresh_token": "ref-token",
                "token_type": "Bearer",
                "expires_in": 900,
                "profile": {
                    "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "uuid": "aaaaaaaabbbbccccddddeeeeeeeeeeee",
                    "username": name,
                    "skinModel": "steve",
                    "skinUrl": "https://textures.example/ada.png"
                }
            })
            .to_string()
        };
        match (method, path) {
            // Live sx.acc answers GET /v1 with the website, not a discovery document.
            ("GET", "/v1") => (200, "text/html", "<!doctype html><title>sx.acc</title>".into()),
            ("POST", "/v1/auth/login") => (200, "application/json", player("Ada")),
            ("POST", "/v1/auth/register") => (201, "application/json", player("Newbie")),
            ("POST", "/v1/oauth/token") => (200, "application/json", player("Ada")),
            ("POST", "/v1/auth/refresh") => (200, "application/json", player("Ada")),
            ("GET", "/v1/profile") => (200, "application/json", player("Ada")),
            ("PUT", "/v1/profile/skin") => (200, "application/json", player("Ada")),
            _ => (
                404,
                "application/json",
                json!({ "message": "missing" }).to_string(),
            ),
        }
    }

    fn request_is_complete(buf: &[u8]) -> bool {
        let Some(split) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let header = String::from_utf8_lossy(&buf[..split]);
        let length = header.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        });
        match length {
            Some(length) => buf.len() >= split + 4 + length,
            None => true,
        }
    }

    fn reason(status: u16) -> &'static str {
        match status {
            200 => "OK",
            201 => "Created",
            404 => "Not Found",
            _ => "Error",
        }
    }
}
