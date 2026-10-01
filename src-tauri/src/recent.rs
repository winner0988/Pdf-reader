//! The recently opened files (#73, docs/architecture/recent-files.md). Full paths are kept only
//! here and in `recent.json` in the app's local (not roaming) data folder; the frontend gets file
//! names and ids that are valid while the app runs.
//!
//! Files the user asked not to record are kept as salted SHA-256 hashes of their paths, so the
//! file on disk does not say which files they are.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use ipc_contract::limits::MAX_RECENT_FILES;
use ipc_contract::types::{RecentFile, RecentId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::documents::display_name;
use crate::local_data;

/// The file in the data folder.
pub const FILE_NAME: &str = "recent.json";
/// Files the user asked not to record, at most; the oldest choices are dropped beyond.
const MAX_EXCLUDED: usize = 1_000;
/// Longer paths are not recorded.
const MAX_PATH_BYTES: usize = 4_096;
/// A larger file was not written by the app: it is ignored, and the list starts empty.
const MAX_FILE_BYTES: u64 = 512 * 1024;
const VERSION: u32 = 1;
const SALT_BYTES: usize = 16;

/// `recent.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    /// Most recent first.
    files: Vec<String>,
    /// Hex; empty until a file is excluded.
    #[serde(default)]
    salt: String,
    /// Hex SHA-256 of `salt` followed by the path's key (see `key`), oldest first.
    #[serde(default)]
    excluded: Vec<String>,
}

struct Entry {
    id: RecentId,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    /// Most recent first.
    entries: Vec<Entry>,
    salt: Vec<u8>,
    excluded: Vec<String>,
    next_id: u32,
}

impl State {
    fn add_front(&mut self, path: PathBuf) {
        let same = key(&path);
        self.entries.retain(|entry| key(&entry.path) != same);
        self.next_id += 1;
        self.entries.insert(
            0,
            Entry {
                id: RecentId(self.next_id),
                path,
            },
        );
        self.entries.truncate(MAX_RECENT_FILES as usize);
    }

    fn hash(&self, path: &Path) -> Option<String> {
        if self.salt.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        hasher.update(&self.salt);
        hasher.update(key(path).as_bytes());
        Some(hex(&hasher.finalize()))
    }

    fn is_excluded(&self, path: &Path) -> bool {
        self.hash(path)
            .is_some_and(|hash| self.excluded.contains(&hash))
    }
}

/// The error of an action on a file the list does not have (any more).
#[derive(Debug, PartialEq, Eq)]
pub struct UnknownEntry;

pub struct RecentFiles {
    /// `recent.json`, or `None` when there is no data folder: then nothing is recorded.
    file: Option<PathBuf>,
    /// Loaded on first use.
    state: Mutex<Option<State>>,
}

impl RecentFiles {
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file,
            state: Mutex::new(None),
        }
    }

    /// The list, most recent first: file names and ids only.
    pub fn list(&self) -> Vec<RecentFile> {
        self.with(|state| list_of(state))
    }

    /// Where a listed file is, to open it.
    pub fn path(&self, id: RecentId) -> Option<PathBuf> {
        self.with(|state| {
            state
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.path.clone())
        })
    }

    /// Puts a file that just opened first on the list, unless the user asked not to record it.
    /// Files without an absolute path of at most `MAX_PATH_BYTES` in UTF-8 are not recorded.
    pub fn record(&self, path: &Path) {
        if !recordable(path) {
            return;
        }
        self.with(|state| {
            if !state.is_excluded(path) {
                state.add_front(path.to_owned());
                self.save(state);
            }
        });
    }

    /// Takes a file off the list; it is recorded again the next time it opens.
    pub fn remove(&self, id: RecentId) -> Result<Vec<RecentFile>, UnknownEntry> {
        self.with(|state| {
            let index = state
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or(UnknownEntry)?;
            state.entries.remove(index);
            self.save(state);
            Ok(list_of(state))
        })
    }

    /// Empties the list. The files the user asked not to record stay unrecorded.
    pub fn clear(&self) {
        self.with(|state| {
            state.entries.clear();
            self.save(state);
        });
    }

    /// Forgets which files the user asked not to record (the settings, B2-12): they are recorded
    /// again the next time they open.
    pub fn forget_exclusions(&self) {
        self.with(|state| {
            state.excluded.clear();
            state.salt.clear();
            self.save(state);
        });
    }

    /// Whether `path` may be on the list (the user did not ask not to record it).
    pub fn is_recorded(&self, path: &Path) -> bool {
        self.with(|state| !state.is_excluded(path))
    }

    /// Whether `path`, a file that is open, may be on the list. Not recording it also takes it off
    /// the list; recording it again puts it first, as it is open, if `list` (the settings may say
    /// not to record any file, B2-12).
    pub fn set_recorded(
        &self,
        path: &Path,
        record: bool,
        list: bool,
    ) -> Result<(), getrandom::Error> {
        self.with(|state| {
            if record {
                if let Some(hash) = state.hash(path) {
                    state.excluded.retain(|excluded| *excluded != hash);
                }
                if list && recordable(path) {
                    state.add_front(path.to_owned());
                }
            } else {
                if state.salt.is_empty() {
                    let mut salt = [0; SALT_BYTES];
                    getrandom::fill(&mut salt)?;
                    state.salt = salt.to_vec();
                }
                let hash = state.hash(path).expect("the salt is set");
                if !state.excluded.contains(&hash) {
                    state.excluded.push(hash);
                }
                let excess = state.excluded.len().saturating_sub(MAX_EXCLUDED);
                state.excluded.drain(..excess);
                let same = key(path);
                state.entries.retain(|entry| key(&entry.path) != same);
            }
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

    /// Writes the list, or deletes the file when there is nothing to keep. Best effort: the list
    /// in memory stays right for this run either way.
    fn save(&self, state: &State) {
        let Some(file) = &self.file else {
            return;
        };
        if state.entries.is_empty() && state.excluded.is_empty() {
            local_data::remove(file);
            return;
        }
        let stored = Stored {
            version: VERSION,
            files: state
                .entries
                .iter()
                .filter_map(|entry| entry.path.to_str().map(str::to_owned))
                .collect(),
            salt: hex(&state.salt),
            excluded: state.excluded.clone(),
        };
        if let Ok(json) = serde_json::to_vec_pretty(&stored) {
            let _ = local_data::write(file, &json);
        }
    }
}

fn list_of(state: &State) -> Vec<RecentFile> {
    state
        .entries
        .iter()
        .map(|entry| RecentFile {
            id: entry.id,
            display_name: display_name(&entry.path),
        })
        .collect()
}

/// What the list keeps: absolute paths of reasonable length that are valid Unicode.
fn recordable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|text| text.len() <= MAX_PATH_BYTES)
}

/// Windows file names ignore case, so one file is one entry however its path is spelled.
pub fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Reads `recent.json`. Anything unexpected (too large, not the app's format) counts as no list;
/// the next change overwrites the file.
fn load(file: &Path) -> State {
    let stored = local_data::read(file, MAX_FILE_BYTES)
        .and_then(|json| serde_json::from_slice::<Stored>(&json).ok());
    let Some(stored) = stored.filter(|stored| stored.version == VERSION) else {
        return State::default();
    };

    let mut state = State {
        salt: unhex(&stored.salt)
            .filter(|salt| salt.len() == SALT_BYTES)
            .unwrap_or_default(),
        ..State::default()
    };
    if !state.salt.is_empty() {
        state.excluded = stored
            .excluded
            .into_iter()
            .filter(|hash| hash.len() == 64 && unhex(hash).is_some())
            .collect();
        let excess = state.excluded.len().saturating_sub(MAX_EXCLUDED);
        state.excluded.drain(..excess);
    }
    // Oldest first, so that the most recent ends up first.
    for path in stored
        .files
        .into_iter()
        .take(MAX_RECENT_FILES as usize)
        .rev()
    {
        let path = PathBuf::from(path);
        if recordable(&path) && !state.is_excluded(&path) {
            state.add_front(path);
        }
    }
    state
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A fresh data folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pdf-reader-recent-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn pdf(name: &str) -> PathBuf {
        PathBuf::from(format!(r"C:\Users\someone\文件\{name}.pdf"))
    }

    fn names(recent: &RecentFiles) -> Vec<String> {
        recent
            .list()
            .into_iter()
            .map(|file| file.display_name)
            .collect()
    }

    #[test]
    fn keeps_the_latest_twenty_files_most_recent_first() {
        let dir = folder("latest");
        let recent = RecentFiles::new(Some(dir.join(FILE_NAME)));
        for index in 0..25 {
            recent.record(&pdf(&format!("f{index}")));
        }
        // Opening a listed file again moves it first, whatever the case of its path.
        recent.record(&PathBuf::from(r"C:\USERS\someone\文件\F10.PDF"));
        let listed = names(&recent);
        assert_eq!(listed.len(), 20);
        assert_eq!(listed[..3], ["F10.PDF", "f24.pdf", "f23.pdf"]);
        assert!(!listed.contains(&"f10.pdf".to_owned()));
        assert!(!listed.contains(&"f4.pdf".to_owned()));

        // Another run of the app reads the same list, with new ids.
        let again = RecentFiles::new(Some(dir.join(FILE_NAME)));
        assert_eq!(names(&again), listed);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_frontend_gets_names_and_ids_the_paths_stay_here() {
        let dir = folder("names");
        let recent = RecentFiles::new(Some(dir.join(FILE_NAME)));
        recent.record(&pdf("報告"));
        let listed = recent.list();
        assert_eq!(listed[0].display_name, "報告.pdf");
        assert_eq!(recent.path(listed[0].id), Some(pdf("報告")));
        assert_eq!(recent.path(RecentId(999)), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn records_only_absolute_unicode_paths_of_reasonable_length() {
        let recent = RecentFiles::new(None);
        recent.record(Path::new("relative.pdf"));
        recent.record(&PathBuf::from(format!(
            r"C:\{}.pdf",
            "a".repeat(MAX_PATH_BYTES)
        )));
        assert!(recent.list().is_empty());
        // Without a data folder the list still works while the app runs.
        recent.record(&pdf("a"));
        assert_eq!(names(&recent), ["a.pdf"]);
    }

    #[test]
    fn remove_and_clear_leave_nothing_behind() {
        let dir = folder("clear");
        let file = dir.join(FILE_NAME);
        let recent = RecentFiles::new(Some(file.clone()));
        recent.record(&pdf("a"));
        recent.record(&pdf("b"));
        let b = recent.list()[0].id;
        assert_eq!(recent.remove(b).map(|list| list.len()), Ok(1));
        assert_eq!(recent.remove(b), Err(UnknownEntry));
        assert!(fs::read_to_string(&file).unwrap().contains("a.pdf"));

        recent.clear();
        assert!(recent.list().is_empty());
        assert!(!file.exists(), "an empty list leaves no file");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_not_to_record_is_kept_as_a_hash_only() {
        let dir = folder("exclude");
        let file = dir.join(FILE_NAME);
        let recent = RecentFiles::new(Some(file.clone()));
        recent.record(&pdf("secret"));
        recent.record(&pdf("plain"));
        recent.set_recorded(&pdf("secret"), false, true).unwrap();
        assert_eq!(names(&recent), ["plain.pdf"]);
        assert!(!recent.is_recorded(&pdf("SECRET")));
        recent.record(&pdf("secret"));
        assert_eq!(names(&recent), ["plain.pdf"]);

        let json = fs::read_to_string(&file).unwrap();
        assert!(!json.contains("secret"), "{json}");
        // It stays unrecorded in the next run and after clearing the list.
        let again = RecentFiles::new(Some(file.clone()));
        again.clear();
        again.record(&pdf("secret"));
        assert!(again.list().is_empty());
        assert!(!again.is_recorded(&pdf("secret")));
        assert!(file.exists(), "the choice is kept");

        // Recording it again puts it first: it is open.
        again.set_recorded(&pdf("secret"), true, true).unwrap();
        assert_eq!(names(&again), ["secret.pdf"]);
        assert!(again.is_recorded(&pdf("secret")));

        // Not listed when the settings say not to record any file.
        again.set_recorded(&pdf("secret"), false, true).unwrap();
        again.set_recorded(&pdf("secret"), true, false).unwrap();
        assert!(again.list().is_empty());
        assert!(again.is_recorded(&pdf("secret")));

        // Forgetting the choices: recorded again, and no salt or hash is kept.
        again.set_recorded(&pdf("secret"), false, true).unwrap();
        again.forget_exclusions();
        assert!(again.is_recorded(&pdf("secret")));
        assert!(!file.exists(), "nothing left to keep");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_the_app_did_not_write_counts_as_no_list() {
        let dir = folder("foreign");
        let file = dir.join(FILE_NAME);
        fs::create_dir_all(&dir).unwrap();
        for content in [
            "not json".to_owned(),
            r#"{"version": 2, "files": ["C:\\a.pdf"]}"#.to_owned(),
            r#"{"version": 1, "files": ["C:\\a.pdf"], "extra": 1}"#.to_owned(),
            format!(
                r#"{{"version": 1, "files": ["C:\\{}.pdf"]}}"#,
                "a".repeat(600_000)
            ),
        ] {
            fs::write(&file, content).unwrap();
            assert!(RecentFiles::new(Some(file.clone())).list().is_empty());
        }
        // Entries that are not absolute paths are dropped; the others are kept.
        fs::write(
            &file,
            r#"{"version": 1, "files": ["C:\\b.pdf", "relative.pdf", "C:\\a.pdf"]}"#,
        )
        .unwrap();
        assert_eq!(
            names(&RecentFiles::new(Some(file.clone()))),
            ["b.pdf", "a.pdf"]
        );
        let _ = fs::remove_dir_all(dir);
    }
}
