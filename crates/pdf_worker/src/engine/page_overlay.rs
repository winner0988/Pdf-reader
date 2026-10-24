//! Shared logic for drawing text and Form XObjects onto page content (B3-04, B3-16).
//!
//! Watermarks and page marks are drawn as page content (not annotations) so that every reader
//! shows them and text search finds them. The page's own content is enclosed in `[q, ..., Q]` so
//! that its graphics state does not leak into the marks. Inherited page resources are copied so
//! that modifying the page's resources dictionary does not affect other pages.

use ipc_contract::types::Rgb;
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};
use mupdf::{Buffer, Matrix};

use super::EngineError;
use super::text_box::{LineFont, TextBoxFont, line_font, line_form};

/// The built-in font of the words: it has Chinese and Latin letters.
const FONT_NAME: &str = "Droid Sans Fallback";

fn droid_sans() -> Result<TextBoxFont<'static>, EngineError> {
    let font = mupdf_fonts_droid::find_by_name(FONT_NAME, false, false)
        .ok_or(EngineError::InvalidEdit("no font for the words"))?;
    Ok(TextBoxFont {
        bytes: font.data,
        face: 0,
        name: FONT_NAME,
    })
}

/// Creates a Form XObject stream containing `text` on one line in the embedded font,
/// returning the stream object and its width and height in points.
pub fn droid_sans_line_form(
    doc: &mut MuPdfDocument,
    text: &str,
    size: f32,
    color: Rgb,
) -> Result<(PdfObject, f32, f32), EngineError> {
    line_form(doc, &droid_sans()?, text, size, color)
}

/// Writes into `doc` one subset of the embedded font with the glyphs of `characters`, for many
/// lines set with `text_box::line_form_in`.
pub fn droid_sans_line_font(
    doc: &mut MuPdfDocument,
    characters: &str,
) -> Result<LineFont, EngineError> {
    line_font(doc, &droid_sans()?, characters)
}

/// Attaches multiple Form XObjects with their transformation matrices to `page`.
/// Ensures inherited resources are copied so that `page` has its own `Resources` and `XObject`
/// dictionaries, and wraps the page's `/Contents` in `[q, ...original..., Q, ...marks...]`.
pub fn attach_forms_to_page(
    doc: &mut MuPdfDocument,
    page: &mut PdfObject,
    forms: &[(&str, &PdfObject, &Matrix)],
) -> Result<(), EngineError> {
    if forms.is_empty() {
        return Ok(());
    }

    // The page's resources, its own: a page that takes them from its parent gets a copy.
    let mut resources = match page.get_dict("Resources")? {
        Some(resources) => resources,
        None => {
            let inherited = match page.get_dict_inheritable("Resources")? {
                Some(inherited) => inherited.copy_dict()?,
                None => doc.new_dict()?,
            };
            page.dict_put("Resources", inherited)?;
            page.get_dict("Resources")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };
    let mut xobjects = match resources.get_dict("XObject")? {
        Some(xobjects) => xobjects,
        None => {
            resources.dict_put("XObject", doc.new_dict()?)?;
            resources
                .get_dict("XObject")?
                .ok_or(EngineError::InvalidEdit("the page has no resources"))?
        }
    };

    for (name, mark, _) in forms {
        xobjects.dict_put(*name, mark.try_clone()?)?;
    }

    let before = doc.add_stream(&Buffer::from_bytes(b"q\n")?, None, false)?;

    let mut draw = String::from("\nQ\n");
    for (name, _, matrix) in forms {
        draw.push_str(&format!(
            "q {:.5} {:.5} {:.5} {:.5} {:.3} {:.3} cm /{name} Do Q\n",
            matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f,
        ));
    }
    let after = doc.add_stream(&Buffer::from_bytes(draw.as_bytes())?, None, false)?;

    let mut contents = doc.new_array()?;
    contents.array_push(before)?;
    match page.get_dict("Contents")? {
        Some(existing) if existing.is_array()? => {
            for part in existing.array_iter()? {
                contents.array_push(part?)?;
            }
        }
        Some(existing) => contents.array_push(existing)?,
        None => {}
    }
    contents.array_push(after)?;
    page.dict_put("Contents", contents)?;

    Ok(())
}

/// Convenience wrapper for attaching a single Form XObject to `page`.
pub fn attach_form_to_page(
    doc: &mut MuPdfDocument,
    page: &mut PdfObject,
    name: &str,
    mark: &PdfObject,
    matrix: &Matrix,
) -> Result<(), EngineError> {
    attach_forms_to_page(doc, page, &[(name, mark, matrix)])
}
