// Origin: chiaro @ 9bc5b9b, src/integration/SurfaceGeometry.hpp and SurfaceGeometry.cpp (the surface
// drawing helpers and CHI_EncodeSurfaceSprite) and CHI_SurfaceBox / CHI_SurfaceFace /
// CHI_SpriteShape of include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Surface geometry and surface sprites (P35): CPU encoders of the surface layer.
//!
//! - [`SurfaceGeometry`] collects triangles in layer pixels whose vertex colour is the encoded
//!   surface pixel ([`surface_pixel`]) of a box, wall, floor or quad. Height is affine across a
//!   planar face and so is its screen projection, so colours interpolated over a triangle encode
//!   every pixel of the face; drawn unblended into the surface layer they give its exact height
//!   and normal (`knorke-integration` draws them on the GPU).
//! - [`encode_surface_sprite`] encodes the surface map of a pre-drawn sprite from its alpha
//!   shape, once at load time: facades, roofs and standing figures.
//!
//! Redesign against the C++ version: the geometry is a triangle list without indices (the GPU
//! draw reads it from a storage buffer, without vertex or index buffers); the projection is an
//! argument, not state of the layers; a failed call appends nothing; errors are typed
//! ([`SurfaceGeometryError`]) with stable diagnostic codes; face kinds are an enum, so an
//! unknown kind cannot occur; the projection is validated by every call.

use knorke_math::{At, Screen, Vec2, Vec3, World2, det};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::codec::surface_pixel;
use crate::projection::{Projection, ProjectionError};

/// An upright box standing on the ground, axis-aligned in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceBox {
    /// Centre of the footprint.
    pub center: At<World2, Vec2>,
    /// Extent along world x, > 0.
    pub width: f32,
    /// Extent along world y, > 0.
    pub depth: f32,
    /// Top above the ground, >= 0.
    pub height: f32,
}

/// One vertex of [`SurfaceGeometry`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceVertex {
    /// Position in layer pixels.
    pub position: At<Screen, Vec2>,
    /// The encoded surface pixel ([`surface_pixel`]) at this vertex, RGBA.
    pub pixel: [u8; 4],
}

/// Why a surface helper refused its input. Nothing is appended or written in that case.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceGeometryError {
    /// The height step is not finite and > 0.
    HeightStep(f32),
    /// The projection is unusable.
    Projection(ProjectionError),
    /// A field of `boxes[index]` is out of range (`field` is `"x, y"` for a non-finite centre).
    Box {
        /// Index of the box.
        index: usize,
        /// The field.
        field: &'static str,
        /// Its value.
        got: f32,
    },
    /// A wall end is not finite or below the ground.
    WallEnds,
    /// The wall top lies below an end or is not finite.
    WallTop(f32),
    /// Both wall ends are the same point on the ground.
    WallLength,
    /// A floor needs at least three points.
    FloorCount(usize),
    /// The floor height is not finite and >= 0.
    FloorHeight(f32),
    /// A floor point is not finite.
    FloorPoint(usize),
    /// A quad corner is not finite or below the ground.
    QuadCorner(usize),
    /// The quad corners span no area.
    QuadArea,
    /// A quad corner lies more than 1 % of the quad size off its plane.
    QuadNotPlanar {
        /// The corner.
        corner: usize,
        /// Its distance to the plane.
        off: f32,
    },
    /// The sprite shape has a size outside 1..=16384 or a pitch below `width * 4`.
    ShapeSize {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Bytes per row.
        pitch: usize,
    },
    /// The foot line of the sprite shape is not finite.
    FootLine,
    /// No face given.
    NoFaces,
    /// `faces[index]` has a negative or non-finite height or a non-finite azimuth.
    Face {
        /// Index of the face.
        index: usize,
        /// Its height.
        height: f32,
        /// Its azimuth.
        azimuth: f32,
    },
    /// A pixel inside the shape names a face that does not exist.
    FaceId {
        /// The face index.
        id: u8,
        /// Pixel column.
        x: u32,
        /// Pixel row.
        y: u32,
        /// Number of faces.
        faces: usize,
    },
    /// A buffer is shorter than the sprite needs.
    BufferTooSmall {
        /// Which buffer.
        what: &'static str,
        /// Bytes needed.
        needed: usize,
        /// Bytes given.
        got: usize,
    },
}

impl core::fmt::Display for SurfaceGeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::HeightStep(got) => write!(f, "height_step must be > 0 (got {got})"),
            Self::Projection(e) => e.fmt(f),
            Self::Box { index, field, got } => {
                let rule = match field {
                    "width" | "depth" => "> 0",
                    "height" => ">= 0",
                    _ => "finite",
                };
                write!(f, "boxes[{index}].{field} must be {rule} (got {got})")
            }
            Self::WallEnds => f.write_str("wall ends a and b must be finite with z >= 0"),
            Self::WallTop(got) => write!(f, "wall top must be >= a.z and b.z (got {got})"),
            Self::WallLength => f.write_str("wall ends a and b must be distinct points"),
            Self::FloorCount(got) => write!(f, "a floor needs a count >= 3 points (got {got})"),
            Self::FloorHeight(got) => write!(f, "floor height must be >= 0 (got {got})"),
            Self::FloorPoint(i) => write!(f, "floor points[{i}] is not finite"),
            Self::QuadCorner(i) => write!(f, "quad corners[{i}] must be finite with z >= 0"),
            Self::QuadArea => f.write_str("the quad corners span no area"),
            Self::QuadNotPlanar { corner, off } => write!(
                f,
                "the quad corners are not planar (corner {corner} is {off} off)"
            ),
            Self::ShapeSize {
                width,
                height,
                pitch,
            } => write!(
                f,
                "shape needs a size of 1..=16384 and pitch >= width * 4 (got {width}x{height}, pitch {pitch})"
            ),
            Self::FootLine => f.write_str("shape foot line must be finite"),
            Self::NoFaces => f.write_str("faces must hold at least one face"),
            Self::Face {
                index,
                height,
                azimuth,
            } => write!(
                f,
                "faces[{index}]: height must be finite and >= 0, azimuth finite (got {height}, {azimuth})"
            ),
            Self::FaceId { id, x, y, faces } => write!(
                f,
                "face_ids has index {id} at ({x}, {y}), but only {faces} faces"
            ),
            Self::BufferTooSmall { what, needed, got } => {
                write!(f, "{what} has {got} bytes but the sprite needs {needed}")
            }
        }
    }
}

impl std::error::Error for SurfaceGeometryError {}

impl SurfaceGeometryError {
    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        let code = match self {
            Self::Projection(e) => return e.to_diagnostic(),
            Self::HeightStep(_) => "knorke.light2d.surface_geometry.height_step",
            Self::Box { .. } => "knorke.light2d.surface_geometry.box",
            Self::WallEnds | Self::WallTop(_) | Self::WallLength => {
                "knorke.light2d.surface_geometry.wall"
            }
            Self::FloorCount(_) | Self::FloorHeight(_) | Self::FloorPoint(_) => {
                "knorke.light2d.surface_geometry.floor"
            }
            Self::QuadCorner(_) | Self::QuadArea | Self::QuadNotPlanar { .. } => {
                "knorke.light2d.surface_geometry.quad"
            }
            Self::ShapeSize { .. } | Self::FootLine => "knorke.light2d.surface_geometry.shape",
            Self::NoFaces | Self::Face { .. } => "knorke.light2d.surface_geometry.face",
            Self::FaceId { .. } => "knorke.light2d.surface_geometry.face_id",
            Self::BufferTooSmall { .. } => "knorke.light2d.surface_geometry.buffer",
        };
        Diagnostic::error(DiagnosticCode::new(code), self.to_string())
    }
}

/// Checks what every helper needs: a usable projection and a positive height step.
fn check_view(projection: &Projection, height_step: f32) -> Result<(), SurfaceGeometryError> {
    projection
        .validate()
        .map_err(SurfaceGeometryError::Projection)?;
    if !(height_step > 0.0) || !height_step.is_finite() {
        return Err(SurfaceGeometryError::HeightStep(height_step));
    }
    Ok(())
}

/// Checks one box; `index` names it in the error.
pub fn validate_surface_box(b: &SurfaceBox, index: usize) -> Result<(), SurfaceGeometryError> {
    let c = b.center.raw();
    let bad = |field, got| Err(SurfaceGeometryError::Box { index, field, got });
    if !c.x.is_finite() || !c.y.is_finite() {
        return bad("x, y", if c.x.is_finite() { c.y } else { c.x });
    }
    if !(b.width > 0.0) || !b.width.is_finite() {
        return bad("width", b.width);
    }
    if !(b.depth > 0.0) || !b.depth.is_finite() {
        return bad("depth", b.depth);
    }
    if !(b.height >= 0.0) || !b.height.is_finite() {
        return bad("height", b.height);
    }
    Ok(())
}

/// The horizontal unit world direction that `projection` maps onto scene +y (toward the
/// viewer). The projection must be valid.
pub fn toward_viewer(projection: &Projection) -> Vec2 {
    let (x, y) = (projection.x_axis, projection.y_axis);
    let det = x.x * y.y - x.y * y.x;
    let (nx, ny) = (-y.x / det, x.x / det);
    let l = (nx * nx + ny * ny).sqrt();
    if l > 0.0 {
        Vec2::new(nx / l, ny / l)
    } else {
        Vec2::new(nx, ny)
    }
}

/// Triangles for the surface layer (see the [module docs](self)). Keeps its capacity across
/// [`clear`](Self::clear), so a warm geometry appends without allocating.
#[derive(Clone, Debug, Default)]
pub struct SurfaceGeometry {
    vertices: Vec<SurfaceVertex>,
}

/// A point of the world (`x`, `y` on the ground, `z` the height) in layer pixels.
fn at(p: &Projection, x: f32, y: f32, z: f32) -> At<Screen, Vec2> {
    p.project(At::new(Vec2::new(x, y)), z)
}

impl SurfaceGeometry {
    /// An empty geometry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes every triangle, keeping the capacity.
    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    /// The triangle list: three vertices per triangle, in the order they were appended
    /// (painter's order: later triangles cover earlier ones).
    pub fn vertices(&self) -> &[SurfaceVertex] {
        &self.vertices
    }

    /// Appends one triangle as given, for geometry of the host's own.
    pub fn push_triangle(&mut self, triangle: [SurfaceVertex; 3]) {
        self.vertices.extend_from_slice(&triangle);
    }

    fn push_quad(&mut self, pts: [At<Screen, Vec2>; 4], pixels: [[u8; 4]; 4]) {
        let v = |i: usize| SurfaceVertex {
            position: pts[i],
            pixel: pixels[i],
        };
        self.vertices
            .extend_from_slice(&[v(0), v(1), v(2), v(0), v(2), v(3)]);
    }

    /// Appends the sides of each box that face the viewer (scene +y) and its roof, in slice
    /// order, with the exact height and normal per pixel. Every box is checked first.
    ///
    /// # Errors
    /// An invalid projection or height step, or a box out of range (named by its index).
    pub fn push_boxes(
        &mut self,
        projection: &Projection,
        height_step: f32,
        boxes: &[SurfaceBox],
    ) -> Result<(), SurfaceGeometryError> {
        check_view(projection, height_step)?;
        for (i, b) in boxes.iter().enumerate() {
            validate_surface_box(b, i)?;
        }
        for b in boxes {
            self.push_valid_box(projection, height_step, b);
        }
        Ok(())
    }

    fn push_valid_box(&mut self, p: &Projection, step: f32, b: &SurfaceBox) {
        let c = b.center.raw();
        let (x0, x1) = (c.x - b.width * 0.5, c.x + b.width * 0.5);
        let (y0, y1) = (c.y - b.depth * 0.5, c.y + b.depth * 0.5);
        let top = b.height;
        let cx = [x0, x1, x1, x0];
        let cy = [y0, y0, y1, y1];
        let v = toward_viewer(p);
        let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        for i in 0..4 {
            let j = (i + 1) % 4;
            let (mut nx, mut ny) = (cy[j] - cy[i], cx[i] - cx[j]);
            let l = (nx * nx + ny * ny).sqrt();
            (nx, ny) = (nx / l, ny / l);
            let (ex, ey) = ((cx[i] + cx[j]) * 0.5, (cy[i] + cy[j]) * 0.5);
            if nx * (ex - mx) + ny * (ey - my) < 0.0 {
                (nx, ny) = (-nx, -ny);
            }
            if nx * v.x + ny * v.y <= 0.0 {
                continue; // a back side: hidden behind the box
            }
            let pts = [
                at(p, cx[i], cy[i], 0.0),
                at(p, cx[j], cy[j], 0.0),
                at(p, cx[j], cy[j], top),
                at(p, cx[i], cy[i], top),
            ];
            let n = Vec3::new(nx, ny, 0.0);
            let (lo, hi) = (surface_pixel(0.0, step, n), surface_pixel(top, step, n));
            self.push_quad(pts, [lo, lo, hi, hi]);
        }
        let pts = core::array::from_fn(|i| at(p, cx[i], cy[i], top));
        let roof = surface_pixel(top, step, Vec3::new(0.0, 0.0, 1.0));
        self.push_quad(pts, [roof; 4]);
    }

    /// Appends a vertical wall on the foot line `a`-`b` (x, y world units, z the foot height at
    /// each end) up to `top`, its normal horizontal and perpendicular to `a`-`b` on the
    /// viewer's side. `a`-`b` and `b`-`a` give the same triangles.
    ///
    /// # Errors
    /// An invalid projection or height step, ends not finite or below the ground, `top` below
    /// an end, or equal ends.
    pub fn push_wall(
        &mut self,
        projection: &Projection,
        height_step: f32,
        a: Vec3,
        b: Vec3,
        top: f32,
    ) -> Result<(), SurfaceGeometryError> {
        check_view(projection, height_step)?;
        let ground = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite() && v.z >= 0.0;
        if !ground(a) || !ground(b) {
            return Err(SurfaceGeometryError::WallEnds);
        }
        if !top.is_finite() || top < a.z || top < b.z {
            return Err(SurfaceGeometryError::WallTop(top));
        }
        // A canonical order, so a-b and b-a give the same triangles.
        let (a, b) = if b.x < a.x || (b.x == a.x && b.y < a.y) {
            (b, a)
        } else {
            (a, b)
        };
        let (nx, ny) = (b.y - a.y, a.x - b.x);
        let l = (nx * nx + ny * ny).sqrt();
        if !(l > 0.0) {
            return Err(SurfaceGeometryError::WallLength);
        }
        let mut n = Vec3::new(nx / l, ny / l, 0.0);
        let v = toward_viewer(projection);
        if n.x * v.x + n.y * v.y < 0.0 {
            n = Vec3::new(-n.x, -n.y, 0.0);
        }
        let p = projection;
        let pts = [
            at(p, a.x, a.y, a.z),
            at(p, b.x, b.y, b.z),
            at(p, b.x, b.y, top),
            at(p, a.x, a.y, top),
        ];
        let hi = surface_pixel(top, height_step, n);
        let pixels = [
            surface_pixel(a.z, height_step, n),
            surface_pixel(b.z, height_step, n),
            hi,
            hi,
        ];
        self.push_quad(pts, pixels);
        Ok(())
    }

    /// Appends a flat convex polygon at `height`, normal up, as a fan from its first point.
    ///
    /// # Errors
    /// An invalid projection or height step, fewer than three points, a point not finite, or a
    /// height not finite and >= 0.
    pub fn push_floor(
        &mut self,
        projection: &Projection,
        height_step: f32,
        points: &[At<World2, Vec2>],
        height: f32,
    ) -> Result<(), SurfaceGeometryError> {
        check_view(projection, height_step)?;
        if points.len() < 3 {
            return Err(SurfaceGeometryError::FloorCount(points.len()));
        }
        if !(height >= 0.0) || !height.is_finite() {
            return Err(SurfaceGeometryError::FloorHeight(height));
        }
        if let Some(i) = points
            .iter()
            .position(|q| !q.raw().x.is_finite() || !q.raw().y.is_finite())
        {
            return Err(SurfaceGeometryError::FloorPoint(i));
        }
        let pixel = surface_pixel(height, height_step, Vec3::new(0.0, 0.0, 1.0));
        let v = |q: &At<World2, Vec2>| SurfaceVertex {
            position: projection.project(*q, height),
            pixel,
        };
        for pair in points[1..].windows(2) {
            self.vertices
                .extend_from_slice(&[v(&points[0]), v(&pair[0]), v(&pair[1])]);
        }
        Ok(())
    }

    /// Appends any planar quad (world units, z = height >= 0). The normal follows the winding:
    /// corners counter-clockwise seen from the side the normal points to.
    ///
    /// # Errors
    /// An invalid projection or height step, a corner not finite or below the ground, corners
    /// without area, or corners more than 1 % of the quad size off their plane.
    pub fn push_quad_corners(
        &mut self,
        projection: &Projection,
        height_step: f32,
        corners: &[Vec3; 4],
    ) -> Result<(), SurfaceGeometryError> {
        check_view(projection, height_step)?;
        let c = corners;
        let mut size = 0.0f32;
        for i in 0..4 {
            let u = c[i];
            if !(u.x.is_finite() && u.y.is_finite() && u.z.is_finite() && u.z >= 0.0) {
                return Err(SurfaceGeometryError::QuadCorner(i));
            }
            let d = c[(i + 1) % 4];
            let (dx, dy, dz) = (d.x - u.x, d.y - u.y, d.z - u.z);
            size = det::max(size, (dx * dx + dy * dy + dz * dz).sqrt());
        }
        // Newell normal: robust for any planar polygon, follows the winding.
        let mut n = [0.0f32; 3];
        for i in 0..4 {
            let (u, v) = (c[i], c[(i + 1) % 4]);
            n[0] += (u.y - v.y) * (u.z + v.z);
            n[1] += (u.z - v.z) * (u.x + v.x);
            n[2] += (u.x - v.x) * (u.y + v.y);
        }
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if !(l > 0.0) {
            return Err(SurfaceGeometryError::QuadArea);
        }
        let n = Vec3::new(n[0] / l, n[1] / l, n[2] / l);
        let mid = |f: fn(&Vec3) -> f32| (f(&c[0]) + f(&c[1]) + f(&c[2]) + f(&c[3])) * 0.25;
        let (cx, cy, cz) = (mid(|v| v.x), mid(|v| v.y), mid(|v| v.z));
        for (corner, u) in c.iter().enumerate() {
            let off = (u.x - cx) * n.x + (u.y - cy) * n.y + (u.z - cz) * n.z;
            if off.abs() > 0.01 * size {
                return Err(SurfaceGeometryError::QuadNotPlanar { corner, off });
            }
        }
        let pts = c.map(|u| at(projection, u.x, u.y, u.z));
        let pixels = c.map(|u| surface_pixel(u.z, height_step, n));
        self.push_quad(pts, pixels);
        Ok(())
    }
}

/// Kinds of [`SurfaceFace`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceKind {
    /// Flat at `height`, normal up: floors, roofs.
    Flat,
    /// Vertical, facing `azimuth`; height measured from the foot line.
    Facade,
    /// Vertical, facing the viewer ([`toward_viewer`]): figures, posts; height measured from
    /// the foot line.
    Standing,
}

/// One face of a sprite for [`encode_surface_sprite`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceFace {
    /// Kind of face.
    pub kind: FaceKind,
    /// [`FaceKind::Flat`]: world units above the ground, >= 0.
    pub height: f32,
    /// [`FaceKind::Facade`]: degrees counter-clockwise from world +x of the direction the
    /// facade faces (its normal), finite.
    pub azimuth: f32,
}

impl SurfaceFace {
    /// A face of `kind` at height 0, azimuth 0.
    pub const fn new(kind: FaceKind) -> Self {
        Self {
            kind,
            height: 0.0,
            azimuth: 0.0,
        }
    }
}

/// The alpha shape of a sprite and the line where its vertical faces meet the ground, in
/// sprite pixels.
#[derive(Clone, Copy, Debug)]
pub struct SpriteShape<'a> {
    /// RGBA pixels (any order with A last); alpha below 128 is outside the sprite.
    pub rgba: &'a [u8],
    /// Pixels, 1..=16384.
    pub width: u32,
    /// Pixels, 1..=16384.
    pub height: u32,
    /// Bytes per row, >= `width * 4`.
    pub pitch: usize,
    /// Start and end of the foot line; vertical faces rise from it along the projection's
    /// `-z_axis`.
    pub foot: [Vec2; 2],
}

/// Height of sprite pixel (x, y) above the foot line, measured along `-z_axis`, in world units
/// (>= 0).
fn height_above_foot(s: &SpriteShape<'_>, p: &Projection, x: u32, y: u32) -> f32 {
    let z = p.z_axis;
    let zl = (z.x * z.x + z.y * z.y).sqrt();
    if zl <= 0.0 {
        return 0.0;
    }
    // Solve pixel + t * down = foot0 + u * (foot1 - foot0), down = -z_axis / |z_axis|.
    let (dx, dy) = (-z.x / zl, -z.y / zl);
    let [f0, f1] = s.foot;
    let (lx, ly) = (f1.x - f0.x, f1.y - f0.y);
    let (px, py) = (x as f32 + 0.5 - f0.x, y as f32 + 0.5 - f0.y);
    let d = dx * -ly - dy * -lx;
    if d.abs() < 1e-6 {
        return 0.0;
    }
    let t = (-px * -ly - -py * -lx) / d; // Cramer for t * d - u * l = -p
    det::max(0.0, t) / zl
}

/// Encodes the surface map of a sprite ([`surface_pixel`] per pixel) from its alpha shape,
/// once at load time: a building with two facades and a roof, a standing figure. The sprite is
/// assumed to map 1:1 to scene pixels under `projection` (its `z_axis` turns pixel offsets into
/// heights); encode again after a zoom change.
///
/// `face_ids`: `None` makes every pixel inside the shape `faces[0]`; otherwise `width * height`
/// bytes, row-major, the index into `faces` per pixel, 255 for no entry. `out` receives
/// `width * height * 4` tightly packed bytes; pixels outside the shape or with id 255 get
/// `[0, 0, 0, 0]`.
///
/// # Errors
/// A bad shape, face, face id, projection or height step, or a buffer too short; nothing is
/// written then.
pub fn encode_surface_sprite(
    shape: &SpriteShape<'_>,
    faces: &[SurfaceFace],
    face_ids: Option<&[u8]>,
    projection: &Projection,
    height_step: f32,
    out: &mut [u8],
) -> Result<(), SurfaceGeometryError> {
    let s = shape;
    let (w, h) = (s.width as usize, s.height as usize);
    if !(1..=16384).contains(&s.width) || !(1..=16384).contains(&s.height) || s.pitch < w * 4 {
        return Err(SurfaceGeometryError::ShapeSize {
            width: s.width,
            height: s.height,
            pitch: s.pitch,
        });
    }
    let too_small = |what, needed: usize, got: usize| {
        (got < needed).then_some(SurfaceGeometryError::BufferTooSmall { what, needed, got })
    };
    if let Some(e) = too_small("rgba", s.pitch * (h - 1) + w * 4, s.rgba.len())
        .or_else(|| face_ids.and_then(|ids| too_small("face_ids", w * h, ids.len())))
        .or_else(|| too_small("out", w * h * 4, out.len()))
    {
        return Err(e);
    }
    if !s.foot.iter().all(|f| f.x.is_finite() && f.y.is_finite()) {
        return Err(SurfaceGeometryError::FootLine);
    }
    if faces.is_empty() {
        return Err(SurfaceGeometryError::NoFaces);
    }
    if let Some(index) = faces
        .iter()
        .position(|f| !(f.height >= 0.0) || !f.height.is_finite() || !f.azimuth.is_finite())
    {
        let f = faces[index];
        return Err(SurfaceGeometryError::Face {
            index,
            height: f.height,
            azimuth: f.azimuth,
        });
    }
    check_view(projection, height_step)?;
    let inside = |x: usize, y: usize| s.rgba[y * s.pitch + x * 4 + 3] >= 128;
    let id = |x: usize, y: usize| face_ids.map_or(0, |ids| ids[y * w + x]);
    for y in 0..h {
        for x in 0..w {
            let i = id(x, y);
            if inside(x, y) && i != 255 && usize::from(i) >= faces.len() {
                return Err(SurfaceGeometryError::FaceId {
                    id: i,
                    x: x as u32,
                    y: y as u32,
                    faces: faces.len(),
                });
            }
        }
    }
    let v = toward_viewer(projection);
    for y in 0..h {
        for x in 0..w {
            let dst = &mut out[(y * w + x) * 4..][..4];
            let i = id(x, y);
            if !inside(x, y) || i == 255 {
                dst.fill(0);
                continue;
            }
            let f = faces[usize::from(i)];
            let (height, normal) = match f.kind {
                FaceKind::Flat => (f.height, Vec3::new(0.0, 0.0, 1.0)),
                FaceKind::Facade => {
                    let (sin, cos) = det::sin_cos(f.azimuth / 57.295_78);
                    (
                        height_above_foot(s, projection, x as u32, y as u32),
                        Vec3::new(cos, sin, 0.0),
                    )
                }
                FaceKind::Standing => (
                    height_above_foot(s, projection, x as u32, y as u32),
                    Vec3::new(v.x, v.y, 0.0),
                ),
            };
            dst.copy_from_slice(&surface_pixel(height, height_step, normal));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode_normal;

    fn iso() -> Projection {
        // chiaro::IsoProjection(0, 0, 2, 1, 64, 64): z_axis (0, -2).
        Projection::iso(At::new(Vec2::ZERO), 2.0, 1.0, 64, 64)
    }

    /// Height and normal of an encoded pixel, `None` outside the shape.
    fn decode(out: &[u8], w: u32, x: u32, y: u32, step: f32) -> Option<(f32, Vec3)> {
        let p = &out[((y * w + x) * 4) as usize..][..4];
        (p[3] >= 128).then(|| (f32::from(p[0]) * step, decode_normal(p[1], p[2])))
    }

    fn azimuth(n: Vec3) -> f32 {
        det::atan2(n.y, n.x).to_degrees()
    }

    fn opaque(w: u32, h: u32) -> Vec<u8> {
        vec![255; (w * h * 4) as usize]
    }

    fn shape(rgba: &[u8], w: u32, h: u32, foot_y: f32) -> SpriteShape<'_> {
        SpriteShape {
            rgba,
            width: w,
            height: h,
            pitch: (w * 4) as usize,
            foot: [Vec2::new(0.0, foot_y), Vec2::new(w as f32, foot_y)],
        }
    }

    fn encode(
        s: &SpriteShape<'_>,
        faces: &[SurfaceFace],
        ids: Option<&[u8]>,
        step: f32,
    ) -> Vec<u8> {
        let mut out = vec![0; (s.width * s.height * 4) as usize];
        encode_surface_sprite(s, faces, ids, &iso(), step, &mut out).unwrap();
        out
    }

    #[test]
    fn a_facade_grows_in_height_with_the_distance_to_its_foot_line() {
        let mut rgba = opaque(8, 16);
        rgba[(4 * 8 + 3) * 4 + 3] = 0;
        let step = 0.25;
        let facade = SurfaceFace {
            azimuth: 45.0,
            ..SurfaceFace::new(FaceKind::Facade)
        };
        let out = encode(&shape(&rgba, 8, 16, 15.0), &[facade], None, step);
        // Pixel centre y + 0.5 lies (15 - y - 0.5) scene pixels above the foot line, 2 per unit.
        let (h, n) = decode(&out, 8, 2, 7, step).unwrap();
        assert!((h - 3.75).abs() <= 0.2, "{h}");
        assert!((decode(&out, 8, 2, 0, step).unwrap().0 - 7.25).abs() <= 0.4);
        assert_eq!(decode(&out, 8, 2, 15, step).unwrap().0, 0.0); // below the line
        assert!((azimuth(n) - 45.0).abs() <= 1.5);
        assert!(n.z.abs() <= 0.01);
        assert!(decode(&out, 8, 3, 4, step).is_none()); // transparent sprite pixel
    }

    #[test]
    fn floor_and_roof_faces_have_a_constant_height_and_point_up() {
        let rgba = opaque(4, 4);
        let s = shape(&rgba, 4, 4, 3.0);
        let roof = SurfaceFace {
            height: 12.0,
            ..SurfaceFace::new(FaceKind::Flat)
        };
        let (h, n) = decode(&encode(&s, &[roof], None, 0.5), 4, 1, 1, 0.5).unwrap();
        assert_eq!(h, 12.0);
        assert_eq!(n.z, 1.0);
        let ground = SurfaceFace::new(FaceKind::Flat);
        assert_eq!(
            decode(&encode(&s, &[ground], None, 0.5), 4, 0, 0, 0.5)
                .unwrap()
                .0,
            0.0
        );
    }

    #[test]
    fn a_standing_object_faces_the_camera() {
        let rgba = opaque(2, 8);
        let out = encode(
            &shape(&rgba, 2, 8, 7.0),
            &[SurfaceFace::new(FaceKind::Standing)],
            None,
            0.25,
        );
        let (h, n) = decode(&out, 2, 0, 2, 0.25).unwrap();
        // Scene +y (toward the viewer) is world (1, 1) / sqrt(2) in this view.
        assert!((azimuth(n) - 45.0).abs() <= 1.5);
        assert!((h - 2.25).abs() <= 0.12, "{h}"); // (7 - 2.5) / 2
    }

    #[test]
    fn a_face_id_template_builds_a_whole_building() {
        let rgba = opaque(4, 4);
        #[rustfmt::skip]
        let ids = [2, 2, 2, 2,
                   0, 0, 1, 1,
                   0, 0, 1, 1,
                   0, 255, 1, 1];
        let facade = |azimuth| SurfaceFace {
            azimuth,
            ..SurfaceFace::new(FaceKind::Facade)
        };
        let roof = SurfaceFace {
            height: 6.0,
            ..SurfaceFace::new(FaceKind::Flat)
        };
        let faces = [facade(225.0), facade(315.0), roof];
        let s = shape(&rgba, 4, 4, 3.5);
        let out = encode(&s, &faces, Some(&ids), 0.5);
        assert_eq!(decode(&out, 4, 0, 0, 0.5).unwrap().0, 6.0);
        assert!((azimuth(decode(&out, 4, 0, 2, 0.5).unwrap().1) + 135.0).abs() <= 1.5);
        assert!((azimuth(decode(&out, 4, 3, 2, 0.5).unwrap().1) + 45.0).abs() <= 1.5);
        assert!(decode(&out, 4, 1, 3, 0.5).is_none()); // face id 255
        let mut out = vec![7; 64];
        let e = encode_surface_sprite(&s, &faces[..2], Some(&ids), &iso(), 0.5, &mut out);
        assert!(matches!(e, Err(SurfaceGeometryError::FaceId { id: 2, .. })));
        assert!(out.iter().all(|b| *b == 7), "nothing written on error");
    }

    #[test]
    fn a_facade_facing_azimuth_90_degrees_decodes_to_plus_y() {
        let rgba = opaque(4, 8);
        let facade = SurfaceFace {
            azimuth: 90.0,
            ..SurfaceFace::new(FaceKind::Facade)
        };
        let out = encode(&shape(&rgba, 4, 8, 7.0), &[facade], None, 0.25);
        let n = decode(&out, 4, 1, 3, 0.25).unwrap().1;
        assert!(det::acos(det::min(n.y, 1.0)).to_degrees() <= 1.0);
    }

    #[test]
    fn surface_sprite_errors_name_the_bad_input() {
        let rgba = opaque(4, 4);
        let roof = [SurfaceFace {
            height: 1.0,
            ..SurfaceFace::new(FaceKind::Flat)
        }];
        let mut out = vec![0; 64];
        let p = Projection::iso(At::new(Vec2::ZERO), 1.0, 1.0, 64, 64);
        let mut message = |s: &SpriteShape<'_>, ids: Option<&[u8]>, step: f32| {
            let e = encode_surface_sprite(s, &roof, ids, &p, step, &mut out).unwrap_err();
            let d = e.to_diagnostic();
            (d.code.as_str().to_owned(), d.message)
        };
        let narrow = SpriteShape {
            pitch: 8,
            ..shape(&rgba, 4, 4, 3.0)
        };
        let (code, m) = message(&narrow, None, 1.0);
        assert_eq!(code, "knorke.light2d.surface_geometry.shape");
        assert!(m.contains("pitch 8"), "{m}");
        let s = shape(&rgba, 4, 4, 3.0);
        let (code, m) = message(&s, None, 0.0);
        assert_eq!(code, "knorke.light2d.surface_geometry.height_step");
        assert!(m.contains("height_step"), "{m}");
        let mut ids = [0u8; 16];
        ids[5] = 3;
        let (code, m) = message(&s, Some(&ids), 1.0);
        assert_eq!(code, "knorke.light2d.surface_geometry.face_id");
        assert!(m.contains("index 3 at (1, 1)"), "{m}");
        let (code, m) = message(&s, Some(&ids[..15]), 1.0);
        assert_eq!(code, "knorke.light2d.surface_geometry.buffer");
        assert!(m.contains("face_ids"), "{m}");
    }

    // ---- geometry ----------------------------------------------------------------------------

    /// An oblique iso-like view: x right-down, y left-down, height up (SurfaceHelperTests.cpp).
    fn oblique() -> Projection {
        Projection {
            origin: At::new(Vec2::new(24.0, 12.0)),
            x_axis: Vec2::new(1.5, 0.75),
            y_axis: Vec2::new(-1.5, 0.75),
            z_axis: Vec2::new(0.0, -1.5),
        }
    }

    const STEP: f32 = 0.1;

    fn boxed(x: f32, y: f32, width: f32, depth: f32, height: f32) -> SurfaceBox {
        SurfaceBox {
            center: At::new(Vec2::new(x, y)),
            width,
            depth,
            height,
        }
    }

    #[test]
    fn a_box_gives_its_viewer_facing_sides_and_its_roof() {
        let mut g = SurfaceGeometry::new();
        g.push_boxes(&oblique(), STEP, &[boxed(0.0, 0.0, 6.0, 4.0, 5.0)])
            .unwrap();
        // The oblique view looks along -y and -x: the +x and +y sides and the roof.
        let v = g.vertices();
        assert_eq!(v.len(), 3 * 6);
        let roof = surface_pixel(5.0, STEP, Vec3::new(0.0, 0.0, 1.0));
        assert!(v[12..].iter().all(|v| v.pixel == roof));
        assert_eq!(v[0].pixel[0], 0);
        assert_eq!(v[2].pixel[0], 50);
        // A single box and a batch of it append the same.
        let mut single = SurfaceGeometry::new();
        let b = [
            boxed(0.0, 0.0, 6.0, 4.0, 5.0),
            boxed(8.0, -3.0, 3.0, 3.0, 2.5),
        ];
        for one in &b {
            single
                .push_boxes(&oblique(), STEP, core::slice::from_ref(one))
                .unwrap();
        }
        g.clear();
        g.push_boxes(&oblique(), STEP, &b).unwrap();
        assert_eq!(g.vertices(), single.vertices());
    }

    #[test]
    fn a_wall_faces_the_viewer_whatever_the_order_of_its_ends() {
        let (a, b) = (Vec3::new(-4.0, 2.0, 0.0), Vec3::new(4.0, 2.0, 0.0));
        let (mut ab, mut ba) = (SurfaceGeometry::new(), SurfaceGeometry::new());
        ab.push_wall(&oblique(), STEP, a, b, 6.0).unwrap();
        ba.push_wall(&oblique(), STEP, b, a, 6.0).unwrap();
        assert_eq!(ab.vertices(), ba.vertices());
        let n = decode_normal(ab.vertices()[0].pixel[1], ab.vertices()[0].pixel[2]);
        let v = toward_viewer(&oblique());
        assert!(n.x * v.x + n.y * v.y > 0.5);
        assert!(n.y > 0.99);
    }

    #[test]
    fn floor_and_quad_of_the_same_square_are_the_same_pixels() {
        let xy = [(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)];
        let points = xy.map(|(x, y)| At::new(Vec2::new(x, y)));
        let mut floor = SurfaceGeometry::new();
        floor.push_floor(&oblique(), STEP, &points, 1.5).unwrap();
        let mut quad = SurfaceGeometry::new();
        let corners = xy.map(|(x, y)| Vec3::new(x, y, 1.5));
        quad.push_quad_corners(&oblique(), STEP, &corners).unwrap();
        // Both are the fan 0 1 2, 0 2 3 with the up pixel.
        assert_eq!(floor.vertices(), quad.vertices());
        let up = surface_pixel(1.5, STEP, Vec3::new(0.0, 0.0, 1.0));
        assert!(floor.vertices().iter().all(|v| v.pixel == up));
        // A slope: height rises along x, the normal tilts toward -x.
        let slope = [
            (-3.0, -3.0, 0.0),
            (3.0, -3.0, 3.0),
            (3.0, 3.0, 3.0),
            (-3.0, 3.0, 0.0),
        ];
        quad.clear();
        quad.push_quad_corners(&oblique(), STEP, &slope.map(|(x, y, z)| Vec3::new(x, y, z)))
            .unwrap();
        let p = quad.vertices()[0].pixel;
        let n = decode_normal(p[1], p[2]);
        assert!(
            (n.x + 0.447).abs() < 0.03 && (n.z - 0.894).abs() < 0.03,
            "{n:?}"
        );
    }

    #[test]
    fn geometry_errors_name_the_input_and_append_nothing() {
        let p = oblique();
        let mut g = SurfaceGeometry::new();
        let code =
            |e: SurfaceGeometryError| (e.to_diagnostic().code.as_str().to_owned(), e.to_string());
        let (c, m) = code(
            g.push_boxes(
                &p,
                STEP,
                &[
                    boxed(0.0, 0.0, 6.0, 4.0, 5.0),
                    boxed(0.0, 0.0, 0.0, 1.0, 1.0),
                ],
            )
            .unwrap_err(),
        );
        assert_eq!(c, "knorke.light2d.surface_geometry.box");
        assert!(m.contains("boxes[1].width"), "{m}");
        let a = Vec3::new(-4.0, 2.0, 0.0);
        let (c, _) = code(g.push_wall(&p, STEP, a, a, 6.0).unwrap_err());
        assert_eq!(c, "knorke.light2d.surface_geometry.wall");
        let (_, m) = code(
            g.push_wall(&p, STEP, a, Vec3::new(4.0, 2.0, 0.0), -1.0)
                .unwrap_err(),
        );
        assert!(m.contains("top"), "{m}");
        let bent = [
            (-3.0, -3.0, 0.0),
            (3.0, -3.0, 0.0),
            (3.0, 3.0, 2.0),
            (-3.0, 3.0, 0.0),
        ];
        let (c, m) = code(
            g.push_quad_corners(&p, STEP, &bent.map(|(x, y, z)| Vec3::new(x, y, z)))
                .unwrap_err(),
        );
        assert_eq!(c, "knorke.light2d.surface_geometry.quad");
        assert!(m.contains("planar"), "{m}");
        let two = [At::new(Vec2::ZERO), At::new(Vec2::X)];
        let (c, m) = code(g.push_floor(&p, STEP, &two, 1.0).unwrap_err());
        assert_eq!(c, "knorke.light2d.surface_geometry.floor");
        assert!(m.contains("count"), "{m}");
        let collinear = Projection {
            y_axis: Vec2::new(2.0, 1.0),
            x_axis: Vec2::new(4.0, 2.0),
            ..p
        };
        let (c, _) = code(g.push_floor(&collinear, STEP, &two, 1.0).unwrap_err());
        assert_eq!(c, "knorke.light2d.projection.collinear_axes");
        let (c, _) = code(g.push_boxes(&p, f32::NAN, &[]).unwrap_err());
        assert_eq!(c, "knorke.light2d.surface_geometry.height_step");
        assert!(g.vertices().is_empty());
    }
}
