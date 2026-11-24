// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/rendering/OutputFlags.hpp), zlib licence:
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
//
// Altered: rewritten in Rust for the Knorke Engine; the flags take the Rust settings types.

//! The flag word of the output shader (`cbuffer OutputParams`, `output.frag.hlsl`).
//!
//! Every optional step of the output pass runs only with its flag, so neutral settings execute
//! exactly the shader code without that step and the image stays bit-identical. The bit values are
//! the contract with the compiled shader and must not change.

use crate::settings::{CrtSettings, Filter, OutputSettings};

/// The target stores encoded values: the shader encodes to sRGB.
pub const ENCODE_SRGB: u32 = 1;
/// The frame has an overlay.
pub const HAS_OVERLAY: u32 = 2;
/// The overlay samples linear values (`*_SRGB` and float formats), blended in linear space.
pub const OVERLAY_LINEAR: u32 = 4;
/// Bloom is added to the exposed scene.
pub const BLOOM: u32 = 8;
/// Bloom per scene pixel (nearest upscaling), so the glow keeps the pixel grid.
pub const BLOOM_SNAP: u32 = 16;
/// Tint and saturation.
pub const GRADE: u32 = 32;
/// Contrast.
pub const CONTRAST: u32 = 64;
/// Vignette.
pub const VIGNETTE: u32 = 128;
/// Vignette per scene pixel (nearest upscaling).
pub const SCENE_SNAP: u32 = 256;
/// Ordered dither of the scene.
pub const DITHER: u32 = 512;
/// Bend and rounded corners of the screen.
pub const CRT_WARP: u32 = 1024;
/// Scanlines.
pub const CRT_SCANLINES: u32 = 2048;
/// Aperture grille.
pub const CRT_MASK: u32 = 4096;
/// Linear multiply of scanlines, mask and brightness.
pub const CRT_BEAM: u32 = 8192;
/// Overlay warped onto the screen.
pub const CRT_OVERLAY: u32 = 16384;
/// Radial displacement before the scene is read.
pub const SHOCKWAVE: u32 = 32768;
/// The offset per scene pixel (nearest upscaling).
pub const SHOCKWAVE_SNAP: u32 = 65536;

/// The grading and vignette flags (E110): each step only with non-neutral values. Nearest
/// filtering snaps the vignette to scene pixels.
pub fn grade_flags(output: &OutputSettings) -> u32 {
    let mut flags = 0;
    let t = output.tint;
    if output.saturation != 1.0 || t[0] != 1.0 || t[1] != 1.0 || t[2] != 1.0 {
        flags |= GRADE;
    }
    if output.contrast != 1.0 {
        flags |= CONTRAST;
    }
    if output.vignette.strength > 0.0 {
        flags |= VIGNETTE;
        if output.filter == Filter::Nearest {
            flags |= SCENE_SNAP;
        }
    }
    flags
}

/// The CRT flags (E137). `scanline_fade` is [`crt_scanline_fade`](crate::crt::crt_scanline_fade)
/// of the frame (0 at 1:1 output, so scanlines alone then set no flag); `overlay` whether the
/// frame has an overlay. The bend flag needs curvature or corner radius above 0; the beam flag is
/// set by any scanline, mask or brightness effect; the overlay flag only with the bend, an overlay
/// and [`CrtSettings::overlay`]. The CRT off gives 0.
pub fn crt_flags(crt: &CrtSettings, scanline_fade: f32, overlay: bool) -> u32 {
    let mut flags = 0;
    if crt.curvature > 0.0 || crt.corner_radius > 0.0 {
        flags |= CRT_WARP;
        if overlay && crt.overlay {
            flags |= CRT_OVERLAY;
        }
    }
    if crt.scanlines * scanline_fade > 0.0 {
        flags |= CRT_SCANLINES | CRT_BEAM;
    }
    if crt.mask > 0.0 {
        flags |= CRT_MASK | CRT_BEAM;
    }
    if crt.brightness != 1.0 {
        flags |= CRT_BEAM;
    }
    flags
}

/// The shockwave flags for `count` distorting waves in view: none for 0, the snap with nearest
/// filtering.
pub fn shockwave_flags(count: usize, filter: Filter) -> u32 {
    if count == 0 {
        return 0;
    }
    SHOCKWAVE
        | if filter == Filter::Nearest {
            SHOCKWAVE_SNAP
        } else {
            0
        }
}

#[cfg(test)]
mod tests {
    // Ported from Chiaro tests/unit/OutputGradeTests.cpp ("neutral settings set no grading flag ..."),
    // tests/unit/CrtTests.cpp ("OutputCrtFlags sets one flag per effect, none for off") and the
    // flag checks of ShockwaveTests.cpp.
    use super::*;

    const GRADING: u32 = GRADE | CONTRAST | VIGNETTE | SCENE_SNAP;
    const CRT_ALL: u32 = CRT_WARP | CRT_SCANLINES | CRT_MASK | CRT_BEAM | CRT_OVERLAY;

    fn with(edit: impl FnOnce(&mut OutputSettings), filter: Filter) -> u32 {
        let mut o = OutputSettings::default();
        edit(&mut o);
        o.filter = filter;
        grade_flags(&o) & GRADING
    }

    #[test]
    fn neutral_settings_set_no_grading_flag() {
        let neutral = OutputSettings::default();
        assert_eq!(grade_flags(&neutral), 0);
        let nearest = OutputSettings {
            filter: Filter::Nearest,
            ..neutral
        };
        assert_eq!(grade_flags(&nearest), 0);
        // Vignette shape and colour alone do nothing while the strength is 0.
        let mut shape_only = nearest;
        shape_only.vignette.radius = 1.0;
        shape_only.vignette.softness = 0.2;
        shape_only.vignette.color = [1.0, 0.0, 0.0];
        assert_eq!(grade_flags(&shape_only), 0);
    }

    #[test]
    fn every_non_neutral_grade_value_sets_its_own_flag() {
        let lin = Filter::Linear;
        assert_eq!(with(|o| o.saturation = 0.0, lin), GRADE);
        assert_eq!(with(|o| o.saturation = 1.5, lin), GRADE);
        assert_eq!(with(|o| o.tint[0] = 0.9, lin), GRADE);
        assert_eq!(with(|o| o.tint[1] = 1.1, lin), GRADE);
        assert_eq!(with(|o| o.tint[2] = 0.5, lin), GRADE);
        assert_eq!(with(|o| o.contrast = 1.2, lin), CONTRAST);
        assert_eq!(with(|o| o.contrast = 0.0, Filter::Nearest), CONTRAST);
        assert_eq!(with(|o| o.vignette.strength = 0.3, lin), VIGNETTE);
        assert_eq!(
            with(|o| o.vignette.strength = 0.3, Filter::Nearest),
            VIGNETTE | SCENE_SNAP
        );
        assert_eq!(
            with(
                |o| {
                    o.saturation = 0.5;
                    o.contrast = 1.5;
                    o.vignette.strength = 1.0;
                },
                Filter::Nearest
            ),
            GRADING
        );
        // No collision with the sRGB, overlay and bloom bits below 32 that the stage sets elsewhere.
        assert_eq!(GRADING & 31, 0);
    }

    #[test]
    fn crt_off_sets_no_flag_whatever_the_rest() {
        let off = CrtSettings::default();
        assert_eq!(crt_flags(&off, 1.0, true), 0);
        let c = CrtSettings {
            overlay: true,
            scanline_period: 4.0,
            mask_pitch: 5.0,
            ..off
        };
        assert_eq!(crt_flags(&c, 1.0, true), 0);
    }

    #[test]
    fn crt_sets_one_flag_per_effect() {
        let off = CrtSettings::default();
        let c = CrtSettings {
            curvature: 0.2,
            ..off
        };
        assert_eq!(crt_flags(&c, 1.0, true), CRT_WARP);
        let c = CrtSettings {
            corner_radius: 0.1,
            ..off
        };
        assert_eq!(crt_flags(&c, 1.0, false), CRT_WARP);
        let c = CrtSettings { overlay: true, ..c };
        assert_eq!(crt_flags(&c, 1.0, true), CRT_WARP | CRT_OVERLAY);
        // No overlay in the frame.
        assert_eq!(crt_flags(&c, 1.0, false), CRT_WARP);

        let c = CrtSettings {
            scanlines: 0.5,
            ..off
        };
        assert_eq!(crt_flags(&c, 1.0, true), CRT_SCANLINES | CRT_BEAM);
        // Faded out at 1:1.
        assert_eq!(crt_flags(&c, 0.0, true), 0);

        let c = CrtSettings { mask: 0.3, ..off };
        assert_eq!(crt_flags(&c, 0.0, true), CRT_MASK | CRT_BEAM);

        let c = CrtSettings {
            brightness: 1.2,
            ..off
        };
        assert_eq!(crt_flags(&c, 1.0, true), CRT_BEAM);
        // The overlay flag needs a bend.
        let c = CrtSettings { overlay: true, ..c };
        assert_eq!(crt_flags(&c, 1.0, true), CRT_BEAM);
    }

    #[test]
    fn shockwave_flags_follow_count_and_filter() {
        assert_eq!(shockwave_flags(0, Filter::Nearest), 0);
        assert_eq!(shockwave_flags(1, Filter::Linear), SHOCKWAVE);
        assert_eq!(
            shockwave_flags(3, Filter::Nearest),
            SHOCKWAVE | SHOCKWAVE_SNAP
        );
    }

    #[test]
    fn flag_groups_do_not_collide() {
        let all = [
            ENCODE_SRGB,
            HAS_OVERLAY,
            OVERLAY_LINEAR,
            BLOOM,
            BLOOM_SNAP,
            GRADE,
            CONTRAST,
            VIGNETTE,
            SCENE_SNAP,
            DITHER,
            CRT_WARP,
            CRT_SCANLINES,
            CRT_MASK,
            CRT_BEAM,
            CRT_OVERLAY,
            SHOCKWAVE,
            SHOCKWAVE_SNAP,
        ];
        let mut seen = 0u32;
        for f in all {
            assert_eq!(f.count_ones(), 1, "{f} is not a single bit");
            assert_eq!(seen & f, 0, "{f} collides");
            seen |= f;
        }
        assert_eq!(CRT_ALL & (GRADING | DITHER | 31), 0);
        // The bit values are the contract with the compiled shader.
        assert_eq!(seen, 0x1_FFFF);
    }
}
