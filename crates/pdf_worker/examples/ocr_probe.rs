//! POC for ADR 0015 (proposed, #99): Windows' own OCR (`Windows.Media.Ocr`) in the worker's
//! sandbox. tests/ocr_poc.rs starts this in the sandbox and sends it a page image; it prints what
//! it recognised, or which step failed. It is an example so that it is never part of the worker
//! or the installer.
//!
//! Usage: `ocr_probe <language tag>` with a binary PGM (8-bit grey, as MuPDF writes it) on stdin.
//! Prints `languages:` and the languages Windows can recognise, then `line:` and the text of each
//! recognised line, or `error:`, the step and what went wrong.
//!
//! With win32k disabled, as in the worker, user32 cannot load. So nothing here may need it:
//! - windows-core imports ole32, which imports user32; build.rs delay-loads ole32, and the probe
//!   avoids the calls that would load it: it joins the multithreaded apartment itself and polls
//!   asynchronous operations instead of registering completion handlers;
//! - bitmaps made from a buffer (`SoftwareBitmap::CreateCopyFromBuffer`, `LockBuffer`) fail with
//!   E_INVALIDARG there, so the image is decoded from a BMP file instead, which works. The
//!   recognition itself then still fails (see ADR 0015).

// One call to RoInitialize.
#![allow(unsafe_code)]

#[cfg(windows)]
fn main() {
    let language = std::env::args().nth(1).unwrap_or_default();
    if let Err(error) = ocr::run(&language) {
        println!("error:{error}");
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod ocr {
    use std::io::Read;
    use std::time::Duration;

    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapDecoder, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{HSTRING, RuntimeType};
    use windows_future::{AsyncStatus, IAsyncOperation};

    pub fn run(language: &str) -> Result<(), String> {
        // Joins the multithreaded apartment through combase; windows-core would otherwise do it
        // through ole32 (CoIncrementMTAUsage).
        // SAFETY: called once, before any other Windows Runtime call on this thread.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(at("apartment"))?;
        let available = OcrEngine::AvailableRecognizerLanguages().map_err(at("languages"))?;
        let mut tags = Vec::new();
        for index in 0..available.Size().map_err(at("languages"))? {
            let language = available.GetAt(index).map_err(at("languages"))?;
            tags.push(language.LanguageTag().map_err(at("languages"))?.to_string());
        }
        println!("languages:{}", tags.join(","));

        let mut input = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut input)
            .map_err(|error| format!("input: {error}"))?;
        let (width, height, grey) = pgm(&input)?;
        let limit = OcrEngine::MaxImageDimension().map_err(at("limit"))?;
        if width > limit || height > limit {
            return Err(format!("input: {width}x{height} is larger than {limit}"));
        }
        let bitmap = decode(&bmp(width, height, grey)).map_err(at("decode"))?;

        let tag = Language::CreateLanguage(&HSTRING::from(language)).map_err(at("language"))?;
        if !OcrEngine::IsLanguageSupported(&tag).map_err(at("recogniser"))? {
            return Err(format!(
                "recogniser: Windows has no OCR for {language} here"
            ));
        }
        let engine = OcrEngine::TryCreateFromLanguage(&tag).map_err(at("recogniser"))?;
        let recognition = engine.RecognizeAsync(&bitmap).map_err(at("recognise"))?;
        let result = wait(&recognition).map_err(at("recognise"))?;
        let lines = result.Lines().map_err(at("result"))?;
        for index in 0..lines.Size().map_err(at("result"))? {
            let line = lines.GetAt(index).map_err(at("result"))?;
            println!("line:{}", line.Text().map_err(at("result"))?);
        }
        Ok(())
    }

    /// Says which step went wrong, and how.
    fn at(step: &'static str) -> impl Fn(windows::core::Error) -> String {
        move |error| format!("{step}: {error}")
    }

    /// The result of `operation`, polled: a completion handler would load ole32.
    fn wait<T: RuntimeType>(operation: &IAsyncOperation<T>) -> windows::core::Result<T> {
        while operation.Status()? == AsyncStatus::Started {
            std::thread::sleep(Duration::from_millis(5));
        }
        operation.GetResults()
    }

    /// The bitmap Windows decodes from the BMP file `bmp`, in memory.
    fn decode(bmp: &[u8]) -> windows::core::Result<SoftwareBitmap> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(bmp)?;
        let stream = InMemoryRandomAccessStream::new()?;
        let write = stream.WriteAsync(&writer.DetachBuffer()?)?;
        while write.Status()? == AsyncStatus::Started {
            std::thread::sleep(Duration::from_millis(5));
        }
        write.GetResults()?;
        stream.Seek(0)?;
        let decoder = wait(&BitmapDecoder::CreateAsync(&stream)?)?;
        wait(&decoder.GetSoftwareBitmapAsync()?)
    }

    /// An 8-bit BMP file of `grey`: rows of `width` pixels, top first, with a grey palette.
    fn bmp(width: u32, height: u32, grey: &[u8]) -> Vec<u8> {
        let row = width.div_ceil(4) * 4;
        let offset = 14 + 40 + 256 * 4;
        let size = offset + row * height;
        let mut file = Vec::with_capacity(size as usize);
        // BITMAPFILEHEADER
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&offset.to_le_bytes());
        // BITMAPINFOHEADER: 8 bits per pixel, uncompressed, 256 palette entries.
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&width.to_le_bytes());
        file.extend_from_slice(&height.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&8u16.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(row * height).to_le_bytes());
        file.extend_from_slice(&[0; 8]);
        file.extend_from_slice(&256u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        for level in 0..=255 {
            file.extend_from_slice(&[level, level, level, 0]);
        }
        // Bottom row first, each padded to four bytes.
        for line in grey.chunks_exact(width as usize).rev() {
            file.extend_from_slice(line);
            file.resize(file.len() + (row - width) as usize, 0);
        }
        file
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
        // At most 10,000 pixels a side (the engine's limit) keeps the BMP's sizes within 32 bits.
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
