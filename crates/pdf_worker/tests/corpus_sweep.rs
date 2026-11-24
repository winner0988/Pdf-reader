//! Integration test for corpus_sweep using repo's tests/corpus (Issue #439, B5-02).
#![cfg(windows)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use worker_host::sweep::run_sweep;

fn worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pdf_worker"))
}

fn repo_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

#[test]
fn test_corpus_sweep_on_repo_corpus() {
    let worker = worker_path();
    let corpus_dir = repo_corpus_dir();
    let output_dir =
        std::env::temp_dir().join(format!("corpus_sweep_int_test_{}", std::process::id()));

    let _ = fs::remove_dir_all(&output_dir);

    let summary = run_sweep(
        &worker,
        &corpus_dir,
        &output_dir,
        Duration::from_secs(30),
        None,
        None,
        false,
        None,
        None,
    )
    .expect("run_sweep on tests/corpus must succeed");

    assert!(summary.total_files > 0, "must find files in tests/corpus");

    // Read results.jsonl
    let jsonl = fs::read_to_string(output_dir.join("results.jsonl")).expect("read results.jsonl");
    let lines: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        summary.total_files,
        "each file must have exactly one line in results.jsonl"
    );

    // Verify at least one normal file has ok result and correct page count (single-page.pdf)
    let normal_line = lines
        .iter()
        .find(|l| l.contains("single-page.pdf"))
        .expect("must contain single-page.pdf");
    assert!(
        normal_line.contains("\"result\":\"ok\""),
        "single-page.pdf must be ok, got: {normal_line}"
    );
    assert!(
        normal_line.contains("\"pages\":1"),
        "single-page.pdf must have 1 page, got: {normal_line}"
    );

    // Whether each file gets a fresh worker is checked by `each_file_runs_in_a_fresh_worker`.

    // Verify at least one malformed file is open_failed
    let malformed_line = lines
        .iter()
        .find(|l| l.contains("empty.pdf") || l.contains("not-a-pdf.pdf"))
        .expect("must contain empty.pdf or not-a-pdf.pdf");
    assert!(
        malformed_line.contains("\"result\":\"open_failed\""),
        "broken file must be open_failed, got: {malformed_line}"
    );

    // Verify summary.md and summary.json exist and contain summary statistics
    let md = fs::read_to_string(output_dir.join("summary.md")).expect("read summary.md");
    assert!(md.contains("公開 PDF 語料庫穩健性掃描摘要"));
    assert!(md.contains("各語料庫結果統計"));

    let json = fs::read_to_string(output_dir.join("summary.json")).expect("read summary.json");
    assert!(json.contains("\"total_files\""));
    assert!(json.contains("\"by_corpus\""));

    let _ = fs::remove_dir_all(&output_dir);
}

/// One page of `side` x `side` points, filled, so that rendering it at scale 1 takes a raster of
/// `side` x `side` pixels.
fn square_page(side: u32) -> Vec<u8> {
    let content = format!("0 0 1 rg 0 0 {side} {side} re f");
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {side} {side}] /Resources << >> /Contents 4 0 R >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ])
}

/// The peak memory of a worker's job only grows while that worker lives (#643). A small page swept
/// after a large one can only show the lower peak if it got a worker of its own: with one worker
/// for both, its peak would be at least the large page's. The large page's raster (2000 x 2000
/// pixels, about 16 MB) is far above the run-to-run noise of the peak (about 1 MB), so two pages of
/// the same size, which can show the very same peak in fresh workers, are not compared.
#[test]
fn each_file_runs_in_a_fresh_worker() {
    let dir =
        std::env::temp_dir().join(format!("corpus_sweep_fresh_worker_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let corpus_dir = dir.join("corpus");
    let output_dir = dir.join("out");
    fs::create_dir_all(&corpus_dir).unwrap();
    // The sweep goes in file name order: the large page first.
    fs::write(corpus_dir.join("a-large-page.pdf"), square_page(2000)).unwrap();
    fs::write(corpus_dir.join("b-small-page.pdf"), square_page(100)).unwrap();

    run_sweep(
        &worker_path(),
        &corpus_dir,
        &output_dir,
        Duration::from_secs(30),
        Some(2),
        None,
        false,
        None,
        None,
    )
    .expect("run_sweep on the two pages must succeed");

    let jsonl = fs::read_to_string(output_dir.join("results.jsonl")).expect("read results.jsonl");
    let peaks: Vec<(String, u64)> = jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("valid json line");
            assert_eq!(v["result"], "ok", "{l}");
            let name = v["relative_path"]
                .as_str()
                .expect("relative_path")
                .to_owned();
            let peak = v["peak_memory_bytes"].as_u64().expect("peak_memory_bytes");
            (name, peak)
        })
        .collect();
    let [(large_name, large), (small_name, small)] = peaks.as_slice() else {
        panic!("expected two results, got {peaks:?}");
    };
    assert_eq!(
        (large_name.as_str(), small_name.as_str()),
        ("a-large-page.pdf", "b-small-page.pdf")
    );
    assert!(
        small < large,
        "the small page must have a worker of its own, so its peak is below the large page's: small={small}, large={large}"
    );

    let _ = fs::remove_dir_all(&dir);
}
