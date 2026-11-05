//! Decoding a picture for a custom stamp (B2-08, ADR 0018) or image watermark (B3-04): verifying
//! its header, decoding its pixels with MuPDF, shrinking it if large, encoding it again as a
//! PNG with only pixels, placing it on a page as a stamp annotation and drawing the page. The
//! sandbox contains a crash (ADR 0008), but it does not replace finding it.
#![no_main]

use ipc_contract::types::Rect;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::{PdfDocument, prepare_stamp_picture};

/// A page to set the stamp on (Letter size).
const PAGE: &[u8] = include_bytes!("../../tests/corpus/benign/single-page.pdf");

fuzz_target!(|data: &[u8]| {
    let Ok(picture) = prepare_stamp_picture(data) else {
        return;
    };
    let Ok(mut document) = PdfDocument::from_bytes(PAGE) else {
        return;
    };
    let rect = Rect {
        x0: 72.0,
        y0: 72.0,
        x1: 200.0,
        y1: 200.0,
    };
    if document.add_image_stamp(0, rect, &picture.png).is_err() {
        return;
    }
    // Drawn small: MuPDF decodes and renders the stamp picture.
    let _ = document.render(0, 0.5, 0);
});
