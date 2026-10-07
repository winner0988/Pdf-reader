//! The edits made to an open document since its file was read or last written (ADR 0013,
//! B2-05): the ones it has, in order, and the ones undone, which can be made again.
//!
//! Undo opens the document again from the bytes its worker keeps and applies all but the last
//! edit; redo applies the next one again. Once the document is written, the file is where
//! everything starts again: the history is emptied and the worker keeps the new file's bytes.
//!
//! The edits are kept as the page asked for them (`Edit`, validated): the ones applied are also
//! the crash recovery journal (B2-13, `recovery.rs`).

use ipc_contract::limits::MAX_UNDO_EDITS;
use ipc_contract::types::Edit;

#[derive(Debug, Default)]
pub struct History {
    /// Every edit since the file was read or last written. The first `applied` are in the
    /// document; the others were undone.
    edits: Vec<Edit>,
    applied: usize,
}

impl History {
    /// Whether the document differs from its file: some edit is in it.
    pub fn unsaved(&self) -> bool {
        self.applied > 0
    }

    pub fn can_undo(&self) -> bool {
        self.applied > 0
    }

    pub fn can_redo(&self) -> bool {
        self.applied < self.edits.len()
    }

    /// Whether another edit is kept: at most `MAX_UNDO_EDITS` between two saves.
    pub fn has_room(&self) -> bool {
        self.applied < MAX_UNDO_EDITS as usize
    }

    /// The edits the document has, in order: to open its file again with them.
    pub fn applied(&self) -> &[Edit] {
        &self.edits[..self.applied]
    }

    /// Every edit kept: the ones the document has, and those undone, which can be made again.
    pub fn all(&self) -> &[Edit] {
        &self.edits
    }

    /// `edit` was applied. What was undone can no longer be made again.
    pub fn push(&mut self, edit: Edit) {
        self.edits.truncate(self.applied);
        self.edits.push(edit);
        self.applied += 1;
    }

    /// The edits the document has once its last one is undone; `None` with nothing to undo.
    pub fn before_last(&self) -> Option<&[Edit]> {
        self.applied.checked_sub(1).map(|kept| &self.edits[..kept])
    }

    /// The last edit was undone.
    pub fn undone(&mut self) {
        self.applied = self.applied.saturating_sub(1);
    }

    /// The edit to make again; `None` with nothing undone.
    pub fn next(&self) -> Option<&Edit> {
        self.edits.get(self.applied)
    }

    /// The edit from `next` was made again.
    pub fn redone(&mut self) {
        self.applied = (self.applied + 1).min(self.edits.len());
    }

    /// The document was written to its file, which now has every edit: nothing to undo or redo.
    pub fn saved(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delete(page: u32) -> Edit {
        Edit::DeletePages { pages: vec![page] }
    }

    #[test]
    fn undo_and_redo_go_back_and_forth_through_the_edits() {
        let mut history = History::default();
        assert!(!history.unsaved() && !history.can_undo() && !history.can_redo());
        history.push(delete(1));
        history.push(delete(2));
        assert_eq!(history.before_last(), Some(&[delete(1)][..]));
        history.undone();
        assert_eq!(history.applied(), [delete(1)]);
        assert_eq!(history.next(), Some(&delete(2)));
        history.undone();
        // Back to the file: nothing unsaved, both can be made again.
        assert!(!history.unsaved() && !history.can_undo() && history.can_redo());
        assert_eq!(history.before_last(), None);
        history.redone();
        assert_eq!(history.applied(), [delete(1)]);
        assert!(history.unsaved());
    }

    #[test]
    fn a_new_edit_drops_what_was_undone() {
        let mut history = History::default();
        history.push(delete(1));
        history.push(delete(2));
        history.undone();
        history.push(delete(3));
        assert_eq!(history.applied(), [delete(1), delete(3)]);
        assert!(!history.can_redo());
    }

    #[test]
    fn saving_starts_over_and_edits_are_bounded() {
        let mut history = History::default();
        for _ in 0..MAX_UNDO_EDITS {
            assert!(history.has_room());
            history.push(delete(0));
        }
        assert!(!history.has_room());
        history.saved();
        assert!(history.has_room() && !history.unsaved() && !history.can_redo());
    }
}
