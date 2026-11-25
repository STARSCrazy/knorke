// Origin: chiaro @ 9bc5b9b, src/core/Context.cpp (BuildFrame, RunStages, TakeStats) and
// src/rendering/OcclusionStage.cpp (Prepare, Record) as the specification of the stage order;
// redesigned, see the module docs.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU side of the pass plan. Ownership follows ADR 0002: the host lends the command buffer
//! and the scene textures; the plan creates its own targets on the host's device and never
//! submits, presents or reads back.

use std::borrow::Cow;

use knorke_gpu::{
    CommandBuffer, Filter, Gpu, PipelineCache, ProgramDesc, Samplers, ShaderLibrary, TextureRef,
};
use knorke_observe::{Inspect, Sink, Visitor};
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{PassStages, PassStats, any_shadow_caster, max_height};
use crate::ambient_zone_culling::{AmbientZoneList, CullScene, collect_ambient_zones};
use crate::ambient_zone_store::AmbientZoneStore;
use crate::animation::AnimationStore;
use crate::atlas::LightAtlases;
use crate::composite::{CompositeFrame, CompositeLight, CompositeStage, InputColorSpace};
use crate::debug_check::{DebugCheckFrame, DebugCheckStage};
use crate::frame_warnings::{DebugChecks, FrameFindings, FrameWarnings, WarningSet};
use crate::light_culling::{LightList, LightModes, collect_lights};
use crate::light_pipelines::LightPipelines;
use crate::light_store::LightStore;
use crate::lighting::{
    DetailMode, LightingFrame, LightingRecord, LightingResult, LightingStage, ShadowMaps,
};
use crate::mask_store::MaskStore;
use crate::obstacle::{ObstacleStore, PatternSource};
use crate::occluder_raster::{
    DomainParams, INFINITE_TOP, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms,
    TileOccluders, thin_circle_radius,
};
use crate::occlusion::{MASK_PROGRAMS, MaskPrograms, MaskRecord, OcclusionMask};
use crate::pattern_occluders::{PATTERN_PROGRAMS, PatternOccluders, PatternPrograms};
use crate::pattern_store::PatternStore;
use crate::profile_store::ProfileStore;
use crate::projection::Projection;
use crate::ring_light::RingLight;
use crate::settings::LightSettings;
use crate::shadow::{MaskAnchor, MaskPlanInput, MaskUpdate, OcclusionDomain};
use crate::zone_pass::ZoneInput;

/// Recording without a successful [`PassPlan::prepare`] since the last record.
pub const NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.pass_plan.not_prepared");
/// A scene buffer of the frame does not have the scene's size.
pub const INVALID_FRAME: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.pass_plan.invalid_frame");
/// An invariant of the plan is broken ([`PassPlan::check_invariants`]).
pub const INVARIANT: DiagnosticCode = DiagnosticCode::new("knorke.light2d.pass_plan.invariant");

const OCCLUSION_LEN: usize = OCCLUDER_PROGRAMS.len() + MASK_PROGRAMS.len() + PATTERN_PROGRAMS.len();

const fn occlusion_programs() -> [ProgramDesc; OCCLUSION_LEN] {
    let mut out = [OCCLUDER_PROGRAMS[0]; OCCLUSION_LEN];
    let mut i = 0;
    while i < OCCLUDER_PROGRAMS.len() {
        out[i] = OCCLUDER_PROGRAMS[i];
        i += 1;
    }
    let mut j = 0;
    while j < MASK_PROGRAMS.len() {
        out[i] = MASK_PROGRAMS[j];
        (i, j) = (i + 1, j + 1);
    }
    j = 0;
    while j < PATTERN_PROGRAMS.len() {
        out[i] = PATTERN_PROGRAMS[j];
        (i, j) = (i + 1, j + 1);
    }
    out
}

/// The pipeline table of the obstacle raster, the occlusion mask and the pattern id texture:
/// [`OCCLUDER_PROGRAMS`], [`MASK_PROGRAMS`], then [`PATTERN_PROGRAMS`].
pub const OCCLUSION_PROGRAMS: [ProgramDesc; OCCLUSION_LEN] = occlusion_programs();

/// The scene textures of one frame, all the host's, borrowed for `'a` (Chiaro `CHI_Frame`
/// without the target and the output parts).
#[derive(Clone, Copy, Debug)]
pub struct PassFrame<'a> {
    /// Scene colour (8-bit RGBA, `SAMPLER` usage); its size is the frame size.
    pub scene: TextureRef<'a>,
    /// Glow buffer (8-bit RGBA, scene size).
    pub glow: Option<TextureRef<'a>>,
    /// Material buffer (codec `material_pixel`, RGBA8 UNORM, scene size).
    pub material: Option<TextureRef<'a>>,
    /// Surface buffer (codec `surface_pixel`, RGBA8 UNORM, scene size).
    pub surface: Option<TextureRef<'a>>,
    /// Application sprite mask (alpha at or above 0.5 occludes, infinitely tall; scene size,
    /// E21).
    pub occlusion: Option<TextureRef<'a>>,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Animation time in seconds (from the host's tick, never a clock).
    pub time: f64,
    /// Encoding of the scene and glow colours; the host feeds it from the same setting as the
    /// output's scene colour space.
    pub input_space: InputColorSpace,
    /// Ring lights of the frame's shockwaves, culled by the host (P21); empty: no ring pass.
    pub ring_lights: &'a [RingLight],
}

impl<'a> PassFrame<'a> {
    /// A frame of `scene` under `projection`: no other buffer, time 0, sRGB colours.
    pub fn new(scene: TextureRef<'a>, projection: &'a Projection) -> Self {
        Self {
            scene,
            glow: None,
            material: None,
            surface: None,
            occlusion: None,
            projection,
            time: 0.0,
            input_space: InputColorSpace::Srgb,
            ring_lights: &[],
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.scene.width(), self.scene.height())
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        let size = self.size();
        if size.0 == 0 || size.1 == 0 {
            return Err(Diagnostic::error(INVALID_FRAME, "the scene has no size"));
        }
        for (name, t) in [
            ("glow", self.glow),
            ("material", self.material),
            ("surface", self.surface),
            ("occlusion", self.occlusion),
        ] {
            if let Some(t) = t
                && (t.width(), t.height()) != size
            {
                return Err(Diagnostic::error(
                    INVALID_FRAME,
                    "a scene buffer does not have the scene's size",
                )
                .at(name));
            }
        }
        Ok(())
    }
}

/// The host's stores the frame draws from (its simulation state; the plan only reads them).
#[derive(Clone, Copy)]
pub struct SceneStores<'a> {
    /// The lights.
    pub lights: &'a LightStore,
    /// The obstacles.
    pub obstacles: &'a ObstacleStore,
    /// Ambient zones; `None`: no zone pass.
    pub zones: Option<&'a AmbientZoneStore>,
    /// Light profiles (needed by profiled lights).
    pub profiles: Option<&'a ProfileStore>,
    /// Light masks (needed by masked lights).
    pub masks: Option<&'a MaskStore>,
    /// Light animations.
    pub animations: Option<&'a AnimationStore>,
    /// Obstacle patterns (needed by pattern obstacles); `None`: every pattern obstacle casts its
    /// scalar opacity, and no pattern stage runs.
    pub patterns: Option<&'a PatternStore>,
}

impl<'a> SceneStores<'a> {
    /// Lights and obstacles only.
    pub fn new(lights: &'a LightStore, obstacles: &'a ObstacleStore) -> Self {
        Self {
            lights,
            obstacles,
            zones: None,
            profiles: None,
            masks: None,
            animations: None,
            patterns: None,
        }
    }
}

/// What [`PassPlan::prepare`] decided for the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassPrepared {
    /// The stages the frame runs.
    pub stages: PassStages,
    /// How the occlusion mask is updated; `None`: no shadows.
    pub mask_update: Option<MaskUpdate>,
    /// The light pass's decision.
    pub lighting: LightingResult,
    /// Targets and textures (re)created by this prepare.
    pub allocations: u32,
}

/// What [`PassPlan::record`] recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassRecord {
    /// The occlusion stage (zero without shadows).
    pub mask: MaskRecord,
    /// The light pass.
    pub lighting: LightingRecord,
    /// Warnings of the frame (debug checks, [`PassPlan::set_debug_checks`]).
    pub warnings: WarningSet,
}

/// The prepared frame, until it is recorded.
#[derive(Clone, Copy, Debug)]
struct Pending {
    prepared: PassPrepared,
    shadows: Option<ShadowState>,
    max_height: f32,
    shadow_scale: f32,
}

/// The shadow inputs of the light pass that are not textures.
#[derive(Clone, Copy, Debug)]
struct ShadowState {
    domain: OcclusionDomain,
    anchor: MaskAnchor,
    height_mode: bool,
    any_partial: bool,
    shadow_top: f32,
    scene_max_top: f32,
}

/// The fixed order of the 2D light stages (see the [module docs](super)).
#[derive(Debug)]
pub struct PassPlan<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    light_pipelines: LightPipelines<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    occlusion: OcclusionMask<'d>,
    patterns: PatternOccluders<'d>,
    atlases: LightAtlases<'d>,
    lighting: LightingStage<'d>,
    composite: CompositeStage<'d>,
    debug_check: DebugCheckStage<'d>,
    lights: LightList,
    zones: AmbientZoneList,
    warnings: FrameWarnings,
    checks: DebugChecks,
    pending: Option<Pending>,
    /// The last recorded frame.
    last: Option<PassPrepared>,
    /// The size of the last recorded frame.
    last_size: (u32, u32),
    last_shadow_scale: f32,
    thin_obstacles: u32,
    stats: PassStats,
}

fn not_prepared(what: &str) -> Diagnostic {
    Diagnostic::error(NOT_PREPARED, what.to_owned())
}

impl ShadowState {
    fn maps<'a>(
        &self,
        mask: &'a OcclusionMask<'_>,
        patterns: &'a PatternOccluders<'_>,
    ) -> Result<ShadowMaps<'a>, Diagnostic> {
        Ok(ShadowMaps {
            domain: self.domain,
            remainder: self.anchor.remainder,
            mask: mask
                .mask()
                .ok_or_else(|| not_prepared("the occlusion mask is missing"))?,
            field: mask
                .field()
                .ok_or_else(|| not_prepared("the distance field is missing"))?,
            height: if self.height_mode {
                mask.height()
            } else {
                None
            },
            height_mode: self.height_mode,
            any_partial: self.any_partial,
            shadow_top: self.shadow_top,
            scene_max_top: self.scene_max_top,
            casters: [mask.caster_maps(0), mask.caster_maps(1)],
            patterns: patterns.maps(),
        })
    }
}

fn lighting_frame<'a>(
    frame: &PassFrame<'a>,
    settings: &'a LightSettings,
    max_height: f32,
    lights: &'a LightList,
    shadows: Option<&'a ShadowMaps<'a>>,
    atlases: &'a LightAtlases<'_>,
) -> LightingFrame<'a> {
    LightingFrame {
        scene: frame.scene,
        surface: frame.surface,
        material: frame.material,
        projection: frame.projection,
        settings,
        max_height,
        lights,
        shadows,
        profile_atlas: atlases.profiles.texture(),
        mask_atlas: atlases.masks.texture(),
        ring_lights: frame.ring_lights,
    }
}

fn composite_frame<'a>(
    frame: &PassFrame<'a>,
    settings: &'a LightSettings,
    light: CompositeLight<'a>,
) -> CompositeFrame<'a> {
    CompositeFrame {
        scene: frame.scene,
        glow: frame.glow,
        material: frame.material,
        surface: frame.surface,
        light,
        medium: None,
        projection: frame.projection,
        settings,
        input_space: frame.input_space,
    }
}

fn debug_check_frame<'a>(
    frame: &PassFrame<'a>,
    settings: &'a LightSettings,
) -> DebugCheckFrame<'a> {
    DebugCheckFrame {
        size: frame.size(),
        surface: frame.surface,
        material: frame.material,
        medium: None,
        settings,
    }
}

impl<'d> PassPlan<'d> {
    /// Loads the shader pack (`knorke-chiaro.<format>.kpak`) and creates the stages (without
    /// targets: the first frame creates them).
    ///
    /// # Errors
    /// The pack is invalid or misses a shader of a stage, or a sampler could not be created.
    pub fn new(gpu: Gpu<'d>, pack: impl Into<Cow<'static, [u8]>>) -> Result<Self, Diagnostic> {
        let shaders = ShaderLibrary::new(gpu, pack)?;
        let pipelines = PipelineCache::new(&shaders, &OCCLUSION_PROGRAMS)?;
        let light_pipelines = LightPipelines::new(&shaders)?;
        let composite = CompositeStage::new(gpu, &shaders)?;
        let debug_check = DebugCheckStage::new(gpu, &shaders)?;
        Ok(Self {
            shaders,
            pipelines,
            light_pipelines,
            samplers: Samplers::new(gpu)?,
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            occlusion: OcclusionMask::new(gpu, MaskPrograms::at(OCCLUDER_PROGRAMS.len())),
            patterns: PatternOccluders::new(
                gpu,
                PatternPrograms::at(OCCLUDER_PROGRAMS.len() + MASK_PROGRAMS.len()),
            ),
            atlases: LightAtlases::new(gpu),
            lighting: LightingStage::new(gpu),
            composite,
            debug_check,
            lights: LightList::default(),
            zones: AmbientZoneList::default(),
            warnings: FrameWarnings::new(),
            checks: DebugChecks::NONE,
            pending: None,
            last: None,
            last_size: (0, 0),
            last_shadow_scale: 1.0,
            thin_obstacles: 0,
            stats: PassStats::default(),
        })
    }

    /// The debug checks the frame warnings evaluate (default none: no warnings). `LAYERS` also
    /// switches on the GPU check with its readback ([`debug_check`](crate::debug_check)).
    pub fn set_debug_checks(&mut self, checks: DebugChecks) {
        self.checks = checks;
    }

    /// The occlusion mask's full rebuild ratio (tests and settings; see
    /// [`OcclusionMask::set_full_rebuild_ratio`]).
    pub fn set_full_rebuild_ratio(&mut self, ratio: f64) {
        self.occlusion.set_full_rebuild_ratio(ratio);
    }

    /// Forgets what the GPU caches hold: the next frame rebuilds the occlusion mask. Call it
    /// after restoring the obstacle store from a snapshot into a running plan.
    pub fn invalidate(&mut self) {
        self.occlusion.invalidate();
    }

    /// Culls the lights and zones of the frame and prepares every stage: occlusion (only with a
    /// shadow-casting light and an active obstacle or a sprite mask), atlases, light pass with
    /// the zones, composite. Nothing is recorded; a failed prepare leaves the previous frame's
    /// result (and makes the next frame rebuild the occlusion mask).
    ///
    /// # Errors
    /// Invalid settings (`knorke.light2d.settings.*`), a buffer of the wrong size
    /// ([`INVALID_FRAME`]), or a stage could not create what it needs (its diagnostic).
    pub fn prepare(
        &mut self,
        frame: &PassFrame<'_>,
        stores: &SceneStores<'_>,
        settings: &LightSettings,
    ) -> Result<PassPrepared, Diagnostic> {
        self.pending = None;
        let result = self.prepare_stages(frame, stores, settings);
        if result.is_err() {
            self.stats.failed += 1;
        }
        result
    }

    fn prepare_stages(
        &mut self,
        frame: &PassFrame<'_>,
        stores: &SceneStores<'_>,
        settings: &LightSettings,
    ) -> Result<PassPrepared, Diagnostic> {
        settings.validate().map_err(|e| e.to_diagnostic())?;
        frame.validate()?;
        let (w, h) = frame.size();
        let obstacles = stores.obstacles;
        let sprite = frame.occlusion.is_some();
        let normal_mode = frame.surface.is_some() || frame.material.is_some();
        let max_height = max_height(
            frame.surface.is_some(),
            obstacles.max_finite_top(),
            obstacles.any_infinite_enabled(),
            sprite,
            settings.surface_height_step,
        );
        let modes = LightModes {
            height_mode: obstacles.any_finite_height(),
            normal_mode,
            max_height,
            profiles: stores.profiles.map(|p| p as _),
            animations: stores.animations,
            masks: stores.masks,
            ..Default::default()
        };
        collect_lights(
            stores.lights,
            settings,
            frame.projection,
            frame.time,
            w,
            h,
            &modes,
            &mut self.lights,
        );
        match stores.zones {
            Some(zones) => {
                let a = settings.ambient;
                let d = self.lights.directional_ambient;
                collect_ambient_zones(
                    zones,
                    [a.x, a.y, a.z],
                    &CullScene {
                        projection: *frame.projection,
                        max_height,
                        has_surface: frame.surface.is_some(),
                        width: w,
                        height: h,
                    },
                    [d.x, d.y, d.z],
                    &mut self.zones,
                );
            }
            None => {
                self.zones.zones.clear();
                self.zones.any_polygon = false;
                self.zones.height_spans = 0;
            }
        }

        let mut prepared = PassPrepared {
            stages: PassStages::COMPOSITE,
            ..Default::default()
        };
        // Steps 1 and 2: occlusion (Chiaro OcclusionStage::Prepare).
        let active = obstacles.iter().any(|(_, o)| o.is_active());
        let shadows = if any_shadow_caster(&self.lights) && (active || sprite) {
            let height_mode = obstacles.any_finite_height() || normal_mode;
            let mut input = MaskPlanInput::new(
                obstacles,
                frame.projection,
                w,
                h,
                settings.resolutions().shadow,
            );
            input.cells = obstacles;
            input.sprite = sprite;
            // The mask's mean opacities of pattern obstacles; without a store every pattern is
            // gone and an obstacle writes its scalar opacity.
            let source = |id| stores.patterns.and_then(|p| p.mean_opacity(id));
            let before = self.occlusion.allocations();
            let update = self.occlusion.prepare(
                &mut self.shaders,
                &mut self.pipelines,
                &input,
                height_mode,
            )?;
            self.occlusion.prepare_casters(self.lights.caster_classes)?;
            let mask_drawn = update != MaskUpdate::Reused;
            let min_radius = thin_circle_radius(frame.projection, &input.domain);
            if mask_drawn {
                self.geometry.prepare(
                    &mut self.shaders,
                    &mut self.pipelines,
                    obstacles,
                    &source,
                    height_mode,
                    min_radius,
                )?;
                self.tiles.prepare(
                    &mut self.shaders,
                    &mut self.pipelines,
                    obstacles,
                    &source,
                    height_mode,
                )?;
            }
            prepared.allocations += self.occlusion.allocations() - before;
            prepared.allocations += self.patterns.prepare(
                &mut self.shaders,
                &mut self.pipelines,
                obstacles,
                stores.patterns,
                frame.projection,
                &input.domain,
                &input.anchor,
                min_radius,
                mask_drawn,
            )?;
            prepared.mask_update = Some(update);
            prepared.stages.insert(PassStages::OCCLUSION);
            let scene_max_top = if height_mode {
                obstacles.max_finite_top()
            } else {
                0.0
            };
            Some(ShadowState {
                domain: input.domain,
                anchor: input.anchor,
                height_mode,
                // The crossing path must run to read a pattern (Chiaro E25).
                any_partial: obstacles.any_partial() || self.patterns.is_active(),
                shadow_top: if sprite || obstacles.any_infinite_enabled() {
                    INFINITE_TOP
                } else {
                    scene_max_top
                },
                scene_max_top,
            })
        } else {
            None
        };

        // Step 3: the atlases the light list needs.
        prepared.allocations +=
            self.atlases
                .prepare(&self.lights, stores.profiles, stores.masks)?;
        if self.lights.profiled_lights > 0 || self.lights.masked_lights > 0 {
            prepared.stages.insert(PassStages::ATLASES);
        }

        // Step 4: zones and lights.
        let maps = match &shadows {
            Some(s) => Some(s.maps(&self.occlusion, &self.patterns)?),
            None => None,
        };
        let light_frame = lighting_frame(
            frame,
            settings,
            max_height,
            &self.lights,
            maps.as_ref(),
            &self.atlases,
        );
        let zones = stores.zones.map(|store| ZoneInput {
            store,
            list: &self.zones,
        });
        let lit = self.lighting.prepare_with_zones(
            &mut self.shaders,
            &mut self.light_pipelines,
            &light_frame,
            zones,
        )?;
        prepared.allocations += lit.allocations;
        if lit.ambient_zones {
            prepared.stages.insert(PassStages::ZONES);
        }
        if self.lighting.draws().light_pass() {
            prepared.stages.insert(PassStages::LIGHTS);
        }
        prepared.lighting = lit;

        // Step 5: composite.
        let composite = self.composite.prepare(
            &mut self.shaders,
            &composite_frame(
                frame,
                settings,
                CompositeLight::from_stage(&self.lighting, &lit),
            ),
        )?;
        prepared.allocations += composite.allocations;

        // Step 6: the GPU debug check (nothing unless the host switched it on).
        prepared.allocations += self.debug_check.prepare(
            &mut self.shaders,
            self.checks,
            &debug_check_frame(frame, settings),
        )?;
        if self.debug_check.is_active() {
            prepared.stages.insert(PassStages::DEBUG_CHECK);
        }
        self.pending = Some(Pending {
            prepared,
            shadows,
            max_height,
            shadow_scale: settings.resolutions().shadow,
        });
        Ok(prepared)
    }

    /// Records the prepared frame into the host's command buffer: occlusion, atlas uploads,
    /// zone and light pass, composite, debug check; then evaluates the frame warnings. `frame` and
    /// `settings` must be the ones given to [`prepare`](Self::prepare). The result is
    /// [`hdr`](Self::hdr).
    ///
    /// # Errors
    /// Not prepared ([`NOT_PREPARED`]), or a pass or binding failed (the next frame then
    /// rebuilds the occlusion mask).
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        frame: &PassFrame<'_>,
        settings: &LightSettings,
    ) -> Result<PassRecord, Diagnostic> {
        let Pending {
            prepared,
            shadows,
            max_height,
            shadow_scale,
        } = self
            .pending
            .take()
            .ok_or_else(|| not_prepared("record needs a successful prepare of the frame"))?;
        let mut out = PassRecord::default();
        if shadows.is_some() {
            out.mask = self.occlusion.record_with_sprite(
                cmd,
                &self.pipelines,
                self.samplers.sampler(Filter::Nearest),
                &mut self.geometry,
                &mut self.tiles,
                frame.occlusion,
            )?;
            self.occlusion.commit();
            if let Some(s) = &shadows {
                self.patterns.record(
                    cmd,
                    &self.pipelines,
                    self.samplers.sampler(Filter::Nearest),
                    &self.tiles,
                    &DomainParams::new(frame.projection, &s.domain, &s.anchor),
                )?;
            }
            self.thin_obstacles = self.geometry.thin_count();
        } else {
            self.thin_obstacles = 0;
        }
        self.atlases.record(cmd)?;
        let maps = match &shadows {
            Some(s) => Some(s.maps(&self.occlusion, &self.patterns)?),
            None => None,
        };
        let light_frame = lighting_frame(
            frame,
            settings,
            max_height,
            &self.lights,
            maps.as_ref(),
            &self.atlases,
        );
        out.lighting =
            self.lighting
                .record(cmd, &self.light_pipelines, &self.samplers, &light_frame)?;
        let light = CompositeLight::from_stage(&self.lighting, &prepared.lighting);
        self.composite.record(
            cmd,
            &self.samplers,
            &composite_frame(frame, settings, light),
        )?;

        let gpu_findings =
            self.debug_check
                .record(cmd, &self.samplers, &debug_check_frame(frame, settings))?;
        out.warnings = self.warnings.update(&FrameFindings {
            checks: self.checks,
            gpu_findings,
            flat_lights: self.lights.flat_lights,
            unreachable_lights: self.lights.unreachable_lights,
            facing_away_lights: self.lights.facing_away_lights,
            thin_obstacles: self.thin_obstacles,
            zone_height_spans: if frame.surface.is_some() {
                0
            } else {
                self.zones.height_spans
            },
        });
        self.stats.count(prepared.mask_update);
        self.last = Some(prepared);
        self.last_size = frame.size();
        self.last_shadow_scale = shadow_scale;
        Ok(out)
    }

    /// The HDR result of the last recorded frame (RGBA16F, scene size, linear light), `None`
    /// before the first one. Read it in the command buffer `record` recorded into.
    pub fn hdr(&self) -> Option<TextureRef<'_>> {
        self.composite.hdr_buffer()
    }

    /// What the last recorded frame ran, `None` before the first one.
    pub fn last_frame(&self) -> Option<&PassPrepared> {
        self.last.as_ref()
    }

    /// Counters over the plan's lifetime.
    pub fn stats(&self) -> PassStats {
        self.stats
    }

    /// Findings the host made between frames (stale layers); the next
    /// [`record`](Self::record) reports them under their debug check.
    pub fn add_pending_warnings(&mut self, warnings: WarningSet) {
        self.warnings.add_pending(warnings);
    }

    /// The frame warnings' state (which kinds were reported so far).
    pub fn warnings(&self) -> &FrameWarnings {
        &self.warnings
    }

    /// The light list of the last prepared frame.
    pub fn light_list(&self) -> &LightList {
        &self.lights
    }

    /// The light stage (its buffers, for tools and tests).
    pub fn lighting(&self) -> &LightingStage<'d> {
        &self.lighting
    }

    /// The occlusion stage (mask and field, for tools and tests).
    pub fn occlusion(&self) -> &OcclusionMask<'d> {
        &self.occlusion
    }

    /// The atlases (for tools and tests).
    pub fn atlases(&self) -> &LightAtlases<'d> {
        &self.atlases
    }

    /// Pipelines created so far: occlusion table, light programs, composite programs, debug check.
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
            + self.light_pipelines.len()
            + self.composite.pipeline_count()
            + self.debug_check.pipeline_count()
    }

    /// Checks what must hold after a recorded frame and reports each broken invariant as an
    /// [`INVARIANT`] diagnostic into `sink`; returns their number. Cold path (tools, tests,
    /// the host's debug builds):
    ///
    /// - the HDR buffer exists and has the size of the last frame's scene;
    /// - a frame that drew a light buffer still has it;
    /// - a frame with shadows has a mask and a distance field of its domain's size.
    pub fn check_invariants(&self, sink: &mut dyn Sink<Diagnostic>) -> u32 {
        let Some(last) = &self.last else {
            return 0;
        };
        let mut found = 0;
        let mut report = |message: &str, path: &str| {
            found += 1;
            sink.emit(Diagnostic::error(INVARIANT, message.to_owned()).at(path.to_owned()));
        };
        let size = |t: Option<TextureRef<'_>>| t.map(|t| (t.width(), t.height()));
        if size(self.hdr()) != Some(self.last_size) {
            report("the HDR buffer does not have the scene size", "hdr");
        }
        if last.lighting.light_buffer && self.lighting.light_buffer().is_none() {
            report(
                "the frame drew a light buffer that is missing",
                "lighting.light",
            );
        }
        if last.mask_update.is_some() {
            let (w, h) = self.last_size;
            let d = OcclusionDomain::new(w, h, self.last_shadow_scale);
            let want = Some((d.width, d.height));
            if size(self.occlusion.mask()) != want || size(self.occlusion.field()) != want {
                report(
                    "mask or distance field do not have the domain size",
                    "occlusion",
                );
            }
        }
        found
    }
}

fn update_name(update: Option<MaskUpdate>) -> &'static str {
    match update {
        None => "none",
        Some(MaskUpdate::Reused) => "reused",
        Some(MaskUpdate::Incremental) => "incremental",
        Some(MaskUpdate::Full) => "full",
    }
}

fn detail_name(mode: DetailMode) -> &'static str {
    match mode {
        DetailMode::None => "none",
        DetailMode::Exact => "exact",
        DetailMode::Direction => "direction",
    }
}

impl Inspect for PassPlan<'_> {
    /// Counters, the stages, mask update and material mode of the last frame, the pipelines
    /// created, and the size of every buffer (0 x 0 for one that does not exist).
    fn inspect(&self, v: &mut dyn Visitor) {
        let stats = self.stats;
        v.field("frames", stats.frames);
        v.field("failed", stats.failed);
        v.field("mask_reused", stats.mask_reused);
        v.field("mask_incremental", stats.mask_incremental);
        v.field("mask_full", stats.mask_full);
        v.field("pipelines", self.pipeline_count());
        v.field("shaders", self.shaders.created_count());
        v.field(
            "lights",
            self.lights.ordered.len() + self.lights.directional.len(),
        );
        v.field("zones", self.zones.zones.len());
        v.field("warnings_reported", self.warnings.reported().len());
        v.begin_object("last");
        if let Some(last) = &self.last {
            for (name, s) in [
                ("occlusion", PassStages::OCCLUSION),
                ("atlases", PassStages::ATLASES),
                ("zones", PassStages::ZONES),
                ("lights", PassStages::LIGHTS),
                ("composite", PassStages::COMPOSITE),
                ("debug_check", PassStages::DEBUG_CHECK),
            ] {
                v.field(name, last.stages.contains(s));
            }
            v.field("mask_update", update_name(last.mask_update));
            v.field("detail_mode", detail_name(last.lighting.detail_mode));
            v.field("allocations", last.allocations);
        }
        v.end();
        v.begin_object("buffers");
        for (name, t) in [
            ("hdr", self.hdr()),
            ("light", self.lighting.light_buffer()),
            ("specular", self.lighting.specular_buffer()),
            ("direction", self.lighting.direction_buffer()),
            ("pure_ambient", self.lighting.pure_ambient()),
            ("mask", self.occlusion.mask()),
            ("field", self.occlusion.field()),
            ("profile_atlas", self.atlases.profiles.texture()),
            ("mask_atlas", self.atlases.masks.texture()),
        ] {
            let (w, h) = t.map_or((0, 0), |t| (t.width(), t.height()));
            v.begin_object(name);
            v.field("width", w);
            v.field("height", h);
            v.end();
        }
        v.end();
    }
}
