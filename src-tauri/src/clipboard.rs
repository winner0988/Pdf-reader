//! Puts text copied from a sensitive document on the clipboard (B3-05, ADR 0004,
//! docs/architecture/sensitive-files.md), with the formats that keep it out of Windows' clipboard
//! history and cloud clipboard, and away from the programs that watch the clipboard and honour
//! them. Text copied from any other document goes through the page (`navigator.clipboard`), as
//! before.

// The clipboard's Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::time::Duration;

use ipc_contract::types::{ErrorCode, IpcError};
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

/// `CF_UNICODETEXT`: UTF-16 text that ends with a NUL.
const CF_UNICODETEXT: u32 = 13;

/// The formats Windows reads ("Cloud Clipboard and Clipboard History Formats"): with 0 in the
/// first two, the clipboard history and the cloud clipboard leave the text out; whatever is in the
/// third, every program that watches the clipboard and honours it does too.
const PROTECTION: [&str; 3] = [
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
    "ExcludeClipboardContentFromMonitorProcessing",
];

/// Another program may have the clipboard open for a moment: it is tried this many times, this far
/// apart.
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_PAUSE: Duration = Duration::from_millis(20);

/// Puts `text` on the clipboard, owned by the window `owner`, with the formats that keep it out of
/// the clipboard history, the cloud clipboard and the programs that watch the clipboard. The
/// formats go first: if one cannot be put, the text is not put at all. Line ends become `\r\n`,
/// as Windows programs expect. `text` has no NUL (`check_clipboard_text`).
pub fn write_protected(owner: HWND, text: &str) -> Result<(), IpcError> {
    let formats = PROTECTION.map(register);
    let wide = windows_text(text);
    let _open = Open::new(owner)?;
    // SAFETY: the clipboard is open, by this thread (`Open`).
    if unsafe { EmptyClipboard() } == 0 {
        return Err(failed("the clipboard could not be emptied"));
    }
    for format in formats {
        put(format?, &0u32.to_ne_bytes())?;
    }
    let bytes: Vec<u8> = wide.iter().flat_map(|unit| unit.to_ne_bytes()).collect();
    put(CF_UNICODETEXT, &bytes)
}

/// `text` as `CF_UNICODETEXT` holds it: UTF-16, `\r\n` between lines, a NUL at the end.
fn windows_text(text: &str) -> Vec<u16> {
    let mut wide = Vec::with_capacity(text.len() + 1);
    let mut after_cr = false;
    for unit in text.encode_utf16() {
        if unit == u16::from(b'\n') && !after_cr {
            wide.push(u16::from(b'\r'));
        }
        after_cr = unit == u16::from(b'\r');
        wide.push(unit);
    }
    wide.push(0);
    wide
}

/// The number of the clipboard format `name`, registered if this is its first use.
fn register(name: &str) -> Result<u32, IpcError> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call.
    match unsafe { RegisterClipboardFormatW(wide.as_ptr()) } {
        0 => Err(failed("a clipboard format could not be registered")),
        format => Ok(format),
    }
}

/// Puts `bytes` on the open clipboard as `format`.
fn put(format: u32, bytes: &[u8]) -> Result<(), IpcError> {
    // SAFETY: the block is allocated with the size copied into it, and locked for the copy only.
    // Once the clipboard takes it, it is the system's; until then it is freed here on failure.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if memory.is_null() {
            return Err(failed("no memory for the clipboard"));
        }
        let target = GlobalLock(memory);
        if target.is_null() {
            GlobalFree(memory);
            return Err(failed("no memory for the clipboard"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
        if SetClipboardData(format, memory).is_null() {
            GlobalFree(memory);
            return Err(failed("the clipboard did not take the text"));
        }
    }
    Ok(())
}

/// The clipboard, open by this thread; closed when dropped.
struct Open;

impl Open {
    fn new(owner: HWND) -> Result<Self, IpcError> {
        for attempt in 1..=OPEN_ATTEMPTS {
            // SAFETY: `owner` is a window handle or null; the clipboard is closed by `drop`.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self);
            }
            if attempt < OPEN_ATTEMPTS {
                std::thread::sleep(OPEN_PAUSE);
            }
        }
        Err(IpcError {
            code: ErrorCode::FileInUse,
            message: "another program has the clipboard open".to_owned(),
        })
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard (`Open::new`).
        unsafe {
            CloseClipboard();
        }
    }
}

fn failed(message: &str) -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_end_as_windows_programs_expect() {
        let text = |wide: Vec<u16>| String::from_utf16(&wide[..wide.len() - 1]).unwrap();
        assert_eq!(text(windows_text("one\ntwo")), "one\r\ntwo");
        assert_eq!(text(windows_text("one\r\ntwo\n")), "one\r\ntwo\r\n");
        assert_eq!(text(windows_text("機密\n\n文件")), "機密\r\n\r\n文件");
        assert_eq!(windows_text("").as_slice(), [0]);
        assert_eq!(*windows_text("a").last().unwrap(), 0);
    }

    /// The real clipboard: only on a runner that is thrown away (CI), or when asked for with
    /// `PDF_READER_TEST_CLIPBOARD=1`. On anyone's own computer it would replace what they copied.
    #[test]
    fn the_text_is_on_the_clipboard_with_the_formats_that_keep_it_out_of_the_history() {
        if std::env::var_os("CI").is_none()
            && std::env::var("PDF_READER_TEST_CLIPBOARD").as_deref() != Ok("1")
        {
            eprintln!(
                "skipped: it replaces what is on the clipboard (PDF_READER_TEST_CLIPBOARD=1)"
            );
            return;
        }
        let window = MessageWindow::new();
        write_protected(window.0, "機密 line one\nline two").expect("written");

        let _open = Open::new(window.0).expect("open");
        // SAFETY: the clipboard is open by this thread; each handle it gives is locked and read
        // within the size Windows reports, then unlocked.
        unsafe {
            use windows_sys::Win32::System::DataExchange::{
                GetClipboardData, IsClipboardFormatAvailable,
            };
            use windows_sys::Win32::System::Memory::GlobalSize;
            let read = |format: u32| -> Vec<u8> {
                let memory = GetClipboardData(format);
                assert!(!memory.is_null(), "format {format}");
                let size = GlobalSize(memory);
                let source = GlobalLock(memory).cast::<u8>();
                let bytes = std::slice::from_raw_parts(source, size).to_vec();
                GlobalUnlock(memory);
                bytes
            };
            for name in PROTECTION {
                let format = register(name).unwrap();
                assert!(IsClipboardFormatAvailable(format) != 0, "{name}");
                assert_eq!(read(format)[..4], [0, 0, 0, 0], "{name}");
            }
            let text: Vec<u16> = read(CF_UNICODETEXT)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_ne_bytes(*pair))
                .take_while(|unit| *unit != 0)
                .collect();
            assert_eq!(
                String::from_utf16(&text).unwrap(),
                "機密 line one\r\nline two"
            );
        }
    }

    /// A window that only exists to own the clipboard (`HWND_MESSAGE`), destroyed when dropped.
    struct MessageWindow(HWND);

    impl MessageWindow {
        fn new() -> Self {
            use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE};
            let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
            // SAFETY: a predefined class, no parameters to keep alive; destroyed by `drop`.
            let window = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!window.is_null());
            Self(window)
        }
    }

    impl Drop for MessageWindow {
        fn drop(&mut self) {
            use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
            // SAFETY: the window this test made, on this thread.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}
