fn main() {
    // Declaring the app's commands makes each one require an explicit `allow-*` permission in
    // src-tauri/capabilities/ (least privilege, AGENTS.md principle 6).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "subscribe_open_events",
        "open_document_dialog",
        "retry_open",
        "close_tab",
        "set_active_tab",
        "render_page",
        "cancel",
        "get_outline",
        "get_page_links",
        "get_page_text",
        "describe_link",
        "open_link",
        "describe_outline_link",
        "open_outline_link",
        "search",
        "open_default_apps_settings",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
