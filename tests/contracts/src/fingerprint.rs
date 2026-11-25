//! Cross-platform determinism fingerprint (ADR 0011).
//!
//! A float-heavy simulation (trigonometry, exp, pow, sqrt, geometry, seeded noise, parallel
//! chunks, slot reuse) runs a fixed number of ticks with fixed commands. The result is a
//! checksum chain over the per-module snapshot checksums of every tick. CI runs it on
//! Windows, Linux, macOS (ARM) and WASM; all must produce the frozen value.

use knorke_math::{Aabb2, Vec2, det};
use knorke_replay::Simulation;
use knorke_types::{
    Checksum, Handle, Parallel, Reader, Seed, Slots, Snapshot, SnapshotBuf, SnapshotError,
    SnapshotValue, StableHasher, Tick, Writer, by_fields, by_tag, restore,
};

/// One moving body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Body {
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Heading in radians.
    pub heading: f32,
}

by_fields! { Body { pos, vel, heading } }

/// Commands of the fingerprint scenario.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    /// Spawn `n` bodies around a point.
    Burst { at: Vec2, n: u16 },
    /// Move the shared target.
    Target(Vec2),
    /// Remove every k-th body (slot reuse).
    Thin(u16),
}

by_tag! { Cmd, SnapshotError::Invalid("cmd") => {
    0 => Burst { at, n },
    1 => Target(t),
    2 => Thin(k),
} }

/// Simulation state (one snapshot section).
#[derive(Default)]
pub struct Swarm {
    /// Seed of the noise.
    pub seed: Seed,
    /// Bodies.
    pub bodies: Slots<Body>,
    /// Shared target.
    pub target: Vec2,
}

impl Snapshot for Swarm {
    const ID: &'static str = "contract.swarm";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.seed);
        w.put(&self.bodies);
        w.put(&self.target);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.seed = r.get()?;
        self.bodies.get_into(r)?;
        self.target = r.get()?;
        Ok(())
    }
}

const WORLD: Aabb2 = Aabb2 {
    min: Vec2::new(-500.0, -500.0),
    max: Vec2::new(500.0, 500.0),
};
const CHUNK: usize = 256;

/// The fingerprint simulation, generic over the executor.
pub struct World<P> {
    /// State.
    pub swarm: Swarm,
    par: P,
    scratch: Vec<(Handle<Body>, Body)>,
}

impl<P: Parallel> World<P> {
    /// Empty world using executor `par`.
    pub fn new(par: P) -> Self {
        Self {
            swarm: Swarm {
                seed: Seed(0x5EED),
                ..Default::default()
            },
            par,
            scratch: Vec::new(),
        }
    }
}

impl<P: Parallel> Simulation for World<P> {
    type Command = Cmd;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.swarm);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        restore(&mut Reader::new(bytes), &mut self.swarm)
    }

    fn step(&mut self, tick: Tick, commands: &[Cmd]) {
        let s = &mut self.swarm;
        for c in commands {
            match *c {
                Cmd::Burst { at, n } => {
                    for i in 0..n {
                        let a = i as f32 * 0.618_034 * core::f32::consts::TAU;
                        let (sn, cs) = det::sin_cos(a);
                        let r = det::pow(1.0 + i as f32, 0.75);
                        s.bodies.insert(Body {
                            pos: at + Vec2::new(cs, sn) * r,
                            vel: Vec2::ZERO,
                            heading: a,
                        });
                    }
                }
                Cmd::Target(t) => s.target = t,
                Cmd::Thin(k) => {
                    let doomed: Vec<_> = s
                        .bodies
                        .iter()
                        .map(|(h, _)| h)
                        .step_by(k.max(1) as usize)
                        .collect();
                    for h in doomed {
                        s.bodies.remove(h);
                    }
                }
            }
        }
        // Parallel update over a packed copy (bodies live in slots; chunking is by index).
        self.scratch.clear();
        self.scratch.extend(s.bodies.iter().map(|(h, b)| (h, *b)));
        let target = s.target;
        let noise = s.seed.derive(tick.0);
        self.par
            .for_each_chunk_mut(&mut self.scratch, CHUNK, |ci, chunk| {
                for (k, (_, b)) in chunk.iter_mut().enumerate() {
                    let i = (ci * CHUNK + k) as u64;
                    let jitter = (noise.derive(i).0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
                    let to = target - b.pos;
                    let want = to.angle();
                    let mut turn = want - b.heading;
                    turn = det::atan2(det::sin(turn), det::cos(turn));
                    b.heading += turn * 0.1 + jitter * 0.05;
                    let speed = 2.0 * (1.0 - det::exp(-to.length() / 50.0));
                    b.vel = b.vel * 0.9 + Vec2::from_angle(b.heading) * speed * 0.1;
                    b.pos += b.vel;
                    if !WORLD.contains(b.pos) {
                        b.pos = WORLD.closest_point(b.pos);
                        b.vel = -b.vel * 0.5;
                    }
                }
            });
        for (h, b) in &self.scratch {
            if let Some(slot) = s.bodies.get_mut(*h) {
                *slot = *b;
            }
        }
    }
}

/// Commands of tick `t`.
pub fn commands(t: u64) -> Vec<Cmd> {
    let mut v = Vec::new();
    if t.is_multiple_of(40) {
        let a = t as f32 * 0.05;
        v.push(Cmd::Burst {
            at: Vec2::new(det::cos(a), det::sin(a)) * 300.0,
            n: 150,
        });
    }
    if t.is_multiple_of(97) {
        v.push(Cmd::Target(Vec2::new(
            (t % 7) as f32 * 60.0 - 180.0,
            (t % 5) as f32 * 70.0 - 140.0,
        )));
    }
    if t % 61 == 30 {
        v.push(Cmd::Thin(5));
    }
    v
}

/// Runs the scenario for `ticks` ticks and returns the checksum chain over all per-tick
/// snapshot checksums.
pub fn run<P: Parallel>(par: P, ticks: u64) -> Checksum {
    use core::hash::Hasher;
    let mut world = World::new(par);
    let mut buf = SnapshotBuf::new();
    let mut chain = StableHasher::default();
    for t in 0..ticks {
        world.step(Tick(t), &commands(t));
        buf.clear();
        world.save(&mut buf);
        chain.write_u64(buf.checksum().0);
    }
    Checksum(chain.finish())
}
