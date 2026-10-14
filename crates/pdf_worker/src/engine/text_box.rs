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

use ipc_contract::limits::MIN_ANNOTATION_SIDE_PT;
use ipc_contract::types::{AnnotationId, FontRefusal, Rect, Rgb, TextAlign, TextBoxStyle};
use ipc_contract::validate::Validate;
use ipc_contract::worker::TextBoxModel;
use mupdf::Buffer;
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType};

pub use font::prepare_text_box;

use super::annotations::{binding_rect, find};
use super::{EngineError, PdfDocument};

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

impl PdfDocument {
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
        let page = self.pdf_page(index)?;
        let mut page_object = page.object();
        let inverse = page_object
            .page_ctm()?
            .invert()
            .ok_or(EngineError::InvalidEdit("the page cannot be placed"))?;

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
        let annotation = pdf::parse(
            &self.doc,
            &format!(
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
            ),
        )?;
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
            overflow: laid.overflow,
        })
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
