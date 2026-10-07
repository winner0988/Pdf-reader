//! Taking out of pages that came from another file what points outside the document, or runs by
//! itself (B2-06, docs/architecture/merge.md).
//!
//! MuPDF takes a page over without its annotations, links and form fields, and nothing of what
//! belongs to the source as a whole, so a script or a launch action does not come along. What
//! does come is what the page uses: its resources, and with them whatever a stream says about
//! where its data is. This looks through everything the grafted pages use, which is all in objects
//! the graft made (numbered above `first_new`: the document's own are never touched), and takes
//! out:
//! - the data of a stream that is in another file or at a URL (`/F`, with how it is filtered):
//!   the app never fetches it, but other readers may;
//! - a form that shows a page of another file (`/Ref`);
//! - triggers of actions (`/AA`), wherever they are.

use std::collections::HashSet;

use mupdf::pdf::{PdfDocument, PdfObject};

use crate::engine::EngineError;

/// Most objects looked at for the pages of one file.
const MAX_OBJECTS: usize = 2_000_000;

/// What a stream says about data that is not in it.
const EXTERNAL_DATA: [&str; 3] = ["F", "FFilter", "FDecodeParms"];

/// What is taken out of every dictionary it is in.
const EVERYWHERE: [&str; 2] = ["AA", "Ref"];

/// Scrubs the `count` pages from page `first` on, which `PdfDocument::insert_pdf` just made from
/// another file; objects numbered below `first_new` were in the document before, and are left
/// alone.
pub fn scrub_pages(
    doc: &PdfDocument,
    first: u32,
    count: u32,
    first_new: u32,
) -> Result<(), EngineError> {
    let mut seen = HashSet::new();
    let mut stack = Vec::new();
    for index in first..first.saturating_add(count) {
        let mut page = doc.find_page(i32::try_from(index).unwrap_or(i32::MAX))?;
        // A page of the source does by itself nothing but show; what else it had is taken out
        // (the graft makes none of it, this is for a graft that does).
        for key in ["AA", "Annots", "B"] {
            page.dict_delete(key)?;
        }
        stack.push(page);
    }
    let mut visited = 0usize;
    while let Some(mut object) = stack.pop() {
        visited += 1;
        if visited > MAX_OBJECTS {
            return Err(EngineError::TooComplex);
        }
        if object.is_indirect()? {
            let number = object.as_indirect()?;
            if u32::try_from(number).is_ok_and(|number| number < first_new) {
                continue;
            }
            if !seen.insert(number) {
                continue;
            }
        }
        if object.is_array()? {
            for item in object.array_iter()? {
                let item = item?;
                if may_contain_more(&item)? {
                    stack.push(item);
                }
            }
            continue;
        }
        if !object.is_dict()? {
            continue;
        }
        if object.is_stream()? {
            for key in EXTERNAL_DATA {
                object.dict_delete(key)?;
            }
        }
        for key in EVERYWHERE {
            object.dict_delete(key)?;
        }
        let mut children = Vec::new();
        for entry in object.dict_iter()? {
            let (key, value) = entry?;
            // The page tree above a page is the document's own.
            if key.as_name()? != b"Parent" && may_contain_more(&value)? {
                children.push(value);
            }
        }
        stack.extend(children);
    }
    Ok(())
}

/// Whether `object` may lead to more dictionaries. A reference is not loaded until it is visited.
fn may_contain_more(object: &PdfObject) -> Result<bool, mupdf::Error> {
    Ok(object.is_indirect()? || object.is_dict()? || object.is_array()?)
}
