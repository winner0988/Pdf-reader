//! Annotations (B2-07, B2-08): highlighter marks, notes, pen drawings and stamps the user adds,
//! changing them, and removing any annotation a page has. They are standard PDF annotations
//! (`Highlight`, `Text`, `Ink`, `Stamp`), with an appearance stream, so other readers show them
//! too.
//!
//! Nothing that tells who made them is written: MuPDF puts no author (`/T`), dates or unique
//! name (`/NM`) in a new annotation, and none is added here.

use std::collections::HashSet;

use ipc_contract::limits::{
    MAX_ANNOTATION_QUADS, MAX_ANNOTATION_SUMMARY_ITEMS, MAX_ANNOTATION_SUMMARY_TEXT_BYTES,
    MAX_ANNOTATION_SUMMARY_TOTAL_BYTES, MAX_ANNOTATIONS_PER_PAGE, MAX_INK_POINTS,
    MAX_NOTE_TEXT_BYTES, MAX_PAGE_SIDE_PT, MAX_PAGE_TEXT_CHARS,
};
use ipc_contract::text::{clean_display_text, clean_note_text};
use ipc_contract::types::{
    AnnotationId, AnnotationKind, AnnotationSummary, AnnotationSummaryItem, AnnotationSummaryKind,
    HighlightColor, HighlightMark, InkColor, InkWidth, PageAnnotation, PageText as TextLayer,
    Point, Quad, Rect, StampName,
};
use mupdf::Error;
use mupdf::color::AnnotationColor;
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType, PdfPage};

use super::{EngineError, PdfDocument};

/// How large a note's icon is on the page, in points (as Acrobat makes it).
const NOTE_SIDE: f32 = 20.0;

/// How far a color may be from one of the highlighter's and still be taken for it.
const COLOR_TOLERANCE: f32 = 0.02;

/// The highlighter's colors as RGB.
fn rgb(color: HighlightColor) -> [f32; 3] {
    match color {
        HighlightColor::Yellow => [1.0, 0.92, 0.0],
        HighlightColor::Green => [0.55, 0.9, 0.35],
        HighlightColor::Blue => [0.45, 0.75, 1.0],
        HighlightColor::Pink => [1.0, 0.55, 0.75],
    }
}

fn annotation_color(color: HighlightColor) -> AnnotationColor {
    annotation_rgb(rgb(color))
}

fn annotation_rgb([red, green, blue]: [f32; 3]) -> AnnotationColor {
    AnnotationColor::Rgb { red, green, blue }
}

/// The pen colors as RGB.
fn ink_rgb(color: InkColor) -> [f32; 3] {
    match color {
        InkColor::Black => [0.0, 0.0, 0.0],
        InkColor::Red => [0.85, 0.1, 0.1],
        InkColor::Blue => [0.1, 0.3, 0.85],
        InkColor::Green => [0.1, 0.6, 0.2],
    }
}

/// How thick the pen draws, in points.
fn ink_line_width(width: InkWidth) -> f32 {
    match width {
        InkWidth::Thin => 1.0,
        InkWidth::Medium => 2.5,
        InkWidth::Thick => 5.0,
    }
}

/// The name PDF gives a standard stamp (readers draw the stamp from it), and the color MuPDF
/// draws it in.
fn stamp_style(stamp: StampName) -> (&'static str, [f32; 3]) {
    const RED: [f32; 3] = [0.8, 0.0, 0.0];
    const GREEN: [f32; 3] = [0.0, 0.5, 0.1];
    const BLUE: [f32; 3] = [0.0, 0.2, 0.7];
    match stamp {
        StampName::Approved => ("Approved", GREEN),
        StampName::NotApproved => ("NotApproved", RED),
        StampName::Draft => ("Draft", RED),
        StampName::Final => ("Final", BLUE),
        StampName::Confidential => ("Confidential", RED),
        StampName::ForComment => ("ForComment", BLUE),
        StampName::AsIs => ("AsIs", BLUE),
        StampName::TopSecret => ("TopSecret", RED),
    }
}

/// Which of the highlighter's colors `color` is, if any.
fn highlight_color(color: AnnotationColor) -> Option<HighlightColor> {
    let AnnotationColor::Rgb { red, green, blue } = color else {
        return None;
    };
    [
        HighlightColor::Yellow,
        HighlightColor::Green,
        HighlightColor::Blue,
        HighlightColor::Pink,
    ]
    .into_iter()
    .find(|&candidate| {
        rgb(candidate)
            .into_iter()
            .zip([red, green, blue])
            .all(|(want, have)| (want - have).abs() <= COLOR_TOLERANCE)
    })
}

fn binding_point(point: Point) -> mupdf::Point {
    mupdf::Point {
        x: point.x,
        y: point.y,
    }
}

pub(super) fn binding_rect(rect: Rect) -> mupdf::Rect {
    mupdf::Rect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x1,
        y1: rect.y1,
    }
}

fn binding_quad(quad: &Quad) -> mupdf::Quad {
    mupdf::Quad {
        ul: binding_point(quad.ul),
        ur: binding_point(quad.ur),
        ll: binding_point(quad.ll),
        lr: binding_point(quad.lr),
    }
}

impl PdfDocument {
    /// The annotations of page `index` that the app shows and edits: links, form fields and
    /// pop-up windows are not among them (MuPDF lists links and fields apart). At most
    /// `MAX_ANNOTATIONS_PER_PAGE`; one that cannot be read is skipped.
    pub fn page_annotations(&self, index: u32) -> Result<Vec<PageAnnotation>, EngineError> {
        let page = self.pdf_page(index)?;
        let rotation = page.rotation().unwrap_or(0);
        let mut annotations = Vec::new();
        for annotation in page.annotations() {
            if annotations.len() == MAX_ANNOTATIONS_PER_PAGE as usize {
                break;
            }
            if let Ok(Some(entry)) = entry(&annotation, rotation) {
                annotations.push(entry);
            }
        }
        Ok(annotations)
    }

    /// Marks text with the highlighter: a `Highlight` annotation on each page of `marks`, over
    /// its quads (page space). Every page is checked first, so the document is unchanged when
    /// one does not fit; MuPDF failing halfway leaves it to be opened again (see `Edit`).
    pub fn add_highlights(
        &mut self,
        marks: &[HighlightMark],
        color: HighlightColor,
    ) -> Result<(), EngineError> {
        let count = self.page_count()?;
        if marks.is_empty() || marks.iter().any(|mark| mark.quads.is_empty()) {
            return Err(EngineError::InvalidEdit("a highlight covers nothing"));
        }
        if marks
            .iter()
            .map(|mark| mark.page)
            .collect::<HashSet<_>>()
            .len()
            != marks.len()
        {
            return Err(EngineError::InvalidEdit("a page appears twice"));
        }
        if let Some(mark) = marks.iter().find(|mark| mark.page >= count) {
            return Err(EngineError::PageOutOfRange(mark.page));
        }
        for mark in marks {
            let mut page = self.pdf_page(mark.page)?;
            let quads: Vec<mupdf::Quad> = mark.quads.iter().map(binding_quad).collect();
            let mut annotation = page.add_highlight_annotation(quads)?;
            annotation.set_color(annotation_color(color))?;
            annotation.update()?;
        }
        Ok(())
    }

    /// Puts a note saying `text` at `at` (page space) on page `index`.
    pub fn add_note(&mut self, index: u32, at: Point, text: &str) -> Result<(), EngineError> {
        let mut page = self.pdf_page(index)?;
        let rect = mupdf::Rect {
            x0: at.x,
            y0: at.y,
            x1: at.x + NOTE_SIDE,
            y1: at.y + NOTE_SIDE,
        };
        let mut annotation = page.add_text_annotation(rect, text)?;
        annotation.update()?;
        Ok(())
    }

    /// Draws `strokes` (page space) with the pen on page `index`: one `Ink` annotation, in `color`
    /// and `width`. Its line has round ends, so a stroke of one point is a dot.
    pub fn add_ink(
        &mut self,
        index: u32,
        strokes: &[Vec<Point>],
        color: InkColor,
        width: InkWidth,
    ) -> Result<(), EngineError> {
        if strokes.is_empty() || strokes.iter().any(Vec::is_empty) {
            return Err(EngineError::InvalidEdit(
                "a drawing needs strokes with points",
            ));
        }
        let mut page = self.pdf_page(index)?;
        let strokes: Vec<Vec<mupdf::Point>> = strokes
            .iter()
            .map(|stroke| stroke.iter().copied().map(binding_point).collect())
            .collect();
        let mut annotation = page.add_ink_annotation(strokes)?;
        annotation.set_color(annotation_rgb(ink_rgb(color)))?;
        annotation.set_border_width(ink_line_width(width))?;
        annotation.update()?;
        Ok(())
    }

    /// Puts the standard stamp `stamp` over `rect` (page space) on page `index`. MuPDF draws it in
    /// English, as other readers draw the stamps of these names, keeping its shape: it fills the
    /// rectangle as far as that shape allows.
    pub fn add_stamp(
        &mut self,
        index: u32,
        rect: Rect,
        stamp: StampName,
    ) -> Result<(), EngineError> {
        let (name, color) = stamp_style(stamp);
        let mut page = self.pdf_page(index)?;
        let mut annotation = page.add_stamp_annotation(binding_rect(rect), name)?;
        annotation.set_color(annotation_rgb(color))?;
        annotation.update()?;
        Ok(())
    }

    /// Moves and resizes the drawing, stamp, text box or redaction mark `id` of page `index` to
    /// `rect` (page space, as listed). A stamp keeps its shape inside the rectangle; a drawing keeps
    /// the thickness of its line, and its points are laid out again in the rectangle; a text box
    /// sets its words again.
    pub fn set_annotation_rect(
        &mut self,
        index: u32,
        id: AnnotationId,
        rect: Rect,
    ) -> Result<(), EngineError> {
        let page = self.pdf_page(index)?;
        let mut annotation = find(&page, id)?;
        match annotation.r#type()? {
            PdfAnnotationType::Stamp => {
                // A picture that is turned keeps its shape and its appearance (B3-01).
                if let Some(fitted) = super::stamp_image::fit_turned(&annotation, rect) {
                    annotation.set_rect(binding_rect(fitted))?;
                } else {
                    annotation.set_rect(binding_rect(rect))?;
                    annotation.update()?;
                }
            }
            PdfAnnotationType::Ink => place_ink(&mut annotation, rect)?,
            // A redaction mark (B2-19) is only an area until it is applied.
            PdfAnnotationType::Redact => {
                annotation.set_rect(binding_rect(rect))?;
                annotation.update()?;
            }
            // Never through MuPDF's setters: they would draw the box black again.
            PdfAnnotationType::FreeText if super::text_box::is_text_box(&annotation) => {
                self.move_text_box(index, id, rect)?;
            }
            _ => {
                return Err(EngineError::InvalidEdit(
                    "only a drawing, a stamp, a text box or a redaction mark can be moved",
                ));
            }
        }
        Ok(())
    }

    /// Removes annotation `id` of page `index`, with its pop-up window. What else still points
    /// to them (a reply, the structure tree) points nowhere afterwards, so that a rewrite leaves
    /// them out of the file ([`crate::unlink`]).
    pub fn delete_annotation(&mut self, index: u32, id: AnnotationId) -> Result<(), EngineError> {
        let mut page = self.pdf_page(index)?;
        let annotation = find(&page, id)?;
        let mut gone = HashSet::from([annotation.xref()?]);
        if let Some(popup) = annotation.object().get_dict("Popup")?
            && let Ok(number) = popup.as_indirect()
            && number > 0
        {
            gone.insert(number);
        }
        page.delete_annotation(annotation)?;
        crate::unlink::unlink(&self.doc, &gone)
    }

    /// Gives the highlighter mark `id` of page `index` another color.
    pub fn set_highlight_color(
        &mut self,
        index: u32,
        id: AnnotationId,
        color: HighlightColor,
    ) -> Result<(), EngineError> {
        let page = self.pdf_page(index)?;
        let mut annotation = find(&page, id)?;
        if annotation.r#type()? != PdfAnnotationType::Highlight {
            return Err(EngineError::InvalidEdit("not a highlighter mark"));
        }
        annotation.set_color(annotation_color(color))?;
        annotation.update()?;
        Ok(())
    }

    /// Replaces what the note `id` of page `index` says.
    pub fn set_note_text(
        &mut self,
        index: u32,
        id: AnnotationId,
        text: &str,
    ) -> Result<(), EngineError> {
        let page = self.pdf_page(index)?;
        let mut annotation = find(&page, id)?;
        if annotation.r#type()? != PdfAnnotationType::Text {
            return Err(EngineError::InvalidEdit("not a note"));
        }
        annotation.set_contents(text)?;
        annotation.update()?;
        Ok(())
    }

    /// The annotations of every page for a summary (B3-18, docs/architecture/annotation-summary.md):
    /// highlighter marks with the text they cover, notes and text boxes with what they say, pen
    /// drawings and stamps by kind only. In page order, and on a page as it is read: from the top
    /// down, then from the left, each from where it starts. MuPDF's page space is the page as it
    /// is shown (its `/Rotate` already applied), so that is the order on a turned page too.
    ///
    /// Leaves out, and says so, what is beyond `MAX_ANNOTATIONS_PER_PAGE` on a page, and stops at
    /// `MAX_ANNOTATION_SUMMARY_ITEMS` items, `MAX_ANNOTATION_SUMMARY_TOTAL_BYTES` of text, or
    /// when finding the text under the marks has taken `MAX_SUMMARY_WORK` steps (a file made to
    /// keep the worker busy).
    pub fn annotation_summary(&self) -> Result<AnnotationSummary, EngineError> {
        let mut summary = SummaryBuilder::new(
            MAX_ANNOTATION_SUMMARY_ITEMS as usize,
            MAX_ANNOTATION_SUMMARY_TOTAL_BYTES as usize,
        );
        let mut work = MAX_SUMMARY_WORK;
        for page_index in 0..self.page_count()? {
            let page = self.pdf_page(page_index)?;
            let mut found: Vec<(PdfAnnotation, SummaryEntry)> = Vec::new();
            for annotation in page.annotations() {
                let Some(entry) = summary_entry(&annotation) else {
                    continue;
                };
                if found.len() == MAX_ANNOTATIONS_PER_PAGE as usize {
                    summary.left_out();
                    break;
                }
                found.push((annotation, entry));
            }
            if found.is_empty() {
                continue;
            }
            // Within half a point counts as the same height: marks on one line are read from
            // the left.
            found.sort_by(|(_, a), (_, b)| {
                a.start
                    .y
                    .round()
                    .total_cmp(&b.start.y.round())
                    .then(a.start.x.total_cmp(&b.start.x))
                    .then(a.number.cmp(&b.number))
            });
            // The text the user selects from (B2-10's recognised text on a scanned page), which
            // is where a highlighter mark is made.
            let layer = found
                .iter()
                .any(|(_, entry)| entry.kind == AnnotationSummaryKind::Highlight)
                .then(|| {
                    self.page_text(page_index, MAX_PAGE_TEXT_CHARS as usize)
                        .ok()
                })
                .flatten();
            for (annotation, entry) in found {
                let text = match entry.kind {
                    AnnotationSummaryKind::Highlight => match &layer {
                        Some(layer) => match covered_text(layer, &entry, &mut work) {
                            Some(text) => text,
                            None => {
                                summary.left_out();
                                return Ok(summary.finish());
                            }
                        },
                        None => String::new(),
                    },
                    AnnotationSummaryKind::Note | AnnotationSummaryKind::TextBox => annotation
                        .contents()
                        .ok()
                        .flatten()
                        .map(|contents| {
                            clean_note_text(contents, MAX_ANNOTATION_SUMMARY_TEXT_BYTES as usize)
                        })
                        .unwrap_or_default(),
                    // Listed by kind only: what they show is not text.
                    AnnotationSummaryKind::Ink | AnnotationSummaryKind::Stamp => String::new(),
                };
                if !summary.push(AnnotationSummaryItem {
                    page_index,
                    kind: entry.kind,
                    rect: entry.rect,
                    text,
                }) {
                    return Ok(summary.finish());
                }
            }
        }
        Ok(summary.finish())
    }

    pub(super) fn pdf_page(&self, index: u32) -> Result<PdfPage, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self
            .doc
            .load_pdf_page(i32::try_from(index).unwrap_or(i32::MAX))?)
    }
}

/// Lays the points of the drawing `annotation` out again so that it fills `rect` (page space, a
/// rectangle as listed: the bounds of the drawing, with the margin MuPDF leaves around its line).
///
/// The margin is kept: the points go into `rect` less what the bounds have more than the points.
/// An axis along which the points have no extent (a straight line) keeps none: the line goes to
/// the middle of the rectangle.
fn place_ink(annotation: &mut PdfAnnotation, rect: Rect) -> Result<(), EngineError> {
    let strokes = annotation.ink_list()?;
    if strokes.iter().map(Vec::len).sum::<usize>() > MAX_INK_POINTS as usize {
        return Err(EngineError::InvalidEdit("the drawing has too many points"));
    }
    let mut points = strokes.iter().flatten();
    let Some(first) = points.next() else {
        return Err(EngineError::InvalidEdit("the drawing has no points"));
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for point in points {
        (x0, x1) = (x0.min(point.x), x1.max(point.x));
        (y0, y1) = (y0.min(point.y), y1.max(point.y));
    }
    let bounds = annotation.bounds()?;
    let map_x = axis_map(x0, x1, bounds.x0, bounds.x1, rect.x0, rect.x1);
    let map_y = axis_map(y0, y1, bounds.y0, bounds.y1, rect.y0, rect.y1);
    let placed: Vec<Vec<mupdf::Point>> = strokes
        .iter()
        .map(|stroke| {
            stroke
                .iter()
                .map(|point| mupdf::Point {
                    x: map_x(point.x),
                    y: map_y(point.y),
                })
                .collect()
        })
        .collect();
    if placed
        .iter()
        .flatten()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(EngineError::InvalidEdit("the drawing does not fit there"));
    }
    annotation.set_ink_list(placed)?;
    annotation.update()?;
    Ok(())
}

/// Where the values `from..=to` of one axis of a drawing go: into `new_from..=new_to` less the
/// margin the drawing bounds (`outer_from..=outer_to`) have around them.
fn axis_map(
    from: f32,
    to: f32,
    outer_from: f32,
    outer_to: f32,
    new_from: f32,
    new_to: f32,
) -> impl Fn(f32) -> f32 {
    let (target_from, target_to) = (new_from + (from - outer_from), new_to - (outer_to - to));
    // A rectangle smaller than the margins: the points gather in its middle.
    let (target_from, target_to) = if target_to < target_from {
        let middle = (new_from + new_to) / 2.0;
        (middle, middle)
    } else {
        (target_from, target_to)
    };
    let extent = to - from;
    move |value| {
        if extent > f32::EPSILON {
            target_from + (value - from) * (target_to - target_from) / extent
        } else {
            (target_from + target_to) / 2.0
        }
    }
}

/// Annotation `id` of `page`, other than a pop-up window.
pub(super) fn find(page: &PdfPage, id: AnnotationId) -> Result<PdfAnnotation, EngineError> {
    let wanted = i32::try_from(id.0).map_err(|_| EngineError::InvalidEdit("no such annotation"))?;
    page.annotations()
        .find(|annotation| {
            annotation.xref().is_ok_and(|number| number == wanted)
                && annotation
                    .r#type()
                    .is_ok_and(|kind| kind != PdfAnnotationType::Popup)
        })
        .ok_or(EngineError::InvalidEdit("no such annotation on the page"))
}

/// What the app lists of `annotation`, on a page turned `page_rotation` degrees; `None` for a
/// pop-up window, or one without a number.
fn entry(annotation: &PdfAnnotation, page_rotation: i32) -> Result<Option<PageAnnotation>, Error> {
    let kind = match annotation.r#type()? {
        PdfAnnotationType::Popup => return Ok(None),
        PdfAnnotationType::Highlight => AnnotationKind::Highlight,
        PdfAnnotationType::Text => AnnotationKind::Note,
        PdfAnnotationType::Ink => AnnotationKind::Ink,
        PdfAnnotationType::Stamp => AnnotationKind::Stamp,
        PdfAnnotationType::FreeText if super::text_box::is_text_box(annotation) => {
            AnnotationKind::TextBox
        }
        PdfAnnotationType::Redact => AnnotationKind::Redaction,
        _ => AnnotationKind::Other,
    };
    let Ok(number) = u32::try_from(annotation.xref()?) else {
        return Ok(None);
    };
    if number == 0 {
        return Ok(None);
    }
    let bounds = annotation.bounds()?;
    let rect = Rect {
        x0: bounds.x0,
        y0: bounds.y0,
        x1: bounds.x1,
        y1: bounds.y1,
    };
    // One the main process would refuse is left out, not the whole page's list.
    if [rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_PAGE_SIDE_PT)
    {
        return Ok(None);
    }
    let color = match kind {
        AnnotationKind::Highlight => annotation.color()?.and_then(highlight_color),
        _ => None,
    };
    let text = match kind {
        AnnotationKind::Note | AnnotationKind::TextBox => annotation
            .contents()
            .ok()
            .flatten()
            .map(|contents| clean_note_text(contents, MAX_NOTE_TEXT_BYTES as usize)),
        _ => None,
    };
    let picture = match kind {
        AnnotationKind::Stamp => super::stamp_image::picture_view(annotation, page_rotation),
        _ => None,
    };
    Ok(Some(PageAnnotation {
        id: AnnotationId(number),
        kind,
        rect,
        color,
        text,
        picture,
    }))
}

/// How many characters a summary may test against the quads of highlighter marks in all (one
/// step each), to find the text they cover: far more than the marks of any real document need
/// (a page has some thousands of characters, a mark a few quads, and only the lines a mark
/// reaches are tested), and about a second's work.
const MAX_SUMMARY_WORK: u64 = 200_000_000;

/// What a summary needs of an annotation besides its text.
struct SummaryEntry {
    kind: AnnotationSummaryKind,
    rect: Rect,
    /// A highlighter mark's quads, at most `MAX_ANNOTATION_QUADS` (none: its rectangle counts).
    quads: Vec<Quad>,
    /// Where it starts: the top left of a mark's first quad (the line it starts on), of the
    /// rectangle of anything else.
    start: Point,
    /// Its object number: the order of two that start at the same place.
    number: u32,
}

/// What a summary lists of `annotation`; `None` for a kind it leaves out (links, fields, pop-ups,
/// shapes), one without a number, or one the main process would refuse.
fn summary_entry(annotation: &PdfAnnotation) -> Option<SummaryEntry> {
    let kind = match annotation.r#type().ok()? {
        PdfAnnotationType::Highlight => AnnotationSummaryKind::Highlight,
        PdfAnnotationType::Text => AnnotationSummaryKind::Note,
        PdfAnnotationType::Ink => AnnotationSummaryKind::Ink,
        PdfAnnotationType::Stamp => AnnotationSummaryKind::Stamp,
        PdfAnnotationType::FreeText if super::text_box::is_text_box(annotation) => {
            AnnotationSummaryKind::TextBox
        }
        _ => return None,
    };
    let number = u32::try_from(annotation.xref().ok()?)
        .ok()
        .filter(|number| *number != 0)?;
    let bounds = annotation.bounds().ok()?;
    let rect = Rect {
        x0: bounds.x0,
        y0: bounds.y0,
        x1: bounds.x1,
        y1: bounds.y1,
    };
    if ![rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_PAGE_SIDE_PT)
    {
        return None;
    }
    let quads: Vec<Quad> = match annotation.quad_count() {
        Ok(count) if kind == AnnotationSummaryKind::Highlight && count <= MAX_ANNOTATION_QUADS => {
            annotation
                .quads()
                .unwrap_or_default()
                .into_iter()
                .map(super::contract_quad)
                .collect()
        }
        _ => Vec::new(),
    };
    let start = quads
        .first()
        .map(|quad| {
            let (x0, y0, _, _) = quad_bounds(quad);
            Point { x: x0, y: y0 }
        })
        .filter(|start| start.x.is_finite() && start.y.is_finite())
        .unwrap_or(Point {
            x: rect.x0,
            y: rect.y0,
        });
    Some(SummaryEntry {
        kind,
        rect,
        quads,
        start,
        number,
    })
}

/// The items of a summary, up to a number of items and of text bytes in all.
struct SummaryBuilder {
    items: Vec<AnnotationSummaryItem>,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
    truncated: bool,
}

impl SummaryBuilder {
    fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            items: Vec::new(),
            bytes: 0,
            max_items,
            max_bytes,
            truncated: false,
        }
    }

    /// Adds `item`; `false`, leaving it out, when it does not fit (and nothing more will).
    fn push(&mut self, item: AnnotationSummaryItem) -> bool {
        let bytes = self.bytes.saturating_add(item.text.len());
        if self.items.len() == self.max_items || bytes > self.max_bytes {
            self.truncated = true;
            return false;
        }
        self.bytes = bytes;
        self.items.push(item);
        true
    }

    /// Some annotation was left out.
    fn left_out(&mut self) {
        self.truncated = true;
    }

    fn finish(self) -> AnnotationSummary {
        AnnotationSummary {
            items: self.items,
            truncated: self.truncated,
        }
    }
}

/// The text the highlighter mark `entry` covers: the characters of the page's text whose middle
/// lies in one of its quads (or, without any, in its rectangle), in the page's order. Where the
/// covered text skips some (the end of a line, a gap), the pieces are joined as the lines of a
/// paragraph are (a space, but none between Chinese letters). Cut to
/// `MAX_ANNOTATION_SUMMARY_TEXT_BYTES`. `None` when testing the characters would take more than
/// the `work` left.
fn covered_text(layer: &TextLayer, entry: &SummaryEntry, work: &mut u64) -> Option<String> {
    let rect = entry.rect;
    let reach = entry
        .quads
        .iter()
        .map(quad_bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        .unwrap_or((rect.x0, rect.y0, rect.x1, rect.y1));
    let covered = |point: Point| {
        if entry.quads.is_empty() {
            (rect.x0..=rect.x1).contains(&point.x) && (rect.y0..=rect.y1).contains(&point.y)
        } else {
            entry.quads.iter().any(|quad| quad_contains(quad, point))
        }
    };
    let max = MAX_ANNOTATION_SUMMARY_TEXT_BYTES as usize;
    let mut text = String::new();
    let mut last: Option<char> = None;
    'lines: for line in &layer.lines {
        // Only the lines the mark reaches are tested.
        let (x0, y0, x1, y1) = quad_bounds(&line.quad);
        if x1 < reach.0 || reach.2 < x0 || y1 < reach.1 || reach.3 < y0 {
            continue;
        }
        let chars: Vec<char> = line.text.chars().collect();
        if line.edges.len() != chars.len() + 1 {
            continue;
        }
        let steps = (chars.len() as u64).saturating_mul(entry.quads.len().max(1) as u64);
        *work = work.checked_sub(steps)?;
        let (along, across) = line_axes(&line.quad);
        let mut gap = true;
        for (index, c) in chars.into_iter().enumerate() {
            let middle = (line.edges[index] + line.edges[index + 1]) / 2.0;
            let point = Point {
                x: line.quad.ul.x + along.x * middle + across.x / 2.0,
                y: line.quad.ul.y + along.y * middle + across.y / 2.0,
            };
            if !covered(point) {
                gap = true;
                continue;
            }
            if gap
                && let Some(last) = last
                && super::docx::space_between(last, c)
            {
                text.push(' ');
            }
            text.push(c);
            last = Some(c);
            gap = false;
            if text.len() > max {
                break 'lines;
            }
        }
    }
    Some(clean_display_text(&text, max))
}

/// The smallest and largest x and y of `quad`'s corners: x0, y0, x1, y1.
fn quad_bounds(quad: &Quad) -> (f32, f32, f32, f32) {
    let corners = [quad.ul, quad.ur, quad.ll, quad.lr];
    corners.iter().fold(
        (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), corner| {
            (
                x0.min(corner.x),
                y0.min(corner.y),
                x1.max(corner.x),
                y1.max(corner.y),
            )
        },
    )
}

/// The unit vector along `quad`'s top edge (the writing direction; across the page when that
/// edge has no length), and the vector from its top-left to its bottom-left corner.
fn line_axes(quad: &Quad) -> (Point, Point) {
    let (dx, dy) = (quad.ur.x - quad.ul.x, quad.ur.y - quad.ul.y);
    let length = dx.hypot(dy);
    let along = if length > 1e-3 {
        Point {
            x: dx / length,
            y: dy / length,
        }
    } else {
        Point { x: 1.0, y: 0.0 }
    };
    let across = Point {
        x: quad.ll.x - quad.ul.x,
        y: quad.ll.y - quad.ul.y,
    };
    (along, across)
}

/// Whether `point` lies in `quad` (convex, its corners in either turning direction) or within
/// half a point of it. A quad with a side of no length covers nothing.
fn quad_contains(quad: &Quad, point: Point) -> bool {
    const SLACK: f32 = 0.5;
    let corners = [quad.ul, quad.ur, quad.lr, quad.ll];
    let mut sides = [0.0f32; 4];
    for (index, side) in sides.iter_mut().enumerate() {
        let (from, to) = (corners[index], corners[(index + 1) % 4]);
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = dx.hypot(dy);
        if length.is_nan() || length <= 1e-3 {
            return false;
        }
        // How far the point is to one side of this edge.
        *side = (dx * (point.y - from.y) - dy * (point.x - from.x)) / length;
    }
    sides.iter().all(|side| *side >= -SLACK) || sides.iter().all(|side| *side <= SLACK)
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{
        AnnotationSummaryItem, AnnotationSummaryKind, PageText, Point, Quad, Rect, TextLine,
    };

    use super::{SummaryBuilder, SummaryEntry, covered_text, quad_contains};

    fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> Quad {
        Quad {
            ul: Point { x: x0, y: y0 },
            ur: Point { x: x1, y: y0 },
            ll: Point { x: x0, y: y1 },
            lr: Point { x: x1, y: y1 },
        }
    }

    /// A line of `text` across the page from (`x`, `y`), each character 10 points wide and the
    /// line 12 high.
    fn line(text: &str, x: f32, y: f32) -> TextLine {
        let count = text.chars().count();
        TextLine {
            text: text.to_owned(),
            quad: quad(x, y, x + 10.0 * count as f32, y + 12.0),
            edges: (0..=count).map(|index| 10.0 * index as f32).collect(),
        }
    }

    fn layer(lines: Vec<TextLine>) -> PageText {
        PageText {
            lines,
            truncated: false,
            recognised: false,
        }
    }

    fn mark(quads: Vec<Quad>, rect: Rect) -> SummaryEntry {
        SummaryEntry {
            kind: AnnotationSummaryKind::Highlight,
            rect,
            quads,
            start: Point {
                x: rect.x0,
                y: rect.y0,
            },
            number: 1,
        }
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    fn covered(layer: &PageText, quads: Vec<Quad>) -> String {
        let mut work = u64::MAX;
        covered_text(layer, &mark(quads, rect(0.0, 0.0, 0.0, 0.0)), &mut work).expect("text")
    }

    #[test]
    fn a_mark_covers_the_characters_whose_middle_is_in_it_joined_as_a_paragraph() {
        let page = layer(vec![
            line("Hello world", 0.0, 0.0),
            line("again here", 0.0, 20.0),
            line("隱私優先", 0.0, 40.0),
            line("閱讀器", 0.0, 60.0),
        ]);
        // The end of one line and the start of the next: a space between them.
        assert_eq!(
            covered(
                &page,
                vec![quad(58.0, -1.0, 112.0, 13.0), quad(-2.0, 19.0, 51.0, 33.0)]
            ),
            "world again"
        );
        // None between Chinese letters.
        assert_eq!(
            covered(
                &page,
                vec![quad(20.0, 40.0, 40.0, 52.0), quad(0.0, 60.0, 20.0, 72.0)]
            ),
            "優先閱讀"
        );
        // Two pieces of one line: what is between them is left out, a space marks the gap.
        assert_eq!(
            covered(
                &page,
                vec![quad(0.0, 0.0, 20.0, 12.0), quad(90.0, 0.0, 110.0, 12.0)]
            ),
            "He ld"
        );
        // A character whose middle (65 for the "w") is beyond half a point of the mark is not
        // covered.
        assert_eq!(covered(&page, vec![quad(65.6, 0.0, 110.0, 12.0)]), "orld");
        assert_eq!(covered(&page, vec![quad(65.4, 0.0, 110.0, 12.0)]), "world");
        // Without quads, the mark's rectangle counts.
        let mut work = u64::MAX;
        let whole = covered_text(
            &page,
            &mark(Vec::new(), rect(0.0, 18.0, 100.0, 34.0)),
            &mut work,
        );
        assert_eq!(whole.as_deref(), Some("again here"));
        // A mark that covers no text has none.
        assert_eq!(covered(&page, vec![quad(300.0, 300.0, 400.0, 400.0)]), "");
    }

    #[test]
    fn finding_the_text_stops_when_the_work_runs_out() {
        let page = layer(vec![
            line("Hello world", 0.0, 0.0),
            line("far below", 0.0, 500.0),
        ]);
        // One quad: a step for each character of the lines it reaches, and none for the
        // others.
        let entry = mark(
            vec![quad(60.0, 0.0, 110.0, 12.0)],
            rect(60.0, 0.0, 110.0, 12.0),
        );
        let mut work = 11;
        assert_eq!(
            covered_text(&page, &entry, &mut work).as_deref(),
            Some("world")
        );
        assert_eq!(work, 0);
        let mut work = 10;
        assert_eq!(covered_text(&page, &entry, &mut work), None);
    }

    #[test]
    fn a_turned_quad_covers_what_is_inside_it_and_a_flat_one_nothing() {
        let point = |x, y| Point { x, y };
        // A square turned 45 degrees, its corners in either turning direction.
        let diamond = Quad {
            ul: point(50.0, 0.0),
            ur: point(100.0, 50.0),
            lr: point(50.0, 100.0),
            ll: point(0.0, 50.0),
        };
        let mirrored = Quad {
            ul: diamond.ur,
            ur: diamond.ul,
            lr: diamond.ll,
            ll: diamond.lr,
        };
        for quad in [&diamond, &mirrored] {
            assert!(quad_contains(quad, point(50.0, 50.0)));
            assert!(quad_contains(quad, point(75.0, 25.0)));
            assert!(!quad_contains(quad, point(10.0, 10.0)));
            assert!(!quad_contains(quad, point(95.0, 95.0)));
        }
        assert!(!quad_contains(
            &quad(0.0, 10.0, 100.0, 10.0),
            point(50.0, 10.0)
        ));
        let broken = Quad {
            ul: point(f32::NAN, 0.0),
            ..diamond
        };
        assert!(!quad_contains(&broken, point(50.0, 50.0)));
    }

    #[test]
    fn a_summary_stops_at_its_limits_and_says_so() {
        let item = |text: &str| AnnotationSummaryItem {
            page_index: 0,
            kind: AnnotationSummaryKind::Note,
            rect: rect(0.0, 0.0, 20.0, 20.0),
            text: text.to_owned(),
        };
        let mut summary = SummaryBuilder::new(2, 10);
        assert!(summary.push(item("12345")));
        assert!(summary.push(item("67890")));
        assert!(!summary.push(item("")));
        let full = summary.finish();
        assert_eq!(full.items.len(), 2);
        assert!(full.truncated);

        let mut summary = SummaryBuilder::new(5, 10);
        assert!(summary.push(item("12345678")));
        assert!(!summary.push(item("901")));
        let long = summary.finish();
        assert_eq!(long.items.len(), 1);
        assert!(long.truncated);

        let mut summary = SummaryBuilder::new(5, 10);
        assert!(summary.push(item("short")));
        assert!(!summary.finish().truncated);
        let mut summary = SummaryBuilder::new(5, 10);
        summary.left_out();
        assert!(summary.finish().truncated);
    }
}
