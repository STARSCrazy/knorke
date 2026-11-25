//! P23 on a real GPU: the occlusion mask cache and its jump flooding distance field
//! (`knorke_light2d::occlusion::OcclusionMask`), read back and compared with C++ Chiaro
//! (chiaro @ 9bc5b9b, dumps of `tests/golden/chiaro/field/field_ref.cpp` and, for the sprite mask
//! in height mode, `occluder/mask_ref.cpp`; zlib license) and
//! with full rebuilds of the same state.
//!
//! The scene is `scene_mask` of P22 (128x64 scene, 32-texel margin band, so the mask is
//! 192x128). The test plays the host (ADR 0002): it owns device, command buffers and the
//! obstacle stages, submits and reads back. It skips (passes with a note) without a Vulkan
//! device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

mod common;

use common::{at, polygon, programs, rect_xy, scene_mask_into};
use knorke_gpu::test_support::{
    assert_same_bytes, golden_bytes, half_to_f32, queue_download, read_download, sampled_rgba8,
    with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Filter, Gpu, PipelineCache, Samplers, ShaderLibrary, TextureRef};
use knorke_light2d::obstacle::ObstacleCommon;
use knorke_light2d::occluder_raster::{
    NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry, OccluderPrograms, TileOccluders,
    thin_circle_radius,
};
use knorke_light2d::occlusion::{FIELD_BOUND, MaskPrograms, OcclusionMask};
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdate};
use knorke_light2d::{ObstacleStore, Projection};
use knorke_math::{At, Screen, Vec2};

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
        half_to_f32(u16::from_le_bytes([self.field[i], self.field[i + 1]]))
    }
}

struct Host<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    occlusion: OcclusionMask<'d>,
    projection: Projection,
}

impl<'d> Host<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &programs()).unwrap();
        let mut occlusion = OcclusionMask::new(gpu, MaskPrograms::at(OCCLUDER_PROGRAMS.len()));
        // Local updates are never cheaper at this size (as in field_ref.cpp).
        occlusion.set_full_rebuild_ratio(1.0);
        Self {
            gpu,
            shaders,
            pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            occlusion,
            projection: Projection::IDENTITY,
        }
    }

    fn frame(&mut self, store: &ObstacleStore, height_mode: bool) -> Out {
        self.frame_with_sprite(store, height_mode, None)
    }

    /// A frame with the application's sprite mask `sprite` (scene size) drawn after the obstacles.
    fn frame_with_sprite(
        &mut self,
        store: &ObstacleStore,
        height_mode: bool,
        sprite: Option<TextureRef<'_>>,
    ) -> Out {
        let mut input = MaskPlanInput::new(store, &self.projection, 128, 64, 1.0);
        input.sprite = sprite.is_some();
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
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        self.occlusion
            .record_with_sprite(
                &mut cmd,
                &self.pipelines,
                self.samplers.sampler(Filter::Nearest),
                &mut self.geometry,
                &mut self.tiles,
                sprite,
            )
            .unwrap();
        self.occlusion.commit();
        let mask = queue_download(self.gpu, &mut cmd, self.occlusion.mask().unwrap());
        let field = queue_download(self.gpu, &mut cmd, self.occlusion.field().unwrap());
        let height = self
            .occlusion
            .height()
            .map(|h| queue_download(self.gpu, &mut cmd, h));
        cmd.submit_and_wait().unwrap();
        Out {
            update,
            mask: read_download(mask),
            field: read_download(field),
            height: height.map_or_else(Vec::new, read_download),
        }
    }
}

fn with_host(body: impl for<'d> FnOnce(&mut Host<'d>, &(dyn Fn() -> Host<'d> + 'd))) {
    with_chiaro_pack(|gpu, pack| {
        let fresh = || Host::new(gpu, pack);
        body(&mut fresh(), &fresh);
    });
}

fn pan(p: &mut Projection, dx: f32, dy: f32) {
    let o = p.origin.raw();
    p.origin = At::<Screen, Vec2>::new(Vec2::new(o.x + dx, o.y + dy));
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
        scene_mask_into(&mut store);
        let out = host.frame(&store, false);
        assert_eq!(out.update, MaskUpdate::Full);
        assert_same_bytes(
            "full mask",
            &out.mask,
            &golden_bytes("field", "full.mask.bin"),
        );
        assert_same_bytes(
            "full field",
            &out.field,
            &golden_bytes("field", "full.field.bin"),
        );
    });
}

/// The frames of `field_ref.cpp`: full, reused, a whole-pixel pan with the bar moved, a
/// fractional pan.
/// `sprite_height` of `tests/golden/chiaro/occluder/mask_ref.cpp`: a finite obstacle (height
/// mode) and a sprite mask with an opaque block over it, a block at alpha 128 (occludes) and one
/// at alpha 127 (does not).
#[test]
fn the_sprite_mask_in_height_mode_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        polygon(
            &mut store,
            &rect_xy(10.0, 10.0, 30.0, 30.0),
            ObstacleCommon {
                height: 3.0,
                base_height: 1.0,
                ..Default::default()
            },
        );
        let mut px = vec![0u8; 128 * 64 * 4];
        let blocks = [
            (20, 20, 40, 34, 255),
            (60, 8, 72, 40, 128),
            (90, 30, 110, 50, 127),
        ];
        for (x0, y0, x1, y1, alpha) in blocks {
            for y in y0..y1 {
                for x in x0..x1 {
                    px[(y * 128 + x) * 4 + 3] = alpha;
                }
            }
        }
        let sprite = sampled_rgba8(host.gpu, 128, 64, &px);
        let out = host.frame_with_sprite(&store, true, Some(sprite.texture_ref()));
        assert_same_bytes(
            "sprite_height mask",
            &out.mask,
            &golden_bytes("occluder", "sprite_height.mask.bin"),
        );
        assert_same_bytes(
            "sprite_height height",
            &out.height,
            &golden_bytes("occluder", "sprite_height.height.bin"),
        );
    });
}

#[test]
fn the_cached_sequence_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host, _| {
        let mut store = ObstacleStore::new();
        let bar = scene_mask_into(&mut store);
        let want: Vec<MaskUpdate> = String::from_utf8(golden_bytes("field", "updates.txt"))
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
        assert_same_bytes(
            "pan_whole mask",
            &whole.mask,
            &golden_bytes("field", "pan_whole.mask.bin"),
        );
        assert_same_bytes(
            "pan_whole field",
            &whole.field,
            &golden_bytes("field", "pan_whole.field.bin"),
        );

        pan(&mut host.projection, -1.4, -0.6);
        let fraction = host.frame(&store, false);
        got.push(fraction.update);
        assert_same_bytes(
            "pan_fraction mask",
            &fraction.mask,
            &golden_bytes("field", "pan_fraction.mask.bin"),
        );
        assert_same_bytes(
            "pan_fraction field",
            &fraction.field,
            &golden_bytes("field", "pan_fraction.field.bin"),
        );
        assert_eq!(got, want);
    });
}

#[test]
fn an_incremental_update_equals_a_full_rebuild_of_the_same_state() {
    with_host(|host, fresh| {
        let mut store = ObstacleStore::new();
        let bar = scene_mask_into(&mut store);
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
            assert_same_bytes("incremental mask", &cached.mask, &full.mask);
            assert_same_bytes("incremental field", &cached.field, &full.field);
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
        scene_mask_into(&mut store);
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
        assert_same_bytes("height mask", &cached.mask, &full.mask);
        assert_same_bytes("height field", &cached.field, &full.field);
        assert_same_bytes("height texture", &cached.height, &full.height);

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
        let bar = scene_mask_into(&mut store);
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
        scene_mask_into(&mut store);
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
        let mut cmd = CommandBuffer::acquire(host.gpu.device()).unwrap();
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
        scene_mask_into(&mut store);
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
