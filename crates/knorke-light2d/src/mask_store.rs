// Origin: chiaro @ 9bc5b9b, src/lighting/MaskStore.hpp and MaskStore.cpp, with core/SlotMap.hpp
// and core/Ids.hpp replaced by knorke_types::Slots.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The light masks of one scene (P10).
//!
//! Each live mask owns an atlas block in `[0, MAX_MASKS)`, the lowest free one at
//! [`create`](MaskStore::create), so the atlas needs `mask_atlas_blocks(block_count())` blocks of
//! `MASK_SIZE` rows. The block is independent of the handle's slot index, so slot reuse never
//! grows the atlas.
//!
//! Redesign against the C++ version:
//!
//! - [`MaskId`] is a generation-checked handle of [`Slots`]. A stale handle never reaches a mask
//!   that reused its slot. The C++ store told "was destroyed" from "never handed out" by the
//!   slot map's handle state; [`Slots`] does not keep that difference, so both are
//!   [`MaskError::InvalidHandle`].
//! - The store does not know the lights; [`destroy`](MaskStore::destroy) takes the
//!   [`LightStore`] to refuse a mask that lights still use, as the C++ context did.
//! - Masks are loaded assets, but lights refer to them by handle, so a restored scene needs the
//!   same handles. The store therefore implements [`Snapshot`] (`knorke.light2d.masks`) with the
//!   slots and blocks, followed by the texels of every mask (256 KiB each, at most 16 MiB). A
//!   host that restores the assets itself can leave the section out. The texels live in one
//!   buffer per atlas block, so a restore into a store that already holds the same blocks
//!   allocates nothing. [`restore`](Snapshot::restore) re-checks the blocks
//!   (`SnapshotError::Invalid`) and bumps [`version`](MaskStore::version) so the atlas is
//!   uploaded again.

use crate::LightStore;
use crate::mask::{MASK_BYTES, MaskError};
use knorke_types::{Handle, Reader, Slots, Snapshot, SnapshotError, SnapshotValue, Writer};

/// Most masks alive at once; keeps the atlas within `64 * 256 = 16384` rows.
pub const MAX_MASKS: u32 = 64;

/// A stored mask: the atlas block that owns its texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mask {
    block: u32,
}

impl Mask {
    /// The atlas block of the mask.
    pub fn block(&self) -> u32 {
        self.block
    }
}

/// Handle of a light mask in a [`MaskStore`].
pub type MaskId = Handle<Mask>;

/// The light masks of one scene. Every mutation validates first and leaves the state unchanged
/// on error.
#[derive(Clone, Debug, Default)]
pub struct MaskStore {
    masks: Slots<Mask>,
    /// The texels of the live mask that owns the block; empty for a free block.
    texels: Vec<Vec<u8>>,
    /// Bit `b` is set while a live mask owns block `b`.
    used: u64,
    version: u64,
}

impl MaskStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of masks.
    pub fn len(&self) -> usize {
        self.masks.len()
    }

    /// `true` if there are no masks.
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// Adds a mask; takes `MASK_BYTES` texels (see
    /// [`MaskShape::build`](crate::mask::MaskShape::build) and
    /// [`build_pixel_mask`](crate::mask::build_pixel_mask)).
    ///
    /// # Errors
    /// Wrong texel size, or [`MAX_MASKS`] masks are alive.
    pub fn create(&mut self, texels: Vec<u8>) -> Result<MaskId, MaskError> {
        if texels.len() != MASK_BYTES {
            return Err(MaskError::TexelSize { got: texels.len() });
        }
        if self.masks.len() >= MAX_MASKS as usize {
            return Err(MaskError::TooMany { max: MAX_MASKS });
        }
        let block = (!self.used).trailing_zeros(); // < 64 while fewer than 64 masks are alive
        let id = self.masks.insert(Mask { block });
        self.used |= 1 << block;
        self.version += 1;
        if self.texels.len() < MAX_MASKS as usize {
            self.texels.resize_with(MAX_MASKS as usize, Vec::new);
        }
        self.texels[block as usize] = texels;
        Ok(id)
    }

    /// Removes a mask. Refused while a light of `lights` still uses it.
    ///
    /// # Errors
    /// Stale handle, or [`MaskError::InUse`].
    pub fn destroy(&mut self, id: MaskId, lights: &LightStore) -> Result<(), MaskError> {
        if !self.masks.contains(id) {
            return Err(MaskError::InvalidHandle(id.to_bits()));
        }
        let users = lights.mask_users(id);
        if users > 0 {
            return Err(MaskError::InUse { users });
        }
        if let Some(mask) = self.masks.remove(id) {
            self.used &= !(1 << mask.block);
            self.texels[mask.block as usize] = Vec::new();
            self.version += 1;
        }
        Ok(())
    }

    /// `true` if `id` names a live mask.
    pub fn contains(&self, id: MaskId) -> bool {
        self.masks.contains(id)
    }

    /// The texels, or `None` for a stale handle.
    pub fn find(&self, id: MaskId) -> Option<&[u8]> {
        self.masks
            .get(id)
            .map(|m| self.texels[m.block as usize].as_slice())
    }

    /// The atlas block of a live mask.
    pub fn block_of(&self, id: MaskId) -> Option<u32> {
        self.masks.get(id).map(Mask::block)
    }

    /// Every live mask as `(block, texels)`, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &[u8])> {
        self.masks
            .iter()
            .map(|(_, m)| (m.block, self.texels[m.block as usize].as_slice()))
    }

    /// One more than the highest block of a live mask; 0 without masks.
    pub fn block_count(&self) -> u32 {
        u64::BITS - self.used.leading_zeros()
    }

    /// Changes with every create, destroy and restore, for the atlas upload.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn restore_state(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.used = 0;
        self.masks.get_into(r)?;
        let mut used = 0u64;
        let mut valid = true;
        for (_, m) in self.masks.iter() {
            valid &= m.block < MAX_MASKS && used & (1u64 << (m.block & 63)) == 0;
            used |= 1u64 << (m.block & 63);
        }
        if !valid {
            return Err(SnapshotError::Invalid("mask atlas block"));
        }
        if self.texels.len() < MAX_MASKS as usize {
            self.texels.resize_with(MAX_MASKS as usize, Vec::new);
        }
        for (_, m) in self.masks.iter() {
            let src = r.take(MASK_BYTES)?;
            let dst = &mut self.texels[m.block as usize];
            dst.clear();
            dst.extend_from_slice(src);
        }
        for (block, dst) in self.texels.iter_mut().enumerate() {
            if used & (1 << block) == 0 {
                *dst = Vec::new();
            }
        }
        self.used = used;
        Ok(())
    }
}

// ---- snapshots (ADR 0010) ----

impl SnapshotValue for Mask {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(self.block as u8));
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            block: u32::from(r.get::<u8>()?),
        })
    }
}

impl Snapshot for MaskStore {
    const ID: &'static str = "knorke.light2d.masks";
    const VERSION: u16 = 1;

    /// The slots (with the block of every mask), then the texels of every mask in slot order.
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.masks);
        for (_, m) in self.masks.iter() {
            w.put_bytes(&self.texels[m.block as usize]);
        }
    }

    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        crate::check_version::<Self>(version)?;
        self.version += 1;
        let result = self.restore_state(r);
        if result.is_err() {
            *self = Self {
                version: self.version,
                ..Self::default()
            };
        }
        result
    }
}
