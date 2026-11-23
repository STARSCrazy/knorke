// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/ProgramBinding.hpp (BindProgram, BindFragmentSamplers, PushFragmentUniform).
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
// Altered: rewritten in Rust for the Knorke Engine. The named slot tables of the light
// programs (LightLayout, LightSampler, …) stay with the domain crate; here a draw passes its
// resources already in slot order, and the counts are checked against the shaders'
// reflection from the pack before anything is bound.

//! Binding a program's resources in slot order, checked against the shaders' reflection.

use core::ptr::NonNull;

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_BindGPUFragmentSamplers, SDL_BindGPUFragmentStorageBuffers, SDL_BindGPUGraphicsPipeline,
    SDL_BindGPUVertexSamplers, SDL_BindGPUVertexStorageBuffers, SDL_GPUBuffer,
    SDL_GPUCommandBuffer, SDL_GPUGraphicsPipeline, SDL_GPURenderPass, SDL_GPUTextureSamplerBinding,
    SDL_PushGPUFragmentUniformData, SDL_PushGPUVertexUniformData,
};

use crate::kpak::Reflection;

/// The resources of a draw do not match what the program's shaders declare.
pub const BINDING_MISMATCH: DiagnosticCode = DiagnosticCode::new("knorke.gpu.binding.mismatch");

/// Resources of one stage, each slice in slot order starting at slot 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageBindings<'a> {
    /// Texture-sampler pairs.
    pub samplers: &'a [SDL_GPUTextureSamplerBinding],
    /// Storage buffers.
    pub storage_buffers: &'a [*mut SDL_GPUBuffer],
    /// Uniform blocks, pushed at slot = position.
    pub uniforms: &'a [&'a [u8]],
}

/// Everything a draw binds for its program.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bindings<'a> {
    /// Vertex stage.
    pub vertex: StageBindings<'a>,
    /// Fragment stage.
    pub fragment: StageBindings<'a>,
}

fn check_stage(stage: &str, r: Reflection, b: &StageBindings<'_>) -> Result<(), Diagnostic> {
    let pairs = [
        ("samplers", usize::from(r.samplers), b.samplers.len()),
        (
            "storage buffers",
            usize::from(r.storage_buffers),
            b.storage_buffers.len(),
        ),
        (
            "uniform buffers",
            usize::from(r.uniform_buffers),
            b.uniforms.len(),
        ),
    ];
    for (what, want, got) in pairs {
        if want != got {
            return Err(Diagnostic::error(
                BINDING_MISMATCH,
                format!("{stage} shader declares {want} {what}, the draw binds {got}"),
            ));
        }
    }
    if r.storage_textures != 0 {
        return Err(Diagnostic::error(
            BINDING_MISMATCH,
            format!(
                "{stage} shader declares {} storage textures, which graphics bindings do not cover",
                r.storage_textures
            ),
        ));
    }
    if let Some(u) = b.uniforms.iter().find(|u| u32::try_from(u.len()).is_err()) {
        return Err(Diagnostic::error(
            BINDING_MISMATCH,
            format!("{stage} uniform block of {} bytes is too large", u.len()),
        ));
    }
    Ok(())
}

/// Checks that `b` binds exactly what the (vertex, fragment) reflection declares.
pub fn check_bindings(
    reflection: (Reflection, Reflection),
    b: &Bindings<'_>,
) -> Result<(), Diagnostic> {
    check_stage("vertex", reflection.0, &b.vertex)?;
    check_stage("fragment", reflection.1, &b.fragment)
}

/// Binds `pipeline` and every resource of `b`, after [`check_bindings`] against
/// `reflection` (use [`PipelineCache::reflection`](crate::PipelineCache::reflection)).
/// Nothing is bound if the check fails.
///
/// # Safety
///
/// `pass` and `cmd` must be a live render pass and its command buffer, `pipeline` a live
/// pipeline whose shaders have `reflection`, and every texture, sampler and buffer in `b`
/// must be live objects of the same device.
pub unsafe fn bind_program(
    pass: NonNull<SDL_GPURenderPass>,
    cmd: NonNull<SDL_GPUCommandBuffer>,
    pipeline: NonNull<SDL_GPUGraphicsPipeline>,
    reflection: (Reflection, Reflection),
    b: &Bindings<'_>,
) -> Result<(), Diagnostic> {
    check_bindings(reflection, b)?;
    let (pass, cmd) = (pass.as_ptr(), cmd.as_ptr());
    // SAFETY: forwarded from the caller; slice pointers and lengths are valid for each call,
    // lengths fit u32 (checked above: they equal u16 reflection counts or were checked).
    unsafe {
        SDL_BindGPUGraphicsPipeline(pass, pipeline.as_ptr());
        for (stage, sb) in [(0, &b.vertex), (1, &b.fragment)] {
            if !sb.samplers.is_empty() {
                let n = sb.samplers.len() as u32;
                if stage == 0 {
                    SDL_BindGPUVertexSamplers(pass, 0, sb.samplers.as_ptr(), n);
                } else {
                    SDL_BindGPUFragmentSamplers(pass, 0, sb.samplers.as_ptr(), n);
                }
            }
            if !sb.storage_buffers.is_empty() {
                let n = sb.storage_buffers.len() as u32;
                if stage == 0 {
                    SDL_BindGPUVertexStorageBuffers(pass, 0, sb.storage_buffers.as_ptr(), n);
                } else {
                    SDL_BindGPUFragmentStorageBuffers(pass, 0, sb.storage_buffers.as_ptr(), n);
                }
            }
            for (slot, u) in sb.uniforms.iter().enumerate() {
                let (data, len) = (u.as_ptr().cast(), u.len() as u32);
                if stage == 0 {
                    SDL_PushGPUVertexUniformData(cmd, slot as u32, data, len);
                } else {
                    SDL_PushGPUFragmentUniformData(cmd, slot as u32, data, len);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(samplers: u16, storage_buffers: u16, uniform_buffers: u16) -> Reflection {
        Reflection {
            samplers,
            storage_textures: 0,
            storage_buffers,
            uniform_buffers,
        }
    }

    #[test]
    fn counts_must_match_reflection() {
        let params = [0u8; 16];
        let uniforms: [&[u8]; 1] = [&params];
        let buffers = [core::ptr::null_mut::<SDL_GPUBuffer>()];
        let b = Bindings {
            vertex: StageBindings {
                storage_buffers: &buffers,
                uniforms: &uniforms,
                ..Default::default()
            },
            fragment: StageBindings::default(),
        };
        assert!(check_bindings((r(0, 1, 1), r(0, 0, 0)), &b).is_ok());
        let e = check_bindings((r(0, 1, 1), r(1, 0, 0)), &b).unwrap_err();
        assert_eq!(e.code, BINDING_MISMATCH);
        assert!(e.message.contains("fragment"), "{}", e.message);
        assert!(check_bindings((r(0, 0, 1), r(0, 0, 0)), &b).is_err());
        let mut st = r(0, 0, 0);
        st.storage_textures = 1;
        assert!(check_bindings((r(0, 1, 1), st), &b).is_err());
    }
}
