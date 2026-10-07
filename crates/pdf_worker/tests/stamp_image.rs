//! A picture for a custom stamp, through the real worker in the real sandbox (B2-08): the worker
//! reads it through a read-only handle (never a path), decodes it, and gives back the pixels
//! alone as a PNG; that PNG then becomes a stamp of a document, which is saved. Nothing a camera
//! wrote besides the pixels reaches the saved file.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::limits::MAX_STAMP_SIDE_PX;
use ipc_contract::types::{AnnotationKind, DocumentId, Rect};
use ipc_contract::validate::stamp_png_size;
use ipc_contract::worker::{WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse};
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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// What the worker makes of the picture in the corpus file `name`.
fn prepared(host: &mut WorkerHost, name: &str) -> Result<(Vec<u8>, u32, u32), HostError> {
    let file = File::open(corpus(name)).expect("the sample");
    match host.prepare_stamp_image(&file)? {
        WorkerResponse::StampImage {
            png, width, height, ..
        } => Ok((png, width, height)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_picture_with_exif_becomes_a_stamp_in_a_saved_document_without_it() {
    let mut host = host();
    let (png, width, height) = prepared(&mut host, "images/stamp-exif.jpg").expect("prepared");
    assert_eq!((width, height), (64, 32));
    assert_eq!(stamp_png_size(&png), Ok((64, 32)));
    assert!(png.len() < 20_000);
    for private in [&b"Canon"[..], b"2023:07:04", b"Exif"] {
        assert!(!contains(&png, private));
    }

    let (doc, _) = match host.open(&corpus("benign/single-page.pdf")).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    };
    let stamped = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddImageStamp {
                page: 0,
                rect: Rect {
                    x0: 100.0,
                    y0: 100.0,
                    x1: 228.0,
                    y1: 164.0,
                },
                png: png.clone(),
            },
        })
        .expect("the stamp");
    assert!(
        matches!(stamped, WorkerResponse::Edited { .. }),
        "{stamped:?}"
    );
    let listed = host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index: 0,
        })
        .expect("annotations");
    let WorkerResponse::PageAnnotations { annotations, .. } = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].kind, AnnotationKind::Stamp);

    let path = std::env::temp_dir().join(format!("pdf-worker-{}-stamp.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    host.save(doc, &file).expect("save");
    drop(file);
    let saved = std::fs::read(&path).expect("saved file");
    std::fs::remove_file(&path).ok();
    assert!(saved.starts_with(b"%PDF"));
    for private in [&b"Canon"[..], b"2023:07:04", b"Exif", b"Mark IV"] {
        assert!(!contains(&saved, private));
    }
}

#[test]
fn a_png_picture_is_made_too_without_its_chunks() {
    let mut host = host();
    let (png, width, height) = prepared(&mut host, "images/stamp-metadata.png").expect("prepared");
    assert_eq!((width, height), (64, 32));
    assert!(!contains(&png, b"Private-Author"));
    assert!(!contains(&png, b"eXIf"));
    assert!(!contains(&png, b"tEXt"));
    assert!(width.max(height) <= MAX_STAMP_SIDE_PX);
}

#[test]
fn what_is_not_a_usable_picture_is_refused_and_the_worker_goes_on() {
    let mut host = host();
    // A PNG of 60000 x 60000 pixels, said in its header: refused, not decoded.
    let huge = prepared(&mut host, "images/stamp-huge-dimensions.png");
    assert!(huge.is_err(), "{huge:?}");
    assert!(host.is_running());
    // A document is no picture.
    let not_a_picture = prepared(&mut host, "benign/single-page.pdf");
    assert!(not_a_picture.is_err(), "{not_a_picture:?}");
    // The worker still serves.
    assert!(prepared(&mut host, "images/stamp-exif.jpg").is_ok());
    // The error the host gives is the worker's code for it.
    let error = prepared(&mut host, "benign/single-page.pdf").unwrap_err();
    assert!(
        matches!(
            error,
            HostError::Worker(ref worker) if worker.code == WorkerErrorCode::InvalidRequest
        ),
        "{error:?}"
    );
}

#[test]
fn a_stamp_edit_with_something_else_than_a_stamp_picture_is_refused() {
    let mut host = host();
    let (doc, _): (DocumentId, _) =
        match host.open(&corpus("benign/single-page.pdf")).expect("open") {
            (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
            other => panic!("{other:?}"),
        };
    let jpeg = std::fs::read(corpus("images/stamp-exif.jpg")).expect("the sample");
    for wrong in [Vec::new(), jpeg] {
        let refused = host.request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddImageStamp {
                page: 0,
                rect: Rect {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 90.0,
                    y1: 50.0,
                },
                png: wrong.clone(),
            },
        });
        assert!(refused.is_err(), "{refused:?}");
    }
    assert!(host.is_running());
}
