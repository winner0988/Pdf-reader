//! Recognising the text of scanned pages (B2-10, ADR 0015) with the Tesseract that MuPDF builds
//! into the worker (an Artifex fork: LSTM only, no graphics), in the worker's own sandbox.
//! Nothing here opens a file or touches the network: the language data is given as bytes by the
//! main process and loaded from memory, and the page's pixels never leave the process.
//!
//! This is the one module that calls Tesseract's C API and the four functions of MuPDF that make
//! Leptonica (which Tesseract uses) allocate through a MuPDF context. All the `unsafe` of OCR is
//! here; each block says why it is sound.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ipc_contract::limits::MAX_PAGE_TEXT_CHARS;
use thiserror::Error;

/// `TessOcrEngineMode::OEM_LSTM_ONLY`, `TessPageSegMode::PSM_AUTO` and the iterator levels
/// `RIL_TEXTLINE`, `RIL_WORD` and `RIL_SYMBOL` (capi.h and publictypes.h).
const OEM_LSTM_ONLY: c_int = 1;
const PSM_AUTO: c_int = 3;
const RIL_TEXTLINE: c_int = 2;
const RIL_WORD: c_int = 3;
const RIL_SYMBOL: c_int = 4;

/// The MuPDF version a context is made for: `mupdf` =0.8.0 is MuPDF 1.27.2. A MuPDF that is not
/// this one refuses the context (the version check of `fz_new_context_imp`), and the tests of
/// this module then fail: that is how a MuPDF upgrade is told to look at OCR again (ADR 0015).
const FZ_VERSION: &CStr = c"1.27.2";
/// How much the MuPDF context that Leptonica allocates through may keep (`FZ_STORE_DEFAULT`).
const STORE_BYTES: usize = 256 << 20;

/// The most pixels a picture is recognised at.
pub const MAX_OCR_PIXELS: u64 = 40_000_000;
/// The longest side of such a picture, in pixels.
pub const MAX_OCR_SIDE_PX: u32 = 12_000;
/// The most characters taken from one picture: a page of noise can have far more than any page of
/// text, which the text layer cuts off anyway (twice what it keeps shows that it was).
const MAX_RECOGNISED_CHARS: usize = 2 * MAX_PAGE_TEXT_CHARS as usize;

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("another recogniser exists: there is one at a time")]
    Busy,
    #[error("the recogniser could not be made: {0}")]
    Init(&'static str),
    #[error("the language data was refused")]
    LanguageData,
    #[error("the picture is empty or too large")]
    BadPicture,
    #[error("the recogniser gave no result")]
    NoResult,
    #[error("the recognition stopped before it was done: the time allowed ended")]
    Stopped,
    #[error("the recognition was cancelled")]
    Cancelled,
}

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
    fn TessBaseAPISetSourceResolution(handle: *mut c_void, ppi: c_int);
    fn TessBaseAPIRecognize(handle: *mut c_void, monitor: *mut c_void) -> c_int;
    fn TessBaseAPIGetIterator(handle: *mut c_void) -> *mut c_void;
    fn TessBaseAPIClear(handle: *mut c_void);
    fn TessResultIteratorDelete(iterator: *mut c_void);
    fn TessResultIteratorGetPageIterator(iterator: *mut c_void) -> *mut c_void;
    fn TessResultIteratorGetUTF8Text(iterator: *const c_void, level: c_int) -> *mut c_char;
    fn TessResultIteratorNext(iterator: *mut c_void, level: c_int) -> c_int;
    fn TessPageIteratorIsAtBeginningOf(iterator: *const c_void, level: c_int) -> c_int;
    fn TessPageIteratorBoundingBox(
        iterator: *const c_void,
        level: c_int,
        left: *mut c_int,
        top: *mut c_int,
        right: *mut c_int,
        bottom: *mut c_int,
    ) -> c_int;
    fn TessDeleteText(text: *const c_char);
    fn TessMonitorCreate() -> *mut c_void;
    fn TessMonitorDelete(monitor: *mut c_void);
    fn TessMonitorSetDeadlineMSecs(monitor: *mut c_void, deadline: c_int);
    fn TessMonitorSetCancelFunc(
        monitor: *mut c_void,
        cancel: extern "C" fn(*mut c_void, c_int) -> bool,
    );
    fn TessMonitorSetCancelThis(monitor: *mut c_void, this: *mut c_void);

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

/// Lets the calling thread give way to the others: recognising a page is background work, and
/// the thread that answers the user's requests (renders above all) goes first.
pub fn lower_thread_priority() {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        // SAFETY: no pointers; the handle is the pseudo handle of the calling thread, which is
        // always valid and needs no closing. Failing leaves the priority as it was.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
    }
}

/// Whether a recogniser exists: MuPDF remembers the context Leptonica allocates through in one
/// global variable, so there is one at a time.
static IN_USE: AtomicBool = AtomicBool::new(false);

/// Lets go of `IN_USE` when dropped.
struct InUse;

impl Drop for InUse {
    fn drop(&mut self) {
        IN_USE.store(false, Ordering::Release);
    }
}

/// A MuPDF context that Leptonica allocates through while this lives.
struct LeptonicaMemory(*mut c_void);

impl LeptonicaMemory {
    fn new() -> Result<Self, OcrError> {
        // SAFETY: MuPDF's default allocator and locks, and the version that is linked.
        let ctx = unsafe {
            fz_new_context_imp(
                std::ptr::null(),
                std::ptr::null(),
                STORE_BYTES,
                FZ_VERSION.as_ptr(),
            )
        };
        if ctx.is_null() {
            return Err(OcrError::Init("no MuPDF context"));
        }
        // SAFETY: a live context, and no other recogniser has set one (`IN_USE`).
        unsafe { fz_set_leptonica_mem(ctx) };
        Ok(Self(ctx))
    }
}

impl Drop for LeptonicaMemory {
    fn drop(&mut self) {
        // SAFETY: set in `new` and cleared once, after Tesseract and so every Leptonica object
        // are gone (`Recogniser` drops its fields in the order that says so); the context is
        // dropped once.
        unsafe {
            fz_clear_leptonica_mem(self.0);
            fz_drop_context(self.0);
        }
    }
}

/// Owns a `TessBaseAPI`.
struct Api(*mut c_void);

impl Drop for Api {
    fn drop(&mut self) {
        // SAFETY: created by TessBaseAPICreate and deleted once.
        unsafe { TessBaseAPIDelete(self.0) };
    }
}

/// Owns a `TessResultIterator`.
struct Results(*mut c_void);

impl Drop for Results {
    fn drop(&mut self) {
        // SAFETY: created by TessBaseAPIGetIterator and deleted once, before the picture is
        // cleared.
        unsafe { TessResultIteratorDelete(self.0) };
    }
}

/// Owns an `ETEXT_DESC`, which says when a recognition must stop.
struct Monitor(*mut c_void);

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: created by TessMonitorCreate and deleted once.
        unsafe { TessMonitorDelete(self.0) };
    }
}

/// One character recognised and where it is in the picture: pixels from its top left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrChar {
    pub ch: char,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// A line of characters recognised, in reading order, spaces between its words included.
pub type OcrLine = Vec<OcrChar>;

/// Tesseract with one language's data, ready to recognise pictures one after another. There is
/// one at a time in a process.
pub struct Recogniser {
    // The fields drop in this order: Tesseract first, then the memory it used.
    api: Api,
    _memory: LeptonicaMemory,
    _in_use: InUse,
}

impl Recogniser {
    /// Loads `data`, the `.traineddata` of `language` ("eng", "chi_tra"...), from memory.
    pub fn new(language: &str, data: &[u8]) -> Result<Self, OcrError> {
        // Tesseract reads wherever the data says, and takes an empty buffer for a folder's name.
        if !ipc_contract::ocr::is_language_name(language)
            || ipc_contract::ocr::check_language_data(data).is_err()
        {
            return Err(OcrError::LanguageData);
        }
        if IN_USE.swap(true, Ordering::Acquire) {
            return Err(OcrError::Busy);
        }
        let in_use = InUse;
        let memory = LeptonicaMemory::new()?;
        let language = CString::new(language).map_err(|_| OcrError::Init("not a language name"))?;
        let size = c_int::try_from(data.len()).map_err(|_| OcrError::LanguageData)?;
        // SAFETY: no arguments; the handle is owned by `api` from here on.
        let handle = unsafe { TessBaseAPICreate() };
        if handle.is_null() {
            return Err(OcrError::Init("no recogniser"));
        }
        let api = Api(handle);
        // SAFETY: `data` and `language` outlive the call, which copies what it keeps; no configs
        // or variables are passed.
        let initialised = unsafe {
            TessBaseAPIInit5(
                api.0,
                data.as_ptr().cast(),
                size,
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
            return Err(OcrError::LanguageData);
        }
        // SAFETY: a live handle.
        unsafe { TessBaseAPISetPageSegMode(api.0, PSM_AUTO) };
        Ok(Self {
            api,
            _memory: memory,
            _in_use: in_use,
        })
    }
}

impl Recogniser {
    /// Recognises the picture `grey` (8 bits a pixel, `width` x `height`, a row after another with
    /// no padding), drawn at `ppi` pixels an inch, in at most `time`: its lines, with the pixels
    /// of every character. Another thread sets `cancel` to stop it early.
    pub fn recognise(
        &mut self,
        grey: &[u8],
        width: u32,
        height: u32,
        ppi: u32,
        time: Duration,
        cancel: &AtomicBool,
    ) -> Result<Vec<OcrLine>, OcrError> {
        if width == 0
            || height == 0
            || width > MAX_OCR_SIDE_PX
            || height > MAX_OCR_SIDE_PX
            || u64::from(width) * u64::from(height) > MAX_OCR_PIXELS
            || grey.len() as u64 != u64::from(width) * u64::from(height)
        {
            return Err(OcrError::BadPicture);
        }
        let w = c_int::try_from(width).map_err(|_| OcrError::BadPicture)?;
        let h = c_int::try_from(height).map_err(|_| OcrError::BadPicture)?;
        let deadline = c_int::try_from(time.as_millis()).unwrap_or(c_int::MAX);
        // SAFETY: a live handle; `grey` holds `w` x `h` bytes, one per pixel, and outlives the
        // recognition below, which reads it; the picture is cleared before this returns, so
        // nothing keeps the pointer.
        unsafe {
            TessBaseAPISetImage(self.api.0, grey.as_ptr(), w, h, 1, w);
            if let Ok(ppi) = c_int::try_from(ppi) {
                TessBaseAPISetSourceResolution(self.api.0, ppi);
            }
        }
        let lines = self.read_lines(deadline, cancel);
        // SAFETY: a live handle; frees the picture's data and the results (the iterator over
        // them was dropped in `read_lines`).
        unsafe { TessBaseAPIClear(self.api.0) };
        lines
    }

    fn read_lines(
        &mut self,
        deadline: c_int,
        cancel: &AtomicBool,
    ) -> Result<Vec<OcrLine>, OcrError> {
        // SAFETY: no arguments; the monitor is owned by `monitor`.
        let raw = unsafe { TessMonitorCreate() };
        if raw.is_null() {
            return Err(OcrError::Init("no monitor"));
        }
        let monitor = Monitor(raw);
        // SAFETY: live handles; the deadline and the cancel flag are read while the recognition
        // runs, which `monitor` outlives, and `cancel` outlives both. `cancelled` only reads the
        // flag through a shared reference.
        let recognised = unsafe {
            TessMonitorSetDeadlineMSecs(monitor.0, deadline);
            TessMonitorSetCancelFunc(monitor.0, cancelled);
            TessMonitorSetCancelThis(monitor.0, std::ptr::from_ref(cancel).cast_mut().cast());
            TessBaseAPIRecognize(self.api.0, monitor.0)
        };
        if recognised != 0 {
            return Err(if cancel.load(Ordering::Relaxed) {
                OcrError::Cancelled
            } else {
                OcrError::Stopped
            });
        }
        // SAFETY: a live handle that has recognised a picture.
        let raw = unsafe { TessBaseAPIGetIterator(self.api.0) };
        if raw.is_null() {
            // Nothing on the page.
            return Ok(Vec::new());
        }
        let results = Results(raw);
        // SAFETY: a live result iterator; its page iterator is the same object, not deleted
        // apart from it.
        let page = unsafe { TessResultIteratorGetPageIterator(results.0) };
        if page.is_null() {
            return Err(OcrError::NoResult);
        }
        let mut lines: Vec<OcrLine> = Vec::new();
        let mut line: OcrLine = Vec::new();
        let mut chars = 0;
        loop {
            // SAFETY: live iterators, at an element (`Next` below said so).
            let (new_line, new_word) = unsafe {
                (
                    TessPageIteratorIsAtBeginningOf(page, RIL_TEXTLINE) != 0,
                    TessPageIteratorIsAtBeginningOf(page, RIL_WORD) != 0,
                )
            };
            if new_line && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let line_so_far = line.len();
            if let Some((text, rect)) = symbol(results.0, page) {
                if new_word
                    && let Some(&before) = line.last()
                    && let Some(first) = text.chars().next()
                    && let Some(space) = space_between(before, first, rect)
                {
                    line.push(space);
                }
                spread(&text, rect, &mut line);
            }
            chars += line.len() - line_so_far;
            // SAFETY: a live result iterator.
            if chars >= MAX_RECOGNISED_CHARS
                || unsafe { TessResultIteratorNext(results.0, RIL_SYMBOL) } == 0
            {
                break;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        Ok(lines)
    }
}

type Rect = (i32, i32, i32, i32);

/// Tesseract asks now and then (every word) whether to go on: not once `this`, the flag that was
/// given to `recognise`, is set.
extern "C" fn cancelled(this: *mut c_void, _words: c_int) -> bool {
    // SAFETY: `this` is the `&AtomicBool` that `read_lines` set, which outlives the recognition
    // that calls this; only a shared reference is made.
    unsafe { &*this.cast::<AtomicBool>() }.load(Ordering::Relaxed)
}

/// The text and the box (left, top, right, bottom) of the symbol the iterator is at.
fn symbol(results: *mut c_void, page: *mut c_void) -> Option<(String, Rect)> {
    // SAFETY: live iterators; the text is a NUL-terminated string Tesseract allocated, read once
    // and freed once.
    let text = unsafe {
        let raw = TessResultIteratorGetUTF8Text(results, RIL_SYMBOL);
        if raw.is_null() {
            return None;
        }
        let copy = CStr::from_ptr(raw).to_string_lossy().into_owned();
        TessDeleteText(raw);
        copy
    };
    let (mut left, mut top, mut right, mut bottom) = (0, 0, 0, 0);
    // SAFETY: a live page iterator and four integers of this frame.
    let found = unsafe {
        TessPageIteratorBoundingBox(
            page,
            RIL_SYMBOL,
            &mut left,
            &mut top,
            &mut right,
            &mut bottom,
        )
    };
    (found != 0 && right >= left && bottom >= top).then_some((text, (left, top, right, bottom)))
}

/// The space between the character `before` and the first one of the next word, `first`, whose
/// box is `next`: as wide as the gap. Tesseract breaks Chinese, Japanese and Korean text into
/// "words" of a few characters, which are not apart: where one side is such a character, there is
/// a space only if the gap is as wide as a quarter of the line.
fn space_between(before: OcrChar, first: char, next: Rect) -> Option<OcrChar> {
    let height = (before.bottom - before.top).max(next.3 - next.1);
    let gap = next.0.saturating_sub(before.right);
    if (is_cjk(before.ch) || is_cjk(first)) && i64::from(gap) * 4 < i64::from(height) {
        return None;
    }
    Some(OcrChar {
        ch: ' ',
        left: before.right,
        top: before.top.min(next.1),
        right: next.0.max(before.right),
        bottom: before.bottom.max(next.3),
    })
}

/// Whether `c` is Chinese, Japanese or Korean script, or the punctuation written with it:
/// text in these is not spaced between words.
fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x11FF       // Hangul Jamo
        | 0x2E80..=0x2FDF     // CJK radicals
        | 0x3000..=0x30FF     // CJK symbols and punctuation, Hiragana, Katakana
        | 0x3100..=0x312F     // Bopomofo
        | 0x3130..=0x318F     // Hangul compatibility Jamo
        | 0x31A0..=0x31FF     // Bopomofo extended, CJK strokes, Katakana extensions
        | 0x3400..=0x4DBF     // CJK extension A
        | 0x4E00..=0x9FFF     // CJK unified ideographs
        | 0xA960..=0xA97F     // Hangul Jamo extended A
        | 0xAC00..=0xD7FF     // Hangul syllables and Jamo extended B
        | 0xF900..=0xFAFF     // CJK compatibility ideographs
        | 0xFE30..=0xFE4F     // CJK compatibility forms
        | 0xFF00..=0xFFEF     // Halfwidth and fullwidth forms
        | 0x20000..=0x2FA1F   // CJK extensions B to F, compatibility supplement
        | 0x30000..=0x3134F   // CJK extension G
    )
}

/// Puts the characters of `text` into `line`, sharing the box among them (a symbol may be more
/// than one character: a ligature).
fn spread(text: &str, (left, top, right, bottom): Rect, line: &mut OcrLine) {
    let chars: Vec<char> = text.chars().collect();
    let count = i64::try_from(chars.len()).unwrap_or(i64::MAX).max(1);
    let width = i64::from(right) - i64::from(left);
    for (index, ch) in chars.into_iter().enumerate() {
        let index = i64::try_from(index).unwrap_or(0);
        let from = i64::from(left) + width * index / count;
        let to = i64::from(left) + width * (index + 1) / count;
        line.push(OcrChar {
            ch,
            left: i32::try_from(from).unwrap_or(left),
            top,
            right: i32::try_from(to).unwrap_or(right),
            bottom,
        });
    }
}

/// Recognisers are made one at a time in a process and tests run side by side: the ones that
/// make one take turns.
#[cfg(test)]
pub(crate) fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TURN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};

    use mupdf::{Colorspace, Document, Matrix};

    use super::*;

    const DPI: u32 = 200;
    const TIME: Duration = Duration::from_secs(120);

    /// The language data the installer ships (src-tauri/resources/tessdata).
    pub(crate) fn traineddata(language: &str) -> Vec<u8> {
        let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/resources/tessdata")
            .join(format!("{language}.traineddata"));
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The first page of the corpus file `name`, in grey at [`DPI`], without its annotations:
    /// what the worker gives the recogniser.
    pub(crate) fn grey_page(name: &str) -> (Vec<u8>, u32, u32) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        let pdf = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let doc = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let scale = DPI as f32 / 72.0;
        let pixmap = doc
            .load_page(0)
            .expect("page")
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_gray(),
                false,
                false,
            )
            .expect("render");
        let (width, height) = (pixmap.width(), pixmap.height());
        let stride = usize::try_from(pixmap.stride()).expect("stride");
        let mut grey = Vec::with_capacity(width as usize * height as usize);
        for row in pixmap.samples().chunks_exact(stride) {
            grey.extend_from_slice(&row[..width as usize]);
        }
        (grey, width, height)
    }

    fn text_of(lines: &[OcrLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.iter().map(|c| c.ch).collect())
            .collect()
    }

    #[test]
    fn english_is_recognised_with_a_box_for_every_character() {
        let _turn = one_at_a_time();
        let (grey, width, height) = grey_page("benign/mixed-text-zh-en.pdf");
        let mut recogniser = Recogniser::new("eng", &traineddata("eng")).expect("english");
        let lines = recogniser
            .recognise(&grey, width, height, DPI, TIME, &AtomicBool::new(false))
            .expect("recognise");
        let text = text_of(&lines);
        assert!(
            text.contains(&"Privacy-first PDF Reader".to_owned()),
            "{text:?}"
        );
        for line in &lines {
            for c in line {
                assert!(c.left <= c.right && c.top <= c.bottom, "{c:?}");
                assert!(
                    c.left >= 0
                        && c.top >= 0
                        && c.right <= width as i32
                        && c.bottom <= height as i32,
                    "{c:?} is outside the {width} x {height} picture"
                );
            }
            // Left to right along a line.
            assert!(
                line.windows(2).all(|pair| pair[0].left <= pair[1].left),
                "{line:?}"
            );
        }
    }

    #[test]
    fn chinese_is_recognised_with_the_chinese_data() {
        let _turn = one_at_a_time();
        let (grey, width, height) = grey_page("benign/mixed-text-zh-en.pdf");
        let mut recogniser = Recogniser::new("chi_tra", &traineddata("chi_tra")).expect("chinese");
        let lines = recogniser
            .recognise(&grey, width, height, DPI, TIME, &AtomicBool::new(false))
            .expect("recognise");
        // The engine may put a space between Chinese characters.
        let text: Vec<String> = text_of(&lines)
            .iter()
            .map(|line| line.replace(' ', ""))
            .collect();
        assert!(
            text.iter().any(|line| line.contains("隱私優先的PDF閱讀器")),
            "{text:?}"
        );
    }

    #[test]
    fn a_blank_picture_has_no_lines() {
        let _turn = one_at_a_time();
        let mut recogniser = Recogniser::new("eng", &traineddata("eng")).expect("english");
        let white = vec![255u8; 400 * 300];
        let lines = recogniser
            .recognise(&white, 400, 300, DPI, TIME, &AtomicBool::new(false))
            .expect("recognise");
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn pictures_that_do_not_fit_are_refused() {
        let _turn = one_at_a_time();
        let mut recogniser = Recogniser::new("eng", &traineddata("eng")).expect("english");
        let mut refused = |grey: &[u8], width: u32, height: u32| {
            matches!(
                recogniser.recognise(grey, width, height, DPI, TIME, &AtomicBool::new(false)),
                Err(OcrError::BadPicture)
            )
        };
        assert!(refused(&[], 0, 0));
        assert!(refused(&[0; 10], 4, 4), "fewer bytes than pixels");
        assert!(refused(&[0; 20], 4, 4), "more bytes than pixels");
        assert!(refused(&[0; 4], u32::MAX, 1));
    }

    #[test]
    fn language_data_that_is_not_language_data_is_refused() {
        let _turn = one_at_a_time();
        for data in [
            &[][..],
            &[0u8; 100][..],
            &[255u8; 1000][..],
            b"eng.traineddata",
        ] {
            assert!(
                matches!(Recogniser::new("eng", data), Err(OcrError::LanguageData)),
                "{} bytes",
                data.len()
            );
        }
        // Every refusal gave the recogniser back.
        Recogniser::new("eng", &traineddata("eng")).expect("english");
    }

    #[test]
    fn a_second_recogniser_is_refused_while_the_first_lives() {
        let _turn = one_at_a_time();
        let first = Recogniser::new("eng", &traineddata("eng")).expect("english");
        assert!(matches!(
            Recogniser::new("eng", &traineddata("eng")),
            Err(OcrError::Busy)
        ));
        drop(first);
        Recogniser::new("eng", &traineddata("eng")).expect("english again");
    }

    #[test]
    fn a_page_that_is_cancelled_gives_nothing_and_the_next_one_is_recognised() {
        let _turn = one_at_a_time();
        let (grey, width, height) = grey_page("benign/mixed-text-zh-en.pdf");
        let mut recogniser = Recogniser::new("eng", &traineddata("eng")).expect("english");
        let cancelled =
            recogniser.recognise(&grey, width, height, DPI, TIME, &AtomicBool::new(true));
        assert!(
            matches!(cancelled, Err(OcrError::Cancelled)),
            "{cancelled:?}"
        );
        let lines = recogniser
            .recognise(&grey, width, height, DPI, TIME, &AtomicBool::new(false))
            .expect("recognise");
        assert!(text_of(&lines).contains(&"Privacy-first PDF Reader".to_owned()));
    }

    #[test]
    fn a_page_that_takes_too_long_is_stopped_and_the_next_one_is_recognised() {
        let _turn = one_at_a_time();
        let (grey, width, height) = grey_page("benign/mixed-text-zh-en.pdf");
        let mut recogniser = Recogniser::new("eng", &traineddata("eng")).expect("english");
        let stopped = recogniser.recognise(
            &grey,
            width,
            height,
            DPI,
            Duration::from_millis(1),
            &AtomicBool::new(false),
        );
        assert!(matches!(stopped, Err(OcrError::Stopped)), "{stopped:?}");
        let lines = recogniser
            .recognise(&grey, width, height, DPI, TIME, &AtomicBool::new(false))
            .expect("recognise");
        assert!(text_of(&lines).contains(&"Privacy-first PDF Reader".to_owned()));
    }
}
