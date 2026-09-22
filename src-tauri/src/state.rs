//! Shared application state.
//!
//! Every manager hangs off [`AppState`], which is registered with Tauri and
//! injected into commands. Managers that depend on user settings (download
//! concurrency, CurseForge key, Redis URL) are held behind `RwLock<Arc<..>>` and
//! **rebuilt** when settings change, so no manager ever caches a stale config.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Child;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::auth::{AccountManager, PendingLogins};
use crate::config::{AppPaths, AppSettings};
use crate::error::{AppError, AppResult};
use crate::instances::{InstanceManager, LaunchPlan};
use crate::models::progress::{ProgressEvent, ProgressSink};
use crate::models::version::{merge_profiles, VersionJson};
use crate::mods::ModEngine;
use crate::network::{
    localnet::LanManager, holepunch::PunchConfig, DirectTransport, RelayTransport, SessionManager,
    TransportRegistry,
};
use crate::store::Database;

/// Tauri event name for job progress.
pub const PROGRESS_EVENT: &str = "job://progress";
/// Tauri event name for session lifecycle events.
pub const SESSION_EVENT: &str = "session://event";
/// Tauri event name for game process state.
pub const GAME_EVENT: &str = "game://state";

/// Emits progress events to the frontend.
#[derive(Clone)]
pub struct TauriProgressSink {
    app: AppHandle,
}

impl TauriProgressSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ProgressSink for TauriProgressSink {
    async fn report(&self, event: ProgressEvent) {
        // A closed window must not break a running download.
        let _ = self.app.emit(PROGRESS_EVENT, &event);
    }
}

/// A launched game process.
pub struct RunningGameHandle {
    pub game: crate::instances::RunningGame,
    pub plan: LaunchPlan,
    pub child: Arc<AsyncMutex<Child>>,
    /// Set when the launch also opened a P2P session (host or guest).
    pub session_id: Option<Uuid>,
}

/// Application-wide state.
pub struct AppState {
    pub paths: AppPaths,
    pub db: Database,
    settings: RwLock<AppSettings>,
    accounts: RwLock<Arc<AccountManager>>,
    instances: RwLock<Arc<InstanceManager>>,
    mods: RwLock<Arc<ModEngine>>,
    /// `None` while disconnected/offline — the server browser then reports a
    /// clear "directory unreachable" error instead of failing silently.
    network: RwLock<Option<Arc<SessionManager>>>,
    /// LAN discovery. Always present: it needs neither a directory nor a server,
    /// so hosting/clients on the same network work out of the box.
    lan: Arc<LanManager>,
    pub pending_logins: PendingLogins,
    pub running: DashMap<Uuid, Arc<RunningGameHandle>>,
    /// Dedicated servers started by the integrated host path (keyed by session id).
    pub hosted_servers: DashMap<Uuid, Arc<crate::instances::HostedServer>>,
}

impl AppState {
    /// Build every manager from the resolved paths and persisted settings.
    pub async fn initialize(app: &AppHandle) -> AppResult<Self> {
        let paths = AppPaths::resolve(app)?;
        paths.ensure()?;

        let db = Database::open(&paths.database_file)?;
        let settings = AppSettings::load(&paths)?;
        // Keep the SQLite copy authoritative when the JSON mirror was edited.
        db.save_app_settings(&settings)?;

        let vault = crate::auth::select_vault().await;
        let accounts = Arc::new(AccountManager::new_with_config(
            db.clone(),
            vault,
            crate::auth::ProviderConfig::from(&settings),
        ));
        accounts.restore_active()?;

        let mods = Arc::new(ModEngine::new(paths.clone(), db.clone(), &settings)?);
        let instances = Arc::new(InstanceManager::new(
            paths.clone(),
            db.clone(),
            mods.clone(),
            settings.clone(),
        ));

        let state = Self {
            paths,
            db,
            settings: RwLock::new(settings.clone()),
            accounts: RwLock::new(accounts),
            instances: RwLock::new(instances),
            mods: RwLock::new(mods),
            network: RwLock::new(None),
            lan: Arc::new(LanManager::new(settings.lan_port)),
            pending_logins: DashMap::new(),
            running: DashMap::new(),
            hosted_servers: DashMap::new(),
        };

        // Connect the directory in the background: a missing Redis must never
        // block app startup, and the launcher is fully usable without one (LAN
        // hosting needs no infrastructure at all).
        if settings.directory_enabled {
            if let Err(err) = state.connect_directory(&settings).await {
                eprintln!("[state] directory not connected (LAN mode stays available): {err}");
            }
        }

        Ok(state)
    }

    /// (Re)connect the global directory with the current settings.
    ///
    /// The embedded MQTT directory is the default: it needs no server of ours
    /// (retained messages on a public broker carry the listings), which is how
    /// the launcher is online out of the box. A non-empty `mqtt_broker` picks
    /// it; an empty one falls back to the legacy Redis URL for self-hosters.
    pub async fn connect_directory(&self, settings: &AppSettings) -> AppResult<Arc<SessionManager>> {
        let directory: Arc<dyn crate::network::Directory> =
            if !settings.mqtt_broker.trim().is_empty() {
                Arc::new(
                    crate::network::mqtt::MqttDirectory::connect(
                        settings.mqtt_broker.trim(),
                        settings.mqtt_port,
                    )
                    .await?,
                )
            } else {
                Arc::new(crate::network::RedisDirectory::connect(&settings.redis_url).await?)
            };
        let punch = PunchConfig::from_servers(&settings.stun_servers);
        let transports = Arc::new(TransportRegistry::new(
            Arc::new(DirectTransport::new("0.0.0.0:0".parse().map_err(
                |err| AppError::Config(format!("invalid bind address: {err}")),
            )?)),
            Arc::new(RelayTransport::new(settings.relay_url.clone())),
        ));

        let manager = Arc::new(SessionManager::new(
            directory,
            transports,
            self.db.clone(),
            punch,
            // TODO(security): generate + persist an Ed25519 keypair and sign
            // signaling envelopes. Until then the field is informational.
            String::new(),
        ));
        *self.network.write() = Some(manager.clone());
        Ok(manager)
    }

    /// Session manager for hosting/joining. Falls back to an in-process
    /// directory when Redis/MQTT is unavailable so join codes still work.
    pub async fn network_for_hosting(&self) -> AppResult<Arc<SessionManager>> {
        if let Some(manager) = self.network_optional() {
            return Ok(manager);
        }
        let settings = self.settings();
        let directory: Arc<dyn crate::network::Directory> =
            Arc::new(crate::network::MemoryDirectory::new());
        let punch = PunchConfig::from_servers(&settings.stun_servers);
        let transports = Arc::new(TransportRegistry::new(
            Arc::new(DirectTransport::new("0.0.0.0:0".parse().map_err(|err| {
                AppError::Config(format!("invalid bind address: {err}"))
            })?)),
            Arc::new(RelayTransport::new(settings.relay_url.clone())),
        ));
        let manager = Arc::new(SessionManager::new(
            directory,
            transports,
            self.db.clone(),
            punch,
            String::new(),
        ));
        *self.network.write() = Some(manager.clone());
        Ok(manager)
    }

    /// The session manager, or a clear error when the directory is unreachable.
    ///
    /// LAN hosting and LAN joins never call this, so a launcher without a
    /// directory still gets a working server list and P2P on the local network.
    pub fn network(&self) -> AppResult<Arc<SessionManager>> {
        self.network.read().clone().ok_or_else(|| {
            AppError::Directory(
                "the global server directory is not connected. Enable it in \
                 Settings → Network, or use Host on an instance — join codes \
                 work without Redis."
                    .to_string(),
            )
        })
    }

    pub fn network_optional(&self) -> Option<Arc<SessionManager>> {
        self.network.read().clone()
    }

    /// LAN discovery/hosting (always available).
    pub fn lan(&self) -> Arc<LanManager> {
        self.lan.clone()
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.read().clone()
    }

    pub fn accounts(&self) -> Arc<AccountManager> {
        self.accounts.read().clone()
    }

    pub fn instances(&self) -> Arc<InstanceManager> {
        self.instances.read().clone()
    }

    pub fn mods(&self) -> Arc<ModEngine> {
        self.mods.read().clone()
    }

    /// Persist settings and rebuild the managers that depend on them.
    pub async fn apply_settings(&self, settings: AppSettings) -> AppResult<AppSettings> {
        let sanitized = settings.sanitized();
        self.db.save_app_settings(&sanitized)?;
        sanitized.save(&self.paths)?;

        let vault = crate::auth::select_vault().await;
        *self.accounts.write() = Arc::new(AccountManager::new_with_config(
            self.db.clone(),
            vault,
            crate::auth::ProviderConfig::from(&sanitized),
        ));

        let mods = Arc::new(ModEngine::new(
            self.paths.clone(),
            self.db.clone(),
            &sanitized,
        )?);
        *self.mods.write() = mods.clone();

        // `InstanceManager` holds an Arc<ModEngine> and a settings snapshot, so
        // it is rebuilt rather than mutated: no manager can ever observe a
        // half-updated configuration.
        *self.instances.write() = Arc::new(InstanceManager::new(
            self.paths.clone(),
            self.db.clone(),
            mods,
            sanitized.clone(),
        ));

        // Reconnect the directory when the endpoint or STUN list changed.
        let previous = self.settings.read().clone();
        if previous.redis_url != sanitized.redis_url
            || previous.stun_servers != sanitized.stun_servers
            || previous.relay_url != sanitized.relay_url
        {
            if sanitized.directory_enabled {
                if let Err(err) = self.connect_directory(&sanitized).await {
                    eprintln!("[state] directory reconnect failed: {err}");
                }
            } else {
                *self.network.write() = None;
            }
        }
        if !sanitized.directory_enabled {
            *self.network.write() = None;
        }
        // The LAN beacon socket is bound to a fixed port, so a changed port means
        // dropping the current socket instead of silently keeping the old one.
        if previous.lan_port != sanitized.lan_port {
            self.lan.rebind(sanitized.lan_port).await;
        }

        *self.settings.write() = sanitized.clone();
        Ok(sanitized)
    }

    /// Resolve a version json from disk, merging `inheritsFrom` profiles.
    ///
    /// Reading locally means launching works offline and reflects exactly what
    /// the installer verified, rather than whatever Mojang serves today.
    pub fn resolve_local_version(&self, version_id: &str) -> AppResult<VersionJson> {
        let child = self.read_version_file(version_id)?;
        match child.inherits_from.clone() {
            None => Ok(child),
            Some(parent_id) => {
                let parent = self.read_version_file(&parent_id)?;
                Ok(merge_profiles(&parent, &child))
            }
        }
    }

    fn read_version_file(&self, version_id: &str) -> AppResult<VersionJson> {
        let path = self.paths.version_json(version_id);
        let raw = std::fs::read_to_string(&path).map_err(|err| {
            AppError::Config(format!(
                "Minecraft {version_id} is not installed ({} is missing): {err}",
                path.display()
            ))
        })?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Running games keyed by instance id.
    pub fn running_games(&self) -> HashMap<Uuid, Arc<RunningGameHandle>> {
        self.running
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }

    /// Stop every game process (app exit).
    pub async fn shutdown(&self) {
        self.lan.shutdown();
        if let Some(network) = self.network_optional() {
            network.shutdown().await;
        }
        for entry in self.running.iter() {
            let mut child = entry.value().child.lock().await;
            let _ = child.start_kill();
        }
        self.running.clear();
        for entry in self.hosted_servers.iter() {
            let mut child = entry.value().child.lock().await;
            let _ = child.start_kill();
        }
        self.hosted_servers.clear();
    }
}

/// Convenience: fetch the state from a command context.
pub fn state_of(app: &AppHandle) -> Result<tauri::State<'_, AppState>, AppError> {
    app.try_state::<AppState>()
        .ok_or_else(|| AppError::Other("application state is not initialized".to_string()))
}

/// Progress sink for a command invocation.
pub fn sink_for(app: &AppHandle) -> Arc<dyn ProgressSink> {
    Arc::new(TauriProgressSink::new(app.clone()))
}

/// Mutable slot used to park a value between two IPC calls.
pub type Slot<T> = Mutex<Option<T>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::progress::CollectingProgressSink;

    #[test]
    fn slug_helpers_are_stable() {
        // Guards the event names the frontend subscribes to.
        assert_eq!(PROGRESS_EVENT, "job://progress");
        assert_eq!(SESSION_EVENT, "session://event");
        assert_eq!(GAME_EVENT, "game://state");
    }

    #[tokio::test]
    async fn collecting_sink_records_events_in_order() {
        let sink = CollectingProgressSink::default();
        let event = ProgressEvent::started(crate::models::progress::JobKind::ModDownload, "test");
        sink.report(event.clone()).await;
        assert_eq!(sink.snapshot().len(), 1);
        assert_eq!(sink.snapshot()[0].label, "test");
    }
}
