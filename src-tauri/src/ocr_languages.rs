//! The languages that scanned pages can be recognised in (B2-10, ADR 0015,
//! docs/architecture/ocr.md): the data that came with the app, and the data the user imported.
//! A language is a Tesseract `.traineddata` file; nothing is downloaded. Only the main process
//! opens these files: the worker is sent their bytes.
//!
//! The data that came with the app is in a `tessdata` folder next to it (the installer puts it
//! there); imported data is kept in a `tessdata` folder in the app's data folder. Each file is
//! named for its language (`eng.traineddata`), and the name is checked wherever one is read.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use ipc_contract::limits::{MAX_IMPORTED_LANGUAGES, MAX_LANGUAGE_DATA_BYTES};
use ipc_contract::ocr::{LanguageDataError, check_language_data, is_language_name};
use ipc_contract::types::{LanguageRefusal, OcrLanguage, OcrLanguages};

use crate::local_data;

/// The folder of language data, in the data folder and next to the app.
pub const FOLDER_NAME: &str = "tessdata";

const EXTENSION: &str = ".traineddata";

/// The languages the app chooses between when the settings leave it to it, by the language of its
/// interface (Traditional Chinese, whose data reads English too) and then at all.
const AUTOMATIC: [&str; 2] = ["chi_tra", "eng"];

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Not a language code, or no such language is installed.
    Missing,
    Unreadable,
    /// The file is not language data (or is too large).
    Refused(LanguageDataError),
}

pub struct Languages {
    /// Where the data that came with the app is; none if it could not be found.
    bundled: Option<PathBuf>,
    /// Where imported data goes; none if there is no data folder: then nothing can be imported.
    imported: Option<PathBuf>,
}

impl Languages {
    pub fn new(bundled: Option<PathBuf>, imported: Option<PathBuf>) -> Self {
        Self { bundled, imported }
    }

    /// Every installed language: those that came with the app, then the imported ones, each in
    /// order of its code. An imported file with the code of one that came with the app is not
    /// listed (it would never be used).
    pub fn list(&self) -> OcrLanguages {
        let mut languages = Vec::new();
        if let Some(folder) = &self.bundled {
            languages.extend(installed(folder, true));
        }
        if let Some(folder) = &self.imported {
            let taken: Vec<String> = languages
                .iter()
                .map(|l: &OcrLanguage| l.code.clone())
                .collect();
            languages.extend(
                installed(folder, false)
                    .into_iter()
                    .filter(|language| !taken.contains(&language.code))
                    .take(MAX_IMPORTED_LANGUAGES as usize),
            );
        }
        let automatic = AUTOMATIC
            .iter()
            .find(|code| languages.iter().any(|l| l.code == **code))
            .map(|code| (*code).to_owned())
            .or_else(|| languages.first().map(|l| l.code.clone()));
        OcrLanguages {
            languages,
            automatic,
        }
    }

    /// The code of the language to read in: `preferred` if that is installed, otherwise the one
    /// the app chooses. None when no language is installed.
    pub fn choose(&self, preferred: Option<&str>) -> Option<String> {
        let list = self.list();
        preferred
            .filter(|code| list.languages.iter().any(|l| l.code == *code))
            .map(str::to_owned)
            .or(list.automatic)
    }

    /// The data of language `code`, as the worker is sent it: read here, within the size limit
    /// and checked to be language data.
    pub fn read(&self, code: &str) -> Result<Vec<u8>, ReadError> {
        if !is_language_name(code) {
            return Err(ReadError::Missing);
        }
        let name = format!("{code}{EXTENSION}");
        let file = [&self.bundled, &self.imported]
            .into_iter()
            .flatten()
            .map(|folder| folder.join(&name))
            .find(|path| path.is_file())
            .ok_or(ReadError::Missing)?;
        let data = read_limited(&file).map_err(|_| ReadError::Unreadable)?;
        check_language_data(&data).map_err(ReadError::Refused)?;
        Ok(data)
    }
}

/// The files of `folder` that are named for a language and a size that could be one, by code.
fn installed(folder: &Path, bundled: bool) -> Vec<OcrLanguage> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<OcrLanguage> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let code = name.strip_suffix(EXTENSION)?;
            let metadata = entry.metadata().ok()?;
            (is_language_name(code)
                && metadata.is_file()
                && metadata.len() > 0
                && metadata.len() <= MAX_LANGUAGE_DATA_BYTES as u64)
                .then(|| OcrLanguage {
                    code: code.to_owned(),
                    bundled,
                    bytes: metadata.len(),
                })
        })
        .collect();
    found.sort_by(|a, b| a.code.cmp(&b.code));
    found
}

/// All of `file`, if it is at most `MAX_LANGUAGE_DATA_BYTES`; `InvalidData` if it is more.
fn read_limited(file: &Path) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(file)?
        .take(MAX_LANGUAGE_DATA_BYTES as u64 + 1)
        .read_to_end(&mut data)?;
    if data.len() > MAX_LANGUAGE_DATA_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(data)
}

impl Languages {
    /// Imports the file `source`, which the user picked in a dialog, as a language of the app's:
    /// it is named for its language, is not larger than `MAX_LANGUAGE_DATA_BYTES`, and is
    /// language data (`check_language_data`). It is copied into the app's data folder; an
    /// imported language of the same code is replaced. A language that came with the app cannot
    /// be replaced.
    pub fn import(&self, source: &Path) -> Result<(), LanguageRefusal> {
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LanguageRefusal::BadName)?;
        let code_len = file_name
            .len()
            .checked_sub(EXTENSION.len())
            .filter(|&at| {
                file_name
                    .get(at..)
                    .is_some_and(|suffix| suffix.eq_ignore_ascii_case(EXTENSION))
            })
            .ok_or(LanguageRefusal::BadName)?;
        let code = &file_name[..code_len];
        if !is_language_name(code) {
            return Err(LanguageRefusal::BadName);
        }
        let came_with_the_app = self
            .bundled
            .as_deref()
            .is_some_and(|folder| installed(folder, true).iter().any(|l| l.code == code));
        if came_with_the_app {
            return Err(LanguageRefusal::NameTaken);
        }
        let folder = self
            .imported
            .as_deref()
            .ok_or(LanguageRefusal::Unreadable)?;
        let present = installed(folder, false);
        if present.len() >= MAX_IMPORTED_LANGUAGES as usize
            && !present.iter().any(|language| language.code == code)
        {
            return Err(LanguageRefusal::TooMany);
        }
        let data = read_limited(source).map_err(|error| match error.kind() {
            std::io::ErrorKind::InvalidData => LanguageRefusal::TooLarge,
            _ => LanguageRefusal::Unreadable,
        })?;
        check_language_data(&data).map_err(|error| match error {
            LanguageDataError::TooLarge => LanguageRefusal::TooLarge,
            _ => LanguageRefusal::NotLanguageData,
        })?;
        local_data::write(&folder.join(format!("{code}{EXTENSION}")), &data)
            .map_err(|_| LanguageRefusal::Unreadable)
    }

    /// Removes the imported language `code`; whether there was one. The languages that came with
    /// the app cannot be removed.
    pub fn remove(&self, code: &str) -> bool {
        let Some(folder) = &self.imported else {
            return false;
        };
        if !is_language_name(code) {
            return false;
        }
        let file = folder.join(format!("{code}{EXTENSION}"));
        file.is_file() && fs::remove_file(file).is_ok()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pdf-reader-languages-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Language data that passes the check: 24 entries, the three of an LSTM model among them.
    pub(crate) fn language_data(extra: u8) -> Vec<u8> {
        let count = 24usize;
        let mut out = i32::try_from(count).unwrap().to_le_bytes().to_vec();
        let mut at = 4 + 8 * count;
        for index in 0..count {
            let offset: i64 = if [17, 21, 22].contains(&index) {
                let here = at;
                at += 4;
                i64::try_from(here).unwrap()
            } else {
                -1
            };
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.resize(at, extra);
        out
    }

    fn install(folder: &Path, code: &str, data: &[u8]) -> PathBuf {
        let file = folder.join(format!("{code}.traineddata"));
        fs::write(&file, data).unwrap();
        file
    }

    fn codes(list: &OcrLanguages) -> Vec<(&str, bool)> {
        list.languages
            .iter()
            .map(|l| (l.code.as_str(), l.bundled))
            .collect()
    }

    #[test]
    fn lists_what_came_with_the_app_then_the_imported_ones() {
        let (bundled, imported) = (folder("list-b"), folder("list-i"));
        install(&bundled, "eng", &language_data(1));
        install(&bundled, "chi_tra", &language_data(2));
        install(&imported, "deu", &language_data(3));
        install(&imported, "eng", &language_data(4)); // shadowed: not listed
        // Not languages: other names, empty files, folders.
        install(&bundled, "readme", b"x");
        fs::write(bundled.join("notes.txt"), b"x").unwrap();
        fs::write(imported.join("empty.traineddata"), b"").unwrap();
        fs::create_dir(imported.join("dir.traineddata")).unwrap();
        fs::write(imported.join("1bad.traineddata"), b"x").unwrap();
        let list = Languages::new(Some(bundled.clone()), Some(imported.clone())).list();
        assert_eq!(
            codes(&list),
            [
                ("chi_tra", true),
                ("eng", true),
                ("readme", true),
                ("deu", false)
            ]
        );
        assert_eq!(list.languages[1].bytes, language_data(1).len() as u64);
        assert_eq!(list.automatic.as_deref(), Some("chi_tra"));
        assert_eq!(ipc_contract::validate::Validate::validate(&list), Ok(()));
        let _ = fs::remove_dir_all(bundled);
        let _ = fs::remove_dir_all(imported);
    }

    #[test]
    fn the_automatic_language_is_chinese_then_english_then_any() {
        let (bundled, imported) = (folder("auto-b"), folder("auto-i"));
        let languages = Languages::new(Some(bundled.clone()), Some(imported.clone()));
        assert_eq!(languages.list().automatic, None);
        assert_eq!(languages.choose(Some("eng")), None);
        install(&imported, "deu", &language_data(1));
        assert_eq!(languages.list().automatic.as_deref(), Some("deu"));
        install(&bundled, "eng", &language_data(1));
        assert_eq!(languages.list().automatic.as_deref(), Some("eng"));
        install(&bundled, "chi_tra", &language_data(1));
        assert_eq!(languages.list().automatic.as_deref(), Some("chi_tra"));
        // A preference that is installed wins; one that is not gives the automatic one.
        assert_eq!(languages.choose(Some("deu")).as_deref(), Some("deu"));
        assert_eq!(languages.choose(Some("fra")).as_deref(), Some("chi_tra"));
        assert_eq!(languages.choose(None).as_deref(), Some("chi_tra"));
        let _ = fs::remove_dir_all(bundled);
        let _ = fs::remove_dir_all(imported);
    }

    #[test]
    fn data_is_read_by_code_and_checked() {
        let (bundled, imported) = (folder("read-b"), folder("read-i"));
        let languages = Languages::new(Some(bundled.clone()), Some(imported.clone()));
        install(&bundled, "eng", &language_data(1));
        install(&imported, "deu", &language_data(2));
        assert_eq!(languages.read("eng").unwrap(), language_data(1));
        assert_eq!(languages.read("deu").unwrap(), language_data(2));
        assert_eq!(languages.read("fra"), Err(ReadError::Missing));
        for not_a_code in ["", "../eng", "eng.traineddata", "a/b", "C:x"] {
            assert_eq!(
                languages.read(not_a_code),
                Err(ReadError::Missing),
                "{not_a_code}"
            );
        }
        // Files that were put there by hand are checked when they are read.
        install(&imported, "bad", b"not language data");
        install(&imported, "legacy", &[24, 0, 0, 0]);
        assert!(matches!(languages.read("bad"), Err(ReadError::Refused(_))));
        assert!(matches!(
            languages.read("legacy"),
            Err(ReadError::Refused(_))
        ));
        let _ = fs::remove_dir_all(bundled);
        let _ = fs::remove_dir_all(imported);
    }

    #[test]
    fn the_data_that_comes_with_the_app_is_language_data() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(FOLDER_NAME);
        let languages = Languages::new(Some(bundled), None);
        for code in ["eng", "chi_tra"] {
            assert!(languages.read(code).is_ok(), "{code}");
        }
        assert_eq!(languages.list().automatic.as_deref(), Some("chi_tra"));
    }

    #[test]
    fn importing_copies_language_data_and_refuses_the_rest() {
        let (bundled, imported, source) = (folder("imp-b"), folder("imp-i"), folder("imp-s"));
        let languages = Languages::new(Some(bundled.clone()), Some(imported.clone()));
        install(&bundled, "eng", &language_data(1));

        let good = install(&source, "deu", &language_data(5));
        assert_eq!(languages.import(&good), Ok(()));
        assert_eq!(languages.read("deu").unwrap(), language_data(5));
        // The same code again replaces it.
        let newer = install(&source, "deu", &language_data(6));
        assert_eq!(languages.import(&newer), Ok(()));
        assert_eq!(languages.read("deu").unwrap(), language_data(6));
        assert!(
            !imported.join("deu.json.tmp").exists(),
            "no temporary file is left"
        );

        assert_eq!(
            languages.import(&install(&source, "eng", &language_data(7))),
            Err(LanguageRefusal::NameTaken)
        );
        assert_eq!(languages.read("eng").unwrap(), language_data(1));
        assert_eq!(
            languages.import(&install(&source, "fra", b"not language data")),
            Err(LanguageRefusal::NotLanguageData)
        );
        assert_eq!(
            languages.import(&install(&source, "spa", &[])),
            Err(LanguageRefusal::NotLanguageData)
        );
        assert_eq!(
            languages.import(&install(&source, "ita", &language_data(0)[..60])),
            Err(LanguageRefusal::NotLanguageData)
        );
        for name in [
            "1abc.traineddata",
            "a b.traineddata",
            "plain.txt",
            "traineddata",
            ".traineddata",
        ] {
            let file = source.join(name);
            fs::write(&file, language_data(1)).unwrap();
            assert_eq!(
                languages.import(&file),
                Err(LanguageRefusal::BadName),
                "{name}"
            );
        }
        assert_eq!(
            languages.import(&source.join("missing.traineddata")),
            Err(LanguageRefusal::Unreadable)
        );
        // The extension may be in any case.
        let upper = source.join("por.TRAINEDDATA");
        fs::write(&upper, language_data(8)).unwrap();
        assert_eq!(languages.import(&upper), Ok(()));
        assert_eq!(languages.read("por").unwrap(), language_data(8));
        // Nothing refused was kept.
        let list = languages.list();
        assert_eq!(
            codes(&list),
            [("eng", true), ("deu", false), ("por", false)]
        );
        for dir in [bundled, imported, source] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn data_over_the_limit_is_refused_and_so_is_the_one_after_the_last() {
        let (imported, source) = (folder("big-i"), folder("big-s"));
        let languages = Languages::new(None, Some(imported.clone()));
        let big = source.join("big.traineddata");
        let file = fs::File::create(&big).unwrap();
        file.set_len(MAX_LANGUAGE_DATA_BYTES as u64 + 1).unwrap();
        drop(file);
        assert_eq!(languages.import(&big), Err(LanguageRefusal::TooLarge));

        for index in 0..MAX_IMPORTED_LANGUAGES {
            let file = install(&source, &format!("l{index}"), &language_data(1));
            assert_eq!(languages.import(&file), Ok(()), "{index}");
        }
        let one_more = install(&source, "more", &language_data(1));
        assert_eq!(languages.import(&one_more), Err(LanguageRefusal::TooMany));
        // A language that is there already can be replaced; and room is made by removing one.
        assert_eq!(languages.import(&source.join("l3.traineddata")), Ok(()));
        assert!(languages.remove("l0"));
        assert_eq!(languages.import(&one_more), Ok(()));
        let _ = fs::remove_dir_all(imported);
        let _ = fs::remove_dir_all(source);
    }

    #[test]
    fn without_a_data_folder_nothing_is_imported_and_only_imported_languages_are_removed() {
        let (bundled, imported, source) = (folder("rm-b"), folder("rm-i"), folder("rm-s"));
        install(&bundled, "eng", &language_data(1));
        let without = Languages::new(Some(bundled.clone()), None);
        let file = install(&source, "deu", &language_data(1));
        assert_eq!(without.import(&file), Err(LanguageRefusal::Unreadable));
        assert!(!without.remove("deu"));

        let languages = Languages::new(Some(bundled.clone()), Some(imported.clone()));
        assert_eq!(languages.import(&file), Ok(()));
        assert!(
            !languages.remove("eng"),
            "those that came with the app stay"
        );
        assert!(languages.read("eng").is_ok());
        assert!(!languages.remove("../eng") && !languages.remove("nothing"));
        assert!(languages.remove("deu"));
        assert!(!languages.remove("deu"));
        assert_eq!(codes(&languages.list()), [("eng", true)]);
        for dir in [bundled, imported, source] {
            let _ = fs::remove_dir_all(dir);
        }
    }
}
