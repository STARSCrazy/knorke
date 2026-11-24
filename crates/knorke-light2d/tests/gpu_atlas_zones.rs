//! P26 on a real GPU: the profile and mask atlases (`knorke_light2d::atlas`) and the ambient zone
//! pass of the light stage (`LightingStage::prepare_with_zones`), read back and compared with
//! the CPU models (Chiaro's `tests/gpu/ProfileTests.cpp`, `MaskTests.cpp`,
//! `AmbientZoneTests.cpp`, checked on the light buffer) and byte for byte with C++ Chiaro
//! (chiaro @ 9bc5b9b, dumps of `tests/golden/chiaro/atlas/atlas_ref.cpp`, zlib license).
//!
//! The test plays the host (ADR 0002): it owns device, command buffers, the obstacle and
//! occlusion stages, the atlases, submits and reads back. It skips (passes with a note) without
//! a Vulkan device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]
// Host test tool: tolerances and comparisons on readbacks are not simulation code, so the
// platform-independent min/max rule does not apply; index loops over channels read clearer.
#![allow(
    clippy::too_many_arguments,
    clippy::disallowed_methods,
    clippy::needless_range_loop,
    clippy::chunks_exact_to_as_chunks
)]

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
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, fold_ambient, make_stored_zone};
use knorke_light2d::ambient_zone_culling::{AmbientZoneList, CullScene, collect_ambient_zones};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::animation::AnimationStore;
use knorke_light2d::atlas::{
    LightAtlases, fill_mask_atlas, fill_profile_atlas, mask_atlas_bytes, mask_atlas_rows,
    profile_atlas_bytes, profile_atlas_rows,
};
use knorke_light2d::light::{Height, LightDesc, LightType, MaskMode, ProfileId};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::light_mask::{make_mask_placement, mask_factor};
use knorke_light2d::light_math::{attenuation, spot_factor};
use knorke_light2d::light_pipelines::LightPipelines;
use knorke_light2d::light_program::{LIGHT_PROGRAMS, LightBase, LightProgram};
use knorke_light2d::lighting::{
    DetailMode, LightingFrame, LightingRecord, LightingResult, LightingStage, PureAmbient,
    ShadowMaps,
};
use knorke_light2d::mask::{MaskPreset, MaskShape, build_pixel_mask};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::occluder_raster::{
    INFINITE_TOP, NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms, TileOccluders,
    thin_circle_radius,
};
use knorke_light2d::occlusion::{MASK_PROGRAMS, MaskPrograms, OcclusionMask};
use knorke_light2d::pass_plan::{
    PassFrame, PassPlan, PassPrepared, PassRecord, PassStages, SceneStores,
};
use knorke_light2d::profile::{
    PROFILE_VALUES, ProfilePreset, ProfileShape, build_shape_profile, profile_factor,
};
use knorke_light2d::profile_import::import_ies;
use knorke_light2d::settings::{LightSettings, Quality, Resolutions};
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdate};
use knorke_light2d::zone_pass::ZoneInput;
use knorke_light2d::{
    LightStore, MaskId, MaskStore, ObstacleStore, Projection, StoreResources,
    profile_store::ProfileStore,
};
use knorke_math::{At, Vec2, Vec3, World2};

static SERIAL: Mutex<()> = Mutex::new(());

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn at(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

/// The device of this machine, or a note why not. The note is a failure when
/// `KNORKE_REQUIRE_GPU=1` is set, so a skip on a machine with a GPU cannot go unnoticed.
fn device() -> Option<(MutexGuard<'static, ()>, Device, Vec<u8>)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let require = std::env::var("KNORKE_REQUIRE_GPU").is_ok_and(|v| v == "1");
    let pack = workspace().join("target/shaders/knorke-chiaro.spirv.kpak");
    let Ok(pack) = std::fs::read(&pack) else {
        assert!(!require, "no shader pack at {}", pack.display());
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
            assert!(!require, "no Vulkan device ({})", e.message);
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
    profile: Option<ProfileId>,
    profile_rotation: f32,
    mask: Option<MaskId>,
    mask_angle: Option<f32>,
    /// Window mode: centre, half extents u and v.
    window: Option<[Vec3; 3]>,
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
    profile: None,
    profile_rotation: 0.0,
    mask: None,
    mask_angle: None,
    window: None,
};

/// Pointing down, as the Chiaro tests aim their profiled and masked lights.
const DOWN: Vec3 = Vec3::new(0.0, 0.0, -1.0);

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
        d.profile = self.profile;
        d.profile_rotation = self.profile_rotation;
        d.mask = self.mask;
        d.mask_angle = self.mask_angle;
        if let Some([c, u, v]) = self.window {
            d.mask_mode = MaskMode::Window;
            d.mask_center = c;
            d.mask_u = u;
            d.mask_v = v;
        }
        d
    }
}

/// One scene: lights, obstacles, stores, settings and which buffers are given.
struct Scene {
    name: &'static str,
    size: (u32, u32),
    lights: LightStore,
    obstacles: ObstacleStore,
    profiles: ProfileStore,
    masks: MaskStore,
    animations: AnimationStore,
    zones: AmbientZoneStore,
    settings: LightSettings,
    material: bool,
    surface: bool,
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
            size: (64, 64),
            lights: LightStore::new(),
            obstacles: ObstacleStore::new(),
            profiles: ProfileStore::new(),
            masks: MaskStore::new(),
            animations: AnimationStore::new(),
            zones: AmbientZoneStore::new(),
            settings,
            material: false,
            surface: false,
            projection: Projection::IDENTITY,
        }
    }

    /// The 96 x 48 scenes of the dumper.
    fn golden(name: &'static str, ambient: Vec3, light_resolution: f32) -> Self {
        let mut s = Self::new(name, 0.0, light_resolution);
        s.settings.ambient = ambient;
        s.size = (96, 48);
        s
    }

    fn light(&mut self, l: L) -> knorke_light2d::light::LightId {
        let res = StoreResources::new(&self.animations, &self.masks).with_profiles(&self.profiles);
        self.lights.create(&l.desc(), &res).unwrap()
    }

    fn lights(&mut self, ls: &[L]) -> &mut Self {
        for l in ls {
            self.light(*l);
        }
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

    /// A profile of a preset with the given quadrant gains replaced.
    fn profile(&mut self, preset: ProfilePreset, gains: [Option<f32>; 4]) -> ProfileId {
        let mut shape = ProfileShape::preset(preset);
        for (g, new) in shape.gain.iter_mut().zip(gains) {
            if let Some(new) = new {
                *g = new;
            }
        }
        self.profiles
            .create(build_shape_profile(&shape).unwrap())
            .unwrap()
    }

    fn mask_shape(&mut self, preset: MaskPreset, glass: Vec3) -> MaskId {
        let mut shape = MaskShape::preset(preset);
        shape.glass = glass;
        shape.validate().unwrap();
        let mut texels = Vec::new();
        shape.build(&mut texels);
        self.masks.create(texels).unwrap()
    }

    /// A gobo from RGBA pixels (sRGB, as `CHI_CreateMask` takes them).
    fn mask_pixels(&mut self, w: u32, h: u32, rgba: &[u8]) -> MaskId {
        let mut texels = Vec::new();
        build_pixel_mask(w, h, rgba, (w * 4) as usize, &mut texels).unwrap();
        self.masks.create(texels).unwrap()
    }

    fn zone(&mut self, p: &AmbientZoneParams) -> knorke_light2d::ambient_zone_store::AmbientZoneId {
        self.zones.create(make_stored_zone(p).unwrap()).unwrap()
    }

    /// Normal mode: the highest height a lit pixel can have (`Context::BuildFrame`).
    fn max_height(&self) -> f32 {
        if !self.surface {
            return 0.0;
        }
        let top = self.obstacles.max_finite_top();
        if top <= 0.0 || self.obstacles.any_infinite_enabled() {
            255.0 * self.settings.surface_height_step
        } else {
            top
        }
    }
}

fn zone_box(x: f32, y: f32, w: f32, d: f32, c: [f32; 3]) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Box,
        x,
        y,
        width: w,
        depth: d,
        ambient: c,
        ..Default::default()
    }
}

fn zone_disc(x: f32, y: f32, r: f32, c: [f32; 3]) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Circle,
        x,
        y,
        radius: r,
        ambient: c,
        ..Default::default()
    }
}

fn zone_polygon(xy: &[f32], c: [f32; 3]) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Polygon,
        vertices: xy.chunks(2).map(|p| Vec2::new(p[0], p[1])).collect(),
        ambient: c,
        ..Default::default()
    }
}

const CELLAR: [f32; 3] = [0.25, 0.5, 1.0];

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

/// FNV-1a 64 of the bytes (the dumper's hash).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// An atlas texture read back: width, height, bytes.
type AtlasDump = (u32, u32, Vec<u8>);

/// What one frame left behind.
struct Out {
    result: LightingResult,
    record: LightingRecord,
    light: Vec<u8>,
    light_size: (u32, u32),
    second: Vec<u8>,
    /// The pure ambient target, if the zone pass wrote one.
    ambient: Vec<u8>,
    programs: Vec<LightProgram>,
    profile_atlas: Option<AtlasDump>,
    mask_atlas: Option<AtlasDump>,
    /// GPU objects alive after the frame.
    live: u32,
}

impl Out {
    /// Channel `c` of light buffer pixel (x, y).
    fn px(&self, x: u32, y: u32, c: usize) -> f32 {
        let i = ((y * self.light_size.0 + x) * 8) as usize + c * 2;
        half(u16::from_le_bytes([self.light[i], self.light[i + 1]]))
    }

    /// Red channel of light buffer pixel (x, y).
    fn red(&self, x: u32, y: u32) -> f32 {
        self.px(x, y, 0)
    }
}

struct Host<'d> {
    gpu: Gpu<'d>,
    device: &'d Device,
    counter: &'d LiveCounter,
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    light_pipelines: LightPipelines<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    occlusion: OcclusionMask<'d>,
    lighting: LightingStage<'d>,
    atlases: LightAtlases<'d>,
    list: LightList,
    zone_list: AmbientZoneList,
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
    fn new(gpu: Gpu<'d>, device: &'d Device, counter: &'d LiveCounter, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &programs()).unwrap();
        let light_pipelines = LightPipelines::new(&shaders).unwrap();
        Self {
            gpu,
            device,
            counter,
            shaders,
            pipelines,
            light_pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            occlusion: OcclusionMask::new(gpu, MaskPrograms::at(OCCLUDER_PROGRAMS.len())),
            lighting: LightingStage::new(gpu),
            atlases: LightAtlases::new(gpu),
            list: LightList::default(),
            zone_list: AmbientZoneList::default(),
        }
    }

    fn download(&self, cmd: &mut CommandBuffer<'_>, t: TextureRef<'_>) -> DownloadBuffer<'d> {
        let down = DownloadBuffer::new(self.gpu, t.desc().bytes() as u32).unwrap();
        cmd.copy_pass().unwrap().download_texture(t, &down).unwrap();
        down
    }

    /// One frame as Chiaro's `Context::Render` runs it up to the light pass, `time` seconds.
    fn frame(&mut self, s: &Scene, time: f64) -> Out {
        let (w, h) = s.size;
        let scene_tex = texture(self.gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        // Empty (all zero) buffers: normal mode on flat ground, as in Chiaro's tests.
        let empty = (s.material || s.surface)
            .then(|| texture(self.gpu, w, h, &vec![0u8; (w * h * 4) as usize]));
        let material_tex = s.material.then(|| empty.as_ref().unwrap());
        let surface_tex = s.surface.then(|| empty.as_ref().unwrap());
        let normal_mode = s.material || s.surface;
        let max_height = s.max_height();
        let shadow_scale = s.settings.resolutions().shadow;
        let modes = LightModes {
            height_mode: s.obstacles.any_finite_height(),
            normal_mode,
            max_height,
            profiles: Some(&s.profiles),
            animations: Some(&s.animations),
            masks: Some(&s.masks),
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
        let ambient = s.settings.ambient;
        let da = self.list.directional_ambient;
        collect_ambient_zones(
            &s.zones,
            [ambient.x, ambient.y, ambient.z],
            &CullScene {
                projection: s.projection,
                max_height,
                has_surface: s.surface,
                width: w,
                height: h,
            },
            [da.x, da.y, da.z],
            &mut self.zone_list,
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
        // The atlases the frame needs, uploaded before the light pass reads them.
        let mut atlas_created = self
            .atlases
            .prepare(&self.list, Some(&s.profiles), Some(&s.masks))
            .unwrap();
        self.atlases.record(&mut cmd).unwrap();
        let frame = LightingFrame {
            scene: scene_tex.texture_ref(),
            surface: surface_tex.map(Texture::texture_ref),
            material: material_tex.map(Texture::texture_ref),
            projection: &s.projection,
            settings: &s.settings,
            max_height,
            lights: &self.list,
            shadows: maps.as_ref(),
            profile_atlas: self.atlases.profiles.texture(),
            mask_atlas: self.atlases.masks.texture(),
        };
        let zones = ZoneInput {
            store: &s.zones,
            list: &self.zone_list,
        };
        let mut result = self
            .lighting
            .prepare_with_zones(
                &mut self.shaders,
                &mut self.light_pipelines,
                &frame,
                Some(zones),
            )
            .unwrap();
        result.allocations += std::mem::take(&mut atlas_created);
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
        let ambient = self
            .lighting
            .pure_ambient()
            .map(|t| self.download(&mut cmd, t));
        let atlas = |t: Option<TextureRef<'_>>, cmd: &mut CommandBuffer<'_>| {
            t.map(|t| (t.width(), t.height(), self.download(cmd, t)))
        };
        let profile_atlas = atlas(self.atlases.profiles.texture(), &mut cmd);
        let mask_atlas = atlas(self.atlases.masks.texture(), &mut cmd);
        cmd.submit_and_wait().unwrap();
        let read = |mut d: DownloadBuffer<'_>| {
            let mut v = vec![0; d.size() as usize];
            d.read(&mut v).unwrap();
            v
        };
        let read_atlas =
            |a: Option<(u32, u32, DownloadBuffer<'_>)>| a.map(|(w, h, d)| (w, h, read(d)));
        Out {
            result,
            record,
            light: light.map(read).unwrap_or_default(),
            light_size,
            second: second.map(read).unwrap_or_default(),
            ambient: ambient.map(read).unwrap_or_default(),
            programs,
            profile_atlas: read_atlas(profile_atlas),
            mask_atlas: read_atlas(mask_atlas),
            live: self.counter.live(),
        }
    }
}

/// A fresh host per call; the counter is the one of all GPU objects of the device.
type Fresh<'a, 'd> = &'a mut dyn FnMut() -> Host<'d>;

fn with_host(body: impl for<'d> FnOnce(Fresh<'_, 'd>)) {
    let Some((_guard, device, pack)) = device() else {
        return;
    };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut fresh = || Host::new(gpu, &device, &counter, &pack);
        body(&mut fresh);
    }
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}

fn golden(name: &str) -> Vec<u8> {
    let path = workspace().join("tests/golden/chiaro/atlas").join(name);
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

/// The `atlases.txt` line of a scene and atlas kind: width, height and the hash of the bytes.
fn golden_atlas(scene: &str, kind: &str) -> Option<(u32, u32, u64)> {
    let text = String::from_utf8(golden("atlases.txt")).unwrap();
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split(' ').collect();
        (f[0] == scene && f[1] == kind).then(|| {
            (
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                u64::from_str_radix(f[4], 16).unwrap(),
            )
        })
    })
}

// ---- the golden scenes of `atlas_ref.cpp` ------------------------------------------------

/// `SceneProfile`: three profiles (blocks 0 to 2), one light shadowed.
fn scene_profile() -> Scene {
    let mut s = Scene::golden("profile", Vec3::new(0.02, 0.02, 0.02), 1.0);
    s.polygon(&[56.0, 30.0, 62.0, 30.0, 62.0, 42.0, 56.0, 42.0], 0.0, 1.0)
        .circle(3.0, 50.0, 14.0, 0.0);
    let hat = s.profile(ProfilePreset::HatLamp, [None, Some(0.5), None, Some(0.0)]);
    let street = s.profile(ProfilePreset::LedStreet, [None; 4]);
    let down = s.profile(ProfilePreset::Downlight, [None; 4]);
    s.lights(&[
        L {
            x: 28.0,
            y: 24.0,
            height: Height::At(8.0),
            radius: 40.0,
            intensity: 2.0,
            direction: DOWN,
            profile: Some(hat),
            ..L0
        },
        L {
            kind: LightType::Spot,
            x: 72.0,
            y: 22.0,
            height: Height::At(8.0),
            radius: 45.0,
            color: Vec3::new(1.0, 0.9, 0.7),
            shadows: true,
            direction: Vec3::new(0.2, 0.3, -1.0),
            inner: 20.0,
            outer: 40.0,
            profile: Some(street),
            profile_rotation: 30.0,
            ..L0
        },
        L {
            x: 14.0,
            y: 38.0,
            height: Height::At(6.0),
            radius: 20.0,
            direction: DOWN,
            profile: Some(down),
            ..L0
        },
        L {
            x: 90.0,
            y: 8.0,
            radius: 12.0,
            ..L0
        },
    ]);
    s
}

/// A 16 x 1 gobo: left half white, right half black.
fn half_gobo() -> Vec<u8> {
    let mut px = vec![255u8; 16 * 4];
    for x in 8..16 {
        px[x * 4..x * 4 + 3].fill(0);
    }
    px
}

/// `SceneMask`: a projector cross, a half gobo, a shadowed window lamp with blue glass.
fn scene_mask() -> Scene {
    let mut s = Scene::golden("mask", Vec3::new(0.02, 0.02, 0.02), 1.0);
    s.polygon(&[40.0, 14.0, 44.0, 14.0, 44.0, 34.0, 40.0, 34.0], 0.0, 1.0);
    let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::new(1.0, 1.0, 1.0));
    let gobo = s.mask_pixels(16, 1, &half_gobo());
    let blue = s.mask_shape(MaskPreset::WindowCross, Vec3::new(0.5, 0.8, 1.0));
    s.lights(&[
        L {
            x: 66.0,
            y: 22.0,
            height: Height::At(8.0),
            radius: 40.0,
            intensity: 2.0,
            direction: DOWN,
            mask: Some(cross),
            mask_angle: Some(60.0),
            ..L0
        },
        L {
            x: 84.0,
            y: 40.0,
            height: Height::At(6.0),
            radius: 14.0,
            color: Vec3::new(1.0, 0.9, 0.8),
            direction: DOWN,
            mask: Some(gobo),
            mask_angle: Some(45.0),
            ..L0
        },
        L {
            x: 14.0,
            y: 24.0,
            height: Height::At(5.0),
            radius: 50.0,
            intensity: 2.0,
            shadows: true,
            direction: DOWN,
            mask: Some(blue),
            mask_angle: Some(60.0),
            window: Some([
                Vec3::new(26.0, 24.0, 3.0),
                Vec3::new(0.0, 4.0, 0.0),
                Vec3::new(0.0, 0.0, 3.0),
            ]),
            ..L0
        },
    ]);
    s
}

/// `SceneZones`: flat mode, five zones, lights on top and a flat directional sum.
fn scene_zones() -> Scene {
    let mut s = Scene::golden("zones", Vec3::new(0.3, 0.35, 0.4), 1.0);
    s.polygon(&[60.0, 8.0, 64.0, 8.0, 64.0, 20.0, 60.0, 20.0], 0.0, 1.0);
    s.lights(&[
        L {
            x: 30.0,
            y: 24.0,
            radius: 30.0,
            intensity: 1.2,
            ..L0
        },
        L {
            x: 70.0,
            y: 14.0,
            radius: 28.0,
            color: Vec3::new(1.0, 0.8, 0.6),
            source: 2.0,
            shadows: true,
            ..L0
        },
        L {
            kind: LightType::Directional,
            intensity: 0.1,
            direction: Vec3::new(0.3, 0.4, -1.0),
            ..L0
        },
    ]);
    s.zone(&AmbientZoneParams {
        soft_edge: 4.0,
        ..zone_box(24.0, 24.0, 28.0, 20.0, CELLAR)
    });
    s.zone(&AmbientZoneParams {
        priority: 1,
        ..zone_disc(66.0, 14.0, 12.0, [0.1, 0.1, 0.1])
    });
    s.zone(&AmbientZoneParams {
        soft_edge: 2.0,
        ..zone_polygon(
            &[
                50.0, 28.0, 90.0, 28.0, 90.0, 44.0, 74.0, 44.0, 74.0, 36.0, 50.0, 36.0,
            ],
            [0.5, 0.2, 0.1],
        )
    });
    s.zone(&AmbientZoneParams {
        relative: true,
        ..zone_box(78.0, 12.0, 24.0, 16.0, [0.5, 0.5, 0.5])
    });
    s.zone(&AmbientZoneParams {
        rotation: 30.0,
        ..zone_box(40.0, 36.0, 20.0, 12.0, [0.2, 0.4, 0.2])
    });
    s
}

/// `SceneZonesOnly`: zones without a light.
fn scene_zones_only() -> Scene {
    let mut s = Scene::golden("zones_only", Vec3::new(0.1, 0.1, 0.1), 1.0);
    s.zone(&AmbientZoneParams {
        soft_edge: 6.0,
        ..zone_box(48.0, 24.0, 40.0, 24.0, CELLAR)
    });
    s.zone(&AmbientZoneParams {
        soft_edge: 3.0,
        ..zone_disc(20.0, 12.0, 10.0, [0.8, 0.6, 0.2])
    });
    s
}

/// `SceneZonesDirection`: a material buffer at half light resolution.
fn scene_zones_direction() -> Scene {
    let mut s = Scene::golden("zones_direction", Vec3::new(0.3, 0.3, 0.3), 0.5);
    s.material = true;
    s.light(L {
        x: 30.0,
        y: 24.0,
        height: Height::At(8.0),
        radius: 40.0,
        ..L0
    });
    s.zone(&AmbientZoneParams {
        soft_edge: 4.0,
        ..zone_box(48.0, 24.0, 60.0, 30.0, [0.1, 0.1, 0.1])
    });
    s.zone(&zone_disc(80.0, 12.0, 10.0, [0.6, 0.2, 0.1]));
    s
}

#[test]
fn scenes_match_chiaro() {
    with_host(|fresh| {
        let mut failures = Vec::new();
        for scene in [
            scene_profile(),
            scene_mask(),
            scene_zones(),
            scene_zones_only(),
            scene_zones_direction(),
        ] {
            let mut host = fresh();
            let out = host.frame(&scene, 0.0);
            let name = scene.name;
            assert_eq!(out.programs, golden_programs(name), "{name}: programs");
            let (n, max) = diff(&out.light, &golden(&format!("{name}.light.bin")));
            eprintln!("{name}: light buffer {n} differing texels (max {max})");
            if n > 0 {
                failures.push(format!("{name}.light: {n} texels, max {max}"));
            }
            if out.result.detail_mode == DetailMode::Direction {
                let (n, max) = diff(&out.second, &golden(&format!("{name}.direction.bin")));
                eprintln!("{name}: direction buffer {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.direction: {n} texels, max {max}"));
                }
            }
            if out.result.pure_ambient == PureAmbient::Separate {
                let (n, max) = diff(&out.ambient, &golden(&format!("{name}.ambient.bin")));
                eprintln!("{name}: pure ambient {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.ambient: {n} texels, max {max}"));
                }
            } else {
                assert!(out.ambient.is_empty(), "{name}: unexpected pure ambient");
            }
            for (kind, got) in [("profile", &out.profile_atlas), ("mask", &out.mask_atlas)] {
                match (golden_atlas(name, kind), got) {
                    (None, None) => {}
                    (Some((w, h, hash)), Some((gw, gh, bytes))) => {
                        let ours = fnv1a(bytes);
                        eprintln!(
                            "{name}: {kind} atlas {gw}x{gh}, hash {ours:016x} (Chiaro {hash:016x})"
                        );
                        if kind == "profile" {
                            // The tables differ from Chiaro's by an ulp here and there (the
                            // deterministic `det` maths, see the tables test); so the upload is
                            // checked against the CPU fill of the same store, exactly.
                            let rows = profile_atlas_rows(scene.profiles.block_count());
                            let mut want = vec![0u8; profile_atlas_bytes(rows) as usize];
                            fill_profile_atlas(&scene.profiles, rows, &mut want);
                            if (w, h) != (*gw, *gh) || want != *bytes {
                                failures.push(format!("{name}.{kind} atlas differs"));
                            }
                        } else if (w, h, hash) != (*gw, *gh, ours) {
                            failures.push(format!("{name}.{kind} atlas differs"));
                        }
                    }
                    (want, got) => failures.push(format!(
                        "{name}.{kind} atlas: Chiaro {want:?}, here {:?}",
                        got.as_ref().map(|g| (g.0, g.1))
                    )),
                }
            }
        }
        assert!(failures.is_empty(), "not bit identical: {failures:#?}");
    });
}

// ---- CPU models ----------------------------------------------------------------------------

fn arr(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// The light of one light at the ground point under pixel (x, y), per channel (Chiaro's
/// `Expected` of `ProfileTests.cpp` and `MaskTests.cpp`).
fn expected(s: &Scene, id: knorke_light2d::light::LightId, x: u32, y: u32) -> [f32; 3] {
    let st = s.lights.stored(id).unwrap();
    let d = &st.desc;
    let pos = d.position.raw();
    let Height::At(height) = d.height else {
        panic!("the model needs a height")
    };
    let to = Vec3::new(x as f32 + 0.5 - pos.x, y as f32 + 0.5 - pos.y, -height);
    let dist = to.length();
    let mut value = d.intensity * attenuation(dist, d.radius, d.falloff);
    if let Some(p) = d.profile {
        value *= profile_factor(
            s.profiles.find(p).unwrap(),
            arr(st.direction),
            arr(st.reference),
            arr(to),
        );
    }
    if d.kind == LightType::Spot {
        value *= spot_factor(to, st.direction, st.cos_inner, st.cos_outer);
    }
    if s.surface || s.material {
        value *= if dist > 0.0 {
            (-to.z / dist).max(0.0)
        } else {
            1.0
        };
    }
    let mut tint = [1.0f32; 3];
    if let Some(m) = d.mask {
        tint = match make_mask_placement(st, height) {
            None => [0.0; 3],
            Some(p) => arr(mask_factor(&p, s.masks.find(m).unwrap(), to)),
        };
    }
    let c = arr(d.color);
    [
        value * c[0] * tint[0],
        value * c[1] * tint[1],
        value * c[2] * tint[2],
    ]
}

/// Largest difference (relative above 1) between the light buffer and the model of `lights` in
/// the window `[x0, x1) x [y0, y1)`, and the peak of the model.
fn max_error(
    s: &Scene,
    out: &Out,
    lights: &[knorke_light2d::light::LightId],
    (x0, x1, y0, y1): (u32, u32, u32, u32),
) -> (f32, f32) {
    let (mut worst, mut peak) = (0.0f32, 0.0f32);
    for y in y0..y1 {
        for x in x0..x1 {
            let mut want = [0.0f32; 3];
            for &id in lights {
                let one = expected(s, id, x, y);
                for c in 0..3 {
                    want[c] += one[c];
                }
            }
            for c in 0..3 {
                let got = out.px(x, y, c);
                worst = worst.max((got - want[c]).abs() / want[c].max(1.0));
                peak = peak.max(want[c]);
            }
        }
    }
    (worst, peak)
}

const WINDOW56: (u32, u32, u32, u32) = (0, 56, 0, 56);

/// Which point/spot program a profiled or masked light is drawn with (Chiaro `ProgramCase`).
#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    shadows: bool,
    heights: bool,
    /// An empty surface buffer: normal mode, Lambert on flat ground.
    surface: bool,
    /// An empty material buffer at scene size: exact mode.
    material: bool,
}

const CASES: [Case; 7] = [
    Case::new("plain", false, false, false, false),
    Case::new("shadows", true, false, false, false),
    Case::new("heights", true, true, false, false),
    Case::new("surface", false, false, true, false),
    Case::new("surface_shadows", true, true, true, false),
    Case::new("material_exact", false, false, false, true),
    Case::new("material_exact_shadows", true, false, false, true),
];

impl Case {
    const fn new(
        name: &'static str,
        shadows: bool,
        heights: bool,
        surface: bool,
        material: bool,
    ) -> Self {
        Self {
            name,
            shadows,
            heights,
            surface,
            material,
        }
    }

    /// A 64 x 64 scene without ambient light and, for the shadow programs, an obstacle out of
    /// the way in the corner.
    fn scene(&self) -> Scene {
        let mut s = Scene::new(self.name, 0.0, 1.0);
        s.surface = self.surface;
        s.material = self.material;
        if self.shadows {
            s.circle(1.0, 62.0, 62.0, if self.heights { 1.0 } else { 0.0 });
        }
        s
    }

    /// The point light of the profile and mask tests: at (32, 30, 8), pointing down.
    fn light(&self) -> L {
        L {
            x: 32.0,
            y: 30.0,
            height: Height::At(8.0),
            radius: 40.0,
            intensity: 2.0,
            shadows: self.shadows,
            direction: DOWN,
            ..L0
        }
    }
}

// ---- Chiaro tests/gpu/ProfileTests.cpp -----------------------------------------------------

#[test]
fn culled_profiled_light_and_unused_profile_change_nothing() {
    with_host(|fresh| {
        let mut plain = Scene::new("plain", 0.0, 1.0);
        plain.light(L {
            x: 20.0,
            y: 20.0,
            height: Height::At(4.0),
            ..L0
        });
        let mut with = Scene::new("with", 0.0, 1.0);
        with.light(L {
            x: 20.0,
            y: 20.0,
            height: Height::At(4.0),
            ..L0
        });
        let profile = with.profile(ProfilePreset::LedStreet, [None; 4]);
        with.light(L {
            x: -500.0,
            direction: DOWN,
            profile: Some(profile),
            ..L0
        });
        let a = fresh().frame(&plain, 0.0);
        let b = fresh().frame(&with, 0.0);
        assert_eq!(a.light, b.light);
        assert!(b.profile_atlas.is_none(), "a culled light needs no atlas");
    });
}

#[test]
fn profiled_point_light_matches_the_cpu_model_in_every_program() {
    with_host(|fresh| {
        for case in CASES {
            let mut host = fresh();
            let mut s = case.scene();
            let profile = s.profile(
                ProfilePreset::HatLamp,
                [None, Some(0.5), None, Some(0.0)], // C90 half as bright, C270 dark
            );
            let id = s.light(L {
                profile: Some(profile),
                ..case.light()
            });
            let out = host.frame(&s, 0.0);
            let (worst, peak) = max_error(&s, &out, &[id], WINDOW56);
            assert!(worst < 0.01, "{}: error {worst}", case.name);
            assert!(peak > 0.5, "{}: peak {peak}", case.name);
            // -y (C270) is dark, +y (C90) dimmer than +-x, and +x like -x.
            assert!(out.red(32, 16) < 0.01, "{}", case.name);
            assert!(out.red(32, 44) < 0.7 * out.red(46, 30), "{}", case.name);
            let (a, b) = (out.red(46, 30), out.red(17, 30));
            assert!(
                (a - b).abs() <= 0.02 * a.abs().max(b.abs()),
                "{}",
                case.name
            );
            let (w, h, _) = out.profile_atlas.as_ref().expect("profile atlas");
            assert_eq!((*w, *h), (181, 4 * 72), "{}", case.name);
        }
    });
}

#[test]
fn the_pixel_right_under_a_profiled_light_is_lit_in_every_program() {
    // Toward that pixel the direction lies on the profile axis, where the azimuth is undefined
    // (atan2(0, 0)); the CPU model takes azimuth 0 there, and so must the GPU.
    with_host(|fresh| {
        for case in CASES {
            let mut host = fresh();
            let mut s = case.scene();
            let profile = s.profile(ProfilePreset::HatLamp, [None; 4]);
            let id = s.light(L {
                x: 32.5,
                y: 30.5,
                profile: Some(profile),
                ..case.light()
            });
            let out = host.frame(&s, 0.0);
            let want = expected(&s, id, 32, 30)[0];
            assert!(want > 0.5, "{}: {want}", case.name);
            let got = out.red(32, 30);
            assert!(
                (got - want).abs() <= 0.01 * want,
                "{}: {got} vs {want}",
                case.name
            );
        }
    });
}

#[test]
fn profile_rotation_turns_the_distribution_about_the_axis() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Case::new("plain", false, false, false, false).scene();
        let profile = s.profile(ProfilePreset::HatLamp, [None, None, None, Some(0.0)]);
        let id = s.light(L {
            profile: Some(profile),
            profile_rotation: 90.0, // C270 now points to +x
            ..CASES[0].light()
        });
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.01);
        assert!(out.red(46, 30) < 0.01);
        assert!(out.red(18, 30) > 0.3);
    });
}

#[test]
fn a_spot_light_with_a_profile_multiplies_cone_and_profile_on_a_tilted_axis() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        let profile = s.profile(ProfilePreset::LedStreet, [None; 4]);
        let id = s.light(L {
            kind: LightType::Spot,
            direction: Vec3::new(0.3, 0.0, -1.0), // tilted towards +x
            inner: 40.0,
            outer: 70.0,
            profile: Some(profile),
            ..CASES[0].light()
        });
        let out = host.frame(&s, 0.0);
        let (worst, peak) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.01, "error {worst}");
        assert!(peak > 0.2);
    });
}

#[test]
fn a_profile_from_an_ies_file_renders_like_its_cpu_samples() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        let ies = "IESNA:LM-63-2002\nTILT=NONE\n1 1000 1 3 3 1 2 0 0 0 1 1 10\n0 45 90\n0 90 180\n\
                   10 30 0\n10 10 0\n10 20 0\n";
        let table = import_ies(ies.as_bytes()).unwrap();
        assert_eq!(table.len(), PROFILE_VALUES);
        let profile = s.profiles.create(table).unwrap();
        let id = s.light(L {
            profile: Some(profile),
            ..CASES[0].light()
        });
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.01, "error {worst}");
    });
}

#[test]
fn the_profile_atlas_grows_reuses_blocks_and_goes_with_the_last_profile() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        let downs: Vec<ProfileId> =
            (0..6) // block 6 below needs an atlas of 8 blocks
                .map(|_| s.profile(ProfilePreset::Downlight, [None; 4]))
                .collect();
        let last = s.profile(ProfilePreset::HatLamp, [Some(0.0), None, None, None]);
        let mut d = L {
            profile: Some(last),
            ..CASES[0].light()
        };
        let light = s.light(d);
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[light], WINDOW56);
        assert!(worst < 0.01);
        assert_eq!(out.profile_atlas.as_ref().unwrap().1, 8 * 72);
        let objects = out.live;
        // Destroy one and make another in its block: the light moves to the new profile.
        s.profiles.destroy(downs[0], &s.lights).unwrap();
        let again = s.profile(ProfilePreset::HatLamp, [None; 4]);
        assert_eq!(s.profiles.block_of(again), Some(0));
        d.profile = Some(again);
        let res = StoreResources::new(&s.animations, &s.masks).with_profiles(&s.profiles);
        s.lights.update(light, &d.desc(), &res).unwrap();
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[light], WINDOW56);
        assert!(worst < 0.01);
        assert_eq!(out.live, objects, "same atlas, same upload buffer");
        assert_eq!(out.result.allocations, 0);
        // A used profile cannot be destroyed; after the light lets go it can.
        assert!(s.profiles.destroy(again, &s.lights).is_err());
        s.lights.destroy(light).unwrap();
        s.profiles.destroy(again, &s.lights).unwrap();
        s.profiles.destroy(last, &s.lights).unwrap();
        for p in &downs[1..] {
            s.profiles.destroy(*p, &s.lights).unwrap();
        }
        // No profile left: an unprofiled frame needs no atlas, a profiled one cannot exist.
        assert_eq!(s.profiles.block_count(), 0);
        s.light(CASES[0].light());
        let out = host.frame(&s, 0.0);
        assert!(
            out.profile_atlas.is_some_and(|a| a.1 == 8 * 72),
            "kept until needed"
        );
    });
}

#[test]
fn a_profiled_light_without_a_profile_atlas_is_refused() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        let profile = s.profile(ProfilePreset::HatLamp, [None; 4]);
        s.light(L {
            profile: Some(profile),
            ..CASES[0].light()
        });
        // The host forgot `LightAtlases::prepare`: the stage reports it instead of drawing.
        let (w, h) = s.size;
        let scene_tex = texture(host.gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        let modes = LightModes {
            profiles: Some(&s.profiles),
            ..Default::default()
        };
        collect_lights(
            &s.lights,
            &s.settings,
            &s.projection,
            0.0,
            w,
            h,
            &modes,
            &mut host.list,
        );
        let frame = LightingFrame {
            scene: scene_tex.texture_ref(),
            surface: None,
            material: None,
            projection: &s.projection,
            settings: &s.settings,
            max_height: 0.0,
            lights: &host.list,
            shadows: None,
            profile_atlas: None,
            mask_atlas: None,
        };
        let e = host
            .lighting
            .prepare(&mut host.shaders, &mut host.light_pipelines, &frame)
            .unwrap_err();
        assert_eq!(e.code, knorke_light2d::lighting::LIGHTING_ATLAS_MISSING);
    });
}

// ---- Chiaro tests/gpu/MaskTests.cpp ----------------------------------------------------------

fn masked(case: &Case, mask: MaskId) -> L {
    L {
        mask: Some(mask),
        mask_angle: Some(60.0),
        ..case.light()
    }
}

#[test]
fn a_culled_masked_light_and_an_unused_mask_change_nothing() {
    with_host(|fresh| {
        let mut plain = Scene::new("plain", 0.0, 1.0);
        let mut with = Scene::new("with", 0.0, 1.0);
        for s in [&mut plain, &mut with] {
            s.light(L {
                x: 20.0,
                y: 20.0,
                height: Height::At(4.0),
                ..L0
            });
        }
        let mask = with.mask_shape(MaskPreset::WindowHall, Vec3::ONE);
        with.light(L {
            x: -500.0,
            direction: DOWN,
            mask: Some(mask),
            ..L0
        });
        let a = fresh().frame(&plain, 0.0);
        let b = fresh().frame(&with, 0.0);
        assert_eq!(a.light, b.light);
        assert!(b.mask_atlas.is_none(), "nothing masked was drawn");
    });
}

#[test]
fn a_projector_mask_matches_the_cpu_model_in_every_program() {
    with_host(|fresh| {
        for case in CASES {
            let mut host = fresh();
            let mut s = case.scene();
            let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
            let id = s.light(masked(&case, cross));
            let out = host.frame(&s, 0.0);
            let (worst, peak) = max_error(&s, &out, &[id], WINDOW56);
            assert!(worst < 0.02, "{}: error {worst}", case.name);
            assert!(peak > 0.5, "{}", case.name);
            // The bars: tan 60 * 8 = 13.9 pixels from the light to the mask's edge.
            assert!(out.red(32, 24) < 0.05 * out.red(38, 23), "{}", case.name);
            assert!(out.red(38, 23) > 0.3, "{}", case.name);
            assert!(out.red(25, 36) > 0.3, "{}", case.name);
            assert!(out.red(32 + 13, 23) < 0.05, "{}", case.name);
            assert_eq!(out.red(32 + 18, 30), 0.0, "{}: outside the mask", case.name);
            assert_eq!(out.mask_atlas.as_ref().unwrap().1, 256, "{}", case.name);
        }
    });
}

#[test]
fn masked_and_unmasked_lights_in_one_frame_read_their_own_masks() {
    // The mask buffer holds only the masked lights' masks: with unmasked lights before them and
    // two different masks, each light must match the model of the sum.
    with_host(|fresh| {
        for case in CASES {
            let mut host = fresh();
            let mut s = case.scene();
            let plain = L {
                x: 10.0,
                y: 10.0,
                radius: 12.0,
                ..case.light()
            };
            let plain2 = L { x: 50.0, ..plain };
            let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
            let gobo_mask = s.mask_pixels(16, 1, &half_gobo());
            let gobo = L {
                x: 16.0,
                y: 44.0,
                radius: 14.0,
                mask: Some(gobo_mask),
                mask_angle: Some(45.0),
                ..case.light()
            };
            let all = [
                s.light(plain),
                s.light(masked(&case, cross)),
                s.light(plain2),
                s.light(gobo),
            ];
            let out = host.frame(&s, 0.0);
            let (worst, _) = max_error(&s, &out, &all, WINDOW56);
            assert!(worst < 0.02, "{}: error {worst}", case.name);
            assert!(
                out.red(12, 44) > 0.05,
                "{}: the gobo's white half",
                case.name
            );
            assert!(out.red(20, 44) < 0.01, "{}: its black half", case.name);
            assert!(out.red(38, 23) > 0.3, "{}: a pane of the cross", case.name);
        }
    });
}

#[test]
fn a_masked_light_in_direction_material_mode_renders_through_the_mask() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("direction", 0.0, 0.5);
        s.material = true;
        let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
        s.light(masked(&CASES[5], cross));
        let out = host.frame(&s, 0.0);
        assert_eq!(out.result.detail_mode, DetailMode::Direction);
        assert_eq!(out.light_size.0, 32);
        assert!(out.red(19, 11) > 0.2, "a pane (scene 38, 23)");
        assert_eq!(
            out.red(26, 15),
            0.0,
            "beyond the mask's edge, within the radius"
        );
    });
}

#[test]
fn the_mask_colour_multiplies_the_light_colour_per_channel() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        // sRGB (255, 128, 0): red passes, green ~0.216 (linear), blue is held back.
        let gobo = s.mask_pixels(1, 1, &[255, 128, 0, 255]);
        let id = s.light(L {
            color: Vec3::new(0.5, 1.0, 1.0),
            mask: Some(gobo),
            mask_angle: Some(60.0),
            ..CASES[0].light()
        });
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.02, "error {worst}");
        let (r, g, b) = (out.px(36, 33, 0), out.px(36, 33, 1), out.px(36, 33, 2));
        assert!(r > 0.2);
        assert!(
            (g / r - 2.0 * 0.2158).abs() <= 0.03 * 2.0 * 0.2158,
            "{}",
            g / r
        );
        assert_eq!(b, 0.0);
    });
}

/// The window light of the Chiaro tests: a wall window at x = 36 facing +-x, 8 wide (y 26 to
/// 34) and 6 high (z 0 to 6).
fn window_light(case: &Case, mask: MaskId, x: f32, height: f32) -> L {
    L {
        x,
        height: Height::At(height),
        radius: 60.0,
        mask: Some(mask),
        mask_angle: Some(60.0),
        window: Some([
            Vec3::new(36.0, 30.0, 3.0),
            Vec3::new(0.0, 4.0, 0.0),
            Vec3::new(0.0, 0.0, 3.0),
        ]),
        ..case.light()
    }
}

#[test]
fn a_window_mask_falls_where_the_geometry_puts_it_both_ways_round() {
    with_host(|fresh| {
        for way in 0..2 {
            let mut host = fresh();
            let mut s = CASES[0].scene();
            let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
            // Lamp inside the hall, light out onto the street; or a lamp high outside, light
            // into the hall.
            let (x, height) = if way == 0 { (26.0, 5.0) } else { (50.0, 12.0) };
            let l = window_light(&CASES[0], cross, x, height);
            let id = s.light(l);
            let out = host.frame(&s, 0.0);
            let (worst, peak) = max_error(&s, &out, &[id], (0, 64, 0, 64));
            assert!(worst < 0.02, "way {way}: error {worst}");
            assert!(peak > 0.2, "way {way}");
            // The light's side of the window stays dark; the far side gets the stretched cross.
            let (near_x, far_x) = if way == 0 { (30, 44) } else { (42, 30) };
            let sum = |x: u32| (0..64).map(|y| out.red(x, y)).sum::<f32>();
            assert_eq!(sum(near_x), 0.0, "way {way}");
            assert!(sum(far_x) > 1.0, "way {way}");
            // On the far side the bars are visible: pixels the unmasked light reaches but the
            // cross holds back.
            let open = L {
                mask: None,
                window: None,
                ..l
            };
            let mut open_scene = CASES[0].scene();
            let open_id = open_scene.light(open);
            let (mut bars, mut panes) = (0, 0);
            for y in 0..64 {
                let xs = if way == 0 { 37..64 } else { 0..35 };
                for x in xs {
                    let full = expected(&open_scene, open_id, x, y)[0];
                    if full > 0.3 && out.red(x, y) < 0.05 * full {
                        bars += 1;
                    } else if full > 0.3 && out.red(x, y) > 0.9 * full {
                        panes += 1;
                    }
                }
            }
            assert!(bars > 20, "way {way}: {bars} bars");
            assert!(panes > 20, "way {way}: {panes} panes");
        }
    });
}

#[test]
fn a_window_lamp_inside_a_building_lights_the_street_and_obstacles_outside_still_cast() {
    with_host(|fresh| {
        for heights in [false, true] {
            let case = Case::new("window", true, heights, false, false);
            let render = |host: &mut Host<'_>, outside_obstacle: bool| {
                let mut s = Scene::new("window", 0.0, 1.0);
                s.circle(1.0, 62.0, 62.0, if heights { 1.0 } else { 0.0 });
                // The hall: a box west of the window wall at x = 36, the lamp inside it.
                let hall_height = if heights { 7.0 } else { 0.0 };
                s.polygon(
                    &[5.0, 12.0, 35.0, 12.0, 35.0, 48.0, 5.0, 48.0],
                    hall_height,
                    1.0,
                );
                if outside_obstacle {
                    s.circle(1.5, 42.0, 33.0, hall_height);
                }
                let cross = s.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
                let id = s.light(window_light(&case, cross, 26.0, 5.0));
                let out = host.frame(&s, 0.0);
                (s, id, out)
            };
            let (s, id, open) = render(&mut fresh(), false);
            // Outside the hall the street is lit as without obstacles (the shadow ray ends at
            // the window).
            let mut model = CASES[0].scene();
            let cross = model.mask_shape(MaskPreset::WindowCross, Vec3::ONE);
            let model_id = model.light(window_light(&CASES[0], cross, 26.0, 5.0));
            let (worst, peak) = max_error(&model, &open, &[model_id], (39, 64, 0, 64));
            assert!(worst < 0.03, "heights {heights}: error {worst}");
            assert!(peak > 0.2);
            let _ = (s, id);
            let (_, _, blocked) = render(&mut fresh(), true);
            let mut darker = 0;
            for y in 0..64 {
                for x in 45..64 {
                    if open.red(x, y) > 0.1 && blocked.red(x, y) < 0.2 * open.red(x, y) {
                        darker += 1;
                    }
                }
            }
            assert!(darker > 10, "heights {heights}: {darker} darker pixels");
        }
    });
}

#[test]
fn the_mask_atlas_grows_with_the_masks_and_lights_find_their_blocks() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = CASES[0].scene();
        let blacks: Vec<MaskId> = (0..3) // block 2 needs an atlas of 4 blocks
            .map(|_| s.mask_pixels(1, 1, &[0, 0, 0, 255]))
            .collect();
        let white = s.mask_pixels(1, 1, &[255, 255, 255, 255]);
        let id = s.light(masked(&CASES[0], white));
        let out = host.frame(&s, 0.0);
        assert_eq!(out.mask_atlas.as_ref().unwrap().1, 4 * 256);
        let (worst, _) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.02);
        assert!(out.red(34, 32) > 0.5);
        // Destroying one of the black masks re-uploads; the white light keeps its block.
        s.masks.destroy(blacks[1], &s.lights).unwrap();
        let out = host.frame(&s, 0.0);
        let (worst, _) = max_error(&s, &out, &[id], WINDOW56);
        assert!(worst < 0.02);
        assert_eq!(out.record.lights_drawn, 1);
    });
}

/// The three profile tables of `scene_profile` against C++ Chiaro's `table<n>.bin`. The shape
/// builder uses the platform independent `det` maths, which differs from `std` by an ulp or two
/// in a few hundred of the 13032 values (finding of P26); the light buffer above is bit
/// identical regardless. Bound here: at most 4 ulp.
#[test]
fn the_profile_tables_of_the_golden_scene_stay_within_four_ulp_of_chiaro() {
    let s = scene_profile();
    let (mut count, mut differing, mut worst) = (0, 0usize, 0u32);
    for (index, (_, table)) in s.profiles.iter().enumerate() {
        let theirs = golden(&format!("table{index}.bin"));
        assert_eq!(theirs.len(), table.len() * 4, "profile {index}");
        for (v, c) in table.iter().zip(theirs.chunks_exact(4)) {
            let c = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if v.to_bits() != c.to_bits() {
                differing += 1;
                worst = worst.max(v.to_bits().abs_diff(c.to_bits()));
            }
        }
        count += 1;
    }
    eprintln!("profile tables: {differing} values differ, at most {worst} ulp");
    assert_eq!(count, 3);
    assert!(worst <= 4, "{worst} ulp");
}

// ---- the atlas bytes on the CPU --------------------------------------------------------------

#[test]
fn the_atlas_bytes_hold_every_block_and_zero_elsewhere() {
    let mut s = CASES[0].scene();
    let a = s.profile(ProfilePreset::Downlight, [None; 4]);
    let b = s.profile(ProfilePreset::HatLamp, [None; 4]);
    let rows = profile_atlas_rows(s.profiles.block_count());
    assert_eq!(rows, 4 * 72);
    let mut bytes = vec![0xAAu8; profile_atlas_bytes(rows) as usize];
    fill_profile_atlas(&s.profiles, rows, &mut bytes);
    let block = PROFILE_VALUES * 4;
    for (id, at) in [(a, 0), (b, 1)] {
        let values = s.profiles.find(id).unwrap();
        let got: Vec<f32> = bytes[at * block..(at + 1) * block]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(got, values);
    }
    assert!(bytes[2 * block..].iter().all(|&v| v == 0));

    let m = s.mask_pixels(1, 1, &[255, 255, 255, 255]);
    let rows = mask_atlas_rows(s.masks.block_count());
    assert_eq!(rows, 256);
    let mut bytes = vec![0xAAu8; mask_atlas_bytes(rows) as usize];
    fill_mask_atlas(&s.masks, rows, &mut bytes);
    assert_eq!(&bytes[..], s.masks.find(m).unwrap());
}

// ---- Chiaro tests/gpu/AmbientZoneTests.cpp ---------------------------------------------------

/// The fold at the centre of every pixel against the light buffer of a white scene without
/// lights (Chiaro `CheckAgainstFold`).
fn check_against_fold(s: &Scene, out: &Out) {
    let ambient = s.settings.ambient;
    let tol = 2.0 / 1024.0;
    let mut bad = 0;
    for y in 0..s.size.1 {
        for x in 0..s.size.0 {
            let want = fold_ambient(
                &s.zones,
                [ambient.x, ambient.y, ambient.z],
                Vec3::new(x as f32 + 0.5, y as f32 + 0.5, 0.0),
                true,
            );
            for (c, w) in arr(want).into_iter().enumerate() {
                let got = out.px(x, y, c);
                if (got - w).abs() > tol * w.abs().max(1.0) {
                    bad += 1;
                    assert!(bad < 8, "pixel {x}, {y} channel {c}: got {got}, fold {w}");
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

fn zone_scene(ambient: f32) -> Scene {
    Scene::new("zones", ambient, 1.0)
}

fn point(x: f32, y: f32, radius: f32, intensity: f32, shadows: bool) -> L {
    L {
        x,
        y,
        height: Height::At(6.0),
        radius,
        intensity,
        shadows,
        ..L0
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 2.0 / 1024.0 * b.abs().max(1.0)
}

#[test]
fn no_zones_and_invisible_zones_change_nothing() {
    with_host(|fresh| {
        for with_light in [false, true] {
            let mut host = fresh();
            let mut s = zone_scene(1.0);
            if with_light {
                s.light(point(20.0, 20.0, 30.0, 1.0, false));
            }
            host.frame(&s, 0.0);
            let before = host.frame(&s, 0.0);
            s.zone(&AmbientZoneParams {
                disabled: true,
                ..zone_box(32.0, 32.0, 20.0, 20.0, CELLAR)
            });
            s.zone(&zone_box(1000.0, 32.0, 20.0, 20.0, CELLAR));
            let after = host.frame(&s, 0.0);
            assert_eq!(after.light, before.light, "light {with_light}");
            assert_eq!(after.record, before.record);
            assert_eq!(after.live, before.live);
            assert_eq!(after.result, before.result);
        }
    });
}

#[test]
fn a_box_zone_without_lights_fills_the_light_buffer() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&zone_box(32.0, 32.0, 20.0, 20.0, CELLAR));
        let out = host.frame(&s, 0.0);
        assert!(out.result.ambient_zones);
        assert!(out.result.light_buffer);
        assert_eq!(out.result.pure_ambient, PureAmbient::LightBuffer);
        assert_eq!(out.px(32, 32, 0), 0.25);
        assert_eq!(out.px(32, 32, 1), 0.5);
        assert_eq!(out.px(32, 32, 2), 1.0);
        assert_eq!(out.px(44, 32, 0), 1.0); // 2 pixels outside
        assert_eq!(out.px(21, 32, 0), 1.0); // the edge lies at x = 22
        assert_eq!(out.px(23, 32, 0), 0.25);
        check_against_fold(&s, &out);
    });
}

#[test]
fn the_soft_edge_follows_the_sampler() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&AmbientZoneParams {
            soft_edge: 6.0,
            ..zone_box(32.0, 32.0, 40.0, 40.0, CELLAR)
        });
        let out = host.frame(&s, 0.0);
        assert_eq!(out.px(10, 32, 0), 1.0);
        assert_eq!(out.px(32, 32, 0), 0.25);
        check_against_fold(&s, &out);
    });
}

#[test]
fn circle_concave_polygon_and_rotated_box_match_the_fold() {
    with_host(|fresh| {
        // Circle.
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&zone_disc(30.0, 34.0, 12.0, CELLAR));
        let out = host.frame(&s, 0.0);
        assert_eq!(out.px(30, 34, 0), 0.25);
        assert_eq!(out.px(30, 50, 0), 1.0);
        check_against_fold(&s, &out);
        // An L: the notch (upper right quarter) stays global.
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&AmbientZoneParams {
            soft_edge: 2.0,
            ..zone_polygon(
                &[
                    8.0, 8.0, 56.0, 8.0, 56.0, 32.0, 32.0, 32.0, 32.0, 56.0, 8.0, 56.0,
                ],
                CELLAR,
            )
        });
        let out = host.frame(&s, 0.0);
        assert_eq!(out.px(16, 16, 0), 0.25);
        assert_eq!(out.px(44, 44, 0), 1.0); // the notch
        check_against_fold(&s, &out);
        // Rotated box.
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&AmbientZoneParams {
            rotation: 45.0,
            ..zone_box(32.0, 32.0, 30.0, 30.0, CELLAR)
        });
        let out = host.frame(&s, 0.0);
        assert_eq!(out.px(32, 32, 0), 0.25);
        assert_eq!(out.px(20, 20, 0), 1.0); // inside the axis-aligned hull, outside the turned box
        check_against_fold(&s, &out);
    });
}

#[test]
fn overlapping_zones_fold_by_priority_then_creation() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&AmbientZoneParams {
            priority: 1,
            ..zone_box(24.0, 32.0, 30.0, 30.0, [0.1; 3])
        });
        let b = s.zone(&zone_box(40.0, 32.0, 30.0, 30.0, [0.2; 3]));
        s.zone(&zone_box(40.0, 40.0, 30.0, 30.0, [0.3; 3]));
        let out = host.frame(&s, 0.0);
        assert!(near(out.px(32, 32, 0), 0.1)); // priority 1 wins over the later zones
        assert!(near(out.px(50, 40, 0), 0.3)); // equal priority: the later created
        assert!(near(out.px(50, 20, 0), 0.2));
        check_against_fold(&s, &out);
        // Set keeps the place: b recoloured still lies below c.
        let recoloured = zone_box(40.0, 32.0, 30.0, 30.0, [0.4; 3]);
        s.zones
            .set(b, make_stored_zone(&recoloured).unwrap())
            .unwrap();
        let out = host.frame(&s, 0.0);
        assert!(near(out.px(50, 40, 0), 0.3));
        assert!(near(out.px(50, 20, 0), 0.4));
        check_against_fold(&s, &out);
    });
}

#[test]
fn a_relative_zone_follows_the_global_ambient() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = zone_scene(1.0);
        s.zone(&AmbientZoneParams {
            relative: true,
            ..zone_box(32.0, 32.0, 20.0, 20.0, [0.5; 3])
        });
        s.settings.ambient = Vec3::new(0.4, 0.4, 0.4);
        let out = host.frame(&s, 0.0);
        assert!(near(out.px(32, 32, 0), 0.2));
        assert!(near(out.px(2, 2, 0), 0.4));
        check_against_fold(&s, &out);
    });
}

#[test]
fn lights_add_on_top_of_the_zones() {
    with_host(|fresh| {
        for shadows in [false, true] {
            let zone = AmbientZoneParams {
                soft_edge: 4.0,
                ..zone_box(32.0, 32.0, 30.0, 30.0, CELLAR)
            };
            let mut zoned = zone_scene(1.0);
            let mut zone_only = zone_scene(1.0);
            let mut light_only = zone_scene(0.0);
            zoned.zone(&zone);
            zone_only.zone(&zone);
            for s in [&mut zoned, &mut light_only] {
                s.light(point(30.0, 30.0, 28.0, 1.5, shadows));
                if shadows {
                    s.polygon(&[40.0, 24.0, 44.0, 24.0, 44.0, 28.0, 40.0, 28.0], 0.0, 1.0);
                }
            }
            let a = fresh().frame(&zoned, 0.0);
            let z = fresh().frame(&zone_only, 0.0);
            let l = fresh().frame(&light_only, 0.0);
            assert_eq!(a.result.pure_ambient, PureAmbient::None);
            let mut bad = 0;
            for y in 0..64 {
                for x in 0..64 {
                    for c in 0..3 {
                        let want = z.px(x, y, c) + l.px(x, y, c);
                        if !near(a.px(x, y, c), want) {
                            bad += 1;
                        }
                    }
                }
            }
            assert_eq!(bad, 0, "shadows {shadows}");
        }
    });
}

#[test]
fn the_flat_directional_sum_stays_inside_the_zones() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = zone_scene(0.2);
        s.light(L {
            kind: LightType::Directional,
            color: Vec3::new(0.1, 0.2, 0.3),
            intensity: 1.0,
            direction: Vec3::new(0.0, 0.0, -1.0),
            ..L0
        });
        s.zone(&zone_box(32.0, 32.0, 20.0, 20.0, [0.05; 3]));
        let out = host.frame(&s, 0.0);
        assert!(near(out.px(32, 32, 0), 0.15));
        assert!(near(out.px(32, 32, 2), 0.35));
        assert!(near(out.px(2, 2, 0), 0.3));
        assert!(near(out.px(2, 2, 2), 0.5));
    });
}

#[test]
fn pixels_outside_the_zones_are_bit_identical_in_every_mode() {
    with_host(|fresh| {
        for (name, surface, material) in [
            ("flat", false, false),
            ("normal", true, false),
            ("exact", false, true),
        ] {
            let build = |zoned: bool| {
                let mut s = zone_scene(0.0);
                s.settings.ambient = Vec3::new(0.3, 0.35, 0.4);
                s.surface = surface;
                s.material = material;
                s.light(point(20.0, 40.0, 30.0, 1.2, true));
                s.light(point(48.0, 16.0, 25.0, 0.8, false));
                s.polygon(&[28.0, 28.0, 32.0, 28.0, 32.0, 34.0, 28.0, 34.0], 0.0, 1.0);
                if zoned {
                    s.zone(&AmbientZoneParams {
                        soft_edge: 2.0,
                        ..zone_box(44.0, 44.0, 12.0, 12.0, CELLAR)
                    });
                }
                s
            };
            let with = fresh().frame(&build(true), 0.0);
            let without = fresh().frame(&build(false), 0.0);
            let (mut differ, mut inside) = (0, 0);
            for y in 0..64u32 {
                for x in 0..64u32 {
                    let near_zone = x + 3 >= 38 && x <= 53 && y + 3 >= 38 && y <= 53;
                    let i = ((y * 64 + x) * 8) as usize;
                    let same = with.light[i..i + 8] == without.light[i..i + 8];
                    if near_zone {
                        inside += usize::from(!same);
                    } else {
                        differ += usize::from(!same);
                    }
                }
            }
            assert_eq!(differ, 0, "{name}");
            assert!(inside > 0, "{name}: the zone does something");
        }
    });
}

#[test]
fn a_whole_image_zone_equals_the_global_ambient() {
    with_host(|fresh| {
        let c = [0.3, 0.2, 0.1];
        for (name, surface, material) in [
            ("flat", false, false),
            ("normal", true, false),
            ("exact", false, true),
        ] {
            for with_light in [false, true] {
                let build = |zoned: bool| {
                    let mut s = zone_scene(0.0);
                    s.settings.ambient = if zoned {
                        Vec3::new(1.0, 1.0, 1.0)
                    } else {
                        Vec3::new(c[0], c[1], c[2])
                    };
                    s.surface = surface;
                    s.material = material;
                    if zoned {
                        s.zone(&zone_box(32.0, 32.0, 200.0, 200.0, c));
                    }
                    if with_light {
                        s.light(point(24.0, 30.0, 30.0, 1.0, false));
                    }
                    s
                };
                let a = fresh().frame(&build(true), 0.0);
                let b = fresh().frame(&build(false), 0.0);
                let mut worst = 0.0f32;
                for y in 0..64 {
                    for x in 0..64 {
                        for ch in 0..3 {
                            // Without zones and lights there is no light buffer: the
                            // composite multiplies by the ambient setting itself.
                            let want = if b.light.is_empty() {
                                c[ch]
                            } else {
                                b.px(x, y, ch)
                            };
                            worst = worst.max((a.px(x, y, ch) - want).abs());
                        }
                    }
                }
                assert!(worst <= 2.0 / 1024.0, "{name} light {with_light}: {worst}");
            }
        }
    });
}

#[test]
fn light_resolution_half_keeps_the_zone_edges_within_two_scene_pixels() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("half", 1.0, 0.5);
        s.zone(&zone_box(32.0, 32.0, 20.0, 20.0, CELLAR));
        let out = host.frame(&s, 0.0);
        assert_eq!(out.light_size, (32, 32));
        for x in [10u32, 19, 45, 55] {
            assert_eq!(out.px(x / 2, 16, 0), 1.0, "x {x}");
        }
        for x in [25u32, 32, 39] {
            assert!(near(out.px(x / 2, 16, 0), 0.25), "x {x}");
        }
    });
}

#[test]
fn zones_without_lights_skip_the_light_pass_and_add_one_pass_with_lights() {
    with_host(|fresh| {
        let (mut plain, mut zoned) = (fresh(), fresh());
        let sp = zone_scene(1.0);
        let mut sz = zone_scene(1.0);
        sz.zone(&zone_box(32.0, 32.0, 20.0, 20.0, CELLAR));
        let (a, b) = (plain.frame(&sp, 0.0), zoned.frame(&sz, 0.0));
        assert_eq!(a.record.draws, 0);
        assert_eq!(b.record.draws, 1);
        assert_eq!(b.record.zones_drawn, 1);
        assert_eq!(b.record.lights_drawn, 0);
        // With a light both have a light pass; the zone adds its draw.
        let mut sp = zone_scene(1.0);
        let mut sz = zone_scene(1.0);
        sz.zone(&zone_box(32.0, 32.0, 20.0, 20.0, CELLAR));
        for s in [&mut sp, &mut sz] {
            s.light(point(20.0, 20.0, 20.0, 1.0, false));
        }
        let (c, d) = (plain.frame(&sp, 0.0), zoned.frame(&sz, 0.0));
        assert_eq!(d.record.draws, c.record.draws + 1);
        assert_eq!(d.record.lights_drawn, c.record.lights_drawn);
    });
}

#[test]
fn zones_in_a_steady_state_allocate_nothing() {
    with_host(|fresh| {
        for with_light in [false, true] {
            let mut host = fresh();
            let mut s = zone_scene(1.0);
            s.surface = true;
            if with_light {
                s.light(point(32.0, 32.0, 30.0, 1.0, false));
            }
            s.zone(&AmbientZoneParams {
                x: 40.0,
                y: 40.0,
                ..zone_polygon(&[0.0, 0.0, 10.0, 0.0, 0.0, 10.0], CELLAR)
            });
            let zone = s.zone(&zone_box(20.0, 20.0, 16.0, 16.0, CELLAR));
            let mut live = 0;
            for i in 0..10 {
                s.projection = Projection::ortho(at(32.0 + i as f32, 32.0), 1.0, 64, 64);
                let v = 0.1 * (i % 5) as f32;
                let params = zone_box(20.0, 20.0, 16.0, 16.0, [v; 3]);
                s.zones
                    .set(zone, make_stored_zone(&params).unwrap())
                    .unwrap();
                let out = host.frame(&s, 0.0);
                if i == 1 {
                    live = out.live;
                }
                if i > 1 {
                    assert_eq!(out.result.allocations, 0, "frame {i}");
                    assert_eq!(out.live, live, "frame {i}");
                }
            }
        }
    });
}

#[test]
fn a_polygon_zone_off_screen_costs_nothing() {
    with_host(|fresh| {
        for with_light in [false, true] {
            let (mut plain, mut zoned) = (fresh(), fresh());
            let (mut sp, mut sz) = (zone_scene(1.0), zone_scene(1.0));
            sp.surface = true;
            sz.surface = true;
            if with_light {
                for s in [&mut sp, &mut sz] {
                    s.light(point(20.0, 20.0, 30.0, 1.0, false));
                }
            }
            for i in 0..100 {
                sz.zone(&AmbientZoneParams {
                    x: 500.0 + 20.0 * i as f32,
                    y: -300.0,
                    ..zone_polygon(&[0.0, 0.0, 10.0, 0.0, 0.0, 10.0, -4.0, 6.0], CELLAR)
                });
            }
            plain.frame(&sp, 0.0);
            zoned.frame(&sz, 0.0);
            let (a, b) = (plain.frame(&sp, 0.0), zoned.frame(&sz, 0.0));
            assert_eq!(b.live, a.live, "light {with_light}");
            assert_eq!(b.record, a.record);
            assert_eq!(b.result, a.result);
            assert_eq!(b.light, a.light);
            assert!(zoned.lighting.zone_pass().vertex_texture().is_none());
        }
    });
}

#[test]
fn a_zone_leaving_the_screen_frees_its_gpu_objects() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("leaving", 0.3, 0.5);
        s.material = true; // direction mode: the visible frame also holds the pure ambient target
        s.light(point(20.0, 20.0, 30.0, 1.0, false));
        let id = s.zone(&AmbientZoneParams {
            x: 10.0,
            y: 10.0,
            ..zone_polygon(&[0.0, 0.0, 30.0, 0.0, 0.0, 30.0], CELLAR)
        });
        let visible = host.frame(&s, 0.0);
        assert_eq!(visible.result.pure_ambient, PureAmbient::Separate);
        assert!(host.lighting.zone_pass().vertex_texture().is_some());
        assert!(host.lighting.pure_ambient().is_some());
        let moved = AmbientZoneParams {
            x: 5000.0,
            y: 10.0,
            ..zone_polygon(&[0.0, 0.0, 30.0, 0.0, 0.0, 30.0], CELLAR)
        };
        s.zones.set(id, make_stored_zone(&moved).unwrap()).unwrap();
        host.frame(&s, 0.0);
        let gone = host.frame(&s, 0.0);
        assert!(!gone.result.ambient_zones);
        assert!(host.lighting.zone_pass().vertex_texture().is_none());
        assert!(host.lighting.pure_ambient().is_none());
        assert!(gone.live < visible.live, "{} < {}", gone.live, visible.live);
    });
}

#[test]
fn zones_without_lights_in_direction_mode_need_no_second_target() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("direction", 1.0, 0.5);
        s.material = true;
        s.zone(&zone_box(32.0, 32.0, 20.0, 20.0, CELLAR));
        let out = host.frame(&s, 0.0);
        assert_eq!(out.result.pure_ambient, PureAmbient::LightBuffer);
        assert!(host.lighting.pure_ambient().is_none());
        assert!(near(out.px(16, 16, 0), 0.25));
        assert_eq!(out.px(2, 2, 0), 1.0);
        assert!(out.second.is_empty(), "no material target without lights");
    });
}

#[test]
fn highlight_in_a_dark_room_keeps_the_pure_ambient_per_pixel() {
    // Direction mode with a light: the zone pass writes the pure ambient next to the light
    // buffer, and it holds the zone's ambient where the light buffer holds ambient plus light.
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = Scene::new("dark", 0.5, 0.5);
        s.material = true;
        s.zone(&zone_box(32.0, 32.0, 44.0, 44.0, [0.1; 3]));
        s.light(L {
            x: 30.0,
            y: 32.0,
            height: Height::At(8.0),
            radius: 26.0,
            intensity: 1.5,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert_eq!(out.result.pure_ambient, PureAmbient::Separate);
        assert_eq!(out.result.detail_mode, DetailMode::Direction);
        let at = |buf: &[u8], x: u32, y: u32| {
            let i = ((y * out.light_size.0 + x) * 8) as usize;
            half(u16::from_le_bytes([buf[i], buf[i + 1]]))
        };
        assert!(
            near(at(&out.ambient, 16, 16), 0.1),
            "pure ambient in the room"
        );
        assert!(near(at(&out.ambient, 2, 2), 0.5), "pure ambient outside");
        assert!(
            at(&out.light, 15, 16) > at(&out.ambient, 15, 16) + 0.3,
            "the lamp adds on top"
        );
    });
}

/// The record of a frame (`record_without_prepare` of the P25 test is unchanged by zones).
#[test]
fn lights_and_zones_repeat_bit_identically() {
    with_host(|fresh| {
        let mut host = fresh();
        let s = scene_zones();
        let a = host.frame(&s, 0.0);
        let b = host.frame(&s, 0.0);
        assert_eq!(a.light, b.light);
        assert_eq!(b.result.allocations, 0);
        assert_eq!(a.record, b.record);
        assert_eq!(a.record.zones_drawn, 5);
    });
}

// ---- the pass plan (P31) --------------------------------------------------------------------
//
// The same golden scenes through `PassPlan`, the fixed stage order: occlusion, atlases, zones and
// lights, composite. Chiaro's HDR buffer (`<scene>.hdr.bin`, `atlas_ref.cpp` since P31) checks the
// composite with ambient zones, the pure ambient of the direction mode included.

/// The scene and material textures of a golden scene (white scene, empty material buffer).
struct PlanTextures<'d> {
    scene: Texture<'d>,
    material: Option<Texture<'d>>,
}

impl<'d> PlanTextures<'d> {
    fn new(gpu: Gpu<'d>, s: &Scene) -> Self {
        let (w, h) = s.size;
        assert!(!s.surface, "the plan scenes have no surface buffer");
        Self {
            scene: texture(gpu, w, h, &vec![255u8; (w * h * 4) as usize]),
            material: s
                .material
                .then(|| texture(gpu, w, h, &vec![0u8; (w * h * 4) as usize])),
        }
    }

    fn frame<'a>(&'a self, s: &'a Scene) -> PassFrame<'a> {
        PassFrame {
            material: self.material.as_ref().map(Texture::texture_ref),
            ..PassFrame::new(self.scene.texture_ref(), &s.projection)
        }
    }
}

fn plan_stores(s: &Scene) -> SceneStores<'_> {
    SceneStores {
        zones: Some(&s.zones),
        profiles: Some(&s.profiles),
        masks: Some(&s.masks),
        animations: Some(&s.animations),
        ..SceneStores::new(&s.lights, &s.obstacles)
    }
}

/// What one frame of the plan left: the HDR and light buffer bytes and the decision.
struct PlanOut {
    prepared: PassPrepared,
    record: PassRecord,
    hdr: Vec<u8>,
    light: Vec<u8>,
}

fn plan_frame(
    gpu: Gpu<'_>,
    plan: &mut PassPlan<'_>,
    s: &Scene,
    stores: &SceneStores<'_>,
    tex: &PlanTextures<'_>,
) -> PlanOut {
    let frame = tex.frame(s);
    let prepared = plan.prepare(&frame, stores, &s.settings).unwrap();
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    let record = plan.record(&mut cmd, &frame, &s.settings).unwrap();
    let download = |cmd: &mut CommandBuffer<'_>, t: TextureRef<'_>| {
        let down = DownloadBuffer::new(gpu, t.desc().bytes() as u32).unwrap();
        cmd.copy_pass().unwrap().download_texture(t, &down).unwrap();
        down
    };
    let hdr = download(&mut cmd, plan.hdr().unwrap());
    let light = plan
        .lighting()
        .light_buffer()
        .map(|t| download(&mut cmd, t));
    cmd.submit_and_wait().unwrap();
    let read = |mut d: DownloadBuffer<'_>| {
        let mut v = vec![0; d.size() as usize];
        d.read(&mut v).unwrap();
        v
    };
    PlanOut {
        prepared,
        record,
        hdr: read(hdr),
        light: light.map(read).unwrap_or_default(),
    }
}

/// The device, the pack and a counter for one test of the plan.
fn with_plan(body: impl for<'d> FnOnce(Gpu<'d>, &'d LiveCounter, &dyn Fn() -> PassPlan<'d>)) {
    let Some((_guard, device, pack)) = device() else {
        return;
    };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let fresh = || PassPlan::new(gpu, pack.clone()).unwrap();
        body(gpu, &counter, &fresh);
    }
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}

#[test]
fn pass_plan_matches_chiaro_up_to_the_hdr_buffer() {
    with_plan(|gpu, _, fresh| {
        let mut failures = Vec::new();
        for s in [
            scene_profile(),
            scene_mask(),
            scene_zones(),
            scene_zones_only(),
            scene_zones_direction(),
        ] {
            let mut plan = fresh();
            let tex = PlanTextures::new(gpu, &s);
            let out = plan_frame(gpu, &mut plan, &s, &plan_stores(&s), &tex);
            let name = s.name;
            for (what, got) in [("light", &out.light), ("hdr", &out.hdr)] {
                let (n, max) = diff(got, &golden(&format!("{name}.{what}.bin")));
                eprintln!("{name}: {what} buffer {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.{what}: {n} texels, max {max}"));
                }
            }
            let stages = out.prepared.stages;
            assert!(stages.contains(PassStages::COMPOSITE), "{name}");
            assert_eq!(
                stages.contains(PassStages::ZONES),
                name.starts_with("zones"),
                "{name}: zone pass"
            );
            assert_eq!(
                stages.contains(PassStages::LIGHTS),
                name != "zones_only",
                "{name}: light pass"
            );
            assert_eq!(
                stages.contains(PassStages::OCCLUSION),
                out.prepared.mask_update.is_some(),
                "{name}"
            );
            if name == "zones_direction" {
                assert_eq!(out.prepared.lighting.pure_ambient, PureAmbient::Separate);
            }
            let mut sink = Diagnostics::default();
            assert_eq!(plan.check_invariants(&mut sink), 0, "{name}: {:?}", sink.0);
        }
        assert!(failures.is_empty(), "not bit identical: {failures:#?}");
    });
}

#[derive(Default)]
struct Diagnostics(Vec<knorke_types::Diagnostic>);

impl knorke_observe::Sink<knorke_types::Diagnostic> for Diagnostics {
    fn emit(&mut self, d: knorke_types::Diagnostic) {
        self.0.push(d);
    }
}

#[test]
fn pass_plan_steady_state_reuses_the_mask_and_allocates_nothing() {
    with_plan(|gpu, counter, fresh| {
        let mut plan = fresh();
        let s = scene_zones();
        let tex = PlanTextures::new(gpu, &s);
        let stores = plan_stores(&s);
        let first = plan_frame(gpu, &mut plan, &s, &stores, &tex);
        assert_eq!(first.prepared.mask_update, Some(MaskUpdate::Full));
        assert!(first.prepared.allocations > 0);
        let live = counter.live();
        for _ in 0..3 {
            let again = plan_frame(gpu, &mut plan, &s, &stores, &tex);
            assert_eq!(again.prepared.mask_update, Some(MaskUpdate::Reused));
            assert_eq!(again.prepared.allocations, 0);
            assert_eq!(again.hdr, first.hdr, "bit identical frame to frame");
            assert_eq!(again.record.lighting, first.record.lighting);
            assert_eq!(counter.live(), live, "no GPU object per frame");
        }
        let stats = plan.stats();
        assert_eq!(
            (stats.frames, stats.mask_full, stats.mask_reused),
            (4, 1, 3)
        );
        // After invalidate the mask is rebuilt, and the image stays the same.
        plan.invalidate();
        let rebuilt = plan_frame(gpu, &mut plan, &s, &stores, &tex);
        assert_eq!(rebuilt.prepared.mask_update, Some(MaskUpdate::Full));
        assert_eq!(rebuilt.hdr, first.hdr);
    });
}

/// GPU resources are no simulation state (ADR 0010): stores saved and restored into fresh
/// stores, drawn by a new plan, give the same image.
#[test]
fn pass_plan_rebuilds_the_same_image_from_restored_stores() {
    use knorke_types::{Reader, SnapshotBuf, restore};
    with_plan(|gpu, _, fresh| {
        let s = scene_mask();
        let tex = PlanTextures::new(gpu, &s);
        let mut plan = fresh();
        let before = plan_frame(gpu, &mut plan, &s, &plan_stores(&s), &tex);
        drop(plan);

        let mut buf = SnapshotBuf::new();
        buf.save(&s.lights);
        buf.save(&s.obstacles);
        buf.save(&s.masks);
        buf.save(&s.animations);
        let mut t = Scene::golden("mask", s.settings.ambient, 1.0);
        let mut r = Reader::new(buf.bytes());
        restore(&mut r, &mut t.lights).unwrap();
        restore(&mut r, &mut t.obstacles).unwrap();
        restore(&mut r, &mut t.masks).unwrap();
        restore(&mut r, &mut t.animations).unwrap();
        assert_eq!(r.remaining(), 0);

        let mut plan = fresh();
        let after = plan_frame(gpu, &mut plan, &t, &plan_stores(&t), &tex);
        assert_eq!(after.prepared.mask_update, Some(MaskUpdate::Full));
        assert_eq!(after.hdr, before.hdr, "same image from the restored stores");
        assert_eq!(after.light, before.light);
    });
}

/// A visitor that keeps `path.name = value` lines.
#[derive(Default)]
struct Lines {
    path: Vec<String>,
    lines: Vec<String>,
}

impl knorke_observe::Visitor for Lines {
    fn value(&mut self, name: &str, value: knorke_observe::Value<'_>) {
        let mut p = self.path.join(".");
        if !p.is_empty() {
            p.push('.');
        }
        self.lines.push(format!("{p}{name} = {value:?}"));
    }
    fn begin_object(&mut self, name: &str) {
        self.path.push(name.to_owned());
    }
    fn begin_list(&mut self, name: &str) {
        self.path.push(name.to_owned());
    }
    fn end(&mut self) {
        self.path.pop();
    }
}

#[test]
fn pass_plan_inspects_its_stages_and_rejects_bad_frames() {
    use knorke_observe::Inspect;
    with_plan(|gpu, _, fresh| {
        let mut plan = fresh();
        let s = scene_zones_direction();
        let tex = PlanTextures::new(gpu, &s);
        let stores = plan_stores(&s);
        // Record without prepare.
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let e = plan
            .record(&mut cmd, &tex.frame(&s), &s.settings)
            .unwrap_err();
        assert_eq!(e.code, knorke_light2d::pass_plan::NOT_PREPARED);
        cmd.submit_and_wait().unwrap();
        // A buffer of the wrong size.
        let small = texture(gpu, 8, 8, &[0u8; 8 * 8 * 4]);
        let bad = PassFrame {
            glow: Some(small.texture_ref()),
            ..tex.frame(&s)
        };
        let e = plan.prepare(&bad, &stores, &s.settings).unwrap_err();
        assert_eq!(e.code, knorke_light2d::pass_plan::INVALID_FRAME);
        assert_eq!(plan.stats().failed, 1);

        plan_frame(gpu, &mut plan, &s, &stores, &tex);
        let mut v = Lines::default();
        plan.inspect(&mut v);
        let text = v.lines.join("\n");
        for line in [
            "frames = U64(1)",
            "failed = U64(1)",
            "last.zones = Bool(true)",
            "last.lights = Bool(true)",
            "last.occlusion = Bool(false)",
            "last.mask_update = Str(\"none\")",
            "last.detail_mode = Str(\"direction\")",
            "buffers.hdr.width = U64(96)",
            "buffers.light.width = U64(48)",
            "buffers.pure_ambient.height = U64(24)",
            "buffers.mask.width = U64(0)",
        ] {
            assert!(text.contains(line), "missing `{line}` in\n{text}");
        }
        assert!(plan.pipeline_count() > 0);
    });
}
