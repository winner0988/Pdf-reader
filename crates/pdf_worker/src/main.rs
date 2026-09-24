//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! Started by the main process inside the sandbox; talks only over stdin/stdout using the IPC
//! contract. Diagnostics go to stderr. Stdout must carry nothing but frames.

use std::fs::File;
use std::io;
use std::process::ExitCode;

/// Stdin without the standard library's buffer: a request may carry a password (MVP-16), and
/// that buffer could not be wiped after reading it. Frames are read with exact-size reads, so
/// nothing is lost without it.
fn unbuffered_stdin() -> io::Result<File> {
    #[cfg(windows)]
    let handle = {
        use std::os::windows::io::AsHandle;
        io::stdin().as_handle().try_clone_to_owned()?
    };
    #[cfg(not(windows))]
    let handle = {
        use std::os::fd::AsFd;
        io::stdin().as_fd().try_clone_to_owned()?
    };
    Ok(File::from(handle))
}

fn main() -> ExitCode {
    let input = match unbuffered_stdin() {
        Ok(input) => input,
        Err(error) => {
            eprintln!("pdf_worker: stdin: {error}");
            return ExitCode::FAILURE;
        }
    };
    match pdf_worker::serve::serve(input, io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pdf_worker: {error}");
            ExitCode::FAILURE
        }
    }
}
