//! qpdf validation helpers (Issue #438, B5-01).
//!
//! Validates generated PDFs using qpdf --check and qpdf --show-encryption.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locates the qpdf executable: checks $env:QPDF first, then PATH.
/// If not found, returns an error message with installation instructions.
#[allow(dead_code)]
pub fn find_qpdf() -> Result<PathBuf, String> {
    find_qpdf_from(std::env::var_os("QPDF"), std::env::var_os("PATH"))
}

#[allow(dead_code)]
pub fn find_qpdf_from(
    qpdf_env: Option<std::ffi::OsString>,
    path_env: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    if let Some(qpdf_val) = qpdf_env {
        let trimmed = qpdf_val.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return Ok(path);
            }
        }
    }

    if let Some(paths) = path_env {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(if cfg!(windows) { "qpdf.exe" } else { "qpdf" });
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(
        "qpdf 12.4.2 is required for tests but was not found.\n\
        To install qpdf:\n\
        - Download: https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2-msvc64.zip\n\
        - SHA-256: db87077e683630c1217e0e8f9a20a9749d952ab676e881c3689187763a5de25d\n\
        - Unpack and set the QPDF environment variable to the path of qpdf.exe, or add its containing folder to PATH."
            .to_string(),
    )
}

fn require_qpdf() -> PathBuf {
    find_qpdf().unwrap_or_else(|err| panic!("{err}"))
}

/// Runs `qpdf --check <path>`. Exit code 0 is required.
/// Exit codes 2 (errors) and 3 (warnings) are considered failures.
#[allow(dead_code)]
pub fn check_qpdf(path: &Path) {
    let qpdf = require_qpdf();
    let output = Command::new(&qpdf)
        .arg("--check")
        .arg(path)
        .output()
        .unwrap_or_else(|err| panic!("failed to execute qpdf at {}: {err}", qpdf.display()));

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if code != 0 {
        panic!(
            "qpdf --check failed on {} with exit code {code}:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            path.display()
        );
    }
}

pub struct TempFileGuard(pub PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes `bytes` to a temporary file and checks it with `qpdf --check`.
#[allow(dead_code)]
pub fn check_qpdf_bytes(bytes: &[u8]) {
    let path = super::temp_pdf("qpdf-check", bytes);
    let _guard = TempFileGuard(path.clone());
    check_qpdf(&path);
}

/// Parsed encryption details from `qpdf --show-encryption`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QpdfEncryption {
    pub r: u32,
    pub p: i64,
    pub print_low: bool,
    pub print_high: bool,
    pub extract_accessibility: bool,
    pub extract_any: bool,
    pub modify_assembly: bool,
    pub modify_forms: bool,
    pub modify_annotations: bool,
    pub modify_other: bool,
    pub modify_anything: bool,
    pub raw: String,
}

#[allow(dead_code)]
pub fn parse_qpdf_encryption(output: &str) -> QpdfEncryption {
    let mut r = 0;
    let mut p = 0;
    let mut print_low = false;
    let mut print_high = false;
    let mut extract_accessibility = false;
    let mut extract_any = false;
    let mut modify_assembly = false;
    let mut modify_forms = false;
    let mut modify_annotations = false;
    let mut modify_other = false;
    let mut modify_anything = false;

    for line in output.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("R = ") {
            r = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("P = ") {
            p = val.trim().parse().unwrap_or(0);
        } else if line.starts_with("print low resolution:") {
            print_low = line.ends_with(": allowed");
        } else if line.starts_with("print high resolution:") {
            print_high = line.ends_with(": allowed");
        } else if line.starts_with("extract for accessibility:") {
            extract_accessibility = line.ends_with(": allowed");
        } else if line.starts_with("extract for any purpose:") {
            extract_any = line.ends_with(": allowed");
        } else if line.starts_with("modify document assembly:") {
            modify_assembly = line.ends_with(": allowed");
        } else if line.starts_with("modify forms:") {
            modify_forms = line.ends_with(": allowed");
        } else if line.starts_with("modify annotations:") {
            modify_annotations = line.ends_with(": allowed");
        } else if line.starts_with("modify other:") {
            modify_other = line.ends_with(": allowed");
        } else if line.starts_with("modify anything:") {
            modify_anything = line.ends_with(": allowed");
        }
    }

    QpdfEncryption {
        r,
        p,
        print_low,
        print_high,
        extract_accessibility,
        extract_any,
        modify_assembly,
        modify_forms,
        modify_annotations,
        modify_other,
        modify_anything,
        raw: output.to_string(),
    }
}

/// Checks an encrypted PDF with `qpdf --check --password=<pw>` and
/// `qpdf --show-encryption --password=<pw>`. Asserts R = 6 (AES-256) and returns parsed details.
#[allow(dead_code)]
pub fn check_qpdf_encrypted(path: &Path, password: Option<&str>) -> QpdfEncryption {
    let qpdf = require_qpdf();

    // 1. qpdf --check [--password=<pw>] <path>
    let mut cmd = Command::new(&qpdf);
    cmd.arg("--check");
    if let Some(pw) = password {
        cmd.arg(format!("--password={pw}"));
    }
    cmd.arg(path);

    let output = cmd
        .output()
        .unwrap_or_else(|err| panic!("failed to execute qpdf at {}: {err}", qpdf.display()));

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if code != 0 {
        panic!(
            "qpdf --check (encrypted) failed on {} with exit code {code}:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            path.display()
        );
    }

    // 2. qpdf --show-encryption [--password=<pw>] <path>
    let mut show_cmd = Command::new(&qpdf);
    show_cmd.arg("--show-encryption");
    if let Some(pw) = password {
        show_cmd.arg(format!("--password={pw}"));
    }
    show_cmd.arg(path);

    let show_output = show_cmd
        .output()
        .unwrap_or_else(|err| panic!("failed to execute qpdf at {}: {err}", qpdf.display()));

    let show_code = show_output.status.code().unwrap_or(-1);
    let show_stdout = String::from_utf8_lossy(&show_output.stdout);
    let show_stderr = String::from_utf8_lossy(&show_output.stderr);

    if show_code != 0 {
        panic!(
            "qpdf --show-encryption failed on {} with exit code {show_code}:\n--- stdout ---\n{show_stdout}\n--- stderr ---\n{show_stderr}",
            path.display()
        );
    }

    let parsed = parse_qpdf_encryption(&show_stdout);
    assert_eq!(
        parsed.r, 6,
        "expected AES-256 (R = 6), got R = {}. Raw output:\n{show_stdout}",
        parsed.r
    );

    parsed
}

/// Writes `bytes` to a temporary file and checks it with `check_qpdf_encrypted`.
#[allow(dead_code)]
pub fn check_qpdf_encrypted_bytes(bytes: &[u8], password: Option<&str>) -> QpdfEncryption {
    let path = super::temp_pdf("qpdf-enc-check", bytes);
    let _guard = TempFileGuard(path.clone());
    check_qpdf_encrypted(&path, password)
}
