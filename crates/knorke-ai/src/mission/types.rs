// Ported from Sagax (https://github.com/STARSCrazy/sagax), revision fab00c0, crates/sagax-decide/src/mission/types.rs.
use crate::memory::{Metric, PointPattern, Pos};
use crate::{DecideTrace, Pick};
use knorke_grid::HexLayout;

/// A place a step or detour refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Where {
    /// A fixed position.
    Fixed(Pos),
    /// Where this subject was last known.
    Subject(u32),
    /// The place of the mission's motive.
    MotivePlace,
    /// Where the motive's subject was last known.
    MotiveSubject,
    /// Where the character stands at that moment.
    Here,
}

/// What a step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepKind {
    /// Walk to the place.
    GoTo(Where),
    /// Wait this many ticks.
    Wait(u32),
    /// Wait for the configured pause (`pause_ticks`).
    Pause,
    /// Follow this subject.
    Follow(u32),
    /// Wait until this subject is within `radius` of the character.
    WaitFor {
        /// The subject to wait for.
        subject: u32,
        /// How close it must come.
        radius: u32,
    },
    /// Search around `centre` for this many ticks: go to the points of the
    /// config's search pattern in turn, again and again.
    Search {
        /// The middle of the search.
        centre: Where,
        /// How far from the middle to search, in the mission's metric
        /// (Euclid compares squares): a pattern point farther away is
        /// skipped and the next one taken. 0 is no limit. When every point
        /// is skipped, the character searches at the centre.
        radius: u32,
        /// How long to search.
        ticks: u32,
    },
    /// Hold the place.
    Hold(Where),
    /// Attack this subject.
    Attack(u32),
    /// Wander about. Any `Wander` step ends the plan: the steps after it
    /// never run, and a looping plan with one emits [`MissionEvent::PlanDone`]
    /// instead of looping. The character then wanders until a condition
    /// of the plan fires or the game sets a new plan.
    Wander,
    /// Give this signal (the step is done at once).
    Signal(u32),
    /// A step the game defines.
    Custom {
        /// The game's own kind.
        kind: u32,
        /// Where it happens.
        at: Where,
        /// A free parameter.
        param: u32,
    },
    /// Call for help on a channel.
    Call {
        /// The channel to call on.
        channel: u32,
        /// The kind of call.
        kind: u32,
    },
    /// Ask for a squad tactic against a target.
    Squad {
        /// The tactic to ask for.
        tactic: u32,
        /// What the tactic is aimed at.
        target: Where,
    },
}

/// A condition that breaks a step or a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fail {
    /// The subject is not known (no fact of the subject kind).
    Unknown {
        /// The subject.
        subject: u32,
    },
    /// The subject is defeated.
    Defeated {
        /// The subject.
        subject: u32,
    },
    /// A fact of this kind about the subject has `data & mask == value`.
    Fact {
        /// The subject.
        subject: u32,
        /// The fact kind.
        kind: u32,
        /// Bits of the fact's `data` to look at.
        mask: u32,
        /// The value those bits must have.
        value: u32,
    },
    /// The game's flags have `flags & mask == value`.
    Flags {
        /// Bits to look at.
        mask: u64,
        /// The value those bits must have.
        value: u64,
    },
    /// A condition the game decides (by its own number).
    Custom(u32),
}

/// How many fail conditions a step and a plan can hold each.
pub const MAX_FAILS: usize = 4;

/// Puts `fail` in the first free slot. A fifth one is dropped.
fn add_fail(slots: &mut [Option<Fail>; MAX_FAILS], fail: Fail) {
    match slots.iter_mut().find(|s| s.is_none()) {
        Some(slot) => *slot = Some(fail),
        None => debug_assert!(false, "more than MAX_FAILS fail conditions"),
    }
}

/// One step of a [`Plan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Step {
    /// What the step does.
    pub kind: StepKind,
    /// Conditions that break the step; filled from the front.
    pub fails: [Option<Fail>; MAX_FAILS],
    /// Skip the step instead of breaking the plan when it cannot be done.
    pub skip_if_impossible: bool,
}

impl Step {
    const fn of(kind: StepKind) -> Self {
        Self {
            kind,
            fails: [None; MAX_FAILS],
            skip_if_impossible: false,
        }
    }

    /// Walk to a fixed position.
    pub const fn go_to(pos: Pos) -> Self {
        Self::of(StepKind::GoTo(Where::Fixed(pos)))
    }

    /// Walk to a place.
    pub const fn go_to_where(at: Where) -> Self {
        Self::of(StepKind::GoTo(at))
    }

    /// Wait this many ticks.
    pub const fn wait(ticks: u32) -> Self {
        Self::of(StepKind::Wait(ticks))
    }

    /// Wait for the configured pause.
    pub const fn pause() -> Self {
        Self::of(StepKind::Pause)
    }

    /// Follow a subject.
    pub const fn follow(subject: u32) -> Self {
        Self::of(StepKind::Follow(subject))
    }

    /// Wait until the subject is within `radius`.
    pub const fn wait_for(subject: u32, radius: u32) -> Self {
        Self::of(StepKind::WaitFor { subject, radius })
    }

    /// Search around `centre` within `radius` for `ticks`.
    pub const fn search(centre: Where, radius: u32, ticks: u32) -> Self {
        Self::of(StepKind::Search {
            centre,
            radius,
            ticks,
        })
    }

    /// Hold a place.
    pub const fn hold(at: Where) -> Self {
        Self::of(StepKind::Hold(at))
    }

    /// Attack a subject.
    pub const fn attack(subject: u32) -> Self {
        Self::of(StepKind::Attack(subject))
    }

    /// Wander about; ends the plan (see [`StepKind::Wander`]).
    pub const fn wander() -> Self {
        Self::of(StepKind::Wander)
    }

    /// Give a signal.
    pub const fn signal(signal: u32) -> Self {
        Self::of(StepKind::Signal(signal))
    }

    /// A step the game defines.
    pub const fn custom(kind: u32, at: Where, param: u32) -> Self {
        Self::of(StepKind::Custom { kind, at, param })
    }

    /// Call for help.
    pub const fn call(channel: u32, kind: u32) -> Self {
        Self::of(StepKind::Call { channel, kind })
    }

    /// Ask for a squad tactic.
    pub const fn squad(tactic: u32, target: Where) -> Self {
        Self::of(StepKind::Squad { tactic, target })
    }

    /// Adds a condition that breaks this step, in the first free slot. At
    /// most [`MAX_FAILS`]; a further one is ignored (a debug assertion
    /// fires).
    pub fn fails_when(mut self, fail: Fail) -> Self {
        add_fail(&mut self.fails, fail);
        self
    }

    /// Skip this step when it cannot be done.
    pub const fn skip_if_impossible(mut self) -> Self {
        self.skip_if_impossible = true;
        self
    }
}

/// A list of steps, once or in a loop, with fail conditions for the whole.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Plan {
    steps: Vec<Step>,
    /// Start again after the last step.
    pub repeat: bool,
    fails: [Option<Fail>; MAX_FAILS],
}

impl Plan {
    fn new(steps: &[Step], repeat: bool) -> Self {
        Self {
            steps: steps.to_vec(),
            repeat,
            fails: [None; MAX_FAILS],
        }
    }

    /// An empty plan with room for `steps` steps (storage of a mission).
    pub(super) fn with_capacity(steps: usize) -> Self {
        Self {
            steps: Vec::with_capacity(steps),
            repeat: false,
            fails: [None; MAX_FAILS],
        }
    }

    /// Makes this plan a copy of `other` without allocating, as long as
    /// `other` has no more steps than this plan's capacity.
    pub(super) fn copy_from(&mut self, other: &Plan) {
        self.steps.clear();
        self.steps.extend_from_slice(&other.steps);
        self.repeat = other.repeat;
        self.fails = other.fails;
    }

    /// Makes this plan run `steps` once (or looping), without fail
    /// conditions, without allocating while `steps` fits its capacity.
    pub(super) fn set_steps(&mut self, steps: &[Step], repeat: bool) {
        self.steps.clear();
        self.steps.extend_from_slice(steps);
        self.repeat = repeat;
        self.fails = [None; MAX_FAILS];
    }

    /// The number of steps this plan holds without allocating.
    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.steps.capacity()
    }

    /// A plan that runs the steps once.
    pub fn once(steps: &[Step]) -> Self {
        Self::new(steps, false)
    }

    /// A plan that runs the steps again and again.
    pub fn looping(steps: &[Step]) -> Self {
        Self::new(steps, true)
    }

    /// Adds a condition that breaks the whole plan, in the first free slot.
    /// At most [`MAX_FAILS`]; a further one is ignored (a debug assertion
    /// fires).
    pub fn fails_when(mut self, fail: Fail) -> Self {
        add_fail(&mut self.fails, fail);
        self
    }

    /// The steps.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The conditions that break the plan; filled from the front.
    pub fn fails(&self) -> &[Option<Fail>; MAX_FAILS] {
        &self.fails
    }
}

/// What the character wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MotiveKind {
    /// Keep a place or a subject safe.
    Guard,
    /// Defeat a subject or the enemies.
    Defeat,
    /// Get to a place.
    Reach,
    /// A motive the game defines.
    Custom(u32),
}

/// The motive of a mission: what the character wants and where or whom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Motive {
    /// What the character wants.
    pub kind: MotiveKind,
    /// The place it is about, if any.
    pub place: Option<Pos>,
    /// The subject it is about, if any.
    pub subject: Option<u32>,
    /// How close counts as there; 0 uses the config's `arrive_radius`.
    pub radius: u32,
}

impl Motive {
    const fn of(kind: MotiveKind, place: Option<Pos>, subject: Option<u32>) -> Self {
        Self {
            kind,
            place,
            subject,
            radius: 0,
        }
    }

    /// Guard a place.
    pub const fn guard_place(place: Pos) -> Self {
        Self::of(MotiveKind::Guard, Some(place), None)
    }

    /// Guard a subject.
    pub const fn guard_subject(subject: u32) -> Self {
        Self::of(MotiveKind::Guard, None, Some(subject))
    }

    /// Defeat a subject.
    pub const fn defeat(subject: u32) -> Self {
        Self::of(MotiveKind::Defeat, None, Some(subject))
    }

    /// Defeat the enemies (no particular subject).
    pub const fn defeat_enemies() -> Self {
        Self::of(MotiveKind::Defeat, None, None)
    }

    /// Reach a place.
    pub const fn reach(place: Pos) -> Self {
        Self::of(MotiveKind::Reach, Some(place), None)
    }

    /// A motive the game defines.
    pub const fn custom(kind: u32) -> Self {
        Self::of(MotiveKind::Custom(kind), None, None)
    }
}

/// What the character does when it loses the subject of a detour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnLost {
    /// Go back to the anchor.
    Return,
    /// Search for a while first.
    Search {
        /// How long to search.
        ticks: u32,
        /// How far around the last known place.
        radius: u32,
    },
    /// Keep going until the detour ends by itself.
    Keep,
}

/// How far and how long a detour may take the character from its plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Leash {
    /// Largest distance from the anchor, in the mission's metric; `None`
    /// for no limit.
    pub distance: Option<u32>,
    /// Longest detour in ticks; `None` for no limit.
    pub ticks: Option<u32>,
    /// What to do when the subject is lost.
    pub on_lost: OnLost,
}

/// How freely a character acts: its leash and whether it may replan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Leeway {
    /// The leash for detours.
    pub leash: Leash,
    /// May take another plan when its plan breaks or ends.
    pub replan: bool,
    /// May fall back to the built-in widening plan (only with `replan`).
    pub widen: bool,
}

impl Leeway {
    /// Distance 3, returns when the subject is lost, never replans.
    pub const STRICT: Self = Self {
        leash: Leash {
            distance: Some(3),
            ticks: None,
            on_lost: OnLost::Return,
        },
        replan: false,
        widen: false,
    };
    /// Distance 12, searches when the subject is lost, replans.
    pub const FLEXIBLE: Self = Self {
        leash: Leash {
            distance: Some(12),
            ticks: None,
            on_lost: OnLost::Search {
                ticks: 60,
                radius: 6,
            },
        },
        replan: true,
        widen: false,
    };
    /// No leash, replans and widens.
    pub const FREE: Self = Self {
        leash: Leash {
            distance: None,
            ticks: None,
            on_lost: OnLost::Keep,
        },
        replan: true,
        widen: true,
    };
}

impl Default for Leeway {
    /// [`Leeway::FLEXIBLE`].
    fn default() -> Self {
        Self::FLEXIBLE
    }
}

/// Settings of a mission.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MissionConfig {
    /// The mission's id (for the seed and for search points).
    pub id: u32,
    /// Most steps a plan may have.
    pub max_steps: usize,
    /// Number of playbook slots.
    pub max_playbook: usize,
    /// The fact kind positions are read from.
    pub subject_kind: u32,
    /// Bits of a fact's `data` that mark an enemy.
    pub enemy_bits: u32,
    /// Bits of a fact's `data` that mark a defeated subject.
    pub defeated_bits: u32,
    /// How close counts as arrived.
    pub arrive_radius: u32,
    /// Ticks after which a subject not seen counts as lost.
    pub lost_after: u32,
    /// Ticks of the [`StepKind::Pause`] step.
    pub pause_ticks: u32,
    /// Distance at which a fleeing character counts as safe.
    pub safe_distance: u32,
    /// The signal given when the motive's place is reached.
    pub reached_signal: u32,
    /// How distances are measured.
    pub metric: Metric,
    /// The pattern of search points. The mission works in cells, so the
    /// distances of the default are in cells (set `min` and `max` for a
    /// bounded map).
    pub search: PointPattern,
    /// Hex layout: positions of the game are then offset coordinates.
    pub hex_layout: Option<HexLayout>,
    /// Extra input columns the game adds to each decider row.
    pub extra_columns: usize,
    /// How the decider picks among playbook plans.
    pub pick: Pick,
}

impl Default for MissionConfig {
    /// Id 0, 16 steps, 8 playbook slots, subject kind 0, enemy bit 1,
    /// defeated bit 2, arrive radius 0, lost after 30, pause 50, safe
    /// distance 12, reached signal 0, Chebyshev, a search pattern in cells
    /// (heading step 0x62, first distance 1, offsets 0, 0x20, -0x20 at
    /// distances 2, 4, 6, unclamped), no hex layout, no extra columns, [`Pick::Best`].
    fn default() -> Self {
        Self {
            id: 0,
            max_steps: 16,
            max_playbook: 8,
            subject_kind: 0,
            enemy_bits: 1,
            defeated_bits: 2,
            arrive_radius: 0,
            lost_after: 30,
            pause_ticks: 50,
            safe_distance: 12,
            reached_signal: 0,
            metric: Metric::Chebyshev,
            search: PointPattern {
                first_distance: 1,
                distances: vec![2, 4, 6],
                ..PointPattern::default()
            },
            hex_layout: None,
            extra_columns: 0,
            pick: Pick::Best,
        }
    }
}

/// A short break from the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Detour {
    /// Go for this subject.
    Engage(u32),
    /// Get away from a place, to a place if given.
    Flee {
        /// What to get away from.
        from: Where,
        /// Where to run to, if the game says.
        to: Option<Pos>,
    },
    /// A detour the game defines.
    Custom {
        /// The game's own kind.
        kind: u32,
        /// Where it happens.
        at: Where,
        /// A free parameter.
        param: u32,
    },
}

/// What the character should do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Nothing.
    None,
    /// Walk to the position.
    GoTo(Pos),
    /// Stand and wait.
    Wait,
    /// Follow a subject, last known at `pos`.
    Follow {
        /// The subject.
        subject: u32,
        /// Where it was last known.
        pos: Pos,
    },
    /// Hold the position.
    Hold(Pos),
    /// Attack a subject, last known at `pos`.
    Attack {
        /// The subject.
        subject: u32,
        /// Where it was last known.
        pos: Pos,
    },
    /// Search at the position.
    Search(Pos),
    /// Wander about.
    Wander,
    /// Run to the position.
    Flee(Pos),
    /// An action of a game-defined step or detour.
    Custom {
        /// The game's own kind.
        kind: u32,
        /// Where, if anywhere.
        pos: Option<Pos>,
        /// A free parameter.
        param: u32,
    },
}

/// Why a plan broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BreakReason {
    /// A fail condition fired.
    Fail(Fail),
    /// A step could not be done.
    Impossible,
    /// The plan ended.
    PlanEnded,
}

/// Why a detour ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DetourEnd {
    /// The subject was defeated.
    Defeated,
    /// The subject is no longer known.
    Forgotten,
    /// The subject was lost.
    Lost,
    /// The leash ran out.
    Leash,
    /// The character is safe.
    Safe,
    /// The character arrived.
    Arrived,
    /// The game ended it.
    Ended,
}

/// Something that happened during a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionEvent {
    /// The step with this index is done.
    StepDone(u16),
    /// The plan started again.
    Looped,
    /// The plan is done.
    PlanDone,
    /// The plan broke.
    Broken(BreakReason),
    /// A new plan was taken; `option` is the decider option.
    Replanned {
        /// The decider option that won.
        option: u32,
    },
    /// The character fell back to holding its position.
    FellBack,
    /// A detour started.
    DetourStarted,
    /// A detour ended.
    DetourEnded(DetourEnd),
    /// The character is back at its anchor and resumes.
    Returned,
    /// A signal was given.
    Signal(u32),
    /// A call for help.
    Call {
        /// The channel.
        channel: u32,
        /// The kind of call.
        kind: u32,
        /// Where the caller is.
        pos: Pos,
        /// The caller's motive at the time of the call.
        motive: Motive,
    },
    /// The mission wants a squad tactic.
    WantsSquad {
        /// The tactic.
        tactic: u32,
        /// The target place, if any.
        target: Option<Pos>,
        /// The target subject, if any.
        subject: Option<u32>,
    },
}

/// What a tick did, for debugging: the record of
/// [`Mission::tick_traced`](super::Mission::tick_traced). `option` is a
/// candidate's decider option.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionTrace {
    /// One column of a candidate's row, after the game's `columns` slot.
    Column {
        /// The candidate's option.
        option: u32,
        /// Index of the column (see [`IN_FEASIBLE`](super::IN_FEASIBLE) and
        /// the other `IN_*`).
        column: usize,
        /// Its value.
        value: i32,
    },
    /// The decider's record of a candidate (its factors and its score);
    /// the candidate index in `event` is always 0.
    Decide {
        /// The candidate's option.
        option: u32,
        /// The decider's record, unchanged.
        event: DecideTrace,
    },
    /// A candidate's score after the game's `score` slot; the pick uses it.
    Rescored {
        /// The candidate's option.
        option: u32,
        /// The decider's score (0 when a consideration reads beyond the
        /// row).
        built_in: u32,
        /// The score after the slot.
        score: u32,
    },
    /// The end of a scoring: the winner and its score. `None` when every
    /// candidate scored 0 (then the fallback applies).
    Picked {
        /// The winning option.
        option: Option<u32>,
        /// Its score.
        score: u32,
    },
    /// A step begins.
    Begin {
        /// Index of the step in the plan.
        index: usize,
        /// The position the step's [`Where`] stands for now (in the game's
        /// coordinates); `None` for a step without one or when it is not
        /// known.
        at: Option<Pos>,
    },
}

/// Most events one tick can give.
pub const MAX_EVENTS: usize = 8;

/// Why a mission could not be made or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionError {
    /// A capacity of 0 was asked for.
    ZeroCapacity,
    /// The plan has no steps.
    EmptyPlan,
    /// The plan has more steps than `max_steps`.
    TooManySteps,
    /// All playbook slots are in use.
    PlaybookFull,
    /// `max_playbook` is above 65535.
    TooManyPlans,
    /// A hex layout needs the hex metric.
    HexNeedsHexMetric,
}

impl std::fmt::Display for MissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ZeroCapacity => "capacity must be at least 1",
            Self::EmptyPlan => "the plan has no steps",
            Self::TooManySteps => "the plan has more steps than the mission allows",
            Self::PlaybookFull => "the playbook is full",
            Self::TooManyPlans => "more playbook slots than a mission allows",
            Self::HexNeedsHexMetric => "a hex layout needs the hex metric",
        })
    }
}

impl std::error::Error for MissionError {}

// --- Snapshot (ADR 0010, ADR 0012) -------------------------------------------------------

use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

const BAD_TAG: SnapshotError = SnapshotError::Invalid("knorke.ai: mission tag");

impl SnapshotValue for Where {
    fn put(&self, w: &mut Writer<'_>) {
        match self {
            Self::Fixed(p) => {
                w.put(&0u8);
                w.put(p);
            }
            Self::Subject(s) => {
                w.put(&1u8);
                w.put(s);
            }
            Self::MotivePlace => w.put(&2u8),
            Self::MotiveSubject => w.put(&3u8),
            Self::Here => w.put(&4u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Fixed(r.get()?),
            1 => Self::Subject(r.get()?),
            2 => Self::MotivePlace,
            3 => Self::MotiveSubject,
            4 => Self::Here,
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for StepKind {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::GoTo(at) => {
                w.put(&0u8);
                w.put(&at);
            }
            Self::Wait(t) => {
                w.put(&1u8);
                w.put(&t);
            }
            Self::Pause => w.put(&2u8),
            Self::Follow(s) => {
                w.put(&3u8);
                w.put(&s);
            }
            Self::WaitFor { subject, radius } => {
                w.put(&4u8);
                w.put(&subject);
                w.put(&radius);
            }
            Self::Search {
                centre,
                radius,
                ticks,
            } => {
                w.put(&5u8);
                w.put(&centre);
                w.put(&radius);
                w.put(&ticks);
            }
            Self::Hold(at) => {
                w.put(&6u8);
                w.put(&at);
            }
            Self::Attack(s) => {
                w.put(&7u8);
                w.put(&s);
            }
            Self::Wander => w.put(&8u8),
            Self::Signal(s) => {
                w.put(&9u8);
                w.put(&s);
            }
            Self::Custom { kind, at, param } => {
                w.put(&10u8);
                w.put(&kind);
                w.put(&at);
                w.put(&param);
            }
            Self::Call { channel, kind } => {
                w.put(&11u8);
                w.put(&channel);
                w.put(&kind);
            }
            Self::Squad { tactic, target } => {
                w.put(&12u8);
                w.put(&tactic);
                w.put(&target);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::GoTo(r.get()?),
            1 => Self::Wait(r.get()?),
            2 => Self::Pause,
            3 => Self::Follow(r.get()?),
            4 => Self::WaitFor {
                subject: r.get()?,
                radius: r.get()?,
            },
            5 => Self::Search {
                centre: r.get()?,
                radius: r.get()?,
                ticks: r.get()?,
            },
            6 => Self::Hold(r.get()?),
            7 => Self::Attack(r.get()?),
            8 => Self::Wander,
            9 => Self::Signal(r.get()?),
            10 => Self::Custom {
                kind: r.get()?,
                at: r.get()?,
                param: r.get()?,
            },
            11 => Self::Call {
                channel: r.get()?,
                kind: r.get()?,
            },
            12 => Self::Squad {
                tactic: r.get()?,
                target: r.get()?,
            },
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for Fail {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::Unknown { subject } => {
                w.put(&0u8);
                w.put(&subject);
            }
            Self::Defeated { subject } => {
                w.put(&1u8);
                w.put(&subject);
            }
            Self::Fact {
                subject,
                kind,
                mask,
                value,
            } => {
                w.put(&2u8);
                w.put(&subject);
                w.put(&kind);
                w.put(&mask);
                w.put(&value);
            }
            Self::Flags { mask, value } => {
                w.put(&3u8);
                w.put(&mask);
                w.put(&value);
            }
            Self::Custom(id) => {
                w.put(&4u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Unknown { subject: r.get()? },
            1 => Self::Defeated { subject: r.get()? },
            2 => Self::Fact {
                subject: r.get()?,
                kind: r.get()?,
                mask: r.get()?,
                value: r.get()?,
            },
            3 => Self::Flags {
                mask: r.get()?,
                value: r.get()?,
            },
            4 => Self::Custom(r.get()?),
            _ => return Err(BAD_TAG),
        })
    }
}

fn put_fails(w: &mut Writer<'_>, fails: &[Option<Fail>; MAX_FAILS]) {
    for f in fails {
        w.put(f);
    }
}

fn get_fails(r: &mut Reader<'_>) -> Result<[Option<Fail>; MAX_FAILS], SnapshotError> {
    let mut fails = [None; MAX_FAILS];
    for f in &mut fails {
        *f = r.get()?;
    }
    Ok(fails)
}

impl SnapshotValue for Step {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.kind);
        put_fails(w, &self.fails);
        w.put(&self.skip_if_impossible);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            kind: r.get()?,
            fails: get_fails(r)?,
            skip_if_impossible: r.get()?,
        })
    }
}

impl Plan {
    /// Restores into this plan, reusing its step buffer; refuses more than `max` steps
    /// before reading them, so a bad length allocates nothing. On error the plan is empty.
    pub(super) fn get_into_bounded(
        &mut self,
        r: &mut Reader<'_>,
        max: usize,
    ) -> Result<(), SnapshotError> {
        let result = (|| {
            let n = r.get_len()?;
            if n > max {
                return Err(SnapshotError::Invalid("knorke.ai: mission plan too long"));
            }
            self.steps.clear();
            for _ in 0..n {
                self.steps.push(r.get()?);
            }
            self.repeat = r.get()?;
            self.fails = get_fails(r)?;
            Ok(())
        })();
        if result.is_err() {
            self.set_steps(&[], false);
        }
        result
    }
}

/// The steps, the loop flag and the plan's fail conditions. Restore reuses the step buffer.
impl SnapshotValue for Plan {
    fn put(&self, w: &mut Writer<'_>) {
        w.put_slice(&self.steps);
        w.put(&self.repeat);
        put_fails(w, &self.fails);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut p = Self::with_capacity(0);
        p.get_into(r)?;
        Ok(p)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let result = (|| {
            r.get_into_vec(&mut self.steps)?;
            self.repeat = r.get()?;
            self.fails = get_fails(r)?;
            Ok(())
        })();
        if result.is_err() {
            self.set_steps(&[], false);
        }
        result
    }
}

impl SnapshotValue for MotiveKind {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::Guard => w.put(&0u8),
            Self::Defeat => w.put(&1u8),
            Self::Reach => w.put(&2u8),
            Self::Custom(k) => {
                w.put(&3u8);
                w.put(&k);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Guard,
            1 => Self::Defeat,
            2 => Self::Reach,
            3 => Self::Custom(r.get()?),
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for Motive {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.kind);
        w.put(&self.place);
        w.put(&self.subject);
        w.put(&self.radius);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            kind: r.get()?,
            place: r.get()?,
            subject: r.get()?,
            radius: r.get()?,
        })
    }
}

impl SnapshotValue for OnLost {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::Return => w.put(&0u8),
            Self::Search { ticks, radius } => {
                w.put(&1u8);
                w.put(&ticks);
                w.put(&radius);
            }
            Self::Keep => w.put(&2u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Return,
            1 => Self::Search {
                ticks: r.get()?,
                radius: r.get()?,
            },
            2 => Self::Keep,
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for Leash {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.distance);
        w.put(&self.ticks);
        w.put(&self.on_lost);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            distance: r.get()?,
            ticks: r.get()?,
            on_lost: r.get()?,
        })
    }
}

impl SnapshotValue for Leeway {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.leash);
        w.put(&self.replan);
        w.put(&self.widen);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            leash: r.get()?,
            replan: r.get()?,
            widen: r.get()?,
        })
    }
}

impl SnapshotValue for Detour {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::Engage(s) => {
                w.put(&0u8);
                w.put(&s);
            }
            Self::Flee { from, to } => {
                w.put(&1u8);
                w.put(&from);
                w.put(&to);
            }
            Self::Custom { kind, at, param } => {
                w.put(&2u8);
                w.put(&kind);
                w.put(&at);
                w.put(&param);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Engage(r.get()?),
            1 => Self::Flee {
                from: r.get()?,
                to: r.get()?,
            },
            2 => Self::Custom {
                kind: r.get()?,
                at: r.get()?,
                param: r.get()?,
            },
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for BreakReason {
    fn put(&self, w: &mut Writer<'_>) {
        match self {
            Self::Fail(f) => {
                w.put(&0u8);
                w.put(f);
            }
            Self::Impossible => w.put(&1u8),
            Self::PlanEnded => w.put(&2u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Fail(r.get()?),
            1 => Self::Impossible,
            2 => Self::PlanEnded,
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for DetourEnd {
    fn put(&self, w: &mut Writer<'_>) {
        let tag: u8 = match self {
            Self::Defeated => 0,
            Self::Forgotten => 1,
            Self::Lost => 2,
            Self::Leash => 3,
            Self::Safe => 4,
            Self::Arrived => 5,
            Self::Ended => 6,
        };
        w.put(&tag);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::Defeated,
            1 => Self::Forgotten,
            2 => Self::Lost,
            3 => Self::Leash,
            4 => Self::Safe,
            5 => Self::Arrived,
            6 => Self::Ended,
            _ => return Err(BAD_TAG),
        })
    }
}

impl SnapshotValue for MissionEvent {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Self::StepDone(i) => {
                w.put(&0u8);
                w.put(&i);
            }
            Self::Looped => w.put(&1u8),
            Self::PlanDone => w.put(&2u8),
            Self::Broken(b) => {
                w.put(&3u8);
                w.put(&b);
            }
            Self::Replanned { option } => {
                w.put(&4u8);
                w.put(&option);
            }
            Self::FellBack => w.put(&5u8),
            Self::DetourStarted => w.put(&6u8),
            Self::DetourEnded(e) => {
                w.put(&7u8);
                w.put(&e);
            }
            Self::Returned => w.put(&8u8),
            Self::Signal(s) => {
                w.put(&9u8);
                w.put(&s);
            }
            Self::Call {
                channel,
                kind,
                pos,
                motive,
            } => {
                w.put(&10u8);
                w.put(&channel);
                w.put(&kind);
                w.put(&pos);
                w.put(&motive);
            }
            Self::WantsSquad {
                tactic,
                target,
                subject,
            } => {
                w.put(&11u8);
                w.put(&tactic);
                w.put(&target);
                w.put(&subject);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Self::StepDone(r.get()?),
            1 => Self::Looped,
            2 => Self::PlanDone,
            3 => Self::Broken(r.get()?),
            4 => Self::Replanned { option: r.get()? },
            5 => Self::FellBack,
            6 => Self::DetourStarted,
            7 => Self::DetourEnded(r.get()?),
            8 => Self::Returned,
            9 => Self::Signal(r.get()?),
            10 => Self::Call {
                channel: r.get()?,
                kind: r.get()?,
                pos: r.get()?,
                motive: r.get()?,
            },
            11 => Self::WantsSquad {
                tactic: r.get()?,
                target: r.get()?,
                subject: r.get()?,
            },
            _ => return Err(BAD_TAG),
        })
    }
}
