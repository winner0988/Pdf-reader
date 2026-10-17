//! Where a password the user remembered for an encrypted document is looked up (ADR 0017).
//!
//! The file says it itself, before any password is given: the verifier of its user password
//! (`/U`) sits unencrypted in the `/Encrypt` dictionary, which exists for exactly one encryption
//! of the file. A hash of the revision and the verifier is the key: nothing is written into the
//! PDF, a copy or a rename of the file keeps it, and a file encrypted again (another password,
//! or the same one with new random salts) gets another one, so that a password remembered for
//! the old encryption is never tried on the new one.

use ipc_contract::worker::PasswordKey;
use mupdf::Document;
use mupdf::pdf::PdfDocument as MuPdfDocument;
use sha2::{Digest, Sha256};

/// Names what the hash is for, and its version: another way of making keys is `v2`, which finds
/// none of the credentials of `v1`.
const DOMAIN: &[u8] = b"pdf-reader/password-key/v1";

/// The key of the encrypted PDF in `bytes`, if it says what its user password verifier is: the
/// standard security handler, revision 2 to 6. `None` for anything else (not encrypted, another
/// handler, a `/U` that is missing, not a string or too short): such a file cannot have its
/// password remembered. Nothing is run and no password is needed.
pub fn of(bytes: &[u8]) -> Option<PasswordKey> {
    let document = Document::from_bytes(bytes, "application/pdf").ok()?;
    let pdf = MuPdfDocument::try_from(document).ok()?;
    let trailer = pdf.trailer().ok()?;
    let encrypt = trailer.get_dict("Encrypt").ok()??;
    let filter = encrypt.get_dict("Filter").ok()??;
    if !filter.is_name().ok()? || filter.as_name().ok()?.as_slice() != b"Standard" {
        return None;
    }
    let revision = encrypt.get_dict("R").ok()??.as_int().ok()?;
    let verifier = encrypt.get_dict("U").ok()??.as_bytes().ok()?;
    derive(revision, &verifier)
}

/// The key for a file of this `revision` (`/R`) whose user password verifier (`/U`) is `verifier`.
/// Only the bytes MuPDF itself checks a password against count: 32 for revisions 2 to 4, 48 for
/// 5 and 6 (the hash and its two salts); what follows is padding the file may have.
pub fn derive(revision: i32, verifier: &[u8]) -> Option<PasswordKey> {
    let length = match revision {
        2..=4 => 32,
        5 | 6 => 48,
        _ => return None,
    };
    let verifier = verifier.get(..length)?;
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    // 2 to 6, so one byte.
    hasher.update([revision as u8]);
    hasher.update(verifier);
    Some(PasswordKey(hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_a_hash_of_the_revision_and_the_verifier_that_counts() {
        let verifier: Vec<u8> = (0..48).collect();
        let key = derive(6, &verifier).expect("revision 6");
        // The same bytes always make the same key: it is looked up by, so it must not vary.
        assert_eq!(Some(key), derive(6, &verifier));
        // Revision 6 takes all 48 bytes, so any of them changes the key...
        for at in [0, 31, 32, 47] {
            let mut changed = verifier.clone();
            changed[at] ^= 1;
            assert_ne!(Some(key), derive(6, &changed), "byte {at}");
        }
        // ...and what comes after them does not.
        let mut padded = verifier.clone();
        padded.extend_from_slice(&[7; 16]);
        assert_eq!(Some(key), derive(6, &padded));
    }

    #[test]
    fn the_revision_is_part_of_the_key_and_only_the_standard_ones_have_one() {
        let verifier = [9u8; 48];
        assert_ne!(derive(5, &verifier), derive(6, &verifier));
        assert_ne!(derive(2, &verifier), derive(3, &verifier));
        // Revisions 2 to 4 take the first 32 bytes, however long the string is.
        assert_eq!(derive(4, &verifier), derive(4, &verifier[..32]));
        for revision in [i32::MIN, -1, 0, 1, 7, 100, i32::MAX] {
            assert_eq!(derive(revision, &verifier), None, "revision {revision}");
        }
    }

    #[test]
    fn a_verifier_that_is_too_short_makes_no_key() {
        assert_eq!(derive(2, &[0; 31]), None);
        assert_eq!(derive(4, &[]), None);
        assert_eq!(derive(5, &[0; 47]), None);
        assert!(derive(2, &[0; 32]).is_some());
        assert!(derive(6, &[0; 48]).is_some());
    }

    #[test]
    fn a_key_made_by_another_domain_or_version_is_not_this_key() {
        // Pins the construction: SHA-256 over the domain, one byte of revision, the verifier.
        let verifier = [0u8; 32];
        let mut hasher = Sha256::new();
        hasher.update(b"pdf-reader/password-key/v1");
        hasher.update([2u8]);
        hasher.update(verifier);
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(derive(2, &verifier), Some(PasswordKey(expected)));
        let mut other = Sha256::new();
        other.update(b"pdf-reader/password-key/v2");
        other.update([2u8]);
        other.update(verifier);
        let other: [u8; 32] = other.finalize().into();
        assert_ne!(expected, other);
    }

    #[test]
    fn what_is_not_an_encrypted_pdf_has_no_key() {
        assert_eq!(of(b""), None);
        assert_eq!(of(b"not a pdf at all"), None);
        assert_eq!(of(b"%PDF-1.7\n%%EOF\n"), None);
    }
}
