//! Image parity metric with an outlier budget (Chiaro port, package P3).
//!
//! The Rust renderer is compared against golden images from the C++ Chiaro demos
//! (`tests/golden/chiaro`). GPU drivers differ in the last bits and a few pixels (edges of
//! shadows, hot highlights) differ a lot more, so neither a hash nor one maximum fits.
//! A [`ParityBudget`] has three limits:
//!
//! - `tolerance`: a pixel is an *outlier* when its largest per-channel difference exceeds it;
//! - `max_outlier_ppm`: how many outliers are allowed, in parts per million of the compared
//!   pixels (integer maths, so the verdict is the same on every platform);
//! - `hard_limit`: no pixel may differ by more than this, budget or not.
//!
//! Inputs are tightly packed RGBA8 buffers; alpha is ignored. Rectangles can be excluded
//! (HUD text of the demos, which shows a frame time).

use std::path::{Path, PathBuf};

/// An axis-aligned pixel rectangle `x, y, w, h` excluded from a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// True when the pixel lies inside the rectangle.
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px - self.x < self.w && py >= self.y && py - self.y < self.h
    }
}

/// Limits of one comparison. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParityBudget {
    /// Per-channel difference up to which a pixel counts as equal.
    pub tolerance: u8,
    /// Allowed outliers in parts per million of the compared pixels.
    pub max_outlier_ppm: u32,
    /// Largest per-channel difference any pixel may have.
    pub hard_limit: u8,
}

impl ParityBudget {
    /// Frozen-clock scenes. Derived from the measured D3D12-versus-Vulkan difference on the
    /// RX 6700 XT (worst case over 6 x 6 runs: 0.0009 % of pixels above 2, maximum 86):
    /// tolerance 2, 100 ppm (0.01 %), hard limit 128. Same-backend runs are bit-identical there.
    pub const STRICT: Self = Self {
        tolerance: 2,
        max_outlier_ppm: 100,
        hard_limit: 128,
    };

    /// Scenes that depend on wall-clock time or a scripted mouse (`deterministic = false` in
    /// the manifest). Measured cross-backend worst case: 0.39 % of pixels above 32; a moving
    /// spot edge reaches 243, so there is no hard limit below 255.
    pub const LOOSE: Self = Self {
        tolerance: 32,
        max_outlier_ppm: 10_000,
        hard_limit: 255,
    };
}

/// Result of [`compare_rgba8`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParityReport {
    /// Pixels compared (image size minus excluded pixels).
    pub compared: u64,
    /// Pixels with any channel different.
    pub differing: u64,
    /// Pixels above the budget's tolerance.
    pub outliers: u64,
    /// Largest per-channel difference of any compared pixel.
    pub max_diff: u8,
    /// Sum of the per-pixel maximum channel differences (mean = `sum_diff / compared`).
    pub sum_diff: u64,
}

/// Why a comparison could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParityError {
    /// A buffer is not `width * height * 4` bytes.
    SizeMismatch,
}

impl ParityReport {
    /// True when the report stays within `budget`.
    pub fn within(&self, budget: &ParityBudget) -> bool {
        self.max_diff <= budget.hard_limit
            && u128::from(self.outliers) * 1_000_000
                <= u128::from(budget.max_outlier_ppm) * u128::from(self.compared)
    }

    /// Outliers in parts per million of the compared pixels (rounded down).
    pub fn outlier_ppm(&self) -> u64 {
        if self.compared == 0 {
            0
        } else {
            (u128::from(self.outliers) * 1_000_000 / u128::from(self.compared)) as u64
        }
    }

    /// One line on stderr: how close a run came, so a passing run documents its margin.
    pub fn print(&self, name: &str, budget_name: &str) {
        eprintln!(
            "{name} vs C++ [{budget_name}]: max diff {}, differing {} / {} px ({:.4} %), outliers {} ({} ppm), mean {:.4}",
            self.max_diff,
            self.differing,
            self.compared,
            self.differing as f64 * 100.0 / self.compared as f64,
            self.outliers,
            self.outlier_ppm(),
            self.sum_diff as f64 / self.compared as f64
        );
    }
}

/// Reads a golden PNG as tightly packed RGBA8 (RGB gets alpha 255). Panics with the path if the
/// file is missing, has another size than `width` x `height`, or another colour type.
pub fn load_png_rgba8(path: &Path, width: u32, height: u32) -> Vec<u8> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut reader = png::Decoder::new(file).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(
        (info.width, info.height),
        (width, height),
        "{}",
        path.display()
    );
    let px = &buf[..info.buffer_size()];
    match info.color_type {
        png::ColorType::Rgb => px
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Rgba => px.to_vec(),
        other => panic!("{}: unexpected colour type {other:?}", path.display()),
    }
}

/// What a picture of the Chiaro golden `chiaro` (`.../tests/golden/chiaro/<rest>`) must match:
/// the re-blessed `.../tests/golden/rows/<rest>` where the light rows (ADR 0019) draw the scene
/// differently, else the Chiaro golden ([`load_png_rgba8`]).
///
/// Side effect: with `KNORKE_BLESS=1` it writes `actual` as the re-blessed golden (removes it when
/// `actual` equals Chiaro's) and returns `actual`.
pub fn expected_png_rgba8(chiaro: &Path, actual: &[u8], width: u32, height: u32) -> Vec<u8> {
    let text = chiaro.to_string_lossy().replace('\\', "/");
    let rows = PathBuf::from(text.replacen("golden/chiaro/", "golden/rows/", 1));
    assert_ne!(rows, PathBuf::from(&text), "not a Chiaro golden: {text}");
    if std::env::var_os("KNORKE_BLESS").is_some() {
        let rgb = |px: &[u8]| -> Vec<u8> {
            px.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        };
        if rgb(&load_png_rgba8(chiaro, width, height)) == rgb(actual) {
            let _ = std::fs::remove_file(&rows);
        } else {
            std::fs::create_dir_all(rows.parent().unwrap()).unwrap();
            save_png_rgba8(&rows, actual, width, height);
        }
        return actual.to_vec();
    }
    if rows.exists() {
        load_png_rgba8(&rows, width, height)
    } else {
        load_png_rgba8(chiaro, width, height)
    }
}

/// Writes tightly packed RGBA8 as a PNG (for looking at a failing scene).
pub fn save_png_rgba8(path: &Path, rgba: &[u8], width: u32, height: u32) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// Compares two RGBA8 images of `width` x `height` pixels, skipping the `ignore` rectangles.
///
/// The per-pixel difference is the largest absolute difference over R, G and B.
pub fn compare_rgba8(
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ignore: &[Rect],
    budget: &ParityBudget,
) -> Result<ParityReport, ParityError> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(ParityError::SizeMismatch)?;
    if a.len() != len || b.len() != len {
        return Err(ParityError::SizeMismatch);
    }
    let mut report = ParityReport::default();
    for y in 0..height {
        for x in 0..width {
            if ignore.iter().any(|r| r.contains(x, y)) {
                continue;
            }
            let i = (y as usize * width as usize + x as usize) * 4;
            let d = a[i]
                .abs_diff(b[i])
                .max(a[i + 1].abs_diff(b[i + 1]))
                .max(a[i + 2].abs_diff(b[i + 2]));
            report.compared += 1;
            report.sum_diff += u64::from(d);
            if d > 0 {
                report.differing += 1;
            }
            if d > budget.tolerance {
                report.outliers += 1;
            }
            report.max_diff = report.max_diff.max(d);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, v: u8) -> Vec<u8> {
        [v, v, v, 255].repeat((w * h) as usize)
    }

    fn set(img: &mut [u8], w: u32, x: u32, y: u32, v: u8) {
        let i = ((y * w + x) * 4) as usize;
        img[i] = v;
    }

    #[test]
    fn identical_images_pass_any_budget() {
        let a = image(8, 8, 40);
        let r = compare_rgba8(&a, &a, 8, 8, &[], &ParityBudget::STRICT).unwrap();
        assert_eq!(
            (r.compared, r.differing, r.outliers, r.max_diff),
            (64, 0, 0, 0)
        );
        assert!(r.within(&ParityBudget {
            tolerance: 0,
            max_outlier_ppm: 0,
            hard_limit: 0
        }));
    }

    #[test]
    fn differences_within_tolerance_are_not_outliers() {
        let a = image(10, 10, 100);
        let mut b = a.clone();
        set(&mut b, 10, 3, 3, 102);
        let r = compare_rgba8(&a, &b, 10, 10, &[], &ParityBudget::STRICT).unwrap();
        assert_eq!((r.differing, r.outliers, r.max_diff), (1, 0, 2));
        assert!(r.within(&ParityBudget {
            tolerance: 2,
            max_outlier_ppm: 0,
            hard_limit: 2
        }));
    }

    #[test]
    fn outlier_budget_is_a_share_of_the_compared_pixels() {
        let a = image(100, 100, 0);
        let mut b = a.clone();
        for x in 0..5 {
            set(&mut b, 100, x, 0, 50); // 5 of 10_000 pixels = 500 ppm
        }
        let budget = ParityBudget {
            tolerance: 2,
            max_outlier_ppm: 500,
            hard_limit: 100,
        };
        let r = compare_rgba8(&a, &b, 100, 100, &[], &budget).unwrap();
        assert_eq!((r.outliers, r.outlier_ppm()), (5, 500));
        assert!(r.within(&budget));
        assert!(!r.within(&ParityBudget {
            max_outlier_ppm: 499,
            ..budget
        }));
    }

    #[test]
    fn hard_limit_rejects_one_pixel_even_with_budget_left() {
        let a = image(100, 100, 0);
        let mut b = a.clone();
        set(&mut b, 100, 7, 7, 200);
        let budget = ParityBudget {
            tolerance: 2,
            max_outlier_ppm: 10_000,
            hard_limit: 128,
        };
        let r = compare_rgba8(&a, &b, 100, 100, &[], &budget).unwrap();
        assert_eq!(r.outliers, 1);
        assert!(!r.within(&budget));
    }

    #[test]
    fn channel_difference_is_the_largest_of_rgb_and_alpha_is_ignored() {
        let a = image(2, 1, 10);
        let mut b = a.clone();
        b[1] = 30; // green +20
        b[2] = 15; // blue +5
        b[3] = 0; // alpha ignored
        b[7] = 0;
        let r = compare_rgba8(&a, &b, 2, 1, &[], &ParityBudget::STRICT).unwrap();
        assert_eq!((r.max_diff, r.differing, r.sum_diff), (20, 1, 20));
    }

    #[test]
    fn ignored_rectangles_are_not_counted() {
        let a = image(20, 10, 0);
        let mut b = a.clone();
        set(&mut b, 20, 2, 2, 255); // inside the HUD
        set(&mut b, 20, 15, 5, 255); // outside
        let hud = [Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 4,
        }];
        let r = compare_rgba8(&a, &b, 20, 10, &hud, &ParityBudget::STRICT).unwrap();
        assert_eq!((r.compared, r.outliers), (200 - 40, 1));
        assert!(
            Rect {
                x: 5,
                y: 5,
                w: 2,
                h: 2
            }
            .contains(6, 6)
        );
        assert!(
            !Rect {
                x: 5,
                y: 5,
                w: 2,
                h: 2
            }
            .contains(7, 6)
        );
    }

    #[test]
    fn wrong_buffer_size_is_an_error() {
        let a = image(4, 4, 0);
        let b = image(4, 3, 0);
        assert_eq!(
            compare_rgba8(&a, &b, 4, 4, &[], &ParityBudget::LOOSE),
            Err(ParityError::SizeMismatch)
        );
    }

    #[test]
    fn empty_comparison_has_zero_ppm() {
        let r = ParityReport::default();
        assert_eq!(r.outlier_ppm(), 0);
        assert!(r.within(&ParityBudget::STRICT));
    }
}
