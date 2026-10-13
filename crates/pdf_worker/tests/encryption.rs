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
fn a_request_for_a_password_says_what_a_remembered_one_is_looked_up_by() {
    // ADR 0017: the worker reports the key with the request for a password, and with nothing
    // else.
    let mut seen = Vec::new();
    for name in [
        "benign/encrypted-rc4-40.pdf",
        "benign/encrypted-aes256.pdf",
        "benign/restricted-open-password.pdf",
    ] {
        let path = corpus(name);
        let mut host = host();
        let Err(HostError::Worker(asked)) = host.open(&path) else {
            panic!("{name} did not ask for a password")
        };
        assert_eq!(asked.code, WorkerErrorCode::Encrypted, "{name}");
        let key = asked.password_key.expect(name);
        // The same file asks the same way again, and the key is what the library makes of it.
        let Err(HostError::Worker(again)) = host.open(&path) else {
            panic!("{name} did not ask again")
        };
        assert_eq!(again.password_key, Some(key), "{name}");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(pdf_worker::password_key::of(&bytes), Some(key), "{name}");
        // A wrong password is no request for one: it is refused, and says nothing of keys.
        let wrong = Password::new("wrong".to_owned());
        let Err(HostError::Worker(refused)) = host.open_with_password(&path, wrong) else {
            panic!("{name} accepted a wrong password")
        };
        assert_eq!(refused.code, WorkerErrorCode::WrongPassword, "{name}");
        assert_eq!(refused.password_key, None, "{name}");
        seen.push(key);
    }
    for (index, key) in seen.iter().enumerate() {
        assert!(!seen[..index].contains(key), "two files share a key");
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
    // /P -44) allows copying and printing, but not changing, annotating or filling in the document; the
    // restricted one allows neither copying nor printing.
    for (name, permissions) in [
        (
            "benign/encrypted-rc4-40.pdf",
            DocumentPermissions {
                modify: false,
                assemble: false,
                annotate: false,
                fill_forms: false,
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
