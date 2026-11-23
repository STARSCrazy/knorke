//! Host-side executors for [`knorke_types::Parallel`] (ADR 0008).
//!
//! Modules only see the `Parallel` trait; the host picks the executor once:
//!
//! | Executor | Use when |
//! |---|---|
//! | [`knorke_types::Serial`] | tests, determinism reference, single-core targets |
//! | [`Rayon`] (feature `rayon`, default) | normal game loop: persistent work-stealing pool, µs-level overhead |
//! | [`ScopedThreads`] | no extra dependency wanted; spawns threads per call, so only for coarse work (≥ ~1 ms) |
//!
//! All executors produce identical results for the same chunking (see the contract tests).

mod scoped;
pub use scoped::ScopedThreads;

#[cfg(feature = "rayon")]
mod rayon_exec;
#[cfg(feature = "rayon")]
pub use rayon_exec::Rayon;

pub use knorke_types::{Parallel, Serial};
