//! Binary layout of a rendered page handed to the frontend as the raw response of the
//! `render_page` command (an `ArrayBuffer` in JavaScript, no JSON or base64).
//!
//! ```text
//! offset  size  field
//!      0     4  magic "PDFR"
//!      4     2  format (u16 LE): 1 = RGBA8, opaque
//!      6     2  reserved (u16 LE): 0
//!      8     4  width in pixels (u32 LE)
//!     12     4  height in pixels (u32 LE)
//!     16     *  pixels: width * height * 4 bytes, rows top to bottom, no padding
//! ```
//!
//! The frontend decoder lives in `src/ipc/raster.ts` and reads these constants from the
//! generated bindings.

use crate::types::{PageSize, Rotation};
use crate::validate::{Validate, ValidationError, check_raster_size, check_scale};
use crate::worker::Raster;

pub const RASTER_MAGIC: [u8; 4] = *b"PDFR";
pub const RASTER_HEADER_BYTES: usize = 16;
pub const RASTER_FORMAT_RGBA8: u16 = 1;

/// Serializes a validated raster into the frontend layout.
pub fn encode_raster(raster: &Raster) -> Result<Vec<u8>, ValidationError> {
    raster.validate()?;
    let mut out = Vec::with_capacity(RASTER_HEADER_BYTES + raster.pixels.len());
    out.extend_from_slice(&RASTER_MAGIC);
    out.extend_from_slice(&RASTER_FORMAT_RGBA8.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&raster.width.to_le_bytes());
    out.extend_from_slice(&raster.height.to_le_bytes());
    out.extend_from_slice(&raster.pixels);
    Ok(out)
}

/// Pixel size of `page` rendered at `scale` and `rotation`, or an error when it would exceed
/// the raster limits. Both processes use this so they agree on the output size.
pub fn raster_dimensions(
    page: PageSize,
    scale: f32,
    rotation: Rotation,
) -> Result<(u32, u32), ValidationError> {
    page.validate()?;
    check_scale(scale)?;
    let to_px = |points: f32| (f64::from(points) * f64::from(scale)).ceil().max(1.0);
    let (width, height) = (to_px(page.width_pt), to_px(page.height_pt));
    let (width, height) = if rotation.is_quarter_turn() {
        (height, width)
    } else {
        (width, height)
    };
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(ValidationError::OutOfRange {
            what: "raster size",
        });
    }
    let (width, height) = (width as u32, height as u32);
    check_raster_size(width, height)?;
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::MAX_RASTER_SIDE_PX;

    /// Shared test vector: `src/ipc/raster.test.ts` decodes exactly these bytes.
    const TEST_VECTOR: [u8; 24] = [
        b'P', b'D', b'F', b'R', 1, 0, 0, 0, // magic, format, reserved
        2, 0, 0, 0, 1, 0, 0, 0, // width 2, height 1
        255, 0, 0, 255, 0, 0, 255, 255, // red, blue
    ];

    #[test]
    fn encodes_the_shared_test_vector() {
        let raster = Raster {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        assert_eq!(encode_raster(&raster).unwrap(), TEST_VECTOR);
    }

    #[test]
    fn refuses_to_encode_an_inconsistent_raster() {
        let raster = Raster {
            width: 2,
            height: 2,
            pixels: vec![0; 8],
        };
        assert!(encode_raster(&raster).is_err());
    }

    #[test]
    fn letter_page_at_150_percent() {
        let letter = PageSize {
            width_pt: 612.0,
            height_pt: 792.0,
        };
        assert_eq!(
            raster_dimensions(letter, 1.5, Rotation::None).unwrap(),
            (918, 1188)
        );
        assert_eq!(
            raster_dimensions(letter, 1.5, Rotation::Cw90).unwrap(),
            (1188, 918)
        );
    }

    #[test]
    fn tiny_pages_render_at_least_one_pixel() {
        let tiny = PageSize {
            width_pt: 0.1,
            height_pt: 0.1,
        };
        assert_eq!(
            raster_dimensions(tiny, 0.01, Rotation::None).unwrap(),
            (1, 1)
        );
    }

    #[test]
    fn oversized_renders_are_rejected() {
        let huge = PageSize {
            width_pt: MAX_RASTER_SIDE_PX as f32,
            height_pt: 10.0,
        };
        assert!(raster_dimensions(huge, 2.0, Rotation::None).is_err());

        let letter = PageSize {
            width_pt: 612.0,
            height_pt: 792.0,
        };
        // 612 * 8 x 792 * 8 = 4896 x 6336 > 4096 x 4096 pixels.
        assert!(raster_dimensions(letter, 8.0, Rotation::None).is_err());
        assert!(raster_dimensions(letter, f32::NAN, Rotation::None).is_err());
    }
}
