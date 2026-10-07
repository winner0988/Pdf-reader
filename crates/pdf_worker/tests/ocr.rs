//! Recognising the text of scanned pages (B2-10, ADR 0015, docs/architecture/ocr.md) through the
//! real worker in its own sandbox (AppContainer, win32k disabled, no network, no files) and the
//! main process's validation of what it answers.
//!
//! The scans are made here: a corpus page rendered by MuPDF and put alone on a new page as an
//! image has no text layer, like a scanned page. The language data is the installer's
//! (src-tauri/resources/tessdata).
#![cfg(windows)]

#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::types::{DocumentId, PageText, SearchHit};
use ipc_contract::worker::{
    OcrFinished, OcrOutcome, OcrPageState, WorkerEdit, WorkerErrorCode, WorkerRequest,
    WorkerResponse,
};
use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument};
use mupdf::{Colorspace, Document, Matrix};
use worker_host::{HostConfig, HostError, WorkerHost};

const PRIVACY: &str = "benign/mixed-text-zh-en.pdf";
const HELLO: &str = "benign/single-page.pdf";

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn traineddata(language: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/tessdata")
        .join(format!("{language}.traineddata"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A document with a page for each corpus file in `names`: its first page as a picture of
/// itself, at 200 dpi, and nothing else.
fn scanned_pdf(names: &[&str]) -> Vec<u8> {
    let mut scanned = PdfDocument::new();
    for (index, name) in names.iter().enumerate() {
        let pdf = std::fs::read(corpus(name)).expect("corpus file");
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let page = doc.load_page(0).expect("page");
        let bounds = page.bounds().expect("bounds");
        let scale = 200.0 / 72.0;
        let image = page
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render");
        let mut new_page = scanned
            .new_page_at(index as i32, (bounds.width(), bounds.height()))
            .expect("new page");
        new_page
            .insert_image(
                &mut scanned,
                bounds,
                PageImageSource::Pixmap(&image),
                InsertImageOptions::default(),
            )
            .expect("insert image");
    }
    let mut bytes = Vec::new();
    scanned.write_to(&mut bytes).expect("write");
    bytes
}

/// A worker with the scan of `names` open.
fn open_scan(names: &[&str]) -> (WorkerHost, DocumentId) {
    let path = common::temp_pdf(&format!("ocr-{}", names.len()), &scanned_pdf(names));
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let doc = host.open(&path).expect("open the scan").0;
    (host, doc)
}

fn open_corpus(name: &str) -> (WorkerHost, DocumentId) {
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let doc = host.open(&corpus(name)).expect("open").0;
    (host, doc)
}

fn load(host: &mut WorkerHost, language: &str) {
    let data = traineddata(language);
    let response = host
        .request(|request| WorkerRequest::OcrLoad {
            request,
            language: language.to_owned(),
            data,
        })
        .expect("load the language");
    assert!(
        matches!(response, WorkerResponse::OcrLoaded { .. }),
        "{response:?}"
    );
}

fn check(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> OcrPageState {
    match host
        .request(|request| WorkerRequest::OcrPage {
            request,
            doc,
            page_index,
            max_millis: 120_000,
        })
        .expect("check the page")
    {
        WorkerResponse::OcrChecked { state, .. } => state,
        other => panic!("{other:?}"),
    }
}

/// What `OcrPoll` reports, and how many pages were waiting after it.
fn poll(host: &mut WorkerHost) -> (Vec<OcrFinished>, u32) {
    match host
        .request(|request| WorkerRequest::OcrPoll { request })
        .expect("poll")
    {
        WorkerResponse::OcrPolled {
            finished, waiting, ..
        } => (finished, waiting),
        other => panic!("{other:?}"),
    }
}

/// Polls until `count` pages are finished (or two minutes are up).
fn finished(host: &mut WorkerHost, count: usize) -> Vec<OcrFinished> {
    let started = Instant::now();
    let mut all = Vec::new();
    while all.len() < count && started.elapsed() < Duration::from_secs(120) {
        all.extend(poll(host).0);
        std::thread::sleep(Duration::from_millis(30));
    }
    all
}

fn text(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> PageText {
    match host
        .request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index,
        })
        .expect("page text")
    {
        WorkerResponse::PageText { text, .. } => text,
        other => panic!("{other:?}"),
    }
}

fn lines(text: &PageText) -> Vec<&str> {
    text.lines.iter().map(|line| line.text.as_str()).collect()
}

fn search(
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
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_scanned_page_is_recognised_in_the_sandbox_and_then_has_text() {
    let (mut host, doc) = open_scan(&[PRIVACY]);
    // Before: no text, no hits, no text layer at all.
    assert!(text(&mut host, doc, 0).lines.is_empty());
    assert_eq!(search(&mut host, doc, 0, "reader"), (Vec::new(), false));

    load(&mut host, "eng");
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Queued);
    assert_eq!(
        check(&mut host, doc, 0),
        OcrPageState::Queued,
        "not queued twice"
    );
    let done = finished(&mut host, 1);
    assert_eq!(
        done.iter()
            .map(|page| (page.doc, page.page_index))
            .collect::<Vec<_>>(),
        [(doc, 0)]
    );
    assert!(
        matches!(done[0].outcome, OcrOutcome::Recognised { chars } if chars > 20),
        "{done:?}"
    );
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Recognised);

    let page = text(&mut host, doc, 0);
    assert!(page.recognised && !page.truncated);
    assert!(
        lines(&page).contains(&"Privacy-first PDF Reader"),
        "{:?}",
        lines(&page)
    );
    // Search finds it, and where it is is where the words are on the page (page space).
    let (hits, has_text) = search(&mut host, doc, 0, "PDF reader");
    assert!(has_text);
    let [hit] = &hits[..] else { panic!("{hits:?}") };
    let quad = hit.quads[0];
    assert!(
        quad.ul.x > 50.0 && quad.ur.x < 400.0 && quad.ul.y > 30.0 && quad.ll.y < 120.0,
        "{quad:?}"
    );
}

fn edit(host: &mut WorkerHost, doc: DocumentId, edit: WorkerEdit) {
    let response = host
        .request(|request| WorkerRequest::Edit { request, doc, edit })
        .expect("edit");
    assert!(
        matches!(response, WorkerResponse::Edited { .. }),
        "{response:?}"
    );
}

/// Reads page `page_index` of the open scan, which must be a scan, and waits for it.
fn read(host: &mut WorkerHost, doc: DocumentId, page_indexes: &[u32]) {
    for &page in page_indexes {
        assert_eq!(check(host, doc, page), OcrPageState::Queued, "page {page}");
    }
    let done = finished(host, page_indexes.len());
    assert_eq!(done.len(), page_indexes.len(), "{done:?}");
    assert!(
        done.iter()
            .all(|page| matches!(page.outcome, OcrOutcome::Recognised { .. })),
        "{done:?}"
    );
}

#[test]
fn pages_with_text_of_their_own_are_not_scans_and_a_language_is_needed() {
    let (mut host, doc) = open_corpus(PRIVACY);
    assert_eq!(check(&mut host, doc, 0), OcrPageState::NoLanguage);
    load(&mut host, "eng");
    assert_eq!(check(&mut host, doc, 0), OcrPageState::NotScan);
    assert_eq!(
        check(&mut host, doc, 0),
        OcrPageState::NotScan,
        "and it is remembered"
    );
    let error = host
        .request(|request| WorkerRequest::OcrPage {
            request,
            doc,
            page_index: 9,
            max_millis: 1000,
        })
        .expect_err("no such page");
    assert!(
        matches!(error, HostError::Worker(error) if error.code == WorkerErrorCode::PageOutOfRange)
    );
    // Its own text is what the page has.
    assert!(!text(&mut host, doc, 0).recognised);
}

#[test]
fn a_picture_without_text_is_read_and_stays_without() {
    let (mut host, doc) = open_corpus("benign/image-only.pdf");
    load(&mut host, "eng");
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Queued);
    let done = finished(&mut host, 1);
    assert_eq!(
        done[0].outcome,
        OcrOutcome::Recognised { chars: 0 },
        "{done:?}"
    );
    let page = text(&mut host, doc, 0);
    assert!(page.lines.is_empty() && !page.recognised);
    assert_eq!(search(&mut host, doc, 0, "a"), (Vec::new(), false));
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Recognised);
}

#[test]
fn chinese_is_recognised_with_the_chinese_data() {
    let (mut host, doc) = open_scan(&[PRIVACY]);
    load(&mut host, "chi_tra");
    read(&mut host, doc, &[0]);
    let page = text(&mut host, doc, 0);
    assert!(
        lines(&page)
            .iter()
            .any(|line| line.replace(' ', "").contains("隱私優先的PDF閱讀器")),
        "{:?}",
        lines(&page)
    );
    let (hits, _) = search(&mut host, doc, 0, "閱讀器");
    assert_eq!(hits.len(), 1);
}

#[test]
fn turning_a_page_or_opening_the_document_again_drops_what_was_read() {
    let (mut host, doc) = open_scan(&[PRIVACY]);
    load(&mut host, "eng");
    read(&mut host, doc, &[0]);
    assert!(text(&mut host, doc, 0).recognised);
    edit(
        &mut host,
        doc,
        WorkerEdit::RotatePages {
            pages: vec![0],
            degrees: 90,
        },
    );
    assert!(text(&mut host, doc, 0).lines.is_empty());
    read(&mut host, doc, &[0]);
    assert!(text(&mut host, doc, 0).recognised);
    // Undo opens the document again: nothing read is carried over.
    let reverted = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: Vec::new(),
            password: None,
        })
        .expect("revert");
    assert!(
        matches!(reverted, WorkerResponse::Edited { .. }),
        "{reverted:?}"
    );
    assert!(text(&mut host, doc, 0).lines.is_empty());
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Queued);
}

#[test]
fn what_was_read_stays_with_its_page_as_pages_are_deleted_moved_and_inserted() {
    let (mut host, doc) = open_scan(&[PRIVACY, HELLO]);
    load(&mut host, "eng");
    read(&mut host, doc, &[0, 1]);
    let hello =
        |host: &mut WorkerHost, page| lines(&text(host, doc, page)).contains(&"Hello, PDF Reader.");
    assert!(hello(&mut host, 1));

    edit(
        &mut host,
        doc,
        WorkerEdit::InsertBlankPage { at: 0, like: 0 },
    );
    assert!(
        text(&mut host, doc, 0).lines.is_empty(),
        "the new page has none"
    );
    assert!(hello(&mut host, 2));
    assert_eq!(check(&mut host, doc, 2), OcrPageState::Recognised);

    edit(
        &mut host,
        doc,
        WorkerEdit::MovePages {
            pages: vec![2],
            before: 0,
        },
    );
    assert!(hello(&mut host, 0));
    assert_eq!(check(&mut host, doc, 0), OcrPageState::Recognised);

    edit(
        &mut host,
        doc,
        WorkerEdit::DeletePages { pages: vec![1, 2] },
    );
    // Only the page that has Hello's picture is left, and it has its text still.
    assert!(hello(&mut host, 0));
    let (hits, has_text) = search(&mut host, doc, 0, "hello");
    assert!(has_text && hits.len() == 1);
}

#[test]
fn a_language_file_that_is_not_one_is_refused_and_the_worker_goes_on() {
    let (mut host, doc) = open_scan(&[PRIVACY]);
    for (language, data) in [
        ("eng", Vec::new()),
        ("eng", vec![0; 100]),
        ("../eng", traineddata("eng")),
    ] {
        let error = host
            .request(|request| WorkerRequest::OcrLoad {
                request,
                language: language.to_owned(),
                data: data.clone(),
            })
            .expect_err("refused");
        assert!(
            matches!(&error, HostError::Worker(error) if error.code == WorkerErrorCode::InvalidRequest),
            "{error:?}"
        );
    }
    assert_eq!(check(&mut host, doc, 0), OcrPageState::NoLanguage);
}

#[test]
fn the_worker_answers_other_requests_while_pages_are_read() {
    let (mut host, doc) = open_scan(&[PRIVACY, PRIVACY, PRIVACY, PRIVACY]);
    load(&mut host, "eng");
    for page in 0..4 {
        assert_eq!(check(&mut host, doc, page), OcrPageState::Queued);
    }
    // Full: as many as the queue holds, until some are collected.
    let started = Instant::now();
    let rendered = host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: ipc_contract::types::Rotation::None,
        })
        .expect("render");
    assert!(matches!(rendered, WorkerResponse::Rendered { .. }));
    let render_time = started.elapsed();
    let (done, waiting) = poll(&mut host);
    eprintln!(
        "render while reading: {render_time:?}; finished {}, waiting {waiting}",
        done.len()
    );
    assert!(
        done.len() < 4 && waiting > 0,
        "the render was answered only after all four pages were read: {done:?}"
    );
    // Stopping drops what is waiting, and the pages can be asked for again.
    let stopped = host
        .request(|request| WorkerRequest::OcrStop { request })
        .expect("stop");
    assert!(matches!(stopped, WorkerResponse::OcrStopped { .. }));
    let started = Instant::now();
    while poll(&mut host).1 > 0 {
        assert!(started.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(20));
    }
}
