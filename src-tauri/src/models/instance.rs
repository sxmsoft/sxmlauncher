//! Instance (isolated game environment) models.
//!
//! Every instance owns a self-contained directory tree so mods, configs, saves
//! and resource packs can never leak between game versions or mod loaders.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Supported mod loaders for an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LoaderKind::Vanilla => "vanilla",
            LoaderKind::Fabric => "fabric",
            LoaderKind::Quilt => "quilt",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
        }
    }

    pub fn from_str_opt(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "vanilla" => Some(LoaderKind::Vanilla),
            "fabric" => Some(LoaderKind::Fabric),
            "quilt" => Some(LoaderKind::Quilt),
            "forge" => Some(LoaderKind::Forge),
            "neoforge" => Some(LoaderKind::NeoForge),
            _ => None,
        }
    }

    /// Loaders that need an installer-driven `--version <loader>-<mc>` profile.
    pub fn is_modded(self) -> bool {
        !matches!(self, LoaderKind::Vanilla)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModLoader {
    pub kind: LoaderKind,
    /// Loader version, e.g. `0.15.11` for Fabric, `47.2.0` for Forge.
    pub version: Option<String>,
    /// Forge/NeoForge build number when the loader exposes one.
    pub build: Option<String>,
}

impl ModLoader {
    pub fn vanilla() -> Self {
        Self {
            kind: LoaderKind::Vanilla,
            version: None,
            build: None,
        }
    }

    pub fn new(kind: LoaderKind, version: impl Into<String>) -> Self {
        Self {
            kind,
            version: Some(version.into()),
            build: None,
        }
    }

    /// The version id used for `versions/<id>/<id>.json`.
    pub fn version_id(&self, game_version: &str) -> String {
        match (self.kind, self.version.as_deref()) {
            (LoaderKind::Vanilla, _) => game_version.to_string(),
            (LoaderKind::Fabric, Some(v)) => format!("fabric-loader-{v}-{game_version}"),
            (LoaderKind::Quilt, Some(v)) => format!("quilt-loader-{v}-{game_version}"),
            (LoaderKind::Forge, Some(v)) => format!("{game_version}-forge-{v}"),
            (LoaderKind::NeoForge, Some(v)) => format!("neoforge-{v}"),
            (kind, None) => format!("{}-{}", kind.as_str(), game_version),
        }
    }

    /// Gradle-style coordinate fragment used in launcher profiles.
    pub fn maven_id(&self, game_version: &str) -> Option<String> {
        let version = self.version.as_deref()?;
        Some(match self.kind {
            LoaderKind::Fabric => format!("net.fabricmc:fabric-loader:{version}"),
            LoaderKind::Quilt => format!("org.quiltmc:quilt-loader:{version}"),
            LoaderKind::Forge => format!("net.minecraftforge:forge:{game_version}-{version}"),
            LoaderKind::NeoForge => format!("net.neoforged:neoforge:{version}"),
            LoaderKind::Vanilla => return None,
        })
    }
}

/// JVM selection strategy for this instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaSettings {
    /// Explicit `java` executable; wins over everything else when set.
    pub override_path: Option<PathBuf>,
    /// Preferred major version (8, 17, 21, ...). `None` means "derive from the
    /// Minecraft version manifest".
    pub preferred_major: Option<u8>,
    /// Allow downloading a managed runtime when nothing suitable is installed.
    pub auto_download: bool,
    /// Extra JVM flags appended after the recommended defaults.
    pub jvm_args: Vec<String>,
}

impl Default for JavaSettings {
    fn default() -> Self {
        Self {
            override_path: None,
            preferred_major: None,
            auto_download: true,
            jvm_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySettings {
    pub min_mb: u32,
    pub max_mb: u32,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            min_mb: 1024,
            max_mb: 4096,
        }
    }
}

impl MemorySettings {
    /// Clamp to a sane range and keep `min <= max` no matter what the UI sends.
    pub fn sanitized(&self) -> Self {
        const FLOOR_MB: u32 = 512;
        let max = self.max_mb.clamp(FLOOR_MB, 128 * 1024);
        let min = self.min_mb.clamp(FLOOR_MB, max);
        Self { min_mb: min, max_mb: max }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionSettings {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
}

impl Default for ResolutionSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fullscreen: false,
        }
    }
}

/// Everything persisted per instance (no derived/runtime state).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceConfig {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Emoji or absolute path to an icon inside the instance folder.
    pub icon: Option<String>,
    pub game_version: String,
    pub loader: ModLoader,
    pub java: JavaSettings,
    pub memory: MemorySettings,
    pub resolution: ResolutionSettings,
    /// Extra game arguments appended after the vanilla ones.
    pub game_args: Vec<String>,
    /// Set when the instance came from a modpack (.mrpack / CurseForge zip).
    pub source_pack: Option<crate::models::modpack::ModpackRefSource>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl InstanceConfig {
    /// The version folder this instance resolves against.
    pub fn resolved_version_id(&self) -> String {
        self.loader.version_id(&self.game_version)
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Install/health state of an instance directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    /// Config exists but assets/libraries/version json are missing.
    NotInstalled,
    Installing,
    /// Ready to launch.
    Ready,
    Running,
    /// Version json or libraries failed verification.
    Corrupted,
    /// A newer loader/modpack build is available.
    UpdateAvailable,
}

impl InstanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            InstanceStatus::NotInstalled => "not_installed",
            InstanceStatus::Installing => "installing",
            InstanceStatus::Ready => "ready",
            InstanceStatus::Running => "running",
            InstanceStatus::Corrupted => "corrupted",
            InstanceStatus::UpdateAvailable => "update_available",
        }
    }
}

/// Config + derived state as the UI needs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    #[serde(flatten)]
    pub config: InstanceConfig,
    pub status: InstanceStatus,
    pub mod_count: u32,
    pub last_played_at: Option<DateTime<Utc>>,
    pub total_playtime_secs: u64,
    pub launch_count: u32,
    /// Bytes on disk for the instance root (cached, refreshed on demand).
    pub size_bytes: u64,
    /// Required Java major version for `game_version` + loader.
    pub required_java_major: u8,
}

/// Canonical on-disk layout for one instance.
///
/// ```text
/// app_dir/
///   instances/<instance_id>/
///     instance.json         <- InstanceConfig (single source of truth)
///     mods/ config/ saves/ resourcepacks/ shaderpacks/ logs/
///     .minecraft -> compatibility root used when a pack hardcodes it
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstancePaths {
    /// `<app_dir>/instances`
    pub instances_root: PathBuf,
    pub id: Uuid,
}

impl InstancePaths {
    pub fn new(instances_root: PathBuf, id: Uuid) -> Self {
        Self { instances_root, id }
    }

    pub fn instance_root(&self, id: Uuid) -> PathBuf {
        self.instances_root.join(id.to_string())
    }

    pub fn root(&self) -> PathBuf {
        self.instance_root(self.id)
    }

    pub fn config_file(&self) -> PathBuf {
        self.root().join("instance.json")
    }

    pub fn mods(&self) -> PathBuf {
        self.root().join("mods")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root().join("config")
    }

    pub fn saves(&self) -> PathBuf {
        self.root().join("saves")
    }

    pub fn resourcepacks(&self) -> PathBuf {
        self.root().join("resourcepacks")
    }

    pub fn shaderpacks(&self) -> PathBuf {
        self.root().join("shaderpacks")
    }

    pub fn logs(&self) -> PathBuf {
        self.root().join("logs")
    }

    pub fn cache(&self) -> PathBuf {
        self.root().join(".cache")
    }

    /// Directories a fresh instance must contain.
    pub fn required_dirs(&self) -> [PathBuf; 6] {
        [
            self.mods(),
            self.config_dir(),
            self.saves(),
            self.resourcepacks(),
            self.shaderpacks(),
            self.logs(),
        ]
    }
}

/// Payload for `instance_create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstanceRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub game_version: String,
    #[serde(default)]
    pub loader: Option<ModLoader>,
    #[serde(default)]
    pub memory: Option<MemorySettings>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Install right after creating (downloads assets/libraries/loader).
    #[serde(default)]
    pub install_now: bool,
}

/// Sparse update payload for `instance_update`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstanceRequest {
    pub id: Uuid,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub game_version: Option<String>,
    pub loader: Option<ModLoader>,
    pub java: Option<JavaSettings>,
    pub memory: Option<MemorySettings>,
    pub resolution: Option<ResolutionSettings>,
    pub game_args: Option<Vec<String>>,
}

/// Filesystem helpers that avoid the `Path` -> `PathBuf` dance at call sites.
pub trait PathExt {
    fn ensure_dir(&self) -> crate::error::AppResult<()>;
}

impl PathExt for Path {
    fn ensure_dir(&self) -> crate::error::AppResult<()> {
        std::fs::create_dir_all(self).map_err(crate::error::AppError::from)
    }
}
