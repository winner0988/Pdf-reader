//! Page render error handling for malformed content (Issue #552), and for tiling patterns that
//! cannot be painted within the render timeout (Issue #550).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::types::Rotation;
use ipc_contract::worker::{WorkerErrorCode, WorkerRequest, WorkerResponse};
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
fn rendering_malformed_predictor_colors_reports_corrupted_and_worker_stays_running() {
    let mut host = host();
    let (doc, opened) = host
        .open(&corpus("malformed/predictor-colors.pdf"))
        .expect("open predictor-colors.pdf");
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
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
            assert!(
                error.detail.contains("too many color components"),
                "detail should preserve MuPDF message, got: {}",
                error.detail
            );
        }
        Err(ref other) => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
        Ok(ref response) => panic!("expected render refusal, got success: {response:?}"),
    }

    assert!(
        host.is_running(),
        "the worker must not be terminated by a failed render"
    );

    // The same worker can open a benign document afterwards and render it.
    let (doc2, opened) = host
        .open(&corpus("benign/single-page.pdf"))
        .expect("open benign document on the same worker");
    assert!(matches!(opened, WorkerResponse::Opened { .. }));
    assert!(host.is_running());

    let render_res2 = host.request(|request| WorkerRequest::Render {
        request,
        doc: doc2,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
    });
    assert!(matches!(render_res2, Ok(WorkerResponse::Rendered { .. })));
    assert!(host.is_running());
}

#[test]
fn render_region_png_jpeg_on_malformed_predictor_colors_report_corrupted() {
    use ipc_contract::types::RasterRegion;

    let mut host = host();
    let (doc, _) = host
        .open(&corpus("malformed/predictor-colors.pdf"))
        .expect("open predictor-colors.pdf");

    // RenderRegion
    let region_res = host.request(|request| WorkerRequest::RenderRegion {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
        rotation: Rotation::None,
        region: RasterRegion {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        without_annotations: false,
    });
    match region_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
        }
        ref other => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
    }

    // RenderPng
    let png_res = host.request(|request| WorkerRequest::RenderPng {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
    });
    match png_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
        }
        ref other => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
    }

    // RenderJpeg
    let jpeg_res = host.request(|request| WorkerRequest::RenderJpeg {
        request,
        doc,
        page_index: 0,
        scale: 1.0,
    });
    match jpeg_res {
        Err(HostError::Worker(ref error)) => {
            assert_eq!(error.code, WorkerErrorCode::Corrupted);
        }
        ref other => panic!("expected HostError::Worker(Corrupted), got: {other:?}"),
    }

    assert!(host.is_running());
}

/// The fork's build-time limits refuse these patterns before painting (#550): a tile as large as
/// the page repeated every 0.1 point, and a pattern with a blend mode, whose content MuPDF runs
/// once per repeat. Without them each render runs into the host's 10 s timeout.
#[test]
fn tiling_patterns_that_cannot_finish_report_limit_exceeded_at_once() {
    let mut host = host();
    for name in [
        "malformed/tiling-pattern-large-tile.pdf",
        "malformed/tiling-pattern-blend.pdf",
    ] {
        let (doc, _) = host
            .open(&corpus(name))
            .unwrap_or_else(|error| panic!("open {name}: {error:?}"));
        let started = Instant::now();
        let render_res = host.request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        });
        match render_res {
            Err(HostError::Worker(ref error)) => {
                assert_eq!(error.code, WorkerErrorCode::LimitExceeded, "{name}");
                assert!(
                    error.detail.contains("tiling pattern"),
                    "{name}: detail should keep MuPDF's message, got: {}",
                    error.detail
                );
            }
            ref other => {
                panic!("{name}: expected HostError::Worker(LimitExceeded), got: {other:?}")
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{name}: refused only after {:?}",
            started.elapsed()
        );
        assert!(
            host.is_running(),
            "{name}: the worker must not be terminated by a refused pattern"
        );
    }
}
