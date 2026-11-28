//! Exporting annotations as XFDF (ISO 19444-1, B5-09).
//!
//! XFDF is an XML standard for exchanging annotations without sending the whole document.
//! Coordinates are in PDF user space (as written in the annotation dictionary), which is what
//! XFDF uses. Values are read directly from the annotation dictionary.
//!
//! Excluded from export:
//! - Custom image stamps and foreign custom stamps: XFDF would require embedding the appearance
//!   stream in base64, which can be very large. Only the 14 standard PDF stamps are exported.
//! - Non-app text boxes, unsupported shapes, links, and form fields.

use std::collections::{HashMap, HashSet};

use ipc_contract::limits::{
    MAX_ANNOTATION_QUADS, MAX_ANNOTATION_SUMMARY_ITEMS, MAX_ANNOTATION_SUMMARY_TEXT_BYTES,
    MAX_ANNOTATION_SUMMARY_TOTAL_BYTES, MAX_ANNOTATIONS_PER_PAGE, MAX_INK_POINTS, MAX_INK_STROKES,
    MAX_PAGE_SIDE_PT, MAX_SHAPE_VERTICES,
};
use ipc_contract::text::clean_note_text;
use ipc_contract::types::{Point, TextAlign};
use ipc_contract::worker::{
    ExportedAnnotation, ExportedAnnotationKind, ExportedAnnotations, ExportedLineEnding,
    ExportedTextStyle,
};
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType, PdfObject};

use super::{EngineError, PdfDocument};

/// The 14 standard PDF stamps defined in the PDF specification (ISO 32000-1, Table 180).
const STANDARD_STAMPS: [&str; 14] = [
    "Approved",
    "Experimental",
    "NotApproved",
    "AsIs",
    "Expired",
    "NotForPublicRelease",
    "Confidential",
    "Final",
    "Sold",
    "Departmental",
    "ForComment",
    "TopSecret",
    "Draft",
    "ForPublicRelease",
];

/// A candidate annotation collected on a page before sorting and resolving reply references.
struct Candidate {
    annotation: PdfAnnotation,
    kind: ExportedAnnotationKind,
    rect: [f32; 4],
    quads: Vec<[f32; 8]>,
    ink: Vec<Vec<[f32; 2]>>,
    line: Option<[f32; 4]>,
    vertices: Vec<[f32; 2]>,
    cloudy: bool,
    start: Point,
    number: u32,
    indirect: i32,
}

struct PartialItem {
    item: ExportedAnnotation,
    irt_target: Option<i32>,
}

impl PdfDocument {
    /// Collects annotations across all pages of the document for XFDF export (B5-09).
    pub fn annotation_export(&self) -> Result<ExportedAnnotations, EngineError> {
        let mut items = Vec::new();
        let mut skipped = 0u32;
        let mut truncated = false;
        let mut total_bytes = 0usize;
        let mut used_names = HashSet::new();
        let mut exported_names_by_indirect: HashMap<i32, String> = HashMap::new();
        let mut partial_items: Vec<PartialItem> = Vec::new();

        let page_count = self.page_count()?;
        'pages: for page_index in 0..page_count {
            let page = self.pdf_page(page_index)?;
            let mut candidates: Vec<Candidate> = Vec::new();

            for annotation in page.annotations() {
                let annot_type = match annotation.r#type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let dict = annotation.object();

                let kind = match annot_type {
                    PdfAnnotationType::Highlight => ExportedAnnotationKind::Highlight,
                    PdfAnnotationType::Underline => ExportedAnnotationKind::Underline,
                    PdfAnnotationType::StrikeOut => ExportedAnnotationKind::StrikeOut,
                    PdfAnnotationType::Squiggly => ExportedAnnotationKind::Squiggly,
                    PdfAnnotationType::Text => ExportedAnnotationKind::Note,
                    PdfAnnotationType::Ink => ExportedAnnotationKind::Ink,
                    PdfAnnotationType::Stamp => {
                        if read_stamp_icon(&dict).is_none() {
                            skipped += 1;
                            continue;
                        }
                        ExportedAnnotationKind::Stamp
                    }
                    PdfAnnotationType::FreeText => {
                        if !super::text_box::is_text_box(&annotation) {
                            continue;
                        }
                        ExportedAnnotationKind::TextBox
                    }
                    PdfAnnotationType::Square => {
                        if super::annotations::has_unsupported_shape_features(&annotation)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        ExportedAnnotationKind::Rectangle
                    }
                    PdfAnnotationType::Circle => {
                        if super::annotations::has_unsupported_shape_features(&annotation)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        ExportedAnnotationKind::Ellipse
                    }
                    PdfAnnotationType::Line => {
                        if super::annotations::has_unsupported_shape_features(&annotation)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        ExportedAnnotationKind::Line
                    }
                    PdfAnnotationType::Polygon => {
                        if super::annotations::has_unsupported_shape_features(&annotation)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        ExportedAnnotationKind::Polygon
                    }
                    PdfAnnotationType::PolyLine => {
                        if super::annotations::has_unsupported_shape_features(&annotation)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        ExportedAnnotationKind::PolyLine
                    }
                    _ => continue,
                };

                let rect = match read_rect(&dict) {
                    Some(r) => r,
                    None => {
                        skipped += 1;
                        continue;
                    }
                };

                let quads = if matches!(
                    kind,
                    ExportedAnnotationKind::Highlight
                        | ExportedAnnotationKind::Underline
                        | ExportedAnnotationKind::StrikeOut
                        | ExportedAnnotationKind::Squiggly
                ) {
                    match read_quad_points(&dict) {
                        Some(q) => q,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    }
                } else {
                    Vec::new()
                };

                let ink = if kind == ExportedAnnotationKind::Ink {
                    match read_ink_list(&dict) {
                        Some(i) => i,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    }
                } else {
                    Vec::new()
                };

                let line = if kind == ExportedAnnotationKind::Line {
                    match read_line(&dict) {
                        Some(l) => Some(l),
                        None => {
                            skipped += 1;
                            continue;
                        }
                    }
                } else {
                    None
                };

                let vertices = match kind {
                    ExportedAnnotationKind::Polygon => match read_vertices(&dict, 3) {
                        Some(v) => v,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    },
                    ExportedAnnotationKind::PolyLine => match read_vertices(&dict, 2) {
                        Some(v) => v,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    },
                    _ => Vec::new(),
                };

                let cloudy = matches!(
                    kind,
                    ExportedAnnotationKind::Rectangle | ExportedAnnotationKind::Polygon
                ) && super::annotations::has_app_cloudy_border(&annotation);

                let indirect = match dict.as_indirect() {
                    Ok(id) => id,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };

                // Determine sorting start position in page space (visual reading order).
                let is_markup = matches!(
                    kind,
                    ExportedAnnotationKind::Highlight
                        | ExportedAnnotationKind::Underline
                        | ExportedAnnotationKind::StrikeOut
                        | ExportedAnnotationKind::Squiggly
                );
                let page_quads = if is_markup {
                    annotation.quads().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let start = page_quads
                    .first()
                    .map(|q| {
                        let min_x = q.ul.x.min(q.ur.x).min(q.ll.x).min(q.lr.x);
                        let min_y = q.ul.y.min(q.ur.y).min(q.ll.y).min(q.lr.y);
                        Point { x: min_x, y: min_y }
                    })
                    .filter(|p| p.x.is_finite() && p.y.is_finite())
                    .unwrap_or_else(|| {
                        let b = annotation.bounds().unwrap_or_default();
                        Point { x: b.x0, y: b.y0 }
                    });

                let number = u32::try_from(annotation.xref().unwrap_or(0)).unwrap_or(0);

                if candidates.len() == MAX_ANNOTATIONS_PER_PAGE as usize {
                    truncated = true;
                    break;
                }

                candidates.push(Candidate {
                    annotation,
                    kind,
                    rect,
                    quads,
                    ink,
                    line,
                    vertices,
                    cloudy,
                    start,
                    number,
                    indirect,
                });
            }

            // Same reading order as annotation summary: top-down, left-to-right (within 0.5pt is same row).
            candidates.sort_by(|a, b| {
                a.start
                    .y
                    .round()
                    .total_cmp(&b.start.y.round())
                    .then(a.start.x.total_cmp(&b.start.x))
                    .then(a.number.cmp(&b.number))
            });

            for (item_idx, candidate) in candidates.into_iter().enumerate() {
                if partial_items.len() == MAX_ANNOTATION_SUMMARY_ITEMS as usize {
                    truncated = true;
                    break 'pages;
                }

                let dict = candidate.annotation.object();

                // Name: /NM if 1..=64 valid chars and not duplicate; otherwise pdf-reader-<page>-<index>.
                let raw_nm = read_nm(&dict);
                let name = match raw_nm {
                    Some(nm) if !used_names.contains(&nm) => nm,
                    _ => {
                        let mut generated_name =
                            format!("pdf-reader-{}-{}", page_index + 1, item_idx + 1);
                        if used_names.contains(&generated_name) {
                            let mut suffix = 1;
                            while used_names.contains(&format!("{generated_name}-{suffix}")) {
                                suffix += 1;
                            }
                            generated_name = format!("{generated_name}-{suffix}");
                        }
                        generated_name
                    }
                };
                used_names.insert(name.clone());
                exported_names_by_indirect.insert(candidate.indirect, name.clone());

                let color = parse_color(&dict, "C");
                let interior = parse_color(&dict, "IC");
                let opacity = read_opacity(&dict);
                let width = read_width(&dict);
                let (print, hidden) = read_flags(&dict);
                let icon = match candidate.kind {
                    ExportedAnnotationKind::Stamp => read_stamp_icon(&dict),
                    ExportedAnnotationKind::Note => read_note_icon(&dict),
                    _ => None,
                };
                let contents = read_contents(&dict);

                if let Some(ref c) = contents {
                    if total_bytes + c.len() > MAX_ANNOTATION_SUMMARY_TOTAL_BYTES as usize {
                        truncated = true;
                        break 'pages;
                    }
                    total_bytes += c.len();
                }

                let line_endings = if candidate.kind == ExportedAnnotationKind::Line {
                    read_line_endings(&candidate.annotation)
                } else {
                    None
                };

                let text_style = if candidate.kind == ExportedAnnotationKind::TextBox {
                    read_text_box_style(&candidate.annotation)
                } else {
                    None
                };

                let irt_target = read_irt_target(&dict);

                let item = ExportedAnnotation {
                    kind: candidate.kind,
                    page: page_index,
                    rect: candidate.rect,
                    name,
                    in_reply_to: None,
                    color,
                    interior,
                    opacity,
                    width,
                    print,
                    hidden,
                    icon,
                    contents,
                    quads: candidate.quads,
                    ink: candidate.ink,
                    line: candidate.line,
                    line_endings,
                    text_style,
                    vertices: candidate.vertices,
                    cloudy: candidate.cloudy,
                };

                partial_items.push(PartialItem { item, irt_target });
            }
        }

        // Pass 2: resolve in_reply_to to the exported parent's name.
        for mut p in partial_items {
            if let Some(target_indirect) = p.irt_target {
                p.item.in_reply_to = exported_names_by_indirect.get(&target_indirect).cloned();
            }
            items.push(p.item);
        }

        Ok(ExportedAnnotations {
            items,
            skipped,
            truncated,
        })
    }
}

/// Reads `/Rect` directly from the dictionary, formatted as `[min_x, min_y, max_x, max_y]`.
fn read_rect(dict: &PdfObject) -> Option<[f32; 4]> {
    let rect_obj = dict.get_dict("Rect").ok()??;
    if rect_obj.len().ok()? != 4 {
        return None;
    }
    let x0 = rect_obj.get_array(0).ok()??.as_float().ok()?;
    let y0 = rect_obj.get_array(1).ok()??.as_float().ok()?;
    let x1 = rect_obj.get_array(2).ok()??.as_float().ok()?;
    let y1 = rect_obj.get_array(3).ok()??.as_float().ok()?;
    if ![x0, y0, x1, y1]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_PAGE_SIDE_PT)
    {
        return None;
    }
    Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

/// Reads `/QuadPoints` directly from the dictionary, chunks by 8 into quads.
fn read_quad_points(dict: &PdfObject) -> Option<Vec<[f32; 8]>> {
    let quad_obj = dict.get_dict("QuadPoints").ok()??;
    let len = quad_obj.len().ok()?;
    if len == 0 || len % 8 != 0 || len > (MAX_ANNOTATION_QUADS as usize) * 8 {
        return None;
    }
    let mut quads = Vec::with_capacity(len / 8);
    for chunk in 0..(len / 8) {
        let mut quad = [0.0f32; 8];
        for (i, slot) in quad.iter_mut().enumerate() {
            let val = quad_obj
                .get_array((chunk * 8 + i) as i32)
                .ok()??
                .as_float()
                .ok()?;
            if !val.is_finite() || val.abs() > MAX_PAGE_SIDE_PT {
                return None;
            }
            *slot = val;
        }
        quads.push(quad);
    }
    Some(quads)
}

/// Reads `/InkList` directly from the dictionary.
fn read_ink_list(dict: &PdfObject) -> Option<Vec<Vec<[f32; 2]>>> {
    let ink_obj = dict.get_dict("InkList").ok()??;
    let stroke_count = ink_obj.len().ok()?;
    if stroke_count == 0 || stroke_count > MAX_INK_STROKES as usize {
        return None;
    }
    let mut strokes = Vec::with_capacity(stroke_count);
    let mut total_points = 0usize;
    for s_idx in 0..stroke_count {
        let stroke_obj = ink_obj.get_array(s_idx as i32).ok()??;
        let p_count = stroke_obj.len().ok()?;
        if p_count == 0 || p_count % 2 != 0 {
            return None;
        }
        let pt_count = p_count / 2;
        total_points += pt_count;
        if total_points > MAX_INK_POINTS as usize {
            return None;
        }
        let mut stroke = Vec::with_capacity(pt_count);
        for pt_idx in 0..pt_count {
            let x = stroke_obj
                .get_array((pt_idx * 2) as i32)
                .ok()??
                .as_float()
                .ok()?;
            let y = stroke_obj
                .get_array((pt_idx * 2 + 1) as i32)
                .ok()??
                .as_float()
                .ok()?;
            if !x.is_finite()
                || !y.is_finite()
                || x.abs() > MAX_PAGE_SIDE_PT
                || y.abs() > MAX_PAGE_SIDE_PT
            {
                return None;
            }
            stroke.push([x, y]);
        }
        strokes.push(stroke);
    }
    Some(strokes)
}

/// Reads `/L` line endpoints `[x1, y1, x2, y2]`.
fn read_line(dict: &PdfObject) -> Option<[f32; 4]> {
    let line_obj = dict.get_dict("L").ok()??;
    if line_obj.len().ok()? != 4 {
        return None;
    }
    let x1 = line_obj.get_array(0).ok()??.as_float().ok()?;
    let y1 = line_obj.get_array(1).ok()??.as_float().ok()?;
    let x2 = line_obj.get_array(2).ok()??.as_float().ok()?;
    let y2 = line_obj.get_array(3).ok()??.as_float().ok()?;
    if ![x1, y1, x2, y2]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_PAGE_SIDE_PT)
    {
        return None;
    }
    Some([x1, y1, x2, y2])
}

/// Reads `/Vertices` directly from the annotation dictionary in PDF user space (ISO 19444-1 § 6.5.31).
fn read_vertices(dict: &PdfObject, min: usize) -> Option<Vec<[f32; 2]>> {
    let obj = dict.get_dict("Vertices").ok()??;
    let len = obj.len().ok()?;
    if len == 0 || len % 2 != 0 {
        return None;
    }
    let count = len / 2;
    if count < min || count > MAX_SHAPE_VERTICES as usize {
        return None;
    }
    let mut vertices = Vec::with_capacity(count);
    for i in 0..count {
        let x = obj.get_array((i * 2) as i32).ok()??.as_float().ok()?;
        let y = obj.get_array((i * 2 + 1) as i32).ok()??.as_float().ok()?;
        if !x.is_finite()
            || !y.is_finite()
            || x.abs() > MAX_PAGE_SIDE_PT
            || y.abs() > MAX_PAGE_SIDE_PT
        {
            return None;
        }
        vertices.push([x, y]);
    }
    Some(vertices)
}

fn parse_le(item: PdfObject) -> ExportedLineEnding {
    if let Ok(name_bytes) = item.as_name() {
        match name_bytes.as_slice() {
            b"OpenArrow" => ExportedLineEnding::OpenArrow,
            b"ClosedArrow" => ExportedLineEnding::ClosedArrow,
            _ => ExportedLineEnding::None,
        }
    } else if let Ok(s) = item.as_string() {
        match s.as_str() {
            "OpenArrow" => ExportedLineEnding::OpenArrow,
            "ClosedArrow" => ExportedLineEnding::ClosedArrow,
            _ => ExportedLineEnding::None,
        }
    } else {
        ExportedLineEnding::None
    }
}

/// Reads line endings from `/LE` or fallback to annotation styles.
fn read_line_endings(annotation: &PdfAnnotation) -> Option<[ExportedLineEnding; 2]> {
    let dict = annotation.object();
    if let Some([le0, le1]) = dict
        .get_dict("LE")
        .ok()
        .flatten()
        .and_then(|le_obj| {
            if le_obj.len().ok() == Some(2) {
                Some((le_obj.get_array(0).ok()??, le_obj.get_array(1).ok()??))
            } else {
                None
            }
        })
        .map(|(a, b)| [parse_le(a), parse_le(b)])
    {
        return Some([le0, le1]);
    }
    if let Ok((start, end)) = annotation.line_ending_styles() {
        let conv = |s: mupdf::pdf::LineEndingStyle| match s {
            mupdf::pdf::LineEndingStyle::OpenArrow => ExportedLineEnding::OpenArrow,
            mupdf::pdf::LineEndingStyle::ClosedArrow => ExportedLineEnding::ClosedArrow,
            _ => ExportedLineEnding::None,
        };
        if start != mupdf::pdf::LineEndingStyle::None || end != mupdf::pdf::LineEndingStyle::None {
            return Some([conv(start), conv(end)]);
        }
    }
    None
}

/// Reads color `/C` or `/IC`:
/// 0 values: None
/// 1 value: grayscale
/// 3 values: RGB
/// 4 values: CMYK -> RGB via 255 * (1 - c) * (1 - k)
fn parse_color(dict: &PdfObject, key: &str) -> Option<[u8; 3]> {
    let color_obj = dict.get_dict(key).ok()??;
    let len = color_obj.len().ok()?;
    match len {
        0 => None,
        1 => {
            let g = color_obj.get_array(0).ok()??.as_float().ok()?;
            if !g.is_finite() {
                return None;
            }
            let val = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
            Some([val, val, val])
        }
        3 => {
            let r = color_obj.get_array(0).ok()??.as_float().ok()?;
            let g = color_obj.get_array(1).ok()??.as_float().ok()?;
            let b = color_obj.get_array(2).ok()??.as_float().ok()?;
            if !r.is_finite() || !g.is_finite() || !b.is_finite() {
                return None;
            }
            let r_u8 = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
            let g_u8 = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
            let b_u8 = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
            Some([r_u8, g_u8, b_u8])
        }
        4 => {
            let c = color_obj.get_array(0).ok()??.as_float().ok()?;
            let m = color_obj.get_array(1).ok()??.as_float().ok()?;
            let y = color_obj.get_array(2).ok()??.as_float().ok()?;
            let k = color_obj.get_array(3).ok()??.as_float().ok()?;
            if !c.is_finite() || !m.is_finite() || !y.is_finite() || !k.is_finite() {
                return None;
            }
            let c = c.clamp(0.0, 1.0);
            let m = m.clamp(0.0, 1.0);
            let y = y.clamp(0.0, 1.0);
            let k = k.clamp(0.0, 1.0);
            let r = (255.0 * (1.0 - c) * (1.0 - k)).round() as u8;
            let g = (255.0 * (1.0 - m) * (1.0 - k)).round() as u8;
            let b = (255.0 * (1.0 - y) * (1.0 - k)).round() as u8;
            Some([r, g, b])
        }
        _ => None,
    }
}

/// Reads `/CA` opacity.
fn read_opacity(dict: &PdfObject) -> Option<f32> {
    let ca_obj = dict.get_dict("CA").ok()??;
    let ca = ca_obj.as_float().ok()?;
    if ca.is_finite() && (0.0..=1.0).contains(&ca) {
        Some(ca)
    } else {
        None
    }
}

/// Reads border width from `/BS /W` or `/Border` 3rd element.
fn read_width(dict: &PdfObject) -> Option<f32> {
    if let Some(w) = dict
        .get_dict("BS")
        .ok()
        .flatten()
        .and_then(|bs| bs.get_dict("W").ok().flatten())
        .and_then(|w| w.as_float().ok())
        .filter(|w| w.is_finite() && (0.0..=100.0).contains(w))
    {
        return Some(w);
    }
    if let Some(w) = dict
        .get_dict("Border")
        .ok()
        .flatten()
        .filter(|border| border.len().unwrap_or(0) >= 3)
        .and_then(|border| border.get_array(2).ok().flatten())
        .and_then(|w| w.as_float().ok())
        .filter(|w| w.is_finite() && (0.0..=100.0).contains(w))
    {
        return Some(w);
    }
    None
}

/// Reads `/F` flags: print (bit 4) and hidden (bit 2).
fn read_flags(dict: &PdfObject) -> (bool, bool) {
    let f = dict
        .get_dict("F")
        .ok()
        .flatten()
        .and_then(|obj| obj.as_int().ok())
        .unwrap_or(0);
    let print = (f & 4) != 0;
    let hidden = (f & 2) != 0;
    (print, hidden)
}

/// Reads and cleans `/Contents`.
fn read_contents(dict: &PdfObject) -> Option<String> {
    let contents_obj = dict.get_dict("Contents").ok()??;
    let text = contents_obj.as_string().ok()?;
    let cleaned = clean_note_text(&text, MAX_ANNOTATION_SUMMARY_TEXT_BYTES as usize);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Reads stamp name and ensures it is one of the 14 standard stamps.
fn read_stamp_icon(dict: &PdfObject) -> Option<String> {
    let name_obj = dict.get_dict("Name").ok()??;
    let name = name_obj
        .as_name()
        .ok()
        .and_then(|n| String::from_utf8(n).ok())
        .or_else(|| name_obj.as_string().ok())?;
    if STANDARD_STAMPS.contains(&name.as_str()) {
        Some(name)
    } else {
        None
    }
}

/// Reads note icon: 1..=32 ASCII alphanumeric.
fn read_note_icon(dict: &PdfObject) -> Option<String> {
    let name_obj = dict.get_dict("Name").ok()??;
    let name = name_obj
        .as_name()
        .ok()
        .and_then(|n| String::from_utf8(n).ok())
        .or_else(|| name_obj.as_string().ok())?;
    if (1..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(name)
    } else {
        None
    }
}

/// Reads `/NM` if valid: 1..=64 characters of `[A-Za-z0-9._:-]`.
fn read_nm(dict: &PdfObject) -> Option<String> {
    let nm_obj = dict.get_dict("NM").ok()??;
    let nm = nm_obj.as_string().ok().or_else(|| {
        nm_obj
            .as_name()
            .ok()
            .and_then(|n| String::from_utf8(n).ok())
    })?;
    if (1..=64).contains(&nm.len())
        && nm
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == ':' || c == '-')
    {
        Some(nm)
    } else {
        None
    }
}

/// Reads the indirect object reference from `/IRT`.
fn read_irt_target(dict: &PdfObject) -> Option<i32> {
    let irt_obj = dict.get_dict("IRT").ok()??;
    irt_obj.as_indirect().ok()
}

/// Reads text box style using `text_box.rs` parser.
fn read_text_box_style(annotation: &PdfAnnotation) -> Option<ExportedTextStyle> {
    let entry = annotation
        .object()
        .get_dict(super::text_box::pdf::KEY)
        .ok()??;
    let (_, _, style) = super::text_box::pdf::read_source_text_box_info(&entry).ok()?;
    let justification = match style.align {
        TextAlign::Left => 0,
        TextAlign::Center => 1,
        TextAlign::Right => 2,
    };
    Some(ExportedTextStyle {
        size: style.size,
        color: [style.color.r, style.color.g, style.color.b],
        justification,
    })
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{
        HighlightColor, HighlightMark, InkColor, InkWidth, Point, Quad, Rect, Rgb, Shape,
        StampName, TextAlign, TextBoxStyle, TextMarkKind,
    };
    use ipc_contract::worker::{ExportedAnnotationKind, ExportedLineEnding};
    use mupdf::pdf::PdfObject;

    use crate::engine::text_box::prepare_text_box;
    use crate::engine::{PdfDocument, TextBoxFont};

    fn corpus(rel: &str) -> Vec<u8> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests")
            .join("corpus")
            .join(rel);
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
    }

    fn droid() -> &'static [u8] {
        mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
            .expect("the bundled font")
            .data
    }

    #[test]
    fn annotation_export_extracts_all_kinds_and_skips_image_stamps() {
        let mut doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).expect("open");

        // 1. Highlight
        let quad_hl = Quad {
            ul: Point { x: 72.0, y: 100.0 },
            ur: Point { x: 172.0, y: 100.0 },
            ll: Point { x: 72.0, y: 120.0 },
            lr: Point { x: 172.0, y: 120.0 },
        };
        doc.add_text_marks(
            TextMarkKind::Highlight,
            &[HighlightMark {
                page: 0,
                quads: vec![quad_hl],
            }],
            HighlightColor::Yellow,
        )
        .expect("highlight");

        // 2. Underline
        let quad_ul = Quad {
            ul: Point { x: 72.0, y: 130.0 },
            ur: Point { x: 172.0, y: 130.0 },
            ll: Point { x: 72.0, y: 150.0 },
            lr: Point { x: 172.0, y: 150.0 },
        };
        doc.add_text_marks(
            TextMarkKind::Underline,
            &[HighlightMark {
                page: 0,
                quads: vec![quad_ul],
            }],
            HighlightColor::Blue,
        )
        .expect("underline");

        // 3. Parent Note
        doc.add_note(0, Point { x: 50.0, y: 50.0 }, "Parent note")
            .expect("note");

        // 4. Reply Note with /IRT pointing to Parent Note
        doc.add_note(0, Point { x: 50.0, y: 70.0 }, "Reply note")
            .expect("reply note");
        {
            let page = doc.pdf_page(0).expect("page");
            let mut annots: Vec<_> = page.annotations().collect();
            let parent = annots
                .iter()
                .find(|a| a.contents().ok().flatten() == Some("Parent note"))
                .expect("parent annot");
            let parent_obj = parent.object();
            let reply = annots
                .iter_mut()
                .find(|a| a.contents().ok().flatten() == Some("Reply note"))
                .expect("reply annot");
            reply.object().dict_put("IRT", parent_obj).expect("put IRT");
            reply.update().expect("update reply");
        }

        // 5. Ink drawing
        doc.add_ink(
            0,
            &[vec![Point { x: 10.0, y: 10.0 }, Point { x: 20.0, y: 20.0 }]],
            InkColor::Red,
            InkWidth::Thin,
        )
        .expect("ink");

        // 6. Standard Stamp (Approved)
        doc.add_stamp(
            0,
            Rect {
                x0: 100.0,
                y0: 200.0,
                x1: 200.0,
                y1: 250.0,
            },
            StampName::Approved,
        )
        .expect("stamp");

        // 7. Custom Image Stamp (must NOT be exported, counted as skipped)
        let jpeg = corpus("images/stamp-exif.jpg");
        let pic = super::super::stamp_image::prepare_stamp_picture(&jpeg).expect("picture");
        doc.add_image_stamp(
            0,
            Rect {
                x0: 210.0,
                y0: 200.0,
                x1: 300.0,
                y1: 250.0,
            },
            &pic.png,
        )
        .expect("image stamp");

        // 8. Rectangle
        doc.add_shape(
            0,
            &Shape::Rectangle {
                rect: Rect {
                    x0: 10.0,
                    y0: 300.0,
                    x1: 50.0,
                    y1: 350.0,
                },
                cloudy: false,
            },
            InkColor::Black,
            InkWidth::Thin,
        )
        .expect("rectangle");

        // 9. Ellipse
        doc.add_shape(
            0,
            &Shape::Ellipse {
                rect: Rect {
                    x0: 60.0,
                    y0: 300.0,
                    x1: 100.0,
                    y1: 350.0,
                },
            },
            InkColor::Green,
            InkWidth::Medium,
        )
        .expect("ellipse");

        // 10. Line
        doc.add_shape(
            0,
            &Shape::Line {
                from: Point { x: 10.0, y: 400.0 },
                to: Point { x: 50.0, y: 400.0 },
                arrow: false,
            },
            InkColor::Blue,
            InkWidth::Thin,
        )
        .expect("line");

        // 11. Arrow (Line with OpenArrow)
        doc.add_shape(
            0,
            &Shape::Line {
                from: Point { x: 60.0, y: 400.0 },
                to: Point { x: 100.0, y: 400.0 },
                arrow: true,
            },
            InkColor::Red,
            InkWidth::Thick,
        )
        .expect("arrow");

        // 12. TextBox
        let font_data = droid();
        let tb_font = TextBoxFont {
            bytes: font_data,
            face: 0,
            name: "Droid Sans Fallback",
        };
        let model = prepare_text_box(&tb_font, "Text Box Content").expect("model");
        let tb_style = TextBoxStyle {
            size: 14.0,
            color: Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
            align: TextAlign::Center,
        };
        doc.place_text_box(
            0,
            Rect {
                x0: 50.0,
                y0: 450.0,
                x1: 250.0,
                y1: 500.0,
            },
            &model,
            &tb_style,
        )
        .expect("place text box");

        // Run export
        let exported = doc.annotation_export().expect("export");

        // Check: skipped is 1 (the image stamp)
        assert_eq!(exported.skipped, 1);
        assert!(!exported.truncated);

        // Check names are all unique
        let mut names = std::collections::HashSet::new();
        for item in &exported.items {
            assert!(names.insert(&item.name), "duplicate name: {}", item.name);
        }

        // Verify reply note points to parent note's exported name
        let parent_item = exported
            .items
            .iter()
            .find(|i| i.contents.as_deref() == Some("Parent note"))
            .expect("parent item");
        let reply_item = exported
            .items
            .iter()
            .find(|i| i.contents.as_deref() == Some("Reply note"))
            .expect("reply item");
        assert_eq!(
            reply_item.in_reply_to.as_deref(),
            Some(parent_item.name.as_str())
        );

        // Check Highlight
        let hl = exported
            .items
            .iter()
            .find(|i| i.kind == ExportedAnnotationKind::Highlight)
            .expect("highlight");
        assert_eq!(hl.quads.len(), 1);

        // Check Underline
        let ul = exported
            .items
            .iter()
            .find(|i| i.kind == ExportedAnnotationKind::Underline)
            .expect("underline");
        assert_eq!(ul.quads.len(), 1);

        // Check Ink
        let ink = exported
            .items
            .iter()
            .find(|i| i.kind == ExportedAnnotationKind::Ink)
            .expect("ink");
        assert_eq!(ink.ink.len(), 1);
        assert_eq!(ink.ink[0].len(), 2);

        // Check Stamp
        let stamp = exported
            .items
            .iter()
            .find(|i| i.kind == ExportedAnnotationKind::Stamp)
            .expect("stamp");
        assert_eq!(stamp.icon.as_deref(), Some("Approved"));

        // Check Rectangle
        assert!(
            exported
                .items
                .iter()
                .any(|i| i.kind == ExportedAnnotationKind::Rectangle)
        );

        // Check Ellipse
        assert!(
            exported
                .items
                .iter()
                .any(|i| i.kind == ExportedAnnotationKind::Ellipse)
        );

        // Check Line
        let line_item = exported
            .items
            .iter()
            .find(|i| {
                i.kind == ExportedAnnotationKind::Line
                    && i.line_endings
                        .as_ref()
                        .is_none_or(|le| le[1] == ExportedLineEnding::None)
            })
            .expect("plain line");
        assert!(line_item.line.is_some());

        // Check Arrow (Line with OpenArrow)
        let arrow_item = exported
            .items
            .iter()
            .find(|i| {
                i.kind == ExportedAnnotationKind::Line
                    && i.line_endings
                        == Some([ExportedLineEnding::None, ExportedLineEnding::OpenArrow])
            })
            .expect("arrow line");
        assert!(arrow_item.line.is_some());

        // Check TextBox
        let tb = exported
            .items
            .iter()
            .find(|i| i.kind == ExportedAnnotationKind::TextBox)
            .expect("text box");
        assert_eq!(tb.contents.as_deref(), Some("Text Box Content"));
        let style = tb.text_style.expect("text style");
        assert_eq!(style.size, 14.0);
        assert_eq!(style.color, [10, 20, 30]);
        assert_eq!(style.justification, 1); // Center
    }

    #[test]
    fn rotated_page_and_shifted_mediabox_keeps_raw_file_rect() {
        let mut doc = PdfDocument::from_bytes(&corpus("benign/rotated-page.pdf")).expect("open");
        // Page 1 has /Rotate 90. Shift MediaBox so it's not at origin.
        {
            let page = doc.pdf_page(1).expect("page");
            let mut mbox = doc.doc.new_array().expect("array");
            for v in [50.0, 50.0, 662.0, 842.0] {
                mbox.array_push(PdfObject::new_real(v).expect("real"))
                    .expect("push");
            }
            page.object()
                .dict_put("MediaBox", mbox)
                .expect("put MediaBox");
        }
        doc.add_note(1, Point { x: 100.0, y: 100.0 }, "Note on rotated page")
            .expect("add note");

        // Read raw rect from annotation dictionary
        let page = doc.pdf_page(1).expect("page");
        let annot = page.annotations().next().expect("annot");
        let raw_rect = super::read_rect(&annot.object()).expect("raw rect");

        let exported = doc.annotation_export().expect("export");
        let item = exported
            .items
            .iter()
            .find(|i| i.contents.as_deref() == Some("Note on rotated page"))
            .expect("item");
        assert_eq!(item.rect, raw_rect);
    }

    #[test]
    fn annotation_export_truncates_when_page_limit_is_exceeded() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).expect("open");
        let mut page = doc.pdf_page(0).expect("page");
        for _ in 0..(ipc_contract::limits::MAX_ANNOTATIONS_PER_PAGE + 5) {
            let mut annot = page
                .create_annotation(mupdf::pdf::PdfAnnotationType::Text)
                .expect("create annot");
            let mut rect = doc.doc.new_array().expect("rect");
            for v in [10.0, 10.0, 20.0, 20.0] {
                rect.array_push(PdfObject::new_real(v).expect("real"))
                    .expect("push");
            }
            annot.object().dict_put("Rect", rect).expect("put");
            annot.update().expect("update");
        }

        let exported = doc.annotation_export().expect("export");
        assert!(exported.truncated);
        assert_eq!(
            exported.items.len(),
            ipc_contract::limits::MAX_ANNOTATIONS_PER_PAGE as usize
        );
    }
}
