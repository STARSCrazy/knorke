//! The medium programs (P29) against Chiaro's compiled shaders (port of the medium parts of
//! `tests/unit/LightProgramLayoutTests.cpp`, E156):
//!
//! - every medium program's reflection counts and bindings are the slots [`medium_layout_of`]
//!   gives;
//! - the map passes' named slots ([`slots`]) match their shaders;
//! - the member offsets of every medium uniform block the compiler gave (`OpMemberDecorate
//!   Offset`) equal the offsets of the Rust structs, in every shader that declares them.
//!
//! Needs the pack from `cargo xtask shaders` in `target/shaders/` (Chiaro 9bc5b9b shaders, zlib
//! license); skips without it.
#![cfg(feature = "gpu")]

use core::mem::{offset_of, size_of};
mod common;

use common::check_layout;
use knorke_gpu::kpak::ShaderPack;
use knorke_gpu::test_support::chiaro_pack;
use knorke_gpu::test_support::spirv::decode;
use knorke_light2d::light_program::{LIGHT_PROGRAMS, LightFeature};
use knorke_light2d::medium_map::{
    MEDIUM_MAP_PROGRAMS, MediumMapParams, MediumParams, MediumProgram, MediumResolveParams,
    MediumSplatParams, MediumTopParams, medium_layout_of, slots,
};
use knorke_light2d::medium_stage::{
    MediumLightPassParams, MediumMaskLightPassParams, PolarQueryParams, PolarRowParams,
    SunHeightParams, SunParams,
};

#[test]
fn every_medium_program_layout_matches_its_shaders() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for program in MediumProgram::ALL {
        let layout = medium_layout_of(program);
        assert!(layout.sampler_count > 0, "{program:?}");
        let frag = pack.get(program.fragment()).unwrap();
        let vert = pack.get(program.vertex()).unwrap();
        check_layout(&frag, &vert, &layout);
    }
}

/// `(set, binding)` of the resource `name` in `code`.
fn binding(code: &[u8], name: &str) -> (u32, u32) {
    *decode(code)
        .bindings
        .get(name)
        .unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn the_map_passes_named_slots_match_their_shaders() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    let [splat, top, resolve] = MEDIUM_MAP_PROGRAMS;
    let v = pack.get(splat.vertex).unwrap();
    assert_eq!(
        (v.reflection.samplers, v.reflection.uniform_buffers),
        (1, 2)
    );
    assert_eq!(
        binding(v.code, "MediumTex"),
        (0, slots::SPLAT_LAYER_SAMPLER)
    );
    assert_eq!(
        binding(v.code, "MediumParams"),
        (1, slots::SPLAT_MEDIUM_PARAMS)
    );
    assert_eq!(
        binding(v.code, "MediumSplatParams"),
        (1, slots::SPLAT_PARAMS)
    );
    let v = pack.get(top.vertex).unwrap();
    assert_eq!(
        (v.reflection.samplers, v.reflection.uniform_buffers),
        (1, 1)
    );
    assert_eq!(binding(v.code, "Union"), (0, slots::TOP_SLAB_SAMPLER));
    assert_eq!(binding(v.code, "TopParams"), (1, slots::TOP_PARAMS));
    let f = pack.get(resolve.fragment).unwrap();
    assert_eq!(
        (f.reflection.samplers, f.reflection.uniform_buffers),
        (slots::RESOLVE_SAMPLER_COUNT as u16, 1)
    );
}

/// `(HLSL member, Rust offset, Rust size)`.
type Layout = &'static [(&'static str, usize, usize)];

macro_rules! f {
    ($t:ty, $hlsl:literal, $field:ident, $n:literal) => {
        ($hlsl, offset_of!($t, $field), $n * 4)
    };
}

const MEDIUM: Layout = &[
    f!(MediumParams, "MediumInvAxes", inv_axes, 4),
    f!(MediumParams, "MediumOriginZAxis", origin_z_axis, 4),
    f!(MediumParams, "MediumSizes", sizes, 4),
    f!(MediumParams, "MediumView", view, 4),
    f!(MediumParams, "MediumSteps", steps, 4),
    f!(MediumParams, "MediumHaze", haze, 4),
    f!(MediumParams, "MediumHazeColor", haze_color, 4),
    f!(MediumParams, "MediumAmbient", ambient, 4),
    f!(MediumParams, "MediumTable", media, 64),
    f!(MediumParams, "MediumLightPass", light_pass, 4),
    f!(MediumParams, "MediumPhase", phase, 16),
];
const MAP: Layout = &[
    f!(MediumMapParams, "MediumMapX", map_x, 4),
    f!(MediumMapParams, "MediumMapY", map_y, 4),
    f!(MediumMapParams, "MediumMapSize", map_size, 4),
    f!(MediumMapParams, "MediumMapInvAxes", inv_axes, 4),
    f!(MediumMapParams, "MediumMapOrigin", origin, 4),
];
const SPLAT: Layout = &[
    f!(MediumSplatParams, "SceneFromMap", scene_from_map, 4),
    f!(MediumSplatParams, "MapFromScene", map_from_scene, 4),
    f!(MediumSplatParams, "MapBlocks", map_blocks, 4),
];
const TOP: Layout = &[f!(MediumTopParams, "TopMap", map, 4)];
const RESOLVE: Layout = &[f!(MediumResolveParams, "Resolve", resolve, 4)];

// The stage's blocks (P30). `medium_light.vert` declares `LightOffset` only with
// `CHI_LIGHT_MASK`; the unmasked block is the masked one's prefix.
const BASE: usize = offset_of!(MediumMaskLightPassParams, base);
const LIGHT_PASS: Layout = &[
    (
        "OriginXAxis",
        BASE + offset_of!(MediumLightPassParams, origin_x_axis),
        16,
    ),
    (
        "YAxisZAxis",
        BASE + offset_of!(MediumLightPassParams, y_axis_z_axis),
        16,
    ),
    ("Sizes", BASE + offset_of!(MediumLightPassParams, sizes), 16),
    f!(MediumMaskLightPassParams, "LightOffset", light_offset, 4),
];
const POLAR_QUERY: Layout = &[
    f!(PolarQueryParams, "PolarMaskX", mask_x, 4),
    f!(PolarQueryParams, "PolarMaskY", mask_y, 4),
    f!(PolarQueryParams, "PolarAtlasSize", atlas, 4),
];
const POLAR_ROW: Layout = &[
    f!(PolarRowParams, "PolarMaskX", mask_x, 4),
    f!(PolarRowParams, "PolarMaskY", mask_y, 4),
    f!(PolarRowParams, "PolarLight", light, 4),
    f!(PolarRowParams, "PolarDomain", domain, 4),
];
const SUN: Layout = &[
    f!(SunParams, "SunColor", color, 4),
    f!(SunParams, "SunToLight", to_light, 4),
    f!(SunParams, "SunMaskX", mask_x, 4),
    f!(SunParams, "SunMaskY", mask_y, 4),
    f!(SunParams, "SunAtlas", atlas, 4),
    f!(SunParams, "SunBlock", block, 4),
];
const SUN_HEIGHT: Layout = &[
    f!(SunHeightParams, "SunMarch", march, 4),
    f!(SunHeightParams, "SunDomain", domain, 4),
];

const STRUCTS: [(&str, Layout, usize); 10] = [
    (
        "type.MediumLightPassParams",
        LIGHT_PASS,
        size_of::<MediumMaskLightPassParams>(),
    ),
    (
        "type.PolarQueryParams",
        POLAR_QUERY,
        size_of::<PolarQueryParams>(),
    ),
    (
        "type.PolarRowParams",
        POLAR_ROW,
        size_of::<PolarRowParams>(),
    ),
    ("type.SunParams", SUN, size_of::<SunParams>()),
    (
        "type.SunHeightParams",
        SUN_HEIGHT,
        size_of::<SunHeightParams>(),
    ),
    ("type.MediumParams", MEDIUM, size_of::<MediumParams>()),
    ("type.MediumMapParams", MAP, size_of::<MediumMapParams>()),
    (
        "type.MediumSplatParams",
        SPLAT,
        size_of::<MediumSplatParams>(),
    ),
    ("type.TopParams", TOP, size_of::<MediumTopParams>()),
    (
        "type.ResolveParams",
        RESOLVE,
        size_of::<MediumResolveParams>(),
    ),
];

#[test]
fn rust_layouts_match_every_medium_shader() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    // The map passes, the medium programs and the `*_medium` light programs.
    let mut shaders: Vec<&str> = MEDIUM_MAP_PROGRAMS
        .iter()
        .flat_map(|p| [p.vertex, p.fragment])
        .chain(
            MediumProgram::ALL
                .iter()
                .flat_map(|p| [p.vertex(), p.fragment()]),
        )
        .chain(
            LIGHT_PROGRAMS
                .iter()
                .filter(|p| p.program().has(LightFeature::Medium))
                .map(|p| p.fragment),
        )
        .collect();
    shaders.sort_unstable();
    shaders.dedup();
    let mut seen = [0usize; STRUCTS.len()];
    for shader in &shaders {
        for (name, members) in decode(pack.get(shader).unwrap().code).structs {
            let Some(k) = STRUCTS.iter().position(|s| s.0 == name) else {
                continue;
            };
            let (_, layout, size) = STRUCTS[k];
            seen[k] += 1;
            for (member, offset) in &members {
                let &(_, rust, len) = layout
                    .iter()
                    .find(|m| m.0 == member)
                    .unwrap_or_else(|| panic!("{shader}: {name}.{member} has no Rust field"));
                assert_eq!(
                    *offset as usize, rust,
                    "{shader}: {name}.{member} at {offset}, Rust at {rust}"
                );
                assert!(rust + len <= size, "{shader}: {name}.{member}");
            }
        }
    }
    for (k, (name, _, _)) in STRUCTS.iter().enumerate() {
        assert!(seen[k] > 0, "no shader declares {name}");
    }
}
