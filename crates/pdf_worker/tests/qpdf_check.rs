//! Tests for the qpdf test helper itself (Issue #438, B5-01).

mod common;

use std::path::Path;

#[test]
fn helper_fails_when_qpdf_not_found() {
    let err = common::find_qpdf_from(None, None).unwrap_err();
    assert!(err.contains("qpdf 12.4.2 is required"));
    assert!(
        err.contains(
            "https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2-msvc64.zip"
        )
    );
    assert!(err.contains("db87077e683630c1217e0e8f9a20a9749d952ab676e881c3689187763a5de25d"));
}

#[test]
fn helper_fails_when_qpdf_env_set_but_file_does_not_exist() {
    let dummy = std::ffi::OsString::from("C:/this/path/does/not/exist/qpdf.exe");
    let path_val = std::env::var_os("PATH");
    let err = common::find_qpdf_from(Some(dummy), path_val).unwrap_err();
    assert!(err.contains(
        "QPDF environment variable is set to 'C:/this/path/does/not/exist/qpdf.exe', but the file does not exist."
    ));
    assert!(
        err.contains(
            "https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2-msvc64.zip"
        )
    );
}

#[test]
fn helper_checks_valid_pdf_and_detects_corrupted_truncated_pdf() {
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/benign/single-page.pdf");
    let valid = std::fs::read(&sample).expect("read sample");
    common::check_qpdf(&sample);
    common::check_qpdf_bytes(&valid);

    // Truncate the valid file to deliberately damage it
    let truncated = &valid[..valid.len().saturating_sub(60)];
    let path = common::temp_pdf("corrupt-test", truncated);
    let result = std::panic::catch_unwind(|| {
        common::check_qpdf(&path);
    });
    std::fs::remove_file(&path).ok();
    assert!(
        result.is_err(),
        "qpdf --check must fail on damaged/truncated PDF"
    );
}

#[test]
fn helper_checks_encrypted_pdf_and_encryption_details() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/benign/encrypted-aes256.pdf");
    let enc = common::check_qpdf_encrypted(&sample, Some("user"));
    assert_eq!(enc.r, 6);
    assert!(enc.print_high);
    assert!(enc.extract_any);
}
