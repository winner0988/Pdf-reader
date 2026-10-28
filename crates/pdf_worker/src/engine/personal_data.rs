//! Finding personal data in document text layers for redaction (Issue #302).
//!
//! Scans a document's text (including OCR text layers) in memory inside `pdf_worker`
//! for common sensitive patterns:
//! - Taiwan National ID & New Alien Resident Certificate (ARC) numbers
//! - Mobile phone numbers (09xx)
//! - Landline phone numbers with area codes
//! - Email addresses
//! - Credit card numbers passing the Luhn algorithm
//! - Taiwan Unified Business Numbers (8-digit tax IDs, supporting both old and new rules)
//! - User-specified custom keywords
//!
//! # Security & In-Memory Only
//! All scanning occurs strictly within the worker process memory. Detected strings
//! are NEVER logged, never recorded in crash recovery journals, and never returned in
//! full across the IPC boundary. Previews returned to the frontend have sensitive
//! middle characters masked (e.g. `A12****789`). Marking a match for redaction produces
//! standard `Edit::AddRedactionGroup` edits carrying bounding rectangles only.

use ipc_contract::limits::{
    MAX_PERSONAL_DATA_MATCHES, MAX_PERSONAL_DATA_PREVIEW_BYTES, MAX_REDACTION_GROUP_RECTS,
    MIN_ANNOTATION_SIDE_PT,
};
use ipc_contract::types::{
    FindPersonalDataQuery, PersonalDataCategory, PersonalDataMatch, PersonalDataMatches, Quad, Rect,
};

/// Longest address `match_email` accepts (RFC 5321 path limit).
const MAX_EMAIL_CHARS: usize = 254;

/// Longest email domain shown unmasked in a preview; longer domains are masked so a
/// preview always fits `MAX_PERSONAL_DATA_PREVIEW_BYTES`.
const MAX_PREVIEW_DOMAIN_CHARS: usize = 64;

/// Character with its page geometry and line index.
#[derive(Clone, Copy, Debug)]
pub struct PageChar {
    pub orig: char,
    pub norm: char,
    pub quad: Option<Quad>,
    pub line_idx: u32,
}

/// Normalizes a character: converts full-width digits, letters, hyphens, parentheses,
/// spaces, and email symbols to their ASCII half-width equivalents.
pub fn normalize_char(c: char) -> char {
    match c {
        // Full-width ASCII digits (U+FF10 - U+FF19)
        '０'..='９' => (b'0' + (c as u32 - '０' as u32) as u8) as char,
        // Full-width Latin capital letters (U+FF21 - U+FF3A)
        'Ａ'..='Ｚ' => (b'A' + (c as u32 - 'Ａ' as u32) as u8) as char,
        // Full-width Latin small letters (U+FF41 - U+FF5A)
        'ａ'..='ｚ' => (b'a' + (c as u32 - 'ａ' as u32) as u8) as char,
        // Full-width and typographic dashes / hyphens
        '－' | '—' | '–' | '─' | 'ｰ' => '-',
        // Full-width parentheses
        '（' => '(',
        '）' => ')',
        // Full-width space (U+3000)
        '　' => ' ',
        // Full-width @ and dot
        '＠' => '@',
        '．' => '.',
        _ => c,
    }
}

/// Checks whether `s` is a valid Taiwan National ID or New Alien Resident Certificate (ARC) number.
///
/// Format: 1 uppercase letter followed by 9 digits.
/// The second character (first digit) must be:
/// - `1` (male citizen) or `2` (female citizen)
/// - `8` (male foreigner, new ARC) or `9` (female foreigner, new ARC)
///
/// Check digit calculation uses the standard weighted checksum mod 10.
pub fn is_valid_taiwan_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    let first = bytes[0] as char;
    let letter_val: u32 = match first {
        'A' => 10,
        'B' => 11,
        'C' => 12,
        'D' => 13,
        'E' => 14,
        'F' => 15,
        'G' => 16,
        'H' => 17,
        'I' => 34,
        'J' => 18,
        'K' => 19,
        'L' => 20,
        'M' => 21,
        'N' => 22,
        'O' => 35,
        'P' => 23,
        'Q' => 24,
        'R' => 25,
        'S' => 26,
        'T' => 27,
        'U' => 28,
        'V' => 29,
        'W' => 32,
        'X' => 30,
        'Y' => 31,
        'Z' => 33,
        _ => return false,
    };
    let d1 = bytes[1];
    if d1 != b'1' && d1 != b'2' && d1 != b'8' && d1 != b'9' {
        return false;
    }
    let x1 = letter_val / 10;
    let x2 = letter_val % 10;
    let mut sum = x1 + x2 * 9;
    for (i, &b) in bytes[1..9].iter().enumerate() {
        if !b.is_ascii_digit() {
            return false;
        }
        let d = (b - b'0') as u32;
        let weight = (8 - i) as u32;
        sum += d * weight;
    }
    if !bytes[9].is_ascii_digit() {
        return false;
    }
    let d9 = (bytes[9] - b'0') as u32;
    sum += d9;
    sum.is_multiple_of(10)
}

/// Checks whether `digits` passes the Luhn (mod 10) algorithm for credit card numbers.
///
/// Length must be between 13 and 19 digits.
pub fn is_valid_luhn(digits: &str) -> bool {
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let mut sum: u32 = 0;
    let mut double = false;
    for c in digits.chars().rev() {
        let d = match c.to_digit(10) {
            Some(d) => d,
            None => return false,
        };
        if double {
            let doubled = d * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d;
        }
        double = !double;
    }
    sum.is_multiple_of(10)
}

/// Checks whether `s` is an 8-digit Taiwan Unified Business Number (UBN) under the old rule
/// (divisible by 10).
pub fn is_valid_ubn_old(s: &str) -> bool {
    validate_ubn(s, 10)
}

/// Checks whether `s` is an 8-digit Taiwan Unified Business Number (UBN) under the new rule
/// (divisible by 5, effective April 1, 2023).
pub fn is_valid_ubn_new(s: &str) -> bool {
    validate_ubn(s, 5)
}

/// Checks whether `s` is an 8-digit Taiwan Unified Business Number (valid under either new or old rule).
pub fn is_valid_ubn(s: &str) -> bool {
    is_valid_ubn_new(s) || is_valid_ubn_old(s)
}

fn validate_ubn(s: &str, divisor: u32) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 8 {
        return false;
    }
    const WEIGHTS: [u32; 8] = [1, 2, 1, 2, 1, 2, 4, 1];
    let mut other_sum = 0;
    for i in 0..8 {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        if i == 6 {
            continue;
        }
        let d = (bytes[i] - b'0') as u32;
        let p = d * WEIGHTS[i];
        other_sum += (p / 10) + (p % 10);
    }
    let d7 = (bytes[6] - b'0') as u32;
    if d7 == 7 {
        // 7 * 4 = 28 -> 2 + 8 = 10 -> either 1 or 0
        (other_sum + 1).is_multiple_of(divisor) || other_sum.is_multiple_of(divisor)
    } else {
        let p7 = d7 * 4;
        let s7 = (p7 / 10) + (p7 % 10);
        (other_sum + s7).is_multiple_of(divisor)
    }
}

/// Checks whether `digits` is a valid Taiwan mobile phone number (10 digits starting with `09`).
pub fn is_valid_mobile(digits: &str) -> bool {
    digits.len() == 10 && digits.starts_with("09") && digits.chars().all(|c| c.is_ascii_digit())
}

/// Checks whether a sequence of digits and its raw text represents a Taiwan landline phone number.
///
/// Area codes:
/// - 2 digits: 02, 03, 04, 05, 06, 07, 08 (local 7 or 8 digits, total 9 or 10)
/// - 3 digits: 037, 049, 082, 089 (local 6 or 7 digits, total 9 or 10)
/// - 4 digits: 0826, 0836 (local 5 or 6 digits, total 8, 9, or 10)
///
/// To prevent false positives on arbitrary numbers, `raw_text` must contain an area code
/// delimiter such as parentheses `(02)` or a hyphen `02-` or space `02 `.
pub fn is_valid_landline(raw_text: &str, digits: &str) -> bool {
    if !digits.starts_with('0') || digits.starts_with("09") {
        return false;
    }
    let has_delim = raw_text.contains('(') || raw_text.contains('-') || raw_text.contains(' ');
    if !has_delim {
        return false;
    }
    // Whole lengths with the area code: Taipei (02) and Nantou (049) have 8- and 7-digit local
    // numbers, Taichung and Changhua (04) both 8 and 7, and every other area 9 in all.
    let d_len = digits.len();
    if digits.starts_with("02") || digits.starts_with("049") {
        return d_len == 10;
    }
    if digits.starts_with("04") {
        return d_len == 9 || d_len == 10;
    }
    if ["03", "05", "06", "07", "08"]
        .iter()
        .any(|code| digits.starts_with(code))
    {
        return d_len == 9;
    }
    false
}

/// Checks whether `s` is a valid email address.
pub fn is_valid_email(s: &str) -> bool {
    let parts: Vec<&str> = s.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    for c in local.chars() {
        if !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '%' && c != '+' && c != '-' {
            return false;
        }
    }
    let domain_labels: Vec<&str> = domain.split('.').collect();
    if domain_labels.len() < 2 {
        return false;
    }
    for (i, label) in domain_labels.iter().enumerate() {
        if label.is_empty() || label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        for c in label.chars() {
            if !c.is_ascii_alphanumeric() && c != '-' {
                return false;
            }
        }
        // TLD (last label) must be at least 2 ASCII letters
        if i == domain_labels.len() - 1
            && (label.len() < 2 || !label.chars().all(|c| c.is_ascii_alphabetic()))
        {
            return false;
        }
    }
    true
}

/// Produces a masked preview string for IPC response (e.g. `A12****789`).
///
/// Full unmasked strings NEVER leave the worker process.
pub fn mask_preview(category: PersonalDataCategory, original: &str) -> String {
    let s: String = original
        .chars()
        .map(normalize_char)
        .filter(|c| !c.is_whitespace())
        .collect();
    let preview = mask_by_category(category, &s);
    // The host rejects a whole response whose preview is too long, so fall back to the
    // short generic mask rather than ever sending one.
    if preview.len() > MAX_PERSONAL_DATA_PREVIEW_BYTES {
        mask_generic(&s)
    } else {
        preview
    }
}

fn mask_by_category(category: PersonalDataCategory, s: &str) -> String {
    match category {
        PersonalDataCategory::TaiwanId => {
            let chars: Vec<char> = s.chars().collect();
            if chars.len() == 10 {
                let first3: String = chars[..3].iter().collect();
                let last3: String = chars[7..].iter().collect();
                format!("{first3}****{last3}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::MobilePhone => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() == 10 {
                let first4 = &digits[..4];
                let last2 = &digits[8..];
                format!("{first4}****{last2}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::LandlinePhone => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 8 {
                let prefix_len = if digits.starts_with("0826") || digits.starts_with("0836") {
                    4
                } else if digits.starts_with("037")
                    || digits.starts_with("049")
                    || digits.starts_with("082")
                    || digits.starts_with("089")
                {
                    3
                } else {
                    2
                };
                let prefix = &digits[..prefix_len];
                let last2 = &digits[digits.len() - 2..];
                format!("({prefix})****{last2}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::CreditCard => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 13 {
                let first4 = &digits[..4];
                let last4 = &digits[digits.len() - 4..];
                format!("{first4} **** **** {last4}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::UnifiedBusinessNumber => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() == 8 {
                let first2 = &digits[..2];
                let last2 = &digits[6..];
                format!("{first2}****{last2}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::Email => {
            if let Some((local, domain)) = s.split_once('@') {
                let masked_local = if local.len() <= 2 {
                    format!("{}*", &local[..1.min(local.len())])
                } else {
                    let first = &local[..1];
                    let last = &local[local.len() - 1..];
                    format!("{first}***{last}")
                };
                let shown_domain = if domain.len() <= MAX_PREVIEW_DOMAIN_CHARS {
                    domain.to_string()
                } else {
                    mask_generic(domain)
                };
                format!("{masked_local}@{shown_domain}")
            } else {
                mask_generic(s)
            }
        }
        PersonalDataCategory::Keyword => mask_generic(s),
    }
}

fn mask_generic(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 2 {
        if chars.is_empty() {
            String::new()
        } else {
            format!("{}*", chars[0])
        }
    } else {
        let first: String = chars[..1].iter().collect();
        let last: String = chars[chars.len() - 1..].iter().collect();
        format!("{first}****{last}")
    }
}

/// Computes bounding rectangles for a matched slice of `PageChar`s, grouped by line.
fn rects_of_chars(slice: &[PageChar]) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut current_line = None;
    let mut line_quads: Vec<Quad> = Vec::new();

    let flush_line = |line_quads: &[Quad], rects: &mut Vec<Rect>| {
        if line_quads.is_empty() {
            return;
        }
        let mut x0 = f32::INFINITY;
        let mut x1 = f32::NEG_INFINITY;
        let mut y0 = f32::INFINITY;
        let mut y1 = f32::NEG_INFINITY;
        for q in line_quads {
            x0 = x0.min(q.ul.x).min(q.ur.x).min(q.ll.x).min(q.lr.x);
            x1 = x1.max(q.ul.x).max(q.ur.x).max(q.ll.x).max(q.lr.x);
            y0 = y0.min(q.ul.y).min(q.ur.y).min(q.ll.y).min(q.lr.y);
            y1 = y1.max(q.ul.y).max(q.ur.y).max(q.ll.y).max(q.lr.y);
        }
        if x1 - x0 < MIN_ANNOTATION_SIDE_PT {
            let mid = (x0 + x1) / 2.0;
            x0 = mid - MIN_ANNOTATION_SIDE_PT / 2.0;
            x1 = mid + MIN_ANNOTATION_SIDE_PT / 2.0;
        }
        if y1 - y0 < MIN_ANNOTATION_SIDE_PT {
            let mid = (y0 + y1) / 2.0;
            y0 = mid - MIN_ANNOTATION_SIDE_PT / 2.0;
            y1 = mid + MIN_ANNOTATION_SIDE_PT / 2.0;
        }
        rects.push(Rect {
            x0: (x0 * 100.0).round() / 100.0,
            y0: (y0 * 100.0).round() / 100.0,
            x1: (x1 * 100.0).round() / 100.0,
            y1: (y1 * 100.0).round() / 100.0,
        });
    };

    for ch in slice {
        if let Some(q) = ch.quad {
            if current_line == Some(ch.line_idx) {
                line_quads.push(q);
            } else {
                flush_line(&line_quads, &mut rects);
                line_quads.clear();
                line_quads.push(q);
                current_line = Some(ch.line_idx);
            }
        }
    }
    flush_line(&line_quads, &mut rects);
    cap_rects(rects)
}

/// Keeps a match within `MAX_REDACTION_GROUP_RECTS` rectangles (a keyword can wrap over many
/// short lines) by joining neighbouring line rectangles. Joined rectangles cover at
/// least the same area, so nothing in the match is left out of the redaction.
fn cap_rects(rects: Vec<Rect>) -> Vec<Rect> {
    let max = MAX_REDACTION_GROUP_RECTS;
    if rects.len() <= max {
        return rects;
    }
    let group = rects.len().div_ceil(max);
    rects
        .chunks(group)
        .map(|chunk| Rect {
            x0: chunk.iter().map(|r| r.x0).fold(f32::INFINITY, f32::min),
            y0: chunk.iter().map(|r| r.y0).fold(f32::INFINITY, f32::min),
            x1: chunk.iter().map(|r| r.x1).fold(f32::NEG_INFINITY, f32::max),
            y1: chunk.iter().map(|r| r.y1).fold(f32::NEG_INFINITY, f32::max),
        })
        .collect()
}

/// The order categories are tried in at each position: the first that matches wins.
const SCAN_ORDER: [PersonalDataCategory; 7] = [
    PersonalDataCategory::TaiwanId,
    PersonalDataCategory::MobilePhone,
    PersonalDataCategory::LandlinePhone,
    PersonalDataCategory::CreditCard,
    PersonalDataCategory::UnifiedBusinessNumber,
    PersonalDataCategory::Email,
    PersonalDataCategory::Keyword,
];

/// Scans a single page's characters for the requested personal data categories.
pub fn scan_page(
    page_idx: u32,
    chars: &[PageChar],
    query: &FindPersonalDataQuery,
    matches: &mut Vec<PersonalDataMatch>,
    max_matches: usize,
) -> bool {
    let mut i = 0;
    while i < chars.len() {
        if matches.len() >= max_matches {
            return true;
        }
        let found = SCAN_ORDER
            .into_iter()
            .filter(|category| query.categories.contains(category))
            .find_map(|category| {
                let end = match_category(category, chars, i, query)?;
                let rects = rects_of_chars(&chars[i..end]);
                (!rects.is_empty()).then_some((category, end, rects))
            });
        if let Some((category, end, rects)) = found {
            let orig_str: String = chars[i..end].iter().map(|c| c.orig).collect();
            matches.push(PersonalDataMatch {
                category,
                page: page_idx,
                rects,
                preview: mask_preview(category, &orig_str),
            });
            i = end;
        } else {
            i += 1;
        }
    }
    matches.len() >= max_matches
}

/// Where a match of `category` starting at `i` ends, if one does: each category may only start
/// where a word or number does.
fn match_category(
    category: PersonalDataCategory,
    chars: &[PageChar],
    i: usize,
    query: &FindPersonalDataQuery,
) -> Option<usize> {
    let c = chars[i].norm;
    let before = i.checked_sub(1).map(|j| chars[j].norm);
    let after_digit_or_plus = before.is_some_and(|b| b.is_ascii_digit() || b == '+');
    match category {
        PersonalDataCategory::TaiwanId => {
            let starts =
                c.is_ascii_alphabetic() && !before.is_some_and(|b| b.is_ascii_alphanumeric());
            if starts {
                match_taiwan_id(chars, i)
            } else {
                None
            }
        }
        PersonalDataCategory::MobilePhone => {
            let starts = c == '0'
                && chars.get(i + 1).is_some_and(|next| next.norm == '9')
                && !after_digit_or_plus
                && !number_goes_on_before(chars, i);
            if starts { match_mobile(chars, i) } else { None }
        }
        PersonalDataCategory::LandlinePhone => {
            let starts =
                (c == '(' || c == '0') && !after_digit_or_plus && !number_goes_on_before(chars, i);
            if starts {
                match_landline(chars, i)
            } else {
                None
            }
        }
        PersonalDataCategory::CreditCard => {
            let starts = c.is_ascii_digit() && !number_goes_on_before(chars, i);
            if starts {
                match_credit_card(chars, i)
            } else {
                None
            }
        }
        PersonalDataCategory::UnifiedBusinessNumber => {
            let starts = c.is_ascii_digit()
                && !before.is_some_and(|b| b.is_ascii_alphanumeric())
                && !number_goes_on_before(chars, i);
            if starts { match_ubn(chars, i) } else { None }
        }
        PersonalDataCategory::Email => {
            let starts = (c.is_ascii_alphanumeric() || c == '_' || c == '+')
                && !before.is_some_and(is_email_char);
            if starts { match_email(chars, i) } else { None }
        }
        PersonalDataCategory::Keyword => query
            .keyword
            .as_deref()
            .filter(|keyword| !keyword.trim().is_empty())
            .and_then(|keyword| match_keyword(chars, i, keyword)),
    }
}

impl super::PdfDocument {
    /// Scans document text layers for personal data matching `query` (Issue #302).
    pub fn find_personal_data(
        &self,
        query: &FindPersonalDataQuery,
    ) -> Result<PersonalDataMatches, super::EngineError> {
        let page_count = self.page_count()?;
        let mut matches = Vec::new();
        let mut has_more = false;

        for page_idx in 0..page_count {
            if matches.len() >= MAX_PERSONAL_DATA_MATCHES as usize {
                has_more = true;
                break;
            }

            let text_page = self
                .load_page(page_idx)?
                .to_text_page(mupdf::TextPageFlags::empty())?;
            let mut page_chars: Vec<PageChar> = Vec::new();
            let mut current_line_idx = 0u32;

            for block in text_page.blocks() {
                for line in block.lines() {
                    push_line_break(&mut page_chars, current_line_idx);
                    current_line_idx += 1;
                    for ch in line.chars() {
                        if let Some(c) = ch.char() {
                            let quad = super::contract_quad(ch.quad());
                            page_chars.push(PageChar {
                                orig: c,
                                norm: normalize_char(c),
                                quad: Some(quad),
                                line_idx: current_line_idx,
                            });
                        }
                    }
                }
            }

            // Fallback to OCR text if page has no native text
            if page_chars.is_empty()
                && let Some(recognised) = self.ocr_text(page_idx)
            {
                for line in &recognised.lines {
                    push_line_break(&mut page_chars, current_line_idx);
                    current_line_idx += 1;
                    for (c, quad) in super::ocr::char_quads(line) {
                        page_chars.push(PageChar {
                            orig: c,
                            norm: normalize_char(c),
                            quad: Some(quad),
                            line_idx: current_line_idx,
                        });
                    }
                }
            }

            // Include text boxes
            for line in self.text_box_lines(page_idx) {
                push_line_break(&mut page_chars, current_line_idx);
                current_line_idx += 1;
                for (c, quad) in line {
                    page_chars.push(PageChar {
                        orig: c,
                        norm: normalize_char(c),
                        quad: Some(quad),
                        line_idx: current_line_idx,
                    });
                }
            }

            if scan_page(
                page_idx,
                &page_chars,
                query,
                &mut matches,
                MAX_PERSONAL_DATA_MATCHES as usize,
            ) {
                has_more = true;
                break;
            }
        }

        Ok(PersonalDataMatches { matches, has_more })
    }
}

/// Ends a line of `chars` so that matches can go on over it but words on two lines do not run
/// together; nothing to do when the line already ended in a space.
fn push_line_break(chars: &mut Vec<PageChar>, line_idx: u32) {
    if chars
        .last()
        .is_some_and(|last| last.norm != ' ' && last.norm != '\n')
    {
        chars.push(PageChar {
            orig: '\n',
            norm: '\n',
            quad: None,
            line_idx,
        });
    }
}

fn match_taiwan_id(chars: &[PageChar], start: usize) -> Option<usize> {
    let mut normalized = String::with_capacity(10);
    normalized.push(chars[start].norm.to_ascii_uppercase());
    let mut j = start + 1;
    let mut digits_found = 0;
    while j < chars.len() && digits_found < 9 {
        let c = chars[j].norm;
        if c.is_ascii_digit() {
            normalized.push(c);
            digits_found += 1;
            j += 1;
        } else if (c == ' ' || c == '-' || c == '\n') && digits_found == 0 {
            // Allow wrap or separator immediately after letter
            j += 1;
        } else if (c == ' ' || c == '\n') && digits_found < 9 {
            // Allow wrapping inside digits
            j += 1;
        } else {
            break;
        }
    }
    if digits_found == 9 {
        // Boundary check after
        if j < chars.len() && chars[j].norm.is_ascii_alphanumeric() {
            return None;
        }
        if is_valid_taiwan_id(&normalized) {
            return Some(j);
        }
    }
    None
}

fn match_mobile(chars: &[PageChar], start: usize) -> Option<usize> {
    let mut digits = String::with_capacity(10);
    let mut j = start;
    while j < chars.len() && digits.len() < 10 {
        let c = chars[j].norm;
        if c.is_ascii_digit() {
            digits.push(c);
            j += 1;
        } else if c == '-' || c == ' ' || c == '\n' {
            j += 1;
        } else {
            break;
        }
    }
    if digits.len() == 10 && is_valid_mobile(&digits) {
        if number_continues_at(chars, j) {
            return None;
        }
        return Some(j);
    }
    None
}

fn match_landline(chars: &[PageChar], start: usize) -> Option<usize> {
    let mut digits = String::with_capacity(10);
    let mut raw = String::new();
    let mut j = start;
    while j < chars.len() && digits.len() < 10 {
        let c = chars[j].norm;
        if c.is_ascii_digit() {
            digits.push(c);
            raw.push(c);
            j += 1;
        } else if c == '(' || c == ')' || c == '-' || c == ' ' || c == '\n' {
            raw.push(c);
            j += 1;
        } else {
            break;
        }
    }
    if is_valid_landline(&raw, &digits) {
        if number_continues_at(chars, j) {
            return None;
        }
        return Some(j);
    }
    None
}

fn match_credit_card(chars: &[PageChar], start: usize) -> Option<usize> {
    let mut digits = String::with_capacity(19);
    let mut j = start;
    while j < chars.len() && digits.len() < 19 {
        let c = chars[j].norm;
        if c.is_ascii_digit() {
            digits.push(c);
            j += 1;
        } else if c == '-' || c == ' ' || c == '\n' {
            j += 1;
        } else {
            break;
        }
    }
    if digits.len() >= 13 && is_valid_luhn(&digits) {
        if number_continues_at(chars, j) {
            return None;
        }
        return Some(j);
    }
    None
}

/// A Unified Business Number is written as eight digits in a row.
fn match_ubn(chars: &[PageChar], start: usize) -> Option<usize> {
    let end = start + 8;
    if end > chars.len() || !chars[start..end].iter().all(|c| c.norm.is_ascii_digit()) {
        return None;
    }
    let digits: String = chars[start..end].iter().map(|c| c.norm).collect();
    if !is_valid_ubn(&digits) {
        return None;
    }
    if end < chars.len() && chars[end].norm.is_ascii_alphanumeric() {
        return None;
    }
    if number_continues_at(chars, end) {
        return None;
    }
    Some(end)
}

/// Whether a longer number joined by `-`, `.` or `/` goes on at `at` (`1111-1112`): a match must
/// not start or end in the middle of one.
fn number_continues_at(chars: &[PageChar], at: usize) -> bool {
    match chars.get(at).map(|c| c.norm) {
        Some(c) if c.is_ascii_digit() => true,
        Some(c) if is_number_joiner(c) => {
            chars.get(at + 1).is_some_and(|n| n.norm.is_ascii_digit())
        }
        _ => false,
    }
}

/// Whether the number starting at `start` is the tail of a longer one (see `number_continues_at`).
fn number_goes_on_before(chars: &[PageChar], start: usize) -> bool {
    match start.checked_sub(1).map(|i| chars[i].norm) {
        Some(c) if c.is_ascii_digit() => true,
        Some(c) if is_number_joiner(c) => start
            .checked_sub(2)
            .is_some_and(|i| chars[i].norm.is_ascii_digit()),
        _ => false,
    }
}

fn is_number_joiner(c: char) -> bool {
    c == '-' || c == '.' || c == '/'
}

fn is_email_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '.'
        || c == '_'
        || c == '%'
        || c == '+'
        || c == '-'
        || c == '@'
}

fn match_email(chars: &[PageChar], start: usize) -> Option<usize> {
    let mut candidate = String::new();
    let mut j = start;
    while j < chars.len() {
        let c = chars[j].norm;
        if is_email_char(c) {
            if candidate.len() == MAX_EMAIL_CHARS {
                return None;
            }
            candidate.push(c);
            j += 1;
        } else {
            break;
        }
    }
    // Trim trailing dot or punctuation if any
    while candidate.ends_with('.') || candidate.ends_with('-') {
        candidate.pop();
        j -= 1;
    }
    if is_valid_email(&candidate) {
        return Some(j);
    }
    None
}

fn match_keyword(chars: &[PageChar], start: usize, keyword: &str) -> Option<usize> {
    let kw_norm: Vec<char> = keyword
        .chars()
        .map(normalize_char)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if kw_norm.is_empty() {
        return None;
    }
    let mut k = 0;
    let mut j = start;
    while j < chars.len() && k < kw_norm.len() {
        let c = chars[j].norm.to_ascii_lowercase();
        if c == kw_norm[k] {
            k += 1;
            j += 1;
        } else if (c == ' ' || c == '\n') && (k > 0 && kw_norm[k - 1] != ' ') {
            // Skip line break / space in text
            j += 1;
        } else {
            return None;
        }
    }
    if k == kw_norm.len() { Some(j) } else { None }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::Point;

    use super::*;

    #[test]
    fn taiwan_id_validation_citizen() {
        // Valid citizen IDs
        assert!(is_valid_taiwan_id("A123456789"));
        assert!(is_valid_taiwan_id("B223456782"));

        // Wrong check digit
        assert!(!is_valid_taiwan_id("A123456788"));
        assert!(!is_valid_taiwan_id("B223456781"));

        // Wrong gender / first digit
        assert!(!is_valid_taiwan_id("A323456789"));
        assert!(!is_valid_taiwan_id("A023456789"));

        // Wrong length
        assert!(!is_valid_taiwan_id("A12345678"));
        assert!(!is_valid_taiwan_id("A1234567890"));
    }

    #[test]
    fn taiwan_id_validation_new_arc() {
        // Valid New ARC
        assert!(is_valid_taiwan_id("A800000014"));
        assert!(is_valid_taiwan_id("B900000017"));

        // Wrong check digit
        assert!(!is_valid_taiwan_id("A800000015"));
        assert!(!is_valid_taiwan_id("B900000010"));
    }

    #[test]
    fn luhn_credit_card_validation() {
        // Valid Luhn
        assert!(is_valid_luhn("4532015012345671"));
        assert!(is_valid_luhn("4012888888881")); // 13 digits
        assert!(is_valid_luhn("378282246310005")); // 15 digits Amex

        // Invalid Luhn checksum
        assert!(!is_valid_luhn("4532015012345678"));

        // Invalid length
        assert!(!is_valid_luhn("123456789012")); // 12 digits (too short)
        assert!(!is_valid_luhn("12345678901234567890")); // 20 digits (too long)
    }

    #[test]
    fn ubn_validation_old_and_new_rules() {
        // 10458575: valid under both old and new (sum is 20)
        assert!(is_valid_ubn_old("10458575"));
        assert!(is_valid_ubn_new("10458575"));
        assert!(is_valid_ubn("10458575"));

        // 04595252: official MOF sample, valid under new rule (% 5), invalid under old (% 10)
        assert!(!is_valid_ubn_old("04595252"));
        assert!(is_valid_ubn_new("04595252"));
        assert!(is_valid_ubn("04595252"));

        // Invalid checksum
        assert!(!is_valid_ubn_new("12345678"));
        assert!(!is_valid_ubn_old("12345678"));
        assert!(!is_valid_ubn("12345678"));

        // Invalid length
        assert!(!is_valid_ubn("1234567"));
        assert!(!is_valid_ubn("123456789"));
    }

    #[test]
    fn phone_numbers_validation() {
        // Mobile
        assert!(is_valid_mobile("0912345678"));
        assert!(!is_valid_mobile("0812345678"));
        assert!(!is_valid_mobile("091234567"));
        assert!(!is_valid_mobile("09123456789"));

        // Landline
        assert!(is_valid_landline("02-2345-6789", "0223456789"));
        assert!(is_valid_landline("(02) 2345-6789", "0223456789"));
        assert!(is_valid_landline("04-2345-6789", "0423456789"));
        assert!(is_valid_landline("037-123456", "037123456"));
        assert!(is_valid_landline("04-723-4567", "047234567"));
        assert!(is_valid_landline("049-223-4567", "0492234567"));
        assert!(is_valid_landline("07-123-4567", "071234567"));
        // Too long for the area: 08, 03, 05, 06 and 07 numbers have 9 digits in all.
        assert!(!is_valid_landline("0812-345-678", "0812345678"));
        assert!(!is_valid_landline("07-1234-5678", "0712345678"));
        // Too short for Taipei.
        assert!(!is_valid_landline("02-234-5678", "022345678"));
        // Without delimiter: not valid landline to avoid false positives
        assert!(!is_valid_landline("0223456789", "0223456789"));
        // Starting with 09: not landline
        assert!(!is_valid_landline("09-1234-5678", "0912345678"));
    }

    #[test]
    fn email_validation() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("first.last+tag@sub.domain.co.tw"));
        assert!(!is_valid_email("user@"));
        assert!(!is_valid_email("@domain.com"));
        assert!(!is_valid_email("user@domain"));
        assert!(!is_valid_email("user..name@domain.com"));
    }

    #[test]
    fn full_width_normalization() {
        assert_eq!(normalize_char('０'), '0');
        assert_eq!(normalize_char('９'), '9');
        assert_eq!(normalize_char('Ａ'), 'A');
        assert_eq!(normalize_char('ｚ'), 'z');
        assert_eq!(normalize_char('－'), '-');
        assert_eq!(normalize_char('—'), '-');
        assert_eq!(normalize_char('（'), '(');
        assert_eq!(normalize_char('）'), ')');
        assert_eq!(normalize_char('　'), ' ');
        assert_eq!(normalize_char('＠'), '@');
        assert_eq!(normalize_char('．'), '.');
        assert_eq!(normalize_char('X'), 'X');
    }

    #[test]
    fn masking_previews_hide_sensitive_characters() {
        assert_eq!(
            mask_preview(PersonalDataCategory::TaiwanId, "A123456789"),
            "A12****789"
        );
        assert_eq!(
            mask_preview(PersonalDataCategory::MobilePhone, "0912-345-678"),
            "0912****78"
        );
        assert_eq!(
            mask_preview(PersonalDataCategory::LandlinePhone, "(02) 2345-6789"),
            "(02)****89"
        );
        assert_eq!(
            mask_preview(PersonalDataCategory::CreditCard, "4532-0150-1234-5671"),
            "4532 **** **** 5671"
        );
        assert_eq!(
            mask_preview(PersonalDataCategory::UnifiedBusinessNumber, "10458575"),
            "10****75"
        );
        assert_eq!(
            mask_preview(PersonalDataCategory::Email, "username@example.com"),
            "u***e@example.com"
        );
    }

    #[test]
    fn long_email_domains_are_masked_and_overlong_addresses_are_skipped() {
        let label = "a".repeat(60);
        let domain = format!("{label}.{label}.{label}.example.com");
        let address = format!("someone@{domain}");
        assert!(address.len() <= MAX_EMAIL_CHARS);
        let preview = mask_preview(PersonalDataCategory::Email, &address);
        assert!(
            preview.len() <= MAX_PERSONAL_DATA_PREVIEW_BYTES,
            "{preview}"
        );
        assert!(!preview.contains(&label), "{preview}");

        let chars: Vec<PageChar> = address
            .chars()
            .enumerate()
            .map(|(i, c)| dummy_char(c, 0, i as f32 * 5.0))
            .collect();
        assert_eq!(match_email(&chars, 0), Some(chars.len()));

        let overlong = format!("someone@{}.example.com", "b".repeat(MAX_EMAIL_CHARS));
        let chars: Vec<PageChar> = overlong
            .chars()
            .enumerate()
            .map(|(i, c)| dummy_char(c, 0, i as f32 * 5.0))
            .collect();
        assert_eq!(match_email(&chars, 0), None);
    }

    #[test]
    fn matches_over_many_lines_stay_within_the_rect_limit() {
        let rects: Vec<Rect> = (0..200)
            .map(|i| Rect {
                x0: 10.0,
                y0: i as f32 * 12.0,
                x1: 20.0,
                y1: i as f32 * 12.0 + 10.0,
            })
            .collect();
        let capped = cap_rects(rects);
        assert!(capped.len() <= MAX_REDACTION_GROUP_RECTS);
        assert_eq!(capped.first().map(|r| r.y0), Some(0.0));
        assert_eq!(capped.last().map(|r| r.y1), Some(199.0 * 12.0 + 10.0));
    }

    fn dummy_char(orig: char, line: u32, x: f32) -> PageChar {
        PageChar {
            orig,
            norm: normalize_char(orig),
            quad: Some(Quad {
                ul: Point {
                    x,
                    y: line as f32 * 20.0,
                },
                ur: Point {
                    x: x + 10.0,
                    y: line as f32 * 20.0,
                },
                ll: Point {
                    x,
                    y: line as f32 * 20.0 + 10.0,
                },
                lr: Point {
                    x: x + 10.0,
                    y: line as f32 * 20.0 + 10.0,
                },
            }),
            line_idx: line,
        }
    }

    #[test]
    fn scanning_across_lines_and_full_width() {
        // ID wrapped across 2 lines with full-width characters:
        // Line 0: "Ａ１２３４"
        // Line 1: "５６７８９"
        let mut chars = Vec::new();
        for (i, c) in "Ａ１２３４".chars().enumerate() {
            chars.push(dummy_char(c, 0, i as f32 * 10.0));
        }
        // Inter-line virtual space
        chars.push(PageChar {
            orig: '\n',
            norm: '\n',
            quad: None,
            line_idx: 0,
        });
        for (i, c) in "５６７８９".chars().enumerate() {
            chars.push(dummy_char(c, 1, i as f32 * 10.0));
        }

        let query = FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::TaiwanId],
            keyword: None,
        };
        let mut matches = Vec::new();
        let capped = scan_page(0, &chars, &query, &mut matches, 100);
        assert!(!capped);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].category, PersonalDataCategory::TaiwanId);
        assert_eq!(matches[0].page, 0);
        // Spans 2 lines, so 2 rects
        assert_eq!(matches[0].rects.len(), 2);
        assert_eq!(matches[0].preview, "A12****789");
    }

    #[test]
    fn boundary_check_ignores_numbers_embedded_in_longer_words() {
        // "ABC123456789DEF" should NOT match Taiwan ID or UBN
        let mut chars = Vec::new();
        for (i, c) in "ABC123456789DEF".chars().enumerate() {
            chars.push(dummy_char(c, 0, i as f32 * 10.0));
        }
        let query = FindPersonalDataQuery {
            categories: vec![
                PersonalDataCategory::TaiwanId,
                PersonalDataCategory::UnifiedBusinessNumber,
            ],
            keyword: None,
        };
        let mut matches = Vec::new();
        scan_page(0, &chars, &query, &mut matches, 100);
        assert!(matches.is_empty());
    }

    #[test]
    fn numbers_inside_longer_joined_numbers_are_not_matched() {
        // The tail of a card number that fails Luhn is not a business number, and a business
        // number is eight digits in a row, not split by a dash.
        let mut chars = Vec::new();
        for (i, c) in "4111-1111-1111-1112 1045-8575".chars().enumerate() {
            chars.push(dummy_char(c, 0, i as f32 * 10.0));
        }
        let query = FindPersonalDataQuery {
            categories: vec![
                PersonalDataCategory::CreditCard,
                PersonalDataCategory::UnifiedBusinessNumber,
                PersonalDataCategory::LandlinePhone,
            ],
            keyword: None,
        };
        let mut matches = Vec::new();
        scan_page(0, &chars, &query, &mut matches, 100);
        assert!(matches.is_empty(), "{matches:?}");
    }

    #[test]
    fn capping_at_limit_reports_has_more() {
        let mut chars = Vec::new();
        // 5 valid IDs
        for k in 0..5 {
            for (i, c) in "A123456789 ".chars().enumerate() {
                chars.push(dummy_char(c, k as u32, i as f32 * 10.0));
            }
        }
        let query = FindPersonalDataQuery {
            categories: vec![PersonalDataCategory::TaiwanId],
            keyword: None,
        };
        let mut matches = Vec::new();
        let capped = scan_page(0, &chars, &query, &mut matches, 3);
        assert!(capped);
        assert_eq!(matches.len(), 3);
    }
}
