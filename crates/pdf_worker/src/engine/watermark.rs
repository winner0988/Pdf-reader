//! Watermarks (B3-04, docs/architecture/watermark.md): words or a picture drawn on the pages of a
//! copy of a document, as page content over what is there. Every reader shows page content, and
//! no click takes it off, as it would an annotation; a reader that copies the text finds the
//! words too. The document itself is not changed: the copy is the document saved to memory and
//! read again, as the privacy export's is.
//!
//! The words are set as a text box's are (ADR 0018), in a subset of the font the worker carries
//! for Chinese, Droid Sans Fallback, which has Latin letters too: no font is read from the
//! computer. A picture is a stamp picture (B2-08): the pixels of a PNG or a JPEG, nothing else.

use std::io::Write;

use ipc_contract::types::{Watermark, WatermarkContent, WatermarkPages, WatermarkPlace};
use ipc_contract::validate::{Validate, stamp_png_size};
use mupdf::pdf::{PdfObject, PdfWriteOptions};
use mupdf::{Buffer, Image, Matrix};

use super::page_overlay::{attach_form_to_page, droid_sans_line_form};
use super::{EngineError, PdfDocument, write_limited};

/// The name the watermark has in the resources of a page: one no document has.
const NAME: &str = "PdfReaderWatermark";
/// How much of the page a watermark may take, across it (or along its diagonal): one that would
/// take more is made smaller, never larger.
const FIT: f32 = 0.9;
/// How far from the top or the bottom of the page a watermark there is, in points.
const MARGIN_PT: f32 = 36.0;

impl PdfDocument {
    /// Writes to `out` a copy of the document, edits included, with `watermark` on its pages
    /// (B3-04). `picture` is the stamp picture (a PNG) of a picture watermark, and none for words.
    /// The open document is not touched. An encrypted document is refused, as for the privacy
    /// export: the worker keeps no password, and the copy could not be encrypted again. The
    /// signatures of a signed one do not hold in the copy, whose pages are not those signed.
    pub fn watermark_copy(
        &self,
        watermark: &Watermark,
        picture: Option<&[u8]>,
        out: &mut impl Write,
    ) -> Result<u64, EngineError> {
        watermark
            .validate()
            .map_err(|_| EngineError::InvalidEdit("the watermark"))?;
        if self.is_encrypted() {
            return Err(EngineError::EncryptedCopy);
        }
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut bytes = Vec::new();
        write_limited(&self.doc, &mut bytes, options)?;
        let mut copy = PdfDocument::from_bytes(&bytes)?;
        drop(bytes);
        let (mark, width, height) = copy.watermark_form(watermark, picture)?;
        for index in 0..copy.page_count()? {
            let wanted = match watermark.pages {
                WatermarkPages::All => true,
                WatermarkPages::First => index == 0,
                WatermarkPages::AllButFirst => index > 0,
            };
            if wanted {
                copy.put_watermark(index, &mark, (width, height), watermark)?;
            }
        }
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        options.set_compress(true);
        write_limited(&copy.doc, out, options)
    }

    /// The form a page draws to show `watermark`, and how wide and how tall it is, in points: the
    /// words or the picture, as much of them as the opacity lets show.
    fn watermark_form(
        &mut self,
        watermark: &Watermark,
        picture: Option<&[u8]>,
    ) -> Result<(PdfObject, f32, f32), EngineError> {
        let (inner, width, height) = match (&watermark.content, picture) {
            (WatermarkContent::Text { text, size, color }, None) => {
                droid_sans_line_form(&mut self.doc, text, *size, *color)?
            }
            (WatermarkContent::Picture { width_pt }, Some(png)) => {
                let (pixels_wide, pixels_high) = stamp_png_size(png)
                    .map_err(|_| EngineError::InvalidEdit("not a stamp picture"))?;
                let pixmap = Image::from_bytes(png)?.to_pixmap()?;
                let image = self.doc.add_image(&Image::from_pixmap(&pixmap)?)?;
                let width = *width_pt;
                let height = width * pixels_high as f32 / pixels_wide as f32;
                let form = self.doc.new_object_from_str(&format!(
                    "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
                     /Resources << /XObject << /I {} 0 R >> >> >>",
                    image.as_indirect()?,
                ))?;
                let content = format!("q {width:.3} 0 0 {height:.3} 0 0 cm /I Do Q");
                let stream = self.doc.add_stream(
                    &Buffer::from_bytes(content.as_bytes())?,
                    Some(&form),
                    false,
                )?;
                (stream, width, height)
            }
            _ => {
                return Err(EngineError::InvalidEdit(
                    "a picture watermark needs its picture, and words none",
                ));
            }
        };
        let opacity = f32::from(watermark.opacity) / 100.0;
        let form = self.doc.new_object_from_str(&format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {width:.3} {height:.3}] \
             /Resources << /ExtGState << /G0 << /Type /ExtGState /ca {opacity:.3} /CA {opacity:.3} >> >> \
             /XObject << /W {} 0 R >> >> >>",
            inner.as_indirect()?,
        ))?;
        let stream =
            self.doc
                .add_stream(&Buffer::from_bytes(b"/G0 gs /W Do")?, Some(&form), false)?;
        Ok((stream, width, height))
    }

    /// Draws `mark`, `size` points large, on page `index` as `watermark` says: in the middle, at
    /// the top or at the bottom of the page as it is shown, level or along its diagonal, and made
    /// smaller if it would not fit. The page's own content is drawn first, with its graphics state
    /// put back after it, so that nothing it leaves set changes the watermark.
    fn put_watermark(
        &mut self,
        index: u32,
        mark: &PdfObject,
        (width, height): (f32, f32),
        watermark: &Watermark,
    ) -> Result<(), EngineError> {
        let (mut page, inverse) = self.page_to_pdf(index)?;
        let bounds = self.load_page(index)?.bounds()?;
        let (page_width, page_height) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        if !(page_width > 0.0 && page_height > 0.0) {
            return Ok(());
        }
        // Page space goes down from the top left corner of the page as it is shown.
        let angle = if watermark.diagonal {
            page_height.atan2(page_width)
        } else {
            0.0
        };
        let room = if watermark.diagonal {
            page_width.hypot(page_height)
        } else {
            page_width
        } * FIT;
        let scale = (room / width).min(1.0);
        // Half of how tall the turned watermark is.
        let reach = (width * scale / 2.0) * angle.sin() + (height * scale / 2.0) * angle.cos();
        let x = bounds.x0 + page_width / 2.0;
        let y = match watermark.place {
            WatermarkPlace::Center => bounds.y0 + page_height / 2.0,
            WatermarkPlace::Top => bounds.y0 + MARGIN_PT + reach,
            WatermarkPlace::Bottom => bounds.y1 - MARGIN_PT - reach,
        };
        // The form's own space goes up from its bottom left corner: centred on the origin, turned
        // upside down into page space, turned to the diagonal (up to the right as it is seen),
        // moved to its place, and taken into the PDF's space.
        let mut matrix = Matrix::new_translate(-width / 2.0, -height / 2.0);
        matrix.concat(Matrix::new_scale(scale, -scale));
        matrix.concat(Matrix::new_rotate(-angle.to_degrees()));
        matrix.concat(Matrix::new_translate(x, y));
        matrix.concat(inverse);

        attach_form_to_page(&mut self.doc, &mut page, NAME, mark, &matrix)
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::Rgb;

    use super::*;
    use crate::engine::{RenderedPage, prepare_stamp_picture};

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    const RED: Rgb = Rgb { r: 220, g: 0, b: 0 };

    fn words(text: &str, opacity: u8) -> Watermark {
        Watermark {
            content: WatermarkContent::Text {
                text: text.to_owned(),
                size: 48.0,
                color: RED,
            },
            opacity,
            diagonal: false,
            place: WatermarkPlace::Center,
            pages: WatermarkPages::All,
        }
    }

    fn copy_of(doc: &PdfDocument, watermark: &Watermark, picture: Option<&[u8]>) -> PdfDocument {
        let mut out = Vec::new();
        let bytes = doc
            .watermark_copy(watermark, picture, &mut out)
            .expect("a copy");
        assert_eq!(bytes, out.len() as u64);
        PdfDocument::from_bytes(&out).expect("the copy opens")
    }

    fn text_of(doc: &PdfDocument, page: u32) -> String {
        doc.page_text(page, 100_000)
            .unwrap()
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Where the page is red (the watermark): x and y of each such pixel.
    fn red(page: &RenderedPage) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for y in 0..page.height as usize {
            for x in 0..page.width as usize {
                let at = (y * page.width as usize + x) * 4;
                let [r, g, b] =
                    [page.rgba[at], page.rgba[at + 1], page.rgba[at + 2]].map(i32::from);
                if r > g + 40 && r > b + 40 {
                    found.push((x, y));
                }
            }
        }
        found
    }

    fn bounds(points: &[(usize, usize)]) -> (usize, usize, usize, usize) {
        let xs = points.iter().map(|p| p.0);
        let ys = points.iter().map(|p| p.1);
        (
            xs.clone().min().expect("some"),
            ys.clone().min().expect("some"),
            xs.max().expect("some"),
            ys.max().expect("some"),
        )
    }

    #[test]
    fn words_go_on_every_page_as_its_content_and_the_document_stays_as_it_was() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();
        let copy = copy_of(&doc, &words("機密 CONFIDENTIAL", 100), None);
        assert_eq!(copy.page_count().unwrap(), 10);
        for page in [0, 4, 9] {
            let text = text_of(&copy, page);
            assert!(
                text.contains("CONFIDENTIAL") && text.contains("機密"),
                "{page}: {text}"
            );
            // In the middle of the page, and level.
            let rendered = copy.render(page, 0.5, 0).unwrap();
            let (x0, y0, x1, y1) = bounds(&red(&rendered));
            let (width, height) = (rendered.width as usize, rendered.height as usize);
            assert!(x0 < width / 2 && x1 > width / 2 && y0 < height / 2 && y1 > height / 2);
            assert!(x1 - x0 > y1 - y0, "level");
        }
        // The document itself has no watermark, and the copy has no annotation.
        assert!(!text_of(&doc, 0).contains("CONFIDENTIAL"));
        assert!(red(&doc.render(0, 0.5, 0).unwrap()).is_empty());
        assert!(copy.page_annotations(0).unwrap().is_empty());
    }

    #[test]
    fn the_first_page_or_all_but_the_first() {
        let doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).unwrap();
        let first = Watermark {
            pages: WatermarkPages::First,
            ..words("DRAFT", 100)
        };
        let copy = copy_of(&doc, &first, None);
        assert!(text_of(&copy, 0).contains("DRAFT"));
        assert!(!text_of(&copy, 1).contains("DRAFT"));
        let rest = Watermark {
            pages: WatermarkPages::AllButFirst,
            ..words("DRAFT", 100)
        };
        let copy = copy_of(&doc, &rest, None);
        assert!(!text_of(&copy, 0).contains("DRAFT"));
        assert!(text_of(&copy, 1).contains("DRAFT") && text_of(&copy, 9).contains("DRAFT"));
    }

    #[test]
    fn a_diagonal_watermark_runs_up_to_the_right_and_one_at_the_top_or_bottom_is_there() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();
        let diagonal = Watermark {
            diagonal: true,
            ..words("CONFIDENTIAL COPY", 100)
        };
        let rendered = copy_of(&doc, &diagonal, None).render(0, 0.5, 0).unwrap();
        let points = red(&rendered);
        let middle = rendered.width as usize / 2;
        let average =
            |side: &[&(usize, usize)]| side.iter().map(|p| p.1).sum::<usize>() / side.len();
        let left: Vec<_> = points.iter().filter(|p| p.0 < middle).collect();
        let right: Vec<_> = points.iter().filter(|p| p.0 >= middle).collect();
        // Higher on the right: a smaller y, as the picture goes down.
        assert!(
            average(&right) + 20 < average(&left),
            "{} {}",
            average(&right),
            average(&left)
        );

        for (place, upper) in [(WatermarkPlace::Top, true), (WatermarkPlace::Bottom, false)] {
            let at = Watermark {
                place,
                ..words("DRAFT", 100)
            };
            let rendered = copy_of(&doc, &at, None).render(0, 0.5, 0).unwrap();
            let (_, y0, _, y1) = bounds(&red(&rendered));
            let height = rendered.height as usize;
            if upper {
                assert!(y1 < height / 4, "{y1} of {height}");
            } else {
                assert!(y0 > height * 3 / 4, "{y0} of {height}");
            }
        }
    }

    #[test]
    fn on_a_turned_page_the_words_are_level_as_the_page_is_shown() {
        let doc = PdfDocument::from_bytes(&corpus("benign/rotated-page.pdf")).unwrap();
        let copy = copy_of(&doc, &words("LEVEL WORDS", 100), None);
        for page in 0..copy.page_count().unwrap() {
            let rendered = copy.render(page, 0.5, 0).unwrap();
            let points = red(&rendered);
            assert!(!points.is_empty(), "{page}");
            let (x0, y0, x1, y1) = bounds(&points);
            assert!(x1 - x0 > 2 * (y1 - y0), "page {page}: level as it is shown");
        }
    }

    #[test]
    fn a_faint_watermark_lets_the_page_show_through() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();
        // The least green of the red pixels: white shows through a faint watermark.
        let palest = |opacity: u8| {
            let rendered = copy_of(&doc, &words("FAINT", opacity), None)
                .render(0, 1.0, 0)
                .unwrap();
            red(&rendered)
                .iter()
                .map(|&(x, y)| rendered.rgba[(y * rendered.width as usize + x) * 4 + 1])
                .min()
                .expect("red")
        };
        assert!(palest(100) < 60, "{}", palest(100));
        assert!(palest(30) > 150, "{}", palest(30));
    }

    #[test]
    fn a_picture_watermark_is_the_picture_and_keeps_its_shape() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();
        // A picture of 64 x 32 pixels whose left half is red: twice as wide as it is tall.
        let picture = prepare_stamp_picture(&corpus("images/stamp-exif.jpg")).unwrap();
        let watermark = Watermark {
            content: WatermarkContent::Picture { width_pt: 200.0 },
            ..words("unused", 100)
        };
        let copy = copy_of(&doc, &watermark, Some(&picture.png));
        let rendered = copy.render(0, 1.0, 0).unwrap();
        // Where the page changed: the picture, 200 by 100 points, in the middle.
        let before = doc.render(0, 1.0, 0).unwrap();
        let mut ink = Vec::new();
        for y in 0..rendered.height as usize {
            for x in 0..rendered.width as usize {
                let at = (y * rendered.width as usize + x) * 4;
                let changed = (0..3).any(|channel| {
                    rendered.rgba[at + channel].abs_diff(before.rgba[at + channel]) > 40
                });
                if changed {
                    ink.push((x, y));
                }
            }
        }
        let (x0, y0, x1, y1) = bounds(&ink);
        let (width, height) = (rendered.width as usize, rendered.height as usize);
        assert!((195..=205).contains(&(x1 - x0)), "{x0}..{x1}");
        assert!((95..=105).contains(&(y1 - y0)), "{y0}..{y1}");
        assert!(x0.abs_diff(width - x1) <= 2 && y0.abs_diff(height - y1) <= 2);
        // And it is the picture: red is in it.
        assert!(!red(&rendered).is_empty());
    }

    #[test]
    fn what_cannot_be_a_watermark_is_refused_and_writes_nothing() {
        let doc = PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).unwrap();
        let picture = prepare_stamp_picture(&corpus("images/stamp-exif.jpg")).unwrap();
        let mut out = Vec::new();
        for (watermark, png) in [
            // A picture watermark without its picture, words with one.
            (
                Watermark {
                    content: WatermarkContent::Picture { width_pt: 100.0 },
                    ..words("x", 100)
                },
                None,
            ),
            (words("DRAFT", 100), Some(picture.png.as_slice())),
            // Nothing to see, nothing to say, and more than one line.
            (words("DRAFT", 0), None),
            (words("   ", 100), None),
            (words("one\ntwo", 100), None),
        ] {
            assert!(
                doc.watermark_copy(&watermark, png, &mut out).is_err(),
                "{watermark:?}"
            );
            assert!(out.is_empty());
        }
        // An encrypted document has no copy that could be encrypted again.
        let encrypted =
            PdfDocument::open(&corpus("benign/encrypted-rc4-40.pdf"), Some("user")).unwrap();
        assert!(matches!(
            encrypted.watermark_copy(&words("DRAFT", 100), None, &mut out),
            Err(EngineError::EncryptedCopy)
        ));
    }
}
