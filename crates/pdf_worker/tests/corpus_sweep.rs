//! Integration test for corpus_sweep using repo's tests/corpus (Issue #439, B5-02).
#![cfg(windows)]

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

    // Verify that each file runs in a fresh worker and does not share a cumulative peak memory:
    // multi-page-10.pdf is processed before single-page.pdf in alphabetical order.
    // If the worker were shared, single-page.pdf's peak memory would be >= multi-page-10.pdf's.
    let ok_entries: Vec<serde_json::Value> = lines
        .iter()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("valid json line"))
        .filter(|v| v["result"] == "ok" && v["peak_memory_bytes"].is_number())
        .collect();
    let single_page = ok_entries
        .iter()
        .find(|v| {
            v["relative_path"]
                .as_str()
                .is_some_and(|p| p.ends_with("single-page.pdf"))
        })
        .expect("must contain single-page.pdf");
    let multi_page = ok_entries
        .iter()
        .find(|v| {
            v["relative_path"]
                .as_str()
                .is_some_and(|p| p.ends_with("multi-page-10.pdf"))
        })
        .expect("must contain multi-page-10.pdf");
    let single_mem = single_page["peak_memory_bytes"].as_u64().unwrap();
    let multi_mem = multi_page["peak_memory_bytes"].as_u64().unwrap();
    assert_ne!(
        single_mem, multi_mem,
        "each file must run in a fresh worker so peak_memory_bytes is not identical: single={single_mem}, multi={multi_mem}"
    );

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
