// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/StagingMath.hpp and tests/unit/StagingMathTests.cpp.
//
// Original notice (zlib):
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered: rewritten in Rust for the Knorke Engine (results returned instead of out-parameters).

//! Size arithmetic for staging and upload buffers. Pure logic, no GPU required.

/// Largest size of one SDL_GPU buffer or transfer buffer (its size field is a `u32`).
pub const MAX_GPU_BUFFER_BYTES: u64 = u32::MAX as u64;

/// Reserves `bytes` in a staging buffer at the next multiple of `alignment` after `*total`.
///
/// On success returns the start offset and sets `*total` to the new end. Returns `None` and
/// changes nothing if the end would exceed `limit` or the start does not fit a `u32`; the sum
/// is computed in `u64`, so it cannot wrap. `alignment` must be non-zero.
pub fn reserve_staging(total: &mut u64, bytes: u64, alignment: u32, limit: u64) -> Option<u32> {
    let alignment = u64::from(alignment);
    let start = total.checked_add(alignment - 1)? / alignment * alignment;
    if start > limit || bytes > limit - start {
        return None;
    }
    let offset = u32::try_from(start).ok()?;
    *total = start + bytes;
    Some(offset)
}

/// Capacity for `needed` units: `max(current, initial, 1)` doubled until it holds `needed`,
/// capped at `limit`. Returns `None` if `needed` exceeds `limit`, so the doubling always ends.
pub fn grow_capacity(current: u64, initial: u64, needed: u64, limit: u64) -> Option<u64> {
    if needed > limit {
        return None;
    }
    let mut c = current.max(initial).max(1);
    while c < needed {
        // Saturating, so the loop ends for any limit (needed <= limit <= u64::MAX).
        c = c.saturating_mul(2);
    }
    Some(c.min(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_reservations_are_aligned_and_packed() {
        let mut total = 0;
        assert_eq!(
            reserve_staging(&mut total, 3, 512, MAX_GPU_BUFFER_BYTES),
            Some(0)
        );
        assert_eq!(total, 3);
        assert_eq!(
            reserve_staging(&mut total, 1024, 512, MAX_GPU_BUFFER_BYTES),
            Some(512)
        );
        assert_eq!(total, 1536);
        assert_eq!(
            reserve_staging(&mut total, 0, 512, MAX_GPU_BUFFER_BYTES),
            Some(1536)
        );
    }

    #[test]
    fn staging_reservations_beyond_4_gib_fail_instead_of_wrapping() {
        // 256 pending 4096x4096 tile grids: the old u32 sum wrapped to 0 here.
        let grid: u64 = 4096 * 4096;
        let mut total = 0;
        let mut offset = 0;
        let mut reserved = 0;
        while let Some(o) = reserve_staging(&mut total, grid, 512, MAX_GPU_BUFFER_BYTES) {
            offset = o;
            reserved += 1;
            assert!(reserved <= 256);
        }
        assert_eq!(reserved, 255); // 255 * 16 MiB fits below 2^32, the 256th grid does not
        assert_eq!(total, 255 * grid);
        assert_eq!(u64::from(offset), 254 * grid); // unchanged by the failed call

        // Exactly up to the limit is fine, one byte more is not.
        total = 0;
        assert!(
            reserve_staging(&mut total, MAX_GPU_BUFFER_BYTES, 512, MAX_GPU_BUFFER_BYTES).is_some()
        );
        total = 1;
        assert!(
            reserve_staging(
                &mut total,
                MAX_GPU_BUFFER_BYTES - 1,
                512,
                MAX_GPU_BUFFER_BYTES
            )
            .is_none()
        );
        assert_eq!(total, 1);
    }

    #[test]
    fn capacity_doubling_terminates_and_respects_the_limit() {
        assert_eq!(
            grow_capacity(0, 64 * 1024, 100, MAX_GPU_BUFFER_BYTES),
            Some(64 * 1024)
        );
        assert_eq!(
            grow_capacity(64 * 1024, 64 * 1024, 200 * 1024, MAX_GPU_BUFFER_BYTES),
            Some(256 * 1024)
        );

        // Past 2^31 a u32 capacity wrapped to 0 and the loop spun forever; now it is capped.
        assert_eq!(
            grow_capacity(64 * 1024, 64 * 1024, (1 << 31) + 1, MAX_GPU_BUFFER_BYTES),
            Some(MAX_GPU_BUFFER_BYTES)
        );
        assert_eq!(
            grow_capacity(
                64 * 1024,
                64 * 1024,
                MAX_GPU_BUFFER_BYTES + 1,
                MAX_GPU_BUFFER_BYTES
            ),
            None
        );

        // Float capacities: the byte size must still fit into a u32.
        const MAX_FLOATS: u64 = MAX_GPU_BUFFER_BYTES / size_of::<f32>() as u64;
        let capacity = grow_capacity(4096, 4096, MAX_FLOATS, MAX_FLOATS).unwrap();
        assert!(capacity * size_of::<f32>() as u64 <= MAX_GPU_BUFFER_BYTES);
    }
}
