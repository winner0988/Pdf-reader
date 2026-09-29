//! Tauri commands for opening and closing documents (docs/architecture/ipc-contract.md).
//! Paths never cross into the WebView: the dialog runs here, and results arrive as
//! [`OpenEvent`]s carrying only a `TabId`, a `DocumentId` and a file name.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use ipc_contract::types::{
    DocumentId, ErrorCode, ExportArgs, ExportEvent, ExportFormat, FileRecordingArgs, IpcError,
    LinkArgs, LinkPreview, OpenEvent, OutlineLinkArgs, OutlineResult, PageLink, PageText,
    RecentFile, RecentId, RenderPageArgs, RequestId, SearchArgs, SearchEvent, TabId, UnlockArgs,
};
use ipc_contract::validate::Validate;
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, DragDropEvent, Manager, WebviewWindow, Window, WindowEvent};

use crate::documents::Documents;
use crate::events::OpenEvents;
use crate::export::{self, Exports};
use crate::file_dialog;
use crate::recent::RecentFiles;
use crate::render::Renderer;
use crate::search::{self, Searches};
use crate::strings;

/// Registers the frontend's channel for [`OpenEvent`]s.
#[tauri::command]
pub async fn subscribe_open_events(
    app: AppHandle,
    on_event: Channel<OpenEvent>,
) -> Result<(), IpcError> {
    blocking(move || {
        let documents = app.state::<Documents>();
        app.state::<OpenEvents>()
            .subscribe(on_event, || documents.snapshot());
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
        app.state::<RecentFiles>()
            .set_recorded(&path, args.record)
            .map_err(|_| IpcError {
                code: ErrorCode::Internal,
                message: "no randomness for the list of files not to record".to_owned(),
            })
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
        ExportFormat::Png { dpi } => {
            let Some(folder) = file_dialog::pick_folder(&window).await? else {
                return Ok(false);
            };
            let targets = export::png_targets(&folder, &stem, &pages);
            let existing = targets.iter().filter(|target| target.exists()).count();
            if existing > 0 && !confirm_overwrite(&window, existing).await {
                return Ok(false);
            }
            blocking(move || {
                export::write_pngs(&app, request, doc, &pages, dpi, &targets, &on_event)
            })
            .await?;
        }
    }
    Ok(true)
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
    let WindowEvent::DragDrop(drag) = event else {
        return;
    };
    let app = window.app_handle();
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
/// list (#73), unless the user asked not to record it.
fn report(app: &AppHandle, event: OpenEvent) {
    if let OpenEvent::Opened { tab, .. } = &event
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
    let name = app.state::<Documents>().active_name();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&strings::window_title(name.as_deref()));
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
