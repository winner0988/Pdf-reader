//! Pages that are only a picture (B2-10, ADR 0015, docs/architecture/ocr.md): finding them,
//! drawing them for the recogniser ([`crate::ocr_worker`]), and keeping what it read, to answer
//! for them as for a page's own text.
//!
//! A page is kept track of by the number of its object in the file, not by its index: pages are
//! deleted, moved and inserted, and what was read from a page stays true. Turning a page makes
//! what was read from it wrong (it was read upright), so that drops it, and anything still being
//! read for the document ([`PdfDocument::ocr_epoch`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use ipc_contract::types::{PageText as TextLayer, Point, Quad, TextLine};
use mupdf::text_page::TextBlockType;
use mupdf::{Colorspace, Matrix, TextPageFlags};

use super::{EngineError, PdfDocument};
use crate::ocr::{MAX_OCR_PIXELS, MAX_OCR_SIDE_PX};
use crate::ocr_worker::{Geometry, Picture};

/// The resolution pages are read at, in pixels an inch: sharp enough for ordinary text and the
/// strokes of Chinese, without a picture or a time far larger than that needs.
const OCR_DPI: f64 = 200.0;

/// A page is drawn with at most this many pixels (a large page is drawn smaller): the picture
/// the recogniser takes at most is bigger still, so the limit on the recogniser is never what
/// stops a page.
const OCR_DRAW_PIXELS: f64 = 16_000_000.0;

/// A page is a scan when images cover at least this much of it and it has no text.
const SCAN_COVERAGE: f32 = 0.5;

/// Epochs are unique in the process, so that what was read for a document that was replaced
/// (undo opens it again) is never taken for what belongs to the new one.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

fn new_epoch() -> u64 {
    NEXT_EPOCH.fetch_add(1, Ordering::Relaxed)
}

/// What is known about a page, as far as recognising it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrKnown {
    /// Its text was read.
    Recognised,
    /// It has text of its own or is not mostly a picture.
    NotScan,
    /// It is queued or being read.
    Waiting,
    /// Reading it failed or took too long.
    Failed,
    /// Not looked at yet.
    Unknown,
}

/// What was read from a document's pages and what is being read.
#[derive(Debug)]
pub(super) struct OcrPages {
    text: HashMap<i32, TextLayer>,
    not_scans: HashSet<i32>,
    failed: HashSet<i32>,
    waiting: HashSet<i32>,
    epoch: u64,
}

impl OcrPages {
    pub(super) fn new() -> Self {
        Self {
            text: HashMap::new(),
            not_scans: HashSet::new(),
            failed: HashSet::new(),
            waiting: HashSet::new(),
            epoch: new_epoch(),
        }
    }

    /// Takes the pages `keys` out of everything.
    fn forget(&mut self, keys: impl IntoIterator<Item = i32>) {
        for key in keys {
            self.text.remove(&key);
            self.not_scans.remove(&key);
            self.failed.remove(&key);
            self.waiting.remove(&key);
        }
    }
}

impl PdfDocument {
    /// The key of page `index`: the number of its object, which stays its own through edits.
    fn ocr_key(&self, index: u32) -> Result<i32, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self.doc.find_page(index as i32)?.as_indirect()?)
    }

    /// The index of the page whose key is `key`, if it is still in the document.
    fn ocr_index(&self, key: i32) -> Option<u32> {
        let object = self.doc.new_indirect(key, 0).ok()?;
        let index = self.doc.lookup_page_number(&object).ok()?;
        let index = u32::try_from(index).ok()?;
        // The lookup walks the page tree: a page that is not in it gives nothing sensible.
        (self.ocr_key(index).ok()? == key).then_some(index)
    }

    /// Changes whenever what is being read for the pages is no longer about them: a page was
    /// turned, or the document is another one.
    pub fn ocr_epoch(&self) -> u64 {
        self.ocr.epoch
    }

    /// What recognising has found out about page `index` so far.
    pub fn ocr_known(&self, index: u32) -> Result<OcrKnown, EngineError> {
        let key = self.ocr_key(index)?;
        Ok(if self.ocr.text.contains_key(&key) {
            OcrKnown::Recognised
        } else if self.ocr.not_scans.contains(&key) {
            OcrKnown::NotScan
        } else if self.ocr.waiting.contains(&key) {
            OcrKnown::Waiting
        } else if self.ocr.failed.contains(&key) {
            OcrKnown::Failed
        } else {
            OcrKnown::Unknown
        })
    }

    /// Whether page `index` is a scan: it has no text and images cover most of it. A page that
    /// is not one is remembered as such.
    pub fn ocr_is_scan(&mut self, index: u32) -> Result<bool, EngineError> {
        let key = self.ocr_key(index)?;
        let page = self.load_page(index)?;
        let bounds = page.bounds()?;
        let area = (bounds.x1 - bounds.x0).max(0.0) * (bounds.y1 - bounds.y0).max(0.0);
        let text_page = page.to_text_page(TextPageFlags::PRESERVE_IMAGES)?;
        let (mut has_text, mut covered) = (false, 0.0_f32);
        for block in text_page.blocks() {
            if block.r#type() == TextBlockType::Image {
                let b = block.bounds();
                let width = (b.x1.min(bounds.x1) - b.x0.max(bounds.x0)).max(0.0);
                let height = (b.y1.min(bounds.y1) - b.y0.max(bounds.y0)).max(0.0);
                covered += width * height;
            } else {
                has_text |= block.lines().any(|line| {
                    line.chars().any(|c| {
                        c.char()
                            .is_some_and(|c| !c.is_whitespace() && !c.is_control())
                    })
                });
            }
        }
        let scan = !has_text && area > 0.0 && covered >= area * SCAN_COVERAGE;
        if !scan {
            self.ocr.not_scans.insert(key);
        }
        Ok(scan)
    }

    /// Draws page `index` for the recogniser: grey, at [`OCR_DPI`] or less for a page too large
    /// for that, without the annotations and form fields (they are not the page's scan).
    pub fn ocr_picture(&self, index: u32) -> Result<(Picture, Geometry, i32), EngineError> {
        let key = self.ocr_key(index)?;
        let page = self.load_page(index)?;
        let bounds = page.bounds()?;
        let (width_pt, height_pt) = (
            f64::from(bounds.x1 - bounds.x0),
            f64::from(bounds.y1 - bounds.y0),
        );
        if !(width_pt > 0.0 && height_pt > 0.0) {
            return Err(EngineError::NoArea);
        }
        let mut scale = OCR_DPI / 72.0;
        scale = scale
            .min(f64::from(MAX_OCR_SIDE_PX) / width_pt.max(height_pt))
            .min((OCR_DRAW_PIXELS / (width_pt * height_pt)).sqrt());
        let pixels = (width_pt * scale).ceil() * (height_pt * scale).ceil();
        if pixels < 1.0 || pixels > MAX_OCR_PIXELS as f64 {
            return Err(EngineError::NoArea);
        }
        let ctm = Matrix::new_scale(scale as f32, scale as f32);
        let pixmap = page.to_pixmap(&ctm, &Colorspace::device_gray(), false, false)?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let stride = usize::try_from(pixmap.stride()).unwrap_or(0);
        let mut grey = Vec::with_capacity(width as usize * height as usize);
        for row in pixmap.samples().chunks_exact(stride.max(1)) {
            grey.extend_from_slice(&row[..width as usize]);
        }
        let picture = Picture {
            grey,
            width,
            height,
            ppi: (scale * 72.0).round() as u32,
        };
        let geometry = Geometry {
            x0: bounds.x0,
            y0: bounds.y0,
            pixels_per_point: scale as f32,
        };
        Ok((picture, geometry, key))
    }
}

impl PdfDocument {
    /// Page `key` was queued for the recogniser.
    pub fn ocr_waiting(&mut self, key: i32) {
        self.ocr.waiting.insert(key);
    }

    /// The recogniser is told to stop: nothing is waiting any more.
    pub fn ocr_stopped(&mut self) {
        self.ocr.waiting.clear();
    }

    /// Takes what the recogniser made of page `key`, as it was when the pages were as in
    /// `epoch`: the text, or that it could not. Gives the page's index if the result is still
    /// about it, and nothing if the page was turned or deleted since.
    pub fn ocr_finish(&mut self, key: i32, epoch: u64, text: Option<TextLayer>) -> Option<u32> {
        self.ocr.waiting.remove(&key);
        if epoch != self.ocr.epoch {
            return None;
        }
        let index = self.ocr_index(key)?;
        match text {
            Some(text) => {
                self.ocr.text.insert(key, text);
            }
            None => {
                self.ocr.failed.insert(key);
            }
        }
        Some(index)
    }

    /// The text read from page `index`, when it has any.
    pub(super) fn ocr_text(&self, index: u32) -> Option<&TextLayer> {
        let key = self.ocr_key(index).ok()?;
        self.ocr
            .text
            .get(&key)
            .filter(|text| !text.lines.is_empty())
    }

    /// Pages `pages` were turned: what was read from them was read upright.
    pub(super) fn ocr_turned(&mut self, pages: &[u32]) {
        let keys: Vec<i32> = pages
            .iter()
            .filter_map(|&page| self.ocr_key(page).ok())
            .collect();
        self.ocr.forget(keys);
        self.ocr.epoch = new_epoch();
    }

    /// Pages that are gone: nothing is known about them.
    pub(super) fn ocr_forget(&mut self, keys: impl IntoIterator<Item = i32>) {
        self.ocr.forget(keys);
    }

    /// Page `index` was just made: nothing is known about it.
    pub(super) fn ocr_forget_page(&mut self, index: u32) {
        if let Ok(key) = self.ocr_key(index) {
            self.ocr.forget([key]);
        }
    }
}

/// The characters of a line of text and a box for each, from the line's box and the edges of its
/// characters: the other way round from how the line was made (`text_layer`).
pub(super) fn char_quads(line: &TextLine) -> Vec<(char, Quad)> {
    let q = &line.quad;
    let (dx, dy) = (q.ur.x - q.ul.x, q.ur.y - q.ul.y);
    let length = dx.hypot(dy);
    let (ux, uy) = if length > 1e-6 {
        (dx / length, dy / length)
    } else {
        (1.0, 0.0)
    };
    let (down_x, down_y) = (q.ll.x - q.ul.x, q.ll.y - q.ul.y);
    let at = |distance: f32| Point {
        x: q.ul.x + ux * distance,
        y: q.ul.y + uy * distance,
    };
    let shifted = |p: Point| Point {
        x: p.x + down_x,
        y: p.y + down_y,
    };
    line.text
        .chars()
        .zip(line.edges.windows(2))
        .map(|(c, edge)| {
            let (from, to) = (at(edge[0]), at(edge[1]));
            let quad = Quad {
                ul: from,
                ur: to,
                ll: shifted(from),
                lr: shifted(to),
            };
            (c, quad)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfDocument as MuPdfDocument};
    use mupdf::{Colorspace, Pixmap, Rect};

    use super::*;
    use crate::text_layer::TextLayerBuilder;

    /// A document of one page `size` points big with a white picture in each of `images`.
    fn page_with_pictures(size: (f32, f32), images: &[Rect]) -> PdfDocument {
        let mut doc = MuPdfDocument::new();
        let mut page = doc.new_page(size).expect("new page");
        let mut white =
            Pixmap::new_with_w_h(&Colorspace::device_gray(), 40, 40, false).expect("pixmap");
        white.clear_with(255).expect("clear");
        for rect in images {
            page.insert_image(
                &mut doc,
                *rect,
                PageImageSource::Pixmap(&white),
                InsertImageOptions::default(),
            )
            .expect("insert image");
        }
        drop(page);
        let mut bytes = Vec::new();
        doc.write_to(&mut bytes).expect("write");
        PdfDocument::from_bytes(&bytes).expect("open")
    }

    const LETTER: (f32, f32) = (612.0, 792.0);

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn a_page_is_a_scan_when_pictures_cover_half_of_it_and_it_has_no_text() {
        let mut full = page_with_pictures(LETTER, &[rect(0.0, 0.0, 612.0, 792.0)]);
        assert_eq!(full.ocr_known(0).unwrap(), OcrKnown::Unknown);
        assert!(full.ocr_is_scan(0).unwrap());
        // Not remembered as anything yet: it is only looked at.
        assert_eq!(full.ocr_known(0).unwrap(), OcrKnown::Unknown);

        // Two strips that together cover 60%.
        let mut strips = page_with_pictures(
            LETTER,
            &[rect(0.0, 0.0, 612.0, 237.6), rect(0.0, 237.6, 612.0, 475.2)],
        );
        assert!(strips.ocr_is_scan(0).unwrap());

        let mut small = page_with_pictures(LETTER, &[rect(0.0, 0.0, 612.0, 300.0)]);
        assert!(
            !small.ocr_is_scan(0).unwrap(),
            "38% is not most of the page"
        );
        assert_eq!(
            small.ocr_known(0).unwrap(),
            OcrKnown::NotScan,
            "and it is remembered"
        );

        let mut bare = page_with_pictures(LETTER, &[]);
        assert!(!bare.ocr_is_scan(0).unwrap());
    }

    #[test]
    fn a_page_with_text_is_not_a_scan() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus/benign/mixed-text-zh-en.pdf"),
        )
        .expect("corpus");
        let mut document = PdfDocument::from_bytes(&bytes).expect("open");
        assert!(!document.ocr_is_scan(0).unwrap());
        assert!(matches!(
            document.ocr_is_scan(3),
            Err(EngineError::PageOutOfRange(3))
        ));
    }

    #[test]
    fn a_page_is_drawn_at_200_dpi_unless_that_is_too_large() {
        let letter = page_with_pictures(LETTER, &[]);
        let (picture, geometry, _) = letter.ocr_picture(0).unwrap();
        assert_eq!(
            (picture.width, picture.height, picture.ppi),
            (1700, 2200, 200)
        );
        assert_eq!(picture.grey.len(), 1700 * 2200);
        assert!(
            picture.grey.iter().all(|&pixel| pixel == 255),
            "a white page"
        );
        assert_eq!((geometry.x0, geometry.y0), (0.0, 0.0));
        assert!((geometry.pixels_per_point - 200.0 / 72.0).abs() < 1e-6);

        // A poster 100 inches on a side: drawn smaller, within the limits.
        let poster = page_with_pictures((7200.0, 7200.0), &[]);
        let (picture, _, _) = poster.ocr_picture(0).unwrap();
        assert!(u64::from(picture.width) * u64::from(picture.height) <= 16_000_000);
        assert!(picture.width.max(picture.height) <= MAX_OCR_SIDE_PX);
        assert!(picture.ppi < 200);
        // A very long page: the side limit holds.
        let strip = page_with_pictures((20.0, 14_000.0), &[]);
        let (picture, _, _) = strip.ocr_picture(0).unwrap();
        assert!(picture.height <= MAX_OCR_SIDE_PX, "{}", picture.height);
    }

    fn text(line: &str) -> TextLayer {
        let mut builder = TextLayerBuilder::new(100);
        let quad = |i: usize| {
            let x = 10.0 + 5.0 * i as f32;
            Quad {
                ul: Point { x, y: 20.0 },
                ur: Point {
                    x: x + 5.0,
                    y: 20.0,
                },
                ll: Point { x, y: 30.0 },
                lr: Point {
                    x: x + 5.0,
                    y: 30.0,
                },
            }
        };
        builder.push_line(line.chars().enumerate().map(|(i, c)| (c, quad(i))));
        let mut text = builder.finish();
        text.recognised = true;
        text
    }

    #[test]
    fn the_boxes_of_a_line_come_back_from_its_edges() {
        let page = text("Hi 中文");
        let boxes = char_quads(&page.lines[0]);
        assert_eq!(boxes.len(), 5);
        for (i, (c, quad)) in boxes.iter().enumerate() {
            assert_eq!(*c, "Hi 中文".chars().nth(i).unwrap());
            let x = 10.0 + 5.0 * i as f32;
            assert!(
                (quad.ul.x - x).abs() < 0.01 && (quad.ur.x - (x + 5.0)).abs() < 0.01,
                "{i}: {quad:?}"
            );
            assert!(
                (quad.ul.y - 20.0).abs() < 0.01 && (quad.ll.y - 30.0).abs() < 0.01,
                "{i}: {quad:?}"
            );
        }
    }

    #[test]
    fn what_was_read_belongs_to_the_page_not_its_place() {
        let mut doc = {
            let mut doc = MuPdfDocument::new();
            for _ in 0..3 {
                doc.new_page(LETTER).expect("page");
            }
            let mut bytes = Vec::new();
            doc.write_to(&mut bytes).expect("write");
            PdfDocument::from_bytes(&bytes).expect("open")
        };
        let key = |doc: &PdfDocument, index| doc.ocr_key(index).unwrap();
        let (zero, first, second) = (key(&doc, 0), key(&doc, 1), key(&doc, 2));
        let epoch = doc.ocr_epoch();
        for page in [zero, first, second] {
            doc.ocr_waiting(page);
        }
        assert_eq!(doc.ocr_known(1).unwrap(), OcrKnown::Waiting);
        // The last page is finished at index 2; then the first page is deleted: it is at 1.
        assert_eq!(doc.ocr_finish(second, epoch, Some(text("later"))), Some(2));
        doc.delete_pages(&[0]).unwrap();
        assert_eq!(doc.ocr_known(1).unwrap(), OcrKnown::Recognised);
        assert_eq!(doc.page_text(1, 1000).unwrap().lines[0].text, "later");
        assert!(doc.page_text(1, 1000).unwrap().recognised);
        // A result for a page that is gone is dropped; one that failed is remembered as such.
        assert_eq!(doc.ocr_finish(zero, epoch, Some(text("gone"))), None);
        assert_eq!(doc.ocr_finish(first, epoch, None), Some(0));
        assert_eq!(doc.ocr_known(0).unwrap(), OcrKnown::Failed);
        // Turning a page drops what was read from it, and what is still being read.
        doc.ocr_waiting(second);
        doc.rotate_pages(&[1], 90).unwrap();
        assert_eq!(doc.ocr_known(1).unwrap(), OcrKnown::Unknown);
        assert_ne!(doc.ocr_epoch(), epoch);
        assert_eq!(doc.ocr_finish(second, epoch, Some(text("old"))), None);
        assert!(doc.page_text(1, 1000).unwrap().lines.is_empty());
        // A page that is made new knows nothing.
        doc.insert_blank_page(1, 0).unwrap();
        assert_eq!(doc.ocr_known(1).unwrap(), OcrKnown::Unknown);
    }
}
