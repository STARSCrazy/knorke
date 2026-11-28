//! Contract: after `InputState::new` and building the bindings, applying events, reading
//! actions and ending ticks allocate nothing – this runs every frame (ADR 0022, ADR 0006).
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_input::{
    Bindings, Button, InputEvent, InputState, MouseButton, PadAxis, PadButton, Scancode, Trigger,
};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn input_steady_state_allocates_nothing() {
    let buttons = [
        Button::Key(Scancode::UP),
        Button::Key(Scancode::SPACE),
        Button::Mouse(MouseButton::Left),
        Button::Pad(PadButton::South),
    ];
    let mut bindings = Bindings::new();
    for (i, &b) in buttons.iter().enumerate() {
        bindings.bind(b, Trigger::Pressed, i);
        bindings.bind(b, Trigger::Held, i + 10);
    }
    let mut state = InputState::new();
    let mut sum = 0;

    let before = allocations();
    for frame in 0..1000usize {
        let button = buttons[frame % buttons.len()];
        state.apply(InputEvent::Button {
            button,
            down: frame % 3 != 0,
        });
        state.apply(InputEvent::MouseMotion {
            x: frame as f32,
            y: 1.0,
        });
        state.apply(InputEvent::Axis {
            axis: PadAxis::LeftX,
            value: 0.5,
        });
        sum += bindings.actions(&state).sum::<usize>();
        if frame % 2 == 0 {
            state.end_tick();
        }
    }
    assert_eq!(allocations() - before, 0);
    assert!(sum > 0);
}
