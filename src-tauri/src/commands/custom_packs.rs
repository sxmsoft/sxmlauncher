//! Custom ("build your own") modpacks.
//!
//! A custom pack is a *named set of mods* the player assembles from the
//! Modrinth/CurseForge browser. It has no `.mrpack` manifest — the pack rows
//! in SQLite are the manifest. Three lifecycle commands:
//!
//! * `custom_pack_create`          — create the pack shell
//! * `custom_pack_add` / `_remove` — pin project (optionally a version) into it
//! * `custom_pack_install`         — resolve the whole set for a game version +
//!   loader, download everything into a fresh instance, record the mods, and
//!   link the instance back to the pack (`source_pack.project_id = pack id`)
//!
//! Pack membership and instance linkage are plain SQLite tables, so sharing a
//! pack is a schema-level operation later (export = read rows + file hashes).

use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::instance::{CreateInstanceRequest, Instance, LoaderKind, ModLoader};
use crate::models::modpack::{ModSource, PackTarget};
use crate::mods::resolver::ModRequest;
use crate::store::instances::{CustomPackItemRow, CustomPackRow};
use crate::state::{sink_for, AppState};

/// Create an empty pack.
#[tauri::command]
pub async fn custom_pack_create(
    name: String,
    description: Option<String>,
    icon_url: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<CustomPackRow> {
    if name.trim().is_empty() {
        return Err(AppError::Config("pack name cannot be empty".into()));
    }
    let pack = CustomPackRow {
        id: Uuid::new_v4(),
        name: name.trim().to_string(),
        description: description.filter(|d| !d.trim().is_empty()),
        icon_url: icon_url.filter(|u| !u.trim().is_empty()),
        game_version: None,
        loader: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    state.db.custom_pack_insert(&pack)?;
    Ok(pack)
}

/// List every pack, newest update first.
#[tauri::command]
pub async fn custom_pack_list(state: State<'_, AppState>) -> AppResult<Vec<CustomPackRow>> {
    state.db.custom_pack_list()
}

/// Delete a pack. Instances created from it are untouched (the reverse link
/// simply dangles, which the UI renders as "custom pack (removed)").
#[tauri::command]
pub async fn custom_pack_delete(id: Uuid, state: State<'_, AppState>) -> AppResult<()> {
    state.db.custom_pack_delete(&id)
}

/// Pin a project into a pack. Pins the latest version compatible with the
/// pack's current game version when none is given.
#[tauri::command]
pub async fn custom_pack_add(
    pack_id: Uuid,
    source: ModSource,
    project_id: String,
    version_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<CustomPackItemRow>> {
    let pack = state
        .db
        .custom_pack_get(&pack_id)?
        .ok_or_else(|| AppError::Config("custom pack not found".into()))?;

    // Empty packs have no target yet: any first mod sets the target and the
    // user can change it from the pack page afterwards.
    let engine = state.mods();
    if let (Some(game_version), Some(loader)) = (&pack.game_version, &pack.loader) {
        let target = PackTarget {
            game_version: game_version.clone(),
            loader: ModLoader {
                kind: loader_from_string(loader),
                version: None,
                build: None,
            },
        };
        let request = ModRequest {
            source,
            project_id: project_id.clone(),
            version_id: version_id.clone(),
            required: true,
        };
        // Resolve with the single request: proves compatibility up front.
        engine.resolve(std::slice::from_ref(&request), &target).await?;
    }

    state
        .db
        .custom_pack_add_item(&pack_id, &source, &project_id, version_id.as_deref())?;
    state.db.custom_pack_touch(&pack_id)?;
    state.db.custom_pack_items(&pack_id) // returned so the UI can refresh its item list
}

/// Remove a project from a pack.
#[tauri::command]
pub async fn custom_pack_remove(
    pack_id: Uuid,
    project_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<CustomPackItemRow>> {
    state.db.custom_pack_remove_item(&pack_id, &project_id)?;
    state.db.custom_pack_touch(&pack_id)?;
    state.db.custom_pack_items(&pack_id)
}

/// List the items of a pack.
#[tauri::command]
pub async fn custom_pack_items(
    pack_id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<Vec<CustomPackItemRow>> {
    state.db.custom_pack_items(&pack_id)
}

/// Set (or clear) the game version + loader a pack resolves against.
#[tauri::command]
pub async fn custom_pack_set_target(
    pack_id: Uuid,
    game_version: String,
    loader: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<CustomPackRow> {
    let pack = state
        .db
        .custom_pack_get(&pack_id)?
        .ok_or_else(|| AppError::Config("custom pack not found".into()))?;
    if game_version.trim().is_empty() {
        return Err(AppError::Config("game version cannot be empty".into()));
    }
    let updated = CustomPackRow {
        game_version: Some(game_version),
        loader: Some(loader.unwrap_or_else(|| "vanilla".to_string())),
        updated_at: chrono::Utc::now(),
        ..pack
    };
    state.db.custom_pack_insert(&updated)?;
    Ok(updated)
}

/// Materialize a pack: fresh instance + every pinned mod downloaded.
#[tauri::command]
pub async fn custom_pack_install(
    pack_id: Uuid,
    name: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Instance> {
    let pack = state
        .db
        .custom_pack_get(&pack_id)?
        .ok_or_else(|| AppError::Config("custom pack not found".into()))?;

    let game_version = pack.game_version.clone().ok_or_else(|| {
        AppError::Config(
            "this pack has no game version yet — open it and pick one before installing".into(),
        )
    })?;
    let loader = pack.loader.clone().unwrap_or_else(|| "vanilla".to_string());

    let items = state.db.custom_pack_items(&pack_id)?;
    if items.is_empty() {
        return Err(AppError::Config(
            "this pack has no mods yet — add mods from the browser first".into(),
        ));
    }

    let engine = state.mods();
    let target = PackTarget {
        game_version: game_version.clone(),
        loader: ModLoader {
            kind: loader_from_string(&loader),
            version: None,
            build: None,
        },
    };

    // 1. Resolve the whole set: dependencies + conflicts, nothing written yet.
    let requests: Vec<ModRequest> = items
        .iter()
        .map(|item| ModRequest {
            source: item.source,
            project_id: item.project_id.clone(),
            version_id: if item.version_id.is_empty() {
                None
            } else {
                Some(item.version_id.clone())
            },
            required: true,
        })
        .collect();
    let plan = engine.resolve(&requests, &target).await?;
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

    // 2. Create the instance shell (configured for the pack's target).
    let loader_kind = loader_from_string(&loader);
    let instance = state
        .instances()
        .create(
            CreateInstanceRequest {
                name: name
                    .unwrap_or_else(|| format!("{} ({game_version})", pack.name)),
                description: Some(pack.description.clone().unwrap_or_else(|| {
                    format!("Custom pack with {} pinned mods", items.len())
                })),
                game_version: game_version.clone(),
                loader: Some(ModLoader {
                    kind: loader_kind,
                    version: None,
                    build: None,
                }),
                memory: None,
                icon: pack.icon_url.clone(),
                install_now: false,
            },
            sink_for(&app),
        )
        .await?;

    // 3. Download everything into the instance.
    let root = state.instances().layout(instance.config.id).root();
    engine
        .install_plan(
            &plan,
            &root,
            format!("Installing pack {}", pack.name),
            sink_for(&app),
        )
        .await?;

    // 4. Record the pack as the instance's source + every file as a mod.
    let mut refreshed = state.instances().get(instance.config.id).await?;
    refreshed.config.source_pack = Some(crate::models::modpack::ModpackRefSource {
        source: ModSource::Modrinth, // custom packs are not from a registry
        project_id: pack.id.to_string(),
        version_id: pack.updated_at.timestamp_millis().to_string(),
        name: pack.name.clone(),
        version_number: format!("custom · {}", items.len()),
        icon_url: pack.icon_url.clone(),
    });
    state
        .instances()
        .update(crate::models::instance::UpdateInstanceRequest {
            id: instance.config.id,
            ..Default::default()
        })
        .await?;

    engine.record_installed(instance.config.id, &plan)?;
    state.instances().refresh(instance.config.id).await?;
    Ok(refreshed)
}

/// `"fabric"` etc. into a [`LoaderKind`], defaulting to vanilla.
fn loader_from_string(loader: &str) -> LoaderKind {
    LoaderKind::from_str_opt(loader).unwrap_or(LoaderKind::Vanilla)
}
