//! The IPC decoding on both sides of the worker boundary (QA-03, ADR 0008): what the main
//! process accepts from a worker that may have been taken over by a hostile PDF, and what the
//! worker accepts from the main process. Every input must be rejected or accepted cleanly:
//! no panic, no unbounded allocation.
#![no_main]

use ipc_contract::frame::{check_hello, decode, read_frame};
use ipc_contract::validate::Validate;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // TEMPORARY (QA-03 acceptance): proves the workflow detects a crash and keeps the sample.
    // Removed in the next commit.
    if data.starts_with(b"QA03") {
        panic!("deliberate crash to check crash detection");
    }
    // Main process side: decode a payload, then validate it the way the host does.
    if let Ok(response) = decode::<WorkerResponse>(data) {
        let _ = check_hello(&response);
        let _ = response.validate();
    }
    // Worker side.
    let _ = decode::<WorkerRequest>(data);
    // The framing itself, as a stream of length-prefixed frames.
    let mut stream = data;
    while let Ok(Some(payload)) = read_frame(&mut stream) {
        let _ = decode::<WorkerResponse>(&payload).map(|response| response.validate());
    }
});
