// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/UploadBuffer.hpp and src/gpu/UploadBuffer.cpp.
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
// Altered: rewritten in Rust for the Knorke Engine. `Map`/`Unmap` became a guard that
// unmaps on drop and hands out a bounded byte slice; errors are `Diagnostic`s.

//! Cycled upload buffers, optionally paired with a GPU buffer.

use core::ptr::NonNull;

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CreateGPUBuffer, SDL_CreateGPUTransferBuffer, SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
    SDL_GPUBuffer, SDL_GPUBufferCreateInfo, SDL_GPUBufferRegion, SDL_GPUBufferUsageFlags,
    SDL_GPUCopyPass, SDL_GPUTransferBuffer, SDL_GPUTransferBufferCreateInfo,
    SDL_GPUTransferBufferLocation, SDL_MapGPUTransferBuffer, SDL_UnmapGPUTransferBuffer,
    SDL_UploadToGPUBuffer,
};

use crate::BufferRef;
use crate::handle::sdl_error;
use crate::staging::{MAX_GPU_BUFFER_BYTES, grow_capacity};
use crate::{Gpu, GpuHandle};

/// A reservation exceeds the 4 GiB size limit of one SDL_GPU buffer.
pub const UPLOAD_LIMIT: DiagnosticCode = DiagnosticCode::new("knorke.gpu.upload.limit");
/// Creating the transfer or GPU buffer failed.
pub const UPLOAD_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.upload.create_failed");
/// Mapping the transfer buffer failed.
pub const UPLOAD_MAP_FAILED: DiagnosticCode = DiagnosticCode::new("knorke.gpu.upload.map_failed");
/// [`UploadBuffer::map`] was called before a successful [`UploadBuffer::reserve`].
pub const UPLOAD_NOT_RESERVED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.upload.not_reserved");

/// A cycled upload (transfer) buffer, optionally paired with a GPU buffer of the same size
/// that it is copied into. Grows by doubling and never shrinks.
///
/// Per frame: [`reserve`](Self::reserve) and [`map`](Self::map) while preparing (failing is
/// allowed there), then record the copy with [`upload`](Self::upload) into a copy pass of the
/// host's command buffer. The crate never submits.
#[derive(Debug)]
pub struct UploadBuffer<'d> {
    gpu: Gpu<'d>,
    usage: SDL_GPUBufferUsageFlags,
    initial_bytes: u32,
    name: &'static str,
    capacity: u32,
    buffer: Option<GpuHandle<'d, SDL_GPUBuffer>>,
    transfer: Option<GpuHandle<'d, SDL_GPUTransferBuffer>>,
}

impl<'d> UploadBuffer<'d> {
    /// An empty buffer. Empty `usage` creates the transfer buffer only (texture uploads).
    /// `name` appears in error messages. Allocates nothing until [`reserve`](Self::reserve).
    pub fn new(
        gpu: Gpu<'d>,
        usage: SDL_GPUBufferUsageFlags,
        initial_bytes: u32,
        name: &'static str,
    ) -> Self {
        Self {
            gpu,
            usage,
            initial_bytes,
            name,
            capacity: 0,
            buffer: None,
            transfer: None,
        }
    }

    fn has_gpu_buffer(&self) -> bool {
        self.usage.0 != 0
    }

    /// Grows to hold at least `bytes`. On failure keeps the old buffers.
    pub fn reserve(&mut self, bytes: u64) -> Result<(), Diagnostic> {
        if bytes <= u64::from(self.capacity) && self.transfer.is_some() {
            return Ok(());
        }
        let Some(grown) = grow_capacity(
            u64::from(self.capacity),
            u64::from(self.initial_bytes),
            bytes,
            MAX_GPU_BUFFER_BYTES,
        ) else {
            return Err(Diagnostic::error(
                UPLOAD_LIMIT,
                format!(
                    "{bytes} bytes of {} exceed the GPU buffer limit of {MAX_GPU_BUFFER_BYTES} bytes",
                    self.name
                ),
            ));
        };
        // grow_capacity caps at MAX_GPU_BUFFER_BYTES == u32::MAX.
        let capacity = u32::try_from(grown).unwrap_or(u32::MAX);
        let device = self.gpu.device().as_ptr();

        let buffer = if self.has_gpu_buffer() {
            let info = SDL_GPUBufferCreateInfo {
                usage: self.usage,
                size: capacity,
                ..Default::default()
            };
            // SAFETY: the device is live for 'd; `info` is fully initialised.
            let raw = unsafe { SDL_CreateGPUBuffer(device, &info) };
            // SAFETY: `raw` was just created on this device and is owned by nobody else.
            unsafe { GpuHandle::from_raw(self.gpu, raw) }
        } else {
            None
        };
        let info = SDL_GPUTransferBufferCreateInfo {
            usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
            size: capacity,
            ..Default::default()
        };
        // SAFETY: the device is live for 'd; `info` is fully initialised.
        let raw = unsafe { SDL_CreateGPUTransferBuffer(device, &info) };
        // SAFETY: as above.
        let transfer = unsafe { GpuHandle::from_raw(self.gpu, raw) };

        match transfer {
            Some(transfer) if buffer.is_some() || !self.has_gpu_buffer() => {
                self.buffer = buffer;
                self.transfer = Some(transfer);
                self.capacity = capacity;
                Ok(())
            }
            // Dropping the half-created pair releases it; the old buffers stay.
            _ => Err(Diagnostic::error(
                UPLOAD_CREATE_FAILED,
                format!(
                    "creating the {} buffers ({capacity} bytes) failed: {}",
                    self.name,
                    sdl_error()
                ),
            )),
        }
    }

    /// Maps the upload buffer, cycling it, so the GPU may still read the old contents.
    /// The returned guard unmaps on drop.
    pub fn map(&mut self) -> Result<Mapped<'_, 'd>, Diagnostic> {
        let Some(transfer) = self.transfer.as_ref() else {
            return Err(Diagnostic::error(
                UPLOAD_NOT_RESERVED,
                format!("mapping the {} upload buffer before reserve", self.name),
            ));
        };
        let device = self.gpu.device().as_ptr();
        let transfer = transfer.as_ptr();
        // SAFETY: device and transfer buffer are live; `cycle = true` gives us a fresh
        // backing store the GPU is not reading.
        let ptr = unsafe { SDL_MapGPUTransferBuffer(device, transfer, true) };
        let Some(ptr) = NonNull::new(ptr.cast::<u8>()) else {
            return Err(Diagnostic::error(
                UPLOAD_MAP_FAILED,
                format!(
                    "mapping the {} upload buffer failed: {}",
                    self.name,
                    sdl_error()
                ),
            ));
        };
        let len = self.capacity as usize;
        Ok(Mapped {
            owner: self,
            ptr,
            len,
        })
    }

    /// Records the copy of the first `bytes` (clamped to the capacity) of the upload buffer
    /// into the GPU buffer. No-op without a GPU buffer or before `reserve`.
    ///
    /// # Safety
    ///
    /// `copy` must be a copy pass that is currently open on a command buffer of this
    /// buffer's device. The buffer must not be mapped (guaranteed by the borrow of
    /// [`Mapped`]).
    pub unsafe fn upload(&self, copy: NonNull<SDL_GPUCopyPass>, bytes: u32) {
        let (Some(buffer), Some(transfer)) = (self.buffer.as_ref(), self.transfer.as_ref()) else {
            return;
        };
        let src = SDL_GPUTransferBufferLocation {
            transfer_buffer: transfer.as_ptr(),
            offset: 0,
        };
        let dst = SDL_GPUBufferRegion {
            buffer: buffer.as_ptr(),
            offset: 0,
            size: bytes.min(self.capacity),
        };
        // SAFETY: the caller guarantees an open copy pass on this device; both buffers are
        // live and the region lies within them.
        unsafe { SDL_UploadToGPUBuffer(copy.as_ptr(), &src, &dst, true) };
    }

    /// Releases both buffers; the next `reserve` allocates again.
    pub fn reset(&mut self) {
        self.buffer = None;
        self.transfer = None;
        self.capacity = 0;
    }

    /// Current capacity in bytes (0 before `reserve`).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// The GPU buffer, if this upload buffer has one and is reserved.
    pub fn buffer(&self) -> Option<*mut SDL_GPUBuffer> {
        self.buffer.as_ref().map(GpuHandle::as_ptr)
    }

    /// The GPU buffer as a view for binding, if this upload buffer has one and is reserved.
    pub fn buffer_ref(&self) -> Option<BufferRef<'_>> {
        self.buffer
            .as_ref()
            .map(|b| BufferRef::of_handle(b, self.usage, self.capacity))
    }

    pub(crate) fn device_ptr(&self) -> *mut sdl3_sys::gpu::SDL_GPUDevice {
        self.gpu.device().as_ptr()
    }

    /// The transfer buffer, once reserved.
    pub fn transfer(&self) -> Option<*mut SDL_GPUTransferBuffer> {
        self.transfer.as_ref().map(GpuHandle::as_ptr)
    }
}

/// A mapped upload buffer; unmaps on drop.
#[derive(Debug)]
pub struct Mapped<'a, 'd> {
    owner: &'a mut UploadBuffer<'d>,
    ptr: NonNull<u8>,
    len: usize,
}

impl Mapped<'_, '_> {
    /// The whole mapped buffer (its capacity).
    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: SDL mapped `len` (= capacity) bytes at `ptr`, writable, until unmap in
        // drop; `&mut self` makes this the only access.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for Mapped<'_, '_> {
    fn drop(&mut self) {
        if let Some(transfer) = self.owner.transfer.as_ref() {
            // SAFETY: the transfer buffer is mapped (we mapped it) and live.
            unsafe {
                SDL_UnmapGPUTransferBuffer(self.owner.gpu.device().as_ptr(), transfer.as_ptr())
            };
        }
    }
}
