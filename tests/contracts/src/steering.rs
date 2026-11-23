//! Shared test scenario: a navigation-like system steering many agents in parallel chunks.
//! Used by the contract tests and benchmarks; not shipped.

use knorke_math::{At, Vec2, World2, det};
use knorke_types::{CommandBuffer, Handle, Parallel, Seed, Slots};

/// A host entity.
pub struct Agent {
    pub id: Handle<Agent>,
    pub pos: At<World2, Vec2>,
    pub goal: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Command emitted by the steering system.
pub enum Spawn {
    Marker { at: Vec2, by: Handle<Agent> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Event emitted by the steering system.
pub enum Arrived {
    Agent(Handle<Agent>),
}

/// Chunk length (fixed, independent of thread count).
pub const CHUNK: usize = 1024;

/// Per-chunk scratch: allocated once, reused every frame.
#[derive(Default)]
pub struct ChunkOut {
    pub spawns: CommandBuffer<Spawn>,
    pub arrived: Vec<Arrived>,
}

/// `n` agents at deterministic pseudo-random positions.
pub fn agents(n: usize) -> Vec<Agent> {
    let mut ids: Slots<()> = Slots::with_capacity(n);
    let seed = Seed(7);
    (0..n)
        .map(|i| {
            let r = seed.derive(i as u64).0;
            let x = (r & 0xFFFF) as f32 / 64.0;
            let y = ((r >> 16) & 0xFFFF) as f32 / 64.0;
            Agent {
                id: ids.insert(()).cast(),
                pos: At::new(Vec2::new(x, y)),
                goal: Vec2::new(512.0, 512.0),
            }
        })
        .collect()
}

/// One steering pass. Reads `agents`, writes `velocities` (same index), fills per-chunk outputs.
pub fn steer(
    par: &impl Parallel,
    agents: &[Agent],
    velocities: &mut [Vec2],
    outs: &mut [ChunkOut],
) {
    assert_eq!(agents.len(), velocities.len());
    par.for_each_chunk_mut(velocities, CHUNK, |ci, vel| {
        let base = ci * CHUNK;
        for (k, v) in vel.iter_mut().enumerate() {
            let a = &agents[base + k];
            let to_goal = a.goal - a.pos.raw();
            // Some real float work per agent so threads have something to chew on.
            let mut d = to_goal.normalize_or_zero();
            for _ in 0..8 {
                d = (d + d.perp() * 0.01).normalize_or_zero();
            }
            *v = d * det::min(to_goal.length(), 4.0);
        }
    });
    par.for_each_chunk_with(agents, CHUNK, outs, |_, chunk, out| {
        for a in chunk {
            if a.pos.raw().distance(a.goal) < 40.0 {
                out.arrived.push(Arrived::Agent(a.id));
                out.spawns.push(Spawn::Marker {
                    at: a.goal,
                    by: a.id,
                });
            }
        }
    });
}
