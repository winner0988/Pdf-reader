//! Redaction (B2-19, ADR 0016) in the worker's engine: the POC's findings
//! (`tests/redaction_poc.rs`) as tests of the app's own code, and what the POC did not try:
//! Chinese text in an embedded font, a turned page, a field with a widget outside the area, marks
//! moved and removed, a signed document. Upgrading MuPDF must keep these passing (ADR 0016).

use ipc_contract::types::{AnnotationKind, CreateKind, Rect, RedactionSummary};
use pdf_worker::engine::{PdfBuilder, PdfDocument};

// Shared with the other test crates, which use the helpers this one does not.
#[allow(dead_code)]
mod common;

const SECRET: &str = "SECRET-ALPHA-123";

/// The POC's page (see `redaction_poc.rs`): the secret in a line of text, a note, a form field, a
/// link, a bookmark and the document information; a picture and a blue rectangle that reach into
/// the area of the secret.
fn sample() -> Vec<u8> {
    let content = format!(
        "BT /F1 18 Tf 72 700 Td (Public line one) Tj ET\n\
         BT /F1 18 Tf 72 650 Td ({SECRET} inside) Tj ET\n\
         BT /F1 18 Tf 72 600 Td (Public line two) Tj ET\n\
         q 120 0 0 120 250 560 cm /Im1 Do Q\n\
         0.1 0.3 0.9 rg 280 630 60 30 re f\n"
    );
    let image = "@".repeat(16);
    let mut objects = vec![
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
    ];
    // The document information, named by the trailer that `build_pdf` writes: kept as an object
    // the catalog points to as well, so the trailer needs no change.
    objects[0] = objects[0].replace(" >> >>", " >> /Info 12 0 R >>");
    objects.push(format!(
        "<< /Title ({SECRET} title) /Author (Jane Public) >>"
    ));
    common::build_pdf(&objects)
}

/// The area of the secret's line in page space (origin at the top left, y down): the POC's.
const AREA: Rect = Rect {
    x0: 60.0,
    y0: 120.0,
    x1: 330.0,
    y1: 150.0,
};

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn text_of(doc: &PdfDocument, index: u32) -> String {
    doc.page_text(index, 100_000)
        .expect("text")
        .lines
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn saved(doc: &PdfDocument) -> Vec<u8> {
    let mut out = Vec::new();
    let (_, incremental) = doc.save(&mut out).expect("save");
    assert!(!incremental, "an unsigned document is rewritten whole");
    out
}

/// The colour of the page at (`x`, `y`) in page points from the top left, rendered at 72 dpi.
fn pixel(doc: &PdfDocument, x: usize, y: usize) -> [u8; 3] {
    let page = doc.render(0, 1.0, 0).expect("render");
    let at = (y * page.width as usize + x) * 4;
    [page.rgba[at], page.rgba[at + 1], page.rgba[at + 2]]
}

fn marked(pdf: &[u8], area: Rect) -> PdfDocument {
    let mut doc = PdfDocument::from_bytes(pdf).expect("open");
    doc.add_redaction(0, area).expect("mark");
    doc
}

/// The box around a search hit, a point wider on each side.
fn hit_area(doc: &PdfDocument, index: u32, query: &str) -> Rect {
    let found = doc.search_page(index, query, true, 8).expect("search");
    assert_eq!(found.hits.len(), 1, "{query} once on page {index}");
    let points: Vec<_> = found.hits[0]
        .quads
        .iter()
        .flat_map(|quad| [quad.ul, quad.ur, quad.ll, quad.lr])
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for point in points {
        (x0, x1) = (x0.min(point.x), x1.max(point.x));
        (y0, y1) = (y0.min(point.y), y1.max(point.y));
    }
    Rect {
        x0: x0 - 1.0,
        y0: y0 - 1.0,
        x1: x1 + 1.0,
        y1: y1 + 1.0,
    }
}

#[test]
fn a_mark_is_an_annotation_until_it_is_applied() {
    let doc = marked(&sample(), AREA);
    let annotations = doc.page_annotations(0).expect("annotations");
    let mark = annotations
        .iter()
        .find(|annotation| annotation.kind == AnnotationKind::Redaction)
        .expect("the mark is listed");
    assert!(
        mark.rect.x0 <= AREA.x0 + 1.0 && mark.rect.y1 >= AREA.y1 - 1.0,
        "{mark:?}"
    );
    // Nothing is removed yet.
    assert!(text_of(&doc, 0).contains(SECRET));
    // What applying would do: one mark on one page; the note and the field in the area go too.
    assert_eq!(
        doc.redaction_summary().expect("summary"),
        RedactionSummary {
            marks: 1,
            pages: 1,
            annotations: 1,
            fields: 1,
        }
    );
}

#[test]
fn text_in_the_area_is_gone_from_the_page_and_from_the_saved_file() {
    let mut doc = marked(&sample(), AREA);
    doc.apply_redactions().expect("apply");
    let text = text_of(&doc, 0);
    assert!(!text.contains(SECRET), "{text}");
    assert!(
        text.contains("Public line one") && text.contains("Public line two"),
        "{text}"
    );
    assert!(
        doc.search_page(0, SECRET, false, 8)
            .unwrap()
            .hits
            .is_empty()
    );
    // The marks are gone; so are the note and the field that reached into the area.
    let left = doc.page_annotations(0).unwrap();
    assert!(
        left.iter()
            .all(|annotation| annotation.kind != AnnotationKind::Redaction)
    );
    assert!(
        left.iter()
            .all(|annotation| annotation.kind != AnnotationKind::Note)
    );
    assert!(doc.page_fields(0).unwrap().is_empty());
    assert_eq!(
        doc.redaction_summary().unwrap(),
        RedactionSummary {
            marks: 0,
            pages: 0,
            annotations: 0,
            fields: 0,
        }
    );

    // Saved (rewritten whole, ADR 0013) and opened again: none of it is in the file.
    let bytes = saved(&doc);
    for gone in [
        "SECRET-ALPHA-123 inside",
        "SECRET-ALPHA-123 in a note",
        "SECRET-ALPHA-123 in a field",
        "secret-alpha-123.example",
    ] {
        assert!(!has(&bytes, gone), "{gone} is still in the file");
    }
    let again = PdfDocument::from_bytes(&bytes).expect("reopen");
    assert!(!text_of(&again, 0).contains(SECRET));
    assert!(
        again
            .search_page(0, SECRET, false, 8)
            .unwrap()
            .hits
            .is_empty()
    );
    // What no area reaches is not handled (ADR 0016): the bookmark and the document information.
    for kept in ["SECRET-ALPHA-123 bookmark", "SECRET-ALPHA-123 title"] {
        assert!(has(&bytes, kept), "{kept}");
    }
}

#[test]
fn a_picture_loses_its_pixels_in_the_area_and_the_graphics_it_touches_go() {
    // Before any mark (the mark itself is drawn over the area). Points clear of the text, the
    // note and the field: the picture's gray inside the area, the picture below it, and the blue
    // rectangle that reaches into the area, where the area does not reach.
    let unmarked = PdfDocument::from_bytes(&sample()).expect("open");
    assert_eq!(pixel(&unmarked, 325, 127), [64, 64, 64]);
    assert_eq!(pixel(&unmarked, 350, 192), [64, 64, 64]);
    let is_blue = |[red, _, blue]: [u8; 3]| blue > 150 && red < 80;
    assert!(is_blue(pixel(&unmarked, 310, 157)));

    let mut doc = marked(&sample(), AREA);
    doc.apply_redactions().expect("apply");
    // Black where the area is (the box over the blanked pixels), the picture as it was elsewhere.
    assert_eq!(pixel(&doc, 325, 127), [0, 0, 0]);
    assert_eq!(pixel(&doc, 350, 192), [64, 64, 64]);
    // The rectangle was touched: it is gone, also where the area does not reach.
    let after = pixel(&doc, 310, 157);
    assert!(!is_blue(after), "{after:?}");
    // The picture's own pixels changed, not only what is drawn over them.
    let bytes = saved(&doc);
    assert!(!has(&bytes, &"@".repeat(16)));
}

#[test]
fn text_in_a_form_object_and_invisible_text_are_removed_too() {
    let form_page = |content: &str, form: &str| {
        common::build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n{form}\nendstream",
                form.len()
            ),
        ])
    };
    for (name, pdf) in [
        (
            "a form object",
            form_page(
                "q /Fm1 Do Q\n",
                &format!("BT /F1 18 Tf 72 650 Td ({SECRET}) Tj ET"),
            ),
        ),
        (
            "invisible text (an OCR layer)",
            form_page(
                &format!("BT /F1 18 Tf 3 Tr 72 650 Td ({SECRET}) Tj ET\n"),
                "",
            ),
        ),
    ] {
        let mut doc = marked(&pdf, AREA);
        doc.apply_redactions().expect("apply");
        assert!(!text_of(&doc, 0).contains(SECRET), "{name}");
        assert!(!has(&saved(&doc), SECRET), "{name}");
    }
}

#[test]
fn chinese_text_in_an_embedded_font_is_removed() {
    // A page MuPDF lays out from HTML, with its bundled Chinese font embedded (B3-03).
    let html = "<p>公開的第一行</p><p>機密：合約編號 ALPHA-123</p><p>公開的第二行</p>";
    let mut builder = PdfBuilder::new();
    builder
        .add(html.as_bytes(), CreateKind::Html)
        .expect("html");
    let mut pdf = Vec::new();
    builder.finish(&mut pdf).expect("pdf");
    let doc = PdfDocument::from_bytes(&pdf).expect("open");
    assert!(text_of(&doc, 0).contains("合約編號"));
    let area = hit_area(&doc, 0, "機密：合約編號 ALPHA-123");
    drop(doc);

    let mut doc = marked(&pdf, area);
    doc.apply_redactions().expect("apply");
    let text = text_of(&doc, 0);
    assert!(
        !text.contains("合約編號") && !text.contains("ALPHA-123"),
        "{text}"
    );
    assert!(
        text.contains("公開的第一行") && text.contains("公開的第二行"),
        "{text}"
    );
    let again = PdfDocument::from_bytes(&saved(&doc)).expect("reopen");
    assert!(
        again
            .search_page(0, "合約編號", false, 8)
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(
        !again
            .search_page(0, "公開的第二行", false, 8)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn on_a_turned_page_the_area_is_where_the_page_shows_it() {
    let content = format!(
        "BT /F1 18 Tf 72 700 Td (Turned public line) Tj ET\nBT /F1 18 Tf 72 650 Td ({SECRET}) Tj ET\n"
    );
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ]);
    let doc = PdfDocument::from_bytes(&pdf).expect("open");
    // The area of the secret as search finds it, in the space the page is shown in.
    let area = hit_area(&doc, 0, SECRET);
    drop(doc);
    let mut doc = marked(&pdf, area);
    doc.apply_redactions().expect("apply");
    let text = text_of(&doc, 0);
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("Turned public line"), "{text}");
}

#[test]
fn a_field_loses_its_value_also_where_it_is_shown_outside_the_area() {
    // One field with two widgets: one in the area, one far below it.
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /DA (/Helv 0 Tf 0 g) >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /Helv 5 0 R >> >> /Annots [7 0 R 8 0 R] >>".into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /FT /Tx /T (account) /V ({SECRET} value) /DA (/Helv 12 Tf 0 g) /Kids [7 0 R 8 0 R] >>"),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [200 640 320 670] /F 4 /P 3 0 R >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [200 100 320 130] /F 4 /P 3 0 R >>".into(),
    ]);
    let mut doc = marked(&pdf, AREA);
    assert_eq!(doc.redaction_summary().unwrap().fields, 1);
    doc.apply_redactions().expect("apply");
    let fields = doc.page_fields(0).expect("fields");
    assert_eq!(fields.len(), 1, "the widget outside the area stays");
    assert!(!fields[0].value.contains(SECRET), "{:?}", fields[0].value);
    assert!(!has(&saved(&doc), SECRET));
}

#[test]
fn marks_can_be_moved_and_removed_before_they_are_applied() {
    let mut doc = marked(&sample(), AREA);
    let mark = doc
        .page_annotations(0)
        .unwrap()
        .into_iter()
        .find(|annotation| annotation.kind == AnnotationKind::Redaction)
        .expect("mark");
    // Moved away from the secret, onto the first line: that is what goes.
    let elsewhere = Rect {
        x0: 60.0,
        y0: 75.0,
        x1: 330.0,
        y1: 100.0,
    };
    doc.set_annotation_rect(0, mark.id, elsewhere)
        .expect("move");
    doc.apply_redactions().expect("apply");
    let text = text_of(&doc, 0);
    assert!(
        text.contains(SECRET) && !text.contains("Public line one"),
        "{text}"
    );

    // Removed: nothing is marked, and applying is refused.
    let mut doc = marked(&sample(), AREA);
    let mark = doc
        .page_annotations(0)
        .unwrap()
        .into_iter()
        .find(|annotation| annotation.kind == AnnotationKind::Redaction)
        .expect("mark");
    doc.delete_annotation(0, mark.id).expect("remove");
    assert_eq!(doc.redaction_summary().unwrap().marks, 0);
    assert!(doc.apply_redactions().is_err());
    assert!(text_of(&doc, 0).contains(SECRET));
}

#[test]
fn a_signed_document_is_neither_marked_nor_redacted() {
    let signed = corpus_bytes("benign/signed.pdf");
    let mut doc = PdfDocument::from_bytes(&signed).expect("open");
    assert!(doc.add_redaction(0, AREA).is_err());
    assert!(doc.apply_redactions().is_err());
    assert!(doc.page_annotations(0).unwrap().is_empty());
}

fn corpus_bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name),
    )
    .expect("corpus")
}

#[test]
fn a_docmdp_signed_document_is_neither_marked_nor_redacted() {
    let docmdp = corpus_bytes("benign/signed-docmdp-p1.pdf");
    let mut doc = PdfDocument::from_bytes(&docmdp).expect("open");
    assert!(doc.add_redaction(0, AREA).is_err());
    assert!(doc.apply_redactions().is_err());
    assert!(doc.page_annotations(0).unwrap().is_empty());
}

#[test]
fn a_scanned_page_loses_its_pixels_in_the_redacted_area() {
    let scan = corpus_bytes("benign/scanned-text.pdf");
    let unmarked = PdfDocument::from_bytes(&scan).expect("open");
    let rendered_unmarked = unmarked.render(0, 1.0, 0).expect("render");
    drop(unmarked);

    // In scanned-text.pdf (Letter size 612x792), "SECRET PAPER" is the second line (y: 200..260).
    let secret_area = Rect {
        x0: 40.0,
        y0: 200.0,
        x1: 350.0,
        y1: 260.0,
    };
    let mut doc = marked(&scan, secret_area);
    doc.apply_redactions().expect("apply");

    // Inside the redacted area, the pixels are now covered by black box [0, 0, 0].
    assert_eq!(pixel(&doc, 100, 220), [0, 0, 0]);

    // Outside the redacted area (the first line "PRIVACY FIRST" at y=150), pixels are preserved.
    let top_line_pixel = pixel(&doc, 100, 150);
    let at_unmarked = (150 * rendered_unmarked.width as usize + 100) * 4;
    assert_eq!(
        top_line_pixel,
        [
            rendered_unmarked.rgba[at_unmarked],
            rendered_unmarked.rgba[at_unmarked + 1],
            rendered_unmarked.rgba[at_unmarked + 2]
        ]
    );

    // When saved, the rewritten file re-opens and keeps the blacked out area.
    let bytes = saved(&doc);
    let again = PdfDocument::from_bytes(&bytes).expect("reopen");
    assert_eq!(pixel(&again, 100, 220), [0, 0, 0]);
}

#[test]
fn redaction_sample_from_corpus_has_all_elements_removed_except_out_of_scope() {
    let sample_pdf = corpus_bytes("benign/redaction-sample.pdf");
    let mut doc = marked(&sample_pdf, AREA);
    assert_eq!(
        doc.redaction_summary().expect("summary"),
        RedactionSummary {
            marks: 1,
            pages: 1,
            annotations: 1,
            fields: 1,
        }
    );
    doc.apply_redactions().expect("apply");
    let text = text_of(&doc, 0);
    assert!(!text.contains(SECRET), "{text}");
    assert!(
        text.contains("Public line one") && text.contains("Public line two"),
        "{text}"
    );
    assert!(
        doc.search_page(0, SECRET, false, 8)
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(doc.page_fields(0).unwrap().is_empty());

    let bytes = saved(&doc);
    for gone in [
        "SECRET-ALPHA-123 inside",
        "SECRET-ALPHA-123 in a note",
        "SECRET-ALPHA-123 in a field",
        "secret-alpha-123.example",
    ] {
        assert!(!has(&bytes, gone), "{gone} is still in the file");
    }
    // Out-of-scope items (bookmark and title) remain as specified in ADR 0016.
    for kept in ["SECRET-ALPHA-123 bookmark", "SECRET-ALPHA-123 title"] {
        assert!(has(&bytes, kept), "{kept}");
    }
}
