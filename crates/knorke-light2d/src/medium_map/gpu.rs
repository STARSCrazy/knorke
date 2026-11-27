// Origin: chiaro @ 9bc5b9b, src/rendering/MediumMapStage.hpp and MediumMapStage.cpp (Prepare,
// Record), src/gpu/PipelineCache.cpp (MediumWorld, MediumWorldTop, MediumWorldResolve,
// kMediumSplatFormat).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

use knorke_gpu::sys::gpu::{
    SDL_GPU_PRIMITIVETYPE_POINTLIST, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT,
    SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Load, PipelineCache, ProgramDesc, RenderTarget,
    SamplerBinding, SamplerRef, SecondTarget, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{MediumMapLayout, MediumMapParams, MediumParams, medium_params};
use crate::Projection;
use crate::medium_table::MediumTable;
use crate::settings::LightSettings;
use crate::shadow::{MaskAnchor, OcclusionDomain};

/// Recording without a prepare that built the map, or a pipeline is missing after prepare.
pub const MEDIUM_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.medium.not_prepared");

/// The medium layer is not 8-bit RGBA UNORM (its bytes are the codec's `medium_pixel`).
pub const MEDIUM_LAYER_FORMAT: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.medium.layer_format");

/// Format of the map: `(sigma, bottom, top, highest top)` per cell.
pub const MEDIUM_MAP_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// Format of both splat targets: 32-bit, because a cell at a low shadow resolution takes hundreds
/// of points (fp16 counts stop at 2048 and sums overflow at 65504).
pub const MEDIUM_SPLAT_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT;
/// Format of the 2 x 1 target with the highest top and the cell rectangle.
pub const MEDIUM_TOP_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT;

/// The map passes, in [`MediumMapPrograms`] order: splat (points into the sums, ONE + ONE, and
/// the slab union, MAX), top (points into 2 x 1, MAX), resolve (fullscreen, opaque).
pub const MEDIUM_MAP_PROGRAMS: [ProgramDesc; 3] = [
    ProgramDesc {
        vertex: "medium_world.vert",
        fragment: "medium_world.frag",
        blend: Blend::Additive,
        second: Some(SecondTarget {
            format: MEDIUM_SPLAT_FORMAT,
            blend: Blend::Max,
        }),
        more: &[],
        primitive: SDL_GPU_PRIMITIVETYPE_POINTLIST,
    },
    ProgramDesc {
        vertex: "medium_world_top.vert",
        fragment: "medium_world_top.frag",
        blend: Blend::Max,
        second: None,
        more: &[],
        primitive: SDL_GPU_PRIMITIVETYPE_POINTLIST,
    },
    ProgramDesc::new(
        "fullscreen.vert",
        "medium_world_resolve.frag",
        Blend::Opaque,
    ),
];

/// Indices into [`MEDIUM_MAP_PROGRAMS`].
#[derive(Clone, Copy, Debug)]
pub struct MediumMapPrograms;

impl MediumMapPrograms {
    /// Splat.
    pub const SPLAT: usize = 0;
    /// Highest top and cell rectangle.
    pub const TOP: usize = 1;
    /// Resolve into the map.
    pub const RESOLVE: usize = 2;
}

/// What the `*_medium` light programs and the medium stage (P30) bind: the map and its block.
#[derive(Clone, Copy, Debug)]
pub struct MediumMapView<'a> {
    /// The map ([`MEDIUM_MAP_FORMAT`], nearest).
    pub map: TextureRef<'a>,
    /// The `MediumMapParams` block.
    pub params: &'a [u8; MediumMapParams::SIZE],
}

/// The inputs of one frame with a medium layer (Chiaro `FrameInputs` as far as the stage reads
/// it); the grid is the occlusion stage's when it runs this frame, otherwise the same kind of
/// grid (`OcclusionDomain::new`, `MaskAnchor::compute`).
#[derive(Clone, Copy, Debug)]
pub struct MediumMapFrame<'a> {
    /// The medium layer (codec `medium_pixel`, RGBA8 UNORM, scene size).
    pub layer: TextureRef<'a>,
    /// The media table the layer picks from.
    pub media: &'a MediumTable,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Settings: medium height step, quality, haze, ambient.
    pub settings: &'a LightSettings,
    /// The frame has a surface buffer.
    pub surface: bool,
    /// The occlusion mask's domain.
    pub domain: OcclusionDomain,
    /// Its anchor.
    pub anchor: MaskAnchor,
}

/// The medium world map (Chiaro `MediumMapStage`, see the [module docs](super)): three passes
/// into a map at a quarter of the occlusion mask's resolution. A frame without a medium layer
/// (or without lights) releases everything: no pass, no binding, nothing held.
///
/// Per frame: [`prepare`](Self::prepare) after the occlusion stage, [`record`](Self::record)
/// before the light pass, then [`view`](Self::view) for the light pass.
#[derive(Debug)]
pub struct MediumMapStage<'d> {
    pipelines: PipelineCache<'d>,
    /// Splat: `(sum of sigma, points)` per cell.
    sums: RenderTarget<'d>,
    /// Splat: `(highest top, -lowest bottom)` per cell.
    union: RenderTarget<'d>,
    /// 2 x 1: the highest top and the cell rectangle of the frame.
    top: RenderTarget<'d>,
    map: RenderTarget<'d>,
    /// `Some` after a prepare that builds the map.
    layout: Option<MediumMapLayout>,
    medium_params: [u8; MediumParams::SIZE],
    map_params: [u8; MediumMapParams::SIZE],
}

fn missing(what: &str) -> Diagnostic {
    Diagnostic::error(
        MEDIUM_NOT_PREPARED,
        format!("{what} is missing after prepare"),
    )
}

impl<'d> MediumMapStage<'d> {
    /// A stage without targets; resolves its three programs in `shaders` (pipelines are created
    /// by the first frame with a medium layer).
    ///
    /// # Errors
    /// The pack misses a shader of the map passes.
    pub fn new(shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        let gpu = shaders.gpu();
        Ok(Self {
            pipelines: PipelineCache::new(shaders, &MEDIUM_MAP_PROGRAMS)?,
            sums: RenderTarget::new(gpu),
            union: RenderTarget::new(gpu),
            top: RenderTarget::new(gpu),
            map: RenderTarget::new(gpu),
            layout: None,
            medium_params: [0; MediumParams::SIZE],
            map_params: [0; MediumMapParams::SIZE],
        })
    }

    fn release(&mut self) {
        self.sums.release();
        self.union.release();
        self.top.release();
        self.map.release();
        self.layout = None;
    }

    /// Prepares the frame: `None` (no medium layer or no light, [`medium_map_builds`]) releases
    /// everything. Returns the textures created.
    ///
    /// # Errors
    /// The layer is not 8-bit RGBA UNORM ([`MEDIUM_LAYER_FORMAT`]), or a target or pipeline could
    /// not be created.
    ///
    /// [`medium_map_builds`]: super::medium_map_builds
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        frame: Option<&MediumMapFrame<'_>>,
    ) -> Result<u32, Diagnostic> {
        self.layout = None;
        let Some(f) = frame else {
            self.release();
            return Ok(0);
        };
        let format = f.layer.format();
        if format != SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM
            && format != SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM
        {
            return Err(Diagnostic::error(
                MEDIUM_LAYER_FORMAT,
                "the medium layer must be 8-bit RGBA UNORM (codec medium_pixel)",
            ));
        }
        let layout = MediumMapLayout::new(
            f.projection,
            &f.domain,
            &f.anchor,
            f.settings.resolved_medium_height_step(),
        );
        let (w, h) = (layout.width, layout.height);
        let mut created = 0;
        for (target, size, format) in [
            (&mut self.sums, (w, h), MEDIUM_SPLAT_FORMAT),
            (&mut self.union, (w, h), MEDIUM_SPLAT_FORMAT),
            (&mut self.map, (w, h), MEDIUM_MAP_FORMAT),
            (&mut self.top, (2, 1), MEDIUM_TOP_FORMAT),
        ] {
            if target.ensure(size.0, size.1, format)? {
                created += 1;
            }
        }
        for (program, format) in [
            (MediumMapPrograms::SPLAT, MEDIUM_SPLAT_FORMAT),
            (MediumMapPrograms::TOP, MEDIUM_TOP_FORMAT),
            (MediumMapPrograms::RESOLVE, MEDIUM_MAP_FORMAT),
        ] {
            self.pipelines.get(shaders, program, format)?;
        }
        let scene = (f.layer.width(), f.layer.height());
        self.medium_params =
            medium_params(f.projection, scene, f.settings, f.surface, true, f.media).to_bytes();
        self.map_params = layout.params.to_bytes();
        self.layout = Some(layout);
        Ok(created)
    }

    /// The grid of the prepared frame; `None` if it builds no map.
    pub fn layout(&self) -> Option<&MediumMapLayout> {
        self.layout.as_ref()
    }

    /// Records the three passes; nothing without a map this frame. `layer` is the frame's medium
    /// layer, `nearest` the host's nearest sampler.
    ///
    /// # Errors
    /// A pass or binding failed.
    pub fn record(
        &self,
        cmd: &mut CommandBuffer<'_>,
        nearest: SamplerRef<'_>,
        layer: TextureRef<'_>,
    ) -> Result<(), Diagnostic> {
        let Some(layout) = &self.layout else {
            return Ok(());
        };
        let pipeline = |program, format| {
            self.pipelines
                .pipeline(program, format)
                .ok_or_else(|| missing("a medium map pipeline"))
        };
        let sums = self.sums.texture_ref().ok_or_else(|| missing("the sums"))?;
        let union = self
            .union
            .texture_ref()
            .ok_or_else(|| missing("the union"))?;
        let top = self.top.texture_ref().ok_or_else(|| missing("the top"))?;
        let map = self.map.texture_ref().ok_or_else(|| missing("the map"))?;
        let (bx, by) = layout.blocks();

        // Every block (2 x 2 per map texel) splats one point: the sums cleared to 0, the union to
        // "no slab" for MAX.
        {
            let mut pass = cmd.render_pass(&[
                ColorTarget::new(sums, Load::Clear([0.0; 4])).cycled(true),
                ColorTarget::new(union, Load::Clear([-1e30, -1e30, 0.0, 0.0])).cycled(true),
            ])?;
            let splat = layout.splat_params().to_bytes();
            let samplers = [SamplerBinding::new(layer, nearest)];
            pass.bind_program(
                pipeline(MediumMapPrograms::SPLAT, MEDIUM_SPLAT_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        samplers: &samplers,
                        uniforms: &[&self.medium_params, &splat],
                        ..Default::default()
                    },
                    fragment: StageBindings::default(),
                },
            )?;
            pass.draw_instanced(bx * by, 1)?;
        }
        // The highest top and the cell rectangle: two points per cell into 2 x 1, cleared to 0.
        {
            let mut pass =
                cmd.render_pass(&[ColorTarget::new(top, Load::Clear([0.0; 4])).cycled(true)])?;
            let params = layout.top_params().to_bytes();
            let samplers = [SamplerBinding::new(union, nearest)];
            pass.bind_program(
                pipeline(MediumMapPrograms::TOP, MEDIUM_TOP_FORMAT)?,
                &Bindings {
                    vertex: StageBindings {
                        samplers: &samplers,
                        uniforms: &[&params],
                        ..Default::default()
                    },
                    fragment: StageBindings::default(),
                },
            )?;
            pass.draw_instanced(2 * layout.width * layout.height, 1)?;
        }
        // Sums, union and the highest top into the map.
        let mut pass = cmd.render_pass(&[ColorTarget::new(map, Load::DontCare).cycled(true)])?;
        let params = layout.resolve_params().to_bytes();
        let samplers = [
            SamplerBinding::new(sums, nearest),
            SamplerBinding::new(union, nearest),
            SamplerBinding::new(top, nearest),
        ];
        pass.bind_program(
            pipeline(MediumMapPrograms::RESOLVE, MEDIUM_MAP_FORMAT)?,
            &Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &samplers,
                    uniforms: &[&params],
                    ..Default::default()
                },
            },
        )?;
        pass.draw_fullscreen()
    }

    /// The map and its block for the light pass; `None` if the frame builds none.
    pub fn view(&self) -> Option<MediumMapView<'_>> {
        self.layout?;
        Some(MediumMapView {
            map: self.map.texture_ref()?,
            params: &self.map_params,
        })
    }

    /// Pipelines created so far.
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }
}
