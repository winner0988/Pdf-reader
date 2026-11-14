//! Reads and formats page labels (/PageLabels, B5-07, PDF 1.7 §12.4.2).
//!
//! Page labels associate 0-based page indices with human-readable labels (e.g. roman numerals
//! for front matter, decimal starting at 1 for body text, or custom prefixes).
//!
//! The `/PageLabels` tree is untrusted input from the file. To protect against malicious trees,
//! we traverse the number tree iteratively using an explicit stack, record visited indirect object
//! numbers to prevent cycles, and strictly enforce limits on tree depth ([`MAX_PAGE_LABEL_DEPTH`])
//! and total visited nodes ([`MAX_PAGE_LABEL_NODES`]). Bounding the total node budget accounts for
//! trees where multiple `/Kids` reference the same child node (DAG structures).
//! When bounds are exceeded or when the structure is malformed, reading safely falls back to physical
//! page numbers (`None`) rather than failing open or crashing.

use std::collections::HashSet;

use ipc_contract::limits::{MAX_PAGE_LABEL_BYTES, MAX_PAGE_LABEL_DEPTH, MAX_PAGE_LABEL_NODES};
use ipc_contract::text::clean_page_label;
use mupdf::pdf::PdfObject;

use super::EngineError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberingStyle {
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperAlpha,
    LowerAlpha,
    None,
}

#[derive(Debug, Clone)]
struct LabelRule {
    style: NumberingStyle,
    prefix: String,
    st: u64,
}

/// Safely walks `/PageLabels` and returns formatted labels for all `page_count` pages.
///
/// Returns `Ok(None)` if `/PageLabels` is absent, malformed, exceeds tree bounds, or if every
/// formatted label is identical to the 1-based physical page number.
pub(crate) fn read_page_labels(
    root: &PdfObject,
    page_count: usize,
) -> Result<Option<Vec<String>>, EngineError> {
    if !root.is_dict()? || page_count == 0 {
        return Ok(None);
    }

    let mut visited_indirect = HashSet::new();
    let mut total_nodes = 0usize;
    let mut entries: Vec<(u32, LabelRule)> = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];

    while let Some((node, depth)) = stack.pop() {
        total_nodes = total_nodes.saturating_add(1);
        if total_nodes > MAX_PAGE_LABEL_NODES || depth > MAX_PAGE_LABEL_DEPTH {
            return Ok(None);
        }

        if let Ok(num) = node.as_indirect()
            && num > 0
            && !visited_indirect.insert(num)
        {
            continue;
        }

        if !node.is_dict()? {
            continue;
        }

        // Check for intermediate node (/Kids)
        if let Ok(Some(kids)) = node.get_dict("Kids")
            && kids.is_array()?
        {
            let mut kids_vec = Vec::new();
            for kid in kids.array_iter()?.flatten() {
                kids_vec.push(kid);
            }
            // Push in reverse order so leftmost child is visited first
            for kid in kids_vec.into_iter().rev() {
                stack.push((kid, depth + 1));
            }
        }

        // Check for leaf node (/Nums)
        if let Ok(Some(nums)) = node.get_dict("Nums")
            && nums.is_array()?
        {
            let items: Vec<PdfObject> = nums.array_iter()?.flatten().collect();
            for chunk in items.as_chunks::<2>().0 {
                let key_obj = &chunk[0];
                let val_obj = &chunk[1];

                // Key must be an integer, >= 0 and within document page count
                let Ok(true) = key_obj.is_int() else {
                    continue;
                };
                let Ok(k) = key_obj.as_int() else {
                    continue;
                };
                if k < 0 || (k as usize) >= page_count {
                    continue;
                }
                let key = k as u32;

                // Number tree keys must be strictly ascending
                if let Some(&(last_k, _)) = entries.last()
                    && key <= last_k
                {
                    continue;
                }

                // Value must be a dictionary
                let Ok(true) = val_obj.is_dict() else {
                    continue;
                };

                let rule = parse_label_dict(val_obj);
                entries.push((key, rule));
            }
        }
    }

    if entries.is_empty() {
        return Ok(None);
    }

    // Format label for every page in 0..page_count
    let mut labels = Vec::with_capacity(page_count);
    let mut rule_idx = 0usize;

    for p in 0..page_count {
        let p_u32 = p as u32;
        // Advance rule if the next rule starts at or before page p
        while rule_idx + 1 < entries.len() && entries[rule_idx + 1].0 <= p_u32 {
            rule_idx += 1;
        }

        let label = if entries[rule_idx].0 <= p_u32 {
            let (start_page, ref rule) = entries[rule_idx];
            let offset = (p_u32 - start_page) as u64;
            format_page_label(rule, offset)
        } else {
            // If the first rule starts after page p, fallback to default decimal 1-based page number
            (p + 1).to_string()
        };

        labels.push(label);
    }

    // If every label equals its physical 1-based page number, report no custom labels
    let all_equal_physical = labels
        .iter()
        .enumerate()
        .all(|(i, l)| l == &(i + 1).to_string());
    if all_equal_physical {
        return Ok(None);
    }

    Ok(Some(labels))
}

fn parse_label_dict(dict: &PdfObject) -> LabelRule {
    let style = match name_of(dict, "S").as_deref() {
        Some(b"D") => NumberingStyle::Decimal,
        Some(b"R") => NumberingStyle::UpperRoman,
        Some(b"r") => NumberingStyle::LowerRoman,
        Some(b"A") => NumberingStyle::UpperAlpha,
        Some(b"a") => NumberingStyle::LowerAlpha,
        _ => NumberingStyle::None,
    };

    let raw_prefix = text_of(dict, "P").unwrap_or_default();
    let prefix = clean_page_label(&raw_prefix, MAX_PAGE_LABEL_BYTES);

    let st = dict
        .get_dict("St")
        .ok()
        .flatten()
        .and_then(|v| v.as_int().ok())
        .map(|s| s.max(1) as u64)
        .unwrap_or(1);

    LabelRule { style, prefix, st }
}

fn format_page_label(rule: &LabelRule, offset: u64) -> String {
    let num = rule.st.saturating_add(offset);
    let num_str = match rule.style {
        NumberingStyle::Decimal => num.to_string(),
        NumberingStyle::UpperRoman => format_roman(num, true),
        NumberingStyle::LowerRoman => format_roman(num, false),
        NumberingStyle::UpperAlpha => format_alpha(num, true, MAX_PAGE_LABEL_BYTES),
        NumberingStyle::LowerAlpha => format_alpha(num, false, MAX_PAGE_LABEL_BYTES),
        NumberingStyle::None => String::new(),
    };

    let combined = format!("{}{num_str}", rule.prefix);
    clean_page_label(&combined, MAX_PAGE_LABEL_BYTES)
}

/// Formats `n` as a Roman numeral for numbers 1..=3999.
/// For 0 or numbers > 3999, falls back to decimal representation.
fn format_roman(mut n: u64, uppercase: bool) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    const ROMAN_TABLE: &[(u64, &str, &str)] = &[
        (1000, "M", "m"),
        (900, "CM", "cm"),
        (500, "D", "d"),
        (400, "CD", "cd"),
        (100, "C", "c"),
        (90, "XC", "xc"),
        (50, "L", "l"),
        (40, "XL", "xl"),
        (10, "X", "x"),
        (9, "IX", "ix"),
        (5, "V", "v"),
        (4, "IV", "iv"),
        (1, "I", "i"),
    ];

    let mut out = String::new();
    for &(val, upper, lower) in ROMAN_TABLE {
        while n >= val {
            out.push_str(if uppercase { upper } else { lower });
            n -= val;
        }
    }
    out
}

/// Formats `n` using alphabetic numbering (1..26 => A..Z, 27..52 => AA..ZZ, etc.).
/// Execution time and allocation are bounded by `max_bytes` even if `n` is large.
fn format_alpha(n: u64, uppercase: bool, max_bytes: usize) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let zero_based = n - 1;
    let letter_idx = (zero_based % 26) as u8;
    let base_char = if uppercase { b'A' } else { b'a' };
    let ch = (base_char + letter_idx) as char;
    let count = (zero_based / 26) + 1;
    let repeat = (count.min(max_bytes as u64)) as usize;
    std::iter::repeat_n(ch, repeat).collect()
}

fn text_of(object: &PdfObject, key: &str) -> Option<String> {
    let text = object.get_dict(key).ok()??;
    if !text.is_string().ok()? {
        return None;
    }
    text.as_string().ok()
}

fn name_of(object: &PdfObject, key: &str) -> Option<Vec<u8>> {
    let name = object.get_dict(key).ok()??;
    if !name.is_name().ok()? {
        return None;
    }
    name.as_name().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn make_pdf(page_count: usize, page_labels_obj: &str) -> Vec<u8> {
        let mut objects = Vec::new();
        // Object 1: Catalog
        objects.push(format!(
            "<< /Type /Catalog /Pages 2 0 R /PageLabels {page_labels_obj} >>"
        ));
        // Object 2: Pages
        let kids: Vec<String> = (0..page_count).map(|i| format!("{} 0 R", i + 3)).collect();
        objects.push(format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            page_count
        ));
        // Pages 3..3+page_count-1
        for _ in 0..page_count {
            objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string());
        }

        // Build PDF
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn open_and_read_labels(pdf_bytes: &[u8]) -> Option<Vec<String>> {
        let doc = crate::engine::PdfDocument::from_bytes(pdf_bytes).expect("open document");
        doc.page_labels().expect("read page labels")
    }

    #[test]
    fn roman_numerals_lower_and_upper() {
        let pdf = make_pdf(5, "<< /Nums [ 0 << /S /r >> 3 << /S /R /St 10 >> ] >>");
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels, vec!["i", "ii", "iii", "X", "XI"]);
    }

    #[test]
    fn alphabetic_style_lower_and_upper() {
        let pdf = make_pdf(
            4,
            "<< /Nums [ 0 << /S /a /St 25 >> 2 << /S /A /St 51 >> ] >>",
        );
        let labels = open_and_read_labels(&pdf).expect("has labels");
        // /a from 25: 25 -> y, 26 -> z
        // /A from 51: 51 -> YY, 52 -> ZZ
        assert_eq!(labels, vec!["y", "z", "YY", "ZZ"]);
    }

    #[test]
    fn prefix_and_start_number() {
        let pdf = make_pdf(3, "<< /Nums [ 0 << /P (Sec-) /S /D /St 10 >> ] >>");
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels, vec!["Sec-10", "Sec-11", "Sec-12"]);
    }

    #[test]
    fn only_prefix_no_style() {
        let pdf = make_pdf(2, "<< /Nums [ 0 << /P (Cover) >> 1 << /P (Back) >> ] >>");
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels, vec!["Cover", "Back"]);
    }

    #[test]
    fn starting_from_middle() {
        // Page 0 has no explicit rule -> default 1-based decimal ("1").
        // Page 1 onwards has prefix "App-"
        let pdf = make_pdf(3, "<< /Nums [ 1 << /P (App-) /S /D /St 1 >> ] >>");
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels, vec!["1", "App-1", "App-2"]);
    }

    #[test]
    fn no_page_labels_returns_none() {
        let pdf = make_pdf(3, "null");
        let labels = open_and_read_labels(&pdf);
        assert_eq!(labels, None);
    }

    #[test]
    fn labels_equal_physical_page_numbers_returns_none() {
        // Explicit decimal 1, 2, 3 -> equals physical pages -> returns None
        let pdf = make_pdf(3, "<< /Nums [ 0 << /S /D /St 1 >> ] >>");
        let labels = open_and_read_labels(&pdf);
        assert_eq!(labels, None);
    }

    #[test]
    fn kids_tree_nested_valid() {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
            // 4: Root node with Kids
            "<< /Kids [5 0 R 6 0 R] >>".to_string(),
            // 5: Leaf node for page 0
            "<< /Nums [ 0 << /S /r >> ] >>".to_string(),
            // 6: Leaf node for page 1
            "<< /Nums [ 1 << /S /D /St 100 >> ] >>".to_string(),
        ];

        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        let labels = open_and_read_labels(&out).expect("has labels");
        assert_eq!(labels, vec!["i", "100"]);
    }

    #[test]
    fn kids_cycle_to_self_finishes_under_one_second() {
        // Object 4 has /Kids [4 0 R] (cycle to self)
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
            "<< /Kids [4 0 R] /Nums [ 0 << /S /r >> ] >>".to_string(),
        ];

        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        let start = Instant::now();
        let labels = open_and_read_labels(&out);
        let elapsed = start.elapsed();
        println!("kids_cycle_to_self elapsed: {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(1));
        // Cycle detected: indirect object revisited, does not hang
        assert_eq!(labels, Some(vec!["i".to_string()]));
    }

    #[test]
    fn kids_cycle_to_ancestor_finishes_under_one_second() {
        // Object 4 has /Kids [5 0 R], and Object 5 has /Kids [4 0 R]
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
            "<< /Kids [5 0 R] >>".to_string(),
            "<< /Kids [4 0 R] /Nums [ 0 << /S /r >> ] >>".to_string(),
        ];

        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        let start = Instant::now();
        let labels = open_and_read_labels(&out);
        let elapsed = start.elapsed();
        println!("kids_cycle_to_ancestor elapsed: {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(1));
        assert_eq!(labels, Some(vec!["i".to_string()]));
    }

    #[test]
    fn kids_shared_nodes_dag_finishes_under_one_second() {
        // Under limit: Object 4 has 20 /Kids pointing to the same child object 5
        let shared_kids_small = vec!["5 0 R"; 20].join(" ");
        let mut objects = Vec::new();
        objects.push("<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>".to_string());
        objects.push("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string());
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string());
        objects.push(format!("<< /Kids [{shared_kids_small}] >>"));
        objects.push("<< /Nums [ 0 << /S /r >> ] >>".to_string());

        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        let start_small = Instant::now();
        let labels_small = open_and_read_labels(&out);
        let elapsed_small = start_small.elapsed();
        assert!(elapsed_small < Duration::from_secs(1));
        assert_eq!(labels_small, Some(vec!["i".to_string()]));

        // Exceeding total work limit: Object 4 has 500 /Kids pointing to child object 5 (> MAX_PAGE_LABEL_NODES)
        let shared_kids_large = vec!["5 0 R"; 500].join(" ");
        let mut objects_large = Vec::new();
        objects_large.push("<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>".to_string());
        objects_large.push("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string());
        objects_large.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string());
        objects_large.push(format!("<< /Kids [{shared_kids_large}] >>"));
        objects_large.push("<< /Nums [ 0 << /S /r >> ] >>".to_string());

        let mut out_large = b"%PDF-1.7\n".to_vec();
        let mut offsets_large = Vec::new();
        for (i, body) in objects_large.iter().enumerate() {
            offsets_large.push(out_large.len());
            out_large.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref_large = out_large.len();
        out_large.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects_large.len() + 1).as_bytes(),
        );
        for offset in offsets_large {
            out_large.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out_large.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_large}\n%%EOF\n",
                objects_large.len() + 1
            )
            .as_bytes(),
        );

        let start_large = Instant::now();
        let labels_large = open_and_read_labels(&out_large);
        let elapsed_large = start_large.elapsed();
        println!("kids_shared_nodes_dag elapsed: {:?}", elapsed_large);
        assert!(elapsed_large < Duration::from_secs(1));
        // Exceeded MAX_PAGE_LABEL_NODES: safely falls back to None
        assert_eq!(labels_large, None);
    }

    #[test]
    fn huge_start_number_finishes_under_one_second() {
        // /St is 2147483647 (max i32) with Roman and Alpha styles
        let pdf = make_pdf(
            2,
            "<< /Nums [ 0 << /S /R /St 2147483647 >> 1 << /S /A /St 2147483647 >> ] >>",
        );
        let start = Instant::now();
        let labels = open_and_read_labels(&pdf).expect("has labels");
        let elapsed = start.elapsed();
        println!("huge_start_number elapsed: {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(1));
        // Roman > 3999 falls back to decimal:
        assert_eq!(labels[0], "2147483647");
        // Alpha caps repeat at MAX_PAGE_LABEL_BYTES (64):
        assert_eq!(labels[1].len(), 64);
        assert!(labels[1].chars().all(|c| c == 'W'));
    }

    #[test]
    fn prefix_long_and_control_characters() {
        // Prefix contains control chars, bidi overrides (U+202E), and exceeds 64 bytes
        let long_raw = "Very-Long-Prefix-Exceeding-Sixty-Four-Bytes-Limit-1234567890-ABCDEFGHIJKLM\u{0000}\u{0007}\u{202E}";
        let pdf = make_pdf(
            1,
            &format!("<< /Nums [ 0 << /P ({long_raw}) /S /D /St 1 >> ] >>"),
        );
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels.len(), 1);
        let label = &labels[0];
        assert!(label.len() <= MAX_PAGE_LABEL_BYTES);
        assert!(!label.contains('\u{0000}'));
        assert!(!label.contains('\u{0007}'));
        assert!(!label.contains('\u{202E}'));
    }

    #[test]
    fn nums_keys_non_integer_or_out_of_order() {
        // Keys: 0 (valid), /Bad (non-int), -5 (negative), 100 (exceeds page count 2),
        // 0 (out of order, <= previous 0), 1 (valid)
        let pdf = make_pdf(
            2,
            "<< /Nums [ 0 << /S /r >> /Bad << /S /D >> -5 << /S /D >> 100 << /S /D >> 0 << /S /D >> 1 << /P (Z-) /S /D >> ] >>",
        );
        let labels = open_and_read_labels(&pdf).expect("has labels");
        assert_eq!(labels, vec!["i", "Z-1"]);
    }
}
