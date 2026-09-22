//! Aggregated job progress.
//!
//! Downloads run dozens of files concurrently. Reporting a progress event per
//! chunk would flood the IPC channel and make the UI stutter, so [`JobTracker`]
//! aggregates counters and rate-limits emission while always letting the final
//! state through.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use uuid::Uuid;

use crate::cancel::{CancelFlag, CancelRegistry};
use crate::error::{AppError, AppResult};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};

/// Minimum gap between two progress emissions for the same job.
const EMIT_INTERVAL: Duration = Duration::from_millis(120);
/// Window used to smooth the throughput estimate.
const RATE_WINDOW: Duration = Duration::from_secs(2);

struct TrackerState {
    stage: JobStage,
    completed_units: u64,
    total_units: u64,
    current_item: Option<String>,
    detail: Option<String>,
    last_emit: Instant,
    window_start: Instant,
    window_units: u64,
    bytes_per_second: u64,
    started_at_ms: i64,
    finished: bool,
}

/// Thread-safe progress aggregator for one logical job.
#[derive(Clone)]
pub struct JobTracker {
    job_id: Uuid,
    kind: JobKind,
    label: String,
    sink: Arc<dyn ProgressSink>,
    state: Arc<Mutex<TrackerState>>,
    /// Flipped by `job_cancel` in the command layer. Long loops poll it.
    cancel: CancelFlag,
}

impl Drop for JobTracker {
    fn drop(&mut self) {
        // The last clone going away means nobody can advance this job any more;
        // release its cancellation slot so the registry cannot grow forever.
        if Arc::strong_count(&self.state) == 1 {
            CancelRegistry::global().forget(self.job_id);
        }
    }
}

impl JobTracker {
    /// Create a tracker and immediately announce the job.
    pub fn start(
        kind: JobKind,
        label: impl Into<String>,
        stage: JobStage,
        sink: Arc<dyn ProgressSink>,
    ) -> Self {
        let label = label.into();
        let job_id = Uuid::new_v4();
        // Registering here means every job the UI can see is also cancellable
        // through `job_cancel`, without the caller having to opt in.
        let cancel = CancelRegistry::global().register(job_id);
        let tracker = Self {
            job_id,
            kind,
            label,
            sink,
            cancel,
            state: Arc::new(Mutex::new(TrackerState {
                stage,
                completed_units: 0,
                total_units: 0,
                current_item: None,
                detail: None,
                last_emit: Instant::now() - EMIT_INTERVAL,
                window_start: Instant::now(),
                window_units: 0,
                bytes_per_second: 0,
                started_at_ms: chrono::Utc::now().timestamp_millis(),
                finished: false,
            })),
        };
        // Announce synchronously so the UI can show the job instantly.
        let event = tracker.snapshot();
        let sink = Arc::clone(&tracker.sink);
        tokio::spawn(async move { sink.report(event).await });
        tracker
    }

    pub fn id(&self) -> Uuid {
        self.job_id
    }

    /// The flag `job_cancel` flips.
    pub fn cancel_flag(&self) -> CancelFlag {
        self.cancel.clone()
    }

    /// `true` once the user asked this job to stop.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Fail with [`AppError::Cancelled`] when the user stopped the job.
    pub fn checkpoint(&self) -> AppResult<()> {
        self.cancel.check()
    }

    /// Total units (bytes or files) the job will process.
    pub fn set_totals(&self, total_units: u64) {
        self.state.lock().total_units = total_units;
    }

    /// Move the job to a new stage (Resolving → Downloading → Verifying …).
    pub async fn set_stage(&self, stage: JobStage) {
        self.state.lock().stage = stage;
        self.emit(true).await;
    }

    /// Record which item is in flight.
    pub async fn set_item(&self, item: Option<String>) {
        self.state.lock().current_item = item;
        self.emit(true).await;
    }

    pub fn set_detail(&self, detail: impl Into<String>) {
        self.state.lock().detail = Some(detail.into());
    }

    /// Add processed units and (maybe) emit.
    pub async fn advance(&self, units: u64) {
        {
            let mut state = self.state.lock();
            state.completed_units = state.completed_units.saturating_add(units);
            state.window_units = state.window_units.saturating_add(units);
            let elapsed = state.window_start.elapsed();
            if elapsed >= RATE_WINDOW {
                state.bytes_per_second = (state.window_units as f64 / elapsed.as_secs_f64()) as u64;
                state.window_start = Instant::now();
                state.window_units = 0;
            }
        }
        self.emit(false).await;
    }

    /// Force an emission (stage changes, item boundaries).
    pub async fn flush(&self) {
        self.emit(true).await;
    }

    /// Mark the job done and emit the final event.
    pub async fn finish(&self) {
        {
            let mut state = self.state.lock();
            state.stage = JobStage::Done;
            state.finished = true;
            state.completed_units = state.total_units.max(state.completed_units);
            state.current_item = None;
        }
        CancelRegistry::global().forget(self.job_id);
        self.emit(true).await;
    }

    /// Mark the job failed, preserving the error message for the UI.
    pub async fn fail(&self, error: impl Into<String>) {
        let error = error.into();
        {
            let mut state = self.state.lock();
            state.stage = JobStage::Failed;
            state.finished = true;
            state.detail = Some(error);
        }
        CancelRegistry::global().forget(self.job_id);
        self.emit(true).await;
    }

    /// Mark the job as stopped by the user.
    ///
    /// This is a *terminal* state and is deliberately distinct from a failure:
    /// the UI shows "cancelled" instead of an error dialog, and nothing is
    /// retried.
    pub async fn cancelled(&self) {
        {
            let mut state = self.state.lock();
            state.stage = JobStage::Failed;
            state.finished = true;
            state.detail = Some(AppError::Cancelled.to_string());
        }
        CancelRegistry::global().forget(self.job_id);
        self.emit(true).await;
    }

    /// Build the current event from the shared state.
    fn snapshot(&self) -> ProgressEvent {
        let state = self.state.lock();
        ProgressEvent {
            job_id: self.job_id,
            kind: self.kind,
            stage: state.stage,
            label: self.label.clone(),
            completed_units: state.completed_units,
            total_units: state.total_units,
            bytes_per_second: state.bytes_per_second,
            current_item: state.current_item.clone(),
            detail: state.detail.clone(),
            finished: state.finished,
            error: if state.stage == JobStage::Failed {
                state.detail.clone()
            } else {
                None
            },
            started_at_ms: state.started_at_ms,
        }
    }

    /// Emit unless rate-limited.
    async fn emit(&self, force: bool) {
        let should_emit = {
            let mut state = self.state.lock();
            if force || state.last_emit.elapsed() >= EMIT_INTERVAL {
                state.last_emit = Instant::now();
                true
            } else {
                false
            }
        };
        if should_emit {
            let event = self.snapshot();
            self.sink.report(event).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::progress::CollectingProgressSink;

    #[tokio::test]
    async fn tracker_reports_start_progress_and_finish() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = JobTracker::start(
            JobKind::ModDownload,
            "Installing mods",
            JobStage::Resolving,
            sink.clone(),
        );
        tracker.set_totals(100);
        tracker.set_stage(JobStage::Downloading).await;
        tracker.advance(40).await;
        tracker.advance(60).await;
        tracker.finish().await;

        let events = sink.snapshot();
        assert!(!events.is_empty());
        let last = events.last().expect("at least one event");
        assert_eq!(last.stage, JobStage::Done);
        assert!(last.finished);
        assert_eq!(last.completed_units, 100);
        assert_eq!(last.total_units, 100);
        assert_eq!(last.fraction(), Some(1.0));
    }

    #[tokio::test]
    async fn tracker_failure_carries_the_message() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = JobTracker::start(
            JobKind::InstanceInstall,
            "Installing",
            JobStage::Downloading,
            sink.clone(),
        );
        tracker.fail("hash mismatch on client.jar").await;

        let events = sink.snapshot();
        let last = events.last().expect("event");
        assert_eq!(last.stage, JobStage::Failed);
        assert!(last.finished);
        assert!(last
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("hash mismatch"));
    }

    #[tokio::test]
    async fn emission_is_rate_limited_until_forced() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = JobTracker::start(
            JobKind::ModDownload,
            "Bulk",
            JobStage::Downloading,
            sink.clone(),
        );
        // The start event is spawned onto the runtime; drain it first.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let baseline = sink.snapshot().len();

        for _ in 0..50 {
            tracker.advance(1).await;
        }
        let after_burst = sink.snapshot().len();
        assert!(
            after_burst - baseline <= 2,
            "expected the burst to be coalesced, got {} events",
            after_burst - baseline
        );

        tracker.flush().await;
        assert!(sink.snapshot().len() > after_burst);
    }

    #[tokio::test]
    async fn unknown_total_reports_no_fraction() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = JobTracker::start(
            JobKind::P2pConnect,
            "Connecting",
            JobStage::ConnectingP2p,
            sink.clone(),
        );
        tracker.advance(10).await;
        let event = tracker.snapshot();
        assert_eq!(event.fraction(), None);
        assert_eq!(event.eta_secs(), None);
    }
}
