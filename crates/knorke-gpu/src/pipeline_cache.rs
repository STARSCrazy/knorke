// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/PipelineCache.hpp and src/gpu/PipelineCache.cpp.
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
// Altered: rewritten in Rust for the Knorke Engine. The cache is generic: the program table
// (Chiaro's kPrograms / kLightPrograms with their target formats) is handed in by the
// domain crate as `ProgramDesc`s instead of being compiled in; shader names are resolved and
// stage-checked when the cache is built. The blend modes and their SDL blend states are
// kept unchanged.

//! Graphics pipelines keyed by (program, color target format), created on first use.

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CreateGPUGraphicsPipeline, SDL_GPU_BLENDFACTOR_ONE,
    SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA, SDL_GPU_BLENDFACTOR_SRC_ALPHA,
    SDL_GPU_BLENDFACTOR_ZERO, SDL_GPU_BLENDOP_ADD, SDL_GPU_BLENDOP_MAX, SDL_GPU_COLORCOMPONENT_B,
    SDL_GPU_COLORCOMPONENT_G, SDL_GPU_COLORCOMPONENT_R, SDL_GPU_CULLMODE_NONE,
    SDL_GPU_FILLMODE_FILL, SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    SDL_GPU_SAMPLECOUNT_1, SDL_GPUBlendOp, SDL_GPUColorTargetBlendState,
    SDL_GPUColorTargetDescription, SDL_GPUGraphicsPipeline, SDL_GPUGraphicsPipelineCreateInfo,
    SDL_GPUPrimitiveType, SDL_GPUTextureFormat,
};

use crate::formats::format_text;
use crate::handle::sdl_error;
use crate::kpak::{Reflection, SHADER_PACK_MISSING_PROGRAM, Stage};
use crate::{Gpu, GpuHandle, ShaderId, ShaderLibrary};
use crate::{MAX_COLOR_TARGETS, PipelineRef};

/// `SDL_CreateGPUGraphicsPipeline` failed.
pub const PIPELINE_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.pipeline.create_failed");
/// A program description names a shader of the wrong stage.
pub const PIPELINE_STAGE_MISMATCH: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.pipeline.stage_mismatch");

/// How a program blends into a color target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// No blending.
    Opaque,
    /// ONE + ONE.
    Additive,
    /// MAX: overlapping writers keep the highest value.
    Max,
    /// ONE + ONE into RGB, A not written.
    AdditiveRgb,
    /// RGB MAX, A ONE + ONE.
    MaxRgbAddA,
    /// SRC_ALPHA / ONE_MINUS_SRC_ALPHA into RGB, A not written.
    AlphaRgb,
}

fn make_blend(
    op: SDL_GPUBlendOp,
    rgb_only: bool,
    alpha_op: Option<SDL_GPUBlendOp>,
) -> SDL_GPUColorTargetBlendState {
    let mut b = SDL_GPUColorTargetBlendState {
        enable_blend: true,
        src_color_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
        dst_color_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
        color_blend_op: op,
        src_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
        dst_alpha_blendfactor: SDL_GPU_BLENDFACTOR_ONE,
        alpha_blend_op: alpha_op.unwrap_or(op),
        ..Default::default()
    };
    if rgb_only {
        b.enable_color_write_mask = true;
        b.color_write_mask =
            SDL_GPU_COLORCOMPONENT_R | SDL_GPU_COLORCOMPONENT_G | SDL_GPU_COLORCOMPONENT_B;
    }
    b
}

/// The SDL blend state of a blend mode.
pub fn blend_state(blend: Blend) -> SDL_GPUColorTargetBlendState {
    match blend {
        Blend::Opaque => SDL_GPUColorTargetBlendState::default(),
        Blend::Additive => make_blend(SDL_GPU_BLENDOP_ADD, false, None),
        Blend::Max => make_blend(SDL_GPU_BLENDOP_MAX, false, None),
        Blend::AdditiveRgb => make_blend(SDL_GPU_BLENDOP_ADD, true, None),
        Blend::MaxRgbAddA => make_blend(SDL_GPU_BLENDOP_MAX, false, Some(SDL_GPU_BLENDOP_ADD)),
        Blend::AlphaRgb => {
            // dst = C * w + dst * (1 - w) in RGB, the target's alpha untouched.
            let mut s = make_blend(SDL_GPU_BLENDOP_ADD, true, None);
            s.src_color_blendfactor = SDL_GPU_BLENDFACTOR_SRC_ALPHA;
            s.dst_color_blendfactor = SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA;
            s.src_alpha_blendfactor = SDL_GPU_BLENDFACTOR_ZERO;
            s.dst_alpha_blendfactor = SDL_GPU_BLENDFACTOR_ONE;
            s
        }
    }
}

/// A second color target of a program, in a fixed format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondTarget {
    /// Texture format of the second target.
    pub format: SDL_GPUTextureFormat,
    /// Its blend mode.
    pub blend: Blend,
}

/// One program of a domain crate: shader pair, blending and targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramDesc {
    /// Vertex shader name in the pack (`fullscreen.vert`).
    pub vertex: &'static str,
    /// Fragment shader name in the pack (`composite.frag`).
    pub fragment: &'static str,
    /// Blend mode of the first color target.
    pub blend: Blend,
    /// Optional second color target.
    pub second: Option<SecondTarget>,
    /// Color targets after the second (with one), at most [`MAX_COLOR_TARGETS`] in all.
    pub more: &'static [SecondTarget],
    /// Primitive topology.
    pub primitive: SDL_GPUPrimitiveType,
}

impl ProgramDesc {
    /// A triangle-list program with one target.
    pub const fn new(vertex: &'static str, fragment: &'static str, blend: Blend) -> Self {
        Self {
            vertex,
            fragment,
            blend,
            second: None,
            more: &[],
            primitive: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
        }
    }
}

/// Index of a program in the table a [`PipelineCache`] was built with.
pub type ProgramIndex = usize;

#[derive(Debug)]
struct Resolved {
    desc: ProgramDesc,
    vertex: ShaderId,
    fragment: ShaderId,
    vertex_reflection: Reflection,
    fragment_reflection: Reflection,
}

#[derive(Debug)]
struct Entry<'d> {
    program: ProgramIndex,
    format: SDL_GPUTextureFormat,
    pipeline: GpuHandle<'d, SDL_GPUGraphicsPipeline>,
}

/// Graphics pipelines keyed by (program, first color target format), created on first use
/// and kept for the lifetime of the cache.
#[derive(Debug)]
pub struct PipelineCache<'d> {
    gpu: Gpu<'d>,
    programs: Vec<Resolved>,
    entries: Vec<Entry<'d>>,
}

impl<'d> PipelineCache<'d> {
    /// Resolves every program's shaders in `shaders` and checks their stages. Fails with
    /// [`SHADER_PACK_MISSING_PROGRAM`] or [`PIPELINE_STAGE_MISMATCH`]; creates no GPU object.
    pub fn new(shaders: &ShaderLibrary<'d>, programs: &[ProgramDesc]) -> Result<Self, Diagnostic> {
        let resolve = |name: &str, stage: Stage| -> Result<(ShaderId, Reflection), Diagnostic> {
            let id = shaders.id(name)?;
            let (actual, reflection) = shaders.info(id).ok_or_else(|| {
                Diagnostic::error(SHADER_PACK_MISSING_PROGRAM, format!("no program '{name}'"))
            })?;
            if actual != stage {
                return Err(Diagnostic::error(
                    PIPELINE_STAGE_MISMATCH,
                    format!("'{name}' is a {actual:?} shader, the program needs {stage:?}"),
                ));
            }
            Ok((id, reflection))
        };
        let programs = programs
            .iter()
            .map(|&desc| {
                let (vertex, vertex_reflection) = resolve(desc.vertex, Stage::Vertex)?;
                let (fragment, fragment_reflection) = resolve(desc.fragment, Stage::Fragment)?;
                Ok(Resolved {
                    desc,
                    vertex,
                    fragment,
                    vertex_reflection,
                    fragment_reflection,
                })
            })
            .collect::<Result<_, Diagnostic>>()?;
        Ok(Self {
            gpu: shaders.gpu(),
            programs,
            entries: Vec::new(),
        })
    }

    /// The pipeline of `program` for a first color target in `target` format. Programs with
    /// a second target add it in its [`SecondTarget`] format.
    pub fn get(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        program: ProgramIndex,
        target: SDL_GPUTextureFormat,
    ) -> Result<*mut SDL_GPUGraphicsPipeline, Diagnostic> {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| e.program == program && e.format == target)
        {
            return Ok(e.pipeline.as_ptr());
        }
        let p = self.programs.get(program).ok_or_else(|| {
            Diagnostic::error(
                SHADER_PACK_MISSING_PROGRAM,
                format!("no program {program} in the pipeline table"),
            )
        })?;
        let mut colors = [SDL_GPUColorTargetDescription::default(); MAX_COLOR_TARGETS];
        colors[0] = SDL_GPUColorTargetDescription {
            format: target,
            blend_state: blend_state(p.desc.blend),
        };
        let mut targets = 1;
        for t in p.desc.second.iter().chain(p.desc.more) {
            colors[targets as usize] = SDL_GPUColorTargetDescription {
                format: t.format,
                blend_state: blend_state(t.blend),
            };
            targets += 1;
        }
        let mut info = SDL_GPUGraphicsPipelineCreateInfo {
            vertex_shader: shaders.shader(p.vertex)?,
            fragment_shader: shaders.shader(p.fragment)?,
            primitive_type: p.desc.primitive,
            ..Default::default()
        };
        info.rasterizer_state.fill_mode = SDL_GPU_FILLMODE_FILL;
        info.rasterizer_state.cull_mode = SDL_GPU_CULLMODE_NONE;
        info.rasterizer_state.front_face = SDL_GPU_FRONTFACE_COUNTER_CLOCKWISE;
        info.multisample_state.sample_count = SDL_GPU_SAMPLECOUNT_1;
        info.target_info.color_target_descriptions = colors.as_ptr();
        info.target_info.num_color_targets = targets;

        // SAFETY: the device is live for 'd; the shaders live as long as `shaders`; `info`
        // and `colors` are alive for the call and fully initialised.
        let raw = unsafe { SDL_CreateGPUGraphicsPipeline(self.gpu.device().as_ptr(), &info) };
        // SAFETY: `raw` was just created on this device and is owned by nobody else.
        let pipeline = unsafe { GpuHandle::from_raw(self.gpu, raw) }.ok_or_else(|| {
            Diagnostic::error(
                PIPELINE_CREATE_FAILED,
                format!(
                    "creating a pipeline ({} + {}) for target format {} failed: {}",
                    p.desc.vertex,
                    p.desc.fragment,
                    format_text(target),
                    sdl_error()
                ),
            )
        })?;
        let ptr = pipeline.as_ptr();
        self.entries.push(Entry {
            program,
            format: target,
            pipeline,
        });
        Ok(ptr)
    }

    /// A pipeline created earlier by [`get`](Self::get), as a view for binding; `None` if
    /// `get` was not called for this program and format yet.
    pub fn pipeline(
        &self,
        program: ProgramIndex,
        target: SDL_GPUTextureFormat,
    ) -> Option<PipelineRef<'_>> {
        let e = self
            .entries
            .iter()
            .find(|e| e.program == program && e.format == target)?;
        let p = self.programs.get(program)?;
        let mut formats = [target; MAX_COLOR_TARGETS];
        let mut targets = 1;
        for t in p.desc.second.iter().chain(p.desc.more) {
            formats[targets] = t.format;
            targets += 1;
        }
        Some(PipelineRef::new(
            e.pipeline.non_null(),
            self.gpu.device().non_null(),
            (p.vertex_reflection, p.fragment_reflection),
            formats,
            targets as u8,
        ))
    }

    /// Reflection of a program's (vertex, fragment) shaders, to check bindings against.
    pub fn reflection(&self, program: ProgramIndex) -> Option<(Reflection, Reflection)> {
        self.programs
            .get(program)
            .map(|p| (p.vertex_reflection, p.fragment_reflection))
    }

    /// Number of programs in the table.
    pub fn program_count(&self) -> usize {
        self.programs.len()
    }

    /// Number of pipelines created so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no pipeline has been created yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a pipeline of `program` (any target format) exists.
    pub fn contains(&self, program: ProgramIndex) -> bool {
        self.entries.iter().any(|e| e.program == program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_states_match_chiaro() {
        let o = blend_state(Blend::Opaque);
        assert!(!o.enable_blend && !o.enable_color_write_mask);

        let a = blend_state(Blend::Additive);
        assert!(a.enable_blend && !a.enable_color_write_mask);
        assert_eq!(a.color_blend_op, SDL_GPU_BLENDOP_ADD);
        assert_eq!(a.alpha_blend_op, SDL_GPU_BLENDOP_ADD);

        let m = blend_state(Blend::MaxRgbAddA);
        assert_eq!(m.color_blend_op, SDL_GPU_BLENDOP_MAX);
        assert_eq!(m.alpha_blend_op, SDL_GPU_BLENDOP_ADD);

        let rgb = blend_state(Blend::AdditiveRgb);
        assert!(rgb.enable_color_write_mask);
        assert_eq!(
            rgb.color_write_mask,
            SDL_GPU_COLORCOMPONENT_R | SDL_GPU_COLORCOMPONENT_G | SDL_GPU_COLORCOMPONENT_B
        );

        let z = blend_state(Blend::AlphaRgb);
        assert_eq!(z.src_color_blendfactor, SDL_GPU_BLENDFACTOR_SRC_ALPHA);
        assert_eq!(
            z.dst_color_blendfactor,
            SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA
        );
        assert_eq!(z.src_alpha_blendfactor, SDL_GPU_BLENDFACTOR_ZERO);
        assert_eq!(z.dst_alpha_blendfactor, SDL_GPU_BLENDFACTOR_ONE);
        assert!(z.enable_color_write_mask);
    }
}
