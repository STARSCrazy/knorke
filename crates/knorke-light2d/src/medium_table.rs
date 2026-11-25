// Origin: chiaro @ 9bc5b9b, src/lighting/MediumTable.hpp and MediumTable.cpp (MediumPreset,
// ValidateMedium, MediumTable) and CHI_MediumDesc in include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The media table (P12): the kinds of medium (smoke, fog, ...) the medium layer picks per scene
//! pixel. The medium stage (P30) reads [`MediumTable::entries`] as colour and extinction per entry.
//!
//! Redesign against the C++ version: presets are an enum ([`MediumPreset`]), so "is this a preset"
//! and "unknown preset gives dust" are gone; failures are a [`MediumError`] with a stable code
//! instead of an SDL error string; the colour is a [`Vec3`]. The table is host configuration, like
//! the settings (ADR 0016), and has no snapshot. The limit of `|anisotropy|` is the haze's
//! ([`MAX_ANISOTROPY`]).

use crate::codec::MEDIUM_COUNT;
use crate::settings::MAX_ANISOTROPY;
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

/// A kind of medium: what floats in the air and swallows or scatters light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediumDesc {
    /// Albedo, each channel in `[0, 1]`: the share of the extinguished light that scatters. Soot is
    /// near 0 (swallows), steam near 1 (scatters).
    pub color: Vec3,
    /// Extinction per world unit at density 1, `>= 0`.
    pub extinction: f32,
    /// Henyey-Greenstein `g` of the phase function, `[-0.9, 0.9]`: `> 0` scatters forward, `< 0`
    /// backward, 0 evenly.
    pub anisotropy: f32,
}

impl MediumDesc {
    /// A description; checked by [`MediumTable::set`], not here.
    pub const fn new(color: Vec3, extinction: f32, anisotropy: f32) -> Self {
        Self {
            color,
            extinction,
            anisotropy,
        }
    }

    /// Checks the fields in the order extinction, anisotropy, colour.
    ///
    /// # Errors
    /// The first field that is not finite or outside its range.
    pub fn validate(&self) -> Result<(), MediumError> {
        let field = |field, value: f32, ok, rule| {
            if !value.is_finite() {
                Err(MediumError::NotFinite { field, value })
            } else if ok {
                Ok(())
            } else {
                Err(MediumError::OutOfRange { field, rule, value })
            }
        };
        let e = self.extinction;
        field("extinction", e, e >= 0.0, "must be >= 0")?;
        let g = self.anisotropy;
        field(
            "anisotropy",
            g,
            g.abs() <= MAX_ANISOTROPY,
            "must be in [-0.9, 0.9]",
        )?;
        for c in [self.color.x, self.color.y, self.color.z] {
            field(
                "color",
                c,
                (0.0..=1.0).contains(&c),
                "components must be in [0, 1]",
            )?;
        }
        Ok(())
    }
}

/// The ready-made media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediumPreset {
    /// Brownish, albedo (0.55, 0.5, 0.42), extinction 0.5, anisotropy 0.5.
    Dust,
    /// Grey, albedo 0.35, extinction 1, anisotropy 0.3.
    Smoke,
    /// Nearly black, albedo 0.05, extinction 2, anisotropy 0.2.
    Soot,
    /// White, albedo 0.95, extinction 0.6, anisotropy 0.8.
    Steam,
    /// Light grey, albedo 0.9, extinction 0.6, anisotropy 0.7.
    Fog,
}

impl MediumPreset {
    /// In the order of the first entries of a new [`MediumTable`].
    pub const ALL: [Self; 5] = [Self::Dust, Self::Smoke, Self::Soot, Self::Steam, Self::Fog];

    /// The preset's values. Extinctions are per metre (1 world unit = 1 m), tuned on Chiaro's
    /// smoke and night demos.
    pub const fn desc(self) -> MediumDesc {
        let (grey, extinction, anisotropy) = match self {
            Self::Dust => return MediumDesc::new(Vec3::new(0.55, 0.5, 0.42), 0.5, 0.5),
            Self::Smoke => (0.35, 1.0, 0.3),
            Self::Soot => (0.05, 2.0, 0.2),
            Self::Steam => (0.95, 0.6, 0.8),
            Self::Fog => (0.9, 0.6, 0.7),
        };
        MediumDesc::new(Vec3::new(grey, grey, grey), extinction, anisotropy)
    }
}

/// Why a medium or an index was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediumError {
    /// The index is not below [`MEDIUM_COUNT`].
    Index(u32),
    /// A value is NaN or infinite.
    NotFinite {
        /// `extinction`, `anisotropy` or `color`.
        field: &'static str,
        /// The received value.
        value: f32,
    },
    /// A value is outside its range.
    OutOfRange {
        /// `extinction`, `anisotropy` or `color`.
        field: &'static str,
        /// The rule.
        rule: &'static str,
        /// The received value.
        value: f32,
    },
}

impl MediumError {
    /// The stable code: `knorke.light2d.medium.not_finite` or `...out_of_range` (also for an
    /// index, like [`MediumOutOfRange`](crate::codec::MediumOutOfRange)).
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::NotFinite { .. } => DiagnosticCode::new("knorke.light2d.medium.not_finite"),
            Self::Index(_) | Self::OutOfRange { .. } => {
                DiagnosticCode::new("knorke.light2d.medium.out_of_range")
            }
        }
    }

    /// As a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.to_string())
    }
}

impl core::fmt::Display for MediumError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Index(i) => write!(f, "medium index must be below {MEDIUM_COUNT} (got {i})"),
            Self::NotFinite { field, value } => {
                write!(f, "medium.{field} must be finite (got {value})")
            }
            Self::OutOfRange { field, rule, value } => {
                write!(f, "medium.{field} {rule} (got {value})")
            }
        }
    }
}

impl core::error::Error for MediumError {}

/// The media table of a renderer: [`MEDIUM_COUNT`] kinds of medium, picked per scene pixel by the
/// medium layer. A new table holds the presets in entries 0 to 4 and dust in 5 to 15.
#[derive(Clone, Debug, PartialEq)]
pub struct MediumTable {
    entries: [MediumDesc; MEDIUM_COUNT as usize],
}

impl Default for MediumTable {
    fn default() -> Self {
        Self::new()
    }
}

impl MediumTable {
    /// The presets, then dust.
    pub fn new() -> Self {
        let dust = MediumPreset::Dust.desc();
        let mut entries = [dust; MEDIUM_COUNT as usize];
        for (entry, preset) in entries.iter_mut().zip(MediumPreset::ALL) {
            *entry = preset.desc();
        }
        Self { entries }
    }

    /// All entries, for the medium stage's uniform block.
    pub fn entries(&self) -> &[MediumDesc; MEDIUM_COUNT as usize] {
        &self.entries
    }

    /// Entry `index`.
    ///
    /// # Errors
    /// [`MediumError::Index`] if `index >= MEDIUM_COUNT`.
    pub fn get(&self, index: u32) -> Result<&MediumDesc, MediumError> {
        self.entries
            .get(index as usize)
            .ok_or(MediumError::Index(index))
    }

    /// Replaces entry `index`; unchanged on error.
    ///
    /// # Errors
    /// The index is out of range or the description violates a rule ([`MediumDesc::validate`]).
    pub fn set(&mut self, index: u32, medium: MediumDesc) -> Result<(), MediumError> {
        let entry = self
            .entries
            .get_mut(index as usize)
            .ok_or(MediumError::Index(index))?;
        medium.validate()?;
        *entry = medium;
        Ok(())
    }
}
