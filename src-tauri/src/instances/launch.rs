//! Building and spawning the Minecraft process.
//!
//! The launcher never guesses arguments: it reads the resolved `versions/<id>/<id>.json`
//! and substitutes Mojang's placeholders exactly as the official launcher does.
//! Getting a placeholder wrong produces a game that starts and then vanishes, so
//! every substitution is explicit and unknown `--flag`-less placeholders are
//! detected rather than silently passed through.
//!
//! ## Joining a P2P session
//!
//! [`LaunchExtras::connect`] injects `--server 127.0.0.1 --port <bridge>` so the
//! player lands in the host's world directly. Because the local port is
//! allocated per session (never hardcoded to 25565) several sessions can be open
//! at once without fighting over a port.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::models::account::LaunchIdentity;
use crate::models::instance::Instance;
use crate::models::version::{Arguments, FeatureSet, VersionJson};
use crate::mods::java_runtime::{JavaRegistry, JavaRuntime};

/// Classpath separator for the running platform.
pub fn classpath_separator() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

/// Extra options layered on top of the instance config for one launch.
#[derive(Debug, Clone, Default)]
pub struct LaunchExtras {
    /// Connect straight into a P2P-hosted world.
    pub connect: Option<SocketAddr>,
    /// `--quickPlaySingleplayer <world>` support.
    pub quick_play_world: Option<String>,
    /// Demo mode flag.
    pub demo: bool,
    /// Custom resolution override (wins over the instance config).
    pub resolution: Option<(u32, u32)>,
}

/// A fully resolved, ready-to-spawn command.
///
/// Mojang's required argv order is `java [jvm args] MainClass [game args]`.
/// JVM and game arguments are kept separate so spawn can never splice the
/// main class into the wrong place.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub java: PathBuf,
    pub main_class: String,
    pub classpath: Vec<PathBuf>,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub working_directory: PathBuf,
    pub env: HashMap<String, String>,
    pub log_file: PathBuf,
    pub required_java_major: u8,
}

impl LaunchPlan {
    /// Full argv, with the executable first (for display/diagnostics).
    pub fn command_line(&self) -> Vec<String> {
        let mut argv = Vec::with_capacity(self.jvm_args.len() + self.game_args.len() + 4);
        argv.push(self.java.to_string_lossy().into_owned());
        argv.extend(self.jvm_args.iter().cloned());
        argv.push(self.main_class.clone());
        argv.extend(self.game_args.iter().cloned());
        argv
    }

    /// Redacted single-line command for the logs (tokens removed).
    pub fn redacted_command_line(&self) -> String {
        let mut argv = self.command_line();
        for index in 0..argv.len() {
            // `--session` is the pre-1.6 spelling of the access token.
            if matches!(argv[index].as_str(), "--accessToken" | "--session" | "--clientId") {
                if index + 1 < argv.len() {
                    argv[index + 1] = "<redacted>".to_string();
                }
            }
        }
        argv.join(" ")
    }

    pub fn display_classpath(&self) -> String {
        self.classpath
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(&classpath_separator().to_string())
    }
}

/// Builds launch plans for instances.
pub struct LaunchPlanner {
    paths: AppPaths,
    /// Detected runtimes, if the caller already probed them.
    runtimes: Vec<JavaRuntime>,
}

impl LaunchPlanner {
    pub fn new(paths: AppPaths, runtimes: Vec<JavaRuntime>) -> Self {
        Self { paths, runtimes }
    }

    /// Assemble the complete command for an instance.
    pub fn build(
        &self,
        instance: &Instance,
        version: &VersionJson,
        identity: &LaunchIdentity,
        extras: &LaunchExtras,
    ) -> AppResult<LaunchPlan> {
        let config = &instance.config;
        let root = self.paths.instance(config.id).root();
        // Natives extracted for the vanilla parent live beside that version,
        // not beside the loader profile id.
        let natives_id = version
            .inherits_from
            .as_deref()
            .unwrap_or(version.id.as_str());
        let natives = self.paths.natives().join(natives_id);
        // LWJGL refuses to extract into a missing directory, and pre-1.19
        // versions load `.dll`/`.so` files from here. 1.19+ never extracts
        // during install (natives are jars on the classpath), so create it
        // at launch or the process exits immediately.
        std::fs::create_dir_all(&natives)?;
        let classpath = self.build_classpath(version, &root)?;

        if classpath.is_empty() {
            return Err(AppError::Java(format!(
                "no libraries are installed for {} — run the installer first",
                version.id
            )));
        }

        let java = self.select_java(instance)?;
        let features = FeatureSet {
            is_demo_user: extras.demo,
            has_custom_resolution: true,
            is_quick_play_singleplayer: extras.quick_play_world.is_some(),
            ..FeatureSet::default()
        };

        let (width, height) = extras
            .resolution
            .unwrap_or((config.resolution.width, config.resolution.height));

        let mut substitutions: HashMap<&str, String> = HashMap::new();
        substitutions.insert("auth_player_name", identity.username.clone());
        substitutions.insert("version_name", version.id.clone());
        substitutions.insert("game_directory", root.to_string_lossy().into_owned());
        substitutions.insert(
            "assets_root",
            self.paths
                .shared
                .join("assets")
                .to_string_lossy()
                .into_owned(),
        );
        substitutions.insert(
            "assets_index_name",
            version
                .assets
                .clone()
                .unwrap_or_else(|| "legacy".to_string()),
        );
        substitutions.insert("auth_uuid", identity.uuid.clone());
        substitutions.insert("auth_access_token", identity.access_token.clone());
        substitutions.insert("auth_session", identity.access_token.clone());
        substitutions.insert("user_type", identity.user_type.as_str().to_string());
        substitutions.insert(
            "version_type",
            identity.version_type(version.release_type.as_deref()),
        );
        substitutions.insert("natives_directory", natives.to_string_lossy().into_owned());
        substitutions.insert("launcher_name", "SXMLAUNCHER".to_string());
        substitutions.insert("launcher_version", env!("CARGO_PKG_VERSION").to_string());
        substitutions.insert("classpath", join_paths(&classpath));
        substitutions.insert("classpath_separator", classpath_separator().to_string());
        substitutions.insert(
            "library_directory",
            self.paths.libraries().to_string_lossy().into_owned(),
        );
        substitutions.insert("resolution_width", width.to_string());
        substitutions.insert("resolution_height", height.to_string());
        substitutions.insert("auth_xuid", identity.xuid.clone().unwrap_or_default());
        substitutions.insert("clientid", identity.client_id.clone().unwrap_or_default());
        substitutions.insert("user_properties", "{}".to_string());
        substitutions.insert(
            "quickPlayPath",
            extras
                .quick_play_world
                .as_ref()
                .map(|_| {
                    self.paths
                        .root
                        .join("quickPlay")
                        .join("quickPlayLog.json")
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_default(),
        );
        substitutions.insert(
            "quickPlaySingleplayer",
            extras.quick_play_world.clone().unwrap_or_default(),
        );
        substitutions.insert("game_assets", String::new());
        substitutions.insert("auth_session_id", String::new());
        substitutions.insert("profile_name", String::new());

        // --- JVM arguments -------------------------------------------------
        let mut jvm: Vec<String> = match &version.arguments {
            Some(arguments) => Arguments::flatten(&arguments.jvm, &features)
                .into_iter()
                .map(|token| substitute(token, &substitutions))
                .collect(),
            // 1.12.2 and older (and Forge profiles that inherit them) have no
            // `arguments` object — only `minecraftArguments`. The placeholders
            // still have to be filled in. Leaving them literal starts a JVM
            // whose classpath is the string `${classpath}`, which prints
            // "Could not find or load main class" and exits 1.
            None => vec![
                substitute("-Djava.library.path=${natives_directory}", &substitutions),
                "-cp".to_string(),
                substitute("${classpath}", &substitutions),
            ],
        };

        // Memory + our own flags come first so an instance can still override
        // them by appending later flags.
        let memory = config.memory.sanitized();
        let mut prefix = vec![
            format!("-Xms{}M", memory.min_mb),
            format!("-Xmx{}M", memory.max_mb),
            "-XX:+UnlockExperimentalVMOptions".to_string(),
            "-XX:+UseG1GC".to_string(),
            "-Dfile.encoding=UTF-8".to_string(),
            // Keep the official launcher's newline handling (matters for log parsers).
            "-Dstdout.encoding=UTF-8".to_string(),
            "-Dstderr.encoding=UTF-8".to_string(),
        ];
        prefix.append(&mut jvm);
        jvm = prefix;

        // Ely.by and other Yggdrasil servers need authlib-injector. The jar is
        // provisioned by the caller (see `ModEngine::ensure_authlib_injector`),
        // so a missing file is a real error rather than a silent Mojang fallback.
        if let Some(authlib) = &identity.authlib_url {
            jvm.push(self.authlib_agent_args(authlib)?);
        }
        jvm.extend(config.java.jvm_args.iter().cloned());

        // Loader profiles often add module-path flags and inherit `-cp` from
        // the vanilla parent. If a profile replaced the JVM args entirely,
        // the game still needs the assembled classpath.
        if !jvm.iter().any(|arg| arg == "-cp" || arg == "-classpath") {
            jvm.push("-cp".to_string());
            jvm.push(join_paths(&classpath));
        }

        // NeoForge scans the classpath as JPMS modules. The vanilla client jar
        // (`1.21.1.jar`) becomes an automatic module that collides with the
        // slim `minecraft` module the installer produced. Their ignore list
        // names `${version_name}.jar`, which is the loader profile id, not
        // the inherited client jar, so add that filename explicitly.
        let client_jar_name = format!(
            "{}.jar",
            crate::models::version::client_jar_version_id(version)
        );
        for arg in &mut jvm {
            let Some(list) = arg.strip_prefix("-DignoreList=") else {
                continue;
            };
            if list.split(',').any(|item| item == client_jar_name) {
                continue;
            }
            arg.push(',');
            arg.push_str(&client_jar_name);
        }

        // --- main class & game arguments -----------------------------------
        let main_class = version.main_class.clone().ok_or_else(|| {
            AppError::Config(format!(
                "{} does not declare a main class; the version json is incomplete",
                version.id
            ))
        })?;

        let mut game: Vec<String> = match (&version.arguments, &version.minecraft_arguments) {
            (Some(arguments), _) if !arguments.game.is_empty() => {
                Arguments::flatten(&arguments.game, &features)
                    .into_iter()
                    .map(|token| substitute(token, &substitutions))
                    .collect()
            }
            // Pre-1.13: a single flat string with ${...} placeholders.
            (_, Some(legacy)) => legacy
                .split_whitespace()
                .map(|token| substitute(token, &substitutions))
                .collect(),
            _ => Vec::new(),
        };

        // Direct-connect into a mirrored world.
        if let Some(address) = extras.connect {
            game.push("--server".to_string());
            game.push(address.ip().to_string());
            game.push("--port".to_string());
            game.push(address.port().to_string());
        }
        game.extend(config.game_args.iter().cloned());

        Ok(LaunchPlan {
            java: java.clone(),
            main_class,
            classpath,
            jvm_args: jvm,
            game_args: game,
            working_directory: root,
            env: std::env::vars().collect(),
            log_file: self.paths.log_file(&format!("instance-{}", config.id)),
            required_java_major: instance.required_java_major,
        })
    }

    /// Libraries + the client jar, in the order the version json declares them.
    pub fn build_classpath(
        &self,
        version: &VersionJson,
        instance_root: &Path,
    ) -> AppResult<Vec<PathBuf>> {
        let features = FeatureSet::default();
        let mut classpath = Vec::new();
        let mut seen = HashSet::new();

        for library in &version.libraries {
            if !library.is_applicable(&features) {
                continue;
            }
            // Native-only entries have no artifact for the classpath.
            if library.native_classifier().is_some() && library.artifact_path(&features).is_none() {
                continue;
            }
            let Some(relative) = library.artifact_path(&features) else {
                continue;
            };
            let path = self.paths.libraries().join(relative);
            if path.is_file() {
                // Inherited profiles repeat vanilla libraries. NeoForge's
                // bootstrap launcher rejects a classpath that lists one jar twice.
                if seen.insert(path.clone()) {
                    classpath.push(path);
                }
            } else {
                // A missing library means a broken install; fail loudly instead
                // of launching into a NoClassDefFoundError.
                return Err(AppError::Java(format!(
                    "missing library {} — repair the instance install",
                    path.display()
                )));
            }
        }

        let client_id = crate::models::version::client_jar_version_id(version);
        let client_jar = self
            .paths
            .version_dir(client_id)
            .join(format!("{client_id}.jar"));
        if !client_jar.is_file() {
            return Err(AppError::Java(format!(
                "missing client jar for {client_id} — repair the instance install",
            )));
        }
        classpath.push(client_jar);
        let _ = instance_root;
        Ok(classpath)
    }

    /// Explicit path > preferred major > required major > best available.
    fn select_java(&self, instance: &Instance) -> AppResult<PathBuf> {
        if let Some(path) = &instance.config.java.override_path {
            if path.is_file() {
                return Ok(path.clone());
            }
            return Err(AppError::Java(format!(
                "the configured Java path does not exist: {}",
                path.display()
            )));
        }

        let wanted = instance
            .config
            .java
            .preferred_major
            .unwrap_or(instance.required_java_major);

        // Only a runtime that can actually run this version is acceptable: an
        // "almost right" older JDK would start and immediately crash the game.
        if let Some(runtime) = JavaRegistry::select_for(&self.runtimes, wanted) {
            return Ok(runtime.path.clone());
        }

        let installed = if self.runtimes.is_empty() {
            "no JDK was detected on this machine".to_string()
        } else {
            format!(
                "detected: {}",
                self.runtimes
                    .iter()
                    .map(JavaRuntime::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Err(AppError::Java(format!(
            "no Java {wanted} runtime is available. {} ({installed}). \
             Open Settings → Fixes to install the right JDK or pick one by hand.",
            JavaRegistry::compatibility_note(wanted)
        )))
    }

    /// The authlib-injector agent jar used by Ely.by (and any other Yggdrasil
    /// server). The file is provisioned by `instance_launch` before the plan is
    /// built, because a missing agent is exactly what makes Ely.by skins and the
    /// session silently fall back to Mojang.
    pub fn authlib_agent_args(&self, authlib_url: &str) -> AppResult<String> {
        let jar = self.paths.authlib_injector();
        if !jar.is_file() {
            return Err(AppError::Java(format!(
                "the authlib-injector agent is missing at {}; it is downloaded on demand \
                 when launching with an Ely.by or sx.acc account",
                jar.display()
            )));
        }
        Ok(format!("-javaagent:{}={authlib_url}", jar.display()))
    }
}

/// Replace `${name}` placeholders using the substitution table.
pub fn substitute(argument: &str, substitutions: &HashMap<&str, String>) -> String {
    if !argument.contains("${") {
        return argument.to_string();
    }
    let mut result = String::with_capacity(argument.len());
    let mut rest = argument;

    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        match tail.find('}') {
            Some(end) => {
                let key = &tail[..end];
                match substitutions.get(key) {
                    Some(value) => result.push_str(value),
                    // Unknown placeholders are left intact: a wrong value would
                    // be worse than a visible `${...}` in the log.
                    None => {
                        result.push_str("${");
                        result.push_str(key);
                        result.push('}');
                    }
                }
                rest = &tail[end + 1..];
            }
            None => {
                result.push_str("${");
                rest = tail;
            }
        }
    }
    result.push_str(rest);
    result
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&classpath_separator().to_string())
}

/// Resolved child process handle.
#[derive(Debug, Clone)]
pub struct RunningGame {
    pub pid: u32,
    pub instance_id: uuid::Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub local_port: Option<u16>,
}

/// Spawn the JVM, streaming stdout/stderr into the instance log file.
pub async fn spawn(
    plan: &LaunchPlan,
    instance_id: uuid::Uuid,
    local_port: Option<u16>,
) -> AppResult<(RunningGame, tokio::process::Child)> {
    use tokio::io::AsyncReadExt;

    if let Some(parent) = plan.log_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // The command is the only way to tell a bad classpath from a mod crash
    // after the process has already exited 1.
    {
        use tokio::io::AsyncWriteExt;
        let header = format!(
            "\n---- launch {} ----\n{}\n",
            chrono::Utc::now().to_rfc3339(),
            plan.redacted_command_line()
        );
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&plan.log_file)
            .await?;
        file.write_all(header.as_bytes()).await?;
    }

    // Required order: java [jvm args] MainClass [game args].
    let mut command = crate::process::command(&plan.java);
    command
        .args(&plan.jvm_args)
        .arg(&plan.main_class)
        .args(&plan.game_args)
        .current_dir(&plan.working_directory)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|err| AppError::Java(format!("could not start {}: {err}", plan.java.display())))?;

    let pid = child
        .id()
        .ok_or_else(|| AppError::Java("the game process exited immediately".to_string()))?;

    // Tee both streams into a single log file.
    let log_path = plan.log_file.clone();
    if let Some(mut stdout) = child.stdout.take() {
        let path = log_path.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 8192];
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .ok();
            while let Ok(read) = stdout.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                if let Some(file) = file.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = file.write_all(&buffer[..read]).await;
                }
            }
        });
    }
    if let Some(mut stderr) = child.stderr.take() {
        let path = log_path;
        tokio::spawn(async move {
            let mut buffer = [0u8; 8192];
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .ok();
            while let Ok(read) = stderr.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                if let Some(file) = file.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = file.write_all(&buffer[..read]).await;
                }
            }
        });
    }

    let running = RunningGame {
        pid,
        instance_id,
        started_at: chrono::Utc::now(),
        local_port,
    };
    Ok((running, child))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_known_placeholders_and_preserves_unknown_ones() {
        let mut map: HashMap<&str, String> = HashMap::new();
        map.insert("auth_player_name", "Steve".into());
        map.insert("version_name", "1.20.1".into());

        assert_eq!(substitute("${auth_player_name}", &map), "Steve");
        assert_eq!(
            substitute(
                "--username ${auth_player_name} --version ${version_name}",
                &map
            ),
            "--username Steve --version 1.20.1"
        );
        // A typo must stay visible rather than becoming an empty string.
        assert_eq!(substitute("${nope}", &map), "${nope}");
    }

    #[test]
    fn substitution_handles_adjacent_and_unterminated_placeholders() {
        let mut map: HashMap<&str, String> = HashMap::new();
        map.insert("a", "1".into());
        map.insert("b", "2".into());

        assert_eq!(substitute("${a}${b}", &map), "12");
        assert_eq!(substitute("${a", &map), "${a");
        assert_eq!(substitute("no placeholders", &map), "no placeholders");
        assert_eq!(substitute("${a}x${b}", &map), "1x2");
    }

    #[test]
    fn classpath_separator_matches_the_platform() {
        if cfg!(windows) {
            assert_eq!(classpath_separator(), ';');
        } else {
            assert_eq!(classpath_separator(), ':');
        }
    }

    #[test]
    fn redacted_command_line_hides_tokens() {
        let plan = LaunchPlan {
            java: PathBuf::from("/java"),
            main_class: "net.minecraft.client.main.Main".into(),
            classpath: vec![PathBuf::from("/a.jar")],
            jvm_args: vec!["-Xmx2G".into()],
            game_args: vec![
                "--accessToken".into(),
                "super-secret".into(),
                "--username".into(),
                "Steve".into(),
            ],
            working_directory: PathBuf::from("/instance"),
            env: HashMap::new(),
            log_file: PathBuf::from("/log"),
            required_java_major: 17,
        };

        let redacted = plan.redacted_command_line();
        assert!(!redacted.contains("super-secret"));
        assert!(redacted.contains("<redacted>"));
        assert!(redacted.contains("Steve"));
        // Main class sits between JVM and game args.
        let line = plan.command_line().join(" ");
        let main_at = line.find("net.minecraft.client.main.Main").expect("main");
        let token_at = line.find("--accessToken").expect("token flag");
        let xmx_at = line.find("-Xmx2G").expect("xmx");
        assert!(xmx_at < main_at);
        assert!(main_at < token_at);
    }

    #[test]
    fn plan_reports_separator_correctly() {
        assert_eq!(join_paths(&[PathBuf::from("a"), PathBuf::from("b")]), {
            let sep = classpath_separator();
            format!("a{sep}b")
        });
    }

    #[test]
    fn neoforge_ignore_list_includes_the_inherited_client_jar() {
        let root =
            std::env::temp_dir().join(format!("sxm-ignore-{}", uuid::Uuid::new_v4().simple()));
        let paths = AppPaths::from_root(&root);
        paths.ensure().expect("layout");
        let client = paths.version_dir("1.21.1").join("1.21.1.jar");
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, b"jar").unwrap();

        let now = chrono::Utc::now();
        let id = uuid::Uuid::new_v4();
        let mut java = crate::models::instance::JavaSettings::default();
        java.override_path = Some(PathBuf::from("/usr/bin/java"));
        let instance = crate::models::instance::Instance {
            config: crate::models::instance::InstanceConfig {
                id,
                name: "Neo".into(),
                description: String::new(),
                icon: None,
                game_version: "1.21.1".into(),
                loader: crate::models::instance::ModLoader::new(
                    crate::models::instance::LoaderKind::NeoForge,
                    "21.1.251",
                ),
                java,
                memory: crate::models::instance::MemorySettings::default(),
                resolution: crate::models::instance::ResolutionSettings::default(),
                game_args: Vec::new(),
                source_pack: None,
                created_at: now,
                updated_at: now,
            },
            status: crate::models::instance::InstanceStatus::Ready,
            mod_count: 0,
            last_played_at: None,
            total_playtime_secs: 0,
            launch_count: 0,
            size_bytes: 0,
            required_java_major: 21,
        };
        let version = VersionJson {
            id: "neoforge-21.1.251".into(),
            inherits_from: Some("1.21.1".into()),
            main_class: Some("cpw.mods.bootstraplauncher.BootstrapLauncher".into()),
            assets: None,
            asset_index: None,
            libraries: Vec::new(),
            arguments: Some(Arguments {
                jvm: vec![crate::models::version::ArgumentValue::Plain(
                    "-DignoreList=client-extra,${version_name}.jar".into(),
                )],
                game: vec![crate::models::version::ArgumentValue::Plain(
                    "--username".into(),
                )],
            }),
            minecraft_arguments: None,
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: None,
        };

        let plan = LaunchPlanner::new(paths, Vec::new())
            .build(
                &instance,
                &version,
                &LaunchIdentity::offline("Steve", uuid::Uuid::nil()),
                &LaunchExtras::default(),
            )
            .expect("plan");
        let ignore = plan
            .jvm_args
            .iter()
            .find(|arg| arg.starts_with("-DignoreList="))
            .expect("ignore list");
        assert!(
            ignore.split(',').any(|item| item == "1.21.1.jar"),
            "inherited client jar must be ignored by BootstrapLauncher, got {ignore}"
        );
        assert!(
            ignore.contains("neoforge-21.1.251.jar"),
            "profile id stays on the ignore list, got {ignore}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Forge 1.12.2 (and vanilla 1.7–1.12) publish `minecraftArguments` and no
    /// `arguments` object. The JVM still needs a real `-cp` and natives path.
    #[test]
    fn legacy_profiles_substitute_classpath_and_natives() {
        let root = std::env::temp_dir().join(format!(
            "sxm-legacy-launch-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let paths = AppPaths::from_root(&root);
        paths.ensure().expect("layout");
        let client = paths.version_dir("1.12.2").join("1.12.2.jar");
        std::fs::create_dir_all(client.parent().unwrap()).unwrap();
        std::fs::write(&client, b"jar").unwrap();

        let now = chrono::Utc::now();
        let id = uuid::Uuid::new_v4();
        let mut java = crate::models::instance::JavaSettings::default();
        java.override_path = Some(PathBuf::from("/usr/bin/java"));
        let instance = crate::models::instance::Instance {
            config: crate::models::instance::InstanceConfig {
                id,
                name: "RLCraft".into(),
                description: String::new(),
                icon: None,
                game_version: "1.12.2".into(),
                loader: crate::models::instance::ModLoader::new(
                    crate::models::instance::LoaderKind::Forge,
                    "14.23.5.2860",
                ),
                java,
                memory: crate::models::instance::MemorySettings::default(),
                resolution: crate::models::instance::ResolutionSettings::default(),
                game_args: Vec::new(),
                source_pack: None,
                created_at: now,
                updated_at: now,
            },
            status: crate::models::instance::InstanceStatus::Ready,
            mod_count: 0,
            last_played_at: None,
            total_playtime_secs: 0,
            launch_count: 0,
            size_bytes: 0,
            required_java_major: 8,
        };
        // Shape of `version.json` inside the Forge 1.12.2 installer.
        let version = VersionJson {
            id: "1.12.2-forge-14.23.5.2860".into(),
            inherits_from: Some("1.12.2".into()),
            main_class: Some("net.minecraft.launchwrapper.Launch".into()),
            assets: Some("1.12".into()),
            asset_index: None,
            libraries: Vec::new(),
            arguments: None,
            minecraft_arguments: Some(
                "--username ${auth_player_name} --version ${version_name} \
                 --gameDir ${game_directory} --assetsDir ${assets_root} \
                 --assetIndex ${assets_index_name} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker"
                    .into(),
            ),
            downloads: None,
            java_version: None,
            release_time: None,
            release_type: Some("release".into()),
        };

        let plan = LaunchPlanner::new(paths, Vec::new())
            .build(
                &instance,
                &version,
                &LaunchIdentity::offline("Steve", uuid::Uuid::nil()),
                &LaunchExtras::default(),
            )
            .expect("plan");

        let cp_at = plan
            .jvm_args
            .iter()
            .position(|arg| arg == "-cp")
            .expect("-cp");
        let classpath = plan.jvm_args.get(cp_at + 1).expect("classpath argument");
        assert!(
            !classpath.contains("${classpath}"),
            "legacy launch passed the placeholder classpath: {classpath}"
        );
        assert!(
            classpath.contains("1.12.2.jar"),
            "classpath must include the inherited client jar, got {classpath}"
        );
        let library_path = plan
            .jvm_args
            .iter()
            .find(|arg| arg.starts_with("-Djava.library.path="))
            .expect("natives path");
        assert!(
            !library_path.contains("${natives_directory}"),
            "natives path was left as a placeholder: {library_path}"
        );
        assert!(
            library_path.contains("1.12.2"),
            "natives path should follow the vanilla parent, got {library_path}"
        );
        assert!(
            plan.game_args
                .windows(2)
                .any(|pair| { pair[0] == "--username" && pair[1] == "Steve" }),
            "legacy minecraftArguments were not substituted: {:?}",
            plan.game_args
        );
        assert_eq!(plan.main_class, "net.minecraft.launchwrapper.Launch");
        assert!(
            !plan.redacted_command_line().contains("${classpath}"),
            "command line still contains an unsubstituted classpath"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
