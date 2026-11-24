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
// reflection from the pack before anything is bound. Binding itself happens safely in
// `RenderPass::bind_program` (record.rs) on borrowed resource views.

//! Resources of a draw in slot order, checked against the shaders' reflection.

use knorke_types::{Diagnostic, DiagnosticCode};

use crate::kpak::Reflection;
use crate::{BufferRef, SamplerRef, TextureRef};

/// The resources of a draw do not match what the program's shaders declare.
pub const BINDING_MISMATCH: DiagnosticCode = DiagnosticCode::new("knorke.gpu.binding.mismatch");

/// A texture sampled through a sampler.
#[derive(Clone, Copy, Debug)]
pub struct SamplerBinding<'a> {
    /// The sampled texture (needs `SAMPLER` usage).
    pub texture: TextureRef<'a>,
    /// The sampler.
    pub sampler: SamplerRef<'a>,
}

impl<'a> SamplerBinding<'a> {
    /// A texture-sampler pair.
    pub fn new(texture: TextureRef<'a>, sampler: SamplerRef<'a>) -> Self {
        Self { texture, sampler }
    }
}

/// Resources of one stage, each slice in slot order starting at slot 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageBindings<'a> {
    /// Texture-sampler pairs.
    pub samplers: &'a [SamplerBinding<'a>],
    /// Read-only storage textures (`GRAPHICS_STORAGE_READ` usage).
    pub storage_textures: &'a [TextureRef<'a>],
    /// Read-only storage buffers (`GRAPHICS_STORAGE_READ` usage).
    pub storage_buffers: &'a [BufferRef<'a>],
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
            "storage textures",
            usize::from(r.storage_textures),
            b.storage_textures.len(),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(samplers: u16, storage_textures: u16, uniform_buffers: u16) -> Reflection {
        Reflection {
            samplers,
            storage_textures,
            storage_buffers: 0,
            uniform_buffers,
        }
    }

    #[test]
    fn counts_must_match_reflection() {
        let params = [0u8; 16];
        let uniforms: [&[u8]; 1] = [&params];
        let b = Bindings {
            vertex: StageBindings {
                uniforms: &uniforms,
                ..Default::default()
            },
            fragment: StageBindings::default(),
        };
        assert!(check_bindings((r(0, 0, 1), r(0, 0, 0)), &b).is_ok());
        let e = check_bindings((r(0, 0, 1), r(1, 0, 0)), &b).unwrap_err();
        assert_eq!(e.code, BINDING_MISMATCH);
        assert!(e.message.contains("fragment"), "{}", e.message);
        assert!(check_bindings((r(0, 0, 2), r(0, 0, 0)), &b).is_err());
        let e = check_bindings((r(0, 0, 1), r(0, 1, 0)), &b).unwrap_err();
        assert!(e.message.contains("storage textures"), "{}", e.message);
    }
}
