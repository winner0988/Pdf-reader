//! Integration tests for worker protocol violation and timeout handling using fake_worker (Issue #422).
#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use ipc_contract::types::{AttachmentId, DocumentId};
use ipc_contract::worker::{FileHandle, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn fake_worker_path() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_fake_worker"))
}

fn test_host() -> WorkerHost {
    let config = HostConfig {
        request_timeout: Duration::from_millis(200),
        handshake_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    WorkerHost::new(fake_worker_path(), config)
}

#[test]
fn corrupted_frame_terminates_worker_and_recovers() {
    let mut host = test_host();

    let err = host.inspect_font(b"corrupted_frame").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    // The next request must start a fresh worker and succeed
    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn validation_failure_terminates_worker_and_recovers() {
    let mut host = test_host();

    let err = host.inspect_font(b"validation_failure").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn request_id_zero_terminates_worker_and_recovers() {
    let mut host = test_host();

    let err = host.inspect_font(b"zero_id").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn future_request_id_terminates_worker_and_recovers() {
    let mut host = test_host();

    let err = host.inspect_font(b"future_id").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn delayed_wrong_request_id_cannot_extend_deadline_and_terminates_worker() {
    let mut host = test_host();

    let err = host.inspect_font(b"delayed_wrong_id").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_) | HostError::Timeout),
        "expected ProtocolViolation or Timeout, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn wrong_response_variant_terminates_worker_and_recovers() {
    let mut host = test_host();

    // WriteAttachment expects AttachmentWritten, but fake_worker sends Saved
    let err = host
        .request(|request| WorkerRequest::WriteAttachment {
            request,
            doc: DocumentId(1),
            attachment: AttachmentId(1),
            file: FileHandle(0),
        })
        .unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn second_hello_terminates_worker_and_recovers() {
    let mut host = test_host();

    let err = host.inspect_font(b"second_hello").unwrap_err();
    assert!(
        matches!(err, HostError::ProtocolViolation(_)),
        "expected ProtocolViolation, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}

#[test]
fn slow_request_times_out_and_terminates_worker() {
    let mut host = test_host();

    let err = host.inspect_font(b"slow").unwrap_err();
    assert!(
        matches!(err, HostError::Timeout),
        "expected Timeout, got {err:?}"
    );
    assert!(!host.is_running(), "worker connection must be dropped");

    let response = host.inspect_font(b"ok").unwrap();
    assert!(matches!(response, WorkerResponse::FontInspected { .. }));
    assert!(host.is_running());
}
