//! Helpers of the Chiaro demo scene tests (`gpu_wet_street.rs`, `gpu_iso_night.rs`): layer
//! fixtures dumped by their C++ reference programs, and diff images for `KNORKE_SCENE_OUT`.
//! Included with `#[path]`, because `common/mod.rs` needs the `pixel` feature.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use knorke_contract_tests::parity::{load_png_rgba8, save_png_rgba8};
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

/// Max channel difference per pixel x32, red where it exceeds the STRICT tolerance.
pub(crate) fn diff_image(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .flat_map(|(a, b)| {
            let m = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            let v = m.saturating_mul(32);
            let g = if m > 2 { 0 } else { v };
            [v, g, g, 255]
        })
        .collect()
}

/// The directory of `KNORKE_SCENE_OUT`, if set.
pub(crate) fn scene_out() -> Option<PathBuf> {
    std::env::var("KNORKE_SCENE_OUT").ok().map(PathBuf::from)
}

/// Writes `<name>.rust.png` and `<name>.diff.png` (against `reference`) of a `w` x `h` image.
pub(crate) fn save_with_diff(
    dir: &Path,
    name: &str,
    pixels: &[u8],
    reference: &[u8],
    (w, h): (u32, u32),
) {
    std::fs::create_dir_all(dir).unwrap();
    save_png_rgba8(&dir.join(format!("{name}.rust.png")), pixels, w, h);
    save_png_rgba8(
        &dir.join(format!("{name}.diff.png")),
        &diff_image(pixels, reference),
        w,
        h,
    );
}
