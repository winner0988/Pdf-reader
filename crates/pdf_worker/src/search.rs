//! Text search over one page's characters (MVP-10). Independent of MuPDF so it can be tested
//! directly; `engine` feeds it the characters of a page's text layer.
//!
//! Matching works on "folded" text: every kind of whitespace becomes one space, runs of spaces
//! collapse, lines are joined with a space (so a phrase can wrap), and letters are lowercased
//! unless the search is case sensitive. The search is linear (Knuth-Morris-Pratt), so a page with
//! a huge amount of text cannot make it quadratic.

use ipc_contract::limits::MAX_QUADS_PER_HIT;
use ipc_contract::types::{Quad, SearchHit};

/// One page's text, folded for matching, with where each character came from.
pub struct PageText {
    case_sensitive: bool,
    chars: Vec<char>,
    /// (line number, quad) of each character; `None` for the space joining two lines.
    sources: Vec<Option<(u32, Quad)>>,
    lines: u32,
    has_text: bool,
}

/// The result of searching one page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSearch {
    pub hits: Vec<SearchHit>,
    pub has_text: bool,
}

fn fold(c: char, case_sensitive: bool) -> char {
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

impl PageText {
    pub fn new(case_sensitive: bool) -> Self {
        Self {
            case_sensitive,
            chars: Vec::new(),
            sources: Vec::new(),
            lines: 0,
            has_text: false,
        }
    }

    /// Adds a line of text: characters with their quads, in reading order.
    pub fn push_line(&mut self, chars: impl IntoIterator<Item = (char, Quad)>) {
        let line = self.lines;
        self.lines += 1;
        if self.chars.last().is_some_and(|&last| last != ' ') {
            self.chars.push(' ');
            self.sources.push(None);
        }
        for (c, quad) in chars {
            let folded = fold(c, self.case_sensitive);
            if folded == ' ' {
                if self.chars.last().is_some_and(|&last| last == ' ') || self.chars.is_empty() {
                    continue;
                }
            } else {
                self.has_text = true;
            }
            self.chars.push(folded);
            self.sources.push(Some((line, quad)));
        }
    }

    /// Finds up to `max_hits` non-overlapping occurrences of `query`.
    pub fn search(&self, query: &str, max_hits: usize) -> PageSearch {
        let pattern = normalize_query(query, self.case_sensitive);
        let hits = find(&self.chars, &pattern, max_hits)
            .into_iter()
            .filter_map(|(start, end)| self.hit(start, end))
            .collect();
        PageSearch {
            hits,
            has_text: self.has_text,
        }
    }

    /// One quad per line covered by the match, from its first to its last character.
    fn hit(&self, start: usize, end: usize) -> Option<SearchHit> {
        let mut quads: Vec<Quad> = Vec::new();
        let mut current_line = None;
        for (line, quad) in self.sources[start..end].iter().flatten() {
            if current_line == Some(*line) {
                let last = quads.last_mut().expect("a quad for the current line");
                last.ur = quad.ur;
                last.lr = quad.lr;
            } else {
                if quads.len() == MAX_QUADS_PER_HIT as usize {
                    break;
                }
                quads.push(*quad);
                current_line = Some(*line);
            }
        }
        (!quads.is_empty()).then_some(SearchHit { quads })
    }
}

/// The query as it is matched: folded like the page text and trimmed.
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

/// Start and end of up to `max` non-overlapping occurrences of `pattern` in `text` (KMP).
pub fn find(text: &[char], pattern: &[char], max: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if pattern.is_empty() || max == 0 {
        return found;
    }
    // failure[i]: length of the longest proper prefix of pattern[..=i] that is also a suffix.
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
    let mut matched = 0;
    for (i, &c) in text.iter().enumerate() {
        while matched > 0 && c != pattern[matched] {
            matched = failure[matched - 1];
        }
        if c == pattern[matched] {
            matched += 1;
        }
        if matched == pattern.len() {
            found.push((i + 1 - pattern.len(), i + 1));
            if found.len() == max {
                break;
            }
            matched = 0; // non-overlapping
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::Point;

    use super::*;

    /// A quad for character `column` on `row`, 10 x 10 points each.
    fn quad(column: usize, row: usize) -> Quad {
        let (x, y) = (column as f32 * 10.0, row as f32 * 12.0);
        Quad {
            ul: Point { x, y },
            ur: Point { x: x + 10.0, y },
            ll: Point { x, y: y + 10.0 },
            lr: Point {
                x: x + 10.0,
                y: y + 10.0,
            },
        }
    }

    fn page(lines: &[&str], case_sensitive: bool) -> PageText {
        let mut text = PageText::new(case_sensitive);
        for (row, line) in lines.iter().enumerate() {
            text.push_line(
                line.chars()
                    .enumerate()
                    .map(|(column, c)| (c, quad(column, row))),
            );
        }
        text
    }

    #[test]
    fn ignores_case_unless_asked() {
        let text = page(&["Privacy first. PRIVACY always."], false);
        assert_eq!(text.search("privacy", 100).hits.len(), 2);
        let exact = page(&["Privacy first. PRIVACY always."], true);
        assert_eq!(exact.search("privacy", 100).hits.len(), 0);
        assert_eq!(exact.search("PRIVACY", 100).hits.len(), 1);
    }

    #[test]
    fn finds_chinese_text() {
        let text = page(&["隱私優先的 PDF 閱讀器，保護隱私。"], false);
        let found = text.search("隱私", 100);
        assert_eq!(found.hits.len(), 2);
        assert_eq!(found.hits[0].quads[0].ul, Point { x: 0.0, y: 0.0 });
        assert_eq!(found.hits[1].quads[0].ul, Point { x: 160.0, y: 0.0 }); // 17th character
    }

    #[test]
    fn a_phrase_can_wrap_to_the_next_line() {
        let text = page(&["the needle is", "sharp"], false);
        let found = text.search("is  sharp", 100);
        assert_eq!(found.hits.len(), 1);
        // One quad per line: "is" at the end of line 0, "sharp" on line 1.
        let quads = &found.hits[0].quads;
        assert_eq!(quads.len(), 2);
        assert_eq!((quads[0].ul.x, quads[0].ur.x), (110.0, 130.0));
        assert_eq!(
            (quads[1].ul.x, quads[1].ur.x, quads[1].ul.y),
            (0.0, 50.0, 12.0)
        );
    }

    #[test]
    fn whitespace_and_line_breaks_in_the_text_count_as_one_space() {
        let text = page(&["needle\t\t 0500"], false);
        assert_eq!(text.search("needle 0500", 100).hits.len(), 1);
        assert_eq!(text.search(" needle   0500 ", 100).hits.len(), 1);
    }

    #[test]
    fn stops_at_the_hit_limit_and_does_not_overlap() {
        let text = page(&["aaaaaaa"], false);
        assert_eq!(text.search("aa", 100).hits.len(), 3);
        assert_eq!(text.search("aa", 2).hits.len(), 2);
        assert_eq!(text.search("aa", 0).hits.len(), 0);
    }

    #[test]
    fn kmp_handles_repetitive_patterns() {
        let text: Vec<char> = "abababcabababab".chars().collect();
        let pattern: Vec<char> = "ababab".chars().collect();
        assert_eq!(find(&text, &pattern, 10), [(0, 6), (7, 13)]);
    }

    #[test]
    fn empty_or_blank_queries_find_nothing() {
        let text = page(&["anything"], false);
        assert!(text.search("", 100).hits.is_empty());
        assert!(text.search("   ", 100).hits.is_empty());
    }

    #[test]
    fn a_page_without_characters_has_no_text() {
        assert!(!page(&[], false).search("x", 1).has_text);
        assert!(!page(&["   "], false).search("x", 1).has_text);
        assert!(page(&["x"], false).search("x", 1).has_text);
    }

    #[test]
    fn a_huge_page_is_searched_in_linear_time() {
        let line = "a".repeat(1_000_000);
        let text = page(&[&line], false);
        let query = format!("{}b", "a".repeat(1_000));
        let started = std::time::Instant::now();
        assert!(text.search(&query, 100).hits.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
