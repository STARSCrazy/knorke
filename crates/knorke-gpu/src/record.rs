//! Safe recording: a command buffer the host lends, and render and copy passes on it.

use core::marker::PhantomData;
use core::ptr::NonNull;

use bytemuck::Pod;
use knorke_types::{Diagnostic, DiagnosticCode};
use sdl3_sys::gpu::{
    SDL_AcquireGPUCommandBuffer, SDL_BeginGPUCopyPass, SDL_BeginGPURenderPass,
    SDL_BindGPUFragmentSamplers, SDL_BindGPUFragmentStorageBuffers,
    SDL_BindGPUFragmentStorageTextures, SDL_BindGPUGraphicsPipeline, SDL_BindGPUVertexSamplers,
    SDL_BindGPUVertexStorageBuffers, SDL_BindGPUVertexStorageTextures, SDL_CancelGPUCommandBuffer,
    SDL_CopyGPUTextureToTexture, SDL_DownloadFromGPUTexture, SDL_DrawGPUPrimitives,
    SDL_EndGPUCopyPass, SDL_EndGPURenderPass, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
    SDL_GPU_LOADOP_CLEAR, SDL_GPU_LOADOP_DONT_CARE, SDL_GPU_LOADOP_LOAD, SDL_GPU_STOREOP_STORE,
    SDL_GPU_TEXTUREFORMAT_INVALID, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPU_TEXTUREUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBuffer,
    SDL_GPUBufferRegion, SDL_GPUColorTargetInfo, SDL_GPUCommandBuffer, SDL_GPUCopyPass,
    SDL_GPUDevice, SDL_GPUGraphicsPipeline, SDL_GPURenderPass, SDL_GPUTexture,
    SDL_GPUTextureFormat, SDL_GPUTextureLocation, SDL_GPUTextureRegion,
    SDL_GPUTextureSamplerBinding, SDL_GPUTextureTransferInfo, SDL_GPUTextureUsageFlags,
    SDL_GPUTransferBufferLocation, SDL_GPUViewport, SDL_PushGPUFragmentUniformData,
    SDL_PushGPUVertexUniformData, SDL_ReleaseGPUFence, SDL_SubmitGPUCommandBuffer,
    SDL_SubmitGPUCommandBufferAndAcquireFence, SDL_UploadToGPUBuffer, SDL_UploadToGPUTexture,
    SDL_WaitForGPUFences,
};
use sdl3_sys::pixels::SDL_FColor;
use sdl3_sys::rect::SDL_Rect;

use crate::binding::check_bindings;
use crate::handle::sdl_error;
use crate::kpak::Reflection;
use crate::{
    Bindings, BufferRef, Device, DownloadBuffer, SamplerBinding, TextureDesc, TextureRef,
    UploadBuffer,
};

/// SDL failed to acquire or submit a command buffer or to begin a pass.
pub const RECORD_FAILED: DiagnosticCode = DiagnosticCode::new("knorke.gpu.record.failed");
/// A resource of another device was used.
pub const RECORD_WRONG_DEVICE: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.record.wrong_device");
/// A resource lacks the usage flag its use needs.
pub const RECORD_USAGE: DiagnosticCode = DiagnosticCode::new("knorke.gpu.record.usage");
/// A pipeline's target formats differ from the render pass's color targets.
pub const RECORD_TARGET_MISMATCH: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.record.target_mismatch");
/// A draw without a pipeline or without every resource its shaders declare.
pub const RECORD_NOT_BOUND: DiagnosticCode = DiagnosticCode::new("knorke.gpu.record.not_bound");
/// A slot, target count or copy size outside the limits.
pub const RECORD_OUT_OF_RANGE: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.record.out_of_range");

/// Most color targets of one render pass.
pub const MAX_COLOR_TARGETS: usize = 4;
/// Sampler slots per stage (SDL_GPU minimum guarantee).
pub const MAX_SAMPLERS: usize = 16;
/// Storage texture slots per stage.
pub const MAX_STORAGE_TEXTURES: usize = 8;
/// Storage buffer slots per stage.
pub const MAX_STORAGE_BUFFERS: usize = 8;
/// Uniform slots per stage.
pub const MAX_UNIFORMS: usize = 4;

fn wrong_device(what: &str) -> Diagnostic {
    Diagnostic::error(
        RECORD_WRONG_DEVICE,
        format!("{what} belongs to another device than the command buffer"),
    )
}

fn out_of_range(what: String) -> Diagnostic {
    Diagnostic::error(RECORD_OUT_OF_RANGE, what)
}

fn has_usage(usage: SDL_GPUTextureUsageFlags, flag: SDL_GPUTextureUsageFlags) -> bool {
    usage.0 & flag.0 == flag.0
}

/// A graphics pipeline borrowed from a [`PipelineCache`](crate::PipelineCache), with the
/// reflection of its shaders and its color target formats.
#[derive(Clone, Copy, Debug)]
pub struct PipelineRef<'a> {
    raw: NonNull<SDL_GPUGraphicsPipeline>,
    device: NonNull<SDL_GPUDevice>,
    reflection: (Reflection, Reflection),
    formats: [SDL_GPUTextureFormat; MAX_COLOR_TARGETS],
    targets: u8,
    _borrow: PhantomData<&'a SDL_GPUGraphicsPipeline>,
}

impl PipelineRef<'_> {
    pub(crate) fn new(
        raw: NonNull<SDL_GPUGraphicsPipeline>,
        device: NonNull<SDL_GPUDevice>,
        reflection: (Reflection, Reflection),
        formats: [SDL_GPUTextureFormat; MAX_COLOR_TARGETS],
        targets: u8,
    ) -> Self {
        Self {
            raw,
            device,
            reflection,
            formats,
            targets,
            _borrow: PhantomData,
        }
    }

    /// Reflection of the (vertex, fragment) shaders.
    pub fn reflection(&self) -> (Reflection, Reflection) {
        self.reflection
    }

    /// The color target formats the pipeline was built for.
    pub fn target_formats(&self) -> &[SDL_GPUTextureFormat] {
        &self.formats[..usize::from(self.targets)]
    }

    /// The raw pipeline.
    pub fn as_ptr(&self) -> *mut SDL_GPUGraphicsPipeline {
        self.raw.as_ptr()
    }
}

/// What happens to a color target's contents when the pass begins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Load {
    /// Keep the contents.
    Load,
    /// Clear to this linear RGBA colour.
    Clear([f32; 4]),
    /// Contents are undefined (every pixel is overwritten).
    DontCare,
}

/// One color target of a render pass. The result is always stored.
#[derive(Clone, Copy, Debug)]
pub struct ColorTarget<'a> {
    /// The target texture (needs `COLOR_TARGET` usage).
    pub texture: TextureRef<'a>,
    /// Load operation.
    pub load: Load,
    /// Cycle the texture (SDL picks a backing store the GPU is not using). Only for
    /// textures whose old contents are not loaded.
    pub cycle: bool,
}

impl<'a> ColorTarget<'a> {
    /// A target with `load` that is not cycled.
    pub fn new(texture: TextureRef<'a>, load: Load) -> Self {
        Self {
            texture,
            load,
            cycle: false,
        }
    }

    /// The same target, cycled or not.
    #[must_use]
    pub fn cycled(mut self, cycle: bool) -> Self {
        self.cycle = cycle;
        self
    }
}

/// A command buffer: acquired and submitted by the host, lent to modules as
/// `&mut CommandBuffer` for recording. Dropping an acquired, unsubmitted command buffer
/// cancels it.
///
/// The host owns the command buffer and submits it (ADR 0002). A module receives
/// `&mut CommandBuffer` and opens passes on it. Each pass borrows the command buffer mutably
/// and ends when dropped, so a pass cannot outlive its command buffer, two passes are never
/// open at once, and the buffer cannot be submitted while a pass is open:
///
/// ```compile_fail
/// # use knorke_gpu::{CommandBuffer, Device};
/// # let device = Device::create(knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
/// let pass;
/// {
///     let mut cmd = CommandBuffer::acquire(&device).unwrap();
///     pass = cmd.copy_pass().unwrap();
/// } // `cmd` dropped here while `pass` borrows it
/// drop(pass);
/// ```
///
/// ```compile_fail
/// # use knorke_gpu::{CommandBuffer, Device};
/// # let device = Device::create(knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
/// let mut cmd = CommandBuffer::acquire(&device).unwrap();
/// let pass = cmd.copy_pass().unwrap();
/// cmd.submit().unwrap(); // the copy pass is still open
/// drop(pass);
/// ```
///
/// ```compile_fail
/// # use knorke_gpu::{CommandBuffer, Device};
/// # let device = Device::create(knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV, false).unwrap();
/// let mut cmd = CommandBuffer::acquire(&device).unwrap();
/// let first = cmd.copy_pass().unwrap();
/// let second = cmd.copy_pass().unwrap(); // two passes at once
/// drop((first, second));
/// ```
///
/// A module cannot submit a borrowed command buffer, since `submit` takes it by value:
///
/// ```compile_fail
/// # use knorke_gpu::CommandBuffer;
/// fn module_record(cmd: &mut CommandBuffer<'_>) {
///     cmd.submit().unwrap();
/// }
/// ```
///
/// Resources are bound through borrowed views ([`TextureRef`], [`SamplerRef`],
/// [`BufferRef`], [`PipelineRef`]) and checked against the command buffer's device, their
/// usage flags, the pass's target formats and the shaders' reflection. A draw fails with
/// [`RECORD_NOT_BOUND`] unless a pipeline and every resource its shaders declare are bound.
/// SDL keeps every bound resource alive until the command buffer has executed, so a view
/// only needs to be alive for the call that binds it.
#[derive(Debug)]
pub struct CommandBuffer<'d> {
    raw: NonNull<SDL_GPUCommandBuffer>,
    device: &'d Device,
    owned: bool,
}

impl<'d> CommandBuffer<'d> {
    /// Acquires a command buffer on `device` (host side).
    pub fn acquire(device: &'d Device) -> Result<Self, Diagnostic> {
        // SAFETY: the device is live for 'd.
        let raw = unsafe { SDL_AcquireGPUCommandBuffer(device.as_ptr()) };
        let raw = NonNull::new(raw).ok_or_else(|| {
            Diagnostic::error(
                RECORD_FAILED,
                format!("acquiring a command buffer failed: {}", sdl_error()),
            )
        })?;
        Ok(Self {
            raw,
            device,
            owned: true,
        })
    }

    /// Wraps a command buffer the host acquired with its own SDL calls. Dropping the wrapper
    /// leaves the buffer to the host; [`submit`](Self::submit) submits it.
    ///
    /// # Safety
    ///
    /// `raw` must be an acquired, unsubmitted command buffer of `device` with no pass open,
    /// and the host must not use it through `raw` while the wrapper exists.
    pub unsafe fn from_raw(device: &'d Device, raw: NonNull<SDL_GPUCommandBuffer>) -> Self {
        Self {
            raw,
            device,
            owned: false,
        }
    }

    /// The device.
    pub fn device(&self) -> &'d Device {
        self.device
    }

    /// The raw command buffer, for the host's own FFI calls.
    pub fn as_ptr(&self) -> *mut SDL_GPUCommandBuffer {
        self.raw.as_ptr()
    }

    fn same_device(&self, device: *mut SDL_GPUDevice, what: &str) -> Result<(), Diagnostic> {
        if device == self.device.as_ptr() {
            Ok(())
        } else {
            Err(wrong_device(what))
        }
    }

    /// Begins a render pass into 1 to [`MAX_COLOR_TARGETS`] color targets (no depth). The
    /// pass ends when dropped.
    pub fn render_pass(
        &mut self,
        targets: &[ColorTarget<'_>],
    ) -> Result<RenderPass<'_>, Diagnostic> {
        if targets.is_empty() || targets.len() > MAX_COLOR_TARGETS {
            return Err(out_of_range(format!(
                "a render pass needs 1 to {MAX_COLOR_TARGETS} color targets, got {}",
                targets.len()
            )));
        }
        let mut infos = [SDL_GPUColorTargetInfo::default(); MAX_COLOR_TARGETS];
        let mut formats = [SDL_GPU_TEXTUREFORMAT_INVALID; MAX_COLOR_TARGETS];
        for (i, t) in targets.iter().enumerate() {
            self.same_device(t.texture.device_ptr(), "a color target")?;
            if !has_usage(t.texture.desc().usage, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET) {
                return Err(Diagnostic::error(
                    RECORD_USAGE,
                    format!("color target {i} lacks COLOR_TARGET usage"),
                ));
            }
            let (load_op, clear) = match t.load {
                Load::Load => (SDL_GPU_LOADOP_LOAD, [0.0; 4]),
                Load::Clear(c) => (SDL_GPU_LOADOP_CLEAR, c),
                Load::DontCare => (SDL_GPU_LOADOP_DONT_CARE, [0.0; 4]),
            };
            infos[i] = SDL_GPUColorTargetInfo {
                texture: t.texture.as_ptr(),
                clear_color: SDL_FColor {
                    r: clear[0],
                    g: clear[1],
                    b: clear[2],
                    a: clear[3],
                },
                load_op,
                store_op: SDL_GPU_STOREOP_STORE,
                cycle: t.cycle,
                ..Default::default()
            };
            formats[i] = t.texture.format();
        }
        // SAFETY: `self` is an acquired, unsubmitted command buffer without an open pass
        // (every pass borrows `self` mutably); the targets are live textures of this device
        // with COLOR_TARGET usage (checked above); `infos` is alive for the call.
        let raw = unsafe {
            SDL_BeginGPURenderPass(
                self.raw.as_ptr(),
                infos.as_ptr(),
                targets.len() as u32,
                core::ptr::null(),
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| {
            Diagnostic::error(
                RECORD_FAILED,
                format!("beginning a render pass failed: {}", sdl_error()),
            )
        })?;
        Ok(RenderPass {
            raw,
            cmd: self.raw,
            device: self.device.as_ptr(),
            formats,
            targets: targets.len(),
            pipeline: None,
            bound: [StageMask::default(); 2],
            _cmd: PhantomData,
        })
    }

    /// Begins a copy pass. The pass ends when dropped.
    pub fn copy_pass(&mut self) -> Result<CopyPass<'_>, Diagnostic> {
        // SAFETY: as in `render_pass`: acquired, unsubmitted, no pass open.
        let raw = unsafe { SDL_BeginGPUCopyPass(self.raw.as_ptr()) };
        let raw = NonNull::new(raw).ok_or_else(|| {
            Diagnostic::error(
                RECORD_FAILED,
                format!("beginning a copy pass failed: {}", sdl_error()),
            )
        })?;
        Ok(CopyPass {
            raw,
            device: self.device.as_ptr(),
            _cmd: PhantomData,
        })
    }

    /// Submits the command buffer (host side).
    pub fn submit(mut self) -> Result<(), Diagnostic> {
        self.owned = false;
        // SAFETY: acquired, unsubmitted, no pass open (passes borrow `self`); consumed here,
        // and `owned = false` keeps drop from cancelling it.
        if unsafe { SDL_SubmitGPUCommandBuffer(self.raw.as_ptr()) } {
            Ok(())
        } else {
            Err(Diagnostic::error(
                RECORD_FAILED,
                format!("submitting the command buffer failed: {}", sdl_error()),
            ))
        }
    }

    /// Submits the command buffer and blocks until the GPU has executed it (host side, e.g.
    /// before a [`DownloadBuffer::read`]).
    pub fn submit_and_wait(mut self) -> Result<(), Diagnostic> {
        self.owned = false;
        let device = self.device.as_ptr();
        // SAFETY: as in `submit`; the fence is waited on and released once.
        unsafe {
            let fence = SDL_SubmitGPUCommandBufferAndAcquireFence(self.raw.as_ptr());
            if fence.is_null() {
                return Err(Diagnostic::error(
                    RECORD_FAILED,
                    format!("submitting the command buffer failed: {}", sdl_error()),
                ));
            }
            let waited = SDL_WaitForGPUFences(device, true, &fence, 1);
            SDL_ReleaseGPUFence(device, fence);
            if waited {
                Ok(())
            } else {
                Err(Diagnostic::error(
                    RECORD_FAILED,
                    format!("waiting for the command buffer failed: {}", sdl_error()),
                ))
            }
        }
    }
}

impl Drop for CommandBuffer<'_> {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: we acquired the buffer and did not submit it; no pass is open.
            unsafe { SDL_CancelGPUCommandBuffer(self.raw.as_ptr()) };
        }
    }
}

/// Bit masks of the slots bound in one stage.
#[derive(Clone, Copy, Debug, Default)]
struct StageMask {
    samplers: u32,
    storage_textures: u32,
    storage_buffers: u32,
    uniforms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Vertex,
    Fragment,
}

impl Stage {
    fn index(self) -> usize {
        match self {
            Self::Vertex => 0,
            Self::Fragment => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
        }
    }
}

fn slot_mask(first: u32, count: usize, max: usize, what: &str) -> Result<u32, Diagnostic> {
    let end = first as usize + count;
    if end > max {
        return Err(out_of_range(format!(
            "{what} slots {first}..{end} exceed the limit of {max}"
        )));
    }
    let ones = if count == 0 {
        0
    } else {
        u32::MAX >> (32 - count)
    };
    Ok(ones << first)
}

fn needed(count: u16) -> u32 {
    if count == 0 {
        0
    } else {
        u32::MAX >> (32 - u32::from(count).min(32))
    }
}

/// An open render pass; ends when dropped. Borrows the command buffer mutably.
#[derive(Debug)]
pub struct RenderPass<'p> {
    raw: NonNull<SDL_GPURenderPass>,
    cmd: NonNull<SDL_GPUCommandBuffer>,
    device: *mut SDL_GPUDevice,
    formats: [SDL_GPUTextureFormat; MAX_COLOR_TARGETS],
    targets: usize,
    pipeline: Option<(Reflection, Reflection)>,
    bound: [StageMask; 2],
    _cmd: PhantomData<&'p mut SDL_GPUCommandBuffer>,
}

impl RenderPass<'_> {
    fn same_device(&self, device: *mut SDL_GPUDevice, what: &str) -> Result<(), Diagnostic> {
        if device == self.device {
            Ok(())
        } else {
            Err(wrong_device(what))
        }
    }

    /// Binds a pipeline whose target formats match this pass's color targets.
    pub fn bind_pipeline(&mut self, pipeline: PipelineRef<'_>) -> Result<(), Diagnostic> {
        self.same_device(pipeline.device.as_ptr(), "the pipeline")?;
        if pipeline.target_formats() != &self.formats[..self.targets] {
            return Err(Diagnostic::error(
                RECORD_TARGET_MISMATCH,
                format!(
                    "the pipeline renders into formats {:?}, the pass has {:?}",
                    pipeline.target_formats(),
                    &self.formats[..self.targets]
                ),
            ));
        }
        // SAFETY: the pass is open (we hold it); the pipeline is live for the call and of
        // this device.
        unsafe { SDL_BindGPUGraphicsPipeline(self.raw.as_ptr(), pipeline.raw.as_ptr()) };
        self.pipeline = Some(pipeline.reflection);
        Ok(())
    }

    fn bind_samplers(
        &mut self,
        stage: Stage,
        first_slot: u32,
        bindings: &[SamplerBinding<'_>],
    ) -> Result<(), Diagnostic> {
        let mask = slot_mask(first_slot, bindings.len(), MAX_SAMPLERS, "sampler")?;
        let mut raw = [SDL_GPUTextureSamplerBinding::default(); MAX_SAMPLERS];
        for (i, b) in bindings.iter().enumerate() {
            self.same_device(b.texture.device_ptr(), "a sampled texture")?;
            self.same_device(b.sampler.device_ptr(), "a sampler")?;
            if !has_usage(b.texture.desc().usage, SDL_GPU_TEXTUREUSAGE_SAMPLER) {
                return Err(Diagnostic::error(
                    RECORD_USAGE,
                    format!(
                        "{} sampler slot {} binds a texture without SAMPLER usage",
                        stage.name(),
                        first_slot as usize + i
                    ),
                ));
            }
            raw[i] = SDL_GPUTextureSamplerBinding {
                texture: b.texture.as_ptr(),
                sampler: b.sampler.as_ptr(),
            };
        }
        if bindings.is_empty() {
            return Ok(());
        }
        let n = bindings.len() as u32;
        // SAFETY: the pass is open; textures and samplers are live objects of this device
        // with the needed usage (checked above); `raw` holds `n` initialised entries.
        unsafe {
            match stage {
                Stage::Vertex => {
                    SDL_BindGPUVertexSamplers(self.raw.as_ptr(), first_slot, raw.as_ptr(), n)
                }
                Stage::Fragment => {
                    SDL_BindGPUFragmentSamplers(self.raw.as_ptr(), first_slot, raw.as_ptr(), n)
                }
            }
        }
        self.bound[stage.index()].samplers |= mask;
        Ok(())
    }

    fn bind_storage_textures(
        &mut self,
        stage: Stage,
        first_slot: u32,
        textures: &[TextureRef<'_>],
    ) -> Result<(), Diagnostic> {
        let mask = slot_mask(
            first_slot,
            textures.len(),
            MAX_STORAGE_TEXTURES,
            "storage texture",
        )?;
        let mut raw: [*mut SDL_GPUTexture; MAX_STORAGE_TEXTURES] =
            [core::ptr::null_mut(); MAX_STORAGE_TEXTURES];
        for (i, t) in textures.iter().enumerate() {
            self.same_device(t.device_ptr(), "a storage texture")?;
            if !has_usage(t.desc().usage, SDL_GPU_TEXTUREUSAGE_GRAPHICS_STORAGE_READ) {
                return Err(Diagnostic::error(
                    RECORD_USAGE,
                    format!(
                        "{} storage texture slot {} lacks GRAPHICS_STORAGE_READ usage",
                        stage.name(),
                        first_slot as usize + i
                    ),
                ));
            }
            raw[i] = t.as_ptr();
        }
        if textures.is_empty() {
            return Ok(());
        }
        let n = textures.len() as u32;
        // SAFETY: as in `bind_samplers`.
        unsafe {
            match stage {
                Stage::Vertex => {
                    SDL_BindGPUVertexStorageTextures(self.raw.as_ptr(), first_slot, raw.as_ptr(), n)
                }
                Stage::Fragment => SDL_BindGPUFragmentStorageTextures(
                    self.raw.as_ptr(),
                    first_slot,
                    raw.as_ptr(),
                    n,
                ),
            }
        }
        self.bound[stage.index()].storage_textures |= mask;
        Ok(())
    }

    fn bind_storage_buffers(
        &mut self,
        stage: Stage,
        first_slot: u32,
        buffers: &[BufferRef<'_>],
    ) -> Result<(), Diagnostic> {
        let mask = slot_mask(
            first_slot,
            buffers.len(),
            MAX_STORAGE_BUFFERS,
            "storage buffer",
        )?;
        let mut raw: [*mut SDL_GPUBuffer; MAX_STORAGE_BUFFERS] =
            [core::ptr::null_mut(); MAX_STORAGE_BUFFERS];
        for (i, b) in buffers.iter().enumerate() {
            self.same_device(b.device_ptr(), "a storage buffer")?;
            let flag = SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ;
            if b.usage().0 & flag.0 != flag.0 {
                return Err(Diagnostic::error(
                    RECORD_USAGE,
                    format!(
                        "{} storage buffer slot {} lacks GRAPHICS_STORAGE_READ usage",
                        stage.name(),
                        first_slot as usize + i
                    ),
                ));
            }
            raw[i] = b.as_ptr();
        }
        if buffers.is_empty() {
            return Ok(());
        }
        let n = buffers.len() as u32;
        // SAFETY: as in `bind_samplers`.
        unsafe {
            match stage {
                Stage::Vertex => {
                    SDL_BindGPUVertexStorageBuffers(self.raw.as_ptr(), first_slot, raw.as_ptr(), n)
                }
                Stage::Fragment => SDL_BindGPUFragmentStorageBuffers(
                    self.raw.as_ptr(),
                    first_slot,
                    raw.as_ptr(),
                    n,
                ),
            }
        }
        self.bound[stage.index()].storage_buffers |= mask;
        Ok(())
    }

    fn push_uniform(&mut self, stage: Stage, slot: u32, data: &[u8]) -> Result<(), Diagnostic> {
        let mask = slot_mask(slot, 1, MAX_UNIFORMS, "uniform")?;
        let len = u32::try_from(data.len())
            .map_err(|_| out_of_range(format!("uniform block of {} bytes", data.len())))?;
        // SAFETY: the command buffer is live and unsubmitted (the pass borrows it); `data`
        // is valid for `len` bytes during the call, SDL copies it.
        unsafe {
            match stage {
                Stage::Vertex => {
                    SDL_PushGPUVertexUniformData(self.cmd.as_ptr(), slot, data.as_ptr().cast(), len)
                }
                Stage::Fragment => SDL_PushGPUFragmentUniformData(
                    self.cmd.as_ptr(),
                    slot,
                    data.as_ptr().cast(),
                    len,
                ),
            }
        }
        self.bound[stage.index()].uniforms |= mask;
        Ok(())
    }

    /// Binds texture-sampler pairs to the vertex stage from `first_slot` on.
    pub fn bind_vertex_samplers(
        &mut self,
        first_slot: u32,
        bindings: &[SamplerBinding<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_samplers(Stage::Vertex, first_slot, bindings)
    }

    /// Binds texture-sampler pairs to the fragment stage from `first_slot` on.
    pub fn bind_fragment_samplers(
        &mut self,
        first_slot: u32,
        bindings: &[SamplerBinding<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_samplers(Stage::Fragment, first_slot, bindings)
    }

    /// Binds read-only storage textures to the vertex stage.
    pub fn bind_vertex_storage_textures(
        &mut self,
        first_slot: u32,
        textures: &[TextureRef<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_storage_textures(Stage::Vertex, first_slot, textures)
    }

    /// Binds read-only storage textures to the fragment stage.
    pub fn bind_fragment_storage_textures(
        &mut self,
        first_slot: u32,
        textures: &[TextureRef<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_storage_textures(Stage::Fragment, first_slot, textures)
    }

    /// Binds read-only storage buffers to the vertex stage.
    pub fn bind_vertex_storage_buffers(
        &mut self,
        first_slot: u32,
        buffers: &[BufferRef<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_storage_buffers(Stage::Vertex, first_slot, buffers)
    }

    /// Binds read-only storage buffers to the fragment stage.
    pub fn bind_fragment_storage_buffers(
        &mut self,
        first_slot: u32,
        buffers: &[BufferRef<'_>],
    ) -> Result<(), Diagnostic> {
        self.bind_storage_buffers(Stage::Fragment, first_slot, buffers)
    }

    /// Pushes a uniform block (any `Pod` value, laid out as the shader declares it) to a
    /// vertex uniform slot.
    pub fn push_vertex_uniform<T: Pod>(&mut self, slot: u32, value: &T) -> Result<(), Diagnostic> {
        self.push_uniform(Stage::Vertex, slot, bytemuck::bytes_of(value))
    }

    /// Pushes a uniform block to a fragment uniform slot.
    pub fn push_fragment_uniform<T: Pod>(
        &mut self,
        slot: u32,
        value: &T,
    ) -> Result<(), Diagnostic> {
        self.push_uniform(Stage::Fragment, slot, bytemuck::bytes_of(value))
    }

    /// Binds `pipeline` and every resource of `b` in slot order, after
    /// [`check_bindings`](crate::check_bindings) against the pipeline's reflection. Nothing
    /// is bound if a check fails.
    pub fn bind_program(
        &mut self,
        pipeline: PipelineRef<'_>,
        b: &Bindings<'_>,
    ) -> Result<(), Diagnostic> {
        check_bindings(pipeline.reflection, b)?;
        self.bind_pipeline(pipeline)?;
        for (stage, sb) in [(Stage::Vertex, &b.vertex), (Stage::Fragment, &b.fragment)] {
            self.bind_samplers(stage, 0, sb.samplers)?;
            self.bind_storage_textures(stage, 0, sb.storage_textures)?;
            self.bind_storage_buffers(stage, 0, sb.storage_buffers)?;
            for (slot, u) in sb.uniforms.iter().enumerate() {
                self.push_uniform(stage, slot as u32, u)?;
            }
        }
        Ok(())
    }

    /// Sets the viewport.
    pub fn set_viewport(&mut self, viewport: &SDL_GPUViewport) {
        // SAFETY: the pass is open; `viewport` is valid for the call.
        unsafe { sdl3_sys::gpu::SDL_SetGPUViewport(self.raw.as_ptr(), viewport) };
    }

    /// Sets the scissor rectangle.
    pub fn set_scissor(&mut self, scissor: &SDL_Rect) {
        // SAFETY: the pass is open; `scissor` is valid for the call.
        unsafe { sdl3_sys::gpu::SDL_SetGPUScissor(self.raw.as_ptr(), scissor) };
    }

    fn check_draw(&self) -> Result<(), Diagnostic> {
        let Some((vertex, fragment)) = self.pipeline else {
            return Err(Diagnostic::error(
                RECORD_NOT_BOUND,
                "draw without a bound pipeline",
            ));
        };
        for (stage, r) in [(Stage::Vertex, vertex), (Stage::Fragment, fragment)] {
            let m = self.bound[stage.index()];
            for (what, want, got) in [
                ("samplers", needed(r.samplers), m.samplers),
                (
                    "storage textures",
                    needed(r.storage_textures),
                    m.storage_textures,
                ),
                (
                    "storage buffers",
                    needed(r.storage_buffers),
                    m.storage_buffers,
                ),
                ("uniform buffers", needed(r.uniform_buffers), m.uniforms),
            ] {
                if want & got != want {
                    return Err(Diagnostic::error(
                        RECORD_NOT_BOUND,
                        format!(
                            "draw with unbound {} {what} (declared mask {want:#b}, bound {got:#b})",
                            stage.name()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Draws `vertices` x `instances` without vertex buffers (vertex shaders generate
    /// positions from the vertex and instance index).
    pub fn draw(
        &mut self,
        vertices: u32,
        instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) -> Result<(), Diagnostic> {
        self.check_draw()?;
        // SAFETY: the pass is open with a bound pipeline and every declared resource.
        unsafe {
            SDL_DrawGPUPrimitives(
                self.raw.as_ptr(),
                vertices,
                instances,
                first_vertex,
                first_instance,
            )
        };
        Ok(())
    }

    /// One fullscreen triangle (3 vertices, `fullscreen.vert`).
    pub fn draw_fullscreen(&mut self) -> Result<(), Diagnostic> {
        self.draw(3, 1, 0, 0)
    }

    /// `instances` instances of `vertices` vertices each.
    pub fn draw_instanced(&mut self, vertices: u32, instances: u32) -> Result<(), Diagnostic> {
        self.draw(vertices, instances, 0, 0)
    }
}

impl Drop for RenderPass<'_> {
    fn drop(&mut self) {
        // SAFETY: we began the pass and end it once.
        unsafe { SDL_EndGPURenderPass(self.raw.as_ptr()) };
    }
}

/// A rectangle of texels of a 2D texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureRegion {
    /// First column.
    pub x: u32,
    /// First row.
    pub y: u32,
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

/// An open copy pass; ends when dropped. Borrows the command buffer mutably.
#[derive(Debug)]
pub struct CopyPass<'p> {
    raw: NonNull<SDL_GPUCopyPass>,
    device: *mut SDL_GPUDevice,
    _cmd: PhantomData<&'p mut SDL_GPUCommandBuffer>,
}

impl CopyPass<'_> {
    fn same_device(&self, device: *mut SDL_GPUDevice, what: &str) -> Result<(), Diagnostic> {
        if device == self.device {
            Ok(())
        } else {
            Err(wrong_device(what))
        }
    }

    /// Copies the first `bytes` of `src`'s upload buffer into its GPU buffer (cycling the
    /// GPU buffer). Fails before `reserve`, without a GPU buffer or beyond the capacity.
    pub fn upload_to_buffer(
        &mut self,
        src: &UploadBuffer<'_>,
        bytes: u32,
    ) -> Result<(), Diagnostic> {
        self.same_device(src.device_ptr(), "the upload buffer")?;
        let (Some(buffer), Some(transfer)) = (src.buffer(), src.transfer()) else {
            return Err(out_of_range(
                "uploading from an unreserved or transfer-only buffer".to_owned(),
            ));
        };
        if bytes > src.capacity() {
            return Err(out_of_range(format!(
                "uploading {bytes} bytes from a {} byte buffer",
                src.capacity()
            )));
        }
        let from = SDL_GPUTransferBufferLocation {
            transfer_buffer: transfer,
            offset: 0,
        };
        let to = SDL_GPUBufferRegion {
            buffer,
            offset: 0,
            size: bytes,
        };
        // SAFETY: the pass is open; both buffers are live objects of this device (`src` is
        // borrowed, so it is neither mapped nor released) and the region lies within them.
        unsafe { SDL_UploadToGPUBuffer(self.raw.as_ptr(), &from, &to, true) };
        Ok(())
    }

    /// Copies `src`'s upload buffer (tightly packed texels) into the whole of `dst`.
    pub fn upload_to_texture(
        &mut self,
        src: &UploadBuffer<'_>,
        dst: TextureRef<'_>,
        cycle: bool,
    ) -> Result<(), Diagnostic> {
        self.same_device(src.device_ptr(), "the upload buffer")?;
        self.same_device(dst.device_ptr(), "the destination texture")?;
        let Some(transfer) = src.transfer() else {
            return Err(out_of_range(
                "uploading from an unreserved buffer".to_owned(),
            ));
        };
        let need = dst.desc().bytes();
        if need > u64::from(src.capacity()) {
            return Err(out_of_range(format!(
                "the texture needs {need} bytes, the upload buffer holds {}",
                src.capacity()
            )));
        }
        let from = SDL_GPUTextureTransferInfo {
            transfer_buffer: transfer,
            ..Default::default()
        };
        let to = SDL_GPUTextureRegion {
            texture: dst.as_ptr(),
            w: dst.width(),
            h: dst.height(),
            d: 1,
            ..Default::default()
        };
        // SAFETY: the pass is open; buffer and texture are live objects of this device and
        // the transfer buffer holds the whole texture (checked above).
        unsafe { SDL_UploadToGPUTexture(self.raw.as_ptr(), &from, &to, cycle) };
        Ok(())
    }

    /// Copies the texels of `region` from `src`'s upload buffer, tightly packed from byte
    /// `offset` on (row after row, `region.width` texels per row), into the same rectangle of
    /// `dst`. `cycle` drops the other texels of `dst`, so it is only for a region that covers
    /// the whole texture. Some backends want `offset` aligned (D3D12: 512 bytes for textures).
    /// Fails before recording if the region leaves the texture or the staged bytes leave the
    /// buffer.
    pub fn upload_to_texture_region(
        &mut self,
        src: &UploadBuffer<'_>,
        offset: u32,
        dst: TextureRef<'_>,
        region: TextureRegion,
        cycle: bool,
    ) -> Result<(), Diagnostic> {
        self.same_device(src.device_ptr(), "the upload buffer")?;
        self.same_device(dst.device_ptr(), "the destination texture")?;
        let Some(transfer) = src.transfer() else {
            return Err(out_of_range(
                "uploading from an unreserved buffer".to_owned(),
            ));
        };
        let fits = |start: u32, size: u32, limit: u32| {
            size > 0 && start.checked_add(size).is_some_and(|end| end <= limit)
        };
        if !fits(region.x, region.width, dst.width())
            || !fits(region.y, region.height, dst.height())
        {
            return Err(out_of_range(format!(
                "the region {}x{} at ({}, {}) leaves the {}x{} texture",
                region.width,
                region.height,
                region.x,
                region.y,
                dst.width(),
                dst.height()
            )));
        }
        let need =
            TextureDesc::new(region.width, region.height, dst.format(), dst.desc().usage).bytes();
        if u64::from(offset) + need > u64::from(src.capacity()) {
            return Err(out_of_range(format!(
                "the region needs {need} bytes from offset {offset}, the upload buffer holds {}",
                src.capacity()
            )));
        }
        let from = SDL_GPUTextureTransferInfo {
            transfer_buffer: transfer,
            offset,
            pixels_per_row: region.width,
            rows_per_layer: region.height,
        };
        let to = SDL_GPUTextureRegion {
            texture: dst.as_ptr(),
            x: region.x,
            y: region.y,
            w: region.width,
            h: region.height,
            d: 1,
            ..Default::default()
        };
        // SAFETY: as in `upload_to_texture`; the transfer buffer holds the region from
        // `offset` on (checked above) and the region lies within the texture.
        unsafe { SDL_UploadToGPUTexture(self.raw.as_ptr(), &from, &to, cycle) };
        Ok(())
    }

    /// Copies the `size[0]` x `size[1]` texels at `src_at` of `src` to `dst_at` of `dst`, on
    /// the GPU. Both textures must have the same format and be different textures. `cycle`
    /// drops the other texels of `dst`, so it is only for a copy that, together with later
    /// writes, covers the whole texture. Fails before recording if a rectangle leaves its
    /// texture or the formats differ.
    pub fn copy_texture_region(
        &mut self,
        src: TextureRef<'_>,
        src_at: [u32; 2],
        dst: TextureRef<'_>,
        dst_at: [u32; 2],
        size: [u32; 2],
        cycle: bool,
    ) -> Result<(), Diagnostic> {
        self.same_device(src.device_ptr(), "the source texture")?;
        self.same_device(dst.device_ptr(), "the destination texture")?;
        if src.format() != dst.format() || src.as_ptr() == dst.as_ptr() {
            return Err(Diagnostic::error(
                RECORD_USAGE,
                "a texture copy needs two different textures of the same format",
            ));
        }
        let fits = |start: u32, size: u32, limit: u32| {
            size > 0 && start.checked_add(size).is_some_and(|end| end <= limit)
        };
        for (what, at, t) in [("source", src_at, &src), ("destination", dst_at, &dst)] {
            if !fits(at[0], size[0], t.width()) || !fits(at[1], size[1], t.height()) {
                return Err(out_of_range(format!(
                    "the {what} region {}x{} at ({}, {}) leaves the {}x{} texture",
                    size[0],
                    size[1],
                    at[0],
                    at[1],
                    t.width(),
                    t.height()
                )));
            }
        }
        let from = SDL_GPUTextureLocation {
            texture: src.as_ptr(),
            x: src_at[0],
            y: src_at[1],
            ..Default::default()
        };
        let to = SDL_GPUTextureLocation {
            texture: dst.as_ptr(),
            x: dst_at[0],
            y: dst_at[1],
            ..Default::default()
        };
        // SAFETY: the pass is open; both textures are live objects of this device with the
        // same format, and both rectangles lie within their textures (checked above).
        unsafe {
            SDL_CopyGPUTextureToTexture(self.raw.as_ptr(), &from, &to, size[0], size[1], 1, cycle)
        };
        Ok(())
    }

    /// Copies the whole of `src` (tightly packed) into `dst` (host side readback).
    pub fn download_texture(
        &mut self,
        src: TextureRef<'_>,
        dst: &DownloadBuffer<'_>,
    ) -> Result<(), Diagnostic> {
        self.same_device(src.device_ptr(), "the source texture")?;
        self.same_device(dst.device_ptr(), "the download buffer")?;
        let need = src.desc().bytes();
        if need > u64::from(dst.size()) {
            return Err(out_of_range(format!(
                "the texture needs {need} bytes, the download buffer holds {}",
                dst.size()
            )));
        }
        let from = SDL_GPUTextureRegion {
            texture: src.as_ptr(),
            w: src.width(),
            h: src.height(),
            d: 1,
            ..Default::default()
        };
        let to = SDL_GPUTextureTransferInfo {
            transfer_buffer: dst.raw(),
            ..Default::default()
        };
        // SAFETY: as in `upload_to_texture`.
        unsafe { SDL_DownloadFromGPUTexture(self.raw.as_ptr(), &from, &to) };
        Ok(())
    }
}

impl Drop for CopyPass<'_> {
    fn drop(&mut self) {
        // SAFETY: we began the pass and end it once.
        unsafe { SDL_EndGPUCopyPass(self.raw.as_ptr()) };
    }
}
