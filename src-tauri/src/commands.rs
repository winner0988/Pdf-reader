//! Tauri commands for opening and closing documents (docs/architecture/ipc-contract.md).
//! Paths never cross into the WebView: the dialog runs here, and results arrive as
//! [`OpenEvent`]s carrying only a `TabId`, a `DocumentId` and a file name.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ipc_contract::types::{
    DocumentId, EditArgs, EncryptArgs, ErrorCode, ExportArgs, ExportEvent, ExportFormat,
    FileRecordingArgs, FormField, IpcError, LanguageImport, LinkArgs, LinkPreview, OcrArgs,
    OcrFocusArgs, OcrLanguages, OpenEvent, OutlineLinkArgs, OutlineResult, PageAnnotation,
    PageLink, PageText, PagesSource, Password, RecentFile, RecentId, RemoveLanguageArgs,
    RenderPageArgs, RequestId, SaveResult, SearchArgs, SearchEvent, Settings, StampImageInfo,
    TabId, UndoArgs, UnlockArgs, UnlockSourceArgs, UpdateCheck,
};
use ipc_contract::validate::{Validate, check_page_index};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, DragDropEvent, Manager, WebviewWindow, Window, WindowEvent};

use crate::documents::Documents;
use crate::events::OpenEvents;
use crate::export::{self, Exports, ImageKind};
use crate::file_dialog;
use crate::ocr::Ocr;
use crate::recent::RecentFiles;
use crate::render::Renderer;
use crate::search::{self, Searches};
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
/// only and is wiped afterwards; whether it opened the file arrives on the open channel.
#[tauri::command]
pub async fn unlock_tab(app: AppHandle, args: UnlockArgs) -> Result<(), IpcError> {
    args.validate().map_err(|error| IpcError {
        code: ErrorCode::InvalidArgument,
        message: error.to_string(),
    })?;
    let UnlockArgs { tab, password } = args;
    blocking(move || {
        let result = app
            .state::<Documents>()
            .unlock(tab, password, &|event| report(&app, event));
        after_tabs_changed(&app);
        result
    })
    .await
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

/// A password nobody knows, not even the user: 128 random bits as hex text. The permissions
/// password of a copy that only has an open password (the PDF standard has the field either way).
fn unguessable_password() -> Result<Password, IpcError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| IpcError {
        code: ErrorCode::Internal,
        message: "no random bytes for a password".to_owned(),
    })?;
    Ok(Password::new(
        bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    ))
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
/// if they chose to `discard` them.
#[tauri::command]
pub async fn close_window(
    app: AppHandle,
    window: WebviewWindow,
    discard: bool,
) -> Result<(), IpcError> {
    let documents = app.state::<Documents>();
    if !discard && !documents.unsaved_tabs().is_empty() {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "documents have unsaved changes".to_owned(),
        });
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
    Ok(())
}

/// Drag and drop onto the window: only the first file is opened.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    if let WindowEvent::CloseRequested { api, .. } = event {
        // Unsaved changes (B2-02): the page asks what to do, then calls `close_window`. Without a
        // page listening, nobody could ask, and the window would never close.
        let unsaved = app.state::<Documents>().unsaved_tabs();
        let events = app.state::<OpenEvents>();
        if !unsaved.is_empty() && events.has_receiver() {
            api.prevent_close();
            events.send(OpenEvent::CloseRequested { tabs: unsaved });
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
/// list (#73), unless the user asked not to record it, or not to record any file (B2-12).
fn report(app: &AppHandle, event: OpenEvent) {
    let recording = app
        .try_state::<SettingsStore>()
        .is_some_and(|settings| settings.get().record_recent_files);
    if let OpenEvent::Opened { tab, .. } = &event
        && recording
        && let (Some(path), Some(recent)) = (
            app.state::<Documents>().tab_path(*tab),
            app.try_state::<RecentFiles>(),
        )
    {
        recent.record(&path);
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
