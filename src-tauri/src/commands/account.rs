//! Account commands: Microsoft, Ely.by, sx.acc, and offline, plus session maintenance.

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::auth::DeviceCodePrompt;
use crate::error::{AppError, AppResult};
use crate::models::account::{AccountProvider, AccountSummary};
use crate::state::AppState;

/// What the UI needs to drive a browser sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingLoginInfo {
    pub login_id: Uuid,
    pub provider: AccountProvider,
    /// URL to open in the system browser (or render as a button).
    pub authorize_url: String,
    pub redirect_uri: String,
}

#[tauri::command]
pub async fn account_list(state: State<'_, AppState>) -> AppResult<Vec<AccountSummary>> {
    state.accounts().list_accounts().await
}

#[tauri::command]
pub async fn account_active(state: State<'_, AppState>) -> AppResult<Option<AccountSummary>> {
    Ok(state
        .accounts()
        .active_account()
        .await?
        .map(|account| account.summary()))
}

#[tauri::command]
pub async fn account_set_active(
    id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().set_active(id).await
}

#[tauri::command]
pub async fn account_login_offline(
    username: String,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().login_offline(&username).await
}

/// Start a Microsoft or Ely.by browser sign-in.
///
/// The pending login (which owns the loopback listener) is parked in state so
/// the completing command can pick it up after the user finishes in the browser.
#[tauri::command]
pub async fn account_begin_login(
    provider: AccountProvider,
    state: State<'_, AppState>,
) -> AppResult<PendingLoginInfo> {
    let accounts = state.accounts();
    let pending = match provider {
        AccountProvider::Microsoft => accounts.begin_msa_login().await?,
        AccountProvider::ElyBy => accounts.begin_elyby_login().await?,
        AccountProvider::SxAcc => accounts.begin_sxacc_login().await?,
        AccountProvider::Offline => {
            return Err(AppError::Config(
                "offline accounts are created with account_login_offline".to_string(),
            ))
        }
    };

    let info = PendingLoginInfo {
        login_id: pending.login_id,
        provider: pending.provider,
        authorize_url: pending.authorize_url.clone(),
        redirect_uri: pending.redirect_uri.clone(),
    };
    state.pending_logins.insert(info.login_id, pending);
    Ok(info)
}

/// Finish a browser sign-in (waits for the loopback redirect).
#[tauri::command]
pub async fn account_complete_login(
    login_id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    let (_, pending) = state.pending_logins.remove(&login_id).ok_or_else(|| {
        AppError::Account("this sign-in attempt already finished or expired".to_string())
    })?;

    let accounts = state.accounts();
    match pending.provider {
        AccountProvider::Microsoft => accounts.complete_msa_login(pending).await,
        AccountProvider::ElyBy => accounts.complete_elyby_login(pending).await,
        AccountProvider::SxAcc => accounts.complete_sxacc_login(pending).await,
        AccountProvider::Offline => Err(AppError::Config(
            "offline accounts do not use the browser flow".to_string(),
        )),
    }
}

/// Ely.by Authlib sign-in (no browser involved).
#[tauri::command]
pub async fn account_login_elyby_password(
    username: String,
    password: String,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().login_elyby_password(&username, &password).await
}

/// sx.acc username (max 16) and password. Live route: `POST {BASE}/v1/auth/login`.
#[tauri::command]
pub async fn account_login_sxacc_password(
    username: String,
    password: String,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().login_sxacc_password(&username, &password).await
}

/// Create an sx.acc account on the configured server and sign in with it.
#[tauri::command]
pub async fn account_register_sxacc(
    email: String,
    password: String,
    username: String,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state
        .accounts()
        .register_sxacc(&email, &password, &username)
        .await
}

/// Which sx.acc flows the configured base URL currently exposes.
#[tauri::command]
pub async fn account_sxacc_capabilities(
    state: State<'_, AppState>,
) -> AppResult<crate::auth::sxacc::SxAccCapabilities> {
    Ok(state.accounts().sxacc_capabilities().await)
}

/// sx.acc device-code grant (RFC 8628), when the server exposes one.
#[tauri::command]
pub async fn account_begin_sxacc_device(
    state: State<'_, AppState>,
) -> AppResult<crate::auth::sxacc::SxAccDevicePrompt> {
    state.accounts().begin_sxacc_device().await
}

#[tauri::command]
pub async fn account_complete_sxacc_device(
    prompt: crate::auth::sxacc::SxAccDevicePrompt,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().complete_sxacc_device(&prompt).await
}

/// Force a session refresh (Settings → Refresh).
#[tauri::command]
pub async fn account_refresh(id: Uuid, state: State<'_, AppState>) -> AppResult<AccountSummary> {
    state.accounts().refresh_account(id).await
}

/// Apply a user-picked PNG skin to the account (Microsoft: real upload;
/// Ely.by: validated locally + deep link; offline: rejected).
#[tauri::command]
pub async fn account_upload_skin(
    id: Uuid,
    model: crate::models::account::SkinModel,
    png: Vec<u8>,
    state: State<'_, AppState>,
) -> AppResult<crate::auth::skin_upload::SkinUploadOutcome> {
    state.accounts().upload_skin(id, model, png).await
}

#[tauri::command]
pub async fn account_sign_out(id: Uuid, state: State<'_, AppState>) -> AppResult<()> {
    state.accounts().sign_out(id).await
}

/// Abandon a pending browser sign-in and release its loopback port.
#[tauri::command]
pub async fn account_cancel_login(login_id: Uuid, state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.pending_logins.remove(&login_id).is_some())
}

/// Device-code flow for machines where opening a browser is not possible.
#[tauri::command]
pub async fn account_begin_device_code(state: State<'_, AppState>) -> AppResult<DeviceCodePrompt> {
    state.accounts().begin_msa_device_code().await
}

/// Re-read the skin/cape of an account from its provider.
///
/// Fixes the "skins stay default after sign-in" case: the texture URLs live on
/// the provider's servers and change there, so the launcher re-reads them on
/// demand instead of trusting an old snapshot.
#[tauri::command]
pub async fn account_refresh_skin(
    id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<crate::models::account::SkinProfile> {
    state.accounts().refresh_skin(id).await
}

#[tauri::command]
pub async fn account_complete_device_code(
    prompt: DeviceCodePrompt,
    state: State<'_, AppState>,
) -> AppResult<AccountSummary> {
    state.accounts().complete_msa_device_code(&prompt).await
}

#[tauri::command]
pub async fn account_vault_backend(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state.accounts().vault_backend().to_string())
}

/// Build a launch identity for an account without launching (diagnostics).
#[tauri::command]
pub async fn account_launch_identity(
    id: Uuid,
    state: State<'_, AppState>,
) -> AppResult<crate::models::account::LaunchIdentity> {
    state.accounts().launch_identity(id).await
}

#[tauri::command]
pub async fn account_forget_credentials(id: Uuid, state: State<'_, AppState>) -> AppResult<()> {
    state.accounts().forget_credentials(id).await
}
