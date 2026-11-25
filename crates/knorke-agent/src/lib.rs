//! The agent plane (plan section 8, ADR 0018): the operations an agent works a scene with.
//!
//! Each operation is a function over typed values; [`run`] is the command line on top of
//! them and answers with one JSON object. An MCP adapter would call the same functions.
//!
//! - [`discover`]: what exists, as JSON Schema generated from the Rust types.
//! - [`inspect`]: a scene in brief, or one entity in full.
//! - `validate`: `knorke_scene::Registry::validate` with the profile's registry.
//! - `mutate`: `knorke_scene::Scene::apply`: a change set, all or nothing, answered with its
//!   undo.
//! - [`execute`]: runs the scene headless for a number of ticks and reports what happened.
//! - [`render`]: the same run, drawn after its last tick and written as a PNG (feature
//!   `gpu`).
//!
//! For now the one profile is `knorke_integration::stealth`.

mod render;

pub use render::{DEFAULT_SHADERS, NO_GPU, NO_SHADERS, render};

use std::path::Path;

use knorke_integration::Cell;
use knorke_integration::stealth::{self, Command, Event};
use knorke_replay::{NoRecord, Simulation};
use knorke_runtime::{FixedStep, Runtime};
use knorke_scene::{ChangeSet, Scene, diagnostic_json, has_errors};
use knorke_types::{Diagnostic, DiagnosticCode, EventCursor, SnapshotBuf, Tick};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A file could not be read or written.
pub const IO: DiagnosticCode = DiagnosticCode::new("knorke.agent.io");
/// A file that is not JSON of the expected shape.
pub const MALFORMED: DiagnosticCode = DiagnosticCode::new("knorke.agent.malformed");
/// The command line was not understood.
pub const USAGE: DiagnosticCode = DiagnosticCode::new("knorke.agent.usage");
/// `inspect` was asked for an entity the scene does not have.
pub const UNKNOWN_ENTITY: DiagnosticCode = DiagnosticCode::new("knorke.agent.unknown_entity");

/// The most ticks one `execute` runs.
pub const MAX_TICKS: u64 = 100_000;

const USAGE_TEXT: &str = "knorke discover | inspect <scene> [entity] | validate <scene> | \
    mutate <scene> <changes> [--undo <file>] | execute <scene> [--ticks <n>] [--orders <file>] | \
    render <scene> --out <file.png> [--ticks <n>] [--orders <file>] [--shaders <pack>]";

/// One input of a run: at `tick` the intruder is told to walk to `goto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Order {
    /// The tick the command enters at.
    pub tick: u64,
    /// The cell `[x, y]` to walk to.
    pub goto: [i32; 2],
}

/// What exists: the formats and component types as JSON Schema, and the operations.
pub fn discover() -> Value {
    let schema = |s: schemars::Schema| serde_json::to_value(s).unwrap_or(Value::Null);
    json!({
        "scene": schema(schemars::schema_for!(Scene)),
        "change_set": schema(schemars::schema_for!(ChangeSet)),
        "orders": schema(schemars::schema_for!(Vec<Order>)),
        "components": stealth::registry().schemas(),
        "operations": USAGE_TEXT,
    })
}

/// The scene in brief (entities with their component ids), or everything about `entity`.
///
/// # Errors
/// [`UNKNOWN_ENTITY`].
pub fn inspect(scene: &Scene, entity: Option<&str>) -> Result<Value, Diagnostic> {
    match entity {
        None => {
            let entities: serde_json::Map<String, Value> = scene
                .entities
                .iter()
                .map(|(name, e)| (name.clone(), json!(e.keys().collect::<Vec<_>>())))
                .collect();
            Ok(json!({ "revision": scene.revision, "entities": entities }))
        }
        Some(name) => match scene.entities.get(name) {
            Some(e) => Ok(json!({ "revision": scene.revision, "entity": name, "components": e })),
            None => Err(
                Diagnostic::error(UNKNOWN_ENTITY, format!("no entity named {name}"))
                    .at(format!("entities/{name}")),
            ),
        },
    }
}

fn state_name(state: u32) -> &'static str {
    ["post", "chase", "return", "caught"]
        .get(state as usize)
        .copied()
        .unwrap_or("unknown")
}

/// The start of a headless run of `scene`: one tick per call.
fn start(scene: &Scene) -> Result<Runtime<stealth::Sim>, Vec<Diagnostic>> {
    let sim = stealth::build(scene)?;
    Ok(Runtime::new(
        sim,
        NoRecord,
        FixedStep::new(1, 1),
        Tick::ZERO,
    ))
}

/// Runs the next tick with the orders that name it.
fn tick(rt: &mut Runtime<stealth::Sim>, orders: &[Order]) {
    let now = rt.next_tick().0;
    for order in orders.iter().filter(|o| o.tick == now) {
        rt.submit(Command::Goto(Cell::new(order.goto[0], order.goto[1], 0)));
    }
    rt.tick();
}

/// Runs `scene` headless for `ticks` ticks with `orders` and reports the events, where
/// everyone ended up and the checksum of the final state (equal runs have equal checksums).
///
/// # Errors
/// What `knorke_integration::stealth::build` reports for the scene.
pub fn execute(scene: &Scene, orders: &[Order], ticks: u64) -> Result<Value, Vec<Diagnostic>> {
    let mut rt = start(scene)?;
    let actors = &rt.sim.world.actors;
    let names: Vec<String> = actors.guards.iter().map(|g| g.name.clone()).collect();
    let mut cursor = EventCursor::default();
    let mut events = Vec::new();
    for t in 0..ticks.min(MAX_TICKS) {
        tick(&mut rt, orders);
        let queue = &rt.sim.world.actors.events;
        for &e in queue.read(&mut cursor).since().unwrap_or_default() {
            events.push(match e {
                Event::Spotted { guard, at } => json!({
                    "tick": t, "kind": "spotted", "guard": names[guard as usize],
                    "at": [at.x, at.y],
                }),
                Event::State { guard, from, to } => json!({
                    "tick": t, "kind": "state", "guard": names[guard as usize],
                    "from": state_name(from), "to": state_name(to),
                }),
            });
        }
    }
    let a = &rt.sim.world.actors;
    let guards: Vec<Value> = a
        .guards
        .iter()
        .map(|g| json!({ "name": g.name, "cell": [g.cell.x, g.cell.y], "state": state_name(g.sm.state()) }))
        .collect();
    let mut state = SnapshotBuf::new();
    rt.sim.save(&mut state);
    Ok(json!({
        "ticks": rt.next_tick().0,
        "events": events,
        "intruder": [a.intruder.x, a.intruder.y],
        "guards": guards,
        "checksum": format!("{:016x}", state.checksum().0),
    }))
}

fn read(path: &str) -> Result<String, Diagnostic> {
    std::fs::read_to_string(path)
        .map_err(|e| Diagnostic::error(IO, format!("{path}: {e}")).at(path))
}

fn read_scene(path: &str) -> Result<Scene, Diagnostic> {
    Scene::from_text(&read(path)?).map_err(|d| d.at(path))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, Diagnostic> {
    serde_json::from_str(&read(path)?)
        .map_err(|e| Diagnostic::error(MALFORMED, e.to_string()).at(path))
}

fn write(path: &str, text: &str) -> Result<(), Diagnostic> {
    std::fs::write(Path::new(path), text)
        .map_err(|e| Diagnostic::error(IO, format!("{path}: {e}")).at(path))
}

fn usage() -> Vec<Diagnostic> {
    vec![Diagnostic::error(USAGE, USAGE_TEXT)]
}

/// The value after `flag` in `args`, if the flag is there.
fn option<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, Vec<Diagnostic>> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => args.get(i + 1).map(|v| Some(v.as_str())).ok_or_else(usage),
    }
}

fn operate(args: &[String]) -> Result<Value, Vec<Diagnostic>> {
    let one = |d: Diagnostic| vec![d];
    let arg = |i: usize| args.get(i).map(String::as_str);
    match (arg(0), arg(1)) {
        (Some("discover"), None) => Ok(discover()),
        (Some("inspect"), Some(path)) => {
            inspect(&read_scene(path).map_err(one)?, arg(2)).map_err(one)
        }
        (Some("validate"), Some(path)) => {
            let scene = read_scene(path).map_err(one)?;
            let found = stealth::registry().validate(&scene);
            if has_errors(&found) {
                return Err(found);
            }
            let warnings: Vec<Value> = found.iter().map(diagnostic_json).collect();
            Ok(json!({ "revision": scene.revision, "warnings": warnings }))
        }
        (Some("mutate"), Some(path)) => {
            let changes = arg(2).ok_or_else(usage)?;
            let mut scene = read_scene(path).map_err(one)?;
            let set: ChangeSet = read_json(changes).map_err(one)?;
            let undo = scene.apply(&stealth::registry(), &set)?;
            write(path, &scene.to_text()).map_err(one)?;
            if let Some(file) = option(args, "--undo")? {
                let text = serde_json::to_string_pretty(&undo).unwrap_or_default();
                write(file, &(text + "\n")).map_err(one)?;
            }
            Ok(json!({ "revision": scene.revision, "undo": undo }))
        }
        (Some(op @ ("execute" | "render")), Some(path)) => {
            let scene = read_scene(path).map_err(one)?;
            let orders: Vec<Order> = match option(args, "--orders")? {
                Some(file) => read_json(file).map_err(one)?,
                None => Vec::new(),
            };
            let ticks = option(args, "--ticks")?.map(|n| n.parse().map_err(|_| usage()));
            if op == "execute" {
                execute(&scene, &orders, ticks.transpose()?.unwrap_or(60))
            } else {
                let out = option(args, "--out")?.ok_or_else(usage)?;
                let shaders = option(args, "--shaders")?;
                // Without `--ticks` the picture shows the scene as it starts.
                render(
                    &scene,
                    &orders,
                    ticks.transpose()?.unwrap_or(0),
                    shaders,
                    out,
                )
            }
        }
        _ => Err(usage()),
    }
}

/// The command line: runs the operation `args` name (without the program name) and returns
/// the exit code with the answer. The answer is `{"ok": true, ...}` with exit code 0, or
/// `{"ok": false, "diagnostics": [...]}` with exit code 1 (2 for a command line that was
/// not understood).
pub fn run(args: &[String]) -> (i32, Value) {
    match operate(args) {
        Ok(Value::Object(mut answer)) => {
            answer.insert("ok".into(), Value::Bool(true));
            (0, Value::Object(answer))
        }
        Ok(other) => (0, json!({ "ok": true, "result": other })),
        Err(found) => {
            let code = if found.iter().any(|d| d.code == USAGE) {
                2
            } else {
                1
            };
            let diagnostics: Vec<Value> = found.iter().map(diagnostic_json).collect();
            (code, json!({ "ok": false, "diagnostics": diagnostics }))
        }
    }
}
