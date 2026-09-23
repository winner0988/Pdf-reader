//! Launches `sandbox_probe` inside the sandbox and checks each restriction actually holds.
#![cfg(windows)]
// One test observes process lifetime through OpenProcess/WaitForSingleObject.
#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sandbox::{SandboxConfig, Sandboxed, delete_app_container_profile};

fn probe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_sandbox_probe"))
}

/// Runs the probe with `args` and returns its trimmed stdout.
fn run(args: &[&str], config: &SandboxConfig, stdin: Option<&str>) -> (String, Option<u32>) {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let mut child = Sandboxed::spawn(probe(), &args, config).expect("spawn sandboxed probe");
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let code = child.wait_timeout(Duration::from_secs(20)).unwrap();
    (output.trim().to_owned(), code)
}

fn run_default(args: &[&str]) -> String {
    run(args, &SandboxConfig::default(), None).0
}

#[test]
fn pipes_carry_stdin_and_stdout() {
    let (output, code) = run(&["echo"], &SandboxConfig::default(), Some("hello\n"));
    assert_eq!(output, "echo:hello");
    assert_eq!(code, Some(0));
}

#[test]
fn runs_at_low_integrity() {
    assert_eq!(run_default(&["integrity"]), "integrity:4096");
}

#[test]
fn cannot_start_child_processes() {
    assert_eq!(run_default(&["spawn"]), "blocked");
}

#[test]
fn cannot_write_to_user_locations() {
    let dir = std::env::temp_dir().join(format!("sandbox-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Sanity check: the unsandboxed test process can write there.
    std::fs::write(dir.join("control.txt"), b"ok").unwrap();

    assert_eq!(run_default(&["write", dir.to_str().unwrap()]), "denied");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn win32k_is_unavailable() {
    assert_eq!(run_default(&["load-user32"]), "failed");
}

#[test]
fn environment_is_minimal() {
    // In the AppContainer, Windows adds LOCALAPPDATA, TEMP and TMP pointing into the
    // container's own folder.
    let output = run_default(&["env"]);
    let names: Vec<&str> = output.trim_start_matches("env:").split(',').collect();
    assert!(
        names
            .iter()
            .all(|name| ["SYSTEMROOT", "LOCALAPPDATA", "TEMP", "TMP"].contains(name)),
        "unexpected environment: {output}"
    );
    let temp = run_default(&["var", "TEMP"]).to_ascii_lowercase();
    assert!(
        temp.contains(r"\packages\pdfreader.worker\"),
        "TEMP is not the container's folder: {temp}"
    );
    let (output, _) = run(&["env"], &without_app_container(), None);
    assert_eq!(output, "env:SYSTEMROOT");
    assert!(
        std::env::vars_os().count() > 1,
        "the test process itself has more variables"
    );
}

#[test]
fn duplicated_file_handles_are_read_only() {
    let path = std::env::temp_dir().join(format!("sandbox-handle-{}.txt", std::process::id()));
    std::fs::write(&path, "twelve bytes").unwrap();
    // Opened read-write here; the duplicate must still be read-only.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();

    let output = read_handle_in_child(&file);
    assert_eq!(output, "read:12 write:denied");
    drop(file);
    std::fs::remove_file(&path).ok();
}

/// Starts a probe that waits for a handle value on stdin, duplicates `file` into it, and
/// reports what the probe could do with it.
fn read_handle_in_child(file: &std::fs::File) -> String {
    let mut child = Sandboxed::spawn(
        probe(),
        &[OsStr::new("read-handle-stdin")],
        &SandboxConfig::default(),
    )
    .unwrap();
    let value = child.duplicate_read_only(file).unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{value}\n").as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    child.wait_timeout(Duration::from_secs(20)).unwrap();
    output.trim().to_owned()
}

#[test]
fn memory_limit_is_enforced() {
    let config = SandboxConfig {
        memory_limit_bytes: 64 * 1024 * 1024,
        ..SandboxConfig::default()
    };
    let (output, _) = run(&["alloc", "256"], &config, None);
    assert_eq!(output, "alloc-failed");
    let (output, _) = run(&["alloc", "16"], &config, None);
    assert_eq!(output, "allocated:16");
}

/// The same probe without the AppContainer, to show the AppContainer tests are meaningful.
fn without_app_container() -> SandboxConfig {
    SandboxConfig {
        app_container: None,
        ..SandboxConfig::default()
    }
}

#[test]
fn network_is_blocked_even_to_localhost() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();

    assert_eq!(run_default(&["connect", &port]), "blocked");
    // Control: Low integrity alone does not stop networking; the AppContainer does.
    assert_eq!(
        run(&["connect", &port], &without_app_container(), None).0,
        "connected"
    );
}

#[test]
fn cannot_read_user_files() {
    let path = std::env::temp_dir().join(format!("sandbox-secret-{}.txt", std::process::id()));
    std::fs::write(&path, "private").unwrap();
    let target = path.to_str().unwrap();

    assert_eq!(run_default(&["read", target]), "denied");
    // Control: Low integrity alone can still read the user's files.
    assert_eq!(
        run(&["read", target], &without_app_container(), None).0,
        "read:7"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn system_fonts_are_readable() {
    // MuPDF will need them for non-embedded fonts (#31); they grant app packages read access.
    let font = Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"Fonts\arial.ttf");
    let size = std::fs::metadata(&font).expect("arial.ttf exists").len();
    assert_eq!(
        run_default(&["read", font.to_str().unwrap()]),
        format!("read:{size}")
    );
}

#[test]
fn app_container_profile_can_be_deleted() {
    const NAME: &str = "PdfReader.SandboxTest";
    let config = SandboxConfig {
        app_container: Some(NAME.to_owned()),
        ..SandboxConfig::default()
    };
    let folder = run(&["var", "LOCALAPPDATA"], &config, None).0;
    let folder = Path::new(&folder);
    assert!(
        folder
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("pdfreader.sandboxtest"),
        "{folder:?}"
    );
    assert!(folder.exists());

    delete_app_container_profile(NAME).unwrap();
    assert!(!folder.exists());
    // Deleting a profile that no longer exists is not an error.
    delete_app_container_profile(NAME).unwrap();
}

#[test]
fn kill_terminates_the_process() {
    let child =
        Sandboxed::spawn(probe(), &[OsStr::new("sleep")], &SandboxConfig::default()).unwrap();
    assert_eq!(
        child.wait_timeout(Duration::from_millis(200)).unwrap(),
        None
    );
    child.kill().unwrap();
    assert!(
        child
            .wait_timeout(Duration::from_secs(5))
            .unwrap()
            .is_some()
    );
}

#[test]
fn dropping_the_handle_kills_the_process() {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    let child =
        Sandboxed::spawn(probe(), &[OsStr::new("sleep")], &SandboxConfig::default()).unwrap();
    // SAFETY: plain Win32 calls on a process id we own; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, child.id()) };
    assert!(!handle.is_null());
    drop(child);
    // SAFETY: `handle` is valid until CloseHandle.
    let waited = unsafe { WaitForSingleObject(handle, 5_000) };
    unsafe { CloseHandle(handle) };
    assert_eq!(waited, WAIT_OBJECT_0, "the process must die with its job");
}
