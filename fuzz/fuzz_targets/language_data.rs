//! Validation of Tesseract language data (.traineddata) headers (B2-10, #429, docs/security/fuzzing.md).
//! Every input must be rejected or accepted cleanly: no panic, no unbounded allocation.
#![no_main]

use ipc_contract::ocr::check_language_data;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = check_language_data(data);
});
