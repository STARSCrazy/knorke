//! The command line end to end, the way an agent uses it: discover, inspect, break
//! something and read the diagnostics, mutate, execute, undo.

use knorke_agent::run;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const YARD: &str = include_str!("../../knorke-integration/scenes/guard_yard.json");

/// A fresh directory for one test, with the yard in it.
fn workspace(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("yard.json"), YARD).unwrap();
    dir
}

fn knorke(args: &[&str]) -> (i32, Value) {
    run(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
}

fn path(dir: &Path, file: &str) -> String {
    dir.join(file).to_string_lossy().into_owned()
}

fn write(dir: &Path, file: &str, value: &Value) -> String {
    let p = path(dir, file);
    std::fs::write(&p, value.to_string()).unwrap();
    p
}

/// The script of the bold intruder from the runtime's guard scene.
fn bold() -> Value {
    json!([{ "tick": 2, "goto": [6, 5] }, { "tick": 18, "goto": [13, 8] }])
}

#[test]
fn discover_describes_components_from_their_types() {
    let (code, answer) = knorke(&["discover"]);
    assert_eq!(code, 0);
    let guard = &answer["components"]["knorke.stealth.guard/1"];
    assert_eq!(guard["properties"]["give_up_after"]["type"], "integer");
    assert_eq!(guard["required"], json!(["post"]));
    assert!(answer["components"]["knorke.stealth.lamp/1"].is_object());
    assert!(answer["change_set"]["properties"]["base_revision"].is_object());
}

#[test]
fn inspect_is_brief_unless_asked_for_one_entity() {
    let dir = workspace("inspect");
    let yard = path(&dir, "yard.json");
    let (_, brief) = knorke(&["inspect", &yard]);
    assert_eq!(
        brief["entities"]["guard"],
        json!(["knorke.stealth.guard/1", "knorke.stealth.lamp/1"])
    );
    let (_, full) = knorke(&["inspect", &yard, "guard"]);
    assert_eq!(
        full["components"]["knorke.stealth.guard/1"]["post"],
        json!([3, 5])
    );
    let (code, missing) = knorke(&["inspect", &yard, "ghost"]);
    assert_eq!(code, 1);
    assert_eq!(
        missing["diagnostics"][0]["code"],
        "knorke.agent.unknown_entity"
    );
}

#[test]
fn execute_reproduces_the_scripted_scene() {
    let dir = workspace("execute");
    let yard = path(&dir, "yard.json");
    let orders = write(&dir, "orders.json", &bold());
    let (code, first) = knorke(&["execute", &yard, "--ticks", "40", "--orders", &orders]);
    assert_eq!(code, 0, "{first:#}");
    assert_eq!(
        first["events"],
        json!([
            { "tick": 18, "kind": "spotted", "guard": "guard", "at": [7, 4] },
            { "tick": 18, "kind": "state", "guard": "guard", "from": "post", "to": "chase" },
            { "tick": 21, "kind": "state", "guard": "guard", "from": "chase", "to": "caught" },
        ])
    );
    assert_eq!(first["intruder"], json!([7, 1]));
    // Equal runs have equal checksums; another script does not.
    let (_, again) = knorke(&["execute", &yard, "--ticks", "40", "--orders", &orders]);
    assert_eq!(first["checksum"], again["checksum"]);
    let (_, idle) = knorke(&["execute", &yard, "--ticks", "40"]);
    assert_ne!(first["checksum"], idle["checksum"]);
    assert_eq!(idle["events"], json!([]));
}

#[test]
fn a_bad_change_is_refused_with_its_path_and_the_file_stays() {
    let dir = workspace("refuse");
    let yard = path(&dir, "yard.json");
    // The guard's post moved into the wall.
    let changes = write(
        &dir,
        "bad.json",
        &json!({ "base_revision": 0, "changes": [{
            "kind": "component.set", "entity": "guard",
            "component": "knorke.stealth.guard/1", "value": { "post": [8, 5] },
        }] }),
    );
    let (code, answer) = knorke(&["mutate", &yard, &changes]);
    assert_eq!(code, 1);
    assert_eq!(
        answer["diagnostics"],
        json!([{
            "severity": "error",
            "code": "knorke.stealth.in_wall",
            "message": "[8, 5] is inside a wall",
            "path": "entities/guard/components/knorke.stealth.guard/1/post",
        }])
    );
    assert_eq!(std::fs::read_to_string(&yard).unwrap(), YARD);
    let (code, _) = knorke(&["validate", &yard]);
    assert_eq!(code, 0);
}

#[test]
fn a_second_guard_changes_the_outcome_and_undo_restores_it() {
    let dir = workspace("mutate");
    let yard = path(&dir, "yard.json");
    let orders = write(&dir, "orders.json", &bold());
    let run_it = || knorke(&["execute", &yard, "--ticks", "40", "--orders", &orders]).1;
    let before = run_it();

    // A second guard near the gap the intruder comes through.
    let changes = write(
        &dir,
        "second_guard.json",
        &json!({ "base_revision": 0, "changes": [
            { "kind": "entity.create", "entity": "gatekeeper" },
            { "kind": "component.set", "entity": "gatekeeper",
              "component": "knorke.stealth.guard/1",
              "value": { "post": [5, 1], "facing": "east" } },
        ] }),
    );
    let undo = path(&dir, "undo.json");
    let (code, answer) = knorke(&["mutate", &yard, &changes, "--undo", &undo]);
    assert_eq!((code, &answer["revision"]), (0, &json!(1)), "{answer:#}");

    let after = run_it();
    assert_ne!(after["checksum"], before["checksum"]);
    // The first guard still makes the catch; the gatekeeper notices one tick later and
    // closes in as well.
    assert_eq!(
        after["events"].as_array().unwrap()[..3],
        before["events"].as_array().unwrap()[..]
    );
    assert_eq!(
        after["events"].as_array().unwrap()[3..],
        [
            json!({ "tick": 22, "kind": "spotted", "guard": "gatekeeper", "at": [7, 1] }),
            json!({ "tick": 22, "kind": "state", "guard": "gatekeeper", "from": "post", "to": "chase" }),
            json!({ "tick": 23, "kind": "state", "guard": "gatekeeper", "from": "chase", "to": "caught" }),
        ]
    );

    // The same change set again is stale; the undo file takes the scene back.
    let (code, stale) = knorke(&["mutate", &yard, &changes]);
    assert_eq!(code, 1);
    assert_eq!(
        stale["diagnostics"][0]["code"],
        "knorke.scene.revision_mismatch"
    );
    let (code, undone) = knorke(&["mutate", &yard, &undo]);
    assert_eq!((code, &undone["revision"]), (0, &json!(2)));
    let (_, brief) = knorke(&["inspect", &yard]);
    assert!(brief["entities"].get("gatekeeper").is_none());
    assert_eq!(run_it()["events"], before["events"]);
}

#[test]
fn a_wrong_command_line_says_how_and_a_missing_file_where() {
    let (code, answer) = knorke(&["explode"]);
    assert_eq!(code, 2);
    assert_eq!(answer["diagnostics"][0]["code"], "knorke.agent.usage");
    let (code, answer) = knorke(&["validate", "no/such/file.json"]);
    assert_eq!(code, 1);
    assert_eq!(answer["diagnostics"][0]["code"], "knorke.agent.io");
    assert_eq!(answer["diagnostics"][0]["path"], "no/such/file.json");
}
