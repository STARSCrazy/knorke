// Origin: chiaro @ 9bc5b9b, src/core/Projection.hpp and Projection.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! World to scene-pixel projection (P5).
//!
//! Redesign against the C++ version: world positions and scene pixels are typed spaces
//! ([`World2`], [`Screen`]); the plain `CHI_Projection` struct becomes [`Projection`] with
//! constructors and a [`Result`]-returning [`Projection::validate`] (stable diagnostic codes
//! instead of the SDL error string).

use knorke_math::{Affine2, At, Screen, Transform2, Vec2, Vec3, World2};
use knorke_types::{Diagnostic, DiagnosticCode};

/// How world coordinates map onto the scene: `scene = origin + x * x_axis + y * y_axis +
/// z * z_axis`, with `z` the height above the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    /// Scene pixel of the world origin.
    pub origin: At<Screen, Vec2>,
    /// Scene pixels per world unit along x.
    pub x_axis: Vec2,
    /// Scene pixels per world unit along y; must not be collinear with `x_axis`.
    pub y_axis: Vec2,
    /// Scene pixels per world unit of height; zero for top-down views.
    pub z_axis: Vec2,
}

impl Default for Projection {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Why a [`Projection`] is unusable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    /// A value is NaN or infinite.
    NotFinite,
    /// `x_axis` and `y_axis` are (nearly) parallel, so the ground plane collapses to a line.
    CollinearAxes,
}

impl core::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "projection values must be finite",
            Self::CollinearAxes => "projection x_axis and y_axis must not be collinear",
        })
    }
}

impl std::error::Error for ProjectionError {}

impl ProjectionError {
    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        let code = match self {
            Self::NotFinite => DiagnosticCode::new("knorke.light2d.projection.not_finite"),
            Self::CollinearAxes => DiagnosticCode::new("knorke.light2d.projection.collinear_axes"),
        };
        Diagnostic::error(code, self.to_string())
    }
}

impl Projection {
    /// `x_axis.x * y_axis.y - x_axis.y * y_axis.x` and the rows of the inverse of
    /// `[x_axis y_axis]`, in double as in Chiaro (one inverse for every block that maps scene
    /// pixels back to the ground, so their world points agree).
    pub(crate) fn inverse_axes(&self) -> (f64, [f32; 4]) {
        let (x, y) = (self.x_axis, self.y_axis);
        let det = f64::from(x.x) * f64::from(y.y) - f64::from(x.y) * f64::from(y.x);
        let inv = [
            (f64::from(y.y) / det) as f32,
            (-f64::from(y.x) / det) as f32,
            (-f64::from(x.y) / det) as f32,
            (f64::from(x.x) / det) as f32,
        ];
        (det, inv)
    }

    /// One world unit per scene pixel, height without effect.
    pub const IDENTITY: Self = Self {
        origin: At::new(Vec2::ZERO),
        x_axis: Vec2::X,
        y_axis: Vec2::Y,
        z_axis: Vec2::ZERO,
    };

    /// Camera at the scene center, `zoom` scene pixels per world unit, top-down.
    pub fn ortho(camera: At<World2, Vec2>, zoom: f32, scene_w: u32, scene_h: u32) -> Self {
        let camera = camera.raw();
        Self {
            origin: At::new(Vec2::new(
                scene_w as f32 * 0.5 - camera.x * zoom,
                scene_h as f32 * 0.5 - camera.y * zoom,
            )),
            x_axis: Vec2::new(zoom, 0.0),
            y_axis: Vec2::new(0.0, zoom),
            z_axis: Vec2::ZERO,
        }
    }

    /// 2:1 isometric: x `(zoom, zoom/2)`, y `(-zoom, zoom/2)`, z `(0, -zoom * height_scale)`;
    /// the camera's ground point lands at the scene center.
    pub fn iso(
        camera: At<World2, Vec2>,
        zoom: f32,
        height_scale: f32,
        scene_w: u32,
        scene_h: u32,
    ) -> Self {
        let camera = camera.raw();
        let x_axis = Vec2::new(zoom, zoom * 0.5);
        let y_axis = Vec2::new(-zoom, zoom * 0.5);
        Self {
            origin: At::new(Vec2::new(
                scene_w as f32 * 0.5 - (camera.x * x_axis.x + camera.y * y_axis.x),
                scene_h as f32 * 0.5 - (camera.x * x_axis.y + camera.y * y_axis.y),
            )),
            x_axis,
            y_axis,
            z_axis: Vec2::new(0.0, -zoom * height_scale),
        }
    }

    /// Checks for non-finite values and collinear x/y axes.
    pub fn validate(&self) -> Result<(), ProjectionError> {
        let o = self.origin.raw();
        let values = [
            o.x,
            o.y,
            self.x_axis.x,
            self.x_axis.y,
            self.y_axis.x,
            self.y_axis.y,
            self.z_axis.x,
            self.z_axis.y,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Err(ProjectionError::NotFinite);
        }
        let det = f64::from(self.x_axis.x) * f64::from(self.y_axis.y)
            - f64::from(self.x_axis.y) * f64::from(self.y_axis.x);
        if det.abs() < 1e-12 {
            return Err(ProjectionError::CollinearAxes);
        }
        Ok(())
    }

    /// World position at `height` above the ground to scene pixel.
    #[inline]
    pub fn project(&self, p: At<World2, Vec2>, height: f32) -> At<Screen, Vec2> {
        let (p, o) = (p.raw(), self.origin.raw());
        At::new(Vec2::new(
            o.x + p.x * self.x_axis.x + p.y * self.y_axis.x + height * self.z_axis.x,
            o.y + p.x * self.x_axis.y + p.y * self.y_axis.y + height * self.z_axis.y,
        ))
    }

    /// Scene-pixel image of a world vector (no origin).
    #[inline]
    pub fn image_of(&self, v: Vec3) -> Vec2 {
        Vec2::new(
            self.x_axis.x * v.x + self.y_axis.x * v.y + self.z_axis.x * v.z,
            self.x_axis.y * v.x + self.y_axis.y * v.y + self.z_axis.y * v.z,
        )
    }

    /// The ground plane (height 0) as a typed transform, for bulk work with `knorke-math`.
    pub fn ground_transform(&self) -> Transform2<World2, Screen> {
        Transform2::new(Affine2 {
            x_axis: self.x_axis,
            y_axis: self.y_axis,
            translation: self.origin.raw(),
        })
    }
}
