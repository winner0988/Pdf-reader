//! Comparing the text of two open tabs (B3-24, E9, docs/architecture/compare.md).
//!
//! Two open documents have their text extracted page-by-page through their existing workers.
//! Diffing is performed on the main process's blocking thread pool using Myers algorithm via `similar`,
//! without holding the document lock continuously. Results are cached in the main process for native export.

pub mod tokenize;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use ipc_contract::limits::{
    MAX_COMPARE_CACHE_BYTES, MAX_COMPARE_DEADLINE_MILLIS, MAX_COMPARE_DIFF_ITEMS,
    MAX_COMPARE_PAGES, MAX_COMPARE_TEXT_BYTES, MAX_COMPARE_TOKENS,
};
use ipc_contract::types::{CompareTextResult, DocumentId, ErrorCode, IpcError, TextDiffItem};
use similar::{Algorithm, DiffOp};

use crate::documents::Documents;
use tokenize::Token;

/// A cached comparison result kept in the main process for file export (B3-24, E9).
#[derive(Debug, Clone)]
pub struct CachedComparison {
    pub doc_a: DocumentId,
    pub doc_b: DocumentId,
    pub name_a: String,
    pub name_b: String,
    pub result: CompareTextResult,
}

/// Thread-safe single-slot cache for the most recent comparison result.
#[derive(Default)]
pub struct CompareCache {
    entry: Mutex<Option<CachedComparison>>,
}

impl CompareCache {
    pub fn store(
        &self,
        doc_a: DocumentId,
        doc_b: DocumentId,
        name_a: String,
        name_b: String,
        result: CompareTextResult,
    ) {
        let mut lock = self.entry.lock().unwrap();
        // Keep memory within MAX_COMPARE_CACHE_BYTES
        let approx_bytes = result
            .items
            .iter()
            .map(|i| i.deleted.len() + i.inserted.len() + 64)
            .sum::<usize>();
        if approx_bytes <= MAX_COMPARE_CACHE_BYTES {
            *lock = Some(CachedComparison {
                doc_a,
                doc_b,
                name_a,
                name_b,
                result,
            });
        } else {
            *lock = None;
        }
    }

    pub fn get(&self, doc_a: DocumentId, doc_b: DocumentId) -> Option<CachedComparison> {
        let lock = self.entry.lock().unwrap();
        lock.as_ref()
            .filter(|c| {
                (c.doc_a == doc_a && c.doc_b == doc_b) || (c.doc_a == doc_b && c.doc_b == doc_a)
            })
            .cloned()
    }
}

/// Truncates `text` at UTF-8 character boundary if it exceeds `max_bytes`, appending ellipsis.
pub fn truncate_hunk_text(text: String, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let limit = max_bytes.saturating_sub(3);
    let mut boundary = 0;
    for (idx, _) in text.char_indices() {
        if idx > limit {
            break;
        }
        boundary = idx;
    }
    let mut out = text[..boundary].to_owned();
    out.push('…');
    (out, true)
}

/// Compares two token sequences, combining adjacent deletes and inserts into diff items.
pub fn diff_tokens(
    tokens_a: &[Token],
    tokens_b: &[Token],
    deadline: Option<Instant>,
) -> (Vec<TextDiffItem>, bool) {
    let ops = similar::capture_diff_slices_deadline(Algorithm::Myers, tokens_a, tokens_b, deadline);
    let timed_out = deadline.is_some_and(|d| Instant::now() >= d);

    let mut items: Vec<TextDiffItem> = Vec::new();
    let mut limit_hit = timed_out;

    // Merge adjacent delete and insert operations into single replacement hunks
    struct Hunk {
        old_start: usize,
        old_end: usize,
        new_start: usize,
        new_end: usize,
    }

    let mut current_hunk: Option<Hunk> = None;

    let flush_hunk = |hunk: Hunk, items: &mut Vec<TextDiffItem>| -> bool {
        if items.len() >= MAX_COMPARE_DIFF_ITEMS as usize {
            return true;
        }

        let page_a = if hunk.old_start < hunk.old_end {
            tokens_a[hunk.old_start].page
        } else if hunk.old_start > 0 && hunk.old_start <= tokens_a.len() {
            tokens_a[hunk.old_start - 1].page
        } else if !tokens_a.is_empty() {
            tokens_a[0].page
        } else {
            0
        };

        let page_b = if hunk.new_start < hunk.new_end {
            tokens_b[hunk.new_start].page
        } else if hunk.new_start > 0 && hunk.new_start <= tokens_b.len() {
            tokens_b[hunk.new_start - 1].page
        } else if !tokens_b.is_empty() {
            tokens_b[0].page
        } else {
            0
        };

        let deleted_raw = tokenize::join_tokens(&tokens_a[hunk.old_start..hunk.old_end]);
        let inserted_raw = tokenize::join_tokens(&tokens_b[hunk.new_start..hunk.new_end]);

        let (deleted, del_trunc) = truncate_hunk_text(deleted_raw, MAX_COMPARE_TEXT_BYTES as usize);
        let (inserted, ins_trunc) =
            truncate_hunk_text(inserted_raw, MAX_COMPARE_TEXT_BYTES as usize);

        items.push(TextDiffItem {
            page_a,
            page_b,
            deleted,
            inserted,
            truncated: del_trunc || ins_trunc,
        });

        false
    };

    for op in ops {
        match op {
            DiffOp::Equal { .. } => {
                if let Some(hunk) = current_hunk.take()
                    && flush_hunk(hunk, &mut items)
                {
                    limit_hit = true;
                    break;
                }
            }
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => {
                let op_old_end = old_index + old_len;
                if let Some(mut hunk) = current_hunk.take() {
                    if hunk.old_end == old_index && hunk.new_end == new_index {
                        hunk.old_end = op_old_end;
                        current_hunk = Some(hunk);
                    } else {
                        if flush_hunk(hunk, &mut items) {
                            limit_hit = true;
                            break;
                        }
                        current_hunk = Some(Hunk {
                            old_start: old_index,
                            old_end: op_old_end,
                            new_start: new_index,
                            new_end: new_index,
                        });
                    }
                } else {
                    current_hunk = Some(Hunk {
                        old_start: old_index,
                        old_end: op_old_end,
                        new_start: new_index,
                        new_end: new_index,
                    });
                }
            }
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                let op_new_end = new_index + new_len;
                if let Some(mut hunk) = current_hunk.take() {
                    if hunk.old_end == old_index && hunk.new_end == new_index {
                        hunk.new_end = op_new_end;
                        current_hunk = Some(hunk);
                    } else {
                        if flush_hunk(hunk, &mut items) {
                            limit_hit = true;
                            break;
                        }
                        current_hunk = Some(Hunk {
                            old_start: old_index,
                            old_end: old_index,
                            new_start: new_index,
                            new_end: op_new_end,
                        });
                    }
                } else {
                    current_hunk = Some(Hunk {
                        old_start: old_index,
                        old_end: old_index,
                        new_start: new_index,
                        new_end: op_new_end,
                    });
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let op_old_end = old_index + old_len;
                let op_new_end = new_index + new_len;
                if let Some(mut hunk) = current_hunk.take() {
                    if hunk.old_end == old_index && hunk.new_end == new_index {
                        hunk.old_end = op_old_end;
                        hunk.new_end = op_new_end;
                        current_hunk = Some(hunk);
                    } else {
                        if flush_hunk(hunk, &mut items) {
                            limit_hit = true;
                            break;
                        }
                        current_hunk = Some(Hunk {
                            old_start: old_index,
                            old_end: op_old_end,
                            new_start: new_index,
                            new_end: op_new_end,
                        });
                    }
                } else {
                    current_hunk = Some(Hunk {
                        old_start: old_index,
                        old_end: op_old_end,
                        new_start: new_index,
                        new_end: op_new_end,
                    });
                }
            }
        }
    }

    if let Some(hunk) = current_hunk.take()
        && flush_hunk(hunk, &mut items)
    {
        limit_hit = true;
    }

    (items, limit_hit)
}

/// Compares the text of two open documents across all their pages.
pub fn compare_documents(
    documents: &Documents,
    doc_a: DocumentId,
    doc_b: DocumentId,
) -> Result<CompareTextResult, IpcError> {
    let info_a = documents.document_info(doc_a).ok_or_else(|| IpcError {
        code: ErrorCode::UnknownDocument,
        message: "no such open document".to_owned(),
    })?;
    let info_b = documents.document_info(doc_b).ok_or_else(|| IpcError {
        code: ErrorCode::UnknownDocument,
        message: "no such open document".to_owned(),
    })?;

    let pages_a = (info_a.pages.len() as u32).min(MAX_COMPARE_PAGES);
    let pages_b = (info_b.pages.len() as u32).min(MAX_COMPARE_PAGES);
    let mut truncated = (info_a.pages.len() as u32) > MAX_COMPARE_PAGES
        || (info_b.pages.len() as u32) > MAX_COMPARE_PAGES;

    let mut tokens_a = Vec::new();
    let mut tokens_b = Vec::new();
    let mut page_text_truncated = false;
    let mut recognised = false;

    // Fetch pages for document A without holding lock between pages
    for page_idx in 0..pages_a {
        if tokens_a.len() >= MAX_COMPARE_TOKENS as usize {
            truncated = true;
            break;
        }
        let pt = documents.page_text(doc_a, page_idx)?;
        if pt.truncated {
            page_text_truncated = true;
        }
        if pt.recognised {
            recognised = true;
        }
        let page_tokens = tokenize::tokenize_page(page_idx, &pt.lines);
        let remaining = (MAX_COMPARE_TOKENS as usize).saturating_sub(tokens_a.len());
        if page_tokens.len() > remaining {
            tokens_a.extend(page_tokens.into_iter().take(remaining));
            truncated = true;
            break;
        } else {
            tokens_a.extend(page_tokens);
        }
    }

    // Fetch pages for document B
    for page_idx in 0..pages_b {
        if tokens_b.len() >= MAX_COMPARE_TOKENS as usize {
            truncated = true;
            break;
        }
        let pt = documents.page_text(doc_b, page_idx)?;
        if pt.truncated {
            page_text_truncated = true;
        }
        if pt.recognised {
            recognised = true;
        }
        let page_tokens = tokenize::tokenize_page(page_idx, &pt.lines);
        let remaining = (MAX_COMPARE_TOKENS as usize).saturating_sub(tokens_b.len());
        if page_tokens.len() > remaining {
            tokens_b.extend(page_tokens.into_iter().take(remaining));
            truncated = true;
            break;
        } else {
            tokens_b.extend(page_tokens);
        }
    }

    let empty = tokens_a.is_empty() && tokens_b.is_empty();
    if empty {
        return Ok(CompareTextResult {
            items: Vec::new(),
            identical: false,
            empty: true,
            recognised,
            page_text_truncated,
            truncated,
        });
    }

    let deadline = Some(Instant::now() + Duration::from_millis(MAX_COMPARE_DEADLINE_MILLIS));
    let (items, diff_truncated) = diff_tokens(&tokens_a, &tokens_b, deadline);
    let truncated = truncated || diff_truncated;
    let identical = items.is_empty() && !truncated;

    Ok(CompareTextResult {
        items,
        identical,
        empty: false,
        recognised,
        page_text_truncated,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenize::Token;

    fn make_tokens(words: &[&str], page: u32) -> Vec<Token> {
        words
            .iter()
            .enumerate()
            .map(|(i, w)| Token {
                text: w.to_string(),
                page,
                has_leading_space: i > 0,
            })
            .collect()
    }

    #[test]
    fn diff_identical_documents() {
        let a = make_tokens(&["Hello", "world", "."], 0);
        let b = make_tokens(&["Hello", "world", "."], 0);
        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert!(items.is_empty());
    }

    #[test]
    fn diff_pure_insertion() {
        let a = make_tokens(&["Hello", "world"], 0);
        let b = make_tokens(&["Hello", "brave", "new", "world"], 0);
        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].page_a, 0);
        assert_eq!(items[0].page_b, 0);
        assert_eq!(items[0].deleted, "");
        assert_eq!(items[0].inserted, "brave new");
        assert!(!items[0].truncated);
    }

    #[test]
    fn diff_pure_deletion() {
        let a = make_tokens(&["Hello", "brave", "new", "world"], 0);
        let b = make_tokens(&["Hello", "world"], 0);
        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].page_a, 0);
        assert_eq!(items[0].page_b, 0);
        assert_eq!(items[0].deleted, "brave new");
        assert_eq!(items[0].inserted, "");
        assert!(!items[0].truncated);
    }

    #[test]
    fn diff_modification_replacement() {
        let a = make_tokens(&["Party", "A", "pays", "100"], 0);
        let b = make_tokens(&["Party", "A", "pays", "200"], 0);
        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].deleted, "100");
        assert_eq!(items[0].inserted, "200");
    }

    #[test]
    fn diff_cross_page_changes() {
        let mut a = make_tokens(&["Clause", "1"], 0);
        a.extend(make_tokens(&["Clause", "2"], 1));

        let mut b = make_tokens(&["Clause", "1", "updated"], 0);
        b.extend(make_tokens(&["Clause", "2", "revised"], 1));

        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].page_a, 0);
        assert_eq!(items[0].page_b, 0);
        assert_eq!(items[0].inserted, "updated");

        assert_eq!(items[1].page_a, 1);
        assert_eq!(items[1].page_b, 1);
        assert_eq!(items[1].inserted, "revised");
    }

    #[test]
    fn diff_completely_different() {
        let a = make_tokens(&["Apple", "Banana"], 0);
        let b = make_tokens(&["Cat", "Dog"], 0);
        let (items, truncated) = diff_tokens(&a, &b, None);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].deleted, "Apple Banana");
        assert_eq!(items[0].inserted, "Cat Dog");
    }

    #[test]
    fn diff_text_truncation_for_large_hunk() {
        let big = "x".repeat(5000);
        let a = vec![Token {
            text: big.clone(),
            page: 0,
            has_leading_space: false,
        }];
        let b = vec![Token {
            text: "short".to_string(),
            page: 0,
            has_leading_space: false,
        }];
        let (items, _) = diff_tokens(&a, &b, None);
        assert_eq!(items.len(), 1);
        assert!(items[0].truncated);
        assert!(items[0].deleted.len() <= MAX_COMPARE_TEXT_BYTES as usize);
        assert!(items[0].deleted.ends_with('…'));
    }

    #[test]
    fn diff_timeout_deadline_marks_truncated() {
        let past = Instant::now() - Duration::from_millis(10);
        let a = make_tokens(&["A", "B", "C"], 0);
        let b = make_tokens(&["D", "E", "F"], 0);
        let (_, truncated) = diff_tokens(&a, &b, Some(past));
        assert!(truncated);
    }
}
