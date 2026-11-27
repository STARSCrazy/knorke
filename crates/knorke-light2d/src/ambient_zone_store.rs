// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/AmbientZoneStore.hpp, src/lighting/AmbientZoneStore.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The ambient zones of one scene (P11): a handle-based store with a fold order.
//!
//! Each zone gets a creation number at [`AmbientZoneStore::create`] that [`AmbientZoneStore::set`]
//! keeps. The fold order is (priority, creation number) ascending and is kept sorted on every
//! change, so iterating takes no lock and needs no mutable access. Every live polygon zone owns
//! a row of the vertex texture, the lowest free one at create (or at the set that makes it a
//! polygon), independent of the slot index, so the texture needs
//! [`AmbientZoneStore::polygon_row_count`] rows.
//!
//! Redesign against the C++ version: `SlotMap` is [`Slots`]; handles are
//! [`AmbientZoneId`]; the C++ `Reserve` (allocation before mutation) is not needed in Rust, since
//! allocation failure aborts; the handle-0 case is gone because a `Handle` is never 0.

use crate::ambient_zone::{StoredZone, ZoneError, ZoneErrorKind, ZoneShape};
use knorke_types::{Handle, Slots};

/// Most zones alive at once.
pub const MAX_ZONES: usize = 4096;

/// Handle kind of an ambient zone.
#[derive(Clone, Copy, Debug)]
pub struct AmbientZoneTag;

/// Handle of an ambient zone in an [`AmbientZoneStore`].
pub type AmbientZoneId = Handle<AmbientZoneTag>;

#[derive(Clone, Debug)]
struct Entry {
    zone: StoredZone,
    /// Creation number: the tie-break of the fold order, kept by `set`.
    serial: u64,
    /// Vertex texture row of a polygon zone.
    row: Option<u32>,
}

/// The ambient zones of one scene.
#[derive(Clone, Debug)]
pub struct AmbientZoneStore {
    zones: Slots<Entry>,
    /// Fold order: ids sorted by (priority, serial).
    order: Vec<AmbientZoneId>,
    /// Vertex texture rows in use.
    row_used: Vec<bool>,
    next_serial: u64,
    polygon_version: u64,
}

impl Default for AmbientZoneStore {
    fn default() -> Self {
        Self {
            zones: Slots::default(),
            order: Vec::new(),
            row_used: Vec::new(),
            next_serial: 1,
            polygon_version: 0,
        }
    }
}

impl AmbientZoneStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live zones.
    pub fn len(&self) -> usize {
        self.zones.len()
    }

    /// `true` if no zone is alive.
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Changes whenever polygon vertices are written (create or set of a polygon zone), for the
    /// texture upload.
    pub fn polygon_version(&self) -> u64 {
        self.polygon_version
    }

    /// `Ok` if `count` more zones fit into the store.
    ///
    /// # Errors
    /// [`ZoneErrorKind::TooMany`] if they would exceed [`MAX_ZONES`].
    pub fn can_create(&self, count: usize) -> Result<(), ZoneError> {
        if count > MAX_ZONES || self.zones.len() + count > MAX_ZONES {
            return Err(ZoneError::new(
                ZoneErrorKind::TooMany,
                format!(
                    "too many ambient zones (at most {MAX_ZONES} alive at once, {} alive, {count} more asked for); destroy some",
                    self.zones.len()
                ),
            ));
        }
        Ok(())
    }

    fn take_row(&mut self) -> u32 {
        if let Some(row) = self.row_used.iter().position(|&used| !used) {
            self.row_used[row] = true;
            return row as u32;
        }
        self.row_used.push(true);
        (self.row_used.len() - 1) as u32
    }

    fn free_row(&mut self, row: u32) {
        if let Some(used) = self.row_used.get_mut(row as usize) {
            *used = false;
        }
    }

    fn key(&self, id: AmbientZoneId) -> (i32, u64) {
        self.zones.get(id.cast()).map_or((i32::MAX, u64::MAX), |e| {
            (e.zone.params().priority, e.serial)
        })
    }

    /// Puts `id` at its place in the fold order (priority, then creation number).
    fn insert_ordered(&mut self, id: AmbientZoneId) {
        let key = self.key(id);
        let pos = self.order.partition_point(|&other| self.key(other) < key);
        self.order.insert(pos, id);
    }

    /// Adds a zone; its id.
    ///
    /// # Errors
    /// [`ZoneErrorKind::TooMany`] when [`MAX_ZONES`] zones are alive.
    pub fn create(&mut self, zone: StoredZone) -> Result<AmbientZoneId, ZoneError> {
        self.can_create(1)?;
        let polygon = zone.params().shape == ZoneShape::Polygon;
        let id: AmbientZoneId = self
            .zones
            .insert(Entry {
                zone,
                serial: self.next_serial,
                row: None,
            })
            .cast();
        self.next_serial += 1;
        if polygon {
            let row = self.take_row();
            if let Some(e) = self.zones.get_mut(id.cast()) {
                e.row = Some(row);
            }
        }
        self.insert_ordered(id);
        if polygon {
            self.polygon_version += 1;
        }
        Ok(id)
    }

    /// Replaces a live zone and keeps its creation number.
    ///
    /// # Errors
    /// The error of [`AmbientZoneStore::describe`] for a handle that is not live.
    pub fn set(&mut self, id: AmbientZoneId, zone: StoredZone) -> Result<(), ZoneError> {
        self.describe(id)?;
        let polygon = zone.params().shape == ZoneShape::Polygon;
        let row = self.zones.get(id.cast()).and_then(|e| e.row);
        let new_row = match (polygon, row) {
            (true, None) => Some(self.take_row()),
            (false, Some(r)) => {
                self.free_row(r);
                None
            }
            (_, r) => r,
        };
        if let Some(e) = self.zones.get_mut(id.cast()) {
            e.zone = zone;
            e.row = new_row;
        }
        // The priority may have changed; the creation number stays.
        self.order.retain(|&o| o != id);
        self.insert_ordered(id);
        if polygon {
            self.polygon_version += 1;
        }
        Ok(())
    }

    /// Destroys a live zone.
    ///
    /// # Errors
    /// The error of [`AmbientZoneStore::describe`] for a handle that is not live.
    pub fn destroy(&mut self, id: AmbientZoneId) -> Result<(), ZoneError> {
        self.describe(id)?;
        if let Some(row) = self.zones.remove(id.cast()).and_then(|e| e.row) {
            self.free_row(row);
        }
        self.order.retain(|&o| o != id);
        Ok(())
    }

    /// `Ok` for a live zone. Otherwise the error says why: a destroyed zone (an older generation
    /// of a slot this store handed out) or a handle this store never issued.
    ///
    /// # Errors
    /// [`ZoneErrorKind::StaleHandle`] or [`ZoneErrorKind::ForeignHandle`].
    pub fn describe(&self, id: AmbientZoneId) -> Result<(), ZoneError> {
        if self.zones.contains(id.cast()) {
            return Ok(());
        }
        let bits = id.to_bits();
        if id.index() < self.zones.index_bound() {
            return Err(ZoneError::new(
                ZoneErrorKind::StaleHandle,
                format!(
                    "zone 0x{bits:016x} was destroyed; create a new zone (a handle of another context can look the same)"
                ),
            ));
        }
        Err(ZoneError::new(
            ZoneErrorKind::ForeignHandle,
            format!(
                "zone 0x{bits:016x} was not created by this context (a handle of another context?)"
            ),
        ))
    }

    /// The zone, or `None` for an unknown or destroyed handle.
    pub fn find(&self, id: AmbientZoneId) -> Option<&StoredZone> {
        self.zones.get(id.cast()).map(|e| &e.zone)
    }

    /// The vertex texture row of a live polygon zone, or `None`.
    pub fn polygon_row(&self, id: AmbientZoneId) -> Option<u32> {
        self.zones.get(id.cast()).and_then(|e| e.row)
    }

    /// One more than the highest row in use; 0 without polygon zones.
    pub fn polygon_row_count(&self) -> u32 {
        self.row_used
            .iter()
            .rposition(|&used| used)
            .map_or(0, |i| i as u32 + 1)
    }

    /// Every live zone in fold order: priority, then creation number.
    pub fn iter_ordered(&self) -> impl Iterator<Item = (AmbientZoneId, &StoredZone)> + '_ {
        self.order
            .iter()
            .filter_map(move |&id| self.zones.get(id.cast()).map(|e| (id, &e.zone)))
    }

    /// `(row, zone)` for every live polygon zone.
    pub fn polygons(&self) -> impl Iterator<Item = (u32, &StoredZone)> + '_ {
        self.zones
            .iter()
            .filter_map(|(_, e)| e.row.map(|r| (r, &e.zone)))
    }
}
