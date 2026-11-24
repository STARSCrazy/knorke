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
// Altered: rewritten in Rust for the Knorke Engine as a throw-away gate spike.

//! **SPIKE – not an API.** Gate spike of the Chiaro port (plan §5.1, §17.12 step 2,
//! ADR 0014): proves that a native Rust path on `knorke-gpu` reproduces Chiaro's image with
//! Chiaro's own compiled shaders (`.kpak`, `cargo xtask shaders`).
//!
//! Scope is deliberately narrow, one point light in one of two modes:
//!
//! - **exact material mode** (material buffer, no shadows): `light_surface` +
//!   `light_surface_exact` into light and specular buffer (golden scene `minimal_normal_map`);
//! - **flat shadow mode** (occlusion sprite buffer, no material): occlusion mask, jump-flooding
//!   distance field (seed, 6 steps, distance) and `light_point` + `light_sdf`.
//!
//! Then composite and output, with Chiaro's default settings. No culling, no obstacle store
//! (only the sprite mask), no heights, profiles, masks, zones, medium, bloom or incremental
//! mask updates. The output pass belongs to `knorke-post` (P20) after the port; it sits here
//! only because the spike needs a finished image.
//!
//! Ownership follows ADR 0002: the host lends the command buffer, the input textures and the
//! target; this module records copy and render passes into the borrowed command buffer and
//! never submits, presents or reads back.
//!
//! The real port replaces this module with P22–P25 (occlusion, distance field, light program
//! table, lighting stage), P27 (composite) and P20 (output). Remove it then.

//! It records through the safe recording API of `knorke-gpu` (P1b, ADR 0015): borrowed
//! [`CommandBuffer`], [`TextureRef`] views of the host's textures, no `unsafe`.

use std::borrow::Cow;

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    SDL_GPU_TEXTUREFORMAT_R8G8_UNORM, SDL_GPU_TEXTUREFORMAT_R16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT,
    SDL_GPUTextureFormat,
};
use knorke_gpu::sys::rect::SDL_Rect;
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, ProgramDesc,
    ProgramIndex, RenderTarget, SamplerBinding, Samplers, SecondTarget, ShaderLibrary,
    StageBindings, TextureRef, UploadBuffer,
};
use knorke_math::{Vec3, det};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::light::{Height, LightType, StoredLight};
use crate::light_math::{ground_radius, scaled_buffer_size};
use crate::material::{screen_right, view_direction};
use crate::projection::Projection;

/// Format of the light, specular and HDR buffers (Chiaro `kLightFormat`, `kHdrFormat`).
pub const HDR_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
const MASK_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8_UNORM;
const SEED_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT;
const FIELD_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16_FLOAT;

/// A spike input the slice does not cover.
pub const SPIKE_UNSUPPORTED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.spike.unsupported");

const AUTO_HEIGHT_SCALE: f32 = 0.4; // LightCulling.hpp kAutoHeightScale
const DEFAULT_WET_RIPPLE_PIXELS: f32 = 8.0; // Settings.hpp
const MAX_SHOCKWAVES: usize = 16; // CHI_MAX_SHOCKWAVES
const FIELD_BOUND: u32 = 32; // SdfShadows::kFieldBound (power of two: JfaSpan() == 32)
/// Jump distances: JfaSpan() / 2 … 1, plus one extra 1 (SdfShadows::RecordPasses).
const JFA_STEPS: [u32; 6] = [16, 8, 4, 2, 1, 1];

const LIGHT_EXACT: ProgramIndex = 0;
const LIGHT_SDF: ProgramIndex = 1;
const COMPOSITE: ProgramIndex = 2;
const OUTPUT: ProgramIndex = 3;
const OCCLUSION_MASK: ProgramIndex = 4;
const JFA_SEED: ProgramIndex = 5;
const JFA_STEP: ProgramIndex = 6;
const JFA_DISTANCE: ProgramIndex = 7;

const PROGRAMS: [ProgramDesc; 8] = [
    // Light + Surface + Exact (cmake/ChiaroLightPrograms.cmake): ONE + ONE into both targets.
    ProgramDesc {
        vertex: "light_surface.vert",
        fragment: "light_surface_exact.frag",
        blend: Blend::Additive,
        second: Some(SecondTarget {
            format: HDR_FORMAT,
            blend: Blend::Additive,
        }),
        primitive: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    },
    // LightSdf without features: point lights (CHI_POINT_LIGHTS) on the flat ground.
    ProgramDesc::new("light_point.vert", "light_sdf.frag", Blend::Additive),
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

/// `GpuLight` (light_common.hlsli), 128 bytes, as CollectLights builds it.
fn gpu_light(light: &StoredLight, frame: &SpikeFrame<'_>, d: &Domain) -> [u8; 128] {
    let desc = &light.desc;
    let normal_mode = frame.material.is_some();
    let z = match desc.height {
        Height::Auto if normal_mode => AUTO_HEIGHT_SCALE * desc.radius,
        Height::Auto => 0.0,
        Height::At(h) => h,
    };
    // Normal mode: the quad reaches the radius when the light reaches the lowest surface
    // (maxHeight 0 without a surface buffer); flat mode: the ground disc.
    let ground = if normal_mode {
        if z < desc.radius { desc.radius } else { 0.0 }
    } else {
        ground_radius(z, desc.radius)
    };
    let p = &frame.projection;
    let at = desc.position.raw();
    let scene = p.project(desc.position, 0.0).raw();
    let axis_scale = ((p.x_axis.x * p.x_axis.x + p.x_axis.y * p.x_axis.y).sqrt()
        + (p.y_axis.x * p.y_axis.x + p.y_axis.y * p.y_axis.y).sqrt())
        * 0.5;
    let mut b = Block::<128>::new();
    b.f4([at.x, at.y, z, desc.radius]);
    let c = desc.color * desc.intensity;
    b.f4([c.x, c.y, c.z, desc.falloff]);
    b.f4([light.direction.x, light.direction.y, light.direction.z, 0.0]);
    b.f4([light.cos_inner, light.cos_outer, ground, 0.0]);
    let m = d.margin as f32;
    b.f4([
        if desc.cast_shadows {
            desc.source_radius * axis_scale
        } else {
            0.0
        },
        desc.specular,
        (scene.x + m) * d.texels[0] - d.remainder[0],
        (scene.y + m) * d.texels[1] - d.remainder[1],
    ]);
    b.f4([-1.0, 0.0, 0.0, 0.0]); // no profile
    b.bytes
}

/// The spike renderer: shaders, pipelines, samplers and its own buffers.
#[derive(Debug)]
pub struct SpikeRenderer<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    light: RenderTarget<'d>,
    specular: RenderTarget<'d>,
    hdr: RenderTarget<'d>,
    mask: RenderTarget<'d>,
    seeds: [RenderTarget<'d>; 2],
    field: RenderTarget<'d>,
    lights: UploadBuffer<'d>,
}

/// Result of [`SpikeRenderer::prepare`]: which mode the frame is drawn in.
#[derive(Clone, Copy, Debug)]
pub struct Prepared {
    shadows: bool,
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
        Ok(Self {
            shaders,
            pipelines,
            samplers: Samplers::new(gpu)?,
            light: RenderTarget::new(gpu),
            specular: RenderTarget::new(gpu),
            hdr: RenderTarget::new(gpu),
            mask: RenderTarget::new(gpu),
            seeds: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            field: RenderTarget::new(gpu),
            lights: UploadBuffer::new(gpu, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, 128, "light"),
        })
    }

    /// Everything that can fail: targets, pipelines and the light buffer contents.
    pub fn prepare(
        &mut self,
        frame: &SpikeFrame<'_>,
        light: &StoredLight,
    ) -> Result<Prepared, Diagnostic> {
        let shadows = frame.occlusion.is_some() && light.desc.cast_shadows;
        if light.desc.kind != LightType::Point
            || frame.material.is_some() == frame.occlusion.is_some()
            || (frame.occlusion.is_some() && !shadows)
        {
            return Err(Diagnostic::error(
                SPIKE_UNSUPPORTED,
                "the spike draws one point light with either a material buffer or a shadow-casting occlusion buffer",
            ));
        }
        let (w, h) = (frame.width(), frame.height());
        let d = domain(frame);
        self.light.ensure(w, h, HDR_FORMAT)?;
        self.hdr.ensure(w, h, HDR_FORMAT)?;
        if frame.material.is_some() {
            self.specular.ensure(w, h, HDR_FORMAT)?;
        } else {
            self.specular.release();
        }
        if shadows {
            self.mask.ensure(d.width, d.height, MASK_FORMAT)?;
            self.field.ensure(d.width, d.height, FIELD_FORMAT)?;
            for s in &mut self.seeds {
                s.ensure(d.width, d.height, SEED_FORMAT)?;
            }
        }
        self.lights.reserve(128)?;
        let bytes = gpu_light(light, frame, &d);
        self.lights.map()?.bytes()[..128].copy_from_slice(&bytes);
        let mut get =
            |p: ProgramIndex, format| self.pipelines.get(&mut self.shaders, p, format).map(|_| ());
        if shadows {
            get(OCCLUSION_MASK, MASK_FORMAT)?;
            get(JFA_SEED, SEED_FORMAT)?;
            get(JFA_STEP, SEED_FORMAT)?;
            get(JFA_DISTANCE, FIELD_FORMAT)?;
        }
        get(if shadows { LIGHT_SDF } else { LIGHT_EXACT }, HDR_FORMAT)?;
        get(COMPOSITE, HDR_FORMAT)?;
        get(OUTPUT, frame.target.format())?;
        Ok(Prepared { shadows })
    }

    /// Records upload, occlusion and distance field (shadow mode), light pass, composite and
    /// output into the host's command buffer. `prepared` must come from
    /// [`prepare`](Self::prepare) with the same frame; otherwise recording fails with a
    /// diagnostic.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        frame: &SpikeFrame<'_>,
        prepared: Prepared,
    ) -> Result<(), Diagnostic> {
        let light_tex = self
            .light
            .texture_ref()
            .ok_or_else(|| missing("light buffer"))?;
        let hdr_tex = self
            .hdr
            .texture_ref()
            .ok_or_else(|| missing("hdr buffer"))?;
        let storage = self
            .lights
            .buffer_ref()
            .ok_or_else(|| missing("light storage buffer"))?;
        let nearest = self.samplers.sampler(Filter::Nearest);
        let linear = self.samplers.sampler(Filter::Linear);
        let pair = SamplerBinding::new;
        let d = domain(frame);
        let p = &frame.projection;
        let (o, x, y, z) = (p.origin.raw(), p.x_axis, p.y_axis, p.z_axis);
        let (w, h) = (frame.width() as f32, frame.height() as f32);
        let a = frame.ambient;
        let zero = [0.0; 4];

        cmd.copy_pass()?.upload_to_buffer(&self.lights, 128)?;

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

        // Light pass: light buffer cleared to the ambient (specular buffer to zero).
        let ambient = [a.x, a.y, a.z, 0.0];
        let buffers = [storage];
        let specular = self.specular.texture_ref();
        if let Some((mask, field)) = shadow_maps {
            // LightPassParams (light.vert family), 64 bytes.
            let mut pass_params = Block::<64>::new();
            pass_params
                .f4([o.x, o.y, x.x, x.y])
                .f4([y.x, y.y, w, h])
                .f(d.texels[0])
                .f(d.texels[1])
                .f(d.margin as f32)
                .u(0) // instance offset
                .f(d.remainder[0])
                .f(d.remainder[1])
                .u(1); // mask base = light count
            // ShadowParams, first row only (flat program): domain size, field bound, no partial.
            let mut shadow = Block::<16>::new();
            shadow.f4([d.width as f32, d.height as f32, FIELD_BOUND as f32, 0.0]);
            let samplers = [
                pair(mask, nearest),
                pair(field, linear),
                pair(frame.scene, nearest), // Profile: no profiles, the scene stands in
            ];
            let b = Bindings {
                vertex: StageBindings {
                    storage_buffers: &buffers,
                    uniforms: &[&pass_params.bytes],
                    ..Default::default()
                },
                fragment: StageBindings {
                    samplers: &samplers,
                    uniforms: &[&shadow.bytes],
                    ..Default::default()
                },
            };
            let t = [ColorTarget::new(light_tex, Load::Clear(ambient)).cycled(true)];
            self.pass(cmd, &t, LIGHT_SDF, &b, 6, None)?;
        } else {
            let material = frame.material.ok_or_else(|| {
                Diagnostic::error(SPIKE_UNSUPPORTED, "exact mode without material")
            })?;
            let specular = specular.ok_or_else(|| missing("specular buffer"))?;
            let det_xy = f64::from(x.x) * f64::from(y.y) - f64::from(x.y) * f64::from(y.x);
            let inv = [
                (f64::from(y.y) / det_xy) as f32,
                (-f64::from(y.x) / det_xy) as f32,
                (-f64::from(x.y) / det_xy) as f32,
                (f64::from(x.x) / det_xy) as f32,
            ];
            let view = view_direction(p);
            let right = screen_right(p);
            let ripple = DEFAULT_WET_RIPPLE_PIXELS / det_xy.abs().sqrt() as f32;
            // LightSurfacePassParams (light_surface.vert), 64 bytes.
            let mut pass_params = Block::<64>::new();
            pass_params
                .f4([o.x, o.y, x.x, x.y])
                .f4([y.x, y.y, w, h])
                .f4([z.x, z.y, 0.0, 0.0]) // maxHeight 0: no surface buffer
                .u(0) // instance offset
                .u(1); // mask base = light count
            // SurfaceParams (surface.hlsli CHI_SURFACE_PARAMS), 160 bytes.
            let mut surface = Block::<160>::new();
            surface
                .f4(inv)
                .f4([o.x, o.y, z.x, z.y])
                .f4([w, h, 1.0, 0.0]) // height step 1, no surface buffer
                .f4([0.0; 4]) // mask map: no shadows
                .f4([0.0; 4])
                .f4([x.x, x.y, y.x, y.y])
                .f4([view.x, view.y, view.z, 0.0])
                .f4([right.x, right.y, right.z, 0.0])
                .f4([z.x, z.y, 1.0, 0.0]) // specular scale 1
                .f4([1.0, 0.0, 1.0 / ripple, 0.0]); // wetness 0: stretch 1, breakup 0
            let samplers = [
                pair(material, nearest),    // Surface: the material stands in
                pair(material, nearest),    // Material
                pair(frame.scene, nearest), // Profile: no profiles, the scene stands in
            ];
            let b = Bindings {
                vertex: StageBindings {
                    storage_buffers: &buffers,
                    uniforms: &[&pass_params.bytes],
                    ..Default::default()
                },
                fragment: StageBindings {
                    samplers: &samplers,
                    uniforms: &[&surface.bytes],
                    ..Default::default()
                },
            };
            let t = [
                ColorTarget::new(light_tex, Load::Clear(ambient)).cycled(true),
                ColorTarget::new(specular, Load::Clear(zero)).cycled(true),
            ];
            self.pass(cmd, &t, LIGHT_EXACT, &b, 6, None)?;
        }

        // Composite into the HDR buffer.
        let exact = frame.material.is_some();
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
