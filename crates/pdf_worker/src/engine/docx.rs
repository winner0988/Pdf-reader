//! A Word document (.docx) of some pages (B3-02, docs/architecture/docx-export.md): their text, in
//! the order MuPDF reads it, as paragraphs whose runs keep the font's name, size, weight, slant,
//! colour, underline and strike-through, and their pictures, one after another where they were
//! met. Nothing of the page's layout is kept (columns, tables, text boxes and the place of a
//! picture on the page): a reader who needs that has the PDF.
//!
//! MuPDF has a writer of its own for this, but the binding gives it a file name to write to and
//! the worker has none by design (ADR 0008): it writes to the handle it is given. So the document
//! is made here, from the structured text MuPDF extracts for every other use, and nothing of it
//! comes from outside: no template, no metadata, no author and no date.

use std::fmt::Write as _;
use std::io::Write;

use mupdf::text_page::TextBlockType;
use mupdf::{ImageFormat, TextBlock, TextChar, TextCharFlags, TextPageFlags};

use super::{EngineError, PdfDocument};
use crate::zip::Archive;

/// The most the document may take once written, in bytes: what the worker holds in memory for it
/// is a little more than twice this.
pub const MAX_DOCX_BYTES: usize = 192 * 1024 * 1024;

/// The most pictures a document keeps, and the most bytes of them: the rest are left out.
const MAX_PICTURES: usize = 2_000;
const MAX_PICTURE_BYTES: usize = 64 * 1024 * 1024;
/// A picture of more pixels than this is left out: its PNG would be too large to be of use.
const MAX_PICTURE_PIXELS: u64 = 25_000_000;

/// How wide, at most, a margin is: an inch, as Word's own.
const MAX_MARGIN_PT: f32 = 72.0;

/// The margin of a page of `width` by `height` points: a tenth of its short side, between a quarter
/// of an inch and an inch, so that a small page keeps room for its text.
fn margin(width: f32, height: f32) -> f32 {
    (width.min(height) * 0.1).clamp(18.0, MAX_MARGIN_PT)
}
/// The largest size of a letter Word has: 819 points, in half points.
const MAX_HALF_POINTS: u32 = 1_638;

const XML_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const NAMESPACES: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
</Types>";

const ROOT_RELATIONSHIPS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

/// What a paragraph and a letter are when nothing says otherwise: Calibri for Latin letters and
/// Microsoft JhengHei for Chinese ones, 11 points, a little room below each paragraph.
const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults>\
<w:rPrDefault><w:rPr>\
<w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Microsoft JhengHei\" w:cs=\"Calibri\"/>\
<w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>\
<w:lang w:val=\"en-US\" w:eastAsia=\"zh-TW\" w:bidi=\"ar-SA\"/>\
</w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
</w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
</w:styles>";

/// The family of fonts Word is told to draw a run in: one of four that every Windows computer
/// has, by what MuPDF says the font is. The font's name is not read: a PDF's fonts are mostly
/// subsets named for the program that made them, which the reader's computer does not have, and
/// the binding's accessor for the name panics on one that is not UTF-8 (found by fuzzing).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Sans,
    Serif,
    Mono,
    /// The private-use letters of a symbol font (a bullet, an arrow) mean what they mean in Symbol.
    Symbol,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Family::Sans => "Arial",
            Family::Serif => "Times New Roman",
            Family::Mono => "Courier New",
            Family::Symbol => "Symbol",
        }
    }
}

/// How a run of letters looks.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    family: Family,
    half_points: u32,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    /// 0xRRGGBB.
    color: u32,
}

impl Style {
    /// The look of the letter `c`, which is `ch`.
    fn of(ch: &TextChar, c: char) -> Self {
        let flags = ch.flags();
        let (bold, italic, family) = match ch.font() {
            Some(font) => (
                font.is_bold(),
                font.is_italic(),
                if font.is_monospaced() {
                    Family::Mono
                } else if font.is_serif() {
                    Family::Serif
                } else {
                    Family::Sans
                },
            ),
            None => (false, false, Family::Sans),
        };
        let half_points = (ch.size() * 2.0).round();
        let half_points = if half_points.is_finite() {
            (half_points as i64).clamp(2, i64::from(MAX_HALF_POINTS)) as u32
        } else {
            22
        };
        Self {
            family: if ('\u{F020}'..='\u{F0FF}').contains(&c) {
                Family::Symbol
            } else {
                family
            },
            half_points,
            bold: bold || flags.contains(TextCharFlags::BOLD),
            italic,
            underline: flags.contains(TextCharFlags::UNDERLINE),
            strike: flags.contains(TextCharFlags::STRIKEOUT),
            color: ch.argb() & 0x00FF_FFFF,
        }
    }
}

/// Whether `c` is written into the document: the letters of a page, but not what is invisible,
/// or controls, or what XML cannot hold, or what turns the order of what follows around.
fn is_kept(c: char) -> bool {
    !(c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFFE}'
                | '\u{FFFF}'
        ))
}

/// Whether `c` is written without a space between it and the next one: Chinese, Japanese and
/// Korean text has none between words.
fn is_wide(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}')
}

/// Whether two lines of a paragraph, the one ending in `last` and the next starting with `next`,
/// are joined with a space: not where there is one already, nor between Chinese letters, nor
/// after the hyphen that breaks a word across two lines (the hyphen stays: a compound word has
/// one, and a reader sees through the other).
pub(super) fn space_between(last: char, next: char) -> bool {
    !(last.is_whitespace()
        || next.is_whitespace()
        || is_wide(last)
        || is_wide(next)
        || (last == '-' && next.is_lowercase()))
}

fn escape(text: &str, into: &mut String) {
    for c in text.chars() {
        match c {
            '&' => into.push_str("&amp;"),
            '<' => into.push_str("&lt;"),
            '>' => into.push_str("&gt;"),
            '"' => into.push_str("&quot;"),
            c => into.push(c),
        }
    }
}

/// A paragraph of text: the XML of its runs, or nothing if it has no letter that shows.
fn text_paragraph(block: &TextBlock) -> Option<String> {
    let mut runs: Vec<(Style, String)> = Vec::new();
    let mut last: Option<char> = None;
    for (number, line) in block.lines().enumerate() {
        let mut first_of_line = true;
        for ch in line.chars() {
            let Some(c) = ch.char().filter(|c| is_kept(*c)) else {
                continue;
            };
            let style = Style::of(&ch, c);
            // Another line of the paragraph: a space, unless there is one already or Chinese.
            if first_of_line
                && number > 0
                && last.is_some_and(|last| space_between(last, c))
                && let Some((_, text)) = runs.last_mut()
            {
                text.push(' ');
            }
            first_of_line = false;
            match runs.last_mut() {
                Some((have, text)) if *have == style => text.push(c),
                _ => runs.push((style, c.to_string())),
            }
            last = Some(c);
        }
    }
    if runs.iter().all(|(_, text)| text.trim().is_empty()) {
        return None;
    }
    // No spaces at either end of the paragraph.
    if let Some((_, text)) = runs.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some((_, text)) = runs.last_mut() {
        *text = text.trim_end().to_owned();
    }
    let mut xml = String::new();
    for (style, text) in runs.iter().filter(|(_, text)| !text.is_empty()) {
        xml.push_str("<w:r><w:rPr>");
        // In the order the schema asks for: fonts, weight, slant, strike, colour, size, underline.
        let family = style.family.name();
        let _ = write!(
            xml,
            "<w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\" w:cs=\"{family}\"/>"
        );
        if style.bold {
            xml.push_str("<w:b/><w:bCs/>");
        }
        if style.italic {
            xml.push_str("<w:i/><w:iCs/>");
        }
        if style.strike {
            xml.push_str("<w:strike/>");
        }
        if style.color != 0 {
            let _ = write!(xml, "<w:color w:val=\"{:06X}\"/>", style.color);
        }
        let _ = write!(
            xml,
            "<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>",
            style.half_points
        );
        if style.underline {
            xml.push_str("<w:u w:val=\"single\"/>");
        }
        xml.push_str("</w:rPr><w:t xml:space=\"preserve\">");
        escape(text, &mut xml);
        xml.push_str("</w:t></w:r>");
    }
    Some(xml)
}

/// The pictures kept so far, as PNG files.
#[derive(Default)]
struct Pictures {
    files: Vec<Vec<u8>>,
    bytes: usize,
}

/// A paragraph of one picture, `width` by `height` points at most as wide as `limit`: its XML, or
/// nothing if it is left out (too small, too large, too many, or not a picture MuPDF gives pixels).
fn picture_paragraph(block: &TextBlock, limit: f32, pictures: &mut Pictures) -> Option<String> {
    let bounds = block.bounds();
    let (mut width, mut height) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    if !(width.is_finite() && height.is_finite()) || width < 2.0 || height < 2.0 {
        return None;
    }
    if pictures.files.len() >= MAX_PICTURES {
        return None;
    }
    let image = block.image()?;
    if u64::from(image.width()) * u64::from(image.height()) > MAX_PICTURE_PIXELS {
        return None;
    }
    let pixmap = image.to_pixmap().ok()?;
    // PNG takes gray and RGB, with or without transparency.
    let color = pixmap.color_space()?;
    if !(color.is_gray() || color.is_rgb()) {
        return None;
    }
    let mut png = Vec::new();
    pixmap.write_to(&mut png, ImageFormat::PNG).ok()?;
    if pictures.bytes.saturating_add(png.len()) > MAX_PICTURE_BYTES {
        return None;
    }
    if width > limit {
        height *= limit / width;
        width = limit;
    }
    pictures.bytes += png.len();
    pictures.files.push(png);
    let number = pictures.files.len();
    let (cx, cy) = (emu(width), emu(height));
    Some(format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{number}\" name=\"Picture {number}\"/>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{number}\" name=\"image{number}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rId{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
        id = number + 1
    ))
}

/// English Metric Units: what Word measures a picture in; 12,700 to the point.
fn emu(points: f32) -> i64 {
    (f64::from(points) * 12_700.0).round().max(1.0) as i64
}

/// Twentieths of a point: what Word measures a page in.
fn twips(points: f32) -> i64 {
    (f64::from(points) * 20.0).round().clamp(240.0, 31_680.0) as i64
}

/// The properties of the section a page is, in the size the page has.
fn section(width: f32, height: f32) -> String {
    let (w, h, m) = (twips(width), twips(height), twips(margin(width, height)));
    let orientation = if width > height {
        " w:orient=\"landscape\""
    } else {
        ""
    };
    format!(
        "<w:sectPr><w:pgSz w:w=\"{w}\" w:h=\"{h}\"{orientation}/>\
<w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" w:header=\"{m}\" w:footer=\"{m}\" w:gutter=\"0\"/></w:sectPr>"
    )
}

/// A paragraph with `content`, which ends the section `section` if it is given.
fn paragraph(content: &str, section: Option<&str>) -> String {
    match section {
        Some(section) => format!("<w:p><w:pPr>{section}</w:pPr>{content}</w:p>"),
        None => format!("<w:p>{content}</w:p>"),
    }
}

impl PdfDocument {
    /// Writes to `out` a Word document of `pages` (0-based, no repeats), in the order given: each
    /// page is a section of its own, the size the page has. Returns the bytes written. The worker
    /// never writes a file but the one it is handed.
    pub fn docx(&self, pages: &[u32], out: &mut impl Write) -> Result<u64, EngineError> {
        if pages.is_empty() {
            return Err(EngineError::InvalidEdit("no pages to export"));
        }
        let count = self.page_count()?;
        let mut seen = std::collections::HashSet::new();
        for &page in pages {
            if page >= count {
                return Err(EngineError::PageOutOfRange(page));
            }
            if !seen.insert(page) {
                return Err(EngineError::InvalidEdit("a page appears twice"));
            }
        }

        let mut body = String::new();
        let mut pictures = Pictures::default();
        for (position, &index) in pages.iter().enumerate() {
            let page = self.load_page(index)?;
            let bounds = page.bounds()?;
            let (width, height) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
            let (width, height) =
                if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                    (width, height)
                } else {
                    (612.0, 792.0)
                };
            let limit = (width - 2.0 * margin(width, height)).max(36.0);
            let text = page.to_text_page(TextPageFlags::PRESERVE_IMAGES)?;
            let mut contents: Vec<String> = Vec::new();
            let mut pending = 0usize;
            for block in text.blocks() {
                let found = match block.r#type() {
                    TextBlockType::Text => text_paragraph(&block),
                    TextBlockType::Image => picture_paragraph(&block, limit, &mut pictures),
                    _ => None,
                };
                if let Some(found) = found {
                    pending += found.len();
                    contents.push(found);
                }
                if body.len() + pending > MAX_DOCX_BYTES {
                    return Err(EngineError::OutputTooLarge);
                }
            }
            // The last paragraph of a page ends its section, so that the next page starts on a new
            // page and may be another size; the last page's section is the body's own.
            let last_page = position + 1 == pages.len();
            let ends = (!last_page).then(|| section(width, height));
            match contents.pop() {
                Some(last) => {
                    for content in &contents {
                        body.push_str(&paragraph(content, None));
                    }
                    body.push_str(&paragraph(&last, ends.as_deref()));
                }
                // A page with nothing to say is an empty paragraph, which a section needs.
                None => body.push_str(&paragraph("", ends.as_deref())),
            }
            if last_page {
                body.push_str(&section(width, height));
            }
        }

        let mut document = String::with_capacity(body.len() + 1024);
        document.push_str(XML_HEADER);
        let _ = write!(document, "<w:document {NAMESPACES}><w:body>");
        document.push_str(&body);
        document.push_str("</w:body></w:document>");
        drop(body);

        let mut relationships = String::from(XML_HEADER);
        relationships.push_str(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
        );
        for number in 1..=pictures.files.len() {
            let _ = write!(
                relationships,
                "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{number}.png\"/>",
                number + 1
            );
        }
        relationships.push_str("</Relationships>");

        let mut archive = Archive::new(MAX_DOCX_BYTES);
        let too_large = |_| EngineError::OutputTooLarge;
        archive
            .add("[Content_Types].xml", CONTENT_TYPES.as_bytes())
            .map_err(too_large)?;
        archive
            .add("_rels/.rels", ROOT_RELATIONSHIPS.as_bytes())
            .map_err(too_large)?;
        archive
            .add("word/document.xml", document.as_bytes())
            .map_err(too_large)?;
        drop(document);
        archive
            .add("word/styles.xml", STYLES.as_bytes())
            .map_err(too_large)?;
        archive
            .add("word/_rels/document.xml.rels", relationships.as_bytes())
            .map_err(too_large)?;
        for (index, png) in pictures.files.iter().enumerate() {
            archive
                .add(&format!("word/media/image{}.png", index + 1), png)
                .map_err(too_large)?;
        }
        drop(pictures);
        let bytes = archive.finish().map_err(too_large)?;
        out.write_all(&bytes).map_err(EngineError::Write)?;
        Ok(bytes.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument as MuPdfDocument};
    use mupdf::{Colorspace, Pixmap, Rect};

    use super::*;
    use crate::zip;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// A PDF whose pages are `(width, height, content stream)`, with the four fonts F1 (Helvetica),
    /// F2 (Helvetica-Bold), F3 (Helvetica-Oblique) and F4 (Courier) to use in them.
    fn pdf_of(pages: &[(u32, u32, &str)]) -> Vec<u8> {
        let kids: Vec<String> = (0..pages.len())
            .map(|k| format!("{} 0 R", 7 + 2 * k))
            .collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                pages.len()
            ),
        ];
        for font in [
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Courier",
        ] {
            objects.push(format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{font} >>"
            ));
        }
        for (k, (width, height, content)) in pages.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents {} 0 R \
/Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R >> >> >>",
                8 + 2 * k
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ));
        }
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (number, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", number + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn docx_of(pdf: &[u8], pages: &[u32]) -> Vec<u8> {
        let doc = PdfDocument::from_bytes(pdf).expect("open");
        let mut out = Vec::new();
        let written = doc.docx(pages, &mut out).expect("exported");
        assert_eq!(written, out.len() as u64);
        out
    }

    fn part(docx: &[u8], name: &str) -> String {
        let files = zip::read(docx);
        let (_, data) = files
            .iter()
            .find(|(file, _)| file == name)
            .unwrap_or_else(|| {
                panic!(
                    "{name} is not in {:?}",
                    files.iter().map(|file| &file.0).collect::<Vec<_>>()
                )
            });
        String::from_utf8(data.clone()).expect("UTF-8")
    }

    /// Checks that `xml` is well formed, as far as what is written here can go wrong: tags nest and
    /// close, attributes are quoted, and `<` and `&` mean what they should.
    fn assert_well_formed(xml: &str) {
        let bytes = xml.as_bytes();
        let mut stack: Vec<String> = Vec::new();
        let mut at = 0;
        let mut roots = 0;
        while at < bytes.len() {
            if xml[at..].starts_with("<?") {
                at += xml[at..].find("?>").expect("the declaration ends") + 2;
            } else if xml[at..].starts_with("</") {
                let end = at + xml[at..].find('>').expect("the closing tag ends");
                let name = &xml[at + 2..end];
                assert_eq!(stack.pop().as_deref(), Some(name), "at {at}");
                at = end + 1;
            } else if bytes[at] == b'<' {
                let mut end = at + 1;
                while !matches!(bytes[end], b' ' | b'/' | b'>') {
                    end += 1;
                }
                let name = xml[at + 1..end].to_owned();
                assert!(!name.is_empty() && !name.contains('<'), "at {at}");
                at = end;
                loop {
                    while bytes[at] == b' ' {
                        at += 1;
                    }
                    match bytes[at] {
                        b'>' => {
                            if stack.is_empty() {
                                roots += 1;
                            }
                            stack.push(name);
                            at += 1;
                            break;
                        }
                        b'/' => {
                            assert_eq!(bytes[at + 1], b'>', "at {at}");
                            if stack.is_empty() {
                                roots += 1;
                            }
                            at += 2;
                            break;
                        }
                        _ => {
                            let equals =
                                at + xml[at..].find('=').expect("an attribute has a value");
                            assert!(!xml[at..equals].contains(['<', '>', '"', ' ']), "at {at}");
                            assert_eq!(bytes[equals + 1], b'"', "at {at}");
                            let close =
                                equals + 2 + xml[equals + 2..].find('"').expect("the value ends");
                            let value = &xml[equals + 2..close];
                            assert!(!value.contains('<'), "{value}");
                            check_entities(value);
                            at = close + 1;
                        }
                    }
                }
            } else {
                let end = xml[at..].find('<').map_or(bytes.len(), |next| at + next);
                check_entities(&xml[at..end]);
                at = end;
            }
        }
        assert!(stack.is_empty(), "unclosed: {stack:?}");
        assert_eq!(roots, 1, "one root");
    }

    /// Every `&` starts one of the five entities.
    fn check_entities(text: &str) {
        let mut rest = text;
        while let Some(at) = rest.find('&') {
            let after = &rest[at..];
            assert!(
                ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"]
                    .iter()
                    .any(|entity| after.starts_with(entity)),
                "a stray & in {text:?}"
            );
            rest = &after[1..];
        }
    }

    /// The runs of a document: what their properties say, and their text.
    fn runs(xml: &str) -> Vec<(String, String)> {
        xml.split("<w:r>")
            .skip(1)
            .filter(|run| run.contains("<w:t "))
            .map(|run| {
                let properties = run.split("</w:rPr>").next().unwrap().to_owned();
                let text = run
                    .split("preserve\">")
                    .nth(1)
                    .unwrap()
                    .split("</w:t>")
                    .next()
                    .unwrap();
                (properties, text.to_owned())
            })
            .collect()
    }

    #[test]
    fn the_words_of_a_page_are_in_the_document_in_both_scripts() {
        let docx = docx_of(&corpus("benign/mixed-text-zh-en.pdf"), &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        assert!(xml.contains("Privacy-first PDF Reader"), "{xml}");
        assert!(xml.contains("隱私優先的"), "{xml}");
        assert!(xml.contains("閱讀器"), "{xml}");
    }

    #[test]
    fn runs_keep_the_font_the_weight_the_slant_the_colour_and_the_size_of_their_letters() {
        let content = "BT /F1 12 Tf 72 720 Td (Plain and ) Tj /F2 12 Tf (bold) Tj /F3 12 Tf ( then italic) Tj ET\n\
BT /F1 24 Tf 1 0 0 rg 72 640 Td (Big red heading) Tj ET\n\
BT /F4 10 Tf 0 0 1 rg 72 560 Td (code) Tj ET";
        let docx = docx_of(&pdf_of(&[(612, 792, content)]), &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        let runs = runs(&xml);
        let find = |text: &str| {
            runs.iter()
                .find(|(_, run)| run.contains(text))
                .unwrap_or_else(|| panic!("{text} in {runs:?}"))
                .0
                .clone()
        };
        let plain = find("Plain and");
        assert!(
            !plain.contains("<w:b/>") && !plain.contains("<w:i/>"),
            "{plain}"
        );
        assert!(plain.contains("w:ascii=\"Arial\""), "{plain}");
        assert!(
            plain.contains("<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>"),
            "{plain}"
        );
        assert!(find("bold").contains("<w:b/><w:bCs/>"));
        assert!(find("then italic").contains("<w:i/><w:iCs/>"));
        let heading = find("Big red heading");
        assert!(heading.contains("<w:color w:val=\"FF0000\"/>"), "{heading}");
        assert!(heading.contains("<w:sz w:val=\"48\"/>"), "{heading}");
        let code = find("code");
        assert!(code.contains("w:ascii=\"Courier New\""), "{code}");
        assert!(code.contains("<w:color w:val=\"0000FF\"/>"), "{code}");
        // Black is Word's own: no colour is written for it.
        assert!(!plain.contains("<w:color"), "{plain}");
    }

    #[test]
    fn the_lines_of_a_paragraph_are_joined_by_a_space_but_not_where_the_text_is_chinese() {
        let content =
            "BT /F1 12 Tf 14 TL 72 720 Td (the first line of it) Tj T* (and the second) Tj ET";
        let docx = docx_of(&pdf_of(&[(612, 792, content)]), &[0]);
        let xml = part(&docx, "word/document.xml");
        assert!(xml.contains("the first line of it and the second"), "{xml}");
        // MuPDF decides what a paragraph is; whatever it decides, the words are in order and apart.
        let texts: Vec<String> = runs(&xml).into_iter().map(|(_, text)| text).collect();
        assert_eq!(
            texts.join("").replace("  ", " ").trim(),
            "the first line of it and the second"
        );
    }

    #[test]
    fn markup_in_the_text_is_text_in_the_document() {
        let content = "BT /F1 12 Tf 72 720 Td (a <b> & \"c\" > 'd') Tj ET";
        let docx = docx_of(&pdf_of(&[(612, 792, content)]), &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        // The page's own encoding makes the apostrophe a right single quote.
        assert!(
            xml.contains("a &lt;b&gt; &amp; &quot;c&quot; &gt; \u{2019}d\u{2019}"),
            "{xml}"
        );
    }

    #[test]
    fn each_page_is_a_section_of_its_own_size_in_the_order_asked() {
        let pages = [
            (612, 792, "BT /F1 12 Tf 72 720 Td (first page) Tj ET"),
            (792, 612, "BT /F1 12 Tf 72 500 Td (second page) Tj ET"),
            (300, 400, "BT /F1 12 Tf 20 360 Td (third page) Tj ET"),
        ];
        let pdf = pdf_of(&pages);
        let docx = docx_of(&pdf, &[2, 0, 1]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        let at = |text: &str| xml.find(text).unwrap_or_else(|| panic!("{text} in {xml}"));
        assert!(at("third page") < at("first page") && at("first page") < at("second page"));
        // Three sections: two end with their page's last paragraph, the last is the body's own.
        assert_eq!(xml.matches("<w:sectPr>").count(), 3, "{xml}");
        assert_eq!(xml.matches("<w:pPr><w:sectPr>").count(), 2, "{xml}");
        assert!(xml.ends_with("</w:sectPr></w:body></w:document>"));
        // The sizes, in twentieths of a point, in the order the pages come: 300 x 400 (with a margin
        // of a tenth of the short side), 612 x 792, and the one on its side.
        let sizes: Vec<&str> = xml
            .split("<w:pgSz ")
            .skip(1)
            .map(|s| s.split("/>").next().unwrap())
            .collect();
        assert_eq!(
            sizes,
            [
                "w:w=\"6000\" w:h=\"8000\"",
                "w:w=\"12240\" w:h=\"15840\"",
                "w:w=\"15840\" w:h=\"12240\" w:orient=\"landscape\""
            ]
        );
        assert!(xml.contains("w:left=\"600\""), "{xml}");
        // Only the pages asked for are in it.
        let one = part(&docx_of(&pdf, &[1]), "word/document.xml");
        assert!(
            one.contains("second page")
                && !one.contains("first page")
                && !one.contains("third page")
        );
        assert_eq!(one.matches("<w:sectPr>").count(), 1);
    }

    #[test]
    fn a_page_with_nothing_on_it_is_an_empty_paragraph_that_ends_its_section() {
        let pages = [
            (612, 792, ""),
            (612, 792, "BT /F1 12 Tf 72 720 Td (after) Tj ET"),
        ];
        let docx = docx_of(&pdf_of(&pages), &[0, 1]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        assert!(xml.contains("<w:p><w:pPr><w:sectPr>"), "{xml}");
        assert!(xml.contains("after"));
    }

    #[test]
    fn what_is_not_a_request_for_pages_is_refused() {
        let pdf = pdf_of(&[(612, 792, ""), (612, 792, "")]);
        let doc = PdfDocument::from_bytes(&pdf).unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            doc.docx(&[], &mut out),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(matches!(
            doc.docx(&[0, 5], &mut out),
            Err(EngineError::PageOutOfRange(5))
        ));
        assert!(matches!(
            doc.docx(&[1, 1], &mut out),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(out.is_empty(), "nothing was written");
    }

    #[test]
    fn the_package_has_the_parts_of_a_document_and_nothing_of_who_made_it_or_when() {
        let docx = docx_of(&corpus("benign/mixed-text-zh-en.pdf"), &[0]);
        let files = zip::read(&docx);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "word/document.xml",
                "word/styles.xml",
                "word/_rels/document.xml.rels"
            ]
        );
        for (name, data) in &files {
            assert_well_formed(std::str::from_utf8(data).unwrap());
            let text = String::from_utf8_lossy(data).to_lowercase();
            for word in [
                "author",
                "creator",
                "created",
                "modified",
                "lastmodifiedby",
                "dcterms",
            ] {
                assert!(!text.contains(word), "{word} in {name}");
            }
        }
        // Every part that is used is said to be there, and by what name it is known.
        let types = part(&docx, "[Content_Types].xml");
        assert!(types.contains("/word/document.xml") && types.contains("/word/styles.xml"));
        let rels = part(&docx, "word/_rels/document.xml.rels");
        assert!(rels.contains("Target=\"styles.xml\""));
    }

    #[test]
    fn a_picture_is_a_file_of_the_package_and_drawn_where_it_was_met() {
        let mut doc = MuPdfDocument::new();
        let mut page = doc.new_page((612.0, 792.0)).unwrap();
        let mut pixmap = Pixmap::new_with_w_h(&Colorspace::device_rgb(), 40, 30, false).unwrap();
        for (at, sample) in pixmap.samples_mut().iter_mut().enumerate() {
            *sample = (at % 251) as u8;
        }
        page.insert_image(
            &mut doc,
            Rect {
                x0: 100.0,
                y0: 100.0,
                x1: 200.0,
                y1: 175.0,
            },
            PageImageSource::Pixmap(&pixmap),
            InsertImageOptions::default(),
        )
        .unwrap();
        drop(page);
        let mut bytes = Vec::new();
        doc.write_to(&mut bytes).unwrap();

        let docx = docx_of(&bytes, &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        // 100 by 75 points: 12,700 units of Word's own to each.
        assert!(
            xml.contains("<wp:extent cx=\"1270000\" cy=\"952500\"/>"),
            "{xml}"
        );
        assert!(xml.contains("<a:blip r:embed=\"rId2\"/>"), "{xml}");
        let rels = part(&docx, "word/_rels/document.xml.rels");
        assert!(
            rels.contains("Id=\"rId2\"") && rels.contains("Target=\"media/image1.png\""),
            "{rels}"
        );
        let files = zip::read(&docx);
        let (_, png) = files
            .iter()
            .find(|(name, _)| name == "word/media/image1.png")
            .expect("the picture");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn a_picture_wider_than_the_text_is_made_to_fit_it() {
        let mut doc = MuPdfDocument::new();
        let mut page = doc.new_page((612.0, 792.0)).unwrap();
        let pixmap = Pixmap::new_with_w_h(&Colorspace::device_gray(), 10, 10, false).unwrap();
        page.insert_image(
            &mut doc,
            Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 612.0,
                y1: 306.0,
            },
            PageImageSource::Pixmap(&pixmap),
            InsertImageOptions::default(),
        )
        .unwrap();
        drop(page);
        let mut bytes = Vec::new();
        doc.write_to(&mut bytes).unwrap();
        let xml = part(&docx_of(&bytes, &[0]), "word/document.xml");
        // The text is 612 less two margins of 61.2 points wide; the shape stays.
        let width = (612.0f64 - 2.0 * 61.2) * 12_700.0;
        assert!(
            xml.contains(&format!(
                "cx=\"{}\" cy=\"{}\"",
                width.round(),
                (width / 2.0).round()
            )),
            "{xml}"
        );
    }

    #[test]
    fn a_font_with_a_name_that_is_not_text_is_no_trouble() {
        // The binding's way to ask a font its name panics on one that is not UTF-8: found by the
        // fuzzer in the first run. The name is not asked for.
        let content = "BT /F2 12 Tf 72 720 Td (Named in bytes) Tj ET";
        let mut pdf = pdf_of(&[(612, 792, content)]);
        let name = b"Helvetica-Bold";
        let at = pdf
            .windows(name.len())
            .position(|window| window == name)
            .expect("the font");
        pdf[at..at + name.len()].copy_from_slice(b"Bad#FF#FEName0");
        let docx = docx_of(&pdf, &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        assert!(xml.contains("Named in bytes"), "{xml}");
    }

    #[test]
    fn the_symbols_of_a_symbol_font_are_in_the_font_that_has_them() {
        assert_eq!(Family::Symbol.name(), "Symbol");
        assert_eq!(Family::Sans.name(), "Arial");
        assert_eq!(Family::Serif.name(), "Times New Roman");
        assert_eq!(Family::Mono.name(), "Courier New");
    }

    #[test]
    fn only_what_can_be_seen_is_written() {
        for c in ['a', 'é', '中', '漢', ' ', '•', '\u{F0B7}'] {
            assert!(is_kept(c), "{c:?}");
        }
        for c in [
            '\0', '\u{1}', '\n', '\u{7F}', '\u{85}', '\u{AD}', '\u{200B}', '\u{200F}', '\u{202E}',
            '\u{2066}', '\u{FEFF}', '\u{FFFE}',
        ] {
            assert!(!is_kept(c), "{c:?}");
        }
        // Words are not run together across lines, and Chinese has no spaces to begin with.
        assert!(space_between('a', 'b'));
        assert!(!space_between('a', ' '));
        assert!(!space_between('中', '文'));
        assert!(!space_between('a', '文'));
        // A line that ends in a hyphen goes on with a lower-case letter without one, not a capital.
        assert!(!space_between('-', 'a'));
        assert!(space_between('-', 'A'));
    }

    #[test]
    fn mupdf_reads_the_document_back_with_its_words() {
        // A second reader of what was written: MuPDF opens Word documents too. It is not Word, but
        // it takes the package and the markup apart on its own, and finds the same words.
        let pdf = pdf_of(&[
            (
                612,
                792,
                "BT /F1 12 Tf 72 720 Td (First page of text) Tj ET",
            ),
            (300, 400, "BT /F2 12 Tf 20 360 Td (Second page) Tj ET"),
        ]);
        let docx = docx_of(&pdf, &[0, 1]);
        let doc = mupdf::Document::from_bytes(
            &docx,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .expect("MuPDF opens it");
        let mut text = String::new();
        for number in 0..doc.page_count().expect("pages") {
            let page = doc.load_page(number).expect("page");
            text.push_str(
                &page
                    .to_text_page(TextPageFlags::empty())
                    .unwrap()
                    .to_text()
                    .unwrap(),
            );
        }
        assert!(text.contains("First page of text"), "{text}");
        assert!(text.contains("Second page"), "{text}");
    }

    /// The text of each paragraph of a document, its runs put together.
    fn paragraphs(xml: &str) -> Vec<String> {
        xml.split("<w:p>")
            .skip(1)
            .map(|paragraph| {
                paragraph
                    .split("</w:p>")
                    .next()
                    .unwrap()
                    .split("preserve\">")
                    .skip(1)
                    .map(|text| text.split("</w:t>").next().unwrap())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_page_of_headings_and_paragraphs_is_paragraphs_in_reading_order() {
        let content = "BT /F2 24 Tf 72 720 Td (A heading) Tj ET\n\
BT /F1 12 Tf 14 TL 72 680 Td (First paragraph, line one) Tj T* (line two of the first) Tj T* (line three of the first.) Tj ET\n\
BT /F1 12 Tf 14 TL 72 600 Td (Second paragraph, line one) Tj T* (line two of the second.) Tj ET\n\
BT /F1 12 Tf 72 540 Td (hyphen-) Tj 0 -14 Td (ated word) Tj ET";
        let docx = docx_of(&pdf_of(&[(612, 792, content)]), &[0]);
        let xml = part(&docx, "word/document.xml");
        assert_well_formed(&xml);
        let paragraphs = paragraphs(&xml);
        assert_eq!(
            paragraphs[..3],
            [
                "A heading",
                "First paragraph, line one line two of the first line three of the first.",
                "Second paragraph, line one line two of the second."
            ],
            "{paragraphs:?}"
        );
        assert!(
            paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("hyphen-ated word")),
            "{paragraphs:?}"
        );
        // The heading is the big and heavy one.
        let heading = runs(&xml)
            .into_iter()
            .find(|(_, text)| text == "A heading")
            .unwrap()
            .0;
        assert!(
            heading.contains("<w:b/>") && heading.contains("<w:sz w:val=\"48\"/>"),
            "{heading}"
        );
    }
}
