//! P22 on a real GPU: the obstacles of a scene drawn into the occlusion mask by the stages of
//! `knorke_light2d::occluder_raster`, read back and checked texel by texel.
//!
//! The expectations are the ones of Chiaro's own mask tests (chiaro @ 9bc5b9b,
//! tests/gpu/OcclusionTests.cpp, zlib license, see `src/occluder_raster/`): the same scenes
//! (128x64 scene, identity projection, 32-texel margin band, so the mask is 192x128 and scene
//! pixel (x, y) is mask texel (x + 32, y + 32)) and the same texels. A second oracle, written
//! independently of the shaders, rasterizes the triangles on the CPU under a skewed projection.
//!
//! The test plays the host (ADR 0002): it owns device, mask and height textures and command
//! buffers, opens the passes, submits and reads back. It skips (passes with a note) without a
//! Vulkan device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

mod common;

use common::{at, polygon, rect_xy, scene_mask};
use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER};
use knorke_gpu::test_support::{
    assert_same_bytes, golden_bytes, half_to_f32, queue_download, read_download, with_chiaro_pack,
};
use knorke_gpu::{
    ColorTarget, CommandBuffer, Filter, Gpu, Load, PipelineCache, Samplers, ShaderLibrary, Texture,
    TextureDesc,
};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::occluder_raster::{
    DomainParams, HEIGHT_FORMAT, MASK_FORMAT, NoPatterns, OCCLUDER_PROGRAMS, OccluderGeometry,
    OccluderPrograms, TileOccluders, thin_circle_radius,
};
use knorke_light2d::shadow::{CellRect, MaskAnchor, OcclusionDomain};
use knorke_light2d::{ObstacleId, ObstacleStore, Projection};
use knorke_math::{At, Vec2};

/// What one frame produced.
struct Out {
    /// R8G8 mask, row-major, `[opacity, footprint]` per texel.
    mask: Vec<u8>,
    /// RGBA16F height texture as raw halves.
    heights: Vec<u16>,
    width: usize,
    uploads: u32,
    draws: u32,
}

impl Out {
    fn at(&self, x: usize, y: usize) -> u8 {
        self.mask[(y * self.width + x) * 2]
    }

    fn footprint(&self, x: usize, y: usize) -> u8 {
        self.mask[(y * self.width + x) * 2 + 1]
    }

    /// Height texel at world (x, y) for the 128x64 scene: (-base, top, finite -base, finite top).
    fn height(&self, x: usize, y: usize) -> [f32; 4] {
        let i = ((y + 32) * self.width + x + 32) * 4;
        [0, 1, 2, 3].map(|c| half_to_f32(self.heights[i + c]))
    }
}

/// The host: shaders, pipelines, the mask and height textures and the two stages.
struct Host<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    geometry: OccluderGeometry<'d>,
    tiles: TileOccluders<'d>,
    mask: Texture<'d>,
    height: Texture<'d>,
    projection: Projection,
    domain: OcclusionDomain,
}

impl<'d> Host<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        Self::with_projection(gpu, pack, Projection::IDENTITY)
    }

    fn with_projection(gpu: Gpu<'d>, pack: &[u8], p: Projection) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let pipelines = PipelineCache::new(&shaders, &OCCLUDER_PROGRAMS).unwrap();
        let domain = OcclusionDomain::new(128, 64, 1.0);
        let usage = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
        let texture = |format| {
            Texture::new(
                gpu,
                TextureDesc::new(domain.width, domain.height, format, usage),
            )
            .unwrap()
        };
        Self {
            gpu,
            shaders,
            pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            geometry: OccluderGeometry::new(gpu, OccluderPrograms::default()),
            tiles: TileOccluders::new(gpu, OccluderPrograms::default()),
            mask: texture(MASK_FORMAT),
            height: texture(HEIGHT_FORMAT),
            projection: p,
            domain,
        }
    }

    fn frame(&mut self, store: &ObstacleStore) -> Out {
        let anchor = MaskAnchor::compute(&self.projection, &self.domain);
        let params = DomainParams::new(&self.projection, &self.domain, &anchor);
        let min_radius = thin_circle_radius(&self.projection, &self.domain);
        self.geometry
            .prepare(
                &mut self.shaders,
                &mut self.pipelines,
                store,
                &NoPatterns,
                min_radius,
            )
            .unwrap();
        self.tiles
            .prepare(&mut self.shaders, &mut self.pipelines, store, &NoPatterns)
            .unwrap();

        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        let mut uploads = 0;
        {
            let mut copy = cmd.copy_pass().unwrap();
            uploads += self.geometry.record_uploads(&mut copy).unwrap();
            uploads += self.tiles.record_uploads(&mut copy).unwrap();
        }
        let mut draws = 0;
        {
            let targets = [
                ColorTarget::new(self.mask.texture_ref(), Load::Clear([0.0; 4])),
                ColorTarget::new(self.height.texture_ref(), Load::Clear([-65504.0; 4])),
            ];
            let mut pass = cmd.render_pass(&targets).unwrap();
            draws += self
                .geometry
                .record_draws(&mut pass, &self.pipelines, &params, true, true)
                .unwrap();
            draws += self
                .tiles
                .record_draws(
                    &mut pass,
                    &self.pipelines,
                    self.samplers.sampler(Filter::Nearest),
                    &params,
                )
                .unwrap();
        }
        let mask_down = queue_download(self.gpu, &mut cmd, self.mask.texture_ref());
        let height_down = queue_download(self.gpu, &mut cmd, self.height.texture_ref());
        cmd.submit_and_wait().unwrap();
        let mask = read_download(mask_down);
        let heights = read_download(height_down)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        Out {
            mask,
            heights,
            width: self.domain.width as usize,
            uploads,
            draws,
        }
    }
}

/// A texel at world (x, y) of the 128x64 scene, in mask coordinates.
fn m(v: usize) -> usize {
    v + 32
}

fn square() -> [Vec2; 4] {
    rect_xy(48.0, 24.0, 56.0, 40.0)
}

fn with_tiles(
    store: &mut ObstacleStore,
    cells: &[u8],
    columns: u32,
    rows: u32,
    size: f32,
) -> ObstacleId {
    let mut d = TileGridDesc::new(columns, rows, cells);
    d.cell_size = Vec2::new(size, size);
    store.create_tile_grid(&d).unwrap()
}

/// Runs `body` with a host, or skips; checks that nothing leaks afterwards.
fn with_host(body: impl for<'d> FnOnce(&mut Host<'d>)) {
    with_chiaro_pack(|gpu, pack| {
        let mut host = Host::new(gpu, pack);
        body(&mut host);
    });
}

#[test]
fn a_static_polygon_is_rasterized_into_the_mask_with_the_margin_band() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        polygon(&mut store, &square(), ObstacleCommon::default());
        let out = host.frame(&store);
        assert_eq!((out.width, out.mask.len()), (192, 192 * 128 * 2));
        assert_eq!(out.at(m(52), m(32)), 255);
        assert_eq!(out.at(m(48), m(24)), 255);
        assert_eq!(out.at(m(55), m(39)), 255);
        assert_eq!(out.at(m(40), m(32)), 0);
        assert_eq!(out.at(m(47), m(32)), 0);
        assert_eq!(out.at(m(56), m(32)), 0);
        assert_eq!(out.at(m(52), m(23)), 0);
        assert_eq!(out.at(m(52), m(40)), 0);
        assert_eq!(out.at(0, 0), 0);
        assert_eq!(out.uploads, 1);
        assert_eq!(out.draws, 1);

        // Unchanged: nothing is uploaded again, the same image is drawn.
        let again = host.frame(&store);
        assert_eq!(again.uploads, 0);
        assert_eq!(again.mask, out.mask);
    });
}

#[test]
fn obstacle_rotation_turns_a_thin_rectangle_onto_the_diagonal() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let common = ObstacleCommon {
            position: at(64.0, 32.0),
            rotation: 45.0,
            ..Default::default()
        };
        polygon(&mut store, &rect_xy(-40.0, -1.5, 40.0, 1.5), common);
        let out = host.frame(&store);
        // Texel centres with x - 64 == y - 32 lie exactly on the rotated axis.
        assert_eq!(out.at(m(64), m(32)), 255);
        assert_eq!(out.at(m(78), m(46)), 255);
        assert_eq!(out.at(m(50), m(18)), 255);
        // The unrotated rectangle and the other diagonal stay empty.
        assert_eq!(out.at(m(90), m(32)), 0);
        assert_eq!(out.at(m(78), m(18)), 0);
    });
}

#[test]
fn a_tile_grid_rasterizes_its_solid_cells() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let mut cells = vec![0u8; 16 * 8];
        cells[3 * 16 + 6] = 1; // non-zero is solid, not just 255
        with_tiles(&mut store, &cells, 16, 8, 8.0);
        let out = host.frame(&store);
        assert_eq!(out.uploads, 1); // the cell texture; no static triangles
        for y in 24..=31 {
            for x in 48..=55 {
                assert_eq!(out.at(m(x), m(y)), 255, "({x}, {y})");
            }
        }
        assert_eq!(out.at(m(47), m(28)), 0);
        assert_eq!(out.at(m(56), m(28)), 0);
        assert_eq!(out.at(m(52), m(23)), 0);
        assert_eq!(out.at(m(52), m(32)), 0);
    });
}

#[test]
fn a_moved_and_rotated_tile_grid_follows_its_transform_without_a_new_upload() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let mut cells = vec![0u8; 4 * 2];
        cells[0] = 255; // cell (0, 0)
        let tiles = with_tiles(&mut store, &cells, 4, 2, 8.0);
        let out = host.frame(&store);
        assert_eq!(out.uploads, 1);
        assert_eq!(out.at(m(4), m(4)), 255);

        // Rotating by 90 degrees about (64, 32) maps local (x, y) to (64 - y, 32 + x): cell
        // (0, 0) covers 56..63 x 32..39.
        store.set_transform(tiles, at(64.0, 32.0), 90.0).unwrap();
        let out = host.frame(&store);
        assert_eq!(out.at(m(60), m(36)), 255);
        assert_eq!(out.at(m(4), m(4)), 0);
        assert_eq!(out.at(m(60), m(44)), 0); // cell (1, 0) is empty
        assert_eq!(out.uploads, 0); // a transform is a uniform, not an upload
    });
}

#[test]
fn the_mask_stores_the_opacity_of_every_obstacle_texel() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let fence = rect_xy(8.0, 8.0, 24.0, 16.0);
        let overlap = rect_xy(16.0, 8.0, 32.0, 16.0);
        let still = ObstacleCommon::default();
        let moving = ObstacleCommon {
            dynamic: true,
            ..still
        };
        let opacity = |c: ObstacleCommon, opacity| ObstacleCommon { opacity, ..c };
        polygon(&mut store, &fence, opacity(still, 0.5));
        polygon(&mut store, &overlap, opacity(moving, 1.0));
        polygon(&mut store, &square(), opacity(moving, 0.25));
        let cells = [1u8, 1, 1, 0];
        let cell_opacity = [255u8, 77, 0, 255];
        let mut t = TileGridDesc::new(4, 1, &cells);
        t.opacity = Some(&cell_opacity);
        t.cell_size = Vec2::new(8.0, 8.0);
        t.common.position = at(64.0, 48.0);
        let tiles = store.create_tile_grid(&t).unwrap();

        let out = host.frame(&store);
        let near = |got: u8, want: i32| (i32::from(got) - want).abs() <= 1;
        assert!(near(out.at(m(10), m(12)), 128)); // fence alone
        assert_eq!(out.at(m(20), m(12)), 255); // fence under the opaque overlap
        assert!(near(out.at(m(52), m(32)), 64)); // the quarter square
        assert_eq!(out.at(m(68), m(52)), 255);
        assert!(near(out.at(m(76), m(52)), 77));
        assert_eq!(out.at(m(84), m(52)), 0); // opacity 0: no obstacle texel
        assert_eq!(out.at(m(92), m(52)), 0); // empty cell

        // The obstacle opacity scales the cell opacities.
        t.common.opacity = 0.5;
        store.set_tile_grid(tiles, &t).unwrap();
        let out = host.frame(&store);
        assert!(near(out.at(m(68), m(52)), 128));
        assert!(near(out.at(m(76), m(52)), 39));
    });
}

#[test]
fn a_footprint_obstacle_marks_the_second_channel() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let common = ObstacleCommon {
            opacity: 0.5,
            shadow_footprint: true,
            ..Default::default()
        };
        polygon(&mut store, &square(), common);
        polygon(
            &mut store,
            &rect_xy(8.0, 8.0, 16.0, 16.0),
            ObstacleCommon::default(),
        );
        let out = host.frame(&store);
        assert_eq!(
            (out.at(m(52), m(32)), out.footprint(m(52), m(32))),
            (128, 255)
        );
        assert_eq!(
            (out.at(m(12), m(12)), out.footprint(m(12), m(12))),
            (255, 0)
        );
    });
}

#[test]
fn an_obstacle_with_opacity_zero_draws_nothing_like_a_disabled_one() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        polygon(
            &mut store,
            &square(),
            ObstacleCommon {
                opacity: 0.0,
                ..Default::default()
            },
        );
        polygon(
            &mut store,
            &square(),
            ObstacleCommon {
                enabled: false,
                dynamic: true,
                ..Default::default()
            },
        );
        let out = host.frame(&store);
        assert_eq!((out.draws, out.uploads), (0, 0));
        assert!(out.mask.iter().all(|&b| b == 0));
    });
}

#[test]
fn circles_thinner_than_a_texel_still_cast_a_shadow() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        // 0.2 around (64.25, 32.25): the texel centre (64.5, 32.5) is 0.35 away, outside the
        // circle, inside the widened one (0.5).
        let common = ObstacleCommon {
            position: at(64.25, 32.25),
            ..Default::default()
        };
        store
            .create_shape(&ShapeDesc {
                common,
                shape: Shape::Circle { radius: 0.2 },
            })
            .unwrap();
        let out = host.frame(&store);
        assert_eq!(host.geometry.thin_count(), 1);
        assert_eq!(out.at(m(64), m(32)), 255);
    });
}

#[test]
fn dynamic_geometry_grows_its_buffer_once_and_then_stays_stable() {
    with_chiaro_pack(|gpu, pack| {
        let counter = gpu.counter();
        let mut host = Host::new(gpu, pack);
        let mut store = ObstacleStore::new();
        // Many dynamic circles: far beyond the initial 4096 floats of the buffer.
        for i in 0..100 {
            let common = ObstacleCommon {
                position: at(i as f32, 10.0),
                dynamic: true,
                ..Default::default()
            };
            store
                .create_shape(&ShapeDesc {
                    common,
                    shape: Shape::Circle { radius: 1.0 },
                })
                .unwrap();
        }
        let out = host.frame(&store);
        assert_eq!(out.at(m(50), m(10)), 255);
        let live = counter.live();
        for _ in 0..3 {
            let again = host.frame(&store);
            assert_eq!(again.uploads, 0, "dynamic uploads do not count");
            assert_eq!(again.mask, out.mask);
        }
        assert_eq!(counter.live(), live);
    });
}

#[test]
fn a_destroyed_tile_grid_releases_its_texture() {
    with_chiaro_pack(|gpu, pack| {
        let counter = gpu.counter();
        let mut host = Host::new(gpu, pack);
        let mut store = ObstacleStore::new();
        polygon(&mut store, &square(), ObstacleCommon::default());
        host.frame(&store);
        host.frame(&store);
        let live = counter.live();

        let tiles = with_tiles(&mut store, &[1; 4], 2, 2, 4.0);
        let out = host.frame(&store);
        let with_tiles = counter.live();
        assert!(with_tiles > live);
        assert_eq!(out.at(m(2), m(2)), 255);

        store.destroy(tiles).unwrap();
        let out = host.frame(&store);
        assert_eq!(out.at(m(2), m(2)), 0);
        // The texture is gone; the transfer buffer for cell uploads stays for reuse.
        assert_eq!(counter.live(), with_tiles - 1);
    });
}

#[test]
fn edited_cells_upload_only_their_bounding_box_and_give_the_same_image() {
    with_chiaro_pack(|gpu, pack| {
        let mut host = Host::new(gpu, pack);
        let mut store = ObstacleStore::new();
        let id = with_tiles(&mut store, &[1; 16 * 8], 16, 8, 8.0);
        host.frame(&store);

        store
            .set_tile_cells(id, CellRect::new(2, 2, 3, 1), Some(&[0, 0, 0]), None, None)
            .unwrap();
        let slot = host.tiles.len();
        assert_eq!(slot, 1);
        let edited = host.frame(&store);
        assert_eq!(edited.uploads, 1);
        assert_eq!(edited.at(m(24), m(20)), 0); // cell (3, 2) is empty now
        assert_eq!(edited.at(m(8), m(20)), 255); // cell (1, 2) next to it is not
        assert_eq!(edited.at(m(48), m(20)), 255); // cell (6, 2)

        // A host that never saw the old grid draws the same image from scratch.
        let mut fresh = Host::new(gpu, pack);
        let whole = fresh.frame(&store);
        assert_eq!(whole.mask, edited.mask);
    });
}

#[test]
fn edge_strips_draw_only_the_named_edges_of_a_cell() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let cells = [1u8, 1];
        let edges = [1u8, 0]; // cell 0: strip along local x low; cell 1: whole
        let mut d = TileGridDesc::new(2, 1, &cells);
        d.cell_size = Vec2::new(8.0, 8.0);
        d.edge_width = 2.0;
        d.edges = Some(&edges);
        store.create_tile_grid(&d).unwrap();
        let out = host.frame(&store);
        assert_eq!(out.at(m(1), m(4)), 255); // inside the strip (x < 2)
        assert_eq!(out.at(m(3), m(4)), 0);
        assert_eq!(out.at(m(6), m(4)), 0);
        assert_eq!(out.at(m(9), m(4)), 255); // cell 1 is whole
        assert_eq!(out.at(m(14), m(4)), 255);
    });
}

// ---- heights ----

#[test]
fn the_height_texture_holds_each_obstacles_interval() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let tall = |height, base| ObstacleCommon {
            height,
            base_height: base,
            ..Default::default()
        };
        polygon(&mut store, &rect_xy(10.0, 10.0, 20.0, 20.0), tall(3.0, 0.0));
        store
            .create_shape(&ShapeDesc {
                common: ObstacleCommon {
                    position: at(60.0, 30.0),
                    ..tall(5.0, 2.0)
                },
                shape: Shape::Circle { radius: 6.0 },
            })
            .unwrap();
        polygon(
            &mut store,
            &rect_xy(90.0, 10.0, 100.0, 50.0),
            ObstacleCommon::default(),
        );
        let out = host.frame(&store);
        assert_eq!(out.height(15, 15)[..2], [0.0, 3.0]);
        assert_eq!(out.height(60, 30)[..2], [-2.0, 5.0]);
        assert_eq!(out.height(95, 30)[..2], [0.0, 65504.0]);
        assert_eq!(out.height(40, 50), [-65504.0; 4]);
        // The mask is drawn alongside.
        assert_eq!(out.at(m(15), m(15)), 255);
    });
}

#[test]
fn overlapping_obstacles_merge_into_the_enclosing_interval() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let common = |height, base, dynamic| ObstacleCommon {
            height,
            base_height: base,
            dynamic,
            ..Default::default()
        };
        polygon(
            &mut store,
            &rect_xy(10.0, 10.0, 30.0, 30.0),
            common(2.0, 1.0, false),
        );
        polygon(
            &mut store,
            &rect_xy(20.0, 20.0, 40.0, 40.0),
            common(6.0, 3.0, true),
        );
        let out = host.frame(&store);
        assert_eq!(out.height(25, 25)[..2], [-1.0, 6.0]);
        assert_eq!(out.height(15, 15)[..2], [-1.0, 2.0]);
    });
}

#[test]
fn an_infinite_obstacle_over_a_finite_one_keeps_the_finite_interval_in_the_last_channels() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let finite = ObstacleCommon {
            height: 6.0,
            base_height: 1.0,
            ..Default::default()
        };
        polygon(&mut store, &rect_xy(10.0, 10.0, 30.0, 30.0), finite);
        polygon(
            &mut store,
            &rect_xy(20.0, 10.0, 25.0, 30.0),
            ObstacleCommon::default(),
        );
        let out = host.frame(&store);
        assert_eq!(out.height(22, 20), [0.0, 65504.0, -1.0, 6.0]);
        assert_eq!(out.height(22, 35)[3], -65504.0);
        assert_eq!(out.height(40, 50)[2..], [-65504.0; 2]);
    });
}

#[test]
fn tile_cells_carry_their_own_heights_and_bottoms() {
    with_host(|host| {
        let mut store = ObstacleStore::new();
        let cells = [0u8, 1, 3, 4];
        let base = [9u8, 0, 2, 0]; // a bridge deck (2..3) next to solid cells; ignored where empty
        let mut d = TileGridDesc::new(2, 2, &cells);
        d.base_levels = Some(&base);
        d.cell_size = Vec2::new(10.0, 10.0);
        d.common.position = at(10.0, 10.0);
        d.level_height = 2.0;
        store.create_tile_grid(&d).unwrap();
        let out = host.frame(&store);
        assert_eq!(out.height(25, 15)[..2], [0.0, 2.0]);
        assert_eq!(out.height(15, 25)[..2], [-4.0, 6.0]);
        assert_eq!(out.height(25, 25)[..2], [0.0, 8.0]);
        assert_eq!(out.height(15, 15)[1], -65504.0);
        // The mask itself keeps only the solid cells.
        assert_eq!(out.at(m(25), m(15)), 255);
        assert_eq!(out.at(m(15), m(15)), 0);
    });
}

// ---- images against C++ Chiaro ----

/// `scene_height` of `mask_ref.cpp`: overlapping finite obstacles, an infinite one, a circle
/// with a bottom and a grid with its own levels and bottoms.
fn scene_height() -> ObstacleStore {
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
    polygon(
        &mut store,
        &rect_xy(20.0, 20.0, 40.0, 40.0),
        tall(6.0, 3.0, true),
    );
    polygon(
        &mut store,
        &rect_xy(50.0, 10.0, 60.0, 30.0),
        tall(0.0, 0.0, false),
    );
    store
        .create_shape(&ShapeDesc {
            common: ObstacleCommon {
                position: at(80.0, 30.0),
                ..tall(5.0, 2.0, false)
            },
            shape: Shape::Circle { radius: 6.0 },
        })
        .unwrap();
    let solid = [0u8, 1, 3, 4, 2, 0];
    let base = [0u8, 0, 2, 0, 1, 0];
    let mut grid = TileGridDesc::new(3, 2, &solid);
    grid.cell_size = Vec2::new(10.0, 10.0);
    grid.level_height = 2.0;
    grid.base_levels = Some(&base);
    grid.common.position = at(84.0, 38.0);
    store.create_tile_grid(&grid).unwrap();
    store
}

#[test]
fn the_mask_scene_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host| {
        let out = host.frame(&scene_mask());
        let lit = out
            .mask
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|t| t[0] > 0)
            .count();
        assert!(
            lit > 1500,
            "the scene should light a good part of the mask, got {lit}"
        );
        assert_same_bytes(
            "mask_scene",
            &out.mask,
            &golden_bytes("occluder", "mask_scene.mask.bin"),
        );
    });
}

#[test]
fn the_height_scene_matches_cpp_chiaro_texel_for_texel() {
    with_host(|host| {
        let out = host.frame(&scene_height());
        assert_same_bytes(
            "height_scene mask",
            &out.mask,
            &golden_bytes("occluder", "height_scene.mask.bin"),
        );
        let bytes: Vec<u8> = out.heights.iter().flat_map(|h| h.to_le_bytes()).collect();
        assert_same_bytes(
            "height_scene height",
            &bytes,
            &golden_bytes("occluder", "height_scene.height.bin"),
        );
    });
}

// ---- an oracle independent of the shaders ----

/// Rasterizes triangles on the CPU: texel centres clearly inside (by `margin` texels from every
/// edge) get `Some(true)`, clearly outside `Some(false)`, the ambiguous ones `None`.
fn reference(
    store: &ObstacleStore,
    p: &Projection,
    d: &OcclusionDomain,
    a: &MaskAnchor,
) -> Vec<Option<bool>> {
    let (w, h) = (d.width as usize, d.height as usize);
    let origin = p.origin.raw();
    let to_texel = |v: Vec2| {
        let sx = f64::from(origin.x)
            + f64::from(v.x) * f64::from(p.x_axis.x)
            + f64::from(v.y) * f64::from(p.y_axis.x);
        let sy = f64::from(origin.y)
            + f64::from(v.x) * f64::from(p.x_axis.y)
            + f64::from(v.y) * f64::from(p.y_axis.y);
        (
            (sx + f64::from(d.margin)) * f64::from(d.texels_per_scene[0])
                - f64::from(a.remainder[0]),
            (sy + f64::from(d.margin)) * f64::from(d.texels_per_scene[1])
                - f64::from(a.remainder[1]),
        )
    };
    let mut triangles = Vec::new();
    for (id, o) in store.iter() {
        if o.is_active() && !o.is_tile_grid() {
            let mut world = Vec::new();
            store.append_world_triangles(id, &mut world);
            for t in world.as_chunks::<3>().0 {
                triangles.push([
                    to_texel(t[0].raw()),
                    to_texel(t[1].raw()),
                    to_texel(t[2].raw()),
                ]);
            }
        }
    }
    let margin = 0.05;
    let mut out = vec![Some(false); w * h];
    for y in 0..h {
        for x in 0..w {
            let c = (x as f64 + 0.5, y as f64 + 0.5);
            let mut state = Some(false);
            for t in &triangles {
                let area =
                    (t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[1].1 - t[0].1) * (t[2].0 - t[0].0);
                if area == 0.0 {
                    continue;
                }
                // Signed distance to each edge, positive inside.
                let mut inside = f64::INFINITY;
                for i in 0..3 {
                    let (p0, p1) = (t[i], t[(i + 1) % 3]);
                    let len =
                        ((p1.0 - p0.0) * (p1.0 - p0.0) + (p1.1 - p0.1) * (p1.1 - p0.1)).sqrt();
                    let e = ((p1.0 - p0.0) * (c.1 - p0.1) - (p1.1 - p0.1) * (c.0 - p0.0)) / len
                        * area.signum();
                    if e < inside {
                        inside = e;
                    }
                }
                if inside > margin {
                    state = Some(true);
                    break;
                } else if inside > -margin {
                    state = None;
                }
            }
            out[y * w + x] = state;
        }
    }
    out
}

#[test]
fn a_skewed_projection_agrees_with_a_cpu_rasterizer() {
    with_chiaro_pack(|gpu, pack| {
        // An isometric-like view with a fractional origin: exercises the axes, the margin and
        // the anchor remainder of the vertex shader.
        let projection = Projection {
            origin: At::new(Vec2::new(60.4, 7.3)),
            x_axis: Vec2::new(1.0, 0.5),
            y_axis: Vec2::new(-1.0, 0.5),
            z_axis: Vec2::ZERO,
        };
        let mut host = Host::with_projection(gpu, pack, projection);
        let mut store = ObstacleStore::new();
        let tri = [
            Vec2::new(0.0, 0.0),
            Vec2::new(30.0, 4.0),
            Vec2::new(8.0, 22.0),
        ];
        polygon(&mut store, &tri, ObstacleCommon::default());
        let quad = [
            Vec2::new(-6.0, -3.0),
            Vec2::new(9.0, -5.0),
            Vec2::new(12.0, 7.0),
            Vec2::new(-4.0, 9.0),
        ];
        let common = ObstacleCommon {
            position: at(25.0, 18.0),
            rotation: 33.0,
            dynamic: true,
            ..Default::default()
        };
        polygon(&mut store, &quad, common);
        store
            .create_shape(&ShapeDesc {
                common: ObstacleCommon {
                    position: at(14.0, 30.0),
                    ..Default::default()
                },
                shape: Shape::Circle { radius: 5.0 },
            })
            .unwrap();

        let out = host.frame(&store);
        let anchor = MaskAnchor::compute(&host.projection, &host.domain);
        let want = reference(&store, &host.projection, &host.domain, &anchor);
        let (mut checked, mut covered) = (0usize, 0usize);
        for (i, w) in want.iter().enumerate() {
            let Some(inside) = w else { continue };
            checked += 1;
            covered += usize::from(*inside);
            let got = out.mask[i * 2] > 0;
            assert_eq!(got, *inside, "texel ({}, {})", i % out.width, i / out.width);
        }
        eprintln!("CPU oracle: {checked} texels compared, {covered} covered");
        assert!(
            covered > 500,
            "the scene should cover a good part of the mask"
        );
    });
}
