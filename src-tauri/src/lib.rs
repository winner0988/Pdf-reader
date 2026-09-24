//! Tauri main process: owns file access and coordinates the `pdf_worker` process (ADR 0008).

#[cfg(not(windows))]
compile_error!(
    "PDF Reader supports Windows only for now (ADR 0007): the worker sandbox is Windows-specific."
);

mod cli;
mod commands;
mod documents;
mod events;
mod strings;

use tauri::Manager;

use crate::documents::Documents;
use crate::events::OpenEvents;

pub fn run() {
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
        .invoke_handler(tauri::generate_handler![
            commands::subscribe_open_events,
            commands::open_document_dialog,
            commands::retry_open,
            commands::close_document,
        ])
        .on_window_event(commands::on_window_event)
        .setup(move |app| {
            if let Some(path) = launch_document {
                commands::open_in_background(app.app_handle().clone(), path, 0);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}
