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

/// Exact bytes of every plain mission value, each enum variant at least once (see
/// `knorke_types::assert_bytes!`): a swapped tag or field order fails here even though it round-trips.
#[test]
fn value_bytes_are_pinned() {
    use crate::mission::types::{
        BreakReason, DetourEnd, Fail, Leash, Leeway, Motive, MotiveKind, OnLost, StepKind, Where,
    };
    let fact = Fail::Fact {
        subject: 1,
        kind: 2,
        mask: 3,
        value: 4,
    };
    let motive = Motive {
        kind: MotiveKind::Reach,
        place: Some([1, 2, 3]),
        subject: None,
        radius: 4,
    };
    let leash = Leash {
        distance: Some(3),
        ticks: None,
        on_lost: OnLost::Search {
            ticks: 6,
            radius: 2,
        },
    };
    knorke_types::assert_bytes!(
        Where::Fixed([1, -2, 3]) => "0001000000feffffff03000000",
        Where::Subject(4) => "0104000000",
        Where::MotivePlace => "02",
        Where::MotiveSubject => "03",
        Where::Here => "04",
        StepKind::GoTo(Where::Here) => "0004",
        StepKind::Wait(3) => "0103000000",
        StepKind::Pause => "02",
        StepKind::Follow(4) => "0304000000",
        StepKind::WaitFor { subject: 5, radius: 6 } => "040500000006000000",
        StepKind::Search { centre: Where::Subject(1), radius: 2, ticks: 3 } => "0501010000000200000003000000",
        StepKind::Hold(Where::MotivePlace) => "0602",
        StepKind::Attack(7) => "0707000000",
        StepKind::Wander => "08",
        StepKind::Signal(8) => "0908000000",
        StepKind::Custom { kind: 1, at: Where::Here, param: 2 } => "0a010000000402000000",
        StepKind::Call { channel: 3, kind: 4 } => "0b0300000004000000",
        StepKind::Squad { tactic: 5, target: Where::MotiveSubject } => "0c0500000003",
        Fail::Unknown { subject: 1 } => "0001000000",
        Fail::Defeated { subject: 2 } => "0102000000",
        fact => "0201000000020000000300000004000000",
        Fail::Flags { mask: 5, value: 6 } => "0305000000000000000600000000000000",
        Fail::Custom(7) => "0407000000",
        Step { kind: StepKind::Pause, fails: [Some(Fail::Custom(1)), None, Some(fact), None], skip_if_impossible: true } => "02010401000000000102010000000200000003000000040000000001",
        MotiveKind::Guard => "00",
        MotiveKind::Defeat => "01",
        MotiveKind::Reach => "02",
        MotiveKind::Custom(9) => "0309000000",
        motive => "02010100000002000000030000000004000000",
        OnLost::Return => "00",
        leash => "010300000000010600000002000000",
        OnLost::Keep => "02",
        Leeway { leash, replan: true, widen: false } => "0103000000000106000000020000000100",
        Detour::Engage(1) => "0001000000",
        Detour::Flee { from: Where::Subject(2), to: Some([3, 4, 5]) } => "01010200000001030000000400000005000000",
        Detour::Custom { kind: 6, at: Where::Here, param: 7 } => "02060000000407000000",
        BreakReason::Fail(Fail::Custom(1)) => "000401000000",
        BreakReason::Impossible => "01",
        BreakReason::PlanEnded => "02",
        DetourEnd::Defeated => "00",
        DetourEnd::Forgotten => "01",
        DetourEnd::Lost => "02",
        DetourEnd::Leash => "03",
        DetourEnd::Safe => "04",
        DetourEnd::Arrived => "05",
        DetourEnd::Ended => "06",
        Activity::Idle => "00",
        Activity::Plan => "01",
        Activity::Wander => "02",
        Activity::Detour => "03",
        Activity::Returning => "04",
        MissionEvent::StepDone(3) => "000300",
        MissionEvent::Looped => "01",
        MissionEvent::PlanDone => "02",
        MissionEvent::Broken(BreakReason::Impossible) => "0301",
        MissionEvent::Replanned { option: 4 } => "0404000000",
        MissionEvent::FellBack => "05",
        MissionEvent::DetourStarted => "06",
        MissionEvent::DetourEnded(DetourEnd::Safe) => "0704",
        MissionEvent::Returned => "08",
        MissionEvent::Signal(5) => "0905000000",
        MissionEvent::Call { channel: 1, kind: 2, pos: [3, 4, 5], motive } => "0a010000000200000003000000040000000500000002010100000002000000030000000004000000",
        MissionEvent::WantsSquad { tactic: 6, target: Some([7, 8, 9]), subject: Some(1) } => "0b06000000010700000008000000090000000101000000",
    );
}
