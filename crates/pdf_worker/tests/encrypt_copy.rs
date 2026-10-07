//! A copy of a document encrypted with AES-256 (B2-15), through the real worker in the real
//! sandbox: the worker writes it through a write-only handle to a file the main process created,
//! with the passwords it was sent once; the document itself is left as it was.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, DocumentPermissions, ErrorCode, Password, Restrictions};
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

const NONE: Restrictions = Restrictions {
    print: false,
    copy: false,
    modify: false,
};

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
fn a_copy_opens_only_with_its_password_and_the_document_is_as_it_was() {
    let mut host = host();
    let doc = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (path, file) = new_file("encrypted");
    let response = host
        .encrypted_copy(
            doc,
            &file,
            Some(password("pw-open-7Qz")),
            password("pw-owner-7Qz"),
            NONE,
        )
        .expect("encrypted copy");
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
    // AES-256, and neither password is in the file.
    assert!(has(&written, b"/AESV3"));
    assert!(!has(&written, b"pw-open-7Qz") && !has(&written, b"pw-owner-7Qz"));

    // It needs a password; a wrong one is refused; either of its own opens it.
    for with in [None, Some("pw-other-7Qz")] {
        assert_eq!(
            opened_with(&path, with).unwrap_err().code(),
            ErrorCode::Encrypted
        );
    }
    for with in ["pw-open-7Qz", "pw-owner-7Qz"] {
        let (permissions, pages) = opened_with(&path, Some(with)).expect("opens");
        assert_eq!((permissions, pages), (DocumentPermissions::ALL, 10));
    }

    // The document, which the worker still has, saves as it was: not encrypted.
    let (plain, plain_file) = new_file("not-encrypted");
    host.save(doc, &plain_file).expect("save");
    drop(plain_file);
    let (_, pages) = opened_with(&plain, None).expect("a plain file opens without a password");
    assert_eq!(pages, 10);
    std::fs::remove_file(path).ok();
    std::fs::remove_file(plain).ok();
}

#[test]
fn restrictions_are_what_the_copy_is_opened_with_unless_the_permissions_password_did() {
    let mut host = host();
    let doc = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (path, file) = new_file("restricted");
    host.encrypted_copy(
        doc,
        &file,
        Some(password("pw-open-7Qz")),
        password("pw-owner-7Qz"),
        Restrictions {
            print: true,
            copy: true,
            modify: false,
        },
    )
    .expect("encrypted copy");
    drop(file);
    let (restricted, _) = opened_with(&path, Some("pw-open-7Qz")).expect("opens");
    assert!(!restricted.print && !restricted.print_high_quality && !restricted.copy);
    assert!(
        restricted.modify && restricted.assemble && restricted.annotate && restricted.fill_forms
    );
    let (lifted, _) = opened_with(&path, Some("pw-owner-7Qz")).expect("opens");
    assert_eq!(lifted, DocumentPermissions::ALL);
    std::fs::remove_file(path).ok();

    // With only restrictions, nobody is asked for a password, and the restrictions hold.
    let (path, file) = new_file("restricted-only");
    host.encrypted_copy(
        doc,
        &file,
        None,
        password("pw-owner-7Qz"),
        Restrictions {
            print: false,
            copy: false,
            modify: true,
        },
    )
    .expect("encrypted copy");
    drop(file);
    let (anyone, pages) = opened_with(&path, None).expect("opens without a password");
    assert_eq!(pages, 10);
    assert!(!anyone.modify && !anyone.assemble && !anyone.annotate && !anyone.fill_forms);
    assert!(anyone.print && anyone.copy);
    std::fs::remove_file(path).ok();
}

#[test]
fn a_signed_document_and_an_encrypted_one_have_no_encrypted_copy() {
    let mut host = host();
    let (_, file) = new_file("refused");
    let signed = open(&mut host, &corpus("benign/signed.pdf"));
    let error = host
        .encrypted_copy(
            signed,
            &file,
            Some(password("pw-open-7Qz")),
            password("o-7Qz"),
            NONE,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert!(
        matches!(&error, HostError::Worker(worker) if worker.code == WorkerErrorCode::InvalidRequest),
        "{error:?}"
    );

    let (locked, response) = host
        .open_with_password(&corpus("benign/encrypted-aes256.pdf"), password("user"))
        .expect("open");
    assert!(matches!(response, WorkerResponse::Opened { .. }));
    let error = host
        .encrypted_copy(
            locked,
            &file,
            Some(password("pw-open-7Qz")),
            password("o-7Qz"),
            NONE,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidArgument);

    // The worker goes on: an unsigned document is served by it.
    let plain = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (_, plain_file) = new_file("served");
    host.encrypted_copy(
        plain,
        &plain_file,
        Some(password("pw-open-7Qz")),
        password("o-7Qz"),
        NONE,
    )
    .expect("encrypted copy");
}

#[test]
fn a_password_a_copy_cannot_have_is_refused_and_the_worker_goes_on() {
    let mut host = host();
    let doc = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (_, file) = new_file("bad-password");
    for (open, owner) in [
        (Some("p".repeat(128)), "owner".to_owned()),
        (None, "p".repeat(128)),
        (Some(String::new()), "owner".to_owned()),
        (Some("a\0b".to_owned()), "owner".to_owned()),
    ] {
        let error = host
            .encrypted_copy(
                doc,
                &file,
                open.map(|text| password(&text)),
                password(&owner),
                NONE,
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
    // Not a crash: the next request is served by the same worker.
    assert!(host.is_running());
    let (_, file) = new_file("after");
    host.encrypted_copy(
        doc,
        &file,
        Some(password("pw-open-7Qz")),
        password("o-7Qz"),
        NONE,
    )
    .expect("encrypted copy");
}
