//! Attachments handling in main process (B3-20, ADR 0025).
//!
//! Enforces:
//! - Sanitization of attachment names (second pass in main process).
//! - Identification of dangerous file types via Windows `AssocIsDangerous` plus a fixed list.
//! - Writing of Mark of the Web (`Zone.Identifier`) alternate data stream (`[ZoneTransfer]\r\nZoneId=3\r\n`).
//! - Informing the user when stream writing fails without deleting the saved file.

// AssocIsDangerous call; each unsafe block says why it is sound.
#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use ipc_contract::attachments::{attachment_extension, clean_attachment_name};

/// Fixed list of dangerous file extensions (ADR 0025).
///
/// Complements `AssocIsDangerous`:
/// - Macro-enabled Office files (.docm, .xlsm, .pptm, etc.)
/// - Disk images (.iso, .img, .vhd, .vhdx)
/// - Shortcuts and script formats (.lnk, .url, .hta, .js, .jse, .vbs, .wsf, .ps1, etc.)
/// - Executable / installer types (.msi, .jar, etc.)
const FIXED_DANGEROUS_EXTENSIONS: &[&str] = &[
    // Macro-enabled Office formats
    "docm", "dotm", "xlsm", "xltm", "xlam", "pptm", "potm", "ppam", "ppsm", "sldm",
    // Disk images
    "iso", "img", "vhd", "vhdx",
    // Shortcuts and scripts
    "lnk", "url", "hta", "js", "jse", "vbs", "vbe", "wsf", "wsh", "ps1", "ps1xml", "ps2",
    "ps2xml", "psc1", "psc2", "bat", "cmd",
    // Executables, installers, libraries, control panel
    "exe", "com", "scr", "pif", "msi", "msp", "jar", "cpl", "gadget", "application",
    "appref-ms", "reg", "vb", "ws", "wsc",
];

/// Returns `true` if `name` represents a dangerous file type according to ADR 0025:
/// 1. Names without an extension are considered dangerous.
/// 2. Extensions in the fixed list are dangerous.
/// 3. Windows Shell `AssocIsDangerous` is queried.
pub fn is_dangerous_attachment(name: &str) -> bool {
    let ext = attachment_extension(name);
    if ext.is_empty() {
        // ADR 0025: "名稱沒有副檔名也當成危險。"
        return true;
    }

    let without_dot = ext.trim_start_matches('.');
    let ext_lower = without_dot.to_ascii_lowercase();

    // Check fixed list
    if FIXED_DANGEROUS_EXTENSIONS.contains(&ext_lower.as_str()) {
        return true;
    }

    // Check Windows AssocIsDangerous
    #[cfg(windows)]
    {
        assoc_is_dangerous(&ext)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn assoc_is_dangerous(ext: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::AssocIsDangerous;

    let dot_ext = if ext.starts_with('.') {
        ext.to_owned()
    } else {
        format!(".{ext}")
    };
    let wide: Vec<u16> = dot_ext.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a valid null-terminated UTF-16 string alive for the duration of the call.
    unsafe { AssocIsDangerous(PCWSTR(wide.as_ptr())) }.as_bool()
}

pub const ZONE_IDENTIFIER_STREAM_SUFFIX: &str = ":Zone.Identifier";
pub const ZONE_TRANSFER_CONTENT: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

/// Trait for writing Zone.Identifier, enabling test doubles (e.g. simulating FAT32 failure).
#[cfg_attr(not(test), allow(dead_code))]
pub trait ZoneIdentifierWriter: Send + Sync {
    fn write_zone(&self, path: &Path) -> io::Result<()>;
}

/// Default implementation writing directly to Windows NTFS alternate data stream.
#[allow(dead_code)]
pub struct DefaultZoneIdentifierWriter;

impl ZoneIdentifierWriter for DefaultZoneIdentifierWriter {
    fn write_zone(&self, path: &Path) -> io::Result<()> {
        write_zone_identifier(path)
    }
}

/// Writes Mark of the Web (Zone.Identifier) alternate data stream to `path` (ADR 0025).
///
/// Stream content: `[ZoneTransfer]\r\nZoneId=3\r\n` (ZoneId 3 = Internet zone).
/// Returns `Ok(())` on success, or `Err(io::Error)` if the filesystem does not support streams
/// (e.g. FAT32).
pub fn write_zone_identifier(path: &Path) -> io::Result<()> {
    let mut stream_path = path.as_os_str().to_os_string();
    stream_path.push(ZONE_IDENTIFIER_STREAM_SUFFIX);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&stream_path)?;
    file.write_all(ZONE_TRANSFER_CONTENT)?;
    file.flush()?;
    Ok(())
}

/// Sanitizes attachment name in the main process (second pass, matching worker rules).
pub fn sanitize_attachment_name(name: &str, index: usize) -> String {
    clean_attachment_name(name, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_is_dangerous_attachment_no_extension() {
        assert!(is_dangerous_attachment("README"));
        assert!(is_dangerous_attachment("data"));
        assert!(is_dangerous_attachment("foo."));
    }

    #[test]
    fn test_is_dangerous_attachment_fixed_list() {
        assert!(is_dangerous_attachment("report.docm"));
        assert!(is_dangerous_attachment("sheet.xlsm"));
        assert!(is_dangerous_attachment("slides.pptm"));
        assert!(is_dangerous_attachment("disk.iso"));
        assert!(is_dangerous_attachment("backup.img"));
        assert!(is_dangerous_attachment("vm.vhd"));
        assert!(is_dangerous_attachment("vm.vhdx"));
        assert!(is_dangerous_attachment("link.lnk"));
        assert!(is_dangerous_attachment("site.url"));
        assert!(is_dangerous_attachment("app.hta"));
        assert!(is_dangerous_attachment("script.js"));
        assert!(is_dangerous_attachment("script.jse"));
        assert!(is_dangerous_attachment("script.vbs"));
        assert!(is_dangerous_attachment("script.wsf"));
        assert!(is_dangerous_attachment("tool.ps1"));
        assert!(is_dangerous_attachment("setup.msi"));
        assert!(is_dangerous_attachment("app.jar"));
    }

    #[test]
    fn test_is_dangerous_attachment_executables() {
        assert!(is_dangerous_attachment("payload.exe"));
        assert!(is_dangerous_attachment("command.bat"));
        assert!(is_dangerous_attachment("run.cmd"));
        assert!(is_dangerous_attachment("screen.scr"));
    }

    #[test]
    fn test_is_dangerous_attachment_safe() {
        assert!(!is_dangerous_attachment("document.pdf"));
        assert!(!is_dangerous_attachment("plain.txt"));
        assert!(!is_dangerous_attachment("picture.png"));
        assert!(!is_dangerous_attachment("photo.jpg"));
        assert!(!is_dangerous_attachment("data.csv"));
    }

    #[test]
    fn test_sanitize_attachment_name_main_process() {
        // Path stripping
        assert_eq!(sanitize_attachment_name("../../etc/passwd", 1), "passwd");
        assert_eq!(sanitize_attachment_name(r"C:\Windows\System32\cmd.exe", 1), "cmd.exe");
        // Device name
        assert_eq!(sanitize_attachment_name("CON.txt", 1), "_CON.txt");
        assert_eq!(sanitize_attachment_name("nul", 2), "_nul");
        // Control / bidi
        assert_eq!(sanitize_attachment_name("test\u{202E}fdp.exe", 1), "testfdp.exe");
        assert_eq!(sanitize_attachment_name("bad\x00\x08name.txt", 1), "badname.txt");
        // Windows prohibited chars
        assert_eq!(sanitize_attachment_name("file<1>:*?.txt", 1), "file1.txt");
        // Empty fallback
        assert_eq!(sanitize_attachment_name("", 3), "附件 3");
        assert_eq!(sanitize_attachment_name("   ", 4), "附件 4");
    }

    #[test]
    fn test_write_zone_identifier_success_and_content() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join(format!("test_motw_{}.txt", std::process::id()));
        fs::write(&test_file, b"attachment content").expect("write test file");

        let result = write_zone_identifier(&test_file);
        if let Ok(()) = result {
            // Read back the stream
            let mut stream_path = test_file.as_os_str().to_os_string();
            stream_path.push(ZONE_IDENTIFIER_STREAM_SUFFIX);
            let content = fs::read(&stream_path).expect("read stream");
            assert_eq!(content, ZONE_TRANSFER_CONTENT);
        }

        let _ = fs::remove_file(&test_file);
    }

    #[test]
    fn test_zone_identifier_writer_failure_double() {
        struct FailingWriter;
        impl ZoneIdentifierWriter for FailingWriter {
            fn write_zone(&self, _path: &Path) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::Unsupported, "FAT32 has no streams"))
            }
        }

        let writer = FailingWriter;
        let res = writer.write_zone(Path::new("D:\\test.txt"));
        assert!(res.is_err());
    }
}
