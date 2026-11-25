//! `MediumBatch` on GPU layers: port of the GPU-layer halves of Chiaro's
//! `tests/gpu/MediumLayerTests.cpp` (chiaro @ 9bc5b9b, zlib license). Renderer layers do not
//! exist here. Skips without a Vulkan device or the shader pack (a failure with
//! `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use knorke_gpu::test_support::{read_texture, sampled_rgba8, with_chiaro_pack};
use knorke_gpu::{CommandBuffer, Gpu, ShaderLibrary, TextureRef};
use knorke_integration::layers::{LAYER_TARGETS, Layer, LayerSet, Layers};
use knorke_integration::medium::{MediumBatch, MediumItem};
use knorke_light2d::codec::encode_medium_pixel;
use knorke_light2d::medium_draw::{MEDIUM_DRAW_INVALID, MediumDraw};

const W: u32 = 16;
const H: u32 = 12;
type Px = [u8; 4];
const NONE: Px = [0; 4];
const RECT_A: [f32; 4] = [1.0, 1.0, 8.0, 6.0];
const RECT_B: [f32; 4] = [5.0, 3.0, 8.0, 7.0];

fn draw(rect: [f32; 4], density: f32, bottom: f32, top: f32, medium: u32) -> MediumDraw {
    MediumDraw {
        rect,
        density,
        bottom,
        top,
        medium,
    }
}

fn plain(d: MediumDraw) -> MediumItem<'static> {
    MediumItem {
        draw: d,
        shape: None,
    }
}

/// The GPU layers of the fixture (step 1 m) and the batch.
struct Fixture<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    batch: MediumBatch<'d>,
    layers: Layers<'d>,
}

impl<'d> Fixture<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let batch = MediumBatch::new(gpu, &shaders).unwrap();
        let kinds = LayerSet::of(Layer::Scene).with(Layer::Medium);
        let layers = Layers::new(gpu, W, H, kinds).unwrap();
        Self {
            gpu,
            shaders,
            batch,
            layers,
        }
    }

    /// Clears the layers, draws `items`, reads the medium layer back.
    fn run(&mut self, items: &[MediumItem<'_>]) -> Vec<u8> {
        self.batch.prepare(&mut self.shaders, items).unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        drop(
            self.layers
                .clear_pass(&mut cmd, self.layers.kinds())
                .unwrap(),
        );
        self.batch
            .record(&mut cmd, &mut self.layers, 1.0, items)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        read_texture(self.gpu, self.layers.texture(Layer::Medium).unwrap())
    }
}

fn at(px: &[u8], x: u32, y: u32) -> Px {
    px[((y * W + x) * 4) as usize..][..4].try_into().unwrap()
}

/// Expected bytes of a shaped draw at shape alpha `alpha`: the slab mid +- half * sqrt(1 - (1 - a)^2).
fn shaped_bytes(alpha: u8, density: f32, bottom: i32, top: i32, kind: u8) -> Px {
    let a = f32::from(alpha) / 255.0;
    let mid = 0.5 * (bottom + top) as f32;
    let half = 0.5 * (top - bottom) as f32;
    let k = (1.0 - (1.0 - a) * (1.0 - a)).sqrt();
    let t = (mid + half * k + 0.5).floor() as i32;
    let b = (mid - half * k + 0.5).floor() as i32;
    [
        t as u8,
        (255 - b) as u8,
        kind * 16,
        (density * a * 255.0).round() as u8,
    ]
}

/// A 1 x 1 shape of alpha `a` with a colour that must not matter.
fn shape<'d>(gpu: Gpu<'d>, a: u8) -> knorke_gpu::Texture<'d> {
    sampled_rgba8(gpu, 1, 1, &[10, 200, 30, a])
}

fn shaped(d: MediumDraw, t: TextureRef<'_>) -> MediumItem<'_> {
    MediumItem {
        draw: d,
        shape: Some(t),
    }
}

#[test]
fn overlapping_draws_add_density_and_keep_the_max_of_heights_and_kind() {
    with_chiaro_pack(|gpu, pack| {
        let mut f = Fixture::new(gpu, pack);
        let draws = [
            draw(RECT_A, 0.3, 0.0, 2.0, 1),
            draw(RECT_B, 0.4, 1.0, 4.0, 3),
        ];
        let px = f.run(&draws.map(plain));
        assert_eq!(at(&px, 2, 2), [2, 255, 16, 77]);
        assert_eq!(at(&px, 11, 8), [4, 254, 48, 102]);
        let both = at(&px, 6, 4);
        assert_eq!(&both[..3], &[4, 255, 48]);
        assert!(both[3].abs_diff(179) <= 1, "{both:?}"); // 0.7 * 255 = 178.5
        assert_eq!(at(&px, 0, 0), NONE);
        assert_eq!(at(&px, 15, 11), NONE);
        // A single draw gives the encoder's bytes.
        let one = f.run(&[plain(draws[0])]);
        assert_eq!(
            at(&one, 2, 2),
            encode_medium_pixel(0.3, 0.0, 2.0, 1, 1.0).unwrap()
        );
    });
}

#[test]
fn added_density_saturates_at_one() {
    with_chiaro_pack(|gpu, pack| {
        let mut f = Fixture::new(gpu, pack);
        let px = f.run(&[plain(draw(RECT_A, 0.4, 0.0, 2.0, 0)); 4]);
        assert_eq!(at(&px, 2, 2), [2, 255, 0, 255]);
    });
}

#[test]
fn a_shape_scales_the_density_and_thins_the_slab_toward_its_middle() {
    with_chiaro_pack(|gpu, pack| {
        let mut f = Fixture::new(gpu, pack);
        let (full, half, clear) = (shape(gpu, 255), shape(gpu, 128), shape(gpu, 0));
        let (left, right, under) = (
            [0.0, 0.0, 6.0, 4.0],
            [8.0, 0.0, 6.0, 4.0],
            [0.0, 6.0, 6.0, 4.0],
        );
        let px = f.run(&[
            shaped(draw(left, 0.8, 2.0, 22.0, 2), full.texture_ref()),
            shaped(draw(right, 0.8, 2.0, 22.0, 2), half.texture_ref()),
            plain(draw(under, 0.25, 1.0, 3.0, 1)),
            shaped(draw(under, 1.0, 0.0, 40.0, 5), clear.texture_ref()),
        ]);
        assert_eq!(at(&px, 3, 2), [22, 253, 32, 204]);
        let want = shaped_bytes(128, 0.8, 2, 22, 2);
        assert_eq!(&want[..2], &[21, 252]);
        let h = at(&px, 11, 2);
        assert_eq!(&h[..3], &want[..3]);
        assert!(h[3].abs_diff(want[3]) <= 1, "{h:?} {want:?}");
        // A clear shape writes nothing: the plain cloud under it keeps its bytes.
        assert_eq!(
            at(&px, 3, 8),
            encode_medium_pixel(0.25, 1.0, 3.0, 1, 1.0).unwrap()
        );
        assert_eq!(at(&px, 15, 2), NONE);
    });
}

#[test]
fn where_the_shape_thins_the_slab_to_nothing_nothing_is_written() {
    with_chiaro_pack(|gpu, pack| {
        let mut f = Fixture::new(gpu, pack);
        let (faint, thin) = (shape(gpu, 10), shape(gpu, 40));
        let px = f.run(&[
            shaped(
                draw([0.0, 0.0, 6.0, 4.0], 1.0, 0.0, 2.0, 1),
                faint.texture_ref(),
            ),
            shaped(
                draw([8.0, 0.0, 6.0, 4.0], 1.0, 0.0, 2.0, 1),
                thin.texture_ref(),
            ),
        ]);
        assert_eq!(at(&px, 3, 2), NONE);
        let want = shaped_bytes(40, 1.0, 0, 2, 1);
        assert_eq!(&want[..2], &[2, 255]);
        let t = at(&px, 11, 2);
        assert_eq!(&t[..3], &want[..3]);
        assert!(t[3].abs_diff(want[3]) <= 1);
    });
}

#[test]
fn one_invalid_draw_rejects_the_batch_and_layers_without_medium_fail() {
    with_chiaro_pack(|gpu, pack| {
        let mut f = Fixture::new(gpu, pack);
        let ok = draw(RECT_A, 0.5, 0.0, 2.0, 1);
        let bad = MediumDraw { medium: 99, ..ok };
        let e = f
            .batch
            .prepare(&mut f.shaders, &[plain(ok), plain(bad)])
            .unwrap_err();
        assert_eq!(e.code, MEDIUM_DRAW_INVALID);
        assert!(e.message.starts_with("draws[1].medium"), "{}", e.message);
        // Not prepared: record refuses and the layer stays empty.
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        drop(f.layers.clear_pass(&mut cmd, f.layers.kinds()).unwrap());
        assert!(
            f.batch
                .record(&mut cmd, &mut f.layers, 1.0, &[plain(ok)])
                .is_err()
        );
        cmd.submit_and_wait().unwrap();
        let px = read_texture(gpu, f.layers.texture(Layer::Medium).unwrap());
        assert_eq!(at(&px, 2, 2), NONE);
        // No draws: nothing to do.
        f.batch.prepare(&mut f.shaders, &[]).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        f.batch.record(&mut cmd, &mut f.layers, 1.0, &[]).unwrap();
        // Layers without a medium layer.
        let mut without = Layers::new(gpu, W, H, LayerSet::of(Layer::Scene)).unwrap();
        f.batch.prepare(&mut f.shaders, &[plain(ok)]).unwrap();
        let e = f
            .batch
            .record(&mut cmd, &mut without, 1.0, &[plain(ok)])
            .unwrap_err();
        assert_eq!(e.code, LAYER_TARGETS);
        assert!(e.message.contains("Medium"), "{}", e.message);
        cmd.submit_and_wait().unwrap();
    });
}
