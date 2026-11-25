// Origin: chiaro @ 9bc5b9b, src/lighting/MediumCulling.hpp and MediumCulling.cpp
// (CollectMediumLights, CollectMediumSuns, MediumSun, kNoShafts, kShaftsInside).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The lights of the medium stage (P12): which lights reach the media between the ground and the
//! highest medium, and what the stage reads of them.
//!
//! A point, spot, line or rect light with `haze > 0` is collected if the sphere of its radius (around
//! every emitter point for line and rect lights), cut to the heights `[0, medium_top]`, can project
//! into the scene rectangle; a light more than its radius above `medium_top` reaches no medium.
//! Animation and flicker apply as for the ground ([`animate_light`], same `time` and `modes`); the
//! haze factor is multiplied into the colour. The [`GpuLight`] copies carry the horizontal half
//! extent of the reach (the ground radius of the light pass) in `spot_cos[2]` and its height range
//! `[bottom, top]` in `area_mask[0..2]`; `shadow[1]` and `area_mask[2..4]` are zero. Directional
//! lights reach every column and are [`MediumSun`]s.
//!
//! Redesign against the C++ version:
//!
//! - The five out parameters are one reusable [`MediumLightList`]; the shaft entries are
//!   `Option<`[`Shafts`]`>` instead of a byte (`kNoShafts`, `kShaftsInside`, `kCasters*`).
//! - Masked lights (E115) are always collected into [`MediumLightList::masked`] when the mask
//!   store is in `modes`; without it they are dropped (never drawn unmasked), as in C++.
//! - Everything reuses the vectors' capacity: after the first frames collecting allocates nothing.

use crate::gpu_light::GpuLight;
use crate::light::{LightRef, LightType};
use crate::light_culling::{
    Casters, LightModes, MaskedLights, animate_light, area_extent_of, casters_of, light_height,
    make_gpu_light,
};
use crate::light_mask::{make_gpu_mask, make_mask_placement};
use crate::light_math::{SceneQuad, ground_radius, quad_intersects_scene};
use crate::mask::mask_atlas_blocks;
use crate::{LightStore, Projection};
use knorke_math::{At, Screen, Vec2, Vec3, det};

/// A light with light shafts: the medium stage traces its shadow ray through the media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shafts {
    /// Which obstacles cast the shafts (E53).
    pub casters: Casters,
    /// The light sits inside an obstacle (E147).
    pub inside: bool,
}

/// The lights of one medium stage, the ground-side lists parallel to the shaft entries.
#[derive(Clone, Debug, Default)]
pub struct MediumLightList {
    /// The lights without a mask, in store order.
    pub lights: Vec<GpuLight>,
    /// One entry per light: `Some` for point and spot lights with shadows (the lights that can have
    /// light shafts); `None` for the others (no shadows, line and rect lights: a polar map around
    /// their middle is wrong, E33).
    pub shafts: Vec<Option<Shafts>>,
    /// The masked lights (E115) with their masks, placed as in the ground culling.
    pub masked: MaskedLights,
    /// One entry per masked light; a window light gets `None` (the polar map around the lamp does
    /// not know where the ray passes the window).
    pub masked_shafts: Vec<Option<Shafts>>,
    /// Enabled non-directional lights with `haze == 0`: they cost nothing else.
    pub skipped_haze: u32,
}

impl MediumLightList {
    fn reset(&mut self) {
        self.lights.clear();
        self.shafts.clear();
        self.masked.clear();
        self.masked_shafts.clear();
        self.skipped_haze = 0;
    }
}

/// Fills `out` with the lights of the medium stage (see the module doc). `medium_top` is the
/// highest height of any medium; `scene_w`, `scene_h` the scene rectangle.
#[allow(clippy::too_many_arguments)]
pub fn collect_medium_lights(
    lights: &LightStore,
    projection: &Projection,
    scene_w: u32,
    scene_h: u32,
    medium_top: f32,
    time: f64,
    modes: &LightModes<'_>,
    out: &mut MediumLightList,
) {
    out.reset();
    let mask_blocks = modes
        .masks
        .map_or(0, |m| mask_atlas_blocks(m.block_count()));
    for (light_ref, original) in lights.iter() {
        let d = &original.desc;
        // Directional lights reach the medium in their own pass (`collect_medium_suns`).
        if !d.enabled || d.kind == LightType::Directional {
            continue;
        }
        if d.haze <= 0.0 {
            out.skipped_haze += 1; // no animation sample, no culling, no draw
            continue;
        }
        let mask_block = match (d.mask, modes.masks) {
            (Some(mask), Some(masks)) => match masks.block_of(mask) {
                Some(block) => Some(block),
                None => continue, // a masked light without the mask to place it
            },
            (Some(_), None) => continue,
            (None, _) => None,
        };
        let Some(animated) = animate_light(
            LightRef::seed_index(light_ref),
            original,
            time,
            modes.animations,
        ) else {
            continue; // dark this frame
        };
        let stored = &*animated.stored;
        // The reach: the sphere of the radius around the light (around every emitter point of a
        // line or rect), between the ground and the highest medium.
        let z = light_height(d, modes.normal_mode);
        let extent = area_extent_of(stored);
        let (low, high) = (z - extent.z, z + extent.z);
        let bottom = det::max(low - d.radius, 0.0);
        let top = det::min(high + d.radius, medium_top);
        if !(bottom < top) {
            continue; // entirely above the media (or below the ground)
        }
        // Horizontal half extent: the sphere's radius at the height of [bottom, top] nearest to
        // the emitter.
        let dz = if low > top {
            low - top
        } else if high < bottom {
            bottom - high
        } else {
            0.0
        };
        let half = ground_radius(dz, d.radius) + det::max(extent.x, extent.y);
        let pos = d.position.raw();
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (1e30f32, 1e30f32, -1e30f32, -1e30f32);
        for k in 0..8u32 {
            let c = projection.project(
                At::new(Vec2::new(
                    pos.x + if k & 1 != 0 { half } else { -half },
                    pos.y + if k & 2 != 0 { half } else { -half },
                )),
                if k & 4 != 0 { top } else { bottom },
            );
            let c = c.raw();
            min_x = det::min(min_x, c.x);
            max_x = det::max(max_x, c.x);
            min_y = det::min(min_y, c.y);
            max_y = det::max(max_y, c.y);
        }
        let corner = |x: f32, y: f32| At::<Screen, Vec2>::new(Vec2::new(x, y));
        let hull = SceneQuad {
            corner: [
                corner(min_x, min_y),
                corner(max_x, min_y),
                corner(min_x, max_y),
                corner(max_x, max_y),
            ],
        };
        if !quad_intersects_scene(&hull, scene_w, scene_h) {
            continue;
        }
        // No area normalization (E51): it stands in for the receiver's Lambert term, which
        // scattering lacks.
        let (mut g, _) = make_gpu_light(&animated, z, modes.profiles);
        for c in &mut g.color_falloff[..3] {
            *c *= d.haze;
        }
        g.spot_cos[2] = half;
        g.shadow[1] = 0.0; // no highlights in the medium
        g.area_mask[0] = bottom;
        g.area_mask[1] = top;
        let can_have_shafts =
            d.cast_shadows && matches!(d.kind, LightType::Point | LightType::Spot);
        let shafts = Shafts {
            casters: casters_of(d),
            inside: d.inside_obstacle,
        };
        if let Some(block) = mask_block {
            let Some(placement) = make_mask_placement(stored, z) else {
                continue; // in its window's plane: lights nothing
            };
            out.masked
                .push(g, make_gpu_mask(&placement, block, mask_blocks));
            out.masked_shafts
                .push((can_have_shafts && !placement.window).then_some(shafts));
            continue;
        }
        out.lights.push(g);
        out.shafts.push(can_have_shafts.then_some(shafts));
    }
}

/// A directional light in the medium (one fullscreen draw of `medium_directional.frag`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediumSun {
    /// Colour * intensity (animation and flicker applied) * haze factor.
    pub color: Vec3,
    /// Unit vector toward the light, world space.
    pub to_light: Vec3,
    /// `cast_shadows` in height mode (E24): it can have a sun shadow height map.
    pub shadows: bool,
    /// Which obstacles cast its shadow (E53); [`Casters::All`] without `shadows`.
    pub casters: Casters,
}

/// Fills `out` with every enabled directional light with `haze > 0` that its animation does not
/// darken this frame, in store order.
pub fn collect_medium_suns(
    lights: &LightStore,
    time: f64,
    modes: &LightModes<'_>,
    out: &mut Vec<MediumSun>,
) {
    out.clear();
    for (light_ref, original) in lights.iter() {
        let d = &original.desc;
        if !d.enabled || d.kind != LightType::Directional || d.haze <= 0.0 {
            continue;
        }
        let Some(animated) = animate_light(
            LightRef::seed_index(light_ref),
            original,
            time,
            modes.animations,
        ) else {
            continue;
        };
        let shadows = d.cast_shadows && modes.height_mode;
        out.push(MediumSun {
            // (colour * strength) * haze in this order: the same rounding as Chiaro.
            color: Vec3::new(
                animated.color.x * animated.strength * d.haze,
                animated.color.y * animated.strength * d.haze,
                animated.color.z * animated.strength * d.haze,
            ),
            to_light: -animated.stored.direction,
            shadows,
            casters: if shadows { casters_of(d) } else { Casters::All },
        });
    }
}
