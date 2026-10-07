//! Verifies a detached CMS signature with the Windows CryptoAPI (B2-14, ADR 0014).
//!
//! This is the only code in the worker that calls `crypt32`. The signature, its certificates and
//! the bytes it covers all come from an untrusted file, and Windows' own C code parses them: here,
//! in the worker's sandbox. Nothing is fetched from anywhere: a chain is built from the
//! certificates the message carries and what Windows already has on this computer
//! (`CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`), and the revocation of a certificate is not asked.

#![allow(unsafe_code)]

/// What Windows made of one signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The signature holds for the two parts: they are as the signer left them.
    Holds {
        /// The name the signer's certificate gives its owner, as Windows displays it.
        signer: String,
        /// The certificate chains to a root Windows trusts, with no problem found.
        trusted: bool,
    },
    /// The signed bytes were changed, the signature does not fit them, or it is no signature.
    Invalid,
    /// It uses an algorithm this Windows does not know.
    UnsupportedAlgorithm,
    /// Windows cannot make sense of it, though it may be a signature (the signer's certificate
    /// is not in it, for one).
    UnsupportedFormat,
    /// This is not Windows (only built there).
    #[cfg_attr(windows, allow(dead_code))]
    NotAvailable,
}

/// Verifies `cms` (the DER of a CMS `SignedData` without its content) against the two parts of
/// the file that it signs, in order.
#[cfg(windows)]
pub fn verify_detached(parts: [&[u8]; 2], cms: &[u8]) -> Verdict {
    cryptoapi::verify(parts, cms)
}

#[cfg(not(windows))]
pub fn verify_detached(_parts: [&[u8]; 2], _cms: &[u8]) -> Verdict {
    Verdict::NotAvailable
}

#[cfg(windows)]
mod cryptoapi {
    use std::mem::size_of;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CRYPT_E_ASN1_ERROR, CRYPT_E_ATTRIBUTES_MISSING, CRYPT_E_BAD_MSG, CRYPT_E_HASH_VALUE,
        CRYPT_E_INVALID_MSG_TYPE, CRYPT_E_NO_SIGNER, CRYPT_E_NOT_FOUND, CRYPT_E_SIGNER_NOT_FOUND,
        CRYPT_E_UNEXPECTED_MSG_TYPE, CRYPT_E_UNKNOWN_ALGO, GetLastError, NTE_BAD_ALGID,
        NTE_BAD_SIGNATURE, STATUS_HASH_NOT_SUPPORTED, STATUS_INVALID_SIGNATURE,
    };
    use windows_sys::Win32::Security::Cryptography::{
        CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT,
        CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE, CERT_CHAIN_PARA, CERT_CONTEXT,
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_TRUST_NO_ERROR, CRYPT_VERIFY_MESSAGE_PARA,
        CertCloseStore, CertFreeCertificateChain, CertFreeCertificateContext,
        CertGetCertificateChain, CertGetNameStringW, CryptGetMessageCertificates,
        CryptVerifyDetachedMessageSignature, HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };

    use super::Verdict;

    /// Longest name read from a certificate, in UTF-16 units (the caller shortens it further).
    const NAME_UNITS: usize = 256;

    /// A certificate Windows handed out; freed when dropped.
    struct Certificate(*const CERT_CONTEXT);

    impl Drop for Certificate {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the context came from a CryptoAPI call that hands out a reference to
                // free, and this is the only owner.
                unsafe { CertFreeCertificateContext(self.0) };
            }
        }
    }

    /// The certificates a message carries, as a store; closed when dropped.
    struct Store(HCERTSTORE);

    impl Drop for Store {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the store came from `CryptGetMessageCertificates` and is closed once.
                unsafe { CertCloseStore(self.0, 0) };
            }
        }
    }

    /// A certificate chain; freed when dropped.
    struct Chain(*const CERT_CHAIN_CONTEXT);

    impl Drop for Chain {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the chain came from `CertGetCertificateChain` and is freed once.
                unsafe { CertFreeCertificateChain(self.0) };
            }
        }
    }

    pub fn verify(parts: [&[u8]; 2], cms: &[u8]) -> Verdict {
        let sizes = parts.map(|part| u32::try_from(part.len()));
        let (Ok(first), Ok(second), Ok(length)) = (sizes[0], sizes[1], u32::try_from(cms.len()))
        else {
            // Sizes that do not fit what Windows takes cannot be a signature of this file.
            return Verdict::Invalid;
        };
        let pointers = parts.map(<[u8]>::as_ptr);
        let sizes = [first, second];
        let para = CRYPT_VERIFY_MESSAGE_PARA {
            cbSize: size_of::<CRYPT_VERIFY_MESSAGE_PARA>() as u32,
            dwMsgAndCertEncodingType: X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            ..Default::default()
        };
        let mut signer: *mut CERT_CONTEXT = null_mut();
        // SAFETY: every pointer refers to a live buffer of the size given with it for the whole
        // call, `para` is filled in as the API wants, and `signer` receives the signer's
        // certificate (or stays null), which `Certificate` frees.
        let verified = unsafe {
            CryptVerifyDetachedMessageSignature(
                &para,
                0,
                cms.as_ptr(),
                length,
                2,
                pointers.as_ptr(),
                sizes.as_ptr(),
                &mut signer,
            )
        };
        let signer = Certificate(signer);
        if verified == 0 {
            // SAFETY: no other call came between the failed one and this.
            return failure(unsafe { GetLastError() });
        }
        if signer.0.is_null() {
            return Verdict::UnsupportedFormat;
        }
        Verdict::Holds {
            signer: name_of(&signer),
            trusted: chain_is_trusted(&signer, cms, length),
        }
    }

    /// What the error code of a failed verification says of the signature.
    fn failure(code: u32) -> Verdict {
        // The ASN.1 decoder's errors (0x80093100 and the 255 that follow): the message is corrupt.
        let asn1 = CRYPT_E_ASN1_ERROR as u32;
        if (asn1..asn1 + 0x100).contains(&code) {
            return Verdict::Invalid;
        }
        match code as i32 {
            CRYPT_E_HASH_VALUE
            | NTE_BAD_SIGNATURE
            | STATUS_INVALID_SIGNATURE
            | CRYPT_E_BAD_MSG
            | CRYPT_E_UNEXPECTED_MSG_TYPE
            | CRYPT_E_INVALID_MSG_TYPE => Verdict::Invalid,
            CRYPT_E_UNKNOWN_ALGO | NTE_BAD_ALGID | STATUS_HASH_NOT_SUPPORTED => {
                Verdict::UnsupportedAlgorithm
            }
            // The signer's certificate is not in the message, or it has no signer or attributes
            // that are needed: it is made in a way this does not handle. So is anything else.
            CRYPT_E_NOT_FOUND
            | CRYPT_E_NO_SIGNER
            | CRYPT_E_SIGNER_NOT_FOUND
            | CRYPT_E_ATTRIBUTES_MISSING => Verdict::UnsupportedFormat,
            _ => Verdict::UnsupportedFormat,
        }
    }

    /// The name the certificate gives its owner, as Windows displays it.
    fn name_of(certificate: &Certificate) -> String {
        let mut name = [0u16; NAME_UNITS];
        // SAFETY: a live certificate context, and a buffer of the length given.
        let length = unsafe {
            CertGetNameStringW(
                certificate.0,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                null(),
                name.as_mut_ptr(),
                NAME_UNITS as u32,
            )
        };
        // The length counts the terminating NUL.
        let units = (length as usize).saturating_sub(1).min(NAME_UNITS);
        String::from_utf16_lossy(&name[..units])
    }

    /// Whether `signer`'s certificate chains to a root Windows trusts, with no other problem
    /// found: what the message carries and what Windows has locally are all there is to build it
    /// with, and nothing is fetched.
    fn chain_is_trusted(signer: &Certificate, cms: &[u8], length: u32) -> bool {
        // SAFETY: `cms` is a live buffer of `length` bytes; the store (null if the message has no
        // certificates) is closed by `Store`.
        let carried = Store(unsafe {
            CryptGetMessageCertificates(
                X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                0,
                0,
                cms.as_ptr(),
                length,
            )
        });
        let para = CERT_CHAIN_PARA {
            cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
            ..Default::default()
        };
        let mut chain: *mut CERT_CHAIN_CONTEXT = null_mut();
        // SAFETY: a live certificate context, the default chain engine and no time (now), the
        // message's own store, and `para` filled in as the API wants; the chain is freed by
        // `Chain`. No flag asks for revocation, and URL retrieval is limited to the cache.
        let built = unsafe {
            CertGetCertificateChain(
                null_mut(),
                signer.0,
                null(),
                carried.0,
                &para,
                CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE,
                null(),
                &mut chain,
            )
        };
        let chain = Chain(chain);
        if built == 0 || chain.0.is_null() {
            return false;
        }
        // SAFETY: a chain context that `CertGetCertificateChain` built and nothing has freed.
        unsafe { (*chain.0).TrustStatus.dwErrorStatus == CERT_TRUST_NO_ERROR }
    }

    #[cfg(test)]
    mod tests {
        use windows_sys::Win32::Security::Cryptography::{
            CertDuplicateCertificateContext, CertEnumCertificatesInStore, CertOpenSystemStoreW,
        };

        use super::*;

        /// Windows' own root store has certificates that chain to themselves, and the chain of
        /// each says it is trusted: the code that decides that is not made to say "no" to all.
        /// (The self-signed certificate of the corpus is the one that says no, in
        /// tests/signatures.rs.)
        #[test]
        fn a_root_windows_trusts_is_trusted() {
            let name: Vec<u16> = "ROOT\0".encode_utf16().collect();
            // SAFETY: a NUL-terminated name; the store is closed by `Store`.
            let store = Store(unsafe { CertOpenSystemStoreW(0, name.as_ptr()) });
            assert!(!store.0.is_null(), "the root store of this computer");
            let mut trusted = 0;
            let mut current: *const CERT_CONTEXT = null();
            for _ in 0..30 {
                // SAFETY: the store is open; the call frees the context it is given and hands out
                // the next, which is null at the end.
                current = unsafe { CertEnumCertificatesInStore(store.0, current) };
                if current.is_null() {
                    break;
                }
                // SAFETY: a live context; the duplicate is a reference of its own, freed by
                // `Certificate`.
                let own = Certificate(unsafe { CertDuplicateCertificateContext(current) });
                if chain_is_trusted(&own, &[], 0) {
                    trusted += 1;
                }
            }
            if !current.is_null() {
                // SAFETY: the enumeration stopped with a context that is still ours to free.
                unsafe { CertFreeCertificateContext(current) };
            }
            assert!(
                trusted > 0,
                "none of the first roots of this computer is trusted"
            );
        }
    }
}
