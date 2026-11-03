//! Merging annotations from a source PDF (B3-23, ADR 0019).
//!
//! Only the app's supported five annotation kinds (Highlight, Note, Ink, Stamp [standard & picture],
//! and Text Box) are extracted as clean pure data models and recreated on the inserted pages.
//! Untrusted dictionaries, streams, /A, /AA, /Popup, /JavaScript, and /Launch actions are completely
//! discarded and never copied into the target document.

use ipc_contract::limits::{
    MAX_ANNOTATION_QUADS, MAX_INK_POINTS, MAX_INK_STROKES, MAX_NOTE_TEXT_BYTES, MAX_PAGE_SIDE_PT,
    MAX_SOURCE_ANNOTATIONS, MAX_SOURCE_TEXT_BOXES, MAX_STAMP_PNG_BYTES, MAX_STAMP_SOURCE_PIXELS,
    MAX_STAMP_SOURCE_SIDE_PX,
};
use ipc_contract::text::clean_note_text;
use ipc_contract::types::{
    Edit, HighlightColor, HighlightMark, InkColor, InkWidth, Point, Quad, Rect, StampName,
    TextMarkKind,
};
use ipc_contract::validate::{
    Validate, check_angle, check_annotated_page, check_annotation_rect, check_note_text,
    stamp_png_size,
};
use ipc_contract::worker::{InsertedTextBox, SourceTextBox};
use mupdf::pdf::{PdfAnnotationType, PdfObject};
use mupdf::{Colorspace, ImageFormat, Pixmap};

use super::annotations::{highlight_color, parse_ink_color, parse_ink_width, parse_stamp_name};
use super::stamp_image::{entry_of, read_setting};
use super::text_box::is_text_box;
use super::text_box::pdf::{KEY as TEXT_BOX_KEY, read_source_text_box_info};
use super::{EngineError, PdfDocument};

/// Whether `bounds` is finite, upright, and within page coordinate limits.
pub(super) fn valid_bounds(bounds: &mupdf::Rect) -> bool {
    bounds.x0.is_finite()
        && bounds.y0.is_finite()
        && bounds.x1.is_finite()
        && bounds.y1.is_finite()
        && bounds.x1 > bounds.x0
        && bounds.y1 > bounds.y0
        && bounds.x0.abs() <= MAX_PAGE_SIDE_PT
        && bounds.y0.abs() <= MAX_PAGE_SIDE_PT
        && bounds.x1.abs() <= MAX_PAGE_SIDE_PT
        && bounds.y1.abs() <= MAX_PAGE_SIDE_PT
}

/// Whether an annotation bounds intersects the page bounds. If there is no overlap, it is
/// completely outside the page and must be skipped.
pub(super) fn intersects_page(page_bounds: &mupdf::Rect, bounds: &mupdf::Rect) -> bool {
    let min_x = page_bounds.x0.min(page_bounds.x1);
    let max_x = page_bounds.x0.max(page_bounds.x1);
    let min_y = page_bounds.y0.min(page_bounds.y1);
    let max_y = page_bounds.y0.max(page_bounds.y1);

    bounds.x1 > min_x && bounds.x0 < max_x && bounds.y1 > min_y && bounds.y0 < max_y
}

/// Determines if the colorspace is RGB or Grayscale (including /DeviceRGB, /DeviceGray, and /ICCBased).
fn is_rgb_or_gray(colorspace: &PdfObject) -> Option<(bool, bool)> {
    if colorspace.as_name().ok().as_deref() == Some(b"DeviceRGB") {
        return Some((true, false));
    }
    if colorspace.as_name().ok().as_deref() == Some(b"DeviceGray") {
        return Some((false, true));
    }
    if let Ok(true) = colorspace.is_array() {
        let first = colorspace
            .get_array(0)
            .ok()
            .flatten()
            .and_then(|o| o.as_name().ok());
        if first.as_deref() == Some(b"DeviceRGB") {
            return Some((true, false));
        }
        if first.as_deref() == Some(b"DeviceGray") {
            return Some((false, true));
        }
        if first.as_deref() == Some(b"ICCBased") {
            let stream_ref = colorspace.get_array(1).ok().flatten()?;
            let stream_dict = stream_ref.resolve().ok().flatten().unwrap_or(stream_ref);
            let n = entry_of(&stream_dict, "N")
                .and_then(|o| o.as_int().ok())
                .unwrap_or(0);
            if n == 3 {
                return Some((true, false));
            }
            if n == 1 {
                return Some((false, true));
            }
        }
    }
    None
}

/// Decodes an image XObject into clean PNG bytes, safely validating sizes and combining soft masks.
fn extract_picture_png(picture_ref: &PdfObject) -> Option<Vec<u8>> {
    let image_dict = picture_ref
        .resolve()
        .ok()
        .flatten()
        .unwrap_or_else(|| picture_ref.clone());
    let width = entry_of(&image_dict, "Width").and_then(|o| o.as_int().ok())? as u32;
    let height = entry_of(&image_dict, "Height").and_then(|o| o.as_int().ok())? as u32;
    if width == 0
        || height == 0
        || width > MAX_STAMP_SOURCE_SIDE_PX
        || height > MAX_STAMP_SOURCE_SIDE_PX
        || (width as u64) * (height as u64) > MAX_STAMP_SOURCE_PIXELS
    {
        return None;
    }
    let bpc = entry_of(&image_dict, "BitsPerComponent")
        .and_then(|o| o.as_int().ok())
        .unwrap_or(8);
    if bpc != 8 {
        return None;
    }
    let colorspace_obj = entry_of(&image_dict, "ColorSpace")?;
    let (is_rgb, _) = is_rgb_or_gray(&colorspace_obj)?;

    let samples = picture_ref.read_stream().ok()?;
    let pixel_count = (width as usize) * (height as usize);
    let expected_len = if is_rgb { pixel_count * 3 } else { pixel_count };
    if samples.len() != expected_len {
        return None;
    }

    let smask_samples = if let Some(smask_ref) = image_dict.get_dict("SMask").ok().flatten() {
        let smask_dict = smask_ref
            .resolve()
            .ok()
            .flatten()
            .unwrap_or_else(|| smask_ref.clone());
        let sw = entry_of(&smask_dict, "Width").and_then(|o| o.as_int().ok())? as u32;
        let sh = entry_of(&smask_dict, "Height").and_then(|o| o.as_int().ok())? as u32;
        if sw != width || sh != height {
            return None;
        }
        let sbytes = smask_ref.read_stream().ok()?;
        let s_bpc = entry_of(&smask_dict, "BitsPerComponent")
            .and_then(|o| o.as_int().ok())
            .unwrap_or(8);
        let mut alpha = Vec::with_capacity(pixel_count);
        if s_bpc == 8 {
            if sbytes.len() != pixel_count {
                return None;
            }
            alpha = sbytes;
        } else if s_bpc == 1 {
            let row_bytes = (width as usize).div_ceil(8);
            if sbytes.len() != row_bytes * (height as usize) {
                return None;
            }
            for y in 0..(height as usize) {
                let row = &sbytes[y * row_bytes..(y + 1) * row_bytes];
                for x in 0..(width as usize) {
                    let byte = row[x / 8];
                    let bit = (byte >> (7 - (x % 8))) & 1;
                    alpha.push(if bit == 1 { 255 } else { 0 });
                }
            }
        } else {
            return None;
        }
        Some(alpha)
    } else {
        None
    };

    let cs = Colorspace::device_rgb();
    let has_alpha = smask_samples.is_some();
    let mut pixmap = Pixmap::new_with_w_h(&cs, width as i32, height as i32, has_alpha).ok()?;
    let dest = pixmap.samples_mut();

    if let Some(alpha) = smask_samples {
        if dest.len() != pixel_count * 4 {
            return None;
        }
        for i in 0..pixel_count {
            let a = alpha[i];
            let (r, g, b) = if is_rgb {
                (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2])
            } else {
                let g = samples[i];
                (g, g, g)
            };
            dest[i * 4] = ((r as u16 * a as u16 + 127) / 255) as u8;
            dest[i * 4 + 1] = ((g as u16 * a as u16 + 127) / 255) as u8;
            dest[i * 4 + 2] = ((b as u16 * a as u16 + 127) / 255) as u8;
            dest[i * 4 + 3] = a;
        }
    } else {
        if dest.len() != pixel_count * 3 {
            return None;
        }
        for i in 0..pixel_count {
            let (r, g, b) = if is_rgb {
                (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2])
            } else {
                let g = samples[i];
                (g, g, g)
            };
            dest[i * 3] = r;
            dest[i * 3 + 1] = g;
            dest[i * 3 + 2] = b;
        }
    }

    let mut png = Vec::new();
    pixmap.write_to(&mut png, ImageFormat::PNG).ok()?;
    stamp_png_size(&png).ok()?;
    if png.len() > MAX_STAMP_PNG_BYTES {
        return None;
    }
    Some(png)
}

/// A clean extracted annotation from a source document.
pub(super) enum ExtractedAnnotation {
    Highlight {
        kind: TextMarkKind,
        page: u32,
        quads: Vec<Quad>,
        color: HighlightColor,
    },
    Note {
        page: u32,
        at: Point,
        text: String,
    },
    Ink {
        page: u32,
        strokes: Vec<Vec<Point>>,
        color: InkColor,
        width: InkWidth,
    },
    Stamp {
        page: u32,
        rect: Rect,
        stamp: StampName,
    },
    PictureStamp {
        page: u32,
        upright_rect: Rect,
        png: Vec<u8>,
        angle_degrees: f32,
    },
}

/// Extracts clean recreatable annotations (Highlight, Note, Ink, Stamp) from a source document.
pub(super) fn extract_source_annotations(
    doc: &PdfDocument,
) -> Result<Vec<ExtractedAnnotation>, EngineError> {
    let page_count = doc.page_count()?;
    let mut extracted = Vec::new();

    for page_idx in 0..page_count {
        if extracted.len() >= MAX_SOURCE_ANNOTATIONS as usize {
            break;
        }
        let Ok(page) = doc.pdf_page(page_idx) else {
            continue;
        };
        let Ok(page_bounds) = page.bounds() else {
            continue;
        };
        if !valid_bounds(&page_bounds) {
            continue;
        }

        for annot in page.annotations() {
            if extracted.len() >= MAX_SOURCE_ANNOTATIONS as usize {
                break;
            }
            let Ok(kind) = annot.r#type() else {
                continue;
            };
            let Ok(bounds) = annot.bounds() else {
                continue;
            };
            if !valid_bounds(&bounds) || !intersects_page(&page_bounds, &bounds) {
                continue;
            }

            match kind {
                PdfAnnotationType::Highlight
                | PdfAnnotationType::Underline
                | PdfAnnotationType::StrikeOut
                | PdfAnnotationType::Squiggly => {
                    let mark_kind = match kind {
                        PdfAnnotationType::Highlight => TextMarkKind::Highlight,
                        PdfAnnotationType::Underline => TextMarkKind::Underline,
                        PdfAnnotationType::StrikeOut => TextMarkKind::StrikeOut,
                        PdfAnnotationType::Squiggly => TextMarkKind::Squiggly,
                        _ => unreachable!(),
                    };
                    let Ok(quads) = annot.quad_points() else {
                        continue;
                    };
                    if quads.is_empty() || quads.len() > MAX_ANNOTATION_QUADS as usize {
                        continue;
                    }
                    let quads: Vec<Quad> = quads
                        .into_iter()
                        .map(|q| Quad {
                            ul: Point {
                                x: q.ul.x,
                                y: q.ul.y,
                            },
                            ur: Point {
                                x: q.ur.x,
                                y: q.ur.y,
                            },
                            ll: Point {
                                x: q.ll.x,
                                y: q.ll.y,
                            },
                            lr: Point {
                                x: q.lr.x,
                                y: q.lr.y,
                            },
                        })
                        .collect();
                    if quads.iter().any(|q| q.validate().is_err()) {
                        continue;
                    }
                    let Some(color) = annot.color().ok().flatten().and_then(highlight_color) else {
                        continue;
                    };
                    extracted.push(ExtractedAnnotation::Highlight {
                        kind: mark_kind,
                        page: page_idx,
                        quads,
                        color,
                    });
                }
                PdfAnnotationType::Text => {
                    let Some(text) = annot.contents().ok().flatten() else {
                        continue;
                    };
                    if text.len() > MAX_NOTE_TEXT_BYTES as usize {
                        continue;
                    }
                    let cleaned = clean_note_text(text, MAX_NOTE_TEXT_BYTES as usize);
                    if cleaned.is_empty()
                        || cleaned.len() > MAX_NOTE_TEXT_BYTES as usize
                        || check_note_text(&cleaned).is_err()
                    {
                        continue;
                    }
                    if annot.rect().is_ok_and(|r| r.x1 < r.x0 || r.y1 < r.y0) {
                        continue;
                    }
                    let r = annot.rect().ok().unwrap_or(bounds);
                    let at = Point { x: r.x0, y: r.y0 };
                    if at.validate().is_err() {
                        continue;
                    }
                    if at.x < page_bounds.x0.min(page_bounds.x1)
                        || at.x > page_bounds.x0.max(page_bounds.x1)
                        || at.y < page_bounds.y0.min(page_bounds.y1)
                        || at.y > page_bounds.y0.max(page_bounds.y1)
                    {
                        continue;
                    }
                    extracted.push(ExtractedAnnotation::Note {
                        page: page_idx,
                        at,
                        text: cleaned,
                    });
                }
                PdfAnnotationType::Ink => {
                    let Ok(strokes) = annot.ink_list() else {
                        continue;
                    };
                    if strokes.is_empty()
                        || strokes.len() > MAX_INK_STROKES as usize
                        || strokes.iter().any(Vec::is_empty)
                        || strokes.iter().map(Vec::len).sum::<usize>() > MAX_INK_POINTS as usize
                    {
                        continue;
                    }
                    let converted: Vec<Vec<Point>> = strokes
                        .into_iter()
                        .map(|stroke| {
                            stroke
                                .into_iter()
                                .map(|p| Point { x: p.x, y: p.y })
                                .collect()
                        })
                        .collect();
                    if converted.iter().flatten().any(|p| p.validate().is_err()) {
                        continue;
                    }
                    let Some(color) = annot.color().ok().flatten().and_then(parse_ink_color) else {
                        continue;
                    };
                    let width = annot
                        .border_width()
                        .ok()
                        .and_then(parse_ink_width)
                        .unwrap_or(InkWidth::Medium);
                    extracted.push(ExtractedAnnotation::Ink {
                        page: page_idx,
                        strokes: converted,
                        color,
                        width,
                    });
                }
                PdfAnnotationType::Stamp => {
                    if let Some((setting, picture)) = read_setting(&annot) {
                        if let Some(png) = extract_picture_png(&picture) {
                            let center_x = (bounds.x0 + bounds.x1) / 2.0;
                            let center_y = (bounds.y0 + bounds.y1) / 2.0;
                            let upright_rect = Rect {
                                x0: center_x - setting.width / 2.0,
                                y0: center_y - setting.height / 2.0,
                                x1: center_x + setting.width / 2.0,
                                y1: center_y + setting.height / 2.0,
                            };
                            let angle_degrees = -setting.angle.to_degrees();
                            if check_annotation_rect(&upright_rect).is_ok()
                                && check_angle(angle_degrees).is_ok()
                            {
                                extracted.push(ExtractedAnnotation::PictureStamp {
                                    page: page_idx,
                                    upright_rect,
                                    png,
                                    angle_degrees,
                                });
                            }
                        }
                    } else {
                        let name_str = annot
                            .icon_name()
                            .ok()
                            .flatten()
                            .map(ToOwned::to_owned)
                            .or_else(|| {
                                annot
                                    .object()
                                    .get_dict("Name")
                                    .ok()
                                    .flatten()
                                    .and_then(|o| o.as_name().ok())
                                    .as_deref()
                                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                                    .map(ToOwned::to_owned)
                            });
                        if let Some(stamp) = name_str.as_deref().and_then(parse_stamp_name) {
                            if annot.rect().is_ok_and(|r| r.x1 < r.x0 || r.y1 < r.y0) {
                                continue;
                            }
                            let r = annot.rect().ok().unwrap_or(bounds);
                            let rect = Rect {
                                x0: r.x0,
                                y0: r.y0,
                                x1: r.x1,
                                y1: r.y1,
                            };
                            if check_annotation_rect(&rect).is_ok() {
                                extracted.push(ExtractedAnnotation::Stamp {
                                    page: page_idx,
                                    rect,
                                    stamp,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(extracted)
}

/// Extracts text box metadata (SourceTextBox) from the source document.
pub(super) fn extract_source_text_boxes(
    doc: &PdfDocument,
) -> Result<Vec<SourceTextBox>, EngineError> {
    let page_count = doc.page_count()?;
    let mut text_boxes = Vec::new();

    for page_idx in 0..page_count {
        if text_boxes.len() >= MAX_SOURCE_TEXT_BOXES as usize {
            break;
        }
        let Ok(page) = doc.pdf_page(page_idx) else {
            continue;
        };
        let Ok(page_bounds) = page.bounds() else {
            continue;
        };
        if !valid_bounds(&page_bounds) {
            continue;
        }

        for annot in page.annotations() {
            if text_boxes.len() >= MAX_SOURCE_TEXT_BOXES as usize {
                break;
            }
            if annot.r#type().ok() != Some(PdfAnnotationType::FreeText) || !is_text_box(&annot) {
                continue;
            }
            let Ok(bounds) = annot.bounds() else {
                continue;
            };
            if !valid_bounds(&bounds) || !intersects_page(&page_bounds, &bounds) {
                continue;
            }
            if annot.rect().is_ok_and(|r| r.x1 < r.x0 || r.y1 < r.y0) {
                continue;
            }
            let rect = Rect {
                x0: bounds.x0,
                y0: bounds.y0,
                x1: bounds.x1,
                y1: bounds.y1,
            };
            if check_annotation_rect(&rect).is_err() {
                continue;
            }
            let Ok(Some(entry)) = annot.object().get_dict(TEXT_BOX_KEY) else {
                continue;
            };
            let Ok((text, font, style)) = read_source_text_box_info(&entry) else {
                continue;
            };
            text_boxes.push(SourceTextBox {
                page: page_idx,
                rect,
                text,
                font_family: font,
                style,
            });
        }
    }

    Ok(text_boxes)
}

/// Recreates the extracted annotations and text boxes on the inserted pages of `doc`.
pub(super) fn recreate_annotations(
    doc: &mut PdfDocument,
    at: u32,
    annotations: &[ExtractedAnnotation],
    text_boxes: &[InsertedTextBox],
) -> Result<(), EngineError> {
    for annot in annotations {
        match annot {
            ExtractedAnnotation::Highlight {
                kind,
                page,
                quads,
                color,
            } => {
                let mark = HighlightMark {
                    page: at + page,
                    quads: quads.clone(),
                };
                let edit = Edit::AddHighlight {
                    mark_kind: *kind,
                    marks: vec![mark.clone()],
                    color: *color,
                };
                if edit.validate().is_ok() {
                    let _ = doc.add_text_marks(*kind, &[mark], *color);
                }
            }
            ExtractedAnnotation::Note { page, at: pt, text } => {
                let edit = Edit::AddNote {
                    page: at + page,
                    at: *pt,
                    text: text.clone(),
                };
                if edit.validate().is_ok() {
                    let _ = doc.add_note(at + page, *pt, text);
                }
            }
            ExtractedAnnotation::Ink {
                page,
                strokes,
                color,
                width,
            } => {
                let edit = Edit::AddInk {
                    page: at + page,
                    strokes: strokes.clone(),
                    color: *color,
                    width: *width,
                };
                if edit.validate().is_ok() {
                    let _ = doc.add_ink(at + page, strokes, *color, *width);
                }
            }
            ExtractedAnnotation::Stamp { page, rect, stamp } => {
                let edit = Edit::AddStamp {
                    page: at + page,
                    rect: *rect,
                    stamp: *stamp,
                };
                if edit.validate().is_ok() {
                    let _ = doc.add_stamp(at + page, *rect, *stamp);
                }
            }
            ExtractedAnnotation::PictureStamp {
                page,
                upright_rect,
                png,
                angle_degrees,
            } => {
                if check_annotated_page(at + page).is_err()
                    || check_annotation_rect(upright_rect).is_err()
                    || check_angle(*angle_degrees).is_err()
                    || png.len() > MAX_STAMP_PNG_BYTES
                    || stamp_png_size(png).is_err()
                {
                    continue;
                }
                let Ok(id) = doc.add_image_stamp(at + page, *upright_rect, png) else {
                    continue;
                };
                if angle_degrees.abs() > 1e-3 {
                    let _ = doc.set_picture_angle(at + page, id, *angle_degrees);
                }
            }
        }
    }

    for tb in text_boxes {
        if check_annotated_page(at + tb.page).is_ok()
            && check_annotation_rect(&tb.rect).is_ok()
            && tb.style.validate().is_ok()
            && tb.model.validate().is_ok()
        {
            let _ = doc.place_text_box(at + tb.page, tb.rect, &tb.model, &tb.style);
        }
    }

    Ok(())
}
