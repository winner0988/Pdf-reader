//! Test-only worker that can be driven to emit protocol violations or delayed responses (Issue #422).
//!
//! Not shipped with the installer (test = false, excluded from tauri bundle externalBin).

#[cfg(windows)]
fn main() -> std::io::Result<()> {
    run()
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn run() -> std::io::Result<()> {
    use std::io::{self, Write};
    use std::time::Duration;

    use ipc_contract::frame;
    use ipc_contract::types::RequestId;
    use ipc_contract::worker::{InspectedFont, Raster, WorkerRequest, WorkerResponse};

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    // 1. Initial handshake
    frame::send(&mut stdout, &WorkerResponse::hello())
        .map_err(|error| io::Error::other(error.to_string()))?;
    stdout.flush()?;

    // 2. Serve requests
    while let Ok(Some(request)) = frame::receive::<_, WorkerRequest>(&mut stdin) {
        match request {
            WorkerRequest::InspectFont {
                bytes,
                request: req_id,
            } => match bytes.as_slice() {
                b"corrupted_frame" => {
                    // Send an invalid frame that cannot be decoded as postcard
                    stdout.write_all(&[0x05, 0xff, 0xff, 0xff, 0xff, 0xff])?;
                    stdout.flush()?;
                }
                b"validation_failure" => {
                    // Send a response that fails Validate::validate() (pixel buffer size mismatch)
                    let response = WorkerResponse::Rendered {
                        request: req_id,
                        raster: Raster {
                            width: 10,
                            height: 10,
                            pixels: vec![0u8; 5],
                        },
                    };
                    frame::send(&mut stdout, &response)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    stdout.flush()?;
                }
                b"zero_id" => {
                    let response = WorkerResponse::FontInspected {
                        request: RequestId(0),
                        faces: vec![],
                    };
                    frame::send(&mut stdout, &response)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    stdout.flush()?;
                }
                b"future_id" => {
                    let response = WorkerResponse::FontInspected {
                        request: RequestId(req_id.0.wrapping_add(100)),
                        faces: vec![],
                    };
                    frame::send(&mut stdout, &response)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    stdout.flush()?;
                }
                b"delayed_wrong_id" => {
                    // Send wrong request id repeatedly with short delays
                    for _ in 0..10 {
                        std::thread::sleep(Duration::from_millis(30));
                        let response = WorkerResponse::FontInspected {
                            request: RequestId(req_id.0.wrapping_add(999)),
                            faces: vec![],
                        };
                        if frame::send(&mut stdout, &response).is_err() {
                            break;
                        }
                        if stdout.flush().is_err() {
                            break;
                        }
                    }
                }
                b"second_hello" => {
                    frame::send(&mut stdout, &WorkerResponse::hello())
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    stdout.flush()?;
                }
                b"slow" => {
                    // Sleep past the test deadline without replying
                    std::thread::sleep(Duration::from_secs(2));
                }
                _ => {
                    // Normal valid response
                    let response = WorkerResponse::FontInspected {
                        request: req_id,
                        faces: vec![InspectedFont {
                            face: 0,
                            name: "Fake Font".to_owned(),
                            family: "Fake".to_owned(),
                            bold: false,
                            italic: false,
                        }],
                    };
                    frame::send(&mut stdout, &response)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    stdout.flush()?;
                }
            },
            WorkerRequest::WriteAttachment {
                request: req_id, ..
            } => {
                // Wrong response type: WriteAttachment expects AttachmentWritten, send Saved instead
                let response = WorkerResponse::Saved {
                    request: req_id,
                    bytes: 100,
                    incremental: false,
                    redaction: None,
                };
                frame::send(&mut stdout, &response)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                stdout.flush()?;
            }
            WorkerRequest::Shutdown => break,
            _ => {}
        }
    }

    Ok(())
}
