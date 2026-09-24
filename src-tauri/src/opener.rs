//! Hands a checked web link to the system's default browser or mail program (MVP-12).
//!
//! `ShellExecuteW` receives the URL as its one file argument; no command line is put together
//! here. The URL has already been through [`crate::links::preview`], so it is plain ASCII
//! with a scheme of `http`, `https` or `mailto` and nothing a command line could misread.
//! It runs on the main thread, where COM is initialised as the shell expects.

// The one Win32 call of the main process; see the SAFETY comment below.
#![allow(unsafe_code)]

use ipc_contract::types::{ErrorCode, IpcError};
use tauri::AppHandle;

fn failed(message: &str) -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message: message.to_owned(),
    }
}

/// Opens `url` (from [`crate::links::preview`]) with the program the user chose for its scheme.
pub async fn open(app: &AppHandle, url: String) -> Result<(), IpcError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(shell_open(&url));
    })
    .map_err(|_| failed("the main thread is not running"))?;
    receiver
        .await
        .map_err(|_| failed("opening was abandoned"))?
}

#[cfg(windows)]
fn shell_open(url: &str) -> Result<(), IpcError> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call; the other
    // pointer arguments are null, which ShellExecuteW documents as "none".
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(failed("the system could not open the link"))
    }
}

#[cfg(not(windows))]
fn shell_open(_url: &str) -> Result<(), IpcError> {
    Err(failed("opening links is only supported on Windows"))
}
