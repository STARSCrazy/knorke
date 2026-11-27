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
// Chiaro's light programs are replaced by the row and sun programs (ADR 0019).

use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPUTextureFormat};
use knorke_gpu::{
    Bindings, BufferRef, ColorTarget, CommandBuffer, Gpu, Load, PipelineRef, RenderPass,
    RenderTarget, SamplerBinding, SamplerRef, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::params::{DetailMode, ShadowDomain, SurfaceParams, ambient_clear, detail_mode_for};
use crate::binding_layout::{LightLayout, LightSampler, LightStorage, LightUniform};
use crate::light_culling::{Casters, LightList};
use crate::light_math::scaled_buffer_size;
use crate::medium_map::MediumMapView;
use crate::pattern_occluders::PatternMaps;
use crate::projection::Projection;
use crate::ring_light::{RingLight, RingLightPass};
use crate::rows::{MaskChange, RowLights};
use crate::settings::LightSettings;
use crate::shadow::OcclusionDomain;
use crate::zone_pass::{AmbientZonePass, ZoneInput};

/// Format of the light buffer (Chiaro `kLightFormat`).
pub const LIGHT_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// Format of the specular and direction targets (Chiaro `kMaterialTargetFormat`).
pub const MATERIAL_TARGET_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

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

/// The occlusion mask and heights of one caster class (E53).
#[derive(Clone, Copy, Debug)]
pub struct CasterMaps<'a> {
    /// Occlusion mask (nearest).
    pub mask: TextureRef<'a>,
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
    /// Pattern obstacles in the domain (P28); `Some` makes the rows read them.
    pub patterns: Option<PatternMaps<'a>>,
    /// Mask anchor of the frame in whole texels (`MaskAnchor::snapped`), for the row lights.
    pub snapped: [i32; 2],
    /// What the occlusion stage redrew in the mask this frame, for the sun maps.
    pub change: MaskChange<'a>,
    /// Where the mask's contents changed this frame, for the rows: `change` without the empty
    /// parts of the strips a pan exposed or pushed out.
    pub content: MaskChange<'a>,
}

impl ShadowMaps<'_> {
    pub(crate) fn maps(&self, casters: Casters) -> Result<CasterMaps<'_>, Diagnostic> {
        match casters.filter_index() {
            None => Ok(CasterMaps {
                mask: self.mask,
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
    /// The medium world map (P29); `Some` makes every draw a `_medium` program (cull the lights
    /// with `LightModes::medium_map`, so every directional light is drawn per pixel).
    pub medium_map: Option<MediumMapView<'a>>,
    /// The medium stage runs this frame (`medium_stage::medium_runs`): with ambient zones the
    /// zone pass then keeps the pure ambient for its base pass.
    pub medium_runs: bool,
}

impl LightingFrame<'_> {
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
    /// Point, spot, line and rect lights drawn (instances of the row programs, ADR 0019).
    pub lights_drawn: u32,
    /// Ambient zones drawn (instances of the zone pass).
    pub zones_drawn: u32,
    /// Ring lights drawn (instances of the ring pass, P21).
    pub rings_drawn: u32,
    /// Draw calls, the zone and ring passes' included (not the row builds).
    pub draws: u32,
    /// Light rows built this frame; 0 while lamps and obstacles rest.
    pub rows_built: u32,
    /// Directional lights drawn by the sun programs, with a map or without one; not in
    /// `lights_drawn`.
    pub suns: u32,
    /// Sun maps built whole this frame; 0 while suns and obstacles rest.
    pub suns_built: u32,
    /// Sun maps updated in part this frame (a pan, redrawn rectangles).
    pub suns_updated: u32,
    /// Sun map texels built or updated this frame.
    pub sun_texels: u64,
}

/// The light pass (Chiaro `LightingStage`): accumulates the lights of a [`LightList`] into an
/// RGBA16F light buffer.
///
/// The buffer is cleared to the ambient sum (or loaded from the zone pass); the row programs
/// add every visible point, spot, line and rect light as an instanced quad, the sun programs
/// every directional light drawn per pixel as a fullscreen triangle, in store order
/// ([`RowLights`]). In exact material mode the programs also write the highlights into a
/// specular buffer of the same size, in direction mode the light direction sums into a
/// direction buffer.
///
/// Per frame: [`prepare`](Self::prepare) (targets, pipelines, buffer contents; may fail), then
/// [`record`](Self::record) into the host's command buffer and, once the command buffer holds
/// the frame, [`commit`](Self::commit). Ownership follows ADR 0002: the stage creates its own
/// targets and buffers on the host's device, never submits, presents or reads back. Its rows
/// and sun maps are a cache of GPU contents, not simulation state (no snapshot).
#[derive(Debug)]
pub struct LightingStage<'d> {
    light: RenderTarget<'d>,
    specular: RenderTarget<'d>,
    direction: RenderTarget<'d>,
    zones: AmbientZonePass<'d>,
    rings: RingLightPass<'d>,
    rows: RowLights<'d>,
    /// The prepared frame draws lights or suns.
    light_pass: bool,
    detail: DetailMode,
    ambient: Vec3,
    prepared: bool,
}

impl<'d> LightingStage<'d> {
    /// A stage without targets and buffers (the first prepare with lights creates them).
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            light: RenderTarget::new(gpu),
            specular: RenderTarget::new(gpu),
            direction: RenderTarget::new(gpu),
            zones: AmbientZonePass::new(gpu),
            rings: RingLightPass::default(),
            rows: RowLights::new(gpu),
            light_pass: false,
            detail: DetailMode::None,
            ambient: Vec3::ZERO,
            prepared: false,
        }
    }

    /// Decides the frame: the targets (light buffer at `light_resolution`, the specular or
    /// direction buffer of the material mode), the rows and sun maps and their pipelines.
    /// Without any light to draw the frame has no light pass and the material targets are
    /// released.
    ///
    /// # Errors
    /// A target, buffer or pipeline could not be created, a profiled or masked light has no
    /// atlas ([`LIGHTING_ATLAS_MISSING`]), or shadowed lights with a caster filter have no
    /// caster maps ([`LIGHTING_CASTERS_MISSING`]). The previous frame's targets stay then.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        frame: &LightingFrame<'_>,
    ) -> Result<LightingResult, Diagnostic> {
        self.prepare_with_zones(shaders, frame, None)
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
        let light_pass = !list.rows.is_empty() || !list.directional.is_empty();
        let (w, h) = frame.scene_size();
        let scale = frame.settings.resolutions().light;
        let detail = detail_mode_for(frame.material.is_some(), w, h, scale);
        if light_pass {
            if list.profiled_lights > 0 && frame.profile_atlas.is_none() {
                return Err(atlas_missing("profiled lights without a profile atlas"));
            }
            if list.masked_lights > 0 && frame.mask_atlas.is_none() {
                return Err(atlas_missing("masked lights without a mask atlas"));
            }
            // Filtered shadows (E53) need the maps of their class.
            if let Some(s) = frame.shadows {
                for casters in [Casters::Static, Casters::Dynamic] {
                    if list.caster_classes & casters.class_bit() != 0 {
                        s.maps(casters)?;
                    }
                }
            }
        }
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
        self.light_pass = light_pass;
        self.detail = DetailMode::None;
        if !light_pass && zones.is_none() && !rings {
            // No light pass, so no material target either.
            self.specular.release();
            self.direction.release();
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
            let direction_mode = detail == DetailMode::Direction;
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
        result.light_buffer = true;
        if !light_pass {
            // Zones or ring lights without lights: the light buffer holds the ambient (and the
            // rings); no material target.
            self.specular.release();
            self.direction.release();
            self.prepared = true;
            return Ok(result);
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
        // Released only once nothing above can fail: a failed prepare keeps the targets.
        if !exact {
            self.specular.release();
        }
        if !direction {
            self.direction.release();
        }
        result.detail_mode = detail;
        result.use_specular = exact && frame.settings.material.specular_scale > 0.0;
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

    /// The row lights and sun maps (ADR 0019).
    pub fn rows(&self) -> &RowLights<'d> {
        &self.rows
    }

    /// Forgets the light rows and sun maps: the next frame builds them all.
    pub fn invalidate(&mut self) {
        self.rows.invalidate();
    }

    /// The prepared frame was recorded: its light rows are built. Without this call every
    /// frame builds all rows.
    pub fn commit(&mut self) {
        self.rows.commit();
    }

    /// Whether the prepared frame draws lights or suns.
    pub fn light_pass(&self) -> bool {
        self.light_pass
    }

    /// Records the prepared frame into `cmd`: one copy pass (lamps, sun bands, zones), the row
    /// and sun map builds, one render pass into the light buffer (and the material target).
    /// `frame` must be the one given to [`prepare`](Self::prepare); `samplers` are the host's.
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
    ) -> Result<LightingRecord, Diagnostic> {
        if !self.prepared {
            return Err(Diagnostic::error(
                LIGHTING_NOT_PREPARED,
                "the light pass was not prepared for this frame",
            ));
        }
        let zones = self.zones.is_active();
        let rings = self.rings.is_active();
        if !self.light_pass && !zones && !rings {
            return Ok(LightingRecord::default());
        }
        let rows = self.rows.frame();
        if zones || rows.lights > 0 || rows.suns_updated > 0 {
            let mut copy = cmd.copy_pass()?;
            self.rows.record_uploads(&mut copy)?;
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
        if !self.light_pass {
            self.record_rings(cmd, samplers, frame, light, zones, &mut record)?;
            return Ok(record);
        }
        self.rows.record_builds(cmd, samplers, frame)?;
        record.rows_built = rows.built;
        record.suns_built = rows.suns_built;
        record.suns_updated = rows.suns_updated;
        record.sun_texels = rows.sun_texels;
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
        let domain = frame.shadow_domain();
        let surface = frame.surface.is_some();
        let surface_params =
            SurfaceParams::new(frame.projection, w, h, frame.settings, &domain, surface).to_bytes();
        // In store order, so the additive sum is the same in every frame.
        for i in 0..frame.lights.directional.len() {
            self.rows
                .record_sun(&mut pass, samplers, frame, &surface_params, i)?;
            record.draws += 1;
            record.suns += 1;
        }
        if rows.lights > 0 {
            self.rows
                .record_lights(&mut pass, samplers, frame, &surface_params)?;
            record.draws += 1;
            record.lights_drawn = rows.lights;
        }
        drop(pass);
        // Ring lights on top of the lights.
        self.record_rings(cmd, samplers, frame, light, true, &mut record)?;
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

/// Everything the medium programs can read, by name (Chiaro `ProgramResources`); each draw
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
