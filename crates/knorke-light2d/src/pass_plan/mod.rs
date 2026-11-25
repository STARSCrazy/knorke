// Origin: chiaro @ 9bc5b9b, src/core/Context.cpp (BuildFrame, RunStages, TakeStats),
// src/rendering/OcclusionStage.cpp (Prepare: AnyShadowCaster, ScanObstacles) and
// src/rendering/Frame.hpp (FrameInputs, the stage results) as the specification of the stage
// order and the resource flow; the code is a redesign, not a port of `Context`.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The pass plan (P31): the fixed order of the 2D light stages as one type.
//!
//! | Step | Stage | Reads | Writes |
//! |---|---|---|---|
//! | 1 | [`OccluderGeometry`](crate::occluder_raster::OccluderGeometry), [`TileOccluders`](crate::occluder_raster::TileOccluders) (P22) | obstacle store | vertex and cell buffers |
//! | 2 | [`OcclusionMask`](crate::occlusion::OcclusionMask) (P23) | step 1, sprite mask | mask, height, distance field |
//! | 3 | [`LightAtlases`](crate::atlas::LightAtlases) (P26) | profile and mask store | atlases |
//! | 4 | [`LightingStage`](crate::lighting::LightingStage) with the zone pass (P25, P26) and the ring light pass (P21) | light list, zone list, ring lights, steps 2 and 3 | light, specular, direction, pure ambient |
//! | 5 | [`CompositeStage`](crate::composite::CompositeStage) (P27) | scene buffers, step 4 | HDR buffer |
//! | 6 | [`DebugCheckStage`](crate::debug_check::DebugCheckStage) (P32, opt-in) | surface and material buffer | findings of an earlier frame |
//!
//! Per frame the host calls [`PassPlan::prepare`] (may fail; culls the lights and zones, decides
//! every stage and creates what the frame needs) and then [`PassPlan::record`] into the command
//! buffer it lends; the HDR result is [`PassPlan::hdr`]. Output (tonemap, bloom, the target) is
//! not part of this crate: `knorke-post` reads the HDR buffer, `knorke-integration` joins the two
//! (ADR 0014).
//!
//! # Redesign against Chiaro's `Context`
//!
//! - No context object owning stores and settings: the host keeps its stores (they are its
//!   simulation state and have snapshots) and hands them in by reference each frame
//!   ([`SceneStores`]), together with the scene textures ([`PassFrame`]) and the
//!   [`LightSettings`](crate::settings::LightSettings). The plan owns only what the GPU frame
//!   needs: stages, pipelines, samplers and the reused cull lists.
//! - The plan's state is a cache of GPU contents, not simulation state. It has no snapshot: a
//!   plan built anew from restored stores draws the same image (tested), and after a restore of
//!   the stores into a running plan the host calls [`PassPlan::invalidate`].
//! - No output stage, no submit or present, and no readback (ADR 0002) except the debug check
//!   the host switches on ([`debug_check`](crate::debug_check)). The result is a texture
//!   reference into the plan's own HDR buffer.
//! - Communication is the fixed order of the stages, each one's result passed on as a value; no
//!   event bus and no callbacks. Frame warnings ([`frame_warnings`](crate::frame_warnings)) are
//!   evaluated in `record` and returned with it.
//! - The caster class masks of filtered shadows (E53) are part of the occlusion stage: no
//!   extra input, nothing extra without a visible filtered light.
//! - Pattern obstacles (P28) are part of the occlusion stage: [`SceneStores::patterns`] gives
//!   the mask their mean opacities and, with a pattern obstacle in the domain, adds the pattern
//!   atlas, the id texture and the `Pattern` shadow programs; without one nothing runs.
//! - Ring lights of shockwaves (P21) come culled in [`PassFrame::ring_lights`] (the waves and
//!   their culling are `knorke-post`'s); step 4 draws them after the lights.
//! - Not built (and not blocked): medium (P12, P29, P30). Its inputs are fields still to come
//!   in [`PassFrame`] and [`SceneStores`] and stages between steps 2 and 5; the stages already
//!   take `None` for them.

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{
    INVALID_FRAME, INVARIANT, NOT_PREPARED, OCCLUSION_PROGRAMS, PassFrame, PassPlan, PassPrepared,
    PassRecord, SceneStores,
};

use crate::light_culling::LightList;
use crate::shadow::MaskUpdate;

/// The stages a frame ran, as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PassStages(u8);

impl PassStages {
    /// No stage.
    pub const NONE: Self = Self(0);
    /// Obstacle raster, occlusion mask and distance field (steps 1 and 2).
    pub const OCCLUSION: Self = Self(1);
    /// Profile or mask atlas (step 3).
    pub const ATLASES: Self = Self(2);
    /// Ambient zone pass (step 4, before the lights).
    pub const ZONES: Self = Self(4);
    /// Light pass (step 4).
    pub const LIGHTS: Self = Self(8);
    /// Composite into the HDR buffer (step 5); every successful frame runs it.
    pub const COMPOSITE: Self = Self(16);
    /// GPU debug check (after the composite; only with `DebugChecks::LAYERS`).
    pub const DEBUG_CHECK: Self = Self(32);

    /// `true` if every stage of `other` is in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Adds the stages of `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// The bits (for inspection).
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Counters over the plan's lifetime: how often the occlusion cache was reused, updated locally
/// or rebuilt, and how often a frame failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Frames recorded.
    pub frames: u64,
    /// Frames whose prepare failed.
    pub failed: u64,
    /// Frames with shadows whose mask was reused unchanged (cache hit).
    pub mask_reused: u64,
    /// Frames with shadows whose mask was updated in the changed rectangles.
    pub mask_incremental: u64,
    /// Frames with shadows whose mask was rebuilt.
    pub mask_full: u64,
}

impl PassStats {
    /// Counts one recorded frame with the mask update `update` (`None`: no shadows).
    pub fn count(&mut self, update: Option<MaskUpdate>) {
        self.frames += 1;
        match update {
            Some(MaskUpdate::Reused) => self.mask_reused += 1,
            Some(MaskUpdate::Incremental) => self.mask_incremental += 1,
            Some(MaskUpdate::Full) => self.mask_full += 1,
            None => {}
        }
    }
}

/// Chiaro `AnyShadowCaster` (OcclusionStage.cpp): a visible light needs the occlusion mask, a
/// shadow caster of any kind or a caster class filter (E53).
pub fn any_shadow_caster(list: &LightList) -> bool {
    !(list.shadowed.is_empty()
        && list.shadowed_area.is_empty()
        && list.masked_shadowed.is_empty()
        && list.directional_shadows == 0
        && list.caster_classes == 0)
}

/// The highest height a lit pixel can have (Chiaro `Context::BuildFrame`): 0 without a surface
/// buffer; with one, the highest finite obstacle top, or the highest encodable height when
/// there is no finite top, an infinitely tall obstacle or a sprite mask (unknown height).
pub fn max_height(
    has_surface: bool,
    max_finite_top: f32,
    any_infinite: bool,
    sprite: bool,
    surface_height_step: f32,
) -> f32 {
    if !has_surface {
        0.0
    } else if max_finite_top <= 0.0 || any_infinite || sprite {
        255.0 * surface_height_step
    } else {
        max_finite_top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_a_bit_set() {
        let mut s = PassStages::NONE;
        s.insert(PassStages::LIGHTS);
        s.insert(PassStages::COMPOSITE);
        assert!(s.contains(PassStages::LIGHTS));
        assert!(!s.contains(PassStages::ZONES));
        assert_eq!(s.bits(), 24);
    }

    #[test]
    fn stats_count_the_mask_updates() {
        let mut s = PassStats::default();
        s.count(Some(MaskUpdate::Full));
        s.count(Some(MaskUpdate::Reused));
        s.count(None);
        assert_eq!(
            s,
            PassStats {
                frames: 3,
                failed: 0,
                mask_reused: 1,
                mask_incremental: 0,
                mask_full: 1
            }
        );
    }

    #[test]
    fn max_height_follows_build_frame() {
        assert_eq!(max_height(false, 10.0, false, false, 0.5), 0.0);
        assert_eq!(max_height(true, 10.0, false, false, 0.5), 10.0);
        assert_eq!(max_height(true, 0.0, false, false, 0.5), 127.5);
        assert_eq!(max_height(true, 10.0, true, false, 0.5), 127.5);
        assert_eq!(max_height(true, 10.0, false, true, 0.5), 127.5);
    }

    #[test]
    fn no_caster_without_shadowed_lights() {
        assert!(!any_shadow_caster(&LightList::default()));
        let list = LightList {
            directional_shadows: 1,
            ..Default::default()
        };
        assert!(any_shadow_caster(&list));
    }
}
