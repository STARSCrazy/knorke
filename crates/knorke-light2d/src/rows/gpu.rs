//! The row lights on the GPU: the row atlas (one row of [`ROW_ANGLES`] texels per lamp), the
//! edge atlas of lamps with a source radius, their programs and what a frame records. Part of
//! the light pass ([`LightingStage`](crate::lighting::LightingStage) owns and drives it); it
//! records into the host's command buffer and never submits or reads back (ADR 0002).

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT,
    SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, CopyPass, Filter, Gpu, Load, PipelineCache,
    ProgramDesc, RenderPass, RenderTarget, SamplerBinding, Samplers, SecondTarget, ShaderLibrary,
    StageBindings, UploadBuffer,
};
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{
    MAX_ROWS, ROW_ANGLES, RowBuildParams, RowCache, RowEdgeParams, RowLamp, RowLight,
    RowLookupParams, RowMask, RowSpace,
};
use crate::light_pipelines::MATERIAL_TARGET_FORMAT;
use crate::lighting::{
    DetailMode, LIGHT_FORMAT, LightSurfacePassParams, LightingFrame, ShadowMaps, height_tolerance,
};

/// A row program, atlas or buffer is missing at record: the frame was not prepared.
pub const ROWS_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.rows.input_missing");

/// Rows and edges: four floats of two 12-bit numbers each (`PackRun` in `lightrow.hlsli`).
const ROW_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT;
/// Fewest rows of the atlas; it doubles when more lamps need one.
const MIN_ROWS: u32 = 16;

const BUILD: usize = 0;
const EDGE: usize = 1;

const fn atlas_program(fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        second: Some(SecondTarget {
            format: ROW_FORMAT,
            blend: Blend::Opaque,
        }),
        ..ProgramDesc::new("lightrow.vert", fragment, Blend::Opaque)
    }
}

const fn material_program(fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        second: Some(SecondTarget {
            format: MATERIAL_TARGET_FORMAT,
            blend: Blend::Additive,
        }),
        ..ProgramDesc::new("lightrow.vert", fragment, Blend::Additive)
    }
}

/// The light is one shader in three versions because the light pass has three target layouts
/// (Chiaro's `surface_light.hlsli`): the light buffer alone, with the specular buffer, with the
/// direction buffer.
const PROGRAMS: [ProgramDesc; 5] = [
    atlas_program("lightrow_build.frag"),
    atlas_program("lightrow_edge.frag"),
    ProgramDesc::new("lightrow.vert", "lightrow_light.frag", Blend::Additive),
    material_program("lightrow_light_exact.frag"),
    material_program("lightrow_light_direction.frag"),
];

const fn light_program(detail: DetailMode) -> usize {
    match detail {
        DetailMode::None => 2,
        DetailMode::Exact => 3,
        DetailMode::Direction => 4,
    }
}

/// What the prepared frame draws. The lamp buffer holds the frame's row lights, then copies of
/// those whose row is built, the ones with a source radius first (the edge pass draws only them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowsRecord {
    /// Row lights drawn.
    pub lights: u32,
    /// Rows built.
    pub built: u32,
    /// Of them with an edge row.
    pub edges: u32,
}

/// The row lights of the light pass (see the [module docs](super)).
#[derive(Debug)]
pub struct RowLights<'d> {
    rows: [RenderTarget<'d>; 2],
    edges: [RenderTarget<'d>; 2],
    lamps: UploadBuffer<'d>,
    pipelines: Option<PipelineCache<'d>>,
    cache: RowCache,
    /// Indices into the frame's row lights of those whose row is built; kept for its storage.
    dirty: Vec<u32>,
    frame: RowsRecord,
    detail: DetailMode,
}

impl<'d> RowLights<'d> {
    /// Without atlases and pipelines: the first frame with row lights creates them.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            rows: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            edges: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            lamps: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                MIN_ROWS * RowLamp::SIZE as u32,
                "row lamp",
            ),
            pipelines: None,
            cache: RowCache::default(),
            dirty: Vec::with_capacity(MAX_ROWS as usize),
            frame: RowsRecord::default(),
            detail: DetailMode::None,
        }
    }

    /// Forgets every row: the next frame builds them all.
    pub fn invalidate(&mut self) {
        self.cache.invalidate();
    }

    /// The frame's builds were recorded; without this call the next frame builds every row.
    pub fn commit(&mut self) {
        self.cache.commit();
    }

    /// What the prepared frame draws.
    pub fn frame(&self) -> RowsRecord {
        self.frame
    }

    /// Row atlas (two targets), for inspection.
    pub fn row_atlas(&self) -> [Option<knorke_gpu::TextureRef<'_>>; 2] {
        [self.rows[0].texture_ref(), self.rows[1].texture_ref()]
    }

    /// Decides the frame: which lamp has which row, which rows are built, the atlases, the
    /// pipelines and the lamp buffer. Returns the targets it (re)allocated. Call it every frame,
    /// also without row lights: a lamp keeps its row only from one frame to the next, because a
    /// frame it sat out may have changed the mask.
    ///
    /// # Errors
    /// An atlas, the buffer or a pipeline could not be created.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        frame: &LightingFrame<'_>,
        detail: DetailMode,
    ) -> Result<u32, Diagnostic> {
        self.frame = RowsRecord::default();
        self.detail = detail;
        self.dirty.clear();
        self.cache.begin();
        let lights: &[RowLight] = &frame.lights.rows;
        if lights.is_empty() {
            return Ok(0);
        }
        // Without the occlusion stage nothing casts a shadow this frame. (Not `height`: the
        // stage creates its textures when it records its first frame.)
        let mask = frame.shadows.filter(|s| s.height_mode).map(row_mask);
        let mut allocations = 0;
        // Twice at most: growing the atlas loses its rows, so every lamp is assigned anew.
        for _ in 0..2 {
            self.dirty.clear();
            for (i, light) in lights.iter().enumerate() {
                if let Some((_, true)) = self.assign(light, mask.as_ref()) {
                    self.dirty.push(i as u32);
                }
            }
            let soft = self.dirty.iter().any(|&i| lights[i as usize].size > 0.0);
            let grown = self.ensure_atlases(soft)?;
            allocations += grown;
            if grown == 0 {
                break;
            }
            self.cache.invalidate();
        }
        // The lamps with an edge row first.
        self.dirty
            .sort_unstable_by_key(|&i| lights[i as usize].size <= 0.0);
        let edges = self
            .dirty
            .iter()
            .filter(|&&i| lights[i as usize].size > 0.0)
            .count();
        self.frame = RowsRecord {
            lights: lights.len() as u32,
            built: self.dirty.len() as u32,
            edges: edges as u32,
        };

        let pipelines = match &mut self.pipelines {
            Some(p) => p,
            none => none.insert(PipelineCache::new(shaders, &PROGRAMS)?),
        };
        pipelines.get(shaders, light_program(detail), LIGHT_FORMAT)?;
        if self.frame.built > 0 {
            pipelines.get(shaders, BUILD, ROW_FORMAT)?;
        }
        if edges > 0 {
            pipelines.get(shaders, EDGE, ROW_FORMAT)?;
        }

        let count = lights.len() + self.dirty.len();
        self.lamps.reserve((count * RowLamp::SIZE) as u64)?;
        let mut mapped = self.lamps.map()?;
        let bytes = mapped.bytes();
        let mut at = 0;
        let order = (0..lights.len() as u32).chain(self.dirty.iter().copied());
        for i in order {
            let light = &lights[i as usize];
            // Assigned above: asking again in the same frame returns the same row.
            let row = self.cache.row_of(light.key).filter(|_| light.shadowed);
            let lamp = RowLamp {
                light: light.light,
                row: [
                    row.filter(|_| mask.is_some()).map_or(-1.0, |r| r as f32),
                    light.size,
                    0.0,
                    0.0,
                ],
            };
            bytes[at..at + RowLamp::SIZE].copy_from_slice(&lamp.to_bytes());
            at += RowLamp::SIZE;
        }
        Ok(allocations)
    }

    fn assign(&mut self, light: &RowLight, mask: Option<&RowMask<'_>>) -> Option<(u32, bool)> {
        // `collect_lights` lets at most MAX_ROWS shadowed row lights through, so a row is free.
        self.cache.assign(light, mask.filter(|_| light.shadowed)?)
    }

    /// The atlases for the cache's rows; `soft` also the edge atlas. Returns how many targets
    /// were (re)allocated for a new height of the row atlas (the edge atlas alone loses nothing:
    /// a lamp's edge row is built with its row).
    fn ensure_atlases(&mut self, soft: bool) -> Result<u32, Diagnostic> {
        let height = self.cache.rows().next_power_of_two().max(MIN_ROWS);
        if self.cache.rows() == 0 {
            return Ok(0);
        }
        let mut grown = 0;
        for target in &mut self.rows {
            grown += u32::from(target.ensure(ROW_ANGLES, height, ROW_FORMAT)?);
        }
        if soft || self.edges[0].texture_ref().is_some() {
            for target in &mut self.edges {
                target.ensure(ROW_ANGLES, height, ROW_FORMAT)?;
            }
        }
        Ok(grown)
    }

    /// The lamp buffer into the frame's copy pass.
    ///
    /// # Errors
    /// The upload failed.
    pub fn record_uploads(&self, copy: &mut CopyPass<'_>) -> Result<(), Diagnostic> {
        let lamps = self.frame.lights + self.frame.built;
        if lamps > 0 {
            copy.upload_to_buffer(&self.lamps, lamps * RowLamp::SIZE as u32)?;
        }
        Ok(())
    }

    fn pass_params(frame: &LightingFrame<'_>, offset: u32, atlas_rows: u32) -> [u8; 64] {
        let (w, h) = (frame.scene.width(), frame.scene.height());
        // The row programs read `mask_base` as the atlas height (`lightrow.vert`).
        let mut p =
            LightSurfacePassParams::new(frame.projection, w, h, frame.max_height, atlas_rows);
        p.instance_offset = offset;
        p.to_bytes()
    }

    /// The row build and the edge pass of the rows this frame builds, each one pass into its
    /// atlas that keeps the other rows. Before the light pass.
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record_builds(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
    ) -> Result<(), Diagnostic> {
        if self.frame.built == 0 {
            return Ok(());
        }
        let shadows = frame.shadows.ok_or_else(|| missing("the occlusion mask"))?;
        let heights = shadows.height.ok_or_else(|| missing("the height map"))?;
        let nearest = samplers.sampler(Filter::Nearest);
        let space = RowSpace::new(frame.projection, &shadows.domain);
        let mask = [shadows.domain.width, shadows.domain.height];
        let [first, second] = self.atlas(&self.rows)?;
        let rows = first.height();
        let lamps = [self
            .lamps
            .buffer_ref()
            .ok_or_else(|| missing("the lamp buffer"))?];
        let vertex = Self::pass_params(frame, self.frame.lights, rows);
        let vertex_uniforms: [&[u8]; 1] = [&vertex];
        let passes: [(usize, u32, _, &[SamplerBinding<'_>], [u8; 32]); 2] = [
            (
                BUILD,
                self.frame.built,
                [first, second],
                &[
                    SamplerBinding::new(heights, nearest),
                    SamplerBinding::new(shadows.mask, nearest),
                ],
                RowBuildParams::new(&space, mask, shadows.any_partial).to_bytes(),
            ),
            (
                EDGE,
                self.frame.edges,
                if self.frame.edges > 0 {
                    self.atlas(&self.edges)?
                } else {
                    [first, second]
                },
                &[
                    SamplerBinding::new(first, nearest),
                    SamplerBinding::new(second, nearest),
                ],
                RowEdgeParams::new(&space, rows).to_bytes(),
            ),
        ];
        for (program, count, targets, textures, params) in passes {
            if count == 0 {
                continue;
            }
            let targets = targets.map(|t| ColorTarget::new(t, Load::Load));
            let mut pass = cmd.render_pass(&targets)?;
            let fragment_uniforms: [&[u8]; 1] = [&params];
            pass.bind_program(
                self.pipeline(program, ROW_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &lamps,
                        uniforms: &vertex_uniforms,
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: textures,
                        uniforms: &fragment_uniforms,
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, count)?;
        }
        Ok(())
    }

    /// The row lights into the open light pass (the targets of `detail` given to
    /// [`prepare`](Self::prepare)), additive like every light. `surface_params` is the pass's
    /// `SurfaceParams` block.
    ///
    /// # Errors
    /// Not prepared, or a binding failed.
    pub fn record_lights(
        &self,
        pass: &mut RenderPass<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
        surface_params: &[u8],
    ) -> Result<(), Diagnostic> {
        if self.frame.lights == 0 {
            return Ok(());
        }
        let nearest = samplers.sampler(Filter::Nearest);
        // What a frame does not have is not read: the scene stands in (lamps without a row
        // return before the mask, lamps without a size before the edges, lamps without a
        // profile before the profile atlas).
        let stand_in = frame.scene;
        let shadows = frame.shadows.filter(|s| s.height_mode);
        let rows = self.rows[0].texture_ref().zip(self.rows[1].texture_ref());
        let edges = self.edges[0].texture_ref().zip(self.edges[1].texture_ref());
        let (row0, row1) = rows.unwrap_or((stand_in, stand_in));
        let (edge0, edge1) = edges.unwrap_or((stand_in, stand_in));
        let mut textures = [
            frame.surface.unwrap_or(stand_in),
            shadows.map_or(stand_in, |s| s.mask),
            shadows.and_then(|s| s.height).unwrap_or(stand_in),
            row0,
            row1,
            edge0,
            edge1,
            frame.material.unwrap_or(stand_in),
            frame.profile_atlas.unwrap_or(stand_in),
        ]
        .map(|t| SamplerBinding::new(t, nearest));
        // The profile atlas follows the material buffer, which only the exact program reads.
        let exact = self.detail == DetailMode::Exact;
        if !exact {
            textures[7] = textures[8];
        }
        let lookup = match shadows {
            Some(s) => RowLookupParams::new(
                &RowSpace::new(frame.projection, &s.domain),
                [s.domain.width, s.domain.height],
                row0.height(),
                height_tolerance(frame.settings),
            ),
            None => RowLookupParams::default(),
        }
        .to_bytes();
        let lamps = [self
            .lamps
            .buffer_ref()
            .ok_or_else(|| missing("the lamp buffer"))?];
        let vertex = Self::pass_params(frame, 0, 0);
        let vertex_uniforms: [&[u8]; 1] = [&vertex];
        let fragment_uniforms: [&[u8]; 2] = [surface_params, &lookup];
        pass.bind_program(
            self.pipeline(light_program(self.detail), LIGHT_FORMAT)?,
            &Bindings {
                vertex: StageBindings {
                    storage_buffers: &lamps,
                    uniforms: &vertex_uniforms,
                    ..Default::default()
                },
                fragment: StageBindings {
                    samplers: &textures[..if exact { 9 } else { 8 }],
                    uniforms: &fragment_uniforms,
                    ..Default::default()
                },
            },
        )?;
        pass.draw_instanced(6, self.frame.lights)
    }

    fn atlas<'a>(
        &self,
        targets: &'a [RenderTarget<'d>; 2],
    ) -> Result<[knorke_gpu::TextureRef<'a>; 2], Diagnostic> {
        match (targets[0].texture_ref(), targets[1].texture_ref()) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(missing("a row atlas")),
        }
    }

    fn pipeline(
        &self,
        program: usize,
        format: SDL_GPUTextureFormat,
    ) -> Result<knorke_gpu::PipelineRef<'_>, Diagnostic> {
        self.pipelines
            .as_ref()
            .and_then(|p| p.pipeline(program, format))
            .ok_or_else(|| missing("a row pipeline"))
    }
}

fn row_mask<'a>(s: &'a ShadowMaps<'a>) -> RowMask<'a> {
    RowMask {
        size: [s.domain.width, s.domain.height],
        snapped: s.snapped,
        change: s.change,
    }
}

fn missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        ROWS_INPUT_MISSING,
        format!("{what} is missing after prepare"),
    )
}
