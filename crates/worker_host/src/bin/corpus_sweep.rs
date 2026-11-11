//! Sweeps pinned public PDF corpora through the sandboxed worker (Issue #439, B5-02).
//!
//! Usage: `corpus_sweep <corpus_dir> <output_dir> [--file-seconds 60] [--expect-files N]`

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match worker_host::sweep::run_cli() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("corpus_sweep: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("corpus_sweep: only supported on Windows");
}
