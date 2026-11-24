//! ShaderLibrary and PipelineCache against a real SDL_GPU device, with the committed test
//! packs (`tests/data/test.*.kpak`). Every test skips when no GPU device can be created.
//! Ported in part from Chiaro tests/gpu/ShaderLibraryTests.cpp (revision 9bc5b9b).
#![cfg(feature = "sdl")]
#![allow(unsafe_code)]

use std::path::PathBuf;

use knorke_gpu::kpak::{
    SHADER_PACK_CHECKSUM, SHADER_PACK_IO, SHADER_PACK_MISSING_PROGRAM,
    SHADER_PACK_UNSUPPORTED_FORMAT, ShaderFormat,
};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
};
use knorke_gpu::test_support::any_device;
use knorke_gpu::{
    Blend, Gpu, LiveCounter, PIPELINE_STAGE_MISMATCH, PipelineCache, ProgramDesc, SecondTarget,
    ShaderLibrary, device_formats, preferred_format, sdl_shader_format,
};

fn pack_path(format: ShaderFormat) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(format.pack_file_name("test"))
}

/// The host's job: pick the pack for the device's backend.
fn library(gpu: Gpu<'_>) -> ShaderLibrary<'_> {
    let format = preferred_format(device_formats(gpu)).expect("device takes a known format");
    ShaderLibrary::load(gpu, &pack_path(format)).unwrap()
}

#[test]
fn shaders_are_created_lazily_and_released() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut lib = library(gpu);
        assert_eq!(lib.created_count(), 0, "nothing before first use");
        lib.require(&["fullscreen.vert", "clear.frag"]).unwrap();
        let a = lib.shader_by_name("fullscreen.vert").unwrap();
        let b = lib.shader_by_name("fullscreen.vert").unwrap();
        assert_eq!(a, b, "cached");
        lib.shader_by_name("clear.frag").unwrap();
        lib.shader_by_name("solid.frag").unwrap();
        lib.shader_by_name("jfa_seed.frag").unwrap();
        assert_eq!(lib.created_count(), 4);
        assert_eq!(counter.live(), 4);
        assert_eq!(
            lib.shader_by_name("missing.frag").unwrap_err().code,
            SHADER_PACK_MISSING_PROGRAM
        );
        assert_eq!(
            lib.require(&["clear.frag", "missing.frag"])
                .unwrap_err()
                .code,
            SHADER_PACK_MISSING_PROGRAM
        );
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn wrong_pack_or_bad_file_is_a_diagnostic() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let supported = device_formats(gpu);
    for format in ShaderFormat::ALL {
        if supported.0 & sdl_shader_format(format).0 == 0 {
            let e = ShaderLibrary::load(gpu, &pack_path(format)).unwrap_err();
            assert_eq!(e.code, SHADER_PACK_UNSUPPORTED_FORMAT);
        }
    }
    let e = ShaderLibrary::load(gpu, &pack_path(ShaderFormat::Spirv).with_extension("none"))
        .unwrap_err();
    assert_eq!(e.code, SHADER_PACK_IO);
    assert!(e.path.is_some());
    let format = preferred_format(supported).unwrap();
    let mut bytes = std::fs::read(pack_path(format)).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert_eq!(
        ShaderLibrary::new(gpu, bytes).unwrap_err().code,
        SHADER_PACK_CHECKSUM
    );
    assert_eq!(counter.live(), 0);
}

#[test]
fn pipeline_cache_keys_by_program_and_target_format() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut lib = library(gpu);
        let programs = [
            ProgramDesc::new("fullscreen.vert", "jfa_seed.frag", Blend::Opaque),
            ProgramDesc::new("fullscreen.vert", "solid.frag", Blend::Max),
            ProgramDesc {
                second: Some(SecondTarget {
                    format: SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
                    blend: Blend::Max,
                }),
                ..ProgramDesc::new("fullscreen.vert", "clear.frag", Blend::Additive)
            },
        ];
        let mut cache = PipelineCache::new(&lib, &programs).unwrap();
        assert!(cache.is_empty());
        let rgba16 = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
        let p0 = cache.get(&mut lib, 0, rgba16).unwrap();
        assert_eq!(cache.get(&mut lib, 0, rgba16).unwrap(), p0, "cached");
        let p0b = cache
            .get(&mut lib, 0, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
            .unwrap();
        assert_ne!(p0, p0b, "one pipeline per target format");
        cache.get(&mut lib, 1, rgba16).unwrap();
        cache.get(&mut lib, 2, rgba16).unwrap();
        assert_eq!(cache.len(), 4);
        assert!(cache.contains(2));
        assert!(cache.get(&mut lib, 3, rgba16).is_err());
        let (v, f) = cache.reflection(0).unwrap();
        assert_eq!((v.samplers, f.samplers, f.uniform_buffers), (0, 1, 1));
        // 4 pipelines + 4 shaders.
        assert_eq!(counter.live(), 8);

        let swapped = [ProgramDesc::new(
            "clear.frag",
            "fullscreen.vert",
            Blend::Opaque,
        )];
        assert_eq!(
            PipelineCache::new(&lib, &swapped).unwrap_err().code,
            PIPELINE_STAGE_MISMATCH
        );
        let missing = [ProgramDesc::new(
            "fullscreen.vert",
            "nope.frag",
            Blend::Opaque,
        )];
        assert_eq!(
            PipelineCache::new(&lib, &missing).unwrap_err().code,
            SHADER_PACK_MISSING_PROGRAM
        );
        drop(cache);
        assert_eq!(counter.live(), 4);
    }
    assert_eq!(counter.live(), 0);
}
