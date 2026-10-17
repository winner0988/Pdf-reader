//! Setting the words of a text box in lines: which characters it sets, where a line is broken, and
//! the drawing operators of the lines. It needs no font, only the model's glyphs and their
//! widths, so a box can be set again at another size or in another box without the font file.

use std::collections::HashMap;

use ipc_contract::types::{TextAlign, TextBoxStyle};
use ipc_contract::worker::TextBoxModel;

/// The space between the words and the edge of the box, in points.
pub(super) const PADDING: f32 = 2.0;

/// A character, the glyph it is drawn with in the subset, and how far it moves the pen, in em.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub ch: char,
    pub cid: u16,
    pub advance: f32,
}

/// Characters the box does not set. A line of glyphs from the font's character map is right for
/// Latin, Greek, Cyrillic and the Chinese, Japanese and Korean letters (ADR 0018), and wrong for
/// what changes the shape or the place of a letter by its neighbours (Arabic and Hebrew, the
/// scripts of India and South-East Asia, marks that sit over a letter, Hangul jamo), and for the
/// controls that change the direction of the text. Those are refused, not drawn wrongly.
pub(super) fn needs_layout(ch: char) -> bool {
    matches!(u32::from(ch),
        0x0300..=0x036F        // combining marks
        | 0x0590..=0x109F      // Hebrew to Myanmar: Arabic, Syriac, India, Thai, Lao, Tibetan
        | 0x1100..=0x11FF      // Hangul jamo
        | 0x1780..=0x17FF      // Khmer
        | 0x200B..=0x200F      // zero-width characters and direction marks
        | 0x202A..=0x202E      // direction embeddings and overrides
        | 0x2060..=0x206F      // word joiner, invisible operators, direction isolates
        | 0x3099..=0x309A      // combining marks of kana
        | 0xFB1D..=0xFDFF      // Hebrew and Arabic presentation forms
        | 0xFE00..=0xFE0F      // variation selectors
        | 0xFE20..=0xFE2F      // combining half marks
        | 0xFE70..=0xFEFF      // Arabic presentation forms, byte order mark
    )
}

/// Marks that do not start a line (a closing bracket, a full stop: Chinese has no spaces to break
/// at, and a line must not begin with one) and brackets that do not end one.
fn closes(ch: char) -> bool {
    "，。、．：；！？）］｝〕〉》」』】〗”’…".contains(ch)
        || matches!(ch, ',' | '.' | ')' | ']' | '}')
}

fn opens(ch: char) -> bool {
    "（［｛〔〈《「『【〖“‘".contains(ch) || matches!(ch, '(' | '[' | '{')
}

/// The glyphs of the text of `model`, a line break being a glyph of no width. `None` if a
/// character has none in the model.
pub(super) fn glyphs(model: &TextBoxModel) -> Option<Vec<Glyph>> {
    let known: HashMap<char, (u16, u16)> = model
        .glyphs
        .iter()
        .map(|glyph| (glyph.ch, (glyph.cid, glyph.advance)))
        .collect();
    model
        .text
        .chars()
        .map(|ch| {
            if ch == '\n' {
                return Some(Glyph {
                    ch,
                    cid: 0,
                    advance: 0.0,
                });
            }
            let &(cid, advance) = known.get(&ch)?;
            Some(Glyph {
                ch,
                cid,
                advance: f32::from(advance) / 1000.0,
            })
        })
        .collect()
}

/// Breaks `glyphs` into lines no wider than `width` points at `size`: at the last space of the
/// line when there is one, else before the glyph that does not fit (Chinese has no spaces), and
/// not so that a line starts with a closing mark or ends with an opening one. A line broken at a
/// space does not keep it, a space that does not fit stays at the end of its line, and a line
/// holds at least one glyph, however wide.
pub(super) fn wrap(glyphs: &[Glyph], size: f32, width: f32) -> Vec<Vec<Glyph>> {
    let mut lines: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut used = 0.0;
    for &glyph in glyphs {
        if glyph.ch == '\n' {
            lines.push(Vec::new());
            used = 0.0;
            continue;
        }
        let step = glyph.advance * size;
        let line = lines.last_mut().expect("a line");
        if used + step > width && !line.is_empty() && glyph.ch != ' ' {
            let first = line.iter().position(|g| g.ch != ' ').unwrap_or(line.len());
            let at = match line.iter().rposition(|g| g.ch == ' ') {
                Some(space) if space > first => space,
                _ => {
                    let mut at = line.len();
                    while at > 1 {
                        let next = line.get(at).map_or(glyph.ch, |g| g.ch);
                        if !closes(next) && !opens(line[at - 1].ch) {
                            break;
                        }
                        at -= 1;
                    }
                    at
                }
            };
            let mut rest = line.split_off(at);
            if rest.first().is_some_and(|g| g.ch == ' ') {
                rest.remove(0);
            }
            used = rest.iter().map(|g| g.advance * size).sum();
            lines.push(rest);
        }
        let line = lines.last_mut().expect("a line");
        line.push(glyph);
        used += step;
    }
    // A space at the end of a line has no ink, and would move a centred or right-aligned line.
    for line in &mut lines {
        while line.last().is_some_and(|g| g.ch == ' ') {
            line.pop();
        }
    }
    lines
}

/// The words of a box set in lines.
pub(super) struct Laid {
    /// Where the baseline of each line (that has glyphs) starts, in the box's own space (points,
    /// from its bottom left), and its glyphs.
    pub lines: Vec<(f32, f32, Vec<Glyph>)>,
    /// The drawing operators of the lines: the content of the appearance.
    pub content: String,
    /// The lines go below the box.
    pub overflow: bool,
}

/// `model` set in a box of `width` x `height` points.
pub(super) fn lay_out(
    model: &TextBoxModel,
    style: &TextBoxStyle,
    (width, height): (f32, f32),
) -> Option<Laid> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let glyphs = glyphs(model)?;
    let wrapped = wrap(&glyphs, style.size, width - 2.0 * PADDING);
    let ascent = (model.ascent as f32 / 1000.0).max(0.8) * style.size;
    let step = model.step as f32 / 1000.0 * style.size;
    let [r, g, b] = [style.color.r, style.color.g, style.color.b].map(|c| f32::from(c) / 255.0);
    let mut content = format!("BT\n/F0 {} Tf\n{r:.4} {g:.4} {b:.4} rg\n", style.size);
    let mut lines = Vec::new();
    for (index, line) in wrapped.iter().enumerate() {
        let y = height - PADDING - ascent - index as f32 * step;
        if line.is_empty() {
            continue;
        }
        let used: f32 = line.iter().map(|glyph| glyph.advance * style.size).sum();
        let x = match style.align {
            TextAlign::Left => PADDING,
            TextAlign::Center => (width - used) / 2.0,
            TextAlign::Right => width - PADDING - used,
        };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let hex: String = line
            .iter()
            .map(|glyph| format!("{:04X}", glyph.cid))
            .collect();
        content.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"));
        lines.push((x, y, line.clone()));
    }
    content.push_str("ET\n");
    let bottom = PADDING + ascent + (wrapped.len() - 1) as f32 * step;
    Some(Laid {
        lines,
        content,
        overflow: bottom > height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph one em wide, so that at size 1 a width is a number of glyphs.
    fn glyphs_of(text: &str) -> Vec<Glyph> {
        text.chars()
            .map(|ch| Glyph {
                ch,
                cid: 1,
                advance: 1.0,
            })
            .collect()
    }

    fn lines(text: &str, width: f32) -> Vec<String> {
        wrap(&glyphs_of(text), 1.0, width)
            .iter()
            .map(|line| line.iter().map(|g| g.ch).collect())
            .collect()
    }

    #[test]
    fn english_is_broken_at_the_last_space_that_fits_and_the_space_is_not_kept() {
        assert_eq!(lines("aa bb cc", 5.0), ["aa bb", "cc"]);
        assert_eq!(lines("aa bb cc", 4.0), ["aa", "bb", "cc"]);
        assert_eq!(lines("ab cd", 2.0), ["ab", "cd"]);
        assert_eq!(lines("ab  cd", 2.0), ["ab", "cd"]);
        assert_eq!(lines("ab   ", 9.0), ["ab"]);
    }

    #[test]
    fn a_word_longer_than_the_line_and_chinese_are_broken_before_the_glyph_that_does_not_fit() {
        assert_eq!(lines("abcdefgh", 3.0), ["abc", "def", "gh"]);
        assert_eq!(lines("ab abcdefgh", 3.0), ["ab", "abc", "def", "gh"]);
        assert_eq!(lines("一二三四五六七", 3.0), ["一二三", "四五六", "七"]);
    }

    #[test]
    fn a_line_does_not_start_with_a_closing_mark_or_end_with_an_opening_one() {
        assert_eq!(lines("一二三。四", 3.0), ["一二", "三。四"]);
        assert_eq!(lines("一二三。。四", 3.0), ["一二", "三。。", "四"]);
        assert_eq!(lines("一二三（四", 3.0), ["一二三", "（四"]);
        assert_eq!(lines("一二（三四", 3.0), ["一二", "（三四"]);
        // When it cannot be done, the line keeps its one glyph.
        assert_eq!(lines("。。。", 1.0), ["。", "。", "。"]);
    }

    #[test]
    fn newlines_make_lines_and_nothing_makes_one_empty_line() {
        assert_eq!(lines("a\n\nb", 9.0), ["a", "", "b"]);
        assert_eq!(lines("", 9.0), [""]);
        assert_eq!(lines("  abcdef", 3.0), ["  a", "bcd", "ef"]);
    }

    #[test]
    fn a_glyph_wider_than_the_line_still_gets_a_line_of_its_own() {
        assert_eq!(lines("abc", 0.5), ["a", "b", "c"]);
    }

    #[test]
    fn the_width_of_the_lines_is_the_sum_of_the_advances_at_the_size() {
        let narrow: Vec<Glyph> = "ab cd ef"
            .chars()
            .map(|ch| Glyph {
                ch,
                cid: 1,
                advance: if ch == ' ' { 0.25 } else { 0.5 },
            })
            .collect();
        // At 10 points a letter is 5 wide and a space 2.5: "ab" is 10, "ab c" is 17.5 and fits in
        // 18, "ab cd" is 22.5 and does not.
        let broken: Vec<String> = wrap(&narrow, 10.0, 18.0)
            .iter()
            .map(|line| line.iter().map(|g| g.ch).collect())
            .collect();
        assert_eq!(broken, ["ab", "cd", "ef"]);
    }

    #[test]
    fn scripts_that_need_layout_are_told_from_those_that_do_not() {
        for needs in [
            '\u{628}', '\u{5d0}', '\u{915}', '\u{e01}', '\u{301}', '\u{202e}', '\u{200d}',
            '\u{1100}', '\u{fb50}', '\u{fe0f}', '\u{feff}', '\u{3099}',
        ] {
            assert!(needs_layout(needs), "{needs:?}");
        }
        for fine in [
            'a', 'Z', '7', ' ', 'é', 'Ω', 'Ж', '你', '體', 'か', 'カ', '한', '。', '€', '¥',
        ] {
            assert!(!needs_layout(fine), "{fine:?}");
        }
    }

    #[test]
    fn lay_out_rejects_non_finite_and_non_positive_dimensions() {
        use ipc_contract::types::Rgb;
        use ipc_contract::worker::ModelGlyph;

        let model = TextBoxModel {
            text: "Hello".to_string(),
            font: "Test".to_string(),
            face: 0,
            view_only: false,
            base: "ABCDEF+Test".to_string(),
            ascent: 800,
            descent: -200,
            cap_height: 700,
            step: 1000,
            bbox: [0, -200, 1000, 800],
            glyphs: vec![
                ModelGlyph {
                    ch: 'H',
                    cid: 1,
                    advance: 600,
                },
                ModelGlyph {
                    ch: 'e',
                    cid: 2,
                    advance: 500,
                },
                ModelGlyph {
                    ch: 'l',
                    cid: 3,
                    advance: 300,
                },
                ModelGlyph {
                    ch: 'o',
                    cid: 4,
                    advance: 500,
                },
            ],
            subset: vec![1, 2, 3, 4],
        };
        let style = TextBoxStyle {
            size: 12.0,
            color: Rgb { r: 0, g: 0, b: 0 },
            align: TextAlign::Left,
        };

        for (w, h) in [
            (f32::NAN, 100.0),
            (100.0, f32::NAN),
            (-50.0, 100.0),
            (100.0, -20.0),
            (0.0, 100.0),
            (100.0, 0.0),
            (f32::INFINITY, 100.0),
            (100.0, f32::NEG_INFINITY),
        ] {
            assert!(
                lay_out(&model, &style, (w, h)).is_none(),
                "should reject ({w}, {h})"
            );
        }
    }
}
