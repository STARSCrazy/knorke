//! Deterministic floating-point functions (ADR 0011).
//!
//! `f32::sin`, `cos`, `exp`, `powf`, … call the platform's math library; results can differ
//! in the last bit between Windows, Linux, macOS, ARM and WASM. For simulation code that
//! must give bit-identical results everywhere (replays across machines, lockstep/rollback
//! netcode) use these functions instead: they are pure-Rust implementations (`libm`, a
//! port of musl) and produce the same bits on every platform.
//!
//! Basic arithmetic (`+ - * /`), `sqrt`, `abs`, `clamp`, `floor`/`ceil`/`round` and
//! `mul_add` are IEEE-754 exact and already deterministic. `mul_add` is only fast where the
//! target has a fused multiply-add instruction; the default x86_64 target has none and calls
//! a software routine, so prefer `a * b + c` unless the extra precision is needed.
//!
//! **Not** deterministic are `f32::min`/`f32::max`: for `-0.0` against `+0.0` either may be
//! returned, and the choice differs between platforms and even between debug and release
//! builds on the same machine. Use [`min`]/[`max`] from this module.
//!
//! Presentation code (rendering, audio, UI) may use `std` freely.
//!
//! The Clippy configuration (`clippy.toml`) forbids the `std` variants in the workspace so
//! simulation code cannot use them by accident.

macro_rules! unary {
    ($($(#[$doc:meta])* $name:ident => $f32:ident;)+) => {$(
        $(#[$doc])*
        #[inline]
        pub fn $name(x: f32) -> f32 {
            libm::$f32(x)
        }
    )+};
}

unary! {
    /// Sine (radians).
    sin => sinf;
    /// Cosine (radians).
    cos => cosf;
    /// Tangent (radians).
    tan => tanf;
    /// Arc sine.
    asin => asinf;
    /// Arc cosine.
    acos => acosf;
    /// Arc tangent.
    atan => atanf;
    /// `e^x`.
    exp => expf;
    /// `2^x`.
    exp2 => exp2f;
    /// Natural logarithm.
    ln => logf;
    /// Base-2 logarithm.
    log2 => log2f;
    /// Base-10 logarithm.
    log10 => log10f;
    /// Cube root.
    cbrt => cbrtf;
    /// Hyperbolic tangent.
    tanh => tanhf;
}

/// Sine and cosine at once.
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}

/// Four-quadrant arc tangent of `y / x`.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// `x^y`.
#[inline]
pub fn pow(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// `sqrt(x² + y²)` without intermediate overflow.
#[inline]
pub fn hypot(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
}

/// Integer power by repeated squaring in a fixed order (unlike `f32::powi`, whose
/// evaluation order is up to the compiler).
///
/// Negative exponents give `1 / base^|exp|`; if that power overflows or underflows, the
/// reciprocal of the base is raised instead, so `powi(10.0, -40)` is a subnormal number
/// and not zero. Defined for every `exp` including `i32::MIN`.
#[inline]
pub fn powi(base: f32, exp: i32) -> f32 {
    #[inline]
    fn pow_abs(mut base: f32, mut e: u32) -> f32 {
        let mut acc = 1.0f32;
        while e > 0 {
            if e & 1 == 1 {
                acc *= base;
            }
            base *= base;
            e >>= 1;
        }
        acc
    }
    let e = exp.unsigned_abs();
    let acc = pow_abs(base, e);
    if exp >= 0 {
        acc
    } else if acc.is_finite() && acc != 0.0 {
        1.0 / acc
    } else {
        pow_abs(1.0 / base, e)
    }
}

/// The larger value. Unlike `f32::max`, the result is the same on every platform and
/// optimisation level: `+0.0` is larger than `-0.0`, and a NaN is ignored unless both
/// values are NaN.
#[inline]
pub fn max(a: f32, b: f32) -> f32 {
    if a > b || b.is_nan() {
        a
    } else if b > a || a.is_nan() || a.is_sign_negative() {
        b
    } else {
        a
    }
}

/// The smaller value. Unlike `f32::min`, the result is the same on every platform and
/// optimisation level: `-0.0` is smaller than `+0.0`, and a NaN is ignored unless both
/// values are NaN.
#[inline]
pub fn min(a: f32, b: f32) -> f32 {
    if a < b || b.is_nan() {
        a
    } else if b < a || a.is_nan() || b.is_sign_negative() {
        b
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_close_to_std() {
        for i in -200..200 {
            let x = i as f32 * 0.0731;
            #[allow(clippy::disallowed_methods)]
            let (s, c) = x.sin_cos();
            let (ds, dc) = sin_cos(x);
            assert!(
                (s - ds).abs() <= 2.0 * f32::EPSILON && (c - dc).abs() <= 2.0 * f32::EPSILON,
                "{x}"
            );
        }
        assert_eq!(powi(2.0, 10), 1024.0);
        assert_eq!(powi(2.0, -2), 0.25);
        assert_eq!(powi(3.0, 0), 1.0);
        assert!((pow(2.0, 0.5) - core::f32::consts::SQRT_2).abs() < 1e-6);
        assert!((atan2(1.0, -1.0) - 3.0 * core::f32::consts::FRAC_PI_4).abs() < 1e-6);
    }

    #[test]
    fn min_max_have_one_answer_for_signed_zeros_and_nan() {
        use core::hint::black_box as bb;
        // `f32::max(-0.0, 0.0)` is -0.0 or 0.0 depending on platform and optimisation level.
        for (a, b) in [(0.0f32, -0.0f32), (-0.0, 0.0)] {
            assert_eq!(max(bb(a), bb(b)).to_bits(), 0.0f32.to_bits());
            assert_eq!(min(bb(a), bb(b)).to_bits(), (-0.0f32).to_bits());
        }
        assert_eq!((max(1.0, 2.0), min(1.0, 2.0)), (2.0, 1.0));
        assert_eq!((max(f32::NAN, 2.0), max(2.0, f32::NAN)), (2.0, 2.0));
        assert_eq!((min(f32::NAN, 2.0), min(2.0, f32::NAN)), (2.0, 2.0));
        assert!(max(f32::NAN, f32::NAN).is_nan() && min(f32::NAN, f32::NAN).is_nan());
        assert_eq!(max(f32::NEG_INFINITY, -1.0), -1.0);
    }

    #[test]
    fn powi_extremes() {
        assert_eq!(powi(2.0, i32::MIN), 0.0);
        assert_eq!(powi(0.5, i32::MIN), f32::INFINITY);
        assert_eq!(powi(1.0, i32::MIN), 1.0);
        assert_eq!(powi(-1.0, i32::MAX), -1.0);
        assert_eq!(powi(0.0, 0), 1.0);
        assert_eq!(powi(0.0, -1), f32::INFINITY);
        assert_eq!(powi(2.0, 127), f32::MAX / (2.0 - f32::EPSILON));
        // The reciprocal is taken of the base, so results near the subnormal range do not
        // collapse to zero through an overflowing intermediate power.
        assert!(powi(10.0, -40) > 0.0);
        assert_eq!(powi(2.0, -140), f32::from_bits(1 << 9));
    }

    #[test]
    fn frozen_bits() {
        // Same bits on every platform: these values are part of the determinism contract
        // and checked by CI on Windows, Linux, macOS (ARM) and WASM.
        assert_eq!(sin(1.0).to_bits(), 0x3F57_6AA4);
        assert_eq!(exp(1.0).to_bits(), 0x402D_F854);
        assert_eq!(pow(1.5, 2.5).to_bits(), 0x4030_5CFF);
    }
}
