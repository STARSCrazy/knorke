//! Borrowed views of GPU objects for recording ([`TextureRef`], [`SamplerRef`],
//! [`BufferRef`]), plus owned textures ([`Texture`]) and readback buffers
//! ([`DownloadBuffer`]) for hosts and tests.
//!
//! A view borrows its owner for `'a`, so the owner can neither be dropped nor reallocated
//! (for example by [`RenderTarget::ensure`](crate::RenderTarget::ensure)) while the view is
//! in use:
//!
//! ```compile_fail
//! # use knorke_gpu::{Device, Gpu, LiveCounter, RenderTarget};
//! # use knorke_gpu::sys::gpu::*;
//! # let device = Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
//! # let counter = LiveCounter::new();
//! let mut target = RenderTarget::new(Gpu::new(&device, &counter));
//! target.ensure(4, 4, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM).unwrap();
//! let view = target.texture_ref().unwrap();
//! target.ensure(8, 8, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM).unwrap(); // reallocates
//! let _ = view.width(); // `view` still borrows the old texture
//! ```
//!
//! ```compile_fail
//! # use knorke_gpu::{Device, Gpu, LiveCounter, Texture, TextureDesc};
//! # use knorke_gpu::sys::gpu::*;
//! # let device = Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
//! # let counter = LiveCounter::new();
//! # let gpu = Gpu::new(&device, &counter);
//! let view;
//! {
//!     let desc = TextureDesc::new(4, 4, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
//!     let texture = Texture::new(gpu, desc).unwrap();
//!     view = texture.texture_ref();
//! } // `texture` released here while `view` borrows it
//! let _ = view.width();
//! ```

use core::marker::PhantomData;
use core::ptr::NonNull;

use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_CalculateGPUTextureFormatSize, SDL_CreateGPUTexture, SDL_CreateGPUTransferBuffer,
    SDL_GPU_TEXTURETYPE_2D, SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, SDL_GPUBuffer,
    SDL_GPUBufferUsageFlags, SDL_GPUDevice, SDL_GPUSampler, SDL_GPUTexture,
    SDL_GPUTextureCreateInfo, SDL_GPUTextureFormat, SDL_GPUTextureUsageFlags,
    SDL_GPUTransferBuffer, SDL_GPUTransferBufferCreateInfo, SDL_MapGPUTransferBuffer,
    SDL_UnmapGPUTransferBuffer,
};

use crate::handle::sdl_error;
use crate::{Device, Gpu, GpuHandle};

/// `SDL_CreateGPUTexture` failed or the description was invalid.
pub const TEXTURE_CREATE_FAILED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.texture.create_failed");
/// Creating or mapping a download buffer failed, or the read was larger than the buffer.
pub const DOWNLOAD_FAILED: DiagnosticCode = DiagnosticCode::new("knorke.gpu.download.failed");

/// Size, format and usage of a 2D texture with one layer and one mip level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureDesc {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Texel format.
    pub format: SDL_GPUTextureFormat,
    /// Usage flags the texture was created with.
    pub usage: SDL_GPUTextureUsageFlags,
}

impl TextureDesc {
    /// A description from its parts.
    pub const fn new(
        width: u32,
        height: u32,
        format: SDL_GPUTextureFormat,
        usage: SDL_GPUTextureUsageFlags,
    ) -> Self {
        Self {
            width,
            height,
            format,
            usage,
        }
    }

    /// Bytes of the whole texture in its format (tightly packed).
    pub fn bytes(&self) -> u64 {
        // SAFETY: pure computation on plain values.
        u64::from(unsafe {
            SDL_CalculateGPUTextureFormatSize(self.format, self.width, self.height, 1)
        })
    }
}

/// A texture borrowed for `'a`, with the description recording checks against.
#[derive(Clone, Copy, Debug)]
pub struct TextureRef<'a> {
    raw: NonNull<SDL_GPUTexture>,
    device: NonNull<SDL_GPUDevice>,
    desc: TextureDesc,
    _borrow: PhantomData<&'a SDL_GPUTexture>,
}

impl<'a> TextureRef<'a> {
    pub(crate) fn of_handle(handle: &'a GpuHandle<'_, SDL_GPUTexture>, desc: TextureDesc) -> Self {
        Self {
            raw: handle.non_null(),
            device: handle.gpu().device().non_null(),
            desc,
            _borrow: PhantomData,
        }
    }

    /// Wraps a texture the host created with its own SDL calls (e.g. a swapchain texture).
    ///
    /// # Safety
    ///
    /// `raw` must be a live texture of `device`, created with exactly `desc` (2D, one layer,
    /// one level), and stay alive for `'a`.
    pub unsafe fn from_raw(
        device: &Device,
        raw: NonNull<SDL_GPUTexture>,
        desc: TextureDesc,
    ) -> Self {
        Self {
            raw,
            device: device.non_null(),
            desc,
            _borrow: PhantomData,
        }
    }

    /// The description.
    pub fn desc(&self) -> TextureDesc {
        self.desc
    }

    /// Width in texels.
    pub fn width(&self) -> u32 {
        self.desc.width
    }

    /// Height in texels.
    pub fn height(&self) -> u32 {
        self.desc.height
    }

    /// Format.
    pub fn format(&self) -> SDL_GPUTextureFormat {
        self.desc.format
    }

    /// The raw texture, for the host's own FFI calls.
    pub fn as_ptr(&self) -> *mut SDL_GPUTexture {
        self.raw.as_ptr()
    }

    pub(crate) fn device_ptr(&self) -> *mut SDL_GPUDevice {
        self.device.as_ptr()
    }
}

/// A sampler borrowed for `'a` (see [`Samplers::sampler`](crate::Samplers::sampler)).
#[derive(Clone, Copy, Debug)]
pub struct SamplerRef<'a> {
    raw: NonNull<SDL_GPUSampler>,
    device: NonNull<SDL_GPUDevice>,
    _borrow: PhantomData<&'a SDL_GPUSampler>,
}

impl<'a> SamplerRef<'a> {
    pub(crate) fn of_handle(handle: &'a GpuHandle<'_, SDL_GPUSampler>) -> Self {
        Self {
            raw: handle.non_null(),
            device: handle.gpu().device().non_null(),
            _borrow: PhantomData,
        }
    }

    /// The raw sampler.
    pub fn as_ptr(&self) -> *mut SDL_GPUSampler {
        self.raw.as_ptr()
    }

    pub(crate) fn device_ptr(&self) -> *mut SDL_GPUDevice {
        self.device.as_ptr()
    }
}

/// A GPU buffer borrowed for `'a` (see [`UploadBuffer::buffer_ref`](crate::UploadBuffer::buffer_ref)).
#[derive(Clone, Copy, Debug)]
pub struct BufferRef<'a> {
    raw: NonNull<SDL_GPUBuffer>,
    device: NonNull<SDL_GPUDevice>,
    usage: SDL_GPUBufferUsageFlags,
    size: u32,
    _borrow: PhantomData<&'a SDL_GPUBuffer>,
}

impl<'a> BufferRef<'a> {
    pub(crate) fn of_handle(
        handle: &'a GpuHandle<'_, SDL_GPUBuffer>,
        usage: SDL_GPUBufferUsageFlags,
        size: u32,
    ) -> Self {
        Self {
            raw: handle.non_null(),
            device: handle.gpu().device().non_null(),
            usage,
            size,
            _borrow: PhantomData,
        }
    }

    /// Usage flags.
    pub fn usage(&self) -> SDL_GPUBufferUsageFlags {
        self.usage
    }

    /// Size in bytes.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The raw buffer.
    pub fn as_ptr(&self) -> *mut SDL_GPUBuffer {
        self.raw.as_ptr()
    }

    pub(crate) fn device_ptr(&self) -> *mut SDL_GPUDevice {
        self.device.as_ptr()
    }
}

/// An owned 2D texture (one layer, one level), e.g. a host's scene layer or output target.
#[derive(Debug)]
pub struct Texture<'d> {
    handle: GpuHandle<'d, SDL_GPUTexture>,
    desc: TextureDesc,
}

impl<'d> Texture<'d> {
    /// Creates a texture; fails with [`TEXTURE_CREATE_FAILED`] on zero size or SDL failure.
    pub fn new(gpu: Gpu<'d>, desc: TextureDesc) -> Result<Self, Diagnostic> {
        if desc.width == 0 || desc.height == 0 {
            return Err(Diagnostic::error(
                TEXTURE_CREATE_FAILED,
                format!(
                    "texture size must be non-zero (got {}x{})",
                    desc.width, desc.height
                ),
            ));
        }
        let info = SDL_GPUTextureCreateInfo {
            r#type: SDL_GPU_TEXTURETYPE_2D,
            format: desc.format,
            usage: desc.usage,
            width: desc.width,
            height: desc.height,
            layer_count_or_depth: 1,
            num_levels: 1,
            ..Default::default()
        };
        // SAFETY: the device is live for 'd; `info` is fully initialised.
        let raw = unsafe { SDL_CreateGPUTexture(gpu.device().as_ptr(), &info) };
        // SAFETY: `raw` was just created on this device and is owned by nobody else.
        let handle = unsafe { GpuHandle::from_raw(gpu, raw) }.ok_or_else(|| {
            Diagnostic::error(
                TEXTURE_CREATE_FAILED,
                format!(
                    "creating a {}x{} texture failed: {}",
                    desc.width,
                    desc.height,
                    sdl_error()
                ),
            )
        })?;
        Ok(Self { handle, desc })
    }

    /// A view for recording, borrowing the texture.
    pub fn texture_ref(&self) -> TextureRef<'_> {
        TextureRef::of_handle(&self.handle, self.desc)
    }

    /// The description.
    pub fn desc(&self) -> TextureDesc {
        self.desc
    }
}

/// A download transfer buffer that GPU textures are read back into (host side, ADR 0002:
/// modules never read back on their own).
#[derive(Debug)]
pub struct DownloadBuffer<'d> {
    transfer: GpuHandle<'d, SDL_GPUTransferBuffer>,
    size: u32,
}

impl<'d> DownloadBuffer<'d> {
    /// A download buffer of `size` bytes.
    pub fn new(gpu: Gpu<'d>, size: u32) -> Result<Self, Diagnostic> {
        let info = SDL_GPUTransferBufferCreateInfo {
            usage: SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD,
            size,
            ..Default::default()
        };
        // SAFETY: the device is live for 'd; `info` is fully initialised.
        let raw = unsafe { SDL_CreateGPUTransferBuffer(gpu.device().as_ptr(), &info) };
        // SAFETY: `raw` was just created on this device and is owned by nobody else.
        let transfer = unsafe { GpuHandle::from_raw(gpu, raw) }.ok_or_else(|| {
            Diagnostic::error(
                DOWNLOAD_FAILED,
                format!(
                    "creating a {size} byte download buffer failed: {}",
                    sdl_error()
                ),
            )
        })?;
        Ok(Self { transfer, size })
    }

    /// Size in bytes.
    pub fn size(&self) -> u32 {
        self.size
    }

    pub(crate) fn raw(&self) -> *mut SDL_GPUTransferBuffer {
        self.transfer.as_ptr()
    }

    pub(crate) fn device_ptr(&self) -> *mut SDL_GPUDevice {
        self.transfer.gpu().device().as_ptr()
    }

    /// Copies the first `out.len()` bytes into `out`. Call it after the command buffer with
    /// the download has completed (e.g. [`CommandBuffer::submit_and_wait`](crate::CommandBuffer::submit_and_wait)).
    pub fn read(&mut self, out: &mut [u8]) -> Result<(), Diagnostic> {
        if out.len() > self.size as usize {
            return Err(Diagnostic::error(
                DOWNLOAD_FAILED,
                format!(
                    "reading {} bytes from a {} byte download buffer",
                    out.len(),
                    self.size
                ),
            ));
        }
        let device = self.device_ptr();
        // SAFETY: device and transfer buffer are live; no cycling, we read the last download.
        let ptr = unsafe { SDL_MapGPUTransferBuffer(device, self.raw(), false) };
        let Some(ptr) = NonNull::new(ptr.cast::<u8>()) else {
            return Err(Diagnostic::error(
                DOWNLOAD_FAILED,
                format!("mapping the download buffer failed: {}", sdl_error()),
            ));
        };
        // SAFETY: SDL mapped `size` >= `out.len()` readable bytes at `ptr` until the unmap
        // below; `out` is a distinct Rust allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.as_ptr(), out.as_mut_ptr(), out.len());
            SDL_UnmapGPUTransferBuffer(device, self.raw());
        }
        Ok(())
    }
}
