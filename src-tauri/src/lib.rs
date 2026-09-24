//! Tauri main process: owns file access and coordinates the `pdf_worker` process (ADR 0008).

#[cfg(not(windows))]
compile_error!(
    "PDF Reader supports Windows only for now (ADR 0007): the worker sandbox is Windows-specific."
);

mod cli;
mod commands;
mod documents;
mod events;
mod links;
mod opener;
mod render;
mod search;
mod strings;

use tauri::Manager;

use crate::documents::Documents;
use crate::events::OpenEvents;
use crate::render::{DEFAULT_CACHE_BYTES, Renderer};
use crate::search::Searches;

pub fn run() {
    // The installer does not install WebView2 (REL-02: webviewInstallMode is "skip"). Without it
    // Tauri cannot create the window and the app would end without a word, so say what is missing.
    if tauri::webview_version().is_err() {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(strings::WEBVIEW2_MISSING_TITLE)
            .set_description(strings::WEBVIEW2_MISSING_MESSAGE)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    }

    let worker =
        worker_host::bundled_worker_path().unwrap_or_else(|_| worker_host::WORKER_FILE_NAME.into());
    #[cfg(debug_assertions)]
    if !worker.is_file() {
        eprintln!(
            "{} is missing next to the app; run `cargo build -p pdf_worker` before `pnpm tauri dev`",
            worker_host::WORKER_FILE_NAME
        );
    }
    let launch_document = cli::document_argument(std::env::args_os());

    tauri::Builder::default()
        .manage(Documents::new(worker))
        .manage(OpenEvents::default())
        .manage(Searches::default())
        .invoke_handler(tauri::generate_handler![
            commands::subscribe_open_events,
            commands::open_document_dialog,
            commands::retry_open,
            commands::close_document,
            commands::render_page,
            commands::cancel,
            commands::get_outline,
            commands::get_page_links,
            commands::describe_link,
            commands::open_link,
            commands::describe_outline_link,
            commands::open_outline_link,
            commands::search,
        ])
        .on_window_event(commands::on_window_event)
        .setup(move |app| {
            let handle = app.app_handle().clone();
            app.manage(Renderer::start(DEFAULT_CACHE_BYTES, move |args| {
                handle.state::<Documents>().render(args)
            }));
            if let Some(path) = launch_document {
                commands::open_in_background(app.app_handle().clone(), path, 0);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}
