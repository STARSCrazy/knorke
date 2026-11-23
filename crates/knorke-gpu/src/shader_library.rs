// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/ShaderLibrary.hpp and src/gpu/ShaderLibrary.cpp.
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
// Altered: rewritten in Rust for the Knorke Engine. Shaders come from a `.kpak` pack the
// host chooses (ADR 0014 §3) instead of tables compiled into the library; the pack holds one
// format, so the format is no longer picked per device but checked against it. Shaders are
// addressed by name or by a resolved `ShaderId` instead of a generated enum. Errors are
// `Diagnostic`s instead of the SDL error string.

//! Shaders of one pack on one device, created on first use.

use std::borrow::Cow;
use std::ffi::CString;
use std::path::Path;

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CreateGPUShader, SDL_GPU_SHADERFORMAT_DXIL, SDL_GPU_SHADERFORMAT_MSL,
    SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_SHADERSTAGE_FRAGMENT, SDL_GPU_SHADERSTAGE_VERTEX,
    SDL_GPUShader, SDL_GPUShaderCreateInfo, SDL_GPUShaderFormat, SDL_GetGPUShaderFormats,
};

use crate::handle::sdl_error;
use crate::kpak::{
    Program, Reflection, SHADER_PACK_IO, SHADER_PACK_MISSING_PROGRAM,
    SHADER_PACK_UNSUPPORTED_FORMAT, ShaderFormat, ShaderPack, Stage,
};
use crate::{Gpu, GpuHandle};

/// `SDL_CreateGPUShader` failed for a program of the pack.
pub const SHADER_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader.create_failed");

/// The SDL flag of a pack format.
pub fn sdl_shader_format(format: ShaderFormat) -> SDL_GPUShaderFormat {
    match format {
        ShaderFormat::Spirv => SDL_GPU_SHADERFORMAT_SPIRV,
        ShaderFormat::Msl => SDL_GPU_SHADERFORMAT_MSL,
        ShaderFormat::Dxil => SDL_GPU_SHADERFORMAT_DXIL,
    }
}

/// The pack format to load for a device that accepts `supported`: SPIR-V before MSL before
/// DXIL (ADR 0014). `None` if the device takes none of them.
pub fn preferred_format(supported: SDL_GPUShaderFormat) -> Option<ShaderFormat> {
    ShaderFormat::ALL
        .into_iter()
        .find(|&f| supported.0 & sdl_shader_format(f).0 != 0)
}

/// The shader formats a device accepts.
pub fn device_formats(gpu: Gpu<'_>) -> SDL_GPUShaderFormat {
    // SAFETY: the device is live for the borrow.
    unsafe { SDL_GetGPUShaderFormats(gpu.device().as_ptr()) }
}

/// Index of a program in a [`ShaderLibrary`]'s pack; resolve once with
/// [`ShaderLibrary::id`], then use it without string lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShaderId(u32);

impl ShaderId {
    /// The table position in the pack.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The shaders of one validated pack on a device. A shader is created by the first
/// [`shader`](Self::shader) call that asks for it and lives as long as the library.
pub struct ShaderLibrary<'d> {
    gpu: Gpu<'d>,
    bytes: Cow<'static, [u8]>,
    shaders: Vec<Option<GpuHandle<'d, SDL_GPUShader>>>,
}

impl core::fmt::Debug for ShaderLibrary<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShaderLibrary")
            .field("bytes", &self.bytes.len())
            .field("created", &self.created_count())
            .finish()
    }
}

impl<'d> ShaderLibrary<'d> {
    /// Validates a pack (owned bytes, or `&'static` bytes from `include_bytes!` for an
    /// embedded pack) and checks that the device accepts its format. Creates no GPU object.
    pub fn new(gpu: Gpu<'d>, bytes: impl Into<Cow<'static, [u8]>>) -> Result<Self, Diagnostic> {
        let bytes = bytes.into();
        let pack = ShaderPack::parse(&bytes)?;
        let format = pack.format();
        let count = pack.len();
        if device_formats(gpu).0 & sdl_shader_format(format).0 == 0 {
            return Err(Diagnostic::error(
                SHADER_PACK_UNSUPPORTED_FORMAT,
                format!(
                    "the device does not accept {} shaders; load the pack for its backend",
                    format.tag()
                ),
            ));
        }
        let mut shaders = Vec::with_capacity(count);
        shaders.resize_with(count, || None);
        Ok(Self {
            gpu,
            bytes,
            shaders,
        })
    }

    /// Reads and validates the pack file at `path` ([`SHADER_PACK_IO`] if it cannot be read).
    pub fn load(gpu: Gpu<'d>, path: &Path) -> Result<Self, Diagnostic> {
        let bytes = std::fs::read(path).map_err(|e| {
            Diagnostic::error(SHADER_PACK_IO, format!("reading shader pack failed: {e}"))
                .at(path.display().to_string())
        })?;
        Self::new(gpu, bytes).map_err(|d| d.at(path.display().to_string()))
    }

    /// The pack view. Never fails: the bytes were validated in [`new`](Self::new).
    pub fn pack(&self) -> ShaderPack<'_> {
        pack_view(&self.bytes).expect("validated in ShaderLibrary::new")
    }

    fn program(&self, id: ShaderId) -> Option<Program<'_>> {
        pack_view(&self.bytes)?.program(id.index())
    }

    /// The pack's shader format.
    pub fn format(&self) -> ShaderFormat {
        pack_view(&self.bytes).map_or(ShaderFormat::Spirv, |p| p.format())
    }

    /// Resolves a program name, or [`SHADER_PACK_MISSING_PROGRAM`].
    pub fn id(&self, name: &str) -> Result<ShaderId, Diagnostic> {
        pack_view(&self.bytes)
            .and_then(|p| p.find(name))
            .map(|i| ShaderId(i as u32))
            .ok_or_else(|| {
                Diagnostic::error(
                    SHADER_PACK_MISSING_PROGRAM,
                    format!("shader pack has no program '{name}'"),
                )
            })
    }

    /// Checks at start-up that every program in `names` exists, so a stale pack fails early
    /// with one diagnostic naming the first missing program.
    pub fn require(&self, names: &[&str]) -> Result<(), Diagnostic> {
        names.iter().try_for_each(|n| self.id(n).map(|_| ()))
    }

    /// Stage and reflection of a program.
    pub fn info(&self, id: ShaderId) -> Option<(Stage, Reflection)> {
        self.program(id).map(|p| (p.stage, p.reflection))
    }

    /// The shader, created on first use. Valid while `self` lives.
    pub fn shader(&mut self, id: ShaderId) -> Result<*mut SDL_GPUShader, Diagnostic> {
        if let Some(Some(h)) = self.shaders.get(id.index()) {
            return Ok(h.as_ptr());
        }
        let gpu = self.gpu;
        let program = self.program(id).ok_or_else(|| {
            Diagnostic::error(
                SHADER_PACK_MISSING_PROGRAM,
                format!("no program at index {}", id.index()),
            )
        })?;
        let name = program.name.to_owned();
        let fail = |why: String| {
            Diagnostic::error(
                SHADER_CREATE_FAILED,
                format!("creating shader '{name}' failed: {why}"),
            )
        };
        let entry = CString::new(program.entry_point)
            .map_err(|_| fail("entry point contains NUL".into()))?;
        let r = program.reflection;
        let info = SDL_GPUShaderCreateInfo {
            code_size: program.code.len(),
            code: program.code.as_ptr(),
            entrypoint: entry.as_ptr(),
            format: sdl_shader_format(self.format()),
            stage: match program.stage {
                Stage::Vertex => SDL_GPU_SHADERSTAGE_VERTEX,
                Stage::Fragment => SDL_GPU_SHADERSTAGE_FRAGMENT,
            },
            num_samplers: u32::from(r.samplers),
            num_storage_textures: u32::from(r.storage_textures),
            num_storage_buffers: u32::from(r.storage_buffers),
            num_uniform_buffers: u32::from(r.uniform_buffers),
            ..Default::default()
        };
        // SAFETY: the device is live for 'd; `info` points into `self.bytes` and `entry`,
        // both alive for the call; SDL copies what it keeps.
        let raw = unsafe { SDL_CreateGPUShader(gpu.device().as_ptr(), &info) };
        // SAFETY: `raw` was just created on this device and is owned by nobody else.
        let handle = unsafe { GpuHandle::from_raw(gpu, raw) }.ok_or_else(|| fail(sdl_error()))?;
        let ptr = handle.as_ptr();
        self.shaders[id.index()] = Some(handle);
        Ok(ptr)
    }

    /// Shorthand for [`id`](Self::id) plus [`shader`](Self::shader).
    pub fn shader_by_name(&mut self, name: &str) -> Result<*mut SDL_GPUShader, Diagnostic> {
        let id = self.id(name)?;
        self.shader(id)
    }

    /// How many shaders exist so far (each counts as one live GPU object).
    pub fn created_count(&self) -> usize {
        self.shaders.iter().filter(|s| s.is_some()).count()
    }

    /// The device and counter.
    pub fn gpu(&self) -> Gpu<'d> {
        self.gpu
    }
}

/// A view of already validated bytes without repeating the checksum pass.
fn pack_view(bytes: &[u8]) -> Option<ShaderPack<'_>> {
    ShaderPack::parse_validated(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_format_order() {
        let all = SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_DXIL;
        assert_eq!(preferred_format(all), Some(ShaderFormat::Spirv));
        assert_eq!(
            preferred_format(SDL_GPU_SHADERFORMAT_DXIL),
            Some(ShaderFormat::Dxil)
        );
        assert_eq!(
            preferred_format(SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_DXIL),
            Some(ShaderFormat::Msl)
        );
        assert_eq!(preferred_format(SDL_GPUShaderFormat(0)), None);
    }
}
