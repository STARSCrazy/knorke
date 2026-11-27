// Origin: chiaro @ 9bc5b9b, src/rendering/OccluderGeometry.hpp and OccluderGeometry.cpp
// (TriangleBuffer, OccluderGeometry).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

use super::{MASK_FORMAT, NOT_PREPARED, OccluderPrograms};
use crate::ObstacleStore;
use crate::obstacle::PatternSource;
use crate::occluder_raster::{
    DomainParams, TriangleSet, triangle_buffer_floats, write_triangle_buffer,
};
use knorke_gpu::sys::gpu::SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ;
use knorke_gpu::{
    Bindings, BufferRef, CopyPass, Gpu, PipelineCache, RenderPass, ShaderLibrary, StageBindings,
    UploadBuffer,
};
use knorke_math::{bytemuck, det};
use knorke_types::{Diagnostic, Revision};

/// First size of a triangle buffer in bytes (4096 floats).
const INITIAL_BYTES: u32 = 4096 * 4;

/// A storage buffer of triangle vertices (or their height intervals) plus its upload buffer.
#[derive(Debug)]
struct TriangleBuffer<'d> {
    upload: UploadBuffer<'d>,
    /// Valid floats (filled in prepare).
    floats: u32,
    /// Triangle buffers: vertices; the opacity per triangle follows them.
    vertices: u32,
}

impl<'d> TriangleBuffer<'d> {
    fn new(gpu: Gpu<'d>, name: &'static str) -> Self {
        Self {
            upload: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                INITIAL_BYTES,
                name,
            ),
            floats: 0,
            vertices: 0,
        }
    }

    /// Fills the buffer with the vertices of `set`, followed by their opacities. Mapping can
    /// fail, so this belongs in prepare.
    fn fill_triangles(&mut self, set: &TriangleSet) -> Result<(), Diagnostic> {
        self.floats = 0;
        self.vertices = 0;
        if set.is_empty() {
            return Ok(());
        }
        let floats = triangle_buffer_floats(set.vertices().len(), set.opacities().len());
        let bytes = floats * 4;
        self.upload.reserve(bytes as u64)?;
        let mut mapped = self.upload.map()?;
        write_triangle_buffer(
            set.vertices(),
            set.opacities(),
            &mut mapped.bytes()[..bytes],
        );
        self.floats = floats as u32;
        self.vertices = set.vertices().len() as u32;
        Ok(())
    }

    /// Fills the buffer with one `[f32; 4]` per triangle.
    fn fill_intervals(&mut self, intervals: &[[f32; 4]]) -> Result<(), Diagnostic> {
        self.floats = 0;
        self.vertices = 0;
        if intervals.is_empty() {
            return Ok(());
        }
        let bytes = intervals.len() * 16;
        self.upload.reserve(bytes as u64)?;
        let mut mapped = self.upload.map()?;
        mapped.bytes()[..bytes].copy_from_slice(bytemuck::cast_slice(intervals));
        self.floats = (bytes / 4) as u32;
        Ok(())
    }

    fn record_upload(&self, copy: &mut CopyPass<'_>) -> Result<(), Diagnostic> {
        copy.upload_to_buffer(&self.upload, self.floats * 4)
    }

    fn buffer(&self) -> Result<BufferRef<'_>, Diagnostic> {
        self.upload.buffer_ref().ok_or_else(|| {
            Diagnostic::error(NOT_PREPARED, "a triangle buffer is missing after prepare")
        })
    }
}

/// What the static triangles were built for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StaticBuild {
    version: Revision,
    min_radius: f32,
}

/// The triangle geometry of the obstacles that are not tile grids: world triangles with
/// opacities and height intervals in storage buffers. The static set is rebuilt and
/// re-uploaded only when the static obstacles or a static circle's widening changed; the
/// dynamic set is rebuilt and uploaded in every frame that draws the mask and has a dynamic
/// obstacle.
///
/// Per frame: [`prepare`](Self::prepare) (may fail), then [`record_uploads`](Self::record_uploads)
/// in a copy pass, then [`record_draws`](Self::record_draws) in the render pass on the mask.
#[derive(Debug)]
pub struct OccluderGeometry<'d> {
    programs: OccluderPrograms,
    statics: TriangleBuffer<'d>,
    dynamics: TriangleBuffer<'d>,
    static_intervals: TriangleBuffer<'d>,
    dynamic_intervals: TriangleBuffer<'d>,
    static_set: TriangleSet,
    dynamic_set: TriangleSet,
    built: Option<StaticBuild>,
    static_thin: u32,
    dynamic_thin: u32,
    static_upload_pending: bool,
    static_interval_upload_pending: bool,
    drawing: bool,
}

impl<'d> OccluderGeometry<'d> {
    /// A stage without buffers yet (they are created by the first prepare that needs them).
    pub fn new(gpu: Gpu<'d>, programs: OccluderPrograms) -> Self {
        Self {
            programs,
            statics: TriangleBuffer::new(gpu, "static occluder"),
            dynamics: TriangleBuffer::new(gpu, "dynamic occluder"),
            static_intervals: TriangleBuffer::new(gpu, "static occluder interval"),
            dynamic_intervals: TriangleBuffer::new(gpu, "dynamic occluder interval"),
            static_set: TriangleSet::new(),
            dynamic_set: TriangleSet::new(),
            built: None,
            static_thin: 0,
            dynamic_thin: 0,
            static_upload_pending: false,
            static_interval_upload_pending: false,
            drawing: false,
        }
    }

    /// Rebuilds what changed (a frame that draws the mask; `min_radius` is
    /// [`thin_circle_radius`](crate::occluder_raster::thin_circle_radius)) and creates the draw
    /// pipeline if there is anything to draw.
    ///
    /// # Errors
    /// Creating or mapping a buffer, or the pipeline, failed. The static geometry is then
    /// rebuilt by the next call.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut PipelineCache<'d>,
        obstacles: &ObstacleStore,
        patterns: &impl PatternSource,
        min_radius: f32,
    ) -> Result<(), Diagnostic> {
        // A changed texel size matters to the static triangles only if a static circle is below
        // the old or the new limit.
        let thin_changed = self.built.is_some_and(|b| {
            b.min_radius != min_radius
                && self.static_set.smallest_circle() < det::max(min_radius, b.min_radius)
        });
        let version = obstacles.static_version();
        if self.built.is_none_or(|b| b.version != version) || thin_changed {
            self.built = None;
            self.static_set
                .collect(obstacles, patterns, false, min_radius);
            self.static_thin = self.static_set.thin();
            self.statics.fill_triangles(&self.static_set)?;
            self.static_intervals
                .fill_intervals(self.static_set.intervals())?;
            self.built = Some(StaticBuild {
                version,
                min_radius,
            });
            self.static_upload_pending = self.statics.floats > 0;
            self.static_interval_upload_pending = self.static_intervals.floats > 0;
        }

        self.dynamics.floats = 0;
        self.dynamic_intervals.floats = 0;
        self.dynamic_thin = 0;
        if obstacles.any_dynamic_enabled() {
            self.dynamic_set
                .collect(obstacles, patterns, true, min_radius);
            self.dynamic_thin = self.dynamic_set.thin();
            self.dynamics.fill_triangles(&self.dynamic_set)?;
            self.dynamic_intervals
                .fill_intervals(self.dynamic_set.intervals())?;
        }

        self.drawing = self.statics.floats > 0 || self.dynamics.floats > 0;
        if self.drawing {
            pipelines.get(shaders, self.programs.occluder, MASK_FORMAT)?;
        }
        Ok(())
    }

    /// Widened circles in the current static and dynamic sets (unchanged while the mask is
    /// reused).
    pub fn thin_count(&self) -> u32 {
        self.static_thin + self.dynamic_thin
    }

    /// Something to upload: a pending static upload or a dynamic set.
    pub fn upload_pending(&self) -> bool {
        self.static_upload_pending
            || self.static_interval_upload_pending
            || self.dynamics.floats > 0
    }

    /// Records the pending uploads into `copy`. Returns the number of static triangle uploads
    /// (what Chiaro counts as `occluder_uploads`).
    ///
    /// # Errors
    /// A buffer is missing or belongs to another device.
    pub fn record_uploads(&mut self, copy: &mut CopyPass<'_>) -> Result<u32, Diagnostic> {
        let mut uploads = 0;
        if self.static_upload_pending {
            self.statics.record_upload(copy)?;
            self.static_upload_pending = false;
            uploads += 1;
        }
        if self.static_interval_upload_pending {
            self.static_intervals.record_upload(copy)?;
            self.static_interval_upload_pending = false;
        }
        for dynamic in [&self.dynamics, &self.dynamic_intervals] {
            if dynamic.floats > 0 {
                dynamic.record_upload(copy)?;
            }
        }
        Ok(uploads)
    }

    /// Draws the static and/or dynamic set into `pass`, whose targets are the mask and the
    /// height texture. Returns the number of draws.
    ///
    /// # Errors
    /// Not prepared, the pass does not match the program's targets, or a binding is wrong.
    pub fn record_draws(
        &self,
        pass: &mut RenderPass<'_>,
        pipelines: &PipelineCache<'_>,
        domain: &DomainParams,
        static_set: bool,
        dynamic_set: bool,
    ) -> Result<u32, Diagnostic> {
        if !self.drawing {
            return Ok(0);
        }
        let pipeline = pipelines
            .pipeline(self.programs.occluder, MASK_FORMAT)
            .ok_or_else(|| {
                Diagnostic::error(
                    NOT_PREPARED,
                    "the occluder pipeline is missing after prepare",
                )
            })?;
        let sets = [
            (&self.statics, &self.static_intervals, static_set),
            (&self.dynamics, &self.dynamic_intervals, dynamic_set),
        ];
        let mut draws = 0;
        for (triangles, intervals, wanted) in sets {
            if triangles.floats == 0 || !wanted {
                continue;
            }
            // The opacities follow the vertices in the same buffer; the pad says where.
            let params = domain.with_opacity_start(triangles.vertices).to_bytes();
            let buffers = [triangles.buffer()?, intervals.buffer()?];
            let bindings = Bindings {
                vertex: StageBindings {
                    storage_buffers: &buffers,
                    uniforms: &[&params],
                    ..Default::default()
                },
                fragment: StageBindings::default(),
            };
            pass.bind_program(pipeline, &bindings)?;
            pass.draw_instanced(triangles.vertices, 1)?;
            draws += 1;
        }
        Ok(draws)
    }
}
