//! Measuring tool for the light rows (ADR 0019, plan 17.13): picture and frame time of the row
//! lights of `knorke-light2d` against a reference picture, on
//! `iso_night_hard` cut down to what rows can draw: the point and spot lights that cast shadows
//! (13), no profile, mask, pattern, animation, flicker, medium, sun, zones, material. Hard
//! shadows, and soft ones with the lamp sizes of `plain.txt`; each as it is ("normal") and with
//! every lamp four times ("last"); the line and rect lights that cast shadows (4) alone
//! ("flächen"); the shop sign's line light alone with a post close in front of it, a third
//! along ("linie pfosten"); the point and spot lights with the demo's two pattern fences, hard
//! and soft ("zaun"), and as tile grids ("gitter"); the walls in a bank of fog ("nebel", the
//! medium world map dims every light on its way); every obstacle three times as tall, beyond
//! the rows' old height limit ("hoch"); every lamp 20 times, 260 rows ("viele"); every lamp on a
//! mast it stands in (`inside_obstacle`, "mast"); every lamp's shadow from the static obstacles
//! alone, which are all of them (a caster filter, "filter"); every lamp reaching 1200 mask
//! texels, its row of 2048 angles ("weit").
//!
//! - New: [`Light2dRenderer`].
//! - True: the same light with an exact march per pixel (`lightrow_truth.frag`), the only part
//!   this file draws itself; a lamp with a size and a line or rect light's emitter are
//!   `KNORKE_PROBE_POINTS` squared points (default 8).
//!
//! The test checks the rows against the reference within [`BOUNDS`].
//! `KNORKE_PROBE_OUT=<dir>` writes `<scene>_neu.png`, `<scene>_wahr.png`, the difference times 8
//! and `messung.txt`; `KNORKE_PROBE_FRAMES=<n>` times n frames per run (wall clock: prepare +
//! record + submit, in batches of [`BATCH`] frames that wait once at their end, so the GPU stays
//! busy), `KNORKE_PROBE_RUNS` runs per case (default 5), the cases in turn: rows with resting
//! lamps, rows with every lamp moved every frame (all rows built), a frame without lights (what
//! no light stage can save) and the camera scrolling. `KNORKE_PROBE_ONLY=<prefix>` runs only the
//! lamp cases whose name begins so.
//!
//! [`sun_map_against_the_reference`] does the same for the moon of the demo with its shadow from
//! a sun map (plan 17.13, L3).
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

use std::fmt::Write as _;
use std::path::PathBuf;

use knorke_contract_tests::parity::save_png_rgba8;
use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUTextureFormat,
};
use knorke_gpu::test_support::{golden_bytes, read_texture, texture, with_chiaro_pack};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, ProgramDesc,
    SamplerBinding, Samplers, ShaderLibrary, StageBindings, Texture, UploadBuffer,
};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_light2d::codec::encode_medium_pixel;
use knorke_light2d::composite::{CompositeFrame, CompositeLight, CompositeStage};
use knorke_light2d::light::{Height, LightDesc, LightId, LightType, ShadowFilter};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::lighting::{
    LightSurfacePassParams, ShadowDomain, SurfaceParams, ambient_clear, height_tolerance,
};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::pass_plan::{PassRecord, SceneStores, max_height};
use knorke_light2d::pattern_occluders::PatternParams;
use knorke_light2d::pattern_store::{PatternDesc, PatternStore, Sheet, SheetFormat, raw_id};
use knorke_light2d::projection::Projection;
use knorke_light2d::rows::{RowLamp, RowLookupParams, RowSpace, SUPPORTS, SunParams};
use knorke_light2d::settings::{GlowColor, LightSettings, Quality, QualitySettings, Resolutions};
use knorke_light2d::shadow::{CellRect, MaskAnchor, OcclusionDomain};
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::output_rect::ScaleMode;
use knorke_post::pass::{PostFrame, PostRenderer};
use knorke_post::settings::{Filter as PostFilter, PostSettings};
use knorke_types::Diagnostic;

const W: u32 = 640;
const H: u32 = 360;
const OUT: (u32, u32) = (1280, 720);
const HEIGHT_STEP: f32 = 0.1;
const HALF: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// World units every lamp moves to and fro in the case that builds all rows.
const NUDGE: f32 = 0.02;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn num(s: &str) -> f32 {
    s.parse().unwrap()
}

/// What [`parse`] makes of the demo's two pattern fences.
#[derive(Clone, Copy, Debug)]
enum Fences {
    /// Walls of this opacity without their patterns.
    Walls(f32),
    /// The fences with their patterns.
    Patterns,
    /// Tile grids with patterns in their place ([`fence_grid`]).
    Grids,
    /// Opaque walls without their patterns, every obstacle this many times as tall.
    Tall(f32),
}

/// The scene text cut down to what the probe compares: the enabled lights of the types in
/// `kinds` (Chiaro's numbers: 0 point, 1 spot, 2 directional, 3 line, 4 rect) that cast shadows (every extra
/// switched off) and the enabled obstacles, the two pattern fences as `fence` says.
fn parse(
    text: &str,
    soft: bool,
    fence: Fences,
    kinds: &str,
) -> (Vec<LightDesc>, ObstacleStore, PatternStore) {
    let (mut lights, mut obstacles) = (Vec::new(), ObstacleStore::new());
    let (mut sheet, mut descs, mut fences) = (Vec::new(), Vec::new(), Vec::new());
    let mut grids = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let v = |i: usize| num(f[i]);
        match f[0] {
            "light" => {
                let kind = match f[1] {
                    "0" => LightType::Point,
                    "1" => LightType::Spot,
                    "2" => LightType::Directional,
                    "3" => LightType::Line,
                    _ => LightType::Rect,
                };
                // CHI_LIGHT_NO_SHADOWS, _DISABLED; CHI_HEIGHT_AUTO is -1 (the moon's, the only light
                // without a height that is kept).
                let auto = v(5) < 0.0;
                let skip = auto && kind != LightType::Directional;
                if !kinds.contains(f[1]) || f[2].parse::<u32>().unwrap() & 3 != 0 || skip {
                    continue;
                }
                let mut d = LightDesc::new(kind);
                d.position = At::new(Vec2::new(v(3), v(4)));
                d.height = if auto { Height::Auto } else { Height::At(v(5)) };
                d.color = Vec3::new(v(6), v(7), v(8));
                d.intensity = v(9);
                d.specular = 0.0;
                d.haze = 0.0;
                d.radius = v(12);
                d.falloff = v(13);
                d.source_radius = if soft { v(14) } else { 0.0 };
                d.flicker = 0.0;
                d.direction = Vec3::new(v(17), v(18), v(19));
                d.inner_angle = v(20);
                d.outer_angle = v(21);
                d.two_sided = f[2].parse::<u32>().unwrap() & 4 != 0;
                d.axis = Vec3::new(v(22), v(23), v(24));
                d.half_width = v(25);
                lights.push(d);
            }
            "shape" => {
                // CHI_OBSTACLE_DYNAMIC, _DISABLED, _FOOTPRINT_SHADOW.
                let flags: u32 = f[2].parse().unwrap();
                let tall = if let Fences::Tall(k) = fence { k } else { 1.0 };
                let common = ObstacleCommon {
                    position: At::new(Vec2::new(v(3), v(4))),
                    rotation: v(5),
                    height: v(6) * tall,
                    base_height: v(7) * tall,
                    opacity: match fence {
                        Fences::Walls(o) if f[12] != "0" => o,
                        _ => 1.0,
                    },
                    // "filter": every obstacle static, so the static ones alone are all.
                    dynamic: flags & 1 != 0 && !kinds.ends_with('s'),
                    enabled: flags & 2 == 0,
                    shadow_footprint: false,
                };
                let shape = match f[1] {
                    "1" => Shape::Circle { radius: v(9) },
                    _ => Shape::Box {
                        width: v(10),
                        depth: v(11),
                    },
                };
                let pattern = f[12].parse::<usize>().unwrap().checked_sub(1);
                if let (Some(k), Fences::Grids) = (pattern, fence) {
                    grids.push((common, Vec2::new(v(10), v(11)), k));
                    continue;
                }
                let id = obstacles
                    .create_shape(&ShapeDesc { common, shape })
                    .unwrap();
                if let (Some(k), Fences::Patterns) = (pattern, fence) {
                    fences.push((id, k));
                }
            }
            "row" => {
                for k in 0..f[1].len() / 2 {
                    let alpha = u8::from_str_radix(&f[1][2 * k..2 * k + 2], 16).unwrap();
                    sheet.extend_from_slice(&[255, 255, 255, alpha]);
                }
            }
            "pattern" => descs.push(PatternDesc {
                repeat_width: v(5),
                repeat_height: v(6),
                ..PatternDesc::new(v(1) as u32, v(2) as u32, v(3) as u32, v(4) as u32)
            }),
            _ => {}
        }
    }
    let mut patterns = PatternStore::new();
    if !fences.is_empty() || !grids.is_empty() {
        // `sheet 20 16` of the demo.
        let sheet = Sheet {
            width: 20,
            height: 16,
            pitch: 80,
            format: SheetFormat::Rgba8,
            pixels: &sheet,
        };
        let mut ids = Vec::new();
        patterns.create(&sheet, &descs, &mut ids).unwrap();
        for (id, k) in fences {
            obstacles
                .set_pattern(id, raw_id(ids[k]), &patterns)
                .unwrap();
        }
        let ids: Vec<u32> = ids.into_iter().map(raw_id).collect();
        for (common, size, k) in grids {
            fence_grid(&mut obstacles, &patterns, common, size, &ids, k);
        }
    }
    (lights, obstacles, patterns)
}

/// Fence `k` of the demo (box of `size`, rotation 0) as a tile grid with patterns over the same
/// ground: the picket fence (0) a column of cells 0.8 long, every fourth empty (a gap), every
/// third with the tower's pattern from the grid's table; the lattice tower (1) four hollow cells
/// (edge strips 0.1 wide) whose tops lie 9, 6, 9 and 4 levels of 1 up, the third half
/// transparent. So the rows and the map meet a grid's cells, table, strips, levels and cell
/// opacities.
fn fence_grid(
    obstacles: &mut ObstacleStore,
    patterns: &PatternStore,
    mut common: ObstacleCommon,
    size: Vec2,
    ids: &[u32],
    k: usize,
) {
    assert_eq!(
        common.rotation, 0.0,
        "the demo's fences stand along the axes"
    );
    common.position = At::new(common.position.raw() - size * 0.5);
    let id = if k == 0 {
        let rows = (size.y / 0.8).round() as u32;
        let solid: Vec<u8> = (0..rows).map(|i| u8::from(i % 4 != 3)).collect();
        let mut grid = TileGridDesc::new(1, rows, &solid);
        grid.common = common;
        grid.cell_size = Vec2::new(size.x, size.y / rows as f32);
        let id = obstacles.create_tile_grid(&grid).unwrap();
        obstacles
            .set_tile_pattern_table(id, &[ids[1]], patterns)
            .unwrap();
        let cells: Vec<u8> = (0..rows).map(|i| u8::from(i % 3 == 2)).collect();
        let all = CellRect {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: rows,
        };
        obstacles.set_tile_pattern_cells(id, all, &cells).unwrap();
        id
    } else {
        let (solid, opacity, edges) = ([9, 6, 9, 4], [255, 255, 128, 255], [15; 4]);
        let mut grid = TileGridDesc::new(2, 2, &solid);
        grid.common = common;
        grid.cell_size = size * 0.5;
        grid.level_height = 1.0;
        grid.edge_width = 0.1;
        grid.opacity = Some(&opacity);
        grid.edges = Some(&edges);
        obstacles.create_tile_grid(&grid).unwrap()
    };
    obstacles.set_pattern(id, ids[k], patterns).unwrap();
}

/// The demo's settings (`gpu_iso_night.rs`) without haze, at mask resolution `shadow`.
fn settings(shadow: f32) -> Light2dSettings {
    let mut s = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.03, 0.035, 0.05),
            quality: QualitySettings {
                quality: Quality::Custom(Resolutions {
                    light: 1.0,
                    shadow,
                    medium: 0.5,
                }),
                ..QualitySettings::default()
            },
            surface_height_step: HEIGHT_STEP,
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    s.light.glow.scale = 2.5;
    s.light.glow.color = GlowColor::Scene;
    s.post.output.scale_mode = ScaleMode::Integer;
    s.post.output.filter = PostFilter::Nearest;
    s.post.bloom.enabled = true;
    s.post.bloom.intensity = 0.8;
    s.post.bloom.radius = 12.0;
    s
}

/// The reference's programs: the lamps' and the hard sun's, then both through the medium world
/// map (`_medium`).
const TRUTH: [ProgramDesc; 4] = [
    ProgramDesc::new("lightrow.vert", "lightrow_truth.frag", Blend::Additive),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_truth.frag",
        Blend::Additive,
    ),
    ProgramDesc::new(
        "lightrow.vert",
        "lightrow_truth_medium.frag",
        Blend::Additive,
    ),
    ProgramDesc::new(
        "fullscreen.vert",
        "lightrow_sun_truth_medium.frag",
        Blend::Additive,
    ),
];

/// The reference picture: the row lights of a frame lit by `lightrow_truth.frag` (or a hard sun
/// by `lightrow_sun_truth.frag`), then the composite stage and the post pass as the renderer runs
/// them.
struct Truth<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    composite: CompositeStage<'d>,
    post: PostRenderer<'d>,
    lamps: UploadBuffer<'d>,
    light: Texture<'d>,
}

impl<'d> Truth<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Result<Self, Diagnostic> {
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec())?;
        let mut pipelines = PipelineCache::new(&shaders, &TRUTH)?;
        for program in 0..TRUTH.len() {
            pipelines.get(&mut shaders, program, HALF)?;
        }
        let target = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
        Ok(Self {
            composite: CompositeStage::new(gpu, &shaders)?,
            post: PostRenderer::new(gpu, pack.to_vec())?,
            samplers: Samplers::new(gpu)?,
            shaders,
            pipelines,
            lamps: UploadBuffer::new(gpu, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, 0, "lamps"),
            light: texture(gpu, (W, H), HALF, target, None),
        })
    }

    /// Draws `frame` with `lights` into its target. `renderer` has drawn the same frame: its
    /// occlusion mask and heights are read.
    fn frame(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Light2dFrame<'_>,
        lights: &LightStore,
        obstacles: &ObstacleStore,
        settings: &Light2dSettings,
        renderer: &Light2dRenderer<'_>,
    ) -> Result<(), Diagnostic> {
        let light = &settings.light;
        let p = frame.projection;
        let top = max_height(
            true,
            obstacles.max_finite_top(),
            obstacles.any_infinite_enabled(),
            false,
            HEIGHT_STEP,
        );
        let modes = LightModes {
            height_mode: true,
            normal_mode: true,
            max_height: top,
            ..LightModes::default()
        };
        let mut list = LightList::default();
        collect_lights(lights, light, p, 0.0, W, H, &modes, &mut list);
        let count = list.rows.len();
        self.lamps.reserve((count * RowLamp::SIZE) as u64)?;
        {
            let mut mapped = self.lamps.map()?;
            for (bytes, l) in (mapped
                .bytes()
                .as_chunks_mut::<{ RowLamp::SIZE }>()
                .0
                .iter_mut())
            .zip(&list.rows)
            {
                let lamp = RowLamp {
                    light: l.light,
                    // Any row: the reference marches instead of reading one.
                    row: [0.0, l.size, 0.0, 0.0],
                    ..RowLamp::default()
                };
                *bytes = lamp.to_bytes();
            }
        }
        let domain = OcclusionDomain::new(W, H, light.resolutions().shadow);
        let anchor = MaskAnchor::compute(p, &domain);
        let shadow = ShadowDomain::new(&domain, anchor.remainder);
        let surface = SurfaceParams::new(p, W, H, light, &shadow, true).to_bytes();
        let mask = [domain.width, domain.height];
        let space = RowSpace::new(p, &domain);
        // The reference reads no row, so no height code.
        let mut lookup = RowLookupParams::new(&space, mask, 1, height_tolerance(light), [0.0; 4]);
        lookup.domain[3] = env("KNORKE_PROBE_POINTS", 8.0);
        let lookup = lookup.to_bytes();
        let vertex = LightSurfacePassParams::new(p, W, H, top, 0).to_bytes();
        self.finish(gpu, frame, settings, renderer, |cmd, t, clear| {
            cmd.copy_pass()?
                .upload_to_buffer(&t.lamps, (count * RowLamp::SIZE) as u32)?;
            let nearest = t.samplers.sampler(Filter::Nearest);
            let occlusion = renderer.plan().occlusion();
            let patterns = renderer.plan().patterns().maps();
            let scene = frame.scene;
            // With a medium world map (the renderer's of this frame) its map follows.
            let medium = renderer.plan().medium_map().view();
            let textures = [
                frame.surface.unwrap(),
                occlusion.mask().unwrap(),
                occlusion.height().unwrap(),
                patterns.map_or(scene, |p| p.ids),
                patterns.map_or(scene, |p| p.atlas),
                medium.map_or(scene, |m| m.map),
            ]
            .map(|t| SamplerBinding::new(t, nearest));
            let lamps = t.lamps.buffer_ref().unwrap();
            let records = patterns.map_or([lamps; 3], |p| [p.obstacles, p.records, p.data]);
            let params = patterns.map_or(PatternParams::default().to_bytes(), |p| p.params);
            let uniforms: [&[u8]; 4] =
                [&surface, &lookup, &params, medium.map_or(&[], |m| m.params)];
            let m = usize::from(medium.is_some());
            let mut pass = cmd.render_pass(&[ColorTarget::new(t.light.texture_ref(), clear)])?;
            pass.bind_program(
                t.pipelines.pipeline(2 * m, HALF).unwrap(),
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &[lamps],
                        uniforms: &[&vertex],
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &textures[..5 + m],
                        storage_buffers: &records,
                        uniforms: &uniforms[..3 + m],
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, count as u32)
        })
    }

    /// Draws `frame` with the sun `sun` alone into its target, averaged over the directions
    /// `rays` (`DirSlope` xyz each, at most 64). `renderer` has drawn the same frame: its occlusion
    /// mask and heights are read.
    fn sun(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Light2dFrame<'_>,
        sun: &SunParams,
        rays: &[[f32; 3]],
        settings: &Light2dSettings,
        renderer: &Light2dRenderer<'_>,
    ) -> Result<(), Diagnostic> {
        let light = &settings.light;
        let domain = OcclusionDomain::new(W, H, light.resolutions().shadow);
        let anchor = MaskAnchor::compute(frame.projection, &domain);
        let shadow = ShadowDomain::new(&domain, anchor.remainder);
        let surface = SurfaceParams::new(frame.projection, W, H, light, &shadow, true).to_bytes();
        let params = sun.to_bytes();
        // `SunRays` of lightrow_sun_truth.frag.
        let mut block = vec![0f32; 64 * 4];
        for (out, ray) in block.as_chunks_mut::<4>().0.iter_mut().zip(rays) {
            out[..3].copy_from_slice(ray);
        }
        block[3] = rays.len() as f32;
        let rays: Vec<u8> = block.iter().flat_map(|v| v.to_le_bytes()).collect();
        // The stand-in for the pattern records of a frame without them.
        self.lamps.reserve(RowLamp::SIZE as u64)?;
        self.finish(gpu, frame, settings, renderer, |cmd, t, clear| {
            let occlusion = renderer.plan().occlusion();
            let patterns = renderer.plan().patterns().maps();
            let scene = frame.scene;
            let medium = renderer.plan().medium_map().view();
            let textures = [
                frame.surface.unwrap(),
                occlusion.mask().unwrap(),
                occlusion.height().unwrap(),
                patterns.map_or(scene, |p| p.ids),
                patterns.map_or(scene, |p| p.atlas),
                medium.map_or(scene, |m| m.map),
            ]
            .map(|tex| SamplerBinding::new(tex, t.samplers.sampler(Filter::Nearest)));
            let lamps = t.lamps.buffer_ref().unwrap();
            let records = patterns.map_or([lamps; 3], |p| [p.obstacles, p.records, p.data]);
            // `SunRays` of lightrow_sun_truth.frag holds the pattern block after the rays.
            let pattern_params = patterns.map_or(PatternParams::default().to_bytes(), |p| p.params);
            let rays = [&rays[..], &pattern_params[..]].concat();
            let uniforms: [&[u8]; 4] = [&surface, &params, &rays, medium.map_or(&[], |m| m.params)];
            let m = usize::from(medium.is_some());
            let mut pass = cmd.render_pass(&[ColorTarget::new(t.light.texture_ref(), clear)])?;
            pass.bind_program(
                t.pipelines.pipeline(1 + 2 * m, HALF).unwrap(),
                &Bindings {
                    vertex: StageBindings::default(),
                    fragment: StageBindings {
                        samplers: &textures[..5 + m],
                        storage_buffers: &records,
                        uniforms: &uniforms[..3 + m],
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_fullscreen()
        })
    }

    /// `draw` records the light into `self.light` (to be cleared with the load it is given, the
    /// ambient light); then the composite stage and the post pass draw `frame` with it, with the
    /// medium stage's result of `renderer` (it does not depend on how the ground is shadowed).
    fn finish(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Light2dFrame<'_>,
        settings: &Light2dSettings,
        renderer: &Light2dRenderer<'_>,
        draw: impl FnOnce(&mut CommandBuffer<'_>, &Self, Load) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        let light = &settings.light;
        let composite = CompositeFrame {
            scene: frame.scene,
            glow: frame.glow,
            material: None,
            surface: frame.surface,
            light: CompositeLight {
                ambient: light.ambient,
                light_buffer: Some(self.light.texture_ref()),
                use_specular: false,
                specular: None,
                direction: None,
                pure_ambient: None,
            },
            medium: renderer.plan().medium().composite_medium(frame.medium),
            projection: frame.projection,
            settings: light,
            input_space: settings.input_space(),
        };
        self.composite.prepare(&mut self.shaders, &composite)?;
        let post = PostFrame {
            hdr: self.composite.hdr_buffer().unwrap(),
            overlay: None,
            target: frame.target,
            shockwaves: None,
        };
        let prepared = self.post.prepare(&post, &settings.post)?;
        let mut cmd = CommandBuffer::acquire(gpu.device())?;
        draw(&mut cmd, self, Load::Clear(ambient_clear(light.ambient)))?;
        self.composite
            .record(&mut cmd, &self.samplers, &composite)?;
        self.post
            .record(&mut cmd, &post, &settings.post, prepared)?;
        cmd.submit_and_wait()
    }
}

/// Prepares, records and submits one frame of `renderer`.
fn draw(
    gpu: Gpu<'_>,
    renderer: &mut Light2dRenderer<'_>,
    frame: &Light2dFrame<'_>,
    stores: &SceneStores<'_>,
    settings: &Light2dSettings,
    wait: bool,
) -> PassRecord {
    let prepared = renderer.prepare(frame, stores, settings).unwrap();
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    let record = renderer
        .record(&mut cmd, frame, settings, prepared)
        .unwrap();
    if wait {
        cmd.submit_and_wait().unwrap();
    } else {
        cmd.submit().unwrap();
    }
    record
}

fn stores_of<'a>(
    lights: &'a LightStore,
    obstacles: &'a ObstacleStore,
    patterns: Option<&'a PatternStore>,
) -> SceneStores<'a> {
    SceneStores {
        patterns,
        ..SceneStores::new(lights, obstacles)
    }
}

/// Median and 95th percentile of `us`.
fn stats(mut us: Vec<f64>) -> (f64, f64) {
    us.sort_by(f64::total_cmp);
    (us[us.len() / 2], us[us.len() * 95 / 100])
}

/// Frames per timed batch. Waiting for every frame let the GPU idle between frames, and its
/// clocks dropped: the median of a frame without lights moved by a factor of 2 between runs.
const BATCH: usize = 100;

/// `frames` calls of `frame` (told whether to wait for the GPU), in batches of [`BATCH`] that wait
/// at their end; the time per frame of each batch (the host times frames: wall clock is a host
/// tool).
#[allow(clippy::disallowed_methods)]
fn time(frames: usize, mut frame: impl FnMut(bool)) -> (f64, f64) {
    let mut us = Vec::with_capacity(frames / BATCH + 1);
    let mut t0 = std::time::Instant::now();
    for i in 0..frames {
        let end = i % BATCH == BATCH - 1 || i + 1 == frames;
        frame(end);
        if end {
            us.push(t0.elapsed().as_secs_f64() * 1e6 / (i % BATCH + 1) as f64);
            t0 = std::time::Instant::now();
        }
    }
    stats(us)
}

/// One line of the table: the median over the runs' medians, their spread and the worst p95.
fn summary(scene: &str, case: &str, runs: &[(f64, f64)]) -> String {
    let (median, _) = stats(runs.iter().map(|r| r.0).collect());
    let low = runs.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    let high = runs.iter().map(|r| r.0).fold(0.0, f64::max);
    let p95 = runs.iter().map(|r| r.1).fold(0.0, f64::max);
    format!(
        "{scene:8} {case:18} median {median:8.1} us  runs {low:8.1} .. {high:8.1}  worst p95 {p95:8.1}  ({} runs)",
        runs.len()
    )
}

/// Mean absolute difference of two pictures over the colour channels, the share of pixels that
/// differ by more than 8 in percent, and the differences times 8 as a picture.
fn compare(a: &[u8], b: &[u8]) -> (f64, f64, Vec<u8>) {
    let (mut sum, mut over) = (0u64, 0usize);
    let mut diff = Vec::with_capacity(a.len());
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = [0, 1, 2].map(|c| p[c].abs_diff(q[c]));
        sum += d.iter().map(|&v| u64::from(v)).sum::<u64>();
        over += usize::from(d.into_iter().max().unwrap() > 8);
        diff.extend([0, 1, 2].map(|c| d[c].saturating_mul(8)));
        diff.push(255);
    }
    let pixels = a.len() / 4;
    (
        sum as f64 / (pixels * 3) as f64,
        100.0 * over as f64 / pixels as f64,
        diff,
    )
}

/// Most the rows may differ from the reference per scene (mean of 255, percent of the pixels
/// over 8): what was measured on 2026-11-26 (Vulkan, RX 6700 XT) plus a tenth. Measured:
/// 0.348 / 0.66, 0.626 / 1.59, 0.631 / 2.31, 0.856 / 3.03; "hart halb", the fences as 50 %
/// walls: 0.250 / 0.88; "flächen" 0.002 / 0.01 (bound rounded up: a tenth of so little is below
/// what a driver may change); "zaun", the fences with their patterns, hard 0.454 / 1.34, soft
/// 0.777 / 3.05; "gitter", the fences as tile grids with patterns ([`fence_grid`]), hard
/// 0.763 / 2.13, soft 0.882 / 2.89 (both 2026-11-27 with the pattern interpolated, a lamp with
/// a size blurring it, a hard lamp seeking each crossing on its own ray; before: "zaun" hard
/// 0.554 / 1.25, soft 1.395 / 5.62, "gitter" hard 0.817 / 1.59, soft 1.148 / 3.71: the nearest
/// pattern texel matched exactly or missed by a lot, the interpolated one misses by a little
/// more often). "nebel", the walls in [`fog_layer`]'s fog (the medium
/// world map; 2026-11-27): hard 0.284 / 0.51, soft 0.485 / 1.81. "hoch", the walls three times
/// as tall (tops to 60, so the rows store heights in fiftieths), hard 0.364 / 0.71; "viele",
/// every lamp 20 times (260 rows) as bright as four, hard 0.683 / 2.35; "mast", every lamp on a
/// mast 0.8 thick up to 1 above it and marked `inside_obstacle` (E147), hard 0.360 / 0.72, soft
/// 0.626 / 2.30; "filter", every lamp static-only (E53) and every obstacle static: as "normal"
/// (2026-11-27). "weit", each lamp reaching about 1200 mask texels (2048 angles in half texels,
/// `WIDE_REACH`; 2026-11-27): hard 0.257 / 1.29, soft 1.040 / 3.56. Chiaro's light was 1.5 to 6
/// times as far from the reference in every case (plan 17.13, L7).
const BOUNDS: [(&str, f64, f64); 21] = [
    ("hart normal", 0.39, 0.73),
    ("hart last", 0.71, 1.76),
    ("weich normal", 0.70, 2.55),
    ("weich last", 0.96, 3.34),
    ("hart halb", 0.35, 1.38),
    ("flächen", 0.004, 0.02),
    ("linie pfosten", 0.014, 0.03),
    ("zaun hart", 0.50, 1.47),
    ("zaun weich", 0.85, 3.36),
    ("gitter hart", 0.84, 2.34),
    ("gitter weich", 0.97, 3.18),
    ("nebel hart", 0.32, 0.57),
    ("nebel weich", 0.54, 2.0),
    ("hoch hart", 0.41, 0.79),
    ("viele hart", 0.76, 2.59),
    ("mast hart", 0.40, 0.80),
    ("mast weich", 0.69, 2.53),
    ("filter hart", 0.39, 0.73),
    ("filter weich", 0.70, 2.55),
    ("weit hart", 0.28, 1.42),
    ("weit weich", 1.14, 3.92),
];

/// The lamps' radius in "weit" (world units): 1200 mask texels at the probe's shadow
/// resolution, beyond `MAX_REACH`.
const WIDE_RADIUS: f32 = 170.0;

/// The facade in "zaun hart" that the spot light beside the lattice tower lights through it
/// (output pixels x, y, width, height), and the most the rows may differ from the reference
/// there (mean of 255). Rays rising out through the tower's top read its pattern where they
/// leave it, not at its far side; reading the far side lost the top of the lattice's shadow
/// there. Where the light's ray grazes the tower, the mask's staircase splits it in two runs;
/// hulling the low building and the facade behind them instead blackened the facade's south
/// end (the build joins the pieces then, `Squeeze`). Measured 1.039 (2026-11-27, Vulkan,
/// RX 6700 XT; the pattern interpolated, each crossing sought on the pixel's own ray), before
/// 1.316, hulled 1.951, at the far side 2.625; plus a tenth.
const FACADE: ([usize; 4], f64) = ([560, 170, 160, 160], 1.14);

/// Mean absolute difference of two pictures over the colour channels within `rect`.
fn region_mean(a: &[u8], b: &[u8], [x, y, w, h]: [usize; 4]) -> f64 {
    let row = OUT.0 as usize * 4;
    let mut sum = 0u64;
    for r in y..y + h {
        let span = r * row + x * 4..r * row + (x + w) * 4;
        for (p, q) in a[span.clone()]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b[span].as_chunks::<4>().0)
        {
            sum += (0..3).map(|c| u64::from(p[c].abs_diff(q[c]))).sum::<u64>();
        }
    }
    sum as f64 / (w * h * 3) as f64
}

/// The post of "linie pfosten": a third along the shop sign's line light (at 31, 26.12, 2.4
/// either way along x, 4.6 high), 0.7 in front of it and taller than it, so it cuts the light of
/// one end off the ground behind it and a line light marched from its middle alone misses that.
fn post() -> ShapeDesc<'static> {
    let common = ObstacleCommon {
        position: At::new(Vec2::new(31.0 - 2.4 + 4.8 / 3.0, 26.82)),
        rotation: 0.0,
        height: 6.0,
        base_height: 0.0,
        opacity: 1.0,
        dynamic: false,
        enabled: true,
        shadow_footprint: false,
    };
    ShapeDesc {
        common,
        shape: Shape::Circle { radius: 0.25 },
    }
}

/// A mast under lamp `d`: a post 0.8 thick (some 4 mask texels) from the ground to 1 above it.
fn mast(d: &LightDesc) -> ShapeDesc<'static> {
    let Height::At(h) = d.height else {
        unreachable!("the probe's lamps have a height")
    };
    ShapeDesc {
        common: ObstacleCommon {
            position: d.position,
            height: h + 1.0,
            ..post().common
        },
        shape: Shape::Circle { radius: 0.4 },
    }
}

/// A bank of ground fog (the preset `Fog`, 3 high) over the middle of the street, densest (0.5)
/// in its middle and thinning to its elliptic edge: the demo's own clouds (`t5`) are too thin
/// to dim a lamp noticeably.
fn fog_layer() -> Vec<u8> {
    let step = settings(0.5).light.resolved_medium_height_step();
    let mut px = vec![0u8; (W * H * 4) as usize];
    for (i, out) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % W) as f32, (i as u32 / W) as f32);
        let e = ((x - 380.0) / 230.0).powi(2) + ((y - 250.0) / 90.0).powi(2);
        if e < 1.0 {
            *out = encode_medium_pixel(0.5 * (1.0 - e), 0.0, 3.0, 4, step).unwrap();
        }
    }
    px
}

/// The demo's scene, glow and surface layers, uploaded; with `fog` also [`fog_layer`].
fn demo_layers(gpu: Gpu<'_>, fog: bool) -> Layers<'_> {
    let mut kinds = LayerSet::of(Layer::Surface).with(Layer::Glow);
    if fog {
        kinds = kinds.with(Layer::Medium);
    }
    let mut layers = Layers::new(gpu, W, H, kinds).unwrap();
    let fixture = |name: &str| scene_common::fixture("iso_night", name, (W, H));
    let (color, glow, surface) = (fixture("scene"), fixture("glow"), fixture("surface"));
    let medium = fog_layer();
    let mut fills = vec![
        (Layer::Scene, &color[..]),
        (Layer::Glow, &glow[..]),
        (Layer::Surface, &surface[..]),
    ];
    if fog {
        fills.push((Layer::Medium, &medium[..]));
    }
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
    let uploads = scene_common::upload_layers(gpu, &mut cmd, &mut layers, &fills);
    cmd.submit_and_wait().unwrap();
    drop(uploads);
    layers
}

/// An output picture's target.
fn target(gpu: Gpu<'_>) -> Texture<'_> {
    texture(
        gpu,
        OUT,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
        None,
    )
}

#[test]
fn light_rows_against_the_reference() {
    let out: Option<PathBuf> = std::env::var("KNORKE_PROBE_OUT").ok().map(PathBuf::from);
    let frames: usize = env("KNORKE_PROBE_FRAMES", 0);
    let runs: usize = env("KNORKE_PROBE_RUNS", 5);
    with_chiaro_pack(|gpu, pack| {
        let text = String::from_utf8(golden_bytes("iso_night", "hard.txt")).unwrap();
        let plain = String::from_utf8(golden_bytes("iso_night", "plain.txt")).unwrap();
        let projection = Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H);
        let rows = settings(0.5);
        let mut report = format!("{frames} frames per run\n");
        let mut full = String::new();

        let (clear, foggy) = (demo_layers(gpu, false), demo_layers(gpu, true));
        let targets = [target(gpu), target(gpu)];
        // Frame k of the camera scrolling along x one mask texel a frame, 30 texels to and fro.
        let domain = OcclusionDomain::new(W, H, rows.light.resolutions().shadow);
        let texel = 1.0 / domain.texels_per_scene[0];
        let scrolled = |k: u32| {
            let mut p = projection;
            let x = 30.0 - (30.0 - (k % 60) as f32).abs();
            p.origin = At::new(projection.origin.raw() + Vec2::new(x * texel, 0.0));
            p
        };
        let mut truth = Truth::new(gpu, pack).unwrap();

        // The load scene: every lamp four times, slightly apart; "viele" 20 times (260 rows, more
        // than the 256 the atlas once had), the others on a grid 0.3 apart.
        let four = [(0.0, 0.0), (0.7, 0.3), (-0.5, 0.6), (0.4, -0.7)];
        let offsets: Vec<(f32, f32)> = (0..20)
            .map(|k| {
                let grid = ((k % 4) as f32 * 0.3 - 0.45, (k / 4) as f32 * 0.3 - 0.75);
                four.get(k).copied().unwrap_or(grid)
            })
            .collect();
        let scenes = [false, true]
            .into_iter()
            .flat_map(|soft| {
                let walls = Fences::Walls(1.0);
                [(soft, 1, walls, "01"), (soft, 4, walls, "01")]
            })
            .chain([
                (false, 1, Fences::Walls(0.5), "01"),
                (false, 1, Fences::Walls(1.0), "34"),
                (false, 1, Fences::Walls(1.0), "3"),
                (false, 1, Fences::Patterns, "01"),
                (true, 1, Fences::Patterns, "01"),
                (false, 1, Fences::Grids, "01"),
                (true, 1, Fences::Grids, "01"),
            ])
            .map(|(soft, copies, fence, kinds)| (soft, copies, fence, kinds, false))
            .chain([
                (false, 1, Fences::Walls(1.0), "01", true),
                (true, 1, Fences::Walls(1.0), "01", true),
                (false, 1, Fences::Tall(3.0), "01", false),
                (false, 20, Fences::Walls(1.0), "01", false),
                (false, 1, Fences::Walls(1.0), "01m", false),
                (true, 1, Fences::Walls(1.0), "01m", false),
                (false, 1, Fences::Walls(1.0), "01s", false),
                (true, 1, Fences::Walls(1.0), "01s", false),
                (false, 1, Fences::Walls(1.0), "01w", false),
                (true, 1, Fences::Walls(1.0), "01w", false),
            ]);
        let only = std::env::var("KNORKE_PROBE_ONLY").unwrap_or_default();
        for ((soft, copies, fence, kinds, fog), bound) in scenes.zip(BOUNDS) {
            let name = bound.0;
            if !name.starts_with(only.as_str()) {
                continue;
            }
            let layers = if fog { &foggy } else { &clear };
            let frame =
                |i: usize| layers.frame(targets[i].texture_ref(), &projection, LayerSet::NONE);
            // A frame of target i seen through p (a closure cannot return the frame borrowing p).
            macro_rules! frame_at {
                ($i:expr, $p:expr) => {
                    layers.frame(targets[$i].texture_ref(), $p, LayerSet::NONE)
                };
            }
            let (mut base, mut obstacles, patterns) = parse(&text, false, fence, kinds);
            if kinds == "3" {
                obstacles.create_shape(&post()).unwrap();
            }
            if kinds.ends_with('m') {
                // Every lamp on a mast up to above it, marked as standing in it (E147).
                for d in &mut base {
                    obstacles.create_shape(&mast(d)).unwrap();
                    d.inside_obstacle = true;
                }
            }
            if kinds.ends_with('w') {
                // Every lamp reaching 1200 mask texels (rows of 2048 angles), a sixth as bright.
                for d in &mut base {
                    d.radius = WIDE_RADIUS;
                    d.intensity /= 6.0;
                }
            }
            if kinds.ends_with('s') {
                // Every lamp's shadow from the static obstacles alone (E53): all of the probe's.
                for d in &mut base {
                    d.shadow_filter = ShadowFilter::StaticOnly;
                }
            }
            if soft {
                // The lamp sizes of the variant with soft shadows, on the same lamps.
                let sized = parse(&plain, true, Fences::Walls(1.0), kinds).0;
                assert_eq!(sized.len(), base.len());
                for (d, s) in base.iter_mut().zip(&sized) {
                    assert_eq!(d.position, s.position);
                    d.source_radius = s.source_radius;
                }
            }
            let (expected, supports) = match kinds {
                "01" | "01m" | "01s" | "01w" => (13, 1),
                "34" => (4, SUPPORTS),
                _ => (1, SUPPORTS),
            };
            assert_eq!(
                base.len(),
                expected,
                "the scene's shadowed lights of the case"
            );
            // More than four copies as bright as four, so the picture does not burn out.
            let dim = 4.0 / copies.max(4) as f32;
            let descs: Vec<LightDesc> = offsets[..copies]
                .iter()
                .flat_map(|&(dx, dy)| {
                    base.iter().map(move |d| LightDesc {
                        position: At::new(d.position.raw() + Vec2::new(dx, dy)),
                        intensity: d.intensity * dim,
                        ..d.clone()
                    })
                })
                .collect();
            let store = || {
                let mut lights = LightStore::new();
                let ids: Vec<_> = descs
                    .iter()
                    .map(|d| lights.create(d, &NoResources).unwrap())
                    .collect();
                (lights, ids)
            };
            let (lights, _) = store();
            let (mut moving, ids) = store();
            let none = LightStore::new();
            let patterns =
                (!matches!(fence, Fences::Walls(_) | Fences::Tall(_))).then_some(&patterns);
            let new = || Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
            let mut renderers = [new(), new(), new()];
            let all = descs.len() as u32;
            let rows_all = all * supports;

            let mut timed: [Vec<(f64, f64)>; 4] = Default::default();
            let mut scroll_rows = 0;
            let mut step = 0u32;
            for run in 0..=runs {
                // Run 0 draws the pictures; the timed runs take the cases in turn.
                let n = if run == 0 { 1 } else { frames };
                if n == 0 {
                    break;
                }
                let [r_rest, r_moved, r_none] = &mut renderers;
                let stores = stores_of(&lights, &obstacles, patterns);
                let moved = time(n, |wait| {
                    step += 1;
                    let x = if step.is_multiple_of(2) { NUDGE } else { 0.0 };
                    for (&id, d) in ids.iter().zip(&descs) {
                        let at = At::new(d.position.raw() + Vec2::new(x, 0.0));
                        moving.set_position(id, at, d.height).unwrap();
                    }
                    let stores = stores_of(&moving, &obstacles, patterns);
                    let record = draw(gpu, r_moved, &frame(1), &stores, &rows, wait);
                    assert_eq!(record.lighting.rows_built, rows_all);
                });
                let floor = time(n, |wait| {
                    let stores = stores_of(&none, &obstacles, patterns);
                    draw(gpu, r_none, &frame(1), &stores, &rows, wait);
                });
                // The first frame builds the rows, from then on the lamps rest.
                let first = draw(gpu, r_rest, &frame(0), &stores, &rows, true).lighting;
                assert_eq!(first.lights_drawn, all, "every lamp");
                let medium = r_rest.plan().medium_map().view().is_some();
                assert_eq!(medium, fog, "{name}: the medium world map");
                assert_eq!(first.rows_built, if run == 0 { rows_all } else { 0 });
                let rest = time(n, |wait| {
                    let record = draw(gpu, r_rest, &frame(0), &stores, &rows, wait);
                    assert_eq!(record.lighting.rows_built, 0);
                });
                if run > 0 {
                    let mut k = 0;
                    let scroll = time(n, |wait| {
                        k += 1;
                        let p = scrolled(k);
                        let f = frame_at!(0, &p);
                        scroll_rows += draw(gpu, r_rest, &f, &stores, &rows, wait)
                            .lighting
                            .rows_built;
                    });
                    // Back where the next run begins, untimed.
                    draw(gpu, r_rest, &frame(0), &stores, &rows, true);
                    let all = [rest, moved, floor, scroll];
                    for (list, value) in timed.iter_mut().zip(all) {
                        list.push(value);
                    }
                    eprintln!(
                        "{name} run {run}: rows resting {:.1} / {:.1}, rows all built {:.1} / {:.1}, no lights {:.1} / {:.1}, scrolling {:.1} / {:.1} us (median / p95)",
                        rest.0, rest.1, moved.0, moved.1, floor.0, floor.1, scroll.0, scroll.1
                    );
                }
            }
            (truth.frame(gpu, &frame(1), &lights, &obstacles, &rows, &renderers[0])).unwrap();
            let pictures = ["neu", "wahr"]
                .into_iter()
                .zip(targets.iter().map(|t| read_texture(gpu, t.texture_ref())))
                .collect::<Vec<_>>();
            let (mean, over, diff) = compare(&pictures[0].1, &pictures[1].1);
            let _ = writeln!(
                report,
                "{name:12} {all} lamps, neu against wahr: mean absolute difference {mean:.3} of 255, {over:.2} % of the pixels differ by more than 8"
            );
            let diffs = [("diff_neu_wahr_x8".to_owned(), diff)];
            if frames > 0 {
                let cases = [
                    "rows, lamps rest",
                    "rows, all built",
                    "no lights",
                    "rows scrolling",
                ];
                for (case, list) in cases.into_iter().zip(&timed) {
                    let _ = writeln!(report, "{}", summary(name, case, list));
                }
                let per_frame = f64::from(scroll_rows) / (frames * runs) as f64;
                let _ = writeln!(
                    report,
                    "{name:12} rows built per frame while scrolling: {per_frame:.2} of {rows_all}"
                );
            }
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let named = pictures.iter().map(|(n, px)| (*n, px));
                for (suffix, px) in named.chain(diffs.iter().map(|(n, px)| (n.as_str(), px))) {
                    let file = dir.join(format!("{}_{suffix}.png", name.replace(' ', "_")));
                    save_png_rgba8(&file, px, OUT.0, OUT.1);
                }
            }
            let facade = (name == "zaun hart")
                .then(|| region_mean(&pictures[0].1, &pictures[1].1, FACADE.0));
            if let Some(facade) = facade {
                let _ = writeln!(
                    report,
                    "{name:12} facade, neu against wahr: mean {facade:.3} of 255"
                );
            }
            eprint!("{report}");
            assert!(
                mean <= bound.1 && over <= bound.2,
                "{name}: rows against the reference {:?}, bound {bound:?}",
                (mean, over)
            );
            assert!(
                facade.is_none_or(|f| f <= FACADE.1),
                "{name}: the facade lit through the lattice tower {facade:?}, bound {FACADE:?}"
            );
            full.push_str(&report);
            report.clear();
        }
        if let Some(dir) = &out {
            std::fs::write(dir.join("messung.txt"), &full).unwrap();
        }
    });
}

/// The cameras of the sun case. Camera B is moved by a fraction of a pixel, so the mask lies
/// differently under the pixels.
const SUN_CAMERAS: [(&str, [f32; 2]); 2] = [("A", [18.0, 14.0]), ("B", [18.037, 14.021])];

/// The fences of the sun cases: as opaque walls ("mauer"), with the demo's patterns ("zaun"), as
/// half transparent walls ("halb"), as tile grids with patterns ("gitter"), as opaque walls in
/// [`fog_layer`]'s fog ("nebel"); `parse`'s `fence` and whether the frame has the fog.
const SUN_FENCES: [(&str, Fences, bool); 5] = [
    ("mauer", Fences::Walls(1.0), false),
    ("zaun", Fences::Patterns, false),
    ("halb", Fences::Walls(0.5), false),
    ("gitter", Fences::Grids, false),
    ("nebel", Fences::Walls(1.0), true),
];

/// The most the sun map may differ from the reference (mean of 255, percent of the pixels over
/// 8), per fences ([`SUN_FENCES`]), moon (hard, soft) and camera: what was measured plus a
/// tenth. Measured on 2026-11-26 (Vulkan, RX 6700 XT): "mauer" hard A 0.0413 / 0.176, B
/// 0.0284 / 0.120; soft (bilinear, the ends widened) A 0.0692 / 0.211, B 0.0769 / 0.187. "zaun"
/// hard A 0.0432 / 0.188, B 0.0385 / 0.158; soft A 0.0889 / 0.259, B 0.1027 / 0.314. "halb"
/// hard A 0.0380 / 0.146, B 0.0305 / 0.147; soft A 0.0708 / 0.165, B 0.0780 / 0.159. "gitter"
/// (the fences as tile grids with patterns): hard A 0.1058 / 0.555, B 0.0970 / 0.530; soft A
/// 0.1270 / 0.569, B 0.1369 / 0.578. "zaun" and "gitter" measured 2026-11-27 with the pattern
/// interpolated; before (nearest texel) "zaun" hard A 0.0417 / 0.171, B 0.0393 / 0.147, soft
/// A 0.1107 / 0.424, B 0.1279 / 0.477; "gitter" hard A 0.1147 / 0.567, B 0.1048 / 0.535, soft
/// A 0.1372 / 0.620, B 0.1463 / 0.619. While "zaun" or "gitter" scrolls,
/// the map is built whole in 5 of 2400 frames: a fence leaves the mask and the other's record
/// index moves. "nebel" (2026-11-27): hard A 0.0267 / 0.086, B 0.0191 / 0.069; soft A
/// 0.0521 / 0.155, B 0.0582 / 0.139. Chiaro's light was 1.2 to 2.2 times as far from the
/// reference in every case (plan 17.13, L3).
const SUN_BOUNDS: [[[(f64, f64); 2]; 2]; 5] = [
    [
        [(0.046, 0.194), (0.032, 0.132)],
        [(0.076, 0.232), (0.085, 0.206)],
    ],
    [
        [(0.048, 0.207), (0.042, 0.174)],
        [(0.098, 0.285), (0.113, 0.345)],
    ],
    [
        [(0.042, 0.161), (0.034, 0.162)],
        [(0.078, 0.182), (0.086, 0.175)],
    ],
    [
        [(0.116, 0.611), (0.107, 0.583)],
        [(0.140, 0.626), (0.151, 0.636)],
    ],
    [
        [(0.030, 0.095), (0.022, 0.076)],
        [(0.058, 0.171), (0.065, 0.153)],
    ],
];

/// As [`SUN_BOUNDS`] for the hard moon from below the horizon ("unten"), per fences and camera.
/// Measured on 2026-11-27 (Vulkan, RX 6700 XT): "mauer" A 0.0168 / 0.042, B 0.0087 / 0.022;
/// "zaun" A 0.0216 / 0.062, B 0.0211 / 0.060; "halb" A 0.0209 / 0.071, B 0.0179 / 0.079;
/// "gitter" A 0.0776 / 0.326, B 0.0749 / 0.336; "nebel" A 0.0082 / 0.026, B 0.0081 / 0.022 (a
/// ray that does not rise walks the whole mask: a map built every frame costs more than from
/// above). "zaun" and "gitter" with the pattern interpolated; before "zaun" A 0.0198 / 0.048,
/// B 0.0215 / 0.051, "gitter" A 0.0782 / 0.328, B 0.0762 / 0.335.
const SUN_LOW_BOUNDS: [[(f64, f64); 2]; 5] = [
    [(0.019, 0.047), (0.010, 0.025)],
    [(0.024, 0.068), (0.023, 0.066)],
    [(0.023, 0.079), (0.020, 0.087)],
    [(0.085, 0.359), (0.082, 0.370)],
    [(0.010, 0.029), (0.009, 0.025)],
];

/// As [`SUN_BOUNDS`] for the hard moon as the third sun of its frame ("spät"), per fences and
/// camera: every sun has a map of the full grid.
const SUN_LATE_BOUNDS: [[(f64, f64); 2]; 5] = [
    [(0.045, 0.194), (0.031, 0.132)],
    [(0.048, 0.207), (0.042, 0.174)],
    [(0.042, 0.161), (0.034, 0.162)],
    [(0.116, 0.611), (0.107, 0.583)],
    [(0.029, 0.095), (0.021, 0.076)],
];

/// The moon of a sun case in `lights`; `late` ("spät") behind two suns too dim to change a
/// pixel, so that it is the third sun of its frame.
fn sky(lights: &mut LightStore, moon: &LightDesc, late: bool) -> LightId {
    if late {
        for x in [0.6, -0.6] {
            let dim = LightDesc {
                direction: Vec3::new(x, 0.3, -0.75),
                intensity: 1e-6,
                ..moon.clone()
            };
            lights.create(&dim, &NoResources).unwrap();
        }
    }
    lights.create(moon, &NoResources).unwrap()
}

/// Radians the moon of the timed case "maps rebuilt" turns to and fro between frames.
const TURN: f32 = 0.002;

/// Points per axis of the grid over a soft moon's disc whose directions the reference averages
/// (those inside the disc).
const DISC: usize = 8;

/// The moon of `iso_night_hard` alone (the demo's only directional light, a hard one; and with
/// the size of `plain.txt`'s moon, soft), with the fences of [`SUN_FENCES`]: its shadow from the
/// sun map against an exact march per pixel (`lightrow_sun_truth.frag`; soft: the mean over the
/// directions of the moon's disc), on both [`SUN_CAMERAS`]. `KNORKE_PROBE_OUT` writes
/// `sonne_<fences>_<moon>_<camera>_{neu,wahr}.png`, the difference times 8 and `sonne.txt`; with
/// `KNORKE_PROBE_FRAMES` camera A times the map kept and built every frame (the moon turning),
/// and the camera scrolling one mask texel a frame (the map updated in bands).
#[test]
fn sun_map_against_the_reference() {
    let out: Option<PathBuf> = std::env::var("KNORKE_PROBE_OUT").ok().map(PathBuf::from);
    let frames: usize = env("KNORKE_PROBE_FRAMES", 0);
    let runs: usize = env("KNORKE_PROBE_RUNS", 5);
    with_chiaro_pack(|gpu, pack| {
        let text = String::from_utf8(golden_bytes("iso_night", "hard.txt")).unwrap();
        let plain = String::from_utf8(golden_bytes("iso_night", "plain.txt")).unwrap();
        let rows = settings(0.5);
        let (clear, foggy) = (demo_layers(gpu, false), demo_layers(gpu, true));
        let targets = [target(gpu), target(gpu)];
        let (moons, _, _) = parse(&text, false, Fences::Walls(1.0), "2");
        assert_eq!(moons.len(), 1, "the scene's moon");
        let hard = moons[0].clone();
        assert_eq!(hard.source_radius, 0.0, "a hard moon");
        // The moon of `plain.txt` (which casts no shadow there) has a size: its field 14.
        let line = plain.lines().find(|l| l.starts_with("light 2 ")).unwrap();
        let soft = LightDesc {
            source_radius: num(line.split(' ').nth(14).unwrap()),
            ..hard.clone()
        };
        assert!(soft.source_radius > 0.0, "the moon's size");
        let domain = OcclusionDomain::new(W, H, rows.light.resolutions().shadow);
        let mut truth = Truth::new(gpu, pack).unwrap();
        let (mut report, mut results) = (String::new(), Vec::new());
        // The hard moon from below the horizon: its rays toward it fall.
        let d = hard.direction;
        let low = LightDesc {
            direction: Vec3::new(d.x, d.y, 0.2 * d.z.abs()),
            ..hard.clone()
        };
        let fenced = SUN_FENCES.into_iter().zip(SUN_BOUNDS).zip(SUN_LOW_BOUNDS);
        let cases = fenced.zip(SUN_LATE_BOUNDS).flat_map(
            |(((fences, [hard_bounds, soft_bounds]), low_bounds), late_bounds)| {
                [
                    (("hart", &hard), hard_bounds),
                    (("weich", &soft), soft_bounds),
                    (("unten", &low), low_bounds),
                    (("spät", &hard), late_bounds),
                ]
                .map(move |(moon, bounds)| (fences, moon, bounds))
            },
        );
        for ((fences, fence, fog), (kind, moon), bounds) in cases {
            let layers = if fog { &foggy } else { &clear };
            let (_, obstacles, patterns) = parse(&text, false, fence, "2");
            let patterns =
                (!matches!(fence, Fences::Walls(_) | Fences::Tall(_))).then_some(&patterns);
            let late = kind == "spät";
            let kind = format!("{fences} {kind}");
            let mut lights = LightStore::new();
            sky(&mut lights, moon, late);
            let suns = if late { 3 } else { 1 };
            for ((camera, [x, y]), (mean_bound, over_bound)) in SUN_CAMERAS.into_iter().zip(bounds)
            {
                let projection = Projection::iso(At::new(Vec2::new(x, y)), 10.0, 1.0, W, H);
                let frame =
                    |i: usize| layers.frame(targets[i].texture_ref(), &projection, LayerSet::NONE);
                let mut r_rows = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
                let stores = stores_of(&lights, &obstacles, patterns);
                let now = draw(gpu, &mut r_rows, &frame(0), &stores, &rows, true).lighting;
                assert_eq!((now.suns, now.suns_built), (suns, suns), "every sun a map");
                let medium = r_rows.plan().medium_map().view().is_some();
                assert_eq!(medium, fog, "{kind}: the medium world map");
                let list = r_rows.plan().lights();
                let last = list.directional.len() - 1;
                let (sun, rise) = (list.directional[last], list.directional_rise[last]);
                assert_eq!(rise > 0.0, moon.source_radius > 0.0, "{kind}: rise {rise}");
                let params = SunParams::new(
                    &sun,
                    rise,
                    [domain.width, domain.height],
                    MaskAnchor::compute(&projection, &domain).snapped,
                    height_tolerance(&rows.light),
                    obstacles.max_finite_top(),
                )
                .with_partial(obstacles.any_partial(), patterns.is_some());
                let rays = disc(moon, &projection, &obstacles, &rows.light);
                truth
                    .sun(gpu, &frame(1), &params, &rays, &rows, &r_rows)
                    .unwrap();
                let name = format!("{}_{camera}", kind.replace(' ', "_"));
                let pictures: Vec<(&str, Vec<u8>)> = ["neu", "wahr"]
                    .into_iter()
                    .zip(targets.iter().map(|t| read_texture(gpu, t.texture_ref())))
                    .collect();
                let (mean, over, diff) = compare(&pictures[0].1, &pictures[1].1);
                let _ = writeln!(
                    report,
                    "sonne {kind} {camera} neu against wahr: mean absolute difference {mean:.4} of 255, {over:.3} % of the pixels differ by more than 8"
                );
                if let Some(dir) = &out {
                    std::fs::create_dir_all(dir).unwrap();
                    let file = dir.join(format!("sonne_{name}_diff_neu_wahr_x8.png"));
                    save_png_rgba8(&file, &diff, OUT.0, OUT.1);
                }
                if let Some(dir) = &out {
                    for (picture, px) in &pictures {
                        let file = dir.join(format!("sonne_{name}_{picture}.png"));
                        save_png_rgba8(&file, px, OUT.0, OUT.1);
                    }
                }
                if frames > 0 && camera == "A" {
                    let (timed, rebuilt) = time_sun(
                        gpu,
                        (moon, late, &obstacles, patterns),
                        &mut r_rows,
                        &rows,
                        (layers, &targets),
                        &projection,
                        (frames, runs),
                    );
                    let cases = [
                        "map kept",
                        "map built every frame",
                        "map scrolling +x",
                        "map scrolling -x",
                    ];
                    for (case, list) in cases.into_iter().zip(&timed) {
                        let _ =
                            writeln!(report, "{}", summary(&format!("sonne {kind}"), case, list));
                    }
                    let _ = writeln!(
                        report,
                        "sonne {kind}: map built whole while scrolling in {rebuilt} frames (a pattern obstacle left the mask and another's record index moved)"
                    );
                }
                results.push((name, (mean, over), (mean_bound, over_bound)));
            }
        }
        if frames > 0 {
            // Suns without a map (the sun programs draw them plain): without shadow, and straight
            // overhead, over the walls from camera A.
            let (_, obstacles, _) = parse(&text, false, Fences::Walls(1.0), "2");
            let flat = LightDesc {
                cast_shadows: false,
                ..hard.clone()
            };
            let overhead = LightDesc {
                direction: Vec3::new(0.0, 0.0, -1.0),
                ..hard.clone()
            };
            let [x, y] = SUN_CAMERAS[0].1;
            let projection = Projection::iso(At::new(Vec2::new(x, y)), 10.0, 1.0, W, H);
            let frame =
                |i: usize| clear.frame(targets[i].texture_ref(), &projection, LayerSet::NONE);
            for (kind, moon) in [("ohne Schatten", &flat), ("senkrecht", &overhead)] {
                let mut lights = LightStore::new();
                lights.create(moon, &NoResources).unwrap();
                let stores = stores_of(&lights, &obstacles, None);
                let mut r_rows = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
                let mut now = Vec::new();
                for _ in 0..runs {
                    now.push(time(frames, |wait| {
                        let r = draw(gpu, &mut r_rows, &frame(0), &stores, &rows, wait).lighting;
                        assert_eq!((r.suns, r.suns_built), (1, 0), "{kind}: no map");
                    }));
                }
                let _ = writeln!(
                    report,
                    "{}",
                    summary(&format!("sonne {kind}"), "rows", &now)
                );
            }
        }
        eprint!("{report}");
        if let Some(dir) = &out {
            std::fs::write(dir.join("sonne.txt"), &report).unwrap();
        }
        for (name, new, bound) in results {
            assert!(
                new.0 <= bound.0 && new.1 <= bound.1,
                "{name}: the sun map against the reference {new:?}, bound {bound:?}"
            );
        }
    });
}

/// `DirSlope` xyz of the directions the reference averages for `moon`: its own if hard, else
/// [`DISC`] squared points over its disc (those inside), each the light record the renderer
/// makes of the moon turned that way.
fn disc(
    moon: &LightDesc,
    projection: &Projection,
    obstacles: &ObstacleStore,
    light: &LightSettings,
) -> Vec<[f32; 3]> {
    let modes = LightModes {
        height_mode: true,
        normal_mode: true,
        max_height: max_height(
            true,
            obstacles.max_finite_top(),
            obstacles.any_infinite_enabled(),
            false,
            HEIGHT_STEP,
        ),
        ..LightModes::default()
    };
    let slope = |toward: Vec3| {
        let mut lights = LightStore::new();
        let desc = LightDesc {
            direction: -toward.normalize_or_zero(),
            source_radius: 0.0,
            ..moon.clone()
        };
        lights.create(&desc, &NoResources).unwrap();
        let mut list = LightList::default();
        collect_lights(&lights, light, projection, 0.0, W, H, &modes, &mut list);
        let [x, y, z, _] = list.directional[0].dir_slope;
        [x, y, z]
    };
    let l = (-moon.direction).normalize_or_zero();
    if moon.source_radius <= 0.0 {
        return vec![slope(l)];
    }
    // The disc: up (in the vertical plane through l) and sideways.
    let up = (Vec3::new(0.0, 0.0, 1.0) - l * l.z).normalize_or_zero();
    let side = l.cross(up);
    let tan = knorke_math::det::tan(moon.source_radius * core::f32::consts::PI / 180.0);
    let at = |i: usize| (i as f32 + 0.5) / DISC as f32 * 2.0 - 1.0;
    (0..DISC * DISC)
        .map(|k| (at(k % DISC), at(k / DISC)))
        .filter(|(x, y)| x * x + y * y <= 1.0)
        .map(|(x, y)| slope(l + (up * x + side * y) * tan))
        .collect()
}

/// The timed cases of the sun case on camera `projection` (see
/// [`sun_map_against_the_reference`]), each `runs` runs of `frames` frames: map kept, map built
/// every frame, map scrolling +x, map scrolling -x; and in how many scrolling frames the map was
/// built whole instead of updated in part.
fn time_sun(
    gpu: Gpu<'_>,
    (moon, late, obstacles, patterns): (&LightDesc, bool, &ObstacleStore, Option<&PatternStore>),
    r_rows: &mut Light2dRenderer<'_>,
    rows: &Light2dSettings,
    (layers, targets): (&Layers<'_>, &[Texture<'_>; 2]),
    projection: &Projection,
    (frames, runs): (usize, usize),
) -> ([Vec<(f64, f64)>; 4], u32) {
    // A frame of target i seen through p (a closure cannot return the frame borrowing p).
    macro_rules! frame {
        ($i:expr, $p:expr) => {
            layers.frame(targets[$i].texture_ref(), $p, LayerSet::NONE)
        };
    }
    let mut lights = LightStore::new();
    sky(&mut lights, moon, late);
    let stores = stores_of(&lights, obstacles, patterns);
    let mut moving = LightStore::new();
    let turning = sky(&mut moving, moon, late);
    let suns = if late { 3 } else { 1 };
    let mut step = 0u32;
    let mut timed: [Vec<(f64, f64)>; 4] = Default::default();
    let mut rebuilt = 0;
    // The camera scrolling along x by one mask texel a frame, `sign` the way.
    let domain = OcclusionDomain::new(W, H, rows.light.resolutions().shadow);
    let texel = 1.0 / domain.texels_per_scene[0];
    let origin = projection.origin.raw();
    let scrolled = |k: usize, sign: f32| {
        let mut p = *projection;
        p.origin = At::new(Vec2::new(origin.x + sign * k as f32 * texel, origin.y));
        p
    };
    for _ in 0..runs {
        let rest = time(frames, |wait| {
            let r = draw(gpu, r_rows, &frame!(0, projection), &stores, rows, wait);
            assert_eq!((r.lighting.suns, r.lighting.suns_built), (suns, 0));
        });
        let built = time(frames, |wait| {
            step += 1;
            let a = if step.is_multiple_of(2) { 0.0 } else { TURN };
            let (s, c) = (knorke_math::det::sin(a), knorke_math::det::cos(a));
            let d = moon.direction;
            let desc = LightDesc {
                direction: Vec3::new(c * d.x - s * d.y, s * d.x + c * d.y, d.z),
                ..moon.clone()
            };
            moving.update(turning, &desc, &NoResources).unwrap();
            let stores = stores_of(&moving, obstacles, patterns);
            let r = draw(gpu, r_rows, &frame!(0, projection), &stores, rows, wait);
            assert_eq!((r.lighting.suns, r.lighting.suns_built), (suns, 1));
        });
        if !step.is_multiple_of(2) {
            // Back to the resting moon, untimed.
            draw(gpu, r_rows, &frame!(0, projection), &stores, rows, true);
        }
        let mut scroll = |sign: f32| {
            let mut k = 0;
            let timed = time(frames, |wait| {
                k += 1;
                let p = scrolled(k, sign);
                let f = frame!(0, &p);
                let r = draw(gpu, r_rows, &f, &stores, rows, wait).lighting;
                assert_eq!((r.suns, r.suns_built + r.suns_updated), (suns, suns));
                rebuilt += r.suns_built;
            });
            // Back, untimed (a jump that far builds the map whole).
            draw(gpu, r_rows, &frame!(0, projection), &stores, rows, true);
            timed
        };
        let (ahead, back) = (scroll(1.0), scroll(-1.0));
        let all = [rest, built, ahead, back];
        for (list, value) in timed.iter_mut().zip(all) {
            list.push(value);
        }
    }
    (timed, rebuilt)
}
