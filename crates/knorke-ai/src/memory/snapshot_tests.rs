//! Snapshots of the memory blocks (ADR 0010, ADR 0012): round trips that continue
//! identically, restores that reuse buffers, and rejected inconsistent data.

use knorke_types::{Reader, SnapshotValue, Writer};

use super::*;

fn save<T: SnapshotValue>(v: &T) -> Vec<u8> {
    let mut b = Vec::new();
    Writer::new(&mut b).put(v);
    b
}

fn load<T: SnapshotValue>(b: &[u8]) -> Result<T, knorke_types::SnapshotError> {
    let mut r = Reader::new(b);
    let v = r.get()?;
    r.finish("test")?;
    Ok(v)
}

fn msg(subject: u32, keep: u8) -> Message {
    Message {
        kind: 1,
        subject,
        pos: [subject as i32, 2, 0],
        seen_at: subject,
        strength: 7,
        data: 9,
        sender: 3,
        keep_if_sender_gone: keep,
    }
}

/// A capacity, then a list of `n` items written by `item`.
fn bounded(capacity: u64, n: usize, item: impl Fn(&mut Writer<'_>, usize)) -> Vec<u8> {
    let mut b = Vec::new();
    let mut w = Writer::new(&mut b);
    w.put(&capacity);
    w.put_len(n);
    for i in 0..n {
        item(&mut w, i);
    }
    b
}

#[test]
fn plain_values_round_trip() {
    let mut p = Progress::default();
    p.start(40, 3);
    assert!(p.update(30, 9, 5));
    assert_eq!(load::<Progress>(&save(&p)).unwrap(), p);
    let mut i = Interval::default();
    i.mark(17);
    assert_eq!(load::<Interval>(&save(&i)).unwrap(), i);
    for rule in [
        Route::Nearest(3),
        Route::InRange(-4),
        Route::All,
        Route::To(8),
    ] {
        assert_eq!(load::<Route>(&save(&rule)).unwrap(), rule);
    }
    for m in [
        Metric::Euclid,
        Metric::Manhattan,
        Metric::Chebyshev,
        Metric::Hex,
    ] {
        assert_eq!(load::<Metric>(&save(&m)).unwrap(), m);
    }
    assert!(load::<Route>(&[4]).is_err());
    assert!(load::<Metric>(&[4]).is_err());
}

#[test]
fn timers_round_trip_and_continue() {
    let mut t = Timers::new(3).unwrap();
    t.set(1, 0, 10);
    t.set(2, 2, 4);
    t.set(3, 3, 20);
    t.set(4, 5, 8); // full: replaces the one with the least time left
    let bytes = save(&t);
    let mut back = Timers::new(1).unwrap();
    back.get_into(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(save(&back), bytes);
    for now in 5..30 {
        for id in 0..6 {
            assert_eq!(back.left(id, now), t.left(id, now), "id {id} at {now}");
        }
    }
    t.set(9, 6, 3);
    back.set(9, 6, 3);
    assert_eq!(save(&back), save(&t));
}

#[test]
fn timers_reject_bad_data() {
    let item = |w: &mut Writer<'_>, i: usize| {
        w.put(&(i as u32));
        w.put(&0u32);
        w.put(&5u32);
    };
    assert!(load::<Timers>(&bounded(2, 2, item)).is_ok());
    assert!(
        load::<Timers>(&bounded(0, 0, item)).is_err(),
        "zero capacity"
    );
    assert!(
        load::<Timers>(&bounded(1, 2, item)).is_err(),
        "over capacity"
    );
    let twice = |w: &mut Writer<'_>, _: usize| item(w, 7);
    let mut t = Timers::new(4).unwrap();
    t.set(1, 0, 3);
    assert!(
        t.get_into(&mut Reader::new(&bounded(4, 2, twice))).is_err(),
        "repeated id"
    );
    assert!(t.is_empty(), "a failed restore leaves no half state");
}

#[test]
fn areas_round_trip_and_continue() {
    let mut a = Areas::new(2).unwrap();
    let area = |kind, x| Area {
        kind,
        pos: [x, 0, 0],
        radius: 3,
        strength: 4,
        lifetime: 20,
        blocking: u8::from(kind == 2),
        ..Area::default()
    };
    a.add(0, area(1, 0));
    a.add(5, area(2, 10));
    let bytes = save(&a);
    let mut back = Areas::new(9).unwrap();
    back.add(0, area(3, 3));
    back.get_into(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(save(&back), bytes);
    assert_eq!(
        back.iter().collect::<Vec<_>>(),
        a.iter().collect::<Vec<_>>()
    );
    // Same capacity: a third area evicts the oldest in both.
    a.add(7, area(4, 20));
    back.add(7, area(4, 20));
    assert_eq!(save(&back), save(&a));
    assert_eq!(back.expire(21), a.expire(21));
    assert_eq!(save(&back), save(&a));
}

#[test]
fn areas_reject_bad_data() {
    let item = |w: &mut Writer<'_>, i: usize| {
        w.put(&Area {
            kind: i as u32,
            ..Area::default()
        })
    };
    assert!(load::<Areas>(&bounded(3, 3, item)).is_ok());
    assert!(load::<Areas>(&bounded(0, 0, item)).is_err());
    assert!(load::<Areas>(&bounded(2, 3, item)).is_err());
    let truncated = bounded(3, 3, item);
    assert!(load::<Areas>(&truncated[..truncated.len() - 1]).is_err());
}

#[test]
fn outbox_and_inbox_round_trip_and_continue() {
    let mut out = Outbox::new(4).unwrap();
    out.post(0, msg(1, 0), 5, 2).unwrap();
    out.post(3, msg(2, 1), 1, 2).unwrap();
    let mut inbox = Inbox::new(2).unwrap();
    assert!(inbox.push(msg(5, 0), 10));
    assert!(inbox.push(msg(6, 0), 4));

    let ob = save(&out);
    let ib = save(&inbox);
    let mut out2 = Outbox::new(1).unwrap();
    let mut in2 = Inbox::new(1).unwrap();
    out2.get_into(&mut Reader::new(&ob)).unwrap();
    in2.get_into(&mut Reader::new(&ib)).unwrap();
    assert_eq!(save(&out2), ob);
    assert_eq!(save(&in2), ib);

    // The cooldown survives the restore.
    assert_eq!(out.post(4, msg(3, 0), 0, 2), out2.post(4, msg(3, 0), 0, 2));
    let (mut a, mut b) = (Vec::new(), Vec::new());
    out.due(5, &mut a);
    out2.due(5, &mut b);
    assert_eq!(a, b);
    // The capacity survives: a full inbox drops the same message.
    assert_eq!(inbox.push(msg(7, 0), 1), in2.push(msg(7, 0), 1));
    inbox.take_due(9, &mut a);
    in2.take_due(9, &mut b);
    assert_eq!(a, b);
    assert_eq!(save(&inbox), save(&in2));
    assert_eq!(save(&out), save(&out2));
}

#[test]
fn restore_reuses_message_buffers() {
    let mut inbox = Inbox::new(8).unwrap();
    for i in 0..8 {
        assert!(inbox.push(msg(i, 0), i));
    }
    let bytes = save(&inbox);
    let mut back = Inbox::new(8).unwrap();
    for _ in 0..3 {
        back.get_into(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(save(&back), bytes);
    }
    assert_eq!(back.len(), 8);
}

#[test]
fn outbox_and_inbox_reject_bad_data() {
    let posted = |w: &mut Writer<'_>, i: usize| {
        w.put(&msg(i as u32, 0));
        w.put(&0u32);
        w.put(&1u32);
    };
    let with_last = |mut b: Vec<u8>| {
        Writer::new(&mut b).put(&Some(3u32));
        b
    };
    assert!(load::<Outbox>(&with_last(bounded(2, 2, posted))).is_ok());
    assert!(load::<Outbox>(&with_last(bounded(0, 0, posted))).is_err());
    assert!(load::<Outbox>(&with_last(bounded(1, 2, posted))).is_err());
    let mut bad_tag = bounded(2, 1, posted);
    bad_tag.push(2); // option tag
    let mut o = Outbox::new(2).unwrap();
    o.post(0, msg(1, 0), 0, 0).unwrap();
    assert!(o.get_into(&mut Reader::new(&bad_tag)).is_err());
    assert_eq!(o.pending(), 0);

    let queued = |w: &mut Writer<'_>, i: usize| {
        w.put(&msg(i as u32, 0));
        w.put(&(i as u32));
    };
    assert!(load::<Inbox>(&bounded(2, 2, queued)).is_ok());
    assert!(load::<Inbox>(&bounded(0, 0, queued)).is_err());
    assert!(load::<Inbox>(&bounded(1, 2, queued)).is_err());
}

#[test]
fn goal_guess_round_trips_and_continues() {
    let mut g = GoalGuess::new(GoalGuessConfig::default(), 3);
    g.set_prior(&[1, 0, 3]);
    g.observe(&[-10, 10, 0], &[-10, 10, 0, 10, -10, 0, 0, 0, 0], 3)
        .unwrap();
    let bytes = save(&g);
    let mut back: GoalGuess = load(&bytes).unwrap();
    assert_eq!(back, g);
    let step = ([10, -10, 0], [10, -10, 0, -10, 10, 0, 0, 0, 0]);
    g.observe(&step.0, &step.1, 3).unwrap();
    back.observe(&step.0, &step.1, 3).unwrap();
    assert_eq!(back, g);
    let empty = GoalGuess::new(GoalGuessConfig::default(), 0);
    assert_eq!(load::<GoalGuess>(&save(&empty)).unwrap(), empty);
}

#[test]
fn goal_guess_rejects_bad_chances() {
    let good = GoalGuess::new(GoalGuessConfig::default(), 2);
    let bytes = save(&good);
    // Layout: config 12 bytes, goals 1, prior 8 × u32, chance 8 × u32.
    let goals = 12;
    let prior = |i: usize| 13 + 4 * i;
    let chance = |i: usize| 13 + 32 + 4 * i;
    let patched = |at: usize, v: &[u8]| {
        let mut b = bytes.clone();
        b[at..at + v.len()].copy_from_slice(v);
        b
    };
    assert!(
        load::<GoalGuess>(&patched(goals, &[9])).is_err(),
        "too many goals"
    );
    assert!(
        load::<GoalGuess>(&patched(goals, &[1])).is_err(),
        "entries past the goals"
    );
    assert!(
        load::<GoalGuess>(&patched(goals, &[0])).is_err(),
        "no goals, chances left"
    );
    assert!(
        load::<GoalGuess>(&patched(chance(0), &1u32.to_le_bytes())).is_err(),
        "chances do not sum up"
    );
    assert!(
        load::<GoalGuess>(&patched(prior(0), &1u32.to_le_bytes())).is_err(),
        "prior does not sum up"
    );
    // A chance for a goal the prior rules out: prior [65536, 0], chance [65535, 1].
    let mut b = patched(prior(0), &65536u32.to_le_bytes());
    b[prior(1)..prior(1) + 4].copy_from_slice(&0u32.to_le_bytes());
    b[chance(0)..chance(0) + 4].copy_from_slice(&65535u32.to_le_bytes());
    b[chance(1)..chance(1) + 4].copy_from_slice(&1u32.to_le_bytes());
    assert!(
        load::<GoalGuess>(&b).is_err(),
        "chance for a ruled-out goal"
    );
    b[chance(0)..chance(0) + 4].copy_from_slice(&65536u32.to_le_bytes());
    b[chance(1)..chance(1) + 4].copy_from_slice(&0u32.to_le_bytes());
    assert!(load::<GoalGuess>(&b).is_ok());
}
