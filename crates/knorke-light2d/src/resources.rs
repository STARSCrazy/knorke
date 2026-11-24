// Origin: new for the Rust port (P8, P10). Replaces the pointers `LightStore::SetAnimations`,
// `SetMasks` and `SetProfiles` of chiaro @ 9bc5b9b, src/lighting/LightStore.hpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The stores a light can refer to, as one [`LightResources`] answer.

use crate::animation::{AnimationId, AnimationStore};
use crate::light::{LightResources, ProfileId};
use crate::mask_store::{MaskId, MaskStore};
use crate::profile_store::ProfileStore;

/// The animation and mask stores (and optionally the profile store) of a scene, handed to every
/// light mutation that can name one.
///
/// Without [`with_profiles`](Self::with_profiles) no profile exists: [`has_profile`] answers
/// `false`, so a light that names a profile is refused.
///
/// [`has_profile`]: LightResources::has_profile
#[derive(Clone, Copy, Debug)]
pub struct StoreResources<'a> {
    /// The animations lights may name.
    pub animations: &'a AnimationStore,
    /// The masks lights may name.
    pub masks: &'a MaskStore,
    /// The profiles lights may name; `None`: there are none.
    pub profiles: Option<&'a ProfileStore>,
}

impl<'a> StoreResources<'a> {
    /// The animation and mask stores of a scene, without profiles.
    pub fn new(animations: &'a AnimationStore, masks: &'a MaskStore) -> Self {
        Self {
            animations,
            masks,
            profiles: None,
        }
    }

    /// The same resources with the profile store of the scene.
    #[must_use]
    pub fn with_profiles(mut self, profiles: &'a ProfileStore) -> Self {
        self.profiles = Some(profiles);
        self
    }
}

impl LightResources for StoreResources<'_> {
    fn has_profile(&self, id: ProfileId) -> bool {
        self.profiles.is_some_and(|p| p.contains(id))
    }

    fn has_animation(&self, id: AnimationId) -> bool {
        self.animations.contains(id)
    }

    fn has_mask(&self, id: MaskId) -> bool {
        self.masks.contains(id)
    }
}
