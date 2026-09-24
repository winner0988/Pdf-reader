//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! Started by the main process inside the sandbox; talks only over stdin/stdout using the IPC
//! contract. Diagnostics go to stderr. Stdout must carry nothing but frames.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    match pdf_worker::serve::serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pdf_worker: {error}");
            ExitCode::FAILURE
        }
    }
}
