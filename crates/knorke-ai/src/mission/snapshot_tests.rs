//! Snapshot tests of [`Mission`]: round trip, buffer reuse and refused input.

use super::*;
use crate::memory::{Evict, Fact, Knowledge};
use crate::mission::types::{Detour, Step};
use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

const KIND: u32 = 0;

fn bytes(m: &Mission) -> Vec<u8> {
    let mut out = Vec::new();
    Writer::new(&mut out).put(m);
    out
}

fn restore(into: &mut Mission, data: &[u8]) -> Result<(), SnapshotError> {
    let mut r = Reader::new(data);
    into.get_into(&mut r)?;
    assert_eq!(r.remaining(), 0);
    Ok(())
}

fn fresh() -> Mission {
    Mission::new(MissionConfig::default(), Motive::guard_place([0, 0, 0])).unwrap()
}

/// A guard with a looping patrol (walk, search, wait) and a playbook plan.
fn guard() -> Mission {
    let mut m = fresh();
    m.set_leeway(Leeway::FLEXIBLE);
    m.set_plan(&Plan::looping(&[
        Step::go_to([3, 0, 0]),
        Step::search(Where::Here, 0, 6),
        Step::wait(2),
    ]))
    .unwrap();
    m.add_playbook(&Plan::once(&[Step::hold(Where::MotivePlace)]), 300, false)
        .unwrap();
    m
}

/// The world at `now`: subject 5 shows up at tick 4 and is defeated at tick 12.
fn world(k: &mut Knowledge, now: u32) {
    if now == 4 {
        k.learn(
            now,
            Fact {
                subject: 5,
                kind: KIND,
                pos: [2, 2, 0],
                ..Fact::default()
            },
        );
    } else if now > 4 {
        k.touch(5, KIND, now);
    }
    if now == 12 {
        k.set_data(5, KIND, 2);
    }
}

type Record = (Action, Vec<MissionEvent>, Vec<u8>);

/// Runs ticks `from..to`, starting the detour at tick 5; records action, events, bytes.
fn drive(m: &mut Mission, k: &mut Knowledge, pos: &mut Pos, from: u32, to: u32) -> Vec<Record> {
    let mut out = Vec::new();
    for now in from..to {
        world(k, now);
        if now == 5 {
            assert!(m.detour(now, Detour::Engage(5), k));
        }
        let o = m.tick(now, *pos, k, 0);
        let (action, events) = (o.action, o.events().to_vec());
        if let Action::GoTo(t) | Action::Attack { pos: t, .. } | Action::Search(t) = action {
            *pos = [
                pos[0] + (t[0] - pos[0]).signum(),
                pos[1] + (t[1] - pos[1]).signum(),
                pos[2],
            ];
        }
        out.push((action, events, bytes(m)));
    }
    out
}

#[test]
fn round_trip_at_every_tick_continues_identically() {
    const END: u32 = 40;
    let mut saw_detour = false;
    for split in 0..END {
        let mut m = guard();
        let mut k = Knowledge::new(8, Evict::Oldest).unwrap();
        let mut pos = [0, 0, 0];
        drive(&mut m, &mut k, &mut pos, 0, split);
        saw_detour |= matches!(m.activity, Activity::Detour | Activity::Returning);
        let saved = bytes(&m);

        let mut copy = fresh();
        restore(&mut copy, &saved).unwrap();
        assert_eq!(bytes(&copy), saved, "split {split}");

        let (mut k2, mut pos2) = (k.clone(), pos);
        let a = drive(&mut m, &mut k, &mut pos, split, END);
        let b = drive(&mut copy, &mut k2, &mut pos2, split, END);
        assert_eq!(a, b, "split {split}");
    }
    assert!(saw_detour, "the scenario should cover a detour");
}

#[test]
fn restore_into_a_used_mission_reuses_its_buffers() {
    let mut src = guard();
    let mut k = Knowledge::new(8, Evict::Oldest).unwrap();
    let mut pos = [0, 0, 0];
    drive(&mut src, &mut k, &mut pos, 0, 9);
    let saved = bytes(&src);

    let mut dst = guard();
    let mut k2 = Knowledge::new(8, Evict::Oldest).unwrap();
    let mut pos2 = [0, 0, 0];
    drive(&mut dst, &mut k2, &mut pos2, 0, 20);
    let caps = dst.capacities();
    let plan_ptr = dst.plan.steps().as_ptr();
    let events_ptr = dst.events.as_ptr();
    restore(&mut dst, &saved).unwrap();
    assert_eq!(dst.capacities(), caps);
    assert_eq!(dst.plan.steps().as_ptr(), plan_ptr);
    assert_eq!(dst.events.as_ptr(), events_ptr);
    assert_eq!(bytes(&dst), saved);
}

#[test]
fn get_builds_a_default_config_mission() {
    let m = guard();
    let saved = bytes(&m);
    let got = Mission::get(&mut Reader::new(&saved)).unwrap();
    assert_eq!(bytes(&got), saved);
}

/// `bad` is refused with `Invalid`, and the target is left without a plan and still ticks.
fn assert_refused(bad: &[u8], mut into: Mission) {
    let err = restore(&mut into, bad).unwrap_err();
    assert!(matches!(err, SnapshotError::Invalid(_)), "{err:?}");
    assert!(into.plan().is_none());
    assert!(into.events.is_empty());
    let k = Knowledge::new(8, Evict::Oldest).unwrap();
    let _ = into.tick(0, [0, 0, 0], &k, 0);
}

fn running() -> Mission {
    let mut m = guard();
    let k = Knowledge::new(8, Evict::Oldest).unwrap();
    let _ = m.tick(0, [0, 0, 0], &k, 0);
    assert_eq!(m.activity, Activity::Plan);
    m
}

#[test]
fn step_index_out_of_range_is_refused() {
    let mut m = running();
    m.step.index = 3;
    assert_refused(&bytes(&m), guard());
}

#[test]
fn too_many_steps_is_refused() {
    let mut m = running();
    m.plan.set_steps(&[Step::pause(); 17], true);
    assert_refused(&bytes(&m), guard());
}

#[test]
fn bad_tag_is_refused() {
    let mut data = bytes(&running());
    data[0] = 9; // the motive kind
    assert_refused(&data, guard());
}

#[test]
fn too_many_events_are_refused() {
    let mut m = running();
    m.events.clear();
    m.events.extend([MissionEvent::Looped; MAX_EVENTS + 1]);
    assert_refused(&bytes(&m), guard());
}

#[test]
fn search_point_out_of_range_is_refused() {
    let mut m = running();
    m.step.search_point = m.search_points();
    assert_refused(&bytes(&m), guard());
}

#[test]
fn resume_into_a_detour_is_refused() {
    let mut m = running();
    m.detour.resume = Activity::Detour;
    assert_refused(&bytes(&m), guard());
}

#[test]
fn plan_activity_without_plan_is_refused() {
    let mut m = running();
    m.has_plan = false;
    assert_refused(&bytes(&m), guard());
}

#[test]
fn other_playbook_size_is_refused() {
    let config = MissionConfig {
        max_playbook: 4,
        ..MissionConfig::default()
    };
    let other = Mission::new(config, Motive::guard_place([0, 0, 0])).unwrap();
    assert_refused(&bytes(&running()), other);
}

#[test]
fn truncated_input_is_refused_safely() {
    let data = bytes(&running());
    for len in 0..data.len() {
        let mut into = guard();
        assert!(restore(&mut into, &data[..len]).is_err());
        assert!(into.plan().is_none());
    }
}
