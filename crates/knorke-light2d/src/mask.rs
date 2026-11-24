// Origin: chiaro @ 9bc5b9b, src/lighting/Mask.hpp and Mask.cpp (stored texels, window shape
// rasterizer, pixel resampler, texel sampling) and the field lists of CHI_MaskShape and
// CHI_MaskPreset in include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Light masks (P10): the stored texels, the window shape rasterizer, the pixel resampler and the
//! CPU sampling that the mask shader mirrors. The store is [`MaskStore`](crate::MaskStore), the
//! placement of a mask for one light is in [`light_mask`](crate::light_mask).
//!
//! A mask is `MASK_SIZE x MASK_SIZE` RGBA8 texels, sRGB-encoded, row 0 the top (`v = +1`).
//!
//! Redesign against the C++ version:
//!
//! - [`MaskPreset`] is an enum (no "unknown preset"), [`MaskShape`] holds its glass colour as a
//!   [`Vec3`], and the builders write into a host-owned `Vec<u8>` and return typed errors
//!   ([`MaskError`], stable codes `knorke.light2d.mask.*`) instead of SDL error strings.
//! - Pixels are a `&[u8]` slice, so "pixels must not be NULL" is replaced by a length check.
//! - sRGB conversions use the exact tables of [`knorke_math`] (`Srgba8::to_linear`,
//!   `LinearRgba::to_srgba8`) instead of `powf` per texel; the encoded bytes can differ from the
//!   C++ rounding by one code in rare cases.

use knorke_math::{LinearRgba, Srgba8, Vec3, det};
use knorke_types::{Diagnostic, DiagnosticCode};

/// Texels per side of every mask.
pub const MASK_SIZE: usize = 256;
/// Bytes of one mask: `MASK_SIZE^2` RGBA8 texels.
pub const MASK_BYTES: usize = MASK_SIZE * MASK_SIZE * 4;
/// Largest pixel mask side [`build_pixel_mask`] accepts.
pub const MAX_MASK_PIXELS: u32 = 4096;

const MAX_PANES: u32 = 64;

/// Atlas blocks for masks up to block `block_count - 1`: a power of two, at least 1, at most 64;
/// 0 for 0.
pub fn mask_atlas_blocks(block_count: u32) -> u32 {
    if block_count == 0 {
        return 0;
    }
    block_count.next_power_of_two()
}

/// Presets of [`MaskShape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaskPreset {
    /// A window with a cross: 2 x 2 panes.
    WindowCross,
    /// A hall window: 4 x 3 panes.
    WindowHall,
    /// Venetian blinds: 1 x 10 slats.
    Blinds,
}

/// A parametric window shape. All widths are shares of the mask width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskShape {
    /// Panes across (u), `[1, 64]`.
    pub columns: u32,
    /// Panes up (v), `[1, 64]`.
    pub rows: u32,
    /// Width of each bar between two panes, `[0, 1)`.
    pub bar_width: f32,
    /// Width of the outer frame, `[0, 0.5)`.
    pub frame_width: f32,
    /// Width / height of the opening, `> 0` (0.75 for an upright window).
    pub aspect: f32,
    /// Blur of every edge, `[0, 0.5]`; 0 is as sharp as the mask's 256 texels allow.
    pub softness: f32,
    /// Colour of the panes, linear RGB, each in `[0, 1]`; `(1, 1, 1)` is clear glass.
    pub glass: Vec3,
}

impl MaskShape {
    /// The defaults of a preset.
    pub fn preset(preset: MaskPreset) -> Self {
        let (columns, rows, bar_width, frame_width, aspect, softness) = match preset {
            MaskPreset::WindowCross => (2, 2, 0.06, 0.06, 0.75, 0.01),
            MaskPreset::WindowHall => (4, 3, 0.035, 0.05, 1.6, 0.01),
            MaskPreset::Blinds => (1, 10, 0.05, 0.02, 1.0, 0.02),
        };
        Self {
            columns,
            rows,
            bar_width,
            frame_width,
            aspect,
            softness,
            glass: Vec3::ONE,
        }
    }

    /// Checks that the rasterizer can use the shape.
    ///
    /// # Errors
    /// The first violated rule, as a [`MaskError`].
    pub fn validate(&self) -> Result<(), MaskError> {
        if self.columns < 1 || self.columns > MAX_PANES || self.rows < 1 || self.rows > MAX_PANES {
            return Err(MaskError::PaneCount {
                columns: self.columns,
                rows: self.rows,
            });
        }
        in_range("bar_width", self.bar_width, 0.0, 1.0, true, "[0, 1)")?;
        in_range("frame_width", self.frame_width, 0.0, 0.5, true, "[0, 0.5)")?;
        in_range("softness", self.softness, 0.0, 0.5, false, "[0, 0.5]")?;
        if !self.aspect.is_finite() || self.aspect <= 0.0 || self.aspect > 64.0 {
            return Err(MaskError::OutOfRange {
                field: "aspect",
                range: "(0, 64]",
                value: self.aspect,
            });
        }
        let g = self.glass;
        for c in [g.x, g.y, g.z] {
            if !c.is_finite() || !(0.0..=1.0).contains(&c) {
                return Err(MaskError::OutOfRange {
                    field: "glass",
                    range: "[0, 1] per channel",
                    value: c,
                });
            }
        }
        if pane_width(self.columns, self.bar_width, self.frame_width) <= 0.0 {
            return Err(MaskError::NoGlass {
                axis: "across",
                count: self.columns,
                bar_width: self.bar_width,
                frame_width: self.frame_width,
                aspect: 1.0,
            });
        }
        if pane_width(
            self.rows,
            self.bar_width * self.aspect,
            self.frame_width * self.aspect,
        ) <= 0.0
        {
            return Err(MaskError::NoGlass {
                axis: "upwards",
                count: self.rows,
                bar_width: self.bar_width,
                frame_width: self.frame_width,
                aspect: self.aspect,
            });
        }
        Ok(())
    }

    /// Rasterizes a valid shape into `MASK_BYTES` texels (`out` is resized, its capacity kept).
    /// Call [`validate`](Self::validate) first.
    pub fn build(&self, out: &mut Vec<u8>) {
        out.clear();
        out.resize(MASK_BYTES, 0);
        let texel = 1.0 / MASK_SIZE as f32;
        let wu = det::max(texel, self.softness);
        let wv = det::max(texel, self.softness * self.aspect);
        let mut across = [0.0f32; MASK_SIZE];
        let mut up = [0.0f32; MASK_SIZE];
        for i in 0..MASK_SIZE {
            let x = (i as f32 + 0.5) * texel;
            across[i] = axis_coverage(x, wu, self.columns, self.bar_width, self.frame_width);
            up[i] = axis_coverage(
                x,
                wv,
                self.rows,
                self.bar_width * self.aspect,
                self.frame_width * self.aspect,
            );
        }
        let glass = self.glass;
        for (row, up) in out
            .as_chunks_mut::<{ MASK_SIZE * 4 }>()
            .0
            .iter_mut()
            .zip(up)
        {
            for (texel, across) in row.as_chunks_mut::<4>().0.iter_mut().zip(across) {
                let c = across * up;
                let rgba = LinearRgba::new(c * glass.x, c * glass.y, c * glass.z, 1.0).to_srgba8();
                texel.copy_from_slice(&[rgba.r, rgba.g, rgba.b, 255]);
            }
        }
    }
}

fn in_range(
    field: &'static str,
    v: f32,
    lo: f32,
    hi: f32,
    open_high: bool,
    range: &'static str,
) -> Result<(), MaskError> {
    let above = if open_high { v >= hi } else { v > hi };
    if !v.is_finite() || v < lo || above {
        return Err(MaskError::OutOfRange {
            field,
            range,
            value: v,
        });
    }
    Ok(())
}

/// Pane width of `count` panes between a frame and bars on an axis of length 1.
fn pane_width(count: u32, bar: f32, frame: f32) -> f32 {
    (1.0 - 2.0 * frame - (count - 1) as f32 * bar) / count as f32
}

/// Share of `[x - w / 2, x + w / 2]` covered by glass (one axis of the window, 0 = left or top).
fn axis_coverage(x: f32, w: f32, count: u32, bar: f32, frame: f32) -> f32 {
    let pane = pane_width(count, bar, frame);
    let (lo, hi) = (x - 0.5 * w, x + 0.5 * w);
    let mut covered = 0.0;
    for k in 0..count {
        let a = frame + k as f32 * (pane + bar);
        let b = a + pane;
        covered += det::max(0.0, det::min(hi, b) - det::max(lo, a));
    }
    (covered / w).clamp(0.0, 1.0)
}

/// Linear value of an 8-bit sRGB code (exact table).
fn decode(byte: u8) -> f32 {
    Srgba8::new(byte, byte, byte, 255).to_linear().r
}

/// Output index to (first source index, weights) of a 1D resample from `n` to `MASK_SIZE`,
/// kept in flat buffers.
struct Taps {
    first: Vec<usize>,
    /// `weights[start[i]..start[i + 1]]` belong to output `i`.
    start: Vec<usize>,
    weights: Vec<f32>,
}

impl Taps {
    fn new(n: u32) -> Self {
        let mut t = Self {
            first: Vec::with_capacity(MASK_SIZE),
            start: Vec::with_capacity(MASK_SIZE + 1),
            weights: Vec::new(),
        };
        let scale = f64::from(n) / MASK_SIZE as f64;
        let last = n as i64 - 1;
        for i in 0..MASK_SIZE {
            t.start.push(t.weights.len());
            if scale >= 1.0 {
                // Shrinking: the area average over the footprint [i scale, (i + 1) scale).
                let (a, b) = (i as f64 * scale, (i + 1) as f64 * scale);
                let j0 = a.floor() as i64;
                let j1 = (b.ceil() as i64).min(i64::from(n));
                t.first.push(j0 as usize);
                for j in j0..j1 {
                    let (lo, hi) = (j as f64, j as f64 + 1.0);
                    let overlap = (if b < hi { b } else { hi }) - (if a > lo { a } else { lo });
                    t.weights.push((overlap / scale) as f32);
                }
            } else {
                // Growing: bilinear at the texel centre, clamped to the outer pixels.
                let c = (i as f64 + 0.5) * scale - 0.5;
                let f0 = c.floor();
                let f = (c - f0) as f32;
                let j0 = (f0 as i64).clamp(0, last);
                let j1 = (f0 as i64 + 1).clamp(0, last);
                t.first.push(j0 as usize);
                if j1 == j0 {
                    t.weights.push(1.0);
                } else {
                    t.weights.push(1.0 - f);
                    t.weights.push(f);
                }
            }
        }
        t.start.push(t.weights.len());
        t
    }

    fn of(&self, i: usize) -> (usize, &[f32]) {
        (
            self.first[i],
            &self.weights[self.start[i]..self.start[i + 1]],
        )
    }
}

/// Resamples RGBA8 sRGB pixels into `MASK_BYTES` texels: per axis an area average in linear
/// light when shrinking, bilinear when growing; alpha is ignored and stored as 255. `pitch` is
/// the number of bytes from one row to the next.
///
/// # Errors
/// A side outside `[1, MAX_MASK_PIXELS]`, a pitch below `4 * width`, or fewer bytes than the
/// last row needs.
pub fn build_pixel_mask(
    width: u32,
    height: u32,
    pixels: &[u8],
    pitch: usize,
    out: &mut Vec<u8>,
) -> Result<(), MaskError> {
    if width < 1 || height < 1 || width > MAX_MASK_PIXELS || height > MAX_MASK_PIXELS {
        return Err(MaskError::PixelSize { width, height });
    }
    let row_bytes = width as usize * 4;
    if pitch < row_bytes {
        return Err(MaskError::Pitch {
            pitch,
            min: row_bytes,
        });
    }
    let needed = (height as usize - 1) * pitch + row_bytes;
    if pixels.len() < needed {
        return Err(MaskError::PixelsTooShort {
            needed,
            got: pixels.len(),
        });
    }
    let (tx, ty) = (Taps::new(width), Taps::new(height));
    // Horizontal pass: `height` rows of MASK_SIZE linear RGB texels.
    let mut rows = vec![0.0f32; height as usize * MASK_SIZE * 3];
    for y in 0..height as usize {
        let line = &pixels[y * pitch..y * pitch + row_bytes];
        for i in 0..MASK_SIZE {
            let (first, weights) = tx.of(i);
            let mut sum = [0.0f32; 3];
            for (k, w) in weights.iter().enumerate() {
                let p = (first + k) * 4;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += w * decode(line[p + c]);
                }
            }
            let dst = (y * MASK_SIZE + i) * 3;
            rows[dst..dst + 3].copy_from_slice(&sum);
        }
    }
    out.clear();
    out.resize(MASK_BYTES, 0);
    for j in 0..MASK_SIZE {
        let (first, weights) = ty.of(j);
        for i in 0..MASK_SIZE {
            let mut sum = [0.0f32; 3];
            for (k, w) in weights.iter().enumerate() {
                let p = ((first + k) * MASK_SIZE + i) * 3;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += w * rows[p + c];
                }
            }
            let rgba = LinearRgba::new(sum[0], sum[1], sum[2], 1.0).to_srgba8();
            let o = (j * MASK_SIZE + i) * 4;
            out[o..o + 4].copy_from_slice(&[rgba.r, rgba.g, rgba.b, 255]);
        }
    }
    Ok(())
}

/// Bilinear sample of the texels at mask coordinates `(u, v)` in linear light, as the GPU
/// samples the atlas: 0 outside `[-1, 1]^2`, the texel positions clamped to the outer texel
/// centres.
///
/// # Panics
/// If `texels` is shorter than `MASK_BYTES`.
pub fn sample_mask_texels(texels: &[u8], u: f32, v: f32) -> Vec3 {
    if !(u.abs() <= 1.0 && v.abs() <= 1.0) {
        return Vec3::ZERO; // outside the square, or NaN
    }
    let texels = &texels[..MASK_BYTES];
    let size = MASK_SIZE as f32;
    let last = size - 1.0;
    let x = ((u * 0.5 + 0.5) * size - 0.5).clamp(0.0, last);
    let y = ((0.5 - v * 0.5) * size - 0.5).clamp(0.0, last);
    let x0 = (x as usize).min(MASK_SIZE - 2);
    let y0 = (y as usize).min(MASK_SIZE - 2);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |xi: usize, yi: usize, c: usize| decode(texels[(yi * MASK_SIZE + xi) * 4 + c]);
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) + (at(x0 + 1, y0, c) - at(x0, y0, c)) * fx;
        let bottom = at(x0, y0 + 1, c) + (at(x0 + 1, y0 + 1, c) - at(x0, y0 + 1, c)) * fx;
        *o = top + (bottom - top) * fy;
    }
    Vec3::new(out[0], out[1], out[2])
}

/// Why a mask or a mask operation was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum MaskError {
    /// `columns` or `rows` of a shape is outside `[1, 64]`.
    PaneCount {
        /// Panes across.
        columns: u32,
        /// Panes up.
        rows: u32,
    },
    /// A shape value is NaN, infinite or outside its range.
    OutOfRange {
        /// Field name of the shape.
        field: &'static str,
        /// The allowed range, e.g. `[0, 1)`.
        range: &'static str,
        /// The received value.
        value: f32,
    },
    /// Bars and frame leave no glass on an axis.
    NoGlass {
        /// `across` or `upwards`.
        axis: &'static str,
        /// Columns or rows.
        count: u32,
        /// The bar width.
        bar_width: f32,
        /// The frame width.
        frame_width: f32,
        /// The aspect (1 across).
        aspect: f32,
    },
    /// A pixel mask side outside `[1, MAX_MASK_PIXELS]`.
    PixelSize {
        /// Pixel columns.
        width: u32,
        /// Pixel rows.
        height: u32,
    },
    /// A pixel row pitch below `4 * width`.
    Pitch {
        /// The received pitch.
        pitch: usize,
        /// The least allowed.
        min: usize,
    },
    /// Fewer pixel bytes than `width x height` rows with this pitch need.
    PixelsTooShort {
        /// Bytes needed.
        needed: usize,
        /// Bytes given.
        got: usize,
    },
    /// The texels of a mask are not `MASK_BYTES` long.
    TexelSize {
        /// The received length.
        got: usize,
    },
    /// The store already holds the most masks.
    TooMany {
        /// The limit.
        max: u32,
    },
    /// A mask handle that is stale or was never issued.
    InvalidHandle(u64),
    /// Lights still use the mask.
    InUse {
        /// How many lights.
        users: usize,
    },
}

impl MaskError {
    /// The stable diagnostic code (`knorke.light2d.mask.*`).
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::PaneCount { .. } | Self::OutOfRange { .. } => {
                DiagnosticCode::new("knorke.light2d.mask.out_of_range")
            }
            Self::NoGlass { .. } => DiagnosticCode::new("knorke.light2d.mask.no_glass"),
            Self::PixelSize { .. } | Self::Pitch { .. } | Self::PixelsTooShort { .. } => {
                DiagnosticCode::new("knorke.light2d.mask.pixels")
            }
            Self::TexelSize { .. } => DiagnosticCode::new("knorke.light2d.mask.texel_size"),
            Self::TooMany { .. } => DiagnosticCode::new("knorke.light2d.mask.too_many"),
            Self::InvalidHandle(_) => DiagnosticCode::new("knorke.light2d.mask.invalid_handle"),
            Self::InUse { .. } => DiagnosticCode::new("knorke.light2d.mask.in_use"),
        }
    }

    /// As a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.to_string())
    }
}

impl core::fmt::Display for MaskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PaneCount { columns, rows } => write!(
                f,
                "mask shape.columns and shape.rows must be in [1, {MAX_PANES}] \
                 (got {columns}, {rows})"
            ),
            Self::OutOfRange {
                field,
                range,
                value,
            } => write!(f, "mask shape.{field} must be in {range} (got {value})"),
            Self::NoGlass {
                axis,
                count,
                bar_width,
                frame_width,
                aspect,
            } => write!(
                f,
                "mask shape bars and frame leave no glass {axis}: {count} panes, bar_width \
                 {bar_width}, frame_width {frame_width}, aspect {aspect}; the bars and the \
                 frame (times aspect upwards) must leave room, \
                 (count - 1) * bar_width + 2 * frame_width < 1"
            ),
            Self::PixelSize { width, height } => write!(
                f,
                "mask pixels: width and height must be in [1, {MAX_MASK_PIXELS}] \
                 (got {width}, {height})"
            ),
            Self::Pitch { pitch, min } => {
                write!(
                    f,
                    "mask pixels: pitch must be >= 4 * width = {min} (got {pitch})"
                )
            }
            Self::PixelsTooShort { needed, got } => write!(
                f,
                "mask pixels: {got} bytes given, {needed} needed for the rows and the pitch"
            ),
            Self::TexelSize { got } => {
                write!(f, "a mask needs {MASK_BYTES} texel bytes (got {got})")
            }
            Self::TooMany { max } => write!(
                f,
                "too many light masks (at most {max} alive at once); destroy one first"
            ),
            Self::InvalidHandle(bits) => {
                write!(f, "invalid or destroyed mask handle 0x{bits:016x}")
            }
            Self::InUse { users } => write!(
                f,
                "the mask is still used by {users} light(s); set their mask to None or destroy \
                 them first"
            ),
        }
    }
}

impl std::error::Error for MaskError {}
