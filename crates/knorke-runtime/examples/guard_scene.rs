//! A guard, an intruder and a wall, run by the runtime.
//! `cargo run -p knorke-runtime --example guard_scene`
//!
//! The yard is data: `crates/knorke-integration/scenes/guard_yard.json`, 16 x 10 cells with
//! a wall across the middle and a gap at the top, the guard at its post west of the wall,
//! the intruder east of it. The profile `knorke_integration::stealth` builds the simulation
//! from it; this file is the host: it tells the intruder where to walk ([`Script`]) and
//! prints what happens.

pub use knorke_integration::Cell;
pub use knorke_integration::stealth::{
    CAUGHT, CHASE, Command, Event, POST, RETURN, Sim, build, build_with,
};
pub use knorke_scene::Scene;

/// The yard as text.
pub const YARD: &str = include_str!("../../knorke-integration/scenes/guard_yard.json");

/// The yard as a scene.
pub fn yard() -> Scene {
    Scene::from_text(YARD).expect("the yard reads")
}

/// The start of a run in the yard.
pub fn start() -> Sim {
    build(&yard()).expect("the yard is valid")
}

/// Where the intruder starts.
pub const HIDEOUT: Cell = Cell::new(13, 8, 0);

/// What the host makes the intruder do: walk to `visit`, and from tick `leave_at` on to
/// `refuge`.
#[derive(Clone, Copy, Debug)]
pub struct Script {
    /// Where it shows itself.
    pub visit: Cell,
    /// The tick it turns to run.
    pub leave_at: u64,
    /// Where it runs to.
    pub refuge: Cell,
}

impl Script {
    /// Stands in front of the guard until spotted, then runs for the gap: the guard cuts
    /// it off.
    pub const BOLD: Script = Script {
        visit: Cell::new(6, 5, 0),
        leave_at: 18,
        refuge: HIDEOUT,
    };

    /// The command the host submits before tick `t`.
    pub fn command(self, t: u64) -> Option<Command> {
        match t {
            2 => Some(Command::Goto(self.visit)),
            t if t == self.leave_at => Some(Command::Goto(self.refuge)),
            _ => None,
        }
    }
}

/// Ticks of the scripted scene.
pub const END: u64 = 40;

/// One line per event, for the example's output.
pub fn describe(e: Event) -> String {
    const STATES: [&str; 4] = ["Post", "Chase", "Return", "Caught"];
    match e {
        Event::Spotted { guard, at } => {
            format!("guard {guard} spots the intruder at ({}, {})", at.x, at.y)
        }
        Event::State { guard, from, to } => format!(
            "guard {guard}: {} -> {}",
            STATES[from as usize], STATES[to as usize]
        ),
    }
}

fn main() {
    use knorke_runtime::{FixedStep, Runtime};
    use knorke_types::{EventCursor, Tick};

    let step = FixedStep::new(1, 1);
    let mut rt = Runtime::new(start(), knorke_replay::NoRecord, step, Tick::ZERO);
    let mut cursor = EventCursor::default();
    for t in 0..END {
        if let Some(c) = Script::BOLD.command(t) {
            rt.submit(c);
        }
        rt.tick();
        let a = &rt.sim.world.actors;
        for &e in a.events.read(&mut cursor).since().unwrap_or_default() {
            println!("tick {t:>2}  {}", describe(e));
        }
        let g = a.guards[0].cell;
        println!(
            "tick {t:>2}  intruder ({:>2}, {})  guard ({:>2}, {})",
            a.intruder.x, a.intruder.y, g.x, g.y
        );
    }
}
