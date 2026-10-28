//! Pictures for custom stamps (B2-08): a PNG or JPEG file is made into the PNG a stamp is
//! made of, and such a PNG is made into a stamp annotation.
//!
//! The picture is untrusted input, decoded here in the sandboxed worker and nowhere else: only
//! PNG and JPEG are accepted (by their first bytes), each side and the pixel count are bounded
//! before any pixel is decoded, and the file is read no further than its limit. What comes out
//! is the pixels alone, encoded again: the decoder keeps nothing of the file metadata (EXIF,
//! the GPS position, camera and time, text chunks, color profiles), and so the PDF has none.

use ipc_contract::limits::{
    MAX_PICTURE_ANGLE_DEGREES, MAX_STAMP_PNG_BYTES, MAX_STAMP_SIDE_PX, MAX_STAMP_SOURCE_BYTES,
    MAX_STAMP_SOURCE_PIXELS, MAX_STAMP_SOURCE_SIDE_PX,
};
use ipc_contract::types::{AnnotationId, Rect};
use ipc_contract::validate::{JPEG_SIGNATURE, PNG_SIGNATURE, stamp_png_size};
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType, PdfObject};
use mupdf::{Buffer, Image, ImageFormat, Pixmap};

use super::annotations::{binding_rect, find};
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
    // A PNG says how large it is in its first chunk, and its chunks tell whether it is whole
    // (MuPDF turns one without pixel data into an empty picture): both are read here, before
    // MuPDF sees it.
    if let Some((width, height)) = png_header_size(bytes) {
        check_size(width, height)?;
        if !png_is_whole(bytes) {
            return Err(EngineError::InvalidPicture("the PNG is cut short"));
        }
    }
    // MuPDF reads the header; the pixels are decoded below, once the size is known to be fine.
    let image = Image::from_bytes(bytes)
        .map_err(|_| EngineError::InvalidPicture("the picture cannot be read"))?;
    check_size(image.width(), image.height())?;
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

/// The size a PNG file says it has (its header chunk comes first), without reading any more.
fn png_header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let header = bytes.get(8..24)?;
    if !bytes.starts_with(&PNG_SIGNATURE) || &header[4..8] != b"IHDR" {
        return None;
    }
    let side = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Some((side(8), side(12)))
}

/// Whether a PNG file has its header chunk first, some pixel data, and its end chunk: the chunks
/// are walked by their lengths, nothing is decoded.
fn png_is_whole(bytes: &[u8]) -> bool {
    let mut at = 8;
    let mut first = true;
    let mut pixels = false;
    while let Some(header) = bytes.get(at..at + 8) {
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        if first != (kind == b"IHDR") {
            return false;
        }
        first = false;
        pixels |= kind == b"IDAT";
        // The data and the check value must be there.
        let end = at.saturating_add(12).saturating_add(length);
        if end > bytes.len() {
            return false;
        }
        if kind == b"IEND" {
            return pixels;
        }
        at = end;
    }
    false
}

/// Refuses a picture of no size, or of more than the worker decodes.
fn check_size(width: u32, height: u32) -> Result<(), EngineError> {
    if width == 0
        || height == 0
        || width > MAX_STAMP_SOURCE_SIDE_PX
        || height > MAX_STAMP_SOURCE_SIDE_PX
        || u64::from(width) * u64::from(height) > MAX_STAMP_SOURCE_PIXELS
    {
        return Err(EngineError::PictureTooLarge);
    }
    Ok(())
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
    ) -> Result<AnnotationId, EngineError> {
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
        let id = u32::try_from(annotation.xref()?).map_err(|_| EngineError::TooComplex)?;
        Ok(AnnotationId(id))
    }
}

/// How the picture of a stamp the app made is set on its page (B3-01), as the stamp itself says
/// it: the form of its appearance draws the picture over its box, turned by the form's matrix, and
/// the viewer fits the turned box to the annotation's rectangle. Lengths are in points, in the
/// space of the PDF (the page's own, before its `/Rotate`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Setting {
    /// The picture when upright.
    pub(super) width: f32,
    pub(super) height: f32,
    /// Counter-clockwise in radians (the PDF's y axis points up).
    pub(super) angle: f32,
    /// `/Rect`, upright: x0, y0, x1, y1.
    pub(super) rect: [f32; 4],
}

/// How close to a quarter turn an angle has to be to count as one, in radians.
const QUARTER_TOLERANCE: f32 = 1e-3;

/// The setting of a picture whose form has the box `bbox` and the matrix `matrix`, drawn in the
/// annotation rectangle `rect`; `None` if they cannot be a picture.
fn measure(bbox: [f32; 4], matrix: [f32; 6], rect: [f32; 4]) -> Option<Setting> {
    let (box_width, box_height) = ((bbox[2] - bbox[0]).abs(), (bbox[3] - bbox[1]).abs());
    // The box that holds the form once the matrix has turned it.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[3]),
    ] {
        let turned = (
            matrix[0] * x + matrix[2] * y + matrix[4],
            matrix[1] * x + matrix[3] * y + matrix[5],
        );
        (x0, x1) = (x0.min(turned.0), x1.max(turned.0));
        (y0, y1) = (y0.min(turned.1), y1.max(turned.1));
    }
    let (held_width, held_height) = (x1 - x0, y1 - y0);
    let (rect_width, rect_height) = (rect[2] - rect[0], rect[3] - rect[1]);
    if [
        box_width,
        box_height,
        held_width,
        held_height,
        rect_width,
        rect_height,
    ]
    .iter()
    .any(|side| !side.is_finite() || *side <= 1e-6)
    {
        return None;
    }
    let angle = matrix[1].atan2(matrix[0]);
    let (across, down) = (rect_width / held_width, rect_height / held_height);
    let half_turns = angle.abs() < QUARTER_TOLERANCE
        || (angle.abs() - std::f32::consts::PI).abs() < QUARTER_TOLERANCE;
    let quarter = (angle.abs() - std::f32::consts::FRAC_PI_2).abs() < QUARTER_TOLERANCE;
    // Upright or upside down, the picture's sides lie along the box's; a quarter turn swaps them;
    // otherwise the box was fitted with one scale (the app keeps the shape).
    let (width, height) = if half_turns {
        (box_width * across, box_height * down)
    } else if quarter {
        (box_width * down, box_height * across)
    } else {
        let scale = (across + down) / 2.0;
        (box_width * scale, box_height * scale)
    };
    (width.is_finite() && height.is_finite()).then_some(Setting {
        width,
        height,
        angle,
        rect,
    })
}

/// A number the PDF object `object` is, if it is one.
fn number(object: &PdfObject) -> Option<f32> {
    let object = object.resolve().ok().flatten()?;
    object.is_number().ok().filter(|&is| is)?;
    object.as_float().ok().filter(|value| value.is_finite())
}

/// The `N` numbers of the array `object` is, if it is an array of just those.
fn numbers<const N: usize>(object: Option<PdfObject>) -> Option<[f32; N]> {
    let array = object?.resolve().ok().flatten()?;
    if !array.is_array().ok()? || array.len().ok()? != N {
        return None;
    }
    let mut found = [0.0; N];
    for (at, slot) in found.iter_mut().enumerate() {
        *slot = number(&array.get_array(i32::try_from(at).ok()?).ok().flatten()?)?;
    }
    Some(found)
}

/// What `key` of `object` is, followed through a reference.
pub(super) fn entry_of(object: &PdfObject, key: &str) -> Option<PdfObject> {
    object
        .get_dict(key)
        .ok()
        .flatten()?
        .resolve()
        .ok()
        .flatten()
}

/// Whether the content of an appearance form does nothing but draw its one picture, the way
/// the app and MuPDF write it: `/I Do`, perhaps scaled by a matrix and between `q` and `Q`. A
/// stamp of another maker, that draws more or draws it otherwise, is not turned: its appearance
/// would be lost.
fn draws_only_the_picture(content: &[u8]) -> bool {
    if content.len() > 256 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(content) else {
        return false;
    };
    let mut tokens: Vec<&str> = text.split_ascii_whitespace().collect();
    if tokens.first() == Some(&"q") {
        if tokens.last() != Some(&"Q") {
            return false;
        }
        tokens.remove(0);
        tokens.pop();
    }
    if tokens.len() >= 2 && tokens[tokens.len() - 1] == "Do" {
        tokens.pop();
        let Some(name) = tokens.pop() else {
            return false;
        };
        if name != "/I" {
            return false;
        }
        // What is left is a matrix: six numbers and `cm`, or nothing.
        return tokens.is_empty()
            || (tokens.len() == 7
                && tokens[6] == "cm"
                && tokens[..6]
                    .iter()
                    .all(|number| number.parse::<f32>().is_ok()));
    }
    false
}

/// The setting of `annotation` and the reference to its picture, if it is a stamp made from a
/// picture, as the app makes it and MuPDF keeps it: its appearance form draws one image, and
/// nothing else (MuPDF names a stamp's image `I`).
pub(super) fn read_setting(annotation: &PdfAnnotation) -> Option<(Setting, PdfObject)> {
    let object = annotation.object();
    // The form as the annotation refers to it: a stream is only one through its reference.
    let form_ref = entry_of(&object, "AP")?.get_dict("N").ok().flatten()?;
    let form = form_ref.resolve().ok().flatten()?;
    let pictures = entry_of(&entry_of(&form, "Resources")?, "XObject")?;
    if pictures.dict_len().ok()? != 1 {
        return None;
    }
    let picture = pictures.get_dict_val(0).ok().flatten()?;
    if !picture.is_indirect().ok()? {
        return None;
    }
    let image = picture.resolve().ok().flatten()?;
    if entry_of(&image, "Subtype")?.as_name().ok()? != b"Image" {
        return None;
    }
    if !draws_only_the_picture(&form_ref.read_stream().ok()?) {
        return None;
    }
    let bbox = numbers::<4>(form.get_dict("BBox").ok().flatten())?;
    let matrix = numbers::<6>(form.get_dict("Matrix").ok().flatten())
        .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let rect = numbers::<4>(object.get_dict("Rect").ok().flatten())?;
    let rect = [
        rect[0].min(rect[2]),
        rect[1].min(rect[3]),
        rect[0].max(rect[2]),
        rect[1].max(rect[3]),
    ];
    Some((measure(bbox, matrix, rect)?, picture))
}

/// What the page is told of a picture stamp (B3-01): the picture upright is `width` by `height`
/// as the page shows it, which a page turned a quarter or three quarters swaps, and is turned
/// `angle` degrees clockwise.
pub(super) fn picture_view(
    annotation: &PdfAnnotation,
    page_rotation: i32,
) -> Option<ipc_contract::types::PictureView> {
    let (setting, _) = read_setting(annotation)?;
    let (width, height) = if page_rotation.rem_euclid(180) == 90 {
        (setting.height, setting.width)
    } else {
        (setting.width, setting.height)
    };
    let mut angle = -setting.angle.to_degrees();
    while angle > 180.0 {
        angle -= 360.0;
    }
    while angle <= -180.0 {
        angle += 360.0;
    }
    // Not minus zero, nor noise of a turn that was a whole number of degrees.
    let angle = (angle * 1000.0).round() / 1000.0 + 0.0;
    Some(ipc_contract::types::PictureView {
        angle,
        width,
        height,
    })
}

/// A picture of `width` by `height` turned `degrees` clockwise about `centre` on a page of box
/// `page`: the form's size and matrix, and the rectangle that holds it. A picture that would not
/// fit the page turned is made smaller, in shape; one that fits is moved in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Turn {
    width: f32,
    height: f32,
    rect: [f32; 4],
    matrix: [f32; 6],
}

fn turn(width: f32, height: f32, centre: (f32, f32), page: [f32; 4], degrees: f32) -> Turn {
    // The PDF's y axis points up, so what looks clockwise is a negative angle.
    let (sin, cos) = (-degrees.to_radians()).sin_cos();
    let held = |w: f32, h: f32| (w * cos.abs() + h * sin.abs(), w * sin.abs() + h * cos.abs());
    let (mut width, mut height) = (width, height);
    let (mut held_width, mut held_height) = held(width, height);
    let (page_width, page_height) = (page[2] - page[0], page[3] - page[1]);
    if held_width > page_width || held_height > page_height {
        let shrink = (page_width / held_width).min(page_height / held_height);
        width *= shrink;
        height *= shrink;
        held_width *= shrink;
        held_height *= shrink;
    }
    let x = centre
        .0
        .clamp(page[0] + held_width / 2.0, page[2] - held_width / 2.0);
    let y = centre
        .1
        .clamp(page[1] + held_height / 2.0, page[3] - held_height / 2.0);
    Turn {
        width,
        height,
        rect: [
            x - held_width / 2.0,
            y - held_height / 2.0,
            x + held_width / 2.0,
            y + held_height / 2.0,
        ],
        matrix: [cos, sin, -sin, cos, 0.0, 0.0],
    }
}

/// The box of `page` in the PDF's space: its crop box, else its media box, else a letter page.
fn page_box(page: &mupdf::pdf::PdfPage) -> [f32; 4] {
    let object = page.object();
    for key in ["CropBox", "MediaBox"] {
        if let Ok(found) = object.get_dict_inheritable(key)
            && let Some(rect) = numbers::<4>(found)
        {
            let rect = [
                rect[0].min(rect[2]),
                rect[1].min(rect[3]),
                rect[0].max(rect[2]),
                rect[1].max(rect[3]),
            ];
            if rect[2] - rect[0] > 0.0 && rect[3] - rect[1] > 0.0 {
                return rect;
            }
        }
    }
    [0.0, 0.0, 612.0, 792.0]
}

impl PdfDocument {
    /// Turns the picture stamp `id` of page `index` to `degrees` clockwise from upright, about its
    /// centre (B3-01). The picture keeps its size; the rectangle becomes the box that holds it,
    /// kept on the page; the appearance is made again over the same picture (a new form with a
    /// matrix), so no picture is copied. Only a stamp the app made from a picture can be turned.
    pub fn set_picture_angle(
        &mut self,
        index: u32,
        id: AnnotationId,
        degrees: f32,
    ) -> Result<(), EngineError> {
        if !degrees.is_finite() || degrees.abs() > MAX_PICTURE_ANGLE_DEGREES {
            return Err(EngineError::InvalidEdit("not an angle"));
        }
        let page = self.pdf_page(index)?;
        let annotation = find(&page, id)?;
        let not_a_picture = EngineError::InvalidEdit("only a picture of your own can be turned");
        if annotation.r#type()? != PdfAnnotationType::Stamp {
            return Err(not_a_picture);
        }
        let Some((setting, picture)) = read_setting(&annotation) else {
            return Err(not_a_picture);
        };
        let centre = (
            (setting.rect[0] + setting.rect[2]) / 2.0,
            (setting.rect[1] + setting.rect[3]) / 2.0,
        );
        let turned = turn(
            setting.width,
            setting.height,
            centre,
            page_box(&page),
            degrees,
        );

        // Named `I`, as MuPDF names the image of a stamp.
        let mut images = self.doc.new_dict()?;
        images.dict_put("I", picture)?;
        let mut resources = self.doc.new_dict()?;
        resources.dict_put("XObject", images)?;
        let mut bounds = self.doc.new_array()?;
        for side in [0.0, 0.0, turned.width, turned.height] {
            bounds.array_push(PdfObject::new_real(side)?)?;
        }
        let mut matrix = self.doc.new_array()?;
        for value in turned.matrix {
            matrix.array_push(PdfObject::new_real(value)?)?;
        }
        let mut form = self.doc.new_dict()?;
        form.dict_put("Type", PdfObject::new_name("XObject")?)?;
        form.dict_put("Subtype", PdfObject::new_name("Form")?)?;
        form.dict_put("BBox", bounds)?;
        form.dict_put("Matrix", matrix)?;
        form.dict_put("Resources", resources)?;
        let content = Buffer::from_bytes(
            format!(
                "q {:.4} 0 0 {:.4} 0 0 cm /I Do Q",
                turned.width, turned.height
            )
            .as_bytes(),
        )?;
        let stream = self.doc.add_stream(&content, Some(&form), false)?;
        let mut appearance = self.doc.new_dict()?;
        appearance.dict_put("N", stream)?;

        // Changed key by key in the annotation's own dictionary, in the PDF's space, as the
        // stamp was made: nothing converts the rectangle between spaces.
        let mut object = annotation.object();
        let mut rect = self.doc.new_array()?;
        for side in turned.rect {
            rect.array_push(PdfObject::new_real(side)?)?;
        }
        object.dict_put("AP", appearance)?;
        object.dict_put("Rect", rect)?;
        Ok(())
    }
}

/// For a picture stamp that is turned (B3-01), the rectangle `wanted` (page space) fitted to the
/// shape of its box: the largest box of that shape that fits, about the middle of `wanted`. The
/// page keeps a turned picture's shape however a box is dragged. `None` for any other stamp. A
/// turned picture is placed with its appearance as it is: asked to update it, MuPDF draws the
/// picture upright again over the rectangle.
pub(super) fn fit_turned(annotation: &PdfAnnotation, wanted: Rect) -> Option<Rect> {
    let (setting, _) = read_setting(annotation)?;
    if setting.angle.abs() < QUARTER_TOLERANCE {
        return None;
    }
    let now = annotation.bounds().ok()?;
    let (now_width, now_height) = (now.x1 - now.x0, now.y1 - now.y0);
    let (want_width, want_height) = (wanted.x1 - wanted.x0, wanted.y1 - wanted.y0);
    if now_width <= 0.0 || now_height <= 0.0 {
        return None;
    }
    let scale = (want_width / now_width).min(want_height / now_height);
    let (middle_x, middle_y) = ((wanted.x0 + wanted.x1) / 2.0, (wanted.y0 + wanted.y1) / 2.0);
    Some(Rect {
        x0: middle_x - now_width * scale / 2.0,
        y0: middle_y - now_height * scale / 2.0,
        x1: middle_x + now_width * scale / 2.0,
        y1: middle_y + now_height * scale / 2.0,
    })
}

#[cfg(test)]
mod tests {
    use ipc_contract::types::{AnnotationKind, PageAnnotation, Point, StampName};
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
        // Cut short, a picture is an error, not a crash: in its header always, and a PNG in its
        // pixels too. (A JPEG cut in its pixels is shown as far as it goes, as everywhere.)
        for name in ["images/stamp-exif.jpg", "images/stamp-metadata.png"] {
            let whole = corpus(name);
            for keep in [10, 33, 40, 100] {
                assert!(
                    prepare_stamp_picture(&whole[..keep]).is_err(),
                    "{name} cut at {keep}"
                );
            }
            if name.ends_with(".png") {
                for keep in [whole.len() / 2, whole.len() - 12, whole.len() - 1] {
                    assert!(
                        matches!(
                            prepare_stamp_picture(&whole[..keep]),
                            Err(EngineError::InvalidPicture(_))
                        ),
                        "{name} cut at {keep}"
                    );
                }
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

    /// The 64 x 32 sample as a stamp over 128 x 64 points, 100 points from the left and the top of
    /// a letter page: its centre is (164, 132).
    fn stamped(file: &str) -> (PdfDocument, AnnotationId) {
        let picture = prepare_stamp_picture(&corpus("images/stamp-exif.jpg")).expect("prepared");
        let mut doc = PdfDocument::from_bytes(&corpus(file)).expect("open");
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
        let id = stamp.id;
        (doc, id)
    }

    fn only(doc: &PdfDocument) -> PageAnnotation {
        let mut found = doc.page_annotations(0).expect("annotations");
        assert_eq!(found.len(), 1);
        found.remove(0)
    }

    fn size(rect: Rect) -> (f32, f32) {
        (rect.x1 - rect.x0, rect.y1 - rect.y0)
    }

    fn near(have: f32, want: f32) -> bool {
        (have - want).abs() < 0.05
    }

    /// The color of the pixel (x, y) of page 0 as the saved document draws it.
    fn drawn(doc: &PdfDocument) -> impl Fn(usize, usize) -> Vec<u8> + use<> {
        let reopened = PdfDocument::from_bytes(&saved(doc)).expect("reopen");
        let page = reopened.render(0, 1.0, 0).expect("render");
        move |x, y| {
            let start = (y * page.width as usize + x) * 4;
            page.rgba[start..start + 3].to_vec()
        }
    }

    fn like(have: Vec<u8>, want: [u8; 3]) -> bool {
        have.iter()
            .zip(want)
            .all(|(have, want)| have.abs_diff(want) < 30)
    }

    #[test]
    fn a_picture_stamp_says_it_is_one_and_upright_until_it_is_turned() {
        let (doc, _) = stamped("benign/single-page.pdf");
        let stamp = only(&doc);
        let view = stamp.picture.expect("a picture");
        assert_eq!((view.angle, view.width, view.height), (0.0, 128.0, 64.0));
        // A standard stamp is no picture, and nothing else is.
        let mut other = single_page();
        let rect = Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 130.0,
            y1: 50.0,
        };
        other.add_stamp(0, rect, StampName::Draft).expect("stamp");
        assert_eq!(only(&other).picture, None);
    }

    #[test]
    fn a_picture_turned_a_quarter_draws_its_top_on_the_right_in_the_box_that_holds_it() {
        let (mut doc, id) = stamped("benign/single-page.pdf");
        doc.set_picture_angle(0, id, 90.0).expect("turned");
        let stamp = only(&doc);
        // About the same centre, a quarter turned: 64 wide, 128 tall.
        let rect = stamp.rect;
        assert!(near((rect.x0 + rect.x1) / 2.0, 164.0), "{rect:?}");
        assert!(near((rect.y0 + rect.y1) / 2.0, 132.0), "{rect:?}");
        let (width, height) = size(rect);
        assert!(near(width, 64.0) && near(height, 128.0), "{rect:?}");
        let view = stamp.picture.expect("still a picture");
        assert!(near(view.angle, 90.0), "{view:?}");
        assert!(
            near(view.width, 128.0) && near(view.height, 64.0),
            "{view:?}"
        );
        // The red top of the picture is on the right now, its blue bottom on the left.
        let at = drawn(&doc);
        assert!(like(at(188, 78), [200, 40, 40]), "{:?}", at(188, 78));
        assert!(like(at(140, 178), [40, 60, 200]), "{:?}", at(140, 178));
        // What the upright picture covered and the turned one does not is white again.
        assert_eq!(at(104, 104), [255, 255, 255]);
    }

    #[test]
    fn a_picture_turned_to_an_angle_is_held_by_the_box_that_fits_it_and_turns_back() {
        let (mut doc, id) = stamped("benign/single-page.pdf");
        doc.set_picture_angle(0, id, 45.0).expect("turned");
        let stamp = only(&doc);
        let (width, height) = size(stamp.rect);
        let held = (128.0 + 64.0) / std::f32::consts::SQRT_2;
        assert!(near(width, held) && near(height, held), "{:?}", stamp.rect);
        assert!(near(stamp.picture.expect("picture").angle, 45.0));
        // The middle of the picture is drawn, and the corners of its box are not.
        let at = drawn(&doc);
        assert!(!like(at(164, 132), [255, 255, 255]), "{:?}", at(164, 132));
        // (Not the top left one: the page has text there.)
        let (x0, y1) = (stamp.rect.x0 as usize + 2, stamp.rect.y1 as usize - 3);
        assert_eq!(at(x0, y1), [255, 255, 255]);
        let x1 = stamp.rect.x1 as usize - 3;
        assert_eq!(at(x1, y1), [255, 255, 255]);

        // Counter-clockwise is a negative angle, and the angle is of the picture, not a turn on
        // top of the last: turned back, it is where it was.
        doc.set_picture_angle(0, id, -30.0).expect("turned");
        assert!(near(only(&doc).picture.expect("picture").angle, -30.0));
        doc.set_picture_angle(0, id, 0.0).expect("turned back");
        let stamp = only(&doc);
        let (width, height) = size(stamp.rect);
        assert!(near(width, 128.0) && near(height, 64.0), "{:?}", stamp.rect);
        assert!(near(stamp.rect.x0, 100.0) && near(stamp.rect.y0, 100.0));
        let view = stamp.picture.expect("picture");
        assert_eq!((view.angle, view.width, view.height), (0.0, 128.0, 64.0));
    }

    #[test]
    fn a_turned_picture_is_in_the_saved_file_and_comes_back_turned() {
        let (mut doc, id) = stamped("benign/single-page.pdf");
        doc.set_picture_angle(0, id, 30.0).expect("turned");
        let reopened = PdfDocument::from_bytes(&saved(&doc)).expect("reopen");
        let stamp = only(&reopened);
        assert_eq!(stamp.kind, AnnotationKind::Stamp);
        let view = stamp.picture.expect("picture");
        assert!(near(view.angle, 30.0), "{view:?}");
        assert!(
            near(view.width, 128.0) && near(view.height, 64.0),
            "{view:?}"
        );
        // The same picture, drawn once: turning does not copy it.
        let bytes = saved(&doc);
        let images = bytes
            .windows(b"/Subtype/Image".len())
            .filter(|window| *window == b"/Subtype/Image")
            .count()
            + bytes
                .windows(b"/Subtype /Image".len())
                .filter(|window| *window == b"/Subtype /Image")
                .count();
        assert_eq!(images, 1, "{images} pictures in the file");
    }

    #[test]
    fn only_a_picture_of_the_users_own_can_be_turned_and_only_by_an_angle() {
        let mut doc = single_page();
        let rect = Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 130.0,
            y1: 50.0,
        };
        doc.add_stamp(0, rect, StampName::Draft).expect("stamp");
        doc.add_note(0, Point { x: 200.0, y: 200.0 }, "a note")
            .expect("note");
        for annotation in doc.page_annotations(0).expect("annotations") {
            assert!(matches!(
                doc.set_picture_angle(0, annotation.id, 30.0),
                Err(EngineError::InvalidEdit(_))
            ));
        }
        let (mut doc, id) = stamped("benign/single-page.pdf");
        for wrong in [f32::NAN, f32::INFINITY, 361.0, -400.0] {
            assert!(matches!(
                doc.set_picture_angle(0, id, wrong),
                Err(EngineError::InvalidEdit(_))
            ));
        }
        assert!(doc.set_picture_angle(5, id, 10.0).is_err());
        assert!(doc.set_picture_angle(0, AnnotationId(9999), 10.0).is_err());
        // Nothing of that changed the stamp.
        assert!(near(only(&doc).rect.x0, 100.0));
    }

    #[test]
    fn a_turned_picture_keeps_its_shape_when_its_box_is_moved_or_resized() {
        let (mut doc, id) = stamped("benign/single-page.pdf");
        doc.set_picture_angle(0, id, 30.0).expect("turned");
        let before = size(only(&doc).rect);
        // A box of another shape: the picture is the largest of its shape that fits, in the middle.
        let wanted = Rect {
            x0: 300.0,
            y0: 300.0,
            x1: 600.0,
            y1: 400.0,
        };
        doc.set_annotation_rect(0, id, wanted).expect("placed");
        let stamp = only(&doc);
        let after = size(stamp.rect);
        assert!(
            near(after.0 / after.1, before.0 / before.1),
            "{before:?} {after:?}"
        );
        assert!(after.0 <= 300.05 && after.1 <= 100.05, "{:?}", stamp.rect);
        assert!(near((stamp.rect.x0 + stamp.rect.x1) / 2.0, 450.0));
        assert!(near((stamp.rect.y0 + stamp.rect.y1) / 2.0, 350.0));
        // The picture is as large as the box allows now, and still turned.
        assert!(near(stamp.picture.expect("picture").angle, 30.0));

        // An upright stamp is as it always was: stretched to the box it is given.
        let (mut upright, id) = stamped("benign/single-page.pdf");
        upright.set_annotation_rect(0, id, wanted).expect("placed");
        assert_eq!(only(&upright).rect, wanted);
    }

    #[test]
    fn a_picture_turned_near_the_edge_stays_on_the_page_and_one_too_large_for_it_is_made_smaller() {
        let picture = prepare_stamp_picture(&corpus("images/stamp-exif.jpg")).expect("prepared");
        let mut doc = single_page();
        // In the corner: turned a quarter about its centre, half of it would be off the page.
        let corner = Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 128.0,
            y1: 64.0,
        };
        doc.add_image_stamp(0, corner, &picture.png).expect("stamp");
        let id = only(&doc).id;
        doc.set_picture_angle(0, id, 90.0).expect("turned");
        let rect = only(&doc).rect;
        assert!(rect.x0 >= -0.05 && rect.y0 >= -0.05, "{rect:?}");
        assert!(
            near(size(rect).0, 64.0) && near(size(rect).1, 128.0),
            "{rect:?}"
        );

        // 600 x 300 on a 612 x 792 page: turned 45 degrees it would be 636 across.
        let mut big = single_page();
        let wide = Rect {
            x0: 6.0,
            y0: 100.0,
            x1: 606.0,
            y1: 400.0,
        };
        big.add_image_stamp(0, wide, &picture.png).expect("stamp");
        let id = only(&big).id;
        big.set_picture_angle(0, id, 45.0).expect("turned");
        let stamp = only(&big);
        assert!(size(stamp.rect).0 <= 612.05 && size(stamp.rect).1 <= 792.05);
        assert!(
            stamp.rect.x0 >= -0.05 && stamp.rect.x1 <= 612.05,
            "{:?}",
            stamp.rect
        );
        // Smaller, in shape: the picture is no longer 600 x 300.
        let view = stamp.picture.expect("picture");
        assert!(near(view.width / view.height, 2.0), "{view:?}");
        assert!(view.width < 600.0, "{view:?}");
    }

    #[test]
    fn on_a_page_that_is_turned_the_picture_is_told_as_the_page_shows_it() {
        let (mut doc, id) = stamped("benign/rotated-page.pdf");
        let view = only(&doc).picture.expect("picture");
        // As it was put on the page: 128 across and 64 down, as seen.
        assert!(
            near(view.width, 128.0) && near(view.height, 64.0),
            "{view:?}"
        );
        let before = size(only(&doc).rect);
        doc.set_picture_angle(0, id, 90.0).expect("turned");
        let stamp = only(&doc);
        let after = size(stamp.rect);
        // Turned a quarter, what it measures across and down swaps, whatever the page's turn.
        assert!(
            near(after.0, before.1) && near(after.1, before.0),
            "{before:?} {after:?}"
        );
        let view = stamp.picture.expect("picture");
        assert!(near(view.angle, 90.0));
        assert!(
            near(view.width, 128.0) && near(view.height, 64.0),
            "{view:?}"
        );
    }

    #[test]
    fn the_setting_of_a_picture_is_read_from_its_form_and_its_rectangle() {
        // Upright: the form's unit square stretched over the rectangle.
        let upright = measure(
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [10.0, 20.0, 138.0, 84.0],
        )
        .expect("setting");
        assert_eq!(
            (upright.width, upright.height, upright.angle),
            (128.0, 64.0, 0.0)
        );
        // Turned a quarter, in a rectangle that is 64 across and 128 tall.
        let quarter = measure(
            [0.0, 0.0, 128.0, 64.0],
            [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 64.0, 128.0],
        )
        .expect("setting");
        assert!(
            near(quarter.width, 128.0) && near(quarter.height, 64.0),
            "{quarter:?}"
        );
        // A form that cannot be a picture.
        assert_eq!(
            measure(
                [0.0, 0.0, 0.0, 10.0],
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 5.0, 5.0]
            ),
            None
        );
        assert_eq!(
            measure(
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                [5.0, 5.0, 5.0, 9.0]
            ),
            None
        );
    }

    #[test]
    fn a_turn_keeps_the_picture_on_the_page() {
        let page = [0.0, 0.0, 200.0, 100.0];
        // Fits where it is.
        let turn_ = turn(100.0, 40.0, (100.0, 50.0), page, 0.0);
        assert_eq!(turn_.rect, [50.0, 30.0, 150.0, 70.0]);
        assert_eq!((turn_.width, turn_.height), (100.0, 40.0));
        // A quarter turn about a centre near the edge: moved in.
        let turn_ = turn(100.0, 40.0, (10.0, 10.0), page, 90.0);
        assert!(
            near(turn_.rect[0], 0.0) && near(turn_.rect[1], 0.0),
            "{turn_:?}"
        );
        assert!(
            near(turn_.rect[2], 40.0) && near(turn_.rect[3], 100.0),
            "{turn_:?}"
        );
        // Too tall for the page once turned: smaller, in shape.
        let turn_ = turn(100.0, 40.0, (100.0, 50.0), page, 90.0);
        assert!(near(turn_.rect[3] - turn_.rect[1], 100.0), "{turn_:?}");
        assert!(near(turn_.width / turn_.height, 2.5), "{turn_:?}");
    }

    #[test]
    fn a_turned_picture_stamp_comes_along_when_its_page_is_inserted() {
        let (mut source, id) = stamped("benign/multi-page-10.pdf");
        source.set_picture_angle(0, id, 30.0).expect("turn");
        let angle_of = |doc: &PdfDocument, page: u32| {
            let page = doc.pdf_page(page).expect("page");
            page.annotations()
                .find_map(|annotation| read_setting(&annotation).map(|(setting, _)| setting.angle))
        };
        let before = angle_of(&source, 0).expect("a picture stamp in the source");
        let rect_before = only(&source).rect;
        let prepared = PdfDocument::prepare_source(&saved(&source), None).expect("prepare");
        let mut doc = PdfDocument::from_bytes(&corpus("benign/multi-page-10.pdf")).expect("open");
        doc.insert_pages(2, &prepared.bytes, &[]).expect("insert");
        let found = doc.page_annotations(2).expect("annotations");
        assert_eq!(
            found.len(),
            1,
            "the picture stamp did not come along: {found:?}"
        );
        assert!(
            found[0].picture.is_some(),
            "not a picture stamp: {:?}",
            found[0]
        );
        let after = angle_of(&doc, 2).expect("a picture stamp on the inserted page");
        assert!(
            (after - before).abs() < 1e-3,
            "angle {before} became {after}"
        );
        let r = found[0].rect;
        let near = |a: f32, b: f32| (a - b).abs() < 1.0;
        assert!(
            near(r.x0, rect_before.x0)
                && near(r.y0, rect_before.y0)
                && near(r.x1, rect_before.x1)
                && near(r.y1, rect_before.y1),
            "{rect_before:?} became {r:?}"
        );
    }
}
