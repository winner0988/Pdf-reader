//! A Word document of some pages (B3-02), through the real worker in the real sandbox: the worker
//! reads the pages it was opened with, makes the document in memory and writes it through a
//! write-only handle to a file the main process created. It never writes anything else.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::DocumentId;
use ipc_contract::worker::{WorkerErrorCode, WorkerResponse};
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

fn open(host: &mut WorkerHost, path: &Path) -> DocumentId {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    }
}

/// A new, empty file to write into, as the main process creates it.
fn new_file(name: &str) -> (PathBuf, File) {
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-{name}.docx", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    (path, file)
}

fn has(bytes: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn the_worker_writes_the_pages_as_a_word_document_through_its_handle_and_leaves_the_document_alone()
{
    let mut host = host();
    let doc = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (path, file) = new_file("pages");
    let response = host.export_docx(doc, &[3, 0], &file).expect("exported");
    drop(file);
    let WorkerResponse::Saved {
        bytes, incremental, ..
    } = response
    else {
        panic!("{response:?}");
    };
    assert!(!incremental);
    let written = std::fs::read(&path).expect("read");
    assert_eq!(written.len() as u64, bytes);
    // A ZIP file with its parts, and the words of the pages, in the order asked.
    assert_eq!(&written[..4], b"PK\x03\x04");
    for part in [
        "[Content_Types].xml",
        "word/document.xml",
        "word/styles.xml",
    ] {
        assert!(has(&written, part), "{part}");
    }
    let at = |text: &str| {
        written
            .windows(text.len())
            .position(|window| window == text.as_bytes())
            .unwrap_or_else(|| panic!("{text} is in the document"))
    };
    let (first, second) = (at("Page 4 of 10"), at("Page 1 of 10"));
    assert!(first < second, "page 4 was asked for first");

    // The document is where it was: another export of the same pages is the same size.
    let (again, second_file) = new_file("pages-again");
    let WorkerResponse::Saved {
        bytes: again_bytes, ..
    } = host
        .export_docx(doc, &[3, 0], &second_file)
        .expect("exported")
    else {
        panic!("a saved response");
    };
    assert_eq!(again_bytes, bytes);
    for file in [path, again] {
        let _ = std::fs::remove_file(file);
    }
}

#[test]
fn what_is_not_a_request_for_pages_is_refused_and_the_worker_goes_on() {
    let mut host = host();
    let doc = open(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (path, file) = new_file("refused");
    for (pages, code) in [
        (vec![], WorkerErrorCode::InvalidRequest),
        (vec![99], WorkerErrorCode::PageOutOfRange),
        (vec![1, 1], WorkerErrorCode::InvalidRequest),
    ] {
        match host.export_docx(doc, &pages, &file) {
            Err(HostError::Worker(error)) => assert_eq!(error.code, code, "{pages:?}"),
            other => panic!("{pages:?}: {other:?}"),
        }
    }
    // Nothing was written, and the worker is still there.
    assert_eq!(std::fs::metadata(&path).expect("file").len(), 0);
    assert!(matches!(
        host.export_docx(doc, &[0], &file),
        Ok(WorkerResponse::Saved { .. })
    ));
    let _ = std::fs::remove_file(path);
}
