// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/rendering/CompositeStage.hpp and src/rendering/CompositeStage.cpp (Prepare, Record),
// src/gpu/CompositeLayout.hpp (program and layout choice) and src/gpu/ProgramBinding.hpp
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
// the inputs are a `CompositeFrame` of borrowed texture views, the stage owns its HDR target and
// its pipelines, errors are `Diagnostic`s.

use knorke_gpu::formats::is_srgb_format;
use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPUTextureFormat};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, ProgramDesc,
    RenderTarget, SamplerBinding, SamplerRef, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::params::{
    COMPOSITE_PROGRAM_COUNT, COMPOSITE_PROGRAMS, CompositeParams, CompositeParamsInput,
    CompositeProgram, InputColorSpace, composite_layout_of, composite_program_of, medium_nearest,
};
use crate::light_program::{LightSampler, LightUniform};
use crate::lighting::{DetailMode, LightingResult, LightingStage};
use crate::projection::Projection;
use crate::settings::LightSettings;

/// Format of the HDR buffer (Chiaro `CompositeStage::kHdrFormat`).
pub const HDR_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

/// [`CompositeStage::record`] without a successful [`CompositeStage::prepare`].
pub const COMPOSITE_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.composite.not_prepared");
/// A program of the frame reads a resource the frame does not have.
pub const COMPOSITE_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.composite.input_missing");

fn input_missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        COMPOSITE_INPUT_MISSING,
        format!("the composite needs {what}, which the frame does not have"),
    )
}

/// The programs of the family as pipeline descriptions, in [`COMPOSITE_PROGRAMS`] order.
fn program_descs() -> [ProgramDesc; COMPOSITE_PROGRAM_COUNT] {
    COMPOSITE_PROGRAMS.map(|p| ProgramDesc::new(p.vertex(), p.fragment(), Blend::Opaque))
}

/// What the light pass left for the composite (Chiaro `LightingResult`, the fields it reads).
#[derive(Clone, Copy, Debug)]
pub struct CompositeLight<'a> {
    /// Ambient plus the directional lights not drawn per pixel; in direction mode with ambient
    /// zones the flat part of the pure ambient.
    pub ambient: Vec3,
    /// The light buffer; `None`: no light pass, the composite multiplies by `ambient`.
    pub light_buffer: Option<TextureRef<'a>>,
    /// The composite adds the specular buffer (exact material mode, specular scale above 0).
    pub use_specular: bool,
    /// Specular buffer of the exact material mode.
    pub specular: Option<TextureRef<'a>>,
    /// Direction sums of the direction material mode; `Some` selects that mode.
    pub direction: Option<TextureRef<'a>>,
    /// The pure ambient per pixel of the ambient zone pass (direction mode with zones).
    pub pure_ambient: Option<TextureRef<'a>>,
}

impl<'a> CompositeLight<'a> {
    /// The result of a [`LightingStage`] frame: its buffers as far as `result` says the last
    /// `prepare` drew them. `pure_ambient` stays `None` (the ambient zone pass is P26).
    pub fn from_stage(stage: &'a LightingStage<'_>, result: &LightingResult) -> Self {
        let drawn = result.light_buffer;
        Self {
            ambient: result.ambient,
            light_buffer: if drawn { stage.light_buffer() } else { None },
            use_specular: result.use_specular,
            specular: if drawn && result.detail_mode == DetailMode::Exact {
                stage.specular_buffer()
            } else {
                None
            },
            direction: if drawn && result.detail_mode == DetailMode::Direction {
                stage.direction_buffer()
            } else {
                None
            },
            pure_ambient: None,
        }
    }
}

/// The medium stage's result (Chiaro `MediumResult` and `CHI_Frame::medium`): present only with
/// haze or a medium layer; the composite then takes a medium program.
#[derive(Clone, Copy, Debug)]
pub struct CompositeMedium<'a> {
    /// RGBA16F scatter buffer: rgb scattered light, a transmittance, pre-filtered.
    pub scatter: TextureRef<'a>,
    /// The guide of each scatter texel.
    pub guides: TextureRef<'a>,
    /// The medium layer of the frame; the scene stands in without one.
    pub layer: Option<TextureRef<'a>>,
    /// Bytes of the medium stage's uniform block (`MediumParams`).
    pub params: &'a [u8],
}

/// One frame of the composite as the host hands it over; every texture is borrowed.
#[derive(Clone, Copy, Debug)]
pub struct CompositeFrame<'a> {
    /// The scene colour; its size is the size of the HDR buffer.
    pub scene: TextureRef<'a>,
    /// Glow buffer (E26): RGB the glow colour, A its strength.
    pub glow: Option<TextureRef<'a>>,
    /// Material buffer; needed with the specular buffer or in direction mode.
    pub material: Option<TextureRef<'a>>,
    /// Surface buffer; direction mode takes the base normal from it, the medium programs the
    /// upsample guide.
    pub surface: Option<TextureRef<'a>>,
    /// What the light pass left.
    pub light: CompositeLight<'a>,
    /// The medium stage's result, if there is one.
    pub medium: Option<CompositeMedium<'a>>,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Settings: glow, specular scale, surface height step, wet ground, medium filter.
    pub settings: &'a LightSettings,
    /// Encoding of the scene and glow textures.
    pub input_space: InputColorSpace,
}

impl CompositeFrame<'_> {
    fn direction(&self) -> bool {
        self.light.direction.is_some()
    }

    fn program(&self) -> CompositeProgram {
        let zones = self.direction() && self.light.pure_ambient.is_some();
        composite_program_of(self.medium.is_some(), medium_nearest(self.settings), zones)
    }

    fn params(&self) -> CompositeParams {
        CompositeParams::new(&CompositeParamsInput {
            input_space: self.input_space,
            scene_format_is_srgb: is_srgb_format(self.scene.format()),
            light_buffer: self.light.light_buffer.is_some(),
            glow: self.glow.map(|g| is_srgb_format(g.format())),
            ambient: self.light.ambient,
            use_specular: self.light.use_specular,
            direction: self.direction(),
            surface_buffer: self.surface.is_some(),
            projection: self.projection,
            scene_size: (self.scene.width(), self.scene.height()),
            settings: self.settings,
        })
    }
}

/// What [`CompositeStage::prepare`] decided for the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositeResult {
    /// The program the frame draws with.
    pub program: CompositeProgram,
    /// Targets (re)allocated by this prepare.
    pub allocations: u32,
}

/// The composite pass (Chiaro `CompositeStage`): scene colour times light, plus glow, into an
/// RGBA16F HDR buffer at scene size. With haze or a medium layer the medium programs also
/// upsample the medium stage's scatter buffer and compute `colour * T + scattered light`.
///
/// Per frame: [`prepare`](Self::prepare) (target and pipeline; may fail), then
/// [`record`](Self::record) into the host's command buffer. Ownership follows ADR 0002: the
/// stage creates its own target and pipelines on the host's device, never submits, presents or
/// reads back. It is a per-frame stage without simulation state (no snapshot).
#[derive(Debug)]
pub struct CompositeStage<'d> {
    pipelines: PipelineCache<'d>,
    hdr: RenderTarget<'d>,
    prepared: Option<CompositeProgram>,
}

impl<'d> CompositeStage<'d> {
    /// Resolves the shaders of the six programs in `shaders`; fails if the pack lacks one.
    /// Creates no pipeline and no target yet.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        Ok(Self {
            pipelines: PipelineCache::new(shaders, &program_descs())?,
            hdr: RenderTarget::new(gpu),
            prepared: None,
        })
    }

    /// Makes the HDR target of the scene's size and the pipeline of the frame's program.
    ///
    /// # Errors
    /// The target or the pipeline could not be created, or the frame lacks an input its program
    /// reads ([`COMPOSITE_INPUT_MISSING`]): the material buffer with the specular buffer or in
    /// direction mode, the specular buffer itself.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        frame: &CompositeFrame<'_>,
    ) -> Result<CompositeResult, Diagnostic> {
        self.prepared = None;
        let light = &frame.light;
        if (light.use_specular || frame.direction()) && frame.material.is_none() {
            return Err(input_missing("the material buffer"));
        }
        if light.use_specular && light.specular.is_none() {
            return Err(input_missing("the specular buffer"));
        }
        let allocations = u32::from(self.hdr.ensure(
            frame.scene.width(),
            frame.scene.height(),
            HDR_FORMAT,
        )?);
        let program = frame.program();
        self.prepare_pipeline(shaders, program)?;
        self.prepared = Some(program);
        Ok(CompositeResult {
            program,
            allocations,
        })
    }

    /// Creates the pipeline of `program` ahead of the first frame that needs it, if not there
    /// yet (hosts use it to avoid a hitch).
    ///
    /// # Errors
    /// The pipeline could not be created.
    pub fn prepare_pipeline(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        program: CompositeProgram,
    ) -> Result<(), Diagnostic> {
        self.pipelines
            .get(shaders, program.index(), HDR_FORMAT)
            .map(|_| ())
    }

    /// Number of pipelines created so far.
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// The HDR buffer of the last prepared frame.
    pub fn hdr_buffer(&self) -> Option<TextureRef<'_>> {
        self.hdr.texture_ref()
    }

    /// Records the prepared frame into `cmd`: one render pass over the whole HDR buffer. `frame`
    /// must be the one given to [`prepare`](Self::prepare); `samplers` are the host's.
    ///
    /// # Errors
    /// Not prepared, or the pass or a binding failed.
    pub fn record<'a>(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &'a Samplers<'_>,
        frame: &CompositeFrame<'a>,
    ) -> Result<(), Diagnostic> {
        let program = self.prepared.ok_or_else(|| {
            Diagnostic::error(
                COMPOSITE_NOT_PREPARED,
                "the composite was not prepared for this frame",
            )
        })?;
        if program != frame.program() {
            return Err(Diagnostic::error(
                COMPOSITE_NOT_PREPARED,
                "the frame changed between prepare and record",
            ));
        }
        let hdr = self
            .hdr
            .texture_ref()
            .ok_or_else(|| input_missing("the HDR buffer"))?;
        let pipeline = self
            .pipelines
            .pipeline(program.index(), HDR_FORMAT)
            .ok_or_else(|| input_missing("the composite pipeline"))?;

        let light = &frame.light;
        let direction = frame.direction();
        let medium = frame.medium;
        let nearest = samplers.sampler(Filter::Nearest);
        let linear = samplers.sampler(Filter::Linear);
        let scene = frame.scene;
        // Without a light pass, glow, material, specular or surface buffer the scene fills their
        // slots; the shader ignores them. The layout binds only what the chosen program has.
        let material = if light.use_specular || direction {
            frame.material
        } else {
            None
        };
        let detail = if light.use_specular {
            light.specular
        } else {
            light.direction
        };
        let surface = if direction || medium.is_some() {
            frame.surface
        } else {
            None
        };
        let layout = composite_layout_of(program);
        let mut slots = [SamplerBinding::new(scene, nearest); LightSampler::COUNT];
        let mut set = |s: LightSampler, t: Option<TextureRef<'a>>, filter: SamplerRef<'a>| {
            if let Some(i) = layout.sampler(s) {
                slots[usize::from(i)] = SamplerBinding::new(t.unwrap_or(scene), filter);
            }
        };
        set(LightSampler::Scene, Some(scene), nearest);
        set(LightSampler::LightBuffer, light.light_buffer, linear);
        set(LightSampler::Glow, frame.glow, nearest);
        set(LightSampler::Material, material, nearest);
        set(LightSampler::Detail, detail, linear);
        set(LightSampler::Surface, surface, nearest);
        set(LightSampler::ScatterTex, medium.map(|m| m.scatter), nearest);
        set(
            LightSampler::MediumTex,
            medium.and_then(|m| m.layer),
            nearest,
        );
        set(
            LightSampler::ScatterGuides,
            medium.map(|m| m.guides),
            nearest,
        );
        set(LightSampler::PureAmbient, light.pure_ambient, linear);

        let params = frame.params().to_bytes();
        let mut uniforms: [&[u8]; 2] = [&[], &[]];
        if let Some(i) = layout.uniform(LightUniform::Composite) {
            uniforms[usize::from(i)] = &params;
        }
        if let Some(i) = layout.uniform(LightUniform::MediumParams) {
            uniforms[usize::from(i)] = medium
                .ok_or_else(|| input_missing("the medium stage's result"))?
                .params;
        }
        let bindings = Bindings {
            vertex: StageBindings::default(),
            fragment: StageBindings {
                samplers: &slots[..usize::from(layout.sampler_count)],
                uniforms: &uniforms[..usize::from(layout.uniform_count)],
                ..Default::default()
            },
        };
        let targets = [ColorTarget::new(hdr, Load::DontCare).cycled(true)];
        let mut pass = cmd.render_pass(&targets)?;
        pass.bind_program(pipeline, &bindings)?;
        pass.draw_fullscreen()
    }
}
