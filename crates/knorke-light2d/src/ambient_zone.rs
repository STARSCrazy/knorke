// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/AmbientZone.hpp, src/lighting/AmbientZone.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Ambient zones (P11): the CPU model of one zone: signed distance to its outline, soft edge,
//! height span, the fold of all zones at a world point, validation and the scene rectangle a
//! zone can touch. The shader `ambient_zone.hlsli` of chiaro mirrors the same maths.
//!
//! Redesign against the C++ version: `CHI_AmbientZoneDesc` and `AmbientZoneParams` become
//! [`AmbientZoneParams`] with a [`ZoneShape`] enum, `disabled`/`relative` booleans instead of
//! flag bits, `height: Option<f32>` (`None` is the C `CHI_HEIGHT_INFINITE`) and the polygon as
//! a `Vec<Vec2>` instead of a pointer. Validation is [`validate_zone`]; [`make_stored_zone`]
//! validates first and then builds the stored form, so no unvalidated zone exists. Errors are
//! [`ZoneError`] with stable codes `knorke.light2d.ambient_zone.*`. Maths that influences results
//! uses [`knorke_math::det`] (ADR 0011). Nothing here panics on bad input.

use crate::ambient_zone_store::AmbientZoneStore;
use crate::projection::Projection;
use crate::triangulate::Triangulator;
use knorke_math::{Vec2, Vec3, det};
use knorke_types::{Diagnostic, DiagnosticCode};

/// Most vertices of a polygon zone (one row of the vertex texture).
pub const MAX_ZONE_VERTICES: usize = 64;

/// Bytes of one [`GpuZone`] record (five `float4`).
pub const GPU_ZONE_BYTES: usize = 80;

/// The outline of a zone (`CHI_ShapeType`). The numeric values in [`ZoneShape::code`] are the
/// ones the shader reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZoneShape {
    /// Simple polygon given by its vertices in local units.
    Polygon,
    /// Disc of `radius` around (x, y).
    Circle,
    /// Rectangle `width` x `depth` centred on (x, y), turned by `rotation`.
    #[default]
    Box,
}

impl ZoneShape {
    /// The shape code of the GPU record (`CHI_SHAPE_POLYGON` 0, `CHI_SHAPE_CIRCLE` 1, `CHI_SHAPE_BOX` 2).
    pub fn code(self) -> f32 {
        match self {
            Self::Polygon => 0.0,
            Self::Circle => 1.0,
            Self::Box => 2.0,
        }
    }
}

/// The fields of an ambient zone description (`CHI_AmbientZoneDesc` without the C ABI).
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientZoneParams {
    /// Outline kind; the fields of the other kinds are not checked.
    pub shape: ZoneShape,
    /// The zone is kept but does not contribute (`CHI_AMBIENT_ZONE_DISABLED`).
    pub disabled: bool,
    /// The colour is multiplied by the global ambient (`CHI_AMBIENT_ZONE_RELATIVE`).
    pub relative: bool,
    /// Ambient colour (linear rgb), each channel `>= 0`.
    pub ambient: [f32; 3],
    /// Width of the blend inside the outline, `>= 0`; 0 is a step.
    pub soft_edge: f32,
    /// Fold order: lower priorities are folded first, equal priorities by creation.
    pub priority: i32,
    /// Top of the span, exclusive; `None` is no top.
    pub height: Option<f32>,
    /// Bottom of the span, inclusive, `>= 0`.
    pub base_height: f32,
    /// Centre, world units.
    pub x: f32,
    /// Centre, world units.
    pub y: f32,
    /// Rotation in degrees, counter-clockwise.
    pub rotation: f32,
    /// Polygon vertices in local units, `3..=MAX_ZONE_VERTICES` of them.
    pub vertices: Vec<Vec2>,
    /// Circle radius, `> 0`.
    pub radius: f32,
    /// Box width, `> 0`.
    pub width: f32,
    /// Box depth, `> 0`.
    pub depth: f32,
}

impl Default for AmbientZoneParams {
    /// The C defaults: a box of 1 x 1 at the origin, white, no top, no flags.
    fn default() -> Self {
        Self {
            shape: ZoneShape::Box,
            disabled: false,
            relative: false,
            ambient: [1.0, 1.0, 1.0],
            soft_edge: 0.0,
            priority: 0,
            height: None,
            base_height: 0.0,
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            vertices: Vec::new(),
            radius: 1.0,
            width: 1.0,
            depth: 1.0,
        }
    }
}

/// Why an ambient zone or a zone operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneErrorKind {
    /// A field is out of range, or the polygon is not simple.
    Invalid,
    /// The store holds its maximum of live zones.
    TooMany,
    /// A handle of a zone that was destroyed.
    StaleHandle,
    /// A handle this store never issued.
    ForeignHandle,
}

/// An ambient zone error with its message (the texts keep the C++ wording).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneError {
    kind: ZoneErrorKind,
    message: String,
}

impl ZoneError {
    pub(crate) fn new(kind: ZoneErrorKind, message: String) -> Self {
        Self { kind, message }
    }

    /// The failure kind.
    pub fn kind(&self) -> ZoneErrorKind {
        self.kind
    }

    /// The stable diagnostic code (`knorke.light2d.ambient_zone.*`).
    pub fn code(&self) -> DiagnosticCode {
        match self.kind {
            ZoneErrorKind::Invalid => DiagnosticCode::new("knorke.light2d.ambient_zone.invalid"),
            ZoneErrorKind::TooMany => DiagnosticCode::new("knorke.light2d.ambient_zone.too_many"),
            ZoneErrorKind::StaleHandle => {
                DiagnosticCode::new("knorke.light2d.ambient_zone.stale_handle")
            }
            ZoneErrorKind::ForeignHandle => {
                DiagnosticCode::new("knorke.light2d.ambient_zone.foreign_handle")
            }
        }
    }

    /// As a diagnostic with its stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.message.clone())
    }
}

impl core::fmt::Display for ZoneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ZoneError {}

fn invalid(message: String) -> ZoneError {
    ZoneError::new(ZoneErrorKind::Invalid, message)
}

/// Checks every field the shape reads (fields of other shapes are not checked).
///
/// # Errors
/// [`ZoneErrorKind::Invalid`] with a message naming the field and the value.
pub fn validate_zone(p: &AmbientZoneParams) -> Result<(), ZoneError> {
    for &c in &p.ambient {
        if !c.is_finite() || c < 0.0 {
            return Err(invalid(format!(
                "ambient must be finite and >= 0 in every channel (got {}, {}, {})",
                p.ambient[0], p.ambient[1], p.ambient[2]
            )));
        }
    }
    if !p.soft_edge.is_finite() || p.soft_edge < 0.0 {
        return Err(invalid(format!(
            "soft_edge must be finite and >= 0 (got {})",
            p.soft_edge
        )));
    }
    if !p.x.is_finite() || !p.y.is_finite() || !p.rotation.is_finite() {
        return Err(invalid(format!(
            "x, y and rotation must be finite (got {}, {}, {})",
            p.x, p.y, p.rotation
        )));
    }
    if !p.base_height.is_finite() || p.base_height < 0.0 {
        return Err(invalid(format!(
            "base_height must be finite and >= 0 (got {})",
            p.base_height
        )));
    }
    if p.height
        .is_some_and(|h| !h.is_finite() || !(h > p.base_height))
    {
        let h = p.height.unwrap_or(f32::NAN);
        return Err(invalid(format!(
            "height must be finite and above base_height {} (got {h}), or None for no top",
            p.base_height
        )));
    }
    match p.shape {
        ZoneShape::Polygon => {
            let n = p.vertices.len();
            if !(3..=MAX_ZONE_VERTICES).contains(&n) {
                return Err(invalid(format!(
                    "vertex_count must be in 3..{MAX_ZONE_VERTICES} (got {n})"
                )));
            }
            for (i, v) in p.vertices.iter().enumerate() {
                if !v.x.is_finite() || !v.y.is_finite() {
                    return Err(invalid(format!(
                        "vertices must be finite (vertex {i} is {}, {})",
                        v.x, v.y
                    )));
                }
            }
            let mut triangles = Vec::new();
            if let Err(e) = Triangulator::new().triangulate(&p.vertices[..], &mut triangles) {
                return Err(invalid(format!(
                    "vertices must form a simple polygon ({e})"
                )));
            }
            Ok(())
        }
        ZoneShape::Circle => {
            if !p.radius.is_finite() || !(p.radius > 0.0) {
                return Err(invalid(format!(
                    "radius must be finite and > 0 (got {})",
                    p.radius
                )));
            }
            Ok(())
        }
        ZoneShape::Box => {
            if !p.width.is_finite() || !p.depth.is_finite() || !(p.width > 0.0) || !(p.depth > 0.0)
            {
                return Err(invalid(format!(
                    "width and depth must be finite and > 0 (got {}, {})",
                    p.width, p.depth
                )));
            }
            Ok(())
        }
    }
}

/// A validated zone in world space: the params (without the vertices) and what the distance
/// needs. Only [`make_stored_zone`] builds one, so the invariants hold.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredZone {
    params: AmbientZoneParams,
    /// Polygon: the vertices rotated and moved; empty otherwise.
    world: Vec<Vec2>,
    cos_r: f32,
    sin_r: f32,
    half_w: f32,
    half_d: f32,
    /// World xy bounding rectangle of the outline: x0, y0, x1, y1.
    hull: [f32; 4],
}

impl StoredZone {
    /// The params as given (the vertices are kept in [`StoredZone::world`], not here).
    pub fn params(&self) -> &AmbientZoneParams {
        &self.params
    }

    /// The world bounding rectangle of the outline: x0, y0, x1, y1.
    pub fn hull(&self) -> [f32; 4] {
        self.hull
    }

    /// Cosine and sine of the rotation.
    pub(crate) fn cos_sin(&self) -> (f32, f32) {
        (self.cos_r, self.sin_r)
    }

    /// Box half extents (x, y).
    pub(crate) fn half_extents(&self) -> (f32, f32) {
        (self.half_w, self.half_d)
    }

    /// Polygon vertices in world space.
    pub(crate) fn world_vertices(&self) -> &[Vec2] {
        &self.world
    }
}

/// Validates `p` and builds the stored zone: world vertices, cos/sin, half extents, hull.
///
/// # Errors
/// The errors of [`validate_zone`].
pub fn make_stored_zone(p: &AmbientZoneParams) -> Result<StoredZone, ZoneError> {
    validate_zone(p)?;
    let rad = p.rotation * (core::f32::consts::PI / 180.0);
    let (sin_r, cos_r) = det::sin_cos(rad);
    let mut params = p.clone();
    params.vertices = Vec::new();
    let mut world = Vec::new();
    let mut corners: Vec<(f32, f32)> = Vec::new();
    // Only a box has half extents; the C++ default for the others is 0.5.
    let (mut half_w, mut half_d) = (0.5f32, 0.5f32);
    match p.shape {
        ZoneShape::Polygon => {
            for v in &p.vertices {
                let wx = cos_r * v.x - sin_r * v.y + p.x;
                let wy = sin_r * v.x + cos_r * v.y + p.y;
                world.push(Vec2::new(wx, wy));
                corners.push((wx, wy));
            }
        }
        ZoneShape::Circle => {
            corners.push((p.x - p.radius, p.y - p.radius));
            corners.push((p.x + p.radius, p.y + p.radius));
        }
        ZoneShape::Box => {
            half_w = 0.5 * p.width;
            half_d = 0.5 * p.depth;
            for k in 0..4 {
                let lx = if k & 1 != 0 { half_w } else { -half_w };
                let ly = if k & 2 != 0 { half_d } else { -half_d };
                corners.push((cos_r * lx - sin_r * ly + p.x, sin_r * lx + cos_r * ly + p.y));
            }
        }
    }
    let mut hull = [0.0f32; 4];
    for (i, &(x, y)) in corners.iter().enumerate() {
        if i == 0 {
            hull = [x, y, x, y];
        } else {
            hull[0] = det::min(hull[0], x);
            hull[1] = det::min(hull[1], y);
            hull[2] = det::max(hull[2], x);
            hull[3] = det::max(hull[3], y);
        }
    }
    Ok(StoredZone {
        params,
        world,
        cos_r,
        sin_r,
        half_w,
        half_d,
        hull,
    })
}

/// Signed distance from `(px, py)` to the zone's outline in world units, positive inside.
/// Polygons: the distance to the nearest edge, the sign by the crossing rule; the same loop
/// runs in `ambient_zone.hlsli` (ZoneSignedDistance).
pub fn zone_signed_distance(z: &StoredZone, px: f32, py: f32) -> f32 {
    let p = &z.params;
    match p.shape {
        ZoneShape::Circle => p.radius - det::hypot(px - p.x, py - p.y),
        ZoneShape::Box => {
            let dx = px - p.x;
            let dy = py - p.y;
            let qx = z.cos_r * dx + z.sin_r * dy; // R(-rotation) (p - c)
            let qy = -z.sin_r * dx + z.cos_r * dy;
            let ax = qx.abs() - z.half_w;
            let ay = qy.abs() - z.half_d;
            let ox = det::max(ax, 0.0);
            let oy = det::max(ay, 0.0);
            -((ox * ox + oy * oy).sqrt() + det::min(det::max(ax, ay), 0.0))
        }
        ZoneShape::Polygon => {
            let n = z.world.len();
            if n == 0 {
                return 0.0;
            }
            // Same loop as ZoneSignedDistance in shaders/src/ambient_zone.hlsli: the nearest edge,
            // the crossing rule.
            let mut best = 3.0e38f32;
            let mut inside = false;
            let mut j = n - 1;
            for i in 0..n {
                let (ax, ay) = (z.world[j].x, z.world[j].y);
                let (bx, by) = (z.world[i].x, z.world[i].y);
                let ex = bx - ax;
                let ey = by - ay;
                let wx = px - ax;
                let wy = py - ay;
                let ee = ex * ex + ey * ey;
                let t = if ee > 0.0 {
                    ((wx * ex + wy * ey) / ee).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let cx = wx - ex * t;
                let cy = wy - ey * t;
                best = det::min(best, cx * cx + cy * cy);
                if (ay > py) != (by > py) && px < ex * (py - ay) / ey + ax {
                    inside = !inside;
                }
                j = i;
            }
            let dist = best.sqrt();
            if inside { dist } else { -dist }
        }
    }
}

/// The soft edge: 0 outside (`d < 0`), 1 inside for `soft_edge` 0, else `smoothstep(0, soft_edge, d)`.
pub fn zone_edge_weight(d: f32, soft_edge: f32) -> f32 {
    if d < 0.0 {
        return 0.0;
    }
    if soft_edge <= 0.0 {
        return 1.0;
    }
    let t = (d / soft_edge).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The height span `[base_height, height)`: `true` if `z_height` lies in it (no top for `None`).
pub fn zone_height_contains(z: &StoredZone, z_height: f32) -> bool {
    let p = &z.params;
    z_height >= p.base_height
        && match p.height {
            None => true,
            Some(h) => z_height < h,
        }
}

/// Weight of a zone at a world point: edge times height (the span only if `check_height`); 0
/// when disabled.
pub fn zone_weight(z: &StoredZone, world: Vec3, check_height: bool) -> f32 {
    if z.params.disabled {
        return 0.0;
    }
    if check_height && !zone_height_contains(z, world.z) {
        return 0.0;
    }
    zone_edge_weight(
        zone_signed_distance(z, world.x, world.y),
        z.params.soft_edge,
    )
}

/// The colour a zone replaces the ambient with: its ambient, times the global one per channel
/// when relative.
pub fn zone_color(z: &StoredZone, global: [f32; 3]) -> [f32; 3] {
    let p = &z.params;
    let mut out = p.ambient;
    if p.relative {
        for (o, g) in out.iter_mut().zip(global) {
            *o *= g;
        }
    }
    out
}

/// The ambient at a world point: the fold `A_i = A_(i-1) + w_i (C_i - A_(i-1))` in store order
/// from `A_0 = global`.
pub fn fold_ambient(
    store: &AmbientZoneStore,
    global: [f32; 3],
    world: Vec3,
    check_height: bool,
) -> Vec3 {
    let mut a = global;
    for (_, z) in store.iter_ordered() {
        let w = zone_weight(z, world, check_height);
        if w <= 0.0 {
            continue;
        }
        let c = zone_color(z, global);
        for (ai, ci) in a.iter_mut().zip(c) {
            *ai = *ai * (1.0 - w) + ci * w; // GPU blend form, exact at w = 1
        }
    }
    Vec3::new(a[0], a[1], a[2])
}

/// The scene rectangle `(x0, y0, x1, y1)` in scene pixels that a zone can touch: the hull at the
/// heights `base_height ..= min(height, max_height)` with a surface buffer, only `z = 0` without
/// one; the 8 corners projected, grown by one pixel and clipped to the scene. `None` if it misses
/// the scene, or with a surface buffer when `base_height > max_height`.
pub fn zone_scene_rect(
    z: &StoredZone,
    projection: &Projection,
    max_height: f32,
    has_surface: bool,
    scene_w: u32,
    scene_h: u32,
) -> Option<[f32; 4]> {
    let p = &z.params;
    let (mut z_lo, mut z_hi) = (0.0f32, 0.0f32);
    if has_surface {
        if p.base_height > max_height {
            return None;
        }
        z_lo = p.base_height;
        z_hi = match p.height {
            None => max_height,
            Some(h) => det::min(h, max_height),
        };
        z_hi = det::max(z_hi, z_lo);
    }
    let q = projection;
    let origin = q.origin.raw();
    let (mut x0, mut y0, mut x1, mut y1) = (3.0e38f32, 3.0e38f32, -3.0e38f32, -3.0e38f32);
    for k in 0..8 {
        let wx = if k & 1 != 0 { z.hull[2] } else { z.hull[0] };
        let wy = if k & 2 != 0 { z.hull[3] } else { z.hull[1] };
        let wz = if k & 4 != 0 { z_hi } else { z_lo };
        let sx = origin.x + wx * q.x_axis.x + wy * q.y_axis.x + wz * q.z_axis.x;
        let sy = origin.y + wx * q.x_axis.y + wy * q.y_axis.y + wz * q.z_axis.y;
        x0 = det::min(x0, sx);
        y0 = det::min(y0, sy);
        x1 = det::max(x1, sx);
        y1 = det::max(y1, sy);
    }
    x0 = det::max(x0 - 1.0, 0.0);
    y0 = det::max(y0 - 1.0, 0.0);
    x1 = det::min(x1 + 1.0, scene_w as f32);
    y1 = det::min(y1 + 1.0, scene_h as f32);
    if !(x1 > x0) || !(y1 > y0) {
        return None;
    }
    Some([x0, y0, x1, y1])
}

/// One visible ambient zone as the zone pass reads it (`GpuZone` in `ambient_zone.hlsli`), in fold
/// order. Plain data with the layout of the shader: five `float4`, 80 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuZone {
    /// x0, y0, x1, y1 in scene pixels.
    pub scene_rect: [f32; 4],
    /// rgb the zone's ambient (relative resolved) plus the flat directional sum, and the shape code.
    pub color: [f32; 4],
    /// Centre x, y, cos and sin of the rotation.
    pub shape: [f32; 4],
    /// soft_edge, base_height, top (`f32::MAX`: none), 1 = check the height (surface buffer).
    pub span: [f32; 4],
    /// Circle: x radius; box: xy half extents; polygon: x vertex texture row, y vertex count.
    pub geom: [f32; 4],
}

const _: () = assert!(core::mem::size_of::<GpuZone>() == GPU_ZONE_BYTES);
