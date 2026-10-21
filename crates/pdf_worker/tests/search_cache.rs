//! Single-document text cache in the real worker and real sandbox (Issue #242).
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, Rect, SearchHit};
use ipc_contract::worker::{WorkerEdit, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(path)
}

fn open(path: &str) -> (WorkerHost, DocumentId) {
    let mut host = host();
    let doc = host
        .open(&corpus(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .0;
    (host, doc)
}

fn search_page(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
    query: &str,
) -> (Vec<SearchHit>, bool) {
    match host
        .request(|request| WorkerRequest::SearchPage {
            request,
            doc,
            page_index,
            query: query.to_owned(),
            case_sensitive: false,
            max_hits: 100,
        })
        .expect("search")
    {
        WorkerResponse::PageSearched { hits, has_text, .. } => (hits, has_text),
        other => panic!("unexpected response: {other:?}"),
    }
}

fn edit(host: &mut WorkerHost, doc: DocumentId, edit: WorkerEdit) {
    match host
        .request(|request| WorkerRequest::Edit { request, doc, edit })
        .expect("edit")
    {
        WorkerResponse::Edited { .. } => {}
        other => panic!("unexpected edit response: {other:?}"),
    }
}

#[test]
fn search_then_delete_page_preserves_correct_page_numbers() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Page 6 (7th page) contains "needle"
    let (hits, has_text) = search_page(&mut host, doc, 6, "needle");
    assert!(has_text);
    assert_eq!(hits.len(), 1);

    // Repeated search hits the in-memory cache and yields identical results
    let (hits2, has_text2) = search_page(&mut host, doc, 6, "needle");
    assert!(has_text2);
    assert_eq!(hits2, hits);

    // Delete page 0: former page 6 becomes page 5
    edit(&mut host, doc, WorkerEdit::DeletePages { pages: vec![0] });

    // Now page 5 has the keyword
    let (hits_after, has_text_after) = search_page(&mut host, doc, 5, "needle");
    assert!(has_text_after);
    assert_eq!(hits_after.len(), 1);

    // Former page 6 is now page 5; page 6 no longer has "needle"
    let (hits_page6, _) = search_page(&mut host, doc, 6, "needle");
    assert!(hits_page6.is_empty());
}

#[test]
fn search_then_apply_redaction_removes_redacted_text_from_search() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Search page 0 for "Page"
    let (hits, has_text) = search_page(&mut host, doc, 0, "Page");
    assert!(has_text);
    assert_eq!(hits.len(), 1);

    // Repeated search hits the cache
    let (hits2, _) = search_page(&mut host, doc, 0, "Page");
    assert_eq!(hits2, hits);

    // Redact the word "Page" on page 0
    let quad = hits[0].quads[0];
    let rect = Rect {
        x0: quad.ul.x - 2.0,
        y0: quad.ul.y - 2.0,
        x1: quad.lr.x + 2.0,
        y1: quad.lr.y + 2.0,
    };
    edit(&mut host, doc, WorkerEdit::AddRedaction { page: 0, rect });
    edit(&mut host, doc, WorkerEdit::ApplyRedactions);

    // After applying redactions, the cache must be invalidated and "Page" is gone
    let (hits_after, _) = search_page(&mut host, doc, 0, "Page");
    assert!(hits_after.is_empty());
}
