//! JPEG 2000 regression tests (#539, #554).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ipc_contract::types::Rotation;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig {
            request_timeout: Duration::from_secs(10),
            ..HostConfig::default()
        },
    )
}

#[test]
fn benign_jpx_image_renders_successfully() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("benign/jpx-image.pdf"))
        .expect("open benign/jpx-image.pdf");
    match opened {
        WorkerResponse::Opened { document, .. } => {
            assert_eq!(document.pages.len(), 1);
        }
        ref other => panic!("expected Opened, got: {other:?}"),
    }

    let render_res = host.request(|request| WorkerRequest::Render {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });

    match render_res {
        Ok(WorkerResponse::Rendered { raster, .. }) => {
            // Count non-white pixels: an 8-bit unsigned JPX image with empty packets
            // reconstructs to mid-gray (128, 128, 128). White is (255, 255, 255).
            let mut non_white = 0;
            for &[r, g, b, _a] in raster.pixels.as_chunks::<4>().0 {
                if !(r == 255 && g == 255 && b == 255) {
                    non_white += 1;
                }
            }
            assert!(
                non_white > 0,
                "benign/jpx-image.pdf must render a non-white image on the page"
            );
        }
        ref other => panic!("expected Rendered, got: {other:?}"),
    }

    assert!(host.is_running());
}

#[test]
fn malformed_jpx_tile_parts_beyond_tlm_does_not_crash_worker() {
    let mut worker_died = 0;
    for iter in 0..20 {
        let mut host = host();
        let open_res = host.open(&corpus("malformed/jpx-tile-parts-beyond-tlm.pdf"));
        let doc = match open_res {
            Ok((doc, _opened)) => doc,
            Err(HostError::Worker(_)) => continue,
            Err(HostError::Crashed) => {
                eprintln!("iteration {iter}: worker crashed on open");
                worker_died += 1;
                continue;
            }
            Err(HostError::ProtocolViolation(err)) => {
                eprintln!("iteration {iter}: protocol violation on open: {err}");
                worker_died += 1;
                continue;
            }
            Err(HostError::Timeout) => {
                eprintln!("iteration {iter}: timeout on open");
                worker_died += 1;
                continue;
            }
            Err(other) => panic!("iteration {iter}: unexpected error on open: {other:?}"),
        };

        let render_res = host.request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        });

        match render_res {
            Ok(WorkerResponse::Rendered { .. }) => {}
            Err(HostError::Worker(_)) => {}
            Err(HostError::Crashed) => {
                eprintln!("iteration {iter}: worker crashed on render");
                worker_died += 1;
                continue;
            }
            Err(HostError::ProtocolViolation(err)) => {
                eprintln!("iteration {iter}: protocol violation on render: {err}");
                worker_died += 1;
                continue;
            }
            Err(HostError::Timeout) => {
                eprintln!("iteration {iter}: timeout on render");
                worker_died += 1;
                continue;
            }
            other => panic!("iteration {iter}: unexpected render response: {other:?}"),
        }

        if !host.is_running() {
            eprintln!("iteration {iter}: worker not running after render");
            worker_died += 1;
            continue;
        }

        let text_res = host.request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index: 0,
        });

        match text_res {
            Ok(WorkerResponse::PageText { .. }) => {}
            Err(HostError::Worker(_)) => {}
            Err(HostError::Crashed) => {
                eprintln!("iteration {iter}: worker crashed on text extraction");
                worker_died += 1;
                continue;
            }
            Err(HostError::ProtocolViolation(err)) => {
                eprintln!("iteration {iter}: protocol violation on text extraction: {err}");
                worker_died += 1;
                continue;
            }
            Err(HostError::Timeout) => {
                eprintln!("iteration {iter}: timeout on text extraction");
                worker_died += 1;
                continue;
            }
            other => panic!("iteration {iter}: unexpected text response: {other:?}"),
        }

        if !host.is_running() {
            eprintln!("iteration {iter}: worker not running after text extraction");
            worker_died += 1;
            continue;
        }

        // After rendering on the malformed document, verify on the same worker
        // that opening and rendering benign/single-page.pdf succeeds.
        let single_res = host.open(&corpus("benign/single-page.pdf"));
        let (single_doc, opened_single) = match single_res {
            Ok((doc, opened)) => (doc, opened),
            Err(err) => {
                eprintln!(
                    "iteration {iter}: failed to open benign/single-page.pdf on same worker: {err:?}"
                );
                worker_died += 1;
                continue;
            }
        };
        assert!(matches!(opened_single, WorkerResponse::Opened { .. }));

        let render_single = host.request(|request| WorkerRequest::Render {
            request,
            doc: single_doc,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        });
        match render_single {
            Ok(WorkerResponse::Rendered { .. }) => {}
            Err(err) => {
                eprintln!(
                    "iteration {iter}: failed to render benign/single-page.pdf on same worker: {err:?}"
                );
                worker_died += 1;
                continue;
            }
            other => panic!("iteration {iter}: unexpected single-page render response: {other:?}"),
        }

        if !host.is_running() {
            eprintln!("iteration {iter}: worker not running after single-page render");
            worker_died += 1;
            continue;
        }
    }

    assert_eq!(
        worker_died, 0,
        "{worker_died} of 20 iterations had worker terminations or crashes"
    );
}
