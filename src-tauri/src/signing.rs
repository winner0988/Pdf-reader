//! Signing a document (B3-06, ADR 0020, docs/architecture/signing.md): the main process's part,
//! the only code of the app that holds a private key.
//!
//! A certificate file the user chose (PKCS #12: `.pfx`, `.p12`) is imported with
//! `PFXImportCertStore` into a store that lives in memory only: its key is not persisted
//! (`PKCS12_NO_PERSIST_KEY`) and cannot be exported (no `CRYPT_EXPORTABLE`), and both go when the
//! [`Signer`] is dropped. The password goes to Windows once and is wiped. A certificate of the
//! user's own store (the current user's "Personal" store) is used where it is: its key stays in
//! the user's key store or on a smart card, and Windows, or the card's own software, asks for its
//! PIN; nothing is imported or copied. A signature is a
//! detached CMS `SignedData` over the two parts of a file that the worker prepared, with SHA-256,
//! the time of signing, and the signer's certificate with the chain this computer can build
//! without fetching anything. No PDF is parsed here: what is signed is bytes, and the worker said
//! which.

// The CryptoAPI calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::mem::size_of;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use ipc_contract::limits::{
    MAX_CERTIFICATE_FILE_BYTES, MAX_SIGNATURE_TEXT_BYTES, MAX_STORE_CERTIFICATES,
};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{CertificateDate, CertificateProblem, ErrorCode, IpcError};
use ipc_contract::worker::SigningTime;
use windows_sys::Win32::Foundation::{
    ERROR_CANCELLED, FILETIME, GetLastError, NTE_USER_CANCELLED, SCARD_E_CANCELLED,
    SCARD_E_NO_READERS_AVAILABLE, SCARD_E_NO_SMARTCARD, SCARD_E_READER_UNAVAILABLE,
    SCARD_W_CANCELLED_BY_USER, SCARD_W_REMOVED_CARD,
};
use windows_sys::Win32::Security::Cryptography::{
    CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT,
    CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE, CERT_CHAIN_PARA, CERT_CONTEXT,
    CERT_DIGITAL_SIGNATURE_KEY_USAGE, CERT_FIND_EXISTING, CERT_FIND_HAS_PRIVATE_KEY, CERT_INFO,
    CERT_KEY_PROV_INFO_PROP_ID, CERT_NAME_ATTR_TYPE, CERT_NAME_ISSUER_FLAG,
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_NCRYPT_KEY_SPEC, CERT_NON_REPUDIATION_KEY_USAGE,
    CERT_STORE_OPEN_EXISTING_FLAG, CERT_STORE_PROV_SYSTEM_W, CERT_STORE_READONLY_FLAG,
    CERT_SYSTEM_STORE_CURRENT_USER, CMSG_CONTENT_PARAM, CMSG_DETACHED_FLAG, CMSG_SIGNED,
    CMSG_SIGNED_ENCODE_INFO, CMSG_SIGNER_ENCODE_INFO, CMSG_SIGNER_ENCODE_INFO_0,
    CRYPT_ACQUIRE_ALLOW_NCRYPT_KEY_FLAG, CRYPT_ACQUIRE_CACHE_FLAG,
    CRYPT_ACQUIRE_PREFER_NCRYPT_KEY_FLAG, CRYPT_ACQUIRE_SILENT_FLAG,
    CRYPT_ACQUIRE_WINDOW_HANDLE_FLAG, CRYPT_ALGORITHM_IDENTIFIER, CRYPT_ATTRIBUTE,
    CRYPT_INTEGER_BLOB, CertCloseStore, CertCreateCertificateContext, CertEnumCertificatesInStore,
    CertFindCertificateInStore, CertFreeCertificateChain, CertFreeCertificateContext,
    CertGetCertificateChain, CertGetCertificateContextProperty, CertGetIntendedKeyUsage,
    CertGetNameStringW, CertOpenStore, CertVerifyTimeValidity, CryptAcquireCertificatePrivateKey,
    CryptEncodeObject, CryptMsgClose, CryptMsgGetParam, CryptMsgOpenToEncode, CryptMsgUpdate,
    CryptReleaseContext, HCERTSTORE, NCryptFreeObject, PFXImportCertStore, PKCS_7_ASN_ENCODING,
    PKCS12_ALWAYS_CNG_KSP, PKCS12_NO_PERSIST_KEY, X509_ASN_ENCODING, szOID_COMMON_NAME,
    szOID_NIST_sha256, szOID_RSA_signingTime,
};
use zeroize::Zeroizing;

const ENCODING: u32 = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;
/// Longest name read from a certificate, in UTF-16 units (the worker's limit cuts it further).
const NAME_UNITS: usize = 256;
/// How many 100-nanosecond intervals there are from 1601-01-01 (Windows' epoch) to 1970-01-01.
const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;

/// Why a certificate does not sign. What the user is told says no more than this: never the
/// password, and not which of the two a wrong one and a file that is no certificate are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningError {
    /// The password is wrong, or the file is not a certificate with its private key (PKCS #12).
    Unreadable,
    Expired,
    NotYetValid,
    /// The certificate is not for signatures (its key usage), or comes without its private key.
    NotForSigning,
    /// The certificate of the store is no longer there, or its key cannot be reached: its smart
    /// card was taken out.
    Unavailable,
    /// The user closed the window that asked for the PIN.
    Cancelled,
    /// Windows did not make the signature.
    Failed,
}

impl From<CertificateProblem> for SigningError {
    fn from(problem: CertificateProblem) -> Self {
        match problem {
            CertificateProblem::Expired => Self::Expired,
            CertificateProblem::NotYetValid => Self::NotYetValid,
            CertificateProblem::NotForSigning => Self::NotForSigning,
        }
    }
}

impl From<SigningError> for IpcError {
    fn from(error: SigningError) -> Self {
        let (code, message) = match error {
            SigningError::Unreadable => (
                ErrorCode::CertificateUnreadable,
                "the password is wrong, or the file is not a certificate",
            ),
            SigningError::Expired => (ErrorCode::CertificateExpired, "the certificate expired"),
            SigningError::NotYetValid => (
                ErrorCode::CertificateNotYetValid,
                "the certificate is not valid yet",
            ),
            SigningError::NotForSigning => (
                ErrorCode::CertificateNotForSigning,
                "the certificate is not for signing",
            ),
            SigningError::Unavailable => (
                ErrorCode::CertificateUnavailable,
                "the certificate or its key cannot be reached",
            ),
            SigningError::Cancelled => (ErrorCode::Cancelled, "the PIN was not given"),
            SigningError::Failed => (ErrorCode::Internal, "Windows did not sign"),
        };
        IpcError {
            code,
            message: message.to_owned(),
        }
    }
}

/// The certificate file the user chose last to sign with (B3-06): its bytes, kept in the main
/// process until another is chosen or the app ends, never on disk; the page has only its number.
/// They are encrypted with the file's password, and wiped when dropped all the same.
#[derive(Default)]
pub struct ChosenCertificate {
    chosen: Mutex<Option<(u32, Zeroizing<Vec<u8>>)>>,
}

impl ChosenCertificate {
    /// Keeps `bytes` as the file chosen last, in place of the one before; gives its number.
    pub fn choose(&self, bytes: Zeroizing<Vec<u8>>) -> u32 {
        let mut chosen = self
            .chosen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let number = chosen
            .as_ref()
            .map_or(1, |(number, _)| number.wrapping_add(1).max(1));
        *chosen = Some((number, bytes));
        number
    }

    /// The bytes of file `number`, if it is still the one chosen last.
    pub fn get(&self, number: u32) -> Option<Zeroizing<Vec<u8>>> {
        let chosen = self
            .chosen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*chosen {
            Some((kept, bytes)) if *kept == number => Some(bytes.clone()),
            _ => None,
        }
    }
}

/// The certificates of the user's store listed last for signing (B3-06), by number: what
/// `sign_document` finds a number in. They are public; a list given again replaces them, and its
/// numbers are new, so a number from an older list finds nothing.
#[derive(Default)]
pub struct ListedCertificates {
    listed: Mutex<Listed>,
}

#[derive(Default)]
struct Listed {
    /// The number given last.
    last: u32,
    /// Each certificate of the list given last, with its number.
    kept: Vec<(u32, Vec<u8>)>,
}

impl ListedCertificates {
    /// Keeps `encoded` in place of the list before; gives the number of each, in order.
    pub fn list(&self, encoded: Vec<Vec<u8>>) -> Vec<u32> {
        let mut listed = self
            .listed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Listed { last, kept } = &mut *listed;
        kept.clear();
        encoded
            .into_iter()
            .map(|certificate| {
                *last = last.wrapping_add(1).max(1);
                kept.push((*last, certificate));
                *last
            })
            .collect()
    }

    /// The DER of certificate `number`, if the list given last has it.
    pub fn get(&self, number: u32) -> Option<Vec<u8>> {
        let listed = self
            .listed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        listed
            .kept
            .iter()
            .find(|(kept, _)| *kept == number)
            .map(|(_, certificate)| certificate.clone())
    }
}

/// Reads the certificate file at `path`, which is at most `MAX_CERTIFICATE_FILE_BYTES` long (a
/// PKCS #12 file is a few kilobytes): its content is for Windows to read.
pub fn read_certificate_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, IpcError> {
    let unreadable = || IpcError::from(SigningError::Unreadable);
    let mut bytes = Zeroizing::new(Vec::new());
    File::open(path)
        .and_then(|file| {
            file.take(MAX_CERTIFICATE_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| unreadable())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_CERTIFICATE_FILE_BYTES {
        return Err(unreadable());
    }
    Ok(bytes)
}

/// A certificate Windows handed out; freed when dropped.
struct Certificate(*const CERT_CONTEXT);

impl Drop for Certificate {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the context came from a CryptoAPI call that hands out a reference to free,
            // and this is the only owner.
            unsafe { CertFreeCertificateContext(self.0) };
        }
    }
}

/// The store a certificate file was imported into, which ends its key when closed, or the
/// user's own; closed when dropped.
struct Store(HCERTSTORE);

impl Drop for Store {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the store came from `PFXImportCertStore` or `CertOpenStore` and is closed
            // once. Contexts still open keep it until they are freed.
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

/// A message being encoded; closed when dropped.
struct Message(*mut c_void);

impl Drop for Message {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the message came from `CryptMsgOpenToEncode` and is closed once.
            unsafe { CryptMsgClose(self.0) };
        }
    }
}

/// The private key of a certificate, as `CryptAcquireCertificatePrivateKey` gave it: a CNG key or
/// an older provider. Released when dropped, if it is this one's to release.
struct Key {
    handle: usize,
    spec: u32,
    release: bool,
}

impl Drop for Key {
    fn drop(&mut self) {
        if !self.release || self.handle == 0 {
            return;
        }
        // SAFETY: the handle came from `CryptAcquireCertificatePrivateKey`, which said the caller
        // frees it, in the way its key spec says; it is freed once.
        unsafe {
            if self.spec == CERT_NCRYPT_KEY_SPEC {
                NCryptFreeObject(self.handle);
            } else {
                CryptReleaseContext(self.handle, 0);
            }
        }
    }
}

/// A certificate with its private key, ready to sign; the key goes when this is dropped.
pub struct Signer {
    // Dropped in this order: the key, then the certificate, then the store.
    key: Key,
    certificate: Certificate,
    store: Store,
}

// SAFETY: the handles are only used through `&self` by one call at a time (signing is one command
// that holds the signer), and CryptoAPI's handles are not tied to the thread that made them.
unsafe impl Send for Signer {}

impl Signer {
    /// Imports the certificate file `pfx` with `password`, and takes its first certificate that
    /// has a private key. Refused: a wrong password or a file that is not a certificate
    /// (`Unreadable`), a certificate with no private key or not for signatures
    /// (`NotForSigning`), one that has expired or is not valid yet, at this computer's time.
    pub fn from_pfx(pfx: &[u8], password: &str) -> Result<Self, SigningError> {
        let length = u32::try_from(pfx.len()).map_err(|_| SigningError::Unreadable)?;
        let blob = CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: pfx.as_ptr().cast_mut(),
        };
        let password: Zeroizing<Vec<u16>> =
            Zeroizing::new(password.encode_utf16().chain(Some(0)).collect());
        // SAFETY: `blob` points at `length` live bytes that Windows only reads, and the password
        // is NUL-terminated; both outlive the call. The store is closed by `Store`. Without
        // `CRYPT_EXPORTABLE` the key cannot be exported, and with `PKCS12_NO_PERSIST_KEY` it is
        // never written to disk.
        let store = Store(unsafe {
            PFXImportCertStore(
                &blob,
                password.as_ptr(),
                PKCS12_NO_PERSIST_KEY | PKCS12_ALWAYS_CNG_KSP,
            )
        });
        drop(password);
        if store.0.is_null() {
            return Err(SigningError::Unreadable);
        }
        // SAFETY: a live store; the context found is freed by `Certificate`.
        let certificate = Certificate(unsafe {
            CertFindCertificateInStore(
                store.0,
                ENCODING,
                0,
                CERT_FIND_HAS_PRIVATE_KEY,
                null(),
                null(),
            )
        });
        if certificate.0.is_null() {
            return Err(SigningError::NotForSigning);
        }
        // Silent: a key from a file needs no PIN, and nothing is shown. Cached: a key kept only
        // in memory is on the certificate as its key context (it has no provider to open it by),
        // which Windows gives only to a caller that takes the cached key; the certificate frees it.
        Self::with_key(
            certificate,
            store,
            CRYPT_ACQUIRE_CACHE_FLAG
                | CRYPT_ACQUIRE_ALLOW_NCRYPT_KEY_FLAG
                | CRYPT_ACQUIRE_SILENT_FLAG,
            null(),
            SigningError::NotForSigning,
        )
    }

    /// Takes the certificate `encoded` (one of [`store_certificates`]) from the user's own store,
    /// with its private key where Windows keeps it: in the user's key store, or on a smart card.
    /// Nothing is imported or copied. Windows, or the card's own software, may ask for a PIN in a
    /// window that `window` owns, now or when signing; the app never sees it. Refused: a
    /// certificate no longer in the store, or whose key cannot be reached (`Unavailable`: its
    /// card was taken out), and one that has expired, is not valid yet, or is not for
    /// signatures.
    pub fn from_store(encoded: &[u8], window: isize) -> Result<Self, SigningError> {
        let store = personal_store().ok_or(SigningError::Unavailable)?;
        let length = u32::try_from(encoded.len()).map_err(|_| SigningError::Unavailable)?;
        // SAFETY: `length` live bytes that Windows only reads (it keeps a copy); the context
        // made is freed by `Certificate`.
        let wanted = Certificate(unsafe {
            CertCreateCertificateContext(ENCODING, encoded.as_ptr(), length)
        });
        if wanted.0.is_null() {
            return Err(SigningError::Unavailable);
        }
        // SAFETY: a live store and a live context to look for; the context found is the store's
        // own, with the properties that say where its key is, and is freed by `Certificate`.
        let certificate = Certificate(unsafe {
            CertFindCertificateInStore(
                store.0,
                ENCODING,
                0,
                CERT_FIND_EXISTING,
                wanted.0.cast(),
                null(),
            )
        });
        drop(wanted);
        if certificate.0.is_null() {
            return Err(SigningError::Unavailable);
        }
        let owner = window as *mut c_void;
        // Not silent: a card's key may need its PIN. CNG first: a key that an older provider
        // keeps is then opened by its key store where it can be, which signs with SHA-256.
        Self::with_key(
            certificate,
            store,
            CRYPT_ACQUIRE_PREFER_NCRYPT_KEY_FLAG | CRYPT_ACQUIRE_WINDOW_HANDLE_FLAG,
            std::ptr::from_ref(&owner).cast(),
            SigningError::Unavailable,
        )
    }

    /// The signer of `certificate`, which is in `store`, once it is valid now and for
    /// signatures, and its private key is at hand: asked for with `flags` and `parameter` as
    /// `CryptAcquireCertificatePrivateKey` takes them, or `missing` when it is not.
    fn with_key(
        certificate: Certificate,
        store: Store,
        flags: u32,
        parameter: *const c_void,
        missing: SigningError,
    ) -> Result<Self, SigningError> {
        // SAFETY: a live certificate context, whose `pCertInfo` lives as long as it does.
        if let Some(problem) = problem_of(unsafe { (*certificate.0).pCertInfo }) {
            return Err(problem.into());
        }
        let mut handle = 0;
        let mut spec = 0;
        let mut release = 0;
        // SAFETY: a live certificate context, the parameter its flags ask for (a window handle
        // that lives through the call, or none), and three places for what the call gives.
        let acquired = unsafe {
            CryptAcquireCertificatePrivateKey(
                certificate.0,
                flags,
                parameter,
                &mut handle,
                &mut spec,
                &mut release,
            )
        };
        let key = Key {
            handle,
            spec,
            release: release != 0,
        };
        if acquired == 0 {
            return Err(match last_failure() {
                SigningError::Cancelled => SigningError::Cancelled,
                _ => missing,
            });
        }
        Ok(Self {
            key,
            certificate,
            store,
        })
    }

    /// Who signs: the common name of the certificate's subject, or, if it has none, the name
    /// Windows displays for it.
    pub fn signer_name(&self) -> String {
        display_name(self.certificate.0, 0)
    }

    /// The DER of a detached CMS `SignedData` over `parts`, in order, as if they were one: signed
    /// with SHA-256 at `unix_seconds` (its signing time), with the signer's certificate and the
    /// chain of it this computer has. Nothing is fetched: the chain is built from the certificate
    /// file and the certificates Windows already has (`CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`, and
    /// no update of the roots); one that cannot be built leaves the signer's certificate alone.
    pub fn sign(&self, parts: [&[u8]; 2], unix_seconds: i64) -> Result<Vec<u8>, SigningError> {
        let failed = |_| SigningError::Failed;
        let time = filetime(unix_seconds).ok_or(SigningError::Failed)?;
        let mut encoded_time = encode_signing_time(&time)?;
        let mut time_value = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(encoded_time.len()).map_err(failed)?,
            pbData: encoded_time.as_mut_ptr(),
        };
        let mut signing_time = CRYPT_ATTRIBUTE {
            pszObjId: szOID_RSA_signingTime.cast_mut(),
            cValue: 1,
            rgValue: &mut time_value,
        };
        let chain = self.chain();
        let mut certificates = chain_certificates(&chain, &self.certificate);
        // SAFETY: a live certificate context.
        let info = unsafe { (*self.certificate.0).pCertInfo };
        let mut signer = CMSG_SIGNER_ENCODE_INFO {
            cbSize: size_of::<CMSG_SIGNER_ENCODE_INFO>() as u32,
            pCertInfo: info,
            // Windows tells a CNG key from a provider by the handle itself.
            Anonymous: CMSG_SIGNER_ENCODE_INFO_0 {
                hCryptProv: self.key.handle,
            },
            dwKeySpec: self.key.spec,
            HashAlgorithm: CRYPT_ALGORITHM_IDENTIFIER {
                pszObjId: szOID_NIST_sha256.cast_mut(),
                Parameters: CRYPT_INTEGER_BLOB::default(),
            },
            pvHashAuxInfo: null_mut(),
            // With an attribute signed, Windows adds the content type and the digest too.
            cAuthAttr: 1,
            rgAuthAttr: &mut signing_time,
            cUnauthAttr: 0,
            rgUnauthAttr: null_mut(),
        };
        let encode_info = CMSG_SIGNED_ENCODE_INFO {
            cbSize: size_of::<CMSG_SIGNED_ENCODE_INFO>() as u32,
            cSigners: 1,
            rgSigners: &mut signer,
            cCertEncoded: u32::try_from(certificates.len()).map_err(failed)?,
            rgCertEncoded: certificates.as_mut_ptr(),
            cCrlEncoded: 0,
            rgCrlEncoded: null_mut(),
        };
        // SAFETY: `encode_info` and everything it points at (the signer, its attribute and the
        // encoded time, the certificates the chain and the signer's context keep alive) live
        // until the message is closed below; no inner type means data, and no stream.
        let message = Message(unsafe {
            CryptMsgOpenToEncode(
                ENCODING,
                CMSG_DETACHED_FLAG,
                CMSG_SIGNED,
                std::ptr::from_ref(&encode_info).cast(),
                null(),
                null(),
            )
        });
        if message.0.is_null() {
            return Err(last_failure());
        }
        for (index, part) in parts.iter().enumerate() {
            let length = u32::try_from(part.len()).map_err(failed)?;
            // SAFETY: a live message, and a live buffer of the length given; the last part is
            // the final one.
            let updated = unsafe {
                CryptMsgUpdate(
                    message.0,
                    part.as_ptr(),
                    length,
                    i32::from(index == parts.len() - 1),
                )
            };
            // A card's key signs here, and its PIN may be asked for.
            if updated == 0 {
                return Err(last_failure());
            }
        }
        let mut size = 0;
        // SAFETY: a live message; with no buffer the call only says how large the content is.
        let sized =
            unsafe { CryptMsgGetParam(message.0, CMSG_CONTENT_PARAM, 0, null_mut(), &mut size) };
        if sized == 0 {
            return Err(SigningError::Failed);
        }
        let mut signature = vec![0u8; size as usize];
        // SAFETY: a live message, and a buffer of `size` bytes, the size the call asked for.
        let got = unsafe {
            CryptMsgGetParam(
                message.0,
                CMSG_CONTENT_PARAM,
                0,
                signature.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if got == 0 {
            return Err(SigningError::Failed);
        }
        signature.truncate(size as usize);
        drop(message);
        drop(chain);
        Ok(signature)
    }

    /// The chain of the signer's certificate, built from what the certificate file had and what
    /// Windows has on this computer, with nothing fetched. Null if it cannot be built.
    fn chain(&self) -> Chain {
        let para = CERT_CHAIN_PARA {
            cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
            ..Default::default()
        };
        let mut chain: *mut CERT_CHAIN_CONTEXT = null_mut();
        // SAFETY: a live certificate context, the default chain engine and no time (now), the
        // certificate file's store, and `para` filled in as the API wants; the chain is freed by
        // `Chain`. No flag asks for revocation, and URL retrieval is limited to the cache.
        let built = unsafe {
            CertGetCertificateChain(
                null_mut(),
                self.certificate.0,
                null(),
                self.store.0,
                &para,
                CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE,
                null(),
                &mut chain,
            )
        };
        if built == 0 {
            return Chain(null());
        }
        Chain(chain)
    }
}

/// The certificates a signature carries: those of the first simple chain of `chain`, the signer's
/// first; or the signer's alone. They point into `chain` and `signer`, which must outlive them.
fn chain_certificates(chain: &Chain, signer: &Certificate) -> Vec<CRYPT_INTEGER_BLOB> {
    let blob = |context: *const CERT_CONTEXT| {
        // SAFETY: a live certificate context, whose encoded bytes live as long as it does.
        unsafe {
            CRYPT_INTEGER_BLOB {
                cbData: (*context).cbCertEncoded,
                pbData: (*context).pbCertEncoded,
            }
        }
    };
    let mut certificates = Vec::new();
    // SAFETY: a chain context that `CertGetCertificateChain` built and nothing has freed: its
    // arrays have the lengths it gives, and their elements are live.
    unsafe {
        if !chain.0.is_null() && (*chain.0).cChain > 0 {
            let simple = *(*chain.0).rgpChain;
            for index in 0..(*simple).cElement as usize {
                let element = *(*simple).rgpElement.add(index);
                if !(*element).pCertContext.is_null() {
                    certificates.push(blob((*element).pCertContext));
                }
            }
        }
    }
    if certificates.is_empty() {
        certificates.push(blob(signer.0));
    }
    certificates
}

/// A certificate of the user's store that the signing dialog lists (B3-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedCertificate {
    /// Its DER, which finds it in the store again.
    pub encoded: Vec<u8>,
    pub subject: String,
    pub issuer: String,
    pub expires: CertificateDate,
    pub problem: Option<CertificateProblem>,
}

/// The certificates of the current user's Personal store that have a private key: in the
/// user's key store, or on a smart card Windows knows of (Windows puts a card's certificates
/// there while it is in). Those that can sign come first; at most `MAX_STORE_CERTIFICATES`.
/// Only the store is read: no key is opened, no card is touched, and nothing is asked.
pub fn store_certificates() -> Vec<ListedCertificate> {
    let Some(store) = personal_store() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut context: *const CERT_CONTEXT = null();
    loop {
        // SAFETY: a live store, and the context it gave last (or none, to begin): the call frees
        // that one and gives the next, or null at the end.
        context = unsafe { CertEnumCertificatesInStore(store.0, context) };
        if context.is_null() {
            break;
        }
        if has_private_key(context) {
            found.push(listed(context));
        }
    }
    found.sort_by_key(|certificate| certificate.problem.is_some());
    found.truncate(MAX_STORE_CERTIFICATES);
    found
}

/// What the signing dialog shows of the live certificate `context`.
fn listed(context: *const CERT_CONTEXT) -> ListedCertificate {
    // SAFETY: a live context: its encoded bytes have the length it gives, and they and its
    // information live as long as it does.
    let (encoded, info) = unsafe {
        let encoded =
            std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize);
        (encoded.to_vec(), (*context).pCertInfo)
    };
    // SAFETY: as above.
    let not_after = unsafe { (*info).NotAfter };
    ListedCertificate {
        encoded,
        subject: display_name(context, 0),
        issuer: display_name(context, CERT_NAME_ISSUER_FLAG),
        expires: date_of(&not_after),
        problem: problem_of(info),
    }
}

/// Whether Windows knows where the key of the live certificate `context` is (a key store, an
/// older provider, a card's driver). The key itself is not opened.
fn has_private_key(context: *const CERT_CONTEXT) -> bool {
    let mut size = 0;
    // SAFETY: a live context; with no buffer the call only says whether the property is there.
    unsafe {
        CertGetCertificateContextProperty(
            context,
            CERT_KEY_PROV_INFO_PROP_ID,
            null_mut(),
            &mut size,
        ) != 0
    }
}

/// The current user's Personal store ("MY"), to read; none if it cannot be opened.
fn personal_store() -> Option<Store> {
    let name: Vec<u16> = "MY".encode_utf16().chain(Some(0)).collect();
    // SAFETY: the system store provider takes the store's name as a NUL-terminated UTF-16
    // string, which outlives the call; the store is closed by `Store`.
    let store = Store(unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            CERT_SYSTEM_STORE_CURRENT_USER
                | CERT_STORE_READONLY_FLAG
                | CERT_STORE_OPEN_EXISTING_FLAG,
            name.as_ptr().cast(),
        )
    });
    (!store.0.is_null()).then_some(store)
}

/// Why the certificate `info` cannot sign now: it has expired, is not valid yet, or is not for
/// signatures; none if it can.
fn problem_of(info: *const CERT_INFO) -> Option<CertificateProblem> {
    // SAFETY: no time means now; `info` is a live certificate's.
    match unsafe { CertVerifyTimeValidity(null(), info) } {
        0 => {}
        before if before < 0 => return Some(CertificateProblem::NotYetValid),
        _ => return Some(CertificateProblem::Expired),
    }
    (!for_signing(info)).then_some(CertificateProblem::NotForSigning)
}

/// Why Windows did not sign, as the last error of this thread says: the user closed the window
/// that asked for the PIN (`Cancelled`), the card is not in (`Unavailable`), or anything else.
fn last_failure() -> SigningError {
    // SAFETY: reads this thread's last error, which the call that failed set.
    failure_of(unsafe { GetLastError() })
}

fn failure_of(code: u32) -> SigningError {
    let is = |hresult: i32| code == hresult as u32;
    if code == ERROR_CANCELLED
        || code == 0x8007_0000 | ERROR_CANCELLED
        || is(SCARD_W_CANCELLED_BY_USER)
        || is(SCARD_E_CANCELLED)
        || is(NTE_USER_CANCELLED)
    {
        SigningError::Cancelled
    } else if is(SCARD_W_REMOVED_CARD)
        || is(SCARD_E_NO_SMARTCARD)
        || is(SCARD_E_READER_UNAVAILABLE)
        || is(SCARD_E_NO_READERS_AVAILABLE)
    {
        SigningError::Unavailable
    } else {
        SigningError::Failed
    }
}

/// Whether the certificate may sign: its key usage includes digital signatures or
/// non-repudiation, or it says nothing of its key usage.
fn for_signing(info: *const CERT_INFO) -> bool {
    let mut usage = 0u8;
    // SAFETY: a live certificate's information, and one byte to fill in (the bits that matter
    // are in the first).
    let has = unsafe { CertGetIntendedKeyUsage(ENCODING, info, &mut usage, 1) };
    if has == 0 {
        // SAFETY: no other call came between the one that failed and this.
        return unsafe { GetLastError() } == 0;
    }
    u32::from(usage) & (CERT_DIGITAL_SIGNATURE_KEY_USAGE | CERT_NON_REPUDIATION_KEY_USAGE) != 0
}

/// Whom the live certificate `context` names (with `CERT_NAME_ISSUER_FLAG`: who issued it): the
/// common name, or, if there is none, the name Windows displays; plain words, bounded.
fn display_name(context: *const CERT_CONTEXT, flags: u32) -> String {
    let common = name_of(
        context,
        CERT_NAME_ATTR_TYPE,
        flags,
        szOID_COMMON_NAME.cast(),
    );
    let name = if common.trim().is_empty() {
        name_of(context, CERT_NAME_SIMPLE_DISPLAY_TYPE, flags, null())
    } else {
        common
    };
    clean_display_text(&name, MAX_SIGNATURE_TEXT_BYTES as usize)
}

fn name_of(
    context: *const CERT_CONTEXT,
    kind: u32,
    flags: u32,
    parameter: *const c_void,
) -> String {
    let mut name = [0u16; NAME_UNITS];
    // SAFETY: a live certificate context, a parameter the kind of name takes (an attribute's
    // object identifier, or none), and a buffer of the length given.
    let length = unsafe {
        CertGetNameStringW(
            context,
            kind,
            flags,
            parameter,
            name.as_mut_ptr(),
            NAME_UNITS as u32,
        )
    };
    // The length counts the terminating NUL.
    let units = (length as usize).saturating_sub(1).min(NAME_UNITS);
    String::from_utf16_lossy(&name[..units])
}

/// The DER of the time of signing, as the CMS attribute takes it (`signingTime`).
fn encode_signing_time(time: &FILETIME) -> Result<Vec<u8>, SigningError> {
    let what = szOID_RSA_signingTime;
    let mut size = 0;
    // SAFETY: a live time; with no buffer the call only says how large the encoding is.
    if unsafe {
        CryptEncodeObject(
            X509_ASN_ENCODING,
            what,
            std::ptr::from_ref(time).cast(),
            null_mut(),
            &mut size,
        )
    } == 0
    {
        return Err(SigningError::Failed);
    }
    let mut encoded = vec![0u8; size as usize];
    // SAFETY: a live time, and a buffer of `size` bytes, the size the call asked for.
    if unsafe {
        CryptEncodeObject(
            X509_ASN_ENCODING,
            what,
            std::ptr::from_ref(time).cast(),
            encoded.as_mut_ptr(),
            &mut size,
        )
    } == 0
    {
        return Err(SigningError::Failed);
    }
    encoded.truncate(size as usize);
    Ok(encoded)
}

/// The day of `time` in UTC, as a certificate's validity is shown; 9999-12-31, the last day a
/// certificate can name, for a time beyond it.
fn date_of(time: &FILETIME) -> CertificateDate {
    let ticks = u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    let unix_seconds = (i128::from(ticks) - i128::from(UNIX_EPOCH_AS_FILETIME)) / 10_000_000;
    i64::try_from(unix_seconds)
        .ok()
        .and_then(|seconds| local_time(seconds, 0))
        .filter(|day| day.year <= 9999)
        .map_or(
            CertificateDate {
                year: 9999,
                month: 12,
                day: 31,
            },
            |day| CertificateDate {
                year: day.year,
                month: day.month,
                day: day.day,
            },
        )
}

/// `unix_seconds` as Windows counts time; none before 1970.
fn filetime(unix_seconds: i64) -> Option<FILETIME> {
    let ticks = u64::try_from(unix_seconds)
        .ok()?
        .checked_mul(10_000_000)?
        .checked_add(UNIX_EPOCH_AS_FILETIME)?;
    Some(FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    })
}

/// The time on the computer's clock at `unix_seconds`, in the time zone `utc_offset_minutes` east
/// of UTC: what a signature's `/M` says, and its box shows. The days are counted as the proleptic
/// Gregorian calendar does (Howard Hinnant's `civil_from_days`).
pub fn local_time(unix_seconds: i64, utc_offset_minutes: i16) -> Option<SigningTime> {
    let local = unix_seconds.checked_add(i64::from(utc_offset_minutes) * 60)?;
    let (days, seconds) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Some(SigningTime {
        year: u16::try_from(year).ok()?,
        month: month as u8,
        day: day as u8,
        hour: (seconds / 3_600) as u8,
        minute: (seconds % 3_600 / 60) as u8,
        second: (seconds % 60) as u8,
        utc_offset_minutes,
    })
}

/// Whether `room` is what a prepared copy keeps for its signature (`/Contents`): `<`, an even
/// number of hexadecimal zeros, `>`.
pub fn is_room(room: &[u8]) -> bool {
    matches!(room, [b'<', digits @ .., b'>'] if digits.len() % 2 == 0 && digits.iter().all(|&digit| digit == b'0'))
}

/// Writes `signature` as hexadecimal digits into `room` (see [`is_room`]). The zeros it does not
/// need stay (a reader takes them as padding). `false` when `room` is not a room, or the
/// signature does not fit it.
pub fn fill_room(room: &mut [u8], signature: &[u8]) -> bool {
    if !is_room(room) || 2 * signature.len() > room.len() - 2 {
        return false;
    }
    let end = room.len() - 1;
    let digits = &mut room[1..end];
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for (pair, byte) in digits.as_chunks_mut::<2>().0.iter_mut().zip(signature) {
        *pair = [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 0x0f)]];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use windows_sys::Win32::Security::Cryptography::{
        CERT_STORE_ADD_REPLACE_EXISTING, CRYPT_DELETEKEYSET, CRYPT_KEY_PROV_INFO,
        CRYPT_VERIFY_MESSAGE_PARA, CertAddCertificateContextToStore,
        CertDeleteCertificateFromStore, CryptAcquireContextW, CryptVerifyDetachedMessageSignature,
        NCryptDeleteKey, NCryptOpenKey, NCryptOpenStorageProvider,
    };

    #[test]
    fn local_time_counts_days_months_and_years() {
        let time = |seconds, offset| {
            let t = local_time(seconds, offset).unwrap();
            (t.year, t.month, t.day, t.hour, t.minute, t.second)
        };
        assert_eq!(time(0, 0), (1970, 1, 1, 0, 0, 0));
        // 2026-10-22 01:05:07 UTC.
        assert_eq!(time(1_792_631_107, 0), (2026, 10, 22, 1, 5, 7));
        assert_eq!(time(1_792_631_107, 480), (2026, 10, 22, 9, 5, 7));
        assert_eq!(time(1_792_631_107, -210), (2026, 10, 21, 21, 35, 7));
        // A leap day, and the day after it.
        assert_eq!(time(1_709_164_800, 0), (2024, 2, 29, 0, 0, 0));
        assert_eq!(time(1_709_251_200, 0), (2024, 3, 1, 0, 0, 0));
        // The last second of a year.
        assert_eq!(time(1_798_761_599, 0), (2026, 12, 31, 23, 59, 59));
        assert_eq!(local_time(i64::MAX, 60), None);
    }

    #[test]
    fn a_signature_fills_its_room_and_nothing_else() {
        let mut room = *b"<00000000>";
        assert!(fill_room(&mut room, &[0xAB, 0x01]));
        assert_eq!(&room, b"<AB010000>");
        // Too large, or not a room of zeros: left as it was.
        let mut small = *b"<0000>";
        assert!(!fill_room(&mut small, &[1, 2, 3]));
        assert_eq!(&small, b"<0000>");
        let mut used = *b"<00A0>";
        assert!(!fill_room(&mut used, &[1]));
        let mut open = *b"0000>";
        assert!(!fill_room(&mut open, &[1]));
        let mut full = *b"<00>";
        assert!(fill_room(&mut full, &[0xFF]));
        assert_eq!(&full, b"<FF>");
        assert!(is_room(b"<>"));
        assert!(!is_room(b"<000>"));
        assert!(!is_room(b"<"));
    }

    #[test]
    fn only_the_certificate_file_chosen_last_is_given() {
        let chosen = ChosenCertificate::default();
        assert_eq!(chosen.get(1), None);
        let first = chosen.choose(Zeroizing::new(b"first".to_vec()));
        assert_eq!(
            chosen.get(first).as_deref().map(Vec::as_slice),
            Some(&b"first"[..])
        );
        let second = chosen.choose(Zeroizing::new(b"second".to_vec()));
        assert_ne!(first, second);
        assert_eq!(chosen.get(first), None);
        assert_eq!(
            chosen.get(second).as_deref().map(Vec::as_slice),
            Some(&b"second"[..])
        );
        assert_eq!(chosen.get(0), None);
    }

    #[test]
    fn a_certificate_listed_is_found_by_its_number_in_the_last_list_alone() {
        let listed = ListedCertificates::default();
        assert_eq!(listed.get(1), None);
        let first = listed.list(vec![b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1]);
        assert_eq!(listed.get(first[1]), Some(b"two".to_vec()));
        // Listed again: new numbers, and the old ones find nothing.
        let second = listed.list(vec![b"two".to_vec()]);
        assert!(!first.contains(&second[0]));
        assert_eq!(listed.get(first[1]), None);
        assert_eq!(listed.get(second[0]), Some(b"two".to_vec()));
        assert_eq!(listed.get(0), None);
        assert!(listed.list(Vec::new()).is_empty());
        assert_eq!(listed.get(second[0]), None);
    }

    #[test]
    fn a_pin_not_given_and_a_card_taken_out_are_told_from_other_failures() {
        for code in [
            ERROR_CANCELLED,
            0x8007_04C7,
            SCARD_W_CANCELLED_BY_USER as u32,
            SCARD_E_CANCELLED as u32,
            NTE_USER_CANCELLED as u32,
        ] {
            assert_eq!(failure_of(code), SigningError::Cancelled, "{code:#x}");
        }
        for code in [
            SCARD_W_REMOVED_CARD as u32,
            SCARD_E_NO_SMARTCARD as u32,
            SCARD_E_READER_UNAVAILABLE as u32,
        ] {
            assert_eq!(failure_of(code), SigningError::Unavailable, "{code:#x}");
        }
        assert_eq!(failure_of(0), SigningError::Failed);
        assert_eq!(failure_of(0x8009_0008), SigningError::Failed);
        // Not given, the PIN is no error to show; a card taken out says so.
        assert_eq!(
            IpcError::from(SigningError::Cancelled).code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            IpcError::from(SigningError::Unavailable).code,
            ErrorCode::CertificateUnavailable
        );
    }

    #[test]
    fn a_certificate_lasts_to_a_day_in_utc() {
        let at = |unix_seconds: u64| {
            let ticks = unix_seconds * 10_000_000 + UNIX_EPOCH_AS_FILETIME;
            date_of(&FILETIME {
                dwLowDateTime: ticks as u32,
                dwHighDateTime: (ticks >> 32) as u32,
            })
        };
        let day = |year, month, day| CertificateDate { year, month, day };
        // 2036-01-01 00:00:00 UTC, and the second before it.
        assert_eq!(at(2_082_758_400), day(2036, 1, 1));
        assert_eq!(at(2_082_758_399), day(2035, 12, 31));
        assert_eq!(at(0), day(1970, 1, 1));
        let last = FILETIME {
            dwLowDateTime: u32::MAX,
            dwHighDateTime: u32::MAX,
        };
        assert_eq!(date_of(&last).year, 9999);
    }

    #[test]
    fn a_time_is_counted_from_windows_epoch() {
        let time = filetime(0).unwrap();
        let ticks = u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
        assert_eq!(ticks, UNIX_EPOCH_AS_FILETIME);
        assert!(filetime(-1).is_none());
    }

    // Importing a certificate file adds it to Windows' own key machinery, on whatever computer
    // the tests run: only on a runner that is thrown away (CI), or when asked for with
    // `PDF_READER_TEST_SIGNING=1`.
    fn may_import() -> bool {
        if std::env::var_os("CI").is_none()
            && std::env::var("PDF_READER_TEST_SIGNING").as_deref() != Ok("1")
        {
            eprintln!("skipped: it imports a test certificate (PDF_READER_TEST_SIGNING=1)");
            return false;
        }
        true
    }

    const PASSWORD: &str = "corpus-test-password";

    /// The tests that change what Windows keeps for the user take turns: one of them counts the
    /// files of the user's keys and certificates.
    static WINDOWS_KEYS: Mutex<()> = Mutex::new(());

    /// A test certificate put in the user's Personal store with its key kept by Windows, as an
    /// import or a smart card's driver leaves one: in a CNG key store (`cng`), or by an older
    /// provider. Taken out again, key and all, when dropped.
    struct InStore {
        context: *const CERT_CONTEXT,
        encoded: Vec<u8>,
        /// Whether an older provider (not a CNG key store) keeps its key.
        older: bool,
    }

    impl InStore {
        fn put(name: &str, cng: bool) -> Self {
            let pfx = certificate(name);
            let blob = CRYPT_INTEGER_BLOB {
                cbData: pfx.len() as u32,
                pbData: pfx.as_ptr().cast_mut(),
            };
            let password: Vec<u16> = PASSWORD.encode_utf16().chain(Some(0)).collect();
            let flags = if cng { PKCS12_ALWAYS_CNG_KSP } else { 0 };
            // SAFETY: as in `Signer::from_pfx`, but the key is kept, so that the store's copy
            // has one.
            let imported = Store(unsafe { PFXImportCertStore(&blob, password.as_ptr(), flags) });
            assert!(!imported.0.is_null(), "{name} imported");
            // SAFETY: a live store; the context found is freed by `Certificate`.
            let found = Certificate(unsafe {
                CertFindCertificateInStore(
                    imported.0,
                    ENCODING,
                    0,
                    CERT_FIND_HAS_PRIVATE_KEY,
                    null(),
                    null(),
                )
            });
            assert!(!found.0.is_null(), "{name} has its key");
            let my: Vec<u16> = "MY".encode_utf16().chain(Some(0)).collect();
            // SAFETY: as in `personal_store`, to write; the context added keeps the store open
            // until it is deleted.
            let personal = Store(unsafe {
                CertOpenStore(
                    CERT_STORE_PROV_SYSTEM_W,
                    0,
                    0,
                    CERT_SYSTEM_STORE_CURRENT_USER,
                    my.as_ptr().cast(),
                )
            });
            assert!(!personal.0.is_null(), "the Personal store opens");
            let mut context = null_mut();
            // SAFETY: a live store and context; the store's copy, with the properties that say
            // where the key is, is given back, and deleted when this is dropped.
            let added = unsafe {
                CertAddCertificateContextToStore(
                    personal.0,
                    found.0,
                    CERT_STORE_ADD_REPLACE_EXISTING,
                    &mut context,
                )
            };
            assert!(added != 0, "{name} put in the store");
            let older = key_provider(context).is_some_and(|(_, _, kind)| kind != 0);
            Self {
                context,
                encoded: listed(context).encoded,
                older,
            }
        }
    }

    impl Drop for InStore {
        fn drop(&mut self) {
            if let Some((container, provider, kind)) = key_provider(self.context) {
                // SAFETY: the names are NUL-terminated and live through the calls; deleting a
                // key frees its handle, and the key store's is freed.
                unsafe {
                    if kind == 0 {
                        let mut store = 0;
                        if NCryptOpenStorageProvider(&mut store, provider.as_ptr(), 0) == 0 {
                            let mut key = 0;
                            if NCryptOpenKey(store, &mut key, container.as_ptr(), 0, 0) == 0 {
                                NCryptDeleteKey(key, 0);
                            }
                            NCryptFreeObject(store);
                        }
                    } else {
                        let mut deleted = 0;
                        CryptAcquireContextW(
                            &mut deleted,
                            container.as_ptr(),
                            provider.as_ptr(),
                            kind,
                            CRYPT_DELETEKEYSET,
                        );
                    }
                }
            }
            // SAFETY: the store's own copy, which the call deletes and frees.
            unsafe { CertDeleteCertificateFromStore(self.context) };
        }
    }

    /// Where the key of the live certificate `context` is: its container's name, its
    /// provider's (both NUL-terminated), and the provider's type (0: a CNG key store).
    fn key_provider(context: *const CERT_CONTEXT) -> Option<(Vec<u16>, Vec<u16>, u32)> {
        let mut size = 0;
        // SAFETY: a live context; with no buffer the call says how large the property is.
        if unsafe {
            CertGetCertificateContextProperty(
                context,
                CERT_KEY_PROV_INFO_PROP_ID,
                null_mut(),
                &mut size,
            )
        } == 0
        {
            return None;
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: a buffer of at least `size` bytes, aligned for the structure it is given.
        if unsafe {
            CertGetCertificateContextProperty(
                context,
                CERT_KEY_PROV_INFO_PROP_ID,
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        } == 0
        {
            return None;
        }
        // SAFETY: the buffer starts with the structure, whose names are NUL-terminated strings
        // in the buffer.
        unsafe {
            let info = &*buffer.as_ptr().cast::<CRYPT_KEY_PROV_INFO>();
            Some((
                wide(info.pwszContainerName),
                wide(info.pwszProvName),
                info.dwProvType,
            ))
        }
    }

    /// The NUL-terminated string at `text`, with its NUL.
    ///
    /// # Safety
    ///
    /// `text` points at a live NUL-terminated UTF-16 string.
    unsafe fn wide(text: *const u16) -> Vec<u16> {
        let mut length = 0;
        // SAFETY: the caller gives a live NUL-terminated string: every unit up to its NUL is
        // there.
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
            std::slice::from_raw_parts(text, length + 1).to_vec()
        }
    }

    fn certificate(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/corpus/certificates")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// Whether Windows says that `cms` signs `parts`.
    fn holds(parts: [&[u8]; 2], cms: &[u8]) -> bool {
        let para = CRYPT_VERIFY_MESSAGE_PARA {
            cbSize: size_of::<CRYPT_VERIFY_MESSAGE_PARA>() as u32,
            dwMsgAndCertEncodingType: ENCODING,
            ..Default::default()
        };
        let pointers = parts.map(<[u8]>::as_ptr);
        let sizes = parts.map(|part| part.len() as u32);
        // SAFETY: live buffers of the sizes given; no signer certificate is asked for.
        unsafe {
            CryptVerifyDetachedMessageSignature(
                &para,
                0,
                cms.as_ptr(),
                cms.len() as u32,
                2,
                pointers.as_ptr(),
                sizes.as_ptr(),
                null_mut(),
            ) != 0
        }
    }

    /// Every file under the folders where Windows keeps keys and certificates of the user.
    fn key_files() -> Vec<std::path::PathBuf> {
        fn walk(folder: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(folder) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.push(path);
                }
            }
        }
        let mut found = Vec::new();
        for base in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(base) {
                let base = std::path::Path::new(&base).join("Microsoft");
                walk(&base.join("Crypto"), &mut found);
                walk(&base.join("SystemCertificates"), &mut found);
            }
        }
        found.sort();
        found
    }

    #[test]
    fn the_test_certificate_signs_two_parts_and_leaves_no_key_behind() {
        if !may_import() {
            return;
        }
        let _turn = WINDOWS_KEYS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = key_files();
        let signer = Signer::from_pfx(&certificate("signer.pfx"), PASSWORD).expect("imported");
        assert_eq!(
            signer.signer_name(),
            "PDF Reader test corpus signer (NOT TRUSTED)"
        );
        let parts: [&[u8]; 2] = [b"%PDF-1.7 the first part <", b"> the second part %%EOF"];
        let cms = signer.sign(parts, 1_792_631_107).expect("signed");
        assert!(cms.len() < 32 * 1024, "{} bytes", cms.len());
        assert!(holds(parts, &cms));
        // One byte changed, or the parts in the other order: it no longer holds.
        assert!(!holds(
            [b"%PDF-1.7 the first part <", b"> the second part %%EOX"],
            &cms
        ));
        assert!(!holds([parts[1], parts[0]], &cms));
        drop(signer);
        assert_eq!(key_files(), before, "no key or certificate was stored");
    }

    #[test]
    fn a_certificate_of_the_store_signs_with_its_key_where_windows_keeps_it() {
        if !may_import() {
            return;
        }
        let _turn = WINDOWS_KEYS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = key_files();
        let mut put_in = Vec::new();
        // A key in a CNG key store, then one kept by an older provider: as an import or a
        // smart card's driver can leave either (ADR 0020, POC 3).
        for cng in [true, false] {
            let signing = InStore::put("signer.pfx", cng);
            assert_eq!(signing.older, !cng, "the key is where it was put");
            let expired = InStore::put("expired.pfx", cng);
            let encryption_only = InStore::put("encryption-only.pfx", cng);
            put_in.extend([&signing, &expired, &encryption_only].map(|it| it.encoded.clone()));

            let listed = store_certificates();
            let problem = |wanted: &InStore| {
                listed
                    .iter()
                    .find(|certificate| certificate.encoded == wanted.encoded)
                    .map(|certificate| certificate.problem)
            };
            assert_eq!(problem(&signing), Some(None), "listed, and it can sign");
            assert_eq!(problem(&expired), Some(Some(CertificateProblem::Expired)));
            assert_eq!(
                problem(&encryption_only),
                Some(Some(CertificateProblem::NotForSigning))
            );
            // Those that can sign come first.
            let first_problem = listed
                .iter()
                .position(|certificate| certificate.problem.is_some())
                .unwrap();
            assert!(listed[first_problem..].iter().all(|c| c.problem.is_some()));
            let shown = listed
                .iter()
                .find(|certificate| certificate.encoded == signing.encoded)
                .unwrap();
            assert_eq!(shown.subject, "PDF Reader test corpus signer (NOT TRUSTED)");
            assert_eq!(shown.issuer, shown.subject, "it signed itself");
            assert_eq!(
                shown.expires,
                CertificateDate {
                    year: 2036,
                    month: 1,
                    day: 1
                }
            );

            let signer = Signer::from_store(&signing.encoded, 0).expect("its key is at hand");
            assert_eq!(
                signer.signer_name(),
                "PDF Reader test corpus signer (NOT TRUSTED)"
            );
            let parts: [&[u8]; 2] = [b"%PDF-1.7 the first part <", b"> the second part %%EOF"];
            let cms = signer.sign(parts, 1_792_631_107).expect("signed");
            assert!(holds(parts, &cms), "with a key cng={cng}");
            assert!(!holds([parts[1], parts[0]], &cms));
            drop(signer);
            assert_eq!(
                Signer::from_store(&expired.encoded, 0).err(),
                Some(SigningError::Expired)
            );
            assert_eq!(
                Signer::from_store(&encryption_only.encoded, 0).err(),
                Some(SigningError::NotForSigning)
            );
        }
        // Taken out of the store (a card taken out): what was listed of it finds nothing.
        let gone = InStore::put("signer.pfx", true).encoded.clone();
        assert_eq!(
            Signer::from_store(&gone, 0).err(),
            Some(SigningError::Unavailable)
        );
        assert_eq!(
            Signer::from_store(b"not a certificate", 0).err(),
            Some(SigningError::Unavailable)
        );
        // Every certificate put in has left the store, and every key is deleted. On the CI
        // runner, Windows kept the store's files of the certificates (and one of their keys'
        // identifiers) after they had left it: those files are not compared.
        for encoded in &put_in {
            assert_eq!(
                Signer::from_store(encoded, 0).err(),
                Some(SigningError::Unavailable),
                "taken out of the store"
            );
        }
        assert_eq!(
            without_certificates(key_files()),
            without_certificates(before),
            "every key put in was deleted"
        );
    }

    /// `files` (of `key_files`) less those of the user's certificate stores.
    fn without_certificates(files: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
        files
            .into_iter()
            .filter(|file| {
                !file
                    .components()
                    .any(|part| part.as_os_str() == "SystemCertificates")
            })
            .collect()
    }

    #[test]
    fn a_wrong_password_or_a_file_that_is_no_certificate_says_only_that() {
        if !may_import() {
            return;
        }
        let pfx = certificate("signer.pfx");
        for (bytes, password) in [
            (&pfx[..], "not the password"),
            (&pfx[..], ""),
            (&pfx[..pfx.len() / 2], PASSWORD),
            (&b"%PDF-1.7 not a certificate"[..], PASSWORD),
            (&[][..], PASSWORD),
        ] {
            assert_eq!(
                Signer::from_pfx(bytes, password).err(),
                Some(SigningError::Unreadable)
            );
        }
        let error = IpcError::from(SigningError::Unreadable);
        assert!(!error.message.contains(PASSWORD));
    }

    #[test]
    fn an_expired_certificate_and_one_not_for_signing_are_refused() {
        if !may_import() {
            return;
        }
        assert_eq!(
            Signer::from_pfx(&certificate("expired.pfx"), PASSWORD).err(),
            Some(SigningError::Expired)
        );
        assert_eq!(
            Signer::from_pfx(&certificate("encryption-only.pfx"), PASSWORD).err(),
            Some(SigningError::NotForSigning)
        );
    }
}
