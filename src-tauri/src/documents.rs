//! The window's documents (MVP-06, MVP-14): every file the user opens gets a tab, and every open
//! document its own sandboxed worker (ADR 0012), so a hostile PDF that takes over its worker
//! cannot reach the other documents. Paths stay in the main process (ADR 0008): the frontend
//! only ever sees tab and document ids and file names.
//!
//! Edits (ADR 0013) change a document in its worker's memory; `save` writes it to a file
//! (src/saving.rs). Until then the main process keeps the edits, to apply them again should the
//! worker have to be restarted, and a journal of them in the app's data folder, to make them again
//! should the app end first (B2-13, src/recovery.rs).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};

use ipc_contract::limits::{
    MAX_CREATE_SOURCES, MAX_DISPLAY_NAME_BYTES, MAX_PAGE_COUNT, MAX_SOURCE_BYTES,
    MAX_STAMP_SOURCE_BYTES, MAX_TABS, MAX_TEXT_BYTES, MAX_UNDO_EDITS,
};
use ipc_contract::raster::{encode_raster, fit_scale};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{
    AnnotationId, BatchOperation, BlockedAction, CompressResult, CreateKind, CreateResult,
    DocumentId, DocumentInfo, DocumentPermissions, Edit, EditArgs, ErrorCode, FormField,
    ImageQuality, IpcError, LinkArgs, LinkPreview, LinkTarget, MarkCheck, NotRemembered, OpenEvent,
    OutlineLinkArgs, OutlineResult, PageAnnotation, PageLink, PageSize, PageText, PagesSource,
    Password, PreparedTextBox, Recovery, RedactionSummary, RememberedPasswords, RenderPageArgs,
    Restrictions, SaveResult, SearchHit, SecurityReport, SignatureReport, SourceId, StampImageInfo,
    TabId, TextBoxInfo, UnlockOutcome,
};
use ipc_contract::validate::{Validate, check_page_index, check_text_box_text, stamp_png_size};
use ipc_contract::worker::{
    FontData, InspectedFont, OcrFinished, OcrPageState, PasswordKey, UnknownFile, WorkerEdit,
    WorkerErrorCode, WorkerRequest, WorkerResponse,
};
use worker_host::{HostConfig, HostError, MAX_DOCUMENT_BYTES, WorkerHost};

use crate::credentials::{CredentialError, Credentials};
use crate::export::ImageKind;
use crate::history::History;
use crate::ocr::{OcrLink, PAGE_MILLIS, Tabs};
use crate::pictures::{Full, Pictures};
use crate::recovery::{Found, JournalId, Journals, Named, TooLarge};
use crate::saving::{self, FileIdentity, Temporary};
use crate::sources::{Full as SourcesFull, Source, Sources};
use crate::text_boxes::{Full as TextBoxesFull, TextBoxes};

/// Display name used when a path has no file name component.
const FALLBACK_NAME: &str = "PDF";

/// The highest number a batch copy's name gets (`name (999).pdf`) before it gives up (B3-04).
const MAX_COPY_NUMBER: u32 = 999;

pub struct Documents {
    worker: PathBuf,
    /// For tab and document ids; never reused while the app runs.
    next_id: AtomicU32,
    inner: Mutex<Inner>,
    /// Where events go that no command is waiting for: a document whose worker died and that
    /// needs its password again (MVP-16). Set once at startup.
    reporter: OnceLock<Box<dyn Fn(OpenEvent) + Send + Sync>>,
    /// The crash recovery journals (B2-13). Set once at startup, when the data folder is known;
    /// without them nothing is kept.
    journals: OnceLock<Journals>,
    /// The passwords the user asked the app to remember (B2-25, ADR 0017). Set once at startup;
    /// without it nothing is remembered or recalled.
    credentials: OnceLock<Credentials>,
}

struct Inner {
    /// In the order the tabs were added.
    tabs: Vec<Arc<Tab>>,
    /// The tab the window shows, for the window title.
    active: Option<TabId>,
}

/// One tab. Locks are always taken in the order `Documents::inner`, `Tab::event`,
/// `Tab::document`, and `inner` is never held during a worker request. `path` and
/// `display_name` are only ever locked on their own.
struct Tab {
    id: TabId,
    /// Never leaves the main process. Changes when the document is saved as another file.
    path: Mutex<PathBuf>,
    display_name: Mutex<String>,
    /// What the frontend was last told about this tab (`Opening`, `Opened` or `Failed`). Readable
    /// while `document` is busy with a long worker request.
    event: Mutex<OpenEvent>,
    document: Mutex<Option<OpenDocument>>,
    /// The recovery journal of a document this tab lost with its password (B2-13): its edits are
    /// made again once the password opens the file again. Only ever locked on its own.
    released: Mutex<Option<JournalId>>,
    /// What is known of the file's password: its key, and whether the app remembers one (B2-25).
    /// Only ever locked on its own.
    password: Mutex<PasswordFacts>,
}

/// What the main process knows of the password of an encrypted file (B2-25, ADR 0017).
#[derive(Debug, Clone, Copy, Default)]
struct PasswordFacts {
    /// Where a remembered password is looked up, as the file's own worker reported it. The
    /// worker is not trusted to give the right one: it is a name, and the password only ever goes
    /// back to that worker.
    key: Option<PasswordKey>,
    /// The app has a password for the key (it may not be the right one).
    record: bool,
}

/// The password of an attempt to open a file (B2-25).
enum Given {
    /// None: the file may ask for one, and a remembered one is tried.
    Nothing,
    /// The user typed it, and may have asked for it to be remembered once it opens the file.
    Typed { password: Password, remember: bool },
    /// The main process took it from the Credential Manager, for a file whose worker died.
    Remembered(Password),
}

/// Hits on one page, and whether the page has any text at all.
#[derive(Debug)]
pub struct PageFound {
    pub hits: Vec<SearchHit>,
    pub has_text: bool,
}

struct OpenDocument {
    /// What the frontend knows. `info.doc` is assigned here, unique among all documents, and
    /// stays the same for the document's lifetime.
    info: DocumentInfo,
    /// This document's own worker (ADR 0012).
    host: WorkerHost,
    path: PathBuf,
    /// The document's id in its worker process. Changes when the worker is restarted and the
    /// file reopened.
    worker_doc: DocumentId,
    /// Opened with a password (MVP-16). The password is not kept, so if the worker dies the
    /// document cannot be reopened: the tab asks for the password again.
    protected: bool,
    /// The worker crashed, timed out or misbehaved; the file is reopened in a fresh worker
    /// before the next request.
    lost: bool,
    /// The file as it was when read or last written, to notice another program changing it.
    identity: Option<FileIdentity>,
    /// Edits since the file was read or last written, applied and undone (B2-05).
    /// `info.unsaved` while some are applied.
    history: History,
    /// The file has a form (B2-09); `info.has_form` is that, unless the form is flattened.
    form_in_file: bool,
    /// The crash recovery journal of the edits applied (B2-13): there while some are.
    journal: Option<JournalId>,
    /// Edits an earlier run left for the file (B2-13), until the user makes them again or
    /// discards them. `info.recovery` says whether they can be made.
    recovered: Option<Found>,
    /// The files whose pages the history took in (B2-06).
    sources: Sources,
    /// What the document's own file has that is active, as the worker said when it opened; the
    /// banner adds what the files taken from have (`show_history`).
    security_in_file: SecurityReport,
    /// The encrypted file the user chose to take pages from, while the password is asked for
    /// (B2-06): the path never leaves the main process.
    pending_source: Option<PathBuf>,
    /// The pictures of the picture stamps of the history (B2-08).
    pictures: Pictures,
    /// The prepared words of the text boxes of the history (B2-29).
    text_boxes: TextBoxes,
    /// What is known of the file's password (B2-25); the key only for a file opened with one.
    password: PasswordFacts,
    /// The file is signed, as the worker said when it opened it: the sensitive mark is not
    /// written into it (B3-05, ADR 0010).
    signed: bool,
    /// The app's document identifier in the file (ADR 0010): the file is marked as sensitive
    /// (B3-05), and `info.sensitive` says so.
    identifier: Option<[u8; 16]>,
}

impl Documents {
    pub fn new(worker: PathBuf) -> Self {
        Self {
            worker,
            next_id: AtomicU32::new(1),
            inner: Mutex::new(Inner {
                tabs: Vec::new(),
                active: None,
            }),
            reporter: OnceLock::new(),
            journals: OnceLock::new(),
            credentials: OnceLock::new(),
        }
    }

    /// Where events go that no command is waiting for (see the field).
    pub fn set_reporter(&self, reporter: impl Fn(OpenEvent) + Send + Sync + 'static) {
        let _ = self.reporter.set(Box::new(reporter));
    }

    /// Where the crash recovery journals go (see the field).
    pub fn set_journals(&self, journals: Journals) {
        let _ = self.journals.set(journals);
    }

    /// Where the remembered passwords are kept (see the field).
    pub fn set_credentials(&self, credentials: Credentials) {
        let _ = self.credentials.set(credentials);
    }

    /// Adds a tab for each of `paths`, in order, and reports `Opening` for each. Files beyond
    /// `MAX_TABS` tabs are not opened and reported once, as `TabLimit`. Each returned tab still
    /// has to be loaded with [`Self::load`].
    pub fn add(&self, paths: &[PathBuf], report: &dyn Fn(OpenEvent)) -> Vec<TabId> {
        let mut opening = Vec::new();
        {
            let mut inner = self.lock();
            for path in paths {
                if inner.tabs.len() >= MAX_TABS as usize {
                    break;
                }
                let id = TabId(self.next_id());
                let display_name = display_name(path);
                let event = OpenEvent::Opening {
                    tab: id,
                    display_name: display_name.clone(),
                };
                inner.tabs.push(Arc::new(Tab {
                    id,
                    path: Mutex::new(path.clone()),
                    display_name: Mutex::new(display_name),
                    event: Mutex::new(event.clone()),
                    document: Mutex::new(None),
                    released: Mutex::new(None),
                    password: Mutex::new(PasswordFacts::default()),
                }));
                opening.push((id, event));
            }
        }
        let ignored = paths.len() - opening.len();
        let tabs = opening.iter().map(|(id, _)| *id).collect();
        for (_, event) in opening {
            report(event);
        }
        if ignored > 0 {
            report(OpenEvent::TabLimit {
                ignored_files: u32::try_from(ignored).unwrap_or(u32::MAX),
            });
        }
        tabs
    }

    /// Opens the file of `tab` in a new worker of its own and reports `Opened` or `Failed`.
    /// Takes as long as the worker needs; tabs load independently of each other. If the tab is
    /// closed meanwhile, the new worker ends and nothing is reported.
    pub fn load(&self, tab: TabId, report: &dyn Fn(OpenEvent)) {
        self.load_with(tab, Given::Nothing, report);
    }

    /// Opens the tab's file with the password it was `given` (MVP-16). A file that needs a
    /// password, or another one than it got, makes the tab ask for it.
    ///
    /// Given none, a password the app remembers for the file is tried once (B2-25, ADR 0017). It
    /// goes to the worker that reported the file's key, which is this tab's own, and nowhere
    /// else; if it does not open the file the user is asked, and the record is left alone. A
    /// password the user typed and asked to remember is remembered once it has opened the file.
    /// Returns why a password the user asked to remember was not.
    fn load_with(
        &self,
        tab: TabId,
        given: Given,
        report: &dyn Fn(OpenEvent),
    ) -> Option<NotRemembered> {
        let tab = self.tab(tab)?;
        let path = lock(&tab.path).clone();
        let display_name = lock(&tab.display_name).clone();
        // What the tab knows of the file's password: from the first try, or from the document
        // that was lost with its worker.
        let mut facts = *lock(&tab.password);
        let (password, to_remember, from_memory) = match given {
            Given::Nothing => (None, None, false),
            Given::Typed {
                password,
                remember: true,
            } => (Some(password.clone()), Some(password), false),
            Given::Typed {
                password,
                remember: false,
            } => (Some(password), None, false),
            Given::Remembered(password) => (Some(password), None, true),
        };
        let given_password = password.is_some();
        // Some(wrong) when the file needs a password.
        let mut asks = None;
        // The password that was tried came from memory and did not open the file.
        let mut stale = from_memory;
        let mut recalled = from_memory;
        let mut not_remembered = None;
        let result = check_file(&path).and_then(|()| {
            // Before the worker reads it: a change while it opens shows as a change later.
            let identity = FileIdentity::of(&path);
            let mut host = WorkerHost::new(self.worker.clone(), HostConfig::default());
            let mut opened = match password {
                Some(password) => host.open_with_password(&path, password),
                None => host.open(&path),
            };
            // The file asked for a password and the worker said where one is looked up.
            let reported = match &opened {
                Err(HostError::Worker(error))
                    if !given_password && error.code == WorkerErrorCode::Encrypted =>
                {
                    Some(error.password_key)
                }
                _ => None,
            };
            if let Some(key) = reported {
                facts = PasswordFacts { key, record: false };
                if let Some(remembered) = key.and_then(|key| self.recall(&key)) {
                    facts.record = true;
                    opened = host.open_with_password(&path, remembered);
                    recalled = opened.is_ok();
                    stale = opened.is_err();
                }
            }
            let (worker_doc, response) = opened.map_err(|error| {
                if let HostError::Worker(error) = &error {
                    match error.code {
                        WorkerErrorCode::Encrypted => asks = Some(false),
                        // The user typed nothing wrong when a remembered password did not fit.
                        WorkerErrorCode::WrongPassword => asks = Some(!stale),
                        _ => {}
                    }
                }
                ipc_error(&error)
            })?;
            let doc = DocumentId(self.next_id());
            let (mut info, facts_of_file) = document_info(doc, display_name.clone(), response)?;
            let info_has_form = info.has_form;
            let security_in_file = info.security.clone();
            // Only now, so that a password that did not open the file is not kept.
            if let Some(typed) = &to_remember {
                match self.remember(facts.key, typed) {
                    Ok(()) => facts.record = true,
                    Err(why) => not_remembered = Some(why),
                }
            }
            let protected = given_password || recalled;
            info.password_remembered = protected && facts.record;
            info.password_recalled = recalled;
            let mut document = OpenDocument {
                info,
                host,
                path: path.clone(),
                worker_doc,
                protected,
                lost: false,
                identity,
                history: History::default(),
                form_in_file: info_has_form,
                journal: None,
                recovered: None,
                sources: Sources::default(),
                security_in_file,
                pending_source: None,
                pictures: Pictures::default(),
                text_boxes: TextBoxes::default(),
                password: facts,
                signed: facts_of_file.signed,
                identifier: facts_of_file.identifier,
            };
            self.recover_on_open(&tab, &mut document);
            Ok(document)
        });
        let event = match (&result, asks) {
            (Ok(document), _) => OpenEvent::Opened {
                tab: tab.id,
                info: document.info.clone(),
            },
            (Err(_), Some(wrong)) => OpenEvent::PasswordNeeded {
                tab: tab.id,
                display_name,
                wrong,
                can_remember: self.can_remember(&facts),
                remembered_failed: stale,
            },
            (Err(error), None) => OpenEvent::Failed {
                tab: tab.id,
                display_name,
                error: error.clone(),
            },
        };
        {
            let inner = self.lock();
            if !inner.tabs.iter().any(|open| Arc::ptr_eq(open, &tab)) {
                return not_remembered;
            }
            *lock(&tab.event) = event.clone();
            *lock(&tab.password) = facts;
            *lock(&tab.document) = result.ok();
        }
        report(event);
        not_remembered
    }

    /// Opens the file of a failed tab again, in the same tab.
    pub fn retry(&self, tab: TabId, report: &dyn Fn(OpenEvent)) -> Result<(), IpcError> {
        let found = self.tab(tab).ok_or_else(unknown_tab)?;
        let opening = OpenEvent::Opening {
            tab,
            display_name: lock(&found.display_name).clone(),
        };
        {
            let mut event = lock(&found.event);
            if !matches!(*event, OpenEvent::Failed { .. }) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "only a tab that failed to open can be retried".to_owned(),
                });
            }
            *event = opening.clone();
        }
        report(opening);
        self.load(tab, report);
        Ok(())
    }

    /// Tries `password` on a tab that asked for one (MVP-16): the tab opens, or asks again. The
    /// password goes to the tab's own new worker only, and is wiped once it has been sent. With
    /// `remember` it is also remembered once it has opened the file (B2-25, ADR 0017); the answer
    /// says why it was not.
    pub fn unlock(
        &self,
        tab: TabId,
        password: Password,
        remember: bool,
        report: &dyn Fn(OpenEvent),
    ) -> Result<UnlockOutcome, IpcError> {
        let found = self.tab(tab).ok_or_else(unknown_tab)?;
        let opening = OpenEvent::Opening {
            tab,
            display_name: lock(&found.display_name).clone(),
        };
        {
            let mut event = lock(&found.event);
            if !matches!(*event, OpenEvent::PasswordNeeded { .. }) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "only a tab that asks for a password can be unlocked".to_owned(),
                });
            }
            *event = opening.clone();
        }
        report(opening);
        let not_remembered = self.load_with(tab, Given::Typed { password, remember }, report);
        Ok(UnlockOutcome { not_remembered })
    }

    /// Closes `tab`: its worker ends and its path is forgotten.
    pub fn close(&self, tab: TabId) -> Result<(), IpcError> {
        let removed = {
            let mut inner = self.lock();
            let index = inner
                .tabs
                .iter()
                .position(|open| open.id == tab)
                .ok_or_else(unknown_tab)?;
            if inner.active == Some(tab) {
                inner.active = None;
            }
            inner.tabs.remove(index)
        };
        if let Some(mut document) = lock(&removed.document).take() {
            // Best effort: the worker process ends with its host anyway.
            let _ = document.host.notify(&WorkerRequest::Close {
                doc: document.worker_doc,
            });
            // The page asked first: the changes are discarded (B2-02). What an earlier run left
            // and the user did not answer about stays, for the next time the file opens.
            self.forget_journals(document, false);
        }
        if let (Some(journals), Some(id)) = (self.journals(), lock(&removed.released).take()) {
            journals.release(&id);
        }
        Ok(())
    }

    /// Deletes the recovery journals no tab uses (B2-13): with the recent files list (B2-12).
    pub fn clear_unused_journals(&self) {
        if let Some(journals) = self.journals() {
            journals.clear_unused();
        }
    }

    /// The user closes the window without saving (B2-02): the journals of the changes go first.
    /// The documents stay as they are until the app ends.
    pub fn discard_unsaved(&self) {
        let Some(journals) = self.journals() else {
            return;
        };
        let tabs = self.lock().tabs.clone();
        for tab in tabs {
            if let Some(id) = lock(&tab.document)
                .as_mut()
                .and_then(|document| document.journal.take())
            {
                journals.remove(&id);
            }
        }
    }

    /// Done with `document`'s journals (B2-13): its own is deleted, unless it is `kept` for the
    /// next opening of the file; edits an earlier run left are kept for it.
    fn forget_journals(&self, document: OpenDocument, kept: bool) -> Option<JournalId> {
        let journals = self.journals()?;
        if let Some(found) = document.recovered {
            journals.release(&found.id);
        }
        let own = document.journal?;
        if kept {
            journals.release(&own);
            Some(own)
        } else {
            journals.remove(&own);
            None
        }
    }

    /// Records the tab the window shows (`None`: no tab) and returns its file name, for the
    /// window title. An unknown tab counts as none.
    pub fn set_active(&self, tab: Option<TabId>) -> Option<String> {
        let mut inner = self.lock();
        let active = tab.and_then(|id| inner.tabs.iter().find(|open| open.id == id).cloned());
        inner.active = active.as_ref().map(|found| found.id);
        drop(inner);
        active.map(|found| lock(&found.display_name).clone())
    }

    /// The file name of the tab the window shows.
    pub fn active_name(&self) -> Option<String> {
        self.active_tab()
            .map(|found| lock(&found.display_name).clone())
    }

    /// Whether the document of the tab the window shows has unsaved changes (B2-02).
    pub fn active_unsaved(&self) -> bool {
        self.active_tab().is_some_and(
            |found| matches!(&*lock(&found.event), OpenEvent::Opened { info, .. } if info.unsaved),
        )
    }

    fn active_tab(&self) -> Option<Arc<Tab>> {
        let inner = self.lock();
        let active = inner.active?;
        inner.tabs.iter().find(|open| open.id == active).cloned()
    }

    /// The tabs whose documents have changes that are not in their files yet (B2-02).
    pub fn unsaved_tabs(&self) -> Vec<TabId> {
        let tabs = self.lock().tabs.clone();
        tabs.iter()
            .filter(
                |tab| matches!(&*lock(&tab.event), OpenEvent::Opened { info, .. } if info.unsaved),
            )
            .map(|tab| tab.id)
            .collect()
    }

    /// Applies an edit to its document in the document's own worker (ADR 0013) and returns the
    /// tab's new state: the document has a new id, its pages as they are now, and `unsaved`.
    /// Only `save` writes the file.
    pub fn apply_edit(&self, args: &EditArgs) -> Result<OpenEvent, IpcError> {
        args.validate().map_err(invalid_argument)?;
        let ((), event) = self.change_document(args.doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            let expected_pages = pages_after(
                &args.edit,
                page_count,
                document.info.permissions,
                &|source| document.sources.pages(source),
            )?;
            let edit = worker_edit(
                &document.sources,
                &document.pictures,
                &document.text_boxes,
                &args.edit,
            )?;
            if !document.history.has_room() {
                return Err(IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: format!("at most {MAX_UNDO_EDITS} edits between saves"),
                });
            }
            // The crash recovery journal must be able to keep it too (B2-13).
            let mut edits = document.history.applied().to_vec();
            edits.push(args.edit.clone());
            let (kept, lost) = journalled(&edits);
            let pictures = document.pictures.used_by(kept);
            let text_boxes = document.text_boxes.used_by(kept);
            Journals::check(
                &document.path,
                document.identity,
                kept,
                Named {
                    pictures: &pictures,
                    text_boxes: &text_boxes,
                },
                lost,
            )
            .map_err(|TooLarge| IpcError {
                code: ErrorCode::LimitExceeded,
                message: "too many unsaved changes to keep safe: save first".to_owned(),
            })?;
            let pages = edit_in_worker(document, &edit).inspect_err(|error| {
                // The worker failed partway through: its copy may be half edited. Opened again
                // from the file with the edits made so far, it is as it was.
                if matches!(error.code, ErrorCode::Internal | ErrorCode::LimitExceeded) {
                    document.lost = true;
                }
            })?;
            if pages.len() != expected_pages as usize {
                return Err(unexpected("Edit"));
            }
            document.history.push(args.edit.clone());
            document.info.pages = pages;
            show_history(document);
            self.keep_journal(document);
            document.info.doc = DocumentId(self.next_id());
            Ok(())
        })?;
        Ok(event)
    }

    /// Undoes the last edit of `doc` (B2-05, ADR 0013): its worker opens the document again from
    /// the bytes it keeps and applies the edits before it. Returns the tab's new state, as
    /// `apply_edit` does.
    ///
    /// A document opened with a password needs it again (#94): the password is not kept
    /// (MVP-16). Without one this answers `encrypted`, and the page asks the user for it; it
    /// goes to the worker in that one request and is wiped.
    pub fn undo(&self, doc: DocumentId, password: Option<Password>) -> Result<OpenEvent, IpcError> {
        let ((), event) = self.change_document(doc, |document| {
            let password = if document.protected {
                // The user is asked only when the app has no password for the file (B2-25).
                Some(
                    password
                        .or_else(|| self.recalled(document))
                        .ok_or_else(|| IpcError {
                            code: ErrorCode::Encrypted,
                            message: "the document's password is needed to undo".to_owned(),
                        })?,
                )
            } else {
                None
            };
            let edits = document
                .history
                .before_last()
                .ok_or_else(|| invalid_argument("nothing to undo"))?
                .iter()
                .map(|edit| {
                    worker_edit(
                        &document.sources,
                        &document.pictures,
                        &document.text_boxes,
                        edit,
                    )
                })
                .collect::<Result<_, _>>()?;
            let pages = revert_in_worker(document, edits, password).inspect_err(|error| {
                if matches!(error.code, ErrorCode::Internal | ErrorCode::LimitExceeded) {
                    document.lost = true;
                }
            })?;
            document.history.undone();
            document.info.pages = pages;
            show_history(document);
            self.keep_journal(document);
            document.info.doc = DocumentId(self.next_id());
            Ok(())
        })?;
        Ok(event)
    }

    /// Makes the last undone edit of `doc` again (B2-05); returns the tab's new state. No
    /// password is needed: the edit is applied to the document as it is.
    pub fn redo(&self, doc: DocumentId) -> Result<OpenEvent, IpcError> {
        let ((), event) = self.change_document(doc, |document| {
            let edit = document
                .history
                .next()
                .ok_or_else(|| invalid_argument("nothing to redo"))
                .and_then(|edit| {
                    worker_edit(
                        &document.sources,
                        &document.pictures,
                        &document.text_boxes,
                        edit,
                    )
                })?;
            let pages = edit_in_worker(document, &edit).inspect_err(|error| {
                if matches!(error.code, ErrorCode::Internal | ErrorCode::LimitExceeded) {
                    document.lost = true;
                }
            })?;
            document.history.redone();
            document.info.pages = pages;
            show_history(document);
            self.keep_journal(document);
            document.info.doc = DocumentId(self.next_id());
            Ok(())
        })?;
        Ok(event)
    }

    /// Makes the edits an earlier run left for the file of `doc` again (B2-13); returns the tab's
    /// new state, as `apply_edit` does. They become the document's history, to undo one by one.
    /// Only on the file as it was when they were made, and with no edits of the document's own:
    /// those would have been made on the file, not on the edits.
    pub fn recover(&self, doc: DocumentId) -> Result<OpenEvent, IpcError> {
        let ((), event) = self.change_document(doc, |document| {
            if !matches!(
                document.info.recovery,
                Recovery::Available | Recovery::Partial
            ) {
                return Err(invalid_argument("no changes that can be made again"));
            }
            if document.history.unsaved() {
                return Err(invalid_argument("the document has changes of its own"));
            }
            let edits = document
                .recovered
                .as_ref()
                .map(|found| found.edits.clone())
                .unwrap_or_default();
            let pictures = document
                .recovered
                .as_ref()
                .map(|found| found.pictures.clone())
                .unwrap_or_default();
            let text_boxes = document
                .recovered
                .as_ref()
                .map(|found| found.text_boxes.clone())
                .unwrap_or_default();
            document.pictures.restore(pictures);
            document.text_boxes.restore(text_boxes);
            // On failure the offer stays: the worker may only have been unlucky.
            replay(document, &edits)?;
            if let Some(found) = document.recovered.take() {
                // The journal of the edits is the document's own now.
                document.journal = Some(found.id);
            }
            document.info.recovery = Recovery::None;
            self.keep_journal(document);
            document.info.doc = DocumentId(self.next_id());
            Ok(())
        })?;
        Ok(event)
    }

    /// Discards the edits an earlier run left for the file of `doc` (B2-13): their journal is
    /// deleted. Returns the tab's new state.
    pub fn discard_recovered(&self, doc: DocumentId) -> Result<OpenEvent, IpcError> {
        let ((), event) = self.change_document(doc, |document| {
            let found = document
                .recovered
                .take()
                .ok_or_else(|| invalid_argument("no changes to discard"))?;
            if let Some(journals) = self.journals() {
                journals.remove(&found.id);
            }
            document.info.recovery = Recovery::None;
            Ok(())
        })?;
        Ok(event)
    }

    /// Writes the open document `doc`, with its edits, to its own file, or to `destination`
    /// (save as), and returns how, with the tab's new state (ADR 0013). Its own file is only
    /// overwritten if no other program changed it since it was read or last written. Whatever
    /// fails, the destination stays as it was, and so do the changes in the app.
    pub fn save(
        &self,
        doc: DocumentId,
        destination: Option<PathBuf>,
    ) -> Result<(SaveResult, OpenEvent), IpcError> {
        self.change_document(doc, |document| {
            let destination = match destination {
                Some(destination) => destination,
                // Nothing to write.
                None if !document.history.unsaved() => {
                    return Ok(SaveResult {
                        incremental: false,
                        redaction: None,
                    });
                }
                None => {
                    match (document.identity, FileIdentity::of(&document.path)) {
                        (Some(then), Some(now)) if then == now => {}
                        _ => {
                            return Err(IpcError {
                                code: ErrorCode::ChangedOnDisk,
                                message: "another program changed the file after it was read"
                                    .to_owned(),
                            });
                        }
                    }
                    document.path.clone()
                }
            };
            saving::check_writable(&destination)?;
            let mut temporary = Temporary::new(&destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .save(worker_doc, temporary.file())
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved {
                bytes,
                incremental,
                redaction,
                ..
            } = response
            else {
                return Err(unexpected("Save"));
            };
            temporary.check(bytes)?;
            temporary.replace(&destination)?;
            document.identity = FileIdentity::of(&destination);
            if destination != document.path {
                document.info.display_name = display_name(&destination);
                document.path = destination;
                // Edits an earlier run left are for the other file: kept for it.
                if let (Some(found), Some(journals)) = (document.recovered.take(), self.journals())
                {
                    journals.release(&found.id);
                }
                document.info.recovery = Recovery::None;
            } else if document.recovered.is_some() {
                // They were for the file as it was.
                document.info.recovery = Recovery::Stale;
            }
            // The file has the form as it is now: flattened, or not.
            document.form_in_file = document.info.has_form;
            document.history.saved();
            document.sources.clear();
            show_history(document);
            self.keep_journal(document);
            // Undo starts from the file now: the worker keeps its bytes instead (ADR 0013).
            rebase(document);
            Ok(SaveResult {
                incremental,
                redaction,
            })
        })
    }

    /// What writing the sensitive mark into the file of `doc`, or taking it out, would do (B3-05,
    /// ADR 0010): asked before the user confirms. The first reason it cannot be done, the one that
    /// lasts longest first; or that it can, with the permissions password for an encrypted file.
    pub fn mark_check(&self, doc: DocumentId) -> Result<MarkCheck, IpcError> {
        self.with_document(doc, |document| {
            Ok(if document.signed {
                MarkCheck::Signed
            } else if saving::check_writable(&document.path).is_err() {
                MarkCheck::ReadOnly
            } else if !unchanged_on_disk(document) {
                MarkCheck::ChangedOnDisk
            } else if unsaved_anywhere(document) {
                MarkCheck::Unsaved
            } else if document.info.encrypted {
                MarkCheck::NeedsPassword
            } else {
                MarkCheck::Ready
            })
        })
    }

    /// Writes the sensitive mark into the file of `doc` (a new random identifier), or takes it out
    /// (B3-05, ADR 0010), once the user confirmed: appended to the file by its worker, through a
    /// temporary file that then takes the file's place (ADR 0013); on any failure the file stays
    /// as it was. `password` is the permissions password of an encrypted file; it goes to the
    /// worker and is not kept. Refused for a signed file, with changes not saved, or when another
    /// program changed the file. The worker then keeps the file's new bytes and opens the
    /// document again from them, so that a later save keeps the mark. Returns the identifier
    /// written or taken out, and the tab's new state.
    pub fn set_sensitive(
        &self,
        doc: DocumentId,
        sensitive: bool,
        password: Option<Password>,
    ) -> Result<([u8; 16], OpenEvent), IpcError> {
        let mut fresh = [0u8; 16];
        getrandom::fill(&mut fresh).map_err(|_| IpcError {
            code: ErrorCode::Internal,
            message: "no random bytes for the document identifier".to_owned(),
        })?;
        self.change_document(doc, |document| {
            if document.signed {
                return Err(IpcError {
                    code: ErrorCode::Signed,
                    message: "a signed file is not written to".to_owned(),
                });
            }
            let identifier = match (sensitive, document.identifier) {
                (true, None) => fresh,
                (false, Some(identifier)) => identifier,
                _ => return Err(invalid_argument("the file is marked so already")),
            };
            if unsaved_anywhere(document) {
                return Err(invalid_argument(
                    "the document has changes not saved: they would go into the file too",
                ));
            }
            if !unchanged_on_disk(document) {
                return Err(IpcError {
                    code: ErrorCode::ChangedOnDisk,
                    message: "another program changed the file after it was read".to_owned(),
                });
            }
            let path = document.path.clone();
            saving::check_writable(&path)?;
            let mut temporary = Temporary::new(&path)?;
            let worker_doc = live_worker(document)?;
            // Opening the document again afterwards needs it as well.
            let again = password.clone();
            let response = document
                .host
                .write_identifier(
                    worker_doc,
                    temporary.file(),
                    sensitive.then_some(identifier),
                    password,
                )
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved { bytes, .. } = response else {
                return Err(unexpected("WriteIdentifier"));
            };
            temporary.check(bytes)?;
            temporary.replace(&path)?;
            document.identity = FileIdentity::of(&path);
            document.identifier = sensitive.then_some(identifier);
            document.info.sensitive = sensitive;
            // The worker keeps the file's bytes now, and opens the document from them: its own
            // copy has the mark, as a later save must.
            rebase(document);
            if !document.lost && revert_in_worker(document, Vec::new(), again).is_err() {
                document.lost = true;
            }
            // The worker's document is another one now, if with the same pages (as after undo).
            document.info.doc = DocumentId(self.next_id());
            Ok(identifier)
        })
    }

    /// Writes the privacy export of `doc` (B2-03) to `destination`: a copy without its metadata,
    /// made by the document's worker. The document, its edits and its file are not changed.
    /// Refused for an encrypted document, and for the document's own file.
    pub fn privacy_export(&self, doc: DocumentId, destination: &Path) -> Result<(), IpcError> {
        // The new /ID: random, so it tells nothing of when or where the copy was made.
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|_| IpcError {
            code: ErrorCode::Internal,
            message: "no random bytes for the new document id".to_owned(),
        })?;
        self.with_document(doc, |document| {
            if document.info.encrypted {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "an encrypted document has no privacy export".to_owned(),
                });
            }
            if same_file(&document.path, destination) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "the privacy export never replaces the document's own file".to_owned(),
                });
            }
            saving::check_writable(destination)?;
            let mut temporary = Temporary::new(destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .privacy_copy(worker_doc, temporary.file(), id)
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved { bytes, .. } = response else {
                return Err(unexpected("PrivacyCopy"));
            };
            temporary.check(bytes)?;
            temporary.replace(destination)
        })
    }

    /// Writes to `destination` a copy of `doc`, edits included, whose pictures are encoded again
    /// at `quality` and shrunk to `max_side` pixels on their longer side (B3-01,
    /// docs/architecture/compress-images.md): made by the document's worker. The document, its
    /// edits and its file are not changed. Refused for an encrypted document (its copy could not
    /// be encrypted again), for a signed one (the worker says so: its signatures would not hold),
    /// and for the document's own file.
    pub fn compress_copy(
        &self,
        doc: DocumentId,
        destination: &Path,
        quality: ImageQuality,
        max_side: Option<u32>,
    ) -> Result<CompressResult, IpcError> {
        self.with_document(doc, |document| {
            if document.info.encrypted {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "an encrypted document has no compressed copy".to_owned(),
                });
            }
            if same_file(&document.path, destination) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "the compressed copy never replaces the document's own file"
                        .to_owned(),
                });
            }
            saving::check_writable(destination)?;
            let mut temporary = Temporary::new(destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .compressed_copy(worker_doc, temporary.file(), quality, max_side)
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Compressed {
                bytes,
                before,
                images,
                changed,
                partial,
                ..
            } = response
            else {
                return Err(unexpected("CompressedCopy"));
            };
            temporary.check(bytes)?;
            temporary.replace(destination)?;
            Ok(CompressResult {
                before,
                after: bytes,
                images,
                changed,
                partial,
            })
        })
    }

    /// Writes a copy of `doc` to `destination`, edits included, encrypted with AES-256 (B2-15,
    /// docs/architecture/encrypt-copy.md), made by the document's worker: `open_password` opens
    /// it (none: anyone can), `owner_password` lifts `restrictions`. The passwords go to the
    /// worker in one request and are wiped; nothing is kept. The document, its edits and its
    /// file are not changed. Refused for an encrypted document, whose copy would drop what its
    /// author asked for, for a signed one (its signatures would not hold; the worker says so), and
    /// for the document's own file.
    pub fn encrypted_copy(
        &self,
        doc: DocumentId,
        destination: &Path,
        open_password: Option<Password>,
        owner_password: Password,
        restrictions: Restrictions,
    ) -> Result<(), IpcError> {
        self.with_document(doc, |document| {
            if document.info.encrypted {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "an encrypted document has no encrypted copy".to_owned(),
                });
            }
            if same_file(&document.path, destination) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "the encrypted copy never replaces the document's own file".to_owned(),
                });
            }
            saving::check_writable(destination)?;
            let mut temporary = Temporary::new(destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .encrypted_copy(
                    worker_doc,
                    temporary.file(),
                    open_password,
                    owner_password,
                    restrictions,
                )
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved { bytes, .. } = response else {
                return Err(unexpected("EncryptedCopy"));
            };
            temporary.check(bytes)?;
            temporary.replace(destination)
        })
    }

    /// Writes the pages `pages` (0-based, no repeats, in this order) of `doc` to `destination` as a
    /// document of their own (B2-06): made by the document's worker from the document as it is
    /// now, edits included. The document and its file are not changed. Refused for an encrypted
    /// document, whose copy could not be encrypted again, and for the document's own file.
    pub fn save_pages(
        &self,
        doc: DocumentId,
        pages: &[u32],
        destination: &Path,
    ) -> Result<(), IpcError> {
        self.with_document(doc, |document| {
            if document.info.encrypted {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "an encrypted document cannot be split: its pages would lose the protection its author asked for"
                        .to_owned(),
                });
            }
            // Pages are copied out of the document: the author's permission to copy covers it.
            if !document.info.permissions.copy {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "the document's author does not allow copying its content"
                        .to_owned(),
                });
            }
            if same_file(&document.path, destination) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "some pages never replace the document's own file".to_owned(),
                });
            }
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            let mut seen = std::collections::HashSet::new();
            for &page in pages {
                check_page_index(page, page_count).map_err(invalid_argument)?;
                if !seen.insert(page) {
                    return Err(invalid_argument("a page appears twice"));
                }
            }
            if pages.is_empty() {
                return Err(invalid_argument("no pages to save"));
            }
            saving::check_writable(destination)?;
            let mut temporary = Temporary::new(destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .save_pages(worker_doc, pages, temporary.file())
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved { bytes, .. } = response else {
                return Err(unexpected("SavePages"));
            };
            temporary.check(bytes)?;
            temporary.replace(destination)
        })
    }

    /// Writes the pages `pages` (0-based, no repeats, in this order) of `doc` to `destination` as
    /// a Word document (B3-02, docs/architecture/docx-export.md): made by the document's worker
    /// from the document as it is now, edits included. The document and its file are not changed.
    /// An encrypted document is exported as its text is: it is open, and the author's permission
    /// to copy is what matters. Refused for the document's own file.
    pub fn docx_export(
        &self,
        doc: DocumentId,
        pages: &[u32],
        destination: &Path,
    ) -> Result<(), IpcError> {
        self.with_document(doc, |document| {
            if !document.info.permissions.copy {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "the document's author does not allow copying its content".to_owned(),
                });
            }
            if same_file(&document.path, destination) {
                return Err(IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "a Word document never replaces the document's own file".to_owned(),
                });
            }
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            let mut seen = std::collections::HashSet::new();
            for &page in pages {
                check_page_index(page, page_count).map_err(invalid_argument)?;
                if !seen.insert(page) {
                    return Err(invalid_argument("a page appears twice"));
                }
            }
            if pages.is_empty() {
                return Err(invalid_argument("no pages to export"));
            }
            saving::check_writable(destination)?;
            let mut temporary = Temporary::new(destination)?;
            let worker_doc = live_worker(document)?;
            let response = document
                .host
                .export_docx(worker_doc, pages, temporary.file())
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::Saved { bytes, .. } = response else {
                return Err(unexpected("ExportDocx"));
            };
            temporary.check_zip(bytes)?;
            temporary.replace(destination)
        })
    }

    /// Makes the copy of the file `source` that `operation` asks for (B3-04,
    /// docs/architecture/batch.md) in `folder`, named `name`, or `name (2)`, `name (3)`, ... when a
    /// file has that name: no file is ever changed or replaced. The file is opened in a worker of
    /// its own, which ends when this returns; nothing of it is kept. An encrypted file is refused:
    /// a batch asks for no password to open one. Returns the copy's name.
    pub fn batch_copy(
        &self,
        source: &Path,
        folder: &Path,
        name: &str,
        operation: &BatchOperation,
    ) -> Result<String, IpcError> {
        check_file(source)?;
        let mut host = WorkerHost::new(self.worker.clone(), HostConfig::default());
        let (doc, opened) = host.open(source).map_err(|error| ipc_error(&error))?;
        let WorkerResponse::Opened { document, .. } = opened else {
            return Err(unexpected("Open"));
        };
        if document.encrypted {
            return Err(IpcError {
                code: ErrorCode::Encrypted,
                message: "an encrypted file has no copy in a batch".to_owned(),
            });
        }
        let mut temporary = Temporary::new(&folder.join(name))?;
        let response = match operation {
            BatchOperation::RemoveMetadata => {
                // The new /ID: random, as the privacy export's.
                let mut id = [0u8; 16];
                getrandom::fill(&mut id).map_err(|_| IpcError {
                    code: ErrorCode::Internal,
                    message: "no random bytes for the new document id".to_owned(),
                })?;
                host.privacy_copy(doc, temporary.file(), id)
            }
            BatchOperation::Encrypt {
                open_password,
                permissions_password,
                restrictions,
            } => {
                let owner_password = match permissions_password {
                    Some(password) => password.clone(),
                    None => crate::commands::unguessable_password()?,
                };
                host.encrypted_copy(
                    doc,
                    temporary.file(),
                    open_password.clone(),
                    owner_password,
                    *restrictions,
                )
            }
            BatchOperation::RemoveExif => host.exif_clean_copy(doc, temporary.file()),
        }
        .map_err(|error| ipc_error(&error))?;
        let (WorkerResponse::Saved { bytes, .. } | WorkerResponse::ExifCleaned { bytes, .. }) =
            response
        else {
            return Err(unexpected("a batch copy"));
        };
        temporary.check(bytes)?;
        for number in 1..=MAX_COPY_NUMBER {
            let destination = folder.join(crate::batch::numbered(name, number));
            if same_file(source, &destination) {
                continue;
            }
            if temporary.place_new(&destination)? {
                return Ok(display_name(&destination));
            }
        }
        Err(IpcError {
            code: ErrorCode::Unwritable,
            message: "every name for the copy is taken".to_owned(),
        })
    }

    /// Makes a PDF of `sources` (files and what each is, B3-03, docs/architecture/create-pdf.md)
    /// and writes it to `destination`: a worker of its own, which ends when this does, reads each
    /// file through a read-only handle and writes the PDF into a file next to the destination,
    /// which then takes the destination's place. No document is involved: nothing open changes,
    /// and nothing is kept. Refused when the destination is one of the files.
    pub fn create_pdf(
        &self,
        sources: &[(PathBuf, CreateKind)],
        destination: &Path,
    ) -> Result<CreateResult, IpcError> {
        let Some(&(_, kind)) = sources.first() else {
            return Err(invalid_argument("no files to make a PDF of"));
        };
        if sources.len() > MAX_CREATE_SOURCES {
            return Err(invalid_argument("too many files to make a PDF of"));
        }
        if sources.iter().any(|(path, _)| same_file(path, destination)) {
            return Err(invalid_argument(
                "the PDF never replaces one of the files it is made of",
            ));
        }
        let files = sources
            .iter()
            .map(|(path, _)| open_create_source(path))
            .collect::<Result<Vec<_>, _>>()?;
        saving::check_writable(destination)?;
        let mut temporary = Temporary::new(destination)?;
        let handed: Vec<(&std::fs::File, CreateKind)> = files
            .iter()
            .zip(sources)
            .map(|(file, (_, kind))| (file, *kind))
            .collect();
        let mut host = WorkerHost::new(self.worker.clone(), HostConfig::default());
        let response = host
            .create_pdf(&handed, temporary.file())
            .map_err(|error| ipc_error(&error))?;
        let WorkerResponse::Created { bytes, pages, .. } = response else {
            return Err(unexpected("CreatePdf"));
        };
        temporary.check(bytes)?;
        temporary.replace(destination)?;
        Ok(CreateResult { pages, kind })
    }

    /// Checks the bytes of a font file to be imported (B2-32, ADR 0018).
    pub fn inspect_font(&self, bytes: &[u8]) -> Result<Vec<InspectedFont>, IpcError> {
        let mut host = WorkerHost::new(self.worker.clone(), HostConfig::default());
        let response = host
            .inspect_font(bytes)
            .map_err(|error| ipc_error(&error))?;
        let WorkerResponse::FontInspected { faces, .. } = response else {
            return Err(unexpected("InspectFont"));
        };
        faces
            .iter()
            .try_for_each(|face| face.validate())
            .map_err(|_| unexpected("InspectFont"))?;
        Ok(faces)
    }

    /// Whether `path` is the file of the open document `doc` (the privacy export may not write
    /// there).
    pub fn is_document_file(&self, doc: DocumentId, path: &Path) -> bool {
        self.with_document(doc, |document| Ok(same_file(&document.path, path)))
            .unwrap_or(false)
    }

    /// Every tab as the frontend last heard about it, in tab order: all a reloaded page needs.
    pub fn snapshot(&self) -> Vec<OpenEvent> {
        let tabs = self.lock().tabs.clone();
        tabs.iter().map(|tab| lock(&tab.event).clone()).collect()
    }

    /// The ids of the open documents (the render cache keeps only these).
    pub fn open_documents(&self) -> Vec<DocumentId> {
        self.snapshot()
            .into_iter()
            .filter_map(|event| match event {
                OpenEvent::Opened { info, .. } => Some(info.doc),
                _ => None,
            })
            .collect()
    }

    /// Renders a page and returns it in the `render_page` wire format (ipc_contract::raster).
    /// Pages too large for the raster limits come back at a lower resolution. If the worker
    /// dies while rendering, this page fails; the next render reopens the file in a new worker.
    pub fn render(&self, args: &RenderPageArgs) -> Result<Vec<u8>, IpcError> {
        args.validate().map_err(invalid_argument)?;
        self.with_document(args.doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(args.page_index, page_count).map_err(invalid_argument)?;
            let page = document.info.pages[args.page_index as usize];
            let scale = fit_scale(page, args.scale).map_err(|error| IpcError {
                code: ErrorCode::LimitExceeded,
                message: format!("page too large to render: {error}"),
            })?;
            let rendered = request(document, |request, doc| WorkerRequest::Render {
                request,
                doc,
                page_index: args.page_index,
                scale,
                rotation: args.rotation,
            })?;
            match rendered {
                WorkerResponse::Rendered { raster, .. } => {
                    encode_raster(&raster).map_err(|error| IpcError {
                        code: ErrorCode::ProtocolViolation,
                        message: format!("invalid raster: {error}"),
                    })
                }
                _ => Err(unexpected("Render")),
            }
        })
    }

    /// Number of pages of `doc`, if it is open.
    pub fn page_count(&self, doc: DocumentId) -> Option<u32> {
        self.snapshot().into_iter().find_map(|event| match event {
            OpenEvent::Opened { info, .. } if info.doc == doc => {
                Some(u32::try_from(info.pages.len()).unwrap_or(u32::MAX))
            }
            _ => None,
        })
    }

    /// Searches one page of `doc` (MVP-10); at most `max_hits` hits.
    pub fn search_page(
        &self,
        doc: DocumentId,
        page_index: u32,
        query: &str,
        case_sensitive: bool,
        max_hits: u32,
    ) -> Result<PageFound, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::SearchPage {
                request,
                doc,
                page_index,
                query: query.to_owned(),
                case_sensitive,
                max_hits,
            })?;
            match response {
                WorkerResponse::PageSearched {
                    page_index: answered,
                    hits,
                    has_text,
                    ..
                } if answered == page_index && hits.len() <= max_hits as usize => {
                    Ok(PageFound { hits, has_text })
                }
                _ => Err(unexpected("SearchPage")),
            }
        })
    }

    /// The document's outline (MVP-09). Every page target is checked against the page count.
    pub fn outline(&self, doc: DocumentId) -> Result<OutlineResult, IpcError> {
        self.with_document(doc, |document| {
            let response = request(document, |request, doc| WorkerRequest::GetOutline {
                request,
                doc,
            })?;
            let WorkerResponse::Outline { outline, .. } = response else {
                return Err(unexpected("GetOutline"));
            };
            let page_count = document.info.pages.len();
            let in_range = outline.items.iter().all(|item| match &item.target {
                Some(LinkTarget::Page { page_index, .. }) => (*page_index as usize) < page_count,
                _ => true,
            });
            if !in_range {
                return Err(IpcError {
                    code: ErrorCode::ProtocolViolation,
                    message: "outline points past the last page".to_owned(),
                });
            }
            // As for page links: a web link no browser could parse is shown as blocked.
            let mut outline = outline;
            for item in &mut outline.items {
                if let Some(target) = &mut item.target {
                    block_unopenable(target);
                }
            }
            Ok(outline)
        })
    }

    /// The web link of outline item `args.item`, checked for opening (#49). Like page links, the
    /// outline is asked from the worker again: the frontend only names the item.
    pub fn outline_link_preview(&self, args: OutlineLinkArgs) -> Result<LinkPreview, IpcError> {
        let outline = self.outline(args.doc)?;
        match outline
            .items
            .get(args.item as usize)
            .and_then(|item| item.target.as_ref())
        {
            Some(LinkTarget::Uri { uri }) => crate::links::preview(uri),
            _ => Err(IpcError {
                code: ErrorCode::InvalidArgument,
                message: "no outline item with a web link there".to_owned(),
            }),
        }
    }

    /// The links of one page (MVP-12). The worker's answer must be about that page, with one id
    /// per link and page targets inside the document.
    /// One page's text for selecting and copying (MVP-15): its lines and where their
    /// characters are, as the worker reported them (validated by the worker host).
    pub fn page_text(&self, doc: DocumentId, page_index: u32) -> Result<PageText, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::GetPageText {
                request,
                doc,
                page_index,
            })?;
            match response {
                WorkerResponse::PageText {
                    page_index: answered,
                    text,
                    ..
                } if answered == page_index => Ok(text),
                _ => Err(unexpected("GetPageText")),
            }
        })
    }

    /// The form fields of one page (B2-09): where they are, what they hold and what can be put
    /// in them. Nothing in a field is ever run.
    pub fn page_fields(
        &self,
        doc: DocumentId,
        page_index: u32,
    ) -> Result<Vec<FormField>, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::GetPageFields {
                request,
                doc,
                page_index,
            })?;
            let WorkerResponse::PageFields {
                page_index: answered,
                fields,
                ..
            } = response
            else {
                return Err(unexpected("GetPageFields"));
            };
            if answered != page_index {
                return Err(IpcError {
                    code: ErrorCode::ProtocolViolation,
                    message: "page fields do not match the document".to_owned(),
                });
            }
            Ok(fields)
        })
    }

    /// Whether pages may be put into `doc` (B2-06): its author allows changing its pages. The
    /// user is not asked for a file otherwise.
    pub fn check_can_insert_pages(&self, doc: DocumentId) -> Result<(), IpcError> {
        self.with_document(doc, |document| {
            let permissions = document.info.permissions;
            if permissions.assemble || permissions.modify {
                Ok(())
            } else {
                Err(not_allowed())
            }
        })
    }

    /// Makes the clean copy of the PDF at `path`, a file the user chose to take pages from
    /// (B2-06), and keeps it for `doc`. The document's own worker does the work, through a
    /// read-only handle, as it reads any file. `password` is for an encrypted file: without it, or
    /// with a wrong one, this answers `encrypted` and remembers the path (`pending_source`, which
    /// never leaves the main process) for `unlock_pages_source`.
    pub fn prepare_pages_source(
        &self,
        doc: DocumentId,
        path: &Path,
        password: Option<Password>,
    ) -> Result<PagesSource, IpcError> {
        self.with_document(doc, |document| {
            let permissions = document.info.permissions;
            if !(permissions.assemble || permissions.modify) {
                return Err(not_allowed());
            }
            let file = open_source(path)?;
            let response = match document.host.prepare_source(&file, password) {
                Ok(response) => response,
                Err(HostError::Worker(error))
                    if matches!(
                        error.code,
                        WorkerErrorCode::Encrypted | WorkerErrorCode::WrongPassword
                    ) =>
                {
                    document.pending_source = Some(path.to_owned());
                    return Err(ipc_error(&HostError::Worker(error)));
                }
                Err(error) => return Err(lost_on(document, &error)),
            };
            let WorkerResponse::Source {
                bytes,
                pages,
                security,
                ..
            } = response
            else {
                return Err(unexpected("PrepareSource"));
            };
            let source = document
                .sources
                .add(
                    Source {
                        bytes,
                        pages,
                        security,
                    },
                    document.history.all(),
                )
                .map_err(|SourcesFull| IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: "too many files in the unsaved changes: save first".to_owned(),
                })?;
            document.pending_source = None;
            Ok(PagesSource { source, pages })
        })
    }

    /// Tries `password` on the encrypted file the user chose just before (B2-06), the one
    /// `prepare_pages_source` could not open without it.
    pub fn unlock_pages_source(
        &self,
        doc: DocumentId,
        password: Password,
    ) -> Result<PagesSource, IpcError> {
        let path = self.with_document(doc, |document| {
            document
                .pending_source
                .clone()
                .ok_or_else(|| invalid_argument("no file waits for a password"))
        })?;
        self.prepare_pages_source(doc, &path, Some(password))
    }

    /// The signatures of `doc` as its file has them (B2-14, ADR 0014), verified offline by the
    /// document's worker: whether each holds, whether the file changed after it, and who signed.
    /// Nothing is fetched, and nothing in the file is run.
    pub fn signatures(&self, doc: DocumentId) -> Result<SignatureReport, IpcError> {
        self.with_document(doc, |document| {
            let response = request(document, |request, doc| WorkerRequest::VerifySignatures {
                request,
                doc,
            })?;
            let WorkerResponse::Signatures { report, .. } = response else {
                return Err(unexpected("VerifySignatures"));
            };
            Ok(report)
        })
    }

    /// What applying the redaction marks of `doc` would do besides removing what their areas
    /// reach (B2-19, ADR 0016): the marks and their pages, and the other annotations and form
    /// fields that would be deleted with them. Asked of the document's worker, as it is now.
    pub fn redaction_summary(&self, doc: DocumentId) -> Result<RedactionSummary, IpcError> {
        self.with_document(doc, |document| {
            let response = request(document, |request, doc| {
                WorkerRequest::SummariseRedactions { request, doc }
            })?;
            let WorkerResponse::RedactionSummary { summary, .. } = response else {
                return Err(unexpected("SummariseRedactions"));
            };
            Ok(summary)
        })
    }

    /// Makes the picture in `file` (a PNG or JPEG the user chose, which the caller opened) into
    /// what a stamp of `doc` is made of, and keeps it for the document (B2-08). The document's
    /// worker does the work: it reads the file through a read-only handle, as it reads any file,
    /// and keeps nothing of it but the pixels. Returns the picture's number and its size.
    pub fn prepare_stamp_image(
        &self,
        doc: DocumentId,
        file: &std::fs::File,
    ) -> Result<StampImageInfo, IpcError> {
        self.with_document(doc, |document| {
            if !document.info.permissions.annotate {
                return Err(not_allowed());
            }
            let response = document
                .host
                .prepare_stamp_image(file)
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::StampImage {
                png, width, height, ..
            } = response
            else {
                return Err(unexpected("PrepareStampImage"));
            };
            // What the worker made is checked as a journal's pictures are: a PNG file of the
            // size it says, of a size a stamp can have.
            if stamp_png_size(&png).map_err(|_| unexpected("PrepareStampImage"))? != (width, height)
            {
                return Err(unexpected("PrepareStampImage"));
            }
            let image = document
                .pictures
                .add(png, document.history.all())
                .map_err(|Full| IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: "too many pictures in the unsaved changes: save first".to_owned(),
                })?;
            Ok(StampImageInfo {
                image,
                width,
                height,
            })
        })
    }

    /// Makes `text` in the font `font` (its name, and the bytes of its file, which the caller
    /// read) into the prepared words of a text box of `doc`, and keeps them for the document
    /// (B2-29, ADR 0018). The document's worker does the work: it reads the font, which is not
    /// trusted, checks its licence, finds a glyph for each character and cuts the font to those
    /// glyphs; what is kept is that small model, and never the font. Returns the number of the
    /// words, to put them in a box with, and whether the font is for viewing and printing only.
    pub fn prepare_text_box(
        &self,
        doc: DocumentId,
        text: &str,
        font: (String, Vec<u8>, u32),
    ) -> Result<PreparedTextBox, IpcError> {
        check_text_box_text(text).map_err(invalid_argument)?;
        let (name, bytes, face) = font;
        self.with_document(doc, |document| {
            if !document.info.permissions.annotate {
                return Err(not_allowed());
            }
            let font = FontData { bytes, face, name };
            let response = document
                .host
                .request(|request| WorkerRequest::PrepareTextBox {
                    request,
                    font,
                    text: text.to_owned(),
                })
                .map_err(|error| lost_on(document, &error))?;
            let WorkerResponse::TextBoxPrepared { model, .. } = response else {
                return Err(unexpected("PrepareTextBox"));
            };
            let view_only = model.view_only;
            let model = document
                .text_boxes
                .add(model, document.history.all())
                .map_err(|TextBoxesFull| IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: "too many text boxes in the unsaved changes: save first".to_owned(),
                })?;
            Ok(PreparedTextBox { model, view_only })
        })
    }

    /// What text box `annotation` of page `page_index` says of itself (B2-29): its words, the
    /// name of its font, its style, and whether the words fit: to fill in the dialog that changes
    /// it. Read from the file by the document's worker; the font is not needed.
    pub fn text_box(
        &self,
        doc: DocumentId,
        page_index: u32,
        annotation: AnnotationId,
    ) -> Result<TextBoxInfo, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::GetTextBox {
                request,
                doc,
                page_index,
                annotation,
            })?;
            let WorkerResponse::TextBox { info, .. } = response else {
                return Err(unexpected("GetTextBox"));
            };
            Ok(info)
        })
    }

    /// The annotations of one page that can be selected and removed (B2-07): highlighter marks,
    /// notes and other kinds, each by its number in the document.
    pub fn page_annotations(
        &self,
        doc: DocumentId,
        page_index: u32,
    ) -> Result<Vec<PageAnnotation>, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::GetPageAnnotations {
                request,
                doc,
                page_index,
            })?;
            let WorkerResponse::PageAnnotations {
                page_index: answered,
                annotations,
                ..
            } = response
            else {
                return Err(unexpected("GetPageAnnotations"));
            };
            let mut ids = std::collections::HashSet::new();
            if answered != page_index
                || !annotations
                    .iter()
                    .all(|annotation| ids.insert(annotation.id))
            {
                return Err(IpcError {
                    code: ErrorCode::ProtocolViolation,
                    message: "page annotations do not match the document".to_owned(),
                });
            }
            Ok(annotations)
        })
    }

    pub fn page_links(&self, doc: DocumentId, page_index: u32) -> Result<Vec<PageLink>, IpcError> {
        self.with_document(doc, |document| {
            let page_count = document.info.pages.len();
            check_page_index(page_index, u32::try_from(page_count).unwrap_or(u32::MAX))
                .map_err(invalid_argument)?;
            let response = request(document, |request, doc| WorkerRequest::GetPageLinks {
                request,
                doc,
                page_index,
            })?;
            let WorkerResponse::PageLinks {
                page_index: answered,
                links,
                ..
            } = response
            else {
                return Err(unexpected("GetPageLinks"));
            };
            let mut ids = std::collections::HashSet::new();
            let consistent = answered == page_index
                && links.iter().all(|link| {
                    ids.insert(link.id)
                        && match link.target {
                            LinkTarget::Page { page_index, .. } => {
                                (page_index as usize) < page_count
                            }
                            _ => true,
                        }
                });
            if !consistent {
                return Err(IpcError {
                    code: ErrorCode::ProtocolViolation,
                    message: "page links do not match the document".to_owned(),
                });
            }
            // A web link that could never be opened (a browser could not parse it) is shown as
            // blocked, so that what the status bar says matches what a click does.
            Ok(links
                .into_iter()
                .map(|mut link| {
                    block_unopenable(&mut link.target);
                    link
                })
                .collect())
        })
    }

    /// The web link `args` names, checked for opening (MVP-12). The link is asked from the
    /// worker again: the frontend only names it, it never supplies the URI.
    pub fn link_preview(&self, args: LinkArgs) -> Result<LinkPreview, IpcError> {
        let links = self.page_links(args.doc, args.link.page_index)?;
        let not_a_web_link = || IpcError {
            code: ErrorCode::InvalidArgument,
            message: "no web link with that id".to_owned(),
        };
        match links.into_iter().find(|link| link.id == args.link) {
            Some(PageLink {
                target: LinkTarget::Uri { uri },
                ..
            }) => crate::links::preview(&uri),
            _ => Err(not_a_web_link()),
        }
    }

    #[cfg(test)]
    fn worker_id(&self, doc: DocumentId) -> Option<u32> {
        self.with_document(doc, |document| Ok(document.host.worker_id()))
            .ok()
            .flatten()
    }

    fn tab(&self, id: TabId) -> Option<Arc<Tab>> {
        self.lock().tabs.iter().find(|open| open.id == id).cloned()
    }

    /// The file of `tab`, for the recent files list (#73). Never leaves the main process.
    pub fn tab_path(&self, tab: TabId) -> Option<PathBuf> {
        self.tab(tab).map(|found| lock(&found.path).clone())
    }

    /// The app's document identifier in the file of `tab` (B3-05, ADR 0010), if it is open and
    /// marked as sensitive.
    pub fn tab_identifier(&self, tab: TabId) -> Option<[u8; 16]> {
        let found = self.tab(tab)?;
        let document = lock(&found.document);
        document.as_ref()?.identifier
    }

    /// The identifiers in the files of the open tabs that are marked as sensitive (B3-05).
    pub fn open_identifiers(&self) -> Vec<[u8; 16]> {
        let tabs = self.lock().tabs.clone();
        tabs.iter()
            .filter_map(|tab| lock(&tab.document).as_ref()?.identifier)
            .collect()
    }

    /// What the frontend was told about the open document `doc` (its name, pages, permissions).
    pub fn document_info(&self, doc: DocumentId) -> Option<DocumentInfo> {
        let tabs = self.lock().tabs.clone();
        tabs.iter().find_map(|tab| match &*lock(&tab.event) {
            OpenEvent::Opened { info, .. } if info.doc == doc => Some(info.clone()),
            _ => None,
        })
    }

    /// A page as a PNG (B2-04) or JPEG (#111) file at `dpi`, unturned, for exporting. Pages too
    /// large for the raster limits come out at a lower resolution, as renders do.
    pub fn render_image(
        &self,
        doc: DocumentId,
        page_index: u32,
        dpi: u32,
        kind: ImageKind,
    ) -> Result<Vec<u8>, IpcError> {
        self.with_document(doc, |document| {
            let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
            check_page_index(page_index, page_count).map_err(invalid_argument)?;
            let page = document.info.pages[page_index as usize];
            let scale = fit_scale(page, dpi as f32 / 72.0).map_err(|error| IpcError {
                code: ErrorCode::LimitExceeded,
                message: format!("page too large to export: {error}"),
            })?;
            match kind {
                ImageKind::Png => {
                    match request(document, |request, doc| WorkerRequest::RenderPng {
                        request,
                        doc,
                        page_index,
                        scale,
                    })? {
                        WorkerResponse::Png { png, .. } => Ok(png),
                        _ => Err(unexpected("RenderPng")),
                    }
                }
                ImageKind::Jpeg => {
                    match request(document, |request, doc| WorkerRequest::RenderJpeg {
                        request,
                        doc,
                        page_index,
                        scale,
                    })? {
                        WorkerResponse::Jpeg { jpeg, .. } => Ok(jpeg),
                        _ => Err(unexpected("RenderJpeg")),
                    }
                }
            }
        })
    }

    /// The file of the open document `doc`, for the recent files list (#73).
    pub fn document_path(&self, doc: DocumentId) -> Option<PathBuf> {
        let tabs = self.lock().tabs.clone();
        tabs.iter()
            .find(|tab| matches!(&*lock(&tab.event), OpenEvent::Opened { info, .. } if info.doc == doc))
            .map(|tab| lock(&tab.path).clone())
    }

    /// Runs `work` on the open document `doc`, holding only that document's lock: requests for
    /// other tabs go on meanwhile.
    fn with_document<T>(
        &self,
        doc: DocumentId,
        work: impl FnOnce(&mut OpenDocument) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        let tabs = self.lock().tabs.clone();
        let tab = tabs
            .iter()
            .find(|tab| matches!(&*lock(&tab.event), OpenEvent::Opened { info, .. } if info.doc == doc))
            .ok_or_else(unknown_document)?;
        let mut document = lock(&tab.document);
        let result = match document.as_mut() {
            Some(document) if document.info.doc == doc => work(document),
            _ => Err(unknown_document()),
        };
        if let Some(lost) = document.take_if(|document| document.protected && document.lost) {
            drop(document);
            self.lose(tab, lost);
        }
        result
    }

    /// Like `with_document`, for `work` that changes the document (an edit, saving). Once it
    /// succeeds, the tab's state is recorded for a reloaded page and returned for the frontend.
    fn change_document<T>(
        &self,
        doc: DocumentId,
        work: impl FnOnce(&mut OpenDocument) -> Result<T, IpcError>,
    ) -> Result<(T, OpenEvent), IpcError> {
        let tabs = self.lock().tabs.clone();
        let tab = tabs
            .iter()
            .find(|tab| matches!(&*lock(&tab.event), OpenEvent::Opened { info, .. } if info.doc == doc))
            .ok_or_else(unknown_document)?;
        let mut document = lock(&tab.document);
        let result = match document.as_mut() {
            Some(document) if document.info.doc == doc => work(document),
            _ => Err(unknown_document()),
        };
        if let Some(lost) = document.take_if(|document| document.protected && document.lost) {
            drop(document);
            self.lose(tab, lost);
            return Err(result.err().unwrap_or_else(unknown_document));
        }
        let value = result?;
        let (info, path) = document
            .as_ref()
            .map(|document| (document.info.clone(), document.path.clone()))
            .ok_or_else(unknown_document)?;
        drop(document);
        *lock(&tab.display_name) = info.display_name.clone();
        *lock(&tab.path) = path;
        let event = OpenEvent::Opened { tab: tab.id, info };
        *lock(&tab.event) = event.clone();
        Ok((value, event))
    }

    /// The worker of `document`, opened with a password, died, and the password was not kept
    /// (MVP-16): the tab asks for it again, unless the app remembers one for the file, which
    /// opens it again by itself (B2-25). The journal of its edits stays, for them to be made
    /// again once the file opens (B2-13).
    fn lose(&self, tab: &Tab, document: OpenDocument) {
        let recalled = self.recalled(&document);
        *lock(&tab.password) = document.password;
        let kept = self.forget_journals(document, true);
        *lock(&tab.released) = kept;
        match recalled {
            Some(password) => {
                let report = |event: OpenEvent| {
                    if let Some(report) = self.reporter.get() {
                        report(event);
                    }
                };
                self.load_with(tab.id, Given::Remembered(password), &report);
            }
            None => self.ask_for_password_again(tab),
        }
    }

    /// The tab asks for its file's password again.
    fn ask_for_password_again(&self, tab: &Tab) {
        let asking = OpenEvent::PasswordNeeded {
            tab: tab.id,
            display_name: lock(&tab.display_name).clone(),
            wrong: false,
            can_remember: self.can_remember(&lock(&tab.password)),
            remembered_failed: false,
        };
        *lock(&tab.event) = asking.clone();
        if let Some(report) = self.reporter.get() {
            report(asking);
        }
    }

    /// Whether the user may ask for the password of a file with these facts to be remembered.
    fn can_remember(&self, facts: &PasswordFacts) -> bool {
        facts.key.is_some() && self.credentials.get().is_some()
    }

    /// The password the app remembers for `key` (B2-25, ADR 0017). A store that cannot be read,
    /// or an entry someone changed, is no password: the user is asked.
    fn recall(&self, key: &PasswordKey) -> Option<Password> {
        let credentials = self.credentials.get()?;
        let remembered = credentials.recall(&key.0).ok().flatten()?;
        Some(Password::new(remembered.as_str().to_owned()))
    }

    /// The password the app remembers for `document`'s file, if it has one.
    fn recalled(&self, document: &OpenDocument) -> Option<Password> {
        let facts = document.password;
        if !facts.record {
            return None;
        }
        facts.key.and_then(|key| self.recall(&key))
    }

    /// Remembers `password` for `key`; why not, if it cannot be.
    fn remember(&self, key: Option<PasswordKey>, password: &Password) -> Result<(), NotRemembered> {
        let key = key.ok_or(NotRemembered::NoKey)?;
        let credentials = self.credentials.get().ok_or(NotRemembered::Unavailable)?;
        credentials
            .remember(&key.0, password.as_str())
            .map_err(|error| match error {
                CredentialError::Unsuitable => NotRemembered::Unsuitable,
                CredentialError::Full => NotRemembered::Full,
                CredentialError::Damaged | CredentialError::Unavailable(_) => {
                    NotRemembered::Unavailable
                }
            })
    }

    /// Forgets the password the app remembers for the file of `tab` (B2-25, ADR 0017). The
    /// document stays open; the returned state says it no longer has one.
    pub fn forget_password(&self, tab: TabId) -> Result<OpenEvent, IpcError> {
        let found = self.tab(tab).ok_or_else(unknown_tab)?;
        let doc = match &*lock(&found.event) {
            OpenEvent::Opened { info, .. } => info.doc,
            _ => {
                return Err(invalid_argument(
                    "only an open document has a remembered password",
                ));
            }
        };
        let ((), event) = self.change_document(doc, |document| {
            if let Some(key) = document.password.key.filter(|_| document.password.record) {
                let credentials = self.credentials.get().ok_or_else(credentials_unavailable)?;
                credentials.forget(&key.0).map_err(credentials_error)?;
            }
            document.password.record = false;
            document.info.password_remembered = false;
            document.info.password_recalled = false;
            Ok(())
        })?;
        Ok(event)
    }

    /// Forgets every password the app remembers; returns how many there were (B2-25). The open
    /// documents no longer have one, and the page is told.
    pub fn forget_all_passwords(&self) -> Result<u32, IpcError> {
        let credentials = self.credentials.get().ok_or_else(credentials_unavailable)?;
        let forgotten = credentials.forget_all().map_err(credentials_error)?;
        let tabs = self.lock().tabs.clone();
        for tab in tabs {
            lock(&tab.password).record = false;
            let event = {
                let mut document = lock(&tab.document);
                match document.as_mut() {
                    Some(document) if document.password.record => {
                        document.password.record = false;
                        document.info.password_remembered = false;
                        document.info.password_recalled = false;
                        OpenEvent::Opened {
                            tab: tab.id,
                            info: document.info.clone(),
                        }
                    }
                    _ => continue,
                }
            };
            *lock(&tab.event) = event.clone();
            if let Some(report) = self.reporter.get() {
                report(event);
            }
        }
        Ok(u32::try_from(forgotten).unwrap_or(u32::MAX))
    }

    /// How many passwords the app remembers, and the most it will (B2-25).
    pub fn remembered_passwords(&self) -> Result<RememberedPasswords, IpcError> {
        let credentials = self.credentials.get().ok_or_else(credentials_unavailable)?;
        let count = credentials.count().map_err(credentials_error)?;
        Ok(RememberedPasswords {
            count: u32::try_from(count).unwrap_or(u32::MAX),
            limit: u32::try_from(credentials.capacity()).unwrap_or(u32::MAX),
        })
    }

    fn journals(&self) -> Option<&Journals> {
        self.journals.get()
    }

    /// Keeps `document`'s crash recovery journal in step with its history (B2-13): the edits it
    /// has and its file does not, or no journal.
    fn keep_journal(&self, document: &mut OpenDocument) {
        let Some(journals) = self.journals() else {
            return;
        };
        if !document.history.unsaved() {
            if let Some(id) = document.journal.take() {
                journals.remove(&id);
            }
            return;
        }
        if document.journal.is_none() {
            document.journal = journals.create();
        }
        if let Some(id) = &document.journal {
            // Checked before each edit (`Journals::check`); undo only makes it smaller, and redo
            // brings back what it had.
            let (kept, lost) = journalled(document.history.applied());
            let pictures = document.pictures.used_by(kept);
            let text_boxes = document.text_boxes.used_by(kept);
            let _ = journals.write(
                id,
                &document.path,
                document.identity,
                kept,
                Named {
                    pictures: &pictures,
                    text_boxes: &text_boxes,
                },
                lost,
            );
        }
    }

    /// What an earlier run left for the file `document` just opened (B2-13): a journal of edits,
    /// offered to the user. The journal of a document this tab lost with its password is no
    /// offer: its edits are made again at once.
    fn recover_on_open(&self, tab: &Tab, document: &mut OpenDocument) {
        let Some(journals) = self.journals() else {
            return;
        };
        let released = lock(&tab.released).take();
        let found = released
            .and_then(|id| journals.take(&id, &document.path, document.identity))
            .map(|found| (found, true))
            .or_else(|| {
                journals
                    .find(&document.path, document.identity)
                    .map(|found| (found, false))
            });
        let Some((found, own)) = found else {
            return;
        };
        let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
        let applicable = found.same_file
            && pages_after_all(&found.edits, page_count, document.info.permissions).is_ok();
        // Edits that were left out (B2-06) are not made again unasked: the document would be
        // without what they did, and the user is told.
        if own && applicable && found.lost == 0 {
            document.pictures.restore(found.pictures.clone());
            document.text_boxes.restore(found.text_boxes.clone());
            if replay(document, &found.edits).is_ok() {
                document.journal = Some(found.id);
                self.keep_journal(document);
                return;
            }
        }
        document.info.recovery = match (applicable, found.lost, found.edits.is_empty()) {
            (false, _, _) => Recovery::Stale,
            (true, 0, _) => Recovery::Available,
            (true, _, true) => Recovery::Lost,
            (true, _, false) => Recovery::Partial,
        };
        document.recovered = Some(found);
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Turns a web link no browser could parse into a blocked one (it could never be opened).
fn block_unopenable(target: &mut LinkTarget) {
    if let LinkTarget::Uri { uri } = target
        && crate::links::preview(uri).is_err()
    {
        *target = LinkTarget::Blocked {
            action: BlockedAction::Other,
            target: Some(clean_display_text(uri, MAX_TEXT_BYTES as usize))
                .filter(|text| !text.is_empty()),
        };
    }
}

/// Sends a request about `document` to its worker (made by `make` from a request id and the
/// worker's document id). If the worker that had the document died, the file is reopened in a
/// new one first; if the worker dies now, the document is marked lost for the next request.
fn request(
    document: &mut OpenDocument,
    make: impl FnOnce(ipc_contract::types::RequestId, DocumentId) -> WorkerRequest,
) -> Result<WorkerResponse, IpcError> {
    let doc = live_worker(document)?;
    document
        .host
        .request(|request| make(request, doc))
        .map_err(|error| lost_on(document, &error))
}

/// As [`request`], for what may take as long as saving (pages taken from a file, B2-06).
fn request_long(
    document: &mut OpenDocument,
    make: impl FnOnce(ipc_contract::types::RequestId, DocumentId) -> WorkerRequest,
) -> Result<WorkerResponse, IpcError> {
    let doc = live_worker(document)?;
    document
        .host
        .request_long(|request| make(request, doc))
        .map_err(|error| lost_on(document, &error))
}

/// Whether `edit` takes the pages of a file: it carries the file, and may take long.
fn takes_pages(edit: &WorkerEdit) -> bool {
    matches!(edit, WorkerEdit::InsertPages { .. })
}

/// The document's id in its worker. If the worker that had the document died, the file is
/// opened again in a new one first, with the edits not yet saved.
fn live_worker(document: &mut OpenDocument) -> Result<DocumentId, IpcError> {
    if document.lost {
        if document.protected {
            // Cannot happen: `with_document` forgets such a document at once. Never reopen it
            // without its password.
            return Err(IpcError {
                code: ErrorCode::Encrypted,
                message: "the document's password is needed again".to_owned(),
            });
        }
        document.worker_doc = reopen(document)?;
        document.lost = false;
    }
    Ok(document.worker_doc)
}

/// What the frontend gets for a failed worker request; a worker that can no longer be used
/// marks the document lost, for the next request to open it again.
fn lost_on(document: &mut OpenDocument, error: &HostError) -> IpcError {
    if matches!(
        error,
        HostError::Crashed | HostError::Timeout | HostError::ProtocolViolation(_)
    ) {
        document.lost = true;
    }
    ipc_error(error)
}

/// The page count after `edit`, made on a document of `page_count` pages whose author allows
/// `permissions`; or why it cannot be made there. Edits from the page and from a recovery
/// journal (B2-13) are checked alike.
fn pages_after(
    edit: &Edit,
    page_count: u32,
    permissions: DocumentPermissions,
    source_pages: &dyn Fn(SourceId) -> Option<u32>,
) -> Result<u32, IpcError> {
    // Annotations (B2-07) have a permission of their own; the worker checks that the annotation
    // is on the page.
    let annotated: Option<Vec<u32>> = match edit {
        Edit::AddHighlight { marks, .. } => Some(marks.iter().map(|mark| mark.page).collect()),
        Edit::AddNote { page, .. }
        | Edit::DeleteAnnotation { page, .. }
        | Edit::SetHighlightColor { page, .. }
        | Edit::SetNoteText { page, .. }
        | Edit::AddInk { page, .. }
        | Edit::AddStamp { page, .. }
        | Edit::AddImageStamp { page, .. }
        | Edit::AddTextBox { page, .. }
        | Edit::SetTextBox { page, .. }
        | Edit::SetAnnotationRect { page, .. }
        | Edit::SetPictureAngle { page, .. } => Some(vec![*page]),
        _ => None,
    };
    // Redaction (B2-19, ADR 0016) changes what the pages say, marks included: the author's
    // permission to change the document covers it. The worker refuses a signed document.
    match edit {
        Edit::AddRedaction { page, .. } => {
            if !permissions.modify {
                return Err(not_allowed());
            }
            check_page_index(*page, page_count).map_err(invalid_argument)?;
            return Ok(page_count);
        }
        Edit::ApplyRedactions => {
            if !permissions.modify {
                return Err(not_allowed());
            }
            return Ok(page_count);
        }
        _ => {}
    }
    if let Some(pages) = annotated {
        if !permissions.annotate {
            return Err(not_allowed());
        }
        pages
            .into_iter()
            .try_for_each(|page| check_page_index(page, page_count))
            .map_err(invalid_argument)?;
        return Ok(page_count);
    }
    // Filling in a form (B2-09) has its own permission too (`/P` bit 9, or bit 6); the worker
    // checks that the field is on the page and can have the value.
    if let Edit::SetFieldValue { page, .. } = edit {
        if !permissions.fill_forms {
            return Err(not_allowed());
        }
        check_page_index(*page, page_count).map_err(invalid_argument)?;
        return Ok(page_count);
    }
    if let Edit::FlattenForm = edit {
        if !permissions.fill_forms {
            return Err(not_allowed());
        }
        return Ok(page_count);
    }
    // The other edits manage pages: assembling the document, as Acrobat reads the author's
    // permissions (MVP-19).
    if !(permissions.assemble || permissions.modify) {
        return Err(not_allowed());
    }
    let check_pages = |pages: &[u32]| {
        pages
            .iter()
            .try_for_each(|&page| check_page_index(page, page_count))
            .map_err(invalid_argument)
    };
    match edit {
        Edit::RotatePages { pages, .. } => {
            check_pages(pages)?;
            Ok(page_count)
        }
        Edit::DeletePages { pages } => {
            check_pages(pages)?;
            // Validated: no repeats, so fewer pages than the document has leave some.
            page_count
                .checked_sub(u32::try_from(pages.len()).unwrap_or(u32::MAX))
                .filter(|&left| left > 0)
                .ok_or_else(|| invalid_argument("a document keeps at least one page"))
        }
        Edit::MovePages { pages, before } => {
            check_pages(pages)?;
            if *before > page_count {
                return Err(invalid_argument("no such place to move pages to"));
            }
            Ok(page_count)
        }
        Edit::InsertPages { at, source } => {
            if *at > page_count {
                return Err(invalid_argument("no such place to insert pages"));
            }
            let inserted =
                source_pages(*source).ok_or_else(|| invalid_argument("no such source file"))?;
            if page_count.saturating_add(inserted) > MAX_PAGE_COUNT {
                return Err(IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: format!("a document has at most {MAX_PAGE_COUNT} pages"),
                });
            }
            Ok(page_count + inserted)
        }
        Edit::InsertBlankPage { at, like } => {
            check_page_index(*like, page_count).map_err(invalid_argument)?;
            if *at > page_count {
                return Err(invalid_argument("no such place to insert a page"));
            }
            if page_count >= MAX_PAGE_COUNT {
                return Err(IpcError {
                    code: ErrorCode::LimitExceeded,
                    message: format!("a document has at most {MAX_PAGE_COUNT} pages"),
                });
            }
            Ok(page_count + 1)
        }
        Edit::AddHighlight { .. }
        | Edit::AddNote { .. }
        | Edit::DeleteAnnotation { .. }
        | Edit::SetHighlightColor { .. }
        | Edit::SetNoteText { .. }
        | Edit::AddInk { .. }
        | Edit::AddStamp { .. }
        | Edit::AddImageStamp { .. }
        | Edit::AddTextBox { .. }
        | Edit::SetTextBox { .. }
        | Edit::SetAnnotationRect { .. }
        | Edit::SetPictureAngle { .. }
        | Edit::SetFieldValue { .. }
        | Edit::FlattenForm
        | Edit::AddRedaction { .. }
        | Edit::ApplyRedactions => Ok(page_count),
    }
}

/// What the crash recovery journal keeps of the edits a document has (B2-06): those before the
/// first that takes the pages of another file, which it cannot keep, and how many edits are left
/// out (that one, and those after it, which are made on the pages it put in).
fn journalled(applied: &[Edit]) -> (&[Edit], u32) {
    let kept = applied
        .iter()
        .position(|edit| matches!(edit, Edit::InsertPages { .. }))
        .unwrap_or(applied.len());
    (
        &applied[..kept],
        u32::try_from(applied.len() - kept).unwrap_or(u32::MAX),
    )
}

/// `pages_after` for `edits` in turn.
fn pages_after_all(
    edits: &[Edit],
    page_count: u32,
    permissions: DocumentPermissions,
) -> Result<u32, IpcError> {
    // A journal never has the pages of a file (it cannot keep one): `pages_after` refuses those.
    edits.iter().try_fold(page_count, |count, edit| {
        pages_after(edit, count, permissions, &|_| None)
    })
}

/// Makes `edits` (from a recovery journal, B2-13) on `document`, which has no edits applied, and
/// makes them its history. Checked first, as edits from the page are; if the worker fails
/// partway, the document is opened again from its file before the next request.
fn replay(document: &mut OpenDocument, edits: &[Edit]) -> Result<(), IpcError> {
    let page_count = u32::try_from(document.info.pages.len()).unwrap_or(u32::MAX);
    let expected = pages_after_all(edits, page_count, document.info.permissions)?;
    if edits.len() > MAX_UNDO_EDITS as usize {
        return Err(IpcError {
            code: ErrorCode::LimitExceeded,
            message: format!("at most {MAX_UNDO_EDITS} edits between saves"),
        });
    }
    let mut pages = document.info.pages.clone();
    for edit in edits {
        let edit = worker_edit(
            &document.sources,
            &document.pictures,
            &document.text_boxes,
            edit,
        )?;
        pages = edit_in_worker(document, &edit).inspect_err(|_| {
            document.lost = true;
        })?;
    }
    if pages.len() != expected as usize {
        document.lost = true;
        return Err(unexpected("Edit"));
    }
    for edit in edits {
        document.history.push(edit.clone());
    }
    document.info.pages = pages;
    show_history(document);
    Ok(())
}

/// `edit` as the worker is asked to make it: an edit that takes the pages of a file is sent with
/// the clean copy of the file (B2-06), a picture stamp with its picture (B2-08) and a text box
/// with its prepared words (B2-29).
fn worker_edit(
    sources: &Sources,
    pictures: &Pictures,
    text_boxes: &TextBoxes,
    edit: &Edit,
) -> Result<WorkerEdit, IpcError> {
    WorkerEdit::of(
        edit,
        |source| sources.bytes(source),
        |image| pictures.png(image),
        |model| text_boxes.model(model),
    )
    .map_err(|unknown| match unknown {
        UnknownFile::Source => invalid_argument("no such source file"),
        UnknownFile::Picture => invalid_argument("no such picture"),
        UnknownFile::TextBox => invalid_argument("no such prepared text"),
    })
}

/// Applies `edit` in the document's worker and returns the document's pages after it.
fn edit_in_worker(
    document: &mut OpenDocument,
    edit: &WorkerEdit,
) -> Result<Vec<PageSize>, IpcError> {
    let send = if takes_pages(edit) {
        request_long
    } else {
        request
    };
    match send(document, |request, doc| WorkerRequest::Edit {
        request,
        doc,
        edit: edit.clone(),
    })? {
        WorkerResponse::Edited { pages, .. } => Ok(pages),
        _ => Err(unexpected("Edit")),
    }
}

/// Opens the document again in its worker from the bytes the worker keeps and applies `edits`
/// (undo, ADR 0013); returns the document's pages after them. `password`: the user's, for a
/// document opened with one.
fn revert_in_worker(
    document: &mut OpenDocument,
    edits: Vec<WorkerEdit>,
    password: Option<Password>,
) -> Result<Vec<PageSize>, IpcError> {
    let send = if edits.iter().any(takes_pages) {
        request_long
    } else {
        request
    };
    match send(document, |request, doc| WorkerRequest::Revert {
        request,
        doc,
        edits,
        password,
    })? {
        WorkerResponse::Edited { pages, .. } => Ok(pages),
        _ => Err(unexpected("Revert")),
    }
}

/// What the frontend is told of the history: unsaved changes, and whether undo and redo can be
/// done.
fn show_history(document: &mut OpenDocument) {
    let history = &document.history;
    // A flattened form is page content: nothing is left to fill in (B2-09).
    let flattened = history
        .applied()
        .iter()
        .any(|edit| matches!(edit, Edit::FlattenForm));
    document.info.has_form = document.form_in_file && !flattened;
    document.info.unsaved = history.unsaved();
    document.info.can_undo = history.can_undo();
    document.info.can_redo = history.can_redo();
    // What the files that pages were taken from had that is active (B2-06): none of it came along,
    // and the banner says it was there.
    document.info.security = document
        .sources
        .security(&document.security_in_file, history.applied());
}

/// Has the document's worker keep the bytes of the file just written, in place of those it
/// opened, to undo from (ADR 0013). Nothing is parsed, so a document opened with a password
/// needs none here. If that fails, the document is opened again at the next request.
fn rebase(document: &mut OpenDocument) {
    let path = document.path.clone();
    let rebased = document
        .host
        .rebase(document.worker_doc, &path)
        .is_ok_and(|response| matches!(response, WorkerResponse::Rebased { .. }));
    if !rebased {
        document.lost = true;
    }
}

/// Whether the file of `document` is as it was when it was read or last written.
fn unchanged_on_disk(document: &OpenDocument) -> bool {
    matches!(
        (document.identity, FileIdentity::of(&document.path)),
        (Some(then), Some(now)) if then == now
    )
}

/// Whether `document` has changes not in its file: its own, or those an earlier run left that the
/// user has not answered for (B2-13).
fn unsaved_anywhere(document: &OpenDocument) -> bool {
    document.history.unsaved()
        || matches!(
            document.info.recovery,
            Recovery::Available | Recovery::Partial
        )
}

fn not_allowed() -> IpcError {
    IpcError {
        code: ErrorCode::InvalidArgument,
        message: "the document's author does not allow this change".to_owned(),
    }
}

/// Whether `a` and `b` name the same existing file, as the system resolves them (letter case,
/// short names, links): a dialog may spell a path differently. A missing file is no other file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn unexpected(request: &str) -> IpcError {
    IpcError {
        code: ErrorCode::ProtocolViolation,
        message: format!("unexpected response to {request}"),
    }
}

/// Opens the document's file again in a fresh worker and applies the edits not yet saved; it
/// must then have the same pages as before.
fn reopen(document: &mut OpenDocument) -> Result<DocumentId, IpcError> {
    check_file(&document.path)?;
    let (doc, response) = document
        .host
        .open(&document.path)
        .map_err(|error| ipc_error(&error))?;
    let mut pages = document_info(doc, document.info.display_name.clone(), response)?
        .0
        .pages;
    for edit in document.history.applied() {
        let edit = worker_edit(
            &document.sources,
            &document.pictures,
            &document.text_boxes,
            edit,
        )?;
        let take_long = takes_pages(&edit);
        let make = |request| WorkerRequest::Edit { request, doc, edit };
        let response = if take_long {
            document.host.request_long(make)
        } else {
            document.host.request(make)
        };
        match response {
            Ok(WorkerResponse::Edited { pages: edited, .. }) => pages = edited,
            Ok(_) => return Err(unexpected("Edit")),
            Err(error) => return Err(ipc_error(&error)),
        }
    }
    if pages != document.info.pages {
        let _ = document.host.notify(&WorkerRequest::Close { doc });
        return Err(IpcError {
            code: ErrorCode::Corrupted,
            message: "the file changed on disk after it was opened".to_owned(),
        });
    }
    Ok(doc)
}

fn unknown_document() -> IpcError {
    IpcError {
        code: ErrorCode::UnknownDocument,
        message: "no such open document".to_owned(),
    }
}

fn unknown_tab() -> IpcError {
    IpcError {
        code: ErrorCode::InvalidArgument,
        message: "no such tab".to_owned(),
    }
}

fn invalid_argument(error: impl std::fmt::Display) -> IpcError {
    IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    }
}

/// The file name of `path`, safe to show and to send to the frontend: no directory part, no
/// characters `validate` rejects, bounded length.
pub fn display_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let max = MAX_DISPLAY_NAME_BYTES as usize;
    if name.len() > max {
        let mut end = max;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    if name.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        name
    }
}

/// Checks done before the worker sees the file: it exists, is a regular file, is not too large.
/// Whether it is a readable PDF is for the worker to decide.
pub fn check_file(path: &Path) -> Result<(), IpcError> {
    let metadata = std::fs::metadata(path).map_err(|_| IpcError {
        code: ErrorCode::Unreadable,
        message: "the file does not exist or cannot be accessed".to_owned(),
    })?;
    if !metadata.is_file() {
        return Err(IpcError {
            code: ErrorCode::NotPdf,
            message: "not a regular file".to_owned(),
        });
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(IpcError {
            code: ErrorCode::TooLarge,
            message: format!("larger than {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(())
}

/// Opens a PDF file the user chose to take pages from (B2-06), read-only, for the worker: it
/// exists, is a regular file, and is no larger than a source may be. Checked on the open file,
/// so that it cannot change between the check and the reading.
pub fn open_source(path: &Path) -> Result<std::fs::File, IpcError> {
    let unreadable = |_| IpcError {
        code: ErrorCode::Unreadable,
        message: "the file does not exist or cannot be read".to_owned(),
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let metadata = file.metadata().map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(IpcError {
            code: ErrorCode::NotPdf,
            message: "not a regular file".to_owned(),
        });
    }
    if metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err(IpcError {
            code: ErrorCode::TooLarge,
            message: format!("larger than {MAX_SOURCE_BYTES} bytes"),
        });
    }
    Ok(file)
}

/// Opens a picture file the user chose for a stamp (B2-08), read-only, for the worker: it
/// exists, is a regular file, and is no larger than a picture for a stamp may be. Checked on the
/// open file, so that it cannot change between the check and the reading.
pub fn open_picture(path: &Path) -> Result<std::fs::File, IpcError> {
    let unreadable = |_| IpcError {
        code: ErrorCode::Unreadable,
        message: "the picture does not exist or cannot be read".to_owned(),
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let metadata = file.metadata().map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(invalid_argument("not a regular file"));
    }
    if metadata.len() > MAX_STAMP_SOURCE_BYTES as u64 {
        return Err(IpcError {
            code: ErrorCode::TooLarge,
            message: format!("larger than {MAX_STAMP_SOURCE_BYTES} bytes"),
        });
    }
    Ok(file)
}

/// The Credential Manager cannot be used (B2-25).
fn credentials_unavailable() -> IpcError {
    IpcError {
        code: ErrorCode::CredentialsUnavailable,
        message: "the Credential Manager is not available".to_owned(),
    }
}

fn credentials_error(error: CredentialError) -> IpcError {
    IpcError {
        code: ErrorCode::CredentialsUnavailable,
        message: format!("the Credential Manager could not be used ({error:?})"),
    }
}

/// Opens a file a PDF is made of (B3-03), read-only, for the worker: it exists, is a regular file,
/// and is no larger than a source may be. Checked on the open file, so that it cannot change
/// between the check and the reading.
fn open_create_source(path: &Path) -> Result<std::fs::File, IpcError> {
    let unreadable = |_| IpcError {
        code: ErrorCode::Unreadable,
        message: "the file does not exist or cannot be read".to_owned(),
    };
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let metadata = file.metadata().map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(invalid_argument("not a regular file"));
    }
    if metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err(IpcError {
            code: ErrorCode::TooLarge,
            message: format!("larger than {MAX_SOURCE_BYTES} bytes"),
        });
    }
    Ok(file)
}

/// Maps a host error to what the frontend receives. The message never includes worker text
/// (which could quote document content) or paths.
pub fn ipc_error(error: &HostError) -> IpcError {
    let message = match error {
        HostError::Spawn(_) => "the PDF engine could not be started".to_owned(),
        HostError::Crashed => "the PDF engine stopped unexpectedly".to_owned(),
        HostError::Timeout => "the PDF engine did not answer in time".to_owned(),
        HostError::ProtocolViolation(_) => "the PDF engine sent an invalid message".to_owned(),
        HostError::Unreadable(error) => format!("the file could not be read ({:?})", error.kind()),
        HostError::TooLarge => format!("larger than {MAX_DOCUMENT_BYTES} bytes"),
        HostError::Worker(error) => format!("the PDF engine reported {:?}", error.code),
    };
    IpcError {
        code: error.code(),
        message,
    }
}

/// What the worker said of the file itself when it opened it, which the frontend is not told.
struct FileFacts {
    signed: bool,
    identifier: Option<[u8; 16]>,
}

fn document_info(
    doc: DocumentId,
    display_name: String,
    response: WorkerResponse,
) -> Result<(DocumentInfo, FileFacts), IpcError> {
    let WorkerResponse::Opened { document, .. } = response else {
        return Err(IpcError {
            code: ErrorCode::ProtocolViolation,
            message: "unexpected response to Open".to_owned(),
        });
    };
    let facts = FileFacts {
        signed: document.signed,
        identifier: document.identifier,
    };
    let info = DocumentInfo {
        doc,
        display_name,
        pages: document.pages,
        has_outline: document.has_outline,
        has_form: document.has_form,
        security: document.security,
        permissions: document.permissions,
        unsaved: false,
        encrypted: document.encrypted,
        can_undo: false,
        can_redo: false,
        recovery: Recovery::None,
        password_remembered: false,
        password_recalled: false,
        sensitive: facts.identifier.is_some(),
    };
    info.validate().map_err(|error| IpcError {
        code: ErrorCode::Internal,
        message: format!("invalid document info: {error}"),
    })?;
    Ok((info, facts))
}

impl Documents {
    /// The tab whose document the page knows as `doc` (B2-10).
    pub fn tab_of(&self, doc: DocumentId) -> Option<TabId> {
        let tabs = self.lock().tabs.clone();
        tabs.iter()
            .find(|tab| matches!(&*lock(&tab.event), OpenEvent::Opened { info, .. } if info.doc == doc))
            .map(|tab| tab.id)
    }
}

/// Every tab, the one the window shows first; and a way into the open document of each, for
/// recognising the text of scanned pages (B2-10, src/ocr.rs).
impl Tabs for Documents {
    fn tabs(&self) -> Vec<TabId> {
        let inner = self.lock();
        let mut tabs: Vec<TabId> = inner.tabs.iter().map(|tab| tab.id).collect();
        if let Some(active) = inner.active
            && let Some(at) = tabs.iter().position(|&tab| tab == active)
        {
            tabs[..=at].rotate_right(1);
        }
        tabs
    }

    /// Only if the document is not busy with another request: recognising never makes the user
    /// wait, and never opens a lost document again (their next request does).
    fn with_link(&self, tab: TabId, step: &mut dyn FnMut(&mut dyn OcrLink)) -> bool {
        let tabs = self.lock().tabs.clone();
        let Some(tab) = tabs.iter().find(|candidate| candidate.id == tab) else {
            return false;
        };
        let mut document = match tab.document.try_lock() {
            Ok(document) => document,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        let Some(document) = document.as_mut() else {
            return false;
        };
        step(&mut Link { document });
        true
    }
}

/// A tab's open document, as recognising scanned pages uses it.
struct Link<'a> {
    document: &'a mut OpenDocument,
}

/// A request about recognising, to the worker of `document`: never to a new one. If the worker
/// that had the document died, this fails, and the document is opened again by the user's next
/// request.
fn ocr_request(
    document: &mut OpenDocument,
    make: impl FnOnce(ipc_contract::types::RequestId, DocumentId) -> WorkerRequest,
) -> Result<WorkerResponse, IpcError> {
    if document.lost {
        return Err(IpcError {
            code: ErrorCode::WorkerCrashed,
            message: "the document's worker was lost".to_owned(),
        });
    }
    let doc = document.worker_doc;
    document
        .host
        .request(|request| make(request, doc))
        .map_err(|error| lost_on(document, &error))
}

impl OcrLink for Link<'_> {
    fn frontend_doc(&self) -> DocumentId {
        self.document.info.doc
    }

    fn worker_doc(&self) -> DocumentId {
        self.document.worker_doc
    }

    fn page_count(&self) -> u32 {
        u32::try_from(self.document.info.pages.len()).unwrap_or(u32::MAX)
    }

    fn is_lost(&self) -> bool {
        self.document.lost
    }

    fn load_language(&mut self, language: &str, data: Vec<u8>) -> Result<(), IpcError> {
        match ocr_request(self.document, |request, _| WorkerRequest::OcrLoad {
            request,
            language: language.to_owned(),
            data,
        })? {
            WorkerResponse::OcrLoaded { .. } => Ok(()),
            _ => Err(unexpected("OcrLoad")),
        }
    }

    fn check_page(&mut self, page_index: u32) -> Result<OcrPageState, IpcError> {
        match ocr_request(self.document, |request, doc| WorkerRequest::OcrPage {
            request,
            doc,
            page_index,
            max_millis: PAGE_MILLIS,
        })? {
            WorkerResponse::OcrChecked { state, .. } => Ok(state),
            _ => Err(unexpected("OcrPage")),
        }
    }

    fn poll(&mut self) -> Result<(Vec<OcrFinished>, u32), IpcError> {
        match ocr_request(self.document, |request, _| WorkerRequest::OcrPoll {
            request,
        })? {
            WorkerResponse::OcrPolled {
                finished, waiting, ..
            } => Ok((finished, waiting)),
            _ => Err(unexpected("OcrPoll")),
        }
    }

    fn stop(&mut self) -> Result<(), IpcError> {
        match ocr_request(self.document, |request, _| WorkerRequest::OcrStop {
            request,
        })? {
            WorkerResponse::OcrStopped { .. } => Ok(()),
            _ => Err(unexpected("OcrStop")),
        }
    }

    fn give_up(&mut self) {
        self.document.host.stop();
        self.document.lost = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_is_only_the_file_name() {
        assert_eq!(
            display_name(Path::new(r"C:\Users\someone\報告 2026.pdf")),
            "報告 2026.pdf"
        );
        assert_eq!(display_name(Path::new(r"\\server\share\a.pdf")), "a.pdf");
        assert_eq!(display_name(Path::new("relative.pdf")), "relative.pdf");
    }

    #[test]
    fn display_name_is_always_valid() {
        // An alternate data stream name contains ':'.
        assert_eq!(
            display_name(Path::new(r"C:\x\a.pdf:hidden")),
            "a.pdf_hidden"
        );
        assert_eq!(display_name(Path::new(r"C:\")), FALLBACK_NAME);
        let long = format!(r"C:\x\{}.pdf", "文".repeat(1000));
        let name = display_name(Path::new(&long));
        assert!(name.len() <= MAX_DISPLAY_NAME_BYTES as usize);
        assert!(name.starts_with('文'));
    }

    #[test]
    fn check_file_classifies_problems() {
        let dir = std::env::temp_dir().join(format!("mvp06-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.pdf");
        std::fs::write(&file, b"%PDF-1.7").unwrap();

        assert_eq!(check_file(&file), Ok(()));
        assert_eq!(
            check_file(&dir.join("missing.pdf")).unwrap_err().code,
            ErrorCode::Unreadable
        );
        assert_eq!(check_file(&dir).unwrap_err().code, ErrorCode::NotPdf);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_picture_classifies_problems() {
        let dir = std::env::temp_dir().join(format!("b208-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("stamp.png");
        std::fs::write(&file, b"\x89PNG").unwrap();
        assert!(open_picture(&file).is_ok());
        assert_eq!(
            open_picture(&dir.join("missing.png")).unwrap_err().code,
            ErrorCode::Unreadable
        );
        // A folder is no picture.
        assert!(open_picture(&dir).is_err());
        // Larger than a picture for a stamp may be (a sparse file: nothing is written).
        let large = dir.join("large.png");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_STAMP_SOURCE_BYTES as u64 + 1)
            .unwrap();
        assert_eq!(open_picture(&large).unwrap_err().code, ErrorCode::TooLarge);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn errors_never_carry_worker_text_or_paths() {
        use ipc_contract::worker::{WorkerError, WorkerErrorCode};
        let error = ipc_error(&HostError::Worker(WorkerError {
            code: WorkerErrorCode::Encrypted,
            detail: r"secret text from C:\Users\someone\a.pdf".to_owned(),
            password_key: None,
        }));
        assert_eq!(error.code, ErrorCode::Encrypted);
        assert!(!error.message.contains("secret") && !error.message.contains(r"C:\"));

        let error = ipc_error(&HostError::Unreadable(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
        assert_eq!(error.code, ErrorCode::Unreadable);
        assert!(error.validate().is_ok());
    }

    fn missing(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mvp14-does-not-exist-{name}.pdf"))
    }

    #[test]
    fn closing_an_unknown_tab_is_an_error() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        assert_eq!(
            documents.close(TabId(7)).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(documents.snapshot(), []);
    }

    #[test]
    fn a_failed_open_is_reported_and_can_be_retried_in_the_same_tab() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        assert_eq!(
            documents.retry(TabId(1), &|_| {}).unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let tabs = documents.add(&[missing("a")], &report);
        let [tab] = tabs[..] else {
            panic!("one tab expected");
        };
        documents.load(tab, &report);
        documents.retry(tab, &report).unwrap();

        let display_name = "mvp14-does-not-exist-a.pdf".to_owned();
        let opening = OpenEvent::Opening {
            tab,
            display_name: display_name.clone(),
        };
        let failed = OpenEvent::Failed {
            tab,
            display_name,
            error: IpcError {
                code: ErrorCode::Unreadable,
                message: "the file does not exist or cannot be accessed".to_owned(),
            },
        };
        assert_eq!(
            events.into_inner(),
            [opening.clone(), failed.clone(), opening, failed.clone()]
        );
        assert_eq!(documents.snapshot(), [failed]);
    }

    #[test]
    fn files_beyond_the_tab_limit_are_not_opened() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        let events = std::cell::RefCell::new(Vec::new());
        let paths: Vec<PathBuf> = (0..MAX_TABS + 2).map(|i| missing(&i.to_string())).collect();
        let tabs = documents.add(&paths, &|event| events.borrow_mut().push(event));
        assert_eq!(tabs.len(), MAX_TABS as usize);
        let events = events.into_inner();
        assert_eq!(events.len(), MAX_TABS as usize + 1);
        assert_eq!(
            events.last(),
            Some(&OpenEvent::TabLimit { ignored_files: 2 })
        );
        // Closing a tab makes room for one more file.
        documents.close(tabs[0]).unwrap();
        assert_eq!(documents.add(&paths[..1], &|_| {}).len(), 1);
    }

    #[test]
    fn the_title_follows_the_active_tab() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        let tabs = documents.add(&[missing("a"), missing("b")], &|_| {});
        assert_eq!(documents.active_name(), None);
        assert_eq!(
            documents.set_active(Some(tabs[1])).as_deref(),
            Some("mvp14-does-not-exist-b.pdf")
        );
        assert_eq!(
            documents.active_name().as_deref(),
            Some("mvp14-does-not-exist-b.pdf")
        );
        // Closing the active tab leaves no active tab until the window says which one it shows.
        documents.close(tabs[1]).unwrap();
        assert_eq!(documents.active_name(), None);
        assert_eq!(documents.set_active(Some(tabs[1])), None);
        // Tab ids are never reused.
        assert!(documents.add(&[missing("c")], &|_| {})[0] > tabs[1]);
    }
}

/// Rendering through the real sandboxed worker. `cargo test --workspace` builds
/// `pdf_worker.exe` into the same target directory before running these.
#[cfg(test)]
mod with_worker {
    use ipc_contract::limits::MAX_RASTER_PIXELS;
    use ipc_contract::types::{
        AnnotationId, AnnotationKind, FieldId, HighlightColor, HighlightMark, InkColor, InkWidth,
        Point, Quad, Rect, RequestId, Rgb, Rotation, StampImageId, StampName, TextAlign,
        TextBoxModelId, TextBoxStyle,
    };

    use super::*;

    fn worker() -> PathBuf {
        let target = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_owned();
        let worker = target.join(worker_host::WORKER_FILE_NAME);
        assert!(
            worker.is_file(),
            "build the worker first: cargo build -p pdf_worker (cargo test --workspace does)"
        );
        worker
    }

    /// A PDF with `pages` Letter pages and a correct xref table.
    fn letter_pdf(pages: usize) -> Vec<u8> {
        pdf_with(pages, "", &[])
    }

    /// `pages` Letter pages; `catalog` is added to the catalog dictionary and `extra` objects are
    /// numbered after the pages (the first one is `3 + pages`).
    fn pdf_with(pages: usize, catalog: &str, extra: &[String]) -> Vec<u8> {
        let kids: Vec<String> = (0..pages)
            .map(|index| format!("{} 0 R", 3 + index))
            .collect();
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                kids.join(" ")
            ),
        ];
        objects.extend(
            (0..pages)
                .map(|_| "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned()),
        );
        objects.extend(extra.iter().cloned());
        pdf_objects(&objects)
    }

    /// A PDF from object bodies (object n = body n-1; object 1 is the catalog).
    fn pdf_objects(objects: &[String]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// Opens `path` in a new tab of `documents`, synchronously.
    fn open_in(documents: &Documents, path: &Path) -> DocumentInfo {
        let opened = std::cell::RefCell::new(None);
        let report = |event: OpenEvent| {
            eprintln!("EVENT: {event:?}");
            if let OpenEvent::Opened { info, .. } = event {
                *opened.borrow_mut() = Some(info);
            }
        };
        for tab in documents.add(&[path.to_owned()], &report) {
            documents.load(tab, &report);
        }
        opened.into_inner().expect("opened")
    }

    fn write_pdf(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("mvp07-{}-{name}.pdf", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn open_bytes(name: &str, bytes: &[u8]) -> (Documents, DocumentInfo, PathBuf) {
        let path = write_pdf(name, bytes);
        let documents = Documents::new(worker());
        let info = open_in(&documents, &path);
        (documents, info, path)
    }

    /// The tab that shows `doc`.
    fn tab_of(documents: &Documents, doc: DocumentId) -> TabId {
        documents
            .snapshot()
            .into_iter()
            .find_map(|event| match event {
                OpenEvent::Opened { tab, info } if info.doc == doc => Some(tab),
                _ => None,
            })
            .expect("open tab")
    }

    fn open(name: &str, pages: usize) -> (Documents, DocumentInfo, PathBuf) {
        open_bytes(name, &letter_pdf(pages))
    }

    fn args(doc: DocumentId, page_index: u32, scale: f32, rotation: Rotation) -> RenderPageArgs {
        RenderPageArgs {
            request: RequestId(1),
            doc,
            page_index,
            scale,
            rotation,
        }
    }

    /// Width and height from the 16-byte raster header.
    fn size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[..4], b"PDFR");
        let read = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let (width, height) = (read(8), read(12));
        assert_eq!(bytes.len(), 16 + width as usize * height as usize * 4);
        (width, height)
    }

    #[test]
    fn renders_pages_at_the_requested_scale_and_rotation() {
        let (documents, info, path) = open("render", 3);
        assert_eq!(info.pages.len(), 3);

        let page = documents
            .render(&args(info.doc, 2, 1.0, Rotation::None))
            .unwrap();
        assert_eq!(size(&page), (612, 792));
        let rotated = documents
            .render(&args(info.doc, 0, 0.5, Rotation::Cw90))
            .unwrap();
        assert_eq!(size(&rotated), (396, 306));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn oversized_renders_come_back_at_a_lower_resolution() {
        let (documents, info, path) = open("huge", 1);
        let (width, height) = size(
            &documents
                .render(&args(info.doc, 0, 8.0, Rotation::None))
                .unwrap(),
        );
        assert!(u64::from(width) * u64::from(height) <= u64::from(MAX_RASTER_PIXELS));
        assert!(
            width > 612 * 5,
            "still as sharp as the limit allows: {width}"
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn bad_requests_are_rejected_before_reaching_the_worker() {
        let (documents, info, path) = open("bad", 2);
        let code = |args| documents.render(&args).unwrap_err().code;
        assert_eq!(
            code(args(info.doc, 2, 1.0, Rotation::None)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(args(info.doc, 0, 0.0, Rotation::None)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(args(info.doc, 0, f32::NAN, Rotation::None)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(args(DocumentId(info.doc.0 + 1), 0, 1.0, Rotation::None)),
            ErrorCode::UnknownDocument
        );
        documents.close(tab_of(&documents, info.doc)).unwrap();
        assert_eq!(
            code(args(info.doc, 0, 1.0, Rotation::None)),
            ErrorCode::UnknownDocument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn every_document_has_a_worker_of_its_own() {
        let documents = Documents::new(worker());
        let first_path = write_pdf("own-worker-1", &letter_pdf(1));
        let second_path = write_pdf("own-worker-2", &letter_pdf(2));
        let first = open_in(&documents, &first_path);
        let second = open_in(&documents, &second_path);
        assert_ne!(first.doc, second.doc);
        let first_worker = documents.worker_id(first.doc).expect("worker running");
        let second_worker = documents.worker_id(second.doc).expect("worker running");
        assert_ne!(first_worker, second_worker);

        // Closing one tab ends its worker; the other document is untouched.
        documents.close(tab_of(&documents, first.doc)).unwrap();
        assert_eq!(
            documents
                .render(&args(first.doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        let page = documents
            .render(&args(second.doc, 1, 0.5, Rotation::None))
            .unwrap();
        assert_eq!(size(&page), (306, 396));
        assert_eq!(documents.worker_id(second.doc), Some(second_worker));
        std::fs::remove_file(first_path).ok();
        std::fs::remove_file(second_path).ok();
    }

    #[test]
    fn a_crashed_worker_fails_one_render_then_the_document_is_reopened() {
        let (documents, info, path) = open("crash", 2);
        documents
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap();

        let pid = documents.worker_id(info.doc).expect("worker running");
        let killed = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(killed.status.success());

        let error = documents
            .render(&args(info.doc, 1, 0.5, Rotation::None))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkerCrashed);
        // Same document id for the frontend; a new worker behind it.
        let page = documents
            .render(&args(info.doc, 1, 0.5, Rotation::None))
            .unwrap();
        assert_eq!(size(&page), (306, 396));
        assert_ne!(documents.worker_id(info.doc), Some(pid));
        assert_eq!(documents.open_documents(), [info.doc]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_file_changed_on_disk_is_not_silently_swapped_in() {
        let (documents, info, path) = open("changed", 2);
        let pid = documents.worker_id(info.doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert_eq!(
            documents
                .render(&args(info.doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::WorkerCrashed
        );
        std::fs::write(&path, letter_pdf(5)).unwrap();
        assert_eq!(
            documents
                .render(&args(info.doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::Corrupted
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_outline_comes_from_the_worker_and_survives_a_crash() {
        // Two pages; outline items are objects 5 and 6 under the root (object 7).
        let pdf = pdf_with(
            2,
            "/Outlines 7 0 R",
            &[
                "<< /Title (One) /Parent 7 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>".to_owned(),
                "<< /Title (Two) /Parent 7 0 R /Prev 5 0 R /Dest [4 0 R /Fit] >>".to_owned(),
                "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>".to_owned(),
            ],
        );
        let (documents, info, path) = open_bytes("outline", &pdf);
        assert!(info.has_outline);
        let pages = |outline: OutlineResult| -> Vec<(String, Option<u32>)> {
            outline
                .items
                .into_iter()
                .map(|item| {
                    let page = match item.target {
                        Some(LinkTarget::Page { page_index, .. }) => Some(page_index),
                        _ => None,
                    };
                    (item.title, page)
                })
                .collect()
        };
        let expected = vec![("One".to_owned(), Some(0)), ("Two".to_owned(), Some(1))];
        assert_eq!(pages(documents.outline(info.doc).unwrap()), expected);

        let pid = documents.worker_id(info.doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert_eq!(
            documents.outline(info.doc).unwrap_err().code,
            ErrorCode::WorkerCrashed
        );
        assert_eq!(pages(documents.outline(info.doc).unwrap()), expected);
        assert_eq!(
            documents
                .outline(DocumentId(info.doc.0 + 1))
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn page_links_come_from_the_worker() {
        let link = |rect: &str, action: &str| {
            format!("<< /Type /Annot /Subtype /Link /Rect [{rect}] /A << {action} >> >>")
        };
        let pdf = pdf_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R 7 0 R 8 0 R] >>"
                .to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
            link("72 700 200 720", "/S /GoTo /D [4 0 R /Fit]"),
            link("72 600 200 620", "/S /URI /URI (https://example.invalid/)"),
            link("72 500 200 520", "/S /Launch /F (calc.exe)"),
            // A browser could not parse this one (a space in the host).
            link("72 400 200 420", "/S /URI /URI (https://exa mple.invalid/)"),
        ]);
        let (documents, info, path) = open_bytes("links", &pdf);

        let links = documents.page_links(info.doc, 0).unwrap();
        let targets: Vec<_> = links.iter().map(|link| link.target.clone()).collect();
        assert_eq!(
            targets,
            [
                LinkTarget::Page {
                    page_index: 1,
                    x: None,
                    y: None
                },
                LinkTarget::Uri {
                    uri: "https://example.invalid/".to_owned()
                },
                LinkTarget::Blocked {
                    action: BlockedAction::Launch,
                    target: Some("calc.exe".to_owned())
                },
                LinkTarget::Blocked {
                    action: BlockedAction::Other,
                    target: Some("https://exa mple.invalid/".to_owned())
                },
            ]
        );

        // Opening names a link; only the web link can be described (and opened).
        let args = |index| LinkArgs {
            doc: info.doc,
            link: ipc_contract::types::LinkId {
                page_index: 0,
                index,
            },
        };
        let preview = documents.link_preview(args(1)).unwrap();
        assert_eq!(preview.opens, "https://example.invalid/");
        assert_eq!(preview.host.as_deref(), Some("example.invalid"));
        for not_web in [0, 2, 3, 99] {
            assert_eq!(
                documents.link_preview(args(not_web)).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "link {not_web}"
            );
        }
        assert_eq!(links[0].rect.y0, 72.0);
        assert!(documents.page_links(info.doc, 1).unwrap().is_empty());
        assert_eq!(
            documents.page_links(info.doc, 2).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            documents
                .page_links(DocumentId(info.doc.0 + 1), 0)
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn outline_web_links_are_opened_only_by_position() {
        // Two pages; outline items are objects 5 to 8 under the root (object 9).
        let item = |title: &str, links: &str, target: &str| {
            format!("<< /Title ({title}) /Parent 9 0 R {links} {target} >>")
        };
        let pdf = pdf_with(
            2,
            "/Outlines 9 0 R",
            &[
                item(
                    "Web",
                    "/Next 6 0 R",
                    "/A << /S /URI /URI (https://example.invalid/) >>",
                ),
                item(
                    "Run",
                    "/Prev 5 0 R /Next 7 0 R",
                    "/A << /S /Launch /F (calc.exe) >>",
                ),
                item("Page", "/Prev 6 0 R /Next 8 0 R", "/Dest [4 0 R /Fit]"),
                item(
                    "Broken",
                    "/Prev 7 0 R",
                    "/A << /S /URI /URI (https://exa mple.invalid/) >>",
                ),
                "<< /Type /Outlines /First 5 0 R /Last 8 0 R /Count 4 >>".to_owned(),
            ],
        );
        let (documents, info, path) = open_bytes("outline-links", &pdf);

        let targets: Vec<_> = documents
            .outline(info.doc)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.target)
            .collect();
        assert!(matches!(targets[0], Some(LinkTarget::Uri { .. })));
        // A web link no browser could parse is shown as blocked, like on a page.
        assert!(matches!(
            targets[3],
            Some(LinkTarget::Blocked {
                action: BlockedAction::Other,
                ..
            })
        ));

        let args = |item| OutlineLinkArgs {
            doc: info.doc,
            item,
        };
        let preview = documents.outline_link_preview(args(0)).unwrap();
        assert_eq!(preview.opens, "https://example.invalid/");
        for not_web in [1, 2, 3, 99] {
            assert_eq!(
                documents
                    .outline_link_preview(args(not_web))
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument,
                "item {not_web}"
            );
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn searches_a_page_through_the_worker() {
        let text = "BT /F1 24 Tf 72 700 Td (Find the Needle here, then another needle.) Tj ET";
        let pdf = pdf_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 6 0 R >> >> >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ]);
        let (documents, info, path) = open_bytes("search", &pdf);

        let found = documents
            .search_page(info.doc, 0, "needle", false, 100)
            .unwrap();
        assert!(found.has_text);
        assert_eq!(found.hits.len(), 2);
        let found = documents
            .search_page(info.doc, 0, "needle", true, 100)
            .unwrap();
        assert_eq!(found.hits.len(), 1);
        let limited = documents
            .search_page(info.doc, 0, "needle", false, 1)
            .unwrap();
        assert_eq!(limited.hits.len(), 1);
        let blank = documents
            .search_page(info.doc, 1, "needle", false, 100)
            .unwrap();
        assert!(!blank.has_text && blank.hits.is_empty());

        assert_eq!(
            documents
                .search_page(info.doc, 2, "needle", false, 100)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(documents.page_count(info.doc), Some(2));
        assert_eq!(documents.page_count(DocumentId(info.doc.0 + 1)), None);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn page_text_comes_from_the_worker() {
        let text = "BT /F1 24 Tf 72 700 Td (Copy me) Tj 0 -30 Td (and me) Tj ET";
        let pdf = pdf_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 6 0 R >> >> >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ]);
        let (documents, info, path) = open_bytes("page-text", &pdf);

        let page = documents.page_text(info.doc, 0).unwrap();
        let lines: Vec<&str> = page.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(lines, ["Copy me", "and me"]);
        // The first line is above the second: y grows down the page.
        assert!(page.lines[0].quad.ul.y < page.lines[1].quad.ul.y);
        assert!(documents.page_text(info.doc, 1).unwrap().lines.is_empty());
        assert_eq!(
            documents.page_text(info.doc, 2).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            documents
                .page_text(DocumentId(info.doc.0 + 1), 0)
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_encrypted_file_asks_for_its_password_until_it_gets_it() {
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-aes256.pdf");
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let last = || events.borrow().last().cloned().expect("an event");
        let [tab] = documents.add(&[path], &report)[..] else {
            panic!("one tab")
        };

        documents.load(tab, &report);
        assert!(matches!(
            last(),
            OpenEvent::PasswordNeeded { wrong: false, .. }
        ));
        documents
            .unlock(tab, Password::new("wrong".to_owned()), false, &report)
            .unwrap();
        assert!(matches!(
            last(),
            OpenEvent::PasswordNeeded { wrong: true, .. }
        ));
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let OpenEvent::Opened { info, .. } = last() else {
            panic!("{:?}", last())
        };
        documents
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap();
        // Only a tab that asks for a password can be unlocked.
        assert_eq!(
            documents
                .unlock(tab, Password::new("user".to_owned()), false, &report)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );

        // Its worker dies. The password was not kept, so the tab asks for it again.
        let asked = Arc::new(Mutex::new(Vec::new()));
        let sink = asked.clone();
        documents.set_reporter(move |event| sink.lock().unwrap().push(event));
        let pid = documents.worker_id(info.doc).expect("worker running");
        let killed = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(killed.status.success());
        let error = documents
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkerCrashed);
        assert!(matches!(
            asked.lock().unwrap().as_slice(),
            [OpenEvent::PasswordNeeded { tab: asking, wrong: false, .. }] if *asking == tab
        ));
        assert!(matches!(
            documents.snapshot().as_slice(),
            [OpenEvent::PasswordNeeded { .. }]
        ));
        assert_eq!(
            documents
                .render(&args(info.doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        documents
            .unlock(tab, Password::new("owner".to_owned()), false, &report)
            .unwrap();
        assert!(matches!(last(), OpenEvent::Opened { .. }));
    }

    fn rotate(doc: DocumentId, pages: Vec<u32>) -> EditArgs {
        EditArgs {
            doc,
            edit: Edit::RotatePages {
                pages,
                by: Rotation::Cw90,
            },
        }
    }

    fn opened_info(event: OpenEvent) -> DocumentInfo {
        match event {
            OpenEvent::Opened { info, .. } => info,
            other => panic!("{other:?}"),
        }
    }

    /// The first page's size in the file at `path`, opened in a fresh window.
    fn first_page_of(path: &Path) -> PageSize {
        open_in(&Documents::new(worker()), path).pages[0]
    }

    const LETTER: PageSize = PageSize {
        width_pt: 612.0,
        height_pt: 792.0,
    };
    const LANDSCAPE: PageSize = PageSize {
        width_pt: 792.0,
        height_pt: 612.0,
    };

    #[test]
    fn an_edit_gives_the_document_a_new_id_and_is_saved_as_a_copy() {
        let (documents, info, path) = open("edit-copy", 2);
        let original = std::fs::read(&path).unwrap();
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        assert_ne!(edited.doc, info.doc);
        assert_eq!(edited.pages, [LANDSCAPE, LETTER]);
        assert!(edited.unsaved);
        assert_eq!(documents.unsaved_tabs(), [tab_of(&documents, edited.doc)]);
        // Pages of the document before the edit are no longer served.
        assert_eq!(
            documents
                .render(&args(info.doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        let (width, height) = size(
            &documents
                .render(&args(edited.doc, 0, 0.5, Rotation::None))
                .unwrap(),
        );
        assert!(width > height);

        let copy = path.with_file_name(format!("b202-copy-{}.pdf", std::process::id()));
        let (result, event) = documents.save(edited.doc, Some(copy.clone())).unwrap();
        assert!(!result.incremental);
        let saved = opened_info(event);
        assert_eq!(saved.doc, edited.doc);
        assert!(!saved.unsaved);
        assert_eq!(saved.display_name, display_name(&copy));
        assert!(documents.unsaved_tabs().is_empty());
        // The tab now stands for the copy; the original is as it was.
        assert_eq!(documents.document_path(saved.doc), Some(copy.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(first_page_of(&copy), LANDSCAPE);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn saving_writes_the_file_itself_once_there_is_something_to_write() {
        let (documents, info, path) = open("save-in-place", 1);
        let before = std::fs::read(&path).unwrap();
        // Nothing changed: nothing is written.
        documents.save(info.doc, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        let (_, event) = documents.save(edited.doc, None).unwrap();
        assert!(!opened_info(event).unsaved);
        assert_eq!(first_page_of(&path), LANDSCAPE);
        // No temporary or backup file is left next to it.
        let stem = path.file_name().unwrap().to_string_lossy().into_owned();
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&stem) && name != &stem)
            .collect();
        assert!(left.is_empty(), "{left:?}");

        // Once saved, the file is the document's own again: a second edit saves over it too.
        let again = opened_info(documents.apply_edit(&rotate(edited.doc, vec![0])).unwrap());
        documents.save(again.doc, None).unwrap();
        assert_eq!(
            first_page_of(&path),
            PageSize {
                width_pt: 612.0,
                height_pt: 792.0
            }
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_file_another_program_changed_is_not_overwritten() {
        let (documents, info, path) = open("changed-before-save", 1);
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        let theirs = letter_pdf(3);
        std::fs::write(&path, &theirs).unwrap();
        let error = documents.save(edited.doc, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::ChangedOnDisk);
        assert_eq!(std::fs::read(&path).unwrap(), theirs);
        // The change is still there, to be saved as another file.
        assert_eq!(documents.unsaved_tabs().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_failed_save_keeps_the_file_and_the_changes() {
        use std::os::windows::fs::OpenOptionsExt;

        let (documents, info, path) = open("save-fails", 1);
        let before = std::fs::read(&path).unwrap();
        let identity = FileIdentity::of(&path);
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        // Another program has the file open without letting it be replaced.
        const FILE_SHARE_READ: u32 = 1;
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let error = documents.save(edited.doc, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::FileInUse);
        drop(holder);
        // Neither its content nor its modification time changed.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(FileIdentity::of(&path), identity);
        assert_eq!(documents.unsaved_tabs().len(), 1);
        // Once the other program lets go, the same changes are saved.
        documents.save(edited.doc, None).unwrap();
        assert_eq!(first_page_of(&path), LANDSCAPE);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_privacy_export_writes_a_clean_copy_and_leaves_the_document_alone() {
        const AUTHOR: &[u8] = b"Jane Q. Private-Author";
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign/metadata-full.pdf");
        let (documents, info, path) = open_bytes("privacy-export", &std::fs::read(sample).unwrap());
        assert!(!info.encrypted);
        let before = std::fs::read(&path).unwrap();
        let identity = FileIdentity::of(&path);
        let copy = path.with_file_name("privacy-export-copy.pdf");
        std::fs::remove_file(&copy).ok();

        documents.privacy_export(info.doc, &copy).unwrap();
        let written = std::fs::read(&copy).unwrap();
        assert!(written.starts_with(b"%PDF-"));
        assert!(!written.windows(AUTHOR.len()).any(|window| window == AUTHOR));
        // The document and its file are as they were.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(FileIdentity::of(&path), identity);
        assert!(documents.unsaved_tabs().is_empty());

        // Never over the document's own file, however the dialog spells it.
        let shouted = PathBuf::from(path.to_string_lossy().to_uppercase());
        assert!(documents.is_document_file(info.doc, &shouted));
        assert!(!documents.is_document_file(info.doc, &copy));
        assert_eq!(
            documents
                .privacy_export(info.doc, &shouted)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_encrypted_document_has_no_privacy_export() {
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-aes256.pdf");
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let Some(OpenEvent::Opened { info, .. }) = events.borrow().last().cloned() else {
            panic!("opened")
        };
        assert!(info.encrypted);
        let copy = std::env::temp_dir().join(format!(
            "pdf-reader-privacy-encrypted-{}.pdf",
            std::process::id()
        ));
        assert_eq!(
            documents.privacy_export(info.doc, &copy).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert!(!copy.exists());
    }

    #[test]
    fn a_compressed_copy_has_the_edits_and_leaves_the_document_alone() {
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign/image-only.pdf");
        let (documents, info, path) = open_bytes("compress-copy", &std::fs::read(sample).unwrap());
        // An edit that is not saved is in the copy, as in any copy of the document.
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        assert_ne!(edited.pages[0], info.pages[0]);
        let before = std::fs::read(&path).unwrap();
        let identity = FileIdentity::of(&path);
        let copy = path.with_file_name("compress-copy-copy.pdf");
        std::fs::remove_file(&copy).ok();

        let result = documents
            .compress_copy(edited.doc, &copy, ImageQuality::Medium, Some(1_000))
            .unwrap();
        let written = std::fs::read(&copy).unwrap();
        assert!(written.starts_with(b"%PDF-"));
        assert_eq!(result.after, written.len() as u64);
        assert!(result.images >= 1, "the sample is a picture of a page");
        assert!(result.changed <= result.images);
        assert!(!result.partial);
        assert_eq!(first_page_of(&copy), edited.pages[0]);
        // The document, its edit and its file are as they were.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(FileIdentity::of(&path), identity);
        assert_eq!(documents.unsaved_tabs().len(), 1);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_compressed_copy_never_replaces_the_documents_own_file() {
        let (documents, info, path) = open("compress-own-file", 1);
        let before = std::fs::read(&path).unwrap();
        let shouted = PathBuf::from(path.to_string_lossy().to_uppercase());
        for destination in [&path, &shouted] {
            assert_eq!(
                documents
                    .compress_copy(info.doc, destination, ImageQuality::Low, None)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_encrypted_document_and_a_signed_one_have_no_compressed_copy() {
        let copy = std::env::temp_dir().join(format!(
            "pdf-reader-compress-refused-{}.pdf",
            std::process::id()
        ));
        std::fs::remove_file(&copy).ok();
        // Encrypted: the copy could not be encrypted again.
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-aes256.pdf");
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let Some(OpenEvent::Opened { info, .. }) = events.borrow().last().cloned() else {
            panic!("opened")
        };
        assert!(info.encrypted);
        assert_eq!(
            documents
                .compress_copy(info.doc, &copy, ImageQuality::Medium, None)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // Signed: the worker says no, and the file it was to write is not left behind.
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign/signed.pdf");
        let (documents, info, path) =
            open_bytes("compress-signed", &std::fs::read(sample).unwrap());
        let error = documents
            .compress_copy(info.doc, &copy, ImageQuality::Medium, None)
            .unwrap_err();
        assert_ne!(error.code, ErrorCode::Internal, "{error:?}");
        assert!(!copy.exists());
        std::fs::remove_file(path).ok();
    }

    /// What a Word document the worker made holds: the names of its files, and its text.
    fn docx_files(path: &Path) -> Vec<String> {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        let end = bytes.len() - 22;
        let count = usize::from(u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]));
        let mut at = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
        let mut names = Vec::new();
        for _ in 0..count {
            let name_length = usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]));
            names.push(String::from_utf8(bytes[at + 46..at + 46 + name_length].to_vec()).unwrap());
            at += 46 + name_length;
        }
        names
    }

    #[test]
    fn a_word_document_has_the_pages_asked_for_edits_included_and_leaves_the_document_alone() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/mixed-text-zh-en.pdf");
        let (documents, info, path) = open_bytes("docx-export", &std::fs::read(sample).unwrap());
        // An edit that is not saved is in the document, as in any export.
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        let before = std::fs::read(&path).unwrap();
        let identity = FileIdentity::of(&path);
        let word = path.with_file_name("docx-export-copy.docx");
        std::fs::remove_file(&word).ok();

        documents.docx_export(edited.doc, &[0], &word).unwrap();
        let names = docx_files(&word);
        assert!(names.contains(&"word/document.xml".to_owned()), "{names:?}");
        // The page, turned, is a section of its own size.
        let bytes = std::fs::read(&word).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Privacy-first PDF Reader"));
        assert!(
            text.contains("w:orient=\"landscape\""),
            "the page was turned"
        );
        // The document, its edit and its file are as they were.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(FileIdentity::of(&path), identity);
        assert_eq!(documents.unsaved_tabs().len(), 1);

        // Never over the document's own file, however the dialog spells it; nor with no pages, a
        // page that is not there, or one twice.
        let shouted = PathBuf::from(path.to_string_lossy().to_uppercase());
        for destination in [&path, &shouted] {
            assert_eq!(
                documents
                    .docx_export(edited.doc, &[0], destination)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument
            );
        }
        for pages in [&[][..], &[3][..], &[0, 0][..]] {
            assert_eq!(
                documents
                    .docx_export(edited.doc, pages, &word)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument,
                "{pages:?}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(word).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_word_document_follows_the_authors_permission_to_copy_and_works_for_an_encrypted_one() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign");
        let word =
            std::env::temp_dir().join(format!("pdf-reader-docx-{}.docx", std::process::id()));
        std::fs::remove_file(&word).ok();
        // The author forbids copying: no export, as for the text.
        let (documents, info, path) = open_bytes(
            "docx-no-copy",
            &std::fs::read(corpus.join("restricted-no-copy-no-print.pdf")).unwrap(),
        );
        assert!(!info.permissions.copy);
        assert_eq!(
            documents
                .docx_export(info.doc, &[0], &word)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert!(!word.exists());
        std::fs::remove_file(path).ok();

        // An encrypted document that may be copied is exported as its text is: it is open.
        let documents = Documents::new(worker());
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[corpus.join("encrypted-aes256.pdf")], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let Some(OpenEvent::Opened { info, .. }) = events.borrow().last().cloned() else {
            panic!("opened")
        };
        assert!(info.encrypted && info.permissions.copy);
        documents.docx_export(info.doc, &[0], &word).unwrap();
        assert!(docx_files(&word).contains(&"word/document.xml".to_owned()));
        std::fs::remove_file(word).ok();
    }

    fn password(text: &str) -> Password {
        Password::new(text.to_owned())
    }

    const COPY_PRINTING: Restrictions = Restrictions {
        print: true,
        copy: false,
        modify: false,
    };

    #[test]
    fn an_encrypted_copy_has_the_edits_and_its_passwords_and_leaves_the_document_alone() {
        let (documents, info, path) = open("encrypted-copy", 3);
        // An edit that is not saved is in the copy, as in any copy of the document.
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![0])).unwrap());
        let before = std::fs::read(&path).unwrap();
        let copy = path.with_file_name("encrypted-copy-copy.pdf");
        std::fs::remove_file(&copy).ok();

        documents
            .encrypted_copy(
                edited.doc,
                &copy,
                Some(password("pw-open-7Qz")),
                password("pw-owner-7Qz"),
                COPY_PRINTING,
            )
            .unwrap();
        let written = std::fs::read(&copy).unwrap();
        assert!(written.windows(6).any(|window| window == b"/AESV3"));
        for secret in [&b"pw-open-7Qz"[..], b"pw-owner-7Qz"] {
            assert!(!written.windows(secret.len()).any(|window| window == secret));
        }
        // The document, its edit and its file are as they were; it still saves as it was.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(documents.unsaved_tabs().len(), 1);
        assert!(!documents.document_info(edited.doc).unwrap().encrypted);
        documents.save(edited.doc, None).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(!saved.windows(8).any(|window| window == b"/Encrypt"));

        // The copy asks for a password, and opens restricted as it was asked to.
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(std::slice::from_ref(&copy), &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        assert!(matches!(
            events.borrow().last(),
            Some(OpenEvent::PasswordNeeded { .. })
        ));
        documents
            .unlock(tab, password("pw-open-7Qz"), false, &report)
            .unwrap();
        let opened = opened_info(events.borrow().last().cloned().unwrap());
        assert!(opened.encrypted);
        assert_eq!(opened.pages.len(), 3);
        assert!(!opened.permissions.print && opened.permissions.copy);
        // An encrypted document has no encrypted copy: its author's restrictions would go.
        let again = copy.with_file_name("encrypted-copy-again.pdf");
        assert_eq!(
            documents
                .encrypted_copy(
                    opened.doc,
                    &again,
                    Some(password("a")),
                    password("b"),
                    COPY_PRINTING
                )
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert!(!again.exists());
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_encrypted_copy_never_replaces_the_documents_own_file_or_a_signed_documents_signatures() {
        let (documents, info, path) = open("encrypted-copy-own", 1);
        let before = std::fs::read(&path).unwrap();
        let shouted = PathBuf::from(path.to_string_lossy().to_uppercase());
        for destination in [&path, &shouted] {
            assert_eq!(
                documents
                    .encrypted_copy(
                        info.doc,
                        destination,
                        Some(password("pw-open-7Qz")),
                        password("pw-owner-7Qz"),
                        Restrictions::default(),
                    )
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).ok();

        // A signed document: the worker refuses, and nothing is left at the destination.
        let signed =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign/signed.pdf");
        let documents = Documents::new(worker());
        let info = open_in(&documents, &signed);
        let copy =
            std::env::temp_dir().join(format!("pdf-reader-signed-copy-{}.pdf", std::process::id()));
        std::fs::remove_file(&copy).ok();
        assert_eq!(
            documents
                .encrypted_copy(
                    info.doc,
                    &copy,
                    Some(password("pw-open-7Qz")),
                    password("pw-owner-7Qz"),
                    Restrictions::default(),
                )
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert!(!copy.exists());
    }

    #[test]
    fn edits_survive_a_worker_crash() {
        let (documents, info, path) = open("edit-crash", 2);
        let edited = opened_info(documents.apply_edit(&rotate(info.doc, vec![1])).unwrap());
        let pid = documents.worker_id(edited.doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert_eq!(
            documents
                .render(&args(edited.doc, 1, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::WorkerCrashed
        );
        // The file is opened again in a new worker, and the edit applied again.
        let (width, height) = size(
            &documents
                .render(&args(edited.doc, 1, 0.5, Rotation::None))
                .unwrap(),
        );
        assert!(width > height);
        let copy = path.with_file_name(format!("b202-crash-copy-{}.pdf", std::process::id()));
        documents.save(edited.doc, Some(copy.clone())).unwrap();
        let reopened = open_in(&Documents::new(worker()), &copy);
        assert_eq!(reopened.pages, [LETTER, LANDSCAPE]);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    /// Applies `edit` to the document `doc`, which then has a new id; returns its pages.
    fn edited_pages(documents: &Documents, doc: &mut DocumentId, edit: Edit) -> Vec<PageSize> {
        let info = opened_info(documents.apply_edit(&EditArgs { doc: *doc, edit }).unwrap());
        *doc = info.doc;
        info.pages
    }

    #[test]
    fn pages_come_and_go_and_survive_a_worker_crash() {
        let (documents, info, path) = open("page-management", 3);
        let mut doc = info.doc;
        // Page 2 turned a quarter tells the pages apart.
        let turn = Edit::RotatePages {
            pages: vec![1],
            by: Rotation::Cw90,
        };
        assert_eq!(
            edited_pages(&documents, &mut doc, turn),
            [LETTER, LANDSCAPE, LETTER]
        );
        let insert = Edit::InsertBlankPage { at: 0, like: 1 };
        assert_eq!(
            edited_pages(&documents, &mut doc, insert),
            [LANDSCAPE, LETTER, LANDSCAPE, LETTER]
        );
        let delete = Edit::DeletePages { pages: vec![1] };
        assert_eq!(
            edited_pages(&documents, &mut doc, delete),
            [LANDSCAPE, LANDSCAPE, LETTER]
        );
        let move_ = Edit::MovePages {
            pages: vec![2],
            before: 0,
        };
        assert_eq!(
            edited_pages(&documents, &mut doc, move_),
            [LETTER, LANDSCAPE, LANDSCAPE]
        );

        // The file is opened again in a new worker, and the four edits applied again.
        let pid = documents.worker_id(doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert_eq!(
            documents
                .render(&args(doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::WorkerCrashed
        );
        let shape = |page| {
            let (width, height) = size(
                &documents
                    .render(&args(doc, page, 0.5, Rotation::None))
                    .unwrap(),
            );
            width > height
        };
        assert_eq!((shape(0), shape(1), shape(2)), (false, true, true));
        let copy = path.with_file_name(format!("b205-copy-{}.pdf", std::process::id()));
        documents.save(doc, Some(copy.clone())).unwrap();
        let reopened = open_in(&Documents::new(worker()), &copy);
        assert_eq!(reopened.pages, [LETTER, LANDSCAPE, LANDSCAPE]);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    /// The tab's state after undoing (or, with `undo` false, redoing) an edit of `doc`, which
    /// then has a new id.
    fn stepped(documents: &Documents, doc: &mut DocumentId, undo: bool) -> DocumentInfo {
        let event = if undo {
            documents.undo(*doc, None)
        } else {
            documents.redo(*doc)
        };
        let info = opened_info(event.unwrap());
        *doc = info.doc;
        info
    }

    fn turn(page: u32) -> Edit {
        Edit::RotatePages {
            pages: vec![page],
            by: Rotation::Cw90,
        }
    }

    /// A page with a secret line between two public ones (B2-19).
    fn redaction_sample() -> Vec<u8> {
        let content = "BT /F1 18 Tf 72 700 Td (Public line one) Tj ET\n\
                       BT /F1 18 Tf 72 650 Td (SECRET-ALPHA-123 inside) Tj ET\n\
                       BT /F1 18 Tf 72 600 Td (Public line two) Tj ET\n";
        pdf_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ])
    }

    /// What page 0 of `doc` says.
    fn first_page_says(documents: &Documents, doc: DocumentId) -> String {
        documents
            .page_text(doc, 0)
            .unwrap()
            .lines
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_redaction_is_marked_applied_undone_and_saved_without_what_it_removed() {
        let (documents, info, path) = open_bytes("b219-redact", &redaction_sample());
        let mut doc = info.doc;
        let area = Rect {
            x0: 60.0,
            y0: 120.0,
            x1: 330.0,
            y1: 150.0,
        };
        edited_pages(
            &documents,
            &mut doc,
            Edit::AddRedaction {
                page: 0,
                rect: area,
            },
        );
        let summary = documents.redaction_summary(doc).unwrap();
        assert_eq!((summary.marks, summary.pages), (1, 1));
        assert!(first_page_says(&documents, doc).contains("SECRET"));

        edited_pages(&documents, &mut doc, Edit::ApplyRedactions);
        let said = first_page_says(&documents, doc);
        assert!(
            !said.contains("SECRET") && said.contains("Public line two"),
            "{said}"
        );
        assert_eq!(documents.redaction_summary(doc).unwrap().marks, 0);

        // Undone (ADR 0013): the text, and the mark, are back; done again, they go again.
        let undone = opened_info(documents.undo(doc, None).unwrap());
        doc = undone.doc;
        assert!(first_page_says(&documents, doc).contains("SECRET"));
        assert_eq!(documents.redaction_summary(doc).unwrap().marks, 1);
        let redone = opened_info(documents.redo(doc).unwrap());
        doc = redone.doc;
        assert!(!first_page_says(&documents, doc).contains("SECRET"));

        // Saved: the file is rewritten whole, has none of it, and the worker says so after
        // reading it again (B2-20).
        let (result, event) = documents.save(doc, None).unwrap();
        assert_eq!(
            result.redaction,
            Some(ipc_contract::types::RedactionCheck { places: 0 })
        );
        doc = opened_info(event).doc;
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(6).any(|window| window == b"SECRET"));
        // The file took the document's place: what was removed is forgotten, and the next save
        // has nothing to check.
        edited_pages(&documents, &mut doc, turn(0));
        assert_eq!(documents.save(doc, None).unwrap().0.redaction, None);
        let again = open_in(&Documents::new(worker()), &path);
        assert_eq!(again.pages.len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn redaction_needs_the_authors_permission_to_change_the_document() {
        let mark = Edit::AddRedaction {
            page: 0,
            rect: Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 50.0,
                y1: 50.0,
            },
        };
        let only_annotating = DocumentPermissions {
            modify: false,
            ..DocumentPermissions::ALL
        };
        for edit in [&mark, &Edit::ApplyRedactions] {
            assert_eq!(
                pages_after(edit, 1, only_annotating, &|_| None)
                    .unwrap_err()
                    .message,
                not_allowed().message
            );
            assert_eq!(
                pages_after(edit, 1, DocumentPermissions::ALL, &|_| None).unwrap(),
                1
            );
        }
        // A mark on a page the document does not have.
        let elsewhere = Edit::AddRedaction {
            page: 3,
            rect: Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 50.0,
                y1: 50.0,
            },
        };
        assert!(pages_after(&elsewhere, 1, DocumentPermissions::ALL, &|_| None).is_err());
    }

    #[test]
    fn redaction_is_refused_for_restricted_corpus_document() {
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        let (_, info) = unlocked(&documents, &path);
        assert!(!info.permissions.modify);
        let area = Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 50.0,
            y1: 50.0,
        };
        assert_eq!(
            documents
                .apply_edit(&EditArgs {
                    doc: info.doc,
                    edit: Edit::AddRedaction {
                        page: 0,
                        rect: area
                    },
                })
                .unwrap_err()
                .message,
            not_allowed().message
        );
        assert_eq!(
            documents
                .apply_edit(&EditArgs {
                    doc: info.doc,
                    edit: Edit::ApplyRedactions,
                })
                .unwrap_err()
                .message,
            not_allowed().message
        );
    }

    /// What writing the sensitive mark into the file of `doc` would do (B3-05).
    fn sensitive_check_of(documents: &Documents, doc: DocumentId) -> MarkCheck {
        documents.mark_check(doc).unwrap()
    }

    #[test]
    fn the_sensitive_mark_is_appended_and_goes_with_the_file_wherever_it_is() {
        let folder = folder("b305-mark");
        let path = folder.join("合約.pdf");
        std::fs::write(&path, letter_pdf(2)).unwrap();
        let original = std::fs::read(&path).unwrap();
        let documents = Documents::new(worker());
        let info = open_in(&documents, &path);
        assert!(!info.sensitive);
        assert_eq!(sensitive_check_of(&documents, info.doc), MarkCheck::Ready);
        assert!(documents.open_identifiers().is_empty());

        let (identifier, event) = documents.set_sensitive(info.doc, true, None).unwrap();
        let marked = opened_info(event);
        assert!(marked.sensitive);
        // The worker's document is opened again from the file: another id, the same pages.
        assert_ne!(marked.doc, info.doc);
        assert_eq!(marked.pages, info.pages);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..original.len()], &original[..]);
        assert_eq!(documents.open_identifiers(), vec![identifier]);
        let tab = tab_of(&documents, marked.doc);
        assert_eq!(documents.tab_identifier(tab), Some(identifier));
        // The document goes on as before.
        assert!(documents.page_text(marked.doc, 0).is_ok());
        // Marked already: refused, and the file stays as it is.
        let again = documents.set_sensitive(marked.doc, true, None).unwrap_err();
        assert_eq!(again.code, ErrorCode::InvalidArgument);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // No temporary file is left next to it.
        assert_eq!(names_in(&folder), ["合約.pdf"]);

        // Copied elsewhere under another name (as a move would leave it): still sensitive.
        let elsewhere = folder.join("renamed elsewhere.pdf");
        std::fs::copy(&path, &elsewhere).unwrap();
        let other = Documents::new(worker());
        assert!(open_in(&other, &elsewhere).sensitive);
        assert_eq!(other.open_identifiers(), vec![identifier]);

        // An edit saved keeps the mark; the privacy export drops it (ADR 0004).
        let mut doc = marked.doc;
        edited_pages(&documents, &mut doc, turn(0));
        let (_, saved) = documents.save(doc, None).unwrap();
        let saved = opened_info(saved);
        assert!(saved.sensitive);
        assert!(open_in(&Documents::new(worker()), &path).sensitive);
        let exported = folder.join("exported.pdf");
        documents.privacy_export(saved.doc, &exported).unwrap();
        assert!(!open_in(&Documents::new(worker()), &exported).sensitive);

        // Taken out: appended too, and the file opens as any other.
        let before = std::fs::read(&path).unwrap();
        let (removed, event) = documents.set_sensitive(saved.doc, false, None).unwrap();
        assert_eq!(removed, identifier);
        assert!(!opened_info(event).sensitive);
        assert_eq!(&std::fs::read(&path).unwrap()[..before.len()], &before[..]);
        assert!(!open_in(&Documents::new(worker()), &path).sensitive);
        assert!(documents.open_identifiers().is_empty());
        drop((documents, other));
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn the_sensitive_mark_is_not_written_where_it_should_not_be() {
        let folder = folder("b305-refused");
        let documents = Documents::new(worker());

        // Signed: refused, whatever else.
        for name in ["signed.pdf", "signed-docmdp-p1.pdf"] {
            let path = folder.join(name);
            std::fs::copy(corpus_path(&format!("benign/{name}")), &path).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let info = open_in(&documents, &path);
            assert_eq!(sensitive_check_of(&documents, info.doc), MarkCheck::Signed);
            let error = documents.set_sensitive(info.doc, true, None).unwrap_err();
            assert_eq!(error.code, ErrorCode::Signed, "{name}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name}");
        }

        // Changes not saved would go into the file too.
        let path = folder.join("edited.pdf");
        std::fs::write(&path, letter_pdf(2)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut doc = open_in(&documents, &path).doc;
        edited_pages(&documents, &mut doc, turn(1));
        assert_eq!(sensitive_check_of(&documents, doc), MarkCheck::Unsaved);
        let error = documents.set_sensitive(doc, true, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        // Another program changed the file.
        let path = folder.join("changed.pdf");
        std::fs::write(&path, letter_pdf(2)).unwrap();
        let doc = open_in(&documents, &path).doc;
        std::fs::write(&path, letter_pdf(3)).unwrap();
        assert_eq!(
            sensitive_check_of(&documents, doc),
            MarkCheck::ChangedOnDisk
        );
        let error = documents.set_sensitive(doc, true, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::ChangedOnDisk);
        assert_eq!(std::fs::read(&path).unwrap(), letter_pdf(3));

        // A read-only file.
        let path = folder.join("read-only.pdf");
        std::fs::write(&path, letter_pdf(1)).unwrap();
        let doc = open_in(&documents, &path).doc;
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();
        assert_eq!(sensitive_check_of(&documents, doc), MarkCheck::ReadOnly);
        let error = documents.set_sensitive(doc, true, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::ReadOnly);
        // Windows only (the crate does not build elsewhere): no world-writable file results.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), letter_pdf(1));

        // Nothing written anywhere, nothing left behind.
        assert_eq!(
            names_in(&folder),
            [
                "changed.pdf",
                "edited.pdf",
                "read-only.pdf",
                "signed-docmdp-p1.pdf",
                "signed.pdf"
            ]
        );
        drop(documents);
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn an_encrypted_file_is_marked_only_with_its_permissions_password() {
        let folder = folder("b305-encrypted");
        let path = folder.join("locked.pdf");
        std::fs::copy(corpus_path("benign/restricted-open-password.pdf"), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let documents = Documents::new(worker());
        let (_, info) = unlocked(&documents, &path);
        assert_eq!(
            sensitive_check_of(&documents, info.doc),
            MarkCheck::NeedsPassword
        );
        for (password, code) in [
            (None, ErrorCode::Encrypted),
            (Some("wrong"), ErrorCode::Encrypted),
            (Some("user"), ErrorCode::NotAllowed),
        ] {
            let error = documents
                .set_sensitive(
                    info.doc,
                    true,
                    password.map(|password| Password::new(password.to_owned())),
                )
                .unwrap_err();
            assert_eq!(error.code, code, "{password:?}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        let (_, event) = documents
            .set_sensitive(info.doc, true, Some(Password::new("owner".to_owned())))
            .unwrap();
        let marked = opened_info(event);
        assert!(marked.sensitive && marked.encrypted);
        // What the author restricts stays restricted here: the permissions are as before.
        assert_eq!(marked.permissions, info.permissions);
        assert!(documents.page_text(marked.doc, 0).is_ok());
        // Opened again with the open password: sensitive, still encrypted.
        let other = Documents::new(worker());
        let (_, again) = unlocked(&other, &path);
        assert!(again.sensitive && again.encrypted);
        drop((documents, other));
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn undo_and_redo_go_through_the_edits_and_back_to_the_file() {
        let (documents, info, path) = open("undo", 3);
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, turn(1));
        edited_pages(&documents, &mut doc, Edit::DeletePages { pages: vec![0] });

        let info = stepped(&documents, &mut doc, true);
        assert_eq!(info.pages, [LETTER, LANDSCAPE, LETTER]);
        assert!(info.unsaved && info.can_undo && info.can_redo);
        let info = stepped(&documents, &mut doc, true);
        assert_eq!(info.pages, [LETTER, LETTER, LETTER]);
        // The file again: nothing to save, nothing more to undo.
        assert!(!info.unsaved && !info.can_undo && info.can_redo);
        assert!(documents.unsaved_tabs().is_empty());
        assert_eq!(
            documents.undo(doc, None).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let info = stepped(&documents, &mut doc, false);
        assert_eq!(info.pages, [LETTER, LANDSCAPE, LETTER]);
        assert!(info.unsaved);
        // A new edit drops what was undone.
        edited_pages(
            &documents,
            &mut doc,
            Edit::InsertBlankPage { at: 0, like: 0 },
        );
        assert_eq!(
            documents.redo(doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn after_saving_undo_starts_from_the_file() {
        let (documents, info, path) = open("undo-save", 2);
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, turn(0));
        let (_, event) = documents.save(doc, None).unwrap();
        let saved = opened_info(event);
        assert!(!saved.unsaved && !saved.can_undo && !saved.can_redo);
        edited_pages(&documents, &mut doc, Edit::DeletePages { pages: vec![1] });
        // Undone to the file as saved, with its page turned; not as it was first opened.
        let info = stepped(&documents, &mut doc, true);
        assert_eq!(info.pages, [LANDSCAPE, LETTER]);
        assert!(!info.unsaved && !info.can_undo);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn what_was_undone_can_be_redone_after_a_worker_crash() {
        let (documents, info, path) = open("undo-crash", 2);
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, turn(0));
        edited_pages(&documents, &mut doc, turn(1));
        stepped(&documents, &mut doc, true);
        let pid = documents.worker_id(doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert_eq!(
            documents
                .render(&args(doc, 0, 0.5, Rotation::None))
                .unwrap_err()
                .code,
            ErrorCode::WorkerCrashed
        );
        // Opened again with the first turn; the second can still be made again.
        let info = stepped(&documents, &mut doc, false);
        assert_eq!(info.pages, [LANDSCAPE, LANDSCAPE]);
        let info = stepped(&documents, &mut doc, true);
        assert_eq!(info.pages, [LANDSCAPE, LETTER]);
        std::fs::remove_file(path).ok();
    }

    /// A set of remembered passwords for one test: the real Credential Manager, under a name no
    /// other set has, so that nothing of the user's is touched. What is left is deleted when the
    /// test ends.
    struct Store {
        prefix: String,
    }

    impl Store {
        fn new(name: &str) -> Self {
            Self {
                prefix: format!("PdfReaderTest/documents/{name}/{}/", std::process::id()),
            }
        }

        fn credentials(&self, capacity: usize) -> Credentials {
            Credentials::with(self.prefix.clone(), capacity)
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = self.credentials(1).forget_all();
        }
    }

    /// Documents that remember passwords in `store`: as the app does, a run of it.
    fn remembering(store: &Store) -> Documents {
        let documents = Documents::new(worker());
        documents.set_credentials(store.credentials(8));
        documents
    }

    fn sample(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign")
            .join(name)
    }

    /// Adds a tab for `path` and loads it; the events it reported.
    fn load_tab(documents: &Documents, path: &Path) -> (TabId, Vec<OpenEvent>) {
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path.to_path_buf()], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        (tab, events.into_inner())
    }

    /// Types `text` into the tab that asks for a password; what came of it and the tab's new state.
    fn type_password(
        documents: &Documents,
        tab: TabId,
        text: &str,
        remember: bool,
    ) -> (UnlockOutcome, OpenEvent) {
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let outcome = documents
            .unlock(tab, password(text), remember, &report)
            .unwrap();
        let last = events.borrow().last().cloned().expect("an event");
        (outcome, last)
    }

    impl Documents {
        /// The key the worker reported for the file of `tab`.
        fn password_key_of(&self, tab: TabId) -> Option<PasswordKey> {
            lock(&self.tab(tab)?.password).key
        }
    }

    #[test]
    fn a_password_the_user_asks_to_remember_opens_the_file_next_time_without_asking() {
        let store = Store::new("remembers");
        let original = sample("encrypted-aes256.pdf");
        let documents = remembering(&store);
        let (tab, events) = load_tab(&documents, &original);
        // The file says where a password is looked up, so the user may ask for it to be remembered.
        assert!(matches!(
            events.last(),
            Some(OpenEvent::PasswordNeeded {
                wrong: false,
                can_remember: true,
                remembered_failed: false,
                ..
            })
        ));
        // Nothing is remembered before the user asks.
        assert_eq!(documents.remembered_passwords().unwrap().count, 0);
        let (outcome, event) = type_password(&documents, tab, "user", true);
        assert_eq!(outcome.not_remembered, None);
        let info = opened_info(event);
        assert!(info.password_remembered && !info.password_recalled);
        assert_eq!(
            documents.remembered_passwords().unwrap(),
            RememberedPasswords { count: 1, limit: 8 }
        );

        // Another run of the app, and the same file under another name: no question.
        let moved =
            std::env::temp_dir().join(format!("pdf-reader-remembered-{}.pdf", std::process::id()));
        std::fs::copy(&original, &moved).unwrap();
        let again = remembering(&store);
        let (_, events) = load_tab(&again, &moved);
        let info = opened_info(events.last().cloned().unwrap());
        assert!(info.encrypted && info.password_remembered && info.password_recalled);
        again
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap();
        std::fs::remove_file(moved).ok();
    }

    #[test]
    fn a_password_the_user_did_not_ask_to_remember_is_not_kept_and_a_wrong_one_never_is() {
        let store = Store::new("forgets");
        let path = sample("encrypted-aes256.pdf");
        let documents = remembering(&store);
        let (tab, _) = load_tab(&documents, &path);
        let (outcome, event) = type_password(&documents, tab, "user", false);
        assert_eq!(outcome.not_remembered, None);
        let info = opened_info(event);
        assert!(!info.password_remembered && !info.password_recalled);
        assert_eq!(documents.remembered_passwords().unwrap().count, 0);
        // Asked to remember, but it does not open the file: nothing is kept.
        let (tab, _) = load_tab(&documents, &path);
        let (outcome, event) = type_password(&documents, tab, "wrong", true);
        assert!(matches!(
            event,
            OpenEvent::PasswordNeeded { wrong: true, .. }
        ));
        assert_eq!(outcome.not_remembered, None);
        assert_eq!(documents.remembered_passwords().unwrap().count, 0);
        // So the next time it asks again.
        let (_, events) = load_tab(&documents, &path);
        assert!(matches!(
            events.last(),
            Some(OpenEvent::PasswordNeeded { .. })
        ));
    }

    #[test]
    fn a_remembered_password_that_does_not_open_the_file_is_asked_about_once_and_left_alone() {
        let store = Store::new("stale");
        let path = sample("encrypted-aes256.pdf");
        let documents = remembering(&store);
        let (first, _) = load_tab(&documents, &path);
        // Someone changed the entry: a password that is not the file's.
        let key = documents
            .password_key_of(first)
            .expect("the file says its key");
        store
            .credentials(8)
            .remember(&key.0, "not the password")
            .unwrap();

        let (second, events) = load_tab(&documents, &path);
        // Tried once, and then the user is asked: nothing was typed wrongly, and the record stays.
        assert!(matches!(
            events.last(),
            Some(OpenEvent::PasswordNeeded {
                wrong: false,
                can_remember: true,
                remembered_failed: true,
                ..
            })
        ));
        assert_eq!(documents.remembered_passwords().unwrap().count, 1);
        // The user's own password opens it. It was not asked to be remembered: the entry stays.
        let (_, event) = type_password(&documents, second, "user", false);
        let info = opened_info(event);
        assert!(info.password_remembered && !info.password_recalled);
        let kept = store.credentials(8).recall(&key.0).unwrap().unwrap();
        assert_eq!(kept.as_str(), "not the password");

        // Asked to be remembered, the new password takes its place.
        let (third, _) = load_tab(&documents, &path);
        type_password(&documents, third, "user", true);
        let kept = store.credentials(8).recall(&key.0).unwrap().unwrap();
        assert_eq!(kept.as_str(), "user");
        let (_, events) = load_tab(&documents, &path);
        let info = opened_info(events.last().cloned().unwrap());
        assert!(info.password_recalled);
    }

    #[test]
    fn undoing_in_a_document_with_a_remembered_password_does_not_ask_for_it() {
        let store = Store::new("undo");
        let documents = remembering(&store);
        let (tab, _) = load_tab(&documents, &sample("encrypted-aes256.pdf"));
        let (_, event) = type_password(&documents, tab, "user", true);
        let opened = opened_info(event);
        let edited = opened_info(
            documents
                .apply_edit(&EditArgs {
                    doc: opened.doc,
                    edit: turn(0),
                })
                .unwrap(),
        );
        assert!(edited.unsaved && edited.can_undo);
        // The page sends none, and the main process takes the remembered one to the worker.
        let undone = opened_info(documents.undo(edited.doc, None).unwrap());
        assert_eq!(undone.pages, opened.pages);
        assert!(!undone.unsaved && undone.can_redo);
    }

    #[test]
    fn a_worker_that_died_is_replaced_and_a_remembered_password_opens_the_file_again_by_itself() {
        let store = Store::new("crash");
        let documents = remembering(&store);
        let (tab, _) = load_tab(&documents, &sample("encrypted-aes256.pdf"));
        let (_, event) = type_password(&documents, tab, "user", true);
        let info = opened_info(event);
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = reported.clone();
        documents.set_reporter(move |event| sink.lock().unwrap().push(event));
        let pid = documents.worker_id(info.doc).expect("worker running");
        let killed = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(killed.status.success());
        let error = documents
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkerCrashed);
        // No question: the tab is open again, in a new worker, with the remembered password.
        let again = match reported.lock().unwrap().as_slice() {
            [
                OpenEvent::Opened {
                    tab: reopened,
                    info,
                },
            ] if *reopened == tab => info.clone(),
            other => panic!("{other:?}"),
        };
        assert!(again.password_remembered && again.password_recalled);
        documents
            .render(&args(again.doc, 0, 0.5, Rotation::None))
            .unwrap();
        assert!(matches!(
            documents.snapshot().as_slice(),
            [OpenEvent::Opened { .. }]
        ));
    }

    #[test]
    fn a_password_is_forgotten_for_one_document_or_for_all_of_them() {
        let store = Store::new("forgetting");
        let path = sample("encrypted-aes256.pdf");
        let documents = remembering(&store);
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = reported.clone();
        documents.set_reporter(move |event| sink.lock().unwrap().push(event));
        let (tab, _) = load_tab(&documents, &path);
        let (_, event) = type_password(&documents, tab, "user", true);
        assert!(opened_info(event).password_remembered);
        assert_eq!(documents.remembered_passwords().unwrap().count, 1);

        // One document: it stays open, and says it has none any more.
        let info = opened_info(documents.forget_password(tab).unwrap());
        assert!(!info.password_remembered && !info.password_recalled);
        assert_eq!(documents.remembered_passwords().unwrap().count, 0);
        let (again, events) = load_tab(&documents, &path);
        assert!(matches!(
            events.last(),
            Some(OpenEvent::PasswordNeeded { .. })
        ));
        // Only an open document has one to forget.
        assert_eq!(
            documents.forget_password(again).unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        // All of them: the documents that are open are told.
        let (_, event) = type_password(&documents, again, "user", true);
        assert!(opened_info(event).password_remembered);
        assert_eq!(documents.forget_all_passwords().unwrap(), 1);
        assert_eq!(documents.remembered_passwords().unwrap().count, 0);
        let told = reported.lock().unwrap();
        let [.., OpenEvent::Opened { tab: last, info }] = told.as_slice() else {
            panic!("{told:?}")
        };
        assert_eq!(*last, again);
        assert!(!info.password_remembered);
    }

    #[test]
    fn a_full_store_or_none_at_all_says_why_and_the_file_opens_all_the_same() {
        let store = Store::new("full");
        let path = sample("encrypted-aes256.pdf");
        let documents = Documents::new(worker());
        documents.set_credentials(store.credentials(1));
        // Another file's key has the only place.
        store
            .credentials(1)
            .remember(&[7; 32], "someone else's")
            .unwrap();
        let (tab, _) = load_tab(&documents, &path);
        let (outcome, event) = type_password(&documents, tab, "user", true);
        assert_eq!(outcome.not_remembered, Some(NotRemembered::Full));
        let info = opened_info(event);
        assert!(info.encrypted && !info.password_remembered);
        assert_eq!(
            documents.remembered_passwords().unwrap(),
            RememberedPasswords { count: 1, limit: 1 }
        );

        // Without a store (the tests of the rest of the app have none), the user is not offered it.
        let documents = Documents::new(worker());
        let (tab, events) = load_tab(&documents, &path);
        assert!(matches!(
            events.last(),
            Some(OpenEvent::PasswordNeeded {
                can_remember: false,
                ..
            })
        ));
        let (outcome, event) = type_password(&documents, tab, "user", true);
        assert_eq!(outcome.not_remembered, Some(NotRemembered::Unavailable));
        assert!(opened_info(event).encrypted);
        assert_eq!(
            documents.remembered_passwords().unwrap_err().code,
            ErrorCode::CredentialsUnavailable
        );
        // A file that does not say how its password is checked has no key to remember it by.
        assert_eq!(
            documents.remember(None, &password("x")),
            Err(NotRemembered::NoKey)
        );
    }

    #[test]
    fn undo_asks_again_for_the_password_of_a_document_opened_with_one() {
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-aes256.pdf");
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let opened = opened_info(events.borrow().last().cloned().unwrap());
        let mut doc = opened.doc;
        let edited = opened_info(
            documents
                .apply_edit(&EditArgs { doc, edit: turn(0) })
                .unwrap(),
        );
        doc = edited.doc;
        assert!(edited.unsaved && edited.can_undo);
        // Its password is not kept: undo asks for it (`encrypted`), and a wrong one is refused
        // the same way, changing nothing; the page knows which, having sent one or not.
        let undo = |password: Option<&str>| {
            documents.undo(
                doc,
                password.map(|password| Password::new(password.to_owned())),
            )
        };
        assert_eq!(undo(None).unwrap_err().code, ErrorCode::Encrypted);
        assert_eq!(undo(Some("wrong")).unwrap_err().code, ErrorCode::Encrypted);
        let undone = opened_info(undo(Some("user")).unwrap());
        assert_eq!(undone.pages, opened.pages);
        assert!(!undone.unsaved && !undone.can_undo && undone.can_redo);
        // Redo needs none: the edit is applied to the document as it is.
        doc = undone.doc;
        let redone = opened_info(documents.redo(doc).unwrap());
        assert_eq!(redone.pages, edited.pages);
    }

    #[test]
    fn edits_are_checked_before_they_reach_the_worker() {
        let (documents, info, path) = open("bad-edit", 1);
        let edit = |edit: Edit| EditArgs {
            doc: info.doc,
            edit,
        };
        for args in [
            rotate(info.doc, vec![1]),
            rotate(info.doc, vec![]),
            // The last page cannot go.
            edit(Edit::DeletePages { pages: vec![0] }),
            edit(Edit::DeletePages { pages: vec![1] }),
            edit(Edit::MovePages {
                pages: vec![0],
                before: 2,
            }),
            edit(Edit::InsertBlankPage { at: 2, like: 0 }),
            edit(Edit::InsertBlankPage { at: 0, like: 1 }),
        ] {
            assert_eq!(
                documents.apply_edit(&args).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{args:?}"
            );
        }
        assert_eq!(
            documents
                .apply_edit(&rotate(DocumentId(9_999), vec![0]))
                .unwrap_err()
                .code,
            ErrorCode::UnknownDocument
        );
        assert!(documents.unsaved_tabs().is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_author_who_forbids_changing_pages_is_obeyed() {
        // RC4, revision 2, /P without the modify bit (tests/corpus/generate.py).
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let info = opened_info(events.borrow().last().cloned().unwrap());
        assert!(!info.permissions.modify && !info.permissions.assemble);
        assert_eq!(
            documents
                .apply_edit(&rotate(info.doc, vec![0]))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    /// A data folder of its own, for the crash recovery journals (B2-13); removed when dropped.
    struct DataFolder(PathBuf);

    impl DataFolder {
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir().join(format!("b213-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            Self(folder)
        }

        fn recovery(&self) -> PathBuf {
            self.0.join(crate::recovery::FOLDER_NAME)
        }

        /// A run of the app that keeps its journals here.
        fn documents(&self) -> Documents {
            let documents = Documents::new(worker());
            documents.set_journals(Journals::new(Some(self.recovery())));
            documents
        }

        fn journals(&self) -> usize {
            std::fs::read_dir(self.recovery()).map_or(0, Iterator::count)
        }

        /// Writes a journal of `edits` for the file at `path` as it is now, as if an earlier run
        /// had.
        fn leave(&self, path: &Path, edits: serde_json::Value) {
            let (len, since) = FileIdentity::of(path).unwrap().parts().unwrap();
            let journal = serde_json::json!({
                "version": 1,
                "path": path.to_str().unwrap(),
                "len": len,
                "modifiedSecs": since.as_secs(),
                "modifiedNanos": since.subsec_nanos(),
                "edits": edits,
            });
            std::fs::create_dir_all(self.recovery()).unwrap();
            std::fs::write(
                self.recovery().join(format!("{:032x}.json", 1)),
                journal.to_string(),
            )
            .unwrap();
        }
    }

    impl Drop for DataFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Opens the encrypted `path` with its user password in a new tab of `documents`.
    fn unlocked(documents: &Documents, path: &Path) -> (TabId, DocumentInfo) {
        let events = std::cell::RefCell::new(Vec::new());
        let report = |event: OpenEvent| events.borrow_mut().push(event);
        let [tab] = documents.add(&[path.to_owned()], &report)[..] else {
            panic!("one tab")
        };
        documents.load(tab, &report);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &report)
            .unwrap();
        let info = opened_info(events.borrow().last().cloned().unwrap());
        (tab, info)
    }

    #[test]
    fn edits_a_run_did_not_save_are_offered_the_next_time_the_file_opens() {
        let data = DataFolder::new("offer");
        let path = write_pdf("b213-offer", &letter_pdf(3));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        edited_pages(&earlier, &mut doc, Edit::DeletePages { pages: vec![1] });
        edited_pages(&earlier, &mut doc, turn(0));
        assert_eq!(data.journals(), 1);
        // The app ends without saving or discarding: its workers go, the journal stays.
        drop(earlier);
        assert_eq!(data.journals(), 1);

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Available);
        assert_eq!(reopened.pages, [LETTER; 3]);
        assert!(!reopened.unsaved);
        // Another tab of the same file is not offered them too.
        assert_eq!(open_in(&later, &path).recovery, Recovery::None);

        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        assert_eq!(recovered.pages, [LANDSCAPE, LETTER]);
        assert!(recovered.unsaved && recovered.can_undo);
        assert_eq!(recovered.recovery, Recovery::None);
        // They are the document's history, undone one by one.
        let undone = opened_info(later.undo(recovered.doc, None).unwrap());
        assert_eq!(undone.pages, [LETTER, LETTER]);
        assert_eq!(data.journals(), 1);
        // Saved, nothing is left to recover.
        later.save(undone.doc, None).unwrap();
        assert_eq!(data.journals(), 0);
        assert_eq!(open_in(&data.documents(), &path).pages, [LETTER; 2]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_file_changed_since_cannot_have_them_and_discarding_deletes_them() {
        let data = DataFolder::new("stale");
        let path = write_pdf("b213-stale", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        edited_pages(&earlier, &mut doc, turn(1));
        drop(earlier);
        // Another program writes the file.
        std::fs::write(&path, letter_pdf(3)).unwrap();

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Stale);
        assert_eq!(
            later.recover(reopened.doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let discarded = opened_info(later.discard_recovered(reopened.doc).unwrap());
        assert_eq!(discarded.recovery, Recovery::None);
        assert_eq!(discarded.pages, [LETTER; 3]);
        assert_eq!(data.journals(), 0);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn saving_closing_or_undoing_everything_leaves_no_journal() {
        let data = DataFolder::new("gone");
        let path = write_pdf("b213-gone", &letter_pdf(2));
        let documents = data.documents();
        let mut doc = open_in(&documents, &path).doc;
        edited_pages(&documents, &mut doc, turn(0));
        assert_eq!(data.journals(), 1);
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(data.journals(), 0);
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!(data.journals(), 1);
        // Closing a tab with changes: the page asked first, and they are discarded.
        documents.close(tab_of(&documents, doc)).unwrap();
        assert_eq!(data.journals(), 0);
        // The window closes without saving them.
        let mut doc = open_in(&documents, &path).doc;
        edited_pages(&documents, &mut doc, turn(0));
        documents.discard_unsaved();
        assert_eq!(data.journals(), 0);
        // Saved as another file.
        let mut doc = open_in(&documents, &path).doc;
        edited_pages(&documents, &mut doc, turn(1));
        let copy = path.with_file_name(format!("b213-gone-copy-{}.pdf", std::process::id()));
        documents.save(doc, Some(copy.clone())).unwrap();
        assert_eq!(data.journals(), 0);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn edits_not_answered_about_stay_until_the_list_is_cleared() {
        let data = DataFolder::new("unanswered");
        let path = write_pdf("b213-unanswered", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        edited_pages(&earlier, &mut doc, turn(0));
        drop(earlier);

        let later = data.documents();
        let offered = open_in(&later, &path);
        assert_eq!(offered.recovery, Recovery::Available);
        later.close(tab_of(&later, offered.doc)).unwrap();
        assert_eq!(data.journals(), 1);
        // Offered again the next time the file opens.
        let again = open_in(&later, &path);
        assert_eq!(again.recovery, Recovery::Available);
        // Clearing the recent files list leaves what an open tab uses, and takes the rest.
        later.clear_unused_journals();
        assert_eq!(data.journals(), 1);
        later.close(tab_of(&later, again.doc)).unwrap();
        later.clear_unused_journals();
        assert_eq!(data.journals(), 0);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_documents_own_edits_come_first() {
        let data = DataFolder::new("own");
        let path = write_pdf("b213-own", &letter_pdf(2));
        data.leave(
            &path,
            serde_json::json!([{ "kind": "deletePages", "pages": [1] }]),
        );
        let documents = data.documents();
        let offered = open_in(&documents, &path);
        assert_eq!(offered.recovery, Recovery::Available);
        let mut doc = offered.doc;
        edited_pages(&documents, &mut doc, turn(0));
        // Those were made on the file, not on the edits offered.
        assert_eq!(
            documents.recover(doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(opened_info(documents.recover(doc).unwrap()).pages, [LETTER]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_journal_is_checked_against_its_file_before_anything_is_made() {
        let data = DataFolder::new("unfit");
        // The file has two pages.
        let path = write_pdf("b213-unfit", &letter_pdf(2));
        data.leave(
            &path,
            serde_json::json!([{ "kind": "deletePages", "pages": [4] }]),
        );
        let documents = data.documents();
        let opened = open_in(&documents, &path);
        assert_eq!(opened.recovery, Recovery::Stale);
        assert_eq!(
            documents.recover(opened.doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(opened.pages, [LETTER; 2]);
        std::fs::remove_file(path).ok();

        // An author who forbids changing pages (RC4, /P without the modify bit) is obeyed.
        let data = DataFolder::new("forbidden");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        data.leave(
            &path,
            serde_json::json!([{ "kind": "rotatePages", "pages": [0], "by": "cw90" }]),
        );
        let (_, info) = unlocked(&data.documents(), &path);
        assert!(!info.permissions.modify && !info.permissions.assemble);
        assert_eq!(info.recovery, Recovery::Stale);
    }

    #[test]
    fn a_document_lost_with_its_password_gets_its_edits_back_once_it_opens_again() {
        let data = DataFolder::new("password");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-aes256.pdf");
        let path = write_pdf("b213-password", &std::fs::read(source).unwrap());
        let documents = data.documents();
        let (tab, opened) = unlocked(&documents, &path);
        let edited = opened_info(
            documents
                .apply_edit(&EditArgs {
                    doc: opened.doc,
                    edit: turn(0),
                })
                .unwrap(),
        );
        // The worker dies; the password was not kept, so the tab asks for it again.
        let pid = documents.worker_id(edited.doc).expect("worker running");
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .unwrap();
        documents
            .render(&args(edited.doc, 0, 0.5, Rotation::None))
            .unwrap_err();
        assert!(matches!(
            documents.snapshot()[..],
            [OpenEvent::PasswordNeeded { .. }]
        ));
        assert_eq!(data.journals(), 1);
        // Given it, the file opens with the edit made again, without asking.
        let reopened = std::cell::RefCell::new(None);
        documents
            .unlock(tab, Password::new("user".to_owned()), false, &|event| {
                *reopened.borrow_mut() = Some(event);
            })
            .unwrap();
        let reopened = opened_info(reopened.into_inner().unwrap());
        assert_eq!(reopened.pages, edited.pages);
        assert!(reopened.unsaved && reopened.can_undo);
        assert_eq!(reopened.recovery, Recovery::None);
        assert_eq!(data.journals(), 1);
        documents.close(tab).unwrap();
        assert_eq!(data.journals(), 0);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn annotations_are_added_changed_removed_and_undone_like_any_edit() {
        let (documents, info, path) = open("annotations", 2);
        let mut doc = info.doc;
        let quad = |x: f32| Quad {
            ul: Point { x, y: 100.0 },
            ur: Point {
                x: x + 100.0,
                y: 100.0,
            },
            ll: Point { x, y: 120.0 },
            lr: Point {
                x: x + 100.0,
                y: 120.0,
            },
        };
        let edit = |doc: &mut DocumentId, edit: Edit| {
            *doc = opened_info(documents.apply_edit(&EditArgs { doc: *doc, edit }).unwrap()).doc;
        };
        edit(
            &mut doc,
            Edit::AddHighlight {
                marks: vec![HighlightMark {
                    page: 0,
                    quads: vec![quad(72.0)],
                }],
                color: HighlightColor::Blue,
            },
        );
        edit(
            &mut doc,
            Edit::AddNote {
                page: 1,
                at: Point { x: 300.0, y: 300.0 },
                text: "附註".to_owned(),
            },
        );
        let [highlight] = &documents.page_annotations(doc, 0).unwrap()[..] else {
            panic!("one highlight")
        };
        assert_eq!(highlight.color, Some(HighlightColor::Blue));
        let [note] = &documents.page_annotations(doc, 1).unwrap()[..] else {
            panic!("one note")
        };
        assert_eq!(note.text.as_deref(), Some("附註"));
        let (highlight, note) = (highlight.id, note.id);

        edit(
            &mut doc,
            Edit::SetNoteText {
                page: 1,
                annotation: note,
                text: "改過".to_owned(),
            },
        );
        edit(
            &mut doc,
            Edit::DeleteAnnotation {
                page: 0,
                annotation: highlight,
            },
        );
        assert!(documents.page_annotations(doc, 0).unwrap().is_empty());
        assert_eq!(
            documents.page_annotations(doc, 1).unwrap()[0]
                .text
                .as_deref(),
            Some("改過")
        );
        // Undo opens the document again and makes the earlier edits again: the same numbers.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(documents.page_annotations(doc, 0).unwrap()[0].id, highlight);

        // An annotation that is not there, on a page that is not either, or text that is not
        // only text, never reaches a document.
        let refused = |edit: Edit| {
            documents
                .apply_edit(&EditArgs { doc, edit })
                .unwrap_err()
                .code
        };
        assert_eq!(
            refused(Edit::DeleteAnnotation {
                page: 0,
                annotation: AnnotationId(9_999),
            }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(Edit::DeleteAnnotation {
                page: 2,
                annotation: highlight,
            }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(Edit::AddNote {
                page: 0,
                at: Point { x: 1.0, y: 1.0 },
                text: "a\u{202E}b".to_owned(),
            }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(documents.page_annotations(doc, 0).unwrap().len(), 1);
        assert_eq!(
            documents.page_annotations(doc, 2).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn some_pages_are_saved_as_a_document_of_their_own_and_the_document_is_not_changed() {
        let (documents, info, path) = open("split-pages", 6);
        let doc = edit_then_split_setup(&documents, info.doc);
        let destination =
            std::env::temp_dir().join(format!("b206-split-{}.pdf", std::process::id()));
        std::fs::remove_file(&destination).ok();
        // Pages 2, 3 and 5 of what the document is now, edits included: one was deleted before.
        documents
            .save_pages(doc.doc, &[1, 2, 4], &destination)
            .unwrap();
        let copy = open_in(&Documents::new(worker()), &destination);
        assert_eq!(copy.pages, [LETTER, LETTER, LETTER]);
        assert!(!copy.unsaved);
        // The document keeps its pages, its edits and its file.
        let after = documents.document_info(doc.doc).unwrap();
        assert_eq!(after.pages.len(), 5);
        assert!(after.unsaved);

        // A page that is not there, one named twice, none, the document's own file or one that
        // is not writable: nothing is written and nothing changes.
        let refused =
            |pages: &[u32], to: &Path| documents.save_pages(doc.doc, pages, to).unwrap_err().code;
        let other = std::env::temp_dir().join(format!("b206-never-{}.pdf", std::process::id()));
        for wrong in [&[][..], &[9], &[1, 1]] {
            assert_eq!(refused(wrong, &other), ErrorCode::InvalidArgument);
        }
        assert_eq!(refused(&[0], &path), ErrorCode::InvalidArgument);
        assert!(!other.exists());
        std::fs::remove_file(&destination).ok();
        std::fs::remove_file(path).ok();
    }

    /// Deletes the first page of the document, which has pages of a mixed size: the edit the
    /// split must take into account.
    fn edit_then_split_setup(documents: &Documents, doc: DocumentId) -> DocumentInfo {
        opened_info(
            documents
                .apply_edit(&EditArgs {
                    doc,
                    edit: Edit::DeletePages { pages: vec![0] },
                })
                .unwrap(),
        )
    }

    #[test]
    fn an_encrypted_document_or_one_that_cannot_be_copied_is_not_split() {
        let documents = Documents::new(worker());
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign");
        let destination =
            std::env::temp_dir().join(format!("b206-refused-{}.pdf", std::process::id()));
        // Encrypted (the user password is "user").
        let (_, info) = unlocked(&documents, &corpus.join("encrypted-rc4-40.pdf"));
        assert!(info.encrypted);
        assert_eq!(
            documents
                .save_pages(info.doc, &[0], &destination)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // The author does not allow copying.
        let info = open_in(&documents, &corpus.join("restricted-no-copy-no-print.pdf"));
        assert!(!info.permissions.copy);
        assert_eq!(
            documents
                .save_pages(info.doc, &[0], &destination)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert!(!destination.exists());
    }

    #[test]
    fn drawings_and_stamps_are_added_moved_removed_and_undone_like_any_annotation() {
        let (documents, info, path) = open("drawings", 2);
        let mut doc = info.doc;
        let edit = |doc: &mut DocumentId, edit: Edit| {
            *doc = opened_info(documents.apply_edit(&EditArgs { doc: *doc, edit }).unwrap()).doc;
        };
        edit(
            &mut doc,
            Edit::AddInk {
                page: 0,
                strokes: vec![vec![
                    Point { x: 100.0, y: 100.0 },
                    Point { x: 200.0, y: 150.0 },
                ]],
                color: InkColor::Green,
                width: InkWidth::Thin,
            },
        );
        edit(
            &mut doc,
            Edit::AddStamp {
                page: 1,
                rect: Rect {
                    x0: 72.0,
                    y0: 72.0,
                    x1: 262.0,
                    y1: 122.0,
                },
                stamp: StampName::Draft,
            },
        );
        let [drawing] = &documents.page_annotations(doc, 0).unwrap()[..] else {
            panic!("one drawing")
        };
        assert_eq!(drawing.kind, AnnotationKind::Ink);
        let (drawing, before) = (drawing.id, drawing.rect);
        let [stamp] = &documents.page_annotations(doc, 1).unwrap()[..] else {
            panic!("one stamp")
        };
        assert_eq!(stamp.kind, AnnotationKind::Stamp);
        let stamp = stamp.id;

        // Moved a little to the right and down.
        let moved = Rect {
            x0: before.x0 + 40.0,
            y0: before.y0 + 25.0,
            x1: before.x1 + 40.0,
            y1: before.y1 + 25.0,
        };
        edit(
            &mut doc,
            Edit::SetAnnotationRect {
                page: 0,
                annotation: drawing,
                rect: moved,
            },
        );
        let at = documents.page_annotations(doc, 0).unwrap()[0].rect;
        assert!((at.x0 - moved.x0).abs() < 0.6 && (at.y1 - moved.y1).abs() < 0.6);
        // Undone: where it was, with the same number.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        let back = documents.page_annotations(doc, 0).unwrap();
        assert_eq!(back[0].id, drawing);
        assert!((back[0].rect.x0 - before.x0).abs() < 0.6);

        edit(
            &mut doc,
            Edit::DeleteAnnotation {
                page: 1,
                annotation: stamp,
            },
        );
        assert!(documents.page_annotations(doc, 1).unwrap().is_empty());

        // What is not there, a page that is not either, or a rectangle too small to grab, never
        // reaches a document.
        let refused = |edit: Edit| {
            documents
                .apply_edit(&EditArgs { doc, edit })
                .unwrap_err()
                .code
        };
        let rect = |side: f32| Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 10.0 + side,
            y1: 10.0 + side,
        };
        for wrong in [
            Edit::SetAnnotationRect {
                page: 0,
                annotation: AnnotationId(9_999),
                rect: rect(50.0),
            },
            Edit::SetAnnotationRect {
                page: 0,
                annotation: drawing,
                rect: rect(1.0),
            },
            Edit::SetAnnotationRect {
                page: 2,
                annotation: drawing,
                rect: rect(50.0),
            },
            Edit::AddStamp {
                page: 2,
                rect: rect(50.0),
                stamp: StampName::Final,
            },
            Edit::AddInk {
                page: 0,
                strokes: vec![],
                color: InkColor::Black,
                width: InkWidth::Thick,
            },
        ] {
            assert_eq!(refused(wrong), ErrorCode::InvalidArgument);
        }
        assert_eq!(documents.page_annotations(doc, 0).unwrap().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_author_who_forbids_annotations_is_obeyed() {
        // RC4, /P without the annotation bit (tests/corpus/generate.py).
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        let (_, info) = unlocked(&documents, &path);
        assert!(!info.permissions.annotate);
        let rect = Rect {
            x0: 72.0,
            y0: 72.0,
            x1: 262.0,
            y1: 122.0,
        };
        for forbidden in [
            Edit::AddNote {
                page: 0,
                at: Point { x: 72.0, y: 72.0 },
                text: "no".to_owned(),
            },
            Edit::AddInk {
                page: 0,
                strokes: vec![vec![Point { x: 72.0, y: 72.0 }]],
                color: InkColor::Black,
                width: InkWidth::Thin,
            },
            Edit::AddStamp {
                page: 0,
                rect,
                stamp: StampName::Approved,
            },
            Edit::SetAnnotationRect {
                page: 0,
                annotation: AnnotationId(5),
                rect,
            },
            Edit::SetPictureAngle {
                page: 0,
                annotation: AnnotationId(5),
                degrees: 30.0,
            },
        ] {
            assert_eq!(
                documents
                    .apply_edit(&EditArgs {
                        doc: info.doc,
                        edit: forbidden,
                    })
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument
            );
        }
        // Listing them is not editing them.
        assert!(documents.page_annotations(info.doc, 0).is_ok());
    }

    /// A copy of the form sample in a file of its own, opened in a window of its own.
    fn open_form(name: &str) -> (Documents, DocumentInfo, PathBuf) {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/corpus/benign/form-fields.pdf");
        open_bytes(name, &std::fs::read(source).unwrap())
    }

    fn field(documents: &Documents, doc: DocumentId, label: &str) -> FormField {
        documents
            .page_fields(doc, 0)
            .unwrap()
            .into_iter()
            .find(|field| field.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no field {label}"))
    }

    fn fill(documents: &Documents, doc: &mut DocumentId, label: &str, value: &str) -> DocumentInfo {
        let id = field(documents, *doc, label).id;
        let info = opened_info(
            documents
                .apply_edit(&EditArgs {
                    doc: *doc,
                    edit: Edit::SetFieldValue {
                        page: 0,
                        field: id,
                        value: value.to_owned(),
                    },
                })
                .unwrap(),
        );
        *doc = info.doc;
        info
    }

    #[test]
    fn a_form_is_filled_in_undone_saved_and_flattened_like_any_edit() {
        let (documents, info, path) = open_form("b209-form");
        let mut doc = info.doc;
        assert!(info.has_form);
        assert_eq!(field(&documents, doc, "Your name").value, "Jane Q. Public");

        let filled = fill(&documents, &mut doc, "Your name", "林 小明");
        assert!(filled.unsaved && filled.can_undo);
        fill(&documents, &mut doc, "I agree", "Yes");
        assert_eq!(field(&documents, doc, "Your name").value, "林 小明");
        assert_eq!(field(&documents, doc, "I agree").value, "Yes");

        // Undo opens the document again and makes the edits before it again: the same numbers.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(field(&documents, doc, "I agree").value, "Off");
        assert_eq!(field(&documents, doc, "Your name").value, "林 小明");
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!(field(&documents, doc, "I agree").value, "Yes");

        // Saved, the values are in the file.
        documents.save(doc, None).unwrap();
        let other = Documents::new(worker());
        let again = open_in(&other, &path);
        assert_eq!(field(&other, again.doc, "Your name").value, "林 小明");
        assert_eq!(field(&other, again.doc, "I agree").value, "Yes");

        // Flattened, there is nothing left to fill in; undo brings the fields back.
        let flattened = opened_info(
            documents
                .apply_edit(&EditArgs {
                    doc,
                    edit: Edit::FlattenForm,
                })
                .unwrap(),
        );
        assert!(documents.page_fields(flattened.doc, 0).unwrap().is_empty());
        assert!(!flattened.has_form);
        let undone = opened_info(documents.undo(flattened.doc, None).unwrap());
        assert!(undone.has_form);
        assert_eq!(field(&documents, undone.doc, "Your name").value, "林 小明");
        // Flattened and saved, the file has no form, and neither has the document that saved it.
        let flattened = opened_info(documents.redo(undone.doc).unwrap());
        documents.save(flattened.doc, None).unwrap();
        assert!(!opened_info(documents.snapshot().remove(0)).has_form);
        assert!(!open_in(&Documents::new(worker()), &path).has_form);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn what_a_form_cannot_take_never_reaches_the_document() {
        let (documents, info, path) = open_form("b209-refused");
        let doc = info.doc;
        let name = field(&documents, doc, "Your name").id;
        let refused = |edit: Edit| {
            documents
                .apply_edit(&EditArgs { doc, edit })
                .unwrap_err()
                .code
        };
        let set = |page: u32, field: FieldId, value: &str| Edit::SetFieldValue {
            page,
            field,
            value: value.to_owned(),
        };
        // Text that is not text, a page that is not there, a field that is not on the page, a
        // value the field cannot have.
        assert_eq!(
            refused(set(0, name, "a\u{202E}b")),
            ErrorCode::InvalidArgument
        );
        assert_eq!(refused(set(5, name, "x")), ErrorCode::InvalidArgument);
        assert_eq!(
            refused(set(0, FieldId(9_999), "x")),
            ErrorCode::InvalidArgument
        );
        let locked = field(&documents, doc, "locked").id;
        assert_eq!(refused(set(0, locked, "x")), ErrorCode::InvalidArgument);
        assert_eq!(
            refused(set(0, name, "two\nlines")),
            ErrorCode::InvalidArgument
        );
        assert_eq!(field(&documents, doc, "Your name").value, "Jane Q. Public");
        // None of that was an edit.
        let info = opened_info(documents.snapshot().remove(0));
        assert!(!info.unsaved && !info.can_undo);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_author_who_forbids_filling_in_forms_is_obeyed() {
        // RC4, revision 2, /P without bit 6, which covers annotating and filling in forms.
        let documents = Documents::new(worker());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        let (_, info) = unlocked(&documents, &path);
        assert!(!info.permissions.fill_forms);
        for edit in [
            Edit::SetFieldValue {
                page: 0,
                field: FieldId(1),
                value: "x".to_owned(),
            },
            Edit::FlattenForm,
        ] {
            assert_eq!(
                documents
                    .apply_edit(&EditArgs {
                        doc: info.doc,
                        edit,
                    })
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidArgument
            );
        }
        // Listing the fields is not filling them in.
        assert!(documents.page_fields(info.doc, 0).is_ok());
    }

    fn corpus_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus")
            .join(name)
    }

    /// An empty folder of its own for a batch test (B3-04).
    fn batch_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("b304-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A folder of its own, empty, for the files a test writes.
    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("b303-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn run_batch(
        documents: &Documents,
        files: &[PathBuf],
        folder: &Path,
        operation: &BatchOperation,
        stop: bool,
    ) -> (
        ipc_contract::types::BatchReport,
        Vec<ipc_contract::types::BatchEvent>,
    ) {
        let stop = std::sync::atomic::AtomicBool::new(stop);
        let events = std::cell::RefCell::new(Vec::new());
        let report = crate::batch::run(documents, files, folder, operation, &stop, &|event| {
            events.borrow_mut().push(event)
        });
        (report, events.into_inner())
    }

    #[test]
    fn a_batch_copies_each_file_and_one_that_fails_stops_none_of_the_others() {
        use ipc_contract::types::{BatchEvent, BatchOutcome, BatchReport};

        let documents = Documents::new(worker());
        let sources = batch_folder("sources");
        let broken = sources.join("broken.pdf");
        std::fs::write(&broken, b"%PDF-1.7 not really").unwrap();
        let files = [
            corpus_path("benign/metadata-full.pdf"),
            corpus_path("benign/encrypted-aes256.pdf"),
            broken.clone(),
            corpus_path("benign/single-page.pdf"),
        ];
        let before: Vec<Vec<u8>> = files
            .iter()
            .map(|file| std::fs::read(file).unwrap())
            .collect();
        let out = batch_folder("out");
        let (report, events) = run_batch(
            &documents,
            &files,
            &out,
            &BatchOperation::RemoveMetadata,
            false,
        );
        assert_eq!(
            report,
            BatchReport {
                done: 2,
                failed: 2,
                skipped: 0
            }
        );
        assert_eq!(
            events[0],
            BatchEvent::Started {
                files: vec![
                    "metadata-full.pdf".to_owned(),
                    "encrypted-aes256.pdf".to_owned(),
                    "broken.pdf".to_owned(),
                    "single-page.pdf".to_owned()
                ]
            }
        );
        let outcomes: Vec<BatchOutcome> = events[1..]
            .iter()
            .map(|event| match event {
                BatchEvent::File { outcome, .. } => outcome.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            outcomes[0],
            BatchOutcome::Done {
                output: "metadata-full（隱私匯出）.pdf".to_owned()
            }
        );
        // An encrypted file asks for a password a batch does not have.
        assert_eq!(
            outcomes[1],
            BatchOutcome::Failed {
                error: ErrorCode::Encrypted
            }
        );
        assert!(
            matches!(outcomes[2], BatchOutcome::Failed { .. }),
            "{outcomes:?}"
        );
        assert_eq!(
            outcomes[3],
            BatchOutcome::Done {
                output: "single-page（隱私匯出）.pdf".to_owned()
            }
        );
        // The copies, and nothing else: no file the copies were made in is left.
        assert_eq!(
            names_in(&out),
            [
                "metadata-full（隱私匯出）.pdf",
                "single-page（隱私匯出）.pdf"
            ]
        );
        let copy = std::fs::read(out.join("metadata-full（隱私匯出）.pdf")).unwrap();
        assert!(
            !copy
                .windows(22)
                .any(|window| window == b"Jane Q. Private-Author")
        );
        // The files themselves are as they were.
        for (file, bytes) in files.iter().zip(&before) {
            assert_eq!(&std::fs::read(file).unwrap(), bytes, "{}", file.display());
        }

        // Again into the same folder: the copies there stay, the new ones get a number.
        let (report, _) = run_batch(
            &documents,
            &files[..1],
            &out,
            &BatchOperation::RemoveMetadata,
            false,
        );
        assert_eq!(report.done, 1);
        assert_eq!(
            names_in(&out),
            [
                "metadata-full（隱私匯出） (2).pdf",
                "metadata-full（隱私匯出）.pdf",
                "single-page（隱私匯出）.pdf"
            ]
        );
        // Stopped before it began: every file is skipped and nothing is written.
        let (report, events) = run_batch(
            &documents,
            &files,
            &out,
            &BatchOperation::RemoveMetadata,
            true,
        );
        assert_eq!(
            report,
            BatchReport {
                done: 0,
                failed: 0,
                skipped: 4
            }
        );
        assert_eq!(events.len(), 5);
        assert_eq!(names_in(&out).len(), 3);
        std::fs::remove_dir_all(&out).ok();
        std::fs::remove_dir_all(&sources).ok();
    }

    #[test]
    fn a_batch_of_encrypted_copies_opens_only_with_the_password() {
        let documents = Documents::new(worker());
        let out = batch_folder("encrypted");
        let operation = BatchOperation::Encrypt {
            open_password: Some(Password::new("user".to_owned())),
            permissions_password: None,
            restrictions: Restrictions::default(),
        };
        let files = [
            corpus_path("benign/single-page.pdf"),
            corpus_path("benign/multi-page-10.pdf"),
        ];
        let (report, _) = run_batch(&documents, &files, &out, &operation, false);
        assert_eq!(report.done, 2);
        assert_eq!(
            names_in(&out),
            ["multi-page-10（已加密）.pdf", "single-page（已加密）.pdf"]
        );
        // The copy needs the password, and opens with it.
        let copy = out.join("multi-page-10（已加密）.pdf");
        let asked = std::cell::RefCell::new(false);
        let report_event = |event: OpenEvent| {
            if matches!(event, OpenEvent::PasswordNeeded { .. }) {
                *asked.borrow_mut() = true;
            }
        };
        for tab in documents.add(std::slice::from_ref(&copy), &report_event) {
            documents.load(tab, &report_event);
        }
        assert!(*asked.borrow());
        let (_, info) = unlocked(&documents, &copy);
        assert_eq!(info.pages.len(), 10);
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn a_batch_takes_out_what_cameras_wrote_into_the_pictures_and_keeps_the_rest() {
        use ipc_contract::types::{BatchEvent, BatchOutcome};

        let documents = Documents::new(worker());
        let out = batch_folder("exif");
        let files = [
            corpus_path("benign/photo-exif.pdf"),
            corpus_path("benign/signed.pdf"),
            corpus_path("benign/metadata-full.pdf"),
        ];
        let (report, events) =
            run_batch(&documents, &files, &out, &BatchOperation::RemoveExif, false);
        assert_eq!((report.done, report.failed), (2, 1));
        // A signed document's pictures cannot change: its signatures would not hold.
        assert!(events.contains(&BatchEvent::File {
            index: 1,
            outcome: BatchOutcome::Failed {
                error: ErrorCode::InvalidArgument
            }
        }));
        assert_eq!(
            names_in(&out),
            [
                "metadata-full（已清除 EXIF）.pdf",
                "photo-exif（已清除 EXIF）.pdf"
            ]
        );
        let has = |bytes: &[u8], needle: &[u8]| {
            bytes.windows(needle.len()).any(|window| window == needle)
        };
        let photo = std::fs::read(out.join("photo-exif（已清除 EXIF）.pdf")).unwrap();
        for private in [&b"Canon"[..], b"Exif", b"Jane Q. Private-Author"] {
            assert!(
                !has(&photo, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // The same page, opened as a document.
        let info = open_in(&documents, &out.join("photo-exif（已清除 EXIF）.pdf"));
        assert_eq!(info.pages.len(), 1);
        // Only the pictures': the document's own metadata is the privacy export's (B2-03), and
        // stays. The picture of that sample loses its XMP.
        let metadata = std::fs::read(out.join("metadata-full（已清除 EXIF）.pdf")).unwrap();
        assert!(has(&metadata, b"SecretWriter 9.1"));
        assert_eq!(
            metadata
                .windows(b"<x:xmpmeta".len())
                .filter(|window| window == b"<x:xmpmeta")
                .count(),
            2,
            "the catalog's and the page's XMP stay, the picture's goes"
        );
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn a_pdf_is_made_of_files_and_opens_as_a_document() {
        let documents = Documents::new(worker());
        let folder = folder("made");
        let made = documents
            .create_pdf(
                &[(corpus_path("sources/letter.docx"), CreateKind::Docx)],
                &folder.join("letter.pdf"),
            )
            .unwrap();
        assert!(made.pages >= 1);
        assert_eq!(made.kind, CreateKind::Docx);
        let info = open_in(&documents, &folder.join("letter.pdf"));
        assert_eq!(info.pages.len() as u32, made.pages);
        assert!(
            documents
                .page_text(info.doc, 0)
                .unwrap()
                .lines
                .iter()
                .any(|line| line.text.contains("First paragraph"))
        );

        // Pictures go together, a page each.
        let pictures = ["images/stamp-exif.jpg", "images/stamp-metadata.png"]
            .map(|name| (corpus_path(name), CreateKind::Image));
        let made = documents
            .create_pdf(&pictures, &folder.join("pictures.pdf"))
            .unwrap();
        assert_eq!((made.pages, made.kind), (2, CreateKind::Image));
        assert_eq!(
            open_in(&documents, &folder.join("pictures.pdf"))
                .pages
                .len(),
            2
        );
        // Nothing else is left in the folder, no file the PDF was made in.
        assert_eq!(names_in(&folder), ["letter.pdf", "pictures.pdf"]);
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_pdf_never_replaces_a_file_it_is_made_of_and_a_failure_leaves_the_destination_alone() {
        let documents = Documents::new(worker());
        let folder = folder("refused");
        let text = folder.join("notes.txt");
        std::fs::write(&text, "some words").unwrap();
        let error = documents
            .create_pdf(&[(text.clone(), CreateKind::Text)], &text)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(std::fs::read(&text).unwrap(), b"some words");

        // A file that is not what its name says: an error, and the file at the destination is
        // the user's still.
        let existing = folder.join("existing.pdf");
        std::fs::write(&existing, b"the user's own file").unwrap();
        let broken = folder.join("broken.docx");
        std::fs::write(&broken, b"not a zip").unwrap();
        let error = documents
            .create_pdf(&[(broken.clone(), CreateKind::Docx)], &existing)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Corrupted);
        assert_eq!(std::fs::read(&existing).unwrap(), b"the user's own file");

        // A file that is gone, a folder, no file, and a file that is too large.
        let gone = folder.join("gone.txt");
        let error = documents
            .create_pdf(&[(gone, CreateKind::Text)], &folder.join("a.pdf"))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unreadable);
        let error = documents
            .create_pdf(&[(folder.clone(), CreateKind::Text)], &folder.join("a.pdf"))
            .unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::InvalidArgument | ErrorCode::Unreadable
        ));
        assert_eq!(
            documents
                .create_pdf(&[], &folder.join("a.pdf"))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let large = folder.join("large.txt");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES as u64 + 1)
            .unwrap();
        let error = documents
            .create_pdf(&[(large.clone(), CreateKind::Text)], &folder.join("a.pdf"))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::TooLarge);

        // A destination marked read-only is not replaced either.
        let mut permissions = std::fs::metadata(&existing).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&existing, permissions.clone()).unwrap();
        let error = documents
            .create_pdf(
                &[(corpus_path("sources/text.txt"), CreateKind::Text)],
                &existing,
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ReadOnly);
        // Windows only (the crate does not build elsewhere): no world-writable file results.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&existing, permissions).unwrap();

        // No file made on the way is left.
        assert_eq!(
            names_in(&folder),
            ["broken.docx", "existing.pdf", "large.txt", "notes.txt"]
        );
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_pdf_made_of_a_file_that_asks_for_too_much_ends_in_an_error_and_the_app_goes_on() {
        let documents = Documents::new(worker());
        let folder = folder("hostile");
        let error = documents
            .create_pdf(
                &[(
                    corpus_path("sources/bomb-declared-size.docx"),
                    CreateKind::Docx,
                )],
                &folder.join("bomb.pdf"),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Corrupted);
        assert!(names_in(&folder).is_empty());
        let made = documents
            .create_pdf(
                &[(corpus_path("sources/web-page.html"), CreateKind::Html)],
                &folder.join("page.pdf"),
            )
            .unwrap();
        assert!(made.pages >= 1);
        std::fs::remove_dir_all(&folder).ok();
    }

    fn insert_pages(at: u32, source: ipc_contract::types::SourceId) -> Edit {
        Edit::InsertPages { at, source }
    }

    #[test]
    fn the_pages_of_another_file_are_an_edit_that_undo_redo_and_saving_follow() {
        let (documents, info, path) = open("merge-edit", 3);
        let taken = documents
            .prepare_pages_source(info.doc, &corpus_path("benign/mixed-page-sizes.pdf"), None)
            .unwrap();
        assert_eq!(taken.pages, 4);
        // Choosing the file is not an edit.
        assert!(!documents.document_info(info.doc).unwrap().unsaved);

        let mut doc = info.doc;
        let pages = edited_pages(&documents, &mut doc, insert_pages(1, taken.source));
        assert_eq!(pages.len(), 7);
        assert_eq!(pages[0], LETTER);
        assert_eq!(pages[5], LETTER);
        assert_ne!(pages[1], LETTER);
        // Undone (the document is opened again from its file and the other edits are made again),
        // and made again from the copy the main process keeps.
        let undone = opened_info(documents.undo(doc, None).unwrap());
        assert_eq!(undone.pages, [LETTER; 3]);
        let redone = opened_info(documents.redo(undone.doc).unwrap());
        assert_eq!(redone.pages, pages);
        // Another edit after it, then undone and redone: the pages are still there.
        let mut doc = redone.doc;
        let turned = edited_pages(&documents, &mut doc, turn(0));
        assert_eq!(turned.len(), 7);
        let back = opened_info(documents.undo(doc, None).unwrap());
        assert_eq!(back.pages, pages);
        // A worker that died is replaced, and the pages are taken again in the new one.
        documents
            .with_document(back.doc, |document| {
                document.lost = true;
                Ok(())
            })
            .unwrap();
        assert!(!documents.page_text(back.doc, 4).unwrap().lines.is_empty());

        // Saved as a copy: the copy has them.
        let copy = std::env::temp_dir().join(format!("b206-{}-copy.pdf", std::process::id()));
        let _ = std::fs::remove_file(&copy);
        documents.save(back.doc, Some(copy.clone())).unwrap();
        assert_eq!(first_page_count(&copy), 7);
        std::fs::remove_file(&copy).ok();
        std::fs::remove_file(path).ok();
    }

    /// How many pages the file at `path` has, opened in a fresh window.
    fn first_page_count(path: &Path) -> usize {
        open_in(&Documents::new(worker()), path).pages.len()
    }

    #[test]
    fn what_was_active_in_the_file_taken_from_is_on_the_banner_only_while_its_pages_are_in() {
        let (documents, info, path) = open("merge-banner", 2);
        let taken = documents
            .prepare_pages_source(info.doc, &corpus_path("malicious/openaction-js.pdf"), None)
            .unwrap();
        // Choosing the file says nothing yet: it is nobody's pages.
        assert!(
            documents
                .document_info(info.doc)
                .unwrap()
                .security
                .findings
                .is_empty()
        );
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, insert_pages(2, taken.source));
        let now = documents.document_info(doc).unwrap();
        assert!(
            now.security
                .findings
                .iter()
                .any(|finding| finding.kind == ipc_contract::types::FindingKind::OpenAction),
            "{:?}",
            now.security
        );
        // Undone, it is not there; made again, it is.
        let undone = opened_info(documents.undo(doc, None).unwrap());
        assert!(undone.security.findings.is_empty());
        let redone = opened_info(documents.redo(undone.doc).unwrap());
        assert!(!redone.security.findings.is_empty());
        // Saved, the file has the pages and nothing active, and so the banner has nothing.
        let copy = std::env::temp_dir().join(format!("b206-{}-banner.pdf", std::process::id()));
        let _ = std::fs::remove_file(&copy);
        let saved = opened_info(documents.save(redone.doc, Some(copy.clone())).unwrap().1);
        assert!(saved.security.findings.is_empty());
        let reopened = open_in(&Documents::new(worker()), &copy);
        assert!(reopened.security.findings.is_empty());
        assert_eq!(reopened.pages.len(), 3);
        std::fs::remove_file(&copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_encrypted_file_waits_for_its_password_and_the_author_may_forbid_taking_pages() {
        let (documents, info, path) = open("merge-password", 1);
        let aes = corpus_path("benign/encrypted-aes256.pdf");
        assert_eq!(
            documents
                .prepare_pages_source(info.doc, &aes, None)
                .unwrap_err()
                .code,
            ErrorCode::Encrypted
        );
        // A wrong password is no better, and the file goes on waiting.
        assert_eq!(
            documents
                .unlock_pages_source(info.doc, Password::new("wrong".to_owned()))
                .unwrap_err()
                .code,
            ErrorCode::Encrypted
        );
        let taken = documents
            .unlock_pages_source(info.doc, Password::new("user".to_owned()))
            .unwrap();
        assert_eq!(taken.pages, 1);
        // Nothing waits any more.
        assert_eq!(
            documents
                .unlock_pages_source(info.doc, Password::new("user".to_owned()))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // Taken, it needs no password: it is a plain copy.
        let mut doc = info.doc;
        assert_eq!(
            edited_pages(&documents, &mut doc, insert_pages(1, taken.source)).len(),
            2
        );

        // What the author of a file forbids, the file's pages are not taken either; the owner
        // password is the author's.
        let restricted = corpus_path("benign/restricted-open-password.pdf");
        assert!(
            documents
                .prepare_pages_source(doc, &restricted, None)
                .is_err()
        );
        assert_eq!(
            documents
                .unlock_pages_source(doc, Password::new("user".to_owned()))
                .unwrap_err()
                .code,
            ErrorCode::NotAllowed
        );
        assert!(
            documents
                .unlock_pages_source(doc, Password::new("owner".to_owned()))
                .is_ok()
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn what_cannot_be_taken_from_or_put_into_is_refused() {
        let (documents, info, path) = open("merge-refused", 2);
        // Not a PDF, a folder, a file that is not there.
        for (what, wrong) in [
            ("manifest.json", ErrorCode::NotPdf),
            ("", ErrorCode::Unreadable),
            ("benign/missing.pdf", ErrorCode::Unreadable),
        ] {
            assert_eq!(
                documents
                    .prepare_pages_source(info.doc, &corpus_path(what), None)
                    .unwrap_err()
                    .code,
                wrong,
                "{what}"
            );
        }
        // An edit that names a file the document does not have, or a place that is not there.
        let taken = documents
            .prepare_pages_source(info.doc, &corpus_path("benign/single-page.pdf"), None)
            .unwrap();
        let refused = |at: u32, source| {
            documents
                .apply_edit(&EditArgs {
                    doc: info.doc,
                    edit: insert_pages(at, source),
                })
                .unwrap_err()
                .code
        };
        assert_eq!(
            refused(0, ipc_contract::types::SourceId(99)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(refused(3, taken.source), ErrorCode::InvalidArgument);
        // The document is as it was after any of it.
        assert_eq!(documents.document_info(info.doc).unwrap().pages.len(), 2);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_document_whose_author_forbids_changing_its_pages_takes_no_pages() {
        // RC4, revision 2, /P without the modify bit (tests/corpus/generate.py).
        let documents = Documents::new(worker());
        let (_, info) = unlocked(&documents, &corpus_path("benign/encrypted-rc4-40.pdf"));
        assert!(!info.permissions.modify && !info.permissions.assemble);
        assert_eq!(
            documents.check_can_insert_pages(info.doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            documents
                .prepare_pages_source(info.doc, &corpus_path("benign/single-page.pdf"), None)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    fn text_box_with(page: u32, model: TextBoxModelId) -> Edit {
        Edit::AddTextBox {
            page,
            rect: Rect {
                x0: 72.0,
                y0: 72.0,
                x1: 372.0,
                y1: 132.0,
            },
            model,
            style: TextBoxStyle {
                size: 12.0,
                color: Rgb { r: 0, g: 0, b: 0 },
                align: TextAlign::Left,
            },
        }
    }

    /// An installed TrueType font with Latin letters, as `Fonts` gives one: its name, the bytes
    /// of its file and which font of the file.
    fn arial() -> (String, Vec<u8>, u32) {
        let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let file = Path::new(&windows).join("Fonts").join("arial.ttf");
        let bytes = std::fs::read(file).expect("Windows has Arial");
        ("Arial".to_owned(), bytes, 0)
    }

    #[test]
    fn words_are_prepared_put_in_boxes_and_made_again_by_undo_redo_and_a_new_worker() {
        let (documents, info, path) = open("text-box-add", 2);
        let prepared = documents
            .prepare_text_box(info.doc, "Hello, Wörld", arial())
            .unwrap();
        assert!(!prepared.view_only);
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, text_box_with(0, prepared.model));
        let [boxed] = &documents.page_annotations(doc, 0).unwrap()[..] else {
            panic!("one text box")
        };
        assert_eq!(boxed.kind, AnnotationKind::TextBox);
        assert_eq!(boxed.text.as_deref(), Some("Hello, Wörld"));

        // The same words in five more boxes: the edit names them, the font is not sent again.
        for page in [0, 1, 0, 1, 0] {
            edited_pages(&documents, &mut doc, text_box_with(page, prepared.model));
        }
        let boxes =
            |doc: DocumentId, page: u32| documents.page_annotations(doc, page).unwrap().len();
        assert_eq!((boxes(doc, 0), boxes(doc, 1)), (4, 2));
        // Undone, and made again, from the words the main process keeps for them.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!((boxes(doc, 0), boxes(doc, 1)), (3, 2));
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!((boxes(doc, 0), boxes(doc, 1)), (4, 2));
        // A worker that died is replaced, and every box is made again in the new one.
        documents
            .with_document(doc, |document| {
                document.lost = true;
                Ok(())
            })
            .unwrap();
        assert_eq!((boxes(doc, 0), boxes(doc, 1)), (4, 2));

        // Saved: the subsets of the font, a few kilobytes each, and not the font six times.
        let copy = std::env::temp_dir().join(format!("b229-{}-copy.pdf", std::process::id()));
        let _ = std::fs::remove_file(&copy);
        documents.save(doc, Some(copy.clone())).unwrap();
        let size = std::fs::metadata(&copy).unwrap().len();
        assert!(size < 200_000, "{size} bytes");
        let reopened = open_in(&documents, &copy);
        assert_eq!(
            documents.page_annotations(reopened.doc, 1).unwrap()[0].kind,
            AnnotationKind::TextBox
        );
        std::fs::remove_file(&copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_font_or_a_text_that_cannot_be_set_makes_no_prepared_words_and_changes_nothing() {
        let (documents, info, path) = open("text-box-refused", 1);
        let (name, bytes, face) = arial();
        let refused = |text: &str, font: (String, Vec<u8>, u32)| {
            documents
                .prepare_text_box(info.doc, text, font)
                .unwrap_err()
                .code
        };
        let junk = ("Junk".to_owned(), b"not a font at all".to_vec(), 0);
        assert_eq!(refused("words", junk), ErrorCode::FontInvalid);
        let arial = || (name.clone(), bytes.clone(), face);
        assert_eq!(
            refused("words \u{e000}", arial()),
            ErrorCode::FontMissingGlyphs
        );
        assert_eq!(
            refused("\u{628}\u{644}", arial()),
            ErrorCode::TextNotSupported
        );
        assert_eq!(refused("  \n ", arial()), ErrorCode::InvalidArgument);
        // Words that nobody prepared are no edit.
        let error = documents
            .apply_edit(&EditArgs {
                doc: info.doc,
                edit: text_box_with(0, TextBoxModelId(99)),
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        // A page the document does not have.
        let prepared = documents
            .prepare_text_box(info.doc, "Hello", arial())
            .unwrap();
        let error = documents
            .apply_edit(&EditArgs {
                doc: info.doc,
                edit: text_box_with(5, prepared.model),
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert!(documents.unsaved_tabs().is_empty());

        // The author's permission to annotate comes first.
        let (_, forbidden) = unlocked(&documents, &corpus_path("benign/encrypted-rc4-40.pdf"));
        assert!(!forbidden.permissions.annotate);
        let error = documents
            .prepare_text_box(forbidden.doc, "Hello", arial())
            .unwrap_err();
        assert!(error.message.contains("author"), "{}", error.message);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn prepared_words_are_in_the_journal_as_a_small_subset_and_come_back_after_a_crash() {
        let data = DataFolder::new("text-box-journal");
        let path = write_pdf("b229-journal", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let prepared = earlier
            .prepare_text_box(doc, "Hello, Wörld", arial())
            .unwrap();
        edited_pages(&earlier, &mut doc, text_box_with(0, prepared.model));
        // One journal: the edit, and the words it names, as a subset of the font; not the font.
        let file = std::fs::read_dir(data.recovery())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"textBoxes\"") && text.contains("addTextBox"));
        assert!(text.len() < 100_000, "{} bytes", text.len());
        drop(earlier);

        // The run ended without saving; the next one offers the box, and makes it.
        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Available);
        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        let [boxed] = &later.page_annotations(recovered.doc, 0).unwrap()[..] else {
            panic!("one text box")
        };
        assert_eq!(boxed.kind, AnnotationKind::TextBox);
        // It is the document's own edit now, which can be undone.
        let undone = opened_info(later.undo(recovered.doc, None).unwrap());
        assert!(later.page_annotations(undone.doc, 0).unwrap().is_empty());
        std::fs::remove_file(path).ok();
    }

    fn red() -> TextBoxStyle {
        TextBoxStyle {
            size: 24.0,
            color: Rgb { r: 200, g: 0, b: 0 },
            align: TextAlign::Center,
        }
    }

    #[test]
    fn a_text_box_is_moved_restyled_and_given_other_words_and_undo_redo_and_a_new_worker_agree() {
        let (documents, info, path) = open("text-box-edit", 1);
        let first = documents
            .prepare_text_box(info.doc, "Hello, Wörld", arial())
            .unwrap();
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, text_box_with(0, first.model));
        let id = documents.page_annotations(doc, 0).unwrap()[0].id;
        // What the dialog is filled in with: read from the file, no font needed.
        let told = documents.text_box(doc, 0, id).unwrap();
        assert_eq!(
            (told.text.as_str(), told.font.as_str()),
            ("Hello, Wörld", "Arial")
        );
        assert_eq!(told.style.size, 12.0);

        let moved = Rect {
            x0: 100.0,
            y0: 300.0,
            x1: 400.0,
            y1: 360.0,
        };
        edited_pages(
            &documents,
            &mut doc,
            Edit::SetAnnotationRect {
                page: 0,
                annotation: id,
                rect: moved,
            },
        );
        // Another style with the words it has: nothing is prepared, no font is read.
        edited_pages(
            &documents,
            &mut doc,
            Edit::SetTextBox {
                page: 0,
                annotation: id,
                model: None,
                style: red(),
            },
        );
        let second = documents.prepare_text_box(doc, "Goodbye", arial()).unwrap();
        edited_pages(
            &documents,
            &mut doc,
            Edit::SetTextBox {
                page: 0,
                annotation: id,
                model: Some(second.model),
                style: red(),
            },
        );
        let says = |doc: DocumentId| {
            let told = documents.text_box(doc, 0, id).unwrap();
            (told.text, told.style)
        };
        assert_eq!(says(doc), ("Goodbye".to_owned(), red()));

        // Undone one edit at a time, and made again: the same annotation, with what it said.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(says(doc), ("Hello, Wörld".to_owned(), red()));
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        let (text, style) = says(doc);
        assert_eq!((text.as_str(), style.size), ("Hello, Wörld", 12.0));
        assert_eq!(
            documents.page_annotations(doc, 0).unwrap()[0].rect.y0,
            moved.y0
        );
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!(says(doc), ("Goodbye".to_owned(), red()));
        // A worker that died is replaced, and every edit is made again in the new one.
        documents
            .with_document(doc, |document| {
                document.lost = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(says(doc), ("Goodbye".to_owned(), red()));
        assert_eq!(documents.page_annotations(doc, 0).unwrap()[0].id, id);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn edited_text_boxes_come_back_after_a_crash_with_the_words_they_were_given() {
        let data = DataFolder::new("text-box-edit-journal");
        let path = write_pdf("b229-edit-journal", &letter_pdf(1));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let first = earlier.prepare_text_box(doc, "Hello", arial()).unwrap();
        edited_pages(&earlier, &mut doc, text_box_with(0, first.model));
        let id = earlier.page_annotations(doc, 0).unwrap()[0].id;
        let second = earlier.prepare_text_box(doc, "Goodbye", arial()).unwrap();
        edited_pages(
            &earlier,
            &mut doc,
            Edit::SetTextBox {
                page: 0,
                annotation: id,
                model: Some(second.model),
                style: red(),
            },
        );
        drop(earlier);

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Available);
        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        let told = later.text_box(recovered.doc, 0, id).unwrap();
        assert_eq!((told.text.as_str(), told.style), ("Goodbye", red()));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_words_of_a_text_box_are_found_and_selected_beside_the_words_of_the_page() {
        let text = "BT /F1 24 Tf 72 700 Td (Copy me) Tj ET";
        let pdf = pdf_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ]);
        let (documents, info, path) = open_bytes("text-box-text", &pdf);
        let prepared = documents
            .prepare_text_box(info.doc, "Hello, Wörld", arial())
            .unwrap();
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, text_box_with(0, prepared.model));

        // Selected and copied: the page's line, then the box's.
        let layer = documents.page_text(doc, 0).unwrap();
        let lines: Vec<&str> = layer.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(lines, ["Copy me", "Hello, Wörld"]);
        // Found: in the box, and in the page.
        let found = |doc: DocumentId, query: &str| {
            documents
                .search_page(doc, 0, query, false, 10)
                .unwrap()
                .hits
                .len()
        };
        assert_eq!(found(doc, "wörld"), 1);
        assert_eq!(found(doc, "copy"), 1);
        assert_eq!(found(doc, "nothing here"), 0);
        // After undo the box and its words are gone; after redo they are back.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert_eq!(found(doc, "wörld"), 0);
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!(found(doc, "wörld"), 1);
        std::fs::remove_file(path).ok();
    }

    /// The picture of the corpus file `name`, opened as the one the user chose is (B2-08).
    fn picture(name: &str) -> std::fs::File {
        open_picture(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/corpus/images")
                .join(name),
        )
        .unwrap()
    }

    fn picture_stamp(page: u32, image: StampImageId) -> Edit {
        Edit::AddImageStamp {
            page,
            rect: Rect {
                x0: 100.0,
                y0: 100.0,
                x1: 228.0,
                y1: 164.0,
            },
            image,
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// What a camera wrote into `images/stamp-exif.jpg` besides its pixels.
    const PRIVATE: [&[u8]; 4] = [b"Canon", b"2023:07:04", b"Exif", b"Mark IV"];

    #[test]
    fn a_picture_becomes_a_stamp_that_undo_redo_and_saving_keep_without_its_exif() {
        let (documents, info, path) = open("picture-stamp", 2);
        let picked = documents
            .prepare_stamp_image(info.doc, &picture("stamp-exif.jpg"))
            .unwrap();
        assert_eq!((picked.width, picked.height), (64, 32));
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, picture_stamp(0, picked.image));
        let [stamp] = &documents.page_annotations(doc, 0).unwrap()[..] else {
            panic!("one stamp")
        };
        assert_eq!(stamp.kind, AnnotationKind::Stamp);
        // Undone, and made again, from the picture the main process keeps for it.
        doc = opened_info(documents.undo(doc, None).unwrap()).doc;
        assert!(documents.page_annotations(doc, 0).unwrap().is_empty());
        doc = opened_info(documents.redo(doc).unwrap()).doc;
        assert_eq!(documents.page_annotations(doc, 0).unwrap().len(), 1);
        // A worker that died is replaced, and the stamp is made again in the new one.
        documents
            .with_document(doc, |document| {
                document.lost = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(documents.page_annotations(doc, 0).unwrap().len(), 1);

        let copy = std::env::temp_dir().join(format!("b208-{}-copy.pdf", std::process::id()));
        let _ = std::fs::remove_file(&copy);
        documents.save(doc, Some(copy.clone())).unwrap();
        let saved = std::fs::read(&copy).unwrap();
        std::fs::remove_file(&copy).ok();
        assert!(contains(&saved, b"/Stamp"));
        for private in PRIVATE {
            assert!(!contains(&saved, private));
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_turned_picture_stays_turned_through_undo_redo_a_worker_that_died_saving_and_a_crash() {
        let data = DataFolder::new("picture-turn");
        let path = write_pdf("b232-turn", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let picked = earlier
            .prepare_stamp_image(doc, &picture("stamp-exif.jpg"))
            .unwrap();
        edited_pages(&earlier, &mut doc, picture_stamp(0, picked.image));
        let stamp = earlier.page_annotations(doc, 0).unwrap().remove(0);
        edited_pages(
            &earlier,
            &mut doc,
            Edit::SetPictureAngle {
                page: 0,
                annotation: stamp.id,
                degrees: 30.0,
            },
        );
        let angle = |documents: &Documents, doc| {
            documents.page_annotations(doc, 0).unwrap()[0]
                .picture
                .expect("a picture")
                .angle
        };
        assert!((angle(&earlier, doc) - 30.0).abs() < 0.01);
        // Undone it is upright, redone it is turned again.
        doc = opened_info(earlier.undo(doc, None).unwrap()).doc;
        assert_eq!(angle(&earlier, doc), 0.0);
        doc = opened_info(earlier.redo(doc).unwrap()).doc;
        assert!((angle(&earlier, doc) - 30.0).abs() < 0.01);
        // A worker that died is replaced, and the edits are made again in the new one.
        earlier
            .with_document(doc, |document| {
                document.lost = true;
                Ok(())
            })
            .unwrap();
        assert!((angle(&earlier, doc) - 30.0).abs() < 0.01);
        drop(earlier);

        // The app ended before saving: the journal has the turn, and it is made again.
        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Available);
        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        assert!((angle(&later, recovered.doc) - 30.0).abs() < 0.01);
        // Saved, and read again: still turned.
        let copy = std::env::temp_dir().join(format!("b232-{}-copy.pdf", std::process::id()));
        let _ = std::fs::remove_file(&copy);
        later.save(recovered.doc, Some(copy.clone())).unwrap();
        let reread = open_in(&later, &copy);
        assert!((angle(&later, reread.doc) - 30.0).abs() < 0.01);
        std::fs::remove_file(copy).ok();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_picture_stamp_an_earlier_run_did_not_save_comes_back_with_its_picture() {
        let data = DataFolder::new("picture");
        let path = write_pdf("b208-picture", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let picked = earlier
            .prepare_stamp_image(doc, &picture("stamp-exif.jpg"))
            .unwrap();
        edited_pages(&earlier, &mut doc, picture_stamp(1, picked.image));
        // The journal has the picture, as text, and the pixels of it alone.
        let journal = std::fs::read_dir(data.recovery())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(journal).unwrap();
        assert!(text.contains("\"pictures\""));
        for private in PRIVATE {
            assert!(!text.contains(&String::from_utf8_lossy(private).into_owned()));
        }
        drop(earlier);

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Available);
        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        assert_eq!(later.page_annotations(recovered.doc, 1).unwrap().len(), 1);
        // Made again from the recovered picture: undo and redo work on it.
        let undone = opened_info(later.undo(recovered.doc, None).unwrap());
        assert!(later.page_annotations(undone.doc, 1).unwrap().is_empty());
        let redone = opened_info(later.redo(undone.doc).unwrap());
        assert_eq!(later.page_annotations(redone.doc, 1).unwrap().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_picture_that_is_not_there_or_not_one_is_refused_and_the_document_goes_on() {
        let (documents, info, path) = open("picture-refused", 1);
        // No such picture.
        assert_eq!(
            documents
                .apply_edit(&EditArgs {
                    doc: info.doc,
                    edit: picture_stamp(0, StampImageId(99)),
                })
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // A PDF file is no picture; the worker says so, and goes on.
        let not_a_picture = std::fs::File::open(&path).unwrap();
        assert!(
            documents
                .prepare_stamp_image(info.doc, &not_a_picture)
                .is_err()
        );
        let picked = documents
            .prepare_stamp_image(info.doc, &picture("stamp-metadata.png"))
            .unwrap();
        let mut doc = info.doc;
        edited_pages(&documents, &mut doc, picture_stamp(0, picked.image));
        assert_eq!(documents.page_annotations(doc, 0).unwrap().len(), 1);
        // A picture that claims to be far too large is refused before it is decoded.
        assert!(
            documents
                .prepare_stamp_image(doc, &picture("stamp-huge-dimensions.png"))
                .is_err()
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_document_that_may_not_be_annotated_takes_no_picture_stamps() {
        // RC4, revision 2, /P without the annotate bit (tests/corpus/generate.py).
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/benign/encrypted-rc4-40.pdf");
        let documents = Documents::new(worker());
        let (_, info) = unlocked(&documents, &path);
        assert!(!info.permissions.annotate);
        assert_eq!(
            documents
                .prepare_stamp_image(info.doc, &picture("stamp-metadata.png"))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_journal_stops_before_the_pages_of_another_file_and_the_rest_is_offered_as_far_as_it_goes()
    {
        let data = DataFolder::new("merge-partial");
        let path = write_pdf("b206-partial", &letter_pdf(3));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let taken = earlier
            .prepare_pages_source(doc, &corpus_path("benign/single-page.pdf"), None)
            .unwrap();
        edited_pages(&earlier, &mut doc, turn(0));
        edited_pages(&earlier, &mut doc, insert_pages(1, taken.source));
        edited_pages(&earlier, &mut doc, turn(2));
        // The journal has the edit before the pages of the other file, and says two are left out.
        let file = std::fs::read_dir(data.recovery())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let kept: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(kept["edits"].as_array().unwrap().len(), 1);
        assert_eq!(kept["lost"], 2);
        drop(earlier);

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Partial);
        assert_eq!(reopened.pages, [LETTER; 3]);
        // What can be made again is: the first turn, and not the pages or the second turn.
        let recovered = opened_info(later.recover(reopened.doc).unwrap());
        assert_eq!(recovered.pages, [LANDSCAPE, LETTER, LETTER]);
        assert_eq!(recovered.recovery, Recovery::None);
        // It is the document's own history now: nothing is left out of it.
        let file = std::fs::read_dir(data.recovery())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let now: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert!(now.get("lost").is_none());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_journal_with_nothing_before_the_pages_of_another_file_can_only_be_told_of() {
        let data = DataFolder::new("merge-lost");
        let path = write_pdf("b206-lost", &letter_pdf(2));
        let earlier = data.documents();
        let mut doc = open_in(&earlier, &path).doc;
        let taken = earlier
            .prepare_pages_source(doc, &corpus_path("benign/single-page.pdf"), None)
            .unwrap();
        edited_pages(&earlier, &mut doc, insert_pages(2, taken.source));
        // There is a journal (the document has changes), though no edit can be kept in it.
        assert_eq!(data.journals(), 1);
        drop(earlier);

        let later = data.documents();
        let reopened = open_in(&later, &path);
        assert_eq!(reopened.recovery, Recovery::Lost);
        assert_eq!(reopened.pages, [LETTER; 2]);
        assert_eq!(
            later.recover(reopened.doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let discarded = opened_info(later.discard_recovered(reopened.doc).unwrap());
        assert_eq!(discarded.recovery, Recovery::None);
        assert_eq!(data.journals(), 0);
        std::fs::remove_file(path).ok();
    }

    /// Recognising the text of scanned pages (B2-10), through the real worker.
    mod ocr {
        use std::time::{Duration, Instant};

        use ipc_contract::types::{OcrProgress, OcrRun, Settings};

        use super::*;
        use crate::ocr::Ocr;
        use crate::ocr_languages::Languages;

        fn corpus(name: &str) -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/corpus")
                .join(name)
        }

        fn ocr() -> Ocr {
            Ocr::new(Languages::new(
                Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/tessdata")),
                None,
            ))
        }

        fn english() -> Settings {
            Settings {
                ocr_language: Some("eng".to_owned()),
                ..Settings::default()
            }
        }

        fn last_progress(events: &[OpenEvent]) -> Option<OcrProgress> {
            events.iter().rev().find_map(|event| match event {
                OpenEvent::Ocr { progress, .. } => Some(*progress),
                _ => None,
            })
        }

        /// Steps recognising until the latest progress is one `until` accepts; everything the
        /// page was told is returned.
        fn run(
            documents: &Documents,
            ocr: &Ocr,
            settings: &Settings,
            until: impl Fn(&OcrProgress) -> bool,
        ) -> Vec<OpenEvent> {
            let mut events = Vec::new();
            let started = Instant::now();
            loop {
                ocr.tick(documents, settings, Instant::now(), &mut |event| {
                    events.push(event);
                });
                if last_progress(&events).is_some_and(|progress| until(&progress)) {
                    return events;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(90),
                    "not done in time: {events:?}"
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        }

        fn done(progress: &OcrProgress) -> bool {
            progress.run == OcrRun::Done
        }

        fn lines(text: &PageText) -> Vec<&str> {
            text.lines.iter().map(|line| line.text.as_str()).collect()
        }

        #[test]
        fn the_scanned_pages_of_a_document_are_read_in_the_background_and_then_found() {
            let documents = Documents::new(worker());
            let info = open_in(&documents, &corpus("benign/scanned-text.pdf"));
            // Before: the page has no text, so there is nothing to select and nothing to find.
            assert!(documents.page_text(info.doc, 0).unwrap().lines.is_empty());
            let found = documents
                .search_page(info.doc, 0, "secret", false, 10)
                .unwrap();
            assert!(!found.has_text && found.hits.is_empty());

            let events = run(&documents, &ocr(), &english(), done);
            let progress = last_progress(&events).unwrap();
            assert_eq!(
                (
                    progress.doc,
                    progress.pages,
                    progress.checked,
                    progress.scans
                ),
                (info.doc, 1, 1, 1)
            );
            assert_eq!((progress.recognised, progress.failed), (1, 0));
            assert!(events.iter().any(|event| matches!(
                event,
                OpenEvent::OcrPage { doc, page_index: 0, .. } if *doc == info.doc
            )));

            let text = documents.page_text(info.doc, 0).unwrap();
            assert!(text.recognised);
            assert_eq!(lines(&text), ["PRIVACY FIRST", "SECRET PAPER"]);
            let found = documents
                .search_page(info.doc, 0, "secret", false, 10)
                .unwrap();
            assert!(found.has_text);
            assert_eq!(found.hits.len(), 1);
        }

        #[test]
        fn the_language_the_app_chooses_reads_the_sample_too() {
            // Traditional Chinese is what the app chooses; its data reads Latin letters as well.
            let documents = Documents::new(worker());
            let info = open_in(&documents, &corpus("benign/scanned-text.pdf"));
            run(&documents, &ocr(), &Settings::default(), done);
            let text = documents.page_text(info.doc, 0).unwrap();
            assert!(text.recognised);
            // Its data reads blocky Latin lettering poorly, so only that something was read is
            // checked (English data reads this sample exactly, above).
            assert!(!lines(&text).is_empty());
        }

        #[test]
        fn a_document_with_text_has_nothing_to_read() {
            let documents = Documents::new(worker());
            let info = open_in(&documents, &corpus("benign/mixed-text-zh-en.pdf"));
            let events = run(&documents, &ocr(), &english(), done);
            let progress = last_progress(&events).unwrap();
            assert_eq!(
                (progress.pages, progress.checked, progress.scans),
                (1, 1, 0)
            );
            assert!(!documents.page_text(info.doc, 0).unwrap().recognised);
            assert!(
                events
                    .iter()
                    .all(|event| !matches!(event, OpenEvent::OcrPage { .. }))
            );
        }

        #[test]
        fn left_to_the_user_the_text_is_only_read_when_they_ask() {
            let documents = Documents::new(worker());
            let info = open_in(&documents, &corpus("benign/scanned-text.pdf"));
            let ocr = ocr();
            let settings = Settings {
                ocr_auto: false,
                ..english()
            };
            let events = run(&documents, &ocr, &settings, |progress| {
                progress.run == OcrRun::Idle
            });
            assert_eq!(last_progress(&events).unwrap().checked, 0);
            for _ in 0..5 {
                ocr.tick(&documents, &settings, Instant::now(), &mut |_| {});
                std::thread::sleep(Duration::from_millis(30));
            }
            assert!(documents.page_text(info.doc, 0).unwrap().lines.is_empty());

            ocr.start_tab(tab_of(&documents, info.doc));
            run(&documents, &ocr, &settings, done);
            assert!(documents.page_text(info.doc, 0).unwrap().recognised);
        }
    }
}
