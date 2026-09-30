//! Editing and saving through the real worker in the real sandbox (B2-02, ADR 0013): the worker
//! changes the document in memory and writes it through a write-only handle to a file the main
//! process created; it never gets a path.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, PageSize};
use ipc_contract::worker::{WorkerEdit, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

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

/// A new, empty file to save into, as the main process creates it.
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

#[test]
fn an_edited_document_is_saved_through_a_write_only_handle() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));

    let edited = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::RotatePages {
                pages: vec![0],
                degrees: 90,
            },
        })
        .expect("edit");
    let WorkerResponse::Edited { pages: turned, .. } = edited else {
        panic!("{edited:?}");
    };
    assert_eq!(turned.len(), pages.len());
    assert_eq!(
        (turned[0].width_pt, turned[0].height_pt),
        (pages[0].height_pt, pages[0].width_pt)
    );
    assert_eq!(turned[1], pages[1]);

    let (path, file) = new_file("saved");
    let saved = host.save(doc, &file).expect("save");
    let WorkerResponse::Saved {
        bytes, incremental, ..
    } = saved
    else {
        panic!("{saved:?}");
    };
    drop(file);
    assert!(!incremental);
    assert_eq!(std::fs::metadata(&path).expect("saved file").len(), bytes);

    // The saved file opens with the page turned, the others as they were.
    let (_, reopened) = opened(&mut host, &path);
    assert_eq!(reopened, turned);
    std::fs::remove_file(&path).ok();
}

#[test]
fn an_edit_of_a_missing_page_is_refused() {
    let mut host = host();
    let (doc, _) = opened(&mut host, &corpus("benign/single-page.pdf"));
    let refused = host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::RotatePages {
            pages: vec![3],
            degrees: 90,
        },
    });
    assert!(refused.is_err(), "{refused:?}");
    // The worker is still there for the document.
    assert!(host.is_running());
}
