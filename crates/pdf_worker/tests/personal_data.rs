//! Finding personal data in the real worker and real sandbox (Issue #302).
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ipc_contract::types::{
    DocumentId, FindPersonalDataQuery, PersonalDataCategory, PersonalDataMatches,
};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

fn corpus(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(path)
}

fn open(path: &str) -> (WorkerHost, DocumentId) {
    let mut host = WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    );
    let doc = host
        .open(&corpus(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .0;
    (host, doc)
}

fn find(
    host: &mut WorkerHost,
    doc: DocumentId,
    query: FindPersonalDataQuery,
) -> PersonalDataMatches {
    match host
        .request(|request| WorkerRequest::FindPersonalData {
            request,
            doc,
            query: query.clone(),
        })
        .expect("find personal data")
    {
        WorkerResponse::PersonalDataMatches { matches, .. } => matches,
        other => panic!("unexpected response: {other:?}"),
    }
}

fn previews(found: &PersonalDataMatches, category: PersonalDataCategory) -> Vec<&str> {
    found
        .matches
        .iter()
        .filter(|m| m.category == category)
        .map(|m| m.preview.as_str())
        .collect()
}

#[test]
fn finds_valid_personal_data_and_skips_invalid_checksums() {
    let (mut host, doc) = open("benign/personal-data.pdf");
    let found = find(
        &mut host,
        doc,
        FindPersonalDataQuery {
            categories: vec![
                PersonalDataCategory::TaiwanId,
                PersonalDataCategory::MobilePhone,
                PersonalDataCategory::LandlinePhone,
                PersonalDataCategory::Email,
                PersonalDataCategory::CreditCard,
                PersonalDataCategory::UnifiedBusinessNumber,
            ],
            keyword: None,
        },
    );
    assert!(!found.has_more);
    assert!(found.matches.iter().all(|m| m.page == 0));

    // The wrapped ID on the last two lines is found as one match over both lines.
    assert_eq!(
        previews(&found, PersonalDataCategory::TaiwanId),
        ["A12****789", "A80****014", "A12****789"]
    );
    let wrapped = found
        .matches
        .iter()
        .rfind(|m| m.category == PersonalDataCategory::TaiwanId)
        .unwrap();
    assert_eq!(wrapped.rects.len(), 2);

    assert_eq!(
        previews(&found, PersonalDataCategory::MobilePhone),
        ["0912****78", "0987****21"]
    );
    // `0812-345-678` is too long for a Pingtung number.
    assert_eq!(
        previews(&found, PersonalDataCategory::LandlinePhone),
        ["(02)****78", "(04)****89"]
    );
    assert_eq!(
        previews(&found, PersonalDataCategory::Email),
        ["a***e@example.com"]
    );
    assert_eq!(
        previews(&found, PersonalDataCategory::CreditCard),
        ["4111 **** **** 1111"]
    );
    // The tail `1111-1112` of the card number that fails Luhn is not a business number.
    assert_eq!(
        previews(&found, PersonalDataCategory::UnifiedBusinessNumber),
        ["10****75", "04****52"]
    );

    // No preview carries a whole number or address.
    for whole in [
        "A123456789",
        "0912345678",
        "4111111111111111",
        "10458575",
        "alice.sample",
    ] {
        assert!(
            found
                .matches
                .iter()
                .all(|m| !m.preview.replace(['-', ' '], "").contains(whole)),
            "{whole}"
        );
    }
}

#[test]
fn finds_a_keyword_across_a_line_break_and_masks_it() {
    let (mut host, doc) = open("benign/personal-data.pdf");
    let found = find(
        &mut host,
        doc,
        FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::Keyword],
            keyword: Some("a123456789".to_owned()),
        },
    );
    assert_eq!(
        previews(&found, PersonalDataCategory::Keyword),
        ["A****9", "A****9"]
    );
    let rects: Vec<usize> = found.matches.iter().map(|m| m.rects.len()).collect();
    assert_eq!(rects, [1, 2]);
}
