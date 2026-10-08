//! POC for ADR 0017 (proposed, #181): what a remembered password of an encrypted PDF can be found
//! by. The file says it itself, before any password is given: its password verifier (`/U`, with
//! its salts) sits unencrypted in the `/Encrypt` dictionary, which exists for exactly one
//! encryption of the file. Nothing has to be written into the PDF (ADR 0010 is not needed), and a
//! password found by it is the right one for that encryption, or the user is asked again.

use mupdf::Document;
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfWriteOptions};
use pdf_worker::engine::PdfDocument;

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What the file says of its encryption without being opened: the revision, the user password's
/// verifier (`/U`) and the file's identifier (`/ID`, first part).
fn says(bytes: &[u8]) -> (i32, Vec<u8>, Option<Vec<u8>>) {
    let doc = Document::from_bytes(bytes, "application/pdf").expect("opens as a document");
    assert!(doc.needs_password().expect("asks"), "it needs a password");
    let pdf = MuPdfDocument::try_from(doc).expect("a PDF");
    let trailer = pdf.trailer().expect("trailer");
    let encrypt = trailer
        .get_dict("Encrypt")
        .expect("encrypt")
        .expect("an Encrypt dictionary");
    let revision = encrypt.get_dict("R").unwrap().unwrap().as_int().unwrap();
    let verifier = encrypt.get_dict("U").unwrap().unwrap().as_bytes().unwrap();
    let id = trailer
        .get_dict("ID")
        .unwrap()
        .and_then(|ids| ids.get_array(0).unwrap())
        .map(|first| first.as_bytes().unwrap());
    (revision, verifier, id)
}

#[test]
fn the_verifier_and_the_id_can_be_read_before_the_password_is_given() {
    let (revision, verifier, id) = says(&corpus("benign/encrypted-aes256.pdf"));
    assert_eq!(revision, 6);
    assert_eq!(verifier.len(), 48, "hash, validation salt, key salt");
    assert_eq!(id.expect("an /ID").len(), 16);
    let (revision, verifier, id) = says(&corpus("benign/encrypted-rc4-40.pdf"));
    assert_eq!(revision, 2);
    assert_eq!(verifier.len(), 32);
    assert_eq!(id.expect("an /ID").len(), 16);
}

#[test]
fn the_verifier_is_what_the_app_saving_leaves_alone() {
    for name in ["benign/encrypted-aes256.pdf", "benign/encrypted-rc4-40.pdf"] {
        let original = corpus(name);
        let (revision, before, id_before) = says(&original);
        let mut doc = PdfDocument::open(&original, Some("user")).expect("open");
        doc.rotate_pages(&[0], 90).expect("rotate");
        let mut saved = Vec::new();
        doc.save(&mut saved).expect("save");
        assert_ne!(saved, original, "{name}");
        let (revision_after, after, id_after) = says(&saved);
        // The same encryption: the password remembered for the file still fits it.
        assert_eq!(revision, revision_after, "{name}");
        assert_eq!(before, after, "{name}");
        assert_eq!(id_before, id_after, "{name}");
    }
}

#[test]
fn another_encryption_of_the_same_document_has_another_verifier() {
    // The same document encrypted again: the salts are random, so the verifier is new even for
    // the same password, and a password remembered for one encryption is not offered for the
    // other (the user is asked, and may remember it).
    let plain = corpus("benign/multi-page-10.pdf");
    let write = |password: &str| {
        let doc = MuPdfDocument::from_bytes(&plain).expect("open");
        let mut options = PdfWriteOptions::default();
        options.set_encryption(mupdf::pdf::Encryption::Aes256);
        options.set_user_password(password);
        options.set_owner_password("some-owner-password");
        let mut bytes = Vec::new();
        doc.write_to_with_options(&mut bytes, options)
            .expect("write");
        bytes
    };
    let first = write("first-password");
    let again = write("first-password");
    let changed = write("another-password");
    let (_, first_verifier, first_id) = says(&first);
    let (_, again_verifier, again_id) = says(&again);
    let (_, changed_verifier, changed_id) = says(&changed);
    assert_ne!(first_verifier, again_verifier);
    assert_ne!(first_verifier, changed_verifier);
    // The file's /ID names the document, not its password: the three copies share it, so a
    // password remembered under it would be offered for the one it does not open.
    assert!(first_id.is_some());
    assert_eq!(first_id, again_id);
    assert_eq!(first_id, changed_id);
    assert!(PdfDocument::open(&first, Some("first-password")).is_ok());
    assert!(PdfDocument::open(&again, Some("first-password")).is_ok());
    assert!(PdfDocument::open(&changed, Some("first-password")).is_err());
    assert!(PdfDocument::open(&changed, Some("another-password")).is_ok());
}
