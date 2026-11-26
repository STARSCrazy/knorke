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
//! - The shadow parameters live in the occlusion mask's texel space: [`OcclusionDomain`] and
//!   [`MaskAnchor`] come from [`crate::shadow`] (P16) and are re-exported here.

use crate::animation::{AnimationStore, rotate_about_z};
use crate::area_light::{AreaEmitter, AreaKind, area_normalization, rect_across};
use crate::gpu_light::{GpuDirectional, GpuLight, GpuMask};
use crate::light::{Height, LightDesc, LightRef, LightType, ProfileId, ShadowFilter, StoredLight};
use crate::light_mask::{make_gpu_mask, make_mask_placement};
use crate::light_math::{
    NEAR_FIELD_SHARE, SceneQuad, flicker_noise, ground_radius, light_quad, quad_intersects_scene,
};
use crate::mask::mask_atlas_blocks;
use crate::mask_store::MaskStore;
use crate::rows::{MAX_ROWS, RowLight, RowSpace, light_suits};
use crate::settings::LightSettings;
pub use crate::shadow::{MaskAnchor, OcclusionDomain};
use crate::{LightStore, Projection};
use knorke_math::{At, Screen, Vec2, Vec3, det};
use std::borrow::Cow;

const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

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
    /// The frame draws suitable lights with light rows ([`crate::rows::frame_suits`]).
    pub rows: bool,
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
            .field("rows", &self.rows)
            .finish()
    }
}

/// Which obstacles cast a light's shadow (E53).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Casters {
    /// Every caster.
    All,
    /// Only static shapes, tile grids and the occlusion layer.
    Static,
    /// Only obstacles marked dynamic.
    Dynamic,
}

impl Casters {
    /// Index into the filtered lists of [`LightList`]; `None` for [`All`](Self::All).
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
    /// Every visible point/spot light without a mask in store order (phase 2 order).
    pub ordered: Vec<GpuLight>,
    /// Visible point/spot lights without `cast_shadows`.
    pub unshadowed: Vec<GpuLight>,
    /// Visible point/spot lights with `cast_shadows`, `shadow` filled.
    pub shadowed: Vec<GpuLight>,
    /// Visible line/rect lights with `cast_shadows` (own shadow programs, E33).
    pub shadowed_area: Vec<GpuLight>,
    /// Directional lights drawn per pixel (E23, E24), in store order.
    pub directional: Vec<GpuDirectional>,
    /// Lights in `directional` that cast shadows (`color[3] == 1`).
    pub directional_shadows: u32,
    /// Shadow caster filter (E53) of each entry of `directional`.
    pub directional_casters: Vec<Casters>,
    /// Shadowed lights with a caster filter, `[Static, Dynamic][0 point/spot, 1 line/rect]`; not
    /// in `shadowed` and `shadowed_area`. Index with [`Casters::filter_index`].
    pub filtered: [[Vec<GpuLight>; 2]; 2],
    /// Masked lights (E115), drawn by the `_mask` programs after the unmasked ones of their kind;
    /// not in the lists above. All of them in store order (frames without active shadows) ...
    pub masked_ordered: MaskedLights,
    /// ... then split like the plain lists: without shadows ...
    pub masked_unshadowed: MaskedLights,
    /// ... with shadows ...
    pub masked_shadowed: MaskedLights,
    /// ... and shadowed with a caster filter, `[Static, Dynamic]`.
    pub masked_filtered: [MaskedLights; 2],
    /// Row lights (ADR 0019, [`crate::rows::light_suits`]) in store order, drawn by the row programs;
    /// in none of the lists above.
    pub rows: Vec<RowLight>,
    /// Entries of `rows` that cast shadows.
    pub row_shadows: u32,
    /// Visible masked lights (`masked_ordered.len()`).
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
        self.ordered.clear();
        self.unshadowed.clear();
        self.shadowed.clear();
        self.shadowed_area.clear();
        self.directional.clear();
        self.directional_shadows = 0;
        self.directional_casters.clear();
        for by_class in &mut self.filtered {
            for list in by_class {
                list.clear();
            }
        }
        self.masked_ordered.clear();
        self.masked_unshadowed.clear();
        self.masked_shadowed.clear();
        for m in &mut self.masked_filtered {
            m.clear();
        }
        self.rows.clear();
        self.row_shadows = 0;
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
/// `tan(angle) * texels_per_world / |m|` per texel.
fn make_directional(
    stored: &StoredLight,
    color: Vec3,
    strength: f32,
    p: &Projection,
    domain: &OcclusionDomain,
    texels_per_world: f32,
) -> GpuDirectional {
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
    if m > 1e-6 {
        g.dir_slope = [
            mx / m,
            my / m,
            lz / m,
            det::tan(d.source_radius * DEG_TO_RAD) * texels_per_world / m,
        ];
    }
    g.color = [
        color.x * strength,
        color.y * strength,
        color.z * strength,
        0.0, // set by the caller
    ];
    g.world = [lx, ly, lz, d.specular];
    g
}

/// Fills `out` from the enabled lights: directional lights go into `directional` in normal mode
/// (all of them, for their Lambert term), with a medium world map (all of them, for the clouds'
/// shade) and in height mode (those that cast shadows), the others into the ambient sum; point
/// and spot lights whose ground disc can reach the scene rectangle into `ordered` and into one of
/// the three split lists (each in store order). Reuses the vectors' capacity.
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
                let mut g = make_directional(
                    stored,
                    color,
                    strength,
                    projection,
                    &domain,
                    axis_scale * shadow_scale,
                );
                g.color[3] = if shadows { 1.0 } else { 0.0 };
                out.directional.push(g);
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
        let quad = light_quad(projection, d.position, ground);
        let mut visible = quad_intersects_scene(&quad, scene_w, scene_h);
        if modes.normal_mode {
            // Bounding box of the ground quad and the quad raised by max_height.
            let (mut min_x, mut min_y, mut max_x, mut max_y) =
                (1e30f32, 1e30f32, -1e30f32, -1e30f32);
            for c in &quad.corner {
                let c = c.raw();
                for h in [0.0, modes.max_height] {
                    min_x = det::min(min_x, c.x + h * projection.z_axis.x);
                    max_x = det::max(max_x, c.x + h * projection.z_axis.x);
                    min_y = det::min(min_y, c.y + h * projection.z_axis.y);
                    max_y = det::max(max_y, c.y + h * projection.z_axis.y);
                }
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
            visible = quad_intersects_scene(&hull, scene_w, scene_h);
        }
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
        if d.cast_shadows {
            // The ray runs in the mask (ground plane) from the pixel to the ground point of the
            // light; with heights (E22) it rises to the height of the light, `position_radius[2]`,
            // on the way.
            let mask = domain.scene_to_domain(projection.project(d.position, 0.0).raw());
            g.shadow[0] = d.source_radius * axis_scale * shadow_scale;
            if d.inside_obstacle {
                g.shadow[0] = -1.0 - g.shadow[0]; // the sign marks the light (E147); R >= 0 stays unmarked
            }
            g.shadow[2] = mask.x - anchor.remainder[0];
            g.shadow[3] = mask.y - anchor.remainder[1];
            if area {
                // The ground images of U and V in mask texels: the ray of a pixel ends at
                // `shadow.zw + t U + s V`.
                let to_mask = |x: f32, y: f32| {
                    [
                        (x * projection.x_axis.x + y * projection.y_axis.x) * tx,
                        (x * projection.x_axis.y + y * projection.y_axis.y) * ty,
                    ]
                };
                let [ux, uy] = to_mask(stored.axis.x, stored.axis.y);
                let [vx, vy] = to_mask(across.x, across.y);
                g.area_mask = [ux, uy, vx, vy];
            }
        }
        if modes.rows {
            let reach = row_space.reach(d.radius);
            // Beyond the atlas's rows a shadowed light stays with Chiaro's programs.
            if light_suits(d, reach) && !(d.cast_shadows && out.row_shadows >= MAX_ROWS) {
                out.row_shadows += u32::from(d.cast_shadows);
                out.rows.push(RowLight {
                    light: g,
                    key: LightRef::seed_index(light_ref),
                    shadowed: d.cast_shadows,
                    reach,
                    size: d.source_radius,
                });
                continue;
            }
        }
        let casters = if d.cast_shadows {
            casters_of(d)
        } else {
            Casters::All
        };
        if let Some(mask) = gpu_mask {
            out.masked_lights += 1;
            out.masked_ordered.push(g, mask);
            if let Some(i) = casters.filter_index() {
                out.masked_filtered[i].push(g, mask);
                out.caster_classes |= casters.class_bit();
            } else if d.cast_shadows {
                out.masked_shadowed.push(g, mask);
            } else {
                out.masked_unshadowed.push(g, mask);
            }
            continue;
        }
        out.ordered.push(g);
        if let Some(i) = casters.filter_index() {
            out.filtered[i][usize::from(area)].push(g);
            out.caster_classes |= casters.class_bit();
        } else if !d.cast_shadows {
            out.unshadowed.push(g);
        } else if area {
            out.shadowed_area.push(g);
        } else {
            out.shadowed.push(g);
        }
    }
}
