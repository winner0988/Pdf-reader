//! POC for ADR 0015 (proposed, #99): recognising the text of a scanned page offline with Windows'
//! own OCR (`Windows.Media.Ocr`), in the worker's sandbox.
//!
//! The scan is made here: the corpus's text page, rendered by MuPDF and put alone on a new page
//! as an image, has no text layer, like a scanned page. MuPDF renders the scan as the worker
//! would, and examples/ocr_probe.rs recognises the image in the sandbox:
//! - with every restriction of the worker except one: win32k stays available. AppContainer with
//!   no capabilities (so no network), Low integrity, no dynamic code, a single process, the
//!   memory cap. Both languages are recognised;
//! - in the worker's own sandbox, with win32k disabled too, recognition fails.
//!
//! Needs Windows' OCR for English, which English Windows has (so does the CI runner). Traditional
//! Chinese is checked where Windows has its OCR too, as Traditional Chinese Windows does.
//!
//! Tesseract (module `tesseract`) recognises the same image in the worker's own sandbox, win32k
//! disabled too: both the copy inside MuPDF and one built apart. It needs the `tesseract-poc`
//! feature and what scripts/ocr-poc/build-tesseract.ps1 builds and fetches (`TESSERACT_POC_DIR`).
#![cfg(windows)]

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument};
use mupdf::{Colorspace, Document, ImageFormat, Matrix, Pixmap, TextPageFlags};
use sandbox::{SandboxConfig, Sandboxed};

/// Resolution of the scan, and of the page image the worker would give the engine.
const DPI: f32 = 200.0;

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The first page of `pdf` in grey at `dpi`, as MuPDF renders it.
fn render(pdf: &[u8], dpi: f32) -> Pixmap {
    let doc = Document::from_bytes(pdf, "application/pdf").expect("open");
    let page = doc.load_page(0).expect("page");
    let scale = dpi / 72.0;
    page.to_pixmap(
        &Matrix::new_scale(scale, scale),
        &Colorspace::device_gray(),
        false,
        true,
    )
    .expect("render")
}

/// A scan of the first page of `pdf`: a page of the same size with nothing but its image.
fn scan(pdf: &[u8]) -> Vec<u8> {
    let bounds = {
        let doc = Document::from_bytes(pdf, "application/pdf").expect("open");
        doc.load_page(0).expect("page").bounds().expect("bounds")
    };
    let image = render(pdf, DPI);
    let mut scanned = PdfDocument::new();
    let mut page = scanned
        .new_page((bounds.width(), bounds.height()))
        .expect("new page");
    page.insert_image(
        &mut scanned,
        bounds,
        PageImageSource::Pixmap(&image),
        InsertImageOptions::default(),
    )
    .expect("insert image");
    drop(page);
    let mut bytes = Vec::new();
    scanned.write_to(&mut bytes).expect("write");
    bytes
}

/// The text layer of the first page of `pdf`.
fn text_layer(pdf: &[u8]) -> String {
    let doc = Document::from_bytes(pdf, "application/pdf").expect("open");
    let page = doc.load_page(0).expect("page");
    let text = page
        .to_text_page(TextPageFlags::empty())
        .expect("text page");
    text.to_text().expect("text")
}

/// A scan of benign/mixed-text-zh-en.pdf as the worker would render it: a binary PGM.
fn scanned_page() -> Vec<u8> {
    let scanned = scan(&corpus("benign/mixed-text-zh-en.pdf"));
    // No text layer: today search and selection find nothing on such a page.
    assert_eq!(text_layer(&scanned).trim(), "");
    let mut pgm = Vec::new();
    render(&scanned, DPI)
        .write_to(&mut pgm, ImageFormat::PNM)
        .expect("write PGM");
    assert!(pgm.starts_with(b"P5"), "a grey pixmap is written as PGM");
    pgm
}

/// The example `name`, which `cargo test` builds next to the tests.
fn probe(name: &str) -> PathBuf {
    let tests = std::env::current_exe().expect("test executable");
    let probe = tests
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target folder")
        .join("examples")
        .join(format!("{name}.exe"));
    assert!(
        probe.exists(),
        "{} is missing: run the whole package's tests (cargo test -p pdf_worker), which build the \
         examples too",
        probe.display()
    );
    probe
}

/// The sandbox of the worker, without its win32k restriction.
fn with_win32k() -> SandboxConfig {
    SandboxConfig {
        allow_win32k: true,
        ..SandboxConfig::default()
    }
}

/// What Windows' OCR (examples/ocr_probe.rs) printed for `image` (a binary PGM) in a sandbox
/// made with `config`.
fn recognise(image: &[u8], language: &str, config: &SandboxConfig) -> Vec<String> {
    run_probe("ocr_probe", language, image, config)
}

/// Runs the example `name` in a sandbox made with `config`, with `input` on its stdin, and
/// returns the lines it printed. Says how long it took and how much memory it used.
fn run_probe(name: &str, language: &str, input: &[u8], config: &SandboxConfig) -> Vec<String> {
    let started = Instant::now();
    let mut child = Sandboxed::spawn(&probe(name), &[OsStr::new(language)], config)
        .expect("start the probe in the sandbox");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = input.to_vec();
    // The probe reads all of it before answering; write while this thread reads.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let mut output = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut output)
        .expect("read the probe's output");
    let sent = writer.join().expect("writer");
    let code = child.wait_timeout(Duration::from_secs(60)).expect("wait");
    assert!(
        sent.is_ok() && code == Some(0),
        "the probe failed (exit code {code:x?}, sending the input: {sent:?}): {output}"
    );
    let memory = child.peak_memory_bytes().unwrap_or(0) / (1024 * 1024);
    eprintln!(
        "{name} {language}, {:?}, {memory} MB: {output}",
        started.elapsed()
    );
    output.lines().map(str::to_owned).collect()
}

/// Whether the probe reported `tag` among the languages Windows can recognise.
fn has_language(output: &[String], tag: &str) -> bool {
    output
        .first()
        .and_then(|line| line.strip_prefix("languages:"))
        .is_some_and(|languages| languages.split(',').any(|language| language == tag))
}

#[test]
fn english_on_a_scanned_page_is_recognised_offline_in_the_sandbox_with_win32k() {
    let output = recognise(&scanned_page(), "en-US", &with_win32k());
    assert!(
        output.contains(&"line:Privacy-first PDF Reader".to_owned()),
        "{output:?}"
    );
}

/// Traditional Chinese Windows has this recogniser; the CI runner, an English Windows Server,
/// does not, and adding it there did not finish in 20 minutes. Without it, the engine must say
/// so instead of recognising nothing.
#[test]
fn chinese_on_a_scanned_page_is_recognised_offline_in_the_sandbox_with_win32k() {
    let output = recognise(&scanned_page(), "zh-Hant-TW", &with_win32k());
    if !has_language(&output, "zh-Hant-TW") {
        eprintln!("no Traditional Chinese OCR in this Windows: only its absence is checked");
        assert!(
            output
                .iter()
                .any(|line| line.starts_with("error:recogniser")),
            "{output:?}"
        );
        return;
    }
    // The engine puts a space between Chinese characters; the app would remove them.
    let lines: Vec<String> = output.iter().map(|line| line.replace(' ', "")).collect();
    assert!(
        lines.contains(&"line:隱私優先的PDF閱讀器".to_owned()),
        "{output:?}"
    );
}

#[test]
fn recognition_fails_in_the_workers_own_sandbox() {
    let output = recognise(&scanned_page(), "en-US", &SandboxConfig::default());
    // The engine starts and has the language, but no text comes back.
    assert!(has_language(&output, "en-US"), "{output:?}");
    assert!(
        output.iter().any(|line| line.starts_with("error:")),
        "{output:?}"
    );
    assert!(
        !output.iter().any(|line| line.starts_with("line:")),
        "{output:?}"
    );
}

/// Tesseract, in the worker's own sandbox, win32k disabled too. The language data goes to the
/// probe on stdin with the image: the sandbox opens no files.
#[cfg(feature = "tesseract-poc")]
mod tesseract {
    use std::path::Path;

    use super::*;

    /// A language's data, as scripts/ocr-poc/build-tesseract.ps1 installed it.
    fn traineddata(language: &str) -> Vec<u8> {
        let dir = std::env::var("TESSERACT_POC_DIR").expect("TESSERACT_POC_DIR");
        let path = Path::new(&dir)
            .join("tessdata")
            .join(format!("{language}.traineddata"));
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn recognise(image: &[u8], language: &str) -> Vec<String> {
        recognise_with("tesseract_probe", image, language)
    }

    /// What `probe` recognised in `image`, run in the worker's own sandbox.
    fn recognise_with(probe: &str, image: &[u8], language: &str) -> Vec<String> {
        let data = traineddata(language);
        let mut input = u32::try_from(data.len())
            .expect("language data size")
            .to_le_bytes()
            .to_vec();
        input.extend_from_slice(&data);
        input.extend_from_slice(image);
        run_probe(probe, language, &input, &SandboxConfig::default())
    }

    #[test]
    fn mupdf_s_own_tesseract_recognises_english_in_the_workers_own_sandbox() {
        let output = recognise_with("mupdf_tesseract_probe", &scanned_page(), "eng");
        assert!(
            output.contains(&"line:Privacy-first PDF Reader".to_owned()),
            "{output:?}"
        );
    }

    #[test]
    fn mupdf_s_own_tesseract_recognises_chinese_in_the_workers_own_sandbox() {
        let output = recognise_with("mupdf_tesseract_probe", &scanned_page(), "chi_tra");
        let lines: Vec<String> = output.iter().map(|line| line.replace(' ', "")).collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("隱私優先的PDF閱讀器")),
            "{output:?}"
        );
    }

    #[test]
    fn english_on_a_scanned_page_is_recognised_in_the_workers_own_sandbox() {
        let output = recognise(&scanned_page(), "eng");
        assert!(
            output.contains(&"line:Privacy-first PDF Reader".to_owned()),
            "{output:?}"
        );
    }

    #[test]
    fn chinese_on_a_scanned_page_is_recognised_in_the_workers_own_sandbox() {
        let output = recognise(&scanned_page(), "chi_tra");
        let lines: Vec<String> = output.iter().map(|line| line.replace(' ', "")).collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("隱私優先的PDF閱讀器")),
            "{output:?}"
        );
    }
}
