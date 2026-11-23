//! The light scene: persistent lights, shadow maps kept between frames, and the per-sample light
//! pass over the surfaces a renderer (or any host) hands in.
//!
//! Origin: pixelraum `src/renderer/lights.rs`, the light part of `src/renderer/mod.rs::end` and
//! of `src/renderer/shade.rs` @ 81927d8. The arithmetic is unchanged (same operations in the
//! same order), so the renderer's reference images stay bit-identical.

use std::collections::BTreeMap;

use knorke_types::{Column, ColumnMut, Parallel, project};

use crate::math::{Vec3, Vec3Ext};
use crate::model::*;
use crate::shadow::*;
use crate::textures::{CasterTextures, Tex};

/// Shadow settings of the light pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightOptions {
    /// Half-size of the sun shadow box around the eye in world units.
    pub shadow_half: f32,
    /// Sun shadow map size in texels.
    pub shadow_res: usize,
    /// Point light shadow cube face size in texels.
    pub point_shadow_res: usize,
    /// Shadow bias in world units; the slope term is 2x this scaled by 1 - n.l.
    pub shadow_bias: f32,
    /// Soft sun shadow edges: width of the penumbra in world units, 0 = hard edges. Nine
    /// shadow-map taps in a fixed light-space pattern, stepped to 0, 1/3, 2/3, 1 with an ordered
    /// dither between steps. Clamped to 0..64; NaN = 0.
    pub shadow_soft: f32,
    /// The same for shadowed point lights (nine cube lookups per light cost much more).
    pub point_shadow_soft: f32,
}

impl Default for LightOptions {
    fn default() -> Self {
        LightOptions {
            shadow_half: 32.0,
            shadow_res: 512,
            point_shadow_res: 128,
            shadow_bias: 0.02,
            shadow_soft: 0.0,
            point_shadow_soft: 0.0,
        }
    }
}

/// What a sample of [`Surfaces`] is.
pub mod sample {
    /// Nothing drawn (sky, background). Ignored.
    pub const EMPTY: u8 = 0;
    /// Drawn but not lit here (pre-lit particles, additive glow). Counts for the visible bounds
    /// that select the point lights, gets no light value.
    pub const UNLIT: u8 = 1;
    /// A lit surface.
    pub const LIT: u8 = 2;
    /// A lit billboard sprite: point lights treat it as facing them (n.l = 1) and look up their
    /// shadow on its upright axis ([`Surfaces::axis`]), as its shadow caster turns towards them.
    pub const BILLBOARD: u8 = 3;
}

/// The surfaces of one frame, as G-buffer-like columns: one entry per sample. All columns have
/// the same length.
///
/// A renderer writes one sample per screen pixel first (sample `i` = pixel `i`, see
/// [`Surfaces::screen`]), then one per transparent fragment. Only the screen samples keep shadow
/// factors between frames.
#[derive(Clone, Copy, Debug)]
pub struct Surfaces<'a> {
    /// Pixels per screen row (to find the dither cell of a sample's pixel).
    pub width: usize,
    /// Camera position (point lights are picked nearest to it).
    pub eye: Vec3,
    /// Sample kind, see [`sample`].
    pub kind: &'a [u8],
    /// World position of the sample.
    pub position: &'a [Vec3],
    /// Surface normal, facing the eye.
    pub normal: &'a [Vec3],
    /// Bottom centre of the billboard for [`sample::BILLBOARD`] samples, ignored otherwise.
    pub axis: &'a [Vec3],
    /// Screen pixel (`y * width + x`) of the sample.
    pub pixel: &'a [u32],
    /// Samples `0..screen` are one per screen pixel, in pixel order.
    pub screen: usize,
}

/// Counters of the last prepared frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightStats {
    /// Point lights used for the frame (at most [`MAX_LIGHTS`]).
    pub lights_used: u32,
    /// Shadow maps rebuilt this frame (the sun map and each point light's cube count one each).
    pub shadow_maps_rebuilt: u32,
}

/// Cache slot of the sun; point lights use their index in the frame's light list.
const SUN_SLOT: usize = MAX_LIGHTS;
/// Slots per pixel: every point light of a frame plus the sun.
const SLOTS: usize = MAX_LIGHTS + 1;
/// Slot without a kept factor.
const NO_CODE: u8 = 255;
/// Every value a shadow factor can take: hard lookups give 0 or 1, soft ones thirds (`soft_shadow`).
/// `CODE_VALUE[k]` is bit for bit the factor that was stored as code k.
const CODE_VALUE: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
/// Samples per chunk of the parallel light pass (fixed, so the split never depends on threads).
const CHUNK: usize = 1024;
/// Shadow map rows per chunk of the band-parallel sun map build.
const MAP_BAND: usize = 16;

/// Shadow factors of one screen sample kept from the previous frame.
///
/// A shadow factor depends on the surface point and normal, the shadow map contents
/// (`ShadowCache::generation`), the light position or sun direction, the bias, the soft widths
/// and the pixel (dither). `shade` compares the per-frame inputs with the last frame and marks
/// the slots that changed; the sample compares its position and normal bits. Where everything
/// matches, the kept factor is exactly what the lookups would give, so they are skipped.
/// Flickering torches with a standing camera then shade without any lookup.
#[derive(Clone, Copy, Debug)]
struct ShadowMemo {
    /// Position and normal bits when the codes were stored; `valid` false = nothing kept.
    key: [u32; 6],
    valid: bool,
    /// Per slot an index into CODE_VALUE, or NO_CODE.
    codes: [u8; SLOTS],
}

impl Default for ShadowMemo {
    fn default() -> Self {
        ShadowMemo {
            key: [0; 6],
            valid: false,
            codes: [NO_CODE; SLOTS],
        }
    }
}

/// Light value and kept shadow factors of one sample.
#[derive(Clone, Copy, Debug, Default)]
struct Px {
    light: [f32; 3],
    memo: ShadowMemo,
}

/// Inputs of the last frame that used the shadow cache (bit patterns, compared exactly).
#[derive(Debug, Default)]
struct MemoKeys {
    frame: [u32; 5],
    slots: [[u32; 6]; SLOTS],
    valid: bool,
}

/// A point light ready for shading this frame.
struct ActiveLight<'a> {
    light: PointLight,
    /// Colour times flickered strength.
    color: [f32; 3],
    /// Cube shadow maps in `cube_face` order; None = no shadows.
    maps: Option<&'a [ShadowMap; 6]>,
}

/// Everything the per-sample pass reads.
struct Shade<'a> {
    bias: f32,
    ambient: [f32; 3],
    /// Normalized sun direction and colour times strength; None without sun or with strength <= 0.
    sun: Option<(Vec3, [f32; 3])>,
    sun_map: Option<&'a ShadowMap>,
    soft: f32,
    point_soft: f32,
    points: [Option<ActiveLight<'a>>; MAX_LIGHTS],
    count: usize,
    width: usize,
}

/// Sun, ambient light, persistent point lights and their shadow maps.
///
/// One frame: [`prepare`](Self::prepare) (picks the lights that reach the visible surfaces,
/// brings shadow maps up to date) → [`shade`](Self::shade) (one light value per sample) →
/// read [`light`](Self::light). Light values are linear factors per channel (0..1, may exceed
/// 1); the host multiplies its surface colours with them. Shadow maps and the shadow cache are
/// kept between frames and only rebuilt when their inputs change.
#[derive(Debug)]
pub struct LightScene {
    /// Shadow settings.
    pub options: LightOptions,
    sun: Option<Sun>,
    ambient: (u32, f32),
    /// Persistent point lights by id; iteration in id order keeps shading deterministic.
    lights: BTreeMap<u32, LightSlot>,
    next_id: u32,
    time: f64,
    sky_tint: (u32, f32),
    sun_shadow: Option<ShadowCache<ShadowMap>>,
    map_gen: u64,
    /// The frame's lights (id, colour times flickered strength), id order.
    picked: Vec<(u32, [f32; 3])>,
    /// The sun map is read this frame.
    sun_on: bool,
    /// Casters or billboards existed this frame (point light maps are read only then).
    casters_any: bool,
    stats: LightStats,
    px: Vec<Px>,
    memo: MemoKeys,
    memo_enabled: bool,
    /// Scratch of `update_shadow_maps`, kept to avoid allocations in steady frames.
    scratch: Scratch,
}

#[derive(Debug, Default)]
struct Scratch {
    caster_bounds: Vec<(Vec3, f32)>,
    billboard_bounds: Vec<(Vec3, f32)>,
    near_c: Vec<Vec<u32>>,
    near_b: Vec<Vec<u32>>,
    prepared: Vec<PreparedTri>,
}

impl Default for LightScene {
    fn default() -> Self {
        LightScene::new()
    }
}

impl LightScene {
    /// No sun, no ambient light, no point lights.
    pub fn new() -> LightScene {
        LightScene {
            options: LightOptions::default(),
            sun: None,
            ambient: (0xffffff, 0.0),
            lights: BTreeMap::new(),
            next_id: 0,
            time: 0.0,
            sky_tint: (0xffffff, 1.0),
            sun_shadow: None,
            map_gen: 0,
            picked: Vec::new(),
            sun_on: false,
            casters_any: false,
            stats: LightStats::default(),
            px: Vec::new(),
            memo: MemoKeys::default(),
            memo_enabled: true,
            scratch: Scratch::default(),
        }
    }

    /// Sets or removes the sun.
    pub fn set_sun(&mut self, s: Option<Sun>) {
        self.sun = s;
    }

    /// The sun, if any.
    pub fn sun(&self) -> Option<Sun> {
        self.sun
    }

    /// Ambient light colour (`0xRRGGBB`) and strength.
    pub fn set_ambient(&mut self, color: u32, strength: f32) {
        self.ambient = (color, strength);
    }

    /// Time in seconds for the flicker of point lights.
    pub fn set_time(&mut self, t: f64) {
        self.time = t;
    }

    /// Sky colour (`0xRRGGBB`) and strength: multiplies what is seen of the sky.
    pub fn set_sky_tint(&mut self, color: u32, strength: f32) {
        self.sky_tint = (color, strength);
    }

    /// The sky tint as a linear factor per channel.
    pub fn sky_tint(&self) -> [f32; 3] {
        color_scaled(self.sky_tint.0, self.sky_tint.1)
    }

    /// Creates a persistent point light and returns its id. Ids are never reused; the light stays
    /// until [`destroy_light`](Self::destroy_light). Any number may exist; each frame uses at most
    /// [`MAX_LIGHTS`] of them (those that reach the visible surfaces, nearest to the eye first).
    pub fn create_light(&mut self, l: PointLight) -> u32 {
        let id = self.next_id;
        self.next_id = id.wrapping_add(1);
        self.lights.insert(
            id,
            LightSlot {
                light: l,
                shadow: None,
            },
        );
        id
    }

    /// Changes a light; false for an unknown or destroyed id.
    pub fn set_light(&mut self, id: u32, l: PointLight) -> bool {
        match self.lights.get_mut(&id) {
            Some(slot) => {
                slot.light = l;
                true
            }
            None => false,
        }
    }

    /// Removes a light; false for an unknown or destroyed id.
    pub fn destroy_light(&mut self, id: u32) -> bool {
        self.lights.remove(&id).is_some()
    }

    /// Light `id`, if it exists.
    pub fn light_of(&self, id: u32) -> Option<&PointLight> {
        self.lights.get(&id).map(|s| &s.light)
    }

    /// Counters of the last prepared frame.
    pub fn stats(&self) -> LightStats {
        self.stats
    }

    /// Switches the shadow cache kept between frames on or off (on by default). Off only for
    /// tests that compare against frames shaded without it; the image is the same either way.
    #[doc(hidden)]
    pub fn set_shadow_cache(&mut self, on: bool) {
        self.memo_enabled = on;
    }

    /// Number of screen samples that keep at least one shadow factor (tests of the cache).
    #[doc(hidden)]
    pub fn shadow_cache_hits(&self) -> usize {
        self.px
            .iter()
            .filter(|p| p.memo.valid && p.memo.codes.iter().any(|&c| c != NO_CODE))
            .count()
    }

    /// Starts a frame: picks the point lights that reach the drawn samples of `s` and brings the
    /// shadow maps up to date for `casters` and `billboards` (textures for their alpha test).
    pub fn prepare<T: CasterTextures + ?Sized>(
        &mut self,
        s: &Surfaces<'_>,
        casters: &[Caster],
        billboards: &[BillboardCaster],
        textures: &T,
        par: &impl Parallel,
    ) {
        let sun_view = self.sun_view(s.eye, casters, billboards);
        self.picked.clear();
        if !self.lights.is_empty() {
            // box around the world positions of all drawn samples
            let mut lo = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
            let mut hi = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
            for (k, p) in s.kind.iter().zip(s.position) {
                if *k != sample::EMPTY {
                    lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                    hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
                }
            }
            if lo.x <= hi.x {
                self.pick_lights(lo, hi, s.eye);
            }
        }
        let rebuilt = self.update_shadow_maps(sun_view, casters, billboards, textures, par);
        self.sun_on = sun_view.is_some();
        self.casters_any = !(casters.is_empty() && billboards.is_empty());
        let used = self
            .picked
            .iter()
            .filter(|(id, _)| self.lights.contains_key(id))
            .count();
        self.stats = LightStats {
            lights_used: used as u32,
            shadow_maps_rebuilt: rebuilt,
        };
    }

    /// One light value per sample of `s` (zero for samples that are not lit), readable with
    /// [`light`](Self::light) until the next call. Call after [`prepare`](Self::prepare) with
    /// the same surfaces.
    pub fn shade(&mut self, s: &Surfaces<'_>, par: &impl Parallel) {
        let n = s.kind.len();
        assert!(
            s.position.len() == n
                && s.normal.len() == n
                && s.axis.len() == n
                && s.pixel.len() == n
                && s.screen <= n,
            "Surfaces: columns of different lengths"
        );
        let o = self.options;
        let mut points: [Option<ActiveLight>; MAX_LIGHTS] = Default::default();
        let mut count = 0;
        for &(id, color) in &self.picked {
            let Some(slot) = self.lights.get(&id) else {
                continue;
            };
            let maps = if slot.light.shadows && self.casters_any {
                slot.shadow.as_ref().map(|c| &c.maps)
            } else {
                None
            };
            points[count] = Some(ActiveLight {
                light: slot.light,
                color,
                maps,
            });
            count += 1;
        }
        let ctx = Shade {
            bias: o.shadow_bias,
            ambient: color_scaled(self.ambient.0, self.ambient.1),
            sun: self
                .sun
                .filter(|s| s.strength > 0.0)
                .map(|s| (s.dir.normalized(), color_scaled(s.color, s.strength))),
            sun_map: if self.sun_on {
                self.sun_shadow.as_ref().map(|c| &c.maps)
            } else {
                None
            },
            soft: soft_width(o.shadow_soft),
            point_soft: soft_width(o.point_shadow_soft),
            points,
            count,
            width: s.width.max(1),
        };
        // shadow cache: only when some shadow map is read
        let memo_on = self.memo_enabled
            && (ctx.sun_map.is_some()
                || ctx.points[..count]
                    .iter()
                    .flatten()
                    .any(|p| p.maps.is_some()));
        let mut reset = 0u32;
        if memo_on {
            let b = |f: f32| f.to_bits();
            let frame = [
                s.width as u32,
                s.screen as u32,
                b(o.shadow_bias),
                b(ctx.soft),
                b(ctx.point_soft),
            ];
            let mut slots = [[0u32; 6]; SLOTS];
            let mut j = 0;
            for &(id, _) in &self.picked {
                let Some(slot) = self.lights.get(&id) else {
                    continue;
                };
                if let (true, true, Some(c)) =
                    (slot.light.shadows, self.casters_any, slot.shadow.as_ref())
                {
                    let l = slot.light.pos;
                    slots[j] = [
                        id,
                        c.generation as u32,
                        (c.generation >> 32) as u32,
                        b(l.x),
                        b(l.y),
                        b(l.z),
                    ];
                }
                j += 1;
            }
            if let (Some((dir, _)), Some(c)) = (
                ctx.sun,
                self.sun_shadow.as_ref().filter(|_| ctx.sun_map.is_some()),
            ) {
                slots[SUN_SLOT] = [
                    c.generation as u32,
                    (c.generation >> 32) as u32,
                    b(dir.x),
                    b(dir.y),
                    b(dir.z),
                    1,
                ];
            }
            let all = (1u32 << SLOTS) - 1;
            if !self.memo.valid || self.memo.frame != frame {
                reset = all;
            } else {
                for (j, (a, b)) in self.memo.slots.iter().zip(&slots).enumerate() {
                    if a != b {
                        reset |= 1 << j;
                    }
                }
            }
            self.memo = MemoKeys {
                frame,
                slots,
                valid: true,
            };
        } else {
            self.memo.valid = false;
        }
        // Screen samples keep their place when the fragment count changes; their key decides.
        self.px.resize(n, Px::default());
        let ctx = &ctx;
        par.for_each_chunk_mut(&mut self.px, CHUNK, |ci, chunk| {
            for (k, p) in chunk.iter_mut().enumerate() {
                let i = ci * CHUNK + k;
                let kind = s.kind[i];
                if kind != sample::LIT && kind != sample::BILLBOARD {
                    p.light = [0.0; 3];
                    p.memo.valid = false;
                    continue;
                }
                let (pos, nrm) = (s.position[i], s.normal[i]);
                let px = s.pixel[i] as usize;
                let (x, y) = (px % ctx.width, px / ctx.width);
                if kind == sample::BILLBOARD {
                    // billboards keep nothing (their factors depend on more)
                    p.memo.valid = false;
                    p.light = light_at_impl::<true>(ctx, x, y, pos, nrm, s.axis[i], None);
                    continue;
                }
                let codes = if memo_on && i < s.screen {
                    let key = [
                        pos.x.to_bits(),
                        pos.y.to_bits(),
                        pos.z.to_bits(),
                        nrm.x.to_bits(),
                        nrm.y.to_bits(),
                        nrm.z.to_bits(),
                    ];
                    let e = &mut p.memo;
                    if !e.valid || e.key != key {
                        *e = ShadowMemo {
                            key,
                            valid: true,
                            codes: [NO_CODE; SLOTS],
                        };
                    } else if reset != 0 {
                        for (j, c) in e.codes.iter_mut().enumerate() {
                            if reset >> j & 1 != 0 {
                                *c = NO_CODE;
                            }
                        }
                    }
                    Some(&mut e.codes)
                } else {
                    p.memo.valid = false;
                    None
                };
                p.light = light_at_impl::<false>(ctx, x, y, pos, nrm, Vec3::default(), codes);
            }
        });
    }

    /// The light values of the last [`shade`](Self::shade), one per sample.
    pub fn light(&self) -> impl Column<[f32; 3]> + '_ {
        project(&self.px[..], |p: &Px| &p.light)
    }

    /// Light at points that are lit as a whole (particles): ambient, the sun as for an
    /// upward-facing surface (no shadow) and the frame's point lights with their falloff, as if
    /// facing each light (no shadow). Writes one value per position into `out` (as many as fit).
    pub fn light_points(&self, positions: impl Column<Vec3>, mut out: impl ColumnMut<[f32; 3]>) {
        let ambient = color_scaled(self.ambient.0, self.ambient.1);
        let sun = self.sun.filter(|s| s.strength > 0.0).map(|sun| {
            let lam = (-sun.dir.normalized().y).max(0.0);
            color_scaled(sun.color, sun.strength).map(|c| c * lam)
        });
        for i in 0..positions.len().min(out.len()) {
            let (Some(&p), Some(o)) = (positions.get(i), out.get_mut(i)) else {
                break;
            };
            let mut l = ambient;
            if let Some(c) = sun {
                for (l, c) in l.iter_mut().zip(c) {
                    *l += c;
                }
            }
            for &(id, c) in &self.picked {
                let Some(slot) = self.lights.get(&id) else {
                    continue;
                };
                let fall = falloff((slot.light.pos - p).length(), slot.light.radius);
                if fall > 0.0 {
                    for (l, c) in l.iter_mut().zip(c) {
                        *l += c * fall;
                    }
                }
            }
            *o = l;
        }
    }

    /// View of the orthographic sun shadow map: centred at the eye, snapped to whole texels in light
    /// space, depth range spanning all casters. None without a shadowing sun or without casters.
    fn sun_view(
        &self,
        eye: Vec3,
        casters: &[Caster],
        billboards: &[BillboardCaster],
    ) -> Option<(usize, LightView)> {
        let sun = self.sun?;
        if !sun.shadows || !(sun.strength > 0.0) || casters.is_empty() && billboards.is_empty() {
            return None;
        }
        let fwd = sun.dir.normalized();
        if fwd.length() == 0.0 {
            return None;
        }
        let (right, up) = basis(fwd);
        let half = self.options.shadow_half;
        let res = self.options.shadow_res.max(1);
        let texel = 2.0 * half / res as f32;
        let snap = |v: f32| (v / texel).round() * texel;
        let origin = right * snap(eye.dot(right)) + up * snap(eye.dot(up));
        // vertical range of everything that can block light, plus a margin
        let (mut y0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY);
        for c in casters {
            for p in &c.tri {
                y0 = y0.min(p.y);
                y1 = y1.max(p.y);
            }
        }
        for b in billboards {
            y0 = y0.min(b.base.y);
            y1 = y1.max(b.base.y + b.height);
        }
        let margin = 1.0;
        let (y0, y1) = (y0 - margin, y1 + margin);
        // depth range: corners of the box around the eye that holds all casters
        let r = half * std::f32::consts::SQRT_2 + margin;
        let (mut near, mut far) = (f32::INFINITY, f32::NEG_INFINITY);
        for cx in [-r, r] {
            for cy in [y0, y1] {
                for cz in [-r, r] {
                    let d = (Vec3::new(eye.x + cx, cy, eye.z + cz) - origin).dot(fwd);
                    near = near.min(d);
                    far = far.max(d);
                }
            }
        }
        Some((
            res,
            LightView::Ortho {
                origin,
                right,
                up,
                fwd,
                half,
                near,
                far,
            },
        ))
    }

    /// Point lights that reach the visible box `(lo, hi)`, at most `MAX_LIGHTS` nearest to `eye`
    /// (ties by id), in id order, with flicker applied: (id, colour times strength).
    fn pick_lights(&mut self, lo: Vec3, hi: Vec3, eye: Vec3) {
        let gap = |v: f32, a: f32, b: f32| {
            if v < a {
                a - v
            } else if v > b {
                v - b
            } else {
                0.0
            }
        };
        let mut picked: [(f32, u32); MAX_LIGHTS + 1] = [(0.0, 0); MAX_LIGHTS + 1];
        let mut len = 0;
        // Keep the MAX_LIGHTS nearest (ties by smaller id) without a heap allocation: insertion
        // into a sorted array, dropping the farthest when full. Same set as sorting all of them.
        let key = |a: &(f32, u32), b: &(f32, u32)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
        for (&id, s) in &self.lights {
            let l = &s.light;
            // sphere against box: squared distance from the centre to the box
            let (dx, dy, dz) = (
                gap(l.pos.x, lo.x, hi.x),
                gap(l.pos.y, lo.y, hi.y),
                gap(l.pos.z, lo.z, hi.z),
            );
            if !(l.radius > 0.0
                && l.strength > 0.0
                && dx * dx + dy * dy + dz * dz < l.radius * l.radius)
            {
                continue;
            }
            let v = l.pos - eye;
            let d = v.dot(v);
            let e = (if d.is_nan() { f32::INFINITY } else { d }, id);
            let mut at = len;
            while at > 0 && key(&e, &picked[at - 1]).is_lt() {
                at -= 1;
            }
            if at >= MAX_LIGHTS {
                continue;
            }
            picked.copy_within(at..len.min(MAX_LIGHTS), at + 1);
            picked[at] = e;
            len = (len + 1).min(MAX_LIGHTS);
        }
        let picked = &mut picked[..len];
        picked.sort_by_key(|p| p.1);
        for &(_, id) in picked.iter() {
            let l = &self.lights[&id].light;
            let strength = flickered_strength(l, self.time);
            if strength > 0.0 {
                self.picked.push((id, color_scaled(l.color, strength)));
            }
        }
    }

    /// Brings the sun map and the cube maps of the picked shadowed lights up to date. A map is
    /// rebuilt only when its key changes: view parameters, texture generation and the casters that
    /// can reach it. Each map (or band of the sun map) is built by one chunk alone, in caster
    /// order, so the result does not depend on the executor. Returns the number of maps rebuilt
    /// (a cube counts one).
    fn update_shadow_maps<T: CasterTextures + ?Sized>(
        &mut self,
        sun_view: Option<(usize, LightView)>,
        casters: &[Caster],
        billboards: &[BillboardCaster],
        textures: &T,
        par: &impl Parallel,
    ) -> u32 {
        if casters.is_empty() && billboards.is_empty() {
            return 0;
        }
        let has_point_shadows = self
            .picked
            .iter()
            .any(|&(id, _)| self.lights.get(&id).is_some_and(|s| s.light.shadows));
        let is_opaque = |t: usize| t == SOLID_CASTER || textures.opaque(t);
        let tex_gen = textures.generation();
        let mut sun_dir = Vec3::default();
        let sun_job = sun_view.and_then(|(res, view)| {
            let mut h = KeyHash::new();
            h.u64(res as u64);
            h.u64(tex_gen);
            if let LightView::Ortho {
                origin,
                right,
                up,
                fwd,
                half,
                near,
                far,
            } = view
            {
                sun_dir = fwd;
                for v in [origin, right, up, fwd] {
                    h.vec3(v);
                }
                for v in [half, near, far] {
                    h.f32(v);
                }
            }
            casters
                .iter()
                .for_each(|c| h.caster(c, is_opaque(c.texture)));
            billboards
                .iter()
                .for_each(|b| h.billboard(b, is_opaque(b.texture)));
            let key = h.finish();
            match &self.sun_shadow {
                Some(c) if c.key == key => None,
                _ => Some((key, res, view)),
            }
        });
        // point light keys and the casters that can reach each light
        let res = self.options.point_shadow_res.max(1);
        let bias = self.options.shadow_bias;
        let sc = &mut self.scratch;
        sc.caster_bounds.clear();
        sc.billboard_bounds.clear();
        // (light id, key, index into near_c / near_b)
        let mut misses: [(u32, u64); MAX_LIGHTS] = [(0, 0); MAX_LIGHTS];
        let mut miss_count = 0;
        if has_point_shadows {
            sc.caster_bounds.extend(casters.iter().map(caster_bounds));
            sc.billboard_bounds
                .extend(billboards.iter().map(billboard_bounds));
            for &(id, _) in &self.picked {
                let Some(slot) = self.lights.get(&id) else {
                    continue;
                };
                let l = &slot.light;
                if !l.shadows {
                    continue;
                }
                // A lookup point lies within radius * (1 + 3 / res) of the light (normal offset in
                // light_at), its depth is at most that distance, and the effective bias is at most
                // 3 * |shadow_bias|. A blocker's depth on a cube face is at least its distance / sqrt(3).
                // Casters farther than sqrt(3) * (that bound) can therefore never shadow anything.
                let reach = l.radius * (1.0 + 3.0 / res as f32) + 3.0 * bias.abs();
                let skip = Some(1.7321 * reach * 1.001 + 1e-3);
                let pos = Some(l.pos);
                if sc.near_c.len() <= miss_count {
                    sc.near_c.resize_with(miss_count + 1, Vec::new);
                    sc.near_b.resize_with(miss_count + 1, Vec::new);
                }
                let (cb, bb) = (&sc.caster_bounds, &sc.billboard_bounds);
                let near_c = &mut sc.near_c[miss_count];
                near_c.clear();
                near_c.extend(
                    (0..casters.len())
                        .filter(|&i| !casters[i].sun_only && !far_away(pos, skip, cb[i].0, cb[i].1))
                        .map(|i| i as u32),
                );
                let near_b = &mut sc.near_b[miss_count];
                near_b.clear();
                near_b.extend(
                    (0..billboards.len())
                        .filter(|&i| !far_away(pos, skip, bb[i].0, bb[i].1))
                        .map(|i| i as u32),
                );
                let mut h = KeyHash::new();
                h.u64(res as u64);
                h.u64(tex_gen);
                h.vec3(l.pos);
                h.f32(1.7321 * reach * 1.001 + 1e-3); // = skip
                near_c.iter().for_each(|&i| {
                    h.caster(&casters[i as usize], is_opaque(casters[i as usize].texture))
                });
                near_b.iter().for_each(|&i| {
                    h.billboard(
                        &billboards[i as usize],
                        is_opaque(billboards[i as usize].texture),
                    )
                });
                let key = h.finish();
                if slot.shadow.as_ref().map(|c| c.key) != Some(key) {
                    misses[miss_count] = (id, key);
                    miss_count += 1;
                }
            }
        }
        let rebuilt = sun_job.is_some() as u32 + miss_count as u32;
        if rebuilt == 0 {
            return 0;
        }
        if let Some((key, sres, view)) = sun_job {
            // The sun map in fixed bands of rows: each band sees the exact same triangles in the
            // same order, so the map is bit-identical for any executor.
            self.map_gen += 1;
            let generation = self.map_gen;
            let c = self.sun_shadow.get_or_insert_with(|| ShadowCache {
                key,
                generation,
                maps: ShadowMap::new(sres, view),
            });
            c.key = key;
            c.generation = generation;
            c.maps.prepare_size(sres, view);
            let prepared = &mut self.scratch.prepared;
            prepared.clear();
            for ca in casters {
                if let Some(tex) = caster_texture(ca, textures) {
                    prepare_caster(
                        &view,
                        sres,
                        ca,
                        tex,
                        tex.is_none() || textures.opaque(ca.texture),
                        prepared,
                    );
                }
            }
            for b in billboards {
                if let Some(tex) = Tex::get(textures, b.texture) {
                    prepare_billboard(
                        &view,
                        sres,
                        b,
                        None,
                        sun_dir,
                        Some(tex),
                        textures.opaque(b.texture),
                        prepared,
                    );
                }
            }
            let (near, far) = view_depth_range(&view);
            let (ox, oy) = pattern_origin(&view, sres);
            let prepared = &*prepared;
            par.for_each_chunk_mut(&mut c.maps.depth, MAP_BAND * sres, |k, slice| {
                let y0 = k * MAP_BAND;
                let y1 = y0 + slice.len() / sres;
                slice.fill(f32::INFINITY);
                let (lo, hi) = (y0 as i64, y1 as i64 - 1);
                for tri in prepared {
                    if tri.y1 >= lo && tri.y0 <= hi {
                        fill_band(slice, sres, y0, y1, tri, textures, near, far, ox, oy);
                    }
                }
            });
        }
        if miss_count == 0 {
            return rebuilt;
        }
        // point light cube faces, one face per job; maps keep their buffers between rebuilds
        let sc = &self.scratch;
        let mut jobs: Vec<(&mut ShadowMap, usize, Vec3)> = Vec::with_capacity(miss_count * 6);
        let mut mi = misses[..miss_count].iter().enumerate().peekable();
        for (id, slot) in self.lights.iter_mut() {
            let Some((m, &(_, key))) = mi.next_if(|(_, m)| m.0 == *id) else {
                continue;
            };
            let pos = slot.light.pos;
            let views = cube_views(pos);
            self.map_gen += 1;
            let generation = self.map_gen;
            let c = slot.shadow.get_or_insert_with(|| ShadowCache {
                key,
                generation,
                maps: views.map(|v| ShadowMap::new(res, v)),
            });
            c.key = key;
            c.generation = generation;
            for (map, v) in c.maps.iter_mut().zip(views) {
                map.reset(res, v);
                jobs.push((map, m, pos));
            }
        }
        let (cb, bb) = (&sc.caster_bounds, &sc.billboard_bounds);
        par.for_each_chunk_mut(&mut jobs, 1, |_, chunk| {
            for (map, m, pos) in chunk.iter_mut() {
                for &i in &sc.near_c[*m] {
                    let (centre, rad) = cb[i as usize];
                    let c = &casters[i as usize];
                    if let Some(tex) = caster_texture(c, textures)
                        && face_may_see(&map.view, centre, rad)
                    {
                        draw_caster(map, c, tex, tex.is_none() || textures.opaque(c.texture));
                    }
                }
                for &i in &sc.near_b[*m] {
                    let (centre, rad) = bb[i as usize];
                    let b = &billboards[i as usize];
                    if let Some(tex) = Tex::get(textures, b.texture)
                        && face_may_see(&map.view, centre, rad)
                    {
                        draw_billboard(
                            map,
                            b,
                            Some(*pos),
                            sun_dir,
                            Some(tex),
                            textures.opaque(b.texture),
                        );
                    }
                }
            }
        });
        rebuilt
    }
}

/// Factor kept in `codes[j]`, or `f()` (then kept). Without codes just `f()`.
#[inline(always)]
fn memo(codes: &mut Option<&mut [u8; SLOTS]>, j: usize, f: impl FnOnce() -> f32) -> f32 {
    match codes {
        Some(c) => {
            let k = c[j];
            if k != NO_CODE {
                return CODE_VALUE[k as usize];
            }
            let v = f();
            c[j] = CODE_VALUE
                .iter()
                .position(|c| c.to_bits() == v.to_bits())
                .map_or(NO_CODE, |k| k as u8);
            v
        }
        None => f(),
    }
}

/// Ordered dither threshold (0..1) of screen pixel (x, y), the matrix of the colour dithering.
#[inline]
fn dither_at(x: usize, y: usize) -> f32 {
    (BAYER4[y & 3][x & 3] as f32 + 0.5) / 16.0
}

/// Point light shadow for a billboard pixel at height `pos.y`: looked up on the sprite's axis,
/// where its own light-facing caster stands, pushed about one texel towards the light. A lookup
/// at the pixel itself would find the sprite's own caster in front of half the sprite.
#[inline(never)]
fn billboard_point_shadow(s: &Shade, m: &[ShadowMap; 6], pos: Vec3, axis: Vec3, lp: Vec3) -> f32 {
    let q = Vec3::new(axis.x, pos.y, axis.z);
    let to = lp - q;
    let h = Vec3::new(to.x, 0.0, to.z);
    let hn = if h.length() > 1e-6 {
        h.normalized()
    } else {
        Vec3::default()
    };
    let texel = 2.0 * to.length() / m[0].res as f32;
    let p = q + hn * (1.5 * texel);
    let (d, bias) = (p - lp, slope_bias(s.bias, 1.0));
    if d.x.is_finite() && d.y.is_finite() && d.z.is_finite() {
        lookup_cube(m, p, lp, bias)
    } else {
        lookup(&m[cube_face(d)], p, bias)
    }
}

/// Light (0..1 per channel, may exceed 1) reaching a surface point with normal `n` (facing the
/// eye) seen through pixel (x, y). `BB`: a billboard standing on `axis` (point lights treat it as
/// facing them). `codes`: the pixel's kept shadow factors, None for fragments.
#[inline(always)]
fn light_at_impl<const BB: bool>(
    s: &Shade,
    x: usize,
    y: usize,
    pos: Vec3,
    n: Vec3,
    axis: Vec3,
    mut codes: Option<&mut [u8; SLOTS]>,
) -> [f32; 3] {
    let mut l = s.ambient;
    if let Some((dir, sc)) = s.sun {
        let ndl = (-n.dot(dir)).max(0.0);
        if ndl > 0.0 {
            let shadow = match s.sun_map {
                Some(m) if s.soft > 0.0 => memo(&mut codes, SUN_SLOT, || {
                    soft_sun(s, m, x, y, pos, n, dir, ndl)
                }),
                Some(m) => memo(&mut codes, SUN_SLOT, || {
                    lookup(m, pos, slope_bias(s.bias, ndl))
                }),
                None => 1.0,
            };
            for k in 0..3 {
                l[k] += sc[k] * ndl * shadow;
            }
        }
    }
    for (j, a) in s.points[..s.count].iter().flatten().enumerate() {
        let to = a.light.pos - pos;
        // cheap reject before the square root: falloff is 0 at and beyond the radius
        let r = a.light.radius;
        if to.dot(to) > r * r * 1.001 {
            continue;
        }
        let d = to.length();
        let fall = falloff(d, r);
        if fall <= 0.0 || d <= 0.0 {
            continue;
        }
        let ndl = if BB { 1.0 } else { n.dot(to) / d };
        if ndl <= 0.0 {
            continue;
        }
        let shadow = match a.maps {
            Some(m) if BB => billboard_point_shadow(s, m, pos, axis, a.light.pos),
            Some(m) if s.point_soft > 0.0 => memo(&mut codes, j, || {
                soft_point(s, m, x, y, pos, n, a.light.pos, d, ndl)
            }),
            Some(m) => memo(&mut codes, j, || {
                // Cube faces see nearby walls almost edge-on, where one texel spans a lot of depth.
                // Push the point off its surface by about one texel at its distance (normal offset).
                let texel = 2.0 * d / m[0].res as f32;
                // (p - light is finite here: d < radius, so pos and the light are finite)
                let p = pos + n * (1.5 * texel);
                lookup_cube(m, p, a.light.pos, slope_bias(s.bias, ndl))
            }),
            None => 1.0,
        };
        #[allow(clippy::needless_range_loop)] // same order as pixelraum (bit-exact sums)
        for k in 0..3 {
            l[k] += a.color[k] * ndl * fall * shadow;
        }
    }
    l
}

/// Soft sun shadow (see `shadow::soft_shadow`); out of line so the hard path stays small.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn soft_sun(
    s: &Shade,
    m: &ShadowMap,
    x: usize,
    y: usize,
    pos: Vec3,
    n: Vec3,
    dir: Vec3,
    ndl: f32,
) -> f32 {
    let bias = slope_bias(s.bias, ndl);
    soft_shadow(pos, n, dir, s.soft, dither_at(x, y), |p, lat| {
        lookup_tap(m, p, pos, lat, bias)
    })
}

/// Soft point light shadow: the same pattern across the direction from the light, with the
/// normal offset of the hard lookup applied to every tap.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn soft_point(
    s: &Shade,
    m: &[ShadowMap; 6],
    x: usize,
    y: usize,
    pos: Vec3,
    n: Vec3,
    lp: Vec3,
    d: f32,
    ndl: f32,
) -> f32 {
    let bias = slope_bias(s.bias, ndl);
    let off = n * (1.5 * 2.0 * d / m[0].res as f32);
    soft_shadow(pos, n, pos - lp, s.point_soft, dither_at(x, y), |p, lat| {
        let q = p + off;
        lookup_cube_tap(m, q, pos + off, lp, lat, bias)
    })
}
