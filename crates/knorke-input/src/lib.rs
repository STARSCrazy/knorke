//! Device-neutral input (ADR 0022).
//!
//! Three stages, each with its own owner: a window library translates device events into
//! [`InputEvent`]; [`InputState`] collects what is held and what changed since the last
//! tick; [`Bindings`] maps that to game-defined actions. Turning actions into simulation
//! commands is game code. No SDL and no engine module here, so a foreign project can use the
//! mapping with its own window library, and bindings are testable headless.
//!
//! Sampling happens at the tick boundary: a press and release between two ticks still yields
//! one action, a held key does not repeat per frame.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use knorke_types::BitSet;

/// Physical key position as a USB HID usage id (the numbering SDL scancodes use), so the
/// binding survives keyboard layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scancode(pub u16);

#[allow(missing_docs)]
impl Scancode {
    pub const A: Self = Self(4);
    pub const D: Self = Self(7);
    pub const S: Self = Self(22);
    pub const W: Self = Self(26);
    pub const RETURN: Self = Self(40);
    pub const ESCAPE: Self = Self(41);
    pub const SPACE: Self = Self(44);
    pub const RIGHT: Self = Self(79);
    pub const LEFT: Self = Self(80);
    pub const DOWN: Self = Self(81);
    pub const UP: Self = Self(82);
    /// Scancodes at or above this are ignored (SDL's `SDL_SCANCODE_COUNT`).
    pub const COUNT: u16 = 512;
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// Gamepad buttons by position, not label (South = A on Xbox, Cross on PlayStation).
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// Gamepad axes; sticks in `-1..=1`, triggers in `0..=1`.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

const MOUSE_BUTTONS: usize = 5;
const PAD_BUTTONS: usize = 15;
const PAD_AXES: usize = 6;
const BUTTON_SLOTS: usize = Scancode::COUNT as usize + MOUSE_BUTTONS + PAD_BUTTONS;

/// Anything that is either down or up. One gamepad for now; more pads need a pad index.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Button {
    Key(Scancode),
    Mouse(MouseButton),
    Pad(PadButton),
}

impl Button {
    /// Dense slot in the state bit sets; `None` for scancodes out of range.
    fn slot(self) -> Option<usize> {
        match self {
            Button::Key(Scancode(c)) => (c < Scancode::COUNT).then_some(c as usize),
            Button::Mouse(b) => Some(Scancode::COUNT as usize + b as usize),
            Button::Pad(b) => Some(Scancode::COUNT as usize + MOUSE_BUTTONS + b as usize),
        }
    }
}

/// One device event, without meaning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// A button went down (`down = true`) or up. Key repeats arrive as further downs and are
    /// ignored by [`InputState`].
    Button {
        /// Which button.
        button: Button,
        /// New state.
        down: bool,
    },
    /// Mouse position in window pixels.
    MouseMotion {
        /// Horizontal position.
        x: f32,
        /// Vertical position.
        y: f32,
    },
    /// New gamepad axis value.
    Axis {
        /// Which axis.
        axis: PadAxis,
        /// New value.
        value: f32,
    },
}

/// What is held now and which edges happened since the last [`InputState::end_tick`].
///
/// Allocates once in [`InputState::new`]; applying events and ending ticks does not.
#[derive(Clone, Debug)]
pub struct InputState {
    held: BitSet,
    pressed: BitSet,
    released: BitSet,
    mouse: (f32, f32),
    axes: [f32; PAD_AXES],
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    /// Nothing held, mouse at the origin, axes at rest.
    pub fn new() -> Self {
        Self {
            held: BitSet::with_capacity(BUTTON_SLOTS),
            pressed: BitSet::with_capacity(BUTTON_SLOTS),
            released: BitSet::with_capacity(BUTTON_SLOTS),
            mouse: (0.0, 0.0),
            axes: [0.0; PAD_AXES],
        }
    }

    /// Folds one event into the state.
    pub fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Button { button, down } => {
                let Some(i) = button.slot() else { return };
                // Edges only on change: key repeat neither re-presses nor double-releases.
                if down && self.held.insert(i) {
                    self.pressed.insert(i);
                } else if !down && self.held.remove(i) {
                    self.released.insert(i);
                }
            }
            InputEvent::MouseMotion { x, y } => self.mouse = (x, y),
            InputEvent::Axis { axis, value } => self.axes[axis as usize] = value,
        }
    }

    /// Clears the edges. Call only after a tick consumed them, so a tap in a frame without a
    /// due tick is not lost.
    pub fn end_tick(&mut self) {
        self.pressed.clear();
        self.released.clear();
    }

    /// Down right now.
    pub fn held(&self, button: Button) -> bool {
        button.slot().is_some_and(|i| self.held.contains(i))
    }

    /// Went down since the last tick (also if it is already up again).
    pub fn pressed(&self, button: Button) -> bool {
        button.slot().is_some_and(|i| self.pressed.contains(i))
    }

    /// Went up since the last tick.
    pub fn released(&self, button: Button) -> bool {
        button.slot().is_some_and(|i| self.released.contains(i))
    }

    /// Last mouse position in window pixels.
    pub fn mouse(&self) -> (f32, f32) {
        self.mouse
    }

    /// Last value of a gamepad axis.
    pub fn axis(&self, axis: PadAxis) -> f32 {
        self.axes[axis as usize]
    }
}

/// When a binding fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Once per press.
    Pressed,
    /// Once per release.
    Released,
    /// Every tick while held; a tap shorter than a tick counts once.
    Held,
}

/// Table input → action `A`, defined by the game. Actions come out in table order, so the
/// resulting command order is fixed.
#[derive(Clone, Debug)]
pub struct Bindings<A> {
    table: Vec<(Button, Trigger, A)>,
}

impl<A> Default for Bindings<A> {
    fn default() -> Self {
        Self { table: Vec::new() }
    }
}

impl<A: Copy> Bindings<A> {
    /// Empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a row; one button may drive several actions.
    pub fn bind(&mut self, button: Button, trigger: Trigger, action: A) -> &mut Self {
        self.table.push((button, trigger, action));
        self
    }

    /// The actions due this tick, in table order.
    pub fn actions<'a>(&'a self, input: &'a InputState) -> impl Iterator<Item = A> + 'a {
        self.table.iter().filter_map(move |&(b, t, a)| {
            let fires = match t {
                Trigger::Pressed => input.pressed(b),
                Trigger::Released => input.released(b),
                Trigger::Held => input.held(b) || input.pressed(b),
            };
            fires.then_some(a)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Act {
        Up,
        Fire,
        Run,
    }

    const UP: Button = Button::Key(Scancode::UP);
    const FIRE: Button = Button::Pad(PadButton::South);

    fn key(button: Button, down: bool) -> InputEvent {
        InputEvent::Button { button, down }
    }

    fn bindings() -> Bindings<Act> {
        let mut b = Bindings::new();
        b.bind(FIRE, Trigger::Pressed, Act::Fire)
            .bind(UP, Trigger::Pressed, Act::Up)
            .bind(Button::Mouse(MouseButton::Left), Trigger::Held, Act::Run);
        b
    }

    fn tick(b: &Bindings<Act>, s: &mut InputState) -> Vec<Act> {
        let out = b.actions(s).collect();
        s.end_tick();
        out
    }

    #[test]
    fn tap_between_ticks_gives_exactly_one_action() {
        let (b, mut s) = (bindings(), InputState::new());
        s.apply(key(UP, true));
        s.apply(key(UP, false));
        assert_eq!(tick(&b, &mut s), [Act::Up]);
        assert_eq!(tick(&b, &mut s), []);
    }

    #[test]
    fn held_key_and_repeats_do_not_repeat_pressed() {
        let (b, mut s) = (bindings(), InputState::new());
        s.apply(key(UP, true));
        assert_eq!(tick(&b, &mut s), [Act::Up]);
        for _ in 0..3 {
            s.apply(key(UP, true)); // OS key repeat
            assert_eq!(tick(&b, &mut s), []);
        }
        s.apply(key(UP, false));
        assert!(s.released(UP) && !s.held(UP));
    }

    #[test]
    fn edges_survive_frames_without_a_tick() {
        let (b, mut s) = (bindings(), InputState::new());
        s.apply(key(UP, true));
        // Two frames pass without a due tick: no end_tick.
        s.apply(key(UP, false));
        assert_eq!(tick(&b, &mut s), [Act::Up]);
    }

    #[test]
    fn held_trigger_fires_every_tick_and_counts_a_short_tap() {
        let (b, mut s) = (bindings(), InputState::new());
        let lmb = Button::Mouse(MouseButton::Left);
        s.apply(key(lmb, true));
        assert_eq!(tick(&b, &mut s), [Act::Run]);
        assert_eq!(tick(&b, &mut s), [Act::Run]);
        s.apply(key(lmb, false));
        assert_eq!(tick(&b, &mut s), []);
        s.apply(key(lmb, true));
        s.apply(key(lmb, false));
        assert_eq!(tick(&b, &mut s), [Act::Run]);
    }

    #[test]
    fn actions_come_in_table_order_not_event_order() {
        let (b, mut s) = (bindings(), InputState::new());
        s.apply(key(UP, true));
        s.apply(key(FIRE, true));
        assert_eq!(tick(&b, &mut s), [Act::Fire, Act::Up]);
    }

    #[test]
    fn analog_values_and_out_of_range_scancodes() {
        let mut s = InputState::new();
        s.apply(InputEvent::MouseMotion { x: 3.0, y: 4.5 });
        s.apply(InputEvent::Axis {
            axis: PadAxis::LeftY,
            value: -0.5,
        });
        let bogus = Button::Key(Scancode(Scancode::COUNT));
        s.apply(key(bogus, true));
        assert_eq!(s.mouse(), (3.0, 4.5));
        assert_eq!(s.axis(PadAxis::LeftY), -0.5);
        assert!(!s.held(bogus) && !s.pressed(bogus));
    }
}
