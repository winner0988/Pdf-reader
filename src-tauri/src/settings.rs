//! The user's settings (B2-12): `settings.json` in the app's local data folder, next to the
//! recent files list (docs/architecture/local-data.md). Only the main process reads and writes
//! the file; the frontend gets and replaces the whole set through typed commands.

use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use ipc_contract::types::{Settings, ThemePreference};
use serde::{Deserialize, Serialize};

use crate::local_data;

/// The file in the data folder.
pub const FILE_NAME: &str = "settings.json";
/// A larger file was not written by the app: it is ignored.
const MAX_FILE_BYTES: u64 = 64 * 1024;
const VERSION: u32 = 1;

/// `settings.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stored {
    version: u32,
    theme: ThemePreference,
    record_recent_files: bool,
}

pub struct SettingsStore {
    /// `settings.json`, or `None` when there is no data folder: then nothing is kept between runs.
    file: Option<PathBuf>,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Reads the settings now (the file is small, and the window needs its theme at once). A
    /// missing file, or one the app did not write, gives the defaults; the next change
    /// overwrites it.
    pub fn load(file: Option<PathBuf>) -> Self {
        let current = file
            .as_deref()
            .and_then(|file| local_data::read(file, MAX_FILE_BYTES))
            .and_then(|json| serde_json::from_slice::<Stored>(&json).ok())
            .filter(|stored| stored.version == VERSION)
            .map_or_else(Settings::default, |stored| Settings {
                theme: stored.theme,
                record_recent_files: stored.record_recent_files,
            });
        Self {
            file,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the settings. They apply at once, even if the file cannot be written; the error
    /// says they will not survive a restart.
    pub fn set(&self, settings: Settings) -> io::Result<()> {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = settings;
        let Some(file) = &self.file else {
            return Ok(());
        };
        let stored = Stored {
            version: VERSION,
            theme: settings.theme,
            record_recent_files: settings.record_recent_files,
        };
        let json = serde_json::to_vec_pretty(&stored).map_err(io::Error::other)?;
        local_data::write(file, &json)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pdf-reader-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn starts_from_the_defaults_and_keeps_changes_for_the_next_run() {
        let dir = folder("keep");
        let file = dir.join(FILE_NAME);
        let store = SettingsStore::load(Some(file.clone()));
        assert_eq!(store.get(), Settings::default());

        let dark = Settings {
            theme: ThemePreference::Dark,
            record_recent_files: false,
        };
        store.set(dark).unwrap();
        assert_eq!(store.get(), dark);
        assert_eq!(SettingsStore::load(Some(file)).get(), dark);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_the_app_did_not_write_gives_the_defaults() {
        let dir = folder("foreign");
        let file = dir.join(FILE_NAME);
        fs::create_dir_all(&dir).unwrap();
        for content in [
            "not json".to_owned(),
            r#"{"version": 2, "theme": "dark", "recordRecentFiles": false}"#.to_owned(),
            r#"{"version": 1, "theme": "dark", "recordRecentFiles": false, "extra": 1}"#.to_owned(),
            r#"{"version": 1, "theme": "sepia", "recordRecentFiles": false}"#.to_owned(),
            format!(
                r#"{{"version": 1, "theme": "dark", "recordRecentFiles": false, "x": "{}"}}"#,
                "a".repeat(70_000)
            ),
        ] {
            fs::write(&file, content).unwrap();
            assert_eq!(
                SettingsStore::load(Some(file.clone())).get(),
                Settings::default()
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_data_folder_the_settings_last_for_the_run() {
        let store = SettingsStore::load(None);
        let light = Settings {
            theme: ThemePreference::Light,
            record_recent_files: true,
        };
        store.set(light).unwrap();
        assert_eq!(store.get(), light);
    }
}
