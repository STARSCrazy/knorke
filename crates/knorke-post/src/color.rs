// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/core/ColorMath.hpp), zlib licence:
//
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.

//! Colour transfer functions, tonemapping, grading and dither threshold (ported from `ColorMath.hpp`).

use knorke_math::{LinearRgba, det};

use crate::clamp;

/// Rec. 709 luminance weights of linear RGB (saturation, E110).
pub const LUMA_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// IEC 61966-2-1 sRGB decode of one channel in `[0, 1]`.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        det::pow((c + 0.055) / 1.055, 2.4)
    }
}

/// IEC 61966-2-1 sRGB encode of one channel in `[0, 1]`.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * det::pow(c, 1.0 / 2.4) - 0.055
    }
}

/// Linear multiplier for an exposure given in photographic stops (EV).
pub fn exposure_scale(ev: f32) -> f32 {
    det::exp2(ev)
}

/// `CHI_TONEMAP_SOFT` on linear RGB (after exposure). Identity up to the knee on the brightest channel
/// `m`; above it `k + (1 - k) * (1 - exp(-(m - k) / (1 - k)))`. The colour is scaled by `f(m) / m` to keep
/// its hue, and above `m = 4` it is mixed towards the grey `f(m)` by up to one half (`m = 16`).
/// The alpha of `rgb` is passed through unchanged.
pub fn tonemap_soft(rgb: LinearRgba, knee: f32) -> LinearRgba {
    let m = det::max(det::max(rgb.r, rgb.g), rgb.b);
    if m <= knee {
        return rgb;
    }
    let f = knee + (1.0 - knee) * (1.0 - det::exp(-(m - knee) / (1.0 - knee)));
    let s = f / m;
    let t = clamp((m - 4.0) / 12.0, 0.0, 1.0) * 0.5;
    let mix = |c: f32| c * s + (f - c * s) * t;
    LinearRgba {
        r: mix(rgb.r),
        g: mix(rgb.g),
        b: mix(rgb.b),
        a: rgb.a,
    }
}

/// `CHI_TONEMAP_ACES` for one linear channel: Narkowicz 2015, saturated to `[0, 1]`.
pub fn tonemap_aces(x: f32) -> f32 {
    let y = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
    clamp(y, 0.0, 1.0)
}

/// The linear part of the colour grading (E110): tint, clamp, then mix with the luminance by
/// `saturation`, clamp. Input: tonemapped linear RGB in `[0, 1]`. The tint's alpha is ignored and the
/// alpha of `rgb` is passed through.
pub fn grade_linear(rgb: LinearRgba, tint: LinearRgba, saturation: f32) -> LinearRgba {
    let r = clamp(rgb.r * tint.r, 0.0, 1.0);
    let g = clamp(rgb.g * tint.g, 0.0, 1.0);
    let b = clamp(rgb.b * tint.b, 0.0, 1.0);
    let y = r * LUMA_WEIGHTS[0] + g * LUMA_WEIGHTS[1] + b * LUMA_WEIGHTS[2];
    let sat = |c: f32| clamp(y + (c - y) * saturation, 0.0, 1.0);
    LinearRgba {
        r: sat(r),
        g: sat(g),
        b: sat(b),
        a: rgb.a,
    }
}

/// Ordered dither threshold of target pixel `(x, y)` in `[-0.5, 0.5)`, mean 0 (E114): the 8x8 Bayer index
/// (bits of `y` and `x ^ y` interleaved, least significant coordinate bit first), centred in its cell.
pub fn dither_threshold(x: u32, y: u32) -> f32 {
    let x = x & 7;
    let y = y & 7;
    let xy = x ^ y;
    let index = ((y & 1) << 5)
        | ((xy & 1) << 4)
        | ((y & 2) << 2)
        | ((xy & 2) << 1)
        | ((y & 4) >> 1)
        | ((xy & 4) >> 2);
    (index as f32 + 0.5) / 64.0 - 0.5
}

/// Contrast of one sRGB-encoded display value (E110): `(v - 0.5) * contrast + 0.5`, clamped to `[0, 1]`.
pub fn grade_contrast(encoded: f32, contrast: f32) -> f32 {
    clamp((encoded - 0.5) * contrast + 0.5, 0.0, 1.0)
}

/// Share of the vignette colour at distance `d` from the scene centre in half shorter scene sides
/// (E110): `strength * smoothstep(radius, radius + softness, d)`; softness 0 is a hard step at the radius.
pub fn vignette_weight(d: f32, strength: f32, radius: f32, softness: f32) -> f32 {
    let t = if softness > 0.0 {
        clamp((d - radius) / softness, 0.0, 1.0)
    } else if d >= radius {
        1.0
    } else {
        0.0
    };
    strength * t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{approx, near};
    use knorke_math::det;

    fn rgb(r: f32, g: f32, b: f32) -> LinearRgba {
        LinearRgba { r, g, b, a: 1.0 }
    }

    fn soft(r: f32, g: f32, b: f32, knee: f32) -> LinearRgba {
        tonemap_soft(rgb(r, g, b), knee)
    }

    // Port of tests/unit/ColorMathTests.cpp.

    #[test]
    fn srgb_transfer_functions() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!(approx(srgb_to_linear(1.0), 1.0));
        assert!(near(srgb_to_linear(0.5), 0.214041, 1e-5));
        assert!(near(srgb_to_linear(0.04045), 0.0031308, 1e-5));
        assert!(near(linear_to_srgb(0.214041), 0.5, 1e-5));
    }

    #[test]
    fn eight_bit_srgb_values_survive_a_round_trip_through_linear() {
        for i in 0..256 {
            let v = linear_to_srgb(srgb_to_linear(i as f32 / 255.0));
            assert_eq!((v * 255.0).round() as i32, i, "value {i}");
        }
    }

    #[test]
    fn exposure_in_stops() {
        assert_eq!(exposure_scale(0.0), 1.0);
        assert_eq!(exposure_scale(1.0), 2.0);
        assert_eq!(exposure_scale(-2.0), 0.25);
    }

    #[test]
    fn soft_tonemap_keeps_values_below_the_knee() {
        for v in [0.0f32, 0.1, 0.5, 0.79, 0.8] {
            let out = soft(v, v * 0.5, 0.0, 0.8);
            assert_eq!(out.r, v, "v {v}");
            assert_eq!(out.g, v * 0.5, "v {v}");
            assert_eq!(out.b, 0.0, "v {v}");
        }
    }

    #[test]
    fn soft_tonemap_rises_monotonically_below_1() {
        // With the default knee 0.8 the shoulder reaches 1 - 2e-8 at 4, which rounds to 1 in float; knee 0.5
        // keeps the shoulder measurable up to 4.
        let mut previous = 0.0f32;
        let mut v = 0.05f32;
        while v <= 16.0 {
            let out = soft(v, v, v, 0.8).r;
            assert!(out >= previous - 1e-6, "v {v}");
            assert!(out <= 1.0, "v {v}");
            previous = out;
            v += 0.05;
        }
        assert!(soft(4.0, 0.0, 0.0, 0.5).r < 1.0);
        assert!(soft(4.0, 0.0, 0.0, 0.5).r > soft(2.0, 0.0, 0.0, 0.5).r);
        assert!(soft(1.0, 0.0, 0.0, 0.8).r < 1.0);
        assert!(approx(
            soft(1.0, 0.0, 0.0, 0.8).r,
            0.8 + 0.2 * (1.0 - det::exp(-1.0))
        ));
    }

    #[test]
    fn soft_tonemap_keeps_the_hue_of_bright_colours() {
        let orange = soft(4.0, 2.0, 0.0, 0.8);
        assert!(orange.r > orange.g);
        assert!(orange.g > orange.b);
        assert!(near(orange.g / orange.r, 0.5, 0.01));
        // Very bright values drift towards white.
        let blast = soft(16.0, 8.0, 0.0, 0.8);
        assert!(blast.g / blast.r > 0.7);
        assert!(blast.b > 0.4);
    }

    #[test]
    fn aces_tonemap_follows_the_narkowicz_2015_fit() {
        assert_eq!(tonemap_aces(0.0), 0.0);
        assert!(near(tonemap_aces(0.5), 0.6163, 1e-3));
        assert!(near(tonemap_aces(1.0), 0.8038, 1e-3));
        assert!(near(tonemap_aces(4.0), 0.9734, 1e-3));
        assert_eq!(tonemap_aces(100.0), 1.0);
    }

    // Port of the CPU-mirror and dither tests of tests/unit/OutputGradeTests.cpp.

    #[test]
    fn grading_cpu_mirrors() {
        // Saturation 0 gives the Rec. 709 luminance in every channel.
        let grey = grade_linear(rgb(1.0, 0.0, 0.0), rgb(1.0, 1.0, 1.0), 0.0);
        assert!(approx(grey.r, 0.2126));
        assert!(approx(grey.g, 0.2126));
        assert!(approx(grey.b, 0.2126));
        // Neutral values change nothing.
        let same = grade_linear(rgb(0.3, 0.6, 0.9), rgb(1.0, 1.0, 1.0), 1.0);
        assert!(approx(same.r, 0.3));
        assert!(approx(same.b, 0.9));
        // Tint multiplies and clamps; saturation 2 pushes away from the luminance and clamps at 0 and 1.
        let tinted = grade_linear(rgb(0.5, 0.5, 0.5), rgb(1.5, 1.0, 0.5), 1.0);
        assert!(approx(tinted.r, 0.75));
        assert!(approx(tinted.b, 0.25));
        let vivid = grade_linear(rgb(0.6, 0.2, 0.2), rgb(1.0, 1.0, 1.0), 2.0);
        let y = 0.6 * 0.2126 + 0.2 * 0.7152 + 0.2 * 0.0722;
        assert!(approx(vivid.r, y + 2.0 * (0.6 - y)));
        assert!(approx(vivid.g, y + 2.0 * (0.2 - y)));
        let clipped = grade_linear(rgb(0.9, 0.1, 0.1), rgb(1.0, 1.0, 1.0), 2.0);
        assert_eq!(clipped.r, 1.0); // 0.27 + 2 * 0.63 > 1
        assert_eq!(clipped.g, 0.0); // 0.27 - 2 * 0.17 < 0
        // Contrast pivots at display mid-grey.
        assert_eq!(grade_contrast(0.5, 3.0), 0.5);
        assert!(approx(grade_contrast(0.6, 2.0), 0.7));
        assert_eq!(grade_contrast(0.25, 2.0), 0.0);
        assert_eq!(grade_contrast(0.9, 0.0), 0.5);
        // Vignette: nothing inside the radius, the full strength from radius + softness on, smoothstep between.
        assert_eq!(vignette_weight(0.4, 0.8, 0.5, 1.5), 0.0);
        assert!(approx(vignette_weight(2.0, 0.8, 0.5, 1.5), 0.8));
        assert!(approx(vignette_weight(1.25, 1.0, 0.5, 1.5), 0.5));
        assert_eq!(vignette_weight(0.49, 1.0, 0.5, 0.0), 0.0);
        assert_eq!(vignette_weight(0.5, 1.0, 0.5, 0.0), 1.0);
    }

    #[test]
    fn dither_threshold_is_a_permutation_of_64_cells_with_mean_0() {
        let mut seen = [0u32; 64];
        let mut sum = 0.0f64;
        for y in 0..8u32 {
            for x in 0..8u32 {
                let t = dither_threshold(x, y);
                assert!((-0.5..0.5).contains(&t));
                let index = ((t + 0.5) * 64.0 - 0.5).round() as i32;
                assert!((0..64).contains(&index));
                seen[index as usize] += 1;
                sum += f64::from(t);
                assert_eq!(dither_threshold(x + 8, y + 16), t); // period 8 in both axes
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        assert!(sum.abs() < 1e-4);
        // Neighbours differ: the pattern is not a ramp (a 2x2 block holds thresholds a quarter apart).
        assert!(approx(dither_threshold(0, 0), -0.5 + 0.5 / 64.0));
        assert!((dither_threshold(1, 0) - dither_threshold(0, 0)).abs() >= 0.25);
    }
}
