// Origin: chiaro @ 9bc5b9b, src/rendering/DebugCheckStage.hpp and DebugCheckStage.cpp
// (DebugCheckParams, the meaning of the four finding bytes, kSlots, kBlowOutLimit).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU debug check (P32, Chiaro E83): half-alpha surface and material texels, blown-out
//! highlights and medium pixels without a height span, found by the GPU and read back a few
//! frames later.
//!
//! It runs only with [`DebugChecks::LAYERS`](crate::frame_warnings::DebugChecks::LAYERS) and a
//! surface, material or medium buffer. That switch is the host asking for the readback (ADR
//! 0002): the stage records a 1x1 download into the host's command buffer and maps it
//! [`SLOTS`] recorded frames later, without a submit, a fence or a wait of its own. Off, the
//! stage creates and records nothing (ADR 0011).

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{DEBUG_CHECK_PROGRAM, DebugCheckFrame, DebugCheckStage, FINDINGS_FORMAT};

use knorke_math::uniform_block;

use crate::frame_warnings::{Warning, WarningSet};
use crate::settings::LightSettings;

/// Readback slots: a finding arrives this many recorded frames after its frame, when the GPU
/// has long finished it.
pub const SLOTS: usize = 4;

/// A highlight peak above this many times the diffuse light warns: just above strength 1,
/// gloss 1, scale 1 (32.25).
pub const BLOW_OUT_LIMIT: f32 = 33.0;

uniform_block! {
    /// `cbuffer DebugCheckParams` of `debug_check.vert`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct DebugCheckParams[48] {
        /// xy scene size in texels, z 1 = surface buffer bound, w 1 = material buffer bound.
        pub size_present: [f32; 4],
        /// x specular scale, y wet stretch (1 off), z wet breakup, w blow-out limit.
        pub highlight: [f32; 4],
        /// x 1 = medium buffer bound.
        pub medium: [f32; 4],
    }
}

impl DebugCheckParams {
    /// The block of a frame of `size` scene texels with the given buffers bound.
    pub fn new(
        size: (u32, u32),
        surface: bool,
        material: bool,
        medium: bool,
        settings: &LightSettings,
    ) -> Self {
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        let wet = settings.material.wet_mapping();
        Self {
            size_present: [size.0 as f32, size.1 as f32, flag(surface), flag(material)],
            highlight: [
                settings.material.specular_scale,
                wet.stretch,
                wet.breakup,
                BLOW_OUT_LIMIT,
            ],
            medium: [flag(medium), 0.0, 0.0, 0.0],
        }
    }
}

/// The findings in the 1x1 target's texel: R half-alpha surface, G half-alpha material, B
/// blown-out highlight, A flat medium; non-zero means found somewhere in the frame.
pub fn findings_of(texel: [u8; 4]) -> WarningSet {
    let mut found = WarningSet::EMPTY;
    for (hit, warning) in [
        (texel[0] != 0 || texel[1] != 0, Warning::HalfAlpha),
        (texel[2] != 0, Warning::BlownOutHighlights),
        (texel[3] != 0, Warning::FlatMedium),
    ] {
        if hit {
            found.insert(warning);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::MaterialSettings;

    #[test]
    fn texel_channels_map_to_warnings() {
        let one = WarningSet::of;
        assert!(findings_of([0; 4]).is_empty());
        assert_eq!(findings_of([255, 0, 0, 0]), one(Warning::HalfAlpha));
        assert_eq!(findings_of([0, 1, 0, 0]), one(Warning::HalfAlpha));
        assert_eq!(
            findings_of([0, 0, 255, 0]),
            one(Warning::BlownOutHighlights)
        );
        assert_eq!(findings_of([0, 0, 0, 255]), one(Warning::FlatMedium));
        assert_eq!(findings_of([255; 4]).len(), 3);
    }

    /// The block matches `struct DebugCheckParams` of DebugCheckStage.cpp field for field.
    #[test]
    fn params_layout_and_values() {
        let settings = LightSettings {
            material: MaterialSettings {
                specular_scale: 2.0,
                wetness: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let p = DebugCheckParams::new((32, 24), false, true, false, &settings);
        assert_eq!(p.size_present, [32.0, 24.0, 0.0, 1.0]);
        assert_eq!(p.highlight, [2.0, 6.0, 0.65, BLOW_OUT_LIMIT]);
        assert_eq!(p.medium, [0.0; 4]);
        let b = p.to_bytes();
        assert_eq!(b.len(), 48);
        assert_eq!(&b[12..16], &1.0f32.to_le_bytes());
        assert_eq!(&b[28..32], &BLOW_OUT_LIMIT.to_le_bytes());
    }
}
