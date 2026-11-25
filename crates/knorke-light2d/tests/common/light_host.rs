//! The host of the light-pass tests (`gpu_lighting_stage`, `gpu_atlas_zones`): the stages a frame
//! of Chiaro's `Context::Render` runs up to the light pass (occluders, occlusion mask, atlases,
//! zones, light pass), the scene description the dumpers share, and the readback of one frame.
//! The tests play the host (ADR 0002); the dumpers are `tests/golden/chiaro/{light,atlas}/*.cpp`.

use knorke_gpu::test_support::{
    golden_bytes, half_to_f32, queue_download, read_download, sampled_rgba8, with_chiaro_pack,
};
use knorke_gpu::{
    CommandBuffer, DownloadBuffer, Filter, Gpu, PipelineCache, Samplers, ShaderLibrary, Texture,
    TextureRef,
};
use knorke_light2d::ambient_zone::{AmbientZoneParams, make_stored_zone};
use knorke_light2d::ambient_zone_culling::{AmbientZoneList, CullScene, collect_ambient_zones};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::animation::AnimationStore;
use knorke_light2d::atlas::LightAtlases;
use knorke_light2d::light::{Height, LightDesc, LightType, MaskMode, ProfileId};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::light_pipelines::LightPipelines;
use knorke_light2d::light_program::{LIGHT_PROGRAMS, LightBase, LightProgram};
use knorke_light2d::lighting::{
    DetailMode, LightingFrame, LightingRecord, LightingResult, LightingStage, ShadowMaps,
};
use knorke_light2d::mask::{MaskPreset, MaskShape, build_pixel_mask};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::occluder_raster::{
    INFINITE_TOP, NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms, TileOccluders,
    thin_circle_radius,
};
use knorke_light2d::occlusion::{MaskPrograms, OcclusionMask};
use knorke_light2d::profile::{ProfilePreset, ProfileShape, build_shape_profile};
use knorke_light2d::settings::{LightSettings, Quality, Resolutions};
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdate};
use knorke_light2d::zone_pass::ZoneInput;
use knorke_light2d::{
    LightStore, MaskId, MaskStore, ObstacleStore, Projection, StoreResources,
    profile_store::ProfileStore,
};
use knorke_math::{Vec2, Vec3};

use super::{H, W, at, dome_material_pixels, programs};

/// A light of the dumper's `Light` struct (same defaults).
#[derive(Clone, Copy)]
pub(crate) struct L {
    pub(crate) kind: LightType,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) height: Height,
    pub(crate) radius: f32,
    pub(crate) color: Vec3,
    pub(crate) intensity: f32,
    pub(crate) source: f32,
    pub(crate) falloff: f32,
    pub(crate) shadows: bool,
    pub(crate) direction: Vec3,
    pub(crate) inner: f32,
    pub(crate) outer: f32,
    pub(crate) axis: Vec3,
    pub(crate) half_width: f32,
    pub(crate) profile: Option<ProfileId>,
    pub(crate) profile_rotation: f32,
    pub(crate) mask: Option<MaskId>,
    pub(crate) mask_angle: Option<f32>,
    /// Window mode: centre, half extents u and v.
    pub(crate) window: Option<[Vec3; 3]>,
}

pub(crate) const L0: L = L {
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
    profile: None,
    profile_rotation: 0.0,
    mask: None,
    mask_angle: None,
    window: None,
};

impl L {
    pub(crate) fn desc(&self) -> LightDesc {
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
pub(crate) struct Scene {
    pub(crate) name: &'static str,
    pub(crate) size: (u32, u32),
    pub(crate) lights: LightStore,
    pub(crate) obstacles: ObstacleStore,
    pub(crate) profiles: ProfileStore,
    pub(crate) masks: MaskStore,
    pub(crate) animations: AnimationStore,
    pub(crate) zones: AmbientZoneStore,
    pub(crate) settings: LightSettings,
    pub(crate) material: bool,
    /// The material buffer holds the dome normal map of `light_ref.cpp` instead of zeros (flat
    /// ground, as in Chiaro's tests).
    pub(crate) dome_material: bool,
    pub(crate) surface: bool,
    pub(crate) projection: Projection,
}

impl Scene {
    pub(crate) fn new(name: &'static str, ambient: f32, light_resolution: f32) -> Self {
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
            dome_material: false,
            surface: false,
            projection: Projection::IDENTITY,
        }
    }

    /// The 96 x 48 scenes of the dumper.
    pub(crate) fn golden(name: &'static str, ambient: Vec3, light_resolution: f32) -> Self {
        let mut s = Self::new(name, 0.0, light_resolution);
        s.settings.ambient = ambient;
        s.size = (W, H);
        s
    }

    pub(crate) fn light(&mut self, l: L) -> knorke_light2d::light::LightId {
        let res = StoreResources::new(&self.animations, &self.masks).with_profiles(&self.profiles);
        self.lights.create(&l.desc(), &res).unwrap()
    }

    pub(crate) fn lights(&mut self, ls: &[L]) -> &mut Self {
        for l in ls {
            self.light(*l);
        }
        self
    }

    pub(crate) fn polygon(&mut self, xy: &[f32], height: f32, opacity: f32) -> &mut Self {
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

    pub(crate) fn circle(&mut self, radius: f32, x: f32, y: f32, height: f32) -> &mut Self {
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
    pub(crate) fn profile(&mut self, preset: ProfilePreset, gains: [Option<f32>; 4]) -> ProfileId {
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

    pub(crate) fn mask_shape(&mut self, preset: MaskPreset, glass: Vec3) -> MaskId {
        let mut shape = MaskShape::preset(preset);
        shape.glass = glass;
        shape.validate().unwrap();
        let mut texels = Vec::new();
        shape.build(&mut texels);
        self.masks.create(texels).unwrap()
    }

    /// A gobo from RGBA pixels (sRGB, as `CHI_CreateMask` takes them).
    pub(crate) fn mask_pixels(&mut self, w: u32, h: u32, rgba: &[u8]) -> MaskId {
        let mut texels = Vec::new();
        build_pixel_mask(w, h, rgba, (w * 4) as usize, &mut texels).unwrap();
        self.masks.create(texels).unwrap()
    }

    pub(crate) fn zone(
        &mut self,
        p: &AmbientZoneParams,
    ) -> knorke_light2d::ambient_zone_store::AmbientZoneId {
        self.zones.create(make_stored_zone(p).unwrap()).unwrap()
    }

    /// Normal mode: the highest height a lit pixel can have (`Context::BuildFrame`).
    pub(crate) fn max_height(&self) -> f32 {
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

/// An atlas texture read back: width, height, bytes.
pub(crate) type AtlasDump = (u32, u32, Vec<u8>);

/// What one frame left behind.
pub(crate) struct Out {
    pub(crate) result: LightingResult,
    pub(crate) record: LightingRecord,
    pub(crate) light: Vec<u8>,
    pub(crate) light_size: (u32, u32),
    pub(crate) second: Vec<u8>,
    /// The pure ambient target, if the zone pass wrote one.
    pub(crate) ambient: Vec<u8>,
    pub(crate) programs: Vec<LightProgram>,
    pub(crate) profile_atlas: Option<AtlasDump>,
    pub(crate) mask_atlas: Option<AtlasDump>,
    /// GPU objects alive after the frame.
    pub(crate) live: u32,
}

impl Out {
    /// Channel `c` of light buffer pixel (x, y).
    pub(crate) fn px(&self, x: u32, y: u32, c: usize) -> f32 {
        let i = ((y * self.light_size.0 + x) * 8) as usize + c * 2;
        half_to_f32(u16::from_le_bytes([self.light[i], self.light[i + 1]]))
    }

    /// Red channel of light buffer pixel (x, y).
    pub(crate) fn red(&self, x: u32, y: u32) -> f32 {
        self.px(x, y, 0)
    }
}

pub(crate) struct Host<'d> {
    pub(crate) gpu: Gpu<'d>,
    pub(crate) shaders: ShaderLibrary<'d>,
    pub(crate) pipelines: PipelineCache<'d>,
    pub(crate) light_pipelines: LightPipelines<'d>,
    pub(crate) samplers: Samplers<'d>,
    pub(crate) geometry: OccluderGeometry<'d>,
    pub(crate) tiles: TileOccluders<'d>,
    pub(crate) occlusion: OcclusionMask<'d>,
    pub(crate) lighting: LightingStage<'d>,
    pub(crate) atlases: LightAtlases<'d>,
    pub(crate) list: LightList,
    pub(crate) zone_list: AmbientZoneList,
}

/// Chiaro `AnyShadowCaster` (OcclusionStage.cpp).
pub(crate) fn any_shadow_caster(l: &LightList) -> bool {
    !(l.shadowed.is_empty()
        && l.shadowed_area.is_empty()
        && l.masked_shadowed.is_empty()
        && l.directional_shadows == 0
        && l.caster_classes == 0)
}

impl<'d> Host<'d> {
    pub(crate) fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &programs()).unwrap();
        let light_pipelines = LightPipelines::new(&shaders).unwrap();
        Self {
            gpu,
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

    /// One frame as Chiaro's `Context::Render` runs it up to the light pass, `time` seconds.
    pub(crate) fn frame(&mut self, s: &Scene, time: f64) -> Out {
        let (w, h) = s.size;
        let scene_tex = sampled_rgba8(self.gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        // Empty (all zero) buffers: normal mode on flat ground, as in Chiaro's tests.
        let zeros = vec![0u8; (w * h * 4) as usize];
        let material_tex = s.material.then(|| {
            let pixels = if s.dome_material {
                dome_material_pixels(w, h)
            } else {
                zeros.clone()
            };
            sampled_rgba8(self.gpu, w, h, &pixels)
        });
        let surface_tex = s.surface.then(|| sampled_rgba8(self.gpu, w, h, &zeros));
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
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
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
            patterns: None,
        });
        // The atlases the frame needs, uploaded before the light pass reads them.
        let mut atlas_created = self
            .atlases
            .prepare(&self.list, Some(&s.profiles), Some(&s.masks))
            .unwrap();
        self.atlases.record(&mut cmd).unwrap();
        let frame = LightingFrame {
            scene: scene_tex.texture_ref(),
            surface: surface_tex.as_ref().map(Texture::texture_ref),
            material: material_tex.as_ref().map(Texture::texture_ref),
            projection: &s.projection,
            settings: &s.settings,
            max_height,
            lights: &self.list,
            shadows: maps.as_ref(),
            profile_atlas: self.atlases.profiles.texture(),
            mask_atlas: self.atlases.masks.texture(),
            ring_lights: &[],
            medium_map: None,
            medium_runs: false,
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
        let light = light_tex.map(|t| queue_download(self.gpu, &mut cmd, t));
        let second = match result.detail_mode {
            DetailMode::Exact => self.lighting.specular_buffer(),
            DetailMode::Direction => self.lighting.direction_buffer(),
            DetailMode::None => None,
        }
        .map(|t| queue_download(self.gpu, &mut cmd, t));
        let ambient = self
            .lighting
            .pure_ambient()
            .map(|t| queue_download(self.gpu, &mut cmd, t));
        let atlas = |t: Option<TextureRef<'_>>, cmd: &mut CommandBuffer<'_>| {
            t.map(|t| (t.width(), t.height(), queue_download(self.gpu, cmd, t)))
        };
        let profile_atlas = atlas(self.atlases.profiles.texture(), &mut cmd);
        let mask_atlas = atlas(self.atlases.masks.texture(), &mut cmd);
        cmd.submit_and_wait().unwrap();
        let read_atlas =
            |a: Option<(u32, u32, DownloadBuffer<'_>)>| a.map(|(w, h, d)| (w, h, read_download(d)));
        Out {
            result,
            record,
            light: light.map_or_else(Vec::new, read_download),
            light_size,
            second: second.map_or_else(Vec::new, read_download),
            ambient: ambient.map_or_else(Vec::new, read_download),
            programs,
            profile_atlas: read_atlas(profile_atlas),
            mask_atlas: read_atlas(mask_atlas),
            live: self.gpu.counter().live(),
        }
    }
}

/// A fresh host per call; the counter is the one of all GPU objects of the device.
/// A host per call of the closure; every host of the test shares the device and the leak check.
pub(crate) type Fresh<'a, 'd> = &'a mut dyn FnMut() -> Host<'d>;

pub(crate) fn with_host(body: impl for<'d> FnOnce(Fresh<'_, 'd>)) {
    with_chiaro_pack(|gpu, pack| body(&mut || Host::new(gpu, pack)));
}

/// The programs a scene used according to `programs.txt` (`base:features`).
pub(crate) fn golden_programs(dir: &str, name: &str) -> Vec<LightProgram> {
    let text = String::from_utf8(golden_bytes(dir, "programs.txt")).unwrap();
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
