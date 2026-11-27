// Origin: chiaro @ 9bc5b9b, src/lighting/LightCulling.hpp and LightCulling.cpp (CollectLights,
// AnimateLight, MakeGpuLight, LightHeight, AreaExtentOf, CastersOf).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The lights of one frame, culled and with animation and flicker applied (P8, P10 for the
//! masked lights).
//!
//! Redesign against the C++ version:
//!
//! - Lights come from a [`LightStore`] (persistent first, then the frame's transient lights);
//!   [`LightRef::seed_index`] seeds flicker and fire as the slot index did.
//! - Settings are [`LightSettings`]; the culling reads the ambient colour and the shadow
//!   resolution (P19).
//! - The profile store (P9) is reached through the small trait [`ProfileAtlas`]: the culling needs
//!   the atlas row of a profile and the inverse atlas height, nothing else.
//! - The pointers of `LightModes` to the animation, mask and profile stores are optional
//!   references; without the animation store animations are ignored, without the mask store
//!   masked lights are dropped (never drawn unmasked).
//! - A beacon turns a copy of the stored light; the copy is a [`Cow`] that only becomes
//!   owned when the light rotates, so no heap allocation happens either way.
//! - Results are a [`LightList`] of plain `repr(C)` records ([`GpuLight`] and friends) in vectors
//!   whose capacity is reused: after the first frames of a given size, collecting allocates
//!   nothing. All maths that influences results uses [`knorke_math::det`].
//! - The shadow parameters live in the occlusion mask's texel space ([`crate::shadow`], P16).

use crate::animation::{AnimationStore, rotate_about_z};
use crate::area_light::{AreaEmitter, AreaKind, area_normalization, rect_across};
use crate::gpu_light::{GpuDirectional, GpuLight, GpuMask};
use crate::light::{
    Height, LightDesc, LightRef, LightType, MaskMode, ProfileId, ShadowFilter, StoredLight,
};
use crate::light_mask::{make_gpu_mask, make_mask_placement};
use crate::light_math::{
    NEAR_FIELD_SHARE, box_bounds, flicker_noise, ground_radius, light_quad, quad_intersects_scene,
    rect_intersects_scene,
};
use crate::mask::mask_atlas_blocks;
use crate::mask_store::MaskStore;
use crate::rows::{MAX_ROWS, RowLight, RowSpace, light_suits, row_key, row_span};
use crate::settings::LightSettings;
use crate::shadow::{MaskAnchor, OcclusionDomain};
use crate::{LightStore, Projection};
use knorke_math::{Vec3, det};
use std::borrow::Cow;

use crate::light_math::DEG_TO_RAD;

/// `Height::Auto` with normals (surface or material buffer): this factor on the radius (E39).
pub const AUTO_HEIGHT_SCALE: f32 = 0.4;

/// What the profile store (P9) answers to the culling.
pub trait ProfileAtlas {
    /// The first atlas row of a live profile (`block * azimuth steps`), `None` for any other handle.
    fn atlas_row(&self, id: ProfileId) -> Option<f32>;
    /// `1 / atlas height in texels`, 0 without profiles.
    fn inv_atlas_height(&self) -> f32;
}

/// What the frame offers beyond phase 3, deciding where directional lights go.
#[derive(Clone, Copy, Default)]
pub struct LightModes<'a> {
    /// An enabled obstacle has a finite height (E22); only then can directional lights be blocked.
    pub height_mode: bool,
    /// A surface or material buffer is given.
    pub normal_mode: bool,
    /// Normal mode: highest height a lit pixel is assumed to have.
    pub max_height: f32,
    /// The frame builds a medium world map: every directional light is drawn per pixel, so
    /// clouds shade it.
    pub medium_map: bool,
    /// Resolves a light's profile to atlas rows; `None`: no profiles.
    pub profiles: Option<&'a dyn ProfileAtlas>,
    /// Evaluates a light's animation; `None`: no animations.
    pub animations: Option<&'a AnimationStore>,
    /// Resolves a light's mask to atlas blocks; `None`: masked lights are dropped.
    pub masks: Option<&'a MaskStore>,
}

impl core::fmt::Debug for LightModes<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LightModes")
            .field("height_mode", &self.height_mode)
            .field("normal_mode", &self.normal_mode)
            .field("max_height", &self.max_height)
            .field("medium_map", &self.medium_map)
            .field("profiles", &self.profiles.is_some())
            .field("animations", &self.animations.is_some())
            .field("masks", &self.masks.is_some())
            .finish()
    }
}

/// Which obstacles cast a light's shadow (E53).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Casters {
    /// Every caster.
    #[default]
    All,
    /// Only static shapes, tile grids and the occlusion layer.
    Static,
    /// Only obstacles marked dynamic.
    Dynamic,
}

impl Casters {
    /// Index of the caster class (`[Static, Dynamic]`); `None` for [`All`](Self::All).
    pub fn filter_index(self) -> Option<usize> {
        match self {
            Self::All => None,
            Self::Static => Some(0),
            Self::Dynamic => Some(1),
        }
    }

    /// The bit of this filter in [`LightList::caster_classes`]; 0 for [`All`](Self::All).
    pub fn class_bit(self) -> u32 {
        self.filter_index().map_or(0, |i| 1 << i)
    }
}

/// The obstacles that cast the light's shadow (E53): [`Casters::All`] without a filter.
pub fn casters_of(d: &LightDesc) -> Casters {
    match d.shadow_filter {
        ShadowFilter::All => Casters::All,
        ShadowFilter::StaticOnly => Casters::Static,
        ShadowFilter::DynamicOnly => Casters::Dynamic,
    }
}

/// Masked point/spot lights of one kind of draw with their masks, index for index (E115).
#[derive(Clone, Debug, Default)]
pub struct MaskedLights {
    /// The lights.
    pub lights: Vec<GpuLight>,
    /// Their masks, same index.
    pub masks: Vec<GpuMask>,
}

impl MaskedLights {
    /// Empties both lists, keeping their capacity.
    pub fn clear(&mut self) {
        self.lights.clear();
        self.masks.clear();
    }

    /// Number of masked lights.
    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// `true` if there are none.
    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    pub(crate) fn push(&mut self, light: GpuLight, mask: GpuMask) {
        self.lights.push(light);
        self.masks.push(mask);
    }
}

/// The lights of one frame, culled and with animation and flicker applied.
#[derive(Clone, Debug, Default)]
pub struct LightList {
    /// Directional lights drawn per pixel (E23, E24), in store order.
    pub directional: Vec<GpuDirectional>,
    /// Lights in `directional` that cast shadows (`color[3] == 1`).
    pub directional_shadows: u32,
    /// Shadow caster filter (E53) of each entry of `directional`.
    pub directional_casters: Vec<Casters>,
    /// Of each entry of `directional`: half the spread of the heights its disc's rays gain per
    /// mask texel of way, from the disc's top to its bottom (0 without a source radius; a sun
    /// map's soft shadow, [`crate::rows::SunParams`]).
    pub directional_rise: Vec<f32>,
    /// Of each entry of `directional`: its [`row_key`] (the medium stage finds its sun map by it).
    pub directional_keys: Vec<u64>,
    /// Every visible point, spot, line and rect light in store order, drawn by the row programs
    /// (ADR 0019).
    pub rows: Vec<RowLight>,
    /// Atlas slots the entries of `rows` need ([`RowLight::slots`]); 0: none casts shadows.
    pub row_shadows: u32,
    /// Shadow-casting lights drawn without shadow: beyond [`crate::rows::WIDE_REACH`] or the
    /// atlas's [`MAX_ROWS`] (warning `unshadowed_lights`).
    pub shadowless_rows: u32,
    /// The masked entries of `rows` (they read the mask atlas).
    pub masked_lights: u32,
    /// `mask_atlas_blocks` of the mask store when the list was made (the `GpuMask` atlas v).
    pub mask_blocks: u32,
    /// Bit [`Casters::class_bit`] for every filter a shadowed visible light uses.
    pub caster_classes: u32,
    /// Visible point/spot lights with a profile.
    pub profiled_lights: u32,
    /// Normal mode: visible point/spot lights with an explicit height `<= 0` (they only graze
    /// the ground).
    pub flat_lights: u32,
    /// Enabled point/spot/line/rect lights too high to reach any pixel.
    pub unreachable_lights: u32,
    /// One-sided rect lights facing away from all they could reach (E51), not drawn.
    pub facing_away_lights: u32,
    /// Visible point/spot/line/rect lights with `Height::Auto`.
    pub auto_height_lights: u32,
    /// Ambient plus every enabled directional light that is not in `directional`.
    pub ambient: Vec3,
    /// The directional part of `ambient` alone (the flat directional sum), summed beside it, not
    /// as a difference.
    pub directional_ambient: Vec3,
}

impl LightList {
    fn reset(&mut self) {
        self.directional.clear();
        self.directional_shadows = 0;
        self.directional_casters.clear();
        self.directional_rise.clear();
        self.directional_keys.clear();
        self.rows.clear();
        self.row_shadows = 0;
        self.shadowless_rows = 0;
        self.masked_lights = 0;
        self.caster_classes = 0;
        self.profiled_lights = 0;
        self.flat_lights = 0;
        self.unreachable_lights = 0;
        self.facing_away_lights = 0;
        self.auto_height_lights = 0;
        self.directional_ambient = Vec3::ZERO;
    }
}

/// A light of one frame with animation and flicker applied.
#[derive(Clone, Debug)]
pub struct AnimatedLight<'a> {
    /// The stored light, or the copy turned by a beacon.
    pub stored: Cow<'a, StoredLight>,
    /// Description colour times the animation's colour.
    pub color: Vec3,
    /// Intensity times the animation's intensity and the flicker.
    pub strength: f32,
}

/// Applies the light's animation at `time` and its flicker. `seed` is the light's slot index
/// ([`LightRef::seed_index`]). `None` if the animation darkens the light this frame (a flash
/// between blinks, an envelope before or after it): it is not drawn.
pub fn animate_light<'a>(
    seed: u32,
    original: &'a StoredLight,
    time: f64,
    animations: Option<&AnimationStore>,
) -> Option<AnimatedLight<'a>> {
    let d = &original.desc;
    // Without an animation `color` and `strength` are exactly the desc values (bit-identical to
    // no animation).
    let mut out = AnimatedLight {
        stored: Cow::Borrowed(original),
        color: d.color,
        strength: d.intensity,
    };
    if let (Some(id), Some(animations)) = (d.animation, animations) {
        let a = animations.sample(Some(id), time, d.animation_offset, seed);
        out.strength *= a.intensity;
        out.color = Vec3::new(
            out.color.x * a.color.x,
            out.color.y * a.color.y,
            out.color.z * a.color.z,
        );
        let peak = det::max(det::max(out.color.x, out.color.y), out.color.z);
        if out.strength * peak <= 0.0 {
            return None;
        }
        if a.rotation != 0.0 {
            let mut turned = original.clone();
            turned.direction = rotate_about_z(turned.direction, a.rotation);
            turned.reference = rotate_about_z(turned.reference, a.rotation);
            turned.axis = rotate_about_z(turned.axis, a.rotation);
            // A projector mask turns with the beacon.
            turned.mask_reference = rotate_about_z(turned.mask_reference, a.rotation);
            out.stored = Cow::Owned(turned);
        }
    }
    if d.flicker > 0.0 {
        out.strength *= 1.0 - d.flicker * flicker_noise(time, d.flicker_speed, seed);
    }
    Some(out)
}

/// The light's height this frame: [`Height::Auto`] (E39) is 0 without normals and
/// [`AUTO_HEIGHT_SCALE`] x radius with.
pub fn light_height(d: &LightDesc, normal_mode: bool) -> f32 {
    match d.height {
        Height::Auto if normal_mode => AUTO_HEIGHT_SCALE * d.radius,
        Height::Auto => 0.0,
        Height::At(h) => h,
    }
}

/// A line or rect light's emitter extent: the rect's half extent across and the emitter's half
/// box per axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AreaExtent {
    /// Rect lights: [`rect_across`]; zero for lines.
    pub across: Vec3,
    /// Half box of the emitter along x.
    pub x: f32,
    /// Half box of the emitter along y.
    pub y: f32,
    /// Half box of the emitter along z.
    pub z: f32,
}

/// Zero for point and spot lights.
pub fn area_extent_of(stored: &StoredLight) -> AreaExtent {
    let d = &stored.desc;
    if !d.kind.is_area() {
        return AreaExtent::default();
    }
    let u = stored.axis;
    let across = if d.kind == LightType::Rect {
        rect_across(stored.direction, u, d.half_width)
    } else {
        Vec3::ZERO
    };
    AreaExtent {
        across,
        x: u.x.abs() + across.x.abs(),
        y: u.y.abs() + across.y.abs(),
        z: u.z.abs() + across.z.abs(),
    }
}

/// The `GpuLight` fields every light pass reads: position (height `z`), colour (colour *
/// strength), falloff, direction and type (with the soft near field share), cone, profile, area
/// axis and half width, specular factor (`shadow[1]`). `spot_cos[2]`, `shadow[0, 2, 3]` and
/// `area_mask` are left zero for the caller. The flag is `true` if a profile was found.
pub fn make_gpu_light(
    light: &AnimatedLight<'_>,
    z: f32,
    profiles: Option<&dyn ProfileAtlas>,
) -> (GpuLight, bool) {
    let stored = &*light.stored;
    let d = &stored.desc;
    let area = d.kind.is_area();
    let pos = d.position.raw();
    let mut g = GpuLight {
        position_radius: [pos.x, pos.y, z, d.radius],
        color_falloff: [
            light.color.x * light.strength,
            light.color.y * light.strength,
            light.color.z * light.strength,
            d.falloff,
        ],
        direction_type: [
            stored.direction.x,
            stored.direction.y,
            stored.direction.z,
            match d.kind {
                LightType::Spot => 1.0,
                LightType::Line => 2.0,
                LightType::Rect if d.two_sided => 4.0,
                LightType::Rect => 3.0,
                LightType::Point | LightType::Directional => 0.0,
            },
        ],
        spot_cos: [stored.cos_inner, stored.cos_outer, 0.0, 0.0],
        profile: [-1.0, 0.0, 0.0, 0.0],
        // Every light; the vertex shaders pass it on as spot_cos.z (E47).
        shadow: [0.0, d.specular, 0.0, 0.0],
        ..GpuLight::default()
    };
    if !area && d.soft_near_field && d.source_radius > 0.0 {
        // Soft near field: the share of the source radius rides on the type (NearFieldRadius in
        // light_common.hlsli).
        g.direction_type[3] += NEAR_FIELD_SHARE * det::min(d.source_radius / d.radius, 1.0);
    }
    if area {
        let (a, w) = (
            stored.axis,
            if d.kind == LightType::Rect {
                d.half_width
            } else {
                0.0
            },
        );
        g.area = [a.x, a.y, a.z, w];
    }
    let row = match (d.profile, profiles) {
        (Some(id), Some(p)) => p.atlas_row(id),
        _ => None,
    };
    if let (Some(row), Some(p)) = (row, profiles) {
        g.spot_cos[3] = p.inv_atlas_height();
        g.profile = [
            row,
            stored.reference.x,
            stored.reference.y,
            stored.reference.z,
        ];
    }
    (g, row.is_some())
}

/// Per-pixel data of a directional light (E24). Moving a distance `s` along the unit vector `L`
/// toward the light moves the ground point by `s * (Lx, Ly)` in the world and by `s * m` in the
/// mask; the ray in the mask therefore rises by `Lz / |m|` per texel, and its penumbra widens by
/// `tan(angle) * texels_per_world / |m|` per texel. Also the rise of `LightList::directional_rise`:
/// tilting `L` up or down by the angle keeps its direction on the ground, so the ray then rises by
/// `tan(elevation) * hypot(Lx, Ly) / |m|` per texel at that elevation.
fn make_directional(
    stored: &StoredLight,
    color: Vec3,
    strength: f32,
    p: &Projection,
    domain: &OcclusionDomain,
    texels_per_world: f32,
) -> (GpuDirectional, f32) {
    let d = &stored.desc;
    let (lx, ly, lz) = (
        -stored.direction.x,
        -stored.direction.y,
        -stored.direction.z,
    );
    let mx = (lx * p.x_axis.x + ly * p.y_axis.x) * domain.texels_per_scene[0];
    let my = (lx * p.x_axis.y + ly * p.y_axis.y) * domain.texels_per_scene[1];
    let m = det::hypot(mx, my);
    let mut g = GpuDirectional::default();
    let mut rise = 0.0;
    if m > 1e-6 {
        let angle = d.source_radius * DEG_TO_RAD;
        g.dir_slope = [
            mx / m,
            my / m,
            lz / m,
            det::tan(angle) * texels_per_world / m,
        ];
        let ground = det::hypot(lx, ly);
        let elevation = det::atan2(lz, ground);
        // Up to just below the zenith: beyond it the disc's top would shine from behind.
        let top = det::tan((elevation + angle).clamp(-1.5, 1.5));
        let bottom = det::tan((elevation - angle).clamp(-1.5, 1.5));
        rise = 0.5 * (top - bottom) * ground / m;
    }
    g.color = [
        color.x * strength,
        color.y * strength,
        color.z * strength,
        0.0, // set by the caller
    ];
    g.world = [lx, ly, lz, d.specular];
    (g, rise)
}

/// Fills `out` from the enabled lights: directional lights go into `directional` in normal mode
/// (all of them, for their Lambert term), with a medium world map (all of them, for the clouds'
/// shade) and in height mode (those that cast shadows), the others into the ambient sum; point,
/// spot, line and rect lights whose ground disc can reach the scene rectangle into `rows` (store
/// order). Reuses the vectors' capacity.
#[allow(clippy::too_many_arguments)]
pub fn collect_lights(
    lights: &LightStore,
    settings: &LightSettings,
    projection: &Projection,
    time: f64,
    scene_w: u32,
    scene_h: u32,
    modes: &LightModes<'_>,
    out: &mut LightList,
) {
    out.reset();
    out.ambient = settings.ambient;
    out.mask_blocks = modes
        .masks
        .map_or(0, |m| mask_atlas_blocks(m.block_count()));

    // Shadow parameters live in the occlusion mask's texel space (same domain as the mask).
    let shadow_scale = settings.resolutions().shadow;
    let domain = OcclusionDomain::new(scene_w, scene_h, shadow_scale);
    // Must equal the occlusion stage's anchor: same projection, scene size and shadow scale.
    let anchor = MaskAnchor::compute(projection, &domain);
    let axis_scale = (det::hypot(projection.x_axis.x, projection.x_axis.y)
        + det::hypot(projection.y_axis.x, projection.y_axis.y))
        * 0.5;
    let [tx, ty] = domain.texels_per_scene;
    let row_space = RowSpace::new(projection, &domain);

    for (light_ref, original) in lights.iter() {
        let d = &original.desc;
        if !d.enabled {
            continue;
        }
        let Some(mut animated) = animate_light(
            LightRef::seed_index(light_ref),
            original,
            time,
            modes.animations,
        ) else {
            continue; // dark this frame
        };
        let stored: &StoredLight = &animated.stored;
        let color = animated.color;
        let mut strength = animated.strength;
        if d.kind == LightType::Directional {
            let shadows = d.cast_shadows && modes.height_mode;
            if shadows || modes.normal_mode || modes.medium_map {
                let (mut g, rise) = make_directional(
                    stored,
                    color,
                    strength,
                    projection,
                    &domain,
                    axis_scale * shadow_scale,
                );
                g.color[3] = if shadows { 1.0 } else { 0.0 };
                out.directional.push(g);
                out.directional_rise.push(rise);
                out.directional_keys.push(row_key(light_ref));
                let casters = if shadows { casters_of(d) } else { Casters::All };
                out.directional_casters.push(casters);
                out.caster_classes |= casters.class_bit();
                out.directional_shadows += u32::from(shadows);
                continue;
            }
            let lit = Vec3::new(color.x * strength, color.y * strength, color.z * strength);
            out.ambient += lit;
            out.directional_ambient += lit;
            continue;
        }
        // Only the ground disc of radius sqrt(r^2 - z^2) can receive light. With a surface
        // buffer (E23) raised pixels can be up to r away horizontally and land up to
        // `max_height * z_axis` higher in the scene. Line and rect lights: the same around every
        // point of the emitter, so the quad grows by the emitter's horizontal half extent and the
        // height is the emitter's nearest to the ground.
        let area = d.kind.is_area();
        // Height::Auto (E39): 0 without normals, 0.4 x radius with them.
        let z = light_height(d, modes.normal_mode);
        let extent = area_extent_of(stored);
        let across = extent.across;
        let (extent_x, extent_y) = (extent.x, extent.y);
        let (mut low, mut high) = (z, z);
        if area {
            low = z - extent.z;
            high = z + extent.z;
            let pos = d.position.raw();
            let e = AreaEmitter {
                kind: match d.kind {
                    LightType::Line => AreaKind::Line,
                    _ if d.two_sided => AreaKind::RectTwoSided,
                    _ => AreaKind::Rect,
                },
                center: Vec3::new(pos.x, pos.y, z),
                u: stored.axis,
                v: across,
                normal: stored.direction,
            };
            if e.kind == AreaKind::Rect {
                // Facing away from everything it could reach (E51): the box of its reach above
                // the ground up to the highest surface lies wholly behind the emitting side,
                // e.g. a panel facing the sky in an iso view.
                let reach_xy = d.radius + det::max(extent_x, extent_y);
                let top = if modes.normal_mode {
                    modes.max_height
                } else {
                    0.0
                };
                let mut front = false;
                for k in 0..8u32 {
                    if front {
                        break;
                    }
                    let c = Vec3::new(
                        pos.x + if k & 1 != 0 { reach_xy } else { -reach_xy },
                        pos.y + if k & 2 != 0 { reach_xy } else { -reach_xy },
                        if k & 4 != 0 { top } else { 0.0 },
                    );
                    front = (c - e.center).dot(e.normal) > 0.0;
                }
                if !front {
                    out.facing_away_lights += 1;
                    continue;
                }
            }
            if modes.normal_mode {
                // As bright in front as a point light (E51).
                strength *= area_normalization(&e, d.radius);
            }
        }
        let mut ground = ground_radius(det::min(det::max(0.0, low), high), d.radius);
        if modes.normal_mode {
            ground = if low - modes.max_height < d.radius {
                d.radius
            } else {
                0.0
            };
        }
        if ground <= 0.0 {
            out.unreachable_lights += 1;
            continue;
        }
        if area {
            ground += det::max(extent_x, extent_y);
        }
        let visible = if modes.normal_mode {
            // The ground quad and the quad raised by max_height.
            let c = d.position.raw();
            let hull = box_bounds(
                projection,
                [c.x - ground, c.y - ground, 0.0],
                [c.x + ground, c.y + ground, modes.max_height],
            );
            rect_intersects_scene(hull, scene_w, scene_h)
        } else {
            quad_intersects_scene(
                &light_quad(projection, d.position, ground),
                scene_w,
                scene_h,
            )
        };
        if !visible {
            continue;
        }
        // A masked light (E115): its placement this frame, only for lights that reach the scene.
        // A light in its window's plane lights nothing and counts like an invisible light (in no
        // counter).
        let mut gpu_mask = None;
        if let Some(mask) = d.mask {
            let block = modes.masks.and_then(|m| m.block_of(mask));
            let placement = block.and_then(|b| {
                make_mask_placement(stored, z).map(|p| make_gpu_mask(&p, b, out.mask_blocks))
            });
            match placement {
                Some(g) => gpu_mask = Some(g),
                None => continue,
            }
            out.masked_lights += 1;
        }
        let auto_height = d.height == Height::Auto;
        if modes.normal_mode && !auto_height && z <= 0.0 {
            out.flat_lights += 1;
        }
        out.auto_height_lights += u32::from(auto_height);
        animated.strength = strength; // with the area normalization (E51)
        let (mut g, profiled) = make_gpu_light(&animated, z, modes.profiles);
        let stored: &StoredLight = &animated.stored;
        g.spot_cos[2] = ground; // half extent of the quad
        out.profiled_lights += u32::from(profiled);
        let mut window = [[0.0; 4]; 2];
        if d.cast_shadows {
            // The ray runs in the mask (ground plane) from the pixel to the ground point of the
            // light; with heights (E22) it rises to the height of the light, `position_radius[2]`,
            // on the way.
            let mask = domain.scene_to_domain(projection.project(d.position, 0.0).raw());
            g.shadow[2] = mask.x - anchor.remainder[0];
            g.shadow[3] = mask.y - anchor.remainder[1];
            // The ground image of a world vector in mask texels.
            let to_mask = |x: f32, y: f32| {
                [
                    (x * projection.x_axis.x + y * projection.y_axis.x) * tx,
                    (x * projection.x_axis.y + y * projection.y_axis.y) * ty,
                ]
            };
            if area {
                // The ray of a pixel ends at `shadow.zw + t U + s V`.
                let [ux, uy] = to_mask(stored.axis.x, stored.axis.y);
                let [vx, vy] = to_mask(across.x, across.y);
                g.area_mask = [ux, uy, vx, vy];
            }
            if gpu_mask.is_some() && d.mask_mode == MaskMode::Window {
                let p = d.position.raw();
                let [cx, cy] = to_mask(d.mask_center.x - p.x, d.mask_center.y - p.y);
                let [ux, uy] = to_mask(d.mask_u.x, d.mask_u.y);
                let [vx, vy] = to_mask(d.mask_v.x, d.mask_v.y);
                window = [[cx, cy, ux, uy], [vx, vy, 0.0, 0.0]];
            }
        }
        let casters = if d.cast_shadows {
            casters_of(d)
        } else {
            Casters::All
        };
        let mut row = RowLight {
            light: g,
            key: row_key(light_ref),
            shadowed: d.cast_shadows,
            reach: row_space.reach(d.radius + row_span(&g)),
            size: d.source_radius,
            mask: gpu_mask,
            window,
            casters,
        };
        // Beyond what a row reaches or the atlas's slots a light casts no shadow (warned).
        let fits = light_suits(d, row.reach) && out.row_shadows + row.slots() <= MAX_ROWS;
        if row.shadowed && !fits {
            row.shadowed = false;
            out.shadowless_rows += 1;
        }
        out.row_shadows += row.slots();
        if row.shadowed {
            out.caster_classes |= casters.class_bit();
        }
        out.rows.push(row);
    }
}
