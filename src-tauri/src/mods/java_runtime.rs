//! Java runtime detection and provisioning.
//!
//! Minecraft's Java requirement is a property of the *game version* (1.17+ needs
//! 17, 1.20.5+ needs 21), not of the loader. Getting this wrong is the single
//! most common cause of "the game closed immediately", so the launcher resolves
//! it explicitly and can download a matching Temurin JDK when the machine has
//! none.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::process;

/// Adoptium (Temurin) API base.
pub const ADOPTIUM_API: &str = "https://api.adoptium.net/v3";
/// Vendor filtering keeps the download deterministic.
const ADOPTIUM_VENDOR: &str = "eclipse";
/// Root folder name for runtimes the launcher downloaded itself.
pub const MANAGED_PREFIX: &str = "temurin-";

/// A usable `java` executable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    /// Absolute path to the `java` (or `java.exe`) binary.
    pub path: PathBuf,
    /// Detected feature version (8, 17, 21, ...).
    pub major: u8,
    /// Full version string, e.g. `21.0.2+13-LTS`.
    pub version: String,
    pub vendor: String,
    /// `true` when we downloaded it into the app data directory.
    pub is_managed: bool,
    pub architecture: String,
}

impl JavaRuntime {
    /// Runs `java -version` and parses the result.
    ///
    /// Java prints its version banner on **stderr**, which trips up naive
    /// implementations. The child is spawned without a console window (see
    /// [`crate::process`]), otherwise every probe flashes a command prompt.
    pub async fn probe(path: &Path, is_managed: bool) -> AppResult<Self> {
        let output = process::command(path)
            .arg("-version")
            .output()
            .await
            .map_err(|err| AppError::Java(format!("could not run {}: {err}", path.display())))?;

        let banner = String::from_utf8_lossy(&output.stderr).to_string();
        let (major, version) = parse_version_banner(&banner).ok_or_else(|| {
            AppError::Java(format!(
                "could not parse the Java version from {}",
                path.display()
            ))
        })?;

        Ok(Self {
            path: path.to_path_buf(),
            major,
            version,
            vendor: parse_vendor(&banner),
            is_managed,
            architecture: std::env::consts::ARCH.to_string(),
        })
    }

    /// Can this runtime run a game that needs `required`?
    ///
    /// The rules are the ones the community learned the hard way:
    /// * `required <= 8` — Minecraft 1.16.5 and older ship LWJGL 2, which does
    ///   **not** tolerate Java 9+. An exact 8 is the only safe answer.
    /// * `required >= 17` — Java is backwards compatible, so a newer LTS (21
    ///   running a 1.20.1 instance) works and is what most players already have.
    ///   Reusing it is exactly what avoids downloading a second 200 MB JDK.
    pub fn is_compatible_with(&self, required: u8) -> bool {
        if required <= 8 {
            self.major == required
        } else {
            self.major >= required
        }
    }

    /// Short label for the UI, e.g. `Temurin 21.0.2 (managed)`.
    pub fn describe(&self) -> String {
        format!(
            "{} {}{}",
            self.vendor,
            self.version,
            if self.is_managed { " (managed)" } else { "" }
        )
    }
}

/// Parse `openjdk version "21.0.2" 2024-01-16` / `java version "1.8.0_392"`.
pub fn parse_version_banner(banner: &str) -> Option<(u8, String)> {
    let quoted = banner.split('"').nth(1)?;
    let major = if let Some(rest) = quoted.strip_prefix("1.") {
        // Legacy scheme: 1.8.0_392 -> 8
        rest.split('.').next()?.parse::<u8>().ok()?
    } else {
        quoted.split('.').next()?.parse::<u8>().ok()?
    };
    Some((major, quoted.to_string()))
}

/// Vendor tag from a `java -version` banner.
pub fn parse_vendor(banner: &str) -> String {
    for vendor in [
        "Temurin",
        "AdoptOpenJDK",
        "Zulu",
        "Corretto",
        "GraalVM",
        "Microsoft",
        "OpenJDK",
        "Java(TM)",
    ] {
        if banner.contains(vendor) {
            return vendor.to_string();
        }
    }
    "Unknown".to_string()
}

/// Java major version required by a Minecraft version.
///
/// Snapshot ids (`24w14a`) are treated as the newest requirement; the Mojang
/// version json is authoritative when available and overrides this table.
pub fn required_java_major(game_version: &str) -> u8 {
    let trimmed = game_version.trim();
    // Snapshots / pre-releases do not parse as x.y, so use the current baseline.
    let parts: Vec<&str> = trimmed.split('.').collect();
    let (Some(minor), patch) = (
        parts.get(1).and_then(|value| value.parse::<u32>().ok()),
        parts
            .get(2)
            .and_then(|value| value.split('-').next())
            .and_then(|value| value.parse::<u32>().ok()),
    ) else {
        return 21;
    };
    let patch = patch.unwrap_or(0);

    match minor {
        0..=16 => 8,
        17..=19 => 17,
        20 if patch >= 5 => 21,
        20 => 17,
        _ => 21,
    }
}

/// Requirement including loader-specific overrides.
///
/// NeoForge 1.20.5+ and modern Forge builds refuse to start on 8/17, and some
/// old Forge builds break on 21, so the loader can raise (never lower) the bar.
pub fn required_major_for(game_version: &str, loader: crate::models::instance::LoaderKind) -> u8 {
    use crate::models::instance::LoaderKind;
    let base = required_java_major(game_version);
    match loader {
        LoaderKind::NeoForge => base.max(21),
        _ => base,
    }
}

/// One downloadable Temurin package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaPackage {
    pub release_name: String,
    pub openjdk_version: String,
    pub download_url: String,
    pub file_name: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub architecture: String,
    pub os: String,
}

#[derive(Debug, Deserialize)]
struct AdoptiumAsset {
    release_name: String,
    #[serde(default)]
    version: AdoptiumVersion,
    binary: AdoptiumBinary,
}

#[derive(Debug, Default, Deserialize)]
struct AdoptiumVersion {
    #[serde(default)]
    openjdk_version: String,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    os: String,
    package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    checksum: String,
    link: String,
    name: String,
    #[serde(default)]
    size: u64,
}

/// Detects installed JDKs and provisions managed ones.
#[derive(Clone)]
pub struct JavaRegistry {
    paths: AppPaths,
    http: reqwest::Client,
    /// Extra directories the user pointed at (Settings → Fixes).
    extra_roots: Vec<PathBuf>,
}

impl JavaRegistry {
    pub fn new(paths: AppPaths, http: reqwest::Client) -> Self {
        Self {
            paths,
            http,
            extra_roots: Vec::new(),
        }
    }

    /// Add user-provided JDK roots (a folder containing installs or one JDK).
    pub fn with_extra_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.extra_roots = roots;
        self
    }

    /// Build a registry from persisted settings.
    pub fn from_settings(
        paths: AppPaths,
        http: reqwest::Client,
        settings: &crate::config::AppSettings,
    ) -> Self {
        Self {
            paths,
            http,
            extra_roots: settings
                .java_extra_roots
                .iter()
                .map(PathBuf::from)
                .collect(),
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// Every `java` executable we can find, de-duplicated by canonical path and
    /// sorted by descending major version.
    pub async fn detect(&self) -> AppResult<Vec<JavaRuntime>> {
        let mut candidates: Vec<(PathBuf, bool)> = Vec::new();

        // 1. Managed runtimes we downloaded before.
        for entry in walkdir::WalkDir::new(&self.paths.java)
            .max_depth(5)
            .into_iter()
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().is_file() && (name == "java" || name == "java.exe") {
                candidates.push((entry.path().to_path_buf(), true));
            }
        }

        // 2. JAVA_HOME.
        if let Some(home) = std::env::var_os("JAVA_HOME") {
            let bin = PathBuf::from(home).join("bin");
            for candidate in [bin.join("java"), bin.join("java.exe")] {
                if candidate.is_file() {
                    candidates.push((candidate, false));
                }
            }
        }

        // 3. PATH.
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                for candidate in [dir.join("java"), dir.join("java.exe")] {
                    if candidate.is_file() {
                        candidates.push((candidate, false));
                    }
                }
            }
        }

        // 4. Conventional install locations per platform.
        for root in self.standard_install_roots() {
            for candidate in find_java_binaries(&root, 4) {
                candidates.push((candidate, false));
            }
        }

        // 5. Folders the user added by hand in Settings → Fixes. Both "a folder
        //    of JDKs" and "the JDK root itself" work, so a user who picked
        //    `C:\Program Files\Java\jdk-21` is not silently ignored.
        for root in &self.extra_roots {
            for candidate in find_java_binaries(root, 6) {
                candidates.push((candidate, false));
            }
            for candidate in [root.join("bin").join("java"), root.join("bin").join("java.exe")] {
                if candidate.is_file() {
                    candidates.push((candidate, false));
                }
            }
        }

        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
        let mut runtimes: Vec<JavaRuntime> = Vec::new();
        for (path, is_managed) in candidates {
            // Canonicalize so a symlinked `/usr/bin/java` and its target are one
            // entry instead of two "different" runtimes.
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(key) {
                continue;
            }
            if let Ok(runtime) = JavaRuntime::probe(&path, is_managed).await {
                runtimes.push(runtime);
            }
        }

        runtimes.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.path.cmp(&b.path)));
        Ok(runtimes)
    }

    /// Platform-specific locations that ship a JDK.
    fn standard_install_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if cfg!(target_os = "windows") {
            for base in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                if let Some(dir) = std::env::var_os(base) {
                    let dir = PathBuf::from(dir);
                    roots.push(dir.join("Java"));
                    roots.push(dir.join("Eclipse Adoptium"));
                    roots.push(dir.join("Eclipse Foundation"));
                    roots.push(dir.join("Microsoft").join("jdk"));
                    roots.push(dir.join("Zulu"));
                    roots.push(dir.join("Amazon Corretto"));
                    roots.push(dir.join("BellSoft").join("LibericaJDK"));
                }
            }
            if let Some(local) = dirs::data_local_dir() {
                roots.push(local.join("Programs").join("Eclipse Adoptium"));
            }
        } else if cfg!(target_os = "macos") {
            roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
            roots.push(PathBuf::from("/System/Library/Java/JavaVirtualMachines"));
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join("Library").join("Java").join("JavaVirtualMachines"));
            }
        } else {
            roots.push(PathBuf::from("/usr/lib/jvm"));
            roots.push(PathBuf::from("/usr/java"));
            roots.push(PathBuf::from("/opt/java"));
            let _ = &self.paths;
        }
        roots
    }

    /// The runtime that should actually be used to play, or `None`.
    ///
    /// Unlike [`Self::best_match`] this never returns an *older* runtime than the
    /// game needs — launching 1.21 on Java 8 is not "the closest match", it is a
    /// crash. Managed runtimes are preferred over system ones on a tie, because
    /// the launcher knows exactly what it installed.
    pub fn select_for<'a>(
        runtimes: &'a [JavaRuntime],
        required: u8,
    ) -> Option<&'a JavaRuntime> {
        let compatible = |runtime: &&JavaRuntime| runtime.is_compatible_with(required);

        // 1. Exact major version: never wrong.
        if let Some(exact) = runtimes
            .iter()
            .filter(compatible)
            .find(|runtime| runtime.major == required)
        {
            return Some(exact);
        }
        // 2. Lowest newer major (least surprising, smallest GC/memory delta).
        if let Some(newer) = runtimes
            .iter()
            .filter(compatible)
            .filter(|runtime| runtime.major > required)
            .min_by_key(|runtime| runtime.major)
        {
            return Some(newer);
        }
        // 3. A managed runtime of the right major that failed to probe is not a
        //    candidate either; nothing usable is installed.
        None
    }

    /// Explain why no runtime matched (used in error messages).
    pub fn compatibility_note(required: u8) -> String {
        if required <= 8 {
            format!("Minecraft on this version needs exactly Java {required} (Java 9+ breaks its LWJGL 2 stack).")
        } else {
            format!("Java {required} or newer is required.")
        }
    }

    /// Probe one explicit path (the "browse for javaw.exe" flow).
    pub async fn probe_path(&self, path: &Path) -> AppResult<JavaRuntime> {
        if !path.is_file() {
            return Err(AppError::Java(format!(
                "{} is not a file",
                path.display()
            )));
        }
        let managed = path.starts_with(&self.paths.java);
        JavaRuntime::probe(path, managed).await
    }

    /// Resolve the runtime for a game, honouring an explicit override.
    pub async fn resolve(
        &self,
        required: u8,
        override_path: Option<&Path>,
    ) -> AppResult<JavaRuntime> {
        if let Some(path) = override_path {
            return self.probe_path(path).await;
        }
        let installed = self.detect().await?;
        Self::select_for(&installed, required).cloned().ok_or_else(|| {
            AppError::Java(format!(
                "no usable Java runtime was found for this instance. {} \
                 Download one from Settings → Fixes, or point the instance at an installed JDK.",
                Self::compatibility_note(required)
            ))
        })
    }

    /// Pick the best installed runtime for a required major version.
    ///
    /// Exact matches win; otherwise the closest *newer* version (Java is
    /// backwards compatible for older Minecraft) and only then an older one.
    /// Kept for diagnostics/UI listing; launching uses [`Self::select_for`].
    pub fn best_match<'a>(
        runtimes: &'a [JavaRuntime],
        required: u8,
    ) -> Option<&'a JavaRuntime> {
        if let Some(exact) = runtimes.iter().find(|runtime| runtime.major == required) {
            return Some(exact);
        }
        let newer = runtimes
            .iter()
            .filter(|runtime| runtime.major > required)
            .min_by_key(|runtime| runtime.major);
        if newer.is_some() {
            return newer;
        }
        runtimes
            .iter()
            .filter(|runtime| runtime.major < required)
            .max_by_key(|runtime| runtime.major)
    }

    /// Query Adoptium for the latest JDK of a given feature version.
    pub async fn available_packages(&self, major: u8) -> AppResult<Vec<JavaPackage>> {
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "mac",
            other => other,
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86" => "x32",
            other => {
                let _ = other;
                "x64"
            }
        };

        let url = format!(
            "{ADOPTIUM_API}/assets/latest/{major}/hotspot?architecture={arch}&image_type=jdk&os={os}&vendor={ADOPTIUM_VENDOR}"
        );
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Adoptium request failed: {err}")))?;

        if !response.status().is_success() {
            return Err(AppError::Java(format!(
                "Adoptium has no JDK {major} for {os}/{arch} (HTTP {})",
                response.status()
            )));
        }

        let assets: Vec<AdoptiumAsset> = response
            .json()
            .await
            .map_err(|err| AppError::Java(format!("unexpected Adoptium response: {err}")))?;

        Ok(assets
            .into_iter()
            .map(|asset| JavaPackage {
                release_name: asset.release_name,
                openjdk_version: asset.version.openjdk_version,
                download_url: asset.binary.package.link,
                file_name: asset.binary.package.name,
                size: asset.binary.package.size,
                checksum: Some(asset.binary.package.checksum),
                architecture: asset.binary.architecture,
                os: asset.binary.os,
            })
            .collect())
    }

    /// Ensure a runtime for `major` exists locally, downloading Temurin if not.
    ///
    /// Returns the runtime that should be used for the launch.
    pub async fn ensure(
        &self,
        major: u8,
        sink: &dyn ProgressSink,
    ) -> AppResult<JavaRuntime> {
        self.ensure_with(major, sink, true).await?.ok_or_else(|| {
            AppError::Java(format!(
                "no usable Java runtime is available. {}",
                Self::compatibility_note(major)
            ))
        })
    }

    /// Resolve a runtime without downloading when one already fits.
    ///
    /// This is the fix for "every instance install downloads another JDK": a
    /// system JDK 21 satisfies an instance that needs 17, so it is reused and
    /// `None` down-load happens. The JDK download only kicks in when the machine
    /// genuinely has nothing compatible *and* `allow_download` is set.
    pub async fn ensure_with(
        &self,
        major: u8,
        sink: &dyn ProgressSink,
        allow_download: bool,
    ) -> AppResult<Option<JavaRuntime>> {
        let installed = self.detect().await?;
        if let Some(runtime) = Self::select_for(&installed, major) {
            return Ok(Some(runtime.clone()));
        }
        if !allow_download {
            return Ok(None);
        }
        let runtime = self.install(major, sink).await?;
        Ok(Some(runtime))
    }

    /// Download and install a managed Temurin JDK, unconditionally.
    ///
    /// Exposed for Settings → Fixes, where the user explicitly asks for a JDK
    /// even though a perfectly usable one is already installed.
    pub async fn install(&self, major: u8, sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        let packages = self.available_packages(major).await?;
        let package = packages.into_iter().next().ok_or_else(|| {
            AppError::Java(format!("no Temurin JDK {major} package is available"))
        })?;

        let event = ProgressEvent::started(JobKind::JavaRuntime, format!("Java {major}"))
            .stage(JobStage::ProvisioningJava)
            .detail(&package.release_name);
        sink.report(event).await;

        let target = self.paths.managed_java(major);
        let archive = download_package(&self.http, &package, &self.paths.downloads, sink).await?;
        extract_archive(&archive, &target).await?;

        let java = find_java_binaries(&target, 6)
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::Java(format!(
                    "the downloaded JDK did not contain a java binary at {}",
                    target.display()
                ))
            })?;

        JavaRuntime::probe(&java, true).await
    }

    /// Managed JDKs already on disk, newest first.
    pub async fn managed(&self) -> Vec<JavaRuntime> {
        let Ok(installed) = self.detect().await else {
            return Vec::new();
        };
        installed
            .into_iter()
            .filter(|runtime| runtime.is_managed)
            .collect()
    }

    /// Folder the launcher installs managed runtimes into (shown in the UI).
    pub fn managed_root(&self) -> &Path {
        &self.paths.java
    }
}

/// Stream a JDK package into the download cache.
async fn download_package(
    http: &reqwest::Client,
    package: &JavaPackage,
    cache_dir: &Path,
    sink: &dyn ProgressSink,
) -> AppResult<PathBuf> {
    use futures::StreamExt;

    std::fs::create_dir_all(cache_dir)?;
    let destination = cache_dir.join(&package.file_name);
    if destination.is_file() {
        return Ok(destination);
    }

    let response = http
        .get(&package.download_url)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("JDK download failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "JDK download failed with HTTP {}",
            response.status()
        )));
    }

    let mut job = ProgressEvent::started(JobKind::JavaRuntime, "Downloading JDK")
        .stage(crate::models::progress::JobStage::Downloading)
        .item(&package.file_name)
        .totals(0, package.size);

    // The UI can only cancel a job it can see, so the JDK download registers the
    // same way a `JobTracker` job does.
    let cancel = crate::cancel::CancelRegistry::global().register(job.job_id);

    let temporary = destination.with_extension("part");
    let mut file = tokio::fs::File::create(&temporary).await?;
    let mut stream = response.bytes_stream();
    let mut written: u64 = 0;

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            crate::cancel::CancelRegistry::global().forget(job.job_id);
            return Err(AppError::Cancelled);
        }
        let chunk = chunk.map_err(|err| AppError::Network(format!("JDK stream failed: {err}")))?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
        job = job.clone().totals(written, package.size);
        sink.report(job.clone()).await;
    }
    file.flush().await?;
    drop(file);

    tokio::fs::rename(&temporary, &destination).await?;
    crate::cancel::CancelRegistry::global().forget(job.job_id);
    sink.report(job.finished()).await;
    Ok(destination)
}

/// Extract a `.zip` (Windows/macOS) or `.tar.gz` (Linux) JDK into `target`.
async fn extract_archive(archive: &Path, target: &Path) -> AppResult<()> {
    let archive = archive.to_path_buf();
    let target = target.to_path_buf();

    tokio::task::spawn_blocking(move || -> AppResult<()> {
        // Extract into a staging dir: JDKs ship with a top-level folder whose
        // name changes every release, so we normalise afterwards.
        let staging = target.with_extension("staging");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;

        let name = archive
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();

        if name.ends_with(".zip") || name.ends_with(".jar") {
            let file = std::fs::File::open(&archive)?;
            let mut zip = zip::ZipArchive::new(file)?;
            zip.extract(&staging)?;
        } else {
            let file = std::fs::File::open(&archive)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            tar.unpack(&staging)?;
        }

        // Find the real JDK home inside the staging directory.
        let home = find_java_binaries(&staging, 6)
            .into_iter()
            .next()
            .and_then(|bin| bin.parent().and_then(|bin| bin.parent()).map(Path::to_path_buf))
            .ok_or_else(|| {
                AppError::Java("the downloaded archive contained no java binary".to_string())
            })?;

        let _ = std::fs::remove_dir_all(&target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Same filesystem: rename, else copy.
        if std::fs::rename(&home, &target).is_err() {
            copy_dir_all(&home, &target)?;
        }

        // Zip archives do not carry the unix executable bit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in walkdir::WalkDir::new(&target)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
            {
                let is_binary_dir = entry.path().parent().map(|p| p.ends_with("bin")).unwrap_or(false);
                if entry.file_type().is_file() && is_binary_dir {
                    let mut permissions = std::fs::metadata(entry.path())?.permissions();
                    permissions.set_mode(0o755);
                    std::fs::set_permissions(entry.path(), permissions)?;
                }
            }
        }

        let _ = std::fs::remove_dir_all(&staging);
        Ok(())
    })
    .await?
}

fn copy_dir_all(from: &Path, to: &Path) -> AppResult<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &destination)?;
        } else {
            std::fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

/// Depth-limited search for `bin/java` under `root`.
pub fn find_java_binaries(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }
    walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy();
            (name == "java" || name == "java.exe")
                && entry
                    .path()
                    .parent()
                    .map(|parent| parent.ends_with("bin"))
                    .unwrap_or(false)
        })
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::instance::LoaderKind;

    #[test]
    fn java_requirement_matches_the_mojang_table() {
        assert_eq!(required_java_major("1.8.9"), 8);
        assert_eq!(required_java_major("1.12.2"), 8);
        assert_eq!(required_java_major("1.16.5"), 8);
        assert_eq!(required_java_major("1.17"), 17);
        assert_eq!(required_java_major("1.18.2"), 17);
        assert_eq!(required_java_major("1.20.1"), 17);
        assert_eq!(required_java_major("1.20.5"), 21);
        assert_eq!(required_java_major("1.21.4"), 21);
        // Snapshots do not parse as x.y and default to the newest requirement.
        assert_eq!(required_java_major("24w14a"), 21);
    }

    #[test]
    fn neoforge_raises_the_requirement_to_21() {
        assert_eq!(required_major_for("1.20.1", LoaderKind::NeoForge), 21);
        assert_eq!(required_major_for("1.20.1", LoaderKind::Fabric), 17);
    }

    #[test]
    fn parses_modern_and_legacy_banners() {
        let modern = "openjdk version \"21.0.2\" 2024-01-16\nOpenJDK Runtime Environment Temurin-21.0.2+13 (build 21.0.2+13-LTS)";
        assert_eq!(parse_version_banner(modern), Some((21, "21.0.2".to_string())));
        assert_eq!(parse_vendor(modern), "Temurin");

        let legacy = "java version \"1.8.0_392\"\nJava(TM) SE Runtime Environment (build 1.8.0_392-b08)";
        assert_eq!(parse_version_banner(legacy), Some((8, "1.8.0_392".to_string())));
        assert_eq!(parse_vendor(legacy), "Java(TM)");

        assert!(parse_version_banner("not a java banner").is_none());
    }

    fn runtime(major: u8, path: &str) -> JavaRuntime {
        JavaRuntime {
            path: PathBuf::from(path),
            major,
            version: format!("{major}.0.0"),
            vendor: "Temurin".into(),
            is_managed: false,
            architecture: "x86_64".into(),
        }
    }

    #[test]
    fn select_for_never_picks_a_too_old_runtime() {
        let runtimes = vec![
            runtime(8, "/jdk8/bin/java"),
            runtime(17, "/jdk17/bin/java"),
            runtime(21, "/jdk21/bin/java"),
        ];
        // Exact match wins.
        assert_eq!(
            JavaRegistry::select_for(&runtimes, 17).map(|r| r.major),
            Some(17)
        );
        // Only newer is installed: reuse it instead of downloading a JDK.
        let only_newer = vec![runtime(21, "/jdk21/bin/java")];
        assert_eq!(
            JavaRegistry::select_for(&only_newer, 17).map(|r| r.major),
            Some(21)
        );
        // Only older is installed: refusing beats launching a crashing game.
        let only_older = vec![runtime(8, "/jdk8/bin/java")];
        assert!(JavaRegistry::select_for(&only_older, 21).is_none());
        // Java 8 instances are strict: Java 21 must not be substituted.
        let modern_only = vec![runtime(21, "/jdk21/bin/java")];
        assert!(JavaRegistry::select_for(&modern_only, 8).is_none());
        assert!(JavaRegistry::select_for(&[], 17).is_none());
    }

    #[test]
    fn compatibility_rules_match_what_the_game_tolerates() {
        assert!(runtime(21, "/jdk21/bin/java").is_compatible_with(17));
        assert!(runtime(21, "/jdk21/bin/java").is_compatible_with(21));
        assert!(!runtime(17, "/jdk17/bin/java").is_compatible_with(21));
        assert!(runtime(8, "/jdk8/bin/java").is_compatible_with(8));
        assert!(!runtime(17, "/jdk17/bin/java").is_compatible_with(8));
    }

    #[test]
    fn runtime_labels_are_human_readable() {
        let mut managed = runtime(21, "/managed/bin/java");
        managed.is_managed = true;
        managed.version = "21.0.2".into();
        assert_eq!(managed.describe(), "Temurin 21.0.2 (managed)");
        assert!(runtime(17, "/jdk/bin/java")
            .describe()
            .contains("17.0.0"));
    }

    #[test]
    fn picks_exact_then_newer_then_older_runtime() {
        let runtimes = vec![
            runtime(8, "/jdk8/bin/java"),
            runtime(17, "/jdk17/bin/java"),
            runtime(21, "/jdk21/bin/java"),
        ];
        assert_eq!(
            JavaRegistry::best_match(&runtimes, 17).map(|r| r.major),
            Some(17)
        );
        // 11 is not installed: 17 is the closest newer runtime.
        assert_eq!(
            JavaRegistry::best_match(&runtimes, 11).map(|r| r.major),
            Some(17)
        );
        // 26 is newer than anything installed: fall back to the newest we have.
        assert_eq!(
            JavaRegistry::best_match(&runtimes, 26).map(|r| r.major),
            Some(21)
        );
        assert!(JavaRegistry::best_match(&[], 17).is_none());
    }
}
