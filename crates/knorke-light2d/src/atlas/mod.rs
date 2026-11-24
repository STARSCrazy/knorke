// Origin: chiaro @ 9bc5b9b, src/rendering/ProfileAtlas.hpp, src/rendering/ProfileAtlas.cpp,
// src/rendering/MaskAtlas.hpp, src/rendering/MaskAtlas.cpp, src/rendering/LightAtlases.hpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The light atlases (P26, Chiaro `ProfileAtlas`, `MaskAtlas`, `LightAtlases`): the profiles of a
//! [`ProfileStore`] in one `R32_FLOAT` texture and the masks of a [`MaskStore`] in one
//! `R8G8B8A8_UNORM_SRGB` texture, rebuilt when the store's version changes.
//!
//! | Module | Content |
//! |---|---|
//! | this one | the texture sizes and the byte layout of both atlases, without a GPU |
//! | `gpu` (feature `gpu`) | [`ProfileAtlasTexture`], [`MaskAtlasTexture`], [`LightAtlases`] |
//!
//! Profile block `b` starts at row `b * PROFILE_AZIMUTH` of a texture [`PROFILE_POLAR`] wide
//! (the table's row-major order), mask block `b` at row `b * MASK_SIZE` of a texture
//! [`MASK_SIZE`] wide. Texels no block covers are 0.
//!
//! # Redesign against the C++ version
//!
//! - The atlases are not part of the lighting stage; the host owns a
//!   [`LightAtlases`], prepares it for the frame's light list, records its upload into the
//!   command buffer and hands the two textures to the light pass through
//!   [`LightingFrame::profile_atlas`](crate::lighting::LightingFrame) and `mask_atlas`.
//! - The profile atlas is created for a frame with profiled lights, the mask atlas for a frame
//!   with masked lights (Chiaro: whenever lights are drawn and a store exists, for masks
//!   whenever masked lights are drawn). A frame without them leaves the atlases alone; the next
//!   frame that needs them brings them up to date by the store's version.
//! - Filling the atlas bytes is a plain function over a slice ([`fill_profile_atlas`],
//!   [`fill_mask_atlas`]), so the steady state allocates nothing on the CPU either.

use crate::mask::{MASK_BYTES, MASK_SIZE, mask_atlas_blocks};
use crate::mask_store::MaskStore;
use crate::profile::{PROFILE_AZIMUTH, PROFILE_POLAR, PROFILE_VALUES, profile_atlas_blocks};
use crate::profile_store::ProfileStore;
use knorke_math::uniform::write_f32s;

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{AtlasSource, AtlasTexture, LightAtlases, MaskAtlasTexture, ProfileAtlasTexture};

/// Rows of the profile atlas for `block_count` profile blocks; 0 for none.
pub fn profile_atlas_rows(block_count: u32) -> u32 {
    profile_atlas_blocks(block_count) * PROFILE_AZIMUTH as u32
}

/// Rows of the mask atlas for `block_count` mask blocks; 0 for none.
pub fn mask_atlas_rows(block_count: u32) -> u32 {
    mask_atlas_blocks(block_count) * MASK_SIZE as u32
}

/// Bytes of a profile atlas of `rows` rows.
pub fn profile_atlas_bytes(rows: u32) -> u64 {
    u64::from(rows) * PROFILE_POLAR as u64 * 4
}

/// Bytes of a mask atlas of `rows` rows.
pub fn mask_atlas_bytes(rows: u32) -> u64 {
    u64::from(rows) * MASK_SIZE as u64 * 4
}

/// Writes the profile atlas into `out` (the first [`profile_atlas_bytes`]`(rows)` bytes): zero,
/// then every profile table little-endian at its block. A profile whose block lies outside `out`
/// is skipped.
pub fn fill_profile_atlas(profiles: &ProfileStore, rows: u32, out: &mut [u8]) {
    let total = (profile_atlas_bytes(rows) as usize).min(out.len());
    out[..total].fill(0);
    let block_bytes = PROFILE_VALUES * 4;
    for (block, values) in profiles.iter() {
        let start = block as usize * block_bytes;
        let Some(dst) = out[..total].get_mut(start..start + block_bytes) else {
            continue;
        };
        write_f32s(dst, values);
    }
}

/// Writes the mask atlas into `out` (the first [`mask_atlas_bytes`]`(rows)` bytes): zero, then
/// every mask's texels at its block. A mask whose block lies outside `out` is skipped.
pub fn fill_mask_atlas(masks: &MaskStore, rows: u32, out: &mut [u8]) {
    let total = (mask_atlas_bytes(rows) as usize).min(out.len());
    out[..total].fill(0);
    for (block, texels) in masks.iter() {
        let start = block as usize * MASK_BYTES;
        if let Some(dst) = out[..total].get_mut(start..start + MASK_BYTES) {
            dst.copy_from_slice(texels);
        }
    }
}
