// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/RenderTarget.hpp and src/gpu/RenderTarget.cpp.
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
// Altered: rewritten in Rust for the Knorke Engine; errors are `Diagnostic`s instead of the
// SDL error string, and a recreate counter was added.

//! Persistent intermediate render targets.

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CreateGPUTexture, SDL_GPU_TEXTUREFORMAT_INVALID, SDL_GPU_TEXTURETYPE_2D,
    SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUTexture,
    SDL_GPUTextureCreateInfo, SDL_GPUTextureFormat,
};

use crate::handle::sdl_error;
use crate::{Gpu, GpuHandle, TextureDesc, TextureRef};

/// [`RenderTarget::ensure`] was called with a zero width or height.
pub const RENDER_TARGET_ZERO_SIZE: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.render_target.zero_size");
/// `SDL_CreateGPUTexture` failed for a render target.
pub const RENDER_TARGET_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.render_target.create_failed");

/// Persistent intermediate render target (`COLOR_TARGET | SAMPLER`).
///
/// The texture is recreated only when the requested size or format changes, so calling
/// [`ensure`](Self::ensure) every frame costs nothing in the steady state.
#[derive(Debug)]
pub struct RenderTarget<'d> {
    gpu: Gpu<'d>,
    texture: Option<GpuHandle<'d, SDL_GPUTexture>>,
    width: u32,
    height: u32,
    format: SDL_GPUTextureFormat,
    recreated: u32,
}

impl<'d> RenderTarget<'d> {
    /// An empty target; the first [`ensure`](Self::ensure) allocates.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            gpu,
            texture: None,
            width: 0,
            height: 0,
            format: SDL_GPU_TEXTUREFORMAT_INVALID,
            recreated: 0,
        }
    }

    /// Makes sure a texture of this size and format exists. Returns `true` if it was
    /// (re)allocated, so the caller knows old contents are gone.
    ///
    /// On zero size or allocation failure returns an error and keeps the old texture.
    pub fn ensure(
        &mut self,
        width: u32,
        height: u32,
        format: SDL_GPUTextureFormat,
    ) -> Result<bool, Diagnostic> {
        if width == 0 || height == 0 {
            return Err(Diagnostic::error(
                RENDER_TARGET_ZERO_SIZE,
                format!("render target size must be non-zero (got {width}x{height})"),
            ));
        }
        if self.texture.is_some()
            && width == self.width
            && height == self.height
            && format == self.format
        {
            return Ok(false);
        }

        let info = SDL_GPUTextureCreateInfo {
            r#type: SDL_GPU_TEXTURETYPE_2D,
            format,
            usage: SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER,
            width,
            height,
            layer_count_or_depth: 1,
            num_levels: 1,
            ..Default::default()
        };
        let device = self.gpu.device().as_ptr();
        // SAFETY: `device` is live for 'd; `info` is a fully initialised create info.
        let raw = unsafe { SDL_CreateGPUTexture(device, &info) };
        // SAFETY: `raw` was just created on this device and is owned by nobody else.
        let Some(texture) = (unsafe { GpuHandle::from_raw(self.gpu, raw) }) else {
            return Err(Diagnostic::error(
                RENDER_TARGET_CREATE_FAILED,
                format!(
                    "creating a {width}x{height} render target failed: {}",
                    sdl_error()
                ),
            ));
        };

        self.texture = Some(texture);
        self.width = width;
        self.height = height;
        self.format = format;
        self.recreated += 1;
        Ok(true)
    }

    /// Releases the texture (no-op without one); the next [`ensure`](Self::ensure) allocates.
    pub fn release(&mut self) {
        self.texture = None;
        self.width = 0;
        self.height = 0;
        self.format = SDL_GPU_TEXTUREFORMAT_INVALID;
    }

    /// The texture, if one is allocated. Valid until the next `ensure`/`release`.
    pub fn texture(&self) -> Option<*mut SDL_GPUTexture> {
        self.texture.as_ref().map(GpuHandle::as_ptr)
    }

    /// The texture as a view for recording, if one is allocated. The view borrows the
    /// target, so it cannot be reallocated or released while the view is in use.
    pub fn texture_ref(&self) -> Option<TextureRef<'_>> {
        let desc = TextureDesc::new(
            self.width,
            self.height,
            self.format,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER,
        );
        self.texture
            .as_ref()
            .map(|t| TextureRef::of_handle(t, desc))
    }

    /// Current width (0 without texture).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Current height (0 without texture).
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Current format (`INVALID` without texture).
    pub fn format(&self) -> SDL_GPUTextureFormat {
        self.format
    }

    /// How often a texture was allocated over the lifetime of this target. A value that
    /// grows every frame points at a size or format that keeps changing.
    pub fn recreated(&self) -> u32 {
        self.recreated
    }
}
