//! The content filter of the `mupdf` fork (#748, ADR 0036): `PdfPage::filter_text` takes single
//! glyphs out of a page's content and leaves everything else. Editing text that is already on a
//! page (#749 to #751) removes the old glyphs this way, because MuPDF's `pdf_redact_page` (the
//! binding's redaction, ADR 0016) also deletes every link that reaches into the area (POC #725,
//! C.6). The second test is the negative control: the same check fails for redaction.

use mupdf::pdf::{
    PdfDocument, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod, PdfRedactOptions,
    PdfRedactTextMethod, PdfWriteOptions, TextFilterAction, TextFilterOptions,
};
use mupdf::{Rect, TextPageFlags};

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

/// Three lines of 12 pt text 13.5 pt apart (the POC's tight leading), and a link over the
/// middle one that reaches into the lines above and below.
fn sample() -> PdfDocument {
    let content = "BT /F1 12 Tf 72 700 Td (Upper line) Tj 0 -13.5 Td (Middle line) Tj \
                   0 -13.5 Td (Lower line) Tj ET";
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        "<< /Type /Annot /Subtype /Link /Rect [70 680 160 702] /Border [0 0 0] \
         /A << /S /URI /URI (https://example.com/) >> >>"
            .to_owned(),
    ]);
    PdfDocument::from_bytes(&pdf).expect("open")
}

fn text_of(page: &PdfPage) -> String {
    page.to_text_page(TextPageFlags::empty())
        .expect("text")
        .to_text()
        .expect("text")
}

/// The box of the middle line in the space of the text page, 15% of its height in from the top
/// and the bottom (ADR 0036: the boxes of tightly set lines overlap).
fn middle_line(page: &PdfPage) -> Rect {
    let hits = page
        .to_text_page(TextPageFlags::empty())
        .expect("text")
        .search("Middle line")
        .expect("search");
    assert_eq!(hits.len(), 1, "the middle line once");
    let line = Rect::from(hits[0].clone());
    let inset = line.height() * 0.15;
    Rect::new(line.x0, line.y0 + inset, line.x1, line.y1 - inset)
}

/// What removing the middle line must leave: the lines above and below whole, and the link.
fn check(page: &PdfPage) -> Result<(), String> {
    let text = text_of(page);
    if text.contains("Middle") {
        return Err(format!("the middle line is still there: {text:?}"));
    }
    for line in ["Upper line", "Lower line"] {
        if !text.contains(line) {
            return Err(format!("{line:?} is damaged: {text:?}"));
        }
    }
    let links: Vec<_> = page.links().map_err(|err| err.to_string())?.collect();
    if links.len() != 1 || links[0].uri != "https://example.com/" {
        return Err(format!(
            "{} links instead of the one over the line",
            links.len()
        ));
    }
    Ok(())
}

/// The document written again, checked with `qpdf --check`, and read back.
fn saved_and_read_back(doc: &PdfDocument) -> PdfDocument {
    let mut options = PdfWriteOptions::default();
    options.set_garbage(true);
    let mut bytes = Vec::new();
    doc.write_to_with_options(&mut bytes, options)
        .expect("write");
    common::check_qpdf_bytes(&bytes);
    PdfDocument::from_bytes(&bytes).expect("open again")
}

#[test]
fn the_filter_removes_a_line_and_keeps_the_link_over_it() {
    let doc = sample();
    let mut page = doc.load_pdf_page(0).expect("page");
    let area = middle_line(&page);
    let mut removed = String::new();
    page.filter_text(TextFilterOptions::default(), |glyph| {
        let bounds = glyph.bounds();
        let (x, y) = ((bounds.x0 + bounds.x1) / 2.0, (bounds.y0 + bounds.y1) / 2.0);
        if area.contains(x, y) {
            removed.extend(glyph.unicode());
            TextFilterAction::Remove
        } else {
            TextFilterAction::Keep
        }
    })
    .expect("filter");

    assert_eq!(removed, "Middle line");
    check(&page).unwrap();
    // The same in the written file.
    let reread = saved_and_read_back(&doc);
    check(&reread.load_pdf_page(0).expect("page")).unwrap();
}

/// The negative control: removing the same line with `pdf_redact_page`, without black boxes and
/// without touching pictures or vector graphics, also deletes the link, so the check fails.
#[test]
fn the_check_fails_when_redaction_removes_the_line() {
    let doc = sample();
    let mut page = doc.load_pdf_page(0).expect("page");
    let area = middle_line(&page);
    page.add_redact_annotation(area).expect("mark");
    page.redact_with_options(PdfRedactOptions {
        black_boxes: false,
        image_method: PdfRedactImageMethod::None,
        line_art: PdfRedactLineArtMethod::None,
        text: PdfRedactTextMethod::Remove,
    })
    .expect("redact");

    let text = text_of(&page);
    assert!(
        !text.contains("Middle") && text.contains("Upper line") && text.contains("Lower line"),
        "{text:?}"
    );
    let failure = check(&page).expect_err("redaction deletes the link");
    assert_eq!(failure, "0 links instead of the one over the line");
}
