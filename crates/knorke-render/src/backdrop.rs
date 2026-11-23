//! Backdrops: background layers behind all geometry (sky, mountain and forest bands, sun, moon).
//!
//! They are drawn only where no opaque geometry was drawn, back to front, before transparent
//! fragments are blended on top. They receive no light, cast no shadows, write no depth and are not
//! pickable. Directions use the camera axes (x east, y up, z south; azimuth 0 = north, positive turns
//! towards east).
//!
//! Band mapping (all angles in radians inside, degrees in the API). For the ray d through the pixel
//! centre (normalised):
//!   a = atan2(d.x, -d.z)                      azimuth of the ray
//!   e = asin(d.y)                             elevation of the ray
//!   u = (a - azimuth) / 2pi * repeats  (+ scroll u)
//!   v = 1 - (e - elevation) / height   (+ scroll v)
//! v = 0 is the top texel row (at elevation + height), v = 1 the bottom edge (at elevation). Outside
//! 0 <= v < 1 the band has a hole, unless CLAMP_TOP (v < 0 -> top row) or CLAMP_BOTTOM (v >= 1 ->
//! bottom row). The legacy sky (`Renderer::set_sky`) is the band elevation 0, height 90, both clamps,
//! alpha ignored, drawn behind every backdrop.

use crate::blend::{BLEND_ADD, BLEND_ALPHA, step_alpha};
use crate::camera::View;
use crate::texture::Texture;
use std::f32::consts::{PI, TAU};

/// A band around the horizon.
pub const BACKDROP_BAND: i32 = 0;
/// An upright image at one direction in the sky (sun, moon).
pub const BACKDROP_SPOT: i32 = 1;

/// Band: above its top edge show the top texel row instead of a hole.
pub const BACKDROP_CLAMP_TOP: u32 = 1;
/// Band: below its bottom edge show the bottom texel row instead of a hole.
pub const BACKDROP_CLAMP_BOTTOM: u32 = 2;
/// The sky tint (`set_sky_tint`) multiplies this layer (only with `Options::lighting`, like the sky).
pub const BACKDROP_TINT: u32 = 4;
/// Distance fog colours this layer as if it were infinitely far (`fog_min` of the fog colour mix).
pub const BACKDROP_FOG: u32 = 8;
/// Internal: texel alpha is ignored (legacy sky).
pub(crate) const BACKDROP_IGNORE_ALPHA: u32 = 1 << 31;
const PUBLIC_FLAGS: u32 = BACKDROP_CLAMP_TOP | BACKDROP_CLAMP_BOTTOM | BACKDROP_TINT | BACKDROP_FOG;

/// One background layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    /// BACKDROP_BAND or BACKDROP_SPOT.
    pub kind: i32,
    /// Texture id.
    pub texture: i32,
    /// Lower = further back; ties by id (lower id further back).
    pub order: i32,
    /// BACKDROP_* flags.
    pub flags: u32,
    /// Degrees. Band: azimuth where u = 0 lies. Spot: direction of the image centre.
    pub azimuth: f32,
    /// Degrees. Band: elevation of the bottom edge (0 = horizon, negative = below). Spot: elevation
    /// of the image centre.
    pub elevation: f32,
    /// Band: angular height in degrees (> 0).
    pub height: f32,
    /// Band: texture repeats per 360 degrees (<= 0 counts as 1).
    pub repeats: f32,
    /// Spot: wanted angular height of the image in degrees; it is drawn at the whole-number scale
    /// (texel = s x s pixels, s >= 1) nearest to that size at the current field of view. 0 = s = 1.
    pub size: f32,
    /// BLEND_NONE (texel alpha < 128 = hole), BLEND_ALPHA or BLEND_ADD (alpha * texel alpha, 1/8 steps).
    pub blend: i32,
    /// Opacity 0..1 for BLEND_ALPHA / BLEND_ADD.
    pub alpha: f32,
}

impl Default for Backdrop {
    fn default() -> Self {
        Backdrop {
            kind: BACKDROP_BAND,
            texture: -1,
            order: 0,
            flags: BACKDROP_TINT,
            azimuth: 0.0,
            elevation: 0.0,
            height: 90.0,
            repeats: 1.0,
            size: 0.0,
            blend: crate::blend::BLEND_NONE,
            alpha: 1.0,
        }
    }
}

impl Backdrop {
    /// Valid kind and finite numbers (texture checked by the renderer); band height > 0.
    pub fn is_valid(&self) -> bool {
        let fin = [
            self.azimuth,
            self.elevation,
            self.height,
            self.repeats,
            self.size,
            self.alpha,
        ]
        .iter()
        .all(|v| v.is_finite());
        fin && match self.kind {
            BACKDROP_BAND => self.height > 0.0,
            BACKDROP_SPOT => self.size >= 0.0,
            _ => false,
        }
    }

    pub(crate) fn public(mut self) -> Backdrop {
        self.flags &= PUBLIC_FLAGS;
        self
    }
}

fn rad(deg: f32) -> f32 {
    // dividing first keeps 90 degrees exactly FRAC_PI_2 (the legacy sky)
    deg / 180.0 * PI
}

/// A layer ready for one frame (texture frame and scroll resolved, spot placed on screen).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Layer {
    Band {
        tex: usize,
        az: f32,
        lo: f32,
        h: f32,
        rep: f32,
        off: Option<[f32; 2]>,
        flags: u32,
        blend: i32,
        alpha: f32,
    },
    /// Screen rectangle x0..x0 + w * s, y0..y0 + h * s.
    Spot {
        tex: usize,
        x0: i64,
        y0: i64,
        s: i64,
        w: i64,
        h: i64,
        flags: u32,
        blend: i32,
        alpha: f32,
    },
}

impl Layer {
    /// `tex` is the texture frame to sample, `t` that texture.
    pub(crate) fn new(
        b: &Backdrop,
        tex: usize,
        t: &Texture,
        off: Option<[f32; 2]>,
        view: &View,
    ) -> Option<Layer> {
        let (blend, alpha) = (b.blend, b.alpha);
        match b.kind {
            BACKDROP_BAND => Some(Layer::Band {
                tex,
                az: rad(b.azimuth),
                lo: rad(b.elevation),
                h: rad(b.height),
                rep: if b.repeats > 0.0 { b.repeats } else { 1.0 },
                off,
                flags: b.flags,
                blend,
                alpha,
            }),
            BACKDROP_SPOT => {
                let (sa, ca) = rad(b.azimuth).sin_cos();
                let (se, ce) = rad(b.elevation).sin_cos();
                let d = crate::math::Vec3::new(ce * sa, se, -ce * ca);
                let vz = d.dot(view.forward);
                if vz <= 1e-6 {
                    return None;
                }
                let px = view.cx + d.dot(view.right) / vz * view.focal;
                let py = view.cy - d.dot(view.up) / vz * view.focal;
                let (w, h) = (t.width as i64, t.height as i64);
                let s = if b.size > 0.0 {
                    ((rad(b.size) * view.focal / h as f32).round() as i64).clamp(1, 1 << 12)
                } else {
                    1
                };
                if !(px.abs() < 1e7 && py.abs() < 1e7) {
                    return None;
                }
                let x0 = (px - (w * s) as f32 * 0.5).round() as i64;
                let y0 = (py - (h * s) as f32 * 0.5).round() as i64;
                Some(Layer::Spot {
                    tex,
                    x0,
                    y0,
                    s,
                    w,
                    h,
                    flags: b.flags,
                    blend,
                    alpha,
                })
            }
            _ => None,
        }
    }
}

/// How a layer colour is changed before it is laid on: sky tint (lighting on) and fog.
pub(crate) struct Look {
    pub tint: Option<[f32; 3]>,
    /// fog_min and fog colour, when fog is on.
    pub fog: Option<(f32, [f32; 3])>,
}

/// Elevation and azimuth of the ray through pixel (x, y) (see the band mapping above).
#[inline]
pub(crate) fn ray_angles(view: &View, x: usize, y: usize) -> [f32; 2] {
    let d = view.ray(x, y);
    [d.y.asin(), d.x.atan2(-d.z)]
}

/// Lays the layers over the background colour `c` of pixel (x, y), back to front.
/// `angles`: `ray_angles` of the pixel kept from an earlier frame with the same view (NaN = not
/// kept yet; filled here). Both only depend on the view and the pixel, so a kept pair is exactly
/// what `ray_angles` would give, and the two library calls per pixel are skipped.
#[inline]
pub(crate) fn composite(
    layers: &[Layer],
    textures: &[Texture],
    view: &View,
    look: &Look,
    x: usize,
    y: usize,
    c: &mut [f32; 3],
    angles: Option<&mut [f32; 2]>,
) {
    let mut ang: Option<[f32; 2]> = None;
    let mut keep = angles;
    for l in layers {
        let (texel, flags, blend, alpha) = match *l {
            Layer::Band {
                tex,
                az,
                lo,
                h,
                rep,
                off,
                flags,
                blend,
                alpha,
            } => {
                let [e, a] = *ang.get_or_insert_with(|| match keep.as_deref_mut() {
                    Some(k) if !k[0].is_nan() && !k[1].is_nan() => *k,
                    Some(k) => {
                        *k = ray_angles(view, x, y);
                        *k
                    }
                    None => ray_angles(view, x, y),
                });
                let mut v = 1.0 - (e - lo) / h;
                if v < 0.0 && flags & BACKDROP_CLAMP_TOP == 0
                    || v >= 1.0 && flags & BACKDROP_CLAMP_BOTTOM == 0
                    || v.is_nan()
                {
                    continue;
                }
                v = v.clamp(0.0, 0.9999);
                let mut u = (a - az) / TAU * rep;
                if let Some([du, dv]) = off {
                    u += du;
                    v += dv;
                }
                (textures[tex].sample(u, v), flags, blend, alpha)
            }
            Layer::Spot {
                tex,
                x0,
                y0,
                s,
                w,
                h,
                flags,
                blend,
                alpha,
            } => {
                let (dx, dy) = (x as i64 - x0, y as i64 - y0);
                if dx < 0 || dy < 0 || dx >= w * s || dy >= h * s {
                    continue;
                }
                (
                    textures[tex].texels[((dy / s) * w + dx / s) as usize],
                    flags,
                    blend,
                    alpha,
                )
            }
        };
        let a = if blend == BLEND_ALPHA || blend == BLEND_ADD {
            step_alpha(alpha * texel[3] as f32 / 255.0)
        } else if texel[3] >= 128 || flags & BACKDROP_IGNORE_ALPHA != 0 {
            1.0
        } else {
            0.0
        };
        if a <= 0.0 {
            continue;
        }
        let mut v = [texel[0] as f32, texel[1] as f32, texel[2] as f32];
        if let (Some(t), true) = (look.tint, flags & BACKDROP_TINT != 0) {
            for k in 0..3 {
                v[k] *= t[k];
            }
        }
        if let (Some((f, fc)), true) = (look.fog, flags & BACKDROP_FOG != 0) {
            for k in 0..3 {
                v[k] = v[k] * f + fc[k] * (1.0 - f);
            }
        }
        if blend == BLEND_ADD {
            for k in 0..3 {
                c[k] += v[k] * a;
            }
        } else if a >= 1.0 {
            *c = v;
        } else {
            for k in 0..3 {
                c[k] = c[k] * (1.0 - a) + v[k] * a;
            }
        }
    }
}
