//! Instance index + installed-mod bookkeeping.
//!
//! The authoritative config lives in `<instance>/instance.json` so an instance
//! folder stays portable (copy the folder, keep the instance). This table is a
//! fast index for list/sort plus the counters the dashboard shows.

use chrono::Utc;
use rusqlite::{params, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::instance::{Instance, InstanceStatus, LoaderKind, ModLoader};
use crate::models::modpack::ModSource;
use crate::store::{bool_to_int, datetime_to_millis, int_to_bool, millis_to_datetime, opt_millis_to_datetime, Database};

/// One row of `installed_mods`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModRow {
    pub instance_id: Uuid,
    pub source: ModSource,
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub file_name: String,
    pub sha1: Option<String>,
    pub enabled: bool,
    pub installed_at: chrono::DateTime<Utc>,
}

/// One pinned project inside a custom pack.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPackItemRow {
    pub pack_id: Uuid,
    pub source: ModSource,
    pub project_id: String,
    /// Empty string = resolve latest compatible at install time.
    pub version_id: String,
    pub added_at: chrono::DateTime<Utc>,
}

/// A custom (player-assembled) modpack.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPackRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    /// Target the pack resolves against; `None` until the first mod pins it.
    pub game_version: Option<String>,
    /// Loader name (`vanilla`/`fabric`/`forge`/…), `None` until pinned.
    pub loader: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl Database {
    /// All instances, most recently updated first.
    pub fn list_instances(&self) -> AppResult<Vec<Instance>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, config_json, status, last_played_at, total_playtime_secs,
                        launch_count, mod_count, size_bytes
                 FROM instances ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_instance)?;
            let mut instances = Vec::new();
            for row in rows {
                instances.push(row?);
            }
            Ok(instances)
        })
    }

    pub fn get_instance(&self, id: Uuid) -> AppResult<Option<Instance>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, config_json, status, last_played_at, total_playtime_secs,
                        launch_count, mod_count, size_bytes
                 FROM instances WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![id.to_string()], row_to_instance)?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
    }

    /// Insert or replace an instance (config JSON is the source of truth).
    pub fn upsert_instance(&self, instance: &Instance) -> AppResult<()> {
        let config = &instance.config;
        let config_json = serde_json::to_string(config)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO instances (
                    id, name, description, icon, game_version, loader, loader_version,
                    config_json, status, last_played_at, total_playtime_secs, launch_count,
                    mod_count, size_bytes, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT (id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    icon = excluded.icon,
                    game_version = excluded.game_version,
                    loader = excluded.loader,
                    loader_version = excluded.loader_version,
                    config_json = excluded.config_json,
                    status = excluded.status,
                    mod_count = excluded.mod_count,
                    size_bytes = excluded.size_bytes,
                    updated_at = excluded.updated_at",
                params![
                    config.id.to_string(),
                    config.name,
                    config.description,
                    config.icon,
                    config.game_version,
                    config.loader.kind.as_str(),
                    config.loader.version,
                    config_json,
                    instance.status.as_str(),
                    instance.last_played_at.map(datetime_to_millis),
                    instance.total_playtime_secs as i64,
                    instance.launch_count,
                    instance.mod_count,
                    instance.size_bytes as i64,
                    datetime_to_millis(config.created_at),
                    datetime_to_millis(config.updated_at),
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_instance(&self, id: Uuid) -> AppResult<()> {
        self.transaction(|tx| {
            let key = id.to_string();
            tx.execute("DELETE FROM installed_mods WHERE instance_id = ?1", params![key])?;
            tx.execute("DELETE FROM instances WHERE id = ?1", params![key])?;
            Ok(())
        })
    }

    pub fn set_instance_status(&self, id: Uuid, status: InstanceStatus) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE instances SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![
                    id.to_string(),
                    status.as_str(),
                    datetime_to_millis(Utc::now())
                ],
            )?;
            Ok(())
        })
    }

    /// Bump launch counters when a session starts.
    pub fn record_launch(&self, id: Uuid) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE instances
                 SET launch_count = launch_count + 1,
                     last_played_at = ?2,
                     status = ?3
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    datetime_to_millis(Utc::now()),
                    InstanceStatus::Running.as_str()
                ],
            )?;
            Ok(())
        })
    }

    /// Add elapsed play time when the game process exits.
    pub fn record_playtime(&self, id: Uuid, seconds: u64) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE instances
                 SET total_playtime_secs = total_playtime_secs + ?2,
                     status = ?3
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    seconds as i64,
                    InstanceStatus::Ready.as_str()
                ],
            )?;
            Ok(())
        })
    }

    /// Refresh derived stats (called after installs and mod changes).
    pub fn update_instance_stats(
        &self,
        id: Uuid,
        mod_count: u32,
        size_bytes: u64,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE instances SET mod_count = ?2, size_bytes = ?3 WHERE id = ?1",
                params![id.to_string(), mod_count, size_bytes as i64],
            )?;
            Ok(())
        })
    }

    /// Replace the loader/version of an instance (used by "update pack").
    pub fn update_instance_loader(
        &self,
        id: Uuid,
        game_version: &str,
        loader: &ModLoader,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE instances
                 SET game_version = ?2, loader = ?3, loader_version = ?4,
                     updated_at = ?5
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    game_version,
                    loader.kind.as_str(),
                    loader.version,
                    datetime_to_millis(Utc::now())
                ],
            )?;
            Ok(())
        })
    }

    // --- installed mods --------------------------------------------------

    pub fn list_installed_mods(&self, instance_id: Uuid) -> AppResult<Vec<InstalledModRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT instance_id, source, project_id, version_id, title, file_name,
                        sha1, enabled, installed_at
                 FROM installed_mods WHERE instance_id = ?1 ORDER BY title",
            )?;
            let rows = stmt.query_map(params![instance_id.to_string()], |row| {
                Ok(InstalledModRow {
                    instance_id: parse_uuid(&row.get::<_, String>(0)?)?,
                    source: ModSource::from_str_opt(&row.get::<_, String>(1)?).unwrap_or(ModSource::Modrinth),
                    project_id: row.get(2)?,
                    version_id: row.get(3)?,
                    title: row.get(4)?,
                    file_name: row.get(5)?,
                    sha1: row.get(6)?,
                    enabled: int_to_bool(row.get(7)?),
                    installed_at: millis_to_datetime(row.get(8)?),
                })
            })?;
            let mut mods = Vec::new();
            for row in rows {
                mods.push(row?);
            }
            Ok(mods)
        })
    }

    /// Record (or replace) an installed mod for an instance.
    pub fn upsert_installed_mod(&self, row: &InstalledModRow) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO installed_mods (
                    instance_id, source, project_id, version_id, title, file_name,
                    sha1, enabled, installed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (instance_id, project_id) DO UPDATE SET
                    source = excluded.source,
                    version_id = excluded.version_id,
                    title = excluded.title,
                    file_name = excluded.file_name,
                    sha1 = excluded.sha1,
                    enabled = excluded.enabled,
                    installed_at = excluded.installed_at",
                params![
                    row.instance_id.to_string(),
                    row.source.as_str(),
                    row.project_id,
                    row.version_id,
                    row.title,
                    row.file_name,
                    row.sha1,
                    bool_to_int(row.enabled),
                    datetime_to_millis(row.installed_at),
                ],
            )?;
            Ok(())
        })
    }

    pub fn remove_installed_mod(&self, instance_id: Uuid, project_id: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM installed_mods WHERE instance_id = ?1 AND project_id = ?2",
                params![instance_id.to_string(), project_id],
            )?;
            Ok(())
        })
    }

    pub fn set_mod_enabled(
        &self,
        instance_id: Uuid,
        project_id: &str,
        enabled: bool,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE installed_mods SET enabled = ?3 WHERE instance_id = ?1 AND project_id = ?2",
                params![instance_id.to_string(), project_id, bool_to_int(enabled)],
            )?;
            Ok(())
        })
    }

    // -- custom modpacks -------------------------------------------------

    /// Insert or replace a custom pack.
    pub fn custom_pack_insert(&self, pack: &CustomPackRow) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO custom_packs (
                    id, name, description, icon_url, game_version, loader, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    icon_url = excluded.icon_url,
                    game_version = excluded.game_version,
                    loader = excluded.loader,
                    updated_at = excluded.updated_at",
                params![
                    pack.id.to_string(),
                    pack.name,
                    pack.description,
                    pack.icon_url,
                    pack.game_version,
                    pack.loader,
                    datetime_to_millis(pack.created_at),
                    datetime_to_millis(pack.updated_at),
                ],
            )?;
            Ok(())
        })
    }

    /// All packs, most recently updated first.
    pub fn custom_pack_list(&self) -> AppResult<Vec<CustomPackRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, description, icon_url, game_version, loader, created_at, updated_at
                 FROM custom_packs ORDER BY updated_at DESC",
            )?;
            let rows = stmt
                .query_map([], custom_pack_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn custom_pack_get(&self, id: &Uuid) -> AppResult<Option<CustomPackRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, description, icon_url, game_version, loader, created_at, updated_at
                 FROM custom_packs WHERE id = ?1",
            )?;
            let mut rows = stmt
                .query_map(params![id.to_string()], custom_pack_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows.pop())
        })
    }

    pub fn custom_pack_delete(&self, id: &Uuid) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM custom_packs WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(())
        })
    }

    /// Bump `updated_at` so pack lists re-sort by actual activity.
    pub fn custom_pack_touch(&self, id: &Uuid) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE custom_packs SET updated_at = ?2 WHERE id = ?1",
                params![id.to_string(), datetime_to_millis(Utc::now())],
            )?;
            Ok(())
        })
    }

    pub fn custom_pack_add_item(
        &self,
        pack_id: &Uuid,
        source: &ModSource,
        project_id: &str,
        version_id: Option<&str>,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO custom_pack_items (pack_id, source, project_id, version_id, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (pack_id, project_id) DO UPDATE SET
                    source = excluded.source,
                    version_id = excluded.version_id",
                params![
                    pack_id.to_string(),
                    source.as_str(),
                    project_id,
                    version_id.unwrap_or(""),
                    datetime_to_millis(Utc::now()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn custom_pack_remove_item(&self, pack_id: &Uuid, project_id: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM custom_pack_items WHERE pack_id = ?1 AND project_id = ?2",
                params![pack_id.to_string(), project_id],
            )?;
            Ok(())
        })
    }

    pub fn custom_pack_items(&self, pack_id: &Uuid) -> AppResult<Vec<CustomPackItemRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT pack_id, source, project_id, version_id, added_at
                 FROM custom_pack_items WHERE pack_id = ?1 ORDER BY added_at",
            )?;
            let rows = stmt
                .query_map(params![pack_id.to_string()], |row| {
                    Ok(CustomPackItemRow {
                        pack_id: parse_uuid(&row.get::<_, String>(0)?)?,
                        source: ModSource::from_str_opt(&row.get::<_, String>(1)?).unwrap_or(ModSource::Modrinth),
                        project_id: row.get(2)?,
                        version_id: row.get(3)?,
                        added_at: millis_to_datetime(row.get(4)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

fn row_to_instance(row: &Row<'_>) -> rusqlite::Result<Instance> {
    let id: String = row.get(0)?;
    let config_json: String = row.get(1)?;
    let status: String = row.get(2)?;
    let last_played_at: Option<i64> = row.get(3)?;
    let total_playtime_secs: i64 = row.get(4)?;
    let launch_count: i64 = row.get(5)?;
    let mod_count: i64 = row.get(6)?;
    let size_bytes: i64 = row.get(7)?;

    let config: crate::models::instance::InstanceConfig = serde_json::from_str(&config_json)
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(AppError::Database(format!("corrupt instance config: {err}"))),
            )
        })?;

    let _ = id;
    Ok(Instance {
        required_java_major: crate::mods::java_runtime::required_java_major(&config.game_version),
        config,
        status: match status.as_str() {
            "installing" => InstanceStatus::Installing,
            "ready" => InstanceStatus::Ready,
            "running" => InstanceStatus::Running,
            "corrupted" => InstanceStatus::Corrupted,
            "update_available" => InstanceStatus::UpdateAvailable,
            _ => InstanceStatus::NotInstalled,
        },
        mod_count: mod_count.max(0) as u32,
        last_played_at: opt_millis_to_datetime(last_played_at),
        total_playtime_secs: total_playtime_secs.max(0) as u64,
        launch_count: launch_count.max(0) as u32,
        size_bytes: size_bytes.max(0) as u64,
    })
}

/// Reader for the 8-column `custom_packs` select.
fn custom_pack_from_row(row: &Row<'_>) -> rusqlite::Result<CustomPackRow> {
    Ok(CustomPackRow {
        id: parse_uuid(&row.get::<_, String>(0)?)?,
        name: row.get(1)?,
        description: row.get(2)?,
        icon_url: row.get(3)?,
        game_version: row.get(4)?,
        loader: row.get(5)?,
        created_at: millis_to_datetime(row.get(6)?),
        updated_at: millis_to_datetime(row.get(7)?),
    })
}

fn parse_uuid(raw: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(AppError::Database(format!("invalid uuid `{raw}`: {err}"))),
        )
    })
}

/// Loader reconstruction helper used by tests and migrations.
pub fn loader_from_columns(kind: &str, version: Option<String>) -> ModLoader {
    let kind = LoaderKind::from_str_opt(kind).unwrap_or(LoaderKind::Vanilla);
    ModLoader {
        kind,
        version,
        build: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::instance::{InstanceConfig, JavaSettings, MemorySettings, ResolutionSettings};

    fn instance(name: &str) -> Instance {
        let now = Utc::now();
        Instance {
            config: InstanceConfig {
                id: Uuid::new_v4(),
                name: name.into(),
                description: String::new(),
                icon: Some("🧱".into()),
                game_version: "1.20.1".into(),
                loader: ModLoader::new(LoaderKind::Fabric, "0.15.11"),
                java: JavaSettings::default(),
                memory: MemorySettings::default(),
                resolution: ResolutionSettings::default(),
                game_args: vec![],
                source_pack: None,
                created_at: now,
                updated_at: now,
            },
            status: InstanceStatus::NotInstalled,
            mod_count: 0,
            last_played_at: None,
            total_playtime_secs: 0,
            launch_count: 0,
            size_bytes: 0,
            required_java_major: 17,
        }
    }

    #[test]
    fn instance_round_trips_through_sqlite() {
        let db = Database::open_in_memory().expect("db");
        let instance = instance("Fabulously Optimized");
        db.upsert_instance(&instance).expect("insert");

        let loaded = db
            .get_instance(instance.config.id)
            .expect("query")
            .expect("present");
        assert_eq!(loaded.config.name, "Fabulously Optimized");
        assert_eq!(loaded.config.loader.kind, LoaderKind::Fabric);
        assert_eq!(loaded.required_java_major, 17);
    }

    #[test]
    fn playtime_and_launch_counters_accumulate() {
        let db = Database::open_in_memory().expect("db");
        let instance = instance("Vanilla");
        db.upsert_instance(&instance).expect("insert");
        let id = instance.config.id;

        db.record_launch(id).expect("launch");
        db.record_playtime(id, 3600).expect("playtime");
        db.record_playtime(id, 600).expect("playtime");

        let loaded = db.get_instance(id).expect("query").expect("present");
        assert_eq!(loaded.launch_count, 1);
        assert_eq!(loaded.total_playtime_secs, 4200);
        assert_eq!(loaded.status, InstanceStatus::Ready);
    }

    #[test]
    fn deleting_an_instance_cascades_to_mods() {
        let db = Database::open_in_memory().expect("db");
        let instance = instance("Modded");
        db.upsert_instance(&instance).expect("insert");
        db.upsert_installed_mod(&InstalledModRow {
            instance_id: instance.config.id,
            source: ModSource::Modrinth,
            project_id: "AANobbMI".into(),
            version_id: "abc123".into(),
            title: "Sodium".into(),
            file_name: "sodium.jar".into(),
            sha1: None,
            enabled: true,
            installed_at: Utc::now(),
        })
        .expect("mod insert");

        assert_eq!(db.list_installed_mods(instance.config.id).expect("list").len(), 1);
        db.delete_instance(instance.config.id).expect("delete");
        assert!(db.list_installed_mods(instance.config.id).expect("list").is_empty());
    }
}
