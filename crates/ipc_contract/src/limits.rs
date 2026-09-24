//! Upper bounds for everything that crosses a process boundary.
//!
//! The worker parses untrusted PDFs, so the main process treats its output as untrusted too:
//! every count, size and string is checked against these limits (see [`crate::validate`]).
//! The frontend receives the same numbers through the generated `LIMITS` constant.

/// Largest frame (length prefix excluded) either side will read or write.
/// Sized for one maximum raster plus message overhead.
pub const MAX_FRAME_BYTES: usize = 80 * 1024 * 1024;

/// Maximum number of pages in a document.
pub const MAX_PAGE_COUNT: u32 = 100_000;

/// Sanity bound for a page side in PDF points. Memory is bounded by the raster limits below,
/// this only rejects absurd values (the PDF spec allows 14 400 units, scaled by `UserUnit`).
pub const MAX_PAGE_SIDE_PT: f32 = 1_000_000.0;

/// Render scale bounds (1.0 = 72 dpi, one PDF point per pixel).
pub const MIN_RENDER_SCALE: f32 = 0.01;
pub const MAX_RENDER_SCALE: f32 = 64.0;

/// Largest raster side in pixels.
pub const MAX_RASTER_SIDE_PX: u32 = 8192;

/// Largest raster area in pixels (4096 x 4096, i.e. 64 MiB of RGBA).
pub const MAX_RASTER_PIXELS: u32 = 4096 * 4096;

/// Outline limits. Longer outlines are truncated by the worker and flagged as such.
pub const MAX_OUTLINE_ITEMS: u32 = 10_000;
pub const MAX_OUTLINE_DEPTH: u16 = 64;

/// Maximum links reported for one page.
pub const MAX_LINKS_PER_PAGE: u32 = 2_000;

/// Maximum length of a URI taken from a PDF (UTF-8 bytes).
pub const MAX_URI_BYTES: u32 = 32_768;

/// Maximum length of short text taken from a PDF: outline titles, blocked action targets.
pub const MAX_TEXT_BYTES: u32 = 1_024;

/// Maximum length of a search query (UTF-8 bytes).
pub const MAX_QUERY_BYTES: u32 = 1_024;

/// Search stops after this many hits and reports `truncated`.
pub const MAX_SEARCH_HITS: u32 = 10_000;

/// Maximum quads for one hit (a hit spanning several lines has one quad per line).
pub const MAX_QUADS_PER_HIT: u32 = 64;

/// Maximum length of an error message or detail string (UTF-8 bytes).
pub const MAX_ERROR_MESSAGE_BYTES: u32 = 1_024;

/// Maximum length of a document display name (file name only, never a path).
pub const MAX_DISPLAY_NAME_BYTES: u32 = 1_024;

/// Maximum number of tabs in the window (MVP-14, ADR 0012). Each open document has its own
/// worker process, so this also bounds the number of workers.
pub const MAX_TABS: u32 = 20;
