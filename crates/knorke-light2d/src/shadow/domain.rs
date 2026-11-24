// Origin: chiaro @ 9bc5b9b, src/shadows/OcclusionDomain.hpp and OcclusionDomain.cpp
// (MakeOcclusionDomain, SceneToDomain, ComputeMaskAnchor).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The occlusion mask's texel space: domain and anchor.

use crate::Projection;
use crate::light_math::scaled_buffer_size;
use knorke_math::Vec2;

/// Area covered by the occlusion mask: the scene rectangle plus a margin band on every side, at a
/// reduced resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcclusionDomain {
    /// Scene width in pixels.
    pub scene_w: u32,
    /// Scene height in pixels.
    pub scene_h: u32,
    /// Band width in scene pixels, `ceil(max(w, h) / 4)`.
    pub margin: u32,
    /// Mask width in texels.
    pub width: u32,
    /// Mask height in texels.
    pub height: u32,
    /// Mask texels per scene pixel, horizontally and vertically.
    pub texels_per_scene: [f32; 2],
}

impl OcclusionDomain {
    /// The domain of a scene at shadow resolution `scale`.
    pub fn new(scene_w: u32, scene_h: u32, scale: f32) -> Self {
        let margin = scene_w.max(scene_h).div_ceil(4);
        let (full_w, full_h) = (scene_w + 2 * margin, scene_h + 2 * margin);
        let (width, height) = (
            scaled_buffer_size(full_w, scale),
            scaled_buffer_size(full_h, scale),
        );
        Self {
            scene_w,
            scene_h,
            margin,
            width,
            height,
            texels_per_scene: [width as f32 / full_w as f32, height as f32 / full_h as f32],
        }
    }

    /// Scene pixel to mask texel coordinates (continuous; the scene origin lies `margin` scene
    /// pixels in).
    pub fn scene_to_domain(&self, scene: Vec2) -> Vec2 {
        let m = self.margin as f32;
        Vec2::new(
            (scene.x + m) * self.texels_per_scene[0],
            (scene.y + m) * self.texels_per_scene[1],
        )
    }
}

/// The projection origin in mask texels, split into whole texels and a rest:
/// `o = origin * texels_per_scene` (per axis, in double), `snapped = floor(o)`,
/// `remainder = o - snapped` in `[0, 1)`. Every writer and reader of the mask uses
/// `mask_texel = (scene + margin) * texels_per_scene - remainder`, so the mask lies on a grid
/// anchored to whole world texels: while `snapped` stays the same the contents stay, and a change
/// of `snapped` shifts them by whole texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskAnchor {
    /// Whole texels of the origin.
    pub snapped: [i32; 2],
    /// The rest in `[0, 1)`.
    pub remainder: [f32; 2],
}

impl MaskAnchor {
    /// `o` is clamped to ±2^30 texels so that `snapped` and differences of it fit into `i32`.
    pub fn compute(p: &Projection, d: &OcclusionDomain) -> Self {
        const LIMIT: f64 = 1_073_741_824.0; // 2^30
        let origin = p.origin.raw();
        let mut a = Self {
            snapped: [0; 2],
            remainder: [0.0; 2],
        };
        for (i, o) in [origin.x, origin.y].into_iter().enumerate() {
            let o = (f64::from(o) * f64::from(d.texels_per_scene[i])).clamp(-LIMIT, LIMIT);
            let snapped = o.floor();
            let mut remainder = (o - snapped) as f32;
            a.snapped[i] = snapped as i32;
            if remainder >= 1.0 {
                // `o - snapped` just below 1 can round up in float.
                a.snapped[i] += 1;
                remainder = 0.0;
            }
            a.remainder[i] = remainder;
        }
        a
    }
}
