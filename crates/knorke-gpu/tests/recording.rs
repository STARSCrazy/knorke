//! The safe recording API (P1b) against a real SDL_GPU device, with the committed test
//! packs. Every test skips when no GPU device can be created. No `unsafe`: the host side
//! runs on `CommandBuffer`, `Texture`, `UploadBuffer` and `DownloadBuffer` as well.
#![cfg(feature = "sdl")]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, Once};

use knorke_gpu::sys::gpu::{
    SDL_GPU_SHADERFORMAT_DXIL, SDL_GPU_SHADERFORMAT_MSL, SDL_GPU_SHADERFORMAT_SPIRV,
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT,
    SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    BINDING_MISMATCH, Bindings, Blend, ColorTarget, CommandBuffer, Device, DownloadBuffer, Filter,
    Gpu, LiveCounter, Load, PipelineCache, ProgramDesc, RECORD_NOT_BOUND, RECORD_OUT_OF_RANGE,
    RECORD_TARGET_MISMATCH, RECORD_USAGE, SamplerBinding, Samplers, ShaderLibrary, StageBindings,
    Texture, TextureDesc, UploadBuffer, device_formats, init_video, preferred_format,
};

static SERIAL: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();

fn device() -> Option<(MutexGuard<'static, ()>, Device)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    INIT.call_once(|| {
        init_video(None);
    });
    let formats = SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL;
    match Device::create(formats, false) {
        Ok(d) => Some((guard, d)),
        Err(e) => {
            eprintln!("skipped: no GPU device available ({})", e.message);
            None
        }
    }
}

const RGBA8: knorke_gpu::sys::gpu::SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

fn upload(gpu: Gpu<'_>, texture: &Texture<'_>, pixels: &[u8]) {
    let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), 256, "pixels");
    up.reserve(pixels.len() as u64).unwrap();
    up.map().unwrap().bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .upload_to_texture(&up, texture.texture_ref(), false)
        .unwrap();
    cmd.submit_and_wait().unwrap();
}

fn download(gpu: Gpu<'_>, texture: &Texture<'_>) -> Vec<u8> {
    let bytes = texture.desc().bytes() as u32;
    let mut down = DownloadBuffer::new(gpu, bytes).unwrap();
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .download_texture(texture.texture_ref(), &down)
        .unwrap();
    cmd.submit_and_wait().unwrap();
    let mut out = vec![0; bytes as usize];
    down.read(&mut out).unwrap();
    out
}

#[test]
fn upload_and_download_round_trip() {
    let Some((_g, device)) = device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let desc = TextureDesc::new(4, 2, RGBA8, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        let texture = Texture::new(gpu, desc).unwrap();
        assert_eq!(desc.bytes(), 32);
        let pixels: Vec<u8> = (0..32).collect();
        upload(gpu, &texture, &pixels);
        assert_eq!(download(gpu, &texture), pixels);

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
fn render_pass_checks_pipeline_bindings_and_usage() {
    let Some((_g, device)) = device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let format = preferred_format(device_formats(gpu)).unwrap();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(format.pack_file_name("test"));
        let mut lib = ShaderLibrary::load(gpu, &path).unwrap();
        // jfa_seed.frag: one sampler, one uniform block; writes (uv of the seed or -1).
        let programs = [ProgramDesc::new(
            "fullscreen.vert",
            "jfa_seed.frag",
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
        upload(gpu, &input, &[0; 64]);

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
