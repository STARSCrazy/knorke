// Origin: chiaro @ 9bc5b9b, src/rendering/MediumMapStage.hpp and MediumMapStage.cpp (the uniform
// blocks, Builds and the map grid of Prepare), src/gpu/MediumLayout.hpp (MediumLayoutOf,
// MediumMapSlots), src/rendering/MediumStage.hpp and MediumStage.cpp (MediumParams,
// FillMediumParams, ScaledSize).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The medium world map (P29, Chiaro E100, E108): the host's medium layer reprojected onto the
//! world grid of the occlusion mask at a quarter of its resolution, RGBA16F
//! `(sigma, bottom, top, highest top)` per cell. The `*_medium` light programs dim each light on
//! its way through it; the medium stage (P30) reads it for its own light pass.
//!
//! | Module | Content |
//! |---|---|
//! | this one | uniform blocks ([`MediumParams`], [`MediumMapParams`], the map passes' blocks), the map grid ([`MediumMapLayout`]), the binding layouts of the medium programs ([`medium_layout_of`]), without a GPU |
//! | `gpu` (feature `gpu`) | [`MediumMapStage`]: splat, top and resolve into the map |
//!
//! The map is built only for a frame with a medium layer and at least one light in the store
//! ([`medium_map_builds`]); otherwise nothing is created, bound or recorded and the light
//! programs stay the plain ones.
//!
//! # Redesign against the C++ version
//!
//! - [`medium_params`] (Chiaro `FillMediumParams`) takes its inputs as values instead of the
//!   frame; P30 fills its block with the same function.
//! - The map's grid is a pure function ([`MediumMapLayout::new`]) of projection, domain and
//!   anchor, so it is tested without a GPU.

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{
    MEDIUM_LAYER_FORMAT, MEDIUM_MAP_FORMAT, MEDIUM_MAP_PROGRAMS, MEDIUM_NOT_PREPARED,
    MEDIUM_SPLAT_FORMAT, MEDIUM_TOP_FORMAT, MediumMapFrame, MediumMapPrograms, MediumMapStage,
    MediumMapView,
};

use crate::binding_layout::{LayoutBuilder, LightLayout, LightSampler, LightStorage, LightUniform};
use crate::codec::MEDIUM_COUNT;
use crate::material::view_direction;
use crate::medium_table::MediumTable;
use crate::settings::LightSettings;
use crate::shadow::{MaskAnchor, OcclusionDomain};
use crate::{LightStore, Projection};
use knorke_math::uniform_block;

/// Map texels per occlusion mask texel, per axis (the spec's quarter resolution).
pub const MASK_TEXELS_PER_CELL: u32 = 4;
/// Splat blocks per map texel, per axis: 4 points land per cell the medium covers.
pub const BLOCKS_PER_CELL: u32 = 2;
/// Most taps per side of a splat block (64 per block); thins out below shadow resolution 0.25.
pub const MAX_BLOCK_TAPS: u32 = 8;
/// Texels of row 0 the cell rectangle needs: the top in 0, the rectangle in 1 to 4 (E108).
pub const RECT_TEXELS: u32 = 5;

uniform_block! {
    /// `cbuffer MediumParams` (`medium_common.hlsli`): the medium programs' block, filled once
    /// per frame by [`medium_params`]. The splat reads the sizes, steps, z axis and the table's
    /// extinctions; the medium stage (P30) and composite read all of it.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumParams [464] {
        /// Rows of the inverse of `[x_axis y_axis]`.
        pub inv_axes: [f32; 4],
        /// xy projection origin, zw z axis.
        pub origin_z_axis: [f32; 4],
        /// xy scene size, zw scatter buffer size.
        pub sizes: [f32; 4],
        /// xyz view direction `w` (`w.z > 0`), w `1 / w.z`.
        pub view: [f32; 4],
        /// x surface height step, y medium height step, z 1 = surface buffer, w 1 = medium layer.
        pub steps: [f32; 4],
        /// x density (0 = none), y height falloff `H` (> 0), z noise cells per world unit.
        pub haze: [f32; 4],
        /// rgb albedo of the haze, a its Henyey-Greenstein `g`.
        pub haze_color: [f32; 4],
        /// rgb ambient light.
        pub ambient: [f32; 4],
        /// The media table: rgb albedo, a extinction.
        pub media: [[f32; 4]; 16],
        /// x scatter samples per pixel and light, y transmission samples per scatter sample.
        pub light_pass: [f32; 4],
        /// Henyey-Greenstein `g` of the media table, entry `i` in `[i / 4][i % 4]`.
        pub phase: [[f32; 4]; 4],
    }
}

uniform_block! {
    /// `cbuffer MediumMapParams` (`medium_transmit.hlsli`): what the programs that read the map
    /// need of it; the `*_medium` light programs and the medium stage push it as is.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumMapParams [80] {
        /// xyz: map texel x = `dot(xyz, (world x, world y, 1))`; w unused.
        pub map_x: [f32; 4],
        /// xyz: map texel y, the same.
        pub map_y: [f32; 4],
        /// xy map size in texels, z highest top the layer can encode (world units), w 1 = map
        /// present, 2 = with the cell rectangle in row 0 (E108).
        pub map_size: [f32; 4],
        /// Rows of the inverse of `[x_axis y_axis]` (scene to ground in fullscreen programs).
        pub inv_axes: [f32; 4],
        /// xy projection origin in scene pixels.
        pub origin: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer MediumSplatParams` (`medium_world.vert.hlsl`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumSplatParams [48] {
        /// Scene position (z = 0) of map position `m`: `m * xy + zw`.
        pub scene_from_map: [f32; 4],
        /// xy map units per scene pixel.
        pub map_from_scene: [f32; 4],
        /// xy map size in texels, z blocks per row, w taps per block side.
        pub map_blocks: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer TopParams` (`medium_world_top.vert.hlsl`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumTopParams [16] {
        /// xy map size in texels.
        pub map: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer ResolveParams` (`medium_world_resolve.frag.hlsl`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumResolveParams [16] {
        /// x points per fully covered cell, yz map size in texels, w 1 = cell rectangle in row 0.
        pub resolve: [f32; 4],
    }
}

/// Whether a frame builds the map (Chiaro `MediumMapStage::Builds`): a medium layer and at least
/// one light in the store, visible or not. Without the layer nothing shields (haze does not dim
/// the light path); without lights nothing reads the map.
pub fn medium_map_builds(has_layer: bool, lights: &LightStore) -> bool {
    has_layer && lights.len() + lights.frame_light_count() > 0
}

/// `max(1, ceil(size * resolution))` (Chiaro `ScaledSize` of the medium stage; unlike
/// [`scaled_buffer_size`](crate::light_math::scaled_buffer_size) without the float noise guard).
pub fn medium_buffer_size(size: u32, resolution: f32) -> u32 {
    let scaled = (f64::from(size) * f64::from(resolution)).ceil();
    if scaled < 1.0 { 1 } else { scaled as u32 }
}

/// The medium programs' block of a frame (Chiaro `FillMediumParams`): projection, scene and
/// scatter buffer size (the quality's medium resolution), steps, haze, ambient, media table and
/// sample counts. `surface`, `layer`: the frame has a surface buffer, a medium layer.
pub fn medium_params(
    p: &Projection,
    (width, height): (u32, u32),
    settings: &LightSettings,
    surface: bool,
    layer: bool,
    media: &MediumTable,
) -> MediumParams {
    let haze = &settings.haze;
    let quality = settings.quality.quality.medium_cost();
    let (det, inv_axes) = p.inverse_axes();
    let w = view_direction(p);
    let origin = p.origin.raw();
    let a = settings.ambient;
    // Noise cells: a whole number per world unit, so that a pan by whole world units moves every
    // cell onto another whole cell; about one scatter texel each.
    let cells = (det.abs().sqrt() * f64::from(quality.resolution) + 0.5).floor();
    let mut m = MediumParams {
        inv_axes,
        origin_z_axis: [origin.x, origin.y, p.z_axis.x, p.z_axis.y],
        sizes: [
            width as f32,
            height as f32,
            medium_buffer_size(width, quality.resolution) as f32,
            medium_buffer_size(height, quality.resolution) as f32,
        ],
        // w.z > 0: the projection's axes are not collinear (validated).
        view: [w.x, w.y, w.z, 1.0 / w.z],
        steps: [
            settings.surface_height_step,
            settings.resolved_medium_height_step(),
            if surface { 1.0 } else { 0.0 },
            if layer { 1.0 } else { 0.0 },
        ],
        haze: [
            haze.density,
            if haze.density > 0.0 {
                settings.haze_falloff(p)
            } else {
                1.0
            },
            if cells < 1.0 { 1.0 } else { cells as f32 },
            0.0,
        ],
        haze_color: [haze.color.x, haze.color.y, haze.color.z, haze.anisotropy],
        ambient: [a.x, a.y, a.z, 0.0],
        media: [[0.0; 4]; MEDIUM_COUNT as usize],
        light_pass: [
            quality.samples as f32,
            quality.transmission_samples as f32,
            0.0,
            0.0,
        ],
        phase: [[0.0; 4]; 4],
    };
    for (i, d) in media.entries().iter().enumerate() {
        m.media[i] = [d.color.x, d.color.y, d.color.z, d.extinction];
        m.phase[i / 4][i % 4] = d.anisotropy;
    }
    m
}

/// The map's grid of a frame (the CPU part of Chiaro `MediumMapStage::Prepare`): its size, its
/// [`MediumMapParams`] and the splat's mapping. The grid is the occlusion mask's (`domain`,
/// `anchor`) at a quarter of its resolution: map position = mask position /
/// [`MASK_TEXELS_PER_CELL`], mask position = `(scene + margin) * texels_per_scene - remainder`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediumMapLayout {
    /// Map size in texels.
    pub width: u32,
    /// Map size in texels.
    pub height: u32,
    /// The block of the programs that read the map.
    pub params: MediumMapParams,
    /// Scene position (z = 0) of map position `m`: `m * xy + zw`.
    pub scene_from_map: [f32; 4],
    /// Map units per scene pixel.
    pub map_from_scene: [f32; 2],
    /// Taps per side of a splat block (its footprint in layer pixels, capped).
    pub block_taps: u32,
}

impl MediumMapLayout {
    /// The grid of `domain` and `anchor` under `p`; `medium_height_step` is the settings'
    /// resolved step (the layer encodes at most 255 of them).
    pub fn new(
        p: &Projection,
        domain: &OcclusionDomain,
        anchor: &MaskAnchor,
        medium_height_step: f32,
    ) -> Self {
        let width = domain.width.div_ceil(MASK_TEXELS_PER_CELL).max(1);
        let height = domain.height.div_ceil(MASK_TEXELS_PER_CELL).max(1);
        let cell = f64::from(MASK_TEXELS_PER_CELL);
        let tps = domain.texels_per_scene.map(f64::from);
        let margin = f64::from(domain.margin);
        let origin = p.origin.raw();
        let origin = [f64::from(origin.x), f64::from(origin.y)];
        let x_axis = [p.x_axis.x, p.x_axis.y];
        let y_axis = [p.y_axis.x, p.y_axis.y];
        let mut rows = [[0.0f32; 4]; 2];
        let mut scene_from_map = [0.0f32; 4];
        let mut map_from_scene = [0.0f32; 2];
        for i in 0..2 {
            let remainder = f64::from(anchor.remainder[i]);
            rows[i][0] = (f64::from(x_axis[i]) * tps[i] / cell) as f32;
            rows[i][1] = (f64::from(y_axis[i]) * tps[i] / cell) as f32;
            rows[i][2] = (((origin[i] + margin) * tps[i] - remainder) / cell) as f32;
            // The inverse of map = (scene + margin) * tps / cell - remainder / cell.
            scene_from_map[i] = (cell / tps[i]) as f32;
            scene_from_map[2 + i] = (remainder / tps[i] - margin) as f32;
            map_from_scene[i] = (tps[i] / cell) as f32;
        }
        // A block is 1 / BLOCKS_PER_CELL of a cell: one tap per layer pixel of its footprint.
        let (bx, by) = (cell / tps[0], cell / tps[1]);
        let block_pixels = if bx > by { bx } else { by } / f64::from(BLOCKS_PER_CELL);
        let block_taps = ((block_pixels - 1e-6).ceil() as u32).clamp(1, MAX_BLOCK_TAPS);
        let (_, inv_axes) = p.inverse_axes();
        let params = MediumMapParams {
            map_x: rows[0],
            map_y: rows[1],
            map_size: [
                width as f32,
                height as f32,
                // The real top of the frame is known on the GPU only: the map holds it in A.
                255.0 * medium_height_step,
                // 2: the cell rectangle in A of texels 1 to 4 of row 0; it needs 5 texels.
                if width >= RECT_TEXELS { 2.0 } else { 1.0 },
            ],
            inv_axes,
            origin: [origin[0] as f32, origin[1] as f32, 0.0, 0.0],
        };
        Self {
            width,
            height,
            params,
            scene_from_map,
            map_from_scene,
            block_taps,
        }
    }

    /// Splat blocks per row and column (2 x 2 per map texel).
    pub fn blocks(&self) -> (u32, u32) {
        (self.width * BLOCKS_PER_CELL, self.height * BLOCKS_PER_CELL)
    }

    /// The splat pass's block.
    pub fn splat_params(&self) -> MediumSplatParams {
        let s = self.params.map_size;
        MediumSplatParams {
            scene_from_map: self.scene_from_map,
            map_from_scene: [self.map_from_scene[0], self.map_from_scene[1], 0.0, 0.0],
            map_blocks: [s[0], s[1], self.blocks().0 as f32, self.block_taps as f32],
        }
    }

    /// The top pass's block.
    pub fn top_params(&self) -> MediumTopParams {
        let s = self.params.map_size;
        MediumTopParams {
            map: [s[0], s[1], 0.0, 0.0],
        }
    }

    /// The resolve pass's block.
    pub fn resolve_params(&self) -> MediumResolveParams {
        let s = self.params.map_size;
        let points = (BLOCKS_PER_CELL * BLOCKS_PER_CELL) as f32;
        MediumResolveParams {
            resolve: [points, s[0], s[1], if s[3] > 1.5 { 1.0 } else { 0.0 }],
        }
    }
}

/// The programs of the medium stage (P30) that bind named resources (Chiaro's `ProgramId`
/// family of [`medium_layout_of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediumProgram {
    /// `medium_base.frag`: transmittance and ambient scatter.
    Base,
    /// `medium_base_zones.frag`: with the pure ambient of ambient zones.
    BaseZones,
    /// `medium_light.vert/.frag`: scattered light of point, spot, line and rect lights.
    Light,
    /// `medium_light_mask.vert/.frag`: the same for masked lights.
    LightMask,
    /// `medium_directional.frag`: scattered light of a directional light.
    Directional,
    /// `medium_filter.frag`: pre-filter and guides of the scatter buffer.
    Filter,
}

impl MediumProgram {
    /// Every program.
    pub const ALL: [Self; 6] = [
        Self::Base,
        Self::BaseZones,
        Self::Light,
        Self::LightMask,
        Self::Directional,
        Self::Filter,
    ];

    /// Vertex shader name in the pack.
    pub const fn vertex(self) -> &'static str {
        match self {
            Self::Light => "medium_light.vert",
            Self::LightMask => "medium_light_mask.vert",
            _ => "fullscreen.vert",
        }
    }

    /// Fragment shader name in the pack.
    pub const fn fragment(self) -> &'static str {
        match self {
            Self::Base => "medium_base.frag",
            Self::BaseZones => "medium_base_zones.frag",
            Self::Light => "medium_light.frag",
            Self::LightMask => "medium_light_mask.frag",
            Self::Directional => "medium_directional.frag",
            Self::Filter => "medium_filter.frag",
        }
    }
}

/// The binding layout of a medium program (Chiaro `MediumLayoutOf`, E156): the slot of each
/// named resource in the order the shaders declare them. The atlases (Profile, Mask) and the
/// world map (MediumMap, MediumMapParams) are the same connectors as in the light programs.
pub const fn medium_layout_of(program: MediumProgram) -> LightLayout {
    use LightSampler as S;
    use LightUniform as U;
    use MediumProgram as P;
    let mut b = LayoutBuilder::new();
    match program {
        P::Base | P::BaseZones => {
            b.sampler(S::Surface);
            b.sampler(S::MediumTex);
            b.uniform(U::MediumParams);
            if matches!(program, P::BaseZones) {
                b.sampler(S::PureAmbient);
                b.uniform(U::Zone);
            }
        }
        P::Light | P::LightMask => {
            b.sampler(S::Surface);
            b.sampler(S::MediumTex);
            b.sampler(S::Profile);
            b.sampler(S::MediumMap);
            b.sampler(S::PolarAtlas);
            b.uniform(U::MediumParams);
            b.uniform(U::MediumMap);
            b.uniform(U::PolarQuery);
            b.storage(LightStorage::Lights);
            b.vertex_uniforms(1); // MediumLightPassParams
            if matches!(program, P::LightMask) {
                b.sampler(S::Mask);
                b.storage(LightStorage::Masks);
            }
        }
        P::Directional => {
            b.sampler(S::Surface);
            b.sampler(S::MediumTex);
            b.sampler(S::MediumMap);
            b.sampler(S::SunHeights);
            b.uniform(U::MediumParams);
            b.uniform(U::MediumMap);
            b.uniform(U::Sun);
        }
        P::Filter => {
            b.sampler(S::ScatterTex);
            b.sampler(S::Surface);
            b.sampler(S::MediumTex);
            b.uniform(U::MediumParams);
        }
    }
    b.finish()
}

/// Slots of the map passes (Chiaro `MediumMapSlots`), each with one fixed layout: splat and top
/// read their input in vertex sampler 0 and take vertex uniform blocks, resolve reads three
/// fragment samplers.
pub mod slots {
    /// Splat: the medium layer (vertex sampler).
    pub const SPLAT_LAYER_SAMPLER: u32 = 0;
    /// Splat: `MediumParams` (vertex uniform).
    pub const SPLAT_MEDIUM_PARAMS: u32 = 0;
    /// Splat: `MediumSplatParams` (vertex uniform).
    pub const SPLAT_PARAMS: u32 = 1;
    /// Top: the slab union (vertex sampler).
    pub const TOP_SLAB_SAMPLER: u32 = 0;
    /// Top: `TopParams` (vertex uniform).
    pub const TOP_PARAMS: u32 = 0;
    /// Resolve: sums, union, top (fragment samplers).
    pub const RESOLVE_SAMPLER_COUNT: u32 = 3;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::LightDesc;
    use crate::light::LightType;
    use knorke_math::Vec2;

    fn iso() -> Projection {
        Projection {
            origin: knorke_math::At::new(Vec2::new(40.0, 10.0)),
            x_axis: Vec2::new(8.0, 4.0),
            y_axis: Vec2::new(-8.0, 4.0),
            z_axis: Vec2::new(0.0, -10.0),
        }
    }

    #[test]
    fn sizes_follow_chiaro() {
        assert_eq!(size_of::<MediumParams>(), 464);
        assert_eq!(core::mem::offset_of!(MediumParams, view), 48);
        assert_eq!(core::mem::offset_of!(MediumParams, steps), 64);
        assert_eq!(core::mem::offset_of!(MediumParams, ambient), 112);
        assert_eq!(core::mem::offset_of!(MediumParams, media), 128);
        assert_eq!(core::mem::offset_of!(MediumParams, light_pass), 384);
        assert_eq!(core::mem::offset_of!(MediumParams, phase), 400);
        assert_eq!(size_of::<MediumMapParams>(), 80);
        assert_eq!(size_of::<MediumSplatParams>(), 48);
        assert_eq!(size_of::<MediumTopParams>(), 16);
        assert_eq!(size_of::<MediumResolveParams>(), 16);
    }

    #[test]
    fn builds_needs_a_layer_and_a_light() {
        let mut lights = LightStore::new();
        assert!(!medium_map_builds(true, &lights));
        lights
            .create(
                &LightDesc::new(LightType::Point),
                &crate::light::NoResources,
            )
            .unwrap();
        assert!(medium_map_builds(true, &lights));
        assert!(!medium_map_builds(false, &lights));
    }

    #[test]
    fn buffer_size_has_no_noise_guard() {
        assert_eq!(medium_buffer_size(100, 0.5), 50);
        assert_eq!(medium_buffer_size(3, 0.25), 1);
        assert_eq!(medium_buffer_size(0, 0.5), 1);
        // 100 * 0.3f is 30.000001: Chiaro's ScaledSize rounds it up.
        assert_eq!(medium_buffer_size(100, 0.3), 31);
    }

    #[test]
    fn params_fill_projection_steps_and_table() {
        let mut settings = LightSettings {
            surface_height_step: 0.5,
            ..LightSettings::default()
        };
        settings.haze.density = 0.2;
        settings.haze.height_falloff = Some(3.0);
        let mut media = MediumTable::new();
        media
            .set(
                7,
                crate::medium_table::MediumDesc::new(
                    knorke_math::Vec3::new(0.1, 0.2, 0.3),
                    4.0,
                    -0.5,
                ),
            )
            .unwrap();
        let p = iso();
        let m = medium_params(&p, (100, 60), &settings, true, true, &media);
        // det = 8 * 4 - 4 * -8 = 64.
        assert_eq!(
            m.inv_axes,
            [4.0 / 64.0, 8.0 / 64.0, -4.0 / 64.0, 8.0 / 64.0]
        );
        assert_eq!(m.origin_z_axis, [40.0, 10.0, 0.0, -10.0]);
        assert_eq!(m.sizes, [100.0, 60.0, 50.0, 30.0]);
        assert_eq!(m.steps, [0.5, 2.0, 1.0, 1.0]);
        // sqrt(64) * 0.5 = 4 noise cells per world unit.
        assert_eq!(m.haze, [0.2, 3.0, 4.0, 0.0]);
        assert_eq!(m.media[7], [0.1, 0.2, 0.3, 4.0]);
        assert_eq!(m.phase[1][3], -0.5);
        assert_eq!(m.light_pass, [16.0, 4.0, 0.0, 0.0]);
        assert_eq!(m.view[3], 1.0 / m.view[2]);
        // Without haze the falloff is 1 and no layer clears the step flag.
        settings.haze.density = 0.0;
        let m = medium_params(&p, (100, 60), &settings, false, false, &media);
        assert_eq!((m.haze[1], m.steps[2], m.steps[3]), (1.0, 0.0, 0.0));
    }

    #[test]
    fn layout_is_a_quarter_of_the_mask_grid() {
        let p = Projection::default();
        let d = OcclusionDomain::new(96, 48, 1.0);
        let a = MaskAnchor::compute(&p, &d);
        let l = MediumMapLayout::new(&p, &d, &a, 4.0);
        // Domain 96 + 2 * 24 = 144 by 48 + 48 = 96 mask texels.
        assert_eq!((d.width, d.height), (144, 96));
        assert_eq!((l.width, l.height), (36, 24));
        assert_eq!(l.params.map_size, [36.0, 24.0, 1020.0, 2.0]);
        // Map = (world + margin) / 4 under the identity projection.
        assert_eq!(l.params.map_x, [0.25, 0.0, 6.0, 0.0]);
        assert_eq!(l.params.map_y, [0.0, 0.25, 6.0, 0.0]);
        assert_eq!(l.scene_from_map, [4.0, 4.0, -24.0, -24.0]);
        assert_eq!(l.map_from_scene, [0.25, 0.25]);
        // A block is 2 scene pixels: 2 taps per side.
        assert_eq!(l.block_taps, 2);
        assert_eq!(l.blocks(), (72, 48));
        assert_eq!(l.splat_params().map_blocks, [36.0, 24.0, 72.0, 2.0]);
        assert_eq!(l.resolve_params().resolve, [4.0, 36.0, 24.0, 1.0]);
        assert_eq!(l.top_params().map, [36.0, 24.0, 0.0, 0.0]);
    }

    #[test]
    fn layout_caps_taps_and_drops_the_rectangle_on_narrow_maps() {
        let p = Projection::default();
        // Shadow resolution 0.125: a block covers 16 layer pixels, capped at 8 taps.
        let d = OcclusionDomain::new(96, 48, 0.125);
        let l = MediumMapLayout::new(&p, &d, &MaskAnchor::compute(&p, &d), 1.0);
        assert_eq!(l.block_taps, MAX_BLOCK_TAPS);
        assert_eq!((l.width, l.height), (5, 3));
        assert_eq!(l.params.map_size[3], 2.0);
        let d = OcclusionDomain::new(16, 16, 0.25);
        let l = MediumMapLayout::new(&p, &d, &MaskAnchor::compute(&p, &d), 1.0);
        assert!(l.width < RECT_TEXELS);
        assert_eq!(l.params.map_size[3], 1.0);
        assert_eq!(l.resolve_params().resolve[3], 0.0);
    }

    #[test]
    fn layout_maps_the_anchor_remainder() {
        let mut p = iso();
        p.origin = knorke_math::At::new(Vec2::new(40.3, 10.7));
        let d = OcclusionDomain::new(100, 60, 0.5);
        let a = MaskAnchor::compute(&p, &d);
        let l = MediumMapLayout::new(&p, &d, &a, 1.0);
        // A world point maps to the mask texel / 4 and back through scene_from_map.
        let world = Vec2::new(1.5, -2.0);
        let mx = l.params.map_x;
        let map_x = mx[0] * world.x + mx[1] * world.y + mx[2];
        let scene = d.scene_to_domain(Vec2::new(
            40.3 + 8.0 * world.x - 8.0 * world.y,
            10.7 + 4.0 * world.x + 4.0 * world.y,
        ));
        assert!((map_x - (scene.x - a.remainder[0]) / 4.0).abs() < 1e-4);
        let back = map_x * l.scene_from_map[0] + l.scene_from_map[2];
        assert!((back - (40.3 + 8.0 * world.x - 8.0 * world.y)).abs() < 1e-3);
    }

    #[test]
    fn medium_layouts_follow_chiaro() {
        let l = medium_layout_of(MediumProgram::LightMask);
        assert_eq!(l.sampler_count, 6);
        assert_eq!(l.sampler(LightSampler::Mask), Some(5));
        assert_eq!(l.uniform(LightUniform::PolarQuery), Some(2));
        assert_eq!(l.vertex_uniform_count, 1);
        let z = medium_layout_of(MediumProgram::BaseZones);
        assert_eq!(z.sampler(LightSampler::PureAmbient), Some(2));
        assert_eq!(z.uniform(LightUniform::Zone), Some(1));
    }
}
