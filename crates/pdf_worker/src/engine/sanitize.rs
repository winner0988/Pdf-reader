//! Sanitizing a copy before sharing (B3-15).
//!
//! Produces a clean copy without active content (JavaScript, dangerous actions, external links),
//! attachments, form scripts, XFA, rich media, hidden metadata/thumbnails, or annotations
//! according to the user's choice.
//! The original document is not modified.

use std::collections::HashSet;

use ipc_contract::limits::MAX_URI_BYTES;
use ipc_contract::text::is_network_path;
use ipc_contract::types::{
    FindingKind, SanitizeAnnotations, SanitizeOptions, SanitizeReport, SecurityReport,
};
use mupdf::pdf::{PdfDocument, PdfObject};

use super::EngineError;
use crate::privacy::{self, name_of};

/// Action types considered active or dangerous that must be removed.
const DANGEROUS_ACTION_TYPES: [&[u8]; 9] = [
    b"JavaScript",
    b"Launch",
    b"SubmitForm",
    b"ImportData",
    b"GoToR",
    b"GoToE",
    b"ResetForm",
    b"Rendition",
    b"RichMediaExecute",
];

/// Media annotation subtypes.
const MEDIA_SUBTYPES: [&[u8]; 5] = [b"RichMedia", b"Screen", b"Movie", b"Sound", b"3D"];

/// Sanitizes `doc` in place according to `options`, replacing trailer `/ID` with `id`
/// if `options.hidden_data` is set.
pub fn sanitize(
    doc: &mut PdfDocument,
    id: &[u8; 16],
    options: SanitizeOptions,
) -> Result<SanitizeReport, EngineError> {
    let mut report = SanitizeReport::default();

    // Step 1: Trailer & Catalog & Names & AcroForm
    clean_trailer_and_catalog(doc, id, options, &mut report)?;

    // Step 2: Bookmarks / Outlines
    clean_outlines(doc, options, &mut report)?;

    // Step 3: Page annotations and links
    clean_pages(doc, options, &mut report)?;

    // Step 4: Annotation flattening (if requested)
    if options.annotations == SanitizeAnnotations::Flatten {
        flatten_annotations(doc, &mut report)?;
    }

    // Step 5: Walk all objects for hidden data, external streams, lingering active content
    clean_all_objects(doc, options, &mut report)?;

    Ok(report)
}

fn clean_trailer_and_catalog(
    doc: &PdfDocument,
    id: &[u8; 16],
    options: SanitizeOptions,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    // Trailer
    if options.hidden_data {
        let mut trailer = doc.trailer()?;
        if trailer.get_dict("Info")?.is_some() {
            trailer.dict_delete("Info")?;
            report.hidden_data += 1;
        }
        let hex: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut identifier = doc.new_array()?;
        identifier.array_push(doc.new_string(&hex)?)?;
        identifier.array_push(doc.new_string(&hex)?)?;
        trailer.dict_put("ID", identifier)?;
        report.hidden_data += 1;
    }

    let mut catalog = doc.catalog()?;

    // Catalog OpenAction
    if options.actions {
        if let Some(open_action) = catalog.get_dict("OpenAction")?
            && (open_action.is_dict()? || open_action.is_array()?) {
                report.actions += 1;
                catalog.dict_delete("OpenAction")?;
            }
        if let Some(aa) = catalog.get_dict("AA")? {
            report.actions += count_dict_entries(&aa)?;
            catalog.dict_delete("AA")?;
        }
    }

    // Catalog Names tree
    if let Some(mut names) = catalog.get_dict("Names")? {
        if options.actions
            && let Some(js) = names.get_dict("JavaScript")? {
                report.actions += count_tree_leaf_entries(&js)?.max(1);
                names.dict_delete("JavaScript")?;
            }
        if options.attachments
            && let Some(ef) = names.get_dict("EmbeddedFiles")? {
                report.attachments += count_tree_leaf_entries(&ef)?.max(1);
                names.dict_delete("EmbeddedFiles")?;
            }
    }

    // Catalog AF (Associated Files) & Collection
    if options.attachments {
        if let Some(af) = catalog.get_dict("AF")? {
            let count = if af.is_array()? {
                af.len()?.max(1) as u32
            } else {
                1
            };
            report.attachments += count;
            catalog.dict_delete("AF")?;
        }
        if catalog.get_dict("Collection")?.is_some() {
            report.attachments += 1;
            catalog.dict_delete("Collection")?;
        }
    }

    // Catalog NeedsRendering (XFA)
    if options.xfa && catalog.get_dict("NeedsRendering")?.is_some() {
        catalog.dict_delete("NeedsRendering")?;
    }

    // AcroForm
    if let Some(mut acroform) = catalog.get_dict("AcroForm")? {
        if options.xfa && acroform.get_dict("XFA")?.is_some() {
            report.xfa += 1;
            acroform.dict_delete("XFA")?;
        }
        if options.form_scripts && acroform.get_dict("CO")?.is_some() {
            report.form_scripts += 1;
            acroform.dict_delete("CO")?;
        }
        clean_form_fields(&mut acroform, options, report)?;
    }

    Ok(())
}

fn clean_form_fields(
    acroform: &mut PdfObject,
    options: SanitizeOptions,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    let Some(fields) = acroform.get_dict("Fields")? else {
        return Ok(());
    };
    if !fields.is_array()? {
        return Ok(());
    }
    let mut stack = Vec::new();
    for idx in 0..fields.len()? {
        if let Some(field) = fields.get_array(i32::try_from(idx).unwrap_or(i32::MAX))? {
            stack.push(field);
        }
    }
    let mut seen = HashSet::new();
    while let Some(mut field) = stack.pop() {
        if field.is_indirect()? && !seen.insert(field.as_indirect()?) {
            continue;
        }
        if !field.is_dict()? {
            continue;
        }
        if let Some(kids) = field.get_dict("Kids")?
            && kids.is_array()? {
                for idx in 0..kids.len()? {
                    if let Some(kid) = kids.get_array(i32::try_from(idx).unwrap_or(i32::MAX))? {
                        stack.push(kid);
                    }
                }
            }
        if let Some(aa) = field.get_dict("AA")?
            && (options.form_scripts || options.actions) {
                let count = count_dict_entries(&aa)?;
                if options.form_scripts {
                    report.form_scripts += count;
                } else {
                    report.actions += count;
                }
                field.dict_delete("AA")?;
            }
        if options.actions
            && let Some(action) = field.get_dict("A")?
                && is_dangerous_action(&action)? {
                    report.actions += 1;
                    field.dict_delete("A")?;
                }
    }
    Ok(())
}

fn clean_outlines(
    doc: &PdfDocument,
    options: SanitizeOptions,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    let Some(catalog) = doc.catalog().ok() else {
        return Ok(());
    };
    let Some(outlines) = catalog.get_dict("Outlines")? else {
        return Ok(());
    };
    let mut stack = Vec::new();
    if let Some(first) = outlines.get_dict("First")? {
        stack.push(first);
    }
    let mut seen = HashSet::new();
    while let Some(mut item) = stack.pop() {
        if item.is_indirect()? && !seen.insert(item.as_indirect()?) {
            continue;
        }
        if !item.is_dict()? {
            continue;
        }
        if let Some(next) = item.get_dict("Next")? {
            stack.push(next);
        }
        if let Some(first) = item.get_dict("First")? {
            stack.push(first);
        }
        if let Some(mut action) = item.get_dict("A")? {
            if is_uri_action(&action)? {
                let uri = get_uri_string(&action)?;
                if !is_allowed_web_link(&uri, options.keep_web_links) {
                    report.links += 1;
                    item.dict_delete("A")?;
                }
            } else if options.actions {
                if is_dangerous_action(&action)? {
                    report.actions += 1;
                    item.dict_delete("A")?;
                } else {
                    report.actions += clean_action_next(doc, &mut action)?;
                }
            }
        }
    }
    Ok(())
}

fn clean_pages(
    doc: &mut PdfDocument,
    options: SanitizeOptions,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    for page_idx in 0..doc.page_count()? {
        let mut page = doc.find_page(page_idx)?;

        // Page /AA
        if options.actions
            && let Some(aa) = page.get_dict("AA")? {
                report.actions += count_dict_entries(&aa)?;
                page.dict_delete("AA")?;
            }

        // Page /Annots
        let Some(annots) = page.get_dict("Annots")? else {
            continue;
        };
        if !annots.is_array()? {
            continue;
        }

        let mut retained = doc.new_array()?;
        for i in 0..annots.len()? {
            let Some(mut annot) = annots.get_array(i32::try_from(i).unwrap_or(i32::MAX))? else {
                continue;
            };
            if !annot.is_dict()? {
                continue;
            }
            let subtype = name_of(&annot, "Subtype")?.unwrap_or_default();

            // Rich media check
            if options.rich_media && MEDIA_SUBTYPES.contains(&subtype.as_slice()) {
                report.rich_media += 1;
                continue;
            }

            // Attachment check
            if options.attachments && subtype == b"FileAttachment" {
                report.attachments += 1;
                continue;
            }

            // Remove annotations option
            if options.annotations == SanitizeAnnotations::Remove
                && subtype != b"Widget"
                && subtype != b"Link"
            {
                report.annotations += 1;
                continue;
            }

            // Link annotation
            if subtype == b"Link" {
                if let Some(mut action) = annot.get_dict("A")? {
                    if is_uri_action(&action)? {
                        let uri = get_uri_string(&action)?;
                        if !is_allowed_web_link(&uri, options.keep_web_links) {
                            report.links += 1;
                            annot.dict_delete("A")?;
                        }
                    } else if options.actions {
                        if is_dangerous_action(&action)? {
                            report.actions += 1;
                            annot.dict_delete("A")?;
                        } else {
                            report.actions += clean_action_next(doc, &mut action)?;
                        }
                    }
                }
                // If link has no action and no destination, remove the link annotation entirely
                let has_a = annot.get_dict("A")?.is_some();
                let has_dest = annot.get_dict("Dest")?.is_some() || annot.get_dict("D")?.is_some();
                if !has_a && !has_dest {
                    continue;
                }
                retained.array_push(annot)?;
                continue;
            }

            // Widget annotation
            if subtype == b"Widget" {
                if let Some(aa) = annot.get_dict("AA")?
                    && (options.form_scripts || options.actions) {
                        let count = count_dict_entries(&aa)?;
                        if options.form_scripts {
                            report.form_scripts += count;
                        } else {
                            report.actions += count;
                        }
                        annot.dict_delete("AA")?;
                    }
                if options.actions
                    && let Some(action) = annot.get_dict("A")?
                        && is_dangerous_action(&action)? {
                            report.actions += 1;
                            annot.dict_delete("A")?;
                        }
                retained.array_push(annot)?;
                continue;
            }

            // Other annotations (Highlight, Text, FreeText, Stamp, Ink, etc.)
            if options.actions {
                if let Some(aa) = annot.get_dict("AA")? {
                    report.actions += count_dict_entries(&aa)?;
                    annot.dict_delete("AA")?;
                }
                if let Some(mut action) = annot.get_dict("A")? {
                    if is_dangerous_action(&action)? {
                        report.actions += 1;
                        annot.dict_delete("A")?;
                    } else {
                        report.actions += clean_action_next(doc, &mut action)?;
                    }
                }
            }

            if options.hidden_data && privacy::is_annotation(&annot)? {
                let mut had_author = false;
                for key in privacy::AUTHOR_ENTRIES {
                    if annot.get_dict(key)?.is_some() {
                        annot.dict_delete(key)?;
                        had_author = true;
                    }
                }
                if had_author {
                    report.hidden_data += 1;
                }
            }

            retained.array_push(annot)?;
        }

        if retained.len()? > 0 {
            page.dict_put("Annots", retained)?;
        } else {
            page.dict_delete("Annots")?;
        }
    }
    Ok(())
}

fn flatten_annotations(
    doc: &mut PdfDocument,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    let mut count = 0u32;
    for page_idx in 0..doc.page_count()? {
        let page = doc.find_page(page_idx)?;
        if let Some(annots) = page.get_dict("Annots")?
            && annots.is_array()? {
                for i in 0..annots.len()? {
                    if let Some(annot) = annots.get_array(i32::try_from(i).unwrap_or(i32::MAX))? {
                        let subtype = name_of(&annot, "Subtype")?.unwrap_or_default();
                        if subtype != b"Widget" && subtype != b"Link" {
                            count += 1;
                        }
                    }
                }
            }
    }
    report.annotations += count;
    doc.bake(true, false)?;
    Ok(())
}

fn clean_all_objects(
    doc: &PdfDocument,
    options: SanitizeOptions,
    report: &mut SanitizeReport,
) -> Result<(), EngineError> {
    privacy::walk_objects(doc, |dict| {
        if options.hidden_data {
            for key in privacy::PRIVATE_ENTRIES {
                if dict.get_dict(key)?.is_some() {
                    dict.dict_delete(key)?;
                    report.hidden_data += 1;
                }
            }
            if name_of(dict, "Type")?.as_deref() == Some(b"Page")
                && dict.get_dict("Thumb")?.is_some() {
                    dict.dict_delete("Thumb")?;
                    report.hidden_data += 1;
                }
        }

        if options.attachments {
            if let Some(f_val) = dict.get_dict("F")?
                && !f_val.is_number().unwrap_or(false) {
                    dict.dict_delete("F")?;
                    dict.dict_delete("FFilter")?;
                    dict.dict_delete("FDecodeParms")?;
                    report.attachments += 1;
                }
            if name_of(dict, "FS")?.as_deref() == Some(b"URL") {
                dict.dict_delete("FS")?;
                dict.dict_delete("F")?;
                dict.dict_delete("UF")?;
                report.attachments += 1;
            }
            if dict.get_dict("AF")?.is_some() {
                dict.dict_delete("AF")?;
            }
        }

        if options.actions {
            if let Some(aa) = dict.get_dict("AA")? {
                let count = count_dict_entries(&aa)?;
                report.actions += count;
                dict.dict_delete("AA")?;
            }
            if dict.get_dict("JS")?.is_some() {
                report.actions += 1;
                dict.dict_delete("JS")?;
            }
        }

        Ok(())
    })
}

fn count_dict_entries(dict: &PdfObject) -> Result<u32, EngineError> {
    if !dict.is_dict()? {
        return Ok(0);
    }
    let mut count = 0u32;
    for i in 0..dict.dict_len()? {
        if dict
            .get_dict_val(i32::try_from(i).unwrap_or(i32::MAX))?
            .is_some()
        {
            count += 1;
        }
    }
    Ok(count)
}

fn count_tree_leaf_entries(node: &PdfObject) -> Result<u32, EngineError> {
    if !node.is_dict()? {
        return Ok(0);
    }
    if let Some(names) = node.get_dict("Names")?
        && names.is_array()? {
            return Ok((names.len()? / 2) as u32);
        }
    if let Some(kids) = node.get_dict("Kids")?
        && kids.is_array()? {
            let mut total = 0u32;
            for i in 0..kids.len()? {
                if let Some(kid) = kids.get_array(i32::try_from(i).unwrap_or(i32::MAX))? {
                    total += count_tree_leaf_entries(&kid)?;
                }
            }
            return Ok(total);
        }
    Ok(1)
}

fn is_uri_action(action: &PdfObject) -> Result<bool, EngineError> {
    if !action.is_dict()? {
        return Ok(false);
    }
    Ok(name_of(action, "S")?.as_deref() == Some(b"URI"))
}

fn get_uri_string(action: &PdfObject) -> Result<String, EngineError> {
    if let Some(uri_obj) = action.get_dict("URI")?
        && uri_obj.is_string()? {
            let bytes = uri_obj.as_bytes()?;
            return Ok(String::from_utf8_lossy(&bytes).to_string());
        }
    Ok(String::new())
}

fn is_allowed_web_link(uri: &str, keep_web_links: bool) -> bool {
    if !keep_web_links {
        return false;
    }
    if uri.len() > MAX_URI_BYTES as usize {
        return false;
    }
    if is_network_path(uri.as_bytes()) {
        return false;
    }
    let scheme = uri
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(scheme.as_str(), "http" | "https" | "mailto")
}

fn is_dangerous_action(action: &PdfObject) -> Result<bool, EngineError> {
    if !action.is_dict()? {
        return Ok(false);
    }
    if action.get_dict("JS")?.is_some() {
        return Ok(true);
    }
    if let Some(s) = name_of(action, "S")?
        && DANGEROUS_ACTION_TYPES.contains(&s.as_slice()) {
            return Ok(true);
        }
    if let Some(next) = action.get_dict("Next")? {
        if next.is_dict()? && is_dangerous_action(&next)? {
            return Ok(true);
        }
        if next.is_array()? {
            for i in 0..next.len()? {
                if let Some(sub) = next.get_array(i32::try_from(i).unwrap_or(i32::MAX))?
                    && is_dangerous_action(&sub)? {
                        return Ok(true);
                    }
            }
        }
    }
    Ok(false)
}

fn clean_action_next(
    doc: &mupdf::pdf::PdfDocument,
    action: &mut PdfObject,
) -> Result<u32, EngineError> {
    let mut removed = 0u32;
    let Some(mut next) = action.get_dict("Next")? else {
        return Ok(0);
    };
    if next.is_dict()? {
        if is_dangerous_action(&next)? {
            removed += 1;
            action.dict_delete("Next")?;
        } else {
            removed += clean_action_next(doc, &mut next)?;
        }
    } else if next.is_array()? {
        let mut retained = doc.new_array()?;
        for i in 0..next.len()? {
            if let Some(mut sub) = next.get_array(i32::try_from(i).unwrap_or(i32::MAX))? {
                if is_dangerous_action(&sub)? {
                    removed += 1;
                } else {
                    removed += clean_action_next(doc, &mut sub)?;
                    retained.array_push(sub)?;
                }
            }
        }
        if retained.len()? > 0 {
            action.dict_put("Next", retained)?;
        } else {
            action.dict_delete("Next")?;
        }
    }
    Ok(removed)
}

/// Checks whether any finding of `report` belongs to an enabled `options` category.
pub fn check_failed(report: &SecurityReport, options: SanitizeOptions) -> bool {
    for finding in &report.findings {
        if finding.count == 0 {
            continue;
        }
        match finding.kind {
            FindingKind::JavaScript
            | FindingKind::OpenAction
            | FindingKind::Launch
            | FindingKind::SubmitForm
            | FindingKind::ImportData
            | FindingKind::RemoteGoTo
            | FindingKind::EmbeddedGoTo
            | FindingKind::UncReference => {
                if options.actions {
                    return true;
                }
            }
            FindingKind::AdditionalActions => {
                if options.actions || options.form_scripts {
                    return true;
                }
            }
            FindingKind::Xfa => {
                if options.xfa {
                    return true;
                }
            }
            FindingKind::RichMedia => {
                if options.rich_media {
                    return true;
                }
            }
            FindingKind::EmbeddedFile => {
                if options.attachments {
                    return true;
                }
            }
            FindingKind::RemoteFileSpec => {
                if options.attachments || options.actions {
                    return true;
                }
            }
        }
    }
    false
}
