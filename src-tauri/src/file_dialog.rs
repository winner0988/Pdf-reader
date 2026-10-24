//! The system's file dialogs (MVP-06, #86, B2-04, B2-02, B2-03, B3-03): `IFileOpenDialog` and `IFileSaveDialog`, shown
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

use ipc_contract::types::{CreateKind, ErrorCode, IpcError};
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

use crate::create_pdf;
use crate::strings;

/// Which dialog to show.
enum Kind {
    /// PDF files to open; several can be picked.
    OpenPdfs,
    /// PDF files a batch is done to (B3-04); several can be picked.
    OpenBatchFiles,
    /// The one PDF file whose pages are put into the document (B2-06).
    OpenPagesSource,
    /// Where to save exported text (B2-04), suggesting `file_name`.
    SaveText { file_name: String },
    /// Where to save exported annotation summary (Issue #306), suggesting `file_name`.
    SaveAnnotationSummary { file_name: String, markdown: bool },
    /// Where to save a document as another file (B2-02), suggesting `file_name`.
    SavePdf { file_name: String },
    /// Where to save some pages as a Word document (B3-02), suggesting `file_name`.
    SaveDocx { file_name: String },
    /// Where to write the privacy export of a document (B2-03), suggesting `file_name`.
    PrivacyExport { file_name: String },
    /// Where to write a sanitized copy of a document (B3-15), suggesting `file_name`.
    SanitizedCopy { file_name: String },
    /// Where to write an encrypted copy of a document (B2-15), suggesting `file_name`.
    EncryptedCopy { file_name: String },
    /// Where to write a copy of a document with its pictures compressed (B3-01), suggesting
    /// `file_name`.
    CompressedCopy { file_name: String },
    /// Where to save some pages of a document as a file of their own (B2-06), suggesting
    /// `file_name`.
    SplitPdf { file_name: String },
    /// A folder for exported page images (B2-04), or for the files of a split document (B2-06),
    /// under the title the caller gives.
    PickFolder { title: &'static str },
    /// One file of language data for recognising text (B2-10).
    OpenLanguageData,
    /// A font file to import for text boxes (B2-32): one TTF or TTC file.
    OpenFont,
    /// The picture of a custom stamp (B2-08): one PNG or JPEG file.
    OpenImage,
    /// The picture of a watermark (B3-04): one PNG or JPEG file.
    OpenWatermarkImage,
    /// The files a PDF is made of (B3-03): several can be picked.
    OpenSources,
    /// Where to save a PDF made of files (B3-03), suggesting `file_name`.
    SaveCreatedPdf { file_name: String },
    /// Where to save an attachment from a document (B3-20, ADR 0025), suggesting `file_name`.
    SaveAttachment {
        file_name: String,
        extension: Option<String>,
    },
}

/// Shows the open dialog (PDF files only; several can be picked) over `window` and returns the
/// picked files, or `None` if the user cancelled.
pub async fn pick_pdfs(window: &WebviewWindow) -> Result<Option<Vec<PathBuf>>, IpcError> {
    run(window, Kind::OpenPdfs).await
}

/// Asks for the PDF files a batch is done to (B3-04); several can be picked. `None` if the user
/// cancelled.
pub async fn pick_batch_files(window: &WebviewWindow) -> Result<Option<Vec<PathBuf>>, IpcError> {
    run(window, Kind::OpenBatchFiles).await
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

/// Asks where to save an exported annotation summary (Issue #306), suggesting `file_name`.
pub async fn save_annotation_summary_file(
    window: &WebviewWindow,
    file_name: String,
    markdown: bool,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(
        window,
        Kind::SaveAnnotationSummary {
            file_name,
            markdown,
        },
    )
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

/// Asks where to save pages as a Word document (B3-02), suggesting `file_name`. The dialog itself
/// asks before replacing an existing file, the document's own included (the caller sees to that).
pub async fn save_docx_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SaveDocx { file_name })
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

/// Asks where to write a sanitized copy of a document (B3-15), suggesting `file_name`. The
/// dialog itself asks before replacing an existing file.
pub async fn sanitized_copy_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SanitizedCopy { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to write an encrypted copy of a document (B2-15), suggesting `file_name`. The
/// dialog itself asks before replacing an existing file.
pub async fn encrypted_copy_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::EncryptedCopy { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to write a copy of a document whose pictures are compressed (B3-01), suggesting
/// `file_name`. The dialog itself asks before replacing an existing file.
pub async fn compressed_copy_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::CompressedCopy { file_name })
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

/// Asks for the picture of a watermark (B3-04): one PNG or JPEG file. `None` if the user
/// cancelled.
pub async fn pick_watermark_image(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenWatermarkImage)
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

/// Asks for the files a PDF is made of (B3-03): several can be picked. `None` if the user
/// cancelled.
pub async fn pick_sources(window: &WebviewWindow) -> Result<Option<Vec<PathBuf>>, IpcError> {
    run(window, Kind::OpenSources).await
}

/// Asks where to save a PDF made of files (B3-03), suggesting `file_name`. The dialog itself asks
/// before replacing an existing file.
pub async fn save_created_pdf_file(
    window: &WebviewWindow,
    file_name: String,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::SaveCreatedPdf { file_name })
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks where to save an attachment from a document (B3-20, ADR 0025), suggesting `file_name` and
/// optional `extension`. The dialog asks before replacing an existing file.
pub async fn save_attachment_file(
    window: &WebviewWindow,
    file_name: String,
    extension: Option<String>,
) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(
        window,
        Kind::SaveAttachment {
            file_name,
            extension,
        },
    )
    .await?
    .and_then(|mut paths| paths.pop()))
}

/// Asks for one file of language data for recognising text (`.traineddata`, B2-10).
pub async fn pick_language_data(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenLanguageData)
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks for one font file to import for text boxes (`.ttf` or `.ttc`, B2-32).
pub async fn pick_font(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenFont)
        .await?
        .and_then(|mut paths| paths.pop()))
}

/// Asks for the PDF file whose pages are put into a document (B2-06): one file. `None` if the user
/// cancelled.
pub async fn pick_pages_source(window: &WebviewWindow) -> Result<Option<PathBuf>, IpcError> {
    Ok(run(window, Kind::OpenPagesSource)
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
        Kind::OpenPdfs | Kind::OpenBatchFiles | Kind::OpenSources => common | FOS_ALLOWMULTISELECT,
        Kind::SaveText { .. }
        | Kind::SaveAnnotationSummary { .. }
        | Kind::SavePdf { .. }
        | Kind::SaveDocx { .. }
        | Kind::PrivacyExport { .. }
        | Kind::SanitizedCopy { .. }
        | Kind::EncryptedCopy { .. }
        | Kind::CompressedCopy { .. }
        | Kind::SplitPdf { .. }
        | Kind::SaveCreatedPdf { .. }
        | Kind::SaveAttachment { .. } => common | FOS_OVERWRITEPROMPT,
        Kind::PickFolder { .. } => common | FOS_PICKFOLDERS,
        Kind::OpenLanguageData
        | Kind::OpenFont
        | Kind::OpenPagesSource
        | Kind::OpenImage
        | Kind::OpenWatermarkImage => common,
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
    let paths = if matches!(
        kind,
        Kind::OpenPdfs | Kind::OpenBatchFiles | Kind::OpenSources
    ) {
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
            | Kind::SaveAnnotationSummary { .. }
            | Kind::SavePdf { .. }
            | Kind::SaveDocx { .. }
            | Kind::PrivacyExport { .. }
            | Kind::SanitizedCopy { .. }
            | Kind::EncryptedCopy { .. }
            | Kind::CompressedCopy { .. }
            | Kind::SplitPdf { .. }
            | Kind::SaveCreatedPdf { .. }
            | Kind::SaveAttachment { .. } => {
                CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?
                    .cast()
            }
            Kind::OpenPdfs
            | Kind::OpenBatchFiles
            | Kind::OpenPagesSource
            | Kind::PickFolder { .. }
            | Kind::OpenLanguageData
            | Kind::OpenFont
            | Kind::OpenImage
            | Kind::OpenWatermarkImage
            | Kind::OpenSources => {
                CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?
                    .cast()
            }
        }
    }
}

/// The filters of the dialog for the files a PDF is made of (B3-03): all of them first, then one
/// for each family of files. The patterns come from the list of extensions the kinds are known by.
fn source_filters() -> Vec<(&'static str, String)> {
    let all = create_pdf::pattern(&create_pdf::KINDS);
    let of = |kinds: &[CreateKind]| create_pdf::pattern(kinds);
    vec![
        (strings::CREATE_ALL_FILTER_NAME, all),
        (strings::CREATE_IMAGES_FILTER_NAME, of(&[CreateKind::Image])),
        (strings::CREATE_HTML_FILTER_NAME, of(&[CreateKind::Html])),
        (strings::CREATE_TEXT_FILTER_NAME, of(&[CreateKind::Text])),
        (
            strings::CREATE_BOOKS_FILTER_NAME,
            of(&[CreateKind::Epub, CreateKind::Xps]),
        ),
        (strings::CREATE_SVG_FILTER_NAME, of(&[CreateKind::Svg])),
        (
            strings::CREATE_OFFICE_FILTER_NAME,
            of(&[CreateKind::Docx, CreateKind::Xlsx, CreateKind::Pptx]),
        ),
    ]
}

fn configure(dialog: &IFileDialog, kind: &Kind) -> windows::core::Result<()> {
    let one = |name: &'static str, pattern: &str| vec![(name, pattern.to_owned())];
    let (title, filters) = match kind {
        Kind::OpenPdfs => (
            strings::OPEN_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::OpenBatchFiles => (
            strings::BATCH_FILES_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SaveText { .. } => (
            strings::EXPORT_TEXT_DIALOG_TITLE,
            one(strings::TEXT_FILTER_NAME, "*.txt"),
        ),
        Kind::SaveAnnotationSummary { markdown, .. } => {
            if *markdown {
                (
                    strings::EXPORT_ANNOTATIONS_DIALOG_TITLE,
                    one(strings::MARKDOWN_FILTER_NAME, "*.md"),
                )
            } else {
                (
                    strings::EXPORT_ANNOTATIONS_DIALOG_TITLE,
                    one(strings::TEXT_FILTER_NAME, "*.txt"),
                )
            }
        }
        Kind::SavePdf { .. } => (
            strings::SAVE_AS_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SaveDocx { .. } => (
            strings::EXPORT_DOCX_DIALOG_TITLE,
            one(strings::DOCX_FILTER_NAME, "*.docx"),
        ),
        Kind::PrivacyExport { .. } => (
            strings::PRIVACY_EXPORT_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SanitizedCopy { .. } => (
            strings::SANITIZED_COPY_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::EncryptedCopy { .. } => (
            strings::ENCRYPTED_COPY_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::CompressedCopy { .. } => (
            strings::COMPRESSED_COPY_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SplitPdf { .. } => (
            strings::SPLIT_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SaveCreatedPdf { .. } => (
            strings::CREATE_PDF_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::SaveAttachment { extension, .. } => {
            // Its own type first, so that the name keeps its extension.
            let mut filters = Vec::new();
            if let Some(extension) = extension {
                filters.push((
                    strings::ATTACHMENT_TYPE_FILTER_NAME,
                    format!("*.{extension}"),
                ));
            }
            filters.push((strings::ALL_FILES_FILTER_NAME, "*.*".to_owned()));
            (strings::SAVE_ATTACHMENT_DIALOG_TITLE, filters)
        }
        Kind::OpenPagesSource => (
            strings::PAGES_SOURCE_DIALOG_TITLE,
            one(strings::PDF_FILTER_NAME, "*.pdf"),
        ),
        Kind::PickFolder { title } => (*title, Vec::new()),
        Kind::OpenLanguageData => (
            strings::LANGUAGE_DIALOG_TITLE,
            one(strings::LANGUAGE_FILTER_NAME, "*.traineddata"),
        ),
        Kind::OpenFont => (
            strings::FONT_DIALOG_TITLE,
            one(strings::FONT_FILTER_NAME, "*.ttf;*.ttc"),
        ),
        Kind::OpenImage => (
            strings::STAMP_IMAGE_DIALOG_TITLE,
            one(strings::IMAGE_FILTER_NAME, "*.png;*.jpg;*.jpeg"),
        ),
        Kind::OpenWatermarkImage => (
            strings::WATERMARK_IMAGE_DIALOG_TITLE,
            one(strings::IMAGE_FILTER_NAME, "*.png;*.jpg;*.jpeg"),
        ),
        Kind::OpenSources => (strings::CREATE_SOURCES_DIALOG_TITLE, source_filters()),
    };
    let title = HSTRING::from(title);
    let texts: Vec<(HSTRING, HSTRING)> = filters
        .iter()
        .map(|(name, pattern)| (HSTRING::from(*name), HSTRING::from(pattern.as_str())))
        .collect();
    let specs: Vec<COMDLG_FILTERSPEC> = texts
        .iter()
        .map(|(name, pattern)| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(pattern.as_ptr()),
        })
        .collect();
    // SAFETY: the strings outlive the calls, which copy them; each filter spec points at the
    // strings of `texts`, which are alive until the end of the function.
    unsafe {
        dialog.SetOptions(options(kind, dialog.GetOptions()?))?;
        dialog.SetTitle(&title)?;
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
        }
        if let Kind::SaveText { file_name }
        | Kind::SaveAnnotationSummary { file_name, .. }
        | Kind::SavePdf { file_name }
        | Kind::SaveDocx { file_name }
        | Kind::PrivacyExport { file_name }
        | Kind::SanitizedCopy { file_name }
        | Kind::EncryptedCopy { file_name }
        | Kind::CompressedCopy { file_name }
        | Kind::SplitPdf { file_name }
        | Kind::SaveCreatedPdf { file_name } = kind
        {
            let extension = match kind {
                Kind::SaveText { .. } => "txt",
                Kind::SaveDocx { .. } => "docx",
                Kind::SaveAnnotationSummary { markdown: true, .. } => "md",
                Kind::SaveAnnotationSummary {
                    markdown: false, ..
                } => "txt",
                _ => "pdf",
            };
            dialog.SetDefaultExtension(&HSTRING::from(extension))?;
            dialog.SetFileName(&HSTRING::from(file_name.as_str()))?;
        }
        if let Kind::SaveAttachment {
            file_name,
            extension,
        } = kind
        {
            if let Some(extension) = extension {
                dialog.SetDefaultExtension(&HSTRING::from(extension.as_str()))?;
            }
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
        let batch = options_of(Kind::OpenBatchFiles);
        assert!(
            batch(FOS_DONTADDTORECENT)
                && batch(FOS_ALLOWMULTISELECT)
                && batch(FOS_FORCEFILESYSTEM)
                && batch(FOS_FILEMUSTEXIST)
        );

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

        let summary = options_of(Kind::SaveAnnotationSummary {
            file_name: "註解.md".to_owned(),
            markdown: true,
        });
        assert!(
            summary(FOS_DONTADDTORECENT)
                && summary(FOS_OVERWRITEPROMPT)
                && summary(FOS_FORCEFILESYSTEM)
        );

        let save_as = options_of(Kind::SavePdf {
            file_name: "報告.pdf".to_owned(),
        });
        assert!(save_as(FOS_DONTADDTORECENT) && save_as(FOS_OVERWRITEPROMPT));

        let docx = options_of(Kind::SaveDocx {
            file_name: "報告.docx".to_owned(),
        });
        assert!(docx(FOS_DONTADDTORECENT) && docx(FOS_OVERWRITEPROMPT));

        let privacy = options_of(Kind::PrivacyExport {
            file_name: "報告（隱私匯出）.pdf".to_owned(),
        });
        assert!(privacy(FOS_DONTADDTORECENT) && privacy(FOS_OVERWRITEPROMPT));

        let sanitized = options_of(Kind::SanitizedCopy {
            file_name: "報告（淨化副本）.pdf".to_owned(),
        });
        assert!(sanitized(FOS_DONTADDTORECENT) && sanitized(FOS_OVERWRITEPROMPT));

        let encrypted = options_of(Kind::EncryptedCopy {
            file_name: "報告（已加密）.pdf".to_owned(),
        });
        assert!(encrypted(FOS_DONTADDTORECENT) && encrypted(FOS_OVERWRITEPROMPT));

        let compressed = options_of(Kind::CompressedCopy {
            file_name: "報告（已壓縮）.pdf".to_owned(),
        });
        assert!(compressed(FOS_DONTADDTORECENT) && compressed(FOS_OVERWRITEPROMPT));

        let split = options_of(Kind::SplitPdf {
            file_name: "報告-p2-4.pdf".to_owned(),
        });
        assert!(split(FOS_DONTADDTORECENT) && split(FOS_OVERWRITEPROMPT));

        // An attachment (B3-20) with an extension, and one without.
        for extension in [Some("txt".to_owned()), None] {
            let attachment = options_of(Kind::SaveAttachment {
                file_name: "附件 1".to_owned(),
                extension,
            });
            assert!(attachment(FOS_DONTADDTORECENT) && attachment(FOS_OVERWRITEPROMPT));
        }

        let language = options_of(Kind::OpenLanguageData);
        assert!(
            language(FOS_DONTADDTORECENT) && language(FOS_FORCEFILESYSTEM),
            "nothing picked goes on the recent items"
        );
        assert!(!language(FOS_ALLOWMULTISELECT), "one file");

        let font = options_of(Kind::OpenFont);
        assert!(
            font(FOS_DONTADDTORECENT) && font(FOS_FORCEFILESYSTEM),
            "nothing picked goes on the recent items"
        );
        assert!(!font(FOS_ALLOWMULTISELECT), "one file");

        // One file whose pages go into the document (B2-06), not several.
        let source = options_of(Kind::OpenPagesSource);
        assert!(
            source(FOS_DONTADDTORECENT) && source(FOS_FORCEFILESYSTEM) && source(FOS_FILEMUSTEXIST)
        );
        assert!(!source(FOS_ALLOWMULTISELECT));

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

        // One picture for a watermark (B3-04), not several.
        let watermark_image = options_of(Kind::OpenWatermarkImage);
        assert!(
            watermark_image(FOS_DONTADDTORECENT)
                && watermark_image(FOS_FORCEFILESYSTEM)
                && watermark_image(FOS_FILEMUSTEXIST)
        );
        assert!(!watermark_image(FOS_ALLOWMULTISELECT));

        // Several files a PDF is made of (B3-03), and where the PDF goes.
        let sources = options_of(Kind::OpenSources);
        assert!(
            sources(FOS_DONTADDTORECENT)
                && sources(FOS_FORCEFILESYSTEM)
                && sources(FOS_FILEMUSTEXIST)
                && sources(FOS_ALLOWMULTISELECT)
        );
        let created = options_of(Kind::SaveCreatedPdf {
            file_name: "test.pdf".to_owned(),
        });
        assert!(
            created(FOS_DONTADDTORECENT)
                && created(FOS_OVERWRITEPROMPT)
                && created(FOS_FORCEFILESYSTEM)
        );
    }

    #[test]
    fn the_dialog_for_the_files_of_a_pdf_lists_all_kinds_then_each_family() {
        let filters = source_filters();
        assert_eq!(filters.len(), 7);
        // The first filter has every extension of every kind, and the others add up to it.
        let listed =
            |pattern: &str| -> Vec<String> { pattern.split(';').map(str::to_owned).collect() };
        let mut together: Vec<String> = filters[1..]
            .iter()
            .flat_map(|(_, pattern)| listed(pattern))
            .collect();
        let mut all = listed(&filters[0].1);
        together.sort();
        all.sort();
        assert_eq!(together, all);
        assert!(all.contains(&"*.docx".to_owned()) && all.contains(&"*.png".to_owned()));
        for (name, pattern) in &filters {
            assert!(!name.is_empty() && !pattern.is_empty());
        }
        // A real dialog object takes them all (`SetFileTypes` fails otherwise).
        let _com = Com::init().unwrap();
        let dialog = new_dialog(&Kind::OpenSources).unwrap();
        configure(&dialog, &Kind::OpenSources).unwrap();
    }
}
