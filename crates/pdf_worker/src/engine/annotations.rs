//! Annotations (B2-07, B2-08): highlighter marks, notes, pen drawings and stamps the user adds,
//! changing them, and removing any annotation a page has. They are standard PDF annotations
//! (`Highlight`, `Text`, `Ink`, `Stamp`), with an appearance stream, so other readers show them
//! too.
//!
//! Nothing that tells who made them is written: MuPDF puts no author (`/T`), dates or unique
//! name (`/NM`) in a new annotation, and none is added here.

use std::collections::HashSet;

use ipc_contract::limits::{
    MAX_ANNOTATIONS_PER_PAGE, MAX_INK_POINTS, MAX_NOTE_TEXT_BYTES, MAX_PAGE_SIDE_PT,
};
use ipc_contract::text::clean_note_text;
use ipc_contract::types::{
    AnnotationId, AnnotationKind, HighlightColor, HighlightMark, InkColor, InkWidth,
    PageAnnotation, Point, Quad, Rect, StampName,
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
        let mut annotations = Vec::new();
        for annotation in page.annotations() {
            if annotations.len() == MAX_ANNOTATIONS_PER_PAGE as usize {
                break;
            }
            if let Ok(Some(entry)) = entry(&annotation) {
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

    /// Moves and resizes the drawing or stamp `id` of page `index` to `rect` (page space, as
    /// listed). A stamp keeps its shape inside the rectangle; a drawing keeps the thickness of its
    /// line, and its points are laid out again in the rectangle.
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
                annotation.set_rect(binding_rect(rect))?;
                annotation.update()?;
            }
            PdfAnnotationType::Ink => place_ink(&mut annotation, rect)?,
            _ => {
                return Err(EngineError::InvalidEdit(
                    "only a drawing or a stamp can be moved",
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
fn find(page: &PdfPage, id: AnnotationId) -> Result<PdfAnnotation, EngineError> {
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

/// What the app lists of `annotation`; `None` for a pop-up window, or one without a number.
fn entry(annotation: &PdfAnnotation) -> Result<Option<PageAnnotation>, Error> {
    let kind = match annotation.r#type()? {
        PdfAnnotationType::Popup => return Ok(None),
        PdfAnnotationType::Highlight => AnnotationKind::Highlight,
        PdfAnnotationType::Text => AnnotationKind::Note,
        PdfAnnotationType::Ink => AnnotationKind::Ink,
        PdfAnnotationType::Stamp => AnnotationKind::Stamp,
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
        AnnotationKind::Note => annotation
            .contents()
            .ok()
            .flatten()
            .map(|contents| clean_note_text(contents, MAX_NOTE_TEXT_BYTES as usize)),
        _ => None,
    };
    Ok(Some(PageAnnotation {
        id: AnnotationId(number),
        kind,
        rect,
        color,
        text,
    }))
}
