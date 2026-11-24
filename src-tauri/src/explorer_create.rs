//! Collects image paths arriving from Explorer's context menu (`--create-pdf-from`, B5-13, #450)
//! into batches and hands each batch to the shared PDF creation flow one at a time.
//!
//! When a user selects multiple images in File Explorer and invokes a static shell verb, Windows
//! may launch `pdf-reader.exe --create-pdf-from "<path>"` once per selected file. The first launch
//! becomes the running instance and subsequent launches forward their arguments via
//! `tauri-plugin-single-instance`. This module gathers those rapid arrivals into one batch:
//! - Starts after [`COLLECT_IDLE`] (1.5 s) with no new arrival;
//! - Waits at most [`COLLECT_MAX_WAIT`] (60 s) from the first arrival of the batch;
//! - Files arriving after a batch starts belong to the next batch, queued behind the active one;
//! - Filters to existing regular files with one of the seven supported image extensions
//!   (case-insensitive), deduplicates by canonical path (`same_file`), sorts by natural file-name
//!   order, and rejects batches exceeding [`MAX_CREATE_SOURCES`] (200).
//! - Paths never leave the main process (ADR 0008); the frontend only receives counts and status.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ipc_contract::limits::MAX_CREATE_SOURCES;
use ipc_contract::types::{CreateKind, ExplorerCreateStatus, OpenEvent};
use tauri::{AppHandle, Manager};

use crate::commands;
use crate::create_pdf;
use crate::events::OpenEvents;

/// Wait this long after the most recent arrival before starting PDF creation.
pub const COLLECT_IDLE: Duration = Duration::from_millis(1_500);

/// Maximum time to wait from the first arrival of a batch before starting PDF creation.
pub const COLLECT_MAX_WAIT: Duration = Duration::from_secs(60);

/// Checks whether a candidate path is an existing regular image file and returns its deduplication
/// key (canonical path matching `documents::same_file`).
pub trait FileProbe {
    fn probe(&self, path: &Path) -> Option<PathBuf>;
}

/// Real filesystem probe used at runtime.
pub struct RealFileProbe;

impl FileProbe for RealFileProbe {
    fn probe(&self, path: &Path) -> Option<PathBuf> {
        if create_pdf::kind_of(path) != Some(CreateKind::Image) {
            return None;
        }
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        // Matches `documents::same_file` (case-insensitive, short names, links on Windows).
        Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    }
}

/// A settled batch ready to be handed to the PDF creation flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyBatch {
    /// Valid, deduplicated image files sorted by natural file-name order. Empty when all files
    /// were skipped.
    pub files: Vec<PathBuf>,
    /// Count of incoming paths skipped (missing, directory, or unsupported extension).
    pub skipped: usize,
    /// True when more than `MAX_CREATE_SOURCES` distinct valid images arrived in this batch.
    pub too_many: bool,
}

#[derive(Debug)]
struct PendingBatch {
    first_at: Instant,
    last_at: Instant,
    files: Vec<PathBuf>,
    canonical: HashSet<PathBuf>,
    skipped: usize,
    too_many: bool,
}

impl PendingBatch {
    fn new(now: Instant) -> Self {
        Self {
            first_at: now,
            last_at: now,
            files: Vec::new(),
            canonical: HashSet::new(),
            skipped: 0,
            too_many: false,
        }
    }

    fn deadline(&self) -> Instant {
        (self.last_at + COLLECT_IDLE).min(self.first_at + COLLECT_MAX_WAIT)
    }

    fn counts(&self) -> (u32, u32) {
        let valid = if self.too_many {
            MAX_CREATE_SOURCES + 1
        } else {
            self.files.len()
        };
        (
            u32::try_from(valid).unwrap_or(u32::MAX),
            u32::try_from(self.skipped).unwrap_or(u32::MAX),
        )
    }

    fn finish(mut self) -> ReadyBatch {
        create_pdf::sort_paths(&mut self.files);
        ReadyBatch {
            files: self.files,
            skipped: self.skipped,
            too_many: self.too_many,
        }
    }
}

/// Pure, clock-injected batch collector.
#[derive(Debug, Default)]
pub struct Collector {
    pending: Option<PendingBatch>,
}

impl Collector {
    /// Probes `paths` with `probe` and adds them at `now`.
    ///
    /// If a previously collecting batch had already reached its deadline by `now`, it is flushed
    /// and returned as the first element so these new paths begin a fresh batch. The second
    /// element holds `(valid_count, skipped_count)` of the active collecting batch.
    pub fn add(
        &mut self,
        paths: Vec<PathBuf>,
        now: Instant,
        probe: &dyn FileProbe,
    ) -> (Option<ReadyBatch>, Option<(u32, u32)>) {
        let flushed = self.take_ready(now);
        if paths.is_empty() {
            return (flushed, self.pending.as_ref().map(PendingBatch::counts));
        }
        let batch = self.pending.get_or_insert_with(|| PendingBatch::new(now));
        batch.last_at = now;
        for path in paths {
            if let Some(canonical_key) = probe.probe(&path) {
                if batch.too_many {
                    continue;
                }
                if batch.canonical.insert(canonical_key) {
                    if batch.files.len() >= MAX_CREATE_SOURCES {
                        batch.too_many = true;
                    } else {
                        batch.files.push(path);
                    }
                }
            } else {
                batch.skipped += 1;
            }
        }
        (flushed, Some(batch.counts()))
    }

    /// Next instant at which the active collecting batch will settle, if any.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(PendingBatch::deadline)
    }

    /// Flushes and returns the collecting batch if its idle or max-wait deadline has elapsed at
    /// `now`.
    pub fn take_ready(&mut self, now: Instant) -> Option<ReadyBatch> {
        let deadline = self.next_deadline()?;
        if now < deadline {
            return None;
        }
        self.pending.take().map(PendingBatch::finish)
    }
}

#[derive(Default)]
struct SharedStatus {
    /// Active creating status while a batch is being converted (takes precedence in snapshot).
    running: Option<ExplorerCreateStatus>,
    /// Active collecting status while gathering incoming files.
    collecting: Option<ExplorerCreateStatus>,
}

impl SharedStatus {
    fn snapshot(&self) -> Option<OpenEvent> {
        self.running
            .clone()
            .or_else(|| self.collecting.clone())
            .map(|status| OpenEvent::ExplorerCreate { status })
    }
}

/// Tauri-managed state coordinating `--create-pdf-from` launches.
pub struct ExplorerCreate {
    incoming_tx: mpsc::Sender<Vec<PathBuf>>,
    incoming_rx: Mutex<Option<mpsc::Receiver<Vec<PathBuf>>>>,
    status: Arc<Mutex<SharedStatus>>,
}

impl ExplorerCreate {
    pub fn new() -> Self {
        let (incoming_tx, incoming_rx) = mpsc::channel();
        Self {
            incoming_tx,
            incoming_rx: Mutex::new(Some(incoming_rx)),
            status: Arc::new(Mutex::new(SharedStatus::default())),
        }
    }

    /// Queues paths from a `--create-pdf-from` launch without blocking the caller.
    pub fn enqueue(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let _ = self.incoming_tx.send(paths);
    }

    /// Current in-progress status for a newly subscribed frontend channel.
    pub fn snapshot(&self) -> Option<OpenEvent> {
        self.status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .snapshot()
    }

    /// Starts the background collector thread and async batch runner task once Tauri's runtime is
    /// ready.
    pub fn start(&self, app: AppHandle) {
        let Some(incoming_rx) = self
            .incoming_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        else {
            return;
        };

        let (ready_tx, mut ready_rx) = tokio::sync::mpsc::unbounded_channel::<ReadyBatch>();
        let collector_status = Arc::clone(&self.status);
        let collector_app = app.clone();

        let _ = std::thread::Builder::new()
            .name("explorer-create-collector".to_owned())
            .spawn(move || {
                let mut collector = Collector::default();
                loop {
                    let received = match collector.next_deadline() {
                        Some(deadline) => {
                            let wait = deadline.saturating_duration_since(Instant::now());
                            match incoming_rx.recv_timeout(wait) {
                                Ok(paths) => Some(paths),
                                Err(RecvTimeoutError::Timeout) => {
                                    if let Some(batch) = collector.take_ready(Instant::now()) {
                                        clear_collecting(&collector_status);
                                        let _ = ready_tx.send(batch);
                                    }
                                    continue;
                                }
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        None => match incoming_rx.recv() {
                            Ok(paths) => Some(paths),
                            Err(_) => break,
                        },
                    };

                    if let Some(paths) = received {
                        handle_arrival(
                            &collector_app,
                            &collector_status,
                            &mut collector,
                            &ready_tx,
                            paths,
                        );
                    }
                }
            });

        let runner_status = Arc::clone(&self.status);
        tauri::async_runtime::spawn(async move {
            while let Some(batch) = ready_rx.recv().await {
                let status_ref = Arc::clone(&runner_status);
                let app_ref = app.clone();
                let final_status = commands::create_pdf_from_explorer(&app, batch, move |status| {
                    let mut guard = status_ref
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    guard.running = match &status {
                        ExplorerCreateStatus::Idle => None,
                        other => Some(other.clone()),
                    };
                    drop(guard);
                    app_ref
                        .state::<OpenEvents>()
                        .send(OpenEvent::ExplorerCreate { status });
                })
                .await;

                let next_collecting = {
                    let mut guard = runner_status
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    guard.running = None;
                    guard.collecting.clone()
                };
                app.state::<OpenEvents>().send(OpenEvent::ExplorerCreate {
                    status: final_status,
                });
                // If another batch is already collecting behind us, surface its active status.
                if let Some(collecting) = next_collecting {
                    app.state::<OpenEvents>()
                        .send(OpenEvent::ExplorerCreate { status: collecting });
                }
            }
        });
    }
}

fn handle_arrival(
    app: &AppHandle,
    status: &Arc<Mutex<SharedStatus>>,
    collector: &mut Collector,
    ready_tx: &tokio::sync::mpsc::UnboundedSender<ReadyBatch>,
    paths: Vec<PathBuf>,
) {
    let (flushed, counts) = collector.add(paths, Instant::now(), &RealFileProbe);
    if let Some(batch) = flushed {
        clear_collecting(status);
        let _ = ready_tx.send(batch);
    }
    if let Some((count, skipped)) = counts {
        let next = ExplorerCreateStatus::Collecting { count, skipped };
        let should_emit = {
            let mut guard = status.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.collecting = Some(next.clone());
            guard.running.is_none()
        };
        if should_emit {
            app.state::<OpenEvents>()
                .send(OpenEvent::ExplorerCreate { status: next });
        }
    }
}

fn clear_collecting(status: &Arc<Mutex<SharedStatus>>) {
    status
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .collecting = None;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Fake probe that accepts supported image extensions (case-insensitive) and canonicalises
    /// paths by lowercasing, without touching the filesystem.
    struct FakeProbe {
        missing: HashSet<String>,
    }

    impl FakeProbe {
        fn new() -> Self {
            Self {
                missing: HashSet::new(),
            }
        }
    }

    impl FileProbe for FakeProbe {
        fn probe(&self, path: &Path) -> Option<PathBuf> {
            let text = path.to_string_lossy().to_ascii_lowercase();
            if self.missing.contains(&text) {
                return None;
            }
            if create_pdf::kind_of(path) != Some(CreateKind::Image) {
                return None;
            }
            Some(PathBuf::from(text))
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pdf-reader-explorer-create-{label}-{}-{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn waits_for_idle_window_and_caps_at_max_wait() {
        let probe = FakeProbe::new();
        let mut collector = Collector::default();
        let t0 = Instant::now();

        let (flushed, counts) = collector.add(vec![PathBuf::from(r"C:\img\1.png")], t0, &probe);
        assert!(flushed.is_none());
        assert_eq!(counts, Some((1, 0)));
        assert_eq!(collector.next_deadline(), Some(t0 + COLLECT_IDLE));

        // Still waiting just before 1.5 s.
        assert!(
            collector
                .take_ready(t0 + COLLECT_IDLE - Duration::from_millis(1))
                .is_none()
        );

        // Second file arrives at 1.0 s -> pushes idle deadline to 2.5 s.
        let t1 = t0 + Duration::from_secs(1);
        let (flushed, counts) = collector.add(vec![PathBuf::from(r"C:\img\2.png")], t1, &probe);
        assert!(flushed.is_none());
        assert_eq!(counts, Some((2, 0)));
        assert_eq!(collector.next_deadline(), Some(t1 + COLLECT_IDLE));
        assert!(collector.take_ready(t0 + COLLECT_IDLE).is_none());

        // Settles at 2.5 s.
        let batch = collector
            .take_ready(t1 + COLLECT_IDLE)
            .expect("batch ready");
        assert_eq!(batch.files.len(), 2);
        assert_eq!(batch.skipped, 0);
        assert!(!batch.too_many);
        assert!(collector.next_deadline().is_none());

        // Continuous arrivals every 1 s cannot defer past COLLECT_MAX_WAIT (60 s).
        let start = t0 + Duration::from_secs(10);
        for second in 0..60 {
            let now = start + Duration::from_secs(second);
            let path = PathBuf::from(format!(r"C:\img\p{second}.jpg"));
            let (flushed, _) = collector.add(vec![path], now, &probe);
            assert!(
                flushed.is_none(),
                "must not flush before max wait at {second}s"
            );
        }
        // At 59 s, last_at + 1.5 s is 60.5 s, clamped to start + 60 s.
        assert_eq!(collector.next_deadline(), Some(start + COLLECT_MAX_WAIT));
        assert!(
            collector
                .take_ready(start + COLLECT_MAX_WAIT - Duration::from_millis(1))
                .is_none()
        );
        let batch = collector
            .take_ready(start + COLLECT_MAX_WAIT)
            .expect("must flush at max wait");
        assert_eq!(batch.files.len(), 60);
    }

    #[test]
    fn arrivals_after_batch_starts_become_a_new_batch() {
        let probe = FakeProbe::new();
        let mut collector = Collector::default();
        let t0 = Instant::now();

        collector.add(vec![PathBuf::from(r"C:\img\a.png")], t0, &probe);
        let first = collector
            .take_ready(t0 + COLLECT_IDLE)
            .expect("first batch");
        assert_eq!(first.files, vec![PathBuf::from(r"C:\img\a.png")]);

        // Arrives right after first batch started.
        let t_next = t0 + COLLECT_IDLE + Duration::from_millis(10);
        let (flushed, counts) = collector.add(vec![PathBuf::from(r"C:\img\b.png")], t_next, &probe);
        assert!(flushed.is_none());
        assert_eq!(counts, Some((1, 0)));

        let second = collector
            .take_ready(t_next + COLLECT_IDLE)
            .expect("second batch");
        assert_eq!(second.files, vec![PathBuf::from(r"C:\img\b.png")]);

        // Also if an arrival occurs AFTER the previous batch's deadline before take_ready was
        // called, the expired batch is flushed and the late arrival starts a brand new batch.
        let t_a = t_next + Duration::from_secs(10);
        collector.add(vec![PathBuf::from(r"C:\img\c.png")], t_a, &probe);
        let t_b = t_a + COLLECT_IDLE + Duration::from_millis(50);
        let (flushed, counts) = collector.add(vec![PathBuf::from(r"C:\img\d.png")], t_b, &probe);
        assert_eq!(
            flushed.expect("expired batch flushed").files,
            vec![PathBuf::from(r"C:\img\c.png")]
        );
        assert_eq!(counts, Some((1, 0)));
        let fourth = collector
            .take_ready(t_b + COLLECT_IDLE)
            .expect("new batch for late arrival");
        assert_eq!(fourth.files, vec![PathBuf::from(r"C:\img\d.png")]);
    }

    #[test]
    fn filters_missing_dirs_unsupported_accepts_uppercase_and_deduplicates_with_real_files() {
        let dir = TempDir::new("real-probe");
        let root = dir.path();

        let png = root.join("photo.png");
        let jpg_upper = root.join("SCAN.JPG");
        let tif_mixed = root.join("Page.Tif");
        let unsupported = root.join("notes.txt");
        let pdf = root.join("doc.pdf");
        let subdir = root.join("folder.png");
        let missing = root.join("missing.png");

        std::fs::write(&png, b"png").unwrap();
        std::fs::write(&jpg_upper, b"jpg").unwrap();
        std::fs::write(&tif_mixed, b"tif").unwrap();
        std::fs::write(&unsupported, b"txt").unwrap();
        std::fs::write(&pdf, b"pdf").unwrap();
        std::fs::create_dir(&subdir).unwrap();

        // Windows filesystem is case-insensitive: PHOTO.PNG refers to the same file as photo.png.
        let duplicate_case = root.join("PHOTO.PNG");
        let duplicate_dot = root.join(".").join("photo.png");

        let mut collector = Collector::default();
        let t0 = Instant::now();
        let (_, counts) = collector.add(
            vec![
                png.clone(),
                duplicate_case,
                duplicate_dot,
                jpg_upper.clone(),
                tif_mixed.clone(),
                unsupported,
                pdf,
                subdir,
                missing,
            ],
            t0,
            &RealFileProbe,
        );
        assert_eq!(counts, Some((3, 4)));

        let batch = collector.take_ready(t0 + COLLECT_IDLE).expect("ready");
        assert!(!batch.too_many);
        assert_eq!(batch.skipped, 4);
        let names: Vec<String> = batch.files.iter().map(|p| name(p)).collect();
        assert_eq!(names, ["Page.Tif", "photo.png", "SCAN.JPG"]);
    }

    #[test]
    fn rejects_more_than_max_create_sources_and_allows_exact_limit_with_duplicates() {
        let probe = FakeProbe::new();
        let mut collector = Collector::default();
        let t0 = Instant::now();

        // Exactly 200 distinct images plus duplicates -> allowed.
        let exact: Vec<PathBuf> = (1..=MAX_CREATE_SOURCES)
            .map(|n| PathBuf::from(format!(r"C:\img\{n}.png")))
            .collect();
        collector.add(exact, t0, &probe);
        // Add duplicates of existing files -> still 200.
        let (_, counts) = collector.add(
            vec![
                PathBuf::from(r"C:\img\1.PNG"),
                PathBuf::from(r"C:\img\200.png"),
            ],
            t0 + Duration::from_millis(200),
            &probe,
        );
        assert_eq!(counts, Some((MAX_CREATE_SOURCES as u32, 0)));
        let ok_batch = collector
            .take_ready(t0 + Duration::from_millis(200) + COLLECT_IDLE)
            .expect("200 allowed");
        assert!(!ok_batch.too_many);
        assert_eq!(ok_batch.files.len(), MAX_CREATE_SOURCES);

        // 201 distinct images -> too_many.
        let t1 = t0 + Duration::from_secs(5);
        let over: Vec<PathBuf> = (1..=MAX_CREATE_SOURCES + 1)
            .map(|n| PathBuf::from(format!(r"C:\img\{n}.jpg")))
            .collect();
        let (_, counts) = collector.add(over, t1, &probe);
        assert_eq!(counts, Some(((MAX_CREATE_SOURCES + 1) as u32, 0)));
        let over_batch = collector.take_ready(t1 + COLLECT_IDLE).expect("over ready");
        assert!(over_batch.too_many);
    }

    #[test]
    fn sorts_by_natural_file_name_order() {
        let probe = FakeProbe::new();
        let mut collector = Collector::default();
        let t0 = Instant::now();

        collector.add(
            vec![
                PathBuf::from(r"C:\img\10.png"),
                PathBuf::from(r"C:\img\page 01b.gif"),
                PathBuf::from(r"C:\img\2.jpg"),
                PathBuf::from(r"C:\img\Page 1.png"),
            ],
            t0,
            &probe,
        );
        let batch = collector.take_ready(t0 + COLLECT_IDLE).expect("sorted");
        let names: Vec<String> = batch.files.iter().map(|p| name(p)).collect();
        assert_eq!(names, ["2.jpg", "10.png", "Page 1.png", "page 01b.gif"]);
    }
}
