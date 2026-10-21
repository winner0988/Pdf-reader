//! Bookmark (outline) editing (B3-17, ADR 0013, docs/architecture/outline-editing.md): adding,
//! renaming, moving and deleting bookmarks.
//!
//! Every edit first reads the outline into a tree ([`Tree::read`]) the way it is shown
//! ([`PdfDocument::outline`]): from the `/Outlines` dictionary through `/First` and `/Next`, an
//! item reached a second time ending its branch. Every item must be an indirect dictionary whose
//! `/Parent`, if it has one, is the dictionary it was reached from: an object that says it belongs
//! elsewhere (a page, whose `/Parent` is in the page tree) is not taken for a bookmark. An outline
//! with such an object, or bigger than the outline limits, is refused as a whole rather than
//! edited in part. A bookmark id is taken only when it is an item of the tree, so no id reaches
//! any other object of the document.
//!
//! The edit is made to the tree, and then the `/Parent`, `/First`, `/Last`, `/Prev`, `/Next` and
//! `/Count` of every item are written from it again: after an edit the outline is what was shown,
//! with the edit, whatever links it had. `/Count` keeps whether each item was open or closed.

use std::collections::HashSet;

use ipc_contract::limits::{MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS, MAX_TEXT_BYTES};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::BookmarkId;
use mupdf::Point;
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};

use super::{EngineError, PdfDocument};

impl PdfDocument {
    /// Adds a bookmark called `title` that goes to the top of page `page` (0-based), as the
    /// `index`-th child of `parent` (`None` for the top level; past the end, the last).
    pub fn add_bookmark(
        &mut self,
        parent: Option<BookmarkId>,
        index: u32,
        title: &str,
        page: u32,
    ) -> Result<BookmarkId, EngineError> {
        if page >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(page));
        }
        let title = clean_title(title)?;
        let mut tree = match Tree::read(&self.doc)? {
            Some(tree) => tree,
            None => Tree::new(&mut self.doc)?,
        };
        let under = parent.map(|id| tree.find(id)).transpose()?;
        if tree.items.len() >= MAX_OUTLINE_ITEMS as usize {
            return Err(EngineError::InvalidEdit("too many bookmarks"));
        }
        if under.map_or(0, |parent| tree.depth(parent) + 1) > MAX_OUTLINE_DEPTH {
            return Err(EngineError::InvalidEdit("bookmarks nested too deep"));
        }

        let mut item = self.doc.new_dict()?;
        item.dict_put("Title", self.doc.new_string(&title)?)?;
        item.dict_put("Dest", self.top_of_page(page)?)?;
        let object = self.doc.add_object(&item)?;
        let number = object.as_indirect()?;
        let added = tree.items.len();
        tree.items.push(Item {
            object,
            number,
            parent: under,
            children: Vec::new(),
            closed: false,
        });
        tree.insert(under, index, added);
        tree.write()?;
        Ok(BookmarkId(u32::try_from(number).unwrap_or(0)))
    }

    /// Renames the bookmark `id` to `title`.
    pub fn rename_bookmark(&mut self, id: BookmarkId, title: &str) -> Result<(), EngineError> {
        let title = clean_title(title)?;
        let tree = Tree::read(&self.doc)?.ok_or(NO_BOOKMARK)?;
        let mut object = tree.items[tree.find(id)?].object.clone();
        object.dict_put("Title", self.doc.new_string(&title)?)?;
        tree.write()
    }

    /// Removes the bookmark `id` and every bookmark under it.
    /// What still points to them is cut, as for removed pages, so that their titles are not kept
    /// in the file when it is written.
    pub fn delete_bookmark(&mut self, id: BookmarkId) -> Result<(), EngineError> {
        let mut tree = Tree::read(&self.doc)?.ok_or(NO_BOOKMARK)?;
        let deleted = tree.find(id)?;
        let parent = tree.items[deleted].parent;
        tree.children_mut(parent).retain(|&child| child != deleted);
        let gone: HashSet<i32> = tree
            .subtree(deleted)
            .into_iter()
            .map(|index| tree.items[index].number)
            .collect();
        tree.write()?;
        crate::unlink::unlink(&self.doc, &gone)
    }

    /// Moves the bookmark `id`, with the bookmarks under it, to be the `index`-th child of
    /// `parent` (`None` for the top level; past the end, the last). Moving up or down and to
    /// another level are all this.
    pub fn move_bookmark(
        &mut self,
        id: BookmarkId,
        parent: Option<BookmarkId>,
        index: u32,
    ) -> Result<(), EngineError> {
        let mut tree = Tree::read(&self.doc)?.ok_or(NO_BOOKMARK)?;
        let moved = tree.find(id)?;
        let under = parent.map(|id| tree.find(id)).transpose()?;
        if under.is_some_and(|under| tree.subtree(moved).contains(&under)) {
            return Err(EngineError::InvalidEdit(
                "a bookmark cannot go under itself",
            ));
        }
        let depth = under.map_or(0, |parent| tree.depth(parent) + 1);
        if depth + tree.height(moved) > MAX_OUTLINE_DEPTH {
            return Err(EngineError::InvalidEdit("bookmarks nested too deep"));
        }
        let from = tree.items[moved].parent;
        tree.children_mut(from).retain(|&child| child != moved);
        tree.items[moved].parent = under;
        tree.insert(under, index, moved);
        tree.write()
    }

    /// An explicit destination to the top left of page `index` as it is shown, with the zoom
    /// left as it is: `[page /XYZ left top null]`, in the page's own space.
    fn top_of_page(&self, index: u32) -> Result<PdfObject, EngineError> {
        let (page, inverse) = self.page_to_pdf(index)?;
        let bounds = self.load_page(index)?.bounds()?;
        let corner = Point::new(bounds.x0, bounds.y0).transform(&inverse);
        let mut dest = self.doc.new_array()?;
        dest.array_push(page)?;
        dest.array_push(PdfObject::new_name("XYZ")?)?;
        dest.array_push(PdfObject::new_real(corner.x)?)?;
        dest.array_push(PdfObject::new_real(corner.y)?)?;
        dest.array_push(PdfObject::new_null())?;
        Ok(dest)
    }
}

/// An id that is not one of the bookmarks shown.
const NO_BOOKMARK: EngineError = EngineError::InvalidEdit("no such bookmark");

/// Why an outline is not edited: one of its items is not what a bookmark is.
const NOT_EDITABLE: EngineError = EngineError::InvalidEdit("the outline cannot be edited");

/// `title` as a bookmark shows it, which must not be blank.
fn clean_title(title: &str) -> Result<String, EngineError> {
    let title = clean_display_text(title, MAX_TEXT_BYTES as usize);
    if title.is_empty() {
        return Err(EngineError::InvalidEdit("a blank bookmark title"));
    }
    Ok(title)
}

/// A bookmark of the outline read.
struct Item {
    /// The item (an indirect reference).
    object: PdfObject,
    number: i32,
    /// Its parent in `Tree::items`; `None` for the top level.
    parent: Option<usize>,
    /// Its children in `Tree::items`, in order.
    children: Vec<usize>,
    /// Whether it was closed when read (a negative `/Count`).
    closed: bool,
}

/// The outline as it is shown: the `/Outlines` dictionary and the bookmarks under it.
struct Tree {
    /// The `/Outlines` dictionary (an indirect reference).
    root: PdfObject,
    /// The top-level bookmarks in `items`, in order.
    top: Vec<usize>,
    /// Every bookmark: an item is never removed from here, only from its parent's children.
    items: Vec<Item>,
}

impl Tree {
    /// Reads the outline of `doc`; `None` if it has none. In the order of
    /// [`PdfDocument::outline`] (an item, its children, then its next sibling), so that an item
    /// reached twice is kept where it is shown.
    fn read(doc: &MuPdfDocument) -> Result<Option<Tree>, EngineError> {
        let Some(root) = doc.catalog()?.get_dict("Outlines")? else {
            return Ok(None);
        };
        if !root.is_dict()? {
            return Ok(None);
        }
        let kind = root.get_dict("Type")?;
        let outlines = match &kind {
            None => true,
            Some(kind) => kind.is_name()? && kind.as_name()? == b"Outlines",
        };
        if !root.is_indirect()? || !outlines {
            return Err(NOT_EDITABLE);
        }
        let mut tree = Tree {
            root,
            top: Vec::new(),
            items: Vec::new(),
        };
        let mut seen = HashSet::from([tree.root.as_indirect()?]);
        // (item, its parent, its depth), the next one to read on top.
        let mut pending: Vec<(PdfObject, Option<usize>, u16)> = Vec::new();
        if let Some(first) = tree.root.get_dict("First")? {
            pending.push((first, None, 0));
        }
        while let Some((node, parent, depth)) = pending.pop() {
            // A null or a missing object ends its list, as when shown.
            if !node.is_dict()? {
                continue;
            }
            if !node.is_indirect()? {
                return Err(NOT_EDITABLE);
            }
            let number = node.as_indirect()?;
            if !seen.insert(number) {
                continue;
            }
            let parent_object = parent.map_or(&tree.root, |parent| &tree.items[parent].object);
            if let Some(said) = node.get_dict("Parent")?
                && !(said.is_indirect()? && said.as_indirect()? == parent_object.as_indirect()?)
            {
                return Err(NOT_EDITABLE);
            }
            if tree.items.len() == MAX_OUTLINE_ITEMS as usize || depth > MAX_OUTLINE_DEPTH {
                return Err(EngineError::InvalidEdit("the outline is too big to edit"));
            }
            let closed = match node.get_dict("Count")? {
                Some(count) => count.is_int()? && count.as_int()? < 0,
                None => false,
            };
            let index = tree.items.len();
            if let Some(next) = node.get_dict("Next")? {
                pending.push((next, parent, depth));
            }
            if let Some(first) = node.get_dict("First")? {
                pending.push((first, Some(index), depth + 1));
            }
            tree.items.push(Item {
                object: node,
                number,
                parent,
                children: Vec::new(),
                closed,
            });
            tree.children_mut(parent).push(index);
        }
        Ok(Some(tree))
    }

    /// Gives `doc` an empty outline.
    fn new(doc: &mut MuPdfDocument) -> Result<Tree, EngineError> {
        let mut root = doc.new_dict()?;
        root.dict_put("Type", PdfObject::new_name("Outlines")?)?;
        let root = doc.add_object(&root)?;
        doc.catalog()?.dict_put("Outlines", root.clone())?;
        Ok(Tree {
            root,
            top: Vec::new(),
            items: Vec::new(),
        })
    }

    /// The item that is bookmark `id`.
    fn find(&self, id: BookmarkId) -> Result<usize, EngineError> {
        let number = i32::try_from(id.0).map_err(|_| NO_BOOKMARK)?;
        self.items
            .iter()
            .position(|item| item.number == number)
            .ok_or(NO_BOOKMARK)
    }

    /// The children of `parent` (`None` for the top level).
    fn children_mut(&mut self, parent: Option<usize>) -> &mut Vec<usize> {
        match parent {
            None => &mut self.top,
            Some(parent) => &mut self.items[parent].children,
        }
    }

    /// Puts `item` as the `index`-th child of `parent`, or the last.
    fn insert(&mut self, parent: Option<usize>, index: u32, item: usize) {
        let children = self.children_mut(parent);
        let at = usize::try_from(index).map_or(children.len(), |at| at.min(children.len()));
        children.insert(at, item);
    }

    /// The level of `item`: 0 for the top level.
    fn depth(&self, item: usize) -> u16 {
        let mut depth = 0;
        let mut at = self.items[item].parent;
        while let Some(parent) = at {
            depth += 1;
            at = self.items[parent].parent;
        }
        depth
    }

    /// How many levels there are under `item`: 0 for none.
    fn height(&self, item: usize) -> u16 {
        let mut height = 0;
        let mut pending = vec![(item, 0)];
        while let Some((at, level)) = pending.pop() {
            height = height.max(level);
            pending.extend(
                self.items[at]
                    .children
                    .iter()
                    .map(|&child| (child, level + 1)),
            );
        }
        height
    }

    /// `item` and every item under it.
    fn subtree(&self, item: usize) -> Vec<usize> {
        let mut all = Vec::new();
        let mut pending = vec![item];
        while let Some(at) = pending.pop() {
            all.push(at);
            pending.extend(&self.items[at].children);
        }
        all
    }

    /// Writes the links of every item from the tree.
    fn write(&self) -> Result<(), EngineError> {
        // Every item before its children.
        let mut order = Vec::new();
        let mut pending: Vec<usize> = self.top.iter().rev().copied().collect();
        while let Some(at) = pending.pop() {
            order.push(at);
            pending.extend(self.items[at].children.iter().rev());
        }
        // How many items under each one are shown when it is open: children first.
        let mut shown = vec![0_i32; self.items.len()];
        for &at in order.iter().rev() {
            shown[at] = self.shown_under(&self.items[at].children, &shown);
        }
        let count = Some(self.shown_under(&self.top, &shown)).filter(|&count| count > 0);
        self.link(&self.root, &self.top, count)?;
        for &at in &order {
            let item = &self.items[at];
            let count = Some(shown[at])
                .filter(|&count| count > 0)
                .map(|count| if item.closed { -count } else { count });
            self.link(&item.object, &item.children, count)?;
        }
        Ok(())
    }

    /// How many of `children` and the items under them are shown: those under a closed one are
    /// not.
    fn shown_under(&self, children: &[usize], shown: &[i32]) -> i32 {
        children
            .iter()
            .map(|&child| {
                1 + if self.items[child].closed {
                    0
                } else {
                    shown[child]
                }
            })
            .sum()
    }

    /// Makes `children` the items under `parent`, in that order, with `count` its `/Count`.
    fn link(
        &self,
        parent: &PdfObject,
        children: &[usize],
        count: Option<i32>,
    ) -> Result<(), EngineError> {
        let mut parent_dict = parent.clone();
        match (children.first(), children.last()) {
            (Some(&first), Some(&last)) => {
                parent_dict.dict_put("First", self.items[first].object.clone())?;
                parent_dict.dict_put("Last", self.items[last].object.clone())?;
            }
            _ => {
                parent_dict.dict_delete("First")?;
                parent_dict.dict_delete("Last")?;
            }
        }
        match count {
            Some(count) => parent_dict.dict_put("Count", PdfObject::new_int(count)?)?,
            None => parent_dict.dict_delete("Count")?,
        }
        for (at, &child) in children.iter().enumerate() {
            let mut item = self.items[child].object.clone();
            item.dict_put("Parent", parent.clone())?;
            match at.checked_sub(1) {
                Some(before) => {
                    item.dict_put("Prev", self.items[children[before]].object.clone())?
                }
                None => item.dict_delete("Prev")?,
            }
            match children.get(at + 1) {
                Some(&after) => item.dict_put("Next", self.items[after].object.clone())?,
                None => item.dict_delete("Next")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::OutlineTarget;
    use super::super::tests::{build_pdf, outline_pdf};
    use super::*;

    /// The outline as shown: (title, depth).
    fn shown(doc: &PdfDocument) -> Vec<(String, u16)> {
        let outline = doc.outline(10_000, 64).unwrap();
        assert!(!outline.truncated);
        outline
            .entries
            .into_iter()
            .map(|entry| (entry.title, entry.depth))
            .collect()
    }

    fn titles(entries: &[(&str, u16)]) -> Vec<(String, u16)> {
        entries
            .iter()
            .map(|&(title, depth)| (title.to_owned(), depth))
            .collect()
    }

    /// The ids of the bookmarks shown, in order.
    fn ids(doc: &PdfDocument) -> Vec<BookmarkId> {
        let outline = doc.outline(10_000, 64).unwrap();
        outline.entries.iter().map(|entry| entry.id).collect()
    }

    /// Asserts that the links of every item under `parent` agree with each other, and returns how
    /// many items under it are shown when it is open.
    fn assert_linked_under(parent: &PdfObject) -> i32 {
        let number = parent.as_indirect().unwrap();
        let mut children = Vec::new();
        let mut shown = 0;
        let mut next = parent.get_dict("First").unwrap();
        while let Some(child) = next {
            assert!(children.len() < 1_000, "a cycle under {number}");
            let said = child.get_dict("Parent").unwrap().unwrap();
            assert_eq!(said.as_indirect().unwrap(), number);
            let prev = child.get_dict("Prev").unwrap();
            assert_eq!(
                prev.map(|prev| prev.as_indirect().unwrap()),
                children.last().copied()
            );
            let under = assert_linked_under(&child);
            let count = child
                .get_dict("Count")
                .unwrap()
                .map(|c| c.as_int().unwrap());
            if child.get_dict("First").unwrap().is_some() {
                assert_eq!(count.map(i32::abs), Some(under));
            } else {
                assert_eq!(count, None);
            }
            shown += 1 + if count.is_some_and(|count| count > 0) {
                under
            } else {
                0
            };
            children.push(child.as_indirect().unwrap());
            next = child.get_dict("Next").unwrap();
        }
        let last = parent.get_dict("Last").unwrap();
        assert_eq!(
            last.map(|last| last.as_indirect().unwrap()),
            children.last().copied()
        );
        shown
    }

    /// Asserts that the links of the whole outline of `doc` agree with each other.
    fn assert_linked(doc: &PdfDocument) {
        let root = doc
            .doc
            .catalog()
            .unwrap()
            .get_dict("Outlines")
            .unwrap()
            .unwrap();
        let shown = assert_linked_under(&root);
        let count = root.get_dict("Count").unwrap().map(|c| c.as_int().unwrap());
        assert_eq!(count, Some(shown).filter(|&shown| shown > 0));
    }

    #[test]
    fn bookmarks_are_added_renamed_moved_and_deleted_with_every_link_kept() {
        let mut doc = PdfDocument::from_bytes(&outline_pdf(
            3,
            &[
                ("(A)", 0, Some(0)),
                ("(A.1)", 1, Some(1)),
                ("(B)", 0, Some(2)),
            ],
        ))
        .unwrap();
        let [a, a1, b] = ids(&doc)[..] else { panic!() };

        // Past the end is the last.
        let a2 = doc.add_bookmark(Some(a), 99, "A.2", 2).unwrap();
        assert_eq!(
            shown(&doc),
            titles(&[("A", 0), ("A.1", 1), ("A.2", 1), ("B", 0)])
        );
        assert_eq!(
            doc.outline(10, 64).unwrap().entries[2].target,
            OutlineTarget::Page(2)
        );
        assert_linked(&doc);

        doc.rename_bookmark(b, "第二章 結論").unwrap();
        doc.move_bookmark(b, Some(a), 0).unwrap();
        assert_eq!(
            shown(&doc),
            titles(&[("A", 0), ("第二章 結論", 1), ("A.1", 1), ("A.2", 1)])
        );
        assert_linked(&doc);

        doc.move_bookmark(a1, None, 0).unwrap();
        doc.move_bookmark(a2, Some(b), 0).unwrap();
        assert_eq!(
            shown(&doc),
            titles(&[("A.1", 0), ("A", 0), ("第二章 結論", 1), ("A.2", 2)])
        );
        assert_linked(&doc);

        doc.delete_bookmark(a).unwrap();
        assert_eq!(shown(&doc), titles(&[("A.1", 0)]));
        assert_linked(&doc);
        doc.delete_bookmark(a1).unwrap();
        assert_eq!(shown(&doc), titles(&[]));
        assert_linked(&doc);

        // An outline with nothing left takes a bookmark again.
        doc.add_bookmark(None, 0, "C", 0).unwrap();
        assert_eq!(shown(&doc), titles(&[("C", 0)]));
        assert_linked(&doc);
    }

    #[test]
    fn a_document_without_an_outline_is_given_one() {
        let mut doc = PdfDocument::from_bytes(&outline_pdf(2, &[("(A)", 0, None)])).unwrap();
        let mut catalog = doc.doc.catalog().unwrap();
        catalog.dict_delete("Outlines").unwrap();
        assert!(doc.rename_bookmark(BookmarkId(5), "B").is_err());

        doc.add_bookmark(None, 0, "Start", 1).unwrap();
        assert_eq!(shown(&doc), titles(&[("Start", 0)]));
        assert_eq!(
            doc.outline(10, 64).unwrap().entries[0].target,
            OutlineTarget::Page(1)
        );
        assert_linked(&doc);
    }

    #[test]
    fn an_id_that_is_not_a_bookmark_shown_is_refused() {
        // 1 catalog, 2 pages, 3 the page, 4 the outline root, 5 bookmark A.
        let mut doc = PdfDocument::from_bytes(&outline_pdf(1, &[("(A)", 0, Some(0))])).unwrap();
        for id in [
            BookmarkId(1),
            BookmarkId(2),
            BookmarkId(3),
            BookmarkId(4),
            BookmarkId(u32::MAX),
        ] {
            assert!(doc.rename_bookmark(id, "X").is_err());
            assert!(doc.delete_bookmark(id).is_err());
            assert!(doc.move_bookmark(id, None, 0).is_err());
            assert!(doc.move_bookmark(BookmarkId(5), Some(id), 0).is_err());
            assert!(doc.add_bookmark(Some(id), 0, "X", 0).is_err());
        }
        let page = doc.doc.find_page(0).unwrap();
        assert!(page.get_dict("Title").unwrap().is_none());
        assert_eq!(
            page.get_dict("Parent")
                .unwrap()
                .unwrap()
                .as_indirect()
                .unwrap(),
            2
        );
        assert_eq!(doc.page_count().unwrap(), 1);
        assert_eq!(shown(&doc), titles(&[("A", 0)]));
    }

    #[test]
    fn an_object_that_belongs_elsewhere_is_not_taken_for_a_bookmark() {
        // The outline's first item is the page, whose /Parent is the page tree.
        let mut doc = PdfDocument::from_bytes(&build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 3 0 R /Last 3 0 R /Count 1 >>",
        ]))
        .unwrap();
        assert!(doc.rename_bookmark(BookmarkId(3), "X").is_err());
        assert!(doc.delete_bookmark(BookmarkId(3)).is_err());
        assert!(doc.add_bookmark(None, 0, "X", 0).is_err());
        let page = doc.doc.find_page(0).unwrap();
        assert!(page.get_dict("Title").unwrap().is_none());
        assert!(page.get_dict("Next").unwrap().is_none());
        assert_eq!(
            page.get_dict("Parent")
                .unwrap()
                .unwrap()
                .as_indirect()
                .unwrap(),
            2
        );
        assert_eq!(doc.page_count().unwrap(), 1);
    }

    #[test]
    fn a_cyclic_outline_is_written_as_it_is_shown() {
        // B's /Next points back at A (A -> B -> A -> ...), and A.1's /First at its own parent.
        let mut doc = PdfDocument::from_bytes(&build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>",
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /First 7 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
            "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Next 5 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
            "<< /Title (A.1) /Parent 5 0 R /First 5 0 R /Dest [3 0 R /XYZ 0 792 0] >>",
        ]))
        .unwrap();
        doc.rename_bookmark(BookmarkId(7), "A.1 new").unwrap();
        // Shown before as now, but with no cycle left.
        assert_eq!(shown(&doc), titles(&[("A", 0), ("A.1 new", 1), ("B", 0)]));
        assert_linked(&doc);

        doc.move_bookmark(BookmarkId(6), Some(BookmarkId(7)), 0)
            .unwrap();
        assert_eq!(shown(&doc), titles(&[("A", 0), ("A.1 new", 1), ("B", 2)]));
        assert_linked(&doc);
    }

    #[test]
    fn closed_bookmarks_stay_closed() {
        // A is closed over its one child; B is open.
        let mut doc = PdfDocument::from_bytes(&build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>",
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /First 7 0 R /Last 7 0 R /Count -1 >>",
            "<< /Title (B) /Parent 4 0 R /Prev 5 0 R >>",
            "<< /Title (A.1) /Parent 5 0 R >>",
        ]))
        .unwrap();
        doc.add_bookmark(Some(BookmarkId(5)), 1, "A.2", 0).unwrap();
        doc.add_bookmark(Some(BookmarkId(6)), 0, "B.1", 0).unwrap();
        assert_linked(&doc);
        let count = |number: i32| {
            let object = doc.doc.new_indirect(number, 0).unwrap();
            object
                .get_dict("Count")
                .unwrap()
                .map(|c| c.as_int().unwrap())
        };
        assert_eq!(count(5), Some(-2));
        assert_eq!(count(6), Some(1));
        // A, B and B.1: A's children are not shown.
        assert_eq!(count(4), Some(3));
    }

    #[test]
    fn a_new_bookmark_goes_to_the_top_left_of_its_page_as_shown() {
        let mut doc = PdfDocument::from_bytes(&build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [50 60 550 700] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 >>",
        ]))
        .unwrap();
        let cropped = doc.add_bookmark(None, 0, "Cropped", 0).unwrap();
        let turned = doc.add_bookmark(None, 1, "Turned", 1).unwrap();
        let corner = |id: BookmarkId| {
            let item = doc
                .doc
                .new_indirect(i32::try_from(id.0).unwrap(), 0)
                .unwrap();
            // No action: a plain destination.
            assert!(item.get_dict("A").unwrap().is_none());
            let dest = item.get_dict("Dest").unwrap().unwrap();
            let page = dest.get_array(0).unwrap().unwrap();
            let at = |i: i32| dest.get_array(i).unwrap().unwrap().as_float().unwrap();
            (doc.doc.lookup_page_number(&page).unwrap(), at(2), at(3))
        };
        assert_eq!(corner(cropped), (0, 50.0, 700.0));
        assert_eq!(corner(turned), (1, 0.0, 0.0));
        let outline = doc.outline(10, 64).unwrap();
        assert_eq!(outline.entries[1].target, OutlineTarget::Page(1));
    }

    #[test]
    fn bookmarks_nest_no_deeper_than_the_limit() {
        // A0 > A1 > ... > A64, then B > B1.
        let titles: Vec<String> = (0..=64).map(|n| format!("(A{n})")).collect();
        let mut items: Vec<(&str, u16, Option<usize>)> = titles
            .iter()
            .enumerate()
            .map(|(depth, title)| (title.as_str(), depth as u16, None))
            .collect();
        items.push(("(B)", 0, None));
        items.push(("(B1)", 1, None));
        let mut doc = PdfDocument::from_bytes(&outline_pdf(1, &items)).unwrap();
        let all = ids(&doc);
        let (a, b) = (&all[..65], all[65]);

        assert!(doc.add_bookmark(Some(a[64]), 0, "X", 0).is_err());
        assert!(doc.move_bookmark(b, Some(a[63]), 0).is_err());
        assert!(doc.move_bookmark(a[1], Some(a[30]), 0).is_err());
        doc.add_bookmark(Some(a[63]), 0, "X", 0).unwrap();
        doc.move_bookmark(b, Some(a[62]), 0).unwrap();
        assert_linked(&doc);

        // An outline deeper than shown is not edited.
        let titles: Vec<String> = (0..=65).map(|n| format!("(A{n})")).collect();
        let items: Vec<(&str, u16, Option<usize>)> = titles
            .iter()
            .enumerate()
            .map(|(depth, title)| (title.as_str(), depth as u16, None))
            .collect();
        let mut doc = PdfDocument::from_bytes(&outline_pdf(1, &items)).unwrap();
        let top = doc.outline(10, 64).unwrap().entries[0].id;
        assert!(doc.rename_bookmark(top, "X").is_err());
    }

    #[test]
    fn a_deleted_bookmark_is_not_kept_in_the_file() {
        let mut doc = PdfDocument::from_bytes(&outline_pdf(
            1,
            &[
                ("(Keep)", 0, Some(0)),
                ("(Secret)", 0, Some(0)),
                ("(Secret child)", 1, None),
            ],
        ))
        .unwrap();
        let secret = ids(&doc)[1];
        doc.delete_bookmark(secret).unwrap();
        let mut saved = Vec::new();
        doc.save(&mut saved).unwrap();

        let copy = PdfDocument::from_bytes(&saved).unwrap();
        assert_eq!(shown(&copy), titles(&[("Keep", 0)]));
        for number in 1..copy.doc.xref_len().unwrap() {
            let Some(object) = copy
                .doc
                .xref_object(i32::try_from(number).unwrap())
                .unwrap()
            else {
                continue;
            };
            if object.is_dict().unwrap()
                && let Some(title) = object.get_dict("Title").unwrap()
            {
                assert!(!title.as_string().unwrap().contains("Secret"));
            }
        }
    }
}
