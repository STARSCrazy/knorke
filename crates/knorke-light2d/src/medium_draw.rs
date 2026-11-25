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

use knorke_math::uniform_block;
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::codec::{MEDIUM_COUNT, encode_medium_pixel};

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
}
