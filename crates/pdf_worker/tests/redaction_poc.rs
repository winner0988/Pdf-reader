//! POC for ADR 0016 (proposed, #178): what MuPDF's redaction removes from a page and from the file
//! written afterwards, and what it leaves. Nothing here is the app's code: the real work would be
//! done by the worker. A sample page has a secret in its text, in a note, in a form field, in a
//! link, in a bookmark and in the document's metadata; an image and a vector rectangle that reach
//! into the area. Each test says what it found, and the ones that find a leak are the ADR's list of
//! what a redaction does not do by itself.

use mupdf::pdf::{
    PdfDocument, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod, PdfRedactOptions,
    PdfRedactTextMethod, PdfWriteOptions,
};
use mupdf::{Colorspace, Matrix};

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

const SECRET: &str = "SECRET-ALPHA-123";

/// A PDF with the objects below, an uncompressed content stream, and `/Info` in the trailer.
fn sample() -> Vec<u8> {
    let content = format!(
        "BT /F1 18 Tf 72 700 Td (Public line one) Tj ET\n\
         BT /F1 18 Tf 72 650 Td ({SECRET} inside) Tj ET\n\
         BT /F1 18 Tf 72 600 Td (Public line two) Tj ET\n\
         q 120 0 0 120 250 560 cm /Im1 Do Q\n\
         0.1 0.3 0.9 rg 280 630 60 30 re f\n"
    );
    let image = "@".repeat(16);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Outlines 10 0 R /AcroForm << /Fields [8 0 R] /DA (/Helv 0 Tf 0 g) >> >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /Helv 5 0 R >> /XObject << /Im1 6 0 R >> >> /Annots [7 0 R 8 0 R 9 0 R] >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!(
            "<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 16 >>\nstream\n{image}\nendstream"
        ),
        format!("<< /Type /Annot /Subtype /Text /Rect [100 645 120 665] /Contents ({SECRET} in a note) /Name /Note /F 4 /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V ({SECRET} in a field) /Rect [200 640 320 670] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"),
        "<< /Type /Annot /Subtype /Link /Rect [70 640 200 670] /A << /S /URI /URI (https://secret-alpha-123.example/) >> /F 4 >>".to_owned(),
        "<< /Type /Outlines /First 11 0 R /Last 11 0 R /Count 1 >>".to_owned(),
        format!("<< /Title ({SECRET} bookmark) /Parent 10 0 R /Dest [3 0 R /Fit] >>"),
        format!("<< /Title ({SECRET} title) /Author (Jane Public) >>"),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 12 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// What the pages' contents say as plain text: the page content streams, decoded.
fn content_text(page: &PdfPage) -> String {
    let Some(contents) = page.contents().expect("contents") else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if contents.is_array().unwrap() {
        for index in 0..contents.len().unwrap() {
            let part = contents.get_array(index as i32).unwrap().unwrap();
            bytes.extend_from_slice(&part.read_stream().unwrap());
        }
    } else {
        bytes = contents.read_stream().unwrap();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn rewrite(doc: &PdfDocument) -> Vec<u8> {
    let mut options = PdfWriteOptions::default();
    options.set_garbage(true);
    let mut bytes = Vec::new();
    doc.write_to_with_options(&mut bytes, options)
        .expect("write");
    bytes
}

/// The gray level of the sample page at (`x`, `y_from_bottom`) in points, rendered at 72 dpi.
fn pixel(doc: &PdfDocument, x: usize, y_from_bottom: usize) -> [u8; 3] {
    let page = doc.load_page(0).expect("page");
    let pixmap = page
        .to_pixmap(&Matrix::IDENTITY, &Colorspace::device_rgb(), false, false)
        .expect("render");
    let (width, n) = (pixmap.width() as usize, pixmap.n() as usize);
    let y = 792 - y_from_bottom;
    let at = (y * width + x) * n;
    let samples = pixmap.samples();
    [samples[at], samples[at + 1], samples[at + 2]]
}

fn redact(doc: &PdfDocument, options: PdfRedactOptions) {
    let mut page = doc.load_pdf_page(0).expect("page");
    let found = page.search(&format!("{SECRET} inside"), 8).expect("search");
    assert_eq!(found.len(), 1, "the secret is on the page");
    page.add_redact_annotation(found[0].clone()).expect("mark");
    assert!(page.redact_with_options(options).expect("redact"));
}

/// A page whose content is `content`, with `form` as the form XObject /Fm1 it can draw.
fn one_page_with_form(content: &str, form: &str) -> Vec<u8> {
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >> >> >>".into(),
        format!("<< /Length {} >>
stream
{content}
endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Length {} >>
stream
{form}
endstream", form.len()),
    ])
}

/// What an application of the redactions should do to what the area reaches, in the options a
/// worker would use: black boxes (so the user sees what was removed), image pixels blanked, line
/// art removed if touched, text removed (visible or not).
const OPTIONS: PdfRedactOptions = PdfRedactOptions {
    black_boxes: true,
    image_method: PdfRedactImageMethod::Pixels,
    line_art: PdfRedactLineArtMethod::RemoveIfTouched,
    text: PdfRedactTextMethod::Remove,
};

/// Marks the area of the secret in `pdf` (found by search, or by place when it is invisible) and
/// applies the redactions.
fn redacted(pdf: &[u8], options: PdfRedactOptions) -> PdfDocument {
    let doc = PdfDocument::from_bytes(pdf).expect("open");
    let mut page = doc.load_pdf_page(0).expect("page");
    let area = mupdf::Rect {
        x0: 60.0,
        y0: 120.0,
        x1: 330.0,
        y1: 150.0,
    };
    page.add_redact_annotation(area).expect("mark");
    assert!(page.redact_with_options(options).expect("redact"));
    doc
}

fn text_of(doc: &PdfDocument) -> String {
    doc.load_page(0)
        .expect("page")
        .to_text_page(mupdf::TextPageFlags::empty())
        .expect("text")
        .to_text()
        .expect("text")
}

#[test]
fn text_in_the_area_is_gone_from_the_page_and_from_the_rewritten_file() {
    let doc = PdfDocument::from_bytes(&sample()).expect("open");
    assert!(text_of(&doc).contains(SECRET));
    redact(&doc, OPTIONS);

    // The page's content has no such text, and the text around it is as it was.
    let content = content_text(&doc.load_pdf_page(0).unwrap());
    assert!(!content.contains("SECRET-ALPHA-123 inside"), "{content}");
    assert!(content.contains("Public line one") && content.contains("Public line two"));
    let text = text_of(&doc);
    assert!(!text.contains(SECRET));
    assert!(text.contains("Public line one") && text.contains("Public line two"));
    // The black box that says something was there is drawn.
    assert!(content.contains("\nf\n"), "{content}");

    // The rewritten file, opened again: no text to find, and no old content stream.
    let bytes = rewrite(&doc);
    assert!(!has(&bytes, "SECRET-ALPHA-123 inside"));
    let again = PdfDocument::from_bytes(&bytes).expect("reopen");
    assert!(!text_of(&again).contains(SECRET));
    assert!(
        again
            .load_page(0)
            .unwrap()
            .search(SECRET, 8)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_marks_are_gone_once_they_are_applied() {
    let doc = PdfDocument::from_bytes(&sample()).expect("open");
    redact(&doc, OPTIONS);
    let page = doc.load_pdf_page(0).unwrap();
    assert!(
        page.annotations()
            .all(|annotation| annotation.r#type().unwrap() != mupdf::pdf::PdfAnnotationType::Redact)
    );
}

#[test]
fn text_in_a_form_object_and_invisible_text_are_removed_too() {
    let in_form = one_page_with_form(
        "q /Fm1 Do Q\n",
        &format!("BT /F1 18 Tf 72 650 Td ({SECRET}) Tj ET"),
    );
    let invisible = one_page_with_form(
        &format!("BT /F1 18 Tf 3 Tr 72 650 Td ({SECRET}) Tj ET\n"),
        "",
    );
    for (name, pdf) in [
        ("form object", in_form),
        ("invisible text (an OCR layer)", invisible),
    ] {
        let doc = redacted(&pdf, OPTIONS);
        assert!(!text_of(&doc).contains(SECRET), "{name}");
        assert!(!has(&rewrite(&doc), SECRET), "{name}");
    }
    // Asked not to, it leaves them: the option decides.
    let invisible = one_page_with_form(
        &format!("BT /F1 18 Tf 3 Tr 72 650 Td ({SECRET}) Tj ET\n"),
        "",
    );
    let doc = redacted(
        &invisible,
        PdfRedactOptions {
            text: PdfRedactTextMethod::None,
            ..OPTIONS
        },
    );
    assert!(has(&rewrite(&doc), SECRET));
}

#[test]
fn an_image_loses_its_pixels_in_the_area_only() {
    let doc = PdfDocument::from_bytes(&sample()).expect("open");
    assert_eq!(pixel(&doc, 270, 655), [64, 64, 64]);
    redact(&doc, OPTIONS);
    // Black where the area is (the box), the image as it was elsewhere.
    assert_eq!(pixel(&doc, 270, 655), [0, 0, 0]);
    assert_eq!(pixel(&doc, 350, 600), [64, 64, 64]);
    // And the image itself has other pixels there, not only a box drawn over them.
    let page = doc.load_pdf_page(0).unwrap();
    let resources = page.object().get_dict("Resources").unwrap().unwrap();
    let image = resources
        .get_dict("XObject")
        .unwrap()
        .unwrap()
        .get_dict("Im1")
        .unwrap()
        .unwrap();
    let data = image.read_stream().unwrap();
    assert_ne!(data, vec![b'@'; 16]);
    assert_eq!(
        data.iter().filter(|byte| **byte == b'@').count(),
        12,
        "{data:?}"
    );
}

#[test]
fn an_option_that_only_covers_leaves_what_it_covers_in_the_file() {
    // Image: with no method the picture is whole, under the box.
    let doc = redacted_secret_area(PdfRedactOptions {
        image_method: PdfRedactImageMethod::None,
        ..OPTIONS
    });
    let page = doc.load_pdf_page(0).unwrap();
    let resources = page.object().get_dict("Resources").unwrap().unwrap();
    let image = resources
        .get_dict("XObject")
        .unwrap()
        .unwrap()
        .get_dict("Im1")
        .unwrap()
        .unwrap();
    assert_eq!(image.read_stream().unwrap(), vec![b'@'; 16]);
    // Line art: touched removes the rectangle that the area only reaches into; covered does not
    // (only what it wholly covers goes); none leaves it, under the box.
    for (line_art, left) in [
        (PdfRedactLineArtMethod::RemoveIfTouched, false),
        (PdfRedactLineArtMethod::RemoveIfCovered, true),
        (PdfRedactLineArtMethod::None, true),
    ] {
        let doc = redacted_secret_area(PdfRedactOptions {
            line_art,
            ..OPTIONS
        });
        let content = content_text(&doc.load_pdf_page(0).unwrap());
        assert_eq!(content.contains("280 630 60 30 re"), left, "{line_art:?}");
    }
}

/// The sample with the area of its secret redacted, which search finds.
fn redacted_secret_area(options: PdfRedactOptions) -> PdfDocument {
    let doc = PdfDocument::from_bytes(&sample()).expect("open");
    redact(&doc, options);
    doc
}

#[test]
fn the_old_content_stays_in_the_file_unless_it_is_rewritten_with_garbage_collection() {
    let doc = redacted_secret_area(OPTIONS);
    // Written as it is: the stream that was replaced is still an object of the file.
    let mut plain = Vec::new();
    doc.write_to_with_options(&mut plain, PdfWriteOptions::default())
        .unwrap();
    assert!(has(&plain, "SECRET-ALPHA-123 inside"));
    // Appended to the original file (what a signed document gets): the old bytes are all there.
    let mut appended = Vec::new();
    let mut options = PdfWriteOptions::default();
    options.set_incremental(true);
    doc.write_to_with_options(&mut appended, options).unwrap();
    assert!(has(&appended, "SECRET-ALPHA-123 inside"));
    // Rewritten without the objects nothing uses: gone.
    assert!(!has(&rewrite(&doc), "SECRET-ALPHA-123 inside"));
}

#[test]
fn what_the_area_does_not_reach_stays_in_the_file() {
    let doc = redacted_secret_area(OPTIONS);
    let bytes = rewrite(&doc);
    // A link in the area is removed with it.
    assert!(!has(&bytes, "secret-alpha-123.example"));
    // These are not: a note and a form field in the area, the bookmark and the metadata.
    for (what, needle) in [
        ("a note in the area", "SECRET-ALPHA-123 in a note"),
        ("a form field in the area", "SECRET-ALPHA-123 in a field"),
        ("a bookmark", "SECRET-ALPHA-123 bookmark"),
        ("the document's information", "SECRET-ALPHA-123 title"),
    ] {
        assert!(has(&bytes, needle), "{what} is still in the file");
    }
}
