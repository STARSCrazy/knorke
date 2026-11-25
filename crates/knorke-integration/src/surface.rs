// Origin: chiaro @ 9bc5b9b, src/integration/Layers.cpp (DrawSurface, FlushSurface: one unblended
// draw of the queued surface geometry into the surface layer, in call order).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Surface geometry on the GPU (P35): a [`SurfaceGeometry`] of `knorke-light2d` (boxes, walls,
//! floors, quads) drawn into the surface layer of the [`Layers`].
//!
//! # Redesign against Chiaro
//!
//! - Chiaro drew the helpers with `SDL_Renderer` into renderer layers only and left GPU layers
//!   to the application (`CHI_BuildSurfaceGeometry`). Here the batch draws on the GPU with
//!   Knorke's own shaders (`shaders/src/surface_geometry.*.hlsl`), into the host's command
//!   buffer (ADR 0002).
//! - No Begin/End queue: the host fills one [`SurfaceGeometry`] in painter's order, and the
//!   batch draws it as one draw. What else writes the surface layer (standing sprites) comes
//!   after it in the order the host records them.

use knorke_gpu::sys::gpu::SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ;
use knorke_gpu::{
    Bindings, Blend, CommandBuffer, Gpu, PipelineCache, ProgramDesc, ShaderLibrary, StageBindings,
    UploadBuffer,
};
use knorke_light2d::surface_geometry::SurfaceGeometry;
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::layers::{LAYER_FORMAT, Layer, Layers};

/// [`SurfaceBatch::record`] without a successful [`SurfaceBatch::prepare`] of the same geometry.
pub const SURFACE_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.integration.surface.not_prepared");

/// Bytes of one vertex in the storage buffer (`struct Vertex` of `surface_geometry.vert.hlsl`).
const VERTEX_BYTES: usize = 16;

/// Draws a [`SurfaceGeometry`] into the surface layer. Per frame [`prepare`](Self::prepare)
/// (may fail), then [`record`](Self::record) into the host's command buffer. Steady state: no
/// allocation.
#[derive(Debug)]
pub struct SurfaceBatch<'d> {
    pipelines: PipelineCache<'d>,
    vertices: UploadBuffer<'d>,
    /// Number of vertices of the last successful prepare.
    prepared: Option<usize>,
}

impl<'d> SurfaceBatch<'d> {
    /// Resolves the surface shaders in `shaders`; fails if the pack lacks one.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        let program = ProgramDesc::new(
            "surface_geometry.vert",
            "surface_geometry.frag",
            Blend::Opaque,
        );
        Ok(Self {
            pipelines: PipelineCache::new(shaders, &[program])?,
            vertices: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                (1024 * VERTEX_BYTES) as u32,
                "surface vertices",
            ),
            prepared: None,
        })
    }

    /// Fills the vertex buffer from `geometry` and creates the pipeline.
    ///
    /// # Errors
    /// The buffer or the pipeline could not be created. Nothing is drawn then.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        geometry: &SurfaceGeometry,
    ) -> Result<(), Diagnostic> {
        self.prepared = None;
        self.pipelines.get(shaders, 0, LAYER_FORMAT)?;
        let vertices = geometry.vertices();
        if !vertices.is_empty() {
            self.vertices
                .reserve((vertices.len() * VERTEX_BYTES) as u64)?;
            let mut mapped = self.vertices.map()?;
            let out = mapped.bytes().as_chunks_mut::<VERTEX_BYTES>().0;
            for (bytes, v) in out.iter_mut().zip(vertices) {
                let p = v.position.raw();
                bytes[0..4].copy_from_slice(&p.x.to_le_bytes());
                bytes[4..8].copy_from_slice(&p.y.to_le_bytes());
                bytes[8..12].copy_from_slice(&v.pixel);
                bytes[12..].fill(0);
            }
        }
        self.prepared = Some(vertices.len());
        Ok(())
    }

    /// Records the prepared `geometry` into `cmd`: the vertex upload, then one render pass on
    /// the surface layer with one unblended draw, triangles in their order. Empty geometry
    /// records nothing and leaves the layer undrawn.
    ///
    /// # Errors
    /// Not prepared for this geometry ([`SURFACE_NOT_PREPARED`]), the layers have no surface
    /// layer ([`LAYER_TARGETS`](crate::layers::LAYER_TARGETS)), or a pass or binding failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        layers: &mut Layers<'_>,
        geometry: &SurfaceGeometry,
    ) -> Result<(), Diagnostic> {
        let count = geometry.vertices().len();
        if self.prepared.take() != Some(count) {
            return Err(Diagnostic::error(
                SURFACE_NOT_PREPARED,
                "the surface batch was not prepared for this geometry",
            ));
        }
        if count == 0 {
            return Ok(());
        }
        let missing =
            |what: &str| Diagnostic::error(SURFACE_NOT_PREPARED, format!("{what} is missing"));
        let target = layers.texture(Layer::Surface);
        let size = target.map(|t| [t.width() as f32, t.height() as f32, 0.0, 0.0]);
        cmd.copy_pass()?
            .upload_to_buffer(&self.vertices, (count * VERTEX_BYTES) as u32)?;
        let vertices = self
            .vertices
            .buffer_ref()
            .ok_or_else(|| missing("the vertex buffer"))?;
        let pipeline = self
            .pipelines
            .pipeline(0, LAYER_FORMAT)
            .ok_or_else(|| missing("the surface pipeline"))?;
        let mut pass = layers.draw_pass(cmd, Layer::Surface)?;
        let mut params = [0u8; 16];
        for (bytes, f) in params
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(size.unwrap_or_default())
        {
            *bytes = f.to_le_bytes();
        }
        pass.bind_program(
            pipeline,
            &Bindings {
                vertex: StageBindings {
                    storage_buffers: &[vertices],
                    uniforms: &[&params],
                    ..Default::default()
                },
                fragment: StageBindings::default(),
            },
        )?;
        pass.draw(count as u32, 1, 0, 0)
    }
}
