// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/core/Bloom.hpp), zlib licence:
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

//! Bloom level plan (E28): the number of levels of the bloom chain and the fade-in of its coarsest level.
//!
//! The settings defaults and value ranges of `CHI_BloomSettings` are not ported here (they need the C
//! settings struct); only the constants the level plan uses are.

use knorke_math::det;

use crate::clamp;

/// Most levels of the chain; level 0 has half the scene resolution.
pub const MAX_BLOOM_LEVELS: u32 = 8;
/// A level is only added while it keeps at least this many texels in both directions.
pub const MIN_BLOOM_LEVEL_SIZE: u32 = 8;
/// Smallest radius the plan accepts (values below are clamped to it).
pub const MIN_BLOOM_RADIUS: f32 = 1.0;
/// Largest radius the plan accepts (values above are clamped to it).
pub const MAX_BLOOM_RADIUS: f32 = 256.0;

/// Size of level `level` along one axis: the scene size halved `level + 1` times, rounded up, at least 1.
pub fn bloom_level_size(scene_size: u32, level: u32) -> u32 {
    let mut size = scene_size.max(1);
    for _ in 0..=level {
        size = size.div_ceil(2);
    }
    size
}

/// The level plan of a bloom chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomPlan {
    /// Number of levels, `1..=MAX_BLOOM_LEVELS`.
    pub levels: u32,
    /// Weight of the coarsest level, in `(0, 1]`: fine control of the reach between powers of 2.
    pub coarsest_weight: f32,
}

/// Plans the chain for `radius` on a scene of `scene_w` x `scene_h` texels.
pub fn plan_bloom(radius: f32, scene_w: u32, scene_h: u32) -> BloomPlan {
    let r = clamp(radius, MIN_BLOOM_RADIUS, MAX_BLOOM_RADIUS);
    let wanted = det::log2(r).ceil() as i32 - 1;
    let mut levels = wanted.clamp(1, MAX_BLOOM_LEVELS as i32) as u32;
    let mut cut = false;
    while levels > 1
        && (bloom_level_size(scene_w, levels - 1) < MIN_BLOOM_LEVEL_SIZE
            || bloom_level_size(scene_h, levels - 1) < MIN_BLOOM_LEVEL_SIZE)
    {
        levels -= 1;
        cut = true;
    }
    let mut coarsest_weight = 1.0;
    if levels > 1 && !cut {
        let base = (1u32 << levels) as f32; // 2^L
        coarsest_weight = clamp((r - base) / base, 0.0, 1.0);
    }
    BloomPlan {
        levels,
        coarsest_weight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::approx;

    // Port of the level-plan tests of tests/unit/BloomTests.cpp. The settings tests of that file need
    // CHI_BloomSettings and Settings and are not ported here.

    #[test]
    fn bloom_levels_follow_the_radius() {
        // levels = clamp(ceil(log2(radius)) - 1, 1, 8), limited to levels of at least 8x8 texels.
        let cases: [(f32, u32); 12] = [
            (1.0, 1),
            (2.0, 1),
            (4.0, 1),
            (4.5, 2),
            (8.0, 2),
            (9.0, 3),
            (16.0, 3),
            (17.0, 4),
            (32.0, 4),
            (64.0, 5),
            (128.0, 6),
            (256.0, 7),
        ];
        for (radius, levels) in cases {
            assert_eq!(
                plan_bloom(radius, 4096, 4096).levels,
                levels,
                "radius {radius}"
            );
        }
        // 512x400: levels 256x200, 128x100, 64x50, 32x25, 16x13; 8x7 would be too small.
        assert_eq!(plan_bloom(16.0, 512, 400).levels, 3);
        assert_eq!(plan_bloom(64.0, 512, 400).levels, 5);
        assert_eq!(plan_bloom(256.0, 512, 400).levels, 5);
        // Tiny scenes keep one level, however small it is.
        assert_eq!(plan_bloom(64.0, 20, 20).levels, 1);
        assert_eq!(plan_bloom(64.0, 3, 1).levels, 1);
    }

    #[test]
    fn the_coarsest_bloom_level_fades_in_between_powers_of_two() {
        assert!(approx(plan_bloom(16.0, 4096, 4096).coarsest_weight, 1.0));
        assert!(approx(plan_bloom(12.0, 4096, 4096).coarsest_weight, 0.5));
        assert!(approx(plan_bloom(9.0, 4096, 4096).coarsest_weight, 0.125));
        assert_eq!(plan_bloom(1.0, 4096, 4096).coarsest_weight, 1.0); // a single level always counts fully
        assert_eq!(plan_bloom(3.0, 4096, 4096).coarsest_weight, 1.0);
        assert_eq!(plan_bloom(256.0, 512, 400).coarsest_weight, 1.0); // limited by the scene: no fade
        // The weight grows with the radius.
        let mut previous = 0.0f32;
        let mut r = 8.25f32;
        while r <= 16.0 {
            let p = plan_bloom(r, 4096, 4096);
            assert_eq!(p.levels, 3, "radius {r}");
            assert!(p.coarsest_weight > previous, "radius {r}");
            previous = p.coarsest_weight;
            r += 0.25;
        }
    }

    #[test]
    fn bloom_level_sizes_halve_and_round_up() {
        assert_eq!(bloom_level_size(512, 0), 256);
        assert_eq!(bloom_level_size(400, 2), 50);
        assert_eq!(bloom_level_size(400, 3), 25);
        assert_eq!(bloom_level_size(400, 4), 13);
        assert_eq!(bloom_level_size(101, 0), 51);
        assert_eq!(bloom_level_size(1, 0), 1);
        assert_eq!(bloom_level_size(1, 5), 1);
    }
}
