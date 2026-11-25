//! A guard, an intruder and a wall, run by the runtime.
//! `cargo run -p knorke-runtime --example guard_scene`
//!
//! A square yard of 16 x 10 cells with a wall across the middle and a gap at the top. The
//! guard stands at its post west of the wall and looks east; the intruder starts east of
//! it. The host tells the intruder where to walk ([`Command::Goto`]); nothing else enters
//! the simulation from outside.
//!
//! One tick runs the table [`SYSTEMS`] in order: the intruder walks (navigation), the guard
//! looks (perception), decides (state machine) and walks (navigation). What happened goes
//! into an [`EventQueue`] the host reads with its own cursor.

use knorke_integration::decide::group::heading;
use knorke_integration::decide::state::{StateMachine, Transition};
use knorke_integration::perception::{
    Attention, Budget as SightBudget, GridSight, PerceptKind, World as SenseWorld, cell_centre,
};
use knorke_integration::prelude::*;
use knorke_observe::{NoProbe, Probe, Scope};
use knorke_replay::Simulation;
use knorke_runtime::{System, run_systems};
use knorke_types::{
    EventQueue, Reader, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, by_tag,
    restore,
};

/// State: at the post.
pub const POST: u32 = 0;
/// State: after the intruder.
pub const CHASE: u32 = 1;
/// State: walking back to the post.
pub const RETURN: u32 = 2;
/// State: next to the intruder, who stays where it is from now on.
pub const CAUGHT: u32 = 3;

const F_SEES: u64 = 1;
const F_LOST: u64 = 2;
const F_HOME: u64 = 4;
const F_REACH: u64 = 8;

/// Fine units per cell.
const CELL: i32 = 256;
const WIDTH: i32 = 16;
const HEIGHT: i32 = 10;
/// The wall's column; it leaves rows 0 and 1 open.
const WALL_X: i32 = 8;
const GUARD_POST: Cell = Cell::new(3, 5, 0);
/// Where the intruder starts.
pub const HIDEOUT: Cell = Cell::new(13, 8, 0);
/// Heading east.
const EAST: u8 = 0;
/// Ticks without a sighting before the guard gives up.
const LOST_AFTER: u32 = 4;
/// Ticks an event stays readable.
const KEEP_EVENTS: u64 = 8;
const INTRUDER: u32 = 99;

/// Input from outside the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The intruder walks to this cell, one cell per tick.
    Goto(Cell),
}

by_tag! { Command, SnapshotError::Invalid("guard scene command tag") => {
    0 => Goto(cell),
} }

/// What happened in a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The guard recognised the intruder at this cell.
    Spotted(Cell),
    /// The guard's state changed.
    State {
        /// The state it left.
        from: u32,
        /// The state it entered.
        to: u32,
    },
}

by_tag! { Event, SnapshotError::Invalid("guard scene event tag") => {
    0 => Spotted(cell),
    1 => State { from, to },
} }

/// Everything a tick changes. One snapshot section.
pub struct Actors {
    /// The intruder's cell.
    pub intruder: Cell,
    goal: Cell,
    /// The guard's cell.
    pub guard: Cell,
    /// The guard's heading (0..=255, 0 = +x, 64 = +y).
    pub facing: u8,
    /// The guard's state ([`POST`], [`CHASE`], [`RETURN`], [`CAUGHT`]).
    pub sm: StateMachine,
    aware: Awareness,
    attention: Attention,
    /// Where and when the guard last saw the intruder.
    clue: Option<(Cell, u32)>,
    /// What happened, for the host.
    pub events: EventQueue<Event>,
}

impl Snapshot for Actors {
    const ID: &'static str = "demo.guard_scene.actors";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.intruder);
        w.put(&self.goal);
        w.put(&self.guard);
        w.put(&self.facing);
        w.put(&self.sm);
        w.put(&self.aware);
        w.put(&self.attention);
        w.put(&self.clue);
        w.put(&self.events);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.intruder = r.get()?;
        self.goal = r.get()?;
        self.guard = r.get()?;
        self.facing = r.get()?;
        self.sm.get_into(r)?;
        self.aware = r.get()?;
        self.attention = r.get()?;
        self.clue = r.get()?;
        self.events.get_into(r)?;
        // The walking systems hand these cells to the path search.
        let inside =
            |c: Cell| c.level == 0 && (0..WIDTH).contains(&c.x) && (0..HEIGHT).contains(&c.y);
        if [self.intruder, self.goal, self.guard]
            .into_iter()
            .all(inside)
        {
            Ok(())
        } else {
            Err(SnapshotError::Invalid("guard scene: actor off the grid"))
        }
    }
}

/// The yard and who is in it.
pub struct Yard {
    /// The walls. Fixed for the whole run, so not part of a snapshot.
    grid: DenseGrid,
    senses: SenseWorld,
    profile: SenseProfile,
    /// The changing state.
    pub actors: Actors,
    /// Shared by both walkers. Every search starts fresh; the snapshot keeps it anyway, so
    /// a restored run cannot depend on what an earlier search left behind.
    nav: Navigator,
    /// Scratch of the looking system, empty between ticks.
    percepts: Vec<Percept>,
}

impl Yard {
    fn new() -> Self {
        let mut grid = DenseGrid::new(WIDTH, HEIGHT);
        for y in 2..HEIGHT {
            grid.set(WALL_X, y, DenseGrid::BLOCKED);
        }
        let mut sm = StateMachine::new(4, POST).unwrap();
        // Catching comes first: a caught intruder is still seen.
        for (from, to, when) in [
            (CHASE, CAUGHT, F_REACH),
            (POST, CHASE, F_SEES),
            (RETURN, CHASE, F_SEES),
            (CHASE, RETURN, F_LOST),
            (RETURN, POST, F_HOME),
        ] {
            sm.add_transition(Transition::new(from, to, when)).unwrap();
        }
        Self {
            grid,
            senses: SenseWorld::square(CELL),
            profile: SenseProfile::guard(CELL),
            actors: Actors {
                intruder: HIDEOUT,
                goal: HIDEOUT,
                guard: GUARD_POST,
                facing: EAST,
                sm,
                aware: Awareness::default(),
                attention: Attention::default(),
                clue: None,
                events: EventQueue::new(),
            },
            nav: Navigator::new(
                Topology::Square(GridRules::preset(Preset::Octile)),
                (WIDTH * HEIGHT) as usize,
            ),
            percepts: Vec::new(),
        }
    }

    /// The next cell on the way from `from` to `goal` (`from` when there is none).
    fn next_cell(&mut self, from: Cell, goal: Cell) -> Cell {
        self.nav
            .find_path(&mut self.grid, from, &[goal], Budget::UNLIMITED);
        self.nav.path().get(1).copied().unwrap_or(from)
    }
}

fn pos(c: Cell) -> Pos {
    [c.x, c.y, c.level]
}

fn walk_intruder(yard: &mut Yard, _: Tick) {
    if yard.actors.sm.state() == CAUGHT {
        return;
    }
    let (from, goal) = (yard.actors.intruder, yard.actors.goal);
    yard.actors.intruder = yard.next_cell(from, goal);
}

fn guard_looks(yard: &mut Yard, tick: Tick) {
    let now = tick.0 as u32;
    let a = &mut yard.actors;
    let eye = Eye {
        id: 0,
        point: cell_centre(&yard.senses, a.guard),
        facing: a.facing,
        profile: &yard.profile,
        stats: Stats::NORMAL,
        disguise: Default::default(),
    };
    let target = Target {
        id: INTRUDER,
        point: cell_centre(&yard.senses, a.intruder),
        size: 256,
        conspicuity: 256,
        ..Target::default()
    };
    yard.percepts.clear();
    look(
        &yard.senses,
        &mut GridSight::square(&yard.grid),
        now,
        &eye,
        &[target],
        core::slice::from_mut(&mut a.aware),
        &mut a.attention,
        SightBudget {
            checks: 1,
            keep: 1,
            instant_cap: 0,
        },
        &mut yard.percepts,
    );
    for p in &yard.percepts {
        if p.event == PerceptKind::Recognized {
            a.events.push(tick, Event::Spotted(a.intruder));
        }
    }
    // Recognised and in view: the guard knows where the intruder is right now.
    if a.aware.stage() >= 2 && a.aware.in_view() {
        a.clue = Some((a.intruder, now));
    }
}

fn guard_decides(yard: &mut Yard, tick: Tick) {
    let now = tick.0 as u32;
    let a = &mut yard.actors;
    let mut flags = 0;
    match a.clue {
        Some((_, at)) if at == now => flags |= F_SEES,
        Some((_, at)) if now - at >= LOST_AFTER => flags |= F_LOST,
        _ => {}
    }
    if a.guard == GUARD_POST {
        flags |= F_HOME;
    }
    // Next to the intruder the guard has it.
    if (a.intruder.x - a.guard.x).abs() <= 1 && (a.intruder.y - a.guard.y).abs() <= 1 {
        flags |= F_REACH;
    }
    if let Some(change) = a.sm.update(now, flags) {
        a.events.push(
            tick,
            Event::State {
                from: change.from,
                to: change.to,
            },
        );
        if change.to == RETURN {
            a.clue = None;
        }
    }
}

fn walk_guard(yard: &mut Yard, _: Tick) {
    let a = &yard.actors;
    let (state, from) = (a.sm.state(), a.guard);
    let goal = match (state, a.clue) {
        (CAUGHT, _) => return,
        (CHASE, Some((seen, _))) => seen,
        _ => GUARD_POST,
    };
    let to = yard.next_cell(from, goal);
    let a = &mut yard.actors;
    a.guard = to;
    if to != from {
        a.facing = heading(Metric::Chebyshev, pos(from), pos(to)) * 32;
    } else if state == POST {
        a.facing = EAST;
    }
}

static WALK_INTRUDER: Scope = Scope::new("demo.guard_scene", "walk_intruder");
static GUARD_LOOKS: Scope = Scope::new("demo.guard_scene", "guard_looks");
static GUARD_DECIDES: Scope = Scope::new("demo.guard_scene", "guard_decides");
static WALK_GUARD: Scope = Scope::new("demo.guard_scene", "walk_guard");

/// The schedule of one tick.
pub static SYSTEMS: [System<Yard>; 4] = [
    System {
        scope: &WALK_INTRUDER,
        run: walk_intruder,
    },
    System {
        scope: &GUARD_LOOKS,
        run: guard_looks,
    },
    System {
        scope: &GUARD_DECIDES,
        run: guard_decides,
    },
    System {
        scope: &WALK_GUARD,
        run: walk_guard,
    },
];

/// The simulation: the yard plus the probe its systems report to ([`NoProbe`]: none).
pub struct Scene<P = NoProbe> {
    /// The world the systems work on.
    pub yard: Yard,
    /// Sees every system of every tick.
    pub probe: P,
}

impl<P> Scene<P> {
    /// The start of the scene.
    pub fn new(probe: P) -> Self {
        Self {
            yard: Yard::new(),
            probe,
        }
    }
}

impl<P: Probe> Simulation for Scene<P> {
    type Command = Command;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.yard.actors);
        out.save(&self.yard.nav);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.yard.actors)?;
        restore(&mut r, &mut self.yard.nav)?;
        r.finish("demo.guard_scene")
    }

    fn step(&mut self, tick: Tick, commands: &[Command]) {
        // Sync point: outside input changes the world here and nowhere else.
        for &Command::Goto(cell) in commands {
            if cell.level == 0 && self.yard.grid.open(cell.x, cell.y) {
                self.yard.actors.goal = cell;
            }
        }
        self.yard.actors.events.keep_ticks(tick, KEEP_EVENTS);
        run_systems(&mut self.yard, tick, &SYSTEMS, &mut self.probe);
    }
}

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
        Event::Spotted(c) => format!("guard spots the intruder at ({}, {})", c.x, c.y),
        Event::State { from, to } => format!(
            "guard: {} -> {}",
            STATES[from as usize], STATES[to as usize]
        ),
    }
}

fn main() {
    use knorke_runtime::{FixedStep, Runtime};
    use knorke_types::EventCursor;

    let step = FixedStep::new(1, 1);
    let mut rt = Runtime::new(
        Scene::new(NoProbe),
        knorke_replay::NoRecord,
        step,
        Tick::ZERO,
    );
    let mut cursor = EventCursor::default();
    for t in 0..END {
        if let Some(c) = Script::BOLD.command(t) {
            rt.submit(c);
        }
        rt.tick();
        let a = &rt.sim.yard.actors;
        for &e in a.events.read(&mut cursor).since().unwrap_or_default() {
            println!("tick {t:>2}  {}", describe(e));
        }
        println!(
            "tick {t:>2}  intruder ({:>2}, {})  guard ({:>2}, {})",
            a.intruder.x, a.intruder.y, a.guard.x, a.guard.y
        );
    }
}
