//! Correcting existing invisible OCR text layers (`3 Tr`, B7-12, Issue #749, ADR 0036).
//!
//! Inspection reads glyphs strictly via `PdfPage::filter_text` with `no_update: true,
//! recurse: true, instance_forms: false` so neither page contents nor Form XObjects are cloned or
//! modified during listing. Matching checks `glyph.render_mode() == Some(TextRenderMode::Invisible)`
//! exclusively (`Tr == 3`) and never `glyph.is_invisible()` (which includes opacity-0 visible text).
//! Replacement text uses the document's shared Type0/Identity-H `GlyphLessFont` (reusing any
//! existing instance across pages) with `Tz` horizontal scaling and `page_to_pdf` placement.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use ipc_contract::limits::{MAX_RECOGNISED_WORD_BYTES, MAX_RECOGNISED_WORDS_PER_PAGE};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{
    Point, Quad, RecognisedWord, RecognisedWordsPage, RecognisedWordsUnusable, Rect,
};
use ipc_contract::validate::{check_annotated_page, check_recognised_word_text};
use mupdf::Buffer;
use mupdf::pdf::{
    PdfDocument as MuPdfDocument, PdfObject, PdfPage, TextFilterAction, TextFilterOptions,
    TextRenderMode,
};

use super::searchable_copy::{FONT_BASE_NAME, write_glyphless_font};
use super::{EngineError, PdfDocument};

const MAX_FONTS_PER_PAGE: usize = 256;
const MAX_W_ENTRIES: usize = 4096;

#[derive(Debug, Clone)]
struct ScannedGlyph {
    glyph_idx: usize,
    raw_mode: i32,
    is_tr3: bool,
    is_clip: bool,
    in_form: bool,
    text: String,
    is_whitespace: bool,
    quad: Quad,
}

#[derive(Debug, Clone)]
struct ScannedWord {
    id: u32,
    text: String,
    rect: Rect,
    quad: Quad,
    glyph_indices: HashSet<usize>,
    in_form: bool,
}

enum ScanOutcome {
    Words(Vec<ScannedWord>),
    MixedRenderModes,
}

fn contract_quad(quad: mupdf::Quad) -> Quad {
    Quad {
        ul: Point {
            x: quad.ul.x,
            y: quad.ul.y,
        },
        ur: Point {
            x: quad.ur.x,
            y: quad.ur.y,
        },
        ll: Point {
            x: quad.ll.x,
            y: quad.ll.y,
        },
        lr: Point {
            x: quad.lr.x,
            y: quad.lr.y,
        },
    }
}

fn clean_glyph_chars(chars: &[char]) -> (String, bool) {
    if !chars.is_empty() && chars.iter().all(|c| c.is_whitespace()) {
        return (String::new(), true);
    }
    let mut cleaned = String::with_capacity(chars.len());
    for &c in chars {
        if c.is_whitespace() {
            continue;
        }
        let mut buf = [0u8; 4];
        cleaned.push_str(&clean_display_text(
            c.encode_utf8(&mut buf),
            MAX_RECOGNISED_WORD_BYTES as usize,
        ));
    }
    (cleaned, false)
}

fn norm(dx: f32, dy: f32) -> Option<(f32, f32, f32)> {
    let len = dx.hypot(dy);
    if len.is_finite() && len > 1e-4 {
        Some((dx / len, dy / len, len))
    } else {
        None
    }
}

struct WritingBasis {
    along: (f32, f32),
    perp: (f32, f32),
    height: f32,
}

fn writing_basis(quad: &Quad) -> Option<WritingBasis> {
    let (ux, uy, _) = norm(quad.ur.x - quad.ul.x, quad.ur.y - quad.ul.y)
        .or_else(|| norm(quad.lr.x - quad.ll.x, quad.lr.y - quad.ll.y))?;
    let (vx, vy, h) = norm(quad.ll.x - quad.ul.x, quad.ll.y - quad.ul.y)
        .or_else(|| norm(quad.lr.x - quad.ur.x, quad.lr.y - quad.ur.y))?;
    Some(WritingBasis {
        along: (ux, uy),
        perp: (vx, vy),
        height: h.max(1.0),
    })
}

fn aligned_with_prev(
    prev: &ScannedGlyph,
    curr: &ScannedGlyph,
    basis: &WritingBasis,
    min_ratio: f32,
    max_ratio: f32,
) -> bool {
    let (ux, uy) = basis.along;
    let (vx, vy) = basis.perp;
    if let Some(curr_basis) = writing_basis(&curr.quad)
        && ux * curr_basis.along.0 + uy * curr_basis.along.1 <= 0.9
    {
        return false;
    }
    let dx_start = curr.quad.ll.x - prev.quad.lr.x;
    let dy_start = curr.quad.ll.y - prev.quad.lr.y;
    let along = dx_start * ux + dy_start * uy;
    let base_dx = curr.quad.ll.x - prev.quad.ll.x;
    let base_dy = curr.quad.ll.y - prev.quad.ll.y;
    let perp = (base_dx * vx + base_dy * vy).abs();
    perp <= 0.45 * basis.height
        && (min_ratio * basis.height..=max_ratio * basis.height).contains(&along)
}

/// Whether `curr` immediately follows `prev` along the same writing baseline inside one contiguous
/// token (so switching between `3 Tr` and `4..=7 Tr` across `(prev, curr)` cannot be separated).
fn continues_same_segment(prev: &ScannedGlyph, curr: &ScannedGlyph) -> bool {
    writing_basis(&prev.quad)
        .is_some_and(|basis| aligned_with_prev(prev, curr, &basis, -0.25, 0.35))
}

/// Whether two consecutive `3 Tr` non-whitespace glyphs belong to the same space-delimited word.
fn same_tr3_word(prev: &ScannedGlyph, curr: &ScannedGlyph) -> bool {
    writing_basis(&prev.quad).is_none_or(|basis| aligned_with_prev(prev, curr, &basis, -0.30, 0.22))
}

fn tight_word_geometry(glyphs: &[&ScannedGlyph]) -> (Quad, Rect) {
    let WritingBasis {
        along: (ux, uy),
        perp: (vx, vy),
        ..
    } = glyphs
        .iter()
        .find_map(|g| writing_basis(&g.quad))
        .unwrap_or(WritingBasis {
            along: (1.0, 0.0),
            perp: (0.0, 1.0),
            height: 12.0,
        });

    let mut u_min = f32::INFINITY;
    let mut u_max = f32::NEG_INFINITY;
    let mut v_min = f32::INFINITY;
    let mut v_max = f32::NEG_INFINITY;
    let mut x0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y0 = f32::INFINITY;
    let mut y1 = f32::NEG_INFINITY;

    for g in glyphs {
        for p in [g.quad.ul, g.quad.ur, g.quad.ll, g.quad.lr] {
            let u = p.x * ux + p.y * uy;
            let v = p.x * vx + p.y * vy;
            u_min = u_min.min(u);
            u_max = u_max.max(u);
            v_min = v_min.min(v);
            v_max = v_max.max(v);
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
        }
    }

    let make_pt = |u: f32, v: f32| Point {
        x: u * ux + v * vx,
        y: u * uy + v * vy,
    };
    let quad = Quad {
        ul: make_pt(u_min, v_min),
        ur: make_pt(u_max, v_min),
        ll: make_pt(u_min, v_max),
        lr: make_pt(u_max, v_max),
    };
    let rect = Rect { x0, y0, x1, y1 };
    (quad, rect)
}

fn scan_tr3_page(page: &mut PdfPage) -> Result<ScanOutcome, EngineError> {
    let mut glyphs = Vec::new();
    let mut next_idx = 0usize;

    // Read-only pass: recurse without instancing forms and without updating page streams.
    let opts = TextFilterOptions {
        recurse: true,
        instance_forms: false,
        ascii: false,
        no_update: true,
        newlines: false,
    };
    page.filter_text(opts, |glyph| {
        let idx = next_idx;
        next_idx += 1;
        let raw_mode = glyph.raw_render_mode();
        // Strictly match Tr == 3 (`Invisible`), never `is_invisible()` (which matches alpha == 0).
        let is_tr3 = glyph.render_mode() == Some(TextRenderMode::Invisible);
        let is_clip = matches!(raw_mode, 4..=7);
        let (text, is_whitespace) = clean_glyph_chars(glyph.unicode());
        let quad = contract_quad(glyph.quad());
        let valid_quad = quad.ul.x.is_finite()
            && quad.ul.y.is_finite()
            && quad.lr.x.is_finite()
            && quad.lr.y.is_finite();
        if valid_quad && (is_tr3 || is_clip) {
            glyphs.push(ScannedGlyph {
                glyph_idx: idx,
                raw_mode,
                is_tr3,
                is_clip,
                in_form: glyph.in_form_xobject(),
                text,
                is_whitespace,
                quad,
            });
        } else if !valid_quad && is_tr3 {
            glyphs.push(ScannedGlyph {
                glyph_idx: idx,
                raw_mode,
                is_tr3,
                is_clip,
                in_form: glyph.in_form_xobject(),
                text,
                is_whitespace: true,
                quad,
            });
        }
        TextFilterAction::Keep
    })?;

    // Check inseparable mixing between 3 Tr and 4..=7 Tr in any continuous non-whitespace run.
    let mut prev_non_ws: Option<&ScannedGlyph> = None;
    for g in &glyphs {
        if g.is_whitespace || g.text.is_empty() {
            if g.is_whitespace {
                prev_non_ws = None;
            }
            continue;
        }
        if let Some(prev) = prev_non_ws
            && prev.raw_mode != g.raw_mode
            && ((prev.is_tr3 && g.is_clip) || (prev.is_clip && g.is_tr3))
            && continues_same_segment(prev, g)
        {
            return Ok(ScanOutcome::MixedRenderModes);
        }
        prev_non_ws = Some(g);
    }

    // Build words strictly from Tr == 3 glyphs.
    let mut raw_words: Vec<(Vec<&ScannedGlyph>, String)> = Vec::new();
    let mut current: Vec<&ScannedGlyph> = Vec::new();
    let mut current_text = String::new();
    let mut saw_space_since_tr3 = false;

    for g in &glyphs {
        if !g.is_tr3 {
            continue;
        }
        if g.is_whitespace {
            if !current.is_empty() {
                raw_words.push((
                    std::mem::take(&mut current),
                    std::mem::take(&mut current_text),
                ));
            }
            saw_space_since_tr3 = true;
            continue;
        }
        if g.text.is_empty() {
            continue;
        }
        let start_new = current.is_empty()
            || saw_space_since_tr3
            || !same_tr3_word(current.last().expect("non-empty"), g)
            || current_text.len() + g.text.len() > MAX_RECOGNISED_WORD_BYTES as usize;
        if start_new && !current.is_empty() {
            raw_words.push((
                std::mem::take(&mut current),
                std::mem::take(&mut current_text),
            ));
        }
        current.push(g);
        current_text.push_str(&g.text);
        saw_space_since_tr3 = false;
    }
    if !current.is_empty() {
        raw_words.push((current, current_text));
    }

    let mut words: Vec<ScannedWord> = raw_words
        .into_iter()
        .filter_map(|(gs, text)| {
            if text.trim().is_empty() {
                return None;
            }
            let (quad, rect) = tight_word_geometry(&gs);
            let glyph_indices = gs.iter().map(|g| g.glyph_idx).collect();
            let in_form = gs.iter().any(|g| g.in_form);
            Some(ScannedWord {
                id: 0,
                text,
                rect,
                quad,
                glyph_indices,
                in_form,
            })
        })
        .collect();

    // Sort in natural visual reading order (top-to-bottom bands, left-to-right within each band).
    words.sort_by(|a, b| {
        let a_mid = (a.rect.y0 + a.rect.y1) * 0.5;
        let b_mid = (b.rect.y0 + b.rect.y1) * 0.5;
        let min_h = (a.rect.y1 - a.rect.y0).min(b.rect.y1 - b.rect.y0).max(4.0);
        if (a_mid - b_mid).abs() <= 0.5 * min_h {
            a.rect
                .x0
                .total_cmp(&b.rect.x0)
                .then_with(|| a.rect.y0.total_cmp(&b.rect.y0))
        } else {
            a_mid.total_cmp(&b_mid)
        }
    });

    if words.len() > MAX_RECOGNISED_WORDS_PER_PAGE as usize {
        return Err(EngineError::TooComplex);
    }
    for (idx, w) in words.iter_mut().enumerate() {
        w.id = idx as u32;
    }

    Ok(ScanOutcome::Words(words))
}

fn is_app_glyphless_font(font: &PdfObject) -> bool {
    let Ok(true) = font.is_indirect() else {
        return false;
    };
    let name_eq = |obj: &PdfObject, key: &str, expected: &[u8]| -> bool {
        obj.get_dict(key)
            .ok()
            .flatten()
            .and_then(|v| v.as_name().ok())
            .is_some_and(|n| n == expected)
    };
    if !name_eq(font, "Subtype", b"Type0") || !name_eq(font, "Encoding", b"Identity-H") {
        return false;
    }
    if !font
        .get_dict("ToUnicode")
        .ok()
        .flatten()
        .is_some_and(|v| !v.is_null().unwrap_or(true))
    {
        return false;
    }
    let Some(descendants) = font.get_dict("DescendantFonts").ok().flatten() else {
        return false;
    };
    let Some(cid) = descendants.get_array(0).ok().flatten() else {
        return false;
    };
    if !name_eq(&cid, "Subtype", b"CIDFontType2")
        || !cid
            .get_dict("CIDToGIDMap")
            .ok()
            .flatten()
            .is_some_and(|v| !v.is_null().unwrap_or(true))
    {
        return false;
    }
    let Some(descriptor) = cid.get_dict("FontDescriptor").ok().flatten() else {
        return false;
    };
    name_eq(&descriptor, "FontName", b"GlyphLessFont")
        && descriptor
            .get_dict("FontFile2")
            .ok()
            .flatten()
            .is_some_and(|v| !v.is_null().unwrap_or(true))
}

fn find_existing_glyphless_font(doc: &MuPdfDocument) -> Result<Option<PdfObject>, EngineError> {
    let page_count = doc.page_count()?.max(0);
    for p in 0..page_count {
        let page = doc.find_page(p)?;
        let Some(resources) = page.get_dict("Resources")? else {
            continue;
        };
        let Some(fonts) = resources.get_dict("Font")? else {
            continue;
        };
        if let Some(ocr_font) = fonts.get_dict(FONT_BASE_NAME)?
            && is_app_glyphless_font(&ocr_font)
        {
            return Ok(Some(ocr_font));
        }
        if fonts.is_dict()?
            && let Ok(iter) = fonts.dict_iter()
        {
            for item in iter.take(MAX_FONTS_PER_PAGE) {
                if let Ok((_, val)) = item
                    && is_app_glyphless_font(&val)
                {
                    return Ok(Some(val));
                }
            }
        }
    }
    Ok(None)
}

fn resolve_glyphless_font(doc: &mut MuPdfDocument) -> Result<PdfObject, EngineError> {
    if let Some(existing) = find_existing_glyphless_font(doc)? {
        return Ok(existing);
    }
    write_glyphless_font(doc)
}

fn ensure_page_font_resource(
    doc: &mut MuPdfDocument,
    page_object: &mut PdfObject,
    font_obj: &PdfObject,
) -> Result<String, EngineError> {
    let target_num = font_obj.as_indirect()?;

    if let Some(res) = page_object.get_dict("Resources")?
        && let Some(fonts) = res.get_dict("Font")?
        && fonts.is_dict()?
        && let Ok(iter) = fonts.dict_iter()
    {
        for item in iter.take(MAX_FONTS_PER_PAGE) {
            if let Ok((key, val)) = item
                && val.as_indirect().ok() == Some(target_num)
                && let Ok(name) = key.as_name()
                && let Ok(s) = std::str::from_utf8(&name)
            {
                return Ok(s.to_owned());
            }
        }
    }

    let mut resources = match page_object.get_dict("Resources")? {
        Some(res) if res.is_dict()? => {
            if res.is_indirect()? {
                res.copy_dict()?
            } else {
                res
            }
        }
        _ => doc.new_dict()?,
    };
    let mut fonts = match resources.get_dict("Font")? {
        Some(f) if f.is_dict()? => {
            if f.is_indirect()? {
                f.copy_dict()?
            } else {
                f
            }
        }
        _ => doc.new_dict()?,
    };

    let mut key = FONT_BASE_NAME.to_owned();
    let mut suffix = 1u32;
    while fonts.get_dict(key.as_str())?.is_some() {
        key = format!("{FONT_BASE_NAME}{suffix}");
        suffix += 1;
    }
    fonts.dict_put(key.as_str(), font_obj.clone())?;
    resources.dict_put("Font", fonts)?;
    page_object.dict_put("Resources", resources)?;
    Ok(key)
}

fn glyphless_metrics_and_widths(font: &PdfObject) -> (f32, f32, f32, BTreeMap<u16, f32>) {
    let mut ascent = 1000.0f32;
    let mut descent = 0.0f32;
    let mut dw = 500.0f32;
    let mut widths = BTreeMap::new();

    if let Some(descendants) = font.get_dict("DescendantFonts").ok().flatten()
        && let Some(cid) = descendants.get_array(0).ok().flatten()
    {
        if let Some(dw_obj) = cid.get_dict("DW").ok().flatten()
            && let Ok(v) = dw_obj.as_float()
            && v > 0.0
        {
            dw = v;
        }
        if let Some(fd) = cid.get_dict("FontDescriptor").ok().flatten() {
            if let Some(a) = fd.get_dict("Ascent").ok().flatten()
                && let Ok(v) = a.as_float()
                && v.is_finite()
            {
                ascent = v;
            }
            if let Some(d) = fd.get_dict("Descent").ok().flatten()
                && let Ok(v) = d.as_float()
                && v.is_finite()
            {
                descent = v;
            }
        }
        if let Some(w_arr) = cid.get_dict("W").ok().flatten()
            && w_arr.is_array().unwrap_or(false)
            && let Ok(len) = w_arr.len()
        {
            let mut i = 0i32;
            let mut entries = 0usize;
            while (i as usize) + 1 < len && entries < MAX_W_ENTRIES {
                entries += 1;
                let Ok(Some(first_obj)) = w_arr.get_array(i) else {
                    break;
                };
                let Ok(Some(second_obj)) = w_arr.get_array(i + 1) else {
                    break;
                };
                let Ok(first_cid) = first_obj.as_int() else {
                    i += 2;
                    continue;
                };
                if second_obj.is_array().unwrap_or(false) {
                    let arr_len = second_obj.len().unwrap_or(0).min(MAX_W_ENTRIES);
                    for j in 0..arr_len {
                        if let Ok(Some(w_obj)) = second_obj.get_array(j as i32)
                            && let Ok(w) = w_obj.as_float()
                            && w.is_finite()
                            && let Ok(cid_u16) = u16::try_from(first_cid + j as i32)
                        {
                            widths.insert(cid_u16, w);
                        }
                    }
                    i += 2;
                } else if (i as usize) + 2 < len
                    && let Ok(last_cid) = second_obj.as_int()
                    && let Ok(Some(w_obj)) = w_arr.get_array(i + 2)
                    && let Ok(w) = w_obj.as_float()
                    && w.is_finite()
                {
                    let start = first_cid.max(0) as usize;
                    let end = last_cid.max(0).min(u16::MAX as i32) as usize;
                    for c in start..=end.min(start + MAX_W_ENTRIES) {
                        if let Ok(cid_u16) = u16::try_from(c) {
                            widths.insert(cid_u16, w);
                        }
                    }
                    i += 3;
                } else {
                    i += 2;
                }
            }
        }
    }
    (ascent, descent, dw, widths)
}

fn build_replacement_stream_bytes(
    inverse: mupdf::Matrix,
    word_quad: &Quad,
    font_obj: &PdfObject,
    font_key: &str,
    new_text: &str,
) -> Vec<u8> {
    let p_ll = mupdf::Point::new(word_quad.ll.x, word_quad.ll.y).transform(&inverse);
    let p_lr = mupdf::Point::new(word_quad.lr.x, word_quad.lr.y).transform(&inverse);
    let p_ul = mupdf::Point::new(word_quad.ul.x, word_quad.ul.y).transform(&inverse);

    let (wx, wy, pdf_w) = norm(p_lr.x - p_ll.x, p_lr.y - p_ll.y).unwrap_or((1.0, 0.0, 1.0));
    let (hx, hy, pdf_h) = norm(p_ul.x - p_ll.x, p_ul.y - p_ll.y).unwrap_or((0.0, 1.0, 1.0));

    let (ascent, descent, dw, widths) = glyphless_metrics_and_widths(font_obj);
    let span = (ascent - descent).max(1.0);
    let font_size = (pdf_h * (1000.0 / span)).max(1.0);
    // Shift baseline upward from the bottom edge of the quad by `-descent / span` of box height.
    let baseline_frac = (-descent / span).clamp(0.0, 0.5);
    let ox = p_ll.x + hx * (pdf_h * baseline_frac);
    let oy = p_ll.y + hy * (pdf_h * baseline_frac);

    let mut total_em = 0.0f32;
    let mut hex = String::with_capacity(new_text.len() * 4);
    for unit in new_text.encode_utf16() {
        let w = widths.get(&unit).copied().unwrap_or(dw);
        total_em += (w / 1000.0).max(0.05);
        let _ = write!(hex, "{unit:04X}");
    }
    let tz = ((pdf_w / (font_size * total_em.max(0.1))) * 100.0).clamp(1.0, 10_000.0);

    format!(
        "q\nBT\n/{font_key} {font_size:.3} Tf\n3 Tr\n{tz:.3} Tz\n{wx:.6} {wy:.6} {hx:.6} {hy:.6} {ox:.3} {oy:.3} Tm\n<{hex}> Tj\nET\nQ\n"
    )
    .into_bytes()
}

fn append_page_content_isolated(
    doc: &mut MuPdfDocument,
    page_object: &mut PdfObject,
    overlay_bytes: &[u8],
) -> Result<(), EngineError> {
    let existing = page_object.get_dict("Contents")?;
    let mut parts: Vec<PdfObject> = Vec::new();
    match existing {
        None => {
            let stream = doc.add_stream(&Buffer::from_bytes(overlay_bytes)?, None, false)?;
            page_object.dict_put("Contents", stream)?;
            return Ok(());
        }
        Some(obj) if obj.is_null()? => {
            let stream = doc.add_stream(&Buffer::from_bytes(overlay_bytes)?, None, false)?;
            page_object.dict_put("Contents", stream)?;
            return Ok(());
        }
        Some(obj) if obj.is_array()? => {
            for item in obj.array_iter()? {
                parts.push(item?);
            }
        }
        Some(obj) => {
            parts.push(obj);
        }
    }

    if parts.is_empty() {
        let stream = doc.add_stream(&Buffer::from_bytes(overlay_bytes)?, None, false)?;
        page_object.dict_put("Contents", stream)?;
        return Ok(());
    }

    let q_prefix = doc.add_stream(&Buffer::from_bytes(b"q\n")?, None, false)?;
    let mut suffix = Vec::with_capacity(3 + overlay_bytes.len());
    suffix.extend_from_slice(b"\nQ\n");
    suffix.extend_from_slice(overlay_bytes);
    let q_suffix_and_overlay = doc.add_stream(&Buffer::from_bytes(&suffix)?, None, false)?;

    let mut arr = doc.new_array()?;
    arr.array_push(q_prefix)?;
    for part in parts {
        arr.array_push(part)?;
    }
    arr.array_push(q_suffix_and_overlay)?;
    page_object.dict_put("Contents", arr)?;
    Ok(())
}

impl PdfDocument {
    /// Lists editable `3 Tr` invisible OCR words on page `index`, or returns why the page cannot
    /// be corrected in place (B7-12, #749, ADR 0036).
    pub fn recognised_words(&self, index: u32) -> Result<RecognisedWordsPage, EngineError> {
        let mut page = self.pdf_page(index)?;
        if self.is_signed() {
            return Ok(RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::Signed,
            });
        }
        if !self.permissions().modify {
            return Ok(RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::NotAllowed,
            });
        }

        match scan_tr3_page(&mut page)? {
            ScanOutcome::MixedRenderModes => Ok(RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::MixedRenderModes,
            }),
            ScanOutcome::Words(words) => {
                if words.is_empty() {
                    let reason = if self.ocr_text(index).is_some() {
                        RecognisedWordsUnusable::MemoryOcrOnly
                    } else {
                        RecognisedWordsUnusable::NoTextLayer
                    };
                    return Ok(RecognisedWordsPage::Unusable { reason });
                }
                Ok(RecognisedWordsPage::Usable {
                    words: words
                        .into_iter()
                        .map(|w| RecognisedWord {
                            id: w.id,
                            text: w.text,
                            rect: w.rect,
                            quad: w.quad,
                        })
                        .collect(),
                })
            }
        }
    }

    /// Replaces one `3 Tr` word (`word_id`) on page `index` with `new_text` after verifying its
    /// existing text equals `expected` (B7-12, #749, ADR 0036).
    pub fn correct_recognised_text(
        &mut self,
        index: u32,
        word_id: u32,
        expected: &str,
        new_text: &str,
    ) -> Result<(), EngineError> {
        check_annotated_page(index).map_err(|_| EngineError::PageOutOfRange(index))?;
        check_recognised_word_text("expected recognised text", expected)
            .map_err(|_| EngineError::InvalidEdit("expected recognised text is invalid"))?;
        check_recognised_word_text("corrected recognised text", new_text)
            .map_err(|_| EngineError::InvalidEdit("corrected recognised text is invalid"))?;

        if self.is_signed() {
            return Err(EngineError::Signed);
        }
        if !self.permissions().modify {
            return Err(EngineError::NotAllowed(
                "modifying this document is not permitted",
            ));
        }

        // First scan read-only before mutating anything on the page or document.
        let target = {
            let mut page = self.pdf_page(index)?;
            match scan_tr3_page(&mut page)? {
                ScanOutcome::MixedRenderModes => {
                    return Err(EngineError::InvalidEdit(
                        "cannot separate invisible OCR text from clipping text in the same segment",
                    ));
                }
                ScanOutcome::Words(words) => {
                    words
                        .into_iter()
                        .find(|w| w.id == word_id)
                        .ok_or(EngineError::InvalidEdit(
                            "recognised word not found on page",
                        ))?
                }
            }
        };

        if target.text != expected {
            return Err(EngineError::InvalidEdit(
                "recognised word does not match expected text",
            ));
        }

        // Remove only the target word's Tr == 3 glyphs via MuPDF's content filter.
        {
            let mut page = self.pdf_page(index)?;
            let mut next_idx = 0usize;
            let opts = TextFilterOptions {
                recurse: false,
                instance_forms: target.in_form,
                ascii: false,
                no_update: false,
                newlines: false,
            };
            page.filter_text(opts, |glyph| {
                let idx = next_idx;
                next_idx += 1;
                if target.glyph_indices.contains(&idx)
                    && glyph.render_mode() == Some(TextRenderMode::Invisible)
                {
                    TextFilterAction::Remove
                } else {
                    TextFilterAction::Keep
                }
            })?;
        }

        // Attach shared GlyphLessFont and append Tz-fitted 3 Tr replacement stream.
        let (mut page_object, inverse) = self.page_to_pdf(index)?;
        let font_obj = resolve_glyphless_font(&mut self.doc)?;
        let font_key = ensure_page_font_resource(&mut self.doc, &mut page_object, &font_obj)?;
        let stream_bytes =
            build_replacement_stream_bytes(inverse, &target.quad, &font_obj, &font_key, new_text);
        append_page_content_isolated(&mut self.doc, &mut page_object, &stream_bytes)?;

        self.invalidate_page_text_cache(index);
        Ok(())
    }
}
