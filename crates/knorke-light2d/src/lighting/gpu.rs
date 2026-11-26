// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/rendering/LightingStage.hpp, src/rendering/LightingStage.cpp (Prepare, Record without the
// ambient zone pass, the medium map and pattern obstacles),
// src/rendering/LightAtlases.hpp (atlases as optional inputs) and src/gpu/ProgramBinding.hpp
// (ProgramResources, BindProgram).
//
// Original notice (zlib):
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered: rewritten in Rust for the Knorke Engine on the safe recording API of `knorke-gpu`:
// the inputs are a `LightingFrame` of borrowed texture views, the occlusion results a
// `ShadowMaps`, errors are `Diagnostic`s; the light atlases are optional views the host passes.

use knorke_gpu::sys::gpu::{SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPUTextureFormat};
use knorke_gpu::{
    Bindings, BufferRef, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineRef, RenderPass,
    RenderTarget, SamplerBinding, SamplerRef, Samplers, ShaderLibrary, StageBindings, TextureRef,
    UploadBuffer,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::params::{
    DetailMode, DirectionalParams, LightPass, LightPassParams, LightSurfacePassParams,
    ProgramModes, ShadowDomain, SurfaceParams, SurfaceShadowParams, ambient_clear, detail_mode_for,
    height_tolerance, program_for,
};
use super::plan::{DrawProgram, LightDraws};
use crate::gpu_light::{GpuLight, GpuMask};
use crate::light_culling::{Casters, LightList};
use crate::light_math::scaled_buffer_size;
use crate::light_pipelines::{LightPipelines, MATERIAL_TARGET_FORMAT};
use crate::light_program::{
    LightFeature, LightLayout, LightProgram, LightSampler, LightStorage, LightUniform,
    light_layout_of,
};
use crate::medium_map::MediumMapView;
use crate::pattern_occluders::PatternMaps;
use crate::projection::Projection;
use crate::ring_light::{RingLight, RingLightPass};
use crate::rows::{MaskChange, RowLights};
use crate::settings::LightSettings;
use crate::shadow::OcclusionDomain;
use crate::zone_pass::{AmbientZonePass, ZoneInput};

/// Format of the light buffer (Chiaro `kLightFormat`).
pub const LIGHT_FORMAT: SDL_GPUTextureFormat =
    knorke_gpu::sys::gpu::SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

/// [`LightingStage::record`] without a successful [`LightingStage::prepare`].
pub const LIGHTING_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.lighting.not_prepared");
/// Profiled or masked lights without the profile or mask atlas.
pub const LIGHTING_ATLAS_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.lighting.atlas_missing");
/// Shadowed lights with a caster filter (E53) without the caster maps of their class.
pub const LIGHTING_CASTERS_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.lighting.casters_missing");
/// A program of the frame reads a resource the frame does not have.
pub const LIGHTING_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.lighting.input_missing");

/// Initial light buffer capacity in lights (Chiaro `kInitialCapacity`).
const INITIAL_CAPACITY: u32 = 64;

/// The occlusion mask, distance field and heights of one caster class (E53).
#[derive(Clone, Copy, Debug)]
pub struct CasterMaps<'a> {
    /// Occlusion mask (nearest).
    pub mask: TextureRef<'a>,
    /// Distance field (linear).
    pub field: TextureRef<'a>,
    /// Heights (nearest), in height mode.
    pub height: Option<TextureRef<'a>>,
}

/// What the occlusion stage (P23) hands the light pass: its textures and the frame's domain.
#[derive(Clone, Copy, Debug)]
pub struct ShadowMaps<'a> {
    /// The domain of the mask.
    pub domain: OcclusionDomain,
    /// Mask anchor remainder of the frame (`MaskAnchor::remainder`).
    pub remainder: [f32; 2],
    /// Occlusion mask of all casters.
    pub mask: TextureRef<'a>,
    /// Its distance field.
    pub field: TextureRef<'a>,
    /// Its heights; `Some` in height mode.
    pub height: Option<TextureRef<'a>>,
    /// The mask has heights (an obstacle with a finite height, or normal mode).
    pub height_mode: bool,
    /// An active obstacle is partly transparent (E25).
    pub any_partial: bool,
    /// Above it no obstacle blocks a point/spot ray (`INFINITE_TOP` if one is infinitely tall).
    pub shadow_top: f32,
    /// Highest finite obstacle top; 0 unless height mode.
    pub scene_max_top: f32,
    /// The maps of the caster classes `[Static, Dynamic]` (E53), where built.
    pub casters: [Option<CasterMaps<'a>>; 2],
    /// Pattern obstacles in the domain (P28); `Some` makes the shadow programs read them.
    pub patterns: Option<PatternMaps<'a>>,
    /// Mask anchor of the frame in whole texels (`MaskAnchor::snapped`), for the row lights.
    pub snapped: [i32; 2],
    /// What the occlusion stage changed in the mask this frame, for the row lights.
    pub change: MaskChange<'a>,
}

impl ShadowMaps<'_> {
    pub(crate) fn maps(&self, casters: Casters) -> Result<CasterMaps<'_>, Diagnostic> {
        match casters.filter_index() {
            None => Ok(CasterMaps {
                mask: self.mask,
                field: self.field,
                height: self.height,
            }),
            Some(c) => self.casters[c].ok_or_else(|| casters_missing(casters)),
        }
    }
}

fn casters_missing(casters: Casters) -> Diagnostic {
    Diagnostic::error(
        LIGHTING_CASTERS_MISSING,
        format!(
            "shadowed lights filtered to {casters:?} casters, but no caster maps of that class"
        ),
    )
}

/// One frame of the light pass as the host hands it over; every texture is borrowed.
#[derive(Clone, Copy, Debug)]
pub struct LightingFrame<'a> {
    /// The scene colour; its size is the scene size. Stands in for absent atlases (not read).
    pub scene: TextureRef<'a>,
    /// Surface buffer, scene size.
    pub surface: Option<TextureRef<'a>>,
    /// Material buffer, scene size (exact or direction material mode).
    pub material: Option<TextureRef<'a>>,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Settings: light resolution, specular scale, surface height step, wet ground.
    pub settings: &'a LightSettings,
    /// Normal mode: highest height a lit pixel can have (as for the culling).
    pub max_height: f32,
    /// The culled lights of the frame (`collect_lights` with the same frame).
    pub lights: &'a LightList,
    /// The occlusion results; `None` without active shadows.
    pub shadows: Option<&'a ShadowMaps<'a>>,
    /// Profile atlas (P26); needed when a light has a profile.
    pub profile_atlas: Option<TextureRef<'a>>,
    /// Mask atlas (P26); needed when a light has a mask.
    pub mask_atlas: Option<TextureRef<'a>>,
    /// Ring lights of the frame's shockwaves (P21), at most
    /// [`MAX_RING_LIGHTS`](crate::ring_light::MAX_RING_LIGHTS); empty: no ring pass.
    pub ring_lights: &'a [RingLight],
    /// The medium world map (P29); `Some` makes every draw a `*_medium` program and every
    /// directional light a per-pixel draw (cull the lights with `LightModes::medium_map`).
    pub medium_map: Option<MediumMapView<'a>>,
    /// The medium stage runs this frame (`medium_stage::medium_runs`): with ambient zones the
    /// zone pass then keeps the pure ambient for its base pass.
    pub medium_runs: bool,
}

impl LightingFrame<'_> {
    fn normal_mode(&self) -> bool {
        self.surface.is_some() || self.material.is_some()
    }

    fn scene_size(&self) -> (u32, u32) {
        (self.scene.width(), self.scene.height())
    }

    fn shadow_domain(&self) -> ShadowDomain {
        self.shadows
            .map(|s| ShadowDomain::new(&s.domain, s.remainder))
            .unwrap_or_default()
    }
}

/// Where the pure ambient of a frame with ambient zones is (Chiaro `LightingResult::pureAmbient`):
/// the ambient per pixel without the lights, read by the composite in direction mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PureAmbient {
    /// Not needed (the composite multiplies by the light buffer) or no zones.
    #[default]
    None,
    /// The light buffer is the pure ambient: zones without a light pass.
    LightBuffer,
    /// A separate target, written by the zone pass next to the light buffer
    /// ([`LightingStage::pure_ambient`]).
    Separate,
}

/// What [`LightingStage::prepare`] decided for the frame (Chiaro `LightingResult`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightingResult {
    /// Ambient plus the directional lights not drawn per pixel.
    pub ambient: Vec3,
    /// The flat directional sum in `ambient`.
    pub zone_directional: Vec3,
    /// Material mode of the frame.
    pub detail_mode: DetailMode,
    /// A light buffer was drawn; `false`: the composite multiplies by `ambient`.
    pub light_buffer: bool,
    /// Ambient zones were drawn: the light buffer starts at the ambient of each pixel, not at
    /// `ambient` (Chiaro `ambientZones`).
    pub ambient_zones: bool,
    /// Where the pure ambient of the zones is.
    pub pure_ambient: PureAmbient,
    /// The composite adds the specular buffer (exact mode, specular scale > 0).
    pub use_specular: bool,
    /// Targets (re)allocated by this prepare.
    pub allocations: u32,
}

/// What [`LightingStage::record`] recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightingRecord {
    /// Instanced lights drawn.
    pub lights_drawn: u32,
    /// Ambient zones drawn (instances of the zone pass).
    pub zones_drawn: u32,
    /// Ring lights drawn (instances of the ring pass, P21).
    pub rings_drawn: u32,
    /// Of them shadowed.
    pub shadow_lights: u32,
    /// Draw calls, the zone and ring passes' included (not the row builds).
    pub draws: u32,
    /// Of `lights_drawn` the row lights (ADR 0019).
    pub row_lights: u32,
    /// Light rows built this frame; 0 while lamps and obstacles rest.
    pub rows_built: u32,
}

/// Slot of each program of the frame.
const PROGRAMS: usize = 6;
const DIRECTIONAL: usize = 5;

const fn slot(p: DrawProgram) -> usize {
    match p {
        DrawProgram::Unshadowed => 0,
        DrawProgram::Shadowed => 1,
        DrawProgram::AreaShadowed => 2,
        DrawProgram::MaskUnshadowed => 3,
        DrawProgram::MaskShadowed => 4,
    }
}

const fn pass_of(p: DrawProgram) -> (LightPass, bool) {
    match p {
        DrawProgram::Unshadowed => (LightPass::Unshadowed, false),
        DrawProgram::Shadowed => (LightPass::Shadowed, false),
        DrawProgram::AreaShadowed => (LightPass::ShadowedArea, false),
        DrawProgram::MaskUnshadowed => (LightPass::Unshadowed, true),
        DrawProgram::MaskShadowed => (LightPass::Shadowed, true),
    }
}

const DRAW_PROGRAMS: [DrawProgram; 5] = [
    DrawProgram::Unshadowed,
    DrawProgram::Shadowed,
    DrawProgram::AreaShadowed,
    DrawProgram::MaskUnshadowed,
    DrawProgram::MaskShadowed,
];

/// The light pass (Chiaro `LightingStage`): accumulates the lights of a
/// [`LightList`] into an RGBA16F light buffer.
///
/// The buffer is cleared to the ambient sum; every visible point, spot, line and rect light
/// adds an instanced quad, every directional light drawn per pixel a fullscreen triangle (see
/// [`LightDraws`] for the order). In exact material mode the programs also write the
/// highlights into a specular buffer of the same size, in direction mode the light direction
/// sums into a direction buffer. Frames without masked lights bind, upload and allocate nothing
/// of the mask buffer.
///
/// Per frame: [`prepare`](Self::prepare) (targets, pipelines, buffer contents; may fail), then
/// [`record`](Self::record) into the host's command buffer. Ownership follows ADR 0002: the
/// stage creates its own targets and buffers on the host's device, never submits, presents
/// or reads back. It is a per-frame stage without simulation state (no snapshot).
#[derive(Debug)]
pub struct LightingStage<'d> {
    light: RenderTarget<'d>,
    specular: RenderTarget<'d>,
    direction: RenderTarget<'d>,
    lights: UploadBuffer<'d>,
    masks: UploadBuffer<'d>,
    zones: AmbientZonePass<'d>,
    rings: RingLightPass<'d>,
    rows: RowLights<'d>,
    draws: LightDraws,
    programs: [Option<LightProgram>; PROGRAMS],
    detail: DetailMode,
    ambient: Vec3,
    prepared: bool,
}

impl<'d> LightingStage<'d> {
    /// A stage without targets and buffers (the first prepare with lights creates them).
    pub fn new(gpu: Gpu<'d>) -> Self {
        let storage = SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ;
        Self {
            light: RenderTarget::new(gpu),
            specular: RenderTarget::new(gpu),
            direction: RenderTarget::new(gpu),
            lights: UploadBuffer::new(
                gpu,
                storage,
                INITIAL_CAPACITY * GpuLight::SIZE as u32,
                "light",
            ),
            masks: UploadBuffer::new(
                gpu,
                storage,
                INITIAL_CAPACITY * GpuMask::SIZE as u32,
                "mask",
            ),
            zones: AmbientZonePass::new(gpu),
            rings: RingLightPass::default(),
            rows: RowLights::new(gpu),
            draws: LightDraws::default(),
            programs: [None; PROGRAMS],
            detail: DetailMode::None,
            ambient: Vec3::ZERO,
            prepared: false,
        }
    }

    /// Decides the frame: the split of the light list, the targets (light buffer at
    /// `light_resolution`, the specular or direction buffer of the material mode), the
    /// pipelines of every program it draws, and the light and mask buffer contents. Without
    /// any light to draw the frame has no light pass and the material targets are released.
    ///
    /// # Errors
    /// A target, buffer or pipeline could not be created, a profiled or masked light has no
    /// atlas ([`LIGHTING_ATLAS_MISSING`]), or shadowed lights with a caster filter have no
    /// caster maps ([`LIGHTING_CASTERS_MISSING`]). The previous frame's targets stay then.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut LightPipelines<'d>,
        frame: &LightingFrame<'_>,
    ) -> Result<LightingResult, Diagnostic> {
        self.prepare_with_zones(shaders, pipelines, frame, None)
    }

    /// [`prepare`](Self::prepare) for a frame with ambient zones (P26, Chiaro E121): with
    /// visible zones the frame has a light buffer even without lights, the zone pass
    /// ([`AmbientZonePass`]) writes the ambient of each pixel into it before the lights, and
    /// the light pass loads it instead of clearing it. `None`, or a list without zones, is
    /// the frame of [`prepare`](Self::prepare) and releases what the zones held.
    ///
    /// # Errors
    /// As [`prepare`](Self::prepare), plus the zone pass's buffers, textures and pipeline.
    pub fn prepare_with_zones(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut LightPipelines<'d>,
        frame: &LightingFrame<'_>,
        zones: Option<ZoneInput<'_>>,
    ) -> Result<LightingResult, Diagnostic> {
        self.prepared = false;
        let list = frame.lights;
        let mut result = LightingResult {
            ambient: list.ambient,
            zone_directional: list.directional_ambient,
            ..Default::default()
        };
        self.ambient = list.ambient;
        let normal_mode = frame.normal_mode();
        let shadows = frame.shadows;
        let per_pixel = shadows.is_some() || normal_mode || frame.medium_map.is_some();
        let draws = LightDraws::new(list, shadows.is_some(), per_pixel);
        let light_pass = draws.light_pass() || !list.rows.is_empty();
        let (w, h) = frame.scene_size();
        let scale = frame.settings.resolutions().light;
        let detail = detail_mode_for(frame.material.is_some(), w, h, scale);
        // Every frame, also without row lights (see `RowLights::prepare`).
        result.allocations += self.rows.prepare(shaders, frame, detail)?;
        let zones = zones.filter(ZoneInput::is_active);
        if zones.is_none() {
            // Nothing drawn: the zone buffers, the vertex texture and the pure ambient target
            // go, so a frame without a visible zone holds what a scene without zones holds.
            self.zones.release();
        }
        // Ring lights (P21) also need the light buffer; they are added after the light pass.
        self.rings.prepare(shaders, frame.ring_lights)?;
        let rings = self.rings.is_active();
        if !light_pass && zones.is_none() && !rings {
            // No light pass, so no material target either.
            self.specular.release();
            self.direction.release();
            self.draws = draws;
            self.programs = [None; PROGRAMS];
            self.detail = DetailMode::None;
            self.prepared = true;
            return Ok(result);
        }
        if self.light.ensure(
            scaled_buffer_size(w, scale),
            scaled_buffer_size(h, scale),
            LIGHT_FORMAT,
        )? {
            result.allocations += 1;
        }
        if let Some(input) = zones {
            // The pure ambient is read by the composite in direction mode. While a light pass
            // adds the lights into the light buffer, the zone pass writes it into a second
            // target as well; without one the light buffer is the pure ambient.
            let direction_mode =
                detail_mode_for(frame.material.is_some(), w, h, scale) == DetailMode::Direction;
            // Ring lights add into the light buffer like the lights, so the medium base pass
            // needs the copy with them too (Chiaro's dual path).
            let dual =
                (light_pass && direction_mode) || ((light_pass || rings) && frame.medium_runs);
            result.allocations += self.zones.prepare(
                shaders,
                &input,
                self.light.width(),
                self.light.height(),
                dual,
            )?;
            result.ambient_zones = true;
            // Ring lights add into the light buffer like the lights: then it is not the pure
            // ambient any more.
            result.pure_ambient = match (dual, light_pass || rings) {
                (true, _) => PureAmbient::Separate,
                (false, false) => PureAmbient::LightBuffer,
                (false, true) => PureAmbient::None,
            };
        }
        if !light_pass {
            // Zones or ring lights without lights: the light buffer holds the ambient (and the
            // rings); no material target.
            self.specular.release();
            self.direction.release();
            self.draws = draws;
            self.programs = [None; PROGRAMS];
            self.detail = DetailMode::None;
            result.light_buffer = true;
            self.prepared = true;
            return Ok(result);
        }
        let count = draws.count();
        if count > 0 {
            self.lights
                .reserve(u64::from(count) * GpuLight::SIZE as u64)?;
        }
        if (count > 0 || !list.rows.is_empty())
            && list.profiled_lights > 0
            && frame.profile_atlas.is_none()
        {
            return Err(atlas_missing("profiled lights without a profile atlas"));
        }
        let masked = draws.masked();
        if masked > 0 {
            if frame.mask_atlas.is_none() {
                return Err(atlas_missing("masked lights without a mask atlas"));
            }
            self.masks
                .reserve(u64::from(masked) * GpuMask::SIZE as u64)?;
        }
        // Filtered shadows (E53) need the maps of their class, the directional ones too.
        if let Some(s) = shadows {
            for casters in [Casters::Static, Casters::Dynamic] {
                let directional = list
                    .directional_casters
                    .iter()
                    .take(draws.directional as usize)
                    .any(|&c| c == casters);
                if draws.needs_casters(casters) || directional {
                    s.maps(casters)?;
                }
            }
        }
        let exact = detail == DetailMode::Exact;
        let direction = detail == DetailMode::Direction;
        if exact || direction {
            let second = if exact {
                &mut self.specular
            } else {
                &mut self.direction
            };
            if second.ensure(
                self.light.width(),
                self.light.height(),
                MATERIAL_TARGET_FORMAT,
            )? {
                result.allocations += 1;
            }
        }
        let modes = ProgramModes {
            detail,
            normal_mode,
            shadow_height_mode: shadows.is_some_and(|s| s.height_mode),
            medium_map: frame.medium_map.is_some(),
            patterns: shadows.is_some_and(|s| s.patterns.is_some()),
        };
        let mut programs = [None; PROGRAMS];
        for p in DRAW_PROGRAMS {
            if draws.needs(p) {
                let (pass, masked) = pass_of(p);
                let program = program_for(pass, masked, &modes);
                pipelines.prepare(shaders, program, LIGHT_FORMAT)?;
                programs[slot(p)] = Some(program);
            }
        }
        if draws.directional > 0 {
            let program = program_for(LightPass::Directional, false, &modes);
            pipelines.prepare(shaders, program, LIGHT_FORMAT)?;
            programs[DIRECTIONAL] = Some(program);
        }
        if count > 0 {
            let bytes = count as usize * GpuLight::SIZE;
            let mut mapped = self.lights.map()?;
            draws.write_lights(list, &mut mapped.bytes()[..bytes]);
        }
        if masked > 0 {
            let bytes = masked as usize * GpuMask::SIZE;
            let mut mapped = self.masks.map()?;
            draws.write_masks(list, &mut mapped.bytes()[..bytes]);
        }
        // Released only once nothing above can fail: a failed prepare keeps the targets.
        if !exact {
            self.specular.release();
        }
        if !direction {
            self.direction.release();
        }
        result.detail_mode = detail;
        result.light_buffer = true;
        result.use_specular = exact && frame.settings.material.specular_scale > 0.0;
        self.draws = draws;
        self.programs = programs;
        self.detail = detail;
        self.prepared = true;
        Ok(result)
    }

    /// The light buffer of the last frame with a light pass.
    pub fn light_buffer(&self) -> Option<TextureRef<'_>> {
        self.light.texture_ref()
    }

    /// The pure ambient target of the last frame whose zone pass wrote one
    /// ([`PureAmbient::Separate`]).
    pub fn pure_ambient(&self) -> Option<TextureRef<'_>> {
        self.zones.ambient_target()
    }

    /// The ambient zone pass, for its textures.
    pub fn zone_pass(&self) -> &AmbientZonePass<'d> {
        &self.zones
    }

    /// The specular buffer of the last frame in exact material mode.
    pub fn specular_buffer(&self) -> Option<TextureRef<'_>> {
        self.specular.texture_ref()
    }

    /// The direction sums of the last frame in direction material mode.
    pub fn direction_buffer(&self) -> Option<TextureRef<'_>> {
        self.direction.texture_ref()
    }

    /// The split of the prepared frame.
    pub fn draws(&self) -> &LightDraws {
        &self.draws
    }

    /// The row lights (ADR 0019).
    pub fn rows(&self) -> &RowLights<'d> {
        &self.rows
    }

    /// Forgets the light rows: the next frame builds them all.
    pub fn invalidate(&mut self) {
        self.rows.invalidate();
    }

    /// The prepared frame was recorded: its light rows are built. Without this call every
    /// frame builds all rows.
    pub fn commit(&mut self) {
        self.rows.commit();
    }

    /// Whether the prepared frame has a light pass: Chiaro's draws or row lights.
    pub fn light_pass(&self) -> bool {
        self.draws.light_pass() || self.rows.frame().lights > 0
    }

    /// The program the prepared frame uses for `p`, if it draws any.
    pub fn program(&self, p: DrawProgram) -> Option<LightProgram> {
        self.programs[slot(p)]
    }

    /// The program of the prepared frame's directional lights, if it draws any.
    pub fn directional_program(&self) -> Option<LightProgram> {
        self.programs[DIRECTIONAL]
    }

    /// Records the prepared frame into `cmd`: one copy pass with the light (and mask)
    /// buffer, one render pass into the light buffer (and the material target). `frame` must
    /// be the one given to [`prepare`](Self::prepare); `samplers` are the host's.
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &LightPipelines<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
    ) -> Result<LightingRecord, Diagnostic> {
        if !self.prepared {
            return Err(Diagnostic::error(
                LIGHTING_NOT_PREPARED,
                "the light pass was not prepared for this frame",
            ));
        }
        let d = &self.draws;
        let zones = self.zones.is_active();
        let rings = self.rings.is_active();
        let light_pass = self.light_pass();
        if !light_pass && !zones && !rings {
            return Ok(LightingRecord::default());
        }
        let count = d.count();
        let masked = d.masked();
        let rows = self.rows.frame();
        if count > 0 || zones || rows.lights > 0 {
            let mut copy = cmd.copy_pass()?;
            self.rows.record_uploads(&mut copy)?;
            if count > 0 {
                copy.upload_to_buffer(&self.lights, count * GpuLight::SIZE as u32)?;
                if masked > 0 {
                    copy.upload_to_buffer(&self.masks, masked * GpuMask::SIZE as u32)?;
                }
            }
            self.zones.record_uploads(&mut copy)?;
        }
        let light = self
            .light
            .texture_ref()
            .ok_or_else(|| input_missing("the light buffer"))?;
        let mut record = LightingRecord::default();
        // The zone pass clears the light buffer to the ambient and folds the zones in; the
        // light pass then loads it (a discarded pixel keeps the cleared bits, so pixels
        // outside every zone stay bit-identical).
        if zones {
            self.zones
                .record_pass(cmd, samplers, frame, self.ambient, light)?;
            record.zones_drawn = self.zones.zone_count();
            record.draws += 1;
        }
        if !light_pass {
            self.record_rings(cmd, samplers, frame, light, zones, &mut record)?;
            return Ok(record);
        }
        self.rows.record_builds(cmd, samplers, frame)?;
        record.rows_built = rows.built;
        let second = match self.detail {
            DetailMode::Exact => self.specular.texture_ref(),
            DetailMode::Direction => self.direction.texture_ref(),
            DetailMode::None => None,
        };
        let first = if zones {
            ColorTarget::new(light, Load::Load)
        } else {
            ColorTarget::new(light, Load::Clear(ambient_clear(self.ambient))).cycled(true)
        };
        let mut targets = [first, first];
        let mut target_count = 1;
        if let Some(s) = second {
            targets[1] = ColorTarget::new(s, Load::Clear([0.0; 4])).cycled(true);
            target_count = 2;
        }
        let mut pass = cmd.render_pass(&targets[..target_count])?;

        let (w, h) = frame.scene_size();
        let p = frame.projection;
        let domain = frame.shadow_domain();
        let normal_mode = frame.normal_mode();
        let nearest = samplers.sampler(Filter::Nearest);
        let linear = samplers.sampler(Filter::Linear);
        let surface_texture = frame.surface.or(frame.material);
        // The flat row light reads the projection from it too (lightrow_surface.hlsli).
        let surface_params = if normal_mode || self.rows.frame().lights > 0 {
            SurfaceParams::new(p, w, h, frame.settings, &domain, frame.surface.is_some())
        } else {
            SurfaceParams::default()
        }
        .to_bytes();
        let tolerance = height_tolerance(frame.settings);
        let (any_partial, shadow_top, scene_max_top) =
            frame.shadows.map_or((false, 0.0, 0.0), |s| {
                (s.any_partial, s.shadow_top, s.scene_max_top)
            });
        let shadow =
            SurfaceShadowParams::new(&domain, any_partial, shadow_top, tolerance).to_bytes();
        let mut pass_params = LightPassParams::new(p, w, h, &domain, d.mask_base);
        let mut surface_pass = LightSurfacePassParams::new(p, w, h, frame.max_height, d.mask_base);
        let stand_in = SamplerBinding::new(frame.scene, nearest);

        let mut res = Resources::new(stand_in);
        res.set(LightSampler::Surface, surface_texture, nearest);
        res.set(
            LightSampler::Material,
            if self.detail == DetailMode::Exact {
                frame.material
            } else {
                None
            },
            nearest,
        );
        // Without profiles (or masks) the scene stands in; no light reads it then.
        res.set(
            LightSampler::Profile,
            Some(frame.profile_atlas.unwrap_or(frame.scene)),
            nearest,
        );
        res.set(
            LightSampler::Mask,
            Some(frame.mask_atlas.unwrap_or(frame.scene)),
            linear,
        );
        res.uniforms[LightUniform::Surface as usize] = Some(&surface_params);
        if let Some(m) = &frame.medium_map {
            res.set(LightSampler::MediumMap, Some(m.map), nearest);
            res.uniforms[LightUniform::MediumMap as usize] = Some(m.params);
        }
        let patterns = frame.shadows.and_then(|s| s.patterns.as_ref());
        if let Some(pm) = patterns {
            res.set(LightSampler::PatternIds, Some(pm.ids), nearest);
            res.set(LightSampler::PatternAtlas, Some(pm.atlas), nearest);
            res.uniforms[LightUniform::Pattern as usize] = Some(&pm.params);
            res.storage[LightStorage::PatternObstacles as usize] = Some(pm.obstacles);
            res.storage[LightStorage::Patterns as usize] = Some(pm.records);
            res.storage[LightStorage::PatternData as usize] = Some(pm.data);
        }
        res.storage[LightStorage::Lights as usize] = self.lights.buffer_ref();
        res.storage[LightStorage::Masks as usize] = if masked > 0 {
            self.masks.buffer_ref()
        } else {
            None
        };

        for draw in d.draws() {
            if draw.count == 0 {
                continue;
            }
            let program = self.programs[slot(draw.program)]
                .ok_or_else(|| input_missing("a light program"))?;
            let layout = light_layout_of(program);
            if let Some(s) = frame.shadows {
                let maps = s.maps(draw.casters)?;
                res.set(LightSampler::OcclusionMask, Some(maps.mask), nearest);
                res.set(LightSampler::DistanceField, Some(maps.field), linear);
                res.set(LightSampler::OcclusionHeight, maps.height, nearest);
            }
            // ShadowParams: the flat program without heights has only the domain row.
            let shadow_bytes: &[u8] = if layout.sampler(LightSampler::OcclusionHeight).is_some() {
                &shadow
            } else {
                &shadow[..SurfaceShadowParams::FLAT_SIZE]
            };
            res.uniforms[LightUniform::Shadow as usize] = Some(shadow_bytes);
            // SV_InstanceID does not include first_instance on every backend: the offset is a
            // uniform.
            let vertex = if program.has(LightFeature::Surface) {
                surface_pass.instance_offset = draw.offset;
                surface_pass.to_bytes()
            } else {
                pass_params.instance_offset = draw.offset;
                pass_params.to_bytes()
            };
            bind(
                &mut pass,
                pipelines,
                program,
                &layout,
                &res,
                Some(&vertex),
                None,
            )?;
            pass.draw_instanced(6, draw.count)?;
            record.draws += 1;
        }
        if d.directional > 0 {
            let program = self.programs[DIRECTIONAL]
                .ok_or_else(|| input_missing("the directional program"))?;
            let layout = light_layout_of(program);
            // Without shadows the surface (or material) buffer stands in for mask, field and
            // heights, without one the scene; the shader does not read them then.
            let stand_in = if normal_mode {
                res.samplers[LightSampler::Surface as usize].unwrap_or(stand_in)
            } else {
                stand_in
            };
            let mut dp =
                DirectionalParams::new(w, h, &domain, any_partial, scene_max_top, tolerance);
            for (i, light) in frame
                .lights
                .directional
                .iter()
                .take(d.directional as usize)
                .enumerate()
            {
                match frame.shadows {
                    Some(s) => {
                        let casters = frame
                            .lights
                            .directional_casters
                            .get(i)
                            .copied()
                            .unwrap_or(Casters::All);
                        let maps = s.maps(casters)?;
                        res.set(LightSampler::OcclusionMask, Some(maps.mask), nearest);
                        res.set(LightSampler::DistanceField, Some(maps.field), linear);
                        // No height texture: no directional light casts shadows (E24).
                        res.samplers[LightSampler::OcclusionHeight as usize] = Some(
                            maps.height
                                .or(s.height)
                                .map_or(stand_in, |t| SamplerBinding::new(t, nearest)),
                        );
                        dp.light = *light;
                    }
                    None => {
                        for s in [
                            LightSampler::OcclusionMask,
                            LightSampler::DistanceField,
                            LightSampler::OcclusionHeight,
                        ] {
                            res.samplers[s as usize] = Some(stand_in);
                        }
                        dp.light = *light;
                        dp.light.color[3] = 0.0;
                    }
                }
                let bytes = dp.to_bytes();
                bind(
                    &mut pass,
                    pipelines,
                    program,
                    &layout,
                    &res,
                    None,
                    Some(&bytes),
                )?;
                pass.draw_fullscreen()?;
                record.draws += 1;
            }
        }
        if rows.lights > 0 {
            self.rows
                .record_lights(&mut pass, samplers, frame, &surface_params)?;
            record.draws += 1;
            record.row_lights = rows.lights;
        }
        drop(pass);
        // Ring lights on top of the lights.
        self.record_rings(cmd, samplers, frame, light, true, &mut record)?;
        record.lights_drawn = count + rows.lights;
        record.shadow_lights = d.shadow_lights();
        Ok(record)
    }

    fn record_rings(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
        light: TextureRef<'_>,
        load: bool,
        record: &mut LightingRecord,
    ) -> Result<(), Diagnostic> {
        if self.rings.is_active() {
            self.rings
                .record_pass(cmd, samplers, frame, self.ambient, light, load)?;
            record.rings_drawn = self.rings.ring_count();
            record.draws += 1;
        }
        Ok(())
    }
}

fn atlas_missing(what: &str) -> Diagnostic {
    Diagnostic::error(LIGHTING_ATLAS_MISSING, what.to_owned())
}

fn input_missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        LIGHTING_INPUT_MISSING,
        format!("{what} is missing after prepare"),
    )
}

/// Everything the light programs can read, by name (Chiaro `ProgramResources`); each draw
/// binds what its program's layout names.
pub(crate) struct Resources<'a> {
    pub(crate) samplers: [Option<SamplerBinding<'a>>; LightSampler::COUNT],
    pub(crate) uniforms: [Option<&'a [u8]>; LightUniform::COUNT],
    pub(crate) storage: [Option<BufferRef<'a>>; LightStorage::COUNT],
    /// Fills the unused entries of the packed arrays.
    filler: SamplerBinding<'a>,
}

impl<'a> Resources<'a> {
    pub(crate) fn new(filler: SamplerBinding<'a>) -> Self {
        Self {
            samplers: [None; LightSampler::COUNT],
            uniforms: [None; LightUniform::COUNT],
            storage: [None; LightStorage::COUNT],
            filler,
        }
    }

    pub(crate) fn set(
        &mut self,
        s: LightSampler,
        texture: Option<TextureRef<'a>>,
        sampler: SamplerRef<'a>,
    ) {
        self.samplers[s as usize] = texture.map(|t| SamplerBinding::new(t, sampler));
    }
}

fn missing_resource(program: &dyn core::fmt::Debug, name: &str) -> Diagnostic {
    Diagnostic::error(
        LIGHTING_INPUT_MISSING,
        format!("{program:?} reads {name}, which the frame does not have"),
    )
}

/// The buffers in slot order, the unused tail filled with the first; `None` without any.
fn packed<T: Copy, const N: usize>(slots: &[Option<T>; N]) -> Option<[T; N]> {
    slots[0].map(|first| core::array::from_fn(|i| slots[i].unwrap_or(first)))
}

/// Binds the pipeline of `program` and every resource `layout` names from `r`, each at its
/// slot (Chiaro `BindProgram`); fails if one is absent. `vertex` is the pass parameter block
/// of the vertex shader, `directional` the `DirectionalParams` of a directional draw.
fn bind(
    pass: &mut RenderPass<'_>,
    pipelines: &LightPipelines<'_>,
    program: LightProgram,
    layout: &LightLayout,
    r: &Resources<'_>,
    vertex: Option<&[u8]>,
    directional: Option<&[u8]>,
) -> Result<(), Diagnostic> {
    let pipeline = pipelines
        .pipeline(program, LIGHT_FORMAT)
        .ok_or_else(|| input_missing("a light pipeline"))?;
    let extra = directional.map(|d| (LightUniform::Directional, d));
    bind_named(pass, pipeline, &program, layout, r, vertex, extra)
}

/// Binds `pipeline` and every resource `layout` names from `r`, each at its slot (Chiaro
/// `BindProgram`); fails naming `program` if one is absent. `vertex` is the vertex uniform
/// block, `extra` a per-draw uniform block that takes the place of its entry in `r`.
pub(crate) fn bind_named(
    pass: &mut RenderPass<'_>,
    pipeline: PipelineRef<'_>,
    program: &dyn core::fmt::Debug,
    layout: &LightLayout,
    r: &Resources<'_>,
    vertex: Option<&[u8]>,
    extra: Option<(LightUniform, &[u8])>,
) -> Result<(), Diagnostic> {
    let mut samplers = [r.filler; LightSampler::COUNT];
    for s in LightSampler::ALL {
        if let Some(i) = layout.sampler(s) {
            samplers[usize::from(i)] =
                r.samplers[s as usize].ok_or_else(|| missing_resource(program, s.hlsl_name()))?;
        }
    }
    let mut uniforms: [&[u8]; LightUniform::COUNT] = [&[]; LightUniform::COUNT];
    for u in LightUniform::ALL {
        if let Some(i) = layout.uniform(u) {
            let block = match extra {
                Some((e, bytes)) if e == u => Some(bytes),
                _ => r.uniforms[u as usize],
            };
            uniforms[usize::from(i)] =
                block.ok_or_else(|| missing_resource(program, u.hlsl_name()))?;
        }
    }
    let mut vertex_storage = [None; LightStorage::COUNT];
    let mut fragment_storage = [None; LightStorage::COUNT];
    for b in LightStorage::ALL {
        if let Some(i) = layout.storage(b) {
            let buffer =
                r.storage[b as usize].ok_or_else(|| missing_resource(program, b.hlsl_name()))?;
            let stage = if b.is_vertex() {
                &mut vertex_storage
            } else {
                &mut fragment_storage
            };
            stage[usize::from(i)] = Some(buffer);
        }
    }
    let (vs, fs) = (packed(&vertex_storage), packed(&fragment_storage));
    let vertex_uniforms: [&[u8]; 1] = [vertex.unwrap_or(&[])];
    if layout.vertex_uniform_count > 0 && vertex.is_none() {
        return Err(missing_resource(program, "its pass parameters"));
    }
    let b = Bindings {
        vertex: StageBindings {
            storage_buffers: vs
                .as_ref()
                .map_or(&[][..], |v| &v[..usize::from(layout.vertex_storage_count)]),
            uniforms: &vertex_uniforms[..usize::from(layout.vertex_uniform_count)],
            ..Default::default()
        },
        fragment: StageBindings {
            samplers: &samplers[..usize::from(layout.sampler_count)],
            storage_buffers: fs.as_ref().map_or(&[][..], |v| {
                &v[..usize::from(layout.fragment_storage_count)]
            }),
            uniforms: &uniforms[..usize::from(layout.uniform_count)],
            ..Default::default()
        },
    };
    pass.bind_program(pipeline, &b)
}
