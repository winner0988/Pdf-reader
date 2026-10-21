//! Attachments in the main process (B3-20, ADR 0025): which types are dangerous, and the
//! "from the internet" mark (Mark of the Web) on a saved attachment.

// AssocIsDangerous; the unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use ipc_contract::attachments::attachment_extension;

/// Dangerous types that `AssocIsDangerous` may not list (ADR 0025), without the dot: Office
/// formats with macros, disk images, shortcuts, scripts, installers and packages.
const DANGEROUS_EXTENSIONS: &[&str] = &[
    // Office formats with macros
    "docm",
    "dotm",
    "xlsm",
    "xltm",
    "xlam",
    "xll",
    "pptm",
    "potm",
    "ppam",
    "ppsm",
    "sldm",
    // Disk images
    "iso",
    "img",
    "vhd",
    "vhdx",
    // Shortcuts
    "lnk",
    "url",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
    // Scripts
    "hta",
    "js",
    "jse",
    "vb",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
    "ws",
    "wsc",
    "ps1",
    "ps1xml",
    "ps2",
    "ps2xml",
    "psc1",
    "psc2",
    "bat",
    "cmd",
    "reg",
    "chm",
    // Programs, installers and packages
    "exe",
    "com",
    "scr",
    "pif",
    "cpl",
    "msi",
    "msp",
    "jar",
    "gadget",
    "application",
    "appref-ms",
    "appx",
    "appxbundle",
    "msix",
    "msixbundle",
    "appinstaller",
    "diagcab",
];

/// Whether an attachment named `name` is of a dangerous type (ADR 0025): one without an
/// extension, one of [`DANGEROUS_EXTENSIONS`], or one Windows says is (`AssocIsDangerous`, the
/// system's own list with the administrator's policy).
pub fn is_dangerous_attachment(name: &str) -> bool {
    let extension = attachment_extension(name);
    let Some(bare) = extension.strip_prefix('.') else {
        return true;
    };
    DANGEROUS_EXTENSIONS.contains(&bare) || windows_says_dangerous(&extension)
}

/// `AssocIsDangerous` for `extension` (with its dot).
fn windows_says_dangerous(extension: &str) -> bool {
    use windows::Win32::UI::Shell::AssocIsDangerous;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = extension.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call, which only reads it.
    unsafe { AssocIsDangerous(PCWSTR(wide.as_ptr())) }.as_bool()
}

/// The "from the internet" mark: the `Zone.Identifier` stream with zone 3 (the internet zone).
const ZONE_IDENTIFIER: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

/// Marks the file at `path` as from the internet (ADR 0025), so that Windows checks it before
/// opening it. Fails where the file system has no streams, such as FAT32.
pub fn mark_from_internet(path: &Path) -> io::Result<()> {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&stream)?;
    file.write_all(ZONE_IDENTIFIER)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_without_an_extension_is_dangerous() {
        for name in ["README", "data", ".exe", "附件 1"] {
            assert!(is_dangerous_attachment(name), "{name}");
        }
    }

    #[test]
    fn the_types_adr_0025_lists_are_dangerous() {
        for name in [
            "report.docm",
            "sheet.xlsm",
            "slides.pptm",
            "disk.iso",
            "backup.img",
            "vm.vhd",
            "vm.vhdx",
            "link.lnk",
            "site.url",
            "app.hta",
            "script.js",
            "script.jse",
            "script.vbs",
            "script.wsf",
            "tool.ps1",
            "setup.msi",
            "app.jar",
            "payload.EXE",
            "command.bat",
            "run.cmd",
            "screen.scr",
        ] {
            assert!(is_dangerous_attachment(name), "{name}");
        }
    }

    #[test]
    fn documents_and_pictures_are_not() {
        for name in [
            "document.pdf",
            "plain.txt",
            "picture.png",
            "photo.jpg",
            "data.csv",
        ] {
            assert!(!is_dangerous_attachment(name), "{name}");
        }
    }

    #[test]
    fn windows_is_asked_about_the_rest() {
        // Not in the app's list; Windows' own list has it.
        assert!(!DANGEROUS_EXTENSIONS.contains(&"hlp"));
        assert!(windows_says_dangerous(".hlp"));
        assert!(!windows_says_dangerous(".txt"));
    }

    #[test]
    fn the_mark_is_the_internet_zone_stream() {
        let path = std::env::temp_dir().join(format!("attachment-mark-{}.txt", std::process::id()));
        std::fs::write(&path, b"attachment").unwrap();

        mark_from_internet(&path).expect("the temporary folder is on NTFS");
        let mut stream = path.as_os_str().to_os_string();
        stream.push(":Zone.Identifier");
        assert_eq!(std::fs::read(&stream).unwrap(), ZONE_IDENTIFIER);
        // The file itself is unchanged.
        assert_eq!(std::fs::read(&path).unwrap(), b"attachment");
        std::fs::remove_file(&path).unwrap();
    }
}
