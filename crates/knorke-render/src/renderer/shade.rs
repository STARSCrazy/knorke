//! Per-pixel pass at the end of a frame: light (handed in by a light pass), fog, blending,
//! palette and dithering.
//!
//! Origin: pixelraum `src/renderer/shade.rs` @ 81927d8 without the light computation (now
//! `knorke-lighting`). Colours are decoded from sRGB to linear (0..255 scale) by table, light,
//! fog and blending run in linear space, and each pixel is encoded to sRGB once before the
//! palette and the output (ADR 0013 §6, `crate::srgb`).

use super::*;

/// Everything the per-pixel pass of `end_lit` reads.
pub(super) struct Shade<'a, L> {
    pub(super) view: View,
    pub(super) o: Options,
    /// Light values are present for every sample (`Options::lighting` and a light pass ran).
    pub(super) lit: bool,
    pub(super) width: usize,
    pub(super) g: &'a GBuffer,
    pub(super) textures: &'a [Texture],
    /// Background layers of this frame, back to front (legacy sky first); empty = background colour.
    pub(super) backdrops: &'a [crate::backdrop::Layer],
    /// Sky tint and fog applied to backdrops this frame.
    pub(super) backdrop_look: crate::backdrop::Look,
    pub(super) palette: Option<&'a Palette>,
    /// One light value per sample: screen pixel `i`, then fragment `k` at `width * height + k`.
    pub(super) light: &'a L,
    /// Factor for the background colour with lighting.
    pub(super) sky_tint: [f32; 3],
    /// Fog colour and the normalized legacy sun direction (Options::sun_dir).
    pub(super) fog_c: [f32; 3],
    pub(super) legacy_sun: Vec3,
    /// Linear light tint of each particle (by owner index) for pre-lit pixels and fragments.
    pub(super) tints: &'a [[f32; 3]],
    pub(super) srgb: &'static crate::srgb::Tables,
}

impl<L> Shade<'_, L> {
    /// Linear colour (0..255 scale) of a stored sRGB colour; pre-lit (particle) colours get
    /// the light tint of their particle.
    #[inline(always)]
    fn base(&self, c: [u8; 3], prelit: bool, owner: u32) -> [f32; 3] {
        let mut v = self.srgb.lin(c);
        if prelit && let Some(t) = self.tints.get(owner as usize) {
            for k in 0..3 {
                v[k] *= t[k];
            }
        }
        v
    }
}

/// Per-frame values of `surface`, hoisted out of the pixel loop.
#[derive(Clone, Copy)]
pub(super) struct Hoisted {
    fog_c: [f32; 3],
    sun: Vec3,
    fog_on: bool,
    lighting: bool,
}

impl Hoisted {
    #[inline(always)]
    pub(super) fn new<L>(s: &Shade<L>) -> Hoisted {
        Hoisted {
            fog_c: s.fog_c,
            sun: s.legacy_sun,
            fog_on: s.o.fog_end > s.o.fog_start,
            lighting: s.lit,
        }
    }
}

/// Colour of a surface point at depth `z` before the palette: light (from sample `sample` of
/// the light pass, or the legacy sun shading) and fog. Shared by opaque pixels and ALPHA
/// fragments. `prelit`: the colour is final (particles), only fog applies.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn surface_h<L: Column<[f32; 3]>>(
    s: &Shade<L>,
    h: &Hoisted,
    sample: usize,
    z: f32,
    normal: Vec3,
    light: f32,
    base: [f32; 3],
    prelit: bool,
) -> [f32; 3] {
    let o = &s.o;
    let Hoisted {
        fog_c,
        sun,
        fog_on,
        lighting,
    } = *h;
    if prelit {
        let f = fog_factor(o, z);
        return std::array::from_fn(|k| base[k] * f + fog_c[k] * (1.0 - f));
    }
    if lighting {
        let base_l = light;
        let l = s.light.get(sample).copied().unwrap_or([0.0; 3]);
        let l = l.map(|v| v * base_l);
        let l = step_light(l, o.light_levels).map(|v| v.clamp(0.0, 1.0));
        let mut f = 1.0;
        if fog_on {
            let t = ((z - o.fog_start) / (o.fog_end - o.fog_start)).clamp(0.0, 1.0);
            f = 1.0 - t * (1.0 - o.fog_min.clamp(0.0, 1.0));
        }
        let mut c = [0.0; 3];
        for k in 0..3 {
            c[k] = base[k] * l[k] * f + fog_c[k] * (1.0 - f);
        }
        c
    } else {
        let mut l = light;
        if o.sun_strength > 0.0 {
            let lam = (-normal.dot(sun)).max(0.0);
            l *= 1.0 - o.sun_strength + o.sun_strength * lam;
        }
        let mut f = 1.0;
        if fog_on {
            let t = ((z - o.fog_start) / (o.fog_end - o.fog_start)).clamp(0.0, 1.0);
            f = 1.0 - t * (1.0 - o.fog_min.clamp(0.0, 1.0));
        }
        l = l.clamp(0.0, 1.0);
        let mut lf = l * f;
        if o.light_levels > 0 {
            let n = o.light_levels as f32;
            lf = (lf * n).round() / n;
            f = if l > 0.0 { (lf / l).min(1.0) } else { 1.0 };
        }
        // Fog colour is added for the part the distance took away.
        let fade = if fog_on { 1.0 - f.min(1.0) } else { 0.0 };
        let mut c = [0.0; 3];
        for k in 0..3 {
            c[k] = base[k] * lf + fog_c[k] * fade;
        }
        c
    }
}

/// Fog factor at depth z: 1 = no fog.
pub(super) fn fog_factor(o: &Options, z: f32) -> f32 {
    if o.fog_end > o.fog_start {
        let t = ((z - o.fog_start) / (o.fog_end - o.fog_start)).clamp(0.0, 1.0);
        1.0 - t * (1.0 - o.fog_min.clamp(0.0, 1.0))
    } else {
        1.0
    }
}

/// Out-of-line copy of `surface_h` for fragments, so the hot opaque loop holds only one inlined copy.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn surface_frag<L: Column<[f32; 3]>>(
    s: &Shade<L>,
    sample: usize,
    z: f32,
    normal: Vec3,
    light: f32,
    base: [f32; 3],
    prelit: bool,
) -> [f32; 3] {
    surface_h(s, &Hoisted::new(s), sample, z, normal, light, base, prelit)
}

/// Blends the transparent fragments of pixel i over colour c, far to near.
/// Kept out of line: frames without transparency must not pay for it in the shading loop.
#[inline(never)]
#[cold]
fn blend_pixel<L: Column<[f32; 3]>>(
    s: &Shade<L>,
    i: usize,
    c: &mut [f32; 3],
    order: &mut Vec<u32>,
) {
    let g = s.g;
    let screen = g.width * g.height;
    g.frags.sorted(i, g.depth[i], order);
    for &k in order.iter() {
        let f = &g.frags.frags[k as usize];
        let a = f.alpha;
        if f.mode == BLEND_ADD {
            let fade = a * fog_factor(&s.o, f.depth);
            for (ch, v) in c.iter_mut().zip(s.base(f.color, f.prelit, f.owner)) {
                *ch += v * fade;
            }
        } else {
            let fc = surface_frag(
                s,
                screen + k as usize,
                f.depth,
                f.normal,
                f.light,
                s.base(f.color, f.prelit, f.owner),
                f.prelit,
            );
            for (ch, v) in c.iter_mut().zip(fc) {
                *ch = *ch * (1.0 - a) + v * a;
            }
        }
    }
}

/// Shade the rows starting at `y0` into `rgba` / `indexed` (whole rows of the frame); `sky`:
/// the same rows of the backdrop ray angle cache.
pub(super) fn shade_rows<L: Column<[f32; 3]>>(
    s: &Shade<L>,
    y0: usize,
    rgba: &mut [u8],
    indexed: &mut [u8],
    sky: Option<&mut [[f32; 2]]>,
    order: &mut Vec<u32>,
) {
    // One instantiation per feature mix, so the plain frame runs the loop without any feature tests.
    match (!s.g.frags.is_empty(), s.g.prelit_any) {
        (false, false) => shade_rows_impl::<false, false, L>(s, y0, rgba, indexed, sky, order),
        (true, false) => shade_rows_impl::<true, false, L>(s, y0, rgba, indexed, sky, order),
        (false, true) => shade_rows_impl::<false, true, L>(s, y0, rgba, indexed, sky, order),
        (true, true) => shade_rows_impl::<true, true, L>(s, y0, rgba, indexed, sky, order),
    }
}

fn shade_rows_impl<const BLEND: bool, const PRELIT: bool, L: Column<[f32; 3]>>(
    s: &Shade<L>,
    y0: usize,
    rgba: &mut [u8],
    indexed: &mut [u8],
    mut sky: Option<&mut [[f32; 2]]>,
    order: &mut Vec<u32>,
) {
    let (o, g, w) = (&s.o, s.g, s.width);
    let bg = crate::srgb::lin_of(o.background);
    let srgb = s.srgb;
    let h = Hoisted::new(s);
    // `order`: fragment order of one pixel; the caller keeps its buffer between frames.
    for (ly, row) in rgba.chunks_mut(w * 4).enumerate() {
        let y = y0 + ly;
        for x in 0..w {
            let i = y * w + x;
            let z = g.depth[i];
            let mut c: [f32; 3];
            if z.is_finite() {
                let prelit = PRELIT && g.prelit[i];
                let base = s.base(g.color[i], prelit, g.owner[i]);
                c = surface_h(s, &h, i, z, g.normal[i], g.light[i], base, prelit);
            } else {
                c = bg;
                if o.lighting {
                    let t = s.sky_tint;
                    for k in 0..3 {
                        c[k] *= t[k];
                    }
                }
                if !s.backdrops.is_empty() {
                    let angles = sky.as_deref_mut().map(|k| &mut k[ly * w + x]);
                    crate::backdrop::composite(
                        s.backdrops,
                        s.textures,
                        &s.view,
                        &s.backdrop_look,
                        x,
                        y,
                        &mut c,
                        angles,
                    );
                }
            }
            if BLEND {
                blend_pixel(s, i, &mut c, order);
            }
            // The one conversion back to sRGB (0..255 scale, not yet rounded).
            let c = c.map(|v| srgb.encode(v));
            let out = &mut row[x * 4..x * 4 + 4];
            if let Some(pal) = s.palette {
                // With lighting, pure black stays pure black (no dither noise in dark dungeons).
                let black = o.lighting && c == [0.0; 3];
                let d = if o.dither && !black {
                    (BAYER4[y & 3][x & 3] as f32 + 0.5) / 16.0 - 0.5
                } else {
                    0.0
                };
                let q = |v: f32| (v + d * o.dither_strength).round().clamp(0.0, 255.0) as u8;
                let idx = pal.nearest([q(c[0]), q(c[1]), q(c[2])]);
                indexed[ly * w + x] = idx;
                if o.quantize {
                    let p = pal.colors[idx as usize];
                    out.copy_from_slice(&[p[0], p[1], p[2], 255]);
                    continue;
                }
            }
            let r = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            out.copy_from_slice(&[r(c[0]), r(c[1]), r(c[2]), 255]);
        }
    }
}
