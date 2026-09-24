//! The active-content scan (MVP-11): every sample of the test corpus, and the structures that
//! must or must not count.

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipc_contract::types::{FindingKind, SecurityFinding, SecurityReport};
use pdf_worker::engine::PdfDocument;
use pdf_worker::scan::ScanBudget;

use common::build_pdf;

fn scan(objects: &[String]) -> SecurityReport {
    scan_with(objects, ScanBudget::default())
}

fn scan_with(objects: &[String], budget: ScanBudget) -> SecurityReport {
    PdfDocument::from_bytes(&build_pdf(objects))
        .unwrap()
        .active_content(budget)
}

/// A one-page document: `catalog_extra` goes into the catalog, `page_extra` into the page,
/// and `more` are objects 4, 5, ...
fn document(catalog_extra: &str, page_extra: &str, more: &[&str]) -> Vec<String> {
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page_extra} >>"),
    ];
    objects.extend(more.iter().map(|body| (*body).to_owned()));
    objects
}

fn findings(report: &SecurityReport) -> Vec<(FindingKind, u32)> {
    report
        .findings
        .iter()
        .map(|SecurityFinding { kind, count }| (*kind, *count))
        .collect()
}

const JS: &str = "<< /S /JavaScript /JS (app.alert\\(1\\)) >>";

#[test]
fn an_ordinary_document_has_no_findings() {
    let report = scan(&document(
        "",
        "/Annots [4 0 R]",
        &["<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D [3 0 R /Fit] >> >>"],
    ));
    assert_eq!(
        report,
        SecurityReport {
            findings: vec![],
            scan_complete: true
        }
    );
}

#[test]
fn opening_at_a_page_is_not_an_open_action() {
    for open_action in [
        "/OpenAction [3 0 R /Fit]",
        "/OpenAction << /S /GoTo /D [3 0 R /Fit] >>",
    ] {
        assert_eq!(
            findings(&scan(&document(open_action, "", &[]))),
            [],
            "{open_action}"
        );
    }
    // A jump followed by something else runs that something.
    let chained = scan(&document(
        &format!("/OpenAction << /S /GoTo /D [3 0 R /Fit] /Next {JS} >>"),
        "",
        &[],
    ));
    assert_eq!(
        findings(&chained),
        [(FindingKind::JavaScript, 1), (FindingKind::OpenAction, 1)]
    );
}

#[test]
fn counts_each_script_and_each_trigger() {
    let report = scan(&document(
        &format!("/AA << /WC {JS} /WS 4 0 R >>"),
        &format!("/AA << /O {JS} /C {JS} >>"),
        &[JS],
    ));
    assert_eq!(
        findings(&report),
        [
            (FindingKind::JavaScript, 4),
            (FindingKind::AdditionalActions, 4)
        ]
    );
}

#[test]
fn a_script_in_another_action_counts() {
    let report = scan(&document(
        "/OpenAction << /S /Rendition /OP 0 /JS (app.alert\\(1\\)) >>",
        "",
        &[],
    ));
    assert_eq!(
        findings(&report),
        [(FindingKind::JavaScript, 1), (FindingKind::OpenAction, 1)]
    );
}

#[test]
fn a_shared_object_counts_once_and_cycles_end() {
    // Two links share one action; the action's /Next points back at itself.
    let report = scan(&document(
        "",
        "/Annots [4 0 R 5 0 R]",
        &[
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A 6 0 R >>",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A 6 0 R >>",
            "<< /S /Launch /F (calc.exe) /Next 6 0 R >>",
        ],
    ));
    assert_eq!(findings(&report), [(FindingKind::Launch, 1)]);
    assert!(report.scan_complete);
}

#[test]
fn network_paths_are_flagged_wherever_a_file_is_named() {
    // A UNC path as a remote document, a URL file spec as external image data, and a
    // file:// URL with a server.
    let report = scan(&document(
        "",
        "/Annots [4 0 R] /Resources << /XObject << /Im1 5 0 R >> >>",
        &[
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoToR /F << /Type /Filespec /F (\\\\\\\\server\\\\share\\\\a.pdf) >> /D [0 /Fit] >> >>",
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /F << /FS /URL /F (file://server/share/x.png) >> /Length 0 >>\nstream\n\nendstream",
        ],
    ));
    assert_eq!(
        findings(&report),
        [
            (FindingKind::RemoteGoTo, 1),
            (FindingKind::RemoteFileSpec, 1),
            (FindingKind::UncReference, 2),
        ]
    );
}

#[test]
fn a_submit_address_is_part_of_the_submit_action() {
    let report = scan(&document(
        "",
        "/Annots [4 0 R]",
        &[
            "<< /Type /Annot /Subtype /Widget /FT /Btn /Rect [0 0 10 10] /A << /S /SubmitForm /F << /FS /URL /F (https://collect.example.invalid/) >> >> >>",
        ],
    ));
    assert_eq!(findings(&report), [(FindingKind::SubmitForm, 1)]);
}

#[test]
fn media_forms_and_attachments() {
    let report = scan(&document(
        "/AcroForm << /Fields [] /XFA 5 0 R >>",
        "/Annots [4 0 R 6 0 R]",
        &[
            "<< /Type /Annot /Subtype /RichMedia /Rect [0 0 10 10] >>",
            "<< /Length 0 >>\nstream\n\nendstream",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [0 0 10 10] /FS << /Type /Filespec /F (a.txt) /EF << /F 7 0 R >> >> >>",
            "<< /Type /EmbeddedFile /Length 0 >>\nstream\n\nendstream",
        ],
    ));
    assert_eq!(
        findings(&report),
        [
            (FindingKind::Xfa, 1),
            (FindingKind::RichMedia, 1),
            (FindingKind::EmbeddedFile, 1),
        ]
    );
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    let depth = 100_000;
    let junk = format!("{}{JS}{}", "[".repeat(depth), "]".repeat(depth));
    let report = scan(&document("", &format!("/Junk {junk}"), &[]));
    // Whatever MuPDF's parser keeps of it is walked without recursion.
    assert!(report.scan_complete);
}

/// A page with `count` links that each run a script.
fn many_scripts(count: usize) -> Vec<String> {
    let annots: Vec<String> = (0..count).map(|i| format!("{} 0 R", i + 4)).collect();
    let bodies: Vec<String> = (0..count)
        .map(|_| format!("<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A {JS} >>"))
        .collect();
    let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();
    document("", &format!("/Annots [{}]", annots.join(" ")), &bodies)
}

#[test]
fn stops_at_the_object_budget_and_says_so() {
    let objects = many_scripts(50);
    let full = scan(&objects);
    assert_eq!(findings(&full), [(FindingKind::JavaScript, 50)]);
    assert!(full.scan_complete);

    let cut = scan_with(
        &objects,
        ScanBudget {
            max_objects: 20,
            max_time: Duration::from_secs(60),
        },
    );
    assert!(!cut.scan_complete);
    assert!(cut.findings.iter().all(|finding| finding.count < 50));
}

#[test]
fn stops_at_the_time_budget_and_says_so() {
    // Enough objects for the clock to be read (every 1024 objects).
    let objects = many_scripts(1500);
    let out_of_time = scan_with(
        &objects,
        ScanBudget {
            max_objects: usize::MAX,
            max_time: Duration::ZERO,
        },
    );
    assert!(!out_of_time.scan_complete);
    assert!(scan(&objects).scan_complete);
}

// --- The test corpus (tests/corpus, QA-01) ---

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn kind_name(kind: FindingKind) -> String {
    serde_json::to_value(kind)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// A sample of the corpus and the finding kinds its manifest expects.
struct Sample {
    path: String,
    file: PathBuf,
    expected: BTreeSet<String>,
}

/// The samples of `tests/corpus/manifest.json` that are present (large files are generated on
/// demand with `generate.py --large` and are skipped when missing).
fn samples() -> Vec<Sample> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus().join("manifest.json")).unwrap()).unwrap();
    manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let path = entry["path"].as_str().unwrap().to_owned();
            let expected = entry["findings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|kind| kind.as_str().unwrap().to_owned())
                .collect();
            let on_demand = entry["onDemand"].as_bool() == Some(true);
            (
                Sample {
                    file: corpus().join(&path),
                    path,
                    expected,
                },
                on_demand,
            )
        })
        .filter(|(sample, on_demand)| !on_demand || sample.file.exists())
        .map(|(sample, _)| sample)
        .collect()
}

fn kinds(report: &SecurityReport) -> BTreeSet<String> {
    report
        .findings
        .iter()
        .map(|finding| kind_name(finding.kind))
        .collect()
}

#[test]
fn every_corpus_sample_reports_the_findings_in_its_manifest() {
    let mut checked = 0;
    for sample in samples() {
        let path = &sample.path;
        let bytes = std::fs::read(&sample.file).unwrap_or_else(|error| panic!("{path}: {error}"));
        // Malformed and encrypted samples may not open at all; that is tested elsewhere.
        let Ok(document) = PdfDocument::from_bytes(&bytes) else {
            assert!(
                sample.expected.is_empty(),
                "{path} has findings but does not open"
            );
            continue;
        };
        let report = document.active_content(ScanBudget::default());
        assert_eq!(kinds(&report), sample.expected, "{path}");
        assert!(report.scan_complete, "{path}: scan incomplete");
        checked += 1;
    }
    assert!(checked >= 30, "only {checked} samples were checked");
}

/// The same through the real worker in its sandbox: the report crosses the IPC boundary and
/// passes its validation.
#[cfg(windows)]
#[test]
fn the_sandboxed_worker_reports_them_when_a_malicious_sample_opens() {
    use ipc_contract::worker::WorkerResponse;
    use worker_host::{HostConfig, WorkerHost};

    let worker = Path::new(env!("CARGO_BIN_EXE_pdf_worker"));
    let mut host = WorkerHost::new(worker, HostConfig::default());
    let mut checked = 0;
    for sample in samples()
        .into_iter()
        .filter(|sample| sample.path.starts_with("malicious/"))
    {
        let path = &sample.path;
        let (_, opened) = host
            .open(&sample.file)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        let WorkerResponse::Opened { document, .. } = opened else {
            panic!("{path}: {opened:?}");
        };
        assert_eq!(kinds(&document.security), sample.expected, "{path}");
        assert!(document.security.scan_complete, "{path}");
        checked += 1;
    }
    assert!(
        checked >= 21,
        "only {checked} malicious samples were checked"
    );
}
