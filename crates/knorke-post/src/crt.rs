// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/core/CrtMath.hpp), zlib licence:
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

//! CRT steps of the output stage (E137): barrel warp, rounded-screen coverage, scanlines, aperture
//! grille and beam. These are the CPU mirrors of the shader steps; the tests here are their oracle.

use core::f32::consts::TAU;

use knorke_math::{Vec2, det};

use crate::clamp;
use crate::color::{linear_to_srgb, srgb_to_linear};

/// `k` of the barrel warp per unit of curvature: at curvature 1 a corner samples 1.25 half sides out.
pub const CRT_WARP_SCALE: f32 = 0.25;

/// Barrel warp of the scene coordinate `(u, v)` in `[0, 1]` (E137). Returns `c' / 2 + 1/2` with
/// `c' = c (1 + k r^2)`, where `r^2` is the squared distance from the centre in aspect-corrected units
/// (`ax`, `ay` = rect sides / shorter rect side), normalised so a corner has `r^2 = 1`.
pub fn crt_warp(u: f32, v: f32, ax: f32, ay: f32, k: f32) -> Vec2 {
    let cx = u * 2.0 - 1.0;
    let cy = v * 2.0 - 1.0;
    let r2 = (cx * cx * ax * ax + cy * cy * ay * ay) / (ax * ax + ay * ay);
    let f = 1.0 + k * r2;
    Vec2::new(cx * f * 0.5 + 0.5, cy * f * 0.5 + 0.5)
}

/// Coverage of the screen at the warped coordinate `(u, v)` (E137): 1 inside a rounded rectangle over
/// `[0, 1]^2`, 0 outside, a one-pixel ramp across the edge. `pixels_per_unit` is target pixels per half
/// shorter rect side.
pub fn crt_coverage(u: f32, v: f32, ax: f32, ay: f32, radius: f32, pixels_per_unit: f32) -> f32 {
    let px = ((u * 2.0 - 1.0) * ax).abs();
    let py = ((v * 2.0 - 1.0) * ay).abs();
    let qx = px - (ax - radius);
    let qy = py - (ay - radius);
    let ox = det::max(qx, 0.0);
    let oy = det::max(qy, 0.0);
    let d = (ox * ox + oy * oy).sqrt() + det::min(det::max(qx, qy), 0.0) - radius;
    clamp(0.5 - d * pixels_per_unit, 0.0, 1.0)
}

/// Share of the scanline strength that is used (E137): `saturate(h - 1)` with
/// `h = rect_height / scene_height * period`.
pub fn crt_scanline_fade(rect_height: f32, scene_height: f32, period: f32) -> f32 {
    let h = rect_height / det::max(scene_height, 1.0) * period;
    clamp(h - 1.0, 0.0, 1.0)
}

/// Scanline weight at the warped scene row position `y` (E137): a cosine profile over `frac(y / period)`
/// peaking at a quarter of the line, `1 - strength (1 - p)`.
pub fn crt_scanline_weight(y: f32, period: f32, strength: f32) -> f32 {
    let s = y / period;
    let t = s - s.floor();
    let p = 0.5 + 0.5 * det::cos(TAU * (t - 0.25));
    1.0 - strength * (1.0 - p)
}

/// Aperture grille weight of target pixel column `x` (E137): the channel of the third that the column
/// falls into keeps 1, the other two get `1 - mask`.
pub fn crt_mask_weight(x: u32, pitch: f32, mask: f32) -> [f32; 3] {
    let s = (x as f32 + 0.5) / pitch;
    let phase = s - s.floor();
    let third = ((phase * 3.0) as usize).min(2);
    let mut w = [1.0 - mask; 3];
    w[third] = 1.0;
    w
}

/// The beam of one sRGB-encoded channel (E137): decode, times weight and brightness, clamp, encode.
pub fn crt_beam(encoded: f32, weight: f32, brightness: f32) -> f32 {
    linear_to_srgb(clamp(
        srgb_to_linear(encoded) * weight * brightness,
        0.0,
        1.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::linear_to_srgb;
    use crate::testing::{approx, near};

    // Port of the shader-mirror tests of tests/unit/CrtTests.cpp. The settings, validation, flag and
    // wrapper tests of that file need CHI_CrtSettings and are not ported here.

    #[test]
    fn warp_keeps_the_centre_and_pushes_the_corners_out() {
        let ax = 16.0f32 / 9.0;
        let ay = 1.0f32;
        let c = crt_warp(0.5, 0.5, ax, ay, 0.25);
        assert!(approx(c.x, 0.5));
        assert!(approx(c.y, 0.5));
        // k = 0: identity.
        let c = crt_warp(0.1, 0.8, ax, ay, 0.0);
        assert!(approx(c.x, 0.1));
        assert!(approx(c.y, 0.8));
        // A corner has r^2 = 1: c' = c (1 + k).
        let c = crt_warp(1.0, 1.0, ax, ay, 0.25);
        assert!(approx(c.x, 1.125));
        assert!(approx(c.y, 1.125));
        // Same factor on both axes (radial): the direction from the centre is kept.
        let c = crt_warp(0.8, 0.6, ax, ay, 0.2);
        assert!(approx((c.x - 0.5) / (c.y - 0.5), 3.0));
        // The middle of the long edge moves out more than the middle of the short edge (aspect-correct r).
        let right = crt_warp(1.0, 0.5, ax, ay, 0.25);
        let top = crt_warp(0.5, 1.0, ax, ay, 0.25);
        assert!(right.x - 1.0 > top.y - 1.0);
    }

    #[test]
    fn coverage_of_the_rounded_screen() {
        let ax = 1.5f32;
        let ay = 1.0f32;
        let ppu = 50.0f32; // 150 x 100 rect
        assert_eq!(crt_coverage(0.5, 0.5, ax, ay, 0.0, ppu), 1.0);
        assert_eq!(crt_coverage(1.2, 0.5, ax, ay, 0.0, ppu), 0.0);
        // A pixel centre half a pixel inside a straight edge is fully covered, on the edge half.
        assert!(approx(
            crt_coverage(0.5 / 150.0, 0.5, ax, ay, 0.0, ppu),
            1.0
        ));
        assert!(approx(crt_coverage(0.0, 0.5, ax, ay, 0.0, ppu), 0.5));
        // The very corner is cut by a radius, but not without one.
        assert_eq!(crt_coverage(0.01, 0.01, ax, ay, 0.0, ppu), 1.0);
        assert_eq!(crt_coverage(0.01, 0.01, ax, ay, 0.2, ppu), 0.0);
        // Inside the radius circle the corner area is covered.
        assert_eq!(crt_coverage(0.1, 0.1, ax, ay, 0.2, ppu), 1.0);
    }

    #[test]
    fn scanline_profile_and_fade() {
        assert!(approx(crt_scanline_weight(0.25, 1.0, 0.6), 1.0)); // peak at a quarter
        assert!(approx(crt_scanline_weight(0.75, 1.0, 0.6), 0.4)); // dark at three quarters
        assert!(approx(crt_scanline_weight(3.75, 1.0, 0.6), 0.4));
        assert!(approx(crt_scanline_weight(0.0, 1.0, 0.6), 0.7)); // half way at the row border
        assert!(approx(crt_scanline_weight(0.5, 2.0, 1.0), 1.0)); // period 2: peak at 0.5
        // 2x upscale: the two target rows of a scene row differ (one bright, one dark).
        assert!(approx(
            crt_scanline_weight(0.25, 1.0, 0.5) - crt_scanline_weight(0.75, 1.0, 0.5),
            0.5
        ));
        assert_eq!(crt_scanline_weight(0.25, 1.0, 0.0), 1.0);
        // Mean over a line is 1 - strength / 2.
        let mut sum = 0.0f32;
        for i in 0..64 {
            sum += crt_scanline_weight((i as f32 + 0.5) / 64.0, 1.0, 0.5);
        }
        assert!(near(sum / 64.0, 0.75, 0.001));

        assert_eq!(crt_scanline_fade(360.0, 360.0, 1.0), 0.0); // 1:1
        assert!(approx(crt_scanline_fade(540.0, 360.0, 1.0), 0.5));
        assert_eq!(crt_scanline_fade(720.0, 360.0, 1.0), 1.0); // 2x
        assert_eq!(crt_scanline_fade(1440.0, 360.0, 1.0), 1.0); // 4x
        assert_eq!(crt_scanline_fade(360.0, 360.0, 2.0), 1.0); // 1:1, but two rows per line
    }

    #[test]
    fn aperture_grille_thirds() {
        let r = crt_mask_weight(0, 3.0, 0.4);
        let g = crt_mask_weight(1, 3.0, 0.4);
        let b = crt_mask_weight(2, 3.0, 0.4);
        assert_eq!(r[0], 1.0);
        assert!(approx(r[1], 0.6));
        assert!(approx(r[2], 0.6));
        assert_eq!(g[1], 1.0);
        assert!(approx(g[0], 0.6));
        assert_eq!(b[2], 1.0);
        assert_eq!(crt_mask_weight(3, 3.0, 0.4)[0], 1.0); // repeats
        // Pitch 6: two pixels per stripe.
        assert_eq!(crt_mask_weight(0, 6.0, 0.4)[0], 1.0);
        assert_eq!(crt_mask_weight(1, 6.0, 0.4)[0], 1.0);
        assert_eq!(crt_mask_weight(2, 6.0, 0.4)[1], 1.0);
        assert_eq!(crt_mask_weight(5, 6.0, 0.4)[2], 1.0);
    }

    #[test]
    fn beam_multiplies_linear_values() {
        assert!(approx(crt_beam(0.5, 1.0, 1.0), 0.5));
        assert!(approx(crt_beam(1.0, 0.5, 1.0), linear_to_srgb(0.5)));
        assert!(approx(crt_beam(1.0, 0.5, 4.0), 1.0)); // clamped
        assert_eq!(crt_beam(0.0, 1.0, 4.0), 0.0);
    }
}
