//! Tauri main process: owns file access and coordinates the `pdf_worker` process (ADR 0008).

#[cfg(not(windows))]
compile_error!(
    "PDF Reader supports Windows only for now (ADR 0007): the worker sandbox is Windows-specific."
);

mod cli;
mod commands;
mod documents;
mod events;
mod export;
mod file_dialog;
mod history;
mod links;
mod local_data;
mod opener;
mod recent;
mod recovery;
mod render;
mod saving;
mod search;
mod settings;
mod strings;
mod update_check;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::documents::Documents;
use crate::events::OpenEvents;
use crate::export::Exports;
use crate::recent::RecentFiles;
use crate::recovery::Journals;
use crate::render::{DEFAULT_CACHE_BYTES, Renderer};
use crate::search::Searches;
use crate::settings::SettingsStore;

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
        .manage(Exports::default())
        .invoke_handler(tauri::generate_handler![
            commands::subscribe_open_events,
            commands::open_document_dialog,
            commands::retry_open,
            commands::unlock_tab,
            commands::close_tab,
            commands::set_active_tab,
            commands::render_page,
            commands::cancel,
            commands::get_outline,
            commands::get_page_links,
            commands::get_page_annotations,
            commands::get_page_fields,
            commands::get_page_text,
            commands::describe_link,
            commands::open_link,
            commands::describe_outline_link,
            commands::open_outline_link,
            commands::search,
            commands::open_default_apps_settings,
            commands::check_for_updates,
            commands::describe_releases_page,
            commands::open_releases_page,
            commands::get_recent_files,
            commands::open_recent_file,
            commands::remove_recent_file,
            commands::clear_recent_files,
            commands::get_file_recording,
            commands::set_file_recording,
            commands::get_settings,
            commands::set_settings,
            commands::clear_recent_exclusions,
            commands::export_pages,
            commands::apply_edit,
            commands::undo_edit,
            commands::redo_edit,
            commands::recover_edits,
            commands::discard_recovered_edits,
            commands::save_document,
            commands::save_document_as,
            commands::close_window,
            commands::privacy_export,
        ])
        .on_window_event(commands::on_window_event)
        .setup(move |app| {
            let data = data_dir(app.app_handle());
            app.manage(SettingsStore::load(
                data.as_ref().map(|dir| dir.join(settings::FILE_NAME)),
            ));
            app.manage(RecentFiles::new(
                data.as_ref().map(|dir| dir.join(recent::FILE_NAME)),
            ));
            app.state::<Documents>().set_journals(Journals::new(
                data.map(|dir| dir.join(recovery::FOLDER_NAME)),
            ));
            let reporter = app.app_handle().clone();
            app.state::<Documents>()
                .set_reporter(move |event| reporter.state::<OpenEvents>().send(event));
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

/// Where the app keeps its own data, such as the recent files list (#73), the settings (B2-12)
/// and the crash recovery journals (B2-13): the folder
/// `PDF_READER_DATA_DIR` names if it is an absolute path (the E2E tests give every run its own),
/// otherwise the app's local data folder, which does not roam with the Windows profile.
fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    std::env::var_os("PDF_READER_DATA_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| app.path().app_local_data_dir().ok())
}

/// Brings the window to the front, e.g. after a second launch handed it a file.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
