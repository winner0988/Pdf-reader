//! What a JPEG file says besides its picture (B3-03, docs/architecture/create-pdf.md): the camera,
//! the time, the place, the program, the owner. All of it is in segments next to the picture's
//! own, and a JPEG goes into a PDF as the file it is, so the segments would go with it. This takes
//! them out without touching the picture (no new encoding, so no loss), and reads the one thing
//! of them that changes how the picture is to be shown: which way up it was taken.
//!
//! Kept are the segments the picture's colours depend on: the JFIF header (`APP0`), the ICC
//! profile (`APP2`) and Adobe's marker (`APP14`, which says how the components are to be read).
//! Everything else that is only information is dropped: the EXIF and XMP of `APP1`, the IPTC and
//! Photoshop blocks of `APP13`, the other applications' `APPn`, and comments.

/// A JPEG without its metadata, and the way up its EXIF said the picture is.
#[derive(Debug, PartialEq, Eq)]
pub struct Cleaned {
    pub bytes: Vec<u8>,
    /// The EXIF orientation, 1 to 8 (1: as it is; 3: turned half a turn; 6: a quarter turn
    /// clockwise to show it right; 8: a quarter turn the other way); 1 when there is none.
    pub orientation: u8,
}

/// `data` without what is only information about it, or `None` if it is not a JPEG this reads
/// through to its scan (the caller then lets MuPDF make of it what it can).
pub fn clean(data: &[u8]) -> Option<Cleaned> {
    if data.len() < 4 || data[..2] != [0xFF, 0xD8] {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut at = 2;
    let mut orientation = 1;
    let mut frame = false;
    loop {
        // A marker is 0xFF and its code, possibly after fill bytes of 0xFF.
        if *data.get(at)? != 0xFF {
            return None;
        }
        let mut code_at = at + 1;
        while *data.get(code_at)? == 0xFF {
            code_at += 1;
        }
        let code = data[code_at];
        match code {
            // The start of the scan: it, the scan and whatever follows are the picture.
            0xDA => {
                if !frame {
                    return None;
                }
                out.extend_from_slice(&data[code_at - 1..]);
                return Some(Cleaned {
                    bytes: out,
                    orientation,
                });
            }
            // The end, or a marker that has no length and has no business before the scan.
            0x00 | 0x01 | 0xD0..=0xD9 => return None,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([
            *data.get(code_at + 1)?,
            *data.get(code_at + 2)?,
        ]));
        if length < 2 {
            return None;
        }
        let end = code_at + 1 + length;
        let payload = data.get(code_at + 3..end)?;
        if matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC4 | 0xC8 | 0xCC) {
            frame = true;
        }
        if code == 0xE1
            && let Some(found) = exif_orientation(payload)
        {
            orientation = found;
        }
        if keeps(code, payload) {
            out.extend_from_slice(&data[code_at - 1..end]);
        }
        at = end;
    }
}

/// Whether the segment with this marker `code` and `payload` stays: all that is not information
/// about the picture, and the three applications' segments its colours depend on.
fn keeps(code: u8, payload: &[u8]) -> bool {
    match code {
        0xE0 => payload.starts_with(b"JFIF\0"),
        0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
        0xEE => payload.starts_with(b"Adobe"),
        0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
        _ => true,
    }
}

/// The orientation (1 to 8) in the EXIF `payload` of an `APP1` segment, if it has one: the first
/// image's directory of a TIFF structure.
fn exif_orientation(payload: &[u8]) -> Option<u8> {
    let tiff = payload.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |at: usize| -> Option<u16> {
        let bytes = [*tiff.get(at)?, *tiff.get(at + 1)?];
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let long = |at: usize| -> Option<u32> {
        let bytes = [
            *tiff.get(at)?,
            *tiff.get(at + 1)?,
            *tiff.get(at + 2)?,
            *tiff.get(at + 3)?,
        ];
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    if short(2)? != 42 {
        return None;
    }
    let directory = usize::try_from(long(4)?).ok()?;
    let count = usize::from(short(directory)?);
    for entry in 0..count.min(256) {
        let at = directory.checked_add(2 + 12 * entry)?;
        // Tag 0x0112, a SHORT, once: its value is in the entry itself.
        if short(at)? == 0x0112 && short(at + 2)? == 3 && long(at + 4)? == 1 {
            let value = short(at + 8)?;
            return u8::try_from(value)
                .ok()
                .filter(|value| (1..=8).contains(value));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use mupdf::Image;

    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn segment(code: u8, payload: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, code];
        segment.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        segment.extend_from_slice(payload);
        segment
    }

    /// The EXIF payload of an `APP1` segment with a directory of the given `tags` (tag, SHORT).
    fn exif(little: bool, tags: &[(u16, u16)]) -> Vec<u8> {
        let short = |value: u16| {
            if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            }
        };
        let long = |value: u32| {
            if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            }
        };
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(if little { b"II" } else { b"MM" });
        payload.extend_from_slice(&short(42));
        payload.extend_from_slice(&long(8));
        payload.extend_from_slice(&short(u16::try_from(tags.len()).unwrap()));
        for (tag, value) in tags {
            payload.extend_from_slice(&short(*tag));
            payload.extend_from_slice(&short(3));
            payload.extend_from_slice(&long(1));
            payload.extend_from_slice(&short(*value));
            payload.extend_from_slice(&[0, 0]);
        }
        payload.extend_from_slice(&long(0));
        payload
    }

    /// The picture of `images/stamp-exif.jpg` with `segments` put right after the start of the file
    /// in place of what it has there.
    fn with(segments: &[Vec<u8>]) -> Vec<u8> {
        let original = corpus("images/stamp-exif.jpg");
        let cleaned = clean(&original).expect("the sample is a JPEG");
        let mut jpeg = cleaned.bytes[..2].to_vec();
        for segment in segments {
            jpeg.extend_from_slice(segment);
        }
        jpeg.extend_from_slice(&cleaned.bytes[2..]);
        jpeg
    }

    #[test]
    fn what_a_camera_wrote_is_taken_out_and_the_picture_is_the_same() {
        let original = corpus("images/stamp-exif.jpg");
        assert!(
            original.windows(10).any(|w| w == b"Canon EOS "),
            "the control"
        );
        let cleaned = clean(&original).expect("a JPEG");
        assert!(cleaned.bytes.len() < original.len());
        for private in [&b"Canon"[..], b"2023:07:04", b"Exif", b"Mark IV"] {
            assert!(
                !cleaned.bytes.windows(private.len()).any(|w| w == private),
                "{}",
                String::from_utf8_lossy(private)
            );
        }
        // Still a picture of the same size, which MuPDF decodes to the same pixels.
        let (before, after) = (
            Image::from_bytes(&original).expect("image"),
            Image::from_bytes(&cleaned.bytes).expect("image"),
        );
        assert_eq!(
            (before.width(), before.height()),
            (after.width(), after.height())
        );
        let (a, b) = (before.to_pixmap().unwrap(), after.to_pixmap().unwrap());
        assert_eq!(a.samples(), b.samples());
    }

    #[test]
    fn only_information_is_dropped_and_what_the_colours_depend_on_stays() {
        let mut icc = b"ICC_PROFILE\0".to_vec();
        icc.extend_from_slice(&[1, 1, 7, 7, 7]);
        let jpeg = with(&[
            segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"),
            segment(0xE1, &exif(true, &[(0x010F, 3)])),
            segment(
                0xE1,
                b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPS</x:xmpmeta>",
            ),
            segment(0xE2, &icc),
            segment(0xED, b"Photoshop 3.0\08BIM"),
            segment(0xEE, b"Adobe\0\x64\0\0\0\0\x01"),
            segment(0xEF, b"another program"),
            segment(0xFE, b"a comment about whose picture it is"),
            segment(0xE5, b"a thumbnail of someone"),
        ]);
        let cleaned = clean(&jpeg).expect("a JPEG").bytes;
        let has = |needle: &[u8]| cleaned.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"JFIF\0") && has(b"ICC_PROFILE\0") && has(b"Adobe"));
        for gone in [
            &b"xmpmeta"[..],
            b"Exif",
            b"Photoshop",
            b"another program",
            b"whose picture",
            b"thumbnail",
        ] {
            assert!(!has(gone), "{}", String::from_utf8_lossy(gone));
        }
        // The tables and the frame stay (it still decodes), and the scan is untouched.
        assert!(Image::from_bytes(&cleaned).is_ok());
        let scan_of = |bytes: &[u8]| {
            let at = bytes.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
            bytes[at..].to_vec()
        };
        assert_eq!(scan_of(&cleaned), scan_of(&jpeg));
    }

    #[test]
    fn the_way_up_is_read_from_the_exif_whichever_way_it_is_written() {
        for little in [true, false] {
            for orientation in 1..=8u16 {
                let jpeg = with(&[segment(
                    0xE1,
                    &exif(little, &[(0x010F, 3), (0x0112, orientation)]),
                )]);
                let cleaned = clean(&jpeg).expect("a JPEG");
                assert_eq!(
                    u16::from(cleaned.orientation),
                    orientation,
                    "little: {little}"
                );
            }
        }
        // None, or one that is not a way up: as it is.
        assert_eq!(clean(&with(&[])).unwrap().orientation, 1);
        for bad in [0u16, 9, 255, 65535] {
            let jpeg = with(&[segment(0xE1, &exif(true, &[(0x0112, bad)]))]);
            assert_eq!(clean(&jpeg).unwrap().orientation, 1, "{bad}");
        }
        // The EXIF of another structure: wrong byte order mark, wrong magic number, a directory
        // that is not there.
        let mut broken = exif(true, &[(0x0112, 6)]);
        broken[6..8].copy_from_slice(b"XX");
        assert_eq!(
            clean(&with(&[segment(0xE1, &broken)])).unwrap().orientation,
            1
        );
        let mut broken = exif(true, &[(0x0112, 6)]);
        broken[8] = 43;
        assert_eq!(
            clean(&with(&[segment(0xE1, &broken)])).unwrap().orientation,
            1
        );
        let mut broken = exif(true, &[(0x0112, 6)]);
        broken[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            clean(&with(&[segment(0xE1, &broken)])).unwrap().orientation,
            1
        );
    }

    #[test]
    fn what_is_not_a_jpeg_read_through_to_its_scan_is_not_touched() {
        let original = corpus("images/stamp-exif.jpg");
        for bad in [
            Vec::new(),
            vec![0xFF],
            vec![0xFF, 0xD8],
            b"GIF89a".to_vec(),
            corpus("images/stamp-metadata.png"),
            // Cut before the scan, or in a segment, or with a length that goes past the end.
            original[..30].to_vec(),
            original[..original.len().min(60)].to_vec(),
            [&[0xFF, 0xD8][..], &[0xFF, 0xE1, 0xFF, 0xFF, 1, 2, 3]].concat(),
            // A length of less than the two bytes that say it.
            [&[0xFF, 0xD8][..], &[0xFF, 0xE1, 0x00, 0x01]].concat(),
            // A scan with no frame before it.
            [&[0xFF, 0xD8][..], &segment(0xDA, &[1, 2, 3, 4])].concat(),
            // Bytes that are not a marker where one is due.
            [&[0xFF, 0xD8][..], &[0x12, 0x34, 0x56, 0x78]].concat(),
            // The end of the picture before its scan.
            [&[0xFF, 0xD8][..], &[0xFF, 0xD9, 0, 0]].concat(),
        ] {
            assert_eq!(clean(&bad), None, "{:?}", &bad[..bad.len().min(12)]);
        }
        // Fill bytes before a marker are allowed.
        let mut filled = original[..2].to_vec();
        filled.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
        filled.extend_from_slice(&original[3..]);
        assert!(clean(&filled).is_some());
    }
}
