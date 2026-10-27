//! Bounds checks for untrusted values.
//!
//! The main process validates every [`WorkerResponse`] before using it, and every command
//! argument coming from the frontend. Page-index checks need the document's page count and are
//! done by the caller with [`check_page_index`].

use std::collections::HashSet;

use thiserror::Error;

use crate::limits::*;
use crate::ocr::is_language_name;
use crate::text::{
    classify_uri, is_clean_copy_text, is_clean_display_text, is_invisible_format, is_note_text,
    is_web_address,
};
use crate::types::{
    AnnotationSummary, AnnotationSummaryItem, AnnotationSummaryKind, AttachmentInfo,
    AttachmentSource, BatchArgs, BatchOperation, CompressArgs, DocumentInfo, Edit, EditArgs,
    EncryptArgs, ExportArgs, ExportFormat, FindPersonalDataQuery, FindingKind, FontImport,
    FontInfo, FormField, IndexSearchArgs, IndexSearchEvent, IndexSnippet, IpcError, LinkTarget,
    OcrLanguages, OcrProgress, OpenEvent, OutlineItem, OutlineResult, PageAnnotation, PageLink,
    PageMarkPages, PageMarkPosition, PageMarks, PageMarksCopyArgs, PageNumberFormat, PageSize,
    PageText, Password, PersonalDataMatch, PersonalDataMatches, Point, PrepareTextBoxArgs, Quad,
    RasterRegion, RecentFile, Rect, RedactionSummary, RemoveFontArgs, RemoveLanguageArgs,
    RenderPageArgs, Restrictions, Rotation, SanitizeArgs, SanitizeOptions, SanitizeReport,
    SanitizeResult, SaveAttachmentResult, SearchArgs, SearchHit, SecurityReport, SensitiveArgs,
    SensitiveFile, Settings, SignArgs, SignatureInfo, SignatureReport, SignatureStatus,
    SigningFileKey, SigningKey, TextBoxInfo, TextBoxStyle, TextLine, UndoArgs, UnlockArgs,
    UnlockSourceArgs, Watermark, WatermarkContent,
};
use crate::worker::{
    InspectedFont, OcrOutcome, OpenedDocument, Raster, TextBoxModel, WorkerAttachment, WorkerError,
    WorkerErrorCode, WorkerPageMarks, WorkerResponse,
};

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

/// Text from a note in the PDF, as `clean_note_text` makes it: lines of display text (or empty
/// ones).
fn check_cleaned_note(what: &'static str, text: &str, max: u32) -> Result<(), ValidationError> {
    check_text(what, text, max)?;
    if !is_note_text(text)
        || text
            .split('\n')
            .any(|line| !line.is_empty() && !is_clean_display_text(line))
    {
        return Err(ValidationError::Invalid {
            what,
            reason: "not cleaned",
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
        check_count(
            "remote addresses",
            self.remote_addresses.len(),
            MAX_REMOTE_ADDRESSES,
        )?;
        let mut addresses = HashSet::new();
        for address in &self.remote_addresses {
            check_text("remote address", address, MAX_URI_BYTES)?;
            // Only http and https may ever be offered for the browser (ADR 0022).
            if !is_web_address(address) {
                return Err(ValidationError::Invalid {
                    what: "remote address",
                    reason: "not an http or https URI",
                });
            }
            if !addresses.insert(address) {
                return Err(ValidationError::Invalid {
                    what: "remote addresses",
                    reason: "duplicate address",
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
        // Only a request for a password has a key to give (ADR 0017).
        if self.password_key.is_some() && self.code != WorkerErrorCode::Encrypted {
            return Err(ValidationError::Invalid {
                what: "password key",
                reason: "only a request for a password has one",
            });
        }
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
            WorkerResponse::Signatures { report, .. } => report.validate(),
            WorkerResponse::RedactionSummary { summary, .. } => summary.validate(),
            WorkerResponse::AnnotationSummary { summary, .. } => summary.validate(),
            WorkerResponse::PersonalDataMatches { matches, .. } => matches.validate(),
            WorkerResponse::TextBoxPrepared { model, .. } => model.validate(),
            WorkerResponse::FontInspected { faces, .. } => {
                if faces.is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "inspected faces",
                        reason: "no faces in font",
                    });
                }
                check_count("inspected faces", faces.len(), 1000)?;
                faces.iter().try_for_each(InspectedFont::validate)
            }
            WorkerResponse::TextBox { info, .. } => info.validate(),
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
            // The room for the signature is as large as the main process asked, and inside the
            // file, with something before it and after it: the rest of the dictionary.
            WorkerResponse::SignaturePrepared {
                bytes,
                contents_start,
                contents_end,
                ..
            } => {
                let room = 2 * u64::from(SIGNATURE_RESERVE_BYTES) + 2;
                if *bytes == 0
                    || *bytes > MAX_DOCUMENT_BYTES
                    || *contents_start == 0
                    || contents_end.checked_sub(*contents_start) != Some(room)
                    || *contents_end >= *bytes
                {
                    return Err(ValidationError::OutOfRange {
                        what: "room for the signature",
                    });
                }
                Ok(())
            }
            WorkerResponse::Compressed {
                bytes,
                before,
                images,
                changed,
                ..
            } => {
                for size in [bytes, before] {
                    if *size == 0 || *size > MAX_DOCUMENT_BYTES {
                        return Err(ValidationError::OutOfRange {
                            what: "compressed file size",
                        });
                    }
                }
                if *changed > *images || *images > MAX_IMAGES_PER_DOCUMENT {
                    return Err(ValidationError::OutOfRange {
                        what: "compressed picture count",
                    });
                }
                Ok(())
            }
            WorkerResponse::ExifCleaned {
                bytes,
                pictures,
                cleaned,
                ..
            } => {
                if *bytes == 0 || *bytes > MAX_DOCUMENT_BYTES {
                    return Err(ValidationError::OutOfRange {
                        what: "cleaned copy size",
                    });
                }
                if *cleaned > *pictures || *pictures > MAX_IMAGES_PER_DOCUMENT {
                    return Err(ValidationError::OutOfRange {
                        what: "cleaned picture count",
                    });
                }
                Ok(())
            }
            WorkerResponse::Sanitized { bytes, report, .. } => {
                if *bytes == 0 || *bytes > MAX_DOCUMENT_BYTES {
                    return Err(ValidationError::OutOfRange {
                        what: "sanitized file size",
                    });
                }
                report.validate()
            }
            WorkerResponse::Created { bytes, pages, .. } => {
                if *bytes == 0 || *bytes > MAX_DOCUMENT_BYTES {
                    return Err(ValidationError::OutOfRange {
                        what: "created file size",
                    });
                }
                if *pages == 0 || *pages > MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "created page count",
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
            WorkerResponse::Source {
                bytes,
                pages,
                security,
                ..
            } => {
                check_count("source file bytes", bytes.len(), MAX_SOURCE_BYTES as u32)?;
                if *pages == 0 || *pages > MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "source page count",
                    });
                }
                security.validate()
            }
            WorkerResponse::Attachments { attachments, .. } => {
                check_count("attachments", attachments.len(), MAX_ATTACHMENTS)?;
                let mut ids = HashSet::new();
                for attachment in attachments {
                    if !ids.insert(attachment.id) {
                        return Err(ValidationError::Invalid {
                            what: "attachments",
                            reason: "an attachment appears twice",
                        });
                    }
                    attachment.validate()?;
                }
                Ok(())
            }
            WorkerResponse::AttachmentWritten { bytes, .. } => {
                if *bytes > MAX_DOCUMENT_BYTES {
                    return Err(ValidationError::OutOfRange {
                        what: "attachment bytes written",
                    });
                }
                Ok(())
            }
            WorkerResponse::Error { error, .. } => error.validate(),
        }
    }
}

/// Longest time text of a signature: "2026-09-24 12:00:00 UTC+08:00" is 30 bytes.
const MAX_SIGNATURE_TIME_BYTES: u32 = 40;

impl Validate for SignatureInfo {
    fn validate(&self) -> Result<(), ValidationError> {
        let holds = matches!(
            self.status,
            SignatureStatus::Valid | SignatureStatus::ChangedAfterSigning
        );
        if self.reason.is_some() != (self.status == SignatureStatus::Unverifiable) {
            return Err(ValidationError::Invalid {
                what: "signature",
                reason: "has a reason only when it could not be verified",
            });
        }
        // Who signed and whether to trust them are only known of a signature that holds.
        if !holds && (self.signer.is_some() || self.signer_trusted) {
            return Err(ValidationError::Invalid {
                what: "signature",
                reason: "names a signer though it does not hold",
            });
        }
        for (what, text, max) in [
            (
                "signature field name",
                &self.field_name,
                MAX_SIGNATURE_TEXT_BYTES,
            ),
            ("signer", &self.signer, MAX_SIGNATURE_TEXT_BYTES),
            ("signing time", &self.claimed_time, MAX_SIGNATURE_TIME_BYTES),
        ] {
            if let Some(text) = text {
                check_text(what, text, max)?;
                check_clean(what, text)?;
            }
        }
        Ok(())
    }
}

impl Validate for SignatureReport {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count("signatures", self.signatures.len(), MAX_SIGNATURES)?;
        self.signatures.iter().try_for_each(SignatureInfo::validate)
    }
}

impl Validate for RenderPageArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_scale(self.scale)?;
        self.region.as_ref().map_or(Ok(()), Validate::validate)
    }
}

/// Within the raster limits, and within the largest page raster a region is drawn from. Whether
/// it lies within its page is `raster::check_region`.
impl Validate for RasterRegion {
    fn validate(&self) -> Result<(), ValidationError> {
        check_raster_size(self.width, self.height)?;
        let right = u64::from(self.x) + u64::from(self.width);
        let bottom = u64::from(self.y) + u64::from(self.height);
        if right.max(bottom) > u64::from(MAX_REGION_PAGE_SIDE_PX) {
            return Err(ValidationError::OutOfRange {
                what: "raster region",
            });
        }
        Ok(())
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

/// A password a copy is encrypted with (B2-15): not empty, no NUL, at most 127 bytes.
fn check_new_password(password: &Password) -> Result<(), ValidationError> {
    check_password(password)?;
    check_text("password", password.as_str(), MAX_NEW_PASSWORD_BYTES)
}

impl Validate for EncryptArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_encryption(
            self.open_password.as_ref(),
            self.permissions_password.as_ref(),
            self.restrictions,
        )
    }
}

impl Validate for Watermark {
    fn validate(&self) -> Result<(), ValidationError> {
        if !(MIN_WATERMARK_OPACITY..=100).contains(&self.opacity) {
            return Err(ValidationError::OutOfRange {
                what: "watermark opacity",
            });
        }
        match &self.content {
            WatermarkContent::Text { text, size, .. } => {
                check_coordinate("watermark size", *size)?;
                if !(MIN_WATERMARK_SIZE_PT..=MAX_WATERMARK_SIZE_PT).contains(size) {
                    return Err(ValidationError::OutOfRange {
                        what: "watermark size",
                    });
                }
                let chars = text.chars().count();
                if text.trim().is_empty() || chars > MAX_WATERMARK_CHARS {
                    return Err(ValidationError::Invalid {
                        what: "watermark words",
                        reason: "empty or too long",
                    });
                }
                // One line of plain words: no line breaks, controls or invisible formatting.
                if !is_clean_display_text(text) || text.contains(['\n', '\r']) {
                    return Err(ValidationError::Invalid {
                        what: "watermark words",
                        reason: "not one line of plain words",
                    });
                }
            }
            WatermarkContent::Picture { width_pt } => {
                check_coordinate("watermark width", *width_pt)?;
                if !(MIN_WATERMARK_WIDTH_PT..=MAX_WATERMARK_WIDTH_PT).contains(width_pt) {
                    return Err(ValidationError::OutOfRange {
                        what: "watermark width",
                    });
                }
            }
        }
        Ok(())
    }
}

impl Validate for PageMarks {
    fn validate(&self) -> Result<(), ValidationError> {
        check_coordinate("page marks font size", self.font_size)?;
        if !(MIN_PAGE_MARK_SIZE_PT..=MAX_PAGE_MARK_SIZE_PT).contains(&self.font_size) {
            return Err(ValidationError::OutOfRange {
                what: "page marks font size",
            });
        }
        check_coordinate("page marks margin", self.margin_pt)?;
        if !(MIN_PAGE_MARK_MARGIN_PT..=MAX_PAGE_MARK_MARGIN_PT).contains(&self.margin_pt) {
            return Err(ValidationError::OutOfRange {
                what: "page marks margin",
            });
        }

        match &self.pages {
            PageMarkPages::All | PageMarkPages::AllButFirst => {}
            PageMarkPages::Custom { pages } => {
                if pages.is_empty() || pages.len() > MAX_PAGE_MARK_PAGES {
                    return Err(ValidationError::Invalid {
                        what: "page marks pages",
                        reason: "empty or too many pages",
                    });
                }
                if !pages.windows(2).all(|w| w[0] < w[1]) {
                    return Err(ValidationError::Invalid {
                        what: "page marks pages",
                        reason: "pages not strictly ascending",
                    });
                }
            }
        }

        let mut positions = HashSet::new();
        let mut has_any_mark = false;

        if let Some(pn) = &self.page_number {
            has_any_mark = true;
            if !positions.insert(pn.position) {
                return Err(ValidationError::Invalid {
                    what: "page marks position",
                    reason: "duplicate mark position",
                });
            }
            if let PageNumberFormat::Custom { prefix, suffix } = &pn.format {
                let prefix_chars = prefix.chars().count();
                let suffix_chars = suffix.chars().count();
                if prefix_chars > MAX_PAGE_MARK_CHARS || suffix_chars > MAX_PAGE_MARK_CHARS {
                    return Err(ValidationError::Invalid {
                        what: "page number custom format",
                        reason: "too long",
                    });
                }
                let bad = |s: &str| s.chars().any(|c| c.is_control() || is_invisible_format(c));
                if bad(prefix) || bad(suffix) {
                    return Err(ValidationError::Invalid {
                        what: "page number custom format",
                        reason: "not plain text",
                    });
                }
            }
        }

        if let Some(bates) = &self.bates {
            has_any_mark = true;
            if !positions.insert(bates.position) {
                return Err(ValidationError::Invalid {
                    what: "page marks position",
                    reason: "duplicate mark position",
                });
            }
            if !(MIN_BATES_DIGITS..=MAX_BATES_DIGITS).contains(&bates.digits) {
                return Err(ValidationError::OutOfRange {
                    what: "bates digits",
                });
            }
            let prefix_chars = bates.prefix.chars().count();
            if prefix_chars > MAX_BATES_PREFIX_CHARS {
                return Err(ValidationError::Invalid {
                    what: "bates prefix",
                    reason: "too long",
                });
            }
            // Spaces are kept as typed ("ABC 000001"); what is not seen is not.
            if bates
                .prefix
                .chars()
                .any(|c| c.is_control() || is_invisible_format(c))
            {
                return Err(ValidationError::Invalid {
                    what: "bates prefix",
                    reason: "not plain text",
                });
            }
            let max_val = 10u64.pow(bates.digits as u32);
            if bates.start >= max_val {
                return Err(ValidationError::OutOfRange {
                    what: "bates start number",
                });
            }
            if let PageMarkPages::Custom { pages } = &self.pages {
                let count = pages.len() as u64;
                if count > 0
                    && bates
                        .start
                        .checked_add(count - 1)
                        .is_none_or(|end| end >= max_val)
                {
                    return Err(ValidationError::OutOfRange {
                        what: "bates number overflow",
                    });
                }
            }
        }

        if let Some(hf) = &self.header_footer {
            let slots = [
                (&hf.header_left, PageMarkPosition::TopLeft),
                (&hf.header_center, PageMarkPosition::TopCenter),
                (&hf.header_right, PageMarkPosition::TopRight),
                (&hf.footer_left, PageMarkPosition::BottomLeft),
                (&hf.footer_center, PageMarkPosition::BottomCenter),
                (&hf.footer_right, PageMarkPosition::BottomRight),
            ];
            for (slot, pos) in slots {
                if let Some(text) = slot {
                    let chars = text.chars().count();
                    if text.trim().is_empty() || chars > MAX_PAGE_MARK_CHARS {
                        return Err(ValidationError::Invalid {
                            what: "header or footer text",
                            reason: "empty or too long",
                        });
                    }
                    if !is_clean_display_text(text) || text.contains(['\n', '\r']) {
                        return Err(ValidationError::Invalid {
                            what: "header or footer text",
                            reason: "not one line of plain words",
                        });
                    }
                    has_any_mark = true;
                    if !positions.insert(pos) {
                        return Err(ValidationError::Invalid {
                            what: "page marks position",
                            reason: "duplicate mark position",
                        });
                    }
                }
            }
        }

        if !has_any_mark {
            return Err(ValidationError::Invalid {
                what: "page marks",
                reason: "no marks specified",
            });
        }

        Ok(())
    }
}

impl Validate for WorkerPageMarks {
    fn validate(&self) -> Result<(), ValidationError> {
        PageMarks::from(self.clone()).validate()
    }
}

impl Validate for PageMarksCopyArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        self.marks.validate()
    }
}

impl Validate for BatchArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        match &self.operation {
            BatchOperation::RemoveMetadata | BatchOperation::RemoveExif => Ok(()),
            BatchOperation::Encrypt {
                open_password,
                permissions_password,
                restrictions,
            } => check_encryption(
                open_password.as_ref(),
                permissions_password.as_ref(),
                *restrictions,
            ),
            BatchOperation::Watermark { watermark } => watermark.validate(),
            BatchOperation::CompressImages { max_side, .. } => check_compress_side(*max_side),
            BatchOperation::Sanitize { options } => options.validate(),
            BatchOperation::PageMarks { marks } => marks.validate(),
        }
    }
}

/// The rules of an encrypted copy (B2-15), one file or a batch of them (B3-04).
fn check_encryption(
    open_password: Option<&Password>,
    permissions_password: Option<&Password>,
    restrictions: Restrictions,
) -> Result<(), ValidationError> {
    for password in [open_password, permissions_password].into_iter().flatten() {
        check_new_password(password)?;
    }
    if open_password.is_none() && !restrictions.any() {
        return Err(ValidationError::Invalid {
            what: "encryption",
            reason: "nothing is asked for: no open password and no restriction",
        });
    }
    if restrictions.any() && permissions_password.is_none() {
        return Err(ValidationError::Invalid {
            what: "encryption",
            reason: "a restriction needs a permissions password to lift it",
        });
    }
    // Whoever knows the open password would have the permissions password's rights too.
    if open_password.is_some() && open_password == permissions_password {
        return Err(ValidationError::Invalid {
            what: "encryption",
            reason: "the permissions password is the open password",
        });
    }
    Ok(())
}

impl Validate for CompressArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_compress_side(self.max_side)
    }
}

impl Validate for SanitizeOptions {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl Validate for SanitizeReport {
    fn validate(&self) -> Result<(), ValidationError> {
        for (count, what) in [
            (self.actions, "removed actions"),
            (self.links, "removed links"),
            (self.attachments, "removed attachments"),
            (self.form_scripts, "removed form scripts"),
            (self.xfa, "removed xfa"),
            (self.rich_media, "removed rich media"),
            (self.hidden_data, "removed hidden data"),
            (self.annotations, "removed annotations"),
        ] {
            if count > MAX_SANITIZED_ITEMS {
                return Err(ValidationError::OutOfRange { what });
            }
        }
        Ok(())
    }
}

impl Validate for SanitizeArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        self.options.validate()
    }
}

impl Validate for SanitizeResult {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.bytes == 0 || self.bytes > MAX_DOCUMENT_BYTES {
            return Err(ValidationError::OutOfRange {
                what: "sanitized file size",
            });
        }
        self.report.validate()
    }
}

/// How long a picture's longer side may stay (B3-01), for one document or a batch (B3-19).
fn check_compress_side(max_side: Option<u32>) -> Result<(), ValidationError> {
    if let Some(side) = max_side
        && !(MIN_COMPRESS_SIDE_PX..=MAX_COMPRESS_SIDE_PX).contains(&side)
    {
        return Err(ValidationError::OutOfRange {
            what: "picture side limit",
        });
    }
    Ok(())
}

impl Validate for UnlockSourceArgs {
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

impl Validate for AnnotationSummary {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count(
            "annotation summary items",
            self.items.len(),
            MAX_ANNOTATION_SUMMARY_ITEMS,
        )?;
        let mut bytes = 0usize;
        for item in &self.items {
            item.validate()?;
            bytes += item.text.len();
        }
        check_count(
            "annotation summary text bytes",
            bytes,
            MAX_ANNOTATION_SUMMARY_TOTAL_BYTES,
        )
    }
}

impl Validate for AnnotationSummaryItem {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.page_index >= MAX_PAGE_COUNT {
            return Err(ValidationError::OutOfRange {
                what: "annotation summary page",
            });
        }
        self.rect.validate()?;
        let max = MAX_ANNOTATION_SUMMARY_TEXT_BYTES;
        match self.kind {
            AnnotationSummaryKind::Highlight => {
                check_text("highlighted text", &self.text, max)?;
                check_clean("highlighted text", &self.text)
            }
            AnnotationSummaryKind::Note | AnnotationSummaryKind::TextBox => {
                check_cleaned_note("annotation summary note", &self.text, max)
            }
            AnnotationSummaryKind::Ink | AnnotationSummaryKind::Stamp if self.text.is_empty() => {
                Ok(())
            }
            AnnotationSummaryKind::Ink | AnnotationSummaryKind::Stamp => {
                Err(ValidationError::Invalid {
                    what: "drawing or stamp in a summary",
                    reason: "has text",
                })
            }
        }
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
            Edit::InsertPages { at, .. } => {
                if *at > MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "where to insert",
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
            Edit::AddTextBox {
                page, rect, style, ..
            } => {
                check_annotated_page(*page)?;
                check_annotation_rect(rect)?;
                style.validate()
            }
            Edit::SetTextBox { page, style, .. } => {
                check_annotated_page(*page)?;
                style.validate()
            }
            Edit::AddStamp { page, rect, .. }
            | Edit::AddImageStamp { page, rect, .. }
            | Edit::SetAnnotationRect { page, rect, .. } => {
                check_annotated_page(*page)?;
                check_annotation_rect(rect)
            }
            Edit::SetPictureAngle { page, degrees, .. } => {
                check_annotated_page(*page)?;
                check_angle(*degrees)
            }
            Edit::AddRedaction { page, rect } => {
                check_annotated_page(*page)?;
                check_annotation_rect(rect)
            }
            Edit::ApplyRedactions => Ok(()),
            Edit::AddBookmark {
                parent: _,
                index,
                title,
                page,
            } => {
                if *index > MAX_OUTLINE_ITEMS {
                    return Err(ValidationError::OutOfRange {
                        what: "bookmark index",
                    });
                }
                if *page >= MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange {
                        what: "bookmark page",
                    });
                }
                check_text("bookmark title", title, MAX_TEXT_BYTES)?;
                check_clean("bookmark title", title)?;
                if title.trim().is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "bookmark title",
                        reason: "is blank",
                    });
                }
                Ok(())
            }
            Edit::RenameBookmark { id: _, title } => {
                check_text("bookmark title", title, MAX_TEXT_BYTES)?;
                check_clean("bookmark title", title)?;
                if title.trim().is_empty() {
                    return Err(ValidationError::Invalid {
                        what: "bookmark title",
                        reason: "is blank",
                    });
                }
                Ok(())
            }
            Edit::DeleteBookmark { id: _ } => Ok(()),
            Edit::MoveBookmark { id, parent, index } => {
                if *index > MAX_OUTLINE_ITEMS {
                    return Err(ValidationError::OutOfRange {
                        what: "bookmark index",
                    });
                }
                if *parent == Some(*id) {
                    return Err(ValidationError::Invalid {
                        what: "bookmark move",
                        reason: "cannot be parent of itself",
                    });
                }
                Ok(())
            }
        }
    }
}

/// What applying the redaction marks would do (B2-19): no more marks than a document's pages can
/// list, nor more pages than a document has, nor more of the others than the marks' pages list.
impl Validate for RedactionSummary {
    fn validate(&self) -> Result<(), ValidationError> {
        let per_page = u64::from(MAX_ANNOTATIONS_PER_PAGE) * u64::from(MAX_PAGE_COUNT);
        if self.pages > MAX_PAGE_COUNT
            || self.pages > self.marks
            || u64::from(self.marks) > per_page
            || u64::from(self.annotations) > per_page
            || u64::from(self.fields) > u64::from(MAX_FIELDS_PER_PAGE) * u64::from(MAX_PAGE_COUNT)
        {
            return Err(ValidationError::OutOfRange {
                what: "redaction summary",
            });
        }
        Ok(())
    }
}

impl Validate for FindPersonalDataQuery {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count("personal data categories", self.categories.len(), 10)?;
        // The keyword is looked for only under its own category: one without the other would
        // quietly find nothing.
        let wants_keyword = self
            .categories
            .contains(&crate::types::PersonalDataCategory::Keyword);
        if wants_keyword != self.keyword.is_some() {
            return Err(ValidationError::Invalid {
                what: "personal data keyword",
                reason: "keyword and its category must come together",
            });
        }
        if let Some(ref keyword) = self.keyword {
            if keyword.trim().is_empty() {
                return Err(ValidationError::Invalid {
                    what: "personal data keyword",
                    reason: "empty",
                });
            }
            check_text(
                "personal data keyword",
                keyword,
                MAX_PERSONAL_DATA_KEYWORD_BYTES as u32,
            )?;
        }
        Ok(())
    }
}

impl Validate for PersonalDataMatch {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.page >= MAX_PAGE_COUNT {
            return Err(ValidationError::OutOfRange {
                what: "personal data match page",
            });
        }
        if self.rects.is_empty() {
            return Err(ValidationError::Invalid {
                what: "personal data match",
                reason: "no rects",
            });
        }
        check_count(
            "personal data match rects",
            self.rects.len(),
            MAX_QUADS_PER_HIT,
        )?;
        self.rects.iter().try_for_each(Rect::validate)?;
        check_text(
            "personal data match preview",
            &self.preview,
            MAX_PERSONAL_DATA_PREVIEW_BYTES as u32,
        )?;
        Ok(())
    }
}

impl Validate for PersonalDataMatches {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count(
            "personal data matches",
            self.matches.len(),
            MAX_PERSONAL_DATA_MATCHES,
        )?;
        self.matches
            .iter()
            .try_for_each(PersonalDataMatch::validate)
    }
}

/// An angle a picture is turned to (B3-01), in degrees: a number, within a full turn each way.
fn check_angle(degrees: f32) -> Result<(), ValidationError> {
    if !degrees.is_finite() || degrees.abs() > MAX_PICTURE_ANGLE_DEGREES {
        return Err(ValidationError::OutOfRange {
            what: "picture angle",
        });
    }
    Ok(())
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

/// What a text box says (B2-28): something to see, within the limit, and only text (lines are
/// broken at `\n`). Whether the font has a glyph for each character is for the worker to say.
pub fn check_text_box_text(text: &str) -> Result<(), ValidationError> {
    if text.trim().is_empty() {
        return Err(ValidationError::Invalid {
            what: "text box text",
            reason: "empty",
        });
    }
    if text.len() > MAX_TEXT_BOX_BYTES {
        return Err(ValidationError::TooLong {
            what: "text box text",
            len: text.len(),
            max: MAX_TEXT_BOX_BYTES,
        });
    }
    if !is_note_text(text) {
        return Err(ValidationError::Invalid {
            what: "text box text",
            reason: "contains control or invisible formatting characters",
        });
    }
    Ok(())
}

impl Validate for TextBoxInfo {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text_box_text(&self.text)?;
        check_text(
            "text box font name",
            &self.font,
            MAX_MODEL_FONT_NAME_BYTES as u32,
        )?;
        check_clean("text box font name", &self.font)?;
        self.style.validate()
    }
}

impl Validate for PrepareTextBoxArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text_box_text(&self.text)
    }
}

impl Validate for RemoveFontArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl Validate for FontInfo {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() || self.name.len() >= MAX_FONT_NAME_BYTES {
            return Err(ValidationError::Invalid {
                what: "font name",
                reason: "is empty or too long",
            });
        }
        if self.family.is_empty() || self.family.len() >= MAX_FONT_NAME_BYTES {
            return Err(ValidationError::Invalid {
                what: "font family",
                reason: "is empty or too long",
            });
        }
        Ok(())
    }
}

impl Validate for FontImport {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            FontImport::Imported { fonts } => {
                check_count("imported fonts", fonts.len(), MAX_FONTS as u32)?;
                fonts.iter().try_for_each(FontInfo::validate)
            }
            FontImport::Cancelled | FontImport::Refused { .. } => Ok(()),
        }
    }
}

impl Validate for InspectedFont {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() || self.name.len() >= MAX_FONT_NAME_BYTES {
            return Err(ValidationError::Invalid {
                what: "font name",
                reason: "is empty or too long",
            });
        }
        if self.family.is_empty() || self.family.len() >= MAX_FONT_NAME_BYTES {
            return Err(ValidationError::Invalid {
                what: "font family",
                reason: "is empty or too long",
            });
        }
        if self.face > 1000 {
            return Err(ValidationError::OutOfRange {
                what: "font face index",
            });
        }
        Ok(())
    }
}

impl Validate for TextBoxStyle {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.size.is_finite() {
            return Err(ValidationError::NotFinite { what: "text size" });
        }
        if !(MIN_TEXT_BOX_SIZE..=MAX_TEXT_BOX_SIZE).contains(&self.size) {
            return Err(ValidationError::OutOfRange { what: "text size" });
        }
        Ok(())
    }
}

/// Longest name of the font of a text box, as the font list's names are (B2-27).
const MAX_MODEL_FONT_NAME_BYTES: usize = 128;

/// The name of a subset font in a file: six capital letters, a plus and the font's name.
fn is_subset_name(base: &str) -> bool {
    let bytes = base.as_bytes();
    (8..=47).contains(&bytes.len())
        && bytes[..6].iter().all(u8::is_ascii_uppercase)
        && bytes[6] == b'+'
        && bytes[7..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

/// A text box model from the worker, from the journal, or from the page's own request (B2-28):
/// everything that is a number or a length is bounded, and every character of the words has a
/// glyph. The font file inside is not looked into here: the worker cuts it again before it uses it.
impl Validate for TextBoxModel {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text_box_text(&self.text)?;
        check_text(
            "text box font name",
            &self.font,
            MAX_MODEL_FONT_NAME_BYTES as u32,
        )?;
        check_clean("text box font name", &self.font)?;
        if !is_subset_name(&self.base) {
            return Err(ValidationError::Invalid {
                what: "text box font",
                reason: "is not named as a subset",
            });
        }
        for value in [
            self.ascent,
            self.descent,
            self.cap_height,
            self.bbox[0],
            self.bbox[1],
            self.bbox[2],
            self.bbox[3],
        ] {
            if value.abs() > 20_000 {
                return Err(ValidationError::OutOfRange {
                    what: "text box font metrics",
                });
            }
        }
        if !(1_000..=2_000).contains(&self.step) {
            return Err(ValidationError::OutOfRange {
                what: "text box line step",
            });
        }
        check_count(
            "text box glyphs",
            self.glyphs.len(),
            MAX_TEXT_BOX_GLYPHS as u32,
        )?;
        let mut chars = HashSet::new();
        let mut advances = std::collections::HashMap::new();
        for glyph in &self.glyphs {
            if glyph.cid == 0 || glyph.advance > 5_000 {
                return Err(ValidationError::OutOfRange {
                    what: "text box glyph",
                });
            }
            if glyph.ch == '\n' || !is_note_text(&glyph.ch.to_string()) || !chars.insert(glyph.ch) {
                return Err(ValidationError::Invalid {
                    what: "text box glyph",
                    reason: "is a control character, or is there twice",
                });
            }
            // One glyph, one width.
            if *advances.entry(glyph.cid).or_insert(glyph.advance) != glyph.advance {
                return Err(ValidationError::Invalid {
                    what: "text box glyph",
                    reason: "has two widths",
                });
            }
        }
        if !self.text.chars().all(|c| c == '\n' || chars.contains(&c)) {
            return Err(ValidationError::Invalid {
                what: "text box glyphs",
                reason: "miss a character of the text",
            });
        }
        if self.subset.len() < 12 || self.subset.len() > MAX_TEXT_BOX_SUBSET_BYTES {
            return Err(ValidationError::OutOfRange {
                what: "text box font subset",
            });
        }
        if self.subset[..4] != [0, 1, 0, 0] {
            return Err(ValidationError::Invalid {
                what: "text box font subset",
                reason: "is not TrueType",
            });
        }
        Ok(())
    }
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
        if let Some(picture) = &self.picture {
            check_angle(picture.angle)?;
            for side in [picture.width, picture.height] {
                if !side.is_finite() || side <= 0.0 || side > MAX_PAGE_SIDE_PT {
                    return Err(ValidationError::OutOfRange {
                        what: "picture size",
                    });
                }
            }
        }
        if let Some(text) = &self.text {
            check_cleaned_note("note text", text, MAX_NOTE_TEXT_BYTES)?;
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

impl Validate for SensitiveFile {
    fn validate(&self) -> Result<(), ValidationError> {
        check_display_name(&self.display_name)
    }
}

/// The list of files marked as sensitive on this computer that the frontend gets (B3-05).
impl Validate for [SensitiveFile] {
    fn validate(&self) -> Result<(), ValidationError> {
        check_count("sensitive files", self.len(), MAX_SENSITIVE_FILES)?;
        self.iter().try_for_each(SensitiveFile::validate)
    }
}

impl Validate for SensitiveArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        self.password.as_ref().map_or(Ok(()), check_password)
    }
}

/// The farthest a time zone is from UTC, in minutes (UTC+14:00; the farthest west is -12:00).
const MAX_UTC_OFFSET_MINUTES: i16 = 14 * 60;

impl Validate for SignArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        if let SigningKey::File(SigningFileKey { password, .. }) = &self.key {
            check_password(password)?;
        }
        if let Some(reason) = &self.reason {
            check_signing_text("signing reason", reason, MAX_SIGNING_REASON_BYTES)?;
        }
        if !(-MAX_UTC_OFFSET_MINUTES..=MAX_UTC_OFFSET_MINUTES).contains(&self.utc_offset_minutes) {
            return Err(ValidationError::OutOfRange { what: "time zone" });
        }
        Ok(())
    }
}

/// Words a signature gives (B3-06): its reason, or its signer as the certificate names them. One
/// line of plain words: some, bounded, without line breaks, controls or invisible formatting.
pub fn check_signing_text(what: &'static str, text: &str, max: u32) -> Result<(), ValidationError> {
    check_text(what, text, max)?;
    if text.trim().is_empty() || !is_clean_display_text(text) || text.contains(['\n', '\r']) {
        return Err(ValidationError::Invalid {
            what,
            reason: "not one line of plain words",
        });
    }
    Ok(())
}

/// Text the page copies from a sensitive document (B3-05), which the main process puts on the
/// clipboard: some, bounded, and without NUL, where the clipboard's text would end.
pub fn check_clipboard_text(text: &str) -> Result<(), ValidationError> {
    if text.is_empty() {
        return Err(ValidationError::Invalid {
            what: "clipboard text",
            reason: "empty",
        });
    }
    check_text("clipboard text", text, MAX_CLIPBOARD_TEXT_BYTES)?;
    if text.contains('\u{0}') {
        return Err(ValidationError::Invalid {
            what: "clipboard text",
            reason: "contains a NUL character",
        });
    }
    Ok(())
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

impl Validate for Settings {
    fn validate(&self) -> Result<(), ValidationError> {
        match &self.ocr_language {
            Some(code) if !is_language_name(code) => Err(ValidationError::Invalid {
                what: "OCR language",
                reason: "not the code of a language",
            }),
            _ => Ok(()),
        }
    }
}

impl Validate for RemoveLanguageArgs {
    fn validate(&self) -> Result<(), ValidationError> {
        if !is_language_name(&self.code) {
            return Err(ValidationError::Invalid {
                what: "OCR language",
                reason: "not the code of a language",
            });
        }
        Ok(())
    }
}

impl Validate for OcrLanguages {
    fn validate(&self) -> Result<(), ValidationError> {
        // Those that came with the app are few; the user's are bounded.
        check_count(
            "OCR languages",
            self.languages.len(),
            MAX_IMPORTED_LANGUAGES + 16,
        )?;
        let mut seen = HashSet::new();
        for language in &self.languages {
            if !is_language_name(&language.code) || !seen.insert(language.code.as_str()) {
                return Err(ValidationError::Invalid {
                    what: "OCR languages",
                    reason: "a code that is not one, or that appears twice",
                });
            }
            if language.bytes > MAX_LANGUAGE_DATA_BYTES as u64 {
                return Err(ValidationError::OutOfRange {
                    what: "language data size",
                });
            }
        }
        if let Some(automatic) = &self.automatic
            && !seen.contains(automatic.as_str())
        {
            return Err(ValidationError::Invalid {
                what: "OCR languages",
                reason: "the automatic language is not installed",
            });
        }
        Ok(())
    }
}

impl Validate for OcrProgress {
    fn validate(&self) -> Result<(), ValidationError> {
        let consistent = self.pages <= MAX_PAGE_COUNT
            && self.checked <= self.pages
            && self.scans <= self.checked
            && u64::from(self.recognised) + u64::from(self.failed) <= u64::from(self.scans);
        if !consistent {
            return Err(ValidationError::Invalid {
                what: "OCR progress",
                reason: "the counts do not add up",
            });
        }
        Ok(())
    }
}

impl Validate for OpenEvent {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            OpenEvent::DragHover { .. }
            | OpenEvent::TabLimit { .. }
            | OpenEvent::CaptureProtection { .. } => Ok(()),
            OpenEvent::Ocr { progress, .. } => progress.validate(),
            OpenEvent::OcrPage { page_index, .. } => {
                if *page_index >= MAX_PAGE_COUNT {
                    return Err(ValidationError::OutOfRange { what: "page index" });
                }
                Ok(())
            }
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

impl Validate for WorkerAttachment {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text("attachment name", &self.name, MAX_ATTACHMENT_NAME_BYTES)?;
        if let Some(desc) = &self.description {
            check_text("attachment description", desc, MAX_ATTACHMENT_DESC_BYTES)?;
        }
        if let AttachmentSource::Page { page } = self.source
            && page >= MAX_PAGE_COUNT
        {
            return Err(ValidationError::OutOfRange { what: "page index" });
        }
        Ok(())
    }
}

impl Validate for AttachmentInfo {
    fn validate(&self) -> Result<(), ValidationError> {
        check_text("attachment name", &self.name, MAX_ATTACHMENT_NAME_BYTES)?;
        check_text(
            "attachment extension",
            &self.extension,
            MAX_ATTACHMENT_NAME_BYTES,
        )?;
        if let Some(desc) = &self.description {
            check_text("attachment description", desc, MAX_ATTACHMENT_DESC_BYTES)?;
        }
        if let AttachmentSource::Page { page } = self.source
            && page >= MAX_PAGE_COUNT
        {
            return Err(ValidationError::OutOfRange { what: "page index" });
        }
        Ok(())
    }
}

impl Validate for SaveAttachmentResult {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.bytes > MAX_DOCUMENT_BYTES {
            return Err(ValidationError::OutOfRange {
                what: "attachment bytes",
            });
        }
        Ok(())
    }
}

/// A search of the text index (B3-21) takes the same queries as a search of one document.
impl Validate for IndexSearchArgs {
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

/// A piece of a snippet: text as it is copied, its spaces collapsed, of at most `max` characters.
fn check_snippet_text(what: &'static str, text: &str, max: u32) -> Result<(), ValidationError> {
    let len = text.chars().count();
    if len > max as usize {
        return Err(ValidationError::TooLong {
            what,
            len,
            max: max as usize,
        });
    }
    if !is_clean_copy_text(text) || text.contains("  ") {
        return Err(ValidationError::Invalid {
            what,
            reason: "not plain text",
        });
    }
    Ok(())
}

impl Validate for IndexSnippet {
    fn validate(&self) -> Result<(), ValidationError> {
        check_snippet_text("text before a hit", &self.before, MAX_INDEX_SNIPPET_CHARS)?;
        check_snippet_text("text after a hit", &self.after, MAX_INDEX_SNIPPET_CHARS)?;
        // As many characters as the query, at most, which has no more characters than bytes.
        check_snippet_text("words found", &self.matched, MAX_QUERY_BYTES)?;
        if self.matched.is_empty() {
            return Err(ValidationError::Invalid {
                what: "words found",
                reason: "empty",
            });
        }
        Ok(())
    }
}

/// What a search of the text index sends the frontend (B3-21): a file name and no path, a bounded
/// list of pages in order, and snippets of plain text.
impl Validate for IndexSearchEvent {
    fn validate(&self) -> Result<(), ValidationError> {
        let IndexSearchEvent::Found {
            display_name,
            total_hits,
            pages,
            pages_with_hits,
            ..
        } = self
        else {
            return Ok(());
        };
        check_display_name(display_name)?;
        check_count("pages with hits", pages.len(), MAX_INDEX_PAGES_PER_DOCUMENT)?;
        if *total_hits > MAX_SEARCH_HITS {
            return Err(ValidationError::OutOfRange { what: "hits" });
        }
        if pages.is_empty() || (*pages_with_hits as usize) < pages.len() {
            return Err(ValidationError::Invalid {
                what: "pages with hits",
                reason: "not as many as listed",
            });
        }
        if !pages
            .windows(2)
            .all(|pair| pair[0].page_index < pair[1].page_index)
        {
            return Err(ValidationError::Invalid {
                what: "pages with hits",
                reason: "not in page order",
            });
        }
        let mut listed: u64 = 0;
        for page in pages {
            if page.hits == 0 {
                return Err(ValidationError::Invalid {
                    what: "page with hits",
                    reason: "has none",
                });
            }
            check_page_index(page.page_index, MAX_PAGE_COUNT)?;
            page.snippet.validate()?;
            listed += u64::from(page.hits);
        }
        if listed > u64::from(*total_hits) {
            return Err(ValidationError::Invalid {
                what: "hits",
                reason: "more on the pages than in the document",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AnnotationId, AnnotationKind, BatesMark, BlockedAction, BookmarkId, DocumentId,
        DocumentPermissions, ErrorCode, FieldId, FieldKind, FieldOption, FontId, HeaderFooterMarks,
        HighlightColor, HighlightMark, ImageQuality, IndexEntryId, IndexPageHits, InkColor,
        InkWidth, LinkId, PageNumberMark, PictureView, RecentId, Recovery, RequestId, Restrictions,
        Rgb, Rotation, SecurityFinding, StampImageId, StampName, TabId, TextAlign, TextBoxInfo,
        TextBoxModelId, UnverifiableReason, WatermarkPages, WatermarkPlace,
    };
    use crate::worker::{ModelGlyph, WorkerErrorCode};

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
            id: BookmarkId(1),
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
            signed: false,
            identifier: None,
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
            remote_addresses: Vec::new(),
        };
        assert!(report.validate().is_err());
    }

    #[test]
    fn remote_addresses_are_http_or_https_each_once_and_not_too_many() {
        let report = |addresses: &[&str]| SecurityReport {
            findings: vec![SecurityFinding {
                kind: FindingKind::RemoteFileSpec,
                count: 1,
            }],
            scan_complete: true,
            remote_addresses: addresses
                .iter()
                .map(|address| (*address).to_owned())
                .collect(),
        };
        assert!(
            report(&["https://img.example.invalid/a.png", "http://example.com/"])
                .validate()
                .is_ok()
        );
        for bad in [
            "mailto:someone@example.com",
            "file://server/share/x.png",
            "\\\\server\\share\\x.png",
            "",
        ] {
            assert!(report(&[bad]).validate().is_err(), "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URI_BYTES as usize));
        assert!(report(&[&long]).validate().is_err());
        assert!(
            report(&["https://example.com/", "https://example.com/"])
                .validate()
                .is_err()
        );
        let many: Vec<String> = (0..=MAX_REMOTE_ADDRESSES)
            .map(|index| format!("https://example.com/{index}"))
            .collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(
            report(&many[..MAX_REMOTE_ADDRESSES as usize])
                .validate()
                .is_ok()
        );
        assert!(report(&many).validate().is_err());
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
    fn ocr_settings_languages_and_progress_are_checked() {
        use crate::types::{DocumentId, OcrLanguage, OcrRun};
        let settings = |language: Option<&str>| Settings {
            ocr_language: language.map(str::to_owned),
            ..Settings::default()
        };
        assert_eq!(settings(None).validate(), Ok(()));
        assert_eq!(settings(Some("chi_tra")).validate(), Ok(()));
        for bad in ["", "../eng", "eng.traineddata", "1x", "a b"] {
            assert!(settings(Some(bad)).validate().is_err(), "{bad:?}");
        }
        assert!(RemoveLanguageArgs { code: "eng".into() }.validate().is_ok());
        assert!(
            RemoveLanguageArgs { code: "x/y".into() }
                .validate()
                .is_err()
        );

        let language = |code: &str, bytes: u64| OcrLanguage {
            code: code.into(),
            bundled: false,
            bytes,
        };
        let languages = |list: Vec<OcrLanguage>, automatic: Option<&str>| OcrLanguages {
            languages: list,
            automatic: automatic.map(str::to_owned),
        };
        assert_eq!(languages(vec![], None).validate(), Ok(()));
        assert_eq!(
            languages(vec![language("eng", 10), language("deu", 20)], Some("deu")).validate(),
            Ok(())
        );
        assert!(
            languages(vec![language("eng", 1), language("eng", 2)], None)
                .validate()
                .is_err()
        );
        assert!(
            languages(vec![language("e/ng", 1)], None)
                .validate()
                .is_err()
        );
        assert!(
            languages(vec![language("eng", u64::MAX)], None)
                .validate()
                .is_err()
        );
        assert!(
            languages(vec![language("eng", 1)], Some("deu"))
                .validate()
                .is_err()
        );

        let progress = |pages, checked, scans, recognised, failed| OcrProgress {
            doc: DocumentId(1),
            run: OcrRun::Running,
            pages,
            checked,
            scans,
            recognised,
            failed,
        };
        assert_eq!(progress(10, 7, 5, 3, 2).validate(), Ok(()));
        assert_eq!(progress(MAX_PAGE_COUNT, 0, 0, 0, 0).validate(), Ok(()));
        for bad in [
            progress(10, 11, 0, 0, 0),
            progress(10, 5, 6, 0, 0),
            progress(10, 5, 5, 4, 2),
            progress(MAX_PAGE_COUNT + 1, 0, 0, 0, 0),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let event = |page_index| OpenEvent::OcrPage {
            tab: TabId(1),
            doc: DocumentId(1),
            page_index,
        };
        assert_eq!(event(MAX_PAGE_COUNT - 1).validate(), Ok(()));
        assert!(event(MAX_PAGE_COUNT).validate().is_err());
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

    #[test]
    fn a_picture_is_turned_to_an_angle_within_a_full_turn_and_a_stamp_says_what_it_measures() {
        let turn = |page: u32, degrees: f32| Edit::SetPictureAngle {
            page,
            annotation: AnnotationId(3),
            degrees,
        };
        for angle in [0.0, 90.0, -45.5, 360.0, -360.0] {
            assert_eq!(turn(0, angle).validate(), Ok(()), "{angle}");
        }
        for angle in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 360.5, -361.0] {
            assert!(turn(0, angle).validate().is_err(), "{angle}");
        }
        assert!(turn(MAX_PAGE_COUNT, 10.0).validate().is_err());

        // What the page is told of a picture: an angle, and a size that is a size.
        let stamp = |picture: Option<PictureView>| PageAnnotation {
            id: AnnotationId(3),
            kind: AnnotationKind::Stamp,
            rect: Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 90.0,
                y1: 90.0,
            },
            color: None,
            text: None,
            picture,
        };
        let view = |angle: f32, width: f32, height: f32| {
            Some(PictureView {
                angle,
                width,
                height,
            })
        };
        assert_eq!(stamp(None).validate(), Ok(()));
        assert_eq!(stamp(view(15.0, 128.0, 64.0)).validate(), Ok(()));
        for wrong in [
            view(f32::NAN, 128.0, 64.0),
            view(400.0, 128.0, 64.0),
            view(0.0, 0.0, 64.0),
            view(0.0, 128.0, -1.0),
            view(0.0, f32::INFINITY, 64.0),
            view(0.0, MAX_PAGE_SIDE_PT * 2.0, 64.0),
        ] {
            assert!(stamp(wrong).validate().is_err(), "{wrong:?}");
        }
    }

    fn unlock(password: &str) -> UnlockArgs {
        UnlockArgs {
            tab: TabId(1),
            password: crate::types::Password::new(password.to_owned()),
            remember: false,
        }
    }

    #[test]
    fn a_password_is_never_shown_and_travels_as_a_plain_string() {
        let args = unlock("s3cret");
        assert!(!format!("{args:?}").contains("s3cret"));
        let json = serde_json::to_string(&args).unwrap();
        assert_eq!(json, r#"{"tab":1,"password":"s3cret","remember":false}"#);
        assert_eq!(serde_json::from_str::<UnlockArgs>(&json).unwrap(), args);
        // Nothing but the tab, the password and whether to remember it.
        assert!(
            serde_json::from_str::<UnlockArgs>(
                r#"{"tab":1,"password":"x","remember":false,"path":"C:\\x.pdf"}"#
            )
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
            can_remember: true,
            remembered_failed: false,
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

    fn password(text: &str) -> Option<Password> {
        Some(Password::new(text.to_owned()))
    }

    fn encrypt(
        open: Option<Password>,
        permissions: Option<Password>,
        restrictions: [bool; 3],
    ) -> EncryptArgs {
        let [print, copy, modify] = restrictions;
        EncryptArgs {
            doc: DocumentId(1),
            open_password: open,
            permissions_password: permissions,
            restrictions: Restrictions {
                print,
                copy,
                modify,
            },
        }
    }

    fn signature(status: SignatureStatus) -> SignatureInfo {
        SignatureInfo {
            status,
            signer_trusted: false,
            reason: None,
            field_name: Some("Signature1".to_owned()),
            signer: None,
            claimed_time: Some("2026-09-24 12:00:00 UTC+08:00".to_owned()),
            certification: None,
        }
    }

    #[test]
    fn a_watermark_is_one_line_of_words_or_a_picture_that_can_be_seen() {
        use crate::types::{Watermark, WatermarkContent, WatermarkPages, WatermarkPlace};
        let words = |text: &str, size: f32, opacity: u8| Watermark {
            content: WatermarkContent::Text {
                text: text.to_owned(),
                size,
                color: Rgb { r: 0, g: 0, b: 0 },
            },
            opacity,
            diagonal: true,
            place: WatermarkPlace::Center,
            pages: WatermarkPages::All,
        };
        assert!(words("機密 CONFIDENTIAL", 48.0, 30).validate().is_ok());
        assert!(
            words("x", MIN_WATERMARK_SIZE_PT, MIN_WATERMARK_OPACITY)
                .validate()
                .is_ok()
        );
        assert!(
            words(&"a".repeat(MAX_WATERMARK_CHARS), MAX_WATERMARK_SIZE_PT, 100)
                .validate()
                .is_ok()
        );
        for wrong in [
            words("", 48.0, 30),
            words("   ", 48.0, 30),
            words(&"a".repeat(MAX_WATERMARK_CHARS + 1), 48.0, 30),
            words("two\nlines", 48.0, 30),
            words("a\u{202e}b", 48.0, 30),
            words("x", MIN_WATERMARK_SIZE_PT - 1.0, 30),
            words("x", MAX_WATERMARK_SIZE_PT + 1.0, 30),
            words("x", f32::NAN, 30),
            words("x", 48.0, MIN_WATERMARK_OPACITY - 1),
            words("x", 48.0, 101),
        ] {
            assert!(wrong.validate().is_err(), "{wrong:?}");
        }
        let picture = |width_pt: f32| Watermark {
            content: WatermarkContent::Picture { width_pt },
            ..words("x", 48.0, 50)
        };
        assert!(picture(120.0).validate().is_ok());
        for width in [
            MIN_WATERMARK_WIDTH_PT - 1.0,
            MAX_WATERMARK_WIDTH_PT + 1.0,
            f32::INFINITY,
        ] {
            assert!(picture(width).validate().is_err(), "{width}");
        }
        // What the page sends, and nothing else.
        let json = r#"{"content":{"kind":"picture","widthPt":100},"opacity":40,"diagonal":false,"place":"bottom","pages":"allButFirst"}"#;
        let parsed: Watermark = serde_json::from_str(json).expect("a watermark");
        assert!(parsed.validate().is_ok());
        let more = r#"{"content":{"kind":"text","text":"x","size":20,"color":{"r":0,"g":0,"b":0}},"opacity":40,"diagonal":false,"place":"top","pages":"all","font":"C:/x.ttf"}"#;
        assert!(serde_json::from_str::<Watermark>(more).is_err());
    }

    #[test]
    fn a_batch_is_checked_as_its_operation_is() {
        let batch = |operation| BatchArgs {
            request: RequestId(7),
            operation,
        };
        assert!(batch(BatchOperation::RemoveMetadata).validate().is_ok());
        assert!(batch(BatchOperation::RemoveExif).validate().is_ok());
        // The rules of one encrypted copy, for each file of a batch.
        for (open, permissions, restrictions, ok) in [
            (password("open"), None, [false; 3], true),
            (None, password("owner"), [true, false, false], true),
            (None, None, [false; 3], false),
            (password("open"), None, [false, true, false], false),
            (
                password("same"),
                password("same"),
                [true, false, false],
                false,
            ),
            (password(""), None, [false; 3], false),
        ] {
            let args = encrypt(open, permissions, restrictions);
            let operation = BatchOperation::Encrypt {
                open_password: args.open_password.clone(),
                permissions_password: args.permissions_password.clone(),
                restrictions: args.restrictions,
            };
            assert_eq!(batch(operation).validate().is_ok(), ok);
            assert_eq!(args.validate().is_ok(), ok);
        }
        // Watermark operation: words or picture.
        let watermark = Watermark {
            content: WatermarkContent::Text {
                text: "CONFIDENTIAL".to_owned(),
                size: 40.0,
                color: Rgb { r: 200, g: 0, b: 0 },
            },
            opacity: 50,
            diagonal: true,
            place: WatermarkPlace::Center,
            pages: WatermarkPages::All,
        };
        assert!(
            batch(BatchOperation::Watermark {
                watermark: watermark.clone()
            })
            .validate()
            .is_ok()
        );

        let mut invalid_watermark = watermark.clone();
        invalid_watermark.opacity = 2; // below 5
        assert!(
            batch(BatchOperation::Watermark {
                watermark: invalid_watermark
            })
            .validate()
            .is_err()
        );

        // What the page sends, and nothing else.
        let json = r#"{"request":3,"operation":{"kind":"encrypt","openPassword":"pw","permissionsPassword":null,"restrictions":{"print":false,"copy":false,"modify":false}}}"#;
        let args: BatchArgs = serde_json::from_str(json).expect("batch args");
        assert!(args.validate().is_ok());
        let watermark_json = r#"{"request":4,"operation":{"kind":"watermark","watermark":{"content":{"kind":"text","text":"DRAFT","size":20.0,"color":{"r":0,"g":0,"b":0}},"opacity":50,"diagonal":false,"place":"center","pages":"all"}}}"#;
        let watermark_args: BatchArgs =
            serde_json::from_str(watermark_json).expect("batch watermark args");
        assert!(watermark_args.validate().is_ok());
        let more = r#"{"request":3,"operation":{"kind":"removeMetadata"},"files":["C:/x.pdf"]}"#;
        assert!(serde_json::from_str::<BatchArgs>(more).is_err());

        // Compressing the pictures (B3-19): the rules of one document's compressed copy.
        for (max_side, ok) in [
            (None, true),
            (Some(MIN_COMPRESS_SIDE_PX), true),
            (Some(2_000), true),
            (Some(MAX_COMPRESS_SIDE_PX), true),
            (Some(MIN_COMPRESS_SIDE_PX - 1), false),
            (Some(MAX_COMPRESS_SIDE_PX + 1), false),
        ] {
            let operation = BatchOperation::CompressImages {
                quality: ImageQuality::Medium,
                max_side,
            };
            assert_eq!(batch(operation).validate().is_ok(), ok, "{max_side:?}");
        }
        let compress =
            r#"{"request":5,"operation":{"kind":"compressImages","quality":"low","maxSide":null}}"#;
        let args: BatchArgs = serde_json::from_str(compress).expect("batch compress args");
        assert_eq!(
            args.operation,
            BatchOperation::CompressImages {
                quality: ImageQuality::Low,
                max_side: None
            }
        );
        assert!(args.validate().is_ok());
        // Nothing but what the page sends: no file, no folder, no quality of its own.
        for more in [
            r#"{"request":5,"operation":{"kind":"compressImages","quality":"low","maxSide":null,"folder":"C:/x"}}"#,
            r#"{"request":5,"operation":{"kind":"encrypt","openPassword":"pw","permissionsPassword":null,"restrictions":{"print":false,"copy":false,"modify":false},"file":"C:/x.pdf"}}"#,
            r#"{"request":5,"operation":{"kind":"compressImages","quality":"lossless","maxSide":null}}"#,
        ] {
            assert!(serde_json::from_str::<BatchArgs>(more).is_err(), "{more}");
        }

        // Page marks (B3-19): page numbers, Bates numbers, headers and footers.
        let page_marks = r#"{"request":7,"operation":{"kind":"pageMarks","marks":{"fontSize":12.0,"color":{"r":0,"g":0,"b":0},"marginPt":36.0,"pages":{"kind":"all"},"pageNumber":{"format":{"kind":"number"},"start":1,"position":"bottomCenter"}}}}"#;
        let args: BatchArgs = serde_json::from_str(page_marks).expect("batch page marks args");
        assert_eq!(
            args.operation,
            BatchOperation::PageMarks {
                marks: PageMarks {
                    page_number: Some(PageNumberMark {
                        format: PageNumberFormat::Number,
                        start: 1,
                        position: PageMarkPosition::BottomCenter,
                    }),
                    bates: None,
                    header_footer: None,
                    font_size: 12.0,
                    color: Rgb { r: 0, g: 0, b: 0 },
                    margin_pt: 36.0,
                    pages: PageMarkPages::All,
                }
            }
        );
        assert!(args.validate().is_ok());

        // Invalid page marks rejected.
        let invalid_marks = PageMarks {
            page_number: None,
            bates: None,
            header_footer: None,
            font_size: 5.0,
            color: Rgb { r: 0, g: 0, b: 0 },
            margin_pt: 36.0,
            pages: PageMarkPages::All,
        };
        assert!(
            batch(BatchOperation::PageMarks {
                marks: invalid_marks
            })
            .validate()
            .is_err()
        );

        // Unknown fields rejected in JSON.
        for more in [
            r#"{"request":7,"operation":{"kind":"pageMarks","marks":{"fontSize":12.0,"color":{"r":0,"g":0,"b":0},"marginPt":36.0,"pages":{"kind":"all"}},"folder":"C:/x"}}"#,
            r#"{"request":7,"operation":{"kind":"pageMarks","marks":{"fontSize":12.0,"color":{"r":0,"g":0,"b":0},"marginPt":36.0,"pages":{"kind":"all"},"extra":true}}}"#,
            r#"{"request":7,"operation":{"kind":"pageMarks"}}"#,
        ] {
            assert!(serde_json::from_str::<BatchArgs>(more).is_err(), "{more}");
        }

        // Sanitizing (B3-19): the options of one document's sanitized copy, as the page sends them.
        let sanitize = r#"{"request":6,"operation":{"kind":"sanitize","options":{"actions":true,"attachments":false,"formScripts":true,"xfa":true,"richMedia":true,"hiddenData":false,"keepWebLinks":false,"annotations":"flatten"}}}"#;
        let args: BatchArgs = serde_json::from_str(sanitize).expect("batch sanitize args");
        assert_eq!(
            args.operation,
            BatchOperation::Sanitize {
                options: SanitizeOptions {
                    attachments: false,
                    hidden_data: false,
                    keep_web_links: false,
                    annotations: crate::types::SanitizeAnnotations::Flatten,
                    ..SanitizeOptions::default()
                }
            }
        );
        assert!(args.validate().is_ok());
        assert!(
            batch(BatchOperation::Sanitize {
                options: SanitizeOptions::default()
            })
            .validate()
            .is_ok()
        );
        // Nothing but what the page sends: no file or folder, no option one document lacks, every
        // option given.
        for more in [
            r#"{"request":6,"operation":{"kind":"sanitize","options":{"actions":true,"attachments":true,"formScripts":true,"xfa":true,"richMedia":true,"hiddenData":true,"keepWebLinks":true,"annotations":"keep"},"folder":"C:/x"}}"#,
            r#"{"request":6,"operation":{"kind":"sanitize","options":{"actions":true,"attachments":true,"formScripts":true,"xfa":true,"richMedia":true,"hiddenData":true,"keepWebLinks":true,"annotations":"keep","signatures":true}}}"#,
            r#"{"request":6,"operation":{"kind":"sanitize","options":{"actions":true,"attachments":true,"formScripts":true,"xfa":true,"richMedia":true,"hiddenData":true,"annotations":"keep"}}}"#,
            r#"{"request":6,"operation":{"kind":"sanitize","options":{"actions":true,"attachments":true,"formScripts":true,"xfa":true,"richMedia":true,"hiddenData":true,"keepWebLinks":true,"annotations":"redact"}}}"#,
            r#"{"request":6,"operation":{"kind":"sanitize"}}"#,
        ] {
            assert!(serde_json::from_str::<BatchArgs>(more).is_err(), "{more}");
        }
    }

    #[test]
    fn what_a_copy_is_encrypted_with_is_checked() {
        let none = [false; 3];
        // An open password alone; restrictions with their password; both.
        assert!(encrypt(password("open"), None, none).validate().is_ok());
        assert!(
            encrypt(None, password("owner"), [true, false, false])
                .validate()
                .is_ok()
        );
        assert!(
            encrypt(password("open"), password("owner"), [true, true, true])
                .validate()
                .is_ok()
        );
        // A permissions password without a restriction is allowed when something else is asked.
        assert!(
            encrypt(password("open"), password("owner"), none)
                .validate()
                .is_ok()
        );

        // Nothing is asked for; a restriction nobody could lift; a permissions password that is
        // the open one, which would let those who open it lift the restrictions.
        assert!(encrypt(None, None, none).validate().is_err());
        assert!(encrypt(None, password("owner"), none).validate().is_err());
        assert!(
            encrypt(password("open"), None, [false, true, false])
                .validate()
                .is_err()
        );
        assert!(
            encrypt(None, None, [true, false, false])
                .validate()
                .is_err()
        );
        assert!(
            encrypt(password("same"), password("same"), [true, false, false])
                .validate()
                .is_err()
        );

        // Passwords: not empty, no NUL, at most 127 bytes (the standard's, MuPDF's buffer).
        let longest = "p".repeat(MAX_NEW_PASSWORD_BYTES as usize);
        assert!(encrypt(password(&longest), None, none).validate().is_ok());
        assert!(
            encrypt(password(&format!("{longest}p")), None, none)
                .validate()
                .is_err()
        );
        assert!(encrypt(password(""), None, none).validate().is_err());
        assert!(encrypt(password("a\0b"), None, none).validate().is_err());
        assert!(
            encrypt(
                password("open"),
                password(&format!("{longest}p")),
                [true, false, false]
            )
            .validate()
            .is_err()
        );
        // Bytes, not characters: 43 of these are 129 bytes.
        assert!(
            encrypt(password(&"密".repeat(43)), None, none)
                .validate()
                .is_err()
        );
        assert!(
            encrypt(password(&"密".repeat(42)), None, none)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_signature_report_is_bounded_and_consistent() {
        let report = |signatures| SignatureReport {
            signatures,
            truncated: false,
        };
        let holds = SignatureInfo {
            signer: Some("Jane Public".to_owned()),
            signer_trusted: true,
            ..signature(SignatureStatus::Valid)
        };
        assert!(
            report(vec![holds.clone(), signature(SignatureStatus::Invalid)])
                .validate()
                .is_ok()
        );
        let many = vec![signature(SignatureStatus::Invalid); MAX_SIGNATURES as usize];
        assert!(report(many.clone()).validate().is_ok());
        let mut too_many = many;
        too_many.push(signature(SignatureStatus::Invalid));
        assert!(report(too_many).validate().is_err());

        // A reason is for a signature that could not be verified, and only for that.
        let unverifiable = SignatureInfo {
            reason: Some(UnverifiableReason::UnsupportedFormat),
            ..signature(SignatureStatus::Unverifiable)
        };
        assert!(unverifiable.validate().is_ok());
        assert!(signature(SignatureStatus::Unverifiable).validate().is_err());
        assert!(
            SignatureInfo {
                reason: Some(UnverifiableReason::TooLarge),
                ..holds.clone()
            }
            .validate()
            .is_err()
        );
        // Nobody signed a signature that does not hold, and nobody is trusted by it.
        for status in [SignatureStatus::Invalid, SignatureStatus::Unverifiable] {
            let named = SignatureInfo {
                signer: Some("Jane Public".to_owned()),
                reason: (status == SignatureStatus::Unverifiable)
                    .then_some(UnverifiableReason::TooLarge),
                ..signature(status)
            };
            assert!(named.validate().is_err(), "{status:?}");
            let trusted = SignatureInfo {
                signer_trusted: true,
                reason: named.reason,
                ..signature(status)
            };
            assert!(trusted.validate().is_err(), "{status:?}");
        }
        // Text is bounded and clean.
        let text = |signer: String| SignatureInfo {
            signer: Some(signer),
            ..holds.clone()
        };
        assert!(
            text("x".repeat(MAX_SIGNATURE_TEXT_BYTES as usize))
                .validate()
                .is_ok()
        );
        assert!(
            text("x".repeat(MAX_SIGNATURE_TEXT_BYTES as usize + 1))
                .validate()
                .is_err()
        );
        assert!(text("Jane\u{202e}Public".to_owned()).validate().is_err());
        assert!(text("Jane\nPublic".to_owned()).validate().is_err());
        let time = |time: &str| SignatureInfo {
            claimed_time: Some(time.to_owned()),
            ..holds.clone()
        };
        assert!(
            time(&"9".repeat(MAX_SIGNATURE_TIME_BYTES as usize))
                .validate()
                .is_ok()
        );
        assert!(
            time(&"9".repeat(MAX_SIGNATURE_TIME_BYTES as usize + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn only_a_request_for_a_password_carries_a_key() {
        use crate::worker::PasswordKey;
        let error = |code, password_key| WorkerResponse::Error {
            request: None,
            error: WorkerError {
                code,
                detail: "x".to_owned(),
                password_key,
            },
        };
        let key = Some(PasswordKey([1; 32]));
        assert!(error(WorkerErrorCode::Encrypted, key).validate().is_ok());
        assert!(error(WorkerErrorCode::Encrypted, None).validate().is_ok());
        for code in [
            WorkerErrorCode::WrongPassword,
            WorkerErrorCode::Corrupted,
            WorkerErrorCode::Internal,
        ] {
            assert!(error(code, key).validate().is_err(), "{code:?}");
            assert!(error(code, None).validate().is_ok(), "{code:?}");
        }
    }

    #[test]
    fn worker_error_detail_is_bounded() {
        let error = |detail: String| WorkerResponse::Error {
            request: None,
            error: WorkerError {
                code: WorkerErrorCode::Corrupted,
                detail,
                password_key: None,
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
            region: None,
        };
        assert!(args(1.0).validate().is_ok());
        assert!(args(0.0).validate().is_err());
        assert!(args(f32::NAN).validate().is_err());
        assert!(args(MAX_RENDER_SCALE * 2.0).validate().is_err());

        // A region (B3-09): not empty, within the raster limits and the largest page raster.
        let region = |x, y, width, height| RenderPageArgs {
            region: Some(RasterRegion {
                x,
                y,
                width,
                height,
            }),
            ..args(8.0)
        };
        assert!(region(5000, 9000, 2400, 1600).validate().is_ok());
        assert!(region(0, 0, 0, 1600).validate().is_err());
        assert!(region(0, 0, MAX_RASTER_SIDE_PX + 1, 1).validate().is_err());
        assert!(region(0, 0, 4097, 4096).validate().is_err());
        assert!(
            region(MAX_REGION_PAGE_SIDE_PX - 10, 0, 10, 10)
                .validate()
                .is_ok()
        );
        assert!(
            region(MAX_REGION_PAGE_SIDE_PX - 10, 0, 11, 10)
                .validate()
                .is_err()
        );
        assert!(region(0, u32::MAX, 10, 10).validate().is_err());
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

    fn sample_model() -> TextBoxModel {
        TextBoxModel {
            text: "Hello\n你好".to_owned(),
            font: "Droid Sans Fallback".to_owned(),
            face: 0,
            view_only: false,
            base: "ABCDEF+DroidSansFallback".to_owned(),
            ascent: 880,
            descent: -120,
            cap_height: 700,
            step: 1_300,
            bbox: [-100, -250, 1_100, 950],
            glyphs: "Helo你好"
                .chars()
                .zip(1u16..)
                .map(|(ch, cid)| ModelGlyph {
                    ch,
                    cid,
                    advance: 600,
                })
                .collect(),
            subset: [&[0u8, 1, 0, 0][..], &[0u8; 60][..]].concat(),
        }
    }

    #[test]
    fn a_text_box_model_has_words_that_all_have_glyphs_a_subset_and_sane_measures() {
        assert!(sample_model().validate().is_ok());
        let broken = |change: &dyn Fn(&mut TextBoxModel)| {
            let mut model = sample_model();
            change(&mut model);
            model.validate().is_err()
        };
        // The words.
        assert!(broken(&|m| m.text = "  \n".to_owned()));
        assert!(broken(&|m| m.text = "x".repeat(MAX_TEXT_BOX_BYTES + 1)));
        assert!(broken(&|m| m.text = "Hello\u{202e}".to_owned()));
        assert!(
            broken(&|m| m.text = "Hello!".to_owned()),
            "a character without a glyph"
        );
        // The glyphs.
        assert!(broken(&|m| m.glyphs[0].cid = 0));
        assert!(broken(&|m| m.glyphs[0].advance = 5_001));
        assert!(broken(&|m| m.glyphs[1].ch = 'H'), "a character twice");
        assert!(broken(&|m| m.glyphs[0].ch = '\n'));
        assert!(
            broken(&|m| {
                m.glyphs[1].cid = m.glyphs[0].cid;
                m.glyphs[1].advance = 601;
            }),
            "one glyph, two widths"
        );
        assert!(sample_model().glyphs.len() < MAX_TEXT_BOX_GLYPHS);
        // The font.
        assert!(broken(&|m| m.base = "Arial".to_owned()));
        assert!(broken(&|m| m.base = "ABCDE+Arial".to_owned()));
        assert!(broken(&|m| m.base = "ABCDEF+Ar ial".to_owned()));
        assert!(broken(&|m| m.font = "x".repeat(129)));
        assert!(broken(&|m| m.font = "Arial\u{0}".to_owned()));
        assert!(broken(&|m| m.step = 500));
        assert!(broken(&|m| m.step = 2_001));
        assert!(broken(&|m| m.ascent = 30_000));
        assert!(broken(&|m| m.bbox[2] = -30_000));
        assert!(
            broken(&|m| m.subset = vec![0, 1, 0, 0]),
            "too short for a font"
        );
        assert!(broken(&|m| m.subset[0] = 1), "not TrueType");
        assert!(broken(
            &|m| m.subset = vec![0; MAX_TEXT_BOX_SUBSET_BYTES + 1]
        ));
    }

    #[test]
    fn a_text_box_model_crosses_the_worker_boundary_and_the_journal_unchanged() {
        let model = sample_model();
        let bytes = postcard::to_allocvec(&model).unwrap();
        assert_eq!(postcard::from_bytes::<TextBoxModel>(&bytes).unwrap(), model);
        let json = serde_json::to_string(&model).unwrap();
        assert_eq!(serde_json::from_str::<TextBoxModel>(&json).unwrap(), model);
    }

    #[test]
    fn the_style_of_a_text_box_is_a_size_a_colour_and_an_alignment() {
        let style = |size: f32| TextBoxStyle {
            size,
            color: Rgb { r: 1, g: 2, b: 3 },
            align: TextAlign::Center,
        };
        assert!(style(MIN_TEXT_BOX_SIZE).validate().is_ok());
        assert!(style(MAX_TEXT_BOX_SIZE).validate().is_ok());
        for bad in [
            0.0,
            -1.0,
            MIN_TEXT_BOX_SIZE - 0.1,
            MAX_TEXT_BOX_SIZE + 0.1,
            f32::NAN,
            f32::INFINITY,
        ] {
            assert!(style(bad).validate().is_err(), "{bad}");
        }
        let json = serde_json::json!({"size": 14.0, "color": {"r": 26, "g": 51, "b": 153}, "align": "right"});
        let parsed: TextBoxStyle = serde_json::from_value(json).unwrap();
        assert_eq!((parsed.size, parsed.align), (14.0, TextAlign::Right));
        for bad in [
            serde_json::json!({"size": 14.0, "color": {"r": 256, "g": 0, "b": 0}, "align": "left"}),
            serde_json::json!({"size": 14.0, "color": {"r": 0, "g": 0, "b": 0}, "align": "justify"}),
        ] {
            assert!(serde_json::from_value::<TextBoxStyle>(bad).is_err());
        }
    }

    #[test]
    fn text_boxes_have_a_place_a_style_and_prepared_words() {
        let rect = Rect {
            x0: 72.0,
            y0: 72.0,
            x1: 372.0,
            y1: 132.0,
        };
        let style = |size: f32| TextBoxStyle {
            size,
            color: Rgb { r: 0, g: 0, b: 0 },
            align: TextAlign::Left,
        };
        let text_box = |size: f32, rect: Rect| Edit::AddTextBox {
            page: 0,
            rect,
            model: TextBoxModelId(3),
            style: style(size),
        };
        assert!(text_box(12.0, rect).validate().is_ok());
        assert!(
            text_box(MIN_TEXT_BOX_SIZE, rect).validate().is_ok()
                && text_box(MAX_TEXT_BOX_SIZE, rect).validate().is_ok()
        );
        for bad in [
            0.0,
            MIN_TEXT_BOX_SIZE - 0.5,
            MAX_TEXT_BOX_SIZE + 0.5,
            f32::NAN,
            f32::INFINITY,
            -12.0,
        ] {
            assert!(text_box(bad, rect).validate().is_err(), "{bad}");
        }
        // The place is checked as any annotation's is.
        assert!(
            text_box(12.0, Rect { x1: 75.0, ..rect })
                .validate()
                .is_err()
        );
        assert!(
            text_box(
                12.0,
                Rect {
                    y1: f32::NAN,
                    ..rect
                }
            )
            .validate()
            .is_err()
        );
        assert!(
            Edit::AddTextBox {
                page: MAX_PAGE_COUNT,
                rect,
                model: TextBoxModelId(3),
                style: style(12.0),
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn the_words_to_prepare_are_something_to_see_within_the_limit_and_only_text() {
        let args = |text: &str| PrepareTextBoxArgs {
            doc: DocumentId(1),
            text: text.to_owned(),
            font: FontId(7),
        };
        assert!(args("Hello").validate().is_ok());
        assert!(args("中文\nsecond line").validate().is_ok());
        assert!(args(&"x".repeat(MAX_TEXT_BOX_BYTES)).validate().is_ok());
        assert!(
            args(&"x".repeat(MAX_TEXT_BOX_BYTES + 1))
                .validate()
                .is_err()
        );
        for bad in [
            "",
            " \n  ",
            "a\u{0}b",
            "a\u{7}b",
            "a\tb",
            "a\u{202e}b",
            "a\u{200b}b",
            "a\rb",
        ] {
            assert!(args(bad).validate().is_err(), "{bad:?}");
        }
        let parsed: PrepareTextBoxArgs =
            serde_json::from_value(serde_json::json!({"doc": 4, "text": "你好", "font": 7}))
                .unwrap();
        assert_eq!((parsed.doc, parsed.font), (DocumentId(4), FontId(7)));
        // Nothing but the document, the words and the font: no path, no bytes.
        assert!(
            serde_json::from_value::<PrepareTextBoxArgs>(
                serde_json::json!({"doc": 4, "text": "x", "font": 7, "path": "C:/x.ttf"})
            )
            .is_err()
        );
    }

    #[test]
    fn a_text_box_edit_has_the_form_the_page_sends() {
        let edit: Edit = serde_json::from_value(serde_json::json!({
            "kind": "addTextBox", "page": 2,
            "rect": {"x0": 72.0, "y0": 72.0, "x1": 372.0, "y1": 132.0},
            "model": 5,
            "style": {"size": 14.0, "color": {"r": 26, "g": 51, "b": 153}, "align": "center"}
        }))
        .unwrap();
        assert_eq!(
            edit,
            Edit::AddTextBox {
                page: 2,
                rect: Rect {
                    x0: 72.0,
                    y0: 72.0,
                    x1: 372.0,
                    y1: 132.0
                },
                model: TextBoxModelId(5),
                style: TextBoxStyle {
                    size: 14.0,
                    color: Rgb {
                        r: 26,
                        g: 51,
                        b: 153
                    },
                    align: TextAlign::Center,
                },
            }
        );
        assert!(edit.validate().is_ok());
        // The words are not in the edit, nor the font: only the number they were prepared under.
        let json = serde_json::to_value(&edit).unwrap();
        assert!(json.get("text").is_none() && json.get("font").is_none());
        assert_eq!(json["model"], 5);
    }

    #[test]
    fn a_text_box_can_get_another_style_and_other_words() {
        let style = |size: f32| TextBoxStyle {
            size,
            color: Rgb { r: 9, g: 9, b: 9 },
            align: TextAlign::Center,
        };
        let set = |model: Option<u32>, size: f32, page: u32| Edit::SetTextBox {
            page,
            annotation: AnnotationId(5),
            model: model.map(TextBoxModelId),
            style: style(size),
        };
        assert!(set(None, 12.0, 0).validate().is_ok());
        assert!(set(Some(3), 12.0, 0).validate().is_ok());
        for bad in [
            0.0,
            MIN_TEXT_BOX_SIZE - 0.5,
            MAX_TEXT_BOX_SIZE + 0.5,
            f32::NAN,
        ] {
            assert!(set(None, bad, 0).validate().is_err(), "{bad}");
        }
        assert!(set(None, 12.0, MAX_PAGE_COUNT).validate().is_err());
        // The form the page sends: the words are optional, and named by number when they are there.
        let with: Edit = serde_json::from_value(serde_json::json!({
            "kind": "setTextBox", "page": 1, "annotation": 5, "model": 3,
            "style": {"size": 12.0, "color": {"r": 9, "g": 9, "b": 9}, "align": "center"}
        }))
        .unwrap();
        assert_eq!(with, set(Some(3), 12.0, 1));
        let without: Edit = serde_json::from_value(serde_json::json!({
            "kind": "setTextBox", "page": 1, "annotation": 5, "model": null,
            "style": {"size": 12.0, "color": {"r": 9, "g": 9, "b": 9}, "align": "center"}
        }))
        .unwrap();
        assert_eq!(without, set(None, 12.0, 1));
    }

    #[test]
    fn what_a_text_box_says_of_itself_is_checked_like_everything_from_the_worker() {
        let info = || TextBoxInfo {
            text: "Hello\n你好".to_owned(),
            font: "Arial".to_owned(),
            style: TextBoxStyle {
                size: 12.0,
                color: Rgb { r: 0, g: 0, b: 0 },
                align: TextAlign::Left,
            },
            view_only: false,
            overflow: true,
        };
        assert!(info().validate().is_ok());
        let broken = |change: &dyn Fn(&mut TextBoxInfo)| {
            let mut info = info();
            change(&mut info);
            info.validate().is_err()
        };
        assert!(broken(&|i| i.text = " ".to_owned()));
        assert!(broken(&|i| i.text = "a\u{202e}b".to_owned()));
        assert!(broken(&|i| i.text = "x".repeat(MAX_TEXT_BOX_BYTES + 1)));
        assert!(broken(&|i| i.font = "x".repeat(129)));
        assert!(broken(&|i| i.font = "Arial\u{0}".to_owned()));
        assert!(broken(&|i| i.style.size = f32::INFINITY));
        let response = WorkerResponse::TextBox {
            request: RequestId(1),
            info: info(),
        };
        assert!(response.validate().is_ok());
        let json = serde_json::to_value(info()).unwrap();
        assert_eq!(json["viewOnly"], false);
        assert_eq!(json["style"]["align"], "left");
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

        // A redaction mark (B2-19) is an area like a stamp's; applying takes nothing.
        let mark = |rect: Rect| Edit::AddRedaction { page: 0, rect };
        let area = |x0: f32, y0: f32, x1: f32, y1: f32| Rect { x0, y0, x1, y1 };
        assert!(mark(area(60.0, 120.0, 330.0, 150.0)).validate().is_ok());
        for wrong in [
            area(60.0, 150.0, 330.0, 120.0),
            area(60.0, 120.0, 60.0 + MIN_ANNOTATION_SIDE_PT / 2.0, 150.0),
            area(f32::NAN, 120.0, 330.0, 150.0),
            area(60.0, 120.0, f32::INFINITY, 150.0),
        ] {
            assert!(mark(wrong).validate().is_err(), "{wrong:?}");
        }
        assert!(
            Edit::AddRedaction {
                page: MAX_PAGE_COUNT,
                rect: area(60.0, 120.0, 330.0, 150.0),
            }
            .validate()
            .is_err()
        );
        assert!(Edit::ApplyRedactions.validate().is_ok());
        assert_eq!(
            serde_json::from_str::<Edit>(r#"{"kind":"applyRedactions"}"#).unwrap(),
            Edit::ApplyRedactions
        );
        assert!(
            serde_json::from_str::<Edit>(
                r#"{"kind":"addRedaction","page":0,"rect":{"x0":1,"y0":2,"x1":30,"y1":40}}"#
            )
            .is_ok()
        );
        let add_bm = |title: &str, page: u32, index: u32| Edit::AddBookmark {
            parent: None,
            index,
            title: title.to_owned(),
            page,
        };
        assert!(add_bm("Chapter 1", 0, 0).validate().is_ok());
        assert!(add_bm("", 0, 0).validate().is_err());
        assert!(add_bm("   ", 0, 0).validate().is_err());
        assert!(
            add_bm(&"x".repeat(MAX_TEXT_BYTES as usize + 1), 0, 0)
                .validate()
                .is_err()
        );
        assert!(add_bm("bad\u{0}title", 0, 0).validate().is_err());
        assert!(add_bm("ok", MAX_PAGE_COUNT, 0).validate().is_err());
        assert!(add_bm("ok", 0, MAX_OUTLINE_ITEMS + 1).validate().is_err());
        let rename_bm = |title: &str| Edit::RenameBookmark {
            id: BookmarkId(5),
            title: title.to_owned(),
        };
        assert!(rename_bm("New Title").validate().is_ok());
        assert!(rename_bm("").validate().is_err());
        assert!(rename_bm(" \t ").validate().is_err());
        assert!(
            rename_bm(&"x".repeat(MAX_TEXT_BYTES as usize + 1))
                .validate()
                .is_err()
        );
        assert!(
            Edit::DeleteBookmark { id: BookmarkId(5) }
                .validate()
                .is_ok()
        );
        let move_bm = |parent: Option<BookmarkId>, index: u32| Edit::MoveBookmark {
            id: BookmarkId(5),
            parent,
            index,
        };
        assert!(move_bm(None, 0).validate().is_ok());
        assert!(move_bm(Some(BookmarkId(2)), 1).validate().is_ok());
        assert!(move_bm(Some(BookmarkId(5)), 0).validate().is_err());
        assert!(move_bm(None, MAX_OUTLINE_ITEMS + 1).validate().is_err());
        assert_eq!(
            serde_json::from_str::<Edit>(
                r#"{"kind":"addBookmark","parent":null,"index":0,"title":"Ch1","page":2}"#
            )
            .unwrap(),
            Edit::AddBookmark {
                parent: None,
                index: 0,
                title: "Ch1".to_owned(),
                page: 2,
            }
        );

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
        let picture = |rect: Rect| Edit::AddImageStamp {
            page: 0,
            rect,
            image: StampImageId(1),
        };
        for make in [stamp, moved, picture] {
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

        // The page of a picture stamp is checked as any annotation's is.
        assert!(
            Edit::AddImageStamp {
                page: MAX_PAGE_COUNT,
                rect: rect(10.0, 10.0, 100.0, 60.0),
                image: StampImageId(1),
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<Edit>(serde_json::json!({
                "kind": "addImageStamp", "page": 1, "image": 4,
                "rect": {"x0": 1.0, "y0": 2.0, "x1": 91.0, "y1": 52.0}
            }))
            .unwrap(),
            Edit::AddImageStamp {
                page: 1,
                rect: rect(1.0, 2.0, 91.0, 52.0),
                image: StampImageId(4),
            }
        );

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
            picture: None,
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
    fn a_redaction_summary_counts_what_a_document_can_have() {
        use crate::types::RedactionSummary;
        let summary = |marks: u32, pages: u32, annotations: u32, fields: u32| RedactionSummary {
            marks,
            pages,
            annotations,
            fields,
        };
        assert!(summary(0, 0, 0, 0).validate().is_ok());
        assert!(summary(3, 2, 1, 1).validate().is_ok());
        // More pages with marks than marks, or than a document has; counts beyond any document.
        for wrong in [
            summary(1, 2, 0, 0),
            summary(u32::MAX, MAX_PAGE_COUNT + 1, 0, 0),
            summary(u32::MAX, 1, 0, 0),
            summary(1, 1, u32::MAX, 0),
            summary(1, 1, 0, u32::MAX),
        ] {
            assert!(wrong.validate().is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn personal_data_query_and_matches_are_checked() {
        use crate::types::PersonalDataCategory;
        let valid_query = FindPersonalDataQuery {
            categories: vec![
                PersonalDataCategory::TaiwanId,
                PersonalDataCategory::MobilePhone,
                PersonalDataCategory::Keyword,
            ],
            keyword: Some("needle".to_owned()),
        };
        assert!(valid_query.validate().is_ok());

        let keyword_without_category = FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::TaiwanId],
            keyword: Some("needle".to_owned()),
        };
        assert!(keyword_without_category.validate().is_err());

        let category_without_keyword = FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::Keyword],
            keyword: None,
        };
        assert!(category_without_keyword.validate().is_err());

        let empty_kw_query = FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::Keyword],
            keyword: Some("   ".to_owned()),
        };
        assert!(empty_kw_query.validate().is_err());

        let valid_match = PersonalDataMatch {
            category: PersonalDataCategory::TaiwanId,
            page: 0,
            rects: vec![Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 100.0,
                y1: 30.0,
            }],
            preview: "A12****789".to_owned(),
        };
        assert!(valid_match.validate().is_ok());

        let no_rects = PersonalDataMatch {
            category: PersonalDataCategory::TaiwanId,
            page: 0,
            rects: vec![],
            preview: "A12****789".to_owned(),
        };
        assert!(no_rects.validate().is_err());

        let matches = PersonalDataMatches {
            matches: vec![valid_match],
            has_more: false,
        };
        assert!(matches.validate().is_ok());
    }

    #[test]
    fn the_sensitive_mark_takes_a_password_only_as_an_unlock_one_and_nothing_else() {
        use crate::types::{SensitiveArgs, SensitiveFile, SensitiveId};
        let args = |password: Option<&str>| SensitiveArgs {
            doc: DocumentId(1),
            sensitive: true,
            password: password.map(|password| Password::new(password.to_owned())),
        };
        assert!(args(None).validate().is_ok());
        assert!(args(Some("owner")).validate().is_ok());
        assert!(args(Some("")).validate().is_err());
        assert!(args(Some("a\u{0}b")).validate().is_err());
        assert_eq!(
            serde_json::from_value::<SensitiveArgs>(
                serde_json::json!({ "doc": 1, "sensitive": true, "password": "owner" })
            )
            .unwrap(),
            args(Some("owner"))
        );
        // A path or an identifier of the page's choosing is no argument.
        for value in [
            serde_json::json!({ "doc": 1, "sensitive": true, "path": "C:/a.pdf" }),
            serde_json::json!({ "doc": 1, "sensitive": true, "identifier": [0, 0, 0, 0] }),
        ] {
            assert!(serde_json::from_value::<SensitiveArgs>(value).is_err());
        }

        // The list: names only, bounded.
        let file = |name: &str| SensitiveFile {
            id: SensitiveId(1),
            display_name: name.to_owned(),
            open: false,
        };
        assert!(file("機密.pdf").validate().is_ok());
        assert!(file("C:\\secret\\a.pdf").validate().is_err());
        let full = vec![file("a.pdf"); MAX_SENSITIVE_FILES as usize];
        assert!(full.as_slice().validate().is_ok());
        let more = vec![file("a.pdf"); MAX_SENSITIVE_FILES as usize + 1];
        assert!(more.as_slice().validate().is_err());
    }

    #[test]
    fn clipboard_text_is_some_bounded_and_whole() {
        assert!(check_clipboard_text("機密\nline two").is_ok());
        assert!(check_clipboard_text("").is_err());
        assert!(check_clipboard_text("before\u{0}after").is_err());
        let most = "a".repeat(MAX_CLIPBOARD_TEXT_BYTES as usize);
        assert!(check_clipboard_text(&most).is_ok());
        let more = "a".repeat(MAX_CLIPBOARD_TEXT_BYTES as usize + 1);
        assert!(check_clipboard_text(&more).is_err());
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
            redaction: None,
        };
        let created = |bytes: u64, pages: u32| WorkerResponse::Created {
            request: RequestId(1),
            bytes,
            pages,
        };
        assert!(created(1_000, 3).validate().is_ok());
        assert!(created(0, 3).validate().is_err());
        assert!(created(1_000, 0).validate().is_err());
        assert!(created(MAX_DOCUMENT_BYTES + 1, 3).validate().is_err());
        assert!(created(1_000, MAX_PAGE_COUNT + 1).validate().is_err());
        assert!(saved(1_234).validate().is_ok());
        assert!(saved(0).validate().is_err());
        assert!(saved(MAX_DOCUMENT_BYTES + 1).validate().is_err());
        let cleaned = |bytes: u64, pictures: u32, cleaned: u32| WorkerResponse::ExifCleaned {
            request: RequestId(1),
            bytes,
            pictures,
            cleaned,
        };
        assert!(cleaned(1_234, 3, 2).validate().is_ok());
        assert!(cleaned(1_234, 0, 0).validate().is_ok());
        assert!(cleaned(0, 3, 2).validate().is_err());
        assert!(cleaned(MAX_DOCUMENT_BYTES + 1, 3, 2).validate().is_err());
        assert!(cleaned(1_234, 2, 3).validate().is_err());
        assert!(
            cleaned(1_234, MAX_IMAGES_PER_DOCUMENT + 1, 0)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn the_copy_of_a_source_file_is_bounded_and_has_pages() {
        let source = |len: usize, pages: u32| WorkerResponse::Source {
            request: RequestId(1),
            bytes: vec![0; len],
            pages,
            security: SecurityReport::default(),
        };
        assert!(source(1_000, 3).validate().is_ok());
        assert!(source(MAX_SOURCE_BYTES, 1).validate().is_ok());
        assert!(source(MAX_SOURCE_BYTES + 1, 1).validate().is_err());
        assert!(source(1_000, 0).validate().is_err());
        assert!(source(1_000, MAX_PAGE_COUNT).validate().is_ok());
        assert!(source(1_000, MAX_PAGE_COUNT + 1).validate().is_err());
        // What the scan says is checked as it is for a document.
        let bad_report = SecurityReport {
            findings: vec![
                SecurityFinding {
                    kind: FindingKind::JavaScript,
                    count: 1,
                };
                20
            ],
            scan_complete: true,
            remote_addresses: Vec::new(),
        };
        assert!(
            WorkerResponse::Source {
                request: RequestId(1),
                bytes: Vec::new(),
                pages: 1,
                security: bad_report,
            }
            .validate()
            .is_err()
        );
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
            password_remembered: false,
            password_recalled: false,
            sensitive: false,
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
    fn an_annotation_summary_holds_cleaned_text_within_its_limits() {
        let item = |kind: AnnotationSummaryKind, text: &str| AnnotationSummaryItem {
            page_index: 0,
            kind,
            rect: Rect {
                x0: 72.0,
                y0: 72.0,
                x1: 200.0,
                y1: 90.0,
            },
            text: text.to_owned(),
        };
        let summary = |items: Vec<AnnotationSummaryItem>| AnnotationSummary {
            items,
            truncated: false,
        };
        assert!(
            summary(vec![
                item(AnnotationSummaryKind::Highlight, "Privacy-first PDF Reader"),
                item(AnnotationSummaryKind::Highlight, ""),
                item(AnnotationSummaryKind::Note, "第一行\nsecond line"),
                item(AnnotationSummaryKind::TextBox, "A box"),
                item(AnnotationSummaryKind::Ink, ""),
                item(AnnotationSummaryKind::Stamp, ""),
            ])
            .validate()
            .is_ok()
        );
        for wrong in [
            // A highlighter mark's text is one line of display text.
            item(AnnotationSummaryKind::Highlight, "two\nlines"),
            item(AnnotationSummaryKind::Highlight, "bidi \u{202E}txt.exe"),
            item(AnnotationSummaryKind::Highlight, " padded"),
            item(AnnotationSummaryKind::Note, "tab\there"),
            item(AnnotationSummaryKind::TextBox, "bell\u{7}"),
            // Drawings and stamps are listed by kind only.
            item(AnnotationSummaryKind::Ink, "第 1 頁有手繪"),
            item(AnnotationSummaryKind::Stamp, "Draft"),
            item(
                AnnotationSummaryKind::Note,
                &"x".repeat(MAX_ANNOTATION_SUMMARY_TEXT_BYTES as usize + 1),
            ),
            AnnotationSummaryItem {
                page_index: MAX_PAGE_COUNT,
                ..item(AnnotationSummaryKind::Ink, "")
            },
            AnnotationSummaryItem {
                rect: Rect {
                    x0: f32::NAN,
                    y0: 0.0,
                    x1: 1.0,
                    y1: 1.0,
                },
                ..item(AnnotationSummaryKind::Ink, "")
            },
        ] {
            assert!(
                summary(vec![wrong.clone()]).validate().is_err(),
                "{wrong:?}"
            );
        }
        let too_many =
            vec![item(AnnotationSummaryKind::Ink, ""); MAX_ANNOTATION_SUMMARY_ITEMS as usize + 1];
        assert!(summary(too_many).validate().is_err());
        let line = "x".repeat(MAX_ANNOTATION_SUMMARY_TEXT_BYTES as usize);
        let count = MAX_ANNOTATION_SUMMARY_TOTAL_BYTES / MAX_ANNOTATION_SUMMARY_TEXT_BYTES;
        let full = vec![item(AnnotationSummaryKind::Highlight, &line); count as usize];
        assert!(summary(full.clone()).validate().is_ok());
        let mut over = full;
        over.push(item(AnnotationSummaryKind::Highlight, "x"));
        assert!(summary(over).validate().is_err());
    }

    #[test]
    fn signing_takes_a_password_one_line_of_reason_and_a_time_zone() {
        let args = |password: &str, reason: Option<&str>, utc_offset_minutes: i16| SignArgs {
            doc: DocumentId(1),
            key: SigningKey::File(SigningFileKey {
                file: 1,
                password: Password::new(password.to_owned()),
            }),
            place: None,
            reason: reason.map(str::to_owned),
            utc_offset_minutes,
        };
        assert!(args("secret", None, 480).validate().is_ok());
        assert!(args("secret", Some("同意本合約"), -720).validate().is_ok());
        assert!(args("secret", Some("x"), 840).validate().is_ok());
        assert!(args("", None, 0).validate().is_err());
        assert!(args("a\0b", None, 0).validate().is_err());
        for reason in [
            "",
            "  ",
            "line\nbreak",
            "\u{202E}reversed",
            &"x".repeat(257),
        ] {
            assert!(
                args("secret", Some(reason), 0).validate().is_err(),
                "{reason:?}"
            );
        }
        assert!(args("secret", None, 841).validate().is_err());
        assert!(args("secret", None, -841).validate().is_err());
        // The page sends nothing else: no path of a certificate, no signer.
        let json = r#"{"doc":1,"key":{"kind":"file","file":1,"password":"p"},"place":{"pageIndex":0,"corner":"bottomRight"},"reason":null,"utcOffsetMinutes":0}"#;
        assert!(serde_json::from_str::<SignArgs>(json).is_ok());
        let more = r#"{"doc":1,"key":{"kind":"file","file":1,"password":"p"},"place":null,"reason":null,"utcOffsetMinutes":0,"path":"C:/x.pfx"}"#;
        assert!(serde_json::from_str::<SignArgs>(more).is_err());
        let corner = r#"{"doc":1,"key":{"kind":"file","file":1,"password":"p"},"place":{"pageIndex":0,"corner":"middle"},"reason":null,"utcOffsetMinutes":0}"#;
        assert!(serde_json::from_str::<SignArgs>(corner).is_err());
    }

    #[test]
    fn a_certificate_of_the_store_is_named_by_its_number_alone() {
        let store = |key: &str| {
            serde_json::from_str::<SignArgs>(&format!(
                r#"{{"doc":1,"key":{key},"place":null,"reason":null,"utcOffsetMinutes":480}}"#
            ))
        };
        let args = store(r#"{"kind":"store","certificate":2}"#).unwrap();
        assert_eq!(
            args.key,
            SigningKey::Store(crate::types::SigningStoreKey { certificate: 2 })
        );
        // No password, PIN or thumbprint: Windows asks for what the key needs.
        assert!(args.validate().is_ok());
        for key in [
            r#"{"kind":"store","certificate":2,"password":"p"}"#,
            r#"{"kind":"store","certificate":2,"pin":"1234"}"#,
            r#"{"kind":"store","thumbprint":"00"}"#,
            r#"{"kind":"file","file":1}"#,
            r#"{"kind":"card","certificate":2}"#,
        ] {
            assert!(store(key).is_err(), "{key}");
        }
    }

    #[test]
    fn the_room_for_a_signature_is_as_large_as_asked_and_inside_the_file() {
        let room = 2 * u64::from(SIGNATURE_RESERVE_BYTES) + 2;
        let prepared = |bytes: u64, contents_start: u64, contents_end: u64| {
            WorkerResponse::SignaturePrepared {
                request: RequestId(1),
                bytes,
                contents_start,
                contents_end,
            }
            .validate()
        };
        assert!(prepared(room + 1_000, 500, 500 + room).is_ok());
        assert!(prepared(room + 1_000, 500, 501 + room).is_err());
        assert!(prepared(room + 1_000, 500, 499 + room).is_err());
        assert!(prepared(room + 1_000, 0, room).is_err());
        assert!(prepared(room + 500, 500, 500 + room).is_err());
        assert!(prepared(room + 1_000, 1_000, 1_000 + room).is_err());
        assert!(prepared(MAX_DOCUMENT_BYTES + 1, 500, 500 + room).is_err());
        assert!(prepared(10, u64::MAX - 1, 5).is_err());
    }

    #[test]
    fn ipc_error_message_is_bounded() {
        let error = IpcError {
            code: ErrorCode::Internal,
            message: "x".repeat(MAX_ERROR_MESSAGE_BYTES as usize + 1),
        };
        assert!(error.validate().is_err());
    }

    #[test]
    fn page_marks_rules_and_limits_are_enforced() {
        let base_marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Number,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            bates: None,
            header_footer: None,
            font_size: 12.0,
            color: Rgb { r: 0, g: 0, b: 0 },
            margin_pt: 36.0,
            pages: PageMarkPages::All,
        };
        assert!(base_marks.validate().is_ok());

        // Font size bounds: 6.0..=72.0
        let mut invalid = base_marks.clone();
        invalid.font_size = 5.9;
        assert!(invalid.validate().is_err());
        invalid.font_size = 72.1;
        assert!(invalid.validate().is_err());
        invalid.font_size = f32::NAN;
        assert!(invalid.validate().is_err());

        // Margin bounds: 0.0..=144.0
        let mut invalid = base_marks.clone();
        invalid.margin_pt = -0.1;
        assert!(invalid.validate().is_err());
        invalid.margin_pt = 144.1;
        assert!(invalid.validate().is_err());
        invalid.margin_pt = f32::NAN;
        assert!(invalid.validate().is_err());

        // Four page number formats are allowed
        for format in [
            PageNumberFormat::Number,
            PageNumberFormat::Page,
            PageNumberFormat::PageOfTotal,
            PageNumberFormat::Custom {
                prefix: "Page ".to_owned(),
                suffix: " of end".to_owned(),
            },
        ] {
            let mut marks = base_marks.clone();
            marks.page_number = Some(PageNumberMark {
                format,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            });
            assert!(marks.validate().is_ok());
        }

        // Custom prefix/suffix limits
        let mut invalid = base_marks.clone();
        invalid.page_number = Some(PageNumberMark {
            format: PageNumberFormat::Custom {
                prefix: "a".repeat(MAX_PAGE_MARK_CHARS + 1),
                suffix: String::new(),
            },
            start: 1,
            position: PageMarkPosition::BottomCenter,
        });
        assert!(invalid.validate().is_err());

        let mut invalid = base_marks.clone();
        invalid.page_number = Some(PageNumberMark {
            format: PageNumberFormat::Custom {
                prefix: "bad\nprefix".to_owned(),
                suffix: String::new(),
            },
            start: 1,
            position: PageMarkPosition::BottomCenter,
        });
        assert!(invalid.validate().is_err());

        // Bates digits: 1..=12
        let mut invalid = base_marks.clone();
        invalid.bates = Some(BatesMark {
            prefix: "DOC-".to_owned(),
            digits: 0,
            start: 1,
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        let mut invalid = base_marks.clone();
        invalid.bates = Some(BatesMark {
            prefix: "DOC-".to_owned(),
            digits: 13,
            start: 1,
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        // Bates prefix max 32 chars and plain text
        let mut invalid = base_marks.clone();
        invalid.bates = Some(BatesMark {
            prefix: "x".repeat(MAX_BATES_PREFIX_CHARS + 1),
            digits: 6,
            start: 1,
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        let mut invalid = base_marks.clone();
        invalid.bates = Some(BatesMark {
            prefix: "bad\rbates".to_owned(),
            digits: 6,
            start: 1,
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        // Bates start out of range
        let mut invalid = base_marks.clone();
        invalid.bates = Some(BatesMark {
            prefix: "DOC-".to_owned(),
            digits: 2,
            start: 100, // 2 digits can only hold 0..99
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        // Bates overflow with known Custom pages
        let mut invalid = base_marks.clone();
        invalid.pages = PageMarkPages::Custom {
            pages: vec![0, 1], // 2 pages
        };
        invalid.bates = Some(BatesMark {
            prefix: "DOC-".to_owned(),
            digits: 1,
            start: 9, // 9 + 2 - 1 = 10 >= 10
            position: PageMarkPosition::BottomRight,
        });
        assert!(invalid.validate().is_err());

        // Valid Bates at limit: start 8, 2 pages -> 8, 9 (ok)
        let mut valid_bates = invalid.clone();
        valid_bates.bates = Some(BatesMark {
            prefix: "DOC-".to_owned(),
            digits: 1,
            start: 8,
            position: PageMarkPosition::BottomRight,
        });
        assert!(valid_bates.validate().is_ok());

        // Header and footer slot rules
        let mut hf_marks = base_marks.clone();
        hf_marks.page_number = None;
        hf_marks.header_footer = Some(HeaderFooterMarks {
            header_left: Some("CONFIDENTIAL".to_owned()),
            header_center: None,
            header_right: Some("DRAFT".to_owned()),
            footer_left: None,
            footer_center: None,
            footer_right: None,
        });
        assert!(hf_marks.validate().is_ok());

        // Header text empty or too long
        let mut invalid = hf_marks.clone();
        invalid.header_footer = Some(HeaderFooterMarks {
            header_left: Some("   ".to_owned()),
            ..Default::default()
        });
        assert!(invalid.validate().is_err());

        let mut invalid = hf_marks.clone();
        invalid.header_footer = Some(HeaderFooterMarks {
            header_left: Some("a".repeat(MAX_PAGE_MARK_CHARS + 1)),
            ..Default::default()
        });
        assert!(invalid.validate().is_err());

        let mut invalid = hf_marks.clone();
        invalid.header_footer = Some(HeaderFooterMarks {
            header_left: Some("line1\nline2".to_owned()),
            ..Default::default()
        });
        assert!(invalid.validate().is_err());

        // Same slot collision: page_number and footer_center both use BottomCenter
        let mut collision = base_marks.clone();
        collision.header_footer = Some(HeaderFooterMarks {
            footer_center: Some("FOOTER".to_owned()),
            ..Default::default()
        });
        assert!(collision.validate().is_err());

        // Same slot collision: bates and page_number both use BottomCenter
        let mut collision = base_marks.clone();
        collision.bates = Some(BatesMark {
            prefix: "B-".to_owned(),
            digits: 4,
            start: 1,
            position: PageMarkPosition::BottomCenter,
        });
        assert!(collision.validate().is_err());

        // Same slot collision: bates and header_right both use TopRight
        let mut collision = base_marks.clone();
        collision.page_number = None;
        collision.bates = Some(BatesMark {
            prefix: "B-".to_owned(),
            digits: 4,
            start: 1,
            position: PageMarkPosition::TopRight,
        });
        collision.header_footer = Some(HeaderFooterMarks {
            header_right: Some("HEADER RIGHT".to_owned()),
            ..Default::default()
        });
        assert!(collision.validate().is_err());

        // Pages: Custom validation
        let mut invalid = base_marks.clone();
        invalid.pages = PageMarkPages::Custom { pages: vec![] };
        assert!(invalid.validate().is_err());

        let mut invalid = base_marks.clone();
        invalid.pages = PageMarkPages::Custom {
            pages: vec![1, 1], // repeat
        };
        assert!(invalid.validate().is_err());

        let mut invalid = base_marks.clone();
        invalid.pages = PageMarkPages::Custom {
            pages: vec![2, 1], // not ascending
        };
        assert!(invalid.validate().is_err());

        // No marks at all
        let mut empty = base_marks.clone();
        empty.page_number = None;
        empty.bates = None;
        empty.header_footer = None;
        assert!(empty.validate().is_err());

        // WorkerPageMarks validates identically
        let worker_marks: WorkerPageMarks = base_marks.clone().into();
        assert!(worker_marks.validate().is_ok());

        // PageMarksCopyArgs validates
        let args = PageMarksCopyArgs {
            doc: DocumentId(1),
            marks: base_marks,
        };
        assert!(args.validate().is_ok());
    }

    #[test]
    fn an_index_search_needs_a_query_of_bounded_text() {
        let args = |query: &str| IndexSearchArgs {
            request: RequestId(1),
            query: query.to_owned(),
            case_sensitive: false,
        };
        assert!(args("harbour bridge").validate().is_ok());
        assert!(args("").validate().is_err());
        assert!(
            args(&"x".repeat(MAX_QUERY_BYTES as usize + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn an_index_snippet_is_plain_text_of_bounded_length() {
        let snippet = |before: &str, matched: &str, after: &str| IndexSnippet {
            before: before.to_owned(),
            matched: matched.to_owned(),
            after: after.to_owned(),
        };
        let max = MAX_INDEX_SNIPPET_CHARS as usize;
        assert!(snippet("the ", "harbour", " bridge").validate().is_ok());
        assert!(snippet("", "harbour", "").validate().is_ok());
        assert!(
            snippet(&"字".repeat(max), "x", &"字".repeat(max))
                .validate()
                .is_ok()
        );
        assert!(snippet("the ", "", "").validate().is_err());
        assert!(snippet("a\nb ", "x", "").validate().is_err());
        assert!(snippet("a  b ", "x", "").validate().is_err());
        assert!(snippet("", "x", "\u{202E}").validate().is_err());
        assert!(snippet(&"字".repeat(max + 1), "x", "").validate().is_err());
        assert!(snippet("", "x", &"a".repeat(max + 1)).validate().is_err());
        assert!(
            snippet("", &"x".repeat(MAX_QUERY_BYTES as usize + 1), "")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn an_index_result_has_a_file_name_and_a_bounded_list_of_pages_in_order() {
        let page = |page_index, hits| IndexPageHits {
            page_index,
            hits,
            snippet: IndexSnippet {
                before: "the ".to_owned(),
                matched: "harbour".to_owned(),
                after: " bridge".to_owned(),
            },
        };
        let found =
            |pages: Vec<IndexPageHits>, total_hits, pages_with_hits| IndexSearchEvent::Found {
                entry: IndexEntryId(1),
                display_name: "report.pdf".to_owned(),
                total_hits,
                pages,
                pages_with_hits,
                partial: false,
            };
        assert!(found(vec![page(0, 1), page(3, 2)], 3, 2).validate().is_ok());
        // More hits and pages than listed.
        assert!(found(vec![page(0, 1), page(3, 2)], 9, 5).validate().is_ok());
        let done = IndexSearchEvent::Done {
            documents: 0,
            found: 0,
        };
        assert!(done.validate().is_ok());

        assert!(found(vec![], 1, 1).validate().is_err());
        assert!(
            found(vec![page(3, 1), page(0, 1)], 2, 2)
                .validate()
                .is_err()
        );
        assert!(
            found(vec![page(0, 1), page(0, 1)], 2, 2)
                .validate()
                .is_err()
        );
        assert!(found(vec![page(0, 0)], 1, 1).validate().is_err());
        assert!(found(vec![page(0, 2)], 1, 1).validate().is_err());
        assert!(
            found(vec![page(0, 1), page(1, 1)], 2, 1)
                .validate()
                .is_err()
        );
        assert!(
            found(vec![page(0, 1)], MAX_SEARCH_HITS + 1, 1)
                .validate()
                .is_err()
        );
        assert!(
            found(vec![page(MAX_PAGE_COUNT, 1)], 1, 1)
                .validate()
                .is_err()
        );
        let many = (0..=MAX_INDEX_PAGES_PER_DOCUMENT)
            .map(|page_index| page(page_index, 1))
            .collect();
        let pages = MAX_INDEX_PAGES_PER_DOCUMENT + 1;
        assert!(found(many, pages, pages).validate().is_err());

        // A path is not a file name.
        let mut path = found(vec![page(0, 1)], 1, 1);
        if let IndexSearchEvent::Found { display_name, .. } = &mut path {
            *display_name = r"C:\Users\someone\report.pdf".to_owned();
        }
        assert!(path.validate().is_err());
    }
}
