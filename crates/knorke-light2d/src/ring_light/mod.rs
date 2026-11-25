// Origin: chiaro @ 9bc5b9b, src/rendering/ShockwaveLightPass.hpp, src/rendering/ShockwaveLightPass.cpp
// (RingPassParams, RingFragParams), src/core/Shockwave.hpp (GpuRingLight), shaders/src/shockwave_light.vert.hlsl,
// shaders/src/shockwave_light.frag.hlsl.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The ring light of shockwaves (P21, Chiaro `ShockwaveLightPass`, E143): one render pass after
//! the light pass that adds a flat torus of light around each wave into the light buffer, in
//! one instanced draw (`ONE + ONE`). The waves come as uniforms, so the pass owns no GPU buffer:
//! a frame without ring light records, binds and allocates nothing.
//!
//! | Module | Content |
//! |---|---|
//! | this one | [`RingLight`] (the input) and the uniform blocks, without a GPU |
//! | `gpu` (feature `gpu`) | `RingLightPass`, owned by the [`LightingStage`](crate::lighting::LightingStage) |
//!
//! # Redesign against the C++ version
//!
//! - The wave list, its validation and the culling (which waves light, their scene rectangle)
//!   live in `knorke-post` (`shockwave::collect_shockwaves`), which this crate must not depend
//!   on (ADR 0014). The pass takes the culled ring lights as its own plain [`RingLight`]
//!   values (`LightingFrame::ring_lights`, `PassFrame::ring_lights`); `knorke-integration`
//!   converts them.

use crate::projection::Projection;
use knorke_math::uniform_block;

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{RING_LIGHTS_TOO_MANY, RingLightPass};

/// Most ring lights per frame (Chiaro `CHI_MAX_SHOCKWAVES`; the size of the uniform arrays).
pub const MAX_RING_LIGHTS: usize = 16;

/// One ring light of a frame, culled by the host (Chiaro `GpuRingLight`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RingLight {
    /// `x0, y0, x1, y1` of the quad in scene pixels (the projected torus plus one pixel).
    pub scene_rect: [f32; 4],
    /// Centre `x, y` (world), radius, `2 / width`.
    pub ring: [f32; 4],
    /// `rgb` light times light intensity (linear), `a` 0.
    pub color: [f32; 4],
}

uniform_block! {
    /// `cbuffer RingPassParams` of `shockwave_light.vert` (16 + 48 × 16 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RingPassParams[16 + 48 * MAX_RING_LIGHTS] {
        /// Scene size in pixels.
        pub scene_size: [f32; 2],
        /// Padding.
        pub pad: [f32; 2],
        /// [`RingLight::scene_rect`] per instance.
        pub rects: [[f32; 4]; MAX_RING_LIGHTS],
        /// [`RingLight::ring`] per instance.
        pub rings: [[f32; 4]; MAX_RING_LIGHTS],
        /// [`RingLight::color`] per instance.
        pub colors: [[f32; 4]; MAX_RING_LIGHTS],
    }
}

impl RingPassParams {
    /// The block of a scene of `w` x `h` with the first [`MAX_RING_LIGHTS`] of `rings`, one
    /// instance each, in order.
    pub fn new(w: u32, h: u32, rings: &[RingLight]) -> Self {
        let mut p = Self {
            scene_size: [w as f32, h as f32],
            ..Self::default()
        };
        for (i, r) in rings.iter().take(MAX_RING_LIGHTS).enumerate() {
            p.rects[i] = r.scene_rect;
            p.rings[i] = r.ring;
            p.colors[i] = r.color;
        }
        p
    }
}

uniform_block! {
    /// `cbuffer RingFragParams` of `shockwave_light.frag` (48 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RingFragParams[48] {
        /// Rows of the inverse of `[x_axis y_axis]`.
        pub inv_axes: [f32; 4],
        /// xy projection origin, zw z axis (scene pixels per unit of height).
        pub origin_z_axis: [f32; 4],
        /// xy scene size in pixels, z surface height step, w 1 = surface buffer present.
        pub scene_step: [f32; 4],
    }
}

impl RingFragParams {
    /// The block of a frame: the first three rows of the zone pass's block, which finds the
    /// world points the same way (Chiaro uses one `GroundInverse` for both).
    pub fn new(
        p: &Projection,
        scene_w: u32,
        scene_h: u32,
        height_step: f32,
        surface_buffer: bool,
    ) -> Self {
        let z = crate::zone_pass::ZoneFragParams::new(
            p,
            scene_w,
            scene_h,
            height_step,
            surface_buffer,
            0,
        );
        Self {
            inv_axes: z.inv_axes,
            origin_z_axis: z.origin_z_axis,
            scene_step: z.scene_step,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_math::{At, Vec2};

    fn ring(i: f32) -> RingLight {
        RingLight {
            scene_rect: [i, i + 1.0, i + 2.0, i + 3.0],
            ring: [i + 4.0, i + 5.0, i + 6.0, i + 7.0],
            color: [i + 8.0, i + 9.0, i + 10.0, 0.0],
        }
    }

    #[test]
    fn the_pass_block_is_chiaros_784_bytes_with_one_array_per_field() {
        let p = RingPassParams::new(96, 48, &[ring(0.0), ring(100.0)]);
        let b = p.to_bytes();
        assert_eq!(b.len(), 784);
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!((f(0), f(4), f(8), f(12)), (96.0, 48.0, 0.0, 0.0));
        // rects at 16, rings at 16 + 256, colors at 16 + 512; instance 1 one float4 further.
        assert_eq!(f(16), 0.0);
        assert_eq!(f(16 + 16 + 12), 103.0);
        assert_eq!(f(16 + 256), 4.0);
        assert_eq!(f(16 + 256 + 16), 104.0);
        assert_eq!(f(16 + 512 + 8), 10.0);
        assert_eq!(f(16 + 512 + 16), 108.0);
        // Unused instances stay zero.
        assert!(b[16 + 32..16 + 256].iter().all(|&x| x == 0));
    }

    #[test]
    fn the_pass_block_takes_at_most_sixteen_rings_in_order() {
        let many: Vec<RingLight> = (0..20).map(|i| ring(i as f32)).collect();
        let p = RingPassParams::new(8, 8, &many);
        assert_eq!(p.rects[15], many[15].scene_rect);
        assert_eq!(p.colors[0], many[0].color);
    }

    #[test]
    fn the_fragment_block_inverts_the_projection_like_the_zones() {
        let p = Projection {
            origin: At::new(Vec2::new(64.0, 20.0)),
            x_axis: Vec2::new(2.0, 1.0),
            y_axis: Vec2::new(-2.0, 1.0),
            z_axis: Vec2::new(0.0, -2.0),
        };
        let f = RingFragParams::new(&p, 128, 96, 0.5, false);
        // det = 2 * 1 - 1 * -2 = 4.
        assert_eq!(f.inv_axes, [0.25, 0.5, -0.25, 0.5]);
        assert_eq!(f.origin_z_axis, [64.0, 20.0, 0.0, -2.0]);
        assert_eq!(f.scene_step, [128.0, 96.0, 0.5, 0.0]);
        assert_eq!(f.to_bytes().len(), 48);
        assert_eq!(
            RingFragParams::new(&p, 128, 96, 0.5, true).scene_step[3],
            1.0
        );
    }
}
