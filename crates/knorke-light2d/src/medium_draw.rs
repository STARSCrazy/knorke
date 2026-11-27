// Origin: chiaro @ 9bc5b9b, src/integration/MediumDraw.cpp (the validation of DrawMediums, ColorOf,
// DrawParams) and CHI_MediumDraw of include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Medium clouds (P30): the CPU part of drawing smoke, steam and fog into the medium layer.
//!
//! A [`MediumDraw`] is one rectangle of medium; [`MediumDraw::params`] gives the uniform block
//! of `medium_draw.vert`, whose colour is the encoded medium pixel ([`encode_medium_pixel`]).
//! The GPU part (pipelines, the pass on the layer) is `MediumBatch` of `knorke-integration`.
//! Overlapping clouds blend MAX on the heights and the kind and ADD on the density (saturating
//! at 1), so the order of the draws does not matter.

use knorke_math::{At, Vec2, World2, det, uniform_block};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::codec::{MEDIUM_COUNT, encode_medium_pixel};
use crate::projection::Projection;

/// A [`MediumDraw`] with a value out of range; the message names the field (`draws[i].` in a
/// batch).
pub const MEDIUM_DRAW_INVALID: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.medium_draw.invalid");

/// One cloud of medium (Chiaro `CHI_MediumDraw` without the texture, which the GPU part holds).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MediumDraw {
    /// x, y, width, height in layer pixels; finite, width and height >= 0.
    pub rect: [f32; 4],
    /// 0 ... 1 before the shape; overlapping densities add up.
    pub density: f32,
    /// Lower edge in world units, finite and >= 0.
    pub bottom: f32,
    /// Upper edge in world units, finite and >= 0.
    pub top: f32,
    /// Entry of the media table, below [`MEDIUM_COUNT`].
    pub medium: u32,
}

uniform_block! {
    /// `cbuffer DrawParams` of `medium_draw.vert`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct MediumDrawParams [48] {
        /// x, y, width, height in layer pixels.
        pub rect: [f32; 4],
        /// The encoded pixel as bytes / 255; A the density itself under a shape.
        pub color: [f32; 4],
        /// xy layer size in pixels.
        pub layer: [f32; 4],
    }
}

impl MediumDraw {
    /// A cloud placed in the world (Chiaro `CHI_MediumCloud`, E112): a disc of `radius` around the
    /// ground point `center`, between the heights `bottom` and `top`. `rect` is the bounding box of
    /// that disc projected at the middle height, because the medium stage places a layer pixel at the
    /// middle of its slab: so the cloud lands on the disc in the world. Top-down that is a square of
    /// `2 * radius` world units, in [`Projection::iso`] a box twice as wide as high. Pass the
    /// projection the layer is drawn with. No validation: [`MediumDraw::validate`] checks the result.
    pub fn cloud(
        projection: &Projection,
        center: At<World2, Vec2>,
        radius: f32,
        bottom: f32,
        top: f32,
        density: f32,
        medium: u32,
    ) -> Self {
        let p = projection;
        let c = p.project(center, 0.5 * (bottom + top)).raw();
        let hw = radius * det::hypot(p.x_axis.x, p.y_axis.x);
        let hh = radius * det::hypot(p.x_axis.y, p.y_axis.y);
        Self {
            rect: [c.x - hw, c.y - hh, 2.0 * hw, 2.0 * hh],
            density,
            bottom,
            top,
            medium,
        }
    }

    /// One puff around the world point `(center, z)` (`AddPuff` of Chiaro's `SmokePuffs.hpp`): a
    /// [`MediumDraw::cloud`] of `radius` with the height span `z ± radius / 2`, clamped at the
    /// ground, and the density clamped to `[0, 1]`. Its shape is a texture of [`smoke_puff`].
    pub fn puff(
        projection: &Projection,
        center: At<World2, Vec2>,
        z: f32,
        radius: f32,
        density: f32,
        medium: u32,
    ) -> Self {
        let bottom = det::max(0.0, z - radius * 0.5);
        let top = det::max(bottom, z + radius * 0.5);
        let density = density.clamp(0.0, 1.0);
        Self::cloud(projection, center, radius, bottom, top, density, medium)
    }

    /// Checks the ranges in Chiaro's order; `index` names the entry of a batch in the message.
    ///
    /// # Errors
    /// [`MEDIUM_DRAW_INVALID`] naming the first bad field.
    pub fn validate(&self, index: Option<usize>) -> Result<(), Diagnostic> {
        let bad = |what: core::fmt::Arguments<'_>| {
            let at = index.map_or(String::new(), |i| format!("draws[{i}]."));
            Err(Diagnostic::error(
                MEDIUM_DRAW_INVALID,
                format!("{at}{what}"),
            ))
        };
        let [x, y, w, h] = self.rect;
        if self.medium >= MEDIUM_COUNT {
            return bad(format_args!(
                "medium {} must be below MEDIUM_COUNT ({MEDIUM_COUNT})",
                self.medium
            ));
        }
        if !(0.0..=1.0).contains(&self.density) {
            return bad(format_args!(
                "density must be in [0, 1] (got {})",
                self.density
            ));
        }
        if !(self.bottom.is_finite() && self.bottom >= 0.0) {
            return bad(format_args!(
                "bottom must be finite and >= 0 (got {})",
                self.bottom
            ));
        }
        if !(self.top.is_finite() && self.top >= 0.0) {
            return bad(format_args!(
                "top must be finite and >= 0 (got {})",
                self.top
            ));
        }
        if !(self.rect.iter().all(|v| v.is_finite()) && w >= 0.0 && h >= 0.0) {
            return bad(format_args!(
                "rect must be finite with w, h >= 0 (got {x}, {y}, {w}, {h})"
            ));
        }
        Ok(())
    }

    /// The vertex colour (Chiaro `ColorOf`): the pixel of [`encode_medium_pixel`] as bytes /
    /// 255; with a shape A stays the density itself, which `medium_draw.frag` scales by the
    /// shape's alpha. `None` if the medium is out of range.
    pub fn color(&self, height_step: f32, shaped: bool) -> Option<[f32; 4]> {
        let b = encode_medium_pixel(
            self.density,
            self.bottom,
            self.top,
            self.medium,
            height_step,
        )
        .ok()?;
        let f = |v: u8| f32::from(v) / 255.0;
        Some([
            f(b[0]),
            f(b[1]),
            f(b[2]),
            if shaped { self.density } else { f(b[3]) },
        ])
    }

    /// The uniform block of this draw into a layer of `layer` pixels.
    pub fn params(&self, height_step: f32, shaped: bool, layer: (u32, u32)) -> MediumDrawParams {
        MediumDrawParams {
            rect: self.rect,
            color: self.color(height_step, shaped).unwrap_or_default(),
            layer: [layer.0 as f32, layer.1 as f32, 0.0, 0.0],
        }
    }
}

/// Deterministic hash to `[0, 1)` (`Hash` of `SmokePuffs.hpp`).
fn hash(mut n: u32) -> f32 {
    n = (n ^ 61) ^ (n >> 16);
    n = n.wrapping_mul(9);
    n ^= n >> 4;
    n = n.wrapping_mul(0x27d4_eb2d);
    n ^= n >> 15;
    (n & 0xff_ffff) as f32 / 16_777_216.0
}

/// Value noise with smooth interpolation, `[0, 1)`; `period > 0` wraps the lattice in x, so that
/// an angle loops seamlessly.
fn value_noise(x: f32, y: f32, seed: u32, period: i32) -> f32 {
    let lattice = |x: i32, y: i32| {
        hash(
            (x as u32).wrapping_mul(73_856_093)
                ^ (y as u32).wrapping_mul(19_349_663)
                ^ seed.wrapping_mul(83_492_791),
        )
    };
    let wrap = |v: i32| {
        if period > 0 {
            ((v % period) + period) % period
        } else {
            v
        }
    };
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (a, b) = (lattice(wrap(x0), y0), lattice(wrap(x0 + 1), y0));
    let (c, d) = (lattice(wrap(x0), y0 + 1), lattice(wrap(x0 + 1), y0 + 1));
    (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy
}

/// Fractal value noise of `octaves` octaves, normalized to `[0, 1)`.
fn fbm(mut x: f32, mut y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0f32, 0.5f32, 0.0f32);
    for o in 0..octaves {
        sum += amp * value_noise(x, y, seed.wrapping_add(o * 131), 0);
        norm += amp;
        x *= 2.03;
        y *= 2.03;
        amp *= 0.5;
    }
    sum / norm
}

/// The shape of one smoke puff (`MakeSmokePuff` of Chiaro's `SmokePuffs.hpp`) as RGBA8 into `out`
/// (`size * size * 4` bytes, rows top down): white, alpha the shape. An irregular outline (the
/// radius follows periodic noise over the angle), the inside broken up by fbm, a soft but ragged
/// edge; no two seeds look alike. Chiaro's demos use `size` 128 and the seeds `i * 977 + 13` for
/// eight variants. Load time, not per frame.
///
/// # Panics
/// If `out` is not `size * size * 4` bytes long.
pub fn smoke_puff(seed: u32, size: u32, out: &mut [u8]) {
    assert_eq!(
        out.len(),
        (size * size * 4) as usize,
        "smoke_puff: out is not size * size * 4 bytes"
    );
    let half = size as f32 * 0.5;
    let ox = hash(seed.wrapping_mul(7).wrapping_add(1)) * 40.0;
    let oy = hash(seed.wrapping_mul(7).wrapping_add(2)) * 40.0;
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % size) as f32, (i as u32 / size) as f32);
        let dx = (x + 0.5 - half) / half;
        let dy = (y + 0.5 - half) / half;
        let d = (dx * dx + dy * dy).sqrt();
        let ang = (det::atan2(dy, dx) / core::f32::consts::TAU + 0.5) * 6.0; // 6 lattice cells around
        // Outline radius: lobes (low frequency) plus ragged detail.
        let lobes = value_noise(ang, 0.5, seed, 6);
        let rag = value_noise(ang * 3.0, 3.5, seed.wrapping_add(17), 18);
        let edge = 0.42 + 0.30 * lobes + 0.10 * rag;
        // Inside: fbm in a fixed frame, so the texture has its own cloud structure.
        let n = fbm(dx * 3.2 + ox, dy * 3.2 + oy, seed.wrapping_add(5), 5);
        let t = ((edge - d) / 0.35).clamp(0.0, 1.0);
        let body = t * t * (3.0 - 2.0 * t);
        let a = (body * (0.25 + 1.3 * n) - (1.0 - body) * 0.2).clamp(0.0, 1.0);
        *px = [255, 255, 255, (a * 255.0 + 0.5) as u8];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: MediumDraw = MediumDraw {
        rect: [1.0, 1.0, 8.0, 6.0],
        density: 0.5,
        bottom: 0.0,
        top: 2.0,
        medium: 1,
    };

    fn message(d: MediumDraw, index: Option<usize>) -> String {
        let e = d.validate(index).unwrap_err();
        assert_eq!(e.code, MEDIUM_DRAW_INVALID);
        e.message.to_string()
    }

    // Port of "medium draws outside their window and invalid draws fail with the field name"
    // (MediumLayerTests.cpp), the field checks.
    #[test]
    fn invalid_draws_fail_with_the_field_name() {
        assert!(OK.validate(None).is_ok());
        let cases: [(MediumDraw, &str); 6] = [
            (MediumDraw { medium: 16, ..OK }, "medium"),
            (
                MediumDraw {
                    density: f32::NAN,
                    ..OK
                },
                "density",
            ),
            (MediumDraw { density: 1.5, ..OK }, "density"),
            (
                MediumDraw {
                    top: f32::INFINITY,
                    ..OK
                },
                "top",
            ),
            (MediumDraw { bottom: -1.0, ..OK }, "bottom"),
            (
                MediumDraw {
                    rect: [1.0, 1.0, -1.0, 6.0],
                    ..OK
                },
                "rect",
            ),
        ];
        for (d, field) in cases {
            assert!(message(d, None).starts_with(field), "{field}");
        }
        assert!(message(MediumDraw { medium: 99, ..OK }, Some(1)).starts_with("draws[1].medium"));
        let nan = MediumDraw {
            rect: [f32::NAN, 0.0, 1.0, 1.0],
            ..OK
        };
        assert!(message(nan, None).starts_with("rect"));
    }

    // "overlapping medium draws ...": the colour of a single draw is the encoder's pixel.
    #[test]
    fn the_colour_is_the_encoded_pixel_and_the_density_under_a_shape() {
        let b = MediumDraw {
            rect: [5.0, 3.0, 8.0, 7.0],
            density: 0.4,
            bottom: 1.0,
            top: 4.0,
            medium: 3,
        };
        let px = encode_medium_pixel(0.4, 1.0, 4.0, 3, 1.0).unwrap();
        assert_eq!(px, [4, 254, 48, 102]);
        let c = b.color(1.0, false).unwrap();
        for (c, p) in c.iter().zip(px) {
            assert_eq!(*c, f32::from(p) / 255.0);
        }
        assert_eq!(b.color(1.0, true).unwrap()[3], 0.4);
        assert_eq!(MediumDraw { medium: 16, ..b }.color(1.0, false), None);
    }

    #[test]
    fn params_carry_the_rect_and_the_layer_size() {
        let p = OK.params(1.0, false, (16, 12));
        assert_eq!(p.rect, OK.rect);
        assert_eq!(p.layer, [16.0, 12.0, 0.0, 0.0]);
        let bytes = p.to_bytes();
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[32..36], &16.0f32.to_le_bytes());
    }

    // Port of the MediumCloud tests of tests/unit/OutputGradeTests.cpp (E112).

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4 * (1.0 + b.abs())
    }

    #[test]
    fn a_top_down_cloud_is_the_square_of_its_disc() {
        let p = Projection::ortho(At::new(Vec2::new(10.0, 20.0)), 4.0, 320, 180);
        let d = MediumDraw::cloud(&p, At::new(Vec2::new(12.0, 21.0)), 1.5, 0.5, 2.5, 0.2, 1);
        // Camera (10, 20) at the centre (160, 90); 4 pixels per unit; height has no effect top-down.
        let want = [
            160.0 + 2.0 * 4.0 - 1.5 * 4.0,
            90.0 + 4.0 - 1.5 * 4.0,
            12.0,
            12.0,
        ];
        assert!(
            d.rect.iter().zip(want).all(|(&a, b)| near(a, b)),
            "{:?}",
            d.rect
        );
        assert_eq!((d.density, d.bottom, d.top, d.medium), (0.2, 0.5, 2.5, 1));
        // The identity: one world unit per scene pixel.
        let d = MediumDraw::cloud(
            &Projection::IDENTITY,
            At::new(Vec2::new(30.0, 40.0)),
            5.0,
            0.0,
            3.0,
            1.0,
            0,
        );
        assert_eq!(d.rect, [25.0, 35.0, 10.0, 10.0]);
    }

    #[test]
    fn an_iso_cloud_is_the_box_of_its_disc_at_the_middle_height() {
        let (zoom, height_scale) = (8.0f32, 0.75f32);
        let p = Projection::iso(At::new(Vec2::ZERO), zoom, height_scale, 400, 300);
        let d = MediumDraw::cloud(&p, At::new(Vec2::new(3.0, 1.0)), 2.0, 1.0, 5.0, 0.15, 2);
        // Ground point (3, 1) at mid height 3: x = 200 + (3 - 1) * 8, y = 150 + (3 + 1) * 4 - 3 * 8 * 0.75.
        let (cx, cy) = (
            200.0 + 2.0 * zoom,
            150.0 + 4.0 * zoom * 0.5 - 3.0 * zoom * height_scale,
        );
        let (hw, hh) = (2.0 * zoom * 2f32.sqrt(), zoom * 2f32.sqrt());
        let want = [cx - hw, cy - hh, 2.0 * hw, 2.0 * hh];
        assert!(
            d.rect.iter().zip(want).all(|(&a, b)| near(a, b)),
            "{:?}",
            d.rect
        );
        assert!(near(d.rect[2], 2.0 * d.rect[3])); // twice as wide as high
        // The box holds the projected circle at mid height: its widest and highest points touch the edges.
        for i in 0..16 {
            let a = i as f32 * core::f32::consts::FRAC_PI_8;
            let w = Vec2::new(3.0 + 2.0 * det::cos(a), 1.0 + 2.0 * det::sin(a));
            let s = p.project(At::new(w), 3.0).raw();
            let [x, y, rw, rh] = d.rect;
            assert!(
                s.x >= x - 1e-3 && s.x <= x + rw + 1e-3 && s.y >= y - 1e-3 && s.y <= y + rh + 1e-3
            );
        }
    }
}
