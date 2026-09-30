//! Cuts what still points to pages removed from a document (B2-05, ADR 0013).
//!
//! MuPDF removes a page from the page tree only. Outline entries, links, named destinations, an
//! open action or the structure tree may still point to it, and anything pointed to is kept when
//! the file is rewritten: the removed page, its content included, would still be in the file.
//! So every such reference becomes `null`. The entry or link itself stays, but goes nowhere, as
//! one to a missing page does in any reader; a rewrite then leaves the page out.

use std::collections::HashSet;

use mupdf::pdf::{PdfDocument, PdfObject};

use crate::engine::EngineError;

/// Most dictionaries and arrays looked at, as for the privacy export; a document needing more is
/// refused rather than left half done.
const MAX_NODES: usize = 20_000_000;

/// Replaces every reference to one of the objects numbered `removed` with `null`, in every
/// object of `doc` and the dictionaries and arrays written directly inside each.
pub fn unlink(doc: &PdfDocument, removed: &HashSet<i32>) -> Result<(), EngineError> {
    if removed.is_empty() {
        return Ok(());
    }
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
                // Keys first: changing an entry while going through them by index is not safe.
                let mut cut = Vec::new();
                for index in 0..node.dict_len()? {
                    let (Some(key), Some(value)) = (
                        node.get_dict_key(index_i32(index))?,
                        node.get_dict_val(index_i32(index))?,
                    ) else {
                        continue;
                    };
                    if points_to(&value, removed)? {
                        cut.push(key);
                    } else {
                        push_direct(&mut pending, value)?;
                    }
                }
                for key in cut {
                    node.dict_put(key, PdfObject::new_null())?;
                }
            } else if node.is_array()? {
                for index in 0..node.len()? {
                    let Some(value) = node.get_array(index_i32(index))? else {
                        continue;
                    };
                    if points_to(&value, removed)? {
                        node.array_put(index_i32(index), PdfObject::new_null())?;
                    } else {
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

/// Whether `value` is a reference to one of the `removed` objects.
fn points_to(value: &PdfObject, removed: &HashSet<i32>) -> Result<bool, EngineError> {
    Ok(value.is_indirect()? && removed.contains(&value.as_indirect()?))
}

/// Queues a dictionary or array written directly inside another; references are skipped.
fn push_direct(pending: &mut Vec<PdfObject>, value: PdfObject) -> Result<(), EngineError> {
    if !value.is_indirect()? && (value.is_dict()? || value.is_array()?) {
        pending.push(value);
    }
    Ok(())
}
