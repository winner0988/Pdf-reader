//! In-memory single-document text cache (Issue #242, D1 Option A).
//!
//! Holds the extracted text lines (including OCR and app text boxes) of searched pages in the
//! worker's memory so subsequent searches skip MuPDF text extraction.
//!
//! - Bounded by `MAX_TEXT_CACHE_BYTES` (64 MiB per document). When the limit is reached,
//!   further pages are not cached and fall back to on-demand extraction.
//! - In-memory only: not written to disk, not written to crash recovery journals, not logged.
//!   Discarded when the document is closed or the worker process terminates.
//! - Invalidated on any edit that changes page structure, page text, redactions, text boxes, or OCR.

use std::collections::HashMap;
use std::mem::size_of;

use ipc_contract::limits::MAX_TEXT_CACHE_BYTES;
use ipc_contract::types::Quad;

use crate::search::PageText;

/// Cached text for one page: character stream with matching quads, and line start indices.
#[derive(Debug, Clone, Default)]
pub struct CachedPageText {
    pub chars: Vec<char>,
    pub quads: Vec<Quad>,
    pub line_starts: Vec<usize>,
    pub has_text: bool,
}

impl CachedPageText {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a line of text: characters with their quads, in reading order.
    pub fn push_line(&mut self, chars: impl IntoIterator<Item = (char, Quad)>) {
        self.line_starts.push(self.chars.len());
        for (c, quad) in chars {
            if !c.is_whitespace() && !c.is_control() {
                self.has_text = true;
            }
            self.chars.push(c);
            self.quads.push(quad);
        }
    }

    /// Whether any line added so far has a character that is not whitespace or control.
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// Approximate heap memory in bytes.
    pub fn memory_bytes(&self) -> usize {
        size_of::<Self>()
            + 32 // estimated hash map node overhead
            + self.chars.capacity() * size_of::<char>()
            + self.quads.capacity() * size_of::<Quad>()
            + self.line_starts.capacity() * size_of::<usize>()
    }

    /// Feeds all lines into `PageText` so it can be searched.
    pub fn populate(&self, text: &mut PageText) {
        let total = self.chars.len();
        for i in 0..self.line_starts.len() {
            let start = self.line_starts[i];
            let end = if i + 1 < self.line_starts.len() {
                self.line_starts[i + 1]
            } else {
                total
            };
            let line_chars = &self.chars[start..end];
            let line_quads = &self.quads[start..end];
            text.push_line(
                line_chars
                    .iter()
                    .copied()
                    .zip(line_quads.iter().copied()),
            );
        }
    }
}

/// In-memory cache of extracted text layers per page for one document.
#[derive(Debug)]
pub struct TextCache {
    pages: HashMap<u32, CachedPageText>,
    bytes_used: usize,
    max_bytes: usize,
}

impl Default for TextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextCache {
    pub fn new() -> Self {
        Self::with_limit(MAX_TEXT_CACHE_BYTES)
    }

    pub fn with_limit(max_bytes: usize) -> Self {
        Self {
            pages: HashMap::new(),
            bytes_used: 0,
            max_bytes,
        }
    }

    pub fn get(&self, page_index: u32) -> Option<&CachedPageText> {
        self.pages.get(&page_index)
    }

    /// Inserts a page's text into the cache if within the memory limit.
    /// If the memory limit would be exceeded, the page is not cached and falls back to
    /// on-demand extraction without failing the search.
    pub fn insert(&mut self, page_index: u32, cached: CachedPageText) {
        let size = cached.memory_bytes();
        if let Some(old) = self.pages.remove(&page_index) {
            self.bytes_used = self.bytes_used.saturating_sub(old.memory_bytes());
        }
        if self.bytes_used.saturating_add(size) <= self.max_bytes {
            self.bytes_used = self.bytes_used.saturating_add(size);
            self.pages.insert(page_index, cached);
        }
    }

    pub fn invalidate_page(&mut self, page_index: u32) {
        if let Some(removed) = self.pages.remove(&page_index) {
            self.bytes_used = self.bytes_used.saturating_sub(removed.memory_bytes());
        }
    }

    pub fn clear(&mut self) {
        self.pages.clear();
        self.bytes_used = 0;
    }

    pub fn bytes_used(&self) -> usize {
        self.bytes_used
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::Point;

    use super::*;

    fn dummy_quad() -> Quad {
        Quad {
            ul: Point { x: 0.0, y: 0.0 },
            ur: Point { x: 10.0, y: 0.0 },
            ll: Point { x: 0.0, y: 10.0 },
            lr: Point {
                x: 10.0,
                y: 10.0,
            },
        }
    }

    #[test]
    fn cached_page_text_populates_page_text_identically() {
        let mut cached = CachedPageText::new();
        cached.push_line(vec![('H', dummy_quad()), ('i', dummy_quad())]);
        cached.push_line(vec![('W', dummy_quad()), ('o', dummy_quad()), ('r', dummy_quad()), ('l', dummy_quad()), ('d', dummy_quad())]);
        assert!(cached.has_text());

        let mut direct = PageText::new(false);
        direct.push_line(vec![('H', dummy_quad()), ('i', dummy_quad())]);
        direct.push_line(vec![('W', dummy_quad()), ('o', dummy_quad()), ('r', dummy_quad()), ('l', dummy_quad()), ('d', dummy_quad())]);

        let mut from_cached = PageText::new(false);
        cached.populate(&mut from_cached);

        assert_eq!(direct.has_text(), from_cached.has_text());
        assert_eq!(direct.search("world", 10), from_cached.search("world", 10));
        assert_eq!(direct.search("hi world", 10), from_cached.search("hi world", 10));
    }

    #[test]
    fn cache_respects_memory_limit() {
        let mut cache = TextCache::with_limit(1000);
        let mut page1 = CachedPageText::new();
        page1.push_line(vec![('a', dummy_quad())]);
        let size1 = page1.memory_bytes();
        assert!(size1 < 1000);

        cache.insert(0, page1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.bytes_used(), size1);
        assert!(cache.get(0).is_some());

        // A very large page that exceeds remaining capacity
        let mut big_page = CachedPageText::new();
        for _ in 0..100 {
            big_page.push_line(vec![('x', dummy_quad()); 20]);
        }
        let big_size = big_page.memory_bytes();
        assert!(cache.bytes_used() + big_size > 1000);

        cache.insert(1, big_page);
        // Exceeds limit: not inserted
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn cache_invalidation_and_clear() {
        let mut cache = TextCache::new();
        let mut page0 = CachedPageText::new();
        page0.push_line(vec![('A', dummy_quad())]);
        let mut page1 = CachedPageText::new();
        page1.push_line(vec![('B', dummy_quad())]);

        cache.insert(0, page0);
        cache.insert(1, page1);
        assert_eq!(cache.len(), 2);
        assert!(cache.bytes_used() > 0);

        cache.invalidate_page(0);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(0).is_none());
        assert!(cache.get(1).is_some());

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.bytes_used(), 0);
    }
}
