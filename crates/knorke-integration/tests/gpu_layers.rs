//! Scene layers (P33) on Vulkan: port of the GPU layer cases of Chiaro's
//! `tests/gpu/LayerTests.cpp` and of the stale layer case of `DebugCheckTests.cpp`.
//!
//! Needs a Vulkan device and the shader pack of `cargo xtask shaders`; skips without them (a
//! failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use knorke_gpu::sys::gpu::{SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPUBufferUsageFlags};
use knorke_gpu::test_support::{
    queue_download, read_download, read_texture, sampled_rgba8, texture, with_chiaro_pack,
    with_vulkan,
};
use knorke_gpu::{CommandBuffer, Gpu, Texture, TextureRef, UploadBuffer};
use knorke_integration::layers::{LAYER_FORMAT, LAYER_TARGETS, Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_light2d::frame_warnings::{DebugChecks, Warning, WarningSet};
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::obstacle::ShapeDesc;
use knorke_light2d::pass_plan::{PassPrepared, PassStages, SceneStores};
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2};

const W: u32 = 32;
const H: u32 = 24;

/// Deterministic test pixels; `opaque` sets every alpha to 255.
fn pattern(seed: u32, opaque: bool) -> Vec<u8> {
    let mut px = Vec::with_capacity((W * H * 4) as usize);
    for i in 0..W * H {
        let v = i
            .wrapping_mul(2_654_435_761)
            .wrapping_add(seed.wrapping_mul(97));
        let [r, g, b, a] = v.to_le_bytes();
        px.extend([r, g, b, if opaque { 255 } else { a }]);
    }
    px
}

/// The host writing a layer its own way.
fn upload(gpu: Gpu<'_>, to: TextureRef<'_>, pixels: &[u8]) {
    let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), pixels.len() as u32, "px");
    up.reserve(pixels.len() as u64).unwrap();
    up.map().unwrap().bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .upload_to_texture(&up, to, false)
        .unwrap();
    cmd.submit_and_wait().unwrap();
}

/// Clears `kinds` as a host does at the start of its frame (and draws nothing).
fn clear(gpu: Gpu<'_>, layers: &mut Layers<'_>, kinds: LayerSet) {
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    drop(layers.clear_pass(&mut cmd, kinds).unwrap());
    cmd.submit_and_wait().unwrap();
}

#[test]
fn formats_clear_values_targets_and_resize() {
    with_vulkan(|gpu| {
        let kinds = LayerSet::of(Layer::Material).with(Layer::Surface);
        let mut layers = Layers::new(gpu, W, H, kinds).unwrap();
        assert_eq!(layers.kinds(), kinds.with(Layer::Scene));
        for layer in Layer::ALL {
            let made = matches!(layer, Layer::Scene | Layer::Surface | Layer::Material);
            let t = layers.texture(layer);
            assert_eq!(t.is_some(), made, "{layer:?}");
            if let Some(t) = t {
                assert_eq!((t.width(), t.height(), t.format()), (W, H, LAYER_FORMAT));
            }
        }

        // Garbage first, so the clear is what the readback shows.
        for layer in [Layer::Scene, Layer::Material] {
            upload(gpu, layers.texture(layer).unwrap(), &pattern(7, false));
        }
        clear(gpu, &mut layers, LayerSet::ALL);
        let scene = read_texture(gpu, layers.texture(Layer::Scene).unwrap());
        assert_eq!(scene, [0, 0, 0, 255].repeat((W * H) as usize));
        let material = read_texture(gpu, layers.texture(Layer::Material).unwrap());
        assert_eq!(material, vec![0; material.len()]);

        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let none = layers.clear_pass(&mut cmd, Layer::Glow.into());
        assert_eq!(none.err().map(|e| e.code), Some(LAYER_TARGETS));
        cmd.submit_and_wait().unwrap();

        layers.resize(W * 2, H).unwrap();
        assert_eq!(layers.texture(Layer::Material).unwrap().width(), W * 2);
        assert!(layers.resize(0, H).is_err());
        assert_eq!(layers.texture(Layer::Material).unwrap().width(), W * 2);
        assert!(Layers::new(gpu, 0, H, LayerSet::NONE).is_err());

        // Seven layers do not fit one pass; two passes clear them all.
        let mut all = Layers::new(gpu, W, H, LayerSet::ALL).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let too_many = all.clear_pass(&mut cmd, LayerSet::ALL);
        assert_eq!(too_many.err().map(|e| e.code), Some(LAYER_TARGETS));
        let first = LayerSet::of(Layer::Scene)
            .with(Layer::Surface)
            .with(Layer::Material)
            .with(Layer::Glow);
        let rest = LayerSet::of(Layer::Occlusion)
            .with(Layer::Overlay)
            .with(Layer::Medium);
        drop(all.clear_pass(&mut cmd, first).unwrap());
        drop(all.clear_pass(&mut cmd, rest).unwrap());
        cmd.submit_and_wait().unwrap();
    });
}

/// An output target of the scene's size.
fn target(gpu: Gpu<'_>) -> Texture<'_> {
    texture(
        gpu,
        (W, H),
        LAYER_FORMAT,
        SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
        None,
    )
}

struct Fx<'d> {
    gpu: Gpu<'d>,
    renderer: Light2dRenderer<'d>,
    lights: LightStore,
    obstacles: ObstacleStore,
    projection: Projection,
    settings: Light2dSettings,
}

impl<'d> Fx<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8], light_at: (f32, f32)) -> Self {
        let mut lights = LightStore::new();
        let mut d = LightDesc::new(LightType::Point);
        d.position = At::new(Vec2::new(light_at.0, light_at.1));
        d.radius = 20.0;
        lights.create(&d, &NoResources).unwrap();
        let mut settings = Light2dSettings::default();
        settings.light.ambient = knorke_math::Vec3::new(0.1, 0.1, 0.1);
        Self {
            gpu,
            renderer: Light2dRenderer::new(gpu, pack.to_vec()).unwrap(),
            lights,
            obstacles: ObstacleStore::new(),
            projection: Projection::default(),
            settings,
        }
    }

    /// One frame: the target's bytes, the warnings and the light's decision.
    fn render(&mut self, frame: &Light2dFrame<'_>) -> (Vec<u8>, WarningSet, PassPrepared) {
        let stores = SceneStores::new(&self.lights, &self.obstacles);
        let prepared = self
            .renderer
            .prepare(frame, &stores, &self.settings)
            .unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        let record = self
            .renderer
            .record(&mut cmd, frame, &self.settings, prepared)
            .unwrap();
        let image = queue_download(self.gpu, &mut cmd, frame.target);
        cmd.submit_and_wait().unwrap();
        (read_download(image), record.warnings, prepared.light)
    }
}

#[test]
fn a_frame_from_layers_equals_one_from_hand_made_buffers_and_a_field_overrides_its_layer() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, (6.0, 5.0));
        let px = [pattern(1, true), pattern(2, true), pattern(3, false)];
        let layers =
            Layers::new(gpu, W, H, LayerSet::of(Layer::Material).with(Layer::Glow)).unwrap();
        for (layer, px) in [Layer::Scene, Layer::Material, Layer::Glow]
            .into_iter()
            .zip(&px)
        {
            upload(gpu, layers.texture(layer).unwrap(), px);
        }
        let [scene, material, glow] = px.each_ref().map(|px| sampled_rgba8(gpu, W, H, px));
        let other = sampled_rgba8(gpu, W, H, &pattern(9, true));
        let target = target(gpu);
        let (target, projection) = (target.texture_ref(), fx.projection);

        let by_hand = Light2dFrame {
            material: Some(material.texture_ref()),
            glow: Some(glow.texture_ref()),
            ..Light2dFrame::new(scene.texture_ref(), target, &projection)
        };
        let by_layers = layers.frame(target, &projection, LayerSet::NONE);
        let a = fx.render(&by_hand).0;
        assert_eq!(fx.render(&by_layers).0, a);

        let other = Some(other.texture_ref());
        let b = fx
            .render(&Light2dFrame {
                material: other,
                ..by_hand
            })
            .0;
        assert_ne!(b, a);
        let overridden = Light2dFrame {
            material: other,
            ..by_layers
        };
        assert_eq!(fx.render(&overridden).0, b);
    });
}

#[test]
fn an_occlusion_layer_nothing_drew_into_is_left_out() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, (4.0, 4.0));
        let mut wall = ShapeDesc::rect(2.0, 2.0);
        wall.common.position = At::new(Vec2::new(10.0, 6.0));
        fx.obstacles.create_shape(&wall).unwrap();
        let mut layers = Layers::new(gpu, W, H, Layer::Occlusion.into()).unwrap();
        let target = target(gpu);
        let (target, projection) = (target.texture_ref(), fx.projection);
        let with_mask = LayerSet::of(Layer::Scene).with(Layer::Occlusion);

        // Only the scene is targeted: no sprite mask, the shadow field is reused.
        clear(gpu, &mut layers, Layer::Scene.into());
        let frame = layers.frame(target, &projection, LayerSet::NONE);
        assert!(frame.occlusion.is_none());
        let untargeted = fx.render(&frame).0;
        let again = fx.render(&frame).2;
        assert!(again.stages.contains(PassStages::OCCLUSION));
        assert_eq!(
            again.mask_update,
            Some(knorke_light2d::shadow::MaskUpdate::Reused)
        );

        // Targeted (and cleared): a sprite mask.
        clear(gpu, &mut layers, with_mask);
        assert!(
            layers
                .frame(target, &projection, LayerSet::NONE)
                .occlusion
                .is_some()
        );
        // The next frame's first clear starts over.
        clear(gpu, &mut layers, Layer::Scene.into());
        layers.take_stale();
        clear(gpu, &mut layers, Layer::Scene.into());
        assert!(
            layers
                .frame(target, &projection, LayerSet::NONE)
                .occlusion
                .is_none()
        );
        layers.mark_drawn(Layer::Occlusion.into());
        assert!(
            layers
                .frame(target, &projection, LayerSet::NONE)
                .occlusion
                .is_some()
        );

        // Skipped explicitly: the image of the untargeted frame.
        let skipped = layers.frame(target, &projection, Layer::Occlusion.into());
        assert!(skipped.occlusion.is_none());
        assert_eq!(fx.render(&skipped).0, untargeted);
    });
}

#[test]
fn layers_rendered_again_without_a_clear_are_stale() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, (16.0, 12.0));
        fx.renderer.plan_mut().set_debug_checks(DebugChecks::LAYERS);
        let mut layers = Layers::new(gpu, W, H, LayerSet::NONE).unwrap();
        let target = target(gpu);
        let projection = fx.projection;
        let stale = WarningSet::of(Warning::StaleLayers);
        for (cleared, want) in [
            (false, stale), // never cleared
            (true, WarningSet::EMPTY),
            (false, stale), // the same content again
            (true, WarningSet::EMPTY),
        ] {
            if cleared {
                clear(gpu, &mut layers, Layer::Scene.into());
            }
            fx.renderer.note_layers(&mut layers);
            let frame = layers.frame(target.texture_ref(), &projection, LayerSet::NONE);
            assert_eq!(fx.render(&frame).1, want, "cleared {cleared}");
        }

        // Checks off: nothing is reported.
        fx.renderer.plan_mut().set_debug_checks(DebugChecks::NONE);
        fx.renderer.note_layers(&mut layers);
        let frame = layers.frame(target.texture_ref(), &projection, LayerSet::NONE);
        assert!(fx.render(&frame).1.is_empty());
    });
}
