//! Throwaway probe for ADR 0019 ("light rows"): can one row per lamp, looked up per pixel, replace
//! the ray march per pixel and light of the Chiaro port? Picture and time
//! on `iso_night_hard`, both sides cut down the same way: only the point and spot lights that
//! cast shadows (13), hard shadows, no profile, mask, pattern, animation, flicker, medium, area
//! lights, sun, zones, material.
//!
//! - Today: [`Light2dRenderer`] unchanged.
//! - New: the obstacle raster of that renderer (mask and heights), then two passes of the
//!   shaders `lightrow*.hlsl` (row build, light with the lookup in the row), then the composite
//!   stage and the post pass as they are.
//! - True: the new light with an exact march per pixel (`lightrow_truth.frag`) as the reference
//!   for both.
//!
//! Environment: `KNORKE_PROBE_OUT=<dir>` writes `<scene>_heute.png`, `<scene>_neu.png`,
//! `<scene>_wahr.png`, the differences times 8 and appends the numbers to `messung.txt`;
//! `KNORKE_PROBE_FRAMES=<n>` times n frames per run (wall clock: prepare + record + submit +
//! wait, as `KNORKE_SCENE_BENCH`), `KNORKE_PROBE_RUNS` runs per case (default 5), today and new
//! in turn; `KNORKE_PROBE_SHADOW` the mask resolution (default 0.5, the demo's),
//! `KNORKE_PROBE_ANGLES` the angles per row (default 1024), `KNORKE_PROBE_STEP` the march step
//! in mask texels (default 0.5), `KNORKE_PROBE_NEAR` the texels before a receiver that do not
//! count (default 1, Chiaro's), `KNORKE_PROBE_ONLY=<i>` keeps lamp i alone (to narrow a
//! difference down), `KNORKE_PROBE_BLEND=1` blends the runs of the two neighbouring angles,
//! `KNORKE_PROBE_SOFT=1` gives the lamps the sizes of `plain.txt` (soft shadows; the new light
//! then runs the edge pass after the row build, at most `KNORKE_PROBE_EDGE` angles to each side,
//! default 32, as many as an obstacle `KNORKE_PROBE_EDGE_NEAR` texels away needs, default 16; the reference takes `KNORKE_PROBE_POINTS` squared lamp points, default 8), `KNORKE_PROBE_MOVE=<i>` also walks lamp i 64 steps of 0.05
//! world units and measures the flicker (`bewegt_*.png`: four pictures in a row around the lamp).
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

use std::fmt::Write as _;
use std::path::PathBuf;

use knorke_contract_tests::parity::save_png_rgba8;
use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT,
    SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUTextureFormat,
};
use knorke_gpu::test_support::{golden_bytes, read_texture, texture, with_chiaro_pack};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, ProgramDesc,
    SamplerBinding, Samplers, SecondTarget, ShaderLibrary, StageBindings, Texture, TextureRef,
    UploadBuffer,
};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dRenderer, Light2dSettings};
use knorke_light2d::composite::{CompositeFrame, CompositeLight, CompositeStage};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::light_math::ground_radius;
use knorke_light2d::lighting::{ShadowDomain, SurfaceParams, ambient_clear, height_tolerance};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{SceneStores, max_height};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::{GlowColor, LightSettings, Quality, QualitySettings, Resolutions};
use knorke_light2d::shadow::{MaskAnchor, OcclusionDomain};
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
/// The moving lamp: steps, world units per step (less than a mask texel), the cut of the strips.
const MOVE_STEPS: u32 = 64;
const MOVE_STEP: f32 = 0.05;
const CUT: (u32, u32) = (320, 300);
const HALF: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
/// Row texels hold packed pairs of 12-bit numbers, which need a 32-bit float.
const ROW: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT;
/// Bytes of `struct Lamp` in `lightrow.hlsli`.
const LAMP_BYTES: usize = 80;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn num(s: &str) -> f32 {
    s.parse().unwrap()
}

/// The scene text cut down to what the probe compares: the enabled point and spot lights that
/// cast shadows (every extra switched off) and the enabled obstacles without their patterns.
fn parse(text: &str, soft: bool) -> (Vec<LightDesc>, ObstacleStore) {
    let (mut lights, mut obstacles) = (Vec::new(), ObstacleStore::new());
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let v = |i: usize| num(f[i]);
        match f[0] {
            "light" => {
                let kind = match f[1] {
                    "0" => LightType::Point,
                    "1" => LightType::Spot,
                    _ => continue,
                };
                // CHI_LIGHT_NO_SHADOWS, _DISABLED; CHI_HEIGHT_AUTO is -1.
                if f[2].parse::<u32>().unwrap() & 3 != 0 || v(5) < 0.0 {
                    continue;
                }
                let mut d = LightDesc::new(kind);
                d.position = At::new(Vec2::new(v(3), v(4)));
                d.height = Height::At(v(5));
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
                lights.push(d);
            }
            "shape" => {
                // CHI_OBSTACLE_DYNAMIC, _DISABLED, _FOOTPRINT_SHADOW.
                let flags: u32 = f[2].parse().unwrap();
                let common = ObstacleCommon {
                    position: At::new(Vec2::new(v(3), v(4))),
                    rotation: v(5),
                    height: v(6),
                    base_height: v(7),
                    opacity: 1.0,
                    dynamic: flags & 1 != 0,
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
                obstacles
                    .create_shape(&ShapeDesc { common, shape })
                    .unwrap();
            }
            _ => {}
        }
    }
    (lights, obstacles)
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

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The programs: row build, light from the rows, light from the exact march.
const BUILD: usize = 0;
const LIGHT: usize = 1;
const TRUTH: usize = 2;
const EDGE: usize = 3;

/// What a frame of the new light reads from outside.
struct Inputs<'a> {
    scene: TextureRef<'a>,
    glow: TextureRef<'a>,
    surface: TextureRef<'a>,
    mask: TextureRef<'a>,
    heights: TextureRef<'a>,
    target: TextureRef<'a>,
    projection: &'a Projection,
    settings: &'a Light2dSettings,
}

/// The new light stage: rows, light buffer; then composite and post.
struct Probe<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    composite: CompositeStage<'d>,
    post: PostRenderer<'d>,
    lamps: UploadBuffer<'d>,
    count: u32,
    rows: [Texture<'d>; 2],
    /// What `lightrow_edge.frag` writes; built only if a lamp has a size.
    edges: [Texture<'d>; 2],
    soft: bool,
    edge_params: Vec<u8>,
    light: Texture<'d>,
    domain: OcclusionDomain,
    remainder: [f32; 2],
    /// The fewest world units a mask texel covers in any direction.
    shortest: f32,
    /// Uniform blocks: the vertex pass block of the row build and of the light, row, surface,
    /// lookup.
    pass: [Vec<u8>; 2],
    row_params: Vec<u8>,
    surface_params: Vec<u8>,
    lookup: Vec<u8>,
    built: bool,
}

const PROGRAMS: [ProgramDesc; 4] = [
    ProgramDesc {
        second: Some(SecondTarget {
            format: ROW,
            blend: Blend::Opaque,
        }),
        ..ProgramDesc::new("lightrow.vert", "lightrow_build.frag", Blend::Opaque)
    },
    ProgramDesc::new("lightrow.vert", "lightrow_light.frag", Blend::Additive),
    ProgramDesc::new("lightrow.vert", "lightrow_truth.frag", Blend::Additive),
    ProgramDesc {
        second: Some(SecondTarget {
            format: ROW,
            blend: Blend::Opaque,
        }),
        ..ProgramDesc::new("lightrow.vert", "lightrow_edge.frag", Blend::Opaque)
    },
];
const FORMATS: [SDL_GPUTextureFormat; 4] = [ROW, HALF, HALF, ROW];

impl<'d> Probe<'d> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        gpu: Gpu<'d>,
        pack: &[u8],
        lights: &[LightDesc],
        obstacles: &ObstacleStore,
        projection: &Projection,
        settings: &Light2dSettings,
        angles: u32,
        step: f32,
    ) -> Result<Self, Diagnostic> {
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec())?;
        let mut pipelines = PipelineCache::new(&shaders, &PROGRAMS)?;
        for (i, format) in FORMATS.into_iter().enumerate() {
            pipelines.get(&mut shaders, i, format)?;
        }
        let domain = OcclusionDomain::new(W, H, settings.light.resolutions().shadow);
        let anchor = MaskAnchor::compute(projection, &domain);
        let shadow = ShadowDomain::new(&domain, anchor.remainder);
        let surface = SurfaceParams::new(projection, W, H, &settings.light, &shadow, true);
        // A vector in mask texels as a world vector.
        let [tx, ty] = domain.texels_per_scene;
        let [a, b, c, d] = surface.inv_axes;
        let world_from_mask = [a / tx, b / ty, c / tx, d / ty];
        // The fewest world units a mask texel covers in any direction: the reach of a lamp in
        // mask texels is its radius divided by it.
        let shortest = (0..720)
            .map(|k| {
                let (s, c) = (k as f32).to_radians().sin_cos();
                let (wx, wy) = (
                    world_from_mask[0] * c + world_from_mask[1] * s,
                    world_from_mask[2] * c + world_from_mask[3] * s,
                );
                (wx * wx + wy * wy).sqrt()
            })
            .fold(f32::INFINITY, f32::min);

        // The row packs distances up to 1023.75 texels.
        let reach = lights.iter().map(|l| l.radius).fold(0.0, f32::max) / shortest + 1.0;
        assert!(reach < 1023.0, "reach {reach} texels does not fit the row");
        eprintln!("longest reach {reach:.1} mask texels");

        // Angles the edge pass walks to each side: a penumbra reaches no further across.
        let reach_angles = env("KNORKE_PROBE_EDGE", 32.0_f32).min(63.0);
        let count = lights.len() as u32;
        let mut lamps = UploadBuffer::new(
            gpu,
            SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
            (lights.len() * LAMP_BYTES) as u32,
            "lamps",
        );
        lamps.reserve((lights.len() * LAMP_BYTES) as u64)?;
        let target = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
        let o = projection.origin.raw();
        let (x, y, z) = (projection.x_axis, projection.y_axis, projection.z_axis);
        let top = max_height(
            true,
            obstacles.max_finite_top(),
            obstacles.any_infinite_enabled(),
            false,
            HEIGHT_STEP,
        );
        let pass = |mode: f32, atlas_height: f32| {
            floats(&[
                o.x,
                o.y,
                x.x,
                x.y, //
                y.x,
                y.y,
                W as f32,
                H as f32, //
                z.x,
                z.y,
                top,
                0.0, //
                mode,
                atlas_height,
                1.0,
                0.0,
            ])
        };
        let mut probe = Self {
            composite: CompositeStage::new(gpu, &shaders)?,
            post: PostRenderer::new(gpu, pack.to_vec())?,
            samplers: Samplers::new(gpu)?,
            shaders,
            pipelines,
            lamps,
            count,
            rows: [(); 2].map(|()| texture(gpu, (angles, count), ROW, target, None)),
            edges: [(); 2].map(|()| texture(gpu, (angles, count), ROW, target, None)),
            soft: lights.iter().any(|l| l.source_radius > 0.0),
            edge_params: floats(&[
                angles as f32,
                count as f32,
                reach_angles,
                0.0,
                shortest * std::f32::consts::TAU / angles as f32,
                env("KNORKE_PROBE_EDGE_NEAR", 16.0),
                0.0,
                0.0,
            ]),
            light: texture(gpu, (W, H), HALF, target, None),
            domain,
            remainder: anchor.remainder,
            shortest,
            pass: [pass(0.0, count as f32), pass(2.0, 0.0)],
            row_params: floats(&[
                world_from_mask[0],
                world_from_mask[1],
                world_from_mask[2],
                world_from_mask[3],
                domain.width as f32,
                domain.height as f32,
                step,
                0.0,
            ]),
            surface_params: surface.to_bytes().to_vec(),
            lookup: floats(&[
                angles as f32,
                count as f32,
                env("KNORKE_PROBE_NEAR", 1.0),
                height_tolerance(&settings.light),
                domain.width as f32,
                domain.height as f32,
                env("KNORKE_PROBE_BLEND", 0.0),
                env("KNORKE_PROBE_POINTS", 8.0),
                world_from_mask[0],
                world_from_mask[1],
                world_from_mask[2],
                world_from_mask[3],
                reach_angles,
                0.0,
                0.0,
                0.0,
            ]),
            built: false,
        };
        probe.set_lamps(lights, projection)?;
        Ok(probe)
    }

    /// Writes the lamps (as many as at the start); the next frame that rebuilds uploads them.
    fn set_lamps(
        &mut self,
        lights: &[LightDesc],
        projection: &Projection,
    ) -> Result<(), Diagnostic> {
        assert_eq!(lights.len() as u32, self.count);
        let mut mapped = self.lamps.map()?;
        let out = mapped.bytes().as_chunks_mut::<LAMP_BYTES>().0;
        for (slot, (bytes, l)) in out.iter_mut().zip(lights).enumerate() {
            let Height::At(z) = l.height else {
                unreachable!("parse keeps lamps with a height only")
            };
            let p = l.position.raw();
            let dir = l.direction;
            let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
            let spot = l.kind == LightType::Spot;
            let ground = self
                .domain
                .scene_to_domain(projection.project(l.position, 0.0).raw());
            let lamp = [
                [p.x, p.y, z, l.radius],
                [
                    l.color.x * l.intensity,
                    l.color.y * l.intensity,
                    l.color.z * l.intensity,
                    l.falloff,
                ],
                [dir.x / len, dir.y / len, dir.z / len, f32::from(spot)],
                [
                    l.inner_angle.to_radians().cos(),
                    l.outer_angle.to_radians().cos(),
                    ground_radius(z, l.radius),
                    l.radius / self.shortest + 1.0,
                ],
                [
                    ground.x - self.remainder[0],
                    ground.y - self.remainder[1],
                    slot as f32,
                    l.source_radius,
                ],
            ];
            bytes.copy_from_slice(&floats(lamp.as_flattened()));
        }
        Ok(())
    }

    /// One pass: `targets` drawn with program `index`, one instance per lamp.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        cmd: &mut CommandBuffer<'_>,
        index: usize,
        lamps: u32,
        targets: &[ColorTarget<'_>],
        samplers: &[SamplerBinding<'_>],
        uniforms: &[&[u8]],
    ) -> Result<(), Diagnostic> {
        let pipeline = self.pipelines.pipeline(index, FORMATS[index]).unwrap();
        let buffer = self.lamps.buffer_ref().unwrap();
        let mut pass = cmd.render_pass(targets)?;
        pass.bind_program(
            pipeline,
            &Bindings {
                vertex: StageBindings {
                    storage_buffers: &[buffer],
                    uniforms: &[&self.pass[usize::from(index == LIGHT || index == TRUTH)]],
                    ..Default::default()
                },
                fragment: StageBindings {
                    samplers,
                    uniforms,
                    ..Default::default()
                },
            },
        )?;
        pass.draw_instanced(6, lamps)
    }

    /// Prepares, records and submits one frame; `rebuild` builds every row anew, else only the
    /// first frame does (the lamps rest). `lamps` of them are drawn: 0 leaves the ambient clear,
    /// composite and post, the part of a frame no light stage can save. `truth` lights with the
    /// exact march instead of the rows.
    fn frame(
        &mut self,
        gpu: Gpu<'_>,
        i: &Inputs<'_>,
        rebuild: bool,
        lamps: u32,
        truth: bool,
    ) -> Result<(), Diagnostic> {
        let light = &i.settings.light;
        let composite = CompositeFrame {
            scene: i.scene,
            glow: Some(i.glow),
            material: None,
            surface: Some(i.surface),
            light: CompositeLight {
                ambient: light.ambient,
                light_buffer: Some(self.light.texture_ref()),
                use_specular: false,
                specular: None,
                direction: None,
                pure_ambient: None,
            },
            medium: None,
            projection: i.projection,
            settings: light,
            input_space: i.settings.input_space(),
        };
        self.composite.prepare(&mut self.shaders, &composite)?;
        let post = PostFrame {
            hdr: self.composite.hdr_buffer().unwrap(),
            overlay: None,
            target: i.target,
            shockwaves: None,
        };
        let prepared = self.post.prepare(&post, &i.settings.post)?;

        let mut cmd = CommandBuffer::acquire(gpu.device())?;
        let nearest = self.samplers.sampler(Filter::Nearest);
        let [first, second] = [self.rows[0].texture_ref(), self.rows[1].texture_ref()];
        let edges = [self.edges[0].texture_ref(), self.edges[1].texture_ref()];
        if rebuild || !self.built {
            let bytes = self.count * LAMP_BYTES as u32;
            cmd.copy_pass()?.upload_to_buffer(&self.lamps, bytes)?;
            self.draw(
                &mut cmd,
                BUILD,
                self.count,
                &[
                    ColorTarget::new(first, Load::DontCare),
                    ColorTarget::new(second, Load::DontCare),
                ],
                &[SamplerBinding::new(i.heights, nearest)],
                &[&self.row_params],
            )?;
            if self.soft {
                self.draw(
                    &mut cmd,
                    EDGE,
                    self.count,
                    &[
                        ColorTarget::new(edges[0], Load::DontCare),
                        ColorTarget::new(edges[1], Load::DontCare),
                    ],
                    &[
                        SamplerBinding::new(first, nearest),
                        SamplerBinding::new(second, nearest),
                    ],
                    &[&self.edge_params],
                )?;
            }
            self.built = true;
        }
        let samplers = [
            i.surface, i.mask, i.heights, first, second, edges[0], edges[1],
        ]
        .map(|texture| SamplerBinding::new(texture, nearest));
        self.draw(
            &mut cmd,
            if truth { TRUTH } else { LIGHT },
            lamps,
            &[ColorTarget::new(
                self.light.texture_ref(),
                Load::Clear(ambient_clear(light.ambient)),
            )],
            &samplers[..if truth { 3 } else { 7 }],
            &[&self.surface_params, &self.lookup],
        )?;
        self.composite
            .record(&mut cmd, &self.samplers, &composite)?;
        self.post
            .record(&mut cmd, &post, &i.settings.post, prepared)?;
        cmd.submit_and_wait()
    }
}

/// Median and 95th percentile of `us`.
fn stats(mut us: Vec<f64>) -> (f64, f64) {
    us.sort_by(f64::total_cmp);
    (us[us.len() / 2], us[us.len() * 95 / 100])
}

/// `frames` calls of `frame`, each timed: the host times frames (wall clock is a host tool).
#[allow(clippy::disallowed_methods)]
fn time(frames: usize, mut frame: impl FnMut()) -> (f64, f64) {
    let mut us = Vec::with_capacity(frames);
    for _ in 0..frames {
        let t0 = std::time::Instant::now();
        frame();
        us.push(t0.elapsed().as_secs_f64() * 1e6);
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

#[test]
fn light_rows_against_chiaro() {
    let out: Option<PathBuf> = std::env::var("KNORKE_PROBE_OUT").ok().map(PathBuf::from);
    let frames: usize = env("KNORKE_PROBE_FRAMES", 0);
    let runs: usize = env("KNORKE_PROBE_RUNS", 5);
    let shadow: f32 = env("KNORKE_PROBE_SHADOW", 0.5);
    let angles: u32 = env("KNORKE_PROBE_ANGLES", 1024);
    let step: f32 = env("KNORKE_PROBE_STEP", 0.5);
    let soft = env("KNORKE_PROBE_SOFT", 0) != 0;
    let index = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|s| s.parse::<usize>().unwrap())
    };
    let (only, mover) = (index("KNORKE_PROBE_ONLY"), index("KNORKE_PROBE_MOVE"));
    with_chiaro_pack(|gpu, pack| {
        let text = String::from_utf8(golden_bytes("iso_night", "hard.txt")).unwrap();
        let projection = Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H);
        let (mut base, obstacles) = parse(&text, false);
        if soft {
            // The lamp sizes of the variant with soft shadows, on the same lamps.
            let plain = String::from_utf8(golden_bytes("iso_night", "plain.txt")).unwrap();
            let sized = parse(&plain, true).0;
            assert_eq!(sized.len(), base.len());
            for (d, s) in base.iter_mut().zip(&sized) {
                assert_eq!(d.position, s.position);
                d.source_radius = s.source_radius;
            }
        }
        assert_eq!(
            base.len(),
            13,
            "the shadowed point and spot lights of the scene"
        );
        if let Some(i) = only {
            base = vec![base[i].clone()];
        }
        let settings = settings(shadow);
        let mut report = format!(
            "mask resolution {shadow}, {angles} angles, march step {step}, {frames} frames per run, lamp {only:?}\n"
        );

        let kinds = LayerSet::of(Layer::Surface).with(Layer::Glow);
        let mut layers = Layers::new(gpu, W, H, kinds).unwrap();
        let fixture = |name: &str| scene_common::fixture("iso_night", name, (W, H));
        let (color, glow, surface) = (fixture("scene"), fixture("glow"), fixture("surface"));
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
        let uploads = scene_common::upload_layers(
            gpu,
            &mut cmd,
            &mut layers,
            &[
                (Layer::Scene, &color),
                (Layer::Glow, &glow),
                (Layer::Surface, &surface),
            ],
        );
        cmd.submit_and_wait().unwrap();
        drop(uploads);
        let target = || {
            texture(
                gpu,
                OUT,
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            )
        };
        let (target_today, target_new, target_true) = (target(), target(), target());

        // The load scene: every lamp four times, slightly apart.
        let offsets = [(0.0, 0.0), (0.7, 0.3), (-0.5, 0.6), (0.4, -0.7)];
        for (name, copies) in [("normal", 1), ("last", 4)] {
            let descs: Vec<LightDesc> = offsets[..copies]
                .iter()
                .flat_map(|&(dx, dy)| {
                    base.iter().map(move |d| {
                        let p = d.position.raw();
                        LightDesc {
                            position: At::new(Vec2::new(p.x + dx, p.y + dy)),
                            ..d.clone()
                        }
                    })
                })
                .collect();
            let mut lights = LightStore::new();
            for d in &descs {
                lights.create(d, &NoResources).unwrap();
            }
            let stores = SceneStores::new(&lights, &obstacles);
            let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
            let frame = layers.frame(target_today.texture_ref(), &projection, LayerSet::NONE);
            let today = |renderer: &mut Light2dRenderer<'_>| {
                let prepared = renderer.prepare(&frame, &stores, &settings).unwrap();
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                renderer
                    .record(&mut cmd, &frame, &settings, prepared)
                    .unwrap();
                cmd.submit_and_wait().unwrap();
            };
            today(&mut renderer);
            let mut probe = Probe::new(
                gpu,
                pack,
                &descs,
                &obstacles,
                &projection,
                &settings,
                angles,
                step,
            )
            .unwrap();

            let mut timed: [Vec<(f64, f64)>; 4] = Default::default();
            for run in 0..=runs {
                // Run 0 draws the pictures; the timed runs alternate today and new.
                let n = if run == 0 { 1 } else { frames };
                if n == 0 {
                    break;
                }
                let t = time(n, || today(&mut renderer));
                let occlusion = renderer.plan().occlusion();
                let inputs = Inputs {
                    scene: frame.scene,
                    glow: frame.glow.unwrap(),
                    surface: frame.surface.unwrap(),
                    mask: occlusion.mask().unwrap(),
                    heights: occlusion.height().unwrap(),
                    target: target_new.texture_ref(),
                    projection: &projection,
                    settings: &settings,
                };
                let all = descs.len() as u32;
                if run == 0 {
                    let truth = Inputs {
                        target: target_true.texture_ref(),
                        ..inputs
                    };
                    probe.frame(gpu, &truth, false, all, true).unwrap();
                }
                let floor = time(n, || probe.frame(gpu, &inputs, false, 0, false).unwrap());
                let moved = time(n, || probe.frame(gpu, &inputs, true, all, false).unwrap());
                // Last, so the pictures show the resting case.
                let rest = time(n, || probe.frame(gpu, &inputs, false, all, false).unwrap());
                if run > 0 {
                    for (list, value) in timed.iter_mut().zip([t, rest, moved, floor]) {
                        list.push(value);
                    }
                    eprintln!(
                        "{name} run {run}: today {:.1} / {:.1}, new resting {:.1} / {:.1}, new rebuilt {:.1} / {:.1}, no lamps {:.1} / {:.1} us (median / p95)",
                        t.0, t.1, rest.0, rest.1, moved.0, moved.1, floor.0, floor.1
                    );
                }
            }
            let pictures = [
                ("heute", read_texture(gpu, target_today.texture_ref())),
                ("neu", read_texture(gpu, target_new.texture_ref())),
                ("wahr", read_texture(gpu, target_true.texture_ref())),
            ];
            let pixels = (OUT.0 * OUT.1) as usize;
            let mut diffs = Vec::new();
            for (i, j) in [(0, 2), (1, 2), (0, 1)] {
                let ((first, a), (second, b)) = (&pictures[i], &pictures[j]);
                let (mut sum, mut over) = (0u64, 0usize);
                let mut diff = Vec::with_capacity(a.len());
                for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
                    let d = [0, 1, 2].map(|c| p[c].abs_diff(q[c]));
                    sum += d.iter().map(|&v| u64::from(v)).sum::<u64>();
                    over += usize::from(d.into_iter().max().unwrap() > 8);
                    diff.extend([0, 1, 2].map(|c| d[c].saturating_mul(8)));
                    diff.push(255);
                }
                let _ = writeln!(
                    report,
                    "{name:8} {} lamps, {first} against {second}: mean absolute difference {:.3} of 255, {:.2} % of the pixels differ by more than 8",
                    descs.len(),
                    sum as f64 / (pixels * 3) as f64,
                    100.0 * over as f64 / pixels as f64
                );
                diffs.push((format!("diff_{first}_{second}_x8"), diff));
            }
            if frames > 0 {
                let cases = [
                    "today",
                    "new, lamps rest",
                    "new, all rebuilt",
                    "new, no lamps",
                ];
                for (case, list) in cases.into_iter().zip(&timed) {
                    let _ = writeln!(report, "{}", summary(name, case, list));
                }
            }
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let named = pictures.iter().map(|(n, px)| (*n, px));
                for (suffix, px) in named.chain(diffs.iter().map(|(n, px)| (n.as_str(), px))) {
                    let file = dir.join(format!("{name}_{suffix}.png"));
                    save_png_rgba8(&file, px, OUT.0, OUT.1);
                }
            }
        }
        if let Some(mover) = mover {
            let mut descs = base.clone();
            let mut lights = LightStore::new();
            let ids: Vec<_> = descs
                .iter()
                .map(|d| lights.create(d, &NoResources).unwrap())
                .collect();
            let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
            let mut probe = Probe::new(
                gpu,
                pack,
                &descs,
                &obstacles,
                &projection,
                &settings,
                angles,
                step,
            )
            .unwrap();
            let frame = layers.frame(target_today.texture_ref(), &projection, LayerSet::NONE);
            let start = descs[mover].position.raw();
            // The cut for the strips: around the middle of the walk, in output pixels.
            let middle = At::new(Vec2::new(
                start.x,
                start.y + 0.5 * MOVE_STEPS as f32 * MOVE_STEP,
            ));
            let centre = projection.project(middle, 0.0).raw();
            let scale = OUT.0 / W;
            let corner = |c: f32, size: u32, full: u32| {
                ((c * scale as f32) as i64 - i64::from(size / 2)).clamp(0, i64::from(full - size))
                    as usize
            };
            let (cx, cy) = (
                corner(centre.x, CUT.0, OUT.0),
                corner(centre.y, CUT.1, OUT.1),
            );
            let names = ["heute", "neu", "wahr"];
            let mut strips: [Vec<u8>; 3] = Default::default();
            let mut last: Option<[Vec<u8>; 3]> = None;
            // Per method the summed change between two pictures; for today and new the part of
            // it the truth does not share, and the worst pair of that.
            let (mut change, mut excess, mut worst) = ([0u64; 3], [0u64; 2], [0u64; 2]);
            for i in 0..=MOVE_STEPS {
                let at = At::new(Vec2::new(start.x, start.y + i as f32 * MOVE_STEP));
                descs[mover].position = at;
                lights
                    .set_position(ids[mover], at, descs[mover].height)
                    .unwrap();
                {
                    let stores = SceneStores::new(&lights, &obstacles);
                    let prepared = renderer.prepare(&frame, &stores, &settings).unwrap();
                    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                    renderer
                        .record(&mut cmd, &frame, &settings, prepared)
                        .unwrap();
                    cmd.submit_and_wait().unwrap();
                }
                probe.set_lamps(&descs, &projection).unwrap();
                let occlusion = renderer.plan().occlusion();
                let inputs = Inputs {
                    scene: frame.scene,
                    glow: frame.glow.unwrap(),
                    surface: frame.surface.unwrap(),
                    mask: occlusion.mask().unwrap(),
                    heights: occlusion.height().unwrap(),
                    target: target_true.texture_ref(),
                    projection: &projection,
                    settings: &settings,
                };
                let all = descs.len() as u32;
                probe.frame(gpu, &inputs, true, all, true).unwrap();
                let inputs = Inputs {
                    target: target_new.texture_ref(),
                    ..inputs
                };
                probe.frame(gpu, &inputs, true, all, false).unwrap();
                let now = [&target_today, &target_new, &target_true]
                    .map(|t| read_texture(gpu, t.texture_ref()));
                if let Some(before) = &last {
                    let mut pair = [0u64; 2];
                    for k in (0..now[0].len()).filter(|k| k % 4 != 3) {
                        let d = [0, 1, 2].map(|m| i32::from(now[m][k]) - i32::from(before[m][k]));
                        for m in 0..3 {
                            change[m] += u64::from(d[m].unsigned_abs());
                        }
                        for m in 0..2 {
                            pair[m] += u64::from((d[m] - d[2]).unsigned_abs());
                        }
                    }
                    for m in 0..2 {
                        excess[m] += pair[m];
                        worst[m] = worst[m].max(pair[m]);
                    }
                }
                if (MOVE_STEPS / 2..MOVE_STEPS / 2 + 4).contains(&i) {
                    for (strip, picture) in strips.iter_mut().zip(&now) {
                        strip.extend_from_slice(picture);
                    }
                }
                last = Some(now);
            }
            let values = (OUT.0 * OUT.1 * 3) as f64;
            let all = values * f64::from(MOVE_STEPS);
            let _ = writeln!(
                report,
                "moving lamp {mover}, {MOVE_STEPS} steps of {MOVE_STEP}: mean change per step today {:.4}, new {:.4}, truth {:.4} of 255",
                change[0] as f64 / all,
                change[1] as f64 / all,
                change[2] as f64 / all
            );
            let _ = writeln!(
                report,
                "  change the truth does not share: today {:.4} (worst pair {:.4}), new {:.4} (worst pair {:.4}), new / today {:.3}",
                excess[0] as f64 / all,
                worst[0] as f64 / values,
                excess[1] as f64 / all,
                worst[1] as f64 / values,
                excess[1] as f64 / excess[0] as f64
            );
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                for (name, strip) in names.into_iter().zip(&strips) {
                    // Four pictures one below the other in `strip`; side by side in the file.
                    let full = (OUT.0 * OUT.1 * 4) as usize;
                    let mut px = Vec::with_capacity((CUT.0 * CUT.1 * 16) as usize);
                    for y in 0..CUT.1 as usize {
                        for picture in strip.chunks_exact(full) {
                            let from = ((cy + y) * OUT.0 as usize + cx) * 4;
                            px.extend_from_slice(&picture[from..from + CUT.0 as usize * 4]);
                        }
                    }
                    let file = dir.join(format!("bewegt_{name}.png"));
                    save_png_rgba8(&file, &px, CUT.0 * 4, CUT.1);
                }
            }
        }
        eprintln!("{report}");
        if let Some(dir) = &out {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("messung.txt"))
                .unwrap();
            writeln!(file, "{report}").unwrap();
        }
    });
}
