//! Snapshot support for the group types (ADR 0010, ADR 0012).
//!
//! [`Squad`], [`Crew`] and [`GoalKeeper`] are owned by the host and saved as values inside
//! the host's own section (no section id, no version of their own), like
//! [`Knowledge`](crate::memory::Knowledge). Each snapshot carries its capacities, so it can be
//! restored into any object (one built with the same config allocates nothing). Scratch
//! buffers are not saved.

use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

use super::crew::{Crew, Seat};
use super::keeper::GoalKeeper;
use super::members::{Group, Member, MemberState};
use super::orders::{Channel, Order, Status};
use super::skill::{MAX_SKILL_ROWS, SkillTable};
use super::spots::{Spot, SpotPick};
use super::squad::{
    Active, DangerReport, Goal, MAX_EXTRA_COLUMNS, MAX_ROLE_COUNTS, Plan, RoleCounts, Squad,
    SquadConfig,
};
use super::tactic::{Abort, Compare, GoalKind, Place, Side, Slot, Start, Tactic, Trigger};
use crate::Decider;
use crate::memory::Metric;

const fn bad(what: &'static str) -> SnapshotError {
    SnapshotError::Invalid(what)
}

fn put_usize(w: &mut Writer<'_>, v: usize) {
    w.put(&(v as u64));
}

fn get_usize(r: &mut Reader<'_>, what: &'static str) -> Result<usize, SnapshotError> {
    usize::try_from(r.get::<u64>()?).map_err(|_| bad(what))
}

/// Reads an `Option<T>` into `opt`, reusing the allocations of a held value.
fn get_option_into<T: SnapshotValue>(
    opt: &mut Option<T>,
    r: &mut Reader<'_>,
) -> Result<(), SnapshotError> {
    match r.get::<u8>()? {
        0 => {
            *opt = None;
            Ok(())
        }
        1 => match opt {
            Some(v) => v.get_into(r),
            None => {
                *opt = Some(r.get()?);
                Ok(())
            }
        },
        _ => Err(bad("option tag must be 0 or 1")),
    }
}

/// True when the ids are all different.
fn unique(ids: impl Iterator<Item = u32> + Clone) -> bool {
    ids.clone()
        .enumerate()
        .all(|(i, a)| ids.clone().skip(i + 1).all(|b| b != a))
}

impl SnapshotValue for Side {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Side::Front,
            1 => Side::Left,
            2 => Side::Right,
            3 => Side::Rear,
            _ => return Err(bad("knorke.ai: squad side")),
        })
    }
}

impl SnapshotValue for Place {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Place::Offset(p) => {
                w.put(&0u8);
                w.put(&p);
            }
            Place::Side(s) => {
                w.put(&1u8);
                w.put(&s);
            }
            Place::Between => w.put(&2u8),
            Place::Tagged(t) => {
                w.put(&3u8);
                w.put(&t);
            }
            Place::Here => w.put(&4u8),
            Place::Custom(id) => {
                w.put(&5u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Place::Offset(r.get()?),
            1 => Place::Side(r.get()?),
            2 => Place::Between,
            3 => Place::Tagged(r.get()?),
            4 => Place::Here,
            5 => Place::Custom(r.get()?),
            _ => return Err(bad("knorke.ai: squad place")),
        })
    }
}

impl SnapshotValue for GoalKind {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            GoalKind::Move => w.put(&0u8),
            GoalKind::Hold => w.put(&1u8),
            GoalKind::Attack => w.put(&2u8),
            GoalKind::Wait => w.put(&3u8),
            GoalKind::Retreat => w.put(&4u8),
            GoalKind::Custom(id) => {
                w.put(&5u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => GoalKind::Move,
            1 => GoalKind::Hold,
            2 => GoalKind::Attack,
            3 => GoalKind::Wait,
            4 => GoalKind::Retreat,
            5 => GoalKind::Custom(r.get()?),
            _ => return Err(bad("knorke.ai: squad goal kind")),
        })
    }
}

impl SnapshotValue for Start {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Start::Now => w.put(&0u8),
            Start::AfterTicks(n) => {
                w.put(&1u8);
                w.put(&n);
            }
            Start::OnSignal => w.put(&2u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Start::Now,
            1 => Start::AfterTicks(r.get()?),
            2 => Start::OnSignal,
            _ => return Err(bad("knorke.ai: squad start")),
        })
    }
}

impl SnapshotValue for Slot {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.roles);
        w.put(&self.min);
        w.put(&self.max);
        w.put(&self.gather);
        w.put(&self.place);
        w.put(&self.kind);
        w.put(&self.start);
        w.put(&self.want_tags);
        w.put(&self.dist);
        w.put(&self.gather_dist);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Slot {
            roles: r.get()?,
            min: r.get()?,
            max: r.get()?,
            gather: r.get()?,
            place: r.get()?,
            kind: r.get()?,
            start: r.get()?,
            want_tags: r.get()?,
            dist: r.get()?,
            gather_dist: r.get()?,
        })
    }
}

impl SnapshotValue for Trigger {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Trigger::None => w.put(&0u8),
            Trigger::AllArrived { max_wait } => {
                w.put(&1u8);
                w.put(&max_wait);
            }
            Trigger::TargetInRadius(d) => {
                w.put(&2u8);
                w.put(&d);
            }
            Trigger::Manual => w.put(&3u8),
            Trigger::Custom(id) => {
                w.put(&4u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Trigger::None,
            1 => Trigger::AllArrived { max_wait: r.get()? },
            2 => Trigger::TargetInRadius(r.get()?),
            3 => Trigger::Manual,
            4 => Trigger::Custom(r.get()?),
            _ => return Err(bad("knorke.ai: squad trigger")),
        })
    }
}

impl SnapshotValue for Compare {
    fn put(&self, w: &mut Writer<'_>) {
        let tag: u8 = match self {
            Compare::AtLeast => 0,
            Compare::AtMost => 1,
            Compare::Equal => 2,
            Compare::NotEqual => 3,
        };
        w.put(&tag);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Compare::AtLeast,
            1 => Compare::AtMost,
            2 => Compare::Equal,
            3 => Compare::NotEqual,
            _ => return Err(bad("knorke.ai: squad compare")),
        })
    }
}

impl SnapshotValue for Abort {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Abort::When { input, cmp, value } => {
                w.put(&0u8);
                put_usize(w, input);
                w.put(&cmp);
                w.put(&value);
            }
            Abort::Custom(id) => {
                w.put(&1u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Abort::When {
                input: get_usize(r, "knorke.ai: squad abort input")?,
                cmp: r.get()?,
                value: r.get()?,
            },
            1 => Abort::Custom(r.get()?),
            _ => return Err(bad("knorke.ai: squad abort")),
        })
    }
}

/// A tactic as data. Restore reuses the slot, consideration and abort buffers.
impl SnapshotValue for Tactic {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.slots);
        w.put(&self.considerations);
        w.put(&self.weight);
        w.put(&self.aborts);
        w.put(&self.trigger);
        w.put(&self.min_skill);
        w.put(&self.needs_target);
        w.put(&self.abort_if_target_lost);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut t = Tactic {
            slots: Vec::new(),
            considerations: Vec::new(),
            weight: 0,
            aborts: Vec::new(),
            trigger: Trigger::None,
            min_skill: 0,
            needs_target: false,
            abort_if_target_lost: false,
        };
        t.get_into(r)?;
        Ok(t)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.slots.get_into(r)?;
        self.considerations.get_into(r)?;
        self.weight = r.get()?;
        self.aborts.get_into(r)?;
        self.trigger = r.get()?;
        self.min_skill = r.get()?;
        self.needs_target = r.get()?;
        self.abort_if_target_lost = r.get()?;
        Ok(())
    }
}

/// All eight rows of every column and the row count as stored (a table changed by hand may
/// have any `rows`; [`Squad`] restore checks its own table).
impl SnapshotValue for SkillTable {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.rows);
        for p in &self.pick {
            w.put(p);
        }
        w.put(&self.think_ticks);
        w.put(&self.spot_rank_error);
        w.put(&self.timing_spread);
        w.put(&self.hold_bonus);
        w.put(&self.give_up_ticks);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let rows = r.get()?;
        let mut pick = [crate::Pick::Best; MAX_SKILL_ROWS];
        for p in &mut pick {
            *p = r.get()?;
        }
        Ok(SkillTable {
            rows,
            pick,
            think_ticks: r.get()?,
            spot_rank_error: r.get()?,
            timing_spread: r.get()?,
            hold_bonus: r.get()?,
            give_up_ticks: r.get()?,
        })
    }
}

/// Rejects a role list longer than [`MAX_ROLE_COUNTS`].
impl SnapshotValue for RoleCounts {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.len);
        w.put(&self.masks);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let len: u8 = r.get()?;
        let masks = r.get()?;
        if usize::from(len) > MAX_ROLE_COUNTS {
            return Err(bad("knorke.ai: squad role count"));
        }
        Ok(RoleCounts { masks, len })
    }
}

impl SnapshotValue for Spot {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.pos);
        w.put(&self.tags);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Spot {
            pos: r.get()?,
            tags: r.get()?,
        })
    }
}

impl SnapshotValue for SpotPick {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            SpotPick::Spot(i) => {
                w.put(&0u8);
                put_usize(w, i);
            }
            SpotPick::Unchecked(p) => {
                w.put(&1u8);
                w.put(&p);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => SpotPick::Spot(get_usize(r, "knorke.ai: spot index")?),
            1 => SpotPick::Unchecked(r.get()?),
            _ => return Err(bad("knorke.ai: spot pick")),
        })
    }
}

impl SnapshotValue for Goal {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.kind);
        w.put(&self.pos);
        w.put(&self.unchecked);
        w.put(&self.then_pos);
        w.put(&self.then_unchecked);
        w.put(&self.target);
        w.put(&self.start);
        w.put(&self.slot);
        w.put(&self.plan);
        w.put(&self.issued_at);
        w.put(&self.start_delay);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Goal {
            kind: r.get()?,
            pos: r.get()?,
            unchecked: r.get()?,
            then_pos: r.get()?,
            then_unchecked: r.get()?,
            target: r.get()?,
            start: r.get()?,
            slot: r.get()?,
            plan: r.get()?,
            issued_at: r.get()?,
            start_delay: r.get()?,
        })
    }
}

/// A plan with its goals. Restore reuses the goal buffer; tactic and slot indexes are checked
/// by [`Squad`] restore, which knows the tactics.
impl SnapshotValue for Plan {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.id);
        put_usize(w, self.tactic);
        w.put(&self.target);
        w.put(&self.goals);
        w.put(&self.made_at);
        w.put(&self.active_at);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut p = Plan {
            id: 0,
            tactic: 0,
            target: None,
            goals: Vec::new(),
            made_at: 0,
            active_at: 0,
        };
        p.get_into(r)?;
        Ok(p)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.id = r.get()?;
        self.tactic = get_usize(r, "knorke.ai: squad plan tactic")?;
        self.target = r.get()?;
        self.goals.get_into(r)?;
        self.made_at = r.get()?;
        self.active_at = r.get()?;
        Ok(())
    }
}

impl SnapshotValue for Order {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.member);
        w.put(&self.goal);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Order {
            member: r.get()?,
            goal: r.get()?,
        })
    }
}

impl SnapshotValue for Channel {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Channel::Shout { range } => {
                w.put(&0u8);
                w.put(&range);
            }
            Channel::Radio { base, per_unit } => {
                w.put(&1u8);
                w.put(&base);
                w.put(&per_unit);
            }
            Channel::None => w.put(&2u8),
            Channel::Custom(id) => {
                w.put(&3u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Channel::Shout { range: r.get()? },
            1 => Channel::Radio {
                base: r.get()?,
                per_unit: r.get()?,
            },
            2 => Channel::None,
            3 => Channel::Custom(r.get()?),
            _ => return Err(bad("knorke.ai: squad channel")),
        })
    }
}

impl SnapshotValue for Status {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Status::Keep => w.put(&0u8),
            Status::Replan(id) => {
                w.put(&1u8);
                w.put(&id);
            }
            Status::Signal => w.put(&2u8),
            Status::NoPlan => w.put(&3u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Status::Keep,
            1 => Status::Replan(r.get()?),
            2 => Status::Signal,
            3 => Status::NoPlan,
            _ => return Err(bad("knorke.ai: squad status")),
        })
    }
}

impl SnapshotValue for MemberState {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => MemberState::Active,
            1 => MemberState::Suspended,
            2 => MemberState::Gone,
            _ => return Err(bad("knorke.ai: squad member state")),
        })
    }
}

impl SnapshotValue for Member {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.id);
        w.put(&self.pos);
        w.put(&self.roles);
        w.put(&self.skill);
        w.put(&self.state);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Member {
            id: r.get()?,
            pos: r.get()?,
            roles: r.get()?,
            skill: r.get()?,
            state: r.get()?,
        })
    }
}

/// The members with the capacity. Restore reuses the buffer and rejects a zero capacity,
/// more members than the capacity and a repeated id (the group is then left empty).
impl SnapshotValue for Group {
    fn put(&self, w: &mut Writer<'_>) {
        put_usize(w, self.capacity);
        w.put(&self.members);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut g = Group {
            members: Vec::new(),
            capacity: 1,
        };
        g.get_into(r)?;
        Ok(g)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let capacity = crate::memory::get_bounded(
            r,
            &mut self.members,
            bad("knorke.ai: squad group capacity"),
        )?;
        if !unique(self.members.iter().map(|m| m.id)) {
            self.members.clear();
            return Err(bad("knorke.ai: squad duplicate member id"));
        }
        self.capacity = capacity;
        Ok(())
    }
}

impl SnapshotValue for Active {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.plan);
        w.put(&self.since);
        w.put(&self.signal_at);
        w.put(&self.arrived);
        w.put(&self.broken);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut a = Active {
            plan: Plan {
                id: 0,
                tactic: 0,
                target: None,
                goals: Vec::new(),
                made_at: 0,
                active_at: 0,
            },
            since: 0,
            signal_at: None,
            arrived: Vec::new(),
            broken: Vec::new(),
        };
        a.get_into(r)?;
        Ok(a)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.plan.get_into(r)?;
        self.since = r.get()?;
        self.signal_at = r.get()?;
        self.arrived.get_into(r)?;
        self.broken.get_into(r)
    }
}

impl SnapshotValue for DangerReport {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.sender);
        w.put(&self.centre);
        w.put(&self.shooter);
        w.put(&self.arrive_at);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(DangerReport {
            sender: r.get()?,
            centre: r.get()?,
            shooter: r.get()?,
            arrive_at: r.get()?,
        })
    }
}

/// Every goal of `plan` names a slot of its tactic.
fn plan_ok(plan: &Plan, tactics: &[Tactic]) -> bool {
    tactics.get(plan.tactic).is_some_and(|t| {
        plan.goals
            .iter()
            .all(|(_, g)| usize::from(g.slot) < t.slots.len())
    })
}

/// The whole squad state: the group, the leader's knowledge, areas and inbox, the danger
/// reports on their way, the tactics (they can be added at run time) with the decider, skill
/// and skill table, leader, target kind, row columns, the pending and active plan, the orders
/// on their way, the channels, the danger template, the spot radius and the next plan id.
/// The capacities (members, memory, areas, inbox, dangers), metric, role list and rule
/// columns are part of the snapshot, so it restores into any squad; one built with the same
/// [`SquadConfig`] allocates nothing on restore. The planning scratch is not saved.
///
/// Restore rejects (with [`SnapshotError::Invalid`]) a zero or exceeded capacity, a repeated
/// member id or channel id, more danger reports than their capacity, a role list longer than
/// [`MAX_ROLE_COUNTS`], a skill table with `rows` outside 1 ..= [`MAX_SKILL_ROWS`], more than
/// [`MAX_EXTRA_COLUMNS`] caller or rule columns, and a pending or active plan whose tactic or
/// goal slot does not exist, plus every bad enum tag. On an error the squad is left empty of
/// members, tactics, plans, orders and reports (safe to use, not the saved state).
impl SnapshotValue for Squad {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.group);
        w.put(&self.knowledge);
        w.put(&self.areas);
        w.put(&self.inbox);
        put_usize(w, self.dangers_cap);
        w.put(&self.dangers);
        w.put(&self.activated);
        w.put(&self.metric);
        w.put(&self.roles);
        w.put(&self.tactics);
        w.put(&self.decider);
        w.put(&self.skill);
        w.put(&self.table);
        w.put(&self.leader);
        w.put(&self.target_kind);
        put_usize(w, self.extra_columns);
        put_usize(w, self.rule_columns);
        w.put(&self.pending);
        w.put(&self.active);
        w.put(&self.orders);
        w.put(&self.channels);
        w.put(&self.danger);
        w.put(&self.spot_radius);
        w.put(&self.next_id);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut s =
            Squad::new(SquadConfig::default()).map_err(|_| bad("knorke.ai: squad config"))?;
        s.get_into(r)?;
        Ok(s)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let result = self.read_into(r);
        if result.is_err() {
            self.group.members.clear();
            self.dangers.clear();
            self.activated = None;
            self.tactics.clear();
            self.decider = Decider::new();
            self.leader = None;
            self.pending = None;
            self.active = None;
            self.orders.clear();
            self.channels.clear();
        }
        result
    }
}

impl Squad {
    fn read_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.group.get_into(r)?;
        self.knowledge.get_into(r)?;
        self.areas.get_into(r)?;
        self.inbox.get_into(r)?;
        let dangers_cap = get_usize(r, "knorke.ai: squad danger capacity")?;
        self.dangers.get_into(r)?;
        if dangers_cap == 0 || self.dangers.len() > dangers_cap {
            return Err(bad("knorke.ai: squad danger capacity"));
        }
        self.activated = r.get()?;
        let metric: Metric = r.get()?;
        let roles: RoleCounts = r.get()?;
        self.tactics.get_into(r)?;
        self.decider.get_into(r)?;
        self.skill = r.get()?;
        let table: SkillTable = r.get()?;
        if !(1..=MAX_SKILL_ROWS).contains(&usize::from(table.rows)) {
            return Err(bad("knorke.ai: squad skill table rows"));
        }
        self.leader = r.get()?;
        self.target_kind = r.get()?;
        let extra_columns = get_usize(r, "knorke.ai: squad extra columns")?;
        let rule_columns = get_usize(r, "knorke.ai: squad rule columns")?;
        if extra_columns > MAX_EXTRA_COLUMNS || rule_columns > MAX_EXTRA_COLUMNS {
            return Err(bad("knorke.ai: squad columns"));
        }
        get_option_into(&mut self.pending, r)?;
        get_option_into(&mut self.active, r)?;
        let plans_ok = self
            .pending
            .iter()
            .chain(self.active.as_ref().map(|a| &a.plan))
            .all(|p| plan_ok(p, &self.tactics));
        if !plans_ok {
            return Err(bad("knorke.ai: squad plan tactic or slot"));
        }
        self.orders.get_into(r)?;
        self.channels.get_into(r)?;
        if !unique(self.channels.iter().map(|c| c.0)) {
            return Err(bad("knorke.ai: squad duplicate channel"));
        }
        self.danger = r.get()?;
        self.spot_radius = r.get()?;
        self.next_id = r.get()?;
        self.dangers_cap = dangers_cap;
        self.metric = metric;
        self.roles = roles;
        self.table = table;
        self.extra_columns = extra_columns;
        self.rule_columns = rule_columns;
        Ok(())
    }
}

/// The goal, the postpone time and whether the keeper is suspended.
impl SnapshotValue for GoalKeeper {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.goal);
        w.put(&self.postponed_at);
        w.put(&self.suspended);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(GoalKeeper {
            goal: r.get()?,
            postponed_at: r.get()?,
            suspended: r.get()?,
        })
    }
}

impl SnapshotValue for Seat {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.id);
        w.put(&self.keeper);
        w.put(&self.paused);
        w.put(&self.start_at);
        w.put(&self.started);
        w.put(&self.arrived);
        w.put(&self.broken);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Seat {
            id: r.get()?,
            keeper: r.get()?,
            paused: r.get()?,
            start_at: r.get()?,
            started: r.get()?,
            arrived: r.get()?,
            broken: r.get()?,
        })
    }
}

/// The squad (see its snapshot), one seat per member (keeper and what the crew already told),
/// the arrive radius and the hex layout. The metric is the squad's. The per-tick order and
/// spot buffers are scratch and not saved.
///
/// Restore rejects what the squad rejects, more seats than the group capacity, a repeated
/// seat id and a hex layout without [`Metric::Hex`]; on an error the squad is left as after
/// its own error and the seats are cleared.
impl SnapshotValue for Crew {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.squad);
        w.put(&self.seats);
        w.put(&self.arrive_radius);
        w.put(&self.hex);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut c =
            Crew::new(super::CrewConfig::default()).map_err(|_| bad("knorke.ai: crew config"))?;
        c.get_into(r)?;
        Ok(c)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let result = (|| {
            self.squad.get_into(r)?;
            self.seats.get_into(r)?;
            if self.seats.len() > self.squad.group.capacity
                || !unique(self.seats.iter().map(|s| s.id))
            {
                return Err(bad("knorke.ai: crew seats"));
            }
            let arrive_radius = r.get()?;
            let hex: Option<knorke_grid::HexLayout> = r.get()?;
            if hex.is_some() && self.squad.metric != Metric::Hex {
                return Err(bad("knorke.ai: crew hex layout needs the hex metric"));
            }
            self.arrive_radius = arrive_radius;
            self.hex = hex;
            Ok(())
        })();
        if result.is_err() {
            self.seats.clear();
            self.squad.group.members.clear();
            self.hex = None;
        }
        self.metric = self.squad.metric;
        result
    }
}
