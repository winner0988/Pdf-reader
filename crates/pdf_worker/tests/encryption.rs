//! Encrypted documents (MVP-16) through the real worker in its sandbox: without a password the
//! worker asks for one, a wrong one is refused, and the user or owner password opens the file.
//! Documents that restrict copying or printing report it (MVP-19), unless the owner password
//! opened them (#88).
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ipc_contract::types::{DocumentId, DocumentPermissions, Password};
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn corpus(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(path)
}

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn worker_error(result: Result<(DocumentId, WorkerResponse), HostError>) -> WorkerErrorCode {
    match result {
        Err(HostError::Worker(error)) => error.code,
        other => panic!("{other:?}"),
    }
}

#[test]
fn encrypted_samples_ask_for_a_password_and_open_with_it() {
    for (name, text) in [
        ("benign/encrypted-rc4-40.pdf", "Encrypted sample"),
        ("benign/encrypted-aes256.pdf", "Encrypted sample, AES-256"),
        (
            "benign/restricted-open-password.pdf",
            "Restricted sample with an open password",
        ),
    ] {
        let path = corpus(name);
        let mut host = host();
        assert_eq!(
            worker_error(host.open(&path)),
            WorkerErrorCode::Encrypted,
            "{name}"
        );
        let wrong = Password::new("wrong".to_owned());
        assert_eq!(
            worker_error(host.open_with_password(&path, wrong)),
            WorkerErrorCode::WrongPassword,
            "{name}"
        );

        for password in ["user", "owner"] {
            let (doc, response) = host
                .open_with_password(&path, Password::new(password.to_owned()))
                .unwrap_or_else(|error| panic!("{name} with {password:?}: {error}"));
            assert!(matches!(response, WorkerResponse::Opened { .. }), "{name}");
            // The same worker keeps serving the opened document: its text is readable.
            let response = host
                .request(|request| WorkerRequest::GetPageText {
                    request,
                    doc,
                    page_index: 0,
                })
                .unwrap();
            let WorkerResponse::PageText { text: page, .. } = response else {
                panic!("{name}: {response:?}")
            };
            assert!(
                page.lines.iter().any(|line| line.text.contains(text)),
                "{name} with {password:?}: {page:?}"
            );
        }
    }
}

#[test]
fn a_document_without_encryption_ignores_a_password() {
    let mut host = host();
    let (_, response) = host
        .open_with_password(
            &corpus("benign/single-page.pdf"),
            Password::new("unused".to_owned()),
        )
        .unwrap();
    assert!(matches!(response, WorkerResponse::Opened { .. }));
}

fn opened_permissions(
    result: Result<(DocumentId, WorkerResponse), HostError>,
) -> DocumentPermissions {
    match result {
        Ok((_, WorkerResponse::Opened { document, .. })) => document.permissions,
        other => panic!("{other:?}"),
    }
}

#[test]
fn restricted_samples_open_without_a_password_and_report_their_permissions() {
    for (name, permissions) in [
        (
            "benign/restricted-no-copy-no-print.pdf",
            DocumentPermissions {
                copy: false,
                print: false,
                print_high_quality: false,
                ..DocumentPermissions::ALL
            },
        ),
        (
            "benign/restricted-low-res-print.pdf",
            DocumentPermissions {
                print_high_quality: false,
                ..DocumentPermissions::ALL
            },
        ),
        ("benign/single-page.pdf", DocumentPermissions::ALL),
    ] {
        assert_eq!(
            opened_permissions(host().open(&corpus(name))),
            permissions,
            "{name}"
        );
    }
    // Opened with the user password: the AES sample allows everything; the RC4 one (revision 2,
    // /P -44) allows copying and printing, but not changing or annotating the document; the
    // restricted one allows neither copying nor printing.
    for (name, permissions) in [
        (
            "benign/encrypted-rc4-40.pdf",
            DocumentPermissions {
                modify: false,
                assemble: false,
                annotate: false,
                ..DocumentPermissions::ALL
            },
        ),
        ("benign/encrypted-aes256.pdf", DocumentPermissions::ALL),
        (
            "benign/restricted-open-password.pdf",
            DocumentPermissions {
                copy: false,
                print: false,
                print_high_quality: false,
                ..DocumentPermissions::ALL
            },
        ),
    ] {
        let password = Password::new("user".to_owned());
        assert_eq!(
            opened_permissions(host().open_with_password(&corpus(name), password)),
            permissions,
            "{name}"
        );
    }
}

/// As in Acrobat, the owner password lifts the author's restrictions (#88): the worker asks MuPDF
/// which password worked, in its sandbox.
#[test]
fn the_owner_password_lifts_the_authors_restrictions() {
    for name in [
        "benign/restricted-open-password.pdf",
        "benign/encrypted-rc4-40.pdf",
    ] {
        let password = Password::new("owner".to_owned());
        assert_eq!(
            opened_permissions(host().open_with_password(&corpus(name), password)),
            DocumentPermissions::ALL,
            "{name}"
        );
    }
}
