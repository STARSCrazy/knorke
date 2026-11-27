//! The row lights on the GPU: the row atlas (one row of [`super::ROW_ANGLES`] texels per lamp, two a line), the
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
    StageBindings, TextureRef, UploadBuffer,
};
use knorke_math::bytemuck;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{
    ATLAS_WIDTH, MAX_SUN_BANDS, PolarRowsParams, Refresh, RowBuildParams, RowCache, RowEdgeParams,
    RowLamp, RowLight, RowLookupParams, RowMask, RowSpace, SUPPORTS, SunBands, SunCache,
    SunHeightRowsParams, SunParams, SunSource, WIDE_SLOT, height_code, sun_bands, sun_plain,
};
use crate::light_culling::Casters;
use crate::lighting::MATERIAL_TARGET_FORMAT;
use crate::lighting::{
    DetailMode, LIGHT_FORMAT, LightSurfacePassParams, LightingFrame, ShadowMaps, height_tolerance,
};
use crate::medium_stage::{POLAR_FORMAT, SUN_HEIGHT_FORMAT};
use crate::obstacle::ObstacleId;
use crate::pattern_occluders::PatternParams;

/// A row program, atlas or buffer is missing at record: the frame was not prepared.
pub const ROWS_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.rows.input_missing");

/// Rows and edges: four floats of two 12-bit numbers each (`PackRun` in `lightrow.hlsli`); a
/// sun map: two intervals (lo, hi).
const ROW_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT;
/// Fewest lines of the atlas; it doubles when more lamps need one.
const MIN_ROWS: u32 = 8;

const BUILD: usize = 0;
const EDGE: usize = 1;
const PATTERN: usize = 5;
const SUN_BUILD: usize = 11;
const SUN_BAND: usize = 16;
/// The medium stage's shaft rows and sun heights read from the rows and maps.
const POLAR: usize = 33;
const SUN_HEIGHT: usize = 34;
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

/// A map with partly transparent runs has two more targets, a run (lo, hi, q, pass) each
/// (`lightrow_sun_build_partial.frag`), and its second one also for a hard sun.
const PARTIAL_TARGETS: [SecondTarget; 2] = [SecondTarget {
    format: ROW_FORMAT,
    blend: Blend::Opaque,
}; 2];

const fn partial_build(vertex: &'static str) -> ProgramDesc {
    ProgramDesc {
        fragment: "lightrow_sun_build_partial.frag",
        more: &PARTIAL_TARGETS,
        ..soft_build(vertex)
    }
}

/// The program that builds a sun's map whole or over its bands (`update`): with the second
/// target if `soft` (the hard programs drop the build's second output), with all four if
/// `partial`.
const fn sun_build_program(update: bool, soft: bool, partial: bool) -> usize {
    match (update, soft, partial) {
        (false, false, false) => SUN_BUILD,
        (true, false, false) => SUN_BAND,
        (false, true, false) => 17,
        (true, true, false) => 18,
        (false, _, true) => 19,
        (true, _, true) => 20,
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
/// for a soft sun's map with its second target and for a map with partly transparent runs.
/// Last the light and sun programs again through the medium world map (`_medium`, Chiaro's
/// `*_medium` light programs), in the order of [`light_program`] and [`sun_program`].
const PROGRAMS: [ProgramDesc; 35] = [
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
    partial_build("fullscreen.vert"),
    partial_build("lightrow_sun_band.vert"),
    ProgramDesc::new(
        "lightrow.vert",
        "lightrow_light_medium.frag",
        Blend::Additive,
    ),
    material_program("lightrow.vert", "lightrow_light_exact_medium.frag"),
    material_program("lightrow.vert", "lightrow_light_direction_medium.frag"),
    ProgramDesc::new(
        "lightrow_mask.vert",
        "lightrow_light_mask_medium.frag",
        Blend::Additive,
    ),
    material_program(
        "lightrow_mask.vert",
        "lightrow_light_exact_mask_medium.frag",
    ),
    material_program(
        "lightrow_mask.vert",
        "lightrow_light_direction_mask_medium.frag",
    ),
    ProgramDesc::new(
        "lightrow.vert",
        "lightrow_light_flat_medium.frag",
        Blend::Additive,
    ),
    ProgramDesc::new(
        "lightrow_mask.vert",
        "lightrow_light_flat_mask_medium.frag",
        Blend::Additive,
    ),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_light_medium.frag",
        Blend::Additive,
    ),
    material_program("fullscreen.vert", "lightrow_sun_light_exact_medium.frag"),
    material_program(
        "fullscreen.vert",
        "lightrow_sun_light_direction_medium.frag",
    ),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_light_flat_medium.frag",
        Blend::Additive,
    ),
    ProgramDesc::new("fullscreen.vert", "lightrow_polar.frag", Blend::Opaque),
    ProgramDesc::new("fullscreen.vert", "lightrow_sun_height.frag", Blend::Opaque),
];

/// The first `_medium` program of [`PROGRAMS`].
const MEDIUM_PROGRAMS: usize = 21;

/// `flat`: no surface and no material buffer, so no detail targets either; `medium`: through
/// the medium world map.
const fn light_program(detail: DetailMode, masked: bool, flat: bool, medium: bool) -> usize {
    let k = if flat {
        6 + masked as usize
    } else {
        3 * masked as usize + detail_index(detail)
    };
    if medium {
        MEDIUM_PROGRAMS + k
    } else {
        [2, 3, 4, 6, 7, 8, 9, 10][k]
    }
}

/// The sun program of `detail` (see [`light_program`]).
const fn sun_program(detail: DetailMode, flat: bool, medium: bool) -> usize {
    let k = if flat { 3 } else { detail_index(detail) };
    k + if medium { MEDIUM_PROGRAMS + 8 } else { 12 }
}

const fn detail_index(detail: DetailMode) -> usize {
    match detail {
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
    /// Directional lights drawn by the sun programs without a map ([`sun_plain`]).
    pub plain_suns: u32,
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
    /// Whose heights its map is built from (E53).
    casters: Casters,
    /// `LightList::directional_keys`: the medium stage finds the map by it.
    key: u64,
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
            work => Some(sun_build_program(
                work == MapWork::Update,
                self.soft(),
                self.params.partial(),
            )),
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
    /// The row of each of the frame's row lights, [`SUPPORTS`] places each (-1: none); kept for its
    /// storage.
    slots: Vec<f32>,
    /// The key of each of the frame's row lights; kept for its storage.
    keys: Vec<u64>,
    frame: RowsRecord,
    detail: DetailMode,
    /// The frame has neither surface nor material buffer.
    flat: bool,
    /// The frame has a medium world map: the `_medium` programs.
    medium: bool,
    /// How the rows hold heights ([`height_code`]); another code rebuilds them all.
    heights: [f32; 4],
    gpu: Gpu<'d>,
    /// The sun maps (`lightrow_sun_build.frag`), as many as the most suns a frame had.
    maps: Vec<RenderTarget<'d>>,
    /// Their second targets, a soft sun's or a map's with partly transparent runs.
    reaches: Vec<RenderTarget<'d>>,
    /// Their partly transparent runs, two targets each.
    partials: Vec<[RenderTarget<'d>; 2]>,
    sun_cache: SunCache,
    /// The frame's suns with a map, the first [`RowsRecord::suns`] of them.
    suns: Vec<FrameSun>,
    /// The texels each of them updates ([`sun_bands`]).
    bands: Vec<SunBands>,
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
            dirty: Vec::new(),
            slots: Vec::new(),
            keys: Vec::new(),
            frame: RowsRecord::default(),
            detail: DetailMode::None,
            flat: false,
            medium: false,
            heights: height_code(0.0),
            gpu,
            maps: Vec::new(),
            reaches: Vec::new(),
            partials: Vec::new(),
            sun_cache: SunCache::default(),
            suns: Vec::new(),
            bands: Vec::new(),
            band_buffer: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                (MAX_SUN_BANDS * SUN_BAND_SIZE) as u32,
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
        self.medium = frame.medium_map.is_some();
        self.dirty.clear();
        self.keys.clear();
        self.cache.begin();
        self.sun_cache.begin();
        // A crossing (a sun's pattern run) names its obstacle by record index, which moves when
        // the frame's pattern obstacles change anywhere; and a row built without them has none.
        // A sun's map rebuilds only if an obstacle it may name moved to another index: the texels
        // that named one which left are rebuilt with the mask change that took it out.
        let members = frame
            .shadows
            .and_then(|s| s.patterns.as_ref())
            .map_or(&[][..], |p| p.members);
        if !members
            .iter()
            .map(|m| m.0)
            .eq(self.pattern_ids.iter().copied())
        {
            self.cache.invalidate();
            let moved =
                self.pattern_ids.iter().enumerate().any(|(k, &id)| {
                    (members.iter().position(|m| m.0 == id)).is_some_and(|j| j != k)
                });
            if moved {
                self.sun_cache.invalidate();
            }
            self.pattern_ids.clear();
            self.pattern_ids.extend(members.iter().map(|m| m.0));
        }
        let allocations = self.prepare_suns(frame)? + self.prepare_rows(frame)?;

        let f = self.frame;
        if f.lights + f.suns + f.plain_suns == 0 {
            return Ok(allocations);
        }
        let pipelines = match &mut self.pipelines {
            Some(p) => p,
            none => none.insert(PipelineCache::new(shaders, &PROGRAMS)?),
        };
        let (flat, medium) = (self.flat, self.medium);
        let patterns = frame.shadows.is_some_and(|s| s.patterns.is_some());
        let programs = [
            (
                f.masked < f.lights,
                light_program(detail, false, flat, medium),
                LIGHT_FORMAT,
            ),
            (
                f.masked > 0,
                light_program(detail, true, flat, medium),
                LIGHT_FORMAT,
            ),
            (f.built > 0, BUILD, ROW_FORMAT),
            (f.edges > 0, EDGE, ROW_FORMAT),
            (f.built > 0 && patterns, PATTERN, ROW_FORMAT),
            (
                f.suns + f.plain_suns > 0,
                sun_program(detail, flat, medium),
                LIGHT_FORMAT,
            ),
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
        // The medium stage reads its shaft rows and sun heights from here (`record_polar`).
        if frame.settings.haze.shafts {
            if f.lights > 0 {
                pipelines.get(shaders, POLAR, POLAR_FORMAT)?;
            }
            if f.suns + f.plain_suns > 0 {
                pipelines.get(shaders, SUN_HEIGHT, SUN_HEIGHT_FORMAT)?;
            }
        }
        Ok(allocations)
    }

    /// The frame's suns with a map (not [`sun_plain`], in a row frame with active shadows; any
    /// number) and their maps. A map no sun uses this frame is released: the cache builds it
    /// anew when a sun takes it again. Returns the maps (re)allocated.
    fn prepare_suns(&mut self, frame: &LightingFrame<'_>) -> Result<u32, Diagnostic> {
        let list = frame.lights;
        let mut n = 0;
        if let Some(s) = frame.shadows {
            let mask = row_mask(s);
            let tolerance = height_tolerance(frame.settings);
            let candidates = (list.directional.iter().zip(&list.directional_casters))
                .zip(&list.directional_rise)
                .enumerate()
                .filter(|(_, ((g, _), _))| !sun_plain(g));
            let count = candidates.clone().count();
            // Storage for the most suns a frame had: steady frames do not allocate.
            while self.suns.len() < count {
                self.suns.push(FrameSun::default());
                self.bands.push(SunBands::default());
                self.maps.push(RenderTarget::new(self.gpu));
                self.reaches.push(RenderTarget::new(self.gpu));
                self.partials
                    .push([RenderTarget::new(self.gpu), RenderTarget::new(self.gpu)]);
            }
            for (i, ((g, &casters), &rise)) in candidates {
                let (top, snapped) = (s.scene_max_top, mask.snapped);
                let params = SunParams::new(g, rise, mask.size, snapped, tolerance, top)
                    .with_partial(s.any_partial, s.patterns.is_some());
                let source = SunSource::new(&params, &mask).with_casters(casters);
                let Some((map, refresh)) = self.sun_cache.assign(i as u64, source, mask.change)
                else {
                    break; // not reached: a map for every sun
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
                    casters,
                    key: list.directional_keys[i],
                };
                n += 1;
            }
        }
        let mut allocations = 0;
        let targets = self.maps.iter_mut().zip(&mut self.reaches);
        for (k, ((target, reach), partial)) in targets.zip(&mut self.partials).enumerate() {
            let f = self.suns[..n].iter_mut().find(|f| f.map as usize == k);
            let size = f
                .as_ref()
                .map(|f| [f.params.origin[2], f.params.origin[3]].map(|v| v as u32));
            let partial_runs = f.as_ref().is_some_and(|f| f.params.partial());
            let wanted = [
                (target, size.is_some(), ROW_FORMAT),
                (
                    reach,
                    f.as_ref().is_some_and(|f| f.soft()) || partial_runs,
                    REACH_FORMAT,
                ),
            ];
            let [p0, p1] = partial;
            let more = [
                (p0, partial_runs, ROW_FORMAT),
                (p1, partial_runs, ROW_FORMAT),
            ];
            let mut grown = false;
            for (t, needed, format) in wanted.into_iter().chain(more) {
                match size.filter(|_| needed) {
                    Some([w, h]) => grown |= t.ensure(w, h, format)?,
                    None => t.release(),
                }
            }
            if let Some(f) = f.filter(|_| grown) {
                allocations += 1;
                f.work = MapWork::Build;
            }
        }
        // Every other directional light is drawn without a map: one without shadow, or with
        // shadows but no obstacle to cast them (no occlusion stage this frame).
        let plain = list.directional.len() - n;
        if n + plain > 0 {
            // The stand-in for the pattern records of a frame without them.
            let bytes = n * MAX_SUN_BANDS * SUN_BAND_SIZE;
            self.band_buffer.reserve(bytes as u64)?;
        }
        self.frame.suns = n as u32;
        self.frame.plain_suns = plain as u32;
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
            // Reserved above (an updated map means n > 0).
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
        let shadows = frame.shadows;
        if let Some(code) = shadows.map(|s| height_code(s.scene_max_top))
            && code != self.heights
        {
            self.heights = code;
            self.cache.invalidate();
        }
        // A row sees nothing beyond the mask's rim, so an empty strip a pan exposed or pushed
        // out changes nothing in it.
        let mask = shadows.map(|s| RowMask {
            change: s.content,
            ..row_mask(s)
        });
        let patterns = shadows.and_then(|s| s.patterns.as_ref());
        let mut allocations = 0;
        // Twice at most: growing the atlas loses its rows, so every lamp is assigned anew.
        for _ in 0..2 {
            self.dirty.clear();
            self.slots.clear();
            self.slots.resize(lights.len() * SUPPORTS as usize, -1.0);
            for (i, light) in lights.iter().enumerate() {
                for k in 0..light.rows() {
                    let at = i as u32 * SUPPORTS + k;
                    if let Some((row, build)) = self.assign(&light.support(k), mask.as_ref()) {
                        let wide = if light.wide() { WIDE_SLOT } else { 0.0 };
                        self.slots[at as usize] = row as f32 + wide;
                        if build {
                            self.dirty.push(at);
                        }
                    }
                }
            }
            self.keys.extend(lights.iter().map(|l| l.key));
            let soft = self.dirty.iter().any(|&i| built(lights, i).size > 0.0);
            let grown = self.ensure_atlases(soft, patterns.is_some())?;
            allocations += grown;
            if grown == 0 {
                break;
            }
            self.cache.invalidate();
        }
        // The lamps with an edge row first, each part by caster class (one build draw a class).
        self.dirty.sort_unstable_by_key(|&i| {
            let lamp = built(lights, i);
            (lamp.size <= 0.0, class(lamp.casters))
        });
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
        // Each lamp with the place of its first row in `slots` (-1 without; a line or rect
        // light's support points follow it), the lights in the order of their draws.
        let order = LIGHT_DRAWS
            .into_iter()
            .flat_map(|draw| (0..lights.len()).filter(move |&i| in_draw(&lights[i], draw)))
            .map(|i| (i * SUPPORTS as usize, lights[i]))
            .chain((self.dirty.iter()).map(|&i| (i as usize, built(lights, i))));
        for (n, (slot, light)) in order.enumerate() {
            let mut lamp = RowLamp {
                light: light.light,
                row: [self.slots[slot], light.size, 0.0, 0.0],
                mask: light.mask.unwrap_or_default(),
                window: light.window[0],
                window_v: light.window[1],
            };
            if n < lights.len() && light.is_area() && light.shadowed {
                lamp.light.profile = [0, 1, 2, 3].map(|k| self.slots[slot + k]);
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

    /// The atlases for the cache's rows (two slots a line); `soft` also the edge atlas,
    /// `patterns` also the crossing atlas. Returns how many targets were (re)allocated for a new
    /// height of the row or the crossing atlas (the edge atlas alone loses nothing: a lamp's edge
    /// row is built with its row).
    fn ensure_atlases(&mut self, soft: bool, patterns: bool) -> Result<u32, Diagnostic> {
        let height = self
            .cache
            .rows()
            .div_ceil(2)
            .next_power_of_two()
            .max(MIN_ROWS);
        if self.cache.rows() == 0 {
            return Ok(0);
        }
        let mut grown = 0;
        for target in &mut self.rows {
            grown += u32::from(target.ensure(ATLAS_WIDTH, height, ROW_FORMAT)?);
        }
        if soft || self.edges[0].texture_ref().is_some() {
            for target in &mut self.edges {
                target.ensure(ATLAS_WIDTH, height, ROW_FORMAT)?;
            }
        }
        if patterns {
            grown += u32::from(self.crossings.ensure(ATLAS_WIDTH, height, ROW_FORMAT)?);
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
            let bytes = self.frame.suns as usize * MAX_SUN_BANDS * SUN_BAND_SIZE;
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
            let [p0, p1] = &self.partials[sun.map as usize];
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
            let all = [Some(map), reach, p0.texture_ref(), p1.texture_ref()];
            let used = if sun.params.partial() {
                4
            } else if sun.soft() {
                2
            } else {
                1
            };
            let mut targets = [ColorTarget::new(map, load); 4];
            for (t, texture) in targets.iter_mut().zip(&all).take(used) {
                *t = ColorTarget::new(texture.ok_or_else(|| missing("a sun map target"))?, load);
            }
            let mut pass = cmd.render_pass(&targets[..used])?;
            let update = sun.work == MapWork::Update;
            let (class_mask, class_heights) = class_maps(shadows, sun.casters)?;
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
                        // The partial program reads mask and ids too (the ids only with
                        // pattern obstacles).
                        samplers: &[
                            class_heights,
                            class_mask,
                            shadows.patterns.as_ref().map_or(heights, |p| p.ids),
                        ]
                        .map(|t| SamplerBinding::new(t, nearest))
                            [..if used == 4 { 3 } else { 1 }],
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
        let bind = |pass: &mut RenderPass<'_>,
                    program,
                    vertex: &[u8],
                    textures: &[SamplerBinding<'_>],
                    params: &[u8]| {
            pass.bind_program(
                self.pipeline(program, ROW_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &lamps,
                        uniforms: &[vertex],
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: textures,
                        uniforms: &[params],
                        ..Default::default()
                    },
                },
            )
        };
        // One draw per caster class of the built lamps (`dirty` is ordered so), from the class's
        // heights and mask.
        let lights = &frame.lights.rows[..];
        let params = RowBuildParams::new(&space, mask, shadows.any_partial, self.heights);
        let params = params.to_bytes();
        let mut pass =
            cmd.render_pass(&[first, second].map(|t| ColorTarget::new(t, Load::Load)))?;
        let mut at = 0;
        while at < self.dirty.len() {
            let casters = built(lights, self.dirty[at]).casters;
            let n = (self.dirty[at..].iter())
                .take_while(|&&i| built(lights, i).casters == casters)
                .count();
            let (class_mask, class_heights) = class_maps(shadows, casters)?;
            let vertex = Self::pass_params(frame, self.frame.lights + at as u32, rows);
            let textures = [class_heights, class_mask].map(|t| SamplerBinding::new(t, nearest));
            bind(&mut pass, BUILD, &vertex, &textures, &params)?;
            pass.draw_instanced(6, n as u32)?;
            at += n;
        }
        drop(pass);
        if self.frame.edges > 0 {
            let targets = self
                .atlas(&self.edges)?
                .map(|t| ColorTarget::new(t, Load::Load));
            let mut pass = cmd.render_pass(&targets)?;
            let textures = [first, second].map(|t| SamplerBinding::new(t, nearest));
            let params = RowEdgeParams::new(&space, rows, self.heights).to_bytes();
            bind(&mut pass, EDGE, &vertex, &textures, &params)?;
            pass.draw_instanced(6, self.frame.edges)?;
        }
        if let Some(p) = &shadows.patterns {
            // After the build: it reads the runs.
            let lookup = RowLookupParams::new(&space, mask, rows, 0.0, self.heights).to_bytes();
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
        let shadows = frame.shadows;
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
            stand_in, // the medium map's place (below)
        ]
        .map(|t| SamplerBinding::new(t, nearest));
        // Filtered as Chiaro's (`light_mask.hlsli`); only the `_mask` programs read it.
        textures[11].sampler = samplers.sampler(Filter::Linear);
        // The rest follows the material buffer, which only the exact program reads.
        let exact = self.detail == DetailMode::Exact;
        if !exact {
            textures.copy_within(8.., 7);
        }
        // The `_medium` programs read the medium map after all other samplers (nearest).
        let medium = frame.medium_map.filter(|_| self.medium);
        let records = patterns.map_or([lamps; 3], |p| [p.obstacles, p.records, p.data]);
        let pattern_params = patterns.map_or(PatternParams::default().to_bytes(), |p| p.params);
        let lookup = match shadows {
            Some(s) => RowLookupParams::new(
                &RowSpace::new(frame.projection, &s.domain),
                [s.domain.width, s.domain.height],
                row0.height(),
                height_tolerance(frame.settings),
                self.heights,
            ),
            None => RowLookupParams::default(),
        }
        .to_bytes();
        let mut fragment_uniforms: [&[u8]; 4] = [surface_params, &lookup, &pattern_params, &[]];
        if let Some(m) = &medium {
            fragment_uniforms[3] = m.params;
        }
        let uniforms = 3 + usize::from(medium.is_some());
        let mut first = 0;
        for draw in LIGHT_DRAWS {
            let (masked, casters) = draw;
            let count = (frame.lights.rows.iter())
                .filter(|l| in_draw(l, draw))
                .count() as u32;
            if count == 0 {
                continue;
            }
            let vertex = Self::pass_params(frame, first, 0);
            first += count;
            let vertex_uniforms: [&[u8]; 1] = [&vertex];
            let mut samplers = usize::from(exact) + usize::from(masked) + 10;
            // A copy: the unmasked lamps' medium map takes the masked ones' mask atlas place;
            // the receiver rule reads the class's mask and heights.
            let mut textures = textures;
            if let Some(s) = &shadows {
                let (class_mask, class_heights) = class_maps(s, casters)?;
                textures[1].texture = class_mask;
                textures[2].texture = class_heights;
            }
            if let Some(m) = &medium {
                textures[samplers] = SamplerBinding::new(m.map, nearest);
                samplers += 1;
            }
            pass.bind_program(
                self.pipeline(
                    light_program(self.detail, masked, self.flat, self.medium),
                    LIGHT_FORMAT,
                )?,
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &[lamps],
                        uniforms: &vertex_uniforms,
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &textures[..samplers],
                        storage_buffers: &records,
                        uniforms: &fragment_uniforms[..uniforms],
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, count)?;
        }
        Ok(())
    }

    /// The directional light `light` (index into `LightList::directional`) into the open light
    /// pass, one fullscreen draw: from its sun map, or unshadowed if the prepared frame gave it
    /// none. `surface_params` is the pass's `SurfaceParams` block.
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
    ) -> Result<(), Diagnostic> {
        let suns = &self.suns[..self.frame.suns as usize];
        let sun = suns.iter().find(|s| s.light as usize == light);
        let plain = frame.lights.directional.get(light);
        // Neither surface nor material buffer: the flat program does not read the first; a hard
        // sun does not read the second map, a map without partly transparent runs not the next
        // two, a frame without pattern obstacles not the atlas and the records; a sun without
        // map reads none of the shadow textures.
        let stand_in = frame.scene;
        let (params, shadows, [map, reach, p0, p1]) = match (sun, plain) {
            (Some(sun), _) => {
                let shadows = frame.shadows.ok_or_else(|| missing("the occlusion mask"))?;
                let k = sun.map as usize;
                let map = (self.maps[k].texture_ref()).ok_or_else(|| missing("a sun map"))?;
                let [p0, p1] = &self.partials[k];
                let more = [&self.reaches[k], p0, p1].map(|t| t.texture_ref().unwrap_or(map));
                (sun.params, Some(shadows), [map, more[0], more[1], more[2]])
            }
            (None, Some(g)) => {
                let mut params = SunParams {
                    light: *g,
                    ..SunParams::default()
                };
                params.light.color[3] = 0.0; // visibility 1 (lightrow_sun_surface.hlsli)
                (params, None, [stand_in; 4])
            }
            (None, None) => return Err(missing("the directional light")),
        };
        let patterns = shadows.and_then(|s| s.patterns.as_ref());
        let (mask, height) = match (shadows, sun) {
            (Some(s), Some(sun)) => class_maps(s, sun.casters)?,
            _ => (stand_in, stand_in),
        };
        let mut textures = [
            frame.surface.or(frame.material).unwrap_or(stand_in),
            mask,
            height,
            map,
            reach,
            p0,
            p1,
            patterns.map_or(map, |p| p.atlas),
            frame.material.unwrap_or(stand_in),
            map, // the medium map's place (below)
        ]
        .map(|t| SamplerBinding::new(t, samplers.sampler(Filter::Nearest)));
        // Only the exact program reads the material buffer; the `_medium` programs read the
        // medium map after all other samplers.
        let mut count = if self.detail == DetailMode::Exact && !self.flat {
            9
        } else {
            8
        };
        let medium = frame.medium_map.filter(|_| self.medium);
        if let Some(m) = &medium {
            textures[count].texture = m.map;
            count += 1;
        }
        let stand_in =
            (self.band_buffer.buffer_ref()).ok_or_else(|| missing("the sun band buffer"))?;
        let records = patterns.map_or([stand_in; 3], |p| [p.obstacles, p.records, p.data]);
        let pattern_params = patterns.map_or(PatternParams::default().to_bytes(), |p| p.params);
        let params = params.to_bytes();
        let uniforms: [&[u8]; 4] = [
            surface_params,
            &params,
            &pattern_params,
            medium.map_or(&[], |m| m.params),
        ];
        pass.bind_program(
            self.pipeline(
                sun_program(self.detail, self.flat, self.medium),
                LIGHT_FORMAT,
            )?,
            &Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &textures[..count],
                    storage_buffers: &records,
                    uniforms: &uniforms[..3 + usize::from(medium.is_some())],
                    ..Default::default()
                },
            },
        )?;
        pass.draw_fullscreen()
    }

    /// The row of the frame's row light `key` ([`super::row_key`]), its slot code as the lamp
    /// buffer has it; `None` without one.
    pub fn row_of(&self, key: u64) -> Option<f32> {
        let i = self.keys.iter().position(|&k| k == key)?;
        Some(self.slots[i * SUPPORTS as usize]).filter(|&slot| slot >= 0.0)
    }

    /// The frame's sun `key` (`LightList::directional_keys`) has a map.
    pub fn has_sun_map(&self, key: u64) -> bool {
        self.frame_sun(key).is_some()
    }

    fn frame_sun(&self, key: u64) -> Option<&FrameSun> {
        self.suns[..self.frame.suns as usize]
            .iter()
            .find(|s| s.key == key)
    }

    /// The medium stage's shaft row of the row light `key` ([`row_of`](Self::row_of)) into the
    /// open pass, whose viewport is the row (Chiaro's polar row from the light's row instead of
    /// the distance field, `lightrow_polar.frag`): `height` the lamp's height, `length` the march
    /// length in mask texels.
    ///
    /// # Errors
    /// The light has no row, or a binding failed.
    pub fn record_polar(
        &self,
        pass: &mut RenderPass<'_>,
        samplers: &Samplers<'_>,
        key: u64,
        height: f32,
        length: f32,
    ) -> Result<(), Diagnostic> {
        let row = self
            .row_of(key)
            .ok_or_else(|| missing("a shaft light's row"))?;
        let [first, second] = self.atlas(&self.rows)?;
        let params = PolarRowsParams {
            row_size: [0.0, first.height() as f32, 0.0, 0.0],
            heights: self.heights,
            lamp: [row, height, length, 0.0],
        }
        .to_bytes();
        let nearest = samplers.sampler(Filter::Nearest);
        let textures = [first, second].map(|t| SamplerBinding::new(t, nearest));
        pass.bind_program(
            self.pipeline(POLAR, POLAR_FORMAT)?,
            &Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &textures,
                    uniforms: &[&params],
                    ..Default::default()
                },
            },
        )?;
        pass.draw_fullscreen()
    }

    /// The medium stage's sun shadow heights of the sun `key` (`LightList::directional_keys`)
    /// into the open pass, whose viewport is its block (Chiaro's from its map instead of the
    /// distance field, `lightrow_sun_height.frag`); a sun without a map stands straight overhead:
    /// from the heights of `casters`' class.
    ///
    /// # Errors
    /// A map or the class's heights are missing, or a binding failed.
    pub fn record_sun_height(
        &self,
        pass: &mut RenderPass<'_>,
        samplers: &Samplers<'_>,
        shadows: &ShadowMaps<'_>,
        key: u64,
        casters: Casters,
    ) -> Result<(), Diagnostic> {
        let (mask, heights) = class_maps(shadows, casters)?;
        let (params, column, [map, p0, p1]) = match self.frame_sun(key) {
            Some(sun) => {
                let k = sun.map as usize;
                let map = (self.maps[k].texture_ref()).ok_or_else(|| missing("a sun map"))?;
                let runs = self.partials[k]
                    .each_ref()
                    .map(|t| t.texture_ref().unwrap_or(map));
                (sun.params, 0.0, [map, runs[0], runs[1]])
            }
            None => (SunParams::default(), 1.0, [mask; 3]),
        };
        let size = [shadows.domain.width as f32, shadows.domain.height as f32];
        let column = SunHeightRowsParams {
            column: [column, size[0], size[1], 0.0],
        }
        .to_bytes();
        let params = params.to_bytes();
        let nearest = samplers.sampler(Filter::Nearest);
        let textures = [map, p0, p1, heights, mask].map(|t| SamplerBinding::new(t, nearest));
        pass.bind_program(
            self.pipeline(SUN_HEIGHT, SUN_HEIGHT_FORMAT)?,
            &Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &textures,
                    uniforms: &[&params, &column],
                    ..Default::default()
                },
            },
        )?;
        pass.draw_fullscreen()
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

/// The draws of the row lights in lamp buffer order: the unmasked ones, then the masked ones
/// (their own programs), each by caster class (the class's mask and heights).
const LIGHT_DRAWS: [(bool, Casters); 6] = [
    (false, Casters::All),
    (false, Casters::Static),
    (false, Casters::Dynamic),
    (true, Casters::All),
    (true, Casters::Static),
    (true, Casters::Dynamic),
];

fn in_draw(light: &RowLight, (masked, casters): (bool, Casters)) -> bool {
    light.mask.is_some() == masked && light.casters == casters
}

/// The order of the caster classes in the draws.
fn class(casters: Casters) -> usize {
    casters.filter_index().map_or(0, |i| i + 1)
}

/// The mask and heights of `casters`' class.
fn class_maps<'s>(
    s: &'s ShadowMaps<'_>,
    casters: Casters,
) -> Result<(TextureRef<'s>, TextureRef<'s>), Diagnostic> {
    let maps = s.maps(casters)?;
    Ok((
        maps.mask,
        (maps.height).ok_or_else(|| missing("the height map"))?,
    ))
}

fn row_mask<'a>(s: &'a ShadowMaps<'a>) -> RowMask<'a> {
    RowMask {
        size: [s.domain.width, s.domain.height],
        snapped: s.snapped,
        change: s.change,
    }
}

fn missing(what: &str) -> Diagnostic {
    crate::missing_after_prepare(ROWS_INPUT_MISSING, what)
}
