//! #440 的 POC，不合併。
//!
//! Uses `pdf_worker::engine::PdfDocument` (the same engine code the app uses) to benchmark POC 2:
//! - Generates 16 sources totaling ~128 MiB with 10,000 outline items (including per-source items),
//!   where 4 sources each have 1,000 app text boxes recreated per ADR 0019.
//! - Also generates datasets of 2, 4, 8, 16, 32, 64, and 100 sources at ~128 MiB total so an
//!   external PowerShell monitor can measure wall times and `PeakPagedMemorySize64` /
//!   `PeakWorkingSet64` for each source count.
//!
//! Usage:
//! - `merge_many_poc_bench --generate`         Generate all datasets under `%TEMP%/poc-440/`.
//! - `merge_many_poc_bench --run <N>`          Run one merge pass for `<N>` sources.
//! - `merge_many_poc_bench`                    Generate `N = 16` if missing and run 3 iterations.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ipc_contract::limits::{
    MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS, MAX_PREPARE_TEXT_BOXES, MAX_SOURCE_BYTES,
};
use ipc_contract::types::{Rect, Rgb, TextAlign, TextBoxStyle};
use ipc_contract::worker::{InsertedTextBox, PreparedTextBoxModel, SourceTextBox};
use pdf_worker::engine::{
    DocumentOutline, PdfDocument, SourceOutlineInput, TextBoxFont, build_merged_outline,
    bundled_font, prepare_text_boxes,
};

const TARGET_TOTAL_BYTES: usize = 128 * 1024 * 1024;
const TOTAL_OUTLINE_ITEMS: usize = 10_000;
const TEXT_BOXES_PER_MARKED_SOURCE: usize = 1_000;
const SOURCE_COUNTS: [usize; 7] = [2, 4, 8, 16, 32, 64, 100];

fn poc_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("poc-440");
    fs::create_dir_all(&dir).expect("create poc-440 dir");
    dir
}

fn dataset_dir(n: usize) -> PathBuf {
    poc_dir().join(format!("bench-{n}"))
}

fn pages_per_source(n: usize) -> u32 {
    match n {
        2 => 80,
        4 => 40,
        8 => 20,
        16 => 10,
        32 => 5,
        64 => 3,
        100 => 2,
        _ => 10,
    }
}

fn pdf_string(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    format!("({escaped})")
}

/// Deterministic pseudo-random RGB payload so stream sizes stay realistic through MuPDF rewrites.
fn deterministic_rgb(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut out = vec![0u8; len];
    for chunk in out.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let bytes = state.to_le_bytes();
        let copy = chunk.len();
        chunk.copy_from_slice(&bytes[..copy]);
    }
    out
}

fn build_pdf_bytes(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Builds a PDF with `pages` pages, `outline_items` 3-level entries, and an optional RGB image
/// payload on page 0 of `image_bytes` bytes (rounded up to 3-byte RGB pixels).
fn build_base_source_pdf(
    source_idx: usize,
    pages: u32,
    outline_items: usize,
    image_bytes: usize,
    seed: u64,
) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let reserve = |objs: &mut Vec<Vec<u8>>| -> usize {
        objs.push(Vec::new());
        objs.len()
    };

    let catalog = reserve(&mut objects); // 1
    let pages_root = reserve(&mut objects); // 2
    let font = reserve(&mut objects); // 3
    objects[font - 1] =
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec();

    let image_obj = if image_bytes >= 3 {
        let pixels = image_bytes / 3;
        let width = 1024usize.min(pixels).max(1);
        let height = (pixels / width).max(1);
        let rgb = deterministic_rgb(width * height * 3, seed);
        let num = reserve(&mut objects);
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
            rgb.len()
        )
        .into_bytes();
        body.extend_from_slice(&rgb);
        body.extend_from_slice(b"\nendstream");
        objects[num - 1] = body;
        Some(num)
    } else {
        None
    };

    let mut page_nums = Vec::with_capacity(pages as usize);
    for _ in 0..pages {
        page_nums.push(reserve(&mut objects));
    }

    for p in 0..pages as usize {
        let mut ops = format!(
            "BT /F1 18 Tf 72 720 Td {} Tj ET\n",
            pdf_string(&format!("Source {} - Page {}", source_idx + 1, p + 1))
        );
        let xobject_res = if p == 0
            && let Some(img) = image_obj
        {
            ops.push_str("q 120 0 0 120 72 540 cm /Im1 Do Q\n");
            format!(" /XObject << /Im1 {img} 0 R >>")
        } else {
            String::new()
        };
        let stream_num = reserve(&mut objects);
        objects[stream_num - 1] =
            format!("<< /Length {} >>\nstream\n{ops}\nendstream", ops.len()).into_bytes();
        objects[page_nums[p] - 1] = format!(
            "<< /Type /Page /Parent {pages_root} 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font} 0 R >>{xobject_res} >> /Contents {stream_num} 0 R >>"
        )
        .into_bytes();
    }

    let kids: Vec<String> = page_nums.iter().map(|n| format!("{n} 0 R")).collect();
    objects[pages_root - 1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {pages} >>",
        kids.join(" ")
    )
    .into_bytes();

    let outlines_ref = if outline_items == 0 {
        String::new()
    } else {
        let root = reserve(&mut objects);
        let nums: Vec<usize> = (0..outline_items).map(|_| reserve(&mut objects)).collect();
        let mut entries: Vec<(String, usize, usize)> = Vec::with_capacity(outline_items);
        for i in 0..outline_items {
            let depth = if i % 7 == 0 {
                0
            } else if i % 3 == 0 {
                1
            } else {
                2.min(i)
            };
            // Ensure depth never increases by more than 1 from previous entry.
            let prev_depth = entries.last().map_or(0, |(_, d, _)| *d);
            let depth = if entries.is_empty() {
                0
            } else {
                depth.min(prev_depth + 1)
            };
            let page = i % (pages as usize);
            entries.push((format!("S{}-Item-{i}", source_idx + 1), depth, page));
        }

        let mut parents: Vec<usize> = Vec::new();
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        children.insert(root, Vec::new());
        for (i, &(_, depth, _)) in entries.iter().enumerate() {
            parents.truncate(depth);
            let parent = parents.last().map_or(root, |&p| nums[p]);
            children.entry(parent).or_default().push(i);
            parents.push(i);
        }
        for (&parent, kids_idx) in &children {
            for (k, &i) in kids_idx.iter().enumerate() {
                let (title, _, page) = &entries[i];
                let mut dict = format!("<< /Title {} /Parent {parent} 0 R", pdf_string(title));
                if k > 0 {
                    dict.push_str(&format!(" /Prev {} 0 R", nums[kids_idx[k - 1]]));
                }
                if k + 1 < kids_idx.len() {
                    dict.push_str(&format!(" /Next {} 0 R", nums[kids_idx[k + 1]]));
                }
                if let Some(own) = children.get(&nums[i])
                    && !own.is_empty()
                {
                    dict.push_str(&format!(
                        " /First {} 0 R /Last {} 0 R /Count {}",
                        nums[own[0]],
                        nums[own[own.len() - 1]],
                        own.len()
                    ));
                }
                dict.push_str(&format!(" /Dest [{} 0 R /Fit] >>", page_nums[*page]));
                objects[nums[i] - 1] = dict.into_bytes();
            }
        }
        let top = &children[&root];
        objects[root - 1] = format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            nums[top[0]],
            nums[top[top.len() - 1]],
            top.len()
        )
        .into_bytes();
        format!(" /Outlines {root} 0 R")
    };

    objects[catalog - 1] =
        format!("<< /Type /Catalog /Pages {pages_root} 0 R{outlines_ref} >>").into_bytes();
    build_pdf_bytes(&objects)
}

fn prepare_models(texts: &[String]) -> Vec<ipc_contract::worker::TextBoxModel> {
    let font = TextBoxFont {
        bytes: bundled_font(),
        face: 0,
        name: "Droid Sans Fallback",
    };
    let mut models = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(MAX_PREPARE_TEXT_BOXES) {
        let prepared = prepare_text_boxes(&font, chunk);
        for item in prepared {
            match item {
                PreparedTextBoxModel::Prepared(model) => models.push(model),
                other => panic!("unexpected prepare_text_boxes result: {other:?}"),
            }
        }
    }
    models
}

fn add_1000_text_boxes(base_pdf: &[u8], source_idx: usize) -> Vec<u8> {
    let mut doc = PdfDocument::from_bytes(base_pdf).expect("open base for text boxes");
    let pages = doc.page_count().expect("page count");
    let texts: Vec<String> = (0..TEXT_BOXES_PER_MARKED_SOURCE)
        .map(|i| format!("S{}-Box-{i:04}", source_idx + 1))
        .collect();
    let models = prepare_models(&texts);
    let style = TextBoxStyle {
        size: 10.0,
        color: Rgb {
            r: 20,
            g: 40,
            b: 140,
        },
        align: TextAlign::Left,
    };
    for (i, model) in models.iter().enumerate() {
        let page = (i as u32) % pages;
        let slot = i / (pages as usize);
        let col = (slot % 10) as f32;
        let row = (slot / 10) as f32;
        let x0 = 36.0 + col * 54.0;
        let y0 = 36.0 + row * 14.0;
        let rect = Rect {
            x0,
            y0,
            x1: x0 + 50.0,
            y1: y0 + 12.0,
        };
        doc.place_text_box(page, rect, model, &style)
            .expect("place_text_box");
    }
    let mut saved = Vec::new();
    doc.save(&mut saved).expect("save text box source");
    saved
}

fn generate_dataset(n: usize) {
    let dir = dataset_dir(n);
    if dir.exists() {
        let existing = fs::read_dir(&dir)
            .map(|rd| rd.filter_map(Result::ok).count())
            .unwrap_or(0);
        if existing == n {
            println!("dataset N={n} already exists in {}", dir.display());
            return;
        }
        fs::remove_dir_all(&dir).expect("clean partial dataset");
    }
    fs::create_dir_all(&dir).expect("create dataset dir");

    let pages = pages_per_source(n);
    let source_outline_total = TOTAL_OUTLINE_ITEMS - n;
    let base_outlines = source_outline_total / n;
    let extra_outlines = source_outline_total % n;
    let marked_sources = n.min(4);

    // Measure text-box source baseline first so total across N sources stays ~128 MiB.
    let probe_base = build_base_source_pdf(0, pages, base_outlines, 0, 1);
    let probe_tb = add_1000_text_boxes(&probe_base, 0);
    let tb_overhead = probe_tb.len();
    let total_target = if n == 2 {
        // Keep each source <= MAX_SOURCE_BYTES (64 MiB) even after prepare_source rewrite.
        2 * (63 * 1024 * 1024)
    } else {
        TARGET_TOTAL_BYTES
    };
    let unmarked_count = n - marked_sources;
    let per_source_target = if total_target / n >= tb_overhead {
        total_target / n
    } else {
        (total_target - marked_sources * tb_overhead)
            .checked_div(unmarked_count)
            .unwrap_or(total_target / n)
    };

    let mut total_bytes = 0usize;
    for i in 0..n {
        let outlines = base_outlines + usize::from(i < extra_outlines);
        let has_tb = i < marked_sources;
        let img_bytes = if has_tb {
            if total_target / n >= tb_overhead {
                (total_target / n).saturating_sub(tb_overhead)
            } else {
                0
            }
        } else {
            per_source_target.saturating_sub(4_096)
        };

        let base =
            build_base_source_pdf(i, pages, outlines, img_bytes, (n as u64) * 1000 + i as u64);
        let final_bytes = if has_tb {
            add_1000_text_boxes(&base, i)
        } else {
            base
        };
        assert!(
            final_bytes.len() <= MAX_SOURCE_BYTES,
            "source {i} for N={n} exceeded MAX_SOURCE_BYTES: {}",
            final_bytes.len()
        );
        let path = dir.join(format!("source-{i:03}.pdf"));
        fs::write(&path, &final_bytes).expect("write source");
        total_bytes += final_bytes.len();
    }
    println!(
        "generated N={n}: total={} MiB ({total_bytes} bytes), marked_sources={marked_sources}, outlines={TOTAL_OUTLINE_ITEMS}",
        total_bytes / (1024 * 1024)
    );
}

fn prepare_inserted_text_boxes(source_tbs: &[SourceTextBox]) -> Vec<InsertedTextBox> {
    if source_tbs.is_empty() {
        return Vec::new();
    }
    let font = TextBoxFont {
        bytes: bundled_font(),
        face: 0,
        name: "Droid Sans Fallback",
    };
    let mut out = Vec::with_capacity(source_tbs.len());
    for chunk in source_tbs.chunks(MAX_PREPARE_TEXT_BOXES) {
        let texts: Vec<String> = chunk.iter().map(|tb| tb.text.clone()).collect();
        let models = prepare_text_boxes(&font, &texts);
        for (tb, res) in chunk.iter().zip(models) {
            let PreparedTextBoxModel::Prepared(model) = res else {
                panic!("unexpected prepare_text_boxes result during merge: {res:?}");
            };
            out.push(InsertedTextBox {
                page: tb.page,
                rect: tb.rect,
                model,
                style: tb.style,
            });
        }
    }
    out
}

struct RunResult {
    n: usize,
    input_bytes: usize,
    prepared_bytes_total: usize,
    prepare_per_source: Vec<Duration>,
    tb_prepare_total: Duration,
    insert_per_source: Vec<Duration>,
    text_boxes_total: usize,
    outline_time: Duration,
    write_time: Duration,
    total_time: Duration,
    output_bytes: u64,
    total_pages: u32,
    total_outlines: usize,
    output_path: PathBuf,
}

fn median(values: &[Duration]) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

fn max_duration(values: &[Duration]) -> Duration {
    values.iter().copied().max().unwrap_or_default()
}

struct PreparedSourceData {
    title: String,
    bytes: Vec<u8>,
    pages: u32,
    outline: DocumentOutline,
    text_boxes: Vec<SourceTextBox>,
}

fn run_once(n: usize, save_name: Option<&str>) -> RunResult {
    let dir = dataset_dir(n);
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("missing dataset {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    paths.sort();
    assert_eq!(paths.len(), n);

    let total_start = Instant::now();
    let mut input_bytes = 0usize;
    let mut prepared_bytes_total = 0usize;
    let mut prepared_copies: Vec<PreparedSourceData> = Vec::with_capacity(n);
    let mut prepare_per_source = Vec::with_capacity(n);

    // Step 2: Prepare each source and keep clean copy + outline + text boxes in worker memory.
    for path in &paths {
        let t0 = Instant::now();
        let raw = fs::read(path).expect("read source");
        input_bytes += raw.len();
        let prepared = PdfDocument::prepare_source(&raw, None).expect("prepare_source");
        let plain = PdfDocument::from_bytes(&prepared.bytes).expect("open plain copy");
        let outline = plain
            .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
            .expect("read outline");
        prepare_per_source.push(t0.elapsed());
        prepared_bytes_total += prepared.bytes.len();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("source")
            .to_owned();
        prepared_copies.push(PreparedSourceData {
            title: stem,
            bytes: prepared.bytes,
            pages: prepared.pages,
            outline,
            text_boxes: prepared.text_boxes,
        });
    }

    // Steps 3 & 4: Insert each source into a blank document (with ADR 0019 text box recreation).
    let mut merged = PdfDocument::new_blank().expect("new_blank");
    let mut start_pages = Vec::with_capacity(n);
    let mut insert_per_source = Vec::with_capacity(n);
    let mut tb_prepare_total = Duration::ZERO;
    let mut text_boxes_total = 0usize;

    for source in &prepared_copies {
        let tb_t0 = Instant::now();
        let inserted_tbs = prepare_inserted_text_boxes(&source.text_boxes);
        tb_prepare_total += tb_t0.elapsed();
        text_boxes_total += inserted_tbs.len();

        let ins_t0 = Instant::now();
        let at = merged.page_count().expect("page_count");
        merged
            .insert_pages(at, &source.bytes, &inserted_tbs)
            .expect("insert_pages");
        insert_per_source.push(ins_t0.elapsed());
        start_pages.push(at);
    }

    // Step 5a: Build and write outline tree.
    let out_t0 = Instant::now();
    let outline_inputs: Vec<SourceOutlineInput<'_>> = prepared_copies
        .iter()
        .zip(&start_pages)
        .map(|(source, &start_page)| SourceOutlineInput {
            title: &source.title,
            start_page,
            page_count: source.pages,
            outline: &source.outline,
        })
        .collect();
    let entries = build_merged_outline(&outline_inputs, true, "（沒有標題）");
    let expected_outlines = entries.len();
    merged.write_outline(&entries).expect("write_outline");
    let outline_time = out_t0.elapsed();

    // Step 5b: Write merged document to disk.
    let file_name = save_name
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("poc2-merged-{n}.pdf"));
    let output_path = poc_dir().join(file_name);
    let wr_t0 = Instant::now();
    let output_bytes = {
        let file = File::create(&output_path).expect("create merged output");
        let mut writer = BufWriter::new(file);
        merged.write_merged(&mut writer).expect("write_merged")
    };
    let write_time = wr_t0.elapsed();

    let total_pages = merged.page_count().expect("final page_count");
    let total_time = total_start.elapsed();

    RunResult {
        n,
        input_bytes,
        prepared_bytes_total,
        prepare_per_source,
        tb_prepare_total,
        insert_per_source,
        text_boxes_total,
        outline_time,
        write_time,
        total_time,
        output_bytes,
        total_pages,
        total_outlines: expected_outlines,
        output_path,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn print_run_result(r: &RunResult) {
    let prep_sum: Duration = r.prepare_per_source.iter().sum();
    let ins_sum: Duration = r.insert_per_source.iter().sum();
    println!(
        "BENCH N={} input_mib={:.2} prepared_mib={:.2} output_mib={:.2} pages={} outlines={} text_boxes={} \
         prep_total_ms={:.2} prep_med_ms={:.2} prep_max_ms={:.2} \
         tb_prep_total_ms={:.2} \
         ins_total_ms={:.2} ins_med_ms={:.2} ins_max_ms={:.2} \
         outline_ms={:.2} write_ms={:.2} total_ms={:.2} path={}",
        r.n,
        r.input_bytes as f64 / (1024.0 * 1024.0),
        r.prepared_bytes_total as f64 / (1024.0 * 1024.0),
        r.output_bytes as f64 / (1024.0 * 1024.0),
        r.total_pages,
        r.total_outlines,
        r.text_boxes_total,
        ms(prep_sum),
        ms(median(&r.prepare_per_source)),
        ms(max_duration(&r.prepare_per_source)),
        ms(r.tb_prepare_total),
        ms(ins_sum),
        ms(median(&r.insert_per_source)),
        ms(max_duration(&r.insert_per_source)),
        ms(r.outline_time),
        ms(r.write_time),
        ms(r.total_time),
        r.output_path.display(),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--generate") {
        let targets: Vec<usize> = if let Some(n_arg) = args.get(2) {
            vec![n_arg.parse().expect("valid N")]
        } else {
            SOURCE_COUNTS.to_vec()
        };
        for n in targets {
            generate_dataset(n);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--run") {
        let n: usize = args
            .get(2)
            .expect("--run requires N")
            .parse()
            .expect("valid N");
        let save_name = args.get(3).map(String::as_str);
        let result = run_once(n, save_name);
        print_run_result(&result);
        return;
    }

    // Default mode (`cargo run --release -p pdf_worker --example merge_many_poc_bench --locked`):
    // ensure N=16 dataset exists and run 3 iterations.
    generate_dataset(16);
    for run in 1..=3 {
        let result = run_once(16, Some(&format!("poc2-merged-16-run{run}.pdf")));
        print!("run={run} ");
        print_run_result(&result);
    }
}
