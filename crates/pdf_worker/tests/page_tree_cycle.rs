//! Page tree /Parent cycle validation when editing documents (Issue #544).
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipc_contract::types::Rotation;
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
        HostConfig {
            request_timeout: Duration::from_secs(10),
            ..HostConfig::default()
        },
    )
}

#[test]
fn move_pages_on_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-parent-cycle.pdf"))
        .expect("open page-tree-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::MovePages {
            pages: vec![0],
            before: 2,
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(
        host.is_running(),
        "the worker must not be terminated by a refused edit"
    );

    // The same worker can open a benign document afterwards.
    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn delete_pages_on_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-parent-cycle.pdf"))
        .expect("open page-tree-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::DeletePages { pages: vec![0] },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_blank_page_on_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-parent-cycle.pdf"))
        .expect("open page-tree-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_pages_on_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-parent-cycle.pdf"))
        .expect("open page-tree-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let source = fs::read(corpus("benign/single-page.pdf")).expect("read single-page.pdf");
    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source,
            text_boxes: vec![],
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn read_only_operations_on_parent_cycle_sample_succeed() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-parent-cycle.pdf"))
        .expect("open page-tree-parent-cycle.pdf");

    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
            assert_eq!(document.pages[0].width_pt, 612.0);
            assert_eq!(document.pages[0].height_pt, 792.0);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    // Render page 0
    let render_res = host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        })
        .expect("render page 0 must succeed");
    assert!(matches!(render_res, WorkerResponse::Rendered { .. }));

    // Get page text for page 0
    let text_res = host
        .request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index: 0,
        })
        .expect("get page text must succeed");
    assert!(matches!(text_res, WorkerResponse::PageText { .. }));

    // Search page 0
    let search_res = host
        .request(|request| WorkerRequest::SearchPage {
            request,
            doc,
            page_index: 0,
            query: "Parent".to_string(),
            case_sensitive: false,
            max_hits: 10,
        })
        .expect("search page must succeed");
    assert!(matches!(search_res, WorkerResponse::PageSearched { .. }));

    assert!(host.is_running());
}

#[test]
fn move_pages_on_middle_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-middle-parent-cycle.pdf"))
        .expect("open page-tree-middle-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::MovePages {
            pages: vec![0],
            before: 2,
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(
        host.is_running(),
        "the worker must not be terminated by a refused edit"
    );

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn delete_pages_on_middle_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-middle-parent-cycle.pdf"))
        .expect("open page-tree-middle-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::DeletePages { pages: vec![0] },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_blank_page_on_middle_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-middle-parent-cycle.pdf"))
        .expect("open page-tree-middle-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_pages_on_middle_parent_cycle_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-middle-parent-cycle.pdf"))
        .expect("open page-tree-middle-parent-cycle.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let source = fs::read(corpus("benign/single-page.pdf")).expect("read single-page.pdf");
    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source,
            text_boxes: vec![],
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn move_pages_on_shared_kids_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-shared-kids.pdf"))
        .expect("open page-tree-shared-kids.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::MovePages {
            pages: vec![0],
            before: 2,
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(
        host.is_running(),
        "the worker must not be terminated by a refused edit"
    );

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn delete_pages_on_shared_kids_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-shared-kids.pdf"))
        .expect("open page-tree-shared-kids.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::DeletePages { pages: vec![0] },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_blank_page_on_shared_kids_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-shared-kids.pdf"))
        .expect("open page-tree-shared-kids.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}

#[test]
fn insert_pages_on_shared_kids_is_refused_with_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/page-tree-shared-kids.pdf"))
        .expect("open page-tree-shared-kids.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 2);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let source = fs::read(corpus("benign/single-page.pdf")).expect("read single-page.pdf");
    let edit_res = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source,
            text_boxes: vec![],
        },
    });

    match edit_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert_eq!(
                error.detail,
                "the page tree is corrupted: /Parent chain does not end"
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected edit refusal, got success: {response:?}"),
    }

    assert!(host.is_running());

    let (_doc, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());
}
