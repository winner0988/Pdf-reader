//! Writes one frame payload per kind of IPC message, as the starting corpus of the
//! `worker_messages` fuzz target (QA-03, docs/security/fuzzing.md). Not shipped.
//!
//! Usage: `fuzz_seeds <directory>`

use std::path::PathBuf;

use ipc_contract::frame::encode;
use ipc_contract::types::{
    AnnotationId, AnnotationKind, BlockedAction, DocumentId, DocumentPermissions, FieldId,
    FieldKind, FieldOption, FindingKind, FormField, HighlightColor, HighlightMark, LinkId,
    LinkTarget, OutlineItem, OutlineResult, PageAnnotation, PageLink, PageSize, PageText, Password,
    Point, Quad, Rect, RequestId, Rotation, SearchHit, SecurityFinding, SecurityReport, TextLine,
};
use ipc_contract::worker::{
    FileHandle, OpenedDocument, Raster, WorkerEdit, WorkerError, WorkerErrorCode, WorkerRequest,
    WorkerResponse,
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
                has_form: true,
                security: SecurityReport {
                    findings: vec![SecurityFinding {
                        kind: FindingKind::JavaScript,
                        count: 2,
                    }],
                    scan_complete: false,
                },
                permissions: DocumentPermissions {
                    copy: false,
                    print: true,
                    print_high_quality: false,
                    modify: false,
                    assemble: true,
                    annotate: false,
                    fill_forms: true,
                },
                encrypted: true,
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
        WorkerResponse::PageFields {
            request,
            page_index: 0,
            fields: vec![
                FormField {
                    id: FieldId(6),
                    group: FieldId(6),
                    kind: FieldKind::Text,
                    rect: Rect {
                        x0: 72.0,
                        y0: 100.0,
                        x1: 300.0,
                        y1: 124.0,
                    },
                    label: Some("Your name".to_owned()),
                    value: "Jane\nPublic".to_owned(),
                    on_value: None,
                    options: vec![],
                    read_only: false,
                    required: true,
                    multiline: true,
                    password: false,
                    editable: false,
                    multi_select: false,
                    max_len: Some(40),
                    has_script: true,
                },
                FormField {
                    id: FieldId(22),
                    group: FieldId(22),
                    kind: FieldKind::Combo,
                    rect: Rect {
                        x0: 72.0,
                        y0: 200.0,
                        x1: 300.0,
                        y1: 224.0,
                    },
                    label: None,
                    value: "TW".to_owned(),
                    on_value: None,
                    options: vec![
                        FieldOption {
                            value: "TW".to_owned(),
                            label: "臺灣".to_owned(),
                        },
                        FieldOption {
                            value: "JP".to_owned(),
                            label: "Japan".to_owned(),
                        },
                    ],
                    read_only: false,
                    required: false,
                    multiline: false,
                    password: false,
                    editable: true,
                    multi_select: false,
                    max_len: None,
                    has_script: false,
                },
            ],
        },
        WorkerResponse::PageAnnotations {
            request,
            page_index: 0,
            annotations: vec![
                PageAnnotation {
                    id: AnnotationId(12),
                    kind: AnnotationKind::Highlight,
                    rect: Rect {
                        x0: 72.0,
                        y0: 80.0,
                        x1: 300.0,
                        y1: 102.0,
                    },
                    color: Some(HighlightColor::Yellow),
                    text: None,
                },
                PageAnnotation {
                    id: AnnotationId(13),
                    kind: AnnotationKind::Note,
                    rect: Rect {
                        x0: 400.0,
                        y0: 80.0,
                        x1: 420.0,
                        y1: 100.0,
                    },
                    color: None,
                    text: Some("第一行\nsecond line".to_owned()),
                },
            ],
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
        WorkerResponse::Png {
            request,
            png: b"\x89PNG\r\n\x1a\nIHDR".to_vec(),
        },
        WorkerResponse::Jpeg {
            request,
            jpeg: b"\xFF\xD8\xFF\xE0JFIF".to_vec(),
        },
        WorkerResponse::Edited {
            request,
            pages: vec![
                PageSize {
                    width_pt: 792.0,
                    height_pt: 612.0,
                },
                page,
            ],
        },
        WorkerResponse::Rebased { request },
        WorkerResponse::Saved {
            request,
            bytes: 4096,
            incremental: true,
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
        WorkerRequest::RenderPng {
            request,
            doc,
            page_index: 0,
            scale: 150.0 / 72.0,
        },
        WorkerRequest::RenderJpeg {
            request,
            doc,
            page_index: 0,
            scale: 300.0 / 72.0,
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
        WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index: 1,
        },
        WorkerRequest::GetPageFields {
            request,
            doc,
            page_index: 0,
        },
        WorkerRequest::SearchPage {
            request,
            doc,
            page_index: 0,
            query: "needle 中文".to_owned(),
            case_sensitive: false,
            max_hits: 100,
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::RotatePages {
                pages: vec![0, 2],
                degrees: 90,
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::DeletePages { pages: vec![3, 1] },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::MovePages {
                pages: vec![4],
                before: 0,
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::InsertBlankPage { at: 1, like: 0 },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddHighlight {
                marks: vec![HighlightMark {
                    page: 0,
                    quads: vec![Quad {
                        ul: Point { x: 72.0, y: 80.0 },
                        ur: Point { x: 300.0, y: 80.0 },
                        ll: Point { x: 72.0, y: 102.0 },
                        lr: Point { x: 300.0, y: 102.0 },
                    }],
                }],
                color: HighlightColor::Green,
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddNote {
                page: 1,
                at: Point { x: 100.0, y: 120.0 },
                text: "附註".to_owned(),
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::DeleteAnnotation {
                page: 0,
                annotation: AnnotationId(12),
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::SetHighlightColor {
                page: 0,
                annotation: AnnotationId(12),
                color: HighlightColor::Pink,
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::SetNoteText {
                page: 1,
                annotation: AnnotationId(13),
                text: "改過的附註".to_owned(),
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::SetFieldValue {
                page: 0,
                field: FieldId(6),
                value: "Jane Q. Public".to_owned(),
            },
        },
        WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::FlattenForm,
        },
        WorkerRequest::Revert {
            request,
            doc,
            edits: vec![
                WorkerEdit::DeletePages { pages: vec![0] },
                WorkerEdit::MovePages {
                    pages: vec![1],
                    before: 0,
                },
            ],
            password: Some(Password::new("user".to_owned())),
        },
        WorkerRequest::Rebase {
            request,
            doc,
            file: FileHandle(0x2b0),
        },
        WorkerRequest::SavePages {
            request,
            doc,
            pages: vec![0, 2, 3],
            file: FileHandle(0x2b8),
        },
        WorkerRequest::Save {
            request,
            doc,
            file: FileHandle(0x2a8),
        },
        WorkerRequest::PrivacyCopy {
            request,
            doc,
            file: FileHandle(0x2ac),
            id: [0x5a; 16],
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
