//! Text taken from a PDF and shown to the user (outline titles, link targets) or copied (a page's
//! text, MVP-15). A PDF controls this text completely, so it is reduced to plain,
//! left-to-right-safe text before it leaves the worker, and the main process rejects anything
//! that was not.

use crate::limits::{MAX_TEXT_BYTES, MAX_URI_BYTES};
use crate::types::{BlockedAction, LinkTarget};

/// Characters that change how surrounding text is displayed without being visible themselves:
/// bidirectional embeddings, overrides and isolates (U+202E can make "exe.txt" read as "txt.exe"),
/// directional marks, zero-width characters and the byte order mark.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Plain display text: control characters (including line breaks and tabs) become spaces,
/// invisible formatting characters are removed, runs of spaces collapse, and the result is
/// trimmed and cut to `max_bytes` at a character boundary.
pub fn clean_display_text(input: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(input.len().min(max_bytes));
    let mut pending_space = false;
    for c in input.chars() {
        if is_invisible_format(c) {
            continue;
        }
        if c.is_control() || c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        let needed = c.len_utf8() + usize::from(pending_space);
        if out.len() + needed > max_bytes {
            break;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}

/// Whether `text` is already what [`clean_display_text`] would produce.
pub fn is_clean_display_text(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c.is_control() || is_invisible_format(c) || (c.is_whitespace() && c != ' '))
        && !text.starts_with(' ')
        && !text.ends_with(' ')
        && !text.contains("  ")
}

/// A character of a page's text as it is selected and copied (MVP-15): whitespace and control
/// characters become a space, and invisible formatting characters are dropped (`None`), so that
/// pasted text reads as it looks. Unlike display text, spaces are not collapsed: every character
/// keeps its place on the page.
pub fn copy_text_char(c: char) -> Option<char> {
    if is_invisible_format(c) {
        return None;
    }
    if c.is_control() || c.is_whitespace() {
        return Some(' ');
    }
    Some(c)
}

/// Whether `text` is already what [`copy_text_char`] would produce.
pub fn is_clean_copy_text(text: &str) -> bool {
    text.chars().all(|c| copy_text_char(c) == Some(c))
}

/// Whether `text` can be what a note says (B2-07): lines of text, without control characters
/// other than the line breaks (`\n`) or invisible formatting characters. The user's own spacing
/// is kept.
pub fn is_note_text(text: &str) -> bool {
    !text
        .chars()
        .any(|c| (c.is_control() && c != '\n') || is_invisible_format(c))
}

/// What a note in a PDF says, as [`is_note_text`] accepts it: each line is display text (see
/// [`clean_display_text`]), empty lines at the start and the end are dropped, and the whole is
/// cut to `max_bytes` at a line or character boundary.
pub fn clean_note_text(input: &str, max_bytes: usize) -> String {
    let lines: Vec<String> = input
        .split(['\n', '\r'])
        .map(|line| clean_display_text(line, max_bytes))
        .collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return String::new();
    };
    let mut out = String::new();
    for line in &lines[first..=last] {
        let separator = usize::from(!out.is_empty());
        if out.len() + separator + line.len() > max_bytes {
            // The part of the line that fits, cut at a character boundary.
            let cut = clean_display_text(line, max_bytes.saturating_sub(out.len() + separator));
            if !cut.is_empty() {
                if separator == 1 {
                    out.push('\n');
                }
                out.push_str(&cut);
            }
            break;
        }
        if separator == 1 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

/// A form field's text as the page shows and edits it (B2-09): line breaks are LF (a space in a
/// single-line field), a tab is a space, control and invisible formatting characters are dropped,
/// and the spacing stays. Cut at `max_bytes` at a character boundary; `true` says it was cut.
pub fn clean_field_text(input: &str, max_bytes: usize, multiline: bool) -> (String, bool) {
    let mut out = String::with_capacity(input.len().min(max_bytes));
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                '\n'
            }
            '\t' => ' ',
            c => c,
        };
        let c = if c == '\n' && !multiline { ' ' } else { c };
        if (c.is_control() && c != '\n') || is_invisible_format(c) {
            continue;
        }
        if out.len() + c.len_utf8() > max_bytes {
            return (out, true);
        }
        out.push(c);
    }
    (out, false)
}

/// Classifies a URI from a PDF. Only `http`, `https` and `mailto` may ever be offered to the
/// user (AGENTS.md principle 5); everything else is a blocked action shown as text.
///
/// An openable URI is kept exactly as written, hidden characters included: the confirmation
/// shows them marked and warns (docs/ux/screen-map.md, section 4) instead of silently removing
/// what the PDF put there.
pub fn classify_uri(uri: &str) -> LinkTarget {
    let scheme = uri
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .unwrap_or_default();
    let openable = matches!(scheme.as_str(), "http" | "https" | "mailto");
    if openable && uri.len() <= MAX_URI_BYTES as usize {
        return LinkTarget::Uri {
            uri: uri.to_owned(),
        };
    }
    let action = if is_network_path(uri.as_bytes()) {
        BlockedAction::NetworkShare
    } else {
        match scheme.as_str() {
            "javascript" => BlockedAction::JavaScript,
            "file" => BlockedAction::LocalFile,
            _ => BlockedAction::Other,
        }
    };
    let target = clean_display_text(uri, MAX_TEXT_BYTES as usize);
    LinkTarget::Blocked {
        action,
        target: (!target.is_empty()).then_some(target),
    }
}

/// A file name or URI that reaches another computer: a UNC path (`\\server\share`, also written
/// with forward slashes), `file://server/...` or `smb:`. On Windows, opening one can send the
/// user's account hash to that server (SMB/NTLM).
///
/// `raw` is a PDF string: UTF-16BE with a byte order mark, or single bytes (PDFDocEncoding,
/// which agrees with ASCII for everything looked at here).
pub fn is_network_path(raw: &[u8]) -> bool {
    let text = decode_pdf_string(raw).to_ascii_lowercase();
    let text = text.trim_start();
    let mut chars = text.chars();
    let slash = |c: Option<char>| matches!(c, Some('\\' | '/'));
    if slash(chars.next()) && slash(chars.next()) {
        return true;
    }
    if let Some(rest) = text.strip_prefix("file:") {
        // file://server/share and file:////server/share; file:/// and file://localhost/ are local.
        let rest = rest.trim_start_matches(['/', '\\']);
        let slashes = text.len() - "file:".len() - rest.len();
        return (slashes == 2 && !rest.starts_with("localhost")) || slashes >= 4;
    }
    text.starts_with("smb:") || text.starts_with("cifs:")
}

fn decode_pdf_string(raw: &[u8]) -> String {
    match raw {
        [0xfe, 0xff, rest @ ..] => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => raw.iter().map(|&byte| char::from(byte)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fields_text_keeps_its_spacing_and_loses_what_is_not_text() {
        let from_pdf = "  one\r\n two\rthree\tfour\u{7}\u{202E}\u{200B}!  ";
        // Several lines, as a multi-line field has them; the spacing stays.
        let (many, cut) = clean_field_text(from_pdf, 1_000, true);
        assert_eq!(many, "  one\n two\nthree four!  ");
        assert!(!cut);
        assert!(is_note_text(&many));
        // A single-line field has one line.
        assert_eq!(
            clean_field_text(from_pdf, 1_000, false).0,
            "  one  two three four!  "
        );
        // Cut at a character boundary, and it says so.
        let (cut_text, cut) = clean_field_text("abc中文字", 7, true);
        assert_eq!((cut_text.as_str(), cut), ("abc中", true));
        assert_eq!(clean_field_text("abc", 3, true), ("abc".to_owned(), false));
    }

    #[test]
    fn a_note_keeps_its_lines_and_loses_everything_else() {
        let from_pdf = "\r\n  第一行\u{202E}  text\t here \r\rsecond\u{200B} line\n\n";
        let note = clean_note_text(from_pdf, MAX_TEXT_BYTES as usize);
        // Each line is display text; the empty line between paragraphs stays, those around go.
        assert_eq!(note, "第一行 text here\n\nsecond line");
        assert!(is_note_text(&note));
        assert_eq!(clean_note_text(" \n\t\n ", 100), "");
        // Cut at a character boundary, within the limit.
        let cut = clean_note_text("abc\n中文字", 7);
        assert_eq!(cut, "abc\n中");
        assert!(cut.len() <= 7);
        // What the user types keeps its spacing; control and invisible characters do not pass.
        assert!(is_note_text("  indented\nand  spaced  "));
        assert!(!is_note_text("tab\there"));
        assert!(!is_note_text("bell\u{7}"));
        assert!(!is_note_text("abc\u{202E}def"));
    }

    #[test]
    fn copied_text_keeps_every_visible_character_in_place() {
        let copied: String = "隱私\u{3000}first\t\u{202E}PDF\u{200B}  x\n"
            .chars()
            .filter_map(copy_text_char)
            .collect();
        // Whitespace stays (as a space) so that characters keep their positions; nothing is
        // collapsed or trimmed, unlike display text.
        assert_eq!(copied, "隱私 first PDF  x ");
        assert!(is_clean_copy_text(&copied));
        assert!(!is_clean_copy_text("a\u{202E}b"));
        assert!(!is_clean_copy_text("a\u{00A0}b"));
    }

    #[test]
    fn keeps_ordinary_titles() {
        assert_eq!(clean_display_text("第 1 章 簡介", 100), "第 1 章 簡介");
        assert!(is_clean_display_text("第 1 章 簡介"));
    }

    #[test]
    fn removes_bidi_overrides_and_invisible_characters() {
        // "Invoice \u{202E}fdp.exe" would display as "Invoice exe.pdf".
        let spoof = "Invoice \u{202E}fdp.exe";
        assert_eq!(clean_display_text(spoof, 100), "Invoice fdp.exe");
        assert_eq!(
            clean_display_text("a\u{200B}b\u{FEFF}c\u{2066}d\u{2069}", 100),
            "abcd"
        );
        assert!(!is_clean_display_text(spoof));
    }

    #[test]
    fn turns_control_characters_and_line_breaks_into_single_spaces() {
        assert_eq!(
            clean_display_text("  One\r\n\ttwo\u{0}\u{7}three  ", 100),
            "One two three"
        );
        assert!(!is_clean_display_text("One\ntwo"));
        assert!(!is_clean_display_text(" padded"));
        assert!(!is_clean_display_text("two  spaces"));
    }

    #[test]
    fn cuts_long_text_at_a_character_boundary() {
        let long = "文".repeat(1000);
        let cut = clean_display_text(&long, 1024);
        assert!(cut.len() <= 1024);
        assert_eq!(cut.chars().count(), 341);
        assert!(is_clean_display_text(&cut));
    }

    #[test]
    fn only_web_and_mail_uris_are_openable() {
        for uri in [
            "https://example.com/a?b=c",
            "HTTP://example.com",
            "mailto:someone@example.com",
        ] {
            assert!(matches!(classify_uri(uri), LinkTarget::Uri { .. }), "{uri}");
        }
        assert_eq!(
            classify_uri("javascript:alert(1)"),
            LinkTarget::Blocked {
                action: BlockedAction::JavaScript,
                target: Some("javascript:alert(1)".to_owned())
            }
        );
        assert!(matches!(
            classify_uri("file:///C:/Windows/System32/calc.exe"),
            LinkTarget::Blocked {
                action: BlockedAction::LocalFile,
                ..
            }
        ));
        for uri in [
            "smb://server/share",
            "\\\\server\\share",
            "\\\\share.example.invalid\\x",
            "file://server/share/doc.pdf",
        ] {
            assert!(
                matches!(
                    classify_uri(uri),
                    LinkTarget::Blocked {
                        action: BlockedAction::NetworkShare,
                        ..
                    }
                ),
                "{uri}"
            );
        }
        for uri in [
            "ms-settings:",
            "ms-msdt:/id",
            "search-ms:query=x",
            "data:text/html,x",
        ] {
            assert!(
                matches!(
                    classify_uri(uri),
                    LinkTarget::Blocked {
                        action: BlockedAction::Other,
                        ..
                    }
                ),
                "{uri}"
            );
        }
    }

    #[test]
    fn a_web_uri_keeps_its_hidden_characters_for_the_confirmation_to_show() {
        let hidden = "https://example.com/\u{202E}gpj.exe";
        assert_eq!(
            classify_uri(hidden),
            LinkTarget::Uri {
                uri: hidden.to_owned()
            }
        );
        let too_long = format!("https://example.com/{}", "a".repeat(MAX_URI_BYTES as usize));
        assert!(matches!(
            classify_uri(&too_long),
            LinkTarget::Blocked {
                action: BlockedAction::Other,
                ..
            }
        ));
    }

    #[test]
    fn recognises_network_paths() {
        for path in [
            &b"\\\\server\\share\\doc.pdf"[..],
            b"//server/share/doc.pdf",
            b"\\/server/share",
            b"  \\\\server",
            b"file://server/share/doc.pdf",
            b"FILE://Server/x",
            b"file:////server/share",
            b"smb://server/share",
            b"\xfe\xff\x00\\\x00\\\x00s",
        ] {
            assert!(is_network_path(path), "{}", String::from_utf8_lossy(path));
        }
        for path in [
            &b"doc.pdf"[..],
            b"C:\\Users\\doc.pdf",
            b"/C/Users/doc.pdf",
            b"file:///C:/doc.pdf",
            b"file://localhost/C:/doc.pdf",
            b"https://example.com/doc.pdf",
            b"",
        ] {
            assert!(!is_network_path(path), "{}", String::from_utf8_lossy(path));
        }
    }
}
