//! Estimating skew angle of scanned pages using projection profiles (B6-11),
//! and isolated noise removal.

use crate::ocr_worker::Picture;

/// Downscale picture so that its longest side is at most this many pixels for deskew estimation.
const MAX_DOWNSCALE_SIDE: u32 = 1000;

/// Number of isolated dark dots per million pixels required to trigger denoising.
pub const NOISE_RATE_PER_MILLION_THRESHOLD: f64 = 100.0;

/// Estimates the skew angle of `picture` in degrees.
///
/// Positive angle means the text is tilted clockwise (sloping downwards to the right in y-down space).
/// Returns 0.0 if the page has almost no black pixels (e.g. blank page).
pub fn estimate_skew(picture: &Picture) -> f64 {
    let (width, height) = (picture.width, picture.height);
    if width < 10 || height < 10 || picture.grey.len() < (width as usize * height as usize) {
        return 0.0;
    }

    let max_side = width.max(height);
    let scale = if max_side > MAX_DOWNSCALE_SIDE {
        MAX_DOWNSCALE_SIDE as f64 / max_side as f64
    } else {
        1.0
    };

    let ds_w = ((width as f64) * scale).round() as usize;
    let ds_h = ((height as f64) * scale).round() as usize;
    if ds_w < 5 || ds_h < 5 {
        return 0.0;
    }

    // Subsample into downsampled grayscale buffer
    let mut downsampled = Vec::with_capacity(ds_w * ds_h);
    for y in 0..ds_h {
        let orig_y = ((y as f64) / scale).floor() as usize;
        let row_start = orig_y * width as usize;
        for x in 0..ds_w {
            let orig_x = ((x as f64) / scale).floor() as usize;
            downsampled.push(picture.grey[row_start + orig_x]);
        }
    }

    // Otsu's thresholding to find black & white threshold
    let mut hist = [0u32; 256];
    for &p in &downsampled {
        hist[p as usize] += 1;
    }

    let total_pixels = (ds_w * ds_h) as u32;
    let mut sum_all = 0.0f64;
    for (i, &count) in hist.iter().enumerate() {
        sum_all += (i as f64) * (count as f64);
    }

    let mut sum_b = 0.0f64;
    let mut w_b = 0u32;
    let mut max_var = 0.0f64;
    let mut threshold = 128u8;

    for (t, &count) in hist.iter().enumerate() {
        w_b += count;
        if w_b == 0 {
            continue;
        }
        let w_f = total_pixels - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += (t as f64) * (count as f64);
        let m_b = sum_b / w_b as f64;
        let m_f = (sum_all - sum_b) / w_f as f64;
        let var_between = (w_b as f64) * (w_f as f64) * (m_b - m_f) * (m_b - m_f);
        if var_between > max_var {
            max_var = var_between;
            threshold = t as u8;
        }
    }

    // Collect black pixel coordinates relative to center
    let cx = ds_w as f64 / 2.0;
    let cy = ds_h as f64 / 2.0;
    let mut black_points = Vec::new();

    for y in 0..ds_h {
        let row_start = y * ds_w;
        for x in 0..ds_w {
            if downsampled[row_start + x] <= threshold {
                black_points.push(((x as f64) - cx, (y as f64) - cy));
            }
        }
    }

    // If almost no black pixels (e.g. blank page) or solid black, return 0.0
    let black_count = black_points.len();
    if black_count < 50 || black_count > (ds_w * ds_h * 9) / 10 {
        return 0.0;
    }

    // Radial distance from center to bounds to determine bin count
    let r = cx.hypot(cy);
    let num_bins = (2.0 * r).ceil() as usize + 4;
    let bin_offset = r + 1.0;

    let score_angle = |deg: f64| -> u64 {
        let rad = deg.to_radians();
        let sin_t = rad.sin();
        let cos_t = rad.cos();
        let mut bins = vec![0u32; num_bins];
        for &(dx, dy) in &black_points {
            let p = -dx * sin_t + dy * cos_t + bin_offset;
            let bin = p.round() as usize;
            if bin < num_bins {
                bins[bin] += 1;
            }
        }
        bins.iter().map(|&cnt| (cnt as u64) * (cnt as u64)).sum()
    };

    // Coarse pass: -15.0 to +15.0 with step 0.5 degrees
    let mut best_angle = 0.0f64;
    let mut best_score = 0u64;

    let mut i = -30;
    while i <= 30 {
        let angle = (i as f64) * 0.5;
        let score = score_angle(angle);
        if score > best_score {
            best_score = score;
            best_angle = angle;
        }
        i += 1;
    }

    // Fine pass: around best_angle +/- 0.45 with step 0.05 degrees
    let mut fine_angle = best_angle;
    let mut fine_score = best_score;

    let mut j = -9;
    while j <= 9 {
        let angle = best_angle + (j as f64) * 0.05;
        if (-15.0..=15.0).contains(&angle) {
            let score = score_angle(angle);
            if score > fine_score {
                fine_score = score;
                fine_angle = angle;
            }
        }
        j += 1;
    }

    fine_angle
}

/// Detects and removes isolated black pixels (where all 8 neighbors in a 3x3 window are white).
/// Only removes noise if the rate of isolated dots exceeds [`NOISE_RATE_PER_MILLION_THRESHOLD`].
/// Returns the number of removed noise pixels.
pub fn denoise_isolated_noise(picture: &mut Picture) -> usize {
    let (width, height) = (picture.width as usize, picture.height as usize);
    if width < 3 || height < 3 || picture.grey.len() < width * height {
        return 0;
    }

    let threshold_dark = 128u8;
    let threshold_white = 200u8;

    let mut isolated = Vec::new();
    for y in 1..(height - 1) {
        let row = y * width;
        let prev_row = (y - 1) * width;
        let next_row = (y + 1) * width;

        for x in 1..(width - 1) {
            let p = picture.grey[row + x];
            if p < threshold_dark {
                // Check if all 8 neighbors are white
                let neighbors_white = picture.grey[prev_row + x - 1] >= threshold_white
                    && picture.grey[prev_row + x] >= threshold_white
                    && picture.grey[prev_row + x + 1] >= threshold_white
                    && picture.grey[row + x - 1] >= threshold_white
                    && picture.grey[row + x + 1] >= threshold_white
                    && picture.grey[next_row + x - 1] >= threshold_white
                    && picture.grey[next_row + x] >= threshold_white
                    && picture.grey[next_row + x + 1] >= threshold_white;
                if neighbors_white {
                    isolated.push(row + x);
                }
            }
        }
    }

    let total = width * height;
    let rate = (isolated.len() as f64 / total as f64) * 1_000_000.0;
    if rate >= NOISE_RATE_PER_MILLION_THRESHOLD {
        let count = isolated.len();
        for idx in isolated {
            picture.grey[idx] = 255;
        }
        count
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_page_estimates_to_zero_degrees() {
        let picture = Picture {
            grey: vec![255; 1000 * 1000],
            width: 1000,
            height: 1000,
            ppi: 200,
        };
        assert_eq!(estimate_skew(&picture), 0.0);
    }

    #[test]
    fn clean_page_does_not_trigger_denoise() {
        let mut picture = Picture {
            grey: vec![255; 1000 * 1000],
            width: 1000,
            height: 1000,
            ppi: 200,
        };
        assert_eq!(denoise_isolated_noise(&mut picture), 0);
    }

    #[test]
    fn noisy_page_triggers_denoise_and_cleans_noise() {
        let width = 500;
        let height = 500;
        let mut grey = vec![255u8; width * height];
        // Add 100 isolated black dots (100 / 0.25M = 400 dots per million, > 100 threshold)
        for i in 1..101 {
            let y = (i * 4) % (height - 2) + 1;
            let x = (i * 3) % (width - 2) + 1;
            grey[y * width + x] = 0;
        }
        let mut picture = Picture {
            grey,
            width: width as u32,
            height: height as u32,
            ppi: 200,
        };
        let cleaned = denoise_isolated_noise(&mut picture);
        assert!(cleaned >= 80, "cleaned: {cleaned}");
        // Verify those pixels became 255
        assert!(picture.grey.iter().all(|&p| p == 255));
    }

    #[test]
    fn skewed_lines_are_detected_accurately() {
        let width = 1200;
        let height = 1500;
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;

        for &target in &[2.0f64, 5.0, 10.0, -5.0] {
            let rad = target.to_radians();
            let cos_t = rad.cos();
            let sin_t = rad.sin();

            let mut grey = vec![255u8; width * height];
            // Draw 5 horizontal lines rotated by target
            for &y0 in &[300.0, 450.0, 600.0, 750.0, 900.0] {
                for x_idx in 200..1000 {
                    let x0 = x_idx as f64;
                    for dy in 0..6 {
                        let y_pt = y0 + dy as f64;
                        let rx = (cx + (x0 - cx) * cos_t - (y_pt - cy) * sin_t).round() as isize;
                        let ry = (cy + (x0 - cx) * sin_t + (y_pt - cy) * cos_t).round() as isize;
                        if rx >= 0 && rx < width as isize && ry >= 0 && ry < height as isize {
                            grey[ry as usize * width + rx as usize] = 0;
                        }
                    }
                }
            }

            let picture = Picture {
                grey,
                width: width as u32,
                height: height as u32,
                ppi: 200,
            };

            let start = std::time::Instant::now();
            let detected = estimate_skew(&picture);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 200,
                "estimation took too long: {elapsed:?}"
            );
            assert!(
                (detected - target).abs() <= 0.15,
                "target: {target}, detected: {detected}"
            );
        }
    }
}
