// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/PipelineCache.cpp (DescFor, PipelineCache::Get for light programs, kSecondTargets
// for Specular and Direction) and src/gpu/PipelineCache.hpp (kMaterialTargetFormat).
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
// Altered: rewritten in Rust for the Knorke Engine on the generic `knorke_gpu::PipelineCache`:
// the light family is its own cache of 92 `ProgramDesc`s instead of a key range in Chiaro's
// shared cache; an unknown combination is a `Diagnostic` instead of an SDL error string.

//! Pipelines of the light program family ([`crate::light_program`]), created lazily per
//! (program, light buffer format) as in Chiaro.
//!
//! All light programs blend ONE + ONE into the light buffer; the exact and direction
//! material modes add a second target in [`MATERIAL_TARGET_FORMAT`], also ONE + ONE.
//! Ownership follows ADR 0002: this only creates pipelines on the host's device.

use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPUTextureFormat};
use knorke_gpu::{Blend, PipelineCache, PipelineRef, ProgramDesc, SecondTarget, ShaderLibrary};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::light_program::{
    LIGHT_PROGRAM_COUNT, LIGHT_PROGRAMS, LightProgram, LightProgramInfo, LightSecondTarget,
    light_program_index,
};

/// A light program combination without shaders was requested.
pub const LIGHT_PROGRAM_UNKNOWN: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.light_program.unknown");

/// Format of the specular and direction targets (Chiaro `kMaterialTargetFormat`).
pub const MATERIAL_TARGET_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

/// The pipeline description of one table entry.
pub const fn light_program_desc(info: &LightProgramInfo) -> ProgramDesc {
    let mut desc = ProgramDesc::new(info.vertex, info.fragment, Blend::Additive);
    desc.second = match info.second {
        LightSecondTarget::None => None,
        LightSecondTarget::Specular | LightSecondTarget::Direction => Some(SecondTarget {
            format: MATERIAL_TARGET_FORMAT,
            blend: Blend::Additive,
        }),
    };
    desc
}

/// The descriptions of all light programs, in table order.
pub fn light_program_descs() -> [ProgramDesc; LIGHT_PROGRAM_COUNT] {
    LIGHT_PROGRAMS.map(|info| light_program_desc(&info))
}

fn unknown(program: LightProgram) -> Diagnostic {
    Diagnostic::error(
        LIGHT_PROGRAM_UNKNOWN,
        format!(
            "no light program for {:?} with features {:#04x}",
            program.base,
            program.features.bits()
        ),
    )
}

/// Pipelines of the light family, created on first use per (program, target format).
#[derive(Debug)]
pub struct LightPipelines<'d> {
    cache: PipelineCache<'d>,
}

impl<'d> LightPipelines<'d> {
    /// Resolves the shaders of all 92 programs in `shaders`; fails if the pack lacks one or
    /// a stage is wrong. Creates no GPU object.
    pub fn new(shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        Ok(Self {
            cache: PipelineCache::new(shaders, &light_program_descs())?,
        })
    }

    /// Creates the pipeline of `program` for a light buffer in `target` format, if not there
    /// yet (Chiaro `PipelineCache::Get`).
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        program: LightProgram,
        target: SDL_GPUTextureFormat,
    ) -> Result<(), Diagnostic> {
        let index = light_program_index(program).ok_or_else(|| unknown(program))?;
        self.cache.get(shaders, index, target).map(|_| ())
    }

    /// The pipeline prepared for `program` and `target`, as a view for binding.
    pub fn pipeline(
        &self,
        program: LightProgram,
        target: SDL_GPUTextureFormat,
    ) -> Option<PipelineRef<'_>> {
        self.cache.pipeline(light_program_index(program)?, target)
    }

    /// Whether a pipeline of `program` (any format) exists.
    pub fn contains(&self, program: LightProgram) -> bool {
        light_program_index(program).is_some_and(|i| self.cache.contains(i))
    }

    /// Number of pipelines created so far.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether no pipeline has been created yet.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
