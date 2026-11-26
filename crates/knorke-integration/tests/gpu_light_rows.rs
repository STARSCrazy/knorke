//! Light rows (ADR 0019, plan 17.13) in the pass plan, on the Chiaro demo scene `iso_night`:
//! what `LightSettings::light_rows` changes and what it must not change, and that a lamp's row
//! is built only when the lamp or the mask around it changed.
//!
//! The rows against the reference picture and their frame times are measured by
//! `gpu_lightrow_probe.rs`.
//!
//! The tests play the host (ADR 0002). They skip without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

#[path = "common/demo_scene.rs"]
mod demo_scene;

#[path = "common/iso_night.rs"]
mod iso_night;

use demo_scene::{Demo, DemoFrames, H, Scene, W};
use knorke_gpu::CommandBuffer;
use knorke_gpu::test_support::with_chiaro_pack;
use knorke_integration::layers::{Layer, LayerSet};
use knorke_integration::light2d::Light2dSettings;
use knorke_light2d::light::LightType;
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::settings::{Quality, Resolutions};
use knorke_math::{At, Vec2};

/// Largest channel difference of two pictures, the mean over all channels and the share of
/// pixels that differ, in percent.
fn diff(a: &[u8], b: &[u8]) -> (u8, f64, f64) {
    assert_eq!(a.len(), b.len());
    let (mut max, mut sum, mut pixels) = (0, 0u64, 0u64);
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = (0..4).map(|i| p[i].abs_diff(q[i]));
        max = max.max(d.clone().max().unwrap());
        sum += d.clone().map(u64::from).sum::<u64>();
        pixels += u64::from(p != q);
    }
    let n = (a.len() / 4) as f64;
    (max, sum as f64 / (4.0 * n), 100.0 * pixels as f64 / n)
}

/// The demo's settings with light rows.
fn with_rows(scene: &Scene) -> Light2dSettings {
    let mut s = iso_night::settings(scene);
    s.light.light_rows = true;
    s
}

/// Makes the demo's two pattern obstacles opaque: they keep the lamps reaching them from the
/// rows. Returns them.
fn drop_patterns(scene: &mut Scene) -> Vec<knorke_light2d::ObstacleId> {
    let ids: Vec<_> = (scene.obstacles.iter())
        .filter(|(_, o)| o.patterns.refs > 0)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids.len(), 2, "the demo's pattern obstacles");
    for &id in &ids {
        scene.obstacles.set_pattern(id, 0, &scene.patterns).unwrap();
    }
    ids
}

/// The obstacles `ids` with this opacity, or switched off.
fn set_walls(scene: &mut Scene, ids: &[knorke_light2d::ObstacleId], opacity: f32, enabled: bool) {
    for &id in ids {
        let (_, o) = scene.obstacles.iter().find(|(i, _)| *i == id).unwrap();
        let common = ObstacleCommon {
            opacity,
            enabled,
            ..o.common
        };
        let outline = scene.obstacles.outline(id).to_vec();
        let shape = Shape::Polygon(&outline);
        scene
            .obstacles
            .set_shape(id, &ShapeDesc { common, shape })
            .unwrap();
    }
}

/// The frame with and without light rows: both pictures and the row lights drawn.
fn on_and_off(f: &mut DemoFrames<'_, '_>) -> (Vec<u8>, Vec<u8>, u32) {
    f.settings.light.light_rows = false;
    let (off, record) = f.frame();
    assert_eq!(record.lighting.row_lights, 0, "rows off");
    f.settings.light.light_rows = true;
    let (on, record) = f.frame();
    (on, off, record.lighting.row_lights)
}

/// `plain` (infinitely tall obstacles, soft shadows, a cleared surface layer): the lamps away
/// from the fences are row lights, and the picture differs along the shadow edges only.
#[test]
fn the_plain_variant_draws_row_lights() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("plain", 45, "", |demo| {
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                let (_, off, _) = on_and_off(f);
                let (on, record) = f.frame();
                let (rows, drawn) = (record.lighting.row_lights, record.lighting.lights_drawn);
                let (max, mean, share) = diff(&on, &off);
                eprintln!(
                    "iso_night_plain: {rows} of {drawn} lights are row lights; against rows off: \
                     largest difference {max}/255, mean {mean:.4}, {share:.2} % of the pixels"
                );
                assert!(rows > 0);
                assert!(mean < 1.0 && share < 10.0, "mean {mean}, {share} %");
            });
        });
    });
}

/// Without obstacles a row light is Chiaro's light: the same falloff, cone, profile, Lambert
/// term, highlight, wet ground and direction sample into the same targets, in all three material
/// modes. Next to Chiaro's lights only the order of the additive draws differs (the row lights
/// come last), which moves a 16-bit float sum by a rounding; with the row lights alone the
/// order is the same and so is every bit.
#[test]
fn row_lights_equal_chiaro_lights_without_obstacles() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            let demo = Demo {
                kinds: demo.kinds.with(Layer::Material),
                ..demo
            };
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                let ids: Vec<_> = f.scene.obstacles.iter().map(|(id, _)| id).collect();
                for id in ids {
                    f.scene.obstacles.destroy(id).unwrap();
                }
                // Any bytes are a material: detail normals around "up", every exponent and
                // specular strength.
                let material: Vec<u8> = (0..W * H)
                    .flat_map(|i| {
                        let (x, y) = (i % W, i / W);
                        [
                            98 + ((x * 7 + y * 3) % 61) as u8,
                            98 + ((x * 5 + y * 11) % 61) as u8,
                            ((x * 13 + y * 7) % 256) as u8,
                            ((x + 3 * y) % 256) as u8,
                        ]
                    })
                    .collect();
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                let fills = [(Layer::Material, &material[..])];
                let uploads = scene_common::upload_layers(gpu, &mut cmd, f.layers, &fills);
                cmd.submit_and_wait().unwrap();
                drop(uploads);
                f.settings.light.material.wetness = 0.6;

                let spots = (f.scene.lights.iter())
                    .filter(|(_, l)| l.desc.kind == LightType::Spot)
                    .count();
                assert!(spots > 0);
                let modes = [
                    ("exact", 1.0, LayerSet::NONE),
                    ("direction", 0.5, LayerSet::NONE),
                    ("none", 1.0, LayerSet::of(Layer::Material)),
                ];
                for (alone, (mode, light, skip)) in [false, true]
                    .into_iter()
                    .flat_map(|a| modes.map(|m| (a, m)))
                {
                    if alone {
                        // Idempotent: the second and third mode find them gone.
                        for &id in &f.scene.light_ids {
                            let other = f.scene.lights.get(id).is_ok_and(|d| {
                                d.kind == LightType::Directional || d.mask.is_some()
                            });
                            if other {
                                f.scene.lights.destroy(id).unwrap();
                            }
                        }
                    }
                    f.settings.light.quality.quality = Quality::Custom(Resolutions {
                        light,
                        shadow: 0.5,
                        medium: 0.5,
                    });
                    f.skip = skip;
                    let (on, off, rows) = on_and_off(f);
                    let (max, mean, share) = diff(&on, &off);
                    eprintln!(
                        "no obstacles, row lights alone {alone}, material mode {mode}: {rows} \
                         row lights, largest difference {max}/255, mean {mean:.5}, {share:.3} % \
                         of the pixels"
                    );
                    let lights = &f.renderer.plan().lights().rows;
                    let profiled = (lights.iter())
                        .filter(|l| !l.is_area() && l.light.profile[0] >= 0.0)
                        .count();
                    let area = lights.iter().filter(|l| l.is_area()).count();
                    assert!(rows > 0 && profiled > 0, "{profiled} profiled row lights");
                    assert_eq!(area, 8, "the demo's line and rect lights");
                    // The direction sums are divided in the composite, which doubles a rounding.
                    let bound = if alone { 0 } else { 2 };
                    assert!(max <= bound, "{mode}: {max}/255");
                }
            });
        });
    });
}

/// The whole demo with opaque obstacles: row lights and Chiaro's lights (profiles, masks, area
/// lights, the sun, zones) in one frame and the same targets.
#[test]
fn row_lights_and_chiaro_lights_share_a_frame() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                drop_patterns(&mut f.scene);
                f.settings.light.light_rows = false;
                let (off, without) = f.frame();
                f.settings.light.light_rows = true;
                let (on, with) = f.frame();
                let (max, mean, share) = diff(&on, &off);
                let l = with.lighting;
                eprintln!(
                    "iso_night_hard, opaque: {} of {} lights are row lights, {} rows built; \
                     against rows off: largest difference {max}/255, mean {mean:.4}, \
                     {share:.2} % of the pixels",
                    l.row_lights, l.lights_drawn, l.rows_built
                );
                assert_eq!(l.lights_drawn, without.lighting.lights_drawn);
                assert!(l.row_lights > 0 && l.row_lights < l.lights_drawn);
                assert!(l.rows_built > 0 && l.rows_built <= l.row_lights);
                // Hard shadows of two methods: they differ along the shadow edges only.
                assert!(mean < 1.0 && share < 10.0, "mean {mean}, {share} %");
            });
        });
    });
}

/// The demo as it is: its two pattern fences are runs of the row whose crossings the pattern pass
/// finds, so the lamps that reach them are row lights too.
#[test]
fn pattern_obstacles_are_runs_of_the_row() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                f.settings.light.light_rows = false;
                let (off, without) = f.frame();
                f.settings.light.light_rows = true;
                let (on, with) = f.frame();
                let (max, mean, share) = diff(&on, &off);
                let (rows, drawn) = (with.lighting.row_lights, with.lighting.lights_drawn);
                assert_eq!(drawn, without.lighting.lights_drawn);
                eprintln!(
                    "iso_night_hard: {rows} of {drawn} lights are row lights; against rows off: \
                     largest difference {max}/255, mean {mean:.4}, {share:.2} % of the pixels"
                );
                // All but the sun, which is no row light.
                assert_eq!(rows + 1, drawn);
                assert!(mean < 1.0 && share < 10.0, "mean {mean}, {share} %");

                // Every shadowed lamp whose disc holds a corner of a fence triangle.
                let mut corners = Vec::new();
                for (id, o) in f.scene.obstacles.iter() {
                    if o.patterns.refs > 0 {
                        f.scene.obstacles.append_world_triangles(id, &mut corners);
                    }
                }
                let near: Vec<u32> = (f.scene.light_ids.iter())
                    .filter(|&&id| {
                        let d = f.scene.lights.get(id).unwrap();
                        d.cast_shadows
                            && corners
                                .iter()
                                .any(|c| (c.raw() - d.position.raw()).length() < d.radius)
                    })
                    .map(|id| id.index())
                    .collect();
                assert!(!near.is_empty(), "a lamp lights a fence");
                let row_keys: Vec<u32> = (f.renderer.plan().lights().rows.iter())
                    .map(|l| (l.key >> 2) as u32)
                    .collect();
                // Five lamps at the fences; the sun's disc holds them too, and one lamp is
                // out of the picture.
                assert!(
                    near.iter().filter(|k| row_keys.contains(k)).count() >= 5,
                    "lamps at the fences {near:?}, row lights {row_keys:?}"
                );
            });
        });
    });
}

/// The demo's fences as walls of 50 % opacity without pattern: a partly transparent run of the
/// row, so every lamp that is a row light with opaque fences is one now. Behind the walls the
/// picture lies between the frames with opaque walls and without them, near Chiaro's.
#[test]
fn half_transparent_walls_are_runs_of_the_row() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                let walls = drop_patterns(&mut f.scene);
                // Each way against itself with opaque walls and without: the two ways' hard
                // edges differ in their own right (`row_lights_and_chiaro_lights_share_a_frame`).
                let mut ends = Vec::new();
                for (opacity, enabled) in [(1.0, false), (1.0, true)] {
                    set_walls(&mut f.scene, &walls, opacity, enabled);
                    ends.push(on_and_off(f));
                }
                set_walls(&mut f.scene, &walls, 0.5, true);
                let (on, off, rows) = on_and_off(f);
                let (max, mean, share) = diff(&on, &off);

                // The walls' shadow: pixels the opaque walls darken by 48 or more (channel sum)
                // with Chiaro's light.
                let sum = |p: &[u8], i: usize| -> u32 {
                    p[4 * i..4 * i + 3].iter().map(|&c| u32::from(c)).sum()
                };
                let (lit, dark) = (&ends[0].1, &ends[1].1);
                let shadow: Vec<usize> = (0..lit.len() / 4)
                    .filter(|&i| sum(lit, i) >= sum(dark, i) + 48)
                    .collect();
                let at = |p: &[u8]| {
                    let total: f64 = shadow.iter().map(|&i| f64::from(sum(p, i))).sum();
                    total / shadow.len() as f64
                };
                // How far the picture lies from opaque walls (0) to none (1).
                let way =
                    |p: &[u8], lit: &[u8], dark: &[u8]| (at(p) - at(dark)) / (at(lit) - at(dark));
                let chiaro = way(&off, lit, dark);
                let row = way(&on, &ends[0].0, &ends[1].0);
                eprintln!(
                    "iso_night_hard, 50 % walls: {rows} row lights; against rows off: largest \
                     difference {max}/255, mean {mean:.4}, {share:.2} % of the pixels; in the \
                     walls' shadow ({} pixels) rows off {chiaro:.4}, rows on {row:.4} of the way \
                     from opaque walls to none",
                    shadow.len()
                );
                assert_eq!(rows, 33, "the row lights with opaque fences");
                assert!(mean < 1.0 && share < 10.0, "mean {mean}, {share} %");
                assert!(shadow.len() > 500, "{} pixels", shadow.len());
                assert!(row > 0.2 && row < 0.8, "neither dark nor lit: {row}");
                // Measured 0.546 against Chiaro's 0.619: Chiaro lets more through, and the probe
                // (`gpu_lightrow_probe.rs`, "hart halb") finds the rows closer to the exact march
                // there than Chiaro's light, so the bound holds that gap with some room only.
                assert!((row - chiaro).abs() < 0.1, "rows {row}, Chiaro {chiaro}");
            });
        });
    });
}

/// A row is built when its lamp or the mask in its reach changed, and not otherwise; whatever
/// was kept, the picture is that of a plan that builds every row.
#[test]
fn rows_are_built_only_when_lamp_or_mask_change() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &with_rows, &mut |f| {
                drop_patterns(&mut f.scene);
                let check = |f: &mut DemoFrames<'_, '_>, what: &str| {
                    let (pixels, record) = f.frame();
                    let built = record.lighting.rows_built;
                    let (max, _, share) = diff(&pixels, &f.fresh_frame(pack));
                    eprintln!(
                        "{what}: {built} rows built; against a new plan: largest difference \
                         {max}/255, {share:.3} % of the pixels"
                    );
                    (built, max)
                };

                let (all, max) = check(f, "first frame");
                assert!(all > 1 && max == 0);
                assert_eq!(check(f, "resting"), (0, 0));
                assert_eq!(f.renderer.plan().stats().rows_built, u64::from(all));

                // One lamp, a third of a world unit.
                let (id, at) = (f.scene.light_ids.iter())
                    .find_map(|&id| {
                        let d = f.scene.lights.get(id).unwrap();
                        let row = d.kind == LightType::Point && d.cast_shadows;
                        (row && d.profile.is_none() && d.mask.is_none())
                            .then_some((id, (d.position, d.height)))
                    })
                    .unwrap();
                let moved = At::new(at.0.raw() + Vec2::new(0.3, 0.0));
                f.scene.lights.set_position(id, moved, at.1).unwrap();
                assert_eq!(check(f, "one lamp moved"), (1, 0));
                assert_eq!(check(f, "resting"), (0, 0));

                // One dynamic obstacle (a static one redraws the whole mask): the lamps that
                // reach its old or new place.
                let (id, pose) = (f.scene.obstacles.iter())
                    .filter(|(_, o)| o.common.dynamic)
                    .map(|(id, o)| (id, (o.common.position, o.common.rotation)))
                    .next()
                    .unwrap();
                let moved = At::new(pose.0.raw() + Vec2::new(0.5, 0.0));
                f.scene.obstacles.set_transform(id, moved, pose.1).unwrap();
                let (built, max) = check(f, "one dynamic obstacle moved");
                assert!(built >= 1 && built < all && max == 0, "{built} of {all}");
                assert_eq!(check(f, "resting"), (0, 0));

                // The camera, half a mask texel: the mask stays where it is in the world.
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(1.0, 0.0));
                assert_eq!(check(f, "camera moved half a mask texel"), (0, 0));
                // Twelve whole texels: only lamps reaching over the mask's rim see new mask.
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(23.0, 8.0));
                let (built, max) = check(f, "camera moved 12 x 4 mask texels");
                assert!(built < all && max == 0, "{built} of {all}");
                assert_eq!(check(f, "resting"), (0, 0));

                f.renderer.plan_mut().invalidate();
                assert_eq!(check(f, "invalidated"), (all, 0));
            });
        });
    });
}
