// Origin: chiaro @ 9bc5b9b, src/rendering/LightingStage.cpp (LightPassParams, DirectionalParams,
// LightSurfacePassParams, SurfaceParams, SurfaceShadowParams, MakeSurfaceParams, ProgramFor,
// DetailModeFor), src/rendering/Frame.hpp (DetailMode), src/rendering/SdfShadows.hpp
// (HeightTolerance) and src/core/Settings.hpp (WetParams).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The uniform blocks of the light pass and the program choice, without a GPU.
//!
//! Every block is a `#[repr(C)]` struct of `f32`/`u32` whose fields follow the `cbuffer` of the
//! shader field for field; [`to_bytes`](LightPassParams::to_bytes) writes the little-endian
//! bytes the stage pushes. The tests pin size and offset of every field against Chiaro and
//! against the member offsets in the compiled SPIR-V (`tests/light_uniform_layouts.rs`).

use knorke_math::Vec3;
use knorke_math::uniform_block;

use crate::gpu_light::GpuDirectional;
use crate::light_math::scaled_buffer_size;
use crate::light_program::{LightBase, LightFeature as F, LightProgram};
use crate::material::{screen_right, view_direction};
use crate::occlusion::FIELD_BOUND;
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

/// The kinds of draw in the light pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightPass {
    /// Point/spot (and, without shadows, line/rect) lights without distance field shadows.
    Unshadowed,
    /// Point/spot lights with distance field shadows.
    Shadowed,
    /// Line/rect lights with distance field shadows (E33).
    ShadowedArea,
    /// Directional lights drawn per pixel.
    Directional,
}

/// What of the frame decides the features of a light program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProgramModes {
    /// Material mode of the frame.
    pub detail: DetailMode,
    /// A surface or material buffer is given.
    pub normal_mode: bool,
    /// Shadows are active and in height mode (E22).
    pub shadow_height_mode: bool,
    /// The frame builds a medium world map (P29).
    pub medium_map: bool,
    /// Shadows are active and pattern obstacles are in view (P28; `false` until then).
    pub patterns: bool,
}

/// The light program of one kind of draw (Chiaro `ProgramFor`, E152): material mode first,
/// then normal mode; height mode only reaches the distance field shadows without surface. The
/// medium world map sets `Medium` on every draw, pattern obstacles set `Pattern` on the shadow
/// programs only.
pub fn program_for(pass: LightPass, masked: bool, m: &ProgramModes) -> LightProgram {
    let exact = m.detail == DetailMode::Exact;
    let direction = m.detail == DetailMode::Direction;
    let surface = exact || direction || m.normal_mode;
    let shadowed = pass != LightPass::Unshadowed;
    let base = match pass {
        LightPass::Directional => LightBase::Directional,
        _ if shadowed => LightBase::LightSdf,
        _ => LightBase::Light,
    };
    LightProgram::new(base)
        .with(F::Surface, surface)
        .with(F::Exact, exact)
        .with(F::Direction, direction)
        .with(
            F::Height,
            base == LightBase::LightSdf && m.shadow_height_mode && !surface,
        )
        .with(F::Area, pass == LightPass::ShadowedArea)
        .with(F::Mask, masked)
        .with(F::Medium, m.medium_map)
        .with(F::Pattern, shadowed && m.patterns)
}

/// Height tolerance below an obstacle top (Chiaro `HeightTolerance`): 1 % of the surface
/// height step.
pub fn height_tolerance(settings: &LightSettings) -> f32 {
    0.01 * settings.surface_height_step
}

uniform_block! {
    /// `cbuffer LightPassParams` of `light.vert` and `light_point.vert` (64 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct LightPassParams[64] {
        /// xy projection origin, zw x axis (scene pixels).
        pub origin_x_axis: [f32; 4],
        /// xy y axis, zw scene size in pixels.
        pub y_axis_scene_size: [f32; 4],
        /// Occlusion mask texels per scene pixel.
        pub mask_texel_scale: [f32; 2],
        /// Margin band of the mask in scene pixels.
        pub mask_margin: f32,
        /// Index of the draw's first light in the light buffer.
        pub instance_offset: u32,
        /// Mask anchor remainder, subtracted from the mask position.
        pub mask_remainder: [f32; 2],
        /// `_mask` programs: light buffer index of the first masked light.
        pub mask_base: u32,
        /// Padding.
        pub pad: f32,
    }
}

impl LightPassParams {
    /// The block of a frame (`instance_offset` 0); `domain`/`remainder` zero without shadows.
    pub fn new(
        p: &Projection,
        scene_w: u32,
        scene_h: u32,
        domain: &ShadowDomain,
        mask_base: u32,
    ) -> Self {
        let o = p.origin.raw();
        Self {
            origin_x_axis: [o.x, o.y, p.x_axis.x, p.x_axis.y],
            y_axis_scene_size: [p.y_axis.x, p.y_axis.y, scene_w as f32, scene_h as f32],
            mask_texel_scale: domain.texels_per_scene,
            mask_margin: domain.margin as f32,
            instance_offset: 0,
            mask_remainder: domain.remainder,
            mask_base,
            pad: 0.0,
        }
    }
}

uniform_block! {
    /// `cbuffer LightSurfacePassParams` of `light_surface.vert` (64 bytes).
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
        /// `_mask` programs: light buffer index of the first masked light.
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
    /// Chiaro `MakeSurfaceParams`. `domain` is zero without shadows (only the shadow programs
    /// read it); `surface_buffer` tells whether a surface buffer is given (otherwise every
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

uniform_block! {
    /// `cbuffer ShadowParams` of `light_sdf_surface.frag` and `light_sdf_height.frag` (32 bytes);
    /// `light_sdf.frag` declares the first row only, which is pushed alone then
    /// ([`FLAT_SIZE`](Self::FLAT_SIZE)).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SurfaceShadowParams[32] {
        /// xy mask size in texels, z field bound, w 1 if an obstacle is partly transparent.
        pub domain_size: [f32; 4],
        /// x height above which nothing occludes, y tolerance below a top.
        pub height_params: [f32; 4],
    }
}

impl SurfaceShadowParams {
    /// Size of the flat program's block (first row only).
    pub const FLAT_SIZE: usize = 16;

    /// The block of a frame.
    pub fn new(domain: &ShadowDomain, any_partial: bool, shadow_top: f32, tolerance: f32) -> Self {
        Self {
            domain_size: [
                domain.width as f32,
                domain.height as f32,
                FIELD_BOUND as f32,
                if any_partial { 1.0 } else { 0.0 },
            ],
            height_params: [shadow_top, tolerance, 0.0, 0.0],
        }
    }
}

uniform_block! {
    /// `cbuffer DirectionalParams` of `directional*.frag` (112 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct DirectionalParams[112] {
        /// The light (first three rows).
        pub light: GpuDirectional,
        /// xy scene size, z mask margin in scene pixels.
        pub scene_size_margin: [f32; 4],
        /// xy mask texels per scene pixel, zw mask anchor remainder.
        pub texel_scale_remainder: [f32; 4],
        /// xy mask size, z field bound, w 1 if an obstacle is partly transparent.
        pub domain_size: [f32; 4],
        /// x highest finite top, y tolerance below a top, z 1 (infinite tops do not block).
        pub height_params: [f32; 4],
    }
}

impl DirectionalParams {
    /// The frame part of the block (the light is set per draw); `domain` zero without shadows.
    pub fn new(
        scene_w: u32,
        scene_h: u32,
        domain: &ShadowDomain,
        any_partial: bool,
        scene_max_top: f32,
        tolerance: f32,
    ) -> Self {
        Self {
            light: GpuDirectional::default(),
            scene_size_margin: [scene_w as f32, scene_h as f32, domain.margin as f32, 0.0],
            texel_scale_remainder: [
                domain.texels_per_scene[0],
                domain.texels_per_scene[1],
                domain.remainder[0],
                domain.remainder[1],
            ],
            domain_size: [
                domain.width as f32,
                domain.height as f32,
                FIELD_BOUND as f32,
                if any_partial { 1.0 } else { 0.0 },
            ],
            height_params: [scene_max_top, tolerance, 1.0, 0.0],
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
    fn sizes_match_chiaro_static_asserts() {
        assert_eq!(size_of::<LightPassParams>(), 64);
        assert_eq!(size_of::<LightSurfacePassParams>(), 64);
        assert_eq!(size_of::<SurfaceParams>(), 160);
        assert_eq!(size_of::<SurfaceShadowParams>(), 32);
        assert_eq!(size_of::<DirectionalParams>(), 112);
        assert_eq!(size_of::<GpuDirectional>(), 48);
    }

    #[test]
    fn light_pass_params_offsets_follow_hlsl_packing() {
        assert_eq!(offset_of!(LightPassParams, origin_x_axis), 0);
        assert_eq!(offset_of!(LightPassParams, y_axis_scene_size), 16);
        assert_eq!(offset_of!(LightPassParams, mask_texel_scale), 32);
        assert_eq!(offset_of!(LightPassParams, mask_margin), 40);
        assert_eq!(offset_of!(LightPassParams, instance_offset), 44);
        assert_eq!(offset_of!(LightPassParams, mask_remainder), 48);
        assert_eq!(offset_of!(LightPassParams, mask_base), 56);
        assert_eq!(offset_of!(LightSurfacePassParams, z_axis_max_height), 32);
        assert_eq!(offset_of!(LightSurfacePassParams, instance_offset), 48);
        assert_eq!(offset_of!(LightSurfacePassParams, mask_base), 52);
        assert_eq!(offset_of!(DirectionalParams, scene_size_margin), 48);
        assert_eq!(offset_of!(DirectionalParams, height_params), 96);
        assert_eq!(offset_of!(SurfaceParams, wet), 144);
    }

    /// `to_bytes` writes every field at its `repr(C)` offset.
    #[test]
    fn bytes_follow_the_struct_layout() {
        let p = LightPassParams {
            origin_x_axis: [1.0, 2.0, 3.0, 4.0],
            y_axis_scene_size: [5.0, 6.0, 7.0, 8.0],
            mask_texel_scale: [9.0, 10.0],
            mask_margin: 11.0,
            instance_offset: 12,
            mask_remainder: [13.0, 14.0],
            mask_base: 15,
            pad: 0.0,
        };
        let b = p.to_bytes();
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!(f(offset_of!(LightPassParams, mask_margin)), 11.0);
        assert_eq!(u(offset_of!(LightPassParams, instance_offset)), 12);
        assert_eq!(f(offset_of!(LightPassParams, mask_remainder) + 4), 14.0);
        assert_eq!(u(offset_of!(LightPassParams, mask_base)), 15);

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

        let mut d = DirectionalParams::default();
        d.light.world = [0.0, 0.0, 0.0, 2.5];
        d.height_params = [0.0, 0.0, 1.0, 0.0];
        let b = d.to_bytes();
        assert_eq!(f32::from_le_bytes(b[44..48].try_into().unwrap()), 2.5);
        assert_eq!(f32::from_le_bytes(b[104..108].try_into().unwrap()), 1.0);
    }

    #[test]
    fn detail_mode_follows_the_light_resolution() {
        assert_eq!(detail_mode_for(false, 64, 64, 1.0), DetailMode::None);
        assert_eq!(detail_mode_for(true, 64, 64, 1.0), DetailMode::Exact);
        assert_eq!(detail_mode_for(true, 64, 64, 0.5), DetailMode::Direction);
        // 0.999 still rounds up to the full size.
        assert_eq!(detail_mode_for(true, 64, 48, 0.9999), DetailMode::Exact);
    }

    #[test]
    fn program_choice_follows_chiaro() {
        let flat = ProgramModes::default();
        assert_eq!(
            program_for(LightPass::Unshadowed, false, &flat),
            LightProgram::new(LightBase::Light)
        );
        assert_eq!(
            program_for(LightPass::ShadowedArea, false, &flat),
            LightProgram::new(LightBase::LightSdf).with(F::Area, true)
        );
        let height = ProgramModes {
            shadow_height_mode: true,
            ..flat
        };
        assert_eq!(
            program_for(LightPass::Shadowed, false, &height),
            LightProgram::new(LightBase::LightSdf).with(F::Height, true)
        );
        // Height mode never reaches the unshadowed or directional programs.
        assert_eq!(
            program_for(LightPass::Directional, false, &height),
            LightProgram::new(LightBase::Directional)
        );
        let exact = ProgramModes {
            detail: DetailMode::Exact,
            normal_mode: true,
            shadow_height_mode: true,
            ..flat
        };
        assert_eq!(
            program_for(LightPass::Shadowed, true, &exact),
            LightProgram::new(LightBase::LightSdf)
                .with(F::Surface, true)
                .with(F::Exact, true)
                .with(F::Mask, true)
        );
        let patterns = ProgramModes {
            patterns: true,
            medium_map: true,
            ..flat
        };
        assert!(!program_for(LightPass::Unshadowed, false, &patterns).has(F::Pattern));
        assert!(program_for(LightPass::Unshadowed, false, &patterns).has(F::Medium));
        assert!(program_for(LightPass::Directional, false, &patterns).has(F::Pattern));
    }

    /// Every program the stage can choose without medium and patterns has shaders.
    #[test]
    fn every_reachable_program_exists() {
        use crate::light_program::light_program_index;
        for detail in [DetailMode::None, DetailMode::Exact, DetailMode::Direction] {
            for normal_mode in [false, true] {
                if detail != DetailMode::None && !normal_mode {
                    continue;
                }
                for shadow_height_mode in [false, true] {
                    let m = ProgramModes {
                        detail,
                        normal_mode,
                        shadow_height_mode,
                        ..Default::default()
                    };
                    for pass in [
                        LightPass::Unshadowed,
                        LightPass::Shadowed,
                        LightPass::ShadowedArea,
                        LightPass::Directional,
                    ] {
                        let masks: &[bool] = match pass {
                            LightPass::Unshadowed | LightPass::Shadowed => &[false, true],
                            _ => &[false],
                        };
                        for &masked in masks {
                            let p = program_for(pass, masked, &m);
                            assert!(light_program_index(p).is_some(), "{p:?} has no shaders");
                        }
                    }
                }
            }
        }
    }
}
