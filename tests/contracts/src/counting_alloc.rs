//! Allocation counter for the no-allocation contracts (ADR 0006).
//!
//! Each test binary installs it with
//! `#[global_allocator] static GLOBAL: Counting = Counting;` and compares
//! [`allocations`] (all threads) or [`thread_allocations`] (calling thread only) before and
//! after the steady-state section. Linking this library alone installs nothing.

// ADR 0005: `unsafe` is allowed for test allocators, here and nowhere else in this crate.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// Forwards to the system allocator and counts every `alloc` and `realloc`.
/// `dealloc` is not counted: freeing is not what the contracts forbid.
pub struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
thread_local! {
    // Per-thread count, so tests running in parallel in one binary do not see each
    // other's allocations (used by `exchange.rs`).
    static THREAD_ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count() {
    // SeqCst so tests that read the counter after rayon work see every increment.
    ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
    // `try_with`: the allocator also runs while thread-locals are being torn down.
    let _ = THREAD_ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
}

/// Allocations on all threads since program start.
pub fn allocations() -> u64 {
    ALLOCATIONS.load(Ordering::SeqCst)
}

/// Allocations on the calling thread since it started.
pub fn thread_allocations() -> u64 {
    THREAD_ALLOCATIONS.with(Cell::get)
}

// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
