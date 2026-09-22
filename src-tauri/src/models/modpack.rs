//! Mod / modpack registry models (Modrinth v2 + CurseForge v1) and the
//! resolved install plan the downloader consumes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which registry a mod/modpack came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModSource {
    Modrinth,
    /// Wire name is `curseforge` (the UI and the database). `curse_forge` is
    /// accepted so configs written by the old snake_case rename still load.
    #[serde(rename = "curseforge", alias = "curse_forge")]
    CurseForge,
}

impl Default for ModSource {
    /// Modrinth is the default registry: no API key required.
    fn default() -> Self {
        ModSource::Modrinth
    }
}

impl ModSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModSource::Modrinth => "modrinth",
            ModSource::CurseForge => "curseforge",
        }
    }

    pub fn from_str_opt(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "modrinth" => Some(ModSource::Modrinth),
            "curseforge" | "curse_forge" | "curse-forge" => Some(ModSource::CurseForge),
            _ => None,
        }
    }
}

/// Reference to the pack an instance was created from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackRefSource {
    pub source: ModSource,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// Hash set as reported by Modrinth (`sha1` is what the launcher verifies).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModHashes {
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
    /// CurseForge only.
    #[serde(default)]
    pub murmur2: Option<u32>,
}

/// Normalized project (mod, resourcepack, shader, modpack) metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModProject {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub source: ModSource,
    pub project_type: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub followers: u64,
    pub categories: Vec<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub license: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub client_side: Option<String>,
    pub server_side: Option<String>,
}

/// A single downloadable release of a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    pub source: ModSource,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub downloads: u64,
    pub file_name: String,
    pub download_url: String,
    pub hashes: ModHashes,
    pub file_size: u64,
    pub published_at: Option<DateTime<Utc>>,
    pub dependencies: Vec<ModDependency>,
}

/// What kind of relation a dependency expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackDependencyKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// Dependency edge from a version to another project/version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModDependency {
    pub kind: PackDependencyKind,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub file_name: Option<String>,
}

/// One search hit as rendered in the mod/modpack grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSearchHit {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub source: ModSource,
    pub project_type: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub categories: Vec<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSearchQuery {
    #[serde(default)]
    pub query: Option<String>,
    pub source: ModSource,
    #[serde(default)]
    pub project_type: Option<String>,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSearchResults {
    pub hits: Vec<ModSearchHit>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
    pub source: ModSource,
}

/// `modrinth.index.json` (format version 1) inside a `.mrpack`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackManifest {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub files: Vec<MrpackFile>,
    /// `{"minecraft": "1.20.1", "fabric-loader": "0.15.11"}`
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

/// An entry of `MrpackManifest::files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    /// Relative destination path inside the instance (may contain `/`).
    pub path: String,
    pub hashes: ModHashes,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub downloads: Vec<String>,
    #[serde(default)]
    pub file_size: u64,
}

impl MrpackFile {
    /// Respect `env.client == "unsupported"` so server-only mods stay out.
    pub fn supports_client(&self) -> bool {
        self.env
            .as_ref()
            .and_then(|env| env.get("client"))
            .is_none_or(|value| value != "unsupported")
    }
}

/// Minecraft version + loader implied by a pack manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackTarget {
    pub game_version: String,
    pub loader: crate::models::instance::ModLoader,
}

/// A concrete file the downloader must fetch and verify.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMod {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub file_name: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: u64,
    /// Path relative to the instance root, e.g. `mods/sodium.jar`.
    pub destination: String,
    pub source: ModSource,
    pub required: bool,
    /// Manifest-declared reason for inclusion (UI grouping).
    pub reason: ModInclusionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModInclusionReason {
    Requested,
    RequiredDependency,
    OptionalDependency,
    FromPackManifest,
    Override,
}

/// Everything needed to install a modpack or a mod selection, fully resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPackPlan {
    pub target: PackTarget,
    pub files: Vec<ResolvedMod>,
    /// Jars already present that must be deleted (removed from the pack).
    pub removals: Vec<String>,
    /// Conflicting projects detected between two requested mods.
    pub conflicts: Vec<ModConflict>,
    pub total_bytes: u64,
    pub java_major: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConflict {
    pub project_id: String,
    pub title: String,
    pub with_project_id: String,
    pub with_title: String,
    pub reason: String,
}

// --- CurseForge wire shapes ------------------------------------------------
//
// These mirror the v1 JSON verbatim (including its quirks) so no lossy mapping
// sits between the API and our own models. `CurseForgeHash::value` is a JSON
// value because CurseForge returns a string for SHA-1 and a number for
// MurmurHash2 in the same array.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeMod {
    pub id: u32,
    #[serde(default)]
    pub game_id: u32,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub class_id: Option<u32>,
    #[serde(default)]
    pub categories: Vec<CurseForgeCategory>,
    #[serde(default)]
    pub logo: Option<CurseForgeAsset>,
    #[serde(default)]
    pub latest_files: Vec<CurseForgeFile>,
    #[serde(default)]
    pub date_modified: Option<DateTime<Utc>>,
    #[serde(default)]
    pub allow_mod_distribution: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeCategory {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub class_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeAsset {
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeFile {
    pub id: u32,
    #[serde(default)]
    pub mod_id: u32,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub file_name: String,
    /// 1 = release, 2 = beta, 3 = alpha.
    #[serde(default)]
    pub release_type: u8,
    #[serde(default)]
    pub hashes: Vec<CurseForgeHash>,
    #[serde(default)]
    pub file_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub file_length: u64,
    #[serde(default)]
    pub download_count: u64,
    /// `None` when the author disabled third-party distribution.
    #[serde(default)]
    pub download_url: Option<String>,
    /// Mixed list of game versions *and* loader names.
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<CurseForgeDependency>,
    #[serde(default)]
    pub file_fingerprint: Option<u32>,
}

impl CurseForgeFile {
    /// `true` when the file advertises support for a loader name.
    pub fn supports_loader(&self, loader: &str) -> bool {
        self.game_versions
            .iter()
            .any(|value| value.eq_ignore_ascii_case(loader))
    }

    /// Author-disabled downloads cannot be fetched by a launcher.
    pub fn is_downloadable(&self) -> bool {
        self.download_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurseForgeHash {
    /// String for SHA-1 (algo 1), number for MurmurHash2 (algo 2).
    pub value: serde_json::Value,
    pub algo: u8,
}

impl CurseForgeHash {
    pub fn text(&self) -> Option<String> {
        match &self.value {
            serde_json::Value::String(value) => Some(value.clone()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<u32> {
        match &self.value {
            serde_json::Value::Number(value) => value.as_u64().and_then(|v| u32::try_from(v).ok()),
            serde_json::Value::String(value) => value.parse::<u32>().ok(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeDependency {
    #[serde(default)]
    pub mod_id: u32,
    /// 1 = embedded, 2 = optional, 3 = required, 4 = tool, 5 = incompatible,
    /// 6 = include.
    #[serde(default)]
    pub relation_type: u8,
}

/// Reference to a specific CurseForge file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeRef {
    pub mod_id: u32,
    pub file_id: u32,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curseforge_serializes_as_curseforge_and_accepts_the_old_name() {
        assert_eq!(
            serde_json::to_string(&ModSource::CurseForge).unwrap(),
            "\"curseforge\""
        );
        assert_eq!(
            serde_json::from_str::<ModSource>("\"curseforge\"").unwrap(),
            ModSource::CurseForge
        );
        assert_eq!(
            serde_json::from_str::<ModSource>("\"curse_forge\"").unwrap(),
            ModSource::CurseForge
        );
        assert_eq!(
            ModSource::from_str_opt("curse_forge"),
            Some(ModSource::CurseForge)
        );
        assert_eq!(
            ModSource::from_str_opt("curse-forge"),
            Some(ModSource::CurseForge)
        );
        assert_eq!(ModSource::CurseForge.as_str(), "curseforge");
    }
}
