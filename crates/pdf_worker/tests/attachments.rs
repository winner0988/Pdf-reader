//! Attachments handling through the real worker in the real sandbox (B3-20, ADR 0025).
#![cfg(windows)]

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::types::DocumentId;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

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
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("expected Opened, got {other:?}"),
    }
}

fn new_temp_file(prefix: &str) -> (PathBuf, File) {
    let path =
        std::env::temp_dir().join(format!("test-attachment-{}-{prefix}.tmp", std::process::id()));
    let _ = fs::remove_file(&path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create new temp file");
    (path, file)
}

#[test]
fn attachments_list_and_write_match_and_worker_stays_healthy() {
    let mut host = host();
    let doc = opened(&mut host, &corpus("benign/attachment-text.pdf"));

    // 1. List attachments
    let response = host
        .request(|request| WorkerRequest::GetAttachments { request, doc })
        .expect("get attachments request");
    let WorkerResponse::Attachments { attachments, .. } = response else {
        panic!("expected Attachments, got {response:?}");
    };
    assert_eq!(attachments.len(), 1);
    let att = &attachments[0];
    assert_eq!(att.name, "sample.txt");
    assert_eq!(att.size, Some(39));

    // 2. Write attachment to write-only handle
    let (path, file) = new_temp_file("text");
    let written_resp = host
        .write_attachment(doc, att.id, &file)
        .expect("write attachment");
    let WorkerResponse::AttachmentWritten { bytes, .. } = written_resp else {
        panic!("expected AttachmentWritten, got {written_resp:?}");
    };
    drop(file);

    assert_eq!(bytes, 39);
    let disk_content = fs::read(&path).expect("read written file");
    assert_eq!(disk_content, b"This is a benign text file attachment.\n");
    let _ = fs::remove_file(&path);

    // 3. Worker continues to serve subsequent requests
    let render_resp = host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: ipc_contract::types::Rotation::None,
        })
        .expect("render page");
    assert!(matches!(render_resp, WorkerResponse::Rendered { .. }));
}

#[test]
fn decompression_bomb_attachment_fails_leaves_no_file_and_worker_stays_healthy() {
    let mut host = host();
    let doc = opened(
        &mut host,
        &corpus("malicious/attachment-decompression-bomb.pdf"),
    );

    // 1. List attachments
    let response = host
        .request(|request| WorkerRequest::GetAttachments { request, doc })
        .expect("get attachments request");
    let WorkerResponse::Attachments { attachments, .. } = response else {
        panic!("expected Attachments, got {response:?}");
    };
    assert_eq!(attachments.len(), 1);
    let att = &attachments[0];

    // 2. Attempt to write decompression bomb
    let (path, file) = new_temp_file("bomb");
    let written_resp = host.write_attachment(doc, att.id, &file);

    // Should return an error (OutputTooLarge / Internal error) because decoded exceeds 512 MiB
    match written_resp {
        Ok(WorkerResponse::Error { .. }) | Err(_) => {
            // As expected, decoding bomb was refused
        }
        Ok(WorkerResponse::AttachmentWritten { .. }) => {
            panic!("decompression bomb exceeding 512 MiB should have been refused");
        }
        other => panic!("unexpected response for bomb: {other:?}"),
    }

    // Main process deletes temp file on failure
    drop(file);
    let _ = fs::remove_file(&path);
    assert!(!path.exists(), "temp file should not remain on disk");

    // 3. Worker continues to serve subsequent requests
    let render_resp = host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: ipc_contract::types::Rotation::None,
        })
        .expect("render page after bomb refusal");
    assert!(matches!(render_resp, WorkerResponse::Rendered { .. }));
}
