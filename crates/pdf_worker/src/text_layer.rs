//! A page's text for selecting and copying (MVP-15). Independent of MuPDF so it can be tested
//! directly; `engine` feeds it the characters of a page's text layer, line by line.
//!
//! Each line becomes one [`TextLine`]: its cleaned text, a quad covering the whole line, and
//! where each character starts along the line. That is enough for the frontend to hit-test and
//! highlight any run of characters, at a fraction of the size of a quad per character.

use ipc_contract::limits::MAX_PAGE_SIDE_PT;
use ipc_contract::text::copy_text_char;
use ipc_contract::types::{PageText, Point, Quad, TextLine};

/// Collects the lines of one page, up to a number of characters.
pub struct TextLayerBuilder {
    lines: Vec<TextLine>,
    chars: usize,
    max_chars: usize,
    truncated: bool,
}

impl TextLayerBuilder {
    pub fn new(max_chars: usize) -> Self {
        Self {
            lines: Vec::new(),
            chars: 0,
            max_chars,
            truncated: false,
        }
    }

    /// Whether the character limit was reached; further lines are ignored.
    pub fn is_full(&self) -> bool {
        self.truncated
    }

    /// Adds a line: its characters with their quads (page space), in reading order.
    /// Whitespace at either end is left out, and a line without any other character is skipped.
    pub fn push_line(&mut self, chars: impl IntoIterator<Item = (char, Quad)>) {
        if self.truncated {
            return;
        }
        let mut kept: Vec<(char, Quad)> = chars
            .into_iter()
            .filter(|(_, quad)| is_usable(quad))
            .filter_map(|(c, quad)| copy_text_char(c).map(|c| (c, quad)))
            .collect();
        let end = kept
            .iter()
            .rposition(|&(c, _)| c != ' ')
            .map_or(0, |i| i + 1);
        kept.truncate(end);
        let start = kept
            .iter()
            .position(|&(c, _)| c != ' ')
            .unwrap_or(kept.len());
        kept.drain(..start);
        if kept.is_empty() {
            return;
        }
        let room = self.max_chars - self.chars;
        if kept.len() > room {
            kept.truncate(room);
            self.truncated = true;
            if kept.is_empty() {
                return;
            }
        }
        if let Some(line) = line(&kept) {
            self.chars += kept.len();
            self.lines.push(line);
        }
    }

    pub fn finish(self) -> PageText {
        PageText {
            lines: self.lines,
            truncated: self.truncated,
        }
    }
}

/// Characters placed absurdly far from any page are left out, so that a line built from them
/// always stays within the contract's coordinate bounds.
const MAX_CHAR_COORDINATE: f32 = MAX_PAGE_SIDE_PT / 4.0;

fn is_usable(quad: &Quad) -> bool {
    [quad.ul, quad.ur, quad.ll, quad.lr].iter().all(|p| {
        p.x.is_finite()
            && p.y.is_finite()
            && p.x.abs() <= MAX_CHAR_COORDINATE
            && p.y.abs() <= MAX_CHAR_COORDINATE
    })
}

#[derive(Clone, Copy)]
struct Vector {
    x: f32,
    y: f32,
}

impl Vector {
    fn between(from: Point, to: Point) -> Self {
        Self {
            x: to.x - from.x,
            y: to.y - from.y,
        }
    }

    fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    fn dot(self, other: Vector) -> f32 {
        self.x * other.x + self.y * other.y
    }
}

fn center(quad: &Quad) -> Point {
    Point {
        x: (quad.ul.x + quad.ur.x + quad.ll.x + quad.lr.x) / 4.0,
        y: (quad.ul.y + quad.ur.y + quad.ll.y + quad.lr.y) / 4.0,
    }
}

/// Distances below this (in points) count as no direction at all.
const EPSILON: f32 = 1e-3;

/// The direction the line is written in, as a unit vector: from its first character to its
/// last (so vertical writing comes out vertical), or along a lone character's top edge.
fn writing_direction(chars: &[(char, Quad)]) -> Vector {
    let first = &chars[0].1;
    let last = &chars[chars.len() - 1].1;
    let across = Vector::between(center(first), center(last));
    let along_top = Vector::between(first.ul, first.ur);
    let direction = if across.length() > EPSILON {
        across
    } else if along_top.length() > EPSILON {
        along_top
    } else {
        Vector { x: 1.0, y: 0.0 }
    };
    let length = direction.length();
    Vector {
        x: direction.x / length,
        y: direction.y / length,
    }
}

/// Rounded to 1/100 point: plenty for hit-testing, and short in JSON.
fn round(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn line(chars: &[(char, Quad)]) -> Option<TextLine> {
    let along = writing_direction(chars);
    // A quarter turn clockwise in page space (y down): from the top of the line to its bottom.
    let down = Vector {
        x: -along.y,
        y: along.x,
    };
    let origin = chars[0].1.ul;
    let project = |point: Point, axis: Vector| Vector::between(origin, point).dot(axis);

    let mut edges = Vec::with_capacity(chars.len() + 1);
    let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut end = f32::NEG_INFINITY;
    for (_, quad) in chars {
        let corners = [quad.ul, quad.ur, quad.ll, quad.lr];
        let start = corners
            .iter()
            .map(|&p| project(p, along))
            .fold(f32::INFINITY, f32::min);
        end = corners
            .iter()
            .map(|&p| project(p, along))
            .fold(end, f32::max);
        for &p in &corners {
            let across = project(p, down);
            top = top.min(across);
            bottom = bottom.max(across);
        }
        // Characters never overlap in the edges: one that starts before the previous one (a
        // combining mark, overprinted text) gets no width of its own.
        let previous = edges.last().copied().unwrap_or(f32::NEG_INFINITY);
        edges.push(start.max(previous));
    }
    let first = edges[0];
    edges.push(end.max(edges[edges.len() - 1]));

    let at = |distance: f32, across: f32| Point {
        x: round(origin.x + along.x * distance + down.x * across),
        y: round(origin.y + along.y * distance + down.y * across),
    };
    let last = edges[edges.len() - 1];
    let quad = Quad {
        ul: at(first, top),
        ur: at(last, top),
        ll: at(first, bottom),
        lr: at(last, bottom),
    };
    let edges: Vec<f32> = edges.iter().map(|&edge| round(edge - first)).collect();
    let text: String = chars.iter().map(|&(c, _)| c).collect();
    let fits = [quad.ul, quad.ur, quad.ll, quad.lr]
        .iter()
        .all(|p| p.x.abs() <= MAX_PAGE_SIDE_PT && p.y.abs() <= MAX_PAGE_SIDE_PT)
        && edges.iter().all(|&edge| edge <= MAX_PAGE_SIDE_PT);
    fits.then_some(TextLine { text, quad, edges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_contract::validate::Validate;

    /// A character box `width` wide and 10 high, with its top left corner at (x, y).
    fn boxed(x: f32, y: f32, width: f32) -> Quad {
        let point = |x, y| Point { x, y };
        Quad {
            ul: point(x, y),
            ur: point(x + width, y),
            ll: point(x, y + 10.0),
            lr: point(x + width, y + 10.0),
        }
    }

    /// Characters of `text` side by side from (x, y), each `width` wide.
    fn row(text: &str, x: f32, y: f32, width: f32) -> Vec<(char, Quad)> {
        text.chars()
            .enumerate()
            .map(|(i, c)| (c, boxed(x + i as f32 * width, y, width)))
            .collect()
    }

    fn built(max_chars: usize, lines: Vec<Vec<(char, Quad)>>) -> PageText {
        let mut builder = TextLayerBuilder::new(max_chars);
        for line in lines {
            builder.push_line(line);
        }
        let text = builder.finish();
        text.validate().expect("valid page text");
        text
    }

    #[test]
    fn a_horizontal_line_has_its_box_and_character_edges() {
        let text = built(100, vec![row("Hi 中文", 72.0, 100.0, 5.0)]);
        assert!(!text.truncated);
        let [line] = &text.lines[..] else {
            panic!("one line")
        };
        assert_eq!(line.text, "Hi 中文");
        assert_eq!(line.quad, {
            let point = |x, y| Point { x, y };
            Quad {
                ul: point(72.0, 100.0),
                ur: point(97.0, 100.0),
                ll: point(72.0, 110.0),
                lr: point(97.0, 110.0),
            }
        });
        assert_eq!(line.edges, vec![0.0, 5.0, 10.0, 15.0, 20.0, 25.0]);
    }

    #[test]
    fn gaps_between_characters_belong_to_the_character_before() {
        // "a" is 4 wide but "b" starts 6 further on: the highlight of "a" reaches "b".
        let mut chars = row("a", 0.0, 0.0, 4.0);
        chars.extend(row("b", 6.0, 0.0, 4.0));
        let text = built(100, vec![chars]);
        assert_eq!(text.lines[0].edges, vec![0.0, 6.0, 10.0]);
    }

    #[test]
    fn vertical_writing_runs_down_the_page() {
        // Three characters stacked from top to bottom, as in vertical Chinese text.
        let chars: Vec<_> = "直書字"
            .chars()
            .enumerate()
            .map(|(i, c)| (c, boxed(200.0, 50.0 + i as f32 * 10.0, 10.0)))
            .collect();
        let text = built(100, vec![chars]);
        let line = &text.lines[0];
        assert_eq!(line.edges, vec![0.0, 10.0, 20.0, 30.0]);
        // Written downwards: ul to ur runs down, ul to ll runs across.
        assert_eq!((line.quad.ul.y, line.quad.ur.y), (50.0, 80.0));
        assert_eq!(line.quad.ul.x, line.quad.ur.x);
        assert_eq!((line.quad.ul.x - line.quad.ll.x).abs(), 10.0);
    }

    #[test]
    fn text_is_cleaned_and_trimmed_for_pasting() {
        let mut chars = row(" \tA", 0.0, 0.0, 5.0);
        // A right-to-left override and a zero-width space would change or hide what is pasted.
        chars.extend(row("\u{202E}B\u{200B}\nC  ", 15.0, 0.0, 5.0));
        let text = built(100, vec![chars]);
        let line = &text.lines[0];
        assert_eq!(line.text, "AB C");
        assert_eq!(line.edges.len(), 5);
        assert_eq!(line.quad.ul.x, 10.0);
    }

    #[test]
    fn empty_lines_and_unusable_characters_are_skipped() {
        let mut far = row("x", 0.0, 0.0, 5.0);
        far[0].1.ur.x = f32::NAN;
        let mut beyond = row("y", 0.0, 0.0, 5.0);
        beyond[0].1.ll.y = MAX_PAGE_SIDE_PT;
        let text = built(100, vec![row("   ", 0.0, 0.0, 5.0), far, beyond, vec![]]);
        assert!(text.lines.is_empty());
        assert!(!text.truncated);
    }

    #[test]
    fn overlapping_characters_never_make_edges_go_back() {
        // An accent drawn back over the letter before it.
        let mut chars = row("e", 0.0, 0.0, 5.0);
        chars.push(('\u{301}', boxed(1.0, 0.0, 3.0)));
        chars.extend(row("x", 5.0, 0.0, 5.0));
        let text = built(100, vec![chars]);
        assert_eq!(text.lines[0].edges, vec![0.0, 1.0, 5.0, 10.0]);
    }

    #[test]
    fn stops_at_the_character_limit() {
        let text = built(
            5,
            vec![
                row("abc", 0.0, 0.0, 5.0),
                row("defg", 0.0, 20.0, 5.0),
                row("hij", 0.0, 40.0, 5.0),
            ],
        );
        assert!(text.truncated);
        let lines: Vec<&str> = text.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(lines, ["abc", "de"]);
        assert_eq!(text.lines[1].edges, vec![0.0, 5.0, 10.0]);
    }
}
