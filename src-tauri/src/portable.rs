//! Portable mode preflight, directory resolution, and exit cleanup (#449 POC, ADR 0034).

#![cfg(windows)]
// Win32 volume queries and env setup in preflight before any worker threads or WebView start.
#![allow(unsafe_code)]

#[cfg(any(feature = "portable", test))]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(any(feature = "portable", test))]
const DRIVE_FIXED: u32 = 3;
#[cfg(any(feature = "portable", test))]
const DRIVE_REMOTE: u32 = 4;
#[cfg(any(feature = "portable", test))]
const DRIVE_RAMDISK: u32 = 6;

#[cfg(any(feature = "portable", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumePolicy {
    /// NTFS / ReFS: full AppContainer + Low integrity + Restricted Token sandbox.
    AppContainer,
    /// exFAT / FAT32 / FAT: no DACL, keep Low integrity + Job Object + Restricted Token without AppContainer.
    NoAppContainer,
    /// Remote/network shares, optical drives, or unknown file systems: refuse to run.
    Unsupported,
}

#[cfg(any(feature = "portable", test))]
pub fn classify_volume(drive_type: u32, fs_name: &str) -> VolumePolicy {
    match drive_type {
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK => {
            match fs_name.to_ascii_uppercase().as_str() {
                "NTFS" | "REFS" => VolumePolicy::AppContainer,
                "FAT32" | "EXFAT" | "FAT" => VolumePolicy::NoAppContainer,
                _ => VolumePolicy::Unsupported,
            }
        }
        _ => VolumePolicy::Unsupported,
    }
}

#[cfg(any(feature = "portable", test))]
fn is_unc_path(path: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let prefix_unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let prefix_dev: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let prefix_plain: Vec<u16> = "\\\\".encode_utf16().collect();
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.starts_with(&prefix_unc) {
        return true;
    }
    units.starts_with(&prefix_plain) && !units.starts_with(&prefix_dev)
}

#[cfg(feature = "portable")]
mod runtime {
    use std::ffi::OsStr;
    use std::fs::OpenOptions;
    use std::io::{self, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::null_mut;

    use sandbox::{
        PORTABLE_WORKER_APP_CONTAINER, SandboxConfig, delete_app_container_profile,
        ensure_portable_worker_acl, ensure_webview2_windows10_acls,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };

    use super::{DRIVE_REMOTE, VolumePolicy, classify_volume, is_unc_path};
    use crate::strings::Strings;
    use crate::{language, settings};

    pub const DATA_FOLDER_NAME: &str = "data";
    pub const WEBVIEW2_FOLDER_NAME: &str = "WebView2";
    const WEBVIEW2_EXE_NAME: &str = "msedgewebview2.exe";

    pub fn exe_dir() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
    }

    pub fn data_dir() -> Option<PathBuf> {
        exe_dir().map(|dir| dir.join(DATA_FOLDER_NAME))
    }

    /// Verifies portable invariants before creating WebView2 or spawning workers (ADR 0034).
    /// Returns `None` after showing a localized error dialog when startup must abort.
    pub fn preflight(worker: &Path) -> Option<SandboxConfig> {
        let exe_dir = exe_dir()?;
        let data_dir = exe_dir.join(DATA_FOLDER_NAME);
        let ui_strings = resolve_ui_strings(&data_dir);

        let policy = match resolve_volume_policy(&exe_dir) {
            Ok(VolumePolicy::Unsupported) | Err(_) => {
                show_error(
                    ui_strings.portable_unsupported_fs_title,
                    ui_strings.portable_unsupported_fs_message,
                );
                return None;
            }
            Ok(policy) => policy,
        };

        if verify_writable(&data_dir).is_err() {
            show_error(
                ui_strings.portable_unwritable_title,
                ui_strings.portable_unwritable_message,
            );
            return None;
        }

        let webview2_dir = exe_dir.join(WEBVIEW2_FOLDER_NAME);
        if !webview2_dir.join(WEBVIEW2_EXE_NAME).is_file() {
            show_error(
                ui_strings.portable_webview2_missing_title,
                ui_strings.portable_webview2_missing_message,
            );
            return None;
        }

        let app_container = match policy {
            VolumePolicy::AppContainer => {
                if ensure_portable_worker_acl(worker).is_err() {
                    show_error(
                        ui_strings.portable_worker_acl_title,
                        ui_strings.portable_worker_acl_message,
                    );
                    return None;
                }
                if ensure_webview2_windows10_acls(&webview2_dir).is_err() {
                    show_error(
                        ui_strings.portable_webview2_acl_title,
                        ui_strings.portable_webview2_acl_message,
                    );
                    return None;
                }
                Some(PORTABLE_WORKER_APP_CONTAINER.to_owned())
            }
            VolumePolicy::NoAppContainer => None,
            VolumePolicy::Unsupported => unreachable!("handled above"),
        };

        // Must be set before `tauri::webview_version()` and `tauri::Builder::build()`:
        // `fixedRuntime` only sets `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` inside `Builder::build()`.
        // SAFETY: Startup is still single-threaded before any worker threads or WebView2 init.
        unsafe {
            std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", &webview2_dir);
        }

        Some(SandboxConfig {
            app_container,
            grant_program_acl: false,
            ..SandboxConfig::default()
        })
    }

    /// Removes the portable AppContainer profile from Windows registry and `%LOCALAPPDATA%\Packages`.
    pub fn cleanup() {
        let _ = delete_app_container_profile(PORTABLE_WORKER_APP_CONTAINER);
    }

    fn show_error(title: &str, message: &str) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(title)
            .set_description(message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }

    fn resolve_ui_strings(data_dir: &Path) -> &'static Strings {
        let settings_file = data_dir.join(settings::FILE_NAME);
        let store = settings::SettingsStore::load(settings_file.is_file().then_some(settings_file));
        let (effective_lang, _) = language::resolve_language(store.get().language);
        crate::strings::for_language(effective_lang)
    }

    fn resolve_volume_policy(exe_dir: &Path) -> io::Result<VolumePolicy> {
        if is_unc_path(exe_dir) {
            return Ok(VolumePolicy::Unsupported);
        }
        let wide_dir = wide(exe_dir.as_os_str());
        let mut root_buf = vec![0u16; wide_dir.len().max(260)];
        // SAFETY: `wide_dir` is NUL-terminated; `root_buf` capacity is passed cleanly.
        let ok = unsafe {
            GetVolumePathNameW(
                wide_dir.as_ptr(),
                root_buf.as_mut_ptr(),
                root_buf.len() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `GetVolumePathNameW` guarantees a NUL-terminated string inside `root_buf`.
        let drive_type = unsafe { GetDriveTypeW(root_buf.as_ptr()) };
        if drive_type == DRIVE_REMOTE {
            return Ok(VolumePolicy::Unsupported);
        }
        let mut fs_buf = [0u16; 64];
        // SAFETY: `root_buf` is NUL-terminated and `fs_buf` size is passed cleanly.
        let ok = unsafe {
            GetVolumeInformationW(
                root_buf.as_ptr(),
                null_mut(),
                0,
                null_mut(),
                null_mut(),
                null_mut(),
                fs_buf.as_mut_ptr(),
                fs_buf.len() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let len = fs_buf.iter().position(|&c| c == 0).unwrap_or(fs_buf.len());
        let fs_name = String::from_utf16_lossy(&fs_buf[..len]);
        Ok(classify_volume(drive_type, &fs_name))
    }

    fn verify_writable(data_dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let probe = data_dir.join(format!(".portable-write-test-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        file.write_all(b"ok")?;
        drop(file);
        std::fs::remove_file(&probe)
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
}

#[cfg(feature = "portable")]
pub use runtime::{cleanup, data_dir, preflight};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE, VolumePolicy, classify_volume,
        is_unc_path,
    };

    #[test]
    fn classifies_local_and_removable_filesystems() {
        for drive in [DRIVE_FIXED, DRIVE_REMOVABLE, DRIVE_RAMDISK] {
            assert_eq!(classify_volume(drive, "NTFS"), VolumePolicy::AppContainer);
            assert_eq!(classify_volume(drive, "ReFS"), VolumePolicy::AppContainer);
            assert_eq!(classify_volume(drive, "ntfs"), VolumePolicy::AppContainer);
            assert_eq!(
                classify_volume(drive, "FAT32"),
                VolumePolicy::NoAppContainer
            );
            assert_eq!(
                classify_volume(drive, "exFAT"),
                VolumePolicy::NoAppContainer
            );
            assert_eq!(classify_volume(drive, "FAT"), VolumePolicy::NoAppContainer);
            assert_eq!(classify_volume(drive, "CDFS"), VolumePolicy::Unsupported);
            assert_eq!(classify_volume(drive, ""), VolumePolicy::Unsupported);
        }
        assert_eq!(
            classify_volume(DRIVE_REMOTE, "NTFS"),
            VolumePolicy::Unsupported
        );
        assert_eq!(classify_volume(5, "CDFS"), VolumePolicy::Unsupported);
        assert_eq!(classify_volume(0, "NTFS"), VolumePolicy::Unsupported);
    }

    #[test]
    fn detects_unc_paths() {
        assert!(is_unc_path(Path::new(r"\\server\share\portable")));
        assert!(is_unc_path(Path::new(r"\\?\UNC\server\share\portable")));
        assert!(!is_unc_path(Path::new(r"C:\portable")));
        assert!(!is_unc_path(Path::new(r"\\?\C:\portable")));
    }
}
