//! Contract: the steady-state paths of `knorke-light2d` allocate nothing once their buffers have
//! grown: transient frame lights, snapshot save and restore of the light store, triangulation
//! into a warm buffer, moving and re-describing obstacles with their dirty tracking, snapshot save
//! of the obstacle store, the pixel and normal-map codecs, collecting the lights of a frame (with
//! animations, masks, flicker and caster filters) into a reused list, and snapshot round trips
//! of the animation and mask stores, the shadow cache plan deciding incremental mask updates, and
//! the obstacle raster collecting triangles, laying out the buffer and staging tile cells,
//! the lights and suns of the medium stage, the light pass's uniform blocks, filling the profile
//! and mask atlas bytes, and culling the ambient zones, writing their records, vertex texels and
//! uniform blocks, and the light rows assigning rows and sun maps and writing their blocks
//! (ADR 0006, ADR 0010, ADR 0019).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_culling::{AmbientZoneList, CullScene, collect_ambient_zones};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::animation::{AnimationDesc, AnimationType};
use knorke_light2d::atlas::{
    fill_mask_atlas, fill_profile_atlas, mask_atlas_bytes, mask_atlas_rows, profile_atlas_bytes,
    profile_atlas_rows,
};
use knorke_light2d::codec::{
    Material, MaterialPreset, NormalMapOptions, convert_normal_map, encode_normal, material_pixel,
    surface_pixel,
};
use knorke_light2d::composite::{
    CompositeParams, CompositeParamsInput, InputColorSpace, composite_layout_of,
    composite_program_of,
};
use knorke_light2d::light::{Height, LightDesc, LightType, ShadowFilter};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::lighting::{LightSurfacePassParams, ShadowDomain, SurfaceParams};
use knorke_light2d::mask::{MaskPreset, MaskShape};
use knorke_light2d::medium_culling::{
    MediumLightList, MediumSun, collect_medium_lights, collect_medium_suns,
};
use knorke_light2d::obstacle::{ShapeDesc, TileGridDesc};
use knorke_light2d::occluder_raster::{
    NoPatterns, TileSet, TriangleSet, write_cell_texels, write_triangle_buffer,
};
use knorke_light2d::profile::{ProfilePreset, ProfileShape, build_shape_profile};
use knorke_light2d::profile_store::ProfileStore;
use knorke_light2d::rows::{
    MaskChange, Refresh, RowBuildParams, RowCache, RowEdgeParams, RowLamp, RowLookupParams,
    RowMask, RowSpace, SunBands, SunCache, SunParams, SunSource, height_code, sun_bands, sun_plain,
};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::shadow::{MaskPlanInput, MaskUpdatePlan, OcclusionDomain};
use knorke_light2d::zone_pass::{
    ZoneFragParams, ZonePassParams, fill_zone_vertices, write_zones, zone_vertex_bytes,
    zone_vertex_rows,
};
use knorke_light2d::{
    AnimationStore, LightStore, MaskStore, NoResources, ObstacleStore, Projection, StoreResources,
    Triangulator,
};
use knorke_math::{At, Vec2, Vec3, World2};
use knorke_types::{Reader, SnapshotBuf, restore};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_light2d_paths_allocate_nothing() {
    // Persistent lights, a snapshot buffer, frame lights, a polygon and a small normal map.
    let mut store = LightStore::new();
    let mut ids = Vec::new();
    for i in 0..32 {
        let mut d = LightDesc::new(LightType::Spot);
        d.position = At::new(Vec2::new(i as f32, 0.0));
        ids.push(store.create(&d, &NoResources).unwrap());
    }
    let frame: Vec<LightDesc> = (0..64).map(|_| LightDesc::new(LightType::Point)).collect();
    let mut snap = SnapshotBuf::new();
    // Animations, masks and lights that use them, collected every step.
    let mut animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mut texels = Vec::new();
    MaskShape::preset(MaskPreset::WindowCross).build(&mut texels);
    let mask = masks.create(texels).unwrap();
    let anims: Vec<_> = AnimationType::ALL
        .iter()
        .map(|&k| animations.create(&AnimationDesc::new(k)).unwrap())
        .collect();
    let mut scene = LightStore::new();
    {
        let res = StoreResources::new(&animations, &masks);
        for i in 0..24usize {
            let mut d = LightDesc::new(match i % 4 {
                0 => LightType::Point,
                1 => LightType::Spot,
                2 => LightType::Line,
                _ => LightType::Directional,
            });
            d.position = At::new(Vec2::new(4.0 * i as f32, 20.0));
            d.radius = 30.0;
            d.flicker = 0.3;
            d.animation = Some(anims[i % anims.len()]);
            if i % 4 == 1 {
                d.mask = Some(mask);
                d.direction = Vec3::new(0.0, 0.0, -1.0);
                d.height = Height::At(5.0);
            }
            if i % 8 == 0 {
                d.shadow_filter = ShadowFilter::DynamicOnly;
            }
            scene.create(&d, &res).unwrap();
        }
        scene.set_frame_lights(&frame[..], &res).unwrap();
    }
    let mut list = LightList::default();
    let mut medium_list = MediumLightList::default();
    let mut medium_suns: Vec<MediumSun> = Vec::new();
    let mut anim_snap = SnapshotBuf::new();
    let mut mask_snap = SnapshotBuf::new();
    let poly = [
        Vec2::new(0.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(4.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 4.0),
        Vec2::new(0.0, 4.0),
    ];
    let mut tri = Triangulator::new();
    let mut triangles = Vec::new();
    let normal_map = vec![200u8; 16 * 16 * 4];
    let mut material_map = vec![0u8; 16 * 16 * 4];

    // Obstacles: polygons, a box, a circle and a tile grid; a warm world-triangle buffer.
    let mut obstacles = ObstacleStore::new();
    let mut moving = Vec::new();
    for i in 0u32..8 {
        let mut d = ShapeDesc::polygon(&poly[..]);
        d.common.dynamic = i.is_multiple_of(2);
        moving.push(obstacles.create_shape(&d).unwrap());
    }
    moving.push(obstacles.create_shape(&ShapeDesc::rect(2.0, 1.0)).unwrap());
    moving.push(obstacles.create_shape(&ShapeDesc::circle(1.5)).unwrap());
    let cells = [1u8, 2, 0, 3];
    let grid = obstacles
        .create_tile_grid(&TileGridDesc::new(2, 2, &cells))
        .unwrap();
    let mut obstacle_snap = SnapshotBuf::new();
    let mut world_triangles = Vec::new();
    // Obstacle raster (P22): triangle sets with intervals, the buffer layout, tile grid slots and
    // the cell texels of a pending upload.
    let mut static_set = TriangleSet::new();
    let mut dynamic_set = TriangleSet::new();
    let mut raster_bytes = vec![0u8; 1 << 14];
    let mut tile_set: TileSet<()> = TileSet::new();
    let mut texels = vec![0u8; 16];
    // Shadow cache plan: a static wall, dynamic obstacles that move and a panning camera.
    let mut shadow_obstacles = ObstacleStore::new();
    shadow_obstacles
        .create_shape(&ShapeDesc::rect(8.0, 1.0))
        .unwrap();
    let mut shadow_movers = Vec::new();
    for _ in 0..6 {
        let mut d = ShapeDesc::polygon(&poly[..]);
        d.common.dynamic = true;
        shadow_movers.push(shadow_obstacles.create_shape(&d).unwrap());
    }
    let mut plan = MaskUpdatePlan::new();
    plan.set_full_rebuild_ratio(1.0);

    // Atlases and ambient zones (P26): profile and mask atlas bytes, the zone list, the record
    // bytes, the vertex texels and the uniform blocks of the zone pass.
    let mut profiles = ProfileStore::new();
    for preset in [ProfilePreset::HatLamp, ProfilePreset::Downlight] {
        let table = build_shape_profile(&ProfileShape::preset(preset)).unwrap();
        profiles.create(table).unwrap();
    }
    let profile_rows = profile_atlas_rows(profiles.block_count());
    let mut profile_atlas = vec![0u8; profile_atlas_bytes(profile_rows) as usize];
    let mask_rows = mask_atlas_rows(masks.block_count());
    let mut mask_atlas = vec![0u8; mask_atlas_bytes(mask_rows) as usize];
    let mut zones = AmbientZoneStore::new();
    for k in 0..4 {
        let shape = if k % 2 == 0 {
            ZoneShape::Box
        } else {
            ZoneShape::Polygon
        };
        let params = AmbientZoneParams {
            shape,
            x: 10.0 + 12.0 * k as f32,
            y: 20.0,
            width: 8.0,
            depth: 8.0,
            vertices: poly.to_vec(),
            ambient: [0.2, 0.3, 0.4],
            ..Default::default()
        };
        zones.create(make_stored_zone(&params).unwrap()).unwrap();
    }
    let mut zone_list = AmbientZoneList::default();
    let zone_rows = zone_vertex_rows(zones.len() as u32 * 8);
    let mut zone_vertices = vec![0u8; zone_vertex_bytes(zone_rows) as usize];
    let mut zone_records = vec![0u8; 1 << 12];
    let zone_scene = CullScene {
        projection: Projection::IDENTITY,
        max_height: 0.0,
        has_surface: false,
        width: 128,
        height: 64,
    };

    let mut row_cache = RowCache::default();
    let mut sun_cache = SunCache::default();
    let mut sun_bands_out = SunBands::default();
    let row_domain = OcclusionDomain::new(128, 64, 1.0);

    let mut step = |i: u32| {
        // Shadow cache plan: move some dynamic obstacles, pan, decide and commit.
        for (k, &id) in shadow_movers.iter().enumerate().skip(i as usize % 2) {
            let p = At::<World2, Vec2>::new(Vec2::new(4.0 * k as f32 + i as f32, 3.0));
            shadow_obstacles.set_transform(id, p, 0.0).unwrap();
        }
        let mut projection = Projection::IDENTITY;
        projection.origin = At::new(Vec2::new(i as f32, 0.5 * i as f32));
        let input = MaskPlanInput::new(&shadow_obstacles, &projection, 128, 64, 1.0);
        let _ = plan.decide(&input);
        let _ = plan.mask_rects().area();
        plan.commit();

        // Obstacles: move, re-describe in place (same sizes), consume the dirty sets, save.
        for (k, &id) in moving.iter().enumerate() {
            let p = At::<World2, Vec2>::new(Vec2::new(k as f32, i as f32));
            obstacles.set_transform(id, p, i as f32).unwrap();
        }
        let mut d = ShapeDesc::polygon(&poly[..]);
        d.common.opacity = if i.is_multiple_of(2) { 0.5 } else { 1.0 };
        obstacles.set_shape(moving[0], &d).unwrap();
        obstacles
            .set_shape(moving[8], &ShapeDesc::rect(2.0, 1.0))
            .unwrap();
        obstacles
            .set_shape(moving[9], &ShapeDesc::circle(1.5))
            .unwrap();
        obstacles
            .set_tile_grid(grid, &TileGridDesc::new(2, 2, &cells))
            .unwrap();
        let _ = (
            obstacles.any_partial(),
            obstacles.any_finite_height(),
            obstacles.max_finite_top(),
            obstacles.dirty_static().count() + obstacles.dirty_dynamic().count(),
        );
        world_triangles.clear();
        obstacles.append_world_triangles(moving[0], &mut world_triangles);
        obstacles.clear_dirty();
        obstacle_snap.clear();
        obstacle_snap.save(&obstacles);

        // Frame lights: validated and derived into the pool.
        store.set_frame_lights(&frame[..], &NoResources).unwrap();
        store.clear_frame_lights();
        // Moving lights and updating one in place.
        for (k, &id) in ids.iter().enumerate() {
            let p = At::new(Vec2::new(k as f32, i as f32));
            store.set_position(id, p, Height::At(1.0)).unwrap();
        }
        // Snapshot round trip into reused buffers.
        snap.clear();
        snap.save(&store);
        restore(&mut Reader::new(snap.bytes()), &mut store).unwrap();
        // Collecting the lights of a frame with animations, masks and flicker.
        let modes = LightModes {
            animations: Some(&animations),
            masks: Some(&masks),
            normal_mode: i.is_multiple_of(2),
            height_mode: true,
            max_height: 4.0,
            ..LightModes::default()
        };
        collect_lights(
            &scene,
            &LightSettings::default(),
            &Projection::IDENTITY,
            f64::from(i) / 60.0,
            128,
            64,
            &modes,
            &mut list,
        );
        // The light rows: their rows (the mask changed in the plan's rectangles) and the blocks
        // of the row programs.
        assert!(!list.rows.is_empty());
        let row_mask = RowMask {
            size: [row_domain.width, row_domain.height],
            snapped: [i as i32 / 8, 0],
            change: MaskChange::Rects(plan.row_rects()),
        };
        row_cache.begin();
        assert!(list.rows.iter().any(|l| l.is_area() && l.shadowed));
        for light in &list.rows {
            // A line light's support points, each a lamp of its own.
            for k in 0..light.rows() {
                let support = light.support(k);
                if let Some((row, _)) = row_cache.assign(&support, &row_mask) {
                    let lamp = RowLamp {
                        light: support.light,
                        row: [row as f32, support.size, 0.0, 0.0],
                        mask: support.mask.unwrap_or_default(),
                        window: support.window[0],
                        window_v: support.window[1],
                    };
                    let _ = lamp.to_bytes();
                }
            }
        }
        row_cache.commit();
        // The suns' maps: their blocks and their slots, built when the mask changed.
        sun_cache.begin();
        assert!(!list.directional.is_empty());
        let directional = list
            .directional
            .iter()
            .zip(&list.directional_casters)
            .zip(&list.directional_rise);
        for (k, ((g, &casters), &rise)) in directional.enumerate() {
            let _ = sun_plain(g);
            let params = SunParams::new(g, rise, row_mask.size, row_mask.snapped, 0.05, 3.0)
                .with_partial(i.is_multiple_of(2), true);
            let _ = params.to_bytes();
            let source = SunSource::new(&params, &row_mask).with_casters(casters);
            if let Some((_, Refresh::Update(before))) =
                sun_cache.assign(k as u64, source, row_mask.change)
            {
                let _ = sun_bands(
                    &params,
                    &source,
                    &before,
                    row_mask.change,
                    &mut sun_bands_out,
                );
            }
        }
        sun_cache.commit();
        let space = RowSpace::new(&Projection::IDENTITY, &row_domain);
        let heights = height_code(50.0);
        let _ = (
            RowBuildParams::new(&space, row_mask.size, true, heights).to_bytes(),
            RowEdgeParams::new(&space, row_cache.rows(), heights).to_bytes(),
            RowLookupParams::new(&space, row_mask.size, row_cache.rows(), 0.05, heights).to_bytes(),
        );
        // The lights and suns of the medium stage.
        collect_medium_lights(
            &scene,
            &Projection::IDENTITY,
            128,
            64,
            20.0,
            f64::from(i) / 60.0,
            &modes,
            &mut medium_list,
        );
        collect_medium_suns(&scene, f64::from(i) / 60.0, &modes, &mut medium_suns);
        // Atlases and the zone pass.
        fill_profile_atlas(&profiles, profile_rows, &mut profile_atlas);
        fill_mask_atlas(&masks, mask_rows, &mut mask_atlas);
        collect_ambient_zones(
            &zones,
            [0.3, 0.3, 0.3],
            &zone_scene,
            [0.0; 3],
            &mut zone_list,
        );
        let written = write_zones(&zone_list, &mut zone_records);
        fill_zone_vertices(&zones, zone_rows, &mut zone_vertices);
        let frag = ZoneFragParams::new(&zone_scene.projection, 128, 64, 0.1, false, zone_rows);
        let pass = ZonePassParams {
            scene_size: [128.0, 64.0],
            instance_offset: i,
            pad: 0,
        };
        let _ = (written, frag.to_bytes(), pass.to_bytes());
        // The light pass's uniform blocks.
        let domain = ShadowDomain::default();
        let settings = LightSettings::default();
        let _ = (
            LightSurfacePassParams::new(&Projection::IDENTITY, 128, 64, 4.0, 0).to_bytes(),
            SurfaceParams::new(&Projection::IDENTITY, 128, 64, &settings, &domain, true).to_bytes(),
        );
        // Snapshot round trips of the animation and mask stores.
        anim_snap.clear();
        anim_snap.save(&animations);
        restore(&mut Reader::new(anim_snap.bytes()), &mut animations).unwrap();
        mask_snap.clear();
        mask_snap.save(&masks);
        restore(&mut Reader::new(mask_snap.bytes()), &mut masks).unwrap();
        // Obstacle raster: collect, lay out, sync the grids, write the changed cells.
        static_set.collect(&obstacles, &NoPatterns, false, 0.5, true);
        dynamic_set.collect(&obstacles, &NoPatterns, true, 0.5, true);
        write_triangle_buffer(
            dynamic_set.vertices(),
            dynamic_set.opacities(),
            &mut raster_bytes,
        );
        tile_set.sync(&obstacles, |_, _| Ok::<(), ()>(())).unwrap();
        for slot in tile_set.slots_mut() {
            if let Some(r) = slot.pending.take() {
                write_cell_texels(&obstacles, slot.handle, &r, &NoPatterns, &mut texels);
            }
        }
        // Geometry and codecs.
        tri.triangulate(&poly[..], &mut triangles).unwrap();
        convert_normal_map(
            &normal_map,
            16,
            16,
            64,
            &NormalMapOptions::default(),
            &mut material_map,
        )
        .unwrap();
        let n = Some(Vec3::new(0.2, 0.3, 0.9));
        let _ = (
            encode_normal(n),
            surface_pixel(3.0, 0.5, Vec3::new(0.0, 0.0, 1.0)),
            material_pixel(Material::preset(MaterialPreset::Glass), n),
        );
        // Composite pass: program choice, binding layout, parameter block.
        let program = composite_program_of(
            i.is_multiple_of(2),
            i.is_multiple_of(3),
            i.is_multiple_of(5),
        );
        let composite = CompositeParams::new(&CompositeParamsInput {
            input_space: InputColorSpace::Srgb,
            scene_format_is_srgb: false,
            light_buffer: true,
            glow: Some(false),
            ambient: Vec3::new(0.1, 0.1, 0.1),
            use_specular: false,
            direction: true,
            surface_buffer: true,
            projection: &Projection::IDENTITY,
            scene_size: (128, 64),
            settings: &settings,
        });
        let _ = (composite_layout_of(program), composite.to_bytes());
    };
    // Warm-up: the pool, the snapshot buffer, the slots and the triangulator grow.
    for i in 0..4 {
        step(i);
    }
    let before = allocations();
    for i in 4..40 {
        step(i);
    }
    let allocated = allocations() - before;
    assert_eq!(allocated, 0, "allocations in 36 steady-state steps");
}
