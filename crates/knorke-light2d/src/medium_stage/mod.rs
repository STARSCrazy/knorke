// Origin: chiaro @ 9bc5b9b, src/rendering/MediumStage.hpp and MediumStage.cpp (MediumRuns,
// SunMapDivisor, the uniform blocks, MaskRows, the CPU parts of Prepare, PrepareShafts and
// PrepareSuns).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The medium stage (P30, Chiaro `MediumStage`): haze and the medium layer between the light
//! pass and the composite. It writes a scatter buffer at the medium resolution, A the
//! transmittance of each pixel's view ray, RGB the light scattered toward the viewer: the
//! ambient in the base pass, then one instanced draw over the reach of every light with
//! `haze > 0` ([`collect_medium_lights`](crate::medium_culling::collect_medium_lights)), each
//! sample dimmed by the medium world map (P29) and hidden from shadowed point and spot lights by
//! the obstacles in the light's row of the polar atlas (the light shafts); then one fullscreen
//! draw per directional light, shaded below the sun shadow heights of its block in the sun
//! height atlas when it casts shadows. A pre-filter writes the filtered buffer and the guides
//! the composite upsamples with.
//!
//! | Module | Content |
//! |---|---|
//! | this one | [`medium_runs`], the uniform blocks, the shaft rows and sun blocks of a frame ([`plan_shafts`], [`plan_suns`]), without a GPU |
//! | `gpu` (feature `gpu`) | `MediumStage` |
//!
//! Without haze (density 0) and without a medium layer the stage records, binds and allocates
//! nothing and the composite runs its plain program.
//!
//! # Redesign against the C++ version
//!
//! - The shaft rows and sun blocks are pure functions of the light list and the shadow inputs,
//!   tested without a GPU; the GPU part only records them.
//! - A frame without the stage releases its targets (Chiaro keeps them), as the medium map does.

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{
    MEDIUM_GUIDE_FORMAT, MEDIUM_PROGRAMS, MEDIUM_STAGE_NOT_PREPARED, MediumLights, MediumStage,
    MediumStageFrame, MediumStageRecord, POLAR_FORMAT, SCATTER_FORMAT, SUN_HEIGHT_FORMAT,
};

use crate::gpu_light::GpuLight;
use crate::light_culling::Casters;
use crate::medium_culling::{MediumSun, Shafts};
use crate::projection::Projection;
use crate::settings::LightSettings;
use crate::shadow::OcclusionDomain;
use knorke_math::{det, uniform_block};

/// Angles per row of the polar atlas (texels across).
pub const POLAR_ANGLES: u32 = 512;
/// The sun height atlas never grows taller: further directional lights scatter unshaded.
pub const MAX_SUN_ATLAS_HEIGHT: u32 = 8192;
/// A sun at or below the horizon still rises this much per unit (as `MediumTransmitToward`).
const MIN_SUN_Z: f32 = 0.05;

/// The stage runs this frame (Chiaro `MediumRuns`): haze or a medium layer. The light pass asks
/// too: with zones it then keeps the pure ambient for the base pass (the dual path).
pub fn medium_runs(settings: &LightSettings, has_layer: bool) -> bool {
    settings.haze.density > 0.0 || has_layer
}

/// Mask texels per texel of the sun shadow height map (E107): about two scatter texels per
/// texel, `round(2 * shadow_resolution / medium_resolution)`, at least 1.
pub fn sun_map_divisor(shadow_resolution: f32, medium_resolution: f32) -> u32 {
    let ratio = 2.0 * f64::from(shadow_resolution) / f64::from(medium_resolution);
    (ratio.round() as u32).max(1)
}

/// The highest point any medium can reach: the layer's top encodes at most 255 steps (its real
/// top is known on the GPU only); haze is sampled up to 5 falloffs above the highest visible
/// point.
pub fn medium_top(
    has_layer: bool,
    medium_height_step: f32,
    haze_density: f32,
    max_height: f32,
    falloff: f32,
) -> f32 {
    let layer = if has_layer {
        255.0 * medium_height_step
    } else {
        0.0
    };
    let haze = if haze_density > 0.0 {
        max_height + 5.0 * falloff
    } else {
        0.0
    };
    det::max(layer, haze)
}

/// Rows mask texel x and y = `dot(row.xyz, (world x, world y, 1))` of the frame's occlusion
/// domain (Chiaro `MaskRows`): mask position = `(scene + margin) * texels_per_scene - remainder`.
pub fn mask_rows(p: &Projection, d: &OcclusionDomain, remainder: [f32; 2]) -> [[f32; 4]; 2] {
    let origin = p.origin.raw();
    let origin = [origin.x, origin.y];
    let (x, y) = ([p.x_axis.x, p.x_axis.y], [p.y_axis.x, p.y_axis.y]);
    core::array::from_fn(|i| {
        let tps = f64::from(d.texels_per_scene[i]);
        [
            (f64::from(x[i]) * tps) as f32,
            (f64::from(y[i]) * tps) as f32,
            ((f64::from(origin[i]) + f64::from(d.margin)) * tps - f64::from(remainder[i])) as f32,
            0.0,
        ]
    })
}

uniform_block! {
    /// `cbuffer PolarQueryParams` (`medium_polar.hlsli`): how the light pass finds its row.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct PolarQueryParams [48] {
        /// xyz: mask texel x = `dot(xyz, (world x, world y, 1))`.
        pub mask_x: [f32; 4],
        /// xyz: mask texel y.
        pub mask_y: [f32; 4],
        /// x angles per row, y atlas height in texels, z tolerance below a top (E22).
        pub atlas: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer SunParams` (`medium_sun.hlsli`): one directional light of the sun pass.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SunParams [96] {
        /// rgb colour * intensity * haze, w the light's atlas block + 1 (0 = no shadows).
        pub color: [f32; 4],
        /// xyz unit vector toward the light, w highest finite obstacle top.
        pub to_light: [f32; 4],
        /// xyz: mask texel x.
        pub mask_x: [f32; 4],
        /// xyz: mask texel y.
        pub mask_y: [f32; 4],
        /// xy mask size in texels, zw atlas size in texels.
        pub atlas: [f32; 4],
        /// xy size of one block in texels.
        pub block: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer MediumLightPassParams` (`medium_light.vert`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumLightPassParams [48] {
        /// xy projection origin, zw x axis.
        pub origin_x_axis: [f32; 4],
        /// xy y axis, zw z axis.
        pub y_axis_z_axis: [f32; 4],
        /// xy scene size, zw scatter buffer size.
        pub sizes: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer MediumLightPassParams` of `medium_light_mask.vert` (`CHI_LIGHT_MASK`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumMaskLightPassParams [64] {
        /// The unmasked block.
        pub base: MediumLightPassParams,
        /// x index of the first masked light in the light buffer (its mask is `Masks[0]`).
        pub light_offset: [u32; 4],
    }
}

uniform_block! {
    /// `cbuffer PolarRowParams` (`medium_polar.frag`): one row of the polar atlas.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct PolarRowParams [64] {
        /// xyz: mask texel x.
        pub mask_x: [f32; 4],
        /// xyz: mask texel y.
        pub mask_y: [f32; 4],
        /// xyz light position in world units, w march length in mask texels (negative: the
        /// light sits inside an obstacle, E147).
        pub light: [f32; 4],
        /// xy mask size in texels, z 1 = height texture, w angles per row.
        pub domain: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer SunHeightParams` (`medium_sun_height.frag`): one block of the sun height atlas.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SunHeightParams [32] {
        /// xy unit direction toward the light in mask texels, z rise per mask texel.
        pub march: [f32; 4],
        /// xy mask size in texels, z highest finite obstacle top.
        pub domain: [f32; 4],
    }
}

/// One row of the polar atlas: the light (index into the stage's light list, masked lights
/// after the others) and the caster class whose field it traces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaftRow {
    /// Index of the light.
    pub light: u32,
    /// Its shafts.
    pub shafts: Shafts,
}

/// The shaft rows of a frame (Chiaro `PrepareShafts`): one per light with [`Shafts`] that can
/// be traced this frame (`traced`: its caster class has a field, or with light rows the light a
/// row), in light order. The light of row `r`
/// then carries `shadow[0] = r + 1` (`medium_light.vert`); the others scatter unshaded.
pub fn plan_shafts<'a>(
    shafts: impl IntoIterator<Item = &'a Option<Shafts>>,
    traced: impl Fn(&Shafts) -> bool,
    rows: &mut Vec<ShaftRow>,
) {
    rows.clear();
    for (i, s) in shafts.into_iter().enumerate() {
        if let Some(s) = s.filter(|s| traced(s)) {
            rows.push(ShaftRow {
                light: i as u32,
                shafts: s,
            });
        }
    }
}

/// The march length of a polar row in mask texels: the light's radius times the Frobenius norm
/// of world to mask (it bounds the stretch); negative marks a light inside an obstacle.
pub fn polar_row(query: &PolarQueryParams, light: &GpuLight, inside: bool) -> [f32; 4] {
    let (x, y) = (query.mask_x, query.mask_y);
    let stretch = (x[0] * x[0] + x[1] * x[1] + y[0] * y[0] + y[1] * y[1]).sqrt();
    let p = light.position_radius;
    let reach = p[3] * stretch;
    [p[0], p[1], p[2], if inside { -reach } else { reach }]
}

/// One block of the sun height atlas: the sun (index into the sun list) and its march.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunBlock {
    /// Index of the sun.
    pub sun: u32,
    /// Its caster class.
    pub casters: Casters,
    /// [`SunHeightParams::march`].
    pub march: [f32; 4],
}

/// The shadow inputs of the sun blocks.
#[derive(Clone, Copy, Debug)]
pub struct SunShadows {
    /// The occlusion domain.
    pub domain: OcclusionDomain,
    /// Highest finite obstacle top.
    pub scene_max_top: f32,
}

/// The sun pass of a frame (Chiaro `PrepareSuns`): one [`SunParams`] per sun and a block of the
/// sun height atlas for each sun with shadows whose caster class has a field and heights
/// (`has_heights`), while the atlas stays below [`MAX_SUN_ATLAS_HEIGHT`]. `shadows`: the
/// frame's shadow inputs, if any; `shafts`: blocks can exist (shadows with heights and a finite
/// top, shafts on). Returns the block size in texels; the atlas fields of `params` are filled by
/// [`finish_suns`].
#[allow(clippy::too_many_arguments)]
pub fn plan_suns(
    suns: &[MediumSun],
    p: &Projection,
    settings: &LightSettings,
    shadows: Option<&SunShadows>,
    shafts: bool,
    has_heights: impl Fn(&MediumSun) -> bool,
    params: &mut Vec<SunParams>,
    blocks: &mut Vec<SunBlock>,
) -> (u32, u32) {
    params.clear();
    blocks.clear();
    let r = settings.resolutions();
    let divisor = sun_map_divisor(r.shadow, r.medium);
    let (dw, dh, tps, top) = shadows.map_or((0, 0, [0.0; 2], 0.0), |s| {
        (
            s.domain.width,
            s.domain.height,
            s.domain.texels_per_scene,
            s.scene_max_top,
        )
    });
    let block = (dw.div_ceil(divisor).max(1), dh.div_ceil(divisor).max(1));
    let max_blocks = if shafts {
        (MAX_SUN_ATLAS_HEIGHT / block.1) as usize
    } else {
        0
    };
    for (i, s) in suns.iter().enumerate() {
        let mut sp = SunParams {
            color: [s.color.x, s.color.y, s.color.z, 0.0],
            to_light: [s.to_light.x, s.to_light.y, s.to_light.z, top],
            ..Default::default()
        };
        if s.shadows && blocks.len() < max_blocks && has_heights(s) {
            // Like the directional light (E24): moving 1 toward the light moves the ground
            // point by (lx, ly) in the world and by m in the mask; the ray rises lz / |m| per
            // mask texel.
            let (lx, ly) = (s.to_light.x, s.to_light.y);
            let lz = det::max(s.to_light.z, MIN_SUN_Z);
            let mx = (lx * p.x_axis.x + ly * p.y_axis.x) * tps[0];
            let my = (lx * p.x_axis.y + ly * p.y_axis.y) * tps[1];
            let m = det::hypot(mx, my);
            let mut march = [0.0; 4];
            if m > 1e-6 {
                march = [mx / m, my / m, lz / m, 0.0];
            }
            blocks.push(SunBlock {
                sun: i as u32,
                casters: s.casters,
                march,
            });
            sp.color[3] = blocks.len() as f32;
        }
        params.push(sp);
    }
    block
}

/// Fills the atlas fields of every sun of a frame with blocks: the mask rows, the mask and
/// atlas sizes and the block size.
pub fn finish_suns(
    params: &mut [SunParams],
    rows: [[f32; 4]; 2],
    domain: &OcclusionDomain,
    atlas: (u32, u32),
    block: (u32, u32),
) {
    for sp in params {
        sp.mask_x = rows[0];
        sp.mask_y = rows[1];
        sp.atlas = [
            domain.width as f32,
            domain.height as f32,
            atlas.0 as f32,
            atlas.1 as f32,
        ];
        sp.block = [block.0 as f32, block.1 as f32, 0.0, 0.0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Quality, Resolutions};
    use knorke_math::{Vec2, Vec3};

    #[test]
    fn sizes_follow_chiaro() {
        assert_eq!(size_of::<PolarQueryParams>(), 48);
        assert_eq!(size_of::<SunParams>(), 96);
        assert_eq!(size_of::<MediumLightPassParams>(), 48);
        assert_eq!(size_of::<MediumMaskLightPassParams>(), 64);
        assert_eq!(size_of::<PolarRowParams>(), 64);
        assert_eq!(size_of::<SunHeightParams>(), 32);
    }

    /// Chiaro `MediumTests.cpp`: "the sun height map spans about two scatter texels per texel".
    #[test]
    fn sun_map_divisor_spans_two_scatter_texels() {
        assert_eq!(sun_map_divisor(0.5, 0.5), 2);
        assert_eq!(sun_map_divisor(1.0, 0.5), 4);
        assert_eq!(sun_map_divisor(0.75, 0.5), 3);
        assert_eq!(sun_map_divisor(0.5, 0.25), 4);
        assert_eq!(sun_map_divisor(1.0, 1.0), 2);
        assert_eq!(sun_map_divisor(0.25, 1.0), 1);
    }

    #[test]
    fn runs_with_haze_or_a_layer() {
        let mut s = LightSettings::default();
        assert!(!medium_runs(&s, false));
        assert!(medium_runs(&s, true));
        s.haze.density = 0.1;
        assert!(medium_runs(&s, false));
    }

    #[test]
    fn top_is_the_higher_of_layer_and_haze() {
        assert_eq!(medium_top(true, 2.0, 0.0, 10.0, 3.0), 510.0);
        assert_eq!(medium_top(false, 2.0, 0.1, 10.0, 3.0), 25.0);
        assert_eq!(medium_top(false, 2.0, 0.0, 10.0, 3.0), 0.0);
        assert_eq!(medium_top(true, 0.01, 0.1, 10.0, 3.0), 25.0);
    }

    fn shafts(casters: Casters) -> Option<Shafts> {
        Some(Shafts {
            casters,
            inside: false,
            key: 0,
        })
    }

    #[test]
    fn shaft_rows_skip_lights_without_a_field() {
        let entries = [
            shafts(Casters::All),
            None,
            shafts(Casters::Dynamic),
            shafts(Casters::Static),
        ];
        let mut rows = Vec::new();
        plan_shafts(&entries, |s| s.casters != Casters::Dynamic, &mut rows);
        assert_eq!(rows.iter().map(|r| r.light).collect::<Vec<_>>(), [0, 3]);
    }

    #[test]
    fn polar_row_marches_the_stretched_radius() {
        let q = PolarQueryParams {
            mask_x: [3.0, 0.0, 1.0, 0.0],
            mask_y: [0.0, 4.0, 1.0, 0.0],
            ..Default::default()
        };
        let light = GpuLight {
            position_radius: [1.0, 2.0, 3.0, 2.0],
            ..Default::default()
        };
        assert_eq!(polar_row(&q, &light, false), [1.0, 2.0, 3.0, 10.0]);
        assert_eq!(polar_row(&q, &light, true)[3], -10.0);
    }

    fn sun(z: f32, shadows: bool, casters: Casters) -> MediumSun {
        MediumSun {
            color: Vec3::new(1.0, 0.5, 0.25),
            to_light: Vec3::new(0.6, 0.0, z),
            shadows,
            casters,
            key: 0,
        }
    }

    #[test]
    fn suns_get_blocks_with_shadows_and_heights_only() {
        let p = Projection::default();
        let mut settings = LightSettings::default();
        settings.quality.quality = Quality::Custom(Resolutions {
            light: 1.0,
            shadow: 0.5,
            medium: 0.5,
        });
        let domain = OcclusionDomain::new(96, 48, 0.5);
        let s = SunShadows {
            domain,
            scene_max_top: 8.0,
        };
        let suns = [
            sun(0.8, true, Casters::All),
            sun(0.8, false, Casters::All),
            sun(0.01, true, Casters::Static),
            sun(0.8, true, Casters::Dynamic),
        ];
        let (mut params, mut blocks) = (Vec::new(), Vec::new());
        let block = plan_suns(
            &suns,
            &p,
            &settings,
            Some(&s),
            true,
            |s| s.casters != Casters::Dynamic,
            &mut params,
            &mut blocks,
        );
        // Divisor 2: the domain (96 + 48) x (48 + 48) at 0.5 is 72 x 48 texels.
        assert_eq!(block, (36, 24));
        assert_eq!(params.len(), 4);
        assert_eq!(
            params.iter().map(|p| p.color[3]).collect::<Vec<_>>(),
            [1.0, 0.0, 2.0, 0.0]
        );
        assert_eq!(params[0].to_light, [0.6, 0.0, 0.8, 8.0]);
        // Identity projection at 0.5: m = 0.3, a unit step in x; the low sun rises 0.05.
        assert_eq!(blocks[0].march, [1.0, 0.0, 0.8 / 0.3, 0.0]);
        assert_eq!(blocks[1].march[2], 0.05 / 0.3);
        finish_suns(
            &mut params,
            mask_rows(&p, &domain, [0.0; 2]),
            &domain,
            (36, 48),
            block,
        );
        assert_eq!(params[3].atlas, [72.0, 48.0, 36.0, 48.0]);
        assert_eq!(params[3].mask_x, [0.5, 0.0, 12.0, 0.0]);
        // Without shafts no blocks, the top stays; without shadows it is 0.
        plan_suns(
            &suns,
            &p,
            &settings,
            Some(&s),
            false,
            |_| true,
            &mut params,
            &mut blocks,
        );
        assert!(blocks.is_empty());
        assert_eq!(params[0].to_light[3], 8.0);
        plan_suns(
            &suns,
            &p,
            &settings,
            None,
            false,
            |_| true,
            &mut params,
            &mut blocks,
        );
        assert!(blocks.is_empty());
        assert!(
            params
                .iter()
                .all(|p| p.color[3] == 0.0 && p.to_light[3] == 0.0)
        );
    }

    #[test]
    fn sun_blocks_stop_at_the_atlas_height() {
        let p = Projection::default();
        let settings = LightSettings::default();
        // A domain 3000 texels tall at divisor 4: blocks of 750, at most 10 in 8192.
        let domain = OcclusionDomain::new(2000, 2000, 1.0);
        let s = SunShadows {
            domain,
            scene_max_top: 1.0,
        };
        let suns = [sun(0.8, true, Casters::All); 12];
        let (mut params, mut blocks) = (Vec::new(), Vec::new());
        let block = plan_suns(
            &suns,
            &p,
            &settings,
            Some(&s),
            true,
            |_| true,
            &mut params,
            &mut blocks,
        );
        assert_eq!(blocks.len(), (MAX_SUN_ATLAS_HEIGHT / block.1) as usize);
        assert_eq!(params[11].color[3], 0.0);
    }

    #[test]
    fn mask_rows_follow_the_anchor() {
        let p = Projection {
            origin: knorke_math::At::new(Vec2::new(10.0, 4.0)),
            x_axis: Vec2::new(2.0, 1.0),
            y_axis: Vec2::new(-2.0, 1.0),
            z_axis: Vec2::new(0.0, -1.0),
        };
        let d = OcclusionDomain::new(40, 20, 0.5);
        let rows = mask_rows(&p, &d, [0.25, 0.5]);
        assert_eq!(rows[0], [1.0, -1.0, (10.0 + 10.0) * 0.5 - 0.25, 0.0]);
        assert_eq!(rows[1], [0.5, 0.5, (4.0 + 10.0) * 0.5 - 0.5, 0.0]);
    }
}
