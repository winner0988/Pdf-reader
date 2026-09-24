//! Page links (MVP-12): where they are and where they point, for every link sample of the test
//! corpus, through the real worker in its sandbox.
#![cfg(windows)]

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};

use ipc_contract::types::{BlockedAction, DocumentId, LinkTarget, PageLink, Rect};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
use pdf_worker::engine::PdfDocument;
use worker_host::{HostConfig, HostError, WorkerHost};

use common::build_pdf;

fn corpus(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(path)
}

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn open(host: &mut WorkerHost, path: &str) -> DocumentId {
    host.open(&corpus(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .0
}

fn request_links(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Result<WorkerResponse, HostError> {
    host.request(|request| WorkerRequest::GetPageLinks {
        request,
        doc,
        page_index,
    })
}

/// The links of the first page of a corpus sample.
fn links(path: &str) -> Vec<PageLink> {
    let mut host = host();
    let doc = open(&mut host, path);
    match request_links(&mut host, doc, 0).unwrap() {
        WorkerResponse::PageLinks {
            page_index: 0,
            links,
            ..
        } => links,
        other => panic!("{path}: {other:?}"),
    }
}

/// The target of the only link on the first page.
fn only_target(path: &str) -> LinkTarget {
    let links = links(path);
    assert_eq!(links.len(), 1, "{path}: {links:?}");
    links.into_iter().next().unwrap().target
}

fn blocked(action: BlockedAction, target: &str) -> LinkTarget {
    LinkTarget::Blocked {
        action,
        target: Some(target.to_owned()),
    }
}

#[test]
fn internal_links_point_at_pages_and_sit_where_the_pdf_puts_them() {
    let links = links("benign/internal-links.pdf");
    let pages: Vec<_> = links
        .iter()
        .map(|link| match link.target {
            LinkTarget::Page { page_index, .. } => page_index,
            ref other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(pages, [2, 1]);
    // /Rect [72 690 300 712] on a Letter page (792 pt tall), measured from the top.
    assert_eq!(
        links[0].rect,
        Rect {
            x0: 72.0,
            y0: 80.0,
            x1: 300.0,
            y1: 102.0
        }
    );
    assert_eq!(
        links.iter().map(|link| link.id.index).collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn web_links_are_offered_exactly_as_written() {
    assert_eq!(
        only_target("benign/external-https-link.pdf"),
        LinkTarget::Uri {
            uri: "https://example.invalid/docs".to_owned()
        }
    );
    // Cyrillic "а" (U+0430): decoded from UTF-8, not garbled, so the look-alike can be shown.
    assert_eq!(
        only_target("malicious/link-idn-homograph.pdf"),
        LinkTarget::Uri {
            uri: "https://\u{0430}pple.example.invalid/".to_owned()
        }
    );
    // The right-to-left override stays, for the confirmation to show and warn about.
    assert_eq!(
        only_target("malicious/link-rtl-override.pdf"),
        LinkTarget::Uri {
            uri: "https://example.invalid/\u{202E}fdp.exe".to_owned()
        }
    );
    let LinkTarget::Uri { uri } = only_target("malicious/link-long-url.pdf") else {
        panic!("the long URL must be offered");
    };
    assert_eq!(uri.len(), 10_000);
}

#[test]
fn everything_else_is_blocked_with_its_reason() {
    for (path, expected) in [
        (
            "malicious/launch.pdf",
            blocked(BlockedAction::Launch, "does-not-exist.example.exe"),
        ),
        (
            "malicious/gotor-unc.pdf",
            blocked(
                BlockedAction::RemoteGoTo,
                "\\\\share.example.invalid\\x\\doc.pdf",
            ),
        ),
        (
            "malicious/gotoe.pdf",
            LinkTarget::Blocked {
                action: BlockedAction::EmbeddedGoTo,
                target: None,
            },
        ),
        (
            "malicious/link-javascript-scheme.pdf",
            blocked(BlockedAction::JavaScript, "javascript:app.alert(1)"),
        ),
        (
            "malicious/link-file-scheme.pdf",
            blocked(
                BlockedAction::LocalFile,
                "file:///C:/Windows/System32/calc.exe",
            ),
        ),
        (
            "malicious/link-smb-scheme.pdf",
            blocked(BlockedAction::NetworkShare, "smb://share.example.invalid/x"),
        ),
        (
            "malicious/link-unc-uri.pdf",
            blocked(BlockedAction::NetworkShare, "\\\\share.example.invalid\\x"),
        ),
        (
            "malicious/link-ms-protocol.pdf",
            blocked(BlockedAction::Other, "ms-msdt:/id PCWDiagnostic"),
        ),
    ] {
        assert_eq!(only_target(path), expected, "{path}");
    }
}

#[test]
fn a_page_without_links_and_a_page_that_does_not_exist() {
    let mut host = host();
    let doc = open(&mut host, "benign/single-page.pdf");
    assert!(matches!(
        request_links(&mut host, doc, 0),
        Ok(WorkerResponse::PageLinks { links, .. }) if links.is_empty()
    ));
    assert!(matches!(
        request_links(&mut host, doc, 1),
        Err(HostError::Worker(error)) if error.code == WorkerErrorCode::PageOutOfRange
    ));
}

/// A page whose link is drawn around the word "needle": the link rectangle, in page space,
/// must cover the word as the text layer places it.
fn link_around_a_word(page_extra: &str) -> Vec<u8> {
    let content = "BT /F1 12 Tf 150 500 Td (needle) Tj ET";
    build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R {page_extra} /Resources << /Font << /F1 5 0 R >> >> \
             /Contents 4 0 R /Annots [6 0 R] >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [140 490 200 520] /A << /S /GoTo /D [3 0 R /Fit] >> >>"
            .into(),
    ])
}

#[test]
fn link_rectangles_follow_rotated_and_offset_pages() {
    for page_extra in [
        "/MediaBox [0 0 612 792]",
        "/MediaBox [0 0 612 792] /Rotate 90",
        "/MediaBox [0 0 612 792] /Rotate 270",
        "/MediaBox [100 100 712 892]",
        "/MediaBox [0 0 612 792] /CropBox [50 50 562 742] /Rotate 180",
    ] {
        let document = PdfDocument::from_bytes(&link_around_a_word(page_extra)).unwrap();
        let links = document.page_links(0, 10).unwrap();
        assert_eq!(links.len(), 1, "{page_extra}");
        let [x0, y0, x1, y1] = links[0].rect;
        let hits = document.search_page(0, "needle", false, 10).unwrap().hits;
        assert_eq!(hits.len(), 1, "{page_extra}");
        for quad in &hits[0].quads {
            for point in [quad.ul, quad.ur, quad.ll, quad.lr] {
                assert!(
                    (x0..=x1).contains(&point.x) && (y0..=y1).contains(&point.y),
                    "{page_extra}: {point:?} outside {:?}",
                    links[0].rect
                );
            }
        }
    }
}

#[test]
fn links_without_a_usable_rectangle_are_skipped() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R 6 0 R 7 0 R] >>"
            .into(),
        "<< /Type /Annot /Subtype /Link /Rect [0 0 10] /A << /S /GoTo /D [3 0 R /Fit] >> >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [0 0 (a) 10] /A << /S /GoTo /D [3 0 R /Fit] >> >>"
            .into(),
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [10 10 0 0] /A << /S /GoTo /D [3 0 R /Fit] >> >>"
            .into(),
    ]);
    let links = PdfDocument::from_bytes(&pdf)
        .unwrap()
        .page_links(0, 10)
        .unwrap();
    // Only the last one: a reversed rectangle is still a rectangle.
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].rect, [0.0, 782.0, 10.0, 792.0]);
}
