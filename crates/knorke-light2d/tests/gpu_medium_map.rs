//! The medium world map (P29) against C++ Chiaro: the map and the light buffer (`*_medium`
//! programs) of two scenes through the pass plan, bit for bit
//! (`tests/golden/chiaro/medium/medium_ref.cpp` writes them). The scenes are mirrored there
//! (`SceneTop`, `SceneIso`, `MediumLayer`); change both together.
#![cfg(feature = "gpu")]

mod common;

use common::at;
use common::light_host::{L, L0, Scene};
use knorke_gpu::test_support::{
    golden_bytes, queue_download, read_download, rgba16f_diff, sampled_rgba8, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu};
use knorke_light2d::light::{Height, LightType};
use knorke_light2d::light_program::LightFeature;
use knorke_light2d::lighting::DrawProgram;
use knorke_light2d::medium_map::MEDIUM_LAYER_FORMAT;
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassStages, SceneStores};
use knorke_light2d::settings::{Quality, Resolutions};
use knorke_light2d::{LightStore, Projection};
use knorke_math::{Vec2, Vec3};

const MOON: Vec3 = Vec3::new(-0.7, 0.15, -0.7);

/// `MediumLayer` of the reference: kind 0 from 2 to 12 steps around (30, 20), kind 3 from 10 to
/// 30 steps around (66, 26).
fn medium_layer(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            let a = (x - 30) * (x - 30) + (y - 20) * (y - 20);
            let b = (x - 66) * (x - 66) + (y - 26) * (y - 26);
            if a < 196 {
                px[i..i + 4].copy_from_slice(&[12, 255 - 2, 0, (255 - a * 255 / 196) as u8]);
            } else if b < 100 {
                px[i..i + 4].copy_from_slice(&[30, 255 - 10, 3 * 16, (200 - b * 2) as u8]);
            }
        }
    }
    px
}

fn light(kind: LightType, (x, y): (f32, f32), height: Height, radius: f32) -> L {
    L {
        kind,
        x,
        y,
        height,
        radius,
        inner: 25.0,
        outer: 45.0,
        ..L0
    }
}

/// Identity projection, no obstacles, unshadowed lights.
fn scene_top() -> Scene {
    let mut s = Scene::golden("top", Vec3::new(0.03, 0.03, 0.04), 1.0);
    let point = light(LightType::Point, (48.0, 24.0), Height::At(20.0), 60.0);
    s.light(L {
        intensity: 1.5,
        ..point
    });
    let sun = light(LightType::Directional, (0.0, 0.0), Height::Auto, 30.0);
    s.light(L {
        direction: MOON,
        intensity: 0.5,
        ..sun
    });
    s
}

/// Isometric with a fractional origin, shadow resolution 0.5, a box, shadowed spot and
/// directional light.
fn scene_iso() -> Scene {
    let mut s = Scene::golden("iso", Vec3::new(0.03, 0.03, 0.04), 1.0);
    s.projection = Projection {
        origin: knorke_math::At::new(Vec2::new(48.3, 10.6)),
        x_axis: Vec2::new(1.6, 0.8),
        y_axis: Vec2::new(-1.6, 0.8),
        z_axis: Vec2::new(0.0, -1.2),
    };
    s.settings.quality.quality = Quality::Custom(Resolutions {
        light: 1.0,
        shadow: 0.5,
        medium: 0.5,
    });
    s.obstacles
        .create_shape(&ShapeDesc {
            common: ObstacleCommon {
                position: at(14.0, 8.0),
                height: 8.0,
                ..Default::default()
            },
            shape: Shape::Box {
                width: 4.0,
                depth: 4.0,
            },
        })
        .unwrap();
    let spot = light(LightType::Spot, (8.0, 16.0), Height::At(12.0), 40.0);
    s.light(L {
        direction: Vec3::new(1.0, -0.2, -0.4),
        intensity: 1.5,
        shadows: true,
        ..spot
    });
    let sun = light(LightType::Directional, (0.0, 0.0), Height::Auto, 30.0);
    s.light(L {
        direction: MOON,
        intensity: 0.5,
        shadows: true,
        ..sun
    });
    s
}

/// One frame through the plan: map (empty without one) and light buffer bytes.
fn render(gpu: Gpu<'_>, plan: &mut PassPlan<'_>, s: &Scene, medium: bool) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = s.size;
    let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
    let layer = sampled_rgba8(gpu, w, h, &medium_layer(w, h));
    let frame = PassFrame {
        medium: medium.then(|| layer.texture_ref()),
        ..PassFrame::new(scene.texture_ref(), &s.projection)
    };
    let stores = SceneStores::new(&s.lights, &s.obstacles);
    let prepared = plan.prepare(&frame, &stores, &s.settings).unwrap();
    assert_eq!(prepared.stages.contains(PassStages::MEDIUM_MAP), medium);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    plan.record(&mut cmd, &frame, &s.settings).unwrap();
    let map = plan
        .medium_map()
        .view()
        .map(|v| queue_download(gpu, &mut cmd, v.map));
    let light = queue_download(gpu, &mut cmd, plan.lighting().light_buffer().unwrap());
    cmd.submit_and_wait().unwrap();
    (
        map.map(read_download).unwrap_or_default(),
        read_download(light),
    )
}

#[test]
fn medium_map_matches_chiaro_bit_for_bit() {
    with_chiaro_pack(|gpu, pack| {
        let mut failures = Vec::new();
        for s in [scene_top(), scene_iso()] {
            let name = s.name;
            let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
            let (map, light) = render(gpu, &mut plan, &s, true);
            // Every draw reads the map, the directional light per pixel.
            let lit = plan.lighting();
            let point = if name == "top" {
                DrawProgram::Unshadowed
            } else {
                DrawProgram::Shadowed
            };
            assert!(
                lit.program(point).unwrap().has(LightFeature::Medium),
                "{name}"
            );
            assert!(
                lit.directional_program().unwrap().has(LightFeature::Medium),
                "{name}"
            );
            for (what, got, file) in [("map", &map, "map"), ("light", &light, "light")] {
                let (n, max) =
                    rgba16f_diff(got, &golden_bytes("medium", &format!("{name}.{file}.bin")));
                eprintln!("{name}: {what} {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.{what}: {n} texels, max {max}"));
                }
            }
            // The map matters: without the layer the same lights are brighter.
            let (none, plain) = render(
                gpu,
                &mut PassPlan::new(gpu, pack.to_vec()).unwrap(),
                &s,
                false,
            );
            assert!(none.is_empty());
            assert_ne!(plain, light, "{name}: the medium changes nothing");
        }
        assert!(failures.is_empty(), "differences from Chiaro: {failures:?}");
    });
}

#[test]
fn a_frame_without_medium_or_lights_builds_nothing() {
    with_chiaro_pack(|gpu, pack| {
        let s = scene_top();
        let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
        render(gpu, &mut plan, &s, true);
        assert_eq!(plan.medium_map().pipeline_count(), 3);
        // The next frame without the layer releases the map and draws plain programs.
        render(gpu, &mut plan, &s, false);
        assert!(plan.medium_map().view().is_none());
        assert!(plan.medium_map().layout().is_none());
        let lit = plan.lighting();
        assert!(
            !lit.program(DrawProgram::Unshadowed)
                .unwrap()
                .has(LightFeature::Medium)
        );
        // A layer without any light in the store builds no map either.
        let (w, h) = s.size;
        let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        let layer = sampled_rgba8(gpu, w, h, &medium_layer(w, h));
        let frame = PassFrame {
            medium: Some(layer.texture_ref()),
            ..PassFrame::new(scene.texture_ref(), &s.projection)
        };
        let lights = LightStore::new();
        let prepared = plan
            .prepare(
                &frame,
                &SceneStores::new(&lights, &s.obstacles),
                &s.settings,
            )
            .unwrap();
        assert!(!prepared.stages.contains(PassStages::MEDIUM_MAP));
        assert!(plan.medium_map().view().is_none());
    });
}

#[test]
fn a_medium_layer_of_another_format_is_refused() {
    with_chiaro_pack(|gpu, pack| {
        let s = scene_top();
        let (w, h) = s.size;
        let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        let layer = knorke_gpu::test_support::sampled(
            gpu,
            w,
            h,
            knorke_gpu::sys::gpu::SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
            &medium_layer(w, h),
        );
        let frame = PassFrame {
            medium: Some(layer.texture_ref()),
            ..PassFrame::new(scene.texture_ref(), &s.projection)
        };
        let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
        let e = plan
            .prepare(
                &frame,
                &SceneStores::new(&s.lights, &s.obstacles),
                &s.settings,
            )
            .unwrap_err();
        assert_eq!(e.code, MEDIUM_LAYER_FORMAT);
    });
}
