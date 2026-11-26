// Origin: chiaro @ 9bc5b9b, src/core/Settings.hpp and Settings.cpp (DefaultSettings,
// QualityResolutions, ResolveWetness, WetRippleSize, ValidateSettings, ResolvedHazeFalloff,
// ResolvedMediumHeightStep, ResolvedMediumQuality) and the light-related structs of
// include/Chiaro/Chiaro.h (CHI_QualitySettings, CHI_SurfaceSettings, CHI_MaterialSettings,
// CHI_GlowSettings, CHI_HazeSettings).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Settings of the 2D light and its quality presets (P19, ADR 0016).
//!
//! Replaces the light part of `CHI_Settings` (and the `CullSettings` of P8). The output grade,
//! bloom and CRT live in `knorke-post`, which does not know this crate.
//!
//! Redesign against the C++ version:
//!
//! - [`Quality`] is an enum: the four presets and [`Quality::Custom`] with its own
//!   [`Resolutions`]. There are no preset fields that the setter overwrites;
//!   [`Quality::resolutions`] is always the value in use.
//! - "0 means derived" is `Option` (`None` = derived): [`MaterialSettings`] wet fields,
//!   [`HazeSettings::height_falloff`], [`LightSettings::medium_height_step`]. An explicit
//!   `Some(0.0)` is refused where 0 has no meaning and means "none" for `wet_breakup`.
//! - Flag bits are `bool` ([`HazeSettings::shafts`]) or enums ([`Filter`], [`GlowColor`]).
//! - [`LightSettings::validate`] names the first violated field as a [`SettingsError`] with a
//!   stable code; [`LightSettings::replace`] swaps all settings or none. The zero-initialised
//!   struct check of the C++ version is gone: [`Default`] is the starting point.
//! - Settings are host configuration, not simulation state: no snapshot (ADR 0016).

use crate::Projection;
use crate::frame_warnings::DebugChecks;
use knorke_math::{Vec3, det};

/// Scene pixels of one ripple cell when [`MaterialSettings::wet_ripple_size`] is `None`.
pub const DEFAULT_WET_RIPPLE_PIXELS: f32 = 8.0;
/// Scene pixels of height in which derived haze falls to 1/e.
pub const DEFAULT_HAZE_PIXELS: f32 = 50.0;
/// Medium height step = this x the surface height step while it is `None`.
pub const MEDIUM_STEP_FACTOR: f32 = 4.0;
/// Largest `glow.scale`.
pub const MAX_GLOW_SCALE: f32 = 64.0;
/// Largest `specular_scale`.
pub const MAX_SPECULAR_SCALE: f32 = 16.0;
/// Largest wet stretch.
pub const MAX_WET_STRETCH: f32 = 64.0;
/// Largest `|anisotropy|` of the haze (the same limit as a medium's).
pub const MAX_ANISOTROPY: f32 = 0.9;

/// Texture filter for scaling up a buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Blocky.
    Nearest,
    /// Smooth.
    #[default]
    Linear,
}

/// Where the colour of the glow buffer comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlowColor {
    /// The RGB of the glow buffer.
    #[default]
    Buffer,
    /// The colour of the scene pixel; only the alpha of the glow buffer counts.
    Scene,
}

/// Resolutions of the buffers relative to the scene, each in `(0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolutions {
    /// Light buffer.
    pub light: f32,
    /// Occlusion mask.
    pub shadow: f32,
    /// Haze and medium scatter buffer.
    pub medium: f32,
}

/// Cost of the haze and medium stage: buffer resolution, scatter samples and transmission samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediumCost {
    /// Relative to the scene, `(0, 1]`.
    pub resolution: f32,
    /// Scatter samples per pixel and light.
    pub samples: u32,
    /// Transmission samples per scatter sample.
    pub transmission_samples: u32,
}

/// Rendering resolution: a preset, or own values. Smaller is cheaper and softer. With a material
/// buffer, a light resolution below 1 also switches to the approximate direction mode.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Quality {
    /// Pixel-exact: light 1, shadow 1, medium 0.5.
    #[default]
    Exact,
    /// Light 0.75, shadow 0.75, medium 0.5.
    High,
    /// Light 0.5, shadow 0.5, medium 0.5.
    Medium,
    /// Light 0.25, shadow 0.5, medium 0.25.
    Low,
    /// Own resolutions; the medium cost is that of [`Quality::Medium`].
    Custom(Resolutions),
}

impl Quality {
    /// The resolutions in use.
    pub fn resolutions(self) -> Resolutions {
        let (light, shadow, medium) = match self {
            Self::Exact => (1.0, 1.0, 0.5),
            Self::High => (0.75, 0.75, 0.5),
            Self::Medium => (0.5, 0.5, 0.5),
            Self::Low => (0.25, 0.5, 0.25),
            Self::Custom(r) => return r,
        };
        Resolutions {
            light,
            shadow,
            medium,
        }
    }

    /// Resolution and sample counts of the medium stage.
    pub fn medium_cost(self) -> MediumCost {
        let (samples, transmission_samples) = match self {
            Self::Exact => (16, 4),
            Self::High => (12, 4),
            Self::Low => (6, 2),
            Self::Medium | Self::Custom(_) => (8, 3),
        };
        MediumCost {
            resolution: self.resolutions().medium,
            samples,
            transmission_samples,
        }
    }
}

/// Quality preset and the filter that scales the scatter buffer up (the same for every preset).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QualitySettings {
    /// Preset or own resolutions.
    pub quality: Quality,
    /// How composite upsamples the haze and medium scatter buffer. Default linear.
    pub medium_filter: Filter,
}

/// Interpretation of the material buffer: highlight strength and wet ground.
///
/// `wetness` is the one-number entry point; it maps to the three expert fields, each of which
/// overrides the mapping when it is `Some` (see [`MaterialSettings::wet_mapping`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSettings {
    /// Artistic HDR factor of the highlights, `[0, 16]`; 1 is physically plausible.
    pub specular_scale: f32,
    /// How wet flat ground looks, `[0, 1]`.
    pub wetness: f32,
    /// Expert, `[1, 64]`: how many times longer the streak is than the round highlight.
    pub wet_stretch: Option<f32>,
    /// Expert, `[0, 1]`: noise that breaks the streaks into ripple bands.
    pub wet_breakup: Option<f32>,
    /// Expert, `> 0`: world units along the streak of one noise cell. `None`: 8 scene pixels on
    /// the ground ([`LightSettings::wet_ripple_size`]).
    pub wet_ripple_size: Option<f32>,
}

impl Default for MaterialSettings {
    fn default() -> Self {
        Self {
            specular_scale: 1.0,
            wetness: 0.0,
            wet_stretch: None,
            wet_breakup: None,
            wet_ripple_size: None,
        }
    }
}

/// The wet-ground parameters in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WetMapping {
    /// Streak length factor, 1 = off.
    pub stretch: f32,
    /// Breakup noise share.
    pub breakup: f32,
    /// World units of a ripple cell; `None` until a projection resolves it.
    pub ripple_size: Option<f32>,
}

impl MaterialSettings {
    /// `wetness` mapped (`stretch = 1 + 5 w`, `breakup = 0.65 w`), explicit fields overriding.
    pub fn wet_mapping(&self) -> WetMapping {
        WetMapping {
            stretch: self.wet_stretch.unwrap_or(1.0 + 5.0 * self.wetness),
            breakup: self.wet_breakup.unwrap_or(0.65 * self.wetness),
            ripple_size: self.wet_ripple_size,
        }
    }
}

/// Interpretation of the glow buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowSettings {
    /// HDR factor, `[0, 64]`.
    pub scale: f32,
    /// Where the colour comes from.
    pub color: GlowColor,
}

impl Default for GlowSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            color: GlowColor::Buffer,
        }
    }
}

/// Haze: global light cones and shafts in the air. The extinction at height `z` above the ground
/// is `density * exp(-z / H)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HazeSettings {
    /// Extinction per world unit at the ground, `>= 0`; 0 = no haze, the frame is unchanged.
    pub density: f32,
    /// World units of height in which the density falls to 1/e, `> 0`; `None`: 50 scene pixels
    /// of height ([`LightSettings::haze_falloff`]).
    pub height_falloff: Option<f32>,
    /// Albedo, each channel in `[0, 1]`.
    pub color: Vec3,
    /// Henyey-Greenstein `g`, `[-0.9, 0.9]`; forward scattering by default.
    pub anisotropy: f32,
    /// Obstacles cast shadows into haze and media (light shafts). Default on.
    pub shafts: bool,
}

impl Default for HazeSettings {
    fn default() -> Self {
        Self {
            density: 0.0,
            height_falloff: None,
            color: Vec3::new(0.9, 0.9, 0.9),
            anisotropy: 0.6,
            shafts: true,
        }
    }
}

/// All settings of the 2D light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightSettings {
    /// Ambient light, linear RGB, each `>= 0`. Default white.
    pub ambient: Vec3,
    /// Resolutions and scatter filter.
    pub quality: QualitySettings,
    /// World units per height step of the surface buffer, `> 0`.
    pub surface_height_step: f32,
    /// Material buffer.
    pub material: MaterialSettings,
    /// Glow buffer.
    pub glow: GlowSettings,
    /// Global haze.
    pub haze: HazeSettings,
    /// World units per height step of the medium layer, `> 0`; `None`: 4 x the surface step.
    pub medium_height_step: Option<f32>,
    /// Which debug checks run ([`FrameWarnings`](crate::frame_warnings::FrameWarnings)). Default none.
    pub debug_checks: DebugChecks,
    /// Transition to light rows (ADR 0019, plan 17.13): point, spot, line and rect lights the
    /// rows can draw ([`rows::light_suits`](crate::rows::light_suits)) take their shadow from one
    /// row per lamp (four for line and rect) instead of a ray march per pixel; all others are
    /// drawn as before. Default off:
    /// then nothing changes.
    pub light_rows: bool,
}

impl Default for LightSettings {
    fn default() -> Self {
        Self {
            ambient: Vec3::ONE,
            quality: QualitySettings::default(),
            surface_height_step: 1.0,
            material: MaterialSettings::default(),
            glow: GlowSettings::default(),
            haze: HazeSettings::default(),
            medium_height_step: None,
            debug_checks: DebugChecks::NONE,
            light_rows: false,
        }
    }
}

knorke_types::settings_error!(
    "knorke.light2d.settings.not_finite",
    "knorke.light2d.settings.out_of_range"
);

fn scale(field: &'static str, v: f32) -> Result<(), SettingsError> {
    check(field, v, v > 0.0 && v <= 1.0, "must be in (0, 1]")
}

fn positive(field: &'static str, v: f32) -> Result<(), SettingsError> {
    check(field, v, v > 0.0, "must be > 0")
}

impl LightSettings {
    /// Checks every field, also those without effect (haze with density 0); the first violation
    /// is returned.
    pub fn validate(&self) -> Result<(), SettingsError> {
        let c = self.ambient;
        for v in [c.x, c.y, c.z] {
            check("ambient", v, v >= 0.0, "components must be >= 0")?;
        }
        if let Quality::Custom(r) = self.quality.quality {
            scale("quality.light_resolution", r.light)?;
            scale("quality.shadow_resolution", r.shadow)?;
            scale("quality.medium_resolution", r.medium)?;
        }
        positive("surface_height_step", self.surface_height_step)?;
        let m = &self.material;
        #[rustfmt::skip]
        ranges(&[
            ("material.specular_scale", m.specular_scale, 0.0, MAX_SPECULAR_SCALE, "must be in [0, 16]"),
            ("material.wetness", m.wetness, 0.0, 1.0, "must be in [0, 1]"),
        ])?;
        if let Some(v) = m.wet_stretch {
            range(
                "material.wet_stretch",
                v,
                1.0,
                MAX_WET_STRETCH,
                "must be in [1, 64]",
            )?;
        }
        if let Some(v) = m.wet_breakup {
            range("material.wet_breakup", v, 0.0, 1.0, "must be in [0, 1]")?;
        }
        if let Some(v) = m.wet_ripple_size {
            positive("material.wet_ripple_size", v)?;
        }
        range(
            "glow.scale",
            self.glow.scale,
            0.0,
            MAX_GLOW_SCALE,
            "must be in [0, 64]",
        )?;
        let h = &self.haze;
        check("haze.density", h.density, h.density >= 0.0, "must be >= 0")?;
        if let Some(v) = h.height_falloff {
            positive("haze.height_falloff", v)?;
        }
        for v in [h.color.x, h.color.y, h.color.z] {
            range("haze.color", v, 0.0, 1.0, "components must be in [0, 1]")?;
        }
        range(
            "haze.anisotropy",
            h.anisotropy,
            -MAX_ANISOTROPY,
            MAX_ANISOTROPY,
            "must be in [-0.9, 0.9]",
        )?;
        if let Some(v) = self.medium_height_step {
            positive("medium_height_step", v)?;
        }
        Ok(())
    }

    /// Validates `new` and, only if it is valid, makes it the settings. On error nothing changes.
    pub fn replace(&mut self, new: LightSettings) -> Result<(), SettingsError> {
        new.validate()?;
        *self = new;
        Ok(())
    }

    /// The resolutions in use.
    pub fn resolutions(&self) -> Resolutions {
        self.quality.quality.resolutions()
    }

    /// World units per height step of the medium layer: the set value, else 4 x the surface step.
    pub fn resolved_medium_height_step(&self) -> f32 {
        self.medium_height_step
            .unwrap_or(MEDIUM_STEP_FACTOR * self.surface_height_step)
    }

    /// Height scale `H` of the haze in world units: the set falloff, else 50 scene pixels of
    /// height (`50 / |z_axis|`, in a top-down projection `50 / |x_axis|`).
    pub fn haze_falloff(&self, projection: &Projection) -> f32 {
        if let Some(h) = self.haze.height_falloff {
            return h;
        }
        let z = f64::from(det::hypot(projection.z_axis.x, projection.z_axis.y));
        let x = f64::from(det::hypot(projection.x_axis.x, projection.x_axis.y));
        let len = if z > 0.0 { z } else { x };
        if len > 0.0 {
            (f64::from(DEFAULT_HAZE_PIXELS) / len) as f32
        } else {
            DEFAULT_HAZE_PIXELS
        }
    }

    /// World units of one ripple cell: the set size, else 8 scene pixels at the projection's
    /// ground scale (`8 / sqrt(|det [x_axis y_axis]|)`), the same in metre and pixel worlds.
    pub fn wet_ripple_size(&self, projection: &Projection) -> f32 {
        if let Some(s) = self.material.wet_ripple_size {
            return s;
        }
        let (x, y) = (projection.x_axis, projection.y_axis);
        let det = (f64::from(x.x) * f64::from(y.y) - f64::from(x.y) * f64::from(y.x)).abs();
        if det > 0.0 {
            (f64::from(DEFAULT_WET_RIPPLE_PIXELS) / det.sqrt()) as f32
        } else {
            DEFAULT_WET_RIPPLE_PIXELS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_math::{At, Screen, Vec2};

    fn valid(s: LightSettings) {
        assert_eq!(s.validate(), Ok(()), "{s:?}");
    }

    fn bad(mutate: impl FnOnce(&mut LightSettings), field: &str) -> SettingsError {
        let mut s = LightSettings::default();
        mutate(&mut s);
        let e = s.validate().expect_err(field);
        let (SettingsError::NotFinite { field: f, .. }
        | SettingsError::OutOfRange { field: f, .. }) = e;
        assert_eq!(f, field);
        e
    }

    #[test]
    fn defaults_are_valid_and_match_chiaro() {
        let s = LightSettings::default();
        valid(s);
        assert_eq!(s.ambient, Vec3::ONE);
        assert_eq!(s.quality.quality, Quality::Exact);
        assert_eq!(s.quality.medium_filter, Filter::Linear);
        assert_eq!(s.surface_height_step, 1.0);
        assert_eq!(s.material.specular_scale, 1.0);
        assert_eq!(s.glow.scale, 1.0);
        assert_eq!(s.glow.color, GlowColor::Buffer);
        assert_eq!(s.haze.density, 0.0);
        assert_eq!(s.haze.anisotropy, 0.6);
        assert!(s.haze.shafts);
        assert_eq!(s.haze.color, Vec3::new(0.9, 0.9, 0.9));
        assert_eq!(s.medium_height_step, None);
        assert!(!s.light_rows);
        assert!(!s.debug_checks.any());
    }

    #[test]
    fn presets_have_the_chiaro_resolutions() {
        let r = |q: Quality| {
            let r = q.resolutions();
            (r.light, r.shadow, r.medium)
        };
        assert_eq!(r(Quality::Exact), (1.0, 1.0, 0.5));
        assert_eq!(r(Quality::High), (0.75, 0.75, 0.5));
        assert_eq!(r(Quality::Medium), (0.5, 0.5, 0.5));
        assert_eq!(r(Quality::Low), (0.25, 0.5, 0.25));
        let custom = Resolutions {
            light: 0.3,
            shadow: 0.6,
            medium: 0.2,
        };
        assert_eq!(Quality::Custom(custom).resolutions(), custom);
    }

    #[test]
    fn medium_cost_follows_the_preset_and_custom_takes_medium() {
        let c = |q: Quality| {
            let c = q.medium_cost();
            (c.resolution, c.samples, c.transmission_samples)
        };
        assert_eq!(c(Quality::Exact), (0.5, 16, 4));
        assert_eq!(c(Quality::High), (0.5, 12, 4));
        assert_eq!(c(Quality::Medium), (0.5, 8, 3));
        assert_eq!(c(Quality::Low), (0.25, 6, 2));
        let custom = Quality::Custom(Resolutions {
            light: 1.0,
            shadow: 1.0,
            medium: 0.125,
        });
        assert_eq!(c(custom), (0.125, 8, 3));
    }

    #[test]
    fn custom_resolutions_are_checked_presets_cannot_be_wrong() {
        let custom = |light, shadow, medium| {
            Quality::Custom(Resolutions {
                light,
                shadow,
                medium,
            })
        };
        valid(LightSettings {
            quality: QualitySettings {
                quality: custom(1.0, 0.01, 0.5),
                ..Default::default()
            },
            ..Default::default()
        });
        for (q, field) in [
            (custom(0.0, 1.0, 1.0), "quality.light_resolution"),
            (custom(1.0, 1.5, 1.0), "quality.shadow_resolution"),
            (custom(1.0, 1.0, f32::NAN), "quality.medium_resolution"),
        ] {
            bad(|s| s.quality.quality = q, field);
        }
    }

    #[test]
    fn ranges_are_enforced_per_field() {
        bad(|s| s.ambient = Vec3::new(1.0, -0.1, 1.0), "ambient");
        bad(|s| s.surface_height_step = 0.0, "surface_height_step");
        bad(
            |s| s.material.specular_scale = 16.5,
            "material.specular_scale",
        );
        bad(|s| s.material.wetness = 1.1, "material.wetness");
        bad(
            |s| s.material.wet_stretch = Some(0.5),
            "material.wet_stretch",
        );
        bad(
            |s| s.material.wet_stretch = Some(65.0),
            "material.wet_stretch",
        );
        bad(
            |s| s.material.wet_breakup = Some(-0.1),
            "material.wet_breakup",
        );
        bad(
            |s| s.material.wet_ripple_size = Some(0.0),
            "material.wet_ripple_size",
        );
        bad(|s| s.glow.scale = 64.1, "glow.scale");
        bad(|s| s.haze.density = -1.0, "haze.density");
        bad(|s| s.haze.height_falloff = Some(0.0), "haze.height_falloff");
        bad(|s| s.haze.color = Vec3::new(0.0, 1.1, 0.0), "haze.color");
        bad(|s| s.haze.anisotropy = 0.95, "haze.anisotropy");
        bad(|s| s.medium_height_step = Some(-1.0), "medium_height_step");
        // The borders themselves are fine.
        valid(LightSettings {
            material: MaterialSettings {
                specular_scale: 16.0,
                wetness: 1.0,
                wet_stretch: Some(1.0),
                wet_breakup: Some(0.0),
                ..Default::default()
            },
            glow: GlowSettings {
                scale: 64.0,
                ..Default::default()
            },
            haze: HazeSettings {
                anisotropy: -0.9,
                ..Default::default()
            },
            ..Default::default()
        });
    }

    #[test]
    fn non_finite_values_get_their_own_code() {
        let e = bad(|s| s.haze.density = f32::INFINITY, "haze.density");
        assert_eq!(e.code().as_str(), "knorke.light2d.settings.not_finite");
        let e = bad(|s| s.glow.scale = 100.0, "glow.scale");
        assert_eq!(e.code().as_str(), "knorke.light2d.settings.out_of_range");
        assert!(
            e.to_string()
                .contains("settings.glow.scale must be in [0, 64]")
        );
        let d = e.to_diagnostic();
        assert_eq!(d.path.as_deref(), Some("settings.glow.scale"));
        assert_eq!(d.severity, knorke_types::Severity::Error);
    }

    #[test]
    fn values_are_checked_even_where_they_have_no_effect() {
        // Haze is off at density 0, but a bad anisotropy is still refused.
        bad(|s| s.haze.anisotropy = f32::NAN, "haze.anisotropy");
    }

    #[test]
    fn replace_is_atomic() {
        let mut s = LightSettings::default();
        let good = LightSettings {
            ambient: Vec3::new(0.2, 0.2, 0.3),
            ..Default::default()
        };
        assert_eq!(s.replace(good), Ok(()));
        assert_eq!(s.ambient, good.ambient);
        let mut wrong = LightSettings::default();
        wrong.glow.scale = -1.0;
        assert!(s.replace(wrong).is_err());
        assert_eq!(s, good, "unchanged");
    }

    #[test]
    fn wetness_maps_and_experts_override() {
        let m = MaterialSettings {
            wetness: 1.0,
            ..Default::default()
        };
        let w = m.wet_mapping();
        assert_eq!((w.stretch, w.breakup, w.ripple_size), (6.0, 0.65, None));
        let dry = MaterialSettings::default().wet_mapping();
        assert_eq!((dry.stretch, dry.breakup), (1.0, 0.0));
        let m = MaterialSettings {
            wetness: 1.0,
            wet_stretch: Some(10.0),
            wet_breakup: Some(0.0),
            wet_ripple_size: Some(2.0),
            ..Default::default()
        };
        let w = m.wet_mapping();
        assert_eq!(
            (w.stretch, w.breakup, w.ripple_size),
            (10.0, 0.0, Some(2.0))
        );
    }

    fn projection(x: [f32; 2], y: [f32; 2], z: [f32; 2]) -> Projection {
        Projection {
            origin: At::<Screen, Vec2>::new(Vec2::new(0.0, 0.0)),
            x_axis: Vec2::new(x[0], x[1]),
            y_axis: Vec2::new(y[0], y[1]),
            z_axis: Vec2::new(z[0], z[1]),
        }
    }

    #[test]
    fn ripple_size_is_derived_from_the_ground_scale() {
        let s = LightSettings::default();
        // 4 scene pixels per world unit on both axes: det 16, 8 / 4 = 2 world units.
        let p = projection([4.0, 0.0], [0.0, 4.0], [0.0, 0.0]);
        assert_eq!(s.wet_ripple_size(&p), 2.0);
        let set = LightSettings {
            material: MaterialSettings {
                wet_ripple_size: Some(3.0),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(set.wet_ripple_size(&p), 3.0);
    }

    #[test]
    fn haze_falloff_and_medium_step_are_derived() {
        let s = LightSettings::default();
        let iso = projection([2.0, 1.0], [-2.0, 1.0], [0.0, -5.0]);
        assert_eq!(s.haze_falloff(&iso), 10.0);
        let top_down = projection([4.0, 0.0], [0.0, 4.0], [0.0, 0.0]);
        assert_eq!(s.haze_falloff(&top_down), 12.5);
        let set = LightSettings {
            haze: HazeSettings {
                height_falloff: Some(7.0),
                ..Default::default()
            },
            surface_height_step: 0.5,
            ..Default::default()
        };
        assert_eq!(set.haze_falloff(&iso), 7.0);
        assert_eq!(set.resolved_medium_height_step(), 2.0);
        let set = LightSettings {
            medium_height_step: Some(3.0),
            ..set
        };
        assert_eq!(set.resolved_medium_height_step(), 3.0);
    }
}
