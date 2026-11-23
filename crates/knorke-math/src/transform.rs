use crate::{Vec2, Vec3, Vec4};
use core::ops::Mul;

/// 2D affine transform (rotation, scale, shear, translation). Column-major, layout:
/// `[x_axis.x, x_axis.y, y_axis.x, y_axis.y, translation.x, translation.y]`, 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2 {
    /// Image of the X unit vector.
    pub x_axis: Vec2,
    /// Image of the Y unit vector.
    pub y_axis: Vec2,
    /// Translation.
    pub translation: Vec2,
}

impl Default for Affine2 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine2 {
    /// No-op transform.
    pub const IDENTITY: Self = Self {
        x_axis: Vec2::X,
        y_axis: Vec2::Y,
        translation: Vec2::ZERO,
    };

    /// Pure translation.
    #[inline]
    pub const fn from_translation(t: Vec2) -> Self {
        Self {
            translation: t,
            ..Self::IDENTITY
        }
    }

    /// Pure (non-uniform) scale.
    #[inline]
    pub const fn from_scale(s: Vec2) -> Self {
        Self {
            x_axis: Vec2::new(s.x, 0.0),
            y_axis: Vec2::new(0.0, s.y),
            translation: Vec2::ZERO,
        }
    }

    /// Counter-clockwise rotation by `angle` radians.
    #[inline]
    pub fn from_angle(angle: f32) -> Self {
        let x = Vec2::from_angle(angle);
        Self {
            x_axis: x,
            y_axis: x.perp(),
            translation: Vec2::ZERO,
        }
    }

    /// Scale, then rotate, then translate.
    #[inline]
    pub fn from_scale_angle_translation(scale: Vec2, angle: f32, translation: Vec2) -> Self {
        let r = Self::from_angle(angle);
        Self {
            x_axis: r.x_axis * scale.x,
            y_axis: r.y_axis * scale.y,
            translation,
        }
    }

    /// Transforms a point (translation applies).
    #[inline]
    pub fn transform_point(&self, p: Vec2) -> Vec2 {
        self.x_axis * p.x + self.y_axis * p.y + self.translation
    }

    /// Transforms a direction (translation does not apply).
    #[inline]
    pub fn transform_vector(&self, v: Vec2) -> Vec2 {
        self.x_axis * v.x + self.y_axis * v.y
    }

    /// Inverse, or `None` if the transform is singular.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.x_axis.perp_dot(self.y_axis);
        if det.abs() <= f32::EPSILON * f32::EPSILON {
            return None;
        }
        let inv = 1.0 / det;
        let x_axis = Vec2::new(self.y_axis.y, -self.x_axis.y) * inv;
        let y_axis = Vec2::new(-self.y_axis.x, self.x_axis.x) * inv;
        let lin = Self {
            x_axis,
            y_axis,
            translation: Vec2::ZERO,
        };
        Some(Self {
            translation: -lin.transform_vector(self.translation),
            ..lin
        })
    }
}

/// `a * b` applies `b` first, then `a`.
impl Mul for Affine2 {
    type Output = Self;
    #[inline]
    fn mul(self, b: Self) -> Self {
        Self {
            x_axis: self.transform_vector(b.x_axis),
            y_axis: self.transform_vector(b.y_axis),
            translation: self.transform_point(b.translation),
        }
    }
}

/// 4×4 `f32` matrix, column-major, layout as four [`Vec4`] columns, 64 bytes. Mainly for
/// GPU uniforms (camera/projection).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    /// Columns.
    pub cols: [Vec4; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    /// Identity.
    pub const IDENTITY: Self = Self {
        cols: [
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        ],
    };

    /// Embeds a 2D affine transform (z passes through).
    pub const fn from_affine2(a: Affine2) -> Self {
        Self {
            cols: [
                Vec4::new(a.x_axis.x, a.x_axis.y, 0.0, 0.0),
                Vec4::new(a.y_axis.x, a.y_axis.y, 0.0, 0.0),
                Vec4::new(0.0, 0.0, 1.0, 0.0),
                Vec4::new(a.translation.x, a.translation.y, 0.0, 1.0),
            ],
        }
    }

    /// Orthographic projection to clip space with depth in `0..=1` (Vulkan/D3D/Metal and
    /// SDL3 GPU convention), right-handed, y up.
    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let rw = 1.0 / (right - left);
        let rh = 1.0 / (top - bottom);
        let rd = 1.0 / (near - far);
        Self {
            cols: [
                Vec4::new(2.0 * rw, 0.0, 0.0, 0.0),
                Vec4::new(0.0, 2.0 * rh, 0.0, 0.0),
                Vec4::new(0.0, 0.0, rd, 0.0),
                Vec4::new(-(left + right) * rw, -(top + bottom) * rh, near * rd, 1.0),
            ],
        }
    }

    /// Row `i` as a vector.
    #[inline]
    fn row(&self, i: usize) -> Vec4 {
        let c = |v: Vec4| v.to_array()[i];
        Vec4::new(
            c(self.cols[0]),
            c(self.cols[1]),
            c(self.cols[2]),
            c(self.cols[3]),
        )
    }

    /// Transforms a homogeneous vector.
    #[inline]
    pub fn transform(&self, v: Vec4) -> Vec4 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z + self.cols[3] * v.w
    }

    /// Transforms a point (w = 1) and divides by w.
    #[inline]
    pub fn project_point(&self, p: Vec3) -> Vec3 {
        let v = self.transform(p.extend(1.0));
        Vec3::new(v.x, v.y, v.z) / v.w
    }
}

/// `a * b` applies `b` first, then `a`.
impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let rows = [self.row(0), self.row(1), self.row(2), self.row(3)];
        let col = |c: Vec4| {
            Vec4::new(
                rows[0].dot(c),
                rows[1].dot(c),
                rows[2].dot(c),
                rows[3].dot(c),
            )
        };
        Self {
            cols: [
                col(b.cols[0]),
                col(b.cols[1]),
                col(b.cols[2]),
                col(b.cols[3]),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::FRAC_PI_2;

    #[test]
    fn affine_compose_and_invert() {
        let a = Affine2::from_scale_angle_translation(
            Vec2::new(2.0, 3.0),
            FRAC_PI_2,
            Vec2::new(10.0, 0.0),
        );
        let p = a.transform_point(Vec2::new(1.0, 1.0));
        assert!(p.abs_diff_eq(Vec2::new(7.0, 2.0), 1e-5), "{p:?}");
        let back = a.inverse().unwrap().transform_point(p);
        assert!(back.abs_diff_eq(Vec2::new(1.0, 1.0), 1e-5), "{back:?}");
        let t = Affine2::from_translation(Vec2::new(1.0, 0.0));
        assert_eq!(
            (t * Affine2::from_scale(Vec2::splat(2.0))).transform_point(Vec2::ONE),
            Vec2::new(3.0, 2.0)
        );
        assert_eq!(Affine2::from_scale(Vec2::new(0.0, 1.0)).inverse(), None);
    }

    #[test]
    fn mat4_matches_affine_and_ortho_maps_corners() {
        let a =
            Affine2::from_scale_angle_translation(Vec2::new(2.0, 1.0), 0.3, Vec2::new(-4.0, 5.0));
        let p = Vec2::new(1.5, -2.0);
        let m = Mat4::from_affine2(a);
        assert!(
            m.project_point(p.extend(0.0))
                .truncate()
                .abs_diff_eq(a.transform_point(p), 1e-5)
        );
        assert_eq!(m * Mat4::IDENTITY, m);
        let o = Mat4::orthographic(0.0, 800.0, 0.0, 600.0, 0.0, 1.0);
        assert!(
            o.project_point(Vec3::new(0.0, 0.0, 0.0))
                .abs_diff_eq(Vec3::new(-1.0, -1.0, 0.0), 1e-6)
        );
        assert!(
            o.project_point(Vec3::new(800.0, 600.0, -1.0))
                .abs_diff_eq(Vec3::new(1.0, 1.0, 1.0), 1e-6)
        );
        let both = Mat4::from_affine2(Affine2::from_translation(Vec2::X)) * Mat4::from_affine2(a);
        assert!(
            both.project_point(p.extend(0.0))
                .truncate()
                .abs_diff_eq(a.transform_point(p) + Vec2::X, 1e-5)
        );
    }
}
