//! Turns a handle value received in an `Open` request into a `File` (ADR 0008).
//!
//! The main process opens the file read-only and duplicates the handle into this process; the
//! worker never sees a path. A bogus value cannot be used to reach other objects silently: the
//! sandbox enables strict handle checks, so using an invalid handle terminates the worker.

#![allow(unsafe_code)]

use std::fs::File;

use ipc_contract::worker::FileHandle;

/// Takes ownership of the handle. Returns `None` for values that cannot be handles.
#[cfg(windows)]
pub fn take_file(handle: FileHandle) -> Option<File> {
    use std::os::windows::io::{FromRawHandle, RawHandle};

    let value = usize::try_from(handle.0).ok()?;
    // Handle values are non-zero multiples of 4; -1 is the pseudo-handle of the current process.
    if value == 0 || value % 4 != 0 || value == usize::MAX {
        return None;
    }
    // SAFETY: the main process duplicated this handle into this process solely for this
    // request, so nothing else owns it; `File` closes it when dropped.
    Some(unsafe { File::from_raw_handle(value as RawHandle) })
}

#[cfg(not(windows))]
pub fn take_file(_handle: FileHandle) -> Option<File> {
    None
}
