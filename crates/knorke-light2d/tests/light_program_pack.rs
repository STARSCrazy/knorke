//! The light program family against Chiaro's compiled shaders (port of
//! `tests/unit/LightProgramLayoutTests.cpp`, light part): every program's shaders exist in
//! the pack, their reflection counts match [`light_layout_of`], and the SPIR-V descriptor set
//! and binding of every named resource is the slot the layout gives. Then all 92 pipelines
//! are created on a real device.
//!
//! Needs the full pack `target/shaders/knorke-chiaro.spirv.kpak`
//! (`cargo xtask shaders --src <chiaro>/shaders/compiled --formats spirv`); skips (passes)
//! without it, and the pipeline test also without a Vulkan device.

#![cfg(feature = "gpu")]

mod common;

use common::check_layout;
use knorke_gpu::kpak::ShaderPack;
use knorke_gpu::sys::gpu::SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
use knorke_gpu::test_support::{chiaro_pack, with_chiaro_pack};
use knorke_gpu::ShaderLibrary;
use knorke_light2d::light_pipelines::{LIGHT_PROGRAM_UNKNOWN, LightPipelines};
use knorke_light2d::light_program::{
    LIGHT_PROGRAMS, LightBase, LightFeature, LightProgram, light_layout_of,
};

#[test]
fn every_light_program_layout_matches_its_shaders() {
    let Some(bytes) = chiaro_pack() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for info in &LIGHT_PROGRAMS {
        let frag = pack.get(info.fragment).unwrap();
        let vert = pack.get(info.vertex).unwrap();
        check_layout(&frag, &vert, &light_layout_of(info.program()));
    }
}

#[test]
fn all_light_pipelines_build_lazily() {
    with_chiaro_pack(|gpu, bytes| {
        let mut shaders = ShaderLibrary::new(gpu, bytes.to_vec()).unwrap();
        let mut pipelines = LightPipelines::new(&shaders).unwrap();
        assert!(pipelines.is_empty(), "lazy: nothing created up front");

        let target = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
        for info in &LIGHT_PROGRAMS {
            let p = info.program();
            assert!(pipelines.pipeline(p, target).is_none());
            pipelines.prepare(&mut shaders, p, target).unwrap();
            assert!(pipelines.contains(p));
            assert!(pipelines.pipeline(p, target).is_some());
        }
        assert_eq!(pipelines.len(), 92);
        // Preparing again reuses the pipeline.
        pipelines
            .prepare(&mut shaders, LIGHT_PROGRAMS[0].program(), target)
            .unwrap();
        assert_eq!(pipelines.len(), 92);

        let invalid = LightProgram::new(LightBase::Light).with(LightFeature::Exact, true);
        let err = pipelines
            .prepare(&mut shaders, invalid, target)
            .unwrap_err();
        assert_eq!(err.code, LIGHT_PROGRAM_UNKNOWN);
        assert!(!pipelines.contains(invalid));
    });
}
