//! Attachments through the real worker in the real sandbox (B3-20, ADR 0025): listed with clean
//! names, written decoded to a handle, and nothing else of the file written.
#![cfg(windows)]

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::{AttachmentId, AttachmentSource, DocumentId, Rotation};
use ipc_contract::worker::{WorkerAttachment, WorkerErrorCode, WorkerRequest, WorkerResponse};
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

fn open(host: &mut WorkerHost, name: &str) -> DocumentId {
    match host.open(&corpus(name)).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("expected Opened, got {other:?}"),
    }
}

fn list(host: &mut WorkerHost, doc: DocumentId) -> Vec<WorkerAttachment> {
    match host.request(|request| WorkerRequest::GetAttachments { request, doc }) {
        Ok(WorkerResponse::Attachments { attachments, .. }) => attachments,
        other => panic!("expected Attachments, got {other:?}"),
    }
}

/// A new, empty file to write into, as the main process creates it.
fn new_file(name: &str) -> (PathBuf, File) {
    let path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-attachment-{name}",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    (path, file)
}

/// The worker is still there and still has the document.
fn still_serves(host: &mut WorkerHost, doc: DocumentId) {
    let rendered = host.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(
        matches!(rendered, Ok(WorkerResponse::Rendered { .. })),
        "{rendered:?}"
    );
}

#[test]
fn an_attachment_is_listed_and_written_decoded() {
    let mut host = host();
    let doc = open(&mut host, "benign/attachment-text.pdf");

    let attachments = list(&mut host, doc);
    assert_eq!(attachments.len(), 1, "/EF /F and /EF /UF name one stream");
    let attachment = &attachments[0];
    assert_eq!(attachment.name, "sample.txt");
    assert_eq!(attachment.size, Some(39));
    assert_eq!(
        attachment.description.as_deref(),
        Some("A plain text file attachment")
    );
    assert_eq!(attachment.source, AttachmentSource::Document);

    let (path, file) = new_file("text");
    let written = host
        .write_attachment(doc, attachment.id, &file)
        .expect("written");
    drop(file);
    assert!(
        matches!(written, WorkerResponse::AttachmentWritten { bytes: 39, .. }),
        "{written:?}"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"This is a benign text file attachment.\n"
    );
    fs::remove_file(&path).unwrap();
}

#[test]
fn untrusted_names_are_cleaned() {
    let mut host = host();
    for (sample, name) in [
        ("malicious/attachment-rlo.pdf", "invoiceexe.pdf"),
        ("malicious/attachment-path.pdf", "calc.exe"),
        ("malicious/attachment-device-name.pdf", "_CON.txt"),
        ("malicious/attachment-exe.pdf", "payload.exe"),
        ("benign/attachment-no-name.pdf", "附件 1"),
    ] {
        let doc = open(&mut host, sample);
        let attachments = list(&mut host, doc);
        assert_eq!(attachments.len(), 1, "{sample}");
        assert_eq!(attachments[0].name, name, "{sample}");
    }
}

#[test]
fn the_size_a_file_claims_is_only_shown() {
    let mut host = host();
    let doc = open(&mut host, "malicious/attachment-size-mismatch.pdf");
    let attachment = list(&mut host, doc).remove(0);
    assert_eq!(attachment.size, Some(100_000_000));

    let (path, file) = new_file("mismatch");
    let written = host
        .write_attachment(doc, attachment.id, &file)
        .expect("written");
    drop(file);
    assert!(
        matches!(written, WorkerResponse::AttachmentWritten { bytes: 31, .. }),
        "{written:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), b"actual payload is only 30 bytes");
    fs::remove_file(&path).unwrap();
}

#[test]
fn only_a_listed_attachment_is_written() {
    let mut host = host();
    let doc = open(&mut host, "benign/attachment-text.pdf");
    let listed = list(&mut host, doc)[0].id;

    // Other object numbers of the same file (its pages, its fonts, its content streams), and
    // numbers it does not have.
    let (path, file) = new_file("unlisted");
    for number in (1..40).chain([100_000, u32::MAX]) {
        if AttachmentId(number) == listed {
            continue;
        }
        match host.write_attachment(doc, AttachmentId(number), &file) {
            Err(HostError::Worker(error)) => {
                assert_eq!(error.code, WorkerErrorCode::InvalidRequest, "{number}")
            }
            other => panic!("object {number}: {other:?}"),
        }
    }
    drop(file);
    assert_eq!(fs::metadata(&path).unwrap().len(), 0, "nothing was written");
    fs::remove_file(&path).unwrap();
    still_serves(&mut host, doc);
}

#[test]
fn a_compression_bomb_is_refused_and_the_worker_stays() {
    let mut host = host();
    let doc = open(&mut host, "malicious/attachment-decompression-bomb.pdf");
    let attachment = list(&mut host, doc).remove(0);

    let (path, file) = new_file("bomb");
    match host.write_attachment(doc, attachment.id, &file) {
        Err(HostError::Worker(error)) => assert!(
            matches!(
                error.code,
                WorkerErrorCode::Corrupted | WorkerErrorCode::LimitExceeded
            ),
            "{error:?}"
        ),
        other => panic!("the bomb was not refused: {other:?}"),
    }
    drop(file);
    // The main process deletes its temporary file; the worker wrote nothing into it.
    assert_eq!(fs::metadata(&path).unwrap().len(), 0);
    fs::remove_file(&path).unwrap();
    still_serves(&mut host, doc);
}
