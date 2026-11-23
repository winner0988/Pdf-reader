//! Adding, retargeting and deleting link annotations (B6-03, Issue #588).

use std::collections::HashSet;

use ipc_contract::limits::MAX_TEXT_BYTES;
use ipc_contract::text::{classify_uri, clean_display_text};
use ipc_contract::types::{LinkId, LinkTarget, NewLinkTarget, Rect};
use ipc_contract::validate::{Validate, check_link_rect};
use mupdf::pdf::PdfObject;

use super::{EngineError, OutlineTarget, PdfDocument};

/// Maps an engine `OutlineTarget` to a contract `LinkTarget` for link reporting and lookup.
pub(crate) fn contract_target(target: OutlineTarget, page_count: u32) -> Option<LinkTarget> {
    match target {
        OutlineTarget::Page(page_index) if page_index < page_count => Some(LinkTarget::Page {
            page_index,
            x: None,
            y: None,
        }),
        OutlineTarget::Page(_) | OutlineTarget::None => None,
        OutlineTarget::Uri(uri) => Some(classify_uri(&uri)),
        OutlineTarget::Blocked { action, target } => Some(LinkTarget::Blocked {
            action,
            target: target
                .map(|text| clean_display_text(&text, MAX_TEXT_BYTES as usize))
                .filter(|text| !text.is_empty()),
        }),
    }
}

/// Recursively collects all indirect object xref numbers within an action dictionary or array
/// (such as chained `/Next` actions) so they can be unlinked from the document.
fn collect_action_xrefs(
    action: &PdfObject,
    unlinked: &mut HashSet<i32>,
) -> Result<(), mupdf::Error> {
    if action.is_indirect()? {
        let x = action.as_indirect()?;
        if x > 0 && !unlinked.insert(x) {
            return Ok(());
        }
    }
    if let Some(next) = action.get_dict("Next")? {
        if next.is_array()? {
            for item in next.array_iter()? {
                let item = item?;
                collect_action_xrefs(&item, unlinked)?;
            }
        } else if next.is_dict()? {
            collect_action_xrefs(&next, unlinked)?;
        }
    }
    Ok(())
}

impl PdfDocument {
    /// Finds the link annotation for `link`, returning the page `PdfObject`, the annotation `PdfObject`,
    /// and its index in the page's `/Annots` array.
    pub(crate) fn find_link_annot(
        &self,
        link: LinkId,
    ) -> Result<(PdfObject, PdfObject, usize), EngineError> {
        let page_count = self.page_count()?;
        if link.page_index >= page_count {
            return Err(EngineError::PageOutOfRange(link.page_index));
        }
        let page = self.doc.find_page(
            i32::try_from(link.page_index)
                .map_err(|_| EngineError::PageOutOfRange(link.page_index))?,
        )?;
        let ctm = page.page_ctm()?;
        let Some(annots) = page.get_dict("Annots")? else {
            return Err(EngineError::InvalidEdit("link not found"));
        };
        if !annots.is_array()? {
            return Err(EngineError::InvalidEdit("link not found"));
        }
        let mut link_idx: u32 = 0;
        for (annot_idx, annot_res) in annots.array_iter()?.enumerate() {
            let Ok(annot) = annot_res else { continue };
            let has_target = match self.link_entry(&annot, &ctm) {
                Ok(Some(entry)) => contract_target(entry.target, page_count).is_some(),
                _ => false,
            };
            if has_target {
                if link_idx == link.index {
                    return Ok((page, annot, annot_idx));
                }
                link_idx += 1;
            }
        }
        Err(EngineError::InvalidEdit("link not found"))
    }

    /// Adds a new link annotation over `rect` on page `page_index` (B6-03, Issue #588).
    ///
    /// The link dictionary contains only `/Type`, `/Subtype`, `/Rect`, `/BS`, `/F`, `/P`,
    /// and `/Dest` (for page target) or `/A` (for URI target).
    pub fn add_link(
        &mut self,
        page_index: u32,
        rect: Rect,
        target: &NewLinkTarget,
    ) -> Result<(), EngineError> {
        check_link_rect(&rect).map_err(|_| EngineError::InvalidEdit("the rectangle of a link"))?;
        target
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the target of a link"))?;

        let page_count = self.page_count()?;
        if page_index >= page_count {
            return Err(EngineError::PageOutOfRange(page_index));
        }
        if matches!(target, NewLinkTarget::Page { page } if *page >= page_count) {
            return Err(EngineError::InvalidEdit("link target page out of range"));
        }

        let mut page = self.doc.find_page(
            i32::try_from(page_index).map_err(|_| EngineError::PageOutOfRange(page_index))?,
        )?;
        let inverse = page
            .page_ctm()?
            .invert()
            .ok_or(EngineError::InvalidEdit("page cannot be placed"))?;

        let at = mupdf::Rect {
            x0: rect.x0,
            y0: rect.y0,
            x1: rect.x1,
            y1: rect.y1,
        }
        .transform(&inverse);

        let mut rect_obj = self.doc.new_array_with_capacity(4)?;
        rect_obj.array_push(PdfObject::new_real(at.x0)?)?;
        rect_obj.array_push(PdfObject::new_real(at.y0)?)?;
        rect_obj.array_push(PdfObject::new_real(at.x1)?)?;
        rect_obj.array_push(PdfObject::new_real(at.y1)?)?;

        let mut bs = self.doc.new_dict_with_capacity(1)?;
        bs.dict_put("W", PdfObject::new_int(0)?)?;

        let mut annot = self.doc.new_dict_with_capacity(7)?;
        annot.dict_put("Type", PdfObject::new_name("Annot")?)?;
        annot.dict_put("Subtype", PdfObject::new_name("Link")?)?;
        annot.dict_put("Rect", rect_obj)?;
        annot.dict_put("BS", bs)?;
        annot.dict_put("F", PdfObject::new_int(4)?)?;
        annot.dict_put("P", page.clone())?;

        match target {
            NewLinkTarget::Uri { uri } => {
                let mut action = self.doc.new_dict_with_capacity(2)?;
                action.dict_put("S", PdfObject::new_name("URI")?)?;
                action.dict_put("URI", PdfObject::new_string(uri)?)?;
                annot.dict_put("A", action)?;
            }
            NewLinkTarget::Page {
                page: dest_page_index,
            } => {
                let dest_page = self.doc.find_page(
                    i32::try_from(*dest_page_index)
                        .map_err(|_| EngineError::PageOutOfRange(*dest_page_index))?,
                )?;
                let dest_inv_ctm =
                    dest_page
                        .page_ctm()?
                        .invert()
                        .ok_or(EngineError::InvalidEdit(
                            "destination page cannot be placed",
                        ))?;
                let (left, top) = dest_inv_ctm.transform_xy(0.0, 0.0);

                let mut dest = self.doc.new_array_with_capacity(5)?;
                dest.array_push(dest_page.clone())?;
                dest.array_push(PdfObject::new_name("XYZ")?)?;
                dest.array_push(PdfObject::new_real(left)?)?;
                dest.array_push(PdfObject::new_real(top)?)?;
                dest.array_push(PdfObject::new_null())?;
                annot.dict_put("Dest", dest)?;
            }
        }

        let annot_ref = self.doc.add_object(&annot)?;
        match page.get_dict("Annots")? {
            Some(mut annots) => {
                annots.array_push(annot_ref)?;
            }
            None => {
                let mut annots = self.doc.new_array()?;
                annots.array_push(annot_ref)?;
                page.dict_put("Annots", annots)?;
            }
        }

        Ok(())
    }

    /// Changes where link `link` points to (B6-03, Issue #588).
    ///
    /// Strips existing `/A` (and recursively unlinks chained `/Next` actions), `/Dest`,
    /// `/AA`, and `/PA`, unlinking any indirect xrefs no longer referenced.
    pub fn set_link_target(
        &mut self,
        link: LinkId,
        target: &NewLinkTarget,
    ) -> Result<(), EngineError> {
        target
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the target of a link"))?;

        let page_count = self.page_count()?;
        if matches!(target, NewLinkTarget::Page { page } if *page >= page_count) {
            return Err(EngineError::InvalidEdit("link target page out of range"));
        }

        let (_page, mut annot, _annot_idx) = self.find_link_annot(link)?;

        let mut unlinked = HashSet::new();
        if let Some(action) = annot.get_dict("A")? {
            collect_action_xrefs(&action, &mut unlinked)?;
        }
        if let Some(aa) = annot.get_dict("AA")? {
            if aa.is_indirect()? {
                let x = aa.as_indirect()?;
                if x > 0 {
                    unlinked.insert(x);
                }
            }
            if aa.is_dict()? {
                for index in 0..aa.dict_len()? {
                    if let Some(sub) = aa.get_dict_val(i32::try_from(index).unwrap_or(i32::MAX))? {
                        collect_action_xrefs(&sub, &mut unlinked)?;
                    }
                }
            }
        }
        if let Some(pa) = annot.get_dict("PA")? {
            collect_action_xrefs(&pa, &mut unlinked)?;
        }

        annot.dict_delete("A")?;
        annot.dict_delete("Dest")?;
        annot.dict_delete("AA")?;
        annot.dict_delete("PA")?;

        match target {
            NewLinkTarget::Uri { uri } => {
                let mut action = self.doc.new_dict_with_capacity(2)?;
                action.dict_put("S", PdfObject::new_name("URI")?)?;
                action.dict_put("URI", PdfObject::new_string(uri)?)?;
                annot.dict_put("A", action)?;
            }
            NewLinkTarget::Page {
                page: dest_page_index,
            } => {
                let dest_page = self.doc.find_page(
                    i32::try_from(*dest_page_index)
                        .map_err(|_| EngineError::PageOutOfRange(*dest_page_index))?,
                )?;
                let dest_inv_ctm =
                    dest_page
                        .page_ctm()?
                        .invert()
                        .ok_or(EngineError::InvalidEdit(
                            "destination page cannot be placed",
                        ))?;
                let (left, top) = dest_inv_ctm.transform_xy(0.0, 0.0);

                let mut dest = self.doc.new_array_with_capacity(5)?;
                dest.array_push(dest_page.clone())?;
                dest.array_push(PdfObject::new_name("XYZ")?)?;
                dest.array_push(PdfObject::new_real(left)?)?;
                dest.array_push(PdfObject::new_real(top)?)?;
                dest.array_push(PdfObject::new_null())?;
                annot.dict_put("Dest", dest)?;
            }
        }

        crate::unlink::unlink(&self.doc, &unlinked)?;
        Ok(())
    }

    /// Removes link `link` from its page and unlinks indirect xrefs (B6-03, Issue #588).
    pub fn delete_link(&mut self, link: LinkId) -> Result<(), EngineError> {
        let (page, annot, annot_idx) = self.find_link_annot(link)?;

        let mut unlinked = HashSet::new();
        if annot.is_indirect()? {
            let x = annot.as_indirect()?;
            if x > 0 {
                unlinked.insert(x);
            }
        }
        if let Some(action) = annot.get_dict("A")? {
            collect_action_xrefs(&action, &mut unlinked)?;
        }
        if let Some(aa) = annot.get_dict("AA")? {
            if aa.is_indirect()? {
                let x = aa.as_indirect()?;
                if x > 0 {
                    unlinked.insert(x);
                }
            }
            if aa.is_dict()? {
                for index in 0..aa.dict_len()? {
                    if let Some(sub) = aa.get_dict_val(i32::try_from(index).unwrap_or(i32::MAX))? {
                        collect_action_xrefs(&sub, &mut unlinked)?;
                    }
                }
            }
        }
        if let Some(pa) = annot.get_dict("PA")? {
            collect_action_xrefs(&pa, &mut unlinked)?;
        }

        let Some(mut annots) = page.get_dict("Annots")? else {
            return Err(EngineError::InvalidEdit("no link annotations on page"));
        };
        annots.array_delete(
            i32::try_from(annot_idx)
                .map_err(|_| EngineError::InvalidEdit("annot index out of range"))?,
        )?;

        crate::unlink::unlink(&self.doc, &unlinked)?;
        Ok(())
    }
}
