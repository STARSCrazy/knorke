use core::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

macro_rules! vector {
    ($(#[$doc:meta])* $name:ident, $scalar:ty, $zero:expr, $one:expr, [$($f:ident),+]) => {
        $(#[$doc])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct $name { $(#[doc = concat!("`", stringify!($f), "` component.")] pub $f: $scalar),+ }

        impl $name {
            /// All components zero.
            pub const ZERO: Self = Self { $($f: $zero),+ };
            /// All components one.
            pub const ONE: Self = Self { $($f: $one),+ };

            /// Creates a vector from its components.
            #[inline]
            pub const fn new($($f: $scalar),+) -> Self { Self { $($f),+ } }

            /// All components set to `v`.
            #[inline]
            pub const fn splat(v: $scalar) -> Self { Self { $($f: v),+ } }

            /// Dot product.
            #[inline]
            pub fn dot(self, o: Self) -> $scalar { $zero $(+ self.$f * o.$f)+ }

            /// Component-wise minimum.
            #[inline]
            pub fn min(self, o: Self) -> Self { Self { $($f: if o.$f < self.$f { o.$f } else { self.$f }),+ } }

            /// Component-wise maximum.
            #[inline]
            pub fn max(self, o: Self) -> Self { Self { $($f: if o.$f > self.$f { o.$f } else { self.$f }),+ } }

            /// Squared length.
            #[inline]
            pub fn length_squared(self) -> $scalar { self.dot(self) }

            /// Components as an array (same memory layout).
            #[inline]
            pub const fn to_array(self) -> [$scalar; [$(stringify!($f)),+].len()] { [$(self.$f),+] }
        }

        impl From<[$scalar; [$(stringify!($f)),+].len()]> for $name {
            #[inline]
            fn from([$($f),+]: [$scalar; [$(stringify!($f)),+].len()]) -> Self { Self { $($f),+ } }
        }
        impl From<$name> for [$scalar; [$(stringify!($f)),+].len()] {
            #[inline]
            fn from(v: $name) -> Self { v.to_array() }
        }

        impl Add for $name { type Output = Self; #[inline] fn add(self, o: Self) -> Self { Self { $($f: self.$f + o.$f),+ } } }
        impl Sub for $name { type Output = Self; #[inline] fn sub(self, o: Self) -> Self { Self { $($f: self.$f - o.$f),+ } } }
        impl Mul<$scalar> for $name { type Output = Self; #[inline] fn mul(self, s: $scalar) -> Self { Self { $($f: self.$f * s),+ } } }
        impl Mul for $name { type Output = Self; #[inline] fn mul(self, o: Self) -> Self { Self { $($f: self.$f * o.$f),+ } } }
        impl Neg for $name { type Output = Self; #[inline] fn neg(self) -> Self { Self { $($f: -self.$f),+ } } }
        impl AddAssign for $name { #[inline] fn add_assign(&mut self, o: Self) { *self = *self + o; } }
        impl SubAssign for $name { #[inline] fn sub_assign(&mut self, o: Self) { *self = *self - o; } }
        impl MulAssign<$scalar> for $name { #[inline] fn mul_assign(&mut self, s: $scalar) { *self = *self * s; } }
    };
}

macro_rules! float_vector {
    ($name:ident, [$($f:ident),+]) => {
        impl $name {
            /// Length.
            #[inline]
            pub fn length(self) -> f32 { self.length_squared().sqrt() }

            /// Unit vector in the same direction, or zero for a (near) zero vector.
            #[inline]
            pub fn normalize_or_zero(self) -> Self {
                let len = self.length();
                if len > f32::EPSILON { self * (1.0 / len) } else { Self::ZERO }
            }

            /// Linear interpolation: `self` at `t = 0`, `o` at `t = 1`.
            #[inline]
            pub fn lerp(self, o: Self, t: f32) -> Self { self + (o - self) * t }

            /// Distance to `o`.
            #[inline]
            pub fn distance(self, o: Self) -> f32 { (o - self).length() }

            /// `true` if every component differs by at most `eps`.
            #[inline]
            pub fn abs_diff_eq(self, o: Self, eps: f32) -> bool { true $(&& (self.$f - o.$f).abs() <= eps)+ }
        }

        impl Div<f32> for $name { type Output = Self; #[inline] fn div(self, s: f32) -> Self { Self { $($f: self.$f / s),+ } } }
    };
}

vector!(
    /// 2D `f32` vector. Layout: `[x, y]`, 8 bytes.
    Vec2,
    f32,
    0.0,
    1.0,
    [x, y]
);
vector!(
    /// 3D `f32` vector. Layout: `[x, y, z]`, 12 bytes (no padding; use [`Vec4`] for GPU
    /// buffers that need 16-byte alignment).
    Vec3,
    f32,
    0.0,
    1.0,
    [x, y, z]
);
vector!(
    /// 4D `f32` vector. Layout: `[x, y, z, w]`, 16 bytes.
    Vec4,
    f32,
    0.0,
    1.0,
    [x, y, z, w]
);
vector!(
    /// 2D `i32` vector, e.g. grid cells. Layout: `[x, y]`, 8 bytes.
    IVec2,
    i32,
    0,
    1,
    [x, y]
);
vector!(
    /// 3D `i32` vector, e.g. grid cells with level. Layout: `[x, y, z]`, 12 bytes.
    IVec3,
    i32,
    0,
    1,
    [x, y, z]
);
float_vector!(Vec2, [x, y]);
float_vector!(Vec3, [x, y, z]);
float_vector!(Vec4, [x, y, z, w]);

impl Eq for IVec2 {}
impl Eq for IVec3 {}
impl core::hash::Hash for IVec2 {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.to_array().hash(h);
    }
}
impl core::hash::Hash for IVec3 {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.to_array().hash(h);
    }
}

impl Vec2 {
    /// Unit X.
    pub const X: Self = Self::new(1.0, 0.0);
    /// Unit Y.
    pub const Y: Self = Self::new(0.0, 1.0);

    /// Z component of the 3D cross product: positive if `o` is counter-clockwise from `self`.
    #[inline]
    pub fn perp_dot(self, o: Self) -> f32 {
        self.x * o.y - self.y * o.x
    }

    /// Angle from +X in radians, counter-clockwise, in `(-π, π]` (deterministic, ADR 0011).
    #[inline]
    pub fn angle(self) -> f32 {
        crate::det::atan2(self.y, self.x)
    }

    /// Rotated by 90° counter-clockwise.
    #[inline]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Unit vector at `angle` radians from +X, counter-clockwise (deterministic, ADR 0011).
    #[inline]
    pub fn from_angle(angle: f32) -> Self {
        let (s, c) = crate::det::sin_cos(angle);
        Self::new(c, s)
    }

    /// Extends to 3D.
    #[inline]
    pub const fn extend(self, z: f32) -> Vec3 {
        Vec3::new(self.x, self.y, z)
    }
}

impl Vec3 {
    /// Cross product.
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    /// Drops `z`.
    #[inline]
    pub const fn truncate(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Extends to 4D.
    #[inline]
    pub const fn extend(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }
}

/// Normalization that only a length of exactly zero stops (the renderer's reference images
/// depend on it). [`Vec3::normalize_or_zero`] treats lengths up to `f32::EPSILON` as zero,
/// which would change the pixels of tiny vectors.
pub trait Vec3Ext {
    /// Divides by the length unless it is exactly zero; the zero vector stays zero.
    fn normalized(self) -> Vec3;
}

impl Vec3Ext for Vec3 {
    #[inline]
    fn normalized(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            Vec3::new(0.0, 0.0, 0.0)
        }
    }
}

impl IVec2 {
    /// Converts to `f32` (exact for |v| ≤ 2²⁴).
    #[inline]
    pub fn as_vec2(self) -> Vec2 {
        Vec2::new(self.x as f32, self.y as f32)
    }
}

impl IVec3 {
    /// Converts to `f32` (exact for |v| ≤ 2²⁴).
    #[inline]
    pub fn as_vec3(self) -> Vec3 {
        Vec3::new(self.x as f32, self.y as f32, self.z as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = Vec2::new(3.0, 4.0);
        assert_eq!(a.length(), 5.0);
        assert_eq!(a + Vec2::ONE, Vec2::new(4.0, 5.0));
        assert_eq!(a * 2.0, Vec2::new(6.0, 8.0));
        assert_eq!(a.dot(Vec2::X), 3.0);
        assert!(a.normalize_or_zero().abs_diff_eq(Vec2::new(0.6, 0.8), 1e-6));
        assert_eq!(Vec2::ZERO.normalize_or_zero(), Vec2::ZERO);
        assert_eq!(Vec2::X.perp(), Vec2::Y);
        assert_eq!(Vec2::X.perp_dot(Vec2::Y), 1.0);
        assert_eq!(
            Vec3::new(1.0, 0.0, 0.0).cross(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        );
        assert_eq!(IVec2::new(2, -1) * 3, IVec2::new(6, -3));
        assert_eq!(
            IVec3::new(1, 2, 3).min(IVec3::splat(2)),
            IVec3::new(1, 2, 2)
        );
        assert_eq!(<[f32; 2]>::from(Vec2::new(1.0, 2.0)), [1.0, 2.0]);
    }
}
