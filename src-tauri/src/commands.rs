//! Tauri commands for opening and closing documents (docs/architecture/ipc-contract.md).
//! Paths never cross into the WebView: the dialog runs here, and results arrive as
//! [`OpenEvent`]s carrying only a `DocumentId` and a file name.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use ipc_contract::types::{
    DocumentId, ErrorCode, IpcError, OpenEvent, OutlineResult, PageLink, RenderPageArgs, RequestId,
    SearchArgs, SearchEvent,
};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, DragDropEvent, Manager, WebviewWindow, Window, WindowEvent};

use crate::documents::Documents;
use crate::events::OpenEvents;
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
        let current = app.state::<Documents>().current();
        app.state::<OpenEvents>().subscribe(on_event, current);
        Ok(())
    })
    .await
}

/// Set while the open dialog is showing.
static DIALOG_SHOWING: AtomicBool = AtomicBool::new(false);

/// Shows the native open dialog (PDF files only). Returns false if the user cancelled, or if a
/// dialog is already showing; otherwise the outcome arrives on the open-events channel.
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

    let picked = rfd::AsyncFileDialog::new()
        .set_title(strings::OPEN_DIALOG_TITLE)
        .add_filter(strings::PDF_FILTER_NAME, &["pdf"])
        .set_parent(&window)
        .pick_file()
        .await;
    let Some(file) = picked else {
        return Ok(false);
    };
    open_in_background(app, file.path().to_owned(), 0);
    Ok(true)
}

/// Opens the most recently attempted file again (after `workerCrashed`, `workerTimeout` or
/// `unreadable`). The outcome arrives on the open-events channel.
#[tauri::command]
pub async fn retry_open(app: AppHandle) -> Result<(), IpcError> {
    blocking(move || {
        let events = app.state::<OpenEvents>();
        app.state::<Documents>().retry(&|event| events.send(event));
        forget_other_documents(&app);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn close_document(app: AppHandle, doc: DocumentId) -> Result<(), IpcError> {
    blocking(move || {
        let result = app.state::<Documents>().close(doc);
        forget_other_documents(&app);
        result
    })
    .await
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

/// Searches the document (MVP-10); hits, progress and a final `done` arrive on `on_event`.
#[tauri::command]
pub async fn search(
    app: AppHandle,
    args: SearchArgs,
    on_event: Channel<SearchEvent>,
) -> Result<(), IpcError> {
    blocking(move || search::run(&app, args, on_event)).await
}

/// Cancels a queued `render_page` request (it then fails with `cancelled`) or a running search.
#[tauri::command]
pub async fn cancel(app: AppHandle, request: RequestId) -> Result<(), IpcError> {
    app.state::<Renderer>().cancel(request);
    app.state::<Searches>().cancel(request);
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
            if let Some(first) = paths.first() {
                let ignored = u32::try_from(paths.len() - 1).unwrap_or(u32::MAX);
                open_in_background(app.clone(), first.clone(), ignored);
            }
        }
        _ => {}
    }
}

/// Opens `path` off the main thread and reports on the open-events channel.
pub fn open_in_background(app: AppHandle, path: PathBuf, ignored_files: u32) {
    tauri::async_runtime::spawn_blocking(move || {
        let events = app.state::<OpenEvents>();
        app.state::<Documents>()
            .open(&path, ignored_files, &|event| events.send(event));
        forget_other_documents(&app);
    });
}

/// Frees cached pages and queued renders of documents that are no longer open.
fn forget_other_documents(app: &AppHandle) {
    let current = app.state::<Documents>().current().map(|info| info.doc);
    app.state::<Renderer>().retain_document(current);
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
