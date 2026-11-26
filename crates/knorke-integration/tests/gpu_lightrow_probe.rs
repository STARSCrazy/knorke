//! Measuring tool for the light rows (ADR 0019, plan 17.13): picture and frame time of the row
//! lights of `knorke-light2d` against the Chiaro light and against a reference picture, on
//! `iso_night_hard` cut down to what rows can draw: the point and spot lights that cast shadows
//! (13), no profile, mask, pattern, animation, flicker, medium, area lights, sun, zones,
//! material. Hard shadows, and soft ones with the lamp sizes of `plain.txt`; each as it is
//! ("normal") and with every lamp four times ("last").
//!
//! - Today: [`Light2dRenderer`] with `light_rows` off.
//! - New: the same with `light_rows` on.
//! - True: the same light with an exact march per pixel (`lightrow_truth.frag`), the only part
//!   this file draws itself; a lamp with a size is `KNORKE_PROBE_POINTS` squared points
//!   (default 8).
//!
//! The test always checks that the rows are closer to the reference than today's light.
//! `KNORKE_PROBE_OUT=<dir>` writes `<scene>_heute.png`, `<scene>_neu.png`, `<scene>_wahr.png`,
//! the differences times 8 and `messung.txt`; `KNORKE_PROBE_FRAMES=<n>` times n frames per run
//! (wall clock: prepare + record + submit + wait), `KNORKE_PROBE_RUNS` runs per case (default
//! 5), the cases in turn: today, rows with resting lamps, rows with every lamp moved every frame
//! (all rows built), and a frame without lights (what no light stage can save).
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
use knorke_light2d::composite::{CompositeFrame, CompositeLight, CompositeStage};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::lighting::{
    LightSurfacePassParams, ShadowDomain, SurfaceParams, ambient_clear, height_tolerance,
};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassRecord, SceneStores, max_height};
use knorke_light2d::projection::Projection;
use knorke_light2d::rows::{RowLamp, RowLookupParams, RowSpace};
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

const TRUTH: [ProgramDesc; 1] = [ProgramDesc::new(
    "lightrow.vert",
    "lightrow_truth.frag",
    Blend::Additive,
)];

/// The reference picture: the row lights of a frame lit by `lightrow_truth.frag`, then the
/// composite stage and the post pass as the renderer runs them.
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
        pipelines.get(&mut shaders, 0, HALF)?;
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
            rows: true,
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
                };
                *bytes = lamp.to_bytes();
            }
        }
        let domain = OcclusionDomain::new(W, H, light.resolutions().shadow);
        let anchor = MaskAnchor::compute(p, &domain);
        let shadow = ShadowDomain::new(&domain, anchor.remainder);
        let surface = SurfaceParams::new(p, W, H, light, &shadow, true).to_bytes();
        let mask = [domain.width, domain.height];
        let mut lookup =
            RowLookupParams::new(&RowSpace::new(p, &domain), mask, 1, height_tolerance(light));
        lookup.domain[3] = env("KNORKE_PROBE_POINTS", 8.0);
        let lookup = lookup.to_bytes();
        let vertex = LightSurfacePassParams::new(p, W, H, top, 0).to_bytes();

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
            medium: None,
            projection: p,
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
        cmd.copy_pass()?
            .upload_to_buffer(&self.lamps, (count * RowLamp::SIZE) as u32)?;
        let nearest = self.samplers.sampler(Filter::Nearest);
        let occlusion = renderer.plan().occlusion();
        let textures = [
            frame.surface.unwrap(),
            occlusion.mask().unwrap(),
            occlusion.height().unwrap(),
        ]
        .map(|t| SamplerBinding::new(t, nearest));
        {
            let clear = Load::Clear(ambient_clear(light.ambient));
            let mut pass = cmd.render_pass(&[ColorTarget::new(self.light.texture_ref(), clear)])?;
            pass.bind_program(
                self.pipelines.pipeline(0, HALF).unwrap(),
                &Bindings {
                    vertex: StageBindings {
                        storage_buffers: &[self.lamps.buffer_ref().unwrap()],
                        uniforms: &[&vertex],
                        ..Default::default()
                    },
                    fragment: StageBindings {
                        samplers: &textures,
                        uniforms: &[&surface, &lookup],
                        ..Default::default()
                    },
                },
            )?;
            pass.draw_instanced(6, count as u32)?;
        }
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
) -> PassRecord {
    let prepared = renderer.prepare(frame, stores, settings).unwrap();
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    let record = renderer
        .record(&mut cmd, frame, settings, prepared)
        .unwrap();
    cmd.submit_and_wait().unwrap();
    record
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
/// 0.348 / 0.66, 0.640 / 1.60, 0.631 / 2.31, 0.869 / 3.03; today's light 0.921 / 1.74,
/// 1.808 / 4.22, 1.662 / 5.44, 1.699 / 5.24.
const BOUNDS: [(&str, f64, f64); 4] = [
    ("hart normal", 0.39, 0.73),
    ("hart last", 0.71, 1.76),
    ("weich normal", 0.70, 2.55),
    ("weich last", 0.96, 3.34),
];

#[test]
fn light_rows_against_chiaro_and_the_reference() {
    let out: Option<PathBuf> = std::env::var("KNORKE_PROBE_OUT").ok().map(PathBuf::from);
    let frames: usize = env("KNORKE_PROBE_FRAMES", 0);
    let runs: usize = env("KNORKE_PROBE_RUNS", 5);
    with_chiaro_pack(|gpu, pack| {
        let text = String::from_utf8(golden_bytes("iso_night", "hard.txt")).unwrap();
        let plain = String::from_utf8(golden_bytes("iso_night", "plain.txt")).unwrap();
        let projection = Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H);
        let today = settings(0.5);
        let mut rows = today;
        rows.light.light_rows = true;
        let mut report = format!("{frames} frames per run\n");
        let mut full = String::new();

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
        let targets = [target(), target(), target()];
        let frame = |i: usize| layers.frame(targets[i].texture_ref(), &projection, LayerSet::NONE);
        let mut truth = Truth::new(gpu, pack).unwrap();

        // The load scene: every lamp four times, slightly apart.
        let offsets = [(0.0, 0.0), (0.7, 0.3), (-0.5, 0.6), (0.4, -0.7)];
        let scenes = [false, true]
            .into_iter()
            .flat_map(|soft| [(soft, 1), (soft, 4)]);
        for ((soft, copies), bound) in scenes.zip(BOUNDS) {
            let name = bound.0;
            let (mut base, obstacles) = parse(&text, false);
            if soft {
                // The lamp sizes of the variant with soft shadows, on the same lamps.
                let sized = parse(&plain, true).0;
                assert_eq!(sized.len(), base.len());
                for (d, s) in base.iter_mut().zip(&sized) {
                    assert_eq!(d.position, s.position);
                    d.source_radius = s.source_radius;
                }
            }
            assert_eq!(base.len(), 13, "the scene's shadowed point and spot lights");
            let descs: Vec<LightDesc> = offsets[..copies]
                .iter()
                .flat_map(|&(dx, dy)| {
                    base.iter().map(move |d| LightDesc {
                        position: At::new(d.position.raw() + Vec2::new(dx, dy)),
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
            let new = || Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
            let mut renderers = [new(), new(), new(), new()];
            let all = descs.len() as u32;

            let mut timed: [Vec<(f64, f64)>; 4] = Default::default();
            let mut step = 0u32;
            for run in 0..=runs {
                // Run 0 draws the pictures; the timed runs take the cases in turn.
                let n = if run == 0 { 1 } else { frames };
                if n == 0 {
                    break;
                }
                let [r_today, r_rest, r_moved, r_none] = &mut renderers;
                let stores = SceneStores::new(&lights, &obstacles);
                let t = time(n, || {
                    draw(gpu, r_today, &frame(0), &stores, &today);
                });
                let moved = time(n, || {
                    step += 1;
                    let x = if step.is_multiple_of(2) { NUDGE } else { 0.0 };
                    for (&id, d) in ids.iter().zip(&descs) {
                        let at = At::new(d.position.raw() + Vec2::new(x, 0.0));
                        moving.set_position(id, at, d.height).unwrap();
                    }
                    let stores = SceneStores::new(&moving, &obstacles);
                    let record = draw(gpu, r_moved, &frame(2), &stores, &rows);
                    assert_eq!(record.lighting.rows_built, all);
                });
                let floor = time(n, || {
                    let stores = SceneStores::new(&none, &obstacles);
                    draw(gpu, r_none, &frame(2), &stores, &rows);
                });
                // The first frame builds the rows, from then on the lamps rest.
                let first = draw(gpu, r_rest, &frame(1), &stores, &rows).lighting;
                assert_eq!(first.row_lights, all, "every lamp is a row light");
                assert_eq!(first.rows_built, if run == 0 { all } else { 0 });
                let rest = time(n, || {
                    let record = draw(gpu, r_rest, &frame(1), &stores, &rows);
                    assert_eq!(record.lighting.rows_built, 0);
                });
                if run > 0 {
                    for (list, value) in timed.iter_mut().zip([t, rest, moved, floor]) {
                        list.push(value);
                    }
                    eprintln!(
                        "{name} run {run}: today {:.1} / {:.1}, rows resting {:.1} / {:.1}, rows all built {:.1} / {:.1}, no lights {:.1} / {:.1} us (median / p95)",
                        t.0, t.1, rest.0, rest.1, moved.0, moved.1, floor.0, floor.1
                    );
                }
            }
            (truth.frame(gpu, &frame(2), &lights, &obstacles, &today, &renderers[0])).unwrap();
            let pictures = ["heute", "neu", "wahr"]
                .into_iter()
                .zip(targets.iter().map(|t| read_texture(gpu, t.texture_ref())))
                .collect::<Vec<_>>();
            let mut diffs = Vec::new();
            let mut numbers = Vec::new();
            for (i, j) in [(0, 2), (1, 2), (0, 1)] {
                let ((first, a), (second, b)) = (&pictures[i], &pictures[j]);
                let (mean, over, diff) = compare(a, b);
                let _ = writeln!(
                    report,
                    "{name:12} {all} lamps, {first} against {second}: mean absolute difference {mean:.3} of 255, {over:.2} % of the pixels differ by more than 8"
                );
                diffs.push((format!("diff_{first}_{second}_x8"), diff));
                numbers.push((mean, over));
            }
            if frames > 0 {
                let cases = ["today", "rows, lamps rest", "rows, all built", "no lights"];
                for (case, list) in cases.into_iter().zip(&timed) {
                    let _ = writeln!(report, "{}", summary(name, case, list));
                }
            }
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let named = pictures.iter().map(|(n, px)| (*n, px));
                for (suffix, px) in named.chain(diffs.iter().map(|(n, px)| (n.as_str(), px))) {
                    let file = dir.join(format!("{}_{suffix}.png", name.replace(' ', "_")));
                    save_png_rgba8(&file, px, OUT.0, OUT.1);
                }
            }
            eprint!("{report}");
            let (old, rows) = (numbers[0], numbers[1]);
            assert!(
                rows.0 < old.0 && rows.1 < old.1,
                "{name}: the rows are not closer to the reference than today's light"
            );
            assert!(
                rows.0 <= bound.1 && rows.1 <= bound.2,
                "{name}: rows against the reference {rows:?}, bound {bound:?}"
            );
            full.push_str(&report);
            report.clear();
        }
        if let Some(dir) = &out {
            std::fs::write(dir.join("messung.txt"), &full).unwrap();
        }
    });
}
