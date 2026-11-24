//! The uniform blocks and storage records of the light pass (P25) against the compiled shaders:
//! for every shader of the 92 light programs in the SPIR-V pack, the member offsets the
//! compiler gave each `cbuffer` and `StructuredBuffer` element (`OpMemberDecorate Offset`)
//! must equal the offsets of the Rust structs, member by member. A slot or field written at the
//! wrong place gives silent image errors; this catches it without a GPU.
//!
//! Needs the pack from `cargo xtask shaders` in `target/shaders/` (Chiaro 9bc5b9b shaders,
//! zlib license); skips without it.
#![cfg(feature = "gpu")]

use core::mem::{offset_of, size_of};
use std::path::Path;

use knorke_gpu::kpak::ShaderPack;
use knorke_light2d::gpu_light::{GpuDirectional, GpuLight, GpuMask};
use knorke_light2d::light_program::LIGHT_PROGRAMS;
use knorke_light2d::lighting::{
    DirectionalParams, LightPassParams, LightSurfacePassParams, SurfaceParams, SurfaceShadowParams,
};

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

/// SPIR-V struct name, Rust layout, Rust size in bytes.
const STRUCTS: [(&str, Layout, usize); 7] = [
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
];

/// Member offsets of every named struct of a SPIR-V module: `(struct, [(member, offset)])`.
fn struct_offsets(code: &[u8]) -> Vec<(String, Vec<(String, u32)>)> {
    let words: Vec<u32> = code
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    assert_eq!(words[0], 0x0723_0203, "not SPIR-V");
    let string = |ws: &[u32]| {
        let bytes: Vec<u8> = ws.iter().flat_map(|w| w.to_le_bytes()).collect();
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec()).unwrap()
    };
    let mut names = Vec::new();
    let mut members = Vec::new();
    let mut offsets = Vec::new();
    let mut i = 5;
    while i < words.len() {
        let (n, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(n > 0, "malformed SPIR-V");
        let a = &words[i + 1..i + n];
        match op {
            5 => names.push((a[0], string(&a[1..]))),
            6 => members.push((a[0], a[1], string(&a[2..]))),
            72 if a[2] == 35 => offsets.push((a[0], a[1], a[3])),
            _ => {}
        }
        i += n;
    }
    names
        .into_iter()
        .map(|(id, name)| {
            let mut m: Vec<(u32, String, u32)> = offsets
                .iter()
                .filter(|o| o.0 == id)
                .map(|&(_, member, off)| {
                    let mname = members
                        .iter()
                        .find(|x| x.0 == id && x.1 == member)
                        .map_or_else(String::new, |x| x.2.clone());
                    (member, mname, off)
                })
                .collect();
            m.sort_by_key(|x| x.0);
            (name, m.into_iter().map(|(_, n, o)| (n, o)).collect())
        })
        .collect()
}

#[test]
fn rust_layouts_match_every_light_shader() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shaders/knorke-chiaro.spirv.kpak");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipped: no shader pack at {} (cargo xtask shaders)",
            path.display()
        );
        return;
    };
    let pack = ShaderPack::parse(&bytes).unwrap();
    let mut shaders: Vec<&str> = LIGHT_PROGRAMS
        .iter()
        .flat_map(|p| [p.vertex, p.fragment])
        .collect();
    shaders.sort_unstable();
    shaders.dedup();
    let mut seen = [0usize; STRUCTS.len()];
    let mut members_seen: Vec<(&str, &str)> = Vec::new();
    for shader in &shaders {
        let program = pack.get(shader).unwrap();
        for (name, members) in struct_offsets(program.code) {
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
