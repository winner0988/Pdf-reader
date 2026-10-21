//! Single-document text cache in the real worker and real sandbox (Issue #242).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::types::{DocumentId, Rect, Rgb, SearchHit, TextAlign, TextBoxStyle};
use ipc_contract::worker::{FontData, OcrOutcome, WorkerEdit, WorkerRequest, WorkerResponse};
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

#[test]
fn search_then_rotate_page_invalidates_cache_and_updates_coordinates() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Search page 0 for "Page"
    let (hits, has_text) = search_page(&mut host, doc, 0, "Page");
    assert!(has_text);
    assert_eq!(hits.len(), 1);
    let original_quad = hits[0].quads[0];

    // Rotate page 0 by 90 degrees
    edit(
        &mut host,
        doc,
        WorkerEdit::RotatePages {
            pages: vec![0],
            degrees: 90,
        },
    );

    // Searching again retrieves the newly rotated page's coordinates
    let (hits_after, has_text_after) = search_page(&mut host, doc, 0, "Page");
    assert!(has_text_after);
    assert_eq!(hits_after.len(), 1);
    let rotated_quad = hits_after[0].quads[0];
    assert_ne!(original_quad, rotated_quad);
}

#[test]
fn search_then_move_pages_invalidates_cache_and_updates_page_numbers() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Page 6 contains "needle"
    let (hits, has_text) = search_page(&mut host, doc, 6, "needle");
    assert!(has_text);
    assert_eq!(hits.len(), 1);

    // Move page 6 to before page 0
    edit(
        &mut host,
        doc,
        WorkerEdit::MovePages {
            pages: vec![6],
            before: 0,
        },
    );

    // Now page 0 has "needle"
    let (hits_p0, has_text_p0) = search_page(&mut host, doc, 0, "needle");
    assert!(has_text_p0);
    assert_eq!(hits_p0.len(), 1);

    // And page 6 no longer has "needle"
    let (hits_p6, _) = search_page(&mut host, doc, 6, "needle");
    assert!(hits_p6.is_empty());
}

#[test]
fn search_then_insert_blank_page_invalidates_cache() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Search page 0 and page 7 before insertion to populate the cache.
    // Page 0 contains "Page 1 of 10" (has text, no needle).
    let (hits_p0_before, has_text_p0_before) = search_page(&mut host, doc, 0, "needle");
    assert!(has_text_p0_before);
    assert!(hits_p0_before.is_empty());

    // Page 7 contains "Page 8 of 10" (has text, no needle).
    let (hits_p7_before, has_text_p7_before) = search_page(&mut host, doc, 7, "needle");
    assert!(has_text_p7_before);
    assert!(hits_p7_before.is_empty());

    // Insert blank page at index 0
    edit(
        &mut host,
        doc,
        WorkerEdit::InsertBlankPage { at: 0, like: 0 },
    );

    // Page 0 is now blank (no text; if cache were not invalidated, it would still say has_text: true)
    let (hits_p0, has_text_p0) = search_page(&mut host, doc, 0, "needle");
    assert!(!has_text_p0);
    assert!(hits_p0.is_empty());

    // Page 7 now has "needle" (shifted from page 6; if cache were not invalidated, it would return 0 hits from old page 7)
    let (hits_p7, has_text_p7) = search_page(&mut host, doc, 7, "needle");
    assert!(has_text_p7);
    assert_eq!(hits_p7.len(), 1);
}

#[test]
fn search_then_place_text_box_finds_new_text() {
    let (mut host, doc) = open("benign/single-page.pdf");

    // Search for non-existent keyword
    let (hits, _) = search_page(&mut host, doc, 0, "SearchableBoxText");
    assert!(hits.is_empty());

    // Prepare text box model
    let font = FontData {
        bytes: mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
            .expect("the bundled font")
            .data
            .to_vec(),
        face: 0,
        name: "Droid Sans Fallback".to_owned(),
    };
    let model = match host
        .request(|request| WorkerRequest::PrepareTextBox {
            request,
            font,
            text: "SearchableBoxText".to_owned(),
        })
        .expect("prepare text box")
    {
        WorkerResponse::TextBoxPrepared { model, .. } => model,
        other => panic!("unexpected response: {other:?}"),
    };

    // Place text box on page 0
    let style = TextBoxStyle {
        size: 14.0,
        color: Rgb { r: 0, g: 0, b: 0 },
        align: TextAlign::Left,
    };
    let rect = Rect {
        x0: 50.0,
        y0: 50.0,
        x1: 250.0,
        y1: 100.0,
    };
    edit(
        &mut host,
        doc,
        WorkerEdit::PlaceTextBox {
            page: 0,
            rect,
            model,
            style,
        },
    );

    // Searching again finds the new text from the text box
    let (hits_after, has_text_after) = search_page(&mut host, doc, 0, "SearchableBoxText");
    assert!(has_text_after);
    assert_eq!(hits_after.len(), 1);

    // Repeated search hits the in-memory cache and returns identical results
    let (hits_cached, _) = search_page(&mut host, doc, 0, "SearchableBoxText");
    assert_eq!(hits_cached, hits_after);
}

#[test]
fn search_then_revert_undo_restores_original_cache_state() {
    let (mut host, doc) = open("benign/multi-page-10.pdf");

    // Page 6 contains "needle"
    let (hits, has_text) = search_page(&mut host, doc, 6, "needle");
    assert!(has_text);
    assert_eq!(hits.len(), 1);

    // Delete page 0: former page 6 becomes page 5
    edit(&mut host, doc, WorkerEdit::DeletePages { pages: vec![0] });

    let (hits_p5, _) = search_page(&mut host, doc, 5, "needle");
    assert_eq!(hits_p5.len(), 1);

    // Revert (undo) to original state (empty edits)
    match host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: Vec::new(),
            password: None,
        })
        .expect("revert")
    {
        WorkerResponse::Edited { pages, .. } => assert_eq!(pages.len(), 10),
        other => panic!("unexpected response: {other:?}"),
    }

    // Now page 6 has "needle" again, and page 5 does not
    let (hits_after_revert_p6, _) = search_page(&mut host, doc, 6, "needle");
    assert_eq!(hits_after_revert_p6.len(), 1);
    let (hits_after_revert_p5, _) = search_page(&mut host, doc, 5, "needle");
    assert!(hits_after_revert_p5.is_empty());
}

#[test]
fn search_then_ocr_recognized_invalidates_cache_and_finds_ocr_text() {
    let (mut host, doc) = open("benign/scanned-text.pdf");

    // Scanned page has no text layer initially
    let (hits, has_text) = search_page(&mut host, doc, 0, "PRIVACY");
    assert!(!has_text);
    assert!(hits.is_empty());

    // Load OCR language data
    let tessdata = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/tessdata/eng.traineddata");
    let data = std::fs::read(&tessdata).expect("read eng.traineddata");
    match host
        .request(|request| WorkerRequest::OcrLoad {
            request,
            language: "eng".to_owned(),
            data,
        })
        .expect("ocr load")
    {
        WorkerResponse::OcrLoaded { .. } => {}
        other => panic!("unexpected ocr load response: {other:?}"),
    }

    // Queue page 0 for OCR
    match host
        .request(|request| WorkerRequest::OcrPage {
            request,
            doc,
            page_index: 0,
            max_millis: 120_000,
        })
        .expect("ocr page")
    {
        WorkerResponse::OcrChecked {
            state: ipc_contract::worker::OcrPageState::Queued,
            ..
        } => {}
        other => panic!("unexpected ocr page response: {other:?}"),
    }

    // Poll until OCR completes (time-bounded wait, up to 120 seconds)
    let started = Instant::now();
    let mut recognized = false;
    while !recognized && started.elapsed() < Duration::from_secs(120) {
        match host
            .request(|request| WorkerRequest::OcrPoll { request })
            .expect("ocr poll")
        {
            WorkerResponse::OcrPolled { finished, .. } => {
                if finished
                    .iter()
                    .any(|f| matches!(f.outcome, OcrOutcome::Recognised { .. }))
                {
                    recognized = true;
                    break;
                }
            }
            other => panic!("unexpected ocr poll response: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(recognized, "OCR did not complete in time");

    // After OCR recognition, text cache must be invalidated, and search now finds "PRIVACY"!
    let (hits_after, has_text_after) = search_page(&mut host, doc, 0, "PRIVACY");
    assert!(has_text_after);
    assert_eq!(hits_after.len(), 1);
}
