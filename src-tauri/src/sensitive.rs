//! The files the user marked as sensitive on this computer (B3-05, ADR 0004,
//! docs/architecture/sensitive-files.md), for the settings to show. The mark itself is in each
//! file: the app's document identifier in its metadata (ADR 0010). This list keeps only that
//! identifier and the name the file had, never a path, in `sensitive.json` in the app's local
//! (not roaming) data folder. A file opened that was marked elsewhere is not added: only what the
//! user marks here.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use ipc_contract::limits::{MAX_DISPLAY_NAME_BYTES, MAX_SENSITIVE_FILES};
use ipc_contract::types::SensitiveId;
use serde::{Deserialize, Serialize};

use crate::local_data;

/// The file in the data folder.
pub const FILE_NAME: &str = "sensitive.json";
/// A larger file was not written by the app: it is ignored, and the list starts empty.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const VERSION: u32 = 1;

/// `sensitive.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    /// Most recently marked first.
    files: Vec<StoredFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    /// 32 lowercase hexadecimal digits, as in the file's metadata.
    identifier: String,
    name: String,
}

struct Entry {
    id: SensitiveId,
    identifier: [u8; 16],
    name: String,
}

#[derive(Default)]
struct State {
    /// Most recently marked first.
    entries: Vec<Entry>,
    next_id: u32,
}

impl State {
    fn add_front(&mut self, identifier: [u8; 16], name: String) {
        self.entries.retain(|entry| entry.identifier != identifier);
        self.next_id += 1;
        self.entries.insert(
            0,
            Entry {
                id: SensitiveId(self.next_id),
                identifier,
                name,
            },
        );
        self.entries.truncate(MAX_SENSITIVE_FILES as usize);
    }
}

/// The error of an action on a file the list does not have (any more).
#[derive(Debug, PartialEq, Eq)]
pub struct UnknownEntry;

/// A file of the list: the id the frontend gets, the identifier in the file, the name it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: SensitiveId,
    pub identifier: [u8; 16],
    pub name: String,
}

pub struct SensitiveFiles {
    /// `sensitive.json`, or `None` when there is no data folder: then the list lasts one run.
    file: Option<PathBuf>,
    /// Loaded on first use.
    state: Mutex<Option<State>>,
}

impl SensitiveFiles {
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file,
            state: Mutex::new(None),
        }
    }

    /// The list, most recently marked first.
    pub fn list(&self) -> Vec<Listed> {
        self.with(|state| {
            state
                .entries
                .iter()
                .map(|entry| Listed {
                    id: entry.id,
                    identifier: entry.identifier,
                    name: entry.name.clone(),
                })
                .collect()
        })
    }

    /// A file the user just marked: first on the list, as `name`. Beyond `MAX_SENSITIVE_FILES` the
    /// oldest entry leaves the list; its file stays marked.
    pub fn add(&self, identifier: [u8; 16], name: &str) {
        let Some(name) = listable(name) else {
            return;
        };
        self.with(|state| {
            state.add_front(identifier, name);
            self.save(state);
        });
    }

    /// A file whose mark was just taken out: off the list.
    pub fn remove_identifier(&self, identifier: &[u8; 16]) {
        self.with(|state| {
            let before = state.entries.len();
            state
                .entries
                .retain(|entry| entry.identifier != *identifier);
            if state.entries.len() != before {
                self.save(state);
            }
        });
    }

    /// A listed file that just opened, perhaps moved or renamed: listed as `name` from now on.
    pub fn rename(&self, identifier: &[u8; 16], name: &str) {
        let Some(name) = listable(name) else {
            return;
        };
        self.with(|state| {
            let Some(entry) = state
                .entries
                .iter_mut()
                .find(|entry| entry.identifier == *identifier)
            else {
                return;
            };
            if entry.name != name {
                entry.name = name;
                self.save(state);
            }
        });
    }

    /// Takes an entry off the list. Its file stays marked: the mark is in the file.
    pub fn remove(&self, id: SensitiveId) -> Result<(), UnknownEntry> {
        self.with(|state| {
            let index = state
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or(UnknownEntry)?;
            state.entries.remove(index);
            self.save(state);
            Ok(())
        })
    }

    /// Runs `action` on the list, loading it first if this is its first use.
    fn with<T>(&self, action: impl FnOnce(&mut State) -> T) -> T {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let state = guard.get_or_insert_with(|| self.file.as_deref().map(load).unwrap_or_default());
        action(state)
    }

    /// Writes the list, or deletes the file when it is empty. Best effort: the list in memory
    /// stays right for this run either way.
    fn save(&self, state: &State) {
        let Some(file) = &self.file else {
            return;
        };
        if state.entries.is_empty() {
            local_data::remove(file);
            return;
        }
        let stored = Stored {
            version: VERSION,
            files: state
                .entries
                .iter()
                .map(|entry| StoredFile {
                    identifier: hex(&entry.identifier),
                    name: entry.name.clone(),
                })
                .collect(),
        };
        if let Ok(json) = serde_json::to_vec_pretty(&stored) {
            let _ = local_data::write(file, &json);
        }
    }
}

/// A name the list keeps: a bare file name of reasonable length (`documents::display_name` makes
/// them so; one read from the file is checked again).
fn listable(name: &str) -> Option<String> {
    let fits = !name.is_empty()
        && name.len() <= MAX_DISPLAY_NAME_BYTES as usize
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control);
    fits.then(|| name.to_owned())
}

/// Reads `sensitive.json`. Anything unexpected (too large, not the app's format) counts as no
/// list; the next change overwrites the file. Entries that are not right are left out.
fn load(file: &Path) -> State {
    let stored = local_data::read(file, MAX_FILE_BYTES)
        .and_then(|json| serde_json::from_slice::<Stored>(&json).ok());
    let Some(stored) = stored.filter(|stored| stored.version == VERSION) else {
        return State::default();
    };
    let mut state = State::default();
    // Oldest first, so that the most recent ends up first.
    for file in stored
        .files
        .into_iter()
        .take(MAX_SENSITIVE_FILES as usize)
        .rev()
    {
        if let (Some(identifier), Some(name)) = (unhex(&file.identifier), listable(&file.name)) {
            state.add_front(identifier, name);
        }
    }
    state
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A fresh data folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pdf-reader-sensitive-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn names(list: &SensitiveFiles) -> Vec<String> {
        list.list().into_iter().map(|file| file.name).collect()
    }

    const A: [u8; 16] = [0xa; 16];
    const B: [u8; 16] = [0xb; 16];
    const C: [u8; 16] = [0xc; 16];

    #[test]
    fn marks_are_listed_newest_first_by_name_and_kept_for_the_next_run() {
        let dir = folder("kept");
        let list = SensitiveFiles::new(Some(dir.join(FILE_NAME)));
        list.add(A, "a.pdf");
        list.add(B, "合約.pdf");
        list.add(C, "c.pdf");
        // Marked again (a copy of the same file, say): first, once.
        list.add(A, "a copy.pdf");
        assert_eq!(names(&list), ["a copy.pdf", "c.pdf", "合約.pdf"]);

        // The file has names and identifiers, and no path.
        let json = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(
            json.contains("合約.pdf") && json.contains(&hex(&B)),
            "{json}"
        );
        assert!(!json.contains('\\') && !json.contains(":/"), "{json}");

        let again = SensitiveFiles::new(Some(dir.join(FILE_NAME)));
        assert_eq!(names(&again), ["a copy.pdf", "c.pdf", "合約.pdf"]);
        assert_eq!(again.list()[2].identifier, B);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn renamed_unmarked_and_removed_files_change_the_list_and_nothing_else() {
        let dir = folder("changes");
        let list = SensitiveFiles::new(Some(dir.join(FILE_NAME)));
        list.add(A, "a.pdf");
        list.add(B, "b.pdf");
        list.rename(&A, "renamed.pdf");
        // A file this list never had is not added by opening it.
        list.rename(&C, "elsewhere.pdf");
        assert_eq!(names(&list), ["b.pdf", "renamed.pdf"]);
        list.remove_identifier(&B);
        assert_eq!(names(&list), ["renamed.pdf"]);
        let id = list.list()[0].id;
        assert_eq!(list.remove(SensitiveId(999)), Err(UnknownEntry));
        assert_eq!(list.remove(id), Ok(()));
        assert!(list.list().is_empty());
        // Nothing left: no file either.
        assert!(!dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_oldest_entry_leaves_a_full_list() {
        let list = SensitiveFiles::new(None);
        for index in 0..=MAX_SENSITIVE_FILES {
            let mut identifier = [0u8; 16];
            identifier[..4].copy_from_slice(&index.to_be_bytes());
            list.add(identifier, &format!("{index}.pdf"));
        }
        let listed = names(&list);
        assert_eq!(listed.len(), MAX_SENSITIVE_FILES as usize);
        assert_eq!(listed[0], format!("{MAX_SENSITIVE_FILES}.pdf"));
        assert!(!listed.contains(&"0.pdf".to_owned()));
    }

    #[test]
    fn only_names_are_kept_and_a_file_not_written_by_the_app_is_no_list() {
        let list = SensitiveFiles::new(None);
        list.add(A, r"C:\folder\a.pdf");
        list.add(B, "");
        assert!(list.list().is_empty());

        let dir = folder("foreign");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE_NAME);
        for content in [
            "not json".to_owned(),
            r#"{"version": 2, "files": []}"#.to_owned(),
            r#"{"version": 1, "files": [], "extra": 1}"#.to_owned(),
        ] {
            fs::write(&file, content).unwrap();
            assert!(SensitiveFiles::new(Some(file.clone())).list().is_empty());
        }
        // Entries that are not right are left out; the others stay.
        fs::write(
            &file,
            format!(
                r#"{{"version": 1, "files": [
                    {{"identifier": "{}", "name": "good.pdf"}},
                    {{"identifier": "xyz", "name": "bad id.pdf"}},
                    {{"identifier": "{}", "name": "C:\\path\\bad.pdf"}}
                ]}}"#,
                hex(&A),
                hex(&B)
            ),
        )
        .unwrap();
        assert_eq!(names(&SensitiveFiles::new(Some(file))), ["good.pdf"]);
        let _ = fs::remove_dir_all(dir);
    }
}
