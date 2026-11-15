//! Editing and saving through the real worker in the real sandbox (B2-02, ADR 0013): the worker
//! changes the document in memory and writes it through a write-only handle to a file the main
//! process created; it never gets a path.
#![cfg(windows)]

mod common;

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
    common::check_qpdf(&path);

    // The saved file opens with the page turned, the others as they were.
    let (_, reopened) = opened(&mut host, &path);
    assert_eq!(reopened, turned);
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_signed_document_is_saved_incrementally_and_passes_qpdf() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/signed.pdf"));

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

    let (path, file) = new_file("saved-signed");
    let saved = host.save(doc, &file).expect("save");
    let WorkerResponse::Saved {
        bytes, incremental, ..
    } = saved
    else {
        panic!("{saved:?}");
    };
    drop(file);
    assert!(incremental);
    assert_eq!(std::fs::metadata(&path).expect("saved file").len(), bytes);
    common::check_qpdf(&path);

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
    common::check_qpdf(&path);
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

#[test]
fn page_edits_update_page_labels_matching_save_and_reopen() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("benign/page-labels-styles.pdf"))
        .expect("open benign/page-labels-styles.pdf");

    let initial_labels = match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 10);
            document.page_labels.expect("has page labels")
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    };
    assert_eq!(initial_labels.len(), 10);

    // 1. Delete a page (page 3)
    let delete_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::DeletePages { pages: vec![3] },
        })
        .expect("delete page must succeed");
    let delete_labels = match delete_res {
        WorkerResponse::Edited {
            pages, page_labels, ..
        } => {
            assert_eq!(pages.len(), 9);
            let labels = page_labels.expect("has page labels after delete");
            assert_eq!(labels.len(), pages.len());
            labels
        }
        ref other => panic!("expected Edited, got: {other:?}"),
    };

    let (path1, file1) = new_file("after-delete");
    let saved1 = host.save(doc, &file1).expect("save after delete");
    assert!(matches!(saved1, WorkerResponse::Saved { .. }));
    drop(file1);
    let (_, reopened1) = host.open(&path1).expect("reopen after delete");
    match reopened1 {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 9);
            assert_eq!(document.page_labels, Some(delete_labels));
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }
    std::fs::remove_file(&path1).ok();

    // 2. Insert blank page at index 0
    let insert_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
        })
        .expect("insert blank page must succeed");
    let insert_labels = match insert_res {
        WorkerResponse::Edited {
            pages, page_labels, ..
        } => {
            assert_eq!(pages.len(), 10);
            let labels = page_labels.expect("has page labels after insert");
            assert_eq!(labels.len(), pages.len());
            labels
        }
        ref other => panic!("expected Edited, got: {other:?}"),
    };

    let (path2, file2) = new_file("after-insert");
    let saved2 = host.save(doc, &file2).expect("save after insert");
    assert!(matches!(saved2, WorkerResponse::Saved { .. }));
    drop(file2);
    let (_, reopened2) = host.open(&path2).expect("reopen after insert");
    match reopened2 {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 10);
            assert_eq!(document.page_labels, Some(insert_labels));
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }
    std::fs::remove_file(&path2).ok();

    // 3. Move a page (move page 2 to before 0)
    let move_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::MovePages {
                pages: vec![2],
                before: 0,
            },
        })
        .expect("move page must succeed");
    let move_labels = match move_res {
        WorkerResponse::Edited {
            pages, page_labels, ..
        } => {
            assert_eq!(pages.len(), 10);
            let labels = page_labels.expect("has page labels after move");
            assert_eq!(labels.len(), pages.len());
            labels
        }
        ref other => panic!("expected Edited, got: {other:?}"),
    };

    let (path3, file3) = new_file("after-move");
    let saved3 = host.save(doc, &file3).expect("save after move");
    assert!(matches!(saved3, WorkerResponse::Saved { .. }));
    drop(file3);
    let (_, reopened3) = host.open(&path3).expect("reopen after move");
    match reopened3 {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 10);
            assert_eq!(document.page_labels, Some(move_labels));
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }
    std::fs::remove_file(&path3).ok();

    assert!(host.is_running());
}

#[test]
fn unlabeled_document_page_edits_keep_page_labels_none_and_save_reopen() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert_eq!(pages.len(), 10);

    // 1. Delete page 3 (0-based index 2)
    let del_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::DeletePages { pages: vec![2] },
        })
        .expect("delete page 3");
    let WorkerResponse::Edited {
        pages: pages_after_del,
        page_labels: del_labels,
        ..
    } = del_res
    else {
        panic!("expected Edited, got {del_res:?}");
    };
    assert_eq!(pages_after_del.len(), 9);
    assert_eq!(del_labels, None);

    // 2. Move page 4 (0-based index 3, which was page 5 before delete) to before 0
    let move_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::MovePages {
                pages: vec![3],
                before: 0,
            },
        })
        .expect("move page 4 to front");
    let WorkerResponse::Edited {
        pages: pages_after_move,
        page_labels: move_labels,
        ..
    } = move_res
    else {
        panic!("expected Edited, got {move_res:?}");
    };
    assert_eq!(pages_after_move.len(), 9);
    assert_eq!(move_labels, None);

    // 3. Rotate page 3 (0-based index 2) by 90 degrees CW
    let rot_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::RotatePages {
                pages: vec![2],
                degrees: 90,
            },
        })
        .expect("rotate page 3");
    let WorkerResponse::Edited {
        pages: pages_after_rot,
        page_labels: rot_labels,
        ..
    } = rot_res
    else {
        panic!("expected Edited, got {rot_res:?}");
    };
    assert_eq!(pages_after_rot.len(), 9);
    assert_eq!(rot_labels, None);

    // Save and reopen
    let (path, file) = new_file("unlabeled-rearranged");
    let saved = host.save(doc, &file).expect("save");
    assert!(matches!(saved, WorkerResponse::Saved { .. }));
    drop(file);

    let (_, reopened) = host.open(&path).expect("reopen");
    let WorkerResponse::Opened { document, .. } = reopened else {
        panic!("expected Opened, got {reopened:?}");
    };
    assert_eq!(document.pages.len(), 9);
    assert_eq!(document.page_labels, None);
    // Page 3 (0-based index 2) is landscape (width > height), while page 2 (index 1) is portrait.
    assert!(document.pages[1].width_pt < document.pages[1].height_pt);
    assert!(document.pages[2].width_pt > document.pages[2].height_pt);
    std::fs::remove_file(&path).ok();
}

#[test]
fn unlabeled_document_insert_blank_and_source_pages_at_front_keeps_page_labels_none() {
    let mut host = host();
    let (doc, pages) = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert_eq!(pages.len(), 10);

    // 1. Insert blank page at index 0
    let insert_blank = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
        })
        .expect("insert blank page");
    let WorkerResponse::Edited {
        pages: blank_pages,
        page_labels: blank_labels,
        ..
    } = insert_blank
    else {
        panic!("expected Edited, got {insert_blank:?}");
    };
    assert_eq!(blank_pages.len(), 11);
    assert_eq!(blank_labels, None);

    // 2. Insert pages from another document at index 0
    let source_file = File::open(corpus("benign/single-page.pdf")).expect("open source pdf");
    let WorkerResponse::Source {
        bytes: source_bytes,
        ..
    } = host
        .prepare_source(&source_file, None)
        .expect("prepare source")
    else {
        panic!("expected Source response");
    };
    let insert_source = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertPages {
                at: 0,
                source: source_bytes,
                text_boxes: Vec::new(),
            },
        })
        .expect("insert source pages");
    let WorkerResponse::Edited {
        pages: source_pages,
        page_labels: source_labels,
        ..
    } = insert_source
    else {
        panic!("expected Edited, got {insert_source:?}");
    };
    assert_eq!(source_pages.len(), 12);
    assert_eq!(source_labels, None);

    // Save and reopen
    let (path, file) = new_file("unlabeled-inserted");
    let saved = host.save(doc, &file).expect("save");
    assert!(matches!(saved, WorkerResponse::Saved { .. }));
    drop(file);

    let (_, reopened) = host.open(&path).expect("reopen");
    let WorkerResponse::Opened { document, .. } = reopened else {
        panic!("expected Opened, got {reopened:?}");
    };
    assert_eq!(document.pages.len(), 12);
    assert_eq!(document.page_labels, None);
    std::fs::remove_file(&path).ok();
}

#[test]
fn labeled_document_insert_blank_page_at_front_keeps_page_labels() {
    let mut host = host();
    let (doc, _) = host
        .open(&corpus("benign/page-labels-styles.pdf"))
        .expect("open");
    let insert_res = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertBlankPage { at: 0, like: 0 },
        })
        .expect("insert blank page at 0");
    let WorkerResponse::Edited {
        pages, page_labels, ..
    } = insert_res
    else {
        panic!("expected Edited, got {insert_res:?}");
    };
    assert_eq!(pages.len(), 11);
    // MuPDF adjusts page labels when inserting a page before page 0 of a document with labels.
    // We verify labels remain present and match the updated page count.
    let labels = page_labels.expect("labeled document keeps page labels after insert");
    assert_eq!(labels.len(), pages.len());
}
