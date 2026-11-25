//! The GPU debug check (P32) through the pass plan and on its own stage: port of the E83 cases
//! of Chiaro's `tests/gpu/DebugCheckTests.cpp` (half-alpha material and surface texels, blown-out
//! highlights, flat medium, findings a few frames late, the image never changes).
//!
//! Needs a Vulkan device and the shader pack of `cargo xtask shaders`; skips without them (a
//! failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use knorke_gpu::test_support::{
    queue_download, read_download, sampled_rgba8, upload_pixels, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu, Samplers, ShaderLibrary, Texture};
use knorke_light2d::codec::{Material, material_pixel};
use knorke_light2d::debug_check::{DebugCheckFrame, DebugCheckStage, SLOTS};
use knorke_light2d::frame_warnings::{DebugChecks, Warning, WarningSet};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassStages, SceneStores};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, ObstacleStore, Projection};

const W: u32 = 32;
const H: u32 = 24;
const BYTES: usize = (W * H * 4) as usize;
/// Frames until the findings of the first one are surely back.
const SETTLED: usize = SLOTS + 1;

/// Every texel empty except the middle one, which gets `texel`.
fn one_texel(texel: [u8; 4]) -> Vec<u8> {
    let mut px = vec![0u8; BYTES];
    let at = ((W * 12 + 16) * 4) as usize;
    px[at..at + 4].copy_from_slice(&texel);
    px
}

/// Material bytes with a flat detail normal, `b` and alpha `a` in the middle texel.
fn material(b: u8, a: u8) -> Vec<u8> {
    one_texel([128, 128, b, a])
}

struct Fx<'d> {
    gpu: Gpu<'d>,
    plan: PassPlan<'d>,
    scene: Texture<'d>,
    material: Texture<'d>,
    surface: Option<Texture<'d>>,
    lights: LightStore,
    obstacles: ObstacleStore,
    projection: Projection,
    settings: LightSettings,
    /// Stages and HDR bytes of the last frame.
    stages: PassStages,
    hdr: Vec<u8>,
}

impl<'d> Fx<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8], surface: bool) -> Self {
        let empty = vec![0u8; BYTES];
        Self {
            gpu,
            plan: PassPlan::new(gpu, pack.to_vec()).unwrap(),
            scene: sampled_rgba8(gpu, W, H, &vec![153u8; BYTES]),
            material: sampled_rgba8(gpu, W, H, &empty),
            surface: surface.then(|| sampled_rgba8(gpu, W, H, &empty)),
            lights: LightStore::new(),
            obstacles: ObstacleStore::new(),
            projection: Projection::default(),
            settings: LightSettings::default(),
            stages: PassStages::NONE,
            hdr: Vec::new(),
        }
    }

    /// Renders `frames` frames; returns the warnings of the last one.
    fn render(&mut self, frames: usize) -> WarningSet {
        let mut warnings = WarningSet::EMPTY;
        for _ in 0..frames {
            let frame = PassFrame {
                material: Some(self.material.texture_ref()),
                surface: self.surface.as_ref().map(Texture::texture_ref),
                ..PassFrame::new(self.scene.texture_ref(), &self.projection)
            };
            let stores = SceneStores::new(&self.lights, &self.obstacles);
            let prepared = self.plan.prepare(&frame, &stores, &self.settings).unwrap();
            let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
            warnings = self
                .plan
                .record(&mut cmd, &frame, &self.settings)
                .unwrap()
                .warnings;
            let hdr = queue_download(self.gpu, &mut cmd, self.plan.hdr().unwrap());
            cmd.submit_and_wait().unwrap();
            self.stages = prepared.stages;
            self.hdr = read_download(hdr);
        }
        warnings
    }

    fn with_material(&mut self, pixels: &[u8], frames: usize) -> WarningSet {
        upload_pixels(self.gpu, &self.material, pixels);
        self.render(frames)
    }
}

#[test]
fn half_alpha_material_texels_warn_a_few_frames_later_and_the_image_is_unchanged() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, false);
        let half = material(0x20, 100);
        assert!(fx.with_material(&half, 1).is_empty());
        assert!(!fx.stages.contains(PassStages::DEBUG_CHECK));
        let (off, pipelines_off) = (fx.hdr.clone(), fx.plan.pipeline_count());

        fx.plan.set_debug_checks(DebugChecks::LAYERS);
        assert!(fx.with_material(&half, 1).is_empty()); // the findings come back later
        assert!(fx.stages.contains(PassStages::DEBUG_CHECK));
        assert_eq!(fx.plan.pipeline_count(), pipelines_off + 1);
        assert_eq!(fx.hdr, off);
        assert_eq!(
            fx.with_material(&half, SLOTS),
            WarningSet::of(Warning::HalfAlpha)
        );
        assert_eq!(fx.hdr, off);

        // Dry entry 255, wet entry 224, empty 0: clean once the old findings are through.
        assert!(fx.with_material(&material(0x20, 255), SETTLED).is_empty());
        assert!(fx.with_material(&material(0x20, 224), SETTLED).is_empty());

        // Off again: no stage and no finding, also not one still on its way.
        fx.with_material(&half, SETTLED);
        fx.plan.set_debug_checks(DebugChecks::NONE);
        assert!(fx.render(SETTLED).is_empty());
        assert!(!fx.stages.contains(PassStages::DEBUG_CHECK));
        fx.plan.set_debug_checks(DebugChecks::LAYERS);
        assert!(fx.with_material(&material(0x20, 255), SETTLED).is_empty());
    });
}

#[test]
fn a_highlight_above_the_energy_conserving_maximum_is_blown_out() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, false);
        fx.plan.set_debug_checks(DebugChecks::LAYERS);
        // Specular 7/7, gloss 15: 32.25 times the diffuse light.
        let brightest = material(0xEF, 255);
        assert!(fx.with_material(&brightest, SETTLED).is_empty());
        fx.settings.material.specular_scale = 2.0;
        assert_eq!(
            fx.with_material(&brightest, SETTLED),
            WarningSet::of(Warning::BlownOutHighlights)
        );
        // Every preset stays below the limit at scale 1, the wet ones also without weather.
        fx.settings.material.specular_scale = 1.0;
        for preset in [
            Material::MATTE,
            Material::SATIN,
            Material::PLASTIC,
            Material::GLASS,
            Material::METAL_DULL,
            Material::METAL_POLISHED,
            Material::ASPHALT_DRY,
            Material::ASPHALT_WET,
            Material::PUDDLE,
        ] {
            let px = material_pixel(preset, None).repeat((W * H) as usize);
            assert!(fx.with_material(&px, SETTLED).is_empty(), "{preset:?}");
        }
    });
}

#[test]
fn half_alpha_surface_texels_warn() {
    with_chiaro_pack(|gpu, pack| {
        let mut fx = Fx::new(gpu, pack, true);
        fx.plan.set_debug_checks(DebugChecks::LAYERS);
        for (alpha, want) in [
            (200, WarningSet::of(Warning::HalfAlpha)),
            (255, WarningSet::EMPTY),
        ] {
            let surface = fx.surface.as_ref().unwrap();
            upload_pixels(gpu, surface, &one_texel([0, 128, 128, alpha]));
            assert_eq!(fx.render(SETTLED), want, "alpha {alpha}");
        }
    });
}

/// The medium layer is not part of the pass plan yet (P12), so its check runs on the stage.
#[test]
fn medium_pixels_with_density_but_no_height_span_warn() {
    with_chiaro_pack(|gpu, pack| {
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let samplers = Samplers::new(gpu).unwrap();
        let mut stage = DebugCheckStage::new(gpu, &shaders).unwrap();
        let medium = sampled_rgba8(gpu, W, H, &vec![0u8; BYTES]);
        let settings = LightSettings::default();
        let mut checks = DebugChecks::LAYERS;
        // R top, G 255 - bottom, B kind, A density, in the middle pixel.
        let mut after = |checks: DebugChecks, density: u8, top: u8, bottom: u8, frames: usize| {
            upload_pixels(gpu, &medium, &one_texel([top, 255 - bottom, 0, density]));
            let frame = DebugCheckFrame {
                size: (W, H),
                surface: None,
                material: None,
                medium: Some(medium.texture_ref()),
                settings: &settings,
            };
            let mut found = WarningSet::EMPTY;
            for _ in 0..frames {
                stage.prepare(&mut shaders, checks, &frame).unwrap();
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                found = stage.record(&mut cmd, &samplers, &frame).unwrap();
                cmd.submit_and_wait().unwrap();
            }
            found
        };
        let flat = WarningSet::of(Warning::FlatMedium);
        assert!(after(checks, 128, 5, 0, SETTLED).is_empty()); // top 5 above bottom 0
        assert!(after(checks, 128, 0, 0, 1).is_empty()); // the finding comes back later
        assert_eq!(after(checks, 128, 0, 0, SETTLED), flat); // top 0 with density
        assert_eq!(after(checks, 128, 3, 7, SETTLED), flat); // top below bottom
        assert_eq!(after(checks, 128, 4, 4, SETTLED), flat); // top equal to bottom
        assert!(after(checks, 0, 0, 0, SETTLED).is_empty()); // no density: no medium
        assert!(after(checks, 128, 5, 0, SETTLED).is_empty());
        checks = DebugChecks::NONE;
        assert!(after(checks, 128, 0, 0, SETTLED).is_empty());
    });
}
