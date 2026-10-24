//! What kind of drive a file is on: the text index (B3-21, ADR 0026) only keeps the text of files
//! on this computer's fixed drives, not of those on removable drives, discs or network places.

// Two Win32 calls; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};

use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};

/// `DRIVE_FIXED`: a drive that cannot be taken out (Windows' own word for it: some external disks
/// say so too).
const DRIVE_FIXED: u32 = 3;

/// Room for the volume of a path: as long as the longest path Windows takes.
const MAX_VOLUME_UNITS: usize = 32_768;

/// Whether `path` is on a fixed drive of this computer. False for a removable drive, a disc, a
/// network place, and anything Windows cannot say.
pub fn is_fixed(path: &Path) -> bool {
    // A network place, said by its name alone: Windows is not asked about it.
    if let Some(Component::Prefix(prefix)) = path.components().next()
        && matches!(
            prefix.kind(),
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..)
        )
    {
        return false;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return false;
    }
    let mut volume = vec![0u16; MAX_VOLUME_UNITS];
    // SAFETY: `wide` is NUL-terminated, and `volume` has room for `MAX_VOLUME_UNITS` units.
    let found =
        unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), MAX_VOLUME_UNITS as u32) };
    if found == 0 {
        return false;
    }
    // SAFETY: Windows wrote a NUL-terminated root path into `volume`.
    unsafe { GetDriveTypeW(volume.as_ptr()) == DRIVE_FIXED }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temporary_folder_is_on_a_fixed_drive_and_a_share_is_not() {
        assert!(is_fixed(&std::env::temp_dir()));
        // Not looked up: nothing goes to the network.
        assert!(!is_fixed(Path::new(r"\\server.invalid\share\file.pdf")));
        assert!(!is_fixed(Path::new(
            r"\\?\UNC\server.invalid\share\file.pdf"
        )));
        assert!(!is_fixed(Path::new("relative\0name.pdf")));
    }
}
