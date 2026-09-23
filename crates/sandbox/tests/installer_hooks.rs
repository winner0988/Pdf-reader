//! The uninstaller deletes the worker's AppContainer profile by name (REL-01); keep it in sync.
#![cfg(windows)]

#[test]
fn uninstaller_deletes_the_worker_profile() {
    let hooks = include_str!("../../../src-tauri/windows/installer-hooks.nsh");
    let call = format!(
        "DeleteAppContainerProfile(w \"{}\")",
        sandbox::WORKER_APP_CONTAINER
    );
    assert!(
        hooks.contains(&call),
        "installer-hooks.nsh must contain {call}"
    );
}
