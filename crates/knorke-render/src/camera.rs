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

impl Camera {
    /// Where `p` lies seen from above: (depth ahead along the turn, offset to the right). Pitch is
    /// ignored, so a tilting camera does not change which upright things (sprites, wall pictures)
    /// count as "too near" or "beside the view"; the host picks the limits.
    pub fn ground_offset(&self, p: Vec3) -> (f32, f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let (dx, dz) = (p.x - self.position.x, p.z - self.position.z);
        (dx * sy - dz * cy, dx * cy + dz * sy)
    }

    /// The camera between `from` (t = 0) and `to` (t = 1), eased out (fast start, soft stop), for a
    /// step or a turn shown over a few frames. Yaw takes the short way round; field of view, near
    /// plane and lens shift come from `to`. The host owns the frame count and the timing.
    pub fn glide(from: &Camera, to: &Camera, t: f32) -> Camera {
        use core::f32::consts::{PI, TAU};
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let k = 1.0 - (1.0 - t) * (1.0 - t);
        let mut turn = (to.yaw - from.yaw).rem_euclid(TAU);
        if turn > PI {
            turn -= TAU;
        }
        Camera {
            position: from.position + (to.position - from.position) * k,
            yaw: from.yaw + turn * k,
            pitch: from.pitch + (to.pitch - from.pitch) * k,
            ..*to
        }
    }
}

/// Fades upright things (sprites, wall pictures) out near the camera, so nothing stands hugely
/// enlarged in the view. Hidden closer than `near` (depth ahead, seen from above), fully shown from
/// `near + fade` on, linear in between. With `flank_slope` > 0 things closer than `flank_depth`
/// are also hidden when they lie further to the side than `depth * flank_slope` (a wide lens that
/// sees beside the camera). `Default` hides nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NearFade {
    /// Depth below which a thing is hidden.
    pub near: f32,
    /// Depth over which it fades in after `near`; 0 = hard cut.
    pub fade: f32,
    /// Side limit as side offset per depth; 0 = off.
    pub flank_slope: f32,
    /// The side limit applies only closer than this depth.
    pub flank_depth: f32,
}

impl NearFade {
    /// Opacity 0..1 of a thing at `p` seen from `cam`; the host multiplies it into the alpha or
    /// leaves the thing out at 0.
    pub fn alpha(&self, cam: &Camera, p: Vec3) -> f32 {
        let (depth, side) = cam.ground_offset(p);
        if self.flank_slope > 0.0
            && depth < self.flank_depth
            && side.abs() > depth * self.flank_slope
        {
            return 0.0;
        }
        if self.fade > 0.0 {
            ((depth - self.near) / self.fade).clamp(0.0, 1.0)
        } else if depth < self.near {
            0.0
        } else {
            1.0
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::{FRAC_PI_2, PI};

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn ground_offset_follows_the_turn() {
        // Yaw 0 looks north (-z): a point to the north is ahead, one to the east is on the right.
        let cam = Camera::default();
        let (d, l) = cam.ground_offset(Vec3::new(0.0, 5.0, -2.0));
        assert!(near(d, 2.0) && near(l, 0.0));
        let (d, l) = cam.ground_offset(Vec3::new(3.0, 0.0, 0.0));
        assert!(near(d, 0.0) && near(l, 3.0));
        // Turned right to face east: east is ahead, south on the right; pitch changes nothing.
        let cam = Camera {
            yaw: FRAC_PI_2,
            pitch: 0.7,
            ..cam
        };
        let (d, l) = cam.ground_offset(Vec3::new(3.0, 0.0, 1.0));
        assert!(near(d, 3.0) && near(l, 1.0));
    }

    #[test]
    fn near_fade_hides_close_and_flank_and_fades_in() {
        let cam = Camera::default(); // looks north (-z)
        assert_eq!(
            NearFade::default().alpha(&cam, Vec3::new(0.0, 0.0, 0.0)),
            1.0
        );
        let f = NearFade {
            near: 0.5,
            fade: 1.0,
            flank_slope: 1.0,
            flank_depth: 2.0,
        };
        assert_eq!(f.alpha(&cam, Vec3::new(0.0, 0.0, -0.2)), 0.0);
        assert!(near(f.alpha(&cam, Vec3::new(0.0, 0.0, -1.0)), 0.5));
        assert_eq!(f.alpha(&cam, Vec3::new(0.0, 0.0, -3.0)), 1.0);
        // 1.8 ahead but 1.9 to the side: beyond the flank slope
        assert_eq!(f.alpha(&cam, Vec3::new(1.9, 0.0, -1.8)), 0.0);
        let hard = NearFade {
            near: 0.5,
            ..NearFade::default()
        };
        assert_eq!(hard.alpha(&cam, Vec3::new(0.0, 0.0, -0.4)), 0.0);
        assert_eq!(hard.alpha(&cam, Vec3::new(0.0, 0.0, -0.6)), 1.0);
    }

    #[test]
    fn glide_ends_on_target_eases_out_and_turns_the_short_way() {
        let from = Camera {
            yaw: PI - 0.1,
            ..Camera::default()
        };
        let to = Camera {
            position: Vec3::new(2.0, 0.0, 0.0),
            yaw: -PI + 0.1,
            fov_y: 90.0,
            ..Camera::default()
        };
        let start = Camera::glide(&from, &to, 0.0);
        assert!(near(start.position.x, 0.0) && near(start.yaw, from.yaw) && start.fov_y == 90.0);
        let end = Camera::glide(&from, &to, 1.0);
        assert!(near(end.position.x, 2.0) && near(end.yaw, PI + 0.1));
        // Halfway in time is three quarters of the way: fast start, soft stop.
        assert!(near(Camera::glide(&from, &to, 0.5).position.x, 1.5));
        assert!(near(Camera::glide(&from, &to, f32::NAN).position.x, 2.0));
    }
}
