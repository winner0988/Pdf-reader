//! Attachment names (B3-20, ADR 0025).
//!
//! An attachment's name comes from an untrusted PDF: it may hold a path, control characters,
//! bidirectional overrides (U+202E can make "exe.txt" read as "txt.exe"), characters Windows does
//! not allow, or a device name. The worker and the main process both clean it with these rules.

use crate::limits::MAX_ATTACHMENT_NAME_BYTES;
use crate::text::is_invisible_format;

/// Longest extension kept when a long name is cut, dot included.
const MAX_KEPT_EXTENSION_BYTES: usize = 16;

/// Characters Windows does not allow in a file name.
fn is_windows_prohibited(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Whether `stem` (the name before its first dot) is a Windows device name: `CON`, `PRN`, `AUX`,
/// `NUL`, `CONIN$`, `CONOUT$`, and `COM`/`LPT` with a digit from 0 to 9 or a superscript 1 to 3.
/// Windows ignores spaces after the device name, so they are too.
pub fn is_reserved_device_name(stem: &str) -> bool {
    let upper = stem.trim_end_matches(' ').to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let (Some(prefix), Some(number)) = (upper.get(..3), upper.get(3..)) else {
        return false;
    };
    matches!(prefix, "COM" | "LPT")
        && (matches!(number, "¹" | "²" | "³")
            || (number.len() == 1 && number.as_bytes()[0].is_ascii_digit()))
}

/// Cuts `name` to at most `max` bytes at a character boundary, keeping a short extension, so that
/// a long name keeps its type.
fn shorten(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    let keep_from = match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= MAX_KEPT_EXTENSION_BYTES => dot,
        _ => name.len(),
    };
    let mut end = max.saturating_sub(name.len() - keep_from).min(keep_from);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let stem = name[..end].trim_end_matches(['.', ' ']);
    format!("{stem}{}", &name[keep_from..])
}

/// Cleans an attachment's name (ADR 0025):
/// 1. only the last part of a path is kept;
/// 2. control characters, invisible formatting characters (bidirectional controls, marks,
///    zero-width characters) and characters Windows does not allow are removed;
/// 3. dots and spaces at the end are removed;
/// 4. a device name (`CON`, `NUL`, `COM1`…) gets `_` in front;
/// 5. the name is cut to `MAX_ATTACHMENT_NAME_BYTES`, keeping a short extension;
/// 6. an empty name becomes "附件 n" (`index` counts from 1).
pub fn clean_attachment_name(raw: &str, index: usize) -> String {
    let last = raw
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("");
    let kept: String = last
        .chars()
        .filter(|&c| !(c.is_control() || is_invisible_format(c) || is_windows_prohibited(c)))
        .collect();
    let mut name = kept.trim_end_matches(['.', ' ']).to_owned();
    if name.is_empty() {
        return format!("附件 {index}");
    }
    let stem = name.split('.').next().unwrap_or("");
    if is_reserved_device_name(stem) {
        name.insert(0, '_');
    }
    shorten(&name, MAX_ATTACHMENT_NAME_BYTES as usize)
}

/// The extension of `name` with its dot, in lowercase (".pdf", ".exe"), or "" when it has none.
/// A name that only starts with a dot (".hidden") has none.
pub fn attachment_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot < name.len() - 1 => name[dot..].to_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_stays() {
        assert_eq!(clean_attachment_name("document.pdf", 1), "document.pdf");
        assert_eq!(
            clean_attachment_name(" leading space.txt", 1),
            " leading space.txt"
        );
    }

    #[test]
    fn only_the_last_part_of_a_path_is_kept() {
        assert_eq!(clean_attachment_name("../../etc/passwd", 1), "passwd");
        assert_eq!(
            clean_attachment_name("C:\\Windows\\notepad.exe", 1),
            "notepad.exe"
        );
        assert_eq!(clean_attachment_name("foo/bar/baz.txt/", 1), "baz.txt");
        assert_eq!(
            clean_attachment_name("\\\\server\\share\\a.doc", 1),
            "a.doc"
        );
        assert_eq!(clean_attachment_name("///", 1), "附件 1");
    }

    #[test]
    fn control_and_invisible_characters_are_removed() {
        assert_eq!(
            clean_attachment_name("invoice\u{202E}fdp.exe", 1),
            "invoicefdp.exe"
        );
        let bidi = "test\u{202A}\u{202B}\u{202C}\u{202D}\u{2066}\u{2067}\u{2068}\u{2069}\u{200E}\u{200F}.txt";
        assert_eq!(clean_attachment_name(bidi, 1), "test.txt");
        assert_eq!(
            clean_attachment_name("zero\u{200B}width\u{FEFF}.txt", 1),
            "zerowidth.txt"
        );
        assert_eq!(
            clean_attachment_name("a\x00b\rc\nd\te\u{85}f.txt", 1),
            "abcdef.txt"
        );
    }

    #[test]
    fn characters_windows_does_not_allow_are_removed() {
        assert_eq!(clean_attachment_name("file<1>:*?\"|.txt", 1), "file1.txt");
        // No alternate data stream can be named.
        assert_eq!(
            clean_attachment_name("report.txt:hidden", 1),
            "report.txthidden"
        );
    }

    #[test]
    fn dots_and_spaces_at_the_end_are_removed() {
        assert_eq!(clean_attachment_name("file.txt...   ", 1), "file.txt");
        assert_eq!(clean_attachment_name("report.   .  ", 1), "report");
        assert_eq!(clean_attachment_name("evil.exe\u{202E}. ", 1), "evil.exe");
    }

    #[test]
    fn device_names_get_an_underscore() {
        for name in [
            "CON.txt",
            "con.tar.gz",
            "NUL",
            "aux.dat",
            "COM1.pdf",
            "lpt9.log",
            "COM0",
            "LPT¹.txt",
            "CONOUT$.log",
            "nul .txt",
        ] {
            assert_eq!(clean_attachment_name(name, 1), format!("_{name}"), "{name}");
        }
        for name in [
            "_CON.txt",
            "CONSOLE.txt",
            "COM10.txt",
            "COMA.txt",
            "nullable.txt",
        ] {
            assert_eq!(clean_attachment_name(name, 1), name, "{name}");
        }
    }

    #[test]
    fn an_empty_name_is_numbered() {
        assert_eq!(clean_attachment_name("", 1), "附件 1");
        assert_eq!(clean_attachment_name("   ", 2), "附件 2");
        assert_eq!(clean_attachment_name("....", 3), "附件 3");
        assert_eq!(clean_attachment_name("\u{202E}\u{202D}", 4), "附件 4");
    }

    #[test]
    fn a_long_name_is_cut_and_keeps_its_extension() {
        let max = MAX_ATTACHMENT_NAME_BYTES as usize;
        let cleaned = clean_attachment_name(&format!("{}.exe", "a".repeat(400)), 1);
        assert_eq!(cleaned.len(), max);
        assert!(cleaned.ends_with("a.exe"));

        // Cut at a character boundary, within the limit.
        let cleaned = clean_attachment_name(&format!("{}.pdf", "字".repeat(200)), 1);
        assert!(
            cleaned.len() <= max && cleaned.ends_with("字.pdf"),
            "{cleaned}"
        );

        // With the underscore of a device name, still within the limit.
        let cleaned = clean_attachment_name(&format!("CON.{}", "x".repeat(400)), 1);
        assert!(cleaned.starts_with("_CON.") && cleaned.len() == max);

        // A long "extension" is not a type to keep: the end is cut.
        let cleaned = clean_attachment_name(&format!("name.{}", "e".repeat(300)), 1);
        assert_eq!(cleaned.len(), max);
    }

    #[test]
    fn the_extension_is_the_part_after_the_last_dot() {
        assert_eq!(attachment_extension("test.PDF"), ".pdf");
        assert_eq!(attachment_extension("archive.tar.GZ"), ".gz");
        assert_eq!(attachment_extension("no_extension"), "");
        assert_eq!(attachment_extension(".hidden"), "");
        assert_eq!(attachment_extension("ending_with_dot."), "");
    }
}
