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
// Altered: rewritten in Rust for the Knorke Engine. The CMake rules became const fns that
// generate `LIGHT_PROGRAMS` at compile time; `light_program_spec` adds the HLSL source and
// defines. Feature bits are a typed set (`LightFeatures`).

//! The light program family (Chiaro E152, E153): which light and shadow fragment programs
//! exist, their shaders, their second color target and the slot of every resource they bind.
//!
//! A program is a [`LightBase`] plus a set of [`LightFeature`]s. Only 92 of the 3 × 256
//! combinations have shaders ([`LIGHT_PROGRAMS`], generated from the rules in this file;
//! [`light_program_spec`] adds the HLSL source and defines).
//! [`light_layout_of`] gives the binding layout the shaders declare, so a draw binds exactly
//! what its program reads. The shader blobs themselves come from the `.kpak` pack (ADR 0014).

/// Base of a light program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightBase {
    /// Unshadowed lights (`light*.frag`).
    Light,
    /// Shadowed lights, sphere traced through the distance field (`light_sdf*.frag`).
    LightSdf,
    /// Directional light, per pixel over the screen (`directional*.frag`).
    Directional,
}

impl LightBase {
    /// All bases in table order.
    pub const ALL: [LightBase; 3] = [Self::Light, Self::LightSdf, Self::Directional];

    /// Stem of the shader names.
    pub const fn stem(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::LightSdf => "light_sdf",
            Self::Directional => "directional",
        }
    }
}

/// One feature of a light program; the discriminant is its bit and name order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightFeature {
    /// Height mode: distance field shadows with obstacle heights, without surface.
    Height = 0,
    /// Surface programs (surface buffer, normals).
    Surface = 1,
    /// Exact material mode (specular into a second target).
    Exact = 2,
    /// Direction material mode (light direction sums into a second target).
    Direction = 3,
    /// Shadowed line and rect lights.
    Area = 4,
    /// Masked point and spot lights.
    Mask = 5,
    /// Pattern obstacles.
    Pattern = 6,
    /// Medium transmission.
    Medium = 7,
}

impl LightFeature {
    /// All features in bit order.
    pub const ALL: [LightFeature; 8] = [
        Self::Height,
        Self::Surface,
        Self::Exact,
        Self::Direction,
        Self::Area,
        Self::Mask,
        Self::Pattern,
        Self::Medium,
    ];

    /// Suffix in shader names.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Surface => "surface",
            Self::Exact => "exact",
            Self::Direction => "direction",
            Self::Area => "area",
            Self::Mask => "mask",
            Self::Pattern => "pattern",
            Self::Medium => "medium",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`LightFeature`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LightFeatures(u8);

impl LightFeatures {
    /// No feature.
    pub const NONE: Self = Self(0);

    /// The set of `features`.
    pub const fn of(features: &[LightFeature]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < features.len() {
            bits |= features[i].bit();
            i += 1;
        }
        Self(bits)
    }

    /// The set from its bits (bit = [`LightFeature`] discriminant); all 256 values are sets.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// The bits (Chiaro's `LightFeature` mask).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `feature` is in the set.
    pub const fn contains(self, feature: LightFeature) -> bool {
        self.0 & feature.bit() != 0
    }

    /// This set with `feature` added when `on`.
    #[must_use]
    pub const fn with(self, feature: LightFeature, on: bool) -> Self {
        if on {
            Self(self.0 | feature.bit())
        } else {
            self
        }
    }

    /// The features in bit order.
    pub fn iter(self) -> impl Iterator<Item = LightFeature> {
        LightFeature::ALL
            .into_iter()
            .filter(move |&f| self.contains(f))
    }
}

/// A program of the light family: a base plus features. The stages set the features of their
/// mode; whether the combination exists says [`find_light_program`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LightProgram {
    /// The base.
    pub base: LightBase,
    /// The features.
    pub features: LightFeatures,
}

impl LightProgram {
    /// `base` without features.
    pub const fn new(base: LightBase) -> Self {
        Self {
            base,
            features: LightFeatures::NONE,
        }
    }

    /// This program with `feature` set when `on`.
    #[must_use]
    pub const fn with(self, feature: LightFeature, on: bool) -> Self {
        Self {
            base: self.base,
            features: self.features.with(feature, on),
        }
    }

    /// Whether the program has `feature`.
    pub const fn has(self, feature: LightFeature) -> bool {
        self.features.contains(feature)
    }

    const fn same(self, other: Self) -> bool {
        self.base as u8 == other.base as u8 && self.features.0 == other.features.0
    }
}

/// The second color target of a light program (format and blend in `light_pipelines`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightSecondTarget {
    /// Only the light buffer.
    None,
    /// Exact material mode: specular buffer.
    Specular,
    /// Direction material mode: light direction sums.
    Direction,
}

/// One entry of [`LIGHT_PROGRAMS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LightProgramInfo {
    /// The base.
    pub base: LightBase,
    /// The features.
    pub features: LightFeatures,
    /// Vertex shader name in the pack.
    pub vertex: &'static str,
    /// Fragment shader name in the pack.
    pub fragment: &'static str,
    /// The second color target.
    pub second: LightSecondTarget,
}

impl LightProgramInfo {
    /// The program of this entry.
    pub const fn program(&self) -> LightProgram {
        LightProgram {
            base: self.base,
            features: self.features,
        }
    }
}

/// Number of light programs.
pub const LIGHT_PROGRAM_COUNT: usize = 92;

use LightFeature as F;

// One source of truth: the rules below (`is_valid_light_program`, `vertex_of`,
// `fragment_name_of`, `second_of`) generate `LIGHT_PROGRAMS` at compile time, as Chiaro's CMake
// generates its table. The other direction (a literal table the rules read from) was rejected:
// `light_program_spec` and the invalid-combination checks still need the validity rules, so the
// knowledge would stay in two places. Fragment names are built into fixed buffers in a `static`
// (no allocation, no `unsafe`); the vertex names are seven literals chosen by rule.
// `tests/light_program.rs` pins the generated table entry by entry against a checked-in copy
// of Chiaro's table, so a rule change cannot silently reorder or rename programs.

/// Whether `program` has shaders (Chiaro `cmake/ChiaroLightPrograms.cmake`).
const fn is_valid_light_program(program: LightProgram) -> bool {
    let sdf = matches!(program.base, LightBase::LightSdf);
    let has_material = program.has(F::Exact) || program.has(F::Direction);
    // Material modes need the surface programs and exclude each other.
    if has_material && !program.has(F::Surface) {
        return false;
    }
    if program.has(F::Exact) && program.has(F::Direction) {
        return false;
    }
    // Height mode: the distance field shadows without surface.
    if program.has(F::Height) && (!sdf || program.has(F::Surface)) {
        return false;
    }
    // Area: the shadowed line and rect lights; masks for point and spot lights only.
    if program.has(F::Area) && !sdf {
        return false;
    }
    if program.has(F::Mask)
        && (program.has(F::Area) || matches!(program.base, LightBase::Directional))
    {
        return false;
    }
    // Pattern obstacles: the shadow programs only.
    !(program.has(F::Pattern) && matches!(program.base, LightBase::Light))
}

/// Whether the program draws point and spot lights only. Decides both the `_point` vertex
/// shader and the `CHI_POINT_LIGHTS` define, so both read it from here.
const fn draws_point_lights(program: LightProgram) -> bool {
    program.has(F::Mask) || (matches!(program.base, LightBase::LightSdf) && !program.has(F::Area))
}

/// Vertex shader of a valid program.
const fn vertex_of(program: LightProgram) -> &'static str {
    if matches!(program.base, LightBase::Directional) {
        return "fullscreen.vert";
    }
    // Masked lights are point lights too, but have their own vertex shader: mask wins.
    match (
        program.has(F::Surface),
        program.has(F::Mask),
        draws_point_lights(program),
    ) {
        (false, true, _) => "light_mask.vert",
        (true, true, _) => "light_surface_mask.vert",
        (false, false, true) => "light_point.vert",
        (true, false, true) => "light_surface_point.vert",
        (false, false, false) => "light.vert",
        (true, false, false) => "light_surface.vert",
    }
}

/// Second color target of a valid program: follows the material mode.
const fn second_of(program: LightProgram) -> LightSecondTarget {
    if program.has(F::Exact) {
        LightSecondTarget::Specular
    } else if program.has(F::Direction) {
        LightSecondTarget::Direction
    } else {
        LightSecondTarget::None
    }
}

/// Room for the longest fragment name, `light_sdf_surface_direction_area_pattern_medium.frag`
/// (52 bytes); a longer one fails the build in `ShaderName::push`.
const NAME_CAPACITY: usize = 64;

/// A shader name built at compile time.
struct ShaderName {
    bytes: [u8; NAME_CAPACITY],
    len: usize,
}

impl ShaderName {
    const fn new() -> Self {
        Self {
            bytes: [0; NAME_CAPACITY],
            len: 0,
        }
    }

    /// Appends `part`; past the capacity the index goes out of bounds and the build fails.
    const fn push(&mut self, part: &str) {
        let part = part.as_bytes();
        let mut i = 0;
        while i < part.len() {
            self.bytes[self.len] = part[i];
            self.len += 1;
            i += 1;
        }
    }

    const fn as_str(&'static self) -> &'static str {
        match core::str::from_utf8(self.bytes.split_at(self.len).0) {
            Ok(s) => s,
            Err(_) => panic!("shader names are built from ASCII parts"),
        }
    }
}

/// Fragment shader name: base stem, then the feature suffixes in bit order.
const fn fragment_name_of(program: LightProgram) -> ShaderName {
    let mut name = ShaderName::new();
    name.push(program.base.stem());
    let mut i = 0;
    while i < LightFeature::ALL.len() {
        let feature = LightFeature::ALL[i];
        if program.has(feature) {
            name.push("_");
            name.push(feature.suffix());
        }
        i += 1;
    }
    name.push(".frag");
    name
}

/// The valid programs in Chiaro's table order: base, then feature bits ascending.
const VALID_LIGHT_PROGRAMS: [LightProgram; LIGHT_PROGRAM_COUNT] = {
    let mut out = [LightProgram::new(LightBase::Light); LIGHT_PROGRAM_COUNT];
    let mut n = 0;
    let mut b = 0;
    while b < LightBase::ALL.len() {
        let mut bits: u16 = 0;
        while bits <= u8::MAX as u16 {
            let program = LightProgram {
                base: LightBase::ALL[b],
                features: LightFeatures::from_bits(bits as u8),
            };
            if is_valid_light_program(program) {
                // More valid programs than LIGHT_PROGRAM_COUNT fail the build here.
                out[n] = program;
                n += 1;
            }
            bits += 1;
        }
        b += 1;
    }
    assert!(
        n == LIGHT_PROGRAM_COUNT,
        "the rules give fewer than LIGHT_PROGRAM_COUNT programs"
    );
    out
};

/// Storage the fragment names of `LIGHT_PROGRAMS` point into. A `static`, so the names are
/// `&'static str` without allocation.
static FRAGMENT_NAMES: [ShaderName; LIGHT_PROGRAM_COUNT] = {
    let mut out = [const { ShaderName::new() }; LIGHT_PROGRAM_COUNT];
    let mut i = 0;
    while i < LIGHT_PROGRAM_COUNT {
        out[i] = fragment_name_of(VALID_LIGHT_PROGRAMS[i]);
        i += 1;
    }
    out
};

/// Every valid light program, in Chiaro's table order (base, then feature bits ascending),
/// generated from the rules above. All of them blend ONE + ONE into the light buffer.
pub const LIGHT_PROGRAMS: [LightProgramInfo; LIGHT_PROGRAM_COUNT] = {
    let unset = LightProgramInfo {
        base: LightBase::Light,
        features: LightFeatures::NONE,
        vertex: "",
        fragment: "",
        second: LightSecondTarget::None,
    };
    let mut out = [unset; LIGHT_PROGRAM_COUNT];
    let mut i = 0;
    while i < LIGHT_PROGRAM_COUNT {
        let program = VALID_LIGHT_PROGRAMS[i];
        out[i] = LightProgramInfo {
            base: program.base,
            features: program.features,
            vertex: vertex_of(program),
            fragment: FRAGMENT_NAMES[i].as_str(),
            second: second_of(program),
        };
        i += 1;
    }
    out
};

/// The entry of `program`, `None` if that combination has no shaders.
pub const fn find_light_program(program: LightProgram) -> Option<&'static LightProgramInfo> {
    match light_program_index(program) {
        Some(i) => Some(&LIGHT_PROGRAMS[i]),
        None => None,
    }
}

/// Index of `program` in [`LIGHT_PROGRAMS`].
pub const fn light_program_index(program: LightProgram) -> Option<usize> {
    let mut i = 0;
    while i < LIGHT_PROGRAM_COUNT {
        if LIGHT_PROGRAMS[i].program().same(program) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// What the rules of `cmake/ChiaroLightPrograms.cmake` say about one program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightProgramSpec {
    /// Vertex shader name.
    pub vertex: String,
    /// Fragment shader name (base stem plus feature suffixes).
    pub fragment: String,
    /// HLSL source the fragment shader is compiled from (`light_surface.frag`, …).
    pub source: String,
    /// Preprocessor defines, in the order the compiled shaders were built with.
    pub defines: Vec<&'static str>,
    /// The second color target.
    pub second: LightSecondTarget,
}

impl LightProgramSpec {
    /// Whether the fragment shader is a variant of another source (own defines or source).
    pub fn is_variant(&self) -> bool {
        !self.defines.is_empty() || self.source != self.fragment
    }
}

/// The full rules for one program: `None` if `program` has no shaders, otherwise its shaders
/// and second target (read from [`LIGHT_PROGRAMS`]) plus the HLSL source and defines it is
/// compiled from. Allocates; meant for tools and tests.
pub fn light_program_spec(program: LightProgram) -> Option<LightProgramSpec> {
    let info = find_light_program(program)?;
    let has = |f| program.has(f);

    // The HLSL source: height and surface programs have their own; the other features are
    // defines on it.
    let mut source = String::from(program.base.stem());
    if has(F::Height) {
        source.push_str("_height");
    } else if has(F::Surface) {
        source.push_str("_surface");
    }
    source.push_str(".frag");

    let mut defines = Vec::new();
    if has(F::Exact) {
        defines.push("CHI_MATERIAL_EXACT");
    } else if has(F::Direction) {
        defines.push("CHI_MATERIAL_DIRECTION");
    }
    if draws_point_lights(program) {
        defines.push("CHI_POINT_LIGHTS");
    }
    if has(F::Mask) {
        defines.push("CHI_LIGHT_MASK");
    }
    if has(F::Medium) {
        defines.push("CHI_MEDIUM");
    }
    if has(F::Pattern) {
        defines.push("CHI_PATTERN");
    }

    Some(LightProgramSpec {
        vertex: String::from(info.vertex),
        fragment: String::from(info.fragment),
        source,
        defines,
        second: info.second,
    })
}

// ---- Binding layout (E153) ---------------------------------------------------------------

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

/// The layout of `program`, in the order its shaders declare their resources: the base's own
/// samplers (Light: [Surface, Material]; LightSdf: [Surface,] OcclusionMask, DistanceField,
/// [OcclusionHeight, Material]; Directional: OcclusionMask, DistanceField, OcclusionHeight,
/// [Surface, Material]), then Profile (not Directional), Mask, MediumMap, PatternIds,
/// PatternAtlas; uniforms likewise the base's own, then MediumMap, Pattern.
pub const fn light_layout_of(program: LightProgram) -> LightLayout {
    let mut b = LayoutBuilder::new();
    let surface = program.has(F::Surface);
    let exact = program.has(F::Exact);
    match program.base {
        LightBase::Light => {
            if surface {
                b.sampler(LightSampler::Surface);
                if exact {
                    b.sampler(LightSampler::Material);
                }
                b.uniform(LightUniform::Surface);
            }
        }
        LightBase::LightSdf => {
            if surface {
                b.sampler(LightSampler::Surface);
            }
            b.sampler(LightSampler::OcclusionMask);
            b.sampler(LightSampler::DistanceField);
            if surface || program.has(F::Height) {
                b.sampler(LightSampler::OcclusionHeight);
            }
            if exact {
                b.sampler(LightSampler::Material);
            }
            if surface {
                b.uniform(LightUniform::Surface);
            }
            b.uniform(LightUniform::Shadow);
        }
        LightBase::Directional => {
            b.sampler(LightSampler::OcclusionMask);
            b.sampler(LightSampler::DistanceField);
            b.sampler(LightSampler::OcclusionHeight);
            b.uniform(LightUniform::Directional);
            if surface {
                b.sampler(LightSampler::Surface);
                if exact {
                    b.sampler(LightSampler::Material);
                }
                b.uniform(LightUniform::Surface);
            }
        }
    }
    if !matches!(program.base, LightBase::Directional) {
        b.sampler(LightSampler::Profile);
        b.storage(LightStorage::Lights);
        b.vertex_uniforms(1);
    }
    if program.has(F::Mask) {
        b.sampler(LightSampler::Mask);
        b.storage(LightStorage::Masks);
    }
    if program.has(F::Medium) {
        b.sampler(LightSampler::MediumMap);
        b.uniform(LightUniform::MediumMap);
    }
    if program.has(F::Pattern) {
        b.sampler(LightSampler::PatternIds);
        b.sampler(LightSampler::PatternAtlas);
        b.uniform(LightUniform::Pattern);
        b.storage(LightStorage::PatternObstacles);
        b.storage(LightStorage::Patterns);
        b.storage(LightStorage::PatternData);
    }
    b.finish()
}

// Chiaro's static_assert: the fullest program has ten samplers.
const _: () = assert!(
    light_layout_of(LightProgram {
        base: LightBase::LightSdf,
        features: LightFeatures::of(&[F::Surface, F::Exact, F::Mask, F::Medium, F::Pattern]),
    })
    .sampler_count
        == 10
);
