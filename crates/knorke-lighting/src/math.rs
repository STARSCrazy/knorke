//! `knorke_math::Vec3` plus the one helper whose exact behaviour the reference images depend on.

pub(crate) use knorke_math::Vec3;

/// Normalization as the software renderer has always done it (origin: pixelraum `math.rs`):
/// divide by the length unless it is exactly zero. `knorke_math::Vec3::normalize_or_zero`
/// treats lengths up to `f32::EPSILON` as zero, which would change pixels of tiny vectors.
pub(crate) trait Vec3Ext {
    /// Unit vector, or zero for the zero vector.
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
