//! Embedded file attachments (B3-20, ADR 0025).
//!
//! PDF files can carry attachments at document level (/Names /EmbeddedFiles) and as page
//! annotations (/Subtype /FileAttachment). Attachments are never opened or executed by
//! the app; they can only be listed and saved to disk with Mark of the Web.

use std::collections::HashSet;
use std::io::Write;

use ipc_contract::attachments::clean_attachment_name;
use ipc_contract::limits::{MAX_ATTACHMENTS, MAX_ATTACHMENT_DESC_BYTES, MAX_DOCUMENT_BYTES};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{AttachmentId, AttachmentSource};
use ipc_contract::worker::WorkerAttachment;
use mupdf::pdf::{PdfAnnotationType, PdfObject};

use super::{EngineError, PdfDocument, text};

impl PdfDocument {
    /// Lists the attachments of the document (ADR 0025): from /Names /EmbeddedFiles and
    /// page annotations (/Subtype /FileAttachment). At most `MAX_ATTACHMENTS` (1,000).
    pub fn list_attachments(&self) -> Result<Vec<WorkerAttachment>, EngineError> {
        let mut attachments = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut counter = 1usize;

        // 1. Document-level attachments: /Root /Names /EmbeddedFiles
        if let Ok(catalog) = self.doc.catalog() {
            if let Ok(Some(names)) = catalog.get_dict("Names") {
                if let Ok(Some(embedded_files)) = names.get_dict("EmbeddedFiles") {
                    let mut visited = HashSet::new();
                    let mut entries = Vec::new();
                    let _ = walk_name_tree(&embedded_files, &mut visited, &mut entries, MAX_ATTACHMENTS as usize);
                    for (key, filespec) in entries {
                        if attachments.len() >= MAX_ATTACHMENTS as usize {
                            break;
                        }
                        if let Ok(Some(item)) = parse_filespec(
                            &filespec,
                            AttachmentSource::Document,
                            counter,
                            key,
                            &mut seen_ids,
                        ) {
                            attachments.push(item);
                            counter += 1;
                        }
                    }
                }
            }
        }

        // 2. Page-level attachments: /Subtype /FileAttachment annotations
        let page_count = self.page_count().unwrap_or(0);
        for page_idx in 0..page_count {
            if attachments.len() >= MAX_ATTACHMENTS as usize {
                break;
            }
            let Ok(page) = self.pdf_page(page_idx) else {
                continue;
            };
            for annot in page.annotations() {
                if attachments.len() >= MAX_ATTACHMENTS as usize {
                    break;
                }
                if annot.r#type().ok() != Some(PdfAnnotationType::FileAttachment) {
                    continue;
                }
                let annot_obj = annot.object();
                let Ok(Some(fs)) = annot_obj.get_dict("FS") else {
                    continue;
                };
                let extra_desc = annot.contents().ok().flatten();
                if let Ok(Some(mut item)) = parse_filespec(
                    &fs,
                    AttachmentSource::Page { page: page_idx },
                    counter,
                    None,
                    &mut seen_ids,
                ) {
                    if item.description.is_none() && extra_desc.is_some() {
                        item.description = extra_desc
                            .map(|desc| clean_display_text(&desc, MAX_ATTACHMENT_DESC_BYTES as usize));
                    }
                    attachments.push(item);
                    counter += 1;
                }
            }
        }

        Ok(attachments)
    }

    /// Decodes attachment `id` and writes its uncompressed bytes to `out` (ADR 0025).
    /// Enforces the 512 MiB limit (`MAX_DOCUMENT_BYTES`).
    pub fn write_attachment(
        &self,
        id: AttachmentId,
        out: &mut impl Write,
    ) -> Result<u64, EngineError> {
        let Ok(xref) = i32::try_from(id.0) else {
            return Err(EngineError::InvalidEdit("invalid attachment id"));
        };
        let obj = self.doc.new_indirect(xref, 0)?;
        let stream = if obj.is_stream()? {
            obj
        } else if obj.is_dict()? {
            let ef = obj.get_dict("EF")?.ok_or(EngineError::InvalidEdit("filespec has no EF"))?;
            let stream = ef
                .get_dict("UF")?
                .or(ef.get_dict("F")?)
                .or(ef.get_dict("DOS")?)
                .or(ef.get_dict("Mac")?)
                .or(ef.get_dict("Unix")?)
                .ok_or(EngineError::InvalidEdit("EF has no stream"))?;
            if !stream.is_stream()? {
                return Err(EngineError::InvalidEdit("EF target is not a stream"));
            }
            stream
        } else {
            return Err(EngineError::InvalidEdit("attachment is neither stream nor filespec"));
        };

        let decoded = stream.read_stream()?;
        if decoded.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(EngineError::OutputTooLarge);
        }

        out.write_all(&decoded).map_err(EngineError::Write)?;
        out.flush().map_err(EngineError::Write)?;
        Ok(decoded.len() as u64)
    }
}

/// Recursively walks a PDF NameTree for /EmbeddedFiles, with cycle detection.
fn walk_name_tree(
    node: &PdfObject,
    visited: &mut HashSet<i32>,
    entries: &mut Vec<(Option<String>, PdfObject)>,
    limit: usize,
) -> Result<(), mupdf::Error> {
    if entries.len() >= limit {
        return Ok(());
    }
    if let Ok(xref) = node.as_indirect() {
        if xref > 0 && !visited.insert(xref) {
            return Ok(()); // cycle detected
        }
    }
    if let Some(kids) = node.get_dict("Kids")? {
        if kids.is_array()? {
            for kid in kids.array_iter()? {
                if entries.len() >= limit {
                    break;
                }
                walk_name_tree(&kid?, visited, entries, limit)?;
            }
        }
    } else if let Some(names) = node.get_dict("Names")? {
        if names.is_array()? {
            let mut key = None;
            for item in names.array_iter()? {
                let item = item?;
                if entries.len() >= limit {
                    break;
                }
                if key.is_none() {
                    key = text(Ok(Some(item)));
                } else {
                    entries.push((key.take(), item));
                }
            }
        }
    }
    Ok(())
}

/// Parses a filespec dictionary into a [`WorkerAttachment`].
fn parse_filespec(
    filespec: &PdfObject,
    source: AttachmentSource,
    counter: usize,
    key_name: Option<String>,
    seen_ids: &mut HashSet<u32>,
) -> Result<Option<WorkerAttachment>, mupdf::Error> {
    if !filespec.is_dict()? {
        return Ok(None);
    }
    let Some(ef) = filespec.get_dict("EF")? else {
        return Ok(None);
    };
    if !ef.is_dict()? {
        return Ok(None);
    }
    let stream = ef
        .get_dict("UF")?
        .or(ef.get_dict("F")?)
        .or(ef.get_dict("DOS")?)
        .or(ef.get_dict("Mac")?)
        .or(ef.get_dict("Unix")?);
    let Some(stream) = stream else {
        return Ok(None);
    };
    if !stream.is_stream()? {
        return Ok(None);
    }

    let stream_xref = stream.as_indirect().unwrap_or(0);
    let filespec_xref = filespec.as_indirect().unwrap_or(0);

    let id_num = if stream_xref > 0 && !seen_ids.contains(&(stream_xref as u32)) {
        stream_xref as u32
    } else if filespec_xref > 0 && !seen_ids.contains(&(filespec_xref as u32)) {
        filespec_xref as u32
    } else {
        // Fallback to synthetic unique id
        let mut synth = (counter as u32) + 100_000;
        while seen_ids.contains(&synth) {
            synth += 1;
        }
        synth
    };
    seen_ids.insert(id_num);

    let raw_name = text(filespec.get_dict("UF"))
        .or_else(|| text(filespec.get_dict("F")))
        .or_else(|| text(filespec.get_dict("DOS")))
        .or_else(|| text(filespec.get_dict("Mac")))
        .or_else(|| text(filespec.get_dict("Unix")))
        .or(key_name)
        .unwrap_or_default();

    let clean_name = clean_attachment_name(&raw_name, counter);

    let size = stream.get_dict("Params")?.and_then(|params| {
        params.get_dict("Size").ok().flatten().and_then(|s| {
            s.as_int()
                .ok()
                .and_then(|val| if val >= 0 { Some(val as u64) } else { None })
        })
    });

    let description = text(filespec.get_dict("Desc")).map(|desc| {
        clean_display_text(&desc, MAX_ATTACHMENT_DESC_BYTES as usize)
    });

    Ok(Some(WorkerAttachment {
        id: AttachmentId(id_num),
        name: clean_name,
        size,
        description,
        source,
    }))
}
