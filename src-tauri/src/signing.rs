//! Signing a document (B3-06, ADR 0020, docs/architecture/signing.md): the main process's part,
//! the only code of the app that holds a private key.
//!
//! A certificate file the user chose (PKCS #12: `.pfx`, `.p12`) is imported with
//! `PFXImportCertStore` into a store that lives in memory only: its key is not persisted
//! (`PKCS12_NO_PERSIST_KEY`) and cannot be exported (no `CRYPT_EXPORTABLE`), and both go when the
//! [`Signer`] is dropped. The password goes to Windows once and is wiped. A signature is a
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

use ipc_contract::limits::MAX_CERTIFICATE_FILE_BYTES;
use ipc_contract::types::{ErrorCode, IpcError};
use ipc_contract::worker::SigningTime;
use windows_sys::Win32::Foundation::{FILETIME, GetLastError};
use windows_sys::Win32::Security::Cryptography::{
    CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT,
    CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE, CERT_CHAIN_PARA, CERT_CONTEXT,
    CERT_DIGITAL_SIGNATURE_KEY_USAGE, CERT_FIND_HAS_PRIVATE_KEY, CERT_INFO, CERT_NAME_ATTR_TYPE,
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_NCRYPT_KEY_SPEC, CERT_NON_REPUDIATION_KEY_USAGE,
    CMSG_CONTENT_PARAM, CMSG_DETACHED_FLAG, CMSG_SIGNED, CMSG_SIGNED_ENCODE_INFO,
    CMSG_SIGNER_ENCODE_INFO, CMSG_SIGNER_ENCODE_INFO_0, CRYPT_ACQUIRE_ALLOW_NCRYPT_KEY_FLAG,
    CRYPT_ACQUIRE_SILENT_FLAG, CRYPT_ALGORITHM_IDENTIFIER, CRYPT_ATTRIBUTE, CRYPT_INTEGER_BLOB,
    CertCloseStore, CertFindCertificateInStore, CertFreeCertificateChain,
    CertFreeCertificateContext, CertGetCertificateChain, CertGetIntendedKeyUsage,
    CertGetNameStringW, CertVerifyTimeValidity, CryptAcquireCertificatePrivateKey,
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
    /// Windows did not make the signature.
    Failed,
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

/// The store a certificate file was imported into; closed when dropped, which ends its key.
struct Store(HCERTSTORE);

impl Drop for Store {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the store came from `PFXImportCertStore` and is closed once. Contexts still
            // open keep it until they are freed.
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
        // SAFETY: a live certificate context, whose `pCertInfo` lives as long as it does.
        let info = unsafe { (*certificate.0).pCertInfo };
        // SAFETY: no time means now; `info` is live.
        match unsafe { CertVerifyTimeValidity(null(), info) } {
            0 => {}
            before if before < 0 => return Err(SigningError::NotYetValid),
            _ => return Err(SigningError::Expired),
        }
        if !for_signing(info) {
            return Err(SigningError::NotForSigning);
        }
        let mut handle = 0;
        let mut spec = 0;
        let mut release = 0;
        // SAFETY: a live certificate context, and three places for what the call gives. Silent:
        // a key from a file needs no PIN, and nothing is shown.
        let acquired = unsafe {
            CryptAcquireCertificatePrivateKey(
                certificate.0,
                CRYPT_ACQUIRE_ALLOW_NCRYPT_KEY_FLAG | CRYPT_ACQUIRE_SILENT_FLAG,
                null(),
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
            return Err(SigningError::NotForSigning);
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
        let common = name_of(
            &self.certificate,
            CERT_NAME_ATTR_TYPE,
            szOID_COMMON_NAME.cast(),
        );
        if common.trim().is_empty() {
            name_of(&self.certificate, CERT_NAME_SIMPLE_DISPLAY_TYPE, null())
        } else {
            common
        }
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
            return Err(SigningError::Failed);
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
            if updated == 0 {
                return Err(SigningError::Failed);
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

fn name_of(certificate: &Certificate, kind: u32, parameter: *const c_void) -> String {
    let mut name = [0u16; NAME_UNITS];
    // SAFETY: a live certificate context, a parameter the kind of name takes (an attribute's
    // object identifier, or none), and a buffer of the length given.
    let length = unsafe {
        CertGetNameStringW(
            certificate.0,
            kind,
            0,
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
        CRYPT_VERIFY_MESSAGE_PARA, CryptVerifyDetachedMessageSignature,
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
