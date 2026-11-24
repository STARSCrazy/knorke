// Origin: chiaro @ 9bc5b9b, src/shadows/MaskUpdatePlan.hpp and MaskUpdatePlan.cpp (MaskUpdate,
// MaskLayout, DynamicFootprint, MaskUpdateLimits, MaskPlanInput, MaskUpdatePlan, SpriteGridRect).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The CPU half of the incremental mask and field update (E21).

use super::dirty_rects::{
    CellRect, DirtyRect, DirtyRectList, clamp_texel_coord, obstacle_grid_rect, tile_cells_grid_rect,
};
use super::domain::{MaskAnchor, OcclusionDomain};
use crate::Projection;
use crate::obstacle::{ObstacleId, ObstacleStore};
use knorke_math::{At, Vec2, World2};
use knorke_types::Revision;

/// How the occlusion stage brings mask and distance field up to date in a frame with shadows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskUpdate {
    /// Unchanged since the last frame: nothing is drawn.
    Reused,
    /// Shifted by a camera pan and/or redrawn in the changed rectangles only.
    Incremental,
    /// Redrawn completely.
    Full,
}

/// A local update whose field rectangles cover more than this share of the mask rebuilds fully.
pub const FULL_REBUILD_RATIO: f64 = 0.5;

/// With a sprite mask, its current scene rectangle is redrawn grown by this many mask texels on
/// every side. That covers the previous rectangle moved by a pan of up to this many texels per
/// axis, so during a steady pan the changed area is the same whether a frame shifts or not.
pub const SPRITE_PAN_ALLOWANCE: i32 = 4;

/// What the mask grid depends on besides the projection origin: equal layouts keep texels
/// comparable.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaskLayout {
    /// [`ObstacleStore::static_version`] of the frame.
    pub static_version: Revision,
    /// Linear part of the projection.
    pub x_axis: Vec2,
    /// Linear part of the projection.
    pub y_axis: Vec2,
    /// Scene width in pixels.
    pub scene_w: u32,
    /// Scene height in pixels.
    pub scene_h: u32,
    /// Shadow resolution scale.
    pub scale: f32,
}

impl MaskLayout {
    /// The layout of a frame.
    pub fn new(
        obstacles: &ObstacleStore,
        projection: &Projection,
        scene_w: u32,
        scene_h: u32,
        scale: f32,
    ) -> Self {
        Self {
            static_version: obstacles.static_version(),
            x_axis: projection.x_axis,
            y_axis: projection.y_axis,
            scene_w,
            scene_h,
            scale,
        }
    }
}

/// One enabled dynamic obstacle as the last recorded frame drew it; a change of revision,
/// position or rotation redraws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicFootprint {
    /// The obstacle.
    pub handle: ObstacleId,
    /// Its [`StoredObstacle::revision`](crate::obstacle::StoredObstacle::revision).
    pub revision: u32,
    /// Its position.
    pub position: At<World2, Vec2>,
    /// Its rotation in degrees.
    pub rotation: f32,
    /// [`obstacle_grid_rect`](super::obstacle_grid_rect) of that state.
    pub grid_rect: DirtyRect,
}

impl DynamicFootprint {
    /// Everything the mask rectangle depends on, besides the layout.
    fn same_state(&self, b: &Self) -> bool {
        self.revision == b.revision
            && self.position.raw().x == b.position.raw().x
            && self.position.raw().y == b.position.raw().y
            && self.rotation == b.rotation
    }
}

/// The rectangle rules' parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskUpdateLimits {
    /// Field rectangles = changed mask rectangles grown by this (the field bound).
    pub field_grow: i32,
    /// Seed rectangles = field rectangles grown by this (jump flooding span + 2).
    pub seed_grow: i32,
    /// Field rectangles covering more than this share of the mask: full rebuild.
    pub full_rebuild_ratio: f64,
}

/// Changes of tile cells (`set_tile_cells`) since a version; the tile grid editing of P14
/// implements it on the obstacle store. Without cell editing use [`NoCellLog`].
pub trait CellLog {
    /// Rises with every logged cell change.
    fn cell_version(&self) -> u64;
    /// Calls `f` for every change after `version` in order; `false` if the log no longer holds
    /// all of them (the plan then rebuilds fully). One call per frame, not per obstacle.
    fn for_each_change_since(&self, version: u64, f: &mut dyn FnMut(ObstacleId, CellRect)) -> bool;
}

/// A [`CellLog`] without changes.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCellLog;

impl CellLog for NoCellLog {
    fn cell_version(&self) -> u64 {
        0
    }
    fn for_each_change_since(&self, _: u64, _: &mut dyn FnMut(ObstacleId, CellRect)) -> bool {
        true
    }
}

/// The frame being prepared, as far as the decision needs it.
#[derive(Clone, Copy)]
pub struct MaskPlanInput<'a> {
    /// The scene's obstacles.
    pub obstacles: &'a ObstacleStore,
    /// Their cell change log.
    pub cells: &'a dyn CellLog,
    /// The frame's projection.
    pub projection: &'a Projection,
    /// The mask domain.
    pub domain: OcclusionDomain,
    /// The mask anchor.
    pub anchor: MaskAnchor,
    /// The mask layout.
    pub layout: MaskLayout,
    /// An application sprite mask takes part.
    pub sprite: bool,
    /// The current textures' contents are unknown (failed frame, new textures, restored scene).
    pub force_full: bool,
}

impl<'a> MaskPlanInput<'a> {
    /// The input of a frame without cell log and sprite mask; domain, anchor and layout follow
    /// from the arguments.
    pub fn new(
        obstacles: &'a ObstacleStore,
        projection: &'a Projection,
        scene_w: u32,
        scene_h: u32,
        scale: f32,
    ) -> Self {
        let domain = OcclusionDomain::new(scene_w, scene_h, scale);
        Self {
            obstacles,
            cells: &NoCellLog,
            projection,
            domain,
            anchor: MaskAnchor::compute(projection, &domain),
            layout: MaskLayout::new(obstacles, projection, scene_w, scene_h, scale),
            sprite: false,
            force_full: false,
        }
    }
}

/// The sprite mask's scene rectangle in grid coordinates, rounded outwards plus one texel like
/// [`obstacle_grid_rect`](super::obstacle_grid_rect).
pub fn sprite_grid_rect(d: &OcclusionDomain, a: &MaskAnchor) -> DirtyRect {
    let (tx, ty) = (
        f64::from(d.texels_per_scene[0]),
        f64::from(d.texels_per_scene[1]),
    );
    let x0 = f64::from(d.margin) * tx - f64::from(a.remainder[0]);
    let y0 = f64::from(d.margin) * ty - f64::from(a.remainder[1]);
    let x1 = x0 + f64::from(d.scene_w) * tx;
    let y1 = y0 + f64::from(d.scene_h) * ty;
    let lo = |v: f64, s: i32| clamp_texel_coord(v.floor() as i64 - 1 - i64::from(s));
    let hi = |v: f64, s: i32| clamp_texel_coord(v.ceil() as i64 + 1 - i64::from(s));
    DirtyRect::new(
        lo(x0, a.snapped[0]),
        lo(y0, a.snapped[1]),
        hi(x1, a.snapped[0]),
        hi(y1, a.snapped[1]),
    )
}

#[derive(Clone, Debug, Default)]
struct State {
    layout: MaskLayout,
    snapped: [i32; 2],
    /// Every enabled dynamic obstacle, sorted by slot index.
    footprints: Vec<DynamicFootprint>,
    sprite_grid_rect: DirtyRect,
    sprite_active: bool,
    /// [`CellLog::cell_version`] the frame includes.
    cell_version: u64,
    /// [`ObstacleStore::revision`] the frame includes.
    revision: Revision,
}

/// Decides how a frame with shadows updates mask and field and, for a local update, which
/// rectangles change. See the [module docs](super) for the rules.
///
/// [`decide`](Self::decide) prepares a frame; [`commit`](Self::commit) adopts it once it is
/// recorded. A frame that is decided but never committed leaves the recorded state untouched.
/// No heap allocation per frame once the footprint buffers have grown to the number of dynamic
/// obstacles in use.
#[derive(Clone, Debug)]
pub struct MaskUpdatePlan {
    limits: MaskUpdateLimits,
    /// The last recorded frame, valid if `drawn_valid`.
    drawn: State,
    /// The decided frame.
    next: State,
    drawn_valid: bool,
    shift: [i32; 2],
    /// Changed rectangles in mask texels of the current frame, unclipped.
    changed: DirtyRectList,
    mask_rects: DirtyRectList,
    field_rects: DirtyRectList,
    seed_rects: DirtyRectList,
    /// One per enabled static obstacle, for `grid_layout`.
    static_grid_rects: Vec<DirtyRect>,
    /// Unchanged dynamic obstacles of the decided frame.
    dynamic_grid_rects: Vec<DirtyRect>,
    grid_scratch: Vec<At<World2, Vec2>>,
    grid_layout: MaskLayout,
    grid_valid: bool,
}

/// Position of `id`'s footprint in `footprints` (sorted by slot index).
fn find(footprints: &[DynamicFootprint], id: ObstacleId) -> Option<&DynamicFootprint> {
    footprints
        .binary_search_by_key(&id.index(), |f| f.handle.index())
        .ok()
        .map(|i| &footprints[i])
        .filter(|f| f.handle == id)
}

impl MaskUpdatePlan {
    /// A plan without recorded frame: the first decision is [`MaskUpdate::Full`].
    pub fn new(limits: MaskUpdateLimits) -> Self {
        Self {
            limits,
            drawn: State::default(),
            next: State::default(),
            drawn_valid: false,
            shift: [0; 2],
            changed: DirtyRectList::new(),
            mask_rects: DirtyRectList::new(),
            field_rects: DirtyRectList::new(),
            seed_rects: DirtyRectList::new(),
            static_grid_rects: Vec::new(),
            dynamic_grid_rects: Vec::new(),
            grid_scratch: Vec::new(),
            grid_layout: MaskLayout::default(),
            grid_valid: false,
        }
    }

    /// Replaces the limits (tests, settings changes).
    pub fn set_limits(&mut self, limits: MaskUpdateLimits) {
        self.limits = limits;
    }

    /// Forgets the recorded frame: the next decision is [`MaskUpdate::Full`]. Call it when the
    /// mask textures are recreated or the scene was restored from a snapshot (the plan is a
    /// cache of what the GPU holds and is therefore not part of snapshots).
    pub fn invalidate(&mut self) {
        self.drawn_valid = false;
        self.grid_valid = false;
    }

    /// Decides how the frame `input` updates mask and field.
    pub fn decide(&mut self, input: &MaskPlanInput<'_>) -> MaskUpdate {
        if self.next.footprints.capacity() < DirtyRectList::MAX_DIRTY_RECTS {
            // Once: commit swaps the two states, both keep this storage.
            let n = DirtyRectList::MAX_DIRTY_RECTS;
            self.drawn.footprints.reserve(n);
            self.next.footprints.reserve(n);
            self.dynamic_grid_rects.reserve(n);
        }
        self.shift = [0; 2];
        self.next.layout = input.layout;
        self.next.snapped = input.anchor.snapped;
        self.next.sprite_active = input.sprite;
        self.next.cell_version = input.cells.cell_version();
        self.next.revision = input.obstacles.revision();
        self.next.sprite_grid_rect = if input.sprite {
            sprite_grid_rect(&input.domain, &input.anchor)
        } else {
            DirtyRect::EMPTY
        };
        // Grid rectangles of the recorded frame are comparable only within the same layout.
        let comparable = !input.force_full && self.drawn_valid && self.drawn.layout == input.layout;
        if comparable
            && self.next.snapped == self.drawn.snapped
            && self.next.revision == self.drawn.revision
            && self.next.cell_version == self.drawn.cell_version
            && !self.drawn.sprite_active
            && !input.sprite
        {
            // Fast path: no obstacle changed (store revision), no pan, no sprite mask.
            self.next.footprints.clear();
            self.next
                .footprints
                .extend_from_slice(&self.drawn.footprints);
            self.clear_rects();
            return MaskUpdate::Reused;
        }
        self.collect_footprints(input, comparable);
        if !comparable {
            return MaskUpdate::Full;
        }
        let dx = i64::from(self.next.snapped[0]) - i64::from(self.drawn.snapped[0]);
        let dy = i64::from(self.next.snapped[1]) - i64::from(self.drawn.snapped[1]);
        if 2 * dx.abs() > i64::from(input.domain.width)
            || 2 * dy.abs() > i64::from(input.domain.height)
        {
            return MaskUpdate::Full;
        }
        self.shift = [dx as i32, dy as i32];
        if !self.build_rects(input) {
            self.shift = [0; 2];
            return MaskUpdate::Full;
        }
        if dx == 0 && dy == 0 && self.field_rects.rects().is_empty() {
            return MaskUpdate::Reused; // nothing changed within reach of the mask
        }
        MaskUpdate::Incremental
    }

    /// The decided frame was recorded: its layout, anchor, footprints and sprite rectangle
    /// become the reference.
    pub fn commit(&mut self) {
        core::mem::swap(&mut self.drawn, &mut self.next); // both keep their storage
        self.drawn_valid = true;
    }

    /// Contents shift of the decided frame (`new[x] = old[x - shift]`), `[0, 0]` unless
    /// incremental.
    pub fn shift(&self) -> [i32; 2] {
        self.shift
    }

    /// Mask texels to redraw, merged (valid for an incremental frame).
    pub fn mask_rects(&self) -> &DirtyRectList {
        &self.mask_rects
    }

    /// Field texels to recompute, merged (valid for an incremental frame).
    pub fn field_rects(&self) -> &DirtyRectList {
        &self.field_rects
    }

    /// Seed and step pass area (valid for an incremental frame).
    pub fn seed_rects(&self) -> &DirtyRectList {
        &self.seed_rects
    }

    /// Footprints of the last committed frame, sorted by slot index.
    pub fn footprints(&self) -> &[DynamicFootprint] {
        &self.drawn.footprints
    }

    fn clear_rects(&mut self) {
        self.mask_rects.clear();
        self.field_rects.clear();
        self.seed_rects.clear();
    }

    /// Fills `next.footprints` from the store; reuses the drawn rectangle of an unchanged
    /// obstacle if `reuse`. The store iterates in slot order, so the list is sorted.
    fn collect_footprints(&mut self, input: &MaskPlanInput<'_>, reuse: bool) {
        self.next.footprints.clear();
        for (handle, o) in input.obstacles.iter() {
            if !o.common.dynamic || !o.is_active() {
                continue; // a disabled one is drawn nowhere, like a destroyed one
            }
            let mut f = DynamicFootprint {
                handle,
                revision: o.revision,
                position: o.common.position,
                rotation: o.common.rotation,
                grid_rect: DirtyRect::EMPTY,
            };
            let prev = if reuse {
                find(&self.drawn.footprints, handle).filter(|p| p.same_state(&f))
            } else {
                None
            };
            f.grid_rect = match prev {
                Some(p) => p.grid_rect,
                None => obstacle_grid_rect(
                    input.obstacles,
                    handle,
                    input.projection,
                    &input.domain,
                    &mut self.grid_scratch,
                ),
            };
            self.next.footprints.push(f);
        }
    }

    /// Fills the rectangle lists; `false` if they overflow or a full rebuild is cheaper.
    fn build_rects(&mut self, input: &MaskPlanInput<'_>) -> bool {
        let (w, h) = (input.domain.width as i32, input.domain.height as i32);
        let [sx, sy] = self.next.snapped;
        let to_mask = |r: DirtyRect| r.offset(sx, sy);

        // Changed mask rectangles in this frame's mask texels, unclipped: obstacles just outside
        // the mask still reach into the field.
        self.changed.clear();
        self.changed
            .add_exposed_strips(self.shift[0], self.shift[1], w, h);
        self.dynamic_grid_rects.clear();
        // Both footprint lists are sorted by slot index: one walk pairs them up.
        let before = &self.drawn.footprints;
        let mut b = 0;
        for cur in &self.next.footprints {
            let key = cur.handle.index();
            while b < before.len() && before[b].handle.index() < key {
                self.changed.add(to_mask(before[b].grid_rect)); // destroyed, disabled, static
                b += 1;
            }
            if b == before.len() || before[b].handle.index() != key {
                self.changed.add(to_mask(cur.grid_rect)); // new, enabled or now dynamic
                continue;
            }
            let prev = &before[b];
            b += 1;
            if prev.handle != cur.handle {
                // The slot was reused by another obstacle.
                self.changed.add(to_mask(prev.grid_rect));
                self.changed.add(to_mask(cur.grid_rect));
            } else if prev.same_state(cur) {
                self.dynamic_grid_rects.push(cur.grid_rect);
            } else {
                self.changed
                    .add_union(to_mask(prev.grid_rect), to_mask(cur.grid_rect));
            }
        }
        for prev in &before[b..] {
            self.changed.add(to_mask(prev.grid_rect));
        }
        // Changed tile cells since the recorded frame; their grid keeps its revision.
        let (changed, scratch) = (&mut self.changed, &mut self.grid_scratch);
        let cells_known =
            input
                .cells
                .for_each_change_since(self.drawn.cell_version, &mut |id, cells| {
                    // A destroyed grid added its whole rectangle already.
                    if let Some(grid) = input.obstacles.get(id).filter(|g| g.is_tile_grid()) {
                        let r = tile_cells_grid_rect(
                            grid,
                            &cells,
                            input.projection,
                            &input.domain,
                            scratch,
                        );
                        changed.add(r.offset(sx, sy));
                    }
                });
        if !cells_known {
            return false;
        }
        if self.drawn.sprite_active || self.next.sprite_active {
            // The sprite mask may change anywhere in its scene rectangle every frame; the current
            // rectangle grows by the pan allowance.
            let current = if self.next.sprite_active {
                to_mask(self.next.sprite_grid_rect).grow(SPRITE_PAN_ALLOWANCE)
            } else {
                DirtyRect::EMPTY
            };
            let previous = if self.drawn.sprite_active {
                to_mask(self.drawn.sprite_grid_rect)
            } else {
                DirtyRect::EMPTY
            };
            self.changed.add_union(previous, current);
        }
        if self.changed.overflowed() {
            return false;
        }

        self.mask_rects.clone_from(&self.changed);
        self.mask_rects.expand(0, w, h); // clip
        self.mask_rects.merge();
        // Field: every changed mask texel moves the field up to the bound around it, including
        // obstacles the shift pushed out of the mask.
        self.field_rects.clone_from(&self.changed);
        if self.shift != [0, 0] {
            self.update_static_grid_rects(input);
            let offset = self.next.snapped;
            self.field_rects.add_vanished_content(
                &self.static_grid_rects,
                offset,
                self.shift,
                w,
                h,
            );
            self.field_rects.add_vanished_content(
                &self.dynamic_grid_rects,
                offset,
                self.shift,
                w,
                h,
            );
        }
        self.field_rects.expand(self.limits.field_grow, w, h);
        self.field_rects.merge();
        let mask_area = f64::from(w) * f64::from(h);
        if self.mask_rects.overflowed()
            || self.field_rects.overflowed()
            || self.field_rects.area() as f64 > self.limits.full_rebuild_ratio * mask_area
        {
            return false;
        }
        self.seed_rects.clone_from(&self.field_rects);
        self.seed_rects.expand(self.limits.seed_grow, w, h);
        self.seed_rects.merge(); // grown rectangles overlap again
        true
    }

    /// Grid rectangles of the enabled static obstacles, recomputed when the layout changed.
    fn update_static_grid_rects(&mut self, input: &MaskPlanInput<'_>) {
        if self.grid_valid && self.grid_layout == input.layout {
            return;
        }
        self.static_grid_rects.clear();
        for (id, o) in input.obstacles.iter() {
            if o.is_active() && !o.common.dynamic {
                let r = obstacle_grid_rect(
                    input.obstacles,
                    id,
                    input.projection,
                    &input.domain,
                    &mut self.grid_scratch,
                );
                self.static_grid_rects.push(r);
            }
        }
        self.grid_layout = input.layout;
        self.grid_valid = true;
    }
}
