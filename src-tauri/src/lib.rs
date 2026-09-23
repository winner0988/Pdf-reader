//! Tauri main process: owns file access and coordinates the `pdf_worker` process (ADR 0008).

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}
