//! How search matches text (MVP-10): shared by the worker, which searches the pages of one open
//! document, and the main process, which searches the text index of the documents opened before
//! (B3-21, ADR 0026), so that both find the same things.
//!
//! Matching works on "folded" text: every kind of whitespace becomes one space, runs of spaces
//! collapse, and letters are lowercased unless the search is case sensitive. The search is linear
//! (Knuth-Morris-Pratt), so a page with a huge amount of text cannot make it quadratic.

/// `c` as it is matched. Only one-to-one lowercase mappings are used, so folded text has a
/// character for each character it came from.
pub fn fold(c: char, case_sensitive: bool) -> char {
    // Most text is ASCII: the same answer without the Unicode tables.
    if c.is_ascii() {
        return if c.is_ascii_whitespace() || c.is_ascii_control() {
            ' '
        } else if case_sensitive {
            c
        } else {
            c.to_ascii_lowercase()
        };
    }
    if c.is_whitespace() || c.is_control() {
        return ' ';
    }
    if case_sensitive {
        return c;
    }
    // Only one-to-one mappings keep characters and quads aligned; the rest stay as they are.
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// The query as it is matched: folded like the text and trimmed.
pub fn normalize_query(query: &str, case_sensitive: bool) -> Vec<char> {
    let mut out: Vec<char> = Vec::with_capacity(query.len());
    for c in query.chars() {
        let folded = fold(c, case_sensitive);
        if folded == ' ' && out.last().is_none_or(|&last| last == ' ') {
            continue;
        }
        out.push(folded);
    }
    while out.last() == Some(&' ') {
        out.pop();
    }
    out
}

/// Finds `pattern` in folded text given a character at a time (KMP), without keeping the text:
/// the main process searches the text index this way (B3-21).
#[derive(Debug, Clone)]
pub struct Matcher<'a> {
    pattern: &'a [char],
    /// failure[i]: length of the longest proper prefix of pattern[..=i] that is also a suffix.
    failure: Vec<usize>,
    matched: usize,
}

impl<'a> Matcher<'a> {
    pub fn new(pattern: &'a [char]) -> Self {
        let mut failure = vec![0usize; pattern.len()];
        let mut length = 0;
        for i in 1..pattern.len() {
            while length > 0 && pattern[i] != pattern[length] {
                length = failure[length - 1];
            }
            if pattern[i] == pattern[length] {
                length += 1;
            }
            failure[i] = length;
        }
        Self {
            pattern,
            failure,
            matched: 0,
        }
    }

    /// The next character of the text: true when an occurrence ends with it. Occurrences do not
    /// overlap. An empty pattern never occurs.
    pub fn push(&mut self, c: char) -> bool {
        if self.pattern.is_empty() {
            return false;
        }
        while self.matched > 0 && c != self.pattern[self.matched] {
            self.matched = self.failure[self.matched - 1];
        }
        if c == self.pattern[self.matched] {
            self.matched += 1;
        }
        if self.matched == self.pattern.len() {
            self.matched = 0;
            return true;
        }
        false
    }

    /// Another text starts.
    pub fn reset(&mut self) {
        self.matched = 0;
    }
}

/// Start and end of up to `max` non-overlapping occurrences of `pattern` in `text` (KMP).
pub fn find(text: &[char], pattern: &[char], max: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if pattern.is_empty() || max == 0 {
        return found;
    }
    let mut matcher = Matcher::new(pattern);
    for (i, &c) in text.iter().enumerate() {
        if matcher.push(c) {
            found.push((i + 1 - pattern.len(), i + 1));
            if found.len() == max {
                break;
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_whitespace_and_case_one_character_for_one() {
        assert_eq!(fold('\t', false), ' ');
        assert_eq!(fold('\u{3000}', true), ' ');
        assert_eq!(fold('A', false), 'a');
        assert_eq!(fold('A', true), 'A');
        assert_eq!(fold('\u{0B}', false), ' ');
        assert_eq!(fold('\u{7F}', false), ' ');
        assert_eq!(fold('\u{C9}', false), '\u{E9}');
        // The ASCII shortcut answers as the Unicode tables do.
        for c in '\0'..='\u{7F}' {
            let slow = if c.is_whitespace() || c.is_control() {
                ' '
            } else {
                c.to_lowercase().next().unwrap()
            };
            assert_eq!(fold(c, false), slow, "{c:?}");
        }
        // "İ" lowercases to two characters: it stays itself.
        assert_eq!(fold('\u{130}', false), '\u{130}');
        assert_eq!(
            normalize_query("  Privacy \n Policy  ", false),
            "privacy policy".chars().collect::<Vec<_>>()
        );
    }

    #[test]
    fn finds_up_to_the_limit_without_overlapping() {
        let text: Vec<char> = "aaaaaa".chars().collect();
        let pattern: Vec<char> = "aa".chars().collect();
        assert_eq!(find(&text, &pattern, 10), [(0, 2), (2, 4), (4, 6)]);
        assert_eq!(find(&text, &pattern, 2), [(0, 2), (2, 4)]);
        assert!(find(&text, &[], 10).is_empty());
    }

    #[test]
    fn a_matcher_finds_what_find_finds_a_character_at_a_time() {
        let text: Vec<char> = "abab abcab ababc".chars().collect();
        let pattern: Vec<char> = "abc".chars().collect();
        let mut matcher = Matcher::new(&pattern);
        let ends: Vec<usize> = (0..text.len())
            .filter(|&i| matcher.push(text[i]))
            .map(|i| i + 1)
            .collect();
        let found: Vec<usize> = find(&text, &pattern, 10)
            .into_iter()
            .map(|(_, end)| end)
            .collect();
        assert_eq!(ends, found);
        assert_eq!(ends, [8, 16]);

        // A new text does not go on from where the last one stopped.
        let mut matcher = Matcher::new(&pattern);
        assert!(!matcher.push('a') && !matcher.push('b'));
        matcher.reset();
        assert!(!matcher.push('c'));
        assert!(!Matcher::new(&[]).push('a'));
    }
}
