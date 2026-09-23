//! Application paths and user settings.
//!
//! Layout under the OS app-data directory:
//!
//! ```text
//! <app_data>/
//!   sxmlauncher.db        SQLite: accounts, instances index, settings, cache
//!   settings.json         user-editable mirror of the settings table
//!   instances/<uuid>/     isolated game environments
//!   shared/               assets, libraries, versions (dedup across instances)
//!   java/                 managed JDK runtimes
//!   cache/downloads/      content-addressed download cache (sha1 named)
//!   logs/
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::models::instance::{InstancePaths, MemorySettings};

/// Default public client id for the Microsoft/Xbox Live OAuth flow.
///
/// This is the long-standing "Minecraft" public client that community launchers
/// use: it is registered for the `XboxLive.signin` scope and for a
/// `http://localhost` loopback redirect. Microsoft occasionally restricts access
/// for third-party apps, so the value is user-editable from Settings → Fixes.
pub const MSA_DEFAULT_CLIENT_ID: &str = "00000000402b5328";
/// Default public client id for Ely.by OAuth.
pub const ELYBY_DEFAULT_CLIENT_ID: &str = "sxmlauncher3";
/// Port the Ely.by loopback callback listens on.
///
/// It is fixed (not ephemeral) because Ely.by requires the `redirect_uri` to
/// match the application registration *exactly*, and a random port can never be
/// registered up front.
pub const ELYBY_DEFAULT_REDIRECT_PORT: u16 = 25564;
/// Port used for LAN world discovery beacons.
pub const DEFAULT_LAN_PORT: u16 = 44511;
/// Public relay used when a direct punch is not possible.
pub const DEFAULT_RELAY_URL: &str = "wss://relay.sxmlauncher.dev";

fn default_mqtt_broker() -> String {
    crate::network::mqtt::DEFAULT_MQTT_BROKER.to_string()
}

fn default_mqtt_port() -> u16 {
    crate::network::mqtt::DEFAULT_MQTT_PORT
}

/// Folder prefix for launcher-managed JDKs (`<app_data>/java/temurin-21`).
pub const MANAGED_JAVA_PREFIX: &str = "temurin-";
/// Copy `source` into `directory`, keeping only the file name.
pub fn import_wallpaper_file(directory: &Path, source: &Path) -> AppResult<PathBuf> {
    if source.starts_with(directory) {
        return Ok(source.to_path_buf());
    }
    if !source.is_file() {
        return Err(AppError::Config(format!(
            "background file does not exist: {}",
            source.display()
        )));
    }
    std::fs::create_dir_all(directory)?;
    let file_name = source.file_name().ok_or_else(|| {
        AppError::Config(format!(
            "background path has no file name: {}",
            source.display()
        ))
    })?;
    let destination = directory.join(file_name);
    if destination != source {
        std::fs::copy(source, &destination)?;
    }
    Ok(destination)
}

/// File name of the Ely.by authlib-injector agent jar.
pub const AUTHLIB_INJECTOR_JAR: &str = "authlib-injector.jar";
/// Maximum size of a server icon we accept into a Redis listing.
pub const MAX_ICON_BYTES: usize = 64 * 1024;

/// Resolved on-disk locations. Cheap to clone, everything hangs off `root`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub root: PathBuf,
    pub instances: PathBuf,
    pub shared: PathBuf,
    pub java: PathBuf,
    pub cache: PathBuf,
    pub downloads: PathBuf,
    pub logs: PathBuf,
    pub database_file: PathBuf,
    pub settings_file: PathBuf,
}

impl AppPaths {
    /// Derive every path from the OS app-data directory Tauri resolves.
    pub fn resolve(app: &AppHandle) -> AppResult<Self> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|err| AppError::Config(format!("cannot resolve app data dir: {err}")))?;
        Ok(Self::from_root(root))
    }

    /// Build the layout from an explicit root (tests, portable mode).
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let shared = root.join("shared");
        let cache = root.join("cache");
        Self {
            instances: root.join("instances"),
            shared,
            java: root.join("java"),
            downloads: cache.join("downloads"),
            cache,
            logs: root.join("logs"),
            database_file: root.join("sxmlauncher.db"),
            settings_file: root.join("settings.json"),
            root,
        }
    }

    /// Create every directory the app assumes exists. Idempotent.
    pub fn ensure(&self) -> AppResult<()> {
        for dir in [
            &self.root,
            &self.instances,
            &self.shared,
            &self.java,
            &self.cache,
            &self.downloads,
            &self.logs,
            &self.assets_indexes(),
            &self.asset_objects(),
            &self.libraries(),
            &self.versions(),
            &self.natives(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Shared `assets/indexes` (one JSON per asset index id).
    pub fn assets_indexes(&self) -> PathBuf {
        self.shared.join("assets").join("indexes")
    }

    /// Shared `assets/objects/<first two hex chars>/<hash>`.
    pub fn asset_objects(&self) -> PathBuf {
        self.shared.join("assets").join("objects")
    }

    pub fn libraries(&self) -> PathBuf {
        self.shared.join("libraries")
    }

    pub fn versions(&self) -> PathBuf {
        self.shared.join("versions")
    }

    /// Extracted native libraries, keyed by version id.
    pub fn natives(&self) -> PathBuf {
        self.shared.join("natives")
    }

    pub fn version_dir(&self, version_id: &str) -> PathBuf {
        self.versions().join(version_id)
    }

    pub fn version_json(&self, version_id: &str) -> PathBuf {
        self.version_dir(version_id)
            .join(format!("{version_id}.json"))
    }

    /// Per-instance layout helper.
    pub fn instance(&self, id: uuid::Uuid) -> InstancePaths {
        InstancePaths::new(self.instances.clone(), id)
    }

    /// Content-addressed cache path for a download (`sha1` or `sha512`).
    pub fn cached_download(&self, hash: &str) -> PathBuf {
        let shard = hash.get(0..2).unwrap_or("00");
        self.downloads.join(shard).join(hash)
    }

    /// Managed JDK install directory for a major version.
    pub fn managed_java(&self, major: u8) -> PathBuf {
        self.java.join(format!("{MANAGED_JAVA_PREFIX}{major}"))
    }

    /// User wallpaper copies. The asset protocol only serves files under app data.
    pub fn wallpapers(&self) -> PathBuf {
        self.root.join("wallpapers")
    }

    /// authlib-injector agent jar (Ely.by and other Yggdrasil servers).
    ///
    /// Kept in the app root rather than per instance so it is downloaded once.
    pub fn authlib_injector(&self) -> PathBuf {
        self.root.join(AUTHLIB_INJECTOR_JAR)
    }

    pub fn log_file(&self, name: &str) -> PathBuf {
        self.logs.join(format!("{name}.log"))
    }

    /// Copy a user-picked image or video into `wallpapers/` so the webview can
    /// load it through the asset protocol. Files outside app data are refused
    /// by the protocol scope even when CSP allows `media-src`.
    pub fn import_wallpaper(&self, source: &Path) -> AppResult<PathBuf> {
        import_wallpaper_file(&self.wallpapers(), source)
    }

    /// `true` when the root lives inside a portable folder (no OS app data).
    pub fn is_portable(&self) -> bool {
        std::env::var_os("SXML_PORTABLE").is_some()
    }

    pub fn describe(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("root", self.root.clone()),
            ("instances", self.instances.clone()),
            ("shared", self.shared.clone()),
            ("java", self.java.clone()),
            ("cache", self.cache.clone()),
            ("database", self.database_file.clone()),
        ]
    }
}

/// User-configurable settings, persisted to SQLite and mirrored to JSON.
///
/// Secrets (`curseforge_api_key`) are persisted in the credential vault instead
/// and only surfaced to the UI as a boolean `has_*` flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    // --- downloads ------------------------------------------------------
    /// Parallel mod/asset downloads (tokio semaphore permits).
    pub max_concurrent_downloads: u32,
    /// Hash verification is always on; this only controls whether mismatches
    /// abort the job or are re-downloaded once first.
    pub re_download_on_hash_mismatch: bool,
    /// Segmented download for large files (client jar, assets).
    pub enable_range_requests: bool,
    pub keep_download_cache: bool,

    // --- game defaults --------------------------------------------------
    pub default_memory: MemorySettings,
    /// Auto-select (and download) the JDK a version requires.
    pub auto_provision_java: bool,
    /// Reuse a compatible JDK that is already on the machine instead of
    /// downloading a managed one. Only a genuinely missing runtime is fetched.
    pub prefer_system_java: bool,
    /// Extra folders scanned for JDKs (Settings → Fixes).
    pub java_extra_roots: Vec<String>,

    // --- accounts / providers -------------------------------------------
    /// Azure public client id used for the Microsoft sign-in.
    pub msa_client_id: String,
    /// Ely.by OAuth application id.
    pub elyby_client_id: String,
    /// Ely.by OAuth secret. Only a custom web application needs one; the
    /// public desktop client `sxmlauncher3` signs in without it.
    pub elyby_client_secret: Option<String>,
    /// Exact loopback redirect URI registered for the Ely.by application.
    pub elyby_redirect_uri: String,
    /// sx.acc origin (`{BASE}/v1` account API, `{BASE}/authlib/` injector).
    /// Empty until the user sets it. `SXACC_BASE_URL` overrides this.
    pub sxacc_base_url: String,

    // --- p2p / hosting --------------------------------------------------
    pub redis_url: String,
    /// Embedded directory broker (client-side global browser; no server of
    /// ours is required — retained messages on a public MQTT broker carry the
    /// listings). Empty host disables the embedded directory.
    #[serde(default = "default_mqtt_broker")]
    pub mqtt_broker: String,
    #[serde(default = "default_mqtt_port")]
    pub mqtt_port: u16,
    pub relay_url: Option<String>,
    pub stun_servers: Vec<String>,
    /// Publish hosted worlds to the global browser by default.
    pub share_by_default: bool,
    /// Also publish non-private interface addresses on hosted sessions.
    /// Loopback and RFC1918 addresses are always published, so a second
    /// launcher on this PC can connect when the relay hostname does not resolve.
    /// On by default.
    pub expose_lan_endpoints: bool,
    pub max_hosted_players: u32,
    pub host_password: Option<String>,
    /// Try to reach the Redis directory at startup. Off = LAN-only launcher
    /// with zero infrastructure to run.
    pub directory_enabled: bool,
    /// Announce (and listen for) worlds on the local network.
    pub lan_discovery: bool,
    /// UDP port used for LAN beacons.
    pub lan_port: u16,
    /// Watch the game log for "Started serving on <port>" and publish it
    /// automatically, so hosting needs no manual port entry.
    pub lan_auto_detect: bool,

    // --- ui -------------------------------------------------------------
    pub theme: String,
    pub accent: String,
    pub reduce_motion: bool,
    pub minimize_to_tray_on_launch: bool,
    pub close_to_tray: bool,
    /// `aurora`, `image` or `video`.
    pub ui_background_kind: String,
    /// Absolute path to a background image or video the user imported.
    pub ui_background_path: Option<String>,
    /// 0.0 – 1.0 layer opacity for the custom background.
    pub ui_background_opacity: f32,
    /// Gaussian blur (px) applied to the custom background.
    pub ui_background_blur: u32,
    /// Swatch id: `purple`, `cyan`, `magenta`, `emerald`, `amber`, `silver`, or `#rrggbb`.
    pub ui_accent: String,
    /// Enable page/element animations.
    pub ui_animations: bool,
    /// Compact card density on the Play page.
    pub ui_compact: bool,

    // --- misc -----------------------------------------------------------
    pub curseforge_api_key: Option<String>,
    pub last_selected_instance: Option<uuid::Uuid>,
    pub analytics_enabled: bool,
    /// Discord application id for Rich Presence. Empty disables presence.
    /// Not a secret — it is the public id from the Discord Developer Portal.
    /// `SXML_DISCORD_APPLICATION_ID` overrides this when set.
    pub discord_application_id: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 12,
            re_download_on_hash_mismatch: true,
            enable_range_requests: true,
            keep_download_cache: true,
            default_memory: MemorySettings::default(),
            auto_provision_java: true,
            prefer_system_java: true,
            java_extra_roots: Vec::new(),
            msa_client_id: MSA_DEFAULT_CLIENT_ID.to_string(),
            elyby_client_id: ELYBY_DEFAULT_CLIENT_ID.to_string(),
            // Never bake private OAuth secrets into the binary — load from
            // `SXML_ELYBY_CLIENT_SECRET` / Settings.
            elyby_client_secret: None,
            elyby_redirect_uri: format!(
                "http://localhost:{ELYBY_DEFAULT_REDIRECT_PORT}/elyby/callback"
            ),
            sxacc_base_url: String::new(),
            redis_url: "redis://127.0.0.1:6379/0".to_string(),
            mqtt_broker: crate::network::mqtt::DEFAULT_MQTT_BROKER.to_string(),
            mqtt_port: crate::network::mqtt::DEFAULT_MQTT_PORT,
            relay_url: Some(DEFAULT_RELAY_URL.to_string()),
            stun_servers: vec![
                "stun.l.google.com:19302".to_string(),
                "stun.cloudflare.com:3478".to_string(),
            ],
            share_by_default: true,
            expose_lan_endpoints: true,
            max_hosted_players: 8,
            host_password: None,
            directory_enabled: true,
            lan_discovery: true,
            lan_port: DEFAULT_LAN_PORT,
            lan_auto_detect: true,
            theme: "dark".to_string(),
            accent: "purple".to_string(),
            reduce_motion: false,
            minimize_to_tray_on_launch: false,
            close_to_tray: true,
            ui_background_kind: "aurora".to_string(),
            ui_background_path: None,
            ui_background_opacity: 0.35,
            ui_background_blur: 0,
            ui_accent: "purple".to_string(),
            ui_animations: true,
            ui_compact: false,
            curseforge_api_key: None,
            last_selected_instance: None,
            analytics_enabled: false,
            discord_application_id: String::new(),
        }
    }
}

impl AppSettings {
    /// Overlay environment overrides (useful for dev and CI runs).
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(url) = std::env::var("SXML_REDIS_URL") {
            self.redis_url = url;
        }
        if let Ok(url) = std::env::var("SXML_RELAY_URL") {
            self.relay_url = Some(url);
        }
        if let Ok(servers) = std::env::var("SXML_STUN_SERVERS") {
            let parsed: Vec<String> = servers
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !parsed.is_empty() {
                self.stun_servers = parsed;
            }
        }
        if let Ok(key) = std::env::var("SXML_CURSEFORGE_API_KEY") {
            if !key.is_empty() {
                self.curseforge_api_key = Some(key);
            }
        }
        if let Ok(id) = std::env::var("SXML_MSA_CLIENT_ID") {
            if !id.trim().is_empty() {
                self.msa_client_id = id;
            }
        }
        if let Ok(id) = std::env::var("SXML_ELYBY_CLIENT_ID") {
            if !id.trim().is_empty() {
                self.elyby_client_id = id;
            }
        }
        if let Ok(secret) = std::env::var("SXML_ELYBY_CLIENT_SECRET") {
            if !secret.trim().is_empty() {
                self.elyby_client_secret = Some(secret);
            }
        }
        if let Ok(uri) = std::env::var("SXML_ELYBY_REDIRECT_URI") {
            if !uri.trim().is_empty() {
                self.elyby_redirect_uri = uri;
            }
        }
        if let Ok(url) = std::env::var("SXACC_BASE_URL") {
            if !url.trim().is_empty() {
                self.sxacc_base_url = url;
            }
        }
        if let Ok(id) = std::env::var("SXML_DISCORD_APPLICATION_ID") {
            if !id.trim().is_empty() {
                self.discord_application_id = id;
            }
        }
        self
    }

    /// Clamp values that come from the UI/JSON so nothing downstream explodes.
    pub fn sanitized(mut self) -> Self {
        self.max_concurrent_downloads = self.max_concurrent_downloads.clamp(1, 64);
        self.max_hosted_players = self.max_hosted_players.clamp(0, 128);
        self.default_memory = self.default_memory.sanitized();
        if self.stun_servers.is_empty() {
            self.stun_servers = AppSettings::default().stun_servers;
        }
        if !self.redis_url.contains("://") {
            self.redis_url = AppSettings::default().redis_url;
        }
        // Providers: an empty id can never authenticate, so fall back to the
        // documented defaults instead of failing with an opaque OAuth error.
        if self.msa_client_id.trim().is_empty() {
            self.msa_client_id = MSA_DEFAULT_CLIENT_ID.to_string();
        }
        if self.elyby_client_id.trim().is_empty() {
            self.elyby_client_id = ELYBY_DEFAULT_CLIENT_ID.to_string();
        }
        if self
            .elyby_client_secret
            .as_deref()
            .is_some_and(|secret| secret.trim().is_empty())
        {
            self.elyby_client_secret = None;
        }
        if !self.elyby_redirect_uri.starts_with("http://") {
            self.elyby_redirect_uri = AppSettings::default().elyby_redirect_uri;
        }
        self.sxacc_base_url =
            crate::auth::sxacc::normalize_base_url(&self.sxacc_base_url).unwrap_or_default();
        // UI: an unknown background kind or accent would leave the window with no
        // styling at all, so both are whitelisted here rather than in the CSS.
        if !["aurora", "image", "video"].contains(&self.ui_background_kind.as_str()) {
            self.ui_background_kind = "aurora".to_string();
        }
        if self.ui_background_kind != "aurora"
            && self
                .ui_background_path
                .as_deref()
                .is_none_or(|path| path.trim().is_empty())
        {
            self.ui_background_kind = "aurora".to_string();
        }
        // Six swatches, a legacy alias, or a live `#rrggbb` from the picker.
        self.ui_accent = canonical_ui_accent(&self.ui_accent);
        self.ui_background_opacity = self.ui_background_opacity.clamp(0.0, 1.0);
        self.ui_background_blur = self.ui_background_blur.min(40);
        // 0 means "let the OS pick a free port for the beacon socket".
        self.lan_port = if self.lan_port == 0 {
            DEFAULT_LAN_PORT
        } else {
            self.lan_port
        };
        self.java_extra_roots.retain(|root| !root.trim().is_empty());
        self.discord_application_id = self.discord_application_id.trim().to_string();
        self
    }

    /// Load `settings.json` when present, otherwise defaults.
    pub fn load(paths: &AppPaths) -> AppResult<Self> {
        if !paths.settings_file.exists() {
            return Ok(Self::default().with_env_overrides().sanitized());
        }
        let raw = std::fs::read_to_string(&paths.settings_file)?;
        let settings: Self = serde_json::from_str(&raw).unwrap_or_default();
        Ok(settings.with_env_overrides().sanitized())
    }

    /// Persist the mirror file (SQLite keeps the authoritative copy).
    pub fn save(&self, paths: &AppPaths) -> AppResult<()> {
        let serialized = serde_json::to_string_pretty(self)?;
        write_atomic(&paths.settings_file, serialized.as_bytes())
    }
}

fn canonical_ui_accent(value: &str) -> String {
    match value {
        "purple" | "violet" => "purple".to_string(),
        "cyan" => "cyan".to_string(),
        "magenta" | "fuchsia" => "magenta".to_string(),
        "emerald" => "emerald".to_string(),
        "amber" => "amber".to_string(),
        "silver" => "silver".to_string(),
        other if is_css_hex(other) => other.to_string(),
        _ => "purple".to_string(),
    }
}

fn is_css_hex(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('#') else {
        return false;
    };
    rest.len() == 6 && rest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Write a file via a temporary sibling then rename, so a crash mid-write can
/// never leave a truncated JSON/`instance.json`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_else(|| "tmp".into())
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_derived_from_root() {
        let paths = AppPaths::from_root("/tmp/sxml-root");
        assert!(paths.instances.ends_with("instances"));
        assert!(paths
            .version_json("1.20.1")
            .ends_with("versions/1.20.1/1.20.1.json"));
        assert_eq!(paths.managed_java(21).file_name().unwrap(), "temurin-21");
    }

    #[test]
    fn download_cache_is_sharded_by_hash_prefix() {
        let paths = AppPaths::from_root("/tmp/sxml-root");
        let sharded = paths.cached_download("abcdef123456");
        assert!(sharded.to_string_lossy().contains("ab"));
    }

    #[test]
    fn settings_sanitizes_hostile_values() {
        let mut settings = AppSettings::default();
        settings.max_concurrent_downloads = 0;
        settings.default_memory = MemorySettings {
            min_mb: 9000,
            max_mb: 1024,
        };
        let sanitized = settings.sanitized();
        assert_eq!(sanitized.max_concurrent_downloads, 1);
        assert!(sanitized.default_memory.max_mb >= sanitized.default_memory.min_mb);
    }

    #[test]
    fn legacy_settings_blob_keeps_working_and_gains_defaults() {
        // A settings.json written before the Fixes/appearance fields existed.
        let legacy = serde_json::json!({
            "maxConcurrentDownloads": 4,
            "redisUrl": "redis://example:6379/0"
        });
        let parsed: AppSettings = serde_json::from_value(legacy).expect("legacy blob parses");

        assert_eq!(parsed.max_concurrent_downloads, 4);
        assert_eq!(parsed.redis_url, "redis://example:6379/0");
        // Newly added fields fall back to their defaults instead of resetting
        // the whole struct (which would silently wipe the user's settings).
        assert_eq!(parsed.msa_client_id, MSA_DEFAULT_CLIENT_ID);
        assert_eq!(parsed.elyby_client_id, ELYBY_DEFAULT_CLIENT_ID);
        assert!(parsed.prefer_system_java);
        assert!(parsed.lan_discovery);
        assert_eq!(parsed.ui_background_kind, "aurora");
        assert!(parsed.discord_application_id.is_empty());
        assert!(parsed.sxacc_base_url.is_empty());
    }

    #[test]
    fn sanitizer_repairs_provider_and_appearance_values() {
        let mut settings = AppSettings::default();
        settings.msa_client_id = "   ".into();
        settings.elyby_redirect_uri = "not-a-url".into();
        settings.ui_background_kind = "video".into();
        settings.ui_background_path = None;
        settings.ui_accent = "chartreuse".into();
        settings.ui_background_opacity = 4.0;
        settings.ui_background_blur = 900;
        settings.sxacc_base_url = "http://127.0.0.1:8787/".into();

        let sanitized = settings.sanitized();
        assert_eq!(sanitized.msa_client_id, MSA_DEFAULT_CLIENT_ID);
        assert!(sanitized.elyby_redirect_uri.starts_with("http://"));
        assert_eq!(sanitized.sxacc_base_url, "http://127.0.0.1:8787");
        let mut rejected = AppSettings::default();
        rejected.sxacc_base_url = "ftp://nope".into();
        assert!(rejected.sanitized().sxacc_base_url.is_empty());
        // A custom background without a file falls back to the built-in aurora.
        assert_eq!(sanitized.ui_background_kind, "aurora");
        assert_eq!(sanitized.ui_accent, "purple");
        assert_eq!(sanitized.ui_background_opacity, 1.0);
        assert_eq!(sanitized.ui_background_blur, 40);
    }

    #[test]
    fn wallpaper_import_copies_into_app_data() {
        let root = std::env::temp_dir().join(format!("sxml-wall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source_dir = root.join("picked");
        std::fs::create_dir_all(&source_dir).expect("mkdir");
        let source = source_dir.join("loop.mp4");
        std::fs::write(&source, b"video-bytes").expect("write");

        let paths = AppPaths::from_root(root.join("app"));
        let imported = paths.import_wallpaper(&source).expect("copy");
        assert!(imported.starts_with(paths.wallpapers()));
        assert_eq!(std::fs::read(&imported).expect("read"), b"video-bytes");
        // A second save of the copied path must not nest another copy.
        let again = paths.import_wallpaper(&imported).expect("already inside");
        assert_eq!(again, imported);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sanitizer_keeps_a_custom_accent_hex() {
        let mut settings = AppSettings::default();
        settings.ui_accent = "#7C5CfC".into();
        assert_eq!(settings.clone().sanitized().ui_accent, "#7C5CfC");

        settings.ui_accent = "#abc".into();
        assert_eq!(settings.clone().sanitized().ui_accent, "purple");

        settings.ui_accent = "fuchsia".into();
        assert_eq!(settings.clone().sanitized().ui_accent, "magenta");
        settings.ui_accent = "violet".into();
        assert_eq!(settings.sanitized().ui_accent, "purple");
    }
}
