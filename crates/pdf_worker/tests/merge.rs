//! The pages of another file going into a document, through the real worker in the real sandbox
//! (B2-06): the worker reads the file through a read-only handle (never a path), gives back a
//! plain copy of it, and later takes the pages from that copy.
#![cfg(windows)]

use std::fs::File;
use std::path::{Path, PathBuf};

use ipc_contract::limits::MAX_SOURCE_BYTES;
use ipc_contract::types::{DocumentId, ErrorCode, PageSize, Password};
use ipc_contract::worker::{WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse};
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

fn opened(host: &mut WorkerHost, path: &Path) -> (DocumentId, Vec<PageSize>) {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    }
}

/// What the worker makes of the corpus file `name`: its plain copy, its page count and what it
/// has that is active.
fn source(
    host: &mut WorkerHost,
    name: &str,
    password: Option<&str>,
) -> Result<(Vec<u8>, u32, usize), HostError> {
    let file = File::open(corpus(name)).expect("the sample");
    match host.prepare_source(&file, password.map(|text| Password::new(text.to_owned())))? {
        WorkerResponse::Source {
            bytes,
            pages,
            security,
            ..
        } => Ok((bytes, pages, security.findings.len())),
        other => panic!("{other:?}"),
    }
}

fn code(error: &HostError) -> ErrorCode {
    error.code()
}

#[test]
fn the_pages_of_an_encrypted_file_go_into_a_document_by_a_plain_copy() {
    let mut host = host();
    // The file needs its password, and says so; a wrong one is no better.
    assert_eq!(
        code(&source(&mut host, "benign/encrypted-aes256.pdf", None).unwrap_err()),
        ErrorCode::Encrypted
    );
    assert_eq!(
        code(&source(&mut host, "benign/encrypted-aes256.pdf", Some("wrong")).unwrap_err()),
        ErrorCode::Encrypted
    );
    let (bytes, pages, findings) =
        source(&mut host, "benign/encrypted-aes256.pdf", Some("user")).expect("source");
    assert_eq!((pages, findings), (1, 0));
    // Plain: nothing in it says it is encrypted, so it needs no password later.
    assert!(!bytes.windows(8).any(|window| window == b"/Encrypt"));

    let (doc, before) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert_eq!(before.len(), 10);
    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertPages {
                at: 2,
                source: bytes.clone(),
            },
        })
        .expect("insert");
    let WorkerResponse::Edited { pages, .. } = edited else {
        panic!("{edited:?}");
    };
    assert_eq!(pages.len(), 11);
    // The same edit, made again by undo (Revert opens the document again from its bytes), and
    // the same source: the pages are there again.
    let reverted = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: vec![WorkerEdit::InsertPages {
                at: 2,
                source: bytes,
            }],
            password: None,
        })
        .expect("revert");
    assert!(matches!(
        reverted,
        WorkerResponse::Edited { pages, .. } if pages.len() == 11
    ));
}

#[test]
fn a_file_that_may_not_be_taken_from_a_file_that_is_no_pdf_and_one_too_large_are_refused() {
    let mut host = host();
    // The author forbids copying: the pages cannot be taken out.
    let refused = source(&mut host, "benign/restricted-no-copy-no-print.pdf", None).unwrap_err();
    assert_eq!(code(&refused), ErrorCode::NotAllowed);
    assert!(matches!(
        refused,
        HostError::Worker(error) if error.code == WorkerErrorCode::NotAllowed
    ));
    // The owner password lifts that.
    assert!(
        source(
            &mut host,
            "benign/restricted-open-password.pdf",
            Some("owner")
        )
        .is_ok()
    );

    // Not a PDF.
    let error = source(&mut host, "manifest.json", None);
    assert!(error.is_err());

    // Larger than a source may be: refused before the worker reads a byte of it.
    let path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-large-source.pdf",
        std::process::id()
    ));
    let large = File::create(&path).expect("create");
    large.set_len(MAX_SOURCE_BYTES as u64 + 1).expect("sparse");
    drop(large);
    let file = File::open(&path).expect("open");
    let error = host.prepare_source(&file, None).unwrap_err();
    std::fs::remove_file(&path).ok();
    assert!(matches!(error, HostError::TooLarge));

    // The worker goes on after any of it.
    let (_, pages, _) = source(&mut host, "benign/mixed-page-sizes.pdf", None).expect("source");
    assert_eq!(pages, 4);
}

#[test]
fn what_is_active_in_a_file_is_reported_and_stays_out_of_the_document() {
    let mut host = host();
    let (doc, _) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (bytes, pages, findings) =
        source(&mut host, "malicious/openaction-js.pdf", None).expect("source");
    // The scan sees it (so that the banner can say so) ...
    assert!(findings > 0);
    // ... and the pages come without it.
    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertPages {
                at: 10,
                source: bytes,
            },
        })
        .expect("insert");
    assert!(matches!(
        edited,
        WorkerResponse::Edited { pages: ref all, .. } if all.len() as u32 == 10 + pages
    ));
    // Saved, and opened again: what it says of active content is nothing.
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-merged.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    host.save(doc, &file).expect("save");
    drop(file);
    let (_, resulting) = match host.open(&path) {
        Ok((doc, WorkerResponse::Opened { document, .. })) => (doc, document),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&path).ok();
    assert_eq!(resulting.pages.len() as u32, 10 + pages);
    assert!(resulting.security.findings.is_empty());
}
