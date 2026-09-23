//! Discord Rich Presence commands.
//!
//! Both commands succeed when Discord is not running. The presence thread
//! records the requested activity and connects only when an application id
//! is configured.

use tauri::State;

use crate::error::AppResult;
use crate::presence::{application_id_from_settings, PresenceActivity, PresenceStatus};
use crate::state::AppState;

#[tauri::command]
pub fn discord_presence_set(
    activity: PresenceActivity,
    state: State<'_, AppState>,
) -> AppResult<PresenceStatus> {
    let application_id = application_id_from_settings(&state.settings());
    let enabled = application_id.is_some();
    state.presence().set(application_id, activity);
    Ok(PresenceStatus { enabled })
}

#[tauri::command]
pub fn discord_presence_clear(state: State<'_, AppState>) -> AppResult<()> {
    state.presence().clear();
    Ok(())
}
