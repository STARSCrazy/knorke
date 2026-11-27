// Origin: chiaro @ 9bc5b9b, src/gpu/CompositeLayout.hpp (CompositeProgramOf, CompositeLayoutOf),
// src/rendering/CompositeStage.cpp (CompositeParams, parameter block of Record) and
// src/core/Settings.hpp (WetParams).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The programs, binding layouts and uniform block of the composite pass, without a GPU.
//!
//! [`CompositeParams`] is a `#[repr(C)]` struct of `f32`/`u32` that follows `cbuffer
//! CompositeParams` of `composite.frag.hlsl` field for field; [`to_bytes`](CompositeParams::to_bytes)
//! writes the little-endian bytes the stage pushes. The tests pin size and offset of every field
//! against Chiaro and against the member offsets in the compiled SPIR-V
//! (`tests/composite_uniform_layouts.rs`).

use knorke_math::Vec3;
use knorke_math::uniform_block;

use crate::binding_layout::{LayoutBuilder, LightLayout, LightSampler as S, LightUniform as U};
use crate::material::{screen_right, view_direction};
use crate::projection::Projection;
use crate::settings::{GlowColor, LightSettings};

/// Encoding of the scene (and glow) texture (Chiaro `CHI_ColorSpace`, `Settings::InputColorSpace`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InputColorSpace {
    /// Values are sRGB-encoded (typical for 8-bit textures); the composite decodes them unless
    /// the texture format does.
    #[default]
    Srgb,
    /// Values are linear.
    Linear,
}

/// The composite programs (Chiaro `ProgramId::Composite*`): the base one, with the medium stage's
/// scatter buffer (two upsample filters) and with the pure ambient of the ambient zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompositeProgram {
    /// `composite.frag`.
    Base,
    /// `composite_zones.frag`.
    Zones,
    /// `composite_medium.frag`.
    Medium,
    /// `composite_medium_zones.frag`.
    MediumZones,
    /// `composite_medium_nearest.frag`.
    MediumNearest,
    /// `composite_medium_nearest_zones.frag`.
    MediumNearestZones,
}

/// Number of composite programs.
pub const COMPOSITE_PROGRAM_COUNT: usize = 6;

/// All composite programs, in table order (the index is [`CompositeProgram::index`]).
pub const COMPOSITE_PROGRAMS: [CompositeProgram; COMPOSITE_PROGRAM_COUNT] = [
    CompositeProgram::Base,
    CompositeProgram::Zones,
    CompositeProgram::Medium,
    CompositeProgram::MediumZones,
    CompositeProgram::MediumNearest,
    CompositeProgram::MediumNearestZones,
];

impl CompositeProgram {
    /// Position in [`COMPOSITE_PROGRAMS`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Vertex shader name in the shader pack (all programs draw one fullscreen triangle).
    pub const fn vertex(self) -> &'static str {
        "fullscreen.vert"
    }

    /// Fragment shader name in the shader pack.
    pub const fn fragment(self) -> &'static str {
        match self {
            Self::Base => "composite.frag",
            Self::Zones => "composite_zones.frag",
            Self::Medium => "composite_medium.frag",
            Self::MediumZones => "composite_medium_zones.frag",
            Self::MediumNearest => "composite_medium_nearest.frag",
            Self::MediumNearestZones => "composite_medium_nearest_zones.frag",
        }
    }

    /// Whether the program reads the medium stage's scatter buffer.
    pub const fn has_medium(self) -> bool {
        !matches!(self, Self::Base | Self::Zones)
    }

    /// Whether the program reads the pure ambient of the ambient zones.
    pub const fn has_zones(self) -> bool {
        matches!(
            self,
            Self::Zones | Self::MediumZones | Self::MediumNearestZones
        )
    }
}

/// The composite program of a frame (Chiaro `CompositeProgramOf`, E160): the base one, with the
/// medium stage's scatter buffer (`medium`; the upsample filter `nearest` only matters then) and
/// with the pure ambient of the ambient zones (`zones`, direction mode only).
pub const fn composite_program_of(medium: bool, nearest: bool, zones: bool) -> CompositeProgram {
    if !medium {
        return if zones {
            CompositeProgram::Zones
        } else {
            CompositeProgram::Base
        };
    }
    match (nearest, zones) {
        (true, true) => CompositeProgram::MediumNearestZones,
        (true, false) => CompositeProgram::MediumNearest,
        (false, true) => CompositeProgram::MediumZones,
        (false, false) => CompositeProgram::Medium,
    }
}

/// The binding layout of a composite program (Chiaro `CompositeLayoutOf`, E160): the slot of each
/// named resource in the order `composite.frag.hlsl` declares them. The six samplers of the base
/// program come first (Scene, LightBuffer, Glow, Material, Detail, Surface), then the medium ones
/// (ScatterTex, MediumTex, ScatterGuides), then the pure ambient (slot 6 without and slot 9 with
/// the medium); `CompositeParams` is uniform 0, `MediumParams` (the medium programs') uniform 1.
pub const fn composite_layout_of(program: CompositeProgram) -> LightLayout {
    let mut b = LayoutBuilder::new();
    b.sampler(S::Scene);
    b.sampler(S::LightBuffer);
    b.sampler(S::Glow);
    b.sampler(S::Material);
    b.sampler(S::Detail);
    b.sampler(S::Surface);
    if program.has_medium() {
        b.sampler(S::ScatterTex);
        b.sampler(S::MediumTex);
        b.sampler(S::ScatterGuides);
    }
    if program.has_zones() {
        b.sampler(S::PureAmbient);
    }
    b.uniform(U::Composite);
    if program.has_medium() {
        b.uniform(U::MediumParams);
    }
    b.finish()
}

/// What the composite parameter block of a frame depends on (Chiaro `CompositeStage::Record`).
#[derive(Clone, Copy, Debug)]
pub struct CompositeParamsInput<'a> {
    /// Encoding of the scene and glow textures.
    pub input_space: InputColorSpace,
    /// The scene texture is an `*_SRGB` format (the sampler decodes it already).
    pub scene_format_is_srgb: bool,
    /// A light buffer was drawn; `false`: the composite multiplies by `ambient`.
    pub light_buffer: bool,
    /// The frame has a glow buffer: `Some(true)` if its format is `*_SRGB`.
    pub glow: Option<bool>,
    /// Ambient plus the directional lights not drawn per pixel; in direction mode the pure
    /// ambient.
    pub ambient: Vec3,
    /// The composite adds the specular buffer (exact material mode).
    pub use_specular: bool,
    /// Direction material mode: the direction sums are in the detail slot.
    pub direction: bool,
    /// A surface buffer is given (direction mode: base normal from it; otherwise every pixel is
    /// ground).
    pub surface_buffer: bool,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Scene size in pixels.
    pub scene_size: (u32, u32),
    /// Glow, specular and wet ground settings.
    pub settings: &'a LightSettings,
}

uniform_block! {
    /// `cbuffer CompositeParams` of `composite.frag` (160 bytes, Chiaro `CompositeParams`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct CompositeParams[160] {
        /// 1: decode the scene colour from sRGB.
        pub decode_srgb: u32,
        /// 1: light from the light buffer, 0: from `ambient`.
        pub use_light_buffer: u32,
        /// 1: add the glow buffer.
        pub use_glow: u32,
        /// 1: the glow colour is sRGB-encoded 8-bit UNORM.
        pub decode_glow: u32,
        /// rgb: light used without a light buffer (in direction mode the pure ambient), w 1.
        pub ambient: [f32; 4],
        /// 1: the glow colour is the scene pixel (`GlowColor::Scene`).
        pub glow_from_scene: u32,
        /// Glow HDR factor.
        pub glow_scale: f32,
        /// 1: add the specular buffer of the exact material mode.
        pub use_specular: u32,
        /// 1: apply the detail normals with the direction sums (direction material mode).
        pub use_direction: u32,
        /// xyz view direction, w specular scale (direction mode).
        pub view_dir_spec: [f32; 4],
        /// xyz screen right, w 1 = surface buffer in the surface slot (direction mode).
        pub screen_right: [f32; 4],
        /// xy x axis, zw y axis (direction mode).
        pub xy_axes: [f32; 4],
        /// xy z axis, zw scene size (direction mode).
        pub z_axis_scene: [f32; 4],
        /// Rows of the inverse of `[x_axis y_axis]` (direction mode, wet ground).
        pub inv_axes: [f32; 4],
        /// xy projection origin, z surface height step (direction mode, wet ground).
        pub origin_step: [f32; 4],
        /// Wet ground: x stretch (1 off), y breakup, z 1 / ripple size (direction mode).
        pub wet: [f32; 4],
    }
}

impl CompositeParams {
    /// The block of a frame (Chiaro `CompositeStage::Record`). The direction-mode rows stay zero
    /// in the other modes, as in Chiaro.
    pub fn new(i: &CompositeParamsInput<'_>) -> Self {
        let settings = i.settings;
        let srgb_input = i.input_space == InputColorSpace::Srgb;
        let mut p = Self {
            decode_srgb: u32::from(srgb_input && !i.scene_format_is_srgb),
            use_light_buffer: u32::from(i.light_buffer),
            use_glow: u32::from(i.glow.is_some()),
            decode_glow: u32::from(i.glow == Some(false) && srgb_input),
            ambient: [i.ambient.x, i.ambient.y, i.ambient.z, 1.0],
            glow_from_scene: u32::from(settings.glow.color == GlowColor::Scene),
            glow_scale: settings.glow.scale,
            use_specular: u32::from(i.use_specular),
            use_direction: u32::from(i.direction),
            ..Self::default()
        };
        if i.direction {
            let pr = i.projection;
            let v = view_direction(pr);
            let r = screen_right(pr);
            let o = pr.origin.raw();
            p.view_dir_spec = [v.x, v.y, v.z, settings.material.specular_scale];
            p.screen_right = [r.x, r.y, r.z, if i.surface_buffer { 1.0 } else { 0.0 }];
            p.xy_axes = [pr.x_axis.x, pr.x_axis.y, pr.y_axis.x, pr.y_axis.y];
            p.z_axis_scene = [
                pr.z_axis.x,
                pr.z_axis.y,
                i.scene_size.0 as f32,
                i.scene_size.1 as f32,
            ];
            p.inv_axes = pr.inverse_axes().1;
            p.origin_step = [o.x, o.y, settings.surface_height_step, 0.0];
            let wet = settings.material.wet_mapping();
            p.wet = [
                wet.stretch,
                wet.breakup,
                1.0 / settings.wet_ripple_size(pr),
                0.0,
            ];
        }
        p
    }
}

/// The upsample filter of the medium programs (Chiaro `MediumFilter() == CHI_FILTER_NEAREST`).
#[cfg(any(feature = "gpu", test))]
pub(crate) fn medium_nearest(settings: &LightSettings) -> bool {
    settings.quality.medium_filter == crate::settings::Filter::Nearest
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;
    use crate::binding_layout::LightSampler;

    #[test]
    fn size_matches_chiaro_static_assert() {
        assert_eq!(size_of::<CompositeParams>(), 160);
        assert_eq!(CompositeParams::SIZE, 160);
    }

    #[test]
    fn offsets_follow_hlsl_packing() {
        assert_eq!(offset_of!(CompositeParams, decode_srgb), 0);
        assert_eq!(offset_of!(CompositeParams, decode_glow), 12);
        assert_eq!(offset_of!(CompositeParams, ambient), 16);
        assert_eq!(offset_of!(CompositeParams, glow_from_scene), 32);
        assert_eq!(offset_of!(CompositeParams, glow_scale), 36);
        assert_eq!(offset_of!(CompositeParams, use_specular), 40);
        assert_eq!(offset_of!(CompositeParams, use_direction), 44);
        assert_eq!(offset_of!(CompositeParams, view_dir_spec), 48);
        assert_eq!(offset_of!(CompositeParams, screen_right), 64);
        assert_eq!(offset_of!(CompositeParams, xy_axes), 80);
        assert_eq!(offset_of!(CompositeParams, z_axis_scene), 96);
        assert_eq!(offset_of!(CompositeParams, inv_axes), 112);
        assert_eq!(offset_of!(CompositeParams, origin_step), 128);
        assert_eq!(offset_of!(CompositeParams, wet), 144);
    }

    /// `LightProgramLayoutTests.cpp`: the counts of the six variants.
    #[test]
    fn layouts_have_chiaros_counts() {
        // medium, nearest, zones, samplers, uniforms
        let variants = [
            (false, false, false, 6, 1),
            (false, false, true, 7, 1),
            (true, false, false, 9, 2),
            (true, false, true, 10, 2),
            (true, true, false, 9, 2),
            (true, true, true, 10, 2),
        ];
        for (medium, nearest, zones, samplers, uniforms) in variants {
            let program = composite_program_of(medium, nearest, zones);
            let layout = composite_layout_of(program);
            assert_eq!(layout.sampler_count, samplers, "{program:?}");
            assert_eq!(layout.uniform_count, uniforms, "{program:?}");
            assert_eq!(layout.vertex_storage_count, 0);
            assert_eq!(layout.vertex_uniform_count, 0);
        }
    }

    #[test]
    fn named_slots_follow_the_shader_declaration() {
        let base = composite_layout_of(CompositeProgram::Base);
        for (slot, s) in [
            LightSampler::Scene,
            LightSampler::LightBuffer,
            LightSampler::Glow,
            LightSampler::Material,
            LightSampler::Detail,
            LightSampler::Surface,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(base.sampler(s), Some(slot as u8), "{s:?}");
        }
        assert_eq!(base.uniform(U::Composite), Some(0));
        assert_eq!(base.uniform(U::MediumParams), None);
        assert_eq!(base.sampler(S::ScatterTex), None);
        // The pure ambient follows the other samplers: slot 6 without the medium, 9 with it.
        assert_eq!(
            composite_layout_of(CompositeProgram::Zones).sampler(S::PureAmbient),
            Some(6)
        );
        let mz = composite_layout_of(CompositeProgram::MediumZones);
        assert_eq!(mz.sampler(S::PureAmbient), Some(9));
        assert_eq!(mz.sampler(S::ScatterTex), Some(6));
        assert_eq!(mz.sampler(S::MediumTex), Some(7));
        assert_eq!(mz.sampler(S::ScatterGuides), Some(8));
        assert_eq!(mz.uniform(U::MediumParams), Some(1));
    }

    #[test]
    fn program_choice_follows_chiaro() {
        assert_eq!(
            composite_program_of(false, false, false),
            CompositeProgram::Base
        );
        // The filter only matters with a medium.
        assert_eq!(
            composite_program_of(false, true, true),
            CompositeProgram::Zones
        );
        assert_ne!(
            composite_program_of(true, true, false),
            composite_program_of(true, false, false)
        );
        // Six distinct programs, indexed in table order.
        for (i, p) in COMPOSITE_PROGRAMS.iter().enumerate() {
            assert_eq!(p.index(), i);
            assert_eq!(p.has_medium(), p.fragment().contains("medium"));
            assert_eq!(p.has_zones(), p.fragment().contains("zones"));
        }
        let mut names: Vec<_> = COMPOSITE_PROGRAMS.iter().map(|p| p.fragment()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMPOSITE_PROGRAM_COUNT);
    }

    fn input<'a>(p: &'a Projection, s: &'a LightSettings) -> CompositeParamsInput<'a> {
        CompositeParamsInput {
            input_space: InputColorSpace::Srgb,
            scene_format_is_srgb: false,
            light_buffer: true,
            glow: None,
            ambient: Vec3::new(0.1, 0.2, 0.3),
            use_specular: false,
            direction: false,
            surface_buffer: false,
            projection: p,
            scene_size: (96, 48),
            settings: s,
        }
    }

    #[test]
    fn flags_follow_the_frame() {
        let (p, s) = (Projection::IDENTITY, LightSettings::default());
        let mut i = input(&p, &s);
        let b = CompositeParams::new(&i);
        assert_eq!(
            (b.decode_srgb, b.use_light_buffer, b.use_glow, b.decode_glow),
            (1, 1, 0, 0)
        );
        assert_eq!(b.ambient, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(b.glow_scale, 1.0);
        // Direction rows stay zero outside the direction mode.
        assert_eq!(b.wet, [0.0; 4]);
        assert_eq!(b.view_dir_spec, [0.0; 4]);

        i.scene_format_is_srgb = true;
        i.light_buffer = false;
        i.glow = Some(false);
        assert_eq!(
            CompositeParams::new(&i).decode_srgb,
            0,
            "an sRGB format decodes itself"
        );
        let b = CompositeParams::new(&i);
        assert_eq!((b.use_light_buffer, b.use_glow, b.decode_glow), (0, 1, 1));
        i.glow = Some(true);
        assert_eq!(CompositeParams::new(&i).decode_glow, 0);
        i.input_space = InputColorSpace::Linear;
        i.glow = Some(false);
        i.scene_format_is_srgb = false;
        let b = CompositeParams::new(&i);
        assert_eq!((b.decode_srgb, b.decode_glow), (0, 0));
    }

    #[test]
    fn direction_rows_follow_the_projection() {
        let p = Projection::IDENTITY;
        let mut s = LightSettings::default();
        s.material.specular_scale = 2.5;
        s.surface_height_step = 0.5;
        let mut i = input(&p, &s);
        i.direction = true;
        i.surface_buffer = true;
        let b = CompositeParams::new(&i);
        assert_eq!(b.use_direction, 1);
        assert_eq!(b.view_dir_spec[3], 2.5);
        assert_eq!(b.screen_right[3], 1.0);
        assert_eq!(b.z_axis_scene[2..], [96.0, 48.0]);
        assert_eq!(b.origin_step[2], 0.5);
        assert_eq!(b.xy_axes, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.inv_axes, [1.0, 0.0, 0.0, 1.0]);
        i.surface_buffer = false;
        assert_eq!(CompositeParams::new(&i).screen_right[3], 0.0);
    }

    #[test]
    fn bytes_follow_the_struct_layout() {
        let b = CompositeParams {
            decode_srgb: 1,
            use_light_buffer: 2,
            use_glow: 3,
            decode_glow: 4,
            ambient: [5.0, 6.0, 7.0, 8.0],
            glow_from_scene: 9,
            glow_scale: 10.0,
            use_specular: 11,
            use_direction: 12,
            view_dir_spec: [13.0, 14.0, 15.0, 16.0],
            screen_right: [17.0, 18.0, 19.0, 20.0],
            xy_axes: [21.0, 22.0, 23.0, 24.0],
            z_axis_scene: [25.0, 26.0, 27.0, 28.0],
            inv_axes: [29.0, 30.0, 31.0, 32.0],
            origin_step: [33.0, 34.0, 35.0, 36.0],
            wet: [37.0, 38.0, 39.0, 40.0],
        };
        let bytes = b.to_bytes();
        // SAFETY-free check: compare with the field offsets word by word.
        let word = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        assert_eq!(word(offset_of!(CompositeParams, decode_srgb)), 1);
        assert_eq!(word(offset_of!(CompositeParams, decode_glow)), 4);
        assert_eq!(word(offset_of!(CompositeParams, ambient)), 5.0f32.to_bits());
        assert_eq!(word(offset_of!(CompositeParams, glow_from_scene)), 9);
        assert_eq!(
            word(offset_of!(CompositeParams, glow_scale)),
            10.0f32.to_bits()
        );
        assert_eq!(word(offset_of!(CompositeParams, use_direction)), 12);
        assert_eq!(
            word(offset_of!(CompositeParams, view_dir_spec)),
            13.0f32.to_bits()
        );
        assert_eq!(
            word(offset_of!(CompositeParams, wet) + 12),
            40.0f32.to_bits()
        );
    }

    #[test]
    fn medium_filter_picks_the_program() {
        let mut s = LightSettings::default();
        assert!(!medium_nearest(&s));
        s.quality.medium_filter = crate::settings::Filter::Nearest;
        assert!(medium_nearest(&s));
    }
}
