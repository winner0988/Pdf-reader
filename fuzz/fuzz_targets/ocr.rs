//! Fuzzing OCR (B2-10, ADR 0015, docs/security/fuzzing.md): verifies that Tesseract is built in
//! on Linux and can initialise with language data.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pdf_worker::ocr::Recogniser;

const ENG: &[u8] = include_bytes!("../../src-tauri/resources/tessdata/eng.traineddata");

fuzz_target!(|_data: &[u8]| {
    let recogniser = Recogniser::new("eng", ENG).expect("Recogniser::new should succeed");
    drop(recogniser);
});
