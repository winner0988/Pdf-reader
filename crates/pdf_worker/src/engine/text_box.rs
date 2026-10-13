//! Text boxes (B2-28, ADR 0018, docs/architecture/text-boxes.md): a FreeText annotation whose
//! words are set in a subset of the font the user chose, and whose appearance is written here.
//!
//! The annotation's dictionary and its appearance are written by hand, and MuPDF is never asked
//! to change a text box afterwards: `set_contents`, `set_rect` and `update()` make MuPDF draw the
//! annotation again from `/DA` (Helvetica, black), and the font and colour are gone. What the
//! user can change later (B2-29) is made by building the whole appearance again.
//!
//! The font is untrusted input: only `skrifa` and `subsetter`, which are memory safe, read it;
//! what goes into the PDF is a subset cut for this box, with a `ToUnicode` map, so that other
//! readers can search and copy the words.

use std::collections::BTreeMap;

use ipc_contract::limits::{MAX_TEXT_BOX_BYTES, MAX_TEXT_BOX_SIZE, MIN_TEXT_BOX_SIZE};
use ipc_contract::types::{FontRefusal, Rect, Rgb, TextAlign};
use mupdf::Buffer;
use mupdf::pdf::PdfObject;
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use subsetter::{GlyphRemapper, subset};

use super::annotations::binding_rect;
use super::{EngineError, PdfDocument};

/// What a text box says and how it is set.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBoxSpec<'a> {
    /// Lines are broken at `\n`, and where the box is not wide enough.
    pub text: &'a str,
    /// In points.
    pub size: f32,
    pub color: Rgb,
    pub align: TextAlign,
}

/// A font file, and which font of it is meant (0 unless it is a collection).
#[derive(Debug, Clone, Copy)]
pub struct TextBoxFont<'a> {
    pub bytes: &'a [u8],
    pub face: u32,
    /// What Windows calls it, kept in the text box to find the font again.
    pub name: &'a str,
}

/// What came of a text box that was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBoxMade {
    /// The font's licence allows a PDF with it to be viewed and printed only (OS/2 `fsType` 4).
    pub view_only: bool,
    /// The text does not fit the height of the box: what is below is cut off.
    pub overflow: bool,
}

/// A font that may be used, and what its licence says.
struct Face<'a> {
    font: FontRef<'a>,
    view_only: bool,
    per_em: f32,
}

/// OS/2 `fsType` (OpenType specification): a restricted licence, preview and print only, no
/// subsetting, bitmaps only.
const RESTRICTED: u16 = 0x0002;
const PREVIEW_AND_PRINT: u16 = 0x0004;
const NO_SUBSETTING: u16 = 0x0100;
const BITMAP_ONLY: u16 = 0x0200;

/// The space between the words and the edge of the box, in points.
const PADDING: f32 = 2.0;

fn has(font: &FontRef, tag: &[u8; 4]) -> bool {
    font.data_for_tag(Tag::new(tag)).is_some()
}

/// Opens the font `font` names, and says no if the first version cannot set text in it.
fn open_face<'a>(font: &TextBoxFont<'a>) -> Result<Face<'a>, FontRefusal> {
    let face = FontRef::from_index(font.bytes, font.face).map_err(|_| FontRefusal::NotAFont)?;
    if has(&face, b"fvar") {
        return Err(FontRefusal::Variable);
    }
    if [b"COLR", b"CBDT", b"sbix", b"SVG "]
        .iter()
        .any(|tag| has(&face, tag))
    {
        return Err(FontRefusal::Colour);
    }
    if !(has(&face, b"glyf") && has(&face, b"loca")) {
        return Err(FontRefusal::NotTrueType);
    }
    let fs_type = face.os2().map_or(0, |os2| os2.fs_type());
    if fs_type & (RESTRICTED | NO_SUBSETTING | BITMAP_ONLY) != 0 {
        return Err(FontRefusal::NotEmbeddable);
    }
    let per_em = f32::from(
        face.head()
            .map_err(|_| FontRefusal::NotAFont)?
            .units_per_em(),
    );
    if per_em < 16.0 {
        return Err(FontRefusal::NotAFont);
    }
    Ok(Face {
        font: face,
        view_only: fs_type & PREVIEW_AND_PRINT != 0,
        per_em,
    })
}

/// A character, its glyph in the font and the advance it takes, in em.
#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    id: u16,
    advance: f32,
}

/// The glyphs of `text`; a character the font has no glyph for is refused, not drawn as a box.
fn shape(face: &Face, text: &str) -> Result<Vec<Glyph>, FontRefusal> {
    let charmap = face.font.charmap();
    let metrics = face
        .font
        .glyph_metrics(Size::unscaled(), LocationRef::default());
    let mut glyphs = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' {
            glyphs.push(Glyph {
                ch,
                id: 0,
                advance: 0.0,
            });
            continue;
        }
        let id = charmap
            .map(ch)
            .map(|glyph| glyph.to_u32())
            .filter(|&glyph| glyph != 0 && glyph <= u32::from(u16::MAX))
            .ok_or(FontRefusal::MissingGlyphs)? as u16;
        let advance = metrics
            .advance_width(GlyphId::new(u32::from(id)))
            .unwrap_or(0.0)
            / face.per_em;
        glyphs.push(Glyph { ch, id, advance });
    }
    Ok(glyphs)
}

/// Characters the box does not set. A line of glyphs from the font's character map is right for
/// Latin, Greek, Cyrillic and the Chinese, Japanese and Korean letters (ADR 0018), and wrong for
/// what changes the shape or the place of a letter by its neighbours (Arabic and Hebrew, the
/// scripts of India and South-East Asia, marks that sit over a letter, Hangul jamo), and for the
/// controls that change the direction of the text. Those are refused, not drawn wrongly.
fn needs_layout(ch: char) -> bool {
    matches!(u32::from(ch),
        0x0300..=0x036F        // combining marks
        | 0x0590..=0x109F      // Hebrew to Myanmar: Arabic, Syriac, India, Thai, Lao, Tibetan
        | 0x1100..=0x11FF      // Hangul jamo
        | 0x1780..=0x17FF      // Khmer
        | 0x200B..=0x200F      // zero-width characters and direction marks
        | 0x202A..=0x202E      // direction embeddings and overrides
        | 0x2060..=0x206F      // word joiner, invisible operators, direction isolates
        | 0x3099..=0x309A      // combining marks of kana
        | 0xFB1D..=0xFDFF      // Hebrew and Arabic presentation forms
        | 0xFE00..=0xFE0F      // variation selectors
        | 0xFE20..=0xFE2F      // combining half marks
        | 0xFE70..=0xFEFF      // Arabic presentation forms, byte order mark
    )
}

/// Marks that do not start a line (a closing bracket, a full stop: Chinese has no spaces to break
/// at, and a line must not begin with one) and brackets that do not end one.
fn closes(ch: char) -> bool {
    "，。、．：；！？）］｝〕〉》」』】〗”’…".contains(ch)
        || matches!(ch, ',' | '.' | ')' | ']' | '}')
}

fn opens(ch: char) -> bool {
    "（［｛〔〈《「『【〖“‘".contains(ch) || matches!(ch, '(' | '[' | '{')
}

/// Breaks `glyphs` into lines no wider than `width` points at `size`: at the last space of the
/// line when there is one, else before the glyph that does not fit (Chinese has no spaces), and
/// not so that a line starts with a closing mark or ends with an opening one. A line broken at a
/// space does not keep it, a space that does not fit stays at the end of its line, and a line
/// holds at least one glyph, however wide.
fn wrap(glyphs: &[Glyph], size: f32, width: f32) -> Vec<Vec<Glyph>> {
    let mut lines: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut used = 0.0;
    for &glyph in glyphs {
        if glyph.ch == '\n' {
            lines.push(Vec::new());
            used = 0.0;
            continue;
        }
        let step = glyph.advance * size;
        let line = lines.last_mut().expect("a line");
        if used + step > width && !line.is_empty() && glyph.ch != ' ' {
            let first = line.iter().position(|g| g.ch != ' ').unwrap_or(line.len());
            let at = match line.iter().rposition(|g| g.ch == ' ') {
                Some(space) if space > first => space,
                _ => {
                    let mut at = line.len();
                    while at > 1 {
                        let next = line.get(at).map_or(glyph.ch, |g| g.ch);
                        if !closes(next) && !opens(line[at - 1].ch) {
                            break;
                        }
                        at -= 1;
                    }
                    at
                }
            };
            let mut rest = line.split_off(at);
            if rest.first().is_some_and(|g| g.ch == ' ') {
                rest.remove(0);
            }
            used = rest.iter().map(|g| g.advance * size).sum();
            lines.push(rest);
        }
        let line = lines.last_mut().expect("a line");
        line.push(glyph);
        used += step;
    }
    // A space at the end of a line has no ink, and would move a centred or right-aligned line.
    for line in &mut lines {
        while line.last().is_some_and(|g| g.ch == ' ') {
            line.pop();
        }
    }
    lines
}

/// The six capital letters a subset font's name starts with (`ABCDEF+Name`), from its bytes.
fn tag(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((hash >> (i * 5)) % 26) as u8))
        .collect()
}

/// The font's PostScript name as a PDF name allows it: letters, digits and hyphens.
fn base_name(face: &Face) -> String {
    let name: String = face
        .font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|name| name.chars().collect())
        .unwrap_or_default();
    let plain: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(40)
        .collect();
    if plain.is_empty() {
        "TextBox".to_owned()
    } else {
        plain
    }
}

fn utf16_hex(text: &str) -> String {
    let units: String = text
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("<FEFF{units}>")
}

fn parse(doc: &mupdf::pdf::PdfDocument, source: &str) -> Result<PdfObject, EngineError> {
    Ok(doc.new_object_from_str(source)?)
}

fn object_number(object: &PdfObject) -> Result<i32, EngineError> {
    Ok(object.as_indirect()?)
}

/// What the glyphs mean, so that other readers can search and copy the words.
fn to_unicode(
    doc: &mut mupdf::pdf::PdfDocument,
    used: &[Glyph],
    renumbered: &BTreeMap<u16, u16>,
) -> Result<PdfObject, EngineError> {
    let pairs: BTreeMap<u16, char> = used
        .iter()
        .map(|glyph| (renumbered[&glyph.id], glyph.ch))
        .collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def \
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", pairs.len()));
    for (id, ch) in &pairs {
        let mut units = [0u16; 2];
        let text: String = ch
            .encode_utf16(&mut units)
            .iter()
            .map(|unit| format!("{unit:04X}"))
            .collect();
        cmap.push_str(&format!("<{id:04X}> <{text}>\n"));
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end\n");
    Ok(doc.add_stream(&Buffer::from_bytes(cmap.as_bytes())?, None, false)?)
}

/// A subset of the font with the glyphs of `used`, as the PDF objects of a Type0 font over a
/// CIDFontType2, and the numbers the subset gave to those glyphs (they are what the text shows).
fn embed(
    doc: &mut mupdf::pdf::PdfDocument,
    face: &Face,
    font: &TextBoxFont,
    used: &[Glyph],
) -> Result<(PdfObject, BTreeMap<u16, u16>), EngineError> {
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);
    for glyph in used {
        remapper.remap(glyph.id);
    }
    let bytes = subset(font.bytes, font.face, &remapper)
        .map_err(|_| EngineError::Font(FontRefusal::NotAFont))?;
    let renumbered: BTreeMap<u16, u16> = used
        .iter()
        .filter_map(|glyph| remapper.get(glyph.id).map(|id| (glyph.id, id)))
        .collect();
    if renumbered.len()
        != used
            .iter()
            .map(|glyph| glyph.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    {
        return Err(EngineError::Font(FontRefusal::NotAFont));
    }
    let scale = 1000.0 / face.per_em;
    let metrics = face.font.metrics(Size::unscaled(), LocationRef::default());
    let head = face
        .font
        .head()
        .map_err(|_| EngineError::Font(FontRefusal::NotAFont))?;
    let base = format!("{}+{}", tag(&bytes), base_name(face));

    let length = parse(doc, &format!("<< /Length1 {} >>", bytes.len()))?;
    let file = doc.add_stream(&Buffer::from_bytes(&bytes)?, Some(&length), false)?;
    let descriptor = parse(
        doc,
        &format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] \
             /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {} 0 R >>",
            (f32::from(head.x_min()) * scale) as i32,
            (f32::from(head.y_min()) * scale) as i32,
            (f32::from(head.x_max()) * scale) as i32,
            (f32::from(head.y_max()) * scale) as i32,
            (metrics.ascent * scale) as i32,
            (metrics.descent * scale) as i32,
            (metrics.cap_height.unwrap_or(metrics.ascent) * scale) as i32,
            object_number(&file)?,
        ),
    )?;
    let descriptor = doc.add_object(&descriptor)?;
    let widths: BTreeMap<u16, i32> = used
        .iter()
        .map(|glyph| {
            (
                renumbered[&glyph.id],
                (glyph.advance * 1000.0).round() as i32,
            )
        })
        .collect();
    let widths: String = widths
        .iter()
        .map(|(id, width)| format!("{id} [{width}] "))
        .collect();
    let descendant = parse(
        doc,
        &format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>",
            object_number(&descriptor)?,
        ),
    )?;
    let descendant = doc.add_object(&descendant)?;
    let unicode = to_unicode(doc, used, &renumbered)?;
    let type0 = parse(
        doc,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H \
             /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
            object_number(&descendant)?,
            object_number(&unicode)?,
        ),
    )?;
    Ok((doc.add_object(&type0)?, renumbered))
}

/// The drawing operators of `lines` in a box of `width` x `height` points (origin at the bottom
/// left, as in the appearance stream), and whether the lines go below the box.
fn drawing(
    face: &Face,
    lines: &[Vec<Glyph>],
    renumbered: &BTreeMap<u16, u16>,
    spec: &TextBoxSpec,
    (width, height): (f32, f32),
) -> (String, bool) {
    let metrics = face.font.metrics(Size::unscaled(), LocationRef::default());
    let ascent = (metrics.ascent / face.per_em).max(0.8);
    let step = ((metrics.ascent - metrics.descent + metrics.leading) / face.per_em)
        .clamp(1.15, 1.6)
        * spec.size;
    let [r, g, b] = [spec.color.r, spec.color.g, spec.color.b].map(|c| f32::from(c) / 255.0);
    let mut out = format!("BT\n/F0 {} Tf\n{r:.4} {g:.4} {b:.4} rg\n", spec.size);
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let used: f32 = line.iter().map(|glyph| glyph.advance * spec.size).sum();
        let x = match spec.align {
            TextAlign::Left => PADDING,
            TextAlign::Center => (width - used) / 2.0,
            TextAlign::Right => width - PADDING - used,
        };
        let y = height - PADDING - ascent * spec.size - index as f32 * step;
        let hex: String = line
            .iter()
            .map(|glyph| format!("{:04X}", renumbered[&glyph.id]))
            .collect();
        out.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"));
    }
    out.push_str("ET\n");
    let bottom = PADDING + ascent * spec.size + (lines.len() - 1) as f32 * step;
    (out, bottom > height)
}

fn finite(rect: Rect) -> bool {
    [rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .all(|edge| edge.is_finite())
}

impl PdfDocument {
    /// Puts a text box over `rect` (page space) on page `index`: a `FreeText` annotation that says
    /// the words and whose appearance sets them in a subset of `font`. A font that cannot be used
    /// (see [`FontRefusal`]), or has no glyph for a character, makes no text box.
    pub fn add_text_box(
        &mut self,
        index: u32,
        rect: Rect,
        spec: &TextBoxSpec,
        font: &TextBoxFont,
    ) -> Result<TextBoxMade, EngineError> {
        if spec.text.trim().is_empty() || spec.text.len() > MAX_TEXT_BOX_BYTES {
            return Err(EngineError::InvalidEdit("the words of a text box"));
        }
        if spec.text.chars().any(needs_layout) {
            return Err(EngineError::InvalidEdit(
                "a script that a text box cannot set",
            ));
        }
        if !spec.size.is_finite() || !(MIN_TEXT_BOX_SIZE..=MAX_TEXT_BOX_SIZE).contains(&spec.size) {
            return Err(EngineError::InvalidEdit("the size of a text box"));
        }
        if !finite(rect) || rect.x1 - rect.x0 < 8.0 || rect.y1 - rect.y0 < 8.0 {
            return Err(EngineError::InvalidEdit("the rectangle of a text box"));
        }
        let face = open_face(font).map_err(EngineError::Font)?;
        let glyphs = shape(&face, spec.text).map_err(EngineError::Font)?;
        let (width, height) = (rect.x1 - rect.x0, rect.y1 - rect.y0);
        let lines = wrap(&glyphs, spec.size, width - 2.0 * PADDING);
        let used: Vec<Glyph> = lines.iter().flatten().copied().collect();
        // The page is looked at before anything is put in the document: a box that cannot be
        // made leaves no objects behind.
        let page = self.pdf_page(index)?;
        let mut page_object = page.object();
        let inverse = page_object
            .page_ctm()?
            .invert()
            .ok_or(EngineError::InvalidEdit("the page cannot be placed"))?;
        let (font_object, renumbered) = embed(&mut self.doc, &face, font, &used)?;
        let (content, overflow) = drawing(&face, &lines, &renumbered, spec, (width, height));

        // The page space of MuPDF has its origin at the top left and goes down; the PDF's has it
        // where the page's box says and goes up, and a page may be turned. The box is drawn in
        // its own space (up from the bottom left): this takes it to the page's, and from there to
        // the PDF's. The form carries the turn only; where it goes is the annotation's `/Rect`.
        let at = binding_rect(rect).transform(&inverse);
        let mut turn = mupdf::Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 0.0);
        turn.concat(inverse);

        let form = parse(
            &self.doc,
            &format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
                 /Matrix [{:.5} {:.5} {:.5} {:.5} 0 0] \
                 /Resources << /Font << /F0 {} 0 R >> >> >>",
                turn.a,
                turn.b,
                turn.c,
                turn.d,
                object_number(&font_object)?,
            ),
        )?;
        let stream =
            self.doc
                .add_stream(&Buffer::from_bytes(content.as_bytes())?, Some(&form), false)?;
        let quadding = match spec.align {
            TextAlign::Left => 0,
            TextAlign::Center => 1,
            TextAlign::Right => 2,
        };
        let [r, g, b] = [spec.color.r, spec.color.g, spec.color.b];
        let annotation = parse(
            &self.doc,
            &format!(
                "<< /Type /Annot /Subtype /FreeText /Rect [{:.3} {:.3} {:.3} {:.3}] \
                 /Contents {} /DA (/Helv {} Tf {:.4} {:.4} {:.4} rg) /Q {quadding} /F 4 \
                 /IT /FreeTextTypeWriter /BS << /W 0 >> /AP << /N {} 0 R >> \
                 /PdfReaderTextBox << /Font {} /Face {} /Size {} /Color [{r} {g} {b}] >> >>",
                at.x0,
                at.y0,
                at.x1,
                at.y1,
                utf16_hex(spec.text),
                spec.size,
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                object_number(&stream)?,
                utf16_hex(font.name),
                font.face,
                spec.size,
            ),
        )?;
        let annotation = self.doc.add_object(&annotation)?;
        match page_object.get_dict("Annots")? {
            Some(mut annots) => annots.array_push(annotation)?,
            None => {
                let mut created = self.doc.new_array()?;
                created.array_push(annotation)?;
                page_object.dict_put("Annots", created)?;
            }
        }
        Ok(TextBoxMade {
            view_only: face.view_only,
            overflow,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph one em wide, so that at size 1 a width is a number of glyphs.
    fn glyphs(text: &str) -> Vec<Glyph> {
        text.chars()
            .map(|ch| Glyph {
                ch,
                id: 1,
                advance: 1.0,
            })
            .collect()
    }

    fn lines(text: &str, width: f32) -> Vec<String> {
        wrap(&glyphs(text), 1.0, width)
            .iter()
            .map(|line| line.iter().map(|g| g.ch).collect())
            .collect()
    }

    #[test]
    fn english_is_broken_at_the_last_space_that_fits_and_the_space_is_not_kept() {
        assert_eq!(lines("aa bb cc", 5.0), ["aa bb", "cc"]);
        assert_eq!(lines("aa bb cc", 4.0), ["aa", "bb", "cc"]);
        assert_eq!(lines("ab cd", 2.0), ["ab", "cd"]);
        assert_eq!(lines("ab  cd", 2.0), ["ab", "cd"]);
        assert_eq!(lines("ab   ", 9.0), ["ab"]);
    }

    #[test]
    fn a_word_longer_than_the_line_and_chinese_are_broken_before_the_glyph_that_does_not_fit() {
        assert_eq!(lines("abcdefgh", 3.0), ["abc", "def", "gh"]);
        assert_eq!(lines("ab abcdefgh", 3.0), ["ab", "abc", "def", "gh"]);
        assert_eq!(lines("一二三四五六七", 3.0), ["一二三", "四五六", "七"]);
    }

    #[test]
    fn a_line_does_not_start_with_a_closing_mark_or_end_with_an_opening_one() {
        assert_eq!(lines("一二三。四", 3.0), ["一二", "三。四"]);
        assert_eq!(lines("一二三。。四", 3.0), ["一二", "三。。", "四"]);
        assert_eq!(lines("一二三（四", 3.0), ["一二三", "（四"]);
        assert_eq!(lines("一二（三四", 3.0), ["一二", "（三四"]);
        // When it cannot be done, the line keeps its one glyph.
        assert_eq!(lines("。。。", 1.0), ["。", "。", "。"]);
    }

    #[test]
    fn newlines_make_lines_and_nothing_makes_one_empty_line() {
        assert_eq!(lines("a\n\nb", 9.0), ["a", "", "b"]);
        assert_eq!(lines("", 9.0), [""]);
        assert_eq!(lines("  abcdef", 3.0), ["  a", "bcd", "ef"]);
    }

    #[test]
    fn a_glyph_wider_than_the_line_still_gets_a_line_of_its_own() {
        assert_eq!(lines("abc", 0.5), ["a", "b", "c"]);
    }

    #[test]
    fn the_width_of_the_lines_is_the_sum_of_the_advances_at_the_size() {
        let narrow: Vec<Glyph> = "ab cd ef"
            .chars()
            .map(|ch| Glyph {
                ch,
                id: 1,
                advance: if ch == ' ' { 0.25 } else { 0.5 },
            })
            .collect();
        // At 10 points a letter is 5 wide and a space 2.5: "ab" is 10, "ab c" is 17.5 and fits in
        // 18, "ab cd" is 22.5 and does not.
        let broken: Vec<String> = wrap(&narrow, 10.0, 18.0)
            .iter()
            .map(|line| line.iter().map(|g| g.ch).collect())
            .collect();
        assert_eq!(broken, ["ab", "cd", "ef"]);
    }

    #[test]
    fn scripts_that_need_layout_are_told_from_those_that_do_not() {
        for needs in [
            '\u{628}', '\u{5d0}', '\u{915}', '\u{e01}', '\u{301}', '\u{202e}', '\u{200d}',
            '\u{1100}', '\u{fb50}', '\u{fe0f}', '\u{feff}', '\u{3099}',
        ] {
            assert!(needs_layout(needs), "{needs:?}");
        }
        for fine in [
            'a', 'Z', '7', ' ', 'é', 'Ω', 'Ж', '你', '體', 'か', 'カ', '한', '。', '€', '¥',
        ] {
            assert!(!needs_layout(fine), "{fine:?}");
        }
    }

    #[test]
    fn the_text_of_a_pdf_string_is_utf16_with_a_byte_order_mark() {
        assert_eq!(utf16_hex("A你"), "<FEFF00414F60>");
        assert_eq!(utf16_hex("😀"), "<FEFFD83DDE00>");
        assert_eq!(utf16_hex(""), "<FEFF>");
    }

    #[test]
    fn the_tag_of_a_subset_is_six_capitals_that_follow_the_bytes() {
        let one = tag(b"one");
        assert_eq!(one.len(), 6);
        assert!(one.chars().all(|c| c.is_ascii_uppercase()));
        assert_eq!(one, tag(b"one"));
        assert_ne!(one, tag(b"two"));
    }
}
