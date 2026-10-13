//! A font file from the user's computer, read, cut to the glyphs of some words and set in a text
//! box (B2-31, ADR 0018). Only `skrifa` and `subsetter` parse the font, and both are memory safe,
//! but a font they cannot cope with can still make them panic, run for ever or ask for a great
//! deal of memory; and what they write is read again by MuPDF when the page is drawn, so a subset
//! that is malformed is what would crash the worker there. A font that makes a model is therefore
//! put on a page, drawn, saved, opened again and searched.
//!
//! The sandbox contains a crash (ADR 0008), but it does not replace finding it.
#![no_main]

use ipc_contract::types::{Rect, Rgb, TextAlign, TextBoxStyle};
use libfuzzer_sys::fuzz_target;
use pdf_worker::engine::{PdfDocument, TextBoxFont, prepare_text_box};

/// A page to set the box on (Letter size).
const PAGE: &[u8] = include_bytes!("../../tests/corpus/benign/single-page.pdf");

/// Words in the scripts the first version sets; the size of the font picks one, so that the
/// input does not spend a byte of the font on it.
const WORDS: [&str; 4] = [
    "Hello, world 123",
    "Ünïcödé ¿¡ ß",
    "隱私優先的 PDF 閱讀器",
    "日本語のテスト",
];

fuzz_target!(|data: &[u8]| {
    // Face 1 of a collection as well as the first font: the same code, another table directory.
    for face in 0..2 {
        let font = TextBoxFont {
            bytes: data,
            face,
            name: "Fuzz",
        };
        for words in [WORDS[0], WORDS[data.len() % WORDS.len()]] {
            let Ok(model) = prepare_text_box(&font, words) else {
                continue;
            };
            let Ok(mut document) = PdfDocument::from_bytes(PAGE) else {
                return;
            };
            let rect = Rect {
                x0: 72.0,
                y0: 72.0,
                x1: 372.0,
                y1: 160.0,
            };
            let style = TextBoxStyle {
                size: 18.0,
                color: Rgb { r: 0, g: 0, b: 160 },
                align: TextAlign::Left,
            };
            if document.place_text_box(0, rect, &model, &style).is_err() {
                continue;
            }
            // Drawn small: the subset is read by MuPDF's TrueType loader.
            let _ = document.render(0, 0.5, 0);
            let mut saved = Vec::new();
            if document.save(&mut saved).is_ok() {
                if let Ok(again) = PdfDocument::from_bytes(&saved) {
                    let _ = again.render(0, 0.5, 0);
                    let _ = again.search_page(0, "o", false, 100);
                }
            }
        }
    }
});
