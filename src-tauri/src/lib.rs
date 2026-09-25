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

use std::ffi::OsString;
use std::path::Path;

use tauri::{AppHandle, Manager};

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
    let launch_documents = cli::document_arguments(std::env::args_os());

    tauri::Builder::default()
        // First, so that a second launch ends before anything else starts: it hands its files to
        // this window (each gets a tab) and exits (MVP-14, ADR 0012). Paths stay in the main
        // processes; a relative one is taken from the second launch's working directory.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let paths = cli::document_arguments(argv.into_iter().map(OsString::from))
                .into_iter()
                .map(|path| Path::new(&cwd).join(path))
                .collect();
            commands::open_paths(app, paths);
            show_window(app);
        }))
        .manage(Documents::new(worker))
        .manage(OpenEvents::default())
        .manage(Searches::default())
        .invoke_handler(tauri::generate_handler![
            commands::subscribe_open_events,
            commands::open_document_dialog,
            commands::retry_open,
            commands::close_tab,
            commands::set_active_tab,
            commands::render_page,
            commands::cancel,
            commands::get_outline,
            commands::get_page_links,
            commands::get_page_text,
            commands::describe_link,
            commands::open_link,
            commands::describe_outline_link,
            commands::open_outline_link,
            commands::search,
            commands::open_default_apps_settings,
        ])
        .on_window_event(commands::on_window_event)
        .setup(move |app| {
            let handle = app.app_handle().clone();
            app.manage(Renderer::start(DEFAULT_CACHE_BYTES, move |args| {
                handle.state::<Documents>().render(args)
            }));
            commands::open_paths(app.app_handle(), launch_documents);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}

/// Brings the window to the front, e.g. after a second launch handed it a file.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
