// Origin: chiaro @ 9bc5b9b, src/core/Settings.hpp and Settings.cpp (DefaultSettings,
// ValidateSettings, DefaultCrtSettings, CrtPreset, ValidateCrt) and src/core/Bloom.hpp
// (DefaultBloomSettings, ranges), with CHI_OutputSettings, CHI_BloomSettings and CHI_CrtSettings
// of include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Settings of the output stage, bloom and CRT (P19, ADR 0016).
//!
//! The post-processing part of `CHI_Settings`; the light part is `knorke_light2d::settings`.
//!
//! Redesign against the C++ version:
//!
//! - Flag bits are `bool` ([`OutputSettings::dither`], [`CrtSettings::overlay`],
//!   [`BloomSettings::enabled`]) and the tonemap knee belongs to the one curve that has it
//!   ([`Tonemap::Soft`]).
//! - The vignette is its own struct; the CRT presets are [`CrtSettings::preset`].
//! - [`PostSettings::validate`] returns the first violated field as a [`SettingsError`] with a
//!   stable code `knorke.post.settings.*`; [`PostSettings::replace`] swaps all or nothing. Values
//!   are checked even where they have no effect (bloom values while bloom is off).
//! - Settings are host configuration, not simulation state (ADR 0016).

use crate::bloom::{MAX_BLOOM_RADIUS, MIN_BLOOM_RADIUS};
use crate::output_rect::ScaleMode;

/// Largest `exposure_ev`; the smallest is its negative.
pub const MAX_EXPOSURE: f32 = 16.0;
/// Smallest knee of [`Tonemap::Soft`].
pub const MIN_TONEMAP_KNEE: f32 = 0.5;
/// Largest knee of [`Tonemap::Soft`].
pub const MAX_TONEMAP_KNEE: f32 = 0.95;
/// Largest saturation.
pub const MAX_SATURATION: f32 = 4.0;
/// Largest contrast.
pub const MAX_CONTRAST: f32 = 4.0;
/// Largest tint component.
pub const MAX_TINT: f32 = 16.0;
/// Largest vignette radius and softness (half shorter scene sides).
pub const MAX_VIGNETTE_DISTANCE: f32 = 4.0;
/// Largest bloom threshold.
pub const MAX_BLOOM_THRESHOLD: f32 = 64.0;
/// Largest bloom intensity.
pub const MAX_BLOOM_INTENSITY: f32 = 8.0;
/// Largest CRT scanline period.
pub const MAX_SCANLINE_PERIOD: f32 = 64.0;
/// Smallest CRT mask pitch.
pub const MIN_MASK_PITCH: f32 = 2.0;
/// Largest CRT mask pitch.
pub const MAX_MASK_PITCH: f32 = 16.0;
/// Smallest CRT brightness.
pub const MIN_CRT_BRIGHTNESS: f32 = 0.5;
/// Largest CRT brightness.
pub const MAX_CRT_BRIGHTNESS: f32 = 4.0;

/// Texture filter for scaling the scene to the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Blocky.
    Nearest,
    /// Smooth.
    #[default]
    Linear,
}

/// Colour space of the scene colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// sRGB-encoded (default).
    #[default]
    Srgb,
    /// Linear.
    Linear,
}

/// Tone curve applied after the exposure and bloom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tonemap {
    /// No tone curve (default).
    #[default]
    None,
    /// Linear up to `knee`, then a soft shoulder; `knee` in `[0.5, 0.95]`.
    Soft {
        /// Where the shoulder begins.
        knee: f32,
    },
    /// ACES fit.
    Aces,
}

/// Darkening toward the edges of the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignetteSettings {
    /// Strength, `[0, 1]`; 0 (default) is off.
    pub strength: f32,
    /// Distance from the centre where it starts, in half shorter scene sides, `[0, 4]`.
    pub radius: f32,
    /// Width of the transition (smoothstep), same units, `[0, 4]`; 0 is a hard edge.
    pub softness: f32,
    /// Colour, each channel in `[0, 1]`; mixed in display (sRGB) values.
    pub color: [f32; 3],
}

impl Default for VignetteSettings {
    fn default() -> Self {
        Self {
            strength: 0.0,
            radius: 0.5,
            softness: 1.5,
            color: [0.0; 3],
        }
    }
}

/// Output stage: scaling, exposure, tonemap, grade, vignette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputSettings {
    /// How the scene fills the target.
    pub scale_mode: ScaleMode,
    /// Filter of the scaling.
    pub filter: Filter,
    /// Colour space of the scene.
    pub scene_color_space: ColorSpace,
    /// Colour of the letterbox bars, sRGB-encoded display values in `[0, 1]`.
    pub letterbox: [f32; 3],
    /// Exposure in stops, `[-16, 16]`.
    pub exposure_ev: f32,
    /// Tone curve.
    pub tonemap: Tonemap,
    /// Saturation, `[0, 4]`; 1 unchanged.
    pub saturation: f32,
    /// Contrast, `[0, 4]`; 1 unchanged.
    pub contrast: f32,
    /// Tint multiplied onto the linear colour, each channel in `[0, 16]`.
    pub tint: [f32; 3],
    /// Vignette.
    pub vignette: VignetteSettings,
    /// Ordered 8x8 dither of +-0.5 LSB before the 8-bit quantization (UNORM and sRGB 8-bit
    /// targets only).
    pub dither: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            scale_mode: ScaleMode::Fit,
            filter: Filter::Linear,
            scene_color_space: ColorSpace::Srgb,
            letterbox: [0.0; 3],
            exposure_ev: 0.0,
            tonemap: Tonemap::None,
            saturation: 1.0,
            contrast: 1.0,
            tint: [1.0; 3],
            vignette: VignetteSettings::default(),
            dither: false,
        }
    }
}

/// Bloom: the glow of bright HDR pixels spreads into their surroundings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    /// Off (default): no bloom pass runs.
    pub enabled: bool,
    /// HDR brightness from which a pixel glows, `[0, 64]`.
    pub threshold: f32,
    /// Soft transition below the threshold as a share of it, `[0, 1]`.
    pub knee: f32,
    /// Strength of the glow, `[0, 8]`.
    pub intensity: f32,
    /// Reach of the glow in scene pixels, `[1, 256]`.
    pub radius: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 1.0,
            knee: 0.5,
            intensity: 0.6,
            radius: 16.0,
        }
    }
}

/// CRT look: curvature, scanlines, phosphor mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrtSettings {
    /// The overlay (HUD) lies on the curved screen too: warped with the scene, not drawn outside
    /// it. Only with a bend (`curvature` or `corner_radius` > 0).
    pub overlay: bool,
    /// Barrel bend, `[0, 1]`.
    pub curvature: f32,
    /// Corner rounding, `[0, 1]`.
    pub corner_radius: f32,
    /// Scanline strength, `[0, 1]`.
    pub scanlines: f32,
    /// Scene pixels per scanline, `(0, 64]`.
    pub scanline_period: f32,
    /// Phosphor mask strength, `[0, 1]`.
    pub mask: f32,
    /// Target pixels per mask triad, `[2, 16]`.
    pub mask_pitch: f32,
    /// Brightness compensation, `[0.5, 4]`.
    pub brightness: f32,
}

impl Default for CrtSettings {
    /// The CRT off.
    fn default() -> Self {
        Self {
            overlay: false,
            curvature: 0.0,
            corner_radius: 0.0,
            scanlines: 0.0,
            scanline_period: 1.0,
            mask: 0.0,
            mask_pitch: 3.0,
            brightness: 1.0,
        }
    }
}

/// A CRT look to start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CrtPreset {
    /// No effect.
    Off,
    /// Light scanlines.
    Subtle,
    /// Scanlines and mask, brighter.
    Arcade,
}

impl CrtSettings {
    /// A preset. Flat on purpose: the bend is its own opt-in (`curvature`).
    pub fn preset(preset: CrtPreset) -> Self {
        let off = Self::default();
        match preset {
            CrtPreset::Off => off,
            CrtPreset::Subtle => Self {
                scanlines: 0.3,
                brightness: 1.15,
                ..off
            },
            CrtPreset::Arcade => Self {
                scanlines: 0.55,
                mask: 0.3,
                brightness: 1.5,
                ..off
            },
        }
    }

    /// Checks every field.
    pub fn validate(&self) -> Result<(), SettingsError> {
        range(
            "crt.curvature",
            self.curvature,
            0.0,
            1.0,
            "must be in [0, 1]",
        )?;
        range(
            "crt.corner_radius",
            self.corner_radius,
            0.0,
            1.0,
            "must be in [0, 1]",
        )?;
        range(
            "crt.scanlines",
            self.scanlines,
            0.0,
            1.0,
            "must be in [0, 1]",
        )?;
        check(
            "crt.scanline_period",
            self.scanline_period,
            self.scanline_period > 0.0 && self.scanline_period <= MAX_SCANLINE_PERIOD,
            "must be in (0, 64]",
        )?;
        range("crt.mask", self.mask, 0.0, 1.0, "must be in [0, 1]")?;
        range(
            "crt.mask_pitch",
            self.mask_pitch,
            MIN_MASK_PITCH,
            MAX_MASK_PITCH,
            "must be in [2, 16]",
        )?;
        range(
            "crt.brightness",
            self.brightness,
            MIN_CRT_BRIGHTNESS,
            MAX_CRT_BRIGHTNESS,
            "must be in [0.5, 4]",
        )
    }
}

/// All settings of the post-processing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostSettings {
    /// Output stage.
    pub output: OutputSettings,
    /// Bloom.
    pub bloom: BloomSettings,
    /// CRT.
    pub crt: CrtSettings,
}

knorke_types::settings_error!(
    "knorke.post.settings.not_finite",
    "knorke.post.settings.out_of_range"
);

fn color(
    field: &'static str,
    c: [f32; 3],
    hi: f32,
    rule: &'static str,
) -> Result<(), SettingsError> {
    c.into_iter()
        .try_for_each(|v| range(field, v, 0.0, hi, rule))
}

impl OutputSettings {
    /// Checks every field.
    pub fn validate(&self) -> Result<(), SettingsError> {
        color(
            "output.letterbox",
            self.letterbox,
            1.0,
            "components must be in [0, 1]",
        )?;
        range(
            "output.exposure_ev",
            self.exposure_ev,
            -MAX_EXPOSURE,
            MAX_EXPOSURE,
            "must be in [-16, 16]",
        )?;
        if let Tonemap::Soft { knee } = self.tonemap {
            range(
                "output.tonemap.knee",
                knee,
                MIN_TONEMAP_KNEE,
                MAX_TONEMAP_KNEE,
                "must be in [0.5, 0.95]",
            )?;
        }
        range(
            "output.saturation",
            self.saturation,
            0.0,
            MAX_SATURATION,
            "must be in [0, 4]",
        )?;
        range(
            "output.contrast",
            self.contrast,
            0.0,
            MAX_CONTRAST,
            "must be in [0, 4]",
        )?;
        color(
            "output.tint",
            self.tint,
            MAX_TINT,
            "components must be in [0, 16]",
        )?;
        let v = &self.vignette;
        range(
            "output.vignette.strength",
            v.strength,
            0.0,
            1.0,
            "must be in [0, 1]",
        )?;
        range(
            "output.vignette.radius",
            v.radius,
            0.0,
            MAX_VIGNETTE_DISTANCE,
            "must be in [0, 4]",
        )?;
        range(
            "output.vignette.softness",
            v.softness,
            0.0,
            MAX_VIGNETTE_DISTANCE,
            "must be in [0, 4]",
        )?;
        color(
            "output.vignette.color",
            v.color,
            1.0,
            "components must be in [0, 1]",
        )
    }
}

impl BloomSettings {
    /// Checks every field, also while bloom is off.
    pub fn validate(&self) -> Result<(), SettingsError> {
        range(
            "bloom.threshold",
            self.threshold,
            0.0,
            MAX_BLOOM_THRESHOLD,
            "must be in [0, 64]",
        )?;
        range("bloom.knee", self.knee, 0.0, 1.0, "must be in [0, 1]")?;
        range(
            "bloom.intensity",
            self.intensity,
            0.0,
            MAX_BLOOM_INTENSITY,
            "must be in [0, 8]",
        )?;
        range(
            "bloom.radius",
            self.radius,
            MIN_BLOOM_RADIUS,
            MAX_BLOOM_RADIUS,
            "must be in [1, 256]",
        )
    }
}

impl PostSettings {
    /// Checks every field; the first violation is returned.
    pub fn validate(&self) -> Result<(), SettingsError> {
        self.output.validate()?;
        self.bloom.validate()?;
        self.crt.validate()
    }

    /// Validates `new` and, only if it is valid, makes it the settings. On error nothing changes.
    pub fn replace(&mut self, new: PostSettings) -> Result<(), SettingsError> {
        new.validate()?;
        *self = new;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad(mutate: impl FnOnce(&mut PostSettings), field: &str) -> SettingsError {
        let mut s = PostSettings::default();
        mutate(&mut s);
        let e = s.validate().expect_err(field);
        let (SettingsError::NotFinite { field: f, .. }
        | SettingsError::OutOfRange { field: f, .. }) = e;
        assert_eq!(f, field);
        e
    }

    #[test]
    fn defaults_are_valid_and_match_chiaro() {
        let s = PostSettings::default();
        assert_eq!(s.validate(), Ok(()));
        let o = s.output;
        assert_eq!(o.scale_mode, ScaleMode::Fit);
        assert_eq!(o.filter, Filter::Linear);
        assert_eq!(o.scene_color_space, ColorSpace::Srgb);
        assert_eq!(o.tonemap, Tonemap::None);
        assert_eq!((o.exposure_ev, o.saturation, o.contrast), (0.0, 1.0, 1.0));
        assert_eq!(o.tint, [1.0; 3]);
        assert_eq!(
            (o.vignette.strength, o.vignette.radius, o.vignette.softness),
            (0.0, 0.5, 1.5)
        );
        assert!(!o.dither);
        let b = s.bloom;
        assert_eq!(
            (b.enabled, b.threshold, b.knee, b.intensity, b.radius),
            (false, 1.0, 0.5, 0.6, 16.0)
        );
        assert_eq!(s.crt, CrtSettings::preset(CrtPreset::Off));
        assert_eq!(s.crt.scanline_period, 1.0);
        assert_eq!(s.crt.mask_pitch, 3.0);
    }

    #[test]
    fn crt_presets_have_the_chiaro_values_and_are_valid() {
        let subtle = CrtSettings::preset(CrtPreset::Subtle);
        assert_eq!(
            (subtle.scanlines, subtle.brightness, subtle.mask),
            (0.3, 1.15, 0.0)
        );
        let arcade = CrtSettings::preset(CrtPreset::Arcade);
        assert_eq!(
            (arcade.scanlines, arcade.mask, arcade.brightness),
            (0.55, 0.3, 1.5)
        );
        for p in [CrtPreset::Off, CrtPreset::Subtle, CrtPreset::Arcade] {
            let c = CrtSettings::preset(p);
            assert_eq!(c.curvature, 0.0, "flat on purpose");
            assert_eq!(c.validate(), Ok(()));
        }
    }

    #[test]
    fn output_ranges_are_enforced() {
        bad(|s| s.output.letterbox = [0.0, 1.1, 0.0], "output.letterbox");
        bad(|s| s.output.exposure_ev = 16.5, "output.exposure_ev");
        bad(
            |s| s.output.tonemap = Tonemap::Soft { knee: 0.4 },
            "output.tonemap.knee",
        );
        bad(
            |s| s.output.tonemap = Tonemap::Soft { knee: 0.96 },
            "output.tonemap.knee",
        );
        bad(|s| s.output.saturation = 4.1, "output.saturation");
        bad(|s| s.output.contrast = -0.1, "output.contrast");
        bad(|s| s.output.tint = [1.0, 17.0, 1.0], "output.tint");
        bad(
            |s| s.output.vignette.strength = 1.5,
            "output.vignette.strength",
        );
        bad(|s| s.output.vignette.radius = 4.5, "output.vignette.radius");
        bad(
            |s| s.output.vignette.softness = -1.0,
            "output.vignette.softness",
        );
        bad(
            |s| s.output.vignette.color = [2.0, 0.0, 0.0],
            "output.vignette.color",
        );
        // The knee only counts for the soft curve.
        let mut s = PostSettings::default();
        s.output.tonemap = Tonemap::Soft { knee: 0.95 };
        assert_eq!(s.validate(), Ok(()));
        s.output.tonemap = Tonemap::Aces;
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn bloom_is_checked_even_while_off() {
        assert!(!PostSettings::default().bloom.enabled);
        bad(|s| s.bloom.radius = 0.5, "bloom.radius");
        bad(|s| s.bloom.radius = 257.0, "bloom.radius");
        bad(|s| s.bloom.threshold = 65.0, "bloom.threshold");
        bad(|s| s.bloom.knee = 1.5, "bloom.knee");
        bad(|s| s.bloom.intensity = 9.0, "bloom.intensity");
    }

    #[test]
    fn crt_ranges_are_enforced() {
        bad(|s| s.crt.curvature = 1.5, "crt.curvature");
        bad(|s| s.crt.corner_radius = -0.1, "crt.corner_radius");
        bad(|s| s.crt.scanlines = 2.0, "crt.scanlines");
        bad(|s| s.crt.scanline_period = 0.0, "crt.scanline_period");
        bad(|s| s.crt.scanline_period = 65.0, "crt.scanline_period");
        bad(|s| s.crt.mask = 1.1, "crt.mask");
        bad(|s| s.crt.mask_pitch = 1.0, "crt.mask_pitch");
        bad(|s| s.crt.brightness = 0.4, "crt.brightness");
    }

    #[test]
    fn errors_carry_stable_codes_and_paths() {
        let e = bad(|s| s.bloom.radius = f32::NAN, "bloom.radius");
        assert_eq!(e.code().as_str(), "knorke.post.settings.not_finite");
        let e = bad(|s| s.crt.mask_pitch = 1.0, "crt.mask_pitch");
        assert_eq!(e.code().as_str(), "knorke.post.settings.out_of_range");
        assert_eq!(
            e.to_string(),
            "settings.crt.mask_pitch must be in [2, 16] (got 1)"
        );
        let d = e.to_diagnostic();
        assert_eq!(d.path.as_deref(), Some("settings.crt.mask_pitch"));
    }

    #[test]
    fn replace_is_atomic() {
        let mut s = PostSettings::default();
        let mut good = PostSettings::default();
        good.output.exposure_ev = 1.5;
        assert_eq!(s.replace(good), Ok(()));
        assert_eq!(s, good);
        let mut wrong = PostSettings::default();
        wrong.crt.brightness = 0.0;
        assert!(s.replace(wrong).is_err());
        assert_eq!(s, good);
    }
}
