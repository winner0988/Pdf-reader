//! The thread that recognises pages (B2-10, ADR 0015), so that the worker goes on answering
//! requests (renders above all) while a page is read. The request loop renders a page to grey
//! pixels, queues them here and later collects the text; Tesseract and Leptonica are only ever
//! called from this thread, which owns the one [`Recogniser`] of the process.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use ipc_contract::limits::{MAX_OCR_QUEUE, MAX_PAGE_TEXT_CHARS};
use ipc_contract::types::{DocumentId, PageText, Point, Quad};

use crate::ocr::{OcrChar, OcrError, OcrLine, Recogniser};
use crate::text_layer::TextLayerBuilder;

/// How long loading a language may take before the request that asked for it is answered with an
/// error (the recogniser may still finish loading; the next load replaces it).
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// A page drawn in grey for recognising: 8 bits a pixel, a row after another with no padding.
pub struct Picture {
    pub grey: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The resolution it was drawn at, in pixels an inch.
    pub ppi: u32,
}

/// Where the picture is on the page, so that pixels become points of the page space the text
/// layer and search use: origin top left, y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The page's top left corner, in page space.
    pub x0: f32,
    pub y0: f32,
    /// Pixels in a point of the page.
    pub pixels_per_point: f32,
}

impl Geometry {
    fn quad(&self, c: &OcrChar) -> Quad {
        let at = |x: i32, y: i32| Point {
            x: self.x0 + x as f32 / self.pixels_per_point,
            y: self.y0 + y as f32 / self.pixels_per_point,
        };
        Quad {
            ul: at(c.left, c.top),
            ur: at(c.right, c.top),
            ll: at(c.left, c.bottom),
            lr: at(c.right, c.bottom),
        }
    }
}

/// The lines recognised in a picture as the text of a page: the cleaned lines and where each
/// character is, as for the text a page has, marked as recognised.
pub fn page_text(lines: &[OcrLine], geometry: Geometry) -> PageText {
    let mut builder = TextLayerBuilder::new(MAX_PAGE_TEXT_CHARS as usize);
    for line in lines {
        builder.push_line(line.iter().map(|c| (c.ch, geometry.quad(c))));
        if builder.is_full() {
            break;
        }
    }
    let mut text = builder.finish();
    text.recognised = true;
    text
}

/// A page to recognise.
pub struct Job {
    pub doc: DocumentId,
    /// The document's pages as they were when the picture was drawn (see `engine`): a result for
    /// pages that have changed since is dropped.
    pub epoch: u64,
    /// The page's key in the document (the number of its object).
    pub page_key: i32,
    pub picture: Picture,
    pub geometry: Geometry,
    /// The most time the page may take.
    pub time: Duration,
}

/// A page that was recognised, or why not.
pub struct Finished {
    pub doc: DocumentId,
    pub epoch: u64,
    pub page_key: i32,
    pub outcome: Result<PageText, OcrError>,
}

/// A language to load, and where to say how it went.
struct Load {
    language: String,
    data: Vec<u8>,
    reply: mpsc::Sender<Result<(), OcrError>>,
}

#[derive(Default)]
struct State {
    loading: Option<Load>,
    /// The language of the recogniser, once it is loaded.
    language: Option<String>,
    queue: VecDeque<Job>,
    /// Stops the page being recognised.
    running: Option<Arc<AtomicBool>>,
    finished: Vec<Finished>,
    shutdown: bool,
}

impl State {
    fn has_room(&self) -> bool {
        let in_hand = self.queue.len() + usize::from(self.running.is_some()) + self.finished.len();
        self.language.is_some() && in_hand < MAX_OCR_QUEUE as usize
    }

    /// Drops the pages waiting and stops the one being read.
    fn stop(&mut self) {
        self.queue.clear();
        if let Some(running) = &self.running {
            running.store(true, Ordering::Relaxed);
        }
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic of the thread must not make the request loop panic too.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the request loop holds: pages go in, text comes out, in the background.
#[derive(Default)]
pub struct OcrWorker {
    shared: Option<Arc<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl OcrWorker {
    /// Loads `data`, the data of `language`, in place of what was loaded before; pages waiting
    /// are dropped and the one being read is stopped. Answers once it is loaded.
    pub fn load(&mut self, language: &str, data: Vec<u8>) -> Result<(), OcrError> {
        let shared = self.start()?;
        let (reply, answer) = mpsc::channel();
        {
            let mut state = shared.lock();
            state.stop();
            state.language = None;
            state.loading = Some(Load {
                language: language.to_owned(),
                data,
                reply,
            });
        }
        shared.wake.notify_all();
        match answer.recv_timeout(LOAD_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(OcrError::Init("loading took too long")),
            Err(RecvTimeoutError::Disconnected) => Err(OcrError::Init("the recogniser stopped")),
        }
    }

    /// The language that pages are recognised in, once it is loaded.
    pub fn language(&self) -> Option<String> {
        self.shared
            .as_ref()
            .and_then(|shared| shared.lock().language.clone())
    }

    /// Whether another page can be queued: a language is loaded, and fewer than the queue holds
    /// are waiting, being read or finished and not yet collected.
    pub fn has_room(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.lock().has_room())
    }

    /// Queues `job`, or gives it back if there is no room (see [`has_room`](Self::has_room)).
    pub fn enqueue(&self, job: Job) -> Result<(), Box<Job>> {
        let Some(shared) = &self.shared else {
            return Err(Box::new(job));
        };
        {
            let mut state = shared.lock();
            if !state.has_room() {
                return Err(Box::new(job));
            }
            state.queue.push_back(job);
        }
        shared.wake.notify_all();
        Ok(())
    }

    /// The pages finished since the last call, and how many are waiting or being read.
    pub fn take_finished(&self) -> (Vec<Finished>, usize) {
        let Some(shared) = &self.shared else {
            return (Vec::new(), 0);
        };
        let mut state = shared.lock();
        let waiting = state.queue.len() + usize::from(state.running.is_some());
        (std::mem::take(&mut state.finished), waiting)
    }

    /// Drops the pages waiting and stops the one being read.
    pub fn stop(&self) {
        if let Some(shared) = &self.shared {
            shared.lock().stop();
        }
    }
}

impl OcrWorker {
    /// Starts the thread the first time.
    fn start(&mut self) -> Result<Arc<Shared>, OcrError> {
        if let Some(shared) = &self.shared {
            return Ok(Arc::clone(shared));
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            // Tesseract recurses deeper than a thread's default stack is meant for.
            std::thread::Builder::new()
                .name("ocr".to_owned())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || run(&shared))
                .map_err(|_| OcrError::Init("no thread"))?
        };
        self.shared = Some(Arc::clone(&shared));
        self.thread = Some(thread);
        Ok(shared)
    }
}

impl Drop for OcrWorker {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            state.shutdown = true;
            state.stop();
            drop(state);
            shared.wake.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread: loads languages and recognises pages as they come, until it is told to end.
fn run(shared: &Shared) {
    crate::ocr::lower_thread_priority();
    let mut recogniser: Option<Recogniser> = None;
    let mut state = shared.lock();
    loop {
        if state.shutdown {
            return;
        }
        if let Some(load) = state.loading.take() {
            drop(state);
            // There is one recogniser at a time: the old one goes before the new one comes.
            recogniser = None;
            let reply = match Recogniser::new(&load.language, &load.data) {
                Ok(made) => {
                    recogniser = Some(made);
                    Ok(())
                }
                Err(error) => Err(error),
            };
            state = shared.lock();
            state.language = recogniser.is_some().then(|| load.language.clone());
            let _ = load.reply.send(reply);
            continue;
        }
        if let Some(job) = state.queue.pop_front() {
            let cancel = Arc::new(AtomicBool::new(false));
            state.running = Some(Arc::clone(&cancel));
            drop(state);
            let outcome = match recogniser.as_mut() {
                None => Err(OcrError::Init("no language is loaded")),
                Some(recogniser) => recognise(recogniser, &job, &cancel),
            };
            state = shared.lock();
            state.running = None;
            // A page that was stopped is not reported: whoever stopped it knows.
            if !matches!(outcome, Err(OcrError::Cancelled)) {
                state.finished.push(Finished {
                    doc: job.doc,
                    epoch: job.epoch,
                    page_key: job.page_key,
                    outcome,
                });
            }
            continue;
        }
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

fn recognise(
    recogniser: &mut Recogniser,
    job: &Job,
    cancel: &AtomicBool,
) -> Result<PageText, OcrError> {
    let Picture {
        grey,
        width,
        height,
        ppi,
    } = &job.picture;
    let lines = recogniser.recognise(grey, *width, *height, *ppi, job.time, cancel)?;
    Ok(page_text(&lines, job.geometry))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::ocr::one_at_a_time;
    use crate::ocr::tests::{grey_page, traineddata};

    const PAGE: &str = "benign/mixed-text-zh-en.pdf";

    fn job(page_key: i32, ppi: u32) -> Job {
        let (grey, width, height) = grey_page(PAGE);
        Job {
            doc: DocumentId(1),
            epoch: 7,
            page_key,
            picture: Picture {
                grey,
                width,
                height,
                ppi,
            },
            geometry: Geometry {
                x0: 0.0,
                y0: 0.0,
                pixels_per_point: ppi as f32 / 72.0,
            },
            time: Duration::from_secs(120),
        }
    }

    /// The pages finished once `count` of them are, or all there are after a minute.
    fn finished(worker: &OcrWorker, count: usize) -> Vec<Finished> {
        let started = Instant::now();
        let mut all = Vec::new();
        while all.len() < count && started.elapsed() < Duration::from_secs(60) {
            all.extend(worker.take_finished().0);
            std::thread::sleep(Duration::from_millis(20));
        }
        all
    }

    fn lines_of(text: &PageText) -> Vec<&str> {
        text.lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn characters_are_put_in_page_space_and_marked_as_recognised() {
        let c = |ch, left, top, right, bottom| OcrChar {
            ch,
            left,
            top,
            right,
            bottom,
        };
        // A picture drawn at 144 dpi (two pixels to the point) of a page whose corner is (10, 20).
        let geometry = Geometry {
            x0: 10.0,
            y0: 20.0,
            pixels_per_point: 2.0,
        };
        let lines = vec![
            vec![c('H', 100, 200, 120, 240), c('i', 130, 200, 140, 240)],
            vec![c(' ', 0, 300, 5, 340)],
            vec![c('a', 100, 400, 120, 440)],
        ];
        let text = page_text(&lines, geometry);
        assert!(text.recognised && !text.truncated);
        assert_eq!(lines_of(&text), ["Hi", "a"]);
        let line = &text.lines[0];
        assert_eq!(line.quad.ul, Point { x: 60.0, y: 120.0 });
        assert_eq!(line.quad.lr, Point { x: 80.0, y: 140.0 });
        assert_eq!(line.edges, [0.0, 15.0, 20.0]);
        assert!(ipc_contract::validate::Validate::validate(&text).is_ok());
    }

    #[test]
    fn a_page_is_recognised_in_the_background() {
        let _turn = one_at_a_time();
        let mut worker = OcrWorker::default();
        assert_eq!(worker.language(), None);
        assert!(!worker.has_room(), "no language, no room");
        assert!(worker.enqueue(job(5, 200)).is_err());
        worker.load("eng", traineddata("eng")).expect("english");
        assert_eq!(worker.language().as_deref(), Some("eng"));
        assert!(worker.enqueue(job(5, 200)).is_ok());
        let done = finished(&worker, 1);
        let [page] = &done[..] else {
            panic!("one page, got {}", done.len())
        };
        assert_eq!((page.doc, page.epoch, page.page_key), (DocumentId(1), 7, 5));
        let text = page.outcome.as_ref().expect("text");
        assert!(text.recognised);
        assert!(
            lines_of(text).contains(&"Privacy-first PDF Reader"),
            "{:?}",
            lines_of(text)
        );
        // Pixels at 200 dpi became points of a 612 x 792 point page.
        let first = &text.lines[0].quad;
        assert!(first.ul.x > 0.0 && first.lr.x < 612.0 && first.ul.y > 0.0 && first.lr.y < 792.0);
        assert_eq!(worker.take_finished().1, 0);
    }

    #[test]
    fn as_many_pages_as_the_queue_holds_are_taken_until_they_are_collected() {
        let _turn = one_at_a_time();
        let mut worker = OcrWorker::default();
        worker.load("eng", traineddata("eng")).expect("english");
        for key in 0..MAX_OCR_QUEUE as i32 {
            assert!(worker.has_room());
            assert!(worker.enqueue(job(key, 72)).is_ok(), "{key}");
        }
        assert!(!worker.has_room());
        assert!(worker.enqueue(job(99, 72)).is_err());
        let done = finished(&worker, MAX_OCR_QUEUE as usize);
        assert_eq!(done.len(), MAX_OCR_QUEUE as usize);
        assert!(worker.has_room(), "collected pages make room");
    }

    #[test]
    fn loading_replaces_the_language_and_refused_data_leaves_none() {
        let _turn = one_at_a_time();
        let mut worker = OcrWorker::default();
        worker.load("eng", traineddata("eng")).expect("english");
        worker
            .load("chi_tra", traineddata("chi_tra"))
            .expect("chinese");
        assert_eq!(worker.language().as_deref(), Some("chi_tra"));
        assert!(matches!(
            worker.load("eng", vec![1, 2, 3]),
            Err(OcrError::LanguageData)
        ));
        assert_eq!(worker.language(), None);
        assert!(!worker.has_room());
        worker
            .load("eng", traineddata("eng"))
            .expect("english again");
        assert!(worker.has_room());
    }

    #[test]
    fn stopping_drops_the_pages_waiting_and_the_one_being_read() {
        let _turn = one_at_a_time();
        let mut worker = OcrWorker::default();
        worker.load("eng", traineddata("eng")).expect("english");
        for key in 0..3 {
            assert!(worker.enqueue(job(key, 200)).is_ok());
        }
        worker.stop();
        let started = Instant::now();
        loop {
            let (done, waiting) = worker.take_finished();
            assert!(done.is_empty(), "a stopped page is not reported");
            if waiting == 0 {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "still {waiting} waiting"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // And the thread takes the next page.
        assert!(worker.enqueue(job(9, 200)).is_ok());
        let done = finished(&worker, 1);
        assert_eq!(done.len(), 1);
        assert!(done[0].outcome.is_ok());
    }

    #[test]
    fn a_page_that_takes_too_long_is_reported_as_stopped() {
        let _turn = one_at_a_time();
        let mut worker = OcrWorker::default();
        worker.load("eng", traineddata("eng")).expect("english");
        let mut quick = job(1, 200);
        quick.time = Duration::from_millis(1);
        assert!(worker.enqueue(quick).is_ok());
        let done = finished(&worker, 1);
        assert!(matches!(done[0].outcome, Err(OcrError::Stopped)));
    }
}
