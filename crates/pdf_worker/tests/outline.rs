//! Outlines through the real, sandboxed worker (MVP-09): ordinary, hostile and broken ones.
#![cfg(windows)]

mod common;

use std::path::Path;

use ipc_contract::limits::{MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS};
use ipc_contract::types::{BlockedAction, DocumentId, LinkTarget, OutlineResult};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

use common::{build_pdf, temp_pdf};

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pdf_worker"))
}

/// One Letter page with an outline. `items` are dictionary bodies for objects 5, 6, ...
/// (the outline root is object 4 and points at `first`..`last`).
fn pdf_with_outline(first: usize, last: usize, items: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
        format!("<< /Type /Outlines /First {first} 0 R /Last {last} 0 R >>"),
    ];
    objects.extend(items.iter().cloned());
    build_pdf(&objects)
}

/// Siblings 5..5+count under the root, each pointing at the page.
fn flat_outline(count: usize) -> Vec<u8> {
    let items: Vec<String> = (0..count)
        .map(|index| {
            let object = 5 + index;
            let mut dict =
                format!("<< /Title (Item {index}) /Parent 4 0 R /Dest [3 0 R /XYZ 0 792 0]");
            if index > 0 {
                dict += &format!(" /Prev {} 0 R", object - 1);
            }
            if index + 1 < count {
                dict += &format!(" /Next {} 0 R", object + 1);
            }
            dict + " >>"
        })
        .collect();
    pdf_with_outline(5, 4 + count, &items)
}

/// A chain of `depth` items, each the only child of the one before.
fn nested_outline(depth: usize) -> Vec<u8> {
    let items: Vec<String> = (0..depth)
        .map(|level| {
            let object = 5 + level;
            let parent = if level == 0 { 4 } else { object - 1 };
            let mut dict = format!(
                "<< /Title (Level {level}) /Parent {parent} 0 R /Dest [3 0 R /XYZ 0 792 0]"
            );
            if level + 1 < depth {
                dict += &format!(" /First {0} 0 R /Last {0} 0 R", object + 1);
            }
            dict + " >>"
        })
        .collect();
    pdf_with_outline(5, 5, &items)
}

/// Opens `pdf` and asks for its outline.
fn outline_of(name: &str, pdf: &[u8]) -> (WorkerHost, Result<OutlineResult, HostError>) {
    let path = temp_pdf(&format!("outline-{name}"), pdf);
    let mut host = WorkerHost::new(worker(), HostConfig::default());
    let (doc, _) = host.open(&path).expect("open");
    let result = outline_request(&mut host, doc);
    std::fs::remove_file(path).ok();
    (host, result)
}

fn outline_request(host: &mut WorkerHost, doc: DocumentId) -> Result<OutlineResult, HostError> {
    host.request(|request| WorkerRequest::GetOutline { request, doc })
        .map(|response| match response {
            WorkerResponse::Outline { outline, .. } => outline,
            other => panic!("unexpected response {other:?}"),
        })
}

#[test]
fn titles_are_cleaned_and_targets_classified() {
    let items = vec![
        // "Invoice <U+202E>fdp.exe" would display as "Invoice exe.pdf".
        "<< /Title <FEFF0049006E0076006F0069006300650020202E006600640070002E006500780065> /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /XYZ 0 792 0] >>".to_owned(),
        "<< /Title (Line\\r\\nbreak) /Parent 4 0 R /Prev 5 0 R /Next 7 0 R /A << /S /URI /URI (https://example.invalid/) >> >>".to_owned(),
        "<< /Title (Script) /Parent 4 0 R /Prev 6 0 R /Next 8 0 R /A << /S /URI /URI (javascript:alert\\(1\\)) >> >>".to_owned(),
        "<< /Title (Other file) /Parent 4 0 R /Prev 7 0 R /Next 9 0 R /A << /S /GoToR /F (C:/secret.pdf) /D [0 /Fit] >> >>".to_owned(),
        "<< /Title (Missing page) /Parent 4 0 R /Prev 8 0 R /Next 10 0 R /Dest [99 /Fit] >>".to_owned(),
        "<< /Title (Dangling) /Parent 4 0 R /Prev 9 0 R /Dest [99 0 R /Fit] >>".to_owned(),
    ];
    let (_host, outline) = outline_of("hostile", &pdf_with_outline(5, 10, &items));
    let outline = outline.expect("outline");
    let titles: Vec<&str> = outline
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Invoice fdp.exe",
            "Line break",
            "Script",
            "Other file",
            "Missing page",
            "Dangling"
        ]
    );
    assert_eq!(
        outline.items[0].target,
        Some(LinkTarget::Page {
            page_index: 0,
            x: None,
            y: None
        })
    );
    assert_eq!(
        outline.items[1].target,
        Some(LinkTarget::Uri {
            uri: "https://example.invalid/".to_owned()
        })
    );
    assert!(matches!(
        outline.items[2].target,
        Some(LinkTarget::Blocked {
            action: BlockedAction::JavaScript,
            ..
        })
    ));
    // Another file is never offered as a jump; its name is shown as text.
    assert_eq!(
        outline.items[3].target,
        Some(LinkTarget::Blocked {
            action: BlockedAction::RemoteGoTo,
            target: Some("C:/secret.pdf".to_owned())
        })
    );
    assert_eq!(
        outline.items[4].target, None,
        "page 100 of a 1-page document"
    );
    // A reference to a missing object is resolved by MuPDF; whatever it picks is a real page.
    assert!(matches!(
        outline.items[5].target,
        None | Some(LinkTarget::Page { page_index: 0, .. })
    ));
}

#[test]
fn a_huge_outline_is_cut_at_the_item_limit() {
    let (_host, outline) = outline_of("huge", &flat_outline(MAX_OUTLINE_ITEMS as usize + 500));
    let outline = outline.expect("outline");
    assert_eq!(outline.items.len(), MAX_OUTLINE_ITEMS as usize);
    assert!(outline.truncated);
}

#[test]
fn a_deeply_nested_outline_is_cut_at_the_depth_limit_without_crashing() {
    let (host, outline) = outline_of("deep", &nested_outline(20_000));
    let outline = outline.expect("the worker must survive a 20,000-level outline");
    assert_eq!(outline.items.len(), usize::from(MAX_OUTLINE_DEPTH) + 1);
    assert_eq!(outline.items.last().unwrap().depth, MAX_OUTLINE_DEPTH);
    assert!(outline.truncated);
    assert!(host.is_running());
}

#[test]
fn a_cyclic_outline_is_cut_at_the_cycle() {
    let items = vec![
        "<< /Title (A) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /XYZ 0 792 0] >>".to_owned(),
        "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Next 5 0 R /Dest [3 0 R /XYZ 0 792 0] >>"
            .to_owned(),
    ];
    let (_host, outline) = outline_of("cycle", &pdf_with_outline(5, 6, &items));
    let outline = outline.expect("outline");
    let titles: Vec<&str> = outline
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(titles, ["A", "B"]);
    assert!(outline.truncated);
}

#[test]
fn documents_without_an_outline_have_an_empty_one() {
    let pdf = common::one_page("0 0 0 rg 0 0 10 10 re f");
    let (_host, outline) = outline_of("none", &pdf);
    assert_eq!(outline.expect("outline"), OutlineResult::default());
}
