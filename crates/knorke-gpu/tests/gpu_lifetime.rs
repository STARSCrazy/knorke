//! Lifetime and liveness tests against a real SDL_GPU device.
//!
//! Every test skips (passes with a note on stderr) when no GPU device can be created, so
//! CI machines without GPU or driver stay green. The test plays the host: it owns device
//! and command buffer and submits; the crate under test never does.
#![cfg(feature = "sdl")]
#![allow(unsafe_code)]

use std::ptr::NonNull;

use knorke_gpu::sys::gpu::{
    SDL_AcquireGPUCommandBuffer, SDL_BeginGPUCopyPass, SDL_EndGPUCopyPass,
    SDL_GPU_BUFFERUSAGE_VERTEX, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPUBufferUsageFlags, SDL_SubmitGPUCommandBuffer,
    SDL_WaitForGPUIdle,
};
use knorke_gpu::test_support::any_device;
use knorke_gpu::{
    Device, Filter, Gpu, LiveCounter, RENDER_TARGET_ZERO_SIZE, RenderTarget, Samplers,
    UPLOAD_LIMIT, UPLOAD_NOT_RESERVED, UploadBuffer, global_live_gpu_objects,
};

#[test]
fn render_target_is_persistent_and_counts_recreation() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let mut rt = RenderTarget::new(Gpu::new(&device, &counter));
        assert_eq!(counter.live(), 0, "no allocation before ensure");
        assert!(rt.texture().is_none());

        assert!(
            rt.ensure(64, 32, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
                .unwrap()
        );
        assert_eq!((rt.width(), rt.height(), rt.recreated()), (64, 32, 1));
        assert_eq!(counter.live(), 1);
        let first = rt.texture().unwrap();

        // Same request: no reallocation, same texture.
        assert!(
            !rt.ensure(64, 32, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
                .unwrap()
        );
        assert_eq!(rt.recreated(), 1);
        assert_eq!(rt.texture(), Some(first));

        // New size and new format each reallocate; the old texture is released.
        assert!(
            rt.ensure(128, 32, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
                .unwrap()
        );
        assert!(
            rt.ensure(128, 32, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT)
                .unwrap()
        );
        assert_eq!(rt.recreated(), 3);
        assert_eq!(counter.live(), 1);

        // Zero size fails with a stable code and keeps the old texture.
        let err = rt
            .ensure(0, 32, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
            .unwrap_err();
        assert_eq!(err.code, RENDER_TARGET_ZERO_SIZE);
        assert_eq!((rt.width(), rt.height()), (128, 32));
        assert_eq!(counter.live(), 1);

        rt.release();
        assert_eq!(counter.live(), 0);
        assert!(rt.texture().is_none());
        assert!(
            rt.ensure(8, 8, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM)
                .unwrap()
        );
        assert_eq!(counter.live(), 1);
    }
    assert_eq!(counter.live(), 0, "drop releases the texture");
}

#[test]
fn samplers_own_two_objects() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let samplers = Samplers::new(Gpu::new(&device, &counter)).unwrap();
        assert_eq!(counter.live(), 2);
        assert!(global_live_gpu_objects() >= 2);
        assert_ne!(samplers.get(Filter::Nearest), samplers.get(Filter::Linear));
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn upload_buffer_grows_maps_and_uploads() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut up = UploadBuffer::new(gpu, SDL_GPU_BUFFERUSAGE_VERTEX, 1024, "test vertices");
        assert_eq!(up.map().unwrap_err().code, UPLOAD_NOT_RESERVED);
        assert_eq!(counter.live(), 0);

        up.reserve(100).unwrap();
        assert_eq!(up.capacity(), 1024);
        assert_eq!(counter.live(), 2, "transfer + GPU buffer");
        let old = up.transfer();

        up.reserve(1024).unwrap();
        assert_eq!(up.transfer(), old, "fits: no reallocation");

        up.reserve(3000).unwrap();
        assert_eq!(up.capacity(), 4096, "doubles from 1024");
        assert_eq!(counter.live(), 2, "old pair released");

        let err = up.reserve(u64::from(u32::MAX) + 1).unwrap_err();
        assert_eq!(err.code, UPLOAD_LIMIT);
        assert_eq!(up.capacity(), 4096, "failure keeps the old buffers");

        {
            let mut mapped = up.map().unwrap();
            let bytes = mapped.bytes();
            assert_eq!(bytes.len(), 4096);
            bytes[..4].copy_from_slice(&[1, 2, 3, 4]);
        } // unmapped here

        // Play the host: command buffer, copy pass, submit.
        // SAFETY: device is live; the command buffer and copy pass are used and ended here.
        unsafe {
            let cmd = SDL_AcquireGPUCommandBuffer(device.as_ptr());
            assert!(!cmd.is_null());
            let copy = NonNull::new(SDL_BeginGPUCopyPass(cmd)).unwrap();
            up.upload(copy, 4);
            SDL_EndGPUCopyPass(copy.as_ptr());
            assert!(SDL_SubmitGPUCommandBuffer(cmd));
            assert!(SDL_WaitForGPUIdle(device.as_ptr()));
        }

        up.reset();
        assert_eq!(counter.live(), 0);
        assert_eq!(up.capacity(), 0);
    }
    assert_eq!(counter.live(), 0);
}

#[test]
fn transfer_only_upload_buffer_owns_one_object() {
    let Some(device) = any_device() else { return };
    let counter = LiveCounter::new();
    {
        let mut up = UploadBuffer::new(
            Gpu::new(&device, &counter),
            SDL_GPUBufferUsageFlags(0),
            256,
            "texture staging",
        );
        up.reserve(10).unwrap();
        assert_eq!(counter.live(), 1);
        assert!(up.buffer().is_none());
        assert!(up.transfer().is_some());
    }
    assert_eq!(counter.live(), 0);
}
