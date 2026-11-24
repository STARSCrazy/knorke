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

#![allow(unsafe_code)] // SDL recording calls (ADR 0005); every block has a `// SAFETY:` note.

use std::borrow::Cow;
use std::ptr::NonNull;

use knorke_gpu::sys::gpu::{
    SDL_BeginGPUCopyPass, SDL_BeginGPURenderPass, SDL_DrawGPUPrimitives, SDL_EndGPUCopyPass,
    SDL_EndGPURenderPass, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_LOADOP_CLEAR,
    SDL_GPU_LOADOP_DONT_CARE, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST, SDL_GPU_STOREOP_STORE,
    SDL_GPU_TEXTUREFORMAT_R8G8_UNORM, SDL_GPU_TEXTUREFORMAT_R16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT, SDL_GPUBuffer,
    SDL_GPUColorTargetInfo, SDL_GPUCommandBuffer, SDL_GPUGraphicsPipeline, SDL_GPULoadOp,
    SDL_GPUTexture, SDL_GPUTextureFormat, SDL_GPUTextureSamplerBinding, SDL_SetGPUScissor,
};
use knorke_gpu::sys::pixels::SDL_FColor;
use knorke_gpu::sys::rect::SDL_Rect;
use knorke_gpu::{
    Bindings, Blend, Filter, Gpu, PipelineCache, ProgramDesc, ProgramIndex, RenderTarget, Samplers,
    SecondTarget, ShaderLibrary, StageBindings, UploadBuffer, bind_program,
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

/// One frame of the spike as the host hands it over. All textures are the host's.
#[derive(Clone, Copy, Debug)]
pub struct SpikeFrame {
    /// Scene colour, `width x height`, RGBA8 UNORM holding sRGB-encoded colour.
    pub scene: NonNull<SDL_GPUTexture>,
    /// Material buffer (codec `material_pixel`), same size and format: exact material mode.
    pub material: Option<NonNull<SDL_GPUTexture>>,
    /// Occlusion sprite buffer (alpha >= 0.5 occludes), same size: flat shadow mode.
    pub occlusion: Option<NonNull<SDL_GPUTexture>>,
    /// Scene width in pixels.
    pub width: u32,
    /// Scene height in pixels.
    pub height: u32,
    /// Output target; must have the scene size (no letterbox in the spike).
    pub target: NonNull<SDL_GPUTexture>,
    /// Its format: an UNORM format (the shader encodes sRGB).
    pub target_format: SDL_GPUTextureFormat,
    /// World to scene projection.
    pub projection: Projection,
    /// Ambient light, linear RGB.
    pub ambient: Vec3,
    /// Output filter: nearest (pixel art) or linear.
    pub output_filter: Filter,
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

fn domain(frame: &SpikeFrame) -> Domain {
    let (w, h) = (frame.width, frame.height);
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
fn gpu_light(light: &StoredLight, frame: &SpikeFrame, d: &Domain) -> [u8; 128] {
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

/// Pipelines of one prepared frame.
#[derive(Clone, Copy, Debug)]
pub struct Prepared {
    light: NonNull<SDL_GPUGraphicsPipeline>,
    composite: NonNull<SDL_GPUGraphicsPipeline>,
    output: NonNull<SDL_GPUGraphicsPipeline>,
    /// Occlusion mask, seed, step, distance (shadow mode).
    shadow: Option<[NonNull<SDL_GPUGraphicsPipeline>; 4]>,
}

fn non_null<T>(p: Option<*mut T>, what: &str) -> Result<NonNull<T>, Diagnostic> {
    p.and_then(NonNull::new).ok_or_else(|| {
        Diagnostic::error(
            SPIKE_UNSUPPORTED,
            format!("{what} is missing after prepare"),
        )
    })
}

fn color_target(
    texture: NonNull<SDL_GPUTexture>,
    load: SDL_GPULoadOp,
    cycle: bool,
    clear: SDL_FColor,
) -> SDL_GPUColorTargetInfo {
    SDL_GPUColorTargetInfo {
        texture: texture.as_ptr(),
        clear_color: clear,
        load_op: load,
        store_op: SDL_GPU_STOREOP_STORE,
        cycle,
        ..Default::default()
    }
}

fn pair(
    t: NonNull<SDL_GPUTexture>,
    s: *mut knorke_gpu::sys::gpu::SDL_GPUSampler,
) -> SDL_GPUTextureSamplerBinding {
    SDL_GPUTextureSamplerBinding {
        texture: t.as_ptr(),
        sampler: s,
    }
}

fn fragment_only<'a>(
    samplers: &'a [SDL_GPUTextureSamplerBinding],
    uniforms: &'a [&'a [u8]],
) -> Bindings<'a> {
    Bindings {
        vertex: StageBindings::default(),
        fragment: StageBindings {
            samplers,
            storage_buffers: &[],
            uniforms,
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
        frame: &SpikeFrame,
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
        let (w, h) = (frame.width, frame.height);
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
        let mut get = |p: ProgramIndex, format| {
            self.pipelines
                .get(&mut self.shaders, p, format)
                .and_then(|x| non_null(Some(x), "pipeline"))
        };
        let shadow = if shadows {
            Some([
                get(OCCLUSION_MASK, MASK_FORMAT)?,
                get(JFA_SEED, SEED_FORMAT)?,
                get(JFA_STEP, SEED_FORMAT)?,
                get(JFA_DISTANCE, FIELD_FORMAT)?,
            ])
        } else {
            None
        };
        Ok(Prepared {
            light: get(if shadows { LIGHT_SDF } else { LIGHT_EXACT }, HDR_FORMAT)?,
            composite: get(COMPOSITE, HDR_FORMAT)?,
            output: get(OUTPUT, frame.target_format)?,
            shadow,
        })
    }

    /// Records upload, occlusion and distance field (shadow mode), light pass, composite and
    /// output into the host's command buffer.
    ///
    /// # Safety
    ///
    /// `cmd` must be an acquired, not yet submitted command buffer of this renderer's device
    /// with no pass open; the textures of `frame` must be live textures of that device with
    /// the stated sizes and sampler/colour-target usage; `prepared` must come from
    /// [`prepare`](Self::prepare) with the same frame.
    pub unsafe fn record(
        &mut self,
        cmd: NonNull<SDL_GPUCommandBuffer>,
        frame: &SpikeFrame,
        prepared: Prepared,
    ) -> Result<(), Diagnostic> {
        let light_tex = non_null(self.light.texture(), "light buffer")?;
        let hdr_tex = non_null(self.hdr.texture(), "hdr buffer")?;
        let storage = non_null(self.lights.buffer(), "light storage buffer")?;
        let nearest = self.samplers.get(Filter::Nearest);
        let linear = self.samplers.get(Filter::Linear);
        let d = domain(frame);
        let p = &frame.projection;
        let (o, x, y, z) = (p.origin.raw(), p.x_axis, p.y_axis, p.z_axis);
        let (w, h) = (frame.width as f32, frame.height as f32);
        let a = frame.ambient;
        let zero = SDL_FColor::default();

        // SAFETY: the caller guarantees an open command buffer without an active pass; the
        // light buffer was reserved and filled in `prepare`.
        unsafe {
            let copy = NonNull::new(SDL_BeginGPUCopyPass(cmd.as_ptr())).ok_or_else(|| {
                Diagnostic::error(SPIKE_UNSUPPORTED, "SDL_BeginGPUCopyPass failed")
            })?;
            self.lights.upload(copy, 128);
            SDL_EndGPUCopyPass(copy.as_ptr());
        }

        // Shadow mode: occlusion mask from the sprite buffer, then the distance field.
        let mut shadow_maps = None;
        if let (Some(occlusion), Some([mask_p, seed_p, step_p, distance_p])) =
            (frame.occlusion, prepared.shadow)
        {
            let mask = non_null(self.mask.texture(), "occlusion mask")?;
            let field = non_null(self.field.texture(), "distance field")?;
            let seeds = [
                non_null(self.seeds[0].texture(), "seeds")?,
                non_null(self.seeds[1].texture(), "seeds")?,
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
            let t = [color_target(mask, SDL_GPU_LOADOP_CLEAR, true, zero)];
            // SAFETY: as above.
            unsafe { self.pass(cmd, &t, mask_p, OCCLUSION_MASK, &b, 3, Some(scissor))? };

            // Jump flooding: seed, k = 16 … 1 plus one extra k = 1, distance (SdfShadows.cpp).
            let (dw, dh) = (d.width as f32, d.height as f32);
            let jfa = |k: f32, bound: f32| {
                let mut j = Block::<32>::new();
                j.f4([dw, dh, 1.0 / dw, 1.0 / dh]).f4([k, bound, 0.0, 0.0]);
                j
            };
            let run = |this: &Self,
                       target: NonNull<SDL_GPUTexture>,
                       cycle: bool,
                       pipeline,
                       program,
                       input: NonNull<SDL_GPUTexture>,
                       params: &Block<32>| {
                let samplers = [pair(input, nearest)];
                let uniforms: [&[u8]; 1] = [&params.bytes];
                let b = fragment_only(&samplers, &uniforms);
                let t = [color_target(target, SDL_GPU_LOADOP_DONT_CARE, cycle, zero)];
                // SAFETY: as above.
                unsafe { this.pass(cmd, &t, pipeline, program, &b, 3, None) }
            };
            run(
                self,
                seeds[0],
                false,
                seed_p,
                JFA_SEED,
                mask,
                &jfa(0.0, 0.0),
            )?;
            let mut current = 0;
            for k in JFA_STEPS {
                run(
                    self,
                    seeds[1 - current],
                    false,
                    step_p,
                    JFA_STEP,
                    seeds[current],
                    &jfa(k as f32, 0.0),
                )?;
                current = 1 - current;
            }
            run(
                self,
                field,
                true,
                distance_p,
                JFA_DISTANCE,
                seeds[current],
                &jfa(0.0, FIELD_BOUND as f32),
            )?;
            shadow_maps = Some((mask, field));
        }

        // Light pass: light buffer cleared to the ambient (specular buffer to zero).
        let ambient = SDL_FColor {
            r: a.x,
            g: a.y,
            b: a.z,
            a: 0.0,
        };
        let buffers: [*mut SDL_GPUBuffer; 1] = [storage.as_ptr()];
        let specular = self.specular.texture().and_then(NonNull::new);
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
                    samplers: &[],
                    storage_buffers: &buffers,
                    uniforms: &[&pass_params.bytes],
                },
                fragment: StageBindings {
                    samplers: &samplers,
                    storage_buffers: &[],
                    uniforms: &[&shadow.bytes],
                },
            };
            let t = [color_target(light_tex, SDL_GPU_LOADOP_CLEAR, true, ambient)];
            // SAFETY: as above.
            unsafe { self.pass(cmd, &t, prepared.light, LIGHT_SDF, &b, 6, None)? };
        } else {
            let material = frame.material.ok_or_else(|| {
                Diagnostic::error(SPIKE_UNSUPPORTED, "exact mode without material")
            })?;
            let specular = non_null(specular.map(NonNull::as_ptr), "specular buffer")?;
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
                    samplers: &[],
                    storage_buffers: &buffers,
                    uniforms: &[&pass_params.bytes],
                },
                fragment: StageBindings {
                    samplers: &samplers,
                    storage_buffers: &[],
                    uniforms: &[&surface.bytes],
                },
            };
            let t = [
                color_target(light_tex, SDL_GPU_LOADOP_CLEAR, true, ambient),
                color_target(specular, SDL_GPU_LOADOP_CLEAR, true, zero),
            ];
            // SAFETY: as above.
            unsafe { self.pass(cmd, &t, prepared.light, LIGHT_EXACT, &b, 6, None)? };
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
        let t = [color_target(hdr_tex, SDL_GPU_LOADOP_DONT_CARE, true, zero)];
        // SAFETY: as above.
        unsafe { self.pass(cmd, &t, prepared.composite, COMPOSITE, &b, 3, None)? };

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
            pair(hdr_tex, self.samplers.get(frame.output_filter)),
            pair(hdr_tex, linear), // overlay: none
            pair(hdr_tex, linear), // bloom: none
        ];
        let uniforms: [&[u8]; 1] = [&output.bytes];
        let b = fragment_only(&samplers, &uniforms);
        let t = [color_target(
            frame.target,
            SDL_GPU_LOADOP_DONT_CARE,
            false,
            zero,
        )];
        // SAFETY: as above.
        unsafe { self.pass(cmd, &t, prepared.output, OUTPUT, &b, 3, None) }
    }

    /// One render pass with one draw (`vertices`, one instance).
    ///
    /// # Safety
    ///
    /// As [`record`](Self::record); `targets` and `b` hold live objects of this device.
    #[allow(clippy::too_many_arguments)]
    unsafe fn pass(
        &self,
        cmd: NonNull<SDL_GPUCommandBuffer>,
        targets: &[SDL_GPUColorTargetInfo],
        pipeline: NonNull<SDL_GPUGraphicsPipeline>,
        program: ProgramIndex,
        b: &Bindings<'_>,
        vertices: u32,
        scissor: Option<SDL_Rect>,
    ) -> Result<(), Diagnostic> {
        let reflection = self
            .pipelines
            .reflection(program)
            .ok_or_else(|| Diagnostic::error(SPIKE_UNSUPPORTED, format!("no program {program}")))?;
        // SAFETY: forwarded from the caller; `targets` and `scissor` are alive for the calls.
        unsafe {
            let pass = SDL_BeginGPURenderPass(
                cmd.as_ptr(),
                targets.as_ptr(),
                targets.len() as u32,
                std::ptr::null(),
            );
            let pass = NonNull::new(pass).ok_or_else(|| {
                Diagnostic::error(SPIKE_UNSUPPORTED, "SDL_BeginGPURenderPass failed")
            })?;
            if let Some(s) = &scissor {
                SDL_SetGPUScissor(pass.as_ptr(), s);
            }
            let bound = bind_program(pass, cmd, pipeline, reflection, b);
            if bound.is_ok() {
                SDL_DrawGPUPrimitives(pass.as_ptr(), vertices, 1, 0, 0);
            }
            SDL_EndGPURenderPass(pass.as_ptr());
            bound
        }
    }
}
