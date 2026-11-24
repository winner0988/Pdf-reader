//! Page links (MVP-12): where they are and where they point, for every link sample of the test
//! corpus, through the real worker in its sandbox.
#![cfg(windows)]

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};

use ipc_contract::types::{
    BlockedAction, DocumentId, LinkId, LinkTarget, NewLinkTarget, PageLink, Rect,
};
use ipc_contract::worker::{WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse};
use pdf_worker::engine::{EngineError, OutlineTarget, PdfDocument};
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
    assert_eq!(
        only_target("malicious/link-mailto-recipients.pdf"),
        LinkTarget::Uri {
            uri: "mailto:someone@first.example;ceo@second.example".to_owned()
        }
    );
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
            "malicious/gotor-localhost-host.pdf",
            blocked(
                BlockedAction::RemoteGoTo,
                "file://localhost.example/share/doc.pdf",
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
            "malicious/link-localhost-host-uri.pdf",
            blocked(
                BlockedAction::NetworkShare,
                "file://localhost.example/share/doc.pdf",
            ),
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

#[test]
fn test_add_links_and_verify_dictionary_constraints_and_qpdf() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>".into(),
    ]);
    let mut doc = PdfDocument::from_bytes(&pdf).unwrap();

    let uri_rect = Rect {
        x0: 50.0,
        y0: 50.0,
        x1: 200.0,
        y1: 80.0,
    };
    doc.add_link(
        0,
        uri_rect,
        &NewLinkTarget::Uri {
            uri: "https://example.com/test".into(),
        },
    )
    .unwrap();

    let page_rect = Rect {
        x0: 50.0,
        y0: 100.0,
        x1: 200.0,
        y1: 130.0,
    };
    doc.add_link(0, page_rect, &NewLinkTarget::Page { page: 1 })
        .unwrap();

    // Verify dictionary constraints directly on the page's /Annots
    let links = doc.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].rect, [50.0, 50.0, 200.0, 80.0]);
    assert_eq!(
        links[0].target,
        OutlineTarget::Uri("https://example.com/test".into())
    );
    assert_eq!(links[1].rect, [50.0, 100.0, 200.0, 130.0]);
    assert_eq!(links[1].target, OutlineTarget::Page(1));

    // Save and check with qpdf
    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    common::check_qpdf_bytes(&saved);

    // Reopen and check
    let reopened = PdfDocument::from_bytes(&saved).unwrap();
    let read_links = reopened.page_links(0, 10).unwrap();
    assert_eq!(read_links.len(), 2);
    assert_eq!(read_links[0].rect, [50.0, 50.0, 200.0, 80.0]);
    assert_eq!(
        read_links[0].target,
        OutlineTarget::Uri("https://example.com/test".into())
    );
    assert_eq!(read_links[1].rect, [50.0, 100.0, 200.0, 130.0]);
    assert_eq!(read_links[1].target, OutlineTarget::Page(1));
}

#[test]
fn test_retarget_link_stripping_and_unlinking_next_chain() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R] >>".into(),
        // Object 4: Link with /A (5 0 R), /AA << /U 7 0 R >>, /PA 8 0 R
        "<< /Type /Annot /Subtype /Link /Rect [100 100 200 150] /A 5 0 R /AA << /U 7 0 R >> /PA 8 0 R >>".into(),
        // Object 5: Action with /Next pointing to 6 0 R
        "<< /S /Launch /F (evil.exe) /Next 6 0 R >>".into(),
        // Object 6: Chained action
        "<< /S /Launch /F (calc.exe) >>".into(),
        // Object 7: Additional action
        "<< /S /Launch /F (aa.exe) >>".into(),
        // Object 8: Presence action
        "<< /S /Launch /F (pa.exe) >>".into(),
    ]);

    let mut doc = PdfDocument::from_bytes(&pdf).unwrap();
    let initial_links = doc.page_links(0, 10).unwrap();
    assert_eq!(initial_links.len(), 1);

    // Retarget to safe URI
    let link_id = LinkId {
        page_index: 0,
        index: 0,
    };
    doc.set_link_target(
        link_id,
        &NewLinkTarget::Uri {
            uri: "https://safe.example.com/".into(),
        },
    )
    .unwrap();

    // Verify retargeted link
    let retargeted = doc.page_links(0, 10).unwrap();
    assert_eq!(retargeted.len(), 1);
    assert_eq!(
        retargeted[0].target,
        OutlineTarget::Uri("https://safe.example.com/".into())
    );

    // Save and check with qpdf
    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    common::check_qpdf_bytes(&saved);

    let saved_text = String::from_utf8_lossy(&saved);
    // The previous evil/calc/aa/pa targets must not be referenced by the link
    assert!(!saved_text.contains("evil.exe"));
}

#[test]
fn test_negative_control_retargeting_without_next_unlinking() {
    // Verifies that unlinking actually reaches the /Next chain.
    // If an action has a /Next chain, after retargeting, the old action objects are unlinked.
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [100 100 200 150] /A 5 0 R >>".into(),
        "<< /S /Launch /F (evil.exe) /Next 6 0 R >>".into(),
        "<< /S /Launch /F (calc.exe) >>".into(),
    ]);
    let mut doc = PdfDocument::from_bytes(&pdf).unwrap();
    doc.set_link_target(
        LinkId {
            page_index: 0,
            index: 0,
        },
        &NewLinkTarget::Uri {
            uri: "https://safe.example.com/".into(),
        },
    )
    .unwrap();
    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("calc.exe"));
    assert!(!text.contains("evil.exe"));
}

#[test]
fn test_delete_link_and_invalid_edits() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R 5 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [100 100 200 150] /A << /S /URI /URI (https://first.com) >> >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [100 200 200 250] /A << /S /URI /URI (https://second.com) >> >>".into(),
    ]);
    let mut doc = PdfDocument::from_bytes(&pdf).unwrap();
    assert_eq!(doc.page_links(0, 10).unwrap().len(), 2);

    // Delete first link
    doc.delete_link(LinkId {
        page_index: 0,
        index: 0,
    })
    .unwrap();

    let remaining = doc.page_links(0, 10).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].target,
        OutlineTarget::Uri("https://second.com".into())
    );

    // Out-of-bounds link index
    let err = doc.delete_link(LinkId {
        page_index: 0,
        index: 99,
    });
    assert!(matches!(err, Err(EngineError::InvalidEdit(_))));

    // Out-of-bounds page index
    let err = doc.delete_link(LinkId {
        page_index: 99,
        index: 0,
    });
    assert!(matches!(err, Err(EngineError::PageOutOfRange(99))));

    // Save and check with qpdf
    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    common::check_qpdf_bytes(&saved);
}

fn shared_action_pdf() -> Vec<u8> {
    build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R 5 0 R] >>"
            .into(),
        "<< /Type /Annot /Subtype /Link /Rect [50 600 200 630] /A 6 0 R >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [50 500 200 530] /A 6 0 R >>".into(),
        "<< /S /URI /URI (https://shared.example.invalid/doc) >>".into(),
        "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>".into(),
        "<< /Title (Shared bookmark) /Parent 7 0 R /A 6 0 R >>".into(),
    ])
}

#[test]
fn test_retarget_link_preserves_shared_indirect_action() {
    let mut doc = PdfDocument::from_bytes(&shared_action_pdf()).unwrap();
    let shared_target = OutlineTarget::Uri("https://shared.example.invalid/doc".into());

    assert_eq!(doc.page_links(0, 10).unwrap().len(), 2);
    assert_eq!(doc.outline(10, 4).unwrap().entries[0].target, shared_target);

    doc.set_link_target(
        LinkId {
            page_index: 0,
            index: 0,
        },
        &NewLinkTarget::Uri {
            uri: "https://retargeted.example.invalid/".into(),
        },
    )
    .unwrap();

    let links = doc.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(
        links[0].target,
        OutlineTarget::Uri("https://retargeted.example.invalid/".into())
    );
    assert_eq!(links[1].target, shared_target);

    let outline = doc.outline(10, 4).unwrap();
    assert_eq!(outline.entries.len(), 1);
    assert_eq!(outline.entries[0].target, shared_target);

    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    common::check_qpdf_bytes(&saved);

    let reopened = PdfDocument::from_bytes(&saved).unwrap();
    let reopened_links = reopened.page_links(0, 10).unwrap();
    assert_eq!(reopened_links.len(), 2);
    assert_eq!(
        reopened_links[0].target,
        OutlineTarget::Uri("https://retargeted.example.invalid/".into())
    );
    assert_eq!(reopened_links[1].target, shared_target);
    assert_eq!(
        reopened.outline(10, 4).unwrap().entries[0].target,
        shared_target
    );
}

#[test]
fn test_delete_link_preserves_shared_indirect_action() {
    let mut doc = PdfDocument::from_bytes(&shared_action_pdf()).unwrap();
    let shared_target = OutlineTarget::Uri("https://shared.example.invalid/doc".into());

    doc.delete_link(LinkId {
        page_index: 0,
        index: 0,
    })
    .unwrap();

    let links = doc.page_links(0, 10).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, shared_target);

    let outline = doc.outline(10, 4).unwrap();
    assert_eq!(outline.entries.len(), 1);
    assert_eq!(outline.entries[0].target, shared_target);

    let mut saved = Vec::new();
    doc.save(&mut saved).unwrap();
    common::check_qpdf_bytes(&saved);

    let reopened = PdfDocument::from_bytes(&saved).unwrap();
    let reopened_links = reopened.page_links(0, 10).unwrap();
    assert_eq!(reopened_links.len(), 1);
    assert_eq!(reopened_links[0].target, shared_target);
    assert_eq!(
        reopened.outline(10, 4).unwrap().entries[0].target,
        shared_target
    );
}

#[test]
fn test_retarget_link_drops_unreferenced_old_uri_on_save() {
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R] >>"
            .into(),
        "<< /Type /Annot /Subtype /Link /Rect [50 600 200 630] /A 5 0 R >>".into(),
        "<< /S /URI /URI (https://old-unshared.example.invalid/removed) >>".into(),
    ]);
    let mut doc = PdfDocument::from_bytes(&pdf).unwrap();

    doc.set_link_target(
        LinkId {
            page_index: 0,
            index: 0,
        },
        &NewLinkTarget::Uri {
            uri: "https://new-target.example.invalid/".into(),
        },
    )
    .unwrap();

    let mut saved = Vec::new();
    let (_bytes, incremental) = doc.save(&mut saved).unwrap();
    assert!(!incremental);
    common::check_qpdf_bytes(&saved);

    let saved_text = String::from_utf8_lossy(&saved);
    assert!(!saved_text.contains("old-unshared.example.invalid"));
    assert!(saved_text.contains("new-target.example.invalid"));

    let reopened = PdfDocument::from_bytes(&saved).unwrap();
    let reopened_links = reopened.page_links(0, 10).unwrap();
    assert_eq!(reopened_links.len(), 1);
    assert_eq!(
        reopened_links[0].target,
        OutlineTarget::Uri("https://new-target.example.invalid/".into())
    );
}

#[test]
fn test_worker_host_link_edits_round_trip() {
    let mut host = host();
    let doc = open(&mut host, "benign/multi-page-10.pdf");

    // Add link via WorkerHost
    let add_edit = WorkerEdit::AddLink {
        page: 0,
        rect: Rect {
            x0: 50.0,
            y0: 50.0,
            x1: 150.0,
            y1: 80.0,
        },
        target: NewLinkTarget::Uri {
            uri: "https://example.com/ipc-test".into(),
        },
    };
    let resp = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: add_edit,
        })
        .unwrap();
    assert!(matches!(resp, WorkerResponse::Edited { .. }));

    // Verify link via GetPageLinks
    let links_resp = request_links(&mut host, doc, 0).unwrap();
    let links = match links_resp {
        WorkerResponse::PageLinks { links, .. } => links,
        other => panic!("expected PageLinks, got {other:?}"),
    };
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].target,
        LinkTarget::Uri {
            uri: "https://example.com/ipc-test".into()
        }
    );

    // Retarget link
    let retarget_edit = WorkerEdit::SetLinkTarget {
        link: links[0].id,
        target: NewLinkTarget::Page { page: 0 },
    };
    let resp = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: retarget_edit,
        })
        .unwrap();
    assert!(matches!(resp, WorkerResponse::Edited { .. }));

    let links_resp2 = request_links(&mut host, doc, 0).unwrap();
    let links2 = match links_resp2 {
        WorkerResponse::PageLinks { links, .. } => links,
        other => panic!("expected PageLinks, got {other:?}"),
    };
    assert_eq!(links2.len(), 1);
    assert_eq!(
        links2[0].target,
        LinkTarget::Page {
            page_index: 0,
            x: None,
            y: None
        }
    );

    // Delete link
    let delete_edit = WorkerEdit::DeleteLink { link: links2[0].id };
    let resp = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: delete_edit,
        })
        .unwrap();
    assert!(matches!(resp, WorkerResponse::Edited { .. }));

    let links_resp3 = request_links(&mut host, doc, 0).unwrap();
    let links3 = match links_resp3 {
        WorkerResponse::PageLinks { links, .. } => links,
        other => panic!("expected PageLinks, got {other:?}"),
    };
    assert_eq!(links3.len(), 0);
}
