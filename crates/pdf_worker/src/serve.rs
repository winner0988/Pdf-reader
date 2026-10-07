//! The worker's request loop: reads `WorkerRequest` frames, answers with `WorkerResponse` frames.
//!
//! Generic over the input and output streams so it can be tested in-process; `main` wires it to
//! stdin and stdout, which are the only channels the sandbox leaves the worker.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};

use ipc_contract::frame::{self, FrameError};
use ipc_contract::limits::{
    MAX_DOCUMENT_BYTES, MAX_ERROR_MESSAGE_BYTES, MAX_LINKS_PER_PAGE, MAX_OUTLINE_DEPTH,
    MAX_OUTLINE_ITEMS, MAX_PAGE_COUNT, MAX_PAGE_TEXT_CHARS, MAX_SEARCH_HITS, MAX_SOURCE_BYTES,
    MAX_STAMP_SOURCE_BYTES, MAX_TEXT_BYTES, MAX_UNDO_EDITS,
};
use ipc_contract::text::{classify_uri, clean_display_text};
use ipc_contract::types::{
    DocumentId, LinkId, LinkTarget, OutlineItem, OutlineResult, PageLink, PageSize, Password, Rect,
    RequestId,
};
use ipc_contract::worker::{
    FileHandle, OpenedDocument, Raster, WorkerEdit, WorkerError, WorkerErrorCode, WorkerRequest,
    WorkerResponse,
};

use crate::engine::{EngineError, OutlineTarget, PdfDocument, prepare_stamp_picture};
use crate::handle;
use crate::scan::ScanBudget;
use crate::signatures;

/// Serves requests until Shutdown or end of input.
pub fn serve<R: Read, W: Write>(mut input: R, mut output: W) -> Result<(), FrameError> {
    frame::send(&mut output, &WorkerResponse::hello())?;
    let mut documents: HashMap<DocumentId, PdfDocument> = HashMap::new();
    // The bytes each document was opened from (or last saved to), to open it again for undo
    // (ADR 0013). For a document opened with a password they are still encrypted; the password
    // itself is not kept: undo asks for it again (#94).
    let mut originals: HashMap<DocumentId, Vec<u8>> = HashMap::new();

    // Wiped after decoding: an Open request may carry a password (MVP-16).
    while let Some(request) = frame::receive_wiped::<_, WorkerRequest>(&mut input)? {
        let response = match request {
            WorkerRequest::Open {
                request,
                doc,
                file,
                password,
            } => Some(open(
                &mut documents,
                &mut originals,
                request,
                doc,
                file,
                password,
            )),
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
            WorkerRequest::GetPageLinks {
                request,
                doc,
                page_index,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match page_links(document, page_index) {
                    Ok(links) => WorkerResponse::PageLinks {
                        request,
                        page_index,
                        links,
                    },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                },
            }),
            WorkerRequest::GetPageFields {
                request,
                doc,
                page_index,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match document.page_fields(page_index) {
                    Ok(fields) => WorkerResponse::PageFields {
                        request,
                        page_index,
                        fields,
                    },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                },
            }),
            WorkerRequest::GetPageAnnotations {
                request,
                doc,
                page_index,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match document.page_annotations(page_index) {
                    Ok(annotations) => WorkerResponse::PageAnnotations {
                        request,
                        page_index,
                        annotations,
                    },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                },
            }),
            WorkerRequest::VerifySignatures { request, doc } => {
                // Signatures cover the bytes of the file, not the document as it is edited.
                Some(match (documents.get(&doc), originals.get(&doc)) {
                    (Some(document), Some(file)) => match signatures::verify(document, file) {
                        Ok(report) => WorkerResponse::Signatures { request, report },
                        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                    },
                    _ => error(
                        request,
                        WorkerErrorCode::UnknownDocument,
                        "unknown document",
                    ),
                })
            }
            WorkerRequest::RenderPng {
                request,
                doc,
                page_index,
                scale,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match document.render_png(page_index, scale) {
                    Ok(png) => WorkerResponse::Png { request, png },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
                },
            }),
            WorkerRequest::RenderJpeg {
                request,
                doc,
                page_index,
                scale,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => match document.render_jpeg(page_index, scale) {
                    Ok(jpeg) => WorkerResponse::Jpeg { request, jpeg },
                    Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
                },
            }),
            WorkerRequest::GetPageText {
                request,
                doc,
                page_index,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => {
                    match document.page_text(page_index, MAX_PAGE_TEXT_CHARS as usize) {
                        Ok(text) => WorkerResponse::PageText {
                            request,
                            page_index,
                            text,
                        },
                        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                    }
                }
            }),
            WorkerRequest::SearchPage {
                request,
                doc,
                page_index,
                query,
                case_sensitive,
                max_hits,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => {
                    let max_hits = max_hits.min(MAX_SEARCH_HITS) as usize;
                    match document.search_page(page_index, &query, case_sensitive, max_hits) {
                        Ok(found) => WorkerResponse::PageSearched {
                            request,
                            page_index,
                            hits: found.hits,
                            has_text: found.has_text,
                        },
                        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
                    }
                }
            }),
            WorkerRequest::Edit { request, doc, edit } => Some(match documents.get_mut(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => {
                    match apply(document, &edit).and_then(|()| page_sizes(document)) {
                        Ok(pages) => WorkerResponse::Edited { request, pages },
                        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
                    }
                }
            }),
            WorkerRequest::Revert {
                request,
                doc,
                edits,
                password,
            } => Some(revert(
                &mut documents,
                &originals,
                request,
                doc,
                &edits,
                password.as_ref(),
            )),
            WorkerRequest::Rebase { request, doc, file } => Some(if documents.contains_key(&doc) {
                match read_limited(handle::take_file(file), MAX_DOCUMENT_BYTES) {
                    Ok(bytes) => {
                        originals.insert(doc, bytes);
                        WorkerResponse::Rebased { request }
                    }
                    Err(response) => response(request),
                }
            } else {
                error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                )
            }),
            WorkerRequest::Save { request, doc, file } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => save(document, request, file),
            }),
            WorkerRequest::PrivacyCopy {
                request,
                doc,
                file,
                id,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => privacy_copy(document, request, file, &id),
            }),
            WorkerRequest::PrepareSource {
                request,
                file,
                password,
            } => Some(prepare_source(request, file, password)),
            WorkerRequest::PrepareStampImage { request, file } => {
                Some(prepare_stamp_image(request, file))
            }
            WorkerRequest::SavePages {
                request,
                doc,
                pages,
                file,
            } => Some(match documents.get(&doc) {
                None => error(
                    request,
                    WorkerErrorCode::UnknownDocument,
                    "unknown document",
                ),
                Some(document) => save_pages(document, request, &pages, file),
            }),
            // Requests are handled one at a time, so there is nothing in flight to cancel.
            WorkerRequest::Cancel { .. } => None,
            WorkerRequest::Close { doc } => {
                documents.remove(&doc);
                originals.remove(&doc);
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

/// Opens a document; `password` (MVP-16) is wiped when this returns, whatever happened.
fn open(
    documents: &mut HashMap<DocumentId, PdfDocument>,
    originals: &mut HashMap<DocumentId, Vec<u8>>,
    request: RequestId,
    doc: DocumentId,
    file: FileHandle,
    password: Option<Password>,
) -> WorkerResponse {
    let bytes = match read_limited(handle::take_file(file), MAX_DOCUMENT_BYTES) {
        Ok(bytes) => bytes,
        Err(response) => return response(request),
    };
    let document = match PdfDocument::open(&bytes, password.as_ref().map(Password::as_str)) {
        Ok(document) => document,
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    };
    match document.page_count() {
        Ok(0) => return error(request, WorkerErrorCode::Corrupted, "document has no pages"),
        Ok(count) if count > MAX_PAGE_COUNT => {
            return error(request, WorkerErrorCode::LimitExceeded, "too many pages");
        }
        Ok(_) => {}
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    }
    let pages = match page_sizes(&document) {
        Ok(pages) => pages,
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    };
    let has_outline = document.has_outline();
    let has_form = document.has_form();
    let security = document.active_content(ScanBudget::default());
    let permissions = document.permissions();
    let encrypted = document.is_encrypted();
    documents.insert(doc, document);
    originals.insert(doc, bytes);
    WorkerResponse::Opened {
        request,
        document: OpenedDocument {
            pages,
            has_outline,
            has_form,
            security,
            permissions,
            encrypted,
        },
    }
}

/// The size of every page, as `Opened` and `Edited` report them.
fn page_sizes(document: &PdfDocument) -> Result<Vec<PageSize>, EngineError> {
    (0..document.page_count()?)
        .map(|index| {
            let (width_pt, height_pt) = document.page_size(index)?;
            Ok(PageSize {
                width_pt,
                height_pt,
            })
        })
        .collect()
}

/// Opens `doc` again from the bytes it was opened from (or last saved to) and applies `edits` in
/// order (undo, ADR 0013); a document opened with a password needs it again. The document is
/// replaced only once every edit is applied: until then, and if one fails, it stays as it was.
fn revert(
    documents: &mut HashMap<DocumentId, PdfDocument>,
    originals: &HashMap<DocumentId, Vec<u8>>,
    request: RequestId,
    doc: DocumentId,
    edits: &[WorkerEdit],
    password: Option<&Password>,
) -> WorkerResponse {
    if !documents.contains_key(&doc) {
        return error(
            request,
            WorkerErrorCode::UnknownDocument,
            "unknown document",
        );
    }
    let Some(bytes) = originals.get(&doc) else {
        return error(
            request,
            WorkerErrorCode::UnknownDocument,
            "unknown document",
        );
    };
    if edits.len() > MAX_UNDO_EDITS as usize {
        return error(request, WorkerErrorCode::LimitExceeded, "too many edits");
    }
    let mut document = match PdfDocument::open(bytes, password.map(Password::as_str)) {
        Ok(document) => document,
        Err(engine) => return engine_error(request, &engine, WorkerErrorCode::Corrupted),
    };
    for edit in edits {
        if let Err(engine) = apply(&mut document, edit) {
            return engine_error(request, &engine, WorkerErrorCode::Internal);
        }
    }
    match page_sizes(&document) {
        Ok(pages) => {
            documents.insert(doc, document);
            WorkerResponse::Edited { request, pages }
        }
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
    }
}

/// Applies an edit to the document in memory (ADR 0013).
fn apply(document: &mut PdfDocument, edit: &WorkerEdit) -> Result<(), EngineError> {
    match edit {
        WorkerEdit::RotatePages { pages, degrees } => document.rotate_pages(pages, *degrees)?,
        WorkerEdit::DeletePages { pages } => document.delete_pages(pages)?,
        WorkerEdit::MovePages { pages, before } => document.move_pages(pages, *before)?,
        WorkerEdit::InsertBlankPage { at, like } => document.insert_blank_page(*at, *like)?,
        WorkerEdit::InsertPages { at, source } => document.insert_pages(*at, source)?,
        WorkerEdit::AddHighlight { marks, color } => document.add_highlights(marks, *color)?,
        WorkerEdit::AddNote { page, at, text } => document.add_note(*page, *at, text)?,
        WorkerEdit::DeleteAnnotation { page, annotation } => {
            document.delete_annotation(*page, *annotation)?;
        }
        WorkerEdit::SetHighlightColor {
            page,
            annotation,
            color,
        } => document.set_highlight_color(*page, *annotation, *color)?,
        WorkerEdit::SetNoteText {
            page,
            annotation,
            text,
        } => document.set_note_text(*page, *annotation, text)?,
        WorkerEdit::SetFieldValue { page, field, value } => {
            document.set_field_value(*page, *field, value)?;
        }
        WorkerEdit::FlattenForm => document.flatten_form()?,
        WorkerEdit::AddInk {
            page,
            strokes,
            color,
            width,
        } => document.add_ink(*page, strokes, *color, *width)?,
        WorkerEdit::AddStamp { page, rect, stamp } => document.add_stamp(*page, *rect, *stamp)?,
        WorkerEdit::SetAnnotationRect {
            page,
            annotation,
            rect,
        } => document.set_annotation_rect(*page, *annotation, *rect)?,
        WorkerEdit::AddImageStamp { page, rect, png } => {
            document.add_image_stamp(*page, *rect, png)?;
        }
    }
    Ok(())
}

/// Makes the picture behind the read-only handle `file` into what a stamp is made of (B2-08).
fn prepare_stamp_image(request: RequestId, file: FileHandle) -> WorkerResponse {
    let bytes = match read_limited(handle::take_file(file), MAX_STAMP_SOURCE_BYTES as u64) {
        Ok(bytes) => bytes,
        Err(response) => return response(request),
    };
    match prepare_stamp_picture(&bytes) {
        Ok(picture) => WorkerResponse::StampImage {
            request,
            png: picture.png,
            width: picture.width,
            height: picture.height,
        },
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
    }
}

/// Writes the document to the write-only handle `file` (ADR 0013). The handle is closed when
/// this returns; the main process checks the file before it replaces anything with it.
fn save(document: &PdfDocument, request: RequestId, file: FileHandle) -> WorkerResponse {
    let Some(mut file) = handle::take_file(file) else {
        return error(
            request,
            WorkerErrorCode::InvalidRequest,
            "invalid file handle",
        );
    };
    match document.save(&mut file) {
        Ok((bytes, incremental)) => WorkerResponse::Saved {
            request,
            bytes,
            incremental,
        },
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
    }
}

/// Writes the privacy export of `document` (B2-03) to `file`, a handle like `save`'s.
fn privacy_copy(
    document: &PdfDocument,
    request: RequestId,
    file: FileHandle,
    id: &[u8; 16],
) -> WorkerResponse {
    let Some(mut file) = handle::take_file(file) else {
        return error(
            request,
            WorkerErrorCode::InvalidRequest,
            "invalid file handle",
        );
    };
    match document.privacy_copy(id, &mut file) {
        Ok(bytes) => WorkerResponse::Saved {
            request,
            bytes,
            incremental: false,
        },
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
    }
}

/// Writes the pages `pages` of `document` as a document of their own (B2-06) to `file`, a handle
/// like `save`'s.
fn save_pages(
    document: &PdfDocument,
    request: RequestId,
    pages: &[u32],
    file: FileHandle,
) -> WorkerResponse {
    let Some(mut file) = handle::take_file(file) else {
        return error(
            request,
            WorkerErrorCode::InvalidRequest,
            "invalid file handle",
        );
    };
    match document.pages_copy(pages, &mut file) {
        Ok(bytes) => WorkerResponse::Saved {
            request,
            bytes,
            incremental: false,
        },
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Internal),
    }
}

/// Makes the plain copy of the PDF in `file`, whose pages are to go into a document (B2-06);
/// `password` is wiped when this returns.
fn prepare_source(
    request: RequestId,
    file: FileHandle,
    password: Option<Password>,
) -> WorkerResponse {
    let bytes = match read_limited(handle::take_file(file), MAX_SOURCE_BYTES as u64) {
        Ok(bytes) => bytes,
        Err(response) => return response(request),
    };
    match PdfDocument::prepare_source(&bytes, password.as_ref().map(Password::as_str)) {
        Ok(source) => WorkerResponse::Source {
            request,
            bytes: source.bytes,
            pages: source.pages,
            security: source.security,
        },
        Err(engine) => engine_error(request, &engine, WorkerErrorCode::Corrupted),
    }
}

type ErrorFor = fn(RequestId) -> WorkerResponse;

fn read_limited(file: Option<File>, limit: u64) -> Result<Vec<u8>, ErrorFor> {
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
    if file.take(limit + 1).read_to_end(&mut bytes).is_err() {
        return Err(|request| {
            error(
                request,
                WorkerErrorCode::Unreadable,
                "could not read the file",
            )
        });
    }
    if bytes.len() as u64 > limit {
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
            target: contract_target(entry.target, page_count),
        })
        .collect();
    Ok(OutlineResult {
        items,
        truncated: outline.truncated,
    })
}

/// The links of a page. A link that points nowhere (a missing page, for example) is left out:
/// there is nothing to click.
fn page_links(document: &PdfDocument, page_index: u32) -> Result<Vec<PageLink>, EngineError> {
    let page_count = document.page_count()?;
    let entries = document.page_links(page_index, MAX_LINKS_PER_PAGE as usize)?;
    let links = entries
        .into_iter()
        .filter_map(|entry| {
            let [x0, y0, x1, y1] = entry.rect;
            let target = contract_target(entry.target, page_count)?;
            Some((Rect { x0, y0, x1, y1 }, target))
        })
        .enumerate()
        .map(|(index, (rect, target))| PageLink {
            id: LinkId {
                page_index,
                index: u32::try_from(index).unwrap_or(u32::MAX),
            },
            rect,
            target,
        })
        .collect();
    Ok(links)
}

/// An outline or link target as the frontend sees it: pages within the document, URIs sorted
/// into openable and blocked ones, and PDF-provided text cleaned for display.
fn contract_target(target: OutlineTarget, page_count: u32) -> Option<LinkTarget> {
    match target {
        OutlineTarget::Page(page_index) if page_index < page_count => Some(LinkTarget::Page {
            page_index,
            x: None,
            y: None,
        }),
        OutlineTarget::Page(_) | OutlineTarget::None => None,
        OutlineTarget::Uri(uri) => Some(classify_uri(&uri)),
        OutlineTarget::Blocked { action, target } => Some(LinkTarget::Blocked {
            action,
            target: target
                .map(|text| clean_display_text(&text, MAX_TEXT_BYTES as usize))
                .filter(|text| !text.is_empty()),
        }),
    }
}

fn engine_error(
    request: RequestId,
    engine: &EngineError,
    other: WorkerErrorCode,
) -> WorkerResponse {
    let code = match engine {
        EngineError::NotPdf => WorkerErrorCode::NotPdf,
        EngineError::Encrypted => WorkerErrorCode::Encrypted,
        EngineError::WrongPassword => WorkerErrorCode::WrongPassword,
        EngineError::UnsupportedEncryption => WorkerErrorCode::UnsupportedEncryption,
        EngineError::PageOutOfRange(_) => WorkerErrorCode::PageOutOfRange,
        EngineError::NotAllowed(_) => WorkerErrorCode::NotAllowed,
        EngineError::InvalidScale
        | EngineError::InvalidRotation
        | EngineError::InvalidEdit(_)
        | EngineError::InvalidPicture(_)
        | EngineError::NoPageLeft
        | EngineError::EncryptedCopy => WorkerErrorCode::InvalidRequest,
        EngineError::TooLarge { .. }
        | EngineError::TooComplex
        | EngineError::TooManyPages
        | EngineError::PictureTooLarge => WorkerErrorCode::LimitExceeded,
        EngineError::Write(error) => match error.kind() {
            std::io::ErrorKind::StorageFull => WorkerErrorCode::DiskFull,
            std::io::ErrorKind::FileTooLarge => WorkerErrorCode::LimitExceeded,
            _ => WorkerErrorCode::Unwritable,
        },
        EngineError::MuPdf(_) | EngineError::Encode(_) => other,
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
