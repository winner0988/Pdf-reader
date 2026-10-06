//! Safe wrapper around MuPDF for opening PDFs and rendering pages (MVP-03).
//!
//! MuPDF is built without its JavaScript engine (`FZ_ENABLE_JS=0`, see
//! docs/architecture/mupdf-binding.md), so no PDF script can run even if something asked for it.
//! Nothing in this crate may call `PdfDocument::enable_js`.

use std::collections::HashSet;
use std::io::{self, Write};

use ipc_contract::limits::{
    MAX_DOCUMENT_BYTES, MAX_JPEG_BYTES, MAX_PAGE_COUNT, MAX_PNG_BYTES, MAX_SOURCE_BYTES,
};
use ipc_contract::types::{
    BlockedAction, DocumentPermissions, PageText as TextLayer, Point, Quad, SecurityReport,
};
use mupdf::pdf::{
    Encryption, InsertPdfOptions, InsertPosition, PageSelection, PdfDocument as MuPdfDocument,
    PdfObject, PdfWriteOptions,
};
use mupdf::{Colorspace, Document, ImageFormat, Matrix, Page, Pixmap, TextPageFlags};

use crate::owner_password;
use crate::scan::{self, ScanBudget};
use crate::search::{PageSearch, PageText};
use crate::text_layer::TextLayerBuilder;
use thiserror::Error;

mod annotations;
mod forms;

/// Most form fields looked at to find a signature (`PdfDocument::is_signed`).
const MAX_FORM_FIELDS: usize = 10_000;

/// Quality of exported JPEG pages (#111), 1 to 100: text stays sharp, the file far smaller than
/// the PNG of the same page.
const JPEG_QUALITY: u8 = 90;

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
    #[error("the document needs a password")]
    Encrypted,
    #[error("the password does not open the document")]
    WrongPassword,
    #[error("the document's encryption is not supported")]
    UnsupportedEncryption,
    #[error("page {0} does not exist")]
    PageOutOfRange(u32),
    #[error("render scale must be between {MIN_RENDER_SCALE} and {MAX_RENDER_SCALE}")]
    InvalidScale,
    #[error("rotation must be 0, 90, 180 or 270 degrees")]
    InvalidRotation,
    #[error("invalid edit: {0}")]
    InvalidEdit(&'static str),
    #[error("a document keeps at least one page")]
    NoPageLeft,
    #[error("a document has at most {MAX_PAGE_COUNT} pages")]
    TooManyPages,
    #[error("a {width} x {height} render exceeds the raster limit")]
    TooLarge { width: u64, height: u64 },
    #[error("could not write the file: {0}")]
    Write(io::Error),
    #[error("could not encode the image: {0}")]
    Encode(String),
    #[error("an encrypted document has no privacy export: its copy could not be encrypted again")]
    EncryptedCopy,
    /// The author of a document forbids what was asked (MVP-19).
    #[error("the document's author does not allow this: {0}")]
    NotAllowed(&'static str),
    #[error("the document has too many objects to clean")]
    TooComplex,
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
    /// The binding's `PdfObject`s do not borrow the document they come from: none may outlive
    /// this field, so they stay local to the methods below.
    doc: MuPdfDocument,
    /// Opened with the owner password: nothing is restricted, as in Acrobat (#88).
    owner: bool,
}

impl PdfDocument {
    /// Opens a PDF from memory. Encrypted documents are refused (not supported in the MVP).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        Self::open(bytes, None)
    }

    /// Opens a PDF from memory. An encrypted one needs `password` (MVP-16): the user or the
    /// owner password, or none at all for a document whose user password is empty.
    pub fn open(bytes: &[u8], password: Option<&str>) -> Result<Self, EngineError> {
        let head = &bytes[..bytes.len().min(HEADER_SEARCH_BYTES)];
        if !head
            .windows(PDF_HEADER.len())
            .any(|window| window == PDF_HEADER)
        {
            return Err(EngineError::NotPdf);
        }
        let mut doc = Document::from_bytes(bytes, "application/pdf").map_err(open_error)?;
        let mut owner = false;
        if doc.needs_password()? {
            let password = password.ok_or(EngineError::Encrypted)?;
            if !doc.authenticate(password)? {
                return Err(EngineError::WrongPassword);
            }
            owner = owner_password::is_owner_password(bytes, password);
        }
        Ok(Self {
            doc: MuPdfDocument::try_from(doc)?,
            owner,
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

    /// What the author allows (MVP-19), from the encryption dictionary's `/P` and `/R`.
    /// MuPDF's own `permissions()` cannot be used: the binding turns any `/P` with the reserved
    /// bits set, that is every real one, into "everything allowed".
    ///
    /// As in Acrobat, whoever opened the file with the owner password has no restrictions (#88,
    /// docs/architecture/encryption.md).
    pub fn permissions(&self) -> DocumentPermissions {
        if self.owner {
            return DocumentPermissions::ALL;
        }
        let Some(encrypt) = self
            .doc
            .trailer()
            .ok()
            .and_then(|trailer| trailer.get_dict("Encrypt").ok().flatten())
        else {
            return DocumentPermissions::ALL;
        };
        let int = |key: &str| {
            encrypt
                .get_dict(key)
                .ok()
                .flatten()
                .and_then(|value| value.as_int().ok())
        };
        match int("P") {
            Some(p) => permissions_from(p, int("R")),
            None => DocumentPermissions::ALL,
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

    /// Whether the document has a form with at least one field (B2-09; cheap: nothing is walked).
    pub fn has_form(&self) -> bool {
        (|| -> Result<bool, mupdf::Error> {
            let Some(fields) = self
                .doc
                .catalog()?
                .get_dict("AcroForm")?
                .and_then(|form| form.get_dict("Fields").ok().flatten())
            else {
                return Ok(false);
            };
            Ok(fields.is_array()? && fields.len()? > 0)
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
        let pixmap = self.pixmap(index, scale, rotation)?;
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

    /// The page as a PNG file, unturned, for exporting (B2-04). MuPDF encodes it; the worker never
    /// writes a file.
    pub fn render_png(&self, index: u32, scale: f32) -> Result<Vec<u8>, EngineError> {
        let pixmap = self.pixmap(index, scale, 0)?;
        let mut png = Vec::new();
        pixmap.write_to(&mut png, ImageFormat::PNG)?;
        if png.len() > MAX_PNG_BYTES {
            return Err(EngineError::TooLarge {
                width: u64::from(pixmap.width()),
                height: u64::from(pixmap.height()),
            });
        }
        Ok(png)
    }

    /// The page as a JPEG file (quality [`JPEG_QUALITY`]), unturned, for exporting (#111). The
    /// binding has no JPEG writer, so `jpeg-encoder` makes it: pure Rust, without `unsafe`.
    /// The worker never writes a file.
    pub fn render_jpeg(&self, index: u32, scale: f32) -> Result<Vec<u8>, EngineError> {
        let pixmap = self.pixmap(index, scale, 0)?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let too_large = || EngineError::TooLarge {
            width: u64::from(width),
            height: u64::from(height),
        };
        // A JPEG side is at most 65535 pixels; the raster limit keeps pages far below it.
        let (Ok(jpeg_width), Ok(jpeg_height)) = (u16::try_from(width), u16::try_from(height))
        else {
            return Err(too_large());
        };
        // The encoder takes rows of RGB without padding.
        let channels = usize::from(pixmap.n());
        let stride = usize::try_from(pixmap.stride()).unwrap_or(0);
        let row_bytes = width as usize * channels;
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for row in pixmap.samples().chunks_exact(stride.max(1)) {
            for pixel in row[..row_bytes].chunks_exact(channels) {
                rgb.extend_from_slice(&pixel[..3]);
            }
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, JPEG_QUALITY)
            .encode(&rgb, jpeg_width, jpeg_height, jpeg_encoder::ColorType::Rgb)
            .map_err(|error| EngineError::Encode(error.to_string()))?;
        if jpeg.len() > MAX_JPEG_BYTES {
            return Err(too_large());
        }
        Ok(jpeg)
    }

    /// The page drawn at `scale` and `rotation`, opaque RGB on white. Oversized pages are refused
    /// before MuPDF allocates anything.
    fn pixmap(&self, index: u32, scale: f32, rotation: u16) -> Result<Pixmap, EngineError> {
        if !scale.is_finite() || !(MIN_RENDER_SCALE..=MAX_RENDER_SCALE).contains(&scale) {
            return Err(EngineError::InvalidScale);
        }
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err(EngineError::InvalidRotation);
        }
        let page = self.load_page(index)?;

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
        Ok(page.to_pixmap(&ctm, &Colorspace::device_rgb(), false, false)?)
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

    /// One page's text for selecting and copying (MVP-15): the lines of the same text layer
    /// search uses, in the same page space, up to `max_chars` characters.
    pub fn page_text(&self, index: u32, max_chars: usize) -> Result<TextLayer, EngineError> {
        let text_page = self
            .load_page(index)?
            .to_text_page(TextPageFlags::empty())?;
        let mut layer = TextLayerBuilder::new(max_chars);
        'blocks: for block in text_page.blocks() {
            for line in block.lines() {
                layer.push_line(
                    line.chars()
                        .filter_map(|ch| ch.char().map(|c| (c, contract_quad(ch.quad())))),
                );
                if layer.is_full() {
                    break 'blocks;
                }
            }
        }
        Ok(layer.finish())
    }

    fn load_page(&self, index: u32) -> Result<Page, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self.doc.load_page(index as i32)?)
    }

    /// Turns `pages` clockwise by `degrees` (90, 180 or 270) on top of their rotation, in memory
    /// (ADR 0013). Every page is checked first; if MuPDF fails halfway, the pages already turned
    /// are turned back, so the document is either fully edited or unchanged.
    pub fn rotate_pages(&mut self, pages: &[u32], degrees: u16) -> Result<(), EngineError> {
        if !matches!(degrees, 90 | 180 | 270) {
            return Err(EngineError::InvalidRotation);
        }
        let count = self.page_count()?;
        if let Some(&page) = pages.iter().find(|&&page| page >= count) {
            return Err(EngineError::PageOutOfRange(page));
        }
        let turn = |page: u32, degrees: i32| -> Result<(), EngineError> {
            let mut pdf_page = self.doc.load_pdf_page(page as i32)?;
            let rotation = pdf_page.rotation()?;
            pdf_page.set_rotation((rotation + degrees).rem_euclid(360))?;
            Ok(())
        };
        for (done, &page) in pages.iter().enumerate() {
            if let Err(error) = turn(page, i32::from(degrees)) {
                for &turned in &pages[..done] {
                    let _ = turn(turned, -i32::from(degrees));
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Removes `pages` (no repeats), leaving at least one (B2-05). Every page is checked first.
    ///
    /// A rewritten file keeps whatever is still referenced, so what only the removed pages had
    /// is taken out too, as ADR 0013 promises:
    /// - their form fields and their part of the structure tree ([`crate::leftovers`], #138);
    /// - every reference to them, or to what was taken out ([`crate::unlink`]): an outline
    ///   entry, link or named destination to a removed page stays, but goes nowhere.
    pub fn delete_pages(&mut self, pages: &[u32]) -> Result<(), EngineError> {
        let count = self.checked_pages(pages)?;
        if pages.len() >= count as usize {
            return Err(EngineError::NoPageLeft);
        }
        let deleting: HashSet<u32> = pages.iter().copied().collect();
        let (mut removed, mut kept) = (HashSet::new(), HashSet::new());
        for index in 0..count {
            let number = self.doc.find_page(index as i32)?.as_indirect()?;
            if deleting.contains(&index) {
                removed.insert(number);
            } else {
                kept.insert(number);
            }
        }
        // A damaged page tree can list one page object twice: one still in it is kept.
        removed.retain(|number| *number != 0 && !kept.contains(number));
        let mut gone = crate::leftovers::take_out(&self.doc, &removed)?;
        let selection: Vec<usize> = pages.iter().map(|&page| page as usize).collect();
        self.doc.delete_pages(PageSelection::Pages(selection))?;
        gone.extend(&removed);
        crate::unlink::unlink(&self.doc, &gone)
    }

    /// Moves `pages` (no repeats) to just before page `before` (the page count: to the end),
    /// together and in their order in the document; the other pages keep theirs (B2-05). If
    /// MuPDF fails halfway, the moves already made are undone.
    pub fn move_pages(&mut self, pages: &[u32], before: u32) -> Result<(), EngineError> {
        let count = self.checked_pages(pages)?;
        if before > count {
            return Err(EngineError::PageOutOfRange(before));
        }
        let mut moved = pages.to_vec();
        moved.sort_unstable();
        let len = moved.len() as u32;
        // Where they start once in place: after the pages that stay and come before `before`.
        let start = before - moved.iter().filter(|&&page| page < before).count() as u32;
        // Each page first goes to the end, in order: the ones already there were before it, so
        // it is `index` places nearer the start than it was. Then the block goes to its place.
        let steps = moved
            .iter()
            .enumerate()
            .map(|(index, &page)| (page - index as u32, count - 1))
            .chain((0..len).map(|index| (count - len + index, start + index)))
            .filter(|(from, to)| from != to);
        let mut done = Vec::new();
        for (from, to) in steps {
            if let Err(error) = self.doc.move_page(from as usize, to as usize) {
                for &(from, to) in done.iter().rev() {
                    let _ = self.doc.move_page(to as usize, from as usize);
                }
                return Err(error.into());
            }
            done.push((from, to));
        }
        Ok(())
    }

    /// Inserts a blank page at index `at` (the page count: after the last page), upright and
    /// the size page `like` is shown at (B2-05).
    pub fn insert_blank_page(&mut self, at: u32, like: u32) -> Result<(), EngineError> {
        let count = self.page_count()?;
        if at > count {
            return Err(EngineError::PageOutOfRange(at));
        }
        if count >= MAX_PAGE_COUNT {
            return Err(EngineError::TooManyPages);
        }
        let size = self.page_size(like)?;
        self.doc.new_page_at(at as i32, size)?;
        Ok(())
    }

    /// Checks the pages an edit names (some, none twice, all in the document) and returns the
    /// page count.
    fn checked_pages(&self, pages: &[u32]) -> Result<u32, EngineError> {
        if pages.is_empty() {
            return Err(EngineError::InvalidEdit("no pages"));
        }
        if pages.iter().collect::<HashSet<_>>().len() != pages.len() {
            return Err(EngineError::InvalidEdit("a page appears twice"));
        }
        let count = self.page_count()?;
        if let Some(&page) = pages.iter().find(|&&page| page >= count) {
            return Err(EngineError::PageOutOfRange(page));
        }
        Ok(count)
    }

    /// Whether the document is signed: its form says signatures exist (`/SigFlags` bit 1), or a
    /// signature field has a value. Saving must then append, to keep the signatures valid.
    pub fn is_signed(&self) -> bool {
        (|| -> Result<bool, mupdf::Error> {
            let Some(form) = self.doc.catalog()?.get_dict("AcroForm")? else {
                return Ok(false);
            };
            if let Some(flags) = form.get_dict("SigFlags")?
                && flags.as_int()? & 1 != 0
            {
                return Ok(true);
            }
            let Some(fields) = form.get_dict("Fields")? else {
                return Ok(false);
            };
            // Breadth first through the field tree, bounded like the other walks here.
            let mut queue = vec![fields];
            let mut seen = 0;
            while let Some(list) = queue.pop() {
                for index in 0..list.len()? {
                    seen += 1;
                    if seen > MAX_FORM_FIELDS {
                        return Ok(false);
                    }
                    let Some(field) = list.get_array(index as i32)? else {
                        continue;
                    };
                    let is_signature = field
                        .get_dict("FT")?
                        .is_some_and(|kind| kind.as_name().is_ok_and(|name| name == b"Sig"));
                    if is_signature && field.get_dict("V")?.is_some() {
                        return Ok(true);
                    }
                    if let Some(kids) = field.get_dict("Kids")? {
                        queue.push(kids);
                    }
                }
            }
            Ok(false)
        })()
        .unwrap_or(false)
    }

    /// Writes the document, with its edits, to `out` (ADR 0013): appended to the original for a
    /// signed document, so its signatures stay valid; otherwise rewritten without its unused
    /// objects, so that removed content is really gone. An encrypted document stays encrypted
    /// as it was. Returns the bytes written and whether they were appended; stops with
    /// `Write(FileTooLarge)` beyond `MAX_DOCUMENT_BYTES`.
    pub fn save(&self, out: &mut impl Write) -> Result<(u64, bool), EngineError> {
        let incremental = self.is_signed() && self.doc.can_be_saved_incrementally();
        let mut options = PdfWriteOptions::default();
        if incremental {
            options.set_incremental(true);
        } else {
            options.set_garbage(true);
        }
        Ok((write_limited(&self.doc, out, options)?, incremental))
    }

    /// Whether the document is encrypted (MVP-16), with a password or only with permissions.
    pub fn is_encrypted(&self) -> bool {
        self.doc
            .trailer()
            .ok()
            .and_then(|trailer| trailer.get_dict("Encrypt").ok().flatten())
            .is_some()
    }

    /// Writes to `out` a copy of the document, edits included, without its metadata, and with
    /// `id` as its identifier (B2-03, [`crate::privacy::strip`]). The copy is rewritten whole,
    /// without the objects nothing uses any more, so none of the metadata is left in the file.
    ///
    /// The open document is not touched: it is saved into memory and opened again, and that
    /// second document is the one cleaned. An encrypted document is refused: the worker keeps no
    /// password, so its copy could not be encrypted again, and an unencrypted copy would drop
    /// what its author asked for.
    pub fn privacy_copy(&self, id: &[u8; 16], out: &mut impl Write) -> Result<u64, EngineError> {
        if self.is_encrypted() {
            return Err(EngineError::EncryptedCopy);
        }
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut bytes = Vec::new();
        write_limited(&self.doc, &mut bytes, options)?;
        let copy = Document::from_bytes(&bytes, "application/pdf").map_err(open_error)?;
        drop(bytes);
        let copy = MuPdfDocument::try_from(copy)?;
        crate::privacy::strip(&copy, id)?;
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        write_limited(&copy, out, options)
    }
}

impl PdfDocument {
    /// Writes the pages `pages` (0-based, no repeats) of this document to `out` as a document of
    /// their own (B2-06), in the order they have in the document. The open document is not
    /// touched: it is saved into memory and opened again, and that second document loses the
    /// other pages as any document does (see `delete_pages`: what only they had goes too, and
    /// what pointed to them points nowhere). What the kept pages have, their annotations,
    /// links and form fields included, stays as it is, and so does what belongs to the document
    /// as a whole.
    ///
    /// An encrypted document is refused, as for the privacy export: the worker keeps no
    /// password, so the copy could not be encrypted again, and an unencrypted one would drop what
    /// its author asked for.
    pub fn pages_copy(&self, pages: &[u32], out: &mut impl Write) -> Result<u64, EngineError> {
        if self.is_encrypted() {
            return Err(EngineError::EncryptedCopy);
        }
        if pages.is_empty() {
            return Err(EngineError::InvalidEdit("no pages to copy"));
        }
        let count = self.page_count()?;
        let mut seen = HashSet::new();
        for &page in pages {
            if page >= count {
                return Err(EngineError::PageOutOfRange(page));
            }
            if !seen.insert(page) {
                return Err(EngineError::InvalidEdit("a page appears twice"));
            }
        }
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut bytes = Vec::new();
        write_limited(&self.doc, &mut bytes, options)?;
        let mut copy = PdfDocument::from_bytes(&bytes)?;
        drop(bytes);
        let keep: HashSet<u32> = pages.iter().copied().collect();
        let others: Vec<u32> = (0..count).filter(|page| !keep.contains(page)).collect();
        if !others.is_empty() {
            copy.delete_pages(&others)?;
        }
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        write_limited(&copy.doc, out, options)
    }
}

/// What `PdfDocument::prepare_source` made of a PDF whose pages the user wants in another
/// document (B2-06).
#[derive(Debug)]
pub struct PreparedSource {
    /// The PDF as MuPDF writes it, plain: not encrypted, so that it can be opened again whenever
    /// the edit that uses it is made again, without a password.
    pub bytes: Vec<u8>,
    pub pages: u32,
    /// The active content the PDF has (MVP-11). None of it comes along with the pages.
    pub security: SecurityReport,
}

impl PdfDocument {
    /// Opens `bytes`, a PDF the user chose to take pages from (with `password`, if it is
    /// encrypted), and makes the plain copy of it that its pages are later taken from (B2-06):
    /// written again by MuPDF, so that what is left is a clean file, and not encrypted, so that
    /// no password has to be kept. It is at most `MAX_SOURCE_BYTES` long.
    ///
    /// The author of an encrypted file may forbid taking pages out of it (`/P` bit 5, which is
    /// what Acrobat calls extracting pages): then nothing is made.
    pub fn prepare_source(
        bytes: &[u8],
        password: Option<&str>,
    ) -> Result<PreparedSource, EngineError> {
        let source = PdfDocument::open(bytes, password)?;
        let pages = source.page_count()?;
        if pages == 0 {
            return Err(EngineError::InvalidEdit("the file has no pages"));
        }
        if pages > MAX_PAGE_COUNT {
            return Err(EngineError::TooManyPages);
        }
        if !source.permissions().copy {
            return Err(EngineError::NotAllowed("taking pages out of the file"));
        }
        let security = source.active_content(ScanBudget::default());
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        // Whatever the file had (the default of MuPDF's writer keeps it).
        options.set_encryption(Encryption::None);
        let mut plain = Vec::new();
        write_at_most(&source.doc, &mut plain, options, MAX_SOURCE_BYTES as u64)?;
        Ok(PreparedSource {
            bytes: plain,
            pages,
            security,
        })
    }

    /// Puts the pages of `source` (a plain PDF, as `prepare_source` made it) into the document
    /// from index `at` on (the page count: after the last page), in their order (B2-06).
    ///
    /// Only what is on the pages comes along (their content and what it uses): MuPDF takes a
    /// page over without its annotations, links and form fields, and nothing that belongs to the
    /// source as a whole (its `/OpenAction`, scripts, outline, embedded files). What a page
    /// does by itself (its `/AA`) is not taken either: see docs/architecture/merge.md. If it
    /// fails, the document is as it was.
    pub fn insert_pages(&mut self, at: u32, source: &[u8]) -> Result<(), EngineError> {
        let count = self.page_count()?;
        if at > count {
            return Err(EngineError::PageOutOfRange(at));
        }
        let source = PdfDocument::from_bytes(source)?;
        let inserted = source.page_count()?;
        if inserted == 0 {
            return Err(EngineError::InvalidEdit("the source has no pages"));
        }
        if count.saturating_add(inserted) > MAX_PAGE_COUNT {
            return Err(EngineError::TooManyPages);
        }
        let target = if at == count {
            InsertPosition::Append
        } else {
            InsertPosition::Before(at as usize)
        };
        // Whatever the graft makes is numbered from here on.
        let first_new = self.doc.count_objects()?;
        self.doc.insert_pdf(
            &source.doc,
            InsertPdfOptions {
                source_pages: PageSelection::All,
                target,
                ..InsertPdfOptions::default()
            },
        )?;
        crate::scrub::scrub_pages(&self.doc, at, inserted, first_new)
    }
}

/// Writes `doc` to `out` with `options`, stopping with `Write(FileTooLarge)` beyond
/// `MAX_DOCUMENT_BYTES`; returns the bytes written.
fn write_limited(
    doc: &MuPdfDocument,
    out: &mut impl Write,
    options: PdfWriteOptions,
) -> Result<u64, EngineError> {
    write_at_most(doc, out, options, MAX_DOCUMENT_BYTES)
}

/// As [`write_limited`], with `limit` bytes the most.
fn write_at_most(
    doc: &MuPdfDocument,
    out: &mut impl Write,
    options: PdfWriteOptions,
    limit: u64,
) -> Result<u64, EngineError> {
    let mut limited = Limited { out, left: limit };
    match doc.write_to_with_options(&mut limited, options) {
        Ok(bytes) => Ok(bytes),
        Err(mupdf::Error::Io(error)) => Err(EngineError::Write(error)),
        Err(error) => Err(EngineError::MuPdf(error)),
    }
}

/// Passes writes through until `left` bytes have gone, then fails with `FileTooLarge`.
struct Limited<'a, W> {
    out: &'a mut W,
    left: u64,
}

impl<W: Write> Write for Limited<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.left {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        let written = self.out.write(buf)?;
        self.left -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The permissions in `/P` (ISO 32000-2, table 22; bit n counts from 1). Revision 2 of the
/// standard security handler has no high-quality bit: whoever may print, prints at full quality.
fn permissions_from(p: i32, revision: Option<i32>) -> DocumentPermissions {
    let allows = |bit: u32| p & (1 << (bit - 1)) != 0;
    let print = allows(3);
    let before_revision_3 = revision.is_some_and(|r| r < 3);
    DocumentPermissions {
        copy: allows(5),
        print,
        print_high_quality: print && (before_revision_3 || allows(12)),
        modify: allows(4),
        // Revision 2 has no assembly bit (bit 11 is reserved, and set): modifying covers it.
        assemble: if before_revision_3 {
            allows(4)
        } else {
            allows(11)
        },
        annotate: allows(6),
        // Bit 6 covers annotating and filling in forms; bit 9 (revision 3 and later; reserved, and
        // set, before) only the latter.
        fill_forms: if before_revision_3 {
            allows(6)
        } else {
            allows(9) || allows(6)
        },
    }
}

/// MuPDF refuses, while opening the file, a security handler other than the standard password
/// one (certificate encryption, `/Adobe.PubSec`) and encryption versions it does not know. Such
/// a document is not broken: it is encrypted in a way the app cannot open.
fn open_error(error: mupdf::Error) -> EngineError {
    let message = error.to_string();
    if message.contains("encryption handler") || message.contains("encryption version") {
        EngineError::UnsupportedEncryption
    } else {
        EngineError::MuPdf(error)
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
    use ipc_contract::types::{
        AnnotationId, AnnotationKind, FieldId, FieldKind, FieldOption, FormField, HighlightColor,
        HighlightMark, PageAnnotation, Rect,
    };

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

    /// Helvetica draws "AB", but the ToUnicode map says the text is 隱私 (U+96B1 U+79C1).
    fn to_unicode_pdf() -> Vec<u8> {
        let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap
                    /CMapName /Test-UCS def /CMapType 2 def
                    1 begincodespacerange <00> <FF> endcodespacerange
                    2 beginbfchar <41> <96B1> <42> <79C1> endbfchar
                    endcmap CMapName currentdict /CMap defineresource pop end end";
        let content = "BT /F1 24 Tf 72 700 Td (AB) Tj ET";
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            &stream(content),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
            &stream(cmap),
        ])
    }

    #[test]
    fn searches_chinese_text_from_the_to_unicode_map() {
        // Search reads the text layer, not the glyphs, and needs no CJK font.
        let doc = PdfDocument::from_bytes(&to_unicode_pdf()).unwrap();
        assert_eq!(doc.search_page(0, "隱私", false, 10).unwrap().hits.len(), 1);
        assert_eq!(doc.search_page(0, "私", false, 10).unwrap().hits.len(), 1);
        assert!(doc.search_page(0, "AB", false, 10).unwrap().hits.is_empty());
    }

    #[test]
    fn gives_the_text_of_a_page_line_by_line() {
        use ipc_contract::validate::Validate;

        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let text = doc.page_text(0, 1000).unwrap();
        text.validate().unwrap();
        assert!(!text.truncated);
        let [line] = &text.lines[..] else {
            panic!("one line: {text:?}")
        };
        assert_eq!(line.text, "Hello MuPDF");
        // The line and its characters are where search finds the same text.
        let hit = doc.search_page(0, "mupdf", false, 1).unwrap().hits[0].quads[0];
        let m = line.text.chars().position(|c| c == 'M').unwrap();
        assert!(
            (line.quad.ul.x + line.edges[m] - hit.ul.x).abs() < 0.5,
            "{line:?} {hit:?}"
        );
        assert!((line.quad.ur.x - hit.ur.x).abs() < 0.5, "{line:?} {hit:?}");
        assert!((line.quad.ul.y - hit.ul.y).abs() < 0.5 && (line.quad.ll.y - hit.ll.y).abs() < 0.5);

        assert!(doc.page_text(1, 1000).unwrap().lines.is_empty());
        assert!(matches!(
            doc.page_text(2, 1000),
            Err(EngineError::PageOutOfRange(2))
        ));
    }

    #[test]
    fn page_text_stops_at_the_character_limit() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let text = doc.page_text(0, 5).unwrap();
        assert!(text.truncated);
        assert_eq!(text.lines[0].text, "Hello");
    }

    #[test]
    fn page_text_comes_from_the_to_unicode_map() {
        let doc = PdfDocument::from_bytes(&to_unicode_pdf()).unwrap();
        let text = doc.page_text(0, 1000).unwrap();
        assert_eq!(text.lines[0].text, "隱私");
        assert_eq!(text.lines[0].edges.len(), 3);
    }

    /// The two-page test PDF with an Encrypt dictionary for `filter` (nothing is encrypted, but
    /// MuPDF looks at the security handler before anything else).
    fn encrypted_with(filter: &str) -> Vec<u8> {
        let mut pdf = two_page_pdf();
        let trailer = b"trailer\n<< /Size 8 /Root 1 0 R";
        let at = pdf
            .windows(trailer.len())
            .position(|window| window == trailer)
            .expect("trailer");
        let encrypt = format!(" /Encrypt << /Filter /{filter} /V 4 /R 4 /Length 128 >>");
        let end = at + trailer.len();
        pdf.splice(end..end, encrypt.into_bytes());
        pdf
    }

    #[test]
    fn certificate_encryption_is_reported_as_unsupported() {
        assert!(matches!(
            PdfDocument::open(&encrypted_with("Adobe.PubSec"), Some("password")),
            Err(EngineError::UnsupportedEncryption)
        ));
    }

    #[test]
    fn reads_the_permission_bits() {
        const PRINT: i32 = 1 << 2;
        const COPY: i32 = 1 << 4;
        const PRINT_HQ: i32 = 1 << 11;
        let all = -4;
        assert_eq!(permissions_from(all, Some(6)), DocumentPermissions::ALL);
        assert_eq!(
            permissions_from(all & !COPY, Some(4)),
            DocumentPermissions {
                copy: false,
                ..DocumentPermissions::ALL
            }
        );
        // Without the high-quality bit, printing is low resolution from revision 3 on.
        let low_res = DocumentPermissions {
            print_high_quality: false,
            ..DocumentPermissions::ALL
        };
        assert_eq!(permissions_from(all & !PRINT_HQ, Some(3)), low_res);
        assert_eq!(permissions_from(all & !PRINT_HQ, None), low_res);
        assert_eq!(
            permissions_from(all & !PRINT_HQ, Some(2)),
            DocumentPermissions::ALL
        );
        // The high-quality bit alone does not allow printing.
        for revision in [Some(2), Some(6)] {
            assert_eq!(
                permissions_from(all & !PRINT & !COPY, revision),
                DocumentPermissions {
                    copy: false,
                    print: false,
                    print_high_quality: false,
                    ..DocumentPermissions::ALL
                }
            );
        }
        // Changing pages needs the assembly bit from revision 3 on, the modify bit before.
        const MODIFY: i32 = 1 << 3;
        const ASSEMBLE: i32 = 1 << 10;
        let pages_only = permissions_from(all & !MODIFY, Some(4));
        assert!(!pages_only.modify && pages_only.assemble);
        assert!(!permissions_from(all & !ASSEMBLE, Some(4)).assemble);
        assert!(permissions_from(all & !ASSEMBLE, Some(2)).assemble);
        assert!(!permissions_from(all & !MODIFY & !ASSEMBLE, Some(2)).assemble);
        // Filling in forms: bit 9 from revision 3 on, or bit 6, which covers it; before revision 3
        // bit 9 is reserved (and set), so only bit 6 counts (B2-09).
        const FILL_FORMS: i32 = 1 << 8;
        const ANNOTATE_BIT: i32 = 1 << 5;
        assert!(permissions_from(all & !ANNOTATE_BIT, Some(4)).fill_forms);
        assert!(permissions_from(all & !FILL_FORMS, Some(4)).fill_forms);
        assert!(!permissions_from(all & !FILL_FORMS & !ANNOTATE_BIT, Some(4)).fill_forms);
        assert!(!permissions_from(all & !ANNOTATE_BIT, Some(2)).fill_forms);
        assert!(permissions_from(all & !FILL_FORMS, Some(2)).fill_forms);
        // Annotating has a bit of its own (6), in every revision (B2-07).
        const ANNOTATE: i32 = 1 << 5;
        for revision in [Some(2), Some(4)] {
            let no_annotations = permissions_from(all & !ANNOTATE, revision);
            assert!(!no_annotations.annotate && no_annotations.modify && no_annotations.assemble);
        }
    }

    #[test]
    fn an_unencrypted_document_allows_everything() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        assert_eq!(doc.permissions(), DocumentPermissions::ALL);
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
    fn exports_a_page_as_a_png_file() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let (width_pt, height_pt) = doc.page_size(0).unwrap();
        let png = doc.render_png(0, 150.0 / 72.0).unwrap();
        assert!(png.starts_with(&ipc_contract::validate::PNG_SIGNATURE));
        // IHDR: width and height, big-endian, right after the signature and chunk header.
        let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        assert_eq!(size(16), (width_pt * 150.0 / 72.0).ceil() as u32);
        assert_eq!(size(20), (height_pt * 150.0 / 72.0).ceil() as u32);
        assert!(matches!(
            doc.render_png(0, 1000.0),
            Err(EngineError::InvalidScale)
        ));
    }

    /// Every dictionary of `doc`: each object's, and those written directly inside it.
    fn every_dictionary(doc: &MuPdfDocument) -> Vec<PdfObject> {
        let mut found = Vec::new();
        for number in 1..doc.xref_len().unwrap() {
            let Some(object) = doc.xref_object(number as i32).unwrap() else {
                continue;
            };
            let mut pending = vec![object];
            while let Some(node) = pending.pop() {
                let children: Vec<PdfObject> = if node.is_dict().unwrap() {
                    (0..node.dict_len().unwrap())
                        .filter_map(|index| node.get_dict_val(index as i32).unwrap())
                        .collect()
                } else if node.is_array().unwrap() {
                    (0..node.len().unwrap())
                        .filter_map(|index| node.get_array(index as i32).unwrap())
                        .collect()
                } else {
                    Vec::new()
                };
                pending.extend(
                    children
                        .into_iter()
                        .filter(|child| !child.is_indirect().unwrap()),
                );
                if node.is_dict().unwrap() {
                    found.push(node);
                }
            }
        }
        found
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn the_privacy_export_leaves_no_metadata() {
        const AUTHOR: &[u8] = b"Jane Q. Private-Author";
        let original = corpus("benign/metadata-full.pdf");
        assert!(contains(&original, AUTHOR), "the sample names its author");
        let doc = PdfDocument::from_bytes(&original).unwrap();
        let id = [0xA5; 16];
        let mut copy = Vec::new();
        let written = doc.privacy_copy(&id, &mut copy).unwrap();
        assert_eq!(written, copy.len() as u64);

        // Not in the file, and not in any of its streams once decoded.
        assert!(!contains(&copy, AUTHOR));
        let exported = PdfDocument::from_bytes(&copy).unwrap();
        let trailer = exported.doc.trailer().unwrap();
        assert!(trailer.get_dict("Info").unwrap().is_none());
        let first_id = trailer
            .get_dict("ID")
            .unwrap()
            .and_then(|id| id.get_array(0).unwrap())
            .and_then(|first| first.as_bytes().ok());
        assert_eq!(first_id, Some("a5".repeat(16).into_bytes()));
        let dictionaries = every_dictionary(&exported.doc);
        assert!(dictionaries.len() > 5, "{}", dictionaries.len());
        for dict in &dictionaries {
            for key in ["Metadata", "PieceInfo", "LastModified", "Thumb"] {
                assert!(dict.get_dict(key).unwrap().is_none(), "/{key} is left");
            }
            if dict.is_stream().unwrap() {
                assert!(!contains(&dict.read_stream().unwrap(), AUTHOR));
            }
        }
        // The sticky note is still there, without its author and dates.
        let note = dictionaries
            .iter()
            .find(|dict| {
                dict.get_dict("Subtype")
                    .unwrap()
                    .is_some_and(|subtype| subtype.as_name().unwrap() == b"Text")
            })
            .expect("the note");
        for key in ["T", "M", "CreationDate"] {
            assert!(note.get_dict(key).unwrap().is_none(), "/{key} of the note");
        }
        assert!(note.get_dict("Contents").unwrap().is_some());

        // The same page.
        assert_eq!(
            exported.render(0, 1.0, 0).unwrap(),
            doc.render(0, 1.0, 0).unwrap()
        );
        // The open document is not changed.
        assert!(
            doc.doc
                .trailer()
                .unwrap()
                .get_dict("Info")
                .unwrap()
                .is_some()
        );
        let mut again = Vec::new();
        doc.privacy_copy(&[0x5A; 16], &mut again).unwrap();
        assert!(!contains(&again, AUTHOR));
    }

    #[test]
    fn the_privacy_export_keeps_form_field_names_and_refuses_encrypted_documents() {
        let form = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R << /Subtype /Square /Rect [10 10 50 50] /T (Direct Author) /M (D:20260102) >>] >>",
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (customer_name) /Rect [72 700 272 720] /V (x) >>",
        ]);
        let doc = PdfDocument::from_bytes(&form).unwrap();
        let mut copy = Vec::new();
        doc.privacy_copy(&[1; 16], &mut copy).unwrap();
        // The field keeps its name; the annotation written directly in /Annots loses its author.
        assert!(contains(&copy, b"customer_name"));
        assert!(!contains(&copy, b"Direct Author"));

        let encrypted = corpus("benign/encrypted-aes256.pdf");
        let doc = PdfDocument::open(&encrypted, Some("user")).unwrap();
        assert!(doc.is_encrypted());
        assert!(matches!(
            doc.privacy_copy(&[1; 16], &mut Vec::new()),
            Err(EngineError::EncryptedCopy)
        ));
    }

    #[test]
    fn exports_a_page_as_a_jpeg_file() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let scale = 150.0 / 72.0;
        let jpeg = doc.render_jpeg(0, scale).unwrap();
        assert!(jpeg.starts_with(&ipc_contract::validate::JPEG_SIGNATURE));

        // MuPDF opens it: a Letter page at 150 dpi is 1275 x 1650 pixels.
        let decoded = mupdf::Image::from_bytes(&jpeg)
            .and_then(|image| image.to_pixmap())
            .expect("a JPEG file MuPDF can open");
        assert_eq!((decoded.width(), decoded.height()), (1275, 1650));
        // The same picture as the page rendered for the screen, up to JPEG's small losses: rows,
        // their order and the colour channels are where they belong.
        let page = doc.render(0, scale, 0).unwrap();
        assert_eq!((page.width, page.height), (1275, 1650));
        let channels = usize::from(decoded.n());
        assert_eq!(channels, 3, "RGB");
        let stride = usize::try_from(decoded.stride()).unwrap();
        let mut difference = 0u64;
        for (y, row) in decoded.samples().chunks_exact(stride).enumerate() {
            for x in 0..page.width as usize {
                let jpeg = &row[x * 3..x * 3 + 3];
                let screen = &page.rgba[(y * page.width as usize + x) * 4..][..3];
                for (a, b) in jpeg.iter().zip(screen) {
                    difference += u64::from(a.abs_diff(*b));
                }
            }
        }
        let mean = difference as f64 / (1275.0 * 1650.0 * 3.0);
        assert!(mean < 2.0, "mean difference {mean}");
        assert!(
            page.rgba.iter().any(|&value| value < 128),
            "the page has dark marks"
        );

        assert!(matches!(
            doc.render_jpeg(0, 1000.0),
            Err(EngineError::InvalidScale)
        ));
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

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn saved(doc: &PdfDocument) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        let (bytes, incremental) = doc.save(&mut out).expect("save");
        assert_eq!(bytes, out.len() as u64);
        (out, incremental)
    }

    /// One page with a highlighter mark (4), a note (5) with its pop-up (6) and a reply (10), a
    /// square (7), a link (8) and a form field (9) (B2-07).
    fn annotated_pdf() -> Vec<u8> {
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [9 0 R] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 9 0 R 10 0 R] >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [72 700 200 720] /QuadPoints [72 720 200 720 72 700 200 700] /C [1 0.92 0] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /Rect [300 700 320 720] /Contents (Existing  note\ttext) /Popup 6 0 R /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Popup /Rect [320 600 520 700] /Parent 5 0 R >>",
            "<< /Type /Annot /Subtype /Square /Rect [100 100 200 200] /C [1 0 0] >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 50 200 70] /A << /S /URI /URI (https://example.com) >> >>",
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [72 400 200 420] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /Rect [330 700 350 720] /Contents (A reply) /IRT 5 0 R >>",
        ])
    }

    /// Whether `rect` covers `area` (x0, y0, x1, y1) and not much more: a highlighter mark shows
    /// a little beyond the text it marks.
    fn covers(rect: Rect, [x0, y0, x1, y1]: [f32; 4]) -> bool {
        let margin = |outer: f32, inner: f32| (0.0..15.0).contains(&(outer - inner));
        margin(x0, rect.x0) && margin(y0, rect.y0) && margin(rect.x1, x1) && margin(rect.y1, y1)
    }

    fn annotation(doc: &PdfDocument, page: u32, id: u32) -> Option<PageAnnotation> {
        doc.page_annotations(page)
            .expect("annotations")
            .into_iter()
            .find(|annotation| annotation.id == AnnotationId(id))
    }

    #[test]
    fn lists_a_pages_annotations_but_not_its_links_fields_or_pop_ups() {
        let doc = PdfDocument::from_bytes(&annotated_pdf()).expect("open");
        let listed = doc.page_annotations(0).expect("annotations");
        let kinds: Vec<(u32, AnnotationKind)> = listed
            .iter()
            .map(|annotation| (annotation.id.0, annotation.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                (4, AnnotationKind::Highlight),
                (5, AnnotationKind::Note),
                (7, AnnotationKind::Other),
                (10, AnnotationKind::Note),
            ]
        );
        let highlight = &listed[0];
        assert_eq!(highlight.color, Some(HighlightColor::Yellow));
        // Page space, origin at the top left: where it shows, the marked area and a little more.
        assert!(covers(highlight.rect, [72.0, 72.0, 200.0, 92.0]));
        // A note's text is cleaned like any text from a PDF.
        assert_eq!(listed[1].text.as_deref(), Some("Existing note text"));
        assert_eq!(listed[2].color, None);
        assert!(matches!(
            doc.page_annotations(1),
            Err(EngineError::PageOutOfRange(1))
        ));
    }

    #[test]
    fn highlights_and_notes_are_standard_annotations_that_say_nothing_of_who_made_them() {
        let mut doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        let quad = Quad {
            ul: Point { x: 72.0, y: 56.0 },
            ur: Point { x: 300.0, y: 56.0 },
            ll: Point { x: 72.0, y: 100.0 },
            lr: Point { x: 300.0, y: 100.0 },
        };
        let mark = |page: u32, quads: Vec<Quad>| HighlightMark { page, quads };
        doc.add_highlights(&[mark(0, vec![quad])], HighlightColor::Green)
            .expect("highlight");
        doc.add_note(1, Point { x: 100.0, y: 120.0 }, "第一行\nsecond line")
            .expect("note");
        // A mark that does not fit leaves the document as it was, its other pages too.
        for wrong in [
            vec![mark(1, vec![quad]), mark(0, vec![])],
            vec![mark(1, vec![quad]), mark(5, vec![quad])],
            vec![mark(1, vec![quad]), mark(1, vec![quad])],
        ] {
            assert!(doc.add_highlights(&wrong, HighlightColor::Green).is_err());
        }

        let (bytes, _) = saved(&doc);
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        let [highlight] = &reopened.page_annotations(0).expect("annotations")[..] else {
            panic!("one highlight");
        };
        assert_eq!(highlight.kind, AnnotationKind::Highlight);
        assert_eq!(highlight.color, Some(HighlightColor::Green));
        assert!(covers(highlight.rect, [72.0, 56.0, 300.0, 100.0]));
        let [note] = &reopened.page_annotations(1).expect("annotations")[..] else {
            panic!("one note");
        };
        assert_eq!(note.kind, AnnotationKind::Note);
        assert_eq!(note.text.as_deref(), Some("第一行\nsecond line"));
        // Standard PDF: a highlight with QuadPoints and an appearance; nothing about who made
        // them or when.
        let annotations: Vec<PdfObject> = every_dictionary(&reopened.doc)
            .into_iter()
            .filter(|dict| {
                dict.get_dict("Type")
                    .ok()
                    .flatten()
                    .and_then(|name| name.as_name().ok())
                    == Some(b"Annot".to_vec())
            })
            .collect();
        assert!(annotations.len() >= 2);
        for dict in &annotations {
            for key in ["T", "M", "CreationDate", "NM"] {
                assert!(dict.get_dict(key).unwrap().is_none(), "/{key} in {dict:?}");
            }
        }
        assert!(annotations.iter().any(|dict| {
            dict.get_dict("QuadPoints").unwrap().is_some() && dict.get_dict("AP").unwrap().is_some()
        }));
    }

    #[test]
    fn annotations_are_changed_and_removed_with_what_points_to_them() {
        let mut doc = PdfDocument::from_bytes(&annotated_pdf()).expect("open");
        doc.set_highlight_color(0, AnnotationId(4), HighlightColor::Pink)
            .expect("color");
        assert_eq!(
            annotation(&doc, 0, 4).unwrap().color,
            Some(HighlightColor::Pink)
        );
        doc.set_note_text(0, AnnotationId(10), "Changed reply")
            .expect("text");
        assert_eq!(
            annotation(&doc, 0, 10).unwrap().text.as_deref(),
            Some("Changed reply")
        );
        // Only a highlighter mark has a color, only a note a text; links, fields and pop-ups are
        // not annotations the app edits.
        for wrong in [
            doc.set_highlight_color(0, AnnotationId(5), HighlightColor::Blue),
            doc.set_note_text(0, AnnotationId(4), "no"),
            doc.delete_annotation(0, AnnotationId(6)),
            doc.delete_annotation(0, AnnotationId(8)),
            doc.delete_annotation(0, AnnotationId(9)),
            doc.delete_annotation(0, AnnotationId(99)),
        ] {
            assert!(
                matches!(wrong, Err(EngineError::InvalidEdit(_))),
                "{wrong:?}"
            );
        }

        // The note goes with its pop-up, and the reply no longer points to it.
        doc.delete_annotation(0, AnnotationId(5)).expect("delete");
        doc.delete_annotation(0, AnnotationId(7)).expect("delete");
        assert_eq!(
            doc.page_annotations(0)
                .expect("annotations")
                .iter()
                .map(|annotation| annotation.id.0)
                .collect::<Vec<_>>(),
            [4, 10]
        );
        let (bytes, _) = saved(&doc);
        assert!(!contains(&bytes, b"Existing"));
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        let dictionaries = every_dictionary(&reopened.doc);
        let says = |dict: &PdfObject, text: &str| {
            dict.get_dict("Contents")
                .ok()
                .flatten()
                .is_some_and(|contents| contents.as_string().ok().as_deref() == Some(text))
        };
        assert!(
            !dictionaries
                .iter()
                .any(|dict| says(dict, "Existing  note\ttext"))
        );
        let subtype = |dict: &PdfObject| {
            dict.get_dict("Subtype")
                .ok()
                .flatten()
                .and_then(|name| name.as_name().ok())
        };
        assert!(
            !dictionaries
                .iter()
                .any(|dict| subtype(dict) == Some(b"Popup".to_vec()))
        );
        assert!(
            !dictionaries
                .iter()
                .any(|dict| subtype(dict) == Some(b"Square".to_vec()))
        );
        let reply = dictionaries
            .iter()
            .find(|dict| says(dict, "Changed reply"))
            .expect("the reply stays");
        assert!(
            reply
                .get_dict("IRT")
                .unwrap()
                .is_none_or(|target| target.is_null().unwrap())
        );
    }

    fn form() -> PdfDocument {
        PdfDocument::from_bytes(&corpus("benign/form-fields.pdf")).expect("open")
    }

    /// The field named `label` (its tooltip, or its name) among the page's, in the order listed.
    fn fields_named(doc: &PdfDocument, label: &str) -> Vec<FormField> {
        doc.page_fields(0)
            .expect("fields")
            .into_iter()
            .filter(|field| field.label.as_deref() == Some(label))
            .collect()
    }

    fn only(doc: &PdfDocument, label: &str) -> FormField {
        let [field] = &fields_named(doc, label)[..] else {
            panic!("one field named {label}")
        };
        field.clone()
    }

    fn set(doc: &mut PdfDocument, label: &str, value: &str) -> Result<(), EngineError> {
        let field = only(doc, label);
        doc.set_field_value(0, field.id, value)
    }

    #[test]
    fn lists_a_pages_form_fields_with_what_the_page_needs_to_fill_them() {
        let doc = form();
        let fields = doc.page_fields(0).expect("fields");
        let kinds: Vec<FieldKind> = fields.iter().map(|field| field.kind).collect();
        assert_eq!(
            kinds,
            [
                FieldKind::Text,
                FieldKind::Text,
                FieldKind::Text,
                FieldKind::Checkbox,
                FieldKind::Radio,
                FieldKind::Radio,
                FieldKind::Radio,
                FieldKind::Combo,
                FieldKind::List,
                FieldKind::Text,
                FieldKind::Text,
            ]
        );
        let name = only(&doc, "Your name");
        assert_eq!(name.value, "Jane Q. Public");
        assert!(!name.read_only && !name.required && !name.multiline && name.max_len.is_none());
        // Page space, origin at the top left: [72 650 300 674] on a 792-point page.
        assert_eq!(
            name.rect,
            Rect {
                x0: 72.0,
                y0: 118.0,
                x1: 300.0,
                y1: 142.0
            }
        );
        assert!(only(&doc, "Notes").multiline);
        assert_eq!(
            only(&doc, "Code of at most five characters").max_len,
            Some(5)
        );
        assert!(only(&doc, "Required").required);
        // A field without a tooltip is called by its name.
        let locked = only(&doc, "locked");
        assert!(locked.read_only);
        assert_eq!(locked.value, "Locked value");

        let agree = only(&doc, "I agree");
        assert_eq!(
            (agree.value.as_str(), agree.on_value.as_deref()),
            ("Off", Some("Yes"))
        );
        let sizes = fields_named(&doc, "Size");
        let on: Vec<_> = sizes
            .iter()
            .map(|field| field.on_value.clone().unwrap())
            .collect();
        assert_eq!(on, ["Small", "Medium", "Large"]);
        assert!(sizes.iter().all(|field| field.value == "Off"));
        // The buttons of a group have the group's own number; any other field is its own group.
        assert!(sizes.iter().all(|field| field.group == sizes[0].group));
        assert!(sizes.iter().all(|field| field.group != field.id));
        assert_eq!(name.group, name.id);
        assert_ne!(name.group, agree.group);

        let country = only(&doc, "Country");
        assert_eq!(country.value, "TW");
        assert_eq!(
            country.options,
            [("TW", "Taiwan"), ("JP", "Japan"), ("US", "United States")].map(|(value, label)| {
                FieldOption {
                    value: value.to_owned(),
                    label: label.to_owned(),
                }
            })
        );
        assert!(!country.editable);
        let fruit = only(&doc, "Fruit");
        assert_eq!(fruit.value, "banana");
        assert_eq!(fruit.options.len(), 3);
        assert!(fields.iter().all(|field| !field.has_script));
        assert!(matches!(
            doc.page_fields(1),
            Err(EngineError::PageOutOfRange(1))
        ));
    }

    #[test]
    fn fields_are_filled_in_and_the_values_are_in_the_saved_file() {
        let mut doc = form();
        set(&mut doc, "Your name", "林 小明").expect("text");
        set(&mut doc, "Notes", "one\n  two").expect("several lines");
        set(&mut doc, "Code of at most five characters", "AB12Z").expect("limit");
        set(&mut doc, "I agree", "Yes").expect("check");
        set(&mut doc, "Country", "JP").expect("combo");
        set(&mut doc, "Fruit", "cherry").expect("list");
        // A radio button takes the whole group with it.
        let sizes = fields_named(&doc, "Size");
        doc.set_field_value(0, sizes[1].id, "Medium")
            .expect("radio");
        let states = |doc: &PdfDocument| -> Vec<String> {
            fields_named(doc, "Size")
                .into_iter()
                .map(|field| field.value)
                .collect()
        };
        assert_eq!(states(&doc), ["Off", "Medium", "Off"]);
        doc.set_field_value(0, sizes[2].id, "Large").expect("radio");
        assert_eq!(states(&doc), ["Off", "Off", "Large"]);

        let (bytes, _) = saved(&doc);
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        assert_eq!(only(&reopened, "Your name").value, "林 小明");
        assert_eq!(only(&reopened, "Notes").value, "one\n  two");
        assert_eq!(
            only(&reopened, "Code of at most five characters").value,
            "AB12Z"
        );
        assert_eq!(only(&reopened, "I agree").value, "Yes");
        assert_eq!(only(&reopened, "Country").value, "JP");
        assert_eq!(only(&reopened, "Fruit").value, "cherry");
        assert_eq!(states(&reopened), ["Off", "Off", "Large"]);
        // The value of a button is a name, as other readers expect (MuPDF writes a text).
        let agree = every_dictionary(&reopened.doc)
            .into_iter()
            .find(|dict| {
                dict.get_dict("T")
                    .ok()
                    .flatten()
                    .is_some_and(|name| name.as_string().ok().as_deref() == Some("agree"))
            })
            .expect("the check box");
        assert!(agree.get_dict("V").unwrap().unwrap().is_name().unwrap());

        // A check box goes off again.
        let mut again = PdfDocument::from_bytes(&bytes).expect("reopen");
        set(&mut again, "I agree", "Off").expect("uncheck");
        assert_eq!(only(&again, "I agree").value, "Off");
        // And a field is cleared.
        set(&mut again, "Your name", "").expect("clear");
        assert_eq!(only(&again, "Your name").value, "");
    }

    #[test]
    fn what_a_field_cannot_have_is_refused_and_changes_nothing() {
        let mut doc = form();
        let before = doc.page_fields(0).expect("fields");
        let name = only(&doc, "Your name");
        let sizes = fields_named(&doc, "Size");
        let code = "Code of at most five characters";
        for (label, value) in [
            ("locked", "other"),
            (code, "six 6!"),
            ("Your name", "two\nlines"),
            ("Country", "FR"),
            ("Fruit", "kiwi"),
            ("I agree", "Maybe"),
        ] {
            assert!(
                matches!(
                    set(&mut doc, label, value),
                    Err(EngineError::InvalidEdit(_))
                ),
                "{label}: {value}"
            );
        }
        // A radio button is only turned on, and the field must be on the page.
        assert!(matches!(
            doc.set_field_value(0, sizes[0].id, "Off"),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.set_field_value(0, sizes[0].id, "Large"),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.set_field_value(0, FieldId(9_999), "x"),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.set_field_value(3, name.id, "x"),
            Err(EngineError::PageOutOfRange(3))
        ));
        assert_eq!(doc.page_fields(0).expect("fields"), before);
    }

    #[test]
    fn a_document_knows_whether_it_has_a_form() {
        let has_form = |name: &str| {
            PdfDocument::from_bytes(&corpus(name))
                .expect("open")
                .has_form()
        };
        assert!(form().has_form());
        assert!(has_form("malicious/field-aa.pdf"));
        assert!(!has_form("benign/single-page.pdf"));
        let mut flattened = form();
        flattened.flatten_form().expect("flatten");
        assert!(!flattened.has_form());
    }

    #[test]
    fn scripts_are_noticed_and_never_run() {
        // /AA keystroke and format scripts: the field is filled in, and the scripts did nothing.
        let mut doc = PdfDocument::from_bytes(&corpus("malicious/field-aa.pdf")).expect("open");
        let [field] = &doc.page_fields(0).expect("fields")[..] else {
            panic!("one field")
        };
        assert!(field.has_script);
        let id = field.id;
        doc.set_field_value(0, id, "12.5").expect("set");
        assert_eq!(doc.page_fields(0).expect("fields")[0].value, "12.5");
        // The sample of fields without scripts says so.
        assert!(
            form()
                .page_fields(0)
                .expect("fields")
                .iter()
                .all(|field| !field.has_script)
        );
    }

    #[test]
    fn flattening_turns_the_fields_into_page_content() {
        let mut doc = form();
        set(&mut doc, "Your name", "Flattened Name").expect("set");
        doc.flatten_form().expect("flatten");
        assert!(doc.page_fields(0).expect("fields").is_empty());
        let (bytes, _) = saved(&doc);
        assert!(!contains(&bytes, b"AcroForm"));
        assert!(!contains(&bytes, b"/FT"));
        assert!(!contains(&bytes, b"/Widget"));
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        assert!(reopened.page_fields(0).expect("fields").is_empty());
        // What the fields showed is on the page now, where the text can be found.
        let text: Vec<String> = reopened
            .page_text(0, 10_000)
            .expect("text")
            .lines
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert!(
            text.iter().any(|line| line.contains("Flattened Name")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|line| line.contains("Locked value")),
            "{text:?}"
        );
        // A signed document keeps its fields: flattening would make its signatures worthless.
        let mut signed = PdfDocument::from_bytes(&corpus("benign/signed.pdf")).expect("open");
        assert!(matches!(
            signed.flatten_form(),
            Err(EngineError::InvalidEdit(_))
        ));
    }

    #[test]
    fn annotation_numbers_stay_the_same_through_saving_and_opening_again() {
        // Saving drops what nothing uses but does not renumber (garbage level 1): the numbers
        // the page was told stay true if the worker has to open the saved file again.
        let mut doc = PdfDocument::from_bytes(&annotated_pdf()).expect("open");
        doc.add_note(0, Point { x: 400.0, y: 400.0 }, "New note")
            .expect("note");
        doc.delete_annotation(0, AnnotationId(7)).expect("delete");
        let numbered = |doc: &PdfDocument| -> Vec<(u32, Option<String>)> {
            doc.page_annotations(0)
                .expect("annotations")
                .into_iter()
                .map(|annotation| (annotation.id.0, annotation.text))
                .collect()
        };
        let before = numbered(&doc);
        assert!(before.len() >= 4, "{before:?}");
        let (bytes, _) = saved(&doc);
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        assert_eq!(numbered(&reopened), before);
    }

    /// The pages `pages` of `doc` as a document of their own, opened again.
    fn pages_of(doc: &PdfDocument, pages: &[u32]) -> (PdfDocument, Vec<u8>) {
        let mut out = Vec::new();
        doc.pages_copy(pages, &mut out).expect("copy");
        (PdfDocument::from_bytes(&out).expect("reopen"), out)
    }

    #[test]
    fn some_pages_are_a_document_of_their_own_in_the_order_of_the_document() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        let (copy, bytes) = pages_of(&doc, &[3, 1, 2]);
        assert_eq!(copy.page_count().expect("count"), 3);
        // However they were named, they keep the order they have in the document.
        assert_eq!(page_titles(&copy), titles(&[2, 3, 4]));
        // Nothing of the other pages comes with them: not their text, not their size in bytes.
        for page in 0..3 {
            let found = copy.search_page(page, "needle", false, 10).expect("search");
            assert!(found.hits.is_empty());
        }
        assert!(bytes.len() < corpus("benign/multi-page-10.pdf").len());
        assert!(!bytes.windows(6).any(|window| window == b"needle"));
        // The open document is as it was.
        assert_eq!(doc.page_count().expect("count"), 10);
        assert_eq!(page_titles(&doc), titles(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]));
        // The page that has the keyword, copied alone, has it.
        let (seven, _) = pages_of(&doc, &[6]);
        assert_eq!(
            seven
                .search_page(0, "needle", false, 10)
                .expect("search")
                .hits
                .len(),
            1
        );
    }

    #[test]
    fn the_pages_are_copied_as_they_are_now_edits_included() {
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        doc.delete_pages(&[0, 1]).expect("delete");
        doc.rotate_pages(&[0], 90).expect("rotate");
        let (copy, _) = pages_of(&doc, &[0, 1]);
        // What were pages 3 and 4, the first of them turned.
        assert_eq!(page_titles(&copy), titles(&[3, 4]));
        assert_eq!(copy.page_size(0).expect("size"), (792.0, 612.0));
        assert_eq!(copy.page_size(1).expect("size"), (612.0, 792.0));
    }

    #[test]
    fn a_page_keeps_its_size_and_what_was_added_to_it() {
        let doc = PdfDocument::from_bytes(&corpus("benign/mixed-page-sizes.pdf")).expect("open");
        let sizes: Vec<(f32, f32)> = (0..4)
            .map(|page| doc.page_size(page).expect("size"))
            .collect();
        let (copy, _) = pages_of(&doc, &[3, 0]);
        assert_eq!(copy.page_size(0).expect("size"), sizes[0]);
        assert_eq!(copy.page_size(1).expect("size"), sizes[3]);

        let annotated = PdfDocument::from_bytes(&annotated_pdf()).expect("open");
        let (copy, _) = pages_of(&annotated, &[0]);
        let kinds: Vec<AnnotationKind> = copy
            .page_annotations(0)
            .expect("annotations")
            .iter()
            .map(|annotation| annotation.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                AnnotationKind::Highlight,
                AnnotationKind::Note,
                AnnotationKind::Other,
                AnnotationKind::Note
            ]
        );
    }

    #[test]
    fn no_page_is_copied_that_is_not_there_or_twice_or_none_and_nothing_is_written() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        for wrong in [vec![], vec![10], vec![1, 1], vec![0, 99]] {
            let mut out = Vec::new();
            assert!(doc.pages_copy(&wrong, &mut out).is_err(), "{wrong:?}");
            assert!(out.is_empty());
        }
        // An encrypted document is refused, as for the privacy export.
        let encrypted =
            PdfDocument::open(&corpus("benign/encrypted-aes256.pdf"), Some("user")).expect("open");
        let mut out = Vec::new();
        assert!(matches!(
            encrypted.pages_copy(&[0], &mut out),
            Err(EngineError::EncryptedCopy)
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn rotating_pages_turns_them_on_top_of_their_rotation() {
        let mut doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        doc.rotate_pages(&[0], 90).expect("rotate");
        assert_eq!(doc.page_size(0).expect("size"), (792.0, 612.0));
        assert_eq!(doc.page_size(1).expect("size"), (612.0, 792.0));
        // Three more quarter turns bring it back.
        doc.rotate_pages(&[0], 270).expect("rotate");
        assert_eq!(doc.page_size(0).expect("size"), (612.0, 792.0));
    }

    #[test]
    fn a_bad_rotation_changes_nothing() {
        let mut doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        assert!(matches!(
            doc.rotate_pages(&[0, 2], 90),
            Err(EngineError::PageOutOfRange(2))
        ));
        assert!(matches!(
            doc.rotate_pages(&[0], 45),
            Err(EngineError::InvalidRotation)
        ));
        assert_eq!(doc.page_size(0).expect("size"), (612.0, 792.0));
    }

    /// The first line of text on each page: which page is where.
    fn page_titles(doc: &PdfDocument) -> Vec<String> {
        (0..doc.page_count().expect("count"))
            .map(|page| {
                doc.page_text(page, 1_000)
                    .expect("text")
                    .lines
                    .first()
                    .map_or_else(String::new, |line| line.text.clone())
            })
            .collect()
    }

    /// The titles of `multi-page-10.pdf`'s pages `numbers` (1-based).
    fn titles(numbers: &[u32]) -> Vec<String> {
        numbers.iter().map(|n| format!("Page {n} of 10")).collect()
    }

    #[test]
    fn deleting_pages_removes_them_and_leaves_one_at_least() {
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        doc.delete_pages(&[9, 0, 4]).expect("delete");
        assert_eq!(page_titles(&doc), titles(&[2, 3, 4, 6, 7, 8, 9]));
        // Out of range, twice, none or all: refused, and nothing changes.
        assert!(matches!(
            doc.delete_pages(&[7]),
            Err(EngineError::PageOutOfRange(7))
        ));
        assert!(matches!(
            doc.delete_pages(&[1, 1]),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.delete_pages(&[]),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.delete_pages(&(0..7).collect::<Vec<_>>()),
            Err(EngineError::NoPageLeft)
        ));
        assert_eq!(doc.page_count().expect("count"), 7);
        doc.delete_pages(&(1..7).collect::<Vec<_>>())
            .expect("all but one");
        assert_eq!(page_titles(&doc), titles(&[2]));
    }

    #[test]
    fn moved_pages_go_together_before_a_page() {
        let open = || PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        for (pages, before, expected) in [
            // Page 5 to the front (the card's example).
            (vec![4], 0, vec![5, 1, 2, 3, 4, 6, 7, 8, 9, 10]),
            // Two pages, listed out of order, before page 9: together, in document order.
            (vec![5, 1], 8, vec![1, 3, 4, 5, 7, 8, 2, 6, 9, 10]),
            (vec![0, 2], 10, vec![2, 4, 5, 6, 7, 8, 9, 10, 1, 3]),
            (vec![9, 8], 1, vec![1, 9, 10, 2, 3, 4, 5, 6, 7, 8]),
            // Before one of themselves, or all of them: already in place.
            (vec![3, 4], 4, (1..=10).collect()),
            ((0..10).collect(), 10, (1..=10).collect()),
        ] {
            let mut doc = open();
            doc.move_pages(&pages, before).expect("move");
            assert_eq!(
                page_titles(&doc),
                titles(&expected),
                "{pages:?} before {before}"
            );
        }
        let mut doc = open();
        assert!(matches!(
            doc.move_pages(&[0], 11),
            Err(EngineError::PageOutOfRange(11))
        ));
        assert!(matches!(
            doc.move_pages(&[10], 0),
            Err(EngineError::PageOutOfRange(10))
        ));
        assert!(matches!(
            doc.move_pages(&[2, 2], 0),
            Err(EngineError::InvalidEdit(_))
        ));
        assert_eq!(page_titles(&doc), titles(&(1..=10).collect::<Vec<_>>()));
    }

    #[test]
    fn a_blank_page_goes_where_asked_the_size_another_is_shown_at() {
        let mut doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        // Page 1 turned a quarter: shown landscape.
        doc.rotate_pages(&[0], 90).expect("rotate");
        doc.insert_blank_page(0, 0).expect("before the first");
        doc.insert_blank_page(3, 2).expect("after the last");
        let sizes: Vec<_> = (0..doc.page_count().expect("count"))
            .map(|page| doc.page_size(page).expect("size"))
            .collect();
        assert_eq!(
            sizes,
            [
                (792.0, 612.0),
                (792.0, 612.0),
                (612.0, 792.0),
                (612.0, 792.0)
            ]
        );
        // Blank and upright: landscape by its own size, not turned.
        assert!(doc.page_text(0, 100).expect("text").lines.is_empty());
        assert_eq!(
            doc.doc
                .load_pdf_page(0)
                .expect("page")
                .rotation()
                .expect("rotation"),
            0
        );
        assert!(matches!(
            doc.insert_blank_page(5, 0),
            Err(EngineError::PageOutOfRange(5))
        ));
        assert!(matches!(
            doc.insert_blank_page(0, 4),
            Err(EngineError::PageOutOfRange(4))
        ));
        assert_eq!(doc.page_count().expect("count"), 4);
    }

    #[test]
    fn a_deleted_page_leaves_the_file_even_where_a_link_pointed_to_it() {
        // Page 1 links to page 3 (a destination) and to page 2 (a GoTo action).
        let mut doc = PdfDocument::from_bytes(&corpus("benign/internal-links.pdf")).expect("open");
        doc.delete_pages(&[2]).expect("delete page 3");
        let (file, _) = saved(&doc);
        assert!(
            !contains(&file, b"(Page 3) Tj"),
            "page 3 is still in the file"
        );
        assert!(contains(&file, b"(Page 2) Tj"));
        // Both links are still there; the one to page 3 goes nowhere.
        let reopened = PdfDocument::from_bytes(&file).expect("reopen");
        let targets: Vec<_> = reopened
            .page_links(0, 10)
            .expect("links")
            .into_iter()
            .map(|link| link.target)
            .collect();
        assert_eq!(targets, [OutlineTarget::None, OutlineTarget::Page(1)]);
    }

    #[test]
    fn a_deleted_page_leaves_the_file_even_where_the_outline_pointed_to_it() {
        let mut doc =
            PdfDocument::from_bytes(&corpus("benign/outline-3-levels.pdf")).expect("open");
        doc.delete_pages(&[2]).expect("delete");
        let (file, _) = saved(&doc);
        assert!(!contains(&file, b"(Subsection 1.1.1) Tj"));
        let outline = PdfDocument::from_bytes(&file)
            .expect("reopen")
            .outline(100, 64)
            .expect("outline");
        let targets: Vec<_> = outline
            .entries
            .iter()
            .map(|entry| (entry.title.as_str(), entry.target.clone()))
            .collect();
        assert_eq!(
            targets,
            [
                ("Chapter 1", OutlineTarget::Page(0)),
                ("Section 1.1", OutlineTarget::Page(1)),
                ("Subsection 1.1.1", OutlineTarget::None),
                ("Chapter 2", OutlineTarget::Page(2)),
                ("Section 2.1", OutlineTarget::Page(3)),
                ("Appendix", OutlineTarget::Page(4)),
            ]
        );
    }

    #[test]
    fn a_deleted_page_leaves_neither_its_form_fields_nor_its_structure_behind() {
        // Each page has a filled field; page two's figure has alt text (tests/corpus).
        let mut doc =
            PdfDocument::from_bytes(&corpus("benign/tagged-form-two-pages.pdf")).expect("open");
        doc.delete_pages(&[1]).expect("delete page two");
        let (file, _) = saved(&doc);
        assert!(!contains(&file, b"Figure alt text that only page two has"));
        assert!(!contains(&file, b"Field value that only page two has"));
        assert!(contains(&file, b"Field value on page one"));

        let reopened = mupdf::pdf::PdfDocument::from_bytes(&file).expect("reopen");
        let catalog = reopened.catalog().expect("catalog");
        let fields = catalog
            .get_dict("AcroForm")
            .expect("form")
            .expect("form")
            .get_dict("Fields")
            .expect("fields")
            .expect("fields");
        assert_eq!(fields.len().expect("fields"), 1);
        // The structure tree keeps page one's paragraph.
        let paragraphs = catalog
            .get_dict("StructTreeRoot")
            .expect("tree")
            .expect("tree")
            .get_dict("K")
            .expect("document")
            .expect("document")
            .get_dict("K")
            .expect("kids")
            .expect("kids");
        assert_eq!(paragraphs.len().expect("kids"), 1);
        let kind = paragraphs
            .get_array(0)
            .expect("kid")
            .expect("kid")
            .get_dict("S")
            .expect("type")
            .expect("type")
            .as_name()
            .expect("name");
        assert_eq!(kind, b"P");
    }

    #[test]
    fn saving_rewrites_the_document_with_its_edits() {
        let mut doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        assert!(!doc.is_signed());
        doc.rotate_pages(&[1], 180).expect("rotate");
        let (file, incremental) = saved(&doc);
        assert!(!incremental);
        assert!(file.starts_with(b"%PDF-"));
        let reopened = mupdf::pdf::PdfDocument::from_bytes(&file).expect("reopen");
        let rotation = |page| {
            reopened
                .load_pdf_page(page)
                .expect("page")
                .rotation()
                .expect("rotation")
        };
        assert_eq!((rotation(0), rotation(1)), (0, 180));
    }

    #[test]
    fn a_signed_document_is_appended_to() {
        let original = corpus("benign/signed.pdf");
        let mut doc = PdfDocument::from_bytes(&original).expect("open");
        assert!(doc.is_signed());
        doc.rotate_pages(&[0], 90).expect("rotate");
        let (file, incremental) = saved(&doc);
        assert!(incremental);
        // The signed bytes come first, unchanged: the signature still covers them.
        assert!(file.len() > original.len());
        assert_eq!(&file[..original.len()], &original[..]);
    }

    #[test]
    fn the_owner_password_lifts_the_authors_restrictions() {
        let restricted = corpus("benign/restricted-open-password.pdf");
        let with_user = PdfDocument::open(&restricted, Some("user"))
            .expect("open")
            .permissions();
        assert_eq!(
            with_user,
            DocumentPermissions {
                copy: false,
                print: false,
                print_high_quality: false,
                ..DocumentPermissions::ALL
            }
        );
        let with_owner = PdfDocument::open(&restricted, Some("owner")).expect("open");
        assert_eq!(with_owner.permissions(), DocumentPermissions::ALL);
        // Without an open password nothing is typed, so the restrictions stay, as in Acrobat.
        let no_open_password =
            PdfDocument::from_bytes(&corpus("benign/restricted-no-copy-no-print.pdf"))
                .expect("open");
        assert!(!no_open_password.permissions().copy);
    }

    #[test]
    fn an_encrypted_document_stays_encrypted() {
        let mut doc =
            PdfDocument::open(&corpus("benign/encrypted-aes256.pdf"), Some("user")).expect("open");
        doc.rotate_pages(&[0], 90).expect("rotate");
        let (file, _) = saved(&doc);
        assert!(matches!(
            PdfDocument::from_bytes(&file),
            Err(EngineError::Encrypted)
        ));
        let reopened = PdfDocument::open(&file, Some("user")).expect("reopen");
        assert_eq!(
            reopened.page_size(0).expect("size"),
            doc.page_size(0).expect("size")
        );
        assert!(matches!(
            PdfDocument::open(&file, Some("wrong")),
            Err(EngineError::WrongPassword)
        ));
    }

    #[test]
    fn nothing_is_written_beyond_the_limit() {
        let mut out = Vec::new();
        let mut limited = Limited {
            out: &mut out,
            left: 4,
        };
        assert_eq!(limited.write(b"%PDF").expect("fits"), 4);
        let error = limited.write(b"-").expect_err("beyond the limit");
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert_eq!(out, b"%PDF");
    }

    #[test]
    fn a_full_disk_is_told_apart() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::StorageFull.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let doc = PdfDocument::from_bytes(&two_page_pdf()).expect("open");
        match doc.save(&mut Full) {
            Err(EngineError::Write(error)) => assert_eq!(error.kind(), io::ErrorKind::StorageFull),
            other => panic!("{other:?}"),
        }
    }

    /// The plain copy `prepare_source` makes of the corpus file `name`.
    fn source_of(name: &str, password: Option<&str>) -> PreparedSource {
        PdfDocument::prepare_source(&corpus(name), password).expect("source")
    }

    /// `bytes` without the data of its streams: what a page shows (its text, say) is not what
    /// the file says.
    fn without_streams(bytes: &[u8]) -> Vec<u8> {
        let (mut kept, mut at) = (Vec::new(), 0);
        while let Some(start) = find(&bytes[at..], b"stream") {
            let from = at + start;
            // "endstream" ends a stream; "stream" alone starts one.
            if bytes[..from].ends_with(b"end") {
                kept.extend_from_slice(&bytes[at..from + 6]);
                at = from + 6;
                continue;
            }
            kept.extend_from_slice(&bytes[at..from + 6]);
            match find(&bytes[from..], b"endstream") {
                Some(end) => at = from + end,
                None => return kept,
            }
        }
        kept.extend_from_slice(&bytes[at..]);
        kept
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Whether `name` (without its slash) is a name in the PDF `bytes`, outside its streams:
    /// followed by a delimiter, so that `/AA` is not `/AAPL`.
    fn has_name(bytes: &[u8], name: &str) -> bool {
        let token = format!("/{name}");
        let bytes = without_streams(bytes);
        bytes.windows(token.len() + 1).any(|window| {
            window.starts_with(token.as_bytes())
                && matches!(
                    window[token.len()],
                    b' ' | b'\r' | b'\n' | b'\t' | b'/' | b'<' | b'>' | b'[' | b']' | b'(' | b')'
                )
        })
    }

    #[test]
    fn the_pages_of_another_file_go_in_at_a_place_in_their_order() {
        let source = source_of("benign/mixed-page-sizes.pdf", None);
        assert_eq!(source.pages, 4);
        let sizes = |doc: &PdfDocument| -> Vec<(u32, u32)> {
            (0..doc.page_count().unwrap())
                .map(|page| {
                    let (width, height) = doc.page_size(page).unwrap();
                    (width.round() as u32, height.round() as u32)
                })
                .collect()
        };
        let own = |doc: &PdfDocument| sizes(doc)[0];
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        let letter = own(&doc);
        let inserted = sizes(&PdfDocument::from_bytes(&source.bytes).expect("copy"));
        assert_eq!(inserted.len(), 4);

        // After the third page.
        doc.insert_pages(3, &source.bytes).expect("insert");
        assert_eq!(doc.page_count().unwrap(), 14);
        let now = sizes(&doc);
        assert_eq!(&now[..3], &[letter; 3]);
        assert_eq!(&now[3..7], &inserted[..]);
        assert_eq!(&now[7..], &[letter; 7]);
        // The pages around are the ones they were, in their order.
        let titles_now = page_titles(&doc);
        assert_eq!(titles_now[..3], titles(&[1, 2, 3])[..]);
        assert_eq!(titles_now[7..], titles(&[4, 5, 6, 7, 8, 9, 10])[..]);

        // At the start and at the end (the page count).
        doc.insert_pages(0, &source.bytes).expect("start");
        assert_eq!(sizes(&doc)[..4], inserted[..]);
        let count = doc.page_count().unwrap();
        doc.insert_pages(count, &source.bytes).expect("end");
        assert_eq!(doc.page_count().unwrap(), count + 4);
        assert_eq!(sizes(&doc)[count as usize..], inserted[..]);
        // Beyond the end there is no place.
        let count = doc.page_count().unwrap();
        assert!(matches!(
            doc.insert_pages(count + 1, &source.bytes),
            Err(EngineError::PageOutOfRange(_))
        ));
        assert_eq!(doc.page_count().unwrap(), count);
    }

    #[test]
    fn inserted_pages_are_in_the_saved_file_and_show_what_they_showed() {
        let source = source_of("benign/single-page.pdf", None);
        let own_text = PdfDocument::from_bytes(&corpus("benign/single-page.pdf"))
            .expect("open")
            .page_text(0, 1_000)
            .expect("text")
            .lines
            .first()
            .map(|line| line.text.clone())
            .expect("a line");
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        doc.insert_pages(2, &source.bytes).expect("insert");
        let (bytes, _) = saved(&doc);
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        assert_eq!(reopened.page_count().unwrap(), 11);
        let titles_now = page_titles(&reopened);
        assert_eq!(titles_now[3], titles(&[3])[0]);
        assert_eq!(titles_now[2], own_text);
        // The page is drawn as it was.
        let drawn = |doc: &PdfDocument, page: u32| doc.render(page, 1.0, 0).expect("render").rgba;
        let original = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).expect("open");
        assert_eq!(drawn(&reopened, 2), drawn(&original, 0));
    }

    #[test]
    fn an_encrypted_file_gives_a_plain_copy_for_its_password_and_none_without() {
        for name in ["benign/encrypted-aes256.pdf", "benign/encrypted-rc4-40.pdf"] {
            let bytes = corpus(name);
            assert!(matches!(
                PdfDocument::prepare_source(&bytes, None),
                Err(EngineError::Encrypted)
            ));
            assert!(matches!(
                PdfDocument::prepare_source(&bytes, Some("not it")),
                Err(EngineError::WrongPassword)
            ));
        }
        let aes = source_of("benign/encrypted-aes256.pdf", Some("user"));
        // Opens with no password at all, and says what the encrypted file did.
        let plain = PdfDocument::from_bytes(&aes.bytes).expect("plain");
        assert!(!plain.is_encrypted());
        assert_eq!(aes.pages, plain.page_count().unwrap());
        let text: Vec<String> = plain
            .page_text(0, 1_000)
            .expect("text")
            .lines
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert!(!text.is_empty());
        // And its pages go into a document.
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        doc.insert_pages(1, &aes.bytes).expect("insert");
        assert_eq!(doc.page_count().unwrap(), 10 + aes.pages);
        assert!(!has_name(&saved(&doc).0, "Encrypt"));
    }

    #[test]
    fn the_author_who_forbids_copying_forbids_taking_pages_out_unless_the_owner_asks() {
        // Nothing to type, and what the author forbids is forbidden.
        let bytes = corpus("benign/restricted-no-copy-no-print.pdf");
        assert!(matches!(
            PdfDocument::prepare_source(&bytes, None),
            Err(EngineError::NotAllowed(_))
        ));
        // With a password: the user's is held to it, and as in Acrobat the owner's lifts every
        // restriction.
        let bytes = corpus("benign/restricted-open-password.pdf");
        assert!(matches!(
            PdfDocument::prepare_source(&bytes, Some("user")),
            Err(EngineError::NotAllowed(_))
        ));
        assert!(PdfDocument::prepare_source(&bytes, Some("owner")).is_ok());
    }

    #[test]
    fn what_is_active_in_a_source_does_not_come_along_and_is_reported() {
        let active = [
            "malicious/openaction-js.pdf",
            "malicious/js-document-level.pdf",
            "malicious/page-aa.pdf",
            "malicious/field-aa.pdf",
            "malicious/launch.pdf",
            "malicious/submitform.pdf",
            "malicious/importdata.pdf",
            "malicious/gotor-unc.pdf",
            "malicious/gotoe.pdf",
            "malicious/xfa.pdf",
            "malicious/embedded-file.pdf",
            "malicious/remote-filespec.pdf",
            "malicious/openaction-uri.pdf",
        ];
        for name in active {
            let source = source_of(name, None);
            // The scan sees it, so that the banner can say so ...
            assert!(!source.security.findings.is_empty(), "{name}");
            // ... and none of it is in a document that took the pages.
            let mut doc =
                PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
            let before = doc.active_content(ScanBudget::default());
            assert!(before.findings.is_empty());
            doc.insert_pages(5, &source.bytes).expect("insert");
            let (bytes, _) = saved(&doc);
            for forbidden in [
                "JavaScript",
                "JS",
                "OpenAction",
                "AA",
                "Launch",
                "SubmitForm",
                "ImportData",
                "GoToR",
                "GoToE",
                "XFA",
                "EmbeddedFile",
                "EmbeddedFiles",
                "AcroForm",
                "URI",
                "FS",
            ] {
                assert!(!has_name(&bytes, forbidden), "{name}: /{forbidden}");
            }
            let after = PdfDocument::from_bytes(&bytes).expect("reopen");
            assert!(
                after
                    .active_content(ScanBudget::default())
                    .findings
                    .is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_source_that_is_no_pdf_or_a_place_that_is_not_there_changes_nothing() {
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        assert!(matches!(
            PdfDocument::prepare_source(b"not a pdf at all", None),
            Err(EngineError::NotPdf)
        ));
        assert!(matches!(
            doc.insert_pages(1, b"not a pdf at all"),
            Err(EngineError::NotPdf)
        ));
        let truncated = &corpus("benign/multi-page-10.pdf")[..200];
        assert!(doc.insert_pages(1, truncated).is_err());
        assert_eq!(doc.page_count().unwrap(), 10);
        assert_eq!(page_titles(&doc), titles(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]));
    }
}
