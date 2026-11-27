// Origin: chiaro @ 9bc5b9b, src/rendering/PatternOccluders.hpp and PatternOccluders.cpp (Prepare,
// Record), src/rendering/PatternAtlas.hpp and PatternAtlas.cpp (the texture and record buffer),
// src/gpu/PipelineCache.cpp (OccluderPatternId, TilesPatternId, kPatternIdFormat).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    SDL_GPU_TEXTUREFORMAT_R8_UNORM, SDL_GPU_TEXTUREFORMAT_R16_UNORM, SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, BufferRef, ColorTarget, CommandBuffer, Gpu, Load, PipelineCache, ProgramDesc,
    ProgramIndex, RenderTarget, SamplerBinding, SamplerRef, ShaderLibrary, StageBindings,
    TextureRef, UploadBuffer,
};
use knorke_math::bytemuck;
use knorke_types::Diagnostic;

use super::{
    GpuPattern, GpuPatternObstacle, PatternParams, PatternSet, TilePatternIdParams,
    fill_pattern_atlas, pattern_params, pattern_records,
};
use crate::ObstacleStore;
use crate::atlas::{AtlasSource, AtlasTexture};
use crate::occluder_raster::{
    DomainParams, NOT_PREPARED, TileOccluders, triangle_buffer_floats, write_triangle_buffer,
};
use crate::pattern_store::{ATLAS_SIZE, PatternStore};
use crate::projection::Projection;
use crate::shadow::{MaskAnchor, OcclusionDomain};

/// Format of the id texture: `1 + record index` per mask texel as `id / 65535`. Not blended:
/// Vulkan does not guarantee blending for `R16_UNORM` (Chiaro `pattern_id.frag.hlsl`), so in
/// the fixed draw order the last writer wins, shapes first, then tile grids.
pub const PATTERN_ID_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16_UNORM;

const fn id_program(vertex: &'static str, fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        vertex,
        fragment,
        blend: Blend::Opaque,
        second: None,
        more: &[],
        primitive: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    }
}

/// The id passes, to be placed in the host's [`PipelineCache`] table at
/// [`PatternPrograms::at`]: shapes (`occluder.vert`) and tile grids (`tile.vert`).
pub const PATTERN_PROGRAMS: [ProgramDesc; 2] = [
    id_program("occluder.vert", "pattern_id.frag"),
    id_program("tile.vert", "tile_pattern_id.frag"),
];

/// Where [`PATTERN_PROGRAMS`] sit in a pipeline table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternPrograms {
    /// Shape triangles into the id texture.
    pub shapes: ProgramIndex,
    /// Tile grids into the id texture.
    pub tiles: ProgramIndex,
}

impl PatternPrograms {
    /// The two programs at `base..base + 2`.
    pub const fn at(base: ProgramIndex) -> Self {
        Self {
            shapes: base,
            tiles: base + 1,
        }
    }
}

impl AtlasSource for PatternStore {
    const FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8_UNORM;
    const WIDTH: u32 = ATLAS_SIZE;
    const BLOCK_BYTES: u32 = ATLAS_SIZE * 16;
    const NAME: &'static str = "pattern atlas";

    fn rows(&self) -> u32 {
        self.atlas_rows().0
    }
    fn version(&self) -> u64 {
        PatternStore::version(self)
    }
    fn bytes(rows: u32) -> u64 {
        u64::from(ATLAS_SIZE) * u64::from(rows)
    }
    fn fill(&self, rows: u32, out: &mut [u8]) {
        fill_pattern_atlas(self, rows, out);
    }
}

/// What the `*_pattern` light programs bind (Chiaro `PatternBinding`).
#[derive(Clone, Copy, Debug)]
pub struct PatternMaps<'a> {
    /// The id texture ([`PATTERN_ID_FORMAT`], mask size).
    pub ids: TextureRef<'a>,
    /// The R8 atlas: alpha and column means.
    pub atlas: TextureRef<'a>,
    /// [`GpuPatternObstacle`] per pattern obstacle of the frame.
    pub obstacles: BufferRef<'a>,
    /// [`GpuPattern`] per live pattern.
    pub records: BufferRef<'a>,
    /// `PatternData` words.
    pub data: BufferRef<'a>,
    /// The `PatternParams` block.
    pub params: [u8; PatternParams::SIZE],
    /// The frame's pattern obstacles in record order.
    pub members: &'a [(crate::obstacle::ObstacleId, u32)],
}

fn storage<'d>(gpu: Gpu<'d>, bytes: u32, name: &'static str) -> UploadBuffer<'d> {
    UploadBuffer::new(gpu, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, bytes, name)
}

fn upload<T: bytemuck::Pod>(buffer: &mut UploadBuffer<'_>, items: &[T]) -> Result<(), Diagnostic> {
    let bytes: &[u8] = bytemuck::cast_slice(items);
    buffer.reserve(bytes.len() as u64)?;
    buffer.map()?.bytes()[..bytes.len()].copy_from_slice(bytes);
    Ok(())
}

fn upload_blocks<const N: usize, T>(
    buffer: &mut UploadBuffer<'_>,
    items: &[T],
    bytes_of: impl Fn(&T) -> [u8; N],
) -> Result<(), Diagnostic> {
    buffer.reserve((items.len() * N) as u64)?;
    let mut mapped = buffer.map()?;
    for (chunk, item) in mapped.bytes().as_chunks_mut::<N>().0.iter_mut().zip(items) {
        chunk.copy_from_slice(&bytes_of(item));
    }
    Ok(())
}

/// Pattern obstacles (Chiaro `PatternOccluders` with its `PatternAtlas`): collects the pattern
/// obstacles in the mask's domain (only in frames that draw the mask; a reused mask keeps the
/// list its id texture was drawn with), prepares the atlas, the pattern records, the obstacle
/// records, the pattern data and the id texture. Without a pattern obstacle in the domain
/// everything is released: no pass, no binding, nothing held (ADR 0011: off costs nothing).
///
/// Per frame: [`prepare`](Self::prepare) after the obstacle stages, [`record`](Self::record)
/// after the mask, then [`maps`](Self::maps) for the light pass.
#[derive(Debug)]
pub struct PatternOccluders<'d> {
    programs: PatternPrograms,
    atlas: AtlasTexture<'d, PatternStore>,
    pattern_records: Vec<GpuPattern>,
    index: Vec<(u32, u32)>,
    records_version: Option<u64>,
    record_buffer: UploadBuffer<'d>,
    set: PatternSet,
    /// What `set.words` was built for: members, cell version, pattern store version.
    data_members: Vec<(crate::obstacle::ObstacleId, u32)>,
    data_key: Option<(u64, u64)>,
    uploaded_records: Vec<GpuPatternObstacle>,
    uploaded_ids: (
        Vec<knorke_math::At<knorke_math::World2, knorke_math::Vec2>>,
        Vec<f32>,
    ),
    obstacle_buffer: UploadBuffer<'d>,
    data_buffer: UploadBuffer<'d>,
    id_shapes: UploadBuffer<'d>,
    id_floats: u32,
    ids: RenderTarget<'d>,
    params: PatternParams,
    upload_records: bool,
    upload_patterns: bool,
    upload_data: bool,
    upload_shapes: bool,
    redraw_ids: bool,
    active: bool,
}

impl<'d> PatternOccluders<'d> {
    /// A stage without resources (the first frame with a pattern obstacle creates them).
    pub fn new(gpu: Gpu<'d>, programs: PatternPrograms) -> Self {
        Self {
            programs,
            atlas: AtlasTexture::new(gpu),
            pattern_records: Vec::new(),
            index: Vec::new(),
            records_version: None,
            record_buffer: storage(gpu, 64 * GpuPattern::SIZE as u32, "pattern"),
            set: PatternSet::default(),
            data_members: Vec::new(),
            data_key: None,
            uploaded_records: Vec::new(),
            uploaded_ids: (Vec::new(), Vec::new()),
            obstacle_buffer: storage(
                gpu,
                16 * GpuPatternObstacle::SIZE as u32,
                "pattern obstacle",
            ),
            data_buffer: storage(gpu, 4096, "pattern data"),
            id_shapes: storage(gpu, 4096 * 4, "pattern id"),
            id_floats: 0,
            ids: RenderTarget::new(gpu),
            params: PatternParams::default(),
            upload_records: false,
            upload_patterns: false,
            upload_data: false,
            upload_shapes: false,
            redraw_ids: false,
            active: false,
        }
    }

    fn release(&mut self) {
        self.atlas.release();
        self.pattern_records.clear();
        self.index.clear();
        self.records_version = None;
        self.record_buffer.reset();
        self.set.members.clear();
        self.data_members.clear();
        self.data_key = None;
        self.uploaded_records.clear();
        self.uploaded_ids.0.clear();
        self.uploaded_ids.1.clear();
        self.obstacle_buffer.reset();
        self.data_buffer.reset();
        self.id_shapes.reset();
        self.id_floats = 0;
        self.ids.release();
        self.upload_records = false;
        self.upload_patterns = false;
        self.upload_data = false;
        self.upload_shapes = false;
        self.redraw_ids = false;
        self.active = false;
    }

    /// Prepares the frame. `mask_drawn`: the mask is drawn this frame (not reused), so the
    /// member list and the id texture are rebuilt. `min_radius` is the frame's
    /// [`thin_circle_radius`](crate::occluder_raster::thin_circle_radius). Returns the textures
    /// created; afterwards [`is_active`](Self::is_active) says whether the light pass binds the
    /// patterns.
    ///
    /// # Errors
    /// A texture, buffer or pipeline could not be created or mapped.
    #[allow(clippy::too_many_arguments)] // the inputs of Chiaro's Prepare, one per frame
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut PipelineCache<'d>,
        obstacles: &ObstacleStore,
        patterns: Option<&PatternStore>,
        projection: &Projection,
        domain: &OcclusionDomain,
        anchor: &MaskAnchor,
        min_radius: f32,
        mask_drawn: bool,
    ) -> Result<u32, Diagnostic> {
        let Some(store) = patterns else {
            self.release();
            return Ok(0);
        };
        if mask_drawn {
            self.set
                .collect(obstacles, min_radius, projection, domain, anchor);
        }
        if self.set.members.is_empty() {
            self.release();
            return Ok(0);
        }
        self.active = false;
        let mut created = self.atlas.prepare(store)?;
        if self
            .ids
            .ensure(domain.width, domain.height, PATTERN_ID_FORMAT)?
        {
            created += 1;
        }
        if self.records_version != Some(store.version()) {
            pattern_records(store, &mut self.pattern_records, &mut self.index);
            upload_blocks(&mut self.record_buffer, &self.pattern_records, |r| {
                r.to_bytes()
            })?;
            self.records_version = Some(store.version());
            self.upload_patterns = true;
        }

        // Pattern data: rebuilt when the members, a tile grid's cells or the pattern indices
        // change.
        let key = (obstacles.cell_version(), store.version());
        if self.data_key != Some(key) || self.data_members != self.set.members {
            self.data_key = None;
            self.set.build_data(obstacles, &self.index);
            upload(&mut self.data_buffer, &self.set.words)?;
            self.upload_data = true;
            self.data_members.clone_from(&self.set.members);
            self.data_key = Some(key);
        }

        // The records, rebuilt every frame (obstacles move), uploaded when they change. The
        // uploaded copies mirror the upload buffers, not the GPU: the flags stay set until
        // `record` copies them, so a failed frame leaves nothing stale.
        self.set.build_records(obstacles, &self.index);
        if self.uploaded_records != self.set.records {
            self.uploaded_records.clear();
            upload_blocks(&mut self.obstacle_buffer, &self.set.records, |r| {
                r.to_bytes()
            })?;
            self.upload_records = true;
            self.uploaded_records.clone_from(&self.set.records);
        }

        if mask_drawn {
            self.set.build_ids(obstacles, min_radius);
            if self.uploaded_ids.0 != self.set.id_vertices
                || self.uploaded_ids.1 != self.set.id_values
            {
                self.uploaded_ids.0.clear();
                self.uploaded_ids.1.clear();
                let floats =
                    triangle_buffer_floats(self.set.id_vertices.len(), self.set.id_values.len());
                self.id_floats = floats as u32;
                if floats > 0 {
                    self.id_shapes.reserve(floats as u64 * 4)?;
                    let mut mapped = self.id_shapes.map()?;
                    write_triangle_buffer(
                        &self.set.id_vertices,
                        &self.set.id_values,
                        &mut mapped.bytes()[..floats * 4],
                    );
                }
                self.upload_shapes = floats > 0;
                self.uploaded_ids.0.clone_from(&self.set.id_vertices);
                self.uploaded_ids.1.clone_from(&self.set.id_values);
            }
            if !self.set.id_vertices.is_empty() {
                pipelines.get(shaders, self.programs.shapes, PATTERN_ID_FORMAT)?;
            }
            if !self.set.tile_draws.is_empty() {
                pipelines.get(shaders, self.programs.tiles, PATTERN_ID_FORMAT)?;
            }
            self.redraw_ids = true;
        }
        self.params = pattern_params(
            projection,
            domain,
            anchor,
            (ATLAS_SIZE, self.atlas.height()),
        );
        self.active = true;
        Ok(created)
    }

    /// `true` after a prepare that found a pattern obstacle in the domain.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Records the pending uploads and, in a frame that draws the mask, the id texture.
    /// `nearest` is the host's nearest sampler, `tiles` the prepared tile grids, `domain` the
    /// frame's [`DomainParams`].
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        tiles: &TileOccluders<'_>,
        domain: &DomainParams,
    ) -> Result<(), Diagnostic> {
        if !self.active {
            return Ok(());
        }
        self.atlas.record(cmd)?;
        if self.upload_records || self.upload_patterns || self.upload_data || self.upload_shapes {
            let mut copy = cmd.copy_pass()?;
            let jobs = [
                (
                    &mut self.upload_patterns,
                    &self.record_buffer,
                    self.pattern_records.len() * GpuPattern::SIZE,
                ),
                (
                    &mut self.upload_records,
                    &self.obstacle_buffer,
                    self.uploaded_records.len() * GpuPatternObstacle::SIZE,
                ),
                (
                    &mut self.upload_data,
                    &self.data_buffer,
                    self.set.words.len() * 4,
                ),
                (
                    &mut self.upload_shapes,
                    &self.id_shapes,
                    self.id_floats as usize * 4,
                ),
            ];
            for (pending, buffer, bytes) in jobs {
                if *pending {
                    copy.upload_to_buffer(buffer, bytes as u32)?;
                    *pending = false;
                }
            }
        }
        if !self.redraw_ids {
            return Ok(());
        }
        self.redraw_ids = false;
        let missing = || crate::missing_after_prepare(NOT_PREPARED, "a pattern resource");
        let target = self.ids.texture_ref().ok_or_else(missing)?;
        let mut pass =
            cmd.render_pass(&[ColorTarget::new(target, Load::Clear([0.0; 4])).cycled(true)])?;
        if !self.set.id_vertices.is_empty() {
            let pipeline = pipelines
                .pipeline(self.programs.shapes, PATTERN_ID_FORMAT)
                .ok_or_else(missing)?;
            let vertices = self.set.id_vertices.len() as u32;
            let params = domain.with_opacity_start(vertices).to_bytes();
            let buffers = [self.id_shapes.buffer_ref().ok_or_else(missing)?];
            let bindings = Bindings {
                vertex: StageBindings {
                    storage_buffers: &buffers,
                    uniforms: &[&params],
                    ..Default::default()
                },
                fragment: StageBindings::default(),
            };
            pass.bind_program(pipeline, &bindings)?;
            pass.draw_instanced(vertices, 1)?;
        }
        if !self.set.tile_draws.is_empty() {
            let pipeline = pipelines
                .pipeline(self.programs.tiles, PATTERN_ID_FORMAT)
                .ok_or_else(missing)?;
            let buffers = [
                self.obstacle_buffer.buffer_ref().ok_or_else(missing)?,
                self.data_buffer.buffer_ref().ok_or_else(missing)?,
            ];
            for &(handle, id) in &self.set.tile_draws {
                let Some((slot, texture)) = tiles
                    .find(handle)
                    .and_then(|s| s.texture.as_ref().map(|t| (s, t)))
                else {
                    continue;
                };
                let params = slot.uniforms.params(domain).to_bytes();
                let mut strip = [0.0f32; 4];
                strip.copy_from_slice(&slot.uniforms.heights[4..8]);
                let ids = TilePatternIdParams {
                    strip,
                    id: [id, 0, 0, 0],
                }
                .to_bytes();
                let samplers = [SamplerBinding::new(texture.texture_ref(), nearest)];
                let bindings = Bindings {
                    vertex: StageBindings {
                        uniforms: &[&params],
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &samplers,
                        storage_buffers: &buffers,
                        uniforms: &[&ids],
                        ..Default::default()
                    },
                };
                pass.bind_program(pipeline, &bindings)?;
                pass.draw_instanced(6, 1)?;
            }
        }
        Ok(())
    }

    /// What the light pass binds; `None` without a pattern obstacle in the domain.
    pub fn maps(&self) -> Option<PatternMaps<'_>> {
        if !self.active {
            return None;
        }
        Some(PatternMaps {
            ids: self.ids.texture_ref()?,
            atlas: self.atlas.texture()?,
            obstacles: self.obstacle_buffer.buffer_ref()?,
            records: self.record_buffer.buffer_ref()?,
            data: self.data_buffer.buffer_ref()?,
            params: self.params.to_bytes(),
            members: &self.set.members,
        })
    }
}
