// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/core/OutputRect.hpp and OutputRect.cpp), zlib licence:
//
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

//! Where the scene lands inside the target for a scale mode (ported from `OutputRect.cpp`).
//!
//! The C++ original takes `CHI_ScaleMode`; here the same three modes are the local [`ScaleMode`] enum
//! (values in the order of the C API, `STRETCH = 0`, `FIT = 1`, `INTEGER = 2`).

/// Destination rectangle of the scene inside the target, in target pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width, at least 1.
    pub w: i32,
    /// Height, at least 1.
    pub h: i32,
}

/// Scale mode of the scene in the target (the same as `CHI_ScaleMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    /// Fill the whole target, ignoring the aspect ratio.
    Stretch,
    /// Largest size that keeps the aspect ratio, centred.
    Fit,
    /// Largest whole-number factor, centred; falls back to [`ScaleMode::Fit`] if the target is smaller
    /// than the scene.
    Integer,
}

/// Computes where the scene lands in the target for `mode`.
///
/// All sizes must be above 0. The result is centred (offsets rounded down) and at least 1x1.
pub fn compute_output_rect(
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    mode: ScaleMode,
) -> OutputRect {
    if mode == ScaleMode::Stretch {
        return OutputRect {
            x: 0,
            y: 0,
            w: dst_w as i32,
            h: dst_h as i32,
        };
    }

    // f64 like the original; written out because clippy forbids f64::min.
    let rx = f64::from(dst_w) / f64::from(src_w);
    let ry = f64::from(dst_h) / f64::from(src_h);
    let scale = if ry < rx { ry } else { rx };

    if mode == ScaleMode::Integer {
        let whole = scale.floor();
        if whole >= 1.0 {
            let factor = whole as i64;
            return centered(
                dst_w,
                dst_h,
                i64::from(src_w) * factor,
                i64::from(src_h) * factor,
            );
        }
        // Target smaller than the scene: no whole factor exists, use FIT.
    }

    centered(
        dst_w,
        dst_h,
        (f64::from(src_w) * scale).round() as i64,
        (f64::from(src_h) * scale).round() as i64,
    )
}

/// Centres a `w` x `h` rect (clamped to at least 1x1 and at most the target) in the target.
fn centered(dst_w: u32, dst_h: u32, w: i64, h: i64) -> OutputRect {
    // Written out instead of `i64::clamp`, which panics for an empty range (a zero target size).
    let cw = (if w < 1 { 1 } else { w.min(i64::from(dst_w)) }) as i32;
    let ch = (if h < 1 { 1 } else { h.min(i64::from(dst_h)) }) as i32;
    OutputRect {
        x: (dst_w as i32 - cw) / 2,
        y: (dst_h as i32 - ch) / 2,
        w: cw,
        h: ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of tests/unit/OutputRectTests.cpp.

    #[test]
    fn stretch_fills_the_target() {
        assert_eq!(
            compute_output_rect(320, 180, 1280, 720, ScaleMode::Stretch),
            OutputRect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            }
        );
        assert_eq!(
            compute_output_rect(320, 180, 1000, 1000, ScaleMode::Stretch),
            OutputRect {
                x: 0,
                y: 0,
                w: 1000,
                h: 1000
            }
        );
    }

    #[test]
    fn fit_keeps_the_aspect_ratio_and_centers() {
        assert_eq!(
            compute_output_rect(320, 180, 1000, 1000, ScaleMode::Fit),
            OutputRect {
                x: 0,
                y: 218,
                w: 1000,
                h: 563
            }
        );
        assert_eq!(
            compute_output_rect(320, 180, 1280, 720, ScaleMode::Fit),
            OutputRect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            }
        );
    }

    #[test]
    fn integer_scaling_uses_the_largest_whole_factor() {
        assert_eq!(
            compute_output_rect(320, 180, 1280, 720, ScaleMode::Integer),
            OutputRect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            }
        );
        assert_eq!(
            compute_output_rect(320, 180, 1000, 1000, ScaleMode::Integer),
            OutputRect {
                x: 20,
                y: 230,
                w: 960,
                h: 540
            }
        );
        assert_eq!(
            compute_output_rect(2, 2, 10, 6, ScaleMode::Integer),
            OutputRect {
                x: 2,
                y: 0,
                w: 6,
                h: 6
            }
        );
    }

    #[test]
    fn integer_scaling_falls_back_to_fit_when_the_target_is_smaller_than_the_scene() {
        assert_eq!(
            compute_output_rect(320, 180, 200, 100, ScaleMode::Integer),
            OutputRect {
                x: 11,
                y: 0,
                w: 178,
                h: 100
            }
        );
    }

    #[test]
    fn extreme_aspect_ratios_never_produce_an_empty_rect() {
        assert_eq!(
            compute_output_rect(1000, 1, 10, 10, ScaleMode::Fit),
            OutputRect {
                x: 0,
                y: 4,
                w: 10,
                h: 1
            }
        );
        assert_eq!(
            compute_output_rect(1, 1000, 10, 10, ScaleMode::Integer),
            OutputRect {
                x: 4,
                y: 0,
                w: 1,
                h: 10
            }
        );
    }
}
