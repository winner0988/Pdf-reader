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

use crate::limits::{MAX_RASTER_PIXELS, MAX_RASTER_SIDE_PX, MIN_RENDER_SCALE};
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

/// The largest scale not above `scale` at which `page` fits the raster limits
/// (`MAX_RASTER_SIDE_PX`, `MAX_RASTER_PIXELS`). Pages too large to render at the requested
/// resolution are rendered at a lower one instead; the frontend stretches the result to the
/// page's display size. Fails only if even `MIN_RENDER_SCALE` does not fit.
pub fn fit_scale(page: PageSize, scale: f32) -> Result<f32, ValidationError> {
    page.validate()?;
    check_scale(scale)?;
    let (width, height) = (f64::from(page.width_pt), f64::from(page.height_pt));
    let by_side = f64::from(MAX_RASTER_SIDE_PX) / width.max(height);
    let by_area = (f64::from(MAX_RASTER_PIXELS) / (width * height)).sqrt();
    let mut fitted = f64::from(scale).min(by_side).min(by_area) as f32;
    // Pixel sizes are rounded up, so the analytic bound can still be a pixel too large.
    while raster_dimensions(page, fitted, Rotation::None).is_err() {
        fitted *= 0.999;
        if fitted < MIN_RENDER_SCALE {
            return Err(ValidationError::OutOfRange {
                what: "raster size",
            });
        }
    }
    Ok(fitted)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn fit_scale_keeps_scales_that_fit() {
        let letter = PageSize {
            width_pt: 612.0,
            height_pt: 792.0,
        };
        assert_eq!(fit_scale(letter, 1.5).unwrap(), 1.5);
        assert_eq!(
            fit_scale(letter, MIN_RENDER_SCALE).unwrap(),
            MIN_RENDER_SCALE
        );
    }

    #[test]
    fn fit_scale_lowers_the_resolution_of_large_renders() {
        let letter = PageSize {
            width_pt: 612.0,
            height_pt: 792.0,
        };
        // 8x would be 4896 x 6336 = 31 M pixels; the area limit is 16.7 M.
        let fitted = fit_scale(letter, 8.0).unwrap();
        assert!(fitted < 8.0 && fitted > 5.5, "{fitted}");
        for rotation in [Rotation::None, Rotation::Cw90] {
            let (width, height) = raster_dimensions(letter, fitted, rotation).unwrap();
            assert!(u64::from(width) * u64::from(height) <= u64::from(MAX_RASTER_PIXELS));
        }

        // A long strip is limited by its longest side instead.
        let strip = PageSize {
            width_pt: 20_000.0,
            height_pt: 100.0,
        };
        let fitted = fit_scale(strip, 2.0).unwrap();
        let (width, _) = raster_dimensions(strip, fitted, Rotation::None).unwrap();
        assert!(width <= MAX_RASTER_SIDE_PX && width > MAX_RASTER_SIDE_PX - 16);
    }

    #[test]
    fn fit_scale_rejects_pages_that_cannot_fit_at_all() {
        let enormous = PageSize {
            width_pt: 1_000_000.0,
            height_pt: 1_000_000.0,
        };
        assert!(fit_scale(enormous, 1.0).is_err());
        let letter = PageSize {
            width_pt: 612.0,
            height_pt: 792.0,
        };
        assert!(fit_scale(letter, f32::INFINITY).is_err());
    }
}
