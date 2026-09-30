//! POC for ADR 0015 (proposed, #99): Tesseract 5.5.3 built apart, in the worker's own sandbox,
//! win32k disabled too. tests/ocr_poc.rs starts this in the sandbox and sends it the language data
//! and a page image; it prints what it recognised. It is an example, never part of the worker or
//! the installer, and is only built with the `tesseract-poc` feature: it links the libraries that
//! scripts/ocr-poc/build-tesseract.ps1 builds (found through build.rs, `TESSERACT_POC_DIR`).
//! examples/mupdf_tesseract_probe.rs does the same with the copy of Tesseract inside MuPDF.
//!
//! Usage: `tesseract_probe <language>`, input and output as examples/common/tesseract.rs says.
//! Prints `error:` and what failed, if anything did.

// The Tesseract C API; each unsafe block says why it is sound.
#![allow(unsafe_code)]

#[cfg(windows)]
#[path = "common/tesseract.rs"]
mod tesseract;

// Only this probe links the Tesseract that is built apart.
#[cfg(windows)]
#[link(name = "tesseract55", kind = "static")]
unsafe extern "C" {}

#[cfg(windows)]
#[link(name = "leptonica-1.87.0", kind = "static")]
unsafe extern "C" {}

#[cfg(windows)]
fn main() {
    let language = std::env::args().nth(1).unwrap_or_default();
    if let Err(error) = tesseract::run(&language, || Ok(())) {
        println!("error:{error}");
    }
}

#[cfg(not(windows))]
fn main() {}
