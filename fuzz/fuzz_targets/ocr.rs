//! Fuzzing OCR (B2-10, ADR 0015, QA-03, #486, docs/security/fuzzing.md):
//! (a) Recognising fuzzed pictures with the bundled English language model within bounded
//!     dimensions, PPI, and pixel count. Fuzzed language data is not passed to Tesseract because
//!     Tesseract aborts the process via `ASSERT_HOST` when LSTM model components fail to
//!     deserialize, terminating the fuzzer instead of exploring recognition logic; `.traineddata`
//!     header validation is covered by the `language_data` target.
//! (b) Scanned pages: untrusted PDF, checked with `ocr_is_scan` on page 0,
//!     rendered to greyscale pixels within a bounded size, recognised with English data,
//!     and constructed into a text layer.
//! Both paths share a single thread-local `Recogniser` instance across iterations.
#![no_main]

use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::PdfDocument;
use pdf_worker::ocr::{MAX_OCR_SIDE_PX, Recogniser};
use pdf_worker::ocr_worker;

const ENG: &[u8] = include_bytes!("../../src-tauri/resources/tessdata/eng.traineddata");

/// Max pixels drawn for scanned page and picture fuzzing to keep execution within reasonable time bounds.
const MAX_FUZZ_SCAN_PIXELS: u64 = 4_000_000;

/// Header length for fuzzed pictures: 2 bytes width, 2 bytes height, 2 bytes PPI (all LE).
const PICTURE_HEADER_LEN: usize = 6;

thread_local! {
    static RECOGNISER: RefCell<Option<Recogniser>> = RefCell::new(Recogniser::new("eng", ENG).ok());
}

fn with_recogniser<R>(f: impl FnOnce(&mut Recogniser) -> R) -> Option<R> {
    RECOGNISER.with(|cell| {
        let mut guard = cell.borrow_mut();
        if guard.is_none() {
            *guard = Recogniser::new("eng", ENG).ok();
        }
        let recogniser = guard.as_mut()?;
        Some(f(recogniser))
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&kind, payload)) = data.split_first() else {
        return;
    };

    match kind {
        0 => {
            // (a) Fuzzed picture recognised with bundled English model
            if payload.len() < PICTURE_HEADER_LEN {
                return;
            }
            let width = u32::from(u16::from_le_bytes([payload[0], payload[1]]));
            let height = u32::from(u16::from_le_bytes([payload[2], payload[3]]));
            let ppi = u32::from(u16::from_le_bytes([payload[4], payload[5]]));

            if width == 0
                || height == 0
                || width > MAX_OCR_SIDE_PX
                || height > MAX_OCR_SIDE_PX
                || u64::from(width) * u64::from(height) > MAX_FUZZ_SCAN_PIXELS
                || !(1..=200).contains(&ppi)
            {
                return;
            }

            let needed = (u64::from(width) * u64::from(height)) as usize;
            let pixel_bytes = &payload[PICTURE_HEADER_LEN..];
            let mut grey_buf;
            let grey: &[u8] = if pixel_bytes.len() >= needed {
                &pixel_bytes[..needed]
            } else {
                grey_buf = Vec::with_capacity(needed);
                grey_buf.extend_from_slice(pixel_bytes);
                grey_buf.resize(needed, 255);
                &grey_buf
            };

            let cancel = AtomicBool::new(false);
            let _ = with_recogniser(|recogniser| {
                recogniser.recognise(
                    grey,
                    width,
                    height,
                    ppi,
                    Duration::from_millis(500),
                    &cancel,
                )
            });
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
            let cancel = AtomicBool::new(false);
            let lines = with_recogniser(|recogniser| {
                recogniser.recognise(
                    &picture.grey,
                    picture.width,
                    picture.height,
                    picture.ppi,
                    Duration::from_millis(500),
                    &cancel,
                )
            });
            if let Some(Ok(lines)) = lines {
                let _ = ocr_worker::page_text(&lines, geometry);
            }
        }
        _ => {}
    }
});
