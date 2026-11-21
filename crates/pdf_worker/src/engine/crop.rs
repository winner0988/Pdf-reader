//! Page cropping (B6-04, #589): setting a page's `/CropBox` in memory (ADR 0013).
//!
//! Margins are measured in from the edges of each page as it is shown (page space).
//! Every page is calculated first; if any page would become smaller than `MIN_CROPPED_SIDE_PT`
//! in width or height, nothing is modified.
//! If writing the new `/CropBox` fails midway, all already-modified pages are restored to their
//! original `/CropBox` (or the key is removed if they didn't have their own).

use ipc_contract::limits::MIN_CROPPED_SIDE_PT;
use ipc_contract::types::CropMargins;
use mupdf::Point;
use mupdf::pdf::PdfObject;

use super::{EngineError, PdfDocument};

struct PlannedCrop {
    page: u32,
    new_box: [f32; 4],
    original_box: Option<PdfObject>,
}

impl PdfDocument {
    /// Crops `pages` by setting each page's `/CropBox` to the area inside `margins` (measured
    /// in from the edges of the page as it is shown).
    ///
    /// Every page is checked and calculated before anything is modified: if any page's cropped
    /// width or height would be less than [`MIN_CROPPED_SIDE_PT`], the whole operation is
    /// rejected with [`EngineError::InvalidEdit`].
    ///
    /// If writing to MuPDF fails midway, any pages already modified are rolled back to their
    /// previous state, leaving the document unchanged.
    pub fn crop_pages(&mut self, pages: &[u32], margins: CropMargins) -> Result<(), EngineError> {
        self.checked_pages(pages)?;

        // 1. Calculate the new crop box for every page before modifying anything.
        let mut planned = Vec::with_capacity(pages.len());
        for &page in pages {
            let bounds = self.load_page(page)?.bounds()?;
            let new_x0 = bounds.x0 + margins.left;
            let new_y0 = bounds.y0 + margins.top;
            let new_x1 = bounds.x1 - margins.right;
            let new_y1 = bounds.y1 - margins.bottom;

            if (new_x1 - new_x0) < MIN_CROPPED_SIDE_PT || (new_y1 - new_y0) < MIN_CROPPED_SIDE_PT {
                return Err(EngineError::InvalidEdit("the crop leaves too little of a page"));
            }

            let (page_object, inverse) = self.page_to_pdf(page)?;
            let p1 = Point::new(new_x0, new_y0).transform(&inverse);
            let p2 = Point::new(new_x1, new_y0).transform(&inverse);
            let p3 = Point::new(new_x0, new_y1).transform(&inverse);
            let p4 = Point::new(new_x1, new_y1).transform(&inverse);

            let pdf_x0 = p1.x.min(p2.x).min(p3.x).min(p4.x);
            let pdf_x1 = p1.x.max(p2.x).max(p3.x).max(p4.x);
            let pdf_y0 = p1.y.min(p2.y).min(p3.y).min(p4.y);
            let pdf_y1 = p1.y.max(p2.y).max(p3.y).max(p4.y);

            let original_box = page_object.get_dict("CropBox")?;
            planned.push(PlannedCrop {
                page,
                new_box: [pdf_x0, pdf_y0, pdf_x1, pdf_y1],
                original_box,
            });
        }

        // 2. Write the new /CropBox for each page. If writing fails midway, rollback.
        for (done, item) in planned.iter().enumerate() {
            let res = (|| -> Result<(), EngineError> {
                let (mut page_object, _) = self.page_to_pdf(item.page)?;
                let mut arr = self.doc.new_array()?;
                for &val in &item.new_box {
                    arr.array_push(PdfObject::new_real(val)?)?;
                }
                page_object.dict_put("CropBox", arr)?;
                Ok(())
            })();

            if let Err(err) = res {
                for previous in &planned[..=done] {
                    if let Ok((mut page_object, _)) = self.page_to_pdf(previous.page) {
                        match &previous.original_box {
                            Some(orig) => {
                                let _ = page_object.dict_put("CropBox", orig.clone());
                            }
                            None => {
                                let _ = page_object.dict_delete("CropBox");
                            }
                        }
                    }
                }
                return Err(err);
            }
        }

        // 3. Invalidate OCR results on cropped pages and clear text cache.
        self.ocr_turned(pages);
        self.clear_text_cache();

        Ok(())
    }
}
