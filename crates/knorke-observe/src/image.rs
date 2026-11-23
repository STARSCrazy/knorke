//! Cheap visual glitch heuristics on captured frames (plan section 17.9).
//!
//! Order of oracles by cost: simulation invariants first, then these pixel checks against a
//! reference frame of the same tick and seed, and only suspicious pairs go to an image model
//! for triage. Reference-based comparison is much more reliable than judging a frame alone.
//!
//! Frames are tightly packed RGBA8 (4 bytes per pixel) or linear float RGBA (`f32`).

/// Summary of one RGBA8 frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStats {
    /// Number of pixels.
    pub pixels: usize,
    /// Mean luma (Rec. 709 weights on the stored values), 0–255.
    pub mean_luma: f32,
    /// Every pixel has the same RGBA value (black/empty/clear-color frame).
    pub uniform: bool,
    /// Every pixel has alpha 0.
    pub fully_transparent: bool,
}

/// Statistics of an RGBA8 frame; `None` if the length is not a multiple of 4 or zero.
pub fn frame_stats(rgba8: &[u8]) -> Option<FrameStats> {
    if rgba8.is_empty() || !rgba8.len().is_multiple_of(4) {
        return None;
    }
    let first = &rgba8[..4];
    let mut uniform = true;
    let mut transparent = true;
    let mut luma = 0.0f64;
    for p in rgba8.as_chunks::<4>().0 {
        uniform &= p == first;
        transparent &= p[3] == 0;
        luma += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
    }
    let pixels = rgba8.len() / 4;
    Some(FrameStats {
        pixels,
        mean_luma: (luma / pixels as f64) as f32,
        uniform,
        fully_transparent: transparent,
    })
}

/// Difference between two RGBA8 frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameDiff {
    /// Pixels where any channel differs by more than the tolerance.
    pub changed: usize,
    /// Share of changed pixels, 0–1.
    pub changed_ratio: f32,
    /// Largest channel difference.
    pub max_delta: u8,
}

/// Compares two equally sized RGBA8 frames; `None` on size mismatch.
pub fn frame_diff(a: &[u8], b: &[u8], tolerance: u8) -> Option<FrameDiff> {
    if a.len() != b.len() || a.is_empty() || !a.len().is_multiple_of(4) {
        return None;
    }
    let mut changed = 0;
    let mut max_delta = 0u8;
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        max_delta = max_delta.max(d);
        if d > tolerance {
            changed += 1;
        }
    }
    Some(FrameDiff {
        changed,
        changed_ratio: changed as f32 / (a.len() / 4) as f32,
        max_delta,
    })
}

/// Flicker score of three consecutive frames: high if the middle frame differs strongly from
/// both neighbours while the neighbours are similar (something popped in for one frame).
/// Returns the changed ratio of the middle frame against its neighbours, or 0 if the
/// neighbours themselves differ by more than `stable_ratio` (real motion).
pub fn flicker_score(
    prev: &[u8],
    cur: &[u8],
    next: &[u8],
    tolerance: u8,
    stable_ratio: f32,
) -> Option<f32> {
    let around = frame_diff(prev, next, tolerance)?;
    if around.changed_ratio > stable_ratio {
        return Some(0.0);
    }
    let a = frame_diff(prev, cur, tolerance)?;
    let b = frame_diff(cur, next, tolerance)?;
    Some(if b.changed_ratio < a.changed_ratio {
        b.changed_ratio
    } else {
        a.changed_ratio
    })
}

/// Number of NaN/infinite values in a float render target (e.g. HDR lighting buffers).
/// Any non-zero count is a bug: NaNs spread through blur and bloom passes.
pub fn non_finite(values: &[f32]) -> usize {
    values.iter().filter(|v| !v.is_finite()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn frame(px: &[[u8; 4]]) -> Vec<u8> {
        px.iter().flatten().copied().collect()
    }

    #[test]
    fn stats_detect_blank_and_transparent_frames() {
        let black = [0u8, 0, 0, 255].repeat(16);
        let s = frame_stats(&black).unwrap();
        assert!(s.uniform && !s.fully_transparent && s.mean_luma == 0.0);
        let clear = vec![0u8; 64];
        assert!(frame_stats(&clear).unwrap().fully_transparent);
        let mixed = frame(&[[255, 255, 255, 255], [0, 0, 0, 255]]);
        let s = frame_stats(&mixed).unwrap();
        assert!(!s.uniform);
        assert!((s.mean_luma - 127.5).abs() < 0.01);
        assert!(frame_stats(&[1, 2, 3]).is_none());
    }

    #[test]
    fn diff_respects_tolerance() {
        let a = frame(&[[10, 10, 10, 255], [10, 10, 10, 255]]);
        let b = frame(&[[12, 10, 10, 255], [90, 10, 10, 255]]);
        let d = frame_diff(&a, &b, 3).unwrap();
        assert_eq!((d.changed, d.max_delta), (1, 80));
        assert_eq!(d.changed_ratio, 0.5);
        assert!(frame_diff(&a, &a[..4], 0).is_none());
    }

    #[test]
    fn flicker_is_separated_from_motion() {
        let calm = frame(&[[0, 0, 0, 255]; 4]);
        let pop = frame(&[
            [255, 0, 255, 255],
            [255, 0, 255, 255],
            [0, 0, 0, 255],
            [0, 0, 0, 255],
        ]);
        assert_eq!(flicker_score(&calm, &pop, &calm, 4, 0.1), Some(0.5));
        let moved = frame(&[[255, 255, 255, 255]; 4]);
        assert_eq!(
            flicker_score(&calm, &pop, &moved, 4, 0.1),
            Some(0.0),
            "neighbours differ: real change"
        );
    }

    #[test]
    fn non_finite_counts_nan_and_inf() {
        assert_eq!(non_finite(&[0.0, f32::NAN, 1.0, f32::NEG_INFINITY]), 2);
    }
}
