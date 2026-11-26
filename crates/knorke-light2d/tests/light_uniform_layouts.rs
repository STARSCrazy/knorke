//! The uniform blocks and storage records of the light pass (P25) and of the light rows (ADR
//! 0019) against the compiled shaders: for every shader of the 92 light programs and of the row
//! programs in the SPIR-V pack, the member offsets the
//! compiler gave each `cbuffer` and `StructuredBuffer` element (`OpMemberDecorate Offset`)
//! must equal the offsets of the Rust structs, member by member. A slot or field written at the
//! wrong place gives silent image errors; this catches it without a GPU.
//!
//! Needs the pack from `cargo xtask shaders` in `target/shaders/` (Chiaro 9bc5b9b shaders,
//! zlib license); skips without it.
#![cfg(feature = "gpu")]

use core::mem::{offset_of, size_of};

use knorke_gpu::kpak::ShaderPack;
use knorke_gpu::test_support::chiaro_pack;
use knorke_gpu::test_support::spirv::decode;
use knorke_light2d::gpu_light::{GpuDirectional, GpuLight, GpuMask};
use knorke_light2d::light_program::LIGHT_PROGRAMS;
use knorke_light2d::lighting::{
    DirectionalParams, LightPassParams, LightSurfacePassParams, SurfaceParams, SurfaceShadowParams,
};
use knorke_light2d::pattern_occluders::{GpuPattern, GpuPatternObstacle, PatternParams};
use knorke_light2d::rows::{RowBuildParams, RowEdgeParams, RowLamp, RowLookupParams};

/// `(HLSL member, Rust offset, Rust size)`; members named `Pad` are padding.
type Layout = &'static [(&'static str, usize, usize)];

macro_rules! f {
    ($t:ty, $hlsl:literal, $field:ident, $n:literal) => {
        ($hlsl, offset_of!($t, $field), $n * 4)
    };
}

const GPU_LIGHT: Layout = &[
    f!(GpuLight, "positionRadius", position_radius, 4),
    f!(GpuLight, "colorFalloff", color_falloff, 4),
    f!(GpuLight, "directionType", direction_type, 4),
    f!(GpuLight, "spotCos", spot_cos, 4),
    f!(GpuLight, "shadow", shadow, 4),
    f!(GpuLight, "profile", profile, 4),
    f!(GpuLight, "area", area, 4),
    f!(GpuLight, "areaMask", area_mask, 4),
];
const GPU_MASK: Layout = &[
    f!(GpuMask, "n", n, 4),
    f!(GpuMask, "p", p, 4),
    f!(GpuMask, "q", q, 4),
];
const LIGHT_PASS: Layout = &[
    f!(LightPassParams, "OriginXAxis", origin_x_axis, 4),
    f!(LightPassParams, "YAxisSceneSize", y_axis_scene_size, 4),
    f!(LightPassParams, "MaskTexelScale", mask_texel_scale, 2),
    f!(LightPassParams, "MaskMargin", mask_margin, 1),
    f!(LightPassParams, "InstanceOffset", instance_offset, 1),
    f!(LightPassParams, "MaskRemainder", mask_remainder, 2),
    f!(LightPassParams, "MaskBase", mask_base, 1),
];
const SURFACE_PASS: Layout = &[
    f!(LightSurfacePassParams, "OriginXAxis", origin_x_axis, 4),
    f!(
        LightSurfacePassParams,
        "YAxisSceneSize",
        y_axis_scene_size,
        4
    ),
    f!(
        LightSurfacePassParams,
        "ZAxisMaxHeight",
        z_axis_max_height,
        4
    ),
    f!(LightSurfacePassParams, "InstanceOffset", instance_offset, 1),
    f!(LightSurfacePassParams, "MaskBase", mask_base, 1),
];
const SURFACE: Layout = &[
    f!(SurfaceParams, "InvAxes", inv_axes, 4),
    f!(SurfaceParams, "OriginZAxis", origin_z_axis, 4),
    f!(SurfaceParams, "SceneStep", scene_step, 4),
    f!(SurfaceParams, "MaskMap", mask_map, 4),
    f!(SurfaceParams, "MaskRemainder", mask_remainder, 4),
    f!(SurfaceParams, "XYAxes", xy_axes, 4),
    f!(SurfaceParams, "ViewDir", view_dir, 4),
    f!(SurfaceParams, "ScreenRight", screen_right, 4),
    f!(SurfaceParams, "ZAxisSpec", z_axis_spec, 4),
    f!(SurfaceParams, "Wet", wet, 4),
];
const SHADOW: Layout = &[
    f!(SurfaceShadowParams, "DomainSize", domain_size, 4),
    f!(SurfaceShadowParams, "HeightParams", height_params, 4),
];
const DIRECTIONAL: Layout = &[
    (
        "DirSlope",
        offset_of!(DirectionalParams, light) + offset_of!(GpuDirectional, dir_slope),
        16,
    ),
    (
        "Color",
        offset_of!(DirectionalParams, light) + offset_of!(GpuDirectional, color),
        16,
    ),
    (
        "World",
        offset_of!(DirectionalParams, light) + offset_of!(GpuDirectional, world),
        16,
    ),
    f!(DirectionalParams, "SceneSizeMargin", scene_size_margin, 4),
    f!(
        DirectionalParams,
        "TexelScaleRemainder",
        texel_scale_remainder,
        4
    ),
    f!(DirectionalParams, "DomainSize", domain_size, 4),
    f!(DirectionalParams, "HeightParams", height_params, 4),
];

const ROW_LAMP: Layout = &[
    ("light", offset_of!(RowLamp, light), size_of::<GpuLight>()),
    f!(RowLamp, "row", row, 4),
    ("mask", offset_of!(RowLamp, mask), size_of::<GpuMask>()),
    f!(RowLamp, "window", window, 4),
    f!(RowLamp, "windowV", window_v, 4),
];
/// The row vertex shader reads the surface pass block, `MaskBase` as its atlas height.
const ROW_PASS: Layout = &[
    f!(LightSurfacePassParams, "OriginXAxis", origin_x_axis, 4),
    f!(
        LightSurfacePassParams,
        "YAxisSceneSize",
        y_axis_scene_size,
        4
    ),
    f!(
        LightSurfacePassParams,
        "ZAxisMaxHeight",
        z_axis_max_height,
        4
    ),
    f!(LightSurfacePassParams, "InstanceOffset", instance_offset, 1),
    f!(LightSurfacePassParams, "AtlasRows", mask_base, 1),
];
const ROW_BUILD: Layout = &[
    f!(RowBuildParams, "WorldFromMask", world_from_mask, 4),
    f!(RowBuildParams, "Domain", domain, 4),
];
const ROW_EDGE: Layout = &[
    f!(RowEdgeParams, "RowSize", row_size, 4),
    f!(RowEdgeParams, "Near", near, 4),
];
const ROW_LOOKUP: Layout = &[
    f!(RowLookupParams, "RowSize", row_size, 4),
    f!(RowLookupParams, "Domain", domain, 4),
    f!(RowLookupParams, "WorldFromMask", world_from_mask, 4),
];
/// Chiaro's pattern records and block, which the row programs read too.
const PATTERN_PARAMS: Layout = &[
    f!(PatternParams, "PatternMaskToWorldX", mask_to_world_x, 4),
    f!(PatternParams, "PatternMaskToWorldY", mask_to_world_y, 4),
    f!(PatternParams, "PatternAtlasSize", atlas_size, 4),
];
const PATTERN: Layout = &[
    f!(GpuPattern, "rect", rect, 4),
    f!(GpuPattern, "mean", mean, 4),
    f!(GpuPattern, "repeat", repeat, 4),
];
const PATTERN_OBSTACLE: Layout = &[
    f!(GpuPatternObstacle, "kind", kind, 4),
    f!(GpuPatternObstacle, "transform", transform, 4),
    f!(GpuPatternObstacle, "shape", shape, 4),
    f!(GpuPatternObstacle, "heights", heights, 4),
    f!(GpuPatternObstacle, "data", data, 4),
    f!(GpuPatternObstacle, "table", table, 4),
];
/// The row programs (`rows::gpu`), with the reference program of the tests.
const ROW_SHADERS: [&str; 12] = [
    "lightrow.vert",
    "lightrow_mask.vert",
    "lightrow_build.frag",
    "lightrow_edge.frag",
    "lightrow_light.frag",
    "lightrow_light_exact.frag",
    "lightrow_light_direction.frag",
    "lightrow_light_mask.frag",
    "lightrow_light_exact_mask.frag",
    "lightrow_light_direction_mask.frag",
    "lightrow_pattern.frag",
    "lightrow_truth.frag",
];

/// SPIR-V struct name, Rust layout, Rust size in bytes.
const STRUCTS: [(&str, Layout, usize); 15] = [
    ("RowLamp", ROW_LAMP, size_of::<RowLamp>()),
    (
        "type.LightRowPass",
        ROW_PASS,
        size_of::<LightSurfacePassParams>(),
    ),
    ("type.RowParams", ROW_BUILD, size_of::<RowBuildParams>()),
    ("type.EdgeParams", ROW_EDGE, size_of::<RowEdgeParams>()),
    (
        "type.LookupParams",
        ROW_LOOKUP,
        size_of::<RowLookupParams>(),
    ),
    ("GpuLight", GPU_LIGHT, size_of::<GpuLight>()),
    ("GpuMask", GPU_MASK, size_of::<GpuMask>()),
    (
        "type.LightPassParams",
        LIGHT_PASS,
        size_of::<LightPassParams>(),
    ),
    (
        "type.LightSurfacePassParams",
        SURFACE_PASS,
        size_of::<LightSurfacePassParams>(),
    ),
    ("type.SurfaceParams", SURFACE, size_of::<SurfaceParams>()),
    (
        "type.ShadowParams",
        SHADOW,
        size_of::<SurfaceShadowParams>(),
    ),
    (
        "type.DirectionalParams",
        DIRECTIONAL,
        size_of::<DirectionalParams>(),
    ),
    (
        "type.PatternParams",
        PATTERN_PARAMS,
        size_of::<PatternParams>(),
    ),
    ("GpuPattern", PATTERN, size_of::<GpuPattern>()),
    (
        "GpuPatternObstacle",
        PATTERN_OBSTACLE,
        size_of::<GpuPatternObstacle>(),
    ),
];

#[test]
fn rust_layouts_match_every_light_shader() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    let mut shaders: Vec<&str> = LIGHT_PROGRAMS
        .iter()
        .flat_map(|p| [p.vertex, p.fragment])
        .chain(ROW_SHADERS)
        .collect();
    shaders.sort_unstable();
    shaders.dedup();
    let mut seen = [0usize; STRUCTS.len()];
    let mut members_seen: Vec<(&str, &str)> = Vec::new();
    for shader in &shaders {
        let program = pack.get(shader).unwrap();
        for (name, members) in decode(program.code).structs {
            let Some(k) = STRUCTS.iter().position(|s| s.0 == name) else {
                continue;
            };
            let (_, layout, size) = STRUCTS[k];
            seen[k] += 1;
            let mut end = 0;
            for (member, offset) in &members {
                if member == "Pad" {
                    continue;
                }
                let Some(&(_, rust, len)) = layout.iter().find(|m| m.0 == member) else {
                    panic!("{shader}: {name}.{member} has no Rust field");
                };
                assert_eq!(
                    *offset as usize, rust,
                    "{shader}: {name}.{member} at {offset}, Rust at {rust}"
                );
                end = end.max(rust + len);
                members_seen.push((
                    STRUCTS[k].0,
                    layout.iter().find(|m| m.0 == member).unwrap().0,
                ));
            }
            assert!(
                end <= size,
                "{shader}: {name} reaches {end} bytes, Rust has {size}"
            );
        }
    }
    for (k, (name, layout, _)) in STRUCTS.iter().enumerate() {
        assert!(seen[k] > 0, "no light shader declares {name}");
        for (member, _, _) in layout.iter() {
            assert!(
                members_seen.contains(&(name, member)),
                "no light shader reads {name}.{member}"
            );
        }
    }
    eprintln!(
        "{} light shaders, struct uses: {:?}",
        shaders.len(),
        STRUCTS
            .iter()
            .zip(seen)
            .map(|(s, n)| (s.0, n))
            .collect::<Vec<_>>()
    );
}
