// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b, a narrow slice of
// src/rendering/LightingStage.cpp (light pass: unshadowed exact material mode, shadowed flat mode),
// src/lighting/LightCulling.cpp (MakeGpuLight, CollectLights for one point light),
// src/shadows/OcclusionDomain.cpp, src/rendering/OcclusionMask.cpp (sprite mask only),
// src/rendering/SdfShadows.cpp (full rebuild), src/rendering/CompositeStage.cpp and
// src/rendering/OutputStage.cpp (parameter blocks).
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
// Altered: rewritten in Rust for the Knorke Engine as a throw-away gate spike. Since P25 the
// light pass is the real `lighting::LightingStage`; what is left here (sprite mask, field,
// composite, output) goes with P27 and P31.

//! **SPIKE – not an API.** Gate spike of the Chiaro port (plan §5.1, §17.12 step 2,
//! ADR 0014): proves that a native Rust path on `knorke-gpu` reproduces Chiaro's image with
//! Chiaro's own compiled shaders (`.kpak`, `cargo xtask shaders`).
//!
//! Scope is deliberately narrow, the lights of a [`LightList`] in one of two modes:
//!
//! - **exact material mode** (material buffer, no shadows; golden scene `minimal_normal_map`);
//! - **flat shadow mode** (occlusion sprite buffer, no material): occlusion mask from the
//!   sprite, jump-flooding distance field (seed, 6 steps, distance).
//!
//! The light pass itself is the real [`LightingStage`] (P25). Then composite and output, with
//! Chiaro's default settings. No obstacle store (only the sprite mask), no heights, zones,
//! medium, bloom or incremental mask updates. The output pass belongs to `knorke-post` (P20)
//! after the port; it sits here only because the spike needs a finished image.
//!
//! Ownership follows ADR 0002: the host lends the command buffer, the input textures and the
//! target; this module records copy and render passes into the borrowed command buffer and
//! never submits, presents or reads back.
//!
//! The rest of the port replaces this module: the sprite mask goes into the occlusion stage
//! (P23 follow-up), composite is P27, the frame plan P31. Remove it then.

//! It records through the safe recording API of `knorke-gpu` (P1b, ADR 0015): borrowed
//! [`CommandBuffer`], [`TextureRef`] views of the host's textures, no `unsafe`.

use std::borrow::Cow;

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8_UNORM, SDL_GPU_TEXTUREFORMAT_R16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT,
    SDL_GPUTextureFormat,
};
use knorke_gpu::sys::rect::SDL_Rect;
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, ProgramDesc,
    ProgramIndex, RenderTarget, SamplerBinding, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::{Vec3, det};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::light_culling::LightList;
use crate::light_math::scaled_buffer_size;
use crate::light_pipelines::LightPipelines;
use crate::lighting::{DetailMode, LightingFrame, LightingStage, ShadowMaps};
use crate::occluder_raster::INFINITE_TOP;
use crate::projection::Projection;
use crate::settings::LightSettings;
use crate::shadow::{MaskAnchor, OcclusionDomain};

/// Format of the light, specular and HDR buffers (Chiaro `kLightFormat`, `kHdrFormat`).
pub const HDR_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
const MASK_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8_UNORM;
const SEED_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT;
const FIELD_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16_FLOAT;

/// A spike input the slice does not cover.
pub const SPIKE_UNSUPPORTED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.spike.unsupported");

const MAX_SHOCKWAVES: usize = 16; // CHI_MAX_SHOCKWAVES
const FIELD_BOUND: u32 = 32; // SdfShadows::kFieldBound (power of two: JfaSpan() == 32)
/// Jump distances: JfaSpan() / 2 … 1, plus one extra 1 (SdfShadows::RecordPasses).
const JFA_STEPS: [u32; 6] = [16, 8, 4, 2, 1, 1];

const COMPOSITE: ProgramIndex = 0;
const OUTPUT: ProgramIndex = 1;
const OCCLUSION_MASK: ProgramIndex = 2;
const JFA_SEED: ProgramIndex = 3;
const JFA_STEP: ProgramIndex = 4;
const JFA_DISTANCE: ProgramIndex = 5;

const PROGRAMS: [ProgramDesc; 6] = [
    ProgramDesc::new("fullscreen.vert", "composite.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "output.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "occlusion_mask.frag", Blend::Max),
    ProgramDesc::new("fullscreen.vert", "jfa_seed.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "jfa_step.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "jfa_distance.frag", Blend::Opaque),
];

/// One frame of the spike as the host hands it over. All textures are the host's, borrowed
/// for `'a`.
#[derive(Clone, Copy, Debug)]
pub struct SpikeFrame<'a> {
    /// Scene colour, RGBA8 UNORM holding sRGB-encoded colour; its size is the frame size.
    pub scene: TextureRef<'a>,
    /// Material buffer (codec `material_pixel`), same size and format: exact material mode.
    pub material: Option<TextureRef<'a>>,
    /// Occlusion sprite buffer (alpha >= 0.5 occludes), same size: flat shadow mode.
    pub occlusion: Option<TextureRef<'a>>,
    /// Output target in an UNORM format (the shader encodes sRGB); must have the scene size
    /// (no letterbox in the spike).
    pub target: TextureRef<'a>,
    /// World to scene projection.
    pub projection: Projection,
    /// Ambient light, linear RGB.
    pub ambient: Vec3,
    /// Output filter: nearest (pixel art) or linear.
    pub output_filter: Filter,
}

impl SpikeFrame<'_> {
    /// Scene width in pixels.
    pub fn width(&self) -> u32 {
        self.scene.width()
    }

    /// Scene height in pixels.
    pub fn height(&self) -> u32 {
        self.scene.height()
    }
}

/// Fixed-size helper that writes a uniform block in declaration order.
struct Block<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Block<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            at: 0,
        }
    }
    fn f(&mut self, v: f32) -> &mut Self {
        self.bytes[self.at..self.at + 4].copy_from_slice(&v.to_le_bytes());
        self.at += 4;
        self
    }
    fn u(&mut self, v: u32) -> &mut Self {
        self.bytes[self.at..self.at + 4].copy_from_slice(&v.to_le_bytes());
        self.at += 4;
        self
    }
    fn f4(&mut self, v: [f32; 4]) -> &mut Self {
        for x in v {
            self.f(x);
        }
        self
    }
    fn skip(&mut self, bytes: usize) -> &mut Self {
        self.at += bytes;
        self
    }
}

/// The occlusion mask domain (OcclusionDomain.cpp) at shadow resolution 1, with the anchor
/// remainder of the projection origin.
#[derive(Clone, Copy, Debug)]
struct Domain {
    margin: u32,
    width: u32,
    height: u32,
    texels: [f32; 2],
    remainder: [f32; 2],
}

fn domain(frame: &SpikeFrame<'_>) -> Domain {
    let (w, h) = (frame.width(), frame.height());
    let margin = w.max(h).div_ceil(4);
    let (full_w, full_h) = (w + 2 * margin, h + 2 * margin);
    let (width, height) = (
        scaled_buffer_size(full_w, 1.0),
        scaled_buffer_size(full_h, 1.0),
    );
    let texels = [width as f32 / full_w as f32, height as f32 / full_h as f32];
    let o = frame.projection.origin.raw();
    let rem = |o: f32, t: f32| {
        let o = f64::from(o) * f64::from(t);
        let r = (o - o.floor()) as f32;
        if r >= 1.0 { 0.0 } else { r }
    };
    Domain {
        margin,
        width,
        height,
        texels,
        remainder: [rem(o.x, texels[0]), rem(o.y, texels[1])],
    }
}

/// The spike renderer: shaders, pipelines, samplers and its own buffers.
#[derive(Debug)]
pub struct SpikeRenderer<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    light_pipelines: LightPipelines<'d>,
    samplers: Samplers<'d>,
    lighting: LightingStage<'d>,
    hdr: RenderTarget<'d>,
    mask: RenderTarget<'d>,
    seeds: [RenderTarget<'d>; 2],
    field: RenderTarget<'d>,
}

/// Result of [`SpikeRenderer::prepare`]: which mode the frame is drawn in.
#[derive(Clone, Copy, Debug)]
pub struct Prepared {
    shadows: bool,
    ambient: Vec3,
    detail: DetailMode,
}

/// The settings of the spike: Chiaro's defaults with the frame's ambient.
fn settings(frame: &SpikeFrame<'_>) -> LightSettings {
    LightSettings {
        ambient: frame.ambient,
        ..LightSettings::default()
    }
}

/// The light pass inputs of the frame; `maps` are the sprite mask and its field.
fn lighting_frame<'a>(
    frame: &'a SpikeFrame<'a>,
    settings: &'a LightSettings,
    lights: &'a LightList,
    maps: Option<&'a ShadowMaps<'a>>,
) -> LightingFrame<'a> {
    LightingFrame {
        scene: frame.scene,
        surface: None,
        material: frame.material,
        projection: &frame.projection,
        settings,
        max_height: 0.0, // no surface buffer
        lights,
        shadows: maps,
        profile_atlas: None,
        mask_atlas: None,
    }
}

/// The sprite mask and its field as the light pass reads them (infinitely tall, flat mode).
fn shadow_maps_of<'a>(
    frame: &SpikeFrame<'_>,
    mask: TextureRef<'a>,
    field: TextureRef<'a>,
) -> ShadowMaps<'a> {
    let domain = OcclusionDomain::new(frame.width(), frame.height(), 1.0);
    ShadowMaps {
        domain,
        remainder: MaskAnchor::compute(&frame.projection, &domain).remainder,
        mask,
        field,
        height: None,
        height_mode: false,
        any_partial: false,
        shadow_top: INFINITE_TOP,
        scene_max_top: 0.0,
        casters: [None, None],
    }
}

fn missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        SPIKE_UNSUPPORTED,
        format!("{what} is missing after prepare"),
    )
}

fn fragment_only<'a>(samplers: &'a [SamplerBinding<'a>], uniforms: &'a [&'a [u8]]) -> Bindings<'a> {
    Bindings {
        vertex: StageBindings::default(),
        fragment: StageBindings {
            samplers,
            uniforms,
            ..Default::default()
        },
    }
}

impl<'d> SpikeRenderer<'d> {
    /// Loads the shader pack (`knorke-chiaro.<format>.kpak`) and creates samplers.
    pub fn new(gpu: Gpu<'d>, pack: impl Into<Cow<'static, [u8]>>) -> Result<Self, Diagnostic> {
        let shaders = ShaderLibrary::new(gpu, pack)?;
        let pipelines = PipelineCache::new(&shaders, &PROGRAMS)?;
        let light_pipelines = LightPipelines::new(&shaders)?;
        Ok(Self {
            shaders,
            pipelines,
            light_pipelines,
            samplers: Samplers::new(gpu)?,
            lighting: LightingStage::new(gpu),
            hdr: RenderTarget::new(gpu),
            mask: RenderTarget::new(gpu),
            seeds: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            field: RenderTarget::new(gpu),
        })
    }

    /// Everything that can fail: targets, pipelines and the light pass (`lights` from
    /// `collect_lights` for the same frame; shadowed lights need the occlusion buffer).
    pub fn prepare(
        &mut self,
        frame: &SpikeFrame<'_>,
        lights: &LightList,
    ) -> Result<Prepared, Diagnostic> {
        let shadows = frame.occlusion.is_some()
            && !(lights.shadowed.is_empty() && lights.shadowed_area.is_empty());
        if frame.material.is_some() == frame.occlusion.is_some()
            || (frame.occlusion.is_some() && !shadows)
        {
            return Err(Diagnostic::error(
                SPIKE_UNSUPPORTED,
                "the spike draws lights with either a material buffer or a shadow-casting occlusion buffer",
            ));
        }
        let (w, h) = (frame.width(), frame.height());
        let d = domain(frame);
        self.hdr.ensure(w, h, HDR_FORMAT)?;
        if shadows {
            self.mask.ensure(d.width, d.height, MASK_FORMAT)?;
            self.field.ensure(d.width, d.height, FIELD_FORMAT)?;
            for s in &mut self.seeds {
                s.ensure(d.width, d.height, SEED_FORMAT)?;
            }
        }
        let mut get =
            |p: ProgramIndex, format| self.pipelines.get(&mut self.shaders, p, format).map(|_| ());
        if shadows {
            get(OCCLUSION_MASK, MASK_FORMAT)?;
            get(JFA_SEED, SEED_FORMAT)?;
            get(JFA_STEP, SEED_FORMAT)?;
            get(JFA_DISTANCE, FIELD_FORMAT)?;
        }
        get(COMPOSITE, HDR_FORMAT)?;
        get(OUTPUT, frame.target.format())?;
        let settings = settings(frame);
        let maps = match (shadows, self.mask.texture_ref(), self.field.texture_ref()) {
            (true, Some(mask), Some(field)) => Some(shadow_maps_of(frame, mask, field)),
            (true, _, _) => return Err(missing("occlusion mask")),
            _ => None,
        };
        let lit = self.lighting.prepare(
            &mut self.shaders,
            &mut self.light_pipelines,
            &lighting_frame(frame, &settings, lights, maps.as_ref()),
        )?;
        Ok(Prepared {
            shadows,
            ambient: lit.ambient,
            detail: lit.detail_mode,
        })
    }

    /// Records upload, occlusion and distance field (shadow mode), light pass, composite and
    /// output into the host's command buffer. `prepared` must come from
    /// [`prepare`](Self::prepare) with the same frame and lights; otherwise recording fails
    /// with a diagnostic.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        frame: &SpikeFrame<'_>,
        lights: &LightList,
        prepared: Prepared,
    ) -> Result<(), Diagnostic> {
        let hdr_tex = self
            .hdr
            .texture_ref()
            .ok_or_else(|| missing("hdr buffer"))?;
        let nearest = self.samplers.sampler(Filter::Nearest);
        let linear = self.samplers.sampler(Filter::Linear);
        let pair = SamplerBinding::new;
        let d = domain(frame);
        let (w, h) = (frame.width() as f32, frame.height() as f32);
        let a = prepared.ambient;
        let zero = [0.0; 4];

        // Shadow mode: occlusion mask from the sprite buffer, then the distance field.
        let mut shadow_maps = None;
        if let (Some(occlusion), true) = (frame.occlusion, prepared.shadows) {
            let mask = self
                .mask
                .texture_ref()
                .ok_or_else(|| missing("occlusion mask"))?;
            let field = self
                .field
                .texture_ref()
                .ok_or_else(|| missing("distance field"))?;
            let seeds = [
                self.seeds[0]
                    .texture_ref()
                    .ok_or_else(|| missing("seeds"))?,
                self.seeds[1]
                    .texture_ref()
                    .ok_or_else(|| missing("seeds"))?,
            ];
            // SpriteParams: the scene rectangle in mask texels, moved by the anchor remainder.
            let rect = [
                d.margin as f32 * d.texels[0] - d.remainder[0],
                d.margin as f32 * d.texels[1] - d.remainder[1],
                w * d.texels[0],
                h * d.texels[1],
            ];
            let mut sprite = Block::<32>::new();
            sprite
                .f4(rect)
                .f4([d.width as f32, d.height as f32, 0.0, 0.0]);
            let clamp = |v: f32, size: u32| v.clamp(0.0, size as f32) as i32;
            let (x0, y0) = (
                clamp(rect[0].floor(), d.width),
                clamp(rect[1].floor(), d.height),
            );
            let x1 = clamp((rect[0] + rect[2]).ceil(), d.width);
            let y1 = clamp((rect[1] + rect[3]).ceil(), d.height);
            let scissor = SDL_Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            };
            let samplers = [pair(occlusion, nearest)];
            let uniforms: [&[u8]; 1] = [&sprite.bytes];
            let b = fragment_only(&samplers, &uniforms);
            let t = [ColorTarget::new(mask, Load::Clear(zero)).cycled(true)];
            self.pass(cmd, &t, OCCLUSION_MASK, &b, 3, Some(scissor))?;

            // Jump flooding: seed, k = 16 … 1 plus one extra k = 1, distance (SdfShadows.cpp).
            let (dw, dh) = (d.width as f32, d.height as f32);
            let jfa = |k: f32, bound: f32| {
                let mut j = Block::<32>::new();
                j.f4([dw, dh, 1.0 / dw, 1.0 / dh]).f4([k, bound, 0.0, 0.0]);
                j
            };
            let mut run = |target: TextureRef<'_>,
                           cycle: bool,
                           program,
                           input: TextureRef<'_>,
                           params: &Block<32>| {
                let samplers = [SamplerBinding::new(input, nearest)];
                let uniforms: [&[u8]; 1] = [&params.bytes];
                let b = fragment_only(&samplers, &uniforms);
                let t = [ColorTarget::new(target, Load::DontCare).cycled(cycle)];
                self.pass(cmd, &t, program, &b, 3, None)
            };
            run(seeds[0], false, JFA_SEED, mask, &jfa(0.0, 0.0))?;
            let mut current = 0;
            for k in JFA_STEPS {
                run(
                    seeds[1 - current],
                    false,
                    JFA_STEP,
                    seeds[current],
                    &jfa(k as f32, 0.0),
                )?;
                current = 1 - current;
            }
            run(
                field,
                true,
                JFA_DISTANCE,
                seeds[current],
                &jfa(0.0, FIELD_BOUND as f32),
            )?;
            shadow_maps = Some((mask, field));
        }

        // Light pass (P25).
        let settings = settings(frame);
        let maps = shadow_maps.map(|(mask, field)| shadow_maps_of(frame, mask, field));
        self.lighting.record(
            cmd,
            &self.light_pipelines,
            &self.samplers,
            &lighting_frame(frame, &settings, lights, maps.as_ref()),
        )?;
        let light_tex = self
            .lighting
            .light_buffer()
            .ok_or_else(|| missing("light buffer"))?;
        let specular = self.lighting.specular_buffer();

        // Composite into the HDR buffer.
        let exact = prepared.detail == DetailMode::Exact;
        let mut composite = Block::<160>::new();
        composite
            .u(1) // decode sRGB: sRGB scene colour in an UNORM texture
            .u(1) // light buffer
            .u(0)
            .u(0) // no glow
            .f4([a.x, a.y, a.z, 1.0])
            .u(0) // glow colour from the glow buffer
            .f(1.0) // glow scale
            .u(u32::from(exact)) // specular buffer
            .u(0); // no direction mode
        let (material, detail) = match (frame.material, specular) {
            (Some(m), Some(s)) if exact => (pair(m, nearest), pair(s, linear)),
            _ => (pair(frame.scene, nearest), pair(frame.scene, linear)),
        };
        let samplers = [
            pair(frame.scene, nearest), // Scene
            pair(light_tex, linear),    // LightBuffer
            pair(frame.scene, nearest), // Glow: none
            material,                   // Material
            detail,                     // Detail: specular buffer
            pair(frame.scene, nearest), // Surface: none
        ];
        let uniforms: [&[u8]; 1] = [&composite.bytes];
        let b = fragment_only(&samplers, &uniforms);
        let t = [ColorTarget::new(hdr_tex, Load::DontCare).cycled(true)];
        self.pass(cmd, &t, COMPOSITE, &b, 3, None)?;

        // Output into the host's target (not cycled: it is the host's texture).
        let mut output = Block::<{ 240 + 32 * MAX_SHOCKWAVES }>::new();
        output
            .f4([0.0, 0.0, w, h]) // scene rect = target
            .f(w)
            .f(h)
            .f(1.0) // exposure 0 EV
            .u(1) // encode sRGB
            .f4([0.0, 0.0, 0.0, 1.0]) // letterbox
            .u(0) // tonemap none
            .f(0.8) // knee
            .f(0.0)
            .f(0.0)
            .f4([0.0; 4]) // no bloom
            .f(w)
            .f(h)
            .skip(8)
            .f4([1.0, 1.0, 1.0, 1.0]) // tint, saturation
            .f4([0.0, 0.5, 1.5, 1.0]) // vignette off, radius, softness, contrast
            .f4([0.0, 0.0, 0.0, 2.0 / det::max(det::min(w, h), 1.0)])
            .f4([0.0, 0.0, 1.0, 1.0]) // CRT off
            .f4([0.0; 4])
            .f4([0.0, 0.5 * det::min(w, h), 0.0, 0.0]);
        let samplers = [
            pair(hdr_tex, self.samplers.sampler(frame.output_filter)),
            pair(hdr_tex, linear), // overlay: none
            pair(hdr_tex, linear), // bloom: none
        ];
        let uniforms: [&[u8]; 1] = [&output.bytes];
        let b = fragment_only(&samplers, &uniforms);
        let t = [ColorTarget::new(frame.target, Load::DontCare)];
        self.pass(cmd, &t, OUTPUT, &b, 3, None)
    }

    /// One render pass with one draw (`vertices`, one instance), with the pipeline of
    /// `program` for the first target's format.
    fn pass(
        &self,
        cmd: &mut CommandBuffer<'_>,
        targets: &[ColorTarget<'_>],
        program: ProgramIndex,
        b: &Bindings<'_>,
        vertices: u32,
        scissor: Option<SDL_Rect>,
    ) -> Result<(), Diagnostic> {
        let format = targets
            .first()
            .ok_or_else(|| missing("color target"))?
            .texture
            .format();
        let pipeline = self
            .pipelines
            .pipeline(program, format)
            .ok_or_else(|| missing("pipeline"))?;
        let mut pass = cmd.render_pass(targets)?;
        if let Some(s) = &scissor {
            pass.set_scissor(s);
        }
        pass.bind_program(pipeline, b)?;
        pass.draw_instanced(vertices, 1)
    }
}
