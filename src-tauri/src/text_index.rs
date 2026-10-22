//! The text index (B3-21, ADR 0026, docs/architecture/text-index.md): the text of the documents
//! the user opened, kept encrypted on this computer, to answer "which document has these words".
//!
//! Off unless the user turns it on, and only while the recent files list is kept. Only files that
//! opened in the app go in, and of them not: sensitive documents, PDFs that needed a password to
//! open (a remembered one too), files the user asked not to record, files on removable drives,
//! discs or network places, and changes that are not saved. A file is known by its path, its size
//! and its modification time: when one of them changes, its text goes and is taken again.
//!
//! On disk, in `index\` in the app's data folder, which only the main process reads and writes:
//! - `key`: a 256-bit key from the operating system, protected by DPAPI for the current user;
//! - `catalog`: the documents, sealed with the key (AES-256-GCM, see `index_crypto`);
//! - `d<id>`: the text of a document's pages, sealed with the key.
//!
//! A key DPAPI cannot unprotect, or anything that does not open with it, means the index is lost:
//! everything is deleted, the key first, and the index starts again with a new key. Clearing
//! deletes the key first too, so that whatever stays on the disk cannot be read.
//!
//! A thread of its own takes each document's text from its worker, a page at a time, as
//! background work on the render thread (renders come first). It waits while the document's
//! scanned pages are being recognised, so that it keeps their text, and stops while the document
//! has changes that are not saved; what it took stays, and the rest is taken when the document
//! opens again.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

use ipc_contract::limits::{
    MAX_INDEX_BYTES, MAX_INDEX_DOCUMENT_TEXT_BYTES, MAX_INDEX_DOCUMENTS,
    MAX_INDEX_PAGES_PER_DOCUMENT, MAX_INDEX_SNIPPET_CHARS, MAX_SEARCH_HITS,
};
use ipc_contract::matching::{Matcher, fold, normalize_query};
use ipc_contract::text::copy_text_char;
use ipc_contract::types::{
    DocumentId, ErrorCode, IndexEntryId, IndexPageHits, IndexSearchArgs, IndexSearchEvent,
    IndexSnippet, IpcError, TabId, TextIndexStatus,
};
use ipc_contract::validate::Validate;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::drives;
use crate::index_crypto::{self, Cipher, CryptoError};
use crate::local_data;
use crate::saving::FileIdentity;

/// The index's folder in the data folder.
pub const FOLDER_NAME: &str = "index";
const KEY_FILE: &str = "key";
const CATALOG_FILE: &str = "catalog";
/// Larger files were not written by the app: the index is lost.
const MAX_KEY_FILE_BYTES: u64 = 4 * 1024;
const MAX_CATALOG_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// A document's file: its text, the framing of its pages, and what sealing adds.
const MAX_DOCUMENT_FILE_BYTES: u64 = MAX_INDEX_DOCUMENT_TEXT_BYTES as u64 * 2;
/// Files with longer paths are not kept.
const MAX_PATH_BYTES: usize = 4 * 1024;
const VERSION: u32 = 1;

/// After a document opens, how long the index leaves it to the user, and to the recognising of its
/// scanned pages, before it takes its text.
const SETTLE: Duration = Duration::from_secs(2);
/// While a document's scanned pages are being recognised, when to look again.
const OCR_WAIT: Duration = Duration::from_secs(2);
/// Between two pages: the worker answers the user's requests meanwhile.
const PAUSE: Duration = Duration::from_millis(5);
/// How often the text taken so far is written while a document is being taken.
const WRITE_EVERY: Duration = Duration::from_secs(10);
/// How many threads search the documents at once, at most.
const MAX_SEARCH_THREADS: usize = 4;

/// What the index needs to know of an open document.
#[derive(Debug, Clone)]
pub struct IndexFacts {
    /// The document as the page knows it: changes with every edit.
    pub doc: DocumentId,
    pub path: PathBuf,
    pub display_name: String,
    pub sensitive: bool,
    /// It needed a password to open, typed or remembered.
    pub protected: bool,
    pub unsaved: bool,
    pub pages: u32,
    /// The file as it was when read or last written.
    pub identity: Option<FileIdentity>,
}

/// What the index needs of the app; a trait so that it can be tested without one.
pub trait Source {
    /// The user has the index on, and the recent files list too.
    fn enabled(&self) -> bool;
    /// `path` may be recorded: the user did not ask for it not to be ("不記錄此檔案").
    fn recorded(&self, path: &Path) -> bool;
    fn tabs(&self) -> Vec<TabId>;
    /// `None` when the tab has no open document.
    fn facts(&self, tab: TabId) -> Option<IndexFacts>;
    /// The text of the tab's scanned pages is being recognised: the index waits for it.
    fn ocr_busy(&self, tab: TabId) -> bool;
    /// The text of a page of `doc`, its lines joined by line breaks, asked for as background work.
    fn page_text(&self, doc: DocumentId, page_index: u32) -> Result<String, IpcError>;
}

/// The catalog: the documents in the index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    version: u32,
    next_id: u64,
    /// Counts the documents opening: what the one opened longest ago is.
    clock: u64,
    entries: Vec<Entry>,
}

/// A document in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    /// Names its file; never used twice.
    id: u64,
    /// The path the file was opened by, to open it again.
    path: String,
    /// The same file's canonical path, to know it by whatever path it opens.
    key: String,
    name: String,
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
    /// When it last opened, on the catalog's clock.
    opened: u64,
    pages: u32,
    /// How many of its first pages are in its file.
    done: u32,
    /// How many of those had no text: they are looked at again when it next opens (their text may
    /// have been recognised by then).
    blank: u32,
    /// Its text reached `MAX_INDEX_DOCUMENT_TEXT_BYTES`: the pages after that are not kept.
    partial: bool,
    /// Its file's size on disk.
    bytes: u64,
}

impl Entry {
    fn same_file(&self, identity: (u64, Duration)) -> bool {
        (self.size, self.modified_secs, self.modified_nanos)
            == (identity.0, identity.1.as_secs(), identity.1.subsec_nanos())
    }
}

/// The index as it is read from disk.
struct Loaded {
    key: Zeroizing<Vec<u8>>,
    cipher: Cipher,
    catalog: Catalog,
    catalog_bytes: u64,
}

#[derive(Default)]
struct State {
    /// The index, once it was read or made.
    loaded: Option<Loaded>,
    /// The disk was looked at: what is on it is in `loaded`, or there is no index yet.
    looked: bool,
    /// How many times the index was deleted: work begun before is not kept.
    generation: u64,
    /// The documents of the last search's results, by the number the page got for each.
    listed: HashMap<u32, u64>,
    next_number: u32,
    /// The tabs whose documents' text is to be taken, and from when.
    queue: VecDeque<(TabId, Instant)>,
    /// Every open document is to be looked at again: the index was lost and starts again.
    reconsider: bool,
}

/// The text index.
pub struct TextIndex {
    folder: Option<PathBuf>,
    state: Mutex<State>,
    wake: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn document_file(id: u64) -> String {
    format!("d{id:016x}")
}

/// What a document's text is sealed with besides the key, so that one document's file does not
/// open as another's.
fn document_data(id: u64) -> Vec<u8> {
    let mut data = b"Pdf-reader text index document ".to_vec();
    data.extend_from_slice(&id.to_le_bytes());
    data
}

const CATALOG_DATA: &[u8] = b"Pdf-reader text index catalog";

/// The pages of a document's text: how many, then each one's length and its UTF-8.
fn encode_pages(pages: &[String]) -> Zeroizing<Vec<u8>> {
    let size = 4 + pages.iter().map(|page| 4 + page.len()).sum::<usize>();
    let mut out = Zeroizing::new(Vec::with_capacity(size));
    out.extend_from_slice(&(pages.len() as u32).to_le_bytes());
    for page in pages {
        out.extend_from_slice(&(page.len() as u32).to_le_bytes());
        out.extend_from_slice(page.as_bytes());
    }
    out
}

/// The pages `encode_pages` wrote, borrowed from `bytes`; `None` if it did not write them.
fn decode_pages(bytes: &[u8]) -> Option<Vec<&str>> {
    let mut rest = bytes;
    let mut take = |length: usize| -> Option<&[u8]> {
        let (head, tail) = rest.split_at_checked(length)?;
        rest = tail;
        Some(head)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut pages = Vec::with_capacity(count.min(bytes.len() / 4));
    for _ in 0..count {
        let length = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        pages.push(std::str::from_utf8(take(length)?).ok()?);
    }
    rest.is_empty().then_some(pages)
}

/// The canonical path of `path`, as the index keeps it: `None` if it cannot be had or is too
/// long, or is not Unicode.
fn canonical(path: &Path) -> Option<String> {
    let found = fs::canonicalize(path).ok()?;
    let text = found.to_str()?;
    (text.len() <= MAX_PATH_BYTES).then(|| text.to_owned())
}

/// The file's size and modification time now (as `FileIdentity::parts`), if it is a file on a
/// fixed drive.
fn identity_now(path: &Path, folders: &mut FixedFolders) -> Option<(u64, Duration)> {
    if !folders.is_fixed(path) {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((metadata.len(), modified))
}

/// What `drives::is_fixed` said of the folders of the files checked in one search. Asking Windows
/// takes about a millisecond (it looks for mount points along the path), and the files of the
/// index are in a few folders: a full index has 2,000.
#[derive(Default)]
struct FixedFolders(HashMap<PathBuf, bool>);

impl FixedFolders {
    /// Whether `path` is on a fixed drive: as its folder is, since a file is on its folder's
    /// volume (only a folder can be a mount point).
    fn is_fixed(&mut self, path: &Path) -> bool {
        match path
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
        {
            Some(folder) => *self
                .0
                .entry(folder.to_owned())
                .or_insert_with(|| drives::is_fixed(path)),
            None => drives::is_fixed(path),
        }
    }
}

impl TextIndex {
    /// The index in `folder`, read from it when it is first needed; without a folder nothing is
    /// kept.
    pub fn new(folder: Option<PathBuf>) -> Self {
        Self {
            folder,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        }
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        self.folder.as_ref().map(|folder| folder.join(name))
    }

    /// Deletes the index: the key first, so that whatever stays on the disk cannot be read, then
    /// everything else. The next document that goes in makes a new key.
    pub fn clear(&self) {
        let mut state = lock(&self.state);
        self.wipe(&mut state);
    }

    fn wipe(&self, state: &mut State) {
        state.loaded = None;
        state.looked = true;
        state.generation += 1;
        state.listed.clear();
        state.queue.clear();
        let Some(folder) = &self.folder else {
            return;
        };
        local_data::remove(&folder.join(KEY_FILE));
        if let Ok(entries) = fs::read_dir(folder) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(folder);
    }

    /// The index, read from disk if it was not yet; with `create`, a new one when there is none.
    fn loaded<'a>(&self, state: &'a mut State, create: bool) -> Option<&'a mut Loaded> {
        if !state.looked {
            state.looked = true;
            match self.read() {
                Ok(found) => state.loaded = found,
                Err(Lost) => self.wipe(state),
            }
        }
        if state.loaded.is_none() && create {
            state.loaded = self.make();
        }
        state.loaded.as_mut()
    }

    /// The index on disk: `None` if there is none, `Lost` if what is there cannot be read.
    fn read(&self) -> Result<Option<Loaded>, Lost> {
        let Some(folder) = &self.folder else {
            return Ok(None);
        };
        let Some(protected) = local_data::read(&folder.join(KEY_FILE), MAX_KEY_FILE_BYTES) else {
            // Nothing without the key can be read.
            return if folder.exists() { Err(Lost) } else { Ok(None) };
        };
        let key = index_crypto::unprotect(&protected).map_err(|_| Lost)?;
        let cipher = Cipher::new(&key).map_err(|_| Lost)?;
        let file = folder.join(CATALOG_FILE);
        let (catalog, catalog_bytes) = match local_data::read(&file, MAX_CATALOG_FILE_BYTES) {
            None if !file.exists() => (
                Catalog {
                    version: VERSION,
                    ..Catalog::default()
                },
                0,
            ),
            None => return Err(Lost),
            Some(sealed) => {
                let plain = cipher.open(CATALOG_DATA, &sealed).map_err(|_| Lost)?;
                let catalog: Catalog = serde_json::from_slice(&plain).map_err(|_| Lost)?;
                if catalog.version != VERSION {
                    return Err(Lost);
                }
                (catalog, sealed.len() as u64)
            }
        };
        // A document's file the catalog does not name was left by a run that stopped between
        // writing it and the catalog.
        if let Ok(entries) = fs::read_dir(folder) {
            for found in entries.flatten() {
                let name = found.file_name();
                let named = catalog
                    .entries
                    .iter()
                    .any(|entry| name.to_str() == Some(&document_file(entry.id)));
                if name != KEY_FILE && name != CATALOG_FILE && !named {
                    let _ = fs::remove_file(found.path());
                }
            }
        }
        Ok(Some(Loaded {
            key,
            cipher,
            catalog,
            catalog_bytes,
        }))
    }

    /// A new, empty index with a new key, written to disk.
    fn make(&self) -> Option<Loaded> {
        let folder = self.folder.as_ref()?;
        let key = index_crypto::new_key().ok()?;
        let protected = index_crypto::protect(&key).ok()?;
        let cipher = Cipher::new(&key).ok()?;
        local_data::write(&folder.join(KEY_FILE), &protected).ok()?;
        let mut loaded = Loaded {
            key,
            cipher,
            catalog: Catalog {
                version: VERSION,
                ..Catalog::default()
            },
            catalog_bytes: 0,
        };
        self.write_catalog(&mut loaded).ok()?;
        Some(loaded)
    }

    fn write_catalog(&self, loaded: &mut Loaded) -> io::Result<()> {
        let Some(file) = self.file(CATALOG_FILE) else {
            return Ok(());
        };
        let plain = Zeroizing::new(serde_json::to_vec(&loaded.catalog).map_err(io::Error::other)?);
        let sealed = loaded
            .cipher
            .seal(CATALOG_DATA, &plain)
            .map_err(|_| io::Error::other("cannot seal the catalog"))?;
        local_data::write(&file, &sealed)?;
        loaded.catalog_bytes = sealed.len() as u64;
        Ok(())
    }

    fn remove_entries(&self, loaded: &mut Loaded, gone: impl Fn(&Entry) -> bool) -> bool {
        let before = loaded.catalog.entries.len();
        loaded.catalog.entries.retain(|entry| {
            if gone(entry) {
                if let Some(file) = self.file(&document_file(entry.id)) {
                    local_data::remove(&file);
                }
                false
            } else {
                true
            }
        });
        loaded.catalog.entries.len() != before
    }

    /// Takes the file at `path` out of the index at once, if it is there.
    pub fn forget(&self, path: &Path) {
        // Only files on fixed drives go in: a network place is not asked for its canonical path.
        let key = drives::is_fixed(path).then(|| canonical(path)).flatten();
        let opened_by = path.to_str().map(str::to_owned);
        let mut state = lock(&self.state);
        let Some(loaded) = self.loaded(&mut state, false) else {
            return;
        };
        let removed = self.remove_entries(loaded, |entry| {
            Some(&entry.key) == key.as_ref() || Some(&entry.path) == opened_by.as_ref()
        });
        if removed {
            let _ = self.write_catalog(loaded);
        }
    }

    /// A tab's document opened, was saved, or was marked as sensitive or no longer: what is in
    /// the index of its file follows at once, and its text is taken soon if it may be.
    pub fn consider(&self, source: &dyn Source, tab: TabId) {
        let Some(facts) = source.facts(tab) else {
            return;
        };
        if facts.sensitive || facts.protected {
            self.forget(&facts.path);
            return;
        }
        if !source.enabled() {
            return;
        }
        if !source.recorded(&facts.path) {
            self.forget(&facts.path);
            return;
        }
        if facts.unsaved || facts.pages == 0 || !drives::is_fixed(&facts.path) {
            return;
        }
        let (Some(identity), Some(key), Some(path)) = (
            facts.identity.and_then(FileIdentity::parts),
            canonical(&facts.path),
            facts
                .path
                .to_str()
                .filter(|path| path.len() <= MAX_PATH_BYTES),
        ) else {
            return;
        };
        let mut state = lock(&self.state);
        let Some(loaded) = self.loaded(&mut state, true) else {
            return;
        };
        // Another size or time: another file, whose text is taken again.
        self.remove_entries(loaded, |entry| {
            entry.key == key && !entry.same_file(identity)
        });
        let catalog = &mut loaded.catalog;
        catalog.clock += 1;
        let opened = catalog.clock;
        match catalog.entries.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => {
                entry.opened = opened;
                entry.path = path.to_owned();
                entry.name.clone_from(&facts.display_name);
            }
            None => {
                let id = catalog.next_id;
                catalog.next_id += 1;
                catalog.entries.push(Entry {
                    id,
                    path: path.to_owned(),
                    key,
                    name: facts.display_name.clone(),
                    size: identity.0,
                    modified_secs: identity.1.as_secs(),
                    modified_nanos: identity.1.subsec_nanos(),
                    opened,
                    pages: facts.pages,
                    done: 0,
                    blank: 0,
                    partial: false,
                    bytes: 0,
                });
            }
        }
        self.keep_within_limits(loaded, None);
        let _ = self.write_catalog(loaded);
        state.queue.retain(|(queued, _)| *queued != tab);
        state.queue.push_back((tab, Instant::now() + SETTLE));
        self.wake.notify_all();
    }

    /// Every open document, as if it had just opened: when the index is turned on.
    pub fn consider_all(&self, source: &dyn Source) {
        for tab in source.tabs() {
            self.consider(source, tab);
        }
    }

    /// Takes out the documents opened longest ago until the index is within its limits; never
    /// `keep`.
    fn keep_within_limits(&self, loaded: &mut Loaded, keep: Option<u64>) {
        let gone = over_limits(
            &loaded.catalog.entries,
            keep,
            MAX_INDEX_DOCUMENTS as usize,
            MAX_INDEX_BYTES.saturating_sub(loaded.catalog_bytes),
        );
        if !gone.is_empty() {
            self.remove_entries(loaded, |entry| gone.contains(&entry.id));
        }
    }

    /// What the index keeps on this computer.
    pub fn status(&self) -> TextIndexStatus {
        let mut state = lock(&self.state);
        let key_bytes = self
            .file(KEY_FILE)
            .and_then(|file| fs::metadata(file).ok())
            .map_or(0, |metadata| metadata.len());
        match self.loaded(&mut state, false) {
            None => TextIndexStatus {
                documents: 0,
                bytes: 0,
            },
            Some(loaded) => TextIndexStatus {
                documents: loaded.catalog.entries.len() as u32,
                bytes: loaded
                    .catalog
                    .entries
                    .iter()
                    .map(|entry| entry.bytes)
                    .sum::<u64>()
                    + loaded.catalog_bytes
                    + key_bytes,
            },
        }
    }

    /// Starts the thread that takes the text of the documents that `consider` queued.
    pub fn start(self: Arc<Self>, source: impl Source + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name("text-index".to_owned())
            .spawn(move || {
                loop {
                    let tab = self.next_tab(&source);
                    if let Some(again) = self.take_text(&source, tab) {
                        let mut state = lock(&self.state);
                        if !state.queue.iter().any(|(queued, _)| *queued == tab) {
                            state.queue.push_back((tab, again));
                        }
                    }
                }
            });
        // Without the thread nothing is taken: the index stays as it is.
        drop(spawned);
    }

    /// The next tab whose text is due, waiting for one.
    fn next_tab(&self, source: &dyn Source) -> TabId {
        let mut state = lock(&self.state);
        loop {
            if state.reconsider {
                state.reconsider = false;
                drop(state);
                self.consider_all(source);
                state = lock(&self.state);
                continue;
            }
            let now = Instant::now();
            if let Some(at) = state.queue.iter().position(|(_, due)| *due <= now)
                && let Some((tab, _)) = state.queue.remove(at)
            {
                return tab;
            }
            let wait = state
                .queue
                .iter()
                .map(|(_, due)| due.saturating_duration_since(now))
                .min()
                .unwrap_or(Duration::from_secs(3600));
            state = self
                .wake
                .wait_timeout(state, wait)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    /// Whether the text of the document `facts` describes may go in now.
    fn may_take(source: &dyn Source, facts: &IndexFacts) -> bool {
        source.enabled()
            && !facts.sensitive
            && !facts.protected
            && !facts.unsaved
            && source.recorded(&facts.path)
    }

    /// Takes the text of the pages of `tab`'s document that the index does not have, a page at a
    /// time, and keeps it. Returns when to try again, if it is to be.
    fn take_text(&self, source: &dyn Source, tab: TabId) -> Option<Instant> {
        let facts = source.facts(tab)?;
        if !Self::may_take(source, &facts) {
            return None;
        }
        if source.ocr_busy(tab) {
            return Some(Instant::now() + OCR_WAIT);
        }
        if !drives::is_fixed(&facts.path) {
            return None;
        }
        let identity = facts.identity.and_then(FileIdentity::parts)?;
        let key = canonical(&facts.path)?;
        let (generation, entry, secret) = {
            let mut state = lock(&self.state);
            let generation = state.generation;
            let loaded = self.loaded(&mut state, false)?;
            let entry = loaded
                .catalog
                .entries
                .iter()
                .find(|entry| entry.key == key && entry.same_file(identity))?
                .clone();
            (generation, entry, loaded.key.clone())
        };
        if entry.done == entry.pages && entry.blank == 0 {
            return None;
        }
        let mut pages: Zeroizing<Vec<String>> = Zeroizing::new(Vec::new());
        let cipher = Cipher::new(&secret).ok()?;
        if entry.done > 0 {
            match self.read_document(&cipher, entry.id) {
                Ok(plain) => match decode_pages(&plain) {
                    Some(found) => pages.extend(found.into_iter().map(str::to_owned)),
                    None => {
                        self.lost(generation);
                        return None;
                    }
                },
                Err(Some(CryptoError::Tampered)) => {
                    self.lost(generation);
                    return None;
                }
                // Not there: it is taken again.
                Err(_) => {}
            }
        }
        pages.truncate(entry.pages as usize);
        let mut partial = entry.partial;
        let mut used: usize = pages.iter().map(String::len).sum();
        let blank: Vec<u32> = (0..pages.len() as u32)
            .filter(|&page| pages[page as usize].is_empty())
            .collect();
        let next: Vec<u32> = if partial {
            Vec::new()
        } else {
            (pages.len() as u32..entry.pages).collect()
        };
        let mut written = Instant::now();
        let mut changed = false;
        for page in blank.into_iter().chain(next) {
            if !source.enabled() {
                return None;
            }
            let Ok(text) = source.page_text(facts.doc, page) else {
                break;
            };
            // Only what the file has: the document did not change while the text was taken.
            match source.facts(tab) {
                Some(now) if now.doc == facts.doc && Self::may_take(source, &now) => {}
                _ => break,
            }
            let index = page as usize;
            let before = pages.get(index).map_or(0, String::len);
            if used - before + text.len() > MAX_INDEX_DOCUMENT_TEXT_BYTES {
                partial = true;
                break;
            }
            used = used - before + text.len();
            if index < pages.len() {
                if pages[index] != text {
                    pages[index] = text;
                    changed = true;
                }
            } else {
                pages.push(text);
                changed = true;
            }
            if changed && written.elapsed() >= WRITE_EVERY {
                if !self.keep(&cipher, generation, &entry, &pages, partial) {
                    return None;
                }
                written = Instant::now();
                changed = false;
            }
            std::thread::sleep(PAUSE);
        }
        if changed || partial != entry.partial {
            self.keep(&cipher, generation, &entry, &pages, partial);
        }
        None
    }

    /// The plain text of the document `id`'s file: `Err(None)` when it cannot be read,
    /// `Err(Some(Tampered))` when it does not open with the key.
    fn read_document(
        &self,
        cipher: &Cipher,
        id: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Option<CryptoError>> {
        let file = self.file(&document_file(id)).ok_or(None)?;
        let sealed = local_data::read(&file, MAX_DOCUMENT_FILE_BYTES).ok_or(None)?;
        cipher.open(&document_data(id), &sealed).map_err(Some)
    }

    /// Writes the text taken of `entry`'s document, and the catalog. False when the document left
    /// the index meanwhile (or the index was deleted): then nothing of it stays.
    fn keep(
        &self,
        cipher: &Cipher,
        generation: u64,
        entry: &Entry,
        pages: &[String],
        partial: bool,
    ) -> bool {
        let Some(file) = self.file(&document_file(entry.id)) else {
            return false;
        };
        let plain = encode_pages(pages);
        let Ok(sealed) = cipher.seal(&document_data(entry.id), &plain) else {
            return false;
        };
        // Written under the lock: an index deleted meanwhile is not made again by a file of the
        // old one.
        let mut state = lock(&self.state);
        if state.generation != generation {
            return false;
        }
        let Some(loaded) = self.loaded(&mut state, false) else {
            return false;
        };
        if !loaded
            .catalog
            .entries
            .iter()
            .any(|found| found.id == entry.id)
        {
            return false;
        }
        if local_data::write(&file, &sealed).is_err() {
            local_data::remove(&file);
            return false;
        }
        if let Some(found) = loaded
            .catalog
            .entries
            .iter_mut()
            .find(|found| found.id == entry.id)
        {
            found.done = pages.len() as u32;
            found.blank = pages.iter().filter(|page| page.is_empty()).count() as u32;
            found.partial = partial;
            found.bytes = sealed.len() as u64;
        }
        self.keep_within_limits(loaded, Some(entry.id));
        let _ = self.write_catalog(loaded);
        true
    }

    /// Something of the index did not open with its key: it is deleted, and starts again with
    /// the documents that are open. Not if it was deleted since `generation` (another key).
    fn lost(&self, generation: u64) {
        let mut state = lock(&self.state);
        if state.generation == generation {
            self.wipe(&mut state);
            state.reconsider = true;
            self.wake.notify_all();
        }
    }

    /// Searches the documents of the index for `args.query`, the ones opened last first, sending
    /// each that has hits through `send` (false: nobody is listening), then `Done`. Before a
    /// document is sent, its file is checked: one that is gone or changed leaves the index and is
    /// not sent, so that what it had is not shown.
    pub fn search(
        &self,
        source: &dyn Source,
        args: &IndexSearchArgs,
        stop: &AtomicBool,
        send: &mut dyn FnMut(IndexSearchEvent) -> bool,
    ) -> Result<(), IpcError> {
        let pattern = normalize_query(&args.query, args.case_sensitive);
        let snapshot = {
            let mut state = lock(&self.state);
            state.listed.clear();
            let generation = state.generation;
            match self.loaded(&mut state, false) {
                Some(loaded) if source.enabled() && !pattern.is_empty() => {
                    let mut entries: Vec<Entry> = loaded
                        .catalog
                        .entries
                        .iter()
                        .filter(|entry| entry.done > 0)
                        .cloned()
                        .collect();
                    entries.sort_by_key(|entry| std::cmp::Reverse(entry.opened));
                    Some((generation, entries, loaded.key.clone()))
                }
                _ => None,
            }
        };
        let Some((generation, entries, secret)) = snapshot else {
            send(IndexSearchEvent::Done {
                documents: 0,
                found: 0,
            });
            return Ok(());
        };
        let next = AtomicUsize::new(0);
        let tampered = AtomicBool::new(false);
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .clamp(1, MAX_SEARCH_THREADS);
        let mut found = 0u32;
        let mut documents = 0u32;
        let mut listening = true;
        let mut folders = FixedFolders::default();
        std::thread::scope(|scope| {
            let (results, received) = mpsc::channel::<(usize, Option<DocumentHits>)>();
            for _ in 0..threads {
                let results = results.clone();
                let (entries, next, tampered, pattern, secret) =
                    (&entries, &next, &tampered, &pattern, &secret);
                scope.spawn(move || {
                    let Ok(cipher) = Cipher::new(secret) else {
                        return;
                    };
                    loop {
                        let at = next.fetch_add(1, Ordering::SeqCst);
                        let Some(entry) = entries.get(at) else {
                            return;
                        };
                        if stop.load(Ordering::SeqCst) || tampered.load(Ordering::SeqCst) {
                            return;
                        }
                        let hits = match self.read_document(&cipher, entry.id) {
                            Ok(plain) => match decode_pages(&plain) {
                                Some(pages) => search_pages(&pages, pattern, args.case_sensitive),
                                None => {
                                    tampered.store(true, Ordering::SeqCst);
                                    None
                                }
                            },
                            Err(Some(CryptoError::Tampered)) => {
                                tampered.store(true, Ordering::SeqCst);
                                None
                            }
                            Err(_) => None,
                        };
                        if results.send((at, hits)).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(results);
            // In the order of the documents, whichever thread searched them.
            let mut waiting: BTreeMap<usize, Option<DocumentHits>> = BTreeMap::new();
            let mut due = 0;
            for (at, hits) in received {
                waiting.insert(at, hits);
                while let Some(hits) = waiting.remove(&due) {
                    let entry = &entries[due];
                    due += 1;
                    documents += 1;
                    let Some(hits) = hits else {
                        continue;
                    };
                    if !listening || stop.load(Ordering::SeqCst) {
                        continue;
                    }
                    if !self.still_there(entry, &mut folders) {
                        continue;
                    }
                    let event = self.list(entry, hits);
                    if event.validate().is_ok() {
                        found += 1;
                        listening = send(event);
                        if !listening {
                            stop.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        });
        if tampered.load(Ordering::SeqCst) {
            self.lost(generation);
        }
        if stop.load(Ordering::SeqCst) {
            return Err(IpcError {
                code: ErrorCode::Cancelled,
                message: "cancelled".to_owned(),
            });
        }
        send(IndexSearchEvent::Done { documents, found });
        Ok(())
    }

    /// Whether `entry`'s file is still there as it was; if not, it leaves the index.
    fn still_there(&self, entry: &Entry, folders: &mut FixedFolders) -> bool {
        let same =
            identity_now(Path::new(&entry.path), folders).is_some_and(|now| entry.same_file(now));
        if !same {
            let mut state = lock(&self.state);
            if let Some(loaded) = self.loaded(&mut state, false)
                && self.remove_entries(loaded, |found| found.id == entry.id)
            {
                let _ = self.write_catalog(loaded);
            }
        }
        same
    }

    /// The result the page gets for `entry`, under a number of its own.
    fn list(&self, entry: &Entry, hits: DocumentHits) -> IndexSearchEvent {
        let mut state = lock(&self.state);
        state.next_number = state.next_number.wrapping_add(1);
        let number = state.next_number;
        state.listed.insert(number, entry.id);
        IndexSearchEvent::Found {
            entry: IndexEntryId(number),
            display_name: entry.name.clone(),
            total_hits: hits.total,
            pages: hits.pages,
            pages_with_hits: hits.pages_with_hits,
            partial: entry.partial || entry.done < entry.pages,
        }
    }

    /// The path of the document of a result of the last search, to open it: if its file is still
    /// there as it was (if not, it leaves the index, and the answer is `unreadable`).
    pub fn file_of(&self, entry: IndexEntryId) -> Result<PathBuf, IpcError> {
        let unknown = || IpcError {
            code: ErrorCode::InvalidArgument,
            message: "no such result".to_owned(),
        };
        let found = {
            let mut state = lock(&self.state);
            let id = *state.listed.get(&entry.0).ok_or_else(unknown)?;
            let loaded = self.loaded(&mut state, false).ok_or_else(unknown)?;
            loaded
                .catalog
                .entries
                .iter()
                .find(|found| found.id == id)
                .cloned()
                .ok_or_else(unknown)?
        };
        if !self.still_there(&found, &mut FixedFolders::default()) {
            return Err(IpcError {
                code: ErrorCode::Unreadable,
                message: "the file is no longer there, or has changed".to_owned(),
            });
        }
        Ok(PathBuf::from(found.path))
    }
}

/// The index on disk cannot be read with its key.
struct Lost;

/// The ids of the entries to take out, those opened longest ago first, until at most `max_count`
/// are left and their files take at most `max_bytes`; never `keep`.
fn over_limits(entries: &[Entry], keep: Option<u64>, max_count: usize, max_bytes: u64) -> Vec<u64> {
    let mut order: Vec<&Entry> = entries.iter().collect();
    order.sort_by_key(|entry| entry.opened);
    let mut count = entries.len();
    let mut bytes: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let mut gone = Vec::new();
    for entry in order {
        if count <= max_count && bytes <= max_bytes {
            break;
        }
        if Some(entry.id) == keep {
            continue;
        }
        count -= 1;
        bytes -= entry.bytes;
        gone.push(entry.id);
    }
    gone
}

/// The hits of a search in one document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentHits {
    total: u32,
    pages_with_hits: u32,
    pages: Vec<IndexPageHits>,
}

/// Searches the pages of one document for `pattern` (folded), as the worker searches a page: a
/// phrase can go on on the next line. `None` when nothing is found.
///
/// The text is folded a character at a time and not kept: a full index is searched in about
/// half a second (docs/architecture/text-index.md).
fn search_pages(pages: &[&str], pattern: &[char], case_sensitive: bool) -> Option<DocumentHits> {
    if pattern.is_empty() {
        return None;
    }
    let mut matcher = Matcher::new(pattern);
    // Where each of the last `pattern.len()` folded characters starts in the page.
    let mut origins = vec![0usize; pattern.len()];
    let mut total = 0u32;
    let mut pages_with_hits = 0u32;
    let mut listed = Vec::new();
    for (page_index, text) in pages.iter().enumerate() {
        if total >= MAX_SEARCH_HITS {
            break;
        }
        matcher.reset();
        let mut hits = 0u32;
        let mut first: Option<(usize, usize)> = None;
        let mut count = 0usize;
        // Spaces at the start are dropped, and runs of them are one.
        let mut after_space = true;
        for (at, original) in text.char_indices() {
            let c = fold(original, case_sensitive);
            if c == ' ' {
                if after_space {
                    continue;
                }
                after_space = true;
            } else {
                after_space = false;
            }
            origins[count % pattern.len()] = at;
            count += 1;
            if matcher.push(c) {
                if first.is_none() {
                    let start = origins[(count - pattern.len()) % pattern.len()];
                    first = Some((start, at + original.len_utf8()));
                }
                hits += 1;
                if total + hits >= MAX_SEARCH_HITS {
                    break;
                }
            }
        }
        let Some((start, end)) = first else {
            continue;
        };
        total += hits;
        pages_with_hits += 1;
        if listed.len() < MAX_INDEX_PAGES_PER_DOCUMENT as usize {
            listed.push(IndexPageHits {
                page_index: page_index as u32,
                hits,
                snippet: snippet(text, start, end),
            });
        }
    }
    (total > 0).then_some(DocumentHits {
        total,
        pages_with_hits,
        pages: listed,
    })
}

/// The words of `text[start..end]` with some text before and after them, as plain text.
fn snippet(text: &str, start: usize, end: usize) -> IndexSnippet {
    let max = MAX_INDEX_SNIPPET_CHARS as usize;
    // Room for what cleaning takes out.
    let window = max * 4;
    let mut before: Vec<char> = text[..start].chars().rev().take(window).collect();
    before.reverse();
    let before = plain(before);
    let before: String = {
        let count = before.chars().count();
        before.chars().skip(count.saturating_sub(max)).collect()
    };
    let after: String = plain(text[end..].chars().take(window))
        .chars()
        .take(max)
        .collect();
    let mut matched = plain(text[start..end].chars());
    if matched.is_empty() {
        matched = "\u{2026}".to_owned();
    }
    IndexSnippet {
        before: collapse_ends(&before),
        matched,
        after: collapse_ends(&after),
    }
}

/// `chars` as copied text, every run of spaces one space.
fn plain(chars: impl IntoIterator<Item = char>) -> String {
    let mut out = String::new();
    for c in chars.into_iter().filter_map(copy_text_char) {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out
}

/// Cutting a run of spaces can leave two side by side across the cut: one is kept.
fn collapse_ends(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::atomic::AtomicU32;

    use ipc_contract::types::RequestId;

    use super::*;

    /// A folder of its own for each test: the files the documents are opened from, and `index\`.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pdf-reader-index-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file to open: what is in it does not matter to the index, only its path, size and time.
    fn pdf(dir: &Path, name: &str) -> PathBuf {
        let file = dir.join(name);
        fs::write(&file, b"%PDF-1.7 synthetic").unwrap();
        file
    }

    /// An open document of the fake app.
    struct Open {
        tab: TabId,
        facts: IndexFacts,
        pages: Vec<String>,
    }

    /// The app, as the index sees it.
    #[derive(Default)]
    struct Fake {
        off: AtomicBool,
        not_recorded: Mutex<Vec<PathBuf>>,
        open: Mutex<Vec<Open>>,
        ocr_busy: AtomicBool,
        /// How many pages' text was asked for.
        asked: AtomicU32,
        /// After this many pages were asked for, the document has changes that are not saved.
        edit_after: Mutex<Option<u32>>,
    }

    impl Fake {
        fn open(&self, tab: u32, doc: u32, file: &Path, pages: &[&str]) {
            let facts = IndexFacts {
                doc: DocumentId(doc),
                path: file.to_owned(),
                display_name: file.file_name().unwrap().to_str().unwrap().to_owned(),
                sensitive: false,
                protected: false,
                unsaved: false,
                pages: pages.len() as u32,
                identity: FileIdentity::of(file),
            };
            let pages = pages.iter().map(|&page| page.to_owned()).collect();
            let mut open = lock(&self.open);
            open.retain(|found| found.tab != TabId(tab));
            open.push(Open {
                tab: TabId(tab),
                facts,
                pages,
            });
        }

        fn change(&self, tab: u32, change: impl FnOnce(&mut Open)) {
            let mut open = lock(&self.open);
            change(
                open.iter_mut()
                    .find(|found| found.tab == TabId(tab))
                    .unwrap(),
            );
        }
    }

    impl Source for Fake {
        fn enabled(&self) -> bool {
            !self.off.load(Ordering::SeqCst)
        }

        fn recorded(&self, path: &Path) -> bool {
            !lock(&self.not_recorded).iter().any(|found| found == path)
        }

        fn tabs(&self) -> Vec<TabId> {
            lock(&self.open).iter().map(|found| found.tab).collect()
        }

        fn facts(&self, tab: TabId) -> Option<IndexFacts> {
            lock(&self.open)
                .iter()
                .find(|found| found.tab == tab)
                .map(|found| found.facts.clone())
        }

        fn ocr_busy(&self, _: TabId) -> bool {
            self.ocr_busy.load(Ordering::SeqCst)
        }

        fn page_text(&self, doc: DocumentId, page_index: u32) -> Result<String, IpcError> {
            let asked = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
            let edit_after = *lock(&self.edit_after);
            let mut open = lock(&self.open);
            let found = open
                .iter_mut()
                .find(|found| found.facts.doc == doc)
                .ok_or_else(|| IpcError {
                    code: ErrorCode::InvalidArgument,
                    message: "no such document".to_owned(),
                })?;
            if edit_after.is_some_and(|after| asked >= after) {
                found.facts.unsaved = true;
            }
            Ok(found.pages[page_index as usize].clone())
        }
    }

    fn search_with(
        index: &TextIndex,
        source: &dyn Source,
        query: &str,
        case_sensitive: bool,
    ) -> Vec<IndexSearchEvent> {
        let mut events = Vec::new();
        let args = IndexSearchArgs {
            request: RequestId(1),
            query: query.to_owned(),
            case_sensitive,
        };
        index
            .search(source, &args, &AtomicBool::new(false), &mut |event| {
                events.push(event);
                true
            })
            .unwrap();
        events
    }

    fn search(index: &TextIndex, source: &dyn Source, query: &str) -> Vec<IndexSearchEvent> {
        search_with(index, source, query, false)
    }

    /// The names of the documents found, and how many documents were searched.
    fn found(events: &[IndexSearchEvent]) -> (Vec<String>, u32) {
        let mut names = Vec::new();
        for event in events {
            match event {
                IndexSearchEvent::Found { display_name, .. } => names.push(display_name.clone()),
                IndexSearchEvent::Done { documents, found } => {
                    assert_eq!(*found as usize, names.len());
                    return (names, *documents);
                }
            }
        }
        panic!("no done: {events:?}");
    }

    fn entry_with(id: u64, opened: u64, bytes: u64) -> Entry {
        Entry {
            id,
            path: String::new(),
            key: String::new(),
            name: String::new(),
            size: 0,
            modified_secs: 0,
            modified_nanos: 0,
            opened,
            pages: 1,
            done: 1,
            blank: 0,
            partial: false,
            bytes,
        }
    }

    #[test]
    fn pages_are_written_and_read_back_and_nothing_else_reads_as_pages() {
        let pages = vec!["one\ntwo".to_owned(), String::new(), "三".to_owned()];
        let bytes = encode_pages(&pages);
        assert_eq!(decode_pages(&bytes).unwrap(), ["one\ntwo", "", "三"]);
        assert_eq!(decode_pages(&bytes[..bytes.len() - 1]), None);
        let mut longer = bytes.to_vec();
        longer.push(0);
        assert_eq!(decode_pages(&longer), None);
        assert_eq!(decode_pages(&[0xff, 0xff, 0xff, 0xff]), None);
        // A page that is not UTF-8.
        assert_eq!(decode_pages(&[1, 0, 0, 0, 1, 0, 0, 0, 0xff]), None);
        assert_eq!(decode_pages(&[]), None);
    }

    #[test]
    fn a_phrase_is_found_across_lines_with_a_little_plain_text_around_it() {
        let pages = [
            "Quarterly report\nfor the  Harbour\nBridge project",
            "nothing here",
            "harbour bridge, and the Harbour Bridge again",
        ];
        let hits = search_pages(&pages, &normalize_query("harbour  bridge", false), false).unwrap();
        assert_eq!((hits.total, hits.pages_with_hits), (3, 2));
        let listed: Vec<(u32, u32)> = hits
            .pages
            .iter()
            .map(|page| (page.page_index, page.hits))
            .collect();
        assert_eq!(listed, [(0, 1), (2, 2)]);
        let first = &hits.pages[0].snippet;
        assert_eq!(first.before, "Quarterly report for the ");
        assert_eq!(first.matched, "Harbour Bridge");
        assert_eq!(first.after, " project");
        assert!(first.validate().is_ok());

        let exact = search_pages(&pages, &normalize_query("Harbour Bridge", true), true).unwrap();
        assert_eq!(exact.total, 2);
        assert_eq!(
            search_pages(&pages, &normalize_query("tunnel", false), false),
            None
        );
    }

    #[test]
    fn the_text_around_a_hit_is_cut_to_its_limit() {
        let page = format!("{} needle {}", "a".repeat(200), "b".repeat(200));
        let hits = search_pages(&[&page], &normalize_query("needle", false), false).unwrap();
        let snippet = &hits.pages[0].snippet;
        let max = MAX_INDEX_SNIPPET_CHARS as usize;
        assert_eq!(snippet.before, format!("{} ", "a".repeat(max - 1)));
        assert_eq!(snippet.after, format!(" {}", "b".repeat(max - 1)));
        assert!(snippet.validate().is_ok());
    }

    #[test]
    fn only_the_first_pages_with_hits_are_listed_but_all_are_counted() {
        let pages: Vec<String> = (0..MAX_INDEX_PAGES_PER_DOCUMENT + 5)
            .map(|page| format!("page {page} has the word"))
            .collect();
        let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
        let hits = search_pages(&pages, &normalize_query("word", false), false).unwrap();
        assert_eq!(hits.total, MAX_INDEX_PAGES_PER_DOCUMENT + 5);
        assert_eq!(hits.pages_with_hits, MAX_INDEX_PAGES_PER_DOCUMENT + 5);
        assert_eq!(hits.pages.len(), MAX_INDEX_PAGES_PER_DOCUMENT as usize);
    }

    #[test]
    fn a_document_goes_in_and_is_found_by_its_name_never_its_path() {
        let dir = folder("found");
        let file = pdf(&dir, "budget-2026.pdf");
        let fake = Fake::default();
        fake.open(
            1,
            1,
            &file,
            &["Annual budget", "The harbour\nbridge costs", "Summary"],
        );
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));

        index.consider(&fake, TabId(1));
        // In the index, but its text is not taken yet.
        assert_eq!(index.status().documents, 1);
        assert_eq!(found(&search(&index, &fake, "harbour")), (vec![], 0));

        assert_eq!(index.take_text(&fake, TabId(1)), None);
        assert_eq!(fake.asked.load(Ordering::SeqCst), 3);
        let events = search(&index, &fake, "HARBOUR bridge");
        let IndexSearchEvent::Found {
            entry,
            display_name,
            total_hits,
            pages,
            pages_with_hits,
            partial,
        } = &events[0]
        else {
            panic!("{events:?}");
        };
        assert_eq!(display_name, "budget-2026.pdf");
        assert_eq!((*total_hits, *pages_with_hits, *partial), (1, 1, false));
        assert_eq!(pages[0].page_index, 1);
        assert_eq!(pages[0].snippet.matched, "harbour bridge");
        assert_eq!(found(&events), (vec!["budget-2026.pdf".to_owned()], 1));
        // Nothing the page gets says where the file is.
        let json = serde_json::to_string(&events).unwrap();
        assert!(
            !json.contains(dir.file_name().unwrap().to_str().unwrap()),
            "{json}"
        );
        assert!(!json.contains(":\\\\"), "{json}");

        assert_eq!(index.file_of(*entry).unwrap(), file);
        // The numbers are of the last search only.
        let entry = *entry;
        search(&index, &fake, "budget");
        assert_eq!(
            index.file_of(entry).unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        // Taken: nothing more is asked for.
        assert_eq!(index.take_text(&fake, TabId(1)), None);
        assert_eq!(fake.asked.load(Ordering::SeqCst), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_case_sensitive_search_finds_only_the_same_letters() {
        let dir = folder("case");
        let file = pdf(&dir, "case.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &file, &["The Harbour Bridge"]);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        assert_eq!(
            found(&search_with(&index, &fake, "harbour", true)).0.len(),
            0
        );
        assert_eq!(
            found(&search_with(&index, &fake, "Harbour", true)).0.len(),
            1
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_on_the_disk_can_be_read_without_the_key() {
        let dir = folder("sealed");
        let file = pdf(&dir, "zebracorn-name.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &file, &["the zebracorn quintessence", "第二頁的文字"]);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        assert_eq!(found(&search(&index, &fake, "quintessence")).0.len(), 1);

        let path = file.to_str().unwrap();
        let needles = ["zebracorn", "quintessence", "第二頁", path, "budget"];
        let mut files = 0;
        for found in fs::read_dir(dir.join(FOLDER_NAME)).unwrap() {
            let bytes = fs::read(found.unwrap().path()).unwrap();
            files += 1;
            for needle in needles {
                let utf8 = needle.as_bytes();
                let utf16: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
                for plain in [utf8, &utf16[..]] {
                    assert!(
                        !bytes.windows(plain.len()).any(|window| window == plain),
                        "{needle}"
                    );
                }
            }
        }
        // The key, the catalog and the document's text.
        assert_eq!(files, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn blank_pages_are_looked_at_again_and_their_text_is_found_once_it_is_there() {
        let dir = folder("blank");
        let file = pdf(&dir, "scan.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &file, &["typed words", ""]);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        assert_eq!(fake.asked.load(Ordering::SeqCst), 2);

        // While its scanned pages are being recognised, nothing is asked for.
        fake.ocr_busy.store(true, Ordering::SeqCst);
        assert!(index.take_text(&fake, TabId(1)).is_some());
        assert_eq!(fake.asked.load(Ordering::SeqCst), 2);
        fake.ocr_busy.store(false, Ordering::SeqCst);

        // Recognised: only the blank page is asked for again.
        fake.change(1, |open| open.pages[1] = "recognised words".to_owned());
        index.take_text(&fake, TabId(1));
        assert_eq!(fake.asked.load(Ordering::SeqCst), 3);
        assert_eq!(found(&search(&index, &fake, "recognised")).0.len(), 1);
        assert_eq!(found(&search(&index, &fake, "typed")).0.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn taking_stops_at_changes_that_are_not_saved_and_goes_on_when_the_file_opens_again() {
        let dir = folder("unsaved");
        let file = pdf(&dir, "draft.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &file, &["alpha", "beta", "gamma", "delta"]);
        *lock(&fake.edit_after) = Some(2);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        // The second page came after the change: only the first is the file's.
        assert_eq!(found(&search(&index, &fake, "alpha")).0.len(), 1);
        assert_eq!(found(&search(&index, &fake, "beta")).0.len(), 0);
        let events = search(&index, &fake, "alpha");
        assert!(matches!(
            events[0],
            IndexSearchEvent::Found { partial: true, .. }
        ));

        // Opened again, unchanged: the rest is taken.
        *lock(&fake.edit_after) = None;
        fake.open(2, 2, &file, &["alpha", "beta", "gamma", "delta"]);
        index.consider(&fake, TabId(2));
        let before = fake.asked.load(Ordering::SeqCst);
        index.take_text(&fake, TabId(2));
        assert_eq!(fake.asked.load(Ordering::SeqCst) - before, 3);
        let events = search(&index, &fake, "delta");
        assert!(matches!(
            events[0],
            IndexSearchEvent::Found { partial: false, .. }
        ));
        assert_eq!(index.status().documents, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn what_may_not_go_in_stays_out() {
        let dir = folder("out");
        let file = pdf(&dir, "kept-out.pdf");
        type Change = fn(&Fake, &mut Open);
        let cases: [(&str, Change); 6] = [
            ("sensitive", |_, open| open.facts.sensitive = true),
            ("needed a password", |_, open| open.facts.protected = true),
            ("unsaved changes", |_, open| open.facts.unsaved = true),
            ("not to record", |fake, open| {
                lock(&fake.not_recorded).push(open.facts.path.clone())
            }),
            ("index off", |fake, _| {
                fake.off.store(true, Ordering::SeqCst)
            }),
            ("on a network place", |_, open| {
                open.facts.path = PathBuf::from(r"\\server.invalid\share\kept-out.pdf");
            }),
        ];
        for (why, change) in cases {
            let fake = Fake::default();
            fake.open(1, 1, &file, &["secret words"]);
            let mut open = lock(&fake.open).pop().unwrap();
            change(&fake, &mut open);
            lock(&fake.open).push(open);
            let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
            index.consider(&fake, TabId(1));
            assert_eq!(index.take_text(&fake, TabId(1)), None, "{why}");
            assert_eq!(index.status().documents, 0, "{why}");
            assert_eq!(fake.asked.load(Ordering::SeqCst), 0, "{why}");
            fake.off.store(false, Ordering::SeqCst);
            assert_eq!(found(&search(&index, &fake, "secret")).0.len(), 0, "{why}");
            index.clear();
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_document_marked_sensitive_or_not_to_record_leaves_at_once() {
        let dir = folder("leaves");
        let file = pdf(&dir, "leaves.pdf");
        let index_dir = dir.join(FOLDER_NAME);
        type Change = fn(&Fake, &mut Open);
        let cases: [(&str, Change); 3] = [
            ("sensitive", |_, open| open.facts.sensitive = true),
            ("needed a password", |_, open| open.facts.protected = true),
            ("not to record", |fake, open| {
                lock(&fake.not_recorded).push(open.facts.path.clone())
            }),
        ];
        for (why, change) in cases {
            let fake = Fake::default();
            fake.open(1, 1, &file, &["private words"]);
            let index = TextIndex::new(Some(index_dir.clone()));
            index.consider(&fake, TabId(1));
            index.take_text(&fake, TabId(1));
            assert_eq!(found(&search(&index, &fake, "private")).0.len(), 1, "{why}");

            fake.change(1, |open| change(&fake, open));
            index.consider(&fake, TabId(1));
            assert_eq!(index.status().documents, 0, "{why}");
            // Its text is gone from the disk, not only from the catalog.
            let left: Vec<_> = fs::read_dir(&index_dir)
                .unwrap()
                .map(|found| found.unwrap().file_name())
                .collect();
            assert_eq!(left.len(), 2, "{why}: {left:?}");
            index.clear();
        }

        // Forgetting by path, as the list of files not to record does.
        let fake = Fake::default();
        fake.open(1, 1, &file, &["private words"]);
        let index = TextIndex::new(Some(index_dir.clone()));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        index.forget(&file);
        assert_eq!(index.status().documents, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_saved_file_is_taken_again_and_its_old_text_goes() {
        let dir = folder("saved");
        let file = pdf(&dir, "contract.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &file, &["old clause"]);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        assert_eq!(found(&search(&index, &fake, "old clause")).0.len(), 1);

        // Saved: another size, another document.
        fs::write(&file, b"%PDF-1.7 synthetic, saved with changes").unwrap();
        fake.change(1, |open| {
            open.facts.doc = DocumentId(2);
            open.facts.identity = FileIdentity::of(&file);
            open.pages = vec!["new clause".to_owned()];
        });
        index.consider(&fake, TabId(1));
        assert_eq!(found(&search(&index, &fake, "old clause")), (vec![], 0));
        index.take_text(&fake, TabId(1));
        assert_eq!(found(&search(&index, &fake, "old clause")).0.len(), 0);
        assert_eq!(found(&search(&index, &fake, "new clause")).0.len(), 1);
        assert_eq!(index.status().documents, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_changed_or_gone_since_is_not_listed_and_leaves() {
        let dir = folder("changed");
        let changed = pdf(&dir, "changed.pdf");
        let gone = pdf(&dir, "gone.pdf");
        let fake = Fake::default();
        fake.open(1, 1, &changed, &["shared words"]);
        fake.open(2, 2, &gone, &["shared words"]);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        for tab in [1, 2] {
            index.consider(&fake, TabId(tab));
            index.take_text(&fake, TabId(tab));
        }
        let events = search(&index, &fake, "shared");
        assert_eq!(found(&events).0.len(), 2);

        fs::write(&changed, b"%PDF-1.7 synthetic, changed by another program").unwrap();
        fs::remove_file(&gone).unwrap();
        // A result listed before: its file is checked again before it opens.
        let IndexSearchEvent::Found { entry, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(
            index.file_of(*entry).unwrap_err().code,
            ErrorCode::Unreadable
        );
        assert_eq!(found(&search(&index, &fake, "shared")), (vec![], 1));
        assert_eq!(index.status().documents, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_documents_opened_last_come_first() {
        let dir = folder("order");
        let fake = Fake::default();
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        for (tab, name) in [(1, "first.pdf"), (2, "second.pdf"), (3, "third.pdf")] {
            fake.open(tab, tab, &pdf(&dir, name), &["common words"]);
            index.consider(&fake, TabId(tab));
            index.take_text(&fake, TabId(tab));
        }
        // Opened again: it comes first.
        index.consider(&fake, TabId(1));
        let (names, documents) = found(&search(&index, &fake, "common"));
        assert_eq!(names, ["first.pdf", "third.pdf", "second.pdf"]);
        assert_eq!(documents, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stopped_search_says_so_and_nobody_listening_stops_it() {
        let dir = folder("stop");
        let fake = Fake::default();
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        for tab in 1..=3 {
            fake.open(
                tab,
                tab,
                &pdf(&dir, &format!("{tab}.pdf")),
                &["common words"],
            );
            index.consider(&fake, TabId(tab));
            index.take_text(&fake, TabId(tab));
        }
        let args = IndexSearchArgs {
            request: RequestId(1),
            query: "common".to_owned(),
            case_sensitive: false,
        };
        let stopped = AtomicBool::new(true);
        let error = index
            .search(&fake, &args, &stopped, &mut |_| true)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);

        let mut sent = 0;
        let error = index
            .search(&fake, &args, &AtomicBool::new(false), &mut |_| {
                sent += 1;
                false
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(sent, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_document_file_that_does_not_open_with_the_key_loses_the_index_which_starts_again() {
        let dir = folder("tampered");
        let file = pdf(&dir, "tampered.pdf");
        let index_dir = dir.join(FOLDER_NAME);
        let fake = Fake::default();
        fake.open(1, 1, &file, &["original words"]);
        let index = TextIndex::new(Some(index_dir.clone()));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        let old_key = fs::read(index_dir.join(KEY_FILE)).unwrap();

        let document = index_dir.join(document_file(0));
        let mut sealed = fs::read(&document).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        fs::write(&document, sealed).unwrap();
        assert_eq!(found(&search(&index, &fake, "original")), (vec![], 1));
        assert!(!index_dir.exists());
        assert_eq!(index.status().documents, 0);
        assert!(lock(&index.state).reconsider);

        // It starts again with the documents that are open, under a new key.
        index.consider_all(&fake);
        index.take_text(&fake, TabId(1));
        assert_eq!(found(&search(&index, &fake, "original")).0.len(), 1);
        assert_ne!(fs::read(index_dir.join(KEY_FILE)).unwrap(), old_key);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_catalog_or_key_that_does_not_open_loses_the_index() {
        let dir = folder("lost");
        let file = pdf(&dir, "lost.pdf");
        let index_dir = dir.join(FOLDER_NAME);
        for spoiled in [CATALOG_FILE, KEY_FILE] {
            let fake = Fake::default();
            fake.open(1, 1, &file, &["some words"]);
            let index = TextIndex::new(Some(index_dir.clone()));
            index.consider(&fake, TabId(1));
            index.take_text(&fake, TabId(1));
            drop(index);

            let spoiled_file = index_dir.join(spoiled);
            let mut bytes = fs::read(&spoiled_file).unwrap();
            let middle = bytes.len() / 2;
            bytes[middle] ^= 1;
            fs::write(&spoiled_file, bytes).unwrap();
            let index = TextIndex::new(Some(index_dir.clone()));
            assert_eq!(index.status().documents, 0, "{spoiled}");
            assert!(!index_dir.exists(), "{spoiled}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_document_file_no_catalog_names_is_deleted() {
        let dir = folder("orphan");
        let file = pdf(&dir, "kept.pdf");
        let index_dir = dir.join(FOLDER_NAME);
        let fake = Fake::default();
        fake.open(1, 1, &file, &["kept words"]);
        let index = TextIndex::new(Some(index_dir.clone()));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        drop(index);
        let orphan = index_dir.join(document_file(7));
        fs::write(&orphan, b"left by a run that stopped").unwrap();

        let index = TextIndex::new(Some(index_dir.clone()));
        assert_eq!(index.status().documents, 1);
        assert!(!orphan.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clearing_deletes_the_key_first_so_what_cannot_go_cannot_be_read() {
        let dir = folder("clear");
        let file = pdf(&dir, "clear.pdf");
        let index_dir = dir.join(FOLDER_NAME);
        let fake = Fake::default();
        fake.open(1, 1, &file, &["words"]);
        let index = TextIndex::new(Some(index_dir.clone()));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));

        // Another program holds the document's file: it cannot be deleted now.
        let document = index_dir.join(document_file(0));
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&document)
            .unwrap();
        index.clear();
        assert!(!index_dir.join(KEY_FILE).exists());
        assert!(document.exists());
        assert_eq!(index.status().documents, 0);
        drop(held);

        // Without its key, what stayed goes the next time the index is read.
        let index = TextIndex::new(Some(index_dir.clone()));
        assert_eq!(index.status().documents, 0);
        assert!(!index_dir.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_drive_of_a_folder_is_asked_for_once_and_is_its_files_drive() {
        let dir = folder("drives");
        let mut folders = FixedFolders::default();
        assert!(folders.is_fixed(&pdf(&dir, "a.pdf")));
        assert!(folders.is_fixed(&dir.join("b.pdf")));
        assert_eq!(folders.0.len(), 1, "one folder, asked for once");
        // A network place is said by its name, as `drives::is_fixed` says it.
        assert!(!folders.is_fixed(Path::new(r"\\server.invalid\share\c.pdf")));
        assert_eq!(folders.0.len(), 2);
    }

    #[test]
    fn without_a_data_folder_nothing_is_kept() {
        let dir = folder("nowhere");
        let fake = Fake::default();
        fake.open(1, 1, &pdf(&dir, "nowhere.pdf"), &["words"]);
        let index = TextIndex::new(None);
        index.consider(&fake, TabId(1));
        assert_eq!(index.take_text(&fake, TabId(1)), None);
        assert_eq!(index.status().documents, 0);
        assert_eq!(found(&search(&index, &fake, "words")), (vec![], 0));
        index.clear();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_documents_opened_longest_ago_leave_first_never_the_one_being_kept() {
        let entries = [
            entry_with(1, 5, 10),
            entry_with(2, 1, 10),
            entry_with(3, 3, 10),
        ];
        assert_eq!(over_limits(&entries, None, 3, 30), Vec::<u64>::new());
        assert_eq!(over_limits(&entries, None, 2, 30), [2]);
        assert_eq!(over_limits(&entries, None, 3, 15), [2, 3]);
        assert_eq!(over_limits(&entries, Some(2), 2, 30), [3]);
        assert_eq!(over_limits(&entries, Some(1), 0, 0), [2, 3]);
    }

    #[test]
    fn a_document_with_more_text_than_its_limit_keeps_its_first_pages() {
        let dir = folder("partial");
        let file = pdf(&dir, "large.pdf");
        let page = "words ".repeat(1024 * 1024 / 6);
        let count = MAX_INDEX_DOCUMENT_TEXT_BYTES / page.len() + 2;
        let pages: Vec<&str> = (0..count).map(|_| page.as_str()).collect();
        let fake = Fake::default();
        fake.open(1, 1, &file, &pages);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        let kept = MAX_INDEX_DOCUMENT_TEXT_BYTES / page.len();
        assert_eq!(fake.asked.load(Ordering::SeqCst) as usize, kept + 1);
        let events = search(&index, &fake, "words");
        let IndexSearchEvent::Found {
            partial,
            pages_with_hits,
            ..
        } = &events[0]
        else {
            panic!("{events:?}");
        };
        assert!(*partial);
        // Every hit is counted up to the search's limit, as in one document.
        assert!(*pages_with_hits as usize <= kept);

        // Nothing more is asked for the next time.
        index.consider(&fake, TabId(1));
        index.take_text(&fake, TabId(1));
        assert_eq!(fake.asked.load(Ordering::SeqCst) as usize, kept + 1);
        let _ = fs::remove_dir_all(dir);
    }

    /// Opens a document for each of `MAX_INDEX_DOCUMENTS` files, with as much text as the index
    /// keeps in all, and times taking and searching it (docs/architecture/text-index.md). Only with
    /// `PDF_READER_INDEX_BENCH=1`: it takes a while and writes about 256 MiB.
    #[test]
    fn bench_a_full_index() {
        if std::env::var_os("PDF_READER_INDEX_BENCH").is_none() {
            return;
        }
        /// Words of a page of `doc`, the same every time.
        fn text_of(doc: u32, bytes: usize) -> String {
            const WORDS: [&str; 16] = [
                "contract", "harbour", "budget", "invoice", "meeting", "report", "summary",
                "delivery", "account", "project", "schedule", "payment", "review", "minutes",
                "order", "notice",
            ];
            let mut seed = u64::from(doc).wrapping_mul(6_364_136_223_846_793_005) | 1;
            let mut out = String::with_capacity(bytes + 16);
            while out.len() < bytes {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                out.push_str(WORDS[(seed >> 60) as usize]);
                out.push(if seed & 0x3f == 0 { '\n' } else { ' ' });
            }
            out
        }
        struct Bench(Vec<(PathBuf, Option<FileIdentity>)>);
        impl Source for Bench {
            fn enabled(&self) -> bool {
                true
            }
            fn recorded(&self, _: &Path) -> bool {
                true
            }
            fn tabs(&self) -> Vec<TabId> {
                (0..self.0.len() as u32).map(TabId).collect()
            }
            fn facts(&self, tab: TabId) -> Option<IndexFacts> {
                let (path, identity) = self.0.get(tab.0 as usize)?;
                Some(IndexFacts {
                    doc: DocumentId(tab.0),
                    path: path.clone(),
                    display_name: format!("{}.pdf", tab.0),
                    sensitive: false,
                    protected: false,
                    unsaved: false,
                    pages: 1,
                    identity: *identity,
                })
            }
            fn ocr_busy(&self, _: TabId) -> bool {
                false
            }
            fn page_text(&self, doc: DocumentId, _: u32) -> Result<String, IpcError> {
                // A few documents have a word the others do not.
                let mut text = text_of(doc.0, PAGE_BYTES);
                if doc.0 % 500 == 7 {
                    text.push_str(" zebracorn");
                }
                Ok(text)
            }
        }
        // As much as the index keeps, less what sealing and the catalog add.
        const PAGE_BYTES: usize = 120 * 1024;
        let dir = folder("bench");
        let files: Vec<(PathBuf, Option<FileIdentity>)> = (0..MAX_INDEX_DOCUMENTS)
            .map(|doc| {
                let file = pdf(&dir, &format!("{doc}.pdf"));
                let identity = FileIdentity::of(&file);
                (file, identity)
            })
            .collect();
        let source = Bench(files);
        let index = TextIndex::new(Some(dir.join(FOLDER_NAME)));
        let started = Instant::now();
        for tab in source.tabs() {
            index.consider(&source, tab);
            index.take_text(&source, tab);
        }
        let taking = started.elapsed();
        let status = index.status();
        let mut times = Vec::new();
        for query in [
            "zebracorn",
            "harbour budget",
            "contract",
            "no such words at all",
        ] {
            // The first time reads the files from the disk; the second, mostly from the cache.
            for run in ["first", "again"] {
                let args = IndexSearchArgs {
                    request: RequestId(1),
                    query: query.to_owned(),
                    case_sensitive: false,
                };
                let started = Instant::now();
                let (mut first, mut found, mut documents) = (None, 0, 0);
                index
                    .search(&source, &args, &AtomicBool::new(false), &mut |event| {
                        match event {
                            IndexSearchEvent::Found { .. } => {
                                first.get_or_insert(started.elapsed());
                                found += 1;
                            }
                            IndexSearchEvent::Done { documents: all, .. } => documents = all,
                        }
                        true
                    })
                    .unwrap();
                times.push(format!(
                    "{query:?} ({run}): {found} found of {documents}, the first after {first:?}, all in {:?}",
                    started.elapsed()
                ));
            }
        }
        eprintln!(
            "text index bench: {} documents, {} MiB, taken in {taking:?}\n{}",
            status.documents,
            status.bytes / (1024 * 1024),
            times.join("\n")
        );
        let _ = fs::remove_dir_all(dir);
    }
}
