//! Tests for document properties and font discovery (B5-04).

#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use common::build_pdf;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use pdf_worker::engine::PdfDocument;
use worker_host::{HostConfig, WorkerHost};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

#[test]
fn test_corpus_metadata_full() {
    let bytes = std::fs::read(corpus("benign/metadata-full.pdf")).expect("read file");
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let props = doc.document_properties().expect("properties");

    assert!(props.format.is_some());
    assert_eq!(props.title.as_deref(), Some("Private title"));
    assert_eq!(props.author.as_deref(), Some("Jane Q. Private-Author"));
    assert_eq!(props.subject.as_deref(), Some("Private subject"));
    assert_eq!(props.keywords.as_deref(), Some("private, keywords"));
    assert_eq!(
        props.creator.as_deref(),
        Some("SecretWriter 9.1 on Windows 10.0.19045")
    );
    assert!(props.creation_date.is_some());
    let cdate = props.creation_date.unwrap();
    assert_eq!(cdate.year, 2026);
    assert!(props.has_xmp);
    assert_eq!(props.signatures_count, 0);
    assert_eq!(props.attachment_count, 0);
}

#[test]
fn test_corpus_signed() {
    let bytes = std::fs::read(corpus("benign/signed.pdf")).expect("read file");
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let props = doc.document_properties().expect("properties");

    assert!(props.signatures_count >= 1);
    assert!(!props.signatures_truncated);
}

#[test]
fn test_corpus_attachment_text() {
    let bytes = std::fs::read(corpus("benign/attachment-text.pdf")).expect("read file");
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let props = doc.document_properties().expect("properties");

    assert!(props.attachment_count >= 1);
}

#[test]
fn test_corpus_encrypted_aes256() {
    let bytes = std::fs::read(corpus("benign/encrypted-aes256.pdf")).expect("read file");
    let doc = PdfDocument::open(&bytes, Some("user")).expect("open doc");
    let props = doc.document_properties().expect("properties");

    assert!(props.encryption_method.is_some());
}

#[test]
fn test_corpus_mixed_text_zh_en_fonts() {
    let bytes = std::fs::read(corpus("benign/mixed-text-zh-en.pdf")).expect("read file");
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let fonts = doc.document_fonts().expect("fonts");

    assert!(fonts.complete);
    assert!(!fonts.fonts.is_empty());
    // Verify sorted case-insensitively
    for window in fonts.fonts.windows(2) {
        assert!(window[0].name.to_lowercase() <= window[1].name.to_lowercase());
    }
}

#[test]
fn test_corpus_single_page() {
    let bytes = std::fs::read(corpus("benign/single-page.pdf")).expect("read file");
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let props = doc.document_properties().expect("properties");
    assert!(props.format.is_some());

    let fonts = doc.document_fonts().expect("fonts");
    assert!(fonts.complete);
}

#[test]
fn stress_form_xobject_self_reference() {
    let start = Instant::now();
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /F1 4 0 R >> >> >>".to_owned(),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /F1 4 0 R >> >> /Length 0 >>\nstream\nendstream".to_owned(),
    ];
    let bytes = build_pdf(&objects);
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let fonts = doc.document_fonts().expect("fonts");
    let elapsed = start.elapsed();
    println!("stress_form_xobject_self_reference: {:?}", elapsed);
    assert!(elapsed.as_millis() < 1000, "took too long: {:?}", elapsed);
    assert!(fonts.complete);
}

#[test]
fn stress_form_xobjects_mutual_reference() {
    let start = Instant::now();
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /F1 4 0 R >> >> >>".to_owned(),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /F2 5 0 R >> >> /Length 0 >>\nstream\nendstream".to_owned(),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /F1 4 0 R >> >> /Length 0 >>\nstream\nendstream".to_owned(),
    ];
    let bytes = build_pdf(&objects);
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let fonts = doc.document_fonts().expect("fonts");
    let elapsed = start.elapsed();
    println!("stress_form_xobjects_mutual_reference: {:?}", elapsed);
    assert!(elapsed.as_millis() < 1000, "took too long: {:?}", elapsed);
    assert!(fonts.complete);
}

#[test]
fn stress_5000_pages_sharing_same_resources() {
    let start = Instant::now();
    let n_pages = 5000;
    let mut objects = Vec::with_capacity(n_pages + 4);
    // Obj 1: Catalog
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_owned());
    // Obj 2: Pages
    let kids = (5..=(n_pages + 4))
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Kids [{kids}] /Count {n_pages} /Resources 3 0 R >>"
    ));
    // Obj 3: Shared Resources
    objects.push("<< /Font << /F1 4 0 R >> >>".to_owned());
    // Obj 4: Font
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    // Obj 5..: Pages
    for _ in 0..n_pages {
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned());
    }

    let bytes = build_pdf(&objects);
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let fonts = doc.document_fonts().expect("fonts");
    let elapsed = start.elapsed();
    println!("stress_5000_pages_sharing_same_resources: {:?}", elapsed);
    assert!(elapsed.as_millis() < 1000, "took too long: {:?}", elapsed);
    assert!(fonts.complete);
    assert_eq!(fonts.fonts.len(), 1);
    assert_eq!(fonts.fonts[0].name, "Helvetica");
    assert_eq!(fonts.fonts[0].subtype, "Type 1");
    assert!(!fonts.fonts[0].embedded);
}

#[test]
fn test_parent_cycle_in_page_tree() {
    let start = Instant::now();
    // Obj 3 has /Parent 3 0 R (direct cycle)
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 612 792] >>".to_owned(),
    ];
    let bytes = build_pdf(&objects);
    if let Ok(doc) = PdfDocument::from_bytes(&bytes) {
        let fonts = doc.document_fonts().expect("fonts");
        assert!(!fonts.complete, "should report incomplete due to cycle");
    }
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 1000, "took too long: {:?}", elapsed);
}

#[test]
fn test_font_subset_prefix_stripping() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> >>".to_owned(),
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+TimesNewRoman >>".to_owned(),
    ];
    let bytes = build_pdf(&objects);
    let doc = PdfDocument::from_bytes(&bytes).expect("open doc");
    let fonts = doc.document_fonts().expect("fonts");
    assert_eq!(fonts.fonts.len(), 1);
    assert_eq!(fonts.fonts[0].name, "TimesNewRoman");
    assert_eq!(fonts.fonts[0].subtype, "TrueType");
}

#[test]
fn test_ipc_worker_host_properties_and_fonts() {
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let (doc, _) = host.open(&corpus("benign/metadata-full.pdf")).expect("open");
    let resp = host
        .request(|request| WorkerRequest::GetDocumentProperties { request, doc })
        .expect("request");
    match resp {
        WorkerResponse::DocumentProperties { properties, .. } => {
            assert_eq!(properties.title.as_deref(), Some("Private title"));
        }
        other => panic!("expected DocumentProperties, got {other:?}"),
    }

    let resp_fonts = host
        .request(|request| WorkerRequest::GetDocumentFonts { request, doc })
        .expect("request");
    match resp_fonts {
        WorkerResponse::DocumentFonts { fonts, .. } => {
            assert!(fonts.complete);
        }
        other => panic!("expected DocumentFonts, got {other:?}"),
    }
}
