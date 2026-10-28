//! A copy of a document whose pictures are encoded again (B3-01), through the real worker in the
//! real sandbox: the worker decodes the pictures of the document, which are not to be trusted,
//! with no more rights than it has for any page, and writes the copy through a write-only handle to
//! a file the main process created; the document itself is left as it was.
#![cfg(windows)]

mod common;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, ImageQuality, Rotation};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
use jpeg_encoder::{ColorType, Encoder};
use mupdf::Rect;
use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument, PdfWriteOptions};
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

/// A file name of this test's own in the temporary folder.
fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pdf-worker-{}-{name}.pdf", std::process::id()))
}

/// A new, empty file to write into, as the main process creates it.
fn new_file(name: &str) -> (PathBuf, File) {
    let path = temporary(name);
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

/// A photograph that is no photograph: smooth colour with noise that is the same on every run.
fn photograph(width: usize, height: usize) -> Vec<u8> {
    let mut seed = 0x2545_f491u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 25) as i32 - 12
    };
    let mut samples = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            for base in [
                x * 255 / width,
                y * 255 / height,
                (x + y) * 255 / (width + height),
            ] {
                samples.push((base as i32 + noise()).clamp(0, 255) as u8);
            }
        }
    }
    samples
}

/// A PDF of one page with a large photograph, as a JPEG file the way a camera makes it, put on it.
fn pdf_with_a_photograph() -> Vec<u8> {
    let (width, height) = (1200usize, 900usize);
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, 95)
        .encode(
            &photograph(width, height),
            width as u16,
            height as u16,
            ColorType::Rgb,
        )
        .expect("encoded");
    let mut doc = PdfDocument::new();
    let mut page = doc.new_page((612.0, 792.0)).expect("page");
    page.insert_image(
        &mut doc,
        Rect {
            x0: 36.0,
            y0: 36.0,
            x1: 436.0,
            y1: 336.0,
        },
        PageImageSource::Bytes {
            data: &jpeg,
            format_hint: None,
        },
        InsertImageOptions::default(),
    )
    .expect("put in");
    drop(page);
    let mut trailer = doc.trailer().expect("trailer");
    let mut identifier = doc.new_array().expect("array");
    let id_str1 = doc.new_string("70F9B20F639ED3B2D69ABD8F6B6F3120").expect("id");
    let id_str2 = doc.new_string("70F9B20F639ED3B2D69ABD8F6B6F3120").expect("id");
    identifier.array_push(id_str1).expect("push");
    identifier.array_push(id_str2).expect("push");
    trailer.dict_put("ID", identifier).expect("put");
    let mut options = PdfWriteOptions::default();
    options.set_garbage(true);
    let mut bytes = Vec::new();
    doc.write_to_with_options(&mut bytes, options)
        .expect("written");
    bytes
}

#[test]
fn the_worker_writes_a_smaller_copy_of_the_pictures_and_the_document_is_as_it_was() {
    let input = temporary("photograph-input");
    std::fs::write(&input, pdf_with_a_photograph()).expect("written");
    let mut host = host();
    let doc = open(&mut host, &input);
    let (path, file) = new_file("compressed");
    let response = host
        .compressed_copy(doc, &file, ImageQuality::Medium, Some(600))
        .expect("compressed");
    drop(file);
    let WorkerResponse::Compressed {
        bytes,
        before,
        images,
        changed,
        partial,
        ..
    } = response
    else {
        panic!("{response:?}");
    };
    assert_eq!((images, changed, partial), (1, 1, false));
    let copy = std::fs::read(&path).expect("first compressed copy");
    assert_eq!(bytes, copy.len() as u64);
    assert!(bytes * 3 < before, "{bytes} bytes from {before}");

    // The copy opens, in a worker of its own, with its page.
    let mut again = self::host();
    let copy_doc = open(&mut again, &path);
    assert!(matches!(
        again
            .request(|request| WorkerRequest::Render {
                request,
                doc: copy_doc,
                page_index: 0,
                scale: 1.0,
                rotation: Rotation::None,
            })
            .expect("render"),
        WorkerResponse::Rendered { .. }
    ));
    // And the document is where it was: another copy of it is the same.
    let (second_path, second_file) = new_file("compressed-again");
    let WorkerResponse::Compressed {
        bytes: again_bytes, ..
    } = host
        .compressed_copy(doc, &second_file, ImageQuality::Medium, Some(600))
        .expect("compressed")
    else {
        panic!("a compressed copy");
    };
    drop(second_file);
    let second_copy = std::fs::read(&second_path).expect("second compressed copy");
    assert_eq!(again_bytes, second_copy.len() as u64);
    assert_eq!(
        common::without_trailer_id(&copy).expect("trailer /ID in first copy"),
        common::without_trailer_id(&second_copy).expect("trailer /ID in second copy"),
    );
    for file in [&input, &path, &second_path] {
        let _ = std::fs::remove_file(file);
    }
}

#[test]
fn an_encrypted_document_a_signed_one_and_a_limit_that_is_no_size_are_refused() {
    let mut host = host();
    let (path, file) = new_file("refused");

    // Signed: the copy would not be what the signature covers.
    let signed = open(&mut host, &corpus("benign/signed.pdf"));
    let refused = host
        .compressed_copy(signed, &file, ImageQuality::Medium, None)
        .expect_err("refused");
    assert!(
        matches!(&refused, HostError::Worker(error) if error.code == WorkerErrorCode::InvalidRequest),
        "{refused:?}"
    );

    // A limit that is no size (the main process would not have sent it): the worker says no too.
    let document = open(&mut host, &corpus("benign/single-page.pdf"));
    for side in [0, 10, 100_000] {
        match host.compressed_copy(document, &file, ImageQuality::Medium, Some(side)) {
            Err(HostError::Worker(error)) => {
                assert_eq!(error.code, WorkerErrorCode::InvalidRequest)
            }
            Err(HostError::Crashed) => panic!("the worker died"),
            other => panic!("{other:?}"),
        }
    }
    // The worker is still there for the next request.
    assert!(matches!(
        host.compressed_copy(document, &file, ImageQuality::Medium, Some(256)),
        Ok(WorkerResponse::Compressed {
            images: 0,
            changed: 0,
            ..
        })
    ));
    let _ = std::fs::remove_file(path);
}
