// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/Profile.hpp, src/lighting/Profile.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Light profiles: the internal table, its sampling and the parametric builder (P9).
//!
//! A profile is a normalized intensity table over polar angle `0..=180` degrees (181 samples)
//! and azimuth `0..360` degrees in 5 degree steps (72 rows). The shaders sample the atlas with
//! the same algorithm as [`sample_profile_table`].
//!
//! Redesign against the C++ version: `CHI_ProfileShape` is [`ProfileShape`] and
//! `CHI_ProfilePreset` is [`ProfilePreset`]. An unknown preset cannot be expressed. Errors are
//! [`ProfileError`] values with stable diagnostic codes `knorke.light2d.profile.*`; the texts keep
//! the C++ wording without the "Chiaro: " prefix. Maths that influences results uses
//! [`knorke_math::det`] (ADR 0011). Nothing here panics on bad input.

use knorke_math::det;
use knorke_types::{Diagnostic, DiagnosticCode};

/// Polar samples per profile: 0, 1, ..., 180 degrees.
pub const PROFILE_POLAR: usize = 181;
/// Azimuth samples per profile: 0, 5, ..., 355 degrees, periodic.
pub const PROFILE_AZIMUTH: usize = 72;
/// Azimuth step of the table in degrees (5).
pub const PROFILE_AZIMUTH_STEP: f32 = 360.0 / PROFILE_AZIMUTH as f32;
/// Values of one table, row-major by azimuth: `values[j * PROFILE_POLAR + i]` is (i degrees, j * 5 degrees).
pub const PROFILE_VALUES: usize = PROFILE_POLAR * PROFILE_AZIMUTH;

const PI: f32 = core::f32::consts::PI;
use crate::light_math::{DEG_TO_RAD, smoothstep};
const RAD_TO_DEG: f32 = 180.0 / PI;

/// Atlas blocks for profiles up to block `block_count - 1`: a power of two, at least 4, at most
/// 128; 0 for 0.
pub fn profile_atlas_blocks(block_count: u32) -> u32 {
    if block_count == 0 {
        return 0;
    }
    let mut blocks = 4u32;
    while blocks < block_count && blocks < 128 {
        blocks *= 2;
    }
    blocks
}

/// One sample of a table; a short slice reads as 0 instead of panicking.
fn at(values: &[f32], polar: usize, azimuth: usize) -> f32 {
    values
        .get(azimuth * PROFILE_POLAR + polar)
        .copied()
        .unwrap_or(0.0)
}

/// Bilinear sample of a table; polar clamped to `[0, 180]`, azimuth periodic.
///
/// `values` must hold [`PROFILE_VALUES`] entries; a shorter slice reads as zeros.
pub fn sample_profile_table(values: &[f32], polar_deg: f32, azimuth_deg: f32) -> f32 {
    let u = polar_deg.clamp(0.0, 180.0);
    let i0 = (u as i32).min(PROFILE_POLAR as i32 - 2) as usize;
    let f = u - i0 as f32;
    let v = azimuth_deg / PROFILE_AZIMUTH_STEP;
    let vf = v.floor();
    let g = v - vf;
    let mut j0 = (vf % PROFILE_AZIMUTH as f32) as i32;
    if j0 < 0 {
        j0 += PROFILE_AZIMUTH as i32;
    }
    let j0 = j0 as usize;
    let j1 = (j0 + 1) % PROFILE_AZIMUTH;
    let a = at(values, i0, j0) + (at(values, i0 + 1, j0) - at(values, i0, j0)) * f;
    let b = at(values, i0, j1) + (at(values, i0 + 1, j1) - at(values, i0, j1)) * f;
    a + (b - a) * g
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The C0 unit vector of a light with unit `axis`: world +x (or +y for an axis within ~8 degrees
/// of x) made perpendicular to the axis, then turned by `rotation_deg` towards C90 = C0 x axis.
pub fn profile_reference(axis: [f32; 3], rotation_deg: f32) -> [f32; 3] {
    let base = if axis[0].abs() > 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let d = dot(base, axis);
    let mut r = [
        base[0] - d * axis[0],
        base[1] - d * axis[1],
        base[2] - d * axis[2],
    ];
    let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    for x in &mut r {
        *x /= l;
    }
    // C90 = C0 x axis
    let b = cross(r, axis);
    let (s, c) = det::sin_cos(rotation_deg * DEG_TO_RAD);
    [
        r[0] * c + b[0] * s,
        r[1] * c + b[1] * s,
        r[2] * c + b[2] * s,
    ]
}

/// The profile value towards `to_pixel` (light to pixel, any length) for unit `axis` and C0
/// `reference`.
pub fn profile_factor(
    values: &[f32],
    axis: [f32; 3],
    reference: [f32; 3],
    to_pixel: [f32; 3],
) -> f32 {
    let l =
        (to_pixel[0] * to_pixel[0] + to_pixel[1] * to_pixel[1] + to_pixel[2] * to_pixel[2]).sqrt();
    if l <= 1e-6 {
        return sample_profile_table(values, 0.0, 0.0);
    }
    let w = [to_pixel[0] / l, to_pixel[1] / l, to_pixel[2] / l];
    let cos_polar = dot(w, axis).clamp(-1.0, 1.0);
    let b = cross(reference, axis);
    let x = dot(w, reference);
    let y = dot(w, b);
    let mut azimuth = det::atan2(y, x) * RAD_TO_DEG;
    if azimuth < 0.0 {
        azimuth += 360.0;
    }
    sample_profile_table(values, det::acos(cos_polar) * RAD_TO_DEG, azimuth)
}

/// The presets of a luminaire type (`CHI_ProfilePreset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfilePreset {
    /// Recessed downlight: brightest along the axis, dark beyond 60 degrees.
    Downlight,
    /// Hat lamp: strong to the side, cut off just above the horizontal, symmetric.
    HatLamp,
    /// LED street lamp: batwing along the street, full cutoff, little to the house side.
    LedStreet,
}

/// The parametric shape of a profile (`CHI_ProfileShape`), in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileShape {
    /// Polar angle of the lobe peak in the C0 plane.
    pub peak_angle: f32,
    /// Polar angle of the lobe peak in the C90 plane.
    pub peak_angle_c90: f32,
    /// Width of the lobe (Gaussian sigma), `(0, 180]`.
    pub spread_angle: f32,
    /// Intensity beyond the peak (towards the axis), relative to the peak, `[0, 1]`.
    pub nadir: f32,
    /// Polar angle where the light ends, `(0, 180]`.
    pub cutoff_angle: f32,
    /// Width of the soft cutoff before `cutoff_angle`, `[0, cutoff_angle]`; 0 is a hard cutoff.
    pub cutoff_fade_angle: f32,
    /// Intensity per quadrant (C0, C90, C180, C270), each `>= 0`, at least one `> 0`.
    pub gain: [f32; 4],
}

impl ProfileShape {
    /// The shape of a preset.
    pub fn preset(preset: ProfilePreset) -> Self {
        struct Row {
            peak: f32,
            peak_c90: f32,
            spread: f32,
            nadir: f32,
            cutoff: f32,
            softness: f32,
            gain: [f32; 4],
        }
        const DOWNLIGHT: Row = Row {
            peak: 0.0,
            peak_c90: 0.0,
            spread: 25.0,
            nadir: 1.0,
            cutoff: 60.0,
            softness: 20.0,
            gain: [1.0, 1.0, 1.0, 1.0],
        };
        const HAT: Row = Row {
            peak: 72.0,
            peak_c90: 72.0,
            spread: 22.0,
            nadir: 0.55,
            cutoff: 97.0,
            softness: 6.0,
            gain: [1.0, 1.0, 1.0, 1.0],
        };
        const LED: Row = Row {
            peak: 68.0,
            peak_c90: 35.0,
            spread: 12.0,
            nadir: 0.3,
            cutoff: 84.0,
            softness: 8.0,
            gain: [1.0, 0.8, 1.0, 0.15],
        };
        let r = match preset {
            ProfilePreset::Downlight => &DOWNLIGHT,
            ProfilePreset::HatLamp => &HAT,
            ProfilePreset::LedStreet => &LED,
        };
        Self {
            peak_angle: r.peak,
            peak_angle_c90: r.peak_c90,
            spread_angle: r.spread,
            nadir: r.nadir,
            cutoff_angle: r.cutoff,
            cutoff_fade_angle: r.softness,
            gain: r.gain,
        }
    }
}

/// Why a profile shape, table or store operation was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileError {
    /// A shape field is NaN, infinite or outside its range.
    ShapeOutOfRange {
        /// Field name, as in [`ProfileShape`].
        field: &'static str,
        /// Lower bound.
        lo: f32,
        /// Upper bound.
        hi: f32,
        /// `true` if `lo` itself is excluded.
        open_low: bool,
        /// The value received.
        got: f32,
    },
    /// A gain is NaN, infinite or negative.
    ShapeGain {
        /// The four gains as received.
        gain: [f32; 4],
    },
    /// All gains are 0.
    ShapeNoGain,
    /// The shape gives no light (its peak lies beyond the cutoff without a nadir).
    ShapeNoLight,
    /// A table holds an infinite intensity.
    NotFinite {
        /// What the table describes, for the message.
        what: String,
    },
    /// A table has no light at all.
    NoLight {
        /// What the table describes, for the message.
        what: String,
    },
    /// Measured planes are empty or do not match their gamma angles.
    PlanesInconsistent {
        /// What the planes describe, for the message.
        what: String,
    },
    /// A table does not have [`PROFILE_VALUES`] entries.
    WrongSize {
        /// The number of entries received.
        got: usize,
    },
    /// The store already holds `max` live profiles.
    TooManyProfiles {
        /// The limit.
        max: usize,
    },
    /// A handle is stale or was never issued by this store.
    InvalidHandle {
        /// The handle bits.
        bits: u64,
    },
    /// Lights still use the profile.
    InUse {
        /// How many lights.
        users: usize,
    },
}

impl ProfileError {
    /// The stable diagnostic code (`knorke.light2d.profile.*`).
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::ShapeOutOfRange { .. } | Self::ShapeGain { .. } | Self::ShapeNoGain => {
                DiagnosticCode::new("knorke.light2d.profile.shape")
            }
            Self::ShapeNoLight | Self::NoLight { .. } => {
                DiagnosticCode::new("knorke.light2d.profile.no_light")
            }
            Self::NotFinite { .. } => DiagnosticCode::new("knorke.light2d.profile.not_finite"),
            Self::PlanesInconsistent { .. } => DiagnosticCode::new("knorke.light2d.profile.planes"),
            Self::WrongSize { .. } => DiagnosticCode::new("knorke.light2d.profile.table_size"),
            Self::TooManyProfiles { .. } => DiagnosticCode::new("knorke.light2d.profile.too_many"),
            Self::InvalidHandle { .. } => {
                DiagnosticCode::new("knorke.light2d.profile.invalid_handle")
            }
            Self::InUse { .. } => DiagnosticCode::new("knorke.light2d.profile.in_use"),
        }
    }

    /// As a diagnostic with its stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.to_string())
    }
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShapeOutOfRange {
                field,
                lo,
                hi,
                open_low,
                got,
            } => write!(
                f,
                "profile shape.{field} must be in {}{lo}, {hi}] (got {got})",
                if *open_low { "(" } else { "[" }
            ),
            Self::ShapeGain { gain } => write!(
                f,
                "profile shape.gain values must be finite and >= 0 (got {}, {}, {}, {})",
                gain[0], gain[1], gain[2], gain[3]
            ),
            Self::ShapeNoGain => write!(f, "profile shape.gain needs at least one value > 0"),
            Self::ShapeNoLight => write!(
                f,
                "profile shape gives no light (peak beyond the cutoff without nadir)"
            ),
            Self::NotFinite { what } => write!(f, "{what} has an infinite intensity"),
            Self::NoLight { what } => {
                write!(f, "{what} has no light (all intensities are 0)")
            }
            Self::PlanesInconsistent { what } => {
                write!(
                    f,
                    "{what}: measured planes are empty or do not match their gamma angles"
                )
            }
            Self::WrongSize { got } => {
                write!(
                    f,
                    "a profile table needs {PROFILE_VALUES} values (got {got})"
                )
            }
            Self::TooManyProfiles { max } => {
                write!(f, "too many light profiles (at most {max} alive at once)")
            }
            Self::InvalidHandle { bits } => {
                write!(f, "invalid or destroyed profile handle 0x{bits:08x}")
            }
            Self::InUse { users } => write!(
                f,
                "the profile is still used by {users} light(s); set their profile to None or \
                 destroy them first"
            ),
        }
    }
}

impl std::error::Error for ProfileError {}

/// Shape value before normalization at polar `g` and azimuth `c` (degrees).
fn shape_value(s: &ProfileShape, g: f32, c: f32) -> f32 {
    let sc = det::sin(c * DEG_TO_RAD);
    let peak = s.peak_angle + (s.peak_angle_c90 - s.peak_angle) * sc * sc;
    let x = (g - peak) / s.spread_angle;
    let lobe = det::exp(-0.5 * x * x);
    let mut v = if g < peak {
        s.nadir + (1.0 - s.nadir) * lobe
    } else {
        lobe
    };
    if s.cutoff_fade_angle > 0.0 {
        v *= 1.0 - smoothstep(s.cutoff_angle - s.cutoff_fade_angle, s.cutoff_angle, g);
    } else if g > s.cutoff_angle {
        v = 0.0;
    }
    let quadrant = c / 90.0;
    let k = (quadrant.floor() as i32) & 3;
    let t = quadrant - quadrant.floor();
    let w = 0.5 - 0.5 * det::cos(PI * t);
    let k = k as usize;
    v * (s.gain[k] * (1.0 - w) + s.gain[(k + 1) & 3] * w)
}

fn in_range(
    field: &'static str,
    v: f32,
    lo: f32,
    hi: f32,
    open_low: bool,
) -> Result<(), ProfileError> {
    if !v.is_finite() || v > hi || (if open_low { v <= lo } else { v < lo }) {
        return Err(ProfileError::ShapeOutOfRange {
            field,
            lo,
            hi,
            open_low,
            got: v,
        });
    }
    Ok(())
}

/// Scales a table so its maximum is 1; an error if it has no light (or an infinite intensity).
/// `what` names the table in the message.
pub fn normalize_profile(values: &mut [f32], what: &str) -> Result<(), ProfileError> {
    let mut peak = 0.0f32;
    for &v in values.iter() {
        peak = det::max(peak, v);
    }
    if !peak.is_finite() {
        return Err(ProfileError::NotFinite {
            what: what.to_owned(),
        });
    }
    if !(peak > 0.0) {
        return Err(ProfileError::NoLight {
            what: what.to_owned(),
        });
    }
    for v in values.iter_mut() {
        *v /= peak;
    }
    Ok(())
}

/// Evaluates a shape into a normalized table of [`PROFILE_VALUES`] entries. An invalid shape
/// gives an error and no table.
pub fn build_shape_profile(s: &ProfileShape) -> Result<Vec<f32>, ProfileError> {
    in_range("peak_angle", s.peak_angle, 0.0, 180.0, false)?;
    in_range("peak_angle_c90", s.peak_angle_c90, 0.0, 180.0, false)?;
    in_range("spread_angle", s.spread_angle, 0.0, 180.0, true)?;
    in_range("nadir", s.nadir, 0.0, 1.0, false)?;
    in_range("cutoff_angle", s.cutoff_angle, 0.0, 180.0, true)?;
    in_range(
        "cutoff_fade_angle",
        s.cutoff_fade_angle,
        0.0,
        s.cutoff_angle,
        false,
    )?;
    let mut max_gain = 0.0f32;
    for &g in &s.gain {
        if !g.is_finite() || g < 0.0 {
            return Err(ProfileError::ShapeGain { gain: s.gain });
        }
        max_gain = det::max(max_gain, g);
    }
    if max_gain <= 0.0 {
        return Err(ProfileError::ShapeNoGain);
    }
    let mut values = vec![0.0f32; PROFILE_VALUES];
    let mut peak = 0.0f32;
    for j in 0..PROFILE_AZIMUTH {
        for i in 0..PROFILE_POLAR {
            let v = shape_value(s, i as f32, j as f32 * PROFILE_AZIMUTH_STEP);
            values[j * PROFILE_POLAR + i] = v;
            peak = det::max(peak, v);
        }
    }
    if peak <= 1e-6 * max_gain {
        return Err(ProfileError::ShapeNoLight);
    }
    normalize_profile(&mut values, "profile shape")?;
    Ok(values)
}
