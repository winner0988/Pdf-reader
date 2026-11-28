//! #592 的 POC，不合併。
//!
//! 比較保留版面的 Word（`.docx`）匯出做法：
//! - 基準（Baseline）：目前 `pdf_worker::engine::PdfDocument::docx`（純文字區塊與內嵌圖片，不保留版面）
//! - 做法 A：MuPDF 內建 `DocumentWriter`（`docx-output`，Artifex 的 `extract` 函式庫）
//! - 做法 B：擴充目前 `docx.rs` 的純 Rust 版面分析與 OOXML 輸出（無新依賴）
//!
//! 測試會將樣本 PDF 與產出的 `.docx` 寫入暫存目錄的 `poc-592/`（供負責人以 Word 開啟檢視），不提交進版控。

#[allow(dead_code)]
mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use mupdf::{Document, DocumentWriter, Matrix, TextPageFlags, WriteMode};
use pdf_worker::engine::PdfDocument;

fn poc_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("poc-592");
    std::fs::create_dir_all(&dir).expect("create poc-592 dir");
    dir
}

fn corpus(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// --------------------------------------------------------------------------- 樣本 PDF 產生器

fn pdf_escape(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 2);
    out.push(b'(');
    for ch in text.chars() {
        let byte = match ch {
            '\u{2022}' => 0x95, // WinAnsi bullet
            c if (c as u32) <= 0xFF => c as u8,
            _ => b'?',
        };
        if matches!(byte, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out.push(b')');
    out
}

type PdfPageSpec<'a> = (u32, u32, Vec<u8>, Vec<(&'a str, usize)>);

fn build_standard_pdf(pages: &[PdfPageSpec<'_>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    ];

    let mut page_nums = Vec::new();
    for (width, height, stream_bytes, xobjects) in pages {
        let content_num = objects.len() + 1;
        let mut content_obj =
            format!("<< /Length {} >>\nstream\n", stream_bytes.len()).into_bytes();
        content_obj.extend_from_slice(stream_bytes);
        content_obj.extend_from_slice(b"\nendstream");
        objects.push(content_obj);
        let page_num = objects.len() + 1;
        let xobj_dict = if xobjects.is_empty() {
            String::new()
        } else {
            let entries: Vec<String> = xobjects
                .iter()
                .map(|(name, obj)| format!("/{name} {obj} 0 R"))
                .collect();
            format!(" /XObject << {} >>", entries.join(" "))
        };
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R >>{xobj_dict} >> \
                 /Contents {content_num} 0 R >>"
            )
            .into_bytes(),
        );
        page_nums.push(page_num);
    }

    let kids: Vec<String> = page_nums.iter().map(|n| format!("{n} 0 R")).collect();
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        page_nums.len()
    )
    .into_bytes();

    assemble_pdf(&objects)
}

fn assemble_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn push_text(buf: &mut Vec<u8>, font: &str, size: f32, x: f32, y: f32, text: &str) {
    buf.extend_from_slice(format!("BT /{font} {size} Tf {x} {y} Td ").as_bytes());
    buf.extend_from_slice(&pdf_escape(text));
    buf.extend_from_slice(b" Tj ET\n");
}

fn sample_two_column_article() -> Vec<u8> {
    let mut s = Vec::new();
    // 跨欄標題與摘要
    push_text(
        &mut s,
        "F2",
        18.0,
        156.0,
        740.0,
        "Two-Column Layout Evaluation Report",
    );
    push_text(
        &mut s,
        "F3",
        11.0,
        72.0,
        712.0,
        "Abstract: This synthetic article places a full-width header above two independent text columns.",
    );
    s.extend_from_slice(b"0.5 w 0.4 0.4 0.4 RG 72 698 m 540 698 l S 0 0 0 RG\n");

    // 左欄 x=72..288
    push_text(
        &mut s,
        "F2",
        13.0,
        72.0,
        672.0,
        "1. Left Column Introduction",
    );
    let left_p1 = [
        "PDF files store glyphs at explicit page",
        "coordinates without semantic columns.",
        "When exported linearly, left and right",
        "columns often interleave line by line",
        "unless horizontal gaps are segmented.",
    ];
    for (i, line) in left_p1.iter().enumerate() {
        push_text(&mut s, "F1", 10.5, 72.0, 650.0 - (i as f32) * 14.0, line);
    }
    push_text(&mut s, "F2", 11.0, 72.0, 560.0, "1.1 Reading Order");
    let left_p2 = [
        "Each paragraph inside the left column",
        "must finish completely before the first",
        "line of the right column starts in Word.",
        "Continuous section breaks keep edit flow.",
    ];
    for (i, line) in left_p2.iter().enumerate() {
        push_text(&mut s, "F1", 10.5, 72.0, 540.0 - (i as f32) * 14.0, line);
    }

    // 右欄 x=324..540
    push_text(&mut s, "F2", 13.0, 324.0, 672.0, "2. Right Column Analysis");
    let right_p1 = [
        "Word represents multi-column layouts via",
        "sectPr cols elements so editors can type",
        "naturally and reflow across the gutter.",
        "Gutter spacing should mirror the PDF",
        "column separation of thirty-six points.",
    ];
    for (i, line) in right_p1.iter().enumerate() {
        push_text(&mut s, "F1", 10.5, 324.0, 650.0 - (i as f32) * 14.0, line);
    }
    push_text(&mut s, "F2", 11.0, 324.0, 560.0, "2.1 Mixed Width Sections");
    let right_p2 = [
        "A concluding full-width paragraph below",
        "both columns verifies whether mixed",
        "one-column and two-column bands can",
        "coexist cleanly on a single page.",
    ];
    for (i, line) in right_p2.iter().enumerate() {
        push_text(&mut s, "F1", 10.5, 324.0, 540.0 - (i as f32) * 14.0, line);
    }

    // 下方跨欄結語
    s.extend_from_slice(b"0.5 w 0.4 0.4 0.4 RG 72 468 m 540 468 l S 0 0 0 RG\n");
    push_text(
        &mut s,
        "F1",
        10.5,
        72.0,
        446.0,
        "Conclusion: This closing paragraph spans the full page width beneath the two-column region.",
    );

    build_standard_pdf(&[(612, 792, s, vec![])])
}

fn sample_bordered_table() -> Vec<u8> {
    let mut s = Vec::new();
    push_text(&mut s, "F2", 16.0, 72.0, 740.0, "Quarterly Expense Summary");
    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        714.0,
        "The table below has explicit horizontal and vertical grid borders.",
    );

    // 表格座標 (PDF y 從下往上)：x = [72, 240, 350, 430, 540]
    // 列頂底：row0(header): 680..654, row1: 654..616 (兩行), row2: 616..590, row3: 590..564, row4(total merged): 564..538
    let xs = [72.0_f32, 240.0, 350.0, 430.0, 540.0];
    let ys = [680.0_f32, 654.0, 616.0, 590.0, 564.0, 538.0];

    // 表頭淺灰底色
    s.extend_from_slice(b"0.91 0.93 0.96 rg 72 654 468 26 re f 0 0 0 rg\n");
    // 畫外框與橫線
    s.extend_from_slice(b"0.75 w 0.2 0.2 0.2 RG\n");
    for &y in &ys {
        s.extend_from_slice(format!("{} {y} m {} {y} l S\n", xs[0], xs[4]).as_bytes());
    }
    // 垂直線：左右外框全高，中間分隔線到 row3 底部 (564)，最後一列合併前 3 欄
    for (idx, &x) in xs.iter().enumerate() {
        let y_bottom = if idx == 1 || idx == 2 { ys[4] } else { ys[5] };
        s.extend_from_slice(format!("{x} {} m {x} {y_bottom} l S\n", ys[0]).as_bytes());
    }
    s.extend_from_slice(b"0 0 0 RG\n");

    // 表頭文字
    push_text(&mut s, "F2", 10.5, 80.0, 663.0, "Item");
    push_text(&mut s, "F2", 10.5, 248.0, 663.0, "Category");
    push_text(&mut s, "F2", 10.5, 396.0, 663.0, "Qty");
    push_text(&mut s, "F2", 10.5, 488.0, 663.0, "Amount");

    // Row 1 (多行儲存格)
    push_text(&mut s, "F1", 10.0, 80.0, 639.0, "Cloud Compute Nodes");
    push_text(&mut s, "F1", 10.0, 80.0, 625.0, "(Annual Reserve)");
    push_text(&mut s, "F1", 10.0, 248.0, 633.0, "Infrastructure");
    push_text(&mut s, "F1", 10.0, 410.0, 633.0, "4");
    push_text(&mut s, "F1", 10.0, 492.0, 633.0, "$3,200");

    // Row 2
    push_text(&mut s, "F1", 10.0, 80.0, 599.0, "Code Signing Cert");
    push_text(&mut s, "F1", 10.0, 248.0, 599.0, "Security");
    push_text(&mut s, "F1", 10.0, 410.0, 599.0, "1");
    push_text(&mut s, "F1", 10.0, 502.0, 599.0, "$450");

    // Row 3
    push_text(&mut s, "F1", 10.0, 80.0, 573.0, "Hardware Test Rig");
    push_text(&mut s, "F1", 10.0, 248.0, 573.0, "QA Lab");
    push_text(&mut s, "F1", 10.0, 410.0, 573.0, "2");
    push_text(&mut s, "F1", 10.0, 492.0, 573.0, "$1,200");

    // Row 4 (合併前三欄)
    push_text(&mut s, "F2", 10.5, 80.0, 547.0, "Total Approved Budget");
    push_text(&mut s, "F2", 10.5, 492.0, 547.0, "$4,850");

    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        504.0,
        "Note: All figures are denominated in USD and audited offline.",
    );

    build_standard_pdf(&[(612, 792, s, vec![])])
}

fn sample_borderless_table() -> Vec<u8> {
    let mut s = Vec::new();
    push_text(
        &mut s,
        "F2",
        16.0,
        72.0,
        740.0,
        "Server Inventory (No Borders)",
    );
    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        714.0,
        "The grid below relies solely on column alignment without any drawn border lines.",
    );

    let cols = [72.0_f32, 200.0, 330.0, 450.0];
    let rows: [(&str, [&str; 4]); 5] = [
        ("F2", ["Hostname", "Region", "Status", "CPU"]),
        ("F1", ["worker-01", "ap-east-1", "Healthy", "34%"]),
        ("F1", ["worker-02", "ap-east-1", "Healthy", "41%"]),
        ("F1", ["archive-01", "us-west-2", "Standby", "12%"]),
        ("F1", ["verifier-01", "eu-central-1", "Healthy", "28%"]),
    ];
    for (r_idx, (font, cells)) in rows.iter().enumerate() {
        let y = 674.0 - (r_idx as f32) * 22.0;
        for (c_idx, text) in cells.iter().enumerate() {
            push_text(&mut s, font, 10.5, cols[c_idx], y, text);
        }
    }

    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        544.0,
        "End of borderless table sample.",
    );

    build_standard_pdf(&[(612, 792, s, vec![])])
}

fn sample_header_footer_page_numbers() -> Vec<u8> {
    let mut pages = Vec::new();
    let bodies: [(&str, [&str; 2]); 3] = [
        (
            "Section 1: Architecture Overview",
            [
                "This first page introduces the isolated worker model and sets",
                "the repeating running header and footer across every page.",
            ],
        ),
        (
            "Section 2: IPC Serialization",
            [
                "The second page verifies whether identical top and bottom bands",
                "are recognized as Word page headers and footers with page numbers.",
            ],
        ),
        (
            "Section 3: Verification Summary",
            [
                "The final page confirms consistent margins and automatic",
                "page numbering across all three sections of the document.",
            ],
        ),
    ];
    for (idx, (heading, [line1, line2])) in bodies.iter().enumerate() {
        let page_num = idx + 1;
        let mut s = Vec::new();
        // 頁首
        s.extend_from_slice(b"0.4 0.4 0.4 rg\n");
        push_text(
            &mut s,
            "F1",
            9.0,
            72.0,
            758.0,
            "Technical Specification Draft",
        );
        push_text(&mut s, "F1", 9.0, 460.0, 758.0, "Doc ID: SPEC-2026");
        s.extend_from_slice(b"0.5 w 0.6 0.6 0.6 RG 72 748 m 540 748 l S 0 0 0 RG 0 0 0 rg\n");

        // 本文（折行於 x=72..540 版心範圍內，避免超出頁面右界）
        push_text(&mut s, "F2", 15.0, 72.0, 700.0, heading);
        push_text(&mut s, "F1", 11.5, 72.0, 668.0, line1);
        push_text(&mut s, "F1", 11.5, 72.0, 650.0, line2);
        push_text(
            &mut s,
            "F1",
            11.5,
            72.0,
            622.0,
            &format!("Unique body paragraph marker for page {page_num}."),
        );

        // 頁尾
        s.extend_from_slice(b"0.5 w 0.6 0.6 0.6 RG 72 56 m 540 56 l S 0 0 0 RG 0.4 0.4 0.4 rg\n");
        push_text(&mut s, "F1", 9.0, 72.0, 42.0, "Privacy-first PDF Reader");
        push_text(
            &mut s,
            "F1",
            9.0,
            488.0,
            42.0,
            &format!("Page {page_num} of 3"),
        );
        s.extend_from_slice(b"0 0 0 rg\n");

        pages.push((612, 792, s, vec![]));
    }
    build_standard_pdf(&pages)
}

/// 與 `tests/corpus/generate.py` 的 `dc_only_jpeg` 相同原理：無外部依賴產生有效 baseline JPEG。
fn make_test_jpeg(width: u16, height: u16) -> Vec<u8> {
    assert!(width.is_multiple_of(8) && height.is_multiple_of(8));
    let mut out = vec![0xFF, 0xD8];
    let seg = |buf: &mut Vec<u8>, marker: u8, body: &[u8]| {
        buf.push(0xFF);
        buf.push(marker);
        buf.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        buf.extend_from_slice(body);
    };
    let mut dqt = vec![0u8];
    dqt.extend_from_slice(&[1u8; 64]);
    seg(&mut out, 0xDB, &dqt);

    let mut sof0 = Vec::new();
    sof0.push(8);
    sof0.extend_from_slice(&height.to_be_bytes());
    sof0.extend_from_slice(&width.to_be_bytes());
    sof0.push(3);
    for c in [1u8, 2, 3] {
        sof0.extend_from_slice(&[c, 0x11, 0]);
    }
    seg(&mut out, 0xC0, &sof0);

    let mut dc_dht = vec![0x00, 0, 0, 0, 12];
    dc_dht.extend_from_slice(&[0u8; 12]);
    dc_dht.extend(0u8..12);
    seg(&mut out, 0xC4, &dc_dht);

    let mut ac_dht = vec![0x10, 1];
    ac_dht.extend_from_slice(&[0u8; 15]);
    ac_dht.push(0);
    seg(&mut out, 0xC4, &ac_dht);

    seg(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x00, 3, 0x00, 0, 63, 0]);

    let mut bits = String::new();
    let mut prev = [0i32; 3];
    for top in (0..height).step_by(8) {
        for left in (0..width).step_by(8) {
            let x = left + 4;
            let y = top + 4;
            let (r, g, b) = if (24..40).contains(&x) {
                (40.0_f64, 160.0, 70.0)
            } else if y < 32 {
                (200.0, 50.0, 50.0)
            } else {
                (45.0, 70.0, 200.0)
            };
            let vals = [
                0.299 * r + 0.587 * g + 0.114 * b,
                128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b,
                128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b,
            ];
            for (idx, val) in vals.into_iter().enumerate() {
                let level = 8 * (val.round().clamp(0.0, 255.0) as i32 - 128);
                let diff = level - prev[idx];
                prev[idx] = level;
                let size = if diff == 0 {
                    0
                } else {
                    32 - diff.unsigned_abs().leading_zeros() as usize
                };
                bits.push_str(&format!("{size:04b}"));
                if size > 0 {
                    let amp = if diff > 0 {
                        diff
                    } else {
                        diff + (1 << size) - 1
                    };
                    bits.push_str(&format!("{amp:0width$b}", width = size));
                }
                bits.push('0');
            }
        }
    }
    while !bits.len().is_multiple_of(8) {
        bits.push('1');
    }
    for chunk in bits.as_bytes().chunks(8) {
        let s = std::str::from_utf8(chunk).unwrap();
        let byte = u8::from_str_radix(s, 2).unwrap();
        out.push(byte);
        if byte == 0xFF {
            out.push(0x00);
        }
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn sample_image_beside_text() -> Vec<u8> {
    let jpeg = make_test_jpeg(64, 64);
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> /XObject << /Im1 6 0 R >> >> /Contents 7 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    let mut img_obj = format!(
        "<< /Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        jpeg.len()
    )
    .into_bytes();
    img_obj.extend_from_slice(&jpeg);
    img_obj.extend_from_slice(b"\nendstream");
    objects.push(img_obj);

    let mut s = Vec::new();
    push_text(
        &mut s,
        "F2",
        16.0,
        72.0,
        740.0,
        "Figure Beside Paragraph Layout",
    );
    // 左側圖片：144 x 144 pt，x=72..216, y=560..704
    s.extend_from_slice(b"q 144 0 0 144 72 560 cm /Im1 Do Q\n");
    // 右側段落：x=236..540，與圖片並排
    let beside_lines = [
        "This paragraph sits directly to the right of",
        "the 144x144 pt figure. Keeping the image next",
        "to its companion text requires either a",
        "floating anchor (wp:anchor with square wrap)",
        "or a two-column layout band / borderless",
        "table row instead of sequential inline blocks.",
    ];
    for (i, line) in beside_lines.iter().enumerate() {
        push_text(&mut s, "F1", 11.0, 236.0, 690.0 - (i as f32) * 18.0, line);
    }
    // 下方全寬段落
    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        516.0,
        "Below the figure-and-text row, body content resumes across the full width of the page.",
    );

    let mut content_obj = format!("<< /Length {} >>\nstream\n", s.len()).into_bytes();
    content_obj.extend_from_slice(&s);
    content_obj.extend_from_slice(b"\nendstream");
    objects.push(content_obj);

    assemble_pdf(&objects)
}

fn sample_bullet_list() -> Vec<u8> {
    let mut s = Vec::new();
    push_text(
        &mut s,
        "F2",
        16.0,
        72.0,
        740.0,
        "Release Readiness Checklist",
    );
    push_text(
        &mut s,
        "F1",
        11.0,
        72.0,
        712.0,
        "Verify the following items before publishing:",
    );

    // 項目符號清單（掛縮排：符號在 x=88，文字在 x=104）
    let bullets = [
        (
            "\u{2022}",
            "Confirm pdf_worker runs inside AppContainer with zero network access",
            "and no arbitrary filesystem paths.",
            682.0_f32,
        ),
        (
            "\u{2022}",
            "Ensure DOCX packages strip author, creation timestamp, and generator",
            "metadata to uphold privacy guarantees.",
            644.0_f32,
        ),
        (
            "\u{2022}",
            "Validate multi-column and table formatting in Microsoft Word.",
            "",
            606.0_f32,
        ),
    ];
    for (mark, l1, l2, y) in bullets {
        push_text(&mut s, "F1", 11.0, 88.0, y, mark);
        push_text(&mut s, "F1", 11.0, 104.0, y, l1);
        if !l2.is_empty() {
            push_text(&mut s, "F1", 11.0, 104.0, y - 15.0, l2);
        }
    }

    push_text(&mut s, "F2", 12.0, 72.0, 566.0, "Numbered Steps");
    for (idx, (num, text)) in [
        ("1.", "Open the target PDF document in the viewer."),
        (
            "2.",
            "Select pages and choose Word Document (.docx) export.",
        ),
        ("3.", "Inspect the resulting file in Microsoft Word."),
    ]
    .iter()
    .enumerate()
    {
        let y = 542.0 - (idx as f32) * 20.0;
        push_text(&mut s, "F1", 11.0, 88.0, y, num);
        push_text(&mut s, "F1", 11.0, 108.0, y, text);
    }

    // 置中與靠右文字（檢查對齊偵測）
    push_text(&mut s, "F3", 10.5, 228.0, 456.0, "Centered Caption Line");
    push_text(&mut s, "F1", 10.5, 430.0, 430.0, "Dated: 2026-11-27");

    build_standard_pdf(&[(612, 792, s, vec![])])
}

fn sample_zh_vertical() -> Vec<u8> {
    let text_col1 = "直書中文排版測試";
    let text_col2 = "從右到左直行閱讀";
    let title = "中文直書一頁";
    let mut all_chars: Vec<u16> = Vec::new();
    for ch in title
        .chars()
        .chain(text_col1.chars())
        .chain(text_col2.chars())
    {
        let u = ch as u16;
        if !all_chars.contains(&u) {
            all_chars.push(u);
        }
    }
    all_chars.sort_unstable();
    let bfchars: String = all_chars
        .iter()
        .map(|c| format!("<{c:04X}> <{c:04X}>\n"))
        .collect();
    let to_unicode = format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
         {} beginbfchar\n{bfchars}endbfchar\n\
         endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        all_chars.len()
    );

    let utf16_hex = |s: &str| -> String {
        let mut h = String::new();
        for u in s.encode_utf16() {
            let _ = write!(h, "{u:04X}");
        }
        h
    };

    let content = format!(
        "BT /F2 18 Tf 72 730 Td <{}> Tj ET\n\
         BT /F3 18 Tf 480 680 Td <{}> Tj ET\n\
         BT /F3 18 Tf 430 680 Td <{}> Tj ET\n",
        utf16_hex(title),
        utf16_hex(text_col1),
        utf16_hex(text_col2),
    );

    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F2 7 0 R /F3 8 0 R >> >> /Contents 9 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{to_unicode}\nendstream", to_unicode.len()).into_bytes(),
        b"<< /Type /FontDescriptor /FontName /MingLiU /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 93 >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /MingLiU /CIDSystemInfo << /Registry (Adobe) /Ordering (CNS1) /Supplement 0 >> /FontDescriptor 5 0 R /DW2 [880 -1000] >>".to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /MingLiU /Encoding /UniCNS-UCS2-H /DescendantFonts [6 0 R] /ToUnicode 4 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /MingLiU /Encoding /UniCNS-UCS2-V /DescendantFonts [6 0 R] /ToUnicode 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
    ];
    assemble_pdf(&objects)
}

struct SampleSpec {
    slug: &'static str,
    pdf: Vec<u8>,
}

fn all_samples() -> Vec<SampleSpec> {
    vec![
        SampleSpec {
            slug: "01-contract-v1",
            pdf: corpus("benign/contract-v1.pdf"),
        },
        SampleSpec {
            slug: "02-mixed-text-zh-en",
            pdf: corpus("benign/mixed-text-zh-en.pdf"),
        },
        SampleSpec {
            slug: "03-mixed-page-sizes",
            pdf: corpus("benign/mixed-page-sizes.pdf"),
        },
        SampleSpec {
            slug: "04-rotated-page",
            pdf: corpus("benign/rotated-page.pdf"),
        },
        SampleSpec {
            slug: "05-multi-page-10",
            pdf: corpus("benign/multi-page-10.pdf"),
        },
        SampleSpec {
            slug: "06-outline-3-levels",
            pdf: corpus("benign/outline-3-levels.pdf"),
        },
        SampleSpec {
            slug: "07-tagged-form-two-pages",
            pdf: corpus("benign/tagged-form-two-pages.pdf"),
        },
        SampleSpec {
            slug: "08-two-column-article",
            pdf: sample_two_column_article(),
        },
        SampleSpec {
            slug: "09-bordered-table",
            pdf: sample_bordered_table(),
        },
        SampleSpec {
            slug: "10-borderless-table",
            pdf: sample_borderless_table(),
        },
        SampleSpec {
            slug: "11-header-footer-page-numbers",
            pdf: sample_header_footer_page_numbers(),
        },
        SampleSpec {
            slug: "12-image-beside-text",
            pdf: sample_image_beside_text(),
        },
        SampleSpec {
            slug: "13-bullet-list",
            pdf: sample_bullet_list(),
        },
        SampleSpec {
            slug: "14-zh-vertical",
            pdf: sample_zh_vertical(),
        },
    ]
}

fn export_approach_a(pdf: &[u8], out_path: &Path, options: &str) -> Result<(), mupdf::Error> {
    let doc = Document::from_bytes(pdf, "application/pdf")?;
    let pages = doc.page_count()?;
    let out_str = out_path.to_str().expect("utf8 path");
    let mut writer = DocumentWriter::new(out_str, "docx", options)?;
    for idx in 0..pages {
        let page = doc.load_page(idx)?;
        let bounds = page.bounds()?;
        let device = writer.begin_page(bounds)?;
        page.run(&device, &Matrix::IDENTITY)?;
        writer.end_page(device)?;
    }
    drop(writer);
    Ok(())
}

// --------------------------------------------------------------------------- 做法 B：純 Rust 版面分析與 OOXML 匯出

mod approach_b {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;

    use mupdf::drawing::{DrawingItem, DrawingType};
    use mupdf::text_page::TextBlockType;
    use mupdf::{
        Document, ImageFormat, Point, Quad, Rect, TextChar, TextCharFlags, TextPageFlags, WriteMode,
    };

    const XML_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    const DOC_NAMESPACES: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

    const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

    const STYLES_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults>\
<w:rPrDefault><w:rPr>\
<w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Microsoft JhengHei\" w:cs=\"Calibri\"/>\
<w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>\
<w:lang w:val=\"en-US\" w:eastAsia=\"zh-TW\" w:bidi=\"ar-SA\"/>\
</w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"100\" w:line=\"252\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
</w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
</w:styles>";

    const NUMBERING_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:abstractNum w:abstractNumId=\"0\">\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"&#x2022;\"/><w:lvlJc w:val=\"left\"/></w:lvl>\
</w:abstractNum>\
<w:abstractNum w:abstractNumId=\"1\">\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/></w:lvl>\
</w:abstractNum>\
<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>\
<w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num>\
</w:numbering>";

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Family {
        Sans,
        Serif,
        Mono,
        Symbol,
    }

    impl Family {
        fn ascii_name(self) -> &'static str {
            match self {
                Self::Sans => "Calibri",
                Self::Serif => "Times New Roman",
                Self::Mono => "Courier New",
                Self::Symbol => "Symbol",
            }
        }

        fn east_asia_name(self) -> &'static str {
            match self {
                Self::Serif => "PMingLiU",
                Self::Sans | Self::Mono | Self::Symbol => "Microsoft JhengHei",
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct RunStyle {
        family: Family,
        half_points: u32,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        color: u32,
    }

    impl RunStyle {
        fn of(ch: &TextChar, c: char) -> Self {
            let flags = ch.flags();
            let (bold, italic, family) = match ch.font() {
                Some(font) => (
                    font.is_bold(),
                    font.is_italic(),
                    if font.is_monospaced() {
                        Family::Mono
                    } else if font.is_serif() {
                        Family::Serif
                    } else {
                        Family::Sans
                    },
                ),
                None => (false, false, Family::Sans),
            };
            let hp = (ch.size() * 2.0).round();
            let half_points = if hp.is_finite() {
                (hp as i64).clamp(10, 1638) as u32
            } else {
                22
            };
            Self {
                family: if ('\u{F020}'..='\u{F0FF}').contains(&c) {
                    Family::Symbol
                } else {
                    family
                },
                half_points,
                bold: bold || flags.contains(TextCharFlags::BOLD),
                italic,
                underline: flags.contains(TextCharFlags::UNDERLINE),
                strike: flags.contains(TextCharFlags::STRIKEOUT),
                color: ch.argb() & 0x00FF_FFFF,
            }
        }
    }

    #[derive(Clone)]
    struct StyledSpan {
        style: RunStyle,
        text: String,
    }

    #[derive(Clone)]
    struct TextSeg {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        baseline_y: f32,
        size_pt: f32,
        vertical: bool,
        spans: Vec<StyledSpan>,
        plain: String,
    }

    impl TextSeg {
        fn is_cjk_heavy(&self) -> bool {
            let total = self.plain.chars().count().max(1);
            let cjk = self.plain.chars().filter(|&c| is_cjk(c)).count();
            cjk * 2 >= total
        }

        fn trim_prefix_bytes(&mut self, mut drop_bytes: usize) {
            if drop_bytes == 0 {
                return;
            }
            let mut next_spans = Vec::with_capacity(self.spans.len());
            for mut span in self.spans.drain(..) {
                if drop_bytes == 0 {
                    next_spans.push(span);
                } else if drop_bytes >= span.text.len() {
                    drop_bytes -= span.text.len();
                } else {
                    let rest = span.text[drop_bytes..].to_owned();
                    drop_bytes = 0;
                    span.text = rest;
                    next_spans.push(span);
                }
            }
            self.spans = next_spans;
            self.plain = self.spans.iter().map(|s| s.text.as_str()).collect();
        }
    }

    #[derive(Clone)]
    struct ImgItem {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        png_bytes: Vec<u8>,
    }

    #[derive(Clone)]
    struct HSeg {
        y: f32,
        x0: f32,
        x1: f32,
        width_pt: f32,
        color: u32,
    }

    #[derive(Clone)]
    struct VSeg {
        x: f32,
        y0: f32,
        y1: f32,
        width_pt: f32,
        color: u32,
    }

    #[derive(Clone)]
    struct BgRect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        fill_rgb: u32,
    }

    struct RawPage {
        width_pt: f32,
        height_pt: f32,
        segs: Vec<TextSeg>,
        imgs: Vec<ImgItem>,
        hlines: Vec<HSeg>,
        vlines: Vec<VSeg>,
        bg_rects: Vec<BgRect>,
    }

    #[derive(Clone, Copy)]
    struct BorderEdge {
        present: bool,
        eighths: u32,
        color: u32,
    }

    impl BorderEdge {
        const NONE: Self = Self {
            present: false,
            eighths: 0,
            color: 0,
        };
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Align {
        Left,
        Center,
        Right,
        Both,
    }

    impl Align {
        fn as_ooxml(self) -> &'static str {
            match self {
                Self::Left => "left",
                Self::Center => "center",
                Self::Right => "right",
                Self::Both => "both",
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ListKind {
        Bullet,
        Decimal,
    }

    #[derive(Clone)]
    enum TokenPiece {
        Span(StyledSpan),
        Tab,
        Field(&'static str, RunStyle),
    }

    #[derive(Clone)]
    struct ParaDoc {
        align: Option<Align>,
        left_ind_twips: i64,
        first_line_twips: i64,
        hanging_twips: i64,
        before_twips: i64,
        after_twips: i64,
        list: Option<ListKind>,
        border_rule: Option<(bool, u32, u32)>, // (is_top, sz_eighths, color_rgb)
        tab_right_twips: Option<i64>,
        page_break_before: bool,
        col_break_after: bool,
        anchor_img: Option<(usize, f32, f32, f32, bool)>,
        pieces: Vec<TokenPiece>,
    }

    struct TableCell {
        col_span: usize,
        v_merge: Option<bool>, // Some(true) = restart, Some(false) = continue
        width_pt: f32,
        fill_rgb: Option<u32>,
        top: BorderEdge,
        bottom: BorderEdge,
        left: BorderEdge,
        right: BorderEdge,
        paras: Vec<ParaDoc>,
    }

    struct TableDoc {
        bounds: Rect,
        col_widths_pt: Vec<f32>,
        rows: Vec<Vec<TableCell>>,
        borderless: bool,
    }

    enum PageItem {
        Para(ParaDoc),
        InlineImg(usize, f32, f32),
        Table(TableDoc),
        ColSection {
            col1_w_pt: f32,
            col2_w_pt: f32,
            gutter_pt: f32,
            left_items: Vec<PageItem>,
            right_items: Vec<PageItem>,
        },
    }

    struct HeaderFooterSpec {
        left_spans: Vec<StyledSpan>,
        right_spans: Vec<(Vec<StyledSpan>, bool, bool)>, // (spans, has_page_field, has_numpages_field)
        total_pages: usize,
        rule_color: Option<(u32, u32)>,
        margin_width_pt: f32,
    }

    fn is_kept(c: char) -> bool {
        !c.is_control()
    }

    fn is_cjk(c: char) -> bool {
        matches!(
            c as u32,
            0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0x20000..=0x2CEAF
        )
    }

    fn space_between(prev: char, gap: f32, size: f32, next: char) -> bool {
        if prev.is_whitespace() || next.is_whitespace() {
            return false;
        }
        if is_cjk(prev) && is_cjk(next) {
            return gap > size * 0.45;
        }
        gap > size * 0.20
    }

    fn rgb_from_floats(rgb: [f32; 3]) -> u32 {
        let r = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u32;
        let g = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u32;
        let b = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u32;
        (r << 16) | (g << 8) | b
    }

    fn quad_bounds(q: &Quad, ox: f32, oy: f32) -> (f32, f32, f32, f32) {
        let xs = [q.ul.x - ox, q.ur.x - ox, q.ll.x - ox, q.lr.x - ox];
        let ys = [q.ul.y - oy, q.ur.y - oy, q.ll.y - oy, q.lr.y - oy];
        let x0 = xs.into_iter().fold(f32::INFINITY, f32::min);
        let x1 = xs.into_iter().fold(f32::NEG_INFINITY, f32::max);
        let y0 = ys.into_iter().fold(f32::INFINITY, f32::min);
        let y1 = ys.into_iter().fold(f32::NEG_INFINITY, f32::max);
        (x0, y0, x1, y1)
    }

    fn push_axis_segment(
        pts: (Point, Point),
        origin: (f32, f32),
        stroke: (f32, u32),
        out: (&mut Vec<HSeg>, &mut Vec<VSeg>),
    ) {
        let (p0, p1) = pts;
        let (ox, oy) = origin;
        let (w_pt, color) = stroke;
        let (hlines, vlines) = out;
        let (x0, y0) = (p0.x - ox, p0.y - oy);
        let (x1, y1) = (p1.x - ox, p1.y - oy);
        if (y0 - y1).abs() <= 1.2 && (x1 - x0).abs() >= 10.0 {
            hlines.push(HSeg {
                y: 0.5 * (y0 + y1),
                x0: x0.min(x1),
                x1: x0.max(x1),
                width_pt: w_pt,
                color,
            });
        } else if (x0 - x1).abs() <= 1.2 && (y1 - y0).abs() >= 10.0 {
            vlines.push(VSeg {
                x: 0.5 * (x0 + x1),
                y0: y0.min(y1),
                y1: y0.max(y1),
                width_pt: w_pt,
                color,
            });
        }
    }

    struct SegAccum {
        spans: Vec<StyledSpan>,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        baseline_y: f32,
        size_sum: f32,
        glyph_count: usize,
        vertical: bool,
    }

    impl SegAccum {
        fn new(vertical: bool) -> Self {
            Self {
                spans: Vec::new(),
                x0: f32::INFINITY,
                y0: f32::INFINITY,
                x1: f32::NEG_INFINITY,
                y1: f32::NEG_INFINITY,
                baseline_y: 0.0,
                size_sum: 0.0,
                glyph_count: 0,
                vertical,
            }
        }

        fn reset_bounds(&mut self) {
            self.x0 = f32::INFINITY;
            self.y0 = f32::INFINITY;
            self.x1 = f32::NEG_INFINITY;
            self.y1 = f32::NEG_INFINITY;
            self.size_sum = 0.0;
            self.glyph_count = 0;
        }

        fn flush(&mut self, out: &mut Vec<TextSeg>) {
            // 移除尾端空白
            while let Some(last) = self.spans.last_mut() {
                let trimmed_len = last.text.trim_end_matches(' ').len();
                last.text.truncate(trimmed_len);
                if last.text.is_empty() {
                    self.spans.pop();
                } else {
                    break;
                }
            }
            // 移除開頭空白
            while let Some(first) = self.spans.first_mut() {
                let trimmed = first.text.trim_start_matches(' ');
                if trimmed.is_empty() {
                    self.spans.remove(0);
                } else {
                    if trimmed.len() != first.text.len() {
                        first.text = trimmed.to_owned();
                    }
                    break;
                }
            }
            if self.spans.is_empty() {
                return;
            }
            let mut taken = std::mem::take(&mut self.spans);
            // 合併相鄰同風格 span
            let mut merged: Vec<StyledSpan> = Vec::with_capacity(taken.len());
            for s in taken.drain(..) {
                if let Some(prev) = merged.last_mut()
                    && prev.style == s.style
                {
                    prev.text.push_str(&s.text);
                    continue;
                }
                merged.push(s);
            }
            let plain: String = merged.iter().map(|s| s.text.as_str()).collect();
            if plain.trim().is_empty() {
                return;
            }
            let avg_size = if self.glyph_count > 0 {
                self.size_sum / (self.glyph_count as f32)
            } else {
                11.0
            };
            out.push(TextSeg {
                x0: self.x0,
                y0: self.y0,
                x1: self.x1,
                y1: self.y1,
                baseline_y: self.baseline_y,
                size_pt: avg_size,
                vertical: self.vertical,
                spans: merged,
                plain,
            });
        }
    }

    fn looks_like_list_prefix(s: &str) -> bool {
        let t = s.trim();
        matches!(t, "•" | "-" | "*" | "◦" | "▪" | "–")
            || (t.len() <= 4
                && t.ends_with(['.', ')'])
                && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit()))
    }

    fn extract_page_raw(doc: &Document, idx: i32) -> Result<RawPage, mupdf::Error> {
        let page = doc.load_page(idx)?;
        let bounds = page.bounds()?;
        let (ox, oy) = (bounds.x0, bounds.y0);
        let width_pt = if bounds.width().is_finite() && bounds.width() > 1.0 {
            bounds.width()
        } else {
            612.0
        };
        let height_pt = if bounds.height().is_finite() && bounds.height() > 1.0 {
            bounds.height()
        } else {
            792.0
        };

        let flags = TextPageFlags::PRESERVE_IMAGES | TextPageFlags::COLLECT_STYLES;
        let tp = page.to_text_page(flags)?;

        let mut segs = Vec::new();
        let mut imgs = Vec::new();

        for block in tp.blocks() {
            match block.r#type() {
                TextBlockType::Text => {
                    for line in block.lines() {
                        let vertical = line.wmode() == WriteMode::Vertical;
                        let mut acc = SegAccum::new(vertical);
                        let mut prev_char: Option<(char, f32)> = None;

                        for ch in line.chars() {
                            let Some(c) = ch.char() else {
                                continue;
                            };
                            if !is_kept(c) {
                                continue;
                            }
                            let style = RunStyle::of(&ch, c);
                            let (gx0, gy0, gx1, gy1) = quad_bounds(&ch.quad(), ox, oy);
                            let char_size = ch.size().max(4.0);
                            let cur_y = ch.origin().y - oy;

                            if !vertical && let Some((pc, px1)) = prev_char {
                                let gap = gx0 - px1;
                                let split_thresh = (char_size * 1.65).max(14.0);
                                let cur_plain: String =
                                    acc.spans.iter().map(|s| s.text.as_str()).collect();
                                let protect_list = gap < 28.0 && looks_like_list_prefix(&cur_plain);
                                if gap > split_thresh && !protect_list && !acc.spans.is_empty() {
                                    acc.flush(&mut segs);
                                    acc.reset_bounds();
                                } else if space_between(pc, gap, char_size, c)
                                    && let Some(last) = acc.spans.last_mut()
                                {
                                    last.text.push(' ');
                                }
                            }

                            acc.x0 = acc.x0.min(gx0);
                            acc.y0 = acc.y0.min(gy0);
                            acc.x1 = acc.x1.max(gx1);
                            acc.y1 = acc.y1.max(gy1);
                            acc.baseline_y = cur_y;
                            acc.size_sum += ch.size().max(4.0);
                            acc.glyph_count += 1;

                            if let Some(last) = acc.spans.last_mut()
                                && last.style == style
                            {
                                last.text.push(c);
                            } else {
                                let mut s = String::new();
                                s.push(c);
                                acc.spans.push(StyledSpan { style, text: s });
                            }
                            prev_char = Some((c, gx1));
                        }

                        acc.flush(&mut segs);
                    }
                }
                TextBlockType::Image => {
                    let b = block.bounds();
                    let (x0, y0, x1, y1) = (b.x0 - ox, b.y0 - oy, b.x1 - ox, b.y1 - oy);
                    if (x1 - x0) >= 4.0
                        && (y1 - y0) >= 4.0
                        && let Some(img) = block.image()
                        && let Ok(pix) = img.to_pixmap()
                        && let Some(cs) = pix.color_space()
                        && (cs.is_gray() || cs.is_rgb())
                    {
                        let mut png = Vec::new();
                        if pix.write_to(&mut png, ImageFormat::PNG).is_ok() {
                            imgs.push(ImgItem {
                                x0,
                                y0,
                                x1,
                                y1,
                                png_bytes: png,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        let mut hlines = Vec::new();
        let mut vlines = Vec::new();
        let mut bg_rects = Vec::new();

        for d in page.drawings()? {
            let is_stroked = matches!(
                d.drawing_type,
                DrawingType::Stroke | DrawingType::FillStroke
            );
            let is_filled = matches!(d.drawing_type, DrawingType::Fill | DrawingType::FillStroke);
            let stroke_rgb = d.color.map(rgb_from_floats).unwrap_or(0);
            let stroke_w = d.width.unwrap_or(0.75).clamp(0.25, 6.0);

            if is_stroked && d.stroke_opacity.unwrap_or(1.0) > 0.05 {
                for item in &d.items {
                    match item {
                        DrawingItem::Line(p0, p1) => {
                            push_axis_segment(
                                (*p0, *p1),
                                (ox, oy),
                                (stroke_w, stroke_rgb),
                                (&mut hlines, &mut vlines),
                            );
                        }
                        DrawingItem::Rect { rect, .. } => {
                            let (rx0, ry0, rx1, ry1) =
                                (rect.x0 - ox, rect.y0 - oy, rect.x1 - ox, rect.y1 - oy);
                            if (rx1 - rx0) >= 10.0 {
                                hlines.push(HSeg {
                                    y: ry0,
                                    x0: rx0,
                                    x1: rx1,
                                    width_pt: stroke_w,
                                    color: stroke_rgb,
                                });
                                hlines.push(HSeg {
                                    y: ry1,
                                    x0: rx0,
                                    x1: rx1,
                                    width_pt: stroke_w,
                                    color: stroke_rgb,
                                });
                            }
                            if (ry1 - ry0) >= 10.0 {
                                vlines.push(VSeg {
                                    x: rx0,
                                    y0: ry0,
                                    y1: ry1,
                                    width_pt: stroke_w,
                                    color: stroke_rgb,
                                });
                                vlines.push(VSeg {
                                    x: rx1,
                                    y0: ry0,
                                    y1: ry1,
                                    width_pt: stroke_w,
                                    color: stroke_rgb,
                                });
                            }
                        }
                        DrawingItem::Quad(q) => {
                            for (a, b) in [(q.ul, q.ur), (q.ur, q.lr), (q.lr, q.ll), (q.ll, q.ul)] {
                                push_axis_segment(
                                    (a, b),
                                    (ox, oy),
                                    (stroke_w, stroke_rgb),
                                    (&mut hlines, &mut vlines),
                                );
                            }
                        }
                        DrawingItem::Curve(..) => {}
                    }
                }
            }

            if is_filled && d.fill_opacity.unwrap_or(1.0) > 0.05 {
                let fill_rgb = d.fill.map(rgb_from_floats).unwrap_or(0xFF_FFFF);
                if fill_rgb != 0xFF_FFFF {
                    for item in &d.items {
                        if let DrawingItem::Rect { rect, .. } = item {
                            let (rx0, ry0, rx1, ry1) =
                                (rect.x0 - ox, rect.y0 - oy, rect.x1 - ox, rect.y1 - oy);
                            let (rw, rh) = ((rx1 - rx0).abs(), (ry1 - ry0).abs());
                            if rh <= 3.5 && rw >= 12.0 {
                                hlines.push(HSeg {
                                    y: 0.5 * (ry0 + ry1),
                                    x0: rx0.min(rx1),
                                    x1: rx0.max(rx1),
                                    width_pt: rh.max(0.5),
                                    color: fill_rgb,
                                });
                            } else if rw <= 3.5 && rh >= 12.0 {
                                vlines.push(VSeg {
                                    x: 0.5 * (rx0 + rx1),
                                    y0: ry0.min(ry1),
                                    y1: ry0.max(ry1),
                                    width_pt: rw.max(0.5),
                                    color: fill_rgb,
                                });
                            } else if rw >= 12.0 && rh >= 8.0 {
                                bg_rects.push(BgRect {
                                    x0: rx0.min(rx1),
                                    y0: ry0.min(ry1),
                                    x1: rx0.max(rx1),
                                    y1: ry0.max(ry1),
                                    fill_rgb,
                                });
                            }
                        }
                    }
                }
            }
        }

        stitch_vertical_cjk_columns(&mut segs);

        Ok(RawPage {
            width_pt,
            height_pt,
            segs,
            imgs,
            hlines,
            vlines,
            bg_rects,
        })
    }

    fn stitch_vertical_cjk_columns(segs: &mut Vec<TextSeg>) {
        // MuPDF 在啟用 COLLECT_STYLES 時可能將直排（UniCNS-UCS2-V）拆成單字水平段；
        // 將同 X 欄垂直堆疊的 >=3 個單/雙 CJK 字元還原為直排 TextSeg，避免誤判成表格或碎段。
        let cand: Vec<usize> = segs
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                let n = s.plain.chars().count();
                (1..=2).contains(&n) && s.plain.chars().all(|c| is_cjk(c) || c.is_whitespace())
            })
            .map(|(i, _)| i)
            .collect();
        if cand.len() < 3 {
            return;
        }

        let mut used = vec![false; segs.len()];
        let mut merged_cols: Vec<TextSeg> = Vec::new();
        let mut sorted_cand = cand;
        sorted_cand.sort_by(|&a, &b| {
            let ca = 0.5 * (segs[a].x0 + segs[a].x1);
            let cb = 0.5 * (segs[b].x0 + segs[b].x1);
            cb.total_cmp(&ca).then(segs[a].y0.total_cmp(&segs[b].y0))
        });

        let mut i = 0usize;
        while i < sorted_cand.len() {
            let base_idx = sorted_cand[i];
            if used[base_idx] {
                i += 1;
                continue;
            }
            let base_cx = 0.5 * (segs[base_idx].x0 + segs[base_idx].x1);
            let tol_x = (segs[base_idx].size_pt * 0.55).max(8.0);
            let mut chain = vec![base_idx];
            let mut j = i + 1;
            while j < sorted_cand.len() {
                let nj = sorted_cand[j];
                let cx = 0.5 * (segs[nj].x0 + segs[nj].x1);
                if (cx - base_cx).abs() > tol_x {
                    break;
                }
                let prev_idx = *chain.last().expect("non-empty chain");
                let dy = segs[nj].y0 - segs[prev_idx].y0;
                if dy > 0.0 && dy <= segs[prev_idx].size_pt * 2.2 {
                    chain.push(nj);
                }
                j += 1;
            }
            if chain.len() >= 3 {
                let mut comb = segs[chain[0]].clone();
                comb.vertical = true;
                for &idx in &chain[1..] {
                    let s = &segs[idx];
                    comb.x0 = comb.x0.min(s.x0);
                    comb.y0 = comb.y0.min(s.y0);
                    comb.x1 = comb.x1.max(s.x1);
                    comb.y1 = comb.y1.max(s.y1);
                    for sp in &s.spans {
                        if let Some(last_sp) = comb.spans.last_mut()
                            && last_sp.style == sp.style
                        {
                            last_sp.text.push_str(&sp.text);
                        } else {
                            comb.spans.push(sp.clone());
                        }
                    }
                    comb.plain.push_str(&s.plain);
                }
                for &idx in &chain {
                    used[idx] = true;
                }
                merged_cols.push(comb);
            }
            i = j.max(i + 1);
        }

        if !merged_cols.is_empty() {
            let mut next = Vec::with_capacity(segs.len());
            for (idx, seg) in segs.drain(..).enumerate() {
                if !used[idx] {
                    next.push(seg);
                }
            }
            next.extend(merged_cols);
            *segs = next;
        }
    }

    fn normalize_mask_digits(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_digits = false;
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !in_digits {
                    out.push('#');
                    in_digits = true;
                }
            } else {
                in_digits = false;
                out.push(c);
            }
        }
        out
    }

    fn find_repeating_zone_segs(
        pages: &[RawPage],
        top_zone: bool,
    ) -> (Option<Vec<usize>>, Vec<Vec<usize>>) {
        // 回傳 (匹配的 seg 索引清單 per page) 與候選 signature 對應
        let n = pages.len();
        let min_pages = (n * 6).div_ceil(10).max(2);
        let mut groups: BTreeMap<(i32, i32, String), Vec<(usize, usize)>> = BTreeMap::new();

        for (p_idx, p) in pages.iter().enumerate() {
            let y_limit = if top_zone {
                p.height_pt * 0.12
            } else {
                p.height_pt * 0.88
            };
            for (s_idx, seg) in p.segs.iter().enumerate() {
                let in_zone = if top_zone {
                    seg.y0 <= y_limit && seg.y1 <= p.height_pt * 0.16
                } else {
                    seg.y1 >= y_limit && seg.y0 >= p.height_pt * 0.84
                };
                if in_zone && seg.size_pt <= 12.0 {
                    let y_bucket = (seg.baseline_y / 6.0).round() as i32;
                    let x_bucket = (seg.x0 / 24.0).round() as i32;
                    let key = (y_bucket, x_bucket, normalize_mask_digits(&seg.plain));
                    groups.entry(key).or_default().push((p_idx, s_idx));
                }
            }
        }

        let mut remove_per_page: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut selected_any = false;
        for hits in groups.into_values() {
            let distinct_pages: BTreeSet<usize> = hits.iter().map(|&(p, _)| p).collect();
            if distinct_pages.len() >= min_pages {
                for (p_idx, s_idx) in hits {
                    remove_per_page[p_idx].push(s_idx);
                    selected_any = true;
                }
            }
        }
        // 安全防呆：不可因抽掉頁首/頁尾而讓任何頁面本文歸零
        for (p_idx, p) in pages.iter().enumerate() {
            let removed: BTreeSet<usize> = remove_per_page[p_idx].iter().copied().collect();
            let remain = p.segs.len().saturating_sub(removed.len()) + p.imgs.len();
            if remain == 0 && !removed.is_empty() {
                return (None, vec![Vec::new(); n]);
            }
        }

        if selected_any {
            let counts: Vec<usize> = remove_per_page.iter().map(Vec::len).collect();
            (Some(counts), remove_per_page)
        } else {
            (None, remove_per_page)
        }
    }

    fn build_hf_pieces(
        seg: &TextSeg,
        p_idx: usize,
        total_pages: usize,
    ) -> (Vec<StyledSpan>, bool, bool) {
        let p_str = (p_idx + 1).to_string();
        let tot_str = total_pages.to_string();
        if total_pages >= 2
            && (seg.plain.contains("Page ") || seg.plain.contains("page "))
            && seg.plain.contains(&p_str)
            && seg.plain.contains(&tot_str)
        {
            return (seg.spans.clone(), true, true);
        }
        (seg.spans.clone(), false, false)
    }

    fn extract_headers_footers(
        pages: &mut [RawPage],
    ) -> (Option<HeaderFooterSpec>, Option<HeaderFooterSpec>) {
        if pages.len() < 2 {
            return (None, None);
        }
        let n = pages.len();
        let margin_w = (pages[0].width_pt - 2.0 * infer_page_margins(&pages[0]).0).max(200.0);

        let extract_zone = |pages: &mut [RawPage], top: bool| -> Option<HeaderFooterSpec> {
            let (ok, mut rem_idxs) = find_repeating_zone_segs(pages, top);
            ok?;
            // 以第 1 個出現的頁面作為代表（並排序由左到右）
            let mut chosen: Vec<&TextSeg> = Vec::new();
            let p0_idxs = &rem_idxs[0];
            if p0_idxs.is_empty() {
                return None;
            }
            for &si in p0_idxs {
                chosen.push(&pages[0].segs[si]);
            }
            chosen.sort_by(|a, b| a.x0.total_cmp(&b.x0));

            let left_spans = chosen[0].spans.clone();
            let right_spans = chosen[1..]
                .iter()
                .map(|s| build_hf_pieces(s, 0, n))
                .collect();

            let avg_y = chosen.iter().map(|s| s.baseline_y).sum::<f32>() / (chosen.len() as f32);
            let rule_color = pages[0]
                .hlines
                .iter()
                .find(|h| (h.y - avg_y).abs() <= 18.0 && (h.x1 - h.x0) >= pages[0].width_pt * 0.4)
                .map(|h| (((h.width_pt * 8.0).round() as u32).clamp(2, 24), h.color));

            // 從各頁 body 移除已提升的頁首/頁尾 seg 與其緊鄰分隔線
            for (p_idx, p) in pages.iter_mut().enumerate() {
                let mut idxs = std::mem::take(&mut rem_idxs[p_idx]);
                idxs.sort_unstable();
                idxs.dedup();
                for &ri in idxs.iter().rev() {
                    p.segs.remove(ri);
                }
                if rule_color.is_some() {
                    p.hlines.retain(|h| (h.y - avg_y).abs() > 18.0);
                }
            }

            Some(HeaderFooterSpec {
                left_spans,
                right_spans,
                total_pages: n,
                rule_color,
                margin_width_pt: margin_w,
            })
        };

        let hdr = extract_zone(pages, true);
        let ftr = extract_zone(pages, false);
        (hdr, ftr)
    }

    fn cluster_coords(vals: &[f32], tol: f32) -> Vec<f32> {
        let mut sorted = vals.to_vec();
        sorted.sort_by(f32::total_cmp);
        let mut out: Vec<f32> = Vec::new();
        for v in sorted {
            if let Some(&last) = out.last()
                && (v - last).abs() <= tol
            {
                let n = out.len();
                out[n - 1] = 0.5 * (last + v);
            } else {
                out.push(v);
            }
        }
        out
    }

    fn nearest_idx(coords: &[f32], val: f32, tol: f32) -> Option<usize> {
        coords
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - val).abs().total_cmp(&(b.1 - val).abs()))
            .and_then(|(i, &c)| {
                if (c - val).abs() <= tol {
                    Some(i)
                } else {
                    None
                }
            })
    }

    fn has_h_segment(hlines: &[HSeg], y: f32, x0: f32, x1: f32) -> Option<BorderEdge> {
        let target = (x1 - x0).max(1.0);
        let mut covered = 0.0_f32;
        let mut best: Option<&HSeg> = None;
        for h in hlines {
            if (h.y - y).abs() <= 3.0 {
                let ov = (h.x1.min(x1) - h.x0.max(x0)).max(0.0);
                if ov > 0.0 {
                    covered += ov;
                    best = Some(h);
                }
            }
        }
        if covered >= target * 0.65
            && let Some(h) = best
        {
            Some(BorderEdge {
                present: true,
                eighths: ((h.width_pt * 8.0).round() as u32).clamp(2, 48),
                color: h.color,
            })
        } else {
            None
        }
    }

    fn has_v_segment(vlines: &[VSeg], x: f32, y0: f32, y1: f32) -> Option<BorderEdge> {
        let target = (y1 - y0).max(1.0);
        let mut covered = 0.0_f32;
        let mut best: Option<&VSeg> = None;
        for v in vlines {
            if (v.x - x).abs() <= 3.0 {
                let ov = (v.y1.min(y1) - v.y0.max(y0)).max(0.0);
                if ov > 0.0 {
                    covered += ov;
                    best = Some(v);
                }
            }
        }
        if covered >= target * 0.65
            && let Some(v) = best
        {
            Some(BorderEdge {
                present: true,
                eighths: ((v.width_pt * 8.0).round() as u32).clamp(2, 48),
                color: v.color,
            })
        } else {
            None
        }
    }

    fn detect_bordered_tables(page: &mut RawPage) -> Vec<TableDoc> {
        // 先群集所有在 (y, x) 上彼此交叉形成格線的線段組件
        let ys_raw: Vec<f32> = page.hlines.iter().map(|h| h.y).collect();
        let xs_raw: Vec<f32> = page.vlines.iter().map(|v| v.x).collect();
        let row_ys = cluster_coords(&ys_raw, 2.5);
        let col_xs = cluster_coords(&xs_raw, 2.5);

        if row_ys.len() < 3 || col_xs.len() < 3 {
            return Vec::new();
        }

        // 只取同時參與交叉點 (>=2 H 線且 >=2 V 線) 的格線區段
        let mut active_h = Vec::new();
        for (ri, &y) in row_ys.iter().enumerate() {
            let intersecting_v = page
                .vlines
                .iter()
                .filter(|v| {
                    v.y0 - 4.0 <= y
                        && y <= v.y1 + 4.0
                        && page.hlines.iter().any(|h| {
                            (h.y - y).abs() <= 2.5 && h.x0 - 4.0 <= v.x && v.x <= h.x1 + 4.0
                        })
                })
                .count();
            if intersecting_v >= 2 {
                active_h.push(ri);
            }
        }
        let mut active_v = Vec::new();
        for (ci, &x) in col_xs.iter().enumerate() {
            let intersecting_h = page
                .hlines
                .iter()
                .filter(|h| {
                    h.x0 - 4.0 <= x
                        && x <= h.x1 + 4.0
                        && page.vlines.iter().any(|v| {
                            (v.x - x).abs() <= 2.5 && v.y0 - 4.0 <= h.y && h.y <= v.y1 + 4.0
                        })
                })
                .count();
            if intersecting_h >= 2 {
                active_v.push(ci);
            }
        }
        if active_h.len() < 3 || active_v.len() < 3 {
            return Vec::new();
        }

        let ys: Vec<f32> = active_h.into_iter().map(|i| row_ys[i]).collect();
        let xs: Vec<f32> = active_v.into_iter().map(|i| col_xs[i]).collect();
        let rows_n = ys.len() - 1;
        let cols_n = xs.len() - 1;
        let gx0 = xs[0];
        let gx1 = xs[cols_n];
        let gy0 = ys[0];
        let gy1 = ys[rows_n];

        // 確認此邊界矩陣確實囊括文字
        let has_cells_text = page.segs.iter().any(|s| {
            let cx = 0.5 * (s.x0 + s.x1);
            let cy = 0.5 * (s.y0 + s.y1);
            cx >= gx0 - 2.0 && cx <= gx1 + 2.0 && cy >= gy0 - 2.0 && cy <= gy1 + 2.0
        });
        if !has_cells_text {
            return Vec::new();
        }

        let mut consumed_segs = vec![false; page.segs.len()];
        let mut visited = vec![vec![false; cols_n]; rows_n];
        let mut table_rows = Vec::with_capacity(rows_n);

        for r in 0..rows_n {
            let mut row_cells = Vec::new();
            for c in 0..cols_n {
                if visited[r][c] {
                    continue;
                }
                // 往右合併（無垂直分隔線時）
                let mut c_end = c;
                while c_end + 1 < cols_n
                    && has_v_segment(&page.vlines, xs[c_end + 1], ys[r], ys[r + 1]).is_none()
                {
                    c_end += 1;
                }
                let span = c_end - c + 1;
                // 檢查上方是否為未水平分隔的垂直合併延續
                let merged_above = r > 0
                    && has_h_segment(&page.hlines, ys[r], xs[c], xs[c + span]).is_none()
                    && visited[r - 1][c];
                let merged_below = r + 1 < rows_n
                    && has_h_segment(&page.hlines, ys[r + 1], xs[c], xs[c + span]).is_none();

                visited[r][c..=c_end].fill(true);

                let cx0 = xs[c];
                let cx1 = xs[c + span];
                let cy0 = ys[r];
                let cy1 = ys[r + 1];

                let top = has_h_segment(&page.hlines, cy0, cx0, cx1).unwrap_or(BorderEdge::NONE);
                let bottom = has_h_segment(&page.hlines, cy1, cx0, cx1).unwrap_or(BorderEdge::NONE);
                let left = has_v_segment(&page.vlines, cx0, cy0, cy1).unwrap_or(BorderEdge::NONE);
                let right = has_v_segment(&page.vlines, cx1, cy0, cy1).unwrap_or(BorderEdge::NONE);

                let fill_rgb = page.bg_rects.iter().find_map(|bg| {
                    let ov_w = (bg.x1.min(cx1) - bg.x0.max(cx0)).max(0.0);
                    let ov_h = (bg.y1.min(cy1) - bg.y0.max(cy0)).max(0.0);
                    if ov_w >= (cx1 - cx0) * 0.5 && ov_h >= (cy1 - cy0) * 0.5 {
                        Some(bg.fill_rgb)
                    } else {
                        None
                    }
                });

                let mut cell_segs = Vec::new();
                for (si, seg) in page.segs.iter().enumerate() {
                    if consumed_segs[si] {
                        continue;
                    }
                    let sx = 0.5 * (seg.x0 + seg.x1);
                    let sy = 0.5 * (seg.y0 + seg.y1);
                    if sx >= cx0 - 2.0 && sx <= cx1 + 2.0 && sy >= cy0 - 2.0 && sy <= cy1 + 2.0 {
                        consumed_segs[si] = true;
                        cell_segs.push(seg.clone());
                    }
                }

                let paras =
                    build_flow_paragraphs(&cell_segs, cx0 + 4.0, cx1 - 4.0, page.width_pt, true);
                let v_merge = if merged_above {
                    Some(false)
                } else if merged_below {
                    Some(true)
                } else {
                    None
                };

                row_cells.push(TableCell {
                    col_span: span,
                    v_merge,
                    width_pt: cx1 - cx0,
                    fill_rgb,
                    top,
                    bottom,
                    left,
                    right,
                    paras,
                });
            }
            table_rows.push(row_cells);
        }

        // 移除已消耗文字、背景與格線
        let mut keep_segs = Vec::new();
        for (i, seg) in page.segs.drain(..).enumerate() {
            if !consumed_segs[i] {
                keep_segs.push(seg);
            }
        }
        page.segs = keep_segs;
        page.hlines
            .retain(|h| h.y < gy0 - 3.0 || h.y > gy1 + 3.0 || h.x1 < gx0 - 3.0 || h.x0 > gx1 + 3.0);
        page.vlines
            .retain(|v| v.x < gx0 - 3.0 || v.x > gx1 + 3.0 || v.y1 < gy0 - 3.0 || v.y0 > gy1 + 3.0);
        page.bg_rects
            .retain(|b| b.x1 <= gx0 || b.x0 >= gx1 || b.y1 <= gy0 || b.y0 >= gy1);

        let col_widths_pt: Vec<f32> = (0..cols_n).map(|c| xs[c + 1] - xs[c]).collect();
        vec![TableDoc {
            bounds: Rect {
                x0: gx0,
                y0: gy0,
                x1: gx1,
                y1: gy1,
            },
            col_widths_pt,
            rows: table_rows,
            borderless: false,
        }]
    }

    fn try_detect_borderless_table(segs: &[TextSeg], page_w: f32) -> Option<TableDoc> {
        if segs.len() < 6 {
            return None;
        }
        // 依 baseline Y 分組為視覺橫列
        let mut ordered: Vec<&TextSeg> = segs.iter().collect();
        ordered.sort_by(|a, b| {
            a.baseline_y
                .total_cmp(&b.baseline_y)
                .then(a.x0.total_cmp(&b.x0))
        });
        let mut y_rows: Vec<Vec<&TextSeg>> = Vec::new();
        for seg in ordered {
            if let Some(last_row) = y_rows.last_mut()
                && (last_row[0].baseline_y - seg.baseline_y).abs() <= seg.size_pt * 0.55
            {
                last_row.push(seg);
            } else {
                y_rows.push(vec![seg]);
            }
        }
        if y_rows.len() < 3 {
            return None;
        }
        let multi_cell_rows = y_rows.iter().filter(|r| r.len() >= 2).count();
        if multi_cell_rows < 3 || multi_cell_rows * 2 < y_rows.len() {
            return None;
        }

        // 以所有 seg 的 x0 投影區間偵測對齊欄數
        let mut col_starts = cluster_coords(&segs.iter().map(|s| s.x0).collect::<Vec<_>>(), 16.0);
        col_starts.retain(|&cx| segs.iter().filter(|s| (s.x0 - cx).abs() <= 16.0).count() >= 2);
        if col_starts.len() < 2 {
            return None;
        }

        // 若僅 2 欄，檢查是否為雙欄長文章或直排單字（避免誤判成無框線表格）
        let max_len = segs.iter().map(|s| s.plain.len()).max().unwrap_or(0);
        let avg_len = segs.iter().map(|s| s.plain.len()).sum::<usize>() / segs.len();
        let tiny_cjk_glyphs = segs
            .iter()
            .all(|s| s.plain.chars().count() <= 2 && s.is_cjk_heavy());
        if tiny_cjk_glyphs || (col_starts.len() == 2 && (max_len > 32 || avg_len > 22)) {
            return None;
        }

        let cols_n = col_starts.len();
        let gx0 = segs.iter().map(|s| s.x0).fold(f32::INFINITY, f32::min);
        let gx1 = segs.iter().map(|s| s.x1).fold(f32::NEG_INFINITY, f32::max);
        let gy0 = segs.iter().map(|s| s.y0).fold(f32::INFINITY, f32::min);
        let gy1 = segs.iter().map(|s| s.y1).fold(f32::NEG_INFINITY, f32::max);

        let mut bounds_x = Vec::with_capacity(cols_n + 1);
        bounds_x.push(col_starts[0]);
        bounds_x.extend_from_slice(&col_starts[1..]);
        bounds_x.push(gx1.max(col_starts[cols_n - 1] + 60.0));
        let col_widths_pt: Vec<f32> = (0..cols_n)
            .map(|c| (bounds_x[c + 1] - bounds_x[c]).max(30.0))
            .collect();

        let mut rows_out = Vec::with_capacity(y_rows.len());
        for y_row in y_rows {
            let mut buckets: Vec<Vec<TextSeg>> = vec![Vec::new(); cols_n];
            for seg in y_row {
                if let Some(ci) = nearest_idx(&col_starts, seg.x0, 9999.0) {
                    buckets[ci].push((*seg).clone());
                }
            }
            let cells: Vec<TableCell> = buckets
                .into_iter()
                .enumerate()
                .map(|(ci, csegs)| {
                    let paras =
                        build_flow_paragraphs(&csegs, bounds_x[ci], bounds_x[ci + 1], page_w, true);
                    TableCell {
                        col_span: 1,
                        v_merge: None,
                        width_pt: col_widths_pt[ci],
                        fill_rgb: None,
                        top: BorderEdge::NONE,
                        bottom: BorderEdge::NONE,
                        left: BorderEdge::NONE,
                        right: BorderEdge::NONE,
                        paras,
                    }
                })
                .collect();
            rows_out.push(cells);
        }

        Some(TableDoc {
            bounds: Rect {
                x0: gx0,
                y0: gy0,
                x1: gx1,
                y1: gy1,
            },
            col_widths_pt,
            rows: rows_out,
            borderless: true,
        })
    }

    fn detect_borderless_tables(page: &mut RawPage) -> Vec<TableDoc> {
        if page.segs.len() < 6 {
            return Vec::new();
        }
        let mut indexed: Vec<(usize, &TextSeg)> = page.segs.iter().enumerate().collect();
        indexed.sort_by(|a, b| {
            a.1.baseline_y
                .total_cmp(&b.1.baseline_y)
                .then(a.1.x0.total_cmp(&b.1.x0))
        });

        let mut y_rows: Vec<Vec<(usize, &TextSeg)>> = Vec::new();
        for (idx, seg) in indexed {
            if seg.vertical {
                continue;
            }
            if let Some(last) = y_rows.last_mut()
                && (last[0].1.baseline_y - seg.baseline_y).abs() <= seg.size_pt * 0.55
            {
                last.push((idx, seg));
            } else {
                y_rows.push(vec![(idx, seg)]);
            }
        }

        let overlaps_image = |row: &[(usize, &TextSeg)]| {
            let ry0 = row.iter().map(|(_, s)| s.y0).fold(f32::INFINITY, f32::min);
            let ry1 = row
                .iter()
                .map(|(_, s)| s.y1)
                .fold(f32::NEG_INFINITY, f32::max);
            page.imgs.iter().any(|im| im.y0 < ry1 && ry0 < im.y1)
        };

        let mut consumed = vec![false; page.segs.len()];
        let mut tables = Vec::new();
        let mut r = 0usize;
        while r < y_rows.len() {
            if y_rows[r].len() < 2 || overlaps_image(&y_rows[r]) {
                r += 1;
                continue;
            }
            let start = r;
            let mut end = r + 1;
            while end < y_rows.len()
                && y_rows[end].len() >= 2
                && !overlaps_image(&y_rows[end])
                && (y_rows[end][0].1.baseline_y - y_rows[end - 1][0].1.baseline_y) <= 42.0
            {
                end += 1;
            }
            if end - start >= 3 {
                let run_segs: Vec<TextSeg> = y_rows[start..end]
                    .iter()
                    .flat_map(|row| row.iter().map(|(_, s)| (*s).clone()))
                    .collect();
                if let Some(tbl) = try_detect_borderless_table(&run_segs, page.width_pt) {
                    for row in &y_rows[start..end] {
                        for &(idx, _) in row {
                            consumed[idx] = true;
                        }
                    }
                    tables.push(tbl);
                }
            }
            r = end;
        }

        if !tables.is_empty() {
            let mut keep = Vec::new();
            for (idx, seg) in page.segs.drain(..).enumerate() {
                if !consumed[idx] {
                    keep.push(seg);
                }
            }
            page.segs = keep;
        }
        tables
    }

    fn strip_list_marker(seg: &TextSeg) -> Option<(ListKind, TextSeg)> {
        let trimmed = seg.plain.trim_start();
        for bullet in ["• ", "- ", "* ", "◦ ", "▪ ", "– "] {
            if trimmed.starts_with(bullet) {
                let mut cloned = seg.clone();
                let leading_ws = seg.plain.len() - trimmed.len();
                cloned.trim_prefix_bytes(leading_ws + bullet.len());
                return Some((ListKind::Bullet, cloned));
            }
        }
        // "1. text" 或 "12) text"（排除大號粗體章節標題，例如 "1. Left Column Introduction"）
        let is_bold_heading = seg.size_pt >= 11.8 && seg.spans.iter().all(|s| s.style.bold);
        if !is_bold_heading
            && let Some(dot_pos) = trimmed.find(". ").or_else(|| trimmed.find(") "))
            && (1..=3).contains(&dot_pos)
            && trimmed[..dot_pos].chars().all(|c| c.is_ascii_digit())
        {
            let mut cloned = seg.clone();
            let leading_ws = seg.plain.len() - trimmed.len();
            cloned.trim_prefix_bytes(leading_ws + dot_pos + 2);
            return Some((ListKind::Decimal, cloned));
        }
        None
    }

    fn merge_same_line(segs: &[TextSeg]) -> Vec<TextSeg> {
        let mut sorted = segs.to_vec();
        if sorted.iter().any(|s| s.vertical) {
            // 直書：依 X 由右到左排序直行，同行內由上到下
            sorted.sort_by(|a, b| b.x1.total_cmp(&a.x1).then(a.y0.total_cmp(&b.y0)));
            return sorted;
        }
        sorted.sort_by(|a, b| {
            a.baseline_y
                .total_cmp(&b.baseline_y)
                .then(a.x0.total_cmp(&b.x0))
        });
        let mut out: Vec<TextSeg> = Vec::new();
        for seg in sorted {
            if let Some(prev) = out.last_mut()
                && !prev.vertical
                && (prev.baseline_y - seg.baseline_y).abs() <= seg.size_pt * 0.45
                && seg.x0 >= prev.x1 - 2.0
                && (seg.x0 - prev.x1) <= (seg.size_pt * 2.0).max(16.0)
            {
                if let Some(last_span) = prev.spans.last_mut()
                    && !last_span.text.ends_with(' ')
                {
                    last_span.text.push(' ');
                }
                prev.spans.extend(seg.spans);
                prev.x1 = prev.x1.max(seg.x1);
                prev.y0 = prev.y0.min(seg.y0);
                prev.y1 = prev.y1.max(seg.y1);
                prev.plain = prev.spans.iter().map(|s| s.text.as_str()).collect();
            } else {
                out.push(seg);
            }
        }
        out
    }

    fn build_flow_paragraphs(
        raw_segs: &[TextSeg],
        col_x0: f32,
        col_x1: f32,
        page_w: f32,
        in_table: bool,
    ) -> Vec<ParaDoc> {
        let lines = merge_same_line(raw_segs);
        if lines.is_empty() {
            return vec![ParaDoc {
                align: None,
                left_ind_twips: 0,
                first_line_twips: 0,
                hanging_twips: 0,
                before_twips: 0,
                after_twips: 0,
                list: None,
                border_rule: None,
                tab_right_twips: None,
                page_break_before: false,
                col_break_after: false,
                anchor_img: None,
                pieces: Vec::new(),
            }];
        }

        // 先建立邏輯段落群組
        let mut groups: Vec<Vec<(Option<ListKind>, TextSeg)>> = Vec::new();
        let mut prev_list_item = false;

        for line in lines {
            let list_hit = strip_list_marker(&line);
            let (lkind, clean_line) = match list_hit {
                Some((k, c)) => (Some(k), c),
                None => (None, line),
            };
            let starts_new_para = if let Some(prev_group) = groups.last() {
                let (_, prev_last) = prev_group.last().expect("non-empty group");
                let dy = clean_line.baseline_y - prev_last.baseline_y;
                let size_diff = (clean_line.size_pt - prev_last.size_pt).abs() > 0.9;
                let prev_bold = prev_last.spans.iter().all(|s| s.style.bold);
                let cur_bold = clean_line.spans.iter().all(|s| s.style.bold);
                let style_break = prev_bold != cur_bold && prev_group.len() == 1;
                let indent_jump = !prev_list_item
                    && (clean_line.x0 - prev_group[0].1.x0).abs() > clean_line.size_pt * 0.9;
                clean_line.vertical
                    || prev_last.vertical
                    || lkind.is_some()
                    || dy > clean_line.size_pt * 1.65
                    || dy < -2.0
                    || size_diff
                    || style_break
                    || indent_jump
            } else {
                true
            };
            if starts_new_para {
                groups.push(vec![(lkind, clean_line)]);
            } else if let Some(last_g) = groups.last_mut() {
                last_g.push((None, clean_line));
            }
            prev_list_item = lkind.is_some();
        }

        let col_mid = 0.5 * (col_x0 + col_x1);
        let col_w = (col_x1 - col_x0).max(40.0);
        let printable_right = page_w - 72.0;

        let mut paras = Vec::with_capacity(groups.len());
        let mut prev_bottom_y: Option<f32> = None;

        for g in groups {
            let list_kind = g[0].0;
            let first = &g[0].1;
            let min_x0 = g.iter().map(|(_, s)| s.x0).fold(f32::INFINITY, f32::min);
            let max_x1 = g
                .iter()
                .map(|(_, s)| s.x1)
                .fold(f32::NEG_INFINITY, f32::max);
            let line_mid = 0.5 * (min_x0 + max_x1);

            let align = if g.len() == 1 && !first.vertical {
                let single_w = first.x1 - first.x0;
                if in_table {
                    if (first.x0 - col_x0) > 12.0 && (col_x1 - first.x1) <= 18.0 {
                        Some(Align::Right)
                    } else if (line_mid - col_mid).abs() <= 10.0 && single_w < col_w * 0.72 {
                        Some(Align::Center)
                    } else {
                        Some(Align::Left)
                    }
                } else {
                    let near_right = (printable_right - first.x1).abs() <= 45.0
                        || (col_x1 - first.x1).abs() <= 24.0;
                    if (first.x0 - col_x0) > col_w * 0.45 && near_right && single_w < col_w * 0.55 {
                        Some(Align::Right)
                    } else if (first.x0 - col_x0) > 20.0
                        && (col_x1 - first.x1) > 20.0
                        && (line_mid - col_mid).abs() <= 26.0
                        && single_w < col_w * 0.8
                    {
                        Some(Align::Center)
                    } else {
                        Some(Align::Left)
                    }
                }
            } else if g.len() >= 2 && !first.vertical && !in_table {
                let non_last = &g[..g.len() - 1];
                let nl_min = non_last
                    .iter()
                    .map(|(_, s)| s.x1)
                    .fold(f32::INFINITY, f32::min);
                let nl_max = non_last
                    .iter()
                    .map(|(_, s)| s.x1)
                    .fold(f32::NEG_INFINITY, f32::max);
                if (nl_max - nl_min) <= 5.0 && (col_x1 - nl_max).abs() <= 14.0 {
                    Some(Align::Both)
                } else {
                    Some(Align::Left)
                }
            } else {
                Some(Align::Left)
            };

            let (left_ind_twips, first_line_twips, hanging_twips) = if list_kind.is_some() {
                let left_off = ((first.x0 - col_x0).max(0.0) * 20.0).round() as i64;
                (left_off.max(360), 0, 320)
            } else if matches!(align, Some(Align::Center | Align::Right))
                || in_table
                || first.vertical
            {
                (0, 0, 0)
            } else {
                let left_pt = (min_x0 - col_x0).max(0.0);
                let first_pt = (first.x0 - min_x0).max(0.0);
                let left_tw = if left_pt > 6.0 {
                    (left_pt * 20.0).round() as i64
                } else {
                    0
                };
                let first_tw = if first_pt > 6.0 {
                    (first_pt * 20.0).round() as i64
                } else {
                    0
                };
                (left_tw, first_tw, 0)
            };

            let before_twips = if let Some(py) = prev_bottom_y {
                let gap = (first.y0 - py).clamp(0.0, 24.0);
                if gap > 8.0 {
                    ((gap * 14.0).round() as i64).min(240)
                } else {
                    0
                }
            } else {
                0
            };
            let last_seg = &g.last().expect("non-empty").1;
            prev_bottom_y = Some(last_seg.y1);

            let mut spans: Vec<StyledSpan> = Vec::new();
            for (idx, (_, seg)) in g.iter().enumerate() {
                if idx > 0
                    && !seg.is_cjk_heavy()
                    && let Some(prev_s) = spans.last_mut()
                    && !prev_s.text.ends_with(' ')
                {
                    prev_s.text.push(' ');
                }
                for sp in &seg.spans {
                    if let Some(prev_s) = spans.last_mut()
                        && prev_s.style == sp.style
                    {
                        prev_s.text.push_str(&sp.text);
                    } else {
                        spans.push(sp.clone());
                    }
                }
            }

            let pieces = spans.into_iter().map(TokenPiece::Span).collect();
            paras.push(ParaDoc {
                align,
                left_ind_twips,
                first_line_twips,
                hanging_twips,
                before_twips,
                after_twips: if in_table { 40 } else { 80 },
                list: list_kind,
                border_rule: None,
                tab_right_twips: None,
                page_break_before: false,
                col_break_after: false,
                anchor_img: None,
                pieces,
            });
        }
        paras
    }

    fn infer_page_margins(page: &RawPage) -> (f32, f32, f32, f32) {
        let default = (page.width_pt.min(page.height_pt) * 0.1).clamp(18.0, 72.0);
        if page.segs.is_empty() && page.imgs.is_empty() {
            return (default, default, default, default);
        }
        let min_x = page
            .segs
            .iter()
            .map(|s| s.x0)
            .chain(page.imgs.iter().map(|i| i.x0))
            .fold(f32::INFINITY, f32::min);
        let max_x = page
            .segs
            .iter()
            .map(|s| s.x1)
            .chain(page.imgs.iter().map(|i| i.x1))
            .fold(f32::NEG_INFINITY, f32::max);
        let left = if min_x.is_finite() {
            min_x.clamp(24.0, 80.0)
        } else {
            default
        };
        let right = if max_x.is_finite() {
            (page.width_pt - max_x).clamp(24.0, 80.0)
        } else {
            default
        };
        (
            left,
            right,
            36.0_f32.max(default * 0.65),
            36.0_f32.max(default * 0.65),
        )
    }

    enum RawElement {
        Seg(TextSeg),
        Img(usize, ImgItem),
        Tbl(TableDoc),
    }

    impl RawElement {
        fn y0(&self) -> f32 {
            match self {
                Self::Seg(s) => s.y0,
                Self::Img(_, i) => i.y0,
                Self::Tbl(t) => t.bounds.y0,
            }
        }
        fn y1(&self) -> f32 {
            match self {
                Self::Seg(s) => s.y1,
                Self::Img(_, i) => i.y1,
                Self::Tbl(t) => t.bounds.y1,
            }
        }
        fn x0(&self) -> f32 {
            match self {
                Self::Seg(s) => s.x0,
                Self::Img(_, i) => i.x0,
                Self::Tbl(t) => t.bounds.x0,
            }
        }
        fn x1(&self) -> f32 {
            match self {
                Self::Seg(s) => s.x1,
                Self::Img(_, i) => i.x1,
                Self::Tbl(t) => t.bounds.x1,
            }
        }
    }

    fn find_clean_x_gutter(els: &[RawElement], page_w: f32) -> Option<(f32, f32, f32)> {
        if els.len() < 2 {
            return None;
        }
        let min_x = els.iter().map(RawElement::x0).fold(f32::INFINITY, f32::min);
        let max_x = els
            .iter()
            .map(RawElement::x1)
            .fold(f32::NEG_INFINITY, f32::max);
        if max_x - min_x < page_w * 0.4 {
            return None;
        }
        let mut x_intervals: Vec<(f32, f32)> = els.iter().map(|e| (e.x0(), e.x1())).collect();
        x_intervals.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut merged_x: Vec<(f32, f32)> = Vec::new();
        for (a, b) in x_intervals {
            if let Some(last) = merged_x.last_mut()
                && a <= last.1 + 2.0
            {
                last.1 = last.1.max(b);
            } else {
                merged_x.push((a, b));
            }
        }
        if merged_x.len() != 2 {
            return None;
        }
        let left_end = merged_x[0].1;
        let right_start = merged_x[1].0;
        let gap = right_start - left_end;
        if gap < 16.0 {
            return None;
        }
        Some((0.5 * (left_end + right_start), left_end, right_start))
    }

    fn build_page_items(
        mut page: RawPage,
        images_pool: &mut Vec<Vec<u8>>,
    ) -> (Vec<PageItem>, (f32, f32, f32, f32)) {
        let mut tables = detect_bordered_tables(&mut page);
        tables.extend(detect_borderless_tables(&mut page));
        let (ml, mr, mt, mb) = infer_page_margins(&page);
        let col_x0 = ml;
        let col_x1 = page.width_pt - mr;

        let mut elements: Vec<RawElement> = Vec::new();
        for t in tables {
            elements.push(RawElement::Tbl(t));
        }
        for s in page.segs.drain(..) {
            elements.push(RawElement::Seg(s));
        }
        for img in page.imgs.drain(..) {
            let id = images_pool.len() + 1;
            images_pool.push(img.png_bytes.clone());
            elements.push(RawElement::Img(id, img));
        }
        elements.sort_by(|a, b| a.y0().total_cmp(&b.y0()).then(a.x0().total_cmp(&b.x0())));

        // 沿 Y 軸切分為彼此在垂直方向完全不重疊的橫帶（bands）
        let mut bands: Vec<Vec<RawElement>> = Vec::new();
        let mut cur_band: Vec<RawElement> = Vec::new();
        let mut band_y1 = f32::NEG_INFINITY;

        for el in elements {
            if !cur_band.is_empty() && el.y0() > band_y1 + 4.0 {
                bands.push(cur_band);
                cur_band = Vec::new();
                band_y1 = el.y1();
            } else {
                band_y1 = band_y1.max(el.y1());
            }
            cur_band.push(el);
        }
        if !cur_band.is_empty() {
            bands.push(cur_band);
        }

        enum BandBuild {
            Ready(Vec<PageItem>),
            TwoCol {
                left_segs: Vec<TextSeg>,
                right_segs: Vec<TextSeg>,
                lx0: f32,
                lx1: f32,
                rx0: f32,
                rx1: f32,
            },
        }

        let mut staged: Vec<BandBuild> = Vec::new();

        for band in bands {
            let has_tbl = band.iter().any(|e| matches!(e, RawElement::Tbl(_)));
            if has_tbl {
                let mut segs = Vec::new();
                let mut imgs = Vec::new();
                let mut tbls = Vec::new();
                for e in band {
                    match e {
                        RawElement::Seg(s) => segs.push(s),
                        RawElement::Img(id, im) => imgs.push((id, im)),
                        RawElement::Tbl(t) => tbls.push(t),
                    }
                }
                let mut items = Vec::new();
                for p in build_flow_paragraphs(&segs, col_x0, col_x1, page.width_pt, false)
                    .into_iter()
                    .filter(|p| !p.pieces.is_empty())
                {
                    items.push(PageItem::Para(p));
                }
                for t in tbls {
                    items.push(PageItem::Table(t));
                }
                for (id, im) in imgs {
                    items.push(PageItem::InlineImg(id, im.x1 - im.x0, im.y1 - im.y0));
                }
                staged.push(BandBuild::Ready(items));
                continue;
            }

            // 檢查是否有水平 X 溝槽（圖片並排文字 或 雙欄文章）
            if let Some((split_x, _lx1, _rx0)) = find_clean_x_gutter(&band, page.width_pt) {
                let mut left_segs = Vec::new();
                let mut left_imgs = Vec::new();
                let mut right_segs = Vec::new();
                let mut right_imgs = Vec::new();

                for e in &band {
                    let is_left = 0.5 * (e.x0() + e.x1()) < split_x;
                    match e {
                        RawElement::Seg(s) => {
                            if is_left {
                                left_segs.push(s.clone());
                            } else {
                                right_segs.push(s.clone());
                            }
                        }
                        RawElement::Img(id, im) => {
                            if is_left {
                                left_imgs.push((*id, im.clone()));
                            } else {
                                right_imgs.push((*id, im.clone()));
                            }
                        }
                        RawElement::Tbl(_) => unreachable!(),
                    }
                }

                // 狀況 A：一側圖片、另一側伴隨文字 -> 浮動文繞圖 anchor
                if (left_imgs.len() == 1
                    && left_segs.is_empty()
                    && right_imgs.is_empty()
                    && !right_segs.is_empty())
                    || (right_imgs.len() == 1
                        && right_segs.is_empty()
                        && left_imgs.is_empty()
                        && !left_segs.is_empty())
                {
                    let img_on_left = !left_imgs.is_empty();
                    let (img_id, im) = if img_on_left {
                        left_imgs.remove(0)
                    } else {
                        right_imgs.remove(0)
                    };
                    let companion = if img_on_left { right_segs } else { left_segs };
                    let comp_x0 = if img_on_left {
                        companion
                            .iter()
                            .map(|s| s.x0)
                            .fold(f32::INFINITY, f32::min)
                            .min(col_x1)
                    } else {
                        col_x0
                    };
                    let mut paras =
                        build_flow_paragraphs(&companion, comp_x0, col_x1, page.width_pt, false);
                    if let Some(first_p) = paras.first_mut() {
                        let pos_h = (im.x0 - ml).max(0.0);
                        first_p.anchor_img =
                            Some((img_id, im.x1 - im.x0, im.y1 - im.y0, pos_h, img_on_left));
                    }
                    // 若圖片底部低於文字底部，為最後一段預留間距免與下方全寬文字重疊
                    let text_y0 = companion.iter().map(|s| s.y0).fold(f32::INFINITY, f32::min);
                    let text_y1 = companion
                        .iter()
                        .map(|s| s.y1)
                        .fold(f32::NEG_INFINITY, f32::max);
                    if im.y1 > text_y1
                        && let Some(last_p) = paras.last_mut()
                    {
                        let extra_pt = (im.y1 - text_y1 + (im.y0 - text_y0).abs() * 0.2).max(12.0);
                        last_p.after_twips = ((extra_pt * 20.0).round() as i64).max(120);
                    }
                    staged.push(BandBuild::Ready(
                        paras.into_iter().map(PageItem::Para).collect(),
                    ));
                    continue;
                }

                // 狀況 B：無圖像且兩側各有文字 -> 檢查是否先有無框線表格，否則暫存雙欄候選群以利合併同欄各段
                if left_imgs.is_empty() && right_imgs.is_empty() {
                    let all_segs: Vec<TextSeg> = left_segs
                        .iter()
                        .cloned()
                        .chain(right_segs.iter().cloned())
                        .collect();
                    if let Some(tbl) = try_detect_borderless_table(&all_segs, page.width_pt) {
                        staged.push(BandBuild::Ready(vec![PageItem::Table(tbl)]));
                        continue;
                    }
                    let is_vertical = all_segs.iter().any(|s| s.vertical);
                    if !is_vertical && !left_segs.is_empty() && !right_segs.is_empty() {
                        let lx0 = left_segs.iter().map(|s| s.x0).fold(f32::INFINITY, f32::min);
                        let lx1 = left_segs
                            .iter()
                            .map(|s| s.x1)
                            .fold(f32::NEG_INFINITY, f32::max);
                        let rx0 = right_segs
                            .iter()
                            .map(|s| s.x0)
                            .fold(f32::INFINITY, f32::min);
                        let rx1 = right_segs
                            .iter()
                            .map(|s| s.x1)
                            .fold(f32::NEG_INFINITY, f32::max);
                        staged.push(BandBuild::TwoCol {
                            left_segs,
                            right_segs,
                            lx0,
                            lx1,
                            rx0,
                            rx1,
                        });
                        continue;
                    }
                }
            }

            // 一般橫帶：分開影像與文字；若為無框線多欄表格則轉為無框線 <w:tbl>
            let mut segs = Vec::new();
            let mut imgs = Vec::new();
            for e in band {
                match e {
                    RawElement::Seg(s) => segs.push(s),
                    RawElement::Img(id, im) => imgs.push((id, im)),
                    RawElement::Tbl(_) => {}
                }
            }
            if imgs.is_empty()
                && let Some(tbl) = try_detect_borderless_table(&segs, page.width_pt)
            {
                staged.push(BandBuild::Ready(vec![PageItem::Table(tbl)]));
                continue;
            }
            let mut items = Vec::new();
            if !segs.is_empty() {
                let last_y1 = segs.iter().map(|s| s.y1).fold(f32::NEG_INFINITY, f32::max);
                let mut paras = build_flow_paragraphs(&segs, col_x0, col_x1, page.width_pt, false);
                // 檢查下方是否緊接一條獨立水平分隔線
                if let Some(hl) = page.hlines.iter().find(|h| {
                    h.y >= last_y1 - 2.0
                        && h.y <= last_y1 + 14.0
                        && (h.x1 - h.x0) >= (col_x1 - col_x0) * 0.5
                }) && let Some(last_p) = paras.last_mut()
                {
                    last_p.border_rule = Some((
                        false,
                        ((hl.width_pt * 8.0).round() as u32).clamp(2, 24),
                        hl.color,
                    ));
                }
                for p in paras {
                    items.push(PageItem::Para(p));
                }
            }
            for (id, im) in imgs {
                let mut w = im.x1 - im.x0;
                let mut h = im.y1 - im.y0;
                let limit = (page.width_pt - ml - mr).max(36.0);
                if w > limit {
                    h *= limit / w;
                    w = limit;
                }
                items.push(PageItem::InlineImg(id, w, h));
            }
            staged.push(BandBuild::Ready(items));
        }

        // 合併相鄰且右欄起點對齊、左欄不侵犯右欄的雙欄區帶（避免因段落間距切成多段 sectPr）
        let mut coalesced: Vec<BandBuild> = Vec::with_capacity(staged.len());
        for cur in staged {
            match (coalesced.last_mut(), cur) {
                (
                    Some(BandBuild::TwoCol {
                        left_segs: al,
                        right_segs: ar,
                        lx0: alx0,
                        lx1: alx1,
                        rx0: arx0,
                        rx1: arx1,
                    }),
                    BandBuild::TwoCol {
                        left_segs: bl,
                        right_segs: br,
                        lx0: blx0,
                        lx1: blx1,
                        rx0: brx0,
                        rx1: brx1,
                    },
                ) if (*arx0 - brx0).abs() <= 24.0 && alx1.max(blx1) + 12.0 < arx0.min(brx0) => {
                    al.extend(bl);
                    ar.extend(br);
                    *alx0 = alx0.min(blx0);
                    *alx1 = alx1.max(blx1);
                    *arx0 = arx0.min(brx0);
                    *arx1 = arx1.max(brx1);
                }
                (_, cur) => coalesced.push(cur),
            }
        }

        let mut out_items: Vec<PageItem> = Vec::new();
        for stage in coalesced {
            match stage {
                BandBuild::Ready(items) => out_items.extend(items),
                BandBuild::TwoCol {
                    left_segs,
                    right_segs,
                    lx0,
                    lx1,
                    rx0,
                    rx1,
                } => {
                    if left_segs.len() >= 2 && right_segs.len() >= 2 {
                        let gutter_pt = (rx0 - lx1).max(12.0);
                        let body_w = (page.width_pt - ml - mr).max(120.0);
                        let col1_w_pt = ((lx1 - lx0).max(60.0)).min(body_w * 0.6);
                        let col2_w_pt = (body_w - col1_w_pt - gutter_pt).max(60.0);

                        let left_items: Vec<PageItem> = build_flow_paragraphs(
                            &left_segs,
                            lx0,
                            lx1.max(lx0 + 40.0),
                            page.width_pt,
                            false,
                        )
                        .into_iter()
                        .map(PageItem::Para)
                        .collect();
                        let right_items: Vec<PageItem> = build_flow_paragraphs(
                            &right_segs,
                            rx0,
                            rx1.max(rx0 + 40.0),
                            page.width_pt,
                            false,
                        )
                        .into_iter()
                        .map(PageItem::Para)
                        .collect();
                        out_items.push(PageItem::ColSection {
                            col1_w_pt,
                            col2_w_pt,
                            gutter_pt,
                            left_items,
                            right_items,
                        });
                    } else {
                        let merged: Vec<TextSeg> =
                            left_segs.into_iter().chain(right_segs).collect();
                        for p in
                            build_flow_paragraphs(&merged, col_x0, col_x1, page.width_pt, false)
                        {
                            out_items.push(PageItem::Para(p));
                        }
                    }
                }
            }
        }

        (out_items, (ml, mr, mt, mb))
    }

    fn escape_xml(text: &str, out: &mut String) {
        for c in text.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c if c.is_control() => {}
                c => out.push(c),
            }
        }
    }

    fn twips(pt: f32) -> i64 {
        (f64::from(pt) * 20.0).round().clamp(120.0, 31_680.0) as i64
    }

    fn emu(pt: f32) -> i64 {
        (f64::from(pt) * 12_700.0).round().max(1.0) as i64
    }

    fn write_run(span: &StyledSpan, out: &mut String) {
        if span.text.is_empty() {
            return;
        }
        out.push_str("<w:r><w:rPr>");
        let asc = span.style.family.ascii_name();
        let ea = span.style.family.east_asia_name();
        let _ = write!(
            out,
            "<w:rFonts w:ascii=\"{asc}\" w:hAnsi=\"{asc}\" w:eastAsia=\"{ea}\" w:cs=\"{asc}\"/>"
        );
        if span.style.bold {
            out.push_str("<w:b/><w:bCs/>");
        }
        if span.style.italic {
            out.push_str("<w:i/><w:iCs/>");
        }
        if span.style.strike {
            out.push_str("<w:strike/>");
        }
        if span.style.color != 0 {
            let _ = write!(out, "<w:color w:val=\"{:06X}\"/>", span.style.color);
        }
        let _ = write!(
            out,
            "<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>",
            span.style.half_points
        );
        if span.style.underline {
            out.push_str("<w:u w:val=\"single\"/>");
        }
        out.push_str("</w:rPr><w:t xml:space=\"preserve\">");
        escape_xml(&span.text, out);
        out.push_str("</w:t></w:r>");
    }

    fn write_inline_pic(img_num: usize, w_pt: f32, h_pt: f32, rid_base: usize, out: &mut String) {
        let (cx, cy) = (emu(w_pt), emu(h_pt));
        let rid = rid_base + img_num;
        let _ = write!(
            out,
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{img_num}\" name=\"Picture {img_num}\"/>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{img_num}\" name=\"image{img_num}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rId{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        );
    }

    fn write_anchor_pic(
        img_num: usize,
        w_pt: f32,
        h_pt: f32,
        pos_h_pt: f32,
        img_on_left: bool,
        rid_base: usize,
        out: &mut String,
    ) {
        let (cx, cy) = (emu(w_pt), emu(h_pt));
        let pos_h_emu = (f64::from(pos_h_pt) * 12_700.0).round() as i64;
        let wrap_text = if img_on_left { "right" } else { "left" };
        let (dist_l, dist_r) = if img_on_left {
            (0, emu(12.0))
        } else {
            (emu(12.0), 0)
        };
        let rid = rid_base + img_num;
        let _ = write!(
            out,
            "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"{dist_l}\" distR=\"{dist_r}\" \
simplePos=\"0\" relativeHeight=\"251658240\" behindDoc=\"0\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\">\
<wp:simplePos x=\"0\" y=\"0\"/>\
<wp:positionH relativeFrom=\"margin\"><wp:posOffset>{pos_h_emu}</wp:posOffset></wp:positionH>\
<wp:positionV relativeFrom=\"paragraph\"><wp:posOffset>0</wp:posOffset></wp:positionV>\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
<wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
<wp:wrapSquare wrapText=\"{wrap_text}\"/>\
<wp:docPr id=\"{img_num}\" name=\"Picture {img_num}\"/>\
<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{img_num}\" name=\"image{img_num}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rId{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
</a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"
        );
    }

    fn write_para(para: &ParaDoc, sect_pr: Option<&str>, rid_base: usize, out: &mut String) {
        out.push_str("<w:p>");
        let has_ppr = sect_pr.is_some()
            || para.page_break_before
            || para.list.is_some()
            || para.border_rule.is_some()
            || para.tab_right_twips.is_some()
            || para.before_twips > 0
            || para.after_twips != 100
            || para.left_ind_twips > 0
            || para.first_line_twips > 0
            || para.hanging_twips > 0
            || para.align.is_some();

        if has_ppr {
            out.push_str("<w:pPr>");
            if para.page_break_before {
                out.push_str("<w:pageBreakBefore/>");
            }
            if let Some(lk) = para.list {
                let num_id = match lk {
                    ListKind::Bullet => 1,
                    ListKind::Decimal => 2,
                };
                let _ = write!(
                    out,
                    "<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{num_id}\"/></w:numPr>"
                );
            }
            if let Some((is_top, sz, col)) = para.border_rule {
                let side = if is_top { "top" } else { "bottom" };
                let _ = write!(
                    out,
                    "<w:pBdr><w:{side} w:val=\"single\" w:sz=\"{sz}\" w:space=\"4\" w:color=\"{col:06X}\"/></w:pBdr>"
                );
            }
            if let Some(tab_pos) = para.tab_right_twips {
                let _ = write!(
                    out,
                    "<w:tabs><w:tab w:val=\"right\" w:pos=\"{tab_pos}\"/></w:tabs>"
                );
            }
            if para.before_twips > 0 || para.after_twips != 100 {
                let _ = write!(
                    out,
                    "<w:spacing w:before=\"{}\" w:after=\"{}\"/>",
                    para.before_twips, para.after_twips
                );
            }
            if para.left_ind_twips > 0 || para.first_line_twips > 0 || para.hanging_twips > 0 {
                out.push_str("<w:ind");
                if para.left_ind_twips > 0 {
                    let _ = write!(out, " w:left=\"{}\"", para.left_ind_twips);
                }
                if para.hanging_twips > 0 {
                    let _ = write!(out, " w:hanging=\"{}\"", para.hanging_twips);
                } else if para.first_line_twips > 0 {
                    let _ = write!(out, " w:firstLine=\"{}\"", para.first_line_twips);
                }
                out.push_str("/>");
            }
            if let Some(a) = para.align {
                let _ = write!(out, "<w:jc w:val=\"{}\"/>", a.as_ooxml());
            }
            if let Some(sp) = sect_pr {
                out.push_str(sp);
            }
            out.push_str("</w:pPr>");
        }

        if let Some((img_id, w, h, pos_h, on_left)) = para.anchor_img {
            write_anchor_pic(img_id, w, h, pos_h, on_left, rid_base, out);
        }
        for piece in &para.pieces {
            match piece {
                TokenPiece::Span(sp) => write_run(sp, out),
                TokenPiece::Tab => out.push_str("<w:r><w:tab/></w:r>"),
                TokenPiece::Field(instr, style) => {
                    let asc = style.family.ascii_name();
                    let _ = write!(
                        out,
                        "<w:fldSimple w:instr=\" {instr} \"><w:r><w:rPr>\
<w:rFonts w:ascii=\"{asc}\" w:hAnsi=\"{asc}\"/><w:sz w:val=\"{}\"/>\
</w:rPr><w:t>1</w:t></w:r></w:fldSimple>",
                        style.half_points
                    );
                }
            }
        }
        if para.col_break_after {
            out.push_str("<w:r><w:br w:type=\"column\"/></w:r>");
        }
        out.push_str("</w:p>");
    }

    fn write_edge(tag: &str, edge: BorderEdge, out: &mut String) {
        if edge.present {
            let _ = write!(
                out,
                "<w:{tag} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{:06X}\"/>",
                edge.eighths, edge.color
            );
        } else {
            let _ = write!(out, "<w:{tag} w:val=\"nil\"/>");
        }
    }

    fn write_table(tbl: &TableDoc, rid_base: usize, out: &mut String) {
        let total_w_pt: f32 = tbl.col_widths_pt.iter().sum();
        let total_tw = twips(total_w_pt);
        out.push_str("<w:tbl><w:tblPr>");
        let _ = write!(out, "<w:tblW w:w=\"{total_tw}\" w:type=\"dxa\"/>");
        if tbl.borderless {
            out.push_str(
                "<w:tblBorders>\
<w:top w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
<w:left w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
<w:bottom w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
<w:right w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
<w:insideH w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
<w:insideV w:val=\"none\" w:sz=\"0\" w:space=\"0\" w:color=\"auto\"/>\
</w:tblBorders>",
            );
        }
        out.push_str("<w:tblLayout w:type=\"fixed\"/>");
        out.push_str(
            "<w:tblCellMar><w:top w:w=\"60\" w:type=\"dxa\"/><w:left w:w=\"100\" w:type=\"dxa\"/>\
<w:bottom w:w=\"60\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar>",
        );
        out.push_str("</w:tblPr><w:tblGrid>");
        for &cw in &tbl.col_widths_pt {
            let _ = write!(out, "<w:gridCol w:w=\"{}\"/>", twips(cw));
        }
        out.push_str("</w:tblGrid>");

        for row in &tbl.rows {
            out.push_str("<w:tr>");
            for cell in row {
                out.push_str("<w:tc><w:tcPr>");
                let _ = write!(
                    out,
                    "<w:tcW w:w=\"{}\" w:type=\"dxa\"/>",
                    twips(cell.width_pt)
                );
                if cell.col_span > 1 {
                    let _ = write!(out, "<w:gridSpan w:val=\"{}\"/>", cell.col_span);
                }
                if let Some(restart) = cell.v_merge {
                    if restart {
                        out.push_str("<w:vMerge w:val=\"restart\"/>");
                    } else {
                        out.push_str("<w:vMerge/>");
                    }
                }
                if !tbl.borderless {
                    out.push_str("<w:tcBorders>");
                    write_edge("top", cell.top, out);
                    write_edge("left", cell.left, out);
                    write_edge("bottom", cell.bottom, out);
                    write_edge("right", cell.right, out);
                    out.push_str("</w:tcBorders>");
                }
                if let Some(fill) = cell.fill_rgb {
                    let _ = write!(
                        out,
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill:06X}\"/>"
                    );
                }
                out.push_str("</w:tcPr>");
                for p in &cell.paras {
                    write_para(p, None, rid_base, out);
                }
                out.push_str("</w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
    }

    #[derive(Clone, Copy)]
    struct SectLayout<'a> {
        w_pt: f32,
        h_pt: f32,
        margins: (f32, f32, f32, f32),
        sect_type: Option<&'static str>,
        cols: Option<(f32, f32, f32)>, // (col1, col2, gutter)
        hdr_rid: Option<&'a str>,
        ftr_rid: Option<&'a str>,
    }

    fn render_sect_pr(s: SectLayout<'_>) -> String {
        let mut out = String::from("<w:sectPr>");
        if let Some(r) = s.hdr_rid {
            let _ = write!(out, "<w:headerReference w:type=\"default\" r:id=\"{r}\"/>");
        }
        if let Some(r) = s.ftr_rid {
            let _ = write!(out, "<w:footerReference w:type=\"default\" r:id=\"{r}\"/>");
        }
        if let Some(t) = s.sect_type {
            let _ = write!(out, "<w:type w:val=\"{t}\"/>");
        }
        let orient = if s.w_pt > s.h_pt {
            " w:orient=\"landscape\""
        } else {
            ""
        };
        let _ = write!(
            out,
            "<w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/>",
            twips(s.w_pt),
            twips(s.h_pt)
        );
        let (ml, mr, mt, mb) = s.margins;
        let _ = write!(
            out,
            "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"420\" w:footer=\"420\" w:gutter=\"0\"/>",
            twips(mt),
            twips(mr),
            twips(mb),
            twips(ml)
        );
        if let Some((c1, c2, g)) = s.cols {
            let _ = write!(
                out,
                "<w:cols w:num=\"2\" w:space=\"{}\" w:equalWidth=\"0\"><w:col w:w=\"{}\" w:space=\"{}\"/><w:col w:w=\"{}\"/></w:cols>",
                twips(g),
                twips(c1),
                twips(g),
                twips(c2)
            );
        }
        out.push_str("</w:sectPr>");
        out
    }

    fn render_hf_xml(tag: &str, spec: &HeaderFooterSpec, is_footer: bool) -> String {
        let mut out = String::from(XML_HEADER);
        let _ = write!(out, "<w:{tag} {DOC_NAMESPACES}>");
        let rule = spec.rule_color.map(|(sz, c)| (is_footer, sz, c));
        if let Some((right_spans, has_page, has_tot)) = spec.right_spans.first() {
            let base_style = right_spans.first().map(|s| s.style).unwrap_or(RunStyle {
                family: Family::Sans,
                half_points: 18,
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                color: 0x666666,
            });
            if *has_page && *has_tot {
                let mut para = ParaDoc {
                    align: None,
                    left_ind_twips: 0,
                    first_line_twips: 0,
                    hanging_twips: 0,
                    before_twips: 0,
                    after_twips: 0,
                    list: None,
                    border_rule: rule,
                    tab_right_twips: Some(twips(spec.margin_width_pt)),
                    page_break_before: false,
                    col_break_after: false,
                    anchor_img: None,
                    pieces: spec
                        .left_spans
                        .iter()
                        .cloned()
                        .map(TokenPiece::Span)
                        .collect(),
                };
                para.pieces.push(TokenPiece::Tab);
                let plain: String = right_spans.iter().map(|s| s.text.as_str()).collect();
                let tot_s = spec.total_pages.to_string();
                if let Some(pos_1) = plain.find('1') {
                    let after_1 = pos_1 + 1;
                    let rest = &plain[after_1..];
                    if let Some(pos_t) = rest.find(&tot_s) {
                        let p1 = &plain[..pos_1];
                        let p2 = &rest[..pos_t];
                        let p3 = &rest[pos_t + tot_s.len()..];
                        if !p1.is_empty() {
                            para.pieces.push(TokenPiece::Span(StyledSpan {
                                style: base_style,
                                text: p1.to_owned(),
                            }));
                        }
                        para.pieces.push(TokenPiece::Field("PAGE", base_style));
                        if !p2.is_empty() {
                            para.pieces.push(TokenPiece::Span(StyledSpan {
                                style: base_style,
                                text: p2.to_owned(),
                            }));
                        }
                        para.pieces.push(TokenPiece::Field("NUMPAGES", base_style));
                        if !p3.is_empty() {
                            para.pieces.push(TokenPiece::Span(StyledSpan {
                                style: base_style,
                                text: p3.to_owned(),
                            }));
                        }
                    }
                }
                write_para(&para, None, 10, &mut out);
                let _ = write!(out, "</w:{tag}>");
                return out;
            }
        }
        let mut para = ParaDoc {
            align: None,
            left_ind_twips: 0,
            first_line_twips: 0,
            hanging_twips: 0,
            before_twips: 0,
            after_twips: 0,
            list: None,
            border_rule: rule,
            tab_right_twips: Some(twips(spec.margin_width_pt)),
            page_break_before: false,
            col_break_after: false,
            anchor_img: None,
            pieces: spec
                .left_spans
                .iter()
                .cloned()
                .map(TokenPiece::Span)
                .collect(),
        };
        if let Some((right_spans, _, _)) = spec.right_spans.first() {
            para.pieces.push(TokenPiece::Tab);
            for sp in right_spans {
                para.pieces.push(TokenPiece::Span(sp.clone()));
            }
        }
        write_para(&para, None, 10, &mut out);
        let _ = write!(out, "</w:{tag}>");
        out
    }

    fn write_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut crc_table = [0u32; 256];
        for (i, slot) in crc_table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        let crc32 = |data: &[u8]| -> u32 {
            let mut c = 0xFFFF_FFFF_u32;
            for &b in data {
                c = crc_table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
            }
            !c
        };

        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data) in entries {
            let name_bytes = name.as_bytes();
            let crc = crc32(data);
            let size = data.len() as u32;
            let offset = out.len() as u32;

            // Local file header (1980-01-01 = time 0, date 0x0021, UTF-8 bit 0x0800)
            out.extend_from_slice(&0x0403_4B50_u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0x0800u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0x0021u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(data);

            // Central directory entry
            central.extend_from_slice(&0x0201_4B50_u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0x0800u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0x0021u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name_bytes);
        }
        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        let count = entries.len() as u16;
        out.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    pub fn export(pdf: &[u8]) -> Result<Vec<u8>, mupdf::Error> {
        let doc = Document::from_bytes(pdf, "application/pdf")?;
        let count = doc.page_count()?;
        let mut raw_pages = Vec::with_capacity(count as usize);
        for idx in 0..count {
            raw_pages.push(extract_page_raw(&doc, idx)?);
        }

        let (hdr_spec, ftr_spec) = extract_headers_footers(&mut raw_pages);
        let mut images: Vec<Vec<u8>> = Vec::new();
        let mut page_layouts = Vec::with_capacity(raw_pages.len());
        for rp in raw_pages {
            let (w, h) = (rp.width_pt, rp.height_pt);
            let (items, margins) = build_page_items(rp, &mut images);
            page_layouts.push((w, h, margins, items));
        }

        let mut next_rid = 2usize;
        let hdr_rid = if hdr_spec.is_some() {
            let id = format!("rId{next_rid}");
            next_rid += 1;
            Some(id)
        } else {
            None
        };
        let ftr_rid = if ftr_spec.is_some() {
            let id = format!("rId{next_rid}");
            next_rid += 1;
            Some(id)
        } else {
            None
        };
        // numbering takes rId = next_rid
        let num_rid = format!("rId{next_rid}");
        let rid_base = next_rid; // image N gets rId(rid_base + N)

        let mut body = String::new();
        let total_pages = page_layouts.len();

        for (p_idx, (pw, ph, margins, items)) in page_layouts.into_iter().enumerate() {
            let is_last_page = p_idx + 1 == total_pages;
            let hdr_ref = hdr_rid.as_deref();
            let ftr_ref = ftr_rid.as_deref();

            // 分成區段：連續的單欄項目合併為一個區段，每個 ColSection 自成雙欄區段
            enum SegGroup {
                Flow(Vec<PageItem>),
                Cols(f32, f32, f32, Vec<PageItem>, Vec<PageItem>),
            }
            let mut segs: Vec<SegGroup> = Vec::new();
            for item in items {
                match item {
                    PageItem::ColSection {
                        col1_w_pt,
                        col2_w_pt,
                        gutter_pt,
                        left_items,
                        right_items,
                    } => segs.push(SegGroup::Cols(
                        col1_w_pt,
                        col2_w_pt,
                        gutter_pt,
                        left_items,
                        right_items,
                    )),
                    other => {
                        if let Some(SegGroup::Flow(vec)) = segs.last_mut() {
                            vec.push(other);
                        } else {
                            segs.push(SegGroup::Flow(vec![other]));
                        }
                    }
                }
            }
            if segs.is_empty() {
                segs.push(SegGroup::Flow(Vec::new()));
            }

            let seg_count = segs.len();
            for (s_idx, seg) in segs.into_iter().enumerate() {
                let is_last_seg_on_page = s_idx + 1 == seg_count;
                let ends_doc = is_last_page && is_last_seg_on_page;
                let start_sect_type = if s_idx > 0 {
                    Some("continuous")
                } else if p_idx > 0 {
                    Some("nextPage")
                } else {
                    None
                };

                match seg {
                    SegGroup::Flow(mut flow_items) => {
                        let sect = render_sect_pr(SectLayout {
                            w_pt: pw,
                            h_pt: ph,
                            margins,
                            sect_type: start_sect_type,
                            cols: None,
                            hdr_rid: hdr_ref,
                            ftr_rid: ftr_ref,
                        });
                        if flow_items.is_empty() {
                            body.push_str("<w:p>");
                            if !ends_doc {
                                let _ = write!(body, "<w:pPr>{sect}</w:pPr>");
                            }
                            body.push_str("</w:p>");
                        } else {
                            let last_idx = flow_items.len() - 1;
                            for (i, item) in flow_items.drain(..).enumerate() {
                                let attach_sect =
                                    (!ends_doc && i == last_idx).then_some(sect.as_str());
                                match item {
                                    PageItem::Para(p) => {
                                        write_para(&p, attach_sect, rid_base, &mut body);
                                    }
                                    PageItem::InlineImg(id, w, h) => {
                                        body.push_str("<w:p>");
                                        if let Some(sp) = attach_sect {
                                            let _ = write!(body, "<w:pPr>{sp}</w:pPr>");
                                        }
                                        write_inline_pic(id, w, h, rid_base, &mut body);
                                        body.push_str("</w:p>");
                                    }
                                    PageItem::Table(t) => {
                                        write_table(&t, rid_base, &mut body);
                                        if let Some(sp) = attach_sect {
                                            let _ = write!(body, "<w:p><w:pPr>{sp}</w:pPr></w:p>");
                                        }
                                    }
                                    PageItem::ColSection { .. } => {}
                                }
                            }
                        }
                        if ends_doc {
                            body.push_str(&sect);
                        }
                    }
                    SegGroup::Cols(c1, c2, g, mut left_items, mut right_items) => {
                        let had_left = !left_items.is_empty();
                        if let Some(PageItem::Para(last_l)) = left_items.last_mut() {
                            last_l.col_break_after = true;
                        }
                        for item in left_items {
                            match item {
                                PageItem::Para(p) => write_para(&p, None, rid_base, &mut body),
                                PageItem::Table(t) => write_table(&t, rid_base, &mut body),
                                PageItem::InlineImg(id, w, h) => {
                                    body.push_str("<w:p>");
                                    write_inline_pic(id, w, h, rid_base, &mut body);
                                    body.push_str("</w:p>");
                                }
                                PageItem::ColSection { .. } => {}
                            }
                        }
                        if had_left && !body.ends_with("<w:r><w:br w:type=\"column\"/></w:r></w:p>")
                        {
                            body.push_str("<w:p><w:r><w:br w:type=\"column\"/></w:r></w:p>");
                        }

                        let sect = render_sect_pr(SectLayout {
                            w_pt: pw,
                            h_pt: ph,
                            margins,
                            sect_type: start_sect_type,
                            cols: Some((c1, c2, g)),
                            hdr_rid: hdr_ref,
                            ftr_rid: ftr_ref,
                        });
                        let r_len = right_items.len();
                        for (i, item) in right_items.drain(..).enumerate() {
                            let attach_sect =
                                (!ends_doc && i + 1 == r_len).then_some(sect.as_str());
                            match item {
                                PageItem::Para(p) => {
                                    write_para(&p, attach_sect, rid_base, &mut body);
                                }
                                PageItem::Table(t) => {
                                    write_table(&t, rid_base, &mut body);
                                    if let Some(sp) = attach_sect {
                                        let _ = write!(body, "<w:p><w:pPr>{sp}</w:pPr></w:p>");
                                    }
                                }
                                PageItem::InlineImg(id, w, h) => {
                                    body.push_str("<w:p>");
                                    if let Some(sp) = attach_sect {
                                        let _ = write!(body, "<w:pPr>{sp}</w:pPr>");
                                    }
                                    write_inline_pic(id, w, h, rid_base, &mut body);
                                    body.push_str("</w:p>");
                                }
                                PageItem::ColSection { .. } => {}
                            }
                        }
                        if r_len == 0 && !ends_doc {
                            let _ = write!(body, "<w:p><w:pPr>{sect}</w:pPr></w:p>");
                        }
                        if ends_doc {
                            body.push_str(&sect);
                        }
                    }
                }
            }
        }

        let mut document = String::with_capacity(body.len() + 512);
        document.push_str(XML_HEADER);
        let _ = write!(
            document,
            "<w:document {DOC_NAMESPACES}><w:body>{body}</w:body></w:document>"
        );

        let mut content_types = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>",
        );
        if hdr_spec.is_some() {
            content_types.push_str("<Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>");
        }
        if ftr_spec.is_some() {
            content_types.push_str("<Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>");
        }
        content_types.push_str("</Types>");

        let mut rels = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
        );
        if let Some(ref r) = hdr_rid {
            let _ = write!(
                rels,
                "<Relationship Id=\"{r}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>"
            );
        }
        if let Some(ref r) = ftr_rid {
            let _ = write!(
                rels,
                "<Relationship Id=\"{r}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>"
            );
        }
        let _ = write!(
            rels,
            "<Relationship Id=\"{num_rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>"
        );
        for num in 1..=images.len() {
            let _ = write!(
                rels,
                "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{num}.png\"/>",
                rid_base + num
            );
        }
        rels.push_str("</Relationships>");

        let hdr_xml = hdr_spec.as_ref().map(|s| render_hf_xml("hdr", s, false));
        let ftr_xml = ftr_spec.as_ref().map(|s| render_hf_xml("ftr", s, true));

        let mut entries: Vec<(String, Vec<u8>)> = vec![
            ("[Content_Types].xml".to_owned(), content_types.into_bytes()),
            ("_rels/.rels".to_owned(), ROOT_RELS.as_bytes().to_vec()),
            ("word/_rels/document.xml.rels".to_owned(), rels.into_bytes()),
            ("word/styles.xml".to_owned(), STYLES_XML.as_bytes().to_vec()),
            (
                "word/numbering.xml".to_owned(),
                NUMBERING_XML.as_bytes().to_vec(),
            ),
            ("word/document.xml".to_owned(), document.into_bytes()),
        ];
        if let Some(hx) = hdr_xml {
            entries.push(("word/header1.xml".to_owned(), hx.into_bytes()));
        }
        if let Some(fx) = ftr_xml {
            entries.push(("word/footer1.xml".to_owned(), fx.into_bytes()));
        }
        for (idx, png) in images.into_iter().enumerate() {
            entries.push((format!("word/media/image{}.png", idx + 1), png));
        }

        let refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        Ok(write_stored_zip(&refs))
    }
}

fn read_zip_text_entry(zip_bytes: &[u8], target: &str) -> Option<String> {
    let mut i = 0usize;
    while i + 30 <= zip_bytes.len() && &zip_bytes[i..i + 4] == b"PK\x03\x04" {
        let comp = u16::from_le_bytes([zip_bytes[i + 8], zip_bytes[i + 9]]);
        let csize = u32::from_le_bytes([
            zip_bytes[i + 18],
            zip_bytes[i + 19],
            zip_bytes[i + 20],
            zip_bytes[i + 21],
        ]) as usize;
        let nlen = u16::from_le_bytes([zip_bytes[i + 26], zip_bytes[i + 27]]) as usize;
        let elen = u16::from_le_bytes([zip_bytes[i + 28], zip_bytes[i + 29]]) as usize;
        let data_start = i + 30 + nlen + elen;
        let name = std::str::from_utf8(&zip_bytes[i + 30..i + 30 + nlen]).ok()?;
        let data_end = data_start + csize;
        if name == target {
            assert_eq!(comp, 0, "approach B must use Stored ZIP (method 0)");
            return Some(
                String::from_utf8(zip_bytes[data_start..data_end].to_vec()).expect("utf8 xml"),
            );
        }
        i = data_end;
    }
    None
}

#[test]
fn poc_compare_docx_layout_approaches() {
    let dir = poc_dir();
    let samples = all_samples();

    // 確認直書樣本確實以 Vertical wmode 抽取出文字
    {
        let doc =
            Document::from_bytes(&samples[13].pdf, "application/pdf").expect("open zh-vertical");
        let page = doc.load_page(0).expect("page");
        let tp = page.to_text_page(TextPageFlags::empty()).expect("stext");
        let has_vertical = tp
            .blocks()
            .any(|b| b.lines().any(|l| l.wmode() == WriteMode::Vertical));
        assert!(
            has_vertical,
            "zh-vertical sample must trigger WriteMode::Vertical"
        );
    }

    for sample in &samples {
        let pdf_path = dir.join(format!("{}.pdf", sample.slug));
        std::fs::write(&pdf_path, &sample.pdf).expect("write sample pdf");

        // Baseline (目前 docx.rs)
        let base_path = dir.join(format!("{}.base.docx", sample.slug));
        let t0 = Instant::now();
        let pdf_doc = PdfDocument::from_bytes(&sample.pdf).expect("worker open");
        let page_count = pdf_doc.page_count().expect("count");
        let page_indices: Vec<u32> = (0..page_count).collect();
        let mut base_bytes = Vec::new();
        pdf_doc
            .docx(&page_indices, &mut base_bytes)
            .expect("baseline docx");
        let base_ms = t0.elapsed().as_secs_f64() * 1000.0;
        std::fs::write(&base_path, &base_bytes).expect("write base docx");

        // Approach A: MuPDF DocumentWriter ("docx", "")
        let a_path = dir.join(format!("{}.a.docx", sample.slug));
        let _ = std::fs::remove_file(&a_path);
        let t1 = Instant::now();
        export_approach_a(&sample.pdf, &a_path, "").expect("approach A");
        let a_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let a_bytes = std::fs::read(&a_path).expect("read A docx");
        let reopened_a = Document::from_bytes(
            &a_bytes,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        );

        // Approach B: Pure-Rust layout analysis
        let b_path = dir.join(format!("{}.b.docx", sample.slug));
        let t2 = Instant::now();
        let b_bytes = approach_b::export(&sample.pdf).expect("approach B");
        let b_ms = t2.elapsed().as_secs_f64() * 1000.0;
        std::fs::write(&b_path, &b_bytes).expect("write B docx");

        let reopened_b = Document::from_bytes(
            &b_bytes,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        );
        assert!(
            reopened_b.is_ok(),
            "{}: approach B failed to reopen",
            sample.slug
        );

        let doc_xml =
            read_zip_text_entry(&b_bytes, "word/document.xml").expect("word/document.xml");
        assert!(
            read_zip_text_entry(&b_bytes, "docProps/app.xml").is_none(),
            "approach B must omit docProps"
        );

        let tbl_count = doc_xml.matches("<w:tbl>").count();
        let cols_count = doc_xml.matches("<w:cols ").count();
        let sect_count = doc_xml.matches("<w:sectPr>").count();
        let anchor_count = doc_xml.matches("<wp:anchor ").count();
        let list_count = doc_xml.matches("<w:numPr>").count();
        let has_hdr = read_zip_text_entry(&b_bytes, "word/header1.xml").is_some();
        let has_ftr = read_zip_text_entry(&b_bytes, "word/footer1.xml").is_some();

        println!(
            "SAMPLE {:30} | base {:6.2}ms {:6}B | A {:6.2}ms {:6}B (reopen={:?}) | B {:6.2}ms {:6}B (tbl={}, cols={}, sect={}, anchor={}, numPr={}, hdr={}, ftr={})",
            sample.slug,
            base_ms,
            base_bytes.len(),
            a_ms,
            a_bytes.len(),
            reopened_a.as_ref().map(|d| d.page_count()),
            b_ms,
            b_bytes.len(),
            tbl_count,
            cols_count,
            sect_count,
            anchor_count,
            list_count,
            has_hdr,
            has_ftr,
        );

        // 針對關鍵版面樣本驗證做法 B 的結構不變量
        match sample.slug {
            "03-mixed-page-sizes" => {
                assert_eq!(doc_xml.matches("<w:pgSz ").count(), 4);
                assert!(doc_xml.contains("w:orient=\"landscape\""));
            }
            "04-rotated-page" => {
                assert!(doc_xml.contains("Page 1: not rotated"));
                assert!(doc_xml.contains("Page 2: /Rotate 90"));
                assert!(doc_xml.contains("w:orient=\"landscape\""));
            }
            "05-multi-page-10" => {
                assert!(doc_xml.contains("needle"));
            }
            "08-two-column-article" => {
                assert!(doc_xml.contains("<w:cols w:num=\"2\""));
                assert!(doc_xml.contains("<w:type w:val=\"continuous\"/>"));
                assert!(doc_xml.contains("<w:br w:type=\"column\"/>"));
                let l1 = doc_xml.find("1. Left Column Introduction").expect("left 1");
                let l2 = doc_xml.find("1.1 Reading Order").expect("left 2");
                let r1 = doc_xml.find("2. Right Column Analysis").expect("right 1");
                let r2 = doc_xml.find("2.1 Mixed Width Sections").expect("right 2");
                let conc = doc_xml.find("Conclusion:").expect("conclusion");
                assert!(
                    l1 < l2 && l2 < r1 && r1 < r2 && r2 < conc,
                    "column reading order broken"
                );
            }
            "09-bordered-table" => {
                assert_eq!(tbl_count, 1);
                assert!(doc_xml.contains("<w:gridSpan w:val=\"3\"/>"));
                assert!(doc_xml.contains("<w:shd "));
                assert!(doc_xml.contains("Quarterly Expense Summary"));
            }
            "10-borderless-table" => {
                assert_eq!(tbl_count, 1);
                assert!(doc_xml.contains("worker-01"));
                assert!(doc_xml.contains("ap-east-1"));
            }
            "11-header-footer-page-numbers" => {
                let hdr = read_zip_text_entry(&b_bytes, "word/header1.xml").expect("hdr xml");
                let ftr = read_zip_text_entry(&b_bytes, "word/footer1.xml").expect("ftr xml");
                assert!(hdr.contains("Technical Specification Draft"));
                assert!(hdr.contains("Doc ID: SPEC-2026"));
                assert!(ftr.contains("Privacy-first PDF Reader"));
                assert!(ftr.contains("w:instr=\" PAGE \""));
                assert!(ftr.contains("w:instr=\" NUMPAGES \""));
                assert!(!doc_xml.contains("Technical Specification Draft"));
            }
            "12-image-beside-text" => {
                assert_eq!(anchor_count, 1);
                assert!(doc_xml.contains("<wp:wrapSquare "));
                assert!(doc_xml.contains("This paragraph sits directly to the right"));
                assert!(doc_xml.contains("Below the figure-and-text row"));
            }
            "13-bullet-list" => {
                assert!(list_count >= 6);
                assert!(doc_xml.contains("<w:jc w:val=\"center\"/>"));
                assert!(doc_xml.contains("<w:jc w:val=\"right\"/>"));
            }
            "14-zh-vertical" => {
                let c1_pos = doc_xml
                    .find("直書中文排版測試")
                    .expect("right vertical col (x=480)");
                let c2_pos = doc_xml
                    .find("從右到左直行閱讀")
                    .expect("left vertical col (x=430)");
                assert!(
                    c1_pos < c2_pos,
                    "vertical columns should flow right-to-left (x=480 before x=430)"
                );
                assert!(
                    doc_xml.matches("</w:p>").count() <= 4,
                    "vertical columns should stay coherent rather than splitting per char"
                );
                assert!(doc_xml.contains("Microsoft JhengHei"));
            }
            _ => {}
        }
    }

    {
        let vdoc =
            Document::from_bytes(&samples[13].pdf, "application/pdf").expect("open zh-vertical");
        let vpage = vdoc.load_page(0).expect("page");
        for (label, fl) in [
            ("empty", TextPageFlags::empty()),
            (
                "PRESERVE_IMAGES|COLLECT_STYLES",
                TextPageFlags::PRESERVE_IMAGES | TextPageFlags::COLLECT_STYLES,
            ),
        ] {
            let tp = vpage.to_text_page(fl).expect("stext");
            let mut line_count = 0usize;
            let mut vert_lines = 0usize;
            for b in tp.blocks() {
                for l in b.lines() {
                    line_count += 1;
                    if l.wmode() == WriteMode::Vertical {
                        vert_lines += 1;
                    }
                }
            }
            println!(
                "14-zh-vertical stext ({label}): lines={line_count}, vertical_wmode_lines={vert_lines}"
            );
        }
    }
}
