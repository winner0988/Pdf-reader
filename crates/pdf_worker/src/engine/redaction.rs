//! Redaction (B2-19, ADR 0016): areas of pages are marked with `Redact` annotations, and applying
//! them with MuPDF's `pdf_redact_page` removes what the areas reach from the document, not only
//! covers it: text (seen or not, also in form objects), the pixels of pictures, and the vector
//! graphics an area touches. The other annotations and form fields that reach into an area go
//! too, with the values of the fields. Saving then rewrites the file without what nothing uses
//! (ADR 0013), so the removed content does not stay in it.
//!
//! After saving, the written file is read again (B2-20): what applying removed is kept in the
//! worker's memory until then, and the file must have no text left in a redacted area and none of
//! the removed words anywhere in its text.

use std::collections::{HashMap, HashSet};

use ipc_contract::types::{Rect, RedactionCheck, RedactionSummary};
use mupdf::TextPageFlags;
use mupdf::color::AnnotationColor;
use mupdf::pdf::{
    Encryption, PdfAnnotationType, PdfObject, PdfPage, PdfRedactImageMethod,
    PdfRedactLineArtMethod, PdfRedactOptions, PdfRedactTextMethod, PdfWriteOptions,
};
use zeroize::Zeroizing;

use super::annotations::binding_rect;
use super::{EngineError, PdfDocument, write_limited};

/// How the areas are applied (ADR 0016, the POC's options): a black box where something was
/// removed, the pixels of pictures in the area blanked, vector graphics removed when the area
/// touches them, and text removed whether it is seen or not. None of the options that only cover
/// what is there (POC 4).
const APPLY: PdfRedactOptions = PdfRedactOptions {
    black_boxes: true,
    image_method: PdfRedactImageMethod::Pixels,
    line_art: PdfRedactLineArtMethod::RemoveIfTouched,
    text: PdfRedactTextMethod::Remove,
};

/// A mark until it is applied: MuPDF draws it as a red outline over a pale area (a `Redact`
/// annotation has no interior colour of its own).
const MARK_RED: AnnotationColor = AnnotationColor::Rgb {
    red: 0.85,
    green: 0.1,
    blue: 0.1,
};
const MARK_OPACITY: f32 = 0.35;

/// How far up the field tree a widget's value may be inherited from; deeper trees are not walked.
const MAX_FIELD_DEPTH: usize = 32;

impl PdfDocument {
    /// Marks `rect` (page space) of page `index` to be redacted: a `Redact` annotation. Nothing is
    /// removed yet. Refused for a signed document, which cannot be redacted (ADR 0016).
    pub fn add_redaction(&mut self, index: u32, rect: Rect) -> Result<(), EngineError> {
        self.refuse_signed()?;
        let mut page = self.pdf_page(index)?;
        let mut mark = page.add_redact_annotation(binding_rect(rect))?;
        mark.set_color(MARK_RED)?;
        mark.set_opacity(MARK_OPACITY)?;
        mark.update()?;
        Ok(())
    }

    /// Marks `rects` (page space) of page `index` to be redacted as a single group (Issue #330):
    /// one `Redact` annotation per rectangle. Their group association is kept in worker memory
    /// only so that after saving, their removed text is checked as a single combined string.
    pub fn add_redaction_group(&mut self, index: u32, rects: &[Rect]) -> Result<(), EngineError> {
        self.refuse_signed()?;
        if rects.is_empty() {
            return Err(EngineError::InvalidEdit("redaction group cannot be empty"));
        }
        if rects.len() > ipc_contract::limits::MAX_REDACTION_GROUP_RECTS {
            return Err(EngineError::InvalidEdit(
                "too many rects in redaction group",
            ));
        }
        let group_id = self.redaction_groups.next_id;
        self.redaction_groups.next_id += 1;
        let mut page = self.pdf_page(index)?;
        for (seg_idx, &rect) in rects.iter().enumerate() {
            let mut mark = page.add_redact_annotation(binding_rect(rect))?;
            mark.set_color(MARK_RED)?;
            mark.set_opacity(MARK_OPACITY)?;
            mark.update()?;
            let xref = mark.xref()?;
            self.redaction_groups
                .by_xref
                .insert(xref, (group_id, seg_idx));
        }
        Ok(())
    }

    /// Prunes recorded redaction group entries whose object numbers are no longer `Redact`
    /// annotations in the document (for example, after marks or pages containing them are deleted).
    ///
    /// Only the mismatched xrefs are removed, leaving other groups intact. This is safe because:
    /// 1. Within the same open worker document, MuPDF appends newly created objects to the end
    ///    of the xref table and never reuses existing object numbers.
    /// 2. Saving uses `set_garbage(true)` (MuPDF garbage collection level 1), which cleans
    ///    unreferenced objects without renumbering existing objects.
    /// 3. Rebase (`WorkerRequest::Rebase`) only updates `originals` with the saved bytes for future
    ///    undo operations; it does not replace or reload the in-memory document.
    /// 4. Undo and redo reopen the document afresh from bytes, and groups are reconstructed from
    ///    replayed edits (`AddRedactionGroup`).
    pub fn prune_groups(&mut self) {
        if self.redaction_groups.by_xref.is_empty() {
            return;
        }
        let Ok(actual_redact_xrefs) = self.collect_redact_xrefs() else {
            return;
        };
        self.redaction_groups
            .by_xref
            .retain(|xref, _| actual_redact_xrefs.contains(xref));
    }

    fn collect_redact_xrefs(&self) -> Result<HashSet<i32>, EngineError> {
        let mut actual_redact_xrefs = HashSet::new();
        for index in 0..self.page_count()? {
            let page = self.pdf_page(index)?;
            for annot in page.annotations() {
                if annot.r#type()? == PdfAnnotationType::Redact {
                    actual_redact_xrefs.insert(annot.xref()?);
                }
            }
        }
        Ok(actual_redact_xrefs)
    }

    /// What applying the marks would do besides removing what their areas reach: how many marks
    /// on how many pages, and how many other annotations and form fields reach into an area.
    pub fn redaction_summary(&self) -> Result<RedactionSummary, EngineError> {
        let mut summary = RedactionSummary {
            marks: 0,
            pages: 0,
            annotations: 0,
            fields: 0,
        };
        for index in 0..self.page_count()? {
            let page = self.pdf_page(index)?;
            let marks = marks_of(&page)?;
            if marks.is_empty() {
                continue;
            }
            summary.marks += u32::try_from(marks.len()).unwrap_or(u32::MAX);
            summary.pages += 1;
            let reaching = reaching(&page, &marks)?;
            summary.annotations += u32::try_from(reaching.annotations.len()).unwrap_or(u32::MAX);
            summary.fields += u32::try_from(reaching.widgets.len()).unwrap_or(u32::MAX);
        }
        Ok(summary)
    }

    /// Applies every mark of the document (ADR 0016): on each page that has some, the other
    /// annotations and the form fields reaching into an area are deleted (fields with their
    /// values), then MuPDF removes what the areas reach and draws the black boxes; the marks go.
    /// What was read from those pages (OCR, ADR 0015) is forgotten: their pictures changed.
    /// Refused for a signed document, and when nothing is marked.
    pub fn apply_redactions(&mut self) -> Result<(), EngineError> {
        self.apply_redactions_with(APPLY)
    }

    /// As [`apply_redactions`](Self::apply_redactions), with `options`: the tests use options
    /// that only cover, which the checks after saving must catch (B2-20).
    pub(super) fn apply_redactions_with(
        &mut self,
        options: PdfRedactOptions,
    ) -> Result<(), EngineError> {
        self.refuse_signed()?;
        self.prune_groups();
        let mut redacted = Vec::new();
        for index in 0..self.page_count()? {
            let mut page = self.pdf_page(index)?;
            let mark_items = mark_items_of(&page)?;
            if mark_items.is_empty() {
                continue;
            }
            let marks: Vec<mupdf::Rect> = mark_items.iter().map(|item| item.rect).collect();
            // What the areas have before they are applied, for the check after saving.
            let number = page_number(&page)?;
            let before = text_in_areas(&page, &marks)?;
            let reaching = reaching(&page, &marks)?;
            self.remove_reaching(&mut page, reaching)?;
            page.redact_with_options(options)?;
            for (item, text) in mark_items.into_iter().zip(before) {
                let group = self.redaction_groups.by_xref.remove(&item.xref);
                self.removed.areas.push(RemovedArea {
                    page: number,
                    area: item.rect,
                    text,
                    group,
                });
            }
            redacted.push(index);
        }
        if redacted.is_empty() {
            return Err(EngineError::InvalidEdit("nothing is marked to be redacted"));
        }
        self.ocr_turned(&redacted);
        self.clear_text_cache();
        Ok(())
    }

    /// What reading `bytes` (the document just written) again finds of what applying the
    /// redactions removed since the last save (B2-20, ADR 0016): each redacted area that still has
    /// text on its page, and each area's removed words (of `MIN_CHECKED_CHARS` letters and more)
    /// that the file's text still has anywhere. A page that was removed since has no area to
    /// check. An encrypted document is written again without its encryption, in memory only, to
    /// be read: the worker keeps no password; the objects are the same.
    pub(super) fn check_removed(&self, bytes: &[u8]) -> Result<RedactionCheck, EngineError> {
        let reread = match PdfDocument::open(bytes, None) {
            Err(EngineError::Encrypted) => {
                let mut options = PdfWriteOptions::default();
                options.set_garbage(true);
                options.set_encryption(Encryption::None);
                let mut plain = Vec::new();
                write_limited(&self.doc, &mut plain, options)?;
                PdfDocument::open(&plain, None)?
            }
            other => other?,
        };
        // Where each page of the document is now: the written file keeps their order.
        let mut index_of = HashMap::new();
        for index in 0..self.page_count()? {
            index_of.insert(page_number(&self.pdf_page(index)?)?, index);
        }
        let count = reread.page_count()?;
        let mut places = 0u32;
        for removed in &self.removed.areas {
            let Some(&index) = index_of.get(&removed.page) else {
                continue;
            };
            if index < count {
                let page = reread.pdf_page(index)?;
                let left = text_in_areas(&page, &[removed.area])?;
                if left.iter().any(|text| !text.trim().is_empty()) {
                    places += 1;
                }
            }
        }
        let mut all = Zeroizing::new(String::new());
        for index in 0..count {
            let page = reread.pdf_page(index)?;
            all.push_str(&page_words(&page)?);
            all.push(' ');
        }
        let all_collapsed = Zeroizing::new(collapse(&all));
        let all_no_spaces: Zeroizing<String> =
            Zeroizing::new(all.chars().filter(|c| !c.is_whitespace()).collect());

        // Standalone marks: checked piece by piece against collapsed text.
        for removed in &self.removed.areas {
            if removed.group.is_some() {
                continue;
            }
            let words = Zeroizing::new(collapse(&removed.text));
            if words.chars().filter(|c| !c.is_whitespace()).count() >= MIN_CHECKED_CHARS
                && all_collapsed.contains(words.as_str())
            {
                places += 1;
            }
        }

        // Grouped marks: segments joined in addition order, all whitespace stripped, checked as one.
        let mut groups: HashMap<u64, Vec<(usize, &Zeroizing<String>)>> = HashMap::new();
        for removed in &self.removed.areas {
            if let Some((group_id, seg_idx)) = removed.group {
                groups
                    .entry(group_id)
                    .or_default()
                    .push((seg_idx, &removed.text));
            }
        }
        for (_group_id, mut segments) in groups {
            segments.sort_by_key(|(seg_idx, _)| *seg_idx);
            let mut combined = Zeroizing::new(String::new());
            for (_, text) in segments {
                for c in text.chars() {
                    if !c.is_whitespace() {
                        combined.push(c);
                    }
                }
            }
            if combined.chars().count() >= MIN_CHECKED_CHARS
                && all_no_spaces.contains(combined.as_str())
            {
                places += 1;
            }
        }

        Ok(RedactionCheck { places })
    }

    /// Forgets what applying the redactions removed: the file that was checked has taken the
    /// document's file's place (B2-20).
    pub fn forget_removed(&mut self) {
        self.removed = Removed::default();
        self.prune_groups();
    }

    fn refuse_signed(&self) -> Result<(), EngineError> {
        if self.is_signed() {
            return Err(EngineError::InvalidEdit(
                "a signed document cannot be redacted: appending keeps the old content, rewriting breaks the signatures",
            ));
        }
        Ok(())
    }

    /// Deletes the annotations and the form fields of `page` in `reaching`, with their pop-up
    /// windows, and takes the values out of the fields; then what still points to any of them
    /// points nowhere, so that a rewrite leaves them out of the file ([`crate::unlink`]).
    fn remove_reaching(&self, page: &mut PdfPage, reaching: Reaching) -> Result<(), EngineError> {
        if reaching.annotations.is_empty() && reaching.widgets.is_empty() {
            return Ok(());
        }
        let mut gone = HashSet::new();
        for widget in &reaching.widgets {
            clear_field_value(&widget_object(page, *widget)?)?;
        }
        let numbers: HashSet<i32> = reaching
            .annotations
            .iter()
            .chain(&reaching.widgets)
            .copied()
            .collect();
        // Found again one at a time: deleting while going through the page's list is not safe.
        for number in numbers {
            let annotation = page
                .annotations()
                .find(|annotation| annotation.xref().is_ok_and(|xref| xref == number))
                .or_else(|| {
                    page.widgets()
                        .find(|widget| widget.xref().is_ok_and(|xref| xref == number))
                        .map(mupdf::pdf::PdfWidget::into_annotation)
                });
            let Some(annotation) = annotation else {
                continue;
            };
            gone.insert(number);
            if let Some(popup) = annotation.object().get_dict("Popup")?
                && let Ok(popup) = popup.as_indirect()
                && popup > 0
            {
                gone.insert(popup);
            }
            page.delete_annotation(annotation)?;
        }
        crate::unlink::unlink(&self.doc, &gone)
    }
}

/// Which marks belong to the same found personal data item (Issue #330).
/// Stored in worker memory only, never written into the PDF.
#[derive(Default, Clone)]
pub(super) struct RedactionGroups {
    pub(super) next_id: u64,
    /// Maps annotation xref -> (group_id, segment_index)
    pub(super) by_xref: HashMap<i32, (u64, usize)>,
}

/// What applying the redactions removed since the document was last saved (B2-20): only in the
/// worker's memory, never in a journal, a log or an answer; the words are wiped when dropped.
#[derive(Default)]
pub(super) struct Removed {
    areas: Vec<RemovedArea>,
}

impl Removed {
    pub(super) fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }
}

struct RemovedArea {
    /// The page's object number: pages may be moved before the document is saved.
    page: i32,
    area: mupdf::Rect,
    text: Zeroizing<String>,
    group: Option<(u64, usize)>,
}

/// Removed words shorter than this (not counting spaces) are not looked for in the whole text:
/// they would be found in ordinary words. The areas themselves are checked all the same.
const MIN_CHECKED_CHARS: usize = 4;

/// The object number of `page`, which stays the same while the document is open.
fn page_number(page: &PdfPage) -> Result<i32, EngineError> {
    Ok(page.object().as_indirect()?)
}

/// The text of `page` in each of `areas`: the characters whose middle is in the area, a line
/// after another with a space between.
fn text_in_areas(
    page: &PdfPage,
    areas: &[mupdf::Rect],
) -> Result<Vec<Zeroizing<String>>, EngineError> {
    let text_page = page.to_text_page(TextPageFlags::empty())?;
    let mut found: Vec<Zeroizing<String>> = areas.iter().map(|_| Zeroizing::default()).collect();
    for block in text_page.blocks() {
        for line in block.lines() {
            let mut in_line: Vec<bool> = vec![false; areas.len()];
            for ch in line.chars() {
                let Some(c) = ch.char() else {
                    continue;
                };
                let quad = ch.quad();
                let x = (quad.ul.x + quad.ur.x + quad.ll.x + quad.lr.x) / 4.0;
                let y = (quad.ul.y + quad.ur.y + quad.ll.y + quad.lr.y) / 4.0;
                for (index, area) in areas.iter().enumerate() {
                    if area.x0 <= x && x <= area.x1 && area.y0 <= y && y <= area.y1 {
                        if !in_line[index] && !found[index].is_empty() {
                            found[index].push(' ');
                        }
                        in_line[index] = true;
                        found[index].push(c);
                    }
                }
            }
        }
    }
    Ok(found)
}

/// All the text of `page`, a line after another with a space between.
fn page_words(page: &PdfPage) -> Result<Zeroizing<String>, EngineError> {
    let text_page = page.to_text_page(TextPageFlags::empty())?;
    let mut words = Zeroizing::new(String::new());
    for block in text_page.blocks() {
        for line in block.lines() {
            for ch in line.chars() {
                if let Some(c) = ch.char() {
                    words.push(c);
                }
            }
            words.push(' ');
        }
    }
    Ok(words)
}

/// `text` with every run of white space made one space, and none at either end.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct MarkItem {
    xref: i32,
    rect: mupdf::Rect,
}

fn mark_items_of(page: &PdfPage) -> Result<Vec<MarkItem>, EngineError> {
    let mut marks = Vec::new();
    for annotation in page.annotations() {
        if annotation.r#type()? == PdfAnnotationType::Redact {
            marks.push(MarkItem {
                xref: annotation.xref()?,
                rect: annotation.rect()?,
            });
        }
    }
    Ok(marks)
}

/// The areas of the marks of `page`.
fn marks_of(page: &PdfPage) -> Result<Vec<mupdf::Rect>, EngineError> {
    Ok(mark_items_of(page)?
        .into_iter()
        .map(|item| item.rect)
        .collect())
}

/// What of a page reaches into one of its marks: annotation and widget numbers.
struct Reaching {
    annotations: Vec<i32>,
    widgets: Vec<i32>,
}

/// The annotations (other than the marks and pop-up windows) and the form fields of `page` whose
/// bounds reach into one of `marks`.
fn reaching(page: &PdfPage, marks: &[mupdf::Rect]) -> Result<Reaching, EngineError> {
    let touches = |bounds: mupdf::Rect| marks.iter().any(|mark| overlaps(&bounds, mark));
    let mut annotations = Vec::new();
    for annotation in page.annotations() {
        let kind = annotation.r#type()?;
        if matches!(kind, PdfAnnotationType::Redact | PdfAnnotationType::Popup) {
            continue;
        }
        if touches(annotation.bounds()?) {
            annotations.push(annotation.xref()?);
        }
    }
    let mut widgets = Vec::new();
    for widget in page.widgets() {
        if touches(widget.annotation().bounds()?) {
            widgets.push(widget.xref()?);
        }
    }
    Ok(Reaching {
        annotations,
        widgets,
    })
}

/// Whether two rectangles share some area (touching edges do not count).
fn overlaps(a: &mupdf::Rect, b: &mupdf::Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// The dictionary of widget `number` of `page`.
fn widget_object(page: &PdfPage, number: i32) -> Result<PdfObject, EngineError> {
    page.widgets()
        .find(|widget| widget.xref().is_ok_and(|xref| xref == number))
        .map(|widget| widget.annotation().object())
        .ok_or(EngineError::InvalidEdit("no such form field on the page"))
}

/// Takes the value of the field a widget belongs to out of the document: `/V` and `/DV` of the
/// widget and of the fields above it (a value can be inherited), and the appearance of every
/// widget of that field (one elsewhere on the page or in the document would still draw the
/// value). The widget itself is deleted by the caller.
fn clear_field_value(widget: &PdfObject) -> Result<(), EngineError> {
    let mut node = Some(widget.clone());
    let mut field = None;
    for _ in 0..MAX_FIELD_DEPTH {
        let Some(mut current) = node.take() else {
            break;
        };
        current.dict_delete("V")?;
        current.dict_delete("DV")?;
        // The field is the nearest one with a name: its widgets are it, or its kids.
        if field.is_none() && current.get_dict("T")?.is_some() {
            field = Some(current.clone());
        }
        node = current.get_dict("Parent")?;
    }
    let field = field.unwrap_or_else(|| widget.clone());
    let mut widgets = vec![field.clone()];
    if let Some(kids) = field.get_dict("Kids")? {
        for index in 0..kids.len()?.min(MAX_KIDS) {
            if let Some(kid) = kids.get_array(i32::try_from(index).unwrap_or(i32::MAX))? {
                widgets.push(kid);
            }
        }
    }
    for mut each in widgets {
        each.dict_delete("AP")?;
        each.dict_delete("AS")?;
    }
    Ok(())
}

/// The most widgets of one field whose appearance is taken away.
const MAX_KIDS: usize = 10_000;

#[cfg(test)]
mod tests {
    use ipc_contract::types::{Rect, RedactionCheck};
    use mupdf::pdf::{PdfRedactOptions, PdfRedactTextMethod};

    use super::APPLY;
    use crate::engine::{EngineError, PdfDocument};

    const SECRET: &str = "SECRET-ALPHA-123 inside";

    /// Pages each with a public line 700 points up and `lines[i]` 650 points up on page i.
    fn pages(lines: &[&str]) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                (0..lines.len())
                    .map(|index| format!("{} 0 R", 4 + 2 * index))
                    .collect::<Vec<_>>()
                    .join(" "),
                lines.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ];
        for (index, line) in lines.iter().enumerate() {
            let content = format!(
                "BT /F1 18 Tf 72 700 Td (Public line {index}) Tj ET\nBT /F1 18 Tf 72 650 Td ({line}) Tj ET\n"
            );
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
                5 + 2 * index
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ));
        }
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// The area of the line 650 points up (page space, from the top).
    const AREA: Rect = Rect {
        x0: 60.0,
        y0: 120.0,
        x1: 330.0,
        y1: 150.0,
    };

    fn saved_check(doc: &PdfDocument) -> (Vec<u8>, Option<RedactionCheck>) {
        let mut out = Vec::new();
        let written = doc.save_checked(&mut out).expect("saved");
        assert_eq!(written.bytes, out.len() as u64);
        (out, written.redaction)
    }

    fn places(doc: &PdfDocument) -> Option<u32> {
        saved_check(doc).1.map(|check| check.places)
    }

    fn has(bytes: &[u8], needle: &str) -> bool {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    const ONLY_COVER: PdfRedactOptions = PdfRedactOptions {
        text: PdfRedactTextMethod::None,
        ..APPLY
    };

    /// Everything found as personal data (#302), marked and applied, is gone from the saved file:
    /// the check after saving (B2-20) finds none of it, the pieces of an ID wrapped over two lines
    /// included.
    #[test]
    fn found_personal_data_is_removed_and_the_check_after_saving_holds() {
        use ipc_contract::types::{FindPersonalDataQuery, PersonalDataCategory};

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/benign/personal-data.pdf");
        let mut doc = PdfDocument::from_bytes(&std::fs::read(path).expect("corpus")).expect("open");
        let found = doc
            .find_personal_data(&FindPersonalDataQuery {
                categories: vec![
                    PersonalDataCategory::TaiwanId,
                    PersonalDataCategory::MobilePhone,
                    PersonalDataCategory::LandlinePhone,
                    PersonalDataCategory::Email,
                    PersonalDataCategory::CreditCard,
                    PersonalDataCategory::UnifiedBusinessNumber,
                ],
                keyword: None,
            })
            .expect("find");
        assert_eq!(found.matches.len(), 11);
        for found in &found.matches {
            for rect in &found.rects {
                doc.add_redaction(found.page, *rect).expect("mark");
            }
        }
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(0));
    }

    #[test]
    fn a_saved_redaction_is_read_again_and_holds() {
        let mut doc = PdfDocument::from_bytes(&pages(&[SECRET])).expect("open");
        // Nothing applied: nothing to check.
        assert_eq!(places(&doc), None);
        doc.add_redaction(0, AREA).expect("mark");
        assert_eq!(places(&doc), None, "a mark alone removes nothing");
        doc.apply_redactions().expect("apply");
        let (bytes, check) = saved_check(&doc);
        assert_eq!(check.map(|check| check.places), Some(0));
        assert!(!has(&bytes, "SECRET-ALPHA-123"));
        // Saved again before the file took the document's place: checked again.
        assert_eq!(places(&doc), Some(0));
        // Once it has, the removed words are forgotten, and nothing is left to check.
        doc.forget_removed();
        assert_eq!(places(&doc), None);
    }

    #[test]
    fn options_that_only_cover_fail_the_check() {
        // The negative control: the text stays under the box.
        let mut doc = PdfDocument::from_bytes(&pages(&[SECRET])).expect("open");
        doc.add_redaction(0, AREA).expect("mark");
        doc.apply_redactions_with(ONLY_COVER).expect("apply");
        let (bytes, check) = saved_check(&doc);
        // The area still has text, and its words are in the document: two places.
        assert_eq!(check.map(|check| check.places), Some(2));
        assert!(has(&bytes, "SECRET-ALPHA-123"));
    }

    #[test]
    fn pages_moved_or_removed_after_applying_are_found_or_left_out() {
        // The secret on the second page; the first page goes after applying: the area is found
        // on the page that is now first.
        let mut doc = PdfDocument::from_bytes(&pages(&["nothing here", SECRET])).expect("open");
        doc.add_redaction(1, AREA).expect("mark");
        doc.apply_redactions_with(ONLY_COVER).expect("apply");
        doc.delete_pages(&[0]).expect("delete");
        assert_eq!(places(&doc), Some(2));

        // The redacted page itself goes: no area to check, and its words are nowhere.
        let mut doc = PdfDocument::from_bytes(&pages(&[SECRET, "other words"])).expect("open");
        doc.add_redaction(0, AREA).expect("mark");
        doc.apply_redactions_with(ONLY_COVER).expect("apply");
        doc.delete_pages(&[0]).expect("delete");
        assert_eq!(places(&doc), Some(0));
    }

    #[test]
    fn short_removed_words_are_not_looked_for_everywhere() {
        // "Pub" is removed from the area but stays in the public line above it ("Public"): too
        // short to mean anything elsewhere. The area itself is still checked, and holds. Four
        // letters are looked for: "line" would be found there.
        let mut doc = PdfDocument::from_bytes(&pages(&["Pub"])).expect("open");
        doc.add_redaction(0, AREA).expect("mark");
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(0));
        let mut doc = PdfDocument::from_bytes(&pages(&["line"])).expect("open");
        doc.add_redaction(0, AREA).expect("mark");
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(1));
    }

    #[test]
    fn an_encrypted_document_is_read_again_without_its_encryption() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus/benign/restricted-no-copy-no-print.pdf"),
        )
        .expect("corpus");
        let mut doc = PdfDocument::open(&bytes, Some("owner")).expect("open");
        let hits = doc
            .search_page(0, "no printing", true, 4)
            .expect("search")
            .hits;
        assert_eq!(hits.len(), 1);
        let points: Vec<_> = hits[0]
            .quads
            .iter()
            .flat_map(|quad| [quad.ul, quad.ur, quad.ll, quad.lr])
            .collect();
        let area = Rect {
            x0: points.iter().map(|p| p.x).fold(f32::MAX, f32::min) - 1.0,
            y0: points.iter().map(|p| p.y).fold(f32::MAX, f32::min) - 1.0,
            x1: points.iter().map(|p| p.x).fold(f32::MIN, f32::max) + 1.0,
            y1: points.iter().map(|p| p.y).fold(f32::MIN, f32::max) + 1.0,
        };
        doc.add_redaction(0, area).expect("mark");
        doc.apply_redactions().expect("apply");
        let (written, check) = saved_check(&doc);
        assert_eq!(check.map(|check| check.places), Some(0));
        // Still encrypted as it was; opened again with its password, the words are gone.
        assert!(matches!(
            PdfDocument::open(&written, None),
            Ok(_) | Err(EngineError::Encrypted)
        ));
        let again = PdfDocument::open(&written, Some("owner")).expect("reopen");
        assert!(
            again
                .search_page(0, "no printing", true, 4)
                .unwrap()
                .hits
                .is_empty()
        );
    }

    fn search_area_hit(doc: &PdfDocument, page: u32, query: &str, hit_index: usize) -> Rect {
        let hits = doc.search_page(page, query, true, 8).expect("search").hits;
        assert!(
            hits.len() > hit_index,
            "query '{query}' hit {hit_index} on page {page}"
        );
        let points: Vec<_> = hits[hit_index]
            .quads
            .iter()
            .flat_map(|quad| [quad.ul, quad.ur, quad.ll, quad.lr])
            .collect();
        Rect {
            x0: points.iter().map(|p| p.x).fold(f32::MAX, f32::min) - 1.0,
            y0: points.iter().map(|p| p.y).fold(f32::MAX, f32::min) - 1.0,
            x1: points.iter().map(|p| p.x).fold(f32::MIN, f32::max) + 1.0,
            y1: points.iter().map(|p| p.y).fold(f32::MIN, f32::max) + 1.0,
        }
    }

    fn search_area(doc: &PdfDocument, page: u32, query: &str) -> Rect {
        search_area_hit(doc, page, query, 0)
    }

    fn make_lines_doc(lines: &[&str]) -> (PdfDocument, Vec<u8>) {
        let mut text = String::new();
        let mut y = 700.0;
        for content in lines {
            text.push_str(&format!("BT /F1 18 Tf 72 {y} Td ({content}) Tj ET\n"));
            y -= 50.0;
        }
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        let doc = PdfDocument::from_bytes(&out).expect("open");
        (doc, out)
    }

    fn wrapped_number_doc(extra_line: &str) -> (PdfDocument, Rect, Rect) {
        let (doc, _) = make_lines_doc(&["ends with A1234", "56789 begins line", extra_line]);
        let area1 = search_area(&doc, 0, "A1234");
        let area2 = search_area(&doc, 0, "56789");
        (doc, area1, area2)
    }

    #[test]
    fn separate_marks_false_positive_on_wrapped_match_piece() {
        // Line 3 has wrong checksum A123456780.
        // Marking A1234 and 56789 with separate AddRedaction reports at least 1 place!
        let (mut doc, area1, area2) = wrapped_number_doc("wrong checksum A123456780");
        doc.add_redaction(0, area1).expect("mark 1");
        doc.add_redaction(0, area2).expect("mark 2");
        doc.apply_redactions().expect("apply");
        let check_places = places(&doc);
        assert_eq!(
            check_places,
            Some(1),
            "separate marks falsely report piece A1234 found in A123456780"
        );
    }

    #[test]
    fn wrapped_personal_data_group_clears_false_positive() {
        // Marking A1234 and 56789 as a single group checks the whole string A123456789
        // without spaces. A123456780 does not match it, so 0 places are reported.
        let (mut doc, area1, area2) = wrapped_number_doc("wrong checksum A123456780");
        doc.add_redaction_group(0, &[area1, area2]).expect("group");
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(0));
    }

    #[test]
    fn true_residual_whole_number_unwrapped_elsewhere_is_reported() {
        // The whole number A123456789 appears on line 3 without wrapping.
        let (mut doc, area1, area2) = wrapped_number_doc("unwrapped A123456789 elsewhere");
        doc.add_redaction_group(0, &[area1, area2]).expect("group");
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(1));
    }

    #[test]
    fn true_residual_wrapped_across_lines_elsewhere_is_reported() {
        // The whole number wraps across line 3 and line 4 elsewhere in the document.
        let (mut doc, _) = make_lines_doc(&[
            "ends with A1234",
            "56789 begins line",
            "elsewhere also ends with A1234",
            "56789 begins another line",
        ]);
        let area1 = search_area_hit(&doc, 0, "A1234", 0);
        let area2 = search_area_hit(&doc, 0, "56789", 0);
        doc.add_redaction_group(0, &[area1, area2]).expect("group");
        doc.apply_redactions().expect("apply");
        assert_eq!(places(&doc), Some(1));
    }

    #[test]
    fn only_cover_option_reports_all_areas() {
        // With ONLY_COVER, underlying text remains in the stream: both areas fail the
        // in-area check, and the combined string is found in the text stream (total 3).
        let (mut doc, area1, area2) = wrapped_number_doc("plain line without match");
        doc.add_redaction_group(0, &[area1, area2]).expect("group");
        doc.apply_redactions_with(ONLY_COVER).expect("apply");
        assert_eq!(places(&doc), Some(3));
    }

    #[test]
    fn undo_and_redo_rebuilds_group_and_still_checks_together() {
        let (doc, raw_bytes) = make_lines_doc(&[
            "ends with A1234",
            "56789 begins line",
            "wrong checksum A123456780",
        ]);
        let area1 = search_area_hit(&doc, 0, "A1234", 0);
        let area2 = search_area_hit(&doc, 0, "56789", 0);

        let edits = vec![
            ipc_contract::worker::WorkerEdit::AddRedactionGroup {
                page: 0,
                rects: vec![area1, area2],
            },
            ipc_contract::worker::WorkerEdit::ApplyRedactions,
        ];

        // Replaying edits (revert) restores the group in memory.
        let mut replayed = PdfDocument::open(&raw_bytes, None).expect("reopen");
        for edit in &edits {
            match edit {
                ipc_contract::worker::WorkerEdit::AddRedactionGroup { page, rects } => {
                    replayed.add_redaction_group(*page, rects).unwrap();
                }
                ipc_contract::worker::WorkerEdit::ApplyRedactions => {
                    replayed.apply_redactions().unwrap();
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(places(&replayed), Some(0));
    }

    #[test]
    fn a_group_added_after_an_earlier_apply_is_still_checked_whole() {
        let (mut doc, _) = make_lines_doc(&[
            "ends with A1234",
            "56789 begins line",
            "wrong checksum A123456780",
            "then ends with B9876",
            "54321 begins line",
            "wrong checksum B987654320",
        ]);
        let a1 = search_area_hit(&doc, 0, "A1234", 0);
        let a2 = search_area_hit(&doc, 0, "56789", 0);
        let b1 = search_area_hit(&doc, 0, "B9876", 0);
        let b2 = search_area_hit(&doc, 0, "54321", 0);
        doc.add_redaction_group(0, &[a1, a2]).expect("group a");
        doc.apply_redactions().expect("apply a");
        doc.add_redaction_group(0, &[b1, b2]).expect("group b");
        doc.apply_redactions().expect("apply b");
        assert_eq!(
            places(&doc),
            Some(0),
            "second group fell back to piece by piece"
        );
    }

    #[test]
    fn deleting_a_page_with_a_group_keeps_the_other_groups() {
        let (mut doc, _) = make_lines_doc(&[
            "ends with A1234",
            "56789 begins line",
            "wrong checksum A123456780",
        ]);
        let a1 = search_area_hit(&doc, 0, "A1234", 0);
        let a2 = search_area_hit(&doc, 0, "56789", 0);
        doc.insert_blank_page(1, 0).expect("blank page");
        doc.add_redaction_group(1, &[a1])
            .expect("group on the blank page");
        doc.delete_pages(&[1]).expect("delete the blank page");
        doc.add_redaction_group(0, &[a1, a2]).expect("group a");
        doc.apply_redactions().expect("apply");
        assert_eq!(
            places(&doc),
            Some(0),
            "group fell back to piece by piece after a page was deleted"
        );
    }
}
