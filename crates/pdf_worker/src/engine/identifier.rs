//! The app's document identifier in a file's metadata (ADR 0010): the mark of a file the user
//! marked as sensitive (B3-05, ADR 0004). It is read when a document opens, and written only when
//! the user asks for it, appended to the file by an incremental update: never when a document is
//! opened or saved.

use std::io::Write;

use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfWriteOptions};

use super::{EngineError, PdfDocument, write_limited};
use crate::owner_password;

/// The entry of the document information dictionary that holds the identifier: a string of 32
/// hexadecimal digits, random. The privacy export removes the whole dictionary, and with it the
/// mark.
pub const IDENTIFIER_KEY: &str = "PdfReaderDocumentId";

impl PdfDocument {
    /// The identifier in the file's metadata, if it has one as the app writes it: anything else
    /// in that entry is not the app's mark.
    pub fn identifier(&self) -> Option<[u8; 16]> {
        identifier_of(&self.doc)
    }
}

fn identifier_of(doc: &MuPdfDocument) -> Option<[u8; 16]> {
    let info = doc.trailer().ok()?.get_dict("Info").ok()??;
    let value = info.get_dict(IDENTIFIER_KEY).ok()??;
    if !value.is_string().ok()? {
        return None;
    }
    parse(&value.as_string().ok()?)
}

/// The 16 bytes written as `text`: exactly 32 hexadecimal digits, of either case.
fn parse(text: &str) -> Option<[u8; 16]> {
    let digits = text.as_bytes();
    if digits.len() != 32 {
        return None;
    }
    let mut identifier = [0u8; 16];
    for (byte, pair) in identifier.iter_mut().zip(digits.as_chunks::<2>().0) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        *byte = u8::try_from(high << 4 | low).ok()?;
    }
    Some(identifier)
}

/// The identifier as it is written: 32 lowercase hexadecimal digits.
pub fn hex(identifier: &[u8; 16]) -> String {
    identifier
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Writes to `out` the PDF file `bytes`, with `identifier` put into its metadata (`None`: taken
/// out), by an incremental update (ADR 0010): first the file's own bytes, unchanged, then the
/// update. `bytes` is the file the document was opened from or last saved to, opened again here
/// on its own: the document open in the worker, with its edits, is not touched. Returns the bytes
/// written.
///
/// Refused:
/// - for a signed document (`Signed`): every check of its signatures would say it was changed
///   after signing, and a certification that allows no change (DocMDP P=1) would be broken;
/// - for an encrypted one, unless `password` is its permissions (owner) password (`NotAllowed`);
///   a wrong password, or none where one is needed, as when opening it;
/// - for a file MuPDF had to repair to open it (`NotAppendable`): its own bytes cannot be kept
///   as they are, so there is nothing to append to.
pub fn write_identifier(
    bytes: &[u8],
    password: Option<&str>,
    identifier: Option<&[u8; 16]>,
    out: &mut impl Write,
) -> Result<u64, EngineError> {
    let mut document = PdfDocument::open(bytes, password)?;
    if document.is_encrypted()
        && !password.is_some_and(|password| owner_password::is_owner_password(bytes, password))
    {
        return Err(EngineError::NotAllowed(
            "an encrypted file is written to only with its permissions password",
        ));
    }
    if document.is_signed() {
        return Err(EngineError::Signed);
    }
    if !document.doc.can_be_saved_incrementally() {
        return Err(EngineError::NotAppendable);
    }
    set_identifier(&mut document.doc, identifier)?;
    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    write_limited(&document.doc, out, options)
}

/// Puts `identifier` into the document information dictionary of `doc`, made if there is none,
/// or takes it out (`None`). Everything else in the dictionary stays as it is.
fn set_identifier(
    doc: &mut MuPdfDocument,
    identifier: Option<&[u8; 16]>,
) -> Result<(), EngineError> {
    let existing = doc.trailer()?.get_dict("Info")?;
    match (existing, identifier) {
        (Some(mut info), Some(identifier)) => {
            info.dict_put(IDENTIFIER_KEY, doc.new_string(&hex(identifier))?)?;
        }
        (None, Some(identifier)) => {
            let mut info = doc.new_dict()?;
            info.dict_put(IDENTIFIER_KEY, doc.new_string(&hex(identifier))?)?;
            let info = doc.add_object(&info)?;
            // Adding the object began the update, with a trailer of its own: that one is written,
            // so it is the one that names the dictionary (the file's last one stays as it was).
            doc.trailer()?.dict_put("Info", info)?;
        }
        (Some(mut info), None) => info.dict_delete(IDENTIFIER_KEY)?,
        (None, None) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::PdfDocument;

    fn corpus(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus")
                .join(name),
        )
        .expect("corpus file")
    }

    const MARK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];

    fn written(bytes: &[u8], password: Option<&str>, identifier: Option<&[u8; 16]>) -> Vec<u8> {
        let mut out = Vec::new();
        let count = write_identifier(bytes, password, identifier, &mut out).expect("written");
        assert_eq!(count, out.len() as u64);
        out
    }

    #[test]
    fn the_mark_is_appended_and_the_file_before_it_is_unchanged() {
        let original = corpus("benign/multi-page-10.pdf");
        assert_eq!(
            PdfDocument::open(&original, None).unwrap().identifier(),
            None
        );
        let marked = written(&original, None, Some(&MARK));
        // An incremental update: the file's bytes first, as they were, then a little more.
        assert_eq!(&marked[..original.len()], &original[..]);
        assert!(marked.len() - original.len() < 4096, "{}", marked.len());
        let tail = String::from_utf8_lossy(&marked[original.len()..]).into_owned();
        assert!(tail.contains(IDENTIFIER_KEY), "{tail}");
        assert!(tail.contains("00112233445566778899aabbccddeeff"), "{tail}");
        let reopened = PdfDocument::open(&marked, None).expect("reopened");
        assert_eq!(reopened.identifier(), Some(MARK));
        assert_eq!(reopened.page_count().unwrap(), 10);

        // Taken out again: appended too, and the identifier is gone from the document.
        let unmarked = written(&marked, None, None);
        assert_eq!(&unmarked[..marked.len()], &marked[..]);
        assert_eq!(
            PdfDocument::open(&unmarked, None).unwrap().identifier(),
            None
        );
    }

    #[test]
    fn the_rest_of_the_metadata_stays() {
        let original = corpus("benign/metadata-full.pdf");
        let info = |bytes: &[u8]| {
            let document = PdfDocument::open(bytes, None).unwrap();
            let info = document
                .doc
                .trailer()
                .unwrap()
                .get_dict("Info")
                .unwrap()
                .unwrap();
            info.get_dict("Author")
                .unwrap()
                .unwrap()
                .as_string()
                .unwrap()
        };
        let marked = written(&original, None, Some(&MARK));
        assert_eq!(info(&marked), info(&original));
        assert_eq!(
            PdfDocument::open(&marked, None).unwrap().identifier(),
            Some(MARK)
        );
    }

    #[test]
    fn a_signed_document_is_never_written_to() {
        for name in ["benign/signed.pdf", "benign/signed-docmdp-p1.pdf"] {
            let mut out = Vec::new();
            let refused = write_identifier(&corpus(name), None, Some(&MARK), &mut out);
            assert!(
                matches!(refused, Err(EngineError::Signed)),
                "{name}: {refused:?}"
            );
            assert!(out.is_empty(), "{name}");
        }
    }

    #[test]
    fn an_encrypted_document_needs_its_permissions_password() {
        for name in [
            "benign/encrypted-rc4-40.pdf",
            "benign/encrypted-aes256.pdf",
            "benign/restricted-open-password.pdf",
        ] {
            let original = corpus(name);
            let attempt = |password: Option<&str>| {
                let mut out = Vec::new();
                let result = write_identifier(&original, password, Some(&MARK), &mut out);
                (result, out)
            };
            let (none, out) = attempt(None);
            assert!(
                matches!(none, Err(EngineError::Encrypted)),
                "{name}: {none:?}"
            );
            assert!(out.is_empty());
            let (wrong, out) = attempt(Some("wrong"));
            assert!(
                matches!(wrong, Err(EngineError::WrongPassword)),
                "{name}: {wrong:?}"
            );
            assert!(out.is_empty());
            // The open password opens it, and is not enough.
            let (user, out) = attempt(Some("user"));
            assert!(
                matches!(user, Err(EngineError::NotAllowed(_))),
                "{name}: {user:?}"
            );
            assert!(out.is_empty());

            let marked = written(&original, Some("owner"), Some(&MARK));
            assert_eq!(&marked[..original.len()], &original[..]);
            // Still encrypted: it asks for a password, and either one shows the mark.
            assert!(matches!(
                PdfDocument::open(&marked, None),
                Err(EngineError::Encrypted)
            ));
            for password in ["user", "owner"] {
                let reopened = PdfDocument::open(&marked, Some(password)).expect("reopened");
                assert_eq!(reopened.identifier(), Some(MARK), "{name} with {password}");
            }
            // The mark is encrypted like the rest of the file's strings.
            assert!(
                !String::from_utf8_lossy(&marked[original.len()..])
                    .contains("00112233445566778899aabbccddeeff"),
                "{name}"
            );
        }
    }

    #[test]
    fn a_file_with_only_permissions_needs_its_permissions_password_too() {
        let original = corpus("benign/restricted-no-copy-no-print.pdf");
        let mut out = Vec::new();
        let refused = write_identifier(&original, None, Some(&MARK), &mut out);
        assert!(
            matches!(refused, Err(EngineError::NotAllowed(_))),
            "{refused:?}"
        );
        assert!(out.is_empty());
        let marked = written(&original, Some("owner"), Some(&MARK));
        assert_eq!(
            PdfDocument::open(&marked, None).unwrap().identifier(),
            Some(MARK)
        );
    }

    #[test]
    fn a_repaired_file_has_nothing_to_append_to() {
        let mut out = Vec::new();
        let refused = write_identifier(
            &corpus("malformed/broken-xref.pdf"),
            None,
            Some(&MARK),
            &mut out,
        );
        assert!(
            matches!(refused, Err(EngineError::NotAppendable)),
            "{refused:?}"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn only_what_the_app_writes_is_its_mark() {
        assert_eq!(parse("00112233445566778899AABBCCDDEEFF"), Some(MARK));
        assert_eq!(parse(&hex(&MARK)), Some(MARK));
        for text in [
            "",
            "0011223344556677",
            "00112233445566778899aabbccddeeff00",
            "00112233445566778899aabbccddeegg",
            "0011223344556677 8899aabbccddeef",
            "０0112233445566778899aabbccddeef",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
