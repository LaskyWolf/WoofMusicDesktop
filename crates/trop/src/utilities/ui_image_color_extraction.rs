// Trop/Utilities/UIImage+ColorExtraction.swift: the player background colours
// and the cover's black-border crop.
//
// Swift draws the `UIImage` into a `CGContext`; here the caller decodes the
// image and passes 8-bit RGBA pixels. `extract_dominant_colors` expects the
// cover already scaled to 8×8, as Swift draws it.

/// A colour as red, green, blue in 0...1.
pub type Rgb = [f64; 3];

/// Swift's fallback pair: a dark blue-grey and almost black.
pub const FALLBACK_COLORS: [Rgb; 2] = [[0.15, 0.15, 0.2], [0.05, 0.05, 0.08]];

/// Swift `Color.darkened(toLuminance:)`: this colour, darkened only as far as
/// `max_luminance` needs. White text reaches 4.5:1 on 0.18; this colour
/// reaches 3:1 on white at 0.3.
pub fn darkened(color: Rgb, max_luminance: f64) -> Rgb {
    fn linear(value: f64) -> f64 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    let [r, g, b] = color;
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    if luminance <= max_luminance {
        return color;
    }
    // Mixing with black scales each channel, and luminance follows about the 2.2 power of that.
    let keep = (max_luminance / luminance).powf(1.0 / 2.2);
    [r * keep, g * keep, b * keep]
}

/// Swift `UIImage.extractDominantColors()`: the most vibrant colour and a
/// second one far enough from it. `rgba` is the cover scaled to 8×8.
pub fn extract_dominant_colors(rgba: &[u8]) -> [Rgb; 2] {
    struct Pixel {
        r: f64,
        g: f64,
        b: f64,
    }
    impl Pixel {
        fn luminance(&self) -> f64 {
            0.299 * self.r + 0.587 * self.g + 0.114 * self.b
        }
        fn saturation(&self) -> f64 {
            let max = self.r.max(self.g).max(self.b);
            let min = self.r.min(self.g).min(self.b);
            if max == 0.0 { 0.0 } else { (max - min) / max }
        }
        fn vibrancy(&self) -> f64 {
            self.saturation() * 0.8 + (1.0 - (self.luminance() - 0.5).abs()) * 0.2
        }
        fn rgb(&self) -> Rgb {
            [self.r, self.g, self.b]
        }
    }

    let mut pixels: Vec<Pixel> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| f64::from(p[3]) / 255.0 > 0.8)
        .map(|p| Pixel {
            r: f64::from(p[0]) / 255.0,
            g: f64::from(p[1]) / 255.0,
            b: f64::from(p[2]) / 255.0,
        })
        .collect();
    if pixels.is_empty() {
        return FALLBACK_COLORS;
    }
    // Stable sort, like Swift's `sorted`: ties keep their order.
    pixels.sort_by(|a, b| b.vibrancy().total_cmp(&a.vibrancy()));

    let primary = &pixels[0];
    let secondary = pixels
        .iter()
        .find(|p| (p.r - primary.r).abs() + (p.g - primary.g).abs() + (p.b - primary.b).abs() > 0.4)
        .map(Pixel::rgb)
        .unwrap_or_else(|| {
            [
                (primary.r - 0.2).max(0.0),
                (primary.g - 0.2).max(0.0),
                (primary.b - 0.2).max(0.0),
            ]
        });
    [primary.rgb(), secondary]
}

/// Swift `UIImage.centerCroppedSquare()`: the largest centred square inside
/// the image once black borders (letterboxing) are trimmed, as
/// `(x, y, side, side)` in pixels.
///
/// Swift scans a copy scaled to at most 160 px; this scans every pixel.
pub fn center_cropped_square(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, u32, u32) {
    let (x, y, w, h) = trimmed_content_rect(width, height, rgba);
    let side = w.min(h);
    (x + (w - side) / 2, y + (h - side) / 2, side, side)
}

/// The bounding box of the non-black content: rows and columns that are at
/// least 90% black count as border.
fn trimmed_content_rect(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, u32, u32) {
    let full = (0, 0, width, height);
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return full;
    }
    let is_black = |x: usize, y: usize| {
        let o = (y * w + x) * 4;
        let sum = u32::from(rgba[o]) + u32::from(rgba[o + 1]) + u32::from(rgba[o + 2]);
        f64::from(sum) / 255.0 / 3.0 < 0.08
    };
    const BLACK_RATIO: f64 = 0.9;
    let column_is_border =
        |x: usize| (0..h).filter(|&y| is_black(x, y)).count() as f64 / h as f64 >= BLACK_RATIO;
    let row_is_border =
        |y: usize| (0..w).filter(|&x| is_black(x, y)).count() as f64 / w as f64 >= BLACK_RATIO;

    let mut min_x = 0;
    while min_x < w && column_is_border(min_x) {
        min_x += 1;
    }
    let mut max_x = w - 1;
    while max_x > min_x && column_is_border(max_x) {
        max_x -= 1;
    }
    let mut min_y = 0;
    while min_y < h && row_is_border(min_y) {
        min_y += 1;
    }
    let mut max_y = h - 1;
    while max_y > min_y && row_is_border(max_y) {
        max_y -= 1;
    }
    if max_x <= min_x || max_y <= min_y {
        return full;
    }
    (
        min_x as u32,
        min_y as u32,
        (max_x - min_x + 1) as u32,
        (max_y - min_y + 1) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, pixel: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .flat_map(|(x, y)| pixel(x, y))
            .collect()
    }

    #[test]
    fn transparent_image_falls_back() {
        assert_eq!(extract_dominant_colors(&[0; 8 * 8 * 4]), FALLBACK_COLORS);
    }

    #[test]
    fn picks_the_vibrant_colour_and_a_distant_second() {
        // Half grey, half pure red: red wins on saturation, grey is far enough.
        let rgba = image(8, 8, |x, _| {
            if x < 4 {
                [128, 128, 128, 255]
            } else {
                [255, 0, 0, 255]
            }
        });
        let [primary, secondary] = extract_dominant_colors(&rgba);
        assert_eq!(primary, [1.0, 0.0, 0.0]);
        assert!((secondary[0] - 128.0 / 255.0).abs() < 1e-9);
    }

    #[test]
    fn one_colour_gets_a_darker_second() {
        let rgba = image(8, 8, |_, _| [255, 0, 0, 255]);
        assert_eq!(
            extract_dominant_colors(&rgba),
            [[1.0, 0.0, 0.0], [0.8, 0.0, 0.0]]
        );
    }

    #[test]
    fn darkened_leaves_dark_colours_alone() {
        assert_eq!(darkened([0.1, 0.1, 0.1], 0.18), [0.1, 0.1, 0.1]);
        let white = darkened([1.0, 1.0, 1.0], 0.18);
        // 0.18^(1/2.2) ≈ 0.459
        assert!((white[0] - 0.18f64.powf(1.0 / 2.2)).abs() < 1e-9);
    }

    #[test]
    fn crops_letterbox_bars() {
        // A 16:9 frame with a 9-pixel-tall picture between black bars.
        let rgba = image(16, 15, |_, y| {
            if (3..12).contains(&y) {
                [200, 50, 50, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        assert_eq!(center_cropped_square(16, 15, &rgba), (3, 3, 9, 9));
    }

    #[test]
    fn all_black_keeps_the_whole_square() {
        let rgba = image(10, 10, |_, _| [0, 0, 0, 255]);
        assert_eq!(center_cropped_square(10, 10, &rgba), (0, 0, 10, 10));
    }
}
