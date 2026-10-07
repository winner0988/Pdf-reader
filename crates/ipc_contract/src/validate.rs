//! Bounds checks for untrusted values.
//!
//! The main process validates every [`WorkerResponse`] before using it, and every command
//! argument coming from the frontend. Page-index checks need the document's page count and are
//! done by the caller with [`check_page_index`].

use std::collections::HashSet;

use thiserror::Error;

use crate::limits::*;
use crate::text::{classify_uri, is_clean_copy_text, is_clean_display_text, is_note_text};
use crate::types::{
    DocumentInfo, Edit, EditArgs, ExportArgs, ExportFormat, FindingKind, FormField, IpcError,
    LinkTarget, OpenEvent, OutlineItem, OutlineResult, PageAnnotation, PageLink, PageSize,
    PageText, Password, Point, Quad, RecentFile, Rect, RenderPageArgs, Rotation, SearchArgs,
    SearchHit, SecurityReport, TextLine, UndoArgs, UnlockArgs,
};
use crate::worker::{OcrOutcome, OpenedDocument, Raster, WorkerError, WorkerResponse};

/// Maximum length of the worker version string in `Hello`.
const MAX_VERSION_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("{what} is not a finite number")]
    NotFinite { what: &'static str },
    #[error("{what} is out of range")]
    OutOfRange { what: &'static str },
    #[error("{what} has {len} items; the limit is {max}")]
    TooMany {
        what: &'static str,
        len: usize,
        max: usize,
    },
    #[error("{what} is {len} bytes; the limit is {max}")]
    TooLong {
        what: &'static str,
        len: usize,
        max: usize,
    },
    #[error("{what}: {reason}")]
    Invalid {
        what: &'static str,
        reason: &'static str,
    },
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

/// Checks that `page_index` addresses one of `page_count` pages.
pub fn check_page_index(page_index: u32, page_count: u32) -> Result<(), ValidationError> {
    if page_index >= page_count {
        return Err(ValidationError::OutOfRange { what: "page index" });
    }
    Ok(())
}

fn check_count(what: &'static str, len: usize, max: u32) -> Result<(), ValidationError> {
    if len > max as usize {
        return Err(ValidationError::TooMany {
            what,
            len,
            max: max as usize,
        });
    }
    Ok(())
}

fn check_text(what: &'static str, text: &str, max: u32) -> Result<(), ValidationError> {
    if text.len() > max as usize {
        return Err(ValidationError::TooLong {
            what,
            len: text.len(),
            max: max as usize,
        });
    }
    Ok(())
}

/// Text from the PDF must arrive already cleaned (no control, bidi or zero-width characters).
fn check_clean(what: &'static str, text: &str) -> Result<(), ValidationError> {
    if !is_clean_display_text(text) {
        return Err(ValidationError::Invalid {
            what,
            reason: "contains control or invisible formatting characters",
        });
    }
    Ok(())
}

fn check_coordinate(what: &'static str, value: f32) -> Result<(), ValidationError> {
    if !value.is_finite() {
        return Err(ValidationError::NotFinite { what });
    }
    if value.abs() > MAX_PAGE_SIDE_PT {
        return Err(ValidationError::OutOfRange { what });
    }
    Ok(())
}

impl Validate for PageSize {
    fn validate(&self) -> Result<(), ValidationError> {
        for (what, side) in [
            ("page width", self.width_pt),
            ("page height", self.height_pt),
        ] {
            if !side.is_finite() {
                return Err(ValidationError::NotFinite { what });
            }
            if side <= 0.0 || side > MAX_PAGE_SIDE_PT {
                return Err(ValidationError::OutOfRange { what });
            }
        }
        Ok(())
    }
}

impl Validate for Point {
    fn validate(&self) -> Result<(), ValidationError> {
        check_coordinate("x coordinate", self.x)?;
        check_coordinate("y coordinate", self.y)
    }
}

impl Validate for Rect {
    fn validate(&self) -> Result<(), ValidationError> {
        for value in [self.x0, self.y0, self.x1, self.y1] {
            check_coordinate("rectangle coordinate", value)?;
        }
        Ok(())
    }
}

impl Validate for Quad {
    fn validate(&self) -> Result<(), ValidationError> {
        for point in [self.ul, self.ur, self.ll, self.lr] {
            point.validate()?;
        }
        Ok(())
    }
}

impl Validate for SecurityReport {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count(
            "security findings",
            self.findings.len(),
            FindingKind::ALL.len() as u32,
        )?;
        let mut seen = HashSet::new();
        for finding in &self.findings {
            if !seen.insert(finding.kind) {
                return Err(ValidationError::Invalid {
                    what: "security findings",
                    reason: "duplicate kind",
                });
            }
        }
        Ok(())
    }
}

impl Validate for LinkTarget {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            LinkTarget::Page { x, y, .. } => {
                for value in [x, y].into_iter().flatten() {
                    check_coordinate("link destination", *value)?;
                }
                Ok(())
            }
            LinkTarget::Uri { uri } => {
                if uri.is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "link URI",
                        reason: "empty",
                    });
                }
                check_text("link URI", uri, MAX_URI_BYTES)?;
                // Only http, https and mailto may be offered at all.
                if !matches!(classify_uri(uri), LinkTarget::Uri { .. }) {
                    return Err(ValidationError::Invalid {
                        what: "link URI",
                        reason: "not an openable http, https or mailto URI",
                    });
                }
                Ok(())
            }
            LinkTarget::Blocked { target, .. } => match target {
                Some(target) => {
                    check_text("blocked action target", target, MAX_TEXT_BYTES)?;
                    check_clean("blocked action target", target)
                }
                None => Ok(()),
            },
        }
    }
}

impl Validate for PageLink {
    fn validate(&self) -> Result<(), ValidationError> {
        self.rect.validate()?;
        self.target.validate()
    }
}

impl Validate for OutlineItem {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text("outline title", &self.title, MAX_TEXT_BYTES)?;
        check_clean("outline title", &self.title)?;
        if self.depth > MAX_OUTLINE_DEPTH {
            return Err(ValidationError::OutOfRange {
                what: "outline depth",
            });
        }
        match &self.target {
            Some(target) => target.validate(),
            None => Ok(()),
        }
    }
}

impl Validate for OutlineResult {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count("outline items", self.items.len(), MAX_OUTLINE_ITEMS)?;
        // Pre-order: starts at depth 0 and never descends more than one level at a time.
        let mut previous_depth: Option<u16> = None;
        for item in &self.items {
            item.validate()?;
            let max_depth = previous_depth.map_or(0, |depth| depth + 1);
            if item.depth > max_depth {
                return Err(ValidationError::Invalid {
                    what: "outline",
                    reason: "depth skips a level",
                });
            }
            previous_depth = Some(item.depth);
        }
        Ok(())
    }
}

impl Validate for PageText {
    fn validate(&self) -> Result<(), ValidationError> {
        let mut chars = 0usize;
        for line in &self.lines {
            line.validate()?;
            chars += line.edges.len() - 1;
            check_count("page text characters", chars, MAX_PAGE_TEXT_CHARS)?;
        }
        Ok(())
    }
}

impl Validate for TextLine {
    fn validate(&self) -> Result<(), ValidationError> {
        self.quad.validate()?;
        if self.text.is_empty() {
            return Err(ValidationError::Invalid {
                what: "text line",
                reason: "empty",
            });
        }
        if !is_clean_copy_text(&self.text) {
            return Err(ValidationError::Invalid {
                what: "text line",
                reason: "contains control or invisible formatting characters",
            });
        }
        if self.edges.len() != self.text.chars().count() + 1 {
            return Err(ValidationError::Invalid {
                what: "text line edges",
                reason: "not one more than the characters",
            });
        }
        let mut previous = 0.0;
        for &edge in &self.edges {
            check_coordinate("text line edge", edge)?;
            if edge < previous {
                return Err(ValidationError::Invalid {
                    what: "text line edges",
                    reason: "negative or decreasing",
                });
            }
            previous = edge;
        }
        Ok(())
    }
}

impl Validate for SearchHit {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.quads.is_empty() {
            return Err(ValidationError::Invalid {
                what: "search hit",
                reason: "no quads",
            });
        }
        check_count("search hit quads", self.quads.len(), MAX_QUADS_PER_HIT)?;
        self.quads.iter().try_for_each(Quad::validate)
    }
}

impl Validate for OpenedDocument {
    fn validate(&self) -> Result<(), ValidationError> {
        check_pages(&self.pages)?;
        self.security.validate()
    }
}

/// A document's pages: at least one, at most `MAX_PAGE_COUNT`, each a sane size.
fn check_pages(pages: &[PageSize]) -> Result<(), ValidationError> {
    if pages.is_empty() {
        return Err(ValidationError::Invalid {
            what: "document",
            reason: "has no pages",
        });
    }
    check_count("pages", pages.len(), MAX_PAGE_COUNT)?;
    pages.iter().try_for_each(PageSize::validate)
}

impl Validate for Raster {
    fn validate(&self) -> Result<(), ValidationError> {
        check_raster_size(self.width, self.height)?;
        let expected = self.width as usize * self.height as usize * 4;
        if self.pixels.len() != expected {
            return Err(ValidationError::Invalid {
                what: "raster",
                reason: "pixel buffer does not match width x height x 4",
            });
        }
        Ok(())
    }
}

/// Checks raster dimensions against [`MAX_RASTER_SIDE_PX`] and [`MAX_RASTER_PIXELS`].
pub fn check_raster_size(width: u32, height: u32) -> Result<(), ValidationError> {
    if width == 0 || height == 0 || width > MAX_RASTER_SIDE_PX || height > MAX_RASTER_SIDE_PX {
        return Err(ValidationError::OutOfRange {
            what: "raster size",
        });
    }
    if u64::from(width) * u64::from(height) > u64::from(MAX_RASTER_PIXELS) {
        return Err(ValidationError::OutOfRange {
            what: "raster area",
        });
    }
    Ok(())
}

impl Validate for WorkerError {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text("error detail", &self.detail, MAX_ERROR_MESSAGE_BYTES)
    }
}

impl Validate for WorkerResponse {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            WorkerResponse::Hello { worker_version, .. } => {
                check_text("worker version", worker_version, MAX_VERSION_BYTES as u32)
            }
            WorkerResponse::Opened { document, .. } => document.validate(),
            WorkerResponse::Rendered { raster, .. } => raster.validate(),
            WorkerResponse::Outline { outline, .. } => outline.validate(),
            WorkerResponse::PageLinks {
                page_index, links, ..
            } => {
                check_count("page links", links.len(), MAX_LINKS_PER_PAGE)?;
                for link in links {
                    if link.id.page_index != *page_index {
                        return Err(ValidationError::Invalid {
                            what: "page link",
                            reason: "id refers to another page",
                        });
                    }
                    link.validate()?;
                }
                Ok(())
            }
            WorkerResponse::PageAnnotations { annotations, .. } => {
                check_count(
                    "page annotations",
                    annotations.len(),
                    MAX_ANNOTATIONS_PER_PAGE,
                )?;
                annotations.iter().try_for_each(PageAnnotation::validate)
            }
            WorkerResponse::PageFields {
                page_index, fields, ..
            } => {
                check_count("page fields", fields.len(), MAX_FIELDS_PER_PAGE)?;
                let mut ids = HashSet::new();
                for field in fields {
                    if !ids.insert(field.id) {
                        return Err(ValidationError::Invalid {
                            what: "page fields",
                            reason: "a field appears twice",
                        });
                    }
                    field.validate()?;
                }
                let _ = page_index;
                Ok(())
            }
            WorkerResponse::PageText { text, .. } => text.validate(),
            WorkerResponse::StampImage {
                png, width, height, ..
            } => {
                if stamp_png_size(png)? != (*width, *height) {
                    return Err(ValidationError::Invalid {
                        what: "stamp picture",
                        reason: "is not the size it says",
                    });
                }
                Ok(())
            }
            WorkerResponse::Png { png, .. } => check_png(png),
            WorkerResponse::Jpeg { jpeg, .. } => check_jpeg(jpeg),
            WorkerResponse::PageSearched { hits, .. } => {
                check_count("search hits", hits.len(), MAX_SEARCH_HITS)?;
                hits.iter().try_for_each(SearchHit::validate)
            }
            WorkerResponse::Edited { pages, .. } => check_pages(pages),
            WorkerResponse::Saved { bytes, .. } => {
                if *bytes == 0 || *bytes > MAX_DOCUMENT_BYTES {
                    return Err(ValidationError::OutOfRange {
                        what: "saved file size",
                    });
                }
                Ok(())
            }
            WorkerResponse::Rebased { .. } => Ok(()),
            WorkerResponse::OcrLoaded { .. }
            | WorkerResponse::OcrChecked { .. }
            | WorkerResponse::OcrStopped { .. } => Ok(()),
            WorkerResponse::OcrPolled {
                finished, waiting, ..
            } => {
                check_count("recognised pages", finished.len(), MAX_OCR_RESULTS)?;
                if *waiting > MAX_OCR_QUEUE {
                    return Err(ValidationError::OutOfRange {
                        what: "pages waiting to be recognised",
                    });
                }
                for page in finished {
                    if page.page_index >= MAX_PAGE_COUNT {
                        return Err(ValidationError::OutOfRange { what: "page index" });
                    }
                    if matches!(page.outcome, OcrOutcome::Recognised { chars } if chars > MAX_PAGE_TEXT_CHARS)
                    {
                        return Err(ValidationError::OutOfRange {
                            what: "recognised characters",
                        });
                    }
                }
                Ok(())
            }
            WorkerResponse::Error { error, .. } => error.validate(),
        }
    }
}

impl Validate for RenderPageArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_scale(self.scale)
    }
}

/// Checks a render scale against [`MIN_RENDER_SCALE`] and [`MAX_RENDER_SCALE`].
pub fn check_scale(scale: f32) -> Result<(), ValidationError> {
    if !scale.is_finite() {
        return Err(ValidationError::NotFinite {
            what: "render scale",
        });
    }
    if !(MIN_RENDER_SCALE..=MAX_RENDER_SCALE).contains(&scale) {
        return Err(ValidationError::OutOfRange {
            what: "render scale",
        });
    }
    Ok(())
}

impl Validate for SearchArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.query.is_empty() {
            return Err(ValidationError::Invalid {
                what: "search query",
                reason: "empty",
            });
        }
        check_text("search query", &self.query, MAX_QUERY_BYTES)
    }
}

impl Validate for UnlockArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_password(&self.password)
    }
}

impl Validate for UndoArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        self.password.as_ref().map_or(Ok(()), check_password)
    }
}

/// A password the user typed: some, no NUL (MuPDF takes it as a C string), bounded.
fn check_password(password: &Password) -> Result<(), ValidationError> {
    let password = password.as_str();
    if password.is_empty() {
        return Err(ValidationError::Invalid {
            what: "password",
            reason: "empty",
        });
    }
    // MuPDF takes the password as a C string.
    if password.contains(' ') {
        return Err(ValidationError::Invalid {
            what: "password",
            reason: "contains a NUL character",
        });
    }
    check_text("password", password, MAX_PASSWORD_BYTES)
}

/// The first bytes of every PNG file.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// An exported page: a PNG file of at most `MAX_PNG_BYTES`.
fn check_png(png: &[u8]) -> Result<(), ValidationError> {
    check_count("PNG bytes", png.len(), MAX_PNG_BYTES as u32)?;
    if !png.starts_with(&PNG_SIGNATURE) {
        return Err(ValidationError::Invalid {
            what: "PNG",
            reason: "does not start with the PNG signature",
        });
    }
    Ok(())
}

/// The picture of a stamp (B2-08): a PNG file of at most `MAX_STAMP_PNG_BYTES`, its first chunk
/// the header, of a size from 1 x 1 up to `MAX_STAMP_SIDE_PX` on each side. Returns that size,
/// as the header says it. Nothing of the picture is decoded.
pub fn stamp_png_size(png: &[u8]) -> Result<(u32, u32), ValidationError> {
    check_count("stamp PNG bytes", png.len(), MAX_STAMP_PNG_BYTES as u32)?;
    // The signature, then the header chunk: 13 bytes of data, named IHDR, the size first.
    let header = png.get(8..24).filter(|_| png.starts_with(&PNG_SIGNATURE));
    let Some(header) =
        header.filter(|header| header[..4] == [0, 0, 0, 13] && &header[4..8] == b"IHDR")
    else {
        return Err(ValidationError::Invalid {
            what: "stamp PNG",
            reason: "does not start with the PNG signature and header",
        });
    };
    let side = |bytes: &[u8]| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let (width, height) = (side(&header[8..12]), side(&header[12..16]));
    for side in [width, height] {
        if side == 0 || side > MAX_STAMP_SIDE_PX {
            return Err(ValidationError::OutOfRange {
                what: "stamp picture size",
            });
        }
    }
    Ok((width, height))
}

/// The first bytes of every JPEG file: the start-of-image marker, then another marker.
pub const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// An exported page (#111): a JPEG file of at most `MAX_JPEG_BYTES`.
fn check_jpeg(jpeg: &[u8]) -> Result<(), ValidationError> {
    check_count("JPEG bytes", jpeg.len(), MAX_JPEG_BYTES as u32)?;
    if !jpeg.starts_with(&JPEG_SIGNATURE) {
        return Err(ValidationError::Invalid {
            what: "JPEG",
            reason: "does not start with the JPEG markers",
        });
    }
    Ok(())
}

impl Validate for ExportArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.pages.is_empty() {
            return Err(ValidationError::Invalid {
                what: "export pages",
                reason: "empty",
            });
        }
        // A PDF takes its pages by reference, not one by one through the page text or a render:
        // as many as a document has.
        let pdf = matches!(
            self.format,
            ExportFormat::Pdf | ExportFormat::PdfEvery { .. }
        );
        check_count(
            "export pages",
            self.pages.len(),
            if pdf {
                MAX_PAGE_COUNT
            } else {
                MAX_EXPORT_PAGES
            },
        )?;
        let unique: HashSet<u32> = self.pages.iter().copied().collect();
        if unique.len() != self.pages.len() {
            return Err(ValidationError::Invalid {
                what: "export pages",
                reason: "a page appears twice",
            });
        }
        if let ExportFormat::Png { dpi } | ExportFormat::Jpg { dpi } = self.format
            && !matches!(dpi, 72 | 150 | 300)
        {
            return Err(ValidationError::OutOfRange { what: "export dpi" });
        }
        if let ExportFormat::PdfEvery { count } = self.format {
            if count == 0 {
                return Err(ValidationError::OutOfRange {
                    what: "pages per file",
                });
            }
            let files = self.pages.len().div_ceil(count as usize);
            check_count("split files", files, MAX_SPLIT_FILES)?;
        }
        Ok(())
    }
}

impl Validate for EditArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        self.edit.validate()
    }
}

/// An edit's own bounds; its pages are checked against the document by the caller
/// ([`check_page_index`]).
impl Validate for Edit {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Edit::RotatePages { pages, by } => {
                check_page_list("pages to rotate", pages)?;
                if *by == Rotation::None {
                    return Err(ValidationError::Invalid {
                        what: "rotation",
                        reason: "turns nothing",
                    });
                }
                Ok(())
            }
            Edit::DeletePages { pages } => check_page_list("pages to delete", pages),
            Edit::MovePages { pages, before } => {
                check_page_list("pages to move", pages)?;
                if *before > MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "page to move before",
                    });
                }
                Ok(())
            }
            Edit::InsertBlankPage { at, like } => {
                if *at > MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "where to insert",
                    });
                }
                if *like >= MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "page to take the size of",
                    });
                }
                Ok(())
            }
            Edit::AddHighlight { marks, .. } => {
                if marks.is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "highlight",
                        reason: "on no page",
                    });
                }
                check_count("highlighted pages", marks.len(), MAX_HIGHLIGHT_PAGES)?;
                let pages: HashSet<u32> = marks.iter().map(|mark| mark.page).collect();
                if pages.len() != marks.len() {
                    return Err(ValidationError::Invalid {
                        what: "highlighted pages",
                        reason: "a page appears twice",
                    });
                }
                let quads: usize = marks.iter().map(|mark| mark.quads.len()).sum();
                check_count("highlight quads", quads, MAX_ANNOTATION_QUADS)?;
                for mark in marks {
                    check_annotated_page(mark.page)?;
                    if mark.quads.is_empty() {
                        return Err(ValidationError::Invalid {
                            what: "highlight",
                            reason: "covers nothing on a page",
                        });
                    }
                    mark.quads.iter().try_for_each(Quad::validate)?;
                }
                Ok(())
            }
            Edit::AddNote { page, at, text } => {
                check_annotated_page(*page)?;
                at.validate()?;
                check_note_text(text)
            }
            Edit::DeleteAnnotation { page, .. } | Edit::SetHighlightColor { page, .. } => {
                check_annotated_page(*page)
            }
            Edit::SetNoteText { page, text, .. } => {
                check_annotated_page(*page)?;
                check_note_text(text)
            }
            Edit::SetFieldValue { page, value, .. } => {
                check_annotated_page(*page)?;
                check_field_value(value)
            }
            Edit::FlattenForm => Ok(()),
            Edit::AddInk { page, strokes, .. } => {
                check_annotated_page(*page)?;
                if strokes.is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "ink",
                        reason: "has no stroke",
                    });
                }
                check_count("ink strokes", strokes.len(), MAX_INK_STROKES)?;
                let points: usize = strokes.iter().map(Vec::len).sum();
                check_count("ink points", points, MAX_INK_POINTS)?;
                for stroke in strokes {
                    if stroke.is_empty() {
                        return Err(ValidationError::Invalid {
                            what: "ink",
                            reason: "has a stroke without points",
                        });
                    }
                    stroke.iter().try_for_each(Point::validate)?;
                }
                Ok(())
            }
            Edit::AddStamp { page, rect, .. } | Edit::SetAnnotationRect { page, rect, .. } => {
                check_annotated_page(*page)?;
                check_annotation_rect(rect)
            }
        }
    }
}

/// A rectangle an annotation is put in or moved to (B2-08): finite, upright, and large enough to
/// be seen and grabbed.
fn check_annotation_rect(rect: &Rect) -> Result<(), ValidationError> {
    rect.validate()?;
    if rect.x1 - rect.x0 < MIN_ANNOTATION_SIDE_PT || rect.y1 - rect.y0 < MIN_ANNOTATION_SIDE_PT {
        return Err(ValidationError::Invalid {
            what: "annotation rectangle",
            reason: "too small, or upside down",
        });
    }
    Ok(())
}

/// The page an annotation edit is on: one a document can have; whether this document has it is
/// checked by the caller ([`check_page_index`]).
fn check_annotated_page(page: u32) -> Result<(), ValidationError> {
    if page >= MAX_PAGE_COUNT {
        return Err(ValidationError::OutOfRange {
            what: "annotated page",
        });
    }
    Ok(())
}

/// What a note says (B2-07): something, within the length limit, and only text.
fn check_note_text(text: &str) -> Result<(), ValidationError> {
    if text.trim().is_empty() {
        return Err(ValidationError::Invalid {
            what: "note text",
            reason: "empty",
        });
    }
    check_text("note text", text, MAX_NOTE_TEXT_BYTES)?;
    if !is_note_text(text) {
        return Err(ValidationError::Invalid {
            what: "note text",
            reason: "contains control or invisible formatting characters",
        });
    }
    Ok(())
}

/// A form field's value (B2-09): within the limit, and only text (it may be empty, and may have
/// several lines).
fn check_field_value(value: &str) -> Result<(), ValidationError> {
    check_text("field value", value, MAX_FIELD_VALUE_BYTES)?;
    if !is_note_text(value) {
        return Err(ValidationError::Invalid {
            what: "field value",
            reason: "contains control or invisible formatting characters",
        });
    }
    Ok(())
}

impl Validate for FormField {
    fn validate(&self) -> Result<(), ValidationError> {
        self.rect.validate()?;
        if let Some(label) = &self.label {
            check_text("field label", label, MAX_TEXT_BYTES)?;
            check_clean("field label", label)?;
        }
        check_field_value(&self.value)?;
        if let Some(on_value) = &self.on_value {
            check_text("field on value", on_value, MAX_TEXT_BYTES)?;
            check_field_text_line("field on value", on_value)?;
        }
        check_count("field options", self.options.len(), MAX_FIELD_OPTIONS)?;
        for option in &self.options {
            check_text("field option", &option.value, MAX_TEXT_BYTES)?;
            check_field_text_line("field option", &option.value)?;
            check_text("field option label", &option.label, MAX_TEXT_BYTES)?;
            check_field_text_line("field option label", &option.label)?;
        }
        if let Some(max_len) = self.max_len
            && max_len == 0
        {
            return Err(ValidationError::OutOfRange {
                what: "field length limit",
            });
        }
        Ok(())
    }
}

/// One line of text of a field that is not the field's own text: no line breaks either.
fn check_field_text_line(what: &'static str, text: &str) -> Result<(), ValidationError> {
    if !is_note_text(text) || text.contains('\n') {
        return Err(ValidationError::Invalid {
            what,
            reason: "contains control or invisible formatting characters",
        });
    }
    Ok(())
}

impl Validate for PageAnnotation {
    fn validate(&self) -> Result<(), ValidationError> {
        self.rect.validate()?;
        if let Some(text) = &self.text {
            check_text("note text", text, MAX_NOTE_TEXT_BYTES)?;
            if !is_note_text(text)
                || text
                    .split('\n')
                    .any(|line| !line.is_empty() && !is_clean_display_text(line))
            {
                return Err(ValidationError::Invalid {
                    what: "note text",
                    reason: "not cleaned",
                });
            }
        }
        Ok(())
    }
}

/// Pages an edit names: some, at most `MAX_PAGE_COUNT`, none twice.
fn check_page_list(what: &'static str, pages: &[u32]) -> Result<(), ValidationError> {
    if pages.is_empty() {
        return Err(ValidationError::Invalid {
            what,
            reason: "empty",
        });
    }
    check_count(what, pages.len(), MAX_PAGE_COUNT)?;
    let unique: HashSet<u32> = pages.iter().copied().collect();
    if unique.len() != pages.len() {
        return Err(ValidationError::Invalid {
            what,
            reason: "a page appears twice",
        });
    }
    Ok(())
}

/// A display name is a bare file name; anything that looks like a path is a bug.
fn check_display_name(name: &str) -> Result<(), ValidationError> {
    check_text("display name", name, MAX_DISPLAY_NAME_BYTES)?;
    if name.contains(['/', '\\', ':']) {
        return Err(ValidationError::Invalid {
            what: "display name",
            reason: "contains path separators",
        });
    }
    Ok(())
}

impl Validate for RecentFile {
    fn validate(&self) -> Result<(), ValidationError> {
        check_display_name(&self.display_name)
    }
}

/// The recent files list the frontend gets (#73).
impl Validate for [RecentFile] {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count("recent files", self.len(), MAX_RECENT_FILES)?;
        self.iter().try_for_each(RecentFile::validate)
    }
}

impl Validate for DocumentInfo {
    fn validate(&self) -> Result<(), ValidationError> {
        check_display_name(&self.display_name)?;
        check_count("pages", self.pages.len(), MAX_PAGE_COUNT)?;
        self.pages.iter().try_for_each(PageSize::validate)?;
        self.security.validate()
    }
}

impl Validate for IpcError {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text("error message", &self.message, MAX_ERROR_MESSAGE_BYTES)
    }
}

impl Validate for OpenEvent {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            OpenEvent::DragHover { .. } | OpenEvent::TabLimit { .. } => Ok(()),
            OpenEvent::CloseRequested { tabs } => check_count("unsaved tabs", tabs.len(), MAX_TABS),
            OpenEvent::Opening { display_name, .. }
            | OpenEvent::PasswordNeeded { display_name, .. } => check_display_name(display_name),
            OpenEvent::Opened { info, .. } => info.validate(),
            OpenEvent::Failed {
                display_name,
                error,
                ..
            } => {
                check_display_name(display_name)?;
                error.validate()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AnnotationId, AnnotationKind, BlockedAction, DocumentId, DocumentPermissions, ErrorCode,
        FieldId, FieldKind, FieldOption, HighlightColor, HighlightMark, InkColor, InkWidth, LinkId,
        RecentId, Recovery, RequestId, Rotation, SecurityFinding, StampName, TabId,
    };
    use crate::worker::WorkerErrorCode;

    fn page(width_pt: f32, height_pt: f32) -> PageSize {
        PageSize {
            width_pt,
            height_pt,
        }
    }

    fn quad() -> Quad {
        let point = |x, y| Point { x, y };
        Quad {
            ul: point(0.0, 0.0),
            ur: point(10.0, 0.0),
            ll: point(0.0, 5.0),
            lr: point(10.0, 5.0),
        }
    }

    fn outline_item(depth: u16) -> OutlineItem {
        OutlineItem {
            title: "Chapter".to_owned(),
            depth,
            target: Some(LinkTarget::Page {
                page_index: 0,
                x: None,
                y: None,
            }),
        }
    }

    #[test]
    fn page_sizes_must_be_positive_finite_and_bounded() {
        assert!(page(612.0, 792.0).validate().is_ok());
        assert!(page(0.0, 792.0).validate().is_err());
        assert!(page(-1.0, 792.0).validate().is_err());
        assert!(page(f32::NAN, 792.0).validate().is_err());
        assert!(page(612.0, f32::INFINITY).validate().is_err());
        assert!(page(MAX_PAGE_SIDE_PT * 2.0, 792.0).validate().is_err());
    }

    #[test]
    fn documents_need_at_least_one_and_at_most_the_limit_of_pages() {
        let doc = |pages: Vec<PageSize>| OpenedDocument {
            pages,
            has_outline: false,
            has_form: false,
            security: SecurityReport::default(),
            permissions: DocumentPermissions::ALL,
            encrypted: false,
        };
        assert!(doc(vec![page(612.0, 792.0)]).validate().is_ok());
        assert!(doc(vec![]).validate().is_err());
        let too_many = vec![page(612.0, 792.0); MAX_PAGE_COUNT as usize + 1];
        assert!(matches!(
            doc(too_many).validate(),
            Err(ValidationError::TooMany { what: "pages", .. })
        ));
    }

    #[test]
    fn duplicate_security_findings_are_rejected() {
        let finding = SecurityFinding {
            kind: FindingKind::JavaScript,
            count: 1,
        };
        let report = SecurityReport {
            findings: vec![finding, finding],
            scan_complete: true,
        };
        assert!(report.validate().is_err());
    }

    #[test]
    fn raster_must_match_its_dimensions_and_limits() {
        let raster = |width: u32, height: u32, len: usize| Raster {
            width,
            height,
            pixels: vec![255; len],
        };
        assert!(raster(2, 3, 24).validate().is_ok());
        assert!(raster(2, 3, 23).validate().is_err());
        assert!(raster(0, 3, 0).validate().is_err());
        assert!(check_raster_size(MAX_RASTER_SIDE_PX + 1, 1).is_err());
        assert!(check_raster_size(MAX_RASTER_SIDE_PX, MAX_RASTER_SIDE_PX).is_err());
        assert!(check_raster_size(4096, 4096).is_ok());
    }

    #[test]
    fn outline_limits_and_structure_are_enforced() {
        let ok = OutlineResult {
            items: vec![
                outline_item(0),
                outline_item(1),
                outline_item(1),
                outline_item(0),
            ],
            truncated: false,
        };
        assert!(ok.validate().is_ok());

        let skips_level = OutlineResult {
            items: vec![outline_item(0), outline_item(2)],
            truncated: false,
        };
        assert!(skips_level.validate().is_err());

        let starts_deep = OutlineResult {
            items: vec![outline_item(1)],
            truncated: false,
        };
        assert!(starts_deep.validate().is_err());

        let too_many = OutlineResult {
            items: vec![outline_item(0); MAX_OUTLINE_ITEMS as usize + 1],
            truncated: true,
        };
        assert!(too_many.validate().is_err());

        let long_title = OutlineItem {
            title: "x".repeat(MAX_TEXT_BYTES as usize + 1),
            ..outline_item(0)
        };
        assert!(long_title.validate().is_err());
    }

    #[test]
    fn link_targets_are_bounded() {
        let uri = |uri: String| LinkTarget::Uri { uri };
        assert!(
            uri("https://example.invalid/".to_owned())
                .validate()
                .is_ok()
        );
        assert!(uri(String::new()).validate().is_err());
        // MVP-12 must be able to show a 10,000-character URL.
        let long = |len: usize| format!("https://example.invalid/{}", "a".repeat(len - 24));
        assert!(uri(long(10_000)).validate().is_ok());
        assert!(uri(long(MAX_URI_BYTES as usize + 1)).validate().is_err());
        // Only http, https and mailto.
        assert!(uri("file:///C:/x.exe".to_owned()).validate().is_err());
        assert!(uri("javascript:alert(1)".to_owned()).validate().is_err());
        // Hidden characters stay, so that the confirmation can show them (MVP-12).
        assert!(
            uri("https://a.invalid/\u{202E}exe.pdf".to_owned())
                .validate()
                .is_ok()
        );

        let blocked = LinkTarget::Blocked {
            action: BlockedAction::Launch,
            target: Some("x".repeat(MAX_TEXT_BYTES as usize + 1)),
        };
        assert!(blocked.validate().is_err());
        let hidden = LinkTarget::Blocked {
            action: BlockedAction::Launch,
            target: Some("calc\u{202E}fdp.exe".to_owned()),
        };
        assert!(hidden.validate().is_err());

        let nan_destination = LinkTarget::Page {
            page_index: 0,
            x: Some(f32::NAN),
            y: None,
        };
        assert!(nan_destination.validate().is_err());
    }

    fn text_line(text: &str, edges: Vec<f32>) -> TextLine {
        TextLine {
            text: text.to_owned(),
            quad: quad(),
            edges,
        }
    }

    #[test]
    fn text_lines_have_clean_text_and_an_edge_per_character() {
        assert_eq!(
            text_line("中文 ok", vec![0.0, 2.0, 4.0, 5.0, 7.0, 9.0]).validate(),
            Ok(())
        );
        // A right-to-left override could reorder what is pasted; a tab is not a space.
        for text in ["a\u{202E}b", "a\tb", "a\nb"] {
            let edges = vec![0.0; text.chars().count() + 1];
            assert!(text_line(text, edges).validate().is_err(), "{text:?}");
        }
        assert!(text_line("", vec![0.0]).validate().is_err());
        assert!(text_line("ab", vec![0.0, 1.0]).validate().is_err());
        assert!(text_line("ab", vec![0.0, 2.0, 1.0]).validate().is_err());
        assert!(text_line("ab", vec![-1.0, 0.0, 1.0]).validate().is_err());
        assert!(
            text_line("ab", vec![0.0, f32::NAN, 1.0])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn page_text_is_bounded() {
        let line = text_line("abcd", vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        let lines = |count: usize| PageText {
            lines: vec![line.clone(); count],
            truncated: true,
            recognised: false,
        };
        let most = MAX_PAGE_TEXT_CHARS as usize / 4;
        assert_eq!(lines(most).validate(), Ok(()));
        assert!(matches!(
            lines(most + 1).validate(),
            Err(ValidationError::TooMany { .. })
        ));
        let response = WorkerResponse::PageText {
            request: RequestId(1),
            page_index: 0,
            text: PageText {
                lines: vec![text_line("a", vec![0.0])],
                truncated: false,
                recognised: false,
            },
        };
        assert!(response.validate().is_err());
    }

    #[test]
    fn recognised_pages_are_bounded() {
        use crate::types::DocumentId;
        use crate::worker::{OcrFinished, OcrOutcome};
        let page = |page_index: u32, outcome: OcrOutcome| OcrFinished {
            doc: DocumentId(1),
            page_index,
            outcome,
        };
        let polled = |finished: Vec<OcrFinished>, waiting: u32| WorkerResponse::OcrPolled {
            request: RequestId(1),
            finished,
            waiting,
        };
        let done = OcrOutcome::Recognised { chars: 10 };
        assert_eq!(polled(vec![], 0).validate(), Ok(()));
        assert_eq!(
            polled(
                vec![page(0, done), page(99_999, OcrOutcome::TimedOut)],
                MAX_OCR_QUEUE
            )
            .validate(),
            Ok(())
        );
        let many = vec![page(0, done); MAX_OCR_RESULTS as usize];
        assert_eq!(polled(many.clone(), 0).validate(), Ok(()));
        let mut too_many = many;
        too_many.push(page(0, done));
        assert!(matches!(
            polled(too_many, 0).validate(),
            Err(ValidationError::TooMany { .. })
        ));
        assert!(polled(vec![], MAX_OCR_QUEUE + 1).validate().is_err());
        assert!(
            polled(vec![page(MAX_PAGE_COUNT, done)], 0)
                .validate()
                .is_err()
        );
        let chars = |chars: u32| polled(vec![page(0, OcrOutcome::Recognised { chars })], 0);
        assert_eq!(chars(MAX_PAGE_TEXT_CHARS).validate(), Ok(()));
        assert!(chars(MAX_PAGE_TEXT_CHARS + 1).validate().is_err());
    }

    fn unlock(password: &str) -> UnlockArgs {
        UnlockArgs {
            tab: TabId(1),
            password: crate::types::Password::new(password.to_owned()),
        }
    }

    #[test]
    fn a_password_is_never_shown_and_travels_as_a_plain_string() {
        let args = unlock("s3cret");
        assert!(!format!("{args:?}").contains("s3cret"));
        let json = serde_json::to_string(&args).unwrap();
        assert_eq!(json, r#"{"tab":1,"password":"s3cret"}"#);
        assert_eq!(serde_json::from_str::<UnlockArgs>(&json).unwrap(), args);
        // Nothing but the tab and the password.
        assert!(
            serde_json::from_str::<UnlockArgs>(r#"{"tab":1,"password":"x","path":"C:\\x.pdf"}"#)
                .is_err()
        );
    }

    #[test]
    fn unlock_passwords_are_bounded() {
        assert_eq!(unlock("中文密碼 and spaces").validate(), Ok(()));
        assert!(unlock("").validate().is_err());
        assert!(unlock("a\0b").validate().is_err());
        assert!(
            unlock(&"x".repeat(MAX_PASSWORD_BYTES as usize))
                .validate()
                .is_ok()
        );
        assert!(
            unlock(&"x".repeat(MAX_PASSWORD_BYTES as usize + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_tab_asking_for_a_password_has_a_plain_file_name() {
        let asking = |display_name: &str| OpenEvent::PasswordNeeded {
            tab: TabId(1),
            display_name: display_name.to_owned(),
            wrong: true,
        };
        assert_eq!(asking("機密.pdf").validate(), Ok(()));
        assert!(asking(r"C:\secret\機密.pdf").validate().is_err());
    }

    #[test]
    fn page_links_must_belong_to_the_reported_page() {
        let link = |page_index| PageLink {
            id: LinkId {
                page_index,
                index: 0,
            },
            rect: Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: 10.0,
            },
            target: LinkTarget::Page {
                page_index: 1,
                x: None,
                y: None,
            },
        };
        let response = |links| WorkerResponse::PageLinks {
            request: RequestId(1),
            page_index: 4,
            links,
        };
        assert!(response(vec![link(4)]).validate().is_ok());
        assert!(response(vec![link(5)]).validate().is_err());
        assert!(
            response(vec![link(4); MAX_LINKS_PER_PAGE as usize + 1])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn search_hits_are_bounded() {
        let hits = |count: usize| WorkerResponse::PageSearched {
            request: RequestId(1),
            page_index: 0,
            hits: vec![
                SearchHit {
                    quads: vec![quad()]
                };
                count
            ],
            has_text: true,
        };
        assert!(hits(3).validate().is_ok());
        assert!(hits(MAX_SEARCH_HITS as usize + 1).validate().is_err());
        assert!(SearchHit { quads: vec![] }.validate().is_err());
        assert!(
            SearchHit {
                quads: vec![quad(); MAX_QUADS_PER_HIT as usize + 1]
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn worker_error_detail_is_bounded() {
        let error = |detail: String| WorkerResponse::Error {
            request: None,
            error: WorkerError {
                code: WorkerErrorCode::Corrupted,
                detail,
            },
        };
        assert!(error("bad xref".to_owned()).validate().is_ok());
        assert!(
            error("x".repeat(MAX_ERROR_MESSAGE_BYTES as usize + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn render_scale_is_bounded() {
        let args = |scale| RenderPageArgs {
            request: RequestId(1),
            doc: DocumentId(1),
            page_index: 0,
            scale,
            rotation: Rotation::None,
        };
        assert!(args(1.0).validate().is_ok());
        assert!(args(0.0).validate().is_err());
        assert!(args(f32::NAN).validate().is_err());
        assert!(args(MAX_RENDER_SCALE * 2.0).validate().is_err());
    }

    #[test]
    fn search_query_must_be_non_empty_and_bounded() {
        let args = |query: String| SearchArgs {
            request: RequestId(1),
            doc: DocumentId(1),
            query,
            case_sensitive: false,
        };
        assert!(args("隱私".to_owned()).validate().is_ok());
        assert!(args(String::new()).validate().is_err());
        assert!(
            args("x".repeat(MAX_QUERY_BYTES as usize + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn exports_name_known_pages_formats_and_resolutions() {
        let args = |pages: Vec<u32>, format: ExportFormat| ExportArgs {
            request: RequestId(1),
            doc: DocumentId(1),
            pages,
            format,
        };
        assert!(args(vec![0, 2, 1], ExportFormat::Text).validate().is_ok());
        assert!(
            args(vec![0], ExportFormat::Png { dpi: 150 })
                .validate()
                .is_ok()
        );
        assert!(args(vec![], ExportFormat::Text).validate().is_err());
        assert!(args(vec![1, 1], ExportFormat::Text).validate().is_err());
        assert!(
            args(vec![0], ExportFormat::Png { dpi: 96 })
                .validate()
                .is_err()
        );
        for dpi in [72, 150, 300] {
            assert!(args(vec![0], ExportFormat::Jpg { dpi }).validate().is_ok());
        }
        assert!(
            args(vec![0], ExportFormat::Jpg { dpi: 600 })
                .validate()
                .is_err()
        );
        let too_many = (0..=MAX_EXPORT_PAGES).collect();
        assert!(matches!(
            args(too_many, ExportFormat::Text).validate(),
            Err(ValidationError::TooMany { .. })
        ));
    }

    #[test]
    fn a_split_takes_as_many_pages_as_a_document_has_but_not_too_many_files() {
        let args = |pages: Vec<u32>, format: ExportFormat| ExportArgs {
            request: RequestId(1),
            doc: DocumentId(1),
            pages,
            format,
        };
        // Pages go to a PDF by reference: more than a text or an image export takes.
        let many: Vec<u32> = (0..=MAX_EXPORT_PAGES).collect();
        assert!(args(many.clone(), ExportFormat::Pdf).validate().is_ok());
        assert!(args(many, ExportFormat::Text).validate().is_err());
        let all: Vec<u32> = (0..MAX_PAGE_COUNT).collect();
        assert!(args(all.clone(), ExportFormat::Pdf).validate().is_ok());
        assert!(
            args((0..=MAX_PAGE_COUNT).collect(), ExportFormat::Pdf)
                .validate()
                .is_err()
        );
        assert!(args(vec![], ExportFormat::Pdf).validate().is_err());
        assert!(args(vec![3, 3], ExportFormat::Pdf).validate().is_err());

        // Pieces: at least a page each, and no more than MAX_SPLIT_FILES of them.
        let every = |count: u32| ExportFormat::PdfEvery { count };
        assert!(args(vec![0, 1, 2], every(1)).validate().is_ok());
        assert!(args(vec![0, 1, 2], every(500)).validate().is_ok());
        assert!(args(vec![0, 1, 2], every(0)).validate().is_err());
        let thousand: Vec<u32> = (0..MAX_SPLIT_FILES).collect();
        assert!(args(thousand, every(1)).validate().is_ok());
        let more: Vec<u32> = (0..=MAX_SPLIT_FILES).collect();
        assert!(args(more.clone(), every(1)).validate().is_err());
        assert!(args(more, every(2)).validate().is_ok());
        assert!(args(all, every(100)).validate().is_ok());
    }

    #[test]
    fn exported_pages_are_png_files_of_bounded_size() {
        let png = |png: Vec<u8>| WorkerResponse::Png {
            request: RequestId(1),
            png,
        };
        let mut file = PNG_SIGNATURE.to_vec();
        file.extend_from_slice(b"rest of the file");
        assert!(png(file).validate().is_ok());
        assert!(png(b"GIF89a".to_vec()).validate().is_err());
        let mut huge = PNG_SIGNATURE.to_vec();
        huge.resize(MAX_PNG_BYTES + 1, 0);
        assert!(png(huge).validate().is_err());
    }

    #[test]
    fn stamp_pictures_from_the_worker_are_png_headers_of_a_bounded_size() {
        let png = |width: u32, height: u32| {
            let mut file = PNG_SIGNATURE.to_vec();
            file.extend_from_slice(&13u32.to_be_bytes());
            file.extend_from_slice(b"IHDR");
            file.extend_from_slice(&width.to_be_bytes());
            file.extend_from_slice(&height.to_be_bytes());
            file.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
            file
        };
        assert_eq!(stamp_png_size(&png(64, 32)), Ok((64, 32)));
        assert_eq!(
            stamp_png_size(&png(MAX_STAMP_SIDE_PX, 1)),
            Ok((MAX_STAMP_SIDE_PX, 1))
        );
        for wrong in [png(0, 32), png(64, 0), png(MAX_STAMP_SIDE_PX + 1, 32)] {
            assert!(stamp_png_size(&wrong).is_err());
        }
        // Not a PNG, not a header first, cut short, or too large.
        assert!(stamp_png_size(&[]).is_err());
        assert!(stamp_png_size(b"GIF89a, and then some more bytes").is_err());
        let mut other_chunk = png(64, 32);
        other_chunk[12..16].copy_from_slice(b"IDAT");
        assert!(stamp_png_size(&other_chunk).is_err());
        assert!(stamp_png_size(&png(64, 32)[..20]).is_err());
        let mut huge = png(64, 32);
        huge.resize(MAX_STAMP_PNG_BYTES + 1, 0);
        assert!(stamp_png_size(&huge).is_err());

        // The size the response says is the size the file has.
        let response = |png: Vec<u8>, width: u32, height: u32| WorkerResponse::StampImage {
            request: RequestId(1),
            png,
            width,
            height,
        };
        assert!(response(png(64, 32), 64, 32).validate().is_ok());
        assert!(response(png(64, 32), 65, 32).validate().is_err());
        assert!(response(png(64, 32), 32, 64).validate().is_err());
    }

    #[test]
    fn exported_pages_are_jpeg_files_of_bounded_size() {
        let jpeg = |jpeg: Vec<u8>| WorkerResponse::Jpeg {
            request: RequestId(1),
            jpeg,
        };
        let mut file = JPEG_SIGNATURE.to_vec();
        file.extend_from_slice(b"\xE0rest of the file");
        assert!(jpeg(file).validate().is_ok());
        assert!(jpeg(PNG_SIGNATURE.to_vec()).validate().is_err());
        assert!(jpeg(vec![0xFF, 0xD8]).validate().is_err());
        let mut huge = JPEG_SIGNATURE.to_vec();
        huge.resize(MAX_JPEG_BYTES + 1, 0);
        assert!(jpeg(huge).validate().is_err());
    }

    #[test]
    fn recent_files_show_names_only_and_are_bounded() {
        let file = |id: u32, display_name: &str| RecentFile {
            id: RecentId(id),
            display_name: display_name.to_owned(),
        };
        assert!([file(1, "報告.pdf")].validate().is_ok());
        assert!([file(1, r"C:\Users\someone\報告.pdf")].validate().is_err());
        let many: Vec<RecentFile> = (0..=MAX_RECENT_FILES).map(|id| file(id, "a.pdf")).collect();
        assert!(matches!(
            many.validate(),
            Err(ValidationError::TooMany {
                what: "recent files",
                ..
            })
        ));
        assert!(many[..MAX_RECENT_FILES as usize].validate().is_ok());
    }

    #[test]
    fn edits_name_existing_pages_once_and_turn_them() {
        let rotate = |pages: Vec<u32>, by: Rotation| EditArgs {
            doc: DocumentId(1),
            edit: Edit::RotatePages { pages, by },
        };
        assert!(rotate(vec![0, 2], Rotation::Cw90).validate().is_ok());
        assert!(rotate(vec![], Rotation::Cw90).validate().is_err());
        assert!(rotate(vec![1, 1], Rotation::Cw180).validate().is_err());
        assert!(rotate(vec![1], Rotation::None).validate().is_err());
        // The frontend's form: tagged by kind, as everything else it sends.
        assert_eq!(
            serde_json::from_value::<EditArgs>(serde_json::json!({
                "doc": 1,
                "edit": { "kind": "rotatePages", "pages": [0], "by": "cw270" }
            }))
            .unwrap(),
            rotate(vec![0], Rotation::Cw270)
        );
    }

    #[test]
    fn annotation_edits_are_bounded_and_notes_are_only_text() {
        let quad = |x: f32| Quad {
            ul: Point { x, y: 10.0 },
            ur: Point {
                x: x + 50.0,
                y: 10.0,
            },
            ll: Point { x, y: 22.0 },
            lr: Point {
                x: x + 50.0,
                y: 22.0,
            },
        };
        let mark = |page: u32, quads: Vec<Quad>| HighlightMark { page, quads };
        let highlight = |marks: Vec<HighlightMark>| Edit::AddHighlight {
            marks,
            color: HighlightColor::Yellow,
        };
        assert!(
            highlight(vec![mark(0, vec![quad(72.0)])])
                .validate()
                .is_ok()
        );
        // Across pages: one edit.
        assert!(
            highlight(vec![mark(0, vec![quad(72.0)]), mark(1, vec![quad(72.0)])])
                .validate()
                .is_ok()
        );
        assert!(highlight(vec![]).validate().is_err());
        assert!(highlight(vec![mark(0, vec![])]).validate().is_err());
        assert!(
            highlight(vec![mark(0, vec![quad(72.0)]), mark(0, vec![quad(72.0)])])
                .validate()
                .is_err()
        );
        assert!(
            highlight(vec![mark(MAX_PAGE_COUNT, vec![quad(72.0)])])
                .validate()
                .is_err()
        );
        assert!(
            highlight(vec![mark(0, vec![quad(f32::NAN)])])
                .validate()
                .is_err()
        );
        assert!(
            highlight(vec![mark(0, vec![quad(2.0 * MAX_PAGE_SIDE_PT)])])
                .validate()
                .is_err()
        );
        // The quads of all its pages count.
        let half = MAX_ANNOTATION_QUADS as usize / 2 + 1;
        assert!(
            highlight(vec![
                mark(0, vec![quad(72.0); half]),
                mark(1, vec![quad(72.0); half])
            ])
            .validate()
            .is_err()
        );
        assert!(
            highlight(
                (0..=MAX_HIGHLIGHT_PAGES)
                    .map(|page| mark(page, vec![quad(72.0)]))
                    .collect()
            )
            .validate()
            .is_err()
        );

        let note = |text: &str| Edit::AddNote {
            page: 0,
            at: Point { x: 72.0, y: 72.0 },
            text: text.to_owned(),
        };
        assert!(
            note("第一行\n  second line, with  its spaces")
                .validate()
                .is_ok()
        );
        assert!(note("").validate().is_err());
        assert!(note(" \n ").validate().is_err());
        assert!(note("a\tb").validate().is_err());
        assert!(note("a\u{202E}b").validate().is_err());
        assert!(
            note(&"字".repeat(MAX_NOTE_TEXT_BYTES as usize))
                .validate()
                .is_err()
        );
        let set_text = |text: &str| Edit::SetNoteText {
            page: 0,
            annotation: AnnotationId(5),
            text: text.to_owned(),
        };
        assert!(set_text("changed").validate().is_ok());
        assert!(set_text("a\u{0}b").validate().is_err());
        let delete = |page: u32| Edit::DeleteAnnotation {
            page,
            annotation: AnnotationId(5),
        };
        assert!(delete(3).validate().is_ok());
        assert!(delete(MAX_PAGE_COUNT).validate().is_err());

        // The frontend's form.
        assert_eq!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "setHighlightColor", "page": 1, "annotation": 9, "color": "pink"
            }))
            .unwrap(),
            Edit::SetHighlightColor {
                page: 1,
                annotation: AnnotationId(9),
                color: HighlightColor::Pink,
            }
        );
    }

    #[test]
    fn drawings_and_stamps_are_bounded_and_their_rectangles_are_upright_and_visible() {
        let ink = |strokes: Vec<Vec<Point>>| Edit::AddInk {
            page: 0,
            strokes,
            color: InkColor::Red,
            width: InkWidth::Thin,
        };
        let at = |x: f32| Point { x, y: 72.0 };
        assert!(
            ink(vec![vec![at(1.0), at(2.0)], vec![at(3.0)]])
                .validate()
                .is_ok()
        );
        assert!(ink(vec![]).validate().is_err());
        assert!(ink(vec![vec![]]).validate().is_err());
        assert!(ink(vec![vec![at(1.0)], vec![]]).validate().is_err());
        assert!(ink(vec![vec![at(f32::NAN)]]).validate().is_err());
        assert!(
            ink(vec![vec![at(2.0 * MAX_PAGE_SIDE_PT)]])
                .validate()
                .is_err()
        );
        // The strokes and the points of all of them count.
        let many = MAX_INK_STROKES as usize;
        assert!(ink(vec![vec![at(1.0)]; many]).validate().is_ok());
        assert!(ink(vec![vec![at(1.0)]; many + 1]).validate().is_err());
        let half = MAX_INK_POINTS as usize / 2 + 1;
        assert!(
            ink(vec![vec![at(1.0); half], vec![at(1.0); half]])
                .validate()
                .is_err()
        );
        assert!(
            ink(vec![vec![at(1.0); MAX_INK_POINTS as usize]])
                .validate()
                .is_ok()
        );
        let on_page = |page: u32| Edit::AddInk {
            page,
            strokes: vec![vec![at(1.0)]],
            color: InkColor::Black,
            width: InkWidth::Thick,
        };
        assert!(on_page(MAX_PAGE_COUNT).validate().is_err());

        let side = MIN_ANNOTATION_SIDE_PT;
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect { x0, y0, x1, y1 };
        let stamp = |rect: Rect| Edit::AddStamp {
            page: 0,
            rect,
            stamp: StampName::Approved,
        };
        let moved = |rect: Rect| Edit::SetAnnotationRect {
            page: 0,
            annotation: AnnotationId(5),
            rect,
        };
        for make in [stamp, moved] {
            assert!(
                make(rect(10.0, 10.0, 10.0 + side, 10.0 + side))
                    .validate()
                    .is_ok()
            );
            assert!(make(rect(10.0, 10.0, 200.0, 60.0)).validate().is_ok());
            // Too small to see, upside down, or not a number.
            assert!(
                make(rect(10.0, 10.0, 10.0 + side - 1.0, 60.0))
                    .validate()
                    .is_err()
            );
            assert!(
                make(rect(10.0, 10.0, 200.0, 10.0 + side - 1.0))
                    .validate()
                    .is_err()
            );
            assert!(make(rect(200.0, 10.0, 10.0, 60.0)).validate().is_err());
            assert!(make(rect(10.0, 60.0, 200.0, 10.0)).validate().is_err());
            assert!(
                make(rect(10.0, 10.0, f32::INFINITY, 60.0))
                    .validate()
                    .is_err()
            );
            assert!(
                make(rect(10.0, 10.0, 2.0 * MAX_PAGE_SIDE_PT, 60.0))
                    .validate()
                    .is_err()
            );
        }

        // The frontend form.
        assert_eq!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "addInk", "page": 2, "color": "blue", "width": "medium",
                "strokes": [[{"x": 1.0, "y": 2.0}, {"x": 3.0, "y": 4.0}]]
            }))
            .unwrap(),
            Edit::AddInk {
                page: 2,
                strokes: vec![vec![Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]],
                color: InkColor::Blue,
                width: InkWidth::Medium,
            }
        );
        assert!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "addStamp", "page": 0, "stamp": "notApproved",
                "rect": {"x0": 1.0, "y0": 2.0, "x1": 91.0, "y1": 52.0}
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "addStamp", "page": 0, "stamp": "Paid",
                "rect": {"x0": 1.0, "y0": 2.0, "x1": 91.0, "y1": 52.0}
            }))
            .is_err()
        );
    }

    #[test]
    fn annotations_from_the_worker_are_checked() {
        let annotation = |text: Option<&str>| PageAnnotation {
            id: AnnotationId(4),
            kind: AnnotationKind::Note,
            rect: Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 30.0,
                y1: 30.0,
            },
            color: None,
            text: text.map(str::to_owned),
        };
        let response = |annotations: Vec<PageAnnotation>| WorkerResponse::PageAnnotations {
            request: RequestId(1),
            page_index: 0,
            annotations,
        };
        assert!(
            response(vec![annotation(Some("one\n\ntwo"))])
                .validate()
                .is_ok()
        );
        assert!(response(vec![annotation(None)]).validate().is_ok());
        // Not cleaned: a line with a double space, a tab, a bidi override.
        for text in ["a  b", "a\tb", "a\u{202E}b"] {
            assert!(
                response(vec![annotation(Some(text))]).validate().is_err(),
                "{text:?}"
            );
        }
        let mut far = annotation(None);
        far.rect.x1 = f32::INFINITY;
        assert!(response(vec![far]).validate().is_err());
        assert!(
            response(vec![
                annotation(None);
                MAX_ANNOTATIONS_PER_PAGE as usize + 1
            ])
            .validate()
            .is_err()
        );
    }

    #[test]
    fn form_edits_are_bounded_and_values_are_only_text() {
        let set = |value: &str| Edit::SetFieldValue {
            page: 0,
            field: FieldId(6),
            value: value.to_owned(),
        };
        // A field may be cleared, and may have several lines and its own spacing.
        for good in ["", "Jane  Q.\nPublic", "隱私 first"] {
            assert!(set(good).validate().is_ok(), "{good:?}");
        }
        for bad in ["a\tb", "a\u{0}b", "a\u{202E}b"] {
            assert!(set(bad).validate().is_err(), "{bad:?}");
        }
        assert!(
            set(&"a".repeat(MAX_FIELD_VALUE_BYTES as usize))
                .validate()
                .is_ok()
        );
        assert!(
            set(&"a".repeat(MAX_FIELD_VALUE_BYTES as usize + 1))
                .validate()
                .is_err()
        );
        assert!(
            Edit::SetFieldValue {
                page: MAX_PAGE_COUNT,
                field: FieldId(6),
                value: String::new(),
            }
            .validate()
            .is_err()
        );
        assert!(Edit::FlattenForm.validate().is_ok());

        // The frontend's form.
        assert_eq!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "setFieldValue", "page": 2, "field": 6, "value": "x"
            }))
            .unwrap(),
            Edit::SetFieldValue {
                page: 2,
                field: FieldId(6),
                value: "x".to_owned()
            }
        );
        assert_eq!(
            serde_json::from_value::<Edit>(serde_json::json!({ "kind": "flattenForm" })).unwrap(),
            Edit::FlattenForm
        );
    }

    #[test]
    fn form_fields_from_the_worker_are_checked() {
        let field = |id: u32| FormField {
            id: FieldId(id),
            group: FieldId(id),
            kind: FieldKind::Combo,
            rect: Rect {
                x0: 72.0,
                y0: 100.0,
                x1: 300.0,
                y1: 124.0,
            },
            label: Some("Country".to_owned()),
            value: "TW".to_owned(),
            on_value: None,
            options: vec![FieldOption {
                value: "TW".to_owned(),
                label: "Taiwan".to_owned(),
            }],
            read_only: false,
            required: false,
            multiline: false,
            password: false,
            editable: false,
            multi_select: false,
            max_len: None,
            has_script: false,
        };
        let response = |fields: Vec<FormField>| WorkerResponse::PageFields {
            request: RequestId(1),
            page_index: 0,
            fields,
        };
        assert!(response(vec![field(1), field(2)]).validate().is_ok());
        assert!(response(vec![field(1), field(1)]).validate().is_err());
        assert!(
            response((0..=MAX_FIELDS_PER_PAGE).map(field).collect())
                .validate()
                .is_err()
        );
        let broken = |change: &dyn Fn(&mut FormField)| {
            let mut broken = field(1);
            change(&mut broken);
            response(vec![broken]).validate().is_err()
        };
        // Not cleaned: a label with a bidirectional override, a value with a control character,
        // an option with a line break; or out of bounds.
        assert!(broken(&|f| f.label = Some("a\u{202E}b".to_owned())));
        assert!(broken(&|f| f.value = "a\u{7}b".to_owned()));
        assert!(broken(&|f| f.options[0].label = "a\nb".to_owned()));
        assert!(broken(&|f| f.on_value = Some("a\tb".to_owned())));
        assert!(broken(&|f| f.rect.x1 = f32::NAN));
        assert!(broken(&|f| f.max_len = Some(0)));
        assert!(broken(&|f| {
            f.options = vec![f.options[0].clone(); MAX_FIELD_OPTIONS as usize + 1];
        }));
        assert!(broken(&|f| {
            f.value = "a".repeat(MAX_FIELD_VALUE_BYTES as usize + 1);
        }));
        // A multi-line value is a field's own.
        let mut text = field(1);
        text.kind = FieldKind::Text;
        text.value = "one\n  two".to_owned();
        text.options.clear();
        assert!(response(vec![text]).validate().is_ok());
    }

    #[test]
    fn an_undo_password_is_checked_as_an_unlock_one_is() {
        let undo = |password: Option<&str>| UndoArgs {
            doc: DocumentId(1),
            password: password.map(|password| Password::new(password.to_owned())),
        };
        assert!(undo(None).validate().is_ok());
        assert!(undo(Some("user")).validate().is_ok());
        assert!(undo(Some("")).validate().is_err());
        assert!(undo(Some("a\u{0}b")).validate().is_err());
        // The frontend's form: no password (null or left out), or the one typed.
        for value in [
            serde_json::json!({ "doc": 1, "password": null }),
            serde_json::json!({ "doc": 1 }),
        ] {
            assert_eq!(
                serde_json::from_value::<UndoArgs>(value).unwrap(),
                undo(None)
            );
        }
        assert_eq!(
            serde_json::from_value::<UndoArgs>(serde_json::json!({ "doc": 1, "password": "user" }))
                .unwrap(),
            undo(Some("user"))
        );
    }

    #[test]
    fn page_management_names_pages_once_and_places_within_bounds() {
        let edit = |edit: Edit| EditArgs {
            doc: DocumentId(1),
            edit,
        };
        let delete = |pages: Vec<u32>| edit(Edit::DeletePages { pages });
        assert!(delete(vec![2, 0]).validate().is_ok());
        assert!(delete(vec![]).validate().is_err());
        assert!(delete(vec![3, 3]).validate().is_err());
        assert!(delete((0..=MAX_PAGE_COUNT).collect()).validate().is_err());

        let move_ = |pages: Vec<u32>, before: u32| edit(Edit::MovePages { pages, before });
        assert!(move_(vec![4], 0).validate().is_ok());
        assert!(move_(vec![0, 1], MAX_PAGE_COUNT).validate().is_ok());
        assert!(move_(vec![], 0).validate().is_err());
        assert!(move_(vec![1, 1], 0).validate().is_err());
        assert!(move_(vec![1], MAX_PAGE_COUNT + 1).validate().is_err());

        let insert = |at: u32, like: u32| edit(Edit::InsertBlankPage { at, like });
        assert!(insert(0, 0).validate().is_ok());
        assert!(
            insert(MAX_PAGE_COUNT, MAX_PAGE_COUNT - 1)
                .validate()
                .is_ok()
        );
        assert!(insert(MAX_PAGE_COUNT + 1, 0).validate().is_err());
        assert!(insert(0, MAX_PAGE_COUNT).validate().is_err());

        // The frontend's form.
        assert_eq!(
            serde_json::from_value::<EditArgs>(serde_json::json!({
                "doc": 1,
                "edit": { "kind": "movePages", "pages": [4], "before": 0 }
            }))
            .unwrap(),
            move_(vec![4], 0)
        );
        assert_eq!(
            serde_json::from_value::<EditArgs>(serde_json::json!({
                "doc": 1,
                "edit": { "kind": "insertBlankPage", "at": 2, "like": 1 }
            }))
            .unwrap(),
            insert(2, 1)
        );
    }

    #[test]
    fn edited_pages_and_saved_files_are_bounded() {
        let edited = |pages: Vec<PageSize>| WorkerResponse::Edited {
            request: RequestId(1),
            pages,
        };
        assert!(edited(vec![page(792.0, 612.0)]).validate().is_ok());
        assert!(edited(Vec::new()).validate().is_err());
        assert!(edited(vec![page(f32::NAN, 612.0)]).validate().is_err());
        let saved = |bytes: u64| WorkerResponse::Saved {
            request: RequestId(1),
            bytes,
            incremental: false,
        };
        assert!(saved(1_234).validate().is_ok());
        assert!(saved(0).validate().is_err());
        assert!(saved(MAX_DOCUMENT_BYTES + 1).validate().is_err());
    }

    #[test]
    fn display_name_must_not_be_a_path() {
        let info = |display_name: &str| DocumentInfo {
            doc: DocumentId(1),
            display_name: display_name.to_owned(),
            pages: vec![page(612.0, 792.0)],
            has_outline: false,
            has_form: false,
            security: SecurityReport::default(),
            permissions: DocumentPermissions::ALL,
            unsaved: false,
            encrypted: false,
            can_undo: false,
            can_redo: false,
            recovery: Recovery::None,
        };
        assert!(info("報告.pdf").validate().is_ok());
        assert!(info(r"C:\Users\someone\報告.pdf").validate().is_err());
        assert!(info("docs/報告.pdf").validate().is_err());
    }

    #[test]
    fn open_events_carry_no_paths() {
        let opening = |display_name: &str| OpenEvent::Opening {
            tab: TabId(1),
            display_name: display_name.to_owned(),
        };
        assert!(opening("報告.pdf").validate().is_ok());
        assert!(opening(r"C:\Users\someone\報告.pdf").validate().is_err());
        let failed = OpenEvent::Failed {
            tab: TabId(1),
            display_name: r"\\server\share\報告.pdf".to_owned(),
            error: IpcError {
                code: ErrorCode::Unreadable,
                message: String::new(),
            },
        };
        assert!(failed.validate().is_err());
        assert!(OpenEvent::DragHover { active: true }.validate().is_ok());
    }

    #[test]
    fn page_index_is_checked_against_the_page_count() {
        assert!(check_page_index(0, 1).is_ok());
        assert!(check_page_index(1, 1).is_err());
    }

    #[test]
    fn ipc_error_message_is_bounded() {
        let error = IpcError {
            code: ErrorCode::Internal,
            message: "x".repeat(MAX_ERROR_MESSAGE_BYTES as usize + 1),
        };
        assert!(error.validate().is_err());
    }
}
