//! Word and character tokenization for document text comparison (B3-24, E9).
//!
//! Latin letters, digits, and alphanumeric sequences are delimited by whitespace and punctuation.
//! CJK ideographs and fullwidth punctuation are tokenized as single-character units.
//! Each token remembers the 0-indexed page index where it appeared and whether whitespace
//! preceded it.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use ipc_contract::types::TextLine;

/// Whether `c` is a CJK ideograph, Hangul syllable, or fullwidth character/punctuation.
pub fn is_wide(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}')
}

/// A token unit in a document, carrying its text slice, origin page, and leading whitespace flag.
#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub page: u32,
    pub has_leading_space: bool,
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Token {}

impl PartialOrd for Token {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Token {
    fn cmp(&self, other: &Self) -> Ordering {
        self.text.cmp(&other.text)
    }
}

impl Hash for Token {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// Tokenizes a single page of text lines into comparison units.
pub fn tokenize_page(page_index: u32, lines: &[TextLine]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current_word = String::new();
    let mut pending_space = false;
    let mut word_had_space = false;

    for (line_idx, line) in lines.iter().enumerate() {
        if line_idx > 0 {
            // Newline between lines acts as whitespace
            pending_space = true;
        }
        for c in line.text.chars() {
            if c.is_whitespace() {
                if !current_word.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current_word),
                        page: page_index,
                        has_leading_space: word_had_space,
                    });
                }
                pending_space = true;
            } else if is_wide(c) {
                if !current_word.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current_word),
                        page: page_index,
                        has_leading_space: word_had_space,
                    });
                }
                tokens.push(Token {
                    text: c.to_string(),
                    page: page_index,
                    has_leading_space: pending_space,
                });
                pending_space = false;
            } else if c.is_alphanumeric() {
                if current_word.is_empty() {
                    word_had_space = pending_space;
                    pending_space = false;
                }
                current_word.push(c);
            } else {
                // Punctuation or symbol: flush current word and emit as individual unit
                if !current_word.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current_word),
                        page: page_index,
                        has_leading_space: word_had_space,
                    });
                }
                tokens.push(Token {
                    text: c.to_string(),
                    page: page_index,
                    has_leading_space: pending_space,
                });
                pending_space = false;
            }
        }
        if !current_word.is_empty() {
            tokens.push(Token {
                text: std::mem::take(&mut current_word),
                page: page_index,
                has_leading_space: word_had_space,
            });
        }
    }

    tokens
}

/// Joins a sequence of tokens into human-readable text for diff display and export.
pub fn join_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 && token.has_leading_space {
            out.push(' ');
        }
        out.push_str(&token.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_contract::types::{Point, Quad, TextLine};

    fn make_line(text: &str) -> TextLine {
        TextLine {
            text: text.to_owned(),
            quad: Quad {
                ul: Point { x: 0.0, y: 0.0 },
                ur: Point { x: 100.0, y: 0.0 },
                ll: Point { x: 0.0, y: 10.0 },
                lr: Point { x: 100.0, y: 10.0 },
            },
            edges: vec![0.0; text.chars().count() + 1],
        }
    }

    #[test]
    fn tokenization_mixed_chinese_and_english() {
        let lines = vec![make_line("Hello, world! 這是測試。")];
        let tokens = tokenize_page(0, &lines);
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Hello", ",", "world", "!", "這", "是", "測", "試", "。"]
        );
        for t in &tokens {
            assert_eq!(t.page, 0);
        }

        let joined = join_tokens(&tokens);
        assert_eq!(joined, "Hello, world! 這是測試。");
    }

    #[test]
    fn tokenization_punctuation_and_fullwidth() {
        let lines = vec![make_line("（一）合約 Clause 1.2: 甲方 ($100)")];
        let tokens = tokenize_page(1, &lines);
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "（", "一", "）", "合", "約", "Clause", "1", ".", "2", ":", "甲", "方", "(", "$",
                "100", ")"
            ]
        );
        for t in &tokens {
            assert_eq!(t.page, 1);
        }

        let joined = join_tokens(&tokens);
        assert_eq!(joined, "（一）合約 Clause 1.2: 甲方 ($100)");
    }
}
