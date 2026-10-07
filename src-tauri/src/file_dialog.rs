//! The system's file dialogs (MVP-06, #86, B2-04, B2-02, B2-03): `IFileOpenDialog` and `IFileSaveDialog`, shown
//! by the main process so that paths never reach the WebView. Nothing picked in them is added to
//! the user's recent items in Windows (#86; `rfd` could not set that option).
//!
//! As `rfd` did, a dialog runs on a thread of its own with apartment-threaded COM, owned by the
//! app window, which it keeps disabled until the user is done.

// COM calls of the dialogs; each unsafe block says why it is sound.
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
    FOS_OVERWRITEPROMPT, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog,
    IFileOpenDialog, IFileSaveDialog, IShellItem, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, Interface, PCWSTR};

use crate::strings;

/// Which dialog to show.
enum Kind {
    /// PDF files to open; several can be picked.
    OpenPdfs,
    /// Where to save exported text (B2-04), suggesting `file_name`.
    SaveText { file_name: String },
    /// Where to save a document as another file (B2-02), suggesting `file_name`.
    SavePdf { file_name: String },
    /// Where to write the privacy export of a document (B2-03), suggesting `file_name`.
    PrivacyExport { file_name: String },
    /// Where to save some pages of a document as a file of their own (B2-06), suggesting
    /// `file_name`.
    SplitPdf { file_name: String },
    /// A folder for exported page images (B2-04), or for the files of a split document (B2-06),
    /// under the title the caller gives.
    PickFolder { title: &'static str },
    /// One file of language data for recognising text (B2-10).
    OpenLanguageData,
    /// The picture of a custom stamp (B2-08): one PNG or JPEG file.
    OpenImage,
}

/// Shows the open dialog (PDF files only; several can be picked) over `window` and returns the
/// picked files, or `None` if the user cancelled.
pub async fn pick_pdfs(window: &WebviewWindow) -> Result<Option<Vec<PathBuf>>, IpcError> {
    run(window, Kind::OpenPdfs).await
}

/// Asks where to save exported text, suggesting `file_name`. The dialog itself asks before
/// replacing an existing file.
pub async fn save_text_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SaveText { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to save a document as another file, suggesting `file_name`. The dialog itself
/// asks before replacing an existing file, its own included.
pub async fn save_pdf_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SavePdf { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to write the privacy export of a document (B2-03), suggesting `file_name`. The
/// dialog itself asks before replacing an existing file.
pub async fn privacy_export_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::PrivacyExport { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks for the picture of a custom stamp (B2-08): one PNG or JPEG file. `None` if the user
/// cancelled.
pub async fn pick_image(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenImage)
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to save some pages of a document as a file of their own (B2-06), suggesting
/// `file_name`. The dialog itself asks before replacing an existing file.
pub async fn split_pdf_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SplitPdf { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks for one file of language data for recognising text (`.traineddata`, B2-10).
pub async fn pick_language_data(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenLanguageData)
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks for a folder (the one exported page images go to, or the files of a split document),
/// under `title`.
pub async fn pick_folder(
    window: &WebviewWindow,
    title: &'static str,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::PickFolder { title })
        .await?
        .and_then(|mut paths| paths.pop()))
}

async fn run(window: &WebviewWindow, kind: Kind) -> Result<Option<Vec<PathBuf>>, IpcError> {
    // A window handle is only a number to another thread; the window outlives the dialog.
    let owner = window.hwnd().map_err(|_| failed())?.0 as isize;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("file dialog".to_owned())
        .spawn(move || {
            let _ = sender.send(show(HWND(owner as *mut _), kind));
        })
        .map_err(|_| failed())?;
    receiver.await.map_err(|_| failed())?.map_err(|_| failed())
}

fn failed() -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message: "the file dialog could not be shown".to_owned(),
    }
}

/// What each dialog adds to the system's default options (which already keep the process's
/// working folder, and for saving ask before replacing a file): files in the file system only,
/// and nothing on the recent items (#86).
fn options(kind: &Kind, defaults: FILEOPENDIALOGOPTIONS) -> FILEOPENDIALOGOPTIONS {
    let common = defaults | FOS_FORCEFILESYSTEM | FOS_DONTADDTORECENT;
    match kind {
        Kind::OpenPdfs => common | FOS_ALLOWMULTISELECT,
        Kind::SaveText { .. }
        | Kind::SavePdf { .. }
        | Kind::PrivacyExport { .. }
        | Kind::SplitPdf { .. } => common | FOS_OVERWRITEPROMPT,
        Kind::PickFolder { .. } => common | FOS_PICKFOLDERS,
        Kind::OpenLanguageData | Kind::OpenImage => common,
    }
}

/// Blocks until the user is done.
fn show(owner: HWND, kind: Kind) -> windows::core::Result<Option<Vec<PathBuf>>> {
    let _com = Com::init()?;
    let dialog = new_dialog(&kind)?;
    configure(&dialog, &kind)?;
    // SAFETY: `owner` is the app window's handle (or a stale one, which the dialog treats as no
    // owner); the dialog is a live COM object of this thread.
    match unsafe { dialog.Show(Some(owner)) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
        Err(error) => return Err(error),
    }
    let paths = if let Kind::OpenPdfs = kind {
        // SAFETY: after a successful `Show`, `GetResults` returns the picked items.
        let items = unsafe { dialog.cast::<IFileOpenDialog>()?.GetResults()? };
        // SAFETY: a live item array of this thread.
        (0..unsafe { items.GetCount()? })
            .map(|index| path_of(&unsafe { items.GetItemAt(index)? }))
            .collect::<windows::core::Result<Vec<_>>>()?
    } else {
        // SAFETY: after a successful `Show`, `GetResult` returns the one item.
        vec![path_of(&unsafe { dialog.GetResult()? })?]
    };
    Ok(Some(paths))
}

fn path_of(item: &IShellItem) -> windows::core::Result<PathBuf> {
    // SAFETY: the display name is a NUL-terminated string the item allocated with the COM
    // allocator; it is read once and freed once with `CoTaskMemFree`.
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(path)
    }
}

fn new_dialog(kind: &Kind) -> windows::core::Result<IFileDialog> {
    // SAFETY: COM is initialised on this thread (`Com`); the class ids are the system's dialogs.
    unsafe {
        match kind {
            Kind::SaveText { .. }
            | Kind::SavePdf { .. }
            | Kind::PrivacyExport { .. }
            | Kind::SplitPdf { .. } => {
                CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?
                    .cast()
            }
            Kind::OpenPdfs | Kind::PickFolder { .. } | Kind::OpenLanguageData | Kind::OpenImage => {
                CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?
                    .cast()
            }
        }
    }
}

fn configure(dialog: &IFileDialog, kind: &Kind) -> windows::core::Result<()> {
    let (title, filter) = match kind {
        Kind::OpenPdfs => (
            strings::OPEN_DIALOG_TITLE,
            Some((strings::PDF_FILTER_NAME, "*.pdf")),
        ),
        Kind::SaveText { .. } => (
            strings::EXPORT_TEXT_DIALOG_TITLE,
            Some((strings::TEXT_FILTER_NAME, "*.txt")),
        ),
        Kind::SavePdf { .. } => (
            strings::SAVE_AS_DIALOG_TITLE,
            Some((strings::PDF_FILTER_NAME, "*.pdf")),
        ),
        Kind::PrivacyExport { .. } => (
            strings::PRIVACY_EXPORT_DIALOG_TITLE,
            Some((strings::PDF_FILTER_NAME, "*.pdf")),
        ),
        Kind::SplitPdf { .. } => (
            strings::SPLIT_DIALOG_TITLE,
            Some((strings::PDF_FILTER_NAME, "*.pdf")),
        ),
        Kind::PickFolder { title } => (*title, None),
        Kind::OpenLanguageData => (
            strings::LANGUAGE_DIALOG_TITLE,
            Some((strings::LANGUAGE_FILTER_NAME, "*.traineddata")),
        ),
        Kind::OpenImage => (
            strings::STAMP_IMAGE_DIALOG_TITLE,
            Some((strings::IMAGE_FILTER_NAME, "*.png;*.jpg;*.jpeg")),
        ),
    };
    let title = HSTRING::from(title);
    // SAFETY: the strings outlive the calls, which copy them; the filter array has one entry.
    unsafe {
        dialog.SetOptions(options(kind, dialog.GetOptions()?))?;
        dialog.SetTitle(&title)?;
        if let Some((name, pattern)) = filter {
            let (name, pattern) = (HSTRING::from(name), HSTRING::from(pattern));
            dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            }])?;
        }
        if let Kind::SaveText { file_name }
        | Kind::SavePdf { file_name }
        | Kind::PrivacyExport { file_name }
        | Kind::SplitPdf { file_name } = kind
        {
            let extension = if let Kind::SaveText { .. } = kind {
                "txt"
            } else {
                "pdf"
            };
            dialog.SetDefaultExtension(&HSTRING::from(extension))?;
            dialog.SetFileName(&HSTRING::from(file_name.as_str()))?;
        }
    }
    Ok(())
}

/// COM on this thread, apartment-threaded as the dialogs require, until dropped. Not `Send`:
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

    /// The options a real dialog object reports once configured, not only what was asked for.
    fn options_of(kind: Kind) -> impl Fn(FILEOPENDIALOGOPTIONS) -> bool {
        let _com = Com::init().unwrap();
        let dialog = new_dialog(&kind).unwrap();
        configure(&dialog, &kind).unwrap();
        // SAFETY: a live dialog of this thread.
        let set = unsafe { dialog.GetOptions() }.unwrap();
        move |option| set.0 & option.0 == option.0
    }

    #[test]
    fn no_dialog_adds_to_the_recent_items() {
        let open = options_of(Kind::OpenPdfs);
        assert!(
            open(FOS_DONTADDTORECENT) && open(FOS_ALLOWMULTISELECT) && open(FOS_FORCEFILESYSTEM)
        );
        // The system's defaults stay (rfd replaced them).
        assert!(open(FOS_FILEMUSTEXIST) && open(FOS_NOCHANGEDIR));

        let save = options_of(Kind::SaveText {
            file_name: "報告.txt".to_owned(),
        });
        assert!(
            save(FOS_DONTADDTORECENT) && save(FOS_OVERWRITEPROMPT) && save(FOS_FORCEFILESYSTEM)
        );

        let save_as = options_of(Kind::SavePdf {
            file_name: "報告.pdf".to_owned(),
        });
        assert!(save_as(FOS_DONTADDTORECENT) && save_as(FOS_OVERWRITEPROMPT));

        let privacy = options_of(Kind::PrivacyExport {
            file_name: "報告（隱私匯出）.pdf".to_owned(),
        });
        assert!(privacy(FOS_DONTADDTORECENT) && privacy(FOS_OVERWRITEPROMPT));

        let split = options_of(Kind::SplitPdf {
            file_name: "報告-p2-4.pdf".to_owned(),
        });
        assert!(split(FOS_DONTADDTORECENT) && split(FOS_OVERWRITEPROMPT));

        let language = options_of(Kind::OpenLanguageData);
        assert!(
            language(FOS_DONTADDTORECENT) && language(FOS_FORCEFILESYSTEM),
            "nothing picked goes on the recent items"
        );
        assert!(!language(FOS_ALLOWMULTISELECT), "one file");

        let folder = options_of(Kind::PickFolder {
            title: strings::EXPORT_IMAGES_DIALOG_TITLE,
        });
        assert!(
            folder(FOS_DONTADDTORECENT) && folder(FOS_PICKFOLDERS) && folder(FOS_FORCEFILESYSTEM)
        );

        // One picture for a stamp (B2-08), not several.
        let image = options_of(Kind::OpenImage);
        assert!(
            image(FOS_DONTADDTORECENT) && image(FOS_FORCEFILESYSTEM) && image(FOS_FILEMUSTEXIST)
        );
        assert!(!image(FOS_ALLOWMULTISELECT));
    }
}
