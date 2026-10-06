//! Pictures for custom stamps (B2-08): a PNG or JPEG file is made into the PNG a stamp is
//! made of, and such a PNG is made into a stamp annotation.
//!
//! The picture is untrusted input, decoded here in the sandboxed worker and nowhere else: only
//! PNG and JPEG are accepted (by their first bytes), each side and the pixel count are bounded
//! before any pixel is decoded, and the file is read no further than its limit. What comes out
//! is the pixels alone, encoded again: the decoder keeps nothing of the file metadata (EXIF,
//! the GPS position, camera and time, text chunks, color profiles), and so the PDF has none.

use ipc_contract::limits::{
    MAX_STAMP_PNG_BYTES, MAX_STAMP_SIDE_PX, MAX_STAMP_SOURCE_BYTES, MAX_STAMP_SOURCE_PIXELS,
    MAX_STAMP_SOURCE_SIDE_PX,
};
use ipc_contract::types::Rect;
use ipc_contract::validate::{JPEG_SIGNATURE, PNG_SIGNATURE, stamp_png_size};
use mupdf::pdf::PdfObject;
use mupdf::{Buffer, Image, ImageFormat, Pixmap};

use super::annotations::binding_rect;
use super::{EngineError, PdfDocument};

/// A stamp picture: the PNG file and its size in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampPicture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Makes the PNG or JPEG file `bytes` into a stamp picture (see the module).
pub fn prepare_stamp_picture(bytes: &[u8]) -> Result<StampPicture, EngineError> {
    if bytes.len() > MAX_STAMP_SOURCE_BYTES {
        return Err(EngineError::PictureTooLarge);
    }
    if !(bytes.starts_with(&PNG_SIGNATURE) || bytes.starts_with(&JPEG_SIGNATURE)) {
        return Err(EngineError::InvalidPicture("not a PNG or JPEG picture"));
    }
    // MuPDF reads the header; the pixels are decoded below, once the size is known to be fine.
    let image = Image::from_bytes(bytes)
        .map_err(|_| EngineError::InvalidPicture("the picture cannot be read"))?;
    let (width, height) = (image.width(), image.height());
    if width == 0
        || height == 0
        || width > MAX_STAMP_SOURCE_SIDE_PX
        || height > MAX_STAMP_SOURCE_SIDE_PX
        || u64::from(width) * u64::from(height) > MAX_STAMP_SOURCE_PIXELS
    {
        return Err(EngineError::PictureTooLarge);
    }
    let mut pixmap = image
        .to_pixmap()
        .map_err(|_| EngineError::InvalidPicture("the picture cannot be read"))?;
    // The PNG writer takes gray and RGB, with or without see-through, and nothing else.
    let usable = pixmap
        .color_space()
        .is_some_and(|space| space.is_gray() || space.is_rgb());
    if !usable {
        return Err(EngineError::InvalidPicture(
            "the picture has unusable colors",
        ));
    }
    let longer = pixmap.width().max(pixmap.height());
    let mut shrinks = 0;
    while (longer >> shrinks) > MAX_STAMP_SIDE_PX {
        shrinks += 1;
    }
    if shrinks > 0 {
        pixmap.shrink(shrinks)?;
    }
    loop {
        let png = encode(&pixmap)?;
        if png.len() <= MAX_STAMP_PNG_BYTES {
            return Ok(StampPicture {
                png,
                width: pixmap.width(),
                height: pixmap.height(),
            });
        }
        // Too many bytes for what it shows: a picture of noise. Halve it again.
        if pixmap.width().max(pixmap.height()) < 16 {
            return Err(EngineError::PictureTooLarge);
        }
        pixmap.shrink(1)?;
    }
}

fn encode(pixmap: &Pixmap) -> Result<Vec<u8>, EngineError> {
    let mut png = Vec::new();
    pixmap.write_to(&mut png, ImageFormat::PNG)?;
    Ok(png)
}

impl PdfDocument {
    /// Puts the stamp picture `png` (a file `prepare_stamp_picture` made) over `rect` (page
    /// space) on page `index`: a `Stamp` annotation whose appearance draws the picture, stretched
    /// to `rect`. The picture is checked again and decoded again here, so that only its pixels
    /// are put in the document, whatever the bytes say.
    pub fn add_image_stamp(
        &mut self,
        index: u32,
        rect: Rect,
        png: &[u8],
    ) -> Result<(), EngineError> {
        stamp_png_size(png).map_err(|_| EngineError::InvalidEdit("not a stamp picture"))?;
        let pixmap = Image::from_bytes(png)?.to_pixmap()?;
        let picture = Image::from_pixmap(&pixmap)?;
        let image_object = self.doc.add_image(&picture)?;

        // The appearance: a form over the unit square, which a reader maps onto the rectangle,
        // that draws the picture over it.
        let mut images = self.doc.new_dict()?;
        images.dict_put("Picture", image_object)?;
        let mut resources = self.doc.new_dict()?;
        resources.dict_put("XObject", images)?;
        let mut bounds = self.doc.new_array()?;
        for corner in [0, 0, 1, 1] {
            bounds.array_push(PdfObject::new_int(corner)?)?;
        }
        let mut form = self.doc.new_dict()?;
        form.dict_put("Type", PdfObject::new_name("XObject")?)?;
        form.dict_put("Subtype", PdfObject::new_name("Form")?)?;
        form.dict_put("BBox", bounds)?;
        form.dict_put("Resources", resources)?;
        let content = Buffer::from_bytes(b"q /Picture Do Q")?;
        let stream = self.doc.add_stream(&content, Some(&form), false)?;
        let mut appearance = self.doc.new_dict()?;
        appearance.dict_put("N", stream)?;

        let mut page = self.pdf_page(index)?;
        // A name that is none of the standard stamps: MuPDF leaves the appearance as it is.
        let mut annotation = page.add_stamp_annotation(binding_rect(rect), "Picture")?;
        annotation.object().dict_put("AP", appearance)?;
        annotation.set_rect(binding_rect(rect))?;
        annotation.update()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::AnnotationKind;
    use mupdf::Colorspace;

    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// The types of the chunks of a PNG file, in order.
    fn chunk_types(png: &[u8]) -> Vec<String> {
        let mut types = Vec::new();
        let mut at = 8;
        while at + 8 <= png.len() {
            let length = u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
            types.push(String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned());
            at += 12 + length as usize;
        }
        types
    }

    /// The color (RGBA) of pixel (x, y) of a PNG file.
    fn pixel(png: &[u8], x: i32, y: i32) -> Vec<u8> {
        let pixmap = Image::from_bytes(png).unwrap().to_pixmap().unwrap();
        pixmap.pixel(x, y).unwrap().components().to_vec()
    }

    const PRIVATE: [&[u8]; 6] = [
        b"Canon",
        b"Canon EOS 5D Mark IV",
        b"2023:07:04 12:34:56",
        b"Jane Q. Private-Author",
        b"Exif",
        b"eXIf",
    ];

    #[test]
    fn a_jpeg_with_exif_becomes_a_png_of_its_pixels_alone() {
        let jpeg = corpus("images/stamp-exif.jpg");
        // The sample does carry what a camera writes.
        for private in [&PRIVATE[1], &PRIVATE[2], &PRIVATE[4]] {
            assert!(
                contains(&jpeg, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        let picture = prepare_stamp_picture(&jpeg).expect("prepared");
        assert_eq!((picture.width, picture.height), (64, 32));
        assert_eq!(stamp_png_size(&picture.png), Ok((64, 32)));
        for private in PRIVATE {
            assert!(
                !contains(&picture.png, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // Nothing but the header, the pixels and the end (and the size it was meant to print at).
        assert!(
            chunk_types(&picture.png)
                .iter()
                .all(|kind| ["IHDR", "pHYs", "IDAT", "IEND"].contains(&kind.as_str())),
            "{:?}",
            chunk_types(&picture.png)
        );
        // The picture: red on top, blue below, a green stripe through it.
        let near = |pixel: Vec<u8>, wanted: [u8; 3]| {
            pixel
                .iter()
                .zip(wanted)
                .all(|(have, want)| have.abs_diff(want) < 16)
        };
        assert!(near(pixel(&picture.png, 4, 4), [200, 40, 40]));
        assert!(near(pixel(&picture.png, 50, 24), [40, 60, 200]));
        assert!(near(pixel(&picture.png, 30, 24), [40, 160, 70]));
    }

    #[test]
    fn a_png_keeps_its_see_through_corner_and_none_of_its_chunks() {
        let png = corpus("images/stamp-metadata.png");
        for kind in ["tEXt", "eXIf"] {
            assert!(chunk_types(&png).iter().any(|found| found == kind));
        }
        let picture = prepare_stamp_picture(&png).expect("prepared");
        assert_eq!((picture.width, picture.height), (64, 32));
        for private in PRIVATE {
            assert!(
                !contains(&picture.png, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        assert!(
            chunk_types(&picture.png)
                .iter()
                .all(|kind| ["IHDR", "pHYs", "IDAT", "IEND"].contains(&kind.as_str())),
            "{:?}",
            chunk_types(&picture.png)
        );
        assert_eq!(
            pixel(&picture.png, 2, 2)[3],
            0,
            "the corner stays see-through"
        );
        assert_eq!(pixel(&picture.png, 30, 10)[3], 255);
    }

    fn single_page() -> PdfDocument {
        PdfDocument::from_bytes(&corpus("benign/single-page.pdf")).expect("open")
    }

    fn saved(doc: &PdfDocument) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save(&mut out).expect("save");
        out
    }

    #[test]
    fn a_picture_stamp_shows_the_picture_and_the_saved_pdf_has_none_of_its_metadata() {
        let jpeg = corpus("images/stamp-exif.jpg");
        let picture = prepare_stamp_picture(&jpeg).expect("prepared");
        let mut doc = single_page();
        // 64 x 32 pixels over 128 x 64 points, 100 points from the left and from the top.
        let rect = Rect {
            x0: 100.0,
            y0: 100.0,
            x1: 228.0,
            y1: 164.0,
        };
        doc.add_image_stamp(0, rect, &picture.png).expect("stamp");
        let [stamp] = &doc.page_annotations(0).expect("annotations")[..] else {
            panic!("one stamp");
        };
        assert_eq!(stamp.kind, AnnotationKind::Stamp);

        let bytes = saved(&doc);
        for private in PRIVATE {
            assert!(
                !contains(&bytes, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // Opened again, the stamp is there and draws the picture over its rectangle.
        let reopened = PdfDocument::from_bytes(&bytes).expect("reopen");
        let [stamp] = &reopened.page_annotations(0).expect("annotations")[..] else {
            panic!("one stamp");
        };
        assert_eq!(stamp.kind, AnnotationKind::Stamp);
        let page = reopened.render(0, 1.0, 0).expect("render");
        let at = |x: usize, y: usize| {
            let start = (y * page.width as usize + x) * 4;
            page.rgba[start..start + 3].to_vec()
        };
        let near = |have: Vec<u8>, want: [u8; 3]| {
            have.iter()
                .zip(want)
                .all(|(have, want)| have.abs_diff(want) < 24)
        };
        // The top of the picture is red, its bottom blue, the stripe between green.
        assert!(near(at(110, 108), [200, 40, 40]), "{:?}", at(110, 108));
        assert!(near(at(210, 156), [40, 60, 200]), "{:?}", at(210, 156));
        assert!(near(at(160, 156), [40, 160, 70]), "{:?}", at(160, 156));
        // Outside the rectangle the page is white.
        assert_eq!(at(60, 60), [255, 255, 255]);
    }

    #[test]
    fn the_same_picture_put_in_as_it_is_would_carry_its_exif() {
        // The control of the test above: MuPDF keeps a JPEG file as it is when it is given the
        // file, so the check for what stays out of the saved PDF can fail.
        let jpeg = corpus("images/stamp-exif.jpg");
        let mut doc = single_page();
        let image = doc
            .doc
            .add_image(&Image::from_bytes(&jpeg).expect("image"))
            .expect("added");
        doc.doc
            .catalog()
            .expect("catalog")
            .dict_put("Probe", image)
            .expect("put");
        let bytes = saved(&doc);
        assert!(contains(&bytes, b"Canon EOS 5D Mark IV"));
        assert!(contains(&bytes, b"2023:07:04 12:34:56"));
    }

    #[test]
    fn pictures_that_are_not_pngs_or_jpegs_or_are_broken_or_too_large_are_refused() {
        let refused = |bytes: &[u8]| prepare_stamp_picture(bytes).expect_err("refused");
        for not_a_picture in [
            Vec::new(),
            b"GIF89a".to_vec(),
            corpus("benign/single-page.pdf"),
            vec![0x42; 1000],
        ] {
            assert!(matches!(
                refused(&not_a_picture),
                EngineError::InvalidPicture(_)
            ));
        }
        // A PNG that says it is 60000 x 60000 pixels is refused for its size, before any pixel.
        assert!(matches!(
            refused(&corpus("images/stamp-huge-dimensions.png")),
            EngineError::PictureTooLarge
        ));
        let mut too_long = PNG_SIGNATURE.to_vec();
        too_long.resize(MAX_STAMP_SOURCE_BYTES + 1, 0);
        assert!(matches!(refused(&too_long), EngineError::PictureTooLarge));
        // Cut short, a picture is an error, not a crash.
        for name in ["images/stamp-exif.jpg", "images/stamp-metadata.png"] {
            let whole = corpus(name);
            for keep in [10, 40, whole.len() / 2, whole.len() - 12] {
                assert!(
                    prepare_stamp_picture(&whole[..keep]).is_err(),
                    "{name} cut at {keep}"
                );
            }
        }
        // The stamp edit takes only what the worker made.
        let mut doc = single_page();
        let rect = Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 90.0,
            y1: 50.0,
        };
        for wrong in [
            Vec::new(),
            corpus("images/stamp-exif.jpg"),
            corpus("images/stamp-huge-dimensions.png"),
        ] {
            assert!(doc.add_image_stamp(0, rect, &wrong).is_err());
        }
        assert!(doc.page_annotations(0).expect("annotations").is_empty());
    }

    fn solid_png(width: i32, height: i32, noise: bool) -> Vec<u8> {
        let mut pixmap =
            Pixmap::new_with_w_h(&Colorspace::device_rgb(), width, height, false).expect("pixmap");
        pixmap.clear_with(0x80).expect("clear");
        if noise {
            let mut state = 12345u32;
            for byte in pixmap.samples_mut() {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                *byte = (state >> 16) as u8;
            }
        }
        let mut png = Vec::new();
        pixmap.write_to(&mut png, ImageFormat::PNG).expect("png");
        png
    }

    #[test]
    fn a_large_picture_is_shrunk_to_a_stamp_and_noise_to_a_file_of_bounded_size() {
        let picture = prepare_stamp_picture(&solid_png(3000, 2000, false)).expect("prepared");
        assert!(picture.width.max(picture.height) <= MAX_STAMP_SIDE_PX);
        // The shape stays (3 : 2), give or take a pixel.
        assert!(
            (picture.width * 2).abs_diff(picture.height * 3) <= 6,
            "{} x {}",
            picture.width,
            picture.height
        );
        assert_eq!(
            stamp_png_size(&picture.png),
            Ok((picture.width, picture.height))
        );
        // About 3 MB of noise does not compress: it is shrunk until the file is small enough.
        let noisy = prepare_stamp_picture(&solid_png(1000, 1000, true)).expect("prepared");
        assert!(noisy.png.len() <= MAX_STAMP_PNG_BYTES);
        assert!(noisy.width < 1000);
    }
}
