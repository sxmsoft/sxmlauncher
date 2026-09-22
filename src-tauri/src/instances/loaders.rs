//! Mod-loader installation.
//!
//! Mojang's version manifest only knows vanilla ids (`1.21.1`). Fabric, Quilt,
//! Forge and NeoForge publish their own metadata and a version json that
//! `inheritsFrom` that vanilla id. The installer therefore:
//!
//! 1. installs the plain Minecraft version through [`Installer::install_version`]
//! 2. resolves a loader build from the loader's meta API
//! 3. writes `versions/<profile>/<profile>.json` and downloads its libraries
//!
//! Forge and NeoForge have no profile endpoint. Their installer jar carries
//! `install_profile.json` plus `version.json`, and client processors (binary
//! patch, mappings) have to run before the profile is launchable.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::instances::installer::Installer;
use crate::models::instance::{InstanceConfig, LoaderKind, ModLoader};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::version::{Library, MavenCoordinate, VersionJson};
use crate::mods::downloader::DownloadTask;
use crate::process;

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const NEOFORGE_META: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

/// Loader build that was installed, plus the profile id written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledLoader {
    pub version: String,
    pub profile_id: String,
}

/// Resolve, download and persist the loader profile for `config`.
///
/// `java` is required for Forge and NeoForge processors. Fabric and Quilt
/// profiles are plain JSON.
pub async fn install_loader(
    installer: &Installer<'_>,
    config: &InstanceConfig,
    java: Option<&Path>,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<InstalledLoader> {
    match config.loader.kind {
        LoaderKind::Vanilla => Err(AppError::Config(
            "vanilla instances do not have a loader profile".into(),
        )),
        LoaderKind::Fabric => install_meta_loader(installer, config, "fabric", sink).await,
        LoaderKind::Quilt => install_meta_loader(installer, config, "quilt", sink).await,
        LoaderKind::Forge | LoaderKind::NeoForge => {
            let java = java.ok_or_else(|| {
                AppError::Java(format!(
                    "installing {} needs a Java runtime to run the loader's installer",
                    config.loader.kind.as_str()
                ))
            })?;
            install_processor_loader(installer, config, java, sink).await
        }
    }
}

async fn install_meta_loader(
    installer: &Installer<'_>,
    config: &InstanceConfig,
    family: &str,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<InstalledLoader> {
    let game = config.mojang_version_id();
    sink.report(
        ProgressEvent::started(JobKind::InstanceInstall, format!("Resolving {family}"))
            .stage(JobStage::Resolving)
            .detail(format!("{family} for Minecraft {game}")),
    )
    .await;

    let version = match config.loader.version.clone() {
        Some(version) => version,
        None => resolve_meta_loader(installer.http(), family, game).await?,
    };
    let profile_id = ModLoader::new(config.loader.kind, version.clone()).version_id(game);
    let profile_url = match family {
        "quilt" => format!("{QUILT_META}/versions/loader/{game}/{version}/profile/json"),
        _ => format!("{FABRIC_META}/versions/loader/{game}/{version}/profile/json"),
    };
    let mut profile: VersionJson = get_json(installer.http(), &profile_url).await?;
    profile.id = profile_id.clone();
    if profile.inherits_from.is_none() {
        profile.inherits_from = Some(game.to_string());
    }

    installer
        .install_libraries(
            &profile,
            &format!("Installing {family} {version}"),
            sink.clone(),
        )
        .await?;
    write_profile(installer.paths, &profile).await?;
    Ok(InstalledLoader {
        version,
        profile_id,
    })
}

async fn resolve_meta_loader(
    http: &reqwest::Client,
    family: &str,
    game: &str,
) -> AppResult<String> {
    let url = match family {
        "quilt" => format!("{QUILT_META}/versions/loader/{game}"),
        _ => format!("{FABRIC_META}/versions/loader/{game}"),
    };
    let entries: Vec<MetaLoaderEntry> = get_json(http, &url).await?;
    pick_meta_loader(&entries).ok_or_else(|| {
        AppError::Config(format!(
            "no {family} loader is published for Minecraft {game}"
        ))
    })
}

async fn install_processor_loader(
    installer: &Installer<'_>,
    config: &InstanceConfig,
    java: &Path,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<InstalledLoader> {
    let game = config.mojang_version_id().to_string();
    let kind = config.loader.kind;
    sink.report(
        ProgressEvent::started(
            JobKind::InstanceInstall,
            format!("Resolving {}", kind.as_str()),
        )
        .stage(JobStage::Resolving)
        .detail(format!("{} for Minecraft {game}", kind.as_str())),
    )
    .await;

    let version = match config.loader.version.clone() {
        Some(version) => normalize_loader_version(kind, &game, &version),
        None => resolve_processor_loader(installer.http(), kind, &game).await?,
    };
    let profile_id = ModLoader::new(kind, version.clone()).version_id(&game);
    let installer_url = installer_jar_url(kind, &game, &version);
    let installer_path = installer
        .paths
        .cache
        .join("installers")
        .join(format!("{profile_id}-installer.jar"));

    let task = DownloadTask::new(
        format!("{profile_id}-installer.jar"),
        installer_url,
        installer_path.clone(),
    )
    .without_cache();
    installer
        .downloader
        .fetch_all(
            JobKind::InstanceInstall,
            format!("Downloading {} installer", kind.as_str()),
            vec![task],
            sink.clone(),
        )
        .await?;

    let extracted = read_installer_jar(&installer_path)?;
    extract_bundled_maven(&installer_path, &installer.paths.libraries())?;

    let mut libraries = extracted.profile.libraries.clone();
    libraries.extend(extracted.version.libraries.iter().cloned());
    let library_profile = version_with_libraries(&profile_id, libraries);
    installer
        .install_libraries(
            &library_profile,
            &format!("Installing {} {version} libraries", kind.as_str()),
            sink.clone(),
        )
        .await?;

    let client_jar = installer
        .paths
        .version_dir(&game)
        .join(format!("{game}.jar"));
    if !client_jar.is_file() {
        return Err(AppError::Config(format!(
            "Minecraft {game} client jar is missing; install the vanilla version before {}",
            kind.as_str()
        )));
    }

    run_client_processors(
        &extracted.profile,
        &installer_path,
        installer.paths,
        &client_jar,
        &game,
        java,
        &sink,
    )
    .await?;

    let mut profile = extracted.version;
    profile.id = profile_id.clone();
    if profile.inherits_from.is_none() {
        profile.inherits_from = Some(game.clone());
    }
    write_profile(installer.paths, &profile).await?;
    Ok(InstalledLoader {
        version,
        profile_id,
    })
}

fn installer_jar_url(kind: LoaderKind, game: &str, version: &str) -> String {
    match kind {
        LoaderKind::NeoForge => {
            format!("{NEOFORGE_MAVEN}/{version}/neoforge-{version}-installer.jar")
        }
        _ => {
            let artifact = format!("{game}-{version}");
            format!("{FORGE_MAVEN}/{artifact}/forge-{artifact}-installer.jar")
        }
    }
}

async fn resolve_processor_loader(
    http: &reqwest::Client,
    kind: LoaderKind,
    game: &str,
) -> AppResult<String> {
    match kind {
        LoaderKind::NeoForge => {
            let xml = get_text(http, NEOFORGE_META).await?;
            let versions = xml_tag_values(&xml, "version");
            newest_neoforge_version(game, versions.iter().map(String::as_str)).ok_or_else(|| {
                AppError::Config(format!(
                    "no NeoForge build is published for Minecraft {game}"
                ))
            })
        }
        LoaderKind::Forge => {
            let promos: serde_json::Value = get_json(http, FORGE_PROMOS).await?;
            forge_promo_version(&promos, game).ok_or_else(|| {
                AppError::Config(format!("no Forge build is published for Minecraft {game}"))
            })
        }
        _ => Err(AppError::Config(format!(
            "{} does not use an installer jar",
            kind.as_str()
        ))),
    }
}

/// Forge promotion versions are `52.1.0`. Accept a value that already includes
/// the Minecraft version or the `forge` token.
pub fn normalize_loader_version(kind: LoaderKind, game: &str, raw: &str) -> String {
    let raw = raw.trim();
    if kind != LoaderKind::Forge {
        return raw.to_string();
    }
    forge_loader_version(game, raw)
}

pub fn forge_loader_version(game: &str, raw: &str) -> String {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix(&format!("{game}-forge-")) {
        return rest.to_string();
    }
    if let Some(rest) = raw.strip_prefix(&format!("{game}-")) {
        return rest.to_string();
    }
    raw.to_string()
}

/// NeoForge versions track Minecraft as `21.1.x` for `1.21.1` and `21.0.x` for `1.21`.
pub fn neoforge_version_prefix(game_version: &str) -> Option<String> {
    let rest = game_version.strip_prefix("1.")?;
    if rest.is_empty() {
        return None;
    }
    if rest.contains('.') {
        Some(rest.to_string())
    } else {
        Some(format!("{rest}.0"))
    }
}

pub fn newest_neoforge_version<'a>(
    game_version: &str,
    versions: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let prefix = neoforge_version_prefix(game_version)?;
    let dotted = format!("{prefix}.");
    let matched: Vec<&str> = versions
        .into_iter()
        .filter(|version| *version == prefix || version.starts_with(&dotted))
        .collect();
    let stable: Vec<&str> = matched
        .iter()
        .copied()
        .filter(|version| !is_prerelease(version))
        .collect();
    let pool = if stable.is_empty() { matched } else { stable };
    pool.into_iter()
        .max_by(|left, right| cmp_version(left, right))
        .map(str::to_string)
}

pub fn forge_promo_version(promos: &serde_json::Value, game: &str) -> Option<String> {
    let table = promos.get("promos")?.as_object()?;
    let raw = table
        .get(&format!("{game}-recommended"))
        .or_else(|| table.get(&format!("{game}-latest")))
        .and_then(|value| value.as_str())?;
    Some(forge_loader_version(game, raw))
}

#[derive(Debug, Deserialize)]
struct MetaLoaderEntry {
    loader: MetaLoaderVersion,
}

#[derive(Debug, Deserialize)]
struct MetaLoaderVersion {
    version: String,
    #[serde(default)]
    stable: bool,
}

fn pick_meta_loader(entries: &[MetaLoaderEntry]) -> Option<String> {
    if let Some(stable) = entries.iter().find(|entry| entry.loader.stable) {
        return Some(stable.loader.version.clone());
    }
    let releases: Vec<&MetaLoaderEntry> = entries
        .iter()
        .filter(|entry| !is_prerelease(&entry.loader.version))
        .collect();
    let pool: Vec<&MetaLoaderEntry> = if releases.is_empty() {
        entries.iter().collect()
    } else {
        releases
    };
    pool.into_iter()
        .max_by(|left, right| cmp_version(&left.loader.version, &right.loader.version))
        .map(|entry| entry.loader.version.clone())
}

fn is_prerelease(version: &str) -> bool {
    version.contains('-') || version.contains('+')
}

fn cmp_version(left: &str, right: &str) -> std::cmp::Ordering {
    let left_parts = version_numbers(left);
    let right_parts = version_numbers(right);
    let len = left_parts.len().max(right_parts.len());
    for index in 0..len {
        let l = left_parts.get(index).copied().unwrap_or(0);
        let r = right_parts.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn version_numbers(version: &str) -> Vec<i64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[derive(Debug, Deserialize)]
struct InstallProfile {
    #[serde(default)]
    data: HashMap<String, DataValue>,
    #[serde(default)]
    processors: Vec<ProcessorSpec>,
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DataValue {
    Sides {
        #[serde(default)]
        client: String,
        /// Server-side data. Client installs only read `client`.
        #[serde(default)]
        #[allow(dead_code)]
        server: String,
    },
    Literal(String),
}

impl DataValue {
    fn client(&self) -> &str {
        match self {
            DataValue::Sides { client, .. } => client,
            DataValue::Literal(value) => value,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProcessorSpec {
    #[serde(default)]
    sides: Vec<String>,
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
}

struct ExtractedInstaller {
    profile: InstallProfile,
    version: VersionJson,
}

fn read_installer_jar(path: &Path) -> AppResult<ExtractedInstaller> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| {
        AppError::Config(format!(
            "loader installer {} is not a jar: {err}",
            path.display()
        ))
    })?;
    let profile = read_zip_json::<InstallProfile>(&mut archive, "install_profile.json")?;
    let version = read_zip_json::<VersionJson>(&mut archive, "version.json")?;
    Ok(ExtractedInstaller { profile, version })
}

fn read_zip_json<T: for<'de> Deserialize<'de>>(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> AppResult<T> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| AppError::Config(format!("loader installer is missing {name}")))?;
    let mut raw = String::new();
    entry.read_to_string(&mut raw)?;
    Ok(serde_json::from_str(&raw)?)
}

fn extract_bundled_maven(installer_jar: &Path, libraries: &Path) -> AppResult<()> {
    let file = std::fs::File::open(installer_jar)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Config(format!("could not read installer jar: {err}")))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| AppError::Config(format!("could not read installer entry: {err}")))?;
        let name = entry.name().to_string();
        if !name.starts_with("maven/") || name.ends_with('/') || name.contains("..") {
            continue;
        }
        let relative = &name["maven/".len()..];
        let destination = libraries.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&destination)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

async fn run_client_processors(
    profile: &InstallProfile,
    installer_jar: &Path,
    paths: &AppPaths,
    client_jar: &Path,
    game: &str,
    java: &Path,
    sink: &Arc<dyn ProgressSink>,
) -> AppResult<()> {
    let client_processors: Vec<&ProcessorSpec> = profile
        .processors
        .iter()
        .filter(|processor| runs_on_client(&processor.sides))
        .collect();
    if client_processors.is_empty() {
        return Ok(());
    }

    let temp = std::env::temp_dir().join(format!("sxml-loader-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp)?;
    let tokens = load_processor_data(profile, paths, client_jar, game, installer_jar, &temp)?;

    for (index, processor) in client_processors.iter().enumerate() {
        let outputs = resolve_outputs(processor, &tokens, &paths.libraries())?;
        // No declared outputs means the processor must run. An empty list is
        // not a cache hit — NeoForge's client processors work that way.
        if !outputs.is_empty() && outputs_are_valid(&outputs)? {
            continue;
        }
        for output in &outputs {
            if output.file.is_file() {
                let _ = std::fs::remove_file(&output.file);
            }
        }
        sink.report(
            ProgressEvent::started(JobKind::InstanceInstall, "Running loader installer")
                .stage(JobStage::Extracting)
                .detail(format!(
                    "processor {}/{} ({})",
                    index + 1,
                    client_processors.len(),
                    processor.jar
                )),
        )
        .await;
        run_processor(processor, &tokens, &paths.libraries(), paths, java).await?;
        if !outputs.is_empty() && !outputs_are_valid(&outputs)? {
            return Err(AppError::Config(format!(
                "loader processor {} produced an unexpected file",
                processor.jar
            )));
        }
    }

    let _ = std::fs::remove_dir_all(&temp);
    Ok(())
}

fn runs_on_client(sides: &[String]) -> bool {
    sides.is_empty() || sides.iter().any(|side| side.eq_ignore_ascii_case("client"))
}

fn load_processor_data(
    profile: &InstallProfile,
    paths: &AppPaths,
    client_jar: &Path,
    game: &str,
    installer_jar: &Path,
    temp: &Path,
) -> AppResult<HashMap<String, String>> {
    let mut tokens = HashMap::new();
    for (key, value) in &profile.data {
        let raw = value.client();
        if raw.is_empty() {
            return Err(AppError::Config(format!(
                "loader installer data '{key}' has no client value"
            )));
        }
        tokens.insert(
            key.clone(),
            resolve_data_value(raw, &paths.libraries(), installer_jar, temp)?,
        );
    }
    tokens.insert("SIDE".into(), "client".into());
    tokens.insert(
        "MINECRAFT_JAR".into(),
        client_jar.to_string_lossy().into_owned(),
    );
    tokens.insert("MINECRAFT_VERSION".into(), game.to_string());
    tokens.insert("ROOT".into(), paths.shared.to_string_lossy().into_owned());
    tokens.insert(
        "INSTALLER".into(),
        installer_jar.to_string_lossy().into_owned(),
    );
    tokens.insert(
        "LIBRARY_DIR".into(),
        paths.libraries().to_string_lossy().into_owned(),
    );
    Ok(tokens)
}

fn resolve_data_value(
    raw: &str,
    libraries: &Path,
    installer_jar: &Path,
    temp: &Path,
) -> AppResult<String> {
    if let Some(inner) = bracketed(raw) {
        return Ok(libraries
            .join(parse_maven(inner)?.to_path())
            .to_string_lossy()
            .into_owned());
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return Ok(raw[1..raw.len() - 1].to_string());
    }
    let relative = raw.trim_start_matches(['/', '\\']);
    let destination = temp.join(relative);
    extract_zip_entry(installer_jar, relative, &destination)?;
    Ok(destination.to_string_lossy().into_owned())
}

fn extract_zip_entry(archive_path: &Path, name: &str, destination: &Path) -> AppResult<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| {
        AppError::Config(format!("could not open {}: {err}", archive_path.display()))
    })?;
    let entry_name = if archive.by_name(name).is_ok() {
        name.to_string()
    } else if archive.by_name(&format!("/{name}")).is_ok() {
        format!("/{name}")
    } else {
        return Err(AppError::Config(format!(
            "loader installer is missing data file {name}"
        )));
    };
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|_| AppError::Config(format!("loader installer is missing data file {name}")))?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::File::create(destination)?;
    std::io::copy(&mut entry, &mut output)?;
    Ok(())
}

struct ProcessorOutput {
    file: PathBuf,
    sha1: String,
}

fn resolve_outputs(
    processor: &ProcessorSpec,
    tokens: &HashMap<String, String>,
    libraries: &Path,
) -> AppResult<Vec<ProcessorOutput>> {
    let mut outputs = Vec::new();
    for (key, value) in &processor.outputs {
        let file = if let Some(inner) = bracketed(key) {
            libraries.join(parse_maven(inner)?.to_path())
        } else {
            PathBuf::from(replace_tokens(tokens, key).map_err(AppError::Config)?)
        };
        let sha1 = replace_tokens(tokens, value).map_err(AppError::Config)?;
        outputs.push(ProcessorOutput { file, sha1 });
    }
    Ok(outputs)
}

fn outputs_are_valid(outputs: &[ProcessorOutput]) -> AppResult<bool> {
    for output in outputs {
        if !output.file.is_file() {
            return Ok(false);
        }
        let actual = sha1_file(&output.file)?;
        if !actual.eq_ignore_ascii_case(output.sha1.trim()) {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn run_processor(
    processor: &ProcessorSpec,
    tokens: &HashMap<String, String>,
    libraries: &Path,
    paths: &AppPaths,
    java: &Path,
) -> AppResult<()> {
    let jar = libraries.join(parse_maven(&processor.jar)?.to_path());
    if !jar.is_file() {
        return Err(AppError::Config(format!(
            "loader processor jar is missing: {}",
            jar.display()
        )));
    }
    let main_class = jar_main_class(&jar)?;
    let mut classpath = vec![jar];
    for dependency in &processor.classpath {
        let path = libraries.join(parse_maven(dependency)?.to_path());
        if !path.is_file() {
            return Err(AppError::Config(format!(
                "loader processor dependency is missing: {}",
                path.display()
            )));
        }
        classpath.push(path);
    }
    let mut args = Vec::with_capacity(processor.args.len());
    for arg in &processor.args {
        args.push(process_processor_arg(arg, tokens, libraries)?);
    }

    let cp = classpath
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&classpath_separator().to_string());

    let output = process::command(java)
        .arg("-cp")
        .arg(cp)
        .arg(main_class)
        .args(&args)
        .current_dir(&paths.shared)
        .output()
        .await
        .map_err(|err| AppError::Java(format!("could not run loader processor: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stderr}{stdout}");
        let tail = combined
            .chars()
            .rev()
            .take(1500)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        return Err(AppError::Java(format!(
            "loader processor {} failed (exit {:?}): {tail}",
            processor.jar,
            output.status.code()
        )));
    }
    Ok(())
}

fn classpath_separator() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

pub fn process_processor_arg(
    arg: &str,
    tokens: &HashMap<String, String>,
    libraries: &Path,
) -> AppResult<String> {
    if arg.is_empty() {
        return Ok(String::new());
    }
    if let Some(inner) = bracketed(arg) {
        return Ok(libraries
            .join(parse_maven(inner)?.to_path())
            .to_string_lossy()
            .into_owned());
    }
    replace_tokens(tokens, arg).map_err(AppError::Config)
}

fn parse_maven(raw: &str) -> AppResult<MavenCoordinate> {
    MavenCoordinate::parse(raw).map_err(|err| AppError::Config(err.to_string()))
}

/// Forge's `Util.replaceTokens`: `{KEY}` is substituted, `'literal'` is kept
/// as text, and `\` escapes the next character.
pub fn replace_tokens(tokens: &HashMap<String, String>, value: &str) -> Result<String, String> {
    let chars: Vec<char> = value.chars().collect();
    let mut buf = String::new();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '\\' {
            if index + 1 >= chars.len() {
                return Err(format!("bad escape in processor argument: {value}"));
            }
            buf.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if current == '{' || current == '\'' {
            let mut key = String::new();
            let mut cursor = index + 1;
            let mut closed = false;
            while cursor < chars.len() {
                let next = chars[cursor];
                if next == '\\' {
                    if cursor + 1 >= chars.len() {
                        return Err(format!("bad escape in processor argument: {value}"));
                    }
                    key.push(chars[cursor + 1]);
                    cursor += 2;
                    continue;
                }
                if (current == '{' && next == '}') || (current == '\'' && next == '\'') {
                    index = cursor;
                    closed = true;
                    break;
                }
                key.push(next);
                cursor += 1;
            }
            if !closed {
                return Err(format!("unclosed processor token in: {value}"));
            }
            if current == '\'' {
                buf.push_str(&key);
            } else {
                let Some(token) = tokens.get(&key) else {
                    return Err(format!("missing processor data '{key}'"));
                };
                buf.push_str(token);
            }
            index += 1;
            continue;
        }
        buf.push(current);
        index += 1;
    }
    Ok(buf)
}

fn bracketed(value: &str) -> Option<&str> {
    if value.len() >= 2 && value.starts_with('[') && value.ends_with(']') {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

fn jar_main_class(path: &Path) -> AppResult<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Config(format!("{} is not a jar: {err}", path.display())))?;
    let mut manifest = archive
        .by_name("META-INF/MANIFEST.MF")
        .map_err(|_| AppError::Config(format!("{} has no manifest", path.display())))?;
    let mut text = String::new();
    manifest.read_to_string(&mut text)?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Main-Class:") {
            let class_name = rest.trim();
            if !class_name.is_empty() {
                return Ok(class_name.to_string());
            }
        }
    }
    Err(AppError::Config(format!(
        "{} does not declare a Main-Class",
        path.display()
    )))
}

fn sha1_file(path: &Path) -> AppResult<String> {
    use sha1::{Digest, Sha1};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn version_with_libraries(id: &str, libraries: Vec<Library>) -> VersionJson {
    VersionJson {
        id: id.to_string(),
        inherits_from: None,
        main_class: None,
        assets: None,
        asset_index: None,
        libraries,
        arguments: None,
        minecraft_arguments: None,
        downloads: None,
        java_version: None,
        release_time: None,
        release_type: None,
    }
}

async fn write_profile(paths: &AppPaths, profile: &VersionJson) -> AppResult<()> {
    let dir = paths.version_dir(&profile.id);
    tokio::fs::create_dir_all(&dir).await?;
    let payload = serde_json::to_vec_pretty(profile)?;
    tokio::fs::write(paths.version_json(&profile.id), payload).await?;
    Ok(())
}

async fn get_json<T: for<'de> Deserialize<'de>>(http: &reqwest::Client, url: &str) -> AppResult<T> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("{url} failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "{url} returned HTTP {}",
            response.status()
        )));
    }
    response
        .json()
        .await
        .map_err(|err| AppError::Network(format!("{url} was not valid JSON: {err}")))
}

async fn get_text(http: &reqwest::Client, url: &str) -> AppResult<String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("{url} failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "{url} returned HTTP {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|err| AppError::Network(format!("{url} body failed: {err}")))
}

fn xml_tag_values(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut rest = xml;
    let mut values = Vec::new();
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        values.push(after[..end].trim().to_string());
        rest = &after[end + close.len()..];
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_prefix_follows_the_minecraft_minor() {
        assert_eq!(neoforge_version_prefix("1.21.1").as_deref(), Some("21.1"));
        assert_eq!(neoforge_version_prefix("1.21").as_deref(), Some("21.0"));
        assert_eq!(neoforge_version_prefix("1.20.4").as_deref(), Some("20.4"));
        assert_eq!(neoforge_version_prefix("1.21.10").as_deref(), Some("21.10"));
    }

    #[test]
    fn newest_neoforge_build_ignores_other_minecraft_versions_and_betas() {
        let versions = [
            "21.1.9",
            "21.1.10",
            "21.10.1",
            "21.1.251-beta",
            "20.4.230",
            "21.1.251",
        ];
        assert_eq!(
            newest_neoforge_version("1.21.1", versions).as_deref(),
            Some("21.1.251")
        );
        assert_eq!(
            newest_neoforge_version("1.21.10", versions).as_deref(),
            Some("21.10.1")
        );
        assert!(newest_neoforge_version("1.19.2", versions).is_none());
    }

    #[test]
    fn forge_promo_prefers_recommended_and_strips_the_game_prefix() {
        let promos = serde_json::json!({
            "promos": {
                "1.21.1-latest": "52.1.16",
                "1.21.1-recommended": "52.1.0",
                "1.21-latest": "51.0.33"
            }
        });
        assert_eq!(
            forge_promo_version(&promos, "1.21.1").as_deref(),
            Some("52.1.0")
        );
        assert_eq!(
            forge_promo_version(&promos, "1.21").as_deref(),
            Some("51.0.33")
        );
        assert_eq!(forge_loader_version("1.21.1", "1.21.1-52.1.0"), "52.1.0");
        assert_eq!(
            forge_loader_version("1.21.1", "1.21.1-forge-52.1.0"),
            "52.1.0"
        );
    }

    #[test]
    fn meta_loader_prefers_stable_then_non_prerelease() {
        let entries = vec![
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.20.0-beta.9".into(),
                    stable: false,
                },
            },
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.19.5".into(),
                    stable: true,
                },
            },
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.24.0".into(),
                    stable: false,
                },
            },
        ];
        assert_eq!(pick_meta_loader(&entries).as_deref(), Some("0.19.5"));

        let quilt = vec![
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.20.0-beta.9".into(),
                    stable: false,
                },
            },
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.23.1".into(),
                    stable: false,
                },
            },
            MetaLoaderEntry {
                loader: MetaLoaderVersion {
                    version: "0.24.0".into(),
                    stable: false,
                },
            },
        ];
        assert_eq!(pick_meta_loader(&quilt).as_deref(), Some("0.24.0"));
    }

    #[test]
    fn processor_tokens_substitute_and_maven_args_become_paths() {
        let mut tokens = HashMap::new();
        tokens.insert("ROOT".into(), "/shared".into());
        tokens.insert("SIDE".into(), "client".into());
        tokens.insert("BINPATCH".into(), "/tmp/data/client.lzma".into());
        assert_eq!(
            replace_tokens(&tokens, "{ROOT}/libraries/unix_args.txt").unwrap(),
            "/shared/libraries/unix_args.txt"
        );
        assert_eq!(
            replace_tokens(&tokens, "--side {SIDE} 'kept'").unwrap(),
            "--side client kept"
        );
        let libraries = Path::new("/libraries");
        let arg = process_processor_arg(
            "[net.minecraft:client:1.21.1:mappings@tsrg]",
            &tokens,
            libraries,
        )
        .unwrap();
        assert!(arg.ends_with("client-1.21.1-mappings.tsrg"));
        assert!(arg.contains("net/minecraft/client/1.21.1"));
    }

    #[test]
    fn xml_version_tags_are_listed_in_order() {
        let xml = r#"<metadata><versioning><latest>21.1.2</latest><versions><version>21.1.1</version><version>21.1.2</version></versions></versioning></metadata>"#;
        assert_eq!(
            xml_tag_values(xml, "version"),
            vec!["21.1.1".to_string(), "21.1.2".to_string()]
        );
    }
}
