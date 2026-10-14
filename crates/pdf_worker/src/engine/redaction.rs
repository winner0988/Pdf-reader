//! Redaction (B2-19, ADR 0016): areas of pages are marked with `Redact` annotations, and applying
//! them with MuPDF's `pdf_redact_page` removes what the areas reach from the document, not only
//! covers it: text (seen or not, also in form objects), the pixels of pictures, and the vector
//! graphics an area touches. The other annotations and form fields that reach into an area go
//! too, with the values of the fields. Saving then rewrites the file without what nothing uses
//! (ADR 0013), so the removed content does not stay in it.

use std::collections::HashSet;

use ipc_contract::types::{Rect, RedactionSummary};
use mupdf::color::AnnotationColor;
use mupdf::pdf::{
    PdfAnnotationType, PdfObject, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod,
    PdfRedactOptions, PdfRedactTextMethod,
};

use super::annotations::binding_rect;
use super::{EngineError, PdfDocument};

/// How the areas are applied (ADR 0016, the POC's options): a black box where something was
/// removed, the pixels of pictures in the area blanked, vector graphics removed when the area
/// touches them, and text removed whether it is seen or not. None of the options that only cover
/// what is there (POC 4).
const APPLY: PdfRedactOptions = PdfRedactOptions {
    black_boxes: true,
    image_method: PdfRedactImageMethod::Pixels,
    line_art: PdfRedactLineArtMethod::RemoveIfTouched,
    text: PdfRedactTextMethod::Remove,
};

/// A mark until it is applied: MuPDF draws it as a red outline over a pale area (a `Redact`
/// annotation has no interior colour of its own).
const MARK_RED: AnnotationColor = AnnotationColor::Rgb {
    red: 0.85,
    green: 0.1,
    blue: 0.1,
};
const MARK_OPACITY: f32 = 0.35;

/// How far up the field tree a widget's value may be inherited from; deeper trees are not walked.
const MAX_FIELD_DEPTH: usize = 32;

impl PdfDocument {
    /// Marks `rect` (page space) of page `index` to be redacted: a `Redact` annotation. Nothing is
    /// removed yet. Refused for a signed document, which cannot be redacted (ADR 0016).
    pub fn add_redaction(&mut self, index: u32, rect: Rect) -> Result<(), EngineError> {
        self.refuse_signed()?;
        let mut page = self.pdf_page(index)?;
        let mut mark = page.add_redact_annotation(binding_rect(rect))?;
        mark.set_color(MARK_RED)?;
        mark.set_opacity(MARK_OPACITY)?;
        mark.update()?;
        Ok(())
    }

    /// What applying the marks would do besides removing what their areas reach: how many marks
    /// on how many pages, and how many other annotations and form fields reach into an area.
    pub fn redaction_summary(&self) -> Result<RedactionSummary, EngineError> {
        let mut summary = RedactionSummary {
            marks: 0,
            pages: 0,
            annotations: 0,
            fields: 0,
        };
        for index in 0..self.page_count()? {
            let page = self.pdf_page(index)?;
            let marks = marks_of(&page)?;
            if marks.is_empty() {
                continue;
            }
            summary.marks += u32::try_from(marks.len()).unwrap_or(u32::MAX);
            summary.pages += 1;
            let reaching = reaching(&page, &marks)?;
            summary.annotations += u32::try_from(reaching.annotations.len()).unwrap_or(u32::MAX);
            summary.fields += u32::try_from(reaching.widgets.len()).unwrap_or(u32::MAX);
        }
        Ok(summary)
    }

    /// Applies every mark of the document (ADR 0016): on each page that has some, the other
    /// annotations and the form fields reaching into an area are deleted (fields with their
    /// values), then MuPDF removes what the areas reach and draws the black boxes; the marks go.
    /// What was read from those pages (OCR, ADR 0015) is forgotten: their pictures changed.
    /// Refused for a signed document, and when nothing is marked.
    pub fn apply_redactions(&mut self) -> Result<(), EngineError> {
        self.refuse_signed()?;
        let mut redacted = Vec::new();
        for index in 0..self.page_count()? {
            let mut page = self.pdf_page(index)?;
            let marks = marks_of(&page)?;
            if marks.is_empty() {
                continue;
            }
            let reaching = reaching(&page, &marks)?;
            self.remove_reaching(&mut page, reaching)?;
            page.redact_with_options(APPLY)?;
            redacted.push(index);
        }
        if redacted.is_empty() {
            return Err(EngineError::InvalidEdit("nothing is marked to be redacted"));
        }
        self.ocr_turned(&redacted);
        Ok(())
    }

    fn refuse_signed(&self) -> Result<(), EngineError> {
        if self.is_signed() {
            return Err(EngineError::InvalidEdit(
                "a signed document cannot be redacted: appending keeps the old content, rewriting breaks the signatures",
            ));
        }
        Ok(())
    }

    /// Deletes the annotations and the form fields of `page` in `reaching`, with their pop-up
    /// windows, and takes the values out of the fields; then what still points to any of them
    /// points nowhere, so that a rewrite leaves them out of the file ([`crate::unlink`]).
    fn remove_reaching(&self, page: &mut PdfPage, reaching: Reaching) -> Result<(), EngineError> {
        if reaching.annotations.is_empty() && reaching.widgets.is_empty() {
            return Ok(());
        }
        let mut gone = HashSet::new();
        for widget in &reaching.widgets {
            clear_field_value(&widget_object(page, *widget)?)?;
        }
        let numbers: HashSet<i32> = reaching
            .annotations
            .iter()
            .chain(&reaching.widgets)
            .copied()
            .collect();
        // Found again one at a time: deleting while going through the page's list is not safe.
        for number in numbers {
            let annotation = page
                .annotations()
                .find(|annotation| annotation.xref().is_ok_and(|xref| xref == number))
                .or_else(|| {
                    page.widgets()
                        .find(|widget| widget.xref().is_ok_and(|xref| xref == number))
                        .map(mupdf::pdf::PdfWidget::into_annotation)
                });
            let Some(annotation) = annotation else {
                continue;
            };
            gone.insert(number);
            if let Some(popup) = annotation.object().get_dict("Popup")?
                && let Ok(popup) = popup.as_indirect()
                && popup > 0
            {
                gone.insert(popup);
            }
            page.delete_annotation(annotation)?;
        }
        crate::unlink::unlink(&self.doc, &gone)
    }
}

/// The areas of the marks of `page`.
fn marks_of(page: &PdfPage) -> Result<Vec<mupdf::Rect>, EngineError> {
    let mut marks = Vec::new();
    for annotation in page.annotations() {
        if annotation.r#type()? == PdfAnnotationType::Redact {
            marks.push(annotation.rect()?);
        }
    }
    Ok(marks)
}

/// What of a page reaches into one of its marks: annotation and widget numbers.
struct Reaching {
    annotations: Vec<i32>,
    widgets: Vec<i32>,
}

/// The annotations (other than the marks and pop-up windows) and the form fields of `page` whose
/// bounds reach into one of `marks`.
fn reaching(page: &PdfPage, marks: &[mupdf::Rect]) -> Result<Reaching, EngineError> {
    let touches = |bounds: mupdf::Rect| marks.iter().any(|mark| overlaps(&bounds, mark));
    let mut annotations = Vec::new();
    for annotation in page.annotations() {
        let kind = annotation.r#type()?;
        if matches!(kind, PdfAnnotationType::Redact | PdfAnnotationType::Popup) {
            continue;
        }
        if touches(annotation.bounds()?) {
            annotations.push(annotation.xref()?);
        }
    }
    let mut widgets = Vec::new();
    for widget in page.widgets() {
        if touches(widget.annotation().bounds()?) {
            widgets.push(widget.xref()?);
        }
    }
    Ok(Reaching {
        annotations,
        widgets,
    })
}

/// Whether two rectangles share some area (touching edges do not count).
fn overlaps(a: &mupdf::Rect, b: &mupdf::Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// The dictionary of widget `number` of `page`.
fn widget_object(page: &PdfPage, number: i32) -> Result<PdfObject, EngineError> {
    page.widgets()
        .find(|widget| widget.xref().is_ok_and(|xref| xref == number))
        .map(|widget| widget.annotation().object())
        .ok_or(EngineError::InvalidEdit("no such form field on the page"))
}

/// Takes the value of the field a widget belongs to out of the document: `/V` and `/DV` of the
/// widget and of the fields above it (a value can be inherited), and the appearance of every
/// widget of that field (one elsewhere on the page or in the document would still draw the
/// value). The widget itself is deleted by the caller.
fn clear_field_value(widget: &PdfObject) -> Result<(), EngineError> {
    let mut node = Some(widget.clone());
    let mut field = None;
    for _ in 0..MAX_FIELD_DEPTH {
        let Some(mut current) = node.take() else {
            break;
        };
        current.dict_delete("V")?;
        current.dict_delete("DV")?;
        // The field is the nearest one with a name: its widgets are it, or its kids.
        if field.is_none() && current.get_dict("T")?.is_some() {
            field = Some(current.clone());
        }
        node = current.get_dict("Parent")?;
    }
    let field = field.unwrap_or_else(|| widget.clone());
    let mut widgets = vec![field.clone()];
    if let Some(kids) = field.get_dict("Kids")? {
        for index in 0..kids.len()?.min(MAX_KIDS) {
            if let Some(kid) = kids.get_array(i32::try_from(index).unwrap_or(i32::MAX))? {
                widgets.push(kid);
            }
        }
    }
    for mut each in widgets {
        each.dict_delete("AP")?;
        each.dict_delete("AS")?;
    }
    Ok(())
}

/// The most widgets of one field whose appearance is taken away.
const MAX_KIDS: usize = 10_000;
