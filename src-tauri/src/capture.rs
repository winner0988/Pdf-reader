//! Keeps the window out of screen captures while a sensitive document is open (B3-13, ADR 0023,
//! docs/architecture/sensitive-files.md). Windows still draws the window on the screen, but
//! PrintScreen, the Snipping Tool, recordings, screen sharing and Windows.Graphics.Capture get
//! nothing of it (`WDA_EXCLUDEFROMCAPTURE`). It is the whole window or nothing (one window, many
//! tabs, ADR 0012), so while one tab is sensitive the others are kept out too.
//!
//! The window is changed before the frontend hears of the sensitive document (`Opened`), so its
//! first page is never on the screen while the window can be captured; it is let back in once the
//! last sensitive tab is closed or unmarked.

// SetWindowDisplayAffinity and GetWindowDisplayAffinity; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::sync::{Mutex, PoisonError};

use ipc_contract::types::CaptureProtection;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

/// A window whose display affinity can be set.
pub trait Window {
    /// Keeps the window out of captures (`exclude`), or lets it back in. Whether Windows did.
    fn exclude_from_capture(&self, exclude: bool) -> bool;
}

impl Window for tauri::WebviewWindow {
    fn exclude_from_capture(&self, exclude: bool) -> bool {
        self.hwnd()
            .is_ok_and(|hwnd| set_affinity(hwnd.0 as HWND, exclude))
    }
}

/// Whether the window is kept out of captures now.
#[derive(Default)]
pub struct CaptureGuard {
    state: Mutex<CaptureProtection>,
}

impl CaptureGuard {
    pub fn state(&self) -> CaptureProtection {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `window` out of captures while `sensitive_open` says a sensitive document is open,
    /// and lets it back in once none is. Returns the new state when it changed. `sensitive_open`
    /// is asked under the guard's lock, so the last of two calls at once leaves the window as the
    /// tabs are then. Without a window nothing changes: there is nothing to show a document in.
    pub fn update(
        &self,
        sensitive_open: impl FnOnce() -> bool,
        window: Option<&dyn Window>,
    ) -> Option<CaptureProtection> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let window = window?;
        let next = match (sensitive_open(), *state) {
            (true, CaptureProtection::On) | (false, CaptureProtection::Off) => return None,
            // Tried again each time: it costs nothing, and the warning goes away if it works.
            (true, _) if window.exclude_from_capture(true) => CaptureProtection::On,
            (true, _) => CaptureProtection::Failed,
            (false, before) => {
                // Also after a failed try, which may have left the window with another affinity.
                let back = window.exclude_from_capture(false);
                // If Windows will not let the window back in, it stays out, as the status bar says.
                if before == CaptureProtection::On && !back {
                    return None;
                }
                CaptureProtection::Off
            }
        };
        if next == *state {
            return None;
        }
        *state = next;
        Some(next)
    }
}

/// Sets the display affinity of `window`, a window of this process, and reads it back: before
/// Windows 10 2004, `WDA_EXCLUDEFROMCAPTURE` is not known, and the call fails or leaves another.
fn set_affinity(window: HWND, exclude: bool) -> bool {
    let wanted = if exclude {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    // SAFETY: plain values; Windows checks the handle and that the window is this process's.
    if unsafe { SetWindowDisplayAffinity(window, wanted) } == 0 {
        return false;
    }
    let mut now = 0;
    // SAFETY: `now` is a live u32 for Windows to write.
    (unsafe { GetWindowDisplayAffinity(window, &mut now) }) != 0 && now == wanted
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A window that records what it was asked, and does it or not.
    struct Fake {
        works: bool,
        asked: RefCell<Vec<bool>>,
    }

    impl Fake {
        fn new(works: bool) -> Self {
            Self {
                works,
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Window for Fake {
        fn exclude_from_capture(&self, exclude: bool) -> bool {
            self.asked.borrow_mut().push(exclude);
            self.works
        }
    }

    #[test]
    fn the_window_is_kept_out_while_a_sensitive_document_is_open_and_let_back_in_after() {
        let guard = CaptureGuard::default();
        let window = Fake::new(true);
        assert_eq!(guard.state(), CaptureProtection::Off);
        // Documents that are not sensitive: nothing to do.
        assert_eq!(guard.update(|| false, Some(&window)), None);
        assert!(window.asked.borrow().is_empty());

        assert_eq!(
            guard.update(|| true, Some(&window)),
            Some(CaptureProtection::On)
        );
        // A second sensitive tab, an edit: already out.
        assert_eq!(guard.update(|| true, Some(&window)), None);
        assert_eq!(guard.state(), CaptureProtection::On);

        assert_eq!(
            guard.update(|| false, Some(&window)),
            Some(CaptureProtection::Off)
        );
        assert_eq!(*window.asked.borrow(), [true, false]);
    }

    #[test]
    fn a_window_windows_will_not_keep_out_is_said_to_be_so_and_tried_again() {
        let guard = CaptureGuard::default();
        let window = Fake::new(false);
        assert_eq!(
            guard.update(|| true, Some(&window)),
            Some(CaptureProtection::Failed)
        );
        // Tried again, and still failing: no new state to tell.
        assert_eq!(guard.update(|| true, Some(&window)), None);
        // The last sensitive tab closed: no warning any more, and the window is let back in.
        assert_eq!(
            guard.update(|| false, Some(&window)),
            Some(CaptureProtection::Off)
        );
        assert_eq!(*window.asked.borrow(), [true, true, false]);

        // It works the next time: the warning goes.
        let guard = CaptureGuard::default();
        assert_eq!(
            guard.update(|| true, Some(&Fake::new(false))),
            Some(CaptureProtection::Failed)
        );
        assert_eq!(
            guard.update(|| true, Some(&Fake::new(true))),
            Some(CaptureProtection::On)
        );
    }

    #[test]
    fn a_window_that_cannot_be_let_back_in_stays_out() {
        let guard = CaptureGuard::default();
        assert_eq!(
            guard.update(|| true, Some(&Fake::new(true))),
            Some(CaptureProtection::On)
        );
        assert_eq!(guard.update(|| false, Some(&Fake::new(false))), None);
        assert_eq!(guard.state(), CaptureProtection::On);
    }

    #[test]
    fn without_a_window_nothing_changes() {
        let guard = CaptureGuard::default();
        let asked = RefCell::new(false);
        assert_eq!(
            guard.update(
                || {
                    *asked.borrow_mut() = true;
                    true
                },
                None
            ),
            None
        );
        assert!(!*asked.borrow());
        assert_eq!(guard.state(), CaptureProtection::Off);
    }

    /// The real call, on a window of this test that is never shown: no screen is captured.
    #[test]
    fn windows_keeps_a_window_of_this_process_out_of_captures() {
        let window = HiddenWindow::new();
        assert!(set_affinity(window.0, true));
        let mut now = 0;
        // SAFETY: the window this test made; `now` is a live u32.
        assert_ne!(unsafe { GetWindowDisplayAffinity(window.0, &mut now) }, 0);
        assert_eq!(now, WDA_EXCLUDEFROMCAPTURE);
        assert!(set_affinity(window.0, false));
        // SAFETY: as above.
        assert_ne!(unsafe { GetWindowDisplayAffinity(window.0, &mut now) }, 0);
        assert_eq!(now, WDA_NONE);
    }

    /// A top-level window that is never shown, destroyed when dropped.
    struct HiddenWindow(HWND);

    impl HiddenWindow {
        fn new() -> Self {
            use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_OVERLAPPED};
            let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
            // SAFETY: a predefined class, no parameters to keep alive; destroyed by `drop`.
            let window = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    100,
                    100,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!window.is_null());
            Self(window)
        }
    }

    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
            // SAFETY: the window this test made, on this thread.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}
