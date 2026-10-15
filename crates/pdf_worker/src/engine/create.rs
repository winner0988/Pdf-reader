//! A PDF made of files that are not one (B3-03, docs/architecture/create-pdf.md): pictures (each a
//! page), a web page, plain text, an EPUB, an XPS file, an SVG picture, and Word, Excel and
//! PowerPoint files (the Office ones are experimental: MuPDF makes of them what it can).
//!
//! Every source is untrusted and MuPDF reads it in this sandboxed process: no script runs, nothing
//! is fetched (the worker has no network, and a source is given as bytes, so a page that points at
//! other files finds none), and what is read is bounded in size and in pages. A source that cannot
//! be read is an error, never a half-made PDF.
//!
//! What is in the PDF is what is in the source's content and nothing about the source: the
//! metadata of an Office file or an EPUB (author, dates) is not carried, and a picture brings its
//! pixels only (a JPEG without the camera's, the place's and the time's segments, a PNG or a
//! TIFF without its text and tags).

use std::borrow::Cow;
use std::io::Write;
use std::sync::Mutex;

use ipc_contract::limits::MAX_PAGE_COUNT;
use ipc_contract::types::CreateKind;
use mupdf::pdf::{
    InsertImageOptions, PageImageSource, PdfDocument as MuPdfDocument, PdfWriteOptions,
};
use mupdf::{Document, Image, Rect, Size};

use super::{EngineError, write_limited};
use crate::jpeg;

/// The most pixels a picture may have: 64 megapixels are 256 MB as RGBA.
const MAX_PIXELS: u64 = 64_000_000;
/// The longest side of a JPEG or a PNG, in pixels.
const MAX_SIDE: u32 = 65_535;
/// The most bytes the pixels of all the pictures that are not JPEGs may take together while the
/// PDF is made: MuPDF keeps them as they are, not compressed, until it writes the PDF.
const MAX_RAW_BYTES: u64 = 768 * 1024 * 1024;

/// How a page of a picture is sized: the picture at its own resolution (96 dpi when it says none),
/// but no more than this on its longer side, in points (A3), and no less than this on it.
const LONGEST_SIDE_PT: f32 = 1190.0;
const SHORTEST_LONGEST_SIDE_PT: f32 = 144.0;
const DEFAULT_DPI: f32 = 96.0;

/// What a page of a text, a web page or a book is laid out to: A4, at 11 points a letter, with a
/// margin of three quarters of an inch on each side (a stylesheet of the user's, which MuPDF
/// reads for an `@page` rule).
const LAYOUT_WIDTH_PT: f32 = 595.0;
const LAYOUT_HEIGHT_PT: f32 = 842.0;
const LAYOUT_EM_PT: f32 = 11.0;
const USER_CSS: &str = "@page { margin: 54pt }";

/// MuPDF keeps the user stylesheet where every thread's context sees it, and replaces it when one
/// sets it: one source is read at a time in this process, so that none is read with another's.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// What was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Created {
    /// The bytes written.
    pub bytes: u64,
    pub pages: u32,
}

/// A PDF being made of sources, one after the other.
pub struct PdfBuilder {
    pdf: Option<MuPdfDocument>,
    /// Whether the document is of pictures, which more pictures can be added to.
    pictures: bool,
    pages: u32,
    /// How many bytes of pixels more pictures that are not JPEGs may take.
    raw_left: u64,
}

impl Default for PdfBuilder {
    fn default() -> Self {
        Self {
            pdf: None,
            pictures: false,
            pages: 0,
            raw_left: MAX_RAW_BYTES,
        }
    }
}

impl PdfBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the source `bytes` of `kind`. Pictures go together, one page each; any other source
    /// is the whole document, alone.
    pub fn add(&mut self, bytes: &[u8], kind: CreateKind) -> Result<(), EngineError> {
        match kind {
            CreateKind::Image => {
                if self.pdf.is_some() && !self.pictures {
                    return Err(EngineError::InvalidEdit("only pictures go together"));
                }
                self.add_picture(bytes)
            }
            _ => {
                if self.pdf.is_some() {
                    return Err(EngineError::InvalidEdit("only pictures go together"));
                }
                self.pdf = Some(convert(bytes, kind)?);
                self.pages = u32::try_from(self.pdf.as_ref().expect("just made").page_count()?)
                    .unwrap_or(u32::MAX);
                Ok(())
            }
        }
    }

    /// The pages so far.
    pub fn pages(&self) -> u32 {
        self.pages
    }

    /// Writes the PDF to `out`: without what nothing uses and without any information about the
    /// sources (the document information dictionary is dropped).
    pub fn finish(self, out: &mut impl Write) -> Result<Created, EngineError> {
        let pdf = self
            .pdf
            .ok_or(EngineError::InvalidEdit("no source to make a PDF of"))?;
        let mut trailer = pdf.trailer()?;
        trailer.dict_delete("Info")?;
        // A document MuPDF makes has its producer line in its catalog too.
        let mut catalog = pdf.catalog()?;
        catalog.dict_delete("Info")?;
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        options.set_compress(true);
        options.set_compress_images(true);
        options.set_compress_fonts(true);
        let bytes = write_limited(&pdf, out, options)?;
        Ok(Created {
            bytes,
            pages: self.pages,
        })
    }

    /// A page of the picture `bytes`: the page is the picture at its own resolution and the
    /// picture fills it. A JPEG goes in as the file it is without its metadata; any other kind of
    /// picture goes in as its pixels.
    fn add_picture(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        if self.pages >= MAX_PAGE_COUNT {
            return Err(EngineError::TooManyPages);
        }
        let (data, orientation): (Cow<[u8]>, u8) = match jpeg::clean(bytes) {
            Some(cleaned) => (Cow::Owned(cleaned.bytes), cleaned.orientation),
            None => (Cow::Borrowed(bytes), 1),
        };
        let is_jpeg = matches!(data, Cow::Owned(_));
        let invalid = |_| EngineError::InvalidSource("not a picture MuPDF reads");
        let picture = Image::from_bytes(&data).map_err(invalid)?;
        let (width, height) = (picture.width(), picture.height());
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return Err(EngineError::PictureTooLarge);
        }
        let (x_dpi, y_dpi) = picture.resolution();
        let dpi = |value: i32| {
            if (1..=10_000).contains(&value) {
                value as f32
            } else {
                DEFAULT_DPI
            }
        };
        // Whatever is not a JPEG is made pixels first: nothing but them is in the PDF.
        let picture = if is_jpeg {
            picture
        } else {
            let pixmap = picture.to_pixmap().map_err(invalid)?;
            self.raw_left = self
                .raw_left
                .checked_sub(pixmap.samples().len() as u64)
                .ok_or(EngineError::PictureTooLarge)?;
            Image::from_pixmap(&pixmap).map_err(invalid)?
        };
        let (mut page_width, mut page_height) = (
            width as f32 * 72.0 / dpi(x_dpi),
            height as f32 * 72.0 / dpi(y_dpi),
        );
        // The page is kept the shape of the picture, within the sizes a page may have here; a
        // picture taken on its side is shown the right way up by turning the page.
        let (turned_width, turned_height) = if matches!(orientation, 5..=8) {
            (page_height, page_width)
        } else {
            (page_width, page_height)
        };
        let longest = turned_width.max(turned_height);
        let scale = if longest > LONGEST_SIDE_PT {
            LONGEST_SIDE_PT / longest
        } else if longest < SHORTEST_LONGEST_SIDE_PT {
            SHORTEST_LONGEST_SIDE_PT / longest
        } else {
            1.0
        };
        page_width *= scale;
        page_height *= scale;
        let rotation = match orientation {
            3 | 4 => 180,
            5 | 6 => 90,
            7 | 8 => 270,
            _ => 0,
        };
        let pdf = self.pdf.get_or_insert_with(MuPdfDocument::new);
        let mut page = pdf.new_page(Size {
            width: page_width,
            height: page_height,
        })?;
        page.insert_image(
            pdf,
            Rect {
                x0: 0.0,
                y0: 0.0,
                x1: page_width,
                y1: page_height,
            },
            PageImageSource::Image(&picture),
            InsertImageOptions::default(),
        )?;
        if rotation != 0 {
            page.set_rotation(rotation)?;
        }
        self.pictures = true;
        self.pages += 1;
        Ok(())
    }
}

/// The PDF MuPDF makes of the one document `bytes` of `kind` (not a picture).
fn convert(bytes: &[u8], kind: CreateKind) -> Result<MuPdfDocument, EngineError> {
    let (magic, text) = match kind {
        CreateKind::Html => (".html", false),
        CreateKind::Text => (".txt", true),
        CreateKind::Epub => (".epub", false),
        CreateKind::Xps => (".xps", false),
        CreateKind::Svg => (".svg", false),
        CreateKind::Docx => (".docx", false),
        CreateKind::Xlsx => (".xlsx", false),
        CreateKind::Pptx => (".pptx", false),
        CreateKind::Image => unreachable!("a picture is a page of its own"),
    };
    let utf8;
    let bytes = if text {
        utf8 = utf8_text(bytes)?;
        &utf8[..]
    } else {
        bytes
    };
    if bytes.starts_with(b"PK")
        || bytes[bytes.len().saturating_sub(22 + 65_535)..]
            .windows(4)
            .any(|w| w == b"PK\x05\x06")
        || (matches!(
            kind,
            CreateKind::Epub
                | CreateKind::Xps
                | CreateKind::Docx
                | CreateKind::Xlsx
                | CreateKind::Pptx
        ) && !bytes.starts_with(b"%PDF-"))
    {
        check_zip_limits(bytes)?;
    }
    let unreadable = |_| EngineError::InvalidSource("MuPDF cannot read this file");
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    mupdf::Context::get()
        .set_user_css(USER_CSS)
        .map_err(unreadable)?;
    let mut document = Document::from_bytes(bytes, magic).map_err(unreadable)?;
    if document.needs_password().map_err(unreadable)? {
        return Err(EngineError::InvalidSource("the file needs a password"));
    }
    if document.is_reflowable().map_err(unreadable)? {
        document
            .layout(LAYOUT_WIDTH_PT, LAYOUT_HEIGHT_PT, LAYOUT_EM_PT)
            .map_err(unreadable)?;
    }
    let pages = document.page_count().map_err(unreadable)?;
    if pages <= 0 {
        return Err(EngineError::InvalidSource("the file has no pages"));
    }
    if u32::try_from(pages).unwrap_or(u32::MAX) > MAX_PAGE_COUNT {
        return Err(EngineError::TooManyPages);
    }
    // Pre-validate every page into a display list before calling `convert_to_pdf`:
    // `mupdf_convert_to_pdf_internal` leaves its stack `fz_page *page` uninitialized and leaks
    // `pdfout` if `fz_load_page` or `fz_run_page` throws midway through the loop.
    for index in 0..pages {
        let page = document.load_page(index).map_err(unreadable)?;
        let bounds = page.bounds().map_err(unreadable)?;
        let (width, height) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(EngineError::InvalidSource("MuPDF cannot read this file"));
        }
        let _ = page.to_display_list(true).map_err(unreadable)?;
    }
    document.convert_to_pdf(0, pages - 1, 0).map_err(unreadable)
}

/// Checks a ZIP container (EPUB, XPS, DOCX, XLSX, PPTX) before handing it to MuPDF's zip reader:
/// MuPDF allocates the declared uncompressed size of an entry up front (`fz_malloc(ctx, usize)`)
/// and only rejects negative 32-bit signed integers (`usize > 0x7FFF_FFFF`), so a 1.5 GiB declared
/// size in a tiny file would immediately exhaust the fuzzer's or worker's memory limit.
fn check_zip_limits(bytes: &[u8]) -> Result<(), EngineError> {
    const MAX_ZIP_ENTRIES: usize = 4_096;
    const MAX_ZIP_UNPACKED_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_ZIP_RATIO: u64 = 200;

    let bad = || EngineError::InvalidSource("MuPDF cannot read this file");
    if bytes.len() < 22 {
        return Err(bad());
    }
    let search_start = bytes.len().saturating_sub(22 + 65_535);
    let mut found_eocd = false;
    for pos in (search_start..=bytes.len() - 22).rev() {
        if &bytes[pos..pos + 4] != b"PK\x05\x06" {
            continue;
        }
        found_eocd = true;
        let count = usize::from(u16::from_le_bytes([bytes[pos + 10], bytes[pos + 11]]));
        let cdir_size = usize::from_le_bytes([
            bytes[pos + 12],
            bytes[pos + 13],
            bytes[pos + 14],
            bytes[pos + 15],
            0,
            0,
            0,
            0,
        ]);
        let start_offset = usize::from_le_bytes([
            bytes[pos + 16],
            bytes[pos + 17],
            bytes[pos + 18],
            bytes[pos + 19],
            0,
            0,
            0,
            0,
        ]);
        if count > MAX_ZIP_ENTRIES || count == 0xFFFF || start_offset == 0xFFFF_FFFF {
            return Err(bad());
        }
        let Some(cdir_end) = start_offset.checked_add(cdir_size) else {
            return Err(bad());
        };
        if cdir_end > pos {
            return Err(bad());
        }
        let mut offset = start_offset;
        let mut total_unpacked: u64 = 0;
        for _ in 0..count {
            let Some(header_end) = offset.checked_add(46) else {
                return Err(bad());
            };
            if header_end > pos || &bytes[offset..offset + 4] != b"PK\x01\x02" {
                return Err(bad());
            }
            let flags = u16::from_le_bytes([bytes[offset + 8], bytes[offset + 9]]);
            let csize = u64::from(u32::from_le_bytes([
                bytes[offset + 20],
                bytes[offset + 21],
                bytes[offset + 22],
                bytes[offset + 23],
            ]));
            let usize_val = u64::from(u32::from_le_bytes([
                bytes[offset + 24],
                bytes[offset + 25],
                bytes[offset + 26],
                bytes[offset + 27],
            ]));
            let namesize =
                usize::from(u16::from_le_bytes([bytes[offset + 28], bytes[offset + 29]]));
            let metasize =
                usize::from(u16::from_le_bytes([bytes[offset + 30], bytes[offset + 31]]));
            let commentsize =
                usize::from(u16::from_le_bytes([bytes[offset + 32], bytes[offset + 33]]));
            let local_offset = usize::from_le_bytes([
                bytes[offset + 42],
                bytes[offset + 43],
                bytes[offset + 44],
                bytes[offset + 45],
                0,
                0,
                0,
                0,
            ]);
            let Some(local_header_end) = local_offset.checked_add(30) else {
                return Err(bad());
            };
            if local_header_end > start_offset
                || &bytes[local_offset..local_offset + 4] != b"PK\x03\x04"
            {
                return Err(bad());
            }
            let local_flags =
                u16::from_le_bytes([bytes[local_offset + 6], bytes[local_offset + 7]]);
            let method = u16::from_le_bytes([bytes[local_offset + 8], bytes[local_offset + 9]]);
            if (flags & 1) != 0 || (local_flags & 1) != 0 {
                return Err(bad());
            }
            let local_csize = u64::from(u32::from_le_bytes([
                bytes[local_offset + 18],
                bytes[local_offset + 19],
                bytes[local_offset + 20],
                bytes[local_offset + 21],
            ]));
            let local_usize = u64::from(u32::from_le_bytes([
                bytes[local_offset + 22],
                bytes[local_offset + 23],
                bytes[local_offset + 24],
                bytes[local_offset + 25],
            ]));
            let local_name = usize::from(u16::from_le_bytes([
                bytes[local_offset + 26],
                bytes[local_offset + 27],
            ]));
            let local_extra = usize::from(u16::from_le_bytes([
                bytes[local_offset + 28],
                bytes[local_offset + 29],
            ]));
            let Some(data_start) = local_header_end
                .checked_add(local_name)
                .and_then(|v| v.checked_add(local_extra))
            else {
                return Err(bad());
            };
            if csize == 0xFFFF_FFFF
                || usize_val == 0xFFFF_FFFF
                || local_csize == 0xFFFF_FFFF
                || local_usize == 0xFFFF_FFFF
            {
                return Err(bad());
            }
            let effective_csize = local_csize.max(csize);
            let effective_usize = local_usize.max(usize_val);
            let Some(data_end) = data_start.checked_add(effective_csize as usize) else {
                return Err(bad());
            };
            if data_end > start_offset {
                return Err(bad());
            }
            if method == 0 && effective_usize != effective_csize {
                return Err(bad());
            }
            if effective_usize > 1024 * 1024
                && effective_usize > effective_csize.saturating_mul(MAX_ZIP_RATIO)
            {
                return Err(bad());
            }
            total_unpacked = total_unpacked.saturating_add(effective_usize);
            if total_unpacked > MAX_ZIP_UNPACKED_BYTES {
                return Err(bad());
            }
            offset = header_end
                .checked_add(namesize)
                .and_then(|v| v.checked_add(metasize))
                .and_then(|v| v.checked_add(commentsize))
                .ok_or_else(bad)?;
        }
    }
    if !found_eocd {
        return Err(bad());
    }
    Ok(())
}

/// The text of a file as UTF-8 without a byte order mark: UTF-8 as it is, UTF-16 (with its mark)
/// converted. Anything else is refused: MuPDF would read the bytes of another encoding as
/// something it is not, and a page of garbage is worse than none.
fn utf8_text(bytes: &[u8]) -> Result<Cow<'_, [u8]>, EngineError> {
    let not_text = || EngineError::InvalidSource("the text is not UTF-8 or UTF-16");
    let empty = || EngineError::InvalidSource("the text has nothing in it");
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text = std::str::from_utf8(rest).map_err(|_| not_text())?;
        return if text.trim().is_empty() {
            Err(empty())
        } else {
            Ok(Cow::Borrowed(rest))
        };
    }
    let little = bytes.starts_with(&[0xFF, 0xFE]);
    if little || bytes.starts_with(&[0xFE, 0xFF]) {
        // A last half unit, if there is one, is left out.
        let units = bytes[2..].as_chunks::<2>().0.iter().map(|&pair| {
            if little {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        });
        let text: String = char::decode_utf16(units)
            .collect::<Result<_, _>>()
            .map_err(|_| not_text())?;
        return if text.trim().is_empty() {
            Err(empty())
        } else {
            Ok(Cow::Owned(text.into_bytes()))
        };
    }
    let text = std::str::from_utf8(bytes).map_err(|_| not_text())?;
    if text.trim().is_empty() {
        return Err(empty());
    }
    Ok(Cow::Borrowed(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::PdfDocument as Engine;

    const KINDS_FOR_TEST: [CreateKind; 9] = [
        CreateKind::Image,
        CreateKind::Html,
        CreateKind::Text,
        CreateKind::Epub,
        CreateKind::Xps,
        CreateKind::Svg,
        CreateKind::Docx,
        CreateKind::Xlsx,
        CreateKind::Pptx,
    ];

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// A ZIP archive of `files`, stored as they are.
    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let crc = |data: &[u8]| {
            let mut value = !0u32;
            for &byte in data {
                value ^= u32::from(byte);
                for _ in 0..8 {
                    value = if value & 1 == 1 {
                        0xEDB8_8320 ^ (value >> 1)
                    } else {
                        value >> 1
                    };
                }
            }
            !value
        };
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, text) in files {
            let data = text.as_bytes();
            let offset = out.len() as u32;
            let (checksum, size, name_length) = (crc(data), data.len() as u32, name.len() as u16);
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
            out.extend_from_slice(&checksum.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_length.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
            directory.extend_from_slice(&checksum.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&name_length.to_le_bytes());
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn made(sources: &[(&[u8], CreateKind)]) -> Result<(Vec<u8>, Created), EngineError> {
        let mut builder = PdfBuilder::new();
        for (bytes, kind) in sources {
            builder.add(bytes, *kind)?;
        }
        let mut out = Vec::new();
        let created = builder.finish(&mut out)?;
        assert_eq!(created.bytes, out.len() as u64);
        // No document information, not even the line MuPDF puts in what it makes.
        assert!(!contains(&out, b"/Info") && !contains(&out, b"/Producer"));
        Ok((out, created))
    }

    fn one(bytes: &[u8], kind: CreateKind) -> (Vec<u8>, Created) {
        made(&[(bytes, kind)]).unwrap_or_else(|e| panic!("{kind:?}: {e}"))
    }

    /// The text of all pages of `pdf`, as the engine reads it.
    fn text_of(pdf: &[u8]) -> String {
        let doc = Engine::from_bytes(pdf).expect("a PDF");
        let mut text = String::new();
        for page in 0..doc.page_count().unwrap() {
            for line in doc.page_text(page, 100_000).unwrap().lines {
                text.push_str(&line.text);
                text.push('\n');
            }
        }
        text
    }

    fn size_of(pdf: &[u8], page: u32) -> (f32, f32) {
        Engine::from_bytes(pdf).unwrap().page_size(page).unwrap()
    }

    const HTML: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>A title</title>\
<link rel=\"stylesheet\" href=\"https://example.invalid/a.css\">\
<script>document.write(\"SCRIPT RAN\")</script></head>\
<body><h1 id=\"top\">Web page heading</h1><p>Hello <b>world</b> 你好</p>\
<p><a href=\"https://example.invalid/page\">a link</a> <a href=\"file:///C:/Windows/win.ini\">a file</a> \
<a href=\"javascript:alert(1)\">a script</a> <a href=\"#top\">the top</a></p>\
<img src=\"http://example.invalid/x.png\" alt=\"remote picture\"></body></html>";

    const EPUB_CHAPTER: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>c</title></head>\
<body><h1>Chapter one</h1><p>The book's first words.</p></body></html>";

    fn epub() -> Vec<u8> {
        zip(&[
            ("mimetype", "application/epub+zip"),
            (
                "META-INF/container.xml",
                "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\
<rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>",
            ),
            (
                "content.opf",
                "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"id\">\
<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Sample</dc:title><dc:creator>Jane Q. Private-Author</dc:creator>\
<dc:identifier id=\"id\">x</dc:identifier><dc:language>en</dc:language></metadata>\
<manifest><item id=\"c1\" href=\"chapter1.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>\
<spine><itemref idref=\"c1\"/></spine></package>",
            ),
            ("chapter1.xhtml", EPUB_CHAPTER),
        ])
    }

    fn xps() -> Vec<u8> {
        zip(&[
            (
                "[Content_Types].xml",
                "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"fdseq\" ContentType=\"application/vnd.ms-package.xps-fixeddocumentsequence+xml\"/>\
<Default Extension=\"fdoc\" ContentType=\"application/vnd.ms-package.xps-fixeddocument+xml\"/>\
<Default Extension=\"fpage\" ContentType=\"application/vnd.ms-package.xps-fixedpage+xml\"/></Types>",
            ),
            (
                "_rels/.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"R1\" Type=\"http://schemas.microsoft.com/xps/2005/06/fixedrepresentation\" Target=\"FixedDocumentSequence.fdseq\"/></Relationships>",
            ),
            (
                "FixedDocumentSequence.fdseq",
                "<?xml version=\"1.0\"?><FixedDocumentSequence xmlns=\"http://schemas.microsoft.com/xps/2005/06\">\
<DocumentReference Source=\"Documents/1/FixedDocument.fdoc\"/></FixedDocumentSequence>",
            ),
            (
                "Documents/1/FixedDocument.fdoc",
                "<?xml version=\"1.0\"?><FixedDocument xmlns=\"http://schemas.microsoft.com/xps/2005/06\">\
<PageContent Source=\"Pages/1.fpage\"/></FixedDocument>",
            ),
            (
                "Documents/1/Pages/1.fpage",
                "<?xml version=\"1.0\"?><FixedPage xmlns=\"http://schemas.microsoft.com/xps/2005/06\" Width=\"816\" Height=\"1056\" xml:lang=\"en-US\">\
<Path Fill=\"#FFFF0000\" Data=\"M 96,96 L 400,96 L 400,300 L 96,300 Z\"/></FixedPage>",
            ),
        ])
    }

    const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"300\" height=\"150\">\
<rect width=\"300\" height=\"150\" fill=\"#cde\"/><text x=\"20\" y=\"80\" font-size=\"24\">Hello SVG</text></svg>";

    const WORD: &str = "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>\
<w:p><w:r><w:t>First paragraph of the letter.</w:t></w:r></w:p>\
<w:p><w:r><w:t>Second paragraph 隱私.</w:t></w:r></w:p></w:body></w:document>";

    fn docx() -> Vec<u8> {
        zip(&[
            (
                "[Content_Types].xml",
                "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>",
            ),
            (
                "_rels/.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>",
            ),
            (
                "docProps/core.xml",
                "<?xml version=\"1.0\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Private title</dc:title><dc:creator>Jane Q. Private-Author</dc:creator></cp:coreProperties>",
            ),
            (
                "word/_rels/document.xml.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>",
            ),
            ("word/document.xml", WORD),
        ])
    }

    fn xlsx() -> Vec<u8> {
        zip(&[
            (
                "[Content_Types].xml",
                "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/></Types>",
            ),
            (
                "_rels/.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>",
            ),
            (
                "xl/workbook.xml",
                "<?xml version=\"1.0\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
            ),
            (
                "xl/_rels/workbook.xml.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>\
<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><v>4242</v></c></row></sheetData></worksheet>",
            ),
            (
                "xl/sharedStrings.xml",
                "<?xml version=\"1.0\"?><sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"1\" uniqueCount=\"1\">\
<si><t>Alpha cell</t></si></sst>",
            ),
        ])
    }

    fn pptx() -> Vec<u8> {
        zip(&[
            (
                "[Content_Types].xml",
                "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/></Types>",
            ),
            (
                "_rels/.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>",
            ),
            (
                "ppt/presentation.xml",
                "<?xml version=\"1.0\"?><p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:sldIdLst><p:sldId id=\"256\" r:id=\"rId1\"/></p:sldIdLst><p:sldSz cx=\"9144000\" cy=\"6858000\"/></p:presentation>",
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/></Relationships>",
            ),
            (
                "ppt/slides/slide1.xml",
                "<?xml version=\"1.0\"?><p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/>\
<p:txBody><a:bodyPr/><a:p><a:r><a:t>Slide title text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
            ),
        ])
    }

    #[test]
    fn plain_text_becomes_pages_of_text_with_a_margin() {
        let (pdf, created) = one("Hello text file\n你好，世界\n".as_bytes(), CreateKind::Text);
        assert_eq!(created.pages, 1);
        let text = text_of(&pdf);
        assert!(text.contains("Hello text file"), "{text}");
        assert!(text.contains("你好，世界"), "{text}");
        // An A4 page, with a margin of three quarters of an inch to the text.
        assert_eq!(size_of(&pdf, 0), (595.0, 842.0));
        let first = Engine::from_bytes(&pdf)
            .unwrap()
            .page_text(0, 1000)
            .unwrap()
            .lines[0]
            .quad
            .ul;
        assert!((first.x - 54.0).abs() < 1.0, "{first:?}");
    }

    #[test]
    fn text_in_the_encodings_a_file_has_is_read_and_one_that_is_not_is_refused() {
        let utf16 = |little: bool| {
            let mut bytes = if little {
                vec![0xFF, 0xFE]
            } else {
                vec![0xFE, 0xFF]
            };
            for unit in "UTF sixteen 文字".encode_utf16() {
                bytes.extend_from_slice(&if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            bytes
        };
        let mut with_mark = vec![0xEF, 0xBB, 0xBF];
        with_mark.extend_from_slice("UTF eight with a mark 文字".as_bytes());
        for (bytes, words) in [
            (utf16(true), "UTF sixteen 文字"),
            (utf16(false), "UTF sixteen 文字"),
            (with_mark, "UTF eight with a mark 文字"),
        ] {
            let (pdf, _) = one(&bytes, CreateKind::Text);
            assert!(text_of(&pdf).contains(words), "{}", text_of(&pdf));
        }
        // Big5 is neither: MuPDF would show it as something it is not.
        let big5 = [0xA7u8, 0xDA, 0xA4, 0xA3, 0xAC, 0x4F, 0xB6, 0xC2];
        assert!(matches!(
            made(&[(&big5, CreateKind::Text)]),
            Err(EngineError::InvalidSource(_))
        ));
        // Half of a UTF-16 unit has nothing in it, and a lone surrogate is no text.
        assert!(made(&[(&[0xFF, 0xFE, 0x41], CreateKind::Text)]).is_err());
        assert!(made(&[(&[0xFF, 0xFE, 0x00, 0xD8], CreateKind::Text)]).is_err());
    }

    #[test]
    fn a_web_page_runs_no_script_and_fetches_nothing() {
        let (pdf, created) = one(HTML.as_bytes(), CreateKind::Html);
        assert!(created.pages >= 1);
        let text = text_of(&pdf);
        assert!(text.contains("Web page heading"), "{text}");
        assert!(text.contains("你好"), "{text}");
        // The script is not run, and its words are not shown; what it names is not in the PDF.
        assert!(!text.contains("SCRIPT RAN"), "{text}");
        assert!(!contains(&pdf, b"example.invalid"));
        // The links are words on the page, not links: none points anywhere in the PDF.
        assert!(
            text.contains("a link") && text.contains("a script"),
            "{text}"
        );
        for target in [
            "/URI",
            "/Link",
            "/Annots",
            "file:",
            "javascript:",
            "win.ini",
        ] {
            assert!(!contains(&pdf, target.as_bytes()), "{target}");
        }
        assert!(
            Engine::from_bytes(&pdf)
                .unwrap()
                .page_links(0, 100)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_book_a_fixed_page_file_and_a_picture_file_become_pages() {
        let (pdf, created) = one(&epub(), CreateKind::Epub);
        assert!(created.pages >= 1);
        let text = text_of(&pdf);
        assert!(
            text.contains("Chapter one") && text.contains("first words"),
            "{text}"
        );
        // What the book says of its author is not carried.
        assert!(!contains(&pdf, b"Private-Author"));

        let (pdf, created) = one(&xps(), CreateKind::Xps);
        assert_eq!(created.pages, 1);
        // 816 x 1056 units of a hundredth... of 96 to the inch: Letter in points.
        assert_eq!(size_of(&pdf, 0), (612.0, 792.0));

        let (pdf, created) = one(SVG.as_bytes(), CreateKind::Svg);
        assert_eq!(created.pages, 1);
        assert!(text_of(&pdf).contains("Hello SVG"), "{}", text_of(&pdf));
    }

    #[test]
    fn office_files_become_pages_and_say_nothing_of_their_author() {
        let (pdf, created) = one(&docx(), CreateKind::Docx);
        assert!(created.pages >= 1);
        let text = text_of(&pdf);
        assert!(text.contains("First paragraph of the letter."), "{text}");
        assert!(text.contains("隱私"), "{text}");
        for private in [
            &b"Private-Author"[..],
            b"Private title",
            b"/Author",
            b"/Title",
            b"/CreationDate",
        ] {
            assert!(
                !contains(&pdf, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        assert!(CreateKind::Docx.is_experimental() && !CreateKind::Html.is_experimental());

        let (pdf, created) = one(&xlsx(), CreateKind::Xlsx);
        assert!(created.pages >= 1);
        let text = text_of(&pdf);
        assert!(
            text.contains("Alpha cell") && text.contains("4242"),
            "{text}"
        );

        let (pdf, created) = one(&pptx(), CreateKind::Pptx);
        assert!(created.pages >= 1);
        assert!(
            text_of(&pdf).contains("Slide title text"),
            "{}",
            text_of(&pdf)
        );
    }

    #[test]
    fn a_picture_is_a_page_of_its_pixels_without_what_a_camera_wrote() {
        let jpeg = corpus("images/stamp-exif.jpg");
        assert!(contains(&jpeg, b"Canon EOS 5D Mark IV"), "the control");
        let (pdf, created) = one(&jpeg, CreateKind::Image);
        assert_eq!(created.pages, 1);
        for private in [&b"Canon"[..], b"2023:07:04", b"Exif", b"Mark IV"] {
            assert!(
                !contains(&pdf, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // 64 x 32 pixels at 96 to the inch would be a postage stamp: the page is made larger, in
        // the shape of the picture.
        assert_eq!(size_of(&pdf, 0), (144.0, 72.0));
        // The JPEG is in as the file it is: no new encoding.
        assert!(contains(&pdf, b"/DCTDecode"));

        // A PNG with text chunks and an EXIF chunk goes in as pixels.
        let png = corpus("images/stamp-metadata.png");
        let (pdf, _) = one(&png, CreateKind::Image);
        for private in [
            &b"Jane Q. Private-Author"[..],
            b"Canon EOS 5D Mark IV",
            b"Software",
            b"Author",
        ] {
            assert!(
                !contains(&pdf, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        assert!(contains(&pdf, b"/FlateDecode") && !contains(&pdf, b"/DCTDecode"));
    }

    /// The EXIF of a picture says it was taken on its side: `orientation`, of EXIF's eight.
    fn with_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
        let mut exif = b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
        exif.extend_from_slice(&[0, orientation, 0, 0, 0, 0, 0, 0]);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn a_picture_taken_on_its_side_is_shown_the_right_way_up() {
        let clean = jpeg::clean(&corpus("images/stamp-exif.jpg")).unwrap().bytes;
        for (orientation, size) in [
            (1, (144.0, 72.0)),
            (3, (144.0, 72.0)),
            (6, (72.0, 144.0)),
            (8, (72.0, 144.0)),
        ] {
            let (pdf, _) = one(&with_orientation(&clean, orientation), CreateKind::Image);
            assert_eq!(size_of(&pdf, 0), size, "orientation {orientation}");
            // And the camera's own mark is not in the file in any case.
            assert!(!contains(&pdf, b"Exif"));
        }
    }

    #[test]
    fn pictures_go_together_one_page_each_in_the_order_given() {
        let jpeg = corpus("images/stamp-exif.jpg");
        let png = corpus("images/stamp-metadata.png");
        let (pdf, created) = made(&[
            (&png, CreateKind::Image),
            (&jpeg, CreateKind::Image),
            (&png, CreateKind::Image),
        ])
        .unwrap();
        assert_eq!(created.pages, 3);
        assert_eq!(Engine::from_bytes(&pdf).unwrap().page_count().unwrap(), 3);
        // Nothing else goes with them, and they go with nothing else.
        for (first, second) in [
            (
                (&png[..], CreateKind::Image),
                (HTML.as_bytes(), CreateKind::Html),
            ),
            (
                (HTML.as_bytes(), CreateKind::Html),
                (&png[..], CreateKind::Image),
            ),
            (
                (HTML.as_bytes(), CreateKind::Html),
                (HTML.as_bytes(), CreateKind::Html),
            ),
            (
                (SVG.as_bytes(), CreateKind::Svg),
                (SVG.as_bytes(), CreateKind::Svg),
            ),
        ] {
            assert!(matches!(
                made(&[first, second]),
                Err(EngineError::InvalidEdit(_))
            ));
        }
        // Nothing to make a PDF of.
        assert!(matches!(
            PdfBuilder::new().finish(&mut Vec::new()),
            Err(EngineError::InvalidEdit(_))
        ));
    }

    /// A picture of `width` by `height` pixels as a BMP: no metadata to speak of.
    fn bmp(width: u32, height: u32) -> Vec<u8> {
        let row = (width * 3).div_ceil(4) * 4;
        let size = 54 + row * height;
        let mut bytes = b"BM".to_vec();
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 24, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&(row * height).to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        bytes.resize(size as usize, 0x7F);
        bytes
    }

    #[test]
    fn the_page_of_a_picture_is_its_own_size_within_what_a_page_may_be() {
        // 800 x 600 at 96 to the inch: 600 x 450 points.
        assert_eq!(
            size_of(&one(&bmp(800, 600), CreateKind::Image).0, 0),
            (600.0, 450.0)
        );
        // A photograph of 3000 x 2000 would be 2250 points wide: no wider than A3.
        let (pdf, _) = one(&bmp(3000, 2000), CreateKind::Image);
        let (width, height) = size_of(&pdf, 0);
        assert_eq!(width, 1190.0);
        assert!((height - 1190.0 * 2.0 / 3.0).abs() < 0.5, "{height}");
        // An icon is no postage stamp.
        assert_eq!(
            size_of(&one(&bmp(16, 16), CreateKind::Image).0, 0),
            (144.0, 144.0)
        );
        // A GIF too.
        let gif = b"GIF89a\x01\0\x01\0\x80\0\0\xff\xff\xff\0\0\0\x21\xf9\x04\x01\0\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0\x3b";
        assert_eq!(made(&[(gif, CreateKind::Image)]).unwrap().1.pages, 1);
    }

    #[test]
    fn what_cannot_be_read_is_an_error_not_a_half_made_pdf() {
        // A picture that says it is far larger than it may be is refused before any pixel is read.
        let huge = corpus("images/stamp-huge-dimensions.png");
        assert!(matches!(
            made(&[(&huge, CreateKind::Image)]),
            Err(EngineError::PictureTooLarge | EngineError::InvalidSource(_))
        ));
        let mut cut = corpus("images/stamp-metadata.png");
        cut.truncate(60);
        for (bytes, kind) in [
            (cut.as_slice(), CreateKind::Image),
            (b"not a picture at all".as_slice(), CreateKind::Image),
            (&[][..], CreateKind::Image),
            (b"not a zip".as_slice(), CreateKind::Docx),
            (b"not a zip".as_slice(), CreateKind::Xlsx),
            (b"not a zip".as_slice(), CreateKind::Pptx),
            (b"not a zip".as_slice(), CreateKind::Epub),
            (b"not a zip".as_slice(), CreateKind::Xps),
            (&[][..], CreateKind::Docx),
            (b"<svg".as_slice(), CreateKind::Svg),
        ] {
            let outcome = made(&[(bytes, kind)]);
            assert!(
                matches!(
                    outcome,
                    Err(EngineError::InvalidSource(_) | EngineError::MuPdf(_))
                ),
                "{kind:?}: {outcome:?}"
            );
        }
        // A text with nothing in it has no page: nothing to make a PDF of.
        let outcome = made(&[(b"", CreateKind::Text)]);
        assert!(
            matches!(outcome, Err(EngineError::InvalidSource(_))),
            "{outcome:?}"
        );
    }

    /// An ASCII PDF with the things a PDF can be made to do on its own.
    fn active_pdf() -> Vec<u8> {
        let content = "BT /F1 12 Tf 20 100 Td (Hello PDF) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (alert1) >> \
/AA << /WC << /S /JavaScript /JS (x) >> >> >>"
                .to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [5 0 R] \
/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> \
/AA << /O << /S /Launch /F (calc.exe) >> >> >>"
                .to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            "<< /Type /Annot /Subtype /Link /Rect [0 0 100 100] /A << /S /Launch /F (calc.exe) >> >>"
                .to_owned(),
        ];
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

    #[test]
    fn a_pdf_given_as_another_kind_brings_none_of_what_it_does_on_its_own() {
        // MuPDF reads what a file is from its content as well as from its name: a PDF given as
        // a text, a book or a Word file is read as the PDF it is, and only its pages are copied.
        let pdf = active_pdf();
        assert!(std::str::from_utf8(&pdf).is_ok());
        for kind in KINDS_FOR_TEST {
            let Ok((out, created)) = made(&[(&pdf, kind)]) else {
                // Not a picture, then.
                assert_eq!(kind, CreateKind::Image);
                continue;
            };
            assert_eq!(created.pages, 1, "{kind:?}");
            assert!(text_of(&out).contains("Hello PDF"), "{kind:?}");
            for active in [
                "/JavaScript",
                "/JS",
                "/OpenAction",
                "/Launch",
                "/AA",
                "/URI",
                "/Annots",
            ] {
                assert!(!contains(&out, active.as_bytes()), "{kind:?}: {active}");
            }
        }
        // One that needs a password is not opened.
        let encrypted = corpus("benign/encrypted-aes256.pdf");
        let outcome = made(&[(&encrypted, CreateKind::Docx)]);
        assert!(
            matches!(outcome, Err(EngineError::InvalidSource(_))),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_pixels_of_all_the_pictures_together_are_bounded() {
        let picture = bmp(100, 100);
        let one_pixmap = Image::from_bytes(&picture)
            .and_then(|image| image.to_pixmap())
            .expect("a picture")
            .samples()
            .len() as u64;
        let mut builder = PdfBuilder::new();
        builder.raw_left = one_pixmap * 5 / 2;
        builder.add(&picture, CreateKind::Image).unwrap();
        builder.add(&picture, CreateKind::Image).unwrap();
        let outcome = builder.add(&picture, CreateKind::Image);
        assert!(
            matches!(outcome, Err(EngineError::PictureTooLarge)),
            "{outcome:?}"
        );
        // The two that fit are still there, and a JPEG takes none of the room.
        assert_eq!(builder.pages(), 2);
        builder
            .add(&corpus("images/stamp-exif.jpg"), CreateKind::Image)
            .unwrap();
        assert_eq!(builder.pages(), 3);
    }
}
