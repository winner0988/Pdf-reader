//! Writes one frame payload per kind of IPC message, as the starting corpus of the
//! `worker_messages` fuzz target (QA-03, docs/security/fuzzing.md). Not shipped.
//!
//! Usage: `fuzz_seeds <directory>`

use std::path::PathBuf;

use ipc_contract::frame::encode;
use ipc_contract::types::{
    BlockedAction, DocumentId, FindingKind, LinkId, LinkTarget, OutlineItem, OutlineResult,
    PageLink, PageSize, PageText, Password, Point, Quad, Rect, RequestId, Rotation, SearchHit,
    SecurityFinding, SecurityReport, TextLine,
};
use ipc_contract::worker::{
    FileHandle, OpenedDocument, Raster, WorkerError, WorkerErrorCode, WorkerRequest, WorkerResponse,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: fuzz_seeds <directory>")?,
    );
    std::fs::create_dir_all(&dir)?;
    let (request, doc) = (RequestId(7), DocumentId(1));
    let page = PageSize {
        width_pt: 612.0,
        height_pt: 792.0,
    };
    let point = |x: f32, y: f32| Point { x, y };
    let targets = [
        LinkTarget::Page {
            page_index: 1,
            x: Some(72.0),
            y: None,
        },
        LinkTarget::Uri {
            uri: "https://example.invalid/\u{202E}fdp.exe".to_owned(),
        },
        LinkTarget::Blocked {
            action: BlockedAction::NetworkShare,
            target: Some("\\\\share.example.invalid\\x".to_owned()),
        },
    ];

    let responses = [
        WorkerResponse::hello(),
        WorkerResponse::Opened {
            request,
            document: OpenedDocument {
                pages: vec![page; 3],
                has_outline: true,
                security: SecurityReport {
                    findings: vec![SecurityFinding {
                        kind: FindingKind::JavaScript,
                        count: 2,
                    }],
                    scan_complete: false,
                },
            },
        },
        WorkerResponse::Rendered {
            request,
            raster: Raster {
                width: 2,
                height: 2,
                pixels: vec![255; 16],
            },
        },
        WorkerResponse::Outline {
            request,
            outline: OutlineResult {
                items: targets
                    .iter()
                    .enumerate()
                    .map(|(depth, target)| OutlineItem {
                        title: format!("Chapter {depth}"),
                        depth: depth as u16,
                        target: Some(target.clone()),
                    })
                    .collect(),
                truncated: true,
            },
        },
        WorkerResponse::PageLinks {
            request,
            page_index: 0,
            links: targets
                .iter()
                .enumerate()
                .map(|(index, target)| PageLink {
                    id: LinkId {
                        page_index: 0,
                        index: index as u32,
                    },
                    rect: Rect {
                        x0: 72.0,
                        y0: 80.0,
                        x1: 300.0,
                        y1: 102.0,
                    },
                    target: target.clone(),
                })
                .collect(),
        },
        WorkerResponse::PageText {
            request,
            page_index: 0,
            text: PageText {
                lines: vec![TextLine {
                    text: "Hello 中文".to_owned(),
                    quad: Quad {
                        ul: point(72.0, 80.0),
                        ur: point(129.0, 80.0),
                        ll: point(72.0, 94.0),
                        lr: point(129.0, 94.0),
                    },
                    edges: vec![0.0, 7.0, 13.0, 16.0, 19.0, 26.0, 29.0, 43.0, 57.0],
                }],
                truncated: false,
            },
        },
        WorkerResponse::PageSearched {
            request,
            page_index: 0,
            hits: vec![SearchHit {
                quads: vec![Quad {
                    ul: point(72.0, 80.0),
                    ur: point(120.0, 80.0),
                    ll: point(72.0, 94.0),
                    lr: point(120.0, 94.0),
                }],
            }],
            has_text: true,
        },
        WorkerResponse::Error {
            request: Some(request),
            error: WorkerError {
                code: WorkerErrorCode::Corrupted,
                detail: "broken xref".to_owned(),
            },
        },
    ];
    let requests = [
        WorkerRequest::Open {
            request,
            doc,
            file: FileHandle(0x1f4),
            password: None,
        },
        WorkerRequest::Open {
            request,
            doc,
            file: FileHandle(0x1f4),
            password: Some(Password::new("user".to_owned())),
        },
        WorkerRequest::Render {
            request,
            doc,
            page_index: 0,
            scale: 1.5,
            rotation: Rotation::Cw90,
        },
        WorkerRequest::GetOutline { request, doc },
        WorkerRequest::GetPageLinks {
            request,
            doc,
            page_index: 2,
        },
        WorkerRequest::GetPageText {
            request,
            doc,
            page_index: 1,
        },
        WorkerRequest::SearchPage {
            request,
            doc,
            page_index: 0,
            query: "needle 中文".to_owned(),
            case_sensitive: false,
            max_hits: 100,
        },
        WorkerRequest::Cancel { target: request },
        WorkerRequest::Close { doc },
        WorkerRequest::Shutdown,
    ];

    let mut written = 0;
    for (index, response) in responses.iter().enumerate() {
        std::fs::write(dir.join(format!("response-{index}")), encode(response)?)?;
        written += 1;
    }
    for (index, request) in requests.iter().enumerate() {
        std::fs::write(dir.join(format!("request-{index}")), encode(request)?)?;
        written += 1;
    }
    println!("wrote {written} seeds to {}", dir.display());
    Ok(())
}
