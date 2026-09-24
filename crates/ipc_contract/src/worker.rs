//! Messages between the main process and `pdf_worker`, encoded with postcard inside
//! [`crate::frame`] frames. These types are never exposed to the frontend.
//!
//! Wire compatibility: postcard encodes enum variants by index. [`WorkerResponse::Hello`] must
//! stay the first variant so that a version mismatch is always detectable; other variants may
//! change freely while the contract is v0, because main and worker ship together.

use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;
use crate::types::{
    DocumentId, ErrorCode, OutlineResult, PageLink, PageSize, PageText, Password, RequestId,
    Rotation, SearchHit, SecurityReport,
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
        /// For an encrypted document (MVP-16): tried if the document needs a password.
        password: Option<Password>,
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
    /// One page's text for selecting and copying (MVP-15).
    GetPageText {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
    },
    /// Searches one page. The main process walks the pages itself, so renders can run between
    /// pages and a search is cancelled by simply not asking for the next page.
    SearchPage {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        query: String,
        case_sensitive: bool,
        /// Stop after this many hits on the page.
        max_hits: u32,
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
    PageText {
        request: RequestId,
        page_index: u32,
        text: PageText,
    },
    PageSearched {
        request: RequestId,
        page_index: u32,
        hits: Vec<SearchHit>,
        /// Whether the page has any text at all (none on every page means no text layer).
        has_text: bool,
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
    /// A password is needed, and none was given.
    Encrypted,
    /// The given password does not open the document.
    WrongPassword,
    UnsupportedEncryption,
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
            WorkerErrorCode::Encrypted | WorkerErrorCode::WrongPassword => ErrorCode::Encrypted,
            WorkerErrorCode::UnsupportedEncryption => ErrorCode::UnsupportedEncryption,
            WorkerErrorCode::Unreadable => ErrorCode::Unreadable,
            WorkerErrorCode::LimitExceeded => ErrorCode::LimitExceeded,
            WorkerErrorCode::Cancelled => ErrorCode::Cancelled,
            WorkerErrorCode::Internal => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockedAction, LinkTarget, OutlineItem};

    fn targets() -> Vec<LinkTarget> {
        vec![
            LinkTarget::Page {
                page_index: 3,
                x: Some(10.0),
                y: None,
            },
            LinkTarget::Uri {
                uri: "https://example.invalid/".to_owned(),
            },
            LinkTarget::Blocked {
                action: BlockedAction::Launch,
                target: Some("calc.exe".to_owned()),
            },
        ]
    }

    #[test]
    fn password_errors_reach_the_frontend_as_encryption_codes() {
        assert_eq!(
            ErrorCode::from(WorkerErrorCode::WrongPassword),
            ErrorCode::Encrypted
        );
        assert_eq!(
            ErrorCode::from(WorkerErrorCode::UnsupportedEncryption),
            ErrorCode::UnsupportedEncryption
        );
    }

    #[test]
    fn an_open_request_carries_its_password_and_hides_it() {
        let request = WorkerRequest::Open {
            request: RequestId(1),
            doc: DocumentId(2),
            file: FileHandle(3),
            password: Some(Password::new("s3cret".to_owned())),
        };
        assert!(!format!("{request:?}").contains("s3cret"));
        let bytes = postcard::to_stdvec(&request).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerRequest>(&bytes).unwrap(),
            request
        );
    }

    #[test]
    fn outlines_and_links_cross_the_worker_boundary() {
        // postcard cannot decode internally tagged enums; LinkTarget must still get through.
        let response = WorkerResponse::Outline {
            request: RequestId(1),
            outline: OutlineResult {
                items: targets()
                    .into_iter()
                    .map(|target| OutlineItem {
                        title: "t".to_owned(),
                        depth: 0,
                        target: Some(target),
                    })
                    .collect(),
                truncated: true,
            },
        };
        let bytes = postcard::to_allocvec(&response).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerResponse>(&bytes).unwrap(),
            response
        );
    }

    #[test]
    fn the_frontend_sees_link_targets_tagged_by_kind() {
        let json: Vec<serde_json::Value> = targets()
            .iter()
            .map(|target| serde_json::to_value(target).unwrap())
            .collect();
        assert_eq!(
            json[0],
            serde_json::json!({ "kind": "page", "pageIndex": 3, "x": 10.0, "y": null })
        );
        assert_eq!(
            json[1],
            serde_json::json!({ "kind": "uri", "uri": "https://example.invalid/" })
        );
        assert_eq!(
            json[2],
            serde_json::json!({ "kind": "blocked", "action": "launch", "target": "calc.exe" })
        );
        for (value, target) in json.into_iter().zip(targets()) {
            assert_eq!(serde_json::from_value::<LinkTarget>(value).unwrap(), target);
        }
    }
}
