//! The window's documents (MVP-06, MVP-14): every file the user opens gets a tab, and every open
//! document its own sandboxed worker (ADR 0012), so a hostile PDF that takes over its worker
//! cannot reach the other documents. Paths stay in the main process (ADR 0008): the frontend
//! only ever sees tab and document ids and file names.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ipc_contract::limits::{MAX_DISPLAY_NAME_BYTES, MAX_TABS, MAX_TEXT_BYTES};
use ipc_contract::raster::{encode_raster, fit_scale};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{
    BlockedAction, DocumentId, DocumentInfo, ErrorCode, IpcError, LinkArgs, LinkPreview,
    LinkTarget, OpenEvent, OutlineLinkArgs, OutlineResult, PageLink, PageText, RenderPageArgs,
    SearchHit, TabId,
};
use ipc_contract::validate::{Validate, check_page_index};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, MAX_DOCUMENT_BYTES, WorkerHost};

/// Display name used when a path has no file name component.
const FALLBACK_NAME: &str = "PDF";

pub struct Documents {
    worker: PathBuf,
    inner: Mutex<Inner>,
}

struct Inner {
    /// For tab and document ids; never reused while the app runs.
    next_id: u32,
    /// In the order the tabs were added.
    tabs: Vec<Arc<Tab>>,
    /// The tab the window shows, for the window title.
    active: Option<TabId>,
}

impl Inner {
    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// One tab. Locks are always taken in the order `Documents::inner`, `Tab::event`,
/// `Tab::document`, and `inner` is never held during a worker request.
struct Tab {
    id: TabId,
    /// Never leaves the main process.
    path: PathBuf,
    display_name: String,
    /// What the frontend was last told about this tab (`Opening`, `Opened` or `Failed`). Readable
    /// while `document` is busy with a long worker request.
    event: Mutex<OpenEvent>,
    document: Mutex<Option<OpenDocument>>,
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
    /// The worker crashed, timed out or misbehaved; the file is reopened in a fresh worker
    /// before the next request.
    lost: bool,
}

impl Documents {
    pub fn new(worker: PathBuf) -> Self {
        Self {
            worker,
            inner: Mutex::new(Inner {
                next_id: 1,
                tabs: Vec::new(),
                active: None,
            }),
        }
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
                let id = TabId(inner.next_id());
                let display_name = display_name(path);
                let event = OpenEvent::Opening {
                    tab: id,
                    display_name: display_name.clone(),
                };
                inner.tabs.push(Arc::new(Tab {
                    id,
                    path: path.clone(),
                    display_name,
                    event: Mutex::new(event.clone()),
                    document: Mutex::new(None),
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
        let Some(tab) = self.tab(tab) else {
            return;
        };
        let result = check_file(&tab.path).and_then(|()| {
            let mut host = WorkerHost::new(self.worker.clone(), HostConfig::default());
            let (worker_doc, response) = host.open(&tab.path).map_err(|error| ipc_error(&error))?;
            let doc = DocumentId(self.lock().next_id());
            let info = document_info(doc, tab.display_name.clone(), response)?;
            Ok(OpenDocument {
                info,
                host,
                path: tab.path.clone(),
                worker_doc,
                lost: false,
            })
        });
        let event = match &result {
            Ok(document) => OpenEvent::Opened {
                tab: tab.id,
                info: document.info.clone(),
            },
            Err(error) => OpenEvent::Failed {
                tab: tab.id,
                display_name: tab.display_name.clone(),
                error: error.clone(),
            },
        };
        {
            let inner = self.lock();
            if !inner.tabs.iter().any(|open| Arc::ptr_eq(open, &tab)) {
                return;
            }
            *lock(&tab.event) = event.clone();
            *lock(&tab.document) = result.ok();
        }
        report(event);
    }

    /// Opens the file of a failed tab again, in the same tab.
    pub fn retry(&self, tab: TabId, report: &dyn Fn(OpenEvent)) -> Result<(), IpcError> {
        let found = self.tab(tab).ok_or_else(unknown_tab)?;
        let opening = OpenEvent::Opening {
            tab,
            display_name: found.display_name.clone(),
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
        }
        Ok(())
    }

    /// Records the tab the window shows (`None`: no tab) and returns its file name, for the
    /// window title. An unknown tab counts as none.
    pub fn set_active(&self, tab: Option<TabId>) -> Option<String> {
        let mut inner = self.lock();
        let active = tab.and_then(|id| inner.tabs.iter().find(|open| open.id == id).cloned());
        inner.active = active.as_ref().map(|found| found.id);
        active.map(|found| found.display_name.clone())
    }

    /// The file name of the tab the window shows.
    pub fn active_name(&self) -> Option<String> {
        let inner = self.lock();
        let active = inner.active?;
        inner
            .tabs
            .iter()
            .find(|open| open.id == active)
            .map(|found| found.display_name.clone())
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
        match document.as_mut() {
            Some(document) if document.info.doc == doc => work(document),
            _ => Err(unknown_document()),
        }
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
    if document.lost {
        document.worker_doc = reopen(document)?;
        document.lost = false;
    }
    let doc = document.worker_doc;
    document
        .host
        .request(|request| make(request, doc))
        .map_err(|error| {
            if matches!(
                error,
                HostError::Crashed | HostError::Timeout | HostError::ProtocolViolation(_)
            ) {
                document.lost = true;
            }
            ipc_error(&error)
        })
}

fn unexpected(request: &str) -> IpcError {
    IpcError {
        code: ErrorCode::ProtocolViolation,
        message: format!("unexpected response to {request}"),
    }
}

/// Opens the document's file again in a fresh worker; it must still have the same pages.
fn reopen(document: &mut OpenDocument) -> Result<DocumentId, IpcError> {
    check_file(&document.path)?;
    let (doc, response) = document
        .host
        .open(&document.path)
        .map_err(|error| ipc_error(&error))?;
    let reopened = document_info(doc, document.info.display_name.clone(), response)?;
    if reopened.pages != document.info.pages {
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

fn document_info(
    doc: DocumentId,
    display_name: String,
    response: WorkerResponse,
) -> Result<DocumentInfo, IpcError> {
    let WorkerResponse::Opened { document, .. } = response else {
        return Err(IpcError {
            code: ErrorCode::ProtocolViolation,
            message: "unexpected response to Open".to_owned(),
        });
    };
    let info = DocumentInfo {
        doc,
        display_name,
        pages: document.pages,
        has_outline: document.has_outline,
        security: document.security,
    };
    info.validate().map_err(|error| IpcError {
        code: ErrorCode::Internal,
        message: format!("invalid document info: {error}"),
    })?;
    Ok(info)
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
    fn errors_never_carry_worker_text_or_paths() {
        use ipc_contract::worker::{WorkerError, WorkerErrorCode};
        let error = ipc_error(&HostError::Worker(WorkerError {
            code: WorkerErrorCode::Encrypted,
            detail: r"secret text from C:\Users\someone\a.pdf".to_owned(),
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
    use ipc_contract::types::{RequestId, Rotation};

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
}
