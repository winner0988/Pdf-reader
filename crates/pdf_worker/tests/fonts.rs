//! The CJK fallback font (DEC-03, #31). A PDF may use a Chinese font without embedding it; the
//! worker then needs a substitute, or MuPDF cannot load the font at all: the text is neither
//! drawn nor extracted, so it cannot be searched either. The substitute is the Droid CJK font
//! compiled into the worker, so the sandbox needs no access to the system's fonts.

use pdf_worker::engine::{PdfDocument, RenderedPage};

const SCALE: f32 = 2.0;

/// English in a standard font, then Traditional Chinese in a CNS1 font that is not embedded.
fn mixed_text() -> PdfDocument {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/benign/mixed-text-zh-en.pdf");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    PdfDocument::from_bytes(&bytes).expect("open")
}

/// Dark pixels inside the box of the first hit for `query`, and the box's area in pixels.
fn ink(doc: &PdfDocument, page: &RenderedPage, query: &str) -> (usize, usize) {
    let found = doc.search_page(0, query, true, 1).expect("search");
    let hit = found
        .hits
        .first()
        .unwrap_or_else(|| panic!("{query:?} not found"));
    let quad = &hit.quads[0];
    let xs = [quad.ul.x, quad.ur.x, quad.ll.x, quad.lr.x];
    let ys = [quad.ul.y, quad.ur.y, quad.ll.y, quad.lr.y];
    let to_px = |v: f32, limit: u32| ((v * SCALE).max(0.0) as usize).min(limit as usize);
    let x0 = to_px(xs.iter().copied().fold(f32::MAX, f32::min), page.width);
    let x1 = to_px(xs.iter().copied().fold(f32::MIN, f32::max), page.width);
    let y0 = to_px(ys.iter().copied().fold(f32::MAX, f32::min), page.height);
    let y1 = to_px(ys.iter().copied().fold(f32::MIN, f32::max), page.height);
    let dark = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * page.width as usize + x) * 4))
        .filter(|&i| {
            page.rgba[i..i + 3]
                .iter()
                .map(|&c| u32::from(c))
                .sum::<u32>()
                < 3 * 128
        })
        .count();
    (dark, (x1 - x0) * (y1 - y0))
}

#[test]
fn chinese_text_in_a_font_that_is_not_embedded_can_be_searched() {
    let doc = mixed_text();
    for query in ["隱私優先", "PDF 閱讀器"] {
        let found = doc.search_page(0, query, true, 10).expect("search");
        assert_eq!(found.hits.len(), 1, "{query:?}");
    }
}

#[test]
fn chinese_text_in_a_font_that_is_not_embedded_is_drawn() {
    let doc = mixed_text();
    let page = doc.render(0, SCALE, 0).expect("render");
    let (english, english_area) = ink(&doc, &page, "Privacy-first");
    let (chinese, chinese_area) = ink(&doc, &page, "隱私優先");
    assert!(
        english * 10 > english_area,
        "English: {english} of {english_area} pixels"
    );
    // Glyphs, not blank space or empty boxes: dense Chinese strokes cover about a third.
    assert!(
        chinese * 5 > chinese_area,
        "Chinese: {chinese} of {chinese_area} pixels"
    );
}
