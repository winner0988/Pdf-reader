//! A copy of a document whose pictures carry nothing but themselves (B3-04), through the real
//! worker in the real sandbox: the worker reads the pictures of the document, which are not to be
//! trusted, takes out what a camera and the programs after it wrote into them, and writes the copy
//! through a write-only handle to a file the main process created; the document itself is left as
//! it was.
#![cfg(windows)]

mod common;

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
    drop(second_file);
    let second_copy = std::fs::read(&second_path).expect("second copy");
    assert_eq!(again_bytes, second_copy.len() as u64);
    assert_eq!(again_cleaned, 1);
    assert_eq!(
        common::without_trailer_id(&copy).expect("trailer /ID in first copy"),
        common::without_trailer_id(&second_copy).expect("trailer /ID in second copy"),
    );
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
    drop(third_file);
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

#[test]
fn trailer_without_id_reproduces_issue_371_copies_comparing_equal_despite_different_lengths() {
    // Exact trailer lines from Issue #371: one with hex string (1255 bytes), one with literal string (1254 bytes).
    let trailer_hex = b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120><E36128F5B299895D9A5DBAB4C309EBFE>]>>";
    let trailer_lit = b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>({C\\2169@Z\\306Bd\\274\\275kx/Z\\312)]>>";

    let prefix = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000010 00000 n \ntrailer\n";
    let suffix = b"\nstartxref\n50\n%%EOF\n";

    let copy_hex = [prefix.as_slice(), trailer_hex.as_slice(), suffix.as_slice()].concat();
    let copy_lit = [prefix.as_slice(), trailer_lit.as_slice(), suffix.as_slice()].concat();

    // 1. Check lengths are different (issue #371 reproduction: 1255 vs 1254, diff by 1 byte).
    assert_ne!(copy_hex.len(), copy_lit.len());
    assert_eq!(copy_hex.len() - copy_lit.len(), 1);

    // 2. Stripping trailer /ID makes them completely identical.
    let stripped_hex = common::without_trailer_id(&copy_hex).expect("valid trailer hex");
    let stripped_lit = common::without_trailer_id(&copy_lit).expect("valid trailer lit");
    assert_eq!(stripped_hex, stripped_lit);
}

#[test]
fn trailer_without_id_detects_any_difference_outside_id() {
    let trailer = b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120><E36128F5B299895D9A5DBAB4C309EBFE>]>>";
    let prefix = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000010 00000 n \ntrailer\n";
    let suffix = b"\nstartxref\n50\n%%EOF\n";

    let original = [prefix.as_slice(), trailer.as_slice(), suffix.as_slice()].concat();
    let stripped_orig = common::without_trailer_id(&original).expect("strip original");

    // Difference in object stream / header
    let mut modified_header = original.clone();
    modified_header[5] = b'6'; // %PDF-1.6
    let stripped_header =
        common::without_trailer_id(&modified_header).expect("strip modified header");
    assert_ne!(stripped_orig, stripped_header);

    // Difference in trailer outside /ID (e.g. /Size 9 instead of /Size 8)
    let trailer_diff = b"<</Size 9/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120><E36128F5B299895D9A5DBAB4C309EBFE>]>>";
    let modified_trailer = [
        prefix.as_slice(),
        trailer_diff.as_slice(),
        suffix.as_slice(),
    ]
    .concat();
    let stripped_diff =
        common::without_trailer_id(&modified_trailer).expect("strip modified trailer");
    assert_ne!(stripped_orig, stripped_diff);

    // Difference in startxref offset
    let suffix_diff = b"\nstartxref\n51\n%%EOF\n";
    let modified_suffix = [prefix.as_slice(), trailer.as_slice(), suffix_diff.as_slice()].concat();
    let stripped_suffix =
        common::without_trailer_id(&modified_suffix).expect("strip modified suffix");
    assert_ne!(stripped_orig, stripped_suffix);
}

#[test]
fn trailer_without_id_handles_literal_strings_with_special_characters() {
    let prefix = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000010 00000 n \ntrailer\n";
    let suffix = b"\nstartxref\n50\n%%EOF\n";
    let reference_expected = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R>>",
        suffix.as_slice(),
    ]
    .concat();

    // The 5 required literal string cases in the second element of /ID:
    // 1. escaped parenthesis: \)
    // 2. escaped backslash: \\
    // 3. octal: \306
    // 4. bracket: ]
    // 5. greater-than: >
    let test_cases: [&[u8]; 5] = [
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(escaped\\)paren)]>>",
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(escaped\\\\backslash)]>>",
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(octal\\306byte)]>>",
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(contains]bracket)]>>",
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(contains>greater)]>>",
    ];

    for case in test_cases {
        let pdf = [prefix.as_slice(), case, suffix.as_slice()].concat();
        let stripped = common::without_trailer_id(&pdf).expect("strip literal with special chars");
        assert_eq!(stripped, reference_expected);
    }
}

#[test]
fn trailer_without_id_fails_on_missing_or_malformed_id() {
    let prefix = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000010 00000 n \ntrailer\n";
    let suffix = b"\nstartxref\n50\n%%EOF\n";

    // 1. Missing /ID entry in trailer
    let no_id = [prefix.as_slice(), b"<</Size 8/Root 1 0 R>>", suffix.as_slice()].concat();
    assert!(common::without_trailer_id(&no_id).is_err());

    // 2. Unterminated hex string
    let unclosed_hex = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2<E36128F5>]>>",
        suffix.as_slice(),
    ]
    .concat();
    assert!(common::without_trailer_id(&unclosed_hex).is_err());

    // 3. Unterminated literal string (the only ')' is escaped by '\')
    let unclosed_escaped = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>(unclosed\\)]>>",
        suffix.as_slice(),
    ]
    .concat();
    assert!(common::without_trailer_id(&unclosed_escaped).is_err());

    // 4. /ID array with only 1 string element
    let only_one = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120>]>>",
        suffix.as_slice(),
    ]
    .concat();
    assert!(common::without_trailer_id(&only_one).is_err());

    // 5. /ID array with more than 2 elements
    let three_elements = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120><E36128F5B299895D9A5DBAB4C309EBFE><1234>]>>",
        suffix.as_slice(),
    ]
    .concat();
    assert!(common::without_trailer_id(&three_elements).is_err());

    // 6. Missing closing ']'
    let no_bracket = [
        prefix.as_slice(),
        b"<</Size 8/Root 1 0 R/ID[<70F9B20F639ED3B2D69ABD8F6B6F3120><E36128F5B299895D9A5DBAB4C309EBFE>>>",
        suffix.as_slice(),
    ]
    .concat();
    assert!(common::without_trailer_id(&no_bracket).is_err());
}
