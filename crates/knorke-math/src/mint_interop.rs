//! Conversions with the `mint` interop types. Same layout, so these compile to moves.

use crate::{IVec2, IVec3, Mat4, Vec2, Vec3, Vec4};

macro_rules! via_mint {
    ($($ours:ty => $theirs:ty { $($f:ident),+ });+ $(;)?) => {$(
        impl From<$theirs> for $ours {
            #[inline]
            fn from(v: $theirs) -> Self { Self { $($f: v.$f),+ } }
        }
        impl From<$ours> for $theirs {
            #[inline]
            fn from(v: $ours) -> Self { Self { $($f: v.$f),+ } }
        }
    )+};
}

via_mint! {
    Vec2 => mint::Vector2<f32> { x, y };
    Vec3 => mint::Vector3<f32> { x, y, z };
    Vec4 => mint::Vector4<f32> { x, y, z, w };
    IVec2 => mint::Vector2<i32> { x, y };
    IVec3 => mint::Vector3<i32> { x, y, z };
    Vec2 => mint::Point2<f32> { x, y };
    Vec3 => mint::Point3<f32> { x, y, z };
}

impl From<mint::ColumnMatrix4<f32>> for Mat4 {
    #[inline]
    fn from(m: mint::ColumnMatrix4<f32>) -> Self {
        Self {
            cols: [m.x.into(), m.y.into(), m.z.into(), m.w.into()],
        }
    }
}

impl From<Mat4> for mint::ColumnMatrix4<f32> {
    #[inline]
    fn from(m: Mat4) -> Self {
        Self {
            x: m.cols[0].into(),
            y: m.cols[1].into(),
            z: m.cols[2].into(),
            w: m.cols[3].into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(Vec3::from(mint::Vector3::from(v)), v);
        let m = Mat4::orthographic(0.0, 2.0, 0.0, 2.0, 0.0, 1.0);
        assert_eq!(Mat4::from(mint::ColumnMatrix4::from(m)), m);
    }
}
