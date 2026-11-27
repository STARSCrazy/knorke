// Origin: chiaro @ 9bc5b9b, src/rendering/GpuLight.hpp (GpuLight, GpuMask, GpuDirectional).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The per-light records the light pass reads (P8, P10): plain `#[repr(C)]` data without any GPU
//! dependency, so the culling that builds them runs and is tested headless. The GPU stages
//! upload them as they are.

use knorke_math::uniform_block;

uniform_block! {
    /// Per-light data of the light pass; matches `struct GpuLight` in `light_common.hlsli`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct GpuLight[128] {
        /// xyz world position, w radius.
        pub position_radius: [f32; 4],
        /// rgb colour * intensity * flicker, a falloff.
        pub color_falloff: [f32; 4],
        /// xyz normalized direction (a rect light's normal), w type: 0 point, 1 spot, 2 line,
        /// 3 rect, 4 two-sided rect (plus the soft near field share for point and spot lights).
        pub direction_type: [f32; 4],
        /// x cos(inner), y cos(outer), z quad half extent (ground radius), w 1 / profile atlas
        /// height (profiles only).
        pub spot_cos: [f32; 4],
        /// x source radius R in mask texels (negative for `inside_obstacle`: `-1 - R`), zw ground
        /// position of the light in mask texels (shadow-casting lights only, zero otherwise); y the
        /// light's specular factor (every light, E47).
        pub shadow: [f32; 4],
        /// x first atlas row of the light's profile or -1 without, yzw C0 unit vector.
        pub profile: [f32; 4],
        /// Line and rect lights (zero otherwise): xyz half extent U along the length, w half width
        /// (rect).
        pub area: [f32; 4],
        /// Shadow-casting line and rect lights: xy U and zw V (the rect's half extent across) in
        /// mask texels. Lights of the medium stage: xy the heights `[bottom, top]` of the reach,
        /// zw zero.
        pub area_mask: [f32; 4],
    }
}

uniform_block! {
    /// The mask of a masked point or spot light (E115); matches `struct GpuMask` in
    /// `light_mask.hlsli`. Kept in a second buffer parallel to the light buffer (same index). The
    /// mask coordinates of a pixel are `u = (w . p) / (w . n)`, `v = (w . q) / (w . n)` for
    /// `w = pixel - light`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct GpuMask[48] {
        /// xyz plane vector n' (projector: the direction; window: normal / distance), w 1 window, 0
        /// projector.
        pub n: [f32; 4],
        /// xyz P' (u), w the block's first atlas v (`block / blocks`).
        pub p: [f32; 4],
        /// xyz Q' (v), w the block's atlas v extent (`1 / blocks`).
        pub q: [f32; 4],
    }
}

uniform_block! {
    /// A directional light (E24); the first three rows of `SunParams` in `lightrow_sun.hlsli`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct GpuDirectional[48] {
        /// xy unit direction toward the light in mask texels, z height per texel, w penumbra per
        /// texel.
        pub dir_slope: [f32; 4],
        /// rgb colour * intensity * flicker, w 1 if it casts shadows.
        pub color: [f32; 4],
        /// xyz unit vector toward the light in world space, w specular factor (E47).
        pub world: [f32; 4],
    }
}

const _: () = assert!(size_of::<GpuLight>() == 128);
const _: () = assert!(size_of::<GpuMask>() == 48);
const _: () = assert!(size_of::<GpuDirectional>() == 48);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    /// Field order and offsets of `struct GpuLight` / `GpuMask` in `GpuLight.hpp`.
    #[test]
    fn layout_matches_chiaro() {
        assert_eq!(offset_of!(GpuLight, position_radius), 0);
        assert_eq!(offset_of!(GpuLight, color_falloff), 16);
        assert_eq!(offset_of!(GpuLight, direction_type), 32);
        assert_eq!(offset_of!(GpuLight, spot_cos), 48);
        assert_eq!(offset_of!(GpuLight, shadow), 64);
        assert_eq!(offset_of!(GpuLight, profile), 80);
        assert_eq!(offset_of!(GpuLight, area), 96);
        assert_eq!(offset_of!(GpuLight, area_mask), 112);
        assert_eq!(offset_of!(GpuMask, p), 16);
        assert_eq!(offset_of!(GpuMask, q), 32);
        assert_eq!(offset_of!(GpuDirectional, world), 32);
        let l = GpuLight {
            shadow: [1.0, 2.0, 3.0, 4.0],
            area_mask: [0.0, 0.0, 0.0, 9.0],
            ..Default::default()
        };
        let b = l.to_bytes();
        assert_eq!(f32::from_le_bytes(b[68..72].try_into().unwrap()), 2.0);
        assert_eq!(f32::from_le_bytes(b[124..128].try_into().unwrap()), 9.0);
    }
}
