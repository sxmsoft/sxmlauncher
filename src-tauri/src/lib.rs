//! SXMLauncher backend.
//!
//! Module map:
//!
//! | module      | responsibility                                               |
//! |-------------|--------------------------------------------------------------|
//! | `auth`      | Microsoft / Ely.by / sx.acc / offline accounts, token vault  |
//! | `instances` | isolated game environments: install, launch, import          |
//! | `mods`      | Modrinth + CurseForge, `.mrpack`, downloads, Java runtimes   |
//! | `network`   | Redis directory, NAT traversal, P2P tunnel, session control  |
//! | `store`     | SQLite: settings, instance index, caches, history            |

pub mod auth;
pub mod cancel;
pub mod commands;
pub mod config;
pub mod error;
pub mod instances;
pub mod jobs;
pub mod models;
pub mod mods;
pub mod network;
pub mod presence;
pub mod process;
pub mod state;
pub mod store;

use tauri::Manager;

use crate::error::AppResult;
use crate::state::AppState;

/// Build and run the desktop application.
pub fn run() {
    tauri::Builder::default()
        // Must be registered first. On Windows and Linux the Microsoft
        // `ms-xal-` redirect starts a second process; this hands the URL to
        // the instance that is waiting for it.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            for arg in argv {
                crate::auth::deliver_oauth_callback(&arg);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        // Plugins the UI calls directly (file pickers, opening folders, ...).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        // Self-update: the UI checks the release feed (endpoints + signing key
        // live in `tauri.conf.json`) and `process` performs the restart that
        // swaps in the downloaded update.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // State initialization is async (vault probe, Redis connect); block
            // the setup hook so commands can never run against a half-built app.
            let state = tauri::async_runtime::block_on(AppState::initialize(&handle))?;
            app.manage(state);

            // Register `ms-xal-00000000402b5328` (Microsoft) and `sxmlauncher`
            // (sx.acc OAuth) so the system browser can return sign-in to this
            // process. A cold start opened by one of those URLs is handled
            // from argv below.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().on_open_url(|event| {
                    for url in event.urls() {
                        crate::auth::deliver_oauth_callback(url.as_str());
                    }
                });
                // The plugin records a cold-start URL before this hook runs, so
                // the listener above misses it. Pull that URL out explicitly.
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    for url in urls {
                        crate::auth::deliver_oauth_callback(url.as_str());
                    }
                }
                // macOS registers the scheme in the bundle, and `register_all`
                // returns unsupported there. Windows and Linux need the runtime
                // registration so a dev build can receive `ms-xal-` too.
                #[cfg(not(target_os = "macos"))]
                if let Err(err) = app.deep_link().register_all() {
                    eprintln!(
                        "[auth] could not register the Microsoft sign-in callback ({err}). \
                         Device-code sign-in still works."
                    );
                }
            }
            for arg in std::env::args().skip(1) {
                crate::auth::deliver_oauth_callback(&arg);
            }

            // Load the window only once state exists, so the first command the
            // UI issues always finds it.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // --- system ---------------------------------------------------
            commands::system::app_info,
            commands::system::app_paths,
            commands::system::settings_get,
            commands::system::settings_update,
            commands::system::settings_test_redis,
            commands::system::cache_clear,
            commands::system::cache_stats,
            commands::system::log_tail,
            commands::presence::discord_presence_set,
            commands::presence::discord_presence_clear,
            // --- updater --------------------------------------------------
            commands::system::updater_check,
            commands::system::updater_download,
            commands::system::updater_install,
            // --- accounts -------------------------------------------------
            commands::account::account_list,
            commands::account::account_active,
            commands::account::account_set_active,
            commands::account::account_login_offline,
            commands::account::account_begin_login,
            commands::account::account_complete_login,
            commands::account::account_login_elyby_password,
            commands::account::account_login_sxacc_password,
            commands::account::account_register_sxacc,
            commands::account::account_sxacc_capabilities,
            commands::account::account_begin_sxacc_device,
            commands::account::account_complete_sxacc_device,
            commands::account::account_refresh,
            commands::account::account_upload_skin,
            commands::account::account_sign_out,
            commands::account::account_cancel_login,
            commands::account::account_begin_device_code,
            commands::account::account_complete_device_code,
            commands::account::account_refresh_skin,
            commands::account::account_vault_backend,
            commands::account::account_launch_identity,
            commands::account::account_forget_credentials,
            // --- instances ------------------------------------------------
            commands::instance::instance_list,
            commands::instance::instance_get,
            commands::instance::instance_create,
            commands::instance::instance_update,
            commands::instance::instance_delete,
            commands::instance::instance_duplicate,
            commands::instance::instance_install,
            commands::instance::instance_refresh,
            commands::instance::instance_mods,
            commands::instance::instance_toggle_mod,
            commands::instance::instance_launch,
            commands::instance::instance_kill,
            commands::instance::instance_running,
            commands::instance::instance_import,
            commands::instance::version_list,
            // --- mods -----------------------------------------------------
            commands::mods::mod_search,
            commands::mods::mod_project,
            commands::mods::mod_versions,
            commands::mods::mod_all_versions,
            commands::mods::modpack_install,
            commands::mods::mod_install,
            commands::mods::mod_download,
            commands::mods::java_runtimes,
            commands::mods::java_install,
            commands::mods::java_resolve,
            commands::mods::java_probe,
            commands::mods::java_install_for_version,
            commands::mods::java_managed_root,
            // --- custom modpacks ------------------------------------------
            commands::custom_packs::custom_pack_create,
            commands::custom_packs::custom_pack_list,
            commands::custom_packs::custom_pack_delete,
            commands::custom_packs::custom_pack_add,
            commands::custom_packs::custom_pack_remove,
            commands::custom_packs::custom_pack_items,
            commands::custom_packs::custom_pack_set_target,
            commands::custom_packs::custom_pack_install,
            // --- jobs -----------------------------------------------------
            commands::jobs::job_cancel,
            commands::jobs::job_active,
            // --- network --------------------------------------------------
            commands::network::server_browse,
            commands::network::server_ping,
            commands::network::server_favorites,
            commands::network::server_set_favorite,
            commands::network::network_status,
            // --- LAN (no server required) -----------------------------------
            commands::network::lan_browse,
            commands::network::lan_worlds,
            commands::network::lan_host_start,
            commands::network::lan_host_stop,
            commands::network::lan_host_for_instance,
            commands::network::lan_address,
            commands::network::host_start,
            commands::network::host_stop,
            commands::network::host_status,
            commands::network::host_kick,
            commands::network::join_code,
            commands::network::join_server,
            commands::network::leave_session,
            commands::network::connection_code,
            commands::network::session_history,
            commands::network::nat_probe,
            commands::network::local_rtt,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Drop Discord presence before the process goes away. Shutdown
                // below also clears it; doing it here covers a fast exit.
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    state.presence().shutdown();
                }
                // Stop hosted worlds and game processes before the app exits so
                // no orphaned java.exe or stale listing is left behind.
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>() {
                        state.shutdown().await;
                    }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running SXMLauncher");
}

/// Initialize the app without spawning the event loop (integration tests).
pub fn init_for_tests() -> AppResult<()> {
    Ok(())
}
