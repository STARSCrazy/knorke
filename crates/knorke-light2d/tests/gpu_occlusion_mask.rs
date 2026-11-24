//! P23 on a real GPU: the occlusion mask cache and its jump flooding distance field
//! (`knorke_light2d::occlusion::OcclusionMask`), read back and compared with C++ Chiaro
//! (chiaro @ 9bc5b9b, dumps of `tests/golden/chiaro/field/field_ref.cpp`, zlib license) and
//! with full rebuilds of the same state.
//!
//! The scene is `scene_mask` of P22 (128x64 scene, 32-texel margin band, so the mask is
//! 192x128). The test plays the host (ADR 0002): it owns device, command buffers and the
//! obstacle stages, submits and reads back. It skips (passes with a note) without a Vulkan
//! device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV;
use knorke_gpu::{
    CommandBuffer, Device, DownloadBuffer, Filter, Gpu, LiveCounter, PipelineCache, ProgramDesc,
    Samplers, ShaderLibrary, TextureRef, init_video,
};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::occluder_raster::{
    NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms, TileOccluders,
    thin_circle_radius,
};
use knorke_light2d::occlusion::{FIELD_BOUND, MASK_PROGRAMS, MaskPrograms, OcclusionMask};
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdate};
use knorke_light2d::{ObstacleId, ObstacleStore, Projection};
use knorke_math::{At, Screen, Vec2, World2};

static SERIAL: Mutex<()> = Mutex::new(());

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

/// What one frame left in the textures.
#[derive(Clone, PartialEq)]
struct Out {
    update: MaskUpdate,
    /// R8G8 mask bytes.
    mask: Vec<u8>,
    /// R16F field as raw little-endian bytes.
    field: Vec<u8>,
    /// RGBA16F height bytes, empty without height mode.
    height: Vec<u8>,
}

impl Out {
    fn distance(&self, x: usize, y: usize) -> f32 {
        let i = (y * 192 + x) * 2;
        half(u16::from_le_bytes([self.field[i], self.field[i + 1]]))
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

struct Host<'d> {
    gpu: Gpu<'d>,
    device: &'d Device,
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    occlusion: OcclusionMask<'d>,
    projection: Projection,
}

fn programs() -> Vec<ProgramDesc> {
    OCCLUDER_PROGRAMS
        .iter()
        .chain(&MASK_PROGRAMS)
        .copied()
        .collect()
}

impl<'d> Host<'d> {
    fn new(gpu: Gpu<'d>, device: &'d Device, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &programs()).unwrap();
        let mut occlusion = OcclusionMask::new(gpu, MaskPrograms::at(OCCLUDER_PROGRAMS.len()));
        // Local updates are never cheaper at this size (as in field_ref.cpp).
        occlusion.set_full_rebuild_ratio(1.0);
        Self {
            gpu,
            device,
            shaders,
            pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            occlusion,
            projection: Projection::IDENTITY,
        }
    }

    fn download(&self, cmd: &mut CommandBuffer<'_>, t: TextureRef<'_>) -> DownloadBuffer<'d> {
        let down = DownloadBuffer::new(self.gpu, t.desc().bytes() as u32).unwrap();
        cmd.copy_pass().unwrap().download_texture(t, &down).unwrap();
        down
    }

    fn frame(&mut self, store: &ObstacleStore, height_mode: bool) -> Out {
        let input = MaskPlanInput::new(store, &self.projection, 128, 64, 1.0);
        let update = self
            .occlusion
            .prepare(&mut self.shaders, &mut self.pipelines, &input, height_mode)
            .unwrap();
        assert_eq!(update, self.occlusion.last_update());
        if update != MaskUpdate::Reused {
            let min_radius = thin_circle_radius(&self.projection, &input.domain);
            self.geometry
                .prepare(
                    &mut self.shaders,
                    &mut self.pipelines,
                    store,
                    &NoPatterns,
                    height_mode,
                    min_radius,
                )
                .unwrap();
            self.tiles
                .prepare(
                    &mut self.shaders,
                    &mut self.pipelines,
                    store,
                    &NoPatterns,
                    height_mode,
                )
                .unwrap();
        }
        let mut cmd = CommandBuffer::acquire(self.device).unwrap();
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
        let mask = self.download(&mut cmd, self.occlusion.mask().unwrap());
        let field = self.download(&mut cmd, self.occlusion.field().unwrap());
        let height = self.occlusion.height().map(|h| self.download(&mut cmd, h));
        cmd.submit_and_wait().unwrap();
        let read = |mut d: DownloadBuffer<'_>| {
            let mut v = vec![0; d.size() as usize];
            d.read(&mut v).unwrap();
            v
        };
        Out {
            update,
            mask: read(mask),
            field: read(field),
            height: height.map(read).unwrap_or_default(),
        }
    }
}

fn with_host(body: impl for<'d> FnOnce(&mut Host<'d>, &(dyn Fn() -> Host<'d> + 'd))) {
    let Some((_guard, device, pack)) = device() else {
        return;
    };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let fresh = || Host::new(gpu, &device, &pack);
        let mut host = fresh();
        body(&mut host, &fresh);
    }
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}

fn polygon(store: &mut ObstacleStore, xy: &[Vec2], common: ObstacleCommon) -> ObstacleId {
    store
        .create_shape(&ShapeDesc {
            common,
            shape: Shape::Polygon(xy),
        })
        .unwrap()
}

fn rect_xy(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
    [
        Vec2::new(x0, y0),
        Vec2::new(x1, y0),
        Vec2::new(x1, y1),
        Vec2::new(x0, y1),
    ]
}

/// `Scene()` of `field_ref.cpp` (= `scene_mask` of P22); returns the dynamic bar.
fn scene(store: &mut ObstacleStore) -> ObstacleId {
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

fn pan(p: &mut Projection, dx: f32, dy: f32) {
    let o = p.origin.raw();
    p.origin = At::<Screen, Vec2>::new(Vec2::new(o.x + dx, o.y + dy));
}

fn golden(name: &str) -> Vec<u8> {
    let path = workspace().join("tests/golden/chiaro/field").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Differing bytes against `want`, reported.
fn compare(name: &str, got: &[u8], want: &[u8]) -> usize {
    assert_eq!(got.len(), want.len(), "{name}: size");
    let count = got.iter().zip(want).filter(|(a, b)| a != b).count();
    eprintln!(
        "{name}: {count} of {} bytes differ from the reference",
        got.len()
    );
    count
}

/// Largest difference of two R16F fields, and how many texels differ by more than 0.01.
fn field_difference(a: &Out, b: &Out) -> (f32, usize) {
    let mut worst = 0.0f32;
    let mut differing = 0;
    for y in 0..128 {
        for x in 0..192 {
            let d = (a.distance(x, y) - b.distance(x, y)).abs();
            worst = knorke_math::det::max(worst, d);
            differing += usize::from(d > 0.01);
        }
    }
    (worst, differing)
}

#[test]
fn the_field_holds_clamped_texel_distances_to_the_nearest_obstacle() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        // One opaque texel column at scene x = 52 .. 53 (mask x = 84), rows 24..40.
        polygon(
            &mut store,
            &rect_xy(52.0, 24.0, 53.0, 40.0),
            ObstacleCommon::default(),
        );
        let out = host.frame(&store, false);
        assert_eq!(out.update, MaskUpdate::Full);
        let (x, y) = (84, 64);
        assert_eq!(out.distance(x, y), 0.0);
        assert_eq!(out.distance(x + 3, y), 3.0);
        assert_eq!(out.distance(x - 7, y), 7.0);
        // Diagonal to the column's end (mask row 56 + 15 = 71): 3-4-5.
        assert_eq!(out.distance(x + 3, 71 + 4), 5.0);
        assert_eq!(out.distance(x + 40, y), FIELD_BOUND as f32);
        assert_eq!(out.distance(0, 0), FIELD_BOUND as f32);
    });
}

#[test]
fn the_full_frame_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        scene(&mut store);
        let out = host.frame(&store, false);
        assert_eq!(out.update, MaskUpdate::Full);
        assert_eq!(compare("full mask", &out.mask, &golden("full.mask.bin")), 0);
        assert_eq!(
            compare("full field", &out.field, &golden("full.field.bin")),
            0
        );
    });
}

/// The frames of `field_ref.cpp`: full, reused, a whole-pixel pan with the bar moved, a
/// fractional pan.
#[test]
fn the_cached_sequence_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        let bar = scene(&mut store);
        let want: Vec<MaskUpdate> = String::from_utf8(golden("updates.txt"))
            .unwrap()
            .lines()
            .map(|l| match l {
                "0" => MaskUpdate::Reused,
                "1" => MaskUpdate::Incremental,
                _ => MaskUpdate::Full,
            })
            .collect();
        let mut got = Vec::new();
        let full = host.frame(&store, false);
        got.push(full.update);
        let reused = host.frame(&store, false);
        got.push(reused.update);
        assert!(reused.mask == full.mask && reused.field == full.field);

        pan(&mut host.projection, -3.0, 2.0);
        store.set_transform(bar, at(96.0, 44.0), 50.0).unwrap();
        let whole = host.frame(&store, false);
        got.push(whole.update);
        assert_eq!(
            compare("pan_whole mask", &whole.mask, &golden("pan_whole.mask.bin")),
            0
        );
        assert_eq!(
            compare(
                "pan_whole field",
                &whole.field,
                &golden("pan_whole.field.bin")
            ),
            0
        );

        pan(&mut host.projection, -1.4, -0.6);
        let fraction = host.frame(&store, false);
        got.push(fraction.update);
        assert_eq!(
            compare(
                "pan_fraction mask",
                &fraction.mask,
                &golden("pan_fraction.mask.bin")
            ),
            0
        );
        assert_eq!(
            compare(
                "pan_fraction field",
                &fraction.field,
                &golden("pan_fraction.field.bin")
            ),
            0
        );
        assert_eq!(got, want);
    });
}

#[test]
fn an_incremental_update_equals_a_full_rebuild_of_the_same_state() {
    with_host(|host, fresh| {
        let mut store = ObstacleStore::new();
        let bar = scene(&mut store);
        host.frame(&store, false);
        // Whole-pixel pans keep the anchor remainder: the cache must be exact. Moving and
        // turning the bar redraws its old and new rectangle.
        for (i, (dx, dy)) in [(-3.0, 2.0), (5.0, 0.0), (0.0, -7.0), (2.0, 3.0)]
            .into_iter()
            .enumerate()
        {
            pan(&mut host.projection, dx, dy);
            let angle = 50.0 + 20.0 * i as f32;
            store
                .set_transform(bar, at(96.0 - 4.0 * i as f32, 44.0), angle)
                .unwrap();
            let cached = host.frame(&store, false);
            assert_eq!(cached.update, MaskUpdate::Incremental, "frame {i}");
            let mut rebuilt = fresh();
            rebuilt.projection = host.projection;
            let full = rebuilt.frame(&store, false);
            assert_eq!(full.update, MaskUpdate::Full);
            assert_eq!(compare("incremental mask", &cached.mask, &full.mask), 0);
            assert_eq!(compare("incremental field", &cached.field, &full.field), 0);
        }
    });
}

#[test]
fn a_fractional_pan_stays_within_chiaros_tolerance_of_a_full_rebuild() {
    // A pan by a fraction of a texel moves the anchor remainder: texels that are shifted keep
    // the old rasterization, the redrawn ones get the new one. Chiaro accepts that
    // (FieldTests.cpp, CompareFields: at most 1 texel, at most 0.1 % of the texels).
    with_host(|host, fresh| {
        let mut store = ObstacleStore::new();
        scene(&mut store);
        host.frame(&store, false);
        for i in 0..6 {
            pan(&mut host.projection, 1.37, 0.61);
            let cached = host.frame(&store, false);
            assert_eq!(cached.update, MaskUpdate::Incremental, "frame {i}");
            let mut rebuilt = fresh();
            rebuilt.projection = host.projection;
            let full = rebuilt.frame(&store, false);
            let (worst, differing) = field_difference(&cached, &full);
            eprintln!("frame {i}: worst {worst}, {differing} texels differ");
            assert!(worst <= 1.0, "frame {i}: {worst}");
            assert!(differing * 1000 <= 192 * 128, "frame {i}: {differing}");
        }
    });
}

#[test]
fn height_mode_shifts_and_redraws_the_height_texture_like_a_full_rebuild() {
    with_host(|host, fresh| {
        let mut store = ObstacleStore::new();
        let tall = |height, base, dynamic| ObstacleCommon {
            height,
            base_height: base,
            dynamic,
            ..Default::default()
        };
        polygon(
            &mut store,
            &rect_xy(10.0, 10.0, 30.0, 30.0),
            tall(2.0, 1.0, false),
        );
        let mover = polygon(
            &mut store,
            &rect_xy(20.0, 20.0, 40.0, 40.0),
            tall(6.0, 3.0, true),
        );
        let first = host.frame(&store, true);
        assert_eq!(first.update, MaskUpdate::Full);
        assert_eq!(first.height.len(), 192 * 128 * 8);

        pan(&mut host.projection, 4.0, -2.0);
        store.set_transform(mover, at(30.0, 5.0), 0.0).unwrap();
        let cached = host.frame(&store, true);
        assert_eq!(cached.update, MaskUpdate::Incremental);
        let mut rebuilt = fresh();
        rebuilt.projection = host.projection;
        let full = rebuilt.frame(&store, true);
        assert_eq!(compare("height mask", &cached.mask, &full.mask), 0);
        assert_eq!(compare("height field", &cached.field, &full.field), 0);
        assert_eq!(compare("height texture", &cached.height, &full.height), 0);

        // Leaving height mode rebuilds and drops the height texture.
        let flat = host.frame(&store, false);
        assert_eq!(flat.update, MaskUpdate::Full);
        assert!(flat.height.is_empty());
    });
}

#[test]
fn unchanged_frames_are_reused_and_allocate_no_textures() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        let bar = scene(&mut store);
        host.frame(&store, false);
        let textures = host.occlusion.allocations();
        assert_eq!(textures, 6); // mask, field and seed pairs
        for _ in 0..3 {
            assert_eq!(host.frame(&store, false).update, MaskUpdate::Reused);
        }
        // A sub-texel pan does not move the anchor by a whole texel: still reused.
        pan(&mut host.projection, 0.25, 0.0);
        assert_eq!(host.frame(&store, false).update, MaskUpdate::Reused);
        for i in 0..4 {
            pan(&mut host.projection, 1.0, 0.0);
            store
                .set_transform(bar, at(90.0 + i as f32, 40.0), 33.0)
                .unwrap();
            assert_eq!(host.frame(&store, false).update, MaskUpdate::Incremental);
        }
        assert_eq!(host.occlusion.allocations(), textures);
    });
}

#[test]
fn a_frame_that_is_not_committed_or_a_restore_rebuilds_fully() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        scene(&mut store);
        host.frame(&store, false);
        assert_eq!(host.frame(&store, false).update, MaskUpdate::Reused);

        // Prepared, never recorded: the textures no longer count.
        let input = MaskPlanInput::new(&store, &host.projection, 128, 64, 1.0);
        host.occlusion
            .prepare(&mut host.shaders, &mut host.pipelines, &input, false)
            .unwrap();
        host.occlusion.commit(); // not recorded: no effect
        assert_eq!(host.frame(&store, false).update, MaskUpdate::Full);
        assert_eq!(host.frame(&store, false).update, MaskUpdate::Reused);

        host.occlusion.invalidate();
        assert_eq!(host.frame(&store, false).update, MaskUpdate::Full);

        // Recording twice is refused.
        let mut cmd = CommandBuffer::acquire(host.device).unwrap();
        let e = host
            .occlusion
            .record(
                &mut cmd,
                &host.pipelines,
                host.samplers.sampler(Filter::Nearest),
                &mut host.geometry,
                &mut host.tiles,
            )
            .unwrap_err();
        assert_eq!(e.code, knorke_light2d::occlusion::NOT_PREPARED);
    });
}

#[test]
fn a_larger_scene_reallocates_and_rebuilds() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        scene(&mut store);
        host.frame(&store, false);
        let before = host.occlusion.allocations();
        let input = MaskPlanInput::new(&store, &host.projection, 256, 64, 1.0);
        let update = host
            .occlusion
            .prepare(&mut host.shaders, &mut host.pipelines, &input, false)
            .unwrap();
        assert_eq!(update, MaskUpdate::Full);
        assert_eq!(host.occlusion.allocations(), before + 6);
    });
}
