//! Page text for selecting and copying (MVP-15): the lines of the corpus's text samples, through
//! the real worker in its sandbox and the main process's validation.
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, PageText};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
use pdf_worker::engine::{PdfDocument, RenderedPage};
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
    // Mode 7 text comes out where the content shows it. MuPDF 1.28 sent it only at ET, after
    // MixedStroke, which the same text object draws after it (#638).
    assert!(
        lines(&text).contains(&"MixedFill MixedClip MixedStroke"),
        "{all:?}"
    );

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

/// The pixels of `page` inside `area` (left, top, right, bottom) for which `wanted` says yes.
fn pixels(
    page: &RenderedPage,
    area: [usize; 4],
    wanted: impl Fn([i32; 3]) -> bool,
) -> Vec<(usize, usize)> {
    let [x0, y0, x1, y1] = area;
    let mut found = Vec::new();
    for y in y0..y1.min(page.height as usize) {
        for x in x0..x1.min(page.width as usize) {
            let at = (y * page.width as usize + x) * 4;
            let [r, g, b] = [page.rgba[at], page.rgba[at + 1], page.rgba[at + 2]].map(i32::from);
            if wanted([r, g, b]) {
                found.push((x, y));
            }
        }
    }
    found
}

#[test]
fn a_text_object_that_clips_and_then_draws_keeps_its_clip() {
    // The glyphs drawn in text rendering modes 4 to 7 become a clip at ET. MuPDF 1.28 decided at
    // ET from the last mode whether to clip, so a text object that clipped Hello (mode 7) and then
    // drew World (mode 0) lost its clip, and Hello was not extracted (#638). The build-time patch
    // in mupdf-sys applies the clip whatever the last mode, and extracts every part of a text
    // object in the order it is shown.
    let path = "benign/text-clip-mode-switch.pdf";
    let text = page_text(path, 0);
    assert!(lines(&text).contains(&"Hello World"), "{:?}", lines(&text));
    let (mut host, doc) = open(path);
    assert_eq!(search_hits(&mut host, doc, "Hello"), 1);

    // After ET the page is filled with blue, which shows only through Hello's glyphs (24 pt at
    // x 72, baseline 192 pixels from the top at scale 1). World, after it, stays black.
    let page = PdfDocument::from_bytes(&std::fs::read(corpus(path)).unwrap())
        .unwrap()
        .render(0, 1.0, 0)
        .unwrap();
    let whole = [0, 0, page.width as usize, page.height as usize];
    let blue = pixels(&page, whole, |[r, g, b]| b > 200 && r < 80 && g < 80);
    assert!(blue.len() > 20, "{} blue pixels", blue.len());
    let outside_hello: Vec<_> = blue
        .iter()
        .filter(|&&(x, y)| !(70..130).contains(&x) || !(170..196).contains(&y))
        .collect();
    assert!(
        outside_hello.is_empty(),
        "{} of {} blue pixels outside Hello, the first {:?}",
        outside_hello.len(),
        blue.len(),
        outside_hello.first()
    );
    let world = pixels(&page, [133, 170, 200, 196], |[r, g, b]| r + g + b < 120);
    assert!(world.len() > 20, "{} black pixels in World", world.len());
}

#[test]
fn a_clipped_glyph_that_stands_for_two_letters_gives_both() {
    // A ToUnicode can map one glyph to more than one character, here the fi ligature to f and i.
    // MuPDF 1.28 extracted only the f of such a glyph in text rendering mode 7, which only clips
    // (#678); the build-time patch in mupdf-sys takes mode 7 text from the copy that has them all.
    let path = "benign/text-clip-ligature.pdf";
    let text = page_text(path, 0);
    let all = lines(&text).join("\n");
    for word in ["Office0", "Office4", "Office7"] {
        assert_eq!(all.matches(word).count(), 1, "{word} in {all:?}");
    }
    let (mut host, doc) = open(path);
    assert_eq!(search_hits(&mut host, doc, "Office7"), 1);
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
