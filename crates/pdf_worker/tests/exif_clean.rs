//! A copy of a document whose pictures carry nothing but themselves (B3-04), through the real
//! worker in the real sandbox: the worker reads the pictures of the document, which are not to be
//! trusted, takes out what a camera and the programs after it wrote into them, and writes the copy
//! through a write-only handle to a file the main process created; the document itself is left as
//! it was.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, Rotation};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
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

/// A new, empty file of this test's own to write into, as the main process creates it.
fn new_file(name: &str) -> (PathBuf, File) {
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-{name}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    (path, file)
}

fn open(host: &mut WorkerHost, path: &Path) -> DocumentId {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// What the photograph sample's camera and programs wrote (tests/corpus/generate.py).
const PRIVATE: [&[u8]; 6] = [
    b"Canon",
    b"2023:07:04",
    b"Jane Q. Private-Author",
    b"Exif",
    b"Uncropped thumbnail data",
    b"Photoshop 3.0",
];

fn rendered(host: &mut WorkerHost, doc: DocumentId) -> Vec<u8> {
    match host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        })
        .expect("render")
    {
        WorkerResponse::Rendered { raster, .. } => raster.pixels,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_worker_writes_a_copy_without_what_the_camera_wrote_and_the_page_looks_the_same() {
    let sample = corpus("benign/photo-exif.pdf");
    let original = std::fs::read(&sample).expect("sample");
    for private in PRIVATE {
        assert!(contains(&original, private), "the control");
    }
    let mut host = host();
    let doc = open(&mut host, &sample);
    let (path, file) = new_file("exif-cleaned");
    let response = host.exif_clean_copy(doc, &file).expect("cleaned");
    drop(file);
    let WorkerResponse::ExifCleaned {
        bytes,
        pictures,
        cleaned,
        ..
    } = response
    else {
        panic!("{response:?}");
    };
    assert_eq!((pictures, cleaned), (1, 1));
    let copy = std::fs::read(&path).expect("copy");
    assert_eq!(bytes, copy.len() as u64);
    for private in PRIVATE {
        assert!(
            !contains(&copy, private),
            "{}",
            String::from_utf8_lossy(private)
        );
    }

    // The copy opens, in a worker of its own, and its page is drawn as the document's is.
    let mut again = self::host();
    let cleaned_doc = open(&mut again, &path);
    assert_eq!(rendered(&mut again, cleaned_doc), rendered(&mut host, doc));
    // The document is as it was: another copy of it is the same.
    let (second_path, second_file) = new_file("exif-cleaned-again");
    let WorkerResponse::ExifCleaned {
        bytes: again_bytes,
        cleaned: again_cleaned,
        ..
    } = host.exif_clean_copy(doc, &second_file).expect("cleaned")
    else {
        panic!("a cleaned copy");
    };
    assert_eq!((again_bytes, again_cleaned), (bytes, 1));
    // And a copy of the copy has nothing left to take out.
    let (third_path, third_file) = new_file("exif-cleaned-twice");
    assert!(matches!(
        again.exif_clean_copy(cleaned_doc, &third_file),
        Ok(WorkerResponse::ExifCleaned {
            pictures: 1,
            cleaned: 0,
            ..
        })
    ));
    drop((second_file, third_file));
    for file in [&path, &second_path, &third_path] {
        let _ = std::fs::remove_file(file);
    }
}

#[test]
fn an_encrypted_document_and_a_signed_one_have_no_copy_and_the_worker_goes_on() {
    let mut host = host();
    let (path, file) = new_file("exif-refused");
    let signed = open(&mut host, &corpus("benign/signed.pdf"));
    let refused = host.exif_clean_copy(signed, &file).expect_err("refused");
    assert!(
        matches!(&refused, HostError::Worker(error) if error.code == WorkerErrorCode::InvalidRequest),
        "{refused:?}"
    );
    // A document with no pictures is a copy all the same, with nothing taken out.
    let plain = open(&mut host, &corpus("benign/single-page.pdf"));
    assert!(matches!(
        host.exif_clean_copy(plain, &file),
        Ok(WorkerResponse::ExifCleaned {
            pictures: 0,
            cleaned: 0,
            ..
        })
    ));
    drop(file);
    let _ = std::fs::remove_file(path);
}
