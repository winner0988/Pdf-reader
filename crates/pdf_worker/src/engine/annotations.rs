//! Annotations (B2-07): highlighter marks and notes the user adds, changing them, and removing
//! any annotation a page has. They are standard PDF annotations (`Highlight`, `Text`), with an
//! appearance stream, so other readers show them too.
//!
//! Nothing that tells who made them is written: MuPDF puts no author (`/T`), dates or unique
//! name (`/NM`) in a new annotation, and none is added here.

use std::collections::HashSet;

use ipc_contract::limits::{MAX_ANNOTATIONS_PER_PAGE, MAX_NOTE_TEXT_BYTES, MAX_PAGE_SIDE_PT};
use ipc_contract::text::clean_note_text;
use ipc_contract::types::{
    AnnotationId, AnnotationKind, HighlightColor, HighlightMark, PageAnnotation, Point, Quad, Rect,
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
    let [red, green, blue] = rgb(color);
    AnnotationColor::Rgb { red, green, blue }
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
        let page = self.annotated_page(index)?;
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
            let mut page = self.annotated_page(mark.page)?;
            let quads: Vec<mupdf::Quad> = mark.quads.iter().map(binding_quad).collect();
            let mut annotation = page.add_highlight_annotation(quads)?;
            annotation.set_color(annotation_color(color))?;
            annotation.update()?;
        }
        Ok(())
    }

    /// Puts a note saying `text` at `at` (page space) on page `index`.
    pub fn add_note(&mut self, index: u32, at: Point, text: &str) -> Result<(), EngineError> {
        let mut page = self.annotated_page(index)?;
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

    /// Removes annotation `id` of page `index`, with its pop-up window. What else still points
    /// to them (a reply, the structure tree) points nowhere afterwards, so that a rewrite leaves
    /// them out of the file ([`crate::unlink`]).
    pub fn delete_annotation(&mut self, index: u32, id: AnnotationId) -> Result<(), EngineError> {
        let mut page = self.annotated_page(index)?;
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
        let page = self.annotated_page(index)?;
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
        let page = self.annotated_page(index)?;
        let mut annotation = find(&page, id)?;
        if annotation.r#type()? != PdfAnnotationType::Text {
            return Err(EngineError::InvalidEdit("not a note"));
        }
        annotation.set_contents(text)?;
        annotation.update()?;
        Ok(())
    }

    fn annotated_page(&self, index: u32) -> Result<PdfPage, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self
            .doc
            .load_pdf_page(i32::try_from(index).unwrap_or(i32::MAX))?)
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
