//! Taking out of the pictures of an untrusted PDF what was written about them (B3-04): opening it,
//! reading each JPEG of it through its filters with MuPDF, cutting its segments, putting it back
//! and writing the copy out. A crash here is a document that kills its worker when the user asks
//! for a batch without EXIF.
#![no_main]

use std::io::sink;

use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;

fuzz_target!(|data: &[u8]| {
    let Ok(document) = PdfDocument::from_bytes(data) else {
        return;
    };
    let _ = document.exif_clean_copy(&mut sink());
});
