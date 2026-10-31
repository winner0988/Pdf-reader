//! Messages between the main process and `pdf_worker`, encoded with postcard inside
//! [`crate::frame`] frames. These types are never exposed to the frontend.
//!
//! Wire compatibility: postcard encodes enum variants by index. [`WorkerResponse::Hello`] must
//! stay the first variant so that a version mismatch is always detectable; other variants may
//! change freely while the contract is v0, because main and worker ship together.

use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;
use crate::types::{
    AnnotationId, AnnotationSummary, AttachmentId, AttachmentSource, BatesMark, BookmarkId,
    CreateKind, DocumentId, DocumentPermissions, Edit, ErrorCode, FieldId, FindPersonalDataQuery,
    FontRefusal, FormField, HeaderFooterMarks, HighlightColor, HighlightMark, ImageQuality,
    InkColor, InkWidth, OutlineResult, PageAnnotation, PageCorner, PageLink, PageMarkPages,
    PageMarkPosition, PageMarks, PageNumberFormat, PageNumberMark, PageSize, PageText, Password,
    PersonalDataMatches, Point, RasterRegion, Rect, RedactionCheck, RedactionSummary, RequestId,
    Restrictions, Rgb, Rotation, SanitizeOptions, SanitizeReport, SearchHit, SecurityReport,
    SignatureReport, SourceId, StampImageId, StampName, TextBoxInfo, TextBoxModelId, TextBoxStyle,
    TextMarkKind, Watermark, WatermarkContent, WatermarkPages, WatermarkPlace,
};

/// An attachment of a document as reported by the worker (B3-20, ADR 0025).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerAttachment {
    pub id: AttachmentId,
    pub name: String,
    pub size: Option<u64>,
    pub description: Option<String>,
    pub source: AttachmentSource,
}

/// A file handle that the main process duplicated into the worker process: read-only for
/// `Open`, write-only for `Save`. The value is only meaningful inside the worker; the worker
/// never receives a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHandle(pub u64);

/// A font file as the worker is given it for a text box (B2-28, ADR 0018): the bytes the main
/// process read from an installed font (not trusted: only the worker reads them), which font of
/// the file is meant (0 unless it is a collection) and the name Windows lists it by.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontData {
    pub bytes: Vec<u8>,
    pub face: u32,
    pub name: String,
}

impl std::fmt::Debug for FontData {
    /// Not the bytes: a font is up to 64 MiB.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("bytes", &self.bytes.len())
            .field("face", &self.face)
            .field("name", &self.name)
            .finish()
    }
}

/// One face of a font file the worker inspected (B2-32, ADR 0018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectedFont {
    pub face: u32,
    pub name: String,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

/// One character of the words of a text box: the glyph the font has for it, numbered in the
/// subset, and how far it moves the pen (B2-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelGlyph {
    pub ch: char,
    /// The glyph's number in the subset (1 and up: 0 is the glyph for what is missing).
    pub cid: u16,
    /// In thousandths of the size of the text.
    pub advance: u16,
}

/// The words of a text box and the font they are set in, made once from the font and then all
/// that is needed to set them again: on another page, at another size, in another document, and
/// in a crash recovery journal, none of which has the font (B2-28, ADR 0018: the journal keeps a
/// subset, not the whole font). The worker makes it from a font file, which is not trusted; a
/// model that comes back from disk or from the worker is checked like any other value
/// ([`crate::validate`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBoxModel {
    /// Lines are broken at `\n` and where the box is not wide enough.
    pub text: String,
    /// What Windows calls the font, to find it again; and which font of its file.
    pub font: String,
    pub face: u32,
    /// The font's licence lets a PDF with it be viewed and printed only (OS/2 `fsType` 4).
    pub view_only: bool,
    /// The name of the subset in the file: `ABCDEF+Name`.
    pub base: String,
    /// In thousandths of the size of the text.
    pub ascent: i32,
    pub descent: i32,
    pub cap_height: i32,
    /// The distance between two lines.
    pub step: i32,
    /// The box the glyphs fit in, `[left, bottom, right, top]`.
    pub bbox: [i32; 4],
    /// The glyphs of the characters of `text`, once each.
    pub glyphs: Vec<ModelGlyph>,
    /// The font cut to those glyphs: TrueType, in the order of `cid`. As JSON (the journal) it is
    /// lowercase hex text, as the pictures of a journal are.
    #[serde(with = "hex_when_text")]
    pub subset: Vec<u8>,
}

/// The outcome of preparing a single text box within a batch request (B3-23, Issue #381).
///
/// When `Deferred` appears, at least one `Prepared` must precede it, and all subsequent
/// items must also be `Deferred`. These two rules ensure the main process makes progress
/// on at least one box per round, preventing the worker from causing infinite resends (Issue #391).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PreparedTextBoxModel {
    /// The text was successfully shaped and subset into a model.
    Prepared(TextBoxModel),
    /// The font cannot shape or subset this text (e.g. missing glyphs, unsupported script, or font refusal).
    Refused,
    /// Not processed in this batch to keep the response within size limits.
    Deferred,
}

/// A text box found in a source file prepared for inserting into a document (B3-23, ADR 0019).
/// The worker reports its text, geometry and style so the main process can prepare a model in
/// an available font (the worker has no fonts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceTextBox {
    pub page: u32,
    pub rect: Rect,
    pub text: String,
    pub font_family: String,
    pub style: TextBoxStyle,
}

/// A text box prepared by the main process to be placed on an inserted page (B3-23, ADR 0019).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertedTextBox {
    pub page: u32,
    pub rect: Rect,
    pub model: TextBoxModel,
    pub style: TextBoxStyle,
}

/// Bytes that are hex text in JSON and plain bytes in postcard.
mod hex_when_text {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            let mut text = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                text.push(char::from(DIGITS[usize::from(byte >> 4)]));
                text.push(char::from(DIGITS[usize::from(byte & 15)]));
            }
            serializer.serialize_str(&text)
        } else {
            bytes.serialize(serializer)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        if !deserializer.is_human_readable() {
            return Vec::<u8>::deserialize(deserializer);
        }
        let text = String::deserialize(deserializer)?;
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let (pairs, rest) = text.as_bytes().as_chunks::<2>();
        if !rest.is_empty() {
            return Err(D::Error::custom("odd number of hex digits"));
        }
        pairs
            .iter()
            .map(|&[high, low]| Some(digit(high)? << 4 | digit(low)?))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| D::Error::custom("not lowercase hex"))
    }
}

/// What a signed copy says of its signature, besides the room for it (B3-06, ADR 0020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSignature {
    /// Who signs, as their certificate names them (its common name): `/Name`.
    pub signer: String,
    /// Why, as the signer says: `/Reason`.
    pub reason: Option<String>,
    /// When, on the computer's clock: `/M`.
    pub time: SigningTime,
    /// A box on a page that shows the signature; none: it is seen on no page.
    pub visible: Option<VisibleSignature>,
}

/// A time on the computer's clock, in its time zone (B3-06): what a signature's `/M` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningTime {
    pub year: u16,
    /// 1 to 12.
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Minutes east of UTC.
    pub utc_offset_minutes: i16,
}

/// The box that shows a signature on a page (B3-06): in a corner of page `page_index` as it is
/// shown, with `lines` in it (the signer, the date, the reason; the main process words them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleSignature {
    pub page_index: u32,
    pub corner: PageCorner,
    pub lines: Vec<String>,
}

/// A watermark as the worker takes it (B3-04). The frontend's form is [`Watermark`], whose
/// content postcard cannot carry (it is tagged for JSON); this one is the same, field by field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerWatermark {
    pub content: WorkerWatermarkContent,
    pub opacity: u8,
    pub diagonal: bool,
    pub place: WatermarkPlace,
    pub pages: WatermarkPages,
}

/// What a watermark is, as the worker takes it (see [`WorkerWatermark`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerWatermarkContent {
    Text { text: String, size: f32, color: Rgb },
    Picture { width_pt: f32 },
}

impl From<Watermark> for WorkerWatermark {
    fn from(watermark: Watermark) -> Self {
        Self {
            content: match watermark.content {
                WatermarkContent::Text { text, size, color } => {
                    WorkerWatermarkContent::Text { text, size, color }
                }
                WatermarkContent::Picture { width_pt } => {
                    WorkerWatermarkContent::Picture { width_pt }
                }
            },
            opacity: watermark.opacity,
            diagonal: watermark.diagonal,
            place: watermark.place,
            pages: watermark.pages,
        }
    }
}

impl From<WorkerWatermark> for Watermark {
    fn from(watermark: WorkerWatermark) -> Self {
        Self {
            content: match watermark.content {
                WorkerWatermarkContent::Text { text, size, color } => {
                    WatermarkContent::Text { text, size, color }
                }
                WorkerWatermarkContent::Picture { width_pt } => {
                    WatermarkContent::Picture { width_pt }
                }
            },
            opacity: watermark.opacity,
            diagonal: watermark.diagonal,
            place: watermark.place,
            pages: watermark.pages,
        }
    }
}

/// Page marks as the worker takes them (B3-16). The frontend's form is [`PageMarks`], whose
/// tagged enums postcard cannot carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerPageMarks {
    pub page_number: Option<WorkerPageNumberMark>,
    pub bates: Option<WorkerBatesMark>,
    pub header_footer: Option<HeaderFooterMarks>,
    pub font_size: f32,
    pub color: Rgb,
    pub margin_pt: f32,
    pub pages: WorkerPageMarkPages,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPageNumberMark {
    pub format: WorkerPageNumberFormat,
    pub start: u32,
    pub position: PageMarkPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerPageNumberFormat {
    Number,
    Page,
    PageOfTotal,
    Custom { prefix: String, suffix: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerBatesMark {
    pub prefix: String,
    pub digits: u8,
    pub start: u64,
    pub position: PageMarkPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerPageMarkPages {
    All,
    AllButFirst,
    Custom { pages: Vec<u32> },
}

impl From<PageNumberFormat> for WorkerPageNumberFormat {
    fn from(format: PageNumberFormat) -> Self {
        match format {
            PageNumberFormat::Number => Self::Number,
            PageNumberFormat::Page => Self::Page,
            PageNumberFormat::PageOfTotal => Self::PageOfTotal,
            PageNumberFormat::Custom { prefix, suffix } => Self::Custom { prefix, suffix },
        }
    }
}

impl From<WorkerPageNumberFormat> for PageNumberFormat {
    fn from(format: WorkerPageNumberFormat) -> Self {
        match format {
            WorkerPageNumberFormat::Number => Self::Number,
            WorkerPageNumberFormat::Page => Self::Page,
            WorkerPageNumberFormat::PageOfTotal => Self::PageOfTotal,
            WorkerPageNumberFormat::Custom { prefix, suffix } => Self::Custom { prefix, suffix },
        }
    }
}

impl From<PageNumberMark> for WorkerPageNumberMark {
    fn from(mark: PageNumberMark) -> Self {
        Self {
            format: mark.format.into(),
            start: mark.start,
            position: mark.position,
        }
    }
}

impl From<WorkerPageNumberMark> for PageNumberMark {
    fn from(mark: WorkerPageNumberMark) -> Self {
        Self {
            format: mark.format.into(),
            start: mark.start,
            position: mark.position,
        }
    }
}

impl From<BatesMark> for WorkerBatesMark {
    fn from(mark: BatesMark) -> Self {
        Self {
            prefix: mark.prefix,
            digits: mark.digits,
            start: mark.start,
            position: mark.position,
        }
    }
}

impl From<WorkerBatesMark> for BatesMark {
    fn from(mark: WorkerBatesMark) -> Self {
        Self {
            prefix: mark.prefix,
            digits: mark.digits,
            start: mark.start,
            position: mark.position,
        }
    }
}

impl From<PageMarkPages> for WorkerPageMarkPages {
    fn from(pages: PageMarkPages) -> Self {
        match pages {
            PageMarkPages::All => Self::All,
            PageMarkPages::AllButFirst => Self::AllButFirst,
            PageMarkPages::Custom { pages } => Self::Custom { pages },
        }
    }
}

impl From<WorkerPageMarkPages> for PageMarkPages {
    fn from(pages: WorkerPageMarkPages) -> Self {
        match pages {
            WorkerPageMarkPages::All => Self::All,
            WorkerPageMarkPages::AllButFirst => Self::AllButFirst,
            WorkerPageMarkPages::Custom { pages } => Self::Custom { pages },
        }
    }
}

impl From<PageMarks> for WorkerPageMarks {
    fn from(marks: PageMarks) -> Self {
        Self {
            page_number: marks.page_number.map(Into::into),
            bates: marks.bates.map(Into::into),
            header_footer: marks.header_footer,
            font_size: marks.font_size,
            color: marks.color,
            margin_pt: marks.margin_pt,
            pages: marks.pages.into(),
        }
    }
}

impl From<WorkerPageMarks> for PageMarks {
    fn from(marks: WorkerPageMarks) -> Self {
        Self {
            page_number: marks.page_number.map(Into::into),
            bates: marks.bates.map(Into::into),
            header_footer: marks.header_footer,
            font_size: marks.font_size,
            color: marks.color,
            margin_pt: marks.margin_pt,
            pages: marks.pages.into(),
        }
    }
}

/// A file a PDF is made of (B3-03): a read-only handle to it, and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSource {
    pub file: FileHandle,
    pub kind: CreateKind,
}

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
    /// document from index `at` on, in their order (B2-06), along with any text boxes prepared
    /// for them (B3-23, ADR 0019). Only what is on the pages comes along: not other annotations,
    /// links and form fields, and nothing active (see docs/architecture/merge.md).
    InsertPages {
        at: u32,
        source: Vec<u8>,
        text_boxes: Vec<InsertedTextBox>,
    },
    /// As [`Edit::AddHighlight`].
    AddHighlight {
        #[serde(default)]
        mark_kind: TextMarkKind,
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
    /// As [`Edit::AddRedaction`].
    AddRedaction { page: u32, rect: Rect },
    /// As [`Edit::AddRedactionGroup`].
    AddRedactionGroup { page: u32, rects: Vec<Rect> },
    /// As [`Edit::ApplyRedactions`].
    ApplyRedactions,
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
    /// As [`Edit::SetPictureAngle`].
    SetPictureAngle {
        page: u32,
        annotation: AnnotationId,
        degrees: f32,
    },
    /// Puts the picture `png` (as `StampImage` made it) over `rect` on page `page` as a stamp
    /// (B2-08).
    AddImageStamp { page: u32, rect: Rect, png: Vec<u8> },
    /// As [`Edit::SetTextBox`], with the model itself if there is one (B2-29).
    SetTextBox {
        page: u32,
        annotation: AnnotationId,
        model: Option<TextBoxModel>,
        style: TextBoxStyle,
    },
    /// As [`Edit::AddTextBox`], with the model itself (B2-28): no font is needed to make it.
    PlaceTextBox {
        page: u32,
        rect: Rect,
        model: TextBoxModel,
        style: TextBoxStyle,
    },
    /// As [`Edit::AddBookmark`] (B3-17).
    AddBookmark {
        parent: Option<BookmarkId>,
        index: u32,
        title: String,
        page: u32,
    },
    /// As [`Edit::RenameBookmark`] (B3-17).
    RenameBookmark { id: BookmarkId, title: String },
    /// As [`Edit::DeleteBookmark`] (B3-17).
    DeleteBookmark { id: BookmarkId },
    /// As [`Edit::MoveBookmark`] (B3-17).
    MoveBookmark {
        id: BookmarkId,
        parent: Option<BookmarkId>,
        index: u32,
    },
}

/// An edit names something its document does not have: a file whose pages it takes (B2-06), the
/// picture of a stamp (B2-08) or the words of a text box (B2-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownFile {
    /// The file whose pages the edit takes.
    Source,
    /// The picture of the stamp.
    Picture,
    /// The words of the text box, prepared.
    TextBox,
}

impl WorkerEdit {
    /// `edit` as the worker is asked to make it. An edit that takes the pages of a file names it;
    /// `source` gives the clean copy of that file (B2-06). A picture stamp names its picture;
    /// `picture` gives the PNG file of it (B2-08). A text box names its prepared words; `text_box`
    /// gives the model of them (B2-28). The worker is sent them with the edit.
    pub fn of(
        edit: &Edit,
        source: impl Fn(SourceId) -> Option<(Vec<u8>, Vec<InsertedTextBox>)>,
        picture: impl Fn(StampImageId) -> Option<Vec<u8>>,
        text_box: impl Fn(TextBoxModelId) -> Option<TextBoxModel>,
    ) -> Result<Self, UnknownFile> {
        Ok(match edit {
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
            Edit::InsertPages { at, source: id } => {
                let (source, text_boxes) = source(*id).ok_or(UnknownFile::Source)?;
                WorkerEdit::InsertPages {
                    at: *at,
                    source,
                    text_boxes,
                }
            }
            Edit::AddHighlight {
                mark_kind,
                marks,
                color,
            } => WorkerEdit::AddHighlight {
                mark_kind: *mark_kind,
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
            Edit::AddRedaction { page, rect } => WorkerEdit::AddRedaction {
                page: *page,
                rect: *rect,
            },
            Edit::AddRedactionGroup { page, rects } => WorkerEdit::AddRedactionGroup {
                page: *page,
                rects: rects.clone(),
            },
            Edit::ApplyRedactions => WorkerEdit::ApplyRedactions,
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
            Edit::AddImageStamp { page, rect, image } => WorkerEdit::AddImageStamp {
                page: *page,
                rect: *rect,
                png: picture(*image).ok_or(UnknownFile::Picture)?,
            },
            Edit::SetTextBox {
                page,
                annotation,
                model,
                style,
            } => WorkerEdit::SetTextBox {
                page: *page,
                annotation: *annotation,
                model: model
                    .map(|id| text_box(id).ok_or(UnknownFile::TextBox))
                    .transpose()?,
                style: *style,
            },
            Edit::AddTextBox {
                page,
                rect,
                model,
                style,
            } => WorkerEdit::PlaceTextBox {
                page: *page,
                rect: *rect,
                model: text_box(*model).ok_or(UnknownFile::TextBox)?,
                style: *style,
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
            Edit::SetPictureAngle {
                page,
                annotation,
                degrees,
            } => WorkerEdit::SetPictureAngle {
                page: *page,
                annotation: *annotation,
                degrees: *degrees,
            },
            Edit::AddBookmark {
                parent,
                index,
                title,
                page,
            } => WorkerEdit::AddBookmark {
                parent: *parent,
                index: *index,
                title: title.clone(),
                page: *page,
            },
            Edit::RenameBookmark { id, title } => WorkerEdit::RenameBookmark {
                id: *id,
                title: title.clone(),
            },
            Edit::DeleteBookmark { id } => WorkerEdit::DeleteBookmark { id: *id },
            Edit::MoveBookmark { id, parent, index } => WorkerEdit::MoveBookmark {
                id: *id,
                parent: *parent,
                index: *index,
            },
        })
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
    /// Only `region` of the page at `scale` (B3-09), answered with `Rendered`: a raster of the
    /// region's size. The page as a whole may be larger than the raster limits.
    RenderRegion {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        scale: f32,
        rotation: Rotation,
        region: RasterRegion,
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
    /// Verifies the signatures of the document as its file has them (B2-14, ADR 0014): offline,
    /// with Windows' own cryptography and what Windows has on this computer. Answered by
    /// `Signatures`.
    VerifySignatures {
        request: RequestId,
        doc: DocumentId,
    },
    /// What applying the redaction marks of `doc` would do besides removing what their areas
    /// reach (B2-19, ADR 0016). Answered by `RedactionSummary`.
    SummariseRedactions {
        request: RequestId,
        doc: DocumentId,
    },
    /// Finds personal data matching `query` in `doc` (Issue #302). Answered by `PersonalDataMatches`.
    FindPersonalData {
        request: RequestId,
        doc: DocumentId,
        query: FindPersonalDataQuery,
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
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, whose pictures
    /// are encoded again at `quality`, and shrunk to `max_side` pixels on their longer side
    /// (B3-01, docs/architecture/compress-images.md). The open document is not changed.
    /// Answered by `Compressed`, never appended. Refused for an encrypted document and for a
    /// signed one (its signatures would not hold).
    CompressedCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        quality: ImageQuality,
        max_side: Option<u32>,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, whose pictures
    /// carry nothing but themselves: what a camera or a program wrote into them (EXIF, XMP and
    /// the rest) is taken out without encoding them again (B3-04, docs/architecture/batch.md).
    /// The open document is not changed. Answered by `ExifCleaned`, never appended. Refused for
    /// an encrypted document, for a signed one (its signatures would not hold), and with
    /// `PictureNotCleaned` when a picture is stored in a way its metadata cannot be taken out of.
    ExifCleanCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, sanitized
    /// according to `options` and with `id` as its identifier (B3-15). The open document
    /// is not changed. Answered by `Sanitized`, never appended. Refused for an encrypted
    /// document and for a signed one.
    SanitizedCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        id: [u8; 16],
        options: SanitizeOptions,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, encrypted with
    /// AES-256 (B2-15, docs/architecture/encrypt-copy.md). The open document is not changed.
    /// Answered by `Saved`, never appended. Refused for a signed document (its signatures would
    /// go). Both passwords are wiped as any password is (MVP-16).
    EncryptedCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        /// Opens the copy; none: anyone can.
        open_password: Option<Password>,
        /// Lifts the restrictions. The main process makes one up when the user gave none.
        owner_password: Password,
        restrictions: Restrictions,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, with `watermark`
    /// on its pages (B3-04, docs/architecture/watermark.md). `picture` is the PNG of a picture
    /// watermark (a stamp picture, B2-08), and nothing for words. The open document is not
    /// changed. Answered by `Saved`, never appended. Refused for an encrypted document.
    WatermarkCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        watermark: WorkerWatermark,
        picture: Option<Vec<u8>>,
    },
    /// Writes to `file` (as for `Save`) a copy of the document, edits included, with `marks`
    /// (page numbers, Bates numbers, headers, and footers) on its pages (B3-16). The open
    /// document is not changed. Answered by `Saved`, never appended. Refused for an encrypted
    /// document.
    PageMarksCopy {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        marks: WorkerPageMarks,
    },
    /// Writes to `file` (as for `Save`) the file `doc` was opened from or last saved to, with a
    /// signature field added by an incremental update (B3-06, ADR 0020): its signature dictionary
    /// says `signature`, keeps room for the signature (`/Contents`, hexadecimal zeros) and has a
    /// `/ByteRange` that covers every other byte. The main process then writes the signature
    /// into that room. The open document is not changed. Refused for an encrypted document, for
    /// one certified to allow no change, and for a file that cannot be appended to. Answered by
    /// `SignaturePrepared`.
    PrepareSignature {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        signature: NewSignature,
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
    /// Writes to `file` (as for `Save`) the file `doc` was opened from or last saved to, with the
    /// app's document identifier put into its metadata (`identifier`), or taken out (`None`), by
    /// an incremental update (B3-05, ADR 0010): the file's bytes, then the update. The open
    /// document is not changed. Refused for a signed document, for an encrypted one without its
    /// permissions password, and for a file that cannot be appended to (MuPDF repaired it).
    /// The password is wiped once used. Answered by `Saved`, always appended.
    WriteIdentifier {
        request: RequestId,
        doc: DocumentId,
        file: FileHandle,
        identifier: Option<[u8; 16]>,
        password: Option<Password>,
    },
    /// Makes the picture `file` (read-only; a PNG or JPEG) into what a stamp is made of: its
    /// pixels, in a PNG file, shrunk to `MAX_STAMP_SIDE_PX` at most. Nothing else of the file
    /// survives (EXIF, GPS position, camera, text chunks, color profile). Answered by
    /// `StampImage` (B2-08).
    PrepareStampImage {
        request: RequestId,
        file: FileHandle,
    },
    /// Makes `text` in the font `font` (a file the main process read; not trusted) into the
    /// model of a text box: the font read and its licence checked, a glyph found for every
    /// character, and the font cut to those glyphs. Answered by `TextBoxPrepared`, or by an
    /// error that says why the font or the text cannot be used (B2-28).
    PrepareTextBox {
        request: RequestId,
        font: FontData,
        text: String,
    },
    /// Makes each text of `texts` in the font `font` into a text box model (B3-23, Issue #381).
    ///
    /// The font is parsed once for all texts in the batch. Answered by `TextBoxesPrepared` with
    /// one element per text (a model, or `Refused` if that text cannot be shaped or subset with this
    /// font, or `Deferred` if it was postponed to keep the response within size limits).
    PrepareTextBoxes {
        request: RequestId,
        font: FontData,
        texts: Vec<String>,
    },
    /// What text box `annotation` of page `page_index` says of itself: its words, font, style and
    /// whether they fit. Answered by `TextBox` (B2-29).
    GetTextBox {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        annotation: AnnotationId,
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
    ExportDocx {
        request: RequestId,
        doc: DocumentId,
        pages: Vec<u32>,
        file: FileHandle,
    },
    /// Makes a PDF of `sources` and writes it to `file` (write-only, as for `Save`): pictures
    /// are a page each and go together, any other source is the whole document, alone (B3-03,
    /// docs/architecture/create-pdf.md). No document is open for it. Answered by `Created`.
    CreatePdf {
        request: RequestId,
        sources: Vec<CreateSource>,
        file: FileHandle,
    },
    /// Loads the data of one language to recognise scanned pages in (B2-10, ADR 0015): the bytes
    /// of a Tesseract `.traineddata` file, which the main process read and checked
    /// (`crate::ocr::check_language_data`; the worker checks it too). It replaces the language
    /// loaded before, and drops the pages waiting. Answered by `OcrLoaded`.
    OcrLoad {
        request: RequestId,
        language: String,
        data: Vec<u8>,
    },
    /// Looks at page `page_index` of `doc` and, if it is a scan (it has no text, and pictures
    /// cover most of it), queues it to be recognised in the background, in at most `max_millis`.
    /// The page is drawn here and recognised on a thread of its own, so the worker goes on
    /// answering other requests. Answered at once by `OcrChecked`. No language is needed to tell
    /// that a page is no scan: only a scan is answered with `NoLanguage` if none was loaded.
    OcrPage {
        request: RequestId,
        doc: DocumentId,
        page_index: u32,
        max_millis: u32,
    },
    /// Takes the pages that were recognised since the last poll. Answered by `OcrPolled`.
    OcrPoll {
        request: RequestId,
    },
    /// Drops the pages waiting and stops the one being read. Answered by `OcrStopped`.
    OcrStop {
        request: RequestId,
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
    /// Checks the bytes of a font file the user wants to import (B2-32, ADR 0018): TrueType
    /// outlines (`glyf`/`loca`), charmap, licence (OS/2 `fsType`) and names. Answered by
    /// `FontInspected`, or by `WorkerError` with `Font` if the font cannot be used.
    InspectFont {
        request: RequestId,
        bytes: Vec<u8>,
    },
    /// Lists the attachments of the document (B3-20, ADR 0025). Answered by `Attachments`.
    GetAttachments {
        request: RequestId,
        doc: DocumentId,
    },
    /// Decodes attachment `attachment` and writes its bytes to `file` (write-only handle,
    /// max 512 MiB, B3-20, ADR 0025). Answered by `AttachmentWritten`.
    WriteAttachment {
        request: RequestId,
        doc: DocumentId,
        attachment: AttachmentId,
        file: FileHandle,
    },
    /// Best effort: the worker drops the target request if it has not finished yet.
    Cancel {
        target: RequestId,
    },
    Close {
        doc: DocumentId,
    },
    /// Collects an annotation summary across all pages, sorted by page and displayed direction (B3-18).
    AnnotationSummary {
        request: RequestId,
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
    Signatures {
        request: RequestId,
        report: SignatureReport,
    },
    /// `SummariseRedactions` is answered.
    RedactionSummary {
        request: RequestId,
        summary: RedactionSummary,
    },
    /// `FindPersonalData` is answered (Issue #302).
    PersonalDataMatches {
        request: RequestId,
        matches: PersonalDataMatches,
    },
    PageText {
        request: RequestId,
        page_index: u32,
        text: PageText,
    },
    /// What text box `annotation` of a page says of itself (`GetTextBox`).
    TextBox {
        request: RequestId,
        info: TextBoxInfo,
    },
    /// The model of `PrepareTextBox`.
    TextBoxPrepared {
        request: RequestId,
        model: TextBoxModel,
    },
    /// The models of `PrepareTextBoxes` (B3-23, Issue #381).
    ///
    /// Exactly one entry per requested text in the same order. `Prepared` if the text was shaped
    /// and subset; `Refused` if the font cannot be used or cannot shape the text; `Deferred` if
    /// the text was deferred to a subsequent batch to keep the response within size limits.
    TextBoxesPrepared {
        request: RequestId,
        models: Vec<PreparedTextBoxModel>,
    },
    /// The faces of `InspectFont`: their family, style and face index.
    FontInspected {
        request: RequestId,
        faces: Vec<InspectedFont>,
    },
    /// The picture of `PrepareStampImage`: a PNG file of `width` x `height` pixels.
    StampImage {
        request: RequestId,
        png: Vec<u8>,
        width: u32,
        height: u32,
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
    /// The copy of the file of `PrepareSource` (B2-06), with any text boxes it has (B3-23, ADR 0019).
    Source {
        request: RequestId,
        bytes: Vec<u8>,
        pages: u32,
        security: SecurityReport,
        text_boxes: Vec<SourceTextBox>,
    },
    /// The document was written: `bytes` long, appended to the original (`incremental`, for a
    /// signed document) or rewritten. `redaction`: what reading the written bytes again found,
    /// when redactions were applied since the last save (B2-20); `None` otherwise, and for the
    /// copies.
    Saved {
        request: RequestId,
        bytes: u64,
        incremental: bool,
        redaction: Option<RedactionCheck>,
    },
    /// `PrepareSignature` is done: the copy is `bytes` long, and its `/Contents`, `<` and `>`
    /// included, is the bytes from `contents_start` up to `contents_end`: hexadecimal zeros for
    /// the main process to fill. The copy's `/ByteRange` covers the rest of it.
    SignaturePrepared {
        request: RequestId,
        bytes: u64,
        contents_start: u64,
        contents_end: u64,
    },
    /// `CompressedCopy` is done: the copy is `bytes` long; with its pictures as they were it
    /// would have been `before` long. `images` pictures were looked at, `changed` of them were
    /// encoded again; `partial`: the time was up first.
    Compressed {
        request: RequestId,
        bytes: u64,
        before: u64,
        images: u32,
        changed: u32,
        partial: bool,
    },
    /// `ExifCleanCopy` is done: the copy is `bytes` long; `pictures` pictures (and other JPEG
    /// streams) were looked at, and `cleaned` of them had something taken out.
    ExifCleaned {
        request: RequestId,
        bytes: u64,
        pictures: u32,
        cleaned: u32,
    },
    /// `SanitizedCopy` is done: the copy is `bytes` long and `report` details what was removed.
    Sanitized {
        request: RequestId,
        bytes: u64,
        report: SanitizeReport,
    },
    /// `CreatePdf` is done: the PDF is `bytes` long and has `pages` pages.
    Created {
        request: RequestId,
        bytes: u64,
        pages: u32,
    },
    /// `OcrLoad` is done: pages are recognised in that language now.
    OcrLoaded {
        request: RequestId,
    },
    OcrChecked {
        request: RequestId,
        state: OcrPageState,
    },
    OcrPolled {
        request: RequestId,
        finished: Vec<OcrFinished>,
        /// Pages waiting or being read.
        waiting: u32,
    },
    OcrStopped {
        request: RequestId,
    },
    /// The annotation summary of `AnnotationSummary`.
    AnnotationSummary {
        request: RequestId,
        summary: AnnotationSummary,
    },
    /// Attachments of the document (`GetAttachments`, B3-20, ADR 0025).
    Attachments {
        request: RequestId,
        attachments: Vec<WorkerAttachment>,
    },
    /// Attachment was written to `file` (`WriteAttachment`, B3-20, ADR 0025).
    AttachmentWritten {
        request: RequestId,
        bytes: u64,
    },
    Error {
        request: Option<RequestId>,
        error: WorkerError,
    },
}

/// What `OcrPage` found out about a page (B2-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcrPageState {
    /// It has text of its own, or is not mostly a picture: nothing to recognise.
    NotScan,
    /// Its text was recognised before.
    Recognised,
    /// It is waiting or being read; `OcrPoll` will say when it is done.
    Queued,
    /// Reading it failed or took too long before, and it is not tried again.
    Failed,
    /// The queue is full: ask again after a poll.
    Full,
    /// The page is a scan, and no language was loaded (`OcrLoad`).
    NoLanguage,
}

/// A page that was recognised or given up on, as `OcrPoll` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrFinished {
    pub doc: DocumentId,
    /// The page's index now: pages may have been moved or deleted since it was queued.
    pub page_index: u32,
    pub outcome: OcrOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcrOutcome {
    /// Its text is kept in the worker, `chars` characters of it (none: the picture had no text).
    Recognised {
        chars: u32,
    },
    /// It took longer than the time allowed.
    TimedOut,
    Failed,
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
    /// Signed: its form says signatures exist, or a signature field has a value. Writing the
    /// sensitive mark into the file is refused (B3-05, ADR 0010).
    pub signed: bool,
    /// The app's document identifier in the file's metadata (ADR 0010; not a `DocumentId`): the
    /// file is marked as sensitive (B3-05, ADR 0004).
    pub identifier: Option<[u8; 16]>,
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
    /// Only with `Encrypted` (a password is needed and none was given): where a password the
    /// user remembered for this file is looked up (ADR 0017). `None` when the file does not say
    /// it, and for every other error.
    pub password_key: Option<PasswordKey>,
}

/// Where a remembered password of an encrypted document is looked up (ADR 0017): a SHA-256 of
/// what the file itself says about its user password (the revision and the verifier of its
/// `/Encrypt` dictionary), so that nothing has to be written into the PDF. It is a name to look
/// something up by, not a secret, and the worker is not trusted to give the right one: the main
/// process only ever sends a remembered password to the worker that reported the key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PasswordKey(pub [u8; 32]);

impl PasswordKey {
    /// The 64 lower-case hexadecimal digits of the key: the end of the name of its credential.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// The first four bytes only: enough to tell two keys apart in a log, not to name the file.
impl std::fmt::Debug for PasswordKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PasswordKey({:02x}{:02x}{:02x}{:02x}..)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
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
    /// The document is signed, and the request would write into its file (B3-05, ADR 0010).
    Signed,
    /// A text box cannot be set in the font (B2-28).
    Font(FontRefusal),
    /// A text box cannot be set in the text: it needs a script the worker does not set (B2-28).
    UnsupportedText,
    /// A picture is stored in a way its metadata cannot be taken out of (B3-04).
    PictureNotCleaned,
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
            WorkerErrorCode::Signed => ErrorCode::Signed,
            WorkerErrorCode::Font(refusal) => match refusal {
                FontRefusal::NotAFont => ErrorCode::FontInvalid,
                FontRefusal::NotTrueType => ErrorCode::FontNotTrueType,
                FontRefusal::Variable => ErrorCode::FontVariable,
                FontRefusal::Colour => ErrorCode::FontColour,
                FontRefusal::NotEmbeddable => ErrorCode::FontNotEmbeddable,
                FontRefusal::MissingGlyphs => ErrorCode::FontMissingGlyphs,
            },
            WorkerErrorCode::UnsupportedText => ErrorCode::TextNotSupported,
            WorkerErrorCode::PictureNotCleaned => ErrorCode::PictureNotCleaned,
            WorkerErrorCode::Internal => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockedAction, BookmarkId, LinkTarget, OutlineItem};

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

    fn sample_model() -> TextBoxModel {
        TextBoxModel {
            text: "Hi".to_owned(),
            font: "Arial Bold".to_owned(),
            face: 2,
            view_only: true,
            base: "ABCDEF+Arial-Bold".to_owned(),
            ascent: 905,
            descent: -212,
            cap_height: 716,
            step: 1_150,
            bbox: [-665, -325, 2_000, 1_006],
            glyphs: vec![
                ModelGlyph {
                    ch: 'H',
                    cid: 1,
                    advance: 722,
                },
                ModelGlyph {
                    ch: 'i',
                    cid: 2,
                    advance: 278,
                },
            ],
            subset: vec![0, 1, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9],
        }
    }

    #[test]
    fn a_text_box_edit_carries_its_model_and_makes_the_same_edit_again() {
        use crate::types::{Rgb, TextAlign};
        let edit = Edit::AddTextBox {
            page: 1,
            rect: Rect {
                x0: 72.0,
                y0: 72.0,
                x1: 372.0,
                y1: 132.0,
            },
            model: TextBoxModelId(4),
            style: TextBoxStyle {
                size: 14.0,
                color: Rgb { r: 1, g: 2, b: 3 },
                align: TextAlign::Right,
            },
        };
        let model = |id: TextBoxModelId| (id == TextBoxModelId(4)).then(sample_model);
        let made = WorkerEdit::of(&edit, |_| None, |_| None, model).unwrap();
        let WorkerEdit::PlaceTextBox {
            page,
            model: sent,
            style,
            ..
        } = &made
        else {
            panic!("{made:?}");
        };
        assert_eq!(
            (*page, style.size, style.align),
            (1, 14.0, TextAlign::Right)
        );
        assert_eq!(*sent, sample_model());
        let bytes = postcard::to_allocvec(&made).unwrap();
        assert_eq!(postcard::from_bytes::<WorkerEdit>(&bytes).unwrap(), made);
        // Words that the document does not have are no edit.
        assert_eq!(
            WorkerEdit::of(&edit, |_| None, |_| None, |_| None),
            Err(UnknownFile::TextBox)
        );
    }

    #[test]
    fn setting_a_text_box_again_looks_for_new_words_only_when_it_has_some() {
        use crate::types::{Rgb, TextAlign};
        let style = TextBoxStyle {
            size: 12.0,
            color: Rgb { r: 1, g: 2, b: 3 },
            align: TextAlign::Left,
        };
        let edit = |model: Option<u32>| Edit::SetTextBox {
            page: 0,
            annotation: AnnotationId(7),
            model: model.map(TextBoxModelId),
            style,
        };
        let models = |id: TextBoxModelId| (id == TextBoxModelId(4)).then(sample_model);
        // Without new words, none are looked for: the box keeps the words it has.
        let kept = WorkerEdit::of(&edit(None), |_| None, |_| None, |_| None).unwrap();
        assert!(matches!(kept, WorkerEdit::SetTextBox { model: None, .. }));
        let new = WorkerEdit::of(&edit(Some(4)), |_| None, |_| None, models).unwrap();
        let WorkerEdit::SetTextBox {
            model: Some(ref words),
            ..
        } = new
        else {
            panic!("{new:?}");
        };
        assert_eq!(*words, sample_model());
        assert_eq!(
            WorkerEdit::of(&edit(Some(9)), |_| None, |_| None, models),
            Err(UnknownFile::TextBox)
        );
        for edit in [&kept, &new] {
            let bytes = postcard::to_allocvec(edit).unwrap();
            assert_eq!(&postcard::from_bytes::<WorkerEdit>(&bytes).unwrap(), edit);
        }
        let request = WorkerRequest::GetTextBox {
            request: RequestId(2),
            doc: DocumentId(3),
            page_index: 1,
            annotation: AnnotationId(7),
        };
        let bytes = postcard::to_allocvec(&request).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerRequest>(&bytes).unwrap(),
            request
        );
    }

    #[test]
    fn insert_pages_edit_carries_source_and_text_boxes() {
        let edit = Edit::InsertPages {
            at: 2,
            source: SourceId(5),
        };
        let tb = InsertedTextBox {
            page: 0,
            rect: Rect {
                x0: 10.0,
                y0: 20.0,
                x1: 100.0,
                y1: 50.0,
            },
            model: sample_model(),
            style: TextBoxStyle {
                size: 14.0,
                color: Rgb {
                    r: 10,
                    g: 20,
                    b: 30,
                },
                align: crate::types::TextAlign::Center,
            },
        };
        let source_fn =
            |id: SourceId| (id == SourceId(5)).then(|| (vec![1, 2, 3, 4], vec![tb.clone()]));
        let made = WorkerEdit::of(&edit, source_fn, |_| None, |_| None).unwrap();
        assert_eq!(
            made,
            WorkerEdit::InsertPages {
                at: 2,
                source: vec![1, 2, 3, 4],
                text_boxes: vec![tb],
            }
        );
        let bytes = postcard::to_allocvec(&made).unwrap();
        assert_eq!(postcard::from_bytes::<WorkerEdit>(&bytes).unwrap(), made);

        assert_eq!(
            WorkerEdit::of(&edit, |_| None, |_| None, |_| None),
            Err(UnknownFile::Source)
        );
    }

    #[test]
    fn redaction_edits_and_their_summary_cross_the_boundary() {
        for edit in [
            WorkerEdit::AddRedaction {
                page: 2,
                rect: Rect {
                    x0: 1.0,
                    y0: 2.0,
                    x1: 30.0,
                    y1: 40.0,
                },
            },
            WorkerEdit::AddRedactionGroup {
                page: 2,
                rects: vec![
                    Rect {
                        x0: 1.0,
                        y0: 2.0,
                        x1: 30.0,
                        y1: 40.0,
                    },
                    Rect {
                        x0: 1.0,
                        y0: 42.0,
                        x1: 30.0,
                        y1: 80.0,
                    },
                ],
            },
            WorkerEdit::ApplyRedactions,
        ] {
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(postcard::from_bytes::<WorkerEdit>(&bytes).unwrap(), edit);
        }
        assert_eq!(
            WorkerEdit::of(&Edit::ApplyRedactions, |_| None, |_| None, |_| None).unwrap(),
            WorkerEdit::ApplyRedactions
        );
    }

    #[test]
    fn bookmark_edits_cross_the_boundary() {
        let edits = [
            (
                Edit::AddBookmark {
                    parent: None,
                    index: 0,
                    title: "Chapter 1".to_owned(),
                    page: 3,
                },
                WorkerEdit::AddBookmark {
                    parent: None,
                    index: 0,
                    title: "Chapter 1".to_owned(),
                    page: 3,
                },
            ),
            (
                Edit::AddBookmark {
                    parent: Some(BookmarkId(10)),
                    index: 2,
                    title: "Section 1.1".to_owned(),
                    page: 4,
                },
                WorkerEdit::AddBookmark {
                    parent: Some(BookmarkId(10)),
                    index: 2,
                    title: "Section 1.1".to_owned(),
                    page: 4,
                },
            ),
            (
                Edit::RenameBookmark {
                    id: BookmarkId(12),
                    title: "Renamed Chapter".to_owned(),
                },
                WorkerEdit::RenameBookmark {
                    id: BookmarkId(12),
                    title: "Renamed Chapter".to_owned(),
                },
            ),
            (
                Edit::DeleteBookmark { id: BookmarkId(12) },
                WorkerEdit::DeleteBookmark { id: BookmarkId(12) },
            ),
            (
                Edit::MoveBookmark {
                    id: BookmarkId(12),
                    parent: Some(BookmarkId(5)),
                    index: 1,
                },
                WorkerEdit::MoveBookmark {
                    id: BookmarkId(12),
                    parent: Some(BookmarkId(5)),
                    index: 1,
                },
            ),
        ];
        for (frontend_edit, worker_edit) in edits {
            let made = WorkerEdit::of(&frontend_edit, |_| None, |_| None, |_| None).unwrap();
            assert_eq!(made, worker_edit);
            let bytes = postcard::to_allocvec(&made).unwrap();
            assert_eq!(postcard::from_bytes::<WorkerEdit>(&bytes).unwrap(), made);
        }
        let response = WorkerResponse::RedactionSummary {
            request: RequestId(9),
            summary: RedactionSummary {
                marks: 3,
                pages: 2,
                annotations: 1,
                fields: 0,
            },
        };
        let bytes = postcard::to_allocvec(&response).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerResponse>(&bytes).unwrap(),
            response
        );
    }

    #[test]
    fn writing_the_identifier_carries_it_and_hides_the_password() {
        let request = WorkerRequest::WriteIdentifier {
            request: RequestId(4),
            doc: DocumentId(5),
            file: FileHandle(0x40),
            identifier: Some([0x3c; 16]),
            password: Some(Password::new("owner secret".to_owned())),
        };
        assert!(!format!("{request:?}").contains("owner secret"));
        let bytes = postcard::to_allocvec(&request).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerRequest>(&bytes).unwrap(),
            request
        );
        let opened = OpenedDocument {
            pages: vec![PageSize {
                width_pt: 612.0,
                height_pt: 792.0,
            }],
            has_outline: false,
            has_form: false,
            security: SecurityReport::default(),
            permissions: DocumentPermissions::ALL,
            encrypted: false,
            signed: true,
            identifier: Some([0xc3; 16]),
        };
        let bytes = postcard::to_allocvec(&opened).unwrap();
        assert_eq!(
            postcard::from_bytes::<OpenedDocument>(&bytes).unwrap(),
            opened
        );
        let bytes = postcard::to_allocvec(&WorkerErrorCode::Signed).unwrap();
        let code = postcard::from_bytes::<WorkerErrorCode>(&bytes).unwrap();
        assert_eq!(ErrorCode::from(code), ErrorCode::Signed);
    }

    #[test]
    fn a_watermark_crosses_to_the_worker_as_it_was_given() {
        use crate::types::{Watermark, WatermarkContent, WatermarkPages, WatermarkPlace};
        for content in [
            WatermarkContent::Text {
                text: "機密 CONFIDENTIAL".to_owned(),
                size: 48.0,
                color: Rgb { r: 9, g: 8, b: 7 },
            },
            WatermarkContent::Picture { width_pt: 120.5 },
        ] {
            let watermark = Watermark {
                content,
                opacity: 30,
                diagonal: true,
                place: WatermarkPlace::Bottom,
                pages: WatermarkPages::AllButFirst,
            };
            // Postcard cannot carry the page's form (tagged for JSON); the worker's is the same.
            assert!(
                postcard::to_allocvec(&watermark).is_err() || {
                    let bytes = postcard::to_allocvec(&watermark).unwrap();
                    postcard::from_bytes::<Watermark>(&bytes).is_err()
                }
            );
            let request = WorkerRequest::WatermarkCopy {
                request: RequestId(5),
                doc: DocumentId(6),
                file: FileHandle(7),
                watermark: watermark.clone().into(),
                picture: Some(vec![1, 2, 3]),
            };
            let bytes = postcard::to_allocvec(&request).unwrap();
            let back = postcard::from_bytes::<WorkerRequest>(&bytes).unwrap();
            assert_eq!(back, request);
            let WorkerRequest::WatermarkCopy {
                watermark: crossed, ..
            } = back
            else {
                panic!("{back:?}");
            };
            assert_eq!(Watermark::from(crossed), watermark);
        }
    }

    #[test]
    fn page_marks_cross_to_the_worker_as_they_were_given() {
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::PageOfTotal,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            bates: Some(BatesMark {
                prefix: "BATES-".to_owned(),
                digits: 6,
                start: 100,
                position: PageMarkPosition::BottomRight,
            }),
            header_footer: Some(HeaderFooterMarks {
                header_left: Some("CONFIDENTIAL".to_owned()),
                header_center: None,
                header_right: Some("DRAFT".to_owned()),
                footer_left: None,
                footer_center: None,
                footer_right: None,
            }),
            font_size: 10.0,
            color: Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
            margin_pt: 36.0,
            pages: PageMarkPages::Custom {
                pages: vec![0, 2, 4],
            },
        };
        // Postcard cannot carry tagged enums of PageMarks directly.
        assert!(
            postcard::to_allocvec(&marks).is_err() || {
                let bytes = postcard::to_allocvec(&marks).unwrap();
                postcard::from_bytes::<PageMarks>(&bytes).is_err()
            }
        );

        let request = WorkerRequest::PageMarksCopy {
            request: RequestId(5),
            doc: DocumentId(6),
            file: FileHandle(7),
            marks: marks.clone().into(),
        };
        let bytes = postcard::to_allocvec(&request).unwrap();
        let back = postcard::from_bytes::<WorkerRequest>(&bytes).unwrap();
        assert_eq!(back, request);
        let WorkerRequest::PageMarksCopy { marks: crossed, .. } = back else {
            panic!("{back:?}");
        };
        assert_eq!(PageMarks::from(crossed), marks);
    }

    #[test]
    fn the_font_sent_to_prepare_words_does_not_print_its_bytes() {
        let request = WorkerRequest::PrepareTextBox {
            request: RequestId(3),
            font: FontData {
                bytes: vec![0, 1, 0, 0, 9, 9, 9, 9],
                face: 2,
                name: "Arial Bold".to_owned(),
            },
            text: "Hello".to_owned(),
        };
        let shown = format!("{request:?}");
        assert!(
            shown.contains("bytes: 8") && !shown.contains("[0, 1, 0, 0"),
            "{shown}"
        );
        let bytes = postcard::to_allocvec(&request).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerRequest>(&bytes).unwrap(),
            request
        );
        let response = WorkerResponse::TextBoxPrepared {
            request: RequestId(3),
            model: sample_model(),
        };
        let bytes = postcard::to_allocvec(&response).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerResponse>(&bytes).unwrap(),
            response
        );
    }

    #[test]
    fn a_models_subset_is_hex_text_in_the_journal_and_plain_bytes_between_the_processes() {
        let model = sample_model();
        let json = serde_json::to_string(&model).unwrap();
        assert!(
            json.contains(r#""subset":"000100000909090909090909""#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<TextBoxModel>(&json).unwrap(), model);
        // Not lowercase hex, an odd number of digits, or a field that is not the model's.
        for bad in [
            json.replace("00010000", "0001000G"),
            json.replace("00010000", "0001000A"),
            json.replace(r#""subset":""#, r#""subset":"0"#),
            json.replace(r#""face":2"#, r#""face":2,"extra":1"#),
        ] {
            assert!(serde_json::from_str::<TextBoxModel>(&bad).is_err(), "{bad}");
        }
        // Bytes cost their own size between the processes, and not twice that.
        let bytes = postcard::to_allocvec(&model).unwrap();
        assert!(
            bytes.len() < json.len(),
            "{} against {}",
            bytes.len(),
            json.len()
        );
    }

    #[test]
    fn a_font_refusal_reaches_the_frontend_as_a_code_it_can_explain() {
        for (refusal, code) in [
            (FontRefusal::NotAFont, ErrorCode::FontInvalid),
            (FontRefusal::NotTrueType, ErrorCode::FontNotTrueType),
            (FontRefusal::Variable, ErrorCode::FontVariable),
            (FontRefusal::Colour, ErrorCode::FontColour),
            (FontRefusal::NotEmbeddable, ErrorCode::FontNotEmbeddable),
            (FontRefusal::MissingGlyphs, ErrorCode::FontMissingGlyphs),
        ] {
            assert_eq!(ErrorCode::from(WorkerErrorCode::Font(refusal)), code);
            let bytes = postcard::to_allocvec(&WorkerErrorCode::Font(refusal)).unwrap();
            assert_eq!(
                postcard::from_bytes::<WorkerErrorCode>(&bytes).unwrap(),
                WorkerErrorCode::Font(refusal)
            );
        }
        assert_eq!(
            ErrorCode::from(WorkerErrorCode::UnsupportedText),
            ErrorCode::TextNotSupported
        );
    }

    #[test]
    fn a_picture_that_cannot_be_cleaned_reaches_the_frontend_as_its_own_code() {
        assert_eq!(
            ErrorCode::from(WorkerErrorCode::PictureNotCleaned),
            ErrorCode::PictureNotCleaned
        );
        let request = WorkerRequest::ExifCleanCopy {
            request: RequestId(1),
            doc: DocumentId(2),
            file: FileHandle(3),
        };
        let bytes = postcard::to_stdvec(&request).unwrap();
        assert_eq!(
            postcard::from_bytes::<WorkerRequest>(&bytes).unwrap(),
            request
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
    fn a_password_key_is_64_hex_digits_and_a_log_shows_four_bytes_of_it() {
        let key = PasswordKey(std::array::from_fn(|index| index as u8));
        assert_eq!(
            key.to_hex(),
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
        assert_eq!(format!("{key:?}"), "PasswordKey(00010203..)");
    }

    #[test]
    fn a_request_for_a_password_crosses_the_worker_boundary_with_its_key() {
        for password_key in [Some(PasswordKey([0xa5; 32])), None] {
            let response = WorkerResponse::Error {
                request: Some(RequestId(7)),
                error: WorkerError {
                    code: WorkerErrorCode::Encrypted,
                    detail: "the document needs a password".to_owned(),
                    password_key,
                },
            };
            let bytes = postcard::to_stdvec(&response).unwrap();
            assert_eq!(
                postcard::from_bytes::<WorkerResponse>(&bytes).unwrap(),
                response
            );
        }
    }

    #[test]
    fn outlines_and_links_cross_the_worker_boundary() {
        // postcard cannot decode internally tagged enums; LinkTarget must still get through.
        let response = WorkerResponse::Outline {
            request: RequestId(1),
            outline: OutlineResult {
                items: targets()
                    .into_iter()
                    .enumerate()
                    .map(|(index, target)| OutlineItem {
                        id: BookmarkId(index as u32 + 1),
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
