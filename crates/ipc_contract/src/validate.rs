//! Bounds checks for untrusted values.
//!
//! The main process validates every [`WorkerResponse`] before using it, and every command
//! argument coming from the frontend. Page-index checks need the document's page count and are
//! done by the caller with [`check_page_index`].

use std::collections::HashSet;

use thiserror::Error;

use crate::limits::*;
use crate::text::{classify_uri, is_clean_copy_text, is_clean_display_text};
use crate::types::{
    DocumentInfo, FindingKind, IpcError, LinkTarget, OpenEvent, OutlineItem, OutlineResult,
    PageLink, PageSize, PageText, Point, Quad, Rect, RenderPageArgs, SearchArgs, SearchHit,
    SecurityReport, TextLine, UnlockArgs,
};
use crate::worker::{OpenedDocument, Raster, WorkerError, WorkerResponse};

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
        if self.pages.is_empty() {
            return Err(ValidationError::Invalid {
                what: "document",
                reason: "has no pages",
            });
        }
        check_count("pages", self.pages.len(), MAX_PAGE_COUNT)?;
        self.pages.iter().try_for_each(PageSize::validate)?;
        self.security.validate()
    }
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
            WorkerResponse::PageText { text, .. } => text.validate(),
            WorkerResponse::PageSearched { hits, .. } => {
                check_count("search hits", hits.len(), MAX_SEARCH_HITS)?;
                hits.iter().try_for_each(SearchHit::validate)
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
        let password = self.password.as_str();
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
        BlockedAction, DocumentId, ErrorCode, LinkId, RequestId, Rotation, SecurityFinding, TabId,
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
            security: SecurityReport::default(),
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
            },
        };
        assert!(response.validate().is_err());
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
    fn display_name_must_not_be_a_path() {
        let info = |display_name: &str| DocumentInfo {
            doc: DocumentId(1),
            display_name: display_name.to_owned(),
            pages: vec![page(612.0, 792.0)],
            has_outline: false,
            security: SecurityReport::default(),
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
