//! Messages between the main process and `pdf_worker`, encoded with postcard inside
//! [`crate::frame`] frames. These types are never exposed to the frontend.
//!
//! Wire compatibility: postcard encodes enum variants by index. [`WorkerResponse::Hello`] must
//! stay the first variant so that a version mismatch is always detectable; other variants may
//! change freely while the contract is v0, because main and worker ship together.

use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;
use crate::types::{
    AnnotationId, DocumentId, DocumentPermissions, Edit, ErrorCode, FieldId, FormField,
    HighlightColor, HighlightMark, InkColor, InkWidth, OutlineResult, PageAnnotation, PageLink,
    PageSize, PageText, Password, Point, Rect, RequestId, Rotation, SearchHit, SecurityReport,
    StampName,
};

/// A file handle that the main process duplicated into the worker process: read-only for
/// `Open`, write-only for `Save`. The value is only meaningful inside the worker; the worker
/// never receives a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHandle(pub u64);

/// An edit as the worker applies it (ADR 0013). The frontend's form is [`Edit`], which postcard
/// cannot carry (it is tagged for JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerEdit {
    /// Adds `degrees` (90, 180 or 270) to the rotation of each of `pages`.
    RotatePages { pages: Vec<u32>, degrees: u16 },
    /// As [`Edit::DeletePages`].
    DeletePages { pages: Vec<u32> },
    /// As [`Edit::MovePages`].
    MovePages { pages: Vec<u32>, before: u32 },
    /// As [`Edit::InsertBlankPage`].
    InsertBlankPage { at: u32, like: u32 },
    /// Puts the pages of `source` (a plain PDF file, as `PrepareSource` made it) into the
    /// document from index `at` on, in their order (B2-06). Only what is on the pages comes
    /// along: not their annotations, links and form fields, and nothing active (see
    /// docs/architecture/merge.md).
    InsertPages { at: u32, source: Vec<u8> },
    /// As [`Edit::AddHighlight`].
    AddHighlight {
        marks: Vec<HighlightMark>,
        color: HighlightColor,
    },
    /// As [`Edit::AddNote`].
    AddNote { page: u32, at: Point, text: String },
    /// As [`Edit::DeleteAnnotation`].
    DeleteAnnotation { page: u32, annotation: AnnotationId },
    /// As [`Edit::SetHighlightColor`].
    SetHighlightColor {
        page: u32,
        annotation: AnnotationId,
        color: HighlightColor,
    },
    /// As [`Edit::SetNoteText`].
    SetNoteText {
        page: u32,
        annotation: AnnotationId,
        text: String,
    },
    /// As [`Edit::SetFieldValue`].
    SetFieldValue {
        page: u32,
        field: FieldId,
        value: String,
    },
    /// As [`Edit::FlattenForm`].
    FlattenForm,
    /// As [`Edit::AddInk`].
    AddInk {
        page: u32,
        strokes: Vec<Vec<Point>>,
        color: InkColor,
        width: InkWidth,
    },
    /// As [`Edit::AddStamp`].
    AddStamp {
        page: u32,
        rect: Rect,
        stamp: StampName,
    },
    /// As [`Edit::SetAnnotationRect`].
    SetAnnotationRect {
        page: u32,
        annotation: AnnotationId,
        rect: Rect,
    },
}

impl From<&Edit> for WorkerEdit {
    fn from(edit: &Edit) -> Self {
        match edit {
            Edit::RotatePages { pages, by } => WorkerEdit::RotatePages {
                pages: pages.clone(),
                degrees: by.degrees(),
            },
            Edit::DeletePages { pages } => WorkerEdit::DeletePages {
                pages: pages.clone(),
            },
            Edit::MovePages { pages, before } => WorkerEdit::MovePages {
                pages: pages.clone(),
                before: *before,
            },
            Edit::InsertBlankPage { at, like } => WorkerEdit::InsertBlankPage {
                at: *at,
                like: *like,
            },
            Edit::AddHighlight { marks, color } => WorkerEdit::AddHighlight {
                marks: marks.clone(),
                color: *color,
            },
            Edit::AddNote { page, at, text } => WorkerEdit::AddNote {
                page: *page,
                at: *at,
                text: text.clone(),
            },
            Edit::DeleteAnnotation { page, annotation } => WorkerEdit::DeleteAnnotation {
                page: *page,
                annotation: *annotation,
            },
            Edit::SetHighlightColor {
                page,
                annotation,
                color,
            } => WorkerEdit::SetHighlightColor {
                page: *page,
                annotation: *annotation,
                color: *color,
            },
            Edit::SetNoteText {
                page,
                annotation,
                text,
            } => WorkerEdit::SetNoteText {
                page: *page,
                annotation: *annotation,
                text: text.clone(),
            },
            Edit::SetFieldValue { page, field, value } => WorkerEdit::SetFieldValue {
                page: *page,
                field: *field,
                value: value.clone(),
            },
            Edit::FlattenForm => WorkerEdit::FlattenForm,
            Edit::AddInk {
                page,
                strokes,
                color,
                width,
            } => WorkerEdit::AddInk {
                page: *page,
                strokes: strokes.clone(),
                color: *color,
                width: *width,
            },
            Edit::AddStamp { page, rect, stamp } => WorkerEdit::AddStamp {
                page: *page,
                rect: *rect,
                stamp: *stamp,
            },
            Edit::SetAnnotationRect {
                page,
                annotation,
                rect,
            } => WorkerEdit::SetAnnotationRect {
                page: *page,
                annotation: *annotation,
                rect: *rect,
            },
        }
    }
}

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
    /// One page's annotations, other than links, form fields and pop-ups (B2-07).
    GetPageAnnotations {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
    },
    /// One page's form fields (B2-09).
    GetPageFields {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
    },
    /// One page as a PNG file, unturned, for exporting (B2-04).
    RenderPng {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        scale: f32,
    },
    /// One page as a JPEG file, unturned, for exporting (#111).
    RenderJpeg {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        scale: f32,
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
    /// Changes the document in memory (ADR 0013); the file is only written by `Save`.
    Edit {
        request: RequestId,
        doc: DocumentId,
        edit: WorkerEdit,
    },
    /// Writes the document, with its edits, to `file`: a write-only handle to a new temporary
    /// file the main process created (ADR 0013). A signed document is appended to, to keep its
    /// signatures valid; any other is rewritten without its unused objects.
    Save {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, without its
    /// metadata and with `id` as its identifier (B2-03). The open document is not changed.
    /// Answered by `Saved`, never appended.
    PrivacyCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        /// Random, from the main process.
        id: [u8; 16],
    },
    /// Opens the document again from the bytes it was opened from, and applies `edits` in
    /// order (undo, ADR 0013). Answered by `Edited`; the document is replaced only once every
    /// edit is applied. Refused for a document opened with a password, whose bytes are not
    /// kept (the password is not either).
    Revert {
        request: RequestId,
        doc: DocumentId,
        edits: Vec<WorkerEdit>,
        /// For a document opened with a password: the user typed it again (#94). Not kept.
        password: Option<Password>,
    },
    /// Keeps the bytes of `file`, read-only, as the ones undo opens `doc` again from: the file
    /// the document was just saved to, in place of the one it was opened from (ADR 0013). The
    /// document itself does not change; nothing is parsed, so no password is needed. Answered
    /// by `Rebased`.
    Rebase {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
    },
    /// Writes the pages `pages` (0-based, no repeats) of the document to `file` (write-only, as
    /// for `Save`) as a document of their own, in the order they have in the document: the
    /// document as it is now is copied and the other pages are taken out of the copy. The
    /// document itself does not change. Answered by `Saved` (`incremental` is false). An encrypted document is refused:
    /// the worker keeps no password, and a copy that was not encrypted again would drop what its
    /// author asked for (B2-06).
    SavePages {
        request: RequestId,
        doc: DocumentId,
        pages: Vec<u32>,
        file: FileHandle,
    },
    /// Makes the PDF in `file` (read-only) into what its pages are later taken from (B2-06): a
    /// plain, clean copy of it, with how many pages it has and what active content it has.
    /// `password` is for an encrypted file, wiped when the request is done. Answered by
    /// `Source`; nothing is kept in the worker.
    PrepareSource {
        request: RequestId,
        file: FileHandle,
        password: Option<Password>,
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
    PageAnnotations {
        request: RequestId,
        page_index: u32,
        annotations: Vec<PageAnnotation>,
    },
    PageFields {
        request: RequestId,
        page_index: u32,
        fields: Vec<FormField>,
    },
    PageText {
        request: RequestId,
        page_index: u32,
        text: PageText,
    },
    /// The PNG file of `RenderPng`.
    Png {
        request: RequestId,
        png: Vec<u8>,
    },
    /// The JPEG file of `RenderJpeg`.
    Jpeg {
        request: RequestId,
        jpeg: Vec<u8>,
    },
    PageSearched {
        request: RequestId,
        page_index: u32,
        hits: Vec<SearchHit>,
        /// Whether the page has any text at all (none on every page means no text layer).
        has_text: bool,
    },
    /// The edit is applied; `pages` are the document's pages now.
    Edited {
        request: RequestId,
        pages: Vec<PageSize>,
    },
    /// `Rebase` is done.
    Rebased {
        request: RequestId,
    },
    /// The copy of the file of `PrepareSource` (B2-06).
    Source {
        request: RequestId,
        bytes: Vec<u8>,
        pages: u32,
        security: SecurityReport,
    },
    /// The document was written: `bytes` long, appended to the original (`incremental`, for a
    /// signed document) or rewritten.
    Saved {
        request: RequestId,
        bytes: u64,
        incremental: bool,
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
    /// The document has a form with at least one field (B2-09).
    pub has_form: bool,
    pub security: SecurityReport,
    pub permissions: DocumentPermissions,
    /// Encrypted (MVP-16), with a password or with permissions only.
    pub encrypted: bool,
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
    /// The author of the document forbids what was asked of it (B2-06).
    NotAllowed,
    Unreadable,
    LimitExceeded,
    Cancelled,
    /// Saving: the disk is full.
    DiskFull,
    /// Saving: the file could not be written for another reason.
    Unwritable,
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
            WorkerErrorCode::NotAllowed => ErrorCode::NotAllowed,
            WorkerErrorCode::Unreadable => ErrorCode::Unreadable,
            WorkerErrorCode::LimitExceeded => ErrorCode::LimitExceeded,
            WorkerErrorCode::Cancelled => ErrorCode::Cancelled,
            WorkerErrorCode::DiskFull => ErrorCode::DiskFull,
            WorkerErrorCode::Unwritable => ErrorCode::Unwritable,
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
