//! The pictures of an open document's picture stamps (B2-08, docs/architecture/annotations.md).
//! The worker makes a picture the user chose into a small PNG file of its pixels alone; the main
//! process keeps that file here, named by a number, for as long as an edit of the document uses
//! it: the edit names the picture, and every time the edit is made again (undo opens the document
//! again and makes all but the last edit again, ADR 0013) the worker is sent the file with it.
//! The same pictures are in the crash recovery journal (B2-13, `recovery.rs`).

use std::sync::Arc;

use ipc_contract::types::{Edit, StampImageId};

/// Most pictures one document keeps. A document has seldom more than one or two; each is at most
/// `MAX_STAMP_PNG_BYTES`, so this bounds the memory they take.
pub const MAX_PICTURES: usize = 16;

/// There is no room for another picture: every one kept is used by an edit.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// What a document keeps of its picture stamps.
#[derive(Debug, Default)]
pub struct Pictures {
    /// The PNG files, in the order the pictures were made.
    kept: Vec<(StampImageId, Arc<Vec<u8>>)>,
    /// The number the next picture gets: never one that was used, so that a number never means
    /// two pictures (an edit of the history may still name an old one).
    next: u32,
}

/// The picture an edit puts on a page, if it does.
fn used_by(edit: &Edit) -> Option<StampImageId> {
    match edit {
        Edit::AddImageStamp { image, .. } => Some(*image),
        _ => None,
    }
}

impl Pictures {
    /// The PNG file of picture `id`.
    pub fn png(&self, id: StampImageId) -> Option<Vec<u8>> {
        self.kept
            .iter()
            .find(|(kept, _)| *kept == id)
            .map(|(_, png)| png.as_ref().clone())
    }

    /// Keeps `png` as a new picture. First the pictures that no edit of `history` (those done
    /// and those undone, which can be done again) uses are let go: a picture that was made but
    /// not used is replaced by the user's next choice. If every one left is in use there is no
    /// room.
    pub fn add<'a>(
        &mut self,
        png: Vec<u8>,
        history: impl IntoIterator<Item = &'a Edit>,
    ) -> Result<StampImageId, Full> {
        let used: Vec<StampImageId> = history.into_iter().filter_map(used_by).collect();
        self.kept.retain(|(id, _)| used.contains(id));
        if self.kept.len() >= MAX_PICTURES {
            return Err(Full);
        }
        let id = StampImageId(self.next);
        self.next += 1;
        self.kept.push((id, Arc::new(png)));
        Ok(id)
    }

    /// The pictures `edits` use, in the order they are first used: what a journal keeps.
    pub fn used_by<'a>(&'a self, edits: &[Edit]) -> Vec<(StampImageId, &'a [u8])> {
        let mut found: Vec<(StampImageId, &[u8])> = Vec::new();
        for id in edits.iter().filter_map(used_by) {
            if found.iter().all(|(seen, _)| *seen != id)
                && let Some((_, png)) = self.kept.iter().find(|(kept, _)| *kept == id)
            {
                found.push((id, png.as_slice()));
            }
        }
        found
    }

    /// Takes the pictures of a journal, under their own numbers.
    pub fn restore(&mut self, pictures: Vec<(StampImageId, Vec<u8>)>) {
        for (id, png) in pictures {
            if self.kept.iter().all(|(kept, _)| *kept != id) {
                self.next = self.next.max(id.0.saturating_add(1));
                self.kept.push((id, Arc::new(png)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::Rect;

    use super::*;

    fn stamp(image: StampImageId) -> Edit {
        Edit::AddImageStamp {
            page: 0,
            rect: Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 50.0,
                y1: 50.0,
            },
            image,
        }
    }

    #[test]
    fn a_picture_is_kept_under_a_number_of_its_own() {
        let mut pictures = Pictures::default();
        let a = pictures.add(vec![1], []).unwrap();
        let b = pictures.add(vec![2], []).unwrap();
        assert_ne!(a, b);
        // The earlier one was not used by an edit: the user chose another.
        assert_eq!(pictures.png(a), None);
        assert_eq!(pictures.png(b), Some(vec![2]));
    }

    #[test]
    fn a_picture_an_edit_uses_stays_as_long_as_the_edit_can_be_made() {
        let mut pictures = Pictures::default();
        let a = pictures.add(vec![1], []).unwrap();
        let history = [stamp(a)];
        let b = pictures.add(vec![2], &history).unwrap();
        assert_eq!(pictures.png(a), Some(vec![1]));
        assert_eq!(pictures.png(b), Some(vec![2]));
        // The edit is gone from the history (saved, or undone and replaced): so is its picture,
        // and its number is not given again.
        let c = pictures.add(vec![3], []).unwrap();
        assert_eq!(pictures.png(a), None);
        assert_eq!(pictures.png(b), None);
        assert!(c.0 > b.0);
    }

    #[test]
    fn there_is_room_for_so_many_used_pictures() {
        let mut pictures = Pictures::default();
        let mut history = Vec::new();
        for index in 0..MAX_PICTURES {
            let id = pictures.add(vec![index as u8], &history).unwrap();
            history.push(stamp(id));
        }
        assert_eq!(pictures.add(vec![0], &history), Err(Full));
        // One edit less, one picture less: there is room again.
        history.remove(0);
        assert!(pictures.add(vec![9], &history).is_ok());
    }

    #[test]
    fn a_journal_gets_the_pictures_its_edits_use_and_gives_them_back_under_their_numbers() {
        let mut pictures = Pictures::default();
        let a = pictures.add(vec![1, 1], []).unwrap();
        let history = [stamp(a), stamp(a), Edit::DeletePages { pages: vec![0] }];
        let b = pictures.add(vec![2], &history).unwrap();
        // Once each, though two edits use it.
        assert_eq!(pictures.used_by(&history), [(a, &[1u8, 1][..])]);
        assert_eq!(pictures.used_by(&[stamp(b)]), [(b, &[2u8][..])]);
        assert!(pictures.used_by(&[stamp(StampImageId(99))]).is_empty());

        let mut later = Pictures::default();
        later.restore(vec![(StampImageId(7), vec![7])]);
        assert_eq!(later.png(StampImageId(7)), Some(vec![7]));
        // A number from a journal is not given to another picture.
        let id = later.add(vec![8], [&stamp(StampImageId(7))]).unwrap();
        assert!(id.0 > 7);
    }
}
