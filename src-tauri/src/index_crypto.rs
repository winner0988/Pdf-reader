//! The text index's encryption (B3-21, ADR 0026, docs/architecture/text-index.md): a 256-bit key
//! from the operating system's random number generator, kept on disk protected by DPAPI for the
//! current Windows user, and AES-256-GCM through Windows CNG with a fresh 96-bit nonce for every
//! piece encrypted. Nothing here is new to the app's dependencies: it is Windows' own.

// CNG's and DPAPI's Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{LocalFree, STATUS_AUTH_TAG_MISMATCH};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_AES_ALGORITHM, BCRYPT_ALG_HANDLE, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO,
    BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_CHAIN_MODE_GCM, BCRYPT_CHAINING_MODE,
    BCRYPT_KEY_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptCloseAlgorithmProvider,
    BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenRandom, BCryptGenerateSymmetricKey,
    BCryptOpenAlgorithmProvider, BCryptSetProperty, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    CryptProtectData, CryptUnprotectData,
};
use zeroize::{Zeroize, Zeroizing};

/// Bytes of the key.
pub const KEY_BYTES: usize = 32;
/// Bytes of the nonce in front of every sealed piece.
const NONCE_BYTES: usize = 12;
/// Bytes of the tag behind every sealed piece.
const TAG_BYTES: usize = 16;
/// What a sealed piece adds to its plain text.
pub const OVERHEAD: usize = NONCE_BYTES + TAG_BYTES;

/// Mixed into DPAPI's protection of the key: another program of the same user has to know it to
/// unprotect the key with DPAPI directly (it can read it here, so this only keeps the key from
/// being taken for some other program's).
const ENTROPY: &[u8] = b"Pdf-reader text index key (B3-21)";

/// Why something could not be encrypted or decrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// Windows could not do it (no random numbers, no AES, DPAPI failed).
    Failed,
    /// The piece was not sealed with this key and these associated data, or was changed since.
    Tampered,
}

/// `length` random bytes from the operating system.
pub fn random(length: usize) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let mut bytes = Zeroizing::new(vec![0u8; length]);
    let size = u32::try_from(length).map_err(|_| CryptoError::Failed)?;
    // SAFETY: `bytes` has `size` writable bytes; no algorithm handle with the system's preferred
    // generator.
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            bytes.as_mut_ptr(),
            size,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(CryptoError::Failed);
    }
    Ok(bytes)
}

/// A new key, from the operating system's random number generator.
pub fn new_key() -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    random(KEY_BYTES)
}

/// `secret` protected by DPAPI for the current Windows user, to keep on disk. Nothing is shown.
pub fn protect(secret: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let input = blob(secret)?;
    let entropy = blob(ENTROPY)?;
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // SAFETY: the blobs point at live bytes that Windows only reads; it allocates the output,
    // which `take_local` copies and frees.
    let done = unsafe {
        CryptProtectData(
            &input,
            null(),
            &entropy,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if done == 0 {
        return Err(CryptoError::Failed);
    }
    // SAFETY: what CryptProtectData allocated, freed once there.
    Ok(unsafe { take_local(output) }.to_vec())
}

/// The secret `protected` holds, if DPAPI can unprotect it for the current Windows user.
pub fn unprotect(protected: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let input = blob(protected)?;
    let entropy = blob(ENTROPY)?;
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // SAFETY: as in `protect`; no description is asked for, and nothing is shown.
    let done = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            &entropy,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if done == 0 {
        return Err(CryptoError::Failed);
    }
    // SAFETY: what CryptUnprotectData allocated, wiped and freed once there.
    Ok(unsafe { take_local(output) })
}

fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, CryptoError> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| CryptoError::Failed)?,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// A copy of what DPAPI allocated in `output`, which is wiped and freed.
///
/// # Safety
/// `output` came from a successful `CryptProtectData` or `CryptUnprotectData`, and is not used
/// again.
unsafe fn take_local(output: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
    if output.pbData.is_null() {
        return Zeroizing::new(Vec::new());
    }
    // SAFETY: DPAPI gave `cbData` bytes at `pbData`, which stay until they are freed below.
    let bytes = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let copy = Zeroizing::new(bytes.to_vec());
    bytes.zeroize();
    // SAFETY: allocated with LocalAlloc by DPAPI, freed once.
    unsafe { LocalFree(output.pbData.cast()) };
    copy
}

/// AES-256-GCM with one key. Every piece sealed gets a nonce of its own, from the operating
/// system's random number generator, which it carries in front of it.
pub struct Cipher {
    algorithm: BCRYPT_ALG_HANDLE,
    key: BCRYPT_KEY_HANDLE,
}

// SAFETY: CNG's handles are not tied to the thread that made them; a cipher is used by one thread
// at a time (it is not `Sync`), and its handles are freed once, in `drop`.
unsafe impl Send for Cipher {}

impl Cipher {
    /// AES-256-GCM with `key` (`KEY_BYTES` bytes).
    pub fn new(key: &[u8]) -> Result<Self, CryptoError> {
        if key.len() != KEY_BYTES {
            return Err(CryptoError::Failed);
        }
        let mut cipher = Self {
            algorithm: null_mut(),
            key: null_mut(),
        };
        // SAFETY: a place for the handle, a constant algorithm name, Microsoft's provider.
        let status = unsafe {
            BCryptOpenAlgorithmProvider(&mut cipher.algorithm, BCRYPT_AES_ALGORITHM, null(), 0)
        };
        if status != 0 {
            return Err(CryptoError::Failed);
        }
        // The mode's name with its NUL, in bytes, as CNG takes a string property.
        // SAFETY: `BCRYPT_CHAIN_MODE_GCM` is a NUL-terminated constant string.
        let mode_units = unsafe { wide_len(BCRYPT_CHAIN_MODE_GCM) } + 1;
        // SAFETY: a live algorithm handle, a constant property name and value of that length.
        let status = unsafe {
            BCryptSetProperty(
                cipher.algorithm,
                BCRYPT_CHAINING_MODE,
                BCRYPT_CHAIN_MODE_GCM.cast(),
                (mode_units * 2) as u32,
                0,
            )
        };
        if status != 0 {
            return Err(CryptoError::Failed);
        }
        // SAFETY: a live algorithm handle, a place for the key handle, CNG keeps the key object
        // itself, and `key` has `KEY_BYTES` bytes it only reads.
        let status = unsafe {
            BCryptGenerateSymmetricKey(
                cipher.algorithm,
                &mut cipher.key,
                null_mut(),
                0,
                key.as_ptr(),
                KEY_BYTES as u32,
                0,
            )
        };
        if status != 0 {
            return Err(CryptoError::Failed);
        }
        Ok(cipher)
    }

    /// `plain`, encrypted and authenticated together with `associated` (which is not encrypted, and
    /// must be given again to open it): the nonce, then the cipher text, then the tag.
    pub fn seal(&self, associated: &[u8], plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let length = u32::try_from(plain.len()).map_err(|_| CryptoError::Failed)?;
        let nonce = random(NONCE_BYTES)?;
        let mut sealed = vec![0u8; plain.len() + OVERHEAD];
        sealed[..NONCE_BYTES].copy_from_slice(&nonce);
        let (head, rest) = sealed.split_at_mut(NONCE_BYTES);
        let (body, tag) = rest.split_at_mut(plain.len());
        let mut info = mode_info(head, associated, tag)?;
        let mut written = 0u32;
        // SAFETY: a live key; `plain` has `length` bytes to read and `body` as many to write; the
        // mode information points at the nonce, the associated data and the tag, which all live
        // through the call. No IV: GCM takes the nonce from the mode information.
        let status = unsafe {
            BCryptEncrypt(
                self.key,
                plain.as_ptr(),
                length,
                std::ptr::from_mut(&mut info).cast::<c_void>(),
                null_mut(),
                0,
                body.as_mut_ptr(),
                length,
                &mut written,
                0,
            )
        };
        if status != 0 || written != length {
            return Err(CryptoError::Failed);
        }
        Ok(sealed)
    }

    /// The plain text of `sealed`, if it was sealed with this key and `associated` and not changed
    /// since (`Tampered` otherwise).
    pub fn open(
        &self,
        associated: &[u8],
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if sealed.len() < OVERHEAD {
            return Err(CryptoError::Tampered);
        }
        let mut nonce = sealed[..NONCE_BYTES].to_vec();
        let body = &sealed[NONCE_BYTES..sealed.len() - TAG_BYTES];
        let mut tag = sealed[sealed.len() - TAG_BYTES..].to_vec();
        let length = u32::try_from(body.len()).map_err(|_| CryptoError::Failed)?;
        let mut plain = Zeroizing::new(vec![0u8; body.len()]);
        let mut info = mode_info(&mut nonce, associated, &mut tag)?;
        let mut written = 0u32;
        // SAFETY: as in `seal`, the other way round; CNG checks the tag before it gives anything.
        let status = unsafe {
            BCryptDecrypt(
                self.key,
                body.as_ptr(),
                length,
                std::ptr::from_mut(&mut info).cast::<c_void>(),
                null_mut(),
                0,
                plain.as_mut_ptr(),
                length,
                &mut written,
                0,
            )
        };
        if status == STATUS_AUTH_TAG_MISMATCH {
            return Err(CryptoError::Tampered);
        }
        if status != 0 || written != length {
            return Err(CryptoError::Failed);
        }
        Ok(plain)
    }
}

impl Drop for Cipher {
    fn drop(&mut self) {
        // SAFETY: handles CNG gave, each freed once; the key before its algorithm.
        unsafe {
            if !self.key.is_null() {
                BCryptDestroyKey(self.key);
            }
            if !self.algorithm.is_null() {
                BCryptCloseAlgorithmProvider(self.algorithm, 0);
            }
        }
    }
}

/// GCM's part of an encryption or decryption: where the nonce, the associated data and the tag
/// are. They must outlive the call that uses it.
fn mode_info(
    nonce: &mut [u8],
    associated: &[u8],
    tag: &mut [u8],
) -> Result<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO, CryptoError> {
    Ok(BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
        dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
        pbNonce: nonce.as_mut_ptr(),
        cbNonce: nonce.len() as u32,
        // CNG only reads the associated data.
        pbAuthData: associated.as_ptr().cast_mut(),
        cbAuthData: u32::try_from(associated.len()).map_err(|_| CryptoError::Failed)?,
        pbTag: tag.as_mut_ptr(),
        cbTag: tag.len() as u32,
        ..Default::default()
    })
}

/// The length of a NUL-terminated UTF-16 string, without its NUL.
///
/// # Safety
/// `text` points at a NUL-terminated UTF-16 string.
unsafe fn wide_len(text: *const u16) -> usize {
    let mut length = 0;
    // SAFETY: the string ends with a NUL, which stops the walk.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_piece_opens_with_its_key_and_associated_data_only() {
        let key = new_key().unwrap();
        let cipher = Cipher::new(&key).unwrap();
        let plain = "隱私權政策 privacy policy".as_bytes();
        let sealed = cipher.seal(b"document 7", plain).unwrap();
        assert_eq!(sealed.len(), plain.len() + OVERHEAD);
        assert!(!sealed.windows(plain.len()).any(|window| window == plain));
        assert_eq!(&cipher.open(b"document 7", &sealed).unwrap()[..], plain);

        // Another piece's place, one byte changed anywhere, another key: none opens.
        assert_eq!(
            cipher.open(b"document 8", &sealed),
            Err(CryptoError::Tampered)
        );
        for at in [0, NONCE_BYTES, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert_eq!(
                cipher.open(b"document 7", &changed),
                Err(CryptoError::Tampered)
            );
        }
        let other = Cipher::new(&new_key().unwrap()).unwrap();
        assert_eq!(
            other.open(b"document 7", &sealed),
            Err(CryptoError::Tampered)
        );
        assert_eq!(
            cipher.open(b"document 7", &sealed[..OVERHEAD - 1]),
            Err(CryptoError::Tampered)
        );
    }

    #[test]
    fn every_piece_gets_a_nonce_of_its_own() {
        let cipher = Cipher::new(&new_key().unwrap()).unwrap();
        let first = cipher.seal(b"", b"the same words").unwrap();
        let second = cipher.seal(b"", b"the same words").unwrap();
        assert_ne!(first[..NONCE_BYTES], second[..NONCE_BYTES]);
        assert_ne!(first, second);
        assert_eq!(
            &cipher.open(b"", &cipher.seal(b"", b"").unwrap()).unwrap()[..],
            b""
        );
    }

    #[test]
    fn the_key_is_kept_protected_for_this_user() {
        let key = new_key().unwrap();
        let protected = protect(&key).unwrap();
        assert!(
            !protected
                .windows(KEY_BYTES)
                .any(|window| window == &key[..])
        );
        assert_eq!(&unprotect(&protected).unwrap()[..], &key[..]);
        let mut changed = protected.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert_eq!(unprotect(&changed), Err(CryptoError::Failed));
        assert!(Cipher::new(&key[..16]).is_err());
    }
}
