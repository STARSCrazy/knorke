//! Pattern obstacles on the GPU (P28) against C++ Chiaro: the occlusion mask and the light buffer
//! of three scenes through the pass plan, bit for bit (`tests/golden/chiaro/pattern/pattern_ref.cpp`
//! writes them). The scenes are mirrored there (`SceneHeight`, `SceneFlat`, `SceneRotated`);
//! change both together.
#![cfg(feature = "gpu")]

mod common;

use common::at;
use common::light_host::{L, L0, Scene};
use knorke_gpu::test_support::{
    compare_bytes, expected_bytes, golden_bytes, queue_download, read_download, rgba16f_diff,
    sampled_rgba8, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu};
use knorke_light2d::light::{Height, LightType};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, SceneStores};
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat, raw_id};
use knorke_light2d::{PatternId, PatternStore};
use knorke_math::{Vec2, Vec3};

/// iso_night_demo's `PatternSheet`: 20 x 16, slats x 0..3 (left half open), lattice x 4..19.
fn pattern_sheet() -> Vec<u8> {
    let (w, h) = (20, 16);
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let closed = if x < 4 {
                x >= 2
            } else {
                let dx = (x - 4) as f32 + 0.5 - 8.0;
                let dy = y as f32 + 0.5 - 8.0;
                ((dx.abs() + dy.abs()) - 7.0).abs() < 1.1
            };
            let i = (y * w + x) * 4;
            px[i..i + 4].copy_from_slice(&[255, 255, 255, if closed { 255 } else { 0 }]);
        }
    }
    px
}

/// The two patterns of the reference: slats repeating every 3, lattice 8 x 8.
fn create_patterns(store: &mut PatternStore) -> [PatternId; 2] {
    let px = pattern_sheet();
    let sheet = Sheet {
        width: 20,
        height: 16,
        pitch: 80,
        format: SheetFormat::Rgba8,
        pixels: &px,
    };
    let descs = [
        PatternDesc {
            repeat_width: 3.0,
            ..PatternDesc::new(0, 0, 4, 16)
        },
        PatternDesc {
            repeat_width: 8.0,
            repeat_height: 8.0,
            ..PatternDesc::new(4, 0, 16, 16)
        },
    ];
    let mut ids = Vec::new();
    store.create(&sheet, &descs, &mut ids).unwrap();
    [ids[0], ids[1]]
}

struct PatternScene {
    scene: Scene,
    patterns: PatternStore,
}

impl PatternScene {
    fn new(name: &'static str) -> Self {
        let mut scene = Scene::golden(name, Vec3::new(0.03, 0.03, 0.04), 1.0);
        scene.size = (96, 48);
        Self {
            scene,
            patterns: PatternStore::new(),
        }
    }

    fn shape(&mut self, shape: Shape<'_>, x: f32, y: f32, height: f32, pattern: PatternId) {
        self.rotated(shape, x, y, height, 0.0, pattern);
    }

    fn rotated(
        &mut self,
        shape: Shape<'_>,
        x: f32,
        y: f32,
        height: f32,
        rotation: f32,
        pattern: PatternId,
    ) {
        let common = ObstacleCommon {
            position: at(x, y),
            height,
            rotation,
            ..Default::default()
        };
        let o = &mut self.scene.obstacles;
        let id = o.create_shape(&ShapeDesc { common, shape }).unwrap();
        o.set_pattern(id, raw_id(pattern), &self.patterns).unwrap();
    }

    fn light(
        &mut self,
        kind: LightType,
        (x, y): (f32, f32),
        height: Height,
        radius: f32,
        direction: Vec3,
        intensity: f32,
    ) {
        self.scene.light(L {
            kind,
            x,
            y,
            height,
            radius,
            direction,
            intensity,
            inner: 25.0,
            outer: 45.0,
            shadows: true,
            ..L0
        });
    }
}

const MOON: Vec3 = Vec3::new(-0.7, 0.15, -0.7);

/// Finite heights: fence under the moon, tower under the floodlight.
fn scene_height() -> PatternScene {
    let mut s = PatternScene::new("height");
    let [slats, lattice] = create_patterns(&mut s.patterns);
    s.shape(
        Shape::Box {
            width: 1.0,
            depth: 36.0,
        },
        70.0,
        24.0,
        6.0,
        slats,
    );
    s.shape(
        Shape::Box {
            width: 8.0,
            depth: 8.0,
        },
        30.0,
        24.0,
        16.0,
        lattice,
    );
    s.light(
        LightType::Spot,
        (42.0, 24.0),
        Height::At(12.0),
        50.0,
        Vec3::new(-1.0, 0.0, -0.4),
        1.5,
    );
    s.light(
        LightType::Directional,
        (0.0, 0.0),
        Height::Auto,
        30.0,
        MOON,
        0.6,
    );
    s
}

/// Infinitely tall: column means; a circle and a tile grid with a pattern table.
fn scene_flat() -> PatternScene {
    let mut s = PatternScene::new("flat");
    let [slats, lattice] = create_patterns(&mut s.patterns);
    s.shape(
        Shape::Box {
            width: 1.0,
            depth: 36.0,
        },
        70.0,
        24.0,
        0.0,
        slats,
    );
    s.shape(
        Shape::Box {
            width: 8.0,
            depth: 8.0,
        },
        30.0,
        30.0,
        0.0,
        lattice,
    );
    s.shape(Shape::Circle { radius: 4.0 }, 52.0, 12.0, 0.0, lattice);
    let solid = [1u8, 1, 0, 1, 1, 1, 1, 0];
    let mut desc = TileGridDesc::new(4, 2, &solid);
    desc.common.position = at(12.0, 6.0);
    desc.cell_size = Vec2::new(3.0, 3.0);
    let o = &mut s.scene.obstacles;
    let grid = o.create_tile_grid(&desc).unwrap();
    o.set_pattern(grid, raw_id(lattice), &s.patterns).unwrap();
    o.set_tile_pattern_table(grid, &[raw_id(slats)], &s.patterns)
        .unwrap();
    o.set_tile_pattern_cells(
        grid,
        knorke_light2d::obstacle::CellRect {
            x0: 0,
            y0: 0,
            x1: 4,
            y1: 2,
        },
        &[0, 0, 0, 0, 1, 1, 1, 1],
    )
    .unwrap();
    s.light(
        LightType::Point,
        (46.0, 26.0),
        Height::Auto,
        60.0,
        Vec3::new(1.0, 0.0, 0.0),
        1.2,
    );
    s.light(
        LightType::Directional,
        (0.0, 0.0),
        Height::Auto,
        30.0,
        MOON,
        0.4,
    );
    s
}

/// Rotated pattern obstacles: a box and a polygon (Chiaro rotates in `double`, Knorke with
/// `det::sin_cos` in `f32`).
fn scene_rotated() -> PatternScene {
    let mut s = PatternScene::new("rotated");
    let [slats, lattice] = create_patterns(&mut s.patterns);
    s.rotated(
        Shape::Box {
            width: 10.0,
            depth: 6.0,
        },
        30.0,
        24.0,
        10.0,
        30.0,
        lattice,
    );
    let tri = [
        Vec2::new(-5.0, -8.0),
        Vec2::new(6.0, -2.0),
        Vec2::new(-1.0, 9.0),
    ];
    s.rotated(Shape::Polygon(&tri), 66.0, 22.0, 8.0, 17.0, slats);
    s.light(
        LightType::Point,
        (48.0, 26.0),
        Height::At(14.0),
        60.0,
        Vec3::new(1.0, 0.0, 0.0),
        1.2,
    );
    s.light(
        LightType::Directional,
        (0.0, 0.0),
        Height::Auto,
        30.0,
        MOON,
        0.4,
    );
    s
}

/// One frame through the plan: mask and light buffer bytes.
fn render(
    gpu: Gpu<'_>,
    plan: &mut PassPlan<'_>,
    s: &PatternScene,
    with_patterns: bool,
) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = s.scene.size;
    let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
    let frame = PassFrame::new(scene.texture_ref(), &s.scene.projection);
    let stores = SceneStores {
        patterns: with_patterns.then_some(&s.patterns),
        ..SceneStores::new(&s.scene.lights, &s.scene.obstacles)
    };
    plan.prepare(&frame, &stores, &s.scene.settings).unwrap();
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    plan.record(&mut cmd, &frame, &s.scene.settings).unwrap();
    let mask = queue_download(gpu, &mut cmd, plan.occlusion().mask().unwrap());
    let light = queue_download(gpu, &mut cmd, plan.lighting().light_buffer().unwrap());
    cmd.submit_and_wait().unwrap();
    (read_download(mask), read_download(light))
}

#[test]
fn pattern_obstacles_match_chiaro_bit_for_bit() {
    with_chiaro_pack(|gpu, pack| {
        let mut failures = Vec::new();
        for s in [scene_height(), scene_flat(), scene_rotated()] {
            let name = s.scene.name;
            let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
            let (mask, light) = render(gpu, &mut plan, &s, true);
            let (n, max) = compare_bytes(
                name,
                &mask,
                &golden_bytes("pattern", &format!("{name}.mask.bin")),
            );
            eprintln!("{name}: mask {n} differing bytes (max {max})");
            if n > 0 {
                failures.push(format!("{name}.mask: {n} bytes, max {max}"));
            }
            let (n, max) = rgba16f_diff(
                &light,
                &expected_bytes("pattern", &format!("{name}.light.bin"), &light),
            );
            eprintln!("{name}: light buffer {n} differing texels (max {max})");
            if n > 0 {
                failures.push(format!("{name}.light: {n} texels, max {max}"));
            }
            // The patterns matter: without them the same scene differs.
            let (_, plain) = render(
                gpu,
                &mut PassPlan::new(gpu, pack.to_vec()).unwrap(),
                &s,
                false,
            );
            assert_ne!(plain, light, "{name}: patterns change nothing");
        }
        assert!(failures.is_empty(), "differences from Chiaro: {failures:?}");
    });
}
