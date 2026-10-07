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

fn pages_of(response: Result<WorkerResponse, worker_host::HostError>) -> Vec<PageSize> {
    match response.expect("request") {
        WorkerResponse::Edited { pages, .. } => pages,
        other => panic!("{other:?}"),
    }
}

#[test]
fn undo_opens_the_kept_bytes_again_and_applies_the_edits_before() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let turn = WorkerEdit::RotatePages {
        pages: vec![0],
        degrees: 90,
    };
    let delete = WorkerEdit::DeletePages { pages: vec![9] };
    for edit in [turn.clone(), delete] {
        pages_of(host.request(|request| WorkerRequest::Edit { request, doc, edit }));
    }
    // Undoing the deletion: the file's bytes again, turned once.
    let undone = pages_of(host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: vec![turn.clone()],
        password: None,
    }));
    assert_eq!(undone.len(), 10);
    assert_eq!(
        (undone[0].width_pt, undone[0].height_pt),
        (pages[0].height_pt, pages[0].width_pt)
    );
    // Undoing everything: the file as it was.
    let original = pages_of(host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: Vec::new(),
        password: None,
    }));
    assert_eq!(original, pages);

    // One edit that cannot be applied: refused, and the document stays as it was.
    let refused = host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: vec![turn, WorkerEdit::DeletePages { pages: vec![42] }],
        password: None,
    });
    assert!(refused.is_err(), "{refused:?}");
    let still = pages_of(host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: Vec::new(),
        password: None,
    }));
    assert_eq!(still, pages);
}

#[test]
fn a_document_opened_with_a_password_is_opened_again_with_it() {
    use ipc_contract::types::Password;
    let mut host = host();
    let response = host
        .open_with_password(
            &corpus("benign/encrypted-aes256.pdf"),
            Password::new("user".to_owned()),
        )
        .expect("open");
    let (doc, WorkerResponse::Opened { document, .. }) = response else {
        panic!("{response:?}");
    };
    // Its password is not kept: without it, or with a wrong one, undo is refused.
    for password in [None, Some(Password::new("wrong".to_owned()))] {
        let refused = host.request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: Vec::new(),
            password,
        });
        assert!(refused.is_err(), "{refused:?}");
    }
    assert!(host.is_running());
    let reverted = pages_of(host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: Vec::new(),
        password: Some(Password::new("user".to_owned())),
    }));
    assert_eq!(reverted, document.pages);
}

#[test]
fn after_a_save_undo_opens_the_saved_file_again() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let delete = WorkerEdit::DeletePages { pages: vec![9] };
    pages_of(host.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: delete,
    }));
    let (path, file) = new_file("rebase");
    host.save(doc, &file).expect("save");
    drop(file);
    let rebased = host.rebase(doc, &path).expect("rebase");
    assert!(
        matches!(rebased, WorkerResponse::Rebased { .. }),
        "{rebased:?}"
    );
    // Undoing everything since the save: the saved file, nine pages, not the ten it was opened with.
    let reverted = pages_of(host.request(|request| WorkerRequest::Revert {
        request,
        doc,
        edits: Vec::new(),
        password: None,
    }));
    assert_eq!(reverted.len(), pages.len() - 1);
    std::fs::remove_file(&path).ok();
}

#[test]
fn some_pages_are_saved_as_a_document_of_their_own_and_the_document_is_not_changed() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert_eq!(pages.len(), 10);

    let (path, file) = new_file("pages");
    let saved = host.save_pages(doc, &[1, 2, 6], &file).expect("save pages");
    let WorkerResponse::Saved {
        bytes, incremental, ..
    } = saved
    else {
        panic!("{saved:?}");
    };
    drop(file);
    assert!(!incremental);
    assert_eq!(std::fs::metadata(&path).expect("file").len(), bytes);
    let (_, copy) = opened(&mut host, &path);
    assert_eq!(copy.len(), 3);
    std::fs::remove_file(&path).ok();

    // The document still has all its pages, and the worker serves it.
    let again = host
        .request(|request| WorkerRequest::GetOutline { request, doc })
        .expect("the document is still there");
    assert!(matches!(again, WorkerResponse::Outline { .. }), "{again:?}");

    // What is not a list of this document's pages is refused, and writes nothing.
    for wrong in [vec![], vec![10], vec![3, 3]] {
        let (path, file) = new_file("refused");
        assert!(host.save_pages(doc, &wrong, &file).is_err(), "{wrong:?}");
        drop(file);
        assert_eq!(std::fs::metadata(&path).expect("file").len(), 0);
        std::fs::remove_file(&path).ok();
    }
    assert!(host.is_running());
}
