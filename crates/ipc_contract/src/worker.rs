//! Messages between the main process and `pdf_worker`, encoded with postcard inside
//! [`crate::frame`] frames. These types are never exposed to the frontend.
//!
//! Wire compatibility: postcard encodes enum variants by index. [`WorkerResponse::Hello`] must
//! stay the first variant so that a version mismatch is always detectable; other variants may
//! change freely while the contract is v0, because main and worker ship together.

use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;
use crate::types::{
    DocumentId, ErrorCode, OutlineResult, PageLink, PageSize, RequestId, Rotation, SearchHit,
    SecurityReport,
};

/// A read-only file handle that the main process duplicated into the worker process.
/// The value is only meaningful inside the worker; the worker never receives a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHandle(pub u64);

/// Main process -> worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerRequest {
    Open {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
    },
    Render {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        scale: f32,
        rotation: Rotation,
    },
    GetOutline {
        request: RequestId,
        doc: DocumentId,
    },
    GetPageLinks {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
    },
    Search {
        request: RequestId,
        doc: DocumentId,
        query: String,
        case_sensitive: bool,
    },
    /// Best effort: the worker drops the target request if it has not finished yet.
    Cancel {
        target: RequestId,
    },
    Close {
        doc: DocumentId,
    },
    Shutdown,
}

/// Worker -> main process. Every value is untrusted until [`crate::validate`] accepts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerResponse {
    /// First frame after start-up. Must remain variant 0 (see module docs).
    Hello {
        protocol_version: u32,
        worker_version: String,
    },
    Opened {
        request: RequestId,
        document: OpenedDocument,
    },
    Rendered {
        request: RequestId,
        raster: Raster,
    },
    Outline {
        request: RequestId,
        outline: OutlineResult,
    },
    PageLinks {
        request: RequestId,
        page_index: u32,
        links: Vec<PageLink>,
    },
    /// Zero or more per search, followed by exactly one `SearchDone` or `Error`.
    SearchHits {
        request: RequestId,
        page_index: u32,
        hits: Vec<SearchHit>,
    },
    SearchProgress {
        request: RequestId,
        pages_searched: u32,
    },
    SearchDone {
        request: RequestId,
        total_hits: u32,
        truncated: bool,
    },
    Error {
        request: Option<RequestId>,
        error: WorkerError,
    },
}

impl WorkerResponse {
    /// The `Hello` the current worker build sends.
    pub fn hello() -> Self {
        WorkerResponse::Hello {
            protocol_version: PROTOCOL_VERSION,
            worker_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// What the worker learned while opening a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenedDocument {
    pub pages: Vec<PageSize>,
    pub has_outline: bool,
    pub security: SecurityReport,
}

/// A rendered page: opaque RGBA8 (alpha is always 255), rows top to bottom, no padding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerError {
    pub code: WorkerErrorCode,
    /// Diagnostic text; must not contain document content.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerErrorCode {
    InvalidRequest,
    UnknownDocument,
    PageOutOfRange,
    NotPdf,
    Corrupted,
    Encrypted,
    Unreadable,
    LimitExceeded,
    Cancelled,
    Internal,
}

impl From<WorkerErrorCode> for ErrorCode {
    fn from(code: WorkerErrorCode) -> Self {
        match code {
            WorkerErrorCode::InvalidRequest | WorkerErrorCode::PageOutOfRange => {
                ErrorCode::InvalidArgument
            }
            WorkerErrorCode::UnknownDocument => ErrorCode::UnknownDocument,
            WorkerErrorCode::NotPdf => ErrorCode::NotPdf,
            WorkerErrorCode::Corrupted => ErrorCode::Corrupted,
            WorkerErrorCode::Encrypted => ErrorCode::Encrypted,
            WorkerErrorCode::Unreadable => ErrorCode::Unreadable,
            WorkerErrorCode::LimitExceeded => ErrorCode::LimitExceeded,
            WorkerErrorCode::Cancelled => ErrorCode::Cancelled,
            WorkerErrorCode::Internal => ErrorCode::Internal,
        }
    }
}
