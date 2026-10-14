//! The pictures of a compressed copy, encoded again (B3-01, docs/architecture/compress-images.md).
//!
//! A photograph is mostly detail the eye cannot tell: encoded again as a JPEG at a lower quality,
//! and shrunk when it has more pixels than a screen or a printer can use, it takes a fraction of
//! the room. This is done to a document that is already a copy (the open one is never touched),
//! picture by picture, and only to a picture where it makes the file smaller and cannot change
//! what the page means:
//!
//! - only a picture of 8 bits a sample in a plain colour space (one gray or three color samples,
//!   or an ICC profile that says so) is looked at; the colour space entry stays, so the numbers
//!   mean what they meant;
//! - a mask of another picture (`/SMask`) stays as it is: JPEG blurs the edge of an alpha;
//! - a picture with a `/Mask`, one that is a stencil, one with a palette, one in a CCITT, JBIG2
//!   or JPEG 2000 file (scans, mostly: they are small already), and one with more than
//!   [`MAX_PIXELS`] pixels stay as they are;
//! - the new picture replaces the old one only when it takes [`WORTH_IT_PERCENT`] percent of
//!   its room or less;
//! - the old one's own metadata (`/Metadata`) goes with it, and a JPEG made here carries none of
//!   the camera's (EXIF, GPS): there is no segment in it to carry them in.
//!
//! The pictures come from the document, which is not to be trusted: they are decoded by MuPDF in
//! this sandboxed process, bounded in pixels and in time, and a picture that cannot be decoded is
//! left as it is.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ipc_contract::limits::{MAX_COMPRESS_SIDE_PX, MIN_COMPRESS_SIDE_PX};
use ipc_contract::types::ImageQuality;
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder};
use mupdf::pdf::{PdfDocument, PdfObject};
use mupdf::{Buffer, Image};

use crate::engine::EngineError;

/// A picture with more pixels than this stays as it is: decoding and shrinking one takes memory
/// and time the worker has other uses for (36 megapixels are 108 MB as RGB).
const MAX_PIXELS: u64 = 36_000_000;

/// The longest side of a JPEG, in pixels.
const MAX_SIDE: u64 = 65_535;

/// How long all the pictures may take. The request has 5 minutes (`worker_host::SAVE_TIMEOUT`) to
/// write the copy as well; a picture not looked at by then is left as it is.
pub const BUDGET: Duration = Duration::from_secs(150);

/// A picture is replaced when the new one is this many hundredths of the old one's size or less:
/// less than that is not worth what encoding twice costs the picture.
const WORTH_IT_PERCENT: u64 = 90;

/// Most filters a picture may have.
const MAX_FILTERS: usize = 8;

/// The filters of a picture MuPDF is asked to decode for this: lossless ones, and JPEG. A picture
/// with any other (JPEG 2000, JBIG2, CCITT, a crypt filter) stays as it is.
const DECODABLE: [&[u8]; 6] = [
    b"FlateDecode",
    b"LZWDecode",
    b"ASCIIHexDecode",
    b"ASCII85Decode",
    b"RunLengthDecode",
    b"DCTDecode",
];

/// What to do to the pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub quality: ImageQuality,
    /// A picture with a longer side than this, in pixels, is shrunk to it.
    pub max_side: Option<u32>,
    /// How long the pictures may take altogether.
    pub budget: Duration,
}

/// What was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// The pictures of the document, but for the masks of other pictures.
    pub images: u32,
    /// Those that were encoded again.
    pub changed: u32,
    /// The time was up before every picture was looked at.
    pub partial: bool,
}

/// The JPEG quality (1 to 100) for `quality`.
pub fn jpeg_quality(quality: ImageQuality) -> u8 {
    match quality {
        ImageQuality::High => 85,
        ImageQuality::Medium => 70,
        ImageQuality::Low => 50,
    }
}

/// Encodes the pictures of `doc` again, in place.
pub fn recompress(doc: &PdfDocument, options: Options) -> Result<Report, EngineError> {
    if let Some(side) = options.max_side
        && !(MIN_COMPRESS_SIDE_PX..=MAX_COMPRESS_SIDE_PX).contains(&side)
    {
        return Err(EngineError::InvalidEdit("not a size to shrink pictures to"));
    }
    let started = Instant::now();
    let count = doc.xref_len()?;
    let masks = masks_of(doc, count);
    let mut report = Report::default();
    for number in 1..count {
        let Ok(number) = i32::try_from(number) else {
            break;
        };
        if masks.contains(&number) {
            continue;
        }
        let Ok(object) = doc.new_indirect(number, 0) else {
            continue;
        };
        if !is_image(&object) {
            continue;
        }
        report.images += 1;
        if started.elapsed() >= options.budget {
            report.partial = true;
            continue;
        }
        if encode_again(object, &options)? {
            report.changed += 1;
        }
    }
    Ok(report)
}

/// The numbers of the pictures some other picture uses as its mask.
fn masks_of(doc: &PdfDocument, count: usize) -> HashSet<i32> {
    let mut masks = HashSet::new();
    for number in 1..count {
        let Ok(number) = i32::try_from(number) else {
            break;
        };
        let Ok(object) = doc.new_indirect(number, 0) else {
            continue;
        };
        if !is_image(&object) {
            continue;
        }
        for key in ["SMask", "Mask"] {
            if let Ok(Some(mask)) = object.get_dict(key)
                && mask.is_indirect().unwrap_or(false)
                && let Ok(target) = mask.as_indirect()
            {
                masks.insert(target);
            }
        }
    }
    masks
}

/// A stream whose `/Subtype` is `/Image`.
fn is_image(object: &PdfObject) -> bool {
    named(object, "Subtype").as_deref() == Some(b"Image".as_slice())
        && object.is_stream().unwrap_or(false)
}

/// The name under `key`, if there is one.
fn named(object: &PdfObject, key: &str) -> Option<Vec<u8>> {
    let value = object.get_dict(key).ok()??;
    if value.is_name().ok()? {
        value.as_name().ok()
    } else {
        None
    }
}

/// The whole number under `key`, if there is one.
fn whole_number(object: &PdfObject, key: &str) -> Result<Option<i64>, mupdf::Error> {
    match object.get_dict(key)? {
        Some(value) if value.is_int()? => Ok(Some(i64::from(value.as_int()?))),
        _ => Ok(None),
    }
}

/// What a picture is, as far as it matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Picture {
    width: u64,
    height: u64,
    /// Samples per pixel: 1 or 3.
    channels: u64,
    /// What it takes in the file.
    stored: u64,
}

/// What `object` is, when it is a picture that may be encoded again.
fn read_picture(object: &PdfObject) -> Result<Option<Picture>, mupdf::Error> {
    let stencil = match object.get_dict("ImageMask")? {
        Some(value) => value.is_bool()? && value.as_bool()?,
        None => false,
    };
    if stencil || object.get_dict("Mask")?.is_some() {
        return Ok(None);
    }
    let (Some(width), Some(height)) = (
        whole_number(object, "Width")?,
        whole_number(object, "Height")?,
    ) else {
        return Ok(None);
    };
    let (Ok(width), Ok(height)) = (u64::try_from(width), u64::try_from(height)) else {
        return Ok(None);
    };
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || width * height > MAX_PIXELS
    {
        return Ok(None);
    }
    let Some(filters) = filters(object)? else {
        return Ok(None);
    };
    if filters
        .iter()
        .any(|filter| !DECODABLE.contains(&filter.as_slice()))
    {
        return Ok(None);
    }
    // A JPEG says its own depth; the others must say 8.
    let jpeg = filters.iter().any(|filter| filter == b"DCTDecode");
    match whole_number(object, "BitsPerComponent")? {
        Some(8) => {}
        None if jpeg => {}
        _ => return Ok(None),
    }
    let Some(channels) = components(object)? else {
        return Ok(None);
    };
    let stored = match whole_number(object, "Length")? {
        Some(length) if length > 0 => length.unsigned_abs(),
        _ => u64::try_from(object.read_raw_stream()?.len()).unwrap_or(u64::MAX),
    };
    Ok(Some(Picture {
        width,
        height,
        channels,
        stored,
    }))
}

/// The names of the filters of `object`, in the order they are undone; none when it has none or
/// they are not names.
fn filters(object: &PdfObject) -> Result<Option<Vec<Vec<u8>>>, mupdf::Error> {
    let Some(entry) = object.get_dict("Filter")? else {
        return Ok(Some(Vec::new()));
    };
    if entry.is_name()? {
        return Ok(Some(vec![entry.as_name()?]));
    }
    if !entry.is_array()? || entry.len()? > MAX_FILTERS {
        return Ok(None);
    }
    let mut names = Vec::new();
    for index in 0..entry.len()? {
        let Some(item) = i32::try_from(index)
            .ok()
            .and_then(|index| entry.get_array(index).ok().flatten())
        else {
            return Ok(None);
        };
        if !item.is_name()? {
            return Ok(None);
        }
        names.push(item.as_name()?);
    }
    Ok(Some(names))
}

/// How many samples a pixel of `object` has, when its colour space is one gray sample or three
/// colour samples; none for a palette, for CMYK, for Lab and for the rest.
fn components(object: &PdfObject) -> Result<Option<u64>, mupdf::Error> {
    let Some(space) = object.get_dict("ColorSpace")? else {
        return Ok(None);
    };
    let by_name = |name: &[u8]| match name {
        b"DeviceRGB" | b"CalRGB" => Some(3),
        b"DeviceGray" | b"CalGray" => Some(1),
        _ => None,
    };
    if space.is_name()? {
        return Ok(by_name(&space.as_name()?));
    }
    if !space.is_array()? || space.len()? == 0 {
        return Ok(None);
    }
    let Some(family) = space.get_array(0)? else {
        return Ok(None);
    };
    if !family.is_name()? {
        return Ok(None);
    }
    let family = family.as_name()?;
    if family == b"ICCBased" {
        let Some(profile) = space.get_array(1)? else {
            return Ok(None);
        };
        return Ok(match whole_number(&profile, "N")? {
            Some(1) => Some(1),
            Some(3) => Some(3),
            _ => None,
        });
    }
    Ok(by_name(&family))
}

/// The size a `width` by `height` picture is to be shrunk to for a limit on its longer side.
fn target(width: u64, height: u64, max_side: Option<u32>) -> (u64, u64) {
    let Some(limit) = max_side.map(u64::from) else {
        return (width, height);
    };
    let longer = width.max(height);
    if longer <= limit {
        return (width, height);
    }
    let scale = limit as f64 / longer as f64;
    let shrink = |side: u64| ((side as f64 * scale).round() as u64).clamp(1, side);
    (shrink(width), shrink(height))
}

/// Encodes the picture `object` again, in place, when that makes it smaller; whether it did. What
/// is wrong with the picture itself (it cannot be read, or decoded, or is not what it says) leaves
/// it as it is; what is wrong with writing the new one is an error: the copy would be left
/// half done.
fn encode_again(mut object: PdfObject, options: &Options) -> Result<bool, EngineError> {
    let Ok(Some(picture)) = read_picture(&object) else {
        return Ok(false);
    };
    // The samples as the PDF means them: whatever the filters, one byte each.
    let Ok(samples) = object.read_stream() else {
        return Ok(false);
    };
    let wanted = picture.width * picture.height * picture.channels;
    if u64::try_from(samples.len()) != Ok(wanted) {
        return Ok(false);
    }
    let (new_width, new_height) = target(picture.width, picture.height, options.max_side);
    let (Ok(channels), Ok(width), Ok(height), Ok(to_width), Ok(to_height)) = (
        usize::try_from(picture.channels),
        usize::try_from(picture.width),
        usize::try_from(picture.height),
        usize::try_from(new_width),
        usize::try_from(new_height),
    ) else {
        return Ok(false);
    };
    let shrunk;
    let pixels = if (to_width, to_height) == (width, height) {
        &samples[..]
    } else {
        shrunk = shrink(&samples, width, height, channels, to_width, to_height);
        &shrunk[..]
    };
    let Some(jpeg) = encode(pixels, to_width, to_height, channels, options.quality) else {
        return Ok(false);
    };
    let new_size = u64::try_from(jpeg.len()).unwrap_or(u64::MAX);
    if new_size.saturating_mul(100) > picture.stored.saturating_mul(WORTH_IT_PERCENT) {
        return Ok(false);
    }
    // What was made is read back as MuPDF would for a page: it is the size it should be.
    let Ok(check) = Image::from_bytes(&jpeg) else {
        return Ok(false);
    };
    if u64::from(check.width()) != new_width
        || u64::from(check.height()) != new_height
        || u64::from(check.n()) != picture.channels
    {
        return Ok(false);
    }
    drop(check);

    object.dict_put("Filter", PdfObject::new_name("DCTDecode")?)?;
    object.dict_delete("DecodeParms")?;
    object.dict_put(
        "Width",
        PdfObject::new_int(i32::try_from(new_width).unwrap_or(i32::MAX))?,
    )?;
    object.dict_put(
        "Height",
        PdfObject::new_int(i32::try_from(new_height).unwrap_or(i32::MAX))?,
    )?;
    object.dict_put("BitsPerComponent", PdfObject::new_int(8)?)?;
    // What the camera or the program that made the picture said of it was in its own metadata.
    object.dict_delete("Metadata")?;
    object.write_raw_stream_buffer(&Buffer::from_bytes(&jpeg)?)?;
    Ok(true)
}

/// `samples` (`channels` bytes a pixel, rows without padding) as a JPEG file at `quality`; none if
/// the encoder says no.
fn encode(
    samples: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    quality: ImageQuality,
) -> Option<Vec<u8>> {
    let (width, height) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
    let mut jpeg = Vec::new();
    let mut encoder = Encoder::new(&mut jpeg, jpeg_quality(quality));
    // Smaller files, and the same picture: only the tables are better fitted.
    encoder.set_optimized_huffman_tables(true);
    encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
    let color = if channels == 3 {
        ColorType::Rgb
    } else {
        ColorType::Luma
    };
    encoder.encode(samples, width, height, color).ok()?;
    Some(jpeg)
}

/// For each of `new` pieces of a line of `old` pixels, the first of the old ones it covers and how
/// much of each (the fractions add up to one): the new pixel is the average of what it covers.
fn spans(old: usize, new: usize) -> Vec<(usize, Vec<f32>)> {
    let step = old as f64 / new as f64;
    (0..new)
        .map(|at| {
            let (start, end) = (at as f64 * step, (at + 1) as f64 * step);
            let first = (start.floor() as usize).min(old - 1);
            let last = (end.ceil() as usize).clamp(first + 1, old);
            let weights = (first..last)
                .map(|pixel| {
                    let covered = end.min((pixel + 1) as f64) - start.max(pixel as f64);
                    (covered.max(0.0) / step) as f32
                })
                .collect();
            (first, weights)
        })
        .collect()
}

/// `samples`, `width` by `height` pixels of `channels` bytes each with no padding, as a picture of
/// `new_width` by `new_height` pixels (neither more than the old): each new pixel is the average of
/// the old ones it covers, in proportion to how much of each it covers.
fn shrink(
    samples: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    new_width: usize,
    new_height: usize,
) -> Vec<u8> {
    let columns = spans(width, new_width);
    let rows = spans(height, new_height);
    let line = new_width * channels;
    let mut out = vec![0u8; line * new_height];
    let mut sum = vec![0f32; line];
    for (row, (first, weights)) in rows.iter().enumerate() {
        sum.fill(0.0);
        for (offset, weight) in weights.iter().enumerate() {
            let source = &samples[(first + offset) * width * channels..][..width * channels];
            for (column, (start, across)) in columns.iter().enumerate() {
                for channel in 0..channels {
                    let mut average = 0f32;
                    for (at, part) in across.iter().enumerate() {
                        average += part * f32::from(source[(start + at) * channels + channel]);
                    }
                    sum[column * channels + channel] += weight * average;
                }
            }
        }
        for (slot, value) in out[row * line..][..line].iter_mut().zip(&sum) {
            *slot = value.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use mupdf::pdf::{InsertImageOptions, PageImageSource, PdfWriteOptions};
    use mupdf::{Colorspace, Pixmap, Rect};

    use super::*;
    use crate::engine::{EngineError as Failure, PdfDocument as Engine};

    const PAGE: (f32, f32) = (612.0, 792.0);

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

    /// A photograph that is no photograph: smooth colour with the same noise on every run, as
    /// full of detail as a real one for what a JPEG makes of it.
    fn photograph(width: usize, height: usize) -> Vec<u8> {
        let mut seed = 0x2545_f491u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 25) as i32 - 12
        };
        let mut samples = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let smooth = [
                    x * 255 / width,
                    y * 255 / height,
                    (x + y) * 255 / (width + height),
                ];
                for base in smooth {
                    samples.push((base as i32 + noise()).clamp(0, 255) as u8);
                }
            }
        }
        samples
    }

    /// The luma of `photograph`'s samples, as a gray picture.
    fn gray(samples: &[u8]) -> Vec<u8> {
        samples
            .as_chunks::<3>()
            .0
            .iter()
            .map(|rgb| {
                ((u32::from(rgb[0]) * 3 + u32::from(rgb[1]) * 6 + u32::from(rgb[2])) / 10) as u8
            })
            .collect()
    }

    fn jpeg_of(
        samples: &[u8],
        width: usize,
        height: usize,
        channels: usize,
        quality: u8,
    ) -> Vec<u8> {
        let mut jpeg = Vec::new();
        let color = if channels == 3 {
            ColorType::Rgb
        } else {
            ColorType::Luma
        };
        // As good a file as a camera or an editor makes: the tables fitted to the picture.
        let mut encoder = Encoder::new(&mut jpeg, quality);
        encoder.set_optimized_huffman_tables(true);
        encoder
            .encode(samples, width as u16, height as u16, color)
            .expect("encoded");
        jpeg
    }

    fn pixmap_of(samples: &[u8], width: usize, height: usize, channels: usize) -> Pixmap {
        let space = if channels == 3 {
            Colorspace::device_rgb()
        } else {
            Colorspace::device_gray()
        };
        let mut pixmap =
            Pixmap::new_with_w_h(&space, width as i32, height as i32, false).expect("pixmap");
        assert_eq!(
            pixmap.samples().len(),
            samples.len(),
            "no padding between rows"
        );
        pixmap.samples_mut().copy_from_slice(samples);
        pixmap
    }

    enum Source<'a> {
        Jpeg(&'a [u8]),
        Pixels(&'a Pixmap),
    }

    /// A document of one page with each of `pictures` on it, as MuPDF puts a picture into a page.
    fn document_with(pictures: &[Source]) -> PdfDocument {
        let mut doc = PdfDocument::new();
        let mut page = doc.new_page(PAGE).expect("new page");
        for (index, picture) in pictures.iter().enumerate() {
            let rect = Rect {
                x0: 20.0 + 10.0 * index as f32,
                y0: 20.0,
                x1: 320.0,
                y1: 240.0,
            };
            let source = match picture {
                Source::Jpeg(data) => PageImageSource::Bytes {
                    data,
                    format_hint: None,
                },
                Source::Pixels(pixmap) => PageImageSource::Pixmap(pixmap),
            };
            page.insert_image(&mut doc, rect, source, InsertImageOptions::default())
                .expect("picture put in");
        }
        drop(page);
        // Written and read again, as a file is: a picture is stored compressed.
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        options.set_compress(true);
        options.set_compress_images(true);
        let mut bytes = Vec::new();
        doc.write_to_with_options(&mut bytes, options)
            .expect("written");
        let file = mupdf::Document::from_bytes(&bytes, "application/pdf").expect("read");
        PdfDocument::try_from(file).expect("a PDF")
    }

    fn bytes_of(doc: &PdfDocument) -> Vec<u8> {
        let mut options = PdfWriteOptions::default();
        options.set_garbage(true);
        let mut out = Vec::new();
        doc.write_to_with_options(&mut out, options)
            .expect("written");
        out
    }

    /// One picture of a document, as it is stored.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Seen {
        number: i32,
        filter: Option<String>,
        width: i64,
        height: i64,
        stored: Vec<u8>,
        decoded: usize,
    }

    fn seen(doc: &PdfDocument) -> Vec<Seen> {
        (1..doc.xref_len().expect("count"))
            .filter_map(|number| {
                let number = i32::try_from(number).ok()?;
                let object = doc.new_indirect(number, 0).ok()?;
                if !is_image(&object) {
                    return None;
                }
                Some(Seen {
                    number,
                    filter: named(&object, "Filter")
                        .map(|name| String::from_utf8_lossy(&name).into_owned()),
                    width: whole_number(&object, "Width").ok()??,
                    height: whole_number(&object, "Height").ok()??,
                    stored: object.read_raw_stream().expect("stored"),
                    decoded: object.read_stream().map_or(0, |samples| samples.len()),
                })
            })
            .collect()
    }

    fn options(quality: ImageQuality, max_side: Option<u32>) -> Options {
        Options {
            quality,
            max_side,
            budget: BUDGET,
        }
    }

    /// The page of `bytes` drawn at 72 dpi, as RGB.
    fn drawn(bytes: &[u8]) -> Vec<u8> {
        let page = Engine::from_bytes(bytes)
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

    /// How far apart two drawn pages are: the mean difference of a sample.
    fn apart(one: &[u8], other: &[u8]) -> f64 {
        assert_eq!(one.len(), other.len());
        let total: u64 = one
            .iter()
            .zip(other)
            .map(|(one, other)| u64::from(one.abs_diff(*other)))
            .sum();
        total as f64 / one.len() as f64
    }

    #[test]
    fn the_sides_of_a_shrunk_picture_keep_its_shape_and_never_grow() {
        assert_eq!(target(4000, 3000, Some(2000)), (2000, 1500));
        assert_eq!(target(3000, 4000, Some(2000)), (1500, 2000));
        // Within the limit, or no limit: as it is.
        assert_eq!(target(1999, 1000, Some(2000)), (1999, 1000));
        assert_eq!(target(2000, 1000, Some(2000)), (2000, 1000));
        assert_eq!(target(4000, 3000, None), (4000, 3000));
        // A thin one keeps a pixel on its short side.
        assert_eq!(target(10_000, 3, Some(1000)), (1000, 1));
        assert_eq!(target(1, 10_000, Some(256)), (1, 256));
    }

    #[test]
    fn pixels_are_averaged_over_what_they_cover() {
        // 4 x 2 of one sample, shrunk to 2 x 1: the mean of each 2 x 2 block.
        let samples = [10, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(shrink(&samples, 4, 2, 1, 2, 1), vec![35, 55]);
        // To one pixel: the mean of all, three samples each.
        let rgb = [0, 100, 200, 100, 100, 0, 0, 100, 200, 100, 100, 0];
        assert_eq!(shrink(&rgb, 2, 2, 3, 1, 1), vec![50, 100, 100]);
        // Three to two: the middle pixel is shared, each new one takes two thirds of one old one
        // and a third of the middle one.
        assert_eq!(shrink(&[0, 90, 180], 3, 1, 1, 2, 1), vec![30, 150]);
    }

    #[test]
    fn a_flat_picture_stays_flat_whatever_it_is_shrunk_to() {
        for (width, height, to_width, to_height) in [
            (7, 5, 3, 2),
            (100, 3, 7, 3),
            (64, 64, 1, 1),
            (13, 29, 12, 28),
        ] {
            let samples = vec![77u8; width * height * 3];
            let shrunk = shrink(&samples, width, height, 3, to_width, to_height);
            assert_eq!(shrunk.len(), to_width * to_height * 3);
            assert!(
                shrunk.iter().all(|&sample| sample == 77),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn the_weights_of_each_new_pixel_add_up_to_one_and_leave_nothing_out() {
        for (old, new) in [(3, 2), (10, 3), (1000, 999), (65, 64), (5, 5), (7, 1)] {
            let spans = spans(old, new);
            assert_eq!(spans.len(), new);
            let mut covered = vec![0f32; old];
            for (first, weights) in &spans {
                let total: f32 = weights.iter().sum();
                assert!((total - 1.0).abs() < 1e-4, "{old} to {new}: {total}");
                assert!(first + weights.len() <= old);
                for (at, weight) in weights.iter().enumerate() {
                    covered[first + at] += weight;
                }
            }
            // Every old pixel is used as much as the others, a new pixel's share of it.
            let share = new as f32 / old as f32;
            assert!(
                covered.iter().all(|&used| (used - share).abs() < 1e-3),
                "{old} to {new}: {covered:?}"
            );
        }
    }

    #[test]
    fn each_quality_has_a_number_and_a_better_one_is_a_higher_one() {
        assert!(jpeg_quality(ImageQuality::High) > jpeg_quality(ImageQuality::Medium));
        assert!(jpeg_quality(ImageQuality::Medium) > jpeg_quality(ImageQuality::Low));
        assert!(jpeg_quality(ImageQuality::Low) >= 30);
        assert!(jpeg_quality(ImageQuality::High) < 90);
    }

    #[test]
    fn a_photograph_is_encoded_again_much_smaller_and_looks_the_same() {
        let samples = photograph(1200, 900);
        let original = jpeg_of(&samples, 1200, 900, 3, 95);
        let doc = document_with(&[Source::Jpeg(&original)]);
        let before = bytes_of(&doc);
        let drawn_before = drawn(&before);
        assert!(
            contains(&before, &original[..64]),
            "the control: its own bytes"
        );

        let report = recompress(&doc, options(ImageQuality::Medium, Some(600))).expect("done");
        assert_eq!(
            report,
            Report {
                images: 1,
                changed: 1,
                partial: false
            }
        );
        let [picture] = &seen(&doc)[..] else {
            panic!("one picture")
        };
        assert_eq!(picture.filter.as_deref(), Some("DCTDecode"));
        assert_eq!((picture.width, picture.height), (600, 450));
        // The decoder agrees: 600 x 450 pixels of three samples.
        assert_eq!(picture.decoded, 600 * 450 * 3);
        let after = bytes_of(&doc);
        assert!(
            after.len() * 3 < before.len(),
            "{} bytes from {}",
            after.len(),
            before.len()
        );
        // The page looks as it did: it is a few levels apart in a sample.
        let difference = apart(&drawn_before, &drawn(&after));
        assert!(difference < 6.0, "{difference}");
        // A worse quality is a smaller file, and a better one a larger one.
        let size_at = |quality| {
            let doc = document_with(&[Source::Jpeg(&original)]);
            recompress(&doc, options(quality, Some(600))).expect("done");
            seen(&doc)[0].stored.len()
        };
        assert!(size_at(ImageQuality::Low) < size_at(ImageQuality::Medium));
        assert!(size_at(ImageQuality::Medium) < size_at(ImageQuality::High));
    }

    #[test]
    fn a_picture_within_the_limit_keeps_its_size_and_one_stored_plain_becomes_a_jpeg() {
        let samples = photograph(800, 600);
        let pixmap = pixmap_of(&samples, 800, 600, 3);
        let doc = document_with(&[Source::Pixels(&pixmap)]);
        let [before] = &seen(&doc)[..] else {
            panic!("one picture")
        };
        assert_eq!(before.filter.as_deref(), Some("FlateDecode"));
        let report = recompress(&doc, options(ImageQuality::Medium, Some(2000))).expect("done");
        assert_eq!((report.images, report.changed), (1, 1));
        let [after] = &seen(&doc)[..] else {
            panic!("one picture")
        };
        assert_eq!(after.filter.as_deref(), Some("DCTDecode"));
        assert_eq!((after.width, after.height), (800, 600));
        assert_eq!(after.decoded, 800 * 600 * 3);
        assert!(after.stored.len() * 4 < before.stored.len());

        // The same of a gray one: one sample a pixel.
        let gray_samples = gray(&samples);
        let pixmap = pixmap_of(&gray_samples, 800, 600, 1);
        let doc = document_with(&[Source::Pixels(&pixmap)]);
        let report = recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert_eq!((report.images, report.changed), (1, 1));
        let [after] = &seen(&doc)[..] else {
            panic!("one picture")
        };
        assert_eq!(after.filter.as_deref(), Some("DCTDecode"));
        assert_eq!(after.decoded, 800 * 600);
    }

    #[test]
    fn a_picture_that_would_not_get_smaller_is_left_as_it_is() {
        // Already a poor JPEG, with tables as good as can be: a better one is larger.
        let samples = photograph(200, 150);
        let poor = jpeg_of(&samples, 200, 150, 3, 30);
        let doc = document_with(&[Source::Jpeg(&poor)]);
        let before = seen(&doc);
        let report = recompress(&doc, options(ImageQuality::High, None)).expect("done");
        assert_eq!((report.images, report.changed), (1, 0));
        assert_eq!(seen(&doc), before);

        // Fine lines, sharp as a drawing or a screenshot: stored plain they are a few kilobytes, as
        // a JPEG many times that.
        let mut lines = vec![255u8; 800 * 600 * 3];
        for (at, sample) in lines.iter_mut().enumerate() {
            let (x, y) = (at / 3 % 800, at / 3 / 800);
            if x % 4 == 0 || y % 4 == 0 {
                *sample = 20;
            }
        }
        let pixmap = pixmap_of(&lines, 800, 600, 3);
        let doc = document_with(&[Source::Pixels(&pixmap)]);
        let before = seen(&doc);
        let report = recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert_eq!((report.images, report.changed), (1, 0));
        assert_eq!(seen(&doc), before);
        assert_eq!(before[0].filter.as_deref(), Some("FlateDecode"));
    }

    #[test]
    fn the_mask_of_a_picture_is_left_as_it_is_and_is_no_picture_of_its_own() {
        let samples = photograph(400, 300);
        let space = Colorspace::device_rgb();
        let mut with_alpha = Pixmap::new_with_w_h(&space, 400, 300, true).expect("pixmap");
        for (pixel, rgb) in with_alpha
            .samples_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(samples.as_chunks::<3>().0)
        {
            pixel[..3].copy_from_slice(rgb);
            pixel[3] = 200;
        }
        let doc = document_with(&[Source::Pixels(&with_alpha)]);
        let before = seen(&doc);
        assert_eq!(before.len(), 2, "the picture and its mask");
        let report = recompress(&doc, options(ImageQuality::Medium, Some(300))).expect("done");
        assert_eq!((report.images, report.changed), (1, 1));
        let after = seen(&doc);
        let changed: Vec<_> = after
            .iter()
            .filter(|one| one.filter.as_deref() == Some("DCTDecode"))
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!((changed[0].width, changed[0].height), (300, 225));
        // The other, the mask, is what it was.
        let mask_before = before
            .iter()
            .find(|one| one.number != changed[0].number)
            .expect("mask");
        let mask_after = after
            .iter()
            .find(|one| one.number != changed[0].number)
            .expect("mask");
        assert_eq!(mask_before, mask_after);
        // It still belongs to the picture it was the mask of.
        let picture = doc.new_indirect(changed[0].number, 0).expect("object");
        let mask = picture.get_dict("SMask").expect("entry").expect("a mask");
        assert_eq!(mask.as_indirect().expect("reference"), mask_after.number);
    }

    /// An image XObject made by hand: `entries` are the dictionary's, written after `/Type /XObject
    /// /Subtype /Image`, and `data` the stream as it is stored.
    fn handmade(doc: &mut PdfDocument, entries: &str, data: &[u8]) -> i32 {
        let dictionary = doc
            .new_object_from_str(&format!("<< /Type /XObject /Subtype /Image {entries} >>"))
            .expect("dictionary");
        let stream = doc
            .add_stream(
                &Buffer::from_bytes(data).expect("buffer"),
                Some(&dictionary),
                true,
            )
            .expect("stream");
        stream.as_indirect().expect("reference")
    }

    #[test]
    fn what_cannot_be_encoded_again_without_changing_what_it_means_is_left_as_it_is() {
        let mut doc = PdfDocument::new();
        let samples = photograph(300, 200);
        // A palette, a stencil, 16 bits a sample, four colours, a colour key mask, a scan (CCITT) and
        // a JPEG 2000 file, a picture too large, one with no colour space, and one of Lab colours:
        // none of them is touched, though each has data a JPEG would make smaller.
        let ignored = [
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 255 <00>]",
            "/Width 300 /Height 200 /BitsPerComponent 1 /ImageMask true",
            "/Width 300 /Height 200 /BitsPerComponent 16 /ColorSpace /DeviceGray",
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceCMYK",
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Mask [0 0 0 0 0 0]",
            "/Width 300 /Height 200 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode",
            "/Width 300 /Height 200 /ColorSpace /DeviceRGB /Filter /JPXDecode",
            "/Width 30000 /Height 30000 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            "/Width 300 /Height 200 /BitsPerComponent 8",
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace [/Lab << /WhitePoint [0.95 1 1.09] >>]",
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter [/FlateDecode /Crypt]",
            "/Width 0 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
        ];
        let numbers: Vec<i32> = ignored
            .iter()
            .map(|entries| handmade(&mut doc, entries, &samples))
            .collect();
        // The control: the same data, said to be what it is.
        let control = handmade(
            &mut doc,
            "/Width 100 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            &samples[..100 * 200 * 3],
        );
        let before = seen(&doc);
        let report = recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert_eq!(usize::try_from(report.images).ok(), Some(ignored.len() + 1));
        assert_eq!(report.changed, 1);
        let after = seen(&doc);
        for number in numbers {
            let was = before
                .iter()
                .find(|one| one.number == number)
                .expect("there");
            let is = after
                .iter()
                .find(|one| one.number == number)
                .expect("there");
            assert_eq!(was, is, "picture {number}");
        }
        let was = before
            .iter()
            .find(|one| one.number == control)
            .expect("there");
        let is = after
            .iter()
            .find(|one| one.number == control)
            .expect("there");
        assert_ne!(was, is);
        assert_eq!(is.filter.as_deref(), Some("DCTDecode"));
    }

    #[test]
    fn a_picture_that_cannot_be_decoded_is_left_as_it_is_and_the_others_are_done() {
        let mut doc = PdfDocument::new();
        let samples = photograph(300, 200);
        let broken = handmade(
            &mut doc,
            "/Width 64 /Height 64 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode",
            &[0x42; 100],
        );
        // Says it is larger than what it holds, and has no data at all.
        let short = handmade(
            &mut doc,
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            &samples[..1000],
        );
        let empty = handmade(
            &mut doc,
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            &[],
        );
        let fine = handmade(
            &mut doc,
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            &samples,
        );
        let before = seen(&doc);
        let report = recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert_eq!((report.images, report.changed), (4, 1));
        let after = seen(&doc);
        for number in [broken, short, empty] {
            let was = before
                .iter()
                .find(|one| one.number == number)
                .expect("there");
            let is = after
                .iter()
                .find(|one| one.number == number)
                .expect("there");
            assert_eq!(was, is, "picture {number}");
        }
        let is = after.iter().find(|one| one.number == fine).expect("there");
        assert_eq!(is.filter.as_deref(), Some("DCTDecode"));
    }

    #[test]
    fn what_a_camera_wrote_into_a_picture_is_not_in_the_one_made_from_it() {
        const SECRET: &[u8] = b"Canon EOS-TEST 5D GPS 25.0330N 121.5654E";
        let samples = photograph(640, 480);
        let plain = jpeg_of(&samples, 640, 480, 3, 92);
        // The EXIF segment (APP1) comes right after the start of the file.
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(SECRET);
        let mut with_exif = plain[..2].to_vec();
        with_exif.extend_from_slice(&[0xFF, 0xE1]);
        with_exif.extend_from_slice(&u16::try_from(exif.len() + 2).expect("short").to_be_bytes());
        with_exif.extend_from_slice(&exif);
        with_exif.extend_from_slice(&plain[2..]);

        let doc = document_with(&[Source::Jpeg(&with_exif)]);
        // The control: MuPDF keeps a picture as it is.
        assert!(contains(&bytes_of(&doc), SECRET));
        let report = recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert_eq!(report.changed, 1);
        let saved = bytes_of(&doc);
        assert!(!contains(&saved, SECRET));
        assert!(!contains(&saved, b"Exif"));
    }

    #[test]
    fn the_metadata_of_a_picture_that_is_replaced_goes_with_it() {
        let samples = photograph(300, 200);
        let mut doc = PdfDocument::new();
        let number = handmade(
            &mut doc,
            "/Width 300 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            &samples,
        );
        let mut object = doc.new_indirect(number, 0).expect("object");
        let metadata = doc
            .new_object_from_str("<< /Type /Metadata /Subtype /XML >>")
            .expect("dictionary");
        let metadata = doc
            .add_stream(
                &Buffer::from_bytes(b"<x:xmpmeta>Secret Camera</x:xmpmeta>").expect("buffer"),
                Some(&metadata),
                true,
            )
            .expect("stream");
        object.dict_put("Metadata", metadata).expect("put");
        // Something refers to the picture, or the writer would leave it out.
        doc.catalog()
            .expect("catalog")
            .dict_put("Probe", doc.new_indirect(number, 0).expect("reference"))
            .expect("put");
        assert!(contains(&bytes_of(&doc), b"Secret Camera"));
        recompress(&doc, options(ImageQuality::Medium, None)).expect("done");
        assert!(!contains(&bytes_of(&doc), b"Secret Camera"));
    }

    #[test]
    fn a_limit_that_is_not_a_size_is_refused_before_anything_is_done() {
        let samples = photograph(300, 200);
        let pixmap = pixmap_of(&samples, 300, 200, 3);
        for side in [
            0,
            1,
            MIN_COMPRESS_SIDE_PX - 1,
            MAX_COMPRESS_SIDE_PX + 1,
            u32::MAX,
        ] {
            let doc = document_with(&[Source::Pixels(&pixmap)]);
            let before = seen(&doc);
            assert!(matches!(
                recompress(&doc, options(ImageQuality::Medium, Some(side))),
                Err(Failure::InvalidEdit(_))
            ));
            assert_eq!(seen(&doc), before);
        }
        let doc = document_with(&[Source::Pixels(&pixmap)]);
        for side in [MIN_COMPRESS_SIDE_PX, MAX_COMPRESS_SIDE_PX] {
            assert!(recompress(&doc, options(ImageQuality::Medium, Some(side))).is_ok());
        }
    }

    #[test]
    fn when_the_time_is_up_the_pictures_left_are_left_as_they_are() {
        let samples = photograph(300, 200);
        let pixmap = pixmap_of(&samples, 300, 200, 3);
        let doc = document_with(&[Source::Pixels(&pixmap), Source::Pixels(&pixmap)]);
        let before = seen(&doc);
        let report = recompress(
            &doc,
            Options {
                quality: ImageQuality::Medium,
                max_side: None,
                budget: Duration::ZERO,
            },
        )
        .expect("done");
        assert!(report.partial);
        assert_eq!(report.changed, 0);
        assert!(report.images >= 1);
        assert_eq!(seen(&doc), before);
    }

    #[test]
    fn a_compressed_copy_is_smaller_opens_again_and_the_document_is_as_it_was() {
        let samples = photograph(1000, 750);
        let original = jpeg_of(&samples, 1000, 750, 3, 95);
        let bytes = bytes_of(&document_with(&[Source::Jpeg(&original)]));
        let doc = Engine::from_bytes(&bytes).expect("open");
        let page_before = doc.render(0, 1.0, 0).expect("rendered");

        let mut copy = Vec::new();
        let made = doc
            .compressed_copy(ImageQuality::Medium, Some(500), &mut copy)
            .expect("compressed");
        assert_eq!(made.bytes, copy.len() as u64);
        assert_eq!(
            (made.report.images, made.report.changed, made.report.partial),
            (1, 1, false)
        );
        assert!(
            made.bytes * 3 < made.before,
            "{} from {}",
            made.bytes,
            made.before
        );
        // It is a document, the same page, drawn nearly as it was.
        let again = Engine::from_bytes(&copy).expect("opens");
        assert_eq!(again.page_count().expect("pages"), 1);
        assert!(apart(&drawn(&bytes), &drawn(&copy)) < 6.0);
        // The open document is as it was: its page is drawn from the picture it had.
        assert_eq!(doc.render(0, 1.0, 0).expect("rendered"), page_before);
        // And a copy of nothing in particular is still a copy: no picture, no change.
        let plain = Engine::from_bytes(&corpus("benign/mixed-text-zh-en.pdf")).expect("open");
        let mut copy = Vec::new();
        let made = plain
            .compressed_copy(ImageQuality::Low, Some(256), &mut copy)
            .expect("compressed");
        assert_eq!(
            (made.report.images, made.report.changed, made.report.partial),
            (0, 0, false)
        );
        assert!(Engine::from_bytes(&copy).is_ok());
    }

    #[test]
    fn an_encrypted_document_and_a_signed_one_have_no_compressed_copy() {
        let encrypted =
            Engine::open(&corpus("benign/encrypted-aes256.pdf"), Some("user")).expect("open");
        assert!(matches!(
            encrypted.compressed_copy(ImageQuality::Medium, None, &mut Vec::new()),
            Err(Failure::EncryptedCopy)
        ));
        let signed = Engine::from_bytes(&corpus("benign/signed.pdf")).expect("open");
        assert!(signed.is_signed());
        let mut out = Vec::new();
        assert!(matches!(
            signed.compressed_copy(ImageQuality::Medium, None, &mut out),
            Err(Failure::InvalidEdit(_))
        ));
        assert!(out.is_empty(), "nothing was written");
    }
}
