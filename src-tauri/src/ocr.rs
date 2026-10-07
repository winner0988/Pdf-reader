//! Recognising the text of scanned pages (B2-10, ADR 0015, docs/architecture/ocr.md): which
//! documents' pages are looked at and read, in what order, and what the page is told about it.
//!
//! Each tab has a [`Session`]. A single thread steps them (see [`Ocr::start`]): it never waits for
//! a document that is busy with a request of the user's, asks the worker only a few quick things
//! at a time, and polls for what the worker's recognising thread has finished. The worker does
//! the reading in the background, so renders of the same document go on; here it is only decided
//! what to ask for, and the answers are counted.
//!
//! A tab that has the document's pages looked at again after every edit or undo (its `info.doc`
//! changes): the worker keeps what it read through edits, and answers at once for pages it knows,
//! so this costs a request a page. A new worker (the old one was lost) starts again with its
//! language.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ipc_contract::types::{
    DocumentId, ErrorCode, IpcError, OcrProgress, OcrRun, OpenEvent, Settings, TabId,
};
use ipc_contract::worker::{OcrFinished, OcrOutcome, OcrPageState};

use crate::ocr_languages::{Languages, ReadError};

/// How long one page may take to be read, at most (the worker stops it then).
pub const PAGE_MILLIS: u32 = 120_000;

/// A page that has been waiting this long for a result, with none coming from any page, means
/// the worker is stuck: it is stopped (`PAGE_MILLIS` and a minute and a half more).
const STALL: Duration = Duration::from_millis(PAGE_MILLIS as u64 + 90_000);

/// Between steps.
const TICK: Duration = Duration::from_millis(150);

/// How long one step keeps asking for pages: the document's lock is held meanwhile.
const STEP_BUDGET: Duration = Duration::from_millis(60);

/// How many documents are asked for new pages at once; the others wait their turn.
const MAX_ACTIVE: usize = 2;

/// After no language was found, when to look again (one may have been imported).
const NO_LANGUAGE_RETRY: Duration = Duration::from_secs(2);

/// What a session needs of a tab's document and its worker; the document's lock is held while it
/// is used. A trait so that the walk over the pages can be tested without a worker.
pub trait OcrLink {
    /// The document as the page knows it: changes with every edit.
    fn frontend_doc(&self) -> DocumentId;
    /// The document in its worker: changes when the worker is replaced.
    fn worker_doc(&self) -> DocumentId;
    fn page_count(&self) -> u32;
    /// The worker was lost; it is replaced by the user's next request, not by this.
    fn is_lost(&self) -> bool;
    fn load_language(&mut self, language: &str, data: Vec<u8>) -> Result<(), IpcError>;
    /// Looks at a page and queues it if it is a scan (at most [`PAGE_MILLIS`] for it).
    fn check_page(&mut self, page_index: u32) -> Result<OcrPageState, IpcError>;
    /// The pages finished since the last poll, and how many are waiting or being read.
    fn poll(&mut self) -> Result<(Vec<OcrFinished>, u32), IpcError>;
    fn stop(&mut self) -> Result<(), IpcError>;
    /// The worker does not answer as it should: it is stopped, and the document is opened again
    /// in a new one by the user's next request.
    fn give_up(&mut self);
}

/// Whether `error` means the worker can no longer be used.
fn worker_lost(error: &IpcError) -> bool {
    matches!(
        error.code,
        ErrorCode::WorkerCrashed | ErrorCode::WorkerTimeout | ErrorCode::ProtocolViolation
    )
}

/// What a step may use besides the document.
pub struct Context<'a> {
    pub settings: &'a Settings,
    pub languages: &'a Languages,
    /// Whether this session may ask for new pages: only so many documents are read at once.
    pub may_start: bool,
    pub now: Instant,
}

/// The work on one tab: its document's pages are looked at, and the scanned ones read.
pub struct Session {
    run: OcrRun,
    /// The user asked for it, so that the setting that leaves it to them does not hold it back.
    manual: bool,
    /// The user stopped it: the worker is told at the next step.
    needs_stop: bool,
    /// The language the worker was given.
    language: Option<String>,
    /// The document in the worker, and the page's document, that `asked` is about.
    worker: Option<DocumentId>,
    layout: Option<DocumentId>,
    /// The worker has the language's data.
    loaded: bool,
    /// Which pages the worker has been asked about, or has nothing more to say about.
    asked: Vec<bool>,
    remaining: u32,
    /// Where the walk goes on from: the page the user looks at, then the ones after it.
    cursor: u32,
    focus: u32,
    /// Pages the worker has queued for this session, until it reports them.
    queued: HashSet<u32>,
    busy_since: Option<Instant>,
    quiet_polls: u32,
    scans: u32,
    recognised: u32,
    failed: u32,
    retry_at: Option<Instant>,
    /// What the page was last told.
    sent: Option<OcrProgress>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            run: OcrRun::Idle,
            manual: false,
            needs_stop: false,
            language: None,
            worker: None,
            layout: None,
            loaded: false,
            asked: Vec::new(),
            remaining: 0,
            cursor: 0,
            focus: 0,
            queued: HashSet::new(),
            busy_since: None,
            quiet_polls: 0,
            scans: 0,
            recognised: 0,
            failed: 0,
            retry_at: None,
            sent: None,
        }
    }

    pub fn run(&self) -> OcrRun {
        self.run
    }

    /// What the page was last told (for a page that subscribes later).
    pub fn sent(&self) -> Option<OcrProgress> {
        self.sent
    }

    /// The user asks to recognise the document's text, whatever the settings say.
    pub fn start(&mut self) {
        self.manual = true;
        self.needs_stop = false;
        self.run = OcrRun::Running;
        self.retry_at = None;
        self.layout = None;
    }

    /// The user stops it; what was read stays.
    pub fn stop(&mut self) {
        if self.run == OcrRun::Running {
            self.run = OcrRun::Stopped;
            self.needs_stop = true;
        }
    }

    /// The page the user looks at: it is read first.
    pub fn set_focus(&mut self, page_index: u32) {
        self.focus = page_index;
        self.cursor = page_index;
    }

    /// A session that has nothing to do until the user does something.
    pub fn is_dormant(&self) -> bool {
        matches!(self.run, OcrRun::Stopped | OcrRun::Failed) && !self.needs_stop
    }

    /// Whether it has pages to ask the worker about, so that it counts as one of the documents
    /// being read.
    pub fn is_reading(&self) -> bool {
        self.run == OcrRun::Running && (self.remaining > 0 || !self.queued.is_empty())
    }

    fn reset_walk(&mut self, pages: u32) {
        self.asked = vec![false; pages as usize];
        self.remaining = pages;
        self.cursor = self.focus.min(pages.saturating_sub(1));
        self.queued.clear();
        self.busy_since = None;
        self.quiet_polls = 0;
        self.scans = 0;
        self.recognised = 0;
        self.failed = 0;
    }

    /// The next page to ask about, from the cursor on and round to the start.
    fn next_page(&mut self) -> Option<u32> {
        if self.remaining == 0 {
            return None;
        }
        let pages = self.asked.len();
        let start = self.cursor as usize % pages.max(1);
        (0..pages)
            .map(|offset| (start + offset) % pages)
            .find(|&page| !self.asked[page])
            .map(|page| {
                self.cursor = u32::try_from(page).unwrap_or(0);
                self.cursor
            })
    }

    fn mark_asked(&mut self, page: u32) {
        if let Some(asked) = self.asked.get_mut(page as usize)
            && !*asked
        {
            *asked = true;
            self.remaining -= 1;
        }
    }

    fn progress(&self, link: &dyn OcrLink) -> OcrProgress {
        let pages = link.page_count();
        let walked = u32::try_from(self.asked.len()).unwrap_or(u32::MAX);
        OcrProgress {
            doc: link.frontend_doc(),
            run: self.run,
            pages,
            checked: walked.saturating_sub(self.remaining).min(pages),
            scans: self.scans,
            recognised: self.recognised,
            failed: self.failed,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// One step of the work on `tab`'s document, whose lock is held: what is to be told to the
    /// page afterwards. Quick: a poll, and a few pages looked at.
    pub fn step(&mut self, tab: TabId, link: &mut dyn OcrLink, ctx: &Context) -> Vec<OpenEvent> {
        let mut events = Vec::new();
        self.work(tab, link, ctx, &mut events);
        let progress = self.progress(link);
        if self.sent != Some(progress) {
            self.sent = Some(progress);
            events.push(OpenEvent::Ocr { tab, progress });
        }
        events
    }

    fn work(
        &mut self,
        tab: TabId,
        link: &mut dyn OcrLink,
        ctx: &Context,
        events: &mut Vec<OpenEvent>,
    ) {
        if link.is_lost() {
            return;
        }
        if self.needs_stop {
            self.needs_stop = false;
            self.queued.clear();
            if link.stop().is_err_and(|error| worker_lost(&error)) {
                self.run = OcrRun::Failed;
            }
            return;
        }
        let (doc, worker) = (link.frontend_doc(), link.worker_doc());
        match self.run {
            OcrRun::Stopped | OcrRun::Failed => return,
            OcrRun::Idle | OcrRun::NoLanguage => {
                if self.retry_at.is_some_and(|at| ctx.now < at)
                    || !(ctx.settings.ocr_auto || self.manual)
                {
                    return;
                }
                self.run = OcrRun::Running;
                self.layout = None;
            }
            OcrRun::Done => {
                // An edit or a new worker may leave pages to read (merged pages, a lost result).
                let changed = self.layout != Some(doc) || self.worker != Some(worker);
                if !changed || !(ctx.settings.ocr_auto || self.manual) {
                    return;
                }
                self.run = OcrRun::Running;
            }
            OcrRun::Running => {}
        }
        if self.worker != Some(worker) {
            self.worker = Some(worker);
            self.loaded = false;
            self.layout = None;
        }
        if self.layout != Some(doc) {
            self.layout = Some(doc);
            self.reset_walk(link.page_count());
        }
        // Only so many documents are read at once: this one waits its turn, without loading a
        // language into its worker before it has one.
        if !ctx.may_start && self.queued.is_empty() {
            return;
        }
        if !self.ensure_language(link, ctx) {
            return;
        }
        if !self.queued.is_empty() {
            self.collect(tab, link, ctx.now, events);
        }
        if self.run == OcrRun::Running && ctx.may_start {
            self.ask(link);
        }
        if self.run == OcrRun::Running && self.remaining == 0 && self.queued.is_empty() {
            self.run = OcrRun::Done;
        }
    }

    /// Has the worker load the language the settings (or the app) choose, unless it has it.
    /// False if the session cannot go on.
    fn ensure_language(&mut self, link: &mut dyn OcrLink, ctx: &Context) -> bool {
        let Some(code) = ctx.languages.choose(ctx.settings.ocr_language.as_deref()) else {
            self.run = OcrRun::NoLanguage;
            self.retry_at = Some(ctx.now + NO_LANGUAGE_RETRY);
            return false;
        };
        if self.language.as_deref() != Some(code.as_str()) {
            self.language = Some(code.clone());
            self.loaded = false;
        }
        if self.loaded {
            return true;
        }
        let data = match ctx.languages.read(&code) {
            Ok(data) => data,
            Err(ReadError::Missing) => {
                self.run = OcrRun::NoLanguage;
                self.retry_at = Some(ctx.now + NO_LANGUAGE_RETRY);
                return false;
            }
            Err(_) => {
                self.run = OcrRun::Failed;
                return false;
            }
        };
        if link.load_language(&code, data).is_err() {
            self.run = OcrRun::Failed;
            return false;
        }
        // The worker dropped the pages that were waiting.
        self.loaded = true;
        self.reset_walk(link.page_count());
        true
    }
}

impl Session {
    /// Takes the pages the worker finished since the last poll: the page is told about each, and
    /// those this session queued are counted.
    fn collect(
        &mut self,
        tab: TabId,
        link: &mut dyn OcrLink,
        now: Instant,
        events: &mut Vec<OpenEvent>,
    ) {
        let (doc, worker) = (link.frontend_doc(), link.worker_doc());
        let (finished, waiting) = match link.poll() {
            Ok(polled) => polled,
            Err(error) => {
                if worker_lost(&error) {
                    self.run = OcrRun::Failed;
                    self.queued.clear();
                }
                return;
            }
        };
        if finished.is_empty() {
            self.quiet_polls += 1;
        } else {
            self.quiet_polls = 0;
            self.busy_since = Some(now);
        }
        for page in finished {
            if page.doc != worker {
                continue;
            }
            events.push(OpenEvent::OcrPage {
                tab,
                doc,
                page_index: page.page_index,
            });
            if self.queued.remove(&page.page_index) {
                match page.outcome {
                    OcrOutcome::Recognised { .. } => self.recognised += 1,
                    OcrOutcome::TimedOut | OcrOutcome::Failed => self.failed += 1,
                }
            }
        }
        if self.queued.is_empty() {
            self.busy_since = None;
        } else if waiting == 0 && self.quiet_polls >= 3 {
            // The worker has nothing for us (its queue was dropped): ask about every page again.
            self.reset_walk(link.page_count());
        } else if self
            .busy_since
            .is_some_and(|since| now.duration_since(since) > STALL)
        {
            link.give_up();
            self.queued.clear();
            self.run = OcrRun::Failed;
        }
    }

    /// Asks the worker about pages, from the cursor on, until it has as many as it takes, or this
    /// step has had its time.
    fn ask(&mut self, link: &mut dyn OcrLink) {
        let started = Instant::now();
        while started.elapsed() < STEP_BUDGET {
            let Some(page) = self.next_page() else {
                break;
            };
            match link.check_page(page) {
                Ok(OcrPageState::NotScan) => self.mark_asked(page),
                Ok(OcrPageState::Recognised) => {
                    self.mark_asked(page);
                    self.scans += 1;
                    self.recognised += 1;
                }
                Ok(OcrPageState::Failed) => {
                    self.mark_asked(page);
                    self.scans += 1;
                    self.failed += 1;
                }
                Ok(OcrPageState::Queued) => {
                    self.mark_asked(page);
                    self.scans += 1;
                    if self.queued.is_empty() {
                        self.busy_since = Some(Instant::now());
                    }
                    self.queued.insert(page);
                }
                // As many as the worker takes: the same page is asked about again at the next step.
                Ok(OcrPageState::Full) => break,
                Ok(OcrPageState::NoLanguage) => {
                    self.loaded = false;
                    break;
                }
                Err(error) if worker_lost(&error) => {
                    self.run = OcrRun::Failed;
                    break;
                }
                // A page that could not even be looked at: one that could not be read.
                Err(_) => {
                    self.mark_asked(page);
                    self.scans += 1;
                    self.failed += 1;
                }
            }
        }
    }
}

/// The tabs, and a way into their documents: what [`Ocr`] steps over.
pub trait Tabs {
    /// Every tab; the one the window shows first, so that it is read first.
    fn tabs(&self) -> Vec<TabId>;
    /// Runs `step` on the tab's open document if that can be had at once: not if it is busy with
    /// a request, or not open. Whether it ran.
    fn with_link(&self, tab: TabId, step: &mut dyn FnMut(&mut dyn OcrLink)) -> bool;
}

/// The sessions of all tabs, stepped by one thread.
pub struct Ocr {
    languages: Languages,
    sessions: Mutex<HashMap<TabId, Arc<Mutex<Session>>>>,
    /// Set when something wants the next step now.
    wake: (Mutex<bool>, Condvar),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Ocr {
    pub fn new(languages: Languages) -> Self {
        Self {
            languages,
            sessions: Mutex::new(HashMap::new()),
            wake: (Mutex::new(false), Condvar::new()),
        }
    }

    pub fn languages(&self) -> &Languages {
        &self.languages
    }

    fn session(&self, tab: TabId) -> Arc<Mutex<Session>> {
        Arc::clone(lock(&self.sessions).entry(tab).or_default())
    }

    /// The user asks to recognise the text of `tab`'s document.
    pub fn start_tab(&self, tab: TabId) {
        lock(&self.session(tab)).start();
        self.wake();
    }

    /// The user stops it.
    pub fn stop_tab(&self, tab: TabId) {
        lock(&self.session(tab)).stop();
        self.wake();
    }

    /// The page of `tab`'s document that the user looks at.
    pub fn set_focus(&self, tab: TabId, page_index: u32) {
        lock(&self.session(tab)).set_focus(page_index);
    }

    /// Where recognising is, in every tab that has been told: for a page that subscribes later.
    pub fn snapshot(&self) -> Vec<OpenEvent> {
        let sessions = lock(&self.sessions);
        let mut events: Vec<(TabId, OpenEvent)> = sessions
            .iter()
            .filter_map(|(&tab, session)| {
                let progress = lock(session).sent()?;
                Some((tab, OpenEvent::Ocr { tab, progress }))
            })
            .collect();
        events.sort_by_key(|(tab, _)| tab.0);
        events.into_iter().map(|(_, event)| event).collect()
    }

    fn wake(&self) {
        *lock(&self.wake.0) = true;
        self.wake.1.notify_all();
    }

    /// Waits for the next step: `TICK`, or less if something asked for it.
    fn rest(&self) {
        let mut woken = lock(&self.wake.0);
        if !*woken {
            woken = self
                .wake
                .1
                .wait_timeout(woken, TICK)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
        *woken = false;
    }

    /// Steps every tab's session once, in the order of `tabs`; what is to be told to the page
    /// goes to `send`.
    pub fn tick(
        &self,
        tabs: &dyn Tabs,
        settings: &Settings,
        now: Instant,
        send: &mut dyn FnMut(OpenEvent),
    ) {
        let order = tabs.tabs();
        lock(&self.sessions).retain(|tab, _| order.contains(tab));
        let mut reading = 0;
        for tab in order {
            let session = self.session(tab);
            let mut session = lock(&session);
            if session.is_dormant() {
                continue;
            }
            let context = Context {
                settings,
                languages: &self.languages,
                may_start: reading < MAX_ACTIVE,
                now,
            };
            let mut events = Vec::new();
            tabs.with_link(tab, &mut |link| {
                events = session.step(tab, link, &context);
            });
            if session.is_reading() {
                reading += 1;
            }
            drop(session);
            events.into_iter().for_each(&mut *send);
        }
    }

    /// Starts the thread that steps the sessions, until the app ends. `tabs` is the app's
    /// documents; `settings` says what to do on its own; events go to `send`.
    pub fn start(
        this: Arc<Self>,
        tabs: impl Tabs + Send + 'static,
        settings: impl Fn() -> Settings + Send + 'static,
        send: impl Fn(OpenEvent) + Send + 'static,
    ) {
        let spawned = std::thread::Builder::new()
            .name("ocr scheduler".to_owned())
            .spawn(move || {
                loop {
                    this.tick(&tabs, &settings(), Instant::now(), &mut |event| send(event));
                    this.rest();
                }
            });
        // Without the thread nothing is read, which the page sees as no progress at all.
        drop(spawned);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ipc_contract::validate::Validate;

    use super::*;
    use crate::ocr_languages::tests::{folder, language_data};

    const TAB: TabId = TabId(1);

    /// A document and its worker, as far as the session can tell, and what it was asked.
    struct Fake {
        frontend: DocumentId,
        worker: DocumentId,
        pages: u32,
        lost: bool,
        /// What `check_page` says, by page; otherwise that it is no scan.
        answers: HashMap<u32, OcrPageState>,
        /// How many pages the worker takes before it is full.
        room: usize,
        queued: usize,
        asked: Vec<u32>,
        loaded: Vec<String>,
        stopped: u32,
        gave_up: bool,
        /// Given at the next poll.
        finished: Vec<OcrFinished>,
        waiting: u32,
        check_error: Option<IpcError>,
        load_error: Option<IpcError>,
    }

    impl Fake {
        fn new(pages: u32) -> Self {
            Self {
                frontend: DocumentId(10),
                worker: DocumentId(1),
                pages,
                lost: false,
                answers: HashMap::new(),
                room: usize::MAX,
                queued: 0,
                asked: Vec::new(),
                loaded: Vec::new(),
                stopped: 0,
                gave_up: false,
                finished: Vec::new(),
                waiting: 0,
                check_error: None,
                load_error: None,
            }
        }

        fn scans(mut self, pages: &[u32]) -> Self {
            for &page in pages {
                self.answers.insert(page, OcrPageState::Queued);
            }
            self
        }

        fn done(&mut self, page_index: u32, outcome: OcrOutcome) {
            self.finished.push(OcrFinished {
                doc: self.worker,
                page_index,
                outcome,
            });
        }
    }

    impl OcrLink for Fake {
        fn frontend_doc(&self) -> DocumentId {
            self.frontend
        }
        fn worker_doc(&self) -> DocumentId {
            self.worker
        }
        fn page_count(&self) -> u32 {
            self.pages
        }
        fn is_lost(&self) -> bool {
            self.lost
        }
        fn load_language(&mut self, language: &str, _data: Vec<u8>) -> Result<(), IpcError> {
            if let Some(error) = &self.load_error {
                return Err(error.clone());
            }
            self.loaded.push(language.to_owned());
            self.queued = 0;
            Ok(())
        }
        fn check_page(&mut self, page_index: u32) -> Result<OcrPageState, IpcError> {
            if let Some(error) = &self.check_error {
                return Err(error.clone());
            }
            self.asked.push(page_index);
            let answer = self
                .answers
                .get(&page_index)
                .copied()
                .unwrap_or(OcrPageState::NotScan);
            if answer == OcrPageState::Queued {
                if self.queued >= self.room {
                    return Ok(OcrPageState::Full);
                }
                self.queued += 1;
                self.waiting += 1;
            }
            Ok(answer)
        }
        fn poll(&mut self) -> Result<(Vec<OcrFinished>, u32), IpcError> {
            let finished = std::mem::take(&mut self.finished);
            self.queued -= finished.len().min(self.queued);
            self.waiting -= u32::try_from(finished.len()).unwrap().min(self.waiting);
            Ok((finished, self.waiting))
        }
        fn stop(&mut self) -> Result<(), IpcError> {
            self.stopped += 1;
            self.queued = 0;
            self.waiting = 0;
            Ok(())
        }
        fn give_up(&mut self) {
            self.gave_up = true;
            self.lost = true;
        }
    }

    fn error(code: ErrorCode) -> IpcError {
        IpcError {
            code,
            message: String::new(),
        }
    }

    /// A folder with a language installed for each of `codes`.
    fn languages(name: &str, codes: &[&str]) -> (Languages, std::path::PathBuf) {
        let dir = folder(name);
        for code in codes {
            fs::write(dir.join(format!("{code}.traineddata")), language_data(1)).unwrap();
        }
        (Languages::new(Some(dir.clone()), None), dir)
    }

    struct Rig {
        session: Session,
        settings: Settings,
        languages: Languages,
        dir: std::path::PathBuf,
        now: Instant,
        events: Vec<OpenEvent>,
    }

    impl Rig {
        fn new(name: &str, codes: &[&str]) -> Self {
            let (languages, dir) = languages(name, codes);
            Self {
                session: Session::new(),
                settings: Settings::default(),
                languages,
                dir,
                now: Instant::now(),
                events: Vec::new(),
            }
        }

        fn step(&mut self, link: &mut Fake) {
            let context = Context {
                settings: &self.settings,
                languages: &self.languages,
                may_start: true,
                now: self.now,
            };
            let events = self.session.step(TAB, link, &context);
            for event in &events {
                assert_eq!(event.validate(), Ok(()), "{event:?}");
            }
            self.events.extend(events);
        }

        fn progress(&self) -> OcrProgress {
            self.events
                .iter()
                .rev()
                .find_map(|event| match event {
                    OpenEvent::Ocr { progress, .. } => Some(*progress),
                    _ => None,
                })
                .expect("a progress event")
        }

        fn pages_told(&self) -> Vec<u32> {
            self.events
                .iter()
                .filter_map(|event| match event {
                    OpenEvent::OcrPage { page_index, .. } => Some(*page_index),
                    _ => None,
                })
                .collect()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_document_is_walked_its_scans_read_and_the_page_told_as_they_finish() {
        let mut rig = Rig::new("walk", &["eng"]);
        let mut link = Fake::new(5).scans(&[1, 3]);
        rig.step(&mut link);
        assert_eq!(link.loaded, ["eng"]);
        assert_eq!(link.asked, [0, 1, 2, 3, 4]);
        let progress = rig.progress();
        assert_eq!(
            (
                progress.run,
                progress.pages,
                progress.checked,
                progress.scans,
                progress.recognised
            ),
            (OcrRun::Running, 5, 5, 2, 0)
        );
        assert_eq!(progress.doc, DocumentId(10));

        link.done(3, OcrOutcome::Recognised { chars: 40 });
        rig.step(&mut link);
        assert_eq!(rig.progress().recognised, 1);
        assert_eq!(rig.progress().run, OcrRun::Running);
        link.done(1, OcrOutcome::TimedOut);
        rig.step(&mut link);
        let progress = rig.progress();
        assert_eq!(
            (progress.run, progress.recognised, progress.failed),
            (OcrRun::Done, 1, 1)
        );
        assert_eq!(rig.pages_told(), [3, 1]);
        // Nothing more is asked of a finished document.
        let asked = link.asked.len();
        rig.step(&mut link);
        assert_eq!(link.asked.len(), asked);
    }

    #[test]
    fn a_document_without_scans_is_done_at_once() {
        let mut rig = Rig::new("none", &["eng"]);
        let mut link = Fake::new(3);
        rig.step(&mut link);
        let progress = rig.progress();
        assert_eq!(
            (progress.run, progress.checked, progress.scans),
            (OcrRun::Done, 3, 0)
        );
    }

    #[test]
    fn the_page_the_user_looks_at_is_asked_about_first() {
        let mut rig = Rig::new("focus", &["eng"]);
        let mut link = Fake::new(6).scans(&[0, 2, 4]);
        link.room = 1;
        rig.session.set_focus(4);
        rig.step(&mut link);
        // Room for one page: the page in view takes it, and page 0 is sent away (full).
        assert_eq!(link.asked, [4, 5, 0]);
        link.done(4, OcrOutcome::Recognised { chars: 1 });
        // The user moves on: the next step takes up from the page now in view.
        rig.session.set_focus(2);
        rig.step(&mut link);
        assert_eq!(&link.asked[3..], [2, 3, 0]);
    }

    #[test]
    fn an_edit_has_the_pages_looked_at_again_and_known_ones_cost_a_question() {
        let mut rig = Rig::new("edit", &["eng"]);
        let mut link = Fake::new(4).scans(&[1]);
        rig.step(&mut link);
        link.done(1, OcrOutcome::Recognised { chars: 9 });
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Done);

        // A page is deleted: the document has a new id, and the worker knows what it read.
        link.frontend = DocumentId(11);
        link.pages = 3;
        link.answers.insert(1, OcrPageState::Recognised);
        let before = link.asked.len();
        rig.step(&mut link);
        assert_eq!(link.asked.len() - before, 3);
        assert_eq!(link.loaded, ["eng"], "the worker still has its language");
        let progress = rig.progress();
        assert_eq!(
            (
                progress.run,
                progress.doc,
                progress.pages,
                progress.scans,
                progress.recognised
            ),
            (OcrRun::Done, DocumentId(11), 3, 1, 1)
        );
    }

    #[test]
    fn a_new_worker_is_given_the_language_again_and_starts_over() {
        let mut rig = Rig::new("worker", &["eng"]);
        let mut link = Fake::new(2).scans(&[0]);
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Running);
        // The worker was lost and the user's next request opened the document in a new one.
        link.worker = DocumentId(2);
        link.queued = 0;
        link.waiting = 0;
        rig.step(&mut link);
        assert_eq!(link.loaded, ["eng", "eng"]);
        assert_eq!(link.asked, [0, 1, 0, 1]);
        assert_eq!(rig.progress().scans, 1, "counted again, not added");
    }

    #[test]
    fn a_lost_document_is_left_alone() {
        let mut rig = Rig::new("lost", &["eng"]);
        let mut link = Fake::new(2).scans(&[0]);
        link.lost = true;
        rig.step(&mut link);
        assert!(link.loaded.is_empty() && link.asked.is_empty());
    }

    #[test]
    fn the_settings_can_leave_it_to_the_user_who_can_start_and_stop_it() {
        let mut rig = Rig::new("manual", &["eng"]);
        rig.settings.ocr_auto = false;
        let mut link = Fake::new(3).scans(&[0]);
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Idle);
        assert!(link.loaded.is_empty() && link.asked.is_empty());

        rig.session.start();
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Running);
        assert_eq!(link.asked, [0, 1, 2]);

        rig.session.stop();
        assert_eq!(rig.session.run(), OcrRun::Stopped);
        rig.step(&mut link);
        assert_eq!(link.stopped, 1);
        assert_eq!(rig.progress().run, OcrRun::Stopped);
        assert!(rig.session.is_dormant());
        // A stopped session asks nothing, and the settings do not start it again.
        rig.settings.ocr_auto = true;
        rig.step(&mut link);
        assert_eq!((link.stopped, link.asked.len()), (1, 3));
    }

    #[test]
    fn with_no_language_it_looks_again_a_little_later() {
        let mut rig = Rig::new("nolang", &[]);
        let mut link = Fake::new(2);
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::NoLanguage);
        fs::write(rig.dir.join("deu.traineddata"), language_data(1)).unwrap();
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::NoLanguage, "not at once");
        rig.now += NO_LANGUAGE_RETRY;
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Done);
        assert_eq!(link.loaded, ["deu"]);
    }

    #[test]
    fn the_language_of_the_settings_is_used_if_it_is_installed() {
        let mut rig = Rig::new("choice", &["eng", "chi_tra", "deu"]);
        rig.settings.ocr_language = Some("deu".to_owned());
        let mut link = Fake::new(1);
        rig.step(&mut link);
        assert_eq!(link.loaded, ["deu"]);
        // One that is not installed gives the app's choice.
        let mut rig = Rig::new("choice2", &["eng", "chi_tra"]);
        rig.settings.ocr_language = Some("fra".to_owned());
        let mut link = Fake::new(1);
        rig.step(&mut link);
        assert_eq!(link.loaded, ["chi_tra"]);
    }

    #[test]
    fn what_goes_wrong_ends_the_session_and_only_the_user_starts_it_again() {
        // The worker is lost while pages are looked at.
        let mut rig = Rig::new("fail", &["eng"]);
        let mut link = Fake::new(2).scans(&[0]);
        link.check_error = Some(error(ErrorCode::WorkerCrashed));
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Failed);
        assert!(rig.session.is_dormant());
        link.check_error = None;
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Failed, "no crash loop");
        rig.session.start();
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Running);

        // The worker refuses the language data.
        let mut rig = Rig::new("fail2", &["eng"]);
        let mut link = Fake::new(1);
        link.load_error = Some(error(ErrorCode::InvalidArgument));
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Failed);

        // Data on disk that is not language data.
        let mut rig = Rig::new("fail3", &[]);
        fs::write(rig.dir.join("eng.traineddata"), b"not language data").unwrap();
        let mut link = Fake::new(1);
        rig.step(&mut link);
        assert_eq!(rig.progress().run, OcrRun::Failed);
        assert!(link.loaded.is_empty());
    }

    #[test]
    fn a_page_that_cannot_be_looked_at_counts_as_one_that_could_not_be_read() {
        let mut rig = Rig::new("bad-page", &["eng"]);
        let mut link = Fake::new(3);
        link.check_error = Some(error(ErrorCode::Corrupted));
        rig.step(&mut link);
        let progress = rig.progress();
        assert_eq!(
            (progress.run, progress.scans, progress.failed),
            (OcrRun::Done, 3, 3)
        );
        assert_eq!(progress.validate(), Ok(()));
    }

    #[test]
    fn a_worker_that_lost_its_queue_is_asked_again_and_a_stuck_one_is_given_up_on() {
        let mut rig = Rig::new("queue", &["eng"]);
        let mut link = Fake::new(2).scans(&[0]);
        rig.step(&mut link);
        // The worker says nothing is waiting, and no result has come: its queue was dropped.
        link.waiting = 0;
        link.queued = 0;
        for _ in 0..3 {
            rig.step(&mut link);
        }
        assert_eq!(link.asked, [0, 1, 0, 1], "asked about every page again");

        // Pages wait for ever.
        let mut rig = Rig::new("stuck", &["eng"]);
        let mut link = Fake::new(2).scans(&[0]);
        rig.step(&mut link);
        rig.now += STALL / 2;
        rig.step(&mut link);
        assert!(!link.gave_up);
        rig.now += STALL;
        rig.step(&mut link);
        assert!(link.gave_up);
        assert_eq!(rig.progress().run, OcrRun::Failed);
    }

    #[test]
    fn results_of_other_documents_and_unqueued_pages_are_not_counted() {
        let mut rig = Rig::new("stale", &["eng"]);
        let mut link = Fake::new(3).scans(&[1]);
        rig.step(&mut link);
        link.finished.push(OcrFinished {
            doc: DocumentId(99),
            page_index: 1,
            outcome: OcrOutcome::Recognised { chars: 1 },
        });
        link.done(2, OcrOutcome::Recognised { chars: 1 });
        rig.step(&mut link);
        assert_eq!(rig.progress().recognised, 0);
        assert_eq!(
            rig.pages_told(),
            [2],
            "the page is told, though it was not one of ours"
        );
        assert_eq!(rig.progress().validate(), Ok(()));
    }

    /// Tabs with a fake document each, stepped by `Ocr::tick`.
    struct FakeTabs {
        links: Mutex<HashMap<TabId, Fake>>,
        order: Vec<TabId>,
        busy: HashSet<TabId>,
    }

    impl Tabs for FakeTabs {
        fn tabs(&self) -> Vec<TabId> {
            self.order.clone()
        }

        fn with_link(&self, tab: TabId, step: &mut dyn FnMut(&mut dyn OcrLink)) -> bool {
            if self.busy.contains(&tab) {
                return false;
            }
            let mut links = lock(&self.links);
            step(links.get_mut(&tab).expect("a fake document"));
            true
        }
    }

    #[test]
    fn only_so_many_documents_are_read_at_once_the_busy_ones_are_skipped_and_closed_tabs_forgotten()
    {
        let (languages, dir) = languages("service", &["eng"]);
        let ocr = Ocr::new(languages);
        let tab = |n| TabId(n);
        let mut links = HashMap::new();
        for n in 1..=4 {
            let mut fake = Fake::new(3).scans(&[0, 1, 2]);
            fake.room = 1;
            links.insert(tab(n), fake);
        }
        let mut tabs = FakeTabs {
            links: Mutex::new(links),
            order: vec![tab(1), tab(2), tab(3), tab(4)],
            busy: HashSet::from([tab(2)]),
        };
        let settings = Settings::default();
        let mut sent = Vec::new();
        ocr.tick(&tabs, &settings, Instant::now(), &mut |event| {
            sent.push(event)
        });
        let asked = |tabs: &FakeTabs, n| lock(&tabs.links)[&tab(n)].asked.clone();
        // The first reads; the second is busy with the user's own request and is not waited for;
        // so the third has a turn too; the fourth waits.
        assert_eq!(asked(&tabs, 1), [0, 1]);
        assert_eq!(asked(&tabs, 2), Vec::<u32>::new());
        assert_eq!(asked(&tabs, 3), [0, 1]);
        assert_eq!(asked(&tabs, 4), Vec::<u32>::new());
        assert!(
            lock(&tabs.links)[&tab(4)].loaded.is_empty(),
            "no language for a document that waits"
        );
        assert!(sent.iter().all(|event| event.validate().is_ok()));

        // Where recognising is, for a page that subscribes now.
        let told: Vec<TabId> = ocr
            .snapshot()
            .into_iter()
            .filter_map(|event| match event {
                OpenEvent::Ocr { tab, .. } => Some(tab),
                _ => None,
            })
            .collect();
        assert!(told.contains(&tab(1)) && told.contains(&tab(3)) && told.contains(&tab(4)));

        // The tabs are closed but one: the others are forgotten.
        tabs.order = vec![tab(3)];
        ocr.tick(&tabs, &settings, Instant::now(), &mut |_| {});
        let told: Vec<TabId> = ocr
            .snapshot()
            .into_iter()
            .filter_map(|event| match event {
                OpenEvent::Ocr { tab, .. } => Some(tab),
                _ => None,
            })
            .collect();
        assert_eq!(told, [tab(3)]);
        let _ = fs::remove_dir_all(dir);
    }
}
