//! POC for ADR 0015 (proposed, #99): the Tesseract that MuPDF already builds into the worker, in
//! the worker's own sandbox, win32k disabled too. On Windows, mupdf-sys builds MuPDF's Visual
//! Studio solution, whose libmupdf.lib holds MuPDF's own copy of Tesseract (an Artifex fork, LSTM
//! only, no graphics) and of Leptonica (no image codecs); the worker links part of it today, for
//! MuPDF's OCR writer, and never calls it. This probe links the same library, through the `mupdf`
//! crate, and nothing else; only built with the `tesseract-poc` feature, like
//! examples/tesseract_probe.rs, which links a Tesseract built apart.
//!
//! MuPDF's own OCR entry point (`fz_new_ocr_device`) reads the language data from a file, which
//! the sandbox cannot open. So this probe calls Tesseract's C API with the data in memory, and
//! does first what MuPDF does before it uses Tesseract: MuPDF builds Leptonica to allocate
//! through a MuPDF context (`LEPTONICA_INTERCEPT_ALLOC`), set with `fz_set_leptonica_mem`.
//!
//! Usage: `mupdf_tesseract_probe <language>`, input and output as examples/common/tesseract.rs
//! says. Prints `error:` and what failed, if anything did.

// MuPDF's and Tesseract's C APIs; each unsafe block says why it is sound.
#![allow(unsafe_code)]

// Links MuPDF's libraries, which hold its Tesseract and Leptonica.
extern crate mupdf as _;

#[cfg(windows)]
#[path = "common/tesseract.rs"]
mod tesseract;

#[cfg(windows)]
fn main() {
    let language = std::env::args().nth(1).unwrap_or_default();
    if let Err(error) = tesseract::run(&language, memory::Leptonica::new) {
        println!("error:{error}");
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod memory {
    use std::ffi::{CStr, c_char, c_void};

    /// The MuPDF version a context is made for: `mupdf` =0.8.0 is MuPDF 1.27.2.
    const FZ_VERSION: &CStr = c"1.27.2";
    /// `FZ_STORE_DEFAULT`.
    const STORE: usize = 256 << 20;

    unsafe extern "C" {
        fn fz_new_context_imp(
            alloc: *const c_void,
            locks: *const c_void,
            max_store: usize,
            version: *const c_char,
        ) -> *mut c_void;
        fn fz_drop_context(ctx: *mut c_void);
        fn fz_set_leptonica_mem(ctx: *mut c_void);
        fn fz_clear_leptonica_mem(ctx: *mut c_void);
    }

    /// A MuPDF context that Leptonica allocates through while this lives.
    pub struct Leptonica(*mut c_void);

    impl Leptonica {
        pub fn new() -> Result<Self, String> {
            // SAFETY: MuPDF's default allocator and locks; the version is the one linked.
            let ctx = unsafe {
                fz_new_context_imp(
                    std::ptr::null(),
                    std::ptr::null(),
                    STORE,
                    FZ_VERSION.as_ptr(),
                )
            };
            if ctx.is_null() {
                return Err("memory: no MuPDF context".into());
            }
            // SAFETY: a live context. MuPDF throws (a longjmp) only if a context is set already,
            // which cannot happen: this single-threaded probe sets one once.
            unsafe { fz_set_leptonica_mem(ctx) };
            Ok(Self(ctx))
        }
    }

    impl Drop for Leptonica {
        fn drop(&mut self) {
            // SAFETY: set in `new` and cleared once, after Tesseract and so every Leptonica object
            // are gone (tesseract::run drops this last); then the context is dropped once.
            unsafe {
                fz_clear_leptonica_mem(self.0);
                fz_drop_context(self.0);
            }
        }
    }
}
