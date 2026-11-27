// Origin: chiaro @ 9bc5b9b, src/rendering/SdfShadows.hpp and SdfShadows.cpp (RectParams,
// DrawTexelRects) and src/rendering/OcclusionMask.cpp (RecordShiftCopy).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU-free half of the occlusion stage: the uniform block of the rectangle shader and the
//! copy of a camera pan.

use crate::shadow::DirtyRectList;
use knorke_math::uniform::write_f32s;

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
        write_f32s(
            &mut self.bytes[size_at..],
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
