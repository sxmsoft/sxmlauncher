//! Mod, modpack and Java runtime commands.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::instance::{Instance, LoaderKind, ModLoader};
use crate::models::modpack::{
    ModProject, ModSearchQuery, ModSearchResults, ModSource, ModVersion, PackTarget,
    ResolvedPackPlan,
};
use crate::mods::resolver::ModRequest;
use crate::mods::{JavaRuntime, JavaRegistry, ModEngine};
use crate::state::{sink_for, AppState};

/// Search mods or modpacks.
#[tauri::command]
pub async fn mod_search(
    query: ModSearchQuery,
    state: State<'_, AppState>,
) -> AppResult<ModSearchResults> {
    search(&state.mods(), &query).await
}

async fn search(engine: &ModEngine, query: &ModSearchQuery) -> AppResult<ModSearchResults> {
    match query.source {
        ModSource::Modrinth => engine.modrinth().search(query).await,
        ModSource::CurseForge => engine.curseforge().search(query).await,
    }
}

/// Project details for the detail pane.
#[tauri::command]
pub async fn mod_project(
    id: String,
    source: ModSource,
    state: State<'_, AppState>,
) -> AppResult<ModProject> {
    let engine = state.mods();
    match source {
        ModSource::Modrinth => engine.modrinth().project(&id).await,
        ModSource::CurseForge => {
            let mod_id: u32 = id
                .parse()
                .map_err(|_| AppError::Config(format!("`{id}` is not a CurseForge mod id")))?;
            let project = engine.curseforge().project(mod_id).await?;
            Ok(ModProject {
                id: project.id.to_string(),
                slug: project.slug.clone().unwrap_or_default(),
                title: project.name.clone(),
                description: project.summary.clone().unwrap_or_default(),
                source: ModSource::CurseForge,
                project_type: "mod".to_string(),
                icon_url: project.logo.and_then(|logo| logo.url),
                downloads: project.download_count,
                followers: 0,
                categories: project
                    .categories
                    .iter()
                    .map(|category| category.name.clone())
                    .collect(),
                game_versions: project
                    .latest_files
                    .iter()
                    .flat_map(|file| file.game_versions.clone())
                    .collect(),
                loaders: Vec::new(),
                license: None,
                updated_at: project.date_modified,
                client_side: None,
                server_side: None,
            })
        }
    }
}

/// Versions of a project compatible with an instance's version + loader.
/// All published versions of a project, unfiltered.
///
/// Modpack detail views need the full history — a pack page filters by game
/// version itself, and packs legitimately ship for many game versions. Mods
/// keep using `mod_versions`, which narrows to one game version/loader.
#[tauri::command]
pub async fn mod_all_versions(
    id: String,
    source: ModSource,
    state: State<'_, AppState>,
) -> AppResult<Vec<ModVersion>> {
    let engine = state.mods();
    match source {
        ModSource::Modrinth => engine.modrinth().all_versions(&id).await,
        ModSource::CurseForge => {
            let mod_id: u32 = id
                .parse()
                .map_err(|_| AppError::Config(format!("`{id}` is not a CurseForge mod id")))?;
            let files = engine.curseforge().all_files(&mod_id).await?;
            Ok(files
                .iter()
                .map(|file| engine.curseforge().file_to_version(file))
                .collect())
        }
    }
}

#[tauri::command]
pub async fn mod_versions(
    id: String,
    source: ModSource,
    game_version: String,
    loader: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<ModVersion>> {
    let engine = state.mods();
    match source {
        ModSource::Modrinth => {
            engine
                .modrinth()
                .compatible_versions(&id, &game_version, loader.as_deref())
                .await
        }
        ModSource::CurseForge => {
            let mod_id: u32 = id
                .parse()
                .map_err(|_| AppError::Config(format!("`{id}` is not a CurseForge mod id")))?;
            let files = engine
                .curseforge()
                .compatible_files(mod_id, &game_version, loader.as_deref())
                .await?;
            Ok(files
                .iter()
                .map(|file| engine.curseforge().file_to_version(file))
                .collect())
        }
    }
}

/// Install a CurseForge/Modrinth mod (plus dependencies) into an instance.
#[tauri::command]
pub async fn mod_install(
    instance_id: Uuid,
    requests: Vec<ModRequestRequest>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<ResolvedPackPlan> {
    let engine = state.mods();
    let instance = state.instances().get(instance_id).await?;
    let target = PackTarget {
        game_version: instance.config.game_version.clone(),
        loader: instance.config.loader.clone(),
    };

    let requests: Vec<ModRequest> = requests
        .into_iter()
        .map(|request| ModRequest {
            source: request.source,
            project_id: request.project_id,
            version_id: request.version_id,
            required: request.required.unwrap_or(true),
        })
        .collect();

    let plan = engine.resolve(&requests, &target).await?;
    if !plan.conflicts.is_empty() {
        // Conflicts are reported to the UI before anything is written; the
        // caller decides whether to proceed.
        return Err(AppError::ModResolution(format!(
            "{} of these mods cannot be installed together: {}",
            plan.conflicts.len(),
            plan.conflicts
                .iter()
                .map(|conflict| conflict.reason.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }

    let root = state.instances().layout(instance_id).root();
    engine
        .install_plan(&plan, &root, format!("Installing {} mods", plan.files.len()), sink_for(&app))
        .await?;
    engine.record_installed(instance_id, &plan)?;
    state.instances().refresh(instance_id).await?;
    Ok(plan)
}

/// Request shape for [`mod_install`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRequestRequest {
    pub source: ModSource,
    pub project_id: String,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
}

/// Download a single mod into the instance's `mods/` folder.
///
/// The browser's quick action: one project, its required dependencies,
/// hash-verified. Same resolution machinery as packs, scoped to one request;
/// incompatible results are reported, never silently installed.
#[tauri::command]
pub async fn mod_download(
    instance_id: Uuid,
    request: ModRequestRequest,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<ResolvedPackPlan> {
    let engine = state.mods();
    let instance = state.instances().get(instance_id).await?;
    let target = PackTarget {
        game_version: instance.config.game_version.clone(),
        loader: instance.config.loader.clone(),
    };

    let request = ModRequest {
        source: request.source,
        project_id: request.project_id,
        version_id: request.version_id,
        required: request.required.unwrap_or(true),
    };

    // Resolve the single request into a full plan (dependencies included).
    let plan = match engine.resolve(std::slice::from_ref(&request), &target).await {
        Ok(plan) => plan,
        // Last-resort fallback: pull exactly the requested version file
        // without dependency resolution, so a hiccup in the dependency graph
        // (or an unconfigured CurseForge key) still lets the user play.
        Err(err) => {
            let version = fetch_single_version(&engine, &request).await?;
            if version.download_url.is_empty() {
                return Err(err);
            }
            ResolvedPackPlan {
                target,
                files: vec![crate::models::modpack::ResolvedMod {
                    source: version.source,
                    project_id: version.project_id.clone(),
                    version_id: version.id.clone(),
                    title: version.name.clone(),
                    file_name: version.file_name.clone(),
                    url: version.download_url.clone(),
                    sha1: version.hashes.sha1.clone(),
                    size: version.file_size,
                    destination: String::new(),
                    required: true,
                    reason: crate::models::modpack::ModInclusionReason::Requested,
                }],
                removals: Vec::new(),
                conflicts: Vec::new(),
                total_bytes: version.file_size,
                java_major: 0,
            }
        }
    };

    if plan.files.is_empty() {
        return Err(AppError::ModResolution(
            "no downloadable file was found for this mod and game version".into(),
        ));
    }
    if !plan.conflicts.is_empty() {
        return Err(AppError::ModResolution(format!(
            "{} of these mods cannot be installed together: {}",
            plan.conflicts.len(),
            plan.conflicts
                .iter()
                .map(|conflict| conflict.reason.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }

    // Put single-mod files straight into the instance's mods/ directory.
    let root = state.instances().layout(instance_id).root();
    let mut plan = plan;
    for file in &mut plan.files {
        if file.destination.is_empty() || file.destination.starts_with("overrides") {
            file.destination = if file.file_name.ends_with(".jar") {
                PathBuf::from("mods").join(&file.file_name).to_string_lossy().into_owned()
            } else {
                file.file_name.clone()
            };
        }
    }

    engine
        .install_plan(&plan, &root, format!("Downloading {}", request.project_id), sink_for(&app))
        .await?;
    engine.record_installed(instance_id, &plan)?;
    state.instances().refresh(instance_id).await?;
    Ok(plan)
}

/// Fetch one concrete version (latest compatible when unpinned), regardless
/// of the resolver's mood. Used by [`mod_download`]'s fallback path.
async fn fetch_single_version(
    engine: &crate::mods::ModEngine,
    request: &ModRequest,
) -> AppResult<ModVersion> {
    match request.source {
        ModSource::Modrinth => {
            let version_id = request.version_id.clone().ok_or_else(|| {
                AppError::ModResolution(
                    "pick a version for this mod and retry".into(),
                )
            })?;
            engine.modrinth().version(&version_id).await
        }
        ModSource::CurseForge => {
            let mod_id: u32 = request
                .project_id
                .parse()
                .map_err(|_| AppError::Config(format!("`{}` is not a CurseForge mod id", request.project_id)))?;
            let wanted: u32 = request
                .version_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| {
                    AppError::ModResolution("pick a version for this mod and retry".into())
                })?;
            let files = engine.curseforge().files(mod_id, None).await?;
            let file = files
                .iter()
                .find(|file| file.id == wanted)
                .ok_or_else(|| AppError::ModResolution(format!("version {wanted} of this mod was not found")))?;
            Ok(engine.curseforge().file_to_version(file))
        }
    }
}

/// Install a Modrinth modpack: creates an instance, downloads it, records the pack.
#[tauri::command]
pub async fn modpack_install(
    project_id: String,
    version_id: Option<String>,
    name: Option<String>,
    source: Option<ModSource>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    let source = source.unwrap_or(ModSource::Modrinth);
    if source == ModSource::CurseForge {
        return Err(AppError::Unsupported(
            "CurseForge modpack install is not supported yet — download the pack as an \
             .mrpack from Modrinth, or create a vanilla/Fabric instance and add mods \
             individually. CurseForge single-mod install still works."
                .into(),
        ));
    }

    let engine = state.mods();

    // 1. Resolve the pack version.
    let version = match &version_id {
        Some(version_id) => engine.modrinth().version(version_id).await?,
        None => {
            let project = engine.modrinth().project(&project_id).await?;
            let game_version = project
                .game_versions
                .last()
                .cloned()
                .ok_or_else(|| {
                    AppError::ModResolution("this pack lists no supported game version".into())
                })?;
            engine
                .modrinth()
                .latest_compatible(&project_id, &game_version, None)
                .await?
                .ok_or_else(|| {
                    AppError::ModResolution("no downloadable version of this pack was found".into())
                })?
        }
    };

    // 2. Create the instance shell. Loader/version are refined after the
    //    `.mrpack` is read — we start with a vanilla placeholder so create()
    //    does not try to install a loader we have not resolved yet.
    let game_version_hint = version
        .game_versions
        .first()
        .cloned()
        .unwrap_or_else(|| "1.20.1".to_string());
    let instance = state
        .instances()
        .create(
            crate::models::instance::CreateInstanceRequest {
                name: name.unwrap_or_else(|| version.name.clone()),
                description: Some(format!("Modpack {}", version.version_number)),
                game_version: game_version_hint,
                loader: Some(ModLoader::vanilla()),
                memory: None,
                icon: None,
                install_now: false,
            },
            sink_for(&app),
        )
        .await?;

    let root = state.instances().layout(instance.config.id).root();

    // 3. Download the .mrpack and install its files/overrides.
    let archive = state.paths.downloads.join(format!(
        "{}-{}.mrpack",
        version.project_id, version.id
    ));
    let downloader = engine.downloader();
    let mut task = crate::mods::DownloadTask::new(
        version.file_name.clone(),
        version.download_url.clone(),
        archive.clone(),
    );
    if let Some(sha1) = &version.hashes.sha1 {
        task = task.with_sha1(sha1.clone());
    }
    let tracker = std::sync::Arc::new(crate::jobs::JobTracker::start(
        crate::models::progress::JobKind::ModpackInstall,
        format!("Downloading {}", version.name),
        crate::models::progress::JobStage::Downloading,
        sink_for(&app),
    ));
    downloader.fetch(task, tracker).await?;

    let plan = engine
        .install_mrpack(archive, root, sink_for(&app))
        .await?;

    // 4. Persist the pack metadata + the loader the manifest declared.
    state
        .instances()
        .update(crate::models::instance::UpdateInstanceRequest {
            id: instance.config.id,
            loader: Some(plan.target.loader.clone()),
            game_version: Some(plan.target.game_version.clone()),
            ..Default::default()
        })
        .await?;

    // Stamp the source pack onto the on-disk config (update() does not cover it).
    {
        let mut stamped = state.instances().get(instance.config.id).await?;
        stamped.config.source_pack = Some(crate::models::modpack::ModpackRefSource {
            source: ModSource::Modrinth,
            project_id: project_id.clone(),
            version_id: version.id.clone(),
            name: version.name.clone(),
            version_number: version.version_number.clone(),
            icon_url: None,
        });
        // Re-write directly so source_pack survives.
        let layout = state.instances().layout(stamped.config.id);
        let payload = serde_json::to_string_pretty(&stamped.config)?;
        crate::config::write_atomic(&layout.config_file(), payload.as_bytes())?;
        state.db.upsert_instance(&stamped)?;
    }

    engine.record_installed(instance.config.id, &plan)?;

    // 5. Install vanilla + the loader profile. Without this step the instance
    //    has mods on disk but no launchable version JSON — Play would fail with
    //    "is not installed".
    let mut instance = state.instances().get(instance.config.id).await?;
    instance = state
        .instances()
        .install(instance, sink_for(&app))
        .await?;

    Ok(instance)
}

/// Detected Java runtimes.
#[tauri::command]
pub async fn java_runtimes(state: State<'_, AppState>) -> AppResult<Vec<JavaRuntime>> {
    state.mods().java().detect().await
}

/// Which JDK an instance (or a raw game version) will actually launch with.
///
/// Backs the Fixes panel: it reports the chosen runtime, whether it came from
/// the system or from the launcher, and what is missing when nothing fits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaResolution {
    pub required_major: u8,
    /// `None` when nothing compatible is installed and downloads are off.
    pub selected: Option<JavaRuntime>,
    /// Every runtime detected on the machine.
    pub candidates: Vec<JavaRuntime>,
    /// Human explanation, always filled in.
    pub message: String,
    /// `true` when the selected runtime satisfies `required_major`.
    pub compatible: bool,
}

#[tauri::command]
pub async fn java_resolve(
    game_version: String,
    loader: Option<String>,
    instance_id: Option<Uuid>,
    state: State<'_, AppState>,
) -> AppResult<JavaResolution> {
    let loader_kind = loader
        .as_deref()
        .and_then(LoaderKind::from_str_opt)
        .unwrap_or(LoaderKind::Vanilla);
    let required = crate::mods::required_major_for(&game_version, loader_kind);

    let registry = state.mods().java().clone();
    let candidates = registry.detect().await.unwrap_or_default();
    let override_path = match instance_id {
        Some(id) => state
            .instances()
            .get(id)
            .await
            .ok()
            .and_then(|instance| instance.config.java.override_path),
        None => None,
    };

    // An explicit per-instance path always wins, even if it looks wrong.
    if let Some(path) = override_path {
        match registry.probe_path(&path).await {
            Ok(runtime) => {
                let compatible = runtime.is_compatible_with(required);
                return Ok(JavaResolution {
                    required_major: required,
                    message: if compatible {
                        format!("This instance pins {} — compatible.", runtime.describe())
                    } else {
                        format!(
                            "This instance pins {}, but {} cannot run it. {}",
                            path.display(),
                            runtime.describe(),
                            JavaRegistry::compatibility_note(required)
                        )
                    },
                    compatible,
                    selected: Some(runtime),
                    candidates,
                });
            }
            Err(err) => {
                return Ok(JavaResolution {
                    required_major: required,
                    selected: None,
                    candidates,
                    compatible: false,
                    message: err.to_string(),
                })
            }
        }
    }

    let selected = JavaRegistry::select_for(&candidates, required).cloned();
    let message = match &selected {
        Some(runtime) if runtime.major == required => format!(
            "{} will be used (launcher-detected, exact match).",
            runtime.describe()
        ),
        Some(runtime) => format!(
            "{} will be used instead of downloading a new JDK (compatible, newer).",
            runtime.describe()
        ),
        None if candidates.is_empty() => format!(
            "No JDK was found on this machine. {} Use “Install JDK” below.",
            JavaRegistry::compatibility_note(required)
        ),
        None => format!(
            "None of the {} detected JDKs can run this version. {}",
            candidates.len(),
            JavaRegistry::compatibility_note(required)
        ),
    };

    Ok(JavaResolution {
        required_major: required,
        compatible: selected.is_some(),
        selected,
        candidates,
        message,
    })
}

/// Probe a `java` executable the user picked by hand.
#[tauri::command]
pub async fn java_probe(path: String, state: State<'_, AppState>) -> AppResult<JavaRuntime> {
    state
        .mods()
        .java()
        .probe_path(std::path::Path::new(&path))
        .await
}

/// Download a managed Temurin JDK.
#[tauri::command]
pub async fn java_install(
    major: u8,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<JavaRuntime> {
    state.mods().java().install(major, sink_for(&app).as_ref())
        .await
}

/// Download the JDK a game version needs, resolving the requirement for the user.
#[tauri::command]
pub async fn java_install_for_version(
    game_version: String,
    loader: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<JavaRuntime> {
    let loader_kind = loader
        .as_deref()
        .and_then(LoaderKind::from_str_opt)
        .unwrap_or(LoaderKind::Vanilla);
    let required = crate::mods::required_major_for(&game_version, loader_kind);
    state.mods().java().install(required, sink_for(&app).as_ref())
        .await
}

/// Folder the launcher installs managed JDKs into.
#[tauri::command]
pub async fn java_managed_root(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state
        .mods()
        .java()
        .managed_root()
        .to_string_lossy()
        .into_owned())
}
