// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/rendering/DebugCheckStage.hpp and src/rendering/DebugCheckStage.cpp (Prepare, Record).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.
//
// Altered: rewritten in Rust for the Knorke Engine on the safe recording API of `knorke-gpu`;
// the findings are a `WarningSet` returned by `record`, errors are `Diagnostic`s.

use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPUTextureFormat};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, DownloadBuffer, Filter, Gpu, Load, PipelineCache,
    ProgramDesc, RenderTarget, SamplerBinding, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{DebugCheckParams, SLOTS, findings_of};
use crate::frame_warnings::{DebugChecks, WarningSet};
use crate::settings::LightSettings;

/// Format of the 1x1 findings target: saturates at 1, "found".
pub const FINDINGS_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

/// One triangle per scene texel into the 1x1 target, ONE + ONE: a texel that fails a check
/// covers the target with its findings, every other one is degenerate.
pub const DEBUG_CHECK_PROGRAM: ProgramDesc =
    ProgramDesc::new("debug_check.vert", "debug_check.frag", Blend::Additive);

const NOT_PREPARED: DiagnosticCode = DiagnosticCode::new("knorke.light2d.debug_check.not_prepared");

/// What the check reads of a frame; every texture is borrowed and has the scene's size.
#[derive(Clone, Copy, Debug)]
pub struct DebugCheckFrame<'a> {
    /// Scene size in texels.
    pub size: (u32, u32),
    /// Surface buffer.
    pub surface: Option<TextureRef<'a>>,
    /// Material buffer.
    pub material: Option<TextureRef<'a>>,
    /// Medium layer.
    pub medium: Option<TextureRef<'a>>,
    /// Specular scale and wet ground, for the highlight peak.
    pub settings: &'a LightSettings,
}

impl<'a> DebugCheckFrame<'a> {
    fn any(&self) -> Option<TextureRef<'a>> {
        self.surface.or(self.material).or(self.medium)
    }
}

#[derive(Debug)]
struct Slot<'d> {
    target: RenderTarget<'d>,
    download: Option<DownloadBuffer<'d>>,
    /// A download was recorded and not read yet.
    pending: bool,
}

/// The GPU debug check (see the [module docs](super)). Per frame
/// [`prepare`](Self::prepare), then [`record`](Self::record), which returns the findings of
/// the frame recorded [`SLOTS`] records earlier. It reads the buffers only, so the image does
/// not depend on it.
///
/// The findings are late, never undefined: each download buffer starts zeroed and each
/// download writes all four bytes. If the host keeps [`SLOTS`] or more frames in flight, a
/// read returns zero or an older finding.
#[derive(Debug)]
pub struct DebugCheckStage<'d> {
    gpu: Gpu<'d>,
    pipelines: PipelineCache<'d>,
    slots: [Slot<'d>; SLOTS],
    next: usize,
    active: bool,
}

impl<'d> DebugCheckStage<'d> {
    /// Resolves the two shaders in `shaders`; fails if the pack lacks one. Creates nothing on
    /// the GPU.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        Ok(Self {
            gpu,
            pipelines: PipelineCache::new(shaders, &[DEBUG_CHECK_PROGRAM])?,
            slots: core::array::from_fn(|_| Slot {
                target: RenderTarget::new(gpu),
                download: None,
                pending: false,
            }),
            next: 0,
            active: false,
        })
    }

    /// Whether the last prepared frame runs the check.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Number of pipelines created so far (0 until the check first runs).
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// Decides whether the frame runs the check (`checks` has `LAYERS` and the frame a surface,
    /// material or medium buffer) and then creates what is missing. Returns the number of GPU
    /// objects created. A frame that does not run it drops the findings still on their way.
    ///
    /// # Errors
    /// The pipeline, a target or a download buffer could not be created.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        checks: DebugChecks,
        frame: &DebugCheckFrame<'_>,
    ) -> Result<u32, Diagnostic> {
        self.active = false;
        if !(checks.contains(DebugChecks::LAYERS) && frame.any().is_some()) {
            for slot in &mut self.slots {
                slot.pending = false;
            }
            return Ok(0);
        }
        self.pipelines.get(shaders, 0, FINDINGS_FORMAT)?;
        let mut allocations = 0;
        for slot in &mut self.slots {
            allocations += u32::from(slot.target.ensure(1, 1, FINDINGS_FORMAT)?);
            if slot.download.is_none() {
                let mut download = DownloadBuffer::new(self.gpu, 4)?;
                download.clear()?;
                slot.download = Some(download);
                allocations += 1;
            }
        }
        self.active = true;
        Ok(allocations)
    }

    /// Reads the findings of the frame recorded [`SLOTS`] records ago and records this frame's
    /// check into `cmd`: one render pass into a 1x1 target and its download. `frame` must be
    /// the one given to [`prepare`](Self::prepare). Empty if the frame does not run the check.
    ///
    /// # Errors
    /// The frame lost its buffers since `prepare`, or a pass, binding or mapping failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &DebugCheckFrame<'_>,
    ) -> Result<WarningSet, Diagnostic> {
        if !self.active {
            return Ok(WarningSet::EMPTY);
        }
        let missing = |what: &str| Diagnostic::error(NOT_PREPARED, format!("{what} is missing"));
        let any = frame.any().ok_or_else(|| missing("a buffer to check"))?;
        let pipeline = self
            .pipelines
            .pipeline(0, FINDINGS_FORMAT)
            .ok_or_else(|| missing("the debug check pipeline"))?;
        let slot = &mut self.slots[self.next];
        let download = slot
            .download
            .as_mut()
            .ok_or_else(|| missing("the download buffer"))?;
        let mut found = WarningSet::EMPTY;
        if slot.pending {
            let mut texel = [0u8; 4];
            download.read(&mut texel)?;
            found = findings_of(texel);
            slot.pending = false;
        }
        let target = slot
            .target
            .texture_ref()
            .ok_or_else(|| missing("the findings target"))?;

        // An unbound slot takes any bound texture; the shader skips it by the flags.
        let nearest = samplers.sampler(Filter::Nearest);
        let slots = [frame.surface, frame.material, frame.medium]
            .map(|t| SamplerBinding::new(t.unwrap_or(any), nearest));
        let params = DebugCheckParams::new(
            frame.size,
            frame.surface.is_some(),
            frame.material.is_some(),
            frame.medium.is_some(),
            frame.settings,
        )
        .to_bytes();
        let bindings = Bindings {
            vertex: StageBindings {
                samplers: &slots,
                uniforms: &[&params],
                ..Default::default()
            },
            fragment: StageBindings::default(),
        };
        {
            let targets = [ColorTarget::new(target, Load::Clear([0.0; 4]))];
            let mut pass = cmd.render_pass(&targets)?;
            pass.bind_program(pipeline, &bindings)?;
            pass.draw(3 * frame.size.0 * frame.size.1, 1, 0, 0)?;
        }
        cmd.copy_pass()?.download_texture(target, download)?;
        slot.pending = true;
        self.next = (self.next + 1) % SLOTS;
        Ok(found)
    }
}
