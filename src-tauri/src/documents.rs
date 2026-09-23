//! The window's document: opens files through the sandboxed worker and keeps paths in the main
//! process (MVP-06, ADR 0008). The frontend only ever sees a `DocumentId` and a file name.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ipc_contract::limits::MAX_DISPLAY_NAME_BYTES;
use ipc_contract::types::{DocumentId, DocumentInfo, ErrorCode, IpcError, OpenEvent};
use ipc_contract::validate::Validate;
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
    current: Option<DocumentInfo>,
    /// Most recent open attempt, for "retry". Never leaves the main process.
    last_path: Option<PathBuf>,
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
            let _ = inner
                .host
                .notify(&WorkerRequest::Close { doc: previous.doc });
        }

        let result = check_file(path).and_then(|()| {
            let (doc, response) = inner.host.open(path).map_err(|error| ipc_error(&error))?;
            document_info(doc, display_name.clone(), response)
        });
        match result {
            Ok(info) => {
                inner.current = Some(info.clone());
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
        match &inner.current {
            Some(current) if current.doc == doc => {
                inner.current = None;
                inner
                    .host
                    .notify(&WorkerRequest::Close { doc })
                    .map_err(|error| ipc_error(&error))
            }
            _ => Err(IpcError {
                code: ErrorCode::UnknownDocument,
                message: "no such open document".to_owned(),
            }),
        }
    }

    /// The open document, if any (sent again when the frontend reloads).
    pub fn current(&self) -> Option<DocumentInfo> {
        self.lock().current.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
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
