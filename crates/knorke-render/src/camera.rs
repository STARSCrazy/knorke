//! Camera: position, yaw (turn), pitch (look up/down), vertical field of view.
//!
//! World axes: x east, y up, z south. Yaw 0 looks north (-z), positive yaw turns right (clockwise seen
//! from above). Positive pitch looks up.

use crate::math::{Vec3, Vec3Ext};

#[derive(Clone, Copy, Debug)]
/// A camera: position, turn, tilt, field of view, lens shift.
pub struct Camera {
    /// Eye position in world space.
    pub position: Vec3,
    /// Turn in radians (0 looks north, -z; positive turns right).
    pub yaw: f32,
    /// Tilt in radians (positive looks up).
    pub pitch: f32,
    /// Vertical field of view in degrees.
    pub fov_y: f32,
    /// Near clip distance in world units.
    pub near: f32,
    /// Lens shift: moves the projection centre (the point straight ahead) by this fraction of the image
    /// width to the right (`shift_x`) and of the image height upwards (`shift_y`). 0 = centre; +0.25 puts
    /// the horizon of a level camera a quarter of the image height above the centre. The view direction
    /// and field of view stay; the image is a crop of a larger one.
    pub shift_x: f32,
    /// Lens shift upwards as a fraction of the image height (see `shift_x`).
    pub shift_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 60.0,
            near: 0.05,
            shift_x: 0.0,
            shift_y: 0.0,
        }
    }
}

/// Camera basis and projection for one frame.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Eye position (snapped with `Options::snap`).
    pub eye: Vec3,
    /// View direction (unit).
    pub forward: Vec3,
    /// Right vector (unit).
    pub right: Vec3,
    /// Up vector (unit).
    pub up: Vec3,
    /// Horizontal right vector (yaw only), used for billboards.
    pub flat_right: Vec3,
    /// Near clip distance.
    pub near: f32,
    /// Pixels per unit of x/z at distance 1.
    pub focal: f32,
    /// Projection centre in pixels (image centre unless the lens is shifted).
    pub cx: f32,
    /// Projection centre y in pixels.
    pub cy: f32,
    /// Frustum side slopes (left, right, top, bottom) plus one pixel of margin, for clipping.
    pub tan_l: f32,
    /// Right side slope.
    pub tan_r: f32,
    /// Top side slope.
    pub tan_t: f32,
    /// Bottom side slope.
    pub tan_b: f32,
}

impl View {
    /// The view of `cam` for a `width x height` frame; `snap` > 0 snaps the eye to that world step.
    pub fn new(cam: &Camera, width: usize, height: usize, snap: f32) -> View {
        let (sy, cy) = cam.yaw.sin_cos();
        let (sp, cp) = cam.pitch.clamp(-1.55, 1.55).sin_cos();
        let forward = Vec3::new(sy * cp, sp, -cy * cp);
        let flat_right = Vec3::new(cy, 0.0, sy);
        let right = flat_right;
        let up = right.cross(forward);

        let mut eye = cam.position;
        if snap > 0.0 {
            // Snap the eye to a grid aligned with the horizontal view axes, so that a camera that slides
            // sideways or forward moves in whole steps and the pixels do not shimmer.
            let flat_fwd = Vec3::new(sy, 0.0, -cy);
            let a = (eye.dot(flat_right) / snap).round() * snap;
            let b = (eye.dot(flat_fwd) / snap).round() * snap;
            let c = (eye.y / snap).round() * snap;
            eye = flat_right * a + flat_fwd * b + Vec3::new(0.0, c, 0.0);
        }

        let (w, h) = (width as f32, height as f32);
        let half_h = h * 0.5;
        let fov = cam.fov_y.clamp(1.0, 170.0).to_radians();
        let focal = half_h / (fov * 0.5).tan();
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let cx = w * 0.5 + finite(cam.shift_x).clamp(-4.0, 4.0) * w;
        let cy = half_h - finite(cam.shift_y).clamp(-4.0, 4.0) * h;
        // One extra pixel of margin keeps clipped edges outside the visible area.
        let tan_l = (cx + 1.0) / focal;
        let tan_r = (w - cx + 1.0) / focal;
        let tan_t = (cy + 1.0) / focal;
        let tan_b = (h - cy + 1.0) / focal;
        View {
            eye,
            forward,
            right,
            up,
            flat_right,
            near: cam.near.max(1e-4),
            focal,
            cx,
            cy,
            tan_l,
            tan_r,
            tan_t,
            tan_b,
        }
    }

    /// World point to view space (x right, y up, z depth along the view direction).
    pub fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.forward))
    }

    /// World direction of the ray through the center of pixel (x, y).
    pub fn ray(&self, x: usize, y: usize) -> Vec3 {
        let sx = (x as f32 + 0.5 - self.cx) / self.focal;
        let sy = (self.cy - (y as f32 + 0.5)) / self.focal;
        (self.forward + self.right * sx + self.up * sy).normalized()
    }
}
