//! Builders shared by the group tests (`tests.rs`, `rules_tests.rs`, `snapshot_tests.rs`,
//! `walk_tests.rs`, `crew_tests.rs`): one definition per scene part, so the files cannot
//! drift apart. Test-only.

use super::*;
use crate::memory::{Fact, Message, Metric};

/// Fact kind of the enemies in every group test.
pub(super) const ENEMY: u32 = 1;
/// Seed of every plan and check in the group tests.
pub(super) const SEED: u64 = 7;
/// Channel of every member but the leader: 2 ticks plus 1 per cell.
pub(super) const RADIO: Channel = Channel::Radio {
    base: 2,
    per_unit: 1,
};

// ---- open squads: member tuples `(id, x, y, roles)`, skill 0 -------------------------

/// Room for 8 members, 8 facts, 4 areas and 8 reports.
pub(super) fn config(metric: Metric) -> SquadConfig {
    SquadConfig {
        members: 8,
        memory: 8,
        areas: 4,
        inbox: 8,
        metric,
        ..SquadConfig::default()
    }
}

/// A squad of `members` `(id, x, y, roles)` (active, skill 0) hunting [`ENEMY`]; the first
/// member leads.
pub(super) fn squad_with(config: SquadConfig, members: &[(u32, i32, i32, u32)]) -> Squad {
    let mut sq = Squad::new(config).unwrap();
    for &(id, x, y, roles) in members {
        sq.group_mut().add(Member {
            id,
            pos: [x, y, 0],
            roles,
            skill: 0,
            state: MemberState::Active,
        });
    }
    sq.set_target_kind(ENEMY);
    if let Some(&(id, ..)) = members.first() {
        assert!(sq.set_leader(id));
    }
    sq
}

/// [`squad_with`] the default [`config`].
pub(super) fn squad_of(metric: Metric, members: &[(u32, i32, i32, u32)]) -> Squad {
    squad_with(config(metric), members)
}

/// Four members without roles on a 2 x 2 block at the origin.
pub(super) fn four() -> [(u32, i32, i32, u32); 4] {
    [(1, 0, 0, 0), (2, 1, 0, 0), (3, 0, 1, 0), (4, 1, 1, 0)]
}

/// An [`ENEMY`] fact about `subject` at `(x, y)`, seen at `now`.
pub(super) fn enemy(subject: u32, x: i32, y: i32, now: u32) -> Fact {
    Fact {
        subject,
        kind: ENEMY,
        pos: [x, y, 0],
        seen_at: now,
        ..Fact::default()
    }
}

/// The squad itself sees enemy `id` at `(x, y)` at `now`.
pub(super) fn see(sq: &mut Squad, id: u32, x: i32, y: i32, now: u32) {
    sq.knowledge_mut().learn(now, enemy(id, x, y, now));
}

pub(super) fn spot(x: i32, y: i32, tags: u32) -> Spot {
    Spot {
        pos: [x, y, 0],
        tags,
    }
}

// ---- the flank scene (the former `examples/squad.rs` of sagax-decide) -----------------

/// The members of the flank scene: id, x, y.
pub(super) const MEMBERS: [(u32, i32, i32); 4] = [(1, 0, 0), (2, 0, 2), (3, 0, -2), (4, -3, 0)];
/// Skill of the squad and of every member in the flank scene.
pub(super) const SKILL: u8 = 192;

pub(super) fn flank_spots() -> [Spot; 4] {
    [
        spot(16, 0, TAG_COVER),
        spot(16, 2, 0),
        spot(20, 4, TAG_HIGH),
        spot(20, -4, 0),
    ]
}

/// The flank scene's squad config with an inbox of `inbox` reports.
pub(super) fn flank_config(inbox: usize) -> SquadConfig {
    SquadConfig {
        members: 4,
        memory: 8,
        areas: 4,
        inbox,
        metric: Metric::Chebyshev,
        ..SquadConfig::default()
    }
}

/// An active flank-scene member of skill [`SKILL`].
pub(super) fn flank_member(id: u32, x: i32, y: i32) -> Member {
    Member {
        id,
        pos: [x, y, 0],
        roles: 0,
        skill: SKILL,
        state: MemberState::Active,
    }
}

/// Everything of the flank scene after the members are in: channels, leader, skill,
/// tactics and member 2's report of enemy 11 at (24, 10), seen at `t0`.
pub(super) fn setup_flank(sq: &mut Squad, t0: u32) {
    for (id, ..) in MEMBERS {
        if id != 1 {
            assert!(sq.set_channel(id, RADIO));
        }
    }
    assert!(sq.set_leader(1));
    sq.set_skill(SKILL);
    sq.set_target_kind(ENEMY);
    sq.add_tactic(tactics::rush());
    sq.add_tactic(tactics::formation(Shape::Line, 2));
    sq.add_tactic(tactics::flank(3, 4));
    assert!(sq.report(Message::enemy(2, enemy(11, 24, 10, t0)), t0));
}

/// What the leader sees in the flank scene: enemy 10 at (20, 0).
pub(super) fn leader_sees(t0: u32) -> Fact {
    enemy(10, 20, 0, t0)
}

/// The whole flank scene as a crew, started at `t0`.
pub(super) fn flank_crew(inbox: usize, t0: u32) -> Crew {
    let mut crew = Crew::new(CrewConfig {
        squad: flank_config(inbox),
        ..CrewConfig::default()
    })
    .unwrap();
    for (id, x, y) in MEMBERS {
        assert!(crew.add_member(flank_member(id, x, y)).is_some());
    }
    setup_flank(crew.squad_mut(), t0);
    crew.observe(t0, leader_sees(t0));
    crew
}
