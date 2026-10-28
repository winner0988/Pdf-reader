//! A text box as PDF objects: the font of its model (a subset, as a Type0 font over a
//! CIDFontType2), and the private entry of its annotation that holds the model again, so that a
//! box can be set again from the file alone (moved, resized, another colour) without the font.

use std::collections::BTreeMap;

use ipc_contract::limits::{MAX_TEXT_BOX_GLYPHS, MAX_TEXT_BOX_SUBSET_BYTES};
use ipc_contract::types::{Rgb, TextAlign, TextBoxStyle};
use ipc_contract::validate::{Validate, check_text_box_text};
use ipc_contract::worker::{ModelGlyph, TextBoxModel};
use mupdf::Buffer;
use mupdf::pdf::PdfObject;

use super::EngineError;

/// The entry of the annotation dictionary that says the app made it, and holds the model.
pub(in crate::engine) const KEY: &str = "PdfReaderTextBox";

/// Version of the private entry.
const VERSION: i32 = 1;

pub(super) fn parse(doc: &mupdf::pdf::PdfDocument, source: &str) -> Result<PdfObject, EngineError> {
    Ok(doc.new_object_from_str(source)?)
}

pub(super) fn object_number(object: &PdfObject) -> Result<i32, EngineError> {
    Ok(object.as_indirect()?)
}

/// `text` as a PDF text string: UTF-16 with a byte order mark, in hexadecimal.
pub(in crate::engine) fn utf16_hex(text: &str) -> String {
    let units: String = text
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("<FEFF{units}>")
}

/// The text of a PDF text string made by [`utf16_hex`]; `None` for anything else.
fn from_utf16(bytes: &[u8]) -> Option<String> {
    let body = bytes.strip_prefix(&[0xFE, 0xFF])?;
    let (pairs, rest) = body.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    String::from_utf16(
        &pairs
            .iter()
            .map(|&[high, low]| u16::from_be_bytes([high, low]))
            .collect::<Vec<_>>(),
    )
    .ok()
}

/// What the glyphs mean, so that other readers can search and copy the words.
fn to_unicode(
    doc: &mut mupdf::pdf::PdfDocument,
    model: &TextBoxModel,
) -> Result<PdfObject, EngineError> {
    let mut pairs: BTreeMap<u16, char> = BTreeMap::new();
    for glyph in &model.glyphs {
        pairs.entry(glyph.cid).or_insert(glyph.ch);
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def \
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", pairs.len()));
    for (cid, ch) in &pairs {
        let mut units = [0u16; 2];
        let text: String = ch
            .encode_utf16(&mut units)
            .iter()
            .map(|unit| format!("{unit:04X}"))
            .collect();
        cmap.push_str(&format!("<{cid:04X}> <{text}>\n"));
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end\n");
    Ok(doc.add_stream(&Buffer::from_bytes(cmap.as_bytes())?, None, false)?)
}

/// The font of `model` as objects of `doc`, with `subset` as its file: the Type0 font, and the
/// stream of the file.
pub(super) fn write_font(
    doc: &mut mupdf::pdf::PdfDocument,
    model: &TextBoxModel,
    subset: &[u8],
) -> Result<(PdfObject, PdfObject), EngineError> {
    let base = &model.base;
    let length = parse(doc, &format!("<< /Length1 {} >>", subset.len()))?;
    let file = doc.add_stream(&Buffer::from_bytes(subset)?, Some(&length), false)?;
    let descriptor = parse(
        doc,
        &format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] \
             /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {} 0 R >>",
            model.bbox[0],
            model.bbox[1],
            model.bbox[2],
            model.bbox[3],
            model.ascent,
            model.descent,
            model.cap_height,
            object_number(&file)?,
        ),
    )?;
    let descriptor = doc.add_object(&descriptor)?;
    let widths: BTreeMap<u16, u16> = model
        .glyphs
        .iter()
        .map(|glyph| (glyph.cid, glyph.advance))
        .collect();
    let widths: String = widths
        .iter()
        .map(|(cid, width)| format!("{cid} [{width}] "))
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
    let unicode = to_unicode(doc, model)?;
    let type0 = parse(
        doc,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H \
             /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
            object_number(&descendant)?,
            object_number(&unicode)?,
        ),
    )?;
    Ok((doc.add_object(&type0)?, file))
}

fn quadding(align: TextAlign) -> i32 {
    match align {
        TextAlign::Left => 0,
        TextAlign::Center => 1,
        TextAlign::Right => 2,
    }
}

/// The private entry that holds `model` and `style`, as dictionary source; `file` is the stream
/// of the subset. `overflow`: the words do not fit the box.
pub(super) fn entry_source(
    model: &TextBoxModel,
    style: &TextBoxStyle,
    overflow: bool,
    file: &PdfObject,
) -> Result<String, EngineError> {
    let glyphs: String = model
        .glyphs
        .iter()
        .map(|glyph| format!("{} {} {} ", glyph.cid, glyph.advance, u32::from(glyph.ch)))
        .collect();
    Ok(format!(
        "<< /V {VERSION} /Text {} /Font {} /Face {} /Base /{} \
         /M [{} {} {} {} {} {} {} {}] /G [{glyphs}] /S {} /C [{} {} {}] /A {} \
         /Over {overflow} /View {} /File {} 0 R >>",
        utf16_hex(&model.text),
        utf16_hex(&model.font),
        model.face,
        model.base,
        model.ascent,
        model.descent,
        model.cap_height,
        model.step,
        model.bbox[0],
        model.bbox[1],
        model.bbox[2],
        model.bbox[3],
        style.size,
        style.color.r,
        style.color.g,
        style.color.b,
        quadding(style.align),
        model.view_only,
        object_number(file)?,
    ))
}

/// The value of `key` in `entry`, which must be there.
fn field(entry: &PdfObject, key: &str) -> Result<PdfObject, EngineError> {
    entry
        .get_dict(key)?
        .ok_or(EngineError::InvalidEdit("a text box that is not the app's"))
}

fn int(entry: &PdfObject, key: &str) -> Result<i32, EngineError> {
    Ok(field(entry, key)?.as_int()?)
}

/// The integers of the array `key` of `entry`, at most `max` of them.
fn ints(entry: &PdfObject, key: &str, max: usize) -> Result<Vec<i32>, EngineError> {
    let array = field(entry, key)?;
    let length = array.len()?;
    if length > max {
        return Err(EngineError::InvalidEdit("a text box that is not the app's"));
    }
    (0..length)
        .map(|at| {
            let item = array
                .get_array(i32::try_from(at).map_err(|_| EngineError::TooComplex)?)?
                .ok_or(EngineError::InvalidEdit("a text box that is not the app's"))?;
            Ok(item.as_int()?)
        })
        .collect()
}

/// The size, colour and alignment in the private entry `entry`.
fn style_of(entry: &PdfObject) -> Result<TextBoxStyle, EngineError> {
    let foreign = || EngineError::InvalidEdit("a text box that is not the app's");
    let size = field(entry, "S")?.as_float()?;
    let color = ints(entry, "C", 3)?;
    let [r, g, b] = <[i32; 3]>::try_from(color).map_err(|_| foreign())?;
    let channel = |value: i32| u8::try_from(value).map_err(|_| foreign());
    let align = match int(entry, "A")? {
        0 => TextAlign::Left,
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => return Err(foreign()),
    };
    Ok(TextBoxStyle {
        size,
        color: Rgb {
            r: channel(r)?,
            g: channel(g)?,
            b: channel(b)?,
        },
        align,
    })
}

/// The words, the font family and the style of a text box, read back from its private entry
/// without its font file (B3-23, ADR 0019).
pub(in crate::engine) fn read_source_text_box_info(
    entry: &PdfObject,
) -> Result<(String, String, TextBoxStyle), EngineError> {
    let foreign = || EngineError::InvalidEdit("a text box that is not the app's");
    if int(entry, "V")? != VERSION {
        return Err(foreign());
    }
    let text = from_utf16(&field(entry, "Text")?.as_bytes()?).ok_or_else(foreign)?;
    check_text_box_text(&text).map_err(|_| foreign())?;
    let font = from_utf16(&field(entry, "Font")?.as_bytes()?).ok_or_else(foreign)?;
    let style = style_of(entry)?;
    style.validate().map_err(|_| foreign())?;
    Ok((text, font, style))
}

/// The words, the glyph widths and the style of a text box, read back from its private entry
/// without its font file: what the text layer needs (B2-29). The model has no subset, so it is not
/// one to set a box from. What a file says is not trusted: the words and the style are checked,
/// and the glyph table is bounded.
pub(super) fn read_for_layout(
    entry: &PdfObject,
) -> Result<(TextBoxModel, TextBoxStyle), EngineError> {
    let foreign = || EngineError::InvalidEdit("a text box that is not the app's");
    if int(entry, "V")? != VERSION {
        return Err(foreign());
    }
    let text = from_utf16(&field(entry, "Text")?.as_bytes()?).ok_or_else(foreign)?;
    check_text_box_text(&text).map_err(|_| foreign())?;
    let metrics = ints(entry, "M", 8)?;
    let [ascent, descent, cap_height, step, left, bottom, right, top] =
        <[i32; 8]>::try_from(metrics).map_err(|_| foreign())?;
    let flat = ints(entry, "G", 3 * MAX_TEXT_BOX_GLYPHS)?;
    let (triples, rest) = flat.as_chunks::<3>();
    if !rest.is_empty() {
        return Err(foreign());
    }
    let glyphs = triples
        .iter()
        .map(|&[cid, advance, code]| {
            Some(ModelGlyph {
                ch: char::from_u32(u32::try_from(code).ok()?)?,
                cid: u16::try_from(cid).ok()?,
                advance: u16::try_from(advance).ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(foreign)?;
    let style = style_of(entry)?;
    style.validate().map_err(|_| foreign())?;
    let model = TextBoxModel {
        text,
        font: String::new(),
        face: 0,
        view_only: false,
        base: String::new(),
        ascent,
        descent,
        cap_height,
        step,
        bbox: [left, bottom, right, top],
        glyphs,
        subset: Vec::new(),
    };
    Ok((model, style))
}

/// The model, the style and whether the words overflow, read back from the private entry of a
/// text box; checked like a model from anywhere else. What a file says is not trusted.
pub(super) fn read_entry(
    entry: &PdfObject,
) -> Result<(TextBoxModel, TextBoxStyle, bool), EngineError> {
    let foreign = || EngineError::InvalidEdit("a text box that is not the app's");
    if int(entry, "V")? != VERSION {
        return Err(foreign());
    }
    let text = from_utf16(&field(entry, "Text")?.as_bytes()?).ok_or_else(foreign)?;
    let font = from_utf16(&field(entry, "Font")?.as_bytes()?).ok_or_else(foreign)?;
    let base = String::from_utf8(field(entry, "Base")?.as_name()?).map_err(|_| foreign())?;
    let metrics = ints(entry, "M", 8)?;
    let [ascent, descent, cap_height, step, left, bottom, right, top] =
        <[i32; 8]>::try_from(metrics).map_err(|_| foreign())?;
    let flat = ints(entry, "G", 3 * MAX_TEXT_BOX_GLYPHS)?;
    let (triples, rest) = flat.as_chunks::<3>();
    if !rest.is_empty() {
        return Err(foreign());
    }
    let glyphs = triples
        .iter()
        .map(|&[cid, advance, code]| {
            Some(ModelGlyph {
                ch: char::from_u32(u32::try_from(code).ok()?)?,
                cid: u16::try_from(cid).ok()?,
                advance: u16::try_from(advance).ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(foreign)?;
    let style = style_of(entry)?;
    let overflow = field(entry, "Over")?.as_bool()?;
    let view_only = field(entry, "View")?.as_bool()?;
    let file = field(entry, "File")?;
    if !file.is_stream()? {
        return Err(foreign());
    }
    let subset = file.read_stream()?;
    if subset.len() > MAX_TEXT_BOX_SUBSET_BYTES {
        return Err(foreign());
    }
    let model = TextBoxModel {
        text,
        font,
        face: u32::try_from(int(entry, "Face")?).map_err(|_| foreign())?,
        view_only,
        base,
        ascent,
        descent,
        cap_height,
        step,
        bbox: [left, bottom, right, top],
        glyphs,
        subset,
    };
    model.validate().map_err(|_| foreign())?;
    style.validate().map_err(|_| foreign())?;
    Ok((model, style, overflow))
}

/// The `/DA` default appearance string other readers use to draw the text again.
pub(super) fn default_appearance(style: &TextBoxStyle) -> String {
    format!(
        "/Helv {} Tf {:.4} {:.4} {:.4} rg",
        style.size,
        f32::from(style.color.r) / 255.0,
        f32::from(style.color.g) / 255.0,
        f32::from(style.color.b) / 255.0,
    )
}

pub(super) fn quadding_of(style: &TextBoxStyle) -> i32 {
    quadding(style.align)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_of_a_pdf_string_is_utf16_with_a_byte_order_mark_and_comes_back() {
        assert_eq!(utf16_hex("A你"), "<FEFF00414F60>");
        assert_eq!(utf16_hex("😀"), "<FEFFD83DDE00>");
        assert_eq!(utf16_hex(""), "<FEFF>");
        assert_eq!(
            from_utf16(&[0xFE, 0xFF, 0, 0x41, 0x4F, 0x60]).as_deref(),
            Some("A你")
        );
        assert_eq!(
            from_utf16(&[0xFE, 0xFF, 0xD8, 0x3D, 0xDE, 0x00]).as_deref(),
            Some("😀")
        );
        // Not made by `utf16_hex`: no mark, an odd length, half of a pair.
        assert_eq!(from_utf16(b"A"), None);
        assert_eq!(from_utf16(&[0xFE, 0xFF, 0x41]), None);
        assert_eq!(from_utf16(&[0xFE, 0xFF, 0xD8, 0x3D]), None);
    }
}
