//! The worker's request loop: reads `WorkerRequest` frames, answers with `WorkerResponse` frames.
//!
//! Generic over the input and output streams so it can be tested in-process; `main` wires it to
//! stdin and stdout, which are the only channels the sandbox leaves the worker.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};

use ipc_contract::frame::{self, FrameError};
use ipc_contract::limits::{
    MAX_ERROR_MESSAGE_BYTES, MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS, MAX_PAGE_COUNT, MAX_TEXT_BYTES,
};
use ipc_contract::text::{classify_uri, clean_display_text};
use ipc_contract::types::{
    DocumentId, LinkTarget, OutlineItem, OutlineResult, PageSize, RequestId, SecurityReport,
};
use ipc_contract::worker::{
    FileHandle, OpenedDocument, Raster, WorkerError, WorkerErrorCode, WorkerRequest, WorkerResponse,
};

use crate::engine::{EngineError, OutlineTarget, PdfDocument};
use crate::handle;

/// Largest document the worker reads (mirrors `worker_host::MAX_DOCUMENT_BYTES`).
const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

/// Serves requests until Shutdown or end of input.
pub fn serve<R: Read, W: Write>(mut input: R, mut output: W) -> Result<(), FrameError> {
    frame::send(&mut output, &WorkerResponse::hello())?;
    let mut documents: HashMap<DocumentId, PdfDocument> = HashMap::new();

    while let Some(request) = frame::receive::<_, WorkerRequest>(&mut input)? {
        let response = match request {
            WorkerRequest::Open { request, doc, file } => {
                Some(open(&mut documents, request, doc, file))
            }
            WorkerRequest::Render {
                request,
                doc,
                page_index,
                scale,
                rotation,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match document.render(page_index, scale, rotation.degrees()) {
                    Ok(page) => WorkerResponse::Rendered {
                        request,
                        raster: Raster {
                            width: page.width,
                            height: page.height,
                            pixels: page.rgba,
                        },
                    },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
                },
            }),
            WorkerRequest::GetOutline { request, doc } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match outline(document) {
                    Ok(outline) => WorkerResponse::Outline { request, outline },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                },
            }),
            WorkerRequest::GetPageLinks { request, .. } | WorkerRequest::Search { request, .. } => {
                Some(error(
                    request,
                    WorkerErrorCode::InvalidRequest,
                    "not implemented yet (MVP-09, MVP-10, MVP-12)",
                ))
            }
            // Requests are handled one at a time, so there is nothing in flight to cancel.
            WorkerRequest::Cancel { .. } => None,
            WorkerRequest::Close { doc } => {
                documents.remove(&doc);
                None
            }
            WorkerRequest::Shutdown => break,
        };
        if let Some(response) = response {
            frame::send(&mut output, &response)?;
        }
    }
    Ok(())
}

fn open(
    documents: &mut HashMap<DocumentId, PdfDocument>,
    request: RequestId,
    doc: DocumentId,
    file: FileHandle,
) -> WorkerResponse {
    let bytes = match read_limited(handle::take_file(file)) {
        Ok(bytes) => bytes,
        Err(response) => return response(request),
    };
    let document = match PdfDocument::from_bytes(&bytes) {
        Ok(document) => document,
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    };
    let count = match document.page_count() {
        Ok(0) => return error(request, WorkerErrorCode::Corrupted, "document has no pages"),
        Ok(count) if count > MAX_PAGE_COUNT => {
            return error(request, WorkerErrorCode::LimitExceeded, "too many pages");
        }
        Ok(count) => count,
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    };
    let mut pages = Vec::with_capacity(count as usize);
    for index in 0..count {
        match document.page_size(index) {
            Ok((width_pt, height_pt)) => pages.push(PageSize {
                width_pt,
                height_pt,
            }),
            Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
        }
    }
    let has_outline = document.has_outline();
    documents.insert(doc, document);
    WorkerResponse::Opened {
        request,
        document: OpenedDocument {
            pages,
            has_outline,
            // The active-content scan arrives with MVP-11; until then nothing has been scanned.
            security: SecurityReport {
                findings: Vec::new(),
                scan_complete: false,
            },
        },
    }
}

type ErrorFor = fn(RequestId) -> WorkerResponse;

fn read_limited(file: Option<File>) -> Result<Vec<u8>, ErrorFor> {
    let Some(file) = file else {
        return Err(|request| {
            error(
                request,
                WorkerErrorCode::InvalidRequest,
                "invalid file handle",
            )
        });
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Err(|request| {
            error(
                request,
                WorkerErrorCode::Unreadable,
                "could not read the file",
            )
        });
    }
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(|request| error(request, WorkerErrorCode::LimitExceeded, "file too large"));
    }
    Ok(bytes)
}

/// The document's outline as the contract carries it: titles cleaned for display, page targets
/// checked against the page count, everything else classified as a web link or a blocked action.
fn outline(document: &PdfDocument) -> Result<OutlineResult, EngineError> {
    let page_count = document.page_count()?;
    let outline = document.outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)?;
    let items = outline
        .entries
        .into_iter()
        .map(|entry| OutlineItem {
            title: clean_display_text(&entry.title, MAX_TEXT_BYTES as usize),
            depth: entry.depth,
            target: match entry.target {
                OutlineTarget::Page(page_index) if page_index < page_count => {
                    Some(LinkTarget::Page {
                        page_index,
                        x: None,
                        y: None,
                    })
                }
                OutlineTarget::Page(_) | OutlineTarget::None => None,
                OutlineTarget::Uri(uri) => Some(classify_uri(&uri)),
                OutlineTarget::Blocked { action, target } => Some(LinkTarget::Blocked {
                    action,
                    target: target
                        .map(|text| clean_display_text(&text, MAX_TEXT_BYTES as usize))
                        .filter(|text| !text.is_empty()),
                }),
            },
        })
        .collect();
    Ok(OutlineResult {
        items,
        truncated: outline.truncated,
    })
}

fn engine_error(
    request: RequestId,
    engine: &EngineError,
    other: WorkerErrorCode,
) -> WorkerResponse {
    let code = match engine {
        EngineError::NotPdf => WorkerErrorCode::NotPdf,
        EngineError::Encrypted => WorkerErrorCode::Encrypted,
        EngineError::PageOutOfRange(_) => WorkerErrorCode::PageOutOfRange,
        EngineError::InvalidScale | EngineError::InvalidRotation => WorkerErrorCode::InvalidRequest,
        EngineError::TooLarge { .. } => WorkerErrorCode::LimitExceeded,
        EngineError::MuPdf(_) => other,
    };
    error(request, code, &engine.to_string())
}

fn error(request: RequestId, code: WorkerErrorCode, detail: &str) -> WorkerResponse {
    let mut detail = detail.to_owned();
    let limit = MAX_ERROR_MESSAGE_BYTES as usize;
    if detail.len() > limit {
        let mut cut = limit;
        while !detail.is_char_boundary(cut) {
            cut -= 1;
        }
        detail.truncate(cut);
    }
    WorkerResponse::Error {
        request: Some(request),
        error: WorkerError { code, detail },
    }
}
