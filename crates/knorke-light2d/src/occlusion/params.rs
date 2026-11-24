// Origin: chiaro @ 9bc5b9b, src/rendering/SdfShadows.hpp and SdfShadows.cpp (kFieldBound,
// JfaSpan, JfaParams, RectParams, DrawTexelRects) and src/rendering/OcclusionMask.cpp
// (kUpdateLimits, RecordShiftCopy).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU-free half of the occlusion stage: the jump flooding schedule, the uniform blocks of
//! the field and rectangle shaders and the copy of a camera pan.

use crate::shadow::{DirtyRectList, FULL_REBUILD_RATIO, MaskUpdateLimits};
use knorke_math::uniform_block;

/// Largest stored distance in texels (E21): farther texels and texels without any obstacle hold
/// this value.
pub const FIELD_BOUND: u32 = 32;

/// `K`, the smallest power of two not below [`FIELD_BOUND`]; jump flooding runs
/// `k = K / 2 ... 1` plus one extra `k = 1`.
pub const fn jfa_span() -> u32 {
    let mut span = 1;
    while span < FIELD_BOUND {
        span *= 2;
    }
    span
}

// Field rectangles (changed rectangles grown by FIELD_BOUND) must hold every texel whose clamped
// distance can change, and jump flooding reads seeds up to jfa_span() away. Growing by
// FIELD_BOUND matches that only while both are equal, that is while FIELD_BOUND is a power of two.
// Seed rectangles grown by jfa_span() + 2 keep stale seeds out of reach.
const _: () = assert!(
    jfa_span() == FIELD_BOUND,
    "FIELD_BOUND must be a power of two"
);

/// The rectangle rules of the cache for this field ([`MaskUpdatePlan`](crate::shadow::MaskUpdatePlan)).
pub const UPDATE_LIMITS: MaskUpdateLimits = MaskUpdateLimits {
    field_grow: FIELD_BOUND as i32,
    seed_grow: jfa_span() as i32 + 2,
    full_rebuild_ratio: FULL_REBUILD_RATIO,
};

/// The jump distances of the step passes: `K / 2, ..., 1` and one extra `1`, which fixes most of
/// the remaining jump flooding errors.
pub fn jfa_steps() -> impl Iterator<Item = u32> {
    let halving = core::iter::successors(Some(jfa_span() / 2), |k| (*k > 1).then_some(k / 2));
    halving.chain(core::iter::once(1))
}

fn put_f32(out: &mut [u8], at: usize, values: &[f32]) {
    for (i, v) in values.iter().enumerate() {
        out[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
}

uniform_block! {
    /// `cbuffer JfaParams` of `jfa_seed.frag`, `jfa_step.frag` and `jfa_distance.frag`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct JfaParams[32] {
        /// xy size in texels, zw `1 / size`.
        pub domain_size: [f32; 4],
        /// x jump distance in texels (step pass), y field bound in texels (distance pass).
        pub step: [f32; 4],
    }
}

impl JfaParams {
    /// The block for a target of `width` x `height` texels, step 0.
    pub fn new(width: u32, height: u32) -> Self {
        let (w, h) = (width as f32, height as f32);
        Self {
            domain_size: [w, h, 1.0 / w, 1.0 / h],
            step: [0.0; 4],
        }
    }

    /// The block of a step pass with jump distance `k`.
    #[must_use]
    pub fn with_step(mut self, k: f32) -> Self {
        self.step = [k, 0.0, 0.0, 0.0];
        self
    }

    /// The block of the distance pass (step 0, bound [`FIELD_BOUND`]).
    #[must_use]
    pub fn distance(mut self) -> Self {
        self.step = [0.0, FIELD_BOUND as f32, 0.0, 0.0];
        self
    }
}

/// `cbuffer RectParams` of `rect.vert`: up to [`DirtyRectList::MAX_DIRTY_RECTS`] half-open texel
/// rectangles, one quad instance each, in a target of the given size. Kept and rewritten in
/// place, so a frame allocates nothing.
#[derive(Clone, Debug)]
pub struct RectParams {
    bytes: [u8; Self::SIZE],
}

impl RectParams {
    /// Size of the block in bytes.
    pub const SIZE: usize = DirtyRectList::MAX_DIRTY_RECTS * 16 + 16;

    /// A block without rectangles.
    pub fn new() -> Self {
        Self {
            bytes: [0; Self::SIZE],
        }
    }

    /// Writes `rects` and the target size; returns the number of rectangles (instances).
    pub fn write(&mut self, rects: &DirtyRectList, width: u32, height: u32) -> u32 {
        let list = rects.rects();
        for (i, r) in list.iter().enumerate() {
            for (c, v) in [r.x0, r.y0, r.x1, r.y1].into_iter().enumerate() {
                let at = i * 16 + c * 4;
                self.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        let size_at = DirtyRectList::MAX_DIRTY_RECTS * 16;
        put_f32(
            &mut self.bytes,
            size_at,
            &[width as f32, height as f32, 0.0, 0.0],
        );
        list.len() as u32
    }

    /// The bytes the shader reads.
    pub fn bytes(&self) -> &[u8; Self::SIZE] {
        &self.bytes
    }
}

impl Default for RectParams {
    fn default() -> Self {
        Self::new()
    }
}

/// The copy of a camera pan: `new[x] = old[x - shift]`, the overlap of the old and the new mask
/// (the exposed strips are redrawn afterwards).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShiftCopy {
    /// First texel read in the old texture.
    pub src: [u32; 2],
    /// First texel written in the new texture.
    pub dst: [u32; 2],
    /// Texels copied per axis.
    pub size: [u32; 2],
}

impl ShiftCopy {
    /// The copy for `shift` in a target of `width` x `height` texels; `None` without a shift or
    /// without overlap.
    pub fn new(shift: [i32; 2], width: u32, height: u32) -> Option<Self> {
        if shift == [0, 0] {
            return None;
        }
        let mut c = Self {
            src: [0; 2],
            dst: [0; 2],
            size: [0; 2],
        };
        for (axis, limit) in [(0, width), (1, height)] {
            let s = shift[axis];
            let along = s.unsigned_abs();
            if along >= limit {
                return None;
            }
            c.size[axis] = limit - along;
            c.src[axis] = if s < 0 { along } else { 0 };
            c.dst[axis] = if s > 0 { along } else { 0 };
        }
        Some(c)
    }
}
