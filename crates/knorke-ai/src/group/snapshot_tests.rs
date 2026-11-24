//! Snapshot tests of the group types: round trips that continue identically, buffer reuse
//! on restore, and rejected input.
use super::orders::Status;
use super::squad::DangerReport;
use super::test_support::*;
use super::*;
use crate::memory::Metric;
use knorke_grid::HexLayout;
use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

fn save<T: SnapshotValue>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.put(&mut Writer::new(&mut out));
    out
}

fn load_into<T: SnapshotValue>(v: &mut T, bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut r = Reader::new(bytes);
    v.get_into(&mut r)?;
    assert_eq!(r.remaining(), 0, "the whole snapshot is read");
    Ok(())
}

fn load<T: SnapshotValue>(bytes: &[u8]) -> Result<T, SnapshotError> {
    T::get(&mut Reader::new(bytes))
}

fn squad() -> Squad {
    let mut sq = Squad::new(flank_config(8)).unwrap();
    for (id, x, y) in MEMBERS {
        sq.group_mut().add(flank_member(id, x, y));
    }
    setup_flank(&mut sq, 0);
    sq.observe(0, enemy(10, 20, 0, 0));
    sq
}

/// One tick of a squad: members drift, a danger report now and then; returns what happened.
fn step(sq: &mut Squad, t: u32, out: &mut Vec<Order>) -> (Status, Vec<Order>) {
    for (k, (id, x, y)) in MEMBERS.iter().enumerate() {
        let drift = (t as i32 + k as i32) / 3;
        sq.group_mut().set_pos(*id, [x + drift, *y, 0]);
    }
    if t.is_multiple_of(5) {
        let _ = sq.report_danger(3, [t as i32, 1, 0], Some([20, 0, 0]), t + 2);
    }
    let status = sq.check(t, &[], &flank_spots(), SEED).unwrap();
    out.clear();
    sq.orders_due(t, out);
    (status, out.clone())
}

/// Runs the squad until a plan is active and orders are still on their way; the tick after.
fn run_until_orders_in_flight(sq: &mut Squad) -> u32 {
    let mut out = Vec::new();
    for t in 0..200 {
        step(sq, t, &mut out);
        if sq.active_plan().is_some() && !sq.orders.is_empty() {
            return t + 1;
        }
    }
    panic!("no orders in flight");
}

#[test]
fn squad_round_trip_continues_identically() {
    let mut a = squad();
    let t0 = run_until_orders_in_flight(&mut a);
    assert!(!a.dangers.is_empty() || a.areas().iter().next().is_some());
    let bytes = save(&a);
    let mut b = Squad::new(flank_config(8)).unwrap();
    load_into(&mut b, &bytes).unwrap();
    assert_eq!(save(&b), bytes);
    let c: Squad = load(&bytes).unwrap();
    assert_eq!(save(&c), bytes);

    let (mut oa, mut ob) = (Vec::new(), Vec::new());
    let mut seen_orders = 0;
    for t in t0..t0 + 60 {
        let ra = step(&mut a, t, &mut oa);
        let rb = step(&mut b, t, &mut ob);
        assert_eq!(ra, rb, "tick {t}");
        seen_orders += ra.1.len();
        if t == t0 + 20 {
            assert!(a.signal(t) == b.signal(t));
        }
    }
    assert!(seen_orders > 0, "the restored orders arrive");
    assert_eq!(save(&a), save(&b));
}

#[test]
fn squad_restore_reuses_buffers() {
    let mut a = squad();
    let t0 = run_until_orders_in_flight(&mut a);
    let bytes = save(&a);
    // Run on, then restore the earlier state into the same, used squad.
    let mut out = Vec::new();
    for t in t0..t0 + 10 {
        step(&mut a, t, &mut out);
    }
    let members = a.group.members.as_ptr();
    let slots = a.tactics[0].slots.as_ptr();
    let goals = a.active.as_ref().unwrap().plan.goals.as_ptr();
    let orders_cap = a.orders.capacity();
    load_into(&mut a, &bytes).unwrap();
    assert_eq!(save(&a), bytes);
    assert_eq!(a.group.members.as_ptr(), members);
    assert_eq!(a.tactics[0].slots.as_ptr(), slots);
    assert_eq!(a.active.as_ref().unwrap().plan.goals.as_ptr(), goals);
    assert!(a.orders.capacity() >= orders_cap);
}

/// Saves `bad`, restores it into a working squad and checks the error and the safe state.
fn rejects(bad: &Squad) {
    let bytes = save(bad);
    let mut s = squad();
    let e = load_into(&mut s, &bytes).unwrap_err();
    assert!(matches!(e, SnapshotError::Invalid(m) if m.starts_with("knorke.ai")));
    assert!(s.group().is_empty());
    assert!(s.pending_plan().is_none() && s.active_plan().is_none());
    assert!(s.orders.is_empty() && s.dangers.is_empty() && s.tactics.is_empty());
    // Still usable.
    assert_eq!(s.check(0, &[], &[], 0), Ok(Status::NoPlan));
    assert!(load::<Squad>(&bytes).is_err());
}

#[test]
fn squad_rejects_invalid_state() {
    let mut base = squad();
    run_until_orders_in_flight(&mut base);
    assert!(base.active.is_some());

    let mut s = base.clone();
    s.group.members.push(s.group.members[0]);
    s.group.capacity = 8;
    rejects(&s);

    let mut s = base.clone();
    s.group.capacity = 2;
    rejects(&s);

    let mut s = base.clone();
    s.active.as_mut().unwrap().plan.tactic = 99;
    rejects(&s);

    let mut s = base.clone();
    s.active.as_mut().unwrap().plan.goals[0].1.slot = 200;
    rejects(&s);

    let mut s = base.clone();
    s.pending = Some(s.active.as_ref().unwrap().plan.clone());
    s.pending.as_mut().unwrap().tactic = 3;
    rejects(&s);

    let mut s = base.clone();
    s.dangers_cap = 1;
    s.dangers.clear();
    for _ in 0..2 {
        s.dangers.push(DangerReport {
            sender: 1,
            centre: [0; 3],
            shooter: None,
            arrive_at: 0,
        });
    }
    rejects(&s);

    let mut s = base.clone();
    s.dangers.clear();
    s.dangers_cap = 0;
    rejects(&s);

    let mut s = base.clone();
    s.table.rows = 0;
    rejects(&s);

    let mut s = base.clone();
    s.table.rows = 9;
    rejects(&s);

    let mut s = base.clone();
    s.rule_columns = MAX_EXTRA_COLUMNS + 1;
    rejects(&s);

    let mut s = base.clone();
    s.extra_columns = MAX_EXTRA_COLUMNS + 1;
    rejects(&s);

    let mut s = base.clone();
    s.roles.len = 9;
    rejects(&s);

    let mut s = base.clone();
    s.channels.push(s.channels[0]);
    rejects(&s);
}

#[test]
fn squad_rejects_truncated_input() {
    let bytes = save(&squad());
    for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
        let mut s = squad();
        assert!(load_into(&mut s, &bytes[..cut]).is_err(), "cut {cut}");
        assert!(s.group().is_empty() && s.tactics.is_empty());
    }
}

#[test]
fn enum_tags_are_checked() {
    let bad = [9u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(load::<MemberState>(&bad).is_err());
    assert!(load::<Channel>(&bad).is_err());
    assert!(load::<GoalKind>(&bad).is_err());
    assert!(load::<Place>(&bad).is_err());
    assert!(load::<Side>(&bad).is_err());
    assert!(load::<Start>(&bad).is_err());
    assert!(load::<Trigger>(&bad).is_err());
    assert!(load::<Compare>(&bad).is_err());
    assert!(load::<Abort>(&bad).is_err());
    assert!(load::<Status>(&bad).is_err());
    assert!(load::<SpotPick>(&bad).is_err());
    assert!(load::<GoalKeeper>(&bad).is_err());
    // RoleCounts longer than the list.
    let mut roles = vec![9u8];
    roles.extend([0u8; 32]);
    assert!(load::<RoleCounts>(&roles).is_err());
}

#[test]
fn value_types_round_trip() {
    for pick in [SpotPick::Spot(3), SpotPick::Unchecked([1, -2, 3])] {
        assert_eq!(load::<SpotPick>(&save(&pick)), Ok(pick));
    }
    let spot = Spot {
        pos: [4, 5, 6],
        tags: TAG_HIGH | TAG_COVER,
    };
    assert_eq!(load::<Spot>(&save(&spot)), Ok(spot));
    for status in [
        Status::Keep,
        Status::Replan(7),
        Status::Signal,
        Status::NoPlan,
    ] {
        assert_eq!(load::<Status>(&save(&status)), Ok(status));
    }
    for ch in [
        Channel::Shout { range: -3 },
        RADIO,
        Channel::None,
        Channel::Custom(5),
    ] {
        assert_eq!(load::<Channel>(&save(&ch)), Ok(ch));
    }
    for t in [
        tactics::rush(),
        tactics::flank(3, 4),
        tactics::formation(Shape::Line, 2),
    ] {
        assert_eq!(load::<Tactic>(&save(&t)), Ok(t));
    }
    let table = SkillTable::default();
    assert_eq!(load::<SkillTable>(&save(&table)), Ok(table));
    let roles = RoleCounts::new(&[1, 2, 4]).unwrap();
    assert_eq!(load::<RoleCounts>(&save(&roles)), Ok(roles));
}

#[test]
fn keeper_round_trip() {
    let mut sq = squad();
    run_until_orders_in_flight(&mut sq);
    let goal = sq.active_plan().unwrap().goals[0].1;
    let mut k = GoalKeeper::new();
    k.set(3, goal);
    k.goal_postponed(5);
    k.suspend();
    let bytes = save(&k);
    assert_eq!(load::<GoalKeeper>(&bytes), Ok(k));
    let mut other = GoalKeeper::new();
    load_into(&mut other, &bytes).unwrap();
    assert_eq!(other, k);
    // A bad bool for `suspended`.
    let mut bad = bytes.clone();
    *bad.last_mut().unwrap() = 2;
    assert!(load::<GoalKeeper>(&bad).is_err());
}

fn crew_config() -> CrewConfig {
    CrewConfig {
        squad: flank_config(8),
        ..CrewConfig::default()
    }
}

/// One crew tick: members walk one cell toward their goal every other tick, member 3
/// postpones its goal now and then.
fn crew_step(c: &mut Crew, t: u32, events: &mut Vec<CrewEvent>) -> Vec<CrewEvent> {
    for (id, ..) in MEMBERS {
        let g = c.squad().group();
        let at = g.get(g.index(id).unwrap()).unwrap().pos;
        let to = c.keeper(id).and_then(|k| k.goal()).and_then(|g| g.pos);
        if let Some(to) = to
            && t.is_multiple_of(2)
        {
            let s = |a: i32, b: i32| a + (b - a).signum();
            assert!(c.set_pos(id, [s(at[0], to[0]), s(at[1], to[1]), 0]));
        }
    }
    c.tick(t, &[], &flank_spots(), SEED, events).unwrap();
    for (id, ..) in MEMBERS {
        let won = !(id == 3 && t % 7 < 3);
        c.member_chose(id, t, won);
    }
    events.clone()
}

/// Runs the crew until a plan is active and orders are on their way; the next tick.
fn crew_until_orders_in_flight(c: &mut Crew) -> u32 {
    let mut events = Vec::new();
    for t in 0..200 {
        crew_step(c, t, &mut events);
        if c.squad().active_plan().is_some()
            && !c.squad.orders.is_empty()
            && c.seats.iter().any(|s| s.keeper.goal().is_some())
        {
            return t + 1;
        }
    }
    panic!("no orders in flight");
}

#[test]
fn crew_round_trip_continues_identically() {
    let mut a = flank_crew(8, 0);
    let t0 = crew_until_orders_in_flight(&mut a);
    let bytes = save(&a);
    let mut b = Crew::new(crew_config()).unwrap();
    load_into(&mut b, &bytes).unwrap();
    assert_eq!(save(&b), bytes);
    let c: Crew = load(&bytes).unwrap();
    assert_eq!(save(&c), bytes);

    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    let mut seen = 0;
    for t in t0..t0 + 80 {
        let xa = crew_step(&mut a, t, &mut ea);
        let xb = crew_step(&mut b, t, &mut eb);
        assert_eq!(xa, xb, "tick {t}");
        assert_eq!(a.member_inputs(2, t), b.member_inputs(2, t));
        seen += xa.len();
    }
    assert!(seen > 0);
    assert_eq!(save(&a), save(&b));
}

#[test]
fn crew_restore_reuses_buffers() {
    let mut a = flank_crew(8, 0);
    let t0 = crew_until_orders_in_flight(&mut a);
    let bytes = save(&a);
    let mut events = Vec::new();
    for t in t0..t0 + 10 {
        crew_step(&mut a, t, &mut events);
    }
    let seats = a.seats.as_ptr();
    let members = a.squad.group.members.as_ptr();
    load_into(&mut a, &bytes).unwrap();
    assert_eq!(save(&a), bytes);
    assert_eq!(a.seats.as_ptr(), seats);
    assert_eq!(a.squad.group.members.as_ptr(), members);
}

fn crew_rejects(bad: &Crew) {
    let bytes = save(bad);
    let mut c = flank_crew(8, 0);
    assert!(matches!(
        load_into(&mut c, &bytes),
        Err(SnapshotError::Invalid(_))
    ));
    assert!(c.seats.is_empty() && c.squad().group().is_empty());
    let mut events = Vec::new();
    c.tick(0, &[], &[], 0, &mut events).unwrap();
    assert!(load::<Crew>(&bytes).is_err());
}

#[test]
fn crew_rejects_invalid_state() {
    let mut base = flank_crew(8, 0);
    crew_until_orders_in_flight(&mut base);

    let mut c = base.clone();
    c.seats.push(c.seats[0]);
    crew_rejects(&c);

    let mut c = base.clone();
    let extra = c.seats[0];
    for k in 0..4 {
        c.seats.push(super::crew::Seat {
            id: 100 + k,
            ..extra
        });
    }
    crew_rejects(&c);

    let mut c = base.clone();
    c.hex = Some(HexLayout::OddR);
    crew_rejects(&c);

    // A squad error inside the crew.
    let mut c = base.clone();
    c.squad.active.as_mut().unwrap().plan.tactic = 42;
    crew_rejects(&c);
}

#[test]
fn hex_crew_round_trip() {
    let mut cfg = crew_config();
    cfg.squad.metric = Metric::Hex;
    cfg.hex_layout = Some(HexLayout::OddR);
    let mut a = Crew::new(cfg).unwrap();
    for (id, x, y) in MEMBERS {
        assert!(a.add_member(flank_member(id, x, y)).is_some());
    }
    setup_flank(a.squad_mut(), 0);
    a.observe(0, enemy(10, 20, 0, 0));
    let mut events = Vec::new();
    for t in 0..10 {
        crew_step(&mut a, t, &mut events);
    }
    let bytes = save(&a);
    let b: Crew = load(&bytes).unwrap();
    assert_eq!(save(&b), bytes);
    assert_eq!(b.hex, Some(HexLayout::OddR));
    assert_eq!(b.metric, Metric::Hex);
}
