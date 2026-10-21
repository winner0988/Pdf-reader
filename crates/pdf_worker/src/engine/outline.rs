//! Bookmark (outline) editing (B3-17, ADR 0013): adding, renaming, moving and deleting bookmarks.
//!
//! Maintains consistency of `/First`, `/Last`, `/Next`, `/Prev`, `/Parent` and `/Count` (preserving
//! open/closed sign) across all outline modifications. Targets are created as explicit page destinations
//! (`/Dest [page /XYZ 0 height 0]`), with no active actions.

use std::collections::HashSet;

use ipc_contract::limits::{MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS, MAX_TEXT_BYTES};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::BookmarkId;
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};

use super::{EngineError, PdfDocument};

impl PdfDocument {
    /// Adds a bookmark pointing to page `page` (0-based) (B3-17).
    /// If `parent` is `None`, added to the top-level outline.
    /// `index` is the 0-based insertion position among siblings.
    pub fn add_bookmark(
        &mut self,
        parent: Option<BookmarkId>,
        index: u32,
        title: &str,
        page: u32,
    ) -> Result<BookmarkId, EngineError> {
        let pages = self.page_count()?;
        if page >= pages {
            return Err(EngineError::PageOutOfRange(page));
        }

        let clean = clean_display_text(title, MAX_TEXT_BYTES as usize);
        if clean.trim().is_empty() {
            return Err(EngineError::InvalidEdit("bookmark title cannot be blank"));
        }

        let current_total = total_items(&self.doc)?;
        if current_total >= MAX_OUTLINE_ITEMS as usize {
            return Err(EngineError::InvalidEdit("maximum number of bookmarks reached"));
        }

        let (mut parent_obj, depth) = match parent {
            None => {
                let root = ensure_outlines_root(&mut self.doc)?;
                (root, 0)
            }
            Some(parent_id) => {
                let p = self.doc.new_indirect(parent_id.0 as i32, 0)?;
                if !p.is_dict()? {
                    return Err(EngineError::InvalidEdit("parent bookmark does not exist"));
                }
                let p_depth = depth_of(&self.doc, &p)?;
                if p_depth + 1 > MAX_OUTLINE_DEPTH {
                    return Err(EngineError::InvalidEdit("maximum outline depth exceeded"));
                }
                (p, p_depth + 1)
            }
        };

        if depth > MAX_OUTLINE_DEPTH {
            return Err(EngineError::InvalidEdit("maximum outline depth exceeded"));
        }

        // Construct explicit page destination [page /XYZ 0 height 0]
        let page_obj = self.doc.find_page(page as i32)?;
        let (_, height) = self.page_size(page)?;
        let mut dest = self.doc.new_array()?;
        dest.array_push(page_obj)?;
        dest.array_push(PdfObject::new_name("XYZ")?)?;
        dest.array_push(PdfObject::new_int(0)?)?;
        dest.array_push(PdfObject::new_int(height as i32)?)?;
        dest.array_push(PdfObject::new_int(0)?)?;

        let mut item_dict = self.doc.new_dict()?;
        item_dict.dict_put("Title", self.doc.new_string(&clean)?)?;
        item_dict.dict_put("Dest", dest)?;
        let item_obj = self.doc.add_object(&item_dict)?;

        let mut siblings = children_of(&parent_obj)?;
        let insert_idx = (index as usize).min(siblings.len());
        siblings.insert(insert_idx, item_obj.clone());
        set_children(&mut parent_obj, &mut siblings)?;

        recompute_all_counts(&mut self.doc)?;

        let id = u32::try_from(item_obj.as_indirect()?).unwrap_or(0);
        Ok(BookmarkId(id))
    }

    /// Renames the bookmark `id` to `title` (B3-17).
    pub fn rename_bookmark(&mut self, id: BookmarkId, title: &str) -> Result<(), EngineError> {
        let clean = clean_display_text(title, MAX_TEXT_BYTES as usize);
        if clean.trim().is_empty() {
            return Err(EngineError::InvalidEdit("bookmark title cannot be blank"));
        }
        let mut node = self.doc.new_indirect(id.0 as i32, 0)?;
        if !node.is_dict()? {
            return Err(EngineError::InvalidEdit("bookmark does not exist"));
        }
        node.dict_put("Title", self.doc.new_string(&clean)?)?;
        Ok(())
    }

    /// Removes the bookmark `id` and all of its descendants (B3-17).
    pub fn delete_bookmark(&mut self, id: BookmarkId) -> Result<(), EngineError> {
        let node = self.doc.new_indirect(id.0 as i32, 0)?;
        if !node.is_dict()? {
            return Err(EngineError::InvalidEdit("bookmark does not exist"));
        }

        // 1. Unlink node from its parent
        if let Some(mut parent) = node.get_dict("Parent")? {
            let mut siblings = children_of(&parent)?;
            siblings.retain(|s| s.as_indirect().ok() != node.as_indirect().ok());
            set_children(&mut parent, &mut siblings)?;
        }

        // 2. Collect all indirect object numbers in node and its descendants
        let mut gone = HashSet::new();
        collect_subtree_objects(&node, &mut gone)?;

        // 3. Recompute counts
        recompute_all_counts(&mut self.doc)?;

        // 4. If outline is now completely empty, clean up root /First, /Last and /Count
        clean_empty_root(&mut self.doc)?;

        // 5. Unlink the deleted objects
        crate::unlink::unlink(&self.doc, &gone)?;
        Ok(())
    }

    /// Moves the bookmark `id` (and its descendants) to be the `index`-th child of `parent`
    /// (`None` for top-level) (B3-17).
    pub fn move_bookmark(
        &mut self,
        id: BookmarkId,
        parent: Option<BookmarkId>,
        index: u32,
    ) -> Result<(), EngineError> {
        let node = self.doc.new_indirect(id.0 as i32, 0)?;
        if !node.is_dict()? {
            return Err(EngineError::InvalidEdit("bookmark does not exist"));
        }
        let node_obj_num = node.as_indirect()?;

        // Destination parent
        let dest_parent = match parent {
            None => ensure_outlines_root(&mut self.doc)?,
            Some(parent_id) => {
                if parent_id == id {
                    return Err(EngineError::InvalidEdit("cannot move bookmark into itself"));
                }
                let p = self.doc.new_indirect(parent_id.0 as i32, 0)?;
                if !p.is_dict()? {
                    return Err(EngineError::InvalidEdit("destination parent does not exist"));
                }
                p
            }
        };

        // Check that dest_parent is NOT a descendant of node
        if is_descendant_of(&self.doc, &dest_parent, node_obj_num)? {
            return Err(EngineError::InvalidEdit(
                "cannot move bookmark into its own descendant",
            ));
        }

        // Check depth
        let dest_parent_depth = if parent.is_none() {
            0
        } else {
            depth_of(&self.doc, &dest_parent)? + 1
        };
        let subtree_height = subtree_depth(&node)?;
        if dest_parent_depth + subtree_height > MAX_OUTLINE_DEPTH {
            return Err(EngineError::InvalidEdit("maximum outline depth exceeded"));
        }

        // Unlink from current parent
        if let Some(mut current_parent) = node.get_dict("Parent")? {
            let mut siblings = children_of(&current_parent)?;
            siblings.retain(|s| s.as_indirect().ok() != Some(node_obj_num));
            set_children(&mut current_parent, &mut siblings)?;
        }

        // Re-fetch dest_parent in case it was modified
        let mut dest_parent = match parent {
            None => ensure_outlines_root(&mut self.doc)?,
            Some(parent_id) => self.doc.new_indirect(parent_id.0 as i32, 0)?,
        };

        // Insert into dest_parent at index
        let mut siblings = children_of(&dest_parent)?;
        let insert_idx = (index as usize).min(siblings.len());
        siblings.insert(insert_idx, node.clone());
        set_children(&mut dest_parent, &mut siblings)?;

        // Recompute counts
        recompute_all_counts(&mut self.doc)?;
        Ok(())
    }
}

/// Ensures the catalog has an `/Outlines` dictionary, creating one if absent.
fn ensure_outlines_root(doc: &mut MuPdfDocument) -> Result<PdfObject, EngineError> {
    let mut catalog = doc.catalog()?;
    if let Some(root) = catalog.get_dict("Outlines")? {
        if root.is_dict()? {
            return Ok(root);
        }
    }
    let mut outlines = doc.new_dict()?;
    outlines.dict_put("Type", PdfObject::new_name("Outlines")?)?;
    let outlines_obj = doc.add_object(&outlines)?;
    catalog.dict_put("Outlines", outlines_obj.clone())?;
    Ok(outlines_obj)
}

/// Children of an outline item or the root dictionary, traversed via `/First` and `/Next`.
fn children_of(parent: &PdfObject) -> Result<Vec<PdfObject>, EngineError> {
    let mut children = Vec::new();
    let mut current = parent.get_dict("First")?;
    let mut seen = HashSet::new();
    while let Some(node) = current {
        if !node.is_dict()? {
            break;
        }
        if node.is_indirect()? && !seen.insert(node.as_indirect()?) {
            break;
        }
        let next = node.get_dict("Next")?;
        children.push(node);
        current = next;
    }
    Ok(children)
}

/// Sets the children of `parent`, maintaining `/First`, `/Last`, `/Parent`, `/Prev`, and `/Next`.
fn set_children(parent: &mut PdfObject, children: &mut [PdfObject]) -> Result<(), EngineError> {
    if children.is_empty() {
        parent.dict_delete("First")?;
        parent.dict_delete("Last")?;
        parent.dict_delete("Count")?;
        return Ok(());
    }

    let first = children[0].clone();
    let last = children[children.len() - 1].clone();

    parent.dict_put("First", first)?;
    parent.dict_put("Last", last)?;

    let len = children.len();
    for i in 0..len {
        let prev = if i > 0 { Some(children[i - 1].clone()) } else { None };
        let next = if i + 1 < len { Some(children[i + 1].clone()) } else { None };
        let child = &mut children[i];
        child.dict_put("Parent", parent.clone())?;

        if let Some(p) = prev {
            child.dict_put("Prev", p)?;
        } else {
            child.dict_delete("Prev")?;
        }

        if let Some(n) = next {
            child.dict_put("Next", n)?;
        } else {
            child.dict_delete("Next")?;
        }
    }
    Ok(())
}

/// Total outline items in the document.
fn total_items(doc: &MuPdfDocument) -> Result<usize, EngineError> {
    let Some(root) = doc.catalog()?.get_dict("Outlines")? else {
        return Ok(0);
    };
    if !root.is_dict()? {
        return Ok(0);
    }
    let mut count = 0;
    let mut stack = children_of(&root)?;
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node.is_indirect()? && !seen.insert(node.as_indirect()?) {
            continue;
        }
        count += 1;
        for child in children_of(&node)? {
            stack.push(child);
        }
    }
    Ok(count)
}

/// Calculates depth of `node` (0 = top-level child under /Outlines root).
fn depth_of(doc: &MuPdfDocument, node: &PdfObject) -> Result<u16, EngineError> {
    let root = doc.catalog()?.get_dict("Outlines")?;
    let root_num = root.as_ref().and_then(|r| r.as_indirect().ok());
    let mut current = node.clone();
    let mut depth: u16 = 0;
    let mut seen = HashSet::new();
    while let Some(parent) = current.get_dict("Parent")? {
        if let Ok(num) = parent.as_indirect() {
            if Some(num) == root_num {
                break;
            }
            if !seen.insert(num) {
                break;
            }
        }
        depth = depth.saturating_add(1);
        current = parent;
    }
    Ok(depth)
}

/// Maximum depth beneath `node` (0 if `node` has no children).
fn subtree_depth(node: &PdfObject) -> Result<u16, EngineError> {
    let children = children_of(node)?;
    if children.is_empty() {
        return Ok(0);
    }
    let mut max_child: u16 = 0;
    for child in children {
        let d = subtree_depth(&child)?;
        if d > max_child {
            max_child = d;
        }
    }
    Ok(1 + max_child)
}

/// Checks whether `candidate` is a descendant of the outline item with object number `ancestor_num`.
fn is_descendant_of(
    doc: &MuPdfDocument,
    candidate: &PdfObject,
    ancestor_num: i32,
) -> Result<bool, EngineError> {
    let root = doc.catalog()?.get_dict("Outlines")?;
    let root_num = root.as_ref().and_then(|r| r.as_indirect().ok());
    let mut current = candidate.clone();
    let mut seen = HashSet::new();
    while let Some(parent) = current.get_dict("Parent")? {
        if let Ok(num) = parent.as_indirect() {
            if num == ancestor_num {
                return Ok(true);
            }
            if Some(num) == root_num || !seen.insert(num) {
                break;
            }
        }
        current = parent;
    }
    Ok(false)
}

/// Recursively collects all indirect object numbers within `node`'s subtree (including `node`).
fn collect_subtree_objects(node: &PdfObject, gone: &mut HashSet<i32>) -> Result<(), EngineError> {
    if let Ok(num) = node.as_indirect() {
        if !gone.insert(num) {
            return Ok(());
        }
    }
    for child in children_of(node)? {
        collect_subtree_objects(&child, gone)?;
    }
    Ok(())
}

/// If root `/Outlines` has no children left, removes `/First`, `/Last`, `/Count`.
fn clean_empty_root(doc: &mut MuPdfDocument) -> Result<(), EngineError> {
    let Some(mut root) = doc.catalog()?.get_dict("Outlines")? else {
        return Ok(());
    };
    if !root.is_dict()? {
        return Ok(());
    }
    let top = children_of(&root)?;
    if top.is_empty() {
        root.dict_delete("First")?;
        root.dict_delete("Last")?;
        root.dict_delete("Count")?;
    }
    Ok(())
}

/// Recomputes `/Count` across all outline items and the root dictionary,
/// strictly preserving the open/closed sign (Count < 0) of existing items.
fn recompute_all_counts(doc: &mut MuPdfDocument) -> Result<(), EngineError> {
    let Some(mut root) = doc.catalog()?.get_dict("Outlines")? else {
        return Ok(());
    };
    if !root.is_dict()? {
        return Ok(());
    }
    let mut top_children = children_of(&root)?;
    if top_children.is_empty() {
        root.dict_delete("First")?;
        root.dict_delete("Last")?;
        root.dict_delete("Count")?;
        return Ok(());
    }
    let mut total_visible = 0;
    for child in &mut top_children {
        let (visible_under, is_open) = recompute_node_count(child)?;
        total_visible += 1;
        if is_open {
            total_visible += visible_under;
        }
    }
    root.dict_put("Count", PdfObject::new_int(total_visible as i32)?)?;
    Ok(())
}

/// Recursively computes and sets `/Count` on `node`.
/// Returns `(visible_descendants_when_open, is_open)`.
fn recompute_node_count(node: &mut PdfObject) -> Result<(usize, bool), EngineError> {
    let mut children = children_of(node)?;
    if children.is_empty() {
        node.dict_delete("First")?;
        node.dict_delete("Last")?;
        node.dict_delete("Count")?;
        return Ok((0, true));
    }

    let was_closed = node
        .get_dict("Count")?
        .and_then(|c| c.as_int().ok())
        .map_or(false, |c| c < 0);

    let mut total_descendants_if_open = 0;
    for child in &mut children {
        let (child_descendants_if_open, child_is_open) = recompute_node_count(child)?;
        total_descendants_if_open += 1;
        if child_is_open {
            total_descendants_if_open += child_descendants_if_open;
        }
    }

    let count_val = if was_closed {
        -(total_descendants_if_open as i32)
    } else {
        total_descendants_if_open as i32
    };

    node.dict_put("Count", PdfObject::new_int(count_val)?)?;
    Ok((total_descendants_if_open, !was_closed))
}
