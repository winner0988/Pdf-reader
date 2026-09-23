//! Scheduling and caching for `render_page` (MVP-07).
//!
//! Requests wait in a first-in, first-out queue served by one thread (the worker renders one
//! page at a time). The frontend cancels requests for pages that scrolled out of view; a
//! cancelled request that has not started is dropped and answered with `cancelled`. Finished
//! pages go into an LRU cache bounded by bytes, so scrolling back does not render again.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use ipc_contract::types::{DocumentId, ErrorCode, IpcError, RenderPageArgs, RequestId, Rotation};
use tokio::sync::oneshot;

/// Initial cache budget for rendered pages.
pub const DEFAULT_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// Identifies one rendering of a page. Scales are compared to 1/1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    doc: DocumentId,
    page_index: u32,
    rotation: Rotation,
    scale_milli: u32,
}

impl CacheKey {
    pub fn of(args: &RenderPageArgs) -> Self {
        Self {
            doc: args.doc,
            page_index: args.page_index,
            rotation: args.rotation,
            scale_milli: (f64::from(args.scale) * 1000.0).round() as u32,
        }
    }
}

/// Least-recently-used cache of encoded pages, bounded by their total size.
pub struct RasterCache {
    capacity: usize,
    used: usize,
    clock: u64,
    entries: HashMap<CacheKey, Entry>,
}

struct Entry {
    bytes: Arc<Vec<u8>>,
    last_used: u64,
}

impl RasterCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(entry.bytes.clone())
    }

    /// Adds a page, evicting the least recently used ones until it fits. A page larger than the
    /// whole budget is not cached.
    pub fn insert(&mut self, key: CacheKey, bytes: Arc<Vec<u8>>) {
        if bytes.len() > self.capacity {
            return;
        }
        self.remove(&key);
        while self.used + bytes.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
                .expect("used > 0 implies an entry");
            self.remove(&oldest);
        }
        self.clock += 1;
        self.used += bytes.len();
        self.entries.insert(
            key,
            Entry {
                bytes,
                last_used: self.clock,
            },
        );
    }

    /// Drops every page that does not belong to `doc`.
    pub fn retain_document(&mut self, doc: Option<DocumentId>) {
        let stale: Vec<CacheKey> = self
            .entries
            .keys()
            .filter(|key| Some(key.doc) != doc)
            .copied()
            .collect();
        for key in stale {
            self.remove(&key);
        }
    }

    #[cfg(test)]
    fn used_bytes(&self) -> usize {
        self.used
    }

    fn remove(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.used -= entry.bytes.len();
        }
    }
}

type Reply = oneshot::Sender<Result<Arc<Vec<u8>>, IpcError>>;

struct Job {
    args: RenderPageArgs,
    reply: Reply,
}

struct State {
    queue: VecDeque<Job>,
    cache: RasterCache,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

pub struct Renderer {
    shared: Arc<Shared>,
}

impl Renderer {
    /// Starts the render thread. `render` produces a page in the `render_page` wire format.
    pub fn start(
        cache_bytes: usize,
        render: impl Fn(&RenderPageArgs) -> Result<Vec<u8>, IpcError> + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                cache: RasterCache::new(cache_bytes),
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("render".to_owned())
            .spawn(move || serve(&worker, &render))
            .expect("start the render thread");
        Self { shared }
    }

    /// Returns the page from the cache, or queues it and waits for the render thread.
    pub async fn render(&self, args: RenderPageArgs) -> Result<Arc<Vec<u8>>, IpcError> {
        let receiver = {
            let mut state = self.shared.lock();
            if let Some(bytes) = state.cache.get(&CacheKey::of(&args)) {
                return Ok(bytes);
            }
            let (reply, receiver) = oneshot::channel();
            state.queue.push_back(Job { args, reply });
            receiver
        };
        self.shared.wake.notify_one();
        receiver.await.unwrap_or_else(|_| {
            Err(IpcError {
                code: ErrorCode::Internal,
                message: "the renderer stopped".to_owned(),
            })
        })
    }

    /// Drops the queued request `request`, which then fails with `cancelled`. A request that is
    /// already rendering finishes (and is cached); the frontend ignores its answer.
    pub fn cancel(&self, request: RequestId) {
        let cancelled: VecDeque<Job> = {
            let mut state = self.shared.lock();
            let (cancelled, kept) = std::mem::take(&mut state.queue)
                .into_iter()
                .partition(|job| job.args.request == request);
            state.queue = kept;
            cancelled
        };
        for job in cancelled {
            let _ = job.reply.send(Err(IpcError {
                code: ErrorCode::Cancelled,
                message: "cancelled".to_owned(),
            }));
        }
    }

    /// Forgets everything about documents other than `doc` (the one now open, if any):
    /// cached pages are freed and queued requests fail with `unknownDocument`.
    pub fn retain_document(&self, doc: Option<DocumentId>) {
        let dropped: VecDeque<Job> = {
            let mut state = self.shared.lock();
            state.cache.retain_document(doc);
            let (kept, dropped) = std::mem::take(&mut state.queue)
                .into_iter()
                .partition(|job| Some(job.args.doc) == doc);
            state.queue = kept;
            dropped
        };
        for job in dropped {
            let _ = job.reply.send(Err(IpcError {
                code: ErrorCode::UnknownDocument,
                message: "the document was closed".to_owned(),
            }));
        }
    }

    #[cfg(test)]
    fn cached_bytes(&self) -> usize {
        self.shared.lock().cache.used_bytes()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.wake.notify_all();
    }
}

fn serve(shared: &Shared, render: &dyn Fn(&RenderPageArgs) -> Result<Vec<u8>, IpcError>) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.stopped {
                    return;
                }
                if let Some(job) = state.queue.pop_front() {
                    break job;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        };
        let key = CacheKey::of(&job.args);
        // The same page may have been requested twice; the second waits for the first.
        let cached = shared.lock().cache.get(&key);
        let result = match cached {
            Some(bytes) => Ok(bytes),
            None => render(&job.args).map(Arc::new),
        };
        if let Ok(bytes) = &result {
            shared.lock().cache.insert(key, bytes.clone());
        }
        let _ = job.reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use tauri::async_runtime::block_on;

    use super::*;

    fn args(request: u32, doc: u32, page_index: u32) -> RenderPageArgs {
        RenderPageArgs {
            request: RequestId(request),
            doc: DocumentId(doc),
            page_index,
            scale: 1.5,
            rotation: Rotation::None,
        }
    }

    fn key(doc: u32, page_index: u32) -> CacheKey {
        CacheKey::of(&args(0, doc, page_index))
    }

    #[test]
    fn cache_keys_ignore_the_request_id_but_not_the_view() {
        assert_eq!(CacheKey::of(&args(1, 1, 0)), CacheKey::of(&args(2, 1, 0)));
        let mut rotated = args(1, 1, 0);
        rotated.rotation = Rotation::Cw90;
        assert_ne!(CacheKey::of(&rotated), CacheKey::of(&args(1, 1, 0)));
        let mut zoomed = args(1, 1, 0);
        zoomed.scale = 1.501;
        assert_ne!(CacheKey::of(&zoomed), CacheKey::of(&args(1, 1, 0)));
    }

    #[test]
    fn lru_evicts_the_least_recently_used_page() {
        let mut cache = RasterCache::new(300);
        cache.insert(key(1, 0), Arc::new(vec![0; 100]));
        cache.insert(key(1, 1), Arc::new(vec![0; 100]));
        cache.insert(key(1, 2), Arc::new(vec![0; 100]));
        assert!(cache.get(&key(1, 0)).is_some()); // page 1 is now the oldest

        cache.insert(key(1, 3), Arc::new(vec![0; 100]));
        assert!(cache.get(&key(1, 1)).is_none());
        assert!(cache.get(&key(1, 0)).is_some());
        assert!(cache.get(&key(1, 3)).is_some());
        assert_eq!(cache.used_bytes(), 300);

        // One large page may push out several small ones.
        cache.insert(key(1, 4), Arc::new(vec![0; 250]));
        assert_eq!(cache.used_bytes(), 250);
        assert!(cache.get(&key(1, 4)).is_some());
    }

    #[test]
    fn lru_skips_pages_larger_than_the_budget_and_replaces_duplicates() {
        let mut cache = RasterCache::new(100);
        cache.insert(key(1, 0), Arc::new(vec![0; 101]));
        assert_eq!(cache.used_bytes(), 0);
        cache.insert(key(1, 0), Arc::new(vec![0; 60]));
        cache.insert(key(1, 0), Arc::new(vec![0; 70]));
        assert_eq!(cache.used_bytes(), 70);
    }

    #[test]
    fn lru_forgets_other_documents() {
        let mut cache = RasterCache::new(1000);
        cache.insert(key(1, 0), Arc::new(vec![0; 10]));
        cache.insert(key(2, 0), Arc::new(vec![0; 20]));
        cache.retain_document(Some(DocumentId(2)));
        assert_eq!(cache.used_bytes(), 20);
        cache.retain_document(None);
        assert_eq!(cache.used_bytes(), 0);
    }

    /// Controls a renderer whose render function blocks each call until the test releases it.
    struct Gate {
        release: mpsc::Sender<()>,
        calls: Arc<AtomicUsize>,
    }

    impl Gate {
        fn release(&self) {
            self.release.send(()).unwrap();
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn gated_renderer() -> (Arc<Renderer>, Gate) {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let renderer = Renderer::start(1024, move |args: &RenderPageArgs| {
            counter.fetch_add(1, Ordering::SeqCst);
            gate.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(vec![args.page_index as u8; 16])
        });
        (Arc::new(renderer), Gate { release, calls })
    }

    #[test]
    fn rendered_pages_are_served_from_the_cache() {
        let (renderer, gate) = gated_renderer();
        gate.release();
        let first = block_on(renderer.render(args(1, 1, 3))).unwrap();
        let again = block_on(renderer.render(args(2, 1, 3))).unwrap();
        assert_eq!(first, again);
        assert_eq!(gate.calls(), 1);
        assert_eq!(renderer.cached_bytes(), 16);
    }

    #[test]
    fn queued_requests_can_be_cancelled() {
        let (renderer, gate) = gated_renderer();

        // Request 1 starts rendering and blocks; requests 2 and 3 wait in the queue.
        let spawn = |request, page| {
            let renderer = renderer.clone();
            std::thread::spawn(move || block_on(renderer.render(args(request, 1, page))))
        };
        let first = spawn(1, 0);
        while gate.calls() == 0 {
            std::thread::yield_now();
        }
        let second = spawn(2, 1);
        let third = spawn(3, 2);
        while renderer.shared.lock().queue.len() < 2 {
            std::thread::yield_now();
        }

        renderer.cancel(RequestId(2));
        assert_eq!(
            second.join().unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
        // Cancelling the running request has no effect; it completes normally.
        renderer.cancel(RequestId(1));
        gate.release();
        gate.release();
        assert!(first.join().unwrap().is_ok());
        assert_eq!(*third.join().unwrap().unwrap(), vec![2u8; 16]);
        assert_eq!(gate.calls(), 2, "the cancelled page was never rendered");
    }

    #[test]
    fn closing_a_document_fails_its_queued_requests() {
        let (renderer, gate) = gated_renderer();
        let spawn = |request, doc| {
            let renderer = renderer.clone();
            std::thread::spawn(move || block_on(renderer.render(args(request, doc, 0))))
        };
        let running = spawn(1, 1);
        while gate.calls() == 0 {
            std::thread::yield_now();
        }
        let old = spawn(2, 1);
        let new = spawn(3, 2);
        while renderer.shared.lock().queue.len() < 2 {
            std::thread::yield_now();
        }

        renderer.retain_document(Some(DocumentId(2)));
        assert_eq!(
            old.join().unwrap().unwrap_err().code,
            ErrorCode::UnknownDocument
        );
        gate.release();
        gate.release();
        assert!(running.join().unwrap().is_ok());
        assert!(new.join().unwrap().is_ok());
    }

    #[test]
    fn render_errors_are_returned_and_not_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let renderer = Renderer::start(1024, move |_: &RenderPageArgs| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(IpcError {
                code: ErrorCode::WorkerCrashed,
                message: String::new(),
            })
        });
        for request in 1..=2 {
            let error = block_on(renderer.render(args(request, 1, 0))).unwrap_err();
            assert_eq!(error.code, ErrorCode::WorkerCrashed);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2, "a retry renders again");
        assert_eq!(renderer.cached_bytes(), 0);
    }
}
