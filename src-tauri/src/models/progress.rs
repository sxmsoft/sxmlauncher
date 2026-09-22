//! Long-running job progress: one event shape for downloads, installs,
//! verification, Java provisioning and P2P connection setup.
//!
//! The backend never formats progress for a specific screen; it publishes
//! normalized [`ProgressEvent`]s on `job://progress` and the UI decides what to
//! show.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which subsystem owns the job (drives the UI icon and wording).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    InstanceInstall,
    ModpackInstall,
    ModDownload,
    JavaRuntime,
    AssetHydration,
    Launch,
    P2pConnect,
    P2pHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    Resolving,
    Downloading,
    Verifying,
    Extracting,
    Linking,
    ProvisioningJava,
    Launching,
    ConnectingP2p,
    Registering,
    Running,
    Done,
    Failed,
}

impl JobStage {
    /// Stages that mean the job will not produce more events.
    pub fn is_terminal(self) -> bool {
        matches!(self, JobStage::Done | JobStage::Failed)
    }
}

/// One progress tick. `total_units == 0` means "unknown total" (indeterminate).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub job_id: Uuid,
    pub kind: JobKind,
    pub stage: JobStage,
    /// Human-readable job title, e.g. "Installing Fabulously Optimized".
    pub label: String,
    /// Units completed (files, bytes or steps depending on the job).
    pub completed_units: u64,
    pub total_units: u64,
    /// Instantaneous throughput, used for the ETA readout.
    pub bytes_per_second: u64,
    /// Item currently in flight, e.g. "sodium-fabric-0.5.8.jar".
    pub current_item: Option<String>,
    /// Secondary line, e.g. "verified 412/620 hashes".
    pub detail: Option<String>,
    pub finished: bool,
    pub error: Option<String>,
    pub started_at_ms: i64,
    /// Instance this job belongs to. The UI uses it to attach progress and
    /// errors to the card that was just created, not whichever card was
    /// selected before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<Uuid>,
}

impl ProgressEvent {
    pub fn started(kind: JobKind, label: impl Into<String>) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            kind,
            stage: JobStage::Queued,
            label: label.into(),
            completed_units: 0,
            total_units: 0,
            bytes_per_second: 0,
            current_item: None,
            detail: None,
            finished: false,
            error: None,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            instance_id: None,
        }
    }

    /// Completed fraction in `0.0..=1.0`; `None` when the total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        if self.total_units == 0 {
            return None;
        }
        Some((self.completed_units as f32 / self.total_units as f32).clamp(0.0, 1.0))
    }

    /// Rough seconds remaining, derived from the current throughput.
    pub fn eta_secs(&self) -> Option<u64> {
        if self.bytes_per_second == 0 || self.total_units <= self.completed_units {
            return None;
        }
        Some((self.total_units - self.completed_units) / self.bytes_per_second)
    }

    pub fn stage(mut self, stage: JobStage) -> Self {
        self.stage = stage;
        self
    }

    pub fn totals(mut self, completed: u64, total: u64) -> Self {
        self.completed_units = completed;
        self.total_units = total;
        self
    }

    pub fn item(mut self, item: impl Into<String>) -> Self {
        self.current_item = Some(item.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn finished(mut self) -> Self {
        self.finished = true;
        self.stage = JobStage::Done;
        self
    }

    pub fn failed(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self.finished = true;
        self.stage = JobStage::Failed;
        self
    }

    pub fn for_job(mut self, job_id: Uuid) -> Self {
        self.job_id = job_id;
        self
    }

    pub fn for_instance(mut self, instance_id: Uuid) -> Self {
        self.instance_id = Some(instance_id);
        self
    }
}

/// Stamps every event with an instance id when the producer did not set one.
pub struct InstanceBoundSink {
    inner: Arc<dyn ProgressSink>,
    instance_id: Uuid,
}

impl InstanceBoundSink {
    pub fn new(inner: Arc<dyn ProgressSink>, instance_id: Uuid) -> Self {
        Self { inner, instance_id }
    }
}

#[async_trait]
impl ProgressSink for InstanceBoundSink {
    async fn report(&self, mut event: ProgressEvent) {
        if event.instance_id.is_none() {
            event.instance_id = Some(self.instance_id);
        }
        self.inner.report(event).await;
    }
}

/// Anything that can consume progress: a Tauri emitter, a test collector or a
/// no-op sink used by unit tests.
#[async_trait]
pub trait ProgressSink: Send + Sync {
    async fn report(&self, event: ProgressEvent);
}

/// Discards every event.
pub struct NoopProgressSink;

#[async_trait]
impl ProgressSink for NoopProgressSink {
    async fn report(&self, _event: ProgressEvent) {}
}

/// Collects events in memory (used by tests and by CLI-style dry runs).
#[derive(Default)]
pub struct CollectingProgressSink {
    events: parking_lot::Mutex<Vec<ProgressEvent>>,
}

impl CollectingProgressSink {
    pub fn snapshot(&self) -> Vec<ProgressEvent> {
        self.events.lock().clone()
    }
}

#[async_trait]
impl ProgressSink for CollectingProgressSink {
    async fn report(&self, event: ProgressEvent) {
        self.events.lock().push(event);
    }
}
