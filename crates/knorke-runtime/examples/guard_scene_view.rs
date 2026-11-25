//! The guard scene as a picture: the host's side of a frame (ADR 0009, phases 3 and 4).
//! `cargo run -p knorke-runtime --features gpu --example guard_scene_view -- <out dir>`
//! writes one PNG per frame.
//!
//! The simulation in `guard_scene.rs` knows nothing of this file. After the ticks of a frame
//! the host takes a [`Pose`] from the actors, blends it with the pose before the last tick
//! ([`Pose::blend`], weight `Frame::alpha`) and hands it to [`View::draw`]: the yard is
//! painted on the CPU (floor, walls, a disc per actor), the walls also into the occlusion
//! mask, and the 2D light adds every guard's lamp (component `knorke.stealth.lamp/1`), which
//! turns with the guard and stops at a wall.
//!
//! Needs a Vulkan device and the shader pack from `cargo xtask shaders` (as the GPU tests).

// The scene is shared with its own example and tests; here its `main` is unused.
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "guard_scene.rs"]
mod guard_scene;

pub use guard_scene::*;

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{sampled_rgba8, texture, upload_pixels};
use knorke_gpu::{CommandBuffer, DownloadBuffer, Gpu, Texture};
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_integration::stealth::{Actors, Guard, Lamp, Yard};
use knorke_light2d::light::{Height, LightDesc, LightId, LightType};
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};

/// Pixels per cell.
pub const PX: u32 = 16;

/// Where the actors are drawn, in cells (fractions between two ticks).
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    /// The intruder's centre.
    pub intruder: Vec2,
    /// Every guard's centre and heading (0..=255, 0 = +x, 64 = +y).
    pub guards: Vec<(Vec2, u8)>,
}

fn centre(c: Cell) -> Vec2 {
    Vec2::new(c.x as f32 + 0.5, c.y as f32 + 0.5)
}

impl Pose {
    /// The pose after the last tick.
    pub fn of(actors: &Actors) -> Self {
        Self {
            intruder: centre(actors.intruder),
            guards: actors
                .guards
                .iter()
                .map(|g| (centre(g.cell), g.facing))
                .collect(),
        }
    }

    /// `alpha` of the way from `before` to `now` (`Frame::alpha`). Headings do not blend:
    /// a lamp turns with the tick.
    pub fn blend(before: &Pose, now: &Pose, alpha: f32) -> Self {
        let mix = |a: Vec2, b: Vec2| a + (b - a) * alpha;
        Self {
            intruder: mix(before.intruder, now.intruder),
            guards: before
                .guards
                .iter()
                .zip(&now.guards)
                .map(|(b, n)| (mix(b.0, n.0), n.1))
                .collect(),
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

/// The host's GPU side: its textures, the lamps and the light renderer.
pub struct View<'d> {
    gpu: Gpu<'d>,
    renderer: Light2dRenderer<'d>,
    colour: Texture<'d>,
    occlusion: Texture<'d>,
    target: Texture<'d>,
    lights: LightStore,
    /// Per guard, in the order of the simulation: its lamp, if the scene gives it one.
    lamps: Vec<Option<(LightId, Lamp)>>,
    /// The yard without actors, and the canvas the actors are painted onto; reused.
    ground: Vec<u8>,
    canvas: Vec<u8>,
    /// Picture size in pixels.
    size: (u32, u32),
}

impl<'d> View<'d> {
    /// Sets up the textures and lamps for a valid `scene`. `pack` is the shader pack.
    pub fn new(gpu: Gpu<'d>, pack: Vec<u8>, scene: &Scene) -> Self {
        let (_, yard) = scene.each::<Yard>().next().expect("a valid scene");
        let (w, h) = (yard.width as u32 * PX, yard.height as u32 * PX);
        let mut ground = vec![0u8; (w * h * 4) as usize];
        let mut mask = ground.clone();
        let pixels = ground.as_chunks_mut::<4>().0.iter_mut();
        for (i, (px, hole)) in pixels.zip(mask.as_chunks_mut::<4>().0).enumerate() {
            let (x, y) = (i as u32 % w / PX, i as u32 / w / PX);
            let wall = yard.walls.contains(&[x as i32, y as i32]);
            hole[3] = if wall { 255 } else { 0 };
            *px = if wall {
                [70, 62, 58, 255]
            } else if (x + y).is_multiple_of(2) {
                [150, 150, 140, 255]
            } else {
                [135, 135, 126, 255]
            };
        }
        let mut lights = LightStore::new();
        // Guards in name order, as the simulation counts them.
        let lamps = scene
            .each::<Guard>()
            .map(|(name, guard)| {
                let lamp = scene.get::<Lamp>(name)?;
                let post = Vec2::new(guard.post[0] as f32 + 0.5, guard.post[1] as f32 + 0.5);
                let desc = Self::lamp(&lamp, post, guard.facing.facing());
                Some((lights.create(&desc, &NoResources).unwrap(), lamp))
            })
            .collect();
        Self {
            gpu,
            renderer: Light2dRenderer::new(gpu, pack).unwrap(),
            colour: sampled_rgba8(gpu, w, h, &ground),
            occlusion: sampled_rgba8(gpu, w, h, &mask),
            target: texture(
                gpu,
                (w, h),
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            ),
            lights,
            lamps,
            canvas: ground.clone(),
            ground,
            size: (w, h),
        }
    }

    /// Width and height of the picture in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Paints the actors of `pose` over the ground.
    fn paint(&mut self, pose: &Pose) {
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        self.canvas.copy_from_slice(&self.ground);
        let canvas = &mut self.canvas;
        let mut disc = |at: Vec2, colour: [u8; 4]| {
            // The disc's bounding box in pixels, clipped to the picture.
            let (x0, x1) = (
                ((at.x - 0.5) * PX as f32) as i32,
                ((at.x + 0.5) * PX as f32) as i32,
            );
            let (y0, y1) = (
                ((at.y - 0.5) * PX as f32) as i32,
                ((at.y + 0.5) * PX as f32) as i32,
            );
            for y in y0.clamp(0, h)..y1.clamp(0, h) {
                for x in x0.clamp(0, w)..x1.clamp(0, w) {
                    let dx = (x as f32 + 0.5) / PX as f32 - at.x;
                    let dy = (y as f32 + 0.5) / PX as f32 - at.y;
                    if dx * dx + dy * dy < 0.4 * 0.4 {
                        let i = ((y * w + x) * 4) as usize;
                        canvas[i..i + 4].copy_from_slice(&colour);
                    }
                }
            }
        };
        for &(at, _) in &pose.guards {
            disc(at, [60, 110, 210, 255]);
        }
        disc(pose.intruder, [200, 60, 50, 255]);
    }

    /// A guard's lamp: a cone from the guard along its heading.
    fn lamp(lamp: &Lamp, at: Vec2, facing: u8) -> LightDesc {
        let mut d = LightDesc::new(LightType::Spot);
        d.position = At::new(at);
        d.height = Height::At(0.0);
        d.direction = lamp_direction(facing);
        d.radius = lamp.radius;
        d.inner_angle = lamp.inner_angle;
        d.outer_angle = lamp.outer_angle;
        d.color = Vec3::new(lamp.color[0], lamp.color[1], lamp.color[2]);
        d.intensity = lamp.intensity;
        d.source_radius = 0.15;
        d
    }

    /// Draws one frame and returns it as RGBA8 ([`size`](Self::size)). Submits and reads
    /// back: this is the host (ADR 0002).
    pub fn draw(&mut self, pose: &Pose) -> Vec<u8> {
        self.paint(pose);
        upload_pixels(self.gpu, &self.colour, &self.canvas);
        for (lamp, &(at, facing)) in self.lamps.iter().zip(&pose.guards) {
            if let Some((id, lamp)) = lamp {
                let desc = Self::lamp(lamp, at, facing);
                self.lights.update(*id, &desc, &NoResources).unwrap();
            }
        }
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
        let bytes = self.size.0 * self.size.1 * 4;
        let mut download = DownloadBuffer::new(self.gpu, bytes).unwrap();
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, &download)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let mut pixels = vec![0u8; bytes as usize];
        download.read(&mut pixels).unwrap();
        pixels
    }
}

/// Simulation ticks per second of the played scene.
pub const TICKS_PER_SECOND: u64 = 4;
/// Drawn frames per second: three frames per tick, so two of three show a blended pose.
pub const FRAMES_PER_SECOND: u64 = 12;

/// Plays `script` at [`FRAMES_PER_SECOND`] for `frames` frames and calls `show` with every
/// drawn frame. The host's whole frame loop: input, ticks, blend, draw.
pub fn play(
    view: &mut View<'_>,
    sim: Sim,
    script: Script,
    frames: u64,
    mut show: impl FnMut(u64, &[u8]),
) {
    use knorke_runtime::{FixedStep, Runtime};
    const SECOND: u64 = 1_000_000_000;
    let step = FixedStep::new(SECOND / TICKS_PER_SECOND, 4);
    let mut rt = Runtime::new(sim, knorke_replay::NoRecord, step, knorke_types::Tick::ZERO);
    // Input: the script speaks in ticks; a command goes in once, just before its tick.
    let feed = |rt: &mut Runtime<Sim>| {
        if let Some(command) = script.command(rt.next_tick().0) {
            rt.submit(command);
        }
    };
    feed(&mut rt);
    let mut before = Pose::of(&rt.sim.world.actors);
    for f in 0..frames {
        let mut now = Pose::of(&rt.sim.world.actors);
        // At most one tick per frame, so the pose before the last tick is at hand to blend.
        let due = rt.frame(SECOND / FRAMES_PER_SECOND);
        debug_assert!(due.ticks <= 1, "three frames per tick");
        if due.ticks > 0 {
            before = now;
            now = Pose::of(&rt.sim.world.actors);
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

    // <out dir> [scene file]; without a scene file the built-in yard.
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("usage: <out dir> [scene.json]"));
    let scene = match args.next() {
        Some(path) => Scene::from_text(&std::fs::read_to_string(path).unwrap()).unwrap(),
        None => yard(),
    };
    let sim = build(&scene).unwrap_or_else(|found| panic!("invalid scene: {found:#?}"));
    std::fs::create_dir_all(&out).unwrap();
    let Some((device, pack)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let mut view = View::new(Gpu::new(&device, &counter), pack, &scene);
    let (w, h) = view.size();
    let frames = END * FRAMES_PER_SECOND / TICKS_PER_SECOND;
    play(&mut view, sim, Script::BOLD, frames, |f, pixels| {
        save_png_rgba8(&out.join(format!("guard_{f:03}.png")), pixels, w, h);
    });
    println!("{frames} frames in {}", out.display());
}
