//! Cooperative job cancellation.
//!
//! Every long job (install, asset hydration, modpack download, JDK provisioning)
//! registers a flag here under the job id it reports progress with. The UI can
//! then call `job_cancel(<jobId>)` and the running task stops at its next
//! checkpoint instead of continuing to burn bandwidth after the user pressed X.
//!
//! The registry is process-global on purpose: the downloader, the job tracker
//! and the Tauri command layer all need to see the same flag without threading
//! another `Arc` through every constructor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use dashmap::DashMap;
use uuid::Uuid;

/// A single job's cancellation flag.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Mark the job as cancelled. Returns `true` if this call flipped the flag.
    pub fn cancel(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Turn a cancellation into a result, for use inside loops.
    pub fn check(&self) -> crate::error::AppResult<()> {
        if self.is_cancelled() {
            Err(crate::error::AppError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Wait until the flag flips (used by `tokio::select!` loops).
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
    }
}

/// Process-wide job id → flag map.
#[derive(Default)]
pub struct CancelRegistry {
    flags: DashMap<Uuid, CancelFlag>,
}

impl CancelRegistry {
    /// The single registry every subsystem shares.
    pub fn global() -> &'static CancelRegistry {
        static REGISTRY: OnceLock<CancelRegistry> = OnceLock::new();
        REGISTRY.get_or_init(CancelRegistry::default)
    }

    /// Register a job and hand back its flag.
    pub fn register(&self, job_id: Uuid) -> CancelFlag {
        let flag = CancelFlag::new();
        self.flags.insert(job_id, flag.clone());
        flag
    }

    /// Forget a finished job so the map cannot grow without bound.
    pub fn forget(&self, job_id: Uuid) {
        self.flags.remove(&job_id);
    }

    /// Ask a job to stop. `false` means the job already finished/never existed.
    pub fn cancel(&self, job_id: Uuid) -> bool {
        self.flags
            .get(&job_id)
            .map(|flag| flag.cancel())
            .unwrap_or(false)
    }

    pub fn is_cancelled(&self, job_id: Uuid) -> bool {
        self.flags
            .get(&job_id)
            .map(|flag| flag.is_cancelled())
            .unwrap_or(false)
    }

    /// Cancel everything (used when the app shuts down).
    pub fn cancel_all(&self) -> usize {
        let mut count = 0;
        for flag in self.flags.iter() {
            if flag.cancel() {
                count += 1;
            }
        }
        count
    }

    /// Job ids that are still cancellable (in-flight).
    pub fn active(&self) -> Vec<Uuid> {
        self.flags.iter().map(|entry| *entry.key()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flips_once_and_stays_flipped() {
        let registry = CancelRegistry::default();
        let id = Uuid::new_v4();
        let flag = registry.register(id);
        assert!(!flag.is_cancelled());
        assert!(registry.cancel(id));
        assert!(flag.is_cancelled());
        // A second cancel is a no-op, so the UI cannot report a double stop.
        assert!(!registry.cancel(id));
    }

    #[test]
    fn unknown_jobs_cancel_to_false() {
        let registry = CancelRegistry::default();
        assert!(!registry.cancel(Uuid::new_v4()));
        assert!(!registry.is_cancelled(Uuid::new_v4()));
    }

    #[test]
    fn forget_releases_the_flag() {
        let registry = CancelRegistry::default();
        let id = Uuid::new_v4();
        registry.register(id);
        assert_eq!(registry.active().len(), 1);
        registry.forget(id);
        assert!(registry.active().is_empty());
    }

    #[test]
    fn cancelled_flags_fail_the_checkpoint() {
        let flag = CancelFlag::new();
        assert!(flag.check().is_ok());
        flag.cancel();
        assert_eq!(
            flag.check().expect_err("must fail").code(),
            crate::error::CODE_CANCELLED
        );
    }
}
