//! What the privacy export removes from its copy of a document (B2-03,
//! docs/architecture/privacy-export.md). Only metadata: the content (text, images, fonts, links,
//! form fields) is left as it is, so pages look the same.

use mupdf::pdf::{PdfDocument, PdfObject};

use crate::engine::EngineError;

/// Entries removed from every dictionary: XMP metadata streams, applications' private data and
/// modification dates.
const PRIVATE_ENTRIES: [&str; 3] = ["Metadata", "PieceInfo", "LastModified"];

/// Entries of an annotation that name or date whoever made it. A form field's widget is left
/// alone: its /T is the field's name.
const AUTHOR_ENTRIES: [&str; 3] = ["T", "M", "CreationDate"];

/// Most dictionaries and arrays looked at; a document needing more is refused rather than half
/// cleaned.
const MAX_NODES: usize = 20_000_000;

/// Removes the metadata of `doc` in place and gives it the identifier `id`: no document
/// information dictionary, none of [`PRIVATE_ENTRIES`] on any object, no page thumbnail, no
/// annotation author or dates. What was referenced only from there is no longer used, so a save
/// with garbage collection leaves it out.
pub fn strip(doc: &PdfDocument, id: &[u8; 16]) -> Result<(), EngineError> {
    let mut trailer = doc.trailer()?;
    trailer.dict_delete("Info")?;
    // Two byte strings, as the standard says; this one is random (from the main process) and
    // says nothing about when or where the file was made. MuPDF replaces the second half again
    // when it writes the file, as on every save.
    let hex: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut identifier = doc.new_array()?;
    identifier.array_push(doc.new_string(&hex)?)?;
    identifier.array_push(doc.new_string(&hex)?)?;
    trailer.dict_put("ID", identifier)?;

    // Every object of the file, and the dictionaries and arrays written directly inside each
    // (an annotation or a property list can be one); references lead to objects visited on
    // their own turn.
    let mut nodes = 0usize;
    for number in 1..doc.xref_len()? {
        let Some(object) = doc.xref_object(i32::try_from(number).unwrap_or(i32::MAX))? else {
            continue;
        };
        let mut pending = vec![object];
        while let Some(mut node) = pending.pop() {
            nodes += 1;
            if nodes > MAX_NODES {
                return Err(EngineError::TooComplex);
            }
            if node.is_dict()? {
                clean_dictionary(&mut node)?;
                for index in 0..node.dict_len()? {
                    if let Some(value) = node.get_dict_val(index_i32(index))? {
                        push_direct(&mut pending, value)?;
                    }
                }
            } else if node.is_array()? {
                for index in 0..node.len()? {
                    if let Some(value) = node.get_array(index_i32(index))? {
                        push_direct(&mut pending, value)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn index_i32(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

/// Queues a dictionary or array written directly inside another; references are skipped.
fn push_direct(pending: &mut Vec<PdfObject>, value: PdfObject) -> Result<(), EngineError> {
    if !value.is_indirect()? && (value.is_dict()? || value.is_array()?) {
        pending.push(value);
    }
    Ok(())
}

fn clean_dictionary(dict: &mut PdfObject) -> Result<(), EngineError> {
    for key in PRIVATE_ENTRIES {
        dict.dict_delete(key)?;
    }
    if name_of(dict, "Type")?.as_deref() == Some(b"Page".as_slice()) {
        dict.dict_delete("Thumb")?;
    }
    if is_annotation(dict)? {
        for key in AUTHOR_ENTRIES {
            dict.dict_delete(key)?;
        }
    }
    Ok(())
}

/// An annotation other than a form field's widget: a /Subtype and a /Rect, and a /Type of
/// /Annot if it says. Fonts, images and forms have a /Subtype too, but no /Rect.
fn is_annotation(dict: &PdfObject) -> Result<bool, EngineError> {
    let Some(subtype) = name_of(dict, "Subtype")? else {
        return Ok(false);
    };
    if subtype == b"Widget" || dict.get_dict("Rect")?.is_none() {
        return Ok(false);
    }
    Ok(matches!(
        name_of(dict, "Type")?.as_deref(),
        None | Some(b"Annot")
    ))
}

/// The name under `key`, if there is one.
fn name_of(dict: &PdfObject, key: &str) -> Result<Option<Vec<u8>>, EngineError> {
    match dict.get_dict(key)? {
        Some(value) if value.is_name()? => Ok(Some(value.as_name()?)),
        _ => Ok(None),
    }
}
