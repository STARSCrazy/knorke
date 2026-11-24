//! The composite pass (P27) against Chiaro's compiled shaders (port of the composite part of
//! `tests/unit/LightProgramLayoutTests.cpp`, E160), for each of the six composite programs in the
//! SPIR-V pack:
//!
//! - the reflection counts and the descriptor set and binding of every named resource are the
//!   slots [`composite_layout_of`] gives;
//! - the member offsets the compiler gave `cbuffer CompositeParams` (`OpMemberDecorate Offset`)
//!   equal the offsets of [`CompositeParams`], member by member (a field written at the wrong
//!   place gives silent image errors);
//! - all six pipelines build lazily on a real device.
//!
//! Needs the pack from `cargo xtask shaders` in `target/shaders/` (Chiaro 9bc5b9b shaders, zlib
//! license); skips without it, the pipeline test also without a Vulkan device.
#![cfg(feature = "gpu")]

use core::mem::{offset_of, size_of};
mod common;

use common::check_layout;
use knorke_gpu::ShaderLibrary;
use knorke_gpu::kpak::ShaderPack;
use knorke_gpu::test_support::spirv::decode;
use knorke_gpu::test_support::{chiaro_pack, with_chiaro_pack};
use knorke_light2d::composite::{
    COMPOSITE_PROGRAMS, CompositeParams, CompositeStage, composite_layout_of,
};

#[test]
fn every_composite_program_layout_matches_its_shaders() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for program in COMPOSITE_PROGRAMS {
        let frag = pack.get(program.fragment()).unwrap();
        let vert = pack.get(program.vertex()).unwrap();
        check_layout(&frag, &vert, &composite_layout_of(program));
    }
}

/// `(HLSL member, Rust offset, Rust size)` of `CompositeParams`.
const MEMBERS: [(&str, usize, usize); 16] = [
    ("DecodeSrgb", offset_of!(CompositeParams, decode_srgb), 4),
    (
        "UseLightBuffer",
        offset_of!(CompositeParams, use_light_buffer),
        4,
    ),
    ("UseGlow", offset_of!(CompositeParams, use_glow), 4),
    ("DecodeGlow", offset_of!(CompositeParams, decode_glow), 4),
    ("Ambient", offset_of!(CompositeParams, ambient), 16),
    (
        "GlowFromScene",
        offset_of!(CompositeParams, glow_from_scene),
        4,
    ),
    ("GlowScale", offset_of!(CompositeParams, glow_scale), 4),
    ("UseSpecular", offset_of!(CompositeParams, use_specular), 4),
    (
        "UseDirection",
        offset_of!(CompositeParams, use_direction),
        4,
    ),
    (
        "ViewDirSpec",
        offset_of!(CompositeParams, view_dir_spec),
        16,
    ),
    ("ScreenRight", offset_of!(CompositeParams, screen_right), 16),
    ("XYAxes", offset_of!(CompositeParams, xy_axes), 16),
    ("ZAxisScene", offset_of!(CompositeParams, z_axis_scene), 16),
    ("InvAxes", offset_of!(CompositeParams, inv_axes), 16),
    ("OriginStep", offset_of!(CompositeParams, origin_step), 16),
    ("Wet", offset_of!(CompositeParams, wet), 16),
];

#[test]
fn rust_layout_matches_every_composite_shader() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    assert_eq!(size_of::<CompositeParams>(), 160);
    for program in COMPOSITE_PROGRAMS {
        let shader = program.fragment();
        let module = decode(pack.get(shader).unwrap().code);
        let (_, members) = module
            .structs
            .iter()
            .find(|s| s.0 == "type.CompositeParams")
            .unwrap_or_else(|| panic!("{shader}: no CompositeParams"));
        // The compiler keeps every member, used or not.
        assert_eq!(members.len(), MEMBERS.len(), "{shader}");
        for (member, offset) in members {
            let &(_, rust, len) = MEMBERS
                .iter()
                .find(|m| m.0 == member)
                .unwrap_or_else(|| panic!("{shader}: CompositeParams.{member} has no Rust field"));
            assert_eq!(
                *offset as usize, rust,
                "{shader}: CompositeParams.{member} at {offset}, Rust at {rust}"
            );
            assert!(rust + len <= size_of::<CompositeParams>());
        }
    }
}

#[test]
fn all_composite_pipelines_build_lazily() {
    with_chiaro_pack(|gpu, bytes| {
        let mut shaders = ShaderLibrary::new(gpu, bytes.to_vec()).unwrap();
        // The stage resolves all six programs up front and creates none of the pipelines.
        let stage = CompositeStage::new(gpu, &shaders).unwrap();
        assert!(stage.hdr_buffer().is_none());
        assert_eq!(stage.pipeline_count(), 0, "lazy: nothing created up front");
        for (n, program) in COMPOSITE_PROGRAMS.iter().enumerate() {
            let mut stage = CompositeStage::new(gpu, &shaders).unwrap();
            stage
                .prepare_pipeline(&mut shaders, *program)
                .unwrap_or_else(|e| panic!("{program:?}: {}", e.message));
            assert_eq!(stage.pipeline_count(), 1, "{program:?} (#{n})");
        }
    });
}
