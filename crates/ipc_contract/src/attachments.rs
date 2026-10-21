//! Attachment name cleaning and helpers (B3-20, ADR 0025).
//!
//! Attachment names originate from untrusted PDF files and may contain path traversal,
//! control characters, bidirectional override tricks (such as RLO), illegal filesystem
//! characters, or reserved device names. Both worker and main process sanitize names
//! using these rules.

use crate::limits::MAX_ATTACHMENT_NAME_BYTES;

/// Returns true if character changes direction or invisible text formatting.
fn is_bidi_or_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Returns true if character is prohibited in Windows file names or paths.
fn is_windows_prohibited(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Checks if a filename stem matches a reserved Windows device name (case-insensitive).
pub fn is_reserved_device_name(stem: &str) -> bool {
    let s = stem.trim();
    if s.len() == 3 {
        let upper = s.to_ascii_uppercase();
        if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
            return true;
        }
    } else if s.len() == 4 {
        let upper = s.to_ascii_uppercase();
        let prefix = &upper[..3];
        let suffix = upper.as_bytes()[3];
        if (prefix == "COM" || prefix == "LPT") && (b'1'..=b'9').contains(&suffix) {
            return true;
        }
    }
    false
}

/// Sanitizes an attachment filename according to ADR 0025:
/// 1. Takes only the final path segment (strips Unix and Windows paths).
/// 2. Removes control characters, bidi controls (U+202A–U+202E, U+2066–U+2069, U+200E, U+200F),
///    and Windows prohibited characters (`< > : " / \ | ? *`).
/// 3. Removes trailing dots and whitespace.
/// 4. Adds leading `_` if the stem matches a Windows device name (`CON`, `NUL`, `COM1`, etc.).
/// 5. Replaces empty names with "附件 n" (1-based index).
/// 6. Enforces length limit (`MAX_ATTACHMENT_NAME_BYTES`).
pub fn clean_attachment_name(raw: &str, index: usize) -> String {
    // 1. Take only the last path component
    let path_cleaned = raw
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .last()
        .unwrap_or("");

    // 2. Filter out control characters, bidi controls, and prohibited characters
    let mut filtered = String::with_capacity(path_cleaned.len());
    for c in path_cleaned.chars() {
        if c.is_control() || is_bidi_or_invisible(c) || is_windows_prohibited(c) {
            continue;
        }
        filtered.push(c);
    }

    // 3. Strip trailing dots and whitespace
    let mut trimmed = filtered.as_str();
    while trimmed.ends_with('.') || trimmed.ends_with(' ') || trimmed.ends_with('\t') {
        trimmed = trimmed.trim_end_matches(['.', ' ', '\t']);
    }

    // 4. Enforce max byte length if needed
    let mut result = if trimmed.len() > MAX_ATTACHMENT_NAME_BYTES as usize {
        let mut end = MAX_ATTACHMENT_NAME_BYTES as usize;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = &trimmed[..end];
        truncated.trim_end_matches(['.', ' ', '\t']).to_owned()
    } else {
        trimmed.to_owned()
    };

    // 5. If empty, fall back to "附件 n"
    if result.is_empty() {
        return format!("附件 {index}");
    }

    // 6. Check for reserved device name (stem before first dot, or entire name)
    let stem = result.split('.').next().unwrap_or("");
    if is_reserved_device_name(stem) {
        result.insert(0, '_');
    }

    result
}

/// Extracts the genuine extension including leading dot in lowercase (e.g. `".pdf"`, `".exe"`),
/// or returns an empty string if there is no extension.
pub fn attachment_extension(name: &str) -> String {
    let trimmed = name.trim();
    if let Some(pos) = trimmed.rfind('.') {
        if pos > 0 && pos < trimmed.len() - 1 {
            return trimmed[pos..].to_ascii_lowercase();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_name() {
        assert_eq!(clean_attachment_name("document.pdf", 1), "document.pdf");
        assert_eq!(attachment_extension("document.pdf"), ".pdf");
    }

    #[test]
    fn test_path_traversal() {
        assert_eq!(clean_attachment_name("../../etc/passwd", 1), "passwd");
        assert_eq!(clean_attachment_name("C:\\Windows\\notepad.exe", 1), "notepad.exe");
        assert_eq!(clean_attachment_name("foo/bar/baz.txt", 1), "baz.txt");
        assert_eq!(clean_attachment_name("///", 1), "附件 1");
    }

    #[test]
    fn test_control_and_bidi_characters() {
        // RLO: U+202E
        let rlo = "invoice\u{202E}fdp.exe";
        assert_eq!(clean_attachment_name(rlo, 1), "invoicefdp.exe");
        assert_eq!(attachment_extension("invoicefdp.exe"), ".exe");

        // Other bidi controls: LRE, RLE, PDF, LRO, LRI, RLI, FSI, PDI, LRM, RLM
        let bidi = "test\u{202A}\u{202B}\u{202C}\u{202D}\u{2066}\u{2067}\u{2068}\u{2069}\u{200E}\u{200F}.txt";
        assert_eq!(clean_attachment_name(bidi, 1), "test.txt");

        // Control characters: \0, \r, \n, \t
        assert_eq!(clean_attachment_name("a\x00b\rc\nd\te.txt", 1), "abcde.txt");
    }

    #[test]
    fn test_prohibited_characters() {
        let prohibited = "file<name>:with\"quotes\"/and\\slashes|and?stars*.doc";
        // slashes split the path, so last segment is "and\slashes|and?stars*.doc" -> "slashes|and?stars*.doc"
        // Then prohibited chars stripped:
        assert_eq!(clean_attachment_name(prohibited, 1), "slashesandstars.doc");
    }

    #[test]
    fn test_trailing_dots_and_spaces() {
        assert_eq!(clean_attachment_name("file.txt...   ", 1), "file.txt");
        assert_eq!(clean_attachment_name("report.   .  ", 1), "report");
    }

    #[test]
    fn test_reserved_device_names() {
        assert_eq!(clean_attachment_name("CON.txt", 1), "_CON.txt");
        assert_eq!(clean_attachment_name("con.tar.gz", 1), "_con.tar.gz");
        assert_eq!(clean_attachment_name("NUL", 1), "_NUL");
        assert_eq!(clean_attachment_name("aux.dat", 1), "_aux.dat");
        assert_eq!(clean_attachment_name("COM1.pdf", 1), "_COM1.pdf");
        assert_eq!(clean_attachment_name("lpt9.log", 1), "_lpt9.log");
        // Already prefixed with underscore should not add another underscore
        assert_eq!(clean_attachment_name("_CON.txt", 1), "_CON.txt");
    }

    #[test]
    fn test_empty_and_fallback() {
        assert_eq!(clean_attachment_name("", 1), "附件 1");
        assert_eq!(clean_attachment_name("   ", 2), "附件 2");
        assert_eq!(clean_attachment_name("....", 3), "附件 3");
        assert_eq!(clean_attachment_name("\u{202E}\u{202D}", 4), "附件 4");
    }

    #[test]
    fn test_attachment_extension() {
        assert_eq!(attachment_extension("test.PDF"), ".pdf");
        assert_eq!(attachment_extension("payload.exe"), ".exe");
        assert_eq!(attachment_extension("no_extension"), "");
        assert_eq!(attachment_extension(".hidden"), "");
        assert_eq!(attachment_extension("ending_with_dot."), "");
    }
}
