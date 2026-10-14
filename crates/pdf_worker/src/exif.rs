//! The pictures of a copy without what was written about them (B3-04, docs/architecture/batch.md).
//!
//! A JPEG goes into a PDF as the file it is (`/DCTDecode`), and with it what a camera or a program
//! wrote into it besides the picture: the camera, the time and the place it was taken (EXIF), XMP,
//! IPTC and Photoshop blocks, comments, a thumbnail, a second picture after its end. All of that is
//! taken out by [`crate::jpeg::clean`] without touching the picture (no new encoding, so no loss).
//! A picture's own entries about itself go too: its XMP (`/Metadata`), the path of its original
//! for a prepress system (`/OPI`), and applications' private data (`/PieceInfo`, `/LastModified`).
//!
//! A JPEG may be stored behind lossless filters (`[/FlateDecode /DCTDecode]`, ASCII85 and the
//! like): MuPDF reads it through them, and it is stored as the JPEG it is. One that cannot be read
//! through to its picture (behind a filter that is not lossless, or not a JPEG this reads) is not
//! left as it is: the copy is refused ([`EngineError::PictureNotCleaned`]) rather than said to be
//! clean.
//!
//! Not looked at: pictures written into a page's content itself (inline images, small and rare),
//! and what a JPEG 2000 file (`/JPXDecode`, scans mostly) carries in boxes of its own.

use mupdf::Buffer;
use mupdf::pdf::{PdfDocument, PdfObject};

use crate::compress::{filters, is_image};
use crate::engine::EngineError;
use crate::jpeg;

/// What a picture says about itself in its dictionary: XMP, the path of its original for a
/// prepress system, and applications' private data and when they changed it.
const PICTURE_ENTRIES: [&str; 4] = ["Metadata", "OPI", "PieceInfo", "LastModified"];

/// The filters a JPEG may be stored behind and still be read through: those that lose nothing,
/// by their names and by the short names MuPDF reads as well.
const LOSSLESS: [&[u8]; 12] = [
    b"FlateDecode",
    b"Fl",
    b"LZWDecode",
    b"LZW",
    b"ASCIIHexDecode",
    b"AHx",
    b"ASCII85Decode",
    b"A85",
    b"RunLengthDecode",
    b"RL",
    b"BrotliDecode",
    b"Br",
];

/// The JPEG filter, by its name or its short name.
fn is_jpeg_filter(name: &[u8]) -> bool {
    name == b"DCTDecode" || name == b"DCT"
}

/// What was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// The pictures of the document, and its other JPEG streams (a page's thumbnail).
    pub pictures: u32,
    /// Those that had something taken out.
    pub cleaned: u32,
}

/// Takes out of the pictures of `doc`, in place, all that is not the picture.
pub fn clean_pictures(doc: &mut PdfDocument) -> Result<Report, EngineError> {
    let count = doc.xref_len()?;
    let mut report = Report::default();
    for number in 1..count {
        let Ok(number) = i32::try_from(number) else {
            break;
        };
        let Ok(object) = doc.new_indirect(number, 0) else {
            continue;
        };
        if !object.is_stream().unwrap_or(false) {
            continue;
        }
        let picture = is_image(&object);
        let names = filters(&object)?;
        let jpeg = names
            .as_ref()
            .is_some_and(|names| names.iter().any(|name| is_jpeg_filter(name)));
        if !picture && !jpeg {
            continue;
        }
        report.pictures = report.pictures.saturating_add(1);
        if clean_picture(doc, object, picture, names.unwrap_or_default())? {
            report.cleaned = report.cleaned.saturating_add(1);
        }
    }
    Ok(report)
}

/// Takes out of the stream `object` all that is not its picture: its entries about itself when
/// it is a `picture`, and the metadata of the JPEG its `filters` end with. Whether anything was
/// taken out.
fn clean_picture(
    doc: &mut PdfDocument,
    mut object: PdfObject,
    picture: bool,
    filters: Vec<Vec<u8>>,
) -> Result<bool, EngineError> {
    let mut changed = false;
    if picture {
        for key in PICTURE_ENTRIES {
            if object.get_dict(key)?.is_some() {
                object.dict_delete(key)?;
                changed = true;
            }
        }
    }
    let Some(at) = filters.iter().position(|name| is_jpeg_filter(name)) else {
        return Ok(changed);
    };
    // The JPEG is the last thing the filters undo, and what comes before it loses nothing.
    if at + 1 != filters.len()
        || !filters[..at]
            .iter()
            .all(|name| LOSSLESS.contains(&name.as_slice()))
    {
        return Err(EngineError::PictureNotCleaned);
    }
    let stored = object
        .read_raw_stream()
        .map_err(|_| EngineError::PictureNotCleaned)?;
    if stored.is_empty() {
        // No picture, and nothing written about one.
        return Ok(changed);
    }
    let data = if at == 0 {
        stored
    } else {
        read_through(doc, &object, &filters[..at], &stored)?
    };
    // What the filters hold is a JPEG this reads through to its picture, or nothing is made.
    let cleaned = jpeg::clean(&data).ok_or(EngineError::PictureNotCleaned)?;
    if cleaned.bytes == data {
        return Ok(changed);
    }
    if at > 0 {
        // Stored as the JPEG it is: the filters before it are not needed any more.
        let own = jpeg_parameters(&object, at)?;
        object.dict_put("Filter", PdfObject::new_name("DCTDecode")?)?;
        match own {
            Some(parameters) => object.dict_put("DecodeParms", parameters)?,
            None => object.dict_delete("DecodeParms")?,
        }
    }
    object.write_raw_stream_buffer(&Buffer::from_bytes(&cleaned.bytes)?)?;
    Ok(true)
}

/// The JPEG that the lossless `filters`, the first ones of `object`, hold in `stored`: MuPDF
/// undoes them on a stream of its own, made for this and removed again.
fn read_through(
    doc: &mut PdfDocument,
    object: &PdfObject,
    filters: &[Vec<u8>],
    stored: &[u8],
) -> Result<Vec<u8>, EngineError> {
    let mut dictionary = doc.new_dict()?;
    let mut names = doc.new_array()?;
    for name in filters {
        let name = std::str::from_utf8(name).map_err(|_| EngineError::PictureNotCleaned)?;
        names.array_push(doc.new_name(name)?)?;
    }
    dictionary.dict_put("Filter", names)?;
    // A list of parameters goes with a list of filters, one each; MuPDF reads nothing else.
    if let Some(all) = object.get_dict("DecodeParms")?
        && all.is_array()?
    {
        let mut parameters = doc.new_array()?;
        for index in 0..filters.len() {
            let own = i32::try_from(index)
                .ok()
                .and_then(|index| all.get_array(index).ok().flatten());
            parameters.array_push(own.unwrap_or_else(|| doc.new_null()))?;
        }
        dictionary.dict_put("DecodeParms", parameters)?;
    }
    let stream = doc.add_stream(&Buffer::from_bytes(stored)?, Some(&dictionary), true)?;
    let number = stream.as_indirect()?;
    let read = stream.read_stream();
    drop(stream);
    doc.delete_object(number)?;
    read.map_err(|_| EngineError::PictureNotCleaned)
}

/// The JPEG filter's own parameters: the entry `at` of the list of `object`'s `/DecodeParms`,
/// when it is a dictionary.
fn jpeg_parameters(object: &PdfObject, at: usize) -> Result<Option<PdfObject>, EngineError> {
    let Some(all) = object.get_dict("DecodeParms")? else {
        return Ok(None);
    };
    if !all.is_array()? {
        return Ok(None);
    }
    let Some(own) = all.get_array(i32::try_from(at).unwrap_or(i32::MAX))? else {
        return Ok(None);
    };
    Ok(if own.is_dict()? {
        Some(own.copy_dict()?)
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfWriteOptions};
    use mupdf::{Document, Rect};

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

    /// What a camera wrote into `images/stamp-exif.jpg`, and what a program adds to a JPEG.
    const PRIVATE: [&[u8]; 6] = [
        b"Canon",
        b"2023:07:04",
        b"Exif",
        b"xmpmeta",
        b"Jane Q. Private-Author",
        b"a second picture",
    ];

    fn segment(code: u8, payload: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, code];
        segment.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        segment.extend_from_slice(payload);
        segment
    }

    /// The camera's JPEG with more of what programs write: XMP naming an author, a comment, and
    /// a second picture after the end of the first, as a phone's or a camera's.
    fn camera_jpeg() -> Vec<u8> {
        let original = corpus("images/stamp-exif.jpg");
        let mut jpeg = original[..2].to_vec();
        jpeg.extend(segment(
            0xE1,
            b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>Jane Q. Private-Author</x:xmpmeta>",
        ));
        jpeg.extend(segment(0xFE, b"Jane Q. Private-Author's holiday"));
        jpeg.extend_from_slice(&original[2..]);
        jpeg.extend_from_slice(b"\xFF\xD8 a second picture \xFF\xD9");
        jpeg
    }

    fn written(doc: &PdfDocument) -> Vec<u8> {
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut out = Vec::new();
        doc.write_to_with_options(&mut out, options)
            .expect("written");
        out
    }

    fn reopened(bytes: &[u8]) -> PdfDocument {
        let file = Document::from_bytes(bytes, "application/pdf").expect("read");
        PdfDocument::try_from(file).expect("a PDF")
    }

    /// A document of one page with the JPEG `jpeg` on it, as MuPDF puts one in: the file as it
    /// is, `/DCTDecode`.
    fn page_with(jpeg: &[u8]) -> PdfDocument {
        let mut doc = PdfDocument::new();
        let mut page = doc.new_page((300.0, 200.0)).expect("page");
        page.insert_image(
            &mut doc,
            Rect {
                x0: 10.0,
                y0: 10.0,
                x1: 266.0,
                y1: 138.0,
            },
            PageImageSource::Bytes {
                data: jpeg,
                format_hint: None,
            },
            InsertImageOptions::default(),
        )
        .expect("picture put in");
        drop(page);
        reopened(&written(&doc))
    }

    /// The numbers of the picture streams of `doc`.
    fn pictures(doc: &PdfDocument) -> Vec<i32> {
        (1..doc.xref_len().expect("count"))
            .filter_map(|number| {
                let number = i32::try_from(number).ok()?;
                is_image(&doc.new_indirect(number, 0).ok()?).then_some(number)
            })
            .collect()
    }

    /// The page drawn, as RGB.
    fn drawn(bytes: &[u8]) -> Vec<u8> {
        let page = crate::engine::PdfDocument::from_bytes(bytes)
            .expect("open")
            .render(0, 1.0, 0)
            .expect("rendered");
        page.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()
    }

    /// An image stream made by hand: `entries` are written after `/Type /XObject /Subtype /Image
    /// /Width 64 /Height 32 /ColorSpace /DeviceRGB /BitsPerComponent 8`, and `data` is stored as
    /// it is. Something refers to it, or writing the document would leave it out.
    fn handmade(doc: &mut PdfDocument, entries: &str, data: &[u8]) -> i32 {
        let dictionary = doc
            .new_object_from_str(&format!(
                "<< /Type /XObject /Subtype /Image /Width 64 /Height 32 /ColorSpace /DeviceRGB \
                 /BitsPerComponent 8 {entries} >>"
            ))
            .expect("dictionary");
        let stream = doc
            .add_stream(&Buffer::from_bytes(data).unwrap(), Some(&dictionary), true)
            .expect("stream");
        let number = stream.as_indirect().expect("reference");
        let mut catalog = doc.catalog().expect("catalog");
        if catalog.get_dict("Probes").expect("entry").is_none() {
            catalog
                .dict_put("Probes", doc.new_array().expect("array"))
                .expect("put");
        }
        let mut probes = catalog.get_dict("Probes").expect("entry").expect("there");
        probes.array_push(stream).expect("pushed");
        number
    }

    fn hex(data: &[u8]) -> Vec<u8> {
        let mut text: Vec<u8> = data
            .iter()
            .flat_map(|byte| format!("{byte:02X}").into_bytes())
            .collect();
        text.push(b'>');
        text
    }

    /// A zlib stream of stored blocks: deflate without compression, which any reader undoes.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = data.chunks(65_535).collect();
        for (index, block) in blocks.iter().enumerate() {
            out.push(u8::from(index + 1 == blocks.len()));
            let length = u16::try_from(block.len()).unwrap();
            out.extend_from_slice(&length.to_le_bytes());
            out.extend_from_slice(&(!length).to_le_bytes());
            out.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in data {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    #[test]
    fn what_a_camera_wrote_leaves_the_pdf_and_the_page_looks_the_same() {
        let jpeg = camera_jpeg();
        let mut doc = page_with(&jpeg);
        let before = written(&doc);
        for private in PRIVATE {
            assert!(
                contains(&before, private),
                "the control: {}",
                String::from_utf8_lossy(private)
            );
        }
        let report = clean_pictures(&mut doc).expect("cleaned");
        assert_eq!(
            report,
            Report {
                pictures: 1,
                cleaned: 1
            }
        );
        let after = written(&doc);
        for private in PRIVATE {
            assert!(
                !contains(&after, private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // The picture is the same file but for what was taken out: drawn the same, pixel for pixel.
        assert_eq!(drawn(&before), drawn(&after));
        let [number] = pictures(&doc)[..] else {
            panic!("one picture")
        };
        let stored = doc
            .new_indirect(number, 0)
            .unwrap()
            .read_raw_stream()
            .unwrap();
        assert_eq!(stored, jpeg::clean(&jpeg).unwrap().bytes);
        // Done again, there is nothing left to take out.
        assert_eq!(
            clean_pictures(&mut doc).expect("cleaned"),
            Report {
                pictures: 1,
                cleaned: 0
            }
        );
    }

    #[test]
    fn a_jpeg_behind_lossless_filters_is_read_through_and_stored_as_the_jpeg_it_is() {
        let jpeg = camera_jpeg();
        let clean = jpeg::clean(&jpeg).unwrap().bytes;
        for (entries, stored) in [
            (
                "/Filter [/ASCIIHexDecode /DCTDecode] /DecodeParms [null << /ColorTransform 1 >>]",
                hex(&jpeg),
            ),
            ("/Filter [/FlateDecode /DCTDecode]", zlib_stored(&jpeg)),
            ("/Filter [/AHx /Fl /DCT]", hex(&zlib_stored(&jpeg))),
        ] {
            let mut doc = PdfDocument::new();
            let number = handmade(&mut doc, entries, &stored);
            let samples = doc.new_indirect(number, 0).unwrap().read_stream().unwrap();
            assert_eq!(samples.len(), 64 * 32 * 3, "the control: {entries}");
            let report = clean_pictures(&mut doc).expect("cleaned");
            assert_eq!((report.pictures, report.cleaned), (1, 1), "{entries}");
            let object = doc.new_indirect(number, 0).unwrap();
            assert_eq!(object.read_raw_stream().unwrap(), clean, "{entries}");
            let filter = object.get_dict("Filter").unwrap().unwrap();
            assert_eq!(filter.as_name().unwrap(), b"DCTDecode", "{entries}");
            // Its own parameters stay, those of the filters that went go with them.
            let parameters = object.get_dict("DecodeParms").unwrap();
            if entries.contains("ColorTransform") {
                let parameters = parameters.expect("its own");
                assert!(parameters.is_dict().unwrap());
                assert_eq!(
                    parameters
                        .get_dict("ColorTransform")
                        .unwrap()
                        .unwrap()
                        .as_int()
                        .unwrap(),
                    1
                );
            } else {
                assert!(parameters.is_none(), "{entries}");
            }
            // The same pixels, and nothing private anywhere in the file, in hexadecimal either.
            assert_eq!(object.read_stream().unwrap(), samples, "{entries}");
            let after = written(&doc);
            for private in PRIVATE {
                assert!(!contains(&after, private), "{entries}");
            }
            assert!(!contains(&after, &hex(b"Canon")[..10]), "{entries}");
        }
    }

    #[test]
    fn a_jpeg_that_cannot_be_read_through_refuses_the_copy() {
        let jpeg = camera_jpeg();
        for (entries, stored) in [
            // Behind a filter that is not lossless, or one nobody knows (MuPDF reads past it).
            ("/Filter [/JBIG2Decode /DCTDecode]", jpeg.clone()),
            ("/Filter [/Unknown /DCTDecode]", jpeg.clone()),
            // A JPEG filter that is not the last.
            ("/Filter [/DCTDecode /FlateDecode]", jpeg.clone()),
            // Not a JPEG this reads through to its picture.
            ("/Filter /DCTDecode", b"Exif Canon, not a JPEG".to_vec()),
            ("/Filter /DCTDecode", jpeg[..40].to_vec()),
            // Hexadecimal that is not.
            ("/Filter [/ASCIIHexDecode /DCTDecode]", b"zz Canon".to_vec()),
        ] {
            let mut doc = PdfDocument::new();
            handmade(&mut doc, entries, &stored);
            assert!(
                matches!(
                    clean_pictures(&mut doc),
                    Err(EngineError::PictureNotCleaned)
                ),
                "{entries}"
            );
        }
    }

    #[test]
    fn a_pictures_own_entries_about_itself_go_and_the_documents_stay() {
        let mut doc = PdfDocument::new();
        let dictionary = doc
            .new_object_from_str("<< /Type /Metadata /Subtype /XML >>")
            .unwrap();
        let xmp = doc
            .add_stream(
                &Buffer::from_bytes(b"<x:xmpmeta>Jane Q. Private-Author</x:xmpmeta>").unwrap(),
                Some(&dictionary),
                true,
            )
            .unwrap()
            .as_indirect()
            .unwrap();
        let number = handmade(
            &mut doc,
            &format!(
                "/Metadata {xmp} 0 R /OPI << /2.0 << /F (C:\\\\Users\\\\Jane\\\\photo.tif) >> >> \
                 /PieceInfo << /Editor << /Private (Jane) >> >> /LastModified (D:20260102)"
            ),
            &[128; 64 * 32 * 3],
        );
        // The document's own XMP is the privacy export's (B2-03), not this one's.
        let mut catalog = doc.catalog().unwrap();
        catalog
            .dict_put("Metadata", doc.new_indirect(xmp, 0).unwrap())
            .unwrap();
        let report = clean_pictures(&mut doc).expect("cleaned");
        assert_eq!((report.pictures, report.cleaned), (1, 1));
        let picture = doc.new_indirect(number, 0).unwrap();
        for key in PICTURE_ENTRIES {
            assert!(picture.get_dict(key).unwrap().is_none(), "{key}");
        }
        assert!(
            doc.catalog()
                .unwrap()
                .get_dict("Metadata")
                .unwrap()
                .is_some()
        );
        // The pixels are what they were.
        assert_eq!(picture.read_stream().unwrap(), vec![128; 64 * 32 * 3]);
    }

    #[test]
    fn what_has_nothing_to_take_out_is_left_as_it_is() {
        let clean = jpeg::clean(&camera_jpeg()).unwrap().bytes;
        let mut doc = PdfDocument::new();
        let plain = handmade(&mut doc, "/Filter /DCTDecode", &clean);
        let pixels = handmade(&mut doc, "", &[7; 64 * 32 * 3]);
        let empty = handmade(&mut doc, "/Filter /DCTDecode", &[]);
        // Behind a filter and with nothing to take out: it keeps its filters.
        let behind = handmade(
            &mut doc,
            "/Filter [/ASCIIHexDecode /DCTDecode]",
            &hex(&clean),
        );
        let raw = |doc: &PdfDocument, number: i32| {
            doc.new_indirect(number, 0)
                .unwrap()
                .read_raw_stream()
                .unwrap()
        };
        let before: Vec<_> = [plain, pixels, empty, behind]
            .iter()
            .map(|&number| raw(&doc, number))
            .collect();
        let report = clean_pictures(&mut doc).expect("cleaned");
        assert_eq!((report.pictures, report.cleaned), (4, 0));
        let after: Vec<_> = [plain, pixels, empty, behind]
            .iter()
            .map(|&number| raw(&doc, number))
            .collect();
        assert_eq!(before, after);
        let filter = doc
            .new_indirect(behind, 0)
            .unwrap()
            .get_dict("Filter")
            .unwrap()
            .unwrap();
        assert!(filter.is_array().unwrap());
    }

    #[test]
    fn a_jpeg_that_is_not_a_picture_of_a_page_is_cleaned_too() {
        // A page's thumbnail has no /Subtype: it is still a JPEG in the file.
        let mut doc = PdfDocument::new();
        let dictionary = doc
            .new_object_from_str(
                "<< /Width 64 /Height 32 /ColorSpace /DeviceRGB /BitsPerComponent 8 \
                 /Filter /DCTDecode >>",
            )
            .unwrap();
        let thumbnail = doc
            .add_stream(
                &Buffer::from_bytes(&camera_jpeg()).unwrap(),
                Some(&dictionary),
                true,
            )
            .unwrap();
        doc.catalog().unwrap().dict_put("Probe", thumbnail).unwrap();
        let report = clean_pictures(&mut doc).expect("cleaned");
        assert_eq!((report.pictures, report.cleaned), (1, 1));
        let after = written(&doc);
        for private in PRIVATE {
            assert!(!contains(&after, private));
        }
    }
}
