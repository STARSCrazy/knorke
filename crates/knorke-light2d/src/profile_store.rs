// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/ProfileStore.hpp, src/lighting/ProfileStore.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The light profiles of one scene (P9): a handle-based store of normalized tables.
//!
//! Redesign against the C++ version: `SlotMap` is [`Slots`], so a profile's slot index is its
//! atlas block ([`ProfileStore::block_of`]) and a stale handle never reaches a profile that
//! reused its slot. The handle type is [`ProfileId`] of the light module, i.e. the same
//! `Handle<ProfileTag>` that [`LightResources`](crate::light::LightResources) asks for, so a
//! host can answer "does this profile exist?" with [`ProfileStore::contains`] without a
//! conversion. The atlas upload itself is a GPU stage and not part of this file.

use crate::light::{ProfileId, ProfileTag};
use crate::profile::{PROFILE_VALUES, ProfileError};
use knorke_types::Slots;

/// Most profiles alive at once; keeps the atlas within 128 * 72 = 9216 rows.
pub const MAX_PROFILES: usize = 128;

/// The light profiles of one scene.
#[derive(Clone, Debug, Default)]
pub struct ProfileStore {
    profiles: Slots<Vec<f32>>,
    version: u64,
}

impl ProfileStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a normalized table of [`PROFILE_VALUES`] entries.
    ///
    /// # Errors
    /// [`ProfileError::WrongSize`] for another length, [`ProfileError::TooManyProfiles`] when
    /// [`MAX_PROFILES`] are alive already. A failed call changes nothing.
    pub fn create(&mut self, values: Vec<f32>) -> Result<ProfileId, ProfileError> {
        if values.len() != PROFILE_VALUES {
            return Err(ProfileError::WrongSize { got: values.len() });
        }
        if self.profiles.len() >= MAX_PROFILES {
            return Err(ProfileError::TooManyProfiles { max: MAX_PROFILES });
        }
        self.version += 1;
        Ok(self.profiles.insert(values).cast::<ProfileTag>())
    }

    /// Destroys a live profile.
    ///
    /// # Errors
    /// [`ProfileError::InvalidHandle`] for a stale or unknown handle.
    pub fn destroy(&mut self, profile: ProfileId) -> Result<(), ProfileError> {
        match self.profiles.remove(profile.cast()) {
            Some(_) => {
                self.version += 1;
                Ok(())
            }
            None => Err(ProfileError::InvalidHandle {
                bits: profile.to_bits(),
            }),
        }
    }

    /// `true` if the handle names a live profile of this store.
    pub fn contains(&self, profile: ProfileId) -> bool {
        self.profiles.contains(profile.cast())
    }

    /// The table of a live profile, or `None` for a stale or unknown handle.
    pub fn find(&self, profile: ProfileId) -> Option<&[f32]> {
        self.profiles.get(profile.cast()).map(Vec::as_slice)
    }

    /// The atlas block of a live profile (its slot index), or `None`.
    pub fn block_of(&self, profile: ProfileId) -> Option<u32> {
        self.contains(profile).then(|| profile.index())
    }

    /// One more than the highest block of a live profile; 0 without profiles.
    pub fn block_count(&self) -> u32 {
        self.profiles
            .iter()
            .map(|(h, _)| h.index() + 1)
            .max()
            .unwrap_or(0)
    }

    /// Calls `(block, table)` for every live profile.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &[f32])> {
        self.profiles
            .iter()
            .map(|(h, values)| (h.index(), values.as_slice()))
    }

    /// Number of live profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// `true` if no profile is alive.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Changes with every create and destroy, for the atlas upload.
    pub fn version(&self) -> u64 {
        self.version
    }
}
