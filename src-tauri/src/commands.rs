//! Tauri commands for opening and closing documents (docs/architecture/ipc-contract.md).
//! Paths never cross into the WebView: the dialog runs here, and results arrive as
//! [`OpenEvent`]s carrying only a `TabId`, a `DocumentId` and a file name.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ipc_contract::limits::MAX_BATCH_FILES;
use ipc_contract::types::{
    AnnotationId, AnnotationSummaryFormat, AnnotationSummaryOutcome, AttachmentId, AttachmentInfo,
    BatchArgs, BatchEvent, BatchOperation, BatchReport, CompressArgs, CompressResult, CreateResult,
    DocumentId, DocumentInfo, EditArgs, EncryptArgs, ErrorCode, ExportAnnotationSummaryArgs,
    ExportArgs, ExportEvent, ExportFormat, FileRecordingArgs, FindPersonalDataQuery, FontImport,
    FontInfo, FormField, IpcError, LanguageImport, LinkArgs, LinkPreview, MarkCheck, OcrArgs,
    OcrFocusArgs, OcrLanguages, OpenEvent, OutlineLinkArgs, OutlineResult, PageAnnotation,
    PageLink, PageText, PagesSource, Password, PersonalDataMatches, PrepareTextBoxArgs,
    PreparedTextBox, RecentFile, RecentId, RedactionSummary, RememberedPasswords, RemoveFontArgs,
    RemoveLanguageArgs, RenderPageArgs, RequestId, SanitizeArgs, SanitizeResult,
    SaveAttachmentResult, SaveResult, SearchArgs, SearchEvent, SensitiveArgs, SensitiveFile,
    SensitiveId, Settings, SignatureReport, StampImageInfo, TabId, TextBoxInfo, UndoArgs,
    UnlockArgs, UnlockOutcome, UnlockSourceArgs, UpdateCheck, WatermarkContent,
};
use ipc_contract::validate::{Validate, check_clipboard_text, check_page_index};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, DragDropEvent, Manager, WebviewWindow, Window, WindowEvent};

use crate::background;
use crate::batch::{self, Batches};
use crate::clipboard;
use crate::create_pdf;
use crate::documents::{self, Documents};
use crate::events::OpenEvents;
use crate::export::{self, Exports, ImageKind};
use crate::file_dialog;
use crate::ocr::Ocr;
use crate::recent::RecentFiles;
use crate::render::Renderer;
use crate::search::{self, Searches};
use crate::sensitive::SensitiveFiles;
use crate::settings::SettingsStore;
use crate::strings;

/// Registers the frontend's channel for [`OpenEvent`]s.
#[tauri::command]
pub async fn subscribe_open_events(
    app: AppHandle,
    on_event: Channel<OpenEvent>,
) -> Result<(), IpcError> {
    blocking(move || {
        let documents = app.state::<Documents>();
        let ocr = app.state::<Arc<Ocr>>();
        app.state::<OpenEvents>().subscribe(on_event, || {
            let mut events = documents.snapshot();
            events.extend(ocr.snapshot());
            events
        });
        Ok(())
    })
    .await
}

/// Set while the open dialog is showing.
static DIALOG_SHOWING: AtomicBool = AtomicBool::new(false);

/// Shows the system's open dialog (PDF files only; several can be picked; nothing is added to
/// the recent items of Windows, #86). Returns false if the user cancelled, or if a dialog is
/// already showing; otherwise every file gets a tab and the outcomes arrive on the open-events
/// channel.
#[tauri::command]
pub async fn open_document_dialog(app: AppHandle, window: WebviewWindow) -> Result<bool, IpcError> {
    // The dialog is modal to the window, but the page could still ask twice.
    if DIALOG_SHOWING.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    struct Showing;
    impl Drop for Showing {
        fn drop(&mut self) {
            DIALOG_SHOWING.store(false, Ordering::SeqCst);
        }
    }
    let _showing = Showing;

    let Some(files) = crate::file_dialog::pick_pdfs(&window).await? else {
        return Ok(false);
    };
    open_paths(&app, files);
    Ok(true)
}

/// Opens the file of a tab that failed to open again (after `workerCrashed`, `workerTimeout` or
/// `unreadable`), in the same tab. The outcome arrives on the open-events channel.
#[tauri::command]
pub async fn retry_open(app: AppHandle, tab: TabId) -> Result<(), IpcError> {
    blocking(move || {
        let result = app
            .state::<Documents>()
            .retry(tab, &|event| report(&app, event));
        after_tabs_changed(&app);
        result
    })
    .await
}

/// Tries a password on a tab whose file is encrypted (MVP-16). It goes to that tab's worker
/// only and is wiped afterwards; whether it opened the file arrives on the open channel. With
/// `remember` it is also remembered once it has opened the file (B2-25, ADR 0017), and the answer
/// says why it was not, if it was not.
#[tauri::command]
pub async fn unlock_tab(app: AppHandle, args: UnlockArgs) -> Result<UnlockOutcome, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let UnlockArgs {
        tab,
        password,
        remember,
    } = args;
    blocking(move || {
        let result = app
            .state::<Documents>()
            .unlock(tab, password, remember, &|event| report(&app, event));
        after_tabs_changed(&app);
        result
    })
    .await
}

/// Forgets the password the app remembers for the file of a tab (B2-25, ADR 0017). The document
/// stays open; its new state arrives on the open-events channel.
#[tauri::command]
pub async fn forget_password(app: AppHandle, tab: TabId) -> Result<(), IpcError> {
    blocking(move || {
        let event = app.state::<Documents>().forget_password(tab)?;
        app.state::<OpenEvents>().send(event);
        Ok(())
    })
    .await
}

/// Forgets every password the app remembers (B2-25, ADR 0017); answers how many there were.
#[tauri::command]
pub async fn forget_all_passwords(app: AppHandle) -> Result<u32, IpcError> {
    blocking(move || app.state::<Documents>().forget_all_passwords()).await
}

/// How many passwords the app remembers, and the most it will (B2-25, ADR 0017).
#[tauri::command]
pub async fn get_remembered_passwords(app: AppHandle) -> Result<RememberedPasswords, IpcError> {
    blocking(move || app.state::<Documents>().remembered_passwords()).await
}

/// Asks for the PDF file whose pages go into `doc` (B2-06, docs/architecture/merge.md) in the
/// system's open dialog, and makes a clean copy of it, which the document keeps. The path stays
/// in the main process: the file is opened here, read-only, and `doc`'s worker, which scans it
/// and writes the copy, gets that handle. `null` when the user closes the dialog; an encrypted
/// file answers `encrypted`, and `unlock_pages_source` gives the password.
#[tauri::command]
pub async fn pick_pages_source(
    app: AppHandle,
    window: WebviewWindow,
    doc: DocumentId,
) -> Result<Option<PagesSource>, IpcError> {
    // The user is not asked for a file the author does not let the document take pages from.
    app.state::<Documents>().check_can_insert_pages(doc)?;
    let Some(path) = file_dialog::pick_pages_source(&window).await? else {
        return Ok(None);
    };
    blocking(move || {
        app.state::<Documents>()
            .prepare_pages_source(doc, &path, None)
            .map(Some)
    })
    .await
}

/// Tries a password on the encrypted file `pick_pages_source` just could not open (B2-06). It
/// goes to the document's worker only and is wiped afterwards. A wrong one answers `encrypted`
/// again, and the file waits for another.
#[tauri::command]
pub async fn unlock_pages_source(
    app: AppHandle,
    args: UnlockSourceArgs,
) -> Result<PagesSource, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let UnlockSourceArgs { doc, password } = args;
    blocking(move || app.state::<Documents>().unlock_pages_source(doc, password)).await
}

/// Closes a tab (MVP-14): its document's worker ends and its path is forgotten.
#[tauri::command]
pub async fn close_tab(app: AppHandle, tab: TabId) -> Result<(), IpcError> {
    blocking(move || {
        let result = app.state::<Documents>().close(tab);
        after_tabs_changed(&app);
        result
    })
    .await
}

/// The tab the window shows (`None` when there is none), for the window title (MVP-14). The
/// title uses the file name the main process has for that tab.
#[tauri::command]
pub async fn set_active_tab(app: AppHandle, tab: Option<TabId>) -> Result<(), IpcError> {
    blocking(move || {
        app.state::<Documents>().set_active(tab);
        show_active_in_title(&app);
        Ok(())
    })
    .await
}

/// The recently opened files (#73): file names and ids; the paths stay in the main process.
#[tauri::command]
pub async fn get_recent_files(app: AppHandle) -> Result<Vec<RecentFile>, IpcError> {
    blocking(move || checked(app.state::<RecentFiles>().list())).await
}

/// Opens a recently opened file in a new tab, by its id. A file that is gone is taken off the
/// list and reported as `unreadable`; otherwise the outcome arrives on the open-events channel.
#[tauri::command]
pub async fn open_recent_file(app: AppHandle, id: RecentId) -> Result<(), IpcError> {
    blocking(move || {
        let recent = app.state::<RecentFiles>();
        let path = recent.path(id).ok_or_else(unknown_recent_file)?;
        if !path.is_file() {
            let _ = recent.remove(id);
            return Err(IpcError {
                code: ErrorCode::Unreadable,
                message: "the file is no longer there".to_owned(),
            });
        }
        open_paths(&app, vec![path]);
        Ok(())
    })
    .await
}

/// Takes a file off the recent files list and returns the list.
#[tauri::command]
pub async fn remove_recent_file(app: AppHandle, id: RecentId) -> Result<Vec<RecentFile>, IpcError> {
    blocking(move || {
        let list = app
            .state::<RecentFiles>()
            .remove(id)
            .map_err(|_| unknown_recent_file())?;
        checked(list)
    })
    .await
}

/// Empties the recent files list; the files the user asked not to record stay unrecorded.
#[tauri::command]
pub async fn clear_recent_files(app: AppHandle) -> Result<(), IpcError> {
    blocking(move || {
        app.state::<RecentFiles>().clear();
        // With them, the unsaved changes earlier runs left (B2-13): they name the files too.
        app.state::<Documents>().clear_unused_journals();
        Ok(())
    })
    .await
}

/// Whether the file of an open document may be on the recent files list ("不記錄此檔案").
#[tauri::command]
pub async fn get_file_recording(app: AppHandle, doc: DocumentId) -> Result<bool, IpcError> {
    blocking(move || {
        let path = document_path(&app, doc)?;
        Ok(app.state::<RecentFiles>().is_recorded(&path))
    })
    .await
}

/// Sets whether the file of an open document may be on the recent files list. Not recording it
/// takes it off the list; the choice is kept as a salted hash, not as the path.
#[tauri::command]
pub async fn set_file_recording(app: AppHandle, args: FileRecordingArgs) -> Result<(), IpcError> {
    blocking(move || {
        let path = document_path(&app, args.doc)?;
        let list = app.state::<SettingsStore>().get().record_recent_files;
        app.state::<RecentFiles>()
            .set_recorded(&path, args.record, list)
            .map_err(|_| IpcError {
                code: ErrorCode::Internal,
                message: "no randomness for the list of files not to record".to_owned(),
            })
    })
    .await
}

/// Forgets which files the user asked not to record (the settings page, B2-12).
#[tauri::command]
pub async fn clear_recent_exclusions(app: AppHandle) -> Result<(), IpcError> {
    blocking(move || {
        app.state::<RecentFiles>().forget_exclusions();
        Ok(())
    })
    .await
}

/// What writing the sensitive mark into an open document's file, or taking it out, would do
/// (B3-05, ADR 0010): asked before the user confirms.
#[tauri::command]
pub async fn check_sensitive_mark(app: AppHandle, doc: DocumentId) -> Result<MarkCheck, IpcError> {
    blocking(move || app.state::<Documents>().mark_check(doc)).await
}

/// Writes the sensitive mark into an open document's file, or takes it out (B3-05, ADR 0004, ADR
/// 0010), once the user confirmed: appended to the file. The list of files marked on this
/// computer follows; a marked file leaves the recent files list, and one no longer marked goes
/// on it like a file that opens. The tab's new state arrives on the open-events channel.
#[tauri::command]
pub async fn set_sensitive(app: AppHandle, args: SensitiveArgs) -> Result<(), IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    blocking(move || {
        let SensitiveArgs {
            doc,
            sensitive,
            password,
        } = args;
        let (identifier, event) = app
            .state::<Documents>()
            .set_sensitive(doc, sensitive, password)?;
        let list = app.state::<SensitiveFiles>();
        match (&event, sensitive) {
            (OpenEvent::Opened { info, .. }, true) => list.add(identifier, &info.display_name),
            _ => list.remove_identifier(&identifier),
        }
        report(&app, event);
        Ok(())
    })
    .await
}

/// The files marked as sensitive on this computer (B3-05), most recently marked first: their
/// names, and whether each is open in a tab.
#[tauri::command]
pub async fn get_sensitive_files(app: AppHandle) -> Result<Vec<SensitiveFile>, IpcError> {
    blocking(move || sensitive_files(&app)).await
}

/// Takes a file off the list of files marked as sensitive on this computer (B3-05) and returns
/// the list. The file stays marked: the mark is in the file.
#[tauri::command]
pub async fn remove_sensitive_file(
    app: AppHandle,
    id: SensitiveId,
) -> Result<Vec<SensitiveFile>, IpcError> {
    blocking(move || {
        app.state::<SensitiveFiles>()
            .remove(id)
            .map_err(|_| IpcError {
                code: ErrorCode::InvalidArgument,
                message: "no such file on the list".to_owned(),
            })?;
        sensitive_files(&app)
    })
    .await
}

fn sensitive_files(app: &AppHandle) -> Result<Vec<SensitiveFile>, IpcError> {
    let open = app.state::<Documents>().open_identifiers();
    let list: Vec<SensitiveFile> = app
        .state::<SensitiveFiles>()
        .list()
        .into_iter()
        .map(|file| SensitiveFile {
            id: file.id,
            open: open.contains(&file.identifier),
            display_name: file.name,
        })
        .collect();
    list.validate().map_err(|error| IpcError {
        code: ErrorCode::Internal,
        message: format!("invalid list of sensitive files: {error}"),
    })?;
    Ok(list)
}

/// Puts text the page copied from a sensitive document on the clipboard (B3-05, ADR 0004), with
/// the formats that keep it out of Windows' clipboard history and cloud clipboard. The text is the
/// page's to copy anyway (`navigator.clipboard` writes it for any other document): nothing is
/// read from the clipboard, and nothing else is written.
#[tauri::command]
pub async fn copy_protected_text(window: WebviewWindow, text: String) -> Result<(), IpcError> {
    check_clipboard_text(&text).map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let owner = window
        .hwnd()
        .map_err(|_| IpcError {
            code: ErrorCode::Internal,
            message: "the window has no handle".to_owned(),
        })?
        .0 as isize;
    blocking(move || clipboard::write_protected(owner as _, &text)).await
}

/// The user's settings (B2-12).
#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Result<Settings, IpcError> {
    blocking(move || Ok(app.state::<SettingsStore>().get())).await
}

/// Replaces the user's settings (B2-12). They apply at once; an error only says they could not
/// be saved for the next run. Turning off the recent files list also empties it.
#[tauri::command]
pub async fn set_settings(app: AppHandle, settings: Settings) -> Result<(), IpcError> {
    settings.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    blocking(move || {
        let record_recent_files = settings.record_recent_files;
        let saved = app.state::<SettingsStore>().set(settings);
        if !record_recent_files {
            app.state::<RecentFiles>().clear();
            app.state::<Documents>().clear_unused_journals();
        }
        saved.map_err(|_| IpcError {
            code: ErrorCode::Unreadable,
            message: "the settings could not be saved".to_owned(),
        })
    })
    .await
}

/// The languages that scanned pages can be recognised in (B2-10, ADR 0015): those that came with
/// the app and the ones the user imported.
#[tauri::command]
pub async fn get_ocr_languages(app: AppHandle) -> Result<OcrLanguages, IpcError> {
    blocking(move || Ok(app.state::<Arc<Ocr>>().languages().list())).await
}

/// The fonts installed in Windows, for the text boxes (B2-27, ADR 0018): ids and names, never a
/// path. Read again every time, so that a font installed a moment ago is in the list.
#[tauri::command]
pub async fn get_fonts(app: AppHandle) -> Result<Vec<FontInfo>, IpcError> {
    blocking(move || Ok(app.state::<crate::fonts::Fonts>().list())).await
}

/// Makes the words a user typed, in a font from `get_fonts`, into what a text box of `doc` is made
/// of (B2-29, ADR 0018, docs/architecture/text-box-engine.md). The font is read here, by its id:
/// the page never gets a path, and the worker, which reads the font file's bytes (they are not
/// trusted), keeps only a small subset of it. The answer names the words for `Edit::AddTextBox`,
/// and says whether the font may only be viewed and printed. A font that cannot be used, or has no
/// glyph for a character, is refused with a code that says why.
#[tauri::command]
pub async fn prepare_text_box(
    app: AppHandle,
    args: PrepareTextBoxArgs,
) -> Result<PreparedTextBox, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    blocking(move || {
        let (name, file) = app
            .state::<crate::fonts::Fonts>()
            .file(args.font)
            .map_err(|error| IpcError {
                code: ErrorCode::FontUnavailable,
                message: format!("the font cannot be read: {error:?}"),
            })?;
        app.state::<Documents>().prepare_text_box(
            args.doc,
            &args.text,
            (name, file.bytes, file.face),
        )
    })
    .await
}

/// What text box `annotation` of page `page_index` of `doc` says of itself (B2-29): its words,
/// its font's name, its style and whether the words fit, to fill in the dialog that changes it.
/// Read from the file by the document's worker; the font is not needed.
#[tauri::command]
pub async fn get_text_box(
    app: AppHandle,
    doc: DocumentId,
    page_index: u32,
    annotation: AnnotationId,
) -> Result<TextBoxInfo, IpcError> {
    blocking(move || {
        app.state::<Documents>()
            .text_box(doc, page_index, annotation)
    })
    .await
}

/// Set while the font dialog is showing.
static FONT_DIALOG_SHOWING: AtomicBool = AtomicBool::new(false);

/// Asks for a `.ttf` or `.ttc` file in a dialog of the main process (the path stays here) and
/// imports it as a font (B2-32, ADR 0018): its name, size and format are checked, TrueType outlines
/// and metrics are verified by the worker, and a file that fails is refused with the reason.
/// Nothing is downloaded.
#[tauri::command]
pub async fn import_font(app: AppHandle, window: WebviewWindow) -> Result<FontImport, IpcError> {
    if FONT_DIALOG_SHOWING.swap(true, Ordering::SeqCst) {
        return Ok(FontImport::Cancelled);
    }
    struct Showing;
    impl Drop for Showing {
        fn drop(&mut self) {
            FONT_DIALOG_SHOWING.store(false, Ordering::SeqCst);
        }
    }
    let _showing = Showing;
    let Some(file) = file_dialog::pick_font(&window).await? else {
        return Ok(FontImport::Cancelled);
    };
    blocking(move || {
        let fonts = app.state::<crate::fonts::Fonts>();
        let documents = app.state::<Documents>();
        Ok(match fonts.import(&file, &documents) {
            Ok(fonts) => FontImport::Imported { fonts },
            Err(reason) => FontImport::Refused { reason },
        })
    })
    .await
}

/// Removes a font the user imported (B2-32); installed Windows fonts stay. Returns
/// the fonts there are now.
#[tauri::command]
pub async fn remove_font(app: AppHandle, args: RemoveFontArgs) -> Result<Vec<FontInfo>, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    blocking(move || {
        let fonts = app.state::<crate::fonts::Fonts>();
        fonts.remove(args.id).map_err(|_| IpcError {
            code: ErrorCode::InvalidArgument,
            message: "font could not be removed".to_owned(),
        })
    })
    .await
}

/// Removes all fonts the user imported (B2-32); installed Windows fonts stay. Returns
/// the fonts there are now.
#[tauri::command]
pub async fn remove_all_fonts(app: AppHandle) -> Result<Vec<FontInfo>, IpcError> {
    blocking(move || {
        let fonts = app.state::<crate::fonts::Fonts>();
        fonts.remove_all().map_err(|_| IpcError {
            code: ErrorCode::Internal,
            message: "imported fonts could not be removed".to_owned(),
        })
    })
    .await
}

/// Set while the language dialog is showing.
static LANGUAGE_DIALOG_SHOWING: AtomicBool = AtomicBool::new(false);

/// Asks for a `.traineddata` file in a dialog of the main process (the path stays here) and
/// imports it as a language (B2-10): its name, size and format are checked, and a file that fails
/// is refused with the reason. Nothing is downloaded.
#[tauri::command]
pub async fn import_ocr_language(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<LanguageImport, IpcError> {
    if LANGUAGE_DIALOG_SHOWING.swap(true, Ordering::SeqCst) {
        return Ok(LanguageImport::Cancelled);
    }
    struct Showing;
    impl Drop for Showing {
        fn drop(&mut self) {
            LANGUAGE_DIALOG_SHOWING.store(false, Ordering::SeqCst);
        }
    }
    let _showing = Showing;
    let Some(file) = file_dialog::pick_language_data(&window).await? else {
        return Ok(LanguageImport::Cancelled);
    };
    blocking(move || {
        let ocr = app.state::<Arc<Ocr>>();
        Ok(match ocr.languages().import(&file) {
            Ok(()) => LanguageImport::Imported {
                languages: ocr.languages().list(),
            },
            Err(reason) => LanguageImport::Refused { reason },
        })
    })
    .await
}

/// Removes a language the user imported (B2-10); the ones that came with the app stay. Returns
/// the languages there are now.
#[tauri::command]
pub async fn remove_ocr_language(
    app: AppHandle,
    args: RemoveLanguageArgs,
) -> Result<OcrLanguages, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    blocking(move || {
        let ocr = app.state::<Arc<Ocr>>();
        ocr.languages().remove(&args.code);
        Ok(ocr.languages().list())
    })
    .await
}

/// The tab that shows `doc`, if it is open (B2-10).
fn tab_of(app: &AppHandle, doc: DocumentId) -> Result<TabId, IpcError> {
    app.state::<Documents>().tab_of(doc).ok_or(IpcError {
        code: ErrorCode::UnknownDocument,
        message: "no such open document".to_owned(),
    })
}

/// Recognises the text of the scanned pages of an open document now, whatever the settings say
/// about doing it on its own (B2-10). Its progress arrives on the open-events channel.
#[tauri::command]
pub async fn start_ocr(app: AppHandle, args: OcrArgs) -> Result<(), IpcError> {
    blocking(move || {
        let tab = tab_of(&app, args.doc)?;
        app.state::<Arc<Ocr>>().start_tab(tab);
        Ok(())
    })
    .await
}

/// Stops recognising the text of an open document's pages; what was read stays (B2-10).
#[tauri::command]
pub async fn stop_ocr(app: AppHandle, args: OcrArgs) -> Result<(), IpcError> {
    blocking(move || {
        let tab = tab_of(&app, args.doc)?;
        app.state::<Arc<Ocr>>().stop_tab(tab);
        Ok(())
    })
    .await
}

/// The page of an open document that the user looks at, which is recognised first (B2-10).
#[tauri::command]
pub async fn set_ocr_focus(app: AppHandle, args: OcrFocusArgs) -> Result<(), IpcError> {
    blocking(move || {
        let documents = app.state::<Documents>();
        let pages = documents.page_count(args.doc).ok_or(IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
        check_page_index(args.page_index, pages).map_err(|error| IpcError {
            code: ErrorCode::InvalidArgument,
            message: error.to_string(),
        })?;
        let tab = tab_of(&app, args.doc)?;
        app.state::<Arc<Ocr>>().set_focus(tab, args.page_index);
        Ok(())
    })
    .await
}

fn document_path(app: &AppHandle, doc: DocumentId) -> Result<PathBuf, IpcError> {
    app.state::<Documents>()
        .document_path(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })
}

fn unknown_recent_file() -> IpcError {
    IpcError {
        code: ErrorCode::InvalidArgument,
        message: "no such recent file".to_owned(),
    }
}

/// The list as the frontend may see it: file names only, at most `MAX_RECENT_FILES`.
fn checked(list: Vec<RecentFile>) -> Result<Vec<RecentFile>, IpcError> {
    list.validate().map_err(|error| IpcError {
        code: ErrorCode::Internal,
        message: format!("invalid recent files list: {error}"),
    })?;
    Ok(list)
}

/// Opens the page of Windows Settings where PDF Reader can be made the default PDF app
/// (REL-03). The address is fixed; this command takes no arguments.
#[tauri::command]
pub async fn open_default_apps_settings(app: AppHandle) -> Result<(), IpcError> {
    crate::opener::open(&app, crate::opener::DEFAULT_APPS_SETTINGS.to_owned()).await
}

/// Asks GitHub whether a newer release exists (#64, ADR 0009): the app's one network request,
/// sent only when the user presses 「檢查更新」. It takes nothing from the page.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateCheck, IpcError> {
    let current = app.package_info().version.to_string();
    blocking(move || crate::update_check::check(&current)).await
}

/// What the link confirmation shows about the releases page (#64): a fixed address.
#[tauri::command]
pub fn describe_releases_page() -> Result<LinkPreview, IpcError> {
    crate::links::preview(crate::update_check::RELEASES_PAGE)
}

/// Opens the releases page once the user confirmed it (#64), as any web link is opened. The
/// address is fixed; this command takes no arguments.
#[tauri::command]
pub async fn open_releases_page(app: AppHandle) -> Result<(), IpcError> {
    let preview = crate::links::preview(crate::update_check::RELEASES_PAGE)?;
    crate::opener::open(&app, preview.opens).await
}

/// Renders a page. The answer is raw bytes (an `ArrayBuffer` in the page) in the layout of
/// `ipc_contract::raster`; see docs/architecture/ipc-contract.md.
#[tauri::command]
pub async fn render_page(app: AppHandle, args: RenderPageArgs) -> Result<Response, IpcError> {
    let bytes = app.state::<Renderer>().render(args).await?;
    Ok(Response::new(Vec::clone(&bytes)))
}

/// The document's outline (MVP-09): cleaned titles, targets checked against the page count.
#[tauri::command]
pub async fn get_outline(app: AppHandle, doc: DocumentId) -> Result<OutlineResult, IpcError> {
    blocking(move || app.state::<Documents>().outline(doc)).await
}

/// Returns the attachments of an open document (B3-20, ADR 0025).
#[tauri::command]
pub async fn get_attachments(
    app: AppHandle,
    doc: DocumentId,
) -> Result<Vec<AttachmentInfo>, IpcError> {
    blocking(move || app.state::<Documents>().get_attachments(doc)).await
}

/// Asks the user where to save an attachment, then decodes it and writes it with Mark of the Web
/// (B3-20, ADR 0025). Returns `None` if the user cancelled the dialog.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    window: WebviewWindow,
    doc: DocumentId,
    attachment: AttachmentId,
) -> Result<Option<SaveAttachmentResult>, IpcError> {
    let attachments = blocking({
        let app = app.clone();
        move || app.state::<Documents>().get_attachments(doc)
    })
    .await?;
    let found = attachments
        .into_iter()
        .find(|att| att.id == attachment)
        .ok_or_else(|| IpcError {
            code: ErrorCode::InvalidArgument,
            message: "no such attachment in document".to_owned(),
        })?;

    let ext = if found.extension.is_empty() {
        None
    } else {
        Some(found.extension.trim_start_matches('.').to_owned())
    };

    let Some(destination) =
        file_dialog::save_attachment_file(&window, found.name, ext).await?
    else {
        return Ok(None);
    };

    blocking(move || app.state::<Documents>().save_attachment(doc, attachment, destination))
        .await
        .map(Some)
}

/// One page's text for selecting and copying (MVP-15): its lines and where their characters
/// are on the page.
#[tauri::command]
pub async fn get_page_text(
    app: AppHandle,
    doc: DocumentId,
    page_index: u32,
) -> Result<PageText, IpcError> {
    blocking(move || app.state::<Documents>().page_text(doc, page_index)).await
}

/// The annotations of one page (B2-07): highlighter marks, notes and the document's own, each by
/// its number, to select, change or remove with `apply_edit`.
#[tauri::command]
pub async fn get_page_annotations(
    app: AppHandle,
    doc: DocumentId,
    page_index: u32,
) -> Result<Vec<PageAnnotation>, IpcError> {
    blocking(move || app.state::<Documents>().page_annotations(doc, page_index)).await
}

/// The form fields of one page (B2-09): where they are, what they hold and what can be put in
/// them. They are filled in with `apply_edit`.
#[tauri::command]
pub async fn get_page_fields(
    app: AppHandle,
    doc: DocumentId,
    page_index: u32,
) -> Result<Vec<FormField>, IpcError> {
    blocking(move || app.state::<Documents>().page_fields(doc, page_index)).await
}

/// The digital signatures of an open document (B2-14, ADR 0014), verified offline by the
/// document's own worker: whether each holds, whether the file changed after it, and who signed.
/// Nothing is fetched from anywhere.
#[tauri::command]
pub async fn get_signatures(app: AppHandle, doc: DocumentId) -> Result<SignatureReport, IpcError> {
    blocking(move || app.state::<Documents>().signatures(doc)).await
}

/// What applying the redaction marks of an open document would do besides removing what their
/// areas reach (B2-19, ADR 0016): how many marks on how many pages, and how many other annotations
/// and form fields would be deleted with them. The confirmation says it before anything goes.
#[tauri::command]
pub async fn get_redaction_summary(
    app: AppHandle,
    doc: DocumentId,
) -> Result<RedactionSummary, IpcError> {
    blocking(move || app.state::<Documents>().redaction_summary(doc)).await
}

/// Finds personal data matching `query` in `doc` (Issue #302).
#[tauri::command]
pub async fn find_personal_data(
    app: AppHandle,
    doc: DocumentId,
    query: FindPersonalDataQuery,
) -> Result<PersonalDataMatches, IpcError> {
    blocking(move || app.state::<Documents>().find_personal_data(doc, query)).await
}

/// The links of one page (MVP-12): where they are and where they point. Opening a web link
/// takes the link's id, never a URI from the frontend.
#[tauri::command]
pub async fn get_page_links(
    app: AppHandle,
    doc: DocumentId,
    page_index: u32,
) -> Result<Vec<PageLink>, IpcError> {
    blocking(move || app.state::<Documents>().page_links(doc, page_index)).await
}

/// What the confirmation shows about a web link (MVP-12). `args` names the link; the URI
/// comes from the worker and is checked here.
#[tauri::command]
pub async fn describe_link(app: AppHandle, args: LinkArgs) -> Result<LinkPreview, IpcError> {
    blocking(move || app.state::<Documents>().link_preview(args)).await
}

/// Opens a web link after the user confirmed it (MVP-12): only a link the worker reports, only
/// `http`, `https` or `mailto`, and only as the checked ASCII form, never a string from the
/// frontend.
#[tauri::command]
pub async fn open_link(app: AppHandle, args: LinkArgs) -> Result<(), IpcError> {
    let preview = {
        let app = app.clone();
        blocking(move || app.state::<Documents>().link_preview(args)).await?
    };
    crate::opener::open(&app, preview.opens).await
}

/// What the confirmation shows about the web link of an outline item (#49), named by its
/// position in the outline.
#[tauri::command]
pub async fn describe_outline_link(
    app: AppHandle,
    args: OutlineLinkArgs,
) -> Result<LinkPreview, IpcError> {
    blocking(move || app.state::<Documents>().outline_link_preview(args)).await
}

/// Opens the web link of an outline item after the user confirmed it (#49), with the same
/// checks as `open_link`.
#[tauri::command]
pub async fn open_outline_link(app: AppHandle, args: OutlineLinkArgs) -> Result<(), IpcError> {
    let preview = {
        let app = app.clone();
        blocking(move || app.state::<Documents>().outline_link_preview(args)).await?
    };
    crate::opener::open(&app, preview.opens).await
}

/// Searches the document (MVP-10); hits, progress and a final `done` arrive on `on_event`.
#[tauri::command]
pub async fn search(
    app: AppHandle,
    args: SearchArgs,
    on_event: Channel<SearchEvent>,
) -> Result<(), IpcError> {
    blocking(move || search::run(&app, args, on_event)).await
}

/// Exports pages of an open document as text or PNG files (B2-04). The WebView says what to
/// export; the main process asks where, in the system's dialogs, and writes the files. Returns
/// false if the user cancelled a dialog; progress arrives on `on_event`; `cancel(args.request)`
/// stops it.
#[tauri::command]
pub async fn export_pages(
    app: AppHandle,
    window: WebviewWindow,
    args: ExportArgs,
    on_event: Channel<ExportEvent>,
) -> Result<bool, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let info = app
        .state::<Documents>()
        .document_info(args.doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    // Exporting copies the content: the author's permission to copy covers it (MVP-19).
    if !info.permissions.copy {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "the document's author does not allow copying its content".to_owned(),
        });
    }
    let page_count = u32::try_from(info.pages.len()).unwrap_or(u32::MAX);
    if args.pages.iter().any(|page| *page >= page_count) {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "no such page".to_owned(),
        });
    }
    // The copy of an encrypted document could not be encrypted again (B2-06).
    if info.encrypted
        && matches!(
            args.format,
            ExportFormat::Pdf | ExportFormat::PdfEvery { .. }
        )
    {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an encrypted document cannot be split".to_owned(),
        });
    }
    let stem = export::stem(&info.display_name);
    let ExportArgs {
        request,
        doc,
        pages,
        format,
    } = args;
    match format {
        ExportFormat::Text => {
            let Some(file) = file_dialog::save_text_file(&window, format!("{stem}.txt")).await?
            else {
                return Ok(false);
            };
            blocking(move || export::write_text(&app, request, doc, &pages, &file, &on_event))
                .await?;
        }
        ExportFormat::Docx => {
            let destination = loop {
                let Some(path) =
                    file_dialog::save_docx_file(&window, format!("{stem}.docx")).await?
                else {
                    return Ok(false);
                };
                if !app.state::<Documents>().is_document_file(doc, &path) {
                    break path;
                }
                tell_same_file(&window).await;
            };
            blocking(move || {
                export::write_docx(&app, request, doc, &pages, &destination, &on_event)
            })
            .await?;
        }
        ExportFormat::Png { dpi } | ExportFormat::Jpg { dpi } => {
            let kind = if matches!(format, ExportFormat::Png { .. }) {
                ImageKind::Png
            } else {
                ImageKind::Jpeg
            };
            let Some(folder) =
                file_dialog::pick_folder(&window, strings::EXPORT_IMAGES_DIALOG_TITLE).await?
            else {
                return Ok(false);
            };
            let targets = export::image_targets(&folder, &stem, &pages, kind);
            let existing = targets.iter().filter(|target| target.exists()).count();
            if existing > 0 && !confirm_overwrite(&window, existing).await {
                return Ok(false);
            }
            blocking(move || {
                export::write_images(&app, request, doc, &pages, dpi, kind, &targets, &on_event)
            })
            .await?;
        }
        ExportFormat::Pdf => {
            let destination = loop {
                let name = strings::split_file_name(&stem, &pages);
                let Some(path) = file_dialog::split_pdf_file(&window, name).await? else {
                    return Ok(false);
                };
                if !app.state::<Documents>().is_document_file(doc, &path) {
                    break path;
                }
                tell_same_file(&window).await;
            };
            let files = vec![export::SplitFile {
                path: destination,
                pages,
            }];
            blocking(move || export::write_pdfs(&app, request, doc, &files, &on_event)).await?;
        }
        ExportFormat::PdfEvery { count } => {
            let Some(folder) =
                file_dialog::pick_folder(&window, strings::SPLIT_FOLDER_DIALOG_TITLE).await?
            else {
                return Ok(false);
            };
            let files = export::split_targets(&folder, &stem, &pages, count as usize);
            let documents = app.state::<Documents>();
            if files
                .iter()
                .any(|file| documents.is_document_file(doc, &file.path))
            {
                tell_same_file(&window).await;
                return Ok(false);
            }
            let existing = files.iter().filter(|file| file.path.exists()).count();
            if existing > 0 && !confirm_overwrite(&window, existing).await {
                return Ok(false);
            }
            blocking(move || export::write_pdfs(&app, request, doc, &files, &on_event)).await?;
        }
    }
    Ok(true)
}

/// Says, in a native message box, that the pages cannot replace the document's own file.
async fn tell_same_file(window: &WebviewWindow) {
    rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(strings::SPLIT_SAME_FILE_TITLE)
        .set_description(strings::SPLIT_SAME_FILE_MESSAGE)
        .set_buttons(rfd::MessageButtons::Ok)
        .set_parent(window)
        .show()
        .await;
}

/// Writes a summary of the annotations of document `doc` (B3-18,
/// docs/architecture/annotation-summary.md) where the user says in the system's save dialog,
/// never over the document's own file. The summary lists the text the marks cover, so the
/// author's permission to copy covers it (MVP-19). The main process makes the file from what the
/// worker found: no text of the document passes through the page.
#[tauri::command]
pub async fn export_annotation_summary(
    app: AppHandle,
    window: WebviewWindow,
    args: ExportAnnotationSummaryArgs,
) -> Result<AnnotationSummaryOutcome, IpcError> {
    let ExportAnnotationSummaryArgs {
        doc,
        format,
        include_name,
    } = args;
    let info = copy_allowed(&app, doc)?;
    let worker_app = app.clone();
    let summary = blocking(move || worker_app.state::<Documents>().annotation_summary(doc)).await?;
    if summary.items.is_empty() && !summary.truncated {
        return Ok(AnnotationSummaryOutcome::Empty);
    }
    let markdown = format == AnnotationSummaryFormat::Markdown;
    let content = export::annotation_summary_file(
        &summary,
        markdown,
        include_name.then_some(info.display_name.as_str()),
    );
    let name = strings::annotation_summary_file_name(&export::stem(&info.display_name), markdown);
    let destination = loop {
        let Some(path) =
            file_dialog::save_annotation_summary_file(&window, name.clone(), markdown).await?
        else {
            return Ok(AnnotationSummaryOutcome::Cancelled);
        };
        if !app.state::<Documents>().is_document_file(doc, &path) {
            break path;
        }
        tell_same_file(&window).await;
    };
    blocking(move || export::write_annotation_summary(&destination, &content)).await?;
    Ok(AnnotationSummaryOutcome::Written {
        count: u32::try_from(summary.items.len()).unwrap_or(u32::MAX),
        truncated: summary.truncated,
    })
}

/// The open document `doc`, if its author allows copying its content (MVP-19).
fn copy_allowed(app: &AppHandle, doc: DocumentId) -> Result<DocumentInfo, IpcError> {
    let info = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    if !info.permissions.copy {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "the document's author does not allow copying its content".to_owned(),
        });
    }
    Ok(info)
}

/// Writes a copy of the document without its metadata (B2-03, docs/architecture/privacy-export.md)
/// where the user says in the system's save dialog, never over the document's own file. `false`
/// when the user closes the dialog. The document itself does not change.
#[tauri::command]
pub async fn privacy_export(
    app: AppHandle,
    window: WebviewWindow,
    doc: DocumentId,
) -> Result<bool, IpcError> {
    let info = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    if info.encrypted {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an encrypted document has no privacy export".to_owned(),
        });
    }
    let file_name = strings::privacy_export_file_name(&export::stem(&info.display_name));
    let destination = loop {
        let Some(path) = file_dialog::privacy_export_file(&window, file_name.clone()).await? else {
            return Ok(false);
        };
        if !app.state::<Documents>().is_document_file(doc, &path) {
            break path;
        }
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::PRIVACY_EXPORT_SAME_FILE_TITLE)
            .set_description(strings::PRIVACY_EXPORT_SAME_FILE_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_parent(&window)
            .show()
            .await;
    };
    blocking(move || app.state::<Documents>().privacy_export(doc, &destination)).await?;
    Ok(true)
}

/// Writes a sanitized copy of the document without active content or hidden data (B3-15)
/// where the user says in the system's save dialog, never over the document's own file.
/// `None` when the user closes the dialog; else what was done. The document itself does not change.
#[tauri::command]
pub async fn sanitize_copy(
    app: AppHandle,
    window: WebviewWindow,
    args: SanitizeArgs,
) -> Result<Option<SanitizeResult>, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let doc = args.doc;
    let info = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    if info.encrypted {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an encrypted document has no sanitized copy".to_owned(),
        });
    }
    let file_name = strings::sanitized_copy_file_name(&export::stem(&info.display_name));
    let destination = loop {
        let Some(path) = file_dialog::sanitized_copy_file(&window, file_name.clone()).await? else {
            return Ok(None);
        };
        if !app.state::<Documents>().is_document_file(doc, &path) {
            break path;
        }
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::SANITIZED_COPY_SAME_FILE_TITLE)
            .set_description(strings::SANITIZED_COPY_SAME_FILE_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_parent(&window)
            .show()
            .await;
    };
    let result = blocking(move || {
        app.state::<Documents>()
            .sanitize_copy(doc, &destination, args.options)
    })
    .await?;
    Ok(Some(result))
}

/// Writes a copy of the document whose pictures are encoded again, smaller (B3-01,
/// docs/architecture/compress-images.md) where the user says in the system's save dialog, never
/// over the document's own file. `None` when the user closes the dialog; else what was done. The
/// document itself does not change.
#[tauri::command]
pub async fn compress_images(
    app: AppHandle,
    window: WebviewWindow,
    args: CompressArgs,
) -> Result<Option<CompressResult>, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let doc = args.doc;
    let info = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    if info.encrypted {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an encrypted document has no compressed copy".to_owned(),
        });
    }
    let file_name = strings::compressed_copy_file_name(&export::stem(&info.display_name));
    let destination = loop {
        let Some(path) = file_dialog::compressed_copy_file(&window, file_name.clone()).await?
        else {
            return Ok(None);
        };
        if !app.state::<Documents>().is_document_file(doc, &path) {
            break path;
        }
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::COMPRESSED_COPY_SAME_FILE_TITLE)
            .set_description(strings::COMPRESSED_COPY_SAME_FILE_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_parent(&window)
            .show()
            .await;
    };
    let result = blocking(move || {
        app.state::<Documents>()
            .compress_copy(doc, &destination, args.quality, args.max_side)
    })
    .await?;
    Ok(Some(result))
}

/// Writes a copy of the document encrypted with AES-256 (B2-15, docs/architecture/encrypt-copy.md)
/// where the user says in the system's save dialog, never over the document's own file. `false`
/// when the user closes the dialog. The document itself does not change. The passwords of `args`
/// are used once and not kept; the copy's permissions password is made up here when the user gave
/// none (only the open password was asked for), and thrown away.
#[tauri::command]
pub async fn encrypt_copy(
    app: AppHandle,
    window: WebviewWindow,
    args: EncryptArgs,
) -> Result<bool, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let doc = args.doc;
    let info = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?;
    if info.encrypted {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an encrypted document has no encrypted copy".to_owned(),
        });
    }
    let file_name = strings::encrypted_copy_file_name(&export::stem(&info.display_name));
    let destination = loop {
        let Some(path) = file_dialog::encrypted_copy_file(&window, file_name.clone()).await? else {
            return Ok(false);
        };
        if !app.state::<Documents>().is_document_file(doc, &path) {
            break path;
        }
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::ENCRYPTED_COPY_SAME_FILE_TITLE)
            .set_description(strings::ENCRYPTED_COPY_SAME_FILE_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_parent(&window)
            .show()
            .await;
    };
    blocking(move || {
        let owner_password = match args.permissions_password {
            Some(password) => password,
            None => unguessable_password()?,
        };
        app.state::<Documents>().encrypted_copy(
            doc,
            &destination,
            args.open_password,
            owner_password,
            args.restrictions,
        )
    })
    .await?;
    Ok(true)
}

/// Does `args.operation` to PDF files the user picks (B3-04, ADR 0005,
/// docs/architecture/batch.md): the system's open dialog for the files (several), then a folder
/// for the copies. The files are done one after the other, each in a worker of its own, and each
/// gets a copy in that folder; no file is changed or replaced, and one that fails does not stop
/// the others. Progress arrives on `on_event`, by file name; `cancel` stops the batch before its
/// next file. `None` when the user closes either dialog. The passwords of an encrypted copy go to
/// each file's worker and are dropped with `args` when the batch ends.
#[tauri::command]
pub async fn run_batch(
    app: AppHandle,
    window: WebviewWindow,
    args: BatchArgs,
    on_event: Channel<BatchEvent>,
) -> Result<Option<BatchReport>, IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let picture = match &args.operation {
        BatchOperation::Watermark { watermark } => match &watermark.content {
            WatermarkContent::Picture { .. } => {
                let Some(path) = file_dialog::pick_watermark_image(&window).await? else {
                    return Ok(None);
                };
                let docs_app = app.clone();
                let png = blocking(move || {
                    docs_app
                        .state::<Documents>()
                        .prepare_watermark_picture(&path)
                })
                .await?;
                Some(png)
            }
            WatermarkContent::Text { .. } => None,
        },
        _ => None,
    };
    let Some(files) = file_dialog::pick_batch_files(&window).await? else {
        return Ok(None);
    };
    if files.len() > MAX_BATCH_FILES as usize {
        return Err(IpcError {
            code: ErrorCode::LimitExceeded,
            message: format!("more than {MAX_BATCH_FILES} files"),
        });
    }
    let Some(folder) =
        file_dialog::pick_folder(&window, strings::BATCH_FOLDER_DIALOG_TITLE).await?
    else {
        return Ok(None);
    };
    let report = blocking(move || {
        let batches = app.state::<Batches>();
        let running = batches.start(args.request);
        // For the notification area icon and the report, if the window is closed meanwhile.
        let record = RefCell::new(background::Record::default());
        let report = batch::run(
            &app.state::<Documents>(),
            &files,
            &folder,
            &args.operation,
            picture.as_deref(),
            running.stop(),
            &|event| {
                record.borrow_mut().hear(&event);
                background::progress(&app, &record);
                let _ = on_event.send(event);
            },
        );
        drop(running);
        background::batch_ended(
            &app,
            background::Ended {
                operation: strings::batch_operation_name(&args.operation),
                folder,
                record: record.into_inner(),
                report,
            },
        );
        Ok(report)
    })
    .await?;
    Ok(Some(report))
}

/// A password nobody knows, not even the user: 128 random bits as hex text. The permissions
/// password of a copy that only has an open password (the PDF standard has the field either way).
pub(crate) fn unguessable_password() -> Result<Password, IpcError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| IpcError {
        code: ErrorCode::Internal,
        message: "no random bytes for a password".to_owned(),
    })?;
    Ok(Password::new(
        bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    ))
}

/// Set while a PDF is being made of files (B3-03), from the open dialog to the end.
static CREATING: AtomicBool = AtomicBool::new(false);

/// Makes a PDF of files the user picks (B3-03, docs/architecture/create-pdf.md): the system's open
/// dialog for the files (pictures, a web page, a text, a book, an XPS file, an SVG picture or an
/// Office file; several pictures can go together, any other kind alone), then the save dialog for
/// the PDF, never over one of the files. A sandboxed worker reads the files and makes the PDF; it
/// then opens in a tab. Paths stay in the main process. `None` when the user closes either dialog,
/// or when a PDF is already being made.
#[tauri::command]
pub async fn create_pdf(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<CreateResult>, IpcError> {
    if CREATING.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    struct Creating;
    impl Drop for Creating {
        fn drop(&mut self) {
            CREATING.store(false, Ordering::SeqCst);
        }
    }
    let _creating = Creating;

    let Some(picked) = file_dialog::pick_sources(&window).await? else {
        return Ok(None);
    };
    let sources = create_pdf::sources_of(picked)?;
    let file_name =
        strings::created_pdf_file_name(&export::stem(&documents::display_name(&sources[0].0)));
    let destination = loop {
        let Some(path) = file_dialog::save_created_pdf_file(&window, file_name.clone()).await?
        else {
            return Ok(None);
        };
        if !sources
            .iter()
            .any(|(source, _)| documents::same_file(source, &path))
        {
            break path;
        }
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(strings::CREATE_PDF_SAME_FILE_TITLE)
            .set_description(strings::CREATE_PDF_SAME_FILE_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .set_parent(&window)
            .show()
            .await;
    };
    let created = {
        let (app, destination) = (app.clone(), destination.clone());
        blocking(move || app.state::<Documents>().create_pdf(&sources, &destination)).await?
    };
    open_paths(&app, vec![destination]);
    Ok(Some(created))
}

/// Asks for a picture (PNG or JPEG) in the system's open dialog and makes it into what a stamp
/// of `doc` is made of (B2-08, docs/architecture/annotations.md). The path stays in the main
/// process: the file is opened here, read-only, and the document's worker, which keeps only its
/// pixels, gets that handle. `None` when the user closes the dialog.
#[tauri::command]
pub async fn pick_stamp_image(
    app: AppHandle,
    window: WebviewWindow,
    doc: DocumentId,
) -> Result<Option<StampImageInfo>, IpcError> {
    let Some(path) = file_dialog::pick_image(&window).await? else {
        return Ok(None);
    };
    blocking(move || {
        let file = crate::documents::open_picture(&path)?;
        app.state::<Documents>()
            .prepare_stamp_image(doc, &file)
            .map(Some)
    })
    .await
}

/// Asks, in a native message box, whether exported files may replace `count` existing ones.
async fn confirm_overwrite(window: &WebviewWindow, count: usize) -> bool {
    let answer = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(strings::OVERWRITE_TITLE)
        .set_description(strings::overwrite_message(count))
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_parent(window)
        .show()
        .await;
    answer == rfd::MessageDialogResult::Yes
}

/// Applies an edit to an open document (B2-02, ADR 0013). The document gets a new id; the tab's
/// new state arrives on the open-events channel. Only saving writes the file.
#[tauri::command]
pub async fn apply_edit(app: AppHandle, args: EditArgs) -> Result<(), IpcError> {
    blocking(move || {
        let event = app.state::<Documents>().apply_edit(&args)?;
        app.state::<OpenEvents>().send(event);
        after_tabs_changed(&app);
        Ok(())
    })
    .await
}

/// Undoes the last edit of an open document (B2-05, ADR 0013). As with `apply_edit`, the tab's
/// new state (a new `DocumentId`) arrives on the open-events channel.
#[tauri::command]
pub async fn undo_edit(app: AppHandle, args: UndoArgs) -> Result<(), IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let UndoArgs { doc, password } = args;
    blocking(move || {
        let event = app.state::<Documents>().undo(doc, password)?;
        app.state::<OpenEvents>().send(event);
        after_tabs_changed(&app);
        Ok(())
    })
    .await
}

/// Makes the last undone edit of an open document again (B2-05).
#[tauri::command]
pub async fn redo_edit(app: AppHandle, doc: DocumentId) -> Result<(), IpcError> {
    blocking(move || {
        let event = app.state::<Documents>().redo(doc)?;
        app.state::<OpenEvents>().send(event);
        after_tabs_changed(&app);
        Ok(())
    })
    .await
}

/// Makes the edits an earlier run left for an open document's file again (B2-13, crash
/// recovery). As with `apply_edit`, the tab's new state arrives on the open-events channel.
#[tauri::command]
pub async fn recover_edits(app: AppHandle, doc: DocumentId) -> Result<(), IpcError> {
    blocking(move || {
        let event = app.state::<Documents>().recover(doc)?;
        app.state::<OpenEvents>().send(event);
        after_tabs_changed(&app);
        Ok(())
    })
    .await
}

/// Discards the edits an earlier run left for an open document's file (B2-13): their journal is
/// deleted. The tab's new state arrives on the open-events channel.
#[tauri::command]
pub async fn discard_recovered_edits(app: AppHandle, doc: DocumentId) -> Result<(), IpcError> {
    blocking(move || {
        let event = app.state::<Documents>().discard_recovered(doc)?;
        app.state::<OpenEvents>().send(event);
        Ok(())
    })
    .await
}

/// Writes an open document, with its edits, to its own file (B2-02, ADR 0013). The tab's new
/// state arrives on the open-events channel.
#[tauri::command]
pub async fn save_document(app: AppHandle, doc: DocumentId) -> Result<SaveResult, IpcError> {
    blocking(move || {
        let (result, event) = app.state::<Documents>().save(doc, None)?;
        app.state::<OpenEvents>().send(event);
        show_active_in_title(&app);
        Ok(result)
    })
    .await
}

/// Writes an open document to another file, which the user picks in the system's save dialog
/// (B2-02): `None` if they closed the dialog. The tab then stands for the new file, which goes
/// on the recent files list like any file that opens (#73).
#[tauri::command]
pub async fn save_document_as(
    app: AppHandle,
    window: WebviewWindow,
    doc: DocumentId,
) -> Result<Option<SaveResult>, IpcError> {
    let name = app
        .state::<Documents>()
        .document_info(doc)
        .ok_or_else(|| IpcError {
            code: ErrorCode::UnknownDocument,
            message: "no such open document".to_owned(),
        })?
        .display_name;
    let Some(destination) = file_dialog::save_pdf_file(&window, name).await? else {
        return Ok(None);
    };
    blocking(move || {
        let (result, event) = app.state::<Documents>().save(doc, Some(destination))?;
        report(&app, event);
        show_active_in_title(&app);
        Ok(Some(result))
    })
    .await
}

/// Closes the window once the user was asked about unsaved changes (B2-02): with some left, only
/// if they chose to `discard` them. The page may not have known of them when it asked to close
/// (a value it was still sending, #153): then the window stays open and the page is asked to ask.
#[tauri::command]
pub async fn close_window(
    app: AppHandle,
    window: WebviewWindow,
    discard: bool,
) -> Result<(), IpcError> {
    // The page answered (see `close_if_unanswered`).
    CLOSE_ANSWERED.store(true, Ordering::SeqCst);
    let documents = app.state::<Documents>();
    let unsaved = documents.unsaved_tabs();
    if !discard && !unsaved.is_empty() {
        app.state::<OpenEvents>()
            .send(OpenEvent::CloseRequested { tabs: unsaved });
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "documents have unsaved changes".to_owned(),
        });
    }
    // While a batch runs, the window is only hidden, and closes when the batch is done (B3-04,
    // ADR 0005): the changes are kept until then, for the user may show it again.
    if background::hide_while_batching(&app, discard) {
        return Ok(());
    }
    // The changes are discarded: nothing is left to recover them from (B2-13).
    documents.discard_unsaved();
    // Unlike `close`, `destroy` does not ask again (see `on_window_event`).
    window.destroy().map_err(|_| IpcError {
        code: ErrorCode::Internal,
        message: "the window could not be closed".to_owned(),
    })
}

/// Cancels a queued `render_page` request (it then fails with `cancelled`), a running search or a
/// running export.
#[tauri::command]
pub async fn cancel(app: AppHandle, request: RequestId) -> Result<(), IpcError> {
    app.state::<Renderer>().cancel(request);
    app.state::<Searches>().cancel(request);
    app.state::<Exports>().cancel(request);
    app.state::<Batches>().cancel(request);
    Ok(())
}

/// How long the page has to answer a request to close the window that names no unsaved tab. A page
/// that has none to ask about answers at once (it calls `close_window`), so this only matters for
/// one that does not answer at all: the window must still close.
const CLOSE_ANSWER: Duration = Duration::from_secs(5);

/// Whether the page has called `close_window` since the window was last asked to close with no tab
/// known to have unsaved changes.
static CLOSE_ANSWERED: AtomicBool = AtomicBool::new(false);

/// Closes `window` if, after `CLOSE_ANSWER`, the page has not answered (called `close_window`) and
/// no tab has unsaved changes: it is not answering (#153). A page that answered and was then
/// asked to wait, by a question the user has not answered, keeps the window.
fn close_if_unanswered(window: Window) {
    std::thread::spawn(move || {
        std::thread::sleep(CLOSE_ANSWER);
        if !CLOSE_ANSWERED.load(Ordering::SeqCst)
            && window
                .app_handle()
                .state::<Documents>()
                .unsaved_tabs()
                .is_empty()
            && !background::hide_while_batching(window.app_handle(), false)
        {
            // `destroy`, not `close`: `close` would ask again (see `on_window_event`).
            let _ = window.destroy();
        }
    });
}

/// Drag and drop onto the window: only the first file is opened.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    if let WindowEvent::CloseRequested { api, .. } = event {
        // The page asks what to do about unsaved changes (B2-02), then calls `close_window`. It
        // is asked even when no tab is known to have any: a value still being typed in a field is
        // a change only the page knows of (#153), and it sends it first. Without a page
        // listening, nobody could ask, and the window would never close.
        let events = app.state::<OpenEvents>();
        if events.has_receiver() {
            let unsaved = app.state::<Documents>().unsaved_tabs();
            let none_known = unsaved.is_empty();
            if none_known {
                // Before the page can answer.
                CLOSE_ANSWERED.store(false, Ordering::SeqCst);
            }
            api.prevent_close();
            events.send(OpenEvent::CloseRequested { tabs: unsaved });
            if none_known {
                close_if_unanswered(window.clone());
            }
        }
        return;
    }
    let WindowEvent::DragDrop(drag) = event else {
        return;
    };
    let events = app.state::<OpenEvents>();
    match drag {
        DragDropEvent::Enter { .. } => events.send(OpenEvent::DragHover { active: true }),
        DragDropEvent::Leave => events.send(OpenEvent::DragHover { active: false }),
        DragDropEvent::Drop { paths, .. } => {
            events.send(OpenEvent::DragHover { active: false });
            open_paths(app, paths.clone());
        }
        _ => {}
    }
}

/// Gives each of `paths` a tab and opens them off the main thread, each in a worker of its own
/// (MVP-14, ADR 0012). The outcomes arrive on the open-events channel.
pub fn open_paths(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let tabs = app
        .state::<Documents>()
        .add(&paths, &|event| report(app, event));
    for tab in tabs {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Documents>()
                .load(tab, &|event| report(&app, event));
            after_tabs_changed(&app);
        });
    }
}

/// Sends an open outcome to the frontend. A file that opened goes first on the recent files
/// list (#73), unless the user asked not to record it, or not to record any file (B2-12). A file
/// marked as sensitive (B3-05) is never on it, and the list of marked files gets its name as it
/// is now (it may have been renamed).
fn report(app: &AppHandle, event: OpenEvent) {
    let recording = app
        .try_state::<SettingsStore>()
        .is_some_and(|settings| settings.get().record_recent_files);
    if let OpenEvent::Opened { tab, info } = &event
        && let (Some(path), Some(recent)) = (
            app.state::<Documents>().tab_path(*tab),
            app.try_state::<RecentFiles>(),
        )
    {
        if info.sensitive {
            recent.forget(&path);
            if let (Some(identifier), Some(list)) = (
                app.state::<Documents>().tab_identifier(*tab),
                app.try_state::<SensitiveFiles>(),
            ) {
                list.rename(&identifier, &info.display_name);
            }
        } else if recording {
            recent.record(&path);
        }
    }
    app.state::<OpenEvents>().send(event);
}

/// Puts the file name of the tab the window shows in the window title (REL-03, MVP-14). The name
/// has no directory components (`DocumentInfo::display_name`).
fn show_active_in_title(app: &AppHandle) {
    let documents = app.state::<Documents>();
    let title = strings::window_title(
        documents.active_name().as_deref(),
        documents.active_unsaved(),
    );
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&title);
    }
}

/// After a tab opened, failed or closed: frees cached pages and queued renders of documents
/// that are no longer open, and keeps the window title in step.
fn after_tabs_changed(app: &AppHandle) {
    let open = app.state::<Documents>().open_documents();
    app.state::<Renderer>().retain_documents(&open);
    show_active_in_title(app);
}

/// Runs `work` on the blocking pool: worker requests can take seconds.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .unwrap_or_else(|_| {
            Err(IpcError {
                code: ErrorCode::Internal,
                message: "background task failed".to_owned(),
            })
        })
}
