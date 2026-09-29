//! The open dialog (MVP-06, #86): the system's own `IFileOpenDialog`, shown by the main process
//! so that picked paths never reach the WebView. It is the same dialog `rfd` showed, with one
//! more option: the picked files are not added to the user's recent items in Windows (#86;
//! `rfd` cannot set that option).
//!
//! As `rfd` does, the dialog runs on a thread of its own with apartment-threaded COM, owned by
//! the app window, which it keeps disabled until the user is done.

// COM calls of the dialog; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::ffi::OsString;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use ipc_contract::types::{ErrorCode, IpcError};
use tauri::WebviewWindow;
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT, FOS_DONTADDTORECENT, FOS_FORCEFILESYSTEM,
    FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PCWSTR};

use crate::strings;

/// Shows the dialog (PDF files only; several can be picked) over `window` and returns the
/// picked files, or `None` if the user cancelled.
pub async fn pick_pdfs(window: &WebviewWindow) -> Result<Option<Vec<PathBuf>>, IpcError> {
    // A window handle is only a number to another thread; the window outlives the dialog.
    let owner = window.hwnd().map_err(|_| failed())?.0 as isize;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("open dialog".to_owned())
        .spawn(move || {
            let _ = sender.send(show(HWND(owner as *mut _)));
        })
        .map_err(|_| failed())?;
    receiver.await.map_err(|_| failed())?.map_err(|_| failed())
}

fn failed() -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message: "the open dialog could not be shown".to_owned(),
    }
}

/// What the dialog adds to the system's default options (which already keep the process's
/// working folder and let only existing files be picked): several files at once, files in the
/// file system only, and nothing on the recent items (#86).
fn options(defaults: FILEOPENDIALOGOPTIONS) -> FILEOPENDIALOGOPTIONS {
    defaults | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_DONTADDTORECENT
}

/// Blocks until the user is done.
fn show(owner: HWND) -> windows::core::Result<Option<Vec<PathBuf>>> {
    let _com = Com::init()?;
    let dialog = new_dialog()?;
    configure(&dialog)?;
    // SAFETY: `owner` is the app window's handle (or a stale one, which the dialog treats as no
    // owner); the dialog is a live COM object of this thread.
    match unsafe { dialog.Show(Some(owner)) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
        Err(error) => return Err(error),
    }
    // SAFETY: after a successful `Show`, `GetResults` returns the picked items; each display
    // name is a NUL-terminated string the dialog allocated with the COM allocator, read once and
    // freed once with `CoTaskMemFree`.
    unsafe {
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for index in 0..items.GetCount()? {
            let name = items.GetItemAt(index)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            paths.push(PathBuf::from(OsString::from_wide(name.as_wide())));
            CoTaskMemFree(Some(name.0 as *const _));
        }
        Ok(Some(paths))
    }
}

fn new_dialog() -> windows::core::Result<IFileOpenDialog> {
    // SAFETY: COM is initialised on this thread (`Com`); the class id is the system's dialog.
    unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
}

fn configure(dialog: &IFileOpenDialog) -> windows::core::Result<()> {
    let title = HSTRING::from(strings::OPEN_DIALOG_TITLE);
    let filter_name = HSTRING::from(strings::PDF_FILTER_NAME);
    let filter = HSTRING::from("*.pdf");
    // SAFETY: the strings outlive the calls, which copy them; the filter array has one entry.
    unsafe {
        dialog.SetOptions(options(dialog.GetOptions()?))?;
        dialog.SetTitle(&title)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: PCWSTR(filter_name.as_ptr()),
            pszSpec: PCWSTR(filter.as_ptr()),
        }])
    }
}

/// COM on this thread, apartment-threaded as the dialog requires, until dropped. Not `Send`:
/// it must be dropped on the thread that made it.
struct Com(PhantomData<*const ()>);

impl Com {
    fn init() -> windows::core::Result<Self> {
        // SAFETY: no reserved pointer. Success (including "already initialised") is balanced by
        // `CoUninitialize` when this is dropped, on the same thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok()?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: `init` initialised COM on this thread.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Shell::{FOS_FILEMUSTEXIST, FOS_NOCHANGEDIR};

    use super::*;

    #[test]
    fn the_dialog_adds_nothing_to_the_recent_items() {
        let _com = Com::init().unwrap();
        let dialog = new_dialog().unwrap();
        configure(&dialog).unwrap();
        // What the real dialog object reports, not only what was asked for.
        // SAFETY: a live dialog of this thread.
        let set = unsafe { dialog.GetOptions() }.unwrap();
        let has = |option: FILEOPENDIALOGOPTIONS| set.0 & option.0 == option.0;
        assert!(has(FOS_DONTADDTORECENT));
        assert!(has(FOS_ALLOWMULTISELECT) && has(FOS_FORCEFILESYSTEM));
        // The system's defaults stay (rfd replaced them).
        assert!(has(FOS_FILEMUSTEXIST) && has(FOS_NOCHANGEDIR));
    }
}
