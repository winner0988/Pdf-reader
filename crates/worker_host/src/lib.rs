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
use ipc_contract::types::{DocumentId, ErrorCode, RequestId};
use ipc_contract::validate::Validate;
use ipc_contract::worker::{FileHandle, WorkerError, WorkerRequest, WorkerResponse};
use sandbox::{SandboxConfig, Sandboxed};
use thiserror::Error;

/// Largest document the worker is asked to open.
pub const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

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
        })?;
        Ok((doc, response))
    }

    /// Sends a request built by `make` (which receives a fresh request id) and returns the
    /// validated response. Worker-reported errors come back as [`HostError::Worker`].
    pub fn request(
        &mut self,
        make: impl FnOnce(RequestId) -> WorkerRequest,
    ) -> Result<WorkerResponse, HostError> {
        self.ensure_running()?;
        let id = RequestId(self.next_request);
        self.next_request = self.next_request.wrapping_add(1).max(1);
        let request = make(id);

        let result = self.exchange(&request, id);
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
        if frame::send(&mut connection.stdin, request).is_err() {
            self.connection = None;
            return Err(HostError::Crashed);
        }
        Ok(())
    }

    fn exchange(
        &mut self,
        request: &WorkerRequest,
        id: RequestId,
    ) -> Result<WorkerResponse, HostError> {
        let timeout = self.config.request_timeout;
        let connection = self.connection.as_mut().expect("running");
        frame::send(&mut connection.stdin, request).map_err(|_| HostError::Crashed)?;
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
        | WorkerResponse::PageText { request, .. }
        | WorkerResponse::PageSearched { request, .. } => Some(*request),
        WorkerResponse::Error { request, .. } => *request,
    }
}
