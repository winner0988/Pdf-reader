//! POC for ADR 0014 (proposed, #100): verifying the corpus's signatures offline with the Windows
//! CryptoAPI. In the real design this runs in the worker, next to the PDF parser; here it runs in
//! the test process. Nothing asks the network: chains are built from what Windows has locally.
#![cfg(windows)]
// FFI to crypt32; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::mem::size_of;
use std::ptr::{null, null_mut};

use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject, PdfWriteOptions};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Security::Cryptography::{
    CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT, CERT_CHAIN_PARA, CERT_CONTEXT,
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_TRUST_IS_UNTRUSTED_ROOT, CRYPT_VERIFY_MESSAGE_PARA,
    CertFreeCertificateChain, CertFreeCertificateContext, CertGetCertificateChain,
    CertGetNameStringW, CryptVerifyDetachedMessageSignature, PKCS_7_ASN_ENCODING,
    X509_ASN_ENCODING,
};

/// The corpus's self-signed test certificate (tests/corpus/generate.py, `SIGNER_NAME`).
const SIGNER: &str = "PDF Reader test corpus signer (NOT TRUSTED)";

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn get(object: &PdfObject, key: &str) -> PdfObject {
    object
        .get_dict(key)
        .expect("get_dict")
        .unwrap_or_else(|| panic!("/{key} is missing"))
}

/// The first signature of `pdf`: the byte ranges it covers and its CMS `SignedData` (DER).
fn signature(pdf: &[u8]) -> ([usize; 4], Vec<u8>) {
    let doc = MuPdfDocument::from_bytes(pdf).expect("open");
    let root = get(&doc.trailer().expect("trailer"), "Root");
    let fields = get(&get(&root, "AcroForm"), "Fields");
    let field = fields
        .get_array(0)
        .expect("get_array")
        .expect("a signature field");
    let value = get(&field, "V");
    let range = get(&value, "ByteRange");
    let range = [0, 1, 2, 3].map(|i| {
        let number = range
            .get_array(i)
            .expect("get_array")
            .expect("four numbers");
        usize::try_from(number.as_int().expect("int")).expect("non-negative")
    });
    let contents = get(&value, "Contents").as_bytes().expect("bytes").to_vec();
    (range, der_prefix(contents))
}

/// `/Contents` is padded with zeros after the DER value; keep the value only.
fn der_prefix(mut bytes: Vec<u8>) -> Vec<u8> {
    assert_eq!(bytes[0], 0x30, "a DER SEQUENCE");
    let (header, length) = match bytes[1] {
        short @ 0..0x80 => (2, usize::from(short)),
        long => {
            let count = usize::from(long & 0x7f);
            let length = bytes[2..2 + count]
                .iter()
                .fold(0, |sum, byte| (sum << 8) | usize::from(*byte));
            (2 + count, length)
        }
    };
    bytes.truncate(header + length);
    bytes
}

#[derive(Debug, PartialEq)]
enum Verdict {
    /// The signed bytes are unchanged and the signature is mathematically valid.
    Valid {
        signer: String,
        /// The certificate chains to a root Windows trusts (checked offline).
        trusted_root: bool,
        /// Nothing was appended after signing (an incremental update would be).
        covers_whole_file: bool,
    },
    /// The signed bytes changed, or the signature does not match: the `GetLastError` code.
    Invalid(u32),
}

fn verify(pdf: &[u8]) -> Verdict {
    let ([start, first, second, rest], der) = signature(pdf);
    let parts = [&pdf[start..start + first], &pdf[second..second + rest]];
    let pointers = parts.map(<[u8]>::as_ptr);
    let sizes = parts.map(|part| u32::try_from(part.len()).expect("size"));
    let para = CRYPT_VERIFY_MESSAGE_PARA {
        cbSize: size_of::<CRYPT_VERIFY_MESSAGE_PARA>() as u32,
        dwMsgAndCertEncodingType: X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
        ..Default::default()
    };
    let mut signer: *mut CERT_CONTEXT = null_mut();
    // SAFETY: every pointer refers to a live buffer of the given size for the whole call; the
    // signer certificate, if any, is freed below.
    let verified = unsafe {
        CryptVerifyDetachedMessageSignature(
            &para,
            0,
            der.as_ptr(),
            u32::try_from(der.len()).expect("size"),
            2,
            pointers.as_ptr(),
            sizes.as_ptr(),
            &mut signer,
        )
    };
    if verified == 0 {
        // SAFETY: no other call in between.
        return Verdict::Invalid(unsafe { GetLastError() });
    }
    let name = name_of(signer);
    let trusted_root = chains_to_trusted_root(signer);
    // SAFETY: `signer` came from CryptVerifyDetachedMessageSignature and is freed once.
    unsafe { CertFreeCertificateContext(signer) };
    Verdict::Valid {
        signer: name,
        trusted_root,
        covers_whole_file: second + rest == pdf.len(),
    }
}

fn name_of(certificate: *const CERT_CONTEXT) -> String {
    let mut name = [0u16; 256];
    // SAFETY: a valid certificate context and a buffer of the given length.
    let length = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            null(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    // The length includes the terminating NUL.
    String::from_utf16_lossy(&name[..(length as usize).saturating_sub(1)])
}

fn chains_to_trusted_root(certificate: *const CERT_CONTEXT) -> bool {
    let para = CERT_CHAIN_PARA {
        cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
        ..Default::default()
    };
    let mut chain: *mut CERT_CHAIN_CONTEXT = null_mut();
    // SAFETY: a valid certificate context; the default chain engine and no extra store; the
    // chain is freed below. CACHE_ONLY: no URL is ever fetched while building it.
    let built = unsafe {
        CertGetCertificateChain(
            null_mut(),
            certificate,
            null(),
            null_mut(),
            &para,
            CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL,
            null(),
            &mut chain,
        )
    };
    assert_ne!(built, 0, "a chain, trusted or not");
    // SAFETY: `chain` came from CertGetCertificateChain and is freed once.
    let status = unsafe { (*chain).TrustStatus.dwErrorStatus };
    unsafe { CertFreeCertificateChain(chain) };
    status & CERT_TRUST_IS_UNTRUSTED_ROOT == 0
}

#[test]
fn the_corpus_signatures_are_valid_but_not_trusted() {
    for name in ["benign/signed.pdf", "benign/signed-docmdp-p1.pdf"] {
        assert_eq!(
            verify(&corpus(name)),
            Verdict::Valid {
                signer: SIGNER.to_owned(),
                trusted_root: false,
                covers_whole_file: true,
            },
            "{name}"
        );
    }
}

#[test]
fn a_change_to_the_signed_bytes_is_detected() {
    let mut pdf = corpus("benign/signed.pdf");
    // One letter of the page's text, inside the signed range.
    let text = b"Signed sample";
    let at = pdf
        .windows(text.len())
        .position(|window| window == text)
        .expect("the page text");
    pdf[at] = b's';
    assert!(matches!(verify(&pdf), Verdict::Invalid(_)));
}

#[test]
fn an_update_after_signing_leaves_the_signature_valid_for_what_was_signed() {
    let original = corpus("benign/signed.pdf");
    let doc = MuPdfDocument::from_bytes(&original).expect("open");
    doc.load_pdf_page(0)
        .expect("page")
        .set_rotation(90)
        .expect("rotate");
    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    let mut updated = Vec::new();
    doc.write_to_with_options(&mut updated, options)
        .expect("write");

    // The signature still verifies, but it no longer covers the whole file: the reader must say
    // the document changed after it was signed.
    assert_eq!(
        verify(&updated),
        Verdict::Valid {
            signer: SIGNER.to_owned(),
            trusted_root: false,
            covers_whole_file: false,
        }
    );
}
