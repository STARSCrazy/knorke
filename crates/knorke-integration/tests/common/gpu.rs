//! Helpers of the Chiaro demo scene tests (`gpu_wet_street.rs`, `gpu_iso_night.rs`): layer
//! fixtures dumped by their C++ reference programs. Included with `#[path]`, because
//! `common/mod.rs` needs the `pixel` feature.
#![allow(dead_code)]

use knorke_contract_tests::parity::{expected_png_rgba8, load_png_rgba8};
use knorke_gpu::sys::gpu::SDL_GPUBufferUsageFlags;
use knorke_gpu::test_support::workspace_root;
use knorke_gpu::{CommandBuffer, Gpu, UploadBuffer};
use knorke_integration::layers::{Layer, LayerSet, Layers};

/// The PNG fixture `tests/golden/chiaro/<dir>/<name>.png` of `w` x `h` pixels.
pub(crate) fn fixture(dir: &str, name: &str, (w, h): (u32, u32)) -> Vec<u8> {
    load_png_rgba8(
        &workspace_root().join(format!("tests/golden/chiaro/{dir}/{name}.png")),
        w,
        h,
    )
}

/// The golden picture `actual` must match: [`fixture`] `dir`/`name`, or its re-blessed rows
/// golden where the light rows draw the scene differently ([`expected_png_rgba8`], which writes
/// it with `KNORKE_BLESS=1`).
pub(crate) fn expected(dir: &str, name: &str, actual: &[u8], (w, h): (u32, u32)) -> Vec<u8> {
    let chiaro = workspace_root().join(format!("tests/golden/chiaro/{dir}/{name}.png"));
    expected_png_rgba8(&chiaro, actual, w, h)
}

/// Records copies of `fills` into their layers on `cmd` and marks them drawn. The returned
/// buffers hold the bytes until the command buffer has run: keep them until after the submit.
pub(crate) fn upload_layers<'d>(
    gpu: Gpu<'d>,
    cmd: &mut CommandBuffer<'_>,
    layers: &mut Layers<'_>,
    fills: &[(Layer, &[u8])],
) -> Vec<UploadBuffer<'d>> {
    let mut uploads: Vec<_> = fills
        .iter()
        .map(|(_, px)| {
            let mut up =
                UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), px.len() as u32, "fixture");
            up.reserve(px.len() as u64).unwrap();
            up.map().unwrap().bytes()[..px.len()].copy_from_slice(px);
            up
        })
        .collect();
    let mut copy = cmd.copy_pass().unwrap();
    let mut drawn = LayerSet::NONE;
    for (up, (layer, _)) in uploads.iter_mut().zip(fills) {
        copy.upload_to_texture(up, layers.texture(*layer).unwrap(), false)
            .unwrap();
        drawn = drawn.with(*layer);
    }
    drop(copy);
    layers.mark_drawn(drawn);
    uploads
}
