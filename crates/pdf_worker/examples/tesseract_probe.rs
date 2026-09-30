//! POC for ADR 0015 (proposed, #99), option C: Tesseract in the worker's own sandbox, win32k
//! disabled too. tests/ocr_poc.rs starts this in the sandbox and sends it the language data and
//! a page image; it prints what it recognised. It is an example, never part of the worker or the
//! installer, and is only built with the `tesseract-poc` feature: it links the libraries that
//! scripts/ocr-poc/build-tesseract.ps1 builds (build.rs, `TESSERACT_POC_DIR`).
//!
//! Usage: `tesseract_probe <language>`, with on stdin the language's `.traineddata` (its length as
//! 4 bytes little-endian, then its bytes) and then a binary PGM (8-bit grey). Nothing is read from
//! a file: the sandbox could not open one anyway. Prints `line:` and the text of each line
//! recognised, or `error:` and what failed.

// The Tesseract C API; each unsafe block says why it is sound.
#![allow(unsafe_code)]

#[cfg(windows)]
fn main() {
    let language = std::env::args().nth(1).unwrap_or_default();
    if let Err(error) = tesseract::run(&language) {
        println!("error:{error}");
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod tesseract {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::io::Read;

    /// `TessOcrEngineMode::OEM_LSTM_ONLY` and `TessPageSegMode::PSM_AUTO` (capi.h).
    const OEM_LSTM_ONLY: c_int = 1;
    const PSM_AUTO: c_int = 3;

    unsafe extern "C" {
        fn TessBaseAPICreate() -> *mut c_void;
        fn TessBaseAPIDelete(handle: *mut c_void);
        #[allow(clippy::too_many_arguments)]
        fn TessBaseAPIInit5(
            handle: *mut c_void,
            data: *const c_char,
            data_size: c_int,
            language: *const c_char,
            mode: c_int,
            configs: *mut *mut c_char,
            configs_size: c_int,
            vars_vec: *mut *mut c_char,
            vars_values: *mut *mut c_char,
            vars_vec_size: usize,
            set_only_non_debug_params: c_int,
        ) -> c_int;
        fn TessBaseAPISetPageSegMode(handle: *mut c_void, mode: c_int);
        fn TessBaseAPISetImage(
            handle: *mut c_void,
            imagedata: *const u8,
            width: c_int,
            height: c_int,
            bytes_per_pixel: c_int,
            bytes_per_line: c_int,
        );
        fn TessBaseAPIGetUTF8Text(handle: *mut c_void) -> *mut c_char;
        fn TessDeleteText(text: *const c_char);
    }

    /// Owns a `TessBaseAPI`; deleted when dropped.
    struct Api(*mut c_void);

    impl Drop for Api {
        fn drop(&mut self) {
            // SAFETY: created by TessBaseAPICreate and deleted once.
            unsafe { TessBaseAPIDelete(self.0) };
        }
    }

    pub fn run(language: &str) -> Result<(), String> {
        let mut input = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut input)
            .map_err(|error| format!("input: {error}"))?;
        let length = input
            .get(..4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")) as usize)
            .ok_or("input: no language data")?;
        let data = input
            .get(4..4 + length)
            .ok_or("input: language data cut short")?;
        let (width, height, grey) = pgm(&input[4 + length..])?;

        let language = CString::new(language).map_err(|_| "language: not a name")?;
        // SAFETY: no arguments; the handle is owned by `api`.
        let api = Api(unsafe { TessBaseAPICreate() });
        if api.0.is_null() {
            return Err("recogniser: not created".into());
        }
        // SAFETY: `data` and `language` outlive the call, which copies what it keeps; no configs
        // or variables are passed.
        let initialised = unsafe {
            TessBaseAPIInit5(
                api.0,
                data.as_ptr().cast(),
                c_int::try_from(data.len()).map_err(|_| "language data too large")?,
                language.as_ptr(),
                OEM_LSTM_ONLY,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0,
            )
        };
        if initialised != 0 {
            return Err("recogniser: the language data was refused".into());
        }
        let (width, height) = (
            c_int::try_from(width).map_err(|_| "input: too wide")?,
            c_int::try_from(height).map_err(|_| "input: too tall")?,
        );
        // SAFETY: a live handle; `grey` holds `width` x `height` bytes, one per pixel, and
        // outlives the recognition below, which reads it.
        let text = unsafe {
            TessBaseAPISetPageSegMode(api.0, PSM_AUTO);
            TessBaseAPISetImage(api.0, grey.as_ptr(), width, height, 1, width);
            TessBaseAPIGetUTF8Text(api.0)
        };
        if text.is_null() {
            return Err("recognise: no result".into());
        }
        // SAFETY: a NUL-terminated string Tesseract allocated; read once, then freed once.
        let recognised = unsafe {
            let copy = CStr::from_ptr(text).to_string_lossy().into_owned();
            TessDeleteText(text);
            copy
        };
        for line in recognised.lines().filter(|line| !line.trim().is_empty()) {
            println!("line:{line}");
        }
        Ok(())
    }

    /// The size and pixels of a binary PGM: `P5`, width, height and 255, separated by
    /// whitespace, one whitespace byte, then a byte per pixel.
    fn pgm(data: &[u8]) -> Result<(u32, u32, &[u8]), String> {
        let mut fields = Vec::new();
        let mut at = 0;
        while fields.len() < 4 {
            while data.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let start = at;
            while data.get(at).is_some_and(|byte| !byte.is_ascii_whitespace()) {
                at += 1;
            }
            fields.push(std::str::from_utf8(&data[start..at]).unwrap_or(""));
        }
        let [magic, width, height, max] = fields[..] else {
            unreachable!("four fields");
        };
        let size = |field: &str| {
            field
                .parse::<u32>()
                .ok()
                .filter(|size| (1..=10_000).contains(size))
        };
        let (Some(width), Some(height)) = (size(width), size(height)) else {
            return Err("input: not a PGM image of at most 10,000 pixels a side".into());
        };
        if magic != "P5" || max != "255" {
            return Err("input: not an 8-bit binary PGM image".into());
        }
        let pixels = data
            .get(at + 1..)
            .filter(|pixels| pixels.len() as u64 == u64::from(width) * u64::from(height))
            .ok_or("input: the PGM image is cut short or too long")?;
        Ok((width, height, pixels))
    }
}
