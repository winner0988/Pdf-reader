//! POC for searchable PDF invisible text layer (B4-08, Issue #416).
//!
//! Investigates embedding invisible text (text rendering mode 3 `3 Tr`) over scanned pages
//! without modifying existing page visuals, evaluating:
//! 1. Font strategies: GlyphLessFont (Identity-H Type0 / CIDFontType2) vs Droid Sans Fallback subset.
//! 2. Coordinate accuracy: Bounding box error (mean and max pt) between Tesseract OCR and MuPDF text extraction.
//! 3. File size delta: KB added per page.
//! 4. Time: Generation duration in milliseconds (excluding OCR).
//! 5. Visual invariance: Pixel-by-pixel rendering comparison.
//! 6. Existing text page detection: `ocr_is_scan`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject, PdfWriteOptions};
use mupdf::{Buffer, Rect};

use super::page_overlay::droid_sans;
use super::text_box::line_font;
use super::{EngineError, PdfDocument, write_limited};
use crate::ocr::{OcrChar, OcrLine};
use crate::ocr_worker::Geometry;

/// The 573-byte minimal TrueType font from Tesseract (`pdf_ttf.h`).
/// (C) Copyright 2020, Google Inc.
/// Licensed under the Apache License, Version 2.0 (Apache-2.0).
pub const GLYPHLESS_FONT: &[u8] = &[
    0x0, 0x1, 0x0, 0x0, 0x0, 0xa, 0x0, 0x80, 0x0, 0x3, 0x0, 0x20, 0x4f, 0x53, 0x2f, 0x32, 0x56,
    0xde, 0xc8, 0x94, 0x0, 0x0, 0x1, 0x28, 0x0, 0x0, 0x0, 0x60, 0x63, 0x6d, 0x61, 0x70, 0x0, 0xa,
    0x0, 0x34, 0x0, 0x0, 0x1, 0x90, 0x0, 0x0, 0x0, 0x1e, 0x67, 0x6c, 0x79, 0x66, 0x15, 0x22, 0x41,
    0x24, 0x0, 0x0, 0x1, 0xb8, 0x0, 0x0, 0x0, 0x18, 0x68, 0x65, 0x61, 0x64, 0xb, 0x78, 0xf1, 0x65,
    0x0, 0x0, 0x0, 0xac, 0x0, 0x0, 0x0, 0x36, 0x68, 0x68, 0x65, 0x61, 0xc, 0x2, 0x4, 0x2, 0x0, 0x0,
    0x0, 0xe4, 0x0, 0x0, 0x0, 0x24, 0x68, 0x6d, 0x74, 0x78, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1,
    0x88, 0x0, 0x0, 0x0, 0x8, 0x6c, 0x6f, 0x63, 0x61, 0x0, 0xc, 0x0, 0x0, 0x0, 0x0, 0x1, 0xb0, 0x0,
    0x0, 0x0, 0x6, 0x6d, 0x61, 0x78, 0x70, 0x0, 0x4, 0x0, 0x5, 0x0, 0x0, 0x1, 0x8, 0x0, 0x0, 0x0,
    0x20, 0x6e, 0x61, 0x6d, 0x65, 0xf2, 0xeb, 0x16, 0xda, 0x0, 0x0, 0x1, 0xd0, 0x0, 0x0, 0x0, 0x4b,
    0x70, 0x6f, 0x73, 0x74, 0x0, 0x1, 0x0, 0x1, 0x0, 0x0, 0x2, 0x1c, 0x0, 0x0, 0x0, 0x20, 0x0, 0x1,
    0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xb0, 0x94, 0x71, 0x10, 0x5f, 0xf, 0x3c, 0xf5, 0x4, 0x7, 0x8,
    0x0, 0x0, 0x0, 0x0, 0x0, 0xcf, 0x9a, 0xfc, 0x6e, 0x0, 0x0, 0x0, 0x0, 0xd4, 0xc3, 0xa7, 0xf2,
    0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x8, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x1, 0x0, 0x0, 0x8, 0x0, 0xff, 0xff, 0x0, 0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x4,
    0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2,
    0x0, 0x1, 0x0, 0x0, 0x0, 0x2, 0x0, 0x4, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x3, 0x0, 0x0, 0x1, 0x90,
    0x0, 0x5, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x5, 0x0, 0x1, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x47, 0x4f, 0x4f, 0x47,
    0x0, 0x40, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0xff, 0xff, 0x0, 0x0, 0x0, 0x1, 0x0, 0x1, 0x80, 0x0,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x14, 0x0, 0x3,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x14, 0x0, 0x6, 0x0, 0xa, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x0, 0xc, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x8, 0x0, 0x0, 0x3,
    0x0, 0x0, 0x31, 0x21, 0x11, 0x21, 0x4, 0x0, 0xfc, 0x0, 0x8, 0x0, 0x0, 0x0, 0x0, 0x3, 0x0, 0x2a,
    0x0, 0x0, 0x0, 0x3, 0x0, 0x0, 0x0, 0x5, 0x0, 0x16, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
    0x5, 0x0, 0xb, 0x0, 0x16, 0x0, 0x3, 0x0, 0x1, 0x4, 0x9, 0x0, 0x5, 0x0, 0x16, 0x0, 0x0, 0x0,
    0x56, 0x0, 0x65, 0x0, 0x72, 0x0, 0x73, 0x0, 0x69, 0x0, 0x6f, 0x0, 0x6e, 0x0, 0x20, 0x0, 0x31,
    0x0, 0x2e, 0x0, 0x30, 0x56, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x31, 0x2e, 0x30, 0x0,
    0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrTextLayerMethod {
    /// Method A1: GlyphLessFont with word-level Tz (Tesseract style)
    GlyphlessWord,
    /// Method A2: GlyphLessFont with character-level exact positioning
    GlyphlessChar,
    /// Method B: Droid Sans Fallback subset (text_box style)
    DroidSans,
}

/// Statistics and measurements for a POC test run.
#[derive(Debug, Clone)]
pub struct OcrPocResult {
    pub method: OcrTextLayerMethod,
    pub original_size_bytes: usize,
    pub output_size_bytes: usize,
    pub delta_kb: f64,
    pub build_time_millis: f64,
    pub matched_chars: usize,
    pub mean_error_pt: f32,
    pub max_error_pt: f32,
    pub visual_identical: bool,
    pub recognized_text: String,
}

/// Writes the GlyphLessFont Type0 font and its descendant CIDFontType2 into `doc`.
/// Returns the Type0 font object.
pub fn write_glyphless_font(doc: &mut MuPdfDocument) -> Result<PdfObject, EngineError> {
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

    // CIDToGIDMap: 65,536 16-bit big-endian words, all 0x0001 (mapping every CID to GID 1).
    let mut map = vec![0u8; 131072];
    for i in 0..65536 {
        map[i * 2 + 1] = 1;
    }
    let map_stream = doc.add_stream(&Buffer::from_bytes(&map)?, None, true)?;

    let cmap = "/CIDInit /ProcSet findresource begin\n\
                12 dict begin\n\
                begincmap\n\
                /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
                /CMapName /Adobe-Identity-UCS def\n\
                /CMapType 2 def\n\
                1 begincodespacerange\n\
                <0000> <FFFF>\n\
                endcodespacerange\n\
                1 beginbfrange\n\
                <0000> <FFFF> <0000>\n\
                endbfrange\n\
                endcmap\n\
                CMapName currentdict /CMap defineresource pop\n\
                end\n\
                end\n";
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

/// Attaches font and invisible text layer to `copy_doc` for page `page_index`.
pub fn apply_text_layer(
    copy_doc: &mut MuPdfDocument,
    page_index: u32,
    lines: &[OcrLine],
    geometry: Geometry,
    method: OcrTextLayerMethod,
) -> Result<(), EngineError> {
    let page_count = copy_doc.page_count()?;
    if page_index >= page_count as u32 {
        return Err(EngineError::PageOutOfRange(page_index));
    }
    let page = copy_doc.load_page(page_index as i32)?;
    let bounds = page.bounds()?;
    let scale = geometry.pixels_per_point;

    let (font_name, font_obj, content) = match method {
        OcrTextLayerMethod::GlyphlessChar => {
            let font_obj = write_glyphless_font(copy_doc)?;
            let mut stream = String::from("q\nBT\n3 Tr\n");
            for line in lines {
                for c in line {
                    if c.ch == ' ' || c.ch.is_control() {
                        continue;
                    }
                    let w = (c.right - c.left) as f32 / scale;
                    let h = (c.bottom - c.top) as f32 / scale;
                    let x = geometry.x0 + c.left as f32 / scale;
                    let y_bottom = bounds.y1 - (c.bottom as f32 / scale);
                    let fontsize = h.max(0.5);
                    let tz = 100.0 * w.max(0.1) / fontsize;
                    let code = c.ch as u32;
                    if code <= 0xFFFF {
                        stream.push_str(&format!(
                            "1 0 0 1 {x:.3} {y_bottom:.3} Tm /F_GLYPH {fontsize:.3} Tf {tz:.2} Tz <{code:04X}> Tj\n"
                        ));
                    }
                }
            }
            stream.push_str("ET\nQ\n");
            ("F_GLYPH", font_obj, stream)
        }
        OcrTextLayerMethod::GlyphlessWord => {
            let font_obj = write_glyphless_font(copy_doc)?;
            let mut stream = String::from("q\nBT\n3 Tr\n");
            for line in lines {
                let mut word: Vec<&OcrChar> = Vec::new();
                for c in line {
                    if c.ch == ' ' || c.ch.is_control() {
                        if !word.is_empty() {
                            emit_glyphless_word(&mut stream, &word, &bounds, scale, geometry)?;
                            word.clear();
                        }
                    } else {
                        word.push(c);
                    }
                }
                if !word.is_empty() {
                    emit_glyphless_word(&mut stream, &word, &bounds, scale, geometry)?;
                }
            }
            stream.push_str("ET\nQ\n");
            ("F_GLYPH", font_obj, stream)
        }
        OcrTextLayerMethod::DroidSans => {
            let mut chars = String::new();
            for line in lines {
                for c in line {
                    if c.ch != ' ' && !c.ch.is_control() && !chars.contains(c.ch) {
                        chars.push(c.ch);
                    }
                }
            }
            if chars.is_empty() {
                chars.push(' ');
            }
            let font = droid_sans()?;
            let lfont = line_font(copy_doc, &font, &chars)?;
            let font_obj = copy_doc.new_indirect(lfont.object_number, 0)?;

            let char_map: BTreeMap<char, (u16, u16)> = lfont
                .model
                .glyphs
                .iter()
                .map(|g| (g.ch, (g.cid, g.advance)))
                .collect();

            let descent = lfont.model.descent as f32;
            let mut stream = String::from("q\nBT\n3 Tr\n");
            for line in lines {
                let mut word: Vec<&OcrChar> = Vec::new();
                for c in line {
                    if c.ch == ' ' || c.ch.is_control() {
                        if !word.is_empty() {
                            emit_droid_word(
                                &mut stream,
                                &word,
                                &bounds,
                                scale,
                                geometry,
                                &char_map,
                                descent,
                            )?;
                            word.clear();
                        }
                    } else {
                        word.push(c);
                    }
                }
                if !word.is_empty() {
                    emit_droid_word(
                        &mut stream,
                        &word,
                        &bounds,
                        scale,
                        geometry,
                        &char_map,
                        descent,
                    )?;
                }
            }
            stream.push_str("ET\nQ\n");
            ("F_DROID", font_obj, stream)
        }
    };

    let mut page_obj = copy_doc.find_page(page_index as i32)?;
    let mut resources = match page_obj.get_dict("Resources")? {
        Some(resources) => resources,
        None => {
            let inherited = match page_obj.get_dict_inheritable("Resources")? {
                Some(inherited) => inherited.copy_dict()?,
                None => copy_doc.new_dict()?,
            };
            page_obj.dict_put("Resources", inherited)?;
            page_obj
                .get_dict("Resources")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };
    let mut fonts = match resources.get_dict("Font")? {
        Some(fonts) => fonts,
        None => {
            let dict = copy_doc.new_dict()?;
            resources.dict_put("Font", dict)?;
            resources
                .get_dict("Font")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };
    fonts.dict_put(font_name, font_obj)?;

    let text_stream = copy_doc.add_stream(&Buffer::from_bytes(content.as_bytes())?, None, true)?;
    let mut contents = copy_doc.new_array()?;
    match page_obj.get_dict("Contents")? {
        Some(existing) if existing.is_array()? => {
            for part in existing.array_iter()? {
                contents.array_push(part?)?;
            }
        }
        Some(existing) => contents.array_push(existing)?,
        None => {}
    }
    contents.array_push(text_stream)?;
    page_obj.dict_put("Contents", contents)?;

    Ok(())
}

fn emit_glyphless_word(
    stream: &mut String,
    word: &[&OcrChar],
    bounds: &Rect,
    scale: f32,
    geometry: Geometry,
) -> Result<(), EngineError> {
    if word.is_empty() {
        return Ok(());
    }
    let first = word[0];
    let last = word[word.len() - 1];
    let min_top = word.iter().map(|c| c.top).min().unwrap_or(first.top);
    let max_bottom = word.iter().map(|c| c.bottom).max().unwrap_or(last.bottom);

    let word_x = geometry.x0 + first.left as f32 / scale;
    let word_y_bottom = bounds.y1 - (max_bottom as f32 / scale);
    let word_w = (last.right - first.left) as f32 / scale;
    let word_h = (max_bottom - min_top) as f32 / scale;
    let fontsize = word_h.max(0.5);
    let char_count = word.len() as f32;
    let tz = 100.0 * word_w.max(0.1) / (fontsize * char_count);

    let mut hex = String::new();
    for c in word {
        let code = c.ch as u32;
        if code <= 0xFFFF {
            hex.push_str(&format!("{code:04X}"));
        }
    }
    stream.push_str(&format!(
        "1 0 0 1 {word_x:.3} {word_y_bottom:.3} Tm /F_GLYPH {fontsize:.3} Tf {tz:.2} Tz <{hex}> Tj\n"
    ));
    Ok(())
}

fn emit_droid_word(
    stream: &mut String,
    word: &[&OcrChar],
    bounds: &Rect,
    scale: f32,
    geometry: Geometry,
    char_map: &BTreeMap<char, (u16, u16)>,
    descent: f32,
) -> Result<(), EngineError> {
    if word.is_empty() {
        return Ok(());
    }
    let first = word[0];
    let last = word[word.len() - 1];
    let min_top = word.iter().map(|c| c.top).min().unwrap_or(first.top);
    let max_bottom = word.iter().map(|c| c.bottom).max().unwrap_or(last.bottom);

    let word_x = geometry.x0 + first.left as f32 / scale;
    let word_y_bottom = bounds.y1 - (max_bottom as f32 / scale);
    let word_w = (last.right - first.left) as f32 / scale;
    let word_h = (max_bottom - min_top) as f32 / scale;
    let fontsize = word_h.max(0.5);

    let mut natural_w = 0.0_f32;
    let mut hex = String::new();
    for c in word {
        if let Some(&(cid, advance)) = char_map.get(&c.ch) {
            natural_w += (advance as f32 / 1000.0) * fontsize;
            hex.push_str(&format!("{cid:04X}"));
        }
    }
    let tz = if natural_w > 0.0 {
        100.0 * word_w.max(0.1) / natural_w
    } else {
        100.0
    };
    let baseline_y = word_y_bottom + (-descent / 1000.0) * fontsize;
    stream.push_str(&format!(
        "1 0 0 1 {word_x:.3} {baseline_y:.3} Tm /F_DROID {fontsize:.3} Tf {tz:.2} Tz <{hex}> Tj\n"
    ));
    Ok(())
}

/// Creates a searchable PDF copy from `original_bytes` with an invisible text layer on `page_index`.
pub fn make_searchable_pdf(
    original_bytes: &[u8],
    page_index: u32,
    lines: &[OcrLine],
    geometry: Geometry,
    method: OcrTextLayerMethod,
) -> Result<Vec<u8>, EngineError> {
    let mut copy = PdfDocument::from_bytes(original_bytes)?;
    apply_text_layer(&mut copy.doc, page_index, lines, geometry, method)?;

    let mut options = PdfWriteOptions::default();
    options.set_garbage(true);
    options.set_compress(true);
    options.set_compress_fonts(true);

    let mut out = Vec::new();
    write_limited(&copy.doc, &mut out, options)?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use ipc_contract::types::Quad;
    use mupdf::pdf::{InsertImageOptions, PageImageSource};
    use mupdf::{Colorspace, Document, Image, Matrix, Size, TextPageFlags};

    use super::*;
    use crate::engine::contract_quad;
    use crate::ocr::{Recogniser, one_at_a_time};

    const DPI: u32 = 200;
    const TIME: Duration = Duration::from_secs(120);

    /// Reads corpus file.
    pub fn corpus_bytes(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// Creates an in-memory scanned Chinese+English PDF by rasterizing `benign/mixed-text-zh-en.pdf`
    /// page 0 into an image-only PDF.
    pub fn create_scanned_chinese_pdf() -> Vec<u8> {
        let source_pdf = corpus_bytes("benign/mixed-text-zh-en.pdf");
        let doc = Document::from_bytes(&source_pdf, "application/pdf").expect("open mixed");
        let page = doc.load_page(0).expect("page 0");
        let scale = DPI as f32 / 72.0;
        let pixmap = page
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render to grey");
        let bounds = page.bounds().expect("bounds");
        let width_pt = bounds.x1 - bounds.x0;
        let height_pt = bounds.y1 - bounds.y0;

        let mut new_pdf = MuPdfDocument::new();
        let mut new_page = new_pdf
            .new_page(Size {
                width: width_pt,
                height: height_pt,
            })
            .expect("new page");
        let page_image = Image::from_pixmap(&pixmap).expect("image from pixmap");
        new_page
            .insert_image(
                &mut new_pdf,
                Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: width_pt,
                    y1: height_pt,
                },
                PageImageSource::Image(&page_image),
                InsertImageOptions::default(),
            )
            .expect("insert image");

        let mut out = Vec::new();
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        options.set_compress(true);
        write_limited(&new_pdf, &mut out, options).expect("write synthetic pdf");
        out
    }

    /// Evaluates accuracy, timing, size, and visual invariance for a given input PDF and method.
    pub fn evaluate_poc(
        original_bytes: &[u8],
        language: &str,
        method: OcrTextLayerMethod,
    ) -> OcrPocResult {
        let _turn = one_at_a_time();
        let original_bytes_snapshot = original_bytes.to_vec();
        let mut doc = PdfDocument::from_bytes(original_bytes).expect("open original");

        // 1. Check scan status before
        assert!(doc.ocr_is_scan(0).expect("is_scan before"));

        // 2. Render picture for OCR
        let (picture, geometry, _) = doc.ocr_picture(0).expect("ocr picture");

        // 3. Run OCR
        let trained = crate::ocr::tests::traineddata(language);
        let mut recogniser = Recogniser::new(language, &trained).expect("recogniser");
        let lines = recogniser
            .recognise(
                &picture.grey,
                picture.width,
                picture.height,
                picture.ppi,
                TIME,
                &AtomicBool::new(false),
            )
            .expect("recognise");

        // 4. Measure time to add invisible text layer
        let start = Instant::now();
        let searchable_bytes =
            make_searchable_pdf(original_bytes, 0, &lines, geometry, method).expect("searchable");
        let duration = start.elapsed();
        let duration_millis = duration.as_secs_f64() * 1000.0;

        // 5. Verify original bytes were not touched
        assert!(!searchable_bytes.is_empty());
        assert_eq!(original_bytes, original_bytes_snapshot.as_slice());

        // 6. Check scan status after adding text layer
        let mut doc_after = PdfDocument::from_bytes(&searchable_bytes).expect("reopen searchable");
        assert!(
            !doc_after.ocr_is_scan(0).expect("is_scan after"),
            "Page with text layer should not be classified as a scan"
        );

        // 7. Verify visual invariance: rasterize both and compare pixel bytes
        let orig_doc = Document::from_bytes(original_bytes, "application/pdf").expect("open orig");
        let copy_doc =
            Document::from_bytes(&searchable_bytes, "application/pdf").expect("open copy");
        let orig_pixmap = orig_doc
            .load_page(0)
            .expect("orig page")
            .to_pixmap(
                &Matrix::new_scale(1.0, 1.0),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render orig");
        let copy_pixmap = copy_doc
            .load_page(0)
            .expect("copy page")
            .to_pixmap(
                &Matrix::new_scale(1.0, 1.0),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render copy");
        let visual_identical = orig_pixmap.samples() == copy_pixmap.samples();
        if !visual_identical {
            println!(
                "orig: {}x{}, len={}; copy: {}x{}, len={}",
                orig_pixmap.width(),
                orig_pixmap.height(),
                orig_pixmap.samples().len(),
                copy_pixmap.width(),
                copy_pixmap.height(),
                copy_pixmap.samples().len(),
            );
            let mut diffs = 0;
            for y in 0..orig_pixmap.height() {
                for x in 0..orig_pixmap.width() {
                    let idx = (y * orig_pixmap.width() + x) as usize;
                    let p1 = orig_pixmap.samples()[idx];
                    let p2 = copy_pixmap.samples()[idx];
                    if p1 != p2 {
                        diffs += 1;
                        if diffs <= 10 {
                            println!("Diff at ({x}, {y}): orig={p1}, copy={p2}");
                        }
                    }
                }
            }
            println!("different bytes: {}", diffs);
        }
        assert!(
            visual_identical,
            "Rasterized pixels of the copy must be 100% identical to original"
        );

        // 8. Extract text and calculate bounding box error
        let page_after = doc_after.load_page(0).expect("load page");
        let text_page = page_after
            .to_text_page(TextPageFlags::empty())
            .expect("to_text_page");

        let mut extracted: Vec<(char, Quad)> = Vec::new();
        for block in text_page.blocks() {
            for line in block.lines() {
                for ch in line.chars() {
                    if let Some(c) = ch.char() {
                        extracted.push((c, contract_quad(ch.quad())));
                    }
                }
            }
        }

        let mut expected: Vec<(char, Quad)> = Vec::new();
        let mut text_str = String::new();
        for line in &lines {
            for c in line {
                if c.ch != ' ' && !c.ch.is_control() {
                    expected.push((c.ch, geometry.quad(c)));
                    text_str.push(c.ch);
                }
            }
            text_str.push(' ');
        }

        let mut total_err = 0.0_f32;
        let mut max_err = 0.0_f32;
        let mut matched = 0;
        let mut ext_idx = 0;

        for (exp_c, exp_q) in &expected {
            while ext_idx < extracted.len() && extracted[ext_idx].0 == ' ' {
                ext_idx += 1;
            }
            if ext_idx >= extracted.len() {
                break;
            }
            let (ext_c, ext_q) = &extracted[ext_idx];
            if *exp_c == *ext_c {
                let d_ul = (ext_q.ul.x - exp_q.ul.x).hypot(ext_q.ul.y - exp_q.ul.y);
                let d_ur = (ext_q.ur.x - exp_q.ur.x).hypot(ext_q.ur.y - exp_q.ur.y);
                let d_ll = (ext_q.ll.x - exp_q.ll.x).hypot(ext_q.ll.y - exp_q.ll.y);
                let d_lr = (ext_q.lr.x - exp_q.lr.x).hypot(ext_q.lr.y - exp_q.lr.y);
                let err = (d_ul + d_ur + d_ll + d_lr) / 4.0;
                total_err += err;
                if err > max_err {
                    max_err = err;
                }
                matched += 1;
                ext_idx += 1;
            } else {
                ext_idx += 1;
            }
        }
        let mean_error_pt = if matched > 0 {
            total_err / matched as f32
        } else {
            0.0
        };

        // File size delta
        let original_size = original_bytes.len();
        let output_size = searchable_bytes.len();
        let delta_kb = (output_size as f64 - original_size as f64) / 1024.0;

        // Optionally write output file for manual reader inspection (Edge)
        if let Ok(out_dir) = std::env::var("POC_416_OUT") {
            let dir = PathBuf::from(out_dir);
            let _ = std::fs::create_dir_all(&dir);
            let lang_tag = if language.starts_with("chi") {
                "mixed-zh-en"
            } else {
                "scanned-text"
            };
            let method_tag = match method {
                OcrTextLayerMethod::GlyphlessWord => "glyphless-word",
                OcrTextLayerMethod::GlyphlessChar => "glyphless-char",
                OcrTextLayerMethod::DroidSans => "droid-sans",
            };
            let out_path = dir.join(format!("{lang_tag}-{method_tag}.pdf"));
            let _ = std::fs::write(&out_path, &searchable_bytes);
        }

        OcrPocResult {
            method,
            original_size_bytes: original_size,
            output_size_bytes: output_size,
            delta_kb,
            build_time_millis: duration_millis,
            matched_chars: matched,
            mean_error_pt,
            max_error_pt: max_err,
            visual_identical,
            recognized_text: text_str.trim().to_string(),
        }
    }

    #[test]
    fn run_poc_evaluations_and_measure_all_metrics() {
        let eng_pdf = corpus_bytes("benign/scanned-text.pdf");
        let zh_pdf = create_scanned_chinese_pdf();

        println!("=== POC 416 RESULTS START ===");

        // English evaluations
        let eng_gw = evaluate_poc(&eng_pdf, "eng", OcrTextLayerMethod::GlyphlessWord);
        let eng_gc = evaluate_poc(&eng_pdf, "eng", OcrTextLayerMethod::GlyphlessChar);
        let eng_ds = evaluate_poc(&eng_pdf, "eng", OcrTextLayerMethod::DroidSans);

        // Chinese evaluations
        let zh_gw = evaluate_poc(&zh_pdf, "chi_tra", OcrTextLayerMethod::GlyphlessWord);
        let zh_gc = evaluate_poc(&zh_pdf, "chi_tra", OcrTextLayerMethod::GlyphlessChar);
        let zh_ds = evaluate_poc(&zh_pdf, "chi_tra", OcrTextLayerMethod::DroidSans);

        println!("--- English (scanned-text.pdf) ---");
        println!(
            "Method A1 (Glyphless Word): delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            eng_gw.delta_kb,
            eng_gw.build_time_millis,
            eng_gw.mean_error_pt,
            eng_gw.max_error_pt,
            eng_gw.matched_chars
        );
        println!(
            "Method A2 (Glyphless Char): delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            eng_gc.delta_kb,
            eng_gc.build_time_millis,
            eng_gc.mean_error_pt,
            eng_gc.max_error_pt,
            eng_gc.matched_chars
        );
        println!(
            "Method B  (Droid Sans):     delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            eng_ds.delta_kb,
            eng_ds.build_time_millis,
            eng_ds.mean_error_pt,
            eng_ds.max_error_pt,
            eng_ds.matched_chars
        );

        println!("--- Chinese+English (synthetic mixed-text-zh-en.pdf) ---");
        println!(
            "Method A1 (Glyphless Word): delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            zh_gw.delta_kb,
            zh_gw.build_time_millis,
            zh_gw.mean_error_pt,
            zh_gw.max_error_pt,
            zh_gw.matched_chars
        );
        println!(
            "Method A2 (Glyphless Char): delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            zh_gc.delta_kb,
            zh_gc.build_time_millis,
            zh_gc.mean_error_pt,
            zh_gc.max_error_pt,
            zh_gc.matched_chars
        );
        println!(
            "Method B  (Droid Sans):     delta={:.2} KB, time={:.2} ms, mean_err={:.2} pt, max_err={:.2} pt, matched={}",
            zh_ds.delta_kb,
            zh_ds.build_time_millis,
            zh_ds.mean_error_pt,
            zh_ds.max_error_pt,
            zh_ds.matched_chars
        );

        println!("=== POC 416 RESULTS END ===");

        assert!(eng_gw.visual_identical);
        assert!(eng_gc.visual_identical);
        assert!(eng_ds.visual_identical);
        assert!(zh_gw.visual_identical);
        assert!(zh_gc.visual_identical);
        assert!(zh_ds.visual_identical);

        assert!(eng_gw.matched_chars > 10);
        assert!(zh_gw.matched_chars > 20);
    }
}
