//! Particles and weather emitters. Particles are camera-facing squares (see `Renderer::draw_particles`).
//! Emitters keep no simulation state: particle i of an emitter is born at i / rate and its position at
//! time t is a pure function of (emitter parameters, seed, i, t), so the same time gives the same picture
//! at any frame rate.

use crate::grid::GridMap;
use crate::math::Vec3;

/// Ignores light (sparks, glows).
pub const PARTICLE_EMISSIVE: u32 = 1;
/// Adds its colour to what is behind instead of covering it.
pub const PARTICLE_ADDITIVE: u32 = 2;
/// Particles drawn per frame at most (pr_draw_particles and emitters together); later ones are dropped.
pub const PARTICLE_BUDGET: usize = 4096;
/// Particles one emitter generates per frame at most (before indoor culling and the frame budget).
pub const EMIT_MAX: usize = 1 << 16;

/// Sparks rising from a point.
pub const EMIT_SPARKS: i32 = 0;
/// Smoke rising and spreading.
pub const EMIT_SMOKE: i32 = 1;
/// Rain falling in a box around a centre.
pub const EMIT_RAIN: i32 = 2;
/// Snow falling in a box around a centre.
pub const EMIT_SNOW: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
/// A camera-facing square in the world.
pub struct Particle {
    /// Centre in world space.
    pub position: Vec3,
    /// Edge length in world units.
    pub size: f32,
    /// 0xRRGGBB; tints the texture.
    pub color: u32,
    /// Texture id, -1 = solid square.
    pub texture: i32,
    /// `PARTICLE_*` flags.
    pub flags: u32,
    /// 0..1; below 1 (or with PARTICLE_ADDITIVE) the particle is blended.
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// A persistent particle source (see `Renderer::create_emitter`).
pub struct Emitter {
    /// `EMIT_*` kind.
    pub kind: i32,
    /// Sparks and smoke: source; rain and snow: centre of the box they fill (usually the camera).
    pub position: Vec3,
    /// Box size; sparks and smoke start anywhere inside it.
    pub box_size: Vec3,
    /// Particles born per second.
    pub rate: f32,
    /// Rise or fall speed in world units per second; <= 0 = default of the kind.
    pub speed: f32,
    /// Particle size in world units; <= 0 = default of the kind.
    pub size: f32,
    /// Particle colour 0xRRGGBB.
    pub color: u32,
    /// Texture id, -1 = solid squares.
    pub texture: i32,
    /// Horizontal drift in world units per second.
    pub wind: [f32; 2],
    /// Seed of the particle sequence.
    pub seed: u32,
}

/// A generated particle plus its height factor (rain streaks are taller than wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitted {
    /// The particle.
    pub particle: Particle,
    /// Height divided by width.
    pub stretch: f32,
}

/// Number in [0, 1) from (seed, index, channel); integer hash (splitmix64), the same everywhere.
pub fn hash01(seed: u32, i: i64, k: u32) -> f32 {
    let mut z = ((seed as u64) << 32 ^ (k as u64))
        .wrapping_add((i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Default speed, size and lifetime of a kind (lifetime of rain and snow: time to fall through the box).
fn defaults(e: &Emitter) -> Option<(f32, f32, f32)> {
    let or = |v: f32, d: f32| if v > 0.0 { v } else { d };
    let fall = |speed: f32| {
        if e.box_size.y > 0.0 {
            e.box_size.y / speed
        } else {
            2.0
        }
    };
    Some(match e.kind {
        EMIT_SPARKS => (or(e.speed, 1.5), or(e.size, 0.04), 1.0),
        EMIT_SMOKE => (or(e.speed, 0.4), or(e.size, 0.25), 3.0),
        EMIT_RAIN => {
            let s = or(e.speed, 8.0);
            (s, or(e.size, 0.02), fall(s))
        }
        EMIT_SNOW => {
            let s = or(e.speed, 1.0);
            (s, or(e.size, 0.05), fall(s))
        }
        _ => return None,
    })
}

/// Checked parameters of an emitter: (speed, size, lifetime); None if it can emit nothing.
fn params(e: &Emitter) -> Option<(f32, f32, f32)> {
    let (speed, size, life) = defaults(e)?;
    let ok = e.rate.is_finite()
        && e.rate > 0.0
        && speed.is_finite()
        && size.is_finite()
        && life.is_finite()
        && life > 0.0
        && finite(e.position)
        && finite(e.box_size)
        && e.wind[0].is_finite()
        && e.wind[1].is_finite();
    ok.then_some((speed, size, life))
}

/// The living particles of `e` at `time`, oldest first, at most `max` (the newest are dropped).
pub fn emit(e: &Emitter, time: f64, max: usize, out: &mut Vec<Emitted>) {
    let Some((lo, hi)) = emit_range(e, time) else {
        return;
    };
    let mut i = lo;
    while i <= hi && out.len() < max {
        if let Some(p) = emit_one(e, time, i) {
            out.push(p);
        }
        i += 1;
    }
}

/// Indices of the particles of `e` that may be alive at `time` (inclusive), oldest first; None for
/// unusable parameters. Feed them to `emit_one`.
pub fn emit_range(e: &Emitter, time: f64) -> Option<(i64, i64)> {
    let (_, _, life) = params(e)?;
    if !time.is_finite() {
        return None;
    }
    let rate = e.rate as f64;
    let (hi, lo) = (time * rate, (time - life as f64) * rate);
    if hi.abs() > 1e15 || lo.abs() > 1e15 {
        return None;
    }
    Some((lo.floor() as i64 + 1, hi.floor() as i64))
}

/// Particle `i` of `e` (born at i / rate) at `time`; None before its birth, after its lifetime or for
/// unusable parameters.
pub fn emit_one(e: &Emitter, time: f64, i: i64) -> Option<Emitted> {
    let (speed, size, life) = params(e)?;
    let age = time - i as f64 / e.rate as f64;
    if age.is_nan() || age < 0.0 || age >= life as f64 {
        return None;
    }
    let age = age as f32;
    let t = (age / life).min(1.0);
    let h = |k: u32| hash01(e.seed, i, k);
    let bx = Vec3::new(
        e.box_size.x.max(0.0),
        e.box_size.y.max(0.0),
        e.box_size.z.max(0.0),
    );
    let corner = e.position - bx * 0.5;
    let start = corner + Vec3::new(h(0) * bx.x, h(1) * bx.y, h(2) * bx.z);
    let wind = Vec3::new(e.wind[0], 0.0, e.wind[1]) * age;
    let base = Particle {
        position: start,
        size,
        color: e.color,
        texture: e.texture,
        flags: 0,
        alpha: 1.0,
    };
    let (particle, stretch) = match e.kind {
        EMIT_SPARKS => {
            let v = Vec3::new(
                (h(3) - 0.5) * speed * 0.5,
                speed * (0.6 + 0.4 * h(4)),
                (h(5) - 0.5) * speed * 0.5,
            );
            let pos = start + v * age + wind + Vec3::new(0.0, -0.6 * age * age, 0.0);
            (
                Particle {
                    position: pos,
                    flags: PARTICLE_EMISSIVE | PARTICLE_ADDITIVE,
                    alpha: 1.0 - t,
                    ..base
                },
                1.0,
            )
        }
        EMIT_SMOKE => {
            let drift = Vec3::new(
                (h(3) - 0.5) * 0.3,
                speed * (0.8 + 0.4 * h(4)),
                (h(5) - 0.5) * 0.3,
            ) * age;
            (
                Particle {
                    position: start + drift + wind,
                    size: size * (1.0 + 2.0 * t),
                    alpha: 0.6 * (1.0 - t),
                    ..base
                },
                1.0,
            )
        }
        _ => {
            // rain / snow: start at the top of the box and fall; wrap sideways so the box stays filled
            let wrap = |v: f32, lo: f32, len: f32| {
                if len > 0.0 {
                    lo + (v - lo).rem_euclid(len)
                } else {
                    lo
                }
            };
            let sway = if e.kind == EMIT_SNOW {
                0.15 * (age * 1.7 + h(6) * std::f32::consts::TAU).sin()
            } else {
                0.0
            };
            let x = wrap(start.x + wind.x + sway, corner.x, bx.x);
            let z = wrap(start.z + wind.z, corner.z, bx.z);
            let y = corner.y + bx.y - speed * age;
            let rain = e.kind == EMIT_RAIN;
            (
                Particle {
                    position: Vec3::new(x, y, z),
                    alpha: if rain { 0.75 } else { 1.0 },
                    ..base
                },
                if rain { 6.0 } else { 1.0 },
            )
        }
    };
    Some(Emitted { particle, stretch })
}

/// True if a rain or snow particle at `p` is covered in its column: below a textured ceiling or a
/// textured floor of a higher span, or inside the solid (below the lowest floor, between spans, or a
/// column without spans). Above the topmost span it is open air. Outside the map nothing is culled.
pub fn indoors(grid: &GridMap, p: Vec3) -> bool {
    let s = grid.cell_size;
    if s.is_nan() || s <= 0.0 {
        return false;
    }
    let (cx, cz) = ((p.x / s).floor() as i64, (p.z / s).floor() as i64);
    if grid.cell(cx, cz).is_none() {
        return false;
    }
    let mut first = true;
    for sp in grid.spans(cx, cz) {
        let c = grid.cell_at(cx, cz, sp.level).unwrap();
        let (floor, ceil) = sp.heights(p.x / s - cx as f32, p.z / s - cz as f32);
        if c.ceiling_texture >= 0 && p.y < ceil {
            return true; // a roof overhead
        }
        // below this span's floor: the lowest floor, a textured floor, or inside the solid between spans
        if p.y < floor && (first || c.floor_texture >= 0 || !in_open_span(grid, cx, cz, p)) {
            return true;
        }
        first = false;
    }
    first // a column without spans is solid
}

/// p is inside one of the open spans of its column.
fn in_open_span(grid: &GridMap, cx: i64, cz: i64, p: Vec3) -> bool {
    grid.span_containing(cx, cz, p).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sparks() -> Emitter {
        Emitter {
            kind: EMIT_SPARKS,
            position: Vec3::new(0.0, 0.0, 0.0),
            box_size: Vec3::new(0.2, 0.0, 0.2),
            rate: 40.0,
            speed: 0.0,
            size: 0.0,
            color: 0xffaa33,
            texture: -1,
            wind: [0.0, 0.0],
            seed: 7,
        }
    }

    #[test]
    fn emit_is_pure() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        emit(&sparks(), 2.5, 1000, &mut a);
        emit(&sparks(), 2.5, 1000, &mut b);
        assert!(!a.is_empty());
        assert_eq!(a, b);
    }

    #[test]
    fn emit_bad_input_is_empty() {
        let mut v = Vec::new();
        emit(
            &Emitter {
                rate: f32::NAN,
                ..sparks()
            },
            1.0,
            100,
            &mut v,
        );
        emit(
            &Emitter {
                kind: 99,
                ..sparks()
            },
            1.0,
            100,
            &mut v,
        );
        emit(&sparks(), f64::INFINITY, 100, &mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn hash_in_range() {
        for i in -50..50 {
            let h = hash01(3, i * 1_000_003, 2);
            assert!((0.0..1.0).contains(&h));
        }
    }
}
