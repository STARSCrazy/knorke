// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/GpuHandle.hpp (GpuHandle, ReleaseGpuObject).
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
// Altered: rewritten in Rust for the Knorke Engine. The device is borrowed for a lifetime
// `'d` instead of held as a raw pointer, so no handle can outlive its device; a handle is
// never null (absence is `Option<GpuHandle>`). `Device` is new.

//! The device borrow and the owning handle for SDL_GPU objects.

use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::NonNull;

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::error::SDL_GetError;
use sdl3_sys::gpu::{
    SDL_CreateGPUDevice, SDL_DestroyGPUDevice, SDL_GPUBuffer, SDL_GPUDevice,
    SDL_GPUGraphicsPipeline, SDL_GPUSampler, SDL_GPUShader, SDL_GPUShaderFormat, SDL_GPUTexture,
    SDL_GPUTransferBuffer, SDL_ReleaseGPUBuffer, SDL_ReleaseGPUGraphicsPipeline,
    SDL_ReleaseGPUSampler, SDL_ReleaseGPUShader, SDL_ReleaseGPUTexture,
    SDL_ReleaseGPUTransferBuffer,
};

use crate::LiveCounter;

/// `SDL_CreateGPUDevice` returned null (no driver, no matching shader format, no GPU).
pub const DEVICE_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.device.create_failed");

/// The current SDL error text (empty if none).
pub(crate) fn sdl_error() -> String {
    let p = SDL_GetError();
    if p.is_null() {
        return String::new();
    }
    // SAFETY: SDL_GetError returns a valid NUL-terminated string owned by SDL that stays
    // valid until the next SDL call on this thread; we copy it out immediately.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Initialises SDL's video subsystem for a host or test that does not do it itself,
/// optionally forcing a GPU driver (`c"vulkan"`, `c"direct3d12"`, `c"metal"`) through the
/// `SDL_GPU_DRIVER` hint. Returns SDL's success flag; a failure also shows up as
/// [`DEVICE_CREATE_FAILED`] in [`Device::create`].
///
/// Calls are serialised: `SDL_Init` is not safe to run from several threads at once, and
/// tests of one binary call this in parallel. Without the lock a second thread could see the
/// video subsystem half initialised ("Video subsystem not initialized") or crash.
pub fn init_video(driver: Option<&CStr>) -> bool {
    static INIT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A poisoned lock only means another caller panicked; the guard is still usable.
    let _guard = INIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: plain FFI calls; the hint name and value are NUL-terminated strings that SDL
    // copies. The lock above keeps concurrent callers out of `SDL_Init`.
    unsafe {
        if let Some(driver) = driver {
            sdl3_sys::hints::SDL_SetHint(sdl3_sys::hints::SDL_HINT_GPU_DRIVER, driver.as_ptr());
        }
        sdl3_sys::init::SDL_Init(sdl3_sys::init::SDL_INIT_VIDEO)
    }
}

/// An SDL_GPU device.
///
/// The host owns the device (ADR 0002). It either creates one with [`Device::create`] or
/// lends an existing one with [`Device::borrow_raw`]. Every GPU object of this crate borrows
/// the `Device` for its lifetime, so the compiler rejects dropping the device first:
///
/// ```compile_fail
/// # use knorke_gpu::{Device, Gpu, LiveCounter, Samplers};
/// let counter = LiveCounter::new();
/// let samplers;
/// {
///     let device = Device::create(knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
///     samplers = Samplers::new(Gpu::new(&device, &counter)).unwrap();
/// } // `device` dropped here while `samplers` still borrows it
/// drop(samplers);
/// ```
#[derive(Debug)]
pub struct Device {
    raw: NonNull<SDL_GPUDevice>,
    owned: bool,
    // Raw pointer semantics: not Send/Sync until thread rules are decided.
    _not_send: PhantomData<*mut SDL_GPUDevice>,
}

impl Device {
    /// Creates a device that supports any of `formats`; destroyed when dropped.
    ///
    /// SDL must be initialised by the host (`SDL_Init(SDL_INIT_VIDEO)` for Vulkan).
    /// Fails with [`DEVICE_CREATE_FAILED`] when no GPU or driver is available.
    pub fn create(formats: SDL_GPUShaderFormat, debug: bool) -> Result<Self, Diagnostic> {
        // SAFETY: plain FFI call with valid arguments; a null name lets SDL pick the driver.
        let raw = unsafe { SDL_CreateGPUDevice(formats, debug, core::ptr::null()) };
        match NonNull::new(raw) {
            Some(raw) => Ok(Self {
                raw,
                owned: true,
                _not_send: PhantomData,
            }),
            None => Err(Diagnostic::error(
                DEVICE_CREATE_FAILED,
                format!("creating the GPU device failed: {}", sdl_error()),
            )),
        }
    }

    /// Wraps a device the host created; dropping the wrapper does not destroy it.
    ///
    /// Returns `None` for a null pointer.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid SDL_GPU device that stays alive (not destroyed) for as long as
    /// the returned `Device` exists.
    pub unsafe fn borrow_raw(raw: *mut SDL_GPUDevice) -> Option<Self> {
        NonNull::new(raw).map(|raw| Self {
            raw,
            owned: false,
            _not_send: PhantomData,
        })
    }

    /// The raw device pointer, for FFI calls of the host.
    pub fn as_ptr(&self) -> *mut SDL_GPUDevice {
        self.raw.as_ptr()
    }

    pub(crate) fn non_null(&self) -> NonNull<SDL_GPUDevice> {
        self.raw
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: we created the device and own it. All objects of this crate borrow
            // `self`, so none of them is still alive here.
            unsafe { SDL_DestroyGPUDevice(self.raw.as_ptr()) };
        }
    }
}

/// A borrowed device plus the liveness counter that GPU objects report to.
#[derive(Clone, Copy, Debug)]
pub struct Gpu<'d> {
    device: &'d Device,
    counter: &'d LiveCounter,
}

impl<'d> Gpu<'d> {
    /// Combines a device borrow and a counter.
    pub fn new(device: &'d Device, counter: &'d LiveCounter) -> Self {
        Self { device, counter }
    }

    /// The borrowed device.
    pub fn device(&self) -> &'d Device {
        self.device
    }

    /// The liveness counter.
    pub fn counter(&self) -> &'d LiveCounter {
        self.counter
    }
}

mod sealed {
    pub trait Sealed {}
}

/// An SDL_GPU object type that a [`GpuHandle`] can own.
pub trait GpuObject: sealed::Sealed {
    /// Releases `object` on `device`.
    ///
    /// # Safety
    ///
    /// `object` must be a live object of this type created on `device`, released only once.
    #[doc(hidden)]
    unsafe fn release(device: *mut SDL_GPUDevice, object: *mut Self);
}

macro_rules! gpu_object {
    ($($ty:ident => $release:ident),* $(,)?) => {$(
        impl sealed::Sealed for $ty {}
        impl GpuObject for $ty {
            unsafe fn release(device: *mut SDL_GPUDevice, object: *mut Self) {
                // SAFETY: forwarded from the caller's contract.
                unsafe { $release(device, object) }
            }
        }
    )*};
}

gpu_object! {
    SDL_GPUTexture => SDL_ReleaseGPUTexture,
    SDL_GPUSampler => SDL_ReleaseGPUSampler,
    SDL_GPUShader => SDL_ReleaseGPUShader,
    SDL_GPUGraphicsPipeline => SDL_ReleaseGPUGraphicsPipeline,
    SDL_GPUTransferBuffer => SDL_ReleaseGPUTransferBuffer,
    SDL_GPUBuffer => SDL_ReleaseGPUBuffer,
}

/// Unique owner of one SDL_GPU object. Releases it on drop and keeps the [`LiveCounter`] up
/// to date. Never null; "no object" is `Option<GpuHandle>`.
pub struct GpuHandle<'d, T: GpuObject> {
    gpu: Gpu<'d>,
    raw: NonNull<T>,
}

impl<'d, T: GpuObject> GpuHandle<'d, T> {
    /// Takes ownership of `raw`; returns `None` (and counts nothing) for null, so the result
    /// of an SDL create call can be passed in directly.
    ///
    /// # Safety
    ///
    /// A non-null `raw` must be a live object created on `gpu.device()` that nobody else
    /// releases.
    pub unsafe fn from_raw(gpu: Gpu<'d>, raw: *mut T) -> Option<Self> {
        let raw = NonNull::new(raw)?;
        gpu.counter.add();
        Some(Self { gpu, raw })
    }

    /// The raw object pointer, valid while the handle lives.
    pub fn as_ptr(&self) -> *mut T {
        self.raw.as_ptr()
    }

    pub(crate) fn non_null(&self) -> NonNull<T> {
        self.raw
    }

    /// The device and counter this handle belongs to.
    pub fn gpu(&self) -> Gpu<'d> {
        self.gpu
    }
}

impl<T: GpuObject> core::fmt::Debug for GpuHandle<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("GpuHandle").field(&self.raw).finish()
    }
}

impl<T: GpuObject> Drop for GpuHandle<'_, T> {
    fn drop(&mut self) {
        // SAFETY: `from_raw` established that we own a live object of type T on this device;
        // the device outlives us through the `'d` borrow; drop runs once.
        unsafe { T::release(self.gpu.device.as_ptr(), self.raw.as_ptr()) };
        self.gpu.counter.remove();
    }
}
