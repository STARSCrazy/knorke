// Origin: chiaro @ 9bc5b9b, src/rendering/ProfileAtlas.hpp, src/rendering/ProfileAtlas.cpp,
// src/rendering/MaskAtlas.hpp, src/rendering/MaskAtlas.cpp, src/rendering/LightAtlases.hpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The atlas textures on the GPU: an owned texture, an upload buffer and the version of the
//! store they were filled from. Ownership follows ADR 0002: the textures live on the host's
//! device, the upload is recorded into the host's command buffer, nothing is submitted or read
//! back.

use core::marker::PhantomData;

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB, SDL_GPU_TEXTUREFORMAT_R32_FLOAT,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags, SDL_GPUTextureFormat,
};
use knorke_gpu::{CommandBuffer, Gpu, Texture, TextureDesc, TextureRef, UploadBuffer};
use knorke_types::Diagnostic;

use super::{
    fill_mask_atlas, fill_profile_atlas, mask_atlas_bytes, mask_atlas_rows, profile_atlas_bytes,
    profile_atlas_rows,
};
use crate::light_culling::LightList;
use crate::mask::{MASK_BYTES, MASK_SIZE};
use crate::mask_store::MaskStore;
use crate::profile::{PROFILE_POLAR, PROFILE_VALUES};
use crate::profile_store::ProfileStore;

/// Version before the first fill (no store has it).
const NO_VERSION: u64 = u64::MAX;

/// A store whose contents become one atlas texture (rows of `WIDTH` texels).
pub trait AtlasSource {
    /// Texel format of the texture.
    const FORMAT: SDL_GPUTextureFormat;
    /// Width of the texture in texels.
    const WIDTH: u32;
    /// Bytes of one block, the unit the upload buffer grows by.
    const BLOCK_BYTES: u32;
    /// Debug name of the upload buffer.
    const NAME: &'static str;

    /// Rows the texture needs for the current contents; 0 without any.
    fn rows(&self) -> u32;
    /// Changes whenever the contents change.
    fn version(&self) -> u64;
    /// Bytes of an atlas of `rows` rows.
    fn bytes(rows: u32) -> u64;
    /// Writes the atlas into `out` (the first [`bytes`](Self::bytes) bytes).
    fn fill(&self, rows: u32, out: &mut [u8]);
}

impl AtlasSource for ProfileStore {
    const FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32_FLOAT;
    const WIDTH: u32 = PROFILE_POLAR as u32;
    const BLOCK_BYTES: u32 = (PROFILE_VALUES * 4) as u32;
    const NAME: &'static str = "profile atlas";

    fn rows(&self) -> u32 {
        profile_atlas_rows(self.block_count())
    }
    fn version(&self) -> u64 {
        ProfileStore::version(self)
    }
    fn bytes(rows: u32) -> u64 {
        profile_atlas_bytes(rows)
    }
    fn fill(&self, rows: u32, out: &mut [u8]) {
        fill_profile_atlas(self, rows, out);
    }
}

impl AtlasSource for MaskStore {
    const FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB;
    const WIDTH: u32 = MASK_SIZE as u32;
    const BLOCK_BYTES: u32 = MASK_BYTES as u32;
    const NAME: &'static str = "mask atlas";

    fn rows(&self) -> u32 {
        mask_atlas_rows(self.block_count())
    }
    fn version(&self) -> u64 {
        MaskStore::version(self)
    }
    fn bytes(rows: u32) -> u64 {
        mask_atlas_bytes(rows)
    }
    fn fill(&self, rows: u32, out: &mut [u8]) {
        fill_mask_atlas(self, rows, out);
    }
}

/// The contents of a store in one atlas texture: texture, upload buffer, size and the store
/// version it holds. Without contents there is no texture.
#[derive(Debug)]
pub struct AtlasTexture<'d, S: AtlasSource> {
    gpu: Gpu<'d>,
    texture: Option<Texture<'d>>,
    upload: UploadBuffer<'d>,
    rows: u32,
    version: u64,
    pending: bool,
    source: PhantomData<fn(&S)>,
}

/// The profiles of a [`ProfileStore`] in one `R32_FLOAT` texture, `PROFILE_POLAR` wide and
/// [`profile_atlas_rows`] rows high (Chiaro `ProfileAtlas`).
pub type ProfileAtlasTexture<'d> = AtlasTexture<'d, ProfileStore>;

/// The masks of a [`MaskStore`] in one `R8G8B8A8_UNORM_SRGB` texture, `MASK_SIZE` wide and
/// [`mask_atlas_rows`] rows high (Chiaro `MaskAtlas`).
pub type MaskAtlasTexture<'d> = AtlasTexture<'d, MaskStore>;

impl<'d, S: AtlasSource> AtlasTexture<'d, S> {
    /// An atlas without a texture.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            gpu,
            texture: None,
            upload: UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), S::BLOCK_BYTES, S::NAME),
            rows: 0,
            version: NO_VERSION,
            pending: false,
            source: PhantomData,
        }
    }

    /// Matches the texture to the store: no contents release it, a new size creates it, a new
    /// version fills the upload buffer. Returns the number of textures created.
    ///
    /// # Errors
    /// The texture or the upload buffer could not be created or mapped; the atlas keeps its
    /// old contents then.
    pub fn prepare(&mut self, store: &S) -> Result<u32, Diagnostic> {
        let rows = store.rows();
        let version = store.version();
        if rows == 0 {
            self.release();
            self.version = version;
            return Ok(0);
        }
        let mut created = 0;
        if self.texture.is_none() || rows != self.rows {
            let desc = TextureDesc::new(S::WIDTH, rows, S::FORMAT, SDL_GPU_TEXTUREUSAGE_SAMPLER);
            self.texture = Some(Texture::new(self.gpu, desc)?);
            self.rows = rows;
            self.version = NO_VERSION;
            created = 1;
        }
        if self.version == version {
            return Ok(created);
        }
        let bytes = S::bytes(rows);
        self.upload.reserve(bytes)?;
        let mut mapped = self.upload.map()?;
        store.fill(rows, &mut mapped.bytes()[..bytes as usize]);
        drop(mapped);
        self.version = version;
        self.pending = true;
        Ok(created)
    }

    /// Drops the texture (the next [`prepare`](Self::prepare) creates it again).
    pub fn release(&mut self) {
        self.texture = None;
        self.upload.reset();
        self.rows = 0;
        self.pending = false;
        self.version = NO_VERSION;
    }

    /// Records the pending upload, if any, as a copy pass.
    ///
    /// # Errors
    /// A copy pass could not be opened.
    pub fn record(&mut self, cmd: &mut CommandBuffer<'_>) -> Result<(), Diagnostic> {
        let (true, Some(texture)) = (self.pending, self.texture.as_ref()) else {
            return Ok(());
        };
        cmd.copy_pass()?
            .upload_to_texture(&self.upload, texture.texture_ref(), true)?;
        self.pending = false;
        Ok(())
    }

    /// An upload is waiting for [`record`](Self::record).
    pub fn upload_pending(&self) -> bool {
        self.pending
    }

    /// The texture; `None` without contents.
    pub fn texture(&self) -> Option<TextureRef<'_>> {
        self.texture.as_ref().map(Texture::texture_ref)
    }

    /// Rows of the texture (0 without one).
    pub fn height(&self) -> u32 {
        self.rows
    }
}

/// Both atlases of a scene (Chiaro `LightAtlases`).
#[derive(Debug)]
pub struct LightAtlases<'d> {
    /// The profile atlas.
    pub profiles: ProfileAtlasTexture<'d>,
    /// The mask atlas.
    pub masks: MaskAtlasTexture<'d>,
}

impl<'d> LightAtlases<'d> {
    /// Atlases without textures.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            profiles: ProfileAtlasTexture::new(gpu),
            masks: MaskAtlasTexture::new(gpu),
        }
    }

    /// Brings the atlases the frame needs up to date: the profile atlas for a list with
    /// profiled lights, the mask atlas for one with masked lights. A needed atlas without its
    /// store is released, so the light pass reports it as missing. Returns the number of
    /// textures created.
    ///
    /// # Errors
    /// A texture or upload buffer could not be created or mapped.
    pub fn prepare(
        &mut self,
        list: &LightList,
        profiles: Option<&ProfileStore>,
        masks: Option<&MaskStore>,
    ) -> Result<u32, Diagnostic> {
        let mut created = 0;
        if list.profiled_lights > 0 {
            match profiles {
                Some(p) => created += self.profiles.prepare(p)?,
                None => self.profiles.release(),
            }
        }
        if list.masked_lights > 0 {
            match masks {
                Some(m) => created += self.masks.prepare(m)?,
                None => self.masks.release(),
            }
        }
        Ok(created)
    }

    /// Records the pending uploads; opens a copy pass only for an atlas that has one.
    ///
    /// # Errors
    /// A copy pass could not be opened.
    pub fn record(&mut self, cmd: &mut CommandBuffer<'_>) -> Result<(), Diagnostic> {
        self.profiles.record(cmd)?;
        self.masks.record(cmd)
    }
}
