//! sRGB <-> linear conversion for the shading pass (ADR 0013 §6).
//!
//! Colours (textures, background, fog, particles) are stored as 8-bit sRGB. The shading pass
//! decodes them to linear values on a 0..255 scale through a 256-entry table, multiplies light
//! and fog there, and encodes the result back to sRGB exactly once per pixel through a
//! 4097-entry table with linear interpolation. Both tables are built once per process from the
//! exact decode table of `knorke-math` and `det::pow`, so no pixel evaluates a power function.

use knorke_math::{Srgba8, det};
use std::sync::OnceLock;

/// Segments of the encode table over linear 0..1.
const ENCODE_STEPS: usize = 4096;

pub(crate) struct Tables {
    /// Linear value (0..255 scale) of each sRGB code.
    pub(crate) decode: [f32; 256],
    /// sRGB value (0..255 scale, fractional) at linear `i / ENCODE_STEPS`.
    encode: [f32; ENCODE_STEPS + 1],
}

impl Tables {
    /// Linear value (0..255 scale) of an sRGB colour.
    #[inline(always)]
    pub(crate) fn lin(&self, c: [u8; 3]) -> [f32; 3] {
        [
            self.decode[c[0] as usize],
            self.decode[c[1] as usize],
            self.decode[c[2] as usize],
        ]
    }

    /// sRGB value (0..255 scale, not rounded) of a linear value on the 0..255 scale; clamped.
    #[inline(always)]
    pub(crate) fn encode(&self, v: f32) -> f32 {
        let x = (v * (ENCODE_STEPS as f32 / 255.0)).clamp(0.0, ENCODE_STEPS as f32);
        if x.is_nan() {
            return 0.0;
        }
        let i = (x as usize).min(ENCODE_STEPS - 1);
        let t = x - i as f32;
        let (a, b) = (self.encode[i], self.encode[i + 1]);
        a + (b - a) * t
    }
}

/// The process-wide tables (built on first use, no allocation).
pub(crate) fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut decode = [0.0; 256];
        for (v, d) in decode.iter_mut().enumerate() {
            *d = Srgba8::new(v as u8, 0, 0, 255).to_linear().r * 255.0;
        }
        let mut encode = [0.0; ENCODE_STEPS + 1];
        for (i, e) in encode.iter_mut().enumerate() {
            let l = i as f32 / ENCODE_STEPS as f32;
            let s = if l <= 0.003_130_8 {
                l * 12.92
            } else {
                1.055 * det::pow(l, 1.0 / 2.4) - 0.055
            };
            *e = s.clamp(0.0, 1.0) * 255.0;
        }
        // exact ends (pow(1, x) need not give exactly 1 after the offset)
        encode[ENCODE_STEPS] = 255.0;
        Tables { decode, encode }
    })
}

/// Linear value (0..255 scale) of colour `0xRRGGBB`.
pub(crate) fn lin_of(c: u32) -> [f32; 3] {
    tables().lin([(c >> 16) as u8, (c >> 8) as u8, c as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_every_code() {
        let t = tables();
        for v in 0..=255u8 {
            let e = t.encode(t.decode[v as usize]);
            assert_eq!(e.round() as u8, v, "code {v}: {e}");
            assert!((e - v as f32).abs() < 0.05, "code {v}: {e}");
        }
    }

    #[test]
    fn ends_and_clamping() {
        let t = tables();
        assert_eq!(t.decode[0], 0.0);
        assert!((t.decode[255] - 255.0).abs() < 1e-4);
        assert_eq!(t.encode(-5.0), 0.0);
        assert_eq!(t.encode(1000.0), 255.0);
        assert_eq!(t.encode(f32::NAN), 0.0);
        // mid grey in linear light is much brighter than code 128 in sRGB
        assert!(t.encode(127.5) > 185.0);
    }
}
