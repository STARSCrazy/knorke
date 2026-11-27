// Origin: chiaro @ 9bc5b9b, src/rendering/LightingStage.cpp (LightSurfacePassParams,
// SurfaceParams, MakeSurfaceParams, DetailModeFor), src/rendering/Frame.hpp (DetailMode),
// src/rendering/SdfShadows.hpp (HeightTolerance) and src/core/Settings.hpp (WetParams).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The uniform blocks the row programs share with the light pass, without a GPU.
//!
//! Every block is a `#[repr(C)]` struct of `f32`/`u32` whose fields follow the `cbuffer` of the
//! shader field for field; [`to_bytes`](SurfaceParams::to_bytes) writes the little-endian bytes
//! the stage pushes. The tests pin size and offset of every field against the member offsets in
//! the compiled SPIR-V (`tests/light_uniform_layouts.rs`).

use knorke_math::Vec3;
use knorke_math::uniform_block;

use crate::light_math::scaled_buffer_size;
use crate::material::{screen_right, view_direction};
use crate::projection::Projection;
use crate::settings::LightSettings;
use crate::shadow::OcclusionDomain;

/// How the light pass uses the material buffer (Chiaro `DetailMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DetailMode {
    /// No material buffer.
    #[default]
    None,
    /// Light buffer at scene size: detail normals per light, highlights into the specular
    /// buffer.
    Exact,
    /// Smaller light buffer: base normals, light direction sums into the direction buffer.
    Direction,
}

/// The material mode of a frame (Chiaro `LightingStage::DetailModeFor`): none without a
/// material buffer, exact with the light buffer at scene size, direction otherwise.
pub fn detail_mode_for(
    has_material: bool,
    scene_w: u32,
    scene_h: u32,
    light_resolution: f32,
) -> DetailMode {
    if !has_material {
        return DetailMode::None;
    }
    if scaled_buffer_size(scene_w, light_resolution) == scene_w
        && scaled_buffer_size(scene_h, light_resolution) == scene_h
    {
        DetailMode::Exact
    } else {
        DetailMode::Direction
    }
}

/// Height tolerance below an obstacle top (Chiaro `HeightTolerance`): 1 % of the surface
/// height step.
pub fn height_tolerance(settings: &LightSettings) -> f32 {
    0.01 * settings.surface_height_step
}

uniform_block! {
    /// `cbuffer LightSurfacePassParams` of `lightrow.vert` (64 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct LightSurfacePassParams[64] {
        /// xy projection origin, zw x axis.
        pub origin_x_axis: [f32; 4],
        /// xy y axis, zw scene size in pixels.
        pub y_axis_scene_size: [f32; 4],
        /// xy z axis, z highest height a lit pixel can have, w 0.
        pub z_axis_max_height: [f32; 4],
        /// Index of the draw's first light in the light buffer.
        pub instance_offset: u32,
        /// `lightrow.vert`: the atlas rows of the frame.
        pub mask_base: u32,
        /// Padding.
        pub pad: [u32; 2],
    }
}

impl LightSurfacePassParams {
    /// The block of a frame (`instance_offset` 0).
    pub fn new(
        p: &Projection,
        scene_w: u32,
        scene_h: u32,
        max_height: f32,
        mask_base: u32,
    ) -> Self {
        let o = p.origin.raw();
        Self {
            origin_x_axis: [o.x, o.y, p.x_axis.x, p.x_axis.y],
            y_axis_scene_size: [p.y_axis.x, p.y_axis.y, scene_w as f32, scene_h as f32],
            z_axis_max_height: [p.z_axis.x, p.z_axis.y, max_height, 0.0],
            instance_offset: 0,
            mask_base,
            pad: [0; 2],
        }
    }
}

uniform_block! {
    /// `CHI_SURFACE_PARAMS` of `surface.hlsli` (160 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SurfaceParams[160] {
        /// Rows of the inverse of `[x_axis y_axis]`.
        pub inv_axes: [f32; 4],
        /// xy origin, zw z axis.
        pub origin_z_axis: [f32; 4],
        /// xy scene size, z surface height step, w 1 = surface buffer.
        pub scene_step: [f32; 4],
        /// xy mask texels per scene pixel, z margin.
        pub mask_map: [f32; 4],
        /// xy mask anchor remainder.
        pub mask_remainder: [f32; 4],
        /// xy x axis, zw y axis.
        pub xy_axes: [f32; 4],
        /// xyz view direction.
        pub view_dir: [f32; 4],
        /// xyz screen right.
        pub screen_right: [f32; 4],
        /// xy z axis, z specular scale.
        pub z_axis_spec: [f32; 4],
        /// Wet ground: x stretch, y breakup, z 1 / ripple size.
        pub wet: [f32; 4],
    }
}

impl SurfaceParams {
    /// Chiaro `MakeSurfaceParams`. `domain` is zero without shadows; `surface_buffer` tells whether a surface buffer is given (otherwise every
    /// pixel is ground).
    pub fn new(
        p: &Projection,
        scene_w: u32,
        scene_h: u32,
        settings: &LightSettings,
        domain: &ShadowDomain,
        surface_buffer: bool,
    ) -> Self {
        let det = f64::from(p.x_axis.x) * f64::from(p.y_axis.y)
            - f64::from(p.x_axis.y) * f64::from(p.y_axis.x);
        let o = p.origin.raw();
        let v = view_direction(p);
        let r = screen_right(p);
        let wet = settings.material.wet_mapping();
        Self {
            inv_axes: [
                (f64::from(p.y_axis.y) / det) as f32,
                (-f64::from(p.y_axis.x) / det) as f32,
                (-f64::from(p.x_axis.y) / det) as f32,
                (f64::from(p.x_axis.x) / det) as f32,
            ],
            origin_z_axis: [o.x, o.y, p.z_axis.x, p.z_axis.y],
            scene_step: [
                scene_w as f32,
                scene_h as f32,
                settings.surface_height_step,
                if surface_buffer { 1.0 } else { 0.0 },
            ],
            mask_map: [
                domain.texels_per_scene[0],
                domain.texels_per_scene[1],
                domain.margin as f32,
                0.0,
            ],
            mask_remainder: [domain.remainder[0], domain.remainder[1], 0.0, 0.0],
            xy_axes: [p.x_axis.x, p.x_axis.y, p.y_axis.x, p.y_axis.y],
            view_dir: [v.x, v.y, v.z, 0.0],
            screen_right: [r.x, r.y, r.z, 0.0],
            z_axis_spec: [
                p.z_axis.x,
                p.z_axis.y,
                settings.material.specular_scale,
                0.0,
            ],
            wet: [
                wet.stretch,
                wet.breakup,
                1.0 / settings.wet_ripple_size(p),
                0.0,
            ],
        }
    }
}

/// The occlusion domain as the light pass reads it: zero without shadows (Chiaro
/// `ShadowsOrNone`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadowDomain {
    /// Band width in scene pixels.
    pub margin: u32,
    /// Mask width in texels.
    pub width: u32,
    /// Mask height in texels.
    pub height: u32,
    /// Mask texels per scene pixel.
    pub texels_per_scene: [f32; 2],
    /// Mask anchor remainder.
    pub remainder: [f32; 2],
}

impl ShadowDomain {
    /// The domain of the occlusion mask with the anchor remainder of the frame.
    pub fn new(d: &OcclusionDomain, remainder: [f32; 2]) -> Self {
        Self {
            margin: d.margin,
            width: d.width,
            height: d.height,
            texels_per_scene: d.texels_per_scene,
            remainder,
        }
    }
}

/// The light buffer clear colour: ambient, alpha 0.
pub fn ambient_clear(ambient: Vec3) -> [f32; 4] {
    [ambient.x, ambient.y, ambient.z, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn sizes_and_offsets_follow_hlsl_packing() {
        assert_eq!(size_of::<LightSurfacePassParams>(), 64);
        assert_eq!(size_of::<SurfaceParams>(), 160);
        assert_eq!(offset_of!(LightSurfacePassParams, z_axis_max_height), 32);
        assert_eq!(offset_of!(LightSurfacePassParams, instance_offset), 48);
        assert_eq!(offset_of!(LightSurfacePassParams, mask_base), 52);
        assert_eq!(offset_of!(SurfaceParams, wet), 144);
    }

    /// `to_bytes` writes every field at its `repr(C)` offset.
    #[test]
    fn bytes_follow_the_struct_layout() {
        let s = LightSurfacePassParams {
            instance_offset: 3,
            mask_base: 7,
            z_axis_max_height: [0.0, 0.5, 9.0, 0.0],
            ..Default::default()
        }
        .to_bytes();
        assert_eq!(s[offset_of!(LightSurfacePassParams, instance_offset)], 3);
        assert_eq!(s[offset_of!(LightSurfacePassParams, mask_base)], 7);
        let z = offset_of!(LightSurfacePassParams, z_axis_max_height) + 8;
        assert_eq!(f32::from_le_bytes(s[z..z + 4].try_into().unwrap()), 9.0);
    }

    #[test]
    fn detail_mode_follows_the_light_resolution() {
        assert_eq!(detail_mode_for(false, 64, 64, 1.0), DetailMode::None);
        assert_eq!(detail_mode_for(true, 64, 64, 1.0), DetailMode::Exact);
        assert_eq!(detail_mode_for(true, 64, 64, 0.5), DetailMode::Direction);
        // 0.999 still rounds up to the full size.
        assert_eq!(detail_mode_for(true, 64, 48, 0.9999), DetailMode::Exact);
    }
}
