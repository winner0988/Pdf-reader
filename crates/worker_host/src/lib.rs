//! Main-process side of the `pdf_worker` protocol (ADR 0008, docs/architecture/worker-sandbox.md).
//!
//! [`WorkerHost`] starts the worker in the sandbox, checks its `Hello`, sends requests, validates
//! every response before returning it, enforces timeouts, and starts a fresh worker after a crash.
//! Everything the worker sends is untrusted: any malformed or invalid frame ends the worker.
//! This crate contains no unsafe code; process isolation lives in the `sandbox` crate.

#![cfg(windows)]

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ipc_contract::frame::{self, FrameError};
use ipc_contract::types::{DocumentId, ErrorCode, Password, RequestId};
use ipc_contract::validate::Validate;
use ipc_contract::worker::{FileHandle, WorkerError, WorkerRequest, WorkerResponse};
use sandbox::{SandboxConfig, Sandboxed};
use thiserror::Error;

/// Largest document the worker is asked to open, or writes when saving.
pub const MAX_DOCUMENT_BYTES: u64 = ipc_contract::limits::MAX_DOCUMENT_BYTES;

/// How long saving a document may take (ADR 0013): a large document is rewritten whole.
pub const SAVE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// File name of the worker executable. The installer puts it next to the app
/// (`bundle.externalBin`); in development cargo builds it into the same `target/<profile>/`.
pub const WORKER_FILE_NAME: &str = "pdf_worker.exe";

/// The worker executable next to the running program (see [`WORKER_FILE_NAME`]).
pub fn bundled_worker_path() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name(WORKER_FILE_NAME))
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub sandbox: SandboxConfig,
    /// How long a freshly started worker may take to send `Hello`.
    pub handshake_timeout: Duration,
    /// How long a single request may take before the worker is killed.
    pub request_timeout: Duration,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            sandbox: SandboxConfig::default(),
            handshake_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("could not start the worker: {0}")]
    Spawn(io::Error),
    #[error("the worker stopped unexpectedly")]
    Crashed,
    #[error("the worker did not answer in time and was stopped")]
    Timeout,
    #[error("the worker sent an invalid message and was stopped: {0}")]
    ProtocolViolation(String),
    #[error("the file could not be read: {0}")]
    Unreadable(io::Error),
    #[error("the file is larger than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    #[error("the worker reported an error: {0:?}")]
    Worker(WorkerError),
}

impl HostError {
    /// The code the frontend shows a localized message for.
    pub fn code(&self) -> ErrorCode {
        match self {
            HostError::Spawn(_) => ErrorCode::Internal,
            HostError::Crashed => ErrorCode::WorkerCrashed,
            HostError::Timeout => ErrorCode::WorkerTimeout,
            HostError::ProtocolViolation(_) => ErrorCode::ProtocolViolation,
            HostError::Unreadable(_) => ErrorCode::Unreadable,
            HostError::TooLarge => ErrorCode::TooLarge,
            HostError::Worker(error) => error.code.into(),
        }
    }
}

/// A running worker with its pipes and the thread reading its responses.
struct Connection {
    process: Sandboxed,
    stdin: File,
    responses: Receiver<Result<WorkerResponse, FrameError>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Dropping `process` closes the job, which kills the worker; kill explicitly first so the
        // reader thread sees end-of-stream promptly.
        let _ = self.process.kill();
    }
}

/// Owns at most one worker process and restarts it on demand.
pub struct WorkerHost {
    program: PathBuf,
    config: HostConfig,
    connection: Option<Connection>,
    next_request: u32,
    next_document: u32,
}

impl WorkerHost {
    /// `program` is the `pdf_worker` executable. Nothing starts until the first request.
    pub fn new(program: impl Into<PathBuf>, config: HostConfig) -> Self {
        Self {
            program: program.into(),
            config,
            connection: None,
            next_request: 1,
            next_document: 1,
        }
    }

    /// Whether a worker process is currently running.
    pub fn is_running(&self) -> bool {
        self.connection.is_some()
    }

    /// Process id of the running worker, if any.
    pub fn worker_id(&self) -> Option<u32> {
        self.connection
            .as_ref()
            .map(|connection| connection.process.id())
    }

    /// Peak committed memory of the running worker, if any (for measurements).
    pub fn worker_peak_memory(&self) -> Option<usize> {
        self.connection
            .as_ref()
            .and_then(|connection| connection.process.peak_memory_bytes().ok())
    }

    /// Stops the worker. The next request starts a new one; open documents are lost.
    pub fn stop(&mut self) {
        self.connection = None;
    }

    /// Opens `path` read-only here, hands the worker a duplicated read-only handle (never the
    /// path) and returns the worker's `Opened` response.
    pub fn open(&mut self, path: &Path) -> Result<(DocumentId, WorkerResponse), HostError> {
        self.open_file(path, None)
    }

    /// Opens an encrypted document with the password the user gave (MVP-16). The password goes
    /// to the worker in the Open request only, and is wiped once the request has been sent.
    pub fn open_with_password(
        &mut self,
        path: &Path,
        password: Password,
    ) -> Result<(DocumentId, WorkerResponse), HostError> {
        self.open_file(path, Some(password))
    }

    fn open_file(
        &mut self,
        path: &Path,
        password: Option<Password>,
    ) -> Result<(DocumentId, WorkerResponse), HostError> {
        let file = File::open(path).map_err(HostError::Unreadable)?;
        let size = file.metadata().map_err(HostError::Unreadable)?.len();
        if size > MAX_DOCUMENT_BYTES {
            return Err(HostError::TooLarge);
        }
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_read_only(&file)
            .map_err(HostError::Spawn)?;
        let doc = DocumentId(self.next_document);
        self.next_document += 1;
        let response = self.request(|request| WorkerRequest::Open {
            request,
            doc,
            file: FileHandle(handle),
            password,
        })?;
        Ok((doc, response))
    }

    /// Writes the open document `doc` to `file` (ADR 0013): `file` is a new temporary file the
    /// caller created and still owns; the worker gets a duplicated write-only handle to it
    /// (never a path), which cannot read, delete or rename. Returns the worker's `Saved`
    /// response. Saving may take longer than other requests: up to [`SAVE_TIMEOUT`].
    pub fn save(&mut self, doc: DocumentId, file: &File) -> Result<WorkerResponse, HostError> {
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_write_only(file)
            .map_err(HostError::Spawn)?;
        self.request_within(SAVE_TIMEOUT, |request| WorkerRequest::Save {
            request,
            doc,
            file: FileHandle(handle),
        })
    }

    /// Writes the pages `pages` of the open document `doc` to `file` as a document of their own
    /// (B2-06), as [`save`](Self::save) writes `doc`: the worker gets a write-only handle, never a
    /// path. Returns `Saved`. The document itself does not change.
    pub fn save_pages(
        &mut self,
        doc: DocumentId,
        pages: &[u32],
        file: &File,
    ) -> Result<WorkerResponse, HostError> {
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_write_only(file)
            .map_err(HostError::Spawn)?;
        self.request_within(SAVE_TIMEOUT, |request| WorkerRequest::SavePages {
            request,
            doc,
            pages: pages.to_vec(),
            file: FileHandle(handle),
        })
    }

    /// Has the worker make the plain copy of the PDF in `file` (a file the main process opened),
    /// whose pages are to go into a document (B2-06): the worker reads it through a read-only
    /// handle, as it reads a document, and never gets a path. `password` is for an encrypted
    /// file. Returns `Source`.
    pub fn prepare_source(
        &mut self,
        file: &File,
        password: Option<Password>,
    ) -> Result<WorkerResponse, HostError> {
        let size = file.metadata().map_err(HostError::Unreadable)?.len();
        if size > ipc_contract::limits::MAX_SOURCE_BYTES as u64 {
            return Err(HostError::TooLarge);
        }
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_read_only(file)
            .map_err(HostError::Spawn)?;
        self.request_within(SAVE_TIMEOUT, |request| WorkerRequest::PrepareSource {
            request,
            file: FileHandle(handle),
            password,
        })
    }

    /// Has the worker keep the bytes of the file at `path`, read-only, as the ones undo opens
    /// `doc` again from (ADR 0013): the file `doc` was just saved to. Returns `Rebased`.
    pub fn rebase(&mut self, doc: DocumentId, path: &Path) -> Result<WorkerResponse, HostError> {
        let file = File::open(path).map_err(HostError::Unreadable)?;
        let size = file.metadata().map_err(HostError::Unreadable)?.len();
        if size > MAX_DOCUMENT_BYTES {
            return Err(HostError::TooLarge);
        }
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_read_only(&file)
            .map_err(HostError::Spawn)?;
        self.request(|request| WorkerRequest::Rebase {
            request,
            doc,
            file: FileHandle(handle),
        })
    }

    /// Writes to `file`, as [`save`](Self::save) does, a copy of `doc` without its metadata and
    /// with `id` as its identifier (B2-03); `doc` itself is not changed.
    pub fn privacy_copy(
        &mut self,
        doc: DocumentId,
        file: &File,
        id: [u8; 16],
    ) -> Result<WorkerResponse, HostError> {
        self.ensure_running()?;
        let handle = self
            .connection
            .as_ref()
            .expect("running")
            .process
            .duplicate_write_only(file)
            .map_err(HostError::Spawn)?;
        self.request_within(SAVE_TIMEOUT, |request| WorkerRequest::PrivacyCopy {
            request,
            doc,
            file: FileHandle(handle),
            id,
        })
    }

    /// Sends a request built by `make` (which receives a fresh request id) and returns the
    /// validated response. Worker-reported errors come back as [`HostError::Worker`].
    pub fn request(
        &mut self,
        make: impl FnOnce(RequestId) -> WorkerRequest,
    ) -> Result<WorkerResponse, HostError> {
        let timeout = self.config.request_timeout;
        self.request_within(timeout, make)
    }

    fn request_within(
        &mut self,
        timeout: Duration,
        make: impl FnOnce(RequestId) -> WorkerRequest,
    ) -> Result<WorkerResponse, HostError> {
        self.ensure_running()?;
        let id = RequestId(self.next_request);
        self.next_request = self.next_request.wrapping_add(1).max(1);
        let request = make(id);

        let result = self.exchange(&request, id, timeout);
        if matches!(
            result,
            Err(HostError::Crashed | HostError::Timeout | HostError::ProtocolViolation(_))
        ) {
            // Any of these means the worker can no longer be trusted or used.
            self.connection = None;
        }
        result
    }

    /// Sends a request that has no response (Close, Cancel).
    pub fn notify(&mut self, request: &WorkerRequest) -> Result<(), HostError> {
        let Some(connection) = self.connection.as_mut() else {
            return Ok(());
        };
        if frame::send_wiped(&mut connection.stdin, request).is_err() {
            self.connection = None;
            return Err(HostError::Crashed);
        }
        Ok(())
    }

    fn exchange(
        &mut self,
        request: &WorkerRequest,
        id: RequestId,
        timeout: Duration,
    ) -> Result<WorkerResponse, HostError> {
        let connection = self.connection.as_mut().expect("running");
        // Wiped after writing: an Open request may carry a password (MVP-16).
        frame::send_wiped(&mut connection.stdin, request).map_err(|_| HostError::Crashed)?;
        loop {
            let response = receive(&connection.responses, timeout)?;
            match response_request(&response) {
                Some(request) if request == id => {}
                // A late answer to an earlier (abandoned) request: ignore it.
                Some(_) => continue,
                None => {}
            }
            return match response {
                WorkerResponse::Error { error, .. } => Err(HostError::Worker(error)),
                WorkerResponse::Hello { .. } => {
                    Err(HostError::ProtocolViolation("unexpected Hello".into()))
                }
                other => Ok(other),
            };
        }
    }

    fn ensure_running(&mut self) -> Result<(), HostError> {
        if self.connection.is_none() {
            self.connection = Some(self.start()?);
        }
        Ok(())
    }

    fn start(&self) -> Result<Connection, HostError> {
        let no_args: [&OsStr; 0] = [];
        let mut process = Sandboxed::spawn(&self.program, &no_args, &self.config.sandbox)
            .map_err(HostError::Spawn)?;
        let stdin = process.stdin.take().expect("stdin pipe");
        let mut stdout = process.stdout.take().expect("stdout pipe");
        let mut stderr = process.stderr.take().expect("stderr pipe");

        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let message = frame::receive::<_, WorkerResponse>(&mut stdout);
                let stop = !matches!(message, Ok(Some(_)));
                let forwarded = match message {
                    Ok(Some(response)) => Ok(response),
                    Ok(None) => break,
                    Err(error) => Err(error),
                };
                if sender.send(forwarded).is_err() || stop {
                    break;
                }
            }
        });
        // The worker's stderr is diagnostics only; keep draining it so it can never block.
        thread::spawn(move || {
            let mut sink = [0u8; 4096];
            while matches!(stderr.read(&mut sink), Ok(n) if n > 0) {}
        });

        let connection = Connection {
            process,
            stdin,
            responses,
        };
        let hello = receive(&connection.responses, self.config.handshake_timeout)?;
        frame::check_hello(&hello)
            .map_err(|error| HostError::ProtocolViolation(error.to_string()))?;
        Ok(connection)
    }
}

/// Waits for the next frame and validates it.
fn receive(
    responses: &Receiver<Result<WorkerResponse, FrameError>>,
    timeout: Duration,
) -> Result<WorkerResponse, HostError> {
    let response = match responses.recv_timeout(timeout) {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(HostError::ProtocolViolation(error.to_string())),
        Err(RecvTimeoutError::Timeout) => return Err(HostError::Timeout),
        Err(RecvTimeoutError::Disconnected) => return Err(HostError::Crashed),
    };
    response
        .validate()
        .map_err(|error| HostError::ProtocolViolation(error.to_string()))?;
    Ok(response)
}

fn response_request(response: &WorkerResponse) -> Option<RequestId> {
    match response {
        WorkerResponse::Hello { .. } => None,
        WorkerResponse::Opened { request, .. }
        | WorkerResponse::Rendered { request, .. }
        | WorkerResponse::Outline { request, .. }
        | WorkerResponse::PageLinks { request, .. }
        | WorkerResponse::PageAnnotations { request, .. }
        | WorkerResponse::PageFields { request, .. }
        | WorkerResponse::PageText { request, .. }
        | WorkerResponse::Png { request, .. }
        | WorkerResponse::Jpeg { request, .. }
        | WorkerResponse::PageSearched { request, .. }
        | WorkerResponse::Edited { request, .. }
        | WorkerResponse::Rebased { request }
        | WorkerResponse::Source { request, .. }
        | WorkerResponse::Saved { request, .. } => Some(*request),
        WorkerResponse::Error { request, .. } => *request,
    }
}
