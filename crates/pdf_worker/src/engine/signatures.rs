//! Reading the signature fields of a document as the file has them (B2-14, ADR 0014,
//! docs/architecture/signatures.md). Nothing is judged here: the values are only read, each on
//! its own, so that one that cannot be read does not hide the others. `crate::signatures` judges
//! them.

use std::collections::HashSet;

use ipc_contract::limits::MAX_SIGNATURES;
use mupdf::pdf::PdfObject;

use super::{EngineError, MAX_FORM_FIELDS, PdfDocument};

/// How deep the tree of fields is followed: a cycle or an absurd depth is cut.
const MAX_FIELD_DEPTH: usize = 64;

/// The signature fields of a document that have a signature in them, in the order of the form.
#[derive(Default)]
pub struct SignatureFields {
    pub fields: Vec<SignatureField>,
    /// There are more than `MAX_SIGNATURES`, or the form has more fields than are looked at.
    pub truncated: bool,
}

/// A signature field with a signature in it, as the file has it. Every part is optional: a
/// signature that cannot be read is still one.
pub struct SignatureField {
    /// The signature dictionary (`/V`).
    value: PdfObject,
    /// The field's own name (`/T`).
    pub name: Option<String>,
    /// How the signature is made (`/SubFilter`), a name.
    pub sub_filter: Option<Vec<u8>>,
    /// The four numbers of `/ByteRange`, as the file says.
    pub byte_range: Option<[i32; 4]>,
    /// When the signer says they signed (`/M`), a date as PDF writes it.
    pub claimed_time: Option<String>,
    /// DocMDP's `/P`, if this is the signature that certifies the document (2 if it says none).
    pub certification: Option<i32>,
}

impl SignatureField {
    /// The signature's own bytes (`/Contents`), copied. Read last, once the range has shown that
    /// the file has room for them: a signature dictionary can point at a string as large as the
    /// file.
    pub fn contents(&self) -> Option<Vec<u8>> {
        let contents = self.value.get_dict("Contents").ok()??;
        if !contents.is_string().ok()? {
            return None;
        }
        contents.as_bytes().ok()
    }
}

impl PdfDocument {
    /// The signature fields that have a signature, at most `MAX_SIGNATURES`. A field that cannot
    /// be read is skipped.
    pub fn signature_fields(&self) -> Result<SignatureFields, EngineError> {
        let mut found = SignatureFields::default();
        let Some(form) = self.doc.catalog()?.get_dict("AcroForm")? else {
            return Ok(found);
        };
        let Some(top) = form.get_dict("Fields")? else {
            return Ok(found);
        };
        let certifying = self.certifying_signature();
        let mut fields_seen = HashSet::new();
        let mut values_seen = HashSet::new();
        let mut visited = 0;
        // Depth first, in the order of the form: kids go on the stack last to first.
        let mut stack = Vec::new();
        push_kids(&mut stack, &top, false, 0, visited);
        while let Some((field, inherited, depth)) = stack.pop() {
            visited += 1;
            if visited > MAX_FORM_FIELDS {
                found.truncated = true;
                break;
            }
            if let Ok(true) = field.is_indirect()
                && !field
                    .as_indirect()
                    .is_ok_and(|number| fields_seen.insert(number))
            {
                continue;
            }
            if !field.is_dict().unwrap_or(false) {
                continue;
            }
            // The type of a field is its own, or the one of the fields above it.
            let is_signature = match field.get_dict("FT").ok().flatten() {
                Some(kind) => kind.as_name().is_ok_and(|name| name == b"Sig"),
                None => inherited,
            };
            if is_signature
                && let Ok(Some(value)) = field.get_dict("V")
                && value.is_dict().unwrap_or(false)
            {
                // One signature that several fields point at is one.
                let new = match value.is_indirect() {
                    Ok(true) => value
                        .as_indirect()
                        .is_ok_and(|number| values_seen.insert(number)),
                    _ => true,
                };
                if new {
                    if found.fields.len() == MAX_SIGNATURES as usize {
                        found.truncated = true;
                        break;
                    }
                    found.fields.push(read_field(&field, value, certifying));
                }
            }
            if depth < MAX_FIELD_DEPTH
                && let Ok(Some(kids)) = field.get_dict("Kids")
            {
                push_kids(&mut stack, &kids, is_signature, depth + 1, visited);
            }
        }
        Ok(found)
    }

    /// The object number of the signature that certifies the document, if the form names one
    /// (`/Perms` has a `/DocMDP`): no other signature does, whatever it says.
    fn certifying_signature(&self) -> Option<i32> {
        let perms = self.doc.catalog().ok()?.get_dict("Perms").ok()??;
        let value = perms.get_dict("DocMDP").ok()??;
        if !value.is_indirect().ok()? {
            return None;
        }
        value.as_indirect().ok()
    }
}

/// Puts the entries of `list` on the stack, last first, as many as there is room for.
fn push_kids(
    stack: &mut Vec<(PdfObject, bool, usize)>,
    list: &PdfObject,
    inherited: bool,
    depth: usize,
    visited: usize,
) {
    if !list.is_array().unwrap_or(false) {
        return;
    }
    let room = MAX_FORM_FIELDS.saturating_sub(visited);
    let count = list.len().unwrap_or(0).min(room);
    for index in (0..count).rev() {
        if let Ok(Some(kid)) = list.get_array(index as i32) {
            stack.push((kid, inherited, depth));
        }
    }
}

fn read_field(field: &PdfObject, value: PdfObject, certifying: Option<i32>) -> SignatureField {
    SignatureField {
        name: text_of(field, "T"),
        sub_filter: name_of(&value, "SubFilter"),
        byte_range: byte_range_of(&value),
        claimed_time: text_of(&value, "M"),
        certification: certification_of(&value, certifying),
        value,
    }
}

fn text_of(object: &PdfObject, key: &str) -> Option<String> {
    let text = object.get_dict(key).ok()??;
    if !text.is_string().ok()? {
        return None;
    }
    text.as_string().ok()
}

fn name_of(object: &PdfObject, key: &str) -> Option<Vec<u8>> {
    let name = object.get_dict(key).ok()??;
    if !name.is_name().ok()? {
        return None;
    }
    name.as_name().ok()
}

fn byte_range_of(value: &PdfObject) -> Option<[i32; 4]> {
    let array = value.get_dict("ByteRange").ok()??;
    if !array.is_array().ok()? || array.len().ok()? != 4 {
        return None;
    }
    let mut range = [0; 4];
    for (index, slot) in range.iter_mut().enumerate() {
        let number = array.get_array(index as i32).ok()??;
        if !number.is_number().ok()? {
            return None;
        }
        *slot = number.as_int().ok()?;
    }
    Some(range)
}

/// DocMDP's `/P` of the signature `value`, if it is the one that certifies the document.
fn certification_of(value: &PdfObject, certifying: Option<i32>) -> Option<i32> {
    if !value.is_indirect().ok()? || value.as_indirect().ok()? != certifying? {
        return None;
    }
    let references = value.get_dict("Reference").ok()??;
    if !references.is_array().ok()? {
        return None;
    }
    // A few entries are plenty: one says DocMDP.
    for index in 0..references.len().ok()?.min(8) {
        let reference = references.get_array(index as i32).ok()??;
        if name_of(&reference, "TransformMethod").as_deref() != Some(b"DocMDP") {
            continue;
        }
        let permission = reference
            .get_dict("TransformParams")
            .ok()
            .flatten()
            .and_then(|params| params.get_dict("P").ok().flatten())
            .and_then(|number| number.as_int().ok());
        return Some(permission.unwrap_or(2));
    }
    None
}
