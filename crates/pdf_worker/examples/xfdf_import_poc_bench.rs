// #447 的 POC，不合併。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ipc_contract::types::{Edit, TextBoxModelId};
use ipc_contract::worker::TextBoxModel;
use pdf_worker::engine::PdfDocument;
use serde_json::json;

#[path = "../tests/xfdf_poc/parser.rs"]
mod parser;

use parser::{FontData, apply_imported, collect_page_geometries, parse_xfdf, resolve_text_boxes};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hold_ms: u64 = args
        .windows(2)
        .find(|w| w[0] == "--hold-ms")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(0);
    let scenario_filter: Option<String> = args
        .windows(2)
        .find(|w| w[0] == "--scenario")
        .map(|w| w[1].clone());

    let pdf_pages = 20u32;
    let pdf_bytes = build_multi_page_pdf(pdf_pages);
    let page_geoms = collect_page_geometries(&pdf_bytes).expect("page geometries");

    let (font_label, font_data) = load_benchmark_cjk_font();
    println!(
        "=== POC 3 Benchmark ===\nPDF: {} pages ({} bytes)\nCJK font: {} (face={}, size={} bytes)",
        pdf_pages,
        pdf_bytes.len(),
        font_label,
        font_data.face,
        font_data.bytes.len()
    );

    if scenario_filter.is_none() || scenario_filter.as_deref() == Some("mixed_10000") {
        bench_mixed_10000(&pdf_bytes, &page_geoms);
    }

    for count in [100usize, 300, 500] {
        let key = format!("cjk_{count}");
        if scenario_filter.is_none() || scenario_filter.as_deref() == Some(&key) {
            bench_chinese_text_boxes(&pdf_bytes, &page_geoms, &font_data, count);
        }
    }

    if scenario_filter.is_none() || scenario_filter.as_deref() == Some("threshold_scan") {
        scan_journal_4mib_threshold(&page_geoms, &font_data);
    }

    if hold_ms > 0 {
        std::thread::sleep(Duration::from_millis(hold_ms));
    }
}

fn bench_mixed_10000(pdf_bytes: &[u8], page_geoms: &[parser::PageGeometry]) {
    let xfdf = generate_mixed_xfdf(10_000, page_geoms.len() as u32);
    println!(
        "\n--- Scenario A: 10,000 mixed annotations (XFDF size = {} bytes) ---",
        xfdf.len()
    );

    let mut parse_ms = Vec::new();
    let mut apply_ms = Vec::new();
    let mut undo_ms = Vec::new();
    let mut journal_bytes = 0usize;
    let mut imported = 0usize;
    let mut est_bytes = 0usize;

    for run in 0..3 {
        let t_parse = Instant::now();
        let report = parse_xfdf(&xfdf, page_geoms, usize::MAX).expect("parse 10k");
        parse_ms.push(t_parse.elapsed().as_secs_f64() * 1000.0);
        imported = report.imported_count;
        est_bytes = report.estimated_journal_bytes;

        let t_apply = Instant::now();
        let mut doc = PdfDocument::from_bytes(pdf_bytes).expect("open pdf");
        let (applied, _) = apply_imported(&mut doc, &report, &[]).expect("apply 10k");
        assert_eq!(applied, 10_000);
        apply_ms.push(t_apply.elapsed().as_secs_f64() * 1000.0);

        // Undo per ADR 0013: reopen original PDF bytes and replay all edits except the last one.
        let mut undo_report = report.clone();
        undo_report.items.pop();
        let t_undo = Instant::now();
        let mut reopen = PdfDocument::from_bytes(pdf_bytes).expect("reopen original");
        let (replayed, _) = apply_imported(&mut reopen, &undo_report, &[]).expect("replay 9999");
        assert_eq!(replayed, 9999);
        undo_ms.push(t_undo.elapsed().as_secs_f64() * 1000.0);

        if run == 0 {
            let edits: Vec<Edit> = report.items.iter().map(|i| i.to_edit(None)).collect();
            journal_bytes = stored_journal_json_len(pdf_bytes.len(), &edits, &[]);
        }
    }

    print_timing_stats("parse", &parse_ms);
    println!("  prep: 0.00 ms (no text boxes in Scenario A)");
    print_timing_stats("apply (10,000 edits)", &apply_ms);
    print_timing_stats("undo replay (reopen + 9,999 edits)", &undo_ms);
    println!(
        "  imported={imported}, parser_est_journal_bytes={est_bytes}, actual_stored_journal_bytes={journal_bytes} ({:.2} MiB)",
        journal_bytes as f64 / (1024.0 * 1024.0)
    );
}

fn bench_chinese_text_boxes(
    pdf_bytes: &[u8],
    page_geoms: &[parser::PageGeometry],
    font_data: &FontData,
    count: usize,
) {
    let xfdf = generate_chinese_freetext_xfdf(count, page_geoms.len() as u32);
    println!(
        "\n--- Scenario B: {count} Traditional Chinese text boxes (XFDF size = {} bytes) ---",
        xfdf.len()
    );

    let mut parse_ms = Vec::new();
    let mut prep_ms = Vec::new();
    let mut apply_ms = Vec::new();
    let mut undo_ms = Vec::new();
    let mut actual_journal_bytes = 0usize;
    let mut total_subset_raw = 0usize;
    let mut parser_est = 0usize;

    for run in 0..3 {
        let t_parse = Instant::now();
        let mut report = parse_xfdf(&xfdf, page_geoms, usize::MAX).expect("parse cjk");
        parse_ms.push(t_parse.elapsed().as_secs_f64() * 1000.0);

        let t_prep = Instant::now();
        let resolved = resolve_text_boxes(&mut report, usize::MAX, |_| Some(font_data.clone()))
            .expect("resolve cjk");
        prep_ms.push(t_prep.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(resolved.len(), count);
        parser_est = report.estimated_journal_bytes;

        let t_apply = Instant::now();
        let mut doc = PdfDocument::from_bytes(pdf_bytes).expect("open pdf");
        let (applied, _) = apply_imported(&mut doc, &report, &resolved).expect("apply cjk");
        assert_eq!(applied, count);
        apply_ms.push(t_apply.elapsed().as_secs_f64() * 1000.0);

        let mut undo_report = report.clone();
        undo_report.items.pop();
        let undo_resolved = &resolved[..resolved.len() - 1];
        let t_undo = Instant::now();
        let mut reopen = PdfDocument::from_bytes(pdf_bytes).expect("reopen original");
        let (replayed, _) =
            apply_imported(&mut reopen, &undo_report, undo_resolved).expect("replay cjk");
        assert_eq!(replayed, count - 1);
        undo_ms.push(t_undo.elapsed().as_secs_f64() * 1000.0);

        if run == 0 {
            let id_map: HashMap<usize, TextBoxModelId> = resolved
                .iter()
                .map(|r| (r.item_index, r.model_id))
                .collect();
            let edits: Vec<Edit> = report
                .items
                .iter()
                .enumerate()
                .map(|(idx, item)| item.to_edit(id_map.get(&idx).copied()))
                .collect();
            let text_boxes: Vec<(u32, &TextBoxModel)> =
                resolved.iter().map(|r| (r.model_id.0, &r.model)).collect();
            total_subset_raw = resolved.iter().map(|r| r.model.subset.len()).sum();
            actual_journal_bytes = stored_journal_json_len(pdf_bytes.len(), &edits, &text_boxes);
        }
    }

    print_timing_stats("parse", &parse_ms);
    print_timing_stats("prep (subsetting)", &prep_ms);
    print_timing_stats("apply", &apply_ms);
    print_timing_stats("undo replay (reopen + N-1 edits)", &undo_ms);
    println!(
        "  raw_subsets_total={}B hex_in_json={}B parser_est={}B actual_stored_journal_bytes={} ({:.2} MiB) avg_per_box={}B",
        total_subset_raw,
        total_subset_raw * 2,
        parser_est,
        actual_journal_bytes,
        actual_journal_bytes as f64 / (1024.0 * 1024.0),
        actual_journal_bytes / count.max(1)
    );
}

fn scan_journal_4mib_threshold(page_geoms: &[parser::PageGeometry], font_data: &FontData) {
    const JOURNAL_LIMIT: usize = 4 * 1024 * 1024;
    let xfdf = generate_chinese_freetext_xfdf(500, page_geoms.len() as u32);
    let mut report = parse_xfdf(&xfdf, page_geoms, usize::MAX).expect("parse 500");
    let resolved = resolve_text_boxes(&mut report, usize::MAX, |_| Some(font_data.clone()))
        .expect("resolve 500");

    let id_map: HashMap<usize, TextBoxModelId> = resolved
        .iter()
        .map(|r| (r.item_index, r.model_id))
        .collect();
    let edits: Vec<Edit> = report
        .items
        .iter()
        .enumerate()
        .map(|(idx, item)| item.to_edit(id_map.get(&idx).copied()))
        .collect();
    let text_boxes: Vec<(u32, &TextBoxModel)> =
        resolved.iter().map(|r| (r.model_id.0, &r.model)).collect();

    let mut lo = 1usize;
    let mut hi = edits.len();
    let mut best_fit = 0usize;
    let mut best_bytes = 0usize;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let sz = stored_journal_json_len(1024, &edits[..mid], &text_boxes[..mid]);
        if sz <= JOURNAL_LIMIT {
            best_fit = mid;
            best_bytes = sz;
            lo = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }
    let next_bytes = if best_fit < edits.len() {
        stored_journal_json_len(1024, &edits[..best_fit + 1], &text_boxes[..best_fit + 1])
    } else {
        best_bytes
    };
    println!("\n--- Journal 4 MiB Threshold Scan (Traditional Chinese text boxes) ---");
    println!(
        "  max_boxes_under_4MiB={best_fit} ({best_bytes} bytes = {:.3} MiB), box {} => {next_bytes} bytes ({:.3} MiB)",
        best_bytes as f64 / (1024.0 * 1024.0),
        best_fit + 1,
        next_bytes as f64 / (1024.0 * 1024.0)
    );
}

fn stored_journal_json_len(
    pdf_len: usize,
    edits: &[Edit],
    text_boxes: &[(u32, &TextBoxModel)],
) -> usize {
    let tb_json: Vec<serde_json::Value> = text_boxes
        .iter()
        .map(|(id, model)| json!({ "id": id, "model": model }))
        .collect();
    let val = if tb_json.is_empty() {
        json!({
            "version": 1u32,
            "path": "C:\\Users\\Bench\\sample.pdf",
            "len": pdf_len as u64,
            "modifiedSecs": 1_760_000_000u64,
            "modifiedNanos": 0u32,
            "edits": edits
        })
    } else {
        json!({
            "version": 1u32,
            "path": "C:\\Users\\Bench\\sample.pdf",
            "len": pdf_len as u64,
            "modifiedSecs": 1_760_000_000u64,
            "modifiedNanos": 0u32,
            "edits": edits,
            "textBoxes": tb_json
        })
    };
    serde_json::to_vec(&val).expect("serialize journal").len()
}

fn print_timing_stats(label: &str, samples: &[f64]) {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap_or(&0.0);
    println!(
        "  {label}: runs=[{:.2}, {:.2}, {:.2}] ms -> median={median:.2} ms, max={max:.2} ms",
        samples[0], samples[1], samples[2]
    );
}

fn generate_mixed_xfdf(total: usize, page_count: u32) -> Vec<u8> {
    let mut s = String::with_capacity(total * 160);
    s.push_str(r##"<?xml version="1.0" encoding="UTF-8"?><xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>"##);
    for i in 0..total {
        let p = (i as u32) % page_count;
        let x = 40.0 + ((i % 10) as f32) * 45.0;
        let y = 80.0 + (((i / 10) % 14) as f32) * 45.0;
        match i % 7 {
            0 => s.push_str(&format!(
                r##"<highlight page="{p}" rect="{x},{y},{},{}" color="#FFE000" coords="{x},{},{},{},{x},{y},{},{y}"/>"##,
                x + 40.0, y + 14.0, y + 14.0, x + 40.0, y + 14.0, x + 40.0
            )),
            1 => s.push_str(&format!(
                r##"<text page="{p}" rect="{x},{y},{},{}"><contents>Note #{i}: review item</contents></text>"##,
                x + 20.0, y + 20.0
            )),
            2 => s.push_str(&format!(
                r##"<ink page="{p}" rect="{x},{y},{},{}" color="#D91A1A" width="2.5"><inklist><gesture>{x},{y};{},{};{},{}</gesture></inklist></ink>"##,
                x + 35.0, y + 25.0, x + 15.0, y + 20.0, x + 35.0, y + 10.0
            )),
            3 => s.push_str(&format!(
                r##"<stamp page="{p}" rect="{x},{y},{},{}" icon="Approved"/>"##,
                x + 80.0, y + 24.0
            )),
            4 => s.push_str(&format!(
                r##"<square page="{p}" rect="{x},{y},{},{}" color="#1A4DD9" width="2.5"/>"##,
                x + 36.0, y + 24.0
            )),
            5 => s.push_str(&format!(
                r##"<circle page="{p}" rect="{x},{y},{},{}" color="#1A9933" width="1"/>"##,
                x + 30.0, y + 30.0
            )),
            _ => s.push_str(&format!(
                r##"<line page="{p}" rect="{x},{y},{},{}" start="{x},{y}" end="{},{}" head="None" tail="OpenArrow" color="#000000" width="2.5"/>"##,
                x + 35.0, y + 20.0, x + 35.0, y + 20.0
            )),
        }
    }
    s.push_str("</annots></xfdf>");
    s.into_bytes()
}

fn generate_chinese_freetext_xfdf(count: usize, page_count: u32) -> Vec<u8> {
    const BANK: &[char] = &[
        '請', '確', '認', '第', '頁', '合', '約', '條', '款', '簽', '署', '日', '期', '與', '金',
        '額', '修', '訂', '審', '核', '通', '過', '待', '補', '充', '資', '料', '保', '密', '檔',
        '案', '版', '本', '號', '碼', '說', '明', '書', '附', '件', '內', '容', '摘', '要', '重',
        '點', '標', '記', '備', '註', '欄', '位', '測', '試', '正', '體', '中', '文', '字', '型',
        '子', '集', '大', '小', '效', '能', '評', '估', '報', '告', '安', '全', '檢', '查', '異',
        '常', '處', '理', '流', '程', '圖', '表', '數', '據', '統', '計', '結', '果', '建', '議',
    ];
    let mut s = String::with_capacity(count * 320);
    s.push_str(r##"<?xml version="1.0" encoding="UTF-8"?><xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>"##);
    for i in 0..count {
        let p = (i as u32) % page_count;
        let x = 50.0 + ((i % 4) as f32) * 125.0;
        let y = 80.0 + (((i / 4) % 10) as f32) * 60.0;
        let mut snippet = String::with_capacity(48);
        for k in 0..24 {
            snippet.push(BANK[(i * 7 + k * 3) % BANK.len()]);
        }
        s.push_str(&format!(
            r##"<freetext page="{p}" rect="{x},{y},{},{}" justification="0"><defaultstyle>font: 12pt "Microsoft JhengHei"; color: #000000</defaultstyle><contents>第{i}筆：{snippet}</contents></freetext>"##,
            x + 110.0,
            y + 42.0
        ));
    }
    s.push_str("</annots></xfdf>");
    s.into_bytes()
}

fn load_benchmark_cjk_font() -> (String, FontData) {
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        for (name, file, face) in [
            ("Microsoft JhengHei", "msjh.ttc", 0u32),
            ("Microsoft YaHei", "msyh.ttc", 0u32),
            ("MingLiU", "mingliu.ttc", 0u32),
        ] {
            let path = windir.join("Fonts").join(file);
            if let Ok(bytes) = std::fs::read(&path) {
                return (
                    format!("{} ({})", name, path.display()),
                    FontData {
                        bytes,
                        face,
                        name: name.to_string(),
                    },
                );
            }
        }
    }
    let droid = mupdf_fonts_droid::find_by_name("Droid Sans Fallback", false, false)
        .expect("fallback droid");
    (
        "Droid Sans Fallback (bundled fallback)".to_string(),
        FontData {
            bytes: droid.data.to_vec(),
            face: 0,
            name: "Droid Sans Fallback".to_string(),
        },
    )
}

fn build_multi_page_pdf(page_count: u32) -> Vec<u8> {
    let mut objects = Vec::with_capacity(3 + page_count as usize * 2);
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    let kids: String = (0..page_count)
        .map(|i| format!("{} 0 R", 4 + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    for i in 0..page_count {
        let content_obj = 4 + i * 2 + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_obj} 0 R >>"
        ));
        let stream_body = format!("BT /F1 12 Tf 72 740 Td (Benchmark Page {}) Tj ET", i + 1);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream_body}\nendstream",
            stream_body.len()
        ));
    }

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
