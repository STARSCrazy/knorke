// Origin: chiaro @ 9bc5b9b, src/rendering/MediumStage.hpp and MediumStage.cpp (Prepare,
// PrepareShafts, PrepareSuns, Record; the shaft rows and sun heights come from the light rows,
// ADR 0019), src/gpu/PipelineCache.cpp
// (the medium programs' blending and second target, kMediumGuideFormat).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R8G8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R16_FLOAT, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
    SDL_GPUTextureFormat, SDL_GPUViewport,
};
use knorke_gpu::{
    Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, PipelineRef, ProgramDesc,
    RenderTarget, SamplerBinding, Samplers, SecondTarget, ShaderLibrary, TextureRef, UploadBuffer,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{
    MediumLightPassParams, MediumMaskLightPassParams, POLAR_ANGLES, PolarQueryParams, ShaftRow,
    SunBlock, SunParams, SunShadows, finish_suns, mask_rows, medium_runs, medium_top, plan_shafts,
    plan_suns, polar_row,
};
use crate::LightStore;
use crate::atlas::LightAtlases;
use crate::binding_layout::{LightSampler, LightStorage, LightUniform};
use crate::composite::CompositeMedium;
use crate::gpu_light::{GpuLight, GpuMask};
use crate::light_culling::LightModes;
use crate::lighting::{Resources, ShadowMaps, bind_named, height_tolerance};
use crate::medium_culling::{
    MediumLightList, MediumSun, collect_medium_lights, collect_medium_suns,
};
use crate::medium_map::{
    MediumMapParams, MediumMapView, MediumParams, MediumProgram, medium_layout_of, medium_params,
};
use crate::medium_table::MediumTable;
use crate::profile_store::ProfileStore;
use crate::projection::Projection;
use crate::rows::RowLights;
use crate::settings::LightSettings;

/// Recording without a prepare of the frame, or a target or pipeline is missing after it.
pub const MEDIUM_STAGE_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.medium_stage.not_prepared");

/// Format of the scatter buffer and the filtered one: rgb scattered light, a transmittance.
pub const SCATTER_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// Format of the guides the pre-filter writes per scatter texel.
pub const MEDIUM_GUIDE_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8_UNORM;
/// Format of the polar atlas: (distance, top) of two obstacles per angle (`lightrow_polar.frag`
/// writes it from the rows).
pub const POLAR_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// Format of the sun shadow height atlas (`lightrow_sun_height.frag` writes it from the maps).
pub const SUN_HEIGHT_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16_FLOAT;

const fn desc(program: MediumProgram) -> ProgramDesc {
    let blend = match program {
        MediumProgram::Light | MediumProgram::LightMask | MediumProgram::Directional => {
            Blend::AdditiveRgb
        }
        _ => Blend::Opaque,
    };
    let mut d = ProgramDesc::new(program.vertex(), program.fragment(), blend);
    if matches!(program, MediumProgram::Filter) {
        d.second = Some(SecondTarget {
            format: MEDIUM_GUIDE_FORMAT,
            blend: Blend::Opaque,
        });
    }
    d
}

/// The stage's programs in [`MediumProgram::ALL`] order.
pub const MEDIUM_PROGRAMS: [ProgramDesc; 6] = {
    let mut out = [desc(MediumProgram::Base); 6];
    let mut i = 0;
    while i < 6 {
        out[i] = desc(MediumProgram::ALL[i]);
        i += 1;
    }
    out
};

const fn index(program: MediumProgram) -> usize {
    match program {
        MediumProgram::Base => 0,
        MediumProgram::BaseZones => 1,
        MediumProgram::Light => 2,
        MediumProgram::LightMask => 3,
        MediumProgram::Directional => 4,
        MediumProgram::Filter => 5,
    }
}

/// Initial light buffer capacity in lights.
const INITIAL_LIGHTS: u32 = 16;

/// The inputs of one frame (Chiaro `FrameInputs` as far as the stage reads them); every texture
/// is borrowed.
#[derive(Clone, Copy)]
pub struct MediumStageFrame<'a> {
    /// The scene colour; stands in for absent textures (not read then).
    pub scene: TextureRef<'a>,
    /// Surface buffer.
    pub surface: Option<TextureRef<'a>>,
    /// The medium layer; `None` with haze only.
    pub layer: Option<TextureRef<'a>>,
    /// The media table the layer picks from.
    pub media: &'a MediumTable,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Settings: haze, medium quality, steps, ambient.
    pub settings: &'a LightSettings,
    /// The occlusion results; `None` without shadows (no shafts, no sun shadows).
    pub shadows: Option<&'a ShadowMaps<'a>>,
    /// The medium world map (P29); `None`: no map.
    pub medium_map: Option<MediumMapView<'a>>,
    /// The pure ambient of the ambient zones (the base pass reads it per pixel); `None`
    /// without zones.
    pub pure_ambient: Option<TextureRef<'a>>,
    /// The flat directional sum of the zones.
    pub zone_directional: Vec3,
    /// The light pass's rows and sun maps (prepared, and recorded before this stage), which the
    /// shaft rows and sun heights are read from.
    pub rows: &'a RowLights<'a>,
}

/// The lights of a frame's medium stage, read by [`MediumStage::prepare`] only.
#[derive(Clone, Copy)]
pub struct MediumLights<'a> {
    /// The lights.
    pub store: &'a LightStore,
    /// Profiles of profiled lights.
    pub profiles: Option<&'a ProfileStore>,
    /// Animation time in seconds.
    pub time: f64,
    /// Culling modes as for the ground; `height_mode` is the shadows' one, `max_height` the
    /// highest height a lit pixel can have.
    pub modes: LightModes<'a>,
}

/// What [`MediumStage::record`] recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediumStageRecord {
    /// Point, spot, line and rect lights and directional lights in the medium.
    pub lights: u32,
    /// Rows of the polar atlas (lights with light shafts).
    pub shaft_rows: u32,
    /// Blocks of the sun height atlas.
    pub sun_blocks: u32,
    /// Draw calls.
    pub draws: u32,
}

/// The medium stage (Chiaro `MediumStage`, see the [module docs](super)).
///
/// Per frame: [`prepare`](Self::prepare) after the light pass's prepare, [`record`](Self::record)
/// after the light pass, then [`composite_medium`](Self::composite_medium) for the composite.
/// A frame without haze and without a medium layer releases everything.
#[derive(Debug)]
pub struct MediumStage<'d> {
    pipelines: PipelineCache<'d>,
    scatter: RenderTarget<'d>,
    /// The scatter buffer after the pre-filter: what the composite reads.
    filtered: RenderTarget<'d>,
    guides: RenderTarget<'d>,
    /// The polar atlas: [`POLAR_ANGLES`] wide, rows grow to the most shaft lights so far.
    polar: RenderTarget<'d>,
    /// The sun shadow height atlas, grows and never shrinks.
    sun_heights: RenderTarget<'d>,
    lights: UploadBuffer<'d>,
    masks: UploadBuffer<'d>,
    list: MediumLightList,
    /// The shaft entries of `list`, masked lights after the others.
    shafts: Vec<Option<crate::medium_culling::Shafts>>,
    rows: Vec<ShaftRow>,
    suns: Vec<MediumSun>,
    sun_params: Vec<SunParams>,
    blocks: Vec<SunBlock>,
    block: (u32, u32),
    query: PolarQueryParams,
    params: [u8; MediumParams::SIZE],
    /// `MediumParams::sizes`: scene and scatter buffer size.
    sizes: [f32; 4],
    base_zones: bool,
    /// `Some` after a prepare of a frame that runs the stage.
    prepared: Option<Prepared>,
}

#[derive(Clone, Copy, Debug)]
struct Prepared {
    /// Lights without a mask (the first in the light buffer).
    mask_base: u32,
    /// All lights, the masked ones included.
    light_count: u32,
}

fn missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        MEDIUM_STAGE_NOT_PREPARED,
        format!("{what} is missing after prepare"),
    )
}

fn viewport(x: f32, y: f32, w: f32, h: f32) -> SDL_GPUViewport {
    SDL_GPUViewport {
        x,
        y,
        w,
        h,
        min_depth: 0.0,
        max_depth: 1.0,
    }
}

impl<'d> MediumStage<'d> {
    /// A stage without targets; resolves its six programs in `shaders`.
    ///
    /// # Errors
    /// The pack misses a shader of the stage.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        let storage = SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ;
        Ok(Self {
            pipelines: PipelineCache::new(shaders, &MEDIUM_PROGRAMS)?,
            scatter: RenderTarget::new(gpu),
            filtered: RenderTarget::new(gpu),
            guides: RenderTarget::new(gpu),
            polar: RenderTarget::new(gpu),
            sun_heights: RenderTarget::new(gpu),
            lights: UploadBuffer::new(
                gpu,
                storage,
                INITIAL_LIGHTS * GpuLight::SIZE as u32,
                "medium light",
            ),
            masks: UploadBuffer::new(
                gpu,
                storage,
                INITIAL_LIGHTS * GpuMask::SIZE as u32,
                "medium mask",
            ),
            list: MediumLightList::default(),
            shafts: Vec::new(),
            rows: Vec::new(),
            suns: Vec::new(),
            sun_params: Vec::new(),
            blocks: Vec::new(),
            block: (1, 1),
            query: PolarQueryParams::default(),
            params: [0; MediumParams::SIZE],
            sizes: [0.0; 4],
            base_zones: false,
            prepared: None,
        })
    }

    fn release(&mut self) {
        for t in [
            &mut self.scatter,
            &mut self.filtered,
            &mut self.guides,
            &mut self.polar,
            &mut self.sun_heights,
        ] {
            t.release();
        }
        self.lights.reset();
        self.masks.reset();
    }

    fn pipeline(&self, program: MediumProgram) -> Result<PipelineRef<'_>, Diagnostic> {
        self.pipelines
            .pipeline(index(program), SCATTER_FORMAT)
            .ok_or_else(|| missing("a medium pipeline"))
    }

    fn light_mut(&mut self, i: usize) -> &mut GpuLight {
        let n = self.list.lights.len();
        if i < n {
            &mut self.list.lights[i]
        } else {
            &mut self.list.masked.lights[i - n]
        }
    }

    /// Prepares the frame: `None` (no haze, no medium layer; [`medium_runs`]) releases
    /// everything. Collects the lights and suns in the medium, plans the shaft rows and sun
    /// blocks, creates targets and pipelines, fills the light and mask buffers and brings the
    /// atlases its lights read up to date. Returns the textures created.
    ///
    /// # Errors
    /// A target, buffer or pipeline could not be created, or masked lights have no mask store.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        frame: Option<&MediumStageFrame<'_>>,
        lights: &MediumLights<'_>,
        atlases: &mut LightAtlases<'d>,
    ) -> Result<u32, Diagnostic> {
        self.prepared = None;
        let Some(f) = frame.filter(|f| medium_runs(f.settings, f.layer.is_some())) else {
            self.release();
            return Ok(0);
        };
        let settings = f.settings;
        let p = f.projection;
        let scene = (f.scene.width(), f.scene.height());
        let m = medium_params(
            p,
            scene,
            settings,
            f.surface.is_some(),
            f.layer.is_some(),
            f.media,
        );
        let (w, h) = (m.sizes[2] as u32, m.sizes[3] as u32);
        let mut created = 0;
        for (t, format) in [
            (&mut self.scatter, SCATTER_FORMAT),
            (&mut self.filtered, SCATTER_FORMAT),
            (&mut self.guides, MEDIUM_GUIDE_FORMAT),
        ] {
            created += u32::from(t.ensure(w, h, format)?);
        }
        // Ambient zones (E121): the base pass reads the pure ambient per pixel.
        self.base_zones = f.pure_ambient.is_some();
        let base = if self.base_zones {
            MediumProgram::BaseZones
        } else {
            MediumProgram::Base
        };
        for program in [base, MediumProgram::Filter] {
            self.pipelines
                .get(shaders, index(program), SCATTER_FORMAT)?;
        }

        let top = medium_top(
            f.layer.is_some(),
            settings.resolved_medium_height_step(),
            settings.haze.density,
            lights.modes.max_height,
            m.haze[1],
        );
        self.params = m.to_bytes();
        self.sizes = m.sizes;
        let (store, time, modes) = (lights.store, lights.time, &lights.modes);
        collect_medium_lights(store, p, scene.0, scene.1, top, time, modes, &mut self.list);
        // Masked lights (E115) follow the others and get their own draw.
        let mask_base = self.list.lights.len() as u32;
        let light_count = mask_base + self.list.masked.len() as u32;
        self.shafts.clear();
        self.shafts.extend_from_slice(&self.list.shafts);
        self.shafts.extend_from_slice(&self.list.masked_shafts);
        collect_medium_suns(store, time, modes, &mut self.suns);
        created += self.prepare_suns(shaders, f)?;
        self.rows.clear();
        if light_count > 0 {
            for (program, any) in [
                (MediumProgram::Light, mask_base > 0),
                (MediumProgram::LightMask, light_count > mask_base),
            ] {
                if any {
                    self.pipelines
                        .get(shaders, index(program), SCATTER_FORMAT)?;
                }
            }
            if light_count > mask_base {
                let masks = modes.masks.ok_or_else(|| missing("the mask store"))?;
                self.masks
                    .reserve(u64::from(light_count - mask_base) * GpuMask::SIZE as u64)?;
                created += atlases.masks.prepare(masks)?;
            }
            self.lights
                .reserve(u64::from(light_count) * GpuLight::SIZE as u64)?;
            if let Some(s) = f.shadows {
                created += self.prepare_shafts(f, s)?;
            }
            // A version check makes this a no-op when the light pass prepared it.
            if let Some(profiles) = lights.profiles {
                created += atlases.profiles.prepare(profiles)?;
            }
            let mut mapped = self.lights.map()?;
            let bytes = mapped.bytes();
            let all = self.list.lights.iter().chain(&self.list.masked.lights);
            for (i, l) in all.enumerate() {
                bytes[i * GpuLight::SIZE..][..GpuLight::SIZE].copy_from_slice(&l.to_bytes());
            }
            drop(mapped);
            if light_count > mask_base {
                let mut mapped = self.masks.map()?;
                let bytes = mapped.bytes();
                for (i, m) in self.list.masked.masks.iter().enumerate() {
                    bytes[i * GpuMask::SIZE..][..GpuMask::SIZE].copy_from_slice(&m.to_bytes());
                }
            }
        }
        if self.rows.is_empty() {
            self.polar.release();
        }
        if self.blocks.is_empty() {
            self.sun_heights.release();
        }
        self.prepared = Some(Prepared {
            mask_base,
            light_count,
        });
        Ok(created)
    }

    /// Chiaro `PrepareShafts`: a light has shafts while the light pass built it a row (one of
    /// its caster class, E53).
    fn prepare_shafts(
        &mut self,
        f: &MediumStageFrame<'_>,
        s: &ShadowMaps<'_>,
    ) -> Result<u32, Diagnostic> {
        if !f.settings.haze.shafts {
            return Ok(0);
        }
        let traced = |key: u64| f.rows.row_of(key).is_some();
        plan_shafts(&self.shafts, traced, &mut self.rows);
        for r in 0..self.rows.len() {
            let light = self.rows[r].light as usize;
            self.light_mut(light).shadow[0] = (r + 1) as f32; // row + 1 (medium_light.vert)
        }
        if self.rows.is_empty() {
            return Ok(0);
        }
        // Rows grow, never shrink: a frame with fewer shaft lights keeps the atlas.
        let height = self.polar.height().max(self.rows.len() as u32);
        let created = u32::from(self.polar.ensure(POLAR_ANGLES, height, POLAR_FORMAT)?);
        let [x, y] = mask_rows(f.projection, &s.domain, s.remainder);
        self.query = PolarQueryParams {
            mask_x: [x[0], x[1], x[2], 0.0],
            mask_y: [y[0], y[1], y[2], 0.0],
            atlas: [
                POLAR_ANGLES as f32,
                self.polar.height() as f32,
                height_tolerance(f.settings),
                0.0,
            ],
        };
        Ok(created)
    }

    /// Chiaro `PrepareSuns`: every directional light with haze scatters in one fullscreen draw;
    /// one with shadows in height mode (E24) gets a block of sun shadow heights when the frame
    /// has heights, shafts are on, (E53) its caster class has heights and the light pass gave
    /// it a map (or it stands straight overhead). Returns the textures created.
    fn prepare_suns(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        f: &MediumStageFrame<'_>,
    ) -> Result<u32, Diagnostic> {
        self.sun_params.clear();
        self.blocks.clear();
        if self.suns.is_empty() {
            return Ok(0);
        }
        self.pipelines
            .get(shaders, index(MediumProgram::Directional), SCATTER_FORMAT)?;
        let shadows = f.shadows.map(|s| SunShadows {
            domain: s.domain,
            scene_max_top: s.scene_max_top,
        });
        // `height_mode`, not `height`: at prepare the maps hold the last committed frame, and
        // this frame's heights exist only from its record on (the first frame had none).
        let shafts = f.settings.haze.shafts
            && f.shadows
                .is_some_and(|s| s.height_mode && s.scene_max_top > 0.0);
        let has_heights = |sun: &MediumSun| {
            let class = sun.casters.filter_index().is_none_or(|i| {
                f.shadows
                    .and_then(|s| s.casters[i])
                    .is_some_and(|m| m.height.is_some())
            });
            // From its map, or straight overhead from the heights.
            let overhead = sun.to_light.x == 0.0 && sun.to_light.y == 0.0;
            class && (overhead || f.rows.has_sun_map(sun.key))
        };
        self.block = plan_suns(
            &self.suns,
            f.settings,
            shadows.as_ref(),
            shafts,
            has_heights,
            &mut self.sun_params,
            &mut self.blocks,
        );
        let (Some(s), false) = (f.shadows, self.blocks.is_empty()) else {
            return Ok(0);
        };
        // Grows, never shrinks: the blocks are addressed with the atlas size.
        let width = self.sun_heights.width().max(self.block.0);
        let height = self
            .sun_heights
            .height()
            .max(self.block.1 * self.blocks.len() as u32);
        let created = u32::from(self.sun_heights.ensure(width, height, SUN_HEIGHT_FORMAT)?);
        finish_suns(
            &mut self.sun_params,
            mask_rows(f.projection, &s.domain, s.remainder),
            &s.domain,
            (self.sun_heights.width(), self.sun_heights.height()),
            self.block,
        );
        Ok(created)
    }

    /// The prepared frame runs the stage.
    pub fn is_active(&self) -> bool {
        self.prepared.is_some()
    }

    /// The composite's input of the prepared frame; `None` if the stage does not run.
    pub fn composite_medium<'a>(
        &'a self,
        layer: Option<TextureRef<'a>>,
    ) -> Option<CompositeMedium<'a>> {
        self.prepared?;
        Some(CompositeMedium {
            scatter: self.filtered.texture_ref()?,
            guides: self.guides.texture_ref()?,
            layer,
            params: &self.params,
        })
    }

    /// The scatter buffer before the pre-filter (tests and tools).
    pub fn scatter(&self) -> Option<TextureRef<'_>> {
        self.scatter.texture_ref()
    }

    /// The pre-filtered scatter buffer the composite reads.
    pub fn filtered(&self) -> Option<TextureRef<'_>> {
        self.filtered.texture_ref()
    }

    /// The guides of the pre-filtered texels.
    pub fn guides(&self) -> Option<TextureRef<'_>> {
        self.guides.texture_ref()
    }

    /// The polar atlas of the last frame with shaft rows.
    pub fn polar_atlas(&self) -> Option<TextureRef<'_>> {
        self.polar.texture_ref()
    }

    /// The sun height atlas of the last frame with sun blocks.
    pub fn sun_heights(&self) -> Option<TextureRef<'_>> {
        self.sun_heights.texture_ref()
    }

    /// Pipelines created so far.
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// Records the prepared frame: light upload, shaft rows, sun blocks, the scatter pass
    /// (base, suns, lights) and the pre-filter. Nothing if the stage does not run. `frame`
    /// must be the one given to [`prepare`](Self::prepare); the atlases' uploads were recorded
    /// before (the pass plan records them ahead of the light pass).
    ///
    /// # Errors
    /// A pass or binding failed.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &MediumStageFrame<'_>,
        atlases: &LightAtlases<'_>,
    ) -> Result<MediumStageRecord, Diagnostic> {
        let Some(prep) = self.prepared else {
            return Ok(MediumStageRecord::default());
        };
        let (mask_base, light_count) = (prep.mask_base, prep.light_count);
        let mut out = MediumStageRecord {
            lights: light_count + self.suns.len() as u32,
            shaft_rows: self.rows.len() as u32,
            sun_blocks: self.blocks.len() as u32,
            draws: 0,
        };
        if light_count > 0 {
            let mut copy = cmd.copy_pass()?;
            copy.upload_to_buffer(&self.lights, light_count * GpuLight::SIZE as u32)?;
            if light_count > mask_base {
                copy.upload_to_buffer(
                    &self.masks,
                    (light_count - mask_base) * GpuMask::SIZE as u32,
                )?;
            }
        }
        let nearest = samplers.sampler(Filter::Nearest);
        let linear = samplers.sampler(Filter::Linear);
        // Shaft rows and sun blocks exist only with shadows.
        if frame.shadows.is_some() && !self.rows.is_empty() {
            out.draws += self.record_shafts(cmd, samplers, frame.rows)?;
        }
        if let (Some(s), false) = (frame.shadows, self.blocks.is_empty()) {
            out.draws += self.record_sun_heights(cmd, samplers, s, frame.rows)?;
        }
        let scene = frame.scene;
        let no_map = MediumMapParams::default().to_bytes();
        // ZoneParams: the flat directional sum D of the zones.
        let d = frame.zone_directional;
        let mut zone = [0u8; 16];
        for (i, v) in [d.x, d.y, d.z].into_iter().enumerate() {
            zone[i * 4..][..4].copy_from_slice(&v.to_le_bytes());
        }
        let query = self.query.to_bytes();
        // Everything the programs can read, by name; each draw binds what its layout names. A
        // missing buffer's slot holds the scene and is not read.
        let mut res = Resources::new(SamplerBinding::new(scene, nearest));
        res.set(
            LightSampler::Surface,
            Some(frame.surface.unwrap_or(scene)),
            nearest,
        );
        res.set(
            LightSampler::MediumTex,
            Some(frame.layer.unwrap_or(scene)),
            nearest,
        );
        res.set(
            LightSampler::Profile,
            Some(atlases.profiles.texture().unwrap_or(scene)),
            nearest,
        );
        res.set(
            LightSampler::MediumMap,
            Some(frame.medium_map.map_or(scene, |m| m.map)),
            nearest,
        );
        res.set(
            LightSampler::PolarAtlas,
            Some(if self.rows.is_empty() {
                scene
            } else {
                self.polar
                    .texture_ref()
                    .ok_or_else(|| missing("the polar atlas"))?
            }),
            nearest,
        );
        res.set(
            LightSampler::SunHeights,
            Some(if self.blocks.is_empty() {
                scene
            } else {
                self.sun_heights
                    .texture_ref()
                    .ok_or_else(|| missing("the sun height atlas"))?
            }),
            linear,
        );
        res.set(LightSampler::PureAmbient, frame.pure_ambient, linear);
        res.uniforms[LightUniform::MediumParams as usize] = Some(&self.params);
        res.uniforms[LightUniform::MediumMap as usize] =
            Some(frame.medium_map.map_or(&no_map[..], |m| &m.params[..]));
        res.uniforms[LightUniform::PolarQuery as usize] = Some(&query);
        res.uniforms[LightUniform::Zone as usize] = Some(&zone);

        let scatter = self
            .scatter
            .texture_ref()
            .ok_or_else(|| missing("the scatter buffer"))?;
        {
            let mut pass =
                cmd.render_pass(&[ColorTarget::new(scatter, Load::DontCare).cycled(true)])?;
            let base = if self.base_zones {
                MediumProgram::BaseZones
            } else {
                MediumProgram::Base
            };
            let layout = medium_layout_of(base);
            bind_named(
                &mut pass,
                self.pipeline(base)?,
                &base,
                &layout,
                &res,
                None,
                None,
            )?;
            pass.draw_fullscreen()?;
            out.draws += 1;
            // Directional lights: one fullscreen draw each.
            if !self.sun_params.is_empty() {
                let program = MediumProgram::Directional;
                let layout = medium_layout_of(program);
                let pipeline = self.pipeline(program)?;
                for sp in &self.sun_params {
                    let sun = sp.to_bytes();
                    let extra = Some((LightUniform::Sun, &sun[..]));
                    bind_named(&mut pass, pipeline, &program, &layout, &res, None, extra)?;
                    pass.draw_fullscreen()?;
                    out.draws += 1;
                }
            }
            if light_count > 0 {
                let p = frame.projection;
                let o = p.origin.raw();
                let params = MediumLightPassParams {
                    origin_x_axis: [o.x, o.y, p.x_axis.x, p.x_axis.y],
                    y_axis_z_axis: [p.y_axis.x, p.y_axis.y, p.z_axis.x, p.z_axis.y],
                    sizes: self.sizes,
                };
                res.storage[LightStorage::Lights as usize] = self.lights.buffer_ref();
                if mask_base > 0 {
                    let program = MediumProgram::Light;
                    let vertex = params.to_bytes();
                    let layout = medium_layout_of(program);
                    let pipeline = self.pipeline(program)?;
                    bind_named(
                        &mut pass,
                        pipeline,
                        &program,
                        &layout,
                        &res,
                        Some(&vertex),
                        None,
                    )?;
                    pass.draw_instanced(6, mask_base)?;
                    out.draws += 1;
                }
                if light_count > mask_base {
                    // The masked lights (E115): the same resources plus the mask atlas and
                    // the masks.
                    let program = MediumProgram::LightMask;
                    let vertex = MediumMaskLightPassParams {
                        base: params,
                        light_offset: [mask_base, 0, 0, 0],
                    }
                    .to_bytes();
                    res.set(LightSampler::Mask, atlases.masks.texture(), linear);
                    res.storage[LightStorage::Masks as usize] = self.masks.buffer_ref();
                    let layout = medium_layout_of(program);
                    let pipeline = self.pipeline(program)?;
                    bind_named(
                        &mut pass,
                        pipeline,
                        &program,
                        &layout,
                        &res,
                        Some(&vertex),
                        None,
                    )?;
                    pass.draw_instanced(6, light_count - mask_base)?;
                    out.draws += 1;
                }
            }
        }
        // Pre-filter (E106): the filtered scatter buffer and the guides, read by the composite.
        let filtered = self
            .filtered
            .texture_ref()
            .ok_or_else(|| missing("the filtered buffer"))?;
        let guides = self
            .guides
            .texture_ref()
            .ok_or_else(|| missing("the guides"))?;
        let mut pass = cmd.render_pass(&[
            ColorTarget::new(filtered, Load::DontCare).cycled(true),
            ColorTarget::new(guides, Load::DontCare).cycled(true),
        ])?;
        res.set(LightSampler::ScatterTex, Some(scatter), nearest);
        let program = MediumProgram::Filter;
        let layout = medium_layout_of(program);
        bind_named(
            &mut pass,
            self.pipeline(program)?,
            &program,
            &layout,
            &res,
            None,
            None,
        )?;
        pass.draw_fullscreen()?;
        out.draws += 1;
        Ok(out)
    }

    /// One pass, one draw per row: the row's viewport, the light's row of the light pass
    /// converted to the polar atlas (`RowLights::record_polar`). Rows past this frame's count
    /// keep stale content; no light reads them.
    fn record_shafts(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        rows: &RowLights<'_>,
    ) -> Result<u32, Diagnostic> {
        let polar = self
            .polar
            .texture_ref()
            .ok_or_else(|| missing("the polar atlas"))?;
        let mut pass = cmd.render_pass(&[ColorTarget::new(polar, Load::DontCare)])?;
        for (r, row) in self.rows.iter().enumerate() {
            let light = self.light(row.light as usize);
            let length = polar_row(&self.query, light)[3];
            pass.set_viewport(&viewport(0.0, r as f32, POLAR_ANGLES as f32, 1.0));
            let height = light.position_radius[2];
            rows.record_polar(&mut pass, samplers, row.key, height, length)?;
        }
        Ok(self.rows.len() as u32)
    }

    /// One pass, one draw per block: the block's viewport, the sun's map of the light pass
    /// converted to shadow heights (`RowLights::record_sun_height`).
    fn record_sun_heights(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        s: &ShadowMaps<'_>,
        rows: &RowLights<'_>,
    ) -> Result<u32, Diagnostic> {
        let target = self
            .sun_heights
            .texture_ref()
            .ok_or_else(|| missing("the sun height atlas"))?;
        let mut pass = cmd.render_pass(&[ColorTarget::new(target, Load::DontCare)])?;
        let (bw, bh) = self.block;
        for (b, block) in self.blocks.iter().enumerate() {
            pass.set_viewport(&viewport(0.0, (b as u32 * bh) as f32, bw as f32, bh as f32));
            let key = self.suns[block.sun as usize].key;
            rows.record_sun_height(&mut pass, samplers, s, key, block.casters)?;
        }
        Ok(self.blocks.len() as u32)
    }

    fn light(&self, i: usize) -> &GpuLight {
        let n = self.list.lights.len();
        if i < n {
            &self.list.lights[i]
        } else {
            &self.list.masked.lights[i - n]
        }
    }
}
