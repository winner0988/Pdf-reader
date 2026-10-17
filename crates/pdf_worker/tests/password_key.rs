//! The key a remembered password of an encrypted PDF is looked up by (ADR 0017, B2-23): made
//! from what the file says of its encryption before any password is given. The file's password
//! verifier (`/U`, with its salts) sits unencrypted in the `/Encrypt` dictionary, which exists
//! for exactly one encryption of the file. Nothing has to be written into the PDF (ADR 0010 is
//! not needed), and a password found by the key is the right one for that encryption, or the
//! user is asked again.

use mupdf::Document;
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfWriteOptions};
use pdf_worker::engine::PdfDocument;
use pdf_worker::password_key;

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
fn the_key_is_made_before_the_password_is_given_and_is_the_hash_of_what_the_file_says() {
    let aes = corpus("benign/encrypted-aes256.pdf");
    let (revision, verifier, id) = says(&aes);
    assert_eq!(revision, 6);
    assert_eq!(verifier.len(), 48, "hash, validation salt, key salt");
    assert_eq!(id.expect("an /ID").len(), 16);
    let aes_key = password_key::of(&aes).expect("a key");
    assert_eq!(Some(aes_key), password_key::derive(revision, &verifier));

    let rc4 = corpus("benign/encrypted-rc4-40.pdf");
    let (revision, verifier, id) = says(&rc4);
    assert_eq!(revision, 2);
    assert_eq!(verifier.len(), 32);
    assert_eq!(id.expect("an /ID").len(), 16);
    let rc4_key = password_key::of(&rc4).expect("a key");
    assert_eq!(Some(rc4_key), password_key::derive(revision, &verifier));

    assert_ne!(aes_key, rc4_key);
    // The password-protected sample of the permissions tests is another file again.
    let restricted = corpus("benign/restricted-open-password.pdf");
    let restricted_key = password_key::of(&restricted).expect("a key");
    assert_ne!(restricted_key, aes_key);
    assert_ne!(restricted_key, rc4_key);
    // And it is the same one every time.
    assert_eq!(password_key::of(&aes), Some(aes_key));
}

#[test]
fn the_key_is_what_the_app_saving_leaves_alone() {
    for name in ["benign/encrypted-aes256.pdf", "benign/encrypted-rc4-40.pdf"] {
        let original = corpus(name);
        let (revision, before, id_before) = says(&original);
        let key = password_key::of(&original).expect("a key");
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
        assert_eq!(password_key::of(&saved), Some(key), "{name}");
    }
}

#[test]
fn another_encryption_of_the_same_document_has_another_key() {
    // The same document encrypted again: the salts are random, so the verifier is new even for
    // the same password, and a password remembered for one encryption is not offered for the
    // other (the user is asked, and may remember it).
    let plain = corpus("benign/multi-page-10.pdf");
    assert_eq!(password_key::of(&plain), None, "not encrypted: no key");
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
    let keys = [&first, &again, &changed].map(|bytes| password_key::of(bytes).expect("a key"));
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[0], keys[2]);
    assert_ne!(keys[1], keys[2]);
    assert!(PdfDocument::open(&first, Some("first-password")).is_ok());
    assert!(PdfDocument::open(&again, Some("first-password")).is_ok());
    assert!(PdfDocument::open(&changed, Some("first-password")).is_err());
    assert!(PdfDocument::open(&changed, Some("another-password")).is_ok());
}

/// A file with no cross-reference table (MuPDF rebuilds one) whose `/Encrypt` dictionary is
/// `encrypt`.
fn with_encrypt(encrypt: &str) -> Vec<u8> {
    let id = "<00112233445566778899aabbccddeeff>";
    format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n\
         3 0 obj\n<< {encrypt} >>\nendobj\n\
         trailer\n<< /Size 4 /Root 1 0 R /Encrypt 3 0 R /ID [{id} {id}] >>\n%%EOF\n"
    )
    .into_bytes()
}

fn hex(length: usize) -> String {
    format!("<{}>", "ab".repeat(length))
}

#[test]
fn a_file_says_its_key_whatever_made_it_and_a_file_that_does_not_has_none() {
    let standard = |revision: &str, verifier: &str| {
        with_encrypt(&format!(
            "/Filter /Standard /V 1 /R {revision} /O {} /U {verifier} /P -4",
            hex(32)
        ))
    };
    // What a well-formed dictionary says is what the key is made of.
    let bytes = standard("2", &hex(32));
    let key = password_key::of(&bytes).expect("a key");
    assert_eq!(Some(key), password_key::derive(2, &[0xab; 32]));

    // Everything else has no key, and nothing panics or hangs.
    let none = [
        standard("2", &hex(31)),
        standard("2", "(short)"),
        standard("2", "/NotAString"),
        standard("2", "42"),
        standard("1", &hex(32)),
        standard("7", &hex(48)),
        standard("(two)", &hex(32)),
        with_encrypt(&format!("/Filter /Standard /V 1 /R 2 /O {} /P -4", hex(32))),
        with_encrypt("/Filter /Standard /V 1 /U <00> /P -4"),
        with_encrypt(""),
        with_encrypt("/Filter /Adobe.PubSec /SubFilter /adbe.pkcs7.s5 /V 4"),
        with_encrypt(&format!(
            "/Filter /Adobe.PubSec /V 4 /R 4 /O {} /U {} /P -4",
            hex(32),
            hex(32)
        )),
        with_encrypt(&format!(
            "/Filter /CustomHandler /V 2 /R 2 /O {} /U {} /P -4",
            hex(32),
            hex(32)
        )),
        with_encrypt(&format!("/V 2 /R 2 /O {} /U {} /P -4", hex(32), hex(32))),
        with_encrypt(&format!(
            "/Filter 12345 /V 2 /R 2 /O {} /U {} /P -4",
            hex(32),
            hex(32)
        )),
        corpus("benign/single-page.pdf"),
        corpus("malformed/not-a-pdf.pdf"),
        corpus("malformed/page-tree-cycle.pdf"),
    ];
    for (index, bytes) in none.iter().enumerate() {
        assert_eq!(password_key::of(bytes), None, "case {index}");
    }
}
