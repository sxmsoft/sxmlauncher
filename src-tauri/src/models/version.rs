//! Mojang version metadata (`versions/<id>/<id>.json`) plus the Maven/library
//! rules needed to assemble a classpath and a native extraction list.
//!
//! Inherited profiles (Fabric/Forge/Quilt) point at a vanilla parent through
//! [`VersionJson::inherits_from`]; the launcher merges child over parent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `versions/versions.json` — the remote version manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionManifest {
    pub latest: ManifestLatest,
    pub versions: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestLatest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub release_type: String,
    pub url: String,
    pub time: String,
    #[serde(rename = "releaseTime", default)]
    pub release_time: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub compliance_level: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    /// Modded profiles reference their vanilla parent here.
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    /// 1.13+ style arguments object.
    #[serde(default)]
    pub arguments: Option<Arguments>,
    /// Pre-1.13 style flat argument string.
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub java_version: Option<JavaVersionRef>,
    #[serde(default)]
    pub release_time: Option<String>,
    #[serde(rename = "type", default)]
    pub release_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    #[serde(rename = "totalSize", default)]
    pub total_size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<DownloadArtifact>,
    #[serde(default)]
    pub server: Option<DownloadArtifact>,
    /// Legacy mapping `classifier -> artifact` (1.2.x and older).
    #[serde(default)]
    pub client_mappings: Option<DownloadArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionRef {
    pub component: String,
    pub major_version: u8,
}

/// A single downloadable artifact with integrity metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadArtifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

impl DownloadArtifact {
    /// Where this artifact lives under the shared `libraries/` or `assets/` dir.
    ///
    /// Falls back to the tail of the URL when the manifest omits `path`.
    pub fn relative_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return Some(PathBuf::from(path));
        }
        let without_query = self.url.split('?').next().unwrap_or(&self.url);
        let segments: Vec<&str> = without_query.split('/').collect();
        if segments.len() >= 3 {
            let tail = segments[segments.len() - 3..].join("/");
            return Some(PathBuf::from(tail));
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    /// Maven coordinate `group:artifact:version[:classifier][@ext]`.
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// `{"windows": "natives-windows"}` — matches only this library's natives.
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    #[serde(default)]
    pub extract: Option<ExtractRules>,
    /// Legacy fallback URL prefix.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    /// Forge/Fabric sometimes mark a library as "not needed at runtime".
    #[serde(default)]
    pub clientreq: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<DownloadArtifact>,
    /// `natives-windows` -> artifact.
    #[serde(default)]
    pub classifiers: Option<HashMap<String, DownloadArtifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRules {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<OsRule>,
    /// `{"is_demo_user": true, "has_custom_resolution": true, ...}`
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsRule {
    /// `windows` | `osx` | `linux`.
    #[serde(default)]
    pub name: Option<String>,
    /// Regex tested against the OS version string.
    #[serde(default)]
    pub version: Option<String>,
    /// `x86` | `x86_64` | `arm64` (also `arm32` in the wild).
    #[serde(default)]
    pub arch: Option<String>,
}

impl OsRule {
    /// The Mojang OS token for the platform this binary runs on.
    pub fn current_os_name() -> &'static str {
        match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            other => other,
        }
    }

    /// Mojang arch token for the running process.
    pub fn current_arch() -> &'static str {
        match std::env::consts::ARCH {
            "x86" => "x86",
            "aarch64" => "arm64",
            "arm" => "arm32",
            _ => "x86_64",
        }
    }

    pub fn matches_current(&self) -> bool {
        if let Some(name) = &self.name {
            if !name.eq_ignore_ascii_case(Self::current_os_name()) {
                return false;
            }
        }
        if let Some(arch) = &self.arch {
            if !arch.eq_ignore_ascii_case(Self::current_arch()) {
                return false;
            }
        }
        if let Some(pattern) = &self.version {
            // Mojang uses regexes such as `^10\.` for Windows 10.
            return regex::Regex::new(pattern)
                .map(|re| re.is_match(os_version_string()))
                .unwrap_or(true);
        }
        true
    }
}

/// Best-effort OS version string used by `Rule.os.version`.
fn os_version_string() -> &'static str {
    if cfg!(target_os = "windows") {
        "10.0"
    } else if cfg!(target_os = "macos") {
        "14.0"
    } else {
        "6.0"
    }
}

impl Rule {
    pub fn applies(&self, features: &FeatureSet) -> bool {
        if let Some(os) = &self.os {
            if !os.matches_current() {
                return false;
            }
        }
        if let Some(required) = &self.features {
            for (key, expected) in required {
                if features.get(key) != Some(*expected) {
                    return false;
                }
            }
        }
        true
    }
}

/// Launch-time feature flags that `Rule::features` can test.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSet {
    pub is_demo_user: bool,
    pub has_custom_resolution: bool,
    pub has_quick_plays_support: bool,
    pub is_quick_play_singleplayer: bool,
    pub is_quick_play_multiplayer: bool,
    pub is_quick_play_realms: bool,
}

impl FeatureSet {
    pub fn get(&self, key: &str) -> Option<bool> {
        match key {
            "is_demo_user" => Some(self.is_demo_user),
            "has_custom_resolution" => Some(self.has_custom_resolution),
            "has_quick_plays_support" => Some(self.has_quick_plays_support),
            "is_quick_play_singleplayer" => Some(self.is_quick_play_singleplayer),
            "is_quick_play_multiplayer" => Some(self.is_quick_play_multiplayer),
            "is_quick_play_realms" => Some(self.is_quick_play_realms),
            _ => None,
        }
    }

    pub fn with_custom_resolution(mut self, enabled: bool) -> Self {
        self.has_custom_resolution = enabled;
        self
    }
}

/// Evaluate a rule list: the last matching rule wins, default is denied.
pub fn rules_allow(rules: &[Rule], features: &FeatureSet) -> bool {
    let mut allowed = false;
    for rule in rules {
        if rule.applies(features) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

/// One argument inside `arguments.game` / `arguments.jvm`: either a plain
/// string or a rule-guarded object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    Plain(String),
    Guarded {
        rules: Vec<Rule>,
        #[serde(default)]
        value: ArgumentEntries,
    },
}

/// `value` may be a single string or a list of strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentEntries {
    One(String),
    Many(Vec<String>),
}

impl Default for ArgumentEntries {
    fn default() -> Self {
        ArgumentEntries::Many(Vec::new())
    }
}

impl ArgumentEntries {
    pub fn iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        match self {
            ArgumentEntries::One(value) => Box::new(std::iter::once(value)),
            ArgumentEntries::Many(values) => Box::new(values.iter()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgumentValue>,
    #[serde(default)]
    pub jvm: Vec<ArgumentValue>,
}

impl Arguments {
    /// Flatten the guard-laden argument lists into launch-ready strings.
    pub fn flatten<'a>(values: &'a [ArgumentValue], features: &'a FeatureSet) -> Vec<&'a str> {
        let mut out = Vec::new();
        for entry in values {
            match entry {
                ArgumentValue::Plain(value) => out.push(value.as_str()),
                ArgumentValue::Guarded { rules, value } => {
                    if rules_allow(rules, features) {
                        out.extend(value.iter().map(|s| s.as_str()));
                    }
                }
            }
        }
        out
    }
}

/// Parsed Maven coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MavenParseError {
    #[error("malformed maven coordinate: {0}")]
    Malformed(String),
}

impl MavenCoordinate {
    /// Parse `group:artifact:version[:classifier][@ext]`.
    pub fn parse(raw: &str) -> Result<Self, MavenParseError> {
        let (coordinates, extension) = match raw.split_once('@') {
            Some((left, ext)) => (left, ext.to_string()),
            None => (raw, "jar".to_string()),
        };
        let mut parts = coordinates.split(':');
        let group = parts
            .next()
            .ok_or_else(|| MavenParseError::Malformed(raw.to_string()))?;
        let artifact = parts
            .next()
            .ok_or_else(|| MavenParseError::Malformed(raw.to_string()))?;
        let version = parts
            .next()
            .ok_or_else(|| MavenParseError::Malformed(raw.to_string()))?;
        if group.is_empty() || artifact.is_empty() || version.is_empty() {
            return Err(MavenParseError::Malformed(raw.to_string()));
        }
        Ok(Self {
            group: group.to_string(),
            artifact: artifact.to_string(),
            version: version.to_string(),
            classifier: parts.next().filter(|c| !c.is_empty()).map(str::to_string),
            extension,
        })
    }

    /// Path inside `libraries/`.
    pub fn to_path(&self) -> PathBuf {
        let group_path = self.group.replace('.', "/");
        let mut file = format!("{}-{}", self.artifact, self.version);
        if let Some(classifier) = &self.classifier {
            file.push('-');
            file.push_str(classifier);
        }
        file.push('.');
        file.push_str(&self.extension);
        Path::new(&group_path)
            .join(&self.artifact)
            .join(&self.version)
            .join(file)
    }

    pub fn file_name(&self) -> String {
        self.to_path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{}-{}.{}", self.artifact, self.version, self.extension))
    }
}

impl Library {
    /// Path under `libraries/` for this library's artifact, if resolvable.
    pub fn artifact_path(&self, feature_set: &FeatureSet) -> Option<PathBuf> {
        if let Some(rules) = &self.rules {
            if !rules_allow(rules, feature_set) {
                return None;
            }
            if self.clientreq == Some(false) {
                return None;
            }
        }
        if let Some(artifact) = self.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            if let Some(path) = artifact.relative_path() {
                return Some(path);
            }
        }
        // Forge/Fabric write bare coordinates without a downloads block.
        MavenCoordinate::parse(&self.name).ok().map(|c| c.to_path())
    }

    /// URL to fetch the artifact from when the manifest omits `downloads`.
    ///
    /// An empty URL means the file is produced locally (Forge/NeoForge
    /// installer processors, or a jar extracted from the installer). Those
    /// must not be fetched.
    pub fn artifact_url(&self) -> Option<String> {
        if let Some(artifact) = self.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            if artifact.url.is_empty() {
                return None;
            }
            return Some(artifact.url.clone());
        }
        let base = self
            .url
            .clone()
            .unwrap_or_else(|| "https://libraries.minecraft.net/".to_string());
        if base.is_empty() {
            return None;
        }
        let coordinate = MavenCoordinate::parse(&self.name).ok()?;
        Some(format!(
            "{}/{}",
            base.trim_end_matches('/'),
            coordinate.to_path().to_string_lossy().replace('\\', "/")
        ))
    }

    /// Native classifier key for this platform, e.g. `natives-windows`.
    pub fn native_classifier(&self) -> Option<&str> {
        let natives = self.natives.as_ref()?;
        let key = OsRule::current_os_name();
        natives.get(key).map(String::as_str)
    }

    /// Native jar to download + extract for this platform.
    pub fn native_artifact(&self) -> Option<&DownloadArtifact> {
        let classifier = self.native_classifier()?;
        self.downloads
            .as_ref()
            .and_then(|d| d.classifiers.as_ref())
            .and_then(|map| map.get(classifier))
    }

    /// `true` when this library is needed on the client for the current OS.
    pub fn is_applicable(&self, features: &FeatureSet) -> bool {
        match &self.rules {
            Some(rules) => rules_allow(rules, features),
            None => true,
        }
    }

    pub fn should_extract(&self) -> bool {
        self.native_classifier().is_some()
    }

    /// Pattern list from the `extract.exclude` block (e.g. `META-INF/`).
    pub fn extract_excludes(&self) -> Vec<String> {
        self.extract
            .as_ref()
            .map(|rules| rules.exclude.clone())
            .unwrap_or_default()
    }
}

/// Version id whose `versions/<id>/<id>.jar` is the Mojang client.
///
/// A merged loader profile keeps the parent's client download but its own id.
/// The jar still lives next to the vanilla version json (`inheritsFrom`).
pub fn client_jar_version_id(version: &VersionJson) -> &str {
    if version.inherits_from.is_some() {
        return version.inherits_from.as_deref().unwrap_or(&version.id);
    }
    &version.id
}

/// Merge a child profile onto its parent (child wins, lists are concatenated).
///
/// Arguments are appended, not replaced. Fabric/Quilt/Forge profiles ship only
/// their extra JVM or game flags and rely on the vanilla parent for `-cp` and
/// `--username`.
pub fn merge_profiles(parent: &VersionJson, child: &VersionJson) -> VersionJson {
    let mut merged = parent.clone();
    merged.id = child.id.clone();
    merged.inherits_from = child.inherits_from.clone();
    if child.main_class.is_some() {
        merged.main_class = child.main_class.clone();
    }
    if child.assets.is_some() {
        merged.assets = child.assets.clone();
    }
    if child.asset_index.is_some() {
        merged.asset_index = child.asset_index.clone();
    }
    if child.java_version.is_some() {
        merged.java_version = child.java_version.clone();
    }
    if let Some(child_args) = &child.arguments {
        let mut combined = merged.arguments.clone().unwrap_or_default();
        combined.game.extend(child_args.game.iter().cloned());
        combined.jvm.extend(child_args.jvm.iter().cloned());
        merged.arguments = Some(combined);
    }
    if child.minecraft_arguments.is_some() {
        merged.minecraft_arguments = child.minecraft_arguments.clone();
    }
    // Modded profiles prepend their libraries so they shadow the vanilla ones.
    let mut libraries = child.libraries.clone();
    libraries.extend(parent.libraries.iter().cloned());
    merged.libraries = libraries;
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maven_coordinates_with_classifier_and_extension() {
        let coordinate =
            MavenCoordinate::parse("org.lwjgl:lwjgl:3.3.1:natives-windows").expect("parses");
        assert_eq!(coordinate.group, "org.lwjgl");
        assert_eq!(coordinate.classifier.as_deref(), Some("natives-windows"));
        assert_eq!(coordinate.extension, "jar");
        assert_eq!(
            coordinate.to_path().to_string_lossy().replace('\\', "/"),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );
    }

    #[test]
    fn parses_extension_override() {
        let coordinate = MavenCoordinate::parse("com.example:lib:1.0@zip").expect("parses");
        assert_eq!(coordinate.extension, "zip");
        assert!(coordinate.classifier.is_none());
    }

    #[test]
    fn rejects_malformed_coordinates() {
        assert!(MavenCoordinate::parse("not-a-coordinate").is_err());
        assert!(MavenCoordinate::parse("a:b:").is_err());
    }

    #[test]
    fn rules_default_deny_and_last_match_wins() {
        let features = FeatureSet::default();
        let allow_windows = Rule {
            action: RuleAction::Allow,
            os: Some(OsRule {
                name: Some("windows".into()),
                version: None,
                arch: None,
            }),
            features: None,
        };
        assert_eq!(rules_allow(&[allow_windows], &features), cfg!(windows));

        let deny_all = Rule {
            action: RuleAction::Disallow,
            os: None,
            features: None,
        };
        assert!(!rules_allow(&[deny_all], &features));
        assert!(!rules_allow(&[], &features));
    }

    #[test]
    fn guarded_arguments_respect_feature_flags() {
        let args = vec![
            ArgumentValue::Plain("--username".into()),
            ArgumentValue::Plain("${auth_player_name}".into()),
            ArgumentValue::Guarded {
                rules: vec![Rule {
                    action: RuleAction::Allow,
                    os: None,
                    features: Some(HashMap::from([("has_custom_resolution".to_string(), true)])),
                }],
                value: ArgumentEntries::Many(vec!["--width".into(), "${resolution_width}".into()]),
            },
        ];
        // `flatten` borrows the feature set, so the temporaries need names.
        let off_features = FeatureSet::default();
        let off = Arguments::flatten(&args, &off_features);
        assert_eq!(off, vec!["--username", "${auth_player_name}"]);

        let on_features = FeatureSet::default().with_custom_resolution(true);
        let on = Arguments::flatten(&args, &on_features);
        assert_eq!(
            on,
            vec![
                "--username",
                "${auth_player_name}",
                "--width",
                "${resolution_width}"
            ]
        );
    }

    #[test]
    fn merge_prefers_child_main_class_and_keeps_parent_libraries() {
        let parent = VersionJson {
            id: "1.20.1".into(),
            inherits_from: None,
            main_class: Some("net.minecraft.client.main.Main".into()),
            assets: Some("5".into()),
            asset_index: None,
            libraries: vec![Library {
                name: "com.mojang:brigadier:1.0.18".into(),
                downloads: None,
                natives: None,
                rules: None,
                extract: None,
                url: None,
                sha1: None,
                clientreq: None,
            }],
            arguments: None,
            minecraft_arguments: None,
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: None,
        };
        let child = VersionJson {
            id: "fabric-loader-0.15.11-1.20.1".into(),
            inherits_from: Some("1.20.1".into()),
            main_class: Some("net.fabricmc.loader.impl.launch.knot.KnotClient".into()),
            assets: None,
            asset_index: None,
            libraries: vec![Library {
                name: "net.fabricmc:fabric-loader:0.15.11".into(),
                downloads: None,
                natives: None,
                rules: None,
                extract: None,
                url: None,
                sha1: None,
                clientreq: None,
            }],
            arguments: None,
            minecraft_arguments: None,
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: None,
        };

        let merged = merge_profiles(&parent, &child);
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        assert_eq!(merged.libraries.len(), 2);
        assert!(merged.libraries[0].name.contains("fabric-loader"));
        assert_eq!(merged.assets.as_deref(), Some("5"));
        assert_eq!(client_jar_version_id(&merged), "1.20.1");
        assert_eq!(client_jar_version_id(&parent), "1.20.1");
    }

    #[test]
    fn merge_appends_loader_arguments_onto_the_vanilla_ones() {
        let parent = VersionJson {
            id: "1.21.1".into(),
            inherits_from: None,
            main_class: Some("net.minecraft.client.main.Main".into()),
            assets: None,
            asset_index: None,
            libraries: Vec::new(),
            arguments: Some(Arguments {
                game: vec![ArgumentValue::Plain("--username".into())],
                jvm: vec![
                    ArgumentValue::Plain("-cp".into()),
                    ArgumentValue::Plain("${classpath}".into()),
                ],
            }),
            minecraft_arguments: None,
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: None,
        };
        let child = VersionJson {
            id: "fabric-loader-0.19.5-1.21.1".into(),
            inherits_from: Some("1.21.1".into()),
            main_class: Some("net.fabricmc.loader.impl.launch.knot.KnotClient".into()),
            assets: None,
            asset_index: None,
            libraries: Vec::new(),
            arguments: Some(Arguments {
                game: Vec::new(),
                jvm: vec![ArgumentValue::Plain(
                    "-DFabricMcEmu=net.minecraft.client.main.Main".into(),
                )],
            }),
            minecraft_arguments: None,
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: None,
        };
        let merged = merge_profiles(&parent, &child);
        let features = FeatureSet::default();
        let jvm = Arguments::flatten(&merged.arguments.as_ref().unwrap().jvm, &features);
        let game = Arguments::flatten(&merged.arguments.as_ref().unwrap().game, &features);
        assert_eq!(
            jvm,
            vec![
                "-cp",
                "${classpath}",
                "-DFabricMcEmu=net.minecraft.client.main.Main"
            ]
        );
        assert_eq!(game, vec!["--username"]);
    }

    #[test]
    fn empty_artifact_url_is_not_downloaded() {
        let library = Library {
            name: "net.minecraftforge:forge:1.21.1-52.1.0:client".into(),
            downloads: Some(LibraryDownloads {
                artifact: Some(DownloadArtifact {
                    path: Some(
                        "net/minecraftforge/forge/1.21.1-52.1.0/forge-1.21.1-52.1.0-client.jar"
                            .into(),
                    ),
                    sha1: Some("abc".into()),
                    size: Some(1),
                    url: String::new(),
                }),
                classifiers: None,
            }),
            natives: None,
            rules: None,
            extract: None,
            url: None,
            sha1: None,
            clientreq: None,
        };
        assert!(library.artifact_url().is_none());
        assert!(library.artifact_path(&FeatureSet::default()).is_some());
    }
}
