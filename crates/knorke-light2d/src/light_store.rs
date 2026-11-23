// Origin: chiaro @ 9bc5b9b, src/lighting/LightStore.hpp and LightStore.cpp (LightStore), with
// core/SlotMap.hpp, core/Ids.hpp and core/Batch.hpp replaced by knorke_types::Slots.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The lights of one scene (P7).
//!
//! Redesign against the C++ version:
//!
//! - `SlotMap` and `LightId` (a `u32`) are [`Slots`] and generation-checked [`LightId`] handles.
//!   A stale handle never reaches a light that reused its slot.
//! - The store does not hold pointers to the profile, animation and mask stores. Every mutation
//!   that can name one takes a [`LightResources`] (the host's answer to "does this handle
//!   exist?"), so the store stays plain, cloneable state.
//! - Bulk input is a [`Column`] (ADR 0006); results go into a host-owned `Vec`.
//! - Every mutation validates first and leaves the state unchanged on error; batches are all or
//!   nothing and name the index of the offending element.
//! - Persistent lights are simulation state and implement [`Snapshot`] (ADR 0010): the section
//!   keeps the slots (so handles stay identical), and only the descriptions are written; derived
//!   values are recomputed and descriptions are re-validated on restore (`SnapshotError::Invalid`).
//!   Transient frame lights are per-frame input and not part of the state: a restore clears them.
//! - After `set_frame_lights` has seen its largest count, steady-state frames allocate nothing.
//!   `restore` into a store that already has the capacity allocates nothing either.

use crate::light::{
    AnimationId, LightDesc, LightError, LightId, LightPosition, LightRef, LightResources,
    MAX_FRAME_LIGHTS, MaskId, ProfileId, StoredLight,
};
use knorke_math::{At, Vec2, World2};
use knorke_types::{Column, Reader, Slots, Snapshot, SnapshotError, SnapshotValue, Writer};

/// The lights of one scene: persistent lights addressed by handle, plus the transient lights
/// of the next frame.
#[derive(Clone, Debug, Default)]
pub struct LightStore {
    lights: Slots<StoredLight>,
    /// Pool; the first `frame_count` entries are the transient lights.
    frame_lights: Vec<StoredLight>,
    frame_count: usize,
    /// `destroy_many`: sorted handle bits for the duplicate check.
    sorted: Vec<u64>,
}

fn batch(field: &'static str, index: usize, source: LightError) -> LightError {
    LightError::Batch {
        field,
        index,
        source: Box::new(source),
    }
}

fn invalid(id: LightId) -> LightError {
    LightError::InvalidHandle(id.to_bits())
}

fn check_all(desc: &LightDesc, resources: &impl LightResources) -> Result<(), LightError> {
    desc.validate()?;
    desc.check_resources(resources)
}

impl LightStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of persistent lights.
    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// `true` if there are no persistent lights.
    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    /// Adds a light.
    ///
    /// # Errors
    /// The description violates a rule or names a profile, animation or mask that does not exist.
    pub fn create(
        &mut self,
        desc: &LightDesc,
        resources: &impl LightResources,
    ) -> Result<LightId, LightError> {
        check_all(desc, resources)?;
        Ok(self.lights.insert(StoredLight::derive(desc)))
    }

    /// Replaces the description of a light; unchanged on error.
    ///
    /// # Errors
    /// Stale handle, or as [`create`](Self::create).
    pub fn update(
        &mut self,
        id: LightId,
        desc: &LightDesc,
        resources: &impl LightResources,
    ) -> Result<(), LightError> {
        if !self.lights.contains(id) {
            return Err(invalid(id));
        }
        check_all(desc, resources)?;
        if let Some(stored) = self.lights.get_mut(id) {
            *stored = StoredLight::derive(desc);
        }
        Ok(())
    }

    /// The description of a light as given.
    ///
    /// # Errors
    /// Stale handle.
    pub fn get(&self, id: LightId) -> Result<&LightDesc, LightError> {
        self.stored(id).map(|s| &s.desc).ok_or_else(|| invalid(id))
    }

    /// The light with its derived values, or `None` for a stale handle.
    pub fn stored(&self, id: LightId) -> Option<&StoredLight> {
        self.lights.get(id)
    }

    /// Moves a light; the derived values do not depend on the position.
    ///
    /// # Errors
    /// Stale handle, a non-finite position or a negative height.
    pub fn set_position(
        &mut self,
        id: LightId,
        xy: At<World2, Vec2>,
        height: crate::light::Height,
    ) -> Result<(), LightError> {
        let stored = self.lights.get_mut(id).ok_or_else(|| invalid(id))?;
        LightPosition::new(xy, height).validate()?;
        stored.desc.position = xy;
        stored.desc.height = height;
        Ok(())
    }

    /// Removes a light.
    ///
    /// # Errors
    /// Stale handle.
    pub fn destroy(&mut self, id: LightId) -> Result<(), LightError> {
        self.lights.remove(id).map(drop).ok_or_else(|| invalid(id))
    }

    /// Adds many lights, all or nothing. The new handles are appended to `out` in order.
    ///
    /// # Errors
    /// [`LightError::Batch`] naming the first bad element of `descs`; nothing is added.
    pub fn create_many(
        &mut self,
        descs: impl Column<LightDesc>,
        resources: &impl LightResources,
        out: &mut Vec<LightId>,
    ) -> Result<(), LightError> {
        for (i, d) in descs.iter().enumerate() {
            check_all(d, resources).map_err(|e| batch("descs", i, e))?;
        }
        out.reserve(descs.len());
        for d in descs.iter() {
            out.push(self.lights.insert(StoredLight::derive(d)));
        }
        Ok(())
    }

    /// Replaces the descriptions of many lights, all or nothing.
    ///
    /// # Errors
    /// [`LightError::Batch`] naming the first bad element (`ids` or `descs`); nothing changes.
    pub fn update_many(
        &mut self,
        ids: impl Column<LightId>,
        descs: impl Column<LightDesc>,
        resources: &impl LightResources,
    ) -> Result<(), LightError> {
        same_length("descs", ids.len(), descs.len())?;
        for (i, (id, d)) in ids.iter().zip(descs.iter()).enumerate() {
            if !self.lights.contains(*id) {
                return Err(batch("ids", i, invalid(*id)));
            }
            check_all(d, resources).map_err(|e| batch("descs", i, e))?;
        }
        for (id, d) in ids.iter().zip(descs.iter()) {
            if let Some(stored) = self.lights.get_mut(*id) {
                *stored = StoredLight::derive(d);
            }
        }
        Ok(())
    }

    /// Moves many lights, all or nothing.
    ///
    /// # Errors
    /// [`LightError::Batch`] naming the first bad element (`ids` or `positions`); nothing moves.
    pub fn set_positions(
        &mut self,
        ids: impl Column<LightId>,
        positions: impl Column<LightPosition>,
    ) -> Result<(), LightError> {
        same_length("positions", ids.len(), positions.len())?;
        for (i, (id, p)) in ids.iter().zip(positions.iter()).enumerate() {
            if !self.lights.contains(*id) {
                return Err(batch("ids", i, invalid(*id)));
            }
            p.validate().map_err(|e| batch("positions", i, e))?;
        }
        for (id, p) in ids.iter().zip(positions.iter()) {
            if let Some(stored) = self.lights.get_mut(*id) {
                stored.desc.position = p.xy;
                stored.desc.height = p.height;
            }
        }
        Ok(())
    }

    /// Removes many lights, all or nothing; stale and duplicate handles are errors.
    ///
    /// # Errors
    /// [`LightError::Batch`] naming the first stale or repeated element of `ids`; nothing is removed.
    pub fn destroy_many(&mut self, ids: impl Column<LightId>) -> Result<(), LightError> {
        for (i, id) in ids.iter().enumerate() {
            if !self.lights.contains(*id) {
                return Err(batch("ids", i, invalid(*id)));
            }
        }
        // Duplicates: find the first element equal to an earlier one through a sorted copy of
        // (bits, index); the scratch is reused.
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
                LightError::Unsupported("a light handle appears twice"),
            ));
        }
        for id in ids.iter() {
            self.lights.remove(*id);
        }
        Ok(())
    }

    /// Replaces the transient lights of the next frame, all or nothing. The pool grows to the
    /// largest count seen and is never shrunk, so a steady count allocates nothing after the
    /// first frame.
    ///
    /// # Errors
    /// More than [`MAX_FRAME_LIGHTS`], or [`LightError::Batch`] naming the first bad element;
    /// the previous frame lights stay.
    pub fn set_frame_lights(
        &mut self,
        descs: impl Column<LightDesc>,
        resources: &impl LightResources,
    ) -> Result<(), LightError> {
        let count = descs.len();
        if count > MAX_FRAME_LIGHTS {
            return Err(LightError::TooManyFrameLights {
                count,
                max: MAX_FRAME_LIGHTS,
            });
        }
        for (i, d) in descs.iter().enumerate() {
            check_all(d, resources).map_err(|e| batch("descs", i, e))?;
        }
        for (i, d) in descs.iter().enumerate() {
            let derived = StoredLight::derive(d);
            match self.frame_lights.get_mut(i) {
                Some(slot) => *slot = derived,
                None => self.frame_lights.push(derived),
            }
        }
        self.frame_count = count;
        Ok(())
    }

    /// After a successful render: the transient lights were drawn once. Keeps the pool.
    pub fn clear_frame_lights(&mut self) {
        self.frame_count = 0;
    }

    /// Number of transient lights of the next frame.
    pub fn frame_light_count(&self) -> usize {
        self.frame_count
    }

    /// Every light: the persistent ones in slot order, then the transient ones in list order.
    pub fn iter(&self) -> impl Iterator<Item = (LightRef, &StoredLight)> {
        self.lights
            .iter()
            .map(|(id, l)| (LightRef::Persistent(id), l))
            .chain(
                self.frame_lights[..self.frame_count]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (LightRef::Transient(i as u32), l)),
            )
    }

    /// Number of lights that use `profile`.
    pub fn profile_users(&self, profile: ProfileId) -> usize {
        self.iter()
            .filter(|(_, l)| l.desc.profile == Some(profile))
            .count()
    }

    /// Number of lights that use `animation`.
    pub fn animation_users(&self, animation: AnimationId) -> usize {
        self.iter()
            .filter(|(_, l)| l.desc.animation == Some(animation))
            .count()
    }

    /// Number of lights that use `mask`.
    pub fn mask_users(&self, mask: MaskId) -> usize {
        self.iter()
            .filter(|(_, l)| l.desc.mask == Some(mask))
            .count()
    }
}

fn same_length(what: &'static str, expected: usize, got: usize) -> Result<(), LightError> {
    if expected == got {
        Ok(())
    } else {
        Err(LightError::LengthMismatch {
            what,
            expected,
            got,
        })
    }
}

impl Snapshot for LightStore {
    const ID: &'static str = "knorke.light2d.lights";
    const VERSION: u16 = 1;

    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.lights);
    }

    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        if version != Self::VERSION {
            return Err(SnapshotError::UnsupportedVersion {
                id: Self::ID,
                version,
            });
        }
        self.frame_count = 0;
        self.lights.get_into(r)
    }
}
