//! What deleting pages has to take out besides the pages (#138, B2-05).
//!
//! MuPDF only takes a page out of the page tree. The form still lists the fields whose widgets
//! were on it, values and appearances included; the structure tree (tagged PDF) still has the
//! elements for its content, alternate texts included. A rewrite keeps whatever is still listed,
//! so they would stay in the file. They are found here before the pages go (afterwards nothing
//! says what was on them) and taken out; [`crate::unlink`] then cuts what else points to them.

use std::collections::HashSet;

use mupdf::pdf::{PdfDocument, PdfObject};

use crate::engine::EngineError;

/// Most fields, or structure elements and their kids, looked at.
const MAX_NODES: usize = 1_000_000;

/// Takes out of `doc` the form fields and structure elements that only the pages numbered
/// `removed` had. Returns the objects taken out, for [`crate::unlink`].
///
/// A structure tree too large to go through is dropped whole: nothing of the removed pages may
/// stay. A form too large is refused (`TooComplex`) with the document unchanged.
pub fn take_out(doc: &PdfDocument, removed: &HashSet<i32>) -> Result<HashSet<i32>, EngineError> {
    let mut gone = HashSet::new();
    let widgets = widgets_on(doc, removed)?;
    if !widgets.is_empty()
        && let Some(form) = doc.catalog()?.get_dict("AcroForm")?
        && let Some(mut fields) = form.get_dict("Fields")?
    {
        // Counted first, so a form too large is refused before anything changes.
        let mut nodes = 0;
        count_fields(&fields, &mut nodes, &mut HashSet::new())?;
        prune_fields(&mut fields, &widgets, &mut gone, &mut HashSet::new())?;
    }
    gone.extend(&widgets);
    if let Some(root) = doc.catalog()?.get_dict("StructTreeRoot")? {
        let mut walk = StructureWalk {
            removed,
            gone: HashSet::new(),
            nodes: 0,
            seen: HashSet::new(),
        };
        match walk.prune(root) {
            Ok(_) => gone.extend(walk.gone),
            Err(EngineError::TooComplex) => doc.catalog()?.dict_delete("StructTreeRoot")?,
            Err(error) => return Err(error),
        }
    }
    Ok(gone)
}

fn index_i32(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

/// The name under `key`, if there is one.
fn name_of(dict: &PdfObject, key: &str) -> Result<Option<Vec<u8>>, EngineError> {
    match dict.get_dict(key)? {
        Some(value) if value.is_name()? => Ok(Some(value.as_name()?)),
        _ => Ok(None),
    }
}

/// The object number `value` refers to, or 0 for a direct object.
fn number_of(value: &PdfObject) -> Result<i32, EngineError> {
    Ok(if value.is_indirect()? {
        value.as_indirect()?
    } else {
        0
    })
}

/// The widget annotations on the `removed` pages, by object number.
fn widgets_on(doc: &PdfDocument, removed: &HashSet<i32>) -> Result<HashSet<i32>, EngineError> {
    let mut widgets = HashSet::new();
    for &page in removed {
        let Some(annots) = doc.new_indirect(page, 0)?.get_dict("Annots")? else {
            continue;
        };
        if !annots.is_array()? {
            continue;
        }
        if annots.len()? > MAX_NODES {
            return Err(EngineError::TooComplex);
        }
        for index in 0..annots.len()? {
            let Some(annot) = annots.get_array(index_i32(index))? else {
                continue;
            };
            if annot.is_indirect()? && name_of(&annot, "Subtype")?.as_deref() == Some(b"Widget") {
                widgets.insert(annot.as_indirect()?);
            }
        }
    }
    Ok(widgets)
}

/// Counts the nodes of a field tree, up to `MAX_NODES`.
fn count_fields(
    kids: &PdfObject,
    nodes: &mut usize,
    seen: &mut HashSet<i32>,
) -> Result<(), EngineError> {
    if !kids.is_array()? {
        return Ok(());
    }
    for index in 0..kids.len()? {
        *nodes += 1;
        if *nodes > MAX_NODES {
            return Err(EngineError::TooComplex);
        }
        let Some(kid) = kids.get_array(index_i32(index))? else {
            continue;
        };
        let number = number_of(&kid)?;
        if number != 0 && !seen.insert(number) {
            continue;
        }
        if let Some(grandkids) = kid.get_dict("Kids")? {
            count_fields(&grandkids, nodes, seen)?;
        }
    }
    Ok(())
}

/// Removes from the field array `kids` the widgets in `widgets`, and the fields left without
/// kids once theirs are removed; adds the fields removed to `gone`.
fn prune_fields(
    kids: &mut PdfObject,
    widgets: &HashSet<i32>,
    gone: &mut HashSet<i32>,
    seen: &mut HashSet<i32>,
) -> Result<(), EngineError> {
    if !kids.is_array()? {
        return Ok(());
    }
    for index in (0..kids.len()?).rev() {
        let Some(kid) = kids.get_array(index_i32(index))? else {
            continue;
        };
        let number = number_of(&kid)?;
        if number != 0 && widgets.contains(&number) {
            kids.array_delete(index_i32(index))?;
            continue;
        }
        // A field listed twice, or in a cycle, is looked at once.
        if number != 0 && !seen.insert(number) {
            continue;
        }
        let Some(mut grandkids) = kid.get_dict("Kids")? else {
            continue;
        };
        let before = if grandkids.is_array()? {
            grandkids.len()?
        } else {
            0
        };
        prune_fields(&mut grandkids, widgets, gone, seen)?;
        if before > 0 && grandkids.len()? == 0 {
            kids.array_delete(index_i32(index))?;
            if number != 0 {
                gone.insert(number);
            }
        }
    }
    Ok(())
}

/// Goes through the structure tree, taking out the elements whose content was all on removed
/// pages.
struct StructureWalk<'a> {
    removed: &'a HashSet<i32>,
    /// The elements taken out, by object number.
    gone: HashSet<i32>,
    nodes: usize,
    /// An element in two places, or in a cycle, is looked at once.
    seen: HashSet<i32>,
}

/// What became of one entry of an element's /K.
enum Kid {
    Kept,
    Gone,
}

impl StructureWalk<'_> {
    fn on_kept_page(&self, page: Option<i32>) -> bool {
        // Content no page is given for cannot be told apart: it stays.
        page.is_none_or(|page| !self.removed.contains(&page))
    }

    /// Takes out of the tree root's /K what was only on removed pages.
    fn prune(&mut self, root: PdfObject) -> Result<bool, EngineError> {
        self.prune_element(root, None)
    }

    /// Takes out of `element`'s /K what was only on removed pages; returns whether anything of
    /// it is on a page that stays. `inherited` is the page its parent says its content is on.
    fn prune_element(
        &mut self,
        mut element: PdfObject,
        inherited: Option<i32>,
    ) -> Result<bool, EngineError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(EngineError::TooComplex);
        }
        let page = match element.get_dict("Pg")? {
            Some(page) if page.is_indirect()? => Some(page.as_indirect()?),
            _ => inherited,
        };
        let Some(mut kids) = element.get_dict("K")? else {
            return Ok(self.on_kept_page(page));
        };
        if kids.is_array()? {
            let mut kept = false;
            for index in (0..kids.len()?).rev() {
                let Some(kid) = kids.get_array(index_i32(index))? else {
                    continue;
                };
                match self.kid(kid, page)? {
                    Kid::Kept => kept = true,
                    Kid::Gone => kids.array_delete(index_i32(index))?,
                }
            }
            Ok(kept)
        } else {
            match self.kid(kids, page)? {
                Kid::Kept => Ok(true),
                Kid::Gone => {
                    element.dict_delete("K")?;
                    Ok(false)
                }
            }
        }
    }

    /// One entry of an element's /K: marked content on a page, an object on a page, or a child
    /// element.
    fn kid(&mut self, kid: PdfObject, page: Option<i32>) -> Result<Kid, EngineError> {
        let kept = |keep: bool| if keep { Kid::Kept } else { Kid::Gone };
        // A marked-content id: content on the element's page.
        if kid.is_int()? {
            return Ok(kept(self.on_kept_page(page)));
        }
        if !kid.is_dict()? {
            return Ok(Kid::Kept);
        }
        // A marked-content or object reference, on its own page or the element's.
        if matches!(name_of(&kid, "Type")?.as_deref(), Some(b"MCR" | b"OBJR")) {
            let own = match kid.get_dict("Pg")? {
                Some(own) if own.is_indirect()? => Some(own.as_indirect()?),
                _ => page,
            };
            return Ok(kept(self.on_kept_page(own)));
        }
        let number = number_of(&kid)?;
        if number != 0 && !self.seen.insert(number) {
            // Looked at already; if it was taken out, unlink cuts this reference too.
            return Ok(Kid::Kept);
        }
        if self.prune_element(kid.try_clone()?, page)? {
            Ok(Kid::Kept)
        } else {
            if number != 0 {
                self.gone.insert(number);
            }
            Ok(Kid::Gone)
        }
    }
}
