//! The window's and the notification area's icon follow Windows' own light or dark mode (UX-04,
//! #548, docs/ux/visual-style.md section 8).
//!
//! The app has two icons of the same picture. The blue tile is the exe's own: Explorer, the
//! shortcuts and the installer show it whatever the mode, because Windows gives an app that is
//! not packaged no way to change those. What the app can change is what its window shows (title
//! bar, the taskbar button while it runs, Alt+Tab) and its icon in the notification area: there,
//! a dark Windows gets the blue tile and a light Windows the dark tile, each the one that stands
//! out against the taskbar.
//!
//! The mode is the taskbar's ("Windows mode", `SystemUsesLightTheme`), not the apps' ("app
//! mode"), which can differ and which the page follows by itself. It is read from the registry,
//! and a thread waits for the key to change, so the icons follow while the app runs. Nothing is
//! written anywhere, and both icons are resources of the exe: no file is read.

// The registry's Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::ptr::null_mut;
use std::sync::{Mutex, PoisonError};

use tauri::image::Image;
use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegCloseKey,
    RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};

/// The exe's own icon, the blue tile (`icons/icon.ico`): the id tauri-build gives it.
const APP_ICON: u16 = tauri::utils::platform::WINDOWS_APP_ICON_RESOURCE_ID;
/// The dark tile (`icons/icon-dark.ico`): the id build.rs gives it. It is above the app icon's,
/// so the app icon stays the first in the exe, the one Explorer and the shortcuts show.
const DARK_TILE_ICON: u16 = 32513;
const _: () = assert!(DARK_TILE_ICON > APP_ICON);

/// Where Windows keeps the user's choice of mode, under `HKEY_CURRENT_USER`.
const MODE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
/// 0: the taskbar and the Start menu are dark; anything else: they are light.
const MODE_VALUE: &str = "SystemUsesLightTheme";

/// The app's one window.
const WINDOW: &str = "main";
/// The size of a window icon when the app's own could not be asked for its.
const DEFAULT_SIZE: u32 = 32;
/// The notification area's icons are half a window icon's size, and never below this.
const SMALLEST_SIZE: u32 = 16;

/// Whether Windows itself (the taskbar, the Start menu, the notification area) is light or dark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsMode {
    Light,
    /// Also when Windows does not say: a taskbar was dark before there was a choice, and the
    /// blue tile this leads to is the one that can be seen on both.
    #[default]
    Dark,
}

impl WindowsMode {
    fn of(value: Option<u32>) -> Self {
        match value {
            None | Some(0) => Self::Dark,
            Some(_) => Self::Light,
        }
    }

    /// The icon that stands out against this mode's taskbar.
    fn icon(self) -> u16 {
        match self {
            Self::Dark => APP_ICON,
            Self::Light => DARK_TILE_ICON,
        }
    }
}

/// The mode whose icon the window shows now. A window starts with the exe's own icon, the one
/// of a dark Windows.
#[derive(Default)]
pub struct AppIcon {
    shown: Mutex<WindowsMode>,
}

/// The registry key the mode is read from. A debug build reads the key
/// `PDF_READER_TEST_WINDOWS_MODE_KEY` names instead, so that both modes can be tried without
/// changing the user's Windows; a release build has no such way in.
fn mode_key() -> String {
    #[cfg(debug_assertions)]
    if let Ok(key) = std::env::var("PDF_READER_TEST_WINDOWS_MODE_KEY") {
        return key;
    }
    MODE_KEY.to_owned()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The mode value of `key` (under `HKEY_CURRENT_USER`); nothing if it is not there or not a number.
fn read_value(key: &str) -> Option<u32> {
    let key = wide(key);
    let value = wide(MODE_VALUE);
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `key` and `value` end in NUL; `data` is `size` bytes that Windows may write, and
    // `RRF_RT_REG_DWORD` makes it refuse a value of any other type or size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut data).cast(),
            &raw mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// Calls `changed` each time a value of `key` (under `HKEY_CURRENT_USER`) is set, from a thread
/// of its own that lives as long as the app. Without the key, or once it is gone, nothing is
/// watched: the icons stay as they are.
fn watch(key: String, changed: impl Fn() + Send + 'static) {
    // Without the thread the icons are those of the mode the app started in.
    let _ = std::thread::Builder::new()
        .name("windows-mode".to_owned())
        .spawn(move || {
            let key = wide(&key);
            let mut opened: HKEY = null_mut();
            // SAFETY: `key` ends in NUL; on success `opened` is a key that is closed below.
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    0,
                    KEY_NOTIFY,
                    &raw mut opened,
                )
            };
            if status != ERROR_SUCCESS {
                return;
            }
            loop {
                // SAFETY: `opened` is the open key; without an event (null, not asynchronous)
                // the call only returns once a value of the key was set, or with an error.
                let status = unsafe {
                    RegNotifyChangeKeyValue(opened, 0, REG_NOTIFY_CHANGE_LAST_SET, null_mut(), 0)
                };
                if status != ERROR_SUCCESS {
                    break;
                }
                changed();
            }
            // SAFETY: `opened` was opened above and is not used after this.
            unsafe { RegCloseKey(opened) };
        });
}

/// The size of the app's window icon: Windows' own large icon size, which grows with the
/// display's scaling.
fn window_size(app: &AppHandle) -> (u32, u32) {
    app.default_window_icon()
        .map_or((DEFAULT_SIZE, DEFAULT_SIZE), |icon| {
            (icon.width(), icon.height())
        })
}

/// Half of `size`: the notification area's icons are Windows' small icons, half the large ones.
fn half(size: u32) -> u32 {
    (size / 2).max(SMALLEST_SIZE)
}

/// The picture of `mode`'s icon Windows would draw at this size; nothing if the exe has no such
/// icon (a test's exe has none).
fn picture(mode: WindowsMode, (width, height): (u32, u32)) -> Option<Image<'static>> {
    Image::from_icon_resource(mode.icon(), width, height).ok()
}

/// The icon for the notification area now: that of Windows' mode at the notification area's
/// size, or else the app's window icon, which Windows shrinks.
pub fn tray_icon(app: &AppHandle) -> Option<Image<'static>> {
    let (width, height) = window_size(app);
    let mode = WindowsMode::of(read_value(&mode_key()));
    picture(mode, (half(width), half(height))).or_else(|| {
        app.default_window_icon()
            .map(|icon| icon.clone().to_owned())
    })
}

/// On the main thread: gives the window, and the notification area's icon if there is one, the
/// icon of Windows' mode, when that is not the one they show.
fn follow(app: &AppHandle) {
    let mode = WindowsMode::of(read_value(&mode_key()));
    let state = app.state::<AppIcon>();
    let mut shown = state.shown.lock().unwrap_or_else(PoisonError::into_inner);
    if *shown == mode {
        return;
    }
    let Some(icon) = picture(mode, window_size(app)) else {
        return;
    };
    if let Some(window) = app.get_webview_window(WINDOW)
        && window.set_icon(icon).is_err()
    {
        // Tried again at the next change.
        return;
    }
    *shown = mode;
    drop(shown);
    crate::background::icon_changed(app);
}

/// Gives the window the icon of Windows' mode, and keeps it and the notification area's icon so
/// as the user changes the mode. Called once, after the window is built.
pub fn follow_windows_mode(app: &AppHandle) {
    follow(app);
    let handle = app.clone();
    watch(mode_key(), move || {
        let on_main = handle.clone();
        let _ = handle.run_on_main_thread(move || follow(&on_main));
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use windows_sys::Win32::System::Registry::{
        KEY_WRITE, REG_DWORD, REG_OPTION_VOLATILE, REG_SZ, RegCreateKeyExW, RegDeleteTreeW,
        RegSetValueExW,
    };

    use super::*;

    /// What the names of the tests' keys start with, under `HKEY_CURRENT_USER`. Each key is
    /// directly under `Software`: with a key of their own above them, that one would be left.
    const SCRATCH: &str = r"Software\PdfReaderTest";

    /// A registry key of this test's own, gone with it (and with the session: it is volatile).
    struct ScratchKey {
        path: String,
        opened: HKEY,
    }

    impl ScratchKey {
        fn new() -> Self {
            static COUNT: AtomicU32 = AtomicU32::new(0);
            let path = format!(
                r"{SCRATCH}-windows-mode-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, Ordering::SeqCst)
            );
            let key = wide(&path);
            let mut opened: HKEY = null_mut();
            // SAFETY: `key` ends in NUL; `opened` is closed when the scratch key is dropped.
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    0,
                    null_mut(),
                    REG_OPTION_VOLATILE,
                    KEY_WRITE,
                    null_mut(),
                    &raw mut opened,
                    null_mut(),
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
            Self { path, opened }
        }

        fn set(&self, name: &str, kind: u32, data: &[u8]) {
            let name = wide(name);
            // SAFETY: `name` ends in NUL, and `data` is as long as the call is told.
            let status = unsafe {
                RegSetValueExW(
                    self.opened,
                    name.as_ptr(),
                    0,
                    kind,
                    data.as_ptr(),
                    data.len() as u32,
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
        }

        fn set_mode(&self, value: u32) {
            self.set(MODE_VALUE, REG_DWORD, &value.to_le_bytes());
        }
    }

    impl Drop for ScratchKey {
        fn drop(&mut self) {
            let key = wide(&self.path);
            // SAFETY: the key is this test's own; `key` ends in NUL, and `opened` is not used
            // again. A thread still watching the key keeps it until the test's process ends.
            unsafe {
                RegCloseKey(self.opened);
                RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr());
            }
        }
    }

    #[test]
    fn a_dark_windows_gets_the_blue_tile_and_a_light_one_the_dark_tile() {
        assert_eq!(WindowsMode::of(Some(0)).icon(), APP_ICON);
        assert_eq!(WindowsMode::of(Some(1)).icon(), DARK_TILE_ICON);
        // Anything but 0 is light, as Windows itself reads the value.
        assert_eq!(WindowsMode::of(Some(7)), WindowsMode::Light);
        // A Windows that does not say gets the icon the exe has anyway.
        assert_eq!(WindowsMode::of(None).icon(), APP_ICON);
        assert_eq!(
            AppIcon::default().shown.into_inner().unwrap().icon(),
            APP_ICON
        );
    }

    #[test]
    fn the_build_script_gives_the_dark_tile_the_id_the_app_asks_for() {
        let script = include_str!("../build.rs");
        assert!(script.contains(&format!(
            "{DARK_TILE_ICON} ICON \\\"icons/icon-dark.ico\\\""
        )));
    }

    #[test]
    fn the_mode_is_read_from_the_key_as_a_number_or_not_at_all() {
        let key = ScratchKey::new();
        assert_eq!(read_value(&key.path), None);
        key.set_mode(1);
        assert_eq!(read_value(&key.path), Some(1));
        key.set_mode(0);
        assert_eq!(read_value(&key.path), Some(0));
        // Text where the number should be is not read as one.
        let text: Vec<u8> = wide("1")
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        key.set(MODE_VALUE, REG_SZ, &text);
        assert_eq!(read_value(&key.path), None);
        assert_eq!(read_value(&format!(r"{SCRATCH}-not-there")), None);
    }

    #[test]
    fn a_change_of_the_key_is_heard_each_time() {
        let key = ScratchKey::new();
        key.set_mode(0);
        let (told, changes) = mpsc::channel();
        watch(key.path.clone(), move || {
            let _ = told.send(());
        });
        // A change is only heard once the thread waits for one, so the value is set until two
        // were: the second shows that the thread went back to waiting after the first.
        let mut value = 0;
        let mut heard = 0;
        for _ in 0..400 {
            if heard == 2 {
                break;
            }
            value = 1 - value;
            key.set_mode(value);
            if changes.recv_timeout(Duration::from_millis(50)).is_ok() {
                heard += 1;
            }
        }
        assert_eq!(heard, 2, "the key's changes were not heard");
        assert_eq!(read_value(&key.path), Some(value));
    }

    #[test]
    fn a_key_that_is_not_there_is_not_watched() {
        let (heard, changes) = mpsc::channel::<()>();
        watch(format!(r"{SCRATCH}-not-there"), move || {
            let _ = heard.send(());
        });
        // The thread ends at once and drops its end: nothing was heard, and nothing will be.
        assert!(changes.recv_timeout(Duration::from_secs(10)).is_err());
    }

    #[test]
    fn the_notification_area_icon_is_half_the_window_icon_and_never_tiny() {
        assert_eq!(half(32), 16);
        assert_eq!(half(48), 24);
        assert_eq!(half(64), 32);
        assert_eq!(half(20), 16);
    }

    #[test]
    fn an_exe_without_the_icons_has_no_picture() {
        // The test's exe has no icon resources; the app then keeps the icon it has.
        assert!(picture(WindowsMode::Light, (32, 32)).is_none());
    }
}
