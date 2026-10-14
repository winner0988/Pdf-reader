//! The prepared words of an open document's text boxes (B2-28, B2-29, ADR 0018,
//! docs/architecture/text-box-engine.md). The worker makes the words a user typed, and the font
//! they chose, into a small model (a subset of the font and what is needed to set the words); the
//! main process keeps the model here, named by a number, for as long as an edit of the document
//! uses it: the edit names the model, and every time the edit is made again (undo opens the
//! document again and makes all but the last edit again, ADR 0013) the worker is sent the model
//! with it, never the font, which is up to 64 MiB. The same models are in the crash recovery
//! journal (B2-13, `recovery.rs`).

use std::sync::Arc;

use ipc_contract::limits::MAX_UNDO_EDITS;
use ipc_contract::types::{Edit, TextBoxModelId};
use ipc_contract::worker::TextBoxModel;

/// Most prepared words one document keeps: one for each edit it can hold.
pub const MAX_TEXT_BOX_MODELS: usize = MAX_UNDO_EDITS as usize;

/// Most bytes of subsets one document keeps. A model is a few kilobytes; the journal, which holds
/// them too, is far smaller than this.
pub const MAX_MODEL_BYTES: usize = 64 * 1024 * 1024;

/// There is no room for more prepared words: every one kept is used by an edit.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// What a document keeps of the words of its text boxes.
#[derive(Debug, Default)]
pub struct TextBoxes {
    /// In the order they were prepared.
    kept: Vec<(TextBoxModelId, Arc<TextBoxModel>)>,
    /// The number the next model gets: never one that was used, so that a number never means
    /// two models (an edit of the history may still name an old one).
    next: u32,
}

/// The prepared words an edit puts in a box, if it does.
fn used_by(edit: &Edit) -> Option<TextBoxModelId> {
    match edit {
        Edit::AddTextBox { model, .. } => Some(*model),
        Edit::SetTextBox { model, .. } => *model,
        _ => None,
    }
}

impl TextBoxes {
    /// The model `id` names, for the worker.
    pub fn model(&self, id: TextBoxModelId) -> Option<TextBoxModel> {
        self.kept
            .iter()
            .find(|(kept, _)| *kept == id)
            .map(|(_, model)| model.as_ref().clone())
    }

    /// Keeps `model` under a new number. First the models that no edit of `history` (those done
    /// and those undone, which can be done again) uses are let go: words that were prepared but
    /// not used are replaced by the next the user prepares. If what is left, with this, is more
    /// than a document keeps, there is no room.
    pub fn add<'a>(
        &mut self,
        model: TextBoxModel,
        history: impl IntoIterator<Item = &'a Edit>,
    ) -> Result<TextBoxModelId, Full> {
        let used: Vec<TextBoxModelId> = history.into_iter().filter_map(used_by).collect();
        self.kept.retain(|(id, _)| used.contains(id));
        let bytes: usize = self.kept.iter().map(|(_, kept)| kept.subset.len()).sum();
        if self.kept.len() >= MAX_TEXT_BOX_MODELS || bytes + model.subset.len() > MAX_MODEL_BYTES {
            return Err(Full);
        }
        let id = TextBoxModelId(self.next);
        self.next += 1;
        self.kept.push((id, Arc::new(model)));
        Ok(id)
    }

    /// The models `edits` use, each once, in the order they are first used: what a journal keeps.
    pub fn used_by<'a>(&'a self, edits: &[Edit]) -> Vec<(TextBoxModelId, &'a TextBoxModel)> {
        let mut found: Vec<(TextBoxModelId, &TextBoxModel)> = Vec::new();
        for id in edits.iter().filter_map(used_by) {
            if found.iter().all(|(seen, _)| *seen != id)
                && let Some((_, model)) = self.kept.iter().find(|(kept, _)| *kept == id)
            {
                found.push((id, model.as_ref()));
            }
        }
        found
    }

    /// Takes the models of a journal, under their own numbers.
    pub fn restore(&mut self, models: Vec<(TextBoxModelId, TextBoxModel)>) {
        for (id, model) in models {
            if self.kept.iter().all(|(kept, _)| *kept != id) {
                self.next = self.next.max(id.0.saturating_add(1));
                self.kept.push((id, Arc::new(model)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{Rect, Rgb, TextAlign, TextBoxStyle};
    use ipc_contract::worker::ModelGlyph;

    use super::*;

    fn model(subset_len: usize) -> TextBoxModel {
        let mut subset = vec![0u8; subset_len.max(12)];
        subset[..4].copy_from_slice(&[0, 1, 0, 0]);
        TextBoxModel {
            text: "Hi".to_owned(),
            font: "Arial".to_owned(),
            face: 0,
            view_only: false,
            base: "ABCDEF+Arial".to_owned(),
            ascent: 905,
            descent: -212,
            cap_height: 716,
            step: 1_150,
            bbox: [-665, -325, 2_000, 1_006],
            glyphs: vec![
                ModelGlyph {
                    ch: 'H',
                    cid: 1,
                    advance: 722,
                },
                ModelGlyph {
                    ch: 'i',
                    cid: 2,
                    advance: 222,
                },
            ],
            subset,
        }
    }

    fn put(model: TextBoxModelId) -> Edit {
        Edit::AddTextBox {
            page: 0,
            rect: Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 50.0,
                y1: 50.0,
            },
            model,
            style: TextBoxStyle {
                size: 12.0,
                color: Rgb { r: 0, g: 0, b: 0 },
                align: TextAlign::Left,
            },
        }
    }

    #[test]
    fn prepared_words_are_kept_under_a_number_of_their_own() {
        let mut boxes = TextBoxes::default();
        let a = boxes.add(model(20), []).unwrap();
        let b = boxes.add(model(30), []).unwrap();
        assert_ne!(a, b);
        // The earlier ones were not used by an edit: the user prepared others.
        assert_eq!(boxes.model(a), None);
        assert_eq!(boxes.model(b).map(|m| m.subset.len()), Some(30));
    }

    #[test]
    fn words_an_edit_uses_stay_as_long_as_the_edit_can_be_made() {
        let mut boxes = TextBoxes::default();
        let a = boxes.add(model(20), []).unwrap();
        let history = [put(a)];
        let b = boxes.add(model(30), &history).unwrap();
        assert!(boxes.model(a).is_some() && boxes.model(b).is_some());
        // The edit is gone from the history (saved, or undone and replaced): so are its words,
        // and their number is not given again.
        let c = boxes.add(model(40), []).unwrap();
        assert!(boxes.model(a).is_none() && boxes.model(b).is_none());
        assert!(c.0 > b.0);
    }

    #[test]
    fn there_is_room_for_so_many_used_words_and_so_many_bytes() {
        let mut boxes = TextBoxes::default();
        let mut history = Vec::new();
        for _ in 0..MAX_TEXT_BOX_MODELS {
            let id = boxes.add(model(12), &history).unwrap();
            history.push(put(id));
        }
        assert_eq!(boxes.add(model(12), &history), Err(Full));
        // One edit less, one model less: there is room again.
        history.remove(0);
        assert!(boxes.add(model(12), &history).is_ok());

        // Few, and large: the bytes they take are what counts.
        let mut boxes = TextBoxes::default();
        let big = model(2 * 1024 * 1024);
        let mut history = Vec::new();
        for _ in 0..MAX_MODEL_BYTES / big.subset.len() {
            let id = boxes.add(big.clone(), &history).unwrap();
            history.push(put(id));
        }
        assert_eq!(boxes.add(big, &history), Err(Full));
    }

    #[test]
    fn a_journal_gets_the_words_its_edits_use_and_gives_them_back_under_their_numbers() {
        let mut boxes = TextBoxes::default();
        let a = boxes.add(model(20), []).unwrap();
        let history = [put(a), put(a), Edit::DeletePages { pages: vec![0] }];
        let b = boxes.add(model(30), &history).unwrap();
        // Once each, though two edits use it.
        let used = boxes.used_by(&history);
        assert_eq!(used.len(), 1);
        assert_eq!(used[0].0, a);
        assert_eq!(boxes.used_by(&[put(b)])[0].0, b);
        assert!(boxes.used_by(&[put(TextBoxModelId(99))]).is_empty());

        let mut later = TextBoxes::default();
        later.restore(vec![(TextBoxModelId(7), model(20))]);
        assert!(later.model(TextBoxModelId(7)).is_some());
        // A number from a journal is not given to other words.
        let id = later.add(model(30), [&put(TextBoxModelId(7))]).unwrap();
        assert!(id.0 > 7);
    }

    #[test]
    fn words_given_to_a_box_later_are_used_by_that_edit_too() {
        let set = |model: Option<TextBoxModelId>| Edit::SetTextBox {
            page: 0,
            annotation: ipc_contract::types::AnnotationId(5),
            model,
            style: TextBoxStyle {
                size: 12.0,
                color: Rgb { r: 0, g: 0, b: 0 },
                align: TextAlign::Left,
            },
        };
        let mut boxes = TextBoxes::default();
        let a = boxes.add(model(20), []).unwrap();
        let history = [set(Some(a)), set(None)];
        // Kept as long as the edit can be made, and in the journal; an edit without new words
        // names nothing.
        let b = boxes.add(model(30), &history).unwrap();
        assert!(boxes.model(a).is_some() && boxes.model(b).is_some());
        assert_eq!(boxes.used_by(&history).len(), 1);
        assert!(boxes.used_by(&[set(None)]).is_empty());
    }
}
