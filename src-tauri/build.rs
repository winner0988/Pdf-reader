fn main() {
    // Declaring the app's commands makes each one require an explicit `allow-*` permission in
    // src-tauri/capabilities/ (least privilege, AGENTS.md principle 6).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "subscribe_open_events",
        "open_document_dialog",
        "retry_open",
        "close_document",
        "render_page",
        "cancel",
        "get_outline",
        "get_page_links",
        "describe_link",
        "open_link",
        "describe_outline_link",
        "open_outline_link",
        "search",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
