//! Attachments: files embedded in a document (B3-20, ADR 0025).
//!
//! A PDF carries them at the document level (/Names /EmbeddedFiles) and in file attachment
//! annotations on its pages (/Subtype /FileAttachment). The app never opens them: it lists them,
//! and writes one, decoded, to a handle the main process gives.

use std::collections::HashSet;
use std::io::Write;

use ipc_contract::attachments::clean_attachment_name;
use ipc_contract::limits::{MAX_ATTACHMENT_DESC_BYTES, MAX_ATTACHMENTS, MAX_DOCUMENT_BYTES};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{AttachmentId, AttachmentSource};
use ipc_contract::worker::WorkerAttachment;
use mupdf::pdf::{PdfAnnotationType, PdfObject};

use super::{EngineError, PdfDocument, text};

/// Deepest name tree walked: a real one is a few levels deep; a made-up one could be deep enough
/// to overflow the stack.
const MAX_NAME_TREE_DEPTH: usize = 32;

/// An attachment found in the document: what the list says, and its embedded file stream.
struct Found {
    listed: WorkerAttachment,
    stream: PdfObject,
}

impl PdfDocument {
    /// The attachments of the document (ADR 0025): those of /Names /EmbeddedFiles first, then
    /// those of the pages' file attachment annotations, at most `MAX_ATTACHMENTS`. An attachment
    /// is known by the object number of its embedded file stream, so one attached twice (for
    /// example in the names and on a page) is listed once.
    pub fn list_attachments(&self) -> Vec<WorkerAttachment> {
        self.attachments()
            .into_iter()
            .map(|found| found.listed)
            .collect()
    }

    /// Writes attachment `id`, decoded, to `out` and returns its length; at most
    /// `MAX_DOCUMENT_BYTES` (ADR 0025). Only an attachment the list has can be written: `id` is
    /// not a way to read any other object of the file.
    ///
    /// MuPDF decodes the whole stream into memory first. It stops a compression bomb itself
    /// (`fz_read_best`: past about 600 times the stream's length, and at least 100 MiB), and the
    /// worker's memory limit stops the rest; what decodes to more than the limit is refused.
    pub fn write_attachment(
        &self,
        id: AttachmentId,
        out: &mut impl Write,
    ) -> Result<u64, EngineError> {
        let found = self
            .attachments()
            .into_iter()
            .find(|found| found.listed.id == id)
            .ok_or(EngineError::InvalidEdit("no such attachment"))?;
        let decoded = found.stream.read_stream()?;
        let length = decoded.len() as u64;
        if length > MAX_DOCUMENT_BYTES {
            return Err(EngineError::OutputTooLarge);
        }
        out.write_all(&decoded).map_err(EngineError::Write)?;
        out.flush().map_err(EngineError::Write)?;
        Ok(length)
    }

    fn attachments(&self) -> Vec<Found> {
        let limit = MAX_ATTACHMENTS as usize;
        let mut found = Vec::new();
        let mut streams = HashSet::new();
        let mut add = |filespec: &PdfObject,
                       source: AttachmentSource,
                       key: Option<String>,
                       found: &mut Vec<Found>| {
            if found.len() < limit
                && let Some(item) = attachment(filespec, source, key, found.len() + 1, &mut streams)
            {
                found.push(item);
            }
        };

        let names = self
            .doc
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get_dict("Names").ok().flatten())
            .and_then(|names| names.get_dict("EmbeddedFiles").ok().flatten());
        if let Some(tree) = names {
            let mut entries = Vec::new();
            walk_name_tree(&tree, 0, &mut HashSet::new(), &mut entries, limit);
            for (key, filespec) in entries {
                add(&filespec, AttachmentSource::Document, key, &mut found);
            }
        }

        let pages = self.page_count().unwrap_or(0);
        for page in 0..pages {
            if found.len() >= limit {
                break;
            }
            let Ok(pdf_page) = self.pdf_page(page) else {
                continue;
            };
            for annotation in pdf_page.annotations() {
                if annotation.r#type().ok() != Some(PdfAnnotationType::FileAttachment) {
                    continue;
                }
                let Ok(Some(filespec)) = annotation.object().get_dict("FS") else {
                    continue;
                };
                let before = found.len();
                add(&filespec, AttachmentSource::Page { page }, None, &mut found);
                // Without a description of its own, the annotation's text says what it is.
                if found.len() > before
                    && let Some(item) = found.last_mut()
                    && item.listed.description.is_none()
                {
                    item.listed.description = annotation
                        .contents()
                        .ok()
                        .flatten()
                        .map(|contents| {
                            clean_display_text(contents, MAX_ATTACHMENT_DESC_BYTES as usize)
                        })
                        .filter(|contents| !contents.is_empty());
                }
            }
        }
        found
    }
}

/// Collects the (key, value) pairs of a name tree, at most `limit`, ignoring a node met twice.
fn walk_name_tree(
    node: &PdfObject,
    depth: usize,
    visited: &mut HashSet<i32>,
    entries: &mut Vec<(Option<String>, PdfObject)>,
    limit: usize,
) {
    if entries.len() >= limit || depth > MAX_NAME_TREE_DEPTH {
        return;
    }
    if let Ok(number) = node.as_indirect()
        && number > 0
        && !visited.insert(number)
    {
        return;
    }
    if let Ok(Some(kids)) = node.get_dict("Kids") {
        let Ok(kids) = kids.array_iter() else {
            return;
        };
        for kid in kids.flatten() {
            if entries.len() >= limit {
                return;
            }
            walk_name_tree(&kid, depth + 1, visited, entries, limit);
        }
    } else if let Ok(Some(names)) = node.get_dict("Names") {
        let Ok(names) = names.array_iter() else {
            return;
        };
        let mut key = None;
        for item in names.flatten() {
            if entries.len() >= limit {
                return;
            }
            match key.take() {
                None => key = Some(text(Ok(Some(item)))),
                Some(name) => entries.push((name, item)),
            }
        }
    }
}

/// The attachment a file specification embeds, if it embeds one not already found: its stream
/// must be an indirect object (as PDF requires), which then identifies it.
fn attachment(
    filespec: &PdfObject,
    source: AttachmentSource,
    key: Option<String>,
    index: usize,
    streams: &mut HashSet<i32>,
) -> Option<Found> {
    if !filespec.is_dict().ok()? {
        return None;
    }
    let files = filespec.get_dict("EF").ok()??;
    let stream = ["UF", "F", "DOS", "Mac", "Unix"]
        .into_iter()
        .find_map(|key| files.get_dict(key).ok().flatten())?;
    if !stream.is_stream().ok()? {
        return None;
    }
    let number = stream.as_indirect().ok().filter(|&number| number > 0)?;
    if !streams.insert(number) {
        return None;
    }

    let raw_name = ["UF", "F", "DOS", "Mac", "Unix"]
        .into_iter()
        .find_map(|key| text(filespec.get_dict(key)))
        .or(key)
        .unwrap_or_default();
    let size = stream
        .get_dict("Params")
        .ok()
        .flatten()
        .and_then(|params| params.get_dict("Size").ok().flatten())
        .and_then(|size| size.as_int().ok())
        .and_then(|size| u64::try_from(size).ok());
    let description = text(filespec.get_dict("Desc"))
        .map(|description| clean_display_text(&description, MAX_ATTACHMENT_DESC_BYTES as usize))
        .filter(|description| !description.is_empty());

    Some(Found {
        listed: WorkerAttachment {
            id: AttachmentId(number.unsigned_abs()),
            name: clean_attachment_name(&raw_name, index),
            size,
            description,
            source,
        },
        stream,
    })
}
