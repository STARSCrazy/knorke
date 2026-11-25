// Origin: chiaro @ 9bc5b9b, src/rendering/ShockwaveLightPass.hpp, src/rendering/ShockwaveLightPass.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The ring light pass on the GPU. It owns only its pipeline (ADR 0002: it records into the
//! host's command buffer and draws into the lighting stage's light buffer).

use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Load, PipelineCache, ProgramDesc,
    SamplerBinding, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{MAX_RING_LIGHTS, RingFragParams, RingLight, RingPassParams};
use crate::lighting::{LIGHT_FORMAT, LightingFrame, ambient_clear};

/// More than [`MAX_RING_LIGHTS`] ring lights in one frame.
pub const RING_LIGHTS_TOO_MANY: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.ring_light.too_many");

const PROGRAMS: [ProgramDesc; 1] = [ProgramDesc::new(
    "shockwave_light.vert",
    "shockwave_light.frag",
    Blend::Additive,
)];

/// The ring light pass (Chiaro `ShockwaveLightPass`).
#[derive(Debug, Default)]
pub struct RingLightPass<'d> {
    cache: Option<PipelineCache<'d>>,
    count: u32,
}

impl<'d> RingLightPass<'d> {
    /// Takes the pipeline when `rings` is not empty; an empty list draws nothing and creates
    /// nothing.
    ///
    /// # Errors
    /// More than [`MAX_RING_LIGHTS`] rings, or the pipeline could not be created.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        rings: &[RingLight],
    ) -> Result<(), Diagnostic> {
        self.count = 0;
        if rings.is_empty() {
            return Ok(());
        }
        if rings.len() > MAX_RING_LIGHTS {
            return Err(Diagnostic::error(
                RING_LIGHTS_TOO_MANY,
                format!(
                    "{} ring lights, at most {MAX_RING_LIGHTS} per frame",
                    rings.len()
                ),
            ));
        }
        if self.cache.is_none() {
            self.cache = Some(PipelineCache::new(shaders, &PROGRAMS)?);
        }
        if let Some(cache) = self.cache.as_mut() {
            cache.get(shaders, 0, LIGHT_FORMAT)?;
        }
        self.count = rings.len() as u32;
        Ok(())
    }

    /// Ring lights are drawn this frame.
    pub fn is_active(&self) -> bool {
        self.count > 0
    }

    /// Ring lights drawn this frame.
    pub fn ring_count(&self) -> u32 {
        self.count
    }

    /// The pass into `light`: loads it when `load` (zones or lights already wrote it this
    /// frame), else clears it to `ambient` (alpha 0).
    ///
    /// # Errors
    /// The pipeline is missing, or the pass or a binding failed.
    pub fn record_pass(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
        ambient: Vec3,
        light: TextureRef<'_>,
        load: bool,
    ) -> Result<(), Diagnostic> {
        if self.count == 0 {
            return Ok(());
        }
        let pipeline = self
            .cache
            .as_ref()
            .and_then(|c| c.pipeline(0, LIGHT_FORMAT))
            .ok_or_else(|| {
                Diagnostic::error(
                    crate::lighting::LIGHTING_INPUT_MISSING,
                    "the ring light pipeline is missing after prepare",
                )
            })?;
        let target = if load {
            ColorTarget::new(light, Load::Load)
        } else {
            ColorTarget::new(light, Load::Clear(ambient_clear(ambient))).cycled(true)
        };
        let mut pass = cmd.render_pass(&[target])?;
        let (w, h) = (frame.scene.width(), frame.scene.height());
        let vertex = RingPassParams::new(w, h, frame.ring_lights).to_bytes();
        let fragment = RingFragParams::new(
            frame.projection,
            w,
            h,
            frame.settings.surface_height_step,
            frame.surface.is_some(),
        )
        .to_bytes();
        // The scene stands in for a missing surface buffer: not read then.
        let samplers = [SamplerBinding::new(
            frame.surface.unwrap_or(frame.scene),
            samplers.sampler(Filter::Nearest),
        )];
        let vertex_uniforms: [&[u8]; 1] = [&vertex];
        let fragment_uniforms: [&[u8]; 1] = [&fragment];
        let bindings = Bindings {
            vertex: StageBindings {
                uniforms: &vertex_uniforms,
                ..Default::default()
            },
            fragment: StageBindings {
                samplers: &samplers,
                uniforms: &fragment_uniforms,
                ..Default::default()
            },
        };
        pass.bind_program(pipeline, &bindings)?;
        pass.draw_instanced(6, self.count)
    }
}
