//! Opening an untrusted PDF with MuPDF, then everything the worker does with it (QA-03):
//! page sizes, rendering, the outline, links, the active-content scan and search. The sandbox
//! contains a crash (ADR 0008), but it does not replace finding it.
#![no_main]

use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;
use pdf_worker::scan::ScanBudget;

fuzz_target!(|data: &[u8]| {
    let Ok(document) = PdfDocument::from_bytes(data) else {
        return;
    };
    let Ok(pages) = document.page_count() else {
        return;
    };
    // The first pages are enough: they exercise the same code, and each run stays fast.
    for index in 0..pages.min(2) {
        let _ = document.page_size(index);
        // Small scales keep rasters tiny; 90° exercises the rotated path.
        let _ = document.render(index, 0.1, 90);
        let _ = document.page_links(index, 100);
        let _ = document.search_page(index, "a", false, 100);
    }
    let _ = document.outline(1_000, 64);
    let _ = document.has_outline();
    let _ = document.active_content(ScanBudget {
        max_objects: 100_000,
        max_time: Duration::from_secs(1),
    });
});
