//! The guard scene as a picture: the host's side of a frame (ADR 0009, phases 3 and 4).
//! `cargo run -p knorke-runtime --features gpu --example guard_scene_view -- <out dir>`
//! writes one PNG per frame.
//!
//! The simulation in `guard_scene.rs` knows nothing of this file. After the ticks of a frame
//! the host takes a [`Pose`] from the actors, blends it with the pose before the last tick
//! ([`Pose::blend`], weight `Frame::alpha`) and hands it to [`View::draw`]: the yard is
//! painted on the CPU (floor, wall, two discs), the wall also into the occlusion mask, and
//! the 2D light adds the guard's lamp, which turns with the guard and stops at the wall.
//!
//! Needs a Vulkan device and the shader pack from `cargo xtask shaders` (as the GPU tests).

// The scene is shared with its own example and tests; here its `main` is unused.
#[allow(dead_code, unreachable_pub)]
#[path = "guard_scene.rs"]
mod guard_scene;

pub use guard_scene::*;

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{sampled_rgba8, texture, upload_pixels};
use knorke_gpu::{CommandBuffer, DownloadBuffer, Gpu, Texture};
use knorke_integration::Cell;
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_light2d::light::{Height, LightDesc, LightId, LightType};
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};

/// Pixels per cell.
pub const PX: u32 = 16;
/// Width of the picture.
pub const W: u32 = 16 * PX;
/// Height of the picture.
pub const H: u32 = 10 * PX;
/// The wall's column and first row, as in the scene.
const WALL: (u32, u32) = (8, 2);

/// Where the actors are drawn, in cells (fractions between two ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The intruder's centre.
    pub intruder: Vec2,
    /// The guard's centre.
    pub guard: Vec2,
    /// The guard's heading (0..=255, 0 = +x, 64 = +y).
    pub facing: u8,
}

fn centre(c: Cell) -> Vec2 {
    Vec2::new(c.x as f32 + 0.5, c.y as f32 + 0.5)
}

impl Pose {
    /// The pose after the last tick.
    pub fn of(actors: &Actors) -> Self {
        Self {
            intruder: centre(actors.intruder),
            guard: centre(actors.guard),
            facing: actors.facing,
        }
    }

    /// `alpha` of the way from `before` to `now` (`Frame::alpha`). The heading does not
    /// blend: the lamp turns with the tick.
    pub fn blend(before: &Pose, now: &Pose, alpha: f32) -> Self {
        let mix = |a: Vec2, b: Vec2| a + (b - a) * alpha;
        Self {
            intruder: mix(before.intruder, now.intruder),
            guard: mix(before.guard, now.guard),
            facing: now.facing,
        }
    }
}

/// The lamp's direction for a heading in steps of 45 degrees.
fn lamp_direction(facing: u8) -> Vec3 {
    const D: f32 = core::f32::consts::FRAC_1_SQRT_2;
    const DIRS: [(f32, f32); 8] = [
        (1.0, 0.0),
        (D, D),
        (0.0, 1.0),
        (-D, D),
        (-1.0, 0.0),
        (-D, -D),
        (0.0, -1.0),
        (D, -D),
    ];
    let (x, y) = DIRS[usize::from(facing / 32) % 8];
    Vec3::new(x, y, 0.0)
}

fn wall_pixel(x: u32, y: u32) -> bool {
    x / PX == WALL.0 && y / PX >= WALL.1
}

/// Paints the yard and the actors of `pose` into `rgba` (`W` x `H`).
fn paint(pose: &Pose, rgba: &mut [u8]) {
    let disc = |x: u32, y: u32, at: Vec2| {
        let dx = (x as f32 + 0.5) / PX as f32 - at.x;
        let dy = (y as f32 + 0.5) / PX as f32 - at.y;
        dx * dx + dy * dy < 0.4 * 0.4
    };
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % W, i as u32 / W);
        *px = if disc(x, y, pose.intruder) {
            [200, 60, 50, 255]
        } else if disc(x, y, pose.guard) {
            [60, 110, 210, 255]
        } else if wall_pixel(x, y) {
            [70, 62, 58, 255]
        } else if (x / PX + y / PX).is_multiple_of(2) {
            [150, 150, 140, 255]
        } else {
            [135, 135, 126, 255]
        };
    }
}

/// The host's GPU side: its textures, the lamp and the light renderer.
pub struct View<'d> {
    gpu: Gpu<'d>,
    renderer: Light2dRenderer<'d>,
    colour: Texture<'d>,
    occlusion: Texture<'d>,
    target: Texture<'d>,
    lights: LightStore,
    lamp: LightId,
    /// The painted yard; reused.
    canvas: Vec<u8>,
}

impl<'d> View<'d> {
    /// Sets up the textures and the lamp. `pack` is the shader pack.
    pub fn new(gpu: Gpu<'d>, pack: Vec<u8>) -> Self {
        let canvas = vec![0u8; (W * H * 4) as usize];
        let mut mask = canvas.clone();
        for (i, px) in mask.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            px[3] = if wall_pixel(i as u32 % W, i as u32 / W) {
                255
            } else {
                0
            };
        }
        let mut lights = LightStore::new();
        let lamp = lights
            .create(&Self::lamp(&Pose::of_start()), &NoResources)
            .unwrap();
        Self {
            gpu,
            renderer: Light2dRenderer::new(gpu, pack).unwrap(),
            colour: sampled_rgba8(gpu, W, H, &canvas),
            occlusion: sampled_rgba8(gpu, W, H, &mask),
            target: texture(
                gpu,
                (W, H),
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            ),
            lights,
            lamp,
            canvas,
        }
    }

    /// The guard's lamp: a warm cone from the guard along its heading.
    fn lamp(pose: &Pose) -> LightDesc {
        let mut d = LightDesc::new(LightType::Spot);
        d.position = At::new(pose.guard);
        d.height = Height::At(0.0);
        d.direction = lamp_direction(pose.facing);
        d.radius = 9.0;
        d.inner_angle = 28.0;
        d.outer_angle = 45.0;
        d.color = Vec3::new(1.0, 0.86, 0.62);
        d.intensity = 1.6;
        d.source_radius = 0.15;
        d
    }

    /// Draws one frame and returns it as RGBA8 (`W` x `H`). Submits and reads back: this is
    /// the host (ADR 0002).
    pub fn draw(&mut self, pose: &Pose) -> Vec<u8> {
        paint(pose, &mut self.canvas);
        upload_pixels(self.gpu, &self.colour, &self.canvas);
        self.lights
            .update(self.lamp, &Self::lamp(pose), &NoResources)
            .unwrap();
        // One world unit is one cell.
        let scale = PX as f32;
        let projection = Projection {
            x_axis: Vec2::new(scale, 0.0),
            y_axis: Vec2::new(0.0, scale),
            ..Projection::default()
        };
        let frame = Light2dFrame {
            occlusion: Some(self.occlusion.texture_ref()),
            ..Light2dFrame::new(
                self.colour.texture_ref(),
                self.target.texture_ref(),
                &projection,
            )
        };
        let settings = Light2dSettings {
            light: LightSettings {
                ambient: Vec3::new(0.12, 0.13, 0.18),
                ..LightSettings::default()
            },
            ..Light2dSettings::default()
        };
        let obstacles = ObstacleStore::new();
        let stores = SceneStores::new(&self.lights, &obstacles);
        let prepared = self.renderer.prepare(&frame, &stores, &settings).unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        self.renderer
            .record(&mut cmd, &frame, &settings, prepared)
            .unwrap();
        let mut download = DownloadBuffer::new(self.gpu, W * H * 4).unwrap();
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, &download)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let mut pixels = vec![0u8; (W * H * 4) as usize];
        download.read(&mut pixels).unwrap();
        pixels
    }
}

impl Pose {
    /// The pose before the first tick.
    pub fn of_start() -> Self {
        Self::of(&Scene::new(knorke_observe::NoProbe).yard.actors)
    }
}

/// Simulation ticks per second of the played scene.
pub const TICKS_PER_SECOND: u64 = 4;
/// Drawn frames per second: three frames per tick, so two of three show a blended pose.
pub const FRAMES_PER_SECOND: u64 = 12;

/// Plays `script` at [`FRAMES_PER_SECOND`] for `frames` frames and calls `show` with every
/// drawn frame. The host's whole frame loop: input, ticks, blend, draw.
pub fn play(view: &mut View<'_>, script: Script, frames: u64, mut show: impl FnMut(u64, &[u8])) {
    use knorke_runtime::{FixedStep, Runtime};
    const SECOND: u64 = 1_000_000_000;
    let step = FixedStep::new(SECOND / TICKS_PER_SECOND, 4);
    let scene = Scene::new(knorke_observe::NoProbe);
    let mut rt = Runtime::new(
        scene,
        knorke_replay::NoRecord,
        step,
        knorke_types::Tick::ZERO,
    );
    // Input: the script speaks in ticks; a command goes in once, just before its tick.
    let feed = |rt: &mut Runtime<Scene>| {
        if let Some(command) = script.command(rt.next_tick().0) {
            rt.submit(command);
        }
    };
    feed(&mut rt);
    let mut before = Pose::of(&rt.sim.yard.actors);
    for f in 0..frames {
        let mut now = Pose::of(&rt.sim.yard.actors);
        // At most one tick per frame, so the pose before the last tick is at hand to blend.
        let due = rt.frame(SECOND / FRAMES_PER_SECOND);
        debug_assert!(due.ticks <= 1, "three frames per tick");
        if due.ticks > 0 {
            before = now;
            now = Pose::of(&rt.sim.yard.actors);
            feed(&mut rt);
        }
        let pixels = view.draw(&Pose::blend(&before, &now, due.alpha));
        show(f, &pixels);
    }
}

#[allow(dead_code)]
fn main() {
    use knorke_contract_tests::parity::save_png_rgba8;
    use knorke_gpu::LiveCounter;
    use knorke_gpu::test_support::vulkan_device_with_pack;

    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: <out dir>"));
    std::fs::create_dir_all(&out).unwrap();
    let Some((device, pack)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let mut view = View::new(Gpu::new(&device, &counter), pack);
    let frames = END * FRAMES_PER_SECOND / TICKS_PER_SECOND;
    play(&mut view, Script::BOLD, frames, |f, pixels| {
        save_png_rgba8(&out.join(format!("guard_{f:03}.png")), pixels, W, H);
    });
    println!("{frames} frames in {}", out.display());
}
