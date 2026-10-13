//! Encoding the pictures of an untrusted PDF again (B3-01): opening it, writing a copy of it
//! whose pictures are decoded by MuPDF, shrunk, encoded as JPEG and put back, and writing that
//! out. The pictures are the part of the document the worker has not looked into before this,
//! and a crash here is a document that kills its worker when the user asks for a smaller copy.
#![no_main]

use std::io::sink;
use std::time::Duration;

use ipc_contract::types::ImageQuality;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;

fuzz_target!(|data: &[u8]| {
    let Ok(document) = PdfDocument::from_bytes(data) else {
        return;
    };
    // The lowest quality and a small limit keep each run fast; a short budget bounds the rest.
    let _ = document.compressed_copy_within(
        ImageQuality::Low,
        Some(256),
        Duration::from_secs(2),
        &mut sink(),
    );
});
