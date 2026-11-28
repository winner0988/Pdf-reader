// #447 的 POC，不合併。
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use ipc_contract::limits::{
    MAX_ANNOTATION_QUADS, MAX_ANNOTATIONS_PER_PAGE, MAX_INK_POINTS, MAX_INK_STROKES,
    MAX_NOTE_TEXT_BYTES, MAX_SOURCE_TEXT_BOXES, MAX_TEXT_BOX_BYTES,
};
use ipc_contract::types::{InkColor, InkWidth, Point, Quad, StampName, TextAlign, TextMarkKind};
use mupdf::pdf::{PdfDocument as MuPdfDocument, PdfObject};
use pdf_worker::engine::PdfDocument;

#[allow(dead_code)]
mod common;
#[path = "xfdf_poc/parser.rs"]
mod parser;

use parser::{
    FontData, ImportedItem, MAX_XFDF_BYTES, MAX_XFDF_IMPORT_BYTES, SkipReason, XfdfError,
    apply_imported, collect_page_geometries, parse_xfdf, resolve_text_boxes,
};

fn corpus(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn droid_font() -> FontData {
    let font = mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
        .expect("bundled Droid Sans Fallback");
    FontData {
        bytes: font.data.to_vec(),
        face: 0,
        name: "Droid Sans Fallback".to_string(),
    }
}

fn single_page_geometry() -> Vec<parser::PageGeometry> {
    collect_page_geometries(&corpus("benign/single-page.pdf")).expect("single-page geometry")
}

#[test]
fn poc1_rejects_malicious_and_malformed_xfdf_inputs_with_reasons_and_timings() {
    let pages = single_page_geometry();

    let too_large_xml = {
        let filler = "a".repeat(MAX_XFDF_BYTES + 32);
        format!("<xfdf><annots><!-- {filler} --></annots></xfdf>").into_bytes()
    };

    let exceeds_annots = {
        let mut s = String::from("<xfdf><annots>");
        for i in 0..10_001 {
            s.push_str(&format!(
                r#"<text page="0" rect="10,10,30,30" name="n{i}"><contents>ok</contents></text>"#
            ));
        }
        s.push_str("</annots></xfdf>");
        s.into_bytes()
    };

    let exceeds_import_budget = {
        let mut s = String::from("<xfdf><annots>");
        let text = "x".repeat(4000);
        // ~900 notes * 4000B = ~3.6 MB projected journal (> 3 MiB), while XML < 4 MB (<< 16 MiB).
        for i in 0..900 {
            s.push_str(&format!(
                r#"<text page="0" rect="10,10,30,30" name="b{i}"><contents>{text}</contents></text>"#
            ));
        }
        s.push_str("</annots></xfdf>");
        s.into_bytes()
    };

    let deep_nesting = {
        let mut s = String::from("<xfdf>");
        for _ in 0..10_000 {
            s.push_str("<a>");
        }
        for _ in 0..10_000 {
            s.push_str("</a>");
        }
        s.push_str("</xfdf>");
        s.into_bytes()
    };

    let mut utf16_le = vec![0xFF, 0xFE];
    for cu in "<xfdf><annots/></xfdf>".encode_utf16() {
        utf16_le.extend_from_slice(&cu.to_le_bytes());
    }
    let mut utf16_be = vec![0xFE, 0xFF];
    for cu in "<xfdf><annots/></xfdf>".encode_utf16() {
        utf16_be.extend_from_slice(&cu.to_be_bytes());
    }

    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "billion_laughs_doctype",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE lolz [
  <!ENTITY lol "lol">
  <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
  <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&lol3;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "xxe_file_uri",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE foo [ <!ENTITY xxe SYSTEM "file:///C:/Windows/win.ini"> ]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&xxe;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "xxe_http_uri",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE foo [ <!ENTITY xxe SYSTEM "http://127.0.0.1:9/ssrf"> ]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&xxe;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "undeclared_entity_text",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&foo;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "undeclared_entity_attr",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30" subject="&foo;"><contents>ok</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "illegal_char_ref_nul",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&#0;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "illegal_char_ref_ctrl_and_ffff",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&#1;&#xFFFF;</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        ("deep_nesting_10000", deep_nesting),
        ("too_many_annotations_10001", exceeds_annots),
        ("exceeds_max_xfdf_import_bytes", exceeds_import_budget),
        ("exceeds_max_xfdf_bytes_16mib", too_large_xml),
        ("utf16_le_bom", utf16_le),
        ("utf16_be_bom", utf16_be),
        (
            "declared_encoding_utf16",
            br#"<?xml version="1.0" encoding="UTF-16"?><xfdf><annots/></xfdf>"#.to_vec(),
        ),
        (
            "declared_encoding_iso8859_1",
            br#"<?xml version="1.0" encoding="ISO-8859-1"?><xfdf><annots/></xfdf>"#.to_vec(),
        ),
        (
            "wrong_root_element",
            br#"<pdf><annots/></pdf>"#.to_vec(),
        ),
        (
            "wrong_xmlns",
            br#"<xfdf xmlns="https://evil.example/ns"><annots/></xfdf>"#.to_vec(),
        ),
        (
            "mismatched_end_tag",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30"></square></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "unclosed_element",
            br#"<xfdf><annots><text page="0" rect="10,10,30,30">"#.to_vec(),
        ),
        (
            "two_root_elements",
            br#"<xfdf><annots/></xfdf><xfdf><annots/></xfdf>"#.to_vec(),
        ),
        (
            "duplicate_attribute",
            br#"<xfdf><annots><text page="0" page="0" rect="10,10,30,30"><contents>hi</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
        (
            "unquoted_attribute",
            br#"<xfdf><annots><text page=0 rect="10,10,30,30"><contents>hi</contents></text></annots></xfdf>"#
                .to_vec(),
        ),
    ];

    println!("--- POC 1 Malicious / Malformed Rejections ---");
    for (label, bytes) in cases {
        let start = Instant::now();
        let err = parse_xfdf(&bytes, &pages, MAX_XFDF_IMPORT_BYTES)
            .expect_err(&format!("case '{label}' MUST be rejected"));
        let elapsed_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        println!(
            "REJECTED {:<32} len={:>9}B elapsed={:>8.1}us reason={}",
            label,
            bytes.len(),
            elapsed_us,
            err
        );

        // Sanity-check key error classifications.
        match label {
            "billion_laughs_doctype" | "xxe_file_uri" | "xxe_http_uri" => {
                assert_eq!(err, XfdfError::DocTypeForbidden);
            }
            "exceeds_max_xfdf_bytes_16mib" => {
                assert!(matches!(err, XfdfError::TooLargeXml { .. }));
            }
            "exceeds_max_xfdf_import_bytes" => {
                assert!(matches!(err, XfdfError::TooMuchImportData { .. }));
            }
            "too_many_annotations_10001" => {
                assert!(matches!(err, XfdfError::TooManyAnnotations { .. }));
            }
            "deep_nesting_10000" => {
                assert!(matches!(err, XfdfError::DepthExceeded { .. }));
            }
            _ => {}
        }
    }
}

#[test]
fn poc1_imports_all_11_kinds_sanitizes_and_writes_clean_pdf() {
    let base_pdf = corpus("benign/single-page.pdf");
    let pages = collect_page_geometries(&base_pdf).expect("pages");
    assert_eq!(pages.len(), 1);

    let xfdf = br##"<?xml version="1.0" encoding="UTF-8"?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <f href="C:\Users\Secret\document.pdf"/>
  <annots>
    <highlight page="0" rect="72,680,220,700" color="#FFEB00" name="id-1" title="Alice" date="D:20261128120000Z" creationdate="D:20261128120000Z" subject="Subj" coords="72,700,220,700,72,680,220,680"/>
    <underline page="0" rect="72,650,220,668" color="#8CE659" coords="72,668,220,668,72,650,220,650"/>
    <strikeout page="0" rect="72,620,220,638" color="#73BFFF" coords="72,638,220,638,72,620,220,620"/>
    <squiggly page="0" rect="72,590,220,608" color="#8000FF" coords="72,608,220,608,72,590,220,590"/>
    <text page="0" rect="60,540,80,560" title="SecretAuthor" name="secret-nm" creationdate="D:20261128" date="D:20261128">
      <contents-richtext><body xmlns="http://www.w3.org/1999/xhtml"><p>Line 1</p><p>Line 2 &amp; bold</p></body></contents-richtext>
    </text>
    <ink page="0" rect="70,440,210,500" color="#D91A1A" width="2.5">
      <inklist>
        <gesture>72,450;120,480;180,460</gesture>
        <gesture>100,490;200,490</gesture>
      </inklist>
    </ink>
    <stamp page="0" rect="250,620,410,670" icon="Approved"/>
    <square page="0" rect="250,530,380,590" color="#1A4DD9" width="5" interior-color="#FFFF00"/>
    <circle page="0" rect="250,440,360,510" color="#1A9933" width="1"/>
    <line page="0" rect="250,360,400,410" start="400,410" end="250,360" head="ClosedArrow" tail="None" color="#000000" width="2.5"/>
    <freetext page="0" rect="72,260,320,340" justification="1">
      <defaultappearance>/Helv 14 Tf 0.1 0.2 0.8 rg</defaultappearance>
      <defaultstyle>font: 14pt "Droid Sans Fallback"; color: #1a33cc; text-align: center</defaultstyle>
      <contents>Hello XFDF TextBox</contents>
    </freetext>
  </annots>
</xfdf>"##;

    let start = Instant::now();
    let mut report = parse_xfdf(xfdf, &pages, MAX_XFDF_IMPORT_BYTES).expect("parse all 11");
    let resolved = resolve_text_boxes(&mut report, MAX_XFDF_IMPORT_BYTES, |_| Some(droid_font()))
        .expect("resolve freetext");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "POC 1 11-kind import+prep elapsed={elapsed_ms:.2}ms imported={} skips={} adjusted={}",
        report.imported_count,
        report.skips.total(),
        report.style_adjusted_count
    );

    assert_eq!(report.imported_count, 11);
    assert_eq!(report.skips.total(), 0);
    // Squiggly (#8000FF -> nearest Pink/Blue) and Square (interior-color) adjusted style.
    assert_eq!(report.style_adjusted_count, 2);
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        report.f_href.as_deref(),
        Some("C:\\Users\\Secret\\document.pdf")
    );

    // Check head-only arrow was swapped so arrow is at `to`.
    assert!(report.items.iter().any(|item| matches!(
        item,
        ImportedItem::Shape {
            shape: ipc_contract::types::Shape::Line { arrow: true, from, to },
            ..
        } if (from.x - 250.0).abs() < 1.0 && (to.x - 400.0).abs() < 1.0
    )));

    // Check note richtext extraction.
    assert!(report.items.iter().any(|item| matches!(
        item,
        ImportedItem::Note { text, .. } if text == "Line 1\nLine 2 & bold"
    )));

    // Check FreeText justification=1 -> Center.
    assert!(report.items.iter().any(|item| matches!(
        item,
        ImportedItem::FreeText { style, .. } if style.align == TextAlign::Center && style.size == 14.0
    )));

    // Check kind conversions present.
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Highlight {
            mark_kind: TextMarkKind::Underline,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Highlight {
            mark_kind: TextMarkKind::StrikeOut,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Highlight {
            mark_kind: TextMarkKind::Squiggly,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Stamp {
            stamp: StampName::Approved,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Ink {
            color: InkColor::Red,
            width: InkWidth::Medium,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Shape {
            shape: ipc_contract::types::Shape::Rectangle { .. },
            color: InkColor::Blue,
            width: InkWidth::Thick,
            ..
        }
    )));
    assert!(report.items.iter().any(|i| matches!(
        i,
        ImportedItem::Shape {
            shape: ipc_contract::types::Shape::Ellipse { .. },
            color: InkColor::Green,
            width: InkWidth::Thin,
            ..
        }
    )));

    // Apply all edits to PdfDocument, save, verify qpdf + no metadata leakage / active actions.
    let mut doc = PdfDocument::from_bytes(&base_pdf).expect("open doc");
    let (applied, touched) = apply_imported(&mut doc, &report, &resolved).expect("apply all");
    assert_eq!(applied, 11);
    assert_eq!(touched, vec![0]);

    let mut saved = Vec::new();
    doc.save(&mut saved).expect("save");
    common::qpdf::check_qpdf_bytes(&saved);

    let reread = PdfDocument::from_bytes(&saved).expect("reopen saved");
    let annots = reread.page_annotations(0).expect("page_annotations");
    assert_eq!(annots.len(), 11);

    // Inspect raw PDF annotation dicts for forbidden keys (/T, /M, /CreationDate, /NM, /A, /AA, /JS, /Launch).
    let raw_doc = MuPdfDocument::try_from(
        mupdf::Document::from_bytes(&saved, "application/pdf").expect("raw"),
    )
    .expect("pdf");
    let page = raw_doc.load_pdf_page(0).expect("page 0");
    for annot in page.annotations() {
        let obj = annot.object();
        for forbidden in ["T", "M", "CreationDate", "NM", "A", "AA", "JS", "Launch"] {
            assert!(
                obj.get_dict(forbidden).ok().flatten().is_none(),
                "annotation dictionary unexpectedly kept /{forbidden}"
            );
        }
    }
}

#[test]
fn poc1_converts_rotated_page_and_shifted_mediabox_coordinates() {
    let mut base_doc = PdfDocument::from_bytes(&corpus("benign/rotated-page.pdf")).expect("open");
    // Construct shifted MediaBox [50 50 662 842] on rotated page 1 via raw mupdf save,
    // place known page-space annotations, export them, and re-import through XFDF parser.
    let shifted_pdf = {
        let mupdf_doc = MuPdfDocument::try_from(
            mupdf::Document::from_bytes(&corpus("benign/rotated-page.pdf"), "application/pdf")
                .expect("mupdf"),
        )
        .expect("pdf");
        let page = mupdf_doc.load_pdf_page(1).expect("page 1");
        let mut mbox = mupdf_doc.new_array().expect("mbox");
        for v in [50.0, 50.0, 662.0, 842.0] {
            mbox.array_push(PdfObject::new_real(v).expect("real"))
                .expect("push");
        }
        page.object().dict_put("MediaBox", mbox).expect("MediaBox");
        drop(page);
        let mut buf = Vec::new();
        mupdf_doc.write_to(&mut buf).expect("write shifted pdf");
        buf
    };
    let _ = &mut base_doc;

    let mut work_doc = PdfDocument::from_bytes(&shifted_pdf).expect("reopen shifted");
    let expected_quad = Quad {
        ul: Point { x: 100.0, y: 120.0 },
        ur: Point { x: 260.0, y: 120.0 },
        ll: Point { x: 100.0, y: 140.0 },
        lr: Point { x: 260.0, y: 140.0 },
    };
    work_doc
        .add_text_marks(
            TextMarkKind::Highlight,
            &[ipc_contract::types::HighlightMark {
                page: 1,
                quads: vec![expected_quad],
            }],
            ipc_contract::types::HighlightColor::Yellow,
        )
        .expect("add highlight");
    work_doc
        .add_note(1, Point { x: 150.0, y: 200.0 }, "Rotated note")
        .expect("add note");

    let exported = work_doc.annotation_export().expect("export");
    let hl_exp = exported
        .items
        .iter()
        .find(|i| !i.quads.is_empty())
        .expect("exported highlight");
    let note_exp = exported
        .items
        .iter()
        .find(|i| i.contents.as_deref() == Some("Rotated note"))
        .expect("exported note");

    let qp = hl_exp.quads[0];
    let xfdf = format!(
        r##"<xfdf><annots>
<highlight page="1" rect="{},{},{},{}" coords="{},{},{},{},{},{},{},{}"/>
<text page="1" rect="{},{},{},{}"><contents>Rotated note</contents></text>
</annots></xfdf>"##,
        hl_exp.rect[0],
        hl_exp.rect[1],
        hl_exp.rect[2],
        hl_exp.rect[3],
        qp[0],
        qp[1],
        qp[2],
        qp[3],
        qp[4],
        qp[5],
        qp[6],
        qp[7],
        note_exp.rect[0],
        note_exp.rect[1],
        note_exp.rect[2],
        note_exp.rect[3],
    );

    let clean_pages = collect_page_geometries(&shifted_pdf).expect("shifted page geom");
    let report = parse_xfdf(xfdf.as_bytes(), &clean_pages, MAX_XFDF_IMPORT_BYTES)
        .expect("parse rotated+shifted");
    assert_eq!(report.imported_count, 2);

    match &report.items[0] {
        ImportedItem::Highlight { page, quads, .. } => {
            assert_eq!(*page, 1);
            let q = quads[0];
            assert!((q.ul.x - expected_quad.ul.x).abs() < 0.05);
            assert!((q.ul.y - expected_quad.ul.y).abs() < 0.05);
            assert!((q.lr.x - expected_quad.lr.x).abs() < 0.05);
            assert!((q.lr.y - expected_quad.lr.y).abs() < 0.05);
        }
        other => panic!("unexpected item[0]: {other:?}"),
    }
    match &report.items[1] {
        ImportedItem::Note { page, at, text } => {
            assert_eq!(*page, 1);
            assert_eq!(text, "Rotated note");
            assert!((at.x - 150.0).abs() < 0.05);
            assert!((at.y - 200.0).abs() < 0.05);
        }
        other => panic!("unexpected item[1]: {other:?}"),
    }
}

#[test]
fn poc1_covers_all_six_skip_reasons() {
    let base_pdf = corpus("benign/single-page.pdf");
    let mut pages = collect_page_geometries(&base_pdf).expect("pages");
    let long_text = "A".repeat((MAX_NOTE_TEXT_BYTES as usize) + 5);
    let long_tb_text = "B".repeat(MAX_TEXT_BOX_BYTES + 5);
    let many_quads = (0..=MAX_ANNOTATION_QUADS)
        .map(|i| {
            let x = 10.0 + (i % 10) as f32 * 5.0;
            format!("{x},700,{},700,{x},690,{},690", x + 3.0, x + 3.0)
        })
        .collect::<Vec<_>>()
        .join(",");
    let many_strokes: String = (0..=MAX_INK_STROKES)
        .map(|i| format!("<gesture>10,{};20,{}</gesture>", 10 + i, 12 + i))
        .collect();
    let many_points: String = (0..=MAX_INK_POINTS)
        .map(|i| format!("{},{}", 10 + (i % 400), 10 + (i % 400)))
        .collect::<Vec<_>>()
        .join(";");

    let mut doc_xml = format!(
        r##"<xfdf><annots>
  <caret page="0" rect="10,10,30,30"/>
  <stamp page="0" rect="10,10,100,40" icon="Experimental"/>
  <stamp page="0" rect="10,10,100,40" icon="Approved"><imagedata>AAAA</imagedata></stamp>
  <text page="0" rect="10,10,30,30" flags="print,hidden"><contents>hidden</contents></text>
  <text page="0" rect="10,10,30,30" flags="noview"><contents>noview</contents></text>
  <text page="0" rect="10,10,30,30" inreplyto="parent-1"><contents>reply</contents></text>
  <text page="99" rect="10,10,30,30"><contents>bad page</contents></text>
  <text page="abc" rect="10,10,30,30"><contents>non numeric page</contents></text>
  <text page="0" rect="10,10,nan,30"><contents>nan</contents></text>
  <text page="0" rect="10,10,1e39,30"><contents>inf</contents></text>
  <text page="0" rect="9999,9999,10020,10020"><contents>off page</contents></text>
  <stamp page="0" rect="10,10,14,14" icon="Approved"/>
  <text page="0" rect="10,10,30,30"><contents>   </contents></text>
  <highlight page="0" rect="10,680,100,710" coords="{many_quads}"/>
  <ink page="0" rect="10,10,300,300"><inklist>{many_strokes}</inklist></ink>
  <ink page="0" rect="10,10,500,500"><inklist><gesture>{many_points}</gesture></inklist></ink>
  <text page="0" rect="10,10,30,30"><contents>{long_text}</contents></text>
  <freetext page="0" rect="10,10,200,50"><contents>{long_tb_text}</contents></freetext>
  <freetext page="0" rect="10,100,200,150"><contents>MissingGlyph: 𠀀</contents></freetext>
</annots></xfdf>"##
    );

    // Use Droid fallback only, which lacks rare CJK Extension B U+20000 for FontUnavailable test.
    let mut report = parse_xfdf(doc_xml.as_bytes(), &pages, MAX_XFDF_IMPORT_BYTES).expect("parse");
    let _ = resolve_text_boxes(&mut report, MAX_XFDF_IMPORT_BYTES, |fam| {
        if fam.eq_ignore_ascii_case("Droid Sans Fallback") || fam == FALLBACK_FIRST {
            Some(droid_font())
        } else {
            None
        }
    })
    .expect("resolve");

    assert_eq!(report.skips.count(SkipReason::UnsupportedKind), 5);
    assert_eq!(report.skips.count(SkipReason::Reply), 1);
    assert_eq!(report.skips.count(SkipReason::PageOutOfRange), 2);
    assert_eq!(report.skips.count(SkipReason::InvalidGeometryOrFormat), 5);
    assert_eq!(report.skips.count(SkipReason::ExceedsLimit), 5);
    assert_eq!(report.skips.count(SkipReason::FontUnavailable), 1);

    // Also test 2000 per-page limit and 1000 text-box limit.
    pages[0].existing_annots = MAX_ANNOTATIONS_PER_PAGE - 1;
    doc_xml = r#"<xfdf><annots>
      <text page="0" rect="10,10,30,30"><contents>fits</contents></text>
      <text page="0" rect="40,10,60,30"><contents>over page cap</contents></text>
    </annots></xfdf>"#
        .to_string();
    let cap_report =
        parse_xfdf(doc_xml.as_bytes(), &pages, MAX_XFDF_IMPORT_BYTES).expect("page cap parse");
    assert_eq!(cap_report.imported_count, 1);
    assert_eq!(cap_report.skips.count(SkipReason::ExceedsLimit), 1);

    pages[0].existing_annots = 0;
    let mut tb_xml = String::from("<xfdf><annots>");
    for i in 0..=MAX_SOURCE_TEXT_BOXES {
        tb_xml.push_str(&format!(
            r#"<freetext page="0" rect="10,10,100,40"><contents>b{i}</contents></freetext>"#
        ));
    }
    tb_xml.push_str("</annots></xfdf>");
    let tb_cap_report = parse_xfdf(tb_xml.as_bytes(), &pages, usize::MAX).expect("tb cap parse");
    assert_eq!(tb_cap_report.imported_count, MAX_SOURCE_TEXT_BOXES as usize);
    assert_eq!(tb_cap_report.skips.count(SkipReason::ExceedsLimit), 1);

    // Verify all 6 SkipReason variants covered distinctly.
    let mut reasons = HashSet::new();
    for s in &report.per_annotation {
        if let parser::AnnotationOutcome::Skipped(r, _) = s.outcome {
            reasons.insert(r);
        }
    }
    assert_eq!(reasons.len(), 6);
}

const FALLBACK_FIRST: &str = "Microsoft JhengHei";
