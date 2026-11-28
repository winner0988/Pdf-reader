// #447 的 POC，不合併。
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use ipc_contract::limits::{
    MAX_ANNOTATION_QUADS, MAX_ANNOTATIONS_PER_PAGE, MAX_DISPLAY_NAME_BYTES, MAX_INK_POINTS,
    MAX_INK_STROKES, MAX_NOTE_TEXT_BYTES, MAX_PAGE_SIDE_PT, MAX_PREPARE_TEXT_BOXES,
    MAX_SOURCE_ANNOTATIONS, MAX_SOURCE_TEXT_BOXES, MAX_TEXT_BOX_BYTES, MAX_TEXT_BOX_SIZE,
    MIN_ANNOTATION_SIDE_PT, MIN_TEXT_BOX_SIZE,
};
use ipc_contract::text::{clean_display_text, clean_field_text, clean_note_text};
use ipc_contract::types::{
    Edit, HighlightColor, HighlightMark, InkColor, InkWidth, Point, Quad, Rect, Rgb, Shape,
    StampName, TextAlign, TextBoxModelId, TextBoxStyle, TextMarkKind,
};
use ipc_contract::validate::{
    Validate, check_annotation_rect, check_note_text, check_text_box_text,
};
use ipc_contract::worker::{PreparedTextBoxModel, TextBoxModel};
use mupdf::Document as MuPdfGenericDocument;
use mupdf::pdf::PdfDocument as MuPdfDocument;
use pdf_worker::engine::{PdfDocument, TextBoxFont, prepare_text_boxes};
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::events::attributes::AttrError;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;

pub const MAX_XFDF_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_XFDF_DEPTH: usize = 32;
pub const MAX_XFDF_IMPORT_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_RICHTEXT_BYTES: usize = 64 * 1024;
pub const COLOR_TOLERANCE: f32 = 0.02;

pub const FALLBACK_FONT_FAMILIES: [&str; 5] = [
    "Microsoft JhengHei",
    "Microsoft YaHei",
    "Yu Gothic",
    "Segoe UI",
    "Arial",
];

const XFDF_NAMESPACE: &str = "http://ns.adobe.com/xfdf/";
const XHTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

#[derive(Debug, Clone)]
pub struct PageGeometry {
    pub bounds: mupdf::Rect,
    pub ctm: mupdf::Matrix,
    pub existing_annots: u32,
}

pub fn collect_page_geometries(pdf_bytes: &[u8]) -> Result<Vec<PageGeometry>, String> {
    let generic = MuPdfGenericDocument::from_bytes(pdf_bytes, "application/pdf")
        .map_err(|e| e.to_string())?;
    let count = generic.page_count().map_err(|e| e.to_string())?;
    let mut bounds_list = Vec::with_capacity(count.max(0) as usize);
    for i in 0..count {
        let page = generic.load_page(i).map_err(|e| e.to_string())?;
        let bounds = page.bounds().map_err(|e| e.to_string())?;
        bounds_list.push(bounds);
    }
    let pdf = MuPdfDocument::try_from(generic).map_err(|e| e.to_string())?;
    let mut pages = Vec::with_capacity(bounds_list.len());
    for (i, bounds) in bounds_list.into_iter().enumerate() {
        let page = pdf
            .load_pdf_page(i32::try_from(i).unwrap_or(i32::MAX))
            .map_err(|e| e.to_string())?;
        let ctm = page.object().page_ctm().map_err(|e| e.to_string())?;
        let existing_annots =
            u32::try_from(page.annotations().count().min(u32::MAX as usize)).unwrap_or(u32::MAX);
        pages.push(PageGeometry {
            bounds,
            ctm,
            existing_annots,
        });
    }
    Ok(pages)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    UnsupportedKind,
    Reply,
    PageOutOfRange,
    InvalidGeometryOrFormat,
    ExceedsLimit,
    FontUnavailable,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::UnsupportedKind => "UnsupportedKind",
            SkipReason::Reply => "Reply",
            SkipReason::PageOutOfRange => "PageOutOfRange",
            SkipReason::InvalidGeometryOrFormat => "InvalidGeometryOrFormat",
            SkipReason::ExceedsLimit => "ExceedsLimit",
            SkipReason::FontUnavailable => "FontUnavailable",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkipCounts {
    pub unsupported_kind: usize,
    pub reply: usize,
    pub page_out_of_range: usize,
    pub invalid_geometry_or_format: usize,
    pub exceeds_limit: usize,
    pub font_unavailable: usize,
}

impl SkipCounts {
    pub fn record(&mut self, reason: SkipReason) {
        match reason {
            SkipReason::UnsupportedKind => self.unsupported_kind += 1,
            SkipReason::Reply => self.reply += 1,
            SkipReason::PageOutOfRange => self.page_out_of_range += 1,
            SkipReason::InvalidGeometryOrFormat => self.invalid_geometry_or_format += 1,
            SkipReason::ExceedsLimit => self.exceeds_limit += 1,
            SkipReason::FontUnavailable => self.font_unavailable += 1,
        }
    }

    pub fn decrement(&mut self, reason: SkipReason) {
        match reason {
            SkipReason::UnsupportedKind => {
                self.unsupported_kind = self.unsupported_kind.saturating_sub(1);
            }
            SkipReason::Reply => self.reply = self.reply.saturating_sub(1),
            SkipReason::PageOutOfRange => {
                self.page_out_of_range = self.page_out_of_range.saturating_sub(1);
            }
            SkipReason::InvalidGeometryOrFormat => {
                self.invalid_geometry_or_format = self.invalid_geometry_or_format.saturating_sub(1);
            }
            SkipReason::ExceedsLimit => self.exceeds_limit = self.exceeds_limit.saturating_sub(1),
            SkipReason::FontUnavailable => {
                self.font_unavailable = self.font_unavailable.saturating_sub(1);
            }
        }
    }

    pub fn count(&self, reason: SkipReason) -> usize {
        match reason {
            SkipReason::UnsupportedKind => self.unsupported_kind,
            SkipReason::Reply => self.reply,
            SkipReason::PageOutOfRange => self.page_out_of_range,
            SkipReason::InvalidGeometryOrFormat => self.invalid_geometry_or_format,
            SkipReason::ExceedsLimit => self.exceeds_limit,
            SkipReason::FontUnavailable => self.font_unavailable,
        }
    }

    pub fn total(&self) -> usize {
        self.unsupported_kind
            + self.reply
            + self.page_out_of_range
            + self.invalid_geometry_or_format
            + self.exceeds_limit
            + self.font_unavailable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XfdfError {
    TooLargeXml { bytes: usize, max: usize },
    UnsupportedEncoding(String),
    InvalidUtf8,
    DocTypeForbidden,
    EntityForbidden(String),
    InvalidXmlChar(u32),
    DepthExceeded { depth: usize, max: usize },
    TooManyAnnotations { count: usize, max: usize },
    TooMuchImportData { bytes: usize, max: usize },
    MalformedXml(String),
    WrongRoot(String),
    WrongNamespace(String),
}

impl fmt::Display for XfdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XfdfError::TooLargeXml { bytes, max } => {
                write!(f, "XFDF input size {bytes} bytes exceeds {max} byte limit")
            }
            XfdfError::UnsupportedEncoding(enc) => {
                write!(f, "unsupported XML encoding or BOM: {enc}")
            }
            XfdfError::InvalidUtf8 => write!(f, "XFDF bytes are not valid UTF-8"),
            XfdfError::DocTypeForbidden => write!(f, "DOCTYPE declarations are forbidden"),
            XfdfError::EntityForbidden(name) => {
                write!(f, "undeclared or forbidden XML entity: {name}")
            }
            XfdfError::InvalidXmlChar(cp) => {
                write!(f, "disallowed XML 1.0 character U+{cp:04X}")
            }
            XfdfError::DepthExceeded { depth, max } => {
                write!(f, "XML nesting depth {depth} exceeds max depth {max}")
            }
            XfdfError::TooManyAnnotations { count, max } => {
                write!(f, "annotation count {count} exceeds max {max}")
            }
            XfdfError::TooMuchImportData { bytes, max } => {
                write!(
                    f,
                    "projected journal import size {bytes} bytes exceeds {max} byte limit"
                )
            }
            XfdfError::MalformedXml(msg) => write!(f, "malformed XML: {msg}"),
            XfdfError::WrongRoot(name) => write!(f, "expected <xfdf> root, found <{name}>"),
            XfdfError::WrongNamespace(ns) => write!(f, "unsupported XFDF XML namespace: {ns}"),
        }
    }
}

impl std::error::Error for XfdfError {}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportedItem {
    Highlight {
        mark_kind: TextMarkKind,
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
        icon_raw: String,
    },
    Shape {
        page: u32,
        shape: Shape,
        color: InkColor,
        width: InkWidth,
    },
    FreeText {
        page: u32,
        rect: Rect,
        text: String,
        font_family: Option<String>,
        style: TextBoxStyle,
    },
}

impl ImportedItem {
    pub fn page(&self) -> u32 {
        match self {
            ImportedItem::Highlight { page, .. }
            | ImportedItem::Note { page, .. }
            | ImportedItem::Ink { page, .. }
            | ImportedItem::Stamp { page, .. }
            | ImportedItem::Shape { page, .. }
            | ImportedItem::FreeText { page, .. } => *page,
        }
    }

    pub fn estimated_journal_bytes(&self) -> usize {
        match self {
            ImportedItem::Highlight { quads, .. } => 96 + quads.len().saturating_mul(72),
            ImportedItem::Note { text, .. } => 64 + text.len(),
            ImportedItem::Ink { strokes, .. } => {
                let pts: usize = strokes.iter().map(Vec::len).sum();
                72 + strokes.len().saturating_mul(8) + pts.saturating_mul(32)
            }
            ImportedItem::Stamp { .. } => 104,
            ImportedItem::Shape { .. } => 128,
            ImportedItem::FreeText { text, .. } => 160 + text.len(),
        }
    }

    pub fn to_edit(&self, model_id: Option<TextBoxModelId>) -> Edit {
        match self {
            ImportedItem::Highlight {
                mark_kind,
                page,
                quads,
                color,
            } => Edit::AddHighlight {
                mark_kind: *mark_kind,
                marks: vec![HighlightMark {
                    page: *page,
                    quads: quads.clone(),
                }],
                color: *color,
            },
            ImportedItem::Note { page, at, text } => Edit::AddNote {
                page: *page,
                at: *at,
                text: text.clone(),
            },
            ImportedItem::Ink {
                page,
                strokes,
                color,
                width,
            } => Edit::AddInk {
                page: *page,
                strokes: strokes.clone(),
                color: *color,
                width: *width,
            },
            ImportedItem::Stamp {
                page, rect, stamp, ..
            } => Edit::AddStamp {
                page: *page,
                rect: *rect,
                stamp: *stamp,
            },
            ImportedItem::Shape {
                page,
                shape,
                color,
                width,
            } => Edit::AddShape {
                page: *page,
                shape: shape.clone(),
                color: *color,
                width: *width,
            },
            ImportedItem::FreeText {
                page, rect, style, ..
            } => Edit::AddTextBox {
                page: *page,
                rect: *rect,
                model: model_id.expect("TextBoxModelId required for FreeText"),
                style: *style,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationOutcome {
    Imported(String),
    Skipped(SkipReason, String),
}

#[derive(Debug, Clone)]
pub struct ParsedAnnotationSummary {
    pub index: usize,
    pub tag: String,
    pub page_attr: Option<String>,
    pub outcome: AnnotationOutcome,
    pub converted_summary: String,
    pub richtext_plain: Option<String>,
    pub stamp_icon: Option<String>,
    pub style_adjusted: bool,
    pub item_idx: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct XfdfReport {
    pub has_doctype: bool,
    pub xml_encoding: Option<String>,
    pub used_namespace: bool,
    pub f_href: Option<String>,
    pub element_counts: BTreeMap<String, usize>,
    pub attribute_counts: BTreeMap<String, usize>,
    pub total_annots_seen: usize,
    pub imported_count: usize,
    pub style_adjusted_count: usize,
    pub skips: SkipCounts,
    pub estimated_journal_bytes: usize,
    pub per_annotation: Vec<ParsedAnnotationSummary>,
    pub items: Vec<ImportedItem>,
}

#[derive(Debug, Clone)]
pub struct FontData {
    pub bytes: Vec<u8>,
    pub face: u32,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedTextBox {
    pub item_index: usize,
    pub model_id: TextBoxModelId,
    pub model: TextBoxModel,
}

pub fn parse_xfdf(
    raw: &[u8],
    pages: &[PageGeometry],
    max_import_bytes: usize,
) -> Result<XfdfReport, XfdfError> {
    if raw.len() > MAX_XFDF_BYTES {
        return Err(XfdfError::TooLargeXml {
            bytes: raw.len(),
            max: MAX_XFDF_BYTES,
        });
    }

    let utf8_bytes = strip_and_check_bom(raw)?;
    let text = std::str::from_utf8(utf8_bytes).map_err(|_| XfdfError::InvalidUtf8)?;

    let mut reader = NsReader::from_str(text);
    {
        let cfg = reader.config_mut();
        cfg.check_comments = true;
        cfg.check_end_names = true;
        cfg.allow_unmatched_ends = false;
        cfg.allow_dangling_amp = false;
        cfg.trim_text(false);
        cfg.expand_empty_elements = false;
    }

    let mut report = XfdfReport {
        has_doctype: false,
        xml_encoding: None,
        used_namespace: false,
        f_href: None,
        element_counts: BTreeMap::new(),
        attribute_counts: BTreeMap::new(),
        total_annots_seen: 0,
        imported_count: 0,
        style_adjusted_count: 0,
        skips: SkipCounts::default(),
        estimated_journal_bytes: 0,
        per_annotation: Vec::new(),
        items: Vec::new(),
    };

    let mut imported_on_page = vec![0u32; pages.len()];
    let mut imported_text_boxes: u32 = 0;

    let mut seen_decl = false;
    let mut seen_root = false;
    let mut stack: Vec<Frame> = Vec::new();
    let mut current_annot: Option<AnnotBuilder> = None;

    loop {
        let (ns, event) = reader.read_resolved_event().map_err(map_xml_err)?;
        match event {
            Event::Decl(decl) => {
                if seen_root || seen_decl {
                    return Err(XfdfError::MalformedXml(
                        "unexpected XML declaration position".into(),
                    ));
                }
                seen_decl = true;
                validate_xml_chars(decl.as_ref())?;
                let ver = decl.xml_version().map_err(map_xml_err)?;
                if ver != XmlVersion::Explicit1_0 {
                    return Err(XfdfError::UnsupportedEncoding(
                        "only XML 1.0 is supported".into(),
                    ));
                }
                if let Some(enc_res) = decl.encoding() {
                    let enc = enc_res.map_err(|e| map_xml_err(e.into()))?;
                    let enc_str = enc.into_owned();
                    validate_xml_chars(&enc_str)?;
                    if !enc_str.eq_ignore_ascii_case("UTF-8") {
                        return Err(XfdfError::UnsupportedEncoding(enc_str));
                    }
                    report.xml_encoding = Some(enc_str);
                }
            }
            Event::DocType(_) => {
                report.has_doctype = true;
                return Err(XfdfError::DocTypeForbidden);
            }
            Event::PI(pi) => {
                validate_xml_chars(pi.as_ref())?;
            }
            Event::Comment(comment) => {
                validate_xml_chars(comment.as_ref())?;
            }
            Event::Text(text_ev) => {
                let raw_str: &str = text_ev.as_ref();
                validate_xml_chars(raw_str)?;
                if stack.is_empty() {
                    if !raw_str
                        .chars()
                        .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
                    {
                        return Err(XfdfError::MalformedXml(
                            "non-whitespace character data outside root element".into(),
                        ));
                    }
                } else if let Some(builder) = current_annot.as_mut() {
                    builder.push_text(raw_str);
                }
            }
            Event::CData(cdata) => {
                let raw_str: &str = cdata.as_ref();
                validate_xml_chars(raw_str)?;
                if stack.is_empty() {
                    return Err(XfdfError::MalformedXml(
                        "CDATA section outside root element".into(),
                    ));
                }
                if let Some(builder) = current_annot.as_mut() {
                    builder.push_text(raw_str);
                }
            }
            Event::GeneralRef(gen_ref) => {
                if stack.is_empty() {
                    return Err(XfdfError::MalformedXml(
                        "character/entity reference outside root element".into(),
                    ));
                }
                let ch = resolve_general_ref(&gen_ref)?;
                if let Some(builder) = current_annot.as_mut() {
                    let mut buf = [0u8; 4];
                    builder.push_text(ch.encode_utf8(&mut buf));
                }
            }
            Event::Start(start) => {
                let depth = stack.len() + 1;
                if depth > MAX_XFDF_DEPTH {
                    return Err(XfdfError::DepthExceeded {
                        depth,
                        max: MAX_XFDF_DEPTH,
                    });
                }
                let local = decode_local_name(start.local_name().as_ref())?;
                let attrs = collect_checked_attributes(&start, &mut report)?;
                *report.element_counts.entry(local.clone()).or_insert(0) += 1;

                if stack.is_empty() {
                    if seen_root {
                        return Err(XfdfError::MalformedXml("multiple root elements".into()));
                    }
                    seen_root = true;
                    check_xfdf_root(&local, &ns, &mut report)?;
                    stack.push(Frame {
                        name: local,
                        role: FrameRole::Root,
                    });
                } else {
                    let role = enter_element(
                        &stack,
                        &local,
                        &ns,
                        &attrs,
                        &mut report,
                        &mut current_annot,
                    )?;
                    stack.push(Frame { name: local, role });
                }
            }
            Event::Empty(empty) => {
                let depth = stack.len() + 1;
                if depth > MAX_XFDF_DEPTH {
                    return Err(XfdfError::DepthExceeded {
                        depth,
                        max: MAX_XFDF_DEPTH,
                    });
                }
                let local = decode_local_name(empty.local_name().as_ref())?;
                let attrs = collect_checked_attributes(&empty, &mut report)?;
                *report.element_counts.entry(local.clone()).or_insert(0) += 1;

                if stack.is_empty() {
                    if seen_root {
                        return Err(XfdfError::MalformedXml("multiple root elements".into()));
                    }
                    seen_root = true;
                    check_xfdf_root(&local, &ns, &mut report)?;
                } else {
                    let role = enter_element(
                        &stack,
                        &local,
                        &ns,
                        &attrs,
                        &mut report,
                        &mut current_annot,
                    )?;
                    exit_element(
                        role,
                        &local,
                        &stack,
                        pages,
                        max_import_bytes,
                        &mut imported_on_page,
                        &mut imported_text_boxes,
                        &mut report,
                        &mut current_annot,
                    )?;
                }
            }
            Event::End(end) => {
                let local = decode_local_name(end.local_name().as_ref())?;
                let Some(frame) = stack.pop() else {
                    return Err(XfdfError::MalformedXml(format!(
                        "unmatched closing tag </{local}>"
                    )));
                };
                if frame.name != local {
                    return Err(XfdfError::MalformedXml(format!(
                        "mismatched closing tag: expected </{}>, found </{local}>",
                        frame.name
                    )));
                }
                exit_element(
                    frame.role,
                    &local,
                    &stack,
                    pages,
                    max_import_bytes,
                    &mut imported_on_page,
                    &mut imported_text_boxes,
                    &mut report,
                    &mut current_annot,
                )?;
            }
            Event::Eof => {
                if !seen_root {
                    return Err(XfdfError::MalformedXml(
                        "missing <xfdf> root element".into(),
                    ));
                }
                if !stack.is_empty() {
                    return Err(XfdfError::MalformedXml(
                        "unclosed XML elements at EOF".into(),
                    ));
                }
                break;
            }
        }
    }

    Ok(report)
}

pub fn resolve_text_boxes<F>(
    report: &mut XfdfReport,
    max_import_bytes: usize,
    mut load_font: F,
) -> Result<Vec<ResolvedTextBox>, XfdfError>
where
    F: FnMut(&str) -> Option<FontData>,
{
    struct PendingBox {
        item_index: usize,
        summary_index: Option<usize>,
        text: String,
        candidates: Vec<String>,
        candidate_pos: usize,
        prepared: Option<TextBoxModel>,
    }

    let summary_map: HashMap<usize, usize> = report
        .per_annotation
        .iter()
        .enumerate()
        .filter_map(|(si, s)| s.item_idx.map(|ii| (ii, si)))
        .collect();

    let mut pending: Vec<PendingBox> = Vec::new();
    for (idx, item) in report.items.iter().enumerate() {
        if let ImportedItem::FreeText {
            text, font_family, ..
        } = item
        {
            let mut candidates = Vec::with_capacity(1 + FALLBACK_FONT_FAMILIES.len());
            if let Some(req) = font_family {
                candidates.push(req.clone());
            }
            for &fallback in &FALLBACK_FONT_FAMILIES {
                if !candidates.iter().any(|c| c.eq_ignore_ascii_case(fallback)) {
                    candidates.push(fallback.to_string());
                }
            }
            pending.push(PendingBox {
                item_index: idx,
                summary_index: summary_map.get(&idx).copied(),
                text: text.clone(),
                candidates,
                candidate_pos: 0,
                prepared: None,
            });
        }
    }

    let mut font_cache: HashMap<String, Option<FontData>> = HashMap::new();
    loop {
        let mut by_family: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (p_idx, entry) in pending.iter_mut().enumerate() {
            if entry.prepared.is_some() {
                continue;
            }
            while entry.candidate_pos < entry.candidates.len() {
                let key = entry.candidates[entry.candidate_pos].to_ascii_lowercase();
                if !font_cache.contains_key(&key) {
                    let loaded = load_font(&entry.candidates[entry.candidate_pos]);
                    font_cache.insert(key.clone(), loaded);
                }
                if font_cache.get(&key).and_then(|o| o.as_ref()).is_some() {
                    by_family.entry(key).or_default().push(p_idx);
                    break;
                }
                entry.candidate_pos += 1;
            }
        }
        if by_family.is_empty() {
            break;
        }
        for (family_key, indices) in by_family {
            let Some(font_data) = font_cache.get(&family_key).and_then(|o| o.as_ref()) else {
                continue;
            };
            let font = TextBoxFont {
                bytes: &font_data.bytes,
                face: font_data.face,
                name: &font_data.name,
            };
            for chunk in indices.chunks(MAX_PREPARE_TEXT_BOXES) {
                let texts: Vec<String> = chunk.iter().map(|&pi| pending[pi].text.clone()).collect();
                let results = prepare_text_boxes(&font, &texts);
                for (&pi, res) in chunk.iter().zip(results.into_iter()) {
                    match res {
                        PreparedTextBoxModel::Prepared(model) => {
                            pending[pi].prepared = Some(model);
                        }
                        PreparedTextBoxModel::Refused | PreparedTextBoxModel::Deferred => {
                            pending[pi].candidate_pos += 1;
                        }
                    }
                }
            }
        }
    }

    let mut resolved = Vec::new();
    let mut next_id = 0u32;
    let mut extra_journal_bytes = 0usize;
    let mut dropped_items: Vec<(usize, Option<usize>)> = Vec::new();

    for entry in pending {
        if let Some(model) = entry.prepared {
            // StoredTextBox JSON size: {"id":N,"model":{...,"subset":"<hex>"}}
            let subset_hex_len = model.subset.len().saturating_mul(2);
            let glyphs_len = model.glyphs.len().saturating_mul(36);
            extra_journal_bytes = extra_journal_bytes
                .saturating_add(240)
                .saturating_add(subset_hex_len)
                .saturating_add(glyphs_len)
                .saturating_add(model.text.len());
            if report
                .estimated_journal_bytes
                .saturating_add(extra_journal_bytes)
                > max_import_bytes
            {
                return Err(XfdfError::TooMuchImportData {
                    bytes: report
                        .estimated_journal_bytes
                        .saturating_add(extra_journal_bytes),
                    max: max_import_bytes,
                });
            }
            resolved.push(ResolvedTextBox {
                item_index: entry.item_index,
                model_id: TextBoxModelId(next_id),
                model,
            });
            next_id = next_id.saturating_add(1);
        } else {
            dropped_items.push((entry.item_index, entry.summary_index));
        }
    }

    if !dropped_items.is_empty() {
        for &(_, summary_idx) in &dropped_items {
            if let Some(si) = summary_idx {
                report.per_annotation[si].outcome = AnnotationOutcome::Skipped(
                    SkipReason::FontUnavailable,
                    "no candidate font could shape text box".into(),
                );
                if report.per_annotation[si].style_adjusted {
                    report.style_adjusted_count = report.style_adjusted_count.saturating_sub(1);
                }
                report.per_annotation[si].item_idx = None;
            }
            report.skips.record(SkipReason::FontUnavailable);
            report.imported_count = report.imported_count.saturating_sub(1);
        }
        // Mark dropped items cleanly while keeping index alignment for resolved_map callers,
        // or return only surviving models mapped by item_index.
    }

    report.estimated_journal_bytes = report
        .estimated_journal_bytes
        .saturating_add(extra_journal_bytes);
    Ok(resolved)
}

pub fn apply_imported(
    doc: &mut PdfDocument,
    report: &XfdfReport,
    resolved_boxes: &[ResolvedTextBox],
) -> Result<(usize, Vec<u32>), String> {
    let model_by_item: HashMap<usize, &TextBoxModel> = resolved_boxes
        .iter()
        .map(|rb| (rb.item_index, &rb.model))
        .collect();
    let skipped_item_indices: HashMap<usize, SkipReason> = report
        .per_annotation
        .iter()
        .filter_map(|s| match &s.outcome {
            AnnotationOutcome::Skipped(r, _) => s.item_idx.map(|idx| (idx, *r)),
            AnnotationOutcome::Imported(_) => None,
        })
        .collect();

    let mut applied = 0usize;
    let mut touched_pages = Vec::new();
    for (idx, item) in report.items.iter().enumerate() {
        if skipped_item_indices.contains_key(&idx) {
            continue;
        }
        match item {
            ImportedItem::Highlight {
                mark_kind,
                page,
                quads,
                color,
            } => {
                doc.add_text_marks(
                    *mark_kind,
                    &[HighlightMark {
                        page: *page,
                        quads: quads.clone(),
                    }],
                    *color,
                )
                .map_err(|e| format!("apply highlight: {e:?}"))?;
                applied += 1;
                if !touched_pages.contains(page) {
                    touched_pages.push(*page);
                }
            }
            ImportedItem::Note { page, at, text } => {
                doc.add_note(*page, *at, text)
                    .map_err(|e| format!("apply note: {e:?}"))?;
                applied += 1;
                if !touched_pages.contains(page) {
                    touched_pages.push(*page);
                }
            }
            ImportedItem::Ink {
                page,
                strokes,
                color,
                width,
            } => {
                doc.add_ink(*page, strokes, *color, *width)
                    .map_err(|e| format!("apply ink: {e:?}"))?;
                applied += 1;
                if !touched_pages.contains(page) {
                    touched_pages.push(*page);
                }
            }
            ImportedItem::Stamp {
                page, rect, stamp, ..
            } => {
                doc.add_stamp(*page, *rect, *stamp)
                    .map_err(|e| format!("apply stamp: {e:?}"))?;
                applied += 1;
                if !touched_pages.contains(page) {
                    touched_pages.push(*page);
                }
            }
            ImportedItem::Shape {
                page,
                shape,
                color,
                width,
            } => {
                doc.add_shape(*page, shape, *color, *width)
                    .map_err(|e| format!("apply shape: {e:?}"))?;
                applied += 1;
                if !touched_pages.contains(page) {
                    touched_pages.push(*page);
                }
            }
            ImportedItem::FreeText {
                page, rect, style, ..
            } => {
                if let Some(model) = model_by_item.get(&idx) {
                    doc.place_text_box(*page, *rect, model, style)
                        .map_err(|e| format!("apply freetext: {e:?}"))?;
                    applied += 1;
                    if !touched_pages.contains(page) {
                        touched_pages.push(*page);
                    }
                }
            }
        }
    }
    Ok((applied, touched_pages))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameRole {
    Root,
    Annots,
    AnnotItem,
    Contents,
    RichText { skip_text: bool },
    DefaultStyle,
    DefaultAppearance,
    InkList,
    Gesture,
    ImageData,
    IgnoredSubtree,
}

#[derive(Debug, Clone)]
struct Frame {
    name: String,
    role: FrameRole,
}

#[derive(Debug)]
struct AnnotBuilder {
    index: usize,
    tag: String,
    attrs: BTreeMap<String, String>,
    contents: Option<String>,
    contents_buf: String,
    contents_truncated: bool,
    richtext: Option<String>,
    richtext_buf: String,
    richtext_truncated: bool,
    default_style: Option<String>,
    style_buf: String,
    default_appearance: Option<String>,
    da_buf: String,
    gestures: Vec<String>,
    gesture_buf: String,
    has_imagedata: bool,
    capture: CaptureTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTarget {
    None,
    Contents,
    RichText { skip_text: bool },
    DefaultStyle,
    DefaultAppearance,
    Gesture,
}

impl AnnotBuilder {
    fn new(index: usize, tag: String, attrs: BTreeMap<String, String>) -> Self {
        Self {
            index,
            tag,
            attrs,
            contents: None,
            contents_buf: String::new(),
            contents_truncated: false,
            richtext: None,
            richtext_buf: String::new(),
            richtext_truncated: false,
            default_style: None,
            style_buf: String::new(),
            default_appearance: None,
            da_buf: String::new(),
            gestures: Vec::new(),
            gesture_buf: String::new(),
            has_imagedata: false,
            capture: CaptureTarget::None,
        }
    }

    fn push_text(&mut self, s: &str) {
        match self.capture {
            CaptureTarget::None | CaptureTarget::RichText { skip_text: true } => {}
            CaptureTarget::Contents => {
                push_capped(
                    &mut self.contents_buf,
                    s,
                    MAX_RICHTEXT_BYTES,
                    &mut self.contents_truncated,
                );
            }
            CaptureTarget::RichText { skip_text: false } => {
                push_capped(
                    &mut self.richtext_buf,
                    s,
                    MAX_RICHTEXT_BYTES,
                    &mut self.richtext_truncated,
                );
            }
            CaptureTarget::DefaultStyle => {
                push_capped(&mut self.style_buf, s, 4096, &mut false);
            }
            CaptureTarget::DefaultAppearance => {
                push_capped(&mut self.da_buf, s, 4096, &mut false);
            }
            CaptureTarget::Gesture => {
                // Gestures can hold up to 20,000 points (~400 KiB); cap at 1 MiB per gesture.
                push_capped(&mut self.gesture_buf, s, 1024 * 1024, &mut false);
            }
        }
    }

    fn push_richtext_newline(&mut self) {
        if !self.richtext_buf.is_empty() && !self.richtext_buf.ends_with('\n') {
            push_capped(
                &mut self.richtext_buf,
                "\n",
                MAX_RICHTEXT_BYTES,
                &mut self.richtext_truncated,
            );
        }
    }
}

fn push_capped(dst: &mut String, s: &str, max_bytes: usize, truncated: &mut bool) {
    if dst.len() >= max_bytes {
        *truncated = true;
        return;
    }
    let avail = max_bytes - dst.len();
    if s.len() <= avail {
        dst.push_str(s);
    } else {
        let mut end = avail;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        dst.push_str(&s[..end]);
        *truncated = true;
    }
}

fn enter_element(
    stack: &[Frame],
    local: &str,
    ns: &ResolveResult<'_>,
    attrs: &BTreeMap<String, String>,
    report: &mut XfdfReport,
    current_annot: &mut Option<AnnotBuilder>,
) -> Result<FrameRole, XfdfError> {
    let parent = stack.last().expect("non-empty stack");
    match &parent.role {
        FrameRole::Root => {
            if local == "annots" {
                check_xfdf_ns(ns)?;
                Ok(FrameRole::Annots)
            } else {
                if local == "f" {
                    check_xfdf_ns(ns)?;
                    if let Some(href) = attrs.get("href") {
                        report.f_href = Some(href.clone());
                    }
                }
                Ok(FrameRole::IgnoredSubtree)
            }
        }
        FrameRole::Annots => {
            check_xfdf_ns(ns)?;
            report.total_annots_seen += 1;
            if report.total_annots_seen > MAX_SOURCE_ANNOTATIONS as usize {
                return Err(XfdfError::TooManyAnnotations {
                    count: report.total_annots_seen,
                    max: MAX_SOURCE_ANNOTATIONS as usize,
                });
            }
            *current_annot = Some(AnnotBuilder::new(
                report.total_annots_seen,
                local.to_ascii_lowercase(),
                attrs.clone(),
            ));
            Ok(FrameRole::AnnotItem)
        }
        FrameRole::AnnotItem => {
            let Some(builder) = current_annot.as_mut() else {
                return Ok(FrameRole::IgnoredSubtree);
            };
            match local {
                "contents" => {
                    check_xfdf_ns(ns)?;
                    builder.contents_buf.clear();
                    builder.capture = CaptureTarget::Contents;
                    Ok(FrameRole::Contents)
                }
                "contents-richtext" => {
                    check_xfdf_ns(ns)?;
                    builder.richtext_buf.clear();
                    builder.capture = CaptureTarget::RichText { skip_text: false };
                    Ok(FrameRole::RichText { skip_text: false })
                }
                "defaultstyle" => {
                    check_xfdf_ns(ns)?;
                    builder.style_buf.clear();
                    builder.capture = CaptureTarget::DefaultStyle;
                    Ok(FrameRole::DefaultStyle)
                }
                "defaultappearance" => {
                    check_xfdf_ns(ns)?;
                    builder.da_buf.clear();
                    builder.capture = CaptureTarget::DefaultAppearance;
                    Ok(FrameRole::DefaultAppearance)
                }
                "inklist" => {
                    check_xfdf_ns(ns)?;
                    Ok(FrameRole::InkList)
                }
                "imagedata" => {
                    check_xfdf_ns(ns)?;
                    builder.has_imagedata = true;
                    Ok(FrameRole::ImageData)
                }
                _ => Ok(FrameRole::IgnoredSubtree),
            }
        }
        FrameRole::InkList => {
            if local == "gesture" {
                check_xfdf_ns(ns)?;
                if let Some(builder) = current_annot.as_mut() {
                    builder.gesture_buf.clear();
                    builder.capture = CaptureTarget::Gesture;
                }
                Ok(FrameRole::Gesture)
            } else {
                Ok(FrameRole::IgnoredSubtree)
            }
        }
        FrameRole::RichText { skip_text } => {
            check_richtext_ns(ns)?;
            let skip =
                *skip_text || matches!(local.to_ascii_lowercase().as_str(), "script" | "style");
            if let Some(builder) = current_annot.as_mut() {
                if !skip && local.eq_ignore_ascii_case("br") {
                    builder.push_richtext_newline();
                }
                builder.capture = CaptureTarget::RichText { skip_text: skip };
            }
            Ok(FrameRole::RichText { skip_text: skip })
        }
        FrameRole::Contents
        | FrameRole::DefaultStyle
        | FrameRole::DefaultAppearance
        | FrameRole::Gesture
        | FrameRole::ImageData
        | FrameRole::IgnoredSubtree => Ok(FrameRole::IgnoredSubtree),
    }
}

#[allow(clippy::too_many_arguments)]
fn exit_element(
    role: FrameRole,
    local: &str,
    stack: &[Frame],
    pages: &[PageGeometry],
    max_import_bytes: usize,
    imported_on_page: &mut [u32],
    imported_text_boxes: &mut u32,
    report: &mut XfdfReport,
    current_annot: &mut Option<AnnotBuilder>,
) -> Result<(), XfdfError> {
    match role {
        FrameRole::Contents => {
            if let Some(builder) = current_annot.as_mut() {
                builder.contents = Some(std::mem::take(&mut builder.contents_buf));
                builder.capture = CaptureTarget::None;
            }
        }
        FrameRole::RichText { .. } => {
            if let Some(builder) = current_annot.as_mut() {
                if local == "contents-richtext" {
                    let text = std::mem::take(&mut builder.richtext_buf);
                    builder.richtext = Some(text);
                    builder.capture = CaptureTarget::None;
                } else {
                    if matches!(
                        local.to_ascii_lowercase().as_str(),
                        "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    ) {
                        builder.push_richtext_newline();
                    }
                    let parent_skip = matches!(
                        stack.last().map(|f| &f.role),
                        Some(FrameRole::RichText { skip_text: true })
                    );
                    builder.capture = CaptureTarget::RichText {
                        skip_text: parent_skip,
                    };
                }
            }
        }
        FrameRole::DefaultStyle => {
            if let Some(builder) = current_annot.as_mut() {
                builder.default_style = Some(std::mem::take(&mut builder.style_buf));
                builder.capture = CaptureTarget::None;
            }
        }
        FrameRole::DefaultAppearance => {
            if let Some(builder) = current_annot.as_mut() {
                builder.default_appearance = Some(std::mem::take(&mut builder.da_buf));
                builder.capture = CaptureTarget::None;
            }
        }
        FrameRole::Gesture => {
            if let Some(builder) = current_annot.as_mut() {
                builder
                    .gestures
                    .push(std::mem::take(&mut builder.gesture_buf));
                builder.capture = CaptureTarget::None;
            }
        }
        FrameRole::AnnotItem => {
            if let Some(builder) = current_annot.take() {
                finalize_annotation(
                    builder,
                    pages,
                    max_import_bytes,
                    imported_on_page,
                    imported_text_boxes,
                    report,
                )?;
            }
        }
        FrameRole::Root
        | FrameRole::Annots
        | FrameRole::InkList
        | FrameRole::ImageData
        | FrameRole::IgnoredSubtree => {}
    }
    Ok(())
}

fn finalize_annotation(
    builder: AnnotBuilder,
    pages: &[PageGeometry],
    max_import_bytes: usize,
    imported_on_page: &mut [u32],
    imported_text_boxes: &mut u32,
    report: &mut XfdfReport,
) -> Result<(), XfdfError> {
    let page_attr = builder.attrs.get("page").cloned();
    let richtext_plain = builder
        .richtext
        .as_deref()
        .map(|s| s.trim_matches('\n').to_string())
        .filter(|s| !s.is_empty());
    let stamp_icon = builder.attrs.get("icon").cloned();

    let result = convert_annotation(&builder, pages, imported_on_page, *imported_text_boxes);

    match result {
        Ok((item, adjusted, summary_str)) => {
            let page = item.page() as usize;
            imported_on_page[page] = imported_on_page[page].saturating_add(1);
            if matches!(item, ImportedItem::FreeText { .. }) {
                *imported_text_boxes = imported_text_boxes.saturating_add(1);
            }
            report.estimated_journal_bytes = report
                .estimated_journal_bytes
                .saturating_add(item.estimated_journal_bytes());
            if report.estimated_journal_bytes > max_import_bytes {
                return Err(XfdfError::TooMuchImportData {
                    bytes: report.estimated_journal_bytes,
                    max: max_import_bytes,
                });
            }
            let item_idx = report.items.len();
            report.items.push(item);
            report.imported_count += 1;
            if adjusted {
                report.style_adjusted_count += 1;
            }
            report.per_annotation.push(ParsedAnnotationSummary {
                index: builder.index,
                tag: builder.tag,
                page_attr,
                outcome: AnnotationOutcome::Imported(summary_str.clone()),
                converted_summary: summary_str,
                richtext_plain,
                stamp_icon,
                style_adjusted: adjusted,
                item_idx: Some(item_idx),
            });
        }
        Err((reason, detail)) => {
            report.skips.record(reason);
            report.per_annotation.push(ParsedAnnotationSummary {
                index: builder.index,
                tag: builder.tag,
                page_attr,
                outcome: AnnotationOutcome::Skipped(reason, detail.clone()),
                converted_summary: detail,
                richtext_plain,
                stamp_icon,
                style_adjusted: false,
                item_idx: None,
            });
        }
    }
    Ok(())
}

fn convert_annotation(
    b: &AnnotBuilder,
    pages: &[PageGeometry],
    imported_on_page: &[u32],
    imported_text_boxes: u32,
) -> Result<(ImportedItem, bool, String), (SkipReason, String)> {
    if b.attrs
        .get("inreplyto")
        .is_some_and(|v| !v.trim().is_empty())
    {
        return Err((SkipReason::Reply, "inreplyto present".into()));
    }

    let kind = match b.tag.as_str() {
        "highlight" => AnnotKind::TextMark(TextMarkKind::Highlight),
        "underline" => AnnotKind::TextMark(TextMarkKind::Underline),
        "strikeout" => AnnotKind::TextMark(TextMarkKind::StrikeOut),
        "squiggly" => AnnotKind::TextMark(TextMarkKind::Squiggly),
        "text" => AnnotKind::Note,
        "ink" => AnnotKind::Ink,
        "stamp" => AnnotKind::Stamp,
        "freetext" => AnnotKind::FreeText,
        "square" => AnnotKind::Square,
        "circle" => AnnotKind::Circle,
        "line" => AnnotKind::Line,
        other => {
            return Err((
                SkipReason::UnsupportedKind,
                format!("unsupported XFDF element <{other}>"),
            ));
        }
    };

    if let Some(flags) = b.attrs.get("flags") {
        for flag in flags.split(',').map(|s| s.trim().to_ascii_lowercase()) {
            if matches!(flag.as_str(), "hidden" | "noview") {
                return Err((
                    SkipReason::UnsupportedKind,
                    format!("flags contains {flag}"),
                ));
            }
        }
    }

    if matches!(kind, AnnotKind::Stamp) && b.has_imagedata {
        return Err((
            SkipReason::UnsupportedKind,
            "stamp has <imagedata> picture".into(),
        ));
    }

    let stamp_name = if matches!(kind, AnnotKind::Stamp) {
        let icon = b.attrs.get("icon").ok_or((
            SkipReason::InvalidGeometryOrFormat,
            "stamp missing icon attribute".into(),
        ))?;
        let parsed = parse_stamp_name(icon).ok_or_else(|| {
            (
                SkipReason::UnsupportedKind,
                format!("non-app stamp icon '{icon}'"),
            )
        })?;
        Some((parsed, icon.clone()))
    } else {
        None
    };

    let page_idx = parse_page_index(b.attrs.get("page"), pages.len())?;
    let page_geom = &pages[page_idx as usize];

    if page_geom
        .existing_annots
        .saturating_add(imported_on_page[page_idx as usize])
        >= MAX_ANNOTATIONS_PER_PAGE
    {
        return Err((
            SkipReason::ExceedsLimit,
            format!("page {page_idx} reached {MAX_ANNOTATIONS_PER_PAGE} annotation limit"),
        ));
    }

    match kind {
        AnnotKind::TextMark(mark_kind) => {
            let coords_raw = b.attrs.get("coords").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "text markup missing coords".into(),
            ))?;
            let quads = parse_and_transform_quads(coords_raw, page_geom)?;
            let (color, adjusted) = map_highlight_color(b.attrs.get("color").map(String::as_str))?;
            let edit = Edit::AddHighlight {
                mark_kind,
                marks: vec![HighlightMark {
                    page: page_idx,
                    quads: quads.clone(),
                }],
                color,
            };
            edit.validate()
                .map_err(|e| (SkipReason::InvalidGeometryOrFormat, format!("{e:?}")))?;
            let first = quads[0];
            let summary = format!(
                "{mark_kind:?} page={page_idx} quads={} first=({:.1},{:.1})..({:.1},{:.1}) color={color:?}",
                quads.len(),
                first.ul.x,
                first.ul.y,
                first.lr.x,
                first.lr.y
            );
            Ok((
                ImportedItem::Highlight {
                    mark_kind,
                    page: page_idx,
                    quads,
                    color,
                },
                adjusted,
                summary,
            ))
        }
        AnnotKind::Note => {
            let rect_raw = b.attrs.get("rect").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "note missing rect".into(),
            ))?;
            let norm = parse_and_transform_rect(rect_raw, page_geom, false)?;
            let raw_text = extract_note_or_freetext_text(b)?;
            let text = clean_note_text(&raw_text, usize::MAX);
            if text.len() > MAX_NOTE_TEXT_BYTES as usize {
                return Err((
                    SkipReason::ExceedsLimit,
                    format!(
                        "note text {} bytes exceeds {MAX_NOTE_TEXT_BYTES}",
                        text.len()
                    ),
                ));
            }
            check_note_text(&text).map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    "empty or invalid note text".into(),
                )
            })?;
            let at = Point {
                x: norm.x0,
                y: norm.y0,
            };
            let edit = Edit::AddNote {
                page: page_idx,
                at,
                text: text.clone(),
            };
            edit.validate()
                .map_err(|e| (SkipReason::InvalidGeometryOrFormat, format!("{e:?}")))?;
            let summary = format!(
                "Note page={page_idx} at=({:.1},{:.1}) len={}B",
                at.x,
                at.y,
                text.len()
            );
            Ok((
                ImportedItem::Note {
                    page: page_idx,
                    at,
                    text,
                },
                false,
                summary,
            ))
        }
        AnnotKind::Ink => {
            if b.gestures.is_empty() {
                return Err((
                    SkipReason::InvalidGeometryOrFormat,
                    "ink has no <gesture> strokes".into(),
                ));
            }
            if b.gestures.len() > MAX_INK_STROKES as usize {
                return Err((
                    SkipReason::ExceedsLimit,
                    format!(
                        "ink stroke count {} exceeds {MAX_INK_STROKES}",
                        b.gestures.len()
                    ),
                ));
            }
            let (strokes, min_x, min_y, max_x, max_y) =
                parse_and_transform_ink(&b.gestures, page_geom)?;
            let (color, width, adjusted) = map_ink_style(
                b.attrs.get("color").map(String::as_str),
                b.attrs.get("width").map(String::as_str),
                false,
            )?;
            let edit = Edit::AddInk {
                page: page_idx,
                strokes: strokes.clone(),
                color,
                width,
            };
            edit.validate()
                .map_err(|e| (SkipReason::InvalidGeometryOrFormat, format!("{e:?}")))?;
            let total_pts: usize = strokes.iter().map(Vec::len).sum();
            let summary = format!(
                "Ink page={page_idx} strokes={} points={} bbox=({min_x:.1},{min_y:.1},{max_x:.1},{max_y:.1}) color={color:?} width={width:?}",
                strokes.len(),
                total_pts
            );
            Ok((
                ImportedItem::Ink {
                    page: page_idx,
                    strokes,
                    color,
                    width,
                },
                adjusted,
                summary,
            ))
        }
        AnnotKind::Stamp => {
            let (stamp, icon_raw) = stamp_name.expect("validated above");
            let rect_raw = b.attrs.get("rect").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "stamp missing rect".into(),
            ))?;
            let rect = parse_and_transform_rect(rect_raw, page_geom, true)?;
            check_annotation_rect(&rect).map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    format!("stamp rect smaller than {MIN_ANNOTATION_SIDE_PT}pt or invalid"),
                )
            })?;
            let edit = Edit::AddStamp {
                page: page_idx,
                rect,
                stamp,
            };
            edit.validate()
                .map_err(|e| (SkipReason::InvalidGeometryOrFormat, format!("{e:?}")))?;
            let summary = format!(
                "Stamp('{icon_raw}') page={page_idx} rect=({:.1},{:.1},{:.1},{:.1})",
                rect.x0, rect.y0, rect.x1, rect.y1
            );
            Ok((
                ImportedItem::Stamp {
                    page: page_idx,
                    rect,
                    stamp,
                    icon_raw,
                },
                false,
                summary,
            ))
        }
        AnnotKind::Square => {
            let rect_raw = b.attrs.get("rect").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "square missing rect".into(),
            ))?;
            let rect = parse_and_transform_rect(rect_raw, page_geom, true)?;
            let shape = Shape::Rectangle { rect };
            shape.validate().map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    "invalid square rectangle".into(),
                )
            })?;
            let has_unsupported_border_or_fill = b.attrs.contains_key("interior-color")
                || b.attrs
                    .get("style")
                    .is_some_and(|s| !s.eq_ignore_ascii_case("solid"))
                || b.attrs.contains_key("intensity");
            let (color, width, adjusted) = map_ink_style(
                b.attrs.get("color").map(String::as_str),
                b.attrs.get("width").map(String::as_str),
                has_unsupported_border_or_fill,
            )?;
            let summary = format!(
                "Square page={page_idx} rect=({:.1},{:.1},{:.1},{:.1}) color={color:?} width={width:?}",
                rect.x0, rect.y0, rect.x1, rect.y1
            );
            Ok((
                ImportedItem::Shape {
                    page: page_idx,
                    shape,
                    color,
                    width,
                },
                adjusted,
                summary,
            ))
        }
        AnnotKind::Circle => {
            let rect_raw = b.attrs.get("rect").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "circle missing rect".into(),
            ))?;
            let rect = parse_and_transform_rect(rect_raw, page_geom, true)?;
            let shape = Shape::Ellipse { rect };
            shape.validate().map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    "invalid circle rectangle".into(),
                )
            })?;
            let has_unsupported_border_or_fill = b.attrs.contains_key("interior-color")
                || b.attrs
                    .get("style")
                    .is_some_and(|s| !s.eq_ignore_ascii_case("solid"))
                || b.attrs.contains_key("intensity");
            let (color, width, adjusted) = map_ink_style(
                b.attrs.get("color").map(String::as_str),
                b.attrs.get("width").map(String::as_str),
                has_unsupported_border_or_fill,
            )?;
            let summary = format!(
                "Circle page={page_idx} rect=({:.1},{:.1},{:.1},{:.1}) color={color:?} width={width:?}",
                rect.x0, rect.y0, rect.x1, rect.y1
            );
            Ok((
                ImportedItem::Shape {
                    page: page_idx,
                    shape,
                    color,
                    width,
                },
                adjusted,
                summary,
            ))
        }
        AnnotKind::Line => {
            let start_raw = b.attrs.get("start").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "line missing start".into(),
            ))?;
            let end_raw = b.attrs.get("end").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "line missing end".into(),
            ))?;
            let start = parse_and_transform_point(start_raw, page_geom)?;
            let end = parse_and_transform_point(end_raw, page_geom)?;
            if !point_or_segment_intersects_page(&page_geom.bounds, start, end) {
                return Err((
                    SkipReason::InvalidGeometryOrFormat,
                    "line lies completely outside page".into(),
                ));
            }
            let head = b.attrs.get("head").map(String::as_str).unwrap_or("None");
            let tail = b.attrs.get("tail").map(String::as_str).unwrap_or("None");
            let (head_arrow, head_adj) = classify_line_ending(head);
            let (tail_arrow, tail_adj) = classify_line_ending(tail);
            let (from, to, arrow, arrow_adj) = match (head_arrow, tail_arrow) {
                (false, false) => (start, end, false, head_adj || tail_adj),
                (false, true) => (start, end, true, head_adj || tail_adj),
                (true, false) => (end, start, true, head_adj || tail_adj),
                (true, true) => (start, end, true, true),
            };
            let shape = Shape::Line { from, to, arrow };
            shape.validate().map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    "line endpoints coincide or out of range".into(),
                )
            })?;
            let style_extra = arrow_adj
                || b.attrs
                    .get("style")
                    .is_some_and(|s| !s.eq_ignore_ascii_case("solid"));
            let (color, width, adjusted) = map_ink_style(
                b.attrs.get("color").map(String::as_str),
                b.attrs.get("width").map(String::as_str),
                style_extra,
            )?;
            let summary = format!(
                "Line page={page_idx} from=({:.1},{:.1}) to=({:.1},{:.1}) arrow={arrow} color={color:?} width={width:?}",
                from.x, from.y, to.x, to.y
            );
            Ok((
                ImportedItem::Shape {
                    page: page_idx,
                    shape,
                    color,
                    width,
                },
                adjusted,
                summary,
            ))
        }
        AnnotKind::FreeText => {
            if imported_text_boxes >= MAX_SOURCE_TEXT_BOXES {
                return Err((
                    SkipReason::ExceedsLimit,
                    format!("text box count reached {MAX_SOURCE_TEXT_BOXES} limit"),
                ));
            }
            let rect_raw = b.attrs.get("rect").ok_or((
                SkipReason::InvalidGeometryOrFormat,
                "freetext missing rect".into(),
            ))?;
            let rect = parse_and_transform_rect(rect_raw, page_geom, true)?;
            check_annotation_rect(&rect).map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    format!("freetext rect smaller than {MIN_ANNOTATION_SIDE_PT}pt or invalid"),
                )
            })?;
            let raw_text = extract_note_or_freetext_text(b)?;
            let (text, _) = clean_field_text(&raw_text, usize::MAX, true);
            if text.len() > MAX_TEXT_BOX_BYTES {
                return Err((
                    SkipReason::ExceedsLimit,
                    format!("freetext {} bytes exceeds {MAX_TEXT_BOX_BYTES}", text.len()),
                ));
            }
            check_text_box_text(&text).map_err(|_| {
                (
                    SkipReason::InvalidGeometryOrFormat,
                    "empty freetext content".into(),
                )
            })?;
            let (font_family, style, adjusted) = parse_freetext_style(b);
            style
                .validate()
                .map_err(|e| (SkipReason::InvalidGeometryOrFormat, format!("{e:?}")))?;
            let summary = format!(
                "FreeText page={page_idx} rect=({:.1},{:.1},{:.1},{:.1}) font={} size={:.1} color=({},{},{}) align={:?} len={}B",
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y1,
                font_family.as_deref().unwrap_or("<fallback>"),
                style.size,
                style.color.r,
                style.color.g,
                style.color.b,
                style.align,
                text.len()
            );
            Ok((
                ImportedItem::FreeText {
                    page: page_idx,
                    rect,
                    text,
                    font_family,
                    style,
                },
                adjusted,
                summary,
            ))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AnnotKind {
    TextMark(TextMarkKind),
    Note,
    Ink,
    Stamp,
    FreeText,
    Square,
    Circle,
    Line,
}

fn extract_note_or_freetext_text(b: &AnnotBuilder) -> Result<String, (SkipReason, String)> {
    let contents_exceeds = b.contents_truncated;
    let richtext_exceeds = b.richtext_truncated;
    if let Some(c) = b.contents.as_deref() {
        if !c.trim().is_empty() {
            if contents_exceeds {
                return Err((
                    SkipReason::ExceedsLimit,
                    "note/freetext content exceeds text byte limit".into(),
                ));
            }
            return Ok(c.to_string());
        }
    }
    if let Some(rt) = b.richtext.as_deref() {
        let trimmed = rt.trim_matches('\n');
        if !trimmed.trim().is_empty() {
            if richtext_exceeds {
                return Err((
                    SkipReason::ExceedsLimit,
                    "richtext content exceeds text byte limit".into(),
                ));
            }
            return Ok(trimmed.to_string());
        }
    }
    Err((
        SkipReason::InvalidGeometryOrFormat,
        "missing or empty <contents> / <contents-richtext>".into(),
    ))
}

fn parse_page_index(raw: Option<&String>, page_count: usize) -> Result<u32, (SkipReason, String)> {
    let s = raw.ok_or_else(|| {
        (
            SkipReason::PageOutOfRange,
            "missing page attribute".to_string(),
        )
    })?;
    let page: u32 = s.trim().parse().map_err(|_| {
        (
            SkipReason::PageOutOfRange,
            format!("invalid page attribute '{s}'"),
        )
    })?;
    if (page as usize) >= page_count {
        return Err((
            SkipReason::PageOutOfRange,
            format!("page {page} out of range (page_count={page_count})"),
        ));
    }
    Ok(page)
}

fn parse_finite_coord(s: &str) -> Option<f32> {
    let v: f32 = s.trim().parse().ok()?;
    if v.is_finite() && v.abs() <= MAX_PAGE_SIDE_PT {
        Some(v)
    } else {
        None
    }
}

fn transform_xy(ctm: &mupdf::Matrix, x: f32, y: f32) -> Result<Point, (SkipReason, String)> {
    let (px, py) = ctm.transform_xy(x, y);
    if !px.is_finite()
        || !py.is_finite()
        || px.abs() > MAX_PAGE_SIDE_PT
        || py.abs() > MAX_PAGE_SIDE_PT
    {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            "transformed coordinate non-finite or out of range".into(),
        ));
    }
    Ok(Point { x: px, y: py })
}

fn intersects_page(page_bounds: &mupdf::Rect, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    let min_px = page_bounds.x0.min(page_bounds.x1);
    let max_px = page_bounds.x0.max(page_bounds.x1);
    let min_py = page_bounds.y0.min(page_bounds.y1);
    let max_py = page_bounds.y0.max(page_bounds.y1);
    x1 > min_px && x0 < max_px && y1 > min_py && y0 < max_py
}

fn point_or_segment_intersects_page(page_bounds: &mupdf::Rect, a: Point, b: Point) -> bool {
    let min_px = page_bounds.x0.min(page_bounds.x1);
    let max_px = page_bounds.x0.max(page_bounds.x1);
    let min_py = page_bounds.y0.min(page_bounds.y1);
    let max_py = page_bounds.y0.max(page_bounds.y1);
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    max_x >= min_px && min_x <= max_px && max_y >= min_py && min_y <= max_py
}

fn parse_and_transform_rect(
    raw: &str,
    page: &PageGeometry,
    require_non_empty_area: bool,
) -> Result<Rect, (SkipReason, String)> {
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 4 {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            format!("rect must have 4 coordinates, found {}", parts.len()),
        ));
    }
    let ux0 = parse_finite_coord(parts[0]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid rect x0".into(),
    ))?;
    let uy0 = parse_finite_coord(parts[1]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid rect y0".into(),
    ))?;
    let ux1 = parse_finite_coord(parts[2]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid rect x1".into(),
    ))?;
    let uy1 = parse_finite_coord(parts[3]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid rect y1".into(),
    ))?;
    let p0 = transform_xy(&page.ctm, ux0, uy0)?;
    let p1 = transform_xy(&page.ctm, ux1, uy1)?;
    let rect = Rect {
        x0: p0.x.min(p1.x),
        y0: p0.y.min(p1.y),
        x1: p0.x.max(p1.x),
        y1: p0.y.max(p1.y),
    };
    if require_non_empty_area {
        if rect.x1 <= rect.x0 || rect.y1 <= rect.y0 {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                "zero-area rectangle".into(),
            ));
        }
        if !intersects_page(&page.bounds, rect.x0, rect.y0, rect.x1, rect.y1) {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                "rectangle outside page bounds".into(),
            ));
        }
    } else if !point_or_segment_intersects_page(
        &page.bounds,
        Point {
            x: rect.x0,
            y: rect.y0,
        },
        Point {
            x: rect.x1,
            y: rect.y1,
        },
    ) {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            "note rectangle outside page bounds".into(),
        ));
    }
    Ok(rect)
}

fn parse_and_transform_point(
    raw: &str,
    page: &PageGeometry,
) -> Result<Point, (SkipReason, String)> {
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 2 {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            format!("point must have 2 coordinates, found {}", parts.len()),
        ));
    }
    let ux = parse_finite_coord(parts[0]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid point x".into(),
    ))?;
    let uy = parse_finite_coord(parts[1]).ok_or((
        SkipReason::InvalidGeometryOrFormat,
        "invalid point y".into(),
    ))?;
    transform_xy(&page.ctm, ux, uy)
}

fn parse_and_transform_quads(
    raw: &str,
    page: &PageGeometry,
) -> Result<Vec<Quad>, (SkipReason, String)> {
    let mut vals = Vec::new();
    for tok in raw.split(',') {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                "empty token in quad coords".into(),
            ));
        }
        vals.push(parse_finite_coord(trimmed).ok_or((
            SkipReason::InvalidGeometryOrFormat,
            "invalid quad coordinate".into(),
        ))?);
    }
    if vals.is_empty() || vals.len() % 8 != 0 {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            format!(
                "quad coords count {} is not a non-zero multiple of 8",
                vals.len()
            ),
        ));
    }
    let quad_count = vals.len() / 8;
    if quad_count > MAX_ANNOTATION_QUADS as usize {
        return Err((
            SkipReason::ExceedsLimit,
            format!("quad count {quad_count} exceeds {MAX_ANNOTATION_QUADS}"),
        ));
    }
    let mut quads = Vec::with_capacity(quad_count);
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for chunk in vals.chunks_exact(8) {
        let ul = transform_xy(&page.ctm, chunk[0], chunk[1])?;
        let ur = transform_xy(&page.ctm, chunk[2], chunk[3])?;
        let ll = transform_xy(&page.ctm, chunk[4], chunk[5])?;
        let lr = transform_xy(&page.ctm, chunk[6], chunk[7])?;
        for p in [ul, ur, ll, lr] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        quads.push(Quad { ul, ur, ll, lr });
    }
    if !intersects_page(&page.bounds, min_x, min_y, max_x, max_y) {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            "quads outside page bounds".into(),
        ));
    }
    Ok(quads)
}

#[allow(clippy::type_complexity)]
fn parse_and_transform_ink(
    gestures: &[String],
    page: &PageGeometry,
) -> Result<(Vec<Vec<Point>>, f32, f32, f32, f32), (SkipReason, String)> {
    let mut strokes = Vec::with_capacity(gestures.len());
    let mut total_points = 0usize;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for g in gestures {
        let trimmed = g.trim();
        if trimmed.is_empty() {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                "empty <gesture> stroke".into(),
            ));
        }
        let pairs: Vec<&str> = if trimmed.contains(';') {
            trimmed
                .split(';')
                .filter(|s| !s.trim().is_empty())
                .collect()
        } else {
            trimmed.split_whitespace().collect()
        };
        if pairs.is_empty() {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                "empty <gesture> stroke".into(),
            ));
        }
        total_points = total_points.saturating_add(pairs.len());
        if total_points > MAX_INK_POINTS as usize {
            return Err((
                SkipReason::ExceedsLimit,
                format!("ink points {total_points} exceeds {MAX_INK_POINTS}"),
            ));
        }
        let mut pts = Vec::with_capacity(pairs.len());
        for pair in pairs {
            let p = parse_and_transform_point(pair, page)?;
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
            pts.push(p);
        }
        strokes.push(pts);
    }

    if !point_or_segment_intersects_page(
        &page.bounds,
        Point { x: min_x, y: min_y },
        Point { x: max_x, y: max_y },
    ) {
        return Err((
            SkipReason::InvalidGeometryOrFormat,
            "ink drawing lies outside page bounds".into(),
        ));
    }

    Ok((strokes, min_x, min_y, max_x, max_y))
}

fn parse_stamp_name(name: &str) -> Option<StampName> {
    match name {
        "Approved" => Some(StampName::Approved),
        "NotApproved" => Some(StampName::NotApproved),
        "Draft" => Some(StampName::Draft),
        "Final" => Some(StampName::Final),
        "Confidential" => Some(StampName::Confidential),
        "ForComment" => Some(StampName::ForComment),
        "AsIs" => Some(StampName::AsIs),
        "TopSecret" => Some(StampName::TopSecret),
        _ => None,
    }
}

fn classify_line_ending(s: &str) -> (bool, bool) {
    match s.trim() {
        "" | "None" => (false, false),
        "OpenArrow" | "ClosedArrow" | "ROpenArrow" | "RClosedArrow" => (true, false),
        _ => (false, true),
    }
}

fn parse_hex_color(raw: &str) -> Option<[f32; 3]> {
    let hex = raw.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    ])
}

fn parse_hex_rgb(raw: &str) -> Option<Rgb> {
    let hex = raw.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some(Rgb {
        r: u8::from_str_radix(&hex[0..2], 16).ok()?,
        g: u8::from_str_radix(&hex[2..4], 16).ok()?,
        b: u8::from_str_radix(&hex[4..6], 16).ok()?,
    })
}

fn within_tolerance(a: [f32; 3], b: [f32; 3]) -> bool {
    (a[0] - b[0]).abs() <= COLOR_TOLERANCE
        && (a[1] - b[1]).abs() <= COLOR_TOLERANCE
        && (a[2] - b[2]).abs() <= COLOR_TOLERANCE
}

fn sq_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    dr * dr + dg * dg + db * db
}

fn map_highlight_color(raw: Option<&str>) -> Result<(HighlightColor, bool), (SkipReason, String)> {
    const PALETTE: [(HighlightColor, [f32; 3]); 4] = [
        (HighlightColor::Yellow, [1.0, 0.92, 0.0]),
        (HighlightColor::Green, [0.55, 0.9, 0.35]),
        (HighlightColor::Blue, [0.45, 0.75, 1.0]),
        (HighlightColor::Pink, [1.0, 0.55, 0.75]),
    ];
    let Some(s) = raw else {
        return Ok((HighlightColor::Yellow, false));
    };
    let rgb = parse_hex_color(s).ok_or_else(|| {
        (
            SkipReason::InvalidGeometryOrFormat,
            format!("invalid color '{s}'"),
        )
    })?;
    let mut best = PALETTE[0];
    let mut best_d = f32::INFINITY;
    for entry in PALETTE {
        let d = sq_dist(rgb, entry.1);
        if d < best_d {
            best_d = d;
            best = entry;
        }
    }
    let adjusted = !within_tolerance(rgb, best.1);
    Ok((best.0, adjusted))
}

fn map_ink_style(
    raw_color: Option<&str>,
    raw_width: Option<&str>,
    extra_adjusted: bool,
) -> Result<(InkColor, InkWidth, bool), (SkipReason, String)> {
    const PALETTE: [(InkColor, [f32; 3]); 4] = [
        (InkColor::Black, [0.0, 0.0, 0.0]),
        (InkColor::Red, [0.85, 0.1, 0.1]),
        (InkColor::Blue, [0.1, 0.3, 0.85]),
        (InkColor::Green, [0.1, 0.6, 0.2]),
    ];
    let mut adjusted = extra_adjusted;

    let color = if let Some(s) = raw_color {
        let rgb = parse_hex_color(s).ok_or_else(|| {
            (
                SkipReason::InvalidGeometryOrFormat,
                format!("invalid color '{s}'"),
            )
        })?;
        let mut best = PALETTE[0];
        let mut best_d = f32::INFINITY;
        for entry in PALETTE {
            let d = sq_dist(rgb, entry.1);
            if d < best_d {
                best_d = d;
                best = entry;
            }
        }
        if !within_tolerance(rgb, best.1) {
            adjusted = true;
        }
        best.0
    } else {
        InkColor::Black
    };

    let width = if let Some(w_str) = raw_width {
        let w: f32 = w_str.trim().parse().map_err(|_| {
            (
                SkipReason::InvalidGeometryOrFormat,
                format!("invalid border width '{w_str}'"),
            )
        })?;
        if !w.is_finite() || w < 0.0 || w > MAX_PAGE_SIDE_PT {
            return Err((
                SkipReason::InvalidGeometryOrFormat,
                format!("out-of-range border width '{w_str}'"),
            ));
        }
        const WIDTHS: [(InkWidth, f32); 3] = [
            (InkWidth::Thin, 1.0),
            (InkWidth::Medium, 2.5),
            (InkWidth::Thick, 5.0),
        ];
        let mut best = WIDTHS[0];
        let mut best_d = f32::INFINITY;
        for entry in WIDTHS {
            let d = (w - entry.1).abs();
            if d < best_d {
                best_d = d;
                best = entry;
            }
        }
        if best_d > COLOR_TOLERANCE {
            adjusted = true;
        }
        best.0
    } else {
        InkWidth::Medium
    };

    Ok((color, width, adjusted))
}

fn validate_font_family(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DISPLAY_NAME_BYTES as usize || trimmed.len() > 128
    {
        return None;
    }
    if trimmed.contains(['\\', '/', ':', '\0', '\r', '\n', '\t']) {
        return None;
    }
    let cleaned = clean_display_text(trimmed, 128);
    if cleaned != trimmed || cleaned.trim().is_empty() {
        return None;
    }
    Some(cleaned)
}

fn parse_da_font_alias(name: &str) -> &str {
    match name {
        "Helv" | "Helvetica" => "Arial",
        "Cour" | "Courier" => "Courier New",
        "TiRo" | "Times-Roman" => "Times New Roman",
        other => other,
    }
}

fn parse_freetext_style(b: &AnnotBuilder) -> (Option<String>, TextBoxStyle, bool) {
    let mut size: Option<f32> = None;
    let mut color: Option<Rgb> = None;
    let mut align: Option<TextAlign> = None;
    let mut font_family: Option<String> = None;
    let mut adjusted = false;

    if let Some(ds) = b.default_style.as_deref() {
        for decl in ds.split(';') {
            let Some((k, v)) = decl.split_once(':') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "font" => {
                    // CSS shorthand e.g. "normal bold 12pt 'Microsoft JhengHei', sans-serif"
                    let mut after_size = String::new();
                    let mut found_size = false;
                    for tok in val.split_whitespace() {
                        if !found_size {
                            if let Some(pt) = parse_font_size_token(tok) {
                                size = Some(pt);
                                found_size = true;
                                continue;
                            }
                        } else {
                            if !after_size.is_empty() {
                                after_size.push(' ');
                            }
                            after_size.push_str(tok);
                        }
                    }
                    if !after_size.is_empty() {
                        if let Some(first_fam) = after_size.split(',').next() {
                            match validate_font_family(first_fam) {
                                Some(valid) => font_family = Some(valid),
                                None => adjusted = true,
                            }
                        }
                    }
                }
                "font-family" => {
                    if let Some(first_fam) = val.split(',').next() {
                        match validate_font_family(first_fam) {
                            Some(valid) => font_family = Some(valid),
                            None => adjusted = true,
                        }
                    }
                }
                "font-size" => {
                    if let Some(pt) = parse_font_size_token(val) {
                        size = Some(pt);
                    } else {
                        adjusted = true;
                    }
                }
                "color" => {
                    if let Some(c) = parse_css_color(val) {
                        color = Some(c);
                    } else {
                        adjusted = true;
                    }
                }
                "text-align" => match val.to_ascii_lowercase().as_str() {
                    "left" | "start" => align = Some(TextAlign::Left),
                    "center" => align = Some(TextAlign::Center),
                    "right" | "end" => align = Some(TextAlign::Right),
                    _ => {
                        align = Some(TextAlign::Left);
                        adjusted = true;
                    }
                },
                _ => {}
            }
        }
    }

    if let Some(da) = b.default_appearance.as_deref() {
        let tokens: Vec<&str> = da.split_whitespace().collect();
        for i in 0..tokens.len() {
            match tokens[i] {
                "Tf" if i >= 2 => {
                    if font_family.is_none() {
                        let raw_name = tokens[i - 2].trim_start_matches('/');
                        let aliased = parse_da_font_alias(raw_name);
                        if let Some(valid) = validate_font_family(aliased) {
                            font_family = Some(valid);
                        }
                    }
                    if size.is_none() {
                        if let Ok(sz) = tokens[i - 1].parse::<f32>() {
                            if sz == 0.0 {
                                size = Some(12.0);
                                adjusted = true;
                            } else if sz.is_finite() {
                                size = Some(sz);
                            }
                        }
                    }
                }
                "rg" | "RG" if i >= 3 && color.is_none() => {
                    if let (Ok(r), Ok(g), Ok(bl)) = (
                        tokens[i - 3].parse::<f32>(),
                        tokens[i - 2].parse::<f32>(),
                        tokens[i - 1].parse::<f32>(),
                    ) {
                        if r.is_finite() && g.is_finite() && bl.is_finite() {
                            color = Some(Rgb {
                                r: (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                                g: (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                                b: (bl.clamp(0.0, 1.0) * 255.0).round() as u8,
                            });
                        }
                    }
                }
                "g" | "G" if i >= 1 && color.is_none() => {
                    if let Ok(gray) = tokens[i - 1].parse::<f32>() {
                        if gray.is_finite() {
                            let v = (gray.clamp(0.0, 1.0) * 255.0).round() as u8;
                            color = Some(Rgb { r: v, g: v, b: v });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(just) = b.attrs.get("justification") {
        match just.trim() {
            "0" => align = Some(TextAlign::Left),
            "1" => align = Some(TextAlign::Center),
            "2" => align = Some(TextAlign::Right),
            _ => adjusted = true,
        }
    }

    let raw_size = size.unwrap_or(12.0);
    let clamped_size = if !raw_size.is_finite() || raw_size <= 0.0 {
        adjusted = true;
        12.0
    } else if raw_size < MIN_TEXT_BOX_SIZE || raw_size > MAX_TEXT_BOX_SIZE {
        adjusted = true;
        raw_size.clamp(MIN_TEXT_BOX_SIZE, MAX_TEXT_BOX_SIZE)
    } else {
        raw_size
    };

    let final_color = color.unwrap_or(Rgb { r: 0, g: 0, b: 0 });
    let final_align = align.unwrap_or(TextAlign::Left);

    (
        font_family,
        TextBoxStyle {
            size: clamped_size,
            color: final_color,
            align: final_align,
        },
        adjusted,
    )
}

fn parse_font_size_token(tok: &str) -> Option<f32> {
    let trimmed = tok.trim();
    let num_str = trimmed
        .strip_suffix("pt")
        .or_else(|| trimmed.strip_suffix("PT"))
        .or_else(|| trimmed.strip_suffix("px"))
        .or_else(|| trimmed.strip_suffix("PX"))
        .unwrap_or(trimmed);
    let v: f32 = num_str.trim().parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

fn parse_css_color(val: &str) -> Option<Rgb> {
    let trimmed = val.trim();
    if let Some(rgb) = parse_hex_rgb(trimmed) {
        return Some(rgb);
    }
    if let Some(inner) = trimmed
        .strip_prefix("rgb(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r: u8 = parts[0].trim().parse().ok()?;
            let g: u8 = parts[1].trim().parse().ok()?;
            let b: u8 = parts[2].trim().parse().ok()?;
            return Some(Rgb { r, g, b });
        }
    }
    None
}

fn strip_and_check_bom(raw: &[u8]) -> Result<&[u8], XfdfError> {
    if raw.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) || raw.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        return Err(XfdfError::UnsupportedEncoding("UTF-32 BOM".into()));
    }
    if raw.starts_with(&[0xFE, 0xFF]) {
        return Err(XfdfError::UnsupportedEncoding("UTF-16 BE BOM".into()));
    }
    if raw.starts_with(&[0xFF, 0xFE]) {
        return Err(XfdfError::UnsupportedEncoding("UTF-16 LE BOM".into()));
    }
    if let Some(rest) = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(rest);
    }
    Ok(raw)
}

fn is_xml10_char(c: char) -> bool {
    let cp = c as u32;
    matches!(cp, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn validate_xml_chars(s: &str) -> Result<(), XfdfError> {
    for c in s.chars() {
        if !is_xml10_char(c) {
            return Err(XfdfError::InvalidXmlChar(c as u32));
        }
    }
    Ok(())
}

fn validate_xml_chars_bytes(bytes: &[u8]) -> Result<(), XfdfError> {
    let s = std::str::from_utf8(bytes).map_err(|_| XfdfError::InvalidUtf8)?;
    validate_xml_chars(s)
}

fn resolve_general_ref(gen_ref: &quick_xml::events::BytesRef<'_>) -> Result<char, XfdfError> {
    if let Some(ch) = gen_ref.resolve_char_ref().map_err(map_xml_err)? {
        if !is_xml10_char(ch) {
            return Err(XfdfError::InvalidXmlChar(ch as u32));
        }
        return Ok(ch);
    }
    match gen_ref.as_ref() {
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "amp" => Ok('&'),
        "quot" => Ok('"'),
        "apos" => Ok('\''),
        other => Err(XfdfError::EntityForbidden(format!("&{other};"))),
    }
}

fn decode_local_name(s: &str) -> Result<String, XfdfError> {
    validate_xml_chars(s)?;
    Ok(s.to_string())
}

fn collect_checked_attributes(
    elem: &quick_xml::events::BytesStart<'_>,
    report: &mut XfdfReport,
) -> Result<BTreeMap<String, String>, XfdfError> {
    let mut map = BTreeMap::new();
    let mut seen_raw: Vec<String> = Vec::new();
    for attr_res in elem.attributes().with_checks(true) {
        let attr = attr_res.map_err(|e: AttrError| map_xml_err(e.into()))?;
        let raw_key = attr.key.as_ref().to_string();
        if seen_raw.iter().any(|k| k == &raw_key) {
            return Err(XfdfError::MalformedXml("duplicate attribute".into()));
        }
        seen_raw.push(raw_key);

        let key_local = decode_local_name(attr.key.local_name().as_ref())?;
        let normalized = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(map_xml_err)?;
        validate_xml_chars(&normalized)?;

        let key_full = decode_local_name(attr.key.as_ref())?;
        *report.attribute_counts.entry(key_full).or_insert(0) += 1;
        map.insert(key_local, normalized.into_owned());
    }
    Ok(map)
}

fn check_xfdf_root(
    local: &str,
    ns: &ResolveResult<'_>,
    report: &mut XfdfReport,
) -> Result<(), XfdfError> {
    check_xfdf_ns(ns)?;
    if matches!(ns, ResolveResult::Bound(_)) {
        report.used_namespace = true;
    }
    if local != "xfdf" {
        return Err(XfdfError::WrongRoot(local.to_string()));
    }
    Ok(())
}

fn check_xfdf_ns(ns: &ResolveResult<'_>) -> Result<(), XfdfError> {
    match ns {
        ResolveResult::Unbound => Ok(()),
        ResolveResult::Bound(Namespace(ns_str)) if *ns_str == XFDF_NAMESPACE => Ok(()),
        ResolveResult::Bound(Namespace(ns_str)) => {
            Err(XfdfError::WrongNamespace((*ns_str).to_string()))
        }
        ResolveResult::Unknown(s) => Err(XfdfError::WrongNamespace(s.clone())),
    }
}

fn check_richtext_ns(ns: &ResolveResult<'_>) -> Result<(), XfdfError> {
    match ns {
        ResolveResult::Unbound => Ok(()),
        ResolveResult::Bound(Namespace(ns_str))
            if *ns_str == XFDF_NAMESPACE || *ns_str == XHTML_NAMESPACE =>
        {
            Ok(())
        }
        ResolveResult::Bound(Namespace(ns_str)) => {
            Err(XfdfError::WrongNamespace((*ns_str).to_string()))
        }
        ResolveResult::Unknown(s) => Err(XfdfError::WrongNamespace(s.clone())),
    }
}

fn map_xml_err(err: quick_xml::Error) -> XfdfError {
    let msg = err.to_string();
    if msg.contains("UnrecognizedEntity") || msg.contains("entity") {
        return XfdfError::EntityForbidden(msg);
    }
    if msg.contains("character reference") || msg.contains("Null") {
        return XfdfError::InvalidXmlChar(0);
    }
    XfdfError::MalformedXml(msg)
}
