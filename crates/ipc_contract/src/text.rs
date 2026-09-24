//! Text taken from a PDF and shown to the user (outline titles, link targets). A PDF controls this
//! text completely, so it is reduced to plain, single-line, left-to-right-safe text before it
//! leaves the worker, and the main process rejects anything that was not.

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

/// Classifies a URI from a PDF. Only `http`, `https` and `mailto` may ever be offered to the
/// user (AGENTS.md principle 5); everything else is a blocked action shown as text.
pub fn classify_uri(uri: &str) -> LinkTarget {
    let scheme = uri
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .unwrap_or_default();
    let openable = matches!(scheme.as_str(), "http" | "https" | "mailto");
    let clean = uri.len() <= MAX_URI_BYTES as usize
        && !uri
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || is_invisible_format(c));
    if openable && clean {
        return LinkTarget::Uri {
            uri: uri.to_owned(),
        };
    }
    let action = match scheme.as_str() {
        "javascript" => BlockedAction::JavaScript,
        "file" => BlockedAction::RemoteGoTo,
        _ => BlockedAction::Other,
    };
    let target = clean_display_text(uri, MAX_TEXT_BYTES as usize);
    LinkTarget::Blocked {
        action,
        target: (!target.is_empty()).then_some(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                action: BlockedAction::RemoteGoTo,
                ..
            }
        ));
        for uri in [
            "smb://server/share",
            "\\\\server\\share",
            "ms-settings:",
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
    fn a_web_uri_with_hidden_characters_is_not_openable() {
        let hidden = "https://example.com/\u{202E}gpj.exe";
        assert!(matches!(classify_uri(hidden), LinkTarget::Blocked { .. }));
        assert!(matches!(
            classify_uri("https://exa mple.com"),
            LinkTarget::Blocked { .. }
        ));
    }
}
