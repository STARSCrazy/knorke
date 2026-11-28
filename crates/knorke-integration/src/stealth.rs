//! Profile `stealth`: a yard with walls, guards and an intruder, built from a scene and run
//! by the runtime (ADR 0018).
//!
//! Authoring side: the component types [`Yard`], [`Guard`], [`Intruder`] and [`Lamp`] with
//! their rules ([`registry`]). Runtime side: [`build`] turns a valid scene into a [`Sim`],
//! a `Simulation` whose tick runs the table [`SYSTEMS`]: the intruder walks (navigation),
//! every guard looks (perception), decides (state machine) and walks (navigation). The only
//! input from outside is a [`Command`] ([`Command::Goto`], [`Command::Step`]); what happened goes into an `EventQueue`.
//!
//! The scene is read once, in [`build`]. A tick never touches it.

use knorke_ai::group::heading;
use knorke_ai::state::{StateMachine, Transition};
use knorke_observe::{NoProbe, Probe, Scope};
use knorke_perception::{Attention, Budget as SightBudget, PerceptKind, World as SenseWorld};
use knorke_replay::{ReplayHeader, ReplayLog, Simulation};
use knorke_runtime::{System, run_systems};
use knorke_scene::{Component, Registry, Scene, component_path, has_errors};
use knorke_types::{
    Checksum, Diagnostic, DiagnosticCode, EventQueue, Reader, Snapshot, SnapshotBuf, SnapshotError,
    SnapshotValue, Tick, Writer, by_tag, restore,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::perception::{GridSight, cell_centre};
use crate::prelude::*;

/// A scene needs exactly one yard.
pub const YARD_COUNT: DiagnosticCode = DiagnosticCode::new("knorke.stealth.yard_count");
/// A yard is 1 to [`MAX_SIDE`] cells wide and high.
pub const YARD_SIZE: DiagnosticCode = DiagnosticCode::new("knorke.stealth.yard_size");
/// A scene needs exactly one intruder.
pub const INTRUDER_COUNT: DiagnosticCode = DiagnosticCode::new("knorke.stealth.intruder_count");
/// A cell outside the yard.
pub const OFF_GRID: DiagnosticCode = DiagnosticCode::new("knorke.stealth.off_grid");
/// An actor placed inside a wall.
pub const IN_WALL: DiagnosticCode = DiagnosticCode::new("knorke.stealth.in_wall");
/// A lamp value out of range.
pub const LAMP_RANGE: DiagnosticCode = DiagnosticCode::new("knorke.stealth.lamp_range");

/// A replay file that cannot be read.
pub const REPLAY_FILE: DiagnosticCode = DiagnosticCode::new("knorke.stealth.replay_file");
/// A replay file recorded on another scene.
pub const REPLAY_SCENE: DiagnosticCode = DiagnosticCode::new("knorke.stealth.replay_scene");

/// The longest side of a yard, in cells.
pub const MAX_SIDE: i32 = 256;

/// State of a guard: at the post.
pub const POST: u32 = 0;
/// State of a guard: after the intruder.
pub const CHASE: u32 = 1;
/// State of a guard: walking back to the post.
pub const RETURN: u32 = 2;
/// State of a guard: next to the intruder, who stays where it is from now on.
pub const CAUGHT: u32 = 3;

const F_SEES: u64 = 1;
const F_LOST: u64 = 2;
const F_HOME: u64 = 4;
const F_REACH: u64 = 8;

/// Fine units per cell.
const CELL: i32 = 256;
/// Ticks an event stays readable.
const KEEP_EVENTS: u64 = 8;
const INTRUDER_ID: u32 = u32::MAX;

// ---- Authoring: components and rules ---------------------------------------------------

/// The ground: its size and the cells that block walking and sight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Yard {
    /// Cells along x.
    pub width: i32,
    /// Cells along y.
    pub height: i32,
    /// Wall cells as `[x, y]`.
    pub walls: Vec<[i32; 2]>,
}

impl Yard {
    fn contains(&self, c: [i32; 2]) -> bool {
        (0..self.width).contains(&c[0]) && (0..self.height).contains(&c[1])
    }
}

impl Component for Yard {
    const ID: &'static str = "knorke.stealth.yard/1";

    fn check(&self, path: &str, out: &mut Vec<Diagnostic>) {
        for (field, side) in [("width", self.width), ("height", self.height)] {
            if !(1..=MAX_SIDE).contains(&side) {
                let message = format!("{field} is {side}, allowed is 1 to {MAX_SIDE}");
                out.push(Diagnostic::error(YARD_SIZE, message).at(format!("{path}/{field}")));
            }
        }
        for (i, &wall) in self.walls.iter().enumerate() {
            if !self.contains(wall) {
                let message = format!("wall {wall:?} is outside the yard");
                out.push(Diagnostic::error(OFF_GRID, message).at(format!("{path}/walls/{i}")));
            }
        }
    }
}

/// A compass heading; `+y` is south.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Heading {
    /// `+x`.
    #[default]
    East,
    /// Between east and south.
    SouthEast,
    /// `+y`.
    South,
    /// Between south and west.
    SouthWest,
    /// `-x`.
    West,
    /// Between west and north.
    NorthWest,
    /// `-y`.
    North,
    /// Between north and east.
    NorthEast,
}

impl Heading {
    /// The heading as perception counts it: 0..=255, 0 = `+x`, 64 = `+y`.
    pub fn facing(self) -> u8 {
        self as u8 * 32
    }
}

fn give_up_default() -> u32 {
    4
}

/// A guard: stands at its post until it sees the intruder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// Its post as `[x, y]`.
    pub post: [i32; 2],
    /// Where it looks while at the post.
    #[serde(default)]
    pub facing: Heading,
    /// Ticks without a sighting before it gives up the chase.
    #[serde(default = "give_up_default")]
    pub give_up_after: u32,
}

impl Component for Guard {
    const ID: &'static str = "knorke.stealth.guard/1";
}

/// The intruder: walks where the host tells it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Intruder {
    /// Where it starts, as `[x, y]`.
    pub start: [i32; 2],
}

impl Component for Intruder {
    const ID: &'static str = "knorke.stealth.intruder/1";
}

/// A guard's lamp. Only drawn: guards see by their own senses, not by this light.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lamp {
    /// Reach in cells, above 0.
    pub radius: f32,
    /// Full light inside this angle from the heading, degrees.
    pub inner_angle: f32,
    /// No light outside this angle, degrees, above `inner_angle` and below 90.
    pub outer_angle: f32,
    /// Linear RGB, each 0 or more.
    pub color: [f32; 3],
    /// Multiplier, 0 or more.
    pub intensity: f32,
}

impl Component for Lamp {
    const ID: &'static str = "knorke.stealth.lamp/1";

    fn check(&self, path: &str, out: &mut Vec<Diagnostic>) {
        let mut need = |ok: bool, field: &str, rule: &str| {
            if !ok {
                out.push(Diagnostic::error(LAMP_RANGE, rule).at(format!("{path}/{field}")));
            }
        };
        need(
            self.radius > 0.0 && self.radius.is_finite(),
            "radius",
            "radius must be above 0",
        );
        need(
            self.inner_angle > 0.0 && self.inner_angle < self.outer_angle,
            "inner_angle",
            "inner_angle must be above 0 and below outer_angle",
        );
        need(
            self.outer_angle < 90.0,
            "outer_angle",
            "outer_angle must be below 90",
        );
        need(
            self.color.iter().all(|c| *c >= 0.0 && c.is_finite()),
            "color",
            "color channels must be 0 or more",
        );
        need(
            self.intensity >= 0.0 && self.intensity.is_finite(),
            "intensity",
            "intensity must be 0 or more",
        );
    }
}

/// One yard, one intruder, and every actor on a free cell of the yard.
fn placement(scene: &Scene, out: &mut Vec<Diagnostic>) {
    let yards: Vec<_> = scene.each::<Yard>().collect();
    if yards.len() != 1 {
        let message = format!("a scene needs exactly one yard, found {}", yards.len());
        out.push(Diagnostic::error(YARD_COUNT, message).at("entities"));
    }
    let intruders = scene.each::<Intruder>().count();
    if intruders != 1 {
        let message = format!("a scene needs exactly one intruder, found {intruders}");
        out.push(Diagnostic::error(INTRUDER_COUNT, message).at("entities"));
    }
    let Some((_, yard)) = yards.first() else {
        return;
    };
    let mut place = |entity: &str, component: &str, field: &str, cell: [i32; 2]| {
        let path = format!("{}/{field}", component_path(entity, component));
        if !yard.contains(cell) {
            let message = format!("{cell:?} is outside the yard");
            out.push(Diagnostic::error(OFF_GRID, message).at(path));
        } else if yard.walls.contains(&cell) {
            let message = format!("{cell:?} is inside a wall");
            out.push(Diagnostic::error(IN_WALL, message).at(path));
        }
    };
    for (name, guard) in scene.each::<Guard>() {
        place(name, Guard::ID, "post", guard.post);
    }
    for (name, intruder) in scene.each::<Intruder>() {
        place(name, Intruder::ID, "start", intruder.start);
    }
}

/// The component types and rules of this profile.
pub fn registry() -> Registry {
    let mut r = Registry::default();
    r.register::<Yard>()
        .register::<Guard>()
        .register::<Intruder>()
        .register::<Lamp>()
        .rule(placement);
    r
}

// ---- Runtime ---------------------------------------------------------------------------

/// Input from outside the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The intruder walks to this cell, one cell per tick. Ignored for a cell it cannot
    /// stand on.
    Goto(Cell),
    /// The intruder steps to the neighbouring cell in this direction (a player's arrow key,
    /// ADR 0022). Ignored for a cell it cannot stand on.
    Step(Dir),
}

/// One of the four directions of a step; `+y` is south.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// `-y`.
    North,
    /// `+x`.
    East,
    /// `+y`.
    South,
    /// `-x`.
    West,
}

impl Dir {
    /// The neighbour of `c` in this direction.
    pub fn from(self, c: Cell) -> Cell {
        let (dx, dy) = match self {
            Dir::North => (0, -1),
            Dir::East => (1, 0),
            Dir::South => (0, 1),
            Dir::West => (-1, 0),
        };
        Cell::new(c.x + dx, c.y + dy, c.level)
    }
}

// New tags only append: tag 0 keeps the bytes of recorded `Goto` commands.
by_tag! {
    Command, SnapshotError::Invalid("stealth command tag") => {
        0 => Goto(cell),
        1 => Step(dir),
    };
    Dir, SnapshotError::Invalid("stealth direction tag") => {
        0 => North,
        1 => East,
        2 => South,
        3 => West,
    };
}

/// What happened in a tick. `guard` counts the guards in the order of their entity names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A guard recognised the intruder at this cell.
    Spotted {
        /// The guard.
        guard: u32,
        /// Where the intruder was.
        at: Cell,
    },
    /// A guard's state changed ([`POST`], [`CHASE`], [`RETURN`], [`CAUGHT`]).
    State {
        /// The guard.
        guard: u32,
        /// The state it left.
        from: u32,
        /// The state it entered.
        to: u32,
    },
}

by_tag! { Event, SnapshotError::Invalid("stealth event tag") => {
    0 => Spotted { guard, at },
    1 => State { guard, from, to },
} }

/// One guard: what the scene says about it and where it stands in the run.
pub struct GuardState {
    /// Its entity name in the scene.
    pub name: String,
    post: Cell,
    post_facing: u8,
    give_up_after: u32,
    /// Its cell.
    pub cell: Cell,
    /// Its heading (0..=255, 0 = +x, 64 = +y).
    pub facing: u8,
    /// Its state ([`POST`], [`CHASE`], [`RETURN`], [`CAUGHT`]).
    pub sm: StateMachine,
    aware: Awareness,
    attention: Attention,
    /// Where and when it last saw the intruder.
    clue: Option<(Cell, u32)>,
}

/// Everything a tick changes. One snapshot section; what the scene fixed (posts, names) is
/// not in it, so a snapshot fits only the scene it was made from.
pub struct Actors {
    /// The intruder's cell.
    pub intruder: Cell,
    goal: Cell,
    /// The guards, in the order of their entity names.
    pub guards: Vec<GuardState>,
    /// What happened, for the host.
    pub events: EventQueue<Event>,
}

impl Actors {
    fn caught(&self) -> bool {
        self.guards.iter().any(|g| g.sm.state() == CAUGHT)
    }
}

impl Snapshot for Actors {
    const ID: &'static str = "knorke.stealth.actors";
    const VERSION: u16 = 1;

    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.intruder);
        w.put(&self.goal);
        w.put_len(self.guards.len());
        for g in &self.guards {
            w.put(&g.cell);
            w.put(&g.facing);
            w.put(&g.sm);
            w.put(&g.aware);
            w.put(&g.attention);
            w.put(&g.clue);
        }
        w.put(&self.events);
    }

    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.intruder = r.get()?;
        self.goal = r.get()?;
        if r.get_len()? != self.guards.len() {
            return Err(SnapshotError::Invalid("stealth: another number of guards"));
        }
        for g in &mut self.guards {
            g.cell = r.get()?;
            g.facing = r.get()?;
            g.sm.get_into(r)?;
            g.aware = r.get()?;
            g.attention = r.get()?;
            g.clue = r.get()?;
        }
        self.events.get_into(r)
    }
}

/// The yard and who is in it.
pub struct World {
    /// The walls. Fixed by the scene, so not part of a snapshot.
    grid: DenseGrid,
    senses: SenseWorld,
    profile: SenseProfile,
    /// The changing state.
    pub actors: Actors,
    /// Shared by all walkers. Every search starts fresh; the snapshot keeps it anyway, so a
    /// restored run cannot depend on what an earlier search left behind.
    nav: Navigator,
    /// Scratch of the looking system, empty between ticks.
    percepts: Vec<Percept>,
}

fn cell(c: [i32; 2]) -> Cell {
    Cell::new(c[0], c[1], 0)
}

fn pos(c: Cell) -> Pos {
    [c.x, c.y, c.level]
}

impl World {
    /// Width and height of the yard in cells.
    pub fn size(&self) -> (i32, i32) {
        (self.grid.width(), self.grid.height())
    }

    /// `true` for a cell of the yard without a wall.
    pub fn open(&self, c: Cell) -> bool {
        c.level == 0 && self.grid.open(c.x, c.y)
    }

    /// The next cell on the way from `from` to `goal` (`from` when there is none).
    fn next_cell(&mut self, from: Cell, goal: Cell) -> Cell {
        self.nav
            .find_path(&mut self.grid, from, &[goal], Budget::UNLIMITED);
        self.nav.path().get(1).copied().unwrap_or(from)
    }
}

fn walk_intruder(world: &mut World, _: Tick) {
    if world.actors.caught() {
        return;
    }
    let (from, goal) = (world.actors.intruder, world.actors.goal);
    world.actors.intruder = world.next_cell(from, goal);
}

fn guards_look(world: &mut World, tick: Tick) {
    let now = tick.0 as u32;
    let a = &mut world.actors;
    let target = Target {
        id: INTRUDER_ID,
        point: cell_centre(&world.senses, a.intruder),
        size: 256,
        conspicuity: 256,
        ..Target::default()
    };
    for (i, g) in a.guards.iter_mut().enumerate() {
        let eye = Eye {
            id: i as u32,
            point: cell_centre(&world.senses, g.cell),
            facing: g.facing,
            profile: &world.profile,
            stats: Stats::NORMAL,
            disguise: Default::default(),
        };
        world.percepts.clear();
        look(
            &world.senses,
            &mut GridSight::square(&world.grid),
            now,
            &eye,
            &[target],
            core::slice::from_mut(&mut g.aware),
            &mut g.attention,
            SightBudget {
                checks: 1,
                keep: 1,
                instant_cap: 0,
            },
            &mut world.percepts,
        );
        for p in &world.percepts {
            if p.event == PerceptKind::Recognized {
                let (guard, at) = (i as u32, a.intruder);
                a.events.push(tick, Event::Spotted { guard, at });
            }
        }
        // Recognised and in view: the guard knows where the intruder is right now.
        if g.aware.stage() >= 2 && g.aware.in_view() {
            g.clue = Some((a.intruder, now));
        }
    }
    world.percepts.clear();
}

fn guards_decide(world: &mut World, tick: Tick) {
    let now = tick.0 as u32;
    let a = &mut world.actors;
    for (i, g) in a.guards.iter_mut().enumerate() {
        let mut flags = 0;
        match g.clue {
            Some((_, at)) if at == now => flags |= F_SEES,
            Some((_, at)) if now - at >= g.give_up_after => flags |= F_LOST,
            _ => {}
        }
        if g.cell == g.post {
            flags |= F_HOME;
        }
        // Next to the intruder the guard has it.
        if (a.intruder.x - g.cell.x).abs() <= 1 && (a.intruder.y - g.cell.y).abs() <= 1 {
            flags |= F_REACH;
        }
        if let Some(change) = g.sm.update(now, flags) {
            let (guard, from, to) = (i as u32, change.from, change.to);
            a.events.push(tick, Event::State { guard, from, to });
            if to == RETURN {
                g.clue = None;
            }
        }
    }
}

fn walk_guards(world: &mut World, _: Tick) {
    for i in 0..world.actors.guards.len() {
        let g = &world.actors.guards[i];
        let (state, from) = (g.sm.state(), g.cell);
        let goal = match (state, g.clue) {
            (CAUGHT, _) => continue,
            (CHASE, Some((seen, _))) => seen,
            _ => g.post,
        };
        let to = world.next_cell(from, goal);
        let g = &mut world.actors.guards[i];
        g.cell = to;
        if to != from {
            g.facing = heading(Metric::Chebyshev, pos(from), pos(to)) * 32;
        } else if state == POST {
            g.facing = g.post_facing;
        }
    }
}

static WALK_INTRUDER: Scope = Scope::new("knorke.stealth", "walk_intruder");
static GUARDS_LOOK: Scope = Scope::new("knorke.stealth", "guards_look");
static GUARDS_DECIDE: Scope = Scope::new("knorke.stealth", "guards_decide");
static WALK_GUARDS: Scope = Scope::new("knorke.stealth", "walk_guards");

/// The schedule of one tick.
pub static SYSTEMS: [System<World>; 4] = [
    System {
        scope: &WALK_INTRUDER,
        run: walk_intruder,
    },
    System {
        scope: &GUARDS_LOOK,
        run: guards_look,
    },
    System {
        scope: &GUARDS_DECIDE,
        run: guards_decide,
    },
    System {
        scope: &WALK_GUARDS,
        run: walk_guards,
    },
];

/// The simulation: the world plus the probe its systems report to ([`NoProbe`]: none).
pub struct Sim<P = NoProbe> {
    /// The world the systems work on.
    pub world: World,
    /// Sees every system of every tick.
    pub probe: P,
}

/// The head of a recording of a run on `scene` (ADR 0022 step 6). It names the scene by a
/// checksum of its canonical text, so formatting does not matter but any change of content
/// does: a snapshot only fits the scene its simulation was built from (plan 17.1).
pub fn replay_header(scene: &Scene) -> ReplayHeader {
    ReplayHeader {
        app: format!(
            "knorke.stealth scene {:016x}",
            Checksum::of(scene.to_text().as_bytes()).0
        ),
        ..ReplayHeader::default()
    }
}

/// Reads a recording written with [`ReplayLog::write`] and checks that it was made on
/// `scene`.
///
/// # Errors
/// [`REPLAY_FILE`] for bytes that are no replay of this format, [`REPLAY_SCENE`] for a
/// replay of another scene.
pub fn read_replay(bytes: &[u8], scene: &Scene) -> Result<ReplayLog<Command>, Diagnostic> {
    let log = ReplayLog::<Command>::read(bytes)
        .map_err(|e| Diagnostic::error(REPLAY_FILE, format!("not a readable replay: {e:?}")))?;
    let want = replay_header(scene).app;
    if log.header.app != want {
        return Err(Diagnostic::error(
            REPLAY_SCENE,
            format!("recorded on '{}', this scene is '{want}'", log.header.app),
        ));
    }
    Ok(log)
}

/// Builds the start of a run from `scene`, with [`NoProbe`].
///
/// # Errors
/// What [`registry`] reports for the scene, if that includes an error.
pub fn build(scene: &Scene) -> Result<Sim, Vec<Diagnostic>> {
    build_with(scene, NoProbe)
}

/// [`build`] with a probe.
///
/// # Errors
/// As [`build`].
pub fn build_with<P>(scene: &Scene, probe: P) -> Result<Sim<P>, Vec<Diagnostic>> {
    let found = registry().validate(scene);
    if has_errors(&found) {
        return Err(found);
    }
    // Valid: exactly one of each, every cell on the grid.
    let (Some((_, yard)), Some((_, intruder))) =
        (scene.each::<Yard>().next(), scene.each::<Intruder>().next())
    else {
        return Err(found);
    };
    let mut grid = DenseGrid::new(yard.width, yard.height);
    for wall in &yard.walls {
        grid.set(wall[0], wall[1], DenseGrid::BLOCKED);
    }
    let guards = scene
        .each::<Guard>()
        .map(|(name, guard)| {
            let mut sm = StateMachine::new(4, POST).expect("four states");
            // Catching comes first: a caught intruder is still seen.
            for (from, to, when) in [
                (CHASE, CAUGHT, F_REACH),
                (POST, CHASE, F_SEES),
                (RETURN, CHASE, F_SEES),
                (CHASE, RETURN, F_LOST),
                (RETURN, POST, F_HOME),
            ] {
                sm.add_transition(Transition::new(from, to, when))
                    .expect("states exist");
            }
            GuardState {
                name: name.to_owned(),
                post: cell(guard.post),
                post_facing: guard.facing.facing(),
                give_up_after: guard.give_up_after,
                cell: cell(guard.post),
                facing: guard.facing.facing(),
                sm,
                aware: Awareness::default(),
                attention: Attention::default(),
                clue: None,
            }
        })
        .collect();
    let start = cell(intruder.start);
    Ok(Sim {
        world: World {
            grid,
            senses: SenseWorld::square(CELL),
            profile: SenseProfile::guard(CELL),
            actors: Actors {
                intruder: start,
                goal: start,
                guards,
                events: EventQueue::new(),
            },
            nav: Navigator::new(
                Topology::Square(GridRules::preset(Preset::Octile)),
                (yard.width * yard.height) as usize,
            ),
            percepts: Vec::new(),
        },
        probe,
    })
}

impl<P: Probe> Simulation for Sim<P> {
    type Command = Command;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.world.actors);
        out.save(&self.world.nav);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.world.actors)?;
        restore(&mut r, &mut self.world.nav)?;
        r.finish("knorke.stealth")?;
        // The walking systems hand these cells to the path search.
        let a = &self.world.actors;
        let cells = [a.intruder, a.goal].into_iter();
        if cells
            .chain(a.guards.iter().map(|g| g.cell))
            .all(|c| self.world.open(c))
        {
            Ok(())
        } else {
            Err(SnapshotError::Invalid("stealth: actor off the yard"))
        }
    }

    fn step(&mut self, tick: Tick, commands: &[Command]) {
        // Sync point: outside input changes the world here and nowhere else.
        for &command in commands {
            let goal = match command {
                Command::Goto(goal) => goal,
                Command::Step(dir) => dir.from(self.world.actors.intruder),
            };
            if self.world.open(goal) {
                self.world.actors.goal = goal;
            }
        }
        self.world.actors.events.keep_ticks(tick, KEEP_EVENTS);
        run_systems(&mut self.world, tick, &SYSTEMS, &mut self.probe);
    }
}
