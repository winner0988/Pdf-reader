//! The sensitive mark (B3-05, ADR 0004, ADR 0010) through the real worker in the real sandbox:
//! the worker says when it opens a file whether the file is signed and has the app's document
//! identifier, and appends the identifier to the file it opened through a write-only handle,
//! refusing a signed file and an encrypted one without its permissions password.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ipc_contract::types::{DocumentId, ErrorCode, Password};
use ipc_contract::worker::{OpenedDocument, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

const MARK: [u8; 16] = [0x5e; 16];

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

fn opened(
    host: &mut WorkerHost,
    path: &Path,
    password: Option<&str>,
) -> (DocumentId, OpenedDocument) {
    let result = match password {
        Some(password) => host.open_with_password(path, Password::new(password.to_owned())),
        None => host.open(path),
    };
    match result.expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document),
        other => panic!("{other:?}"),
    }
}

/// A path in the temp directory no other test uses.
fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "pdf-worker-{}-sensitive-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// What the worker writes for `doc`: the file's bytes and where it is.
fn write(
    host: &mut WorkerHost,
    doc: DocumentId,
    identifier: Option<[u8; 16]>,
    password: Option<&str>,
) -> Result<(PathBuf, Vec<u8>), HostError> {
    let path = temp_path("marked.pdf");
    let file: File = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    let result = host.write_identifier(
        doc,
        &file,
        identifier,
        password.map(|password| Password::new(password.to_owned())),
    );
    drop(file);
    match result {
        Ok(WorkerResponse::Saved {
            bytes,
            incremental: true,
            ..
        }) => {
            let written = std::fs::read(&path).expect("written");
            assert_eq!(written.len() as u64, bytes);
            Ok((path, written))
        }
        Ok(other) => panic!("{other:?}"),
        Err(error) => {
            assert_eq!(std::fs::metadata(&path).expect("still there").len(), 0);
            std::fs::remove_file(&path).ok();
            Err(error)
        }
    }
}

#[test]
fn the_mark_is_appended_to_the_file_and_seen_when_it_opens_again() {
    let mut host = host();
    let original_path = corpus("benign/multi-page-10.pdf");
    let original = std::fs::read(&original_path).unwrap();
    let (doc, document) = opened(&mut host, &original_path, None);
    assert!(!document.signed);
    assert_eq!(document.identifier, None);

    let (marked_path, marked) = write(&mut host, doc, Some(MARK), None).expect("marked");
    assert_eq!(&marked[..original.len()], &original[..]);
    let (again, document) = opened(&mut host, &marked_path, None);
    assert_eq!(document.identifier, Some(MARK));
    assert_eq!(document.pages.len(), 10);

    // Taken out, from the marked file.
    let (unmarked_path, unmarked) = write(&mut host, again, None, None).expect("unmarked");
    assert_eq!(&unmarked[..marked.len()], &marked[..]);
    assert_eq!(opened(&mut host, &unmarked_path, None).1.identifier, None);
    for path in [marked_path, unmarked_path] {
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn a_signed_file_says_so_and_is_never_written_to() {
    let mut host = host();
    for name in ["benign/signed.pdf", "benign/signed-docmdp-p1.pdf"] {
        let (doc, document) = opened(&mut host, &corpus(name), None);
        assert!(document.signed, "{name}");
        let refused = write(&mut host, doc, Some(MARK), None).expect_err("refused");
        assert_eq!(refused.code(), ErrorCode::Signed, "{name}: {refused}");
        assert!(host.is_running());
    }
}

#[test]
fn an_encrypted_file_is_written_to_only_with_its_permissions_password() {
    let mut host = host();
    let (doc, document) = opened(
        &mut host,
        &corpus("benign/restricted-open-password.pdf"),
        Some("user"),
    );
    assert!(document.encrypted && !document.signed);
    for (password, code) in [
        (None, ErrorCode::Encrypted),
        (Some("wrong"), ErrorCode::Encrypted),
        (Some("user"), ErrorCode::NotAllowed),
    ] {
        let refused = write(&mut host, doc, Some(MARK), password).expect_err("refused");
        assert_eq!(refused.code(), code, "{password:?}: {refused}");
        assert!(host.is_running());
    }
    let (path, _) = write(&mut host, doc, Some(MARK), Some("owner")).expect("marked");
    assert_eq!(
        opened(&mut host, &path, Some("user")).1.identifier,
        Some(MARK)
    );
    std::fs::remove_file(path).ok();
}
