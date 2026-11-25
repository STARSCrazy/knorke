//! The stealth profile as a picture (features `stealth` + `gpu`): the host's side of a
//! frame (ADR 0009, phases 3 and 4).
//!
//! After the ticks of a frame the host takes a [`Pose`] from the actors, may blend it with
//! the pose before the last tick ([`Pose::blend`], weight `Frame::alpha`) and hands it to
//! [`View::draw`]: the yard is painted on the CPU (floor, walls, a disc per actor), the
//! walls also into the occlusion mask, and the 2D light adds every guard's lamp (component
//! `knorke.stealth.lamp/1`), which turns with the guard and stops at a wall.
//!
//! [`View`] plays the host (ADR 0002): it owns its textures, submits and reads back. The
//! simulation knows nothing of this module.

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags, SDL_GPUTextureUsageFlags,
};
use knorke_gpu::{CommandBuffer, DownloadBuffer, Gpu, Texture, TextureDesc, UploadBuffer};
use knorke_grid::Cell;
use knorke_light2d::light::{Height, LightDesc, LightId, LightType};
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};
use knorke_scene::Scene;
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use crate::stealth::{Actors, Guard, Lamp, Yard};

/// The scene is not a valid stealth scene, or a lamp the light store rejects.
pub const INVALID_SCENE: DiagnosticCode = DiagnosticCode::new("knorke.stealth.view.invalid_scene");

fn invalid(what: impl core::fmt::Debug) -> Diagnostic {
    Diagnostic::error(INVALID_SCENE, format!("{what:?}"))
}

/// Fills `texture` with `pixels`; submitted and waited for.
fn upload(gpu: Gpu<'_>, texture: &Texture<'_>, pixels: &[u8]) -> Result<(), Diagnostic> {
    let mut up = UploadBuffer::new(
        gpu,
        SDL_GPUBufferUsageFlags(0),
        pixels.len() as u32,
        "pixels",
    );
    up.reserve(pixels.len() as u64)?;
    up.map()?.bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device())?;
    cmd.copy_pass()?
        .upload_to_texture(&up, texture.texture_ref(), false)?;
    cmd.submit_and_wait()
}

fn rgba8<'d>(
    gpu: Gpu<'d>,
    (w, h): (u32, u32),
    usage: SDL_GPUTextureUsageFlags,
    pixels: Option<&[u8]>,
) -> Result<Texture<'d>, Diagnostic> {
    let desc = TextureDesc::new(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, usage);
    let texture = Texture::new(gpu, desc)?;
    if let Some(pixels) = pixels {
        upload(gpu, &texture, pixels)?;
    }
    Ok(texture)
}

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
    ///
    /// # Errors
    /// [`INVALID_SCENE`], or what the GPU reports.
    pub fn new(gpu: Gpu<'d>, pack: Vec<u8>, scene: &Scene) -> Result<Self, Diagnostic> {
        let Some((_, yard)) = scene.each::<Yard>().next() else {
            return Err(invalid("no yard"));
        };
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
        let mut lamps = Vec::new();
        for (name, guard) in scene.each::<Guard>() {
            lamps.push(match scene.get::<Lamp>(name) {
                Some(lamp) => {
                    let post = Vec2::new(guard.post[0] as f32 + 0.5, guard.post[1] as f32 + 0.5);
                    let desc = Self::lamp(&lamp, post, guard.facing.facing());
                    Some((lights.create(&desc, &NoResources).map_err(invalid)?, lamp))
                }
                None => None,
            });
        }
        Ok(Self {
            gpu,
            renderer: Light2dRenderer::new(gpu, pack)?,
            colour: rgba8(gpu, (w, h), SDL_GPU_TEXTUREUSAGE_SAMPLER, Some(&ground))?,
            occlusion: rgba8(gpu, (w, h), SDL_GPU_TEXTUREUSAGE_SAMPLER, Some(&mask))?,
            target: rgba8(gpu, (w, h), SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, None)?,
            lights,
            lamps,
            canvas: ground.clone(),
            ground,
            size: (w, h),
        })
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
    ///
    /// # Errors
    /// What the GPU reports.
    pub fn draw(&mut self, pose: &Pose) -> Result<Vec<u8>, Diagnostic> {
        self.paint(pose);
        upload(self.gpu, &self.colour, &self.canvas)?;
        for (lamp, &(at, facing)) in self.lamps.iter().zip(&pose.guards) {
            if let Some((id, lamp)) = lamp {
                let desc = Self::lamp(lamp, at, facing);
                self.lights
                    .update(*id, &desc, &NoResources)
                    .map_err(invalid)?;
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
        let prepared = self.renderer.prepare(&frame, &stores, &settings)?;
        let mut cmd = CommandBuffer::acquire(self.gpu.device())?;
        self.renderer
            .record(&mut cmd, &frame, &settings, prepared)?;
        let bytes = self.size.0 * self.size.1 * 4;
        let mut download = DownloadBuffer::new(self.gpu, bytes)?;
        cmd.copy_pass()?.download_texture(frame.target, &download)?;
        cmd.submit_and_wait()?;
        let mut pixels = vec![0u8; bytes as usize];
        download.read(&mut pixels)?;
        Ok(pixels)
    }
}
