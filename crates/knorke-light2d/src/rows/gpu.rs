//! The row lights on the GPU: the row atlas (one row of [`ROW_ANGLES`] texels per lamp), the
//! edge atlas of lamps with a source radius, the crossing atlas of frames with pattern obstacles,
//! the maps of the suns, their programs and what a frame records. Part of
//! the light pass ([`LightingStage`](crate::lighting::LightingStage) owns and drives it); it
//! records into the host's command buffer and never submits or reads back (ADR 0002).

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT, SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, CopyPass, Filter, Gpu, Load, PipelineCache,
    ProgramDesc, RenderPass, RenderTarget, SamplerBinding, Samplers, SecondTarget, ShaderLibrary,
    StageBindings, UploadBuffer,
};
use knorke_math::bytemuck;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{
    MAX_ROWS, MAX_SUN_BANDS, MAX_SUN_MAP, MAX_SUNS, ROW_ANGLES, Refresh, RowBuildParams, RowCache,
    RowEdgeParams, RowLamp, RowLight, RowLookupParams, RowMask, RowSpace, SUPPORTS, SunBands,
    SunCache, SunGrid, SunParams, SunSource, frame_suits, sun_bands, sun_suits,
};
use crate::light_pipelines::MATERIAL_TARGET_FORMAT;
use crate::lighting::{
    DetailMode, LIGHT_FORMAT, LightSurfacePassParams, LightingFrame, ShadowMaps, height_tolerance,
};
use crate::obstacle::ObstacleId;
use crate::pattern_occluders::PatternParams;

/// A row program, atlas or buffer is missing at record: the frame was not prepared.
pub const ROWS_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.rows.input_missing");

/// Rows and edges: four floats of two 12-bit numbers each (`PackRun` in `lightrow.hlsli`); a
/// sun map: two intervals (lo, hi).
const ROW_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT;
/// Fewest rows of the atlas; it doubles when more lamps need one.
const MIN_ROWS: u32 = 16;

const BUILD: usize = 0;
const EDGE: usize = 1;
const PATTERN: usize = 5;
const SUN_BUILD: usize = 11;
const SUN_BAND: usize = 16;
/// A soft sun's map has a second target: how far along the ray its intervals' ends lie.
const REACH_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

const fn soft_build(vertex: &'static str) -> ProgramDesc {
    ProgramDesc {
        second: Some(SecondTarget {
            format: REACH_FORMAT,
            blend: Blend::Opaque,
        }),
        ..ProgramDesc::new(vertex, "lightrow_sun_build.frag", Blend::Opaque)
    }
}

/// The program that builds a sun's map whole or over its bands (`update`), with the second
/// target if `soft` (the hard programs drop the build's second output).
const fn sun_build_program(update: bool, soft: bool) -> usize {
    match (update, soft) {
        (false, false) => SUN_BUILD,
        (true, false) => SUN_BAND,
        (false, true) => 17,
        (true, true) => 18,
    }
}
/// Bytes of a band (`SunBand` of `lightrow_sun_band.vert`).
const SUN_BAND_SIZE: usize = 32;

const fn atlas_program(fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        second: Some(SecondTarget {
            format: ROW_FORMAT,
            blend: Blend::Opaque,
        }),
        ..ProgramDesc::new("lightrow.vert", fragment, Blend::Opaque)
    }
}

const fn material_program(vertex: &'static str, fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        second: Some(SecondTarget {
            format: MATERIAL_TARGET_FORMAT,
            blend: Blend::Additive,
        }),
        ..ProgramDesc::new(vertex, fragment, Blend::Additive)
    }
}

/// The light is one shader in three versions because the light pass has three target layouts
/// (Chiaro's `surface_light.hlsli`): the light buffer alone, with the specular buffer, with the
/// direction buffer; and each again for masked lamps. A frame without surface and material
/// buffer has Chiaro's flat light (no Lambert term), with and without a mask. A sun is a
/// fullscreen draw (Chiaro's `fullscreen.vert`) in the same four layouts, its map build one too;
/// an update of the map draws the build over its bands (`lightrow_sun_band.vert`); both again
/// for a soft sun's map with its second target.
const PROGRAMS: [ProgramDesc; 19] = [
    atlas_program("lightrow_build.frag"),
    atlas_program("lightrow_edge.frag"),
    ProgramDesc::new("lightrow.vert", "lightrow_light.frag", Blend::Additive),
    material_program("lightrow.vert", "lightrow_light_exact.frag"),
    material_program("lightrow.vert", "lightrow_light_direction.frag"),
    ProgramDesc::new("lightrow.vert", "lightrow_pattern.frag", Blend::Opaque),
    ProgramDesc::new(
        "lightrow_mask.vert",
        "lightrow_light_mask.frag",
        Blend::Additive,
    ),
    material_program("lightrow_mask.vert", "lightrow_light_exact_mask.frag"),
    material_program("lightrow_mask.vert", "lightrow_light_direction_mask.frag"),
    ProgramDesc::new("lightrow.vert", "lightrow_light_flat.frag", Blend::Additive),
    ProgramDesc::new(
        "lightrow_mask.vert",
        "lightrow_light_flat_mask.frag",
        Blend::Additive,
    ),
    ProgramDesc::new("fullscreen.vert", "lightrow_sun_build.frag", Blend::Opaque),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_light.frag",
        Blend::Additive,
    ),
    material_program("fullscreen.vert", "lightrow_sun_light_exact.frag"),
    material_program("fullscreen.vert", "lightrow_sun_light_direction.frag"),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_light_flat.frag",
        Blend::Additive,
    ),
    ProgramDesc::new(
        "lightrow_sun_band.vert",
        "lightrow_sun_build.frag",
        Blend::Opaque,
    ),
    soft_build("fullscreen.vert"),
    soft_build("lightrow_sun_band.vert"),
];

/// `flat`: no surface and no material buffer, so no detail targets either.
const fn light_program(detail: DetailMode, masked: bool, flat: bool) -> usize {
    if flat {
        return if masked { 10 } else { 9 };
    }
    let base = if masked { 6 } else { 2 };
    base + match detail {
        DetailMode::None => 0,
        DetailMode::Exact => 1,
        DetailMode::Direction => 2,
    }
}

/// The sun program of `detail` (see [`light_program`]).
const fn sun_program(detail: DetailMode, flat: bool) -> usize {
    if flat {
        return 15;
    }
    12 + match detail {
        DetailMode::None => 0,
        DetailMode::Exact => 1,
        DetailMode::Direction => 2,
    }
}

/// What the prepared frame draws. The lamp buffer holds the frame's row lights, the masked ones
/// last (their own programs), then the lamps whose row is built (a line or rect light's support
/// points), the ones with a source radius first (the edge pass draws only them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowsRecord {
    /// Row lights drawn.
    pub lights: u32,
    /// Of them with a mask.
    pub masked: u32,
    /// Rows built.
    pub built: u32,
    /// Of them with an edge row.
    pub edges: u32,
    /// Directional lights drawn with a sun map.
    pub suns: u32,
    /// Sun maps built whole.
    pub suns_built: u32,
    /// Sun maps updated in part (a pan, redrawn rectangles).
    pub suns_updated: u32,
    /// Sun map texels built or updated.
    pub sun_texels: u64,
}

/// What a sun's map needs this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MapWork {
    /// Nothing: it holds.
    #[default]
    Keep,
    /// The texels of its bands.
    Update,
    /// All of it.
    Build,
}

/// A sun of the prepared frame with a map.
#[derive(Clone, Copy, Debug, Default)]
struct FrameSun {
    /// Index into `LightList::directional`.
    light: u32,
    /// Its map.
    map: u32,
    work: MapWork,
    params: SunParams,
}

impl FrameSun {
    /// A source radius: the map has a second target.
    fn soft(&self) -> bool {
        self.params.build[2] > 0.0
    }

    /// The program its map's work this frame draws with, `None` when kept.
    fn build_program(&self) -> Option<usize> {
        match self.work {
            MapWork::Keep => None,
            work => Some(sun_build_program(work == MapWork::Update, self.soft())),
        }
    }
}

/// The row lights of the light pass (see the [module docs](super)).
#[derive(Debug)]
pub struct RowLights<'d> {
    rows: [RenderTarget<'d>; 2],
    edges: [RenderTarget<'d>; 2],
    /// Where a ray crosses a pattern obstacle (`lightrow_pattern.frag`).
    crossings: RenderTarget<'d>,
    /// The pattern obstacles the crossings name by record index, in that order.
    pattern_ids: Vec<ObstacleId>,
    lamps: UploadBuffer<'d>,
    pipelines: Option<PipelineCache<'d>>,
    cache: RowCache,
    /// Rows built this frame: index into the frame's row lights times [`SUPPORTS`] plus the
    /// support point; kept for its storage.
    dirty: Vec<u32>,
    frame: RowsRecord,
    detail: DetailMode,
    /// The frame has neither surface nor material buffer.
    flat: bool,
    /// The sun maps (`lightrow_sun_build.frag`).
    maps: [RenderTarget<'d>; MAX_SUNS],
    /// Their second targets, a soft sun's only.
    reaches: [RenderTarget<'d>; MAX_SUNS],
    sun_cache: SunCache,
    /// The frame's suns with a map, [`RowsRecord::suns`] of them.
    suns: [FrameSun; MAX_SUNS],
    /// The texels each of them updates ([`sun_bands`]).
    bands: [SunBands; MAX_SUNS],
    /// The bands of the frame's updates, [`MAX_SUN_BANDS`] per sun.
    band_buffer: UploadBuffer<'d>,
}

impl<'d> RowLights<'d> {
    /// Without atlases and pipelines: the first frame with row lights creates them.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            rows: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            edges: [RenderTarget::new(gpu), RenderTarget::new(gpu)],
            crossings: RenderTarget::new(gpu),
            pattern_ids: Vec::new(),
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
            flat: false,
            maps: core::array::from_fn(|_| RenderTarget::new(gpu)),
            reaches: core::array::from_fn(|_| RenderTarget::new(gpu)),
            sun_cache: SunCache::default(),
            suns: [FrameSun::default(); MAX_SUNS],
            bands: [SunBands::default(); MAX_SUNS],
            band_buffer: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                (MAX_SUNS * MAX_SUN_BANDS * SUN_BAND_SIZE) as u32,
                "sun band",
            ),
        }
    }

    /// Forgets every row and sun map: the next frame builds them all.
    pub fn invalidate(&mut self) {
        self.cache.invalidate();
        self.sun_cache.invalidate();
    }

    /// The frame's builds were recorded; without this call the next frame builds every row and
    /// sun map.
    pub fn commit(&mut self) {
        self.cache.commit();
        self.sun_cache.commit();
    }

    /// What the prepared frame draws.
    pub fn frame(&self) -> RowsRecord {
        self.frame
    }

    /// Row atlas (two targets), for inspection.
    pub fn row_atlas(&self) -> [Option<knorke_gpu::TextureRef<'_>>; 2] {
        [self.rows[0].texture_ref(), self.rows[1].texture_ref()]
    }

    /// Decides the frame: which lamp has which row, which sun has which map, which rows and maps
    /// are built, the atlases, the pipelines and the lamp buffer. Returns the targets it
    /// (re)allocated. Call it every frame, also without row lights: a lamp keeps its row (a sun
    /// its map) only from one frame to the next, because a frame it sat out may have changed the
    /// mask.
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
        self.flat = frame.surface.is_none() && frame.material.is_none();
        self.dirty.clear();
        self.cache.begin();
        self.sun_cache.begin();
        let allocations = self.prepare_suns(frame)? + self.prepare_rows(frame)?;

        let f = self.frame;
        if f.lights + f.suns == 0 {
            return Ok(allocations);
        }
        let pipelines = match &mut self.pipelines {
            Some(p) => p,
            none => none.insert(PipelineCache::new(shaders, &PROGRAMS)?),
        };
        let flat = self.flat;
        let patterns = frame.shadows.is_some_and(|s| s.patterns.is_some());
        let programs = [
            (
                f.masked < f.lights,
                light_program(detail, false, flat),
                LIGHT_FORMAT,
            ),
            (
                f.masked > 0,
                light_program(detail, true, flat),
                LIGHT_FORMAT,
            ),
            (f.built > 0, BUILD, ROW_FORMAT),
            (f.edges > 0, EDGE, ROW_FORMAT),
            (f.built > 0 && patterns, PATTERN, ROW_FORMAT),
            (f.suns > 0, sun_program(detail, flat), LIGHT_FORMAT),
        ];
        for (needed, program, format) in programs {
            if needed {
                pipelines.get(shaders, program, format)?;
            }
        }
        for sun in &self.suns[..f.suns as usize] {
            if let Some(program) = sun.build_program() {
                pipelines.get(shaders, program, ROW_FORMAT)?;
            }
        }
        Ok(allocations)
    }

    /// The frame's suns with a map ([`sun_suits`], in a frame [`frame_suits`] accepts with
    /// active shadows whose obstacles are all opaque without pattern; at most [`MAX_SUNS`]) and
    /// their maps. A map no sun uses this frame is released: the cache builds it anew when a sun
    /// takes it again. Returns the maps (re)allocated.
    fn prepare_suns(&mut self, frame: &LightingFrame<'_>) -> Result<u32, Diagnostic> {
        let list = frame.lights;
        let mut n = 0;
        if let Some(s) = frame
            .shadows
            .filter(|s| s.height_mode && !s.partial_in_mask)
        {
            let mask = row_mask(s);
            let suits = SunGrid::size_of(mask.size)[1] <= MAX_SUN_MAP
                && frame_suits(frame.settings, frame.medium_map.is_some(), s.scene_max_top);
            let tolerance = height_tolerance(frame.settings);
            let candidates = (list.directional.iter().zip(&list.directional_casters))
                .zip(&list.directional_rise)
                .enumerate()
                .filter(|(_, ((g, c), _))| suits && sun_suits(g, **c));
            for (i, ((g, _), &rise)) in candidates {
                let (top, snapped) = (s.scene_max_top, mask.snapped);
                let params = SunParams::new(g, rise, mask.size, snapped, tolerance, top);
                let source = SunSource::new(&params, &mask);
                let Some((map, refresh)) = self.sun_cache.assign(i as u64, source, mask.change)
                else {
                    break; // all maps taken
                };
                let work = match refresh {
                    Refresh::Keep => MapWork::Keep,
                    Refresh::Build => MapWork::Build,
                    Refresh::Update(before) => {
                        if sun_bands(&params, &source, &before, mask.change, &mut self.bands[n]) {
                            MapWork::Update
                        } else {
                            MapWork::Build
                        }
                    }
                };
                self.suns[n] = FrameSun {
                    light: i as u32,
                    map,
                    work,
                    params,
                };
                n += 1;
            }
        }
        let mut allocations = 0;
        for (k, (target, reach)) in self.maps.iter_mut().zip(&mut self.reaches).enumerate() {
            match self.suns[..n].iter_mut().find(|f| f.map as usize == k) {
                Some(f) => {
                    let [w, h] = [f.params.origin[2], f.params.origin[3]].map(|v| v as u32);
                    if target.ensure(w, h, ROW_FORMAT)? {
                        allocations += 1;
                        f.work = MapWork::Build;
                    }
                    if !f.soft() {
                        reach.release();
                    } else if reach.ensure(w, h, REACH_FORMAT)? {
                        allocations += 1;
                        f.work = MapWork::Build;
                    }
                }
                None => {
                    target.release();
                    reach.release();
                }
            }
        }
        self.frame.suns = n as u32;
        for (f, bands) in self.suns[..n].iter().zip(&self.bands) {
            let texels = match f.work {
                MapWork::Keep => 0,
                MapWork::Update => bands.texels(),
                MapWork::Build => (f.params.origin[2] * f.params.origin[3]) as u64,
            };
            self.frame.suns_built += u32::from(f.work == MapWork::Build);
            self.frame.suns_updated += u32::from(f.work == MapWork::Update && texels > 0);
            self.frame.sun_texels += texels;
        }
        if self.frame.suns_updated > 0 {
            let bytes = MAX_SUNS * MAX_SUN_BANDS * SUN_BAND_SIZE;
            self.band_buffer.reserve(bytes as u64)?;
            let mut mapped = self.band_buffer.map()?;
            let out = mapped.bytes();
            for (k, bands) in self.bands[..n].iter().enumerate() {
                let at = k * MAX_SUN_BANDS * SUN_BAND_SIZE;
                for (i, q) in bands.quads().iter().enumerate() {
                    let at = at + i * SUN_BAND_SIZE;
                    out[at..at + SUN_BAND_SIZE].copy_from_slice(bytemuck::bytes_of(q));
                }
            }
        }
        Ok(allocations)
    }

    /// The frame's row lights: their rows, the atlases and the lamp buffer. Returns the targets
    /// it (re)allocated.
    fn prepare_rows(&mut self, frame: &LightingFrame<'_>) -> Result<u32, Diagnostic> {
        let lights: &[RowLight] = &frame.lights.rows;
        if lights.is_empty() {
            return Ok(0);
        }
        // Without the occlusion stage nothing casts a shadow this frame. (Not `height`: the
        // stage creates its textures when it records its first frame.)
        let shadows = frame.shadows.filter(|s| s.height_mode);
        let mask = shadows.map(row_mask);
        let patterns = shadows.and_then(|s| s.patterns.as_ref());
        // A crossing names its obstacle by record index, which moves when the frame's pattern
        // obstacles change anywhere; and a row built without them has no crossings.
        let members = patterns.map_or(&[][..], |p| p.members);
        if !members
            .iter()
            .map(|m| m.0)
            .eq(self.pattern_ids.iter().copied())
        {
            self.cache.invalidate();
            self.pattern_ids.clear();
            self.pattern_ids.extend(members.iter().map(|m| m.0));
        }
        let mut allocations = 0;
        // Twice at most: growing the atlas loses its rows, so every lamp is assigned anew.
        for _ in 0..2 {
            self.dirty.clear();
            for (i, light) in lights.iter().enumerate() {
                for k in 0..light.rows() {
                    if let Some((_, true)) = self.assign(&light.support(k), mask.as_ref()) {
                        self.dirty.push(i as u32 * SUPPORTS + k);
                    }
                }
            }
            let soft = self.dirty.iter().any(|&i| built(lights, i).size > 0.0);
            let grown = self.ensure_atlases(soft, patterns.is_some())?;
            allocations += grown;
            if grown == 0 {
                break;
            }
            self.cache.invalidate();
        }
        // The lamps with an edge row first.
        self.dirty
            .sort_unstable_by_key(|&i| built(lights, i).size <= 0.0);
        let edges = self
            .dirty
            .iter()
            .filter(|&&i| built(lights, i).size > 0.0)
            .count();
        let masked = lights.iter().filter(|l| l.mask.is_some()).count() as u32;
        if masked > 0 && frame.mask_atlas.is_none() {
            return Err(missing("the mask atlas of masked row lights"));
        }
        self.frame = RowsRecord {
            lights: lights.len() as u32,
            masked,
            built: self.dirty.len() as u32,
            edges: edges as u32,
            ..self.frame
        };

        let count = lights.len() + self.dirty.len();
        self.lamps.reserve((count * RowLamp::SIZE) as u64)?;
        let mut mapped = self.lamps.map()?;
        let bytes = mapped.bytes();
        let mut at = 0;
        let order = lights
            .iter()
            .filter(|l| l.mask.is_none())
            .chain(lights.iter().filter(|l| l.mask.is_some()))
            .copied()
            .chain(self.dirty.iter().map(|&i| built(lights, i)));
        // Assigned above: asking again in the same frame returns the same row. Without the mask
        // no lamp has one.
        let slot = |key| {
            self.cache
                .row_of(key)
                .filter(|_| mask.is_some())
                .map_or(-1.0, |r| r as f32)
        };
        for (n, light) in order.enumerate() {
            let row = if light.shadowed {
                slot(light.key)
            } else {
                -1.0
            };
            let mut lamp = RowLamp {
                light: light.light,
                row: [row, light.size, 0.0, 0.0],
                mask: light.mask.unwrap_or_default(),
                window: light.window[0],
                window_v: light.window[1],
            };
            if n < lights.len() && light.is_area() && light.shadowed {
                // Support point k's key is the light's plus k (`RowLight::support`).
                lamp.light.profile = [0, 1, 2, 3].map(|k| slot(light.key + k));
            }
            bytes[at..at + RowLamp::SIZE].copy_from_slice(&lamp.to_bytes());
            at += RowLamp::SIZE;
        }
        Ok(allocations)
    }

    fn assign(&mut self, light: &RowLight, mask: Option<&RowMask<'_>>) -> Option<(u32, bool)> {
        // `collect_lights` lets at most MAX_ROWS shadowed row lights through, so a row is free.
        self.cache.assign(light, mask.filter(|_| light.shadowed)?)
    }

    /// The atlases for the cache's rows; `soft` also the edge atlas, `patterns` also the crossing
    /// atlas. Returns how many targets were (re)allocated for a new height of the row or the
    /// crossing atlas (the edge atlas alone loses nothing: a lamp's edge row is built with its row).
    fn ensure_atlases(&mut self, soft: bool, patterns: bool) -> Result<u32, Diagnostic> {
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
        if patterns {
            grown += u32::from(self.crossings.ensure(ROW_ANGLES, height, ROW_FORMAT)?);
        } else {
            self.crossings.release();
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
        if self.frame.suns_updated > 0 {
            let bytes = MAX_SUNS * MAX_SUN_BANDS * SUN_BAND_SIZE;
            copy.upload_to_buffer(&self.band_buffer, bytes as u32)?;
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

    /// The sun maps this frame builds or updates, one pass each (an update draws the build over
    /// its bands into the kept map); the row build and the edge pass of the rows this frame
    /// builds, each one pass into its atlas that keeps the other rows. Before the light pass.
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record_builds(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
    ) -> Result<(), Diagnostic> {
        let f = self.frame;
        if f.built + f.suns_built + f.suns_updated == 0 {
            return Ok(());
        }
        let shadows = frame.shadows.ok_or_else(|| missing("the occlusion mask"))?;
        let heights = shadows.height.ok_or_else(|| missing("the height map"))?;
        let nearest = samplers.sampler(Filter::Nearest);
        for (k, (sun, bands)) in self.suns[..f.suns as usize]
            .iter()
            .zip(&self.bands)
            .enumerate()
        {
            let count = bands.quads().len() as u32;
            let load = match sun.work {
                MapWork::Update if count > 0 => Load::Load,
                MapWork::Build => Load::DontCare,
                _ => continue,
            };
            let Some(program) = sun.build_program() else {
                continue;
            };
            let map = self.maps[sun.map as usize]
                .texture_ref()
                .ok_or_else(|| missing("a sun map"))?;
            let reach = self.reaches[sun.map as usize].texture_ref();
            let params = sun.params.to_bytes();
            // `SunBandPass` of lightrow_sun_band.vert: where the map's first lattice texel lies in
            // the texture, the map's size and this sun's first band.
            let mut pass_bytes = [0u8; 32];
            let [x, y, w, h] = [
                sun.params.lattice[0].to_bits() as i32,
                sun.params.lattice[1].to_bits() as i32,
                sun.params.origin[2] as i32,
                sun.params.origin[3] as i32,
            ];
            let band = [
                x.rem_euclid(w) as f32,
                y.rem_euclid(h) as f32,
                w as f32,
                h as f32,
            ];
            pass_bytes[..16].copy_from_slice(bytemuck::bytes_of(&band));
            pass_bytes[16..20].copy_from_slice(&((k * MAX_SUN_BANDS) as u32).to_le_bytes());
            let buffer = self.band_buffer.buffer_ref();
            let targets = match (sun.soft(), reach) {
                (false, _) => &[ColorTarget::new(map, load)][..],
                (true, Some(reach)) => {
                    &[ColorTarget::new(map, load), ColorTarget::new(reach, load)]
                }
                (true, None) => return Err(missing("a soft sun's second map")),
            };
            let mut pass = cmd.render_pass(targets)?;
            let update = sun.work == MapWork::Update;
            let vertex = match (update, buffer) {
                (true, Some(buffer)) => StageBindings {
                    storage_buffers: &[buffer],
                    uniforms: &[&pass_bytes],
                    ..Default::default()
                },
                (true, None) => return Err(missing("the sun band buffer")),
                (false, _) => StageBindings::default(),
            };
            pass.bind_program(
                self.pipeline(program, ROW_FORMAT)?,
                &Bindings {
                    vertex,
                    fragment: StageBindings {
                        samplers: &[SamplerBinding::new(heights, nearest)],
                        uniforms: &[&params],
                        ..Default::default()
                    },
                },
            )?;
            if !update {
                pass.draw_fullscreen()?;
            } else {
                // Each band four times, moved by the map's size: it wraps around the texture.
                pass.draw_instanced(6, 4 * count)?;
            }
        }
        if self.frame.built == 0 {
            return Ok(());
        }
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
        if let Some(p) = &shadows.patterns {
            // After the build: it reads the runs.
            let lookup = RowLookupParams::new(&space, mask, rows, 0.0).to_bytes();
            let target =
                (self.crossings.texture_ref()).ok_or_else(|| missing("the crossing atlas"))?;
            let mut pass = cmd.render_pass(&[ColorTarget::new(target, Load::Load)])?;
            pass.bind_program(
                self.pipeline(PATTERN, ROW_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &lamps,
                        uniforms: &vertex_uniforms,
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &[first, second, p.ids].map(|t| SamplerBinding::new(t, nearest)),
                        storage_buffers: &[p.obstacles, p.data],
                        uniforms: &[&lookup, &p.params],
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, self.frame.built)?;
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
        // profile before the profile atlas, a frame without pattern obstacles has all-zero
        // pattern params and reads no crossing); the lamp buffer stands in for the records.
        let stand_in = frame.scene;
        let shadows = frame.shadows.filter(|s| s.height_mode);
        let patterns = shadows.and_then(|s| s.patterns.as_ref());
        let lamps = self
            .lamps
            .buffer_ref()
            .ok_or_else(|| missing("the lamp buffer"))?;
        let rows = self.rows[0].texture_ref().zip(self.rows[1].texture_ref());
        let edges = self.edges[0].texture_ref().zip(self.edges[1].texture_ref());
        let (row0, row1) = rows.unwrap_or((stand_in, stand_in));
        let (edge0, edge1) = edges.unwrap_or((stand_in, stand_in));
        let mut textures = [
            // Without a surface buffer the material buffer, as Chiaro's light programs; neither:
            // not read (SurfaceAt).
            frame.surface.or(frame.material).unwrap_or(stand_in),
            shadows.map_or(stand_in, |s| s.mask),
            shadows.and_then(|s| s.height).unwrap_or(stand_in),
            row0,
            row1,
            edge0,
            edge1,
            frame.material.unwrap_or(stand_in),
            frame.profile_atlas.unwrap_or(stand_in),
            self.crossings.texture_ref().unwrap_or(stand_in),
            patterns.map_or(stand_in, |p| p.atlas),
            frame.mask_atlas.unwrap_or(stand_in),
        ]
        .map(|t| SamplerBinding::new(t, nearest));
        // Filtered as Chiaro's (`light_mask.hlsli`); only the `_mask` programs read it.
        textures[11].sampler = samplers.sampler(Filter::Linear);
        // The rest follows the material buffer, which only the exact program reads.
        let exact = self.detail == DetailMode::Exact;
        if !exact {
            textures.copy_within(8.., 7);
        }
        let records = patterns.map_or([lamps; 3], |p| [p.obstacles, p.records, p.data]);
        let pattern_params = patterns.map_or(PatternParams::default().to_bytes(), |p| p.params);
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
        let fragment_uniforms: [&[u8]; 3] = [surface_params, &lookup, &pattern_params];
        let plain = self.frame.lights - self.frame.masked;
        for (masked, first, count) in [(false, 0, plain), (true, plain, self.frame.masked)] {
            if count == 0 {
                continue;
            }
            let vertex = Self::pass_params(frame, first, 0);
            let vertex_uniforms: [&[u8]; 1] = [&vertex];
            let samplers = usize::from(exact) + usize::from(masked) + 10;
            pass.bind_program(
                self.pipeline(light_program(self.detail, masked, self.flat), LIGHT_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &[lamps],
                        uniforms: &vertex_uniforms,
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &textures[..samplers],
                        storage_buffers: &records,
                        uniforms: &fragment_uniforms,
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, count)?;
        }
        Ok(())
    }

    /// The directional light `light` (index into `LightList::directional`) into the open light
    /// pass if the prepared frame gave it a sun map, one fullscreen draw; `false` if not (it is
    /// Chiaro's then). `surface_params` is the pass's `SurfaceParams` block.
    ///
    /// # Errors
    /// Not prepared, or a binding failed.
    pub fn record_sun(
        &self,
        pass: &mut RenderPass<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
        surface_params: &[u8],
        light: usize,
    ) -> Result<bool, Diagnostic> {
        let suns = &self.suns[..self.frame.suns as usize];
        let Some(sun) = suns.iter().find(|s| s.light as usize == light) else {
            return Ok(false);
        };
        let shadows = frame.shadows.ok_or_else(|| missing("the occlusion mask"))?;
        let map = self.maps[sun.map as usize]
            .texture_ref()
            .ok_or_else(|| missing("a sun map"))?;
        // Neither surface nor material buffer: the flat program does not read the first; a hard
        // sun does not read the second map.
        let stand_in = frame.scene;
        let textures = [
            frame.surface.or(frame.material).unwrap_or(stand_in),
            shadows.mask,
            shadows.height.ok_or_else(|| missing("the height map"))?,
            map,
            self.reaches[sun.map as usize].texture_ref().unwrap_or(map),
            frame.material.unwrap_or(stand_in),
        ]
        .map(|t| SamplerBinding::new(t, samplers.sampler(Filter::Nearest)));
        // Only the exact program reads the material buffer.
        let count = if self.detail == DetailMode::Exact && !self.flat {
            6
        } else {
            5
        };
        let params = sun.params.to_bytes();
        pass.bind_program(
            self.pipeline(sun_program(self.detail, self.flat), LIGHT_FORMAT)?,
            &Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &textures[..count],
                    uniforms: &[surface_params, &params],
                    ..Default::default()
                },
            },
        )?;
        pass.draw_fullscreen()?;
        Ok(true)
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

/// The lamp of a `dirty` entry.
fn built(lights: &[RowLight], i: u32) -> RowLight {
    lights[(i / SUPPORTS) as usize].support(i % SUPPORTS)
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
