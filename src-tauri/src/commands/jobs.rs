//! Job control commands.
//!
//! The backend reports progress on `job://progress`; this is the reverse
//! direction — the UI telling a running job to stop. Without it the ✕ button in
//! the status strip only hid the bar while the download kept going in the
//! background.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cancel::CancelRegistry;
use crate::error::AppResult;

/// What a cancel request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelOutcome {
    /// `true` when the job existed and is now stopping.
    pub cancelled: bool,
}

/// Ask a running job to stop.
///
/// `false` is not an error: the job may have finished a moment before the click
/// landed, and the UI treats that as "already gone".
#[tauri::command]
pub async fn job_cancel(job_id: Uuid) -> AppResult<CancelOutcome> {
    Ok(CancelOutcome {
        cancelled: CancelRegistry::global().cancel(job_id),
    })
}

/// Job ids that can still be cancelled (diagnostics / reconnect after a reload).
#[tauri::command]
pub async fn job_active() -> AppResult<Vec<Uuid>> {
    Ok(CancelRegistry::global().active())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_an_unknown_job_is_reported_not_thrown() {
        let outcome = job_cancel(Uuid::new_v4()).await.expect("no error");
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn cancelling_a_registered_job_flips_its_flag() {
        let id = Uuid::new_v4();
        let flag = CancelRegistry::global().register(id);
        let outcome = job_cancel(id).await.expect("no error");
        assert!(outcome.cancelled);
        assert!(flag.is_cancelled());
        CancelRegistry::global().forget(id);
    }
}
