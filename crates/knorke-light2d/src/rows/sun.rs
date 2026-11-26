//! Suns as row-style lights (plan 17.13, L3): a directional light's shadow from a shadow map
//! built once instead of Chiaro's march per pixel through the distance field.
//!
//! The map lies over the occlusion mask turned to the sun: a texel is [`SUN_ALONG`] mask texels
//! long toward the sun and [`SUN_ACROSS`] across. Per texel it holds the receiver heights a ray
//! toward the sun from the texel's middle finds shadowed, merged into two intervals (more are
//! hulled); a pixel reads the nearest texel and moves its height exactly along the light, so only
//! the offset across the light (at most an eighth of a texel) is an error. Measured against an
//! exact march: about half of the error of Chiaro's march (plan 17.13, L3).
//!
//! Partly transparent obstacles (E25) and pattern obstacles are runs of their own, two per texel
//! in two more targets: a run of texels with the same interval lets through 1 - o of its first
//! texel (o the mask's opacity), once per crossing as Chiaro's trace; on a pattern obstacle the
//! light reads its pattern where the ray enters, at the receiver's ray height there, as the
//! rows do (`lightrow_pattern.hlsli`). A third run joins the second (their intervals hulled,
//! their mean passes multiplied).
//!
//! A soft sun (a source radius) keeps how far along the ray each interval's ends lie in a second
//! target; its pixel blends the four nearest texels and widens the ends to ramps as high as the
//! sun's disc spreads the rays' rise there (`LightList::directional_rise`). The rays' spread
//! across the light is not in the map. Again about half of the error of Chiaro's march.
//!
//! The texels sit on a lattice fixed in the world ([`SunGrid`]) and are stored wrapped around
//! the texture (texel (i, j) of the lattice at (i mod w, j mod h)), and the build marches in
//! world texels: a texel's contents depend only on the sun, the finite obstacles along its ray
//! and where the mask ends, not on where the camera is. So the map is kept while sun, mask size
//! and heights stay ([`SunCache`](super::SunCache)); a pan by whole texels or a redrawn rectangle
//! rebuilds only the texels whose rays reach what changed ([`sun_bands`]), without copying.
//!
//! The shaders are `lightrow_sun*` in `knorke-integration/shaders`.

use knorke_math::{det, uniform_block};

use super::{MaskChange, Refresh, RowMask, Source};
use crate::gpu_light::GpuDirectional;
use crate::light_culling::Casters;
use crate::shadow::DirtyRectList;

/// Most suns with a map per frame; further ones stay with Chiaro's programs.
pub const MAX_SUNS: usize = 2;
/// Mask texels a map texel spans toward the sun.
pub const SUN_ALONG: f32 = 1.0;
/// Mask texels a map texel spans across the sun's direction.
pub const SUN_ACROSS: f32 = 0.25;
/// Longest side of a map in texels (the map spans the mask's diagonal both ways); larger masks
/// keep their suns with Chiaro's programs. The build marches at most 4096 texels
/// (`kMaxSteps` in `lightrow_sun_build.frag`), more than such a mask's width plus height.
pub const MAX_SUN_MAP: u32 = 8192;
/// Texels before the receiver that do not count as occluders (Chiaro's trace: 1).
const SUN_RECEIVER_GAP: f32 = 1.0;

/// Whether a directional light of [`LightList::directional`](crate::light_culling::LightList)
/// can take its shadow from a map, in a frame [`super::frame_suits`] accepts: it casts shadows
/// from every caster (E53), does not stand overhead
/// and does not shine from below the horizon. All others stay with Chiaro's programs.
pub fn sun_suits(g: &GpuDirectional, casters: Casters) -> bool {
    let [x, y, slope, _] = g.dir_slope;
    g.color[3] > 0.5 && casters == Casters::All && (x != 0.0 || y != 0.0) && slope > 0.0
}

/// How a sun's map lies over the mask. Lattice texel (I, J)'s middle is at
/// `(I + 0.5) * a + (J + 0.5) * b` in world texels (mask texel minus [`SunGrid::offset`]),
/// `a` toward the sun, `b` across; the map holds the `size` texels from `corner` on, which
/// cover the mask. Its size depends on the mask's size alone, so a turning sun keeps its texture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunGrid {
    /// Step of lattice texel I.
    pub a: [f32; 2],
    /// Step of lattice texel J.
    pub b: [f32; 2],
    /// The lattice texel of the map's first texel.
    pub corner: [i32; 2],
    /// The corner of lattice texel `corner` in mask texels.
    pub origin: [f32; 2],
    /// Map size in texels.
    pub size: [u32; 2],
    /// Mask texel minus world texel: [`mask_offset`] of the anchor.
    pub offset: [i32; 2],
}

/// Mask texel minus world texel for a mask anchored at `snapped` (`MaskAnchor::snapped`): a pan
/// moves the mask's contents by the change of `snapped` (`add_exposed_strips` of the mask plan).
pub fn mask_offset(snapped: [i32; 2]) -> [i32; 2] {
    snapped
}

impl SunGrid {
    /// Map size for a mask of `mask` texels: its diagonal along and across, and a texel on
    /// each side for the lattice not being centred.
    pub fn size_of(mask: [u32; 2]) -> [u32; 2] {
        let [w, h] = mask.map(|v| v as f32);
        let diagonal = (w * w + h * h).sqrt();
        [SUN_ALONG, SUN_ACROSS].map(|step| (diagonal / step).ceil() as u32 + 2)
    }

    /// The grid over a mask of `mask` texels anchored at `snapped` for the unit direction `d`
    /// toward the sun (mask texels).
    pub fn new(mask: [u32; 2], d: [f32; 2], snapped: [i32; 2]) -> Self {
        let size = Self::size_of(mask);
        let a = [d[0] * SUN_ALONG, d[1] * SUN_ALONG];
        let b = [-d[1] * SUN_ACROSS, d[0] * SUN_ACROSS];
        let (a64, b64) = (a.map(f64::from), b.map(f64::from));
        let offset = mask_offset(snapped);
        // The mask's middle in world texels, in lattice coordinates.
        let centre = [0, 1].map(|k| f64::from(mask[k]) * 0.5 - f64::from(offset[k]));
        let along = dot64(centre, a64) / dot64(a64, a64);
        let across = dot64(centre, b64) / dot64(b64, b64);
        let corner = [
            (along - f64::from(size[0]) * 0.5).floor() as i32,
            (across - f64::from(size[1]) * 0.5).floor() as i32,
        ];
        let (i, j) = (f64::from(corner[0]), f64::from(corner[1]));
        Self {
            a,
            b,
            corner,
            origin: [0, 1].map(|k| (i * a64[k] + j * b64[k] + f64::from(offset[k])) as f32),
            size,
            offset,
        }
    }

    /// The rows that give a mask point p's map texel coordinates: (dot(xy, p - origin),
    /// dot(zw, p - origin)).
    pub fn inverse(&self) -> [f32; 4] {
        let (a, b) = (self.a, self.b);
        let (a2, b2) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1]);
        [a[0] / a2, a[1] / a2, b[0] / b2, b[1] / b2]
    }
}

fn dot64(p: [f64; 2], q: [f64; 2]) -> f64 {
    p[0] * q[0] + p[1] * q[1]
}

uniform_block! {
    /// `SunParams` of `lightrow_sun.hlsli`: the sun's map build and its light.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SunParams[160] {
        /// The light as Chiaro's directional programs get it.
        pub light: GpuDirectional,
        /// xy mask size in texels, z tolerance below a top, w highest finite top.
        pub domain: [f32; 4],
        /// [`SunGrid::a`] and [`SunGrid::b`].
        pub axes: [f32; 4],
        /// xy [`SunGrid::origin`], zw [`SunGrid::size`].
        pub origin: [f32; 4],
        /// [`SunGrid::inverse`].
        pub inverse: [f32; 4],
        /// x texels before the receiver that do not count (Chiaro: 1), y the lowest receiver
        /// height a map texel stands in for, z a soft sun's rise (`LightList::directional_rise`,
        /// 0: hard), w the height the build's march rises by per texel.
        pub build: [f32; 4],
        /// The bits of `i32`s: xy [`SunGrid::corner`], zw [`SunGrid::offset`].
        pub lattice: [f32; 4],
        /// x 1: the map keeps partly transparent runs (the build reads the mask's opacity), y 1:
        /// the build reads the pattern ids too ([`SunParams::with_partial`]).
        pub partial: [f32; 4],
    }
}

impl SunParams {
    /// The block of `light` (soft with `rise` > 0, `LightList::directional_rise`) over a mask of
    /// `mask` texels anchored at `snapped` with Chiaro's `tolerance` below a top and the highest
    /// finite top `top`.
    pub fn new(
        light: &GpuDirectional,
        rise: f32,
        mask: [u32; 2],
        snapped: [i32; 2],
        tolerance: f32,
        top: f32,
    ) -> Self {
        let [x, y, slope, _] = light.dir_slope;
        let g = SunGrid::new(mask, [x, y], snapped);
        let bits = |v: i32| f32::from_bits(v as u32);
        Self {
            light: *light,
            domain: [mask[0] as f32, mask[1] as f32, tolerance, top],
            axes: [g.a[0], g.a[1], g.b[0], g.b[1]],
            origin: [g.origin[0], g.origin[1], g.size[0] as f32, g.size[1] as f32],
            inverse: g.inverse(),
            // A ground receiver reads a texel whose middle lies up to half a texel farther from
            // the sun, so its height there is down to half a texel's rise below the ground.
            // A soft sun's lowest ray rises slower and meets obstacles farther away; near the
            // horizon the disc's lower half is cut at half the slope.
            build: [
                SUN_RECEIVER_GAP,
                -slope * SUN_ALONG - 0.1,
                rise,
                det::max(slope - rise, 0.5 * slope),
            ],
            lattice: [g.corner[0], g.corner[1], g.offset[0], g.offset[1]].map(bits),
            partial: [0.0; 4],
        }
    }

    /// The block for a mask with partly transparent obstacles (`partial`) and pattern obstacles
    /// (`patterns`, only with `partial`).
    pub fn with_partial(mut self, partial: bool, patterns: bool) -> Self {
        self.partial = [
            f32::from(u8::from(partial)),
            f32::from(u8::from(partial && patterns)),
            0.0,
            0.0,
        ];
        self
    }

    /// The map keeps partly transparent runs: its build draws two more targets.
    pub fn partial(&self) -> bool {
        self.partial[0] > 0.0
    }

    /// Mask texels a ray from a map texel walks at most toward the sun: until it rises above the
    /// highest finite top from the lowest receiver.
    fn reach(&self) -> f64 {
        (f64::from(self.domain[3]) - f64::from(self.build[1])) / f64::from(self.build[3])
    }
}

/// What a sun's map was built from: everything [`SunParams`] holds except the colour and the
/// direction in the world, which only the light reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunSource {
    /// `dir_slope` xyz and the rise (a soft map has a second target).
    aim: [f32; 4],
    /// Mask size.
    size: [u32; 2],
    /// `MaskAnchor::snapped` of the frame.
    snapped: [i32; 2],
    /// Tolerance below a top, highest finite top.
    heights: [f32; 2],
    /// [`SunParams::partial`]: the map has its targets for partly transparent runs. (Whether
    /// the build reads pattern ids may change while the map holds: a texel that named an
    /// obstacle now gone was rebuilt with the mask change that took it out.)
    partial: f32,
}

impl SunSource {
    /// The source of `params` over `mask`.
    pub fn new(params: &SunParams, mask: &RowMask<'_>) -> Self {
        let [x, y, slope, _] = params.light.dir_slope;
        Self {
            aim: [x, y, slope, params.build[2]],
            size: mask.size,
            snapped: mask.snapped,
            heights: [params.domain[2], params.domain[3]],
            partial: params.partial[0],
        }
    }
}

impl Source for SunSource {
    /// A turned sun, a resized mask, other heights, partial runs kept or not, a mask redrawn
    /// everywhere or too many redrawn rectangles build the map anew; a pan or redrawn rectangles
    /// update it.
    fn refresh(&self, now: &Self, change: MaskChange<'_>) -> Refresh<Self> {
        let same = self.aim == now.aim && self.size == now.size && self.heights == now.heights;
        if !same || self.partial != now.partial {
            return Refresh::Build;
        }
        match change {
            MaskChange::All => Refresh::Build,
            MaskChange::Rects(list) if list.overflowed() => Refresh::Build,
            MaskChange::Rects(list) if !list.rects().is_empty() => Refresh::Update(*self),
            _ if self.snapped != now.snapped => Refresh::Update(*self),
            _ => Refresh::Keep,
        }
    }
}

/// Most bands of one map update: one per redrawn rectangle, eight rim strips of a pan, two strips
/// the pan brought into the map.
pub const MAX_SUN_BANDS: usize = DirtyRectList::MAX_DIRTY_RECTS + 10;

/// Map texels of margin around a band, for rounding.
const BAND_MARGIN: f64 = 1.5;

/// The texels a map update builds again ([`sun_bands`]): parallelograms in map texels counted
/// from the map's first lattice texel, each `[o.x, o.y, e1.x, e1.y, e2.x, e2.y, 0, 0]` (corner
/// and the two sides), as `SunBand` of `lightrow_sun_band.vert` reads them. Fixed storage.
#[derive(Clone, Copy, Debug)]
pub struct SunBands {
    quads: [[f32; 8]; MAX_SUN_BANDS],
    count: usize,
    area: f64,
}

impl Default for SunBands {
    fn default() -> Self {
        Self {
            quads: [[0.0; 8]; MAX_SUN_BANDS],
            count: 0,
            area: 0.0,
        }
    }
}

impl SunBands {
    /// The parallelograms.
    pub fn quads(&self) -> &[[f32; 8]] {
        &self.quads[..self.count]
    }

    /// Their area in map texels (overlaps counted twice).
    pub fn texels(&self) -> u64 {
        self.area.ceil() as u64
    }

    /// The parallelogram `o + s e1 + t e2`, s and t in [0, 1]; `false` if there is no room.
    fn push(&mut self, o: [f64; 2], e1: [f64; 2], e2: [f64; 2]) -> bool {
        if self.count == MAX_SUN_BANDS {
            return false;
        }
        let q = [o[0], o[1], e1[0], e1[1], e2[0], e2[1], 0.0, 0.0];
        self.quads[self.count] = q.map(|v| v as f32);
        self.count += 1;
        self.area += (e1[0] * e2[1] - e1[1] * e2[0]).abs();
        true
    }
}

/// The texels of the map of `now` (from `after`) to build again when it was built from `before`
/// and the mask changed by `change` since: those whose ray toward the sun reaches a redrawn
/// rectangle or a rim strip the pan exposed or gave up, and those the pan brought into the map.
/// `false` if that is no less than the whole map (build it all).
///
/// A changed rectangle seen from the map is a parallelogram; the texels whose rays reach it lie
/// in it swept against the sun by the reach. That is bounded by a parallelogram with one side
/// against the sun and the other along the rectangle's longer side: tight for the thin strips a
/// pan exposes, which run slantwise through the map.
pub fn sun_bands(
    now: &SunParams,
    after: &SunSource,
    before: &SunSource,
    change: MaskChange<'_>,
    out: &mut SunBands,
) -> bool {
    out.count = 0;
    out.area = 0.0;
    let [x, y, _, _] = now.light.dir_slope;
    let mask = after.size;
    let g = SunGrid::new(mask, [x, y], after.snapped);
    let old = SunGrid::new(mask, [x, y], before.snapped);
    let size = g.size.map(f64::from);

    // Lattice texels new to the map: the strips the shift of the corner uncovered.
    let shift = [0, 1].map(|k| f64::from(g.corner[k]) - f64::from(old.corner[k]));
    if shift[0].abs() >= size[0] || shift[1].abs() >= size[1] {
        return false;
    }
    let first = |k: usize| {
        if shift[k] > 0.0 {
            size[k] - shift[k]
        } else {
            0.0
        }
    };
    let mut fits = true;
    if shift[0] != 0.0 {
        fits &= out.push([first(0), 0.0], [shift[0].abs(), 0.0], [0.0, size[1]]);
    }
    if shift[1] != 0.0 {
        fits &= out.push([0.0, first(1)], [size[0], 0.0], [0.0, shift[1].abs()]);
    }

    // World rectangles of mask texels that changed, each with whether rays from inside the mask
    // reach it from afar: redrawn ones, and the rim strips a pan exposed (new mask minus old) or
    // gave up (old minus new).
    let mut changed = [([0i64; 4], true); DirtyRectList::MAX_DIRTY_RECTS + 8];
    let mut n = 0;
    let world = |offset: [i32; 2]| {
        let o = offset.map(|v| -i64::from(v));
        [
            o[0],
            o[1],
            o[0] + i64::from(mask[0]),
            o[1] + i64::from(mask[1]),
        ]
    };
    let (new_mask, old_mask) = (world(g.offset), world(old.offset));
    // A rim strip lies at the edge of its mask: only rays toward that edge come from inside; the
    // texels beyond it start outside the mask and stay empty.
    let toward = [x < 0.0, x > 0.0, y < 0.0, y > 0.0];
    for (p, q) in [(new_mask, old_mask), (old_mask, new_mask)] {
        // p minus q, as up to four strips of p: left, right, top, bottom.
        let strips = [
            [p[0], p[1], p[2].min(q[0]), p[3]],
            [p[0].max(q[2]), p[1], p[2], p[3]],
            [p[0], p[1], p[2], p[3].min(q[1])],
            [p[0], p[3].max(q[3]), p[2], p[3]],
        ];
        for (strip, far) in strips.into_iter().zip(toward) {
            if strip[0] < strip[2] && strip[1] < strip[3] {
                changed[n] = (strip, far);
                n += 1;
            }
        }
    }
    // The redrawn rectangles; the mask plan redraws the strips a pan exposed, too, and those
    // are already in.
    let exposed = n;
    if let MaskChange::Rects(list) = change {
        let s = g.offset.map(i64::from);
        for r in list.rects() {
            let r = [
                i64::from(r.x0) - s[0],
                i64::from(r.y0) - s[1],
                i64::from(r.x1) - s[0],
                i64::from(r.y1) - s[1],
            ];
            let inside = |(e, _): &([i64; 4], bool)| {
                e[0] <= r[0] && e[1] <= r[1] && r[2] <= e[2] && r[3] <= e[3]
            };
            if !changed[..exposed].iter().any(inside) {
                changed[n] = (r, true);
                n += 1;
            }
        }
    }

    // Map texel coordinates (u along, v across) counted from the map's first lattice texel:
    // texel i covers [i, i + 1), its middle marches along u.
    let (a, b) = (g.a.map(f64::from), g.b.map(f64::from));
    let (a2, b2) = (dot64(a, a), dot64(b, b));
    let corner = g.corner.map(f64::from);
    let local = |p: [i64; 2]| {
        let p = p.map(|v| v as f64);
        [dot64(p, a) / a2 - corner[0], dot64(p, b) / b2 - corner[1]]
    };
    for &(r, far) in &changed[..n] {
        let reach = if far {
            now.reach()
        } else {
            f64::from(SUN_RECEIVER_GAP) + 1.0
        };
        let c = [[r[0], r[1]], [r[2], r[1]], [r[0], r[3]]].map(local);
        let sides = [
            [c[1][0] - c[0][0], c[1][1] - c[0][1]],
            [c[2][0] - c[0][0], c[2][1] - c[0][1]],
        ];
        // Along the longer side in the world, unless that runs along u.
        let long = usize::from(r[3] - r[1] > r[2] - r[0]);
        let d = if sides[long][1].abs() > 1e-3 * (sides[long][0].abs() + 1.0) {
            sides[long]
        } else {
            sides[1 - long]
        };
        // P = alpha (1, 0) + beta d; the four corners' ranges, swept against the sun.
        let corners = [
            c[0],
            c[1],
            c[2],
            [c[1][0] + sides[1][0], c[1][1] + sides[1][1]],
        ];
        let coords = corners.map(|p| {
            let beta = p[1] / d[1];
            (p[0] - beta * d[0], beta)
        });
        let (mut lo, mut hi) = (coords[0], coords[0]);
        for (alpha, beta) in coords {
            lo = (min64(lo.0, alpha), min64(lo.1, beta));
            hi = (max64(hi.0, alpha), max64(hi.1, beta));
        }
        let pad = BAND_MARGIN / d[1].abs();
        let (alpha, beta) = (lo.0 - reach - BAND_MARGIN, lo.1 - pad);
        let (width, length) = (hi.0 + BAND_MARGIN - alpha, hi.1 + pad - beta);
        fits &= out.push(
            [alpha + beta * d[0], beta * d[1]],
            [width, 0.0],
            [length * d[0], length * d[1]],
        );
    }
    fits && out.area < size[0] * size[1]
}

fn min64(p: f64, q: f64) -> f64 {
    if q < p { q } else { p }
}

fn max64(p: f64, q: f64) -> f64 {
    if q > p { q } else { p }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::SunCache;
    use crate::shadow::DirtyRect;

    fn sun(x: f32, y: f32, slope: f32) -> GpuDirectional {
        let n = (x * x + y * y).sqrt();
        GpuDirectional {
            dir_slope: [x / n, y / n, slope, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
            world: [0.0, 0.0, 1.0, 0.0],
        }
    }

    #[test]
    fn suits_names_what_a_map_cannot_do() {
        let hard = sun(1.0, 2.0, 0.5);
        assert!(sun_suits(&hard, Casters::All));
        assert!(!sun_suits(&hard, Casters::Static), "caster filter");
        let soft = GpuDirectional {
            dir_slope: [hard.dir_slope[0], hard.dir_slope[1], 0.5, 0.01],
            ..hard
        };
        assert!(sun_suits(&soft, Casters::All), "source radius");
        let unshadowed = GpuDirectional {
            color: [1.0, 1.0, 1.0, 0.0],
            ..hard
        };
        assert!(!sun_suits(&unshadowed, Casters::All), "no shadows");
        let overhead = GpuDirectional {
            dir_slope: [0.0; 4],
            ..hard
        };
        assert!(!sun_suits(&overhead, Casters::All), "overhead");
        assert!(!sun_suits(&sun(1.0, 0.0, -0.1), Casters::All), "from below");
    }

    #[test]
    fn the_grid_is_fixed_in_the_world_and_covers_the_mask() {
        let mask = [480, 340];
        let d = sun(3.0, -4.0, 1.0).dir_slope;
        for snapped in [[0, 0], [37, -5], [-1000, 77]] {
            let g = SunGrid::new(mask, [d[0], d[1]], snapped);
            assert_eq!(g.size, [591, 2355]);
            assert_eq!(
                SunGrid::size_of(mask),
                g.size,
                "independent of the direction"
            );
            let inv = g.inverse();
            let to_map = |p: [f32; 2]| {
                let r = [p[0] - g.origin[0], p[1] - g.origin[1]];
                [inv[0] * r[0] + inv[1] * r[1], inv[2] * r[0] + inv[3] * r[1]]
            };
            // Every corner of the mask maps into the grid.
            for p in [[0.0, 0.0], [480.0, 0.0], [0.0, 340.0], [480.0, 340.0]] {
                let m = to_map(p);
                assert!(m[0] >= 0.0 && m[0] <= g.size[0] as f32, "{p:?} {m:?}");
                assert!(m[1] >= 0.0 && m[1] <= g.size[1] as f32, "{p:?} {m:?}");
            }
            // A texel middle maps back to itself; one step toward the sun is one texel along.
            let c = [0, 1].map(|k| g.origin[k] + 10.5 * g.a[k] + 20.5 * g.b[k]);
            let m = to_map(c);
            assert!(
                (m[0] - 10.5).abs() < 1e-2 && (m[1] - 20.5).abs() < 1e-2,
                "{m:?}"
            );
            let m = to_map([c[0] + d[0], c[1] + d[1]]);
            assert!(
                (m[0] - 11.5).abs() < 1e-2 && (m[1] - 20.5).abs() < 1e-2,
                "{m:?}"
            );
        }
        // The lattice stays in the world: a pan moves the corner by whole lattice texels and the
        // origin by the pan.
        let (g, h) = (
            SunGrid::new(mask, [d[0], d[1]], [0, 0]),
            SunGrid::new(mask, [d[0], d[1]], [37, -5]),
        );
        let shift = [h.corner[0] - g.corner[0], h.corner[1] - g.corner[1]];
        for k in 0..2 {
            let moved = g.origin[k] + shift[0] as f32 * g.a[k] + shift[1] as f32 * g.b[k];
            let pan = [37.0, -5.0][k];
            assert!((h.origin[k] - moved - pan).abs() < 1e-3, "{k}");
        }
    }

    #[test]
    fn a_map_is_kept_until_sun_or_mask_change() {
        let mut cache = SunCache::default();
        let none = MaskChange::None;
        let empty = DirtyRectList::new();
        let mut one = DirtyRectList::new();
        one.add(DirtyRect::new(190, 90, 191, 91));
        let mut full = DirtyRectList::new();
        for _ in 0..=DirtyRectList::MAX_DIRTY_RECTS {
            full.add(DirtyRect::new(0, 0, 1, 1));
        }
        let mask = |snapped, change| RowMask {
            size: [200, 100],
            snapped,
            change,
        };
        let source = |light: &GpuDirectional, m: &RowMask<'_>| {
            SunSource::new(&SunParams::new(light, 0.0, m.size, m.snapped, 0.05, 3.0), m)
        };
        let mut frame = |light: &GpuDirectional, m: &RowMask<'_>| {
            cache.begin();
            let (map, refresh) = cache.assign(0, source(light, m), m.change).unwrap();
            cache.commit();
            (map, refresh)
        };
        let a = sun(1.0, 2.0, 0.5);
        let still = mask([0, 0], none);
        assert_eq!(frame(&a, &still), (0, Refresh::Build));
        assert_eq!(frame(&a, &still), (0, Refresh::Keep));
        // Colour and the world direction's specular factor are not in the map.
        let red = GpuDirectional {
            color: [1.0, 0.0, 0.0, 1.0],
            ..a
        };
        assert_eq!(frame(&red, &still), (0, Refresh::Keep));
        // An empty list of redrawn texels changes nothing; one texel or a pan updates; too many
        // rectangles or all of the mask build.
        let before = source(&a, &still);
        assert_eq!(
            frame(&a, &mask([0, 0], MaskChange::Rects(&empty))),
            (0, Refresh::Keep)
        );
        let rects = mask([0, 0], MaskChange::Rects(&one));
        assert_eq!(frame(&a, &rects), (0, Refresh::Update(before)));
        assert_eq!(
            frame(&a, &mask([0, 0], MaskChange::Rects(&full))),
            (0, Refresh::Build)
        );
        assert_eq!(
            frame(&a, &mask([0, 0], MaskChange::All)),
            (0, Refresh::Build)
        );
        assert_eq!(frame(&a, &mask([1, 0], none)), (0, Refresh::Update(before)));
        // Turned, steeper, a larger mask.
        let panned = mask([1, 0], none);
        assert_eq!(frame(&sun(1.0, 2.1, 0.5), &panned), (0, Refresh::Build));
        assert_eq!(frame(&sun(1.0, 2.1, 0.6), &panned), (0, Refresh::Build));
        let wide = RowMask {
            size: [201, 100],
            ..panned
        };
        assert_eq!(frame(&sun(1.0, 2.1, 0.6), &wide), (0, Refresh::Build));
        // A size: the map gets its second target, and its lowest ray reaches farther.
        let light = sun(1.0, 2.1, 0.6);
        let soft = SunParams::new(&light, 0.01, wide.size, wide.snapped, 0.05, 3.0);
        let hard = SunParams::new(&light, 0.0, wide.size, wide.snapped, 0.05, 3.0);
        assert!(soft.reach() > hard.reach());
        cache.begin();
        let refresh = cache.assign(0, SunSource::new(&soft, &wide), none);
        cache.commit();
        assert_eq!(refresh, Some((0, Refresh::Build)));
    }

    #[test]
    fn a_change_rebuilds_the_band_its_rays_reach() {
        let mask = [200, 100];
        let a = sun(1.0, 2.0, 0.5);
        let at = |snapped| {
            let m = RowMask {
                size: mask,
                snapped,
                change: MaskChange::None,
            };
            let params = SunParams::new(&a, 0.0, mask, snapped, 0.05, 3.0);
            (params, SunSource::new(&params, &m))
        };
        let (params, before) = at([0, 0]);
        let total = {
            let s = SunGrid::size_of(mask);
            u64::from(s[0]) * u64::from(s[1])
        };
        let mut out = SunBands::default();
        // One texel: a band as wide as the texel across and as long as the reach.
        let mut one = DirtyRectList::new();
        one.add(DirtyRect::new(100, 50, 101, 51));
        let rects = MaskChange::Rects(&one);
        assert!(sun_bands(&params, &before, &before, rects, &mut out));
        let reach = params.reach();
        assert!((7.0..7.5).contains(&reach), "{reach}");
        let band = out.texels();
        assert!((4 * 8..12 * 16).contains(&band), "{band}");
        // A pan by one texel: the rim strips' bands and a strip of the map, far less than all.
        let (panned, after) = at([1, 0]);
        assert!(sun_bands(
            &panned,
            &after,
            &before,
            MaskChange::None,
            &mut out
        ));
        assert!(out.texels() < total / 8, "{} of {total}", out.texels());
        let pan = out.quads().len();
        // A pan farther than the map: all.
        let (far, beyond) = at([5000, 0]);
        assert!(!sun_bands(
            &far,
            &beyond,
            &before,
            MaskChange::None,
            &mut out
        ));
        // Panned with a redrawn texel: the pan's bands and the texel's.
        assert!(sun_bands(&panned, &after, &before, rects, &mut out));
        assert_eq!(out.quads().len(), pan + 1);
    }
}
