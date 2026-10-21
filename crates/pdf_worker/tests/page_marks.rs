//! A copy with page marks (B3-16), through the real worker in the real sandbox: the worker writes
//! it through a write-only handle, page numbers and Bates numbers are on the pages of the copy and
//! not of the open document, and what cannot be marked is refused while the worker goes on.
#![cfg(windows)]

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ipc_contract::types::{
    BatesMark, DocumentId, ErrorCode, HeaderFooterMarks, PageMarkPages, PageMarkPosition,
    PageMarks, PageNumberFormat, PageNumberMark, Rgb,
};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn opened(host: &mut WorkerHost, path: &Path) -> DocumentId {
    host.open(path).expect("open").0
}

/// A path in the temp directory no other test uses.
fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "pdf-worker-{}-page-marks-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn copy(
    host: &mut WorkerHost,
    doc: DocumentId,
    marks: PageMarks,
) -> Result<(PathBuf, Vec<u8>), HostError> {
    let path = temp_path("copy.pdf");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    let result = host.page_marks_copy(doc, &file, marks);
    drop(file);
    match result {
        Ok(WorkerResponse::Saved {
            bytes,
            incremental: false,
            ..
        }) => {
            let written = std::fs::read(&path).expect("the copy");
            assert_eq!(written.len() as u64, bytes);
            Ok((path, written))
        }
        Ok(other) => panic!("{other:?}"),
        Err(error) => {
            std::fs::remove_file(&path).ok();
            Err(error)
        }
    }
}

fn page_text(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> String {
    let response = host
        .request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index,
        })
        .expect("text");
    let WorkerResponse::PageText { text, .. } = response else {
        panic!("{response:?}");
    };
    text.lines
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn page_numbers_and_bates_numbers_on_every_page_through_real_sandbox() {
    let mut host = host();
    let original = corpus("benign/multi-page-10.pdf");
    let doc = opened(&mut host, &original);

    let marks = PageMarks {
        font_size: 12.0,
        color: Rgb { r: 0, g: 0, b: 0 },
        margin_pt: 36.0,
        pages: PageMarkPages::All,
        page_number: Some(PageNumberMark {
            format: PageNumberFormat::PageOfTotal,
            start: 1,
            position: PageMarkPosition::BottomCenter,
        }),
        bates: Some(BatesMark {
            prefix: "DEP-".to_string(),
            digits: 5,
            start: 100,
            position: PageMarkPosition::BottomRight,
        }),
        header_footer: Some(HeaderFooterMarks {
            header_left: Some("CONFIDENTIAL".to_string()),
            header_center: None,
            header_right: None,
            footer_left: None,
            footer_center: None,
            footer_right: None,
        }),
    };

    let (copy_path, bytes) = copy(&mut host, doc, marks).expect("copy with page marks");
    assert!(bytes.len() > 1000);

    // Verify copy has marks on pages 0 and 9
    let copy_doc = opened(&mut host, &copy_path);
    let p0 = page_text(&mut host, copy_doc, 0);
    assert!(p0.contains("第 1 頁／共 10 頁"), "p0 page num: {p0}");
    assert!(p0.contains("DEP-00100"), "p0 bates: {p0}");
    assert!(p0.contains("CONFIDENTIAL"), "p0 header: {p0}");

    let p9 = page_text(&mut host, copy_doc, 9);
    assert!(p9.contains("第 10 頁／共 10 頁"), "p9 page num: {p9}");
    assert!(p9.contains("DEP-00109"), "p9 bates: {p9}");
    assert!(p9.contains("CONFIDENTIAL"), "p9 header: {p9}");

    // Original document is untouched
    let orig_p0 = page_text(&mut host, doc, 0);
    assert!(!orig_p0.contains("DEP-00100"), "original p0: {orig_p0}");
    assert!(!orig_p0.contains("CONFIDENTIAL"), "original p0: {orig_p0}");

    std::fs::remove_file(&copy_path).ok();
}

#[test]
fn refused_operations_do_not_kill_worker() {
    let mut host = host();

    // 1. Encrypted document is refused
    let enc_path = corpus("benign/encrypted-rc4-40.pdf");
    let (enc_doc, _) = host
        .open_with_password(&enc_path, ipc_contract::types::Password::new("user".to_owned()))
        .expect("open");
    let marks = PageMarks {
        font_size: 12.0,
        color: Rgb { r: 0, g: 0, b: 0 },
        margin_pt: 36.0,
        pages: PageMarkPages::All,
        page_number: Some(PageNumberMark {
            format: PageNumberFormat::Number,
            start: 1,
            position: PageMarkPosition::BottomCenter,
        }),
        bates: None,
        header_footer: None,
    };

    let err = copy(&mut host, enc_doc, marks.clone()).unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidArgument);

    // 2. Bates overflow is refused
    let plain_path = corpus("benign/multi-page-10.pdf");
    let plain_doc = opened(&mut host, &plain_path);
    let overflow_marks = PageMarks {
        bates: Some(BatesMark {
            prefix: "TEST".to_string(),
            digits: 1,
            start: 1, // 10 pages with 1 digit -> overflow
            position: PageMarkPosition::BottomRight,
        }),
        ..marks
    };
    let err2 = copy(&mut host, plain_doc, overflow_marks).unwrap_err();
    assert_eq!(err2.code(), ErrorCode::InvalidArgument);

    // 3. Worker is still alive and serves subsequent requests
    assert!(host.is_running());
    let p0 = page_text(&mut host, plain_doc, 0);
    assert!(!p0.is_empty(), "worker still works");
}
