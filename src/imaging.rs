//! Automatic page cleanup: skew detection, straightening, dark-border trimming and blank-page
//! detection. Nothing here edits the original file; results become page settings that can be
//! switched off or undone.
use image::{DynamicImage, GrayImage, Rgb, RgbImage};

/// Largest skew considered. Feeders rarely skew a sheet further, and a larger "best" angle on a
/// page usually means a diagram or photo rather than tilted text.
pub const MAX_SKEW: f32 = 15.0;
/// Longest side of the copy used for analysis. Text lines are still several pixels apart.
const ANALYSIS_SIZE: u32 = 1200;
/// Never trim more than this fraction from one edge.
const MAX_TRIM: f32 = 0.12;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Analysis {
    /// Clockwise skew of the content in degrees; 0 when no consistent text direction was found.
    pub skew: f32,
    /// Dark scanner-background border to trim before straightening: left, top, right, bottom.
    pub crop: [f32; 4],
    /// Almost no ink after trimming.
    pub blank: bool,
}

/// The scanner's background is square to the scan while the paper may be tilted, so dark edges
/// are trimmed first, then the skew is measured on what remains.
pub fn analyze(img: &DynamicImage) -> Analysis {
    let small = downscale(img);
    let crop = detect_border(&image::imageops::grayscale(&small));
    let trimmed = apply_crop(small.into(), crop).to_rgb8();
    let skew = detect_skew(&image::imageops::grayscale(&trimmed));
    let straight = if skew != 0.0 {
        rotate(&trimmed, -skew)
    } else {
        trimmed
    };
    Analysis {
        skew,
        crop,
        blank: is_blank(&image::imageops::grayscale(&straight)),
    }
}

fn downscale(img: &DynamicImage) -> RgbImage {
    if img.width().max(img.height()) > ANALYSIS_SIZE {
        img.resize(
            ANALYSIS_SIZE,
            ANALYSIS_SIZE,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8()
    } else {
        img.to_rgb8()
    }
}

/// Otsu's threshold: the grey level that best separates ink from paper.
fn otsu(g: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total = g.pixels().len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
    let (mut below, mut below_sum, mut best, mut best_t) = (0.0, 0.0, 0.0, 0u8);
    for (t, &n) in hist.iter().enumerate() {
        below += n as f64;
        if below == 0.0 || below == total {
            continue;
        }
        below_sum += t as f64 * n as f64;
        let above = total - below;
        let m0 = below_sum / below;
        let m1 = (sum - below_sum) / above;
        let between = below * above * (m0 - m1).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

/// Finds the angle at which rows of ink line up most sharply (projection-profile method).
/// Returns the clockwise skew of the content, or 0 if the page has no clear text direction.
pub fn detect_skew(g: &GrayImage) -> f32 {
    let (w, h) = g.dimensions();
    if w < 32 || h < 32 {
        return 0.0;
    }
    let threshold = otsu(g).min(170);
    // Skip a margin so scanner edges and punch holes don't count as text.
    let (mx, my) = (w / 25, h / 25);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut ink: Vec<(f32, f32)> = vec![];
    for y in my..h - my {
        for x in mx..w - mx {
            if g.get_pixel(x, y).0[0] < threshold {
                ink.push((x as f32 - cx, y as f32 - cy));
            }
        }
    }
    let area = ((w - 2 * mx) * (h - 2 * my)) as usize;
    // Blank pages, and pages that are mostly dark (photos), give no reliable reading.
    if ink.len() < 300 || ink.len() > area / 2 {
        return 0.0;
    }
    let step = (ink.len() / 150_000).max(1);
    let ink: Vec<_> = ink.into_iter().step_by(step).collect();
    let bins = (w + h) as usize + 2;
    let offset = bins as f32 / 2.0;
    let mut counts = vec![0u32; bins];
    let mut score = |deg: f32| -> f64 {
        let (s, c) = deg.to_radians().sin_cos();
        counts.iter_mut().for_each(|n| *n = 0);
        for &(x, y) in &ink {
            // Undo a clockwise rotation by `deg`: original rows become constant.
            let row = (-x * s + y * c + offset) as usize;
            counts[row.min(bins - 1)] += 1;
        }
        counts.iter().map(|&n| (n as f64).powi(2)).sum()
    };
    let coarse: Vec<(f32, f64)> = (-30..=30)
        .map(|i| i as f32 * MAX_SKEW / 30.0)
        .map(|a| (a, score(a)))
        .collect();
    let (mut best, mut best_score) = coarse
        .iter()
        .copied()
        .fold((0.0, f64::MIN), |b, c| if c.1 > b.1 { c } else { b });
    let mean = coarse.iter().map(|c| c.1).sum::<f64>() / coarse.len() as f64;
    // Text gives one sharp peak; drawings and noise give a flat response.
    if best_score < mean * 1.25 {
        return 0.0;
    }
    for i in -10..=10 {
        let a = best + i as f32 * 0.05;
        let s = score(a);
        if s > best_score {
            (best, best_score) = (a, s);
        }
    }
    let best = (best * 20.0).round() / 20.0;
    if best.abs() < 0.15 || best.abs() > MAX_SKEW {
        0.0
    } else {
        best
    }
}

/// Rotates content clockwise by `deg` about the centre, keeping the page size; uncovered
/// corners become white, like paper.
pub fn rotate(img: &RgbImage, deg: f32) -> RgbImage {
    let (w, h) = img.dimensions();
    let mut out = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
    if w == 0 || h == 0 {
        return out;
    }
    let (s, c) = deg.to_radians().sin_cos();
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let src = img.as_raw();
    let stride = w as usize * 3;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per = (h as usize).div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(rows_per * stride).enumerate() {
            scope.spawn(move || {
                for (r, row) in chunk.chunks_mut(stride).enumerate() {
                    let dy = (chunk_index * rows_per + r) as f32 - cy;
                    for x in 0..w as usize {
                        let dx = x as f32 - cx;
                        let sx = dx * c + dy * s + cx;
                        let sy = -dx * s + dy * c + cy;
                        if sx < 0.0 || sy < 0.0 || sx > w as f32 - 1.0 || sy > h as f32 - 1.0 {
                            continue;
                        }
                        let (x0, y0) = (sx as usize, sy as usize);
                        let (x1, y1) = ((x0 + 1).min(w as usize - 1), (y0 + 1).min(h as usize - 1));
                        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                        for k in 0..3 {
                            let p = |x: usize, y: usize| src[y * stride + x * 3 + k] as f32;
                            let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
                            let bottom = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
                            row[x * 3 + k] = (top * (1.0 - fy) + bottom * fy + 0.5) as u8;
                        }
                    }
                }
            });
        }
    });
    out
}

/// Trims each edge by a fraction of the image (left, top, right, bottom). At least 10% of
/// each dimension is always kept.
pub fn apply_crop(img: DynamicImage, crop: [f32; 4]) -> DynamicImage {
    if crop.iter().all(|&c| c <= 0.0) {
        return img;
    }
    let (w, h) = (img.width() as f32, img.height() as f32);
    let c = crop.map(|c| c.clamp(0.0, 0.45));
    let x0 = (w * c[0]).round();
    let y0 = (h * c[1]).round();
    let x1 = (w * (1.0 - c[2])).round().max(x0 + (w * 0.1).max(1.0));
    let y1 = (h * (1.0 - c[3])).round().max(y0 + (h * 0.1).max(1.0));
    img.crop_imm(
        x0 as u32,
        y0 as u32,
        (x1 - x0).min(w - x0) as u32,
        (y1 - y0).min(h - y0) as u32,
    )
}

/// Dark bands along the edges, where the scanner saw its background instead of paper.
pub fn detect_border(g: &GrayImage) -> [f32; 4] {
    let (w, h) = g.dimensions();
    if w < 16 || h < 16 {
        return [0.0; 4];
    }
    let dark = |x: u32, y: u32| g.get_pixel(x, y).0[0] < 80;
    let col_dark = |x: u32| (0..h).filter(|&y| dark(x, y)).count() as f32 / h as f32 > 0.5;
    let row_dark = |y: u32| (0..w).filter(|&x| dark(x, y)).count() as f32 / w as f32 > 0.5;
    let measure = |len: u32, is_dark: &dyn Fn(u32) -> bool, from_end: bool| -> f32 {
        let limit = (len as f32 * MAX_TRIM) as u32;
        let n = (0..limit)
            .take_while(|&i| is_dark(if from_end { len - 1 - i } else { i }))
            .count();
        if n == 0 {
            0.0
        } else {
            // A little extra removes the soft shadow where background meets paper.
            ((n as f32 + len as f32 * 0.004) / len as f32).min(MAX_TRIM)
        }
    };
    [
        measure(w, &col_dark, false),
        measure(h, &row_dark, false),
        measure(w, &col_dark, true),
        measure(h, &row_dark, true),
    ]
}

/// Less than 0.2% of the page, ignoring a thin margin, is clearly darker than paper.
/// Light show-through from the other side of the sheet does not count.
pub fn is_blank(g: &GrayImage) -> bool {
    let (w, h) = g.dimensions();
    let (mx, my) = (w / 50, h / 50);
    let mut ink = 0usize;
    let mut total = 0usize;
    for y in my..h.saturating_sub(my) {
        for x in mx..w.saturating_sub(mx) {
            total += 1;
            if g.get_pixel(x, y).0[0] < 160 {
                ink += 1;
            }
        }
    }
    total > 0 && (ink as f64) < total as f64 * 0.002
}

#[cfg(test)]
pub mod tests {
    use super::*;
    /// A synthetic worksheet: rows of word-like dark blocks on white paper.
    pub fn worksheet(w: u32, h: u32) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([250, 250, 246]));
        let line_h = h / 40;
        for line in 4..36 {
            let y = line * line_h;
            let mut x = w / 10 + (line * 37 % 50);
            while x < w * 9 / 10 {
                let word = 20 + (x * 7 + line * 13) % 70;
                for yy in y..y + line_h / 2 {
                    for xx in x..(x + word).min(w * 9 / 10) {
                        img.put_pixel(xx, yy, Rgb([30, 30, 30]));
                    }
                }
                x += word + 18;
            }
        }
        img
    }
    #[test]
    fn measures_skew_in_both_directions() {
        let page = worksheet(1000, 1300);
        for angle in [-7.0f32, -2.5, -0.8, 1.2, 4.0, 9.5] {
            let tilted = rotate(&page, angle);
            let found = detect_skew(&DynamicImage::from(tilted).to_luma8());
            assert!((found - angle).abs() < 0.2, "rotated {angle}, found {found}");
        }
    }
    #[test]
    fn straightening_removes_the_skew() {
        let tilted = rotate(&worksheet(1000, 1300), 3.0);
        let a = analyze(&tilted.clone().into());
        let fixed = rotate(&tilted, -a.skew);
        assert_eq!(detect_skew(&DynamicImage::from(fixed).to_luma8()), 0.0);
        assert!(!a.blank);
    }
    #[test]
    fn level_and_empty_pages_are_left_alone() {
        let level = worksheet(1000, 1300);
        assert_eq!(detect_skew(&DynamicImage::from(level).to_luma8()), 0.0);
        let paper = GrayImage::from_pixel(800, 1000, image::Luma([245]));
        assert_eq!(detect_skew(&paper), 0.0);
        assert!(is_blank(&paper));
    }
    #[test]
    fn noise_has_no_text_direction() {
        let mut img = GrayImage::from_pixel(800, 1000, image::Luma([255]));
        let mut seed = 12345u32;
        for p in img.pixels_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            if (seed >> 16) % 9 == 0 {
                p.0[0] = 0;
            }
        }
        assert_eq!(detect_skew(&img), 0.0);
    }
    #[test]
    fn show_through_counts_as_blank_but_a_short_note_does_not() {
        let mut back = GrayImage::from_pixel(800, 1000, image::Luma([248]));
        for y in 300..320 {
            for x in 100..700 {
                back.put_pixel(x, y, image::Luma([205])); // faint text from the front side
            }
        }
        assert!(is_blank(&back));
        for y in 500..520 {
            for x in 100..200 {
                back.put_pixel(x, y, image::Luma([40]));
            }
        }
        assert!(!is_blank(&back));
    }
    #[test]
    fn trims_dark_scanner_background() {
        let mut img = GrayImage::from_pixel(1000, 1000, image::Luma([250]));
        for y in 0..1000 {
            for x in 0..1000 {
                if x < 40 || y > 960 {
                    img.put_pixel(x, y, image::Luma([20]));
                }
            }
        }
        let c = detect_border(&img);
        assert!(c[0] > 0.04 && c[0] < 0.05, "{c:?}");
        assert!(c[3] > 0.039 && c[3] < 0.05, "{c:?}");
        assert_eq!((c[1], c[2]), (0.0, 0.0));
        let cropped = apply_crop(img.into(), c);
        assert!(cropped.width() < 960 && cropped.height() < 961);
    }
}
