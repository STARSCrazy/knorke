//! Shared by the GPU test binaries of knorke-light2d (each binary includes it with
//! `mod common;`, so items one binary does not use are fine).
#![allow(dead_code)]

pub(crate) mod light_host;

use std::collections::BTreeMap;

use knorke_gpu::ProgramDesc;
use knorke_gpu::kpak::{Program, Stage};
use knorke_light2d::codec::{AlphaSource, Material, NormalMapOptions, convert_normal_map};
use knorke_light2d::light_program::{LightLayout, LightSampler, LightStorage, LightUniform};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::occluder_raster::OCCLUDER_PROGRAMS;
use knorke_light2d::occlusion::MASK_PROGRAMS;
use knorke_light2d::{ObstacleId, ObstacleStore};
use knorke_math::{At, Vec2, World2};

/// Size of the dumpers' golden scenes (`light_ref.cpp`, `composite_ref.cpp`, `atlas_ref.cpp`).
pub(crate) const W: u32 = 96;
pub(crate) const H: u32 = 48;

pub(crate) fn at(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

/// `name` must be bound at `(set, slot)` when the layout gives it a slot, and must not exist
/// in the shader otherwise.
fn check_binding(
    bindings: &BTreeMap<String, (u32, u32)>,
    name: &str,
    set: u32,
    slot: Option<u8>,
    ctx: &str,
) {
    match slot {
        None => assert!(!bindings.contains_key(name), "{ctx}: unexpected {name}"),
        Some(s) => assert_eq!(
            bindings.get(name),
            Some(&(set, u32::from(s))),
            "{ctx}: {name}"
        ),
    }
}

/// The reflection counts of a fragment/vertex pair and the descriptor set and binding of every
/// named resource are the ones a [`LightLayout`] gives (port of Chiaro's
/// `LightProgramLayoutTests.cpp`; the composite programs share the layout type).
pub(crate) fn check_layout(frag: &Program<'_>, vert: &Program<'_>, l: &LightLayout) {
    let ctx = format!("{} with {}", frag.name, vert.name);
    assert_eq!(frag.stage, Stage::Fragment, "{ctx}");
    assert_eq!(vert.stage, Stage::Vertex, "{ctx}");
    let (f, v) = (frag.reflection, vert.reflection);
    assert_eq!(f.samplers, u16::from(l.sampler_count), "{ctx}: samplers");
    assert_eq!(
        f.uniform_buffers,
        u16::from(l.uniform_count),
        "{ctx}: uniforms"
    );
    assert_eq!(
        f.storage_buffers,
        u16::from(l.fragment_storage_count),
        "{ctx}: storage"
    );
    assert_eq!(f.storage_textures, 0, "{ctx}");
    assert_eq!(v.samplers, 0, "{ctx}");
    assert_eq!(
        v.storage_buffers,
        u16::from(l.vertex_storage_count),
        "{ctx}: vertex storage"
    );
    assert_eq!(
        v.uniform_buffers,
        u16::from(l.vertex_uniform_count),
        "{ctx}: vertex uniforms"
    );

    // SDL's SPIR-V convention: fragment samplers then storage buffers in set 2, fragment
    // uniforms in set 3, vertex storage buffers in set 0.
    let fb = knorke_gpu::test_support::spirv::decode(frag.code).bindings;
    let vb = knorke_gpu::test_support::spirv::decode(vert.code).bindings;
    for s in LightSampler::ALL {
        check_binding(&fb, s.hlsl_name(), 2, l.sampler(s), &ctx);
    }
    for u in LightUniform::ALL {
        check_binding(&fb, u.hlsl_name(), 3, l.uniform(u), &ctx);
    }
    for b in LightStorage::ALL {
        let slot = l.storage(b);
        if b.is_vertex() {
            check_binding(&vb, b.hlsl_name(), 0, slot, &ctx);
        } else {
            check_binding(
                &fb,
                b.hlsl_name(),
                2,
                slot.map(|s| l.sampler_count + s),
                &ctx,
            );
        }
    }
}

/// The dome normal map of `Material()` in the dumpers `light_ref.cpp` and `composite_ref.cpp`, as
/// material pixels (satin): a half sphere of radius 20 around (48, 24).
pub(crate) fn dome_material_pixels(w: u32, h: u32) -> Vec<u8> {
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

/// The programs of the occluder stages and the occlusion mask, in the order their stages index them.
pub(crate) fn programs() -> Vec<ProgramDesc> {
    OCCLUDER_PROGRAMS
        .iter()
        .chain(&MASK_PROGRAMS)
        .copied()
        .collect()
}

pub(crate) fn polygon(
    store: &mut ObstacleStore,
    xy: &[Vec2],
    common: ObstacleCommon,
) -> ObstacleId {
    store
        .create_shape(&ShapeDesc {
            common,
            shape: Shape::Polygon(xy),
        })
        .unwrap()
}

pub(crate) fn rect_xy(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
    [
        Vec2::new(x0, y0),
        Vec2::new(x1, y0),
        Vec2::new(x1, y1),
        Vec2::new(x0, y1),
    ]
}

/// The scene of `tests/golden/chiaro/occluder/mask_ref.cpp` (`scene_mask`) and of `field_ref.cpp`
/// (`Scene()`), built into `store`: static and dynamic polygons with opacities, a footprint
/// obstacle, two circles (one thinner than a texel) and two tile grids (one rotated with cell
/// opacities, one with edge strips). Returns the dynamic bar.
pub(crate) fn scene_mask_into(store: &mut ObstacleStore) -> ObstacleId {
    let still = ObstacleCommon::default();
    let with = |x, y, rotation, opacity| ObstacleCommon {
        position: at(x, y),
        rotation,
        opacity,
        ..still
    };
    polygon(
        store,
        &rect_xy(48.0, 24.0, 56.0, 40.0),
        with(0.0, 0.0, 0.0, 0.75),
    );
    let tri = [
        Vec2::new(70.0, 10.0),
        Vec2::new(90.0, 14.0),
        Vec2::new(76.0, 30.0),
    ];
    polygon(store, &tri, with(0.0, 0.0, 0.0, 1.0));
    let moving = ObstacleCommon {
        dynamic: true,
        ..with(100.0, 40.0, 33.0, 0.5)
    };
    let bar = polygon(store, &rect_xy(-10.0, -2.0, 10.0, 2.0), moving);
    for (radius, x, y) in [(6.0, 30.0, 45.0), (0.2, 64.25, 32.25)] {
        store
            .create_shape(&ShapeDesc {
                common: with(x, y, 0.0, 1.0),
                shape: Shape::Circle { radius },
            })
            .unwrap();
    }
    let footprint = ObstacleCommon {
        shadow_footprint: true,
        ..with(0.0, 0.0, 0.0, 0.6)
    };
    polygon(store, &rect_xy(10.0, 10.0, 20.0, 18.0), footprint);

    let solid: Vec<u8> = (0..32).map(|i| u8::from((i * 7 + 3) % 5 != 0)).collect();
    let opacity: Vec<u8> = (0..32).map(|i| (255 - (i % 4) * 60) as u8).collect();
    let mut grid = TileGridDesc::new(8, 4, &solid);
    grid.cell_size = Vec2::new(6.0, 5.0);
    grid.opacity = Some(&opacity);
    grid.common = with(60.0, 38.0, 20.0, 0.8);
    store.create_tile_grid(&grid).unwrap();

    let solid = [1u8; 6];
    let edges = [1u8, 2, 4, 8, 5, 0];
    let mut grid = TileGridDesc::new(3, 2, &solid);
    grid.cell_size = Vec2::new(10.0, 10.0);
    grid.edge_width = 3.0;
    grid.edges = Some(&edges);
    grid.common = with(90.0, 5.0, 0.0, 1.0);
    store.create_tile_grid(&grid).unwrap();
    bar
}

pub(crate) fn scene_mask() -> ObstacleStore {
    let mut store = ObstacleStore::new();
    scene_mask_into(&mut store);
    store
}
