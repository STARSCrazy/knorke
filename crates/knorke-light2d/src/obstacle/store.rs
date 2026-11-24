// Origin: chiaro @ 9bc5b9b, src/shadows/ObstacleStore.hpp and ObstacleStore.cpp (create, replace,
// destroy, batches, counters and the static version), with core/SlotMap.hpp, core/Ids.hpp and
// core/Batch.hpp replaced by knorke_types::Slots. Tile cell editing, the cell log and patterns of
// the original store are P14/P15.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The obstacles of one scene (P13).
//!
//! Redesign against the C++ version:
//!
//! - `SlotMap`/`ObstacleId` (a `u32`) are [`Slots`] and generation-checked [`ObstacleId`]
//!   handles; a stale handle never reaches an obstacle that reused its slot.
//! - Every mutation validates first and leaves the state unchanged on error; batches are all or
//!   nothing and name the index of the offending element. Bulk input is a [`Column`] (ADR 0006).
//! - The store owns its working memory (triangulator, scratch triangles). Moving, enabling and
//!   re-describing obstacles allocates nothing once the buffers have grown to the sizes in use.
//! - Persistent state is a [`Snapshot`] section `knorke.light2d.obstacles` (ADR 0010).
//!   `restore` re-validates every obstacle and re-triangulates it to compare with the saved
//!   triangles (`SnapshotError::Invalid`). Neither `save` nor `restore` allocates once the
//!   buffers have grown to the sizes in use (P13b, P14).
//! - Triangles and outlines of all obstacles live in two arenas of the store ([`Span`]s in
//!   [`StoredObstacle`]), not in a `Vec` per obstacle; the cell channels and pattern cell
//!   indices of tile grids live in a byte arena and the pattern tables in a word arena (P14). Replacing an obstacle by a shape of the
//!   same size writes in place; otherwise the new data is appended and the old span becomes
//!   garbage, which the store compacts (into a spare buffer, so without allocation in steady
//!   state) once it exceeds the live data. Batch replacements always append.
//!
//! # Dirty tracking (for the shadow cache, P16)
//!
//! The incremental shadow mask must know what changed since it last drew. Two mechanisms, both
//! cheap and deterministic (ADR 0008, no callbacks):
//!
//! 1. **Versions for any number of passive observers.** [`ObstacleStore::revision`] rises with
//!    *every* change, [`ObstacleStore::static_version`] with every change of static geometry
//!    (create, replace, move or destroy of an obstacle that is not dynamic, and a replace that
//!    switches `dynamic`). Each obstacle has its own [`StoredObstacle::revision`], which rises
//!    when it is replaced (not when it is moved: position and rotation are read directly).
//!    A consumer remembers what it drew (revision, position, rotation per handle) and compares.
//! 2. **A dirty set for the one owner that consumes changes** (the cache):
//!    [`ObstacleStore::dirty_static`] and [`ObstacleStore::dirty_dynamic`] are [`BitSet`]s over
//!    the *slot index* ([`Handle::index`]) of every obstacle created, replaced, moved or
//!    destroyed since the last [`ObstacleStore::clear_dirty`]. Static and dynamic are kept apart
//!    because a static change forces a full rebuild while dynamic ones redraw local rectangles.
//!    A set bit whose slot is empty (or now holds another generation) means "removed here": the
//!    consumer compares with its own per-handle state. Only the owner calls `clear_dirty`.
//!
//! Both are part of the snapshot, so a restored store reports exactly what the saved one did.

use super::desc::{ObstaclePose, Shape, ShapeDesc, TileGridDesc};
use super::error::ObstacleError;
use super::patterns::{MAX_PATTERN_TABLE, MAX_PATTERN_VERTICES, check_fits_shape};
use super::store_edit::EditState;
use super::stored::{ObstacleKind, Span, StoredObstacle, outline_of, prepare_shape};
use crate::triangulate::Triangulator;
use knorke_math::{At, Vec2, World2, det};
use knorke_types::{
    BitSet, Column, Handle, Reader, Revision, Slots, Snapshot, SnapshotError, SnapshotValue, Writer,
};

/// Generation-checked handle of an obstacle.
pub type ObstacleId = Handle<StoredObstacle>;

/// A growing array that obstacles own ranges of.
#[derive(Clone, Debug)]
pub(super) struct Arena<T> {
    pub(super) data: Vec<T>,
    /// Target of a compaction, kept to reuse its capacity.
    spare: Vec<T>,
    /// Elements of `data` no obstacle refers to any more.
    pub(super) garbage: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            spare: Vec::new(),
            garbage: 0,
        }
    }
}

impl<T: Copy> Arena<T> {
    /// Smallest garbage worth compacting.
    const MIN_GARBAGE: usize = 64;

    pub(super) fn get(&self, span: Span) -> Option<&[T]> {
        let start = span.start as usize;
        self.data
            .as_slice()
            .get(start..start.checked_add(span.len as usize)?)
    }

    /// The span as a mutable slice (empty if it lies outside, which a checked store never has).
    pub(super) fn slice_mut(&mut self, span: Span) -> &mut [T] {
        let start = span.start as usize;
        self.data
            .get_mut(start..start + span.len as usize)
            .unwrap_or(&mut [])
    }

    /// The span of an obstacle of the store (always inside the arena).
    pub(super) fn slice(&self, span: Span) -> &[T] {
        self.get(span).unwrap_or(&[])
    }

    pub(super) fn push(&mut self, items: &[T]) -> Span {
        if items.is_empty() {
            return Span::EMPTY;
        }
        let start = u32::try_from(self.data.len()).expect("obstacle arena exceeds u32 elements");
        self.data.extend_from_slice(items);
        Span {
            start,
            len: items.len() as u32,
        }
    }

    /// Appends `n` copies of `value`.
    pub(super) fn push_fill(&mut self, n: usize, value: T) -> Span {
        if n == 0 {
            return Span::EMPTY;
        }
        let start = u32::try_from(self.data.len()).expect("obstacle arena exceeds u32 elements");
        self.data.resize(self.data.len() + n, value);
        Span {
            start,
            len: n as u32,
        }
    }

    pub(super) fn release(&mut self, span: Span) {
        self.garbage += span.len as usize;
    }

    /// Stores `items` in place of `old`: in place for the same length, else appended.
    pub(super) fn replace(&mut self, old: Span, items: &[T]) -> Span {
        if !old.is_empty() && old.len as usize == items.len() {
            let start = old.start as usize;
            self.data[start..start + items.len()].copy_from_slice(items);
            return old;
        }
        self.release(old);
        self.push(items)
    }

    pub(super) fn needs_compaction(&self) -> bool {
        self.garbage >= Self::MIN_GARBAGE && self.garbage * 2 > self.data.len()
    }

    pub(super) fn begin_compaction(&mut self) {
        self.spare.clear();
    }

    /// Copies a live span into the spare buffer and returns where it went.
    pub(super) fn relocate(&mut self, span: Span) -> Span {
        if span.is_empty() {
            return Span::EMPTY;
        }
        let start = span.start as usize;
        let moved = Span {
            start: self.spare.len() as u32,
            len: span.len,
        };
        self.spare
            .extend_from_slice(&self.data[start..start + span.len as usize]);
        moved
    }

    pub(super) fn finish_compaction(&mut self) {
        core::mem::swap(&mut self.data, &mut self.spare);
        self.spare.clear();
        self.garbage = 0;
    }
}

/// The obstacles of one scene.
#[derive(Clone, Debug, Default)]
pub struct ObstacleStore {
    pub(super) obstacles: Slots<StoredObstacle>,
    /// Local triangles of all polygons and circles.
    triangles: Arena<Vec2>,
    /// Local corners of all polygons.
    outlines: Arena<Vec2>,
    /// Cell channels of tile grids and pattern cell indices (P14).
    pub(super) bytes: Arena<u8>,
    /// Pattern tables of tile grids (P14).
    pub(super) words: Arena<u32>,
    /// Cell log and working memory of cell and pattern edits (P14).
    pub(super) edit: EditState,
    /// Batches: obstacles built but not yet inserted.
    pending: Vec<StoredObstacle>,
    revision: Revision,
    static_version: Revision,
    dirty_static: BitSet,
    dirty_dynamic: BitSet,
    /// Active partial obstacles, kept up to date by every mutation.
    pub(super) partial_active: u32,
    tri: Triangulator,
    scratch: Vec<Vec2>,
    /// `destroy_many`: sorted handle bits for the duplicate check.
    sorted: Vec<u64>,
}

pub(super) fn batch(field: &'static str, index: usize, source: ObstacleError) -> ObstacleError {
    ObstacleError::Batch {
        field,
        index,
        source: Box::new(source),
    }
}

pub(super) fn invalid(id: ObstacleId) -> ObstacleError {
    ObstacleError::InvalidHandle(id.to_bits())
}

fn same_length(what: &'static str, expected: usize, got: usize) -> Result<(), ObstacleError> {
    if expected == got {
        Ok(())
    } else {
        Err(ObstacleError::LengthMismatch {
            what,
            expected,
            got,
        })
    }
}

/// 1 if the obstacle counts for [`ObstacleStore::any_partial`].
pub(super) fn partial_active(s: &StoredObstacle) -> u32 {
    u32::from(s.is_partial() && s.is_active())
}

impl ObstacleStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    // ---- create, replace, destroy ----

    /// Adds a shape obstacle.
    ///
    /// # Errors
    /// The description violates a rule or the polygon is not simple.
    pub fn create_shape(&mut self, desc: &ShapeDesc<'_>) -> Result<ObstacleId, ObstacleError> {
        let stored = self.build_shape(desc)?;
        Ok(self.insert(stored))
    }

    /// Adds a tile grid obstacle.
    ///
    /// # Errors
    /// The description violates a rule.
    pub fn create_tile_grid(
        &mut self,
        desc: &TileGridDesc<'_>,
    ) -> Result<ObstacleId, ObstacleError> {
        let stored = self.build_grid(desc)?;
        Ok(self.insert(stored))
    }

    /// Replaces an obstacle by a shape (the kind may change); its revision grows. Unchanged on
    /// error.
    ///
    /// # Errors
    /// Stale handle, or as [`create_shape`](Self::create_shape).
    pub fn set_shape(&mut self, id: ObstacleId, desc: &ShapeDesc<'_>) -> Result<(), ObstacleError> {
        let old = self.obstacles.get(id).ok_or_else(|| invalid(id))?;
        check_fits_shape(old, desc)?;
        let box_corners = prepare_shape(&mut self.tri, &mut self.scratch, desc)?;
        let (old_triangles, old_outline, was_grid) = (old.triangles, old.outline, old.is_tile_grid());
        if was_grid {
            self.release_grid_data(id);
        }
        let triangles = self.triangles.replace(old_triangles, &self.scratch);
        let outline = self
            .outlines
            .replace(old_outline, outline_of(desc, &box_corners));
        let mut next = |s: &mut StoredObstacle| s.assign_shape(desc, triangles, outline);
        let result = Self::replace(&mut self.obstacles, &mut self.partial_active, id, &mut next)
            .map(|dynamic| self.note_change(id.index(), dynamic));
        self.compact_if_needed();
        result
    }

    /// Replaces an obstacle by a tile grid (the kind may change); its revision grows. Unchanged
    /// on error.
    ///
    /// # Errors
    /// Stale handle, or as [`create_tile_grid`](Self::create_tile_grid).
    pub fn set_tile_grid(
        &mut self,
        id: ObstacleId,
        desc: &TileGridDesc<'_>,
    ) -> Result<(), ObstacleError> {
        if !self.obstacles.contains(id) {
            return Err(invalid(id));
        }
        desc.validate()?;
        self.apply_grid(id, desc)
    }

    /// Moves and turns an obstacle. A move does not change the obstacle's revision, but it is a
    /// change for the dirty tracking.
    ///
    /// # Errors
    /// Stale handle or a non-finite position or rotation.
    pub fn set_transform(
        &mut self,
        id: ObstacleId,
        position: At<World2, Vec2>,
        rotation: f32,
    ) -> Result<(), ObstacleError> {
        let pose = ObstaclePose { position, rotation };
        let stored = self.obstacles.get_mut(id).ok_or_else(|| invalid(id))?;
        pose.validate()?;
        stored.common.position = position;
        stored.common.rotation = rotation;
        let dynamic = stored.common.dynamic;
        self.note_change(id.index(), dynamic);
        Ok(())
    }

    /// Removes an obstacle.
    ///
    /// # Errors
    /// Stale handle.
    pub fn destroy(&mut self, id: ObstacleId) -> Result<(), ObstacleError> {
        let stored = self.obstacles.remove(id).ok_or_else(|| invalid(id))?;
        self.partial_active -= partial_active(&stored);
        self.triangles.release(stored.triangles);
        self.outlines.release(stored.outline);
        self.note_change(id.index(), stored.common.dynamic);
        self.compact_if_needed();
        Ok(())
    }

    // ---- batches (all or nothing) ----

    /// Adds many shapes; the new handles are appended to `out` in order.
    ///
    /// # Errors
    /// [`ObstacleError::Batch`] naming the first bad element of `descs`; nothing is added.
    pub fn create_shapes<'a>(
        &mut self,
        descs: impl Column<ShapeDesc<'a>>,
        out: &mut Vec<ObstacleId>,
    ) -> Result<(), ObstacleError> {
        self.stage_shapes(&descs)?;
        self.insert_pending(out);
        Ok(())
    }

    /// Adds many tile grids; the new handles are appended to `out` in order.
    ///
    /// # Errors
    /// [`ObstacleError::Batch`] naming the first bad element of `descs`; nothing is added.
    pub fn create_tile_grids<'a>(
        &mut self,
        descs: impl Column<TileGridDesc<'a>>,
        out: &mut Vec<ObstacleId>,
    ) -> Result<(), ObstacleError> {
        for (i, d) in descs.iter().enumerate() {
            d.validate().map_err(|e| batch("descs", i, e))?;
        }
        self.pending.clear();
        for d in descs.iter() {
            // Validated above, so this cannot fail and the arenas never need a rollback.
            if let Ok(stored) = self.build_grid(d) {
                self.pending.push(stored);
            }
        }
        self.insert_pending(out);
        Ok(())
    }

    /// Replaces many obstacles by shapes.
    ///
    /// # Errors
    /// Differing lengths, or [`ObstacleError::Batch`] naming the first bad element (`ids` or
    /// `descs`); nothing changes.
    pub fn set_shapes<'a>(
        &mut self,
        ids: impl Column<ObstacleId>,
        descs: impl Column<ShapeDesc<'a>>,
    ) -> Result<(), ObstacleError> {
        same_length("descs", ids.len(), descs.len())?;
        self.check_handles(&ids)?;
        for (i, (id, d)) in ids.iter().zip(descs.iter()).enumerate() {
            if let Some(current) = self.obstacles.get(*id) {
                check_fits_shape(current, d).map_err(|e| batch("descs", i, e))?;
            }
        }
        self.stage_shapes(&descs)?;
        let mut pending = core::mem::take(&mut self.pending);
        for (id, next) in ids.iter().zip(pending.drain(..)) {
            self.release_geometry(*id);
            if self.obstacles.get(*id).is_some_and(|s| s.is_tile_grid()) {
                self.release_grid_data(*id);
            }
            let mut next = Some(next);
            let mut apply = |s: &mut StoredObstacle| {
                if let Some(n) = next.take() {
                    s.replace_with(n);
                }
            };
            if let Ok(dynamic) = Self::replace(
                &mut self.obstacles,
                &mut self.partial_active,
                *id,
                &mut apply,
            ) {
                self.note_change(id.index(), dynamic);
            }
        }
        self.pending = pending;
        self.compact_if_needed();
        Ok(())
    }

    /// Replaces many obstacles by tile grids.
    ///
    /// # Errors
    /// Differing lengths, or [`ObstacleError::Batch`] naming the first bad element (`ids` or
    /// `descs`); nothing changes.
    pub fn set_tile_grids<'a>(
        &mut self,
        ids: impl Column<ObstacleId>,
        descs: impl Column<TileGridDesc<'a>>,
    ) -> Result<(), ObstacleError> {
        same_length("descs", ids.len(), descs.len())?;
        self.check_handles(&ids)?;
        for (i, d) in descs.iter().enumerate() {
            d.validate().map_err(|e| batch("descs", i, e))?;
        }
        for (id, d) in ids.iter().zip(descs.iter()) {
            // Cannot fail: handles and descriptions were checked.
            let _ = self.apply_grid(*id, d);
        }
        Ok(())
    }

    /// Moves and turns many obstacles.
    ///
    /// # Errors
    /// Differing lengths, or [`ObstacleError::Batch`] naming the first bad element (`ids` or
    /// `poses`); nothing moves.
    pub fn set_transforms(
        &mut self,
        ids: impl Column<ObstacleId>,
        poses: impl Column<ObstaclePose>,
    ) -> Result<(), ObstacleError> {
        same_length("poses", ids.len(), poses.len())?;
        self.check_handles(&ids)?;
        for (i, p) in poses.iter().enumerate() {
            p.validate().map_err(|e| batch("poses", i, e))?;
        }
        for (id, p) in ids.iter().zip(poses.iter()) {
            // Cannot fail: handles and values were checked.
            let _ = self.set_transform(*id, p.position, p.rotation);
        }
        Ok(())
    }

    /// Removes many obstacles; stale and repeated handles are errors.
    ///
    /// # Errors
    /// [`ObstacleError::Batch`] naming the first stale or repeated element of `ids`; nothing is
    /// removed.
    pub fn destroy_many(&mut self, ids: impl Column<ObstacleId>) -> Result<(), ObstacleError> {
        self.check_handles(&ids)?;
        self.sorted.clear();
        self.sorted.extend(ids.iter().map(|id| id.to_bits()));
        self.sorted.sort_unstable();
        if self.sorted.windows(2).any(|w| w[0] == w[1]) {
            let index = (0..ids.len())
                .find(|&i| {
                    ids.get(i).is_some_and(|id| {
                        (0..i).any(|j| ids.get(j).is_some_and(|other| other == id))
                    })
                })
                .unwrap_or(0);
            return Err(batch(
                "ids",
                index,
                ObstacleError::Unsupported("an obstacle handle appears twice"),
            ));
        }
        for id in ids.iter() {
            let _ = self.destroy(*id);
        }
        Ok(())
    }

    // ---- reading ----

    /// Number of obstacles.
    pub fn len(&self) -> usize {
        self.obstacles.len()
    }

    /// `true` if there are no obstacles.
    pub fn is_empty(&self) -> bool {
        self.obstacles.is_empty()
    }

    /// The obstacle behind a handle, or `None` for a stale handle.
    pub fn get(&self, id: ObstacleId) -> Option<&StoredObstacle> {
        self.obstacles.get(id)
    }

    /// The local-space triangles (three vertices each) of a polygon or circle; empty for tile
    /// grids and stale handles.
    pub fn triangles(&self, id: ObstacleId) -> &[Vec2] {
        self.obstacles
            .get(id)
            .map_or(&[], |s| self.triangles.slice(s.triangles))
    }

    /// The local corners of a polygon (a box has four); empty for circles, tile grids and stale
    /// handles.
    pub fn outline(&self, id: ObstacleId) -> &[Vec2] {
        self.obstacles
            .get(id)
            .map_or(&[], |s| self.outlines.slice(s.outline))
    }

    /// The local triangles of an obstacle rotated by its rotation (degrees, counter-clockwise)
    /// and moved by its position, appended to `out`. Nothing for a stale handle.
    pub fn append_world_triangles(&self, id: ObstacleId, out: &mut Vec<At<World2, Vec2>>) {
        let Some(s) = self.obstacles.get(id) else {
            return;
        };
        let local = self.triangles.slice(s.triangles);
        let rad = s.common.rotation * (core::f32::consts::PI / 180.0);
        let (sin, cos) = det::sin_cos(rad);
        let p = s.common.position.raw();
        out.reserve(local.len());
        out.extend(local.iter().map(|v| {
            At::new(Vec2::new(
                cos * v.x - sin * v.y + p.x,
                sin * v.x + cos * v.y + p.y,
            ))
        }));
    }

    /// Every obstacle in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (ObstacleId, &StoredObstacle)> {
        self.obstacles.iter()
    }

    /// An active obstacle is dynamic.
    pub fn any_dynamic_enabled(&self) -> bool {
        self.iter().any(|(_, s)| s.common.dynamic && s.is_active())
    }

    /// An active obstacle has a finite height.
    pub fn any_finite_height(&self) -> bool {
        self.iter()
            .any(|(_, s)| s.is_active() && s.has_finite_height())
    }

    /// An active obstacle is infinitely tall.
    pub fn any_infinite_enabled(&self) -> bool {
        self.iter()
            .any(|(_, s)| s.is_active() && !s.has_finite_height())
    }

    /// An active obstacle is partly transparent.
    pub fn any_partial(&self) -> bool {
        self.partial_active > 0
    }

    /// Highest finite top among the active obstacles, `0` without any.
    pub fn max_finite_top(&self) -> f32 {
        let mut top = 0.0;
        for (_, s) in self.iter() {
            if s.is_active() && s.has_finite_height() {
                top = knorke_math::det::max(top, s.finite_top());
            }
        }
        top
    }

    // ---- dirty tracking, see the module docs ----

    /// Rises with every change of any obstacle.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Rises with every change of static geometry (a change of a non-dynamic obstacle, or a
    /// replace that switches `dynamic`).
    pub fn static_version(&self) -> Revision {
        self.static_version
    }

    /// Slot indices of obstacles with a static change since the last [`clear_dirty`](Self::clear_dirty).
    pub fn dirty_static(&self) -> &BitSet {
        &self.dirty_static
    }

    /// Slot indices of obstacles with a dynamic change since the last [`clear_dirty`](Self::clear_dirty).
    pub fn dirty_dynamic(&self) -> &BitSet {
        &self.dirty_dynamic
    }

    /// `true` if some obstacle changed since the last [`clear_dirty`](Self::clear_dirty).
    pub fn is_dirty(&self) -> bool {
        !self.dirty_static.is_empty() || !self.dirty_dynamic.is_empty()
    }

    /// The owner of the dirty sets (the shadow cache) calls this after it consumed them. The
    /// versions are not affected.
    pub fn clear_dirty(&mut self) {
        self.dirty_static.clear();
        self.dirty_dynamic.clear();
    }

    // ---- internals ----

    pub(super) fn note_change(&mut self, index: u32, dynamic: bool) {
        self.revision.bump();
        if dynamic {
            self.dirty_dynamic.insert(index as usize);
        } else {
            self.static_version.bump();
            self.dirty_static.insert(index as usize);
        }
    }

    fn insert(&mut self, stored: StoredObstacle) -> ObstacleId {
        let partial = partial_active(&stored);
        let dynamic = stored.common.dynamic;
        let id = self.obstacles.insert(stored);
        self.partial_active += partial;
        self.note_change(id.index(), dynamic);
        id
    }

    /// Builds a shape obstacle, appending its geometry to the arenas.
    fn build_shape(&mut self, desc: &ShapeDesc<'_>) -> Result<StoredObstacle, ObstacleError> {
        let box_corners = prepare_shape(&mut self.tri, &mut self.scratch, desc)?;
        let triangles = self.triangles.push(&self.scratch);
        let outline = self.outlines.push(outline_of(desc, &box_corners));
        let mut s = StoredObstacle::empty();
        s.assign_shape(desc, triangles, outline);
        Ok(s)
    }

    /// Builds every shape into `pending`; on the first error the arenas and `pending` are as
    /// before.
    fn stage_shapes<'a>(
        &mut self,
        descs: &impl Column<ShapeDesc<'a>>,
    ) -> Result<(), ObstacleError> {
        let (triangles, outlines) = (self.triangles.data.len(), self.outlines.data.len());
        self.pending.clear();
        for (i, d) in descs.iter().enumerate() {
            match self.build_shape(d) {
                Ok(stored) => self.pending.push(stored),
                Err(e) => {
                    self.pending.clear();
                    self.triangles.data.truncate(triangles);
                    self.outlines.data.truncate(outlines);
                    return Err(batch("descs", i, e));
                }
            }
        }
        Ok(())
    }

    fn insert_pending(&mut self, out: &mut Vec<ObstacleId>) {
        let mut pending = core::mem::take(&mut self.pending);
        out.reserve(pending.len());
        for stored in pending.drain(..) {
            out.push(self.insert(stored));
        }
        self.pending = pending;
    }

    /// Marks the geometry of an obstacle as garbage (its handle is valid, else nothing).
    fn release_geometry(&mut self, id: ObstacleId) {
        if let Some(s) = self.obstacles.get(id) {
            self.triangles.release(s.triangles);
            self.outlines.release(s.outline);
        }
    }

    /// Squeezes out the garbage of the arenas once it outweighs the live data.
    pub(super) fn compact_if_needed(&mut self) {
        if self.triangles.needs_compaction() {
            self.triangles.begin_compaction();
            for (_, s) in self.obstacles.iter_mut() {
                s.triangles = self.triangles.relocate(s.triangles);
            }
            self.triangles.finish_compaction();
        }
        if self.outlines.needs_compaction() {
            self.outlines.begin_compaction();
            for (_, s) in self.obstacles.iter_mut() {
                s.outline = self.outlines.relocate(s.outline);
            }
            self.outlines.finish_compaction();
        }
        if self.bytes.needs_compaction() {
            self.bytes.begin_compaction();
            for (_, s) in self.obstacles.iter_mut() {
                let g = &mut s.grid;
                g.solid = self.bytes.relocate(g.solid);
                g.base_levels = self.bytes.relocate(g.base_levels);
                g.opacity = self.bytes.relocate(g.opacity);
                g.edges = self.bytes.relocate(g.edges);
                s.patterns.cells = self.bytes.relocate(s.patterns.cells);
            }
            self.bytes.finish_compaction();
        }
        if self.words.needs_compaction() {
            self.words.begin_compaction();
            for (_, s) in self.obstacles.iter_mut() {
                s.patterns.table = self.words.relocate(s.patterns.table);
            }
            self.words.finish_compaction();
        }
    }

    /// Builds a tile grid obstacle, appending its channels to the byte arena.
    fn build_grid(&mut self, desc: &TileGridDesc<'_>) -> Result<StoredObstacle, ObstacleError> {
        desc.validate()?;
        let channels = [
            self.bytes.push(desc.solid),
            self.bytes.push(desc.base_levels.unwrap_or(&[])),
            self.bytes.push(desc.opacity.unwrap_or(&[])),
            self.bytes.push(desc.edges.unwrap_or(&[])),
        ];
        let mut s = StoredObstacle::empty();
        s.assign_tile_grid(desc, channels);
        Ok(s)
    }

    /// Replaces an obstacle by a validated tile grid: the channels are written in place when the
    /// size is the same, else appended; patterns stay as far as they fit.
    fn apply_grid(&mut self, id: ObstacleId, desc: &TileGridDesc<'_>) -> Result<(), ObstacleError> {
        let old = self.obstacles.get(id).ok_or_else(|| invalid(id))?;
        let (grid, cells, was_grid) = (old.grid, old.patterns.cells, old.is_tile_grid());
        self.release_geometry(id);
        let (columns, rows) = (desc.columns, desc.rows);
        if was_grid && (grid.columns != columns || grid.rows != rows) {
            self.bytes.release(cells); // the cell indices go with the old size
        }
        let old = if was_grid {
            [grid.solid, grid.base_levels, grid.opacity, grid.edges]
        } else {
            [Span::EMPTY; 4]
        };
        let channels = [
            self.bytes.replace(old[0], desc.solid),
            self.bytes.replace(old[1], desc.base_levels.unwrap_or(&[])),
            self.bytes.replace(old[2], desc.opacity.unwrap_or(&[])),
            self.bytes.replace(old[3], desc.edges.unwrap_or(&[])),
        ];
        let mut next = |s: &mut StoredObstacle| s.assign_tile_grid(desc, channels);
        let result = Self::replace(&mut self.obstacles, &mut self.partial_active, id, &mut next)
            .map(|dynamic| self.note_change(id.index(), dynamic));
        self.compact_if_needed();
        result
    }

    /// Marks the channels and pattern spans of a tile grid as garbage (the obstacle is about to
    /// stop being a grid).
    fn release_grid_data(&mut self, id: ObstacleId) {
        if let Some(s) = self.obstacles.get(id) {
            let g = s.grid;
            for span in [g.solid, g.base_levels, g.opacity, g.edges, s.patterns.cells] {
                self.bytes.release(span);
            }
            self.words.release(s.patterns.table);
        }
    }

    fn check_handles(&self, ids: &impl Column<ObstacleId>) -> Result<(), ObstacleError> {
        for (i, id) in ids.iter().enumerate() {
            if !self.obstacles.contains(*id) {
                return Err(batch("ids", i, invalid(*id)));
            }
        }
        Ok(())
    }

    /// Runs `change` on the obstacle, grows its revision and keeps the partial counter up to
    /// date. Returns whether the change counts as dynamic (both before and after).
    pub(super) fn replace(
        obstacles: &mut Slots<StoredObstacle>,
        partial_active_count: &mut u32,
        id: ObstacleId,
        change: &mut dyn FnMut(&mut StoredObstacle),
    ) -> Result<bool, ObstacleError> {
        let current = obstacles.get_mut(id).ok_or_else(|| invalid(id))?;
        let was_dynamic = current.common.dynamic;
        let before = partial_active(current);
        change(current);
        current.revision = current.revision.wrapping_add(1);
        *partial_active_count = *partial_active_count - before + partial_active(current);
        Ok(was_dynamic && current.common.dynamic)
    }

    /// Checks a restored store: every span lies inside its arena and the saved triangles are
    /// those of the described shape. Recomputes the counters and the garbage of the arenas.
    fn check_restored(&mut self) -> Result<(), SnapshotError> {
        const BAD: SnapshotError = SnapshotError::Invalid("obstacle violates the obstacle rules");
        const BAD_SPAN: SnapshotError =
            SnapshotError::Invalid("obstacle geometry outside its arena");
        let mut live = (0usize, 0usize);
        for (_, s) in self.obstacles.iter() {
            let (outline, shape) = match s.kind {
                ObstacleKind::TileGrid => {
                    if !s.triangles.is_empty() || !s.outline.is_empty() {
                        return Err(BAD_SPAN);
                    }
                    continue;
                }
                ObstacleKind::Circle => {
                    if !s.outline.is_empty() {
                        return Err(BAD_SPAN);
                    }
                    (&[][..], Shape::Circle { radius: s.radius })
                }
                ObstacleKind::Polygon => {
                    let outline = self.outlines.get(s.outline).ok_or(BAD_SPAN)?;
                    if s.box_shape && outline.len() != 4 {
                        return Err(BAD);
                    }
                    (outline, Shape::Polygon(outline))
                }
            };
            let saved = self.triangles.get(s.triangles).ok_or(BAD_SPAN)?;
            let desc = ShapeDesc {
                common: s.common,
                shape,
            };
            prepare_shape(&mut self.tri, &mut self.scratch, &desc).map_err(|_| BAD)?;
            if self.scratch != saved {
                return Err(SnapshotError::Invalid(
                    "obstacle triangles do not match its shape",
                ));
            }
            live.0 += s.triangles.len as usize;
            live.1 += outline.len();
        }
        self.triangles.garbage = self.triangles.data.len().saturating_sub(live.0);
        self.outlines.garbage = self.outlines.data.len().saturating_sub(live.1);
        self.check_restored_grids()?;
        self.partial_active = self.obstacles.iter().map(|(_, s)| partial_active(s)).sum();
        Ok(())
    }
}

impl ObstacleStore {
    /// The part of the restore check that concerns tile grids and pattern assignments: spans
    /// inside the arenas, values by the description rules, derived counts, pattern limits.
    /// Recomputes the garbage of the byte and word arenas.
    fn check_restored_grids(&mut self) -> Result<(), SnapshotError> {
        const BAD: SnapshotError = SnapshotError::Invalid("obstacle violates the obstacle rules");
        const BAD_SPAN: SnapshotError = SnapshotError::Invalid("obstacle cells outside their arena");
        const BAD_PATTERN: SnapshotError = SnapshotError::Invalid("obstacle pattern assignment");
        let (mut live_bytes, mut live_words) = (0usize, 0usize);
        for (_, s) in self.obstacles.iter_mut() {
            let p = s.patterns;
            let table = self.words.get(p.table).ok_or(BAD_SPAN)?;
            let cells = self.bytes.get(p.cells).ok_or(BAD_SPAN)?;
            live_words += table.len();
            live_bytes += cells.len();
            if table.len() > MAX_PATTERN_TABLE || p.refs as usize > table.len() + 1 {
                return Err(BAD_PATTERN);
            }
            if s.kind != ObstacleKind::TileGrid {
                let big = s.kind == ObstacleKind::Polygon
                    && p.pattern != 0
                    && s.outline.len as usize > MAX_PATTERN_VERTICES;
                if big || !table.is_empty() || !cells.is_empty() {
                    return Err(BAD_PATTERN);
                }
                continue;
            }
            let g = s.grid;
            let n = g.columns as usize * g.rows as usize;
            let channel = |span: Span| self.bytes.get(span).ok_or(BAD_SPAN);
            let (solid, base, opacity, edges) = (
                channel(g.solid)?,
                channel(g.base_levels)?,
                channel(g.opacity)?,
                channel(g.edges)?,
            );
            let some = |c: &[u8]| (!c.is_empty()).then_some(());
            let desc = TileGridDesc {
                common: s.common,
                columns: g.columns,
                rows: g.rows,
                cell_size: g.cell_size,
                level_height: g.level_height,
                edge_width: g.edge_width,
                solid,
                base_levels: some(base).map(|_| base),
                opacity: some(opacity).map(|_| opacity),
                edges: some(edges).map(|_| edges),
            };
            desc.validate().map_err(|_| BAD)?;
            if (!cells.is_empty() && cells.len() != n)
                || cells.iter().any(|&i| usize::from(i) > table.len())
            {
                return Err(BAD_PATTERN);
            }
            s.grid.count(solid, opacity, edges);
            live_bytes += solid.len() + base.len() + opacity.len() + edges.len();
        }
        self.bytes.garbage = self.bytes.data.len().saturating_sub(live_bytes);
        self.words.garbage = self.words.data.len().saturating_sub(live_words);
        self.partial_active = self.obstacles.iter().map(|(_, s)| partial_active(s)).sum();
        Ok(())
    }
}

impl Snapshot for ObstacleStore {
    const ID: &'static str = "knorke.light2d.obstacles";
    const VERSION: u16 = 3;

    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.obstacles);
        w.put(&self.revision);
        w.put(&self.static_version);
        w.put(&self.dirty_static);
        w.put(&self.dirty_dynamic);
        w.put(&self.triangles.data);
        w.put(&self.outlines.data);
        w.put(&self.bytes.data);
        w.put(&self.words.data);
        self.edit.log.save(w);
    }

    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        if version != Self::VERSION {
            return Err(SnapshotError::UnsupportedVersion {
                id: Self::ID,
                version,
            });
        }
        self.obstacles.get_into(r)?;
        self.revision = r.get()?;
        self.static_version = r.get()?;
        self.dirty_static.get_into(r)?;
        self.dirty_dynamic.get_into(r)?;
        self.triangles.data.get_into(r)?;
        self.outlines.data.get_into(r)?;
        self.bytes.data.get_into(r)?;
        self.words.data.get_into(r)?;
        self.edit.log.restore(r)?;
        self.check_restored()
    }
}
