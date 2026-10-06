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

/// Largest document the worker opens, and largest file it writes when saving (ADR 0013).
pub const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

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

/// Maximum length of a document password (UTF-8 bytes, MVP-16). PDF itself uses at most 127.
pub const MAX_PASSWORD_BYTES: u32 = 1_024;

/// Maximum length of a search query (UTF-8 bytes).
pub const MAX_QUERY_BYTES: u32 = 1_024;

/// Search stops after this many hits and reports `truncated`.
pub const MAX_SEARCH_HITS: u32 = 10_000;

/// Maximum quads for one hit (a hit spanning several lines has one quad per line).
pub const MAX_QUADS_PER_HIT: u32 = 64;

/// Most characters of one page's text sent for selecting and copying (MVP-15). A normal page
/// has a few thousand; a page with more is cut off and flagged.
pub const MAX_PAGE_TEXT_CHARS: u32 = 100_000;

/// Maximum length of an error message or detail string (UTF-8 bytes).
pub const MAX_ERROR_MESSAGE_BYTES: u32 = 1_024;

/// Maximum length of a document display name (file name only, never a path).
pub const MAX_DISPLAY_NAME_BYTES: u32 = 1_024;

/// Maximum number of tabs in the window (MVP-14, ADR 0012). Each open document has its own
/// worker process, so this also bounds the number of workers.
pub const MAX_TABS: u32 = 20;

/// Maximum number of recently opened files kept and listed (#73, spec §3).
pub const MAX_RECENT_FILES: u32 = 20;

/// Most edits kept for undo between two saves (B2-05); another one is refused until the
/// document is saved. Also bounds the edits of one `WorkerRequest::Revert`.
pub const MAX_UNDO_EDITS: u32 = 1_000;

/// Maximum number of pages one export writes (B2-04).
pub const MAX_EXPORT_PAGES: u32 = 1_000;

/// Most quadrilaterals one highlighter mark covers (B2-07), on all its pages: one per line of
/// selected text.
pub const MAX_ANNOTATION_QUADS: u32 = 1_000;

/// Most pages one highlighter mark spans (B2-07).
pub const MAX_HIGHLIGHT_PAGES: u32 = 100;

/// Longest text of a note (B2-07), in UTF-8 bytes; also the most of a note's text the worker
/// reports.
pub const MAX_NOTE_TEXT_BYTES: u32 = 4_096;

/// Most strokes one pen drawing has (B2-08): a stroke is one press of the pen to its release.
pub const MAX_INK_STROKES: u32 = 256;

/// Most points one pen drawing has, on all its strokes (B2-08). The page thins what the pen
/// reports (a point every few pixels is plenty), so a drawing seldom has a tenth of this.
pub const MAX_INK_POINTS: u32 = 20_000;

/// Largest picture file a custom stamp is made from (B2-08), in bytes; the worker reads no more.
pub const MAX_STAMP_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// Most pixels such a picture may have (B2-08). It is decoded in the worker, four bytes a pixel
/// at most; what is larger is refused before any pixel is decoded.
pub const MAX_STAMP_SOURCE_PIXELS: u64 = 16 * 1024 * 1024;

/// Longest side such a picture may have (B2-08), in pixels.
pub const MAX_STAMP_SOURCE_SIDE_PX: u32 = 8_192;

/// The picture a stamp is made of is shrunk until neither side is longer than this (B2-08), in
/// pixels: a stamp is a few centimeters wide.
pub const MAX_STAMP_SIDE_PX: u32 = 1_024;

/// Most bytes of the PNG file of a stamp picture (B2-08), below the frame limit; also what a
/// stamp edit carries.
pub const MAX_STAMP_PNG_BYTES: usize = 2 * 1024 * 1024;

/// Least a stamp or any annotation the app moves or resizes may measure on each side (B2-08), in
/// points: smaller than that it cannot be seen or grabbed.
pub const MIN_ANNOTATION_SIDE_PT: f32 = 8.0;

/// Most annotations reported for one page (B2-07); the rest are not listed.
pub const MAX_ANNOTATIONS_PER_PAGE: u32 = 2_000;

/// Most form fields reported for one page (B2-09); the rest are not listed.
pub const MAX_FIELDS_PER_PAGE: u32 = 5_000;

/// Longest value of a form field (B2-09), in UTF-8 bytes; also the most of one the worker
/// reports (a field with more is shown but cannot be changed).
pub const MAX_FIELD_VALUE_BYTES: u32 = 16 * 1024;

/// Most choices of a combo box or list box (B2-09).
pub const MAX_FIELD_OPTIONS: u32 = 1_000;

/// Maximum size of one exported PNG page (B2-04); below the frame limit.
pub const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;

/// Maximum size of one exported JPEG page (#111); below the frame limit. A page at the largest
/// raster the worker makes stays well under it at the export quality.
pub const MAX_JPEG_BYTES: usize = 64 * 1024 * 1024;
