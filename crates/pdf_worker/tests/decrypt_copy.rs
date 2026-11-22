//! A copy of an encrypted document without its password or restrictions (B6-01), through the
//! real worker in the real sandbox: the worker writes it through a write-only handle to a file the
//! main process created, with the password it was sent once; the open document is left as it was.
#![cfg(windows)]

mod common;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use ipc_contract::types::{DocumentId, DocumentPermissions, ErrorCode, Password};
use ipc_contract::worker::{WorkerErrorCode, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn open(host: &mut WorkerHost, path: &Path) -> DocumentId {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    }
}

fn open_with_password(host: &mut WorkerHost, path: &Path, pw: &str) -> DocumentId {
    match host.open_with_password(path, password(pw)).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    }
}

/// A new, empty file to write into, as the main process creates it.
fn new_file(name: &str) -> (PathBuf, File) {
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-{name}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    (path, file)
}

fn password(text: &str) -> Password {
    Password::new(text.to_owned())
}

/// The permissions a copy is opened with and its page count, by the password it is opened with.
fn opened_with(path: &Path, with: Option<&str>) -> Result<(DocumentPermissions, usize), HostError> {
    let mut host = host();
    let (_, response) = match with {
        Some(text) => host.open_with_password(path, password(text))?,
        None => host.open(path)?,
    };
    match response {
        WorkerResponse::Opened { document, .. } => Ok((document.permissions, document.pages.len())),
        other => panic!("{other:?}"),
    }
}

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn a_copy_opens_without_password_and_the_document_is_as_it_was() {
    let mut host = host();
    let doc = open_with_password(&mut host, &corpus("benign/encrypted-aes256.pdf"), "user");
    let (path, file) = new_file("decrypted");
    let response = host
        .decrypted_copy(doc, &file, password("user"))
        .expect("decrypted copy");
    let WorkerResponse::Saved {
        bytes, incremental, ..
    } = response
    else {
        panic!("{response:?}");
    };
    drop(file);
    let written = std::fs::read(&path).expect("read");
    assert_eq!(written.len() as u64, bytes);
    assert!(!incremental);
    assert!(!has(&written, b"/Encrypt"));

    // Opens with no password at all, has full permissions and 1 page.
    let (permissions, pages) = opened_with(&path, None).expect("opens without password");
    assert_eq!(permissions, DocumentPermissions::ALL);
    assert_eq!(pages, 1);

    // It also works with the owner password.
    let (owner_path, owner_file) = new_file("decrypted-owner");
    host.decrypted_copy(doc, &owner_file, password("owner"))
        .expect("owner decrypted copy");
    drop(owner_file);
    let (permissions, pages) = opened_with(&owner_path, None).expect("opens without password");
    assert_eq!(permissions, DocumentPermissions::ALL);
    assert_eq!(pages, 1);

    // The document, which the worker still has, saves as it was: encrypted.
    let (saved_path, saved_file) = new_file("saved-still-encrypted");
    host.save(doc, &saved_file).expect("save");
    drop(saved_file);
    assert_eq!(
        opened_with(&saved_path, None).unwrap_err().code(),
        ErrorCode::Encrypted
    );
    assert!(opened_with(&saved_path, Some("user")).is_ok());

    std::fs::remove_file(path).ok();
    std::fs::remove_file(owner_path).ok();
    std::fs::remove_file(saved_path).ok();
}

#[test]
fn a_restricted_document_requires_owner_password() {
    let mut host = host();
    let doc = open_with_password(
        &mut host,
        &corpus("benign/restricted-open-password.pdf"),
        "user",
    );
    let (path, file) = new_file("restricted-copy");

    // User password fails with NotAllowed
    let err = host
        .decrypted_copy(doc, &file, password("user"))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::NotAllowed);
    drop(file);

    // Wrong password fails with Encrypted (WrongPassword)
    let (_, file) = new_file("restricted-copy-2");
    let err = host
        .decrypted_copy(doc, &file, password("wrong"))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Encrypted);
    drop(file);

    // Owner password succeeds and lifts all restrictions in the copy
    let (_, file) = new_file("restricted-copy-3");
    host.decrypted_copy(doc, &file, password("owner"))
        .expect("owner decrypts");
    drop(file);
    let path3 = std::env::temp_dir().join(format!(
        "pdf-worker-{}-restricted-copy-3.pdf",
        std::process::id()
    ));
    let (permissions, pages) = opened_with(&path3, None).expect("opens");
    assert_eq!(permissions, DocumentPermissions::ALL);
    assert_eq!(pages, 1);
    std::fs::remove_file(path).ok();
    std::fs::remove_file(path3).ok();

    // benign/restricted-no-copy-no-print.pdf (no open password, restricted)
    let doc_nopw = open(&mut host, &corpus("benign/restricted-no-copy-no-print.pdf"));
    let (nopw_path, nopw_file) = new_file("restricted-nopw");
    let err = host
        .decrypted_copy(doc_nopw, &nopw_file, password("user"))
        .unwrap_err();
    assert!(matches!(
        err.code(),
        ErrorCode::Encrypted | ErrorCode::NotAllowed
    ));
    drop(nopw_file);

    let (_, nopw_file2) = new_file("restricted-nopw-2");
    host.decrypted_copy(doc_nopw, &nopw_file2, password("owner"))
        .expect("owner decrypts");
    drop(nopw_file2);
    let nopw_path2 = std::env::temp_dir().join(format!(
        "pdf-worker-{}-restricted-nopw-2.pdf",
        std::process::id()
    ));
    let (permissions, _) = opened_with(&nopw_path2, None).expect("opens");
    assert_eq!(permissions, DocumentPermissions::ALL);
    std::fs::remove_file(nopw_path).ok();
    std::fs::remove_file(nopw_path2).ok();
}

#[test]
fn a_signed_encrypted_document_and_an_unencrypted_one_have_no_decrypted_copy() {
    let mut host = host();
    let (_, file) = new_file("unencrypted-refused");
    let plain = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let err = host
        .decrypted_copy(plain, &file, password("owner"))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    assert!(
        matches!(&err, HostError::Worker(worker) if worker.code == WorkerErrorCode::InvalidRequest),
        "{err:?}"
    );
    drop(file);

    // Encrypt benign/signed.pdf into a temporary file using qpdf
    let qpdf_bin = common::find_qpdf().expect("qpdf binary");
    let temp_signed = std::env::temp_dir().join(format!(
        "pdf-worker-{}-temp-signed-enc.pdf",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&temp_signed);
    let status = Command::new(&qpdf_bin)
        .arg(corpus("benign/signed.pdf"))
        .args(["--encrypt", "user", "owner", "256", "--"])
        .arg(&temp_signed)
        .status()
        .expect("qpdf encrypt signed pdf");
    assert!(status.success(), "qpdf failed to encrypt signed pdf");

    let signed_doc = open_with_password(&mut host, &temp_signed, "user");
    let (_, file) = new_file("signed-refused");
    let err = host
        .decrypted_copy(signed_doc, &file, password("owner"))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    assert!(
        matches!(&err, HostError::Worker(worker) if worker.code == WorkerErrorCode::InvalidRequest),
        "{err:?}"
    );
    drop(file);
    std::fs::remove_file(temp_signed).ok();
}

#[test]
fn wrong_password_leaves_worker_running() {
    let mut host = host();
    let doc = open_with_password(&mut host, &corpus("benign/encrypted-aes256.pdf"), "user");
    let (_, file) = new_file("wrong-pw");
    let err = host
        .decrypted_copy(doc, &file, password("wrong"))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Encrypted);
    drop(file);

    // Not a crash: worker is still running and serves subsequent requests.
    assert!(host.is_running());
    let (path, file) = new_file("after-wrong-pw");
    host.decrypted_copy(doc, &file, password("user"))
        .expect("decrypted copy after failure");
    drop(file);
    std::fs::remove_file(path).ok();
}
