//! Searchable PDF copy with an invisible OCR text layer (B4-08, Issue #416, ADR 0028).
//!
//! Writes a rewritten copy of the document with invisible text (`3 Tr`) overlaid on each scanned
//! page that already has OCR recognition results in the worker. Original pages with text of their
//! own are not touched; the open document and its source file are never modified.
//!
//! - Font: Tesseract's 573-byte `GlyphLessFont` (`pdf_ttf.h`), embedded once per document as a
//!   Type0 / CIDFontType2 / Identity-H font with a 65,536-entry `CIDToGIDMap` (all mapped to GID 1)
//!   and a PDF 32000-1 §9.10.3 compliant `ToUnicode` CMap (256 codes per `bfrange`, at most 100
//!   entries per `beginbfrange` block).
//! - Layout (ADR 0028 hybrid): one font size and one baseline per horizontal line, explicit U+0020
//!   spaces between words, and per-character horizontal positioning via `Tz` (re-synced with `Tm`
//!   only if drift exceeds 0.25 pt). Non-BMP characters (> U+FFFF) are encoded as UTF-16 surrogate
//!   pairs wrapped in `/Span << /ActualText <FEFF...> >> BDC ... EMC`.
//! - Isolation: each modified page's original content streams are enclosed in `[q, ..., Q]` before
//!   appending `BT 3 Tr ... ET`, and coordinates are mapped from MuPDF page space into PDF user
//!   space via [`PdfDocument::page_to_pdf`] (handling `/Rotate` and non-zero MediaBox/CropBox).

use std::fmt::Write as _;
use std::io::Write;

use ipc_contract::limits::MAX_PAGE_TEXT_CHARS;
use ipc_contract::types::{
    PageText as TextLayer, SearchableSkipReason, SearchableSkippedPage, TextLine,
};
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject, PdfWriteOptions};
use mupdf::{Buffer, Matrix};

use super::ocr::SearchablePageSource;
use super::{EngineError, PdfDocument, write_limited};

/// The resource name used for the shared GlyphLessFont on modified pages.
pub(super) const FONT_BASE_NAME: &str = "PdfReaderOcrFont";

/// Minimal TrueType font (`pdf.ttf` / `GlyphLessFont`) from Tesseract (`src/api/pdf_ttf.h`,
/// 573 bytes). Contains a single empty glyph so all Unicode code points can be mapped invisibly
/// without subsetting a full CJK font.
///
/// ///////////////////////////////////////////////////////////////////////
/// // File:        pdf_ttf.h
/// // Description: pdf.ttf (GlyphLessFont) replacement.
/// //              Generated with: "bin2cpp pdf.ttf pdf_ttf cpp17"
/// // Author:      Zdenko Podobny
/// //
/// // (C) Copyright 2020, Google Inc.
/// // Licensed under the Apache License, Version 2.0 (the "License");
/// // you may not use this file except in compliance with the License.
/// // You may obtain a copy of the License at
/// // http://www.apache.org/licenses/LICENSE-2.0
/// // Unless required by applicable law or agreed to in writing, software
/// // distributed under the License is distributed on an "AS IS" BASIS,
/// // WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
/// // See the License for the specific language governing permissions and
/// // limitations under the License.
/// ///////////////////////////////////////////////////////////////////////
pub(crate) const GLYPHLESS_FONT: &[u8] = &[
    0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x80, 0x00, 0x03, 0x00, 0x20, 0x4f, 0x53, 0x2f, 0x32,
    0x56, 0xde, 0xc8, 0x94, 0x00, 0x00, 0x01, 0x28, 0x00, 0x00, 0x00, 0x60, 0x63, 0x6d, 0x61, 0x70,
    0x00, 0x0a, 0x00, 0x34, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x1e, 0x67, 0x6c, 0x79, 0x66,
    0x15, 0x22, 0x41, 0x24, 0x00, 0x00, 0x01, 0xb8, 0x00, 0x00, 0x00, 0x18, 0x68, 0x65, 0x61, 0x64,
    0x0b, 0x78, 0xf1, 0x65, 0x00, 0x00, 0x00, 0xac, 0x00, 0x00, 0x00, 0x36, 0x68, 0x68, 0x65, 0x61,
    0x0c, 0x02, 0x04, 0x02, 0x00, 0x00, 0x00, 0xe4, 0x00, 0x00, 0x00, 0x24, 0x68, 0x6d, 0x74, 0x78,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x88, 0x00, 0x00, 0x00, 0x08, 0x6c, 0x6f, 0x63, 0x61,
    0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x01, 0xb0, 0x00, 0x00, 0x00, 0x06, 0x6d, 0x61, 0x78, 0x70,
    0x00, 0x04, 0x00, 0x05, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x20, 0x6e, 0x61, 0x6d, 0x65,
    0xf2, 0xeb, 0x16, 0xda, 0x00, 0x00, 0x01, 0xd0, 0x00, 0x00, 0x00, 0x4b, 0x70, 0x6f, 0x73, 0x74,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x1c, 0x00, 0x00, 0x00, 0x20, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0xb0, 0x94, 0x71, 0x10, 0x5f, 0x0f, 0x3c, 0xf5, 0x04, 0x07, 0x08, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xcf, 0x9a, 0xfc, 0x6e, 0x00, 0x00, 0x00, 0x00, 0xd4, 0xc3, 0xa7, 0xf2,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x08, 0x00, 0xff, 0xff, 0x00, 0x00, 0x04, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x01, 0x90, 0x00, 0x05,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x47, 0x4f, 0x4f, 0x47, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x03, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x14, 0x00, 0x06, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00,
    0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x31, 0x21, 0x11, 0x21, 0x04, 0x00, 0xfc, 0x00, 0x08, 0x00,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x05, 0x00, 0x16,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x0b, 0x00, 0x16, 0x00, 0x03,
    0x00, 0x01, 0x04, 0x09, 0x00, 0x05, 0x00, 0x16, 0x00, 0x00, 0x00, 0x56, 0x00, 0x65, 0x00, 0x72,
    0x00, 0x73, 0x00, 0x69, 0x00, 0x6f, 0x00, 0x6e, 0x00, 0x20, 0x00, 0x31, 0x00, 0x2e, 0x00, 0x30,
    0x56, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x31, 0x2e, 0x30, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Outcome of [`PdfDocument::searchable_copy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchableCopy {
    /// Bytes written to the output stream.
    pub bytes: u64,
    /// Pages that received an invisible OCR text layer.
    pub added_pages: u32,
    /// Pages that did not receive a text layer, in ascending page order, with reasons.
    pub skipped: Vec<SearchableSkippedPage>,
}

/// Builds the identity `ToUnicode` CMap stream for UTF-16 code units `0x0000..=0xFFFF`.
///
/// PDF 32000-1 §9.10.3 stipulates that incrementing the last byte of a `bfrange` destination string
/// past 255 is undefined, and each `beginbfrange` block may contain at most 100 entries. We emit
/// 256 ranges of 256 codes each (`<XX00> <XXFF> <XX00>`), grouped into blocks of at most 100.
fn build_to_unicode_cmap() -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );
    let high_bytes: Vec<u16> = (0u16..=0xFF).collect();
    for chunk in high_bytes.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfrange", chunk.len());
        for &hi in chunk {
            let start = hi << 8;
            let end = start | 0xFF;
            let _ = writeln!(cmap, "<{start:04X}> <{end:04X}> <{start:04X}>");
        }
        cmap.push_str("endbfrange\n");
    }
    cmap.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n",
    );
    cmap
}

/// Writes the single shared `GlyphLessFont` Type0 font hierarchy into `doc`.
pub(super) fn write_glyphless_font(doc: &mut MuPdfDocument) -> Result<PdfObject, EngineError> {
    let length_dict =
        doc.new_object_from_str(&format!("<< /Length1 {} >>", GLYPHLESS_FONT.len()))?;
    let font_file = doc.add_stream(
        &Buffer::from_bytes(GLYPHLESS_FONT)?,
        Some(&length_dict),
        true,
    )?;

    let descriptor = doc.new_object_from_str(&format!(
        "<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 \
         /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent -1 \
         /CapHeight 1000 /StemV 80 /FontFile2 {} 0 R >>",
        font_file.as_indirect()?
    ))?;
    let descriptor = doc.add_object(&descriptor)?;

    // Map all 65,536 CIDs (0x0000..=0xFFFF) to GID 1 (the empty glyph in GlyphLessFont).
    let mut cid_to_gid = vec![0u8; 65536 * 2];
    for i in 0..65536 {
        cid_to_gid[i * 2 + 1] = 1;
    }
    let map_stream = doc.add_stream(&Buffer::from_bytes(&cid_to_gid)?, None, true)?;

    let cmap = build_to_unicode_cmap();
    let to_unicode = doc.add_stream(&Buffer::from_bytes(cmap.as_bytes())?, None, true)?;

    let descendant = doc.new_object_from_str(&format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {} 0 R /CIDToGIDMap {} 0 R /DW 1000 >>",
        descriptor.as_indirect()?,
        map_stream.as_indirect()?,
    ))?;
    let descendant = doc.add_object(&descendant)?;

    let type0 = doc.new_object_from_str(&format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H \
         /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
        descendant.as_indirect()?,
        to_unicode.as_indirect()?,
    ))?;
    Ok(doc.add_object(&type0)?)
}

/// Returns true when `line` runs horizontally left-to-right (within ±45° of horizontal).
/// Vertical lines (e.g. from `*_vert` traineddata) are skipped per ADR 0028 Decision 2.
fn is_horizontal_line(line: &TextLine) -> bool {
    let dx = line.quad.ur.x - line.quad.ul.x;
    let dy = line.quad.ur.y - line.quad.ul.y;
    dx.is_finite() && dy.is_finite() && dx > dy.abs()
}

/// Builds the invisible text layer content stream (`BT 3 Tr ... ET`) for one page using the
/// ADR 0028 hybrid layout: one font size and one baseline per horizontal line, explicit U+0020
/// spaces between words, and per-character horizontal positioning via `Tz`.
///
/// Returns `None` if no writable horizontal characters exist.
fn build_page_text_stream(text: &TextLayer, inverse: &Matrix, font_name: &str) -> Option<String> {
    let mut out = String::from("BT\n3 Tr\n");
    let mut chars_written: usize = 0;
    let max_chars = MAX_PAGE_TEXT_CHARS as usize;

    for line in &text.lines {
        if chars_written >= max_chars {
            break;
        }
        if !is_horizontal_line(line) {
            continue;
        }
        let char_count = line.text.chars().count();
        if char_count == 0 || line.edges.len() != char_count + 1 {
            continue;
        }

        let q = &line.quad;
        let dx = q.ur.x - q.ul.x;
        let dy = q.ur.y - q.ul.y;
        let line_len = dx.hypot(dy);
        if !(line_len > 1e-4 && line_len.is_finite()) {
            continue;
        }
        let (ux, uy) = (dx / line_len, dy / line_len);

        let up_dx = q.ul.x - q.ll.x;
        let up_dy = q.ul.y - q.ll.y;
        let line_height = up_dx.hypot(up_dy);
        if !(line_height > 1e-4 && line_height.is_finite()) {
            continue;
        }
        let (up_x, up_y) = (up_dx / line_height, up_dy / line_height);
        let fontsize = line_height.max(0.5);

        let mut pdf_mat = Matrix::new(ux, uy, up_x, up_y, q.ll.x, q.ll.y);
        pdf_mat.concat(inverse.clone());
        if ![
            pdf_mat.a, pdf_mat.b, pdf_mat.c, pdf_mat.d, pdf_mat.e, pdf_mat.f,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            continue;
        }

        let mut line_has_chars = false;
        let mut pen = 0.0_f32;
        let mut last_tz: Option<f32> = None;

        for (ch, pair) in line.text.chars().zip(line.edges.windows(2)) {
            if chars_written >= max_chars {
                break;
            }
            if ch.is_control() {
                continue;
            }
            let start = pair[0];
            let end = pair[1];
            if !(start.is_finite() && end.is_finite()) {
                continue;
            }

            if !line_has_chars {
                let ex = pdf_mat.a * start + pdf_mat.e;
                let fy = pdf_mat.b * start + pdf_mat.f;
                let _ = writeln!(
                    out,
                    "{:.5} {:.5} {:.5} {:.5} {:.3} {:.3} Tm /{font_name} {fontsize:.3} Tf",
                    pdf_mat.a, pdf_mat.b, pdf_mat.c, pdf_mat.d, ex, fy
                );
                pen = start;
                line_has_chars = true;
            } else if (start - pen).abs() > 0.25 {
                let ex = pdf_mat.a * start + pdf_mat.e;
                let fy = pdf_mat.b * start + pdf_mat.f;
                let _ = writeln!(
                    out,
                    "{:.5} {:.5} {:.5} {:.5} {:.3} {:.3} Tm",
                    pdf_mat.a, pdf_mat.b, pdf_mat.c, pdf_mat.d, ex, fy
                );
                pen = start;
            }

            let target_w = (end - pen).max(0.05);
            let mut utf16 = [0u16; 2];
            let units = ch.encode_utf16(&mut utf16);
            let raw_tz = (100.0 * target_w / fontsize).clamp(0.01, 100_000.0);
            let tz = (raw_tz * 100.0).round() / 100.0;

            if units.len() == 1 {
                if last_tz.is_none_or(|prev| (prev - tz).abs() >= 0.005) {
                    let _ = write!(out, "{tz:.2} Tz ");
                    last_tz = Some(tz);
                }
                let _ = writeln!(out, "<{:04X}> Tj", units[0]);
                pen += fontsize * (tz / 100.0);
            } else {
                // Non-BMP character (> U+FFFF): write UTF-16 surrogate pair as two CIDs, wrapped
                // in ActualText so both MuPDF and PDFium/Acrobat extract the single supplementary
                // Unicode scalar. The high surrogate carries the full character width so MuPDF's
                // ActualText flush (which uses the first glyph's trm) places the quad accurately,
                // while the low surrogate uses minimal scaling so total pen advance matches `end`.
                if last_tz.is_none_or(|prev| (prev - tz).abs() >= 0.005) {
                    let _ = write!(out, "{tz:.2} Tz ");
                }
                let _ = writeln!(
                    out,
                    "/Span << /ActualText <FEFF{:04X}{:04X}> >> BDC <{:04X}> Tj 0.01 Tz <{:04X}> Tj EMC",
                    units[0], units[1], units[0], units[1]
                );
                last_tz = Some(0.01);
                pen += fontsize * (tz / 100.0) + fontsize * 0.0001;
            }

            chars_written += 1;
        }
    }

    if chars_written == 0 {
        return None;
    }
    out.push_str("ET\n");
    Some(out)
}

/// Finds an unused `/Resources /Font` key on `page` so existing page fonts are never clobbered.
fn unused_font_name(page: &PdfObject) -> Result<String, EngineError> {
    let resources = match page.get_dict("Resources")? {
        Some(resources) => Some(resources),
        None => page.get_dict_inheritable("Resources")?,
    };
    let fonts = match resources {
        Some(res) => res.get_dict("Font")?,
        None => None,
    };
    for n in 1..=100 {
        let name = if n == 1 {
            FONT_BASE_NAME.to_owned()
        } else {
            format!("{FONT_BASE_NAME}{n}")
        };
        let exists = match &fonts {
            Some(dict) => dict.get_dict(name.as_str())?.is_some(),
            None => false,
        };
        if !exists {
            return Ok(name);
        }
    }
    Err(EngineError::InvalidEdit("too many OCR fonts on a page"))
}

/// Attaches the shared font to `page`'s isolated `/Resources /Font` dictionary and wraps the
/// page's existing `/Contents` in `[q, ...original..., Q, text_layer]`.
fn attach_text_layer(
    doc: &mut MuPdfDocument,
    page: &mut PdfObject,
    font_name: &str,
    font_obj: &PdfObject,
    text_stream: &str,
) -> Result<(), EngineError> {
    // Isolate `/Resources` if inherited or shared via an indirect reference.
    let mut resources = match page.get_dict("Resources")? {
        Some(res) if res.is_indirect()? => {
            let copy = res.copy_dict()?;
            page.dict_put("Resources", copy)?;
            page.get_dict("Resources")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
        Some(res) => res,
        None => {
            let inherited = match page.get_dict_inheritable("Resources")? {
                Some(inherited) => inherited.copy_dict()?,
                None => doc.new_dict()?,
            };
            page.dict_put("Resources", inherited)?;
            page.get_dict("Resources")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };

    // Isolate `/Font` if shared via an indirect reference.
    let mut fonts = match resources.get_dict("Font")? {
        Some(fonts) if fonts.is_indirect()? => {
            let copy = fonts.copy_dict()?;
            resources.dict_put("Font", copy)?;
            resources
                .get_dict("Font")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
        Some(fonts) => fonts,
        None => {
            resources.dict_put("Font", doc.new_dict()?)?;
            resources
                .get_dict("Font")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };

    fonts.dict_put(font_name, font_obj.try_clone()?)?;

    let before = doc.add_stream(&Buffer::from_bytes(b"q\n")?, None, false)?;
    let mut after_text = String::with_capacity(text_stream.len() + 3);
    after_text.push_str("\nQ\n");
    after_text.push_str(text_stream);
    let after = doc.add_stream(&Buffer::from_bytes(after_text.as_bytes())?, None, true)?;

    let mut contents = doc.new_array()?;
    contents.array_push(before)?;
    match page.get_dict("Contents")? {
        Some(existing) if existing.is_array()? => {
            for part in existing.array_iter()? {
                contents.array_push(part?)?;
            }
        }
        Some(existing) => contents.array_push(existing)?,
        None => {}
    }
    contents.array_push(after)?;
    page.dict_put("Contents", contents)?;
    Ok(())
}

impl PdfDocument {
    /// Writes to `out` a rewritten copy of the document, edits included, with invisible OCR text
    /// layers on scanned pages that have already been recognised in this worker (B4-08, Issue
    /// #416, ADR 0028).
    ///
    /// The open document is untouched. Encrypted documents and signed documents are refused before
    /// writing anything to `out`.
    pub fn searchable_copy(&self, out: &mut impl Write) -> Result<SearchableCopy, EngineError> {
        if self.is_encrypted() {
            return Err(EngineError::EncryptedCopy);
        }
        if self.is_signed() {
            return Err(EngineError::Signed);
        }

        let page_count = self.page_count()?;
        let mut page_plans: Vec<Result<&TextLayer, SearchableSkipReason>> =
            Vec::with_capacity(page_count as usize);
        for index in 0..page_count {
            let plan = match self.ocr_searchable_source(index)? {
                SearchablePageSource::Text(text) => Ok(text),
                SearchablePageSource::NotScan => Err(SearchableSkipReason::NotScan),
                SearchablePageSource::TimedOut => Err(SearchableSkipReason::TimedOut),
                SearchablePageSource::Failed => Err(SearchableSkipReason::Failed),
                SearchablePageSource::NoText => Err(SearchableSkipReason::NoText),
                SearchablePageSource::Unknown => {
                    if self.is_scan_page(index)? {
                        Err(SearchableSkipReason::Failed)
                    } else {
                        Err(SearchableSkipReason::NotScan)
                    }
                }
            };
            page_plans.push(plan);
        }

        let mut base_options = PdfWriteOptions::default();
        base_options.set_garbage(true);
        let mut bytes = Vec::new();
        write_limited(&self.doc, &mut bytes, base_options)?;
        let mut copy = PdfDocument::from_bytes(&bytes)?;
        drop(bytes);

        let mut shared_font: Option<PdfObject> = None;
        let mut added_pages: u32 = 0;
        let mut skipped: Vec<SearchableSkippedPage> = Vec::new();

        for (i, plan) in page_plans.into_iter().enumerate() {
            let page_index = u32::try_from(i).unwrap_or(u32::MAX);
            match plan {
                Err(reason) => skipped.push(SearchableSkippedPage { page_index, reason }),
                Ok(text) => {
                    let (mut page_obj, inverse) = copy.page_to_pdf(page_index)?;
                    let font_name = unused_font_name(&page_obj)?;
                    match build_page_text_stream(text, &inverse, &font_name) {
                        None => skipped.push(SearchableSkippedPage {
                            page_index,
                            reason: SearchableSkipReason::NoText,
                        }),
                        Some(stream) => {
                            let font = match &shared_font {
                                Some(font) => font,
                                None => {
                                    let created = write_glyphless_font(&mut copy.doc)?;
                                    shared_font = Some(created);
                                    shared_font.as_ref().expect("just set")
                                }
                            };
                            attach_text_layer(
                                &mut copy.doc,
                                &mut page_obj,
                                &font_name,
                                font,
                                &stream,
                            )?;
                            added_pages += 1;
                        }
                    }
                }
            }
        }

        let mut final_options = PdfWriteOptions::default();
        final_options.set_garbage(true);
        final_options.set_compress(true);
        final_options.set_compress_fonts(true);
        let written = write_limited(&copy.doc, out, final_options)?;

        Ok(SearchableCopy {
            bytes: written,
            added_pages,
            skipped,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use ipc_contract::types::{Point, Quad};
    use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument as MuPdfDocument};
    use mupdf::{Colorspace, Document, Image, Matrix, Pixmap, Rect, Size, TextPageFlags};

    use super::*;
    use crate::engine::{contract_quad, ocr::char_quads};
    use crate::ocr::{Recogniser, one_at_a_time};
    use crate::ocr_worker::page_text;
    use crate::text_layer::TextLayerBuilder;

    const DPI: u32 = 200;
    const OCR_TIME: Duration = Duration::from_secs(120);

    fn corpus_bytes(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn rasterize_page_to_scan_pdf(source: &[u8]) -> Vec<u8> {
        let doc = Document::from_bytes(source, "application/pdf").expect("open source");
        let page = doc.load_page(0).expect("page 0");
        let scale = DPI as f32 / 72.0;
        let pixmap = page
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render");
        let bounds = page.bounds().expect("bounds");
        let w = bounds.x1 - bounds.x0;
        let h = bounds.y1 - bounds.y0;

        let mut new_pdf = MuPdfDocument::new();
        let mut new_page = new_pdf
            .new_page(Size {
                width: w,
                height: h,
            })
            .expect("new page");
        let img = Image::from_pixmap(&pixmap).expect("image");
        new_page
            .insert_image(
                &mut new_pdf,
                Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: w,
                    y1: h,
                },
                PageImageSource::Image(&img),
                InsertImageOptions::default(),
            )
            .expect("insert image");
        let mut out = Vec::new();
        let mut opts = PdfWriteOptions::default();
        opts.set_garbage(true);
        opts.set_compress(true);
        write_limited(&new_pdf, &mut out, opts).expect("write");
        out
    }

    fn recognise_page_in_doc(doc: &mut PdfDocument, page_index: u32, language: &str) -> TextLayer {
        let _turn = one_at_a_time();
        assert!(doc.ocr_is_scan(page_index).expect("is_scan"));
        let (picture, geometry, key) = doc.ocr_picture(page_index).expect("ocr picture");
        let trained = crate::ocr::tests::traineddata(language);
        let mut recogniser = Recogniser::new(language, &trained).expect("recogniser");
        let lines = recogniser
            .recognise(
                &picture.grey,
                picture.width,
                picture.height,
                picture.ppi,
                OCR_TIME,
                &AtomicBool::new(false),
            )
            .expect("recognise");
        let text = page_text(&lines, geometry);
        let epoch = doc.ocr_epoch();
        doc.ocr_finish(key, epoch, Some(text.clone()));
        text
    }

    fn render_grey(bytes: &[u8], page_index: i32) -> Vec<u8> {
        let doc = Document::from_bytes(bytes, "application/pdf").expect("open");
        let pix = doc
            .load_page(page_index)
            .expect("page")
            .to_pixmap(
                &Matrix::new_scale(1.0, 1.0),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("pixmap");
        pix.samples().to_vec()
    }

    fn decoded_page_contents(doc: &MuPdfDocument, page_index: i32) -> Vec<u8> {
        let page = doc.find_page(page_index).expect("page");
        let mut bytes = Vec::new();
        match page.get_dict("Contents").expect("contents") {
            Some(c) if c.is_array().expect("is_array") => {
                for part in c.array_iter().expect("iter") {
                    let part = part.expect("part");
                    bytes.extend_from_slice(&part.read_stream().expect("read stream"));
                }
            }
            Some(c) => {
                bytes.extend_from_slice(&c.read_stream().expect("read stream"));
            }
            None => {}
        }
        bytes
    }

    fn extract_chars_with_size(doc: &PdfDocument, page_index: u32) -> Vec<(char, Quad, f32)> {
        let page = doc.load_page(page_index).expect("load page");
        let text_page = page
            .to_text_page(TextPageFlags::empty())
            .expect("to_text_page");
        let mut out = Vec::new();
        for block in text_page.blocks() {
            for line in block.lines() {
                for ch in line.chars() {
                    if let Some(c) = ch.char() {
                        out.push((c, contract_quad(ch.quad()), ch.size()));
                    }
                }
            }
        }
        out
    }

    fn quad_corner_error(a: &Quad, b: &Quad) -> f32 {
        let d_ul = (a.ul.x - b.ul.x).hypot(a.ul.y - b.ul.y);
        let d_ur = (a.ur.x - b.ur.x).hypot(a.ur.y - b.ur.y);
        let d_ll = (a.ll.x - b.ll.x).hypot(a.ll.y - b.ll.y);
        let d_lr = (a.lr.x - b.lr.x).hypot(a.lr.y - b.lr.y);
        (d_ul + d_ur + d_ll + d_lr) / 4.0
    }

    fn measure_position_error(
        expected: &TextLayer,
        reopened: &PdfDocument,
        page: u32,
    ) -> (f32, f32, usize) {
        let extracted = extract_chars_with_size(reopened, page);
        let mut expected_chars = Vec::new();
        for line in &expected.lines {
            for (c, q) in char_quads(line) {
                if c != ' ' {
                    expected_chars.push((c, q));
                }
            }
        }
        let mut total = 0.0_f32;
        let mut max_err = 0.0_f32;
        let mut matched = 0usize;
        let mut idx = 0usize;
        for (exp_c, exp_q) in &expected_chars {
            while idx < extracted.len() && extracted[idx].0 == ' ' {
                idx += 1;
            }
            if idx >= extracted.len() {
                break;
            }
            if extracted[idx].0 == *exp_c {
                let err = quad_corner_error(&extracted[idx].1, exp_q);
                total += err;
                max_err = max_err.max(err);
                matched += 1;
            }
            idx += 1;
        }
        let mean = if matched > 0 {
            total / matched as f32
        } else {
            0.0
        };
        (mean, max_err, matched)
    }

    fn single_line_text(text: &str, x0: f32, y0: f32, char_w: f32, height: f32) -> TextLayer {
        let mut builder = TextLayerBuilder::new(MAX_PAGE_TEXT_CHARS as usize);
        builder.push_line(text.chars().enumerate().map(|(i, c)| {
            let x = x0 + i as f32 * char_w;
            let quad = Quad {
                ul: Point { x, y: y0 },
                ur: Point {
                    x: x + char_w,
                    y: y0,
                },
                ll: Point { x, y: y0 + height },
                lr: Point {
                    x: x + char_w,
                    y: y0 + height,
                },
            };
            (c, quad)
        }));
        let mut layer = builder.finish();
        layer.recognised = true;
        layer
    }

    #[test]
    fn glyphless_font_is_573_bytes() {
        assert_eq!(GLYPHLESS_FONT.len(), 573);
        assert_eq!(&GLYPHLESS_FONT[0..4], &[0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn to_unicode_cmap_has_no_range_over_256_and_covers_all_utf16_units() {
        let cmap = build_to_unicode_cmap();
        let mut in_block = false;
        let mut declared = 0usize;
        let mut seen_in_block = 0usize;
        let mut covered = vec![false; 65536];

        for line in cmap.lines() {
            let line = line.trim();
            if let Some(count_str) = line.strip_suffix(" beginbfrange") {
                assert!(!in_block);
                declared = count_str.parse::<usize>().expect("block count");
                assert!(
                    declared > 0 && declared <= 100,
                    "PDF 32000-1 §9.10.3 allows at most 100 entries per beginbfrange, got {declared}"
                );
                in_block = true;
                seen_in_block = 0;
            } else if line == "endbfrange" {
                assert!(in_block);
                assert_eq!(seen_in_block, declared);
                in_block = false;
            } else if in_block {
                let tokens: Vec<&str> = line.split_whitespace().collect();
                assert_eq!(tokens.len(), 3, "bfrange line: {line}");
                let parse_hex = |tok: &str| -> u32 {
                    let inner = tok
                        .strip_prefix('<')
                        .and_then(|s| s.strip_suffix('>'))
                        .expect("hex");
                    u32::from_str_radix(inner, 16).expect("u32")
                };
                let lo = parse_hex(tokens[0]);
                let hi = parse_hex(tokens[1]);
                let dst = parse_hex(tokens[2]);
                assert!(hi >= lo, "inverted range {lo:04X}..{hi:04X}");
                let span = hi - lo + 1;
                assert!(
                    span <= 256,
                    "range <{lo:04X}> <{hi:04X}> spans {span} codes (> 256)"
                );
                assert!(
                    (dst & 0xFF) + (hi - lo) <= 0xFF,
                    "destination low byte overflows 255 in {line}"
                );
                for code in lo..=hi {
                    assert_eq!(dst + (code - lo), code);
                    covered[code as usize] = true;
                }
                seen_in_block += 1;
            }
        }
        assert!(!in_block);
        assert!(
            covered.iter().all(|&c| c),
            "all 65536 UTF-16 code units must be mapped"
        );
    }

    #[test]
    fn scanned_english_and_chinese_copies_are_searchable_visually_identical_and_accurate() {
        // 1. English scanned corpus file
        let eng_bytes = corpus_bytes("benign/scanned-text.pdf");
        let mut eng_doc = PdfDocument::from_bytes(&eng_bytes).expect("open eng scan");
        let eng_ocr = recognise_page_in_doc(&mut eng_doc, 0, "eng");

        let mut out = Vec::new();
        let report = eng_doc.searchable_copy(&mut out).expect("searchable copy");
        assert_eq!(report.added_pages, 1);
        assert!(report.skipped.is_empty());
        assert_eq!(report.bytes, out.len() as u64);

        // Visual pixels identical
        assert_eq!(render_grey(&eng_bytes, 0), render_grey(&out, 0));

        // Reopen: no longer a scan, search finds phrases with space, uniform font size per line
        let mut reopened = PdfDocument::from_bytes(&out).expect("reopen eng");
        assert!(!reopened.ocr_is_scan(0).expect("not scan after"));
        let page_txt = reopened
            .page_text(0, MAX_PAGE_TEXT_CHARS as usize)
            .expect("text");
        assert!(
            !page_txt.recognised,
            "text now comes from PDF content, not OCR memory"
        );
        let joined: String = page_txt
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("PRIVACY FIRST"), "got: {joined:?}");
        assert!(joined.contains("SECRET PAPER"), "got: {joined:?}");

        let hits = reopened
            .search_page(0, "PRIVACY FIRST", false, 10)
            .expect("search");
        assert!(hits.has_text && !hits.hits.is_empty());

        let (eng_mean, eng_max, eng_matched) = measure_position_error(&eng_ocr, &reopened, 0);
        assert!(eng_matched >= 20, "matched only {eng_matched}");
        assert!(
            eng_mean < 0.25 && eng_max < 0.5,
            "eng position error too high: mean={eng_mean:.3} pt, max={eng_max:.3} pt"
        );

        // Verify uniform font size (one Tf per line in PDF) and uniform height & baseline per line
        let content_bytes = decoded_page_contents(&reopened.doc, 0);
        let content_str = String::from_utf8_lossy(&content_bytes);
        let tf_count = content_str.matches(" Tf").count();
        assert_eq!(
            tf_count,
            eng_ocr.lines.len(),
            "expected exactly one Tf per horizontal line, got {tf_count} for {} lines",
            eng_ocr.lines.len()
        );
        let p = reopened.load_page(0).expect("load");
        let tp = p.to_text_page(TextPageFlags::empty()).expect("tp");
        for block in tp.blocks() {
            for line in block.lines() {
                let metrics: Vec<(f32, f32)> = line
                    .chars()
                    .map(|c| {
                        let q = contract_quad(c.quad());
                        (q.ll.y - q.ul.y, q.ll.y)
                    })
                    .collect();
                if let Some(&(first_h, first_y)) = metrics.first() {
                    assert!(
                        metrics
                            .iter()
                            .all(|&(h, y)| (h - first_h).abs() < 0.1 && (y - first_y).abs() < 0.1),
                        "line height and baseline must be uniform across a line: {metrics:?}"
                    );
                }
            }
        }

        // 2. Chinese + English scanned page
        let zh_scan = rasterize_page_to_scan_pdf(&corpus_bytes("benign/mixed-text-zh-en.pdf"));
        let mut zh_doc = PdfDocument::from_bytes(&zh_scan).expect("open zh scan");
        let zh_ocr = recognise_page_in_doc(&mut zh_doc, 0, "chi_tra");

        let mut zh_out = Vec::new();
        let zh_report = zh_doc.searchable_copy(&mut zh_out).expect("zh searchable");
        assert_eq!(zh_report.added_pages, 1);
        assert_eq!(render_grey(&zh_scan, 0), render_grey(&zh_out, 0));

        let zh_reopened = PdfDocument::from_bytes(&zh_out).expect("reopen zh");
        let zh_hits = zh_reopened
            .search_page(0, "隱私優先", false, 10)
            .expect("search zh");
        assert!(
            zh_hits.has_text && !zh_hits.hits.is_empty(),
            "Chinese search failed"
        );
        let en_hits = zh_reopened
            .search_page(0, "PDF", false, 10)
            .expect("search PDF");
        assert!(
            en_hits.has_text && !en_hits.hits.is_empty(),
            "English search in mixed failed"
        );

        let (zh_mean, zh_max, zh_matched) = measure_position_error(&zh_ocr, &zh_reopened, 0);
        assert!(zh_matched >= 15, "zh matched only {zh_matched}");
        assert!(
            zh_mean < 0.25 && zh_max < 0.5,
            "zh position error too high: mean={zh_mean:.3} pt, max={zh_max:.3} pt"
        );
        eprintln!(
            "searchable_copy accuracy: eng mean={eng_mean:.4} pt max={eng_max:.4} pt (n={eng_matched}); \
             zh mean={zh_mean:.4} pt max={zh_max:.4} pt (n={zh_matched})"
        );
    }

    #[test]
    fn text_pages_are_untouched_and_multiple_scan_pages_share_one_font_object() {
        // Build a 3-page document: page 0 scan, page 1 native text, page 2 scan.
        let mut pdf = MuPdfDocument::new();
        let mut white =
            Pixmap::new_with_w_h(&Colorspace::device_gray(), 40, 40, false).expect("pixmap");
        white.clear_with(255).expect("clear");
        let img = Image::from_pixmap(&white).expect("img");

        for i in 0..3 {
            let mut page = pdf
                .new_page(Size {
                    width: 612.0,
                    height: 792.0,
                })
                .expect("page");
            if i == 1 {
                // Native text page
                let mut page_obj = pdf.find_page(1).expect("page 1");
                let font = pdf
                    .new_object_from_str(
                        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
                    )
                    .expect("font");
                let font = pdf.add_object(&font).expect("add font");
                let res = pdf
                    .new_object_from_str(&format!(
                        "<< /Font << /F1 {} 0 R >> >>",
                        font.as_indirect().unwrap()
                    ))
                    .expect("res");
                page_obj.dict_put("Resources", res).expect("put res");
                let stream = pdf
                    .add_stream(
                        &Buffer::from_bytes(b"BT /F1 18 Tf 72 700 Td (Native Text) Tj ET\n")
                            .unwrap(),
                        None,
                        false,
                    )
                    .expect("stream");
                page_obj.dict_put("Contents", stream).expect("put contents");
            } else {
                page.insert_image(
                    &mut pdf,
                    Rect {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 612.0,
                        y1: 792.0,
                    },
                    PageImageSource::Image(&img),
                    InsertImageOptions::default(),
                )
                .expect("insert image");
            }
        }
        let mut orig_bytes = Vec::new();
        pdf.write_to(&mut orig_bytes).expect("write");

        let mut doc = PdfDocument::from_bytes(&orig_bytes).expect("open");
        let orig_page1_stream = decoded_page_contents(&doc.doc, 1);
        doc.ocr_set_text_for_test(
            0,
            single_line_text("Scan Page One", 72.0, 100.0, 10.0, 18.0),
        );
        doc.ocr_set_text_for_test(
            2,
            single_line_text("Scan Page Three", 72.0, 120.0, 10.0, 18.0),
        );

        let mut out = Vec::new();
        let result = doc.searchable_copy(&mut out).expect("searchable copy");
        assert_eq!(result.added_pages, 2);
        assert_eq!(
            result.skipped,
            vec![SearchableSkippedPage {
                page_index: 1,
                reason: SearchableSkipReason::NotScan,
            }]
        );

        let reopened = PdfDocument::from_bytes(&out).expect("reopen");
        // Page 1 content stream bytes unchanged
        assert_eq!(decoded_page_contents(&reopened.doc, 1), orig_page1_stream);

        // Both scan pages reference the EXACT same Type0 font indirect object
        let mut font_nums = HashSet::new();
        for p in [0, 2] {
            let page_obj = reopened.doc.find_page(p).expect("page");
            let fonts = page_obj
                .get_dict("Resources")
                .unwrap()
                .unwrap()
                .get_dict("Font")
                .unwrap()
                .unwrap();
            let f = fonts.get_dict(FONT_BASE_NAME).unwrap().expect("ocr font");
            font_nums.insert(f.as_indirect().expect("indirect"));
        }
        assert_eq!(
            font_nums.len(),
            1,
            "all pages must share a single GlyphLessFont object, got {font_nums:?}"
        );
    }

    #[test]
    fn handles_rotated_offset_mediabox_cropbox_stray_cm_and_non_bmp_chars() {
        // Construct a single-page PDF with:
        // - Non-zero MediaBox [100 200 712 992]
        // - CropBox [120 220 692 972]
        // - /Rotate 90
        // - Original content leaving a stray unclosed `0.5 0 0 0.5 50 50 cm` transform!
        let mut pdf = MuPdfDocument::new();
        let mut page = pdf
            .new_page(Size {
                width: 612.0,
                height: 792.0,
            })
            .expect("page");
        let mut white =
            Pixmap::new_with_w_h(&Colorspace::device_gray(), 40, 40, false).expect("pixmap");
        white.clear_with(255).expect("clear");
        let img = Image::from_pixmap(&white).expect("img");
        page.insert_image(
            &mut pdf,
            Rect {
                x0: 100.0,
                y0: 200.0,
                x1: 712.0,
                y1: 992.0,
            },
            PageImageSource::Image(&img),
            InsertImageOptions::default(),
        )
        .expect("insert image");
        drop(page);

        let mut page_obj = pdf.find_page(0).expect("page 0");
        let mb = pdf.new_object_from_str("[100 200 712 992]").expect("mb");
        let cb = pdf.new_object_from_str("[120 220 692 972]").expect("cb");
        page_obj.dict_put("MediaBox", mb).expect("put mb");
        page_obj.dict_put("CropBox", cb).expect("put cb");
        page_obj
            .dict_put("Rotate", PdfObject::new_int(90).expect("rotate"))
            .expect("put rotate");

        // Append an unclosed `cm` operator to the original page content stream
        let stray_cm = pdf
            .add_stream(
                &Buffer::from_bytes(b"\n0.5 0 0 0.5 50 50 cm\n").unwrap(),
                None,
                false,
            )
            .expect("stray cm");
        let mut contents = pdf.new_array().expect("arr");
        if let Some(existing) = page_obj.get_dict("Contents").expect("existing") {
            contents.array_push(existing).expect("push");
        }
        contents.array_push(stray_cm).expect("push cm");
        page_obj
            .dict_put("Contents", contents)
            .expect("put contents");

        let mut orig_bytes = Vec::new();
        pdf.write_to(&mut orig_bytes).expect("write");

        let mut doc = PdfDocument::from_bytes(&orig_bytes).expect("open");
        // Include ASCII, space, CJK, and a supplementary-plane (> U+FFFF) CJK Extension B char '𠀀' (U+20000)
        let sample = "Hello 𠀀 世界";
        let expected = single_line_text(sample, 100.0, 150.0, 14.0, 20.0);
        doc.ocr_set_text_for_test(0, expected.clone());

        let mut out = Vec::new();
        let report = doc.searchable_copy(&mut out).expect("searchable copy");
        assert_eq!(report.added_pages, 1);

        let reopened = PdfDocument::from_bytes(&out).expect("reopen");
        let extracted_page = reopened
            .page_text(0, MAX_PAGE_TEXT_CHARS as usize)
            .expect("text");
        let extracted_str: String = extracted_page
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            extracted_str.contains("Hello")
                && extracted_str.contains('𠀀')
                && extracted_str.contains("世界"),
            "missing expected text (including >U+FFFF char): {extracted_str:?}"
        );

        let (mean_err, max_err, matched) = measure_position_error(&expected, &reopened, 0);
        assert_eq!(
            matched,
            sample.chars().filter(|&c| c != ' ').count(),
            "all non-space chars (including U+20000) must match"
        );
        assert!(
            mean_err < 0.25 && max_err < 0.5,
            "rotated + offset MediaBox/CropBox + stray cm position error too high: mean={mean_err:.3}, max={max_err:.3}"
        );
    }

    #[test]
    fn vertical_lines_are_skipped_and_skip_reasons_are_reported() {
        let mut pdf = MuPdfDocument::new();
        let mut white =
            Pixmap::new_with_w_h(&Colorspace::device_gray(), 40, 40, false).expect("pixmap");
        white.clear_with(255).expect("clear");
        let img = Image::from_pixmap(&white).expect("img");
        for _ in 0..4 {
            let mut p = pdf
                .new_page(Size {
                    width: 612.0,
                    height: 792.0,
                })
                .expect("page");
            p.insert_image(
                &mut pdf,
                Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 612.0,
                    y1: 792.0,
                },
                PageImageSource::Image(&img),
                InsertImageOptions::default(),
            )
            .expect("img");
        }
        let mut bytes = Vec::new();
        pdf.write_to(&mut bytes).expect("write");
        let mut doc = PdfDocument::from_bytes(&bytes).expect("open");

        // Page 0: vertical line only -> NoText
        let mut vert = TextLayerBuilder::new(100);
        vert.push_line("直書字".chars().enumerate().map(|(i, c)| {
            let y = 50.0 + i as f32 * 14.0;
            (
                c,
                Quad {
                    ul: Point { x: 200.0, y },
                    ur: Point { x: 214.0, y },
                    ll: Point {
                        x: 200.0,
                        y: y + 14.0,
                    },
                    lr: Point {
                        x: 214.0,
                        y: y + 14.0,
                    },
                },
            )
        }));
        doc.ocr_set_text_for_test(0, vert.finish());

        // Page 1: empty OCR result -> NoText
        doc.ocr_set_text_for_test(
            1,
            TextLayer {
                lines: Vec::new(),
                truncated: false,
                recognised: true,
            },
        );
        // Page 2: timed out -> TimedOut
        doc.ocr_set_failed_for_test(2, true);
        // Page 3: failed -> Failed
        doc.ocr_set_failed_for_test(3, false);

        let mut out = Vec::new();
        let report = doc.searchable_copy(&mut out).expect("searchable copy");
        assert_eq!(report.added_pages, 0);
        assert_eq!(
            report.skipped,
            vec![
                SearchableSkippedPage {
                    page_index: 0,
                    reason: SearchableSkipReason::NoText,
                },
                SearchableSkippedPage {
                    page_index: 1,
                    reason: SearchableSkipReason::NoText,
                },
                SearchableSkippedPage {
                    page_index: 2,
                    reason: SearchableSkipReason::TimedOut,
                },
                SearchableSkippedPage {
                    page_index: 3,
                    reason: SearchableSkipReason::Failed,
                },
            ]
        );
    }

    #[test]
    fn encrypted_and_signed_documents_are_refused_without_writing_anything() {
        let encrypted =
            PdfDocument::open(&corpus_bytes("benign/encrypted-aes256.pdf"), Some("user"))
                .expect("open encrypted");
        let mut out = Vec::new();
        assert!(matches!(
            encrypted.searchable_copy(&mut out),
            Err(EngineError::EncryptedCopy)
        ));
        assert!(out.is_empty(), "must write nothing for encrypted document");

        let signed = PdfDocument::from_bytes(&corpus_bytes("benign/signed.pdf")).expect("signed");
        let mut out = Vec::new();
        assert!(matches!(
            signed.searchable_copy(&mut out),
            Err(EngineError::Signed)
        ));
        assert!(out.is_empty(), "must write nothing for signed document");
    }

    #[test]
    fn sensitive_document_identifier_is_preserved_in_searchable_copy() {
        let orig = corpus_bytes("benign/scanned-text.pdf");
        let id = [0x42u8; 16];
        let mut marked = Vec::new();
        crate::engine::write_identifier(&orig, None, Some(&id), &mut marked).expect("mark");
        let mut doc = PdfDocument::from_bytes(&marked).expect("open marked");
        assert_eq!(doc.identifier(), Some(id));
        doc.ocr_set_text_for_test(0, single_line_text("SENSITIVE", 72.0, 100.0, 10.0, 18.0));

        let mut copy_bytes = Vec::new();
        doc.searchable_copy(&mut copy_bytes)
            .expect("searchable copy");
        let copy = PdfDocument::from_bytes(&copy_bytes).expect("reopen copy");
        assert_eq!(
            copy.identifier(),
            Some(id),
            "sensitive document identifier must survive in searchable copy"
        );
    }
}
