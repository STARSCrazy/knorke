//! The safe recording API (P1b) against a real SDL_GPU device, with the committed test
//! packs. Every test skips when no GPU device can be created. No `unsafe`: the host side
//! runs on `CommandBuffer`, `Texture`, `UploadBuffer` and `DownloadBuffer` as well.
#![cfg(feature = "sdl")]

use std::path::PathBuf;

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
    SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags,
};
use knorke_gpu::test_support::{any_device, read_texture, upload_pixels};
use knorke_gpu::{
    BINDING_MISMATCH, Bindings, Blend, ColorTarget, CommandBuffer, DownloadBuffer, Filter, Gpu,
    LiveCounter, Load, PipelineCache, ProgramDesc, RECORD_NOT_BOUND, RECORD_OUT_OF_RANGE,
    RECORD_TARGET_MISMATCH, RECORD_USAGE, SamplerBinding, Samplers, ShaderLibrary, StageBindings,
    Texture, TextureDesc, TextureRegion, UploadBuffer, device_formats, preferred_format,
};

const RGBA8: knorke_gpu::sys::gpu::SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

#[test]
fn upload_and_download_round_trip() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let desc = TextureDesc::new(4, 2, RGBA8, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        let texture = Texture::new(gpu, desc).unwrap();
        assert_eq!(desc.bytes(), 32);
        let pixels: Vec<u8> = (0..32).collect();
        upload_pixels(gpu, &texture, &pixels);
        assert_eq!(read_texture(gpu, texture.texture_ref()), pixels);

        // Too small a download buffer is refused before anything is recorded.
        let small = DownloadBuffer::new(gpu, 16).unwrap();
        let mut cmd = CommandBuffer::acquire(&device).unwrap();
        let e = cmd
            .copy_pass()
            .unwrap()
            .download_texture(texture.texture_ref(), &small)
            .unwrap_err();
        assert_eq!(e.code, RECORD_OUT_OF_RANGE);
        // Dropped without submit: cancelled.
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn partial_texture_upload_writes_only_its_region() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let desc = TextureDesc::new(4, 4, RGBA8, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        let texture = Texture::new(gpu, desc).unwrap();
        upload_pixels(gpu, &texture, &[7; 64]);

        // 2x2 texels staged at byte offset 512 (the alignment D3D12 wants), into (1, 2).
        let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), 1024, "cells");
        up.reserve(1024).unwrap();
        let patch: Vec<u8> = (100..116).collect();
        up.map().unwrap().bytes()[512..528].copy_from_slice(&patch);
        let region = TextureRegion {
            x: 1,
            y: 2,
            width: 2,
            height: 2,
        };
        let mut cmd = CommandBuffer::acquire(&device).unwrap();
        {
            let mut copy = cmd.copy_pass().unwrap();
            // Outside the texture, or beyond the staged bytes: refused, nothing recorded.
            let outside = TextureRegion { x: 3, ..region };
            let e = copy
                .upload_to_texture_region(&up, 512, texture.texture_ref(), outside, false)
                .unwrap_err();
            assert_eq!(e.code, RECORD_OUT_OF_RANGE);
            let e = copy
                .upload_to_texture_region(&up, 1020, texture.texture_ref(), region, false)
                .unwrap_err();
            assert_eq!(e.code, RECORD_OUT_OF_RANGE);
            copy.upload_to_texture_region(&up, 512, texture.texture_ref(), region, false)
                .unwrap();
        }
        cmd.submit_and_wait().unwrap();

        let got = read_texture(gpu, texture.texture_ref());
        let mut want = vec![7u8; 64];
        for row in 0..2usize {
            let dst = ((2 + row) * 4 + 1) * 4;
            want[dst..dst + 8].copy_from_slice(&patch[row * 8..row * 8 + 8]);
        }
        assert_eq!(got, want);
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn texture_to_texture_copy_moves_a_region_between_textures() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let desc = TextureDesc::new(4, 4, RGBA8, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        let src = Texture::new(gpu, desc).unwrap();
        let dst = Texture::new(gpu, desc).unwrap();
        let pixels: Vec<u8> = (0..64).collect();
        upload_pixels(gpu, &src, &pixels);
        upload_pixels(gpu, &dst, &[9; 64]);
        let other = Texture::new(
            gpu,
            TextureDesc::new(4, 4, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, desc.usage),
        )
        .unwrap();

        let mut cmd = CommandBuffer::acquire(&device).unwrap();
        {
            let mut copy = cmd.copy_pass().unwrap();
            // Leaving either texture, or another format: refused, nothing recorded.
            let e = copy
                .copy_texture_region(
                    src.texture_ref(),
                    [2, 0],
                    dst.texture_ref(),
                    [0, 0],
                    [3, 1],
                    false,
                )
                .unwrap_err();
            assert_eq!(e.code, RECORD_OUT_OF_RANGE);
            let e = copy
                .copy_texture_region(
                    src.texture_ref(),
                    [0, 0],
                    dst.texture_ref(),
                    [0, 3],
                    [1, 2],
                    false,
                )
                .unwrap_err();
            assert_eq!(e.code, RECORD_OUT_OF_RANGE);
            let e = copy
                .copy_texture_region(
                    src.texture_ref(),
                    [0, 0],
                    other.texture_ref(),
                    [0, 0],
                    [1, 1],
                    false,
                )
                .unwrap_err();
            assert_eq!(e.code, RECORD_USAGE);
            // Texels (0..3, 1..3) of src to (1, 0) of dst.
            copy.copy_texture_region(
                src.texture_ref(),
                [0, 1],
                dst.texture_ref(),
                [1, 0],
                [3, 2],
                false,
            )
            .unwrap();
        }
        cmd.submit_and_wait().unwrap();

        let got = read_texture(gpu, dst.texture_ref());
        let mut want = vec![9u8; 64];
        for row in 0..2usize {
            for col in 0..3usize {
                let s = ((row + 1) * 4 + col) * 4;
                let d = (row * 4 + col + 1) * 4;
                want[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
            }
        }
        assert_eq!(got, want);
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn render_pass_checks_pipeline_bindings_and_usage() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let format = preferred_format(device_formats(gpu)).unwrap();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(format.pack_file_name("test"));
        let mut lib = ShaderLibrary::load(gpu, &path).unwrap();
        // occlusion_mask.frag: one sampler, one uniform block of 32 bytes.
        let programs = [ProgramDesc::new(
            "fullscreen.vert",
            "occlusion_mask.frag",
            Blend::Opaque,
        )];
        let mut cache = PipelineCache::new(&lib, &programs).unwrap();
        cache.get(&mut lib, 0, RGBA8).unwrap();
        cache
            .get(&mut lib, 0, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT)
            .unwrap();
        assert!(cache.pipeline(0, RGBA8).is_some());
        assert!(cache.pipeline(1, RGBA8).is_none());
        let samplers = Samplers::new(gpu).unwrap();

        let both = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
        let target = Texture::new(gpu, TextureDesc::new(4, 4, RGBA8, both)).unwrap();
        let input = Texture::new(
            gpu,
            TextureDesc::new(4, 4, RGBA8, SDL_GPU_TEXTUREUSAGE_SAMPLER),
        )
        .unwrap();
        let target_only = Texture::new(
            gpu,
            TextureDesc::new(4, 4, RGBA8, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET),
        )
        .unwrap();
        upload_pixels(gpu, &input, &[0; 64]);

        let mut cmd = CommandBuffer::acquire(&device).unwrap();
        // A color target needs COLOR_TARGET usage.
        let e = cmd
            .render_pass(&[ColorTarget::new(input.texture_ref(), Load::DontCare)])
            .unwrap_err();
        assert_eq!(e.code, RECORD_USAGE);
        {
            let t = [ColorTarget::new(
                target.texture_ref(),
                Load::Clear([0.0; 4]),
            )];
            let mut pass = cmd.render_pass(&t).unwrap();
            assert_eq!(pass.draw_fullscreen().unwrap_err().code, RECORD_NOT_BOUND);
            let wrong = cache
                .pipeline(0, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT)
                .unwrap();
            assert_eq!(
                pass.bind_pipeline(wrong).unwrap_err().code,
                RECORD_TARGET_MISMATCH
            );
            let pipeline = cache.pipeline(0, RGBA8).unwrap();
            let e = pass
                .bind_program(pipeline, &Bindings::default())
                .unwrap_err();
            assert_eq!(e.code, BINDING_MISMATCH);
            // Pipeline alone: the draw still lacks the sampler and the uniform.
            pass.bind_pipeline(pipeline).unwrap();
            assert_eq!(pass.draw_fullscreen().unwrap_err().code, RECORD_NOT_BOUND);
            let nearest = samplers.sampler(Filter::Nearest);
            let bad = [SamplerBinding::new(target_only.texture_ref(), nearest)];
            assert_eq!(
                pass.bind_fragment_samplers(0, &bad).unwrap_err().code,
                RECORD_USAGE
            );

            let good = [SamplerBinding::new(input.texture_ref(), nearest)];
            let params = [0u8; 32];
            let uniforms: [&[u8]; 1] = [&params];
            let b = Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &good,
                    uniforms: &uniforms,
                    ..Default::default()
                },
            };
            pass.bind_program(pipeline, &b).unwrap();
            pass.draw_fullscreen().unwrap();
            // Typed uniform push: any Pod value.
            pass.push_fragment_uniform(0, &[0f32; 8]).unwrap();
            pass.draw_instanced(3, 1).unwrap();
        }
        cmd.submit_and_wait().unwrap();
    }
    assert_eq!(counter.live(), 0);
}
