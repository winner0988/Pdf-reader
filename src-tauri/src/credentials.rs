//! The passwords the user asked to remember, in Windows' Credential Manager (ADR 0017, B2-24).
//!
//! The app stores no password itself: an entry of the Credential Manager holds it, under a name
//! made of a prefix and the key of the file (`ipc_contract::worker::PasswordKey`, a hash of what
//! the file says of its encryption). **Nothing of the file is in an entry**: no name, no path, no
//! content. Windows shows the entries in its own Credential Manager, where the user can delete
//! them too.
//!
//! Only the main process uses this module. The worker has no access to the Credential Manager
//! (its sandbox forbids it) and must not see the passwords of other documents.

// The Credential Manager's Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::null_mut;
use std::slice;
use std::sync::{Mutex, MutexGuard};

use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredEnumerateW,
    CredFree, CredReadW, CredWriteW,
};
use zeroize::Zeroizing;

/// The longest password that is remembered, in UTF-8 bytes (the Credential Manager takes 2,560).
pub const MAX_PASSWORD_BYTES: usize = 1_024;
/// How many passwords the app remembers at most; one more is refused, never swapped for another.
pub const MAX_ENTRIES: usize = 256;

/// What the name of every entry of the app starts with; `v1` is the version of the key.
const TARGET_PREFIX: &str = "PdfReader/password/v1/";
/// Shown as the user name of an entry in Windows' Credential Manager.
const USER_NAME: &str = "PDF Reader";
/// Shown as the comment of an entry.
const COMMENT: &str =
    "A password PDF Reader remembers for an encrypted PDF. Delete it here, or in the app.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    /// The password is empty or longer than [`MAX_PASSWORD_BYTES`].
    Unsuitable,
    /// The set has [`MAX_ENTRIES`] already: forget one first.
    Full,
    /// The entry is not text: someone else changed it.
    Damaged,
    /// Windows could not do it (the service is off, a policy forbids it, ...): its error code.
    Unavailable(u32),
}

/// A set of passwords in the Credential Manager, told apart from other sets by the start of
/// their names.
pub struct Credentials {
    prefix: String,
    capacity: usize,
}

/// One call at a time into the Credential Manager from this process. Its store is kept in
/// files that Windows rewrites; calls made at the same moment by several threads have brought
/// back entries that had just been deleted (seen in this module's own tests).
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn hex(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn last_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

/// `CredEnumerateW` and `CredReadW` hand over buffers of Windows': the password in one is wiped
/// before the buffer is given back.
///
/// # Safety
/// `credential` must point to a valid `CREDENTIALW` of such a buffer that is not yet freed.
unsafe fn wipe(credential: *const CREDENTIALW) {
    // SAFETY: by the caller's promise the structure is valid; its blob has the size it says, in
    // memory this process owns until `CredFree`.
    unsafe {
        let credential = &*credential;
        if !credential.CredentialBlob.is_null() {
            std::ptr::write_bytes(
                credential.CredentialBlob,
                0,
                credential.CredentialBlobSize as usize,
            );
        }
    }
}

impl Credentials {
    /// The passwords of the app: `PdfReader/password/v1/<key>`, at most [`MAX_ENTRIES`].
    pub fn app() -> Self {
        Self::with(TARGET_PREFIX.to_owned(), MAX_ENTRIES)
    }

    /// Another set of passwords (the tests use their own, so that nothing of the user's is touched).
    pub fn with(prefix: String, capacity: usize) -> Self {
        Self { prefix, capacity }
    }

    /// The most passwords this set holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn target(&self, key: &[u8; 32]) -> Vec<u16> {
        wide(&format!("{}{}", self.prefix, hex(key)))
    }

    /// Remembers `password` for `key`, replacing one that is there.
    pub fn remember(&self, key: &[u8; 32], password: &str) -> Result<(), CredentialError> {
        if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
            return Err(CredentialError::Unsuitable);
        }
        let _serial = serial();
        let names = self.names()?;
        if !names.contains(&self.target(key)) && names.len() >= self.capacity {
            return Err(CredentialError::Full);
        }
        self.write(key, password.as_bytes())
    }

    fn write(&self, key: &[u8; 32], blob: &[u8]) -> Result<(), CredentialError> {
        // The structure wants mutable pointers although Windows only reads through them; the
        // copy of the password is wiped when this returns.
        let mut blob = Zeroizing::new(blob.to_vec());
        let mut target = self.target(key);
        let mut comment = wide(COMMENT);
        let mut user = wide(USER_NAME);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            Comment: comment.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer is to a buffer that outlives the call (the strings end in NUL,
        // the blob has the size given); `CredWriteW` only reads them and copies what it keeps.
        let written = unsafe { CredWriteW(&credential, 0) };
        if written == 0 {
            return Err(CredentialError::Unavailable(last_error()));
        }
        Ok(())
    }

    /// The password remembered for `key`, if there is one.
    pub fn recall(&self, key: &[u8; 32]) -> Result<Option<Zeroizing<String>>, CredentialError> {
        let _serial = serial();
        let target = self.target(key);
        let mut found: *mut CREDENTIALW = null_mut();
        // SAFETY: `target` ends in NUL; on success `found` receives a buffer that is freed below.
        let read = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) };
        if read == 0 {
            return match last_error() {
                ERROR_NOT_FOUND => Ok(None),
                code => Err(CredentialError::Unavailable(code)),
            };
        }
        // SAFETY: the call succeeded, so `found` is a valid structure whose blob has the size it
        // says (or none); the copy is made before `wipe` and `CredFree` give the buffer back.
        let blob = unsafe {
            let credential = &*found;
            let blob = if credential.CredentialBlob.is_null() {
                Vec::new()
            } else {
                slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
                .to_vec()
            };
            wipe(found);
            CredFree(found as *const c_void);
            Zeroizing::new(blob)
        };
        match std::str::from_utf8(&blob) {
            Ok(text) if !text.is_empty() => Ok(Some(Zeroizing::new(text.to_owned()))),
            _ => Err(CredentialError::Damaged),
        }
    }

    /// Forgets the password for `key`; whether there was one.
    pub fn forget(&self, key: &[u8; 32]) -> Result<bool, CredentialError> {
        let _serial = serial();
        self.delete(&self.target(key))
    }

    fn delete(&self, target: &[u16]) -> Result<bool, CredentialError> {
        // SAFETY: `target` ends in NUL (made by `wide`, or copied from an entry's name).
        let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if deleted != 0 {
            return Ok(true);
        }
        match last_error() {
            ERROR_NOT_FOUND => Ok(false),
            code => Err(CredentialError::Unavailable(code)),
        }
    }

    /// How many passwords are remembered.
    pub fn count(&self) -> Result<usize, CredentialError> {
        let _serial = serial();
        Ok(self.names()?.len())
    }

    /// Forgets every password of this set (and of no other); how many there were.
    pub fn forget_all(&self) -> Result<usize, CredentialError> {
        let _serial = serial();
        let mut forgotten = 0;
        for name in self.names()? {
            if self.delete(&name)? {
                forgotten += 1;
            }
        }
        Ok(forgotten)
    }

    /// The names of the entries of this set, each ending in NUL.
    fn names(&self) -> Result<Vec<Vec<u16>>, CredentialError> {
        let filter = wide(&format!("{}*", self.prefix));
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = null_mut();
        // SAFETY: `filter` ends in NUL; on success `list` receives `count` pointers in a buffer
        // that is freed below.
        let listed = unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) };
        if listed == 0 {
            return match last_error() {
                ERROR_NOT_FOUND => Ok(Vec::new()),
                code => Err(CredentialError::Unavailable(code)),
            };
        }
        let mut names = Vec::with_capacity(count as usize);
        // SAFETY: the call succeeded, so `list` holds `count` valid structures, each with a name
        // that ends in NUL; the names are copied, and the passwords wiped, before the buffer is
        // freed.
        unsafe {
            for &credential in slice::from_raw_parts(list, count as usize) {
                let name = (*credential).TargetName;
                if !name.is_null() && (*credential).Type == CRED_TYPE_GENERIC {
                    let mut length = 0;
                    while *name.add(length) != 0 {
                        length += 1;
                    }
                    names.push(slice::from_raw_parts(name, length + 1).to_vec());
                }
                wipe(credential);
            }
            CredFree(list as *const c_void);
        }
        Ok(names)
    }
}

// These tests use the real Credential Manager of the user running them, in sets of their own
// (`PdfReaderTest/<process>-<name>/`) that are emptied when each test ends, even by a failure.
#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(Credentials);

    impl Scratch {
        fn new(name: &str, capacity: usize) -> Self {
            let prefix = format!("PdfReaderTest/{}-{name}/", std::process::id());
            let set = Credentials::with(prefix, capacity);
            set.forget_all().expect("an empty set to start from");
            Self(set)
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Credentials;
        fn deref(&self) -> &Credentials {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = self.0.forget_all();
        }
    }

    const KEY: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [8; 32];

    #[test]
    fn a_password_is_read_back_and_forgotten() {
        let set = Scratch::new("round-trip", MAX_ENTRIES);
        assert_eq!(set.recall(&KEY), Ok(None));
        set.remember(&KEY, "pässwörd 密碼 🔑").unwrap();
        assert_eq!(
            set.recall(&KEY).unwrap().as_deref().map(String::as_str),
            Some("pässwörd 密碼 🔑")
        );
        assert_eq!(
            set.recall(&OTHER),
            Ok(None),
            "another file's password is not found"
        );
        assert_eq!(set.count(), Ok(1));
        assert_eq!(set.forget(&KEY), Ok(true));
        assert_eq!(set.recall(&KEY), Ok(None));
        assert_eq!(set.forget(&KEY), Ok(false), "nothing left to forget");
        assert_eq!(set.count(), Ok(0));
    }

    #[test]
    fn remembering_again_replaces_the_password_and_is_not_counted_twice() {
        let set = Scratch::new("replace", MAX_ENTRIES);
        set.remember(&KEY, "first").unwrap();
        set.remember(&KEY, "second").unwrap();
        assert_eq!(
            set.recall(&KEY).unwrap().as_deref().map(String::as_str),
            Some("second")
        );
        assert_eq!(set.count(), Ok(1));
    }

    #[test]
    fn a_password_that_is_empty_or_too_long_is_refused() {
        let set = Scratch::new("sizes", MAX_ENTRIES);
        assert_eq!(set.remember(&KEY, ""), Err(CredentialError::Unsuitable));
        let longest = "k".repeat(MAX_PASSWORD_BYTES);
        set.remember(&KEY, &longest).unwrap();
        assert_eq!(
            set.recall(&KEY).unwrap().as_deref().map(String::as_str),
            Some(longest.as_str())
        );
        let too_long = "k".repeat(MAX_PASSWORD_BYTES + 1);
        assert_eq!(
            set.remember(&OTHER, &too_long),
            Err(CredentialError::Unsuitable)
        );
        assert_eq!(set.count(), Ok(1));
    }

    #[test]
    fn a_full_set_refuses_a_password_for_a_new_file_and_keeps_the_others() {
        let set = Scratch::new("full", 2);
        set.remember(&[1; 32], "one").unwrap();
        set.remember(&[2; 32], "two").unwrap();
        assert_eq!(set.remember(&[3; 32], "three"), Err(CredentialError::Full));
        assert_eq!(set.recall(&[3; 32]), Ok(None));
        assert_eq!(
            set.recall(&[1; 32]).unwrap().as_deref().map(String::as_str),
            Some("one")
        );
        // A file that has a password can still have it changed, and a place frees up by forgetting.
        set.remember(&[2; 32], "two again").unwrap();
        assert_eq!(set.forget(&[1; 32]), Ok(true));
        set.remember(&[3; 32], "three").unwrap();
        assert_eq!(set.count(), Ok(2));
    }

    #[test]
    fn forgetting_all_leaves_the_other_sets_alone() {
        let mine = Scratch::new("mine", MAX_ENTRIES);
        let theirs = Scratch::new("theirs", MAX_ENTRIES);
        mine.remember(&[1; 32], "a").unwrap();
        mine.remember(&[2; 32], "b").unwrap();
        theirs.remember(&[1; 32], "c").unwrap();
        assert_eq!(mine.forget_all(), Ok(2));
        assert_eq!(mine.count(), Ok(0));
        assert_eq!(
            theirs
                .recall(&[1; 32])
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("c")
        );
        assert_eq!(mine.forget_all(), Ok(0));
    }

    #[test]
    fn an_entry_that_is_not_text_is_damaged_and_still_forgotten() {
        let set = Scratch::new("damaged", MAX_ENTRIES);
        {
            let _serial = serial();
            set.write(&KEY, &[0xff, 0xfe, 0xfd]).unwrap();
        }
        assert_eq!(set.recall(&KEY), Err(CredentialError::Damaged));
        assert_eq!(set.forget(&KEY), Ok(true));
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    /// What Windows has under the name of an entry, read with its own call.
    #[test]
    fn an_entry_has_the_key_in_its_name_and_nothing_of_the_file() {
        let prefix = format!("PdfReaderTest/{}-entry/", std::process::id());
        let set = Credentials::with(prefix.clone(), MAX_ENTRIES);
        let key = [0x3c; 32];
        set.forget_all().unwrap();
        set.remember(&key, "s3cret").unwrap();
        let _serial = serial();
        let target = wide(&format!("{prefix}{}", hex(&key)));
        let mut found: *mut CREDENTIALW = null_mut();
        // SAFETY: `target` ends in NUL; `found` is freed below, after the fields are copied.
        let read = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) };
        assert_ne!(read, 0, "the entry is there under the key's name");
        // SAFETY: the call succeeded, so the structure and the strings in it are valid.
        let (user, comment, persist, blob) = unsafe {
            let entry = &*found;
            let text = |pointer: *const u16| {
                let mut length = 0;
                while *pointer.add(length) != 0 {
                    length += 1;
                }
                String::from_utf16_lossy(slice::from_raw_parts(pointer, length))
            };
            let blob =
                slice::from_raw_parts(entry.CredentialBlob, entry.CredentialBlobSize as usize)
                    .to_vec();
            let fields = (
                text(entry.UserName),
                text(entry.Comment),
                entry.Persist,
                blob,
            );
            wipe(found);
            CredFree(found as *const c_void);
            fields
        };
        drop(_serial);
        set.forget_all().unwrap();
        assert_eq!(user, USER_NAME);
        assert_eq!(comment, COMMENT);
        assert_eq!(persist, CRED_PERSIST_LOCAL_MACHINE);
        assert_eq!(blob, b"s3cret");
        assert_eq!(hex(&key).len(), 64);
        assert_eq!(TARGET_PREFIX, "PdfReader/password/v1/");
    }
}
