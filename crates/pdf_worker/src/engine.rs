//! Safe wrapper around MuPDF for opening PDFs and rendering pages (MVP-03).
//!
//! MuPDF is built without its JavaScript engine (`FZ_ENABLE_JS=0`, see
//! docs/architecture/mupdf-binding.md), so no PDF script can run even if something asked for it.
//! Nothing in this crate may call `PdfDocument::enable_js`.

use std::collections::HashSet;

use ipc_contract::types::{BlockedAction, Point, Quad, SecurityReport};
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};
use mupdf::{Colorspace, Document, Matrix, Page, TextPageFlags};

use crate::scan::{self, ScanBudget};
use crate::search::{PageSearch, PageText};
use thiserror::Error;

/// PDF files must start with this header within the first 1024 bytes (PDF 1.7, 7.5.2).
const PDF_HEADER: &[u8] = b"%PDF-";
const HEADER_SEARCH_BYTES: usize = 1024;

/// Render scale bounds (mirrors `ipc_contract::limits`).
pub const MIN_RENDER_SCALE: f32 = 0.01;
pub const MAX_RENDER_SCALE: f32 = 64.0;

/// Largest raster the engine produces, checked before MuPDF allocates it
/// (mirrors `ipc_contract::limits::MAX_RASTER_PIXELS`).
pub const MAX_RASTER_PIXELS: u64 = 4096 * 4096;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("not a PDF document")]
    NotPdf,
    #[error("the document is encrypted")]
    Encrypted,
    #[error("page {0} does not exist")]
    PageOutOfRange(u32),
    #[error("render scale must be between {MIN_RENDER_SCALE} and {MAX_RENDER_SCALE}")]
    InvalidScale,
    #[error("rotation must be 0, 90, 180 or 270 degrees")]
    InvalidRotation,
    #[error("a {width} x {height} render exceeds the raster limit")]
    TooLarge { width: u64, height: u64 },
    #[error("MuPDF: {0}")]
    MuPdf(#[from] mupdf::Error),
}

/// A rendered page: opaque RGBA8, rows top to bottom, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Where an outline entry points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlineTarget {
    None,
    /// 0-based page number (not yet checked against the page count).
    Page(u32),
    /// A URI action; the caller decides whether it may ever be offered.
    Uri(String),
    /// An action that is never performed. `target` is raw PDF text (a file name, for example).
    Blocked {
        action: BlockedAction,
        target: Option<String>,
    },
}

/// One outline entry, in reading order (a parent before its children).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Raw title from the PDF; the caller cleans it before showing it.
    pub title: String,
    pub depth: u16,
    pub target: OutlineTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentOutline {
    pub entries: Vec<OutlineEntry>,
    /// Entries beyond `max_items` or deeper than `max_depth` were left out.
    pub truncated: bool,
}

/// A link annotation: where it is on the page and where it points.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLinkEntry {
    /// Page space (points, origin top left of the page as displayed at 0°), x0 <= x1, y0 <= y1.
    pub rect: [f32; 4],
    pub target: OutlineTarget,
}

/// An open PDF document.
pub struct PdfDocument {
    doc: MuPdfDocument,
}

impl PdfDocument {
    /// Opens a PDF from memory. Encrypted documents are refused (not supported in the MVP).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        let head = &bytes[..bytes.len().min(HEADER_SEARCH_BYTES)];
        if !head
            .windows(PDF_HEADER.len())
            .any(|window| window == PDF_HEADER)
        {
            return Err(EngineError::NotPdf);
        }
        let doc = Document::from_bytes(bytes, "application/pdf")?;
        if doc.needs_password()? {
            return Err(EngineError::Encrypted);
        }
        Ok(Self {
            doc: MuPdfDocument::try_from(doc)?,
        })
    }

    pub fn page_count(&self) -> Result<u32, EngineError> {
        Ok(u32::try_from(self.doc.page_count()?).unwrap_or(0))
    }

    /// Page size in PDF points, before any view rotation.
    pub fn page_size(&self, index: u32) -> Result<(f32, f32), EngineError> {
        let bounds = self.load_page(index)?.bounds()?;
        Ok((bounds.x1 - bounds.x0, bounds.y1 - bounds.y0))
    }

    /// Active content and remote references in the document (see [`scan`]). Nothing is run.
    pub fn active_content(&self, budget: ScanBudget) -> SecurityReport {
        match self.doc.catalog() {
            Ok(catalog) => scan::scan(catalog, budget),
            Err(_) => SecurityReport {
                findings: Vec::new(),
                scan_complete: false,
            },
        }
    }

    /// Whether the document has an outline with at least one entry (cheap: nothing is walked).
    pub fn has_outline(&self) -> bool {
        (|| -> Result<bool, mupdf::Error> {
            let Some(root) = self.doc.catalog()?.get_dict("Outlines")? else {
                return Ok(false);
            };
            Ok(root.get_dict("First")?.is_some())
        })()
        .unwrap_or(false)
    }

    /// The outline (table of contents), flattened in reading order, with at most `max_items`
    /// entries no deeper than `max_depth` (0 = top level).
    ///
    /// Walks the outline objects itself instead of using MuPDF's outline loader, which recurses
    /// once per nesting level and rejects the whole outline when a single destination is bad.
    /// Here the walk uses an explicit stack, a node reached a second time (a cycle) ends that
    /// branch, and a broken entry only loses its own target.
    pub fn outline(
        &self,
        max_items: usize,
        max_depth: u16,
    ) -> Result<DocumentOutline, EngineError> {
        let mut outline = DocumentOutline::default();
        let Some(root) = self.doc.catalog()?.get_dict("Outlines")? else {
            return Ok(outline);
        };
        let mut stack: Vec<(PdfObject, u16)> = Vec::new();
        if let Some(first) = root.get_dict("First")? {
            stack.push((first, 0));
        }
        let mut seen = HashSet::new();
        while let Some((node, depth)) = stack.pop() {
            if node.is_indirect()? && !seen.insert(node.as_indirect()?) {
                outline.truncated = true;
                continue;
            }
            if !node.is_dict()? {
                continue;
            }
            if outline.entries.len() == max_items {
                outline.truncated = true;
                break;
            }
            let title = text(node.get_dict("Title")).unwrap_or_default();
            let target = self.action_target(&node);
            outline.entries.push(OutlineEntry {
                title,
                depth,
                target,
            });
            // The next sibling waits until this entry's children are done (reading order).
            if let Ok(Some(next)) = node.get_dict("Next") {
                stack.push((next, depth));
            }
            if let Ok(Some(first)) = node.get_dict("First") {
                if depth < max_depth {
                    stack.push((first, depth + 1));
                } else {
                    outline.truncated = true;
                }
            }
        }
        Ok(outline)
    }

    /// Where an outline item or a link annotation points: its /Dest, or else its /A action.
    fn action_target(&self, node: &PdfObject) -> OutlineTarget {
        if let Ok(Some(dest)) = node.get_dict("Dest") {
            return self.destination(&dest);
        }
        let Ok(Some(action)) = node.get_dict("A") else {
            return OutlineTarget::None;
        };
        let kind = action
            .get_dict("S")
            .ok()
            .flatten()
            .and_then(|name| name.as_name().ok())
            .unwrap_or_default();
        let blocked = |action_kind: BlockedAction, target: Option<String>| OutlineTarget::Blocked {
            action: action_kind,
            target,
        };
        match kind.as_slice() {
            b"GoTo" => match action.get_dict("D") {
                Ok(Some(dest)) => self.destination(&dest),
                _ => OutlineTarget::None,
            },
            b"URI" => {
                uri_text(action.get_dict("URI")).map_or(OutlineTarget::None, OutlineTarget::Uri)
            }
            b"Launch" => blocked(BlockedAction::Launch, file_name(&action)),
            b"GoToR" => blocked(BlockedAction::RemoteGoTo, file_name(&action)),
            b"GoToE" => blocked(BlockedAction::EmbeddedGoTo, None),
            // Never the script itself: it is not something to show.
            b"JavaScript" => blocked(BlockedAction::JavaScript, None),
            b"SubmitForm" => blocked(BlockedAction::SubmitForm, file_name(&action)),
            b"ImportData" => blocked(BlockedAction::ImportData, file_name(&action)),
            // Viewer navigation such as NextPage: harmless, but not a place to jump to.
            b"Named" => OutlineTarget::None,
            _ => blocked(BlockedAction::Other, None),
        }
    }

    /// A page for an explicit destination ([page /XYZ ...], where the page is a page object or,
    /// in some files, a number) or a named one; anything that does not resolve is no target.
    fn destination(&self, dest: &PdfObject) -> OutlineTarget {
        let page = || -> Result<Option<u32>, mupdf::Error> {
            if dest.is_array()? {
                let Some(first) = dest.get_array(0)? else {
                    return Ok(None);
                };
                let number = if first.is_int()? {
                    first.as_int()?
                } else {
                    self.doc.lookup_page_number(&first)?
                };
                return Ok(u32::try_from(number).ok());
            }
            if dest.is_name()? || dest.is_string()? {
                let name = if dest.is_name()? {
                    String::from_utf8_lossy(&dest.as_name()?).into_owned()
                } else {
                    dest.as_string()?
                };
                let uri = format!("#nameddest={}", percent_encode(&name));
                return Ok(self
                    .doc
                    .resolve_link(&uri)?
                    .map(|link| link.loc.page_number));
            }
            Ok(None)
        };
        match page() {
            Ok(Some(number)) => OutlineTarget::Page(number),
            _ => OutlineTarget::None,
        }
    }

    /// The link annotations of page `index`, in the order of its /Annots, at most `max`.
    ///
    /// Reads the annotations itself rather than using MuPDF's link list, which turns actions
    /// into URIs (a Launch becomes a `file:` link) and so loses what kind of action it was.
    /// An annotation that cannot be read is skipped.
    pub fn page_links(&self, index: u32, max: usize) -> Result<Vec<PageLinkEntry>, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        let page = self
            .doc
            .find_page(i32::try_from(index).unwrap_or(i32::MAX))?;
        let ctm = page.page_ctm()?;
        let Some(annots) = page.get_dict("Annots")? else {
            return Ok(Vec::new());
        };
        if !annots.is_array()? {
            return Ok(Vec::new());
        }
        let mut links = Vec::new();
        for annot in annots.array_iter()? {
            if links.len() == max {
                break;
            }
            let Ok(annot) = annot else { continue };
            if let Ok(Some(link)) = self.link_entry(&annot, &ctm) {
                links.push(link);
            }
        }
        Ok(links)
    }

    fn link_entry(
        &self,
        annot: &PdfObject,
        ctm: &Matrix,
    ) -> Result<Option<PageLinkEntry>, mupdf::Error> {
        let subtype = annot.get_dict("Subtype")?;
        if !annot.is_dict()?
            || subtype.map(|name| name.as_name()).transpose()?.as_deref() != Some(b"Link")
        {
            return Ok(None);
        }
        let Some(rect) = annot.get_dict("Rect")? else {
            return Ok(None);
        };
        let mut numbers = [0.0f32; 4];
        for (slot, value) in numbers.iter_mut().zip(rect.array_iter()?) {
            let value = value?;
            if !value.is_number()? {
                return Ok(None);
            }
            *slot = value.as_float()?;
        }
        if rect.len()? != 4 || numbers.iter().any(|value| !value.is_finite()) {
            return Ok(None);
        }
        // The rectangle is in PDF user space (origin bottom left); map its corners to page space,
        // which also applies the page's /Rotate and crop box.
        let [x0, y0, x1, y1] = numbers;
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| ctm.transform_xy(x, y));
        let xs = corners.map(|(x, _)| x);
        let ys = corners.map(|(_, y)| y);
        let min = |values: [f32; 4]| values.into_iter().fold(f32::INFINITY, f32::min);
        let max = |values: [f32; 4]| values.into_iter().fold(f32::NEG_INFINITY, f32::max);
        Ok(Some(PageLinkEntry {
            rect: [min(xs), min(ys), max(xs), max(ys)],
            target: self.action_target(annot),
        }))
    }

    /// Renders a page at `scale` (1.0 = 72 dpi) rotated clockwise by `rotation` degrees.
    pub fn render(
        &self,
        index: u32,
        scale: f32,
        rotation: u16,
    ) -> Result<RenderedPage, EngineError> {
        if !scale.is_finite() || !(MIN_RENDER_SCALE..=MAX_RENDER_SCALE).contains(&scale) {
            return Err(EngineError::InvalidScale);
        }
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err(EngineError::InvalidRotation);
        }
        let page = self.load_page(index)?;

        // Reject oversized renders before MuPDF allocates the pixmap.
        let bounds = page.bounds()?;
        let to_px = |points: f32| (f64::from(points) * f64::from(scale)).ceil().max(1.0) as u64;
        let (mut width, mut height) = (to_px(bounds.x1 - bounds.x0), to_px(bounds.y1 - bounds.y0));
        if rotation % 180 == 90 {
            std::mem::swap(&mut width, &mut height);
        }
        if width.saturating_mul(height) > MAX_RASTER_PIXELS {
            return Err(EngineError::TooLarge { width, height });
        }

        let mut ctm = Matrix::new_scale(scale, scale);
        ctm.concat(Matrix::new_rotate(f32::from(rotation)));
        // No alpha: MuPDF paints an opaque white background, which the contract requires.
        let pixmap = page.to_pixmap(&ctm, &Colorspace::device_rgb(), false, false)?;

        let (width, height) = (pixmap.width(), pixmap.height());
        let channels = usize::from(pixmap.n());
        let stride = usize::try_from(pixmap.stride()).unwrap_or(0);
        let row_bytes = width as usize * channels;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in pixmap.samples().chunks_exact(stride.max(1)) {
            for pixel in row[..row_bytes].chunks_exact(channels) {
                rgba.extend_from_slice(&pixel[..3]);
                rgba.push(255);
            }
        }
        Ok(RenderedPage {
            width,
            height,
            rgba,
        })
    }

    /// Searches one page's text layer for `query` (MVP-10); coordinates are page points with
    /// the origin at the top left, like the page size.
    pub fn search_page(
        &self,
        index: u32,
        query: &str,
        case_sensitive: bool,
        max_hits: usize,
    ) -> Result<PageSearch, EngineError> {
        let text_page = self
            .load_page(index)?
            .to_text_page(TextPageFlags::empty())?;
        let mut text = PageText::new(case_sensitive);
        for block in text_page.blocks() {
            for line in block.lines() {
                text.push_line(
                    line.chars()
                        .filter_map(|ch| ch.char().map(|c| (c, contract_quad(ch.quad())))),
                );
            }
        }
        Ok(text.search(query, max_hits))
    }

    fn load_page(&self, index: u32) -> Result<Page, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self.doc.load_page(index as i32)?)
    }
}

fn contract_quad(quad: mupdf::Quad) -> Quad {
    let point = |p: mupdf::Point| Point { x: p.x, y: p.y };
    Quad {
        ul: point(quad.ul),
        ur: point(quad.ur),
        ll: point(quad.ll),
        lr: point(quad.lr),
    }
}

/// A PDF text string as UTF-8, or nothing if the value is missing or not a string.
fn text(value: Result<Option<PdfObject>, mupdf::Error>) -> Option<String> {
    let value = value.ok()??;
    if !value.is_string().ok()? {
        return None;
    }
    value.as_string().ok()
}

/// The file an action refers to (/F as a string, or a file specification with /UF or /F).
/// A URI string. PDF says it is ASCII, but IRIs are commonly stored as raw UTF-8, which the
/// PDFDocEncoding of ordinary text strings would garble (and so hide a look-alike host name).
/// UTF-16 with a byte order mark and valid UTF-8 are decoded as such; other bytes one to one.
fn uri_text(value: Result<Option<PdfObject>, mupdf::Error>) -> Option<String> {
    let value = value.ok()??;
    if !value.is_string().ok()? {
        return None;
    }
    let bytes = value.as_bytes().ok()?;
    Some(match bytes.as_slice() {
        [0xfe, 0xff, rest @ ..] => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        }
        raw => match std::str::from_utf8(raw) {
            Ok(utf8) => utf8.to_owned(),
            Err(_) => raw.iter().map(|&byte| char::from(byte)).collect(),
        },
    })
}

fn file_name(action: &PdfObject) -> Option<String> {
    let spec = action.get_dict("F").ok()??;
    if spec.is_dict().ok()? {
        return text(spec.get_dict("UF")).or_else(|| text(spec.get_dict("F")));
    }
    text(Ok(Some(spec)))
}

/// Percent-encodes a destination name for MuPDF's `#nameddest=` link syntax.
fn percent_encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PDF with a correct xref table from object bodies (object n = body n-1).
    fn build_pdf(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn stream(content: &str) -> String {
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
    }

    /// Two Letter pages. Page 1: black square at (100,100)-(300,300) and Helvetica text near the top.
    fn two_page_pdf() -> Vec<u8> {
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>",
            &stream("0 0 0 rg 100 100 200 200 re f\nBT /F1 36 Tf 72 700 Td (Hello MuPDF) Tj ET"),
            &stream(""),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ])
    }

    /// A PDF whose outline is given as (title, depth, target page) in reading order; object
    /// numbers: 1 catalog, 2 pages, 3..3+pages page objects, then the outline root and items.
    fn outline_pdf(pages: usize, items: &[(&str, u16, Option<usize>)]) -> Vec<u8> {
        let first_page = 3;
        let root = first_page + pages;
        let item_obj = |index: usize| root + 1 + index;
        let kids: Vec<String> = (0..pages)
            .map(|p| format!("{} 0 R", first_page + p))
            .collect();
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R /Outlines {root} 0 R >>"),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                kids.join(" ")
            ),
        ];
        objects.extend(
            (0..pages)
                .map(|_| "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned()),
        );
        // Parent, first child, last child, previous and next sibling of every item.
        let parent = |index: usize| -> Option<usize> {
            (0..index)
                .rev()
                .find(|&before| items[before].1 < items[index].1)
        };
        let children = |of: Option<usize>| -> Vec<usize> {
            (0..items.len())
                .filter(|&index| parent(index) == of)
                .collect()
        };
        let top = children(None);
        objects.push(format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            item_obj(top[0]),
            item_obj(*top.last().unwrap()),
            items.len()
        ));
        for (index, (title, _, page)) in items.iter().enumerate() {
            let parent_obj = parent(index).map_or(root, item_obj);
            let siblings = children(parent(index));
            let at = siblings
                .iter()
                .position(|&sibling| sibling == index)
                .unwrap();
            let mut dict = format!("<< /Title {title} /Parent {parent_obj} 0 R");
            if at > 0 {
                dict += &format!(" /Prev {} 0 R", item_obj(siblings[at - 1]));
            }
            if at + 1 < siblings.len() {
                dict += &format!(" /Next {} 0 R", item_obj(siblings[at + 1]));
            }
            let kids = children(Some(index));
            if let (Some(first), Some(last)) = (kids.first(), kids.last()) {
                dict += &format!(
                    " /First {} 0 R /Last {} 0 R /Count {}",
                    item_obj(*first),
                    item_obj(*last),
                    kids.len()
                );
            }
            if let Some(page) = page {
                dict += &format!(" /Dest [{} 0 R /XYZ 0 792 0]", first_page + page);
            }
            objects.push(dict + " >>");
        }
        let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
        build_pdf(&refs)
    }

    #[test]
    fn reads_a_three_level_outline_in_reading_order() {
        let pdf = outline_pdf(
            6,
            &[
                ("(Chapter 1)", 0, Some(0)),
                ("(Section 1.1)", 1, Some(1)),
                ("(Subsection 1.1.1)", 2, Some(2)),
                ("(Chapter 2)", 0, Some(3)),
                ("(Section 2.1)", 1, Some(4)),
                ("(Appendix)", 0, Some(5)),
            ],
        );
        let outline = PdfDocument::from_bytes(&pdf)
            .unwrap()
            .outline(100, 64)
            .unwrap();
        let flat: Vec<(&str, u16, OutlineTarget)> = outline
            .entries
            .iter()
            .map(|entry| (entry.title.as_str(), entry.depth, entry.target.clone()))
            .collect();
        assert_eq!(
            flat,
            [
                ("Chapter 1", 0, OutlineTarget::Page(0)),
                ("Section 1.1", 1, OutlineTarget::Page(1)),
                ("Subsection 1.1.1", 2, OutlineTarget::Page(2)),
                ("Chapter 2", 0, OutlineTarget::Page(3)),
                ("Section 2.1", 1, OutlineTarget::Page(4)),
                ("Appendix", 0, OutlineTarget::Page(5)),
            ]
        );
        assert!(!outline.truncated);
    }

    #[test]
    fn searches_the_text_layer() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let found = doc.search_page(0, "mupdf", false, 10).unwrap();
        assert!(found.has_text);
        assert_eq!(found.hits.len(), 1);
        // "Hello MuPDF" is drawn at y = 700 from the bottom: near the top of the page.
        let quad = found.hits[0].quads[0];
        assert!(quad.ul.y > 40.0 && quad.ll.y < 120.0, "{quad:?}");
        assert!(quad.ul.x > 150.0 && quad.ur.x < 400.0, "{quad:?}");

        assert!(
            doc.search_page(0, "mupdf", true, 10)
                .unwrap()
                .hits
                .is_empty()
        );
        assert_eq!(doc.search_page(0, "MuPDF", true, 10).unwrap().hits.len(), 1);
        let empty_page = doc.search_page(1, "mupdf", false, 10).unwrap();
        assert!(!empty_page.has_text && empty_page.hits.is_empty());
        assert!(matches!(
            doc.search_page(2, "x", false, 10),
            Err(EngineError::PageOutOfRange(2))
        ));
    }

    #[test]
    fn searches_chinese_text_from_the_to_unicode_map() {
        // Helvetica draws "AB", but the ToUnicode map says the text is 隱私 (U+96B1 U+79C1):
        // search reads the text layer, not the glyphs, and needs no CJK font.
        let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap
                    /CMapName /Test-UCS def /CMapType 2 def
                    1 begincodespacerange <00> <FF> endcodespacerange
                    2 beginbfchar <41> <96B1> <42> <79C1> endbfchar
                    endcmap CMapName currentdict /CMap defineresource pop end end";
        let content = "BT /F1 24 Tf 72 700 Td (AB) Tj ET";
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            &stream(content),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
            &stream(cmap),
        ]);
        let doc = PdfDocument::from_bytes(&pdf).unwrap();
        assert_eq!(doc.search_page(0, "隱私", false, 10).unwrap().hits.len(), 1);
        assert_eq!(doc.search_page(0, "私", false, 10).unwrap().hits.len(), 1);
        assert!(doc.search_page(0, "AB", false, 10).unwrap().hits.is_empty());
    }

    #[test]
    fn knows_whether_there_is_an_outline() {
        assert!(
            !PdfDocument::from_bytes(&two_page_pdf())
                .unwrap()
                .has_outline()
        );
        let pdf = outline_pdf(1, &[("(Only)", 0, Some(0))]);
        assert!(PdfDocument::from_bytes(&pdf).unwrap().has_outline());
    }

    #[test]
    fn a_document_without_outline_has_no_entries() {
        let outline = PdfDocument::from_bytes(&two_page_pdf())
            .unwrap()
            .outline(100, 64)
            .unwrap();
        assert_eq!(outline, DocumentOutline::default());
    }

    #[test]
    fn stops_at_the_item_and_depth_limits() {
        let items: Vec<(String, u16, Option<usize>)> = (0..50)
            .map(|index| (format!("(Item {index})"), 0, Some(0)))
            .collect();
        let refs: Vec<(&str, u16, Option<usize>)> = items
            .iter()
            .map(|(title, depth, page)| (title.as_str(), *depth, *page))
            .collect();
        let outline = PdfDocument::from_bytes(&outline_pdf(1, &refs))
            .unwrap()
            .outline(10, 64)
            .unwrap();
        assert_eq!(outline.entries.len(), 10);
        assert!(outline.truncated);

        let nested = outline_pdf(
            1,
            &[
                ("(A)", 0, None),
                ("(B)", 1, None),
                ("(C)", 2, None),
                ("(D)", 0, None),
            ],
        );
        let outline = PdfDocument::from_bytes(&nested)
            .unwrap()
            .outline(100, 1)
            .unwrap();
        let titles: Vec<&str> = outline
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert_eq!(titles, ["A", "B", "D"]);
        assert!(outline.truncated);
    }

    #[test]
    fn a_cycle_in_the_outline_is_cut() {
        // B's /Next points back at A (A -> B -> A -> ...), and A.1's /First at its own parent.
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>",
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /First 7 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
            "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Next 5 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
            "<< /Title (A.1) /Parent 5 0 R /First 5 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
        ]);
        let outline = PdfDocument::from_bytes(&pdf)
            .unwrap()
            .outline(10_000, 64)
            .unwrap();
        let titles: Vec<&str> = outline
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert_eq!(titles, ["A", "A.1", "B"]);
        assert!(outline.truncated);
    }

    #[test]
    fn actions_are_recognised_and_never_resolved_to_pages() {
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /Names << /Dests 12 0 R >> >>",
            "<< /Type /Pages /Kids [3 0 R 13 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 5 0 R /Last 11 0 R /Count 7 >>",
            "<< /Title (Web) /Parent 4 0 R /Next 6 0 R /A << /S /URI /URI (https://example.invalid/) >> >>",
            "<< /Title (Run) /Parent 4 0 R /Prev 5 0 R /Next 7 0 R /A << /S /Launch /F (calc.exe) >> >>",
            "<< /Title (Other) /Parent 4 0 R /Prev 6 0 R /Next 8 0 R /A << /S /GoToR /F << /Type /Filespec /UF (other.pdf) >> /D [0 /Fit] >> >>",
            "<< /Title (Script) /Parent 4 0 R /Prev 7 0 R /Next 9 0 R /A << /S /JavaScript /JS (app.alert\\(1\\)) >> >>",
            "<< /Title (Bad page) /Parent 4 0 R /Prev 8 0 R /Next 10 0 R /Dest [99 /Fit] >>",
            "<< /Title (Named) /Parent 4 0 R /Prev 9 0 R /Next 11 0 R /Dest (second) >>",
            "<< /Title (GoTo) /Parent 4 0 R /Prev 10 0 R /A << /S /GoTo /D [13 0 R /Fit] >> >>",
            "<< /Names [(second) [13 0 R /XYZ 0 792 0]] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]);
        let outline = PdfDocument::from_bytes(&pdf)
            .unwrap()
            .outline(10, 64)
            .unwrap();
        let targets: Vec<OutlineTarget> = outline
            .entries
            .iter()
            .map(|entry| entry.target.clone())
            .collect();
        assert_eq!(
            targets,
            [
                OutlineTarget::Uri("https://example.invalid/".to_owned()),
                OutlineTarget::Blocked {
                    action: BlockedAction::Launch,
                    target: Some("calc.exe".to_owned())
                },
                OutlineTarget::Blocked {
                    action: BlockedAction::RemoteGoTo,
                    target: Some("other.pdf".to_owned())
                },
                OutlineTarget::Blocked {
                    action: BlockedAction::JavaScript,
                    target: None
                },
                // Page 100 of 2: the caller drops it against the page count.
                OutlineTarget::Page(99),
                OutlineTarget::Page(1),
                OutlineTarget::Page(1),
            ]
        );
    }

    fn pixel(page: &RenderedPage, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * page.width + x) * 4) as usize;
        page.rgba[at..at + 4].try_into().unwrap()
    }

    fn dark_pixels(page: &RenderedPage, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(page, x, y)[0] < 128)
            .count()
    }

    #[test]
    fn reports_pages_and_sizes() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        assert_eq!(doc.page_count().unwrap(), 2);
        assert_eq!(doc.page_size(0).unwrap(), (612.0, 792.0));
    }

    #[test]
    fn renders_opaque_rgba_with_graphics_and_text() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let page = doc.render(0, 1.0, 0).unwrap();

        assert_eq!((page.width, page.height), (612, 792));
        assert_eq!(page.rgba.len(), 612 * 792 * 4);
        let (pixels, rest) = page.rgba.as_chunks::<4>();
        assert!(rest.is_empty());
        assert!(pixels.iter().all(|px| px[3] == 255), "must be opaque");
        // Square: PDF y=200 is 792-200=592 from the top.
        assert_eq!(pixel(&page, 200, 592), [0, 0, 0, 255]);
        assert_eq!(pixel(&page, 10, 10), [255, 255, 255, 255]);
        // Text drawn with the bundled base-14 Helvetica (baseline at y=700 -> 92 from top).
        assert!(
            dark_pixels(&page, 72, 60, 320, 95) > 200,
            "Helvetica text must render"
        );
        // The blank page is entirely white.
        let blank = doc.render(1, 0.5, 0).unwrap();
        assert!(
            blank
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [255; 4])
        );
    }

    #[test]
    fn scales_and_rotates() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let scaled = doc.render(0, 2.0, 0).unwrap();
        assert_eq!((scaled.width, scaled.height), (1224, 1584));

        let rotated = doc.render(0, 1.0, 90).unwrap();
        assert_eq!((rotated.width, rotated.height), (792, 612));
        // Clockwise quarter turn of a W x H image maps (x, y) to (H - y, x): the square's centre
        // (200, 592) lands on (200, 200); a counter-clockwise turn would put it at (592, 412).
        assert_eq!(pixel(&rotated, 200, 200), [0, 0, 0, 255]);
        assert_eq!(pixel(&rotated, 592, 412), [255, 255, 255, 255]);
    }

    #[test]
    fn rejects_bad_render_requests_before_rendering() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        assert!(matches!(
            doc.render(2, 1.0, 0),
            Err(EngineError::PageOutOfRange(2))
        ));
        assert!(matches!(
            doc.render(0, 0.0, 0),
            Err(EngineError::InvalidScale)
        ));
        assert!(matches!(
            doc.render(0, f32::NAN, 0),
            Err(EngineError::InvalidScale)
        ));
        assert!(matches!(
            doc.render(0, 1.0, 45),
            Err(EngineError::InvalidRotation)
        ));
        assert!(matches!(
            doc.render(0, 64.0, 0),
            Err(EngineError::TooLarge { .. })
        ));
    }

    #[test]
    fn huge_media_box_is_rejected_instead_of_allocated() {
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000000000 1000000000] >>",
        ]);
        let doc = PdfDocument::from_bytes(&pdf).unwrap();
        assert!(matches!(
            doc.render(0, 1.0, 0),
            Err(EngineError::TooLarge { .. })
        ));
    }

    #[test]
    fn non_pdf_input_is_rejected() {
        assert!(matches!(
            PdfDocument::from_bytes(b""),
            Err(EngineError::NotPdf)
        ));
        assert!(matches!(
            PdfDocument::from_bytes(b"This is a plain text file with a .pdf extension.\n"),
            Err(EngineError::NotPdf)
        ));
    }

    #[test]
    fn garbage_after_the_header_does_not_panic() {
        let result = PdfDocument::from_bytes(b"%PDF-1.7\n\x00\xff garbage without objects");
        if let Ok(doc) = result {
            assert!(doc.page_count().is_ok());
        }
    }

    /// The JavaScript engine is compiled out: even an explicit request cannot enable it.
    /// (Test-only call; production code must never call `enable_js`.)
    #[test]
    fn javascript_is_compiled_out() {
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (app.alert\\(1\\)) >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]);
        let mut doc = mupdf::pdf::PdfDocument::from_bytes(&pdf).unwrap();
        doc.enable_js().unwrap();
        assert!(!doc.is_js_supported().unwrap());
    }
}
