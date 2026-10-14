//! A copy with a watermark (B3-04), through the real worker in the real sandbox: the worker writes
//! it through a write-only handle, the words are on the pages of the copy and not of the document,
//! a picture comes as a stamp picture's PNG, and what cannot be a watermark is refused while the
//! worker goes on.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ipc_contract::types::{
    DocumentId, ErrorCode, Rgb, Watermark, WatermarkContent, WatermarkPages, WatermarkPlace,
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

fn words(text: &str) -> Watermark {
    Watermark {
        content: WatermarkContent::Text {
            text: text.to_owned(),
            size: 40.0,
            color: Rgb { r: 200, g: 0, b: 0 },
        },
        opacity: 40,
        diagonal: true,
        place: WatermarkPlace::Center,
        pages: WatermarkPages::All,
    }
}

/// A path in the temp directory no other test uses.
fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "pdf-worker-{}-watermark-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// What the worker writes for `doc` with `watermark`: the file's bytes and where it is.
fn copy(
    host: &mut WorkerHost,
    doc: DocumentId,
    watermark: Watermark,
    picture: Option<Vec<u8>>,
) -> Result<(PathBuf, Vec<u8>), HostError> {
    let path = temp_path("copy.pdf");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    let result = host.watermark_copy(doc, &file, watermark, picture);
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

fn first_page_text(host: &mut WorkerHost, path: &Path) -> String {
    let doc = opened(host, path);
    let response = host
        .request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index: 0,
        })
        .expect("text");
    let WorkerResponse::PageText { text, .. } = response else {
        panic!("{response:?}");
    };
    text.lines
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_copy_has_the_words_on_its_pages_and_the_document_has_none() {
    let mut host = host();
    let doc = opened(&mut host, &corpus("benign/multi-page-10.pdf"));
    let (path, bytes) = copy(&mut host, doc, words("機密 CONFIDENTIAL"), None).expect("copy");
    assert!(bytes.starts_with(b"%PDF-"));
    let text = first_page_text(&mut host, &path);
    assert!(
        text.contains("CONFIDENTIAL") && text.contains("機密"),
        "{text}"
    );
    std::fs::remove_file(&path).ok();
    // The document the copy was made of is as it was.
    let original = first_page_text(&mut host, &corpus("benign/multi-page-10.pdf"));
    assert!(!original.contains("CONFIDENTIAL"));
}

#[test]
fn a_picture_watermark_is_a_stamp_picture_and_anything_else_is_refused() {
    let mut host = host();
    let file = File::open(corpus("images/stamp-exif.jpg")).expect("the picture");
    let WorkerResponse::StampImage { png, .. } = host.prepare_stamp_image(&file).expect("stamp")
    else {
        panic!("a stamp picture");
    };
    let doc = opened(&mut host, &corpus("benign/single-page.pdf"));
    let picture = Watermark {
        content: WatermarkContent::Picture { width_pt: 150.0 },
        ..words("unused")
    };
    let (path, _) = copy(&mut host, doc, picture.clone(), Some(png.clone())).expect("copy");
    std::fs::remove_file(&path).ok();

    // Not a PNG; words with a picture; a picture without one.
    for (watermark, bytes) in [
        (picture.clone(), Some(b"not a png at all".to_vec())),
        (words("DRAFT"), Some(png.clone())),
        (picture, None),
    ] {
        let refused = copy(&mut host, doc, watermark, bytes).expect_err("refused");
        assert_eq!(refused.code(), ErrorCode::InvalidArgument, "{refused}");
        assert!(host.is_running());
    }
    // The worker goes on.
    let (path, _) = copy(&mut host, doc, words("DRAFT"), None).expect("copy");
    std::fs::remove_file(&path).ok();
}

#[test]
fn an_encrypted_document_has_no_copy_with_a_watermark() {
    let mut host = host();
    let doc = match host.open_with_password(
        &corpus("benign/encrypted-rc4-40.pdf"),
        ipc_contract::types::Password::new("user".to_owned()),
    ) {
        Ok((doc, _)) => doc,
        Err(error) => panic!("{error}"),
    };
    let refused = copy(&mut host, doc, words("DRAFT"), None).expect_err("refused");
    assert_eq!(refused.code(), ErrorCode::InvalidArgument);
    assert!(host.is_running());
}
