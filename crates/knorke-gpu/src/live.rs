// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/GpuHandle.hpp (ResourceCounter, g_liveGpuObjects).
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
// Altered: rewritten in Rust for the Knorke Engine; the counter is atomic so handles may be
// dropped from any thread that owns them.

//! Liveness counters for GPU objects (leak tests). Pure logic, no GPU required.

use core::sync::atomic::{AtomicU32, Ordering};

/// Process-wide number of live GPU objects created through [`GpuHandle`](crate::GpuHandle).
static GLOBAL_LIVE: AtomicU32 = AtomicU32::new(0);

/// Process-wide number of live GPU objects owned through any [`LiveCounter`].
///
/// Diagnostic only: concurrently running tests share it, so leak tests compare their own
/// [`LiveCounter`] instead.
pub fn global_live_gpu_objects() -> u32 {
    GLOBAL_LIVE.load(Ordering::Relaxed)
}

/// Counts the GPU objects owned by one user (a context, a pass, a test).
///
/// Every [`GpuHandle`](crate::GpuHandle) adds one on creation and removes it on drop, so
/// `live() == 0` after teardown proves that nothing leaked.
#[derive(Debug, Default)]
pub struct LiveCounter {
    live: AtomicU32,
}

impl LiveCounter {
    /// A counter with no live objects.
    pub const fn new() -> Self {
        Self {
            live: AtomicU32::new(0),
        }
    }

    /// Number of objects currently alive under this counter.
    pub fn live(&self) -> u32 {
        self.live.load(Ordering::Relaxed)
    }

    /// Records one created object.
    pub fn add(&self) {
        self.live.fetch_add(1, Ordering::Relaxed);
        GLOBAL_LIVE.fetch_add(1, Ordering::Relaxed);
    }

    /// Records one released object. Panics in debug builds on underflow (a double release).
    pub fn remove(&self) {
        let before = self.live.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(before > 0, "LiveCounter underflow: object released twice");
        GLOBAL_LIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_tracks_adds_and_removes() {
        let c = LiveCounter::new();
        assert_eq!(c.live(), 0);
        c.add();
        c.add();
        assert_eq!(c.live(), 2);
        c.remove();
        assert_eq!(c.live(), 1);
        c.remove();
        assert_eq!(c.live(), 0);
    }

    #[test]
    fn counters_are_independent() {
        let a = LiveCounter::new();
        let b = LiveCounter::new();
        a.add();
        assert_eq!((a.live(), b.live()), (1, 0));
        a.remove();
    }
}
