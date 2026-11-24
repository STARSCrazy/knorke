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
//!   `restore` re-validates and re-triangulates every obstacle (`SnapshotError::Invalid`) and
//!   therefore allocates; `save` does not.
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

use super::desc::{ObstaclePose, ShapeDesc, TileGridDesc};
use super::error::ObstacleError;
use super::stored::{StoredObstacle, outline_of, prepare_shape};
use crate::triangulate::Triangulator;
use knorke_math::{At, Vec2, World2};
use knorke_types::{
    BitSet, Column, Handle, Reader, Revision, Slots, Snapshot, SnapshotError, SnapshotValue, Writer,
};

/// Generation-checked handle of an obstacle.
pub type ObstacleId = Handle<StoredObstacle>;

/// The obstacles of one scene.
#[derive(Clone, Debug, Default)]
pub struct ObstacleStore {
    obstacles: Slots<StoredObstacle>,
    revision: Revision,
    static_version: Revision,
    dirty_static: BitSet,
    dirty_dynamic: BitSet,
    /// Active partial obstacles, kept up to date by every mutation.
    partial_active: u32,
    tri: Triangulator,
    scratch: Vec<Vec2>,
    /// `destroy_many`: sorted handle bits for the duplicate check.
    sorted: Vec<u64>,
}

fn batch(field: &'static str, index: usize, source: ObstacleError) -> ObstacleError {
    ObstacleError::Batch {
        field,
        index,
        source: Box::new(source),
    }
}

fn invalid(id: ObstacleId) -> ObstacleError {
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
fn partial_active(s: &StoredObstacle) -> u32 {
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
        let stored = Self::build_grid(desc)?;
        Ok(self.insert(stored))
    }

    /// Replaces an obstacle by a shape (the kind may change); its revision grows. Unchanged on
    /// error.
    ///
    /// # Errors
    /// Stale handle, or as [`create_shape`](Self::create_shape).
    pub fn set_shape(&mut self, id: ObstacleId, desc: &ShapeDesc<'_>) -> Result<(), ObstacleError> {
        if !self.obstacles.contains(id) {
            return Err(invalid(id));
        }
        let box_corners = prepare_shape(&mut self.tri, &mut self.scratch, desc)?;
        let (scratch, outline) = (&self.scratch, outline_of(desc, &box_corners));
        let mut next = |s: &mut StoredObstacle| s.assign_shape(desc, scratch, outline);
        Self::replace(&mut self.obstacles, &mut self.partial_active, id, &mut next)
            .map(|dynamic| self.note_change(id.index(), dynamic))
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
        let mut next = |s: &mut StoredObstacle| s.assign_tile_grid(desc);
        Self::replace(&mut self.obstacles, &mut self.partial_active, id, &mut next)
            .map(|dynamic| self.note_change(id.index(), dynamic))
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
        self.note_change(id.index(), stored.common.dynamic);
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
        let mut built = Vec::with_capacity(descs.len());
        for (i, d) in descs.iter().enumerate() {
            built.push(self.build_shape(d).map_err(|e| batch("descs", i, e))?);
        }
        out.reserve(built.len());
        for stored in built {
            out.push(self.insert(stored));
        }
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
        let mut built = Vec::with_capacity(descs.len());
        for (i, d) in descs.iter().enumerate() {
            built.push(Self::build_grid(d).map_err(|e| batch("descs", i, e))?);
        }
        out.reserve(built.len());
        for stored in built {
            out.push(self.insert(stored));
        }
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
        let mut built = Vec::with_capacity(descs.len());
        for (i, d) in descs.iter().enumerate() {
            built.push(self.build_shape(d).map_err(|e| batch("descs", i, e))?);
        }
        for (id, next) in ids.iter().zip(built) {
            let mut next = Some(next);
            let mut apply = |s: &mut StoredObstacle| {
                if let Some(n) = next.take() {
                    let revision = s.revision;
                    *s = n;
                    s.revision = revision;
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
            let mut apply = |s: &mut StoredObstacle| s.assign_tile_grid(d);
            if let Ok(dynamic) = Self::replace(
                &mut self.obstacles,
                &mut self.partial_active,
                *id,
                &mut apply,
            ) {
                self.note_change(id.index(), dynamic);
            }
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

    fn note_change(&mut self, index: u32, dynamic: bool) {
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

    fn build_shape(&mut self, desc: &ShapeDesc<'_>) -> Result<StoredObstacle, ObstacleError> {
        let box_corners = prepare_shape(&mut self.tri, &mut self.scratch, desc)?;
        let mut s = StoredObstacle::empty();
        s.assign_shape(desc, &self.scratch, outline_of(desc, &box_corners));
        Ok(s)
    }

    fn build_grid(desc: &TileGridDesc<'_>) -> Result<StoredObstacle, ObstacleError> {
        desc.validate()?;
        let mut s = StoredObstacle::empty();
        s.assign_tile_grid(desc);
        Ok(s)
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
    fn replace(
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

    /// Recomputes the counters after a restore.
    fn recount(&mut self) {
        self.partial_active = self.obstacles.iter().map(|(_, s)| partial_active(s)).sum();
    }
}

impl Snapshot for ObstacleStore {
    const ID: &'static str = "knorke.light2d.obstacles";
    const VERSION: u16 = 1;

    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.obstacles);
        w.put(&self.revision);
        w.put(&self.static_version);
        w.put(&self.dirty_static);
        w.put(&self.dirty_dynamic);
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
        self.recount();
        Ok(())
    }
}
