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
// Altered: rewritten in Rust for the Knorke Engine. The CMake rules became `light_program_spec`;
// the table Chiaro generated from them is the literal `LIGHT_PROGRAMS`, and a test checks that
// the rules reproduce it entry by entry. Feature bits are a typed set (`LightFeatures`).

//! The light program family (Chiaro E152, E153): which light and shadow fragment programs
//! exist, their shaders, their second color target and the slot of every resource they bind.
//!
//! A program is a [`LightBase`] plus a set of [`LightFeature`]s. Only 92 of the 3 × 256
//! combinations have shaders ([`LIGHT_PROGRAMS`]); the rules are [`light_program_spec`].
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

const fn info(
    base: LightBase,
    features: &[LightFeature],
    vertex: &'static str,
    fragment: &'static str,
    second: LightSecondTarget,
) -> LightProgramInfo {
    LightProgramInfo {
        base,
        features: LightFeatures::of(features),
        vertex,
        fragment,
        second,
    }
}

/// Number of light programs.
pub const LIGHT_PROGRAM_COUNT: usize = 92;

use LightFeature as F;

/// One program per line: `Base [features] "vertex" "fragment" SecondTarget;`.
macro_rules! light_programs {
    ($($base:ident [$($feature:ident),*] $vertex:literal $fragment:literal $second:ident;)*) => {
        [$(info(
            LightBase::$base,
            &[$(LightFeature::$feature),*],
            $vertex,
            $fragment,
            LightSecondTarget::$second,
        )),*]
    };
}

/// Every valid light program, in Chiaro's table order (base, then feature bits ascending).
/// All of them blend ONE + ONE into the light buffer.
pub const LIGHT_PROGRAMS: [LightProgramInfo; LIGHT_PROGRAM_COUNT] = light_programs! {
    Light [] "light.vert" "light.frag" None;
    Light [Surface] "light_surface.vert" "light_surface.frag" None;
    Light [Surface, Exact] "light_surface.vert" "light_surface_exact.frag" Specular;
    Light [Surface, Direction] "light_surface.vert" "light_surface_direction.frag" Direction;
    Light [Mask] "light_mask.vert" "light_mask.frag" None;
    Light [Surface, Mask] "light_surface_mask.vert" "light_surface_mask.frag" None;
    Light [Surface, Exact, Mask] "light_surface_mask.vert" "light_surface_exact_mask.frag" Specular;
    Light [Surface, Direction, Mask] "light_surface_mask.vert" "light_surface_direction_mask.frag" Direction;
    Light [Medium] "light.vert" "light_medium.frag" None;
    Light [Surface, Medium] "light_surface.vert" "light_surface_medium.frag" None;
    Light [Surface, Exact, Medium] "light_surface.vert" "light_surface_exact_medium.frag" Specular;
    Light [Surface, Direction, Medium] "light_surface.vert" "light_surface_direction_medium.frag" Direction;
    Light [Mask, Medium] "light_mask.vert" "light_mask_medium.frag" None;
    Light [Surface, Mask, Medium] "light_surface_mask.vert" "light_surface_mask_medium.frag" None;
    Light [Surface, Exact, Mask, Medium] "light_surface_mask.vert" "light_surface_exact_mask_medium.frag" Specular;
    Light [Surface, Direction, Mask, Medium] "light_surface_mask.vert" "light_surface_direction_mask_medium.frag" Direction;
    LightSdf [] "light_point.vert" "light_sdf.frag" None;
    LightSdf [Height] "light_point.vert" "light_sdf_height.frag" None;
    LightSdf [Surface] "light_surface_point.vert" "light_sdf_surface.frag" None;
    LightSdf [Surface, Exact] "light_surface_point.vert" "light_sdf_surface_exact.frag" Specular;
    LightSdf [Surface, Direction] "light_surface_point.vert" "light_sdf_surface_direction.frag" Direction;
    LightSdf [Area] "light.vert" "light_sdf_area.frag" None;
    LightSdf [Height, Area] "light.vert" "light_sdf_height_area.frag" None;
    LightSdf [Surface, Area] "light_surface.vert" "light_sdf_surface_area.frag" None;
    LightSdf [Surface, Exact, Area] "light_surface.vert" "light_sdf_surface_exact_area.frag" Specular;
    LightSdf [Surface, Direction, Area] "light_surface.vert" "light_sdf_surface_direction_area.frag" Direction;
    LightSdf [Mask] "light_mask.vert" "light_sdf_mask.frag" None;
    LightSdf [Height, Mask] "light_mask.vert" "light_sdf_height_mask.frag" None;
    LightSdf [Surface, Mask] "light_surface_mask.vert" "light_sdf_surface_mask.frag" None;
    LightSdf [Surface, Exact, Mask] "light_surface_mask.vert" "light_sdf_surface_exact_mask.frag" Specular;
    LightSdf [Surface, Direction, Mask] "light_surface_mask.vert" "light_sdf_surface_direction_mask.frag" Direction;
    LightSdf [Pattern] "light_point.vert" "light_sdf_pattern.frag" None;
    LightSdf [Height, Pattern] "light_point.vert" "light_sdf_height_pattern.frag" None;
    LightSdf [Surface, Pattern] "light_surface_point.vert" "light_sdf_surface_pattern.frag" None;
    LightSdf [Surface, Exact, Pattern] "light_surface_point.vert" "light_sdf_surface_exact_pattern.frag" Specular;
    LightSdf [Surface, Direction, Pattern] "light_surface_point.vert" "light_sdf_surface_direction_pattern.frag" Direction;
    LightSdf [Area, Pattern] "light.vert" "light_sdf_area_pattern.frag" None;
    LightSdf [Height, Area, Pattern] "light.vert" "light_sdf_height_area_pattern.frag" None;
    LightSdf [Surface, Area, Pattern] "light_surface.vert" "light_sdf_surface_area_pattern.frag" None;
    LightSdf [Surface, Exact, Area, Pattern] "light_surface.vert" "light_sdf_surface_exact_area_pattern.frag" Specular;
    LightSdf [Surface, Direction, Area, Pattern] "light_surface.vert" "light_sdf_surface_direction_area_pattern.frag" Direction;
    LightSdf [Mask, Pattern] "light_mask.vert" "light_sdf_mask_pattern.frag" None;
    LightSdf [Height, Mask, Pattern] "light_mask.vert" "light_sdf_height_mask_pattern.frag" None;
    LightSdf [Surface, Mask, Pattern] "light_surface_mask.vert" "light_sdf_surface_mask_pattern.frag" None;
    LightSdf [Surface, Exact, Mask, Pattern] "light_surface_mask.vert" "light_sdf_surface_exact_mask_pattern.frag" Specular;
    LightSdf [Surface, Direction, Mask, Pattern] "light_surface_mask.vert" "light_sdf_surface_direction_mask_pattern.frag" Direction;
    LightSdf [Medium] "light_point.vert" "light_sdf_medium.frag" None;
    LightSdf [Height, Medium] "light_point.vert" "light_sdf_height_medium.frag" None;
    LightSdf [Surface, Medium] "light_surface_point.vert" "light_sdf_surface_medium.frag" None;
    LightSdf [Surface, Exact, Medium] "light_surface_point.vert" "light_sdf_surface_exact_medium.frag" Specular;
    LightSdf [Surface, Direction, Medium] "light_surface_point.vert" "light_sdf_surface_direction_medium.frag" Direction;
    LightSdf [Area, Medium] "light.vert" "light_sdf_area_medium.frag" None;
    LightSdf [Height, Area, Medium] "light.vert" "light_sdf_height_area_medium.frag" None;
    LightSdf [Surface, Area, Medium] "light_surface.vert" "light_sdf_surface_area_medium.frag" None;
    LightSdf [Surface, Exact, Area, Medium] "light_surface.vert" "light_sdf_surface_exact_area_medium.frag" Specular;
    LightSdf [Surface, Direction, Area, Medium] "light_surface.vert" "light_sdf_surface_direction_area_medium.frag" Direction;
    LightSdf [Mask, Medium] "light_mask.vert" "light_sdf_mask_medium.frag" None;
    LightSdf [Height, Mask, Medium] "light_mask.vert" "light_sdf_height_mask_medium.frag" None;
    LightSdf [Surface, Mask, Medium] "light_surface_mask.vert" "light_sdf_surface_mask_medium.frag" None;
    LightSdf [Surface, Exact, Mask, Medium] "light_surface_mask.vert" "light_sdf_surface_exact_mask_medium.frag" Specular;
    LightSdf [Surface, Direction, Mask, Medium] "light_surface_mask.vert" "light_sdf_surface_direction_mask_medium.frag" Direction;
    LightSdf [Pattern, Medium] "light_point.vert" "light_sdf_pattern_medium.frag" None;
    LightSdf [Height, Pattern, Medium] "light_point.vert" "light_sdf_height_pattern_medium.frag" None;
    LightSdf [Surface, Pattern, Medium] "light_surface_point.vert" "light_sdf_surface_pattern_medium.frag" None;
    LightSdf [Surface, Exact, Pattern, Medium] "light_surface_point.vert" "light_sdf_surface_exact_pattern_medium.frag" Specular;
    LightSdf [Surface, Direction, Pattern, Medium] "light_surface_point.vert" "light_sdf_surface_direction_pattern_medium.frag" Direction;
    LightSdf [Area, Pattern, Medium] "light.vert" "light_sdf_area_pattern_medium.frag" None;
    LightSdf [Height, Area, Pattern, Medium] "light.vert" "light_sdf_height_area_pattern_medium.frag" None;
    LightSdf [Surface, Area, Pattern, Medium] "light_surface.vert" "light_sdf_surface_area_pattern_medium.frag" None;
    LightSdf [Surface, Exact, Area, Pattern, Medium] "light_surface.vert" "light_sdf_surface_exact_area_pattern_medium.frag" Specular;
    LightSdf [Surface, Direction, Area, Pattern, Medium] "light_surface.vert" "light_sdf_surface_direction_area_pattern_medium.frag" Direction;
    LightSdf [Mask, Pattern, Medium] "light_mask.vert" "light_sdf_mask_pattern_medium.frag" None;
    LightSdf [Height, Mask, Pattern, Medium] "light_mask.vert" "light_sdf_height_mask_pattern_medium.frag" None;
    LightSdf [Surface, Mask, Pattern, Medium] "light_surface_mask.vert" "light_sdf_surface_mask_pattern_medium.frag" None;
    LightSdf [Surface, Exact, Mask, Pattern, Medium] "light_surface_mask.vert" "light_sdf_surface_exact_mask_pattern_medium.frag" Specular;
    LightSdf [Surface, Direction, Mask, Pattern, Medium] "light_surface_mask.vert" "light_sdf_surface_direction_mask_pattern_medium.frag" Direction;
    Directional [] "fullscreen.vert" "directional.frag" None;
    Directional [Surface] "fullscreen.vert" "directional_surface.frag" None;
    Directional [Surface, Exact] "fullscreen.vert" "directional_surface_exact.frag" Specular;
    Directional [Surface, Direction] "fullscreen.vert" "directional_surface_direction.frag" Direction;
    Directional [Pattern] "fullscreen.vert" "directional_pattern.frag" None;
    Directional [Surface, Pattern] "fullscreen.vert" "directional_surface_pattern.frag" None;
    Directional [Surface, Exact, Pattern] "fullscreen.vert" "directional_surface_exact_pattern.frag" Specular;
    Directional [Surface, Direction, Pattern] "fullscreen.vert" "directional_surface_direction_pattern.frag" Direction;
    Directional [Medium] "fullscreen.vert" "directional_medium.frag" None;
    Directional [Surface, Medium] "fullscreen.vert" "directional_surface_medium.frag" None;
    Directional [Surface, Exact, Medium] "fullscreen.vert" "directional_surface_exact_medium.frag" Specular;
    Directional [Surface, Direction, Medium] "fullscreen.vert" "directional_surface_direction_medium.frag" Direction;
    Directional [Pattern, Medium] "fullscreen.vert" "directional_pattern_medium.frag" None;
    Directional [Surface, Pattern, Medium] "fullscreen.vert" "directional_surface_pattern_medium.frag" None;
    Directional [Surface, Exact, Pattern, Medium] "fullscreen.vert" "directional_surface_exact_pattern_medium.frag" Specular;
    Directional [Surface, Direction, Pattern, Medium] "fullscreen.vert" "directional_surface_direction_pattern_medium.frag" Direction;
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

/// The rules of the light program family: `None` if `program` has no shaders, otherwise its
/// shaders, source, defines and second target. [`LIGHT_PROGRAMS`] is exactly the valid
/// programs of these rules. Allocates; meant for tools and tests.
pub fn light_program_spec(program: LightProgram) -> Option<LightProgramSpec> {
    let has = |f| program.has(f);
    let base = program.base;
    let sdf = base == LightBase::LightSdf;
    // Material modes need the surface programs and exclude each other.
    if (has(F::Exact) || has(F::Direction)) && !has(F::Surface) {
        return None;
    }
    if has(F::Exact) && has(F::Direction) {
        return None;
    }
    // Height mode: the distance field shadows without surface.
    if has(F::Height) && (!sdf || has(F::Surface)) {
        return None;
    }
    // Area: the shadowed line and rect lights; masks for point and spot lights only.
    if has(F::Area) && !sdf {
        return None;
    }
    if has(F::Mask) && (has(F::Area) || base == LightBase::Directional) {
        return None;
    }
    // Pattern obstacles: the shadow programs only.
    if has(F::Pattern) && base == LightBase::Light {
        return None;
    }

    let mut fragment = String::from(base.stem());
    for f in program.features.iter() {
        fragment.push('_');
        fragment.push_str(f.suffix());
    }
    fragment.push_str(".frag");

    let mut source = String::from(base.stem());
    if has(F::Height) {
        source.push_str("_height");
    } else if has(F::Surface) {
        source.push_str("_surface");
    }
    source.push_str(".frag");

    let point = has(F::Mask) || (sdf && !has(F::Area));
    let vertex = if base == LightBase::Directional {
        String::from("fullscreen.vert")
    } else {
        let mut v = String::from("light");
        if has(F::Surface) {
            v.push_str("_surface");
        }
        if has(F::Mask) {
            v.push_str("_mask");
        } else if point {
            v.push_str("_point");
        }
        v.push_str(".vert");
        v
    };

    let mut defines = Vec::new();
    if has(F::Exact) {
        defines.push("CHI_MATERIAL_EXACT");
    } else if has(F::Direction) {
        defines.push("CHI_MATERIAL_DIRECTION");
    }
    if point {
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

    let second = if has(F::Exact) {
        LightSecondTarget::Specular
    } else if has(F::Direction) {
        LightSecondTarget::Direction
    } else {
        LightSecondTarget::None
    };
    Some(LightProgramSpec {
        vertex,
        fragment,
        source,
        defines,
        second,
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
