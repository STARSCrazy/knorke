//! Shadow cache plan (P16): dirty rectangles and the incremental mask update decision.
//! Ported from chiaro @ 9bc5b9b, tests/unit/DirtyRectsTests.cpp and MaskUpdatePlanTests.cpp.

use knorke_light2d::obstacle::{ObstacleId, ShapeDesc, TileGridDesc};
use knorke_light2d::shadow::{
    CellLog, CellRect, DirtyRect, DirtyRectList, FULL_REBUILD_RATIO, MaskAnchor, MaskLayout,
    MaskPlanInput, MaskUpdate, MaskUpdateLimits, MaskUpdatePlan, OcclusionDomain,
    SPRITE_PAN_ALLOWANCE, obstacle_grid_rect, sprite_grid_rect,
};
use knorke_light2d::{ObstacleStore, Projection};
use knorke_math::{At, Vec2};

fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> DirtyRect {
    DirtyRect::new(x0, y0, x1, y1)
}

fn covers(list: &DirtyRectList, x: i32, y: i32) -> bool {
    list.rects()
        .iter()
        .any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
}

fn no_overlaps(list: &DirtyRectList) -> bool {
    let rs = list.rects();
    (0..rs.len()).all(|i| {
        (i + 1..rs.len()).all(|j| {
            let (a, b) = (rs[i], rs[j]);
            !(a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1)
        })
    })
}

// ---- dirty rectangles ----

#[test]
fn exposed_strips_of_a_camera_shift() {
    let mut list = DirtyRectList::new();
    list.add_exposed_strips(3, 0, 100, 50);
    assert_eq!(list.rects(), &[r(0, 0, 3, 50)]);

    list.clear();
    list.add_exposed_strips(-3, -2, 100, 50);
    assert_eq!(list.rects(), &[r(97, 0, 100, 50), r(0, 48, 100, 50)]);
    list.merge();
    assert!(no_overlaps(&list));
    assert_eq!(list.area(), 3 * 50 + 100 * 2 - 3 * 2);
    for y in 0..50 {
        for x in 0..100 {
            assert_eq!(covers(&list, x, y), x >= 97 || y >= 48, "({x}, {y})");
        }
    }

    list.clear();
    list.add_exposed_strips(0, 0, 100, 50);
    assert!(list.rects().is_empty());
}

#[test]
fn expand_grows_clips_and_drops() {
    let mut list = DirtyRectList::new();
    assert!(list.add(r(0, 0, 3, 50)));
    assert!(list.add(r(40, 20, 50, 30)));
    assert!(list.add(r(95, 45, 100, 50)));
    list.expand(10, 100, 50);
    assert_eq!(
        list.rects(),
        &[r(0, 0, 13, 50), r(30, 10, 60, 40), r(85, 35, 100, 50)]
    );

    list.clear();
    assert!(list.add(r(-30, 0, -20, 50)));
    assert!(list.add(r(-5, 0, 0, 50)));
    list.expand(10, 100, 50);
    assert_eq!(list.rects(), &[r(0, 0, 10, 50)]);
}

#[test]
fn merge_folds_overlapping_and_touching_rectangles() {
    let cases: [(&[DirtyRect], &[DirtyRect]); 5] = [
        (&[r(0, 0, 10, 10), r(5, 2, 15, 10)], &[r(0, 0, 15, 10)]),
        (&[r(0, 0, 10, 10), r(10, 0, 20, 10)], &[r(0, 0, 20, 10)]),
        (
            &[r(0, 0, 10, 10), r(20, 20, 30, 30)],
            &[r(0, 0, 10, 10), r(20, 20, 30, 30)],
        ),
        (
            &[r(0, 0, 10, 10), r(20, 0, 30, 10), r(8, 0, 22, 10)],
            &[r(0, 0, 30, 10)],
        ),
        (&[r(2, 2, 4, 4), r(0, 0, 10, 10)], &[r(0, 0, 10, 10)]),
    ];
    for (input, expected) in cases {
        let mut list = DirtyRectList::new();
        for x in input {
            assert!(list.add(*x));
        }
        list.merge();
        assert_eq!(list.rects(), expected, "{input:?}");
    }
}

#[test]
fn merge_cuts_an_l_instead_of_covering_its_bounding_box() {
    let mut list = DirtyRectList::new();
    assert!(list.add(r(0, 0, 65, 512)));
    assert!(list.add(r(0, 0, 768, 65)));
    list.merge();
    assert!(no_overlaps(&list));
    assert_eq!(list.area(), 65 * 512 + 768 * 65 - 65 * 65);
    assert_eq!(list.rects().len(), 2);
}

#[test]
fn merge_leaves_no_overlaps_and_keeps_every_texel() {
    let mut list = DirtyRectList::new();
    let mut input = Vec::new();
    let mut seed: u32 = 12345;
    let mut next = |range: u32| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((seed >> 8) % range) as i32
    };
    for _ in 0..40 {
        let (x, y) = (next(90), next(40));
        let rect = r(x, y, x + 1 + next(20), y + 1 + next(15));
        input.push(rect);
        assert!(list.add(rect));
    }
    list.merge();
    assert!(no_overlaps(&list));
    for rect in &input {
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                assert!(covers(&list, x, y));
            }
        }
    }
}

#[test]
fn the_list_holds_max_dirty_rects_and_skips_empty_ones() {
    let mut list = DirtyRectList::new();
    assert!(list.add(r(5, 5, 5, 10)));
    assert!(list.add(r(5, 5, 10, 4)));
    assert!(list.rects().is_empty());
    for i in 0..DirtyRectList::MAX_DIRTY_RECTS as i32 {
        assert!(list.add(r(i * 4, 0, i * 4 + 2, 2)));
    }
    assert!(!list.overflowed());
    assert!(!list.add(r(1000, 0, 1002, 2)));
    assert!(list.overflowed());
    assert_eq!(list.rects().len(), DirtyRectList::MAX_DIRTY_RECTS);
    assert_eq!(list.area(), DirtyRectList::MAX_DIRTY_RECTS as u64 * 4);
    list.clear();
    assert!(list.rects().is_empty() && !list.overflowed() && list.area() == 0);
}

#[test]
fn content_shifted_out_of_the_domain_marks_where_the_field_forgets_it() {
    let grid = [r(2, 5, 10, 15), r(50, 20, 60, 30)];
    let mut list = DirtyRectList::new();
    list.add_vanished_content(&grid, [10 - 15, 0], [-15, 0], 100, 50);
    assert_eq!(list.rects(), &[r(-3, 5, 0, 15)]);
    list.clear();
    list.add_vanished_content(&grid, [10 + 15, 0], [15, 0], 100, 50);
    assert!(list.rects().is_empty());
    list.clear();
    list.add_vanished_content(&grid, [10, 25], [0, 25], 100, 50);
    assert_eq!(list.rects(), &[r(60, 50, 70, 55)]);
    list.clear();
    list.add_vanished_content(&grid, [10, 0], [0, 0], 100, 50);
    assert!(list.rects().is_empty());
}

#[test]
fn union_folds_only_when_cheaper() {
    let mut list = DirtyRectList::new();
    list.add_union(r(0, 0, 4, 4), r(1, 0, 5, 4));
    assert_eq!(list.rects(), &[r(0, 0, 5, 4)]);
    list.clear();
    list.add_union(r(0, 0, 4, 4), r(40, 40, 44, 44));
    assert_eq!(list.rects().len(), 2);
    list.clear();
    list.add_union(DirtyRect::EMPTY, r(1, 1, 2, 2));
    assert_eq!(list.rects(), &[r(1, 1, 2, 2)]);
}

// ---- domain and anchor ----

#[test]
fn domain_and_anchor() {
    let d = OcclusionDomain::new(64, 48, 0.5);
    assert_eq!(d.margin, 16);
    assert_eq!((d.width, d.height), (48, 40));
    assert_eq!(
        d.scene_to_domain(Vec2::new(0.0, 0.0)),
        Vec2::new(16.0 * d.texels_per_scene[0], 16.0 * d.texels_per_scene[1])
    );
    let p = Projection {
        origin: At::new(Vec2::new(5.5, -2.25)),
        ..Projection::default()
    };
    let a = MaskAnchor::compute(&p, &OcclusionDomain::new(64, 64, 1.0));
    assert_eq!(a.snapped, [5, -3]);
    assert_eq!(a.remainder, [0.5, 0.75]);
}

// ---- the plan ----

/// A 64 x 64 scene with an 18-pixel band, one mask texel per scene pixel and per world unit:
/// mask texel = world + 18 + origin.
struct Plan {
    plan: MaskUpdatePlan,
    store: ObstacleStore,
    projection: Projection,
    domain: OcclusionDomain,
    sprite: bool,
    force_full: bool,
}

const SQUARE4: [Vec2; 4] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(4.0, 0.0),
    Vec2::new(4.0, 4.0),
    Vec2::new(0.0, 4.0),
];

impl Plan {
    fn new(limits: MaskUpdateLimits) -> Self {
        let projection = Projection {
            origin: At::new(Vec2::ZERO),
            x_axis: Vec2::new(1.0, 0.0),
            y_axis: Vec2::new(0.0, 1.0),
            ..Projection::default()
        };
        Self {
            plan: MaskUpdatePlan::new(limits),
            store: ObstacleStore::new(),
            projection,
            domain: domain(64, 64, 100, 100),
            sprite: false,
            force_full: false,
        }
    }

    fn plain() -> Self {
        Self::new(MaskUpdateLimits {
            field_grow: 0,
            seed_grow: 0,
            full_rebuild_ratio: 1.0,
        })
    }

    fn decide(&mut self) -> MaskUpdate {
        self.decide_with(&knorke_light2d::shadow::NoCellLog)
    }

    fn decide_with(&mut self, cells: &dyn CellLog) -> MaskUpdate {
        let anchor = MaskAnchor::compute(&self.projection, &self.domain);
        let layout = MaskLayout::new(
            &self.store,
            &self.projection,
            self.domain.scene_w,
            self.domain.scene_h,
            1.0,
        );
        let input = MaskPlanInput {
            obstacles: &self.store,
            cells,
            projection: &self.projection,
            domain: self.domain,
            anchor,
            layout,
            sprite: self.sprite,
            force_full: self.force_full,
        };
        self.plan.decide(&input)
    }

    fn frame(&mut self) -> MaskUpdate {
        let u = self.decide();
        self.plan.commit();
        u
    }

    fn pan(&mut self, x: f32, y: f32) {
        self.projection.origin = At::new(Vec2::new(x, y));
    }

    fn boxed(&mut self, x: f32, y: f32, size: f32, dynamic: bool) -> ObstacleId {
        let v = SQUARE4.map(|p| Vec2::new(p.x * size / 4.0, p.y * size / 4.0));
        let mut d = ShapeDesc::polygon(&v);
        d.common.dynamic = dynamic;
        d.common.position = At::new(Vec2::new(x, y));
        self.store.create_shape(&d).unwrap()
    }

    fn set_enabled(&mut self, id: ObstacleId, x: f32, y: f32, enabled: bool) {
        let mut d = ShapeDesc::polygon(&SQUARE4);
        d.common.dynamic = true;
        d.common.enabled = enabled;
        d.common.position = At::new(Vec2::new(x, y));
        self.store.set_shape(id, &d).unwrap();
    }

    fn move_to(&mut self, id: ObstacleId, x: f32, y: f32, rotation: f32) {
        self.store
            .set_transform(id, At::new(Vec2::new(x, y)), rotation)
            .unwrap();
    }
}

fn domain(scene_w: u32, scene_h: u32, width: u32, height: u32) -> OcclusionDomain {
    OcclusionDomain {
        scene_w,
        scene_h,
        margin: 18,
        width,
        height,
        texels_per_scene: [1.0, 1.0],
    }
}

#[test]
fn first_frame_forced_frame_and_static_change_rebuild_fully() {
    let mut p = Plan::plain();
    p.boxed(10.0, 10.0, 4.0, false);
    assert_eq!(p.frame(), MaskUpdate::Full);
    assert_eq!(p.frame(), MaskUpdate::Reused);
    p.force_full = true;
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.force_full = false;
    assert_eq!(p.frame(), MaskUpdate::Reused);
    p.boxed(30.0, 30.0, 4.0, false); // a new static version
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.plan.invalidate();
    assert_eq!(p.frame(), MaskUpdate::Full);
    assert_eq!(p.frame(), MaskUpdate::Reused);
}

#[test]
fn a_shift_by_half_the_mask_stays_local_one_texel_more_rebuilds() {
    let mut p = Plan::plain();
    p.domain = domain(64, 41, 100, 61);
    assert_eq!(p.frame(), MaskUpdate::Full);
    let shift = |p: &mut Plan, x, y| {
        p.pan(x, y);
        p.decide() // never committed
    };
    assert_eq!(shift(&mut p, 50.0, 0.0), MaskUpdate::Incremental);
    assert_eq!(p.plan.shift(), [50, 0]);
    assert_eq!(p.plan.mask_rects().rects(), &[r(0, 0, 50, 61)]);
    assert_eq!(shift(&mut p, 51.0, 0.0), MaskUpdate::Full);
    assert_eq!(shift(&mut p, -50.0, 0.0), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(50, 0, 100, 61)]);
    assert_eq!(shift(&mut p, -51.0, 0.0), MaskUpdate::Full);
    assert_eq!(shift(&mut p, 0.0, 30.0), MaskUpdate::Incremental);
    assert_eq!(shift(&mut p, 0.0, 31.0), MaskUpdate::Full);
    assert_eq!(shift(&mut p, 0.0, -30.0), MaskUpdate::Incremental);
    assert_eq!(shift(&mut p, 0.0, -31.0), MaskUpdate::Full);
    assert_eq!(shift(&mut p, 0.0, 0.0), MaskUpdate::Reused);
    assert_eq!(p.plan.shift(), [0, 0]);
}

#[test]
fn field_rects_of_exactly_the_ratio_stay_local() {
    let mut p = Plan::new(MaskUpdateLimits {
        field_grow: 0,
        seed_grow: 0,
        full_rebuild_ratio: FULL_REBUILD_RATIO,
    });
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.pan(50.0, 0.0);
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert_eq!(p.plan.field_rects().area(), 5000);
    p.pan(50.0, 1.0);
    assert_eq!(p.decide(), MaskUpdate::Full);
}

#[test]
fn field_and_seed_rects_grow_by_the_limits() {
    let mut p = Plan::new(MaskUpdateLimits {
        field_grow: 10,
        seed_grow: 5,
        full_rebuild_ratio: 1.0,
    });
    let o = p.boxed(30.0, 30.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.move_to(o, 31.0, 30.0, 0.0);
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(47, 47, 54, 53)]);
    assert_eq!(p.plan.field_rects().rects(), &[r(37, 37, 64, 63)]);
    assert_eq!(p.plan.seed_rects().rects(), &[r(32, 32, 69, 68)]);
}

#[test]
fn an_unchanged_dynamic_obstacle_adds_no_rectangle() {
    let mut p = Plan::plain();
    p.boxed(10.0, 10.0, 4.0, false);
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    assert_eq!(p.frame(), MaskUpdate::Reused);
    p.move_to(o, 40.0, 40.0, 0.0); // the same transform: the revision rises, the state does not
    assert_eq!(p.frame(), MaskUpdate::Reused);
    assert_eq!(p.plan.footprints().len(), 1);
    assert_eq!(p.plan.footprints()[0].handle, o);

    p.pan(2.0, 0.0);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(0, 0, 2, 100)]);

    p.move_to(o, 41.0, 40.0, 0.0);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(59, 57, 66, 63)]);
    p.move_to(o, 41.0, 40.0, 30.0); // a rotation alone
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.frame(), MaskUpdate::Reused);
}

#[test]
fn a_destroyed_dynamic_obstacle_adds_its_previous_rectangle_once() {
    let mut p = Plan::plain();
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.store.destroy(o).unwrap();
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(57, 57, 63, 63)]);
    assert!(p.plan.footprints().is_empty());
    assert_eq!(p.frame(), MaskUpdate::Reused);
}

#[test]
fn a_disabled_then_reenabled_obstacle() {
    let mut p = Plan::plain();
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.set_enabled(o, 40.0, 40.0, false);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(57, 57, 63, 63)]);
    assert_eq!(p.frame(), MaskUpdate::Reused);
    p.set_enabled(o, 10.0, 10.0, true);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(27, 27, 33, 33)]);
}

#[test]
fn a_pan_that_also_disables_moves_the_previous_rectangle_along() {
    let mut p = Plan::plain();
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.set_enabled(o, 40.0, 40.0, false);
    p.pan(-3.0, 2.0);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.shift(), [-3, 2]);
    assert!(covers(p.plan.mask_rects(), 54, 59));
    assert!(covers(p.plan.mask_rects(), 59, 64));
    assert!(!covers(p.plan.mask_rects(), 60, 58));
}

#[test]
fn a_reused_slot_counts_as_removed_plus_new() {
    let mut p = Plan::plain();
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.store.destroy(o).unwrap();
    let n = p.boxed(10.0, 10.0, 4.0, true);
    assert_eq!(n.index(), o.index());
    assert_ne!(n, o);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert!(covers(p.plan.mask_rects(), 60, 60)); // old
    assert!(covers(p.plan.mask_rects(), 30, 30)); // new
    assert_eq!(p.plan.footprints()[0].handle, n);
}

#[test]
fn a_decided_frame_that_is_never_committed_changes_nothing() {
    let mut p = Plan::plain();
    let o = p.boxed(40.0, 40.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.move_to(o, 41.0, 40.0, 0.0);
    p.pan(1.0, 0.0);
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert_eq!(p.plan.shift(), [1, 0]);
    assert!(covers(p.plan.mask_rects(), 58, 58));
    assert_eq!(p.plan.footprints().len(), 1);
    assert_eq!(p.plan.footprints()[0].position.raw().x, 40.0);
}

#[test]
fn a_sprite_mask_adds_its_rectangle_every_frame_and_the_previous_once() {
    let mut p = Plan::plain();
    p.boxed(10.0, 10.0, 4.0, false);
    let anchor = MaskAnchor::compute(&p.projection, &p.domain);
    assert_eq!(sprite_grid_rect(&p.domain, &anchor), r(17, 17, 83, 83));
    assert_eq!(SPRITE_PAN_ALLOWANCE, 4);
    p.sprite = true;
    assert_eq!(p.frame(), MaskUpdate::Full);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(13, 13, 87, 87)]);

    p.pan(3.0, 0.0);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert!(covers(p.plan.mask_rects(), 85, 50));
    assert!(covers(p.plan.mask_rects(), 13, 50));
    assert!(covers(p.plan.mask_rects(), 1, 50));
    assert!(!covers(p.plan.mask_rects(), 87, 50));
    p.pan(9.0, 0.0);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert!(covers(p.plan.mask_rects(), 88, 50));

    p.sprite = false;
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(17, 17, 83, 83)]);
    assert_eq!(p.frame(), MaskUpdate::Reused);
}

fn sixty_four_boxes(p: &mut Plan) -> Vec<ObstacleId> {
    let mut boxes = Vec::new();
    for j in 0..8 {
        for i in 0..8 {
            boxes.push(p.boxed((12 * i - 16) as f32, (12 * j - 16) as f32, 1.0, true));
        }
    }
    boxes
}

#[test]
fn more_changed_rectangles_than_the_list_holds_rebuild_fully() {
    let mut p = Plan::plain();
    let boxes = sixty_four_boxes(&mut p);
    assert_eq!(p.frame(), MaskUpdate::Full);
    assert_eq!(p.frame(), MaskUpdate::Reused);
    for (k, id) in boxes.iter().enumerate() {
        let (i, j) = ((k % 8) as i32, (k / 8) as i32);
        p.move_to(*id, (12 * i - 16) as f32 + 1.0, (12 * j - 16) as f32, 0.0);
    }
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects().len(), 64);
    p.pan(1.0, 0.0);
    assert_eq!(p.decide(), MaskUpdate::Full);
}

#[test]
fn a_65th_dynamic_obstacle_is_tracked_like_the_others() {
    let mut p = Plan::plain();
    sixty_four_boxes(&mut p);
    assert_eq!(p.frame(), MaskUpdate::Full);
    let extra = p.boxed(80.0, 80.0, 4.0, true);
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.plan.mask_rects().rects(), &[r(97, 97, 100, 100)]);
    assert_eq!(p.frame(), MaskUpdate::Reused);
    assert_eq!(p.plan.footprints().len(), 65);
    p.store.destroy(extra).unwrap();
    assert_eq!(p.frame(), MaskUpdate::Incremental);
    assert_eq!(p.frame(), MaskUpdate::Reused);
}

#[test]
fn a_pan_marks_static_obstacles_pushed_out_of_the_mask_for_the_field() {
    let mut p = Plan::new(MaskUpdateLimits {
        field_grow: 2,
        seed_grow: 0,
        full_rebuild_ratio: 1.0,
    });
    // World [-18, -14] is mask texels [0, 4] (+1 texel round-out on each side).
    p.boxed(-18.0, 10.0, 4.0, false);
    assert_eq!(p.frame(), MaskUpdate::Full);
    p.pan(-3.0, 0.0); // contents move 3 texels left: the box leaves over the left edge
    assert_eq!(p.decide(), MaskUpdate::Incremental);
    assert!(p.plan.mask_rects().rects().iter().all(|r| r.x0 >= 97));
    assert!(covers(p.plan.field_rects(), 0, 30)); // the field still remembers it
}

/// A cell log with one change after version 0.
struct OneChange(ObstacleId, CellRect, bool);

impl CellLog for OneChange {
    fn cell_version(&self) -> u64 {
        1
    }
    fn for_each_change_since(&self, v: u64, f: &mut dyn FnMut(ObstacleId, CellRect)) -> bool {
        if v < 1 {
            f(self.0, self.1);
        }
        self.2
    }
}

#[test]
fn logged_tile_cell_changes_redraw_their_cells_or_rebuild_when_lost() {
    let mut p = Plan::plain();
    let solid = [1u8; 16];
    let mut d = TileGridDesc::new(4, 4, &solid);
    d.common.position = At::new(Vec2::new(0.0, 0.0));
    let grid = p.store.create_tile_grid(&d).unwrap();
    assert_eq!(p.frame(), MaskUpdate::Full);
    let cell_size = p.store.get(grid).unwrap().grid.cell_size;
    let cells = CellRect {
        x0: 1,
        y0: 1,
        x1: 2,
        y1: 2,
    };
    let log = OneChange(grid, cells, true);
    assert_eq!(p.decide_with(&log), MaskUpdate::Incremental);
    let (cx, cy) = (
        18 + (cell_size.x * 1.5) as i32,
        18 + (cell_size.y * 1.5) as i32,
    );
    assert!(covers(p.plan.mask_rects(), cx, cy));
    let lost = OneChange(grid, cells, false);
    assert_eq!(p.decide_with(&lost), MaskUpdate::Full);
}

#[test]
fn obstacle_grid_rect_rounds_outwards() {
    let mut p = Plan::plain();
    let o = p.boxed(30.0, 30.0, 4.0, false);
    let mut scratch = Vec::new();
    let rect = obstacle_grid_rect(&p.store, o, &p.projection, &p.domain, &mut scratch);
    assert_eq!(rect, r(47, 47, 53, 53));
}
