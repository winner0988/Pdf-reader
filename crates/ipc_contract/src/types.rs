//! Values visible to the frontend. Serialized as camelCase JSON over Tauri IPC and mirrored in
//! TypeScript by [`crate::typescript`]. Nothing in this module may carry a file path.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;
use zeroize::Zeroize;

/// Opaque handle for an open document, assigned by the main process. Never a path. It stands for
/// the document's content: every edit gives the document a new id (B2-02), so pages, text and
/// links fetched for an earlier id are never mistaken for the current ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct DocumentId(pub u32);

/// One tab of the window (MVP-14, ADR 0012): a file from the moment it starts opening until the
/// tab is closed, whether it opened or failed. Assigned by the main process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct TabId(pub u32);

/// An entry of the recently opened files (#73), valid while the app runs. The main process keeps
/// the entry's path; the frontend only ever gets this id and the file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct RecentId(pub u32);

/// A password the user typed to open an encrypted document (MVP-16, docs/architecture/encryption.md).
/// It lives only while the document is being opened: it is never stored or logged (`Debug` does
/// not show it), and its memory is wiped when it is dropped.
#[derive(Clone, PartialEq, Eq, TS)]
pub struct Password(String);

impl Password {
    pub fn new(password: String) -> Self {
        Self(password)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(..)")
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A plain string on the wire.
impl Serialize for Password {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Password {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Password)
    }
}

/// Arguments of `unlock_tab`: the password for a tab that asked for one. Any other field is
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnlockArgs {
    pub tab: TabId,
    pub password: Password,
}

/// Arguments of `undo_edit` (B2-05). A document opened with a password is opened again to undo
/// an edit, so its password is asked for again and sent here (#94); it is not kept. Any other
/// field is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoArgs {
    pub doc: DocumentId,
    pub password: Option<Password>,
}

/// A recently opened file as the frontend sees it (#73): no path, only the file name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub id: RecentId,
    /// File name for display only (no directory components).
    pub display_name: String,
}

/// Which colours the app uses (B2-12): the system's light or dark mode, or always one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// The user's settings (B2-12), kept by the main process in the app's local data folder. The
/// frontend reads them and sends the whole set back; any other field is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub theme: ThemePreference,
    /// Whether files that open go on the recent files list (#73).
    pub record_recent_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            record_recent_files: true,
        }
    }
}

/// What an export writes (B2-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportFormat {
    /// The pages' text, one UTF-8 `.txt` file.
    Text,
    /// One PNG file per page at `dpi` dots per inch: 72, 150 or 300.
    Png { dpi: u32 },
    /// One JPEG file per page at `dpi` dots per inch, as for PNG (#111).
    Jpg { dpi: u32 },
}

/// Arguments of `export_pages` (B2-04): what to export, never where; the main process asks the
/// user. Any other field is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportArgs {
    /// Stops the export with `cancel`.
    pub request: RequestId,
    pub doc: DocumentId,
    /// Zero-based, in the order they are exported; at most `MAX_EXPORT_PAGES`, no repeats.
    pub pages: Vec<u32>,
    pub format: ExportFormat,
}

/// Progress of `export_pages` on its channel (B2-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportEvent {
    #[serde(rename_all = "camelCase")]
    Progress { pages_done: u32, total: u32 },
}

/// A change to an open document (ADR 0013), applied in its worker. Pages are 0-based and are
/// those of the document as it is before the edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Edit {
    /// Turns `pages` (no repeats) clockwise by `by`, on top of their current rotation.
    RotatePages { pages: Vec<u32>, by: Rotation },
    /// Removes `pages` (no repeats); at least one page must remain (B2-05).
    DeletePages { pages: Vec<u32> },
    /// Moves `pages` (no repeats) to just before page `before` (the page count: to the end),
    /// together and in their current order; the other pages keep theirs (B2-05).
    MovePages { pages: Vec<u32>, before: u32 },
    /// Inserts a blank page at index `at` (the page count: after the last page), upright and the
    /// size page `like` is shown at (B2-05).
    InsertBlankPage { at: u32, like: u32 },
    /// Marks text of page `page` with a highlighter (a `Highlight` annotation) over `quads`, in
    /// page space as text selection gives them: at least one, at most `MAX_ANNOTATION_QUADS`
    /// (B2-07).
    AddHighlight {
        page: u32,
        quads: Vec<Quad>,
        color: HighlightColor,
    },
    /// Puts a note (a `Text` annotation) saying `text` at `at` on page `page` (B2-07).
    AddNote { page: u32, at: Point, text: String },
    /// Removes annotation `annotation` of page `page`: one the app added, or one the document
    /// had (B2-07).
    DeleteAnnotation { page: u32, annotation: AnnotationId },
    /// Gives the highlighter annotation `annotation` of page `page` another color (B2-07).
    SetHighlightColor {
        page: u32,
        annotation: AnnotationId,
        color: HighlightColor,
    },
    /// Replaces what the note `annotation` of page `page` says (B2-07).
    SetNoteText {
        page: u32,
        annotation: AnnotationId,
        text: String,
    },
}

/// The highlighter's colors (B2-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HighlightColor {
    Yellow,
    Green,
    Blue,
    Pink,
}

/// An annotation of an open document: the number of its object in the document, which stays
/// the same through edits until the document is saved (B2-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct AnnotationId(pub u32);

/// What an annotation of a page is, as far as the app edits it (B2-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationKind {
    /// A highlighter mark (`Highlight`): its color can be changed.
    Highlight,
    /// A note (`Text`): what it says can be changed.
    Note,
    /// Any other kind (a drawing, a stamp, a comment box): it can only be removed.
    Other,
}

/// An annotation of a page (B2-07), from the page's worker; links, form fields and pop-up
/// windows are not listed. `text` is what a note says, cleaned like any text from a PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PageAnnotation {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    /// Where it is on the page (page space).
    pub rect: Rect,
    /// A highlighter mark in one of the app's colors.
    pub color: Option<HighlightColor>,
    pub text: Option<String>,
}

/// Arguments of `apply_edit` (B2-02). Any other field is rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditArgs {
    pub doc: DocumentId,
    pub edit: Edit,
}

/// What `check_for_updates` found (#64, ADR 0009). Versions are `major.minor.patch`; the latest
/// one is rewritten from its three numbers, so nothing else from GitHub's answer reaches the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateCheck {
    /// No release is newer than this app.
    UpToDate { current: String },
    /// A newer release exists; the page offers the releases page (`open_releases_page`).
    Available { current: String, latest: String },
    /// The project has not published a release yet.
    NoRelease { current: String },
}

/// How `save_document` or `save_document_as` wrote the file (B2-02, ADR 0013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    /// Appended to the file instead of rewriting it, to keep its signatures valid: content that
    /// was removed may still be in the file.
    pub incremental: bool,
}

/// Arguments of `set_file_recording` (#73): whether an open document's file may be on the recent
/// files list. Any other field is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRecordingArgs {
    pub doc: DocumentId,
    pub record: bool,
}

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

/// A clockwise rotation in quarter turns: of the view in `render_page`, which never changes the
/// file, or of pages in `Edit::RotatePages`.
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

/// What the document's author allows (MVP-19). An encrypted PDF can forbid copying its text and
/// printing it; the app obeys, as Adobe Acrobat does. An unencrypted document allows everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPermissions {
    pub copy: bool,
    pub print: bool,
    /// Printing at full quality; without it only a low-resolution image is printed.
    pub print_high_quality: bool,
    /// Changing the document (`/P` bit 4).
    pub modify: bool,
    /// Inserting, deleting and rotating pages (`/P` bit 11, or bit 4 before revision 3).
    pub assemble: bool,
    /// Adding, changing and removing annotations (`/P` bit 6, B2-07).
    pub annotate: bool,
}

impl DocumentPermissions {
    pub const ALL: Self = Self {
        copy: true,
        print: true,
        print_high_quality: true,
        modify: true,
        assemble: true,
        annotate: true,
    };
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
    pub permissions: DocumentPermissions,
    /// Changed since it was opened or last saved (B2-02): the file does not have the changes yet.
    pub unsaved: bool,
    /// Encrypted (MVP-16): it has no privacy export (B2-03).
    pub encrypted: bool,
    /// An edit made since the file was last written can be undone (B2-05, ADR 0013). A document
    /// opened with a password asks for it again to undo: it is not kept (MVP-16, #94).
    pub can_undo: bool,
    /// An edit that was undone can be made again.
    pub can_redo: bool,
    /// Unsaved changes an earlier run of the app left for this file (B2-13, ADR 0013).
    pub recovery: Recovery,
}

/// Changes to a file that an earlier run of the app made but neither saved nor discarded: it
/// ended first (B2-13, ADR 0013). They are kept in the app's local data folder until the user
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Recovery {
    /// There are none, or the user has answered.
    None,
    /// They can be made again (`recover_edits`): the file is as it was when they were made.
    Available,
    /// Another program changed the file since: they cannot be made on it any more.
    Stale,
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
    /// A `file:` URI on this computer.
    LocalFile,
    /// A UNC path (`\\server\share`), `smb:` or `file://server`: on Windows, following it can
    /// send the user's account hash to that server.
    NetworkShare,
    Other,
}

/// Where a link or outline item points.
///
/// Serialized with a `kind` tag in JSON (for the frontend), but externally tagged in binary
/// formats: postcard, used between the main process and the worker, cannot decode internally
/// tagged enums. See [`link_target_serde`].
#[derive(Debug, Clone, PartialEq, TS)]
#[ts(tag = "kind", rename_all = "camelCase")]
pub enum LinkTarget {
    /// A page in the same document; `x`/`y` are page-space coordinates when specified.
    #[ts(rename_all = "camelCase")]
    Page {
        page_index: u32,
        x: Option<f32>,
        y: Option<f32>,
    },
    /// An external `http`, `https` or `mailto` URI, exactly as the PDF has it (hidden characters
    /// included, so that the confirmation can show them). Opening it requires confirmation and
    /// goes through the main process by link id (MVP-12); the frontend never opens URIs itself.
    Uri { uri: String },
    /// A recognised action that is blocked. `target` is PDF-provided text for display only.
    Blocked {
        action: BlockedAction,
        target: Option<String>,
    },
}

/// Chooses the representation of [`LinkTarget`] by format: tagged for human-readable formats
/// (JSON to the frontend), externally tagged otherwise (postcard to and from the worker).
mod link_target_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{BlockedAction, LinkTarget};

    #[derive(Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    enum Tagged {
        #[serde(rename_all = "camelCase")]
        Page {
            page_index: u32,
            x: Option<f32>,
            y: Option<f32>,
        },
        Uri {
            uri: String,
        },
        Blocked {
            action: BlockedAction,
            target: Option<String>,
        },
    }

    #[derive(Serialize, Deserialize)]
    enum Compact {
        Page {
            page_index: u32,
            x: Option<f32>,
            y: Option<f32>,
        },
        Uri {
            uri: String,
        },
        Blocked {
            action: BlockedAction,
            target: Option<String>,
        },
    }

    macro_rules! convert {
        ($from:ident => $to:ident) => {
            impl From<$from> for $to {
                fn from(target: $from) -> Self {
                    match target {
                        $from::Page { page_index, x, y } => $to::Page { page_index, x, y },
                        $from::Uri { uri } => $to::Uri { uri },
                        $from::Blocked { action, target } => $to::Blocked { action, target },
                    }
                }
            }
        };
    }
    convert!(LinkTarget => Tagged);
    convert!(LinkTarget => Compact);
    convert!(Tagged => LinkTarget);
    convert!(Compact => LinkTarget);

    impl Serialize for LinkTarget {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            if serializer.is_human_readable() {
                Tagged::from(self.clone()).serialize(serializer)
            } else {
                Compact::from(self.clone()).serialize(serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for LinkTarget {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            if deserializer.is_human_readable() {
                Tagged::deserialize(deserializer).map(Into::into)
            } else {
                Compact::deserialize(deserializer).map(Into::into)
            }
        }
    }
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

/// Arguments of `describe_link` and `open_link` (MVP-12): a link the worker reported, by id.
/// There is deliberately no field for a URI; any other field is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkArgs {
    pub doc: DocumentId,
    pub link: LinkId,
}

/// Arguments of `describe_outline_link` and `open_outline_link` (#49): an outline item by its
/// position in the outline the worker reported (`OutlineResult::items`), never a URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutlineLinkArgs {
    pub doc: DocumentId,
    pub item: u32,
}

/// What the confirmation shows about a web link before it is opened (MVP-12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    /// The URI exactly as the PDF has it; the frontend writes out its hidden characters.
    pub uri: String,
    /// What the system is given when the user opens it: ASCII only, the host in punycode and
    /// everything else percent-encoded. Also what "copy link" copies.
    pub opens: String,
    /// The site the browser will contact (for `mailto`, the mail domain), as it reads.
    pub host: Option<String>,
    /// The host's ASCII (punycode) form when it differs from `host`: an internationalised
    /// name, which may imitate another one.
    pub ascii_host: Option<String>,
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
    /// `no_text_layer`: no page had any text, so the document needs OCR to be searchable.
    #[serde(rename_all = "camelCase")]
    Done {
        total_hits: u32,
        truncated: bool,
        no_text_layer: bool,
    },
}

/// One page's text for selecting and copying (MVP-15): its lines in reading order, from the
/// same text layer that search looks at.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
pub struct PageText {
    pub lines: Vec<TextLine>,
    /// The page had more than `LIMITS.maxPageTextChars` characters; the rest was left out.
    pub truncated: bool,
}

/// A line of text in page space. Its characters sit side by side from the quad's `ul` towards
/// `ur` (the writing direction, which need not be horizontal), and the line reaches from `ul`
/// to `ll` across.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct TextLine {
    /// Plain text: every kind of whitespace is a space, and there are no control or invisible
    /// formatting characters (they could hide or reorder what is pasted).
    pub text: String,
    pub quad: Quad,
    /// Where each character of `text` (a Unicode scalar value) starts, in points from `ul`
    /// towards `ur`, then where the last one ends: one more value than characters, never
    /// decreasing.
    pub edges: Vec<f32>,
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
    /// The document needs a password (the tab asks for it, MVP-16).
    Encrypted,
    /// Encrypted in a way the app cannot open (not the standard password security handler, for
    /// example with a certificate).
    UnsupportedEncryption,
    Unreadable,
    TooLarge,
    LimitExceeded,
    WorkerCrashed,
    WorkerTimeout,
    /// The worker sent a message that failed validation.
    ProtocolViolation,
    /// Saving (B2-02): the file or its folder cannot be written (read-only, or no access).
    ReadOnly,
    /// Saving: the disk is full.
    DiskFull,
    /// Saving: another program has the file open.
    FileInUse,
    /// Saving: another program changed the file after it was opened; overwriting it would lose
    /// that change, so the user is asked to save a copy instead.
    ChangedOnDisk,
    /// Saving: the file could not be written for another reason.
    Unwritable,
    /// The update check (#64) got no usable answer from GitHub: offline, blocked, or an answer
    /// it did not expect.
    NetworkFailed,
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
/// opened from the dialog, by drag and drop, from the command line or from a second launch of
/// the app. Every file gets its own tab (MVP-14): `Opening` adds it, then `Opened` or `Failed`
/// says how it went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenEvent {
    /// Files are being dragged over the window (`true`), or the drag left without a drop.
    DragHover { active: bool },
    /// A tab was added (or a failed one is being retried) and its file is opening.
    #[serde(rename_all = "camelCase")]
    Opening { tab: TabId, display_name: String },
    #[serde(rename_all = "camelCase")]
    Opened { tab: TabId, info: DocumentInfo },
    /// The tab's file is encrypted and needs a password; `wrong` after one that did not open it.
    #[serde(rename_all = "camelCase")]
    PasswordNeeded {
        tab: TabId,
        display_name: String,
        wrong: bool,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        tab: TabId,
        display_name: String,
        error: IpcError,
    },
    /// `ignored_files` files were not opened: the window already has `MAX_TABS` tabs.
    #[serde(rename_all = "camelCase")]
    TabLimit { ignored_files: u32 },
    /// The user asked to close the window while `tabs` have unsaved changes (B2-02): the window
    /// stays open until the frontend has asked what to do and calls `close_window`.
    CloseRequested { tabs: Vec<TabId> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_travel_as_camel_case_and_take_no_other_field() {
        let settings = Settings {
            theme: ThemePreference::Dark,
            record_recent_files: false,
        };
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "theme": "dark", "recordRecentFiles": false })
        );
        assert!(
            serde_json::from_value::<Settings>(
                serde_json::json!({ "theme": "dark", "recordRecentFiles": false, "path": "C:/x" })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Settings>(
                serde_json::json!({ "theme": "sepia", "recordRecentFiles": true })
            )
            .is_err()
        );
        assert_eq!(Settings::default().theme, ThemePreference::System);
        assert!(Settings::default().record_recent_files);
    }
}
