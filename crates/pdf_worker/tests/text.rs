//! Page text for selecting and copying (MVP-15): the lines of the corpus's text samples, through
//! the real worker in its sandbox and the main process's validation.
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, PageText};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn corpus(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(path)
}

fn open(path: &str) -> (WorkerHost, DocumentId) {
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let doc = host
        .open(&corpus(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .0;
    (host, doc)
}

fn request_text(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Result<WorkerResponse, HostError> {
    host.request(|request| WorkerRequest::GetPageText {
        request,
        doc,
        page_index,
    })
}

fn page_text(path: &str, page_index: u32) -> PageText {
    let (mut host, doc) = open(path);
    match request_text(&mut host, doc, page_index).unwrap() {
        WorkerResponse::PageText {
            page_index: index,
            text,
            ..
        } if index == page_index => text,
        other => panic!("{path}: {other:?}"),
    }
}

fn lines(text: &PageText) -> Vec<&str> {
    text.lines.iter().map(|line| line.text.as_str()).collect()
}

fn search_hits(host: &mut WorkerHost, doc: DocumentId, query: &str) -> usize {
    match host
        .request(|request| WorkerRequest::SearchPage {
            request,
            doc,
            page_index: 0,
            query: query.to_owned(),
            case_sensitive: false,
            max_hits: 100,
        })
        .unwrap_or_else(|error| panic!("{query}: {error}"))
    {
        WorkerResponse::PageSearched { hits, .. } => hits.len(),
        other => panic!("{query}: {other:?}"),
    }
}

#[test]
fn chinese_and_english_lines_come_out_as_written() {
    // The Chinese line is in a CNS1 font that is not embedded: its text comes from the
    // ToUnicode map, whatever font draws it (DEC-03).
    let text = page_text("benign/mixed-text-zh-en.pdf", 0);
    assert!(!text.truncated);
    let lines = lines(&text);
    assert!(lines.contains(&"Privacy-first PDF Reader"), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("隱私優先") && line.contains("PDF 閱讀器")),
        "{lines:?}"
    );
    for line in &text.lines {
        assert_eq!(line.edges.len(), line.text.chars().count() + 1, "{line:?}");
    }
}

#[test]
fn every_page_has_its_own_text() {
    let seventh = page_text("benign/multi-page-10.pdf", 6);
    assert!(lines(&seventh).iter().any(|line| line.contains("needle")));
    let first = page_text("benign/multi-page-10.pdf", 0);
    assert!(
        lines(&first).contains(&"Page 1 of 10"),
        "{:?}",
        lines(&first)
    );
    assert!(!lines(&first).iter().any(|line| line.contains("needle")));
}

#[test]
fn text_that_is_drawn_and_clipped_comes_out_once() {
    // Text rendering modes 4 to 6 draw the glyphs and also add them to the clip that ET applies.
    // MuPDF 1.28 took that clip copy as text a second time (#628); the build-time patch in
    // mupdf-sys gives the copy no character. Text that is only clipped (mode 7) still comes out,
    // and a word the page really draws twice comes out twice.
    let path = "benign/text-render-modes.pdf";
    let text = page_text(path, 0);
    let all = lines(&text).join("\n");
    for word in [
        "Mode0Fill",
        "Mode4FillClip",
        "Mode5StrokeClip",
        "Mode6FillStrokeClip",
        "Mode7ClipOnly",
        "MixedFill",
        "MixedClip",
        "MixedStroke",
    ] {
        assert_eq!(all.matches(word).count(), 1, "{word} in {all:?}");
    }
    assert_eq!(all.matches("Twice").count(), 2, "{all:?}");

    let (mut host, doc) = open(path);
    for (query, hits) in [
        ("Mode4FillClip", 1),
        ("Mode7ClipOnly", 1),
        ("MixedStroke", 1),
        ("Twice", 2),
    ] {
        assert_eq!(search_hits(&mut host, doc, query), hits, "{query}");
    }
}

#[test]
fn glyph_names_give_the_characters_they_stand_for() {
    // MuPDF 1.28 reads a glyph name of g and hexadecimal digits as a TrueType glyph index, as
    // old Ghostscript subsets use them (g0024 is A). A Type 3 font that names its glyphs after
    // the decimal character code (g72 is H) got wrong letters (#629); the build-time patch in
    // mupdf-sys leaves Type 3 fonts out of that guess.
    let type3 = page_text("benign/type3-decimal-glyph-names.pdf", 0);
    assert!(
        lines(&type3).contains(&"Hello World 2026"),
        "{:?}",
        lines(&type3)
    );
    let truetype = page_text("benign/truetype-glyph-index-names.pdf", 0);
    assert!(
        lines(&truetype).contains(&"GLYPH INDEX"),
        "{:?}",
        lines(&truetype)
    );
}

#[test]
fn a_page_without_a_text_layer_has_no_lines() {
    let text = page_text("benign/image-only.pdf", 0);
    assert!(text.lines.is_empty() && !text.truncated);
}

#[test]
fn a_page_that_does_not_exist_is_an_error() {
    let (mut host, doc) = open("benign/single-page.pdf");
    match request_text(&mut host, doc, 1) {
        Err(HostError::Worker(error)) => assert_eq!(error.code, WorkerErrorCode::PageOutOfRange),
        other => panic!("{other:?}"),
    }
}
