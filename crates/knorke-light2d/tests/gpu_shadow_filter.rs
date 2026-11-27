//! Shadow caster filter per light (E53) through the pass plan: port of Chiaro's
//! `tests/gpu/ShadowFilterTests.cpp` (chiaro @ 9bc5b9b). A dynamic-only light is shadowed by the
//! dynamic disc but not by the static wall, a static-only one the other way round; the caster
//! class masks exist only while a visible light uses a filter, and without one the frame is
//! bit-identical. Filtered lights and suns read the class masks through their rows and sun
//! maps. (The flag validation case of the C++ test has no counterpart: the
//! filter is one enum, both flags at once cannot be expressed.)
//!
//! Needs a Vulkan device and the shader pack of `cargo xtask shaders`; skips without them (a
//! failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use knorke_gpu::test_support::{
    half_to_f32, queue_download, read_download, sampled_rgba8, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu, Texture};
use knorke_light2d::codec::surface_pixel;
use knorke_light2d::light::{LightDesc, LightId, LightType, ShadowFilter};
use knorke_light2d::obstacle::{ObstacleCommon, ObstacleId, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassRecord, SceneStores};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3};

const W: u32 = 64;
const H: u32 = 32;
const BYTES: usize = (W * H * 4) as usize;

struct Fx<'d> {
    gpu: Gpu<'d>,
    plan: PassPlan<'d>,
    scene: Texture<'d>,
    surface: Texture<'d>,
    normals: bool,
    lights: LightStore,
    obstacles: ObstacleStore,
    disc: ObstacleId,
    projection: Projection,
    settings: LightSettings,
}

impl<'d> Fx<'d> {
    /// Upper half (y < 16): a static wall at x 20..22; lower half: a dynamic disc at (21, 24).
    /// `height` 0 is infinitely tall; directional shadows need a finite one.
    fn new(gpu: Gpu<'d>, pack: &[u8], height: f32) -> Self {
        let flat = surface_pixel(0.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        let mut obstacles = ObstacleStore::new();
        let common = ObstacleCommon {
            height,
            ..ObstacleCommon::default()
        };
        obstacles
            .create_shape(&ShapeDesc {
                common: ObstacleCommon {
                    position: At::new(Vec2::new(21.0, 8.0)),
                    ..common
                },
                shape: Shape::Box {
                    width: 2.0,
                    depth: 16.0,
                },
            })
            .unwrap();
        let disc = obstacles
            .create_shape(&ShapeDesc {
                common: ObstacleCommon {
                    position: At::new(Vec2::new(21.0, 24.0)),
                    dynamic: true,
                    ..common
                },
                shape: Shape::Circle { radius: 2.0 },
            })
            .unwrap();
        Self {
            disc,
            gpu,
            plan: PassPlan::new(gpu, pack.to_vec()).unwrap(),
            scene: sampled_rgba8(gpu, W, H, &vec![255u8; BYTES]),
            surface: sampled_rgba8(gpu, W, H, &flat.repeat((W * H) as usize)),
            normals: false,
            lights: LightStore::new(),
            obstacles,
            projection: Projection::default(),
            settings: LightSettings {
                ambient: Vec3::ZERO,
                ..LightSettings::default()
            },
        }
    }

    fn light(&mut self, filter: Option<ShadowFilter>, y: f32) -> LightId {
        let mut d = LightDesc::new(LightType::Point);
        d.position = At::new(Vec2::new(6.0, y));
        d.radius = 60.0;
        d.intensity = 0.5;
        if self.normals {
            d.height = knorke_light2d::light::Height::At(4.0);
        }
        d.cast_shadows = filter.is_some();
        d.shadow_filter = filter.unwrap_or_default();
        self.lights.create(&d, &NoResources).unwrap()
    }

    /// One frame: the record and the red channel of the HDR result.
    fn render(&mut self) -> (PassRecord, Vec<f32>) {
        let frame = PassFrame {
            surface: self.normals.then(|| self.surface.texture_ref()),
            ..PassFrame::new(self.scene.texture_ref(), &self.projection)
        };
        let stores = SceneStores::new(&self.lights, &self.obstacles);
        self.plan.prepare(&frame, &stores, &self.settings).unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        let record = self.plan.record(&mut cmd, &frame, &self.settings).unwrap();
        let hdr = queue_download(self.gpu, &mut cmd, self.plan.hdr().unwrap());
        cmd.submit_and_wait().unwrap();
        let red = read_download(hdr)
            .as_chunks::<8>()
            .0
            .iter()
            .map(|t| half_to_f32(u16::from_le_bytes([t[0], t[1]])))
            .collect();
        (record, red)
    }
}

fn red(img: &[f32], x: u32, y: u32) -> f32 {
    img[(y * W + x) as usize]
}

/// Shadowed: the probe behind the obstacle below `open / SHADOWED`. Chiaro uses 3 on 8-bit
/// display values; the HDR result here is linear, where the unshadowed falloff of a low light
/// over a surface buffer alone is about 5 (24 against 6 texels away), so the bound is stricter.
const SHADOWED: f32 = 20.0;

const FILTERS: [ShadowFilter; 3] = [
    ShadowFilter::All,
    ShadowFilter::DynamicOnly,
    ShadowFilter::StaticOnly,
];

#[test]
fn dynamic_only_shadows_the_disc_but_not_the_wall_static_only_the_reverse() {
    with_chiaro_pack(|gpu, pack| {
        for normals in [false, true] {
            for filter in FILTERS {
                // One light per frame: the other row's light would reach the probe past the
                // obstacle.
                let shadowed = |row: f32| {
                    let mut fx = Fx::new(gpu, pack, 0.0);
                    fx.normals = normals;
                    fx.light(Some(filter), row);
                    let (record, img) = fx.render();
                    let y = row as u32;
                    let (behind, open) = (red(&img, 30, y), red(&img, 12, y));
                    let masks = record.mask.caster_masks;
                    assert_eq!(masks, u32::from(filter != ShadowFilter::All));
                    eprintln!("{normals} {filter:?} row {row}: behind {behind}, open {open}");
                    behind < open / SHADOWED
                };
                let ctx = format!("normals {normals}, {filter:?}");
                assert_eq!(
                    shadowed(8.0),
                    filter != ShadowFilter::DynamicOnly,
                    "{ctx}: behind the static wall"
                );
                assert_eq!(
                    shadowed(24.0),
                    filter != ShadowFilter::StaticOnly,
                    "{ctx}: behind the dynamic disc"
                );
            }
        }
    });
}

#[test]
fn both_filters_in_one_frame_use_both_class_masks_and_a_directional_light_filters_too() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, 0.0);
        fx.light(Some(ShadowFilter::DynamicOnly), 8.0);
        fx.light(Some(ShadowFilter::StaticOnly), 24.0);
        let (record, img) = fx.render();
        assert_eq!(record.mask.caster_masks, 2);
        // The wall casts nothing for the dynamic-only light, the disc nothing for the static one.
        assert!(red(&img, 30, 8) > red(&img, 12, 8) / 3.0);
        assert!(red(&img, 30, 24) > red(&img, 12, 24) / 3.0);

        // A directional light from -x, dynamic-only: only the disc's shadow line.
        let mut sun = Fx::new(gpu, pack, 10.0); // shadow 10 / tan(20 deg) = 27 long
        sun.normals = true;
        let mut d = LightDesc::new(LightType::Directional);
        d.direction = Vec3::new(0.939_692_6, 0.0, -0.342_020_14); // 20 degrees above the horizon
        d.intensity = 1.0;
        d.cast_shadows = true;
        d.shadow_filter = ShadowFilter::DynamicOnly;
        sun.lights.create(&d, &NoResources).unwrap();
        let (record, lit) = sun.render();
        assert_eq!(record.mask.caster_masks, 1);
        let (wall, disc, open) = (red(&lit, 30, 8), red(&lit, 30, 24), red(&lit, 12, 4));
        assert!(wall == open, "behind wall {wall}, open {open}");
        assert!(disc < open / SHADOWED, "behind disc {disc}, open {open}");
    });
}

/// A static-only light (row light, sun map) draws what an unfiltered one draws without the
/// dynamic disc, bit for bit: the class mask and heights are the static obstacles' alone.
#[test]
fn a_static_only_light_equals_an_unfiltered_one_without_the_dynamic_disc() {
    with_chiaro_pack(|gpu, pack| {
        for sun in [false, true] {
            let mut images = Vec::new();
            for (filter, disc) in [(ShadowFilter::StaticOnly, true), (ShadowFilter::All, false)] {
                let mut fx = Fx::new(gpu, pack, 10.0);
                fx.normals = true;
                if !disc {
                    fx.obstacles.destroy(fx.disc).unwrap();
                }
                if sun {
                    let mut d = LightDesc::new(LightType::Directional);
                    d.direction = Vec3::new(0.939_692_6, 0.0, -0.342_020_14);
                    d.intensity = 1.0;
                    d.cast_shadows = true;
                    d.shadow_filter = filter;
                    fx.lights.create(&d, &NoResources).unwrap();
                } else {
                    fx.light(Some(filter), 16.0);
                }
                images.push(fx.render().1);
            }
            assert!(images[0] == images[1], "sun {sun}: the image differs");
        }
    });
}

#[test]
fn caster_masks_count_up_and_back_to_none_in_one_plan() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, 0.0);
        fx.light(None, 4.0);
        assert_eq!(fx.render().0.mask.caster_masks, 0); // no shadow caster: no occlusion
        let mut ids = Vec::new();
        for (n, filter) in FILTERS.into_iter().enumerate() {
            ids.push(fx.light(Some(filter), [8.0, 24.0, 28.0][n]));
            let (record, _) = fx.render();
            // The filtered ones draw their class masks.
            assert_eq!(record.mask.caster_masks, n as u32);
        }
        // The same frame again creates nothing.
        fx.render();
        assert_eq!(fx.plan.last_frame().unwrap().allocations, 0);
        for id in ids {
            fx.lights.destroy(id).unwrap();
        }
        let (record, _) = fx.render();
        assert_eq!(record.mask, Default::default());
    });
}

#[test]
fn without_a_visible_filtered_light_the_frame_is_bit_identical_with_the_same_draws() {
    with_chiaro_pack(|gpu, pack| {
        // A filtered light that is not visible (outside the scene) must not change anything.
        let mut out = Vec::new();
        for far in [false, true] {
            let mut fx = Fx::new(gpu, pack, 0.0);
            fx.light(Some(ShadowFilter::All), 8.0);
            fx.light(None, 24.0);
            if far {
                let mut d = LightDesc::new(LightType::Point);
                d.position = At::new(Vec2::new(1000.0, 1000.0));
                d.radius = 10.0;
                d.cast_shadows = true;
                d.shadow_filter = ShadowFilter::StaticOnly;
                fx.lights.create(&d, &NoResources).unwrap();
            }
            let (record, img) = fx.render();
            out.push((record, img, fx.plan.pipeline_count()));
        }
        assert!(out[0].1 == out[1].1, "the image changed");
        assert_eq!(out[0].0.mask, out[1].0.mask);
        assert_eq!(out[0].0.lighting, out[1].0.lighting);
        assert_eq!(out[0].2, out[1].2);
        assert_eq!(out[1].0.mask.caster_masks, 0);
    });
}
