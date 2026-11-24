//! Helpers that every GPU test of the workspace shares (feature `sdl`). Not engine API: the
//! host owns device and frame in a real program (ADR 0002); tests play the host, and this
//! module is the one place where they do it the same way.
//!
//! - **One device at a time.** Drivers dislike several devices created in parallel, and the
//!   tests of one binary run on parallel threads. [`vulkan_device`] and [`any_device`] hand out
//!   a [`GpuTest`] that holds a process-wide lock until it is dropped; every GPU test goes
//!   through them (the lock is per test binary, binaries run one after the other).
//! - **Skip or fail.** A test without a device or without the shader pack passes with a note on
//!   stderr, unless `KNORKE_REQUIRE_GPU=1` is set: then the skip is a failure. Acceptance runs
//!   on a machine with a GPU set it, so a silent skip cannot hide a broken test.
//! - **Golden files and readbacks** (`tests/golden/chiaro/<dir>/<name>`, RGBA16F decoding,
//!   texture upload and download) so no test carries its own copy.

use std::ffi::CStr;
use std::fmt::Display;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use sdl3_sys::gpu::{
    SDL_GPU_SHADERFORMAT_DXIL, SDL_GPU_SHADERFORMAT_MSL, SDL_GPU_SHADERFORMAT_SPIRV,
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags,
    SDL_GPUTextureFormat, SDL_GPUTextureUsageFlags,
};

use crate::{
    CommandBuffer, Device, DownloadBuffer, Gpu, LiveCounter, Texture, TextureDesc, TextureRef,
    UploadBuffer, init_video,
};

static SERIAL: Mutex<()> = Mutex::new(());

/// A device together with the lock that keeps the other GPU tests of this binary waiting.
/// Derefs to the [`Device`]; the device drops before the lock is released.
pub struct GpuTest {
    device: Device,
    _serial: MutexGuard<'static, ()>,
}

impl Deref for GpuTest {
    type Target = Device;

    fn deref(&self) -> &Device {
        &self.device
    }
}

/// Whether a missing GPU or shader pack is an error (`KNORKE_REQUIRE_GPU=1`).
pub fn gpu_required() -> bool {
    std::env::var("KNORKE_REQUIRE_GPU").is_ok_and(|v| v == "1")
}

/// Ends a test early with a note, or fails it if [`gpu_required`]. Use as
/// `let Some(x) = ... else { return skip(...) }`, or via the functions of this module.
pub fn skip<T>(reason: impl Display) -> Option<T> {
    assert!(!gpu_required(), "KNORKE_REQUIRE_GPU=1 but: {reason}");
    eprintln!("skipped: {reason}");
    None
}

fn serial() -> MutexGuard<'static, ()> {
    // A poisoned lock only means another test panicked; the lock itself still works.
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn create(guard: MutexGuard<'static, ()>, vulkan: bool) -> Option<GpuTest> {
    // The SPIR-V-only path pins the driver, so a test never runs on a different backend than
    // the pack it loads; the any-device path takes whatever SDL offers.
    let (driver, formats): (Option<&CStr>, _) = if vulkan {
        (Some(c"vulkan"), SDL_GPU_SHADERFORMAT_SPIRV)
    } else {
        (
            None,
            SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL,
        )
    };
    init_video(driver);
    match Device::create(formats, false) {
        Ok(device) => Some(GpuTest {
            device,
            _serial: guard,
        }),
        Err(e) => skip(format_args!("no GPU device ({})", e.message)),
    }
}

/// A Vulkan device (SPIR-V), or `None` after a skip note.
pub fn vulkan_device() -> Option<GpuTest> {
    create(serial(), true)
}

/// Whatever device SDL offers (SPIR-V, DXIL or MSL), or `None` after a skip note.
pub fn any_device() -> Option<GpuTest> {
    create(serial(), false)
}

/// The root of the workspace (all test crates sit two levels below it).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The full Chiaro SPIR-V pack of `cargo xtask shaders` (`target/shaders/`, not committed), or
/// `None` after a skip note.
pub fn chiaro_pack() -> Option<Vec<u8>> {
    let path = workspace_root().join("target/shaders/knorke-chiaro.spirv.kpak");
    std::fs::read(&path).map_or_else(
        |_| {
            skip(format_args!(
                "no shader pack at {} (cargo xtask shaders)",
                path.display()
            ))
        },
        Some,
    )
}

/// A Vulkan device and the full Chiaro pack. The lock is taken before the pack is looked for,
/// so a missing pack never lets two tests race for the device.
pub fn vulkan_device_with_pack() -> Option<(GpuTest, Vec<u8>)> {
    let guard = serial();
    let pack = chiaro_pack()?;
    Some((create(guard, true)?, pack))
}

/// Runs `body` on a Vulkan device with a fresh [`LiveCounter`] and fails if any GPU object
/// outlives it. Skips like [`vulkan_device`].
pub fn with_vulkan(body: impl for<'d> FnOnce(Gpu<'d>)) {
    let Some(device) = vulkan_device() else {
        return;
    };
    run_counted(&device, body);
}

/// [`with_vulkan`] on the device with the full Chiaro pack, which `body` gets as bytes.
pub fn with_chiaro_pack(body: impl for<'d> FnOnce(Gpu<'d>, &[u8])) {
    let Some((device, pack)) = vulkan_device_with_pack() else {
        return;
    };
    run_counted(&device, |gpu| body(gpu, &pack));
}

fn run_counted(device: &Device, body: impl for<'d> FnOnce(Gpu<'d>)) {
    let counter = LiveCounter::new();
    body(Gpu::new(device, &counter));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}

// ---- Golden files -----------------------------------------------------------------------

/// A committed Chiaro dump: `tests/golden/chiaro/<dir>/<name>`. Panics if it is missing.
pub fn golden_bytes(dir: &str, name: &str) -> Vec<u8> {
    let path = workspace_root()
        .join("tests/golden/chiaro")
        .join(dir)
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// An IEEE half float as `f32` (the dumps and readbacks of `R16G16B16A16_FLOAT` targets);
/// infinities are kept, NaNs are not expected.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let (e, m) = (i32::from((h >> 10) & 0x1F), f32::from(h & 0x3FF));
    match e {
        0 => sign * m * f32::from_bits((127 - 24) << 23),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + m / 1024.0) * f32::from_bits(((e - 15 + 127) as u32) << 23),
    }
}

/// Differing RGBA16F texels of two readbacks and the largest channel difference.
pub fn rgba16f_diff(a: &[u8], b: &[u8]) -> (usize, f32) {
    assert_eq!(a.len(), b.len(), "RGBA16F readbacks differ in size");
    let channel =
        |texel: &[u8], c: usize| half_to_f32(u16::from_le_bytes([texel[c * 2], texel[c * 2 + 1]]));
    let mut differing = 0;
    let mut largest = 0.0f32;
    for (x, y) in a.as_chunks::<8>().0.iter().zip(b.as_chunks::<8>().0) {
        if x != y {
            differing += 1;
            for c in 0..4 {
                // Not `f32::max`: its result for -0.0 / +0.0 is not defined across builds.
                let d = (channel(x, c) - channel(y, c)).abs();
                if d > largest {
                    largest = d;
                }
            }
        }
    }
    (differing, largest)
}

/// Differing bytes of a readback against a dump and the largest byte difference; both are
/// reported on stderr so a run documents how close it came.
pub fn compare_bytes(name: &str, got: &[u8], want: &[u8]) -> (usize, u8) {
    assert_eq!(got.len(), want.len(), "{name}: size");
    let (count, max) = got.iter().zip(want).fold((0, 0), |(n, m), (a, b)| {
        let d = a.abs_diff(*b);
        (n + usize::from(d > 0), m.max(d))
    });
    eprintln!(
        "{name} vs C++ Chiaro: {count} of {} bytes differ, max difference {max}",
        got.len()
    );
    (count, max)
}

/// [`compare_bytes`] that fails unless the readback equals the dump byte for byte.
pub fn assert_same_bytes(name: &str, got: &[u8], want: &[u8]) {
    let (count, max) = compare_bytes(name, got, want);
    assert_eq!(count, 0, "{name}: {count} bytes differ (max {max})");
}

// ---- Textures ---------------------------------------------------------------------------

/// Fills `texture` with `pixels` (the host's work: Chiaro's demos draw these layers with their
/// renderer). The upload is submitted and waited for.
pub fn upload_pixels(gpu: Gpu<'_>, texture: &Texture<'_>, pixels: &[u8]) {
    let mut up = UploadBuffer::new(
        gpu,
        SDL_GPUBufferUsageFlags(0),
        pixels.len() as u32,
        "pixels",
    );
    up.reserve(pixels.len() as u64).unwrap();
    up.map().unwrap().bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .upload_to_texture(&up, texture.texture_ref(), false)
        .unwrap();
    cmd.submit_and_wait().unwrap();
}

/// A texture, filled with `pixels` if given.
pub fn texture<'d>(
    gpu: Gpu<'d>,
    (w, h): (u32, u32),
    format: SDL_GPUTextureFormat,
    usage: SDL_GPUTextureUsageFlags,
    pixels: Option<&[u8]>,
) -> Texture<'d> {
    let texture = Texture::new(gpu, TextureDesc::new(w, h, format, usage)).expect("texture");
    if let Some(px) = pixels {
        upload_pixels(gpu, &texture, px);
    }
    texture
}

/// A sampled texture of `format` filled with `pixels`.
pub fn sampled<'d>(
    gpu: Gpu<'d>,
    w: u32,
    h: u32,
    format: SDL_GPUTextureFormat,
    pixels: &[u8],
) -> Texture<'d> {
    texture(
        gpu,
        (w, h),
        format,
        SDL_GPU_TEXTUREUSAGE_SAMPLER,
        Some(pixels),
    )
}

/// A sampled `R8G8B8A8_UNORM` texture filled with `pixels`.
pub fn sampled_rgba8<'d>(gpu: Gpu<'d>, w: u32, h: u32, pixels: &[u8]) -> Texture<'d> {
    sampled(gpu, w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, pixels)
}

/// Records the copy of a whole texture into a new [`DownloadBuffer`] on `cmd`. The bytes are
/// there after the host submitted and waited; read them with [`read_download`].
pub fn queue_download<'d>(
    gpu: Gpu<'d>,
    cmd: &mut CommandBuffer<'_>,
    texture: TextureRef<'_>,
) -> DownloadBuffer<'d> {
    let down = DownloadBuffer::new(gpu, texture.desc().bytes() as u32).unwrap();
    cmd.copy_pass()
        .unwrap()
        .download_texture(texture, &down)
        .unwrap();
    down
}

/// The bytes of a finished [`DownloadBuffer`].
pub fn read_download(mut download: DownloadBuffer<'_>) -> Vec<u8> {
    let mut bytes = vec![0; download.size() as usize];
    download.read(&mut bytes).unwrap();
    bytes
}

/// A texture read back on its own command buffer (submits and waits).
pub fn read_texture(gpu: Gpu<'_>, texture: TextureRef<'_>) -> Vec<u8> {
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    let down = queue_download(gpu, &mut cmd, texture);
    cmd.submit_and_wait().unwrap();
    read_download(down)
}

/// What the layout tests need from a compiled SPIR-V shader: where each named resource is
/// bound and at which byte offset each struct member sits. Reads only the debug names and
/// decorations, so it needs no GPU.
pub mod spirv {
    use std::collections::BTreeMap;

    /// One decoded SPIR-V module.
    pub struct Module {
        /// Variable name to `(descriptor set, binding)`.
        pub bindings: BTreeMap<String, (u32, u32)>,
        /// Every named id with the `(member name, byte offset)` pairs `OpMemberDecorate Offset`
        /// gave it, in member order (empty for names that are not structs).
        pub structs: Vec<(String, Vec<(String, u32)>)>,
    }

    fn string(words: &[u32]) -> String {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec()).unwrap()
    }

    /// Decodes a module; panics on anything that is not well-formed SPIR-V.
    pub fn decode(code: &[u8]) -> Module {
        let words: Vec<u32> = code
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        assert_eq!(words[0], 0x0723_0203, "not SPIR-V");
        let mut names = BTreeMap::new();
        let mut members = Vec::new();
        let mut offsets = Vec::new();
        let mut decorations: BTreeMap<u32, (Option<u32>, Option<u32>)> = BTreeMap::new();
        let mut i = 5;
        while i < words.len() {
            let (n, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
            assert!(n > 0, "malformed SPIR-V");
            let a = &words[i + 1..i + n];
            match op {
                5 => {
                    names.insert(a[0], string(&a[1..])); // OpName
                }
                6 => members.push((a[0], a[1], string(&a[2..]))), // OpMemberName
                71 if n >= 4 => {
                    // OpDecorate
                    let d = decorations.entry(a[0]).or_default();
                    match a[1] {
                        34 => d.0 = Some(a[2]), // DescriptorSet
                        33 => d.1 = Some(a[2]), // Binding
                        _ => {}
                    }
                }
                72 if a[2] == 35 => offsets.push((a[0], a[1], a[3])), // OpMemberDecorate Offset
                _ => {}
            }
            i += n;
        }
        let bindings = decorations
            .into_iter()
            .filter_map(|(id, (set, binding))| {
                Some((names.get(&id)?.clone(), (set.unwrap_or(u32::MAX), binding?)))
            })
            .collect();
        let structs = names
            .iter()
            .map(|(&id, name)| {
                let mut m: Vec<(u32, String, u32)> = offsets
                    .iter()
                    .filter(|o| o.0 == id)
                    .map(|&(_, member, off)| {
                        let member_name = members
                            .iter()
                            .find(|x| x.0 == id && x.1 == member)
                            .map_or_else(String::new, |x| x.2.clone());
                        (member, member_name, off)
                    })
                    .collect();
                m.sort_by_key(|x| x.0);
                (
                    name.clone(),
                    m.into_iter().map(|(_, n, o)| (n, o)).collect(),
                )
            })
            .collect();
        Module { bindings, structs }
    }
}
