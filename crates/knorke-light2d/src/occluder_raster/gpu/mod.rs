// Origin: chiaro @ 9bc5b9b, src/rendering/OccluderGeometry.* and TileOccluders.* (the GPU parts),
// src/gpu/PipelineCache.cpp (the program table entries OccluderHeight, TilesHeight) and
// src/rendering/OcclusionCommon.hpp (kMaskFormat, kHeightFormat).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU stages of the obstacle raster: buffers, textures, uploads and draws. Ownership
//! follows ADR 0002: the host lends the copy and render passes; nothing here submits, presents
//! or reads back.

mod geometry;
mod tiles;

use knorke_gpu::sys::gpu::{
    SDL_GPU_PRIMITIVETYPE_TRIANGLELIST, SDL_GPU_TEXTUREFORMAT_R8G8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPUTextureFormat,
};
use knorke_gpu::{Blend, ProgramDesc, ProgramIndex, SecondTarget};
use knorke_types::DiagnosticCode;

/// Recording needs something that prepare was meant to create (prepare was not called, failed,
/// or the frame changed since).
pub const NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.occluder_raster.not_prepared");
/// The cell uploads pending in one frame are larger than a GPU transfer buffer can be.
pub const TILE_UPLOAD_LIMIT: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.occluder_raster.tile_upload_limit");

pub use geometry::OccluderGeometry;
pub use tiles::TileOccluders;

/// Format of the occlusion mask: R the opacity (0 free, 1 opaque), G 1 for a texel of a
/// footprint obstacle.
pub const MASK_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8_UNORM;
/// Format of the height texture drawn next to the mask: `(-base, top)` of
/// every obstacle (RG) and of the finite ones (BA), cleared to `-INFINITE_TOP`.
pub const HEIGHT_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

const fn program(vertex: &'static str, fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        vertex,
        fragment,
        blend: Blend::Max,
        second: Some(SecondTarget {
            format: HEIGHT_FORMAT,
            blend: Blend::Max,
        }),
        more: &[],
        primitive: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    }
}

/// The programs of the raster, to be placed in the host's
/// [`PipelineCache`](knorke_gpu::PipelineCache) table at [`OccluderPrograms::at`]. Both draw
/// into the mask and the height texture (second color target) and blend with MAX, so
/// overlapping obstacles keep the highest value.
pub const OCCLUDER_PROGRAMS: [ProgramDesc; 2] = [
    program("occluder_height.vert", "solid_height.frag"),
    program("tile.vert", "tile_height.frag"),
];

/// Where [`OCCLUDER_PROGRAMS`] sit in a pipeline table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccluderPrograms {
    /// Triangles into mask and height texture.
    pub occluder: ProgramIndex,
    /// Tile grids into mask and height texture.
    pub tiles: ProgramIndex,
}

impl OccluderPrograms {
    /// The two programs at `base..base + 2`, in the order of [`OCCLUDER_PROGRAMS`].
    pub const fn at(base: ProgramIndex) -> Self {
        Self {
            occluder: base,
            tiles: base + 1,
        }
    }
}

impl Default for OccluderPrograms {
    /// The table that holds only [`OCCLUDER_PROGRAMS`].
    fn default() -> Self {
        Self::at(0)
    }
}
