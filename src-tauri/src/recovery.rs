//! Crash recovery (B2-13, ADR 0013, docs/architecture/crash-recovery.md). The edits a document
//! has and that are not in its file yet are kept in a journal in the app's local data folder,
//! rewritten after every change. Saving, discarding the changes or closing the tab deletes it;
//! one that is still there when its file opens again was left by a run that ended first, and
//! the user is offered to make its edits again.
//!
//! A journal is read as untrusted data, like any file in the data folder: with a size limit,
//! only in the app's format, and with every edit checked as an edit from the page is. The
//! pictures of picture stamps (B2-08) are in it too, as the edits name them: as hex text, each
//! a PNG file that is checked as the worker's is.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ipc_contract::limits::MAX_UNDO_EDITS;
use ipc_contract::types::{Edit, StampImageId};
use ipc_contract::validate::{Validate, stamp_png_size};
use serde::{Deserialize, Serialize};

use crate::local_data;
use crate::pictures::MAX_PICTURES;
use crate::recent::key;
use crate::saving::FileIdentity;

/// The folder in the data folder.
pub const FOLDER_NAME: &str = "recovery";
/// The largest journal. An edit that would make one larger is refused: the document has to be
/// saved first.
pub const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;
/// Journals looked at when a file opens. A run has at most one per tab; more were not all left
/// by the app.
const MAX_JOURNALS: usize = 64;
const VERSION: u32 = 1;
const ID_BYTES: usize = 16;
const EXTENSION: &str = "json";

/// A journal file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stored {
    version: u32,
    /// The file's full path, as it was opened.
    path: String,
    /// The file as it was read: its size, and when it last changed (since the Unix epoch).
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
    /// The edits made to it since, in order.
    edits: Vec<Edit>,
    /// The pictures those edits name (B2-08).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pictures: Vec<StoredPicture>,
    /// How many more were made, which are not here: the first that put the pages of another file
    /// into the document, and all after it (B2-06; the journal cannot keep the other file).
    #[serde(default, skip_serializing_if = "is_zero")]
    lost: u32,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// A picture in a journal: its number, and its PNG file as hex text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPicture {
    id: u32,
    png: String,
}

/// The pictures an edit list names, with their PNG files (B2-08).
pub type PictureFiles<'a> = &'a [(StampImageId, &'a [u8])];

/// The same, read from a journal.
pub type ReadPictures = Vec<(StampImageId, Vec<u8>)>;

/// A journal's file name in the folder: random, so the folder says nothing of the files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JournalId(String);

/// A journal an earlier run left for a file.
#[derive(Debug, PartialEq)]
pub struct Found {
    pub id: JournalId,
    pub edits: Vec<Edit>,
    /// The pictures the edits name, under the numbers they have in them.
    pub pictures: ReadPictures,
    /// How many edits of the run are not in `edits` (see `Stored::lost`).
    pub lost: u32,
    /// The file is as it was when the edits were made: they can be made on it again.
    pub same_file: bool,
}

/// The journal would be larger than `MAX_JOURNAL_BYTES`.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

pub struct Journals {
    /// `None` without a data folder: then nothing is kept.
    folder: Option<PathBuf>,
    /// The journals this run uses: its tabs' own, and those offered to a tab. No other tab is
    /// offered them, and clearing leaves them.
    in_use: Mutex<HashSet<JournalId>>,
}

impl Journals {
    pub fn new(folder: Option<PathBuf>) -> Self {
        Self {
            folder,
            in_use: Mutex::new(HashSet::new()),
        }
    }

    /// A name for a new journal; nothing is written until `write`. `None` without a data folder.
    pub fn create(&self) -> Option<JournalId> {
        self.folder.as_ref()?;
        let mut bytes = [0u8; ID_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let id = JournalId(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
        self.lock().insert(id.clone());
        Some(id)
    }

    /// Whether a journal of `edits` (and the `pictures` they name) to the file at `path`, and
    /// `lost` more that it cannot keep, would be small enough to keep.
    pub fn check(
        path: &Path,
        identity: Option<FileIdentity>,
        edits: &[Edit],
        pictures: PictureFiles,
        lost: u32,
    ) -> Result<(), TooLarge> {
        match encode(path, identity, edits, pictures, lost) {
            Some(json) if json.len() > MAX_JOURNAL_BYTES => Err(TooLarge),
            _ => Ok(()),
        }
    }

    /// Replaces journal `id` with `edits` to the file at `path`, which was `identity` when read,
    /// and the `pictures` they name, and says that `lost` more were made. Best effort: if the file
    /// cannot be written, a crash loses the changes, nothing else.
    pub fn write(
        &self,
        id: &JournalId,
        path: &Path,
        identity: Option<FileIdentity>,
        edits: &[Edit],
        pictures: PictureFiles,
        lost: u32,
    ) -> Result<(), TooLarge> {
        let (Some(file), Some(json)) =
            (self.file(id), encode(path, identity, edits, pictures, lost))
        else {
            return Ok(());
        };
        if json.len() > MAX_JOURNAL_BYTES {
            return Err(TooLarge);
        }
        let _ = local_data::write(&file, &json);
        Ok(())
    }

    /// Deletes journal `id`: its edits were saved or discarded.
    pub fn remove(&self, id: &JournalId) {
        if let Some(file) = self.file(id) {
            local_data::remove(&file);
        }
        self.lock().remove(id);
    }

    /// Leaves journal `id` for a later run, or a later opening of its file: its tab closed
    /// before the user answered, or lost its document.
    pub fn release(&self, id: &JournalId) {
        self.lock().remove(id);
    }

    /// The journal an earlier run left for the file at `path`, which is now `now`; it is then in
    /// use. Journals that are not the app's, or whose edits are not valid, are left alone.
    pub fn find(&self, path: &Path, now: Option<FileIdentity>) -> Option<Found> {
        let folder = self.folder.as_ref()?;
        let mut names: Vec<String> = fs::read_dir(folder)
            .ok()?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| id_of(name).is_some())
            .collect();
        names.sort();
        names.truncate(MAX_JOURNALS);
        names
            .iter()
            .filter_map(|name| id_of(name))
            .find_map(|id| self.take(&id, path, now))
    }

    /// Journal `id`, if it is one for the file at `path` and not in use; it is then in use.
    pub fn take(&self, id: &JournalId, path: &Path, now: Option<FileIdentity>) -> Option<Found> {
        if self.lock().contains(id) {
            return None;
        }
        let (stored, pictures) = read(&self.file(id)?, path)?;
        // Checked by `read`: the nanoseconds never carry into the seconds.
        let then = FileIdentity::from_parts(
            stored.len,
            Duration::new(stored.modified_secs, stored.modified_nanos),
        );
        // Another tab may have taken it meanwhile.
        if !self.lock().insert(id.clone()) {
            return None;
        }
        Some(Found {
            id: id.clone(),
            edits: stored.edits,
            pictures,
            lost: stored.lost,
            same_file: now.is_some() && now == then,
        })
    }

    /// Deletes every journal this run does not use (clearing the recent files list, B2-12).
    pub fn clear_unused(&self) {
        let Some(folder) = self.folder.as_ref() else {
            return;
        };
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };
        let in_use = self.lock().clone();
        for entry in entries.filter_map(Result::ok) {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            // A leftover of an interrupted write (`local_data::write`) too.
            let stem = name.strip_suffix(".tmp").unwrap_or(&name);
            if id_of(stem).is_some_and(|id| !in_use.contains(&id)) {
                local_data::remove(&entry.path());
            }
        }
    }

    fn file(&self, id: &JournalId) -> Option<PathBuf> {
        Some(self.folder.as_ref()?.join(format!("{}.{EXTENSION}", id.0)))
    }

    fn lock(&self) -> MutexGuard<'_, HashSet<JournalId>> {
        self.in_use.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The id of a journal file name: 32 lowercase hex digits and the extension.
fn id_of(name: &str) -> Option<JournalId> {
    let hex = name.strip_suffix(EXTENSION)?.strip_suffix('.')?;
    (hex.len() == 2 * ID_BYTES
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    .then(|| JournalId(hex.to_owned()))
}

/// The journal file's content; `None` if the file has no usable identity or its path no text.
fn encode(
    path: &Path,
    identity: Option<FileIdentity>,
    edits: &[Edit],
    pictures: PictureFiles,
    lost: u32,
) -> Option<Vec<u8>> {
    let (len, since_epoch) = identity?.parts()?;
    serde_json::to_vec(&Stored {
        version: VERSION,
        path: path.to_str()?.to_owned(),
        len,
        modified_secs: since_epoch.as_secs(),
        modified_nanos: since_epoch.subsec_nanos(),
        edits: edits.to_vec(),
        pictures: pictures
            .iter()
            .map(|(id, png)| StoredPicture {
                id: id.0,
                png: hex(png),
            })
            .collect(),
        lost,
    })
    .ok()
}

/// `bytes` as lowercase hex text.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    text
}

/// The bytes of lowercase hex `text`; `None` for anything else.
fn unhex(text: &str) -> Option<Vec<u8>> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|&[high, low]| Some(digit(high)? << 4 | digit(low)?))
        .collect()
}

/// Journal `file`, if it is one the app could have written for the file at `path`; with its
/// pictures, decoded.
fn read(file: &Path, path: &Path) -> Option<(Stored, ReadPictures)> {
    let json = local_data::read(file, MAX_JOURNAL_BYTES as u64)?;
    let stored: Stored = serde_json::from_slice(&json).ok()?;
    let valid = stored.version == VERSION
        && Path::new(&stored.path).is_absolute()
        && key(Path::new(&stored.path)) == key(path)
        && stored.modified_nanos < 1_000_000_000
        && (!stored.edits.is_empty() || stored.lost > 0)
        && stored.edits.len() <= MAX_UNDO_EDITS as usize
        && stored.lost <= MAX_UNDO_EDITS
        && stored.edits.len() + stored.lost as usize <= MAX_UNDO_EDITS as usize
        // The journal cannot keep another file: it never has an edit that takes its pages.
        && stored
            .edits
            .iter()
            .all(|edit| !matches!(edit, Edit::InsertPages { .. }) && edit.validate().is_ok());
    if !valid {
        return None;
    }
    let pictures = pictures_of(&stored)?;
    Some((stored, pictures))
}

/// The pictures of a journal, if they are all good: each a PNG file whose header says a size a
/// stamp can have, no number twice, and one for every picture the edits name (B2-08). The worker
/// checks and decodes a picture again when an edit puts it on a page.
fn pictures_of(stored: &Stored) -> Option<ReadPictures> {
    if stored.pictures.len() > MAX_PICTURES {
        return None;
    }
    let mut pictures = ReadPictures::new();
    for picture in &stored.pictures {
        let id = StampImageId(picture.id);
        let png = unhex(&picture.png).filter(|png| stamp_png_size(png).is_ok())?;
        if pictures.iter().any(|(seen, _)| *seen == id) {
            return None;
        }
        pictures.push((id, png));
    }
    let named = stored.edits.iter().all(|edit| match edit {
        Edit::AddImageStamp { image, .. } => pictures.iter().any(|(id, _)| id == image),
        _ => true,
    });
    named.then_some(pictures)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use ipc_contract::types::Rotation;

    use super::*;

    /// A data folder of its own for each test, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let folder = std::env::temp_dir().join(format!(
                "pdf-reader-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&folder);
            Self(folder)
        }

        fn journals(&self) -> Journals {
            Journals::new(Some(self.0.join(FOLDER_NAME)))
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.0.join(FOLDER_NAME))
                .map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file() -> PathBuf {
        PathBuf::from(r"C:\Users\someone\報告.pdf")
    }

    fn identity(len: u64) -> Option<FileIdentity> {
        FileIdentity::from_parts(len, Duration::new(1_790_000_000, 123))
    }

    fn edits() -> Vec<Edit> {
        vec![
            Edit::DeletePages { pages: vec![2] },
            Edit::RotatePages {
                pages: vec![0, 1],
                by: Rotation::Cw90,
            },
        ]
    }

    #[test]
    fn a_journal_left_by_an_earlier_run_is_found_for_its_file_only() {
        let folder = Folder::new();
        let earlier = folder.journals();
        let id = earlier.create().unwrap();
        earlier
            .write(&id, &file(), identity(1_000), &edits(), &[], 0)
            .unwrap();
        assert_eq!(folder.files(), [format!("{}.json", id.0)]);
        // The name says nothing of the file.
        assert!(!folder.files()[0].contains("報告"));

        let later = folder.journals();
        assert_eq!(
            later.find(Path::new(r"C:\Users\someone\other.pdf"), identity(1_000)),
            None
        );
        // Windows paths ignore case.
        let found = later
            .find(Path::new(r"c:\users\SOMEONE\報告.PDF"), identity(1_000))
            .unwrap();
        assert_eq!(
            found,
            Found {
                id: id.clone(),
                edits: edits(),
                pictures: Vec::new(),
                lost: 0,
                same_file: true
            }
        );
        // In use now: not offered twice.
        assert_eq!(later.find(&file(), identity(1_000)), None);
        later.remove(&id);
        assert!(folder.files().is_empty());
    }

    #[test]
    fn a_file_changed_since_is_told_apart() {
        let folder = Folder::new();
        let earlier = folder.journals();
        let id = earlier.create().unwrap();
        earlier
            .write(&id, &file(), identity(1_000), &edits(), &[], 0)
            .unwrap();
        let found = folder.journals().find(&file(), identity(999)).unwrap();
        assert!(!found.same_file);
        assert!(!folder.journals().find(&file(), None).unwrap().same_file);
    }

    #[test]
    fn this_runs_journals_are_not_offered_until_released() {
        let folder = Folder::new();
        let journals = folder.journals();
        let id = journals.create().unwrap();
        journals
            .write(&id, &file(), identity(1_000), &edits(), &[], 0)
            .unwrap();
        assert_eq!(journals.find(&file(), identity(1_000)), None);
        journals.release(&id);
        assert_eq!(journals.find(&file(), identity(1_000)).unwrap().id, id);
    }

    #[test]
    fn clearing_deletes_the_journals_not_in_use() {
        let folder = Folder::new();
        let earlier = folder.journals();
        let left = earlier.create().unwrap();
        earlier
            .write(&left, &file(), identity(1_000), &edits(), &[], 0)
            .unwrap();
        let journals = folder.journals();
        let own = journals.create().unwrap();
        journals
            .write(&own, &file(), identity(1_000), &edits(), &[], 0)
            .unwrap();
        fs::write(
            folder
                .0
                .join(FOLDER_NAME)
                .join(format!("{}.json.tmp", left.0)),
            b"{",
        )
        .unwrap();
        fs::write(
            folder.0.join(FOLDER_NAME).join("notes.txt"),
            b"not the app's",
        )
        .unwrap();

        journals.clear_unused();
        assert_eq!(
            folder.files(),
            [format!("{}.json", own.0), "notes.txt".to_owned()]
        );
    }

    #[test]
    fn a_journal_is_never_larger_than_the_limit() {
        let folder = Folder::new();
        let journals = folder.journals();
        let id = journals.create().unwrap();
        let all: Vec<u32> = (0..99_999).collect();
        let large = vec![Edit::DeletePages { pages: all }; 10];
        assert_eq!(
            Journals::check(&file(), identity(1_000), &large, &[], 0),
            Err(TooLarge)
        );
        assert_eq!(
            journals.write(&id, &file(), identity(1_000), &large, &[], 0),
            Err(TooLarge)
        );
        assert!(folder.files().is_empty());
        assert_eq!(
            Journals::check(&file(), identity(1_000), &edits(), &[], 0),
            Ok(())
        );
        // Without the file's identity there is nothing to check a later file against.
        journals
            .write(&id, &file(), None, &edits(), &[], 0)
            .unwrap();
        assert!(folder.files().is_empty());
    }

    #[test]
    fn journals_are_checked_as_untrusted_data() {
        let folder = Folder::new();
        let dir = folder.0.join(FOLDER_NAME);
        fs::create_dir_all(&dir).unwrap();
        let path = serde_json::to_string(&file().to_str().unwrap()).unwrap();
        let journal = |edits: &str, extra: &str| {
            format!(
                r#"{{"version":1,"path":{path},"len":1000,"modifiedSecs":1790000000,"modifiedNanos":123,"edits":{edits}{extra}}}"#
            )
        };
        let bad = [
            // Not JSON, another version, unknown fields.
            "{".to_owned(),
            journal(r#"[{"kind":"deletePages","pages":[2]}]"#, "")
                .replace("\"version\":1", "\"version\":2"),
            journal(r#"[{"kind":"deletePages","pages":[2]}]"#, r#","more":1"#),
            // No edits, an edit that is not valid (a page twice), too many edits.
            journal("[]", ""),
            journal(r#"[{"kind":"deletePages","pages":[2,2]}]"#, ""),
            journal(
                &format!(
                    "[{}]",
                    vec![r#"{"kind":"deletePages","pages":[2]}"#; MAX_UNDO_EDITS as usize + 1]
                        .join(",")
                ),
                "",
            ),
            // A relative path.
            journal(r#"[{"kind":"deletePages","pages":[2]}]"#, "").replace(&path, r#""報告.pdf""#),
        ];
        for (index, content) in bad.iter().enumerate() {
            let name = format!("{index:032x}.json");
            fs::write(dir.join(&name), content).unwrap();
            assert_eq!(
                folder
                    .journals()
                    .find(Path::new("報告.pdf"), identity(1_000)),
                None,
                "{content}"
            );
            assert_eq!(
                folder.journals().find(&file(), identity(1_000)),
                None,
                "{content}"
            );
            fs::remove_file(dir.join(name)).unwrap();
        }
        // Too large, even if it were valid.
        let mut huge = journal(r#"[{"kind":"deletePages","pages":[2]}]"#, "");
        huge.insert_str(1, &" ".repeat(MAX_JOURNAL_BYTES));
        fs::write(dir.join(format!("{:032x}.json", 0)), &huge).unwrap();
        assert_eq!(folder.journals().find(&file(), identity(1_000)), None);
        // A time the system cannot represent: no file is like that.
        let far = journal(r#"[{"kind":"deletePages","pages":[2]}]"#, "")
            .replace("1790000000", &u64::MAX.to_string());
        fs::write(dir.join(format!("{:032x}.json", 0)), far).unwrap();
        assert!(
            !folder
                .journals()
                .find(&file(), identity(1_000))
                .unwrap()
                .same_file
        );
        // And the same journal, the right size, is found.
        fs::write(
            dir.join(format!("{:032x}.json", 0)),
            journal(r#"[{"kind":"deletePages","pages":[2]}]"#, ""),
        )
        .unwrap();
        assert!(
            folder
                .journals()
                .find(&file(), identity(1_000))
                .unwrap()
                .same_file
        );
    }

    #[test]
    fn without_a_data_folder_nothing_is_kept() {
        let journals = Journals::new(None);
        assert_eq!(journals.create(), None);
        assert_eq!(journals.find(&file(), identity(1_000)), None);
        journals.clear_unused();
    }

    /// A PNG file as far as its header goes, which is all a journal's picture is checked for here
    /// (the worker decodes it when an edit puts it on a page).
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png
    }

    fn picture_stamp(image: u32) -> Edit {
        Edit::AddImageStamp {
            page: 0,
            rect: ipc_contract::types::Rect {
                x0: 100.0,
                y0: 100.0,
                x1: 228.0,
                y1: 164.0,
            },
            image: StampImageId(image),
        }
    }

    #[test]
    fn hex_text_is_lowercase_and_strict() {
        assert_eq!(hex(&[0, 15, 255, 0x4a]), "000fff4a");
        assert_eq!(unhex("000fff4a"), Some(vec![0, 15, 255, 0x4a]));
        assert_eq!(unhex(""), Some(Vec::new()));
        for bad in ["0", "000FFF", "0g", "00 0f", "é0"] {
            assert_eq!(unhex(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_pictures_of_picture_stamps_are_kept_as_hex_text_and_come_back() {
        let folder = Folder::new();
        let journals = folder.journals();
        let id = journals.create().unwrap();
        let picture = png(64, 32);
        let edits = vec![picture_stamp(3), Edit::DeletePages { pages: vec![2] }];
        journals
            .write(
                &id,
                &file(),
                identity(1_000),
                &edits,
                &[(StampImageId(3), &picture)],
                0,
            )
            .unwrap();
        let text =
            fs::read_to_string(folder.0.join(FOLDER_NAME).join(format!("{}.json", id.0))).unwrap();
        assert!(text.contains(&hex(&picture)));
        journals.release(&id);
        let found = folder.journals().find(&file(), identity(1_000)).unwrap();
        assert_eq!(found.edits, edits);
        assert_eq!(found.pictures, [(StampImageId(3), picture)]);
    }

    #[test]
    fn a_journal_with_a_picture_it_cannot_have_is_not_the_apps() {
        let folder = Folder::new();
        let dir = folder.0.join(FOLDER_NAME);
        fs::create_dir_all(&dir).unwrap();
        let path = serde_json::to_string(&file().to_str().unwrap()).unwrap();
        let journal = |pictures: &str| {
            format!(
                r#"{{"version":1,"path":{path},"len":1000,"modifiedSecs":1790000000,"modifiedNanos":123,"edits":[{{"kind":"addImageStamp","page":0,"rect":{{"x0":100,"y0":100,"x1":228,"y1":164}},"image":3}}],"pictures":{pictures}}}"#
            )
        };
        let picture = |id: u32, png: &[u8]| format!(r#"{{"id":{id},"png":"{}"}}"#, hex(png));
        let good = journal(&format!("[{}]", picture(3, &png(64, 32))));
        let many: Vec<String> = (3..3 + crate::pictures::MAX_PICTURES as u32 + 1)
            .map(|id| picture(id, &png(8, 8)))
            .collect();
        let bad = [
            // The edit names a picture that is not there, or there is none.
            journal("[]"),
            journal(&format!("[{}]", picture(4, &png(64, 32)))),
            // Not hex in the app's way; not a PNG file; a size no stamp has; a number twice; and
            // more pictures than a document keeps.
            journal(r#"[{"id":3,"png":"89504E47"}]"#),
            journal(r#"[{"id":3,"png":"0"}]"#),
            journal(&format!("[{}]", picture(3, b"not a picture at all"))),
            journal(&format!("[{}]", picture(3, &png(0, 32)))),
            journal(&format!("[{}]", picture(3, &png(64, 5_000)))),
            journal(&format!(
                "[{},{}]",
                picture(3, &png(64, 32)),
                picture(3, &png(8, 8))
            )),
            journal(&format!("[{}]", many.join(","))),
            journal(r#"[{"id":3,"png":"","more":1}]"#),
        ];
        for (index, content) in bad.iter().enumerate() {
            let name = format!("{index:032x}.json");
            fs::write(dir.join(&name), content).unwrap();
            assert_eq!(
                folder.journals().find(&file(), identity(1_000)),
                None,
                "{content}"
            );
            fs::remove_file(dir.join(name)).unwrap();
        }
        fs::write(dir.join(format!("{:032x}.json", 0)), good).unwrap();
        let found = folder.journals().find(&file(), identity(1_000)).unwrap();
        assert_eq!(found.pictures, [(StampImageId(3), png(64, 32))]);
    }

    #[test]
    fn one_picture_fits_a_journal_and_two_of_the_largest_do_not() {
        let mut large = png(64, 32);
        large.resize(ipc_contract::limits::MAX_STAMP_PNG_BYTES, 0);
        let edits = vec![picture_stamp(3), picture_stamp(4)];
        let one = [(StampImageId(3), &large[..])];
        let two = [(StampImageId(3), &large[..]), (StampImageId(4), &large[..])];
        assert_eq!(
            Journals::check(&file(), identity(1_000), &edits, &one, 0),
            Ok(())
        );
        assert_eq!(
            Journals::check(&file(), identity(1_000), &edits, &two, 0),
            Err(TooLarge)
        );
    }

    #[test]
    fn what_a_journal_could_not_keep_is_counted() {
        let folder = Folder::new();
        let journals = folder.journals();
        let id = journals.create().unwrap();
        // Two edits kept, three left out.
        journals
            .write(&id, &file(), identity(1_000), &edits(), &[], 3)
            .unwrap();
        let path = folder.0.join(FOLDER_NAME).join(format!("{}.json", id.0));
        assert!(fs::read_to_string(&path).unwrap().contains(r#""lost":3"#));
        journals.release(&id);
        let found = folder.journals().find(&file(), identity(1_000)).unwrap();
        assert_eq!((found.edits.len(), found.lost), (2, 3));
        // None kept at all is a journal too, if something was left out; and none of either is not.
        let other = Folder::new();
        let journals = other.journals();
        let id = journals.create().unwrap();
        journals
            .write(&id, &file(), identity(1_000), &[], &[], 1)
            .unwrap();
        journals.release(&id);
        let found = other.journals().find(&file(), identity(1_000)).unwrap();
        assert_eq!((found.edits.len(), found.lost), (0, 1));
        let path = other.0.join(FOLDER_NAME).join(format!("{}.json", id.0));
        let empty = fs::read_to_string(&path)
            .unwrap()
            .replace(r#","lost":1"#, "");
        fs::write(&path, empty).unwrap();
        assert_eq!(other.journals().find(&file(), identity(1_000)), None);
    }

    #[test]
    fn a_journal_that_has_the_pages_of_another_file_or_too_many_left_out_is_not_the_apps() {
        let folder = Folder::new();
        let dir = folder.0.join(FOLDER_NAME);
        fs::create_dir_all(&dir).unwrap();
        let path = serde_json::to_string(&file().to_str().unwrap()).unwrap();
        let journal = |edits: &str, lost: u32| {
            format!(
                r#"{{"version":1,"path":{path},"len":1000,"modifiedSecs":1790000000,"modifiedNanos":123,"edits":{edits},"lost":{lost}}}"#
            )
        };
        let delete = r#"{"kind":"deletePages","pages":[2]}"#;
        let bad = [
            // The pages of another file cannot be in a journal: it cannot keep the file.
            journal(r#"[{"kind":"insertPages","at":1,"source":3}]"#, 0),
            // More left out than a document has edits.
            journal(&format!("[{delete}]"), MAX_UNDO_EDITS),
            journal("[]", MAX_UNDO_EDITS + 1),
        ];
        for (index, content) in bad.iter().enumerate() {
            let name = format!("{index:032x}.json");
            fs::write(dir.join(&name), content).unwrap();
            assert_eq!(
                folder.journals().find(&file(), identity(1_000)),
                None,
                "{content}"
            );
            fs::remove_file(dir.join(name)).unwrap();
        }
        fs::write(
            dir.join(format!("{:032x}.json", 0)),
            journal(&format!("[{delete}]"), 4),
        )
        .unwrap();
        assert_eq!(
            folder
                .journals()
                .find(&file(), identity(1_000))
                .map(|found| found.lost),
            Some(4)
        );
    }
}
