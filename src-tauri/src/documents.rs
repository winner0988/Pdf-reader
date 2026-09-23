//! The window's document: opens files through the sandboxed worker and keeps paths in the main
//! process (MVP-06, ADR 0008). The frontend only ever sees a `DocumentId` and a file name.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ipc_contract::limits::MAX_DISPLAY_NAME_BYTES;
use ipc_contract::raster::{encode_raster, fit_scale};
use ipc_contract::types::{
    DocumentId, DocumentInfo, ErrorCode, IpcError, OpenEvent, RenderPageArgs,
};
use ipc_contract::validate::{Validate, check_page_index};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, MAX_DOCUMENT_BYTES, WorkerHost};

/// Display name used when a path has no file name component.
const FALLBACK_NAME: &str = "PDF";

pub struct Documents {
    inner: Mutex<Inner>,
}

struct Inner {
    host: WorkerHost,
    /// The window shows at most one document.
    current: Option<OpenDocument>,
    /// Most recent open attempt, for "retry". Never leaves the main process.
    last_path: Option<PathBuf>,
}

struct OpenDocument {
    /// What the frontend knows; `info.doc` stays the same for the document's lifetime.
    info: DocumentInfo,
    path: PathBuf,
    /// The document's id in the current worker process. Differs from `info.doc` after the
    /// worker was restarted and the file reopened.
    worker_doc: DocumentId,
    /// The worker that had the document open crashed, timed out or misbehaved; the file is
    /// reopened in a fresh worker before the next render.
    lost: bool,
}

impl Documents {
    pub fn new(worker: PathBuf) -> Self {
        Self {
            inner: Mutex::new(Inner {
                host: WorkerHost::new(worker, HostConfig::default()),
                current: None,
                last_path: None,
            }),
        }
    }

    /// Opens `path` as the window's document, replacing the current one, and reports
    /// `Opening` followed by `Opened` or `Failed`. Opens are serialized, so the events of two
    /// opens never interleave.
    pub fn open(&self, path: &Path, ignored_files: u32, report: &dyn Fn(OpenEvent)) {
        let mut inner = self.lock();
        let display_name = display_name(path);
        report(OpenEvent::Opening {
            display_name: display_name.clone(),
        });
        inner.last_path = Some(path.to_owned());
        if let Some(previous) = inner.current.take() {
            // Best effort: if the worker is gone, so is the document.
            let _ = inner.host.notify(&WorkerRequest::Close {
                doc: previous.worker_doc,
            });
        }

        let result = check_file(path).and_then(|()| {
            let (doc, response) = inner.host.open(path).map_err(|error| ipc_error(&error))?;
            document_info(doc, display_name.clone(), response)
        });
        match result {
            Ok(info) => {
                inner.current = Some(OpenDocument {
                    info: info.clone(),
                    path: path.to_owned(),
                    worker_doc: info.doc,
                    lost: false,
                });
                report(OpenEvent::Opened {
                    info,
                    ignored_files,
                });
            }
            Err(error) => report(OpenEvent::Failed {
                display_name,
                error,
                ignored_files,
            }),
        }
    }

    /// Opens the most recently attempted file again. Returns false if there is none.
    pub fn retry(&self, report: &dyn Fn(OpenEvent)) -> bool {
        let Some(path) = self.lock().last_path.clone() else {
            return false;
        };
        self.open(&path, 0, report);
        true
    }

    /// Closes `doc` and releases it in the worker.
    pub fn close(&self, doc: DocumentId) -> Result<(), IpcError> {
        let mut inner = self.lock();
        match inner.current.take_if(|current| current.info.doc == doc) {
            Some(current) => inner
                .host
                .notify(&WorkerRequest::Close {
                    doc: current.worker_doc,
                })
                .map_err(|error| ipc_error(&error)),
            None => Err(unknown_document()),
        }
    }

    /// The open document, if any (sent again when the frontend reloads).
    pub fn current(&self) -> Option<DocumentInfo> {
        self.lock()
            .current
            .as_ref()
            .map(|current| current.info.clone())
    }

    /// Renders a page and returns it in the `render_page` wire format (ipc_contract::raster).
    /// Pages too large for the raster limits come back at a lower resolution. If the worker
    /// dies while rendering, this page fails; the next render reopens the file in a new worker.
    pub fn render(&self, args: &RenderPageArgs) -> Result<Vec<u8>, IpcError> {
        args.validate().map_err(invalid_argument)?;
        let mut inner = self.lock();
        let Inner { host, current, .. } = &mut *inner;
        let current = current
            .as_mut()
            .filter(|current| current.info.doc == args.doc)
            .ok_or_else(unknown_document)?;
        let page_count = u32::try_from(current.info.pages.len()).unwrap_or(u32::MAX);
        check_page_index(args.page_index, page_count).map_err(invalid_argument)?;
        let page = current.info.pages[args.page_index as usize];
        let scale = fit_scale(page, args.scale).map_err(|error| IpcError {
            code: ErrorCode::LimitExceeded,
            message: format!("page too large to render: {error}"),
        })?;

        if current.lost {
            current.worker_doc = reopen(host, current)?;
            current.lost = false;
        }
        let rendered = host.request(|request| WorkerRequest::Render {
            request,
            doc: current.worker_doc,
            page_index: args.page_index,
            scale,
            rotation: args.rotation,
        });
        match rendered {
            Ok(WorkerResponse::Rendered { raster, .. }) => {
                encode_raster(&raster).map_err(|error| IpcError {
                    code: ErrorCode::ProtocolViolation,
                    message: format!("invalid raster: {error}"),
                })
            }
            Ok(_) => Err(IpcError {
                code: ErrorCode::ProtocolViolation,
                message: "unexpected response to Render".to_owned(),
            }),
            Err(error) => {
                if matches!(
                    error,
                    HostError::Crashed | HostError::Timeout | HostError::ProtocolViolation(_)
                ) {
                    current.lost = true;
                }
                Err(ipc_error(&error))
            }
        }
    }

    #[cfg(test)]
    fn worker_id(&self) -> Option<u32> {
        self.lock().host.worker_id()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// Opens the current document's file again in a fresh worker; it must still have the same pages.
fn reopen(host: &mut WorkerHost, current: &OpenDocument) -> Result<DocumentId, IpcError> {
    check_file(&current.path)?;
    let (doc, response) = host
        .open(&current.path)
        .map_err(|error| ipc_error(&error))?;
    let reopened = document_info(doc, current.info.display_name.clone(), response)?;
    if reopened.pages != current.info.pages {
        let _ = host.notify(&WorkerRequest::Close { doc });
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

    #[test]
    fn closing_an_unknown_document_is_an_error() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        assert_eq!(
            documents.close(DocumentId(7)).unwrap_err().code,
            ErrorCode::UnknownDocument
        );
        assert_eq!(documents.current(), None);
    }

    #[test]
    fn a_failed_open_is_reported_and_can_be_retried() {
        let documents = Documents::new(PathBuf::from("missing-worker.exe"));
        assert!(!documents.retry(&|_| {}));

        let events = std::cell::RefCell::new(Vec::new());
        let missing = std::env::temp_dir().join("mvp06-does-not-exist.pdf");
        documents.open(&missing, 2, &|event| events.borrow_mut().push(event));
        assert!(documents.retry(&|event| events.borrow_mut().push(event)));

        let failed = OpenEvent::Failed {
            display_name: "mvp06-does-not-exist.pdf".to_owned(),
            error: IpcError {
                code: ErrorCode::Unreadable,
                message: "the file does not exist or cannot be accessed".to_owned(),
            },
            ignored_files: 2,
        };
        let events = events.into_inner();
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0],
            OpenEvent::Opening {
                display_name: "mvp06-does-not-exist.pdf".to_owned()
            }
        );
        assert_eq!(events[1], failed);
        // A retry is a fresh attempt: nothing was dropped this time.
        assert!(matches!(
            &events[3],
            OpenEvent::Failed {
                ignored_files: 0,
                ..
            }
        ));
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
        let kids: Vec<String> = (0..pages)
            .map(|index| format!("{} 0 R", 3 + index))
            .collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                kids.join(" ")
            ),
        ];
        objects.extend(
            (0..pages)
                .map(|_| "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned()),
        );
        let mut out = b"%PDF-1.7
"
        .to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{} 0 obj
{body}
endobj
",
                    index + 1
                )
                .as_bytes(),
            );
        }
        let xref = out.len();
        out.extend_from_slice(
            format!(
                "xref
0 {}
0000000000 65535 f 
",
                objects.len() + 1
            )
            .as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(
                format!(
                    "{offset:010} 00000 n 
"
                )
                .as_bytes(),
            );
        }
        out.extend_from_slice(
            format!(
                "trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn open(name: &str, pages: usize) -> (Documents, DocumentInfo, PathBuf) {
        let path = std::env::temp_dir().join(format!("mvp07-{}-{name}.pdf", std::process::id()));
        std::fs::write(&path, letter_pdf(pages)).unwrap();
        let documents = Documents::new(worker());
        let opened = std::cell::RefCell::new(None);
        documents.open(&path, 0, &|event| {
            if let OpenEvent::Opened { info, .. } = event {
                *opened.borrow_mut() = Some(info);
            }
        });
        let info = opened.into_inner().expect("opened");
        (documents, info, path)
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
        documents.close(info.doc).unwrap();
        assert_eq!(
            code(args(info.doc, 0, 1.0, Rotation::None)),
            ErrorCode::UnknownDocument
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_crashed_worker_fails_one_render_then_the_document_is_reopened() {
        let (documents, info, path) = open("crash", 2);
        documents
            .render(&args(info.doc, 0, 0.5, Rotation::None))
            .unwrap();

        let pid = documents.worker_id().expect("worker running");
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
        assert_ne!(documents.worker_id(), Some(pid));
        assert_eq!(documents.current().unwrap().doc, info.doc);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_file_changed_on_disk_is_not_silently_swapped_in() {
        let (documents, info, path) = open("changed", 2);
        let pid = documents.worker_id().expect("worker running");
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
}
