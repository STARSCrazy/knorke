//! Depth-only shadow maps: rasterize light blockers from a light's point of view.
//!
//! Depth is the distance along the view's forward axis. One sample per lookup, no filtering, so
//! shadow edges are hard pixel steps like the rest of the picture. `soft_shadow` widens an edge
//! into a stepped penumbra (0, 1/3, 2/3, 1) from a fixed 3x3 pattern of such lookups.

use crate::math::{Vec3, Vec3Ext};
use crate::model::{BAYER4, step_alpha};
use crate::model::{BillboardCaster, Caster, SOLID_CASTER};
use crate::textures::{CasterTextures, Tex};

/// Near plane of the perspective (cube face) views.
const PERSP_NEAR: f32 = 0.01;

#[derive(Clone, Copy, Debug)]
pub(crate) enum LightView {
    /// Parallel light (sun/moon). The map covers [-half, half] along `right` and `up` around `origin`.
    Ortho {
        origin: Vec3,
        right: Vec3,
        up: Vec3,
        fwd: Vec3,
        half: f32,
        near: f32,
        far: f32,
    },
    /// One 90 degree square face of a point light's cube.
    Persp {
        eye: Vec3,
        right: Vec3,
        up: Vec3,
        fwd: Vec3,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ShadowMap {
    pub(crate) res: usize,
    pub(crate) depth: Vec<f32>,
    pub(crate) view: LightView,
}

impl ShadowMap {
    pub(crate) fn new(res: usize, view: LightView) -> ShadowMap {
        ShadowMap {
            res,
            depth: vec![f32::INFINITY; res * res],
            view,
        }
    }

    /// Clear for a new view, reusing the depth buffer.
    pub(crate) fn reset(&mut self, res: usize, view: LightView) {
        self.res = res;
        self.view = view;
        self.depth.clear();
        self.depth.resize(res * res, f32::INFINITY);
    }

    /// Resize for a new view without clearing (caller fills every band before rasterizing).
    pub(crate) fn prepare_size(&mut self, res: usize, view: LightView) {
        self.res = res;
        self.view = view;
        let len = res * res;
        if self.depth.len() != len {
            self.depth.resize(len, f32::INFINITY);
        }
    }
}

/// Bounding sphere of a caster triangle (centroid and farthest corner).
pub(crate) fn caster_bounds(c: &Caster) -> (Vec3, f32) {
    let centre = (c.tri[0] + c.tri[1] + c.tri[2]) * (1.0 / 3.0);
    let rad = c
        .tri
        .iter()
        .map(|v| (*v - centre).length())
        .fold(0.0f32, f32::max);
    (centre, rad)
}

/// Bounding sphere of a billboard for any facing.
pub(crate) fn billboard_bounds(b: &BillboardCaster) -> (Vec3, f32) {
    let (hw, hh) = (b.width.abs() * 0.5, b.height.abs() * 0.5);
    (
        b.base + Vec3::new(0.0, b.height * 0.5, 0.0),
        (hw * hw + hh * hh).sqrt(),
    )
}

/// False if a sphere lies entirely outside a cube face's 90 degree frustum (it cannot cover any
/// texel of that face). Always true for ortho views. Conservative: a small margin is kept.
pub(crate) fn face_may_see(view: &LightView, centre: Vec3, rad: f32) -> bool {
    let LightView::Persp {
        eye,
        right,
        up,
        fwd,
    } = *view
    else {
        return true;
    };
    let d = centre - eye;
    let (x, y, z) = (d.dot(right), d.dot(up), d.dot(fwd));
    // planes z = +-x and z = +-y through the eye; signed distance (z -+ x) / sqrt 2
    let lim = -(rad * 1.001 + 1e-4) * std::f32::consts::SQRT_2;
    // NaN compares false: kept
    !(z - x < lim || z + x < lim || z - y < lim || z + y < lim)
}

/// Orthonormal `(right, up)` for a forward direction; stable for straight down/up and horizontal.
pub(crate) fn basis(fwd: Vec3) -> (Vec3, Vec3) {
    let f = fwd.normalized();
    let reference = if f.y.abs() > 0.99 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let right = reference.cross(f).normalized();
    let up = f.cross(right);
    (right, up)
}

/// Cube face for a direction from the light: largest axis, order +x -x +y -y +z -z.
pub(crate) fn cube_face(d: Vec3) -> usize {
    let (ax, ay, az) = (d.x.abs(), d.y.abs(), d.z.abs());
    if ax >= ay && ax >= az {
        if d.x >= 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if d.y >= 0.0 { 2 } else { 3 }
    } else if d.z >= 0.0 {
        4
    } else {
        5
    }
}

/// Cube face for `d` (a point minus the light position) and the point in that face's light space,
/// bit for bit what `to_light(&cube_views(light)[face], point)` gives: the face axes from `basis`
/// are exact unit axes, so each dot product is one component with a sign (plus zeros, which only
/// change the sign of a zero result; `lookup` treats both zeros alike). Checked against `to_light`
/// in `cube_light_matches_to_light`.
#[inline(always)]
pub(crate) fn cube_light(d: Vec3) -> (usize, Vec3) {
    let face = cube_face(d);
    let l = match face {
        0 => Vec3::new(-d.z, d.y, d.x),
        1 => Vec3::new(d.z, d.y, -d.x),
        2 => Vec3::new(-d.x, d.z, d.y),
        3 => Vec3::new(d.x, d.z, -d.y),
        4 => d,
        _ => Vec3::new(-d.x, d.y, -d.z),
    };
    (face, l)
}

/// Depth (forward component) of `d` in cube face `face`'s light space; see `cube_light`.
#[inline(always)]
fn cube_depth(face: usize, d: Vec3) -> f32 {
    match face {
        0 => d.x,
        1 => -d.x,
        2 => d.y,
        3 => -d.y,
        4 => d.z,
        _ => -d.z,
    }
}

/// The six 90 degree views of a point light, in `cube_face` order.
pub(crate) fn cube_views(pos: Vec3) -> [LightView; 6] {
    let dirs = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
    ];
    dirs.map(|fwd| {
        let (right, up) = basis(fwd);
        LightView::Persp {
            eye: pos,
            right,
            up,
            fwd,
        }
    })
}

/// World point to light space (x right, y up, z forward depth).
fn to_light(view: &LightView, p: Vec3) -> Vec3 {
    let (o, r, u, f) = match *view {
        LightView::Ortho {
            origin,
            right,
            up,
            fwd,
            ..
        } => (origin, right, up, fwd),
        LightView::Persp {
            eye,
            right,
            up,
            fwd,
        } => (eye, right, up, fwd),
    };
    let d = p - o;
    Vec3::new(d.dot(r), d.dot(u), d.dot(f))
}

/// Light space to continuous map pixel coordinates plus 1/w; None behind a perspective eye.
fn to_map(view: &LightView, res: usize, l: Vec3) -> Option<(f32, f32, f32)> {
    let n = res as f32;
    match *view {
        LightView::Ortho { half, .. } => Some((
            (l.x / half * 0.5 + 0.5) * n,
            (0.5 - l.y / half * 0.5) * n,
            1.0,
        )),
        LightView::Persp { .. } => {
            if l.z <= 0.0 {
                return None;
            }
            let iw = 1.0 / l.z;
            Some(((l.x * iw * 0.5 + 0.5) * n, (0.5 - l.y * iw * 0.5) * n, iw))
        }
    }
}

#[derive(Clone, Copy)]
struct LVert {
    p: Vec3,
    u: f32,
    v: f32,
}

/// How a caster covers the map: its texture (None = untextured, fully opaque), whether the alpha
/// test can be skipped, and its draw alpha (1 = opaque caster; below 1 = stippled, see `fill`).
struct Cover<'a, T: ?Sized> {
    tex: Option<Tex<'a, T>>,
    wrap: bool,
    opaque: bool,
    alpha: f32,
}

impl<T: ?Sized> Clone for Cover<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Cover<'_, T> {}

/// Rasterize one light-space triangle (nearest depth, alpha test on the base level, no culling).
fn raster<T: CasterTextures + ?Sized>(map: &mut ShadowMap, tri: [LVert; 3], cover: Cover<T>) {
    // clip against the near plane (perspective only; ortho rejects per pixel by depth range).
    // One plane clips a triangle to at most 4 vertices: fixed arrays, no allocation.
    let mut poly = [tri[0]; 4];
    let mut len = 3;
    poly[1] = tri[1];
    poly[2] = tri[2];
    if let LightView::Persp { .. } = map.view {
        let mut out = [tri[0]; 4];
        let mut n = 0;
        for i in 0..3 {
            let a = tri[i];
            let b = tri[(i + 1) % 3];
            let da = a.p.z - PERSP_NEAR;
            let db = b.p.z - PERSP_NEAR;
            if da >= 0.0 {
                out[n] = a;
                n += 1;
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                out[n] = LVert {
                    p: a.p.lerp(b.p, t),
                    u: a.u + (b.u - a.u) * t,
                    v: a.v + (b.v - a.v) * t,
                };
                n += 1;
            }
        }
        poly = out;
        len = n;
    }
    if len < 3 {
        return;
    }
    // per vertex: (sx, sy, 1/w, u/w, v/w, z/w)
    let mut sv = [[0.0f32; 6]; 4];
    for k in 0..len {
        let v = poly[k];
        let Some((x, y, iw)) = to_map(&map.view, map.res, v.p) else {
            return;
        };
        sv[k] = [x, y, iw, v.u * iw, v.v * iw, v.p.z * iw];
    }
    let (near, far) = match map.view {
        LightView::Ortho { near, far, .. } => (near, far),
        LightView::Persp { .. } => (PERSP_NEAR, f32::INFINITY),
    };
    // the clipped polygon is convex and planar: fan triangulation
    for i in 1..len - 1 {
        fill(map, [sv[0], sv[i], sv[i + 1]], cover, near, far);
    }
}

#[inline]
fn edge(a: &[f32; 6], b: &[f32; 6], px: f32, py: f32) -> f32 {
    (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
}

/// Screen-space triangle ready for shadow-map rasterization (used for band-parallel sun maps).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PreparedTri {
    pub(crate) t: [[f32; 6]; 3],
    pub(crate) inv_area: f32,
    pub(crate) x0: i64,
    pub(crate) x1: i64,
    pub(crate) y0: i64,
    pub(crate) y1: i64,
    pub(crate) tex_idx: usize,
    pub(crate) wrap: bool,
    pub(crate) opaque: bool,
    pub(crate) alpha: f32,
    pub(crate) spans: bool,
    /// Ortho view with all vertex depths strictly inside (near, far): inner loop skips range/iw checks.
    pub(crate) ortho_in_range: bool,
}

/// Depth range `(near, far)` of a shadow map view.
pub(crate) fn view_depth_range(view: &LightView) -> (f32, f32) {
    match *view {
        LightView::Ortho { near, far, .. } => (near, far),
        LightView::Persp { .. } => (PERSP_NEAR, f32::INFINITY),
    }
}

/// Light-space texel index of map texel (0, 0) for the stipple pattern: the sun map moves in whole
/// texels with the camera, so the pattern is anchored to the world, not to the map.
pub(crate) fn pattern_origin(view: &LightView, res: usize) -> (i64, i64) {
    match *view {
        LightView::Ortho {
            origin,
            right,
            up,
            half,
            ..
        } => {
            let texel = 2.0 * half / res as f32;
            let k = |v: f32| if v.is_finite() { v.round() as i64 } else { 0 };
            (k(origin.dot(right) / texel), k(origin.dot(up) / texel))
        }
        LightView::Persp { .. } => (0, 0),
    }
}

/// See-through casters (alpha < 1) write depth only on an ordered pattern of texels: the effective
/// alpha (draw alpha times texel alpha, in eighths) picks how many of each 4x4 block of the Bayer
/// matrix block light, so alpha 1/2 gives a checkered shadow. `(ax, ay)`: texel in light space.
#[inline]
fn stipple_keeps(alpha: f32, texel_alpha: u8, ax: i64, ay: i64) -> bool {
    let eighths = step_alpha(alpha * texel_alpha as f32 / 255.0) * 8.0;
    (BAYER4[ay.rem_euclid(4) as usize][ax.rem_euclid(4) as usize] as f32) < 2.0 * eighths
}

/// Prepare one light-space triangle into zero or more clipped `PreparedTri` entries.
fn prepare_tri<T: CasterTextures + ?Sized>(
    view: &LightView,
    res: usize,
    tri: [LVert; 3],
    tex_idx: usize,
    tex: Option<Tex<'_, T>>,
    wrap: bool,
    opaque: bool,
    alpha: f32,
    out: &mut Vec<PreparedTri>,
) {
    let mut poly = [tri[0]; 4];
    let mut len = 3;
    poly[1] = tri[1];
    poly[2] = tri[2];
    if let LightView::Persp { .. } = *view {
        let mut clipped = [tri[0]; 4];
        let mut n = 0;
        for i in 0..3 {
            let a = tri[i];
            let b = tri[(i + 1) % 3];
            let da = a.p.z - PERSP_NEAR;
            let db = b.p.z - PERSP_NEAR;
            if da >= 0.0 {
                clipped[n] = a;
                n += 1;
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                clipped[n] = LVert {
                    p: a.p.lerp(b.p, t),
                    u: a.u + (b.u - a.u) * t,
                    v: a.v + (b.v - a.v) * t,
                };
                n += 1;
            }
        }
        poly = clipped;
        len = n;
    }
    if len < 3 {
        return;
    }
    let mut sv = [[0.0f32; 6]; 4];
    for k in 0..len {
        let v = poly[k];
        let Some((x, y, iw)) = to_map(view, res, v.p) else {
            return;
        };
        sv[k] = [x, y, iw, v.u * iw, v.v * iw, v.p.z * iw];
    }
    let n = res as i64;
    let (near, far) = view_depth_range(view);
    let is_ortho = matches!(*view, LightView::Ortho { .. });
    for i in 1..len - 1 {
        let mut t = [sv[0], sv[i], sv[i + 1]];
        let mut area = edge(&t[0], &t[1], t[2][0], t[2][1]);
        if area == 0.0 || !area.is_finite() {
            continue;
        }
        if area < 0.0 {
            t.swap(1, 2);
            area = -area;
        }
        let lo = |k: usize| t.iter().map(|v| v[k]).fold(f32::INFINITY, f32::min);
        let hi = |k: usize| t.iter().map(|v| v[k]).fold(f32::NEG_INFINITY, f32::max);
        let x0 = ((lo(0) - 0.5).floor() as i64).max(0);
        let x1 = ((hi(0) - 0.5).ceil() as i64).min(n - 1);
        let y0 = ((lo(1) - 0.5).floor() as i64).max(0);
        let y1 = ((hi(1) - 0.5).ceil() as i64).min(n - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let mut ortho_in_range = false;
        if is_ortho && near < far {
            let zmin = lo(5);
            let zmax = hi(5);
            if zmax < near - 1e-4 || zmin > far + 1e-4 {
                continue;
            }
            ortho_in_range = zmin > near + 1e-4 && zmax < far - 1e-4;
        }
        let mut is_opaque = opaque || tex.is_none();
        // Constant-UV ortho triangle (e.g. solid-colour roof faces sampling one texel of a
        // texture with holes elsewhere): sample once if safely away from texel boundaries.
        if !is_opaque
            && alpha >= 1.0
            && is_ortho
            && let Some(tx) = tex
        {
            let (u, v) = (t[0][3], t[0][4]);
            if u == t[1][3] && u == t[2][3] && v == t[1][4] && v == t[2][4] {
                let fu = (u * tx.width() as f32).rem_euclid(1.0);
                let fv = (v * tx.height() as f32).rem_euclid(1.0);
                if fu > 1e-3 && fu < 0.999 && fv > 1e-3 && fv < 0.999 {
                    let ta = tx.alpha(u, v, wrap);
                    if ta < 128 {
                        continue;
                    }
                    is_opaque = true;
                }
            }
        }
        let spans = t.iter().all(|v| v[0].abs() < 1e15 && v[1].abs() < 1e15);
        out.push(PreparedTri {
            t,
            inv_area: 1.0 / area,
            x0,
            x1,
            y0,
            y1,
            tex_idx: if is_opaque { SOLID_CASTER } else { tex_idx },
            wrap,
            opaque: is_opaque,
            alpha,
            spans,
            ortho_in_range,
        });
    }
}

/// Prepare one caster triangle for band-parallel shadow rasterization.
pub(crate) fn prepare_caster<T: CasterTextures + ?Sized>(
    view: &LightView,
    res: usize,
    c: &Caster,
    tex: Option<Tex<'_, T>>,
    opaque: bool,
    out: &mut Vec<PreparedTri>,
) {
    let tri = [0, 1, 2].map(|k| LVert {
        p: to_light(view, c.tri[k]),
        u: c.uv[k][0],
        v: c.uv[k][1],
    });
    prepare_tri(view, res, tri, c.texture, tex, c.wrap, opaque, c.alpha, out);
}

/// Prepare one billboard caster (two triangles) for band-parallel shadow rasterization.
pub(crate) fn prepare_billboard<T: CasterTextures + ?Sized>(
    view: &LightView,
    res: usize,
    b: &BillboardCaster,
    light_pos: Option<Vec3>,
    light_dir: Vec3,
    tex: Option<Tex<'_, T>>,
    opaque: bool,
    out: &mut Vec<PreparedTri>,
) {
    let to = match light_pos {
        Some(p) => p - b.base,
        None => -light_dir,
    };
    let h = Vec3::new(to.x, 0.0, to.z);
    let n = if h.length() > 1e-6 {
        h.normalized()
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let right = Vec3::new(n.z, 0.0, -n.x) * (b.width * 0.5);
    let up = Vec3::new(0.0, b.height, 0.0);
    let (u0, u1) = if b.flip { (1.0, 0.0) } else { (0.0, 1.0) };
    let corners = [
        (b.base - right, u0, 1.0),
        (b.base + right, u1, 1.0),
        (b.base + right + up, u1, 0.0),
        (b.base - right + up, u0, 0.0),
    ];
    let [du, dv] = b.scroll.unwrap_or([0.0, 0.0]);
    let wrap = b.scroll.is_some();
    let lv = corners.map(|(p, u, v)| LVert {
        p: to_light(view, p),
        u: if wrap { u + du } else { u },
        v: if wrap { v + dv } else { v },
    });
    prepare_tri(
        view,
        res,
        [lv[0], lv[1], lv[2]],
        b.texture,
        tex,
        wrap,
        opaque,
        b.alpha,
        out,
    );
    prepare_tri(
        view,
        res,
        [lv[0], lv[2], lv[3]],
        b.texture,
        tex,
        wrap,
        opaque,
        b.alpha,
        out,
    );
}

/// Rasterize one prepared triangle into a horizontal band `[y_start, y_end)` of a shadow map.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_band<T: CasterTextures + ?Sized>(
    depth: &mut [f32],
    res: usize,
    y_start: usize,
    y_end: usize,
    tri: &PreparedTri,
    textures: &T,
    near: f32,
    far: f32,
    ox: i64,
    oy: i64,
) {
    let ry0 = tri.y0.max(y_start as i64);
    let ry1 = tri.y1.min(y_end as i64 - 1);
    if ry0 > ry1 {
        return;
    }
    let tex = if tri.tex_idx == SOLID_CASTER {
        None
    } else {
        Tex::get(textures, tri.tex_idx)
    };
    let cover = Cover {
        tex,
        wrap: tri.wrap,
        opaque: tri.opaque,
        alpha: tri.alpha,
    };
    fill_rows::<true, T>(
        depth,
        res,
        y_start,
        ry0,
        ry1,
        tri.x0,
        tri.x1,
        &tri.t,
        tri.inv_area,
        tri.spans,
        tri.ortho_in_range,
        cover,
        near,
        far,
        ox,
        oy,
    );
}

fn fill<T: CasterTextures + ?Sized>(
    map: &mut ShadowMap,
    t: [[f32; 6]; 3],
    cover: Cover<T>,
    near: f32,
    far: f32,
) {
    fill_impl::<true, T>(map, t, cover, near, far)
}

/// `fill`; SPANS = false tests every texel of the bounding box (reference for the tests).
fn fill_impl<const SPANS: bool, T: CasterTextures + ?Sized>(
    map: &mut ShadowMap,
    mut t: [[f32; 6]; 3],
    cover: Cover<T>,
    near: f32,
    far: f32,
) {
    let (ox, oy) = if cover.alpha < 1.0 {
        pattern_origin(&map.view, map.res)
    } else {
        (0, 0)
    };
    let mut area = edge(&t[0], &t[1], t[2][0], t[2][1]);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        t.swap(1, 2);
        area = -area;
    }
    let n = map.res as i64;
    let lo = |k: usize| t.iter().map(|v| v[k]).fold(f32::INFINITY, f32::min);
    let hi = |k: usize| t.iter().map(|v| v[k]).fold(f32::NEG_INFINITY, f32::max);
    let x0 = ((lo(0) - 0.5).floor() as i64).max(0);
    let x1 = ((hi(0) - 0.5).ceil() as i64).min(n - 1);
    let y0 = ((lo(1) - 0.5).floor() as i64).max(0);
    let y1 = ((hi(1) - 0.5).ceil() as i64).min(n - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let spans = SPANS && t.iter().all(|v| v[0].abs() < 1e15 && v[1].abs() < 1e15);
    let res = map.res;
    fill_rows::<SPANS, T>(
        &mut map.depth,
        res,
        0,
        y0,
        y1,
        x0,
        x1,
        &t,
        1.0 / area,
        spans,
        false,
        cover,
        near,
        far,
        ox,
        oy,
    );
}

/// Core row rasterizer for `depth` covering rows `[y_start, ..)`.
/// Hoists row-constant edge terms and finds exact span endpoints analytically (with exact edge
/// verification at the boundaries) so the inner loop is branch-free on edges; exact same IEEE-754
/// operations and texels as testing every texel.
#[allow(clippy::too_many_arguments)]
fn fill_rows<const SPANS: bool, T: CasterTextures + ?Sized>(
    depth: &mut [f32],
    res: usize,
    y_start: usize,
    y0: i64,
    y1: i64,
    x0: i64,
    x1: i64,
    t: &[[f32; 6]; 3],
    inv_area: f32,
    spans: bool,
    ortho_in_range: bool,
    cover: Cover<T>,
    near: f32,
    far: f32,
    ox: i64,
    oy: i64,
) {
    let Cover {
        tex,
        wrap,
        opaque,
        alpha,
    } = cover;
    let opaque_fast = (opaque || tex.is_none()) && alpha >= 1.0;

    // Edge coefficients: edge(a, b, cx, cy) = dx * (cy - ay) - dy * (cx - ax)
    let (ax0, ay0, dx0, dy0) = (t[1][0], t[1][1], t[2][0] - t[1][0], t[2][1] - t[1][1]);
    let (ax1, ay1, dx1, dy1) = (t[2][0], t[2][1], t[0][0] - t[2][0], t[0][1] - t[2][1]);
    let (ax2, ay2, dx2, dy2) = (t[0][0], t[0][1], t[1][0] - t[0][0], t[1][1] - t[0][1]);
    let (iw0, iw1, iw2) = (t[0][2], t[1][2], t[2][2]);
    let (z0, z1, z2) = (t[0][5], t[1][5], t[2][5]);
    let inv_dy0 = if dy0 != 0.0 { 1.0 / dy0 } else { 0.0 };
    let inv_dy1 = if dy1 != 0.0 { 1.0 / dy1 } else { 0.0 };
    let inv_dy2 = if dy2 != 0.0 { 1.0 / dy2 } else { 0.0 };

    for py in y0..=y1 {
        let cy = py as f32 + 0.5;
        let c0 = dx0 * (cy - ay0);
        let c1 = dx1 * (cy - ay1);
        let c2 = dx2 * (cy - ay2);

        let (start, end) = if SPANS && spans {
            // Horizontal edges (dy == 0) are constant along the row: if negative, the row is empty.
            if (dy0 == 0.0 && c0 < 0.0) || (dy1 == 0.0 && c1 < 0.0) || (dy2 == 0.0 && c2 < 0.0) {
                continue;
            }
            // Analytical estimate of [s, e) from edge crossings cx = ax + c / dy, then exact
            // adjustment against the exact edge functions so float rounding never misses/adds a texel.
            let mut s = x0;
            let mut e = x1 + 1;
            for &(ax, c, dy, inv_dy) in &[
                (ax0, c0, dy0, inv_dy0),
                (ax1, c1, dy1, inv_dy1),
                (ax2, c2, dy2, inv_dy2),
            ] {
                if dy < 0.0 {
                    let x = ax + c * inv_dy - 0.5;
                    if x.is_finite() {
                        s = s.max(x.floor() as i64);
                    }
                } else if dy > 0.0 {
                    let x = ax + c * inv_dy - 0.5;
                    if x.is_finite() {
                        e = e.min(x.ceil() as i64 + 1);
                    }
                }
            }
            s = s.clamp(x0, x1 + 1);
            e = e.clamp(s, x1 + 1);
            let grows_ok = |px: i64| {
                let cx = px as f32 + 0.5;
                (dy0 > 0.0 || c0 - dy0 * (cx - ax0) >= 0.0)
                    && (dy1 > 0.0 || c1 - dy1 * (cx - ax1) >= 0.0)
                    && (dy2 > 0.0 || c2 - dy2 * (cx - ax2) >= 0.0)
            };
            while s > x0 && grows_ok(s - 1) {
                s -= 1;
            }
            while s <= x1 && !grows_ok(s) {
                s += 1;
            }
            if s > x1 {
                continue;
            }
            let shrinks_ok = |px: i64| {
                let cx = px as f32 + 0.5;
                (dy0 <= 0.0 || c0 - dy0 * (cx - ax0) >= 0.0)
                    && (dy1 <= 0.0 || c1 - dy1 * (cx - ax1) >= 0.0)
                    && (dy2 <= 0.0 || c2 - dy2 * (cx - ax2) >= 0.0)
            };
            if e < s {
                e = s;
            }
            while e > s && !shrinks_ok(e - 1) {
                e -= 1;
            }
            while e <= x1 && shrinks_ok(e) {
                e += 1;
            }
            if s >= e {
                continue;
            }
            (s, e)
        } else {
            (x0, x1 + 1)
        };

        let row = &mut depth[(py as usize - y_start) * res..];
        if SPANS && spans && opaque_fast && ortho_in_range {
            // Branchless ortho opaque path: iw == b0 + b1 + b2 > 0 and z is strictly in (near, far).
            let slice = &mut row[start as usize..end as usize];
            let mut cx = start as f32 + 0.5;
            for slot in slice.iter_mut() {
                let e0 = c0 - dy0 * (cx - ax0);
                let e1 = c1 - dy1 * (cx - ax1);
                let e2 = c2 - dy2 * (cx - ax2);
                let (b0, b1, b2) = (e0 * inv_area, e1 * inv_area, e2 * inv_area);
                let w = 1.0 / (b0 + b1 + b2);
                let z = (b0 * z0 + b1 * z1 + b2 * z2) * w;
                if z < *slot {
                    *slot = z;
                }
                cx += 1.0;
            }
        } else if SPANS && spans && opaque_fast {
            let slice = &mut row[start as usize..end as usize];
            for (k, slot) in slice.iter_mut().enumerate() {
                let cx = (start + k as i64) as f32 + 0.5;
                let e0 = c0 - dy0 * (cx - ax0);
                let e1 = c1 - dy1 * (cx - ax1);
                let e2 = c2 - dy2 * (cx - ax2);
                let (b0, b1, b2) = (e0 * inv_area, e1 * inv_area, e2 * inv_area);
                let iw = b0 * iw0 + b1 * iw1 + b2 * iw2;
                if iw <= 0.0 {
                    continue;
                }
                let w = 1.0 / iw;
                let z = (b0 * z0 + b1 * z1 + b2 * z2) * w;
                if z >= near && z <= far && z < *slot {
                    *slot = z;
                }
            }
        } else {
            for px in start..end {
                let cx = px as f32 + 0.5;
                let e0 = c0 - dy0 * (cx - ax0);
                let e1 = c1 - dy1 * (cx - ax1);
                let e2 = c2 - dy2 * (cx - ax2);
                if !(SPANS && spans) && (e0 < 0.0 || e1 < 0.0 || e2 < 0.0) {
                    continue;
                }
                let (b0, b1, b2) = (e0 * inv_area, e1 * inv_area, e2 * inv_area);
                let iw = b0 * iw0 + b1 * iw1 + b2 * iw2;
                if iw <= 0.0 {
                    continue;
                }
                let w = 1.0 / iw;
                let z = (b0 * z0 + b1 * z1 + b2 * z2) * w;
                if z < near || z > far {
                    continue;
                }
                let slot = &mut row[px as usize];
                if z >= *slot {
                    continue;
                }
                if !opaque_fast {
                    let ta = match tex {
                        Some(tex) => {
                            let u = (b0 * t[0][3] + b1 * t[1][3] + b2 * t[2][3]) * w;
                            let v = (b0 * t[0][4] + b1 * t[1][4] + b2 * t[2][4]) * w;
                            tex.alpha(u, v, wrap)
                        }
                        None => 255,
                    };
                    if alpha < 1.0 {
                        if !stipple_keeps(alpha, ta, ox + px, oy - py) {
                            continue;
                        }
                    } else if ta < 128 {
                        continue;
                    }
                }
                *slot = z;
            }
        }
    }
}

/// Fill the map with the nearest blocker depth. Casters with an unknown texture id are skipped.
/// Billboards stand upright and turn around the vertical axis to face the light.
/// `skip_beyond`: casters whose bounding sphere lies entirely farther than this from `light_pos`
/// are skipped (the caller picks a distance at which they cannot change any lookup).
#[cfg(test)]
pub(crate) fn render_shadow<T: CasterTextures + ?Sized>(
    map: &mut ShadowMap,
    casters: &[Caster],
    billboards: &[BillboardCaster],
    light_pos: Option<Vec3>,
    light_dir: Vec3,
    textures: &T,
    skip_beyond: Option<f32>,
) {
    for c in casters {
        if skip_beyond.is_some() {
            let (centre, rad) = caster_bounds(c);
            if far_away(light_pos, skip_beyond, centre, rad) {
                continue;
            }
        }
        if let Some(tex) = caster_texture(c, textures) {
            draw_caster(map, c, tex, false);
        }
    }
    for b in billboards {
        let (centre, rad) = billboard_bounds(b);
        if far_away(light_pos, skip_beyond, centre, rad) {
            continue;
        }
        if let Some(tex) = Tex::get(textures, b.texture) {
            draw_billboard(map, b, light_pos, light_dir, Some(tex), false);
        }
    }
}

/// True if a sphere lies entirely farther than `skip_beyond` from `light_pos`.
pub(crate) fn far_away(
    light_pos: Option<Vec3>,
    skip_beyond: Option<f32>,
    centre: Vec3,
    rad: f32,
) -> bool {
    // NaN anywhere makes the comparison false: nothing is skipped
    match (light_pos, skip_beyond) {
        (Some(p), Some(lim)) => (centre - p).length() - rad > lim,
        _ => false,
    }
}

/// Texture of a caster: Some(None) for an untextured solid caster (`SOLID_CASTER`), Some(Some(t))
/// for a known texture, None for an unknown texture id (the caster is skipped).
pub(crate) fn caster_texture<'a, T: CasterTextures + ?Sized>(
    c: &Caster,
    textures: &'a T,
) -> Option<Option<Tex<'a, T>>> {
    if c.texture == SOLID_CASTER {
        return Some(None);
    }
    Tex::get(textures, c.texture).map(Some)
}

/// Rasterize one caster triangle with its texture `tex` (None = untextured and opaque;
/// `opaque`: no texel is transparent).
pub(crate) fn draw_caster<T: CasterTextures + ?Sized>(
    map: &mut ShadowMap,
    c: &Caster,
    tex: Option<Tex<'_, T>>,
    opaque: bool,
) {
    let view = map.view;
    let tri = [0, 1, 2].map(|k| LVert {
        p: to_light(&view, c.tri[k]),
        u: c.uv[k][0],
        v: c.uv[k][1],
    });
    raster(
        map,
        tri,
        Cover {
            tex,
            wrap: c.wrap,
            opaque,
            alpha: c.alpha,
        },
    );
}

/// Rasterize one billboard, turned towards the light (`light_pos`, or against `light_dir` for the sun).
pub(crate) fn draw_billboard<T: CasterTextures + ?Sized>(
    map: &mut ShadowMap,
    b: &BillboardCaster,
    light_pos: Option<Vec3>,
    light_dir: Vec3,
    tex: Option<Tex<'_, T>>,
    opaque: bool,
) {
    let view = map.view;
    // horizontal direction from the billboard towards the light
    let to = match light_pos {
        Some(p) => p - b.base,
        None => -light_dir,
    };
    let h = Vec3::new(to.x, 0.0, to.z);
    // sun straight down: any fixed facing
    let n = if h.length() > 1e-6 {
        h.normalized()
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let right = Vec3::new(n.z, 0.0, -n.x) * (b.width * 0.5);
    let up = Vec3::new(0.0, b.height, 0.0);
    let (u0, u1) = if b.flip { (1.0, 0.0) } else { (0.0, 1.0) };
    let corners = [
        (b.base - right, u0, 1.0),
        (b.base + right, u1, 1.0),
        (b.base + right + up, u1, 0.0),
        (b.base - right + up, u0, 0.0),
    ];
    // same texture coordinates as on screen (Renderer::resolve_quad)
    let [du, dv] = b.scroll.unwrap_or([0.0, 0.0]);
    let wrap = b.scroll.is_some();
    let lv = corners.map(|(p, u, v)| LVert {
        p: to_light(&view, p),
        u: if wrap { u + du } else { u },
        v: if wrap { v + dv } else { v },
    });
    let cover = Cover {
        tex,
        wrap,
        opaque,
        alpha: b.alpha,
    };
    raster(map, [lv[0], lv[1], lv[2]], cover);
    raster(map, [lv[0], lv[2], lv[3]], cover);
}

/// 1.0 lit, 0.0 in shadow. Points outside the map's area or depth range count as lit.
pub(crate) fn lookup(map: &ShadowMap, world: Vec3, bias: f32) -> f32 {
    let l = to_light(&map.view, world);
    if let LightView::Ortho { near, far, .. } = map.view
        && (l.z < near || l.z > far)
    {
        return 1.0;
    }
    let Some((x, y, _)) = to_map(&map.view, map.res, l) else {
        return 1.0;
    };
    if !(x >= 0.0 && y >= 0.0 && x < map.res as f32 && y < map.res as f32) {
        return 1.0;
    }
    let d = map.depth[y as usize * map.res + x as usize];
    if l.z - bias > d { 0.0 } else { 1.0 }
}

/// `lookup(&maps[cube_face(world - light)], world, bias)` for the cube maps of a point light at
/// `light`, without the generic light-space transform (same result bit for bit, see `cube_light`).
/// Only for finite `world - light`: an infinite component would turn into NaN in the dot products.
#[inline(always)]
pub(crate) fn lookup_cube(maps: &[ShadowMap; 6], world: Vec3, light: Vec3, bias: f32) -> f32 {
    let (face, l) = cube_light(world - light);
    let map = &maps[face];
    let Some((x, y, _)) = to_map(&map.view, map.res, l) else {
        return 1.0;
    };
    if !(x >= 0.0 && y >= 0.0 && x < map.res as f32 && y < map.res as f32) {
        return 1.0;
    }
    let d = map.depth[y as usize * map.res + x as usize];
    if l.z - bias > d { 0.0 } else { 1.0 }
}

/// `lookup_tap(&maps[cube_face(world - light)], world, centre, lateral, bias)` for cube maps,
/// like `lookup_cube`.
#[inline(always)]
pub(crate) fn lookup_cube_tap(
    maps: &[ShadowMap; 6],
    world: Vec3,
    centre: Vec3,
    light: Vec3,
    lateral: f32,
    bias: f32,
) -> f32 {
    let (face, l) = cube_light(world - light);
    let map = &maps[face];
    let Some((x, y, _)) = to_map(&map.view, map.res, l) else {
        return 1.0;
    };
    if !(x >= 0.0 && y >= 0.0 && x < map.res as f32 && y < map.res as f32) {
        return 1.0;
    }
    let d = map.depth[y as usize * map.res + x as usize];
    let z = if lateral > 0.0 {
        l.z.min(cube_depth(face, centre - light))
    } else {
        l.z
    };
    if z - bias - TAP_CONE * lateral > d {
        0.0
    } else {
        1.0
    }
}

/// How much nearer to the light than the receiver a blocker must be, per world unit of a tap's
/// sideways offset, to count for that tap (`lookup_tap`).
pub(crate) const TAP_CONE: f32 = 2.0;

/// `lookup` for one soft shadow tap at `world`, offset sideways by `lateral` (world units) from the
/// receiver point `centre`. The blocker found in the map must be in front of the receiver point
/// itself (the nearer of the tap's and the centre's light depth) by `bias` plus
/// `TAP_CONE * lateral`, i.e. inside a cone from the receiver towards the light. Without that, a
/// tap slid along the receiver plane past a concave edge (below the floor at a wall's foot) finds
/// the adjacent surface in front of it and speckles a fully lit wall; a real blocker that close to
/// the receiver casts a sharp contact shadow anyway.
pub(crate) fn lookup_tap(
    map: &ShadowMap,
    world: Vec3,
    centre: Vec3,
    lateral: f32,
    bias: f32,
) -> f32 {
    let l = to_light(&map.view, world);
    if let LightView::Ortho { near, far, .. } = map.view
        && (l.z < near || l.z > far)
    {
        return 1.0;
    }
    let Some((x, y, _)) = to_map(&map.view, map.res, l) else {
        return 1.0;
    };
    if !(x >= 0.0 && y >= 0.0 && x < map.res as f32 && y < map.res as f32) {
        return 1.0;
    }
    let d = map.depth[y as usize * map.res + x as usize];
    let z = if lateral > 0.0 {
        l.z.min(to_light(&map.view, centre).z)
    } else {
        l.z
    };
    if z - bias - TAP_CONE * lateral > d {
        0.0
    } else {
        1.0
    }
}

/// Upper limit of the soft shadow width in world units (larger values, inf and NaN are clamped).
pub(crate) const MAX_SOFT: f32 = 64.0;

/// Soft shadow width from `Options::shadow_soft`: 0 for zero, negative or NaN, at most `MAX_SOFT`.
pub(crate) fn soft_width(soft: f32) -> f32 {
    if soft > 0.0 { soft.min(MAX_SOFT) } else { 0.0 }
}

/// Stepped soft shadow (0, 1/3, 2/3 or 1) at `pos` with surface normal `n` for light travelling
/// along `fwd`. Nine taps on a 3x3 grid spanning `soft` (world units) across the light's view,
/// each slid along `fwd` onto the receiver plane so a sloped surface does not shadow itself.
/// The pattern hangs on the world point and the light's axes only, so it does not move with the
/// camera. `lit` returns 1 (lit) or 0 for one tap at a point; its second argument is the tap's sideways
/// offset from `pos` while the centre is lit, else 0 (see `lookup_tap`). When the centre and the four corners agree, the
/// edge taps are skipped (an edge thinner than half the width can then be missed). The lit fraction is cut to thirds; `dither`
/// (0..1, an ordered screen pattern) decides between the two neighbouring steps.
pub(crate) fn soft_shadow(
    pos: Vec3,
    n: Vec3,
    fwd: Vec3,
    soft: f32,
    dither: f32,
    mut lit: impl FnMut(Vec3, f32) -> f32,
) -> f32 {
    let (right, up) = basis(fwd);
    let f = fwd.normalized();
    let nf = n.dot(f);
    let h = soft * 0.5;
    // A lit centre ignores blockers close behind its taps (`lookup_tap`'s cone): those come from
    // surfaces meeting the receiver at a concave edge. A shadowed centre counts every blocker, so a
    // contact shadow (floor at the foot of the wall that shades it) does not leak light.
    let centre = lit(pos, 0.0);
    let cone = if centre > 0.0 { h } else { 0.0 };
    let mut tap = |a: f32, b: f32| {
        let d = right * (a * h) + up * (b * h);
        // n . (d + f t) = 0; grazing surfaces: limit the slide
        let t = if nf.abs() > 1e-4 {
            (-n.dot(d) / nf).clamp(-2.0 * soft, 2.0 * soft)
        } else {
            0.0
        };
        let t = if t.is_finite() { t } else { 0.0 };
        lit(pos + d + f * t, (a * a + b * b).sqrt() * cone)
    };
    // centre and corners first: all equal = no edge inside the pattern (fully lit or umbra)
    let first = [
        centre,
        tap(-1.0, -1.0),
        tap(1.0, -1.0),
        tap(-1.0, 1.0),
        tap(1.0, 1.0),
    ];
    let mut sum: f32 = first.iter().sum();
    if sum == 0.0 || sum == 5.0 {
        return sum / 5.0;
    }
    for (a, b) in [(0.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
        sum += tap(a, b);
    }
    let v = sum / 3.0; // 0..3 in thirds
    let step = v.floor();
    let level = step + if v - step > dither + 1e-4 { 1.0 } else { 0.0 };
    (level / 3.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textures::AlphaTexture;

    fn tex(alpha: u8) -> AlphaTexture {
        AlphaTexture::new(2, 2, vec![alpha; 4]).unwrap()
    }

    fn sun_down_map() -> ShadowMap {
        let fwd = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = basis(fwd);
        ShadowMap::new(
            64,
            LightView::Ortho {
                origin: Vec3::new(0.0, 10.0, 0.0),
                right,
                up,
                fwd,
                half: 8.0,
                near: 0.0,
                far: 20.0,
            },
        )
    }

    fn quad_y1(texture: usize) -> Vec<Caster> {
        let a = Vec3::new(-1.0, 1.0, -1.0);
        let b = Vec3::new(1.0, 1.0, -1.0);
        let c = Vec3::new(1.0, 1.0, 1.0);
        let d = Vec3::new(-1.0, 1.0, 1.0);
        vec![
            Caster {
                tri: [a, b, c],
                uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                texture,
                wrap: false,
                alpha: 1.0,
                sun_only: false,
            },
            Caster {
                tri: [a, c, d],
                uv: [[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                texture,
                wrap: false,
                alpha: 1.0,
                sun_only: false,
            },
        ]
    }

    #[test]
    fn ortho_quad_shadows_point_below() {
        let mut m = sun_down_map();
        render_shadow(
            &mut m,
            &quad_y1(0),
            &[],
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(255)][..],
            None,
        );
        assert_eq!(lookup(&m, Vec3::new(0.0, 0.0, 0.0), 0.05), 0.0);
        assert_eq!(lookup(&m, Vec3::new(3.0, 0.0, 0.0), 0.05), 1.0);
        // the caster itself is not shadowed by itself
        assert_eq!(lookup(&m, Vec3::new(0.0, 1.0, 0.0), 0.05), 1.0);
    }

    #[test]
    fn alpha_texels_do_not_cast() {
        let mut m = sun_down_map();
        render_shadow(
            &mut m,
            &quad_y1(0),
            &[],
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(0)][..],
            None,
        );
        assert_eq!(lookup(&m, Vec3::new(0.0, 0.0, 0.0), 0.05), 1.0);
    }

    #[test]
    fn bad_texture_id_is_skipped() {
        let mut m = sun_down_map();
        render_shadow(
            &mut m,
            &quad_y1(5),
            &[],
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(255)][..],
            None,
        );
        assert_eq!(lookup(&m, Vec3::new(0.0, 0.0, 0.0), 0.05), 1.0);
    }

    /// Tiny deterministic generator for the brute-force tests (no rand crate).
    fn hash_f32(k: u64) -> f32 {
        let mut h = k.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xc2b2_ae3d_27d4_eb4f;
        h ^= h >> 31;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 29;
        (h >> 40) as f32 / (1u64 << 24) as f32
    }

    #[test]
    fn cube_light_matches_to_light() {
        let light = Vec3::new(1.25, 0.75, -3.5);
        let views = cube_views(light);
        let mut k = 0u64;
        let mut next = || {
            k += 1;
            hash_f32(k) * 2.0 - 1.0
        };
        for i in 0..20000 {
            let s = [0.001, 0.1, 1.0, 10.0, 1000.0][i % 5];
            let mut p = Vec3::new(
                light.x + next() * s,
                light.y + next() * s,
                light.z + next() * s,
            );
            // exact ties between axes and zero components
            match i % 7 {
                0 => p.y = light.y + (p.x - light.x),
                1 => p.z = light.z - (p.x - light.x),
                2 => p.x = light.x,
                _ => {}
            }
            let d = p - light;
            let (face, l) = cube_light(d);
            assert_eq!(face, cube_face(d));
            let want = to_light(&views[face], p);
            let same = |a: f32, b: f32| a.to_bits() == b.to_bits() || a == 0.0 && b == 0.0;
            assert!(
                same(l.x, want.x) && same(l.y, want.y) && same(l.z, want.z),
                "{d:?}: {l:?} vs {want:?}"
            );
            for (f, view) in views.iter().enumerate() {
                assert!(same(cube_depth(f, d), to_light(view, p).z));
            }
        }
    }

    #[test]
    fn cube_lookups_match_generic() {
        let light = Vec3::new(0.0, 1.0, 0.0);
        let mut maps = cube_views(light).map(|v| ShadowMap::new(32, v));
        let mut k = 1000u64;
        let mut next = || {
            k += 1;
            hash_f32(k)
        };
        for m in maps.iter_mut() {
            for d in m.depth.iter_mut() {
                let r = next();
                *d = if r < 0.3 { f32::INFINITY } else { r * 3.0 };
            }
        }
        for _ in 0..20000 {
            let p = Vec3::new(next() * 6.0 - 3.0, next() * 6.0 - 2.0, next() * 6.0 - 3.0);
            let c = p + Vec3::new(next() * 0.2 - 0.1, next() * 0.2 - 0.1, next() * 0.2 - 0.1);
            let (bias, lat) = (next() * 0.1, if next() < 0.5 { 0.0 } else { next() * 0.3 });
            let f = cube_face(p - light);
            assert_eq!(
                lookup_cube(&maps, p, light, bias),
                lookup(&maps[f], p, bias)
            );
            assert_eq!(
                lookup_cube_tap(&maps, p, c, light, lat, bias),
                lookup_tap(&maps[f], p, c, lat, bias)
            );
        }
    }

    #[test]
    fn row_spans_fill_the_same_texels() {
        let mut k = 7000u64;
        let mut next = || {
            k += 1;
            hash_f32(k)
        };
        let view = sun_down_map().view;
        let cover: Cover<[AlphaTexture]> = Cover {
            tex: None,
            wrap: false,
            opaque: true,
            alpha: 1.0,
        };
        let mut filled = 0;
        for i in 0..3000 {
            // small, large, thin and off-map triangles, some with shared x or y (flat edges)
            let s = [2.0, 20.0, 80.0, 300.0][i % 4];
            let mut t = [[0.0f32; 6]; 3];
            for v in t.iter_mut() {
                *v = [
                    next() * s - s * 0.3,
                    next() * s - s * 0.3,
                    0.5 + next(),
                    0.0,
                    0.0,
                    next() * 10.0,
                ];
                v[5] *= v[2];
            }
            if i % 5 == 0 {
                t[1][1] = t[0][1];
            }
            if i % 7 == 0 {
                t[2][0] = t[1][0];
            }
            if i % 11 == 0 {
                // texel-centre aligned vertices: edges exactly through texel centres
                for v in t.iter_mut() {
                    v[0] = v[0].floor() + 0.5;
                    v[1] = v[1].floor() + 0.5;
                }
            }
            let (mut a, mut b) = (ShadowMap::new(64, view), ShadowMap::new(64, view));
            fill_impl::<true, _>(&mut a, t, cover, f32::NEG_INFINITY, f32::INFINITY);
            fill_impl::<false, _>(&mut b, t, cover, f32::NEG_INFINITY, f32::INFINITY);
            assert!(
                a.depth
                    .iter()
                    .zip(&b.depth)
                    .all(|(x, y)| x.to_bits() == y.to_bits()),
                "triangle {i}: {t:?}"
            );
            filled += a.depth.iter().filter(|d| d.is_finite()).count();
        }
        assert!(filled > 100_000, "{filled}");
    }

    #[test]
    fn cube_face_picks_largest_axis() {
        assert_eq!(cube_face(Vec3::new(2.0, 1.0, -1.0)), 0);
        assert_eq!(cube_face(Vec3::new(-2.0, 1.0, 1.0)), 1);
        assert_eq!(cube_face(Vec3::new(0.1, 3.0, 1.0)), 2);
        assert_eq!(cube_face(Vec3::new(0.1, -3.0, 1.0)), 3);
        assert_eq!(cube_face(Vec3::new(0.1, 0.2, 1.0)), 4);
        assert_eq!(cube_face(Vec3::new(0.1, 0.2, -1.0)), 5);
    }

    #[test]
    fn point_light_cube_shadows() {
        // torch above the quad: point under the quad is dark, point far beside is lit
        let pos = Vec3::new(0.0, 3.0, 0.0);
        let mut maps: Vec<ShadowMap> = cube_views(pos)
            .iter()
            .map(|v| ShadowMap::new(64, *v))
            .collect();
        for m in maps.iter_mut() {
            render_shadow(
                m,
                &quad_y1(0),
                &[],
                Some(pos),
                Vec3::new(0.0, -1.0, 0.0),
                &[tex(255)][..],
                None,
            );
        }
        let at = |p: Vec3| lookup(&maps[cube_face(p - pos)], p, 0.05);
        assert_eq!(at(Vec3::new(0.0, 0.0, 0.0)), 0.0);
        assert_eq!(at(Vec3::new(4.0, 0.0, 0.0)), 1.0);
    }

    #[test]
    fn billboard_casts_towards_sun_and_light() {
        let bb = [BillboardCaster {
            base: Vec3::new(0.0, 0.0, 0.0),
            width: 1.0,
            height: 2.0,
            texture: 0,
            flip: false,
            scroll: None,
            alpha: 1.0,
        }];
        // sun straight down: thin upright quad still renders, no NaN
        let mut m = sun_down_map();
        render_shadow(
            &mut m,
            &[],
            &bb,
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(255)][..],
            None,
        );
        assert!(m.depth.iter().all(|d| !d.is_nan()));
        // slanted sun from +x: shadow falls to -x on the ground
        let fwd = Vec3::new(-1.0, -1.0, 0.0).normalized();
        let (right, up) = basis(fwd);
        let mut m = ShadowMap::new(
            64,
            LightView::Ortho {
                origin: Vec3::new(5.0, 5.0, 0.0),
                right,
                up,
                fwd,
                half: 8.0,
                near: 0.0,
                far: 30.0,
            },
        );
        render_shadow(&mut m, &[], &bb, None, fwd, &[tex(255)][..], None);
        assert_eq!(lookup(&m, Vec3::new(-1.0, 0.0, 0.0), 0.05), 0.0);
        assert_eq!(lookup(&m, Vec3::new(1.0, 0.0, 0.0), 0.05), 1.0);
        // point light beside it: shadow behind the billboard
        let pos = Vec3::new(3.0, 1.0, 0.0);
        let mut maps: Vec<ShadowMap> = cube_views(pos)
            .iter()
            .map(|v| ShadowMap::new(64, *v))
            .collect();
        for m in maps.iter_mut() {
            render_shadow(
                m,
                &[],
                &bb,
                Some(pos),
                Vec3::new(0.0, -1.0, 0.0),
                &[tex(255)][..],
                None,
            );
        }
        let p = Vec3::new(-2.0, 1.0, 0.0);
        assert_eq!(lookup(&maps[cube_face(p - pos)], p, 0.05), 0.0);
    }

    #[test]
    fn soft_lookup_is_stepped_and_widens_the_edge() {
        let mut m = sun_down_map();
        render_shadow(
            &mut m,
            &quad_y1(0),
            &[],
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(255)][..],
            None,
        );
        let up = Vec3::new(0.0, 1.0, 0.0);
        let fwd = Vec3::new(0.0, -1.0, 0.0);
        let at = |x: f32, d: f32| {
            soft_shadow(Vec3::new(x, 0.0, 0.3), up, fwd, 0.6, d, |p, _| {
                lookup(&m, p, 0.05)
            })
        };
        let mut seen = Vec::new();
        for k in 0..=40 {
            let x = 0.5 + k as f32 * 0.025;
            for d in [0.03, 0.5, 0.97] {
                let v = at(x, d);
                assert!(
                    [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0].contains(&v),
                    "stepped: {v}"
                );
                seen.push(v);
            }
        }
        // umbra inside, lit outside, steps in between
        assert_eq!(at(0.5, 0.5), 0.0);
        assert_eq!(at(1.5, 0.5), 1.0);
        assert!(seen.iter().any(|&v| v > 0.0 && v < 1.0));
        // monotone in the dither threshold: a lower threshold never gives less light
        assert!(at(1.0, 0.03) >= at(1.0, 0.97));
    }

    /// Lit share of the points under the quad, one per map texel (texel size 0.25).
    fn lit_share(m: &ShadowMap) -> f32 {
        let mut lit = 0;
        for i in 0..8 {
            for k in 0..8 {
                let p = Vec3::new(-0.875 + i as f32 * 0.25, 0.0, -0.875 + k as f32 * 0.25);
                lit += lookup(m, p, 0.05) as i32;
            }
        }
        lit as f32 / 64.0
    }

    #[test]
    fn see_through_caster_stipples_by_alpha() {
        let mut shares = Vec::new();
        for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let mut m = sun_down_map();
            let cs: Vec<Caster> = quad_y1(0)
                .into_iter()
                .map(|c| Caster { alpha, ..c })
                .collect();
            render_shadow(
                &mut m,
                &cs,
                &[],
                None,
                Vec3::new(0.0, -1.0, 0.0),
                &[tex(255)][..],
                None,
            );
            shares.push(lit_share(&m));
            if alpha == 0.5 {
                // checkered: of two neighbouring texels exactly one blocks
                let row = m.res / 2;
                for x in m.res / 2 - 3..m.res / 2 + 3 {
                    let a = m.depth[row * m.res + x].is_finite();
                    let b = m.depth[row * m.res + x + 1].is_finite();
                    let c = m.depth[(row + 1) * m.res + x].is_finite();
                    assert!(a != b && a != c, "checkered at texel {x}");
                }
            }
        }
        // lit share falls with alpha, about 1 - alpha
        for (k, want) in [1.0f32, 0.75, 0.5, 0.25, 0.0].iter().enumerate() {
            assert!(
                (shares[k] - want).abs() < 0.01,
                "alpha {}: lit share {} (want {want})",
                k as f32 * 0.25,
                shares[k]
            );
        }
        // half-transparent texels count as well (draw alpha times texel alpha)
        let mut m = sun_down_map();
        let cs: Vec<Caster> = quad_y1(0)
            .into_iter()
            .map(|c| Caster { alpha: 0.999, ..c })
            .collect();
        render_shadow(
            &mut m,
            &cs,
            &[],
            None,
            Vec3::new(0.0, -1.0, 0.0),
            &[tex(128)][..],
            None,
        );
        assert_eq!(lit_share(&m), 0.5);
    }

    /// The sun map moves with the camera in whole texels; the stipple pattern stays on the world.
    #[test]
    fn stipple_pattern_fixed_in_light_space() {
        let fwd = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = basis(fwd);
        let texel = 16.0 / 64.0;
        let map_at = |shift: f32| {
            let origin =
                Vec3::new(0.0, 10.0, 0.0) + right * (shift * texel) + up * (2.0 * shift * texel);
            let mut m = ShadowMap::new(
                64,
                LightView::Ortho {
                    origin,
                    right,
                    up,
                    fwd,
                    half: 8.0,
                    near: 0.0,
                    far: 20.0,
                },
            );
            let cs: Vec<Caster> = quad_y1(0)
                .into_iter()
                .map(|c| Caster { alpha: 0.5, ..c })
                .collect();
            render_shadow(&mut m, &cs, &[], None, fwd, &[tex(255)][..], None);
            m
        };
        let (a, b) = (map_at(0.0), map_at(3.0));
        let mut differ = 0;
        for i in 0..40 {
            for k in 0..40 {
                let p = Vec3::new(-0.95 + i as f32 * 0.0487, 0.0, -0.95 + k as f32 * 0.0487);
                differ += (lookup(&a, p, 0.05) != lookup(&b, p, 0.05)) as i32;
            }
        }
        assert_eq!(differ, 0, "pattern moved with the map");
    }

    #[test]
    fn outside_map_is_lit() {
        let m = sun_down_map();
        assert_eq!(lookup(&m, Vec3::new(100.0, 0.0, 0.0), 0.05), 1.0);
    }

    #[test]
    fn band_parallel_matches_single_pass_bit_for_bit() {
        let fwd = Vec3::new(0.4, -0.7, 0.5).normalized();
        let (right, up) = basis(fwd);
        let view = LightView::Ortho {
            origin: Vec3::new(1.0, 2.0, 1.0),
            right,
            up,
            fwd,
            half: 6.0,
            near: -10.0,
            far: 20.0,
        };
        let res = 128usize;
        let tex_opaque = AlphaTexture::new(4, 4, vec![255; 16]).unwrap();
        let mut hole_px = vec![255u8; 16];
        hole_px[0] = 0; // transparent corner
        hole_px[1] = 0;
        let tex_holes = AlphaTexture::new(4, 4, hole_px).unwrap();
        let textures = [tex_opaque, tex_holes];
        let textures = &textures[..];
        let casters = [
            // opaque quad
            Caster {
                tri: [
                    Vec3::new(-2.0, 1.0, -2.0),
                    Vec3::new(2.0, 1.0, -2.0),
                    Vec3::new(2.0, 1.0, 2.0),
                ],
                uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                texture: 0,
                wrap: false,
                alpha: 1.0,
                sun_only: false,
            },
            // alpha-tested triangle with varying UVs
            Caster {
                tri: [
                    Vec3::new(-1.0, 1.5, -1.0),
                    Vec3::new(1.5, 1.5, 0.0),
                    Vec3::new(0.0, 1.5, 2.0),
                ],
                uv: [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
                texture: 1,
                wrap: false,
                alpha: 1.0,
                sun_only: false,
            },
            // constant-UV triangle on a texture with holes (opaque texel at 0.625, 0.625)
            Caster {
                tri: [
                    Vec3::new(-3.0, 0.8, 0.0),
                    Vec3::new(-1.0, 0.8, 1.5),
                    Vec3::new(-2.0, 0.8, 2.5),
                ],
                uv: [[0.625, 0.625]; 3],
                texture: 1,
                wrap: true,
                alpha: 1.0,
                sun_only: false,
            },
            // constant-UV triangle on a transparent texel (0.125, 0.125)
            Caster {
                tri: [
                    Vec3::new(1.0, 0.8, -3.0),
                    Vec3::new(3.0, 0.8, -2.0),
                    Vec3::new(2.0, 0.8, -1.0),
                ],
                uv: [[0.125, 0.125]; 3],
                texture: 1,
                wrap: true,
                alpha: 1.0,
                sun_only: false,
            },
            // stippled see-through caster
            Caster {
                tri: [
                    Vec3::new(-1.0, 2.0, -2.0),
                    Vec3::new(2.0, 2.0, -1.0),
                    Vec3::new(0.0, 2.0, 1.0),
                ],
                uv: [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
                texture: 0,
                wrap: false,
                alpha: 0.5,
                sun_only: false,
            },
        ];
        let mut ref_map = ShadowMap::new(res, view);
        render_shadow(&mut ref_map, &casters, &[], None, fwd, textures, None);

        let mut prepared = Vec::new();
        for c in &casters {
            let tex = caster_texture(c, textures).unwrap();
            prepare_caster(&view, res, c, tex, c.texture == 0, &mut prepared);
        }
        let (near, far) = view_depth_range(&view);
        let (ox, oy) = pattern_origin(&view, res);
        let mut banded = vec![f32::INFINITY; res * res];
        let band_h = 17; // intentionally non-divisor of 128
        for (k, slice) in banded.chunks_mut(band_h * res).enumerate() {
            let y0 = k * band_h;
            let y1 = y0 + slice.len() / res;
            for tri in &prepared {
                if tri.y1 >= y0 as i64 && tri.y0 < y1 as i64 {
                    fill_band(slice, res, y0, y1, tri, textures, near, far, ox, oy);
                }
            }
        }
        assert!(
            ref_map
                .depth
                .iter()
                .zip(&banded)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }
}
