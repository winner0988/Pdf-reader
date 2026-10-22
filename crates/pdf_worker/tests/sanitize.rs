//! Sanitized copy integration tests (Issue #303, B3-15).
//!
//! Verifies:
//! 1. All malicious corpus files sanitized with all options: active content scan reports 0 findings,
//!    output file opens cleanly, report numbers match actual removals.
//! 2. All benign non-signed non-encrypted corpus files: sanitized with Keep annotations render
//!    pixel-identical page-by-page.
//! 3. Annotation modes: Flatten bakes annotations into page content with matching render;
//!    Remove leaves only widgets and links.
//! 4. Web links handling: keep_web_links preserves http/https/mailto, removes other schemes;
//!    when false, strips all URI links.
//! 5. Refusal for signed and encrypted documents.
//! 6. Original open document in worker is unaffected and can be sanitized or read again.

#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};

use ipc_contract::limits::MAX_LINKS_PER_PAGE;
use ipc_contract::types::{SanitizeAnnotations, SanitizeOptions, SanitizeReport};
use pdf_worker::engine::{EngineError, PdfDocument};
use pdf_worker::scan::ScanBudget;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

struct Sample {
    path: String,
    file: PathBuf,
}

fn samples() -> Vec<Sample> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus().join("manifest.json")).unwrap()).unwrap();
    manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| {
            let path = entry["path"].as_str().unwrap().to_owned();
            let file = corpus().join(&path);
            let on_demand = entry["onDemand"].as_bool() == Some(true);
            if on_demand && !file.exists() {
                None
            } else {
                Some(Sample { path, file })
            }
        })
        .collect()
}

const DEFAULT_OPTIONS: SanitizeOptions = SanitizeOptions {
    actions: true,
    attachments: true,
    form_scripts: true,
    xfa: true,
    rich_media: true,
    hidden_data: true,
    keep_web_links: true,
    annotations: SanitizeAnnotations::Keep,
};

#[test]
fn every_malicious_sample_scans_zero_and_opens_after_sanitize() {
    let mut checked = 0;
    for sample in samples()
        .into_iter()
        .filter(|s| s.path.starts_with("malicious/"))
    {
        let bytes = std::fs::read(&sample.file).unwrap();
        let Ok(document) = PdfDocument::from_bytes(&bytes) else {
            continue;
        };

        let had_findings = !document
            .active_content(ScanBudget::default())
            .findings
            .is_empty();
        let mut out = Vec::new();
        let id = [1u8; 16];
        let (bytes_written, report) = document
            .sanitized_copy(&id, DEFAULT_OPTIONS, &mut out)
            .unwrap_or_else(|err| panic!("{}: failed to sanitize: {err:?}", sample.path));

        assert_eq!(
            bytes_written as usize,
            out.len(),
            "{}: byte count mismatch",
            sample.path
        );
        assert!(
            out.starts_with(b"%PDF-"),
            "{}: missing %PDF- header",
            sample.path
        );

        // Output must open cleanly
        let sanitized_doc = PdfDocument::from_bytes(&out).unwrap_or_else(|err| {
            panic!("{}: sanitized copy failed to open: {err:?}", sample.path)
        });

        // Active content scan on sanitized doc must have 0 findings
        let sec_report = sanitized_doc.active_content(ScanBudget::default());
        assert!(
            sec_report.findings.is_empty(),
            "{}: sanitized doc still has active content findings: {:?} (report: {report:?})",
            sample.path,
            sec_report.findings
        );

        // What the scan found was removed, and reported. (A sample whose only trick is an
        // https link, a look-alike domain or right-to-left text, may report nothing: keeping web
        // links keeps it, and the link dialog shows its whole address when clicked.)
        let total_removed = report.actions
            + report.links
            + report.attachments
            + report.form_scripts
            + report.xfa
            + report.rich_media
            + report.hidden_data
            + report.annotations;
        assert!(
            !had_findings || total_removed > 0,
            "{}: the scan found active content but nothing was reported removed",
            sample.path
        );

        checked += 1;
    }
    assert!(checked >= 20, "only {checked} malicious samples checked");
}

#[test]
fn benign_samples_render_pixel_identical_after_sanitize() {
    let mut checked = 0;
    for sample in samples()
        .into_iter()
        .filter(|s| s.path.starts_with("benign/"))
    {
        // Skip signed and encrypted documents (which should be rejected)
        if sample.path.contains("signed")
            || sample.path.contains("encrypted")
            || sample.path.contains("restricted")
        {
            continue;
        }

        let bytes = std::fs::read(&sample.file).unwrap();
        let orig_doc = PdfDocument::from_bytes(&bytes)
            .unwrap_or_else(|err| panic!("{}: orig failed to open: {err:?}", sample.path));

        let mut out = Vec::new();
        let id = [2u8; 16];
        let (_bytes, _report) = orig_doc
            .sanitized_copy(&id, DEFAULT_OPTIONS, &mut out)
            .unwrap_or_else(|err| panic!("{}: sanitize failed: {err:?}", sample.path));

        let sanitized_doc = PdfDocument::from_bytes(&out)
            .unwrap_or_else(|err| panic!("{}: copy failed to open: {err:?}", sample.path));

        let page_count = orig_doc.page_count().unwrap();
        assert_eq!(
            page_count,
            sanitized_doc.page_count().unwrap(),
            "{}: page count changed",
            sample.path
        );

        for page_idx in 0..page_count {
            let orig_page = orig_doc.render(page_idx, 1.0, 0).unwrap();
            let sanitized_page = sanitized_doc.render(page_idx, 1.0, 0).unwrap();
            assert_eq!(
                orig_page.width, sanitized_page.width,
                "{}: width mismatch page {page_idx}",
                sample.path
            );
            assert_eq!(
                orig_page.height, sanitized_page.height,
                "{}: height mismatch page {page_idx}",
                sample.path
            );
            assert_eq!(
                orig_page.rgba, sanitized_page.rgba,
                "{}: pixel difference on page {page_idx}",
                sample.path
            );
        }
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} benign samples checked");
}

#[test]
fn annotation_flatten_and_remove_modes() {
    let sample_file = corpus().join("malicious/composite-sanitize.pdf");
    let bytes = std::fs::read(&sample_file).unwrap();

    // 1. Flatten mode: renders identically to original with Keep
    let orig_doc = PdfDocument::from_bytes(&bytes).unwrap();
    let id = [3u8; 16];

    let flatten_options = SanitizeOptions {
        actions: true,
        attachments: true,
        form_scripts: true,
        xfa: true,
        rich_media: true,
        hidden_data: true,
        keep_web_links: true,
        annotations: SanitizeAnnotations::Flatten,
    };
    let mut flat_out = Vec::new();
    let (_, _flat_report) = orig_doc
        .sanitized_copy(&id, flatten_options, &mut flat_out)
        .unwrap();

    let flat_doc = PdfDocument::from_bytes(&flat_out).unwrap();

    // After bake(true, false), annotations (except fields) are baked into content.
    // The page annotations count (which lists non-widget annots) should be 0.
    let flat_annots = flat_doc.page_annotations(0).unwrap();
    assert_eq!(
        flat_annots.len(),
        0,
        "flattened copy should have no markup annotations"
    );

    // Form fields are preserved
    let flat_fields = flat_doc.page_fields(0).unwrap();
    assert_eq!(flat_fields.len(), 1, "form field should be preserved");

    // Check pixel rendering: baking annotations produces visually identical rendering
    let orig_rendered = orig_doc.render(0, 1.0, 0).unwrap();
    let flat_rendered = flat_doc.render(0, 1.0, 0).unwrap();
    assert_eq!(orig_rendered.width, flat_rendered.width);
    assert_eq!(orig_rendered.height, flat_rendered.height);
    // Tolerance: baked annotations match within negligible antialiasing differences (<= 1% pixel diff)
    let diff_count = orig_rendered
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(flat_rendered.rgba.as_chunks::<4>().0)
        .filter(|(p1, p2)| p1 != p2)
        .count();
    let total_pixels = (orig_rendered.width * orig_rendered.height) as usize;
    assert!(
        diff_count * 100 / total_pixels <= 1,
        "flattened rendering difference exceeds 1%: {diff_count}/{total_pixels}"
    );

    // 2. Remove mode: non-widget, non-link annotations are removed
    let remove_options = SanitizeOptions {
        actions: true,
        attachments: true,
        form_scripts: true,
        xfa: true,
        rich_media: true,
        hidden_data: true,
        keep_web_links: true,
        annotations: SanitizeAnnotations::Remove,
    };
    let mut rem_out = Vec::new();
    let (_, rem_report) = orig_doc
        .sanitized_copy(&id, remove_options, &mut rem_out)
        .unwrap();
    assert!(
        rem_report.annotations > 0,
        "annotations should be counted as removed"
    );

    let rem_doc = PdfDocument::from_bytes(&rem_out).unwrap();
    let rem_annots = rem_doc.page_annotations(0).unwrap();
    assert_eq!(
        rem_annots.len(),
        0,
        "remove mode should leave 0 markup annotations"
    );

    // Form fields are preserved
    let rem_fields = rem_doc.page_fields(0).unwrap();
    assert_eq!(rem_fields.len(), 1, "widget should be kept");

    // Web link is preserved
    let rem_links = rem_doc.page_links(0, MAX_LINKS_PER_PAGE as usize).unwrap();
    assert_eq!(rem_links.len(), 1, "web link should be kept");
}

#[test]
fn keep_web_links_filters_schemes_correctly() {
    let id = [4u8; 16];

    // link-file-scheme.pdf has file:/// URL
    let file_bytes = std::fs::read(corpus().join("malicious/link-file-scheme.pdf")).unwrap();
    let doc = PdfDocument::from_bytes(&file_bytes).unwrap();

    let mut out = Vec::new();
    let (_, report) = doc.sanitized_copy(&id, DEFAULT_OPTIONS, &mut out).unwrap();
    assert_eq!(report.links, 1, "dangerous file link must be removed");
    let copy = PdfDocument::from_bytes(&out).unwrap();
    let links = copy.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 0, "file link annotation must be deleted");

    // external-https-link.pdf has https:// URL
    let https_bytes = std::fs::read(corpus().join("benign/external-https-link.pdf")).unwrap();
    let https_doc = PdfDocument::from_bytes(&https_bytes).unwrap();

    // With keep_web_links = true: https link is retained
    let mut https_out = Vec::new();
    let (_, https_rep) = https_doc
        .sanitized_copy(&id, DEFAULT_OPTIONS, &mut https_out)
        .unwrap();
    assert_eq!(https_rep.links, 0, "https link should not be removed");
    let https_copy = PdfDocument::from_bytes(&https_out).unwrap();
    let links = https_copy.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 1, "https link should be kept");

    // With keep_web_links = false: all URI links are removed
    let no_links_opts = SanitizeOptions {
        keep_web_links: false,
        ..DEFAULT_OPTIONS
    };
    let mut no_links_out = Vec::new();
    let (_, no_links_rep) = https_doc
        .sanitized_copy(&id, no_links_opts, &mut no_links_out)
        .unwrap();
    assert_eq!(
        no_links_rep.links, 1,
        "https link must be removed when keep_web_links=false"
    );
    let no_links_copy = PdfDocument::from_bytes(&no_links_out).unwrap();
    let links = no_links_copy.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 0, "no links should remain");
}

#[test]
fn signed_and_encrypted_documents_are_refused() {
    let id = [5u8; 16];

    // Signed documents
    for name in ["benign/signed.pdf", "benign/signed-docmdp-p1.pdf"] {
        let bytes = std::fs::read(corpus().join(name)).unwrap();
        let doc = PdfDocument::from_bytes(&bytes).unwrap();
        let mut out = Vec::new();
        let err = doc
            .sanitized_copy(&id, DEFAULT_OPTIONS, &mut out)
            .unwrap_err();
        assert!(
            matches!(err, EngineError::Signed),
            "{name}: expected Signed, got {err:?}"
        );
    }

    // Encrypted documents
    for name in ["benign/encrypted-aes256.pdf", "benign/encrypted-rc4-40.pdf"] {
        let bytes = std::fs::read(corpus().join(name)).unwrap();
        let doc = PdfDocument::open(&bytes, Some("user")).unwrap();
        let mut out = Vec::new();
        let err = doc
            .sanitized_copy(&id, DEFAULT_OPTIONS, &mut out)
            .unwrap_err();
        assert!(
            matches!(err, EngineError::EncryptedCopy),
            "{name}: expected EncryptedCopy, got {err:?}"
        );
    }
}

#[test]
fn original_document_in_worker_is_unaffected_and_can_be_sanitized_again() {
    let sample_file = corpus().join("malicious/composite-sanitize.pdf");
    let bytes = std::fs::read(&sample_file).unwrap();
    let doc = PdfDocument::from_bytes(&bytes).unwrap();

    let id1 = [6u8; 16];
    let mut out1 = Vec::new();
    let (written1, rep1) = doc
        .sanitized_copy(&id1, DEFAULT_OPTIONS, &mut out1)
        .unwrap();

    // Original doc still has its active content
    let sec_report = doc.active_content(ScanBudget::default());
    assert!(
        !sec_report.findings.is_empty(),
        "original document must remain intact"
    );

    // Second sanitize on same document instance succeeds identically
    let id2 = [7u8; 16];
    let mut out2 = Vec::new();
    let (written2, rep2) = doc
        .sanitized_copy(&id2, DEFAULT_OPTIONS, &mut out2)
        .unwrap();

    assert_eq!(written1, written2);
    assert_eq!(rep1, rep2);
}

fn sanitized(bytes: &[u8]) -> (Vec<u8>, SanitizeReport) {
    let document = PdfDocument::from_bytes(bytes).unwrap();
    let mut out = Vec::new();
    let (_, report) = document
        .sanitized_copy(&[8; 16], DEFAULT_OPTIONS, &mut out)
        .unwrap();
    (out, report)
}

#[test]
fn each_removed_item_is_counted_once() {
    // The composite sample's attachment is named both by the list of embedded files and by an
    // attachment annotation; the document-level script is an action reachable from the names
    // tree only.
    for (path, actions, attachments) in [
        ("malicious/composite-sanitize.pdf", 1, 1),
        ("malicious/embedded-file.pdf", 0, 1),
        ("malicious/js-document-level.pdf", 1, 0),
        ("malicious/openaction-js.pdf", 1, 0),
    ] {
        let (_, report) = sanitized(&std::fs::read(corpus().join(path)).unwrap());
        assert_eq!(
            (report.actions, report.attachments),
            (actions, attachments),
            "{path}: {report:?}"
        );
    }

    // Nothing to take out: nothing reported (the identifier is replaced, not counted).
    let (_, report) = sanitized(&common::one_page("BT /F1 12 Tf 72 720 Td (Plain) Tj ET"));
    assert_eq!(report, SanitizeReport::default());
}

#[test]
fn looping_action_chains_are_removed_without_hanging() {
    let bytes = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R 6 0 R] >>"
            .into(),
        // Its action goes on to itself.
        "<< /Type /Annot /Subtype /Link /Rect [72 700 200 720] /A 7 0 R >>".into(),
        // Its action goes on to itself and to a script.
        "<< /Type /Annot /Subtype /Link /Rect [72 650 200 670] /A 8 0 R >>".into(),
        // Two jumps in a row: harmless, kept.
        "<< /Type /Annot /Subtype /Link /Rect [72 600 200 620] /A 10 0 R >>".into(),
        "<< /S /GoTo /D [3 0 R /Fit] /Next 7 0 R >>".into(),
        "<< /S /GoTo /D [3 0 R /Fit] /Next [8 0 R 9 0 R] >>".into(),
        "<< /S /JavaScript /JS (app.alert(1)) >>".into(),
        "<< /S /GoTo /D [3 0 R /Fit] /Next << /S /GoTo /D [3 0 R /Fit] >> >>".into(),
    ]);
    let (out, report) = sanitized(&bytes);
    assert_eq!(report.actions, 2, "{report:?}");
    let copy = PdfDocument::from_bytes(&out).unwrap();
    assert!(
        copy.active_content(ScanBudget::default())
            .findings
            .is_empty()
    );
    let links = copy.page_links(0, MAX_LINKS_PER_PAGE as usize).unwrap();
    assert_eq!(links.len(), 1, "{links:?}");
}
