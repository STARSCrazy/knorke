// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/Samplers.hpp and src/gpu/Samplers.cpp.
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
// Altered: rewritten in Rust for the Knorke Engine; `Init` became a constructor that
// returns a `Diagnostic` on failure, `CHI_Filter` became `Filter`.

//! The fixed sampler set: nearest and linear, clamp to edge.

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CreateGPUSampler, SDL_GPU_FILTER_LINEAR, SDL_GPU_FILTER_NEAREST,
    SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE, SDL_GPU_SAMPLERMIPMAPMODE_NEAREST, SDL_GPUFilter,
    SDL_GPUSampler, SDL_GPUSamplerCreateInfo,
};

use crate::handle::sdl_error;
use crate::{Gpu, GpuHandle};

/// `SDL_CreateGPUSampler` failed.
pub const SAMPLER_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.sampler.create_failed");

/// Texture filter of a [`Samplers`] entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Nearest neighbour (pixel art, data textures).
    Nearest,
    /// Bilinear.
    Linear,
}

/// The fixed sampler set of a context.
#[derive(Debug)]
pub struct Samplers<'d> {
    nearest: GpuHandle<'d, SDL_GPUSampler>,
    linear: GpuHandle<'d, SDL_GPUSampler>,
}

fn create(
    gpu: Gpu<'_>,
    filter: SDL_GPUFilter,
) -> Result<GpuHandle<'_, SDL_GPUSampler>, Diagnostic> {
    let info = SDL_GPUSamplerCreateInfo {
        min_filter: filter,
        mag_filter: filter,
        mipmap_mode: SDL_GPU_SAMPLERMIPMAPMODE_NEAREST,
        address_mode_u: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_v: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_w: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        ..Default::default()
    };
    // SAFETY: the device is live for 'd; `info` is fully initialised.
    let raw = unsafe { SDL_CreateGPUSampler(gpu.device().as_ptr(), &info) };
    // SAFETY: `raw` was just created on this device and is owned by nobody else.
    unsafe { GpuHandle::from_raw(gpu, raw) }.ok_or_else(|| {
        Diagnostic::error(
            SAMPLER_CREATE_FAILED,
            format!("creating samplers failed: {}", sdl_error()),
        )
    })
}

impl<'d> Samplers<'d> {
    /// Creates both samplers. On failure nothing stays allocated.
    pub fn new(gpu: Gpu<'d>) -> Result<Self, Diagnostic> {
        Ok(Self {
            nearest: create(gpu, SDL_GPU_FILTER_NEAREST)?,
            linear: create(gpu, SDL_GPU_FILTER_LINEAR)?,
        })
    }

    /// The sampler for `filter`, valid while `self` lives.
    pub fn get(&self, filter: Filter) -> *mut SDL_GPUSampler {
        match filter {
            Filter::Nearest => self.nearest.as_ptr(),
            Filter::Linear => self.linear.as_ptr(),
        }
    }
}
