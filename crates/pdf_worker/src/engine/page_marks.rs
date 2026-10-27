//! Page marks (B3-16, docs/architecture/page-marks.md): page numbers, headers and footers,
//! and Bates numbers drawn on the pages of a copy of a document, as page content over what is there.
//! Every reader shows page content, and no click takes it off, as it would an annotation; a reader
//! that copies the text finds the numbers and words too. The document itself is not changed:
//! the copy is the document saved to memory and read again, as the watermark's is.
//!
//! The words are set in a subset of Droid Sans Fallback, which has Chinese and Latin letters,
//! with ToUnicode mappings so that search and copy-paste work. One subset, written once, has every
//! character the marks can use: a number on each of thousands of pages adds a few hundred bytes a
//! page, not a font.

use std::collections::BTreeSet;
use std::io::Write;

use ipc_contract::types::{PageMarkPosition, PageMarks, PageMarksError, PageNumberFormat};
use ipc_contract::validate::Validate;
use mupdf::pdf::{PdfObject, PdfWriteOptions};
use mupdf::{Matrix, Rect};

use super::page_overlay::{attach_forms_to_page, droid_sans_line_font};
use super::text_box::line_form_in;
use super::{EngineError, PdfDocument, write_limited};

/// Returns the Form XObject name for `position`.
fn slot_name(position: PageMarkPosition) -> &'static str {
    match position {
        PageMarkPosition::TopLeft => "PdfReaderPageMarks_TL",
        PageMarkPosition::TopCenter => "PdfReaderPageMarks_TC",
        PageMarkPosition::TopRight => "PdfReaderPageMarks_TR",
        PageMarkPosition::BottomLeft => "PdfReaderPageMarks_BL",
        PageMarkPosition::BottomCenter => "PdfReaderPageMarks_BC",
        PageMarkPosition::BottomRight => "PdfReaderPageMarks_BR",
    }
}

/// Calculates the center coordinates `(x, y)` in page space for placing a mark of size
/// `(width, height)` at `position`, taking page `bounds` and `margin` into account.
fn slot_center(
    position: PageMarkPosition,
    bounds: &Rect,
    margin: f32,
    width: f32,
    height: f32,
) -> (f32, f32) {
    let left = bounds.x0;
    let top = bounds.y0;
    let right = bounds.x1;
    let bottom = bounds.y1;
    let page_width = right - left;

    let x = match position {
        PageMarkPosition::TopLeft | PageMarkPosition::BottomLeft => left + margin + width / 2.0,
        PageMarkPosition::TopCenter | PageMarkPosition::BottomCenter => left + page_width / 2.0,
        PageMarkPosition::TopRight | PageMarkPosition::BottomRight => right - margin - width / 2.0,
    };

    let y = match position {
        PageMarkPosition::TopLeft | PageMarkPosition::TopCenter | PageMarkPosition::TopRight => {
            top + margin + height / 2.0
        }
        PageMarkPosition::BottomLeft
        | PageMarkPosition::BottomCenter
        | PageMarkPosition::BottomRight => bottom - margin - height / 2.0,
    };

    (x, y)
}

/// Where a form `width` by `height` at `position` goes on a page whose displayed bounds are
/// `bounds`: level as the page is shown, mapped to the page's own space by `inverse`.
fn placement(
    position: PageMarkPosition,
    bounds: &Rect,
    margin: f32,
    (width, height): (f32, f32),
    inverse: &Matrix,
) -> Matrix {
    let (x, y) = slot_center(position, bounds, margin, width, height);
    let mut matrix = Matrix::new_translate(-width / 2.0, -height / 2.0);
    matrix.concat(Matrix::new_scale(1.0, -1.0));
    matrix.concat(Matrix::new_translate(x, y));
    matrix.concat(inverse.clone());
    matrix
}

/// The words of page number `n` of `total`.
fn page_number_text(format: &PageNumberFormat, n: u32, total: u32) -> String {
    match format {
        PageNumberFormat::Number => format!("{n}"),
        PageNumberFormat::Page => format!("第 {n} 頁"),
        PageNumberFormat::PageOfTotal => format!("第 {n} 頁／共 {total} 頁"),
        PageNumberFormat::Custom { prefix, suffix } => format!("{prefix}{n}{suffix}"),
    }
}

/// Every character the marks can have: their fixed words and the ten digits.
fn characters(marks: &PageMarks) -> String {
    let mut all = BTreeSet::new();
    if let Some(number) = &marks.page_number {
        all.extend(page_number_text(&number.format, 0, 0).chars());
        all.extend('0'..='9');
    }
    if let Some(bates) = &marks.bates {
        all.extend(bates.prefix.chars());
        all.extend('0'..='9');
    }
    if let Some(words) = &marks.header_footer {
        for text in [
            &words.header_left,
            &words.header_center,
            &words.header_right,
            &words.footer_left,
            &words.footer_center,
            &words.footer_right,
        ]
        .into_iter()
        .flatten()
        {
            all.extend(text.chars());
        }
    }
    all.into_iter().collect()
}

impl PdfDocument {
    /// Writes to `out` a copy of the document, edits included, with `marks` on its pages
    /// (B3-16).
    /// The open document is not touched. An encrypted document is refused, as for the privacy
    /// export: the worker keeps no password, and the copy could not be encrypted again. The
    /// signatures of a signed one do not hold in the copy, whose pages are not those signed.
    pub fn page_marks_copy(
        &self,
        marks: &PageMarks,
        out: &mut impl Write,
    ) -> Result<u64, EngineError> {
        marks
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the page marks"))?;
        if self.is_encrypted() {
            return Err(EngineError::EncryptedCopy);
        }
        let total_pages = self.page_count()?;
        if total_pages == 0 {
            return Err(EngineError::NoPageLeft);
        }

        let marked_pages = marks
            .marked_pages(total_pages)
            .map_err(|error| match error {
                PageMarksError::PageOutOfRange(past) => EngineError::PageOutOfRange(past),
                PageMarksError::NoPageToMark => EngineError::InvalidEdit("no page to mark"),
                PageMarksError::BatesOverflow => EngineError::InvalidEdit("bates number overflow"),
            })?;
        let marked_count = marked_pages.len() as u64;

        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut bytes = Vec::new();
        write_limited(&self.doc, &mut bytes, options)?;
        let mut copy = PdfDocument::from_bytes(&bytes)?;
        drop(bytes);

        let font = droid_sans_line_font(&mut copy.doc, &characters(marks))?;
        let line = |copy: &mut PdfDocument, text: &str| {
            line_form_in(&mut copy.doc, &font, text, marks.font_size, marks.color)
        };

        // Headers and footers are the same on every page: one form each.
        let mut header_footer = Vec::new();
        if let Some(words) = &marks.header_footer {
            for (position, text) in [
                (PageMarkPosition::TopLeft, &words.header_left),
                (PageMarkPosition::TopCenter, &words.header_center),
                (PageMarkPosition::TopRight, &words.header_right),
                (PageMarkPosition::BottomLeft, &words.footer_left),
                (PageMarkPosition::BottomCenter, &words.footer_center),
                (PageMarkPosition::BottomRight, &words.footer_right),
            ] {
                if let Some(text) = text.as_deref().filter(|text| !text.is_empty()) {
                    let (form, width, height) = line(&mut copy, text)?;
                    header_footer.push((position, form, (width, height)));
                }
            }
        }

        let last_number = marks.page_number.as_ref().map_or(0, |number| {
            number.start.saturating_add((marked_count - 1) as u32)
        });
        for (sequence, &page_index) in marked_pages.iter().enumerate() {
            let (mut page_object, inverse) = copy.page_to_pdf(page_index)?;
            let bounds = copy.load_page(page_index)?.bounds()?;
            if !(bounds.x1 - bounds.x0 > 0.0 && bounds.y1 - bounds.y0 > 0.0) {
                continue;
            }

            let mut forms: Vec<(&'static str, PdfObject, Matrix)> = Vec::new();
            for (position, form, size) in &header_footer {
                let matrix = placement(*position, &bounds, marks.margin_pt, *size, &inverse);
                forms.push((slot_name(*position), form.try_clone()?, matrix));
            }
            if let Some(number) = &marks.page_number {
                let n = number.start.saturating_add(sequence as u32);
                let text = page_number_text(&number.format, n, last_number);
                let (form, width, height) = line(&mut copy, &text)?;
                let matrix = placement(
                    number.position,
                    &bounds,
                    marks.margin_pt,
                    (width, height),
                    &inverse,
                );
                forms.push((slot_name(number.position), form, matrix));
            }
            if let Some(bates) = &marks.bates {
                let n = bates.start + sequence as u64;
                let text = bates.format_number(n);
                let (form, width, height) = line(&mut copy, &text)?;
                let matrix = placement(
                    bates.position,
                    &bounds,
                    marks.margin_pt,
                    (width, height),
                    &inverse,
                );
                forms.push((slot_name(bates.position), form, matrix));
            }

            let forms: Vec<(&str, &PdfObject, &Matrix)> = forms
                .iter()
                .map(|(name, form, matrix)| (*name, form, matrix))
                .collect();
            attach_forms_to_page(&mut copy.doc, &mut page_object, &forms)?;
        }

        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        options.set_compress(true);
        write_limited(&copy.doc, out, options)
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{
        BatesMark, HeaderFooterMarks, PageMarkPages, PageMarkPosition, PageMarks, PageNumberFormat,
        PageNumberMark, Rgb,
    };
    use mupdf::TextPageFlags;

    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn copy_of(doc: &PdfDocument, marks: &PageMarks) -> Result<PdfDocument, EngineError> {
        let mut out = Vec::new();
        doc.page_marks_copy(marks, &mut out)?;
        PdfDocument::from_bytes(&out)
    }

    fn text_of(doc: &PdfDocument, page: u32) -> String {
        let p = doc.load_page(page).unwrap();
        let text_page = p.to_text_page(TextPageFlags::empty()).unwrap();
        let mut words = String::new();
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
        words
    }

    fn default_marks() -> PageMarks {
        PageMarks {
            font_size: 12.0,
            color: Rgb { r: 0, g: 0, b: 0 },
            margin_pt: 36.0,
            pages: PageMarkPages::All,
            page_number: None,
            bates: None,
            header_footer: None,
        }
    }

    #[test]
    fn four_page_number_formats_and_total_calculation() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();

        // 1. Number
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Number,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(text_of(&copy, 0).contains("1"));
        assert!(text_of(&copy, 9).contains("10"));

        // 2. Page
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Page,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(text_of(&copy, 0).contains("第 1 頁"));
        assert!(text_of(&copy, 9).contains("第 10 頁"));

        // 3. PageOfTotal with AllButFirst: 10 pages total, marked 9 pages.
        // First marked page is page 1, start 1 -> "第 1 頁／共 9 頁".
        let marks = PageMarks {
            pages: PageMarkPages::AllButFirst,
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::PageOfTotal,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(!text_of(&copy, 0).contains("第 1 頁"));
        assert!(text_of(&copy, 1).contains("第 1 頁／共 9 頁"));
        assert!(text_of(&copy, 9).contains("第 9 頁／共 9 頁"));

        // 4. Custom
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Custom {
                    prefix: "- ".to_string(),
                    suffix: " -".to_string(),
                },
                start: 5,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(text_of(&copy, 0).contains("- 5 -"));
        assert!(text_of(&copy, 9).contains("- 14 -"));
    }

    #[test]
    fn custom_page_range_and_out_of_bounds() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();
        let marks = PageMarks {
            pages: PageMarkPages::Custom {
                pages: vec![2, 5, 8],
            },
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::PageOfTotal,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(!text_of(&copy, 0).contains("第 1 頁"));
        assert!(!text_of(&copy, 1).contains("第 1 頁"));
        assert!(text_of(&copy, 2).contains("第 1 頁／共 3 頁"));
        assert!(text_of(&copy, 5).contains("第 2 頁／共 3 頁"));
        assert!(text_of(&copy, 8).contains("第 3 頁／共 3 頁"));

        // Out of bounds page index is rejected
        let oob = PageMarks {
            pages: PageMarkPages::Custom { pages: vec![0, 10] },
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Number,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        assert!(matches!(
            copy_of(&doc, &oob),
            Err(EngineError::PageOutOfRange(10))
        ));
    }

    #[test]
    fn bates_padding_and_overflow_rejection() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();

        // 6 digits padding: ABC000001 to ABC000010
        let marks = PageMarks {
            bates: Some(BatesMark {
                prefix: "ABC".to_string(),
                digits: 6,
                start: 1,
                position: PageMarkPosition::BottomRight,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();
        assert!(text_of(&copy, 0).contains("ABC000001"));
        assert!(text_of(&copy, 9).contains("ABC000010"));

        // Overflow: 10 pages with 1 digit starting at 1 -> needs up to 10 (2 digits) -> rejected!
        let overflow = PageMarks {
            bates: Some(BatesMark {
                prefix: "CASE-".to_string(),
                digits: 1,
                start: 1,
                position: PageMarkPosition::BottomRight,
            }),
            ..default_marks()
        };
        let mut out = Vec::new();
        assert!(matches!(
            doc.page_marks_copy(&overflow, &mut out),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(out.is_empty(), "no file should be written on overflow");
    }

    #[test]
    fn six_slots_positions_verified_by_text_quads() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();

        let marks = PageMarks {
            header_footer: Some(HeaderFooterMarks {
                header_left: Some("TL_MARK".to_string()),
                header_center: Some("TC_MARK".to_string()),
                header_right: Some("TR_MARK".to_string()),
                footer_left: Some("BL_MARK".to_string()),
                footer_center: Some("BC_MARK".to_string()),
                footer_right: Some("BR_MARK".to_string()),
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();

        let page = copy.load_page(0).unwrap();
        let bounds = page.bounds().unwrap();
        let (w, h) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);

        let text_page = page.to_text_page(TextPageFlags::empty()).unwrap();
        let mut mark_centers = Vec::new();

        for block in text_page.blocks() {
            for line in block.lines() {
                let mut line_text = String::new();
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                for ch in line.chars() {
                    if let Some(c) = ch.char() {
                        line_text.push(c);
                        let q = ch.quad();
                        xs.push((q.ul.x + q.ur.x + q.ll.x + q.lr.x) / 4.0);
                        ys.push((q.ul.y + q.ur.y + q.ll.y + q.lr.y) / 4.0);
                    }
                }
                let line_text = line_text.trim();
                if line_text.contains("_MARK") && !xs.is_empty() {
                    let avg_x: f32 = xs.iter().sum::<f32>() / xs.len() as f32;
                    let avg_y: f32 = ys.iter().sum::<f32>() / ys.len() as f32;
                    mark_centers.push((line_text.to_string(), avg_x, avg_y));
                }
            }
        }

        assert_eq!(mark_centers.len(), 6, "all 6 marks found");
        for (name, x, y) in mark_centers {
            if name.contains("TL_MARK") {
                assert!(x < w / 3.0, "TL x {x} < {}", w / 3.0);
                assert!(y < h / 3.0, "TL y {y} < {}", h / 3.0);
            } else if name.contains("TC_MARK") {
                assert!(x >= w / 3.0 && x <= 2.0 * w / 3.0, "TC x {x}");
                assert!(y < h / 3.0, "TC y {y} < {}", h / 3.0);
            } else if name.contains("TR_MARK") {
                assert!(x > 2.0 * w / 3.0, "TR x {x} > {}", 2.0 * w / 3.0);
                assert!(y < h / 3.0, "TR y {y} < {}", h / 3.0);
            } else if name.contains("BL_MARK") {
                assert!(x < w / 3.0, "BL x {x} < {}", w / 3.0);
                assert!(y > 2.0 * h / 3.0, "BL y {y} > {}", 2.0 * h / 3.0);
            } else if name.contains("BC_MARK") {
                assert!(x >= w / 3.0 && x <= 2.0 * w / 3.0, "BC x {x}");
                assert!(y > 2.0 * h / 3.0, "BC y {y} > {}", 2.0 * h / 3.0);
            } else if name.contains("BR_MARK") {
                assert!(x > 2.0 * w / 3.0, "BR x {x} > {}", 2.0 * w / 3.0);
                assert!(y > 2.0 * h / 3.0, "BR y {y} > {}", 2.0 * h / 3.0);
            }
        }
    }

    #[test]
    fn rotated_page_marks_are_level_and_document_is_untouched() {
        let doc = PdfDocument::from_bytes(&corpus("benign/rotated-page.pdf")).unwrap();
        let marks = PageMarks {
            font_size: 24.0,
            color: Rgb { r: 255, g: 0, b: 0 },
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Custom {
                    prefix: "PAGE_ROTATED_".to_string(),
                    suffix: "".to_string(),
                },
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let copy = copy_of(&doc, &marks).unwrap();

        // 1. Copy has no annotations (marks are page content)
        assert!(copy.page_annotations(0).unwrap().is_empty());

        // 2. Open document itself is unchanged
        assert!(text_of(&copy, 0).contains("PAGE_ROTATED_1"));
        assert!(!text_of(&doc, 0).contains("PAGE_ROTATED_"));

        // 3. Rendered text on rotated page is level as shown (width > height)
        let rendered = copy.render(0, 1.0, 0).unwrap();
        let mut red_points = Vec::new();
        for y in 0..rendered.height as usize {
            for x in 0..rendered.width as usize {
                let at = (y * rendered.width as usize + x) * 4;
                let [r, g, b] = [
                    rendered.rgba[at] as i32,
                    rendered.rgba[at + 1] as i32,
                    rendered.rgba[at + 2] as i32,
                ];
                if r > g + 40 && r > b + 40 {
                    red_points.push((x, y));
                }
            }
        }
        assert!(!red_points.is_empty(), "red mark rendered");
        let min_x = red_points.iter().map(|p| p.0).min().unwrap();
        let max_x = red_points.iter().map(|p| p.0).max().unwrap();
        let min_y = red_points.iter().map(|p| p.1).min().unwrap();
        let max_y = red_points.iter().map(|p| p.1).max().unwrap();
        assert!(
            max_x - min_x > (max_y - min_y) * 2,
            "mark is level on rotated page: w={}, h={}",
            max_x - min_x,
            max_y - min_y
        );
    }

    #[test]
    fn encrypted_document_is_rejected() {
        let encrypted =
            PdfDocument::open(&corpus("benign/encrypted-rc4-40.pdf"), Some("user")).unwrap();
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Number,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let mut out = Vec::new();
        assert!(matches!(
            encrypted.page_marks_copy(&marks, &mut out),
            Err(EngineError::EncryptedCopy)
        ));
    }

    #[test]
    fn one_font_serves_every_mark_of_every_page() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();
        let marks = PageMarks {
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::PageOfTotal,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            bates: Some(BatesMark {
                prefix: "案號 ".to_owned(),
                digits: 6,
                start: 1,
                position: PageMarkPosition::BottomRight,
            }),
            header_footer: Some(HeaderFooterMarks {
                header_left: Some("機密文件".to_owned()),
                ..HeaderFooterMarks::default()
            }),
            ..default_marks()
        };
        let mut out = Vec::new();
        doc.page_marks_copy(&marks, &mut out).unwrap();
        // The sample embeds no font of its own.
        let fonts = out
            .windows(b"/FontFile2".len())
            .filter(|window| *window == b"/FontFile2")
            .count();
        assert_eq!(fonts, 1);

        let copy = PdfDocument::from_bytes(&out).unwrap();
        for page in [0, 9] {
            let words = text_of(&copy, page);
            let n = page + 1;
            assert!(words.contains(&format!("第 {n} 頁／共 10 頁")), "{words}");
            assert!(words.contains(&format!("案號 {n:06}")), "{words}");
            assert!(words.contains("機密文件"), "{words}");
        }
    }

    #[test]
    fn all_but_the_first_page_of_one_page_is_refused() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();
        let marks = PageMarks {
            pages: PageMarkPages::AllButFirst,
            page_number: Some(PageNumberMark {
                format: PageNumberFormat::Number,
                start: 1,
                position: PageMarkPosition::BottomCenter,
            }),
            ..default_marks()
        };
        let mut out = Vec::new();
        assert!(matches!(
            doc.page_marks_copy(&marks, &mut out),
            Err(EngineError::InvalidEdit(_))
        ));
        assert!(out.is_empty());
    }
}
