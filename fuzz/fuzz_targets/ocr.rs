//! Fuzzing OCR (B2-10, ADR 0015, QA-03, docs/security/fuzzing.md):
//! (a) Language data (.traineddata) from memory, checked with `check_language_data`,
//!     loaded into `Recogniser`, and recognising a small fixed picture.
//! (b) Scanned pages: untrusted PDF, checked with `ocr_is_scan` on page 0,
//!     rendered to greyscale pixels within a bounded size, recognised with English data,
//!     and constructed into a text layer.
#![no_main]

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ipc_contract::ocr::check_language_data;
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;
use pdf_worker::ocr::Recogniser;
use pdf_worker::ocr_worker;

const ENG: &[u8] = include_bytes!("../../src-tauri/resources/tessdata/eng.traineddata");

/// A small fixed greyscale picture for exercising recognition on valid language models.
const FIXED_WIDTH: u32 = 64;
const FIXED_HEIGHT: u32 = 32;

fn fixed_image() -> Vec<u8> {
    let mut grey = vec![255u8; (FIXED_WIDTH * FIXED_HEIGHT) as usize];
    // Draw simple marks/bars
    for y in 10..22 {
        for x in 10..15 {
            grey[(y * FIXED_WIDTH + x) as usize] = 0;
        }
        for x in 25..30 {
            grey[(y * FIXED_WIDTH + x) as usize] = 0;
        }
        for x in 40..45 {
            grey[(y * FIXED_WIDTH + x) as usize] = 0;
        }
    }
    grey
}

/// Max pixels drawn for scanned page fuzzing to keep execution within reasonable time bounds.
const MAX_FUZZ_SCAN_PIXELS: u64 = 4_000_000;

fuzz_target!(|data: &[u8]| {
    let Some((&kind, payload)) = data.split_first() else {
        return;
    };

    match kind {
        0 => {
            // (a) Language data
            if check_language_data(payload).is_err() {
                return;
            }
            let Ok(mut recogniser) = Recogniser::new("eng", payload) else {
                return;
            };
            let grey = fixed_image();
            let cancel = AtomicBool::new(false);
            let _ = recogniser.recognise(
                &grey,
                FIXED_WIDTH,
                FIXED_HEIGHT,
                72,
                Duration::from_millis(500),
                &cancel,
            );
        }
        1 => {
            // (b) Scanned page
            let Ok(mut doc) = PdfDocument::from_bytes(payload) else {
                return;
            };
            let Ok(is_scan) = doc.ocr_is_scan(0) else {
                return;
            };
            if !is_scan {
                return;
            }
            let Ok((picture, geometry, _)) = doc.ocr_picture(0) else {
                return;
            };
            if u64::from(picture.width) * u64::from(picture.height) > MAX_FUZZ_SCAN_PIXELS
                || picture.grey.len() as u64 != u64::from(picture.width) * u64::from(picture.height)
            {
                return;
            }
            let Ok(mut recogniser) = Recogniser::new("eng", ENG) else {
                return;
            };
            let cancel = AtomicBool::new(false);
            if let Ok(lines) = recogniser.recognise(
                &picture.grey,
                picture.width,
                picture.height,
                picture.ppi,
                Duration::from_millis(500),
                &cancel,
            ) {
                let _ = ocr_worker::page_text(&lines, geometry);
            }
        }
        _ => {}
    }
});
