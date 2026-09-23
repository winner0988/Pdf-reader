//! Values visible to the frontend. Serialized as camelCase JSON over Tauri IPC and mirrored in
//! TypeScript by [`crate::typescript`]. Nothing in this module may carry a file path.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Opaque handle for an open document, assigned by the main process. Never a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct DocumentId(pub u32);

/// Caller-chosen id used to correlate and cancel a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct RequestId(pub u32);

/// Page size in PDF points (1/72 inch), before any view rotation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PageSize {
    pub width_pt: f32,
    pub height_pt: f32,
}

/// Clockwise view rotation. Only affects rendering; the file is never modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    pub fn degrees(self) -> u16 {
        match self {
            Rotation::None => 0,
            Rotation::Cw90 => 90,
            Rotation::Cw180 => 180,
            Rotation::Cw270 => 270,
        }
    }

    /// True when width and height swap.
    pub fn is_quarter_turn(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }
}

/// A point in page space: PDF points, origin at the top-left of the unrotated page, y down.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// An axis-aligned rectangle in page space (see [`Point`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// A quadrilateral in page space, used for text that may be rotated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct Quad {
    pub ul: Point,
    pub ur: Point,
    pub ll: Point,
    pub lr: Point,
}

/// Kinds of active content or remote references found in a document (MVP-11).
/// None of them is ever executed or followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    JavaScript,
    OpenAction,
    AdditionalActions,
    Launch,
    SubmitForm,
    ImportData,
    RemoteGoTo,
    EmbeddedGoTo,
    RemoteFileSpec,
    UncReference,
    Xfa,
    RichMedia,
    EmbeddedFile,
}

impl FindingKind {
    pub const ALL: [FindingKind; 13] = [
        FindingKind::JavaScript,
        FindingKind::OpenAction,
        FindingKind::AdditionalActions,
        FindingKind::Launch,
        FindingKind::SubmitForm,
        FindingKind::ImportData,
        FindingKind::RemoteGoTo,
        FindingKind::EmbeddedGoTo,
        FindingKind::RemoteFileSpec,
        FindingKind::UncReference,
        FindingKind::Xfa,
        FindingKind::RichMedia,
        FindingKind::EmbeddedFile,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SecurityFinding {
    pub kind: FindingKind,
    pub count: u32,
}

/// Result of the worker's active-content scan. Each kind appears at most once.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReport {
    pub findings: Vec<SecurityFinding>,
    /// False when the scan hit its time or size budget before finishing.
    pub scan_complete: bool,
}

/// An open document as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub doc: DocumentId,
    /// File name for display only (no directory components).
    pub display_name: String,
    /// One entry per page; the page count is `pages.length`.
    pub pages: Vec<PageSize>,
    pub has_outline: bool,
    pub security: SecurityReport,
}

/// Arguments of the `render_page` command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderPageArgs {
    pub request: RequestId,
    pub doc: DocumentId,
    pub page_index: u32,
    /// 1.0 = one PDF point per pixel. Includes the device pixel ratio.
    pub scale: f32,
    pub rotation: Rotation,
}

/// Actions that are recognised but never performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BlockedAction {
    Launch,
    RemoteGoTo,
    EmbeddedGoTo,
    JavaScript,
    SubmitForm,
    ImportData,
    Other,
}

/// Where a link or outline item points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkTarget {
    /// A page in the same document; `x`/`y` are page-space coordinates when specified.
    #[serde(rename_all = "camelCase")]
    Page {
        page_index: u32,
        x: Option<f32>,
        y: Option<f32>,
    },
    /// An external URI, shown to the user as-is. Opening it requires confirmation and goes
    /// through the main process by link id (MVP-12); the frontend never opens URIs itself.
    Uri { uri: String },
    /// A recognised action that is blocked. `target` is PDF-provided text for display only.
    Blocked {
        action: BlockedAction,
        target: Option<String>,
    },
}

/// Identifies a link reported by the worker, so the main process can look it up later
/// instead of trusting a URI string sent back by the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkId {
    pub page_index: u32,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PageLink {
    pub id: LinkId,
    pub rect: Rect,
    pub target: LinkTarget,
}

/// One outline entry, in pre-order; `depth` 0 is the top level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct OutlineItem {
    pub title: String,
    pub depth: u16,
    pub target: Option<LinkTarget>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
pub struct OutlineResult {
    pub items: Vec<OutlineItem>,
    /// True when the outline exceeded the item or depth limit and was cut short.
    pub truncated: bool,
}

/// Arguments of the `search` command. Results arrive on a Tauri channel as [`SearchEvent`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchArgs {
    pub request: RequestId,
    pub doc: DocumentId,
    pub query: String,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchHit {
    pub quads: Vec<Quad>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SearchEvent {
    #[serde(rename_all = "camelCase")]
    Hits {
        page_index: u32,
        hits: Vec<SearchHit>,
    },
    #[serde(rename_all = "camelCase")]
    Progress { pages_searched: u32 },
    #[serde(rename_all = "camelCase")]
    Done { total_hits: u32, truncated: bool },
}

/// Error codes the frontend maps to localized messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    UnknownDocument,
    InvalidArgument,
    Cancelled,
    NotPdf,
    Corrupted,
    /// Encrypted documents are not supported in the MVP.
    Encrypted,
    Unreadable,
    TooLarge,
    LimitExceeded,
    WorkerCrashed,
    WorkerTimeout,
    /// The worker sent a message that failed validation.
    ProtocolViolation,
    Internal,
}

/// Rejection value of every command. `message` is for logs and must not contain paths or
/// document content; the UI shows a localized text chosen by `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

/// Pushed by the main process on the channel passed to `subscribe_open_events`, for documents
/// opened from the dialog, by drag and drop, or from the command line. A window shows one
/// document at a time, so the latest event always describes what the window should show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenEvent {
    /// Files are being dragged over the window (`true`), or the drag left without a drop.
    DragHover { active: bool },
    /// Opening has started.
    #[serde(rename_all = "camelCase")]
    Opening { display_name: String },
    /// `ignored_files`: other files in the same drop that were not opened.
    #[serde(rename_all = "camelCase")]
    Opened {
        info: DocumentInfo,
        ignored_files: u32,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        display_name: String,
        error: IpcError,
        ignored_files: u32,
    },
}
