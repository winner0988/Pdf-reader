//! Whether an encrypted document was opened with its owner password (#88), so that, as in
//! Acrobat, the author's restrictions are lifted (docs/architecture/encryption.md).
//!
//! MuPDF knows which password worked, but the `mupdf` binding only says whether one did, and
//! keeps its context and documents to itself. So the document is opened a second time here,
//! straight through mupdf-sys, on a MuPDF context of its own, only to ask MuPDF.
//!
//! MuPDF reports errors with a long jump, which must never cross Rust frames:
//! - opening goes through mupdf-sys's C wrapper, which catches every error;
//! - the other calls run on this module's own context, where no MuPDF `fz_try` block is ever
//!   active: there MuPDF ends the process on an error instead of jumping. They only fail when
//!   out of memory, as the binding has already checked the same password on the same bytes.

#![allow(unsafe_code)]

use std::ptr;

use mupdf_sys::{
    FZ_STORE_DEFAULT, fz_drop_buffer, fz_drop_context, fz_new_buffer_from_shared_data,
    fz_new_context_imp, fz_set_error_callback, fz_set_warning_callback, mupdf_drop_error,
    mupdf_error_t, mupdf_pdf_open_document_from_bytes, pdf_authenticate_password,
    pdf_drop_document,
};
use zeroize::Zeroizing;

/// The MuPDF version a context must be made for: `mupdf` =0.8.0 is MuPDF 1.27.2. With any other,
/// MuPDF makes no context and this says no owner password (the tests would fail).
const FZ_VERSION: &std::ffi::CStr = c"1.27.2";

/// What `pdf_authenticate_password` returns for the owner password (with 2 for the user one).
const OWNER_PASSWORD: i32 = 4;

/// Whether `password`, which has opened the encrypted PDF in `bytes`, is its owner password.
/// False when MuPDF cannot tell.
pub fn is_owner_password(bytes: &[u8], password: &str) -> bool {
    if password.contains('\0') {
        return false;
    }
    // MuPDF's copy of the password, NUL-terminated; cleared when dropped, like the one the worker
    // received. Made at its final size, so no other copy is left behind by growing it.
    let mut c_password = Zeroizing::new(Vec::with_capacity(password.len() + 1));
    c_password.extend_from_slice(password.as_bytes());
    c_password.push(0);
    // SAFETY: every pointer MuPDF gets is live for the call. The context, buffer and document
    // are made and dropped here, in that order reversed, on this thread only; the buffer shares
    // `bytes`, which outlive it. Errors: see the module comment.
    unsafe {
        let ctx = fz_new_context_imp(
            ptr::null(),
            ptr::null(),
            FZ_STORE_DEFAULT as usize,
            FZ_VERSION.as_ptr(),
        );
        if ctx.is_null() {
            return false;
        }
        // Quiet, like the binding's context: a damaged file is reported when the binding opens it.
        fz_set_warning_callback(ctx, None, ptr::null_mut());
        fz_set_error_callback(ctx, None, ptr::null_mut());
        let buffer = fz_new_buffer_from_shared_data(ctx, bytes.as_ptr(), bytes.len());
        let mut error: *mut mupdf_error_t = ptr::null_mut();
        let doc = mupdf_pdf_open_document_from_bytes(ctx, buffer, &mut error);
        let owner = if doc.is_null() {
            mupdf_drop_error(error);
            false
        } else {
            let access = pdf_authenticate_password(ctx, doc, c_password.as_ptr().cast());
            pdf_drop_document(ctx, doc);
            access & OWNER_PASSWORD != 0
        };
        fz_drop_buffer(ctx, buffer);
        fz_drop_context(ctx);
        owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn tells_the_owner_password_from_the_user_password() {
        for name in [
            "benign/restricted-open-password.pdf",
            "benign/encrypted-aes256.pdf",
            "benign/encrypted-rc4-40.pdf",
        ] {
            let bytes = corpus(name);
            assert!(is_owner_password(&bytes, "owner"), "{name}");
            assert!(!is_owner_password(&bytes, "user"), "{name}");
            assert!(!is_owner_password(&bytes, "wrong"), "{name}");
        }
    }

    #[test]
    fn says_no_when_mupdf_cannot_tell() {
        assert!(!is_owner_password(b"not a PDF", "owner"));
        assert!(!is_owner_password(
            &corpus("benign/single-page.pdf"),
            "owner"
        ));
        assert!(!is_owner_password(
            &corpus("benign/restricted-open-password.pdf"),
            "own\0er"
        ));
    }
}
