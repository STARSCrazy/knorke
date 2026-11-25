//! Triangle rasterizer: frustum clipping in view space, integer edge functions with 1/16 pixel
//! sub-pixel precision and the top-left fill rule (shared edges are drawn exactly once),
//! perspective-correct texture coordinates, depth test.

use crate::blend::{Blend, Frag, FragStore, NO_FRAG, step_alpha};
use crate::math::Vec3;
use crate::texture::Texture;

const SUB: i64 = 16; // sub-pixel steps per pixel

/// 4x4 ordered dither for the choice between two texture levels.
const BAYER: [f32; 16] = [
    0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0,
];

/// One vertex in view space with its attributes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClipVert {
    pub(crate) pos: Vec3, // view space: x right, y up, z depth
    pub(crate) u: f32,
    pub(crate) v: f32,
    pub(crate) light: f32,
}

/// Per-pixel geometry buffer filled by the rasterizer.
pub(crate) struct GBuffer {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) color: Vec<[u8; 3]>,
    pub(crate) light: Vec<f32>,
    /// View depth (distance along the view direction); infinity where nothing was drawn.
    pub(crate) depth: Vec<f32>,
    /// World-space unit normal; zero where nothing was drawn.
    pub(crate) normal: Vec<Vec3>,
    /// Pick id per pixel (0 = nothing); only written while `pick` is set.
    pub(crate) id: Vec<u32>,
    pub(crate) pick: bool,
    /// Per pixel: colour is final (particles), light ignored. Only cleared and kept up to date once
    /// `prelit_any` is set, so frames without particles pay nothing.
    pub(crate) prelit: Vec<bool>,
    pub(crate) prelit_any: bool,
    /// While `pick` is set: depth and normal of the surface under each pixel a particle covers
    /// (particles keep the pick id of what is behind them).
    pub(crate) pick_under: std::collections::BTreeMap<usize, (f32, Vec3)>,
    /// Transparent fragments (see `blend`); never touch the buffers above.
    pub(crate) frags: FragStore,
    /// Per pixel: the particle (prelit pixels) or billboard sprite that drew it, else `NO_OWNER`.
    /// All `NO_OWNER` while `owner_any` is false; then frames without particles and billboards
    /// pay nothing.
    pub(crate) owner: Vec<u32>,
    pub(crate) owner_any: bool,
    /// Owner written by the draw call in progress (set around billboard sprites).
    pub(crate) cur_owner: u32,
}

/// `GBuffer::owner`: drawn by nothing that needs it.
pub(crate) const NO_OWNER: u32 = u32::MAX;

impl GBuffer {
    pub(crate) fn new(width: usize, height: usize) -> GBuffer {
        let n = width * height;
        GBuffer {
            width,
            height,
            color: vec![[0; 3]; n],
            light: vec![0.0; n],
            depth: vec![f32::INFINITY; n],
            normal: vec![Vec3::default(); n],
            id: vec![0; n],
            pick: false,
            prelit: vec![false; n],
            prelit_any: false,
            frags: FragStore::new(n),
            pick_under: std::collections::BTreeMap::new(),
            owner: vec![NO_OWNER; n],
            owner_any: false,
            cur_owner: NO_OWNER,
        }
    }
    pub(crate) fn clear(&mut self) {
        self.color.fill([0; 3]);
        self.light.fill(0.0);
        self.depth.fill(f32::INFINITY);
        self.normal.fill(Vec3::default());
        if self.pick {
            self.id.fill(0);
        }
        if self.prelit_any {
            self.prelit.fill(false);
            self.prelit_any = false;
        }
        self.frags.clear();
        if self.owner_any {
            self.owner.fill(NO_OWNER);
            self.owner_any = false;
        }
        self.cur_owner = NO_OWNER;
        if !self.pick_under.is_empty() {
            self.pick_under.clear();
        }
    }
}

/// Frustum parameters for clipping and projection.
#[derive(Clone, Copy)]
pub(crate) struct Proj {
    pub(crate) near: f32,
    /// Side slopes left, right, top, bottom (asymmetric with a lens shift).
    pub(crate) tan_l: f32,
    pub(crate) tan_r: f32,
    pub(crate) tan_t: f32,
    pub(crate) tan_b: f32,
    pub(crate) focal: f32,
    /// Projection centre in pixels.
    pub(crate) cx: f32,
    pub(crate) cy: f32,
}

/// Draw settings for one triangle.
pub(crate) struct Material<'a> {
    pub(crate) texture: &'a Texture,
    pub(crate) wrap: bool,
    pub(crate) cull_back: bool,
    /// World-space face normal; written to the normal buffer.
    pub(crate) normal: Vec3,
    /// Pick a smaller texture level for far surfaces (Options::texture_lod); lod_bias shifts the choice.
    pub(crate) lod: bool,
    pub(crate) lod_bias: f32,
    /// Ordered dither between two levels; false = nearest level.
    pub(crate) lod_dither: bool,
    /// One level for the whole face (set by the caller from the face centre); NaN = per pixel.
    pub(crate) face_lod: f32,
    /// Pick id written to the id buffer (GBuffer::pick).
    pub(crate) id: u32,
    /// Some = transparent: fragments go to `GBuffer::frags` (no depth, normal or pick writes).
    pub(crate) blend: Option<Blend>,
}

fn lex_less(a: &ClipVert, b: &ClipVert) -> bool {
    (a.pos.x, a.pos.y, a.pos.z, a.u, a.v) < (b.pos.x, b.pos.y, b.pos.z, b.u, b.v)
}

/// Intersection of edge a-b with a plane, given signed distances. The endpoints are put in a fixed
/// order first, so two triangles that share an edge get bit-identical new vertices (no cracks).
fn intersect(a: &ClipVert, da: f32, b: &ClipVert, db: f32) -> ClipVert {
    let (a, da, b, db) = if lex_less(b, a) {
        (b, db, a, da)
    } else {
        (a, da, b, db)
    };
    let t = da / (da - db);
    ClipVert {
        pos: a.pos.lerp(b.pos, t),
        u: a.u + (b.u - a.u) * t,
        v: a.v + (b.v - a.v) * t,
        light: a.light + (b.light - a.light) * t,
    }
}

/// A clipped convex polygon on the stack: a triangle clipped by five planes has at most
/// 3 + 5 = 8 vertices, so clipping never allocates (steady-state frames allocate nothing).
#[derive(Clone, Copy)]
pub(crate) struct Poly {
    v: [ClipVert; Poly::MAX],
    n: usize,
}

impl Poly {
    const MAX: usize = 9;
    const EMPTY: Poly = Poly {
        v: [ClipVert {
            pos: Vec3::ZERO,
            u: 0.0,
            v: 0.0,
            light: 0.0,
        }; Poly::MAX],
        n: 0,
    };

    fn of(tri: &[ClipVert; 3]) -> Poly {
        let mut p = Poly::EMPTY;
        p.v[..3].copy_from_slice(tri);
        p.n = 3;
        p
    }

    fn push(&mut self, c: ClipVert) {
        self.v[self.n] = c;
        self.n += 1;
    }
}

impl std::ops::Deref for Poly {
    type Target = [ClipVert];
    fn deref(&self) -> &[ClipVert] {
        &self.v[..self.n]
    }
}

fn clip_plane(input: &Poly, out: &mut Poly, dist: impl Fn(&ClipVert) -> f32) {
    out.n = 0;
    let n = input.len();
    for i in 0..n {
        let a = &input[i];
        let b = &input[(i + 1) % n];
        let da = dist(a);
        let db = dist(b);
        if da >= 0.0 {
            out.push(*a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(intersect(a, da, b, db));
        }
    }
}

/// Clip a triangle against the view frustum (near plane and the four sides). Returns a convex polygon.
pub(crate) fn clip_triangle(tri: &[ClipVert; 3], p: &Proj) -> Poly {
    let all_in = tri.iter().all(|v| {
        v.pos.z >= p.near
            && v.pos.x <= v.pos.z * p.tan_r
            && -v.pos.x <= v.pos.z * p.tan_l
            && v.pos.y <= v.pos.z * p.tan_t
            && -v.pos.y <= v.pos.z * p.tan_b
    });
    if all_in {
        return Poly::of(tri);
    }
    // Trivial reject: all vertices outside one plane.
    let outside = |f: &dyn Fn(&ClipVert) -> f32| tri.iter().all(|v| f(v) < 0.0);
    let planes: [&dyn Fn(&ClipVert) -> f32; 5] = [
        &|v| v.pos.z - p.near,
        &|v| v.pos.z * p.tan_r - v.pos.x,
        &|v| v.pos.z * p.tan_l + v.pos.x,
        &|v| v.pos.z * p.tan_t - v.pos.y,
        &|v| v.pos.z * p.tan_b + v.pos.y,
    ];
    if planes.iter().any(|f| outside(*f)) {
        return Poly::EMPTY;
    }
    let mut a = Poly::of(tri);
    let mut b = Poly::EMPTY;
    for f in planes {
        clip_plane(&a, &mut b, f);
        std::mem::swap(&mut a, &mut b);
        if a.len() < 3 {
            return Poly::EMPTY;
        }
    }
    a
}

/// Screen-space vertex: fixed-point position plus attributes divided by depth.
#[derive(Clone, Copy)]
struct ScreenVert {
    x: i64,
    y: i64,
    inv_z: f32,
    u_z: f32,
    v_z: f32,
    l_z: f32,
}

fn project(v: &ClipVert, p: &Proj) -> ScreenVert {
    let inv_z = 1.0 / v.pos.z;
    let sx = p.cx + v.pos.x * inv_z * p.focal;
    let sy = p.cy - v.pos.y * inv_z * p.focal;
    ScreenVert {
        x: (sx * SUB as f32).round() as i64,
        y: (sy * SUB as f32).round() as i64,
        inv_z,
        u_z: v.u * inv_z,
        v_z: v.v * inv_z,
        l_z: v.light * inv_z,
    }
}

#[inline]
fn edge(a: &ScreenVert, b: &ScreenVert, px: i64, py: i64) -> i64 {
    (b.x - a.x) * (py - a.y) - (b.y - a.y) * (px - a.x)
}

/// Top-left rule for screen y pointing down and positive (clockwise on screen) winding.
#[inline]
fn is_top_left(a: &ScreenVert, b: &ScreenVert) -> bool {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    (dy == 0 && dx > 0) || dy < 0
}

/// Rasterize a view-space triangle into the G-buffer.
pub(crate) fn draw_triangle(g: &mut GBuffer, p: &Proj, tri: &[ClipVert; 3], m: &Material) {
    let poly = clip_triangle(tri, p);
    if poly.len() < 3 {
        return;
    }
    // The clipped polygon is convex and planar: fan triangulation keeps the winding.
    let s0 = project(&poly[0], p);
    let mut prev = project(&poly[1], p);
    for v in &poly[2..] {
        let next = project(v, p);
        fill(g, [s0, prev, next], m, &mut |_, _| {});
        prev = next;
    }
}

/// Fill one screen triangle. `visit` is called for every covered pixel (used by tests).
/// Out of line so the opaque fill loop stays as small as before transparency existed.
#[inline(never)]
#[cold]
fn push_frag(frags: &mut FragStore, i: usize, f: Frag) {
    frags.push(i, f);
}

fn fill(
    g: &mut GBuffer,
    mut t: [ScreenVert; 3],
    m: &Material,
    visit: &mut dyn FnMut(usize, usize),
) {
    let mut area = edge(&t[0], &t[1], t[2].x, t[2].y);
    if area == 0 {
        return;
    }
    if area < 0 {
        if m.cull_back {
            return;
        }
        t.swap(1, 2);
        area = -area;
    }
    let w = g.width as i64;
    let h = g.height as i64;
    let minx = t.iter().map(|v| v.x).min().unwrap();
    let maxx = t.iter().map(|v| v.x).max().unwrap();
    let miny = t.iter().map(|v| v.y).min().unwrap();
    let maxy = t.iter().map(|v| v.y).max().unwrap();
    // Pixel centers at (x + 0.5) * SUB.
    let x0 = ((minx - SUB / 2).div_euclid(SUB)).max(0);
    let x1 = ((maxx - SUB / 2).div_euclid(SUB) + 1).min(w - 1);
    let y0 = ((miny - SUB / 2).div_euclid(SUB)).max(0);
    let y1 = ((maxy - SUB / 2).div_euclid(SUB) + 1).min(h - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let bias = [
        if is_top_left(&t[1], &t[2]) { 0 } else { 1 },
        if is_top_left(&t[2], &t[0]) { 0 } else { 1 },
        if is_top_left(&t[0], &t[1]) { 0 } else { 1 },
    ];
    let inv_area = 1.0 / area as f32;
    let normal = m.normal;
    // Edge values are affine in the pixel centre: step them per pixel and row instead of
    // recomputing. Exact in i64, so coverage and barycentrics stay bit-identical.
    let dx = [
        -(t[2].y - t[1].y) * SUB,
        -(t[0].y - t[2].y) * SUB,
        -(t[1].y - t[0].y) * SUB,
    ];
    let dy = [
        (t[2].x - t[1].x) * SUB,
        (t[0].x - t[2].x) * SUB,
        (t[1].x - t[0].x) * SUB,
    ];
    let (cx0, cy0) = (x0 * SUB + SUB / 2, y0 * SUB + SUB / 2);
    let mut row_e = [
        edge(&t[1], &t[2], cx0, cy0),
        edge(&t[2], &t[0], cx0, cy0),
        edge(&t[0], &t[1], cx0, cy0),
    ];
    for py in y0..=y1 {
        let cy = py * SUB + SUB / 2;
        let row = py as usize * g.width;
        let mut e = row_e;
        for k in 0..3 {
            row_e[k] += dy[k];
        }
        for px in x0..=x1 {
            let [e0, e1, e2] = e;
            for k in 0..3 {
                e[k] += dx[k];
            }
            if e0 < bias[0] || e1 < bias[1] || e2 < bias[2] {
                continue;
            }
            let cx = px * SUB + SUB / 2;
            let b0 = e0 as f32 * inv_area;
            let b1 = e1 as f32 * inv_area;
            let b2 = e2 as f32 * inv_area;
            let inv_z = b0 * t[0].inv_z + b1 * t[1].inv_z + b2 * t[2].inv_z;
            if inv_z <= 0.0 {
                continue;
            }
            let z = 1.0 / inv_z;
            let i = row + px as usize;
            if z >= g.depth[i] {
                continue;
            }
            let u = (b0 * t[0].u_z + b1 * t[1].u_z + b2 * t[2].u_z) * z;
            let v = (b0 * t[0].v_z + b1 * t[1].v_z + b2 * t[2].v_z) * z;
            let tex = if m.lod && !m.face_lod.is_nan() {
                // between two levels: an ordered dither pattern picks the farther one for a growing share
                // of the pixels, so the switch fades in while moving instead of jumping
                let l = m.face_lod + m.lod_bias;
                let frac = l - l.floor();
                let thr = if m.lod_dither {
                    (BAYER[(py as usize & 3) * 4 + (px as usize & 3)] + 0.5) / 16.0
                } else {
                    0.5
                };
                m.texture.level(if frac > thr {
                    l.floor() + 1.0
                } else {
                    l.floor()
                })
            } else if m.lod {
                // texture coordinates one pixel to the right and one down: the footprint of this pixel
                let at = |dx: i64, dy: i64| {
                    let f0 = edge(&t[1], &t[2], cx + dx, cy + dy) as f32 * inv_area;
                    let f1 = edge(&t[2], &t[0], cx + dx, cy + dy) as f32 * inv_area;
                    let f2 = edge(&t[0], &t[1], cx + dx, cy + dy) as f32 * inv_area;
                    let iz = f0 * t[0].inv_z + f1 * t[1].inv_z + f2 * t[2].inv_z;
                    let zz = if iz > 0.0 { 1.0 / iz } else { z };
                    (
                        (f0 * t[0].u_z + f1 * t[1].u_z + f2 * t[2].u_z) * zz,
                        (f0 * t[0].v_z + f1 * t[1].v_z + f2 * t[2].v_z) * zz,
                    )
                };
                let (ux, vx) = at(SUB, 0);
                let (uy, vy) = at(0, SUB);
                let tw = m.texture.width as f32;
                let th = m.texture.height as f32;
                let fx = (((ux - u) * tw).powi(2) + ((vx - v) * th).powi(2)).sqrt();
                let fy = (((uy - u) * tw).powi(2) + ((vy - v) * th).powi(2)).sqrt();
                // dithered between the two nearest levels (see above): a soft band instead of a seam
                let l = fx.max(fy).max(1e-6).log2() + m.lod_bias;
                let thr = if m.lod_dither {
                    (BAYER[(py as usize & 3) * 4 + (px as usize & 3)] + 0.5) / 16.0
                } else {
                    0.5
                };
                m.texture.level(if l - l.floor() > thr {
                    l.floor() + 1.0
                } else {
                    l.floor()
                })
            } else {
                m.texture
            };
            let texel = if m.wrap {
                tex.sample(u, v)
            } else {
                tex.sample_clamp(u, v)
            };
            if let Some(b) = m.blend {
                let a = step_alpha(b.alpha * texel[3] as f32 / 255.0);
                if a > 0.0 {
                    let light = (b0 * t[0].l_z + b1 * t[1].l_z + b2 * t[2].l_z) * z;
                    push_frag(
                        &mut g.frags,
                        i,
                        Frag {
                            depth: z,
                            color: [texel[0], texel[1], texel[2]],
                            mode: b.mode,
                            alpha: a,
                            light,
                            normal,
                            order: b.order,
                            prelit: false,
                            owner: g.cur_owner,
                            next: NO_FRAG,
                        },
                    );
                    visit(px as usize, py as usize);
                }
                continue;
            }
            if texel[3] < 128 {
                continue;
            }
            let l = (b0 * t[0].l_z + b1 * t[1].l_z + b2 * t[2].l_z) * z;
            g.depth[i] = z;
            g.color[i] = [texel[0], texel[1], texel[2]];
            g.light[i] = l;
            if g.prelit_any {
                g.prelit[i] = false;
            }
            if g.owner_any {
                g.owner[i] = g.cur_owner;
            }
            g.normal[i] = normal;
            if g.pick {
                g.id[i] = m.id;
            }
            visit(px as usize, py as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(x: f32, y: f32) -> ScreenVert {
        ScreenVert {
            x: (x * SUB as f32).round() as i64,
            y: (y * SUB as f32).round() as i64,
            inv_z: 1.0,
            u_z: 0.0,
            v_z: 0.0,
            l_z: 1.0,
        }
    }

    /// Triangles sharing edges must cover every pixel of their union exactly once.
    #[test]
    fn shared_edges_are_watertight() {
        let tex = Texture::from_rgba(1, 1, &[255, 255, 255, 255]).unwrap();
        let m = Material {
            texture: &tex,
            wrap: false,
            cull_back: false,
            normal: Vec3::default(),
            lod: false,
            lod_bias: 0.0,
            lod_dither: true,
            face_lod: f32::NAN,
            id: 0,
            blend: None,
        };
        let (w, h) = (40, 30);
        let mut g = GBuffer::new(w, h);
        let mut hits = vec![0u32; w * h];
        // A fan of triangles around an odd center point, with odd sub-pixel coordinates.
        let c = sv(19.3, 14.7);
        let ring = [sv(2.1, 1.3), sv(37.9, 2.6), sv(38.2, 28.1), sv(1.7, 27.4)];
        for k in 0..4 {
            g.depth.fill(f32::INFINITY);
            fill(&mut g, [c, ring[k], ring[(k + 1) % 4]], &m, &mut |x, y| {
                hits[y * w + x] += 1
            });
        }
        assert!(hits.iter().all(|&n| n <= 1), "a pixel was drawn twice");
        // Inner area must be fully covered.
        for y in 4..26 {
            for x in 4..36 {
                assert_eq!(hits[y * w + x], 1, "gap at {x},{y}");
            }
        }
    }

    #[test]
    fn clipping_keeps_triangle_in_front() {
        let p = Proj {
            near: 0.1,
            tan_l: 1.0,
            tan_r: 1.0,
            tan_t: 1.0,
            tan_b: 1.0,
            focal: 10.0,
            cx: 10.0,
            cy: 10.0,
        };
        let v = |x, y, z| ClipVert {
            pos: Vec3::new(x, y, z),
            u: 0.0,
            v: 0.0,
            light: 1.0,
        };
        let poly = clip_triangle(
            &[v(0.0, 0.0, -1.0), v(1.0, 0.0, 2.0), v(-1.0, 0.0, 2.0)],
            &p,
        );
        assert!(poly.len() >= 3);
        assert!(poly.iter().all(|c| c.pos.z >= 0.1 - 1e-6));
        let behind = clip_triangle(
            &[v(0.0, 0.0, -1.0), v(1.0, 0.0, -2.0), v(-1.0, 0.0, -2.0)],
            &p,
        );
        assert!(behind.is_empty());
    }
}
