//! The encrypted and signed samples of the corpus (QA-01, QA-04). tests/corpus/generate.py writes
//! them with its own AES, RSA and CMS code, so MuPDF has to confirm they are what they claim to
//! be; otherwise they would test nothing.

use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};
use mupdf::{Document, TextPageFlags};
use pdf_worker::engine::{EngineError, PdfDocument};

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The first page's text once `password` is accepted, or `None` if it is not.
fn text_with_password(bytes: &[u8], password: &str) -> Option<String> {
    let mut doc = Document::from_bytes(bytes, "application/pdf").expect("open");
    assert!(doc.needs_password().expect("needs_password"));
    if !doc.authenticate(password).expect("authenticate") {
        return None;
    }
    let page = doc.load_page(0).expect("page");
    let text = page
        .to_text_page(TextPageFlags::empty())
        .expect("text page");
    Some(text.to_text().expect("text"))
}

#[test]
fn encrypted_samples_open_only_with_their_passwords() {
    for (name, text) in [
        (
            "benign/encrypted-rc4-40.pdf",
            "Encrypted sample (user password: user)",
        ),
        (
            "benign/encrypted-aes256.pdf",
            "Encrypted sample, AES-256 (user password: user)",
        ),
    ] {
        let bytes = corpus(name);
        // The worker asks for a password (MVP-16), rejects a wrong one, and opens with either.
        assert!(
            matches!(PdfDocument::from_bytes(&bytes), Err(EngineError::Encrypted)),
            "{name}"
        );
        assert!(
            matches!(
                PdfDocument::open(&bytes, Some("wrong")),
                Err(EngineError::WrongPassword)
            ),
            "{name}"
        );
        for password in ["user", "owner"] {
            let doc = PdfDocument::open(&bytes, Some(password))
                .unwrap_or_else(|e| panic!("{name} with {password:?}: {e}"));
            let lines = doc.page_text(0, 1000).expect("page text").lines;
            assert!(
                lines.iter().any(|line| line.text.contains(text)),
                "{name} with {password:?}: {lines:?}"
            );
        }
        assert_eq!(text_with_password(&bytes, "wrong"), None, "{name}");
        for password in ["user", "owner"] {
            let found = text_with_password(&bytes, password)
                .unwrap_or_else(|| panic!("{name}: {password:?} was rejected"));
            assert!(found.contains(text), "{name} with {password:?}: {found:?}");
        }
    }
}

fn get(object: &PdfObject, key: &str) -> PdfObject {
    object
        .get_dict(key)
        .expect("get_dict")
        .unwrap_or_else(|| panic!("/{key} is missing"))
}

fn at(array: &PdfObject, index: i32) -> PdfObject {
    array
        .get_array(index)
        .expect("get_array")
        .unwrap_or_else(|| panic!("index {index} is missing"))
}

#[test]
fn signed_samples_have_a_readable_signature() {
    for (name, certified) in [
        ("benign/signed.pdf", false),
        ("benign/signed-docmdp-p1.pdf", true),
    ] {
        let bytes = corpus(name);
        // An ordinary document for the worker: it opens.
        PdfDocument::from_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));

        let doc = MuPdfDocument::from_bytes(&bytes).expect("open");
        let root = get(&doc.trailer().expect("trailer"), "Root");
        let field = at(&get(&get(&root, "AcroForm"), "Fields"), 0);
        assert_eq!(get(&field, "FT").as_name().unwrap(), b"Sig", "{name}");
        let signature = get(&field, "V");
        assert_eq!(
            get(&signature, "SubFilter").as_name().unwrap(),
            b"adbe.pkcs7.detached",
            "{name}"
        );

        // /ByteRange covers the whole file except the /Contents hex string.
        let range = get(&signature, "ByteRange");
        let range: Vec<usize> = (0..4)
            .map(|i| usize::try_from(at(&range, i).as_int().unwrap()).unwrap())
            .collect();
        assert_eq!(range[0], 0, "{name}");
        assert_eq!(range[2] + range[3], bytes.len(), "{name}");
        assert_eq!(
            (bytes[range[1]], bytes[range[2] - 1]),
            (b'<', b'>'),
            "{name}"
        );
        // /Contents holds the DER of a CMS ContentInfo: a SEQUENCE.
        let contents = get(&signature, "Contents").as_bytes().unwrap();
        assert_eq!(contents.first(), Some(&0x30), "{name}");

        let perms = root.get_dict("Perms").expect("get_dict");
        if certified {
            let docmdp = get(&perms.expect("/Perms"), "DocMDP");
            let params = get(&at(&get(&docmdp, "Reference"), 0), "TransformParams");
            assert_eq!(get(&params, "P").as_int().unwrap(), 1, "{name}");
        } else {
            assert!(perms.is_none(), "{name}");
        }
    }
}
