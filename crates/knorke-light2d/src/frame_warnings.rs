// Origin: chiaro @ 9bc5b9b, src/core/FrameWarnings.hpp and FrameWarnings.cpp (FrameWarnings::Update,
// the CHI_WARN_* and CHI_DEBUG_* bits of include/Chiaro/Chiaro.h).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Findings of a frame as stable diagnostics (P18, ADR 0016).
//!
//! Redesign against the C++ version:
//!
//! - The `CHI_WARN_*` bits are the typed [`Warning`] and the bit set [`WarningSet`]; the
//!   `CHI_DEBUG_*` bits are [`DebugChecks`]. Every kind has a stable code
//!   `knorke.light2d.warning.<name>` and becomes a [`Diagnostic`] of severity warning on request.
//! - The module does not log. Chiaro logged each kind once per context; here
//!   [`FrameWarnings::newly_reported`] names the kinds that appeared for the first time in the
//!   last frame, and the host decides what to do with them.
//! - Off costs nothing (ADR 0011): [`DebugChecks::NONE`] is the default, and then
//!   [`FrameWarnings::update`] only clears its state.
//! - Findings of the GPU check arrive as a [`WarningSet`] in [`FrameFindings::gpu_findings`]
//!   (or through [`FrameWarnings::add_pending`] for findings made between frames).
//! - Patterns: the host passes [`Warning::PatternNoAlpha`] as pending when
//!   `PatternStore::create` reports a sheet without alpha. Instead of Chiaro's callback
//!   `destroyedPatternUsers` and its scan flag, [`FrameFindings::destroyed_pattern_users`] is the
//!   count the plan takes every frame while [`DebugChecks::OBSTACLES`] is on (off: no scan).
//!
//! Nothing here allocates except [`FrameWarnings::diagnostics`], which writes into a vector of
//! the host.

use knorke_types::{Diagnostic, DiagnosticCode};

/// Which families of debug checks run (`CHI_DEBUG_*`). Default: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DebugChecks(u8);

impl DebugChecks {
    /// No check runs.
    pub const NONE: Self = Self(0);
    /// Uncleared layers, half-transparent buffer texels, blown-out highlights, flat medium.
    pub const LAYERS: Self = Self(1);
    /// Flat, unreachable and away-facing lights, ambient zone height spans.
    pub const LIGHTS: Self = Self(2);
    /// Thin obstacles, obstacles naming a destroyed pattern, pattern sheets without alpha.
    pub const OBSTACLES: Self = Self(4);
    /// Every check, also those added later.
    pub const ALL: Self = Self(!0);

    /// `true` if every check of `other` is on.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` if at least one check is on.
    pub const fn any(self) -> bool {
        self.0 != 0
    }
}

impl core::ops::BitOr for DebugChecks {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// One kind of finding (`CHI_WARN_*`), with a stable code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Warning {
    /// Layers were rendered again without being cleared and hold the previous frame.
    StaleLayers,
    /// Surface or material texels with a half alpha.
    HalfAlpha,
    /// Material highlights more than 33 times as bright as the diffuse light.
    BlownOutHighlights,
    /// Medium pixels with a density but no height.
    FlatMedium,
    /// Point or spot lights at height 0 with a surface or material buffer.
    FlatLights,
    /// Lights higher than their radius above every pixel.
    UnreachableLights,
    /// One-sided rect lights facing away from everything.
    FacingAwayLights,
    /// Ambient zones with a height span in a frame without a surface buffer.
    AmbientZoneHeights,
    /// Circle obstacles thinner than one occlusion mask texel.
    ThinObstacles,
    /// Active obstacles still name a destroyed pattern and act as if they had none.
    PatternDestroyed,
    /// Patterns were cut from a sheet without alpha, so they are fully opaque.
    PatternNoAlpha,
    /// Shadow-casting lights drawn without shadow: reaching farther than a light row
    /// (`rows::WIDE_REACH`) or beyond the row atlas's slots (`rows::MAX_ROWS`). Reported
    /// whatever the debug checks.
    UnshadowedLights,
}

impl Warning {
    /// Every kind, in the order of [`WarningSet::iter`].
    pub const ALL: [Warning; 12] = [
        Self::StaleLayers,
        Self::HalfAlpha,
        Self::BlownOutHighlights,
        Self::FlatMedium,
        Self::FlatLights,
        Self::UnreachableLights,
        Self::FacingAwayLights,
        Self::AmbientZoneHeights,
        Self::ThinObstacles,
        Self::PatternDestroyed,
        Self::PatternNoAlpha,
        Self::UnshadowedLights,
    ];

    const fn bit(self) -> u16 {
        1 << self as u16
    }

    /// The stable code, `knorke.light2d.warning.<name>`.
    pub fn code(self) -> DiagnosticCode {
        DiagnosticCode::new(match self {
            Self::StaleLayers => "knorke.light2d.warning.stale_layers",
            Self::HalfAlpha => "knorke.light2d.warning.half_alpha",
            Self::BlownOutHighlights => "knorke.light2d.warning.blown_out_highlights",
            Self::FlatMedium => "knorke.light2d.warning.flat_medium",
            Self::FlatLights => "knorke.light2d.warning.flat_lights",
            Self::UnreachableLights => "knorke.light2d.warning.unreachable_lights",
            Self::FacingAwayLights => "knorke.light2d.warning.facing_away_lights",
            Self::AmbientZoneHeights => "knorke.light2d.warning.ambient_zone_heights",
            Self::ThinObstacles => "knorke.light2d.warning.thin_obstacles",
            Self::PatternDestroyed => "knorke.light2d.warning.pattern_destroyed",
            Self::PatternNoAlpha => "knorke.light2d.warning.pattern_no_alpha",
            Self::UnshadowedLights => "knorke.light2d.warning.unshadowed_lights",
        })
    }

    /// The debug checks that switch this kind on.
    pub const fn check(self) -> DebugChecks {
        match self {
            Self::StaleLayers | Self::HalfAlpha | Self::BlownOutHighlights | Self::FlatMedium => {
                DebugChecks::LAYERS
            }
            Self::FlatLights
            | Self::UnreachableLights
            | Self::FacingAwayLights
            | Self::AmbientZoneHeights => DebugChecks::LIGHTS,
            Self::ThinObstacles | Self::PatternDestroyed | Self::PatternNoAlpha => {
                DebugChecks::OBSTACLES
            }
            Self::UnshadowedLights => DebugChecks::NONE,
        }
    }

    /// What is wrong and what to do about it.
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnshadowedLights => {
                "shadow-casting lights were drawn without shadow: they reach farther than 2047 \
                 occlusion mask texels, or the frame needs more than 16384 light rows; lower \
                 their radius, the shadow resolution or the number of shadowed lights"
            }
            Self::StaleLayers => {
                "layers were rendered without being cleared since the last frame, so they hold \
                 the previous frame's content; clear them every frame"
            }
            Self::HalfAlpha => {
                "a surface buffer has texels with alpha 1..254 or a material buffer with alpha \
                 1..191; draw maps with nearest sampling, no blending and alpha 0 or 255"
            }
            Self::BlownOutHighlights => {
                "material texels have highlights more than 33 x as bright as the diffuse light, \
                 more than any energy-conserving material reflects; lower specular_scale or the \
                 specular strength of those texels"
            }
            Self::FlatMedium => {
                "medium layer pixels have a density but their top edge is not above their bottom \
                 edge, so they hold no air; give the top a value above the bottom (at least one \
                 medium height step)"
            }
            Self::FlatLights => {
                "point/spot lights at height 0 with a surface or material buffer leave flat \
                 ground dark; use Height::Auto or about 0.4 x radius"
            }
            Self::UnreachableLights => {
                "lights are higher than their radius above every pixel and light nothing; raise \
                 the radius or lower the height"
            }
            Self::FacingAwayLights => {
                "one-sided rect lights face away from everything they could light (e.g. up into \
                 the sky); point the direction toward the scene or make them two-sided"
            }
            Self::AmbientZoneHeights => {
                "ambient zones with base_height > 0 or a finite height are drawn without a \
                 surface buffer, where every pixel is ground and the span is ignored; pass a \
                 surface buffer or leave the span at its default"
            }
            Self::ThinObstacles => {
                "circle obstacles thinner than one occlusion mask texel were widened to one \
                 texel; make poles at least 1 / shadow_resolution scene pixels thick"
            }
            Self::PatternDestroyed => {
                "active obstacles are still assigned a pattern that was destroyed and act as if                  they had none; assign another pattern or none before destroying it"
            }
            Self::PatternNoAlpha => {
                "patterns were cut from a sheet without an alpha channel, so they are fully                  opaque; use a sheet format with alpha"
            }
        }
    }

    /// The finding as a diagnostic (severity warning, no path).
    pub fn to_diagnostic(self) -> Diagnostic {
        Diagnostic::warning(self.code(), self.message())
    }
}

/// A set of [`Warning`] kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WarningSet(u16);

impl WarningSet {
    /// No kind.
    pub const EMPTY: Self = Self(0);

    /// The set of exactly `w`.
    pub const fn of(w: Warning) -> Self {
        Self(w.bit())
    }

    /// Adds `w`.
    pub fn insert(&mut self, w: Warning) {
        self.0 |= w.bit();
    }

    /// `true` if `w` is in the set.
    pub const fn contains(self, w: Warning) -> bool {
        self.0 & w.bit() != 0
    }

    /// `true` if the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of kinds.
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// The kinds in the order of [`Warning::ALL`].
    pub fn iter(self) -> impl Iterator<Item = Warning> {
        Warning::ALL.into_iter().filter(move |w| self.contains(*w))
    }

    /// The kinds of `self` that `other` does not have.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl FromIterator<Warning> for WarningSet {
    fn from_iter<I: IntoIterator<Item = Warning>>(iter: I) -> Self {
        let mut s = Self::EMPTY;
        for w in iter {
            s.insert(w);
        }
        s
    }
}

/// What one successful frame found, as the debug checks see it (counts only, no stores).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameFindings {
    /// The checks in effect.
    pub checks: DebugChecks,
    /// Kinds the GPU check found in an earlier frame (they arrive a few frames late).
    pub gpu_findings: WarningSet,
    /// Point/spot lights at height 0 with normals.
    pub flat_lights: u32,
    /// Lights that cannot reach the ground.
    pub unreachable_lights: u32,
    /// One-sided rect lights facing away.
    pub facing_away_lights: u32,
    /// Obstacles thinner than one mask texel.
    pub thin_obstacles: u32,
    /// Active obstacles that name a destroyed pattern.
    pub destroyed_pattern_users: u32,
    /// Visible ambient zones with a height span in a frame without a surface buffer.
    pub zone_height_spans: u32,
    /// Shadow-casting lights drawn without shadow (`LightList::shadowless_rows`).
    pub unshadowed_lights: u32,
}

/// Turns the findings of frames into [`WarningSet`]s and remembers which kinds were seen.
#[derive(Clone, Debug, Default)]
pub struct FrameWarnings {
    pending: WarningSet,
    warnings: WarningSet,
    reported: WarningSet,
    new: WarningSet,
    flat_lights_hint: bool,
    flat_lights_seen: bool,
}

impl FrameWarnings {
    /// An evaluator without history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Findings made between frames, reported by the next [`update`](Self::update).
    pub fn add_pending(&mut self, w: WarningSet) {
        self.pending.0 |= w.0;
    }

    /// Drops the pending findings (the call that made them failed).
    pub fn drop_pending(&mut self) {
        self.pending = WarningSet::EMPTY;
    }

    /// The kinds of the last [`update`](Self::update).
    pub fn warnings(&self) -> WarningSet {
        self.warnings
    }

    /// The kinds of the last update that no earlier update had reported (log these).
    pub fn newly_reported(&self) -> WarningSet {
        self.new
    }

    /// Every kind reported so far.
    pub fn reported(&self) -> WarningSet {
        self.reported
    }

    /// `true` in the update in which flat lights were found for the first time. Independent of
    /// the debug checks, as in Chiaro: the hint is always worth showing once.
    pub fn flat_lights_hint(&self) -> bool {
        self.flat_lights_hint
    }

    /// Evaluates one successful frame and returns its kinds; the pending findings are consumed.
    pub fn update(&mut self, f: &FrameFindings) -> WarningSet {
        self.flat_lights_hint = f.flat_lights > 0 && !self.flat_lights_seen;
        self.flat_lights_seen |= f.flat_lights > 0;
        self.pending.0 |= f.gpu_findings.0;

        let mut w = WarningSet::EMPTY;
        if f.checks.any() {
            if f.checks.contains(DebugChecks::LAYERS) {
                for k in [
                    Warning::StaleLayers,
                    Warning::HalfAlpha,
                    Warning::BlownOutHighlights,
                    Warning::FlatMedium,
                ] {
                    if self.pending.contains(k) {
                        w.insert(k);
                    }
                }
            }
            if f.checks.contains(DebugChecks::LIGHTS) {
                for (k, n) in [
                    (Warning::FlatLights, f.flat_lights),
                    (Warning::UnreachableLights, f.unreachable_lights),
                    (Warning::FacingAwayLights, f.facing_away_lights),
                    (Warning::AmbientZoneHeights, f.zone_height_spans),
                ] {
                    if n > 0 {
                        w.insert(k);
                    }
                }
            }
            if f.checks.contains(DebugChecks::OBSTACLES) {
                for (k, found) in [
                    (Warning::ThinObstacles, f.thin_obstacles > 0),
                    (Warning::PatternDestroyed, f.destroyed_pattern_users > 0),
                    (
                        Warning::PatternNoAlpha,
                        self.pending.contains(Warning::PatternNoAlpha),
                    ),
                ] {
                    if found {
                        w.insert(k);
                    }
                }
            }
        }
        if f.unshadowed_lights > 0 {
            w.insert(Warning::UnshadowedLights);
        }
        self.pending = WarningSet::EMPTY;
        self.warnings = w;
        self.new = w.without(self.reported);
        self.reported.0 |= w.0;
        w
    }

    /// Appends the kinds of the last update as diagnostics to `out`, in [`Warning::ALL`] order.
    pub fn diagnostics(&self, out: &mut Vec<Diagnostic>) {
        out.extend(self.warnings.iter().map(Warning::to_diagnostic));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_types::Severity;

    fn lights(n: u32) -> FrameFindings {
        FrameFindings {
            checks: DebugChecks::ALL,
            flat_lights: n,
            unreachable_lights: n,
            facing_away_lights: n,
            zone_height_spans: n,
            thin_obstacles: n,
            destroyed_pattern_users: n,
            ..FrameFindings::default()
        }
    }

    #[test]
    fn off_by_default_and_reports_nothing() {
        let mut w = FrameWarnings::new();
        let f = FrameFindings {
            flat_lights: 3,
            thin_obstacles: 2,
            gpu_findings: WarningSet::of(Warning::HalfAlpha),
            ..FrameFindings::default()
        };
        assert!(w.update(&f).is_empty());
        assert!(w.newly_reported().is_empty());
        assert_eq!(DebugChecks::default(), DebugChecks::NONE);
    }

    #[test]
    fn each_check_switches_only_its_kinds() {
        let mut w = FrameWarnings::new();
        let mut f = lights(1);
        for (checks, expect) in [
            (DebugChecks::LAYERS, &Warning::ALL[..4]),
            (DebugChecks::LIGHTS, &Warning::ALL[4..8]),
            (DebugChecks::OBSTACLES, &Warning::ALL[8..10]),
        ] {
            f.checks = checks;
            f.gpu_findings = Warning::ALL[..4].iter().copied().collect();
            let got: Vec<_> = w.update(&f).iter().collect();
            // The GPU findings only show with the layer checks; the rest follows the counts.
            assert_eq!(got, expect, "{checks:?}");
        }
        f.checks = DebugChecks::LAYERS | DebugChecks::OBSTACLES;
        f.gpu_findings = WarningSet::of(Warning::StaleLayers);
        assert_eq!(w.update(&f).len(), 3);
        // A sheet without alpha shows once, with the obstacle checks only.
        f = FrameFindings {
            checks: DebugChecks::LAYERS,
            ..FrameFindings::default()
        };
        w.add_pending(WarningSet::of(Warning::PatternNoAlpha));
        assert!(w.update(&f).is_empty());
        f.checks = DebugChecks::OBSTACLES;
        w.add_pending(WarningSet::of(Warning::PatternNoAlpha));
        assert_eq!(w.update(&f), WarningSet::of(Warning::PatternNoAlpha));
        assert!(w.update(&f).is_empty(), "consumed");
    }

    #[test]
    fn pending_is_consumed_and_can_be_dropped() {
        let mut w = FrameWarnings::new();
        let quiet = FrameFindings {
            checks: DebugChecks::LAYERS,
            ..FrameFindings::default()
        };
        w.add_pending(WarningSet::of(Warning::StaleLayers));
        assert!(w.update(&quiet).contains(Warning::StaleLayers));
        assert!(w.update(&quiet).is_empty(), "consumed");
        w.add_pending(WarningSet::of(Warning::FlatMedium));
        w.drop_pending();
        assert!(w.update(&quiet).is_empty());
    }

    #[test]
    fn pending_is_dropped_at_the_frame_end_even_with_checks_off() {
        let mut w = FrameWarnings::new();
        w.add_pending(WarningSet::of(Warning::HalfAlpha));
        let off = FrameFindings::default();
        assert!(w.update(&off).is_empty());
        let on = FrameFindings {
            checks: DebugChecks::LAYERS,
            ..off
        };
        assert!(w.update(&on).is_empty());
    }

    #[test]
    fn new_kinds_are_reported_once() {
        let mut w = FrameWarnings::new();
        let f = lights(1);
        w.update(&f);
        assert_eq!(w.newly_reported().len(), 6);
        w.update(&f);
        assert!(w.newly_reported().is_empty());
        assert_eq!(w.warnings().len(), 6);
        assert_eq!(w.reported().len(), 6);
        // A kind that went away and comes back is not new.
        w.update(&lights(0));
        assert!(w.warnings().is_empty());
        w.update(&f);
        assert!(w.newly_reported().is_empty());
    }

    #[test]
    fn flat_lights_hint_ignores_the_checks_and_fires_once() {
        let mut w = FrameWarnings::new();
        let f = FrameFindings {
            flat_lights: 2,
            ..FrameFindings::default()
        };
        w.update(&f);
        assert!(w.flat_lights_hint());
        w.update(&f);
        assert!(!w.flat_lights_hint());
    }

    #[test]
    fn diagnostics_have_stable_distinct_codes() {
        let mut w = FrameWarnings::new();
        w.update(&lights(1));
        let mut out = Vec::new();
        w.diagnostics(&mut out);
        assert_eq!(out.len(), 6);
        assert!(out.iter().all(|d| d.severity == Severity::Warning));
        assert_eq!(out[0].code.as_str(), "knorke.light2d.warning.flat_lights");
        let mut codes: Vec<_> = Warning::ALL.iter().map(|w| w.code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), Warning::ALL.len());
        assert!(Warning::ALL.iter().all(|w| !w.message().is_empty()));
        // Every kind but the unshadowed lights (always reported) follows a check.
        let unchecked = Warning::ALL.iter().filter(|w| !w.check().any());
        assert!(unchecked.eq([&Warning::UnshadowedLights]));
    }
}
