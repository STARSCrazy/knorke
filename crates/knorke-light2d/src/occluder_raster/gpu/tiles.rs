// Origin: chiaro @ 9bc5b9b, src/rendering/TileOccluders.hpp and TileOccluders.cpp
// (TileOccluders: textures, staging, uploads, draws).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

use super::{MASK_FORMAT, NOT_PREPARED, OccluderPrograms, TILE_UPLOAD_LIMIT};
use crate::ObstacleStore;
use crate::obstacle::{ObstacleId, PatternSource};
use crate::occluder_raster::{CELL_BYTES, DomainParams, TileSet, TileSlot, write_cell_texels};
use knorke_gpu::staging::{MAX_GPU_BUFFER_BYTES, reserve_staging};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    Bindings, CopyPass, Gpu, PipelineCache, RenderPass, SamplerBinding, SamplerRef, ShaderLibrary,
    StageBindings, Texture, TextureDesc, TextureRegion, UploadBuffer,
};
use knorke_types::Diagnostic;

/// D3D12 wants texture upload offsets on 512-byte boundaries; other backends accept them too.
const TEXTURE_UPLOAD_ALIGNMENT: u32 = 512;
/// First size of the transfer buffer for cell uploads.
const INITIAL_TILE_BYTES: u32 = 64 * 1024;

/// The tile grids among the obstacles: one cell texture per active grid, kept across frames,
/// with the changed cells staged in prepare and copied while recording (a changed grid uploads
/// only the bounding box of its changed cells), and one quad per grid drawn into the mask.
/// Pending uploads stay pending until a copy pass records them, even if a frame fails.
///
/// Per frame: [`prepare`](Self::prepare) (may fail), then [`record_uploads`](Self::record_uploads)
/// in a copy pass, then [`record_draws`](Self::record_draws) in the render pass on the mask.
#[derive(Debug)]
pub struct TileOccluders<'d> {
    gpu: Gpu<'d>,
    programs: OccluderPrograms,
    tiles: TileSet<Texture<'d>>,
    transfer: UploadBuffer<'d>,
    upload_pending: bool,
    drawing: bool,
}

impl<'d> TileOccluders<'d> {
    /// A stage without textures yet.
    pub fn new(gpu: Gpu<'d>, programs: OccluderPrograms) -> Self {
        Self {
            gpu,
            programs,
            tiles: TileSet::new(),
            transfer: UploadBuffer::new(
                gpu,
                SDL_GPUBufferUsageFlags(0),
                INITIAL_TILE_BYTES,
                "tile cell",
            ),
            upload_pending: false,
            drawing: false,
        }
    }

    /// Brings the textures in line with the active grids, stages the changed cells and creates
    /// the draw pipeline (a frame that draws the mask).
    ///
    /// # Errors
    /// Creating a texture or the transfer buffer or mapping it failed, the pending uploads are
    /// larger than one transfer buffer can be, or the pipeline could not be created. Pending
    /// cells stay pending.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut PipelineCache<'d>,
        obstacles: &ObstacleStore,
        patterns: &impl PatternSource,
    ) -> Result<(), Diagnostic> {
        self.drawing = false;
        self.prepare_textures(obstacles, patterns)?;
        if !self.tiles.is_empty() {
            pipelines.get(shaders, self.programs.tiles, MASK_FORMAT)?;
            self.drawing = true;
        }
        Ok(())
    }

    fn prepare_textures(
        &mut self,
        obstacles: &ObstacleStore,
        patterns: &impl PatternSource,
    ) -> Result<(), Diagnostic> {
        let gpu = self.gpu;
        self.tiles.sync(obstacles, |columns, rows| {
            let desc = TextureDesc::new(
                columns,
                rows,
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                SDL_GPU_TEXTUREUSAGE_SAMPLER,
            );
            Texture::new(gpu, desc)
        })?;

        // Every pending upload is (re)written: a failed frame leaves uploads pending, and
        // mapping cycles the buffer. Sums in u64: enough pending 4096x4096 grids exceed what
        // one transfer buffer can hold.
        let mut bytes = 0u64;
        self.upload_pending = false;
        for slot in self.tiles.slots_mut() {
            let Some(r) = slot.pending else { continue };
            let size = r.area() as u64 * CELL_BYTES as u64;
            let Some(offset) = reserve_staging(
                &mut bytes,
                size,
                TEXTURE_UPLOAD_ALIGNMENT,
                MAX_GPU_BUFFER_BYTES,
            ) else {
                return Err(Diagnostic::error(
                    TILE_UPLOAD_LIMIT,
                    format!(
                        "the pending tile uploads exceed the GPU transfer buffer limit of \
                         {MAX_GPU_BUFFER_BYTES} bytes; create fewer or smaller tile grids per frame"
                    ),
                ));
            };
            slot.staging_offset = offset;
        }
        if bytes == 0 {
            return Ok(());
        }
        self.transfer.reserve(bytes)?;
        {
            let mut mapped = self.transfer.map()?;
            let out = mapped.bytes();
            for slot in self.tiles.slots() {
                if let Some(r) = slot.pending {
                    let start = slot.staging_offset as usize;
                    let end = start + r.area() * CELL_BYTES;
                    write_cell_texels(obstacles, slot.handle, &r, patterns, &mut out[start..end]);
                }
            }
        }
        self.upload_pending = true;
        Ok(())
    }

    /// Something to upload.
    pub fn upload_pending(&self) -> bool {
        self.upload_pending
    }

    /// Number of grids with a texture.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// `true` without grids.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The slot (and so the texture) of the grid `handle`, `None` if it is not an active grid.
    pub fn find(&self, handle: ObstacleId) -> Option<&TileSlot<Texture<'d>>> {
        self.tiles.find(handle)
    }

    /// Records the staged cell uploads into `copy`. Returns the number of uploads (each counts
    /// as an `occluder_upload` in Chiaro).
    ///
    /// # Errors
    /// The texture or the transfer buffer is missing or belongs to another device.
    pub fn record_uploads(&mut self, copy: &mut CopyPass<'_>) -> Result<u32, Diagnostic> {
        let mut uploads = 0;
        for slot in self.tiles.slots_mut() {
            let (Some(r), Some(texture)) = (slot.pending, slot.texture.as_ref()) else {
                continue;
            };
            let region = TextureRegion {
                x: r.x0,
                y: r.y0,
                width: r.width(),
                height: r.height(),
            };
            // Cycling would drop the cells outside a partial upload.
            let whole = region.width == slot.columns && region.height == slot.rows;
            copy.upload_to_texture_region(
                &self.transfer,
                slot.staging_offset,
                texture.texture_ref(),
                region,
                whole,
            )?;
            slot.pending = None;
            uploads += 1;
        }
        self.upload_pending = false;
        Ok(uploads)
    }

    /// Draws one quad per grid into `pass`, whose targets are the mask and the height texture.
    /// `nearest` is the host's nearest-neighbour sampler. Returns the number of draws.
    ///
    /// # Errors
    /// Not prepared, the pass does not match the program's targets, or a binding is wrong.
    pub fn record_draws(
        &self,
        pass: &mut RenderPass<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        domain: &DomainParams,
    ) -> Result<u32, Diagnostic> {
        if !self.drawing {
            return Ok(0);
        }
        let pipeline = pipelines
            .pipeline(self.programs.tiles, MASK_FORMAT)
            .ok_or_else(|| {
                Diagnostic::error(NOT_PREPARED, "the tile pipeline is missing after prepare")
            })?;
        let mut draws = 0;
        for slot in self.tiles.slots() {
            let Some(texture) = slot.texture.as_ref() else {
                continue;
            };
            let params = slot.uniforms.params(domain).to_bytes();
            let heights = slot.uniforms.heights_bytes();
            let samplers = [SamplerBinding::new(texture.texture_ref(), nearest)];
            let bindings = Bindings {
                vertex: StageBindings {
                    uniforms: &[&params],
                    ..Default::default()
                },
                fragment: StageBindings {
                    samplers: &samplers,
                    uniforms: &[&heights],
                    ..Default::default()
                },
            };
            pass.bind_program(pipeline, &bindings)?;
            pass.draw_instanced(6, 1)?;
            draws += 1;
        }
        Ok(draws)
    }
}
