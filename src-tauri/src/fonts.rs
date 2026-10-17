//! The fonts Windows has installed, for the text boxes (B2-27, ADR 0018).
//!
//! Windows lists them in the registry, under `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`
//! (everyone's: the value is the file's name, in the Windows fonts folder) and the same key of
//! `HKCU` (the user's own, installed for the current user only: the value is the file's full
//! path). A font's value name is what Windows calls it, with its kind in parentheses; a bold or
//! an italic face is a value of its own. A collection (`.ttc`) is one value whose name joins the
//! names of its fonts with ` & `, in the order of the file.
//!
//! The page gets a list of ids and names, never a path. The file is read by the main process
//! when the user has chosen the font, and its bytes go to the worker, which is the only one to
//! parse them (ADR 0018).

// The registry's Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]
// The worker's side of the text box (B2-28, #225) reads the files; until then only the list is used.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use ipc_contract::limits::{MAX_FONT_BYTES, MAX_FONT_NAME_BYTES, MAX_FONTS, MAX_IMPORTED_FONTS};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{ErrorCode, FontId, FontImportRefusal, FontInfo, FontSource};
use ipc_contract::worker::InspectedFont;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegEnumValueW, RegOpenKeyExW,
};

use crate::documents::Documents;

const FONTS_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";

/// The folder of imported fonts, in the app's local data folder.
pub const FOLDER_NAME: &str = "fonts";
pub const METADATA_FILE: &str = "metadata.json";

/// Which of the two keys a registration is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    /// Installed for everyone.
    Machine,
    /// Installed for this user only.
    User,
}

/// One value of the key: the name Windows lists a font by, and the file it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub hive: Hive,
    pub name: String,
    pub file: String,
}

/// Where a listed font is: its file, and which font of a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: PathBuf,
    pub face: u32,
}

/// The bytes of a font file, and which of its fonts was chosen (0 unless it is a collection).
#[derive(Debug, PartialEq, Eq)]
pub struct FontFile {
    pub bytes: Vec<u8>,
    pub face: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// Not a font of the last list.
    Unknown,
    /// The file is not there any more.
    Missing,
    /// Larger than `MAX_FONT_BYTES`.
    TooLarge,
    /// The file could not be read.
    Unreadable,
}

/// Whether `bytes` start with a TrueType font, TrueType collection, or OpenType header.
pub fn check_font_header(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0, 1, 0, 0])
        || bytes.starts_with(b"true")
        || bytes.starts_with(b"ttcf")
        || bytes.starts_with(b"OTTO")
}

/// Whether `bytes` start with a header valid for importing (B2-32: 00 01 00 00, true, or ttcf).
pub fn check_import_header(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true") || bytes.starts_with(b"ttcf")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ImportedMetadata {
    #[serde(default)]
    fonts: Vec<ImportedFontRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportedFontRecord {
    file: String,
    hash: String,
    faces: Vec<InspectedFont>,
}

fn read_metadata(folder: &Path) -> ImportedMetadata {
    let path = folder.join(METADATA_FILE);
    let Some(bytes) = crate::local_data::read(&path, 512 * 1024) else {
        return ImportedMetadata::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn write_metadata(folder: &Path, metadata: &ImportedMetadata) {
    let path = folder.join(METADATA_FILE);
    if let Ok(bytes) = serde_json::to_vec_pretty(metadata) {
        let _ = crate::local_data::write(&path, &bytes);
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// What a name says of the font's style, and the rest of it: the family.
fn style(name: &str) -> (String, bool, bool) {
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut end = words.len();
    let (mut bold, mut italic) = (false, false);
    while end > 1 {
        match words[end - 1].to_ascii_lowercase().as_str() {
            "bold" => bold = true,
            "italic" | "oblique" => italic = true,
            "regular" => {}
            _ => break,
        }
        end -= 1;
    }
    (words[..end].join(" "), bold, italic)
}

/// A name without the kind Windows puts after it in parentheses: `Arial (TrueType)`.
fn without_kind(name: &str) -> &str {
    let name = name.trim();
    // Nothing but a kind is no name.
    if name.starts_with('(') && name.ends_with(')') {
        return "";
    }
    match name.rfind(" (") {
        Some(at) if name.ends_with(')') => name[..at].trim_end(),
        _ => name,
    }
}

/// Whether an absolute path is safely on a local disk (e.g. `C:\...`). UNC paths (`\\server\share`),
/// device paths (`\\.\...`), verbatim UNC (`\\?\UNC\...`), drive-relative (`C:foo`), and paths
/// containing parent traversal (`..`) are strictly rejected.
fn is_safe_disk_path(path: &Path) -> bool {
    let mut components = path.components();
    // The path must start with a local disk prefix (e.g. `C:`).
    match components.next() {
        Some(Component::Prefix(prefix)) => {
            if !matches!(prefix.kind(), Prefix::Disk(_)) {
                return false;
            }
        }
        _ => return false,
    }
    // The disk prefix must be immediately followed by the root directory slash (`\`).
    if !matches!(components.next(), Some(Component::RootDir)) {
        return false;
    }
    // Traversal (`..`) or invalid components anywhere in the remaining path are disallowed.
    for component in components {
        match component {
            Component::Normal(_) => {}
            _ => return false,
        }
    }
    true
}

/// The path a registration's file is at: a full path on a local disk as it is, a bare file name
/// in the fonts folder. `None` for what is neither (a relative path with folders or `..`, or any
/// network, UNC or device path).
fn path_of(file: &str, folder: &Path) -> Option<PathBuf> {
    let trimmed = file.trim();
    if trimmed.starts_with(r"\\") || trimmed.starts_with("//") {
        return None;
    }
    let path = Path::new(trimmed);
    if is_safe_disk_path(path) {
        return Some(path.to_path_buf());
    }
    let mut parts = path.components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(name)), None) => Some(folder.join(name)),
        _ => None,
    }
}

/// The fonts the registrations stand for, in the order of the list the user sees: by family,
/// then regular, bold, italic, bold italic. A font that is in both keys is the user's own.
/// Registrations of files that are not TrueType or OpenType fonts (bitmap `.fon` fonts, Type 1)
/// and names that are empty or too long to be a name are left out.
pub fn list(registrations: &[Registration], folder: &Path) -> Vec<(FontInfo, Located)> {
    let mut found: Vec<(Hive, FontInfo, Located)> = Vec::new();
    for registration in registrations {
        let Some(path) = path_of(&registration.file, folder) else {
            continue;
        };
        let extension = path
            .extension()
            .map(|extension| extension.to_ascii_lowercase());
        let collection = match extension
            .as_deref()
            .and_then(|extension| extension.to_str())
        {
            Some("ttf" | "otf") => false,
            Some("ttc") => true,
            _ => continue,
        };
        let name = without_kind(&registration.name);
        let names: Vec<&str> = if collection {
            name.split(" & ").collect()
        } else {
            vec![name]
        };
        for (face, name) in names.into_iter().enumerate() {
            let name = clean_display_text(name, MAX_FONT_NAME_BYTES);
            if name.is_empty() || name.len() >= MAX_FONT_NAME_BYTES {
                continue;
            }
            let (family, bold, italic) = style(&name);
            let info = FontInfo {
                id: FontId(0),
                name,
                family,
                bold,
                italic,
                source: FontSource::Installed,
            };
            let located = Located {
                path: path.clone(),
                face: face as u32,
            };
            found.push((registration.hive, info, located));
        }
    }
    // The user's own first, so that the first of a name is the one that stays.
    found.sort_by_key(|(hive, ..)| *hive == Hive::Machine);
    let mut seen = std::collections::HashSet::new();
    found.retain(|(_, info, _)| seen.insert(info.name.to_lowercase()));
    found.truncate(MAX_FONTS);
    found.sort_by_key(|(_, info, _)| {
        (
            info.family.to_lowercase(),
            info.bold,
            info.italic,
            info.name.to_lowercase(),
        )
    });
    let mut taken = std::collections::HashSet::new();
    found
        .into_iter()
        .map(|(_, mut info, located)| {
            info.id = id_of(&info.name, &located, &mut taken);
            (info, located)
        })
        .collect()
}

/// A number made from the font's name and place, so that it is the same in every list; another
/// when two would be the same.
fn id_of(name: &str, located: &Located, taken: &mut std::collections::HashSet<u32>) -> FontId {
    let mut hasher = Sha256::new();
    hasher.update(name.to_lowercase().as_bytes());
    hasher.update([0]);
    hasher.update(located.path.to_string_lossy().to_lowercase().as_bytes());
    hasher.update(located.face.to_le_bytes());
    let digest = hasher.finalize();
    let mut id = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    while !taken.insert(id) {
        id = id.wrapping_add(1);
    }
    FontId(id)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Every value of the fonts key of one hive; nothing if the key is not there or cannot be read.
fn registrations_of(root: HKEY, hive: Hive) -> Vec<Registration> {
    let mut found = Vec::new();
    let key = wide(FONTS_KEY);
    let mut opened: HKEY = null_mut();
    // SAFETY: `key` ends in NUL; on success `opened` is a key that is closed below.
    let status = unsafe { RegOpenKeyExW(root, key.as_ptr(), 0, KEY_READ, &mut opened) };
    if status != ERROR_SUCCESS {
        return found;
    }
    for index in 0..u32::MAX {
        // Names and paths are short; a value that does not fit is skipped, not grown for.
        let mut name = [0u16; 512];
        let mut data = [0u16; 1024];
        let mut name_length = name.len() as u32;
        let mut data_bytes = std::mem::size_of_val(&data) as u32;
        let mut kind = 0u32;
        // SAFETY: the buffers are as long as the lengths given, and live through the call; `opened`
        // is the open key.
        let status = unsafe {
            RegEnumValueW(
                opened,
                index,
                name.as_mut_ptr(),
                &mut name_length,
                null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut data_bytes,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_MORE_DATA
            || (status == ERROR_SUCCESS && ![REG_SZ, REG_EXPAND_SZ].contains(&kind))
        {
            continue;
        }
        if status != ERROR_SUCCESS {
            break;
        }
        let name = String::from_utf16_lossy(&name[..name_length as usize]);
        let units = (data_bytes as usize / 2).min(data.len());
        let file = String::from_utf16_lossy(&data[..units]);
        let file = file.trim_end_matches('\0').to_owned();
        found.push(Registration { hive, name, file });
    }
    // SAFETY: `opened` was opened above and is not used again.
    unsafe { RegCloseKey(opened) };
    found
}

/// What Windows lists as installed, for everyone and for this user.
pub fn installed() -> Vec<Registration> {
    let mut found = registrations_of(HKEY_CURRENT_USER, Hive::User);
    found.extend(registrations_of(HKEY_LOCAL_MACHINE, Hive::Machine));
    found
}

/// The folder Windows keeps the fonts of everyone in.
fn fonts_folder() -> PathBuf {
    let windows = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    windows.join("Fonts")
}

/// The fonts of the last list, to read the file of the one the user chooses.
pub struct Fonts {
    imported_folder: Option<PathBuf>,
    listed: Mutex<Listed>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Default)]
struct Listed {
    located: HashMap<FontId, Located>,
    /// What Windows calls each, which a text box keeps to find its font again.
    names: HashMap<FontId, String>,
}

impl Fonts {
    pub fn new(imported_folder: Option<PathBuf>) -> Self {
        Self {
            imported_folder,
            listed: Mutex::new(Listed::default()),
        }
    }

    /// The installed and imported fonts. Read again every time, so that a font installed or
    /// imported a moment ago is in it.
    pub fn list(&self) -> Vec<FontInfo> {
        let installed_fonts = list(&installed(), &fonts_folder());
        let mut imported_fonts: Vec<(FontInfo, Located)> = Vec::new();
        if let Some(folder) = &self.imported_folder {
            let metadata = read_metadata(folder);
            for record in metadata.fonts {
                let path = folder.join(&record.file);
                if !path.is_file() {
                    continue;
                }
                let Ok(file) = std::fs::File::open(&path) else {
                    continue;
                };
                let Ok(meta) = file.metadata() else {
                    continue;
                };
                if meta.len() > MAX_FONT_BYTES as u64 || meta.len() < 12 {
                    continue;
                }
                let mut header = [0u8; 12];
                if file.take(12).read_exact(&mut header).is_err() || !check_import_header(&header) {
                    continue;
                }
                for face in record.faces {
                    let info = FontInfo {
                        id: FontId(0),
                        name: face.name,
                        family: face.family,
                        bold: face.bold,
                        italic: face.italic,
                        source: FontSource::Imported,
                    };
                    let located = Located {
                        path: path.clone(),
                        face: face.face,
                    };
                    imported_fonts.push((info, located));
                }
            }
        }

        // Imported fonts come first so if there is an installed font with the same name, the user's
        // imported font takes precedence.
        let mut all_fonts = Vec::with_capacity(imported_fonts.len() + installed_fonts.len());
        all_fonts.extend(imported_fonts);
        all_fonts.extend(installed_fonts);

        let mut seen = std::collections::HashSet::new();
        all_fonts.retain(|(info, _)| seen.insert(info.name.to_lowercase()));
        all_fonts.truncate(MAX_FONTS);
        all_fonts.sort_by_key(|(info, _)| {
            (
                info.family.to_lowercase(),
                info.bold,
                info.italic,
                info.name.to_lowercase(),
            )
        });

        let mut taken = std::collections::HashSet::new();
        let mut listed = self.lock();
        listed.located.clear();
        listed.names.clear();
        let mut infos = Vec::with_capacity(all_fonts.len());
        for (mut info, place) in all_fonts {
            info.id = id_of(&info.name, &place, &mut taken);
            listed.located.insert(info.id, place);
            listed.names.insert(info.id, info.name.clone());
            infos.push(info);
        }
        infos
    }

    /// Imports `source` into the app's data folder (B2-32, ADR 0018): checks extension, count,
    /// size, header, and has the worker inspect it with `skrifa`. Copies it under its SHA-256
    /// hash. Returns the updated font list.
    pub fn import(
        &self,
        source: &Path,
        documents: &Documents,
    ) -> Result<Vec<FontInfo>, FontImportRefusal> {
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .ok_or(FontImportRefusal::BadExtension)?;
        if extension != "ttf" && extension != "ttc" {
            return Err(FontImportRefusal::BadExtension);
        }
        let folder = self
            .imported_folder
            .as_ref()
            .ok_or(FontImportRefusal::Unreadable)?;

        let mut metadata = read_metadata(folder);
        metadata
            .fonts
            .retain(|record| folder.join(&record.file).is_file());

        let file = std::fs::File::open(source).map_err(|_| FontImportRefusal::Unreadable)?;
        let length = file
            .metadata()
            .map_err(|_| FontImportRefusal::Unreadable)?
            .len();
        if length > MAX_FONT_BYTES as u64 {
            return Err(FontImportRefusal::TooLarge);
        }
        let mut bytes = Vec::with_capacity(length as usize);
        file.take(MAX_FONT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FontImportRefusal::Unreadable)?;
        if bytes.len() > MAX_FONT_BYTES {
            return Err(FontImportRefusal::TooLarge);
        }
        if bytes.len() < 12 {
            return Err(FontImportRefusal::NotAFont);
        }
        if !check_import_header(&bytes) {
            if bytes.starts_with(b"OTTO") {
                return Err(FontImportRefusal::NotTrueType);
            }
            return Err(FontImportRefusal::NotAFont);
        }

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = hex(&hasher.finalize());

        let already_has = metadata.fonts.iter().any(|r| r.hash == hash);
        if !already_has && metadata.fonts.len() >= MAX_IMPORTED_FONTS {
            return Err(FontImportRefusal::TooMany);
        }

        let faces = documents
            .inspect_font(&bytes)
            .map_err(|error| match error.code {
                ErrorCode::FontInvalid => FontImportRefusal::NotAFont,
                ErrorCode::FontNotTrueType => FontImportRefusal::NotTrueType,
                ErrorCode::FontVariable => FontImportRefusal::Variable,
                ErrorCode::FontColour => FontImportRefusal::Colour,
                ErrorCode::FontNotEmbeddable => FontImportRefusal::NotEmbeddable,
                _ => FontImportRefusal::Unreadable,
            })?;
        if faces.is_empty() {
            return Err(FontImportRefusal::NotAFont);
        }

        let file_name = format!("{hash}.{extension}");
        let dest = folder.join(&file_name);
        crate::local_data::write(&dest, &bytes).map_err(|_| FontImportRefusal::Unreadable)?;

        metadata.fonts.retain(|r| r.hash != hash);
        metadata.fonts.push(ImportedFontRecord {
            file: file_name,
            hash,
            faces,
        });
        write_metadata(folder, &metadata);

        Ok(self.list())
    }

    /// Removes an imported font by its `id` (B2-32): deletes the font file from the data folder
    /// and updates metadata. Returns the updated font list.
    pub fn remove(&self, id: FontId) -> Result<Vec<FontInfo>, ()> {
        let Some(folder) = &self.imported_folder else {
            return Err(());
        };
        let located = self.lock().located.get(&id).cloned().ok_or(())?;
        if !located.path.starts_with(folder) {
            return Err(());
        }
        let _ = std::fs::remove_file(&located.path);
        let mut metadata = read_metadata(folder);
        if let Some(file_name) = located.path.file_name().and_then(|f| f.to_str()) {
            metadata.fonts.retain(|r| r.file != file_name);
            write_metadata(folder, &metadata);
        }
        Ok(self.list())
    }

    /// Removes all imported fonts from the app's data folder (B2-32). Returns the updated font list.
    pub fn remove_all(&self) -> Result<Vec<FontInfo>, ()> {
        let Some(folder) = &self.imported_folder else {
            return Err(());
        };
        let metadata = read_metadata(folder);
        for record in metadata.fonts {
            let _ = std::fs::remove_file(folder.join(record.file));
        }
        write_metadata(folder, &ImportedMetadata::default());
        Ok(self.list())
    }

    /// The file of a font of the last list.
    pub fn read(&self, id: FontId) -> Result<FontFile, FontError> {
        self.file(id).map(|(_, file)| file)
    }

    /// The name and the file of a font of the last list; of the list read again if the font is
    /// not in it (nothing listed yet in this run, or a font installed since).
    pub fn file(&self, id: FontId) -> Result<(String, FontFile), FontError> {
        match self.try_file(id) {
            Err(FontError::Unknown) => {
                self.list();
                self.try_file(id)
            }
            found => found,
        }
    }

    fn try_file(&self, id: FontId) -> Result<(String, FontFile), FontError> {
        let (name, located) = {
            let listed = self.lock();
            (
                listed.names.get(&id).cloned(),
                listed.located.get(&id).cloned(),
            )
        };
        let (Some(name), Some(located)) = (name, located) else {
            return Err(FontError::Unknown);
        };
        Ok((name, read_file(&located)?))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Listed> {
        self.listed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The bytes of a font's file, at most `MAX_FONT_BYTES` of them. Whether they are a font is for
/// the worker to say.
pub fn read_file(located: &Located) -> Result<FontFile, FontError> {
    let file = std::fs::File::open(&located.path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => FontError::Missing,
        _ => FontError::Unreadable,
    })?;
    let length = file.metadata().map_err(|_| FontError::Unreadable)?.len();
    if length > MAX_FONT_BYTES as u64 {
        return Err(FontError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(length as usize);
    // One byte more than allowed: a file that grew since `metadata` is caught too.
    file.take(MAX_FONT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FontError::Unreadable)?;
    if bytes.len() > MAX_FONT_BYTES {
        return Err(FontError::TooLarge);
    }
    Ok(FontFile {
        bytes,
        face: located.face,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(hive: Hive, name: &str, file: &str) -> Registration {
        Registration {
            hive,
            name: name.to_owned(),
            file: file.to_owned(),
        }
    }

    fn folder() -> PathBuf {
        PathBuf::from(r"C:\Windows\Fonts")
    }

    #[test]
    fn a_name_loses_its_kind_and_says_its_style() {
        let said = |name: &str| {
            let (family, bold, italic) = style(without_kind(name));
            (family, bold, italic)
        };
        let family = |name: &str, bold, italic| (name.to_owned(), bold, italic);
        assert_eq!(said("Arial (TrueType)"), family("Arial", false, false));
        assert_eq!(
            said("Arial Bold Italic (TrueType)"),
            family("Arial", true, true)
        );
        assert_eq!(
            said("Calibri Light Italic (TrueType)"),
            family("Calibri Light", false, true)
        );
        assert_eq!(
            said("Segoe UI Semibold (TrueType)"),
            family("Segoe UI Semibold", false, false)
        );
        assert_eq!(
            said("Courier New Bold Oblique (OpenType)"),
            family("Courier New", true, true)
        );
        assert_eq!(
            said("Bahnschrift Regular"),
            family("Bahnschrift", false, false)
        );
        // A name that is only a style is kept whole.
        assert_eq!(said("Bold"), family("Bold", false, false));
    }

    #[test]
    fn a_collection_gives_a_font_for_each_name_in_the_order_of_the_file() {
        let found = list(
            &[registration(
                Hive::Machine,
                "Microsoft JhengHei & Microsoft JhengHei UI (TrueType)",
                "msjh.ttc",
            )],
            &folder(),
        );
        let faces: Vec<_> = found
            .iter()
            .map(|(info, located)| (info.name.as_str(), located.face))
            .collect();
        assert_eq!(
            faces,
            [("Microsoft JhengHei", 0), ("Microsoft JhengHei UI", 1)]
        );
        assert!(
            found
                .iter()
                .all(|(_, located)| located.path == folder().join("msjh.ttc"))
        );
        // A font that is not a collection is not cut at an ampersand.
        let single = list(
            &[registration(
                Hive::Machine,
                "AT&T Sans & More (TrueType)",
                "att.ttf",
            )],
            &folder(),
        );
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn system_fonts_are_in_the_fonts_folder_and_the_users_have_their_full_path() {
        let user_file = r"C:\Users\someone\AppData\Local\Microsoft\Windows\Fonts\Mine.ttf";
        let found = list(
            &[
                registration(Hive::Machine, "Arial (TrueType)", "arial.ttf"),
                registration(Hive::User, "Mine (TrueType)", user_file),
            ],
            &folder(),
        );
        let place = |name: &str| {
            found
                .iter()
                .find(|(info, _)| info.name == name)
                .unwrap()
                .1
                .path
                .clone()
        };
        assert_eq!(place("Arial"), folder().join("arial.ttf"));
        assert_eq!(place("Mine"), PathBuf::from(user_file));
        assert!(
            found
                .iter()
                .all(|(info, _)| info.source == FontSource::Installed)
        );
    }

    #[test]
    fn what_is_not_a_font_file_or_cannot_be_a_name_is_left_out() {
        let found = list(
            &[
                registration(Hive::Machine, "Courier 10,12,15 (VGA res)", "COURE.FON"),
                registration(Hive::Machine, "Type One (Type 1)", "one.pfb"),
                registration(Hive::Machine, "Escape (TrueType)", r"..\..\evil.ttf"),
                registration(Hive::Machine, "Folder (TrueType)", r"sub\dir.ttf"),
                registration(Hive::Machine, " (TrueType)", "empty.ttf"),
                registration(
                    Hive::Machine,
                    &format!("{} (TrueType)", "x".repeat(200)),
                    "long.ttf",
                ),
                registration(Hive::Machine, "No File (TrueType)", ""),
                registration(Hive::Machine, "Upper (TrueType)", "UPPER.TTF"),
                registration(Hive::Machine, "Open (OpenType)", "open.otf"),
            ],
            &folder(),
        );
        let names: Vec<_> = found.iter().map(|(info, _)| info.name.as_str()).collect();
        assert_eq!(names, ["Open", "Upper"]);
    }

    #[test]
    fn a_name_is_plain_text() {
        let found = list(
            &[registration(
                Hive::Machine,
                "Tab\tFont\u{202e}Name\n (TrueType)",
                "a.ttf",
            )],
            &folder(),
        );
        // The tab becomes a space; the right-to-left override is not text and is dropped.
        assert_eq!(found[0].0.name, "Tab FontName");
    }

    #[test]
    fn a_font_in_both_keys_is_the_users_own_and_the_list_is_in_order() {
        let found = list(
            &[
                registration(Hive::Machine, "Zed Bold (TrueType)", "zb.ttf"),
                registration(Hive::Machine, "Zed (TrueType)", "z.ttf"),
                registration(Hive::Machine, "Alpha (TrueType)", "a.ttf"),
                registration(Hive::User, "alpha (TrueType)", r"C:\Mine\a.ttf"),
                registration(Hive::Machine, "Zed Bold Italic (TrueType)", "zbi.ttf"),
                registration(Hive::Machine, "Zed Italic (TrueType)", "zi.ttf"),
            ],
            &folder(),
        );
        let order: Vec<_> = found
            .iter()
            .map(|(info, _)| (info.name.as_str(), info.bold, info.italic))
            .collect();
        assert_eq!(
            order,
            [
                ("alpha", false, false),
                ("Zed", false, false),
                ("Zed Italic", false, true),
                ("Zed Bold", true, false),
                ("Zed Bold Italic", true, true),
            ]
        );
        assert_eq!(found[0].1.path, PathBuf::from(r"C:\Mine\a.ttf"));
    }

    #[test]
    fn ids_are_the_same_in_every_list_and_never_repeat() {
        let registrations: Vec<_> = (0..300)
            .map(|index| {
                registration(
                    Hive::Machine,
                    &format!("Font {index} (TrueType)"),
                    &format!("f{index}.ttf"),
                )
            })
            .collect();
        let first = list(&registrations, &folder());
        let again = list(&registrations, &folder());
        assert_eq!(first, again);
        let ids: std::collections::HashSet<_> = first.iter().map(|(info, _)| info.id).collect();
        assert_eq!(ids.len(), 300);
    }

    #[test]
    fn the_real_registry_lists_fonts_whose_files_can_be_read() {
        let fonts = Fonts::default();
        let list = fonts.list();
        assert!(list.len() >= 20, "{} fonts", list.len());
        assert!(
            list.iter()
                .all(|info| !info.name.is_empty() && info.name.len() < MAX_FONT_NAME_BYTES)
        );
        // Some font of this computer is there and starts as a TrueType file or collection does.
        let readable = list
            .iter()
            .find_map(|info| fonts.read(info.id).ok())
            .expect("a readable font");
        assert!(
            [&[0, 1, 0, 0][..], b"true", b"ttcf", b"OTTO"].contains(&&readable.bytes[..4]),
            "{:?}",
            &readable.bytes[..4]
        );
        // The ids are the ones of this list; one from nowhere is not a font.
        assert_eq!(
            fonts
                .read(FontId(u32::MAX - 1))
                .err()
                .or(Some(FontError::Unknown)),
            Some(FontError::Unknown)
        );
        let known: std::collections::HashSet<_> = list.iter().map(|info| info.id).collect();
        let unknown = (0..=u32::MAX)
            .map(FontId)
            .find(|id| !known.contains(id))
            .unwrap();
        assert_eq!(fonts.read(unknown), Err(FontError::Unknown));
    }

    #[test]
    fn a_font_comes_with_its_name_and_its_file_also_when_nothing_was_listed_yet() {
        // The real registry: a font is picked from one list, and asked for from a service that
        // has not listed anything (the app was restarted, or a font was installed since).
        let list = Fonts::default().list();
        let listed = list
            .iter()
            .find(|info| info.name.eq_ignore_ascii_case("Arial"))
            .or_else(|| list.first())
            .expect("an installed font");
        let fresh = Fonts::default();
        let (name, file) = fresh.file(listed.id).expect("the font");
        assert_eq!(name, listed.name);
        assert!(
            file.bytes.starts_with(&[0, 1, 0, 0])
                || file.bytes.starts_with(b"true")
                || file.bytes.starts_with(b"ttcf")
                || file.bytes.starts_with(b"OTTO"),
            "{:?}",
            &file.bytes[..4]
        );
        assert_eq!(fresh.read(listed.id).expect("again").bytes, file.bytes);
    }

    #[test]
    fn a_file_that_is_gone_or_too_big_is_refused_and_one_in_between_is_read() {
        let folder = std::env::temp_dir().join(format!("pdf-reader-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let at = |name: &str, face| Located {
            path: folder.join(name),
            face,
        };
        assert_eq!(read_file(&at("missing.ttf", 0)), Err(FontError::Missing));
        std::fs::write(folder.join("small.ttc"), b"ttcf....").unwrap();
        assert_eq!(
            read_file(&at("small.ttc", 2)),
            Ok(FontFile {
                bytes: b"ttcf....".to_vec(),
                face: 2
            })
        );
        // A sparse file of one byte over the limit: its size alone refuses it, nothing is read.
        let big = std::fs::File::create(folder.join("big.ttf")).unwrap();
        big.set_len(MAX_FONT_BYTES as u64 + 1).unwrap();
        drop(big);
        assert_eq!(read_file(&at("big.ttf", 0)), Err(FontError::TooLarge));
        // A folder is no file.
        std::fs::create_dir_all(folder.join("dir.ttf")).unwrap();
        assert_eq!(read_file(&at("dir.ttf", 0)), Err(FontError::Unreadable));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn unc_network_device_and_traversal_paths_are_strictly_rejected() {
        let found = list(
            &[
                registration(
                    Hive::User,
                    "UNC Backslash (TrueType)",
                    r"\\attacker\share\font.ttf",
                ),
                registration(
                    Hive::User,
                    "UNC Slash (TrueType)",
                    "//attacker/share/font.ttf",
                ),
                registration(
                    Hive::User,
                    "Verbatim UNC (TrueType)",
                    r"\\?\UNC\attacker\share\font.ttf",
                ),
                registration(Hive::User, "Device Path (TrueType)", r"\\.\COM1"),
                registration(Hive::User, "Named Pipe (TrueType)", r"\\.\pipe\test.ttf"),
                registration(
                    Hive::User,
                    "Verbatim Disk (TrueType)",
                    r"\\?\C:\Windows\Fonts\evil.ttf",
                ),
                registration(
                    Hive::User,
                    "Absolute Traversal (TrueType)",
                    r"C:\Windows\Fonts\..\..\evil.ttf",
                ),
                registration(Hive::User, "Drive Relative (TrueType)", r"C:evil.ttf"),
                registration(Hive::User, "Root Relative (TrueType)", r"\evil.ttf"),
                registration(
                    Hive::User,
                    "Valid Disk Font (TrueType)",
                    r"C:\Users\someone\AppData\Local\Microsoft\Windows\Fonts\Valid.ttf",
                ),
            ],
            &folder(),
        );
        let names: Vec<_> = found.iter().map(|(info, _)| info.name.as_str()).collect();
        assert_eq!(names, ["Valid Disk Font"]);
    }

    #[test]
    fn import_header_checks() {
        assert!(check_import_header(&[0, 1, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7]));
        assert!(check_import_header(b"true\0\0\0\0\0\0\0\0"));
        assert!(check_import_header(b"ttcf\0\0\0\0\0\0\0\0"));
        assert!(!check_import_header(b"OTTO\0\0\0\0\0\0\0\0"));
        assert!(!check_import_header(b"wOFF\0\0\0\0\0\0\0\0"));
        assert!(!check_import_header(b"bad header!!"));
    }

    #[test]
    fn import_refuses_bad_extension_unreadable_and_too_large() {
        let temp =
            std::env::temp_dir().join(format!("pdf-reader-font-test-{}", std::process::id()));
        let imported = temp.join("imported");
        std::fs::create_dir_all(&imported).unwrap();
        let fonts = Fonts::new(Some(imported.clone()));
        let documents = Documents::new(PathBuf::from("dummy-worker.exe"));

        // Bad extension (.otf or .txt)
        let txt = temp.join("test.txt");
        std::fs::write(&txt, b"hello").unwrap();
        assert_eq!(
            fonts.import(&txt, &documents),
            Err(FontImportRefusal::BadExtension)
        );

        let otf = temp.join("test.otf");
        std::fs::write(&otf, b"OTTO").unwrap();
        assert_eq!(
            fonts.import(&otf, &documents),
            Err(FontImportRefusal::BadExtension)
        );

        // Missing file
        let missing = temp.join("missing.ttf");
        assert_eq!(
            fonts.import(&missing, &documents),
            Err(FontImportRefusal::Unreadable)
        );

        // Header check: starts with OTTO
        let fake_otto = temp.join("fake_cff.ttf");
        std::fs::write(&fake_otto, b"OTTO12345678").unwrap();
        assert_eq!(
            fonts.import(&fake_otto, &documents),
            Err(FontImportRefusal::NotTrueType)
        );

        // Header check: random bytes
        let fake_bad = temp.join("fake_bad.ttf");
        std::fs::write(&fake_bad, b"badheader123").unwrap();
        assert_eq!(
            fonts.import(&fake_bad, &documents),
            Err(FontImportRefusal::NotAFont)
        );

        // Too large
        let big = temp.join("big.ttf");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FONT_BYTES as u64 + 1).unwrap();
        drop(f);
        assert_eq!(
            fonts.import(&big, &documents),
            Err(FontImportRefusal::TooLarge)
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn metadata_round_trip_and_font_lifecycle() {
        let temp =
            std::env::temp_dir().join(format!("pdf-reader-font-lifecycle-{}", std::process::id()));
        let imported = temp.join("imported");
        std::fs::create_dir_all(&imported).unwrap();
        let fonts = Fonts::new(Some(imported.clone()));

        // Initially no imported fonts
        let initial_metadata = read_metadata(&imported);
        assert!(initial_metadata.fonts.is_empty());

        // Write a fake imported font record
        let record = ImportedFontRecord {
            file: "fakehash.ttf".to_owned(),
            hash: "fakehash".to_owned(),
            faces: vec![InspectedFont {
                face: 0,
                name: "Custom Font".to_owned(),
                family: "Custom Font".to_owned(),
                bold: false,
                italic: false,
            }],
        };
        std::fs::write(
            imported.join("fakehash.ttf"),
            [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let metadata = ImportedMetadata {
            fonts: vec![record],
        };
        write_metadata(&imported, &metadata);

        let read_back = read_metadata(&imported);
        assert_eq!(read_back.fonts.len(), 1);
        assert_eq!(read_back.fonts[0].file, "fakehash.ttf");

        // Listed fonts should include Custom Font with Source::Imported
        let list = fonts.list();
        let custom = list
            .iter()
            .find(|f| f.name == "Custom Font")
            .expect("Custom Font in list");
        assert_eq!(custom.source, FontSource::Imported);

        // Remove by id
        let after_remove = fonts.remove(custom.id).expect("remove succeeded");
        assert!(after_remove.iter().all(|f| f.name != "Custom Font"));
        assert!(!imported.join("fakehash.ttf").exists());

        let meta_after = read_metadata(&imported);
        assert!(meta_after.fonts.is_empty());

        // Test remove_all
        std::fs::write(
            imported.join("fake1.ttf"),
            [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        std::fs::write(
            imported.join("fake2.ttf"),
            [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let multi_meta = ImportedMetadata {
            fonts: vec![
                ImportedFontRecord {
                    file: "fake1.ttf".to_owned(),
                    hash: "h1".to_owned(),
                    faces: vec![InspectedFont {
                        face: 0,
                        name: "Font 1".to_owned(),
                        family: "Font 1".to_owned(),
                        bold: false,
                        italic: false,
                    }],
                },
                ImportedFontRecord {
                    file: "fake2.ttf".to_owned(),
                    hash: "h2".to_owned(),
                    faces: vec![InspectedFont {
                        face: 0,
                        name: "Font 2".to_owned(),
                        family: "Font 2".to_owned(),
                        bold: false,
                        italic: false,
                    }],
                },
            ],
        };
        write_metadata(&imported, &multi_meta);
        let list_multi = fonts.list();
        assert!(list_multi.iter().any(|f| f.name == "Font 1"));
        assert!(list_multi.iter().any(|f| f.name == "Font 2"));

        fonts.remove_all().expect("remove all succeeded");
        assert!(!imported.join("fake1.ttf").exists());
        assert!(!imported.join("fake2.ttf").exists());
        let meta_empty = read_metadata(&imported);
        assert!(meta_empty.fonts.is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }
}
