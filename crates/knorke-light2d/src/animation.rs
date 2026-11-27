// Origin: chiaro @ 9bc5b9b, src/lighting/Animation.hpp and Animation.cpp (curves, validation,
// RotateAboutZ), src/lighting/AnimationStore.hpp and AnimationStore.cpp (AnimationStore), and the
// field list of CHI_AnimationDesc in include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Light animation (P8): curves that are evaluated on the CPU once per frame and light. They only
//! change the colour, intensity and direction of a light, never anything per pixel.
//!
//! Redesign against the C++ version:
//!
//! - [`AnimationType`] is an enum, so an "unknown type" cannot exist; `CHI_AnimationDesc` is
//!   [`AnimationDesc`] with typed fields, and the sample is [`AnimationSample`] with a [`Vec3`].
//! - [`AnimationStore`] is built on [`Slots`]; its handles are generation-checked
//!   [`AnimationId`]s. The C++ context refused to destroy an animation that lights still use; the
//!   store does not know the lights, so [`AnimationStore::destroy`] takes the [`LightStore`].
//! - Errors are [`AnimationError`] values with stable diagnostic codes
//!   `knorke.light2d.animation.*` instead of SDL error strings.
//! - The pulse uses [`det::cos`] on an `f32` angle (the C++ used `double` `std::cos`), so results
//!   are identical on every platform; they differ from the C++ values by about 1e-7.
//! - The store is simulation state and implements [`Snapshot`] (`knorke.light2d.animations`):
//!   the section keeps the slots, so handles stay identical, and descriptions are re-validated on
//!   restore (`SnapshotError::Invalid`).

use crate::LightStore;
use crate::light_math::flicker_noise;
use knorke_math::{Vec3, det};
use knorke_types::{
    Diagnostic, DiagnosticCode, Handle, Reader, Slots, Snapshot, SnapshotError, SnapshotValue,
    Writer,
};

const TWO_PI: f32 = core::f32::consts::TAU;
const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

/// Handle of a light animation in an [`AnimationStore`].
pub type AnimationId = Handle<AnimationDesc>;

/// Kinds of light animation. Each gives a curve value `w` in `[0, 1]` over the light's local time
/// `u = time + animation_offset - start`, with the phase `p = frac(u / period)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimationType {
    /// Smooth pulse, `w = (1 - cos(2 pi p)) / 2`: warning lights, breathing glow.
    Pulse,
    /// `w = 1` during `count` flashes of `duty * period` each, starting every `2 * duty * period`
    /// from `p = 0`, else 0: blinkers, strobes, double flashes.
    Flash,
    /// Rotating beacon: `w = 1`; direction and profile turn by `360 * p` degrees counter-clockwise
    /// about world z (seen from above).
    Beacon,
    /// Deterministic value noise with cells of `period` seconds, different for every light;
    /// dimmer moments take on `low_color`.
    Fire,
    /// `w` rises linearly over `attack`, stays 1 for `hold`, falls as `(1 - x)^2` over `decay`,
    /// then 0. `period` 0: once from `start` (0 before); `period > 0`: repeated. Explosions,
    /// muzzle flashes, lightning.
    Envelope,
}

impl AnimationType {
    /// Every type, in declaration order.
    pub const ALL: [AnimationType; 5] = [
        Self::Pulse,
        Self::Flash,
        Self::Beacon,
        Self::Fire,
        Self::Envelope,
    ];

    fn tag(self) -> u8 {
        match self {
            Self::Pulse => 0,
            Self::Flash => 1,
            Self::Beacon => 2,
            Self::Fire => 3,
            Self::Envelope => 4,
        }
    }

    fn from_tag(t: u8) -> Result<Self, SnapshotError> {
        Self::ALL
            .get(usize::from(t))
            .copied()
            .ok_or(SnapshotError::Invalid("animation type"))
    }
}

/// A light animation.
///
/// The light is scaled by `minimum + (1 - minimum) * w` in intensity and by
/// `low_color + (1 - low_color) * w` per colour channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationDesc {
    /// Seconds of phase 0; for a one-shot envelope the trigger time. Finite. Default 0.
    pub start: f64,
    /// Kind of curve.
    pub kind: AnimationType,
    /// Seconds of one cycle (beacon: one turn; fire: one noise cell), `> 0`. Envelope: 0 plays
    /// once, otherwise `>= attack + hold + decay`.
    pub period: f32,
    /// Intensity factor at `w = 0`, `[0, 1]`.
    pub minimum: f32,
    /// Colour factor at `w = 0`, each `>= 0`; `(1, 1, 1)` keeps the colour.
    pub low_color: Vec3,
    /// Flash: share of the period one flash lasts, `(0, 1]`.
    pub duty: f32,
    /// Flash: flashes per period, `>= 1` with `(2 * count - 1) * duty <= 1`.
    pub count: u32,
    /// Envelope: seconds to rise, `>= 0`.
    pub attack: f32,
    /// Envelope: seconds at full, `>= 0`.
    pub hold: f32,
    /// Envelope: seconds to fade, `>= 0`; `attack + hold + decay > 0`.
    pub decay: f32,
}

/// The curve value and rotation at one local time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationCurve {
    /// The curve value in `[0, 1]`.
    pub w: f32,
    /// Degrees the direction and profile are turned about world z.
    pub rotation: f32,
}

/// What an animation does to one light at one time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationSample {
    /// Factor on the intensity.
    pub intensity: f32,
    /// Factor on each colour channel.
    pub color: Vec3,
    /// Degrees the direction and profile are turned about world z.
    pub rotation: f32,
}

impl AnimationSample {
    /// The identity sample: intensity 1, colour factor `(1, 1, 1)`, no rotation.
    pub const IDENTITY: Self = Self {
        intensity: 1.0,
        color: Vec3::ONE,
        rotation: 0.0,
    };
}

/// `frac(u / period)` in `[0, 1)`, also for negative `u`.
fn phase(u: f64, period: f32) -> f64 {
    let x = u / f64::from(period);
    let p = x - x.floor();
    if p < 1.0 { p } else { 0.0 }
}

impl AnimationDesc {
    /// The defaults of a type; every field is filled, so the result is valid for every type.
    pub fn new(kind: AnimationType) -> Self {
        let mut d = Self {
            start: 0.0,
            kind,
            period: 1.0,
            minimum: 0.2,
            low_color: Vec3::ONE,
            duty: 0.1,
            count: 1,
            attack: 0.04,
            hold: 0.08,
            decay: 0.9,
        };
        match kind {
            AnimationType::Pulse => {}
            AnimationType::Flash => {
                d.minimum = 0.0;
                d.duty = 0.08;
            }
            AnimationType::Beacon => {
                d.period = 1.5;
                d.minimum = 1.0;
            }
            AnimationType::Fire => {
                d.period = 0.12;
                d.minimum = 0.55;
                d.low_color = Vec3::new(1.0, 0.55, 0.3);
            }
            AnimationType::Envelope => {
                d.period = 0.0;
                d.minimum = 0.0;
                d.low_color = Vec3::new(1.0, 0.35, 0.1);
            }
        }
        d
    }

    /// Checks that the curves can use the description.
    ///
    /// # Errors
    /// The first violated rule, as an [`AnimationError`].
    pub fn validate(&self) -> Result<(), AnimationError> {
        if !self.start.is_finite() {
            return Err(AnimationError::NotFinite {
                field: "start",
                value: self.start,
            });
        }
        let scalars: [(&'static str, f32); 9] = [
            ("period", self.period),
            ("minimum", self.minimum),
            ("duty", self.duty),
            ("attack", self.attack),
            ("hold", self.hold),
            ("decay", self.decay),
            ("low_color", self.low_color.x),
            ("low_color", self.low_color.y),
            ("low_color", self.low_color.z),
        ];
        for (field, v) in scalars {
            if !v.is_finite() {
                return Err(AnimationError::NotFinite {
                    field,
                    value: f64::from(v),
                });
            }
        }
        if !(0.0..=1.0).contains(&self.minimum) {
            return Err(range("minimum", "must be in [0, 1]", self.minimum));
        }
        let c = self.low_color;
        if let Some(v) = [c.x, c.y, c.z].into_iter().find(|v| *v < 0.0) {
            return Err(range("low_color", "must be >= 0", v));
        }
        if self.duty <= 0.0 || self.duty > 1.0 {
            return Err(range("duty", "must be in (0, 1]", self.duty));
        }
        if self.count < 1
            || (2 * u64::from(self.count) - 1) as f64 * f64::from(self.duty) > 1.0 + 1e-6
        {
            return Err(AnimationError::Count {
                count: self.count,
                duty: self.duty,
            });
        }
        if self.attack < 0.0
            || self.hold < 0.0
            || self.decay < 0.0
            || self.attack + self.hold + self.decay <= 0.0
        {
            return Err(AnimationError::Envelope {
                attack: self.attack,
                hold: self.hold,
                decay: self.decay,
            });
        }
        if self.kind == AnimationType::Envelope {
            let total = self.attack + self.hold + self.decay;
            if self.period < 0.0 || (self.period > 0.0 && self.period < total) {
                return Err(AnimationError::EnvelopePeriod {
                    period: self.period,
                    total,
                });
            }
        } else if self.period <= 0.0 {
            return Err(range("period", "must be > 0", self.period));
        }
        Ok(())
    }

    /// The curve value `w` in `[0, 1]` and the rotation in degrees at local time `u`
    /// (`time + offset - start`); `seed` decorrelates fire.
    pub fn curve(&self, u: f64, seed: u32) -> AnimationCurve {
        let mut c = AnimationCurve {
            w: 1.0,
            rotation: 0.0,
        };
        match self.kind {
            AnimationType::Pulse => {
                // f64 phase, f32 angle: the cosine comes from `det`.
                let angle = (f64::from(TWO_PI) * phase(u, self.period)) as f32;
                c.w = 0.5 - 0.5 * det::cos(angle);
            }
            AnimationType::Flash => {
                let p = phase(u, self.period);
                let spacing = 2.0 * f64::from(self.duty);
                let k = (p / spacing).floor();
                c.w = if k < f64::from(self.count) && p - k * spacing < f64::from(self.duty) {
                    1.0
                } else {
                    0.0
                };
            }
            AnimationType::Beacon => {
                c.rotation = (360.0 * phase(u, self.period)) as f32;
            }
            AnimationType::Fire => {
                let speed = 1.0 / self.period;
                c.w = 0.65 * flicker_noise(u, speed, seed)
                    + 0.35 * flicker_noise(u, 2.7 * speed, seed ^ 0x5bd1_e995);
                c.w = c.w.clamp(0.0, 1.0);
            }
            AnimationType::Envelope => {
                let mut t = u;
                if self.period > 0.0 {
                    t = phase(u, self.period) * f64::from(self.period);
                } else if t < 0.0 {
                    c.w = 0.0;
                    return c;
                }
                let (a, h, dec) = (
                    f64::from(self.attack),
                    f64::from(self.hold),
                    f64::from(self.decay),
                );
                c.w = if t < a {
                    (t / a) as f32
                } else if t < a + h {
                    1.0
                } else if t < a + h + dec {
                    let x = (t - a - h) / dec;
                    ((1.0 - x) * (1.0 - x)) as f32
                } else {
                    0.0
                };
            }
        }
        c
    }

    /// Intensity and colour factors and rotation of a light with `offset` at `time`; `seed`
    /// decorrelates fire (the slot index of the light).
    pub fn sample(&self, time: f64, offset: f32, seed: u32) -> AnimationSample {
        let c = self.curve(time + f64::from(offset) - self.start, seed);
        let low = self.low_color;
        AnimationSample {
            intensity: self.minimum + (1.0 - self.minimum) * c.w,
            color: Vec3::new(
                low.x + (1.0 - low.x) * c.w,
                low.y + (1.0 - low.y) * c.w,
                low.z + (1.0 - low.z) * c.w,
            ),
            rotation: c.rotation,
        }
    }
}

fn range(field: &'static str, rule: &'static str, value: f32) -> AnimationError {
    AnimationError::OutOfRange {
        field,
        rule,
        value: f64::from(value),
    }
}

/// Turns `v` by `degrees` counter-clockwise about world z.
pub fn rotate_about_z(v: Vec3, degrees: f32) -> Vec3 {
    let (s, c) = det::sin_cos(degrees * DEG_TO_RAD);
    Vec3::new(c * v.x - s * v.y, s * v.x + c * v.y, v.z)
}

/// Why an animation or an animation operation was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum AnimationError {
    /// A value is NaN or infinite.
    NotFinite {
        /// Field name of the description.
        field: &'static str,
        /// The received value.
        value: f64,
    },
    /// A value is outside its allowed range.
    OutOfRange {
        /// Field name of the description.
        field: &'static str,
        /// The rule, e.g. `must be > 0`.
        rule: &'static str,
        /// The received value.
        value: f64,
    },
    /// `count` is below 1 or `(2 * count - 1) * duty` exceeds 1.
    Count {
        /// Flashes per period.
        count: u32,
        /// Share of the period of one flash.
        duty: f32,
    },
    /// `attack`, `hold` and `decay` must be `>= 0` with a sum `> 0`.
    Envelope {
        /// Seconds to rise.
        attack: f32,
        /// Seconds at full.
        hold: f32,
        /// Seconds to fade.
        decay: f32,
    },
    /// An envelope's period is neither 0 (once) nor `>= attack + hold + decay`.
    EnvelopePeriod {
        /// The period.
        period: f32,
        /// `attack + hold + decay`.
        total: f32,
    },
    /// An animation handle that is stale or was never issued.
    InvalidHandle(u64),
    /// Lights still use the animation.
    InUse {
        /// How many lights.
        users: usize,
    },
}

impl AnimationError {
    /// The stable diagnostic code (`knorke.light2d.animation.*`).
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::NotFinite { .. } => DiagnosticCode::new("knorke.light2d.animation.not_finite"),
            Self::OutOfRange { .. }
            | Self::Count { .. }
            | Self::Envelope { .. }
            | Self::EnvelopePeriod { .. } => {
                DiagnosticCode::new("knorke.light2d.animation.out_of_range")
            }
            Self::InvalidHandle(_) => {
                DiagnosticCode::new("knorke.light2d.animation.invalid_handle")
            }
            Self::InUse { .. } => DiagnosticCode::new("knorke.light2d.animation.in_use"),
        }
    }

    /// As a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.to_string())
    }
}

impl core::fmt::Display for AnimationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFinite { field, value } => {
                write!(f, "animation desc.{field} must be finite (got {value})")
            }
            Self::OutOfRange { field, rule, value } => {
                write!(f, "animation desc.{field} {rule} (got {value})")
            }
            Self::Count { count, duty } => write!(
                f,
                "animation desc.count must be >= 1 with (2 * count - 1) * duty <= 1 \
                 (got {count}, {duty})"
            ),
            Self::Envelope {
                attack,
                hold,
                decay,
            } => write!(
                f,
                "animation desc.attack, hold and decay must be >= 0 with a sum > 0 \
                 (got {attack}, {hold}, {decay})"
            ),
            Self::EnvelopePeriod { period, total } => write!(
                f,
                "an envelope's desc.period must be 0 (once) or >= attack + hold + decay \
                 (got {period}, {total})"
            ),
            Self::InvalidHandle(bits) => {
                write!(f, "invalid or destroyed animation handle 0x{bits:016x}")
            }
            Self::InUse { users } => write!(
                f,
                "the animation is still used by {users} light(s); set their animation to None \
                 or destroy them first"
            ),
        }
    }
}

impl std::error::Error for AnimationError {}

/// The light animations of one scene. Every mutation validates first and leaves the state
/// unchanged on error.
#[derive(Clone, Debug, Default)]
pub struct AnimationStore {
    animations: Slots<AnimationDesc>,
}

impl AnimationStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of animations.
    pub fn len(&self) -> usize {
        self.animations.len()
    }

    /// `true` if there are no animations.
    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    /// Adds an animation.
    ///
    /// # Errors
    /// The description violates a rule.
    pub fn create(&mut self, desc: &AnimationDesc) -> Result<AnimationId, AnimationError> {
        desc.validate()?;
        Ok(self.animations.insert(*desc))
    }

    /// Replaces the description of an animation; every light that uses it follows from the next
    /// frame. Setting `start` to the current time retriggers a one-shot envelope.
    ///
    /// # Errors
    /// Stale handle, or a description that violates a rule; unchanged on error.
    pub fn update(&mut self, id: AnimationId, desc: &AnimationDesc) -> Result<(), AnimationError> {
        let stored = self.animations.get_mut(id).ok_or_else(|| invalid(id))?;
        desc.validate()?;
        *stored = *desc;
        Ok(())
    }

    /// The description of an animation.
    ///
    /// # Errors
    /// Stale handle.
    pub fn get(&self, id: AnimationId) -> Result<&AnimationDesc, AnimationError> {
        self.animations.get(id).ok_or_else(|| invalid(id))
    }

    /// The description, or `None` for a stale handle.
    pub fn find(&self, id: AnimationId) -> Option<&AnimationDesc> {
        self.animations.get(id)
    }

    /// `true` if `id` names a live animation.
    pub fn contains(&self, id: AnimationId) -> bool {
        self.animations.contains(id)
    }

    /// Removes an animation. Refused while a light of `lights` still uses it.
    ///
    /// # Errors
    /// Stale handle, or [`AnimationError::InUse`].
    pub fn destroy(&mut self, id: AnimationId, lights: &LightStore) -> Result<(), AnimationError> {
        if !self.animations.contains(id) {
            return Err(invalid(id));
        }
        let users = lights.animation_users(id);
        if users > 0 {
            return Err(AnimationError::InUse { users });
        }
        self.animations.remove(id);
        Ok(())
    }

    /// What `animation` does at `time` to a light with `offset`; `seed` (the slot index of the
    /// light, like flicker) decorrelates fire. The identity for `None` or a stale handle.
    pub fn sample(
        &self,
        animation: Option<AnimationId>,
        time: f64,
        offset: f32,
        seed: u32,
    ) -> AnimationSample {
        match animation.and_then(|id| self.animations.get(id)) {
            Some(desc) => desc.sample(time, offset, seed),
            None => AnimationSample::IDENTITY,
        }
    }
}

fn invalid(id: AnimationId) -> AnimationError {
    AnimationError::InvalidHandle(id.to_bits())
}

// ---- snapshots (ADR 0010) ----

impl SnapshotValue for AnimationDesc {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.start);
        w.put(&self.kind.tag());
        w.put(&self.period);
        w.put(&self.minimum);
        w.put(&self.low_color);
        w.put(&self.duty);
        w.put(&self.count);
        w.put(&self.attack);
        w.put(&self.hold);
        w.put(&self.decay);
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            start: r.get()?,
            kind: AnimationType::from_tag(r.get()?)?,
            period: r.get()?,
            minimum: r.get()?,
            low_color: r.get()?,
            duty: r.get()?,
            count: r.get()?,
            attack: r.get()?,
            hold: r.get()?,
            decay: r.get()?,
        })
    }
}

impl Snapshot for AnimationStore {
    const ID: &'static str = "knorke.light2d.animations";
    const VERSION: u16 = 1;

    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.animations);
    }

    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        crate::check_version::<Self>(version)?;
        self.animations.get_into(r)?;
        if self.animations.iter().any(|(_, d)| d.validate().is_err()) {
            return Err(SnapshotError::Invalid(
                "animation desc violates the animation rules",
            ));
        }
        Ok(())
    }
}
