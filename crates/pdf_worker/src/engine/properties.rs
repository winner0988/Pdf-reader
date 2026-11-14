//! Document properties and font discovery (B5-04).
//!
//! Provides `document_properties` (metadata, format, encryption, signature and attachment counts)
//! and `document_fonts` (traversing pages and Form XObjects for fonts with strict resource bounds).

use std::collections::HashSet;
use std::time::Instant;

use ipc_contract::limits::{
    MAX_FONT_ENTRIES, MAX_FONT_RESOURCE_DEPTH, MAX_FONT_RESOURCE_OBJECTS, MAX_FONT_TIME_MILLIS,
    MAX_PAGE_PARENT_DEPTH, MAX_PROPERTY_TEXT_BYTES,
};
use ipc_contract::text::clean_display_text;
use ipc_contract::types::{DocumentFontEntry, DocumentFonts, PdfDate};
use ipc_contract::worker::WorkerDocumentProperties;
use mupdf::pdf::PdfObject;

use super::{EngineError, PdfDocument};

fn is_font_embedded(font_val: &PdfObject, subtype: &str) -> bool {
    if subtype == "Type 3" || subtype == "Type3" {
        return true;
    }
    if subtype == "Type 0" || subtype == "Type0" {
        if let Ok(Some(descendants)) = font_val.get_dict("DescendantFonts") {
            if let Ok(Some(first)) = descendants.get_array(0) {
                if let Ok(Some(descriptor)) = first.get_dict("FontDescriptor") {
                    return has_font_file(&descriptor);
                }
            }
        }
        return false;
    }
    if let Ok(Some(descriptor)) = font_val.get_dict("FontDescriptor") {
        return has_font_file(&descriptor);
    }
    false
}

fn has_font_file(descriptor: &PdfObject) -> bool {
    descriptor.get_dict("FontFile").ok().flatten().is_some()
        || descriptor.get_dict("FontFile2").ok().flatten().is_some()
        || descriptor.get_dict("FontFile3").ok().flatten().is_some()
}

fn extract_subtype(font_val: &PdfObject) -> String {
    if let Ok(Some(st)) = font_val.get_dict("Subtype") {
        if let Ok(bytes) = st.as_name() {
            let name = String::from_utf8_lossy(&bytes);
            let trimmed = name.trim_start_matches('/');
            match trimmed {
                "Type1" => return "Type 1".to_string(),
                "Type0" => return "Type 0".to_string(),
                "Type3" => return "Type 3".to_string(),
                "MMType1" => return "MMType 1".to_string(),
                "TrueType" => return "TrueType".to_string(),
                "CIDFontType0" => return "CIDFontType0".to_string(),
                "CIDFontType2" => return "CIDFontType2".to_string(),
                other => return clean_display_text(other, MAX_PROPERTY_TEXT_BYTES),
            }
        }
    }
    "Unknown".to_string()
}

fn extract_font_name(font_val: &PdfObject) -> String {
    let raw = if let Ok(Some(bf)) = font_val.get_dict("BaseFont") {
        bf.as_name()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .or_else(|_| bf.as_string())
            .ok()
    } else if let Ok(Some(nm)) = font_val.get_dict("Name") {
        nm.as_name()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .or_else(|_| nm.as_string())
            .ok()
    } else {
        None
    };

    let Some(raw_str) = raw else {
        return "Unnamed".to_string();
    };

    let trimmed = raw_str.trim_start_matches('/');
    // Strip subset prefix if it matches [A-Z]{6}\+
    let without_prefix = if trimmed.len() > 7
        && trimmed.as_bytes()[6] == b'+'
        && trimmed.as_bytes()[0..6].iter().all(|b| b.is_ascii_uppercase())
    {
        &trimmed[7..]
    } else {
        trimmed
    };

    let cleaned = clean_display_text(without_prefix, MAX_PROPERTY_TEXT_BYTES);
    if cleaned.is_empty() {
        "Unnamed".to_string()
    } else {
        cleaned
    }
}

fn get_page_resources(
    page_obj: &PdfObject,
    complete: &mut bool,
) -> Result<Option<PdfObject>, EngineError> {
    if let Some(res) = page_obj.get_dict("Resources")? {
        return Ok(Some(res));
    }
    let mut current = page_obj.get_dict("Parent")?;
    let mut depth = 0;
    let mut seen_parents = HashSet::new();

    while let Some(parent) = current {
        if depth >= MAX_PAGE_PARENT_DEPTH {
            *complete = false;
            break;
        }
        depth += 1;
        if let Ok(id) = parent.as_indirect() {
            if id > 0 && !seen_parents.insert(id) {
                *complete = false;
                break;
            }
        }
        if let Some(res) = parent.get_dict("Resources")? {
            return Ok(Some(res));
        }
        current = parent.get_dict("Parent")?;
    }
    Ok(None)
}

fn traverse_resources(
    res_obj: &PdfObject,
    depth: usize,
    complete: &mut bool,
    seen_resources: &mut HashSet<i32>,
    seen_xobjects: &mut HashSet<i32>,
    seen_fonts: &mut HashSet<i32>,
    object_count: &mut usize,
    raw_fonts: &mut Vec<DocumentFontEntry>,
    start_time: Instant,
) -> Result<(), EngineError> {
    if depth > MAX_FONT_RESOURCE_DEPTH {
        *complete = false;
        return Ok(());
    }
    if *object_count >= MAX_FONT_RESOURCE_OBJECTS
        || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
        || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
    {
        *complete = false;
        return Ok(());
    }

    if let Ok(id) = res_obj.as_indirect() {
        if id > 0 && !seen_resources.insert(id) {
            return Ok(());
        }
    }
    *object_count += 1;

    // 1. Process /Font dictionary
    if let Some(font_dict) = res_obj.get_dict("Font")? {
        if *object_count >= MAX_FONT_RESOURCE_OBJECTS
            || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
            || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
        {
            *complete = false;
            return Ok(());
        }
        *object_count += 1;

        if font_dict.is_dict().unwrap_or(false) {
            if let Ok(iter) = font_dict.dict_iter() {
                for item in iter {
                    if *object_count >= MAX_FONT_RESOURCE_OBJECTS
                        || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
                        || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
                    {
                        *complete = false;
                        return Ok(());
                    }
                    *object_count += 1;

                    let Ok((_key, font_val)) = item else {
                        continue;
                    };

                    if let Ok(id) = font_val.as_indirect() {
                        if id > 0 && !seen_fonts.insert(id) {
                            continue;
                        }
                    }

                    let subtype = extract_subtype(&font_val);
                    let name = extract_font_name(&font_val);
                    let embedded = is_font_embedded(&font_val, &subtype);

                    raw_fonts.push(DocumentFontEntry {
                        name,
                        subtype,
                        embedded,
                    });
                }
            }
        }
    }

    // 2. Process /XObject dictionary
    if let Some(xobj_dict) = res_obj.get_dict("XObject")? {
        if *object_count >= MAX_FONT_RESOURCE_OBJECTS
            || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
            || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
        {
            *complete = false;
            return Ok(());
        }
        *object_count += 1;

        if xobj_dict.is_dict().unwrap_or(false) {
            if let Ok(iter) = xobj_dict.dict_iter() {
                for item in iter {
                    if *object_count >= MAX_FONT_RESOURCE_OBJECTS
                        || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
                        || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
                    {
                        *complete = false;
                        return Ok(());
                    }
                    *object_count += 1;

                    let Ok((_key, xobj_val)) = item else {
                        continue;
                    };

                    let is_form = if let Ok(Some(st)) = xobj_val.get_dict("Subtype") {
                        st.as_name().map(|n| n == b"Form").unwrap_or(false)
                    } else {
                        false
                    };

                    if !is_form {
                        continue;
                    }

                    if let Ok(id) = xobj_val.as_indirect() {
                        if id > 0 && !seen_xobjects.insert(id) {
                            continue;
                        }
                    }

                    if let Some(x_res) = xobj_val.get_dict("Resources")? {
                        traverse_resources(
                            &x_res,
                            depth + 1,
                            complete,
                            seen_resources,
                            seen_xobjects,
                            seen_fonts,
                            object_count,
                            raw_fonts,
                            start_time,
                        )?;
                    }
                }
            }
        }
    }

    Ok(())
}

impl PdfDocument {
    /// Read basic metadata, encryption, signatures count, and attachment count (B5-04).
    pub fn document_properties(&self) -> Result<WorkerDocumentProperties, EngineError> {
        let get_text = |meta: mupdf::MetadataName| -> Option<String> {
            match self.doc.metadata(meta) {
                Ok(s) => {
                    let cleaned = clean_display_text(&s, MAX_PROPERTY_TEXT_BYTES);
                    if cleaned.is_empty() {
                        None
                    } else {
                        Some(cleaned)
                    }
                }
                Err(_) => None,
            }
        };

        let format = get_text(mupdf::MetadataName::Format);
        let title = get_text(mupdf::MetadataName::Title);
        let author = get_text(mupdf::MetadataName::Author);
        let subject = get_text(mupdf::MetadataName::Subject);
        let keywords = get_text(mupdf::MetadataName::Keywords);
        let creator = get_text(mupdf::MetadataName::Creator);
        let producer = get_text(mupdf::MetadataName::Producer);

        let creation_date = self
            .doc
            .metadata(mupdf::MetadataName::CreationDate)
            .ok()
            .and_then(|raw| PdfDate::parse(&raw));

        let mod_date = self
            .doc
            .metadata(mupdf::MetadataName::ModDate)
            .ok()
            .and_then(|raw| PdfDate::parse(&raw));

        let has_xmp = self
            .doc
            .catalog()
            .ok()
            .and_then(|cat| cat.get_dict("Metadata").ok().flatten())
            .is_some();

        let encryption_method = match self.doc.metadata(mupdf::MetadataName::Encryption) {
            Ok(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                    None
                } else {
                    let cleaned = clean_display_text(trimmed, MAX_PROPERTY_TEXT_BYTES);
                    if cleaned.is_empty() {
                        None
                    } else {
                        Some(cleaned)
                    }
                }
            }
            Err(_) => None,
        };

        let sigs = self.signature_fields().unwrap_or_default();
        let signatures_count = sigs.fields.len() as u32;
        let signatures_truncated = sigs.truncated;

        let attachment_count = self.list_attachments().len() as u32;

        Ok(WorkerDocumentProperties {
            format,
            title,
            author,
            subject,
            keywords,
            creator,
            producer,
            creation_date,
            mod_date,
            has_xmp,
            encryption_method,
            signatures_count,
            signatures_truncated,
            attachment_count,
        })
    }

    /// Traverses pages and Form XObjects for fonts with strict resource bounds (B5-04).
    pub fn document_fonts(&self) -> Result<DocumentFonts, EngineError> {
        let start_time = Instant::now();
        let mut complete = true;
        let mut seen_resources = HashSet::new();
        let mut seen_xobjects = HashSet::new();
        let mut seen_fonts = HashSet::new();
        let mut object_count = 0;
        let mut raw_fonts = Vec::new();

        let page_count = self.page_count()?;
        for i in 0..page_count {
            if object_count >= MAX_FONT_RESOURCE_OBJECTS
                || raw_fonts.len() >= MAX_FONT_ENTRIES as usize
                || start_time.elapsed().as_millis() >= MAX_FONT_TIME_MILLIS as u128
            {
                complete = false;
                break;
            }

            let Ok(page_obj) = self.doc.find_page(i as i32) else {
                continue;
            };

            let Ok(Some(res)) = get_page_resources(&page_obj, &mut complete) else {
                continue;
            };

            traverse_resources(
                &res,
                0,
                &mut complete,
                &mut seen_resources,
                &mut seen_xobjects,
                &mut seen_fonts,
                &mut object_count,
                &mut raw_fonts,
                start_time,
            )?;
        }

        let mut fonts = raw_fonts;
        fonts.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| b.embedded.cmp(&a.embedded))
                .then_with(|| a.subtype.cmp(&b.subtype))
        });
        fonts.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));

        if fonts.len() > MAX_FONT_ENTRIES as usize {
            fonts.truncate(MAX_FONT_ENTRIES as usize);
            complete = false;
        }

        Ok(DocumentFonts { fonts, complete })
    }
}
