//! Tests for outline (bookmark) editing (B3-17).
#![cfg(windows)]

mod common;

use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, LinkTarget, OutlineResult};
use ipc_contract::worker::{WorkerEdit, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

use common::{one_page, temp_pdf};

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pdf_worker"))
}

struct TestDoc {
    host: WorkerHost,
    doc: DocumentId,
    path: PathBuf,
}

impl Drop for TestDoc {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

fn open_test_pdf(pdf: &[u8]) -> TestDoc {
    let path = temp_pdf("outline-edit", pdf);
    let mut host = WorkerHost::new(worker(), HostConfig::default());
    let (doc, _) = host.open(&path).expect("open");
    TestDoc { host, doc, path }
}

fn get_outline(host: &mut WorkerHost, doc: DocumentId) -> OutlineResult {
    let resp = host
        .request(|request| WorkerRequest::GetOutline { request, doc })
        .expect("get outline");
    match resp {
        WorkerResponse::Outline { outline, .. } => outline,
        other => panic!("expected Outline response, got {other:?}"),
    }
}

fn apply_edit(host: &mut WorkerHost, doc: DocumentId, edit: WorkerEdit) -> Result<(), HostError> {
    let resp = host.request(|request| WorkerRequest::Edit { request, doc, edit })?;
    match resp {
        WorkerResponse::Edited { .. } => Ok(()),
        other => panic!("expected Edited response, got {other:?}"),
    }
}

#[test]
fn add_first_bookmark_to_empty_document() {
    let mut t = open_test_pdf(&one_page(""));
    let initial = get_outline(&mut t.host, t.doc);
    assert_eq!(initial.items.len(), 0);

    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "Chapter 1".into(),
            page: 0,
        },
    )
    .expect("add bookmark");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 1);
    assert_eq!(outline.items[0].title, "Chapter 1");
    assert_eq!(outline.items[0].depth, 0);
    assert_eq!(
        outline.items[0].target,
        Some(LinkTarget::Page {
            page_index: 0,
            x: None,
            y: None,
        })
    );
}

#[test]
fn add_child_rename_and_delete_bookmarks() {
    let mut t = open_test_pdf(&one_page(""));

    // 1. Add Chapter 1
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "Chapter 1".into(),
            page: 0,
        },
    )
    .expect("add chap 1");

    let outline = get_outline(&mut t.host, t.doc);
    let chap1_id = outline.items[0].id;

    // 2. Add Section 1.1 under Chapter 1
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: Some(chap1_id),
            index: 0,
            title: "Section 1.1".into(),
            page: 0,
        },
    )
    .expect("add sec 1.1");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 2);
    assert_eq!(outline.items[0].title, "Chapter 1");
    assert_eq!(outline.items[0].depth, 0);
    assert_eq!(outline.items[1].title, "Section 1.1");
    assert_eq!(outline.items[1].depth, 1);
    let sec11_id = outline.items[1].id;

    // 3. Rename Section 1.1
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::RenameBookmark {
            id: sec11_id,
            title: "1.1 Introduction".into(),
        },
    )
    .expect("rename sec 1.1");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items[1].title, "1.1 Introduction");

    // 4. Renaming to blank is rejected
    assert!(
        apply_edit(
            &mut t.host,
            t.doc,
            WorkerEdit::RenameBookmark {
                id: sec11_id,
                title: "   ".into(),
            },
        )
        .is_err()
    );

    // 5. Delete Section 1.1 (leaf)
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::DeleteBookmark { id: sec11_id },
    )
    .expect("delete sec 1.1");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 1);
    assert_eq!(outline.items[0].title, "Chapter 1");

    // 6. Delete Chapter 1 (becomes empty)
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::DeleteBookmark { id: chap1_id },
    )
    .expect("delete chap 1");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 0);
}

#[test]
fn move_bookmark_reorders_and_prevents_cycles() {
    let mut t = open_test_pdf(&one_page(""));

    // Create Chapter 1, Chapter 2, Chapter 3
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "A".into(),
            page: 0,
        },
    )
    .expect("A");
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 1,
            title: "B".into(),
            page: 0,
        },
    )
    .expect("B");
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 2,
            title: "C".into(),
            page: 0,
        },
    )
    .expect("C");

    let outline = get_outline(&mut t.host, t.doc);
    let a_id = outline.items[0].id;
    let b_id = outline.items[1].id;
    let c_id = outline.items[2].id;

    // Move C to index 0: should become C, A, B
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::MoveBookmark {
            id: c_id,
            parent: None,
            index: 0,
        },
    )
    .expect("move C to 0");

    let outline = get_outline(&mut t.host, t.doc);
    let titles: Vec<&str> = outline.items.iter().map(|it| it.title.as_str()).collect();
    assert_eq!(titles, ["C", "A", "B"]);

    // Demote B under A: should become C, A (depth 0), with B (depth 1)
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::MoveBookmark {
            id: b_id,
            parent: Some(a_id),
            index: 0,
        },
    )
    .expect("demote B under A");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 3);
    assert_eq!(outline.items[0].title, "C");
    assert_eq!(outline.items[0].depth, 0);
    assert_eq!(outline.items[1].title, "A");
    assert_eq!(outline.items[1].depth, 0);
    assert_eq!(outline.items[2].title, "B");
    assert_eq!(outline.items[2].depth, 1);

    // Attempting to move A into B (into its own descendant) must fail
    assert!(
        apply_edit(
            &mut t.host,
            t.doc,
            WorkerEdit::MoveBookmark {
                id: a_id,
                parent: Some(b_id),
                index: 0,
            },
        )
        .is_err()
    );
}

#[test]
fn replay_edits_through_revert_restores_identical_outline() {
    let mut t = open_test_pdf(&one_page(""));

    let edits = vec![
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "Chapter 1".into(),
            page: 0,
        },
        WorkerEdit::AddBookmark {
            parent: None,
            index: 1,
            title: "Chapter 2".into(),
            page: 0,
        },
    ];

    for edit in &edits {
        apply_edit(&mut t.host, t.doc, edit.clone()).expect("apply");
    }

    let before = get_outline(&mut t.host, t.doc);
    assert_eq!(before.items.len(), 2);

    // Replay edits via WorkerRequest::Revert
    let reverted = t
        .host
        .request(|request| WorkerRequest::Revert {
            request,
            doc: t.doc,
            edits,
            password: None,
        })
        .expect("revert");
    assert!(matches!(reverted, WorkerResponse::Edited { .. }));

    let after = get_outline(&mut t.host, t.doc);
    assert_eq!(after.items.len(), 2);
    assert_eq!(after.items[0].title, "Chapter 1");
    assert_eq!(after.items[1].title, "Chapter 2");
}

#[test]
fn deleting_parent_removes_entire_subtree() {
    let mut t = open_test_pdf(&one_page(""));

    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "Parent".into(),
            page: 0,
        },
    )
    .expect("Parent");

    let outline = get_outline(&mut t.host, t.doc);
    let parent_id = outline.items[0].id;

    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: Some(parent_id),
            index: 0,
            title: "Child 1".into(),
            page: 0,
        },
    )
    .expect("Child 1");

    let outline = get_outline(&mut t.host, t.doc);
    let child1_id = outline.items[1].id;

    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: Some(child1_id),
            index: 0,
            title: "Grandchild".into(),
            page: 0,
        },
    )
    .expect("Grandchild");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 3);

    // Deleting Parent should remove Parent, Child 1, and Grandchild
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::DeleteBookmark { id: parent_id },
    )
    .expect("delete parent");

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 0);
}

#[test]
fn only_a_bookmark_shown_can_be_edited_and_the_worker_stays() {
    let mut t = open_test_pdf(&one_page(""));
    apply_edit(
        &mut t.host,
        t.doc,
        WorkerEdit::AddBookmark {
            parent: None,
            index: 0,
            title: "A".into(),
            page: 0,
        },
    )
    .expect("add A");
    let a = get_outline(&mut t.host, t.doc).items[0].id;

    // Every other object number (the catalog, the pages, the page, the outline root) is refused.
    for number in (0..=20).chain([u32::MAX]).filter(|&number| number != a.0) {
        let id = ipc_contract::types::BookmarkId(number);
        let edits = [
            WorkerEdit::RenameBookmark {
                id,
                title: "X".into(),
            },
            WorkerEdit::DeleteBookmark { id },
            WorkerEdit::MoveBookmark {
                id,
                parent: None,
                index: 0,
            },
            WorkerEdit::MoveBookmark {
                id: a,
                parent: Some(id),
                index: 0,
            },
        ];
        for edit in edits {
            assert!(apply_edit(&mut t.host, t.doc, edit).is_err(), "{number}");
        }
    }

    let outline = get_outline(&mut t.host, t.doc);
    assert_eq!(outline.items.len(), 1);
    assert_eq!(outline.items[0].title, "A");
}

#[test]
fn temp_pdf_produces_distinct_files_for_the_same_name() {
    let first = temp_pdf("same-name", b"first");
    let second = temp_pdf("same-name", b"second");

    assert_ne!(first, second);

    std::fs::remove_file(&first).expect("remove first file");
    assert!(second.exists());
    assert_eq!(std::fs::read(&second).expect("read second"), b"second");

    std::fs::remove_file(&second).ok();
}
