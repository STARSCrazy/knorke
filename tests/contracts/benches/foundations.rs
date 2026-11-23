//! Micro- and frame-benchmarks of the foundations. Run: `cargo bench -p knorke-contract-tests`.
//! Plain `Instant` timing, best of several runs – no benchmark framework dependency.
//! Compares against std (`HashMap::new`, platform math) on purpose.

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use knorke_contract_tests::steering::{Agent, CHUNK, ChunkOut, agents, steer};
use knorke_exec::{Rayon, ScopedThreads};
use knorke_math::{Vec2, det};
use knorke_observe::{NoProbe, Probe, Recorder, scope};
use knorke_types::{
    Column, CommandBuffer, EventQueue, Handle, HandleMap, Parallel, Serial, Slots, StableState,
    Tick, chunk_count, project,
};
use std::collections::{BTreeMap, HashMap};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn best(runs: u32, mut f: impl FnMut()) -> Duration {
    f();
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn row(name: &str, d: Duration, items: usize) {
    let per = d.as_secs_f64() * 1e9 / items as f64;
    println!(
        "{name:<52} {:>10.3} ms {:>9.2} ns/item",
        d.as_secs_f64() * 1e3,
        per
    );
}

fn sum_x(c: impl Column<knorke_math::At<knorke_math::World2, Vec2>>) -> f32 {
    c.iter().map(|p| p.raw().x).sum()
}

fn main() {
    let n = 1_000_000;
    let agents_v: Vec<Agent> = agents(n);
    println!(
        "{} logical cores, rayon threads: {}\n",
        std::thread::available_parallelism().map_or(1, |c| c.get()),
        Rayon::global().threads()
    );

    println!("-- data access ({n} agents)");
    let packed: Vec<_> = agents_v.iter().map(|a| a.pos).collect();
    row(
        "packed slice, direct loop",
        best(20, || {
            black_box(packed.iter().map(|p| p.raw().x).sum::<f32>());
        }),
        n,
    );
    row(
        "Column over packed slice",
        best(20, || {
            black_box(sum_x(&packed[..]));
        }),
        n,
    );
    row(
        "project() over host structs (no copy)",
        best(20, || {
            black_box(sum_x(project(&agents_v, |a| &a.pos)));
        }),
        n,
    );
    row(
        "host structs, direct field loop",
        best(20, || {
            black_box(agents_v.iter().map(|a| a.pos.raw().x).sum::<f32>());
        }),
        n,
    );

    println!("\n-- per-entity lookup ({n} lookups)");
    let mut slots: Slots<u32> = Slots::with_capacity(n);
    let hs: Vec<Handle<u32>> = (0..n as u32).map(|i| slots.insert(i)).collect();
    let mut hm: HandleMap<u32, u32> = HandleMap::with_index_bound(n as u32);
    let mut stable: HashMap<Handle<u32>, u32, StableState> = HashMap::default();
    let mut std_map: HashMap<Handle<u32>, u32> = HashMap::new();
    let mut ordered: BTreeMap<Handle<u32>, u32> = BTreeMap::new();
    for (i, &h) in hs.iter().enumerate() {
        hm.insert(h, i as u32);
        stable.insert(h, i as u32);
        std_map.insert(h, i as u32);
        ordered.insert(h, i as u32);
    }
    let order: Vec<Handle<u32>> = (0..n).map(|i| hs[(i * 7919) % n]).collect();
    row(
        "HandleMap::get (random order)",
        best(10, || {
            black_box(
                order
                    .iter()
                    .map(|h| *hm.get(*h).unwrap() as u64)
                    .sum::<u64>(),
            );
        }),
        n,
    );
    row(
        "BTreeMap::get (allowed in simulation code)",
        best(10, || {
            black_box(
                order
                    .iter()
                    .map(|h| *ordered.get(h).unwrap() as u64)
                    .sum::<u64>(),
            );
        }),
        n,
    );
    let small: BTreeMap<Handle<u32>, u32> = hs.iter().take(1000).map(|&h| (h, 1)).collect();
    row(
        "BTreeMap::get, 1000 entries",
        best(10, || {
            black_box(
                order
                    .iter()
                    .map(|h| *small.get(&hs[h.index() as usize % 1000]).unwrap() as u64)
                    .sum::<u64>(),
            );
        }),
        n,
    );
    row(
        "HashMap<_, _, StableState>::get (host tools only)",
        best(10, || {
            black_box(
                order
                    .iter()
                    .map(|h| *stable.get(h).unwrap() as u64)
                    .sum::<u64>(),
            );
        }),
        n,
    );
    row(
        "HashMap (std SipHash)::get",
        best(10, || {
            black_box(
                order
                    .iter()
                    .map(|h| *std_map.get(h).unwrap() as u64)
                    .sum::<u64>(),
            );
        }),
        n,
    );

    println!("\n-- seeded noise ({n} derivations)");
    row(
        "Seed::derive (two SplitMix64 rounds)",
        best(10, || {
            let seed = black_box(knorke_types::Seed(7));
            black_box((0..n as u64).fold(0u64, |acc, i| acc ^ seed.derive(i).0));
        }),
        n,
    );

    println!("\n-- communication ({n} items, buffers warm)");
    let mut q: EventQueue<u64> = EventQueue::with_capacity(n);
    let mut cur = q.cursor_at_end();
    row(
        "EventQueue push + read + retire",
        best(10, || {
            for i in 0..n as u64 {
                q.push(Tick(1), i);
            }
            black_box(q.read(&mut cur).since().unwrap().len());
            q.clear();
        }),
        n,
    );
    let mut cb: CommandBuffer<u64> = CommandBuffer::with_capacity(n);
    row(
        "CommandBuffer push + apply",
        best(10, || {
            for i in 0..n as u64 {
                cb.push(i);
            }
            let mut s = 0;
            cb.apply(|c| s += c);
            black_box(s);
        }),
        n,
    );

    println!("\n-- instrumentation (1M scopes)");
    let mut rec = Recorder::new("bench", 1, 1 << 20);
    let mut noop = NoProbe;
    row(
        "Probe::scope with NoProbe",
        best(10, || {
            for _ in 0..1_000_000 {
                let _s = noop.scope(scope!("bench", "s"));
            }
        }),
        1_000_000,
    );
    row(
        "Probe::scope with Recorder",
        best(5, || {
            for _ in 0..1_000_000 {
                let _s = rec.scope(scope!("bench", "s"));
            }
        }),
        1_000_000,
    );

    println!("\n-- parallel steering frame ({n} agents, chunk {CHUNK})");
    let mut vel = vec![Vec2::ZERO; n];
    let mut outs: Vec<ChunkOut> = (0..chunk_count(n, CHUNK))
        .map(|_| ChunkOut::default())
        .collect();
    let mut frame = |p: &dyn Fn(&mut [Vec2], &mut [ChunkOut])| {
        best(10, || {
            p(&mut vel, &mut outs);
            for o in outs.iter_mut() {
                o.spawns.clear();
                o.arrived.clear();
            }
        })
    };
    let serial = frame(&|v, o| steer(&Serial, &agents_v, v, o));
    row("Serial", serial, n);
    let rayon = Rayon::global();
    let r = frame(&|v, o| steer(&rayon, &agents_v, v, o));
    row(
        &format!(
            "Rayon (global, {} threads)  speedup {:.1}x",
            rayon.threads(),
            serial.as_secs_f64() / r.as_secs_f64()
        ),
        r,
        n,
    );
    let scoped = ScopedThreads::available();
    let s = frame(&|v, o| steer(&scoped, &agents_v, v, o));
    row(
        &format!(
            "ScopedThreads ({} threads)  speedup {:.1}x",
            scoped.threads(),
            serial.as_secs_f64() / s.as_secs_f64()
        ),
        s,
        n,
    );

    println!("\n-- deterministic math vs std (1M calls, f32)");
    let xs: Vec<f32> = (0..1_000_000).map(|i| i as f32 * 1e-4).collect();
    let n = xs.len();
    #[allow(clippy::disallowed_methods)]
    let std_pairs: [(&str, &dyn Fn(f32) -> f32); 4] = [
        ("sin_cos", &|x| {
            let (s, c) = x.sin_cos();
            s + c
        }),
        ("pow(x, 1/2.4)", &|x| x.powf(1.0 / 2.4)),
        ("atan2", &|x| x.atan2(1.0 - x)),
        ("exp", &|x| x.exp()),
    ];
    let det_fns: [&dyn Fn(f32) -> f32; 4] = [
        &|x| {
            let (s, c) = det::sin_cos(x);
            s + c
        },
        &|x| det::pow(x, 1.0 / 2.4),
        &|x| det::atan2(x, 1.0 - x),
        &|x| det::exp(x),
    ];
    for ((name, s), d) in std_pairs.into_iter().zip(det_fns) {
        let ts = best(10, || {
            black_box(xs.iter().map(|&x| s(black_box(x))).sum::<f32>());
        });
        let td = best(10, || {
            black_box(xs.iter().map(|&x| d(black_box(x))).sum::<f32>());
        });
        row(&format!("std {name}"), ts, n);
        row(
            &format!(
                "det {name}  ({:.2}x std)",
                td.as_secs_f64() / ts.as_secs_f64()
            ),
            td,
            n,
        );
    }

    println!(
        "
-- exact but possibly slow: mul_add, min/max (1M calls, f32)"
    );
    {
        let plain = best(10, || {
            black_box(
                xs.iter()
                    .map(|&x| black_box(x) * 1.000_1 + 0.5)
                    .sum::<f32>(),
            );
        });
        let fused = best(10, || {
            black_box(
                xs.iter()
                    .map(|&x| black_box(x).mul_add(1.000_1, 0.5))
                    .sum::<f32>(),
            );
        });
        row("x * a + b", plain, n);
        row(
            &format!(
                "x.mul_add(a, b)  ({:.1}x)",
                fused.as_secs_f64() / plain.as_secs_f64()
            ),
            fused,
            n,
        );
        let std_max = best(10, || {
            black_box(xs.iter().map(|&x| black_box(x).max(50.0)).sum::<f32>());
        });
        let det_max = best(10, || {
            black_box(
                xs.iter()
                    .map(|&x| det::max(black_box(x), 50.0))
                    .sum::<f32>(),
            );
        });
        row("std f32::max", std_max, n);
        row(
            &format!(
                "det::max  ({:.2}x std)",
                det_max.as_secs_f64() / std_max.as_secs_f64()
            ),
            det_max,
            n,
        );
    }

    println!(
        "
-- optional features switched off vs. absent (fingerprint world, 300 ticks)"
    );
    {
        use knorke_contract_tests::fingerprint::{World, commands};
        use knorke_observe::{Invariants, NoopSink, Sink};
        use knorke_replay::{
            NoRecord, RecordOptions, Recorder, ReplayHeader, Simulation, TickDriver,
        };
        let cmds: Vec<_> = (0..300).map(commands).collect();
        let fresh = || World::new(Serial);
        let ticks = cmds.len();
        let bare = best(5, || {
            let mut w = fresh();
            for (t, c) in cmds.iter().enumerate() {
                w.step(Tick(t as u64), c);
            }
            black_box(w.swarm.bodies.len());
        });
        row("sim.step directly", bare, ticks);
        type StepFn<'a> =
            dyn FnMut(&mut World<Serial>, Tick, &[knorke_contract_tests::fingerprint::Cmd]) + 'a;
        let drive = |d: &mut StepFn<'_>| {
            best(5, || {
                let mut w = fresh();
                for (t, c) in cmds.iter().enumerate() {
                    d(&mut w, Tick(t as u64), c);
                }
                black_box(w.swarm.bodies.len());
            })
        };
        let rel = |d: Duration| {
            format!(
                "{:+.1} %",
                (d.as_secs_f64() / bare.as_secs_f64() - 1.0) * 100.0
            )
        };
        let mut none = NoRecord;
        let d = drive(&mut |w, t, c| none.step(w, t, c));
        row(
            &format!("NoRecord (off, compile time)  {}", rel(d)),
            d,
            ticks,
        );
        let mut off: Option<Recorder<_>> = None;
        let d = drive(&mut |w, t, c| off.step(w, t, c));
        row(
            &format!("Option<Recorder> = None (off, run time)  {}", rel(d)),
            d,
            ticks,
        );
        let mut inv: Invariants<World<Serial>> = Invariants::new();
        let mut sink = NoopSink;
        let d = drive(&mut |w, t, c| {
            w.step(t, c);
            black_box(inv.run(w, t, &mut sink as &mut dyn Sink<_>));
        });
        row(&format!("+ empty Invariants::run  {}", rel(d)), d, ticks);
        for every in [60u32, 1] {
            let d = best(5, || {
                let mut w = fresh();
                let opts = RecordOptions {
                    checksum_every: every,
                    keyframe_every: 120,
                    window: Some(240),
                };
                let mut rec = Recorder::start(&w, Tick(0), ReplayHeader::default(), opts);
                for (t, c) in cmds.iter().enumerate() {
                    rec.step(&mut w, Tick(t as u64), c);
                }
                black_box(rec.log().ticks().len());
            });
            row(
                &format!("Recorder on, checksum every {every} tick(s)  {}", rel(d)),
                d,
                ticks,
            );
        }
    }

    println!("\n-- fork-join overhead (empty work)");
    let mut tiny = vec![0u8; 64];
    row(
        "Rayon for_each_chunk_mut, 64 tiny chunks",
        best(1000, || {
            rayon.for_each_chunk_mut(&mut tiny, 1, |_, c| {
                black_box(c);
            })
        }),
        1,
    );
    row(
        "ScopedThreads for_each_chunk_mut, 64 tiny chunks",
        best(100, || {
            scoped.for_each_chunk_mut(&mut tiny, 1, |_, c| {
                black_box(c);
            })
        }),
        1,
    );
}
