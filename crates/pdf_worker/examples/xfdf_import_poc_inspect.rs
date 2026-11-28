// #447 的 POC，不合併。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pdf_worker::engine::PdfDocument;

#[path = "../tests/xfdf_poc/parser.rs"]
mod parser;

use parser::{
    AnnotationOutcome, FALLBACK_FONT_FAMILIES, FontData, MAX_XFDF_BYTES, MAX_XFDF_IMPORT_BYTES,
    SkipReason, apply_imported, collect_page_geometries, parse_xfdf, resolve_text_boxes,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--malicious-bench") {
        let case = args.get(2).map(String::as_str).unwrap_or("help");
        let hold_ms: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1200);
        run_malicious_bench(case, hold_ms);
        return;
    }

    if args.len() < 3 {
        eprintln!(
            "Usage:\n  xfdf_import_poc_inspect <pdf> <xfdf> [out_dir]\n  xfdf_import_poc_inspect --malicious-bench <case> [hold_ms]"
        );
        std::process::exit(2);
    }

    let pdf_path = Path::new(&args[1]);
    let xfdf_path = Path::new(&args[2]);
    let out_dir = args.get(3).map(PathBuf::from);

    let pdf_bytes = std::fs::read(pdf_path)
        .unwrap_or_else(|e| panic!("failed to read PDF {}: {e}", pdf_path.display()));
    let xfdf_bytes = std::fs::read(xfdf_path)
        .unwrap_or_else(|e| panic!("failed to read XFDF {}: {e}", xfdf_path.display()));

    let pages = collect_page_geometries(&pdf_bytes)
        .unwrap_or_else(|e| panic!("failed to inspect PDF pages: {e}"));

    println!("=== PDF Summary ===");
    println!("File: {}", pdf_path.display());
    println!("Pages: {}", pages.len());
    for (idx, pg) in pages.iter().enumerate() {
        println!(
            "  Page {}: bounds=({:.1},{:.1},{:.1},{:.1}) ctm=[{:.3},{:.3},{:.3},{:.3},{:.1},{:.1}] existing_annots={}",
            idx,
            pg.bounds.x0,
            pg.bounds.y0,
            pg.bounds.x1,
            pg.bounds.y1,
            pg.ctm.a,
            pg.ctm.b,
            pg.ctm.c,
            pg.ctm.d,
            pg.ctm.e,
            pg.ctm.f,
            pg.existing_annots
        );
    }

    println!("\n=== XFDF Summary ===");
    println!("File: {} ({} bytes)", xfdf_path.display(), xfdf_bytes.len());
    let t0 = Instant::now();
    let mut report = match parse_xfdf(&xfdf_bytes, &pages, MAX_XFDF_IMPORT_BYTES) {
        Ok(r) => r,
        Err(err) => {
            println!("REJECTED: {err}");
            std::process::exit(1);
        }
    };

    let font_map = discover_local_fonts();
    let resolved = resolve_text_boxes(&mut report, MAX_XFDF_IMPORT_BYTES, |family| {
        find_font_for_family(&font_map, family)
    })
    .unwrap_or_else(|e| {
        println!("REJECTED during text box resolution: {e}");
        std::process::exit(1);
    });
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    println!("Has DOCTYPE: {}", report.has_doctype);
    println!(
        "Declared encoding: {}",
        report
            .xml_encoding
            .as_deref()
            .unwrap_or("<omitted (UTF-8)>")
    );
    println!("Uses XFDF XML namespace: {}", report.used_namespace);
    println!("<f href>: {}", report.f_href.as_deref().unwrap_or("<none>"));
    println!("Parse+Prep time: {elapsed_ms:.2} ms");
    println!(
        "Estimated journal delta: {} bytes",
        report.estimated_journal_bytes
    );

    println!("\nElements seen (name: count):");
    for (name, cnt) in &report.element_counts {
        println!("  {name}: {cnt}");
    }
    println!("\nAttributes seen (name: count):");
    for (name, cnt) in &report.attribute_counts {
        println!("  {name}: {cnt}");
    }

    println!("\n=== Outcome Totals ===");
    println!("Total <annots> entries: {}", report.total_annots_seen);
    println!(
        "Imported: {} (style adjusted: {})",
        report.imported_count, report.style_adjusted_count
    );
    println!("Skipped total: {}", report.skips.total());
    for reason in [
        SkipReason::UnsupportedKind,
        SkipReason::Reply,
        SkipReason::PageOutOfRange,
        SkipReason::InvalidGeometryOrFormat,
        SkipReason::ExceedsLimit,
        SkipReason::FontUnavailable,
    ] {
        println!("  {}: {}", reason.label(), report.skips.count(reason));
    }

    println!("\n=== Per-Annotation Details ===");
    for ann in &report.per_annotation {
        let page_str = ann.page_attr.as_deref().unwrap_or("?");
        let status = match &ann.outcome {
            AnnotationOutcome::Imported(_) => "IMPORTED".to_string(),
            AnnotationOutcome::Skipped(r, _) => format!("SKIPPED({})", r.label()),
        };
        println!(
            "#{} <{}> page={page_str} status={status} style_adjusted={}",
            ann.index, ann.tag, ann.style_adjusted
        );
        println!("   summary/reason: {}", ann.converted_summary);
        if let Some(icon) = &ann.stamp_icon {
            println!("   stamp icon: {icon}");
        }
        if let Some(rt) = &ann.richtext_plain {
            println!("   <contents-richtext> plain text: {:?}", rt);
        }
    }

    let mut doc = PdfDocument::from_bytes(&pdf_bytes).expect("open PDF for apply");
    let (applied, mut touched_pages) =
        apply_imported(&mut doc, &report, &resolved).expect("apply edits to PDF");
    println!("\nApplied {applied} annotations to PDF document.");

    if let Some(dir) = out_dir {
        std::fs::create_dir_all(&dir).expect("create out_dir");
        if touched_pages.is_empty() && !pages.is_empty() {
            touched_pages.push(0);
        }
        for page_idx in touched_pages {
            let png = doc.render_png(page_idx, 1.5).expect("render PNG");
            let out_png = dir.join(format!("poc2_page_{}.png", page_idx + 1));
            std::fs::write(&out_png, &png).expect("write PNG");
            println!("Rendered PNG: {} ({} bytes)", out_png.display(), png.len());
        }
    }
}

fn discover_local_fonts() -> HashMap<String, (PathBuf, u32, String)> {
    let mut map = HashMap::new();
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let fonts_dir = windir.join("Fonts");
        for (names, file, face) in [
            (
                &["Microsoft JhengHei", "Microsoft JhengHei UI"][..],
                "msjh.ttc",
                0u32,
            ),
            (
                &["Microsoft YaHei", "Microsoft YaHei UI"][..],
                "msyh.ttc",
                0u32,
            ),
            (&["Yu Gothic"][..], "YuGothM.ttc", 0u32),
            (&["Segoe UI"][..], "segoeui.ttf", 0u32),
            (&["Arial", "Helvetica"][..], "arial.ttf", 0u32),
            (&["Courier New"][..], "cour.ttf", 0u32),
            (&["Times New Roman"][..], "times.ttf", 0u32),
        ] {
            let p = fonts_dir.join(file);
            if p.exists() {
                for &n in names {
                    map.insert(n.to_ascii_lowercase(), (p.clone(), face, n.to_string()));
                }
            }
        }
    }
    let _ = &FALLBACK_FONT_FAMILIES;
    map
}

fn find_font_for_family(
    local_map: &HashMap<String, (PathBuf, u32, String)>,
    family: &str,
) -> Option<FontData> {
    if let Some((path, face, canonical)) = local_map.get(&family.to_ascii_lowercase())
        && let Ok(bytes) = std::fs::read(path)
    {
        return Some(FontData {
            bytes,
            face: *face,
            name: canonical.clone(),
        });
    }
    if (family.eq_ignore_ascii_case("Arial") || family.eq_ignore_ascii_case("Microsoft JhengHei"))
        && let Some(droid) = mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
    {
        return Some(FontData {
            bytes: droid.data.to_vec(),
            face: 0,
            name: "Droid Sans Fallback".to_string(),
        });
    }
    None
}

fn run_malicious_bench(case: &str, hold_ms: u64) {
    let single_pdf = build_simple_pdf();
    let pages = collect_page_geometries(&single_pdf).expect("pages");
    let payload: Vec<u8> = match case {
        "billion_laughs_doctype" => br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE lolz [
  <!ENTITY lol "lol">
  <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
  <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&lol3;</contents></text></annots></xfdf>"#
            .to_vec(),
        "xxe_file_uri" => br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE foo [ <!ENTITY xxe SYSTEM "file:///C:/Windows/win.ini"> ]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&xxe;</contents></text></annots></xfdf>"#
            .to_vec(),
        "xxe_http_uri" => br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE foo [ <!ENTITY xxe SYSTEM "http://127.0.0.1:9/ssrf"> ]>
<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&xxe;</contents></text></annots></xfdf>"#
            .to_vec(),
        "undeclared_entity" => br#"<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&foo;</contents></text></annots></xfdf>"#.to_vec(),
        "illegal_char_ref" => br#"<xfdf><annots><text page="0" rect="10,10,30,30"><contents>&#0;&#1;&#xFFFF;</contents></text></annots></xfdf>"#.to_vec(),
        "deep_nesting_10000" => {
            let mut s = String::from("<xfdf>");
            for _ in 0..10_000 { s.push_str("<a>"); }
            for _ in 0..10_000 { s.push_str("</a>"); }
            s.push_str("</xfdf>");
            s.into_bytes()
        }
        "too_many_annotations_10001" => {
            let mut s = String::from("<xfdf><annots>");
            for i in 0..10_001 {
                s.push_str(&format!(
                    r#"<text page="0" rect="10,10,30,30" name="n{i}"><contents>ok</contents></text>"#
                ));
            }
            s.push_str("</annots></xfdf>");
            s.into_bytes()
        }
        "exceeds_max_xfdf_import_bytes" => {
            let mut s = String::from("<xfdf><annots>");
            let text = "x".repeat(4000);
            for i in 0..900 {
                s.push_str(&format!(
                    r#"<text page="0" rect="10,10,30,30" name="b{i}"><contents>{text}</contents></text>"#
                ));
            }
            s.push_str("</annots></xfdf>");
            s.into_bytes()
        }
        "exceeds_max_xfdf_bytes_16mib" => {
            let filler = "a".repeat(MAX_XFDF_BYTES + 32);
            format!("<xfdf><annots><!-- {filler} --></annots></xfdf>").into_bytes()
        }
        "utf16_le_bom" => {
            let mut out = vec![0xFF, 0xFE];
            for cu in "<xfdf><annots/></xfdf>".encode_utf16() {
                out.extend_from_slice(&cu.to_le_bytes());
            }
            out
        }
        "utf16_be_bom" => {
            let mut out = vec![0xFE, 0xFF];
            for cu in "<xfdf><annots/></xfdf>".encode_utf16() {
                out.extend_from_slice(&cu.to_be_bytes());
            }
            out
        }
        "declared_encoding_utf16" => {
            br#"<?xml version="1.0" encoding="UTF-16"?><xfdf><annots/></xfdf>"#.to_vec()
        }
        "wrong_root_or_xmlns" => {
            br#"<xfdf xmlns="https://evil.example/ns"><annots/></xfdf>"#.to_vec()
        }
        "malformed_xml" => {
            br#"<xfdf><annots><text page="0" page="0" rect=10></square></annots></xfdf>"#.to_vec()
        }
        other => panic!("unknown bench case '{other}'"),
    };

    let t0 = Instant::now();
    let res = parse_xfdf(&payload, &pages, MAX_XFDF_IMPORT_BYTES);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "case={case} input_bytes={} elapsed_ms={elapsed_ms:.3} result={:?}",
        payload.len(),
        res.as_ref().err().map(ToString::to_string)
    );
    assert!(res.is_err(), "expected rejection for {case}");
    if hold_ms > 0 {
        std::thread::sleep(Duration::from_millis(hold_ms));
    }
}

fn build_simple_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
