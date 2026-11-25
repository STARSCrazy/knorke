// Origin: chiaro @ 9bc5b9b, src/integration/MediumDraw.cpp (DrawMediums, DrawMediumsGpu: one pass
// on the medium layer, the flat or the shaped pipeline per draw, blend MAX on RGB and ADD on A).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Medium clouds on the GPU (P30): [`MediumDraw`]s of `knorke-light2d` drawn into the medium
//! layer of the [`Layers`] with Chiaro's `medium_draw.*` shaders.
//!
//! # Redesign against Chiaro
//!
//! - Chiaro acquired and submitted a command buffer of its own per call; here the batch
//!   records into the host's (ADR 0002).
//! - Only GPU layers: there are no renderer layers. Chiaro required the layer to be cleared
//!   this frame first; here the host orders its passes ([`Layers::clear_pass`] before
//!   [`MediumBatch::record`]), as for the surface batch.

use knorke_gpu::{
    Bindings, Blend, CommandBuffer, Filter, Gpu, PipelineCache, ProgramDesc, SamplerBinding,
    Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_light2d::medium_draw::MediumDraw;
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::layers::{LAYER_FORMAT, Layer, Layers};

/// [`MediumBatch::record`] without a successful [`MediumBatch::prepare`] of the same draws.
pub const MEDIUM_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.integration.medium.not_prepared");

/// One cloud and its shape: a texture with `SAMPLER` usage whose alpha thins the cloud
/// (`None`: the full rectangle at constant thickness). The shape is sampled linearly; its
/// colour is ignored.
#[derive(Clone, Copy, Debug)]
pub struct MediumItem<'a> {
    /// The cloud.
    pub draw: MediumDraw,
    /// Its shape.
    pub shape: Option<TextureRef<'a>>,
}

const FLAT: usize = 0;
const SHAPED: usize = 1;

/// Draws [`MediumItem`]s into the medium layer. Per frame [`prepare`](Self::prepare) (validates,
/// may fail), then [`record`](Self::record) into the host's command buffer. Steady state: no
/// allocation.
#[derive(Debug)]
pub struct MediumBatch<'d> {
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    /// Number of draws of the last successful prepare.
    prepared: Option<usize>,
}

impl<'d> MediumBatch<'d> {
    /// Resolves the medium shaders in `shaders`; fails if the pack lacks one.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        let program = |fragment| ProgramDesc::new("medium_draw.vert", fragment, Blend::MaxRgbAddA);
        Ok(Self {
            pipelines: PipelineCache::new(
                shaders,
                &[
                    program("medium_draw_flat.frag"),
                    program("medium_draw.frag"),
                ],
            )?,
            samplers: Samplers::new(gpu)?,
            prepared: None,
        })
    }

    /// Validates every draw (one bad entry rejects the batch, as in Chiaro) and creates the
    /// pipelines the draws need.
    ///
    /// # Errors
    /// [`MEDIUM_DRAW_INVALID`](knorke_light2d::medium_draw::MEDIUM_DRAW_INVALID) naming
    /// `draws[i].<field>`, or a pipeline could not be created. Nothing is drawn then.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        items: &[MediumItem<'_>],
    ) -> Result<(), Diagnostic> {
        self.prepared = None;
        for (i, item) in items.iter().enumerate() {
            item.draw.validate(Some(i))?;
        }
        for (k, any) in [
            (FLAT, items.iter().any(|d| d.shape.is_none())),
            (SHAPED, items.iter().any(|d| d.shape.is_some())),
        ] {
            if any {
                self.pipelines.get(shaders, k, LAYER_FORMAT)?;
            }
        }
        self.prepared = Some(items.len());
        Ok(())
    }

    /// Records the prepared `items` into `cmd`: one pass on the medium layer that keeps its
    /// content, one draw of 6 vertices per item in their order. `height_step` is the light
    /// settings' `resolved_medium_height_step`. No items record nothing.
    ///
    /// # Errors
    /// Not prepared for these items ([`MEDIUM_NOT_PREPARED`]), the layers have no medium layer
    /// ([`LAYER_TARGETS`](crate::layers::LAYER_TARGETS)), or a pass or binding failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        layers: &mut Layers<'_>,
        height_step: f32,
        items: &[MediumItem<'_>],
    ) -> Result<(), Diagnostic> {
        if self.prepared.take() != Some(items.len()) {
            return Err(Diagnostic::error(
                MEDIUM_NOT_PREPARED,
                "the medium batch was not prepared for these draws",
            ));
        }
        if items.is_empty() {
            return Ok(());
        }
        let size = layers
            .texture(Layer::Medium)
            .map_or((0, 0), |t| (t.width(), t.height()));
        let linear = self.samplers.sampler(Filter::Linear);
        let mut pass = layers.draw_pass(cmd, Layer::Medium)?;
        for item in items {
            let k = if item.shape.is_some() { SHAPED } else { FLAT };
            let pipeline = self.pipelines.pipeline(k, LAYER_FORMAT).ok_or_else(|| {
                Diagnostic::error(MEDIUM_NOT_PREPARED, "a medium pipeline is missing")
            })?;
            let params = item.draw.params(height_step, item.shape.is_some(), size);
            let bytes = params.to_bytes();
            let shape = item.shape.map(|texture| SamplerBinding {
                texture,
                sampler: linear,
            });
            pass.bind_program(
                pipeline,
                &Bindings {
                    vertex: StageBindings {
                        uniforms: &[&bytes],
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: shape.as_slice(),
                        ..Default::default()
                    },
                },
            )?;
            pass.draw(6, 1, 0, 0)?;
        }
        Ok(())
    }
}
