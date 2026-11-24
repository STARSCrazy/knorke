// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/rendering/OutputStage.cpp and
// BloomStage.cpp, plus the programs Output, BloomPrefilter, BloomDown and BloomUp of
// src/gpu/PipelineCache.cpp), zlib licence:
//
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
// Altered: rewritten in Rust for the Knorke Engine on the safe recording API of knorke-gpu; the
// stages record into a command buffer the host lends them and never submit (ADR 0002).

//! The GPU passes of the post-processing: the bloom chain and the output pass (P20).
//!
//! [`PostRenderer`] takes the finished linear HDR buffer of a frame and writes the application's
//! target: exposure, bloom, tonemap, grade, vignette, CRT, shockwave distortion, scaling into the
//! letterboxed scene rectangle, the overlay and the sRGB encoding (exactly once, at the end, ADR 0002),
//! using Chiaro's compiled shaders (`output.frag`, `bloom_*.frag`, `fullscreen.vert`, pack
//! `knorke-post.<format>.kpak` or any pack that holds them).
//!
//! Ownership (ADR 0002): the host owns device, textures, command buffers and submission. The
//! renderer owns only the bloom level textures (created on first use and on size changes, kept
//! when bloom is switched off), its pipelines and samplers. Per frame:
//!
//! 1. [`PostRenderer::prepare`]: everything that can fail or allocate (settings check, target
//!    format, bloom textures, pipelines).
//! 2. [`PostRenderer::record`]: opens render passes on the borrowed [`CommandBuffer`] (2 * levels - 1
//!    bloom passes with bloom on, then one output pass) and never submits, presents or reads back.
//!
//! Textures are borrowed views ([`TextureRef`]) of the host's textures. The HDR buffer must have
//! `SAMPLER` usage, the target `COLOR_TARGET`; the target is not cycled (it is the application's).

use std::borrow::Cow;

use knorke_gpu::formats::{OutputEncoding, format_text, is_srgb_format, output_encoding_for};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB,
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT,
    SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter as GpuFilter, Gpu, Load, PipelineCache,
    ProgramDesc, ProgramIndex, RenderTarget, SamplerBinding, Samplers, ShaderLibrary,
    StageBindings, TextureRef,
};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::bloom::{BloomPlan, MAX_BLOOM_LEVELS, bloom_level_size, plan_bloom};
use crate::params::{
    BloomInput, OutputFrame, OverlayBlend, down_params, output_params, prefilter_params, up_params,
};
use crate::settings::{Filter, PostSettings};
use crate::shockwave::{Projection, ShockwaveFrame};

/// Format of the bloom level textures (Chiaro `BloomStage::kBloomFormat`).
pub const BLOOM_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

/// The target format is not one the output pass supports.
pub const POST_UNSUPPORTED_TARGET: DiagnosticCode =
    DiagnosticCode::new("knorke.post.output.unsupported_target");
/// The frame's textures do not fit the pass (e.g. an empty HDR buffer).
pub const POST_INVALID_FRAME: DiagnosticCode = DiagnosticCode::new("knorke.post.frame.invalid");
/// A resource the pass needs is missing after `prepare`, or `record` got a different frame.
pub const POST_NOT_PREPARED: DiagnosticCode = DiagnosticCode::new("knorke.post.frame.not_prepared");

const BLOOM_PREFILTER: ProgramIndex = 0;
const BLOOM_DOWN: ProgramIndex = 1;
const BLOOM_UP: ProgramIndex = 2;
const OUTPUT: ProgramIndex = 3;

const PROGRAMS: [ProgramDesc; 4] = [
    ProgramDesc::new("fullscreen.vert", "bloom_prefilter.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "bloom_down.frag", Blend::Opaque),
    // The upsample adds onto the next larger level (ONE + ONE).
    ProgramDesc::new("fullscreen.vert", "bloom_up.frag", Blend::Additive),
    ProgramDesc::new("fullscreen.vert", "output.frag", Blend::Opaque),
];

/// The shader names the renderer needs in its pack.
pub const REQUIRED_SHADERS: [&str; 5] = [
    "fullscreen.vert",
    "bloom_prefilter.frag",
    "bloom_down.frag",
    "bloom_up.frag",
    "output.frag",
];

/// One frame of the post pass as the host hands it over. All textures are the host's, borrowed
/// for `'a`.
#[derive(Clone, Copy, Debug)]
pub struct PostFrame<'a> {
    /// The linear HDR buffer (`RGBA16F` in Chiaro); its size is the scene size. Needs `SAMPLER`
    /// usage.
    pub hdr: TextureRef<'a>,
    /// Optional overlay (HUD) over the whole target, premultiplied alpha, `SAMPLER` usage. 8-bit
    /// UNORM overlays blend in sRGB-encoded space, `*_SRGB` and float overlays in linear space.
    pub overlay: Option<TextureRef<'a>>,
    /// The output target (`COLOR_TARGET` usage), one of `R8G8B8A8_UNORM(_SRGB)`,
    /// `B8G8R8A8_UNORM(_SRGB)`, `R10G10B10A2_UNORM`, `R16G16B16A16_FLOAT`.
    pub target: TextureRef<'a>,
    /// Projection and the shockwaves culled for this frame
    /// ([`collect_shockwaves`](crate::shockwave::collect_shockwaves)); only waves with a
    /// distortion in view have an effect here.
    pub shockwaves: Option<(&'a Projection, &'a ShockwaveFrame)>,
}

impl PostFrame<'_> {
    /// Scene width in pixels (the HDR buffer's).
    pub fn scene_width(&self) -> u32 {
        self.hdr.width()
    }

    /// Scene height in pixels.
    pub fn scene_height(&self) -> u32 {
        self.hdr.height()
    }
}

/// Result of [`PostRenderer::prepare`]: what the frame needs, so `record` cannot fail on
/// anything but a mismatched frame.
#[derive(Clone, Copy, Debug)]
pub struct Prepared {
    plan: Option<BloomPlan>,
    encode_srgb: bool,
    dither_target: bool,
}

impl Prepared {
    /// The bloom plan of the frame; `None` without bloom.
    pub fn bloom_plan(&self) -> Option<BloomPlan> {
        self.plan
    }
}

/// Whether samples of `format` are linear values rather than sRGB-encoded ones (Chiaro
/// `SamplesLinear`): `*_SRGB` (decoded by the hardware) and float formats.
fn samples_linear(format: SDL_GPUTextureFormat) -> bool {
    is_srgb_format(format)
        || format == SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT
        || format == SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT
}

/// The 8-bit colour formats the dither applies to (Chiaro `IsDitherFormat`): UNORM and
/// UNORM_SRGB.
fn is_dither_format(format: SDL_GPUTextureFormat) -> bool {
    [
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
        SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB,
    ]
    .contains(&format)
}

fn gpu_filter(filter: Filter) -> GpuFilter {
    match filter {
        Filter::Nearest => GpuFilter::Nearest,
        Filter::Linear => GpuFilter::Linear,
    }
}

fn missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        POST_NOT_PREPARED,
        format!("{what} is missing: record needs the frame and settings of prepare"),
    )
}

fn level_size(frame: &PostFrame<'_>, level: u32) -> (u32, u32) {
    (
        bloom_level_size(frame.scene_width(), level),
        bloom_level_size(frame.scene_height(), level),
    )
}

/// The bloom chain and the output pass: pipelines, samplers and the bloom level textures.
#[derive(Debug)]
pub struct PostRenderer<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    levels: [RenderTarget<'d>; MAX_BLOOM_LEVELS as usize],
}

impl<'d> PostRenderer<'d> {
    /// Loads the shader pack (see [`REQUIRED_SHADERS`]) and creates samplers. Fails with a
    /// `knorke.gpu.shader_pack.*` diagnostic if a shader is missing.
    pub fn new(gpu: Gpu<'d>, pack: impl Into<Cow<'static, [u8]>>) -> Result<Self, Diagnostic> {
        let shaders = ShaderLibrary::new(gpu, pack)?;
        shaders.require(&REQUIRED_SHADERS)?;
        let pipelines = PipelineCache::new(&shaders, &PROGRAMS)?;
        Ok(Self {
            shaders,
            pipelines,
            samplers: Samplers::new(gpu)?,
            levels: std::array::from_fn(|_| RenderTarget::new(gpu)),
        })
    }

    /// Number of bloom level textures (re)allocated so far, summed over all levels; stays
    /// constant while the scene size and the level count do (leak and churn checks).
    pub fn bloom_allocations(&self) -> u32 {
        self.levels.iter().map(RenderTarget::recreated).sum()
    }

    /// Everything that can fail: the settings (`knorke.post.settings.*`), the target format,
    /// the bloom level textures (only with bloom on) and the pipelines.
    pub fn prepare(
        &mut self,
        frame: &PostFrame<'_>,
        settings: &PostSettings,
    ) -> Result<Prepared, Diagnostic> {
        settings.validate().map_err(|e| e.to_diagnostic())?;
        if frame.scene_width() == 0 || frame.scene_height() == 0 {
            return Err(Diagnostic::error(
                POST_INVALID_FRAME,
                "the HDR buffer has no size",
            ));
        }
        let target = frame.target.format();
        let Some(encoding) = output_encoding_for(target) else {
            return Err(Diagnostic::error(
                POST_UNSUPPORTED_TARGET,
                format!("unsupported target format {}", format_text(target)),
            ));
        };
        let plan = if settings.bloom.enabled {
            let plan = plan_bloom(
                settings.bloom.radius,
                frame.scene_width(),
                frame.scene_height(),
            );
            for i in 0..plan.levels {
                let (w, h) = level_size(frame, i);
                self.levels[i as usize].ensure(w, h, BLOOM_FORMAT)?;
            }
            for program in [BLOOM_PREFILTER, BLOOM_DOWN, BLOOM_UP] {
                self.pipelines
                    .get(&mut self.shaders, program, BLOOM_FORMAT)?;
            }
            Some(plan)
        } else {
            None
        };
        self.pipelines.get(&mut self.shaders, OUTPUT, target)?;
        Ok(Prepared {
            plan,
            encode_srgb: encoding == OutputEncoding::Srgb,
            dither_target: is_dither_format(target),
        })
    }

    /// Records the bloom chain (with bloom on) and the output pass into the host's command
    /// buffer. `prepared` must come from [`prepare`](Self::prepare) with the same frame and
    /// settings; otherwise recording fails with a diagnostic or draws a stale image.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        frame: &PostFrame<'_>,
        settings: &PostSettings,
        prepared: Prepared,
    ) -> Result<(), Diagnostic> {
        let nearest = self.samplers.sampler(GpuFilter::Nearest);
        let linear = self.samplers.sampler(GpuFilter::Linear);
        let scene = (frame.scene_width(), frame.scene_height());

        let mut bloom = None;
        if let Some(plan) = prepared.plan {
            let level = |i: u32| -> Result<TextureRef<'_>, Diagnostic> {
                self.levels[i as usize]
                    .texture_ref()
                    .ok_or_else(|| missing("bloom level"))
            };
            let first = level(0)?;
            let size = |t: TextureRef<'_>| (t.width(), t.height());
            if size(first) != level_size(frame, 0) {
                return Err(missing("bloom level of the scene size"));
            }
            // Level 0: thresholded, Karis-averaged downsample of the exposed HDR buffer.
            let params = prefilter_params(
                scene,
                size(first),
                settings.output.exposure_ev,
                &settings.bloom,
            );
            self.pass(
                cmd,
                ColorTarget::new(first, Load::DontCare).cycled(true),
                BLOOM_PREFILTER,
                SamplerBinding::new(frame.hdr, nearest),
                &params,
            )?;
            // Down the chain, then back up adding each level onto the next larger one.
            for i in 1..plan.levels {
                let (src, dst) = (level(i - 1)?, level(i)?);
                let params = down_params(size(src), size(dst));
                self.pass(
                    cmd,
                    ColorTarget::new(dst, Load::DontCare).cycled(true),
                    BLOOM_DOWN,
                    SamplerBinding::new(src, linear),
                    &params,
                )?;
            }
            for i in (1..plan.levels).rev() {
                let (src, dst) = (level(i)?, level(i - 1)?);
                let weight = if i == plan.levels - 1 {
                    plan.coarsest_weight
                } else {
                    1.0
                };
                let params = up_params(size(src), size(dst), weight);
                self.pass(
                    cmd,
                    ColorTarget::new(dst, Load::Load),
                    BLOOM_UP,
                    SamplerBinding::new(src, linear),
                    &params,
                )?;
            }
            bloom = Some((
                first,
                BloomInput::new(settings.bloom.intensity, plan, size(first)),
            ));
        }

        let overlay_blend = frame.overlay.map(|o| {
            if samples_linear(o.format()) {
                OverlayBlend::Linear
            } else {
                OverlayBlend::Srgb
            }
        });
        let params = output_params(
            settings,
            &OutputFrame {
                scene,
                target: (frame.target.width(), frame.target.height()),
                encode_srgb: prepared.encode_srgb,
                dither_target: prepared.dither_target,
                overlay: overlay_blend,
                bloom: bloom.map(|(_, input)| input),
                shockwaves: frame.shockwaves,
            },
        );
        // Without an overlay or bloom the HDR texture fills their slots; the shader ignores them.
        let samplers = [
            SamplerBinding::new(
                frame.hdr,
                self.samplers.sampler(gpu_filter(settings.output.filter)),
            ),
            SamplerBinding::new(frame.overlay.unwrap_or(frame.hdr), linear),
            SamplerBinding::new(bloom.map_or(frame.hdr, |(t, _)| t), linear),
        ];
        // Every pixel is written, but the target is the application's: no cycling.
        self.pass_with(
            cmd,
            ColorTarget::new(frame.target, Load::DontCare),
            OUTPUT,
            &samplers,
            &params,
        )
    }

    /// One fullscreen pass with one sampled input and fragment uniforms in slot 0.
    fn pass(
        &self,
        cmd: &mut CommandBuffer<'_>,
        target: ColorTarget<'_>,
        program: ProgramIndex,
        input: SamplerBinding<'_>,
        params: &[u8],
    ) -> Result<(), Diagnostic> {
        self.pass_with(cmd, target, program, &[input], params)
    }

    fn pass_with(
        &self,
        cmd: &mut CommandBuffer<'_>,
        target: ColorTarget<'_>,
        program: ProgramIndex,
        samplers: &[SamplerBinding<'_>],
        params: &[u8],
    ) -> Result<(), Diagnostic> {
        let pipeline = self
            .pipelines
            .pipeline(program, target.texture.format())
            .ok_or_else(|| missing("pipeline"))?;
        let bindings = Bindings {
            vertex: StageBindings::default(),
            fragment: StageBindings {
                samplers,
                uniforms: &[params],
                ..Default::default()
            },
        };
        let mut pass = cmd.render_pass(&[target])?;
        pass.bind_program(pipeline, &bindings)?;
        pass.draw_fullscreen()
    }
}
