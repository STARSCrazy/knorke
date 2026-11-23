use crate::Vec4;
use crate::srgb_table::{SRGB_ENCODE_THRESHOLDS, SRGB_TO_LINEAR};

/// Linear-light RGBA, premultiplication **not** implied. Layout: `[r, g, b, a]`, 16 bytes.
///
/// All lighting math happens in this space (ADR 0002 §5). Convert from/to [`Srgba8`] only
/// at the edges: texture import and final output.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinearRgba {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha (coverage), always linear.
    pub a: f32,
}

/// 8-bit sRGB-encoded RGBA with linear alpha, as stored in typical textures. Layout:
/// `[r, g, b, a]`, 4 bytes – matches `R8G8B8A8_UNORM_SRGB`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Srgba8 {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha.
    pub a: u8,
}

impl LinearRgba {
    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    /// Opaque white.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// Fully transparent.
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Creates a color from linear components.
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Same memory as a [`Vec4`] (`x = r`, …).
    #[inline]
    pub const fn to_vec4(self) -> Vec4 {
        Vec4::new(self.r, self.g, self.b, self.a)
    }

    /// Multiplies the color channels by alpha.
    #[inline]
    pub fn premultiplied(self) -> Self {
        Self::new(self.r * self.a, self.g * self.a, self.b * self.a, self.a)
    }

    /// Encodes to 8-bit sRGB (exact piecewise sRGB curve, rounded half up, clamped; NaN → 0).
    pub fn to_srgba8(self) -> Srgba8 {
        // Exact table lookup (8 comparisons), deterministic on every platform.
        fn enc(c: f32) -> u8 {
            SRGB_ENCODE_THRESHOLDS.partition_point(|&t| t <= c) as u8
        }
        Srgba8 {
            r: enc(self.r),
            g: enc(self.g),
            b: enc(self.b),
            a: (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        }
    }
}

impl Srgba8 {
    /// Creates a color from sRGB-encoded bytes.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Decodes to linear light (exact piecewise sRGB curve).
    pub fn to_linear(self) -> LinearRgba {
        fn dec(c: u8) -> f32 {
            SRGB_TO_LINEAR[c as usize]
        }
        LinearRgba::new(dec(self.r), dec(self.g), dec(self.b), self.a as f32 / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trip_is_exact_for_all_bytes() {
        for v in 0..=255u8 {
            let c = Srgba8::new(v, v, v, v);
            assert_eq!(c.to_linear().to_srgba8(), c, "byte {v}");
        }
    }

    #[test]
    fn table_matches_curve_within_one_code() {
        fn enc_ref(c: f64) -> f64 {
            let c = c.clamp(0.0, 1.0);
            let s = if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * libm::pow(c, 1.0 / 2.4) - 0.055
            };
            s * 255.0
        }
        for i in 0..=10_000 {
            let c = i as f32 / 10_000.0;
            let got = LinearRgba::new(c, c, c, 1.0).to_srgba8().r as f64;
            let exact = enc_ref(c as f64);
            assert!((got - exact).abs() <= 0.5 + 1e-4, "{c}: {got} vs {exact}");
        }
        assert_eq!(
            LinearRgba::new(f32::NAN, -1.0, 2.0, 0.5).to_srgba8(),
            Srgba8::new(0, 0, 255, 128)
        );
    }

    #[test]
    fn known_values() {
        let mid = Srgba8::new(188, 188, 188, 255).to_linear();
        assert!((mid.r - 0.502_886).abs() < 1e-5, "{}", mid.r);
        assert_eq!(
            LinearRgba::WHITE.to_srgba8(),
            Srgba8::new(255, 255, 255, 255)
        );
        assert_eq!(
            LinearRgba::new(0.5, 1.0, 0.0, 0.5).premultiplied(),
            LinearRgba::new(0.25, 0.5, 0.0, 0.5)
        );
    }
}
