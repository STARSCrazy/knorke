//! Post-processing of the Knorke Engine, ported from Chiaro (ADR 0014, packages P4, P17, P19, P20).
//!
//! The CPU side of the output stage: colour transfer functions, tonemapping, grading, the output
//! rectangle, the CRT and bloom parameter math, the shockwave helpers, the settings of these
//! stages ([`settings`], P19), the flag word of the output shader ([`output_flags`]) and the
//! uniform blocks of the bloom and output shaders ([`params`], P20). All of it is GPU-free and
//! allocation-free.
//!
//! With feature `gpu` the module `pass` adds [`PostRenderer`](pass::PostRenderer): the bloom
//! chain and the output pass (exposure, bloom, tonemap, grade, vignette, CRT, shockwave
//! distortion, scaling and letterbox, overlay, sRGB encode) on `knorke-gpu`, recording into a
//! command buffer the host lends (ADR 0002). Shaders are Chiaro's compiled ones from a
//! `.kpak` pack (`cargo xtask shaders`). The crate does not depend on any rendering or lighting
//! crate.
//!
//! # Not ported (yet)
//!
//! - `Formats.*` (ADR 0014 P4): the format tables map `SDL_GPUTextureFormat` values and live in
//!   `knorke_gpu::formats`; the GPU pass uses them.
//! - Tests that need the C API structs (`CHI_CrtSettings`, `CHI_BloomSettings`, `CHI_ShockwaveDesc`,
//!   `CHI_OutputSettings`), the settings store, the SDL error strings, the output flags or the C++
//!   wrapper: the same checks run here on the Rust types (field names and values), the struct/flag/
//!   error-string parts are dropped.
//! - `MediumCloud` and its tests in `OutputGradeTests.cpp` (E112): `knorke_light2d::medium_draw`.
//!
//! Simulation-safe math goes through `knorke_math::det` (clippy forbids the `std` variants).
//! Clamping uses [`clamp`], which mirrors C++ `std::clamp` and never panics, also for NaN bounds.

pub mod bloom;
pub mod color;
pub mod crt;
pub mod output_flags;
pub mod output_rect;
pub mod params;
#[cfg(feature = "gpu")]
pub mod pass;
pub mod settings;
pub mod shockwave;

/// Clamps `v` to `[lo, hi]` like C++ `std::clamp`: a value below `lo` gives `lo`, above `hi` gives
/// `hi`, otherwise `v` (also NaN). Unlike `f32::clamp` it never panics.
#[inline]
pub(crate) fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

/// Test helpers mirroring doctest's `Approx`: `|a - b| < eps * (1 + max(|a|, |b|))`.
#[cfg(test)]
pub(crate) mod testing {
    use knorke_math::det;

    /// doctest's default epsilon for `float`.
    pub(crate) const DEFAULT_EPS: f32 = f32::EPSILON * 100.0;

    /// Same test as doctest `Approx` with the default epsilon.
    pub(crate) fn approx(actual: f32, expected: f32) -> bool {
        near(actual, expected, DEFAULT_EPS)
    }

    /// Same test as doctest `Approx(expected).epsilon(eps)`.
    pub(crate) fn near(actual: f32, expected: f32, eps: f32) -> bool {
        (actual - expected).abs() < eps * (1.0 + det::max(actual.abs(), expected.abs()))
    }
}
