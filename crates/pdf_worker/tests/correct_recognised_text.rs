//! Correcting existing invisible OCR text layers (`3 Tr`, B7-12, Issue #749, ADR 0036)
//! through the real sandboxed worker (`WorkerHost`) and engine.
#![cfg(windows)]

#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::types::{
    AnnotationKind, DocumentId, LinkTarget, PageAnnotation, PageLink, PageText, Password,
    RecognisedWord, RecognisedWordsPage, RecognisedWordsUnusable, Restrictions, Rotation,
    SearchHit,
};
use ipc_contract::worker::{
    OcrOutcome, OcrPageState, Raster, WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse,
};
use mupdf::pdf::{
    InsertImageOptions, PageImageSource, PdfDocument as MuPdfDocument, PdfPage, TextFilterAction,
    TextFilterOptions,
};
use mupdf::{Colorspace, Document, Matrix, TextPageFlags};
use worker_host::{HostConfig, HostError, WorkerHost};

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

fn new_host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn get_recognised_words(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Result<RecognisedWordsPage, HostError> {
    match host.request(|request| WorkerRequest::GetRecognisedWords {
        request,
        doc,
        page_index,
    })? {
        WorkerResponse::RecognisedWords {
            page_index: answered,
            result,
            ..
        } => {
            assert_eq!(answered, page_index);
            Ok(result)
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

fn expect_usable_words(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Vec<RecognisedWord> {
    match get_recognised_words(host, doc, page_index).expect("GetRecognisedWords") {
        RecognisedWordsPage::Usable { words } => words,
        RecognisedWordsPage::Unusable { reason } => {
            panic!("expected usable words on page {page_index}, got Unusable({reason:?})")
        }
    }
}

fn apply_worker_edit(
    host: &mut WorkerHost,
    doc: DocumentId,
    edit: WorkerEdit,
) -> Result<(), HostError> {
    match host.request(|request| WorkerRequest::Edit { request, doc, edit })? {
        WorkerResponse::Edited { .. } => Ok(()),
        other => panic!("unexpected response: {other:?}"),
    }
}

fn render_page(host: &mut WorkerHost, doc: DocumentId, page_index: u32, scale: f32) -> Raster {
    match host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index,
            scale,
            rotation: Rotation::None,
        })
        .expect("render")
    {
        WorkerResponse::Rendered { raster, .. } => raster,
        other => panic!("unexpected render response: {other:?}"),
    }
}

fn page_text(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> PageText {
    match host
        .request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index,
        })
        .expect("page text")
    {
        WorkerResponse::PageText { text, .. } => text,
        other => panic!("unexpected page text response: {other:?}"),
    }
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
        other => panic!("unexpected search response: {other:?}"),
    }
}

fn page_links(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> Vec<PageLink> {
    match host
        .request(|request| WorkerRequest::GetPageLinks {
            request,
            doc,
            page_index,
        })
        .expect("links")
    {
        WorkerResponse::PageLinks { links, .. } => links,
        other => panic!("unexpected links response: {other:?}"),
    }
}

fn page_annotations(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Vec<PageAnnotation> {
    match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index,
        })
        .expect("annotations")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("unexpected annotations response: {other:?}"),
    }
}

fn save_temp(host: &mut WorkerHost, doc: DocumentId, tag: &str) -> common::TempFileGuard {
    let guard = common::temp_pdf(tag, &[]);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&*guard)
        .expect("open output file");
    match host.save(doc, &file).expect("save") {
        WorkerResponse::Saved { .. } => {}
        other => panic!("unexpected save response: {other:?}"),
    }
    drop(file);
    common::qpdf::check_qpdf(&guard);
    guard
}

fn scanned_pdf(names: &[&str]) -> Vec<u8> {
    let mut scanned = MuPdfDocument::new();
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

fn compact_pdf_ascii(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if !b.is_ascii_whitespace() {
            out.push(if b.is_ascii() { b as char } else { '.' });
        }
    }
    out
}

#[test]
fn corpus_sample_lists_only_tr3_words_and_passes_qpdf() {
    let path = corpus("benign/recognised-text-layer.pdf");
    common::qpdf::check_qpdf(&path);

    let mut host = new_host();
    let doc = host.open(&path).expect("open corpus sample").0;
    let words = expect_usable_words(&mut host, doc, 0);
    let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, vec!["Hel1o", "W0rld"]);
    assert!(!texts.contains(&"VisibleKeep"));
    assert!(!texts.contains(&"ClipKeep"));
    assert!(!texts.contains(&"AlphaZeroKeep"));
    assert!(!texts.contains(&"Recognised"));
}

#[test]
fn correct_recognised_text_updates_search_keeps_render_and_preserves_negative_controls() {
    let mut host = new_host();
    let doc = host
        .open(&corpus("benign/recognised-text-layer.pdf"))
        .expect("open")
        .0;

    let before_1x = render_page(&mut host, doc, 0, 1.0);
    let before_2x = render_page(&mut host, doc, 0, 2.0);
    let before_links = page_links(&mut host, doc, 0);
    let before_annots = page_annotations(&mut host, doc, 0);
    assert!(before_links.iter().any(|l| matches!(
        &l.target,
        LinkTarget::Uri { uri } if uri == "https://example.invalid/recognised"
    )));
    assert!(before_annots.iter().any(|a| {
        a.kind == AnnotationKind::Note && a.text.as_deref() == Some("Keep this note")
    }));

    let words = expect_usable_words(&mut host, doc, 0);
    let target = words
        .iter()
        .find(|w| w.text == "Hel1o")
        .expect("Hel1o word");

    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: target.id,
            expected: "Hel1o".to_owned(),
            text: "Hello".to_owned(),
        },
    )
    .expect("correct Hel1o to Hello");

    // Re-save and reopen in a clean worker.
    let saved = save_temp(&mut host, doc, "recognised-edited");
    let mut reader = new_host();
    let reopened = reader.open(&saved).expect("reopen edited").0;

    let (hello_hits, _) = search_page(&mut reader, reopened, 0, "Hello");
    assert!(!hello_hits.is_empty(), "new word must be searchable");
    let (old_hits, _) = search_page(&mut reader, reopened, 0, "Hel1o");
    assert!(old_hits.is_empty(), "old word must no longer match");

    // Pixel-identical rendering before and after.
    assert_eq!(before_1x, render_page(&mut reader, reopened, 0, 1.0));
    assert_eq!(before_2x, render_page(&mut reader, reopened, 0, 2.0));

    // Negative controls: same-box 0 Tr, 7 Tr clip, alpha-0 text, link and annotation remain.
    for kept in ["W0rld", "VisibleKeep", "ClipKeep", "AlphaZeroKeep"] {
        assert!(
            !search_page(&mut reader, reopened, 0, kept).0.is_empty(),
            "negative control '{kept}' must remain searchable"
        );
    }
    let after_links = page_links(&mut reader, reopened, 0);
    assert_eq!(after_links.len(), before_links.len());
    assert!(after_links.iter().any(|l| matches!(
        &l.target,
        LinkTarget::Uri { uri } if uri == "https://example.invalid/recognised"
    )));
    let after_annots = page_annotations(&mut reader, reopened, 0);
    assert_eq!(after_annots.len(), before_annots.len());
    assert!(after_annots.iter().any(|a| {
        a.kind == AnnotationKind::Note && a.text.as_deref() == Some("Keep this note")
    }));
}

#[test]
fn control_using_is_invisible_removes_alpha_zero_text() {
    let bytes = std::fs::read(corpus("benign/recognised-text-layer.pdf")).expect("read corpus");
    let doc = MuPdfDocument::from_bytes(&bytes).expect("open MuPDF");
    let mut page = PdfPage::try_from(doc.load_page(0).expect("page")).expect("pdf page");
    page.filter_text(TextFilterOptions::default(), |glyph| {
        if glyph.is_invisible() {
            TextFilterAction::Remove
        } else {
            TextFilterAction::Keep
        }
    })
    .expect("filter_text");
    let extracted = page
        .to_text_page(TextPageFlags::empty())
        .expect("stext")
        .to_text()
        .expect("to_text");
    assert!(
        !extracted.contains("AlphaZeroKeep"),
        "is_invisible() mistakenly removes alpha-0 text; got: {extracted}"
    );
    assert!(extracted.contains("VisibleKeep"));
}

#[test]
fn search_and_page_text_reflect_correction_immediately_before_save() {
    let mut host = new_host();
    let doc = host
        .open(&corpus("benign/recognised-text-layer.pdf"))
        .expect("open")
        .0;

    // Warm caches before edit.
    assert!(search_page(&mut host, doc, 0, "Hello").0.is_empty());
    let before_text = page_text(&mut host, doc, 0);
    assert!(before_text.lines.iter().any(|l| l.text.contains("Hel1o")));

    let words = expect_usable_words(&mut host, doc, 0);
    let target = words.iter().find(|w| w.text == "Hel1o").unwrap();
    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: target.id,
            expected: "Hel1o".to_owned(),
            text: "Hello".to_owned(),
        },
    )
    .expect("apply edit");

    // Without saving, search and page_text see "Hello" and not "Hel1o".
    assert!(!search_page(&mut host, doc, 0, "Hello").0.is_empty());
    assert!(search_page(&mut host, doc, 0, "Hel1o").0.is_empty());
    let after_text = page_text(&mut host, doc, 0);
    assert!(after_text.lines.iter().any(|l| l.text.contains("Hello")));
    assert!(!after_text.lines.iter().any(|l| l.text.contains("Hel1o")));
}

#[test]
fn refuses_mismatch_signed_not_allowed_no_tr3_memory_ocr_and_mixed_render_modes() {
    // 1. Mismatching expected text leaves page unchanged.
    {
        let mut host = new_host();
        let doc = host
            .open(&corpus("benign/recognised-text-layer.pdf"))
            .expect("open")
            .0;
        let before = render_page(&mut host, doc, 0, 1.0);
        let err = apply_worker_edit(
            &mut host,
            doc,
            WorkerEdit::CorrectRecognisedText {
                page: 0,
                word: 0,
                expected: "NotTheActualWord".to_owned(),
                text: "Changed".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            HostError::Worker(e) if e.code == WorkerErrorCode::InvalidRequest
        ));
        assert!(!search_page(&mut host, doc, 0, "Hel1o").0.is_empty());
        assert_eq!(before, render_page(&mut host, doc, 0, 1.0));
    }

    // 2. Signed document (corpus signed.pdf + synthetic signed doc with 3 Tr text).
    {
        let mut host = new_host();
        let signed_corpus = host.open(&corpus("benign/signed.pdf")).expect("signed").0;
        assert_eq!(
            get_recognised_words(&mut host, signed_corpus, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::Signed
            }
        );

        let pdf = common::build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /SigFlags 3 >> >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>".into(),
            "<< /Length 42 >>\nstream\nBT /F1 16 Tf 72 700 Td 3 Tr (SignedOcr) Tj ET\nendstream".into(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sig1) /Rect [0 0 0 0] /P 3 0 R /V << /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] /Contents <00> >> >>".into(),
        ]);
        let guard = common::temp_pdf("signed-tr3", &pdf);
        let doc = host.open(&guard).expect("open signed tr3").0;
        assert_eq!(
            get_recognised_words(&mut host, doc, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::Signed
            }
        );
        let err = apply_worker_edit(
            &mut host,
            doc,
            WorkerEdit::CorrectRecognisedText {
                page: 0,
                word: 0,
                expected: "SignedOcr".to_owned(),
                text: "Tampered".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            HostError::Worker(e) if e.code == WorkerErrorCode::Signed
        ));
        assert!(!search_page(&mut host, doc, 0, "SignedOcr").0.is_empty());
    }

    // 3. /P permissions forbidding modification.
    {
        let mut host = new_host();
        let nomod = host
            .open(&corpus("benign/restricted-no-modify.pdf"))
            .expect("open restricted")
            .0;
        assert_eq!(
            get_recognised_words(&mut host, nomod, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::NotAllowed
            }
        );

        let src = host
            .open(&corpus("benign/recognised-text-layer.pdf"))
            .expect("open")
            .0;
        let enc_guard = common::temp_pdf("restricted-tr3", &[]);
        let enc_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&*enc_guard)
            .unwrap();
        let resp = host
            .encrypted_copy(
                src,
                &enc_file,
                None,
                Password::new("owner".to_owned()),
                Restrictions {
                    print: false,
                    copy: false,
                    modify: true,
                },
            )
            .expect("encrypted copy");
        assert!(matches!(resp, WorkerResponse::Saved { .. }));
        drop(enc_file);

        let locked = host.open(&enc_guard).expect("open encrypted copy").0;
        assert_eq!(
            get_recognised_words(&mut host, locked, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::NotAllowed
            }
        );
        let err = apply_worker_edit(
            &mut host,
            locked,
            WorkerEdit::CorrectRecognisedText {
                page: 0,
                word: 0,
                expected: "Hel1o".to_owned(),
                text: "Hello".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            HostError::Worker(e) if e.code == WorkerErrorCode::NotAllowed
        ));
        assert!(!search_page(&mut host, locked, 0, "Hel1o").0.is_empty());
    }

    // 4. Page with only visible 0 Tr text -> NoTextLayer.
    {
        let mut host = new_host();
        let doc = host
            .open(&corpus("benign/single-page.pdf"))
            .expect("open")
            .0;
        assert_eq!(
            get_recognised_words(&mut host, doc, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::NoTextLayer
            }
        );
    }

    // 5. In-memory OCR result before saving searchable PDF -> MemoryOcrOnly.
    {
        let guard = common::temp_pdf("memory-ocr-only", &scanned_pdf(&["benign/single-page.pdf"]));
        let mut host = new_host();
        let doc = host.open(&guard).expect("open scan").0;
        let load = host
            .request(|request| WorkerRequest::OcrLoad {
                request,
                language: "eng".to_owned(),
                data: traineddata("eng"),
            })
            .expect("load eng");
        assert!(matches!(load, WorkerResponse::OcrLoaded { .. }));
        let checked = host
            .request(|request| WorkerRequest::OcrPage {
                request,
                doc,
                page_index: 0,
                max_millis: 120_000,
            })
            .expect("ocr check");
        assert!(matches!(
            checked,
            WorkerResponse::OcrChecked {
                state: OcrPageState::Queued,
                ..
            }
        ));
        let started = Instant::now();
        let mut finished = false;
        while !finished && started.elapsed() < Duration::from_secs(120) {
            if let WorkerResponse::OcrPolled { finished: f, .. } = host
                .request(|request| WorkerRequest::OcrPoll { request })
                .unwrap()
                && f.iter()
                    .any(|item| matches!(item.outcome, OcrOutcome::Recognised { .. }))
            {
                finished = true;
            }
            if !finished {
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        assert!(finished, "OCR must complete within timeout");
        assert_eq!(
            get_recognised_words(&mut host, doc, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::MemoryOcrOnly
            }
        );
    }

    // 6. Mixed 3 Tr and 4..=7 Tr in same continuous word segment -> MixedRenderModes.
    {
        let content = "q BT /F1 14 Tf 72 700 Td 3 Tr (Part) Tj 7 Tr (Clip) Tj ET 0.5 g 72 694 120 20 re f Q\n";
        let pdf = common::build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ]);
        let guard = common::temp_pdf("mixed-tr3-tr7", &pdf);
        let mut host = new_host();
        let doc = host.open(&guard).expect("open mixed").0;
        assert_eq!(
            get_recognised_words(&mut host, doc, 0).unwrap(),
            RecognisedWordsPage::Unusable {
                reason: RecognisedWordsUnusable::MixedRenderModes
            }
        );
        let err = apply_worker_edit(
            &mut host,
            doc,
            WorkerEdit::CorrectRecognisedText {
                page: 0,
                word: 0,
                expected: "Part".to_owned(),
                text: "Other".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            HostError::Worker(e) if e.code == WorkerErrorCode::InvalidRequest
        ));
    }
}

#[test]
fn reuses_single_glyphless_font_object_across_pages_and_searchable_copies() {
    // Part A: 2-page Helvetica 3 Tr PDF -> adds only ONE GlyphLessFont shared across both pages.
    let c0 = "BT /F1 16 Tf 72 700 Td 3 Tr (ErrZero) Tj ET\n";
    let c1 = "BT /F1 16 Tf 72 700 Td 3 Tr (ErrOne) Tj ET\n";
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{c0}\nendstream", c0.len()),
        format!("<< /Length {} >>\nstream\n{c1}\nendstream", c1.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ]);
    let guard = common::temp_pdf("multi-page-tr3", &pdf);
    let mut host = new_host();
    let doc = host.open(&guard).expect("open").0;
    let w0 = expect_usable_words(&mut host, doc, 0);
    let w1 = expect_usable_words(&mut host, doc, 1);
    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: w0[0].id,
            expected: "ErrZero".to_owned(),
            text: "FixZero".to_owned(),
        },
    )
    .expect("edit page 0");
    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 1,
            word: w1[0].id,
            expected: "ErrOne".to_owned(),
            text: "FixOne".to_owned(),
        },
    )
    .expect("edit page 1");
    let saved = save_temp(&mut host, doc, "shared-font-out");
    let bytes = std::fs::read(&*saved).expect("read saved");
    let compact = compact_pdf_ascii(&bytes);
    assert_eq!(
        compact.matches("/FontName/GlyphLessFont").count(),
        1,
        "must embed GlyphLessFont descriptor at most once across pages"
    );

    // Part B: Existing searchable_copy already contains GlyphLessFont -> reuses it (still count 1).
    let scan_guard = common::temp_pdf(
        "scan-searchable-reuse",
        &scanned_pdf(&["benign/single-page.pdf", "benign/single-page.pdf"]),
    );
    let mut ocr_host = new_host();
    let scan_doc = ocr_host.open(&scan_guard).expect("open scan").0;
    let _ = ocr_host
        .request(|request| WorkerRequest::OcrLoad {
            request,
            language: "eng".to_owned(),
            data: traineddata("eng"),
        })
        .unwrap();
    for p in 0..2 {
        let _ = ocr_host
            .request(|request| WorkerRequest::OcrPage {
                request,
                doc: scan_doc,
                page_index: p,
                max_millis: 120_000,
            })
            .unwrap();
    }
    let started = Instant::now();
    let mut count_done = 0;
    while count_done < 2 && started.elapsed() < Duration::from_secs(120) {
        if let WorkerResponse::OcrPolled { finished: f, .. } = ocr_host
            .request(|request| WorkerRequest::OcrPoll { request })
            .unwrap()
        {
            count_done += f.len();
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(count_done, 2);

    let sc_guard = common::temp_pdf("searchable-copy-reuse", &[]);
    let sc_file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&*sc_guard)
        .unwrap();
    assert!(matches!(
        ocr_host.searchable_copy(scan_doc, &sc_file).unwrap(),
        WorkerResponse::Searchable { added_pages: 2, .. }
    ));
    drop(sc_file);

    let mut edit_host = new_host();
    let sc_doc = edit_host.open(&sc_guard).expect("open searchable copy").0;
    let words0 = expect_usable_words(&mut edit_host, sc_doc, 0);
    assert!(!words0.is_empty());
    let first = &words0[0];
    apply_worker_edit(
        &mut edit_host,
        sc_doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: first.id,
            expected: first.text.clone(),
            text: "CorrectedToken".to_owned(),
        },
    )
    .expect("edit searchable copy");
    let final_guard = save_temp(&mut edit_host, sc_doc, "searchable-copy-edited");
    let final_bytes = std::fs::read(&*final_guard).expect("read final");
    let final_compact = compact_pdf_ascii(&final_bytes);
    assert_eq!(
        final_compact.matches("/FontName/GlyphLessFont").count(),
        1,
        "must reuse existing GlyphLessFont without duplicating font objects"
    );
    assert!(
        !search_page(&mut edit_host, sc_doc, 0, "CorrectedToken")
            .0
            .is_empty()
    );
}

#[test]
fn rotated_90_page_with_mediabox_offset_places_corrected_text_at_matching_box() {
    let content = "BT /F1 16 Tf 200 200 Td 3 Tr (Wr0ngRot) Tj ET\n";
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [100 50 712 842] /Rotate 90 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ]);
    let guard = common::temp_pdf("rotated-offset", &pdf);
    let mut host = new_host();
    let doc = host.open(&guard).expect("open").0;

    let before = render_page(&mut host, doc, 0, 1.0);
    let words = expect_usable_words(&mut host, doc, 0);
    assert_eq!(words.len(), 1);
    let orig = &words[0];
    assert_eq!(orig.text, "Wr0ngRot");
    let orig_rect = orig.rect;

    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: orig.id,
            expected: "Wr0ngRot".to_owned(),
            text: "RightRot".to_owned(),
        },
    )
    .expect("edit rotated page");

    let saved = save_temp(&mut host, doc, "rotated-offset-saved");
    let mut reader = new_host();
    let reopened = reader.open(&saved).expect("reopen").0;
    assert_eq!(before, render_page(&mut reader, reopened, 0, 1.0));

    let (hits, _) = search_page(&mut reader, reopened, 0, "RightRot");
    assert_eq!(hits.len(), 1);
    assert!(
        search_page(&mut reader, reopened, 0, "Wr0ngRot")
            .0
            .is_empty()
    );

    let hit_quad = hits[0].quads[0];
    let hit_x0 = hit_quad.ul.x.min(hit_quad.ll.x);
    let hit_y0 = hit_quad.ul.y.min(hit_quad.ur.y);
    let hit_x1 = hit_quad.ur.x.max(hit_quad.lr.x);
    let hit_y1 = hit_quad.ll.y.max(hit_quad.lr.y);
    // Replacement quad must overlap the original bounding box tightly on the rotated/offset page.
    let overlap_x = (hit_x1.min(orig_rect.x1) - hit_x0.max(orig_rect.x0)).max(0.0);
    let overlap_y = (hit_y1.min(orig_rect.y1) - hit_y0.max(orig_rect.y0)).max(0.0);
    assert!(
        overlap_x > (orig_rect.x1 - orig_rect.x0) * 0.5 && overlap_y > 0.0,
        "rotated replacement box ({hit_x0},{hit_y0},{hit_x1},{hit_y1}) did not match original {orig_rect:?}"
    );
}

#[test]
fn form_xobject_tr3_edit_isolates_target_page_without_affecting_other_page() {
    let form_stream = "BT /F1 16 Tf 72 700 Td 3 Tr (SharedOcr) Tj ET\n";
    let page_stream = "q /Fm0 Do Q\n";
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /XObject << /Fm0 6 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /XObject << /Fm0 6 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{page_stream}\nendstream", page_stream.len()),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Length {} >>\nstream\n{form_stream}\nendstream",
            form_stream.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ]);
    let guard = common::temp_pdf("form-xobject-shared", &pdf);
    let mut host = new_host();
    let doc = host.open(&guard).expect("open").0;

    let words_p0 = expect_usable_words(&mut host, doc, 0);
    assert_eq!(words_p0.len(), 1);
    assert_eq!(words_p0[0].text, "SharedOcr");
    let words_p1 = expect_usable_words(&mut host, doc, 1);
    assert_eq!(words_p1.len(), 1);

    let before_p0 = render_page(&mut host, doc, 0, 1.0);
    apply_worker_edit(
        &mut host,
        doc,
        WorkerEdit::CorrectRecognisedText {
            page: 0,
            word: words_p0[0].id,
            expected: "SharedOcr".to_owned(),
            text: "FixedPageZero".to_owned(),
        },
    )
    .expect("edit form xobject on page 0");

    let saved = save_temp(&mut host, doc, "form-xobject-saved");
    let mut reader = new_host();
    let reopened = reader.open(&saved).expect("reopen").0;
    assert_eq!(before_p0, render_page(&mut reader, reopened, 0, 1.0));
    assert!(
        !search_page(&mut reader, reopened, 0, "FixedPageZero")
            .0
            .is_empty()
    );
    assert!(
        search_page(&mut reader, reopened, 0, "SharedOcr")
            .0
            .is_empty()
    );
    assert!(
        !search_page(&mut reader, reopened, 1, "SharedOcr")
            .0
            .is_empty()
    );
    assert!(
        search_page(&mut reader, reopened, 1, "FixedPageZero")
            .0
            .is_empty()
    );
}

#[test]
fn undo_and_redo_restore_original_and_corrected_tr3_text() {
    let mut host = new_host();
    let doc = host
        .open(&corpus("benign/recognised-text-layer.pdf"))
        .expect("open")
        .0;
    let words = expect_usable_words(&mut host, doc, 0);
    let target = words.iter().find(|w| w.text == "Hel1o").unwrap();
    let edit = WorkerEdit::CorrectRecognisedText {
        page: 0,
        word: target.id,
        expected: "Hel1o".to_owned(),
        text: "Hello".to_owned(),
    };

    apply_worker_edit(&mut host, doc, edit.clone()).expect("apply edit");
    assert!(!search_page(&mut host, doc, 0, "Hello").0.is_empty());
    assert!(search_page(&mut host, doc, 0, "Hel1o").0.is_empty());

    // Undo via Revert with empty edits.
    let undo_resp = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: Vec::new(),
            password: None,
        })
        .expect("undo revert");
    assert!(matches!(undo_resp, WorkerResponse::Edited { .. }));
    assert!(!search_page(&mut host, doc, 0, "Hel1o").0.is_empty());
    assert!(search_page(&mut host, doc, 0, "Hello").0.is_empty());

    // Redo via Revert replaying the edit.
    let redo_resp = host
        .request(|request| WorkerRequest::Revert {
            request,
            doc,
            edits: vec![edit],
            password: None,
        })
        .expect("redo revert");
    assert!(matches!(redo_resp, WorkerResponse::Edited { .. }));
    assert!(!search_page(&mut host, doc, 0, "Hello").0.is_empty());
    assert!(search_page(&mut host, doc, 0, "Hel1o").0.is_empty());
}
