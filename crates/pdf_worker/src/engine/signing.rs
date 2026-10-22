//! Signing a document (B3-06, ADR 0020, docs/architecture/signing.md): the worker's part. It
//! appends to the file a signature field whose signature dictionary keeps room for the signature
//! and has a `/ByteRange` that covers every other byte; the main process signs those bytes and
//! writes the signature into the room. No key, password or certificate ever comes here: the
//! worker only lays out what the main process then signs.

use std::io::Write;

use ipc_contract::limits::{
    MAX_SIGNATURE_TEXT_BYTES, MAX_SIGNING_REASON_BYTES, SIGNATURE_RESERVE_BYTES,
};
use ipc_contract::types::{PageCorner, Rgb};
use ipc_contract::validate::check_signing_text;
use ipc_contract::worker::{NewSignature, SigningTime, VisibleSignature};
use mupdf::Buffer;
use mupdf::pdf::{PdfObject, PdfWriteOptions};

use super::text_box::pdf::utf16_hex;
use super::text_box::{TextBoxFont, line_form};
use super::{EngineError, PdfDocument, write_limited};

/// What each number of `/ByteRange` is while the copy is written: as wide as any offset in a
/// document can be, so that the real ones, known once it is written, fit in its place.
const BYTE_RANGE_PLACEHOLDER: i32 = i32::MAX;

/// A visible signature's box, in points: its size, and how far it is from the page's edges.
const BOX_WIDTH_PT: f32 = 216.0;
const BOX_HEIGHT_PT: f32 = 54.0;
const MARGIN_PT: f32 = 36.0;
/// How large the words in the box are, and how far from its edges.
const TEXT_SIZE_PT: f32 = 9.0;
const PADDING_PT: f32 = 4.0;
/// Most lines in the box (the signer, the date, the reason), and the longest one, in UTF-8 bytes.
const MAX_LINES: usize = 3;
const MAX_LINE_BYTES: u32 = 512;
/// The font of the words: the one the worker carries for Chinese, which has Latin letters too.
const FONT_NAME: &str = "Droid Sans Fallback";
const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

/// Where the signature goes in the copy that [`prepare_signature`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedSignature {
    /// How long the copy is.
    pub bytes: u64,
    /// Where `/Contents` starts (its `<`) and where it ends (after its `>`).
    pub contents_start: u64,
    pub contents_end: u64,
}

/// Writes to `out` the PDF file `bytes` with a signature field appended by an incremental update
/// (ADR 0020): first the file's own bytes, unchanged, then the field, its widget (on the page of
/// a visible signature, with its box; otherwise on the first page, with no area), and its
/// signature dictionary, which says `signature` and keeps room for the signature in hexadecimal
/// zeros. `/ByteRange` covers every byte of the copy but that room. `bytes` is the file the
/// document was opened from or last saved to, opened again here on its own: the open document is
/// not touched.
///
/// Refused:
/// - for an encrypted document (`NotAllowed`, or `Encrypted` when it needs a password to open);
/// - for one certified to allow no change, not even another signature (DocMDP `/P` 1:
///   `NotAllowed`);
/// - for a file MuPDF had to repair to open it (`NotAppendable`): its own bytes cannot be kept as
///   they are.
pub fn prepare_signature(
    bytes: &[u8],
    signature: &NewSignature,
    out: &mut impl Write,
) -> Result<PreparedSignature, EngineError> {
    check(signature)?;
    let mut document = PdfDocument::open(bytes, None)?;
    if document.is_encrypted() {
        return Err(EngineError::NotAllowed(
            "an encrypted document is not signed",
        ));
    }
    if document
        .signature_fields()?
        .fields
        .iter()
        .any(|field| field.certification == Some(1))
    {
        return Err(EngineError::NotAllowed(
            "the document is certified to allow no change",
        ));
    }
    if !document.doc.can_be_saved_incrementally() {
        return Err(EngineError::NotAppendable);
    }
    document.add_signature_field(signature)?;

    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    let mut copy = Vec::new();
    write_limited(&document.doc, &mut copy, options)?;
    let prepared = fill_byte_range(&mut copy, bytes.len())?;
    out.write_all(&copy).map_err(EngineError::Write)?;
    Ok(prepared)
}

/// What the main process sent is what a signature can say.
fn check(signature: &NewSignature) -> Result<(), EngineError> {
    let invalid = |_| EngineError::InvalidEdit("what the signature says");
    check_signing_text("signer", &signature.signer, MAX_SIGNATURE_TEXT_BYTES).map_err(invalid)?;
    if let Some(reason) = &signature.reason {
        check_signing_text("signing reason", reason, MAX_SIGNING_REASON_BYTES).map_err(invalid)?;
    }
    let SigningTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        utc_offset_minutes,
    } = signature.time;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || utc_offset_minutes.abs() > 14 * 60
    {
        return Err(EngineError::InvalidEdit("the time of the signature"));
    }
    if let Some(visible) = &signature.visible
        && (visible.lines.is_empty() || visible.lines.len() > MAX_LINES)
    {
        return Err(EngineError::InvalidEdit("the lines of a visible signature"));
    }
    for line in signature.visible.iter().flat_map(|visible| &visible.lines) {
        check_signing_text("signature line", line, MAX_LINE_BYTES).map_err(invalid)?;
    }
    Ok(())
}

/// `time` as PDF writes a date (ISO 32000-1, 7.9.4): `D:YYYYMMDDHHmmSS` and the time zone.
fn pdf_date(time: &SigningTime) -> String {
    let zone = match time.utc_offset_minutes {
        0 => "Z".to_owned(),
        offset => format!(
            "{}{:02}'{:02}'",
            if offset < 0 { '-' } else { '+' },
            offset.unsigned_abs() / 60,
            offset.unsigned_abs() % 60,
        ),
    };
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}{zone}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

impl PdfDocument {
    /// Adds the signature field of `signature`, with its widget and its signature dictionary.
    fn add_signature_field(&mut self, signature: &NewSignature) -> Result<(), EngineError> {
        let reason = signature
            .reason
            .as_deref()
            .map(|reason| format!(" /Reason {}", utf16_hex(reason)))
            .unwrap_or_default();
        let value = self.doc.new_object_from_str(&format!(
            "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached \
             /Name {} /M ({}){reason} \
             /ByteRange [0 {BYTE_RANGE_PLACEHOLDER} {BYTE_RANGE_PLACEHOLDER} {BYTE_RANGE_PLACEHOLDER}] \
             /Contents <{}> >>",
            utf16_hex(&signature.signer),
            pdf_date(&signature.time),
            "0".repeat(2 * SIGNATURE_RESERVE_BYTES as usize),
        ))?;
        let value = self.doc.add_object(&value)?;

        let page_index = signature
            .visible
            .as_ref()
            .map_or(0, |visible| visible.page_index);
        if page_index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(page_index));
        }
        let (mut page, inverse) = self.page_to_pdf(page_index)?;
        let (rect, appearance) = match &signature.visible {
            Some(visible) => {
                let (rect, form) = self.signature_box(visible, inverse)?;
                (rect, format!(" /AP << /N {} 0 R >>", form.as_indirect()?))
            }
            None => ([0.0; 4], String::new()),
        };
        let name = self.free_signature_name()?;
        // Printed with the page, and locked: the field cannot be changed or deleted.
        let field = self.doc.new_object_from_str(&format!(
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T ({name}) /V {} 0 R /F 132 /P {} 0 R \
             /Rect [{:.3} {:.3} {:.3} {:.3}]{appearance} >>",
            value.as_indirect()?,
            page.as_indirect()?,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
        ))?;
        let field = self.doc.add_object(&field)?;
        match page.get_dict("Annots")? {
            Some(mut annots) if annots.is_array()? => annots.array_push(field.try_clone()?)?,
            _ => {
                let mut annots = self.doc.new_array()?;
                annots.array_push(field.try_clone()?)?;
                page.dict_put("Annots", annots)?;
            }
        }

        let mut catalog = self.doc.catalog()?;
        let mut form = match catalog.get_dict("AcroForm")? {
            Some(form) if form.is_dict()? => form,
            _ => {
                catalog.dict_put("AcroForm", self.doc.new_dict()?)?;
                catalog
                    .get_dict("AcroForm")?
                    .ok_or(EngineError::InvalidEdit("the document's form"))?
            }
        };
        match form.get_dict("Fields")? {
            Some(mut fields) if fields.is_array()? => fields.array_push(field)?,
            _ => {
                let mut fields = self.doc.new_array()?;
                fields.array_push(field)?;
                form.dict_put("Fields", fields)?;
            }
        }
        // SignaturesExist and AppendOnly (ISO 32000-1, 12.7.2): a reader keeps later changes to
        // incremental updates, so that the signature still holds for what it signed.
        let flags = match form.get_dict("SigFlags")? {
            Some(flags) if flags.is_int()? => flags.as_int()?,
            _ => 0,
        };
        form.dict_put("SigFlags", self.doc.new_int(flags | 3)?)?;
        Ok(())
    }

    /// `Signature1`, or the first of `Signature2`, `Signature3`... that no field at the top of
    /// the form is called.
    fn free_signature_name(&self) -> Result<String, EngineError> {
        let mut taken = Vec::new();
        if let Some(form) = self.doc.catalog()?.get_dict("AcroForm")?
            && let Some(fields) = form.get_dict("Fields")?
            && fields.is_array()?
        {
            for index in 0..fields.len()?.min(super::MAX_FORM_FIELDS) {
                if let Some(field) = fields.get_array(index as i32)?
                    && let Some(name) = field.get_dict("T")?
                    && let Ok(name) = name.as_string()
                {
                    taken.push(name);
                }
            }
        }
        Ok((1..)
            .map(|number| format!("Signature{number}"))
            .find(|name| !taken.contains(name))
            .expect("some name is free"))
    }

    /// The box of a visible signature: where it goes (`/Rect`, PDF space) and its appearance, a
    /// form with a thin frame and `visible.lines`, set as a text box's words are (ADR 0018), each
    /// made smaller if it would not fit. It sits in a corner of the page as the page is shown,
    /// level as it is seen; on a small page it is smaller.
    fn signature_box(
        &mut self,
        visible: &VisibleSignature,
        inverse: mupdf::Matrix,
    ) -> Result<([f32; 4], PdfObject), EngineError> {
        let bounds = self.load_page(visible.page_index)?.bounds()?;
        let (page_width, page_height) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        if !(page_width > 0.0 && page_height > 0.0) {
            return Err(EngineError::NoArea);
        }
        let width = BOX_WIDTH_PT.min(page_width * 0.9);
        let height = BOX_HEIGHT_PT.min(page_height * 0.9);
        let margin_x = MARGIN_PT.min((page_width - width) / 2.0);
        let margin_y = MARGIN_PT.min((page_height - height) / 2.0);
        let x0 = match visible.corner {
            PageCorner::TopLeft | PageCorner::BottomLeft => bounds.x0 + margin_x,
            PageCorner::TopRight | PageCorner::BottomRight => bounds.x1 - margin_x - width,
        };
        let y0 = match visible.corner {
            PageCorner::TopLeft | PageCorner::TopRight => bounds.y0 + margin_y,
            PageCorner::BottomLeft | PageCorner::BottomRight => bounds.y1 - margin_y - height,
        };
        // Page space goes down from the top left corner of the page as it is shown; the form's
        // goes up from its bottom left corner. The form carries the turn only; where it goes is
        // the widget's `/Rect`.
        let at = mupdf::Rect {
            x0,
            y0,
            x1: x0 + width,
            y1: y0 + height,
        }
        .transform(&inverse);
        let mut turn = mupdf::Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 0.0);
        turn.concat(inverse);

        let font = mupdf_fonts_droid::find_by_name(FONT_NAME, false, false)
            .ok_or(EngineError::InvalidEdit("no font for the signature"))?;
        let font = TextBoxFont {
            bytes: font.data,
            face: 0,
            name: FONT_NAME,
        };
        let mut lines = Vec::new();
        for line in &visible.lines {
            let (form, line_width, line_height) =
                line_form(&mut self.doc, &font, line, TEXT_SIZE_PT, BLACK)?;
            let scale = ((width - 2.0 * PADDING_PT) / line_width).min(1.0);
            lines.push((form, scale, line_height));
        }
        // Lines that are too tall together are all made smaller.
        let tall: f32 = lines.iter().map(|(_, scale, high)| scale * high).sum();
        let fit = ((height - 2.0 * PADDING_PT) / tall).min(1.0);

        let mut content = String::from("q 0.3 G 0.75 w 0.375 0.375 ");
        content.push_str(&format!(
            "{:.3} {:.3} re S Q\n",
            width - 0.75,
            height - 0.75
        ));
        let mut resources = String::new();
        let mut top = height - PADDING_PT;
        for (index, (form, scale, line_height)) in lines.iter().enumerate() {
            let scale = scale * fit;
            top -= line_height * scale;
            content.push_str(&format!(
                "q {scale:.5} 0 0 {scale:.5} {PADDING_PT:.3} {top:.3} cm /L{index} Do Q\n"
            ));
            resources.push_str(&format!(" /L{index} {} 0 R", form.as_indirect()?));
        }
        let form = self.doc.new_object_from_str(&format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
             /Matrix [{:.5} {:.5} {:.5} {:.5} 0 0] /Resources << /XObject <<{resources} >> >> >>",
            turn.a, turn.b, turn.c, turn.d,
        ))?;
        let form =
            self.doc
                .add_stream(&Buffer::from_bytes(content.as_bytes())?, Some(&form), false)?;
        Ok(([at.x0, at.y0, at.x1, at.y1], form))
    }
}

/// Finds in `copy`, after the `original` bytes it starts with, the signature dictionary's room
/// (`/Contents`) and its `/ByteRange`, and writes into the latter the numbers that say every byte
/// but the room: as wide as the placeholder, with spaces after it. Each must be there exactly once.
fn fill_byte_range(copy: &mut [u8], original: usize) -> Result<PreparedSignature, EngineError> {
    let missing = || EngineError::InvalidEdit("the room for the signature in the copy");
    let mut room = Vec::with_capacity(2 * SIGNATURE_RESERVE_BYTES as usize + 2);
    room.push(b'<');
    room.resize(2 * SIGNATURE_RESERVE_BYTES as usize + 1, b'0');
    room.push(b'>');
    let start = find_once(copy, original, &room).ok_or_else(missing)?;
    let end = start + room.len();
    let placeholder =
        format!("[0 {BYTE_RANGE_PLACEHOLDER} {BYTE_RANGE_PLACEHOLDER} {BYTE_RANGE_PLACEHOLDER}]");
    let at = find_once(copy, original, placeholder.as_bytes()).ok_or_else(missing)?;
    let byte_range = format!("[0 {start} {end} {}]", copy.len() - end);
    if byte_range.len() > placeholder.len() {
        return Err(missing());
    }
    let filled = format!("{byte_range:<width$}", width = placeholder.len());
    copy[at..at + filled.len()].copy_from_slice(filled.as_bytes());
    Ok(PreparedSignature {
        bytes: copy.len() as u64,
        contents_start: start as u64,
        contents_end: end as u64,
    })
}

/// Where `needle` is in `haystack` from `from` on, if it is there exactly once.
fn find_once(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let first = needle.first()?;
    let mut found = None;
    let mut at = from;
    while let Some(offset) = haystack.get(at..)?.iter().position(|byte| byte == first) {
        let start = at + offset;
        if haystack.get(start..start + needle.len()) == Some(needle) {
            if found.is_some() {
                return None;
            }
            found = Some(start);
            at = start + needle.len();
        } else {
            at = start + 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RenderedPage;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    const TIME: SigningTime = SigningTime {
        year: 2026,
        month: 10,
        day: 22,
        hour: 9,
        minute: 5,
        second: 7,
        utc_offset_minutes: 480,
    };

    fn invisible() -> NewSignature {
        NewSignature {
            signer: "王小明 Test Signer".to_owned(),
            reason: Some("同意本文件".to_owned()),
            time: TIME,
            visible: None,
        }
    }

    fn visible(page_index: u32, corner: PageCorner) -> NewSignature {
        NewSignature {
            visible: Some(VisibleSignature {
                page_index,
                corner,
                lines: vec![
                    "數位簽署：王小明 Test Signer".to_owned(),
                    "日期：2026-10-22 09:05 UTC+08:00".to_owned(),
                    "原因：同意本文件".to_owned(),
                ],
            }),
            ..invisible()
        }
    }

    fn prepare(bytes: &[u8], signature: &NewSignature) -> (Vec<u8>, PreparedSignature) {
        let mut out = Vec::new();
        let prepared = prepare_signature(bytes, signature, &mut out).expect("prepared");
        assert_eq!(prepared.bytes, out.len() as u64);
        (out, prepared)
    }

    /// The four numbers of the copy's last `/ByteRange`, read back from its bytes.
    fn byte_range(copy: &[u8]) -> [u64; 4] {
        let key = b"/ByteRange";
        let at = copy
            .windows(key.len())
            .rposition(|window| window == key)
            .expect("a byte range");
        let rest = &copy[at + key.len()..];
        let open = rest.iter().position(|&b| b == b'[').unwrap();
        let close = rest.iter().position(|&b| b == b']').unwrap();
        let numbers: Vec<u64> = std::str::from_utf8(&rest[open + 1..close])
            .unwrap()
            .split_ascii_whitespace()
            .map(|number| number.parse().unwrap())
            .collect();
        numbers.try_into().expect("four numbers")
    }

    #[test]
    fn the_copy_is_the_file_then_a_field_with_room_for_the_signature_and_a_range_around_it() {
        let original = corpus("benign/single-page.pdf");
        let (copy, prepared) = prepare(&original, &invisible());
        assert!(
            copy.starts_with(&original),
            "appended, the file's bytes unchanged"
        );
        let (start, end) = (
            prepared.contents_start as usize,
            prepared.contents_end as usize,
        );
        assert!(start > original.len());
        assert_eq!(end - start, 2 * SIGNATURE_RESERVE_BYTES as usize + 2);
        assert_eq!(copy[start], b'<');
        assert_eq!(copy[end - 1], b'>');
        assert!(copy[start + 1..end - 1].iter().all(|&b| b == b'0'));
        assert_eq!(
            byte_range(&copy),
            [0, start as u64, end as u64, (copy.len() - end) as u64]
        );

        // Read back as a signature field, with what it says.
        let signed = PdfDocument::from_bytes(&copy).expect("the copy opens");
        let fields = signed.signature_fields().unwrap();
        assert_eq!(fields.fields.len(), 1);
        let field = &fields.fields[0];
        assert_eq!(field.name.as_deref(), Some("Signature1"));
        assert_eq!(
            field.sub_filter.as_deref(),
            Some(&b"adbe.pkcs7.detached"[..])
        );
        assert_eq!(
            field.claimed_time.as_deref(),
            Some("D:20261022090507+08'00'")
        );
        assert_eq!(
            field.byte_range,
            Some([0, start as i32, end as i32, (copy.len() - end) as i32])
        );
        assert_eq!(
            field.contents().map(|contents| contents.len()),
            Some(SIGNATURE_RESERVE_BYTES as usize)
        );
        assert_eq!(signed.page_count().unwrap(), 1);
        // The document itself has no field.
        let document = PdfDocument::from_bytes(&original).unwrap();
        assert!(document.signature_fields().unwrap().fields.is_empty());
    }

    #[test]
    fn an_invisible_signature_draws_nothing() {
        let original = corpus("benign/single-page.pdf");
        let (copy, _) = prepare(&original, &invisible());
        let before = PdfDocument::from_bytes(&original)
            .unwrap()
            .render(0, 0.5, 0)
            .unwrap();
        let after = PdfDocument::from_bytes(&copy)
            .unwrap()
            .render(0, 0.5, 0)
            .unwrap();
        assert_eq!(before.rgba, after.rgba);
    }

    /// The bounds of the dark pixels that the copy has and the original does not.
    fn drawn(before: &RenderedPage, after: &RenderedPage) -> (usize, usize, usize, usize) {
        let width = after.width as usize;
        let mut found = Vec::new();
        for (index, (old, new)) in before
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(after.rgba.as_chunks::<4>().0)
            .enumerate()
        {
            if old != new && new[0] < 128 {
                found.push((index % width, index / width));
            }
        }
        assert!(!found.is_empty(), "something is drawn");
        let xs = found.iter().map(|p| p.0);
        let ys = found.iter().map(|p| p.1);
        (
            xs.clone().min().unwrap(),
            ys.clone().min().unwrap(),
            xs.max().unwrap(),
            ys.max().unwrap(),
        )
    }

    #[test]
    fn a_visible_signature_is_a_box_in_the_corner_of_the_page_as_it_is_shown() {
        // The second page of rotated-page.pdf is turned (`/Rotate 90`).
        for (name, page, corner) in [
            ("benign/single-page.pdf", 0, PageCorner::BottomRight),
            ("benign/single-page.pdf", 0, PageCorner::TopLeft),
            ("benign/rotated-page.pdf", 1, PageCorner::BottomRight),
            ("benign/rotated-page.pdf", 1, PageCorner::TopRight),
        ] {
            let original = corpus(name);
            let document = PdfDocument::from_bytes(&original).unwrap();
            let (copy, _) = prepare(&original, &visible(page, corner));
            let signed = PdfDocument::from_bytes(&copy).unwrap();
            let before = document.render(page, 0.5, 0).unwrap();
            let after = signed.render(page, 0.5, 0).unwrap();
            let (x0, y0, x1, y1) = drawn(&before, &after);
            let (width, height) = (after.width as usize, after.height as usize);
            let right = matches!(corner, PageCorner::TopRight | PageCorner::BottomRight);
            let bottom = matches!(corner, PageCorner::BottomLeft | PageCorner::BottomRight);
            assert_eq!(
                x0 > width / 2,
                right,
                "{name} {corner:?}: {x0}..{x1} of {width}"
            );
            assert_eq!(
                y0 > height / 2,
                bottom,
                "{name} {corner:?}: {y0}..{y1} of {height}"
            );
            // Level as it is seen: wider than tall.
            assert!(x1 - x0 > 2 * (y1 - y0), "{name} {corner:?}");
        }
    }

    #[test]
    fn a_signed_document_gets_another_signature_with_a_name_of_its_own() {
        let original = corpus("benign/signed.pdf");
        let (copy, prepared) = prepare(&original, &invisible());
        assert!(copy.starts_with(&original));
        let fields = PdfDocument::from_bytes(&copy)
            .unwrap()
            .signature_fields()
            .unwrap()
            .fields;
        let names: Vec<_> = fields.iter().map(|field| field.name.clone()).collect();
        assert_eq!(
            names,
            [Some("Signature1".to_owned()), Some("Signature2".to_owned())]
        );
        assert_eq!(
            fields[1].byte_range.map(|range| range[1] as u64),
            Some(prepared.contents_start)
        );
    }

    #[test]
    fn what_cannot_be_signed_is_refused_and_writes_nothing() {
        for (name, signature) in [
            ("benign/signed-docmdp-p1.pdf", invisible()),
            ("benign/encrypted-aes256.pdf", invisible()),
            ("benign/restricted-no-modify.pdf", invisible()),
            ("malformed/truncated.pdf", invisible()),
            ("benign/single-page.pdf", visible(1, PageCorner::BottomLeft)),
            (
                "benign/single-page.pdf",
                NewSignature {
                    signer: "line\nbreak".to_owned(),
                    ..invisible()
                },
            ),
            (
                "benign/single-page.pdf",
                NewSignature {
                    time: SigningTime { month: 13, ..TIME },
                    ..invisible()
                },
            ),
            (
                "benign/single-page.pdf",
                NewSignature {
                    visible: Some(VisibleSignature {
                        page_index: 0,
                        corner: PageCorner::TopLeft,
                        lines: Vec::new(),
                    }),
                    ..invisible()
                },
            ),
        ] {
            let mut out = Vec::new();
            assert!(
                prepare_signature(&corpus(name), &signature, &mut out).is_err(),
                "{name} {signature:?}"
            );
            assert!(out.is_empty(), "{name}");
        }
    }

    #[test]
    fn a_date_says_its_time_zone() {
        assert_eq!(pdf_date(&TIME), "D:20261022090507+08'00'");
        let west = SigningTime {
            utc_offset_minutes: -210,
            ..TIME
        };
        assert_eq!(pdf_date(&west), "D:20261022090507-03'30'");
        let utc = SigningTime {
            utc_offset_minutes: 0,
            ..TIME
        };
        assert_eq!(pdf_date(&utc), "D:20261022090507Z");
    }

    #[test]
    fn a_needle_is_found_only_once_and_only_after_where_the_search_starts() {
        assert_eq!(find_once(b"ab<00>cd", 0, b"<00>"), Some(2));
        assert_eq!(find_once(b"<00><00>", 0, b"<00>"), None);
        assert_eq!(find_once(b"<00>x<00>", 1, b"<00>"), Some(5));
        assert_eq!(find_once(b"<0<00>", 0, b"<00>"), Some(2));
        assert_eq!(find_once(b"abc", 0, b"<00>"), None);
    }
}
