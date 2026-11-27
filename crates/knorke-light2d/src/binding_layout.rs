// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/gpu/LightProgram.hpp and cmake/ChiaroLightPrograms.cmake.
//
// Original notice (zlib):
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered: rewritten in Rust for the Knorke Engine. Of Chiaro's light program family only the
// binding vocabulary is left (the light programs are replaced by the row programs, ADR 0019).

//! The binding vocabulary of the medium and composite programs (Chiaro E153): the resources a
//! program can read by name, and the slot of each in one program's layout ([`LightLayout`],
//! built by [`LayoutBuilder`] in the order its shaders declare them), so a draw binds exactly
//! what its program reads.

/// Fragment samplers (texture + sampler pairs). The medium and composite entries belong to
/// the layouts of those stages (Chiaro `MediumLayoutOf`, `CompositeLayoutOf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightSampler {
    /// `SurfaceTex`: surface buffer.
    Surface,
    /// `Mask`: occlusion mask of the caster class.
    OcclusionMask,
    /// `Field`: distance field of the caster class.
    DistanceField,
    /// `Heights`: obstacle heights of the caster class.
    OcclusionHeight,
    /// `MaterialTex`: material buffer (exact mode).
    Material,
    /// `ProfileAtlas` (point and spot lights).
    Profile,
    /// `MaskAtlas` (masked lights).
    Mask,
    /// `MediumMap`: medium world map.
    MediumMap,
    /// `PatternIds`: pattern obstacle id texture.
    PatternIds,
    /// `PatternAtlas`.
    PatternAtlas,
    /// `MediumTex` (medium programs).
    MediumTex,
    /// `ScatterTex` (medium programs).
    ScatterTex,
    /// `PureAmbientTex` (medium programs).
    PureAmbient,
    /// `PolarAtlas` (medium programs).
    PolarAtlas,
    /// `SunHeights` (medium programs).
    SunHeights,
    /// `Scene` (composite).
    Scene,
    /// `LightBuffer` (composite).
    LightBuffer,
    /// `GlowTex` (composite).
    Glow,
    /// `DetailTex` (composite).
    Detail,
    /// `GuideTex` (composite).
    ScatterGuides,
}

impl LightSampler {
    /// Number of sampler kinds.
    pub const COUNT: usize = 20;
    /// All kinds in order.
    pub const ALL: [LightSampler; Self::COUNT] = [
        Self::Surface,
        Self::OcclusionMask,
        Self::DistanceField,
        Self::OcclusionHeight,
        Self::Material,
        Self::Profile,
        Self::Mask,
        Self::MediumMap,
        Self::PatternIds,
        Self::PatternAtlas,
        Self::MediumTex,
        Self::ScatterTex,
        Self::PureAmbient,
        Self::PolarAtlas,
        Self::SunHeights,
        Self::Scene,
        Self::LightBuffer,
        Self::Glow,
        Self::Detail,
        Self::ScatterGuides,
    ];

    /// HLSL name of the resource.
    pub const fn hlsl_name(self) -> &'static str {
        match self {
            Self::Surface => "SurfaceTex",
            Self::OcclusionMask => "Mask",
            Self::DistanceField => "Field",
            Self::OcclusionHeight => "Heights",
            Self::Material => "MaterialTex",
            Self::Profile => "ProfileAtlas",
            Self::Mask => "MaskAtlas",
            Self::MediumMap => "MediumMap",
            Self::PatternIds => "PatternIds",
            Self::PatternAtlas => "PatternAtlas",
            Self::MediumTex => "MediumTex",
            Self::ScatterTex => "ScatterTex",
            Self::PureAmbient => "PureAmbientTex",
            Self::PolarAtlas => "PolarAtlas",
            Self::SunHeights => "SunHeights",
            Self::Scene => "Scene",
            Self::LightBuffer => "LightBuffer",
            Self::Glow => "GlowTex",
            Self::Detail => "DetailTex",
            Self::ScatterGuides => "GuideTex",
        }
    }
}

/// Fragment uniform blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightUniform {
    /// `SurfaceParams`.
    Surface,
    /// `ShadowParams` (distance field programs).
    Shadow,
    /// `DirectionalParams`.
    Directional,
    /// `MediumMapParams`.
    MediumMap,
    /// `PatternParams`.
    Pattern,
    /// `MediumParams` (medium programs, composite).
    MediumParams,
    /// `ZoneParams` (medium programs).
    Zone,
    /// `PolarQueryParams` (medium programs).
    PolarQuery,
    /// `SunParams` (medium programs).
    Sun,
    /// `PolarRowParams` (medium programs).
    PolarRow,
    /// `SunHeightParams` (medium programs).
    SunHeight,
    /// `CompositeParams` (composite).
    Composite,
}

impl LightUniform {
    /// Number of uniform kinds.
    pub const COUNT: usize = 12;
    /// All kinds in order.
    pub const ALL: [LightUniform; Self::COUNT] = [
        Self::Surface,
        Self::Shadow,
        Self::Directional,
        Self::MediumMap,
        Self::Pattern,
        Self::MediumParams,
        Self::Zone,
        Self::PolarQuery,
        Self::Sun,
        Self::PolarRow,
        Self::SunHeight,
        Self::Composite,
    ];

    /// HLSL name of the block.
    pub const fn hlsl_name(self) -> &'static str {
        match self {
            Self::Surface => "SurfaceParams",
            Self::Shadow => "ShadowParams",
            Self::Directional => "DirectionalParams",
            Self::MediumMap => "MediumMapParams",
            Self::Pattern => "PatternParams",
            Self::MediumParams => "MediumParams",
            Self::Zone => "ZoneParams",
            Self::PolarQuery => "PolarQueryParams",
            Self::Sun => "SunParams",
            Self::PolarRow => "PolarRowParams",
            Self::SunHeight => "SunHeightParams",
            Self::Composite => "CompositeParams",
        }
    }
}

/// Storage buffers: `Lights` and `Masks` belong to the vertex shader, the pattern buffers to
/// the fragment shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightStorage {
    /// Vertex: `StructuredBuffer<GpuLight>`.
    Lights,
    /// Vertex: `StructuredBuffer<GpuMask>`.
    Masks,
    /// Fragment.
    PatternObstacles,
    /// Fragment.
    Patterns,
    /// Fragment.
    PatternData,
}

impl LightStorage {
    /// Number of storage kinds.
    pub const COUNT: usize = 5;
    /// All kinds in order.
    pub const ALL: [LightStorage; Self::COUNT] = [
        Self::Lights,
        Self::Masks,
        Self::PatternObstacles,
        Self::Patterns,
        Self::PatternData,
    ];

    /// Whether the vertex shader reads this buffer.
    pub const fn is_vertex(self) -> bool {
        matches!(self, Self::Lights | Self::Masks)
    }

    /// HLSL name of the buffer.
    pub const fn hlsl_name(self) -> &'static str {
        match self {
            Self::Lights => "Lights",
            Self::Masks => "Masks",
            Self::PatternObstacles => "PatternObstacles",
            Self::Patterns => "Patterns",
            Self::PatternData => "PatternData",
        }
    }
}

/// Slot of each resource of one program, `None` where the program does not have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LightLayout {
    samplers: [Option<u8>; LightSampler::COUNT],
    uniforms: [Option<u8>; LightUniform::COUNT],
    storage: [Option<u8>; LightStorage::COUNT],
    /// Fragment samplers.
    pub sampler_count: u8,
    /// Fragment uniform blocks.
    pub uniform_count: u8,
    /// Vertex storage buffers.
    pub vertex_storage_count: u8,
    /// Fragment storage buffers.
    pub fragment_storage_count: u8,
    /// Vertex uniform blocks (pass params of the light vertex shaders; 0 for fullscreen).
    pub vertex_uniform_count: u8,
}

impl LightLayout {
    /// Slot of sampler `s`.
    pub const fn sampler(&self, s: LightSampler) -> Option<u8> {
        self.samplers[s as usize]
    }
    /// Slot of uniform block `u`.
    pub const fn uniform(&self, u: LightUniform) -> Option<u8> {
        self.uniforms[u as usize]
    }
    /// Slot of storage buffer `b` within the stage it belongs to.
    pub const fn storage(&self, b: LightStorage) -> Option<u8> {
        self.storage[b as usize]
    }
}

/// Hands out the slots of a layout in declaration order (shared with the medium and
/// composite layouts of later stages).
#[derive(Clone, Copy, Debug)]
pub struct LayoutBuilder {
    layout: LightLayout,
}

impl Default for LayoutBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutBuilder {
    /// An empty layout.
    pub const fn new() -> Self {
        Self {
            layout: LightLayout {
                samplers: [None; LightSampler::COUNT],
                uniforms: [None; LightUniform::COUNT],
                storage: [None; LightStorage::COUNT],
                sampler_count: 0,
                uniform_count: 0,
                vertex_storage_count: 0,
                fragment_storage_count: 0,
                vertex_uniform_count: 0,
            },
        }
    }
    /// Appends sampler `s`.
    pub const fn sampler(&mut self, s: LightSampler) {
        self.layout.samplers[s as usize] = Some(self.layout.sampler_count);
        self.layout.sampler_count += 1;
    }
    /// Appends uniform block `u`.
    pub const fn uniform(&mut self, u: LightUniform) {
        self.layout.uniforms[u as usize] = Some(self.layout.uniform_count);
        self.layout.uniform_count += 1;
    }
    /// Appends storage buffer `b` to its stage.
    pub const fn storage(&mut self, b: LightStorage) {
        if b.is_vertex() {
            self.layout.storage[b as usize] = Some(self.layout.vertex_storage_count);
            self.layout.vertex_storage_count += 1;
        } else {
            self.layout.storage[b as usize] = Some(self.layout.fragment_storage_count);
            self.layout.fragment_storage_count += 1;
        }
    }
    /// Sets the number of vertex uniform blocks.
    pub const fn vertex_uniforms(&mut self, count: u8) {
        self.layout.vertex_uniform_count = count;
    }
    /// The finished layout.
    pub const fn finish(self) -> LightLayout {
        self.layout
    }
}
