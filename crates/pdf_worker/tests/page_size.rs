//! Page size validation when opening and editing documents (Issue #541).
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use ipc_contract::worker::{WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

#[test]
fn opening_oversized_page_fails_with_worker_error_and_worker_stays_running() {
    let mut host = host();
    let res = host.open(&corpus("malformed/huge-page-size.pdf"));
    match res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::LimitExceeded);
            assert_eq!(error.detail, "page too large");
        }
        Err(ref other) => {
            panic!("expected HostError::Worker(LimitExceeded), got: {other:?}");
        }
        Ok((_, ref response)) => {
            panic!("expected worker error, got success: {response:?}");
        }
    }
    assert!(
        host.is_running(),
        "the worker must not be terminated by an oversized page"
    );

    // The same worker can open a benign document afterwards.
    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn inserting_oversized_page_via_edit_is_refused_with_worker_error() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open initial benign document");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));

    let huge_bytes =
        fs::read(corpus("malformed/huge-page-size.pdf")).expect("read huge-page-size.pdf");

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source: huge_bytes,
            text_boxes: vec![],
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::LimitExceeded);
            assert_eq!(error.detail, "page too large");
        }
        Err(ref other) => {
            panic!("expected HostError::Worker(LimitExceeded), got: {other:?}");
        }
        Ok(ref response) => {
            panic!("expected edit refusal, got success: {response:?}");
        }
    }

    assert!(
        host.is_running(),
        "the worker must not be terminated by an oversized edit"
    );

    // The worker remains healthy and can open another document.
    let (_, opened) = host
        .open(&corpus("benign/multi-page-10.pdf"))
        .expect("open document after refused edit");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
}
