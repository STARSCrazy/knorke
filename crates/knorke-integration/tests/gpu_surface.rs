//! Surface geometry (P35) on Vulkan: the cases of Chiaro's `tests/gpu/SurfaceHelperTests.cpp`
//! that apply to the GPU batch (a box equals the demo's gradient building, bytes exact; a wall
//! faces the viewer whatever the order of its ends; floor and quad encode height and normal
//! exactly; painter's order), plus what the redesign adds (every wall pixel against its exact
//! height, errors as codes) and the checks of the crate's own shaders.
//!
//! The GPU tests need a Vulkan device and the shader pack of `cargo xtask shaders` with this
//! crate's shaders in it; they skip without them (a failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use std::path::PathBuf;

use knorke_gpu::test_support::{read_texture, spirv, with_chiaro_pack};
use knorke_gpu::{CommandBuffer, Gpu, ShaderLibrary};
use knorke_integration::layers::{LAYER_TARGETS, Layer, LayerSet, Layers};
use knorke_integration::surface::{SURFACE_NOT_PREPARED, SurfaceBatch};
use knorke_light2d::Projection;
use knorke_light2d::codec::{decode_normal, surface_pixel};
use knorke_light2d::surface_geometry::{SurfaceBox, SurfaceGeometry, SurfaceVertex, toward_viewer};
use knorke_math::{At, Vec2, Vec3};

const W: u32 = 48;
const H: u32 = 32;
const STEP: f32 = 0.1;

/// An oblique iso-like view: x right-down, y left-down, height up.
const ISO: Projection = Projection {
    origin: At::new(Vec2::new(24.0, 12.0)),
    x_axis: Vec2::new(1.5, 0.75),
    y_axis: Vec2::new(-1.5, 0.75),
    z_axis: Vec2::new(0.0, -1.5),
};

struct Rig<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    batch: SurfaceBatch<'d>,
    layers: Layers<'d>,
}

impl<'d> Rig<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        Self {
            gpu,
            batch: SurfaceBatch::new(gpu, &shaders).unwrap(),
            layers: Layers::new(gpu, W, H, Layer::Surface.into()).unwrap(),
            shaders,
        }
    }

    /// Clears the layers, draws each geometry in order and returns the surface layer.
    fn draw(&mut self, geometries: &[&SurfaceGeometry]) -> Vec<u8> {
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        drop(self.layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
        for g in geometries {
            self.batch.prepare(&mut self.shaders, g).unwrap();
            self.batch.record(&mut cmd, &mut self.layers, g).unwrap();
        }
        cmd.submit_and_wait().unwrap();
        read_texture(self.gpu, self.layers.texture(Layer::Surface).unwrap())
    }
}

fn entries(px: &[u8]) -> usize {
    px.chunks(4).filter(|p| p[3] >= 128).count()
}

fn pixel(px: &[u8], at: At<knorke_math::Screen, Vec2>) -> &[u8] {
    let p = at.raw();
    &px[((p.y as u32 * W + p.x as u32) * 4) as usize..][..4]
}

fn project(x: f32, y: f32, z: f32) -> At<knorke_math::Screen, Vec2> {
    ISO.project(At::new(Vec2::new(x, y)), z)
}

fn boxed(x: f32, y: f32, width: f32, depth: f32, height: f32) -> SurfaceBox {
    SurfaceBox {
        center: At::new(Vec2::new(x, y)),
        width,
        depth,
        height,
    }
}

/// The iso_night_demo building code (examples/iso_night_demo/Scene.cpp of Chiaro), surface pass
/// only, as triangles of the host's own.
fn demo_box(g: &mut SurfaceGeometry, x0: f32, y0: f32, x1: f32, y1: f32, top: f32) {
    let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let v = toward_viewer(&ISO);
    let centre = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let mut quad = |pts: [(f32, f32, f32); 4], n: Vec3| {
        let vertex = |(x, y, z): (f32, f32, f32)| SurfaceVertex {
            position: project(x, y, z),
            pixel: surface_pixel(z, STEP, n),
        };
        let q = pts.map(vertex);
        g.push_triangle([q[0], q[1], q[2]]);
        g.push_triangle([q[0], q[2], q[3]]);
    };
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        let (mut nx, mut ny) = (b.1 - a.1, a.0 - b.0);
        let l = (nx * nx + ny * ny).sqrt();
        (nx, ny) = (nx / l, ny / l);
        let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        if nx * (mid.0 - centre.0) + ny * (mid.1 - centre.1) < 0.0 {
            (nx, ny) = (-nx, -ny);
        }
        if nx * v.x + ny * v.y <= 0.0 {
            continue;
        }
        let pts = [
            (a.0, a.1, 0.0),
            (b.0, b.1, 0.0),
            (b.0, b.1, top),
            (a.0, a.1, top),
        ];
        quad(pts, Vec3::new(nx, ny, 0.0));
    }
    quad(c.map(|(x, y)| (x, y, top)), Vec3::new(0.0, 0.0, 1.0));
}

#[test]
fn a_box_equals_the_demo_gradient_building_bytes_exact() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let boxes = [
            boxed(0.0, 0.0, 6.0, 4.0, 5.0),
            boxed(8.0, -3.0, 3.0, 3.0, 2.5),
        ];
        let mut helper = SurfaceGeometry::new();
        helper.push_boxes(&ISO, STEP, &boxes).unwrap();
        let mut demo = SurfaceGeometry::new();
        demo_box(&mut demo, -3.0, -2.0, 3.0, 2.0, 5.0);
        demo_box(&mut demo, 6.5, -4.5, 9.5, -1.5, 2.5);
        let drawn = rig.draw(&[&helper]);
        assert!(entries(&drawn) > 100);
        assert!(drawn == rig.draw(&[&demo]));
        // A batch equals one geometry per box, drawn in order.
        let singles: Vec<SurfaceGeometry> = boxes
            .iter()
            .map(|b| {
                let mut g = SurfaceGeometry::new();
                g.push_boxes(&ISO, STEP, core::slice::from_ref(b)).unwrap();
                g
            })
            .collect();
        assert!(rig.draw(&[&singles[0], &singles[1]]) == drawn);
    });
}

#[test]
fn a_wall_faces_the_viewer_and_every_pixel_holds_its_height() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let (a, b) = (Vec3::new(-4.0, 2.0, 0.0), Vec3::new(4.0, 2.0, 0.0));
        let (mut ab, mut ba) = (SurfaceGeometry::new(), SurfaceGeometry::new());
        ab.push_wall(&ISO, STEP, a, b, 6.0).unwrap();
        ba.push_wall(&ISO, STEP, b, a, 6.0).unwrap();
        let px = rig.draw(&[&ab]);
        assert!(entries(&px) > 50);
        assert!(px == rig.draw(&[&ba]));
        let v = toward_viewer(&ISO);
        let mid = pixel(&px, project(0.0, 2.0, 3.0));
        assert_eq!(mid[3], 255);
        let n = decode_normal(mid[1], mid[2]);
        assert!(n.x * v.x + n.y * v.y > 0.5 && n.y > 0.99, "{n:?}");
        // Every entry: the normal bytes of the wall and the height of its pixel centre, within
        // one step (the interpolation rounds where the exact height lies near half a step).
        let want = surface_pixel(0.0, STEP, Vec3::new(0.0, 1.0, 0.0));
        for (i, p) in px.chunks(4).enumerate().filter(|(_, p)| p[3] > 0) {
            let (sx, sy) = ((i as u32 % W) as f32 + 0.5, (i as u32 / W) as f32 + 0.5);
            // Invert the projection on the wall plane y = 2.
            let x = (sx - 24.0 + 3.0) / 1.5;
            let z = (12.0 + 1.5 + 0.75 * x - sy) / 1.5;
            let exact = (z / STEP).round().clamp(0.0, 60.0);
            assert!(p[1..] == want[1..], "({sx}, {sy}): {p:?}");
            assert!(
                (f32::from(p[0]) - exact).abs() <= 1.0,
                "({sx}, {sy}): {} vs {exact}",
                p[0]
            );
        }
    });
}

#[test]
fn floor_and_quad_encode_height_and_normal_exactly() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let xy = [(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)];
        let mut floor = SurfaceGeometry::new();
        let points = xy.map(|(x, y)| At::new(Vec2::new(x, y)));
        floor.push_floor(&ISO, STEP, &points, 1.5).unwrap();
        let drawn = rig.draw(&[&floor]);
        let up = surface_pixel(1.5, STEP, Vec3::new(0.0, 0.0, 1.0));
        assert!(entries(&drawn) > 50);
        assert!(drawn.chunks(4).filter(|p| p[3] > 0).all(|p| p == up));
        assert_eq!(pixel(&drawn, project(0.0, 0.0, 1.5)), up);

        let mut quad = SurfaceGeometry::new();
        let corners = xy.map(|(x, y)| Vec3::new(x, y, 1.5));
        quad.push_quad_corners(&ISO, STEP, &corners).unwrap();
        assert!(rig.draw(&[&quad]) == drawn);
        let slope = [
            (-3.0, -3.0, 0.0),
            (3.0, -3.0, 3.0),
            (3.0, 3.0, 3.0),
            (-3.0, 3.0, 0.0),
        ];
        quad.clear();
        quad.push_quad_corners(&ISO, STEP, &slope.map(|(x, y, z)| Vec3::new(x, y, z)))
            .unwrap();
        let s = rig.draw(&[&quad]);
        let p = pixel(&s, project(0.0, 0.0, 1.5));
        let n = decode_normal(p[1], p[2]);
        assert!(
            (n.x + 0.447).abs() < 0.03 && (n.z - 0.894).abs() < 0.03,
            "{n:?}"
        );
        assert!((f32::from(p[0]) - 15.0).abs() <= 1.0, "{p:?}");
    });
}

#[test]
fn one_geometry_draws_in_painters_order() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let (a, b) = (Vec3::new(-6.0, 1.0, 0.0), Vec3::new(6.0, 1.0, 0.0));
        let floor = [(-8.0, -6.0), (8.0, -6.0), (8.0, 6.0), (-8.0, 6.0)]
            .map(|(x, y)| At::new(Vec2::new(x, y)));
        let building = [boxed(0.0, -2.0, 6.0, 4.0, 5.0)];
        let mut parts = [(); 3].map(|_| SurfaceGeometry::new());
        parts[0].push_floor(&ISO, STEP, &floor, 0.5).unwrap();
        parts[1].push_boxes(&ISO, STEP, &building).unwrap();
        parts[2].push_wall(&ISO, STEP, a, b, 3.0).unwrap();
        let mut all = SurfaceGeometry::new();
        all.push_floor(&ISO, STEP, &floor, 0.5).unwrap();
        all.push_boxes(&ISO, STEP, &building).unwrap();
        all.push_wall(&ISO, STEP, a, b, 3.0).unwrap();
        let painter = rig.draw(&[&all]);
        assert!(painter == rig.draw(&[&parts[0], &parts[1], &parts[2]]));
        // The order matters: the wall drawn first is covered by the box.
        let mut reversed = SurfaceGeometry::new();
        reversed.push_wall(&ISO, STEP, a, b, 3.0).unwrap();
        reversed.push_boxes(&ISO, STEP, &building).unwrap();
        reversed.push_floor(&ISO, STEP, &floor, 0.5).unwrap();
        assert!(rig.draw(&[&reversed]) != painter);
        // A failed call appends nothing.
        assert!(all.push_wall(&ISO, STEP, a, a, 3.0).is_err());
        assert!(rig.draw(&[&all]) == painter);
    });
}

#[test]
fn errors_for_an_unprepared_batch_and_a_missing_surface_layer() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let mut g = SurfaceGeometry::new();
        g.push_boxes(&ISO, STEP, &[boxed(0.0, 0.0, 2.0, 2.0, 1.0)])
            .unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let e = rig.batch.record(&mut cmd, &mut rig.layers, &g).unwrap_err();
        assert_eq!(e.code, SURFACE_NOT_PREPARED);
        let mut other = Layers::new(gpu, W, H, Layer::Material.into()).unwrap();
        rig.batch.prepare(&mut rig.shaders, &g).unwrap();
        let e = rig.batch.record(&mut cmd, &mut other, &g).unwrap_err();
        assert_eq!(e.code, LAYER_TARGETS);
        assert!(e.message.contains("Surface"), "{}", e.message);
        cmd.submit_and_wait().unwrap();
    });
}

/// The hand-written reflection files of the surface shaders match what the SPIR-V declares,
/// at the descriptor sets SDL expects (vertex resources in set 0, vertex uniforms in 1).
#[test]
fn surface_reflection_files_match_the_spirv() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders/compiled");
    let bindings = |name: &str| {
        let code = std::fs::read(dir.join(format!("{name}.spv"))).unwrap();
        let json = std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap();
        let found: Vec<(String, (u32, u32))> = spirv::decode(&code).bindings.into_iter().collect();
        (found, json)
    };
    let (vert, json) = bindings("surface_geometry.vert");
    let named: Vec<(&str, (u32, u32))> = vert.iter().map(|(n, b)| (n.as_str(), *b)).collect();
    assert_eq!(named, [("SurfaceParams", (1, 0)), ("Vertices", (0, 0))]);
    assert!(json.contains("\"storage_buffers\": 1") && json.contains("\"uniform_buffers\": 1"));
    let (frag, json) = bindings("surface_geometry.frag");
    assert!(frag.is_empty(), "{frag:?}");
    assert!(json.contains("\"samplers\": 0") && json.contains("\"storage_buffers\": 0"));
}
