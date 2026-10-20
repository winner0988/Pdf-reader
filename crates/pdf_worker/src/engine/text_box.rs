//! Text boxes (B2-28, ADR 0018, docs/architecture/text-box-engine.md): a FreeText annotation whose
//! words are set in a subset of the font the user chose, and whose appearance is written here.
//!
//! Making one takes two steps. [`prepare_text_box`] reads the font (not trusted: only `skrifa` and
//! `subsetter`, which are memory safe, read it) and cuts it to the glyphs of the words, which
//! gives a model; it needs no document. [`PdfDocument::place_text_box`] sets a model in a box on a
//! page. A model is all that is needed to set the words again, so undo, redo and the crash
//! recovery journal keep the model and never the font, and a box can be moved, resized or given
//! another colour from the file alone.
//!
//! The annotation's dictionary and its appearance are written by hand, and MuPDF is never asked
//! to change a text box afterwards: `set_contents`, `set_rect` and `update()` make MuPDF draw the
//! annotation again from `/DA` (Helvetica, black), and the font and colour are gone.

mod font;
mod layout;
mod pdf;

use ipc_contract::limits::{MAX_PAGE_SIDE_PT, MIN_ANNOTATION_SIDE_PT};
use ipc_contract::types::{
    AnnotationId, FontRefusal, Point, Quad, Rect, Rgb, TextAlign, TextBoxStyle,
};
use ipc_contract::validate::Validate;
use ipc_contract::worker::TextBoxModel;
use mupdf::Buffer;
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType};

pub use font::{inspect_font, prepare_text_box};

use mupdf::pdf::PdfObject;

use super::annotations::{binding_rect, find};
use super::{EngineError, PdfDocument};

/// `text` on one line, `size` points high, in `color`, as a form of its own (a watermark, B3-04):
/// the words are set as a text box's are, in a subset of `font`, so that every reader shows them
/// and search finds them. Gives the form, and how wide and how tall it is, in points.
pub(super) fn line_form(
    doc: &mut mupdf::pdf::PdfDocument,
    font: &TextBoxFont,
    text: &str,
    size: f32,
    color: Rgb,
) -> Result<(PdfObject, f32, f32), EngineError> {
    let model = prepare_text_box(font, text)?;
    let words = || EngineError::InvalidEdit("the words of a watermark");
    let glyphs = layout::glyphs(&model).ok_or_else(words)?;
    let line: f32 = glyphs.iter().map(|glyph| glyph.advance * size).sum();
    // As `layout::lay_out` sets a line: its baseline the ascent below the top, and room for what
    // goes below the baseline under it.
    let ascent = (model.ascent as f32 / 1000.0).max(0.8) * size;
    let descent = (model.descent as f32 / 1000.0).clamp(-1.0, -0.05) * size;
    let width = line + 2.0 * layout::PADDING;
    let height = ascent - descent + 2.0 * layout::PADDING;
    let style = TextBoxStyle {
        size,
        color,
        align: TextAlign::Left,
    };
    // A little wider than the line, so that rounding never breaks it in two.
    let laid = layout::lay_out(&model, &style, (width + 1.0, height)).ok_or_else(words)?;
    let subset = font::clean_subset(&model).ok_or(EngineError::Font(FontRefusal::NotAFont))?;
    let (font_object, _) = pdf::write_font(doc, &model, &subset)?;
    let form = pdf::parse(
        doc,
        &format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
             /Resources << /Font << /F0 {} 0 R >> >> >>",
            pdf::object_number(&font_object)?,
        ),
    )?;
    let stream = doc.add_stream(
        &Buffer::from_bytes(laid.content.as_bytes())?,
        Some(&form),
        false,
    )?;
    Ok((stream, width, height))
}

/// What a text box says and how it is set.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBoxSpec<'a> {
    /// Lines are broken at `\n`, and where the box is not wide enough.
    pub text: &'a str,
    /// In points.
    pub size: f32,
    pub color: Rgb,
    pub align: TextAlign,
}

/// A font file, and which font of it is meant (0 unless it is a collection).
#[derive(Debug, Clone, Copy)]
pub struct TextBoxFont<'a> {
    pub bytes: &'a [u8],
    pub face: u32,
    /// What Windows calls it, kept in the text box to find the font again.
    pub name: &'a str,
}

/// What came of a text box that was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBoxMade {
    /// The font's licence allows a PDF with it to be viewed and printed only (OS/2 `fsType` 4).
    pub view_only: bool,
    /// The text does not fit the height of the box: what is below is cut off.
    pub overflow: bool,
}

/// Whether `annotation` is a text box this app made (a `FreeText` with the app's entry).
pub(super) fn is_text_box(annotation: &PdfAnnotation) -> bool {
    annotation
        .object()
        .get_dict(pdf::KEY)
        .is_ok_and(|entry| entry.is_some())
}

fn finite(rect: Rect) -> bool {
    [rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .all(|edge| edge.is_finite())
}

/// Whether `bounds` is finite, upright, and within page coordinate limits.
fn valid_bounds(bounds: &mupdf::Rect) -> bool {
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

/// Most text boxes of a page whose words go into its text layer.
const MAX_BOXES_IN_TEXT: usize = 500;

/// Whether two sides are the same length, as far as a box on a page can tell.
fn same_length(one: f32, other: f32) -> bool {
    (one - other).abs() < 0.01
}

impl PdfDocument {
    /// Checks everything about a text box that can be checked, then writes the objects of it (its
    /// font and appearance) in the document and gives the source of its annotation dictionary
    /// and whether the words overflow the box. Nothing is put in the document before every check
    /// has passed. `inverse` takes the page space of MuPDF to the PDF's.
    fn make_text_box(
        &mut self,
        rect: Rect,
        model: &TextBoxModel,
        style: &TextBoxStyle,
        inverse: mupdf::Matrix,
    ) -> Result<(String, bool), EngineError> {
        model
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the words of a text box"))?;
        style
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the style of a text box"))?;
        if !finite(rect)
            || rect.x1 - rect.x0 < MIN_ANNOTATION_SIDE_PT
            || rect.y1 - rect.y0 < MIN_ANNOTATION_SIDE_PT
        {
            return Err(EngineError::InvalidEdit("the rectangle of a text box"));
        }
        let (width, height) = (rect.x1 - rect.x0, rect.y1 - rect.y0);
        let laid = layout::lay_out(model, style, (width, height))
            .ok_or(EngineError::InvalidEdit("the words of a text box"))?;
        let subset = font::clean_subset(model).ok_or(EngineError::Font(FontRefusal::NotAFont))?;

        let (font_object, file) = pdf::write_font(&mut self.doc, model, &subset)?;

        // The page space of MuPDF has its origin at the top left and goes down; the PDF's has it
        // where the page's box says and goes up, and a page may be turned. The box is drawn in
        // its own space (up from the bottom left): this takes it to the page's, and from there to
        // the PDF's. The form carries the turn only; where it goes is the annotation's `/Rect`.
        let at = binding_rect(rect).transform(&inverse);
        let mut turn = mupdf::Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 0.0);
        turn.concat(inverse);

        let form = pdf::parse(
            &self.doc,
            &format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
                 /Matrix [{:.5} {:.5} {:.5} {:.5} 0 0] \
                 /Resources << /Font << /F0 {} 0 R >> >> >>",
                turn.a,
                turn.b,
                turn.c,
                turn.d,
                pdf::object_number(&font_object)?,
            ),
        )?;
        let stream = self.doc.add_stream(
            &Buffer::from_bytes(laid.content.as_bytes())?,
            Some(&form),
            false,
        )?;
        let entry = pdf::entry_source(model, style, laid.overflow, &file)?;
        let source = format!(
            "<< /Type /Annot /Subtype /FreeText /Rect [{:.3} {:.3} {:.3} {:.3}] \
             /Contents {} /DA ({}) /Q {} /F 4 \
             /IT /FreeTextTypeWriter /BS << /W 0 >> /AP << /N {} 0 R >> /{} {entry} >>",
            at.x0,
            at.y0,
            at.x1,
            at.y1,
            pdf::utf16_hex(&model.text),
            pdf::default_appearance(style),
            pdf::quadding_of(style),
            pdf::object_number(&stream)?,
            pdf::KEY,
        );
        Ok((source, laid.overflow))
    }

    /// The matrix that takes page space (MuPDF's, from the top left) to the PDF's, for page
    /// `index`, and the page's object.
    pub(super) fn page_to_pdf(
        &self,
        index: u32,
    ) -> Result<(PdfObject, mupdf::Matrix), EngineError> {
        let page = self.pdf_page(index)?;
        let object = page.object();
        let inverse = object
            .page_ctm()?
            .invert()
            .ok_or(EngineError::InvalidEdit("the page cannot be placed"))?;
        Ok((object, inverse))
    }

    /// Sets `model` in a text box over `rect` (page space) on page `index`: a `FreeText`
    /// annotation that says the words and whose appearance sets them in the subset of the model.
    /// Nothing is put in the document before every check has passed: a box that cannot be made
    /// leaves no objects behind.
    pub fn place_text_box(
        &mut self,
        index: u32,
        rect: Rect,
        model: &TextBoxModel,
        style: &TextBoxStyle,
    ) -> Result<TextBoxMade, EngineError> {
        let (mut page_object, inverse) = self.page_to_pdf(index)?;
        let (source, overflow) = self.make_text_box(rect, model, style, inverse)?;
        let annotation = pdf::parse(&self.doc, &source)?;
        let annotation = self.doc.add_object(&annotation)?;
        match page_object.get_dict("Annots")? {
            Some(mut annots) => annots.array_push(annotation)?,
            None => {
                let mut created = self.doc.new_array()?;
                created.array_push(annotation)?;
                page_object.dict_put("Annots", created)?;
            }
        }
        Ok(TextBoxMade {
            view_only: model.view_only,
            overflow,
        })
    }

    /// Sets text box `id` of page `index` again, in `rect` (page space), from `model` and `style`:
    /// a new font subset and appearance, and the same annotation (its number does not change).
    /// The dictionary is changed key by key, never through MuPDF's annotation calls, which would
    /// draw the box black again. What it had before is no longer used, and a full save leaves it
    /// out.
    pub fn replace_text_box(
        &mut self,
        index: u32,
        id: AnnotationId,
        rect: Rect,
        model: &TextBoxModel,
        style: &TextBoxStyle,
    ) -> Result<TextBoxMade, EngineError> {
        let (page_object, inverse) = self.page_to_pdf(index)?;
        let page = self.pdf_page(index)?;
        let annotation = find(&page, id)?;
        if annotation.r#type()? != PdfAnnotationType::FreeText || !is_text_box(&annotation) {
            return Err(EngineError::InvalidEdit("not a text box"));
        }
        drop(page_object);
        let (source, overflow) = self.make_text_box(rect, model, style, inverse)?;
        let fresh = pdf::parse(&self.doc, &source)?;
        let mut object = annotation.object();
        for key in ["Rect", "Contents", "DA", "Q", "AP", pdf::KEY] {
            let value = fresh
                .get_dict(key)?
                .ok_or(EngineError::InvalidEdit("not a text box"))?;
            object.dict_put(key, value)?;
        }
        Ok(TextBoxMade {
            view_only: model.view_only,
            overflow,
        })
    }

    /// Where text box `id` of page `index` is (page space).
    fn text_box_rect(&self, index: u32, id: AnnotationId) -> Result<Rect, EngineError> {
        let page = self.pdf_page(index)?;
        let bounds = find(&page, id)?.bounds()?;
        if !valid_bounds(&bounds) {
            return Err(EngineError::InvalidEdit("the rectangle of a text box"));
        }
        Ok(Rect {
            x0: bounds.x0,
            y0: bounds.y0,
            x1: bounds.x1,
            y1: bounds.y1,
        })
    }

    /// Moves and resizes text box `id` of page `index` to `rect` (page space). A move changes the
    /// `/Rect` of the annotation and nothing else; a new size sets the words again (the lines
    /// break differently), from the model the file has, so the font is not needed.
    pub fn move_text_box(
        &mut self,
        index: u32,
        id: AnnotationId,
        rect: Rect,
    ) -> Result<(), EngineError> {
        let (model, style, _) = self.text_box(index, id)?;
        let old = self.text_box_rect(index, id)?;
        if !same_length(old.x1 - old.x0, rect.x1 - rect.x0)
            || !same_length(old.y1 - old.y0, rect.y1 - rect.y0)
        {
            self.replace_text_box(index, id, rect, &model, &style)?;
            return Ok(());
        }
        if !finite(rect) {
            return Err(EngineError::InvalidEdit("the rectangle of a text box"));
        }
        let (_, inverse) = self.page_to_pdf(index)?;
        let at = binding_rect(rect).transform(&inverse);
        let page = self.pdf_page(index)?;
        let mut object = find(&page, id)?.object();
        let value = self.doc.new_object_from_str(&format!(
            "[{:.3} {:.3} {:.3} {:.3}]",
            at.x0, at.y0, at.x1, at.y1
        ))?;
        object.dict_put("Rect", value)?;
        Ok(())
    }

    /// Gives text box `id` of page `index` another `style`, and other words if `model` is given
    /// (otherwise those it has): it is set again in the rectangle it has.
    pub fn set_text_box(
        &mut self,
        index: u32,
        id: AnnotationId,
        model: Option<&TextBoxModel>,
        style: &TextBoxStyle,
    ) -> Result<TextBoxMade, EngineError> {
        let (own, _, _) = self.text_box(index, id)?;
        let rect = self.text_box_rect(index, id)?;
        self.replace_text_box(index, id, rect, model.unwrap_or(&own), style)
    }

    /// [`prepare_text_box`] and [`place_text_box`](Self::place_text_box) in one: a text box of
    /// `spec` in `font`. A font that cannot be used (see `FontRefusal`), or has no glyph for a
    /// character, makes no text box.
    pub fn add_text_box(
        &mut self,
        index: u32,
        rect: Rect,
        spec: &TextBoxSpec,
        font: &TextBoxFont,
    ) -> Result<TextBoxMade, EngineError> {
        let model = prepare_text_box(font, spec.text)?;
        let style = TextBoxStyle {
            size: spec.size,
            color: spec.color,
            align: spec.align,
        };
        self.place_text_box(index, rect, &model, &style)
    }

    /// The words of the text boxes of page `index`, line by line, each character with its quad in
    /// page space (B2-29): MuPDF's text extraction leaves annotations out, so search and selection
    /// would not find them. A box is laid out again, from the glyph widths in its entry, in the
    /// size it has now; the font is not needed. One that cannot be read is left out.
    pub(super) fn text_box_lines(&self, index: u32) -> Vec<Vec<(char, Quad)>> {
        let Ok(page) = self.pdf_page(index) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let mut boxes = 0;
        for annotation in page.annotations() {
            if boxes == MAX_BOXES_IN_TEXT {
                break;
            }
            if annotation.r#type().ok() != Some(PdfAnnotationType::FreeText)
                || !is_text_box(&annotation)
            {
                continue;
            }
            boxes += 1;
            let Ok(bounds) = annotation.bounds() else {
                continue;
            };
            if !valid_bounds(&bounds) {
                continue;
            }
            let Ok(Some(entry)) = annotation.object().get_dict(pdf::KEY) else {
                continue;
            };
            let Ok((model, style)) = pdf::read_for_layout(&entry) else {
                continue;
            };
            let size = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
            let Some(laid) = layout::lay_out(&model, &style, size) else {
                continue;
            };
            let above = (model.ascent as f32 / 1000.0).clamp(0.5, 2.0) * style.size;
            let below = (model.descent as f32 / 1000.0).clamp(-1.0, -0.05) * style.size;
            // The box's own space goes up from its bottom left; the page's goes down.
            let at = |x: f32, y: f32| Point {
                x: bounds.x0 + x,
                y: bounds.y1 - y,
            };
            for (x, baseline, glyphs) in laid.lines {
                let mut left = x;
                let mut line = Vec::with_capacity(glyphs.len());
                for glyph in glyphs {
                    let right = left + glyph.advance * style.size;
                    let quad = Quad {
                        ul: at(left, baseline + above),
                        ur: at(right, baseline + above),
                        ll: at(left, baseline + below),
                        lr: at(right, baseline + below),
                    };
                    if quad.validate().is_err() {
                        continue;
                    }
                    line.push((glyph.ch, quad));
                    left = right;
                }
                if !line.is_empty() {
                    lines.push(line);
                }
            }
        }
        lines
    }

    /// The model and the style of text box `id` of page `index`, read back from the file; and
    /// whether its words overflow its box. For what is not a text box this app made, an error.
    pub fn text_box(
        &self,
        index: u32,
        id: AnnotationId,
    ) -> Result<(TextBoxModel, TextBoxStyle, bool), EngineError> {
        let page = self.pdf_page(index)?;
        let annotation = find(&page, id)?;
        if annotation.r#type()? != PdfAnnotationType::FreeText {
            return Err(EngineError::InvalidEdit("not a text box"));
        }
        let entry = annotation
            .object()
            .get_dict(pdf::KEY)?
            .ok_or(EngineError::InvalidEdit("not a text box"))?;
        pdf::read_entry(&entry)
    }
}
