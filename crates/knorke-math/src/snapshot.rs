//! `SnapshotValue` for all math types (ADR 0010): field by field, little-endian, so
//! snapshots are identical on every platform.

use crate::{
    Aabb2, Affine2, At, Capsule2, Circle, IVec2, IVec3, LinearRgba, Mat4, Ray2, Segment2, Space,
    Srgba8, Vec2, Vec3, Vec4,
};
use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

macro_rules! by_fields {
    ($($t:ty { $($f:ident),+ });+ $(;)?) => {$(
        impl SnapshotValue for $t {
            #[inline]
            fn put(&self, w: &mut Writer<'_>) { $( w.put(&self.$f); )+ }
            #[inline]
            fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> { Ok(Self { $( $f: r.get()? ),+ }) }
        }
    )+};
}

by_fields! {
    Vec2 { x, y };
    Vec3 { x, y, z };
    Vec4 { x, y, z, w };
    IVec2 { x, y };
    IVec3 { x, y, z };
    Affine2 { x_axis, y_axis, translation };
    Mat4 { cols };
    LinearRgba { r, g, b, a };
    Srgba8 { r, g, b, a };
    Aabb2 { min, max };
    Circle { center, radius };
    Segment2 { a, b };
    Capsule2 { segment, radius };
    Ray2 { origin, dir };
}

impl<S: Space, V: SnapshotValue> SnapshotValue for At<S, V> {
    fn put(&self, w: &mut Writer<'_>) {
        self.value.put(w);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(At::new(V::get(r)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World2;
    use knorke_types::SnapshotBuf;

    #[test]
    fn round_trip() {
        let mut buf = SnapshotBuf::new();
        let pos: Vec<At<World2, Vec2>> = vec![At::new(Vec2::new(1.5, -2.0)); 3];
        let m = Mat4::orthographic(0.0, 4.0, 0.0, 3.0, 0.0, 1.0);
        let cap = Capsule2::new(Vec2::ZERO, Vec2::X, 0.25);
        buf.write_section("knorke.test.math", 1, |w| {
            w.put(&pos);
            w.put(&m);
            w.put(&cap);
            w.put(&Srgba8::new(1, 2, 3, 4));
        });
        let (_, mut r) = Reader::new(buf.bytes())
            .section("knorke.test.math")
            .unwrap();
        assert_eq!(r.get::<Vec<At<World2, Vec2>>>().unwrap(), pos);
        assert_eq!(r.get::<Mat4>().unwrap(), m);
        assert_eq!(r.get::<Capsule2>().unwrap(), cap);
        assert_eq!(r.get::<Srgba8>().unwrap(), Srgba8::new(1, 2, 3, 4));
        r.finish("knorke.test.math").unwrap();
        assert_eq!(
            &buf.bytes()[buf.sections()[0].payload.start + 4..][..8],
            &[0, 0, 0xC0, 0x3F, 0, 0, 0, 0xC0],
            "LE f32: 1.5, -2.0"
        );
    }
}
