//! Post-processing math of the Knorke Engine, ported from Chiaro (ADR 0014, packages P4 and P17).
//!
//! This crate holds the CPU side of the output stage: colour transfer functions, tonemapping,
//! grading, the output rectangle, the CRT and bloom parameter math, the shockwave helpers and the
//! settings of these stages ([`settings`], P19).
//! It contains no GPU code and does not depend on any rendering or lighting crate.
//!
//! # Not ported (yet)
//!
//! - `Formats.*` (ADR 0014 P4): the format tables map `SDL_GPUTextureFormat` values. That enum belongs to the
//!   GPU wrapper (`knorke-gpu`, P1/P3), and this crate must not depend on SDL or a GPU crate. The table moves
//!   there with the GPU wrapper.
//! - Tests that need the C API structs (`CHI_CrtSettings`, `CHI_BloomSettings`, `CHI_ShockwaveDesc`,
//!   `CHI_OutputSettings`), the settings store, the SDL error strings, the output flags or the C++
//!   wrapper: the same checks run here on the Rust types (field names and values), the struct/flag/
//!   error-string parts are dropped.
//! - `CHI_Kelvin` / `CHI_PlanckRGB_` (colour temperature): not in P4 or P17. Shockwaves take the light colour
//!   from the caller, and [`shockwave::ExplosionParams::kelvin`] carries the temperature, so the colour-ratio
//!   checks of the C tests are not ported.
//! - `MediumCloud` tests in `OutputGradeTests.cpp`: a package of their own (E112).
//!
//! Simulation-safe math goes through `knorke_math::det` (clippy forbids the `std` variants).
//! Clamping uses [`clamp`], which mirrors C++ `std::clamp` and never panics, also for NaN bounds.

pub mod bloom;
pub mod color;
pub mod crt;
pub mod output_rect;
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
