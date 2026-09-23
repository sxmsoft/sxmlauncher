//! Parallel, hash-verified downloads.
//!
//! Guarantees the rest of the pipeline relies on:
//! * **Nothing lands unless it verifies.** Every file is written to a `.part`
//!   sibling and only renamed into place after its SHA-1 matches.
//! * **One copy on disk per hash.** Verified files are stored content-addressed
//!   in the download cache, so two instances sharing Sodium share one download.
//! * **Bounded concurrency.** A semaphore caps parallel requests; retries use
//!   exponential backoff with jitter so a flaky CDN is not hammered in lockstep.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rand::Rng;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::jobs::JobTracker;
use crate::models::progress::{JobKind, JobStage, ProgressSink};

/// Files smaller than this are buffered in memory instead of streamed.
const IN_MEMORY_THRESHOLD: u64 = 512 * 1024;

/// One file to fetch.
#[derive(Debug, Clone)]
pub struct DownloadTask {
    /// Shown in the UI, e.g. `sodium-fabric-0.5.8.jar`.
    pub label: String,
    pub url: String,
    /// Final location (inside an instance `mods/` folder or `shared/`).
    pub destination: PathBuf,
    pub expected_sha1: Option<String>,
    pub expected_sha256: Option<String>,
    pub expected_size: Option<u64>,
    /// Consult/populate the content-addressed cache (skip for tiny files).
    pub use_cache: bool,
}

impl DownloadTask {
    pub fn new(label: impl Into<String>, url: impl Into<String>, destination: PathBuf) -> Self {
        Self {
            label: label.into(),
            url: url.into(),
            destination,
            expected_sha1: None,
            expected_sha256: None,
            expected_size: None,
            use_cache: true,
        }
    }

    pub fn with_sha1(mut self, sha1: impl Into<String>) -> Self {
        self.expected_sha1 = Some(sha1.into().to_lowercase());
        self
    }

    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.expected_sha256 = Some(sha256.into().to_lowercase());
        self
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.expected_size = Some(size);
        self
    }

    pub fn without_cache(mut self) -> Self {
        self.use_cache = false;
        self
    }
}

/// Result of one successful fetch.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub label: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// Served from the local cache or already present at the destination.
    pub from_cache: bool,
    /// SHA-1 actually observed, when the file was hashed.
    pub sha1: Option<String>,
}

/// The download engine.
#[derive(Clone)]
pub struct Downloader {
    http: reqwest::Client,
    cache_dir: PathBuf,
    permits: Arc<Semaphore>,
    concurrency: usize,
    attempts: u32,
}

impl Downloader {
    pub fn new(http: reqwest::Client, cache_dir: PathBuf, max_concurrent: u32) -> Self {
        let concurrency = max_concurrent.clamp(1, 64) as usize;
        Self {
            http,
            cache_dir,
            permits: Arc::new(Semaphore::new(concurrency)),
            concurrency,
            attempts: 3,
        }
    }

    /// Parallel request budget.
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// Override the retry budget (used by tests).
    pub fn with_attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// Download one file, verifying its hash.
    ///
    /// Byte counters are advanced exactly once per received chunk (inside
    /// [`Self::attempt`]) so aggregate progress can never double count.
    pub async fn fetch(
        &self,
        task: DownloadTask,
        tracker: Arc<JobTracker>,
    ) -> AppResult<DownloadOutcome> {
        // 1. Already installed and correct? Nothing to do.
        if task.destination.is_file() {
            if let Some(expected) = &task.expected_sha256 {
                if self.verify_file_sha256(&task.destination, expected).await? {
                    let bytes = file_size(&task.destination);
                    tracker.advance(bytes).await;
                    return Ok(DownloadOutcome {
                        label: task.label,
                        path: task.destination,
                        bytes,
                        from_cache: true,
                        sha1: None,
                    });
                }
            } else if let Some(expected) = &task.expected_sha1 {
                if self.verify_file(&task.destination, expected).await? {
                    let bytes = file_size(&task.destination);
                    tracker.advance(bytes).await;
                    return Ok(DownloadOutcome {
                        label: task.label,
                        path: task.destination,
                        bytes,
                        from_cache: true,
                        sha1: Some(expected.clone()),
                    });
                }
            } else if task
                .expected_size
                .is_none_or(|size| file_size(&task.destination) == size)
            {
                let bytes = file_size(&task.destination);
                tracker.advance(bytes).await;
                return Ok(DownloadOutcome {
                    label: task.label,
                    path: task.destination,
                    bytes,
                    from_cache: true,
                    sha1: None,
                });
            }
        }

        // 2. Content-addressed cache hit?
        if task.use_cache {
            if let Some(expected) = &task.expected_sha1 {
                let cached = self.cache_path(expected);
                if cached.is_file() {
                    tracker.set_item(Some(task.label.clone())).await;
                    link_or_copy(&cached, &task.destination).await?;
                    let bytes = file_size(&cached);
                    tracker.advance(bytes).await;
                    return Ok(DownloadOutcome {
                        label: task.label,
                        path: task.destination,
                        bytes,
                        from_cache: true,
                        sha1: Some(expected.clone()),
                    });
                }
            }
        }

        tracker.set_item(Some(task.label.clone())).await;

        // 3. Download with retries.
        if tracker.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let mut last_error: Option<AppError> = None;
        for attempt in 0..self.attempts {
            tracker.checkpoint()?;
            let _permit = self
                .permits
                .acquire()
                .await
                .map_err(|_| AppError::Other("download semaphore closed".to_string()))?;
            tracker.checkpoint()?;

            match self.attempt(&task, tracker.clone()).await {
                Ok(bytes) => {
                    let sha1 = match &task.expected_sha1 {
                        Some(expected) => Some(expected.clone()),
                        None => tokio::task::spawn_blocking({
                            let path = task.destination.clone();
                            move || sha1_file(&path).ok()
                        })
                        .await
                        .unwrap_or(None),
                    };
                    let _ = tracker.set_stage(JobStage::Downloading).await;
                    return Ok(DownloadOutcome {
                        label: task.label,
                        path: task.destination,
                        bytes,
                        from_cache: false,
                        sha1,
                    });
                }
                Err(err) if err.is_retryable() && attempt + 1 < self.attempts => {
                    // Exponential backoff with jitter: 500ms, 1s, 2s (+/- 250ms).
                    let base = 500u64 * (1 << attempt);
                    let jitter = rand::thread_rng().gen_range(0..=250);
                    tokio::time::sleep(Duration::from_millis(base + jitter)).await;
                    tracker.set_detail(format!(
                        "retrying {} (attempt {}/{})",
                        task.label,
                        attempt + 2,
                        self.attempts
                    ));
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }

        Err(last_error
            .unwrap_or_else(|| AppError::Network(format!("gave up downloading {}", task.label))))
    }

    /// Fetch a batch concurrently, returning every outcome in input order.
    pub async fn fetch_all(
        &self,
        kind: JobKind,
        label: impl Into<String>,
        tasks: Vec<DownloadTask>,
        sink: Arc<dyn ProgressSink>,
    ) -> AppResult<Vec<DownloadOutcome>> {
        let total_bytes: u64 = tasks
            .iter()
            .filter_map(|task| task.expected_size)
            .sum::<u64>()
            .max(1);

        let mut seen_destinations = std::collections::HashSet::new();
        let tasks: Vec<DownloadTask> = tasks
            .into_iter()
            .filter(|task| seen_destinations.insert(task.destination.clone()))
            .collect();

        let tracker = Arc::new(JobTracker::start(
            kind,
            label,
            JobStage::Downloading,
            sink.clone(),
        ));
        tracker.set_totals(total_bytes);

        let mut stream = futures::stream::iter(tasks.into_iter().map(|task| {
            let downloader = self.clone();
            let tracker = tracker.clone();
            async move { downloader.fetch(task, tracker).await }
        }))
        .buffer_unordered(self.concurrency);

        let mut outcomes = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(outcome) => outcomes.push(outcome),
                Err(AppError::Cancelled) => {
                    // Stopped by the user: report it as such instead of as a
                    // failure, so the UI does not pop an error dialog.
                    tracker.cancelled().await;
                    return Err(AppError::Cancelled);
                }
                Err(err) => {
                    tracker.fail(err.to_string()).await;
                    return Err(err);
                }
            }
        }

        tracker.finish().await;
        Ok(outcomes)
    }

    /// Mark a one-file job finished or failed.
    ///
    /// [`Self::fetch`] does not own the tracker: batch downloads share one job
    /// and settle it in [`Self::fetch_all`]. A caller that opened a tracker for
    /// a single file (authlib-injector, a server jar) must call this, or the
    /// Activity row stays on Downloading after the file is already on disk.
    pub async fn settle(
        tracker: &JobTracker,
        result: AppResult<DownloadOutcome>,
    ) -> AppResult<DownloadOutcome> {
        match result {
            Ok(outcome) => {
                tracker.finish().await;
                Ok(outcome)
            }
            Err(AppError::Cancelled) => {
                tracker.cancelled().await;
                Err(AppError::Cancelled)
            }
            Err(err) => {
                let message = err.to_string();
                tracker.fail(message).await;
                Err(err)
            }
        }
    }

    /// One download attempt: stream to `.part`, verify, then rename.
    async fn attempt(&self, task: &DownloadTask, tracker: Arc<JobTracker>) -> AppResult<u64> {
        if let Some(parent) = task.destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                AppError::Io(std::io::Error::other(format!(
                    "creating {}: {err}",
                    parent.display()
                )))
            })?;
        }
        let partial = partial_path(&task.destination);
        let _ = tokio::fs::remove_file(&partial).await;

        let response = self
            .http
            .get(&task.url)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("{}: {err}", task.label)))?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "{}: HTTP {} from {}",
                task.label,
                response.status(),
                task.url
            )));
        }

        let declared_size = response.content_length().or(task.expected_size);
        let mut written: u64 = 0;
        // Report the total for this file too, so a batch of unknown sizes still
        // shows a meaningful bar once the first response headers arrive.
        let mut hasher = Sha1::new();
        let mut sha256_hasher = task.expected_sha256.as_ref().map(|_| Sha256::new());

        // Parallel installs create the same parent at once. A second create can
        // observe ENOENT until that parent is visible, so retry once.
        let mut file = match tokio::fs::File::create(&partial).await {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = partial.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|err| {
                        AppError::Io(std::io::Error::other(format!(
                            "creating {}: {err}",
                            parent.display()
                        )))
                    })?;
                }
                tokio::fs::File::create(&partial).await.map_err(|err| {
                    AppError::Io(std::io::Error::other(format!(
                        "creating {}: {err}",
                        partial.display()
                    )))
                })?
            }
            Err(err) => {
                return Err(AppError::Io(std::io::Error::other(format!(
                    "creating {}: {err}",
                    partial.display()
                ))))
            }
        };
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            // Cancellation must be visible within one chunk (~64 KiB), otherwise
            // a large client jar keeps downloading after the user pressed X.
            if tracker.is_cancelled() {
                drop(file);
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(AppError::Cancelled);
            }
            let chunk = chunk.map_err(|err| {
                AppError::Network(format!("{}: stream interrupted: {err}", task.label))
            })?;
            hasher.update(&chunk);
            if let Some(hasher256) = sha256_hasher.as_mut() {
                hasher256.update(&chunk);
            }
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;

            // Progress is reported against the *whole* batch, so the tracker
            // receives byte deltas as they arrive.
            tracker.advance(chunk.len() as u64).await;
        }
        file.flush().await?;
        drop(file);

        let observed = hex::encode(hasher.finalize());

        // Verify before the file is visible under its real name.
        if let Some(expected) = &task.expected_sha1 {
            if !observed.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(AppError::HashMismatch {
                    file: task.label.clone(),
                    expected: expected.clone(),
                    actual: observed,
                });
            }
        }
        if let (Some(expected), Some(hasher256)) = (&task.expected_sha256, sha256_hasher) {
            let observed256 = hex::encode(hasher256.finalize());
            if !observed256.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(AppError::HashMismatch {
                    file: task.label.clone(),
                    expected: expected.clone(),
                    actual: observed256,
                });
            }
        }
        if let Some(expected_size) = task.expected_size {
            if written != expected_size {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(AppError::Network(format!(
                    "{}: expected {expected_size} bytes but received {written}",
                    task.label
                )));
            }
        }

        tokio::fs::rename(&partial, &task.destination)
            .await
            .map_err(|err| {
                AppError::Io(std::io::Error::other(format!(
                    "renaming {} to {}: {err}",
                    partial.display(),
                    task.destination.display()
                )))
            })?;

        // Populate the content cache for the next instance that needs this file.
        if task.use_cache && written >= IN_MEMORY_THRESHOLD {
            let cache_path = self.cache_path(&observed);
            if !cache_path.is_file() {
                if let Some(parent) = cache_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                // A failure here is not fatal: the file is already installed.
                let _ = tokio::fs::copy(&task.destination, &cache_path).await;
            }
        }

        let _ = declared_size;
        Ok(written)
    }

    /// `true` when the file exists and its SHA-1 matches.
    pub async fn verify_file(&self, path: &Path, expected_sha1: &str) -> AppResult<bool> {
        let path = path.to_path_buf();
        let expected = expected_sha1.to_lowercase();
        let observed = tokio::task::spawn_blocking(move || sha1_file(&path)).await??;
        Ok(observed.eq_ignore_ascii_case(&expected))
    }

    /// `true` when the file exists and its SHA-256 matches.
    pub async fn verify_file_sha256(&self, path: &Path, expected_sha256: &str) -> AppResult<bool> {
        let path = path.to_path_buf();
        let expected = expected_sha256.to_lowercase();
        let observed = tokio::task::spawn_blocking(move || sha256_file(&path)).await??;
        Ok(observed.eq_ignore_ascii_case(&expected))
    }

    /// Content-addressed cache location for a hash.
    /// Root of the content-addressed cache (shown in Settings).
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Cache location for a SHA-1: `<cache>/<ab>/<rest-of-digest>`.
    ///
    /// Two details matter here. The digest is lowercased first, because
    /// Modrinth, CurseForge and a hand-copied mods folder all hand us the same
    /// hash with different casing — without normalizing, one file would be
    /// cached twice and never hit. And the shard is taken from the digest
    /// rather than written as a second copy, so a single directory cannot grow
    /// to hundreds of thousands of entries.
    pub fn cache_path(&self, sha1: &str) -> PathBuf {
        let digest = sha1.trim().to_ascii_lowercase();
        // Non-hex input (or a digest too short to shard) never reaches the
        // slice below, which would otherwise panic on a byte boundary.
        if digest.len() < 3 || !digest.is_ascii() {
            return self.cache_dir.join(digest);
        }
        self.cache_dir.join(&digest[..2]).join(&digest[2..])
    }

    /// Total size of the download cache in bytes.
    pub async fn cache_size(&self) -> u64 {
        let dir = self.cache_dir.clone();
        tokio::task::spawn_blocking(move || directory_size(&dir))
            .await
            .unwrap_or(0)
    }

    /// Delete every cached file (Settings → "Clear download cache").
    pub async fn clear_cache(&self) -> AppResult<()> {
        let dir = self.cache_dir.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            if dir.exists() {
                for entry in std::fs::read_dir(&dir)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        std::fs::remove_dir_all(entry.path())?;
                    } else {
                        std::fs::remove_file(entry.path())?;
                    }
                }
            }
            Ok(())
        })
        .await?
    }
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    // Unique per attempt so two workers fetching one object cannot delete
    // each other's in-progress file out from under the rename.
    name.push('.');
    name.push_str(&uuid::Uuid::new_v4().simple().to_string());
    name.push_str(".part");
    destination.with_file_name(name)
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// Total bytes under `dir` (recursive, best effort).
pub fn directory_size(dir: &Path) -> u64 {
    if !dir.exists() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// SHA-1 of a file, computed in a single streaming pass.
pub fn sha1_file(path: &Path) -> AppResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// SHA-1 of a byte slice.
pub fn sha1_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// SHA-256 of a file (authlib-injector metadata, etc.).
pub fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// Hard-link when possible (same volume, instant), else copy.
async fn link_or_copy(from: &Path, to: &Path) -> AppResult<()> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            AppError::Io(std::io::Error::other(format!(
                "creating {}: {err}",
                parent.display()
            )))
        })?;
    }
    let _ = tokio::fs::remove_file(to).await;
    if tokio::fs::hard_link(from, to).await.is_ok() {
        return Ok(());
    }
    tokio::fs::copy(from, to).await.map_err(|err| {
        AppError::Io(std::io::Error::other(format!(
            "copying {} to {}: {err}",
            from.display(),
            to.display()
        )))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::progress::CollectingProgressSink;

    fn downloader(dir: &Path) -> Downloader {
        Downloader::new(reqwest::Client::new(), dir.join("cache"), 4).with_attempts(1)
    }

    #[tokio::test]
    async fn skips_download_when_the_destination_already_verifies() {
        let temp = tempdir();
        let target = temp.path().join("mods").join("sodium.jar");
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target, b"already here").expect("write");

        let expected = sha1_bytes(b"already here");
        let tracker = Arc::new(JobTracker::start(
            JobKind::ModDownload,
            "test",
            JobStage::Downloading,
            Arc::new(CollectingProgressSink::default()),
        ));

        let outcome = downloader(temp.path())
            .fetch(
                DownloadTask::new("sodium.jar", "http://127.0.0.1:1/none", target.clone())
                    .with_sha1(expected),
                tracker,
            )
            .await
            .expect("cached hit");

        assert!(outcome.from_cache);
        assert_eq!(outcome.bytes, 12);
    }

    #[tokio::test]
    async fn refuses_to_use_a_corrupt_destination() {
        let temp = tempdir();
        let target = temp.path().join("mods").join("broken.jar");
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target, b"corrupt").expect("write");

        let tracker = Arc::new(JobTracker::start(
            JobKind::ModDownload,
            "test",
            JobStage::Downloading,
            Arc::new(CollectingProgressSink::default()),
        ));

        // The hash does not match, so it must attempt a real download and fail
        // on the (unreachable) URL rather than silently accepting the file.
        let error = downloader(temp.path())
            .fetch(
                DownloadTask::new("broken.jar", "http://127.0.0.1:1/none", target)
                    .with_sha1("0".repeat(40)),
                tracker,
            )
            .await
            .expect_err("must not accept a corrupt file");
        assert_eq!(error.code(), crate::error::CODE_NETWORK);
    }

    #[tokio::test]
    async fn settling_a_single_file_marks_the_job_done() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = Arc::new(JobTracker::start(
            JobKind::Launch,
            "authlib-injector (Ely.by agent)",
            JobStage::Downloading,
            sink.clone(),
        ));
        let outcome = DownloadOutcome {
            label: "authlib-injector".into(),
            path: PathBuf::from("authlib-injector.jar"),
            bytes: 4,
            from_cache: false,
            sha1: None,
        };
        Downloader::settle(&tracker, Ok(outcome))
            .await
            .expect("settled");
        let last = sink.snapshot().last().cloned().expect("event");
        assert_eq!(last.stage, JobStage::Done);
        assert!(last.finished);
        assert!(last.error.is_none());
    }

    #[tokio::test]
    async fn settling_a_failure_does_not_leave_the_job_downloading() {
        let sink = Arc::new(CollectingProgressSink::default());
        let tracker = Arc::new(JobTracker::start(
            JobKind::Launch,
            "authlib-injector (Ely.by agent)",
            JobStage::Downloading,
            sink.clone(),
        ));
        let error = Downloader::settle(&tracker, Err(AppError::Network("connection reset".into())))
            .await
            .expect_err("failed download");
        assert_eq!(error.code(), crate::error::CODE_NETWORK);
        let last = sink.snapshot().last().cloned().expect("event");
        assert_eq!(last.stage, JobStage::Failed);
        assert!(last.finished);
    }

    #[test]
    fn sha1_matches_the_known_digest() {
        // Well-known test vector.
        assert_eq!(
            sha1_bytes(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn cache_paths_are_sharded_by_the_first_byte() {
        let temp = tempdir();
        let dl = downloader(temp.path());

        let path = dl.cache_path("abcdef1234567890");
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(
            text.ends_with("/ab/cdef1234567890"),
            "unexpected layout: {text}"
        );

        // The same digest in a different case must resolve to one file.
        assert_eq!(dl.cache_path("ABcdef1234567890"), path);
        assert_eq!(dl.cache_path("  abcdef1234567890 "), path);
    }

    #[test]
    fn cache_paths_never_panic_on_odd_input() {
        let temp = tempdir();
        let dl = downloader(temp.path());
        let cache = dl.cache_dir();

        // Too short to shard: the digest is used as-is.
        assert_eq!(dl.cache_path("ab"), cache.join("ab"));
        // Non-ASCII must not panic on a byte-boundary slice.
        assert!(dl.cache_path("ünïcödé").starts_with(cache));
        assert!(dl.cache_path("").starts_with(cache));
    }

    #[test]
    fn partial_paths_are_siblings_of_the_target() {
        let destination = Path::new("/x/mods/sodium.jar");
        let partial = partial_path(destination);
        let text = partial.to_string_lossy().replace('\\', "/");
        assert_eq!(partial.parent(), destination.parent());
        assert!(
            text.starts_with("/x/mods/sodium.jar.") && text.ends_with(".part"),
            "unexpected partial path: {text}"
        );
        // Concurrent downloads of one object must not share a partial file.
        assert_ne!(partial_path(destination), partial);
    }

    #[test]
    fn directory_size_sums_files() {
        let temp = tempdir();
        let dir = temp.path().join("mods");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("a.jar"), vec![0u8; 100]).expect("write");
        std::fs::write(dir.join("b.jar"), vec![0u8; 50]).expect("write");
        assert_eq!(directory_size(&dir), 150);
    }

    /// Minimal temp-dir helper (avoids an extra dev-dependency).
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn tempdir() -> TempDir {
        let mut path = std::env::temp_dir();
        path.push(format!("sxml-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }
}
