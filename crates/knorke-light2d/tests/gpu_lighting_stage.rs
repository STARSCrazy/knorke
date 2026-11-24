//! P25 on a real GPU: the light pass (`knorke_light2d::lighting::LightingStage`) after the
//! culling (P8) and the occlusion stages (P22, P23), read back and compared byte for byte with
//! C++ Chiaro (chiaro @ 9bc5b9b, dumps of `tests/golden/chiaro/light/light_ref.cpp`, zlib
//! license), and the light behaviour of Chiaro's `tests/gpu/LightingTests.cpp` checked on the
//! light buffer.
//!
//! The test plays the host (ADR 0002): it owns device, command buffers, the obstacle and
//! occlusion stages, submits and reads back. It skips (passes with a note) without a Vulkan
//! device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use knorke_gpu::sys::gpu::{
    SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER,
    SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    CommandBuffer, Device, DownloadBuffer, Filter, Gpu, LiveCounter, PipelineCache, ProgramDesc,
    Samplers, ShaderLibrary, Texture, TextureDesc, TextureRef, UploadBuffer, init_video,
};
use knorke_light2d::codec::{AlphaSource, Material, NormalMapOptions, convert_normal_map};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::light_math::attenuation;
use knorke_light2d::light_pipelines::LightPipelines;
use knorke_light2d::light_program::{LIGHT_PROGRAMS, LightBase, LightProgram};
use knorke_light2d::lighting::{
    DetailMode, LightingFrame, LightingRecord, LightingResult, LightingStage, ShadowMaps,
};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::occluder_raster::{
    INFINITE_TOP, NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms, TileOccluders,
    thin_circle_radius,
};
use knorke_light2d::occlusion::{MASK_PROGRAMS, MaskPrograms, OcclusionMask};
use knorke_light2d::settings::{LightSettings, Quality, Resolutions};
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdate};
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3, World2};

static SERIAL: Mutex<()> = Mutex::new(());

const W: u32 = 96;
const H: u32 = 48;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn at(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn device() -> Option<(MutexGuard<'static, ()>, Device, Vec<u8>)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let pack = workspace().join("target/shaders/knorke-chiaro.spirv.kpak");
    let Ok(pack) = std::fs::read(&pack) else {
        eprintln!(
            "skipped: no shader pack at {} (cargo xtask shaders)",
            pack.display()
        );
        return None;
    };
    init_video(Some(c"vulkan"));
    match Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false) {
        Ok(d) => Some((guard, d, pack)),
        Err(e) => {
            eprintln!("skipped: no Vulkan device ({})", e.message);
            None
        }
    }
}

fn half(h: u16) -> f32 {
    let (e, m) = (i32::from((h >> 10) & 0x1F), f32::from(h & 0x3FF));
    let v = match e {
        0 => m * f32::from_bits((127 - 24) << 23),
        _ => (1.0 + m / 1024.0) * f32::from_bits(((e - 15 + 127) as u32) << 23),
    };
    if h & 0x8000 != 0 { -v } else { v }
}

/// A light of the dumper's `Light` struct (same defaults).
#[derive(Clone, Copy)]
struct L {
    kind: LightType,
    x: f32,
    y: f32,
    height: Height,
    radius: f32,
    color: Vec3,
    intensity: f32,
    source: f32,
    falloff: f32,
    shadows: bool,
    direction: Vec3,
    inner: f32,
    outer: f32,
    axis: Vec3,
    half_width: f32,
}

const L0: L = L {
    kind: LightType::Point,
    x: 0.0,
    y: 0.0,
    height: Height::Auto,
    radius: 30.0,
    color: Vec3::new(1.0, 1.0, 1.0),
    intensity: 1.0,
    source: 0.0,
    falloff: 2.0,
    shadows: false,
    direction: Vec3::new(1.0, 0.0, 0.0),
    inner: 20.0,
    outer: 30.0,
    axis: Vec3::new(1.0, 0.0, 0.0),
    half_width: 0.5,
};

impl L {
    fn desc(&self) -> LightDesc {
        let mut d = LightDesc::new(self.kind);
        d.position = at(self.x, self.y);
        d.height = self.height;
        d.radius = self.radius;
        d.color = self.color;
        d.intensity = self.intensity;
        d.source_radius = self.source;
        d.falloff = self.falloff;
        d.cast_shadows = self.shadows;
        d.direction = self.direction;
        d.inner_angle = self.inner;
        d.outer_angle = self.outer;
        d.axis = self.axis;
        d.half_width = self.half_width;
        d
    }
}

/// One scene: lights, obstacles, settings and whether a material buffer is given.
struct Scene {
    name: &'static str,
    lights: LightStore,
    obstacles: ObstacleStore,
    settings: LightSettings,
    material: bool,
    projection: Projection,
}

impl Scene {
    fn new(name: &'static str, ambient: f32, light_resolution: f32) -> Self {
        let mut settings = LightSettings {
            ambient: Vec3::new(ambient, ambient, ambient),
            ..LightSettings::default()
        };
        if light_resolution < 1.0 {
            settings.quality.quality = Quality::Custom(Resolutions {
                light: light_resolution,
                shadow: 1.0,
                medium: 0.5,
            });
        }
        Self {
            name,
            lights: LightStore::new(),
            obstacles: ObstacleStore::new(),
            settings,
            material: false,
            projection: Projection::IDENTITY,
        }
    }

    fn light(&mut self, l: L) -> &mut Self {
        self.lights.create(&l.desc(), &NoResources).unwrap();
        self
    }

    fn polygon(&mut self, xy: &[f32], height: f32, opacity: f32) -> &mut Self {
        let v: Vec<Vec2> = xy.chunks(2).map(|c| Vec2::new(c[0], c[1])).collect();
        let common = ObstacleCommon {
            height,
            opacity,
            ..Default::default()
        };
        self.obstacles
            .create_shape(&ShapeDesc {
                common,
                shape: Shape::Polygon(&v),
            })
            .unwrap();
        self
    }

    fn circle(&mut self, radius: f32, x: f32, y: f32, height: f32) -> &mut Self {
        let common = ObstacleCommon {
            position: at(x, y),
            height,
            ..Default::default()
        };
        self.obstacles
            .create_shape(&ShapeDesc {
                common,
                shape: Shape::Circle { radius },
            })
            .unwrap();
        self
    }
}

/// `SceneFlat` of `light_ref.cpp`.
fn scene_flat() -> Scene {
    let mut s = Scene::new("flat", 0.0, 1.0);
    s.settings.ambient = Vec3::new(0.05, 0.06, 0.07);
    s.light(L {
        x: 20.0,
        y: 24.0,
        radius: 30.0,
        color: Vec3::new(1.0, 0.8, 0.6),
        intensity: 1.2,
        ..L0
    })
    .light(L {
        x: 48.0,
        y: 10.0,
        height: Height::At(8.0),
        radius: 20.0,
        falloff: 1.5,
        ..L0
    })
    .light(L {
        kind: LightType::Spot,
        x: 70.0,
        y: 30.0,
        radius: 40.0,
        direction: Vec3::new(-1.0, 0.3, -0.2),
        inner: 15.0,
        outer: 35.0,
        ..L0
    })
    .light(L {
        kind: LightType::Line,
        x: 40.0,
        y: 40.0,
        radius: 18.0,
        color: Vec3::new(0.4, 0.7, 1.0),
        axis: Vec3::new(12.0, 0.0, 0.0),
        ..L0
    })
    .light(L {
        kind: LightType::Directional,
        intensity: 0.1,
        direction: Vec3::new(0.3, 0.4, -1.0),
        ..L0
    });
    s
}

fn obstacles(s: &mut Scene, box_h: f32, tri_h: f32, tri_opacity: f32) {
    s.polygon(
        &[40.0, 14.0, 48.0, 14.0, 48.0, 30.0, 40.0, 30.0],
        box_h,
        1.0,
    )
    .polygon(&[60.0, 8.0, 72.0, 12.0, 64.0, 22.0], tri_h, tri_opacity)
    .circle(4.0, 26.0, 34.0, 0.0);
}

/// `SceneShadow` of `light_ref.cpp`.
fn scene_shadow() -> Scene {
    let mut s = Scene::new("shadow", 0.03, 1.0);
    obstacles(&mut s, 0.0, 0.0, 0.6);
    s.light(L {
        x: 18.0,
        y: 20.0,
        radius: 50.0,
        shadows: true,
        ..L0
    })
    .light(L {
        x: 80.0,
        y: 36.0,
        radius: 45.0,
        color: Vec3::new(1.0, 0.7, 0.5),
        source: 3.0,
        shadows: true,
        ..L0
    })
    .light(L {
        kind: LightType::Spot,
        x: 50.0,
        y: 44.0,
        radius: 40.0,
        source: 1.0,
        shadows: true,
        direction: Vec3::new(0.0, -1.0, 0.0),
        inner: 20.0,
        outer: 40.0,
        ..L0
    })
    .light(L {
        kind: LightType::Rect,
        x: 30.0,
        y: 6.0,
        radius: 30.0,
        color: Vec3::new(0.6, 0.8, 1.0),
        shadows: true,
        direction: Vec3::new(0.0, 1.0, 0.0),
        axis: Vec3::new(6.0, 0.0, 0.0),
        half_width: 2.0,
        ..L0
    })
    .light(L {
        x: 88.0,
        y: 6.0,
        radius: 15.0,
        ..L0
    });
    s
}

/// `SceneHeight` of `light_ref.cpp`.
fn scene_height() -> Scene {
    let mut s = Scene::new("height", 0.0, 1.0);
    s.settings.ambient = Vec3::new(0.04, 0.04, 0.05);
    obstacles(&mut s, 6.0, 3.0, 1.0);
    s.light(L {
        x: 18.0,
        y: 20.0,
        height: Height::At(10.0),
        radius: 50.0,
        source: 1.0,
        shadows: true,
        ..L0
    })
    .light(L {
        kind: LightType::Line,
        x: 70.0,
        y: 40.0,
        height: Height::At(4.0),
        radius: 30.0,
        shadows: true,
        axis: Vec3::new(0.0, 6.0, 0.0),
        ..L0
    })
    .light(L {
        kind: LightType::Directional,
        intensity: 0.6,
        source: 1.0,
        shadows: true,
        direction: Vec3::new(0.6, 0.3, -0.8),
        ..L0
    });
    s
}

/// `SceneMaterial` of `light_ref.cpp`.
fn scene_material(name: &'static str, light_resolution: f32) -> Scene {
    let mut s = Scene::new(name, 0.05, light_resolution);
    s.material = true;
    s.polygon(&[70.0, 30.0, 78.0, 30.0, 78.0, 40.0, 70.0, 40.0], 6.0, 1.0)
        .circle(3.0, 20.0, 10.0, 4.0);
    s.light(L {
        x: 20.0,
        y: 24.0,
        radius: 40.0,
        color: Vec3::new(1.0, 0.9, 0.8),
        ..L0
    })
    .light(L {
        x: 60.0,
        y: 10.0,
        radius: 50.0,
        source: 2.0,
        shadows: true,
        ..L0
    })
    .light(L {
        kind: LightType::Spot,
        x: 90.0,
        y: 44.0,
        height: Height::At(6.0),
        radius: 50.0,
        color: Vec3::new(0.5, 0.7, 1.0),
        shadows: true,
        direction: Vec3::new(-1.0, -0.5, -0.3),
        inner: 20.0,
        outer: 35.0,
        ..L0
    })
    .light(L {
        kind: LightType::Directional,
        intensity: 0.4,
        source: 1.0,
        shadows: true,
        direction: Vec3::new(-0.5, 0.4, -0.7),
        ..L0
    });
    s
}

/// The dome normal map of `Material()` in `light_ref.cpp`, as material pixels (satin).
fn material_pixels(w: u32, h: u32) -> Vec<u8> {
    let mut normal = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - 48.0) / 20.0;
            let dy = (y as f32 + 0.5 - 24.0) / 20.0;
            let d2 = dx * dx + dy * dy;
            let n = if d2 < 1.0 {
                [dx, -dy, (1.0 - d2).sqrt()]
            } else {
                [0.0, 0.0, 1.0]
            };
            let p = ((y * w + x) * 4) as usize;
            for i in 0..3 {
                normal[p + i] = ((n[i] * 0.5 + 0.5) * 255.0).round() as u8;
            }
            normal[p + 3] = 255;
        }
    }
    let color = vec![255u8; normal.len()];
    let options = NormalMapOptions {
        material: Material::SATIN,
        alpha: Some(AlphaSource {
            rgba: &color,
            pitch: (w * 4) as usize,
        }),
        ..Default::default()
    };
    let mut out = vec![0u8; normal.len()];
    convert_normal_map(&normal, w, h, (w * 4) as usize, &options, &mut out).unwrap();
    out
}

fn texture<'d>(gpu: Gpu<'d>, w: u32, h: u32, pixels: &[u8]) -> Texture<'d> {
    let desc = TextureDesc::new(
        w,
        h,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREUSAGE_SAMPLER,
    );
    let texture = Texture::new(gpu, desc).unwrap();
    let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), pixels.len() as u32, "px");
    up.reserve(pixels.len() as u64).unwrap();
    up.map().unwrap().bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .upload_to_texture(&up, texture.texture_ref(), false)
        .unwrap();
    cmd.submit_and_wait().unwrap();
    texture
}

/// What one frame left behind.
struct Out {
    result: LightingResult,
    record: LightingRecord,
    light: Vec<u8>,
    light_size: (u32, u32),
    second: Vec<u8>,
    programs: Vec<LightProgram>,
}

impl Out {
    /// Red channel of light buffer pixel (x, y).
    fn red(&self, x: u32, y: u32) -> f32 {
        let i = ((y * self.light_size.0 + x) * 8) as usize;
        half(u16::from_le_bytes([self.light[i], self.light[i + 1]]))
    }
}

struct Host<'d> {
    gpu: Gpu<'d>,
    device: &'d Device,
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    light_pipelines: LightPipelines<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    occlusion: OcclusionMask<'d>,
    lighting: LightingStage<'d>,
    list: LightList,
}

fn programs() -> Vec<ProgramDesc> {
    OCCLUDER_PROGRAMS
        .iter()
        .chain(&MASK_PROGRAMS)
        .copied()
        .collect()
}

/// Chiaro `AnyShadowCaster` (OcclusionStage.cpp).
fn any_shadow_caster(l: &LightList) -> bool {
    !(l.shadowed.is_empty()
        && l.shadowed_area.is_empty()
        && l.masked_shadowed.is_empty()
        && l.directional_shadows == 0
        && l.caster_classes == 0)
}

impl<'d> Host<'d> {
    fn new(gpu: Gpu<'d>, device: &'d Device, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &programs()).unwrap();
        let light_pipelines = LightPipelines::new(&shaders).unwrap();
        Self {
            gpu,
            device,
            shaders,
            pipelines,
            light_pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            occlusion: OcclusionMask::new(gpu, MaskPrograms::at(OCCLUDER_PROGRAMS.len())),
            lighting: LightingStage::new(gpu),
            list: LightList::default(),
        }
    }

    fn download(&self, cmd: &mut CommandBuffer<'_>, t: TextureRef<'_>) -> DownloadBuffer<'d> {
        let down = DownloadBuffer::new(self.gpu, t.desc().bytes() as u32).unwrap();
        cmd.copy_pass().unwrap().download_texture(t, &down).unwrap();
        down
    }

    /// One frame as Chiaro's `Context::Render` runs it up to the light pass, `time` seconds.
    fn frame(&mut self, s: &Scene, time: f64) -> Out {
        let scene_tex = texture(self.gpu, W, H, &vec![255u8; (W * H * 4) as usize]);
        let material_tex = s
            .material
            .then(|| texture(self.gpu, W, H, &material_pixels(W, H)));
        let (w, h) = (W, H);
        let normal_mode = s.material;
        let shadow_scale = s.settings.resolutions().shadow;
        let modes = LightModes {
            height_mode: s.obstacles.any_finite_height(),
            normal_mode,
            max_height: 0.0, // no surface buffer
            ..Default::default()
        };
        collect_lights(
            &s.lights,
            &s.settings,
            &s.projection,
            time,
            w,
            h,
            &modes,
            &mut self.list,
        );
        let active = s.obstacles.iter().any(|(_, o)| o.is_active());
        let shadows = any_shadow_caster(&self.list) && active;
        let height_mode = s.obstacles.any_finite_height() || normal_mode;
        let mut cmd = CommandBuffer::acquire(self.device).unwrap();
        let input = MaskPlanInput::new(&s.obstacles, &s.projection, w, h, shadow_scale);
        if shadows {
            let update = self
                .occlusion
                .prepare(&mut self.shaders, &mut self.pipelines, &input, height_mode)
                .unwrap();
            if update != MaskUpdate::Reused {
                let min_radius = thin_circle_radius(&s.projection, &input.domain);
                self.geometry
                    .prepare(
                        &mut self.shaders,
                        &mut self.pipelines,
                        &s.obstacles,
                        &NoPatterns,
                        height_mode,
                        min_radius,
                    )
                    .unwrap();
                self.tiles
                    .prepare(
                        &mut self.shaders,
                        &mut self.pipelines,
                        &s.obstacles,
                        &NoPatterns,
                        height_mode,
                    )
                    .unwrap();
            }
            self.occlusion
                .record(
                    &mut cmd,
                    &self.pipelines,
                    self.samplers.sampler(Filter::Nearest),
                    &mut self.geometry,
                    &mut self.tiles,
                )
                .unwrap();
            self.occlusion.commit();
        }
        let scene_max_top = if height_mode {
            s.obstacles.max_finite_top()
        } else {
            0.0
        };
        let maps = shadows.then(|| ShadowMaps {
            domain: input.domain,
            remainder: input.anchor.remainder,
            mask: self.occlusion.mask().unwrap(),
            field: self.occlusion.field().unwrap(),
            height: self.occlusion.height(),
            height_mode,
            any_partial: s.obstacles.any_partial(),
            shadow_top: if s.obstacles.any_infinite_enabled() {
                INFINITE_TOP
            } else {
                scene_max_top
            },
            scene_max_top,
            casters: [None, None],
        });
        let frame = LightingFrame {
            scene: scene_tex.texture_ref(),
            surface: None,
            material: material_tex.as_ref().map(Texture::texture_ref),
            projection: &s.projection,
            settings: &s.settings,
            max_height: 0.0,
            lights: &self.list,
            shadows: maps.as_ref(),
            profile_atlas: None,
            mask_atlas: None,
        };
        let result = self
            .lighting
            .prepare(&mut self.shaders, &mut self.light_pipelines, &frame)
            .unwrap();
        let record = self
            .lighting
            .record(&mut cmd, &self.light_pipelines, &self.samplers, &frame)
            .unwrap();
        let mut programs: Vec<LightProgram> = LIGHT_PROGRAMS
            .iter()
            .map(|i| i.program())
            .filter(|&p| self.light_pipelines.contains(p))
            .collect();
        programs.sort();
        let light_tex = self.lighting.light_buffer();
        let light_size = light_tex.map_or((0, 0), |t| (t.width(), t.height()));
        let light = light_tex.map(|t| self.download(&mut cmd, t));
        let second = match result.detail_mode {
            DetailMode::Exact => self.lighting.specular_buffer(),
            DetailMode::Direction => self.lighting.direction_buffer(),
            DetailMode::None => None,
        }
        .map(|t| self.download(&mut cmd, t));
        cmd.submit_and_wait().unwrap();
        let read = |mut d: DownloadBuffer<'_>| {
            let mut v = vec![0; d.size() as usize];
            d.read(&mut v).unwrap();
            v
        };
        Out {
            result,
            record,
            light: light.map(read).unwrap_or_default(),
            light_size,
            second: second.map(read).unwrap_or_default(),
            programs,
        }
    }
}

fn with_host(body: impl for<'d> FnOnce(&mut dyn FnMut() -> Host<'d>)) {
    let Some((_guard, device, pack)) = device() else {
        return;
    };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut fresh = || Host::new(gpu, &device, &pack);
        body(&mut fresh);
    }
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}

fn golden(name: &str) -> Vec<u8> {
    let path = workspace().join("tests/golden/chiaro/light").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Differing RGBA16F texels and the largest difference.
fn diff(a: &[u8], b: &[u8]) -> (usize, f32) {
    assert_eq!(a.len(), b.len());
    let mut n = 0;
    let mut max = 0.0f32;
    for (x, y) in a.chunks(8).zip(b.chunks(8)) {
        if x != y {
            n += 1;
            for c in 0..4 {
                let u = half(u16::from_le_bytes([x[c * 2], x[c * 2 + 1]]));
                let v = half(u16::from_le_bytes([y[c * 2], y[c * 2 + 1]]));
                let d = (u - v).abs();
                if d > max {
                    max = d;
                }
            }
        }
    }
    (n, max)
}

/// The programs a scene used according to `programs.txt` (`base:features`).
fn golden_programs(name: &str) -> Vec<LightProgram> {
    let text = String::from_utf8(golden("programs.txt")).unwrap();
    let line = text
        .lines()
        .find(|l| l.split(' ').next() == Some(name))
        .unwrap();
    let mut v: Vec<LightProgram> = line
        .split(' ')
        .skip(1)
        .map(|p| {
            let (b, f) = p.split_once(':').unwrap();
            let base = LightBase::ALL[b.parse::<usize>().unwrap()];
            LIGHT_PROGRAMS
                .iter()
                .map(|i| i.program())
                .find(|q| {
                    q.base == base && u32::from(q.features.bits()) == f.parse::<u32>().unwrap()
                })
                .unwrap()
        })
        .collect();
    v.sort();
    v
}

#[test]
fn light_buffers_match_chiaro() {
    with_host(|fresh| {
        let mut failures = Vec::new();
        for scene in [
            scene_flat(),
            scene_shadow(),
            scene_height(),
            scene_material("exact", 1.0),
            scene_material("direction", 0.5),
        ] {
            let mut host = fresh();
            let out = host.frame(&scene, 0.0);
            let name = scene.name;
            assert_eq!(out.programs, golden_programs(name), "{name}: programs");
            let (n, max) = diff(&out.light, &golden(&format!("{name}.light.bin")));
            eprintln!("{name}: light buffer {} differing texels (max {max})", n);
            if n > 0 {
                failures.push(format!("{name}.light: {n} texels, max {max}"));
            }
            let second = match out.result.detail_mode {
                DetailMode::Exact => Some("specular"),
                DetailMode::Direction => Some("direction"),
                DetailMode::None => None,
            };
            if let Some(kind) = second {
                let (n, max) = diff(&out.second, &golden(&format!("{name}.{kind}.bin")));
                eprintln!("{name}: {kind} buffer {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.{kind}: {n} texels, max {max}"));
                }
            }
        }
        assert!(failures.is_empty(), "not bit identical: {failures:#?}");
    });
}

/// The same frame twice gives the same bytes, and the second prepare allocates nothing.
#[test]
fn frames_repeat_bit_identically() {
    with_host(|fresh| {
        let mut host = fresh();
        let scene = scene_material("exact", 1.0);
        let a = host.frame(&scene, 0.0);
        let b = host.frame(&scene, 0.0);
        assert!(a.result.allocations > 0);
        assert_eq!(b.result.allocations, 0);
        assert_eq!(a.light, b.light);
        assert_eq!(a.second, b.second);
        assert_eq!(a.record, b.record);
        assert_eq!(a.record.lights_drawn, 3);
        assert_eq!(a.record.shadow_lights, 2);
        // unshadowed + shadowed draw + one directional
        assert_eq!(a.record.draws, 3);
    });
}

// ---- Chiaro tests/gpu/LightingTests.cpp, checked on the light buffer -----------------------

/// Light at (lx, ly, lz) seen from the centre of pixel (x, y), identity projection.
fn att_at_pixel(x: u32, y: u32, lx: f32, ly: f32, lz: f32, r: f32) -> f32 {
    let dx = x as f32 + 0.5 - lx;
    let dy = y as f32 + 0.5 - ly;
    attenuation((dx * dx + dy * dy + lz * lz).sqrt(), r, 2.0)
}

fn dark(name: &'static str) -> Scene {
    Scene::new(name, 0.0, 1.0)
}

fn near(got: f32, want: f32) -> bool {
    (got - want).abs() <= 2e-3 + want.abs() * 2e-3
}

#[test]
fn ambient_alone_needs_no_light_pass() {
    with_host(|fresh| {
        let mut host = fresh();
        let s = Scene::new("ambient", 0.25, 1.0);
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.record, LightingRecord::default());
        assert_eq!(out.result.ambient, Vec3::new(0.25, 0.25, 0.25));
    });
}

#[test]
fn point_light_falls_off_smoothly_and_reaches_zero_at_its_radius() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("point");
        s.light(L {
            x: 32.0,
            y: 32.0,
            height: Height::At(0.0),
            radius: 32.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        for (x, y) in [(32, 32), (48, 32), (32, 40)] {
            assert!(near(
                out.red(x, y),
                att_at_pixel(x, y, 32.0, 32.0, 0.0, 32.0)
            ));
        }
        assert_eq!(out.red(0, 0), 0.0);
        assert_eq!(out.record.lights_drawn, 1);
    });
}

#[test]
fn a_raised_light_is_dimmer_directly_below() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("raised");
        s.light(L {
            x: 32.0,
            y: 32.0,
            height: Height::At(16.0),
            radius: 32.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        let want = att_at_pixel(32, 32, 32.0, 32.0, 16.0, 32.0);
        assert!(want < 0.8);
        assert!(near(out.red(32, 32), want));
    });
}

#[test]
fn spot_light_only_lights_its_cone() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("spot");
        s.light(L {
            kind: LightType::Spot,
            x: 32.0,
            y: 24.0,
            radius: 32.0,
            direction: Vec3::new(1.0, 0.0, 0.0),
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(near(
            out.red(48, 24),
            att_at_pixel(48, 24, 32.0, 24.0, 0.0, 32.0)
        ));
        assert_eq!(out.red(16, 24), 0.0); // behind
        assert_eq!(out.red(32, 40), 0.0); // 90 degrees off
    });
}

#[test]
fn directional_light_without_shadows_stays_in_the_ambient() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("directional");
        s.light(L {
            kind: LightType::Directional,
            intensity: 0.5,
            direction: Vec3::new(0.0, 0.0, -1.0),
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.result.ambient, Vec3::new(0.5, 0.5, 0.5));
    });
}

#[test]
fn lights_live_in_world_space_zoom_scales_their_footprint() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("zoom");
        s.projection = Projection::ortho(at(32.0, 24.0), 2.0, W, H);
        s.light(L {
            x: 32.0,
            y: 24.0,
            height: Height::At(0.0),
            radius: 16.0, // 32 scene pixels
            ..L0
        });
        let out = host.frame(&s, 0.0);
        // Scene pixel (64, 24) centre -> world (32 + 16.5 / 2, 24 + 0.5 / 2).
        let (wx, wy) = (32.0 + 16.5 / 2.0, 24.0 + 0.5 / 2.0);
        let d = ((wx - 32.0f32) * (wx - 32.0) + (wy - 24.0f32) * (wy - 24.0)).sqrt();
        assert!(near(out.red(64, 24), attenuation(d, 16.0, 2.0)));
        assert_eq!(out.red(0, 0), 0.0);
    });
}

#[test]
fn light_buffer_and_scene_share_the_vertical_orientation() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("orientation");
        s.light(L {
            x: 32.0,
            y: 12.0,
            height: Height::At(0.0),
            radius: 20.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(near(
            out.red(32, 4),
            att_at_pixel(32, 4, 32.0, 12.0, 0.0, 20.0)
        ));
        assert!(near(
            out.red(32, 28),
            att_at_pixel(32, 28, 32.0, 12.0, 0.0, 20.0)
        ));
        assert_eq!(out.red(32, 40), 0.0);
    });
}

#[test]
fn many_lights_grow_the_buffers_once_and_then_stay_stable() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("many");
        for i in 0..1500u32 {
            s.light(L {
                x: (i % 96) as f32,
                y: ((i * 7) % 48) as f32,
                radius: 2.0,
                intensity: 0.01,
                ..L0
            });
        }
        let a = host.frame(&s, 0.0);
        assert_eq!(a.record.lights_drawn, 1500);
        let b = host.frame(&s, 0.0);
        assert_eq!(b.result.allocations, 0);
        assert_eq!(a.light, b.light);
    });
}

#[test]
fn light_buffer_follows_the_resolution_scale() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("scale", 0.0, 0.5);
        s.light(L {
            x: 32.0,
            y: 24.0,
            radius: 32.0,
            ..L0
        });
        let a = host.frame(&s, 0.0);
        assert_eq!(a.light_size, (48, 24));
        assert_eq!(a.result.allocations, 1);
        assert!(a.red(16, 12) > 0.95);
        let b = host.frame(&s, 0.0);
        assert_eq!(b.result.allocations, 0);
    });
}

#[test]
fn disabled_lights_are_not_drawn() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("disabled");
        let mut d = L0.desc();
        d.enabled = false;
        s.lights.create(&d, &NoResources).unwrap();
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.record.lights_drawn, 0);
    });
}

#[test]
fn record_without_prepare_fails() {
    with_host(|fresh| {
        let host = fresh();
        let scene = texture(host.gpu, 4, 4, &[255u8; 64]);
        let list = LightList::default();
        let settings = LightSettings::default();
        let frame = LightingFrame {
            scene: scene.texture_ref(),
            surface: None,
            material: None,
            projection: &Projection::IDENTITY,
            settings: &settings,
            max_height: 0.0,
            lights: &list,
            shadows: None,
            profile_atlas: None,
            mask_atlas: None,
        };
        let mut cmd = CommandBuffer::acquire(host.device).unwrap();
        let e = host
            .lighting
            .record(&mut cmd, &host.light_pipelines, &host.samplers, &frame)
            .unwrap_err();
        assert_eq!(e.code, knorke_light2d::lighting::LIGHTING_NOT_PREPARED);
    });
}
