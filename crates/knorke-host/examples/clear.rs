//! Opens a window that shows a clear colour and prints every input event; Escape or closing
//! the window ends it. An optional argument ends it after that many seconds (for smoke runs).
//!
//! `cargo run -p knorke-host --features sdl-build-static --example clear [seconds]`

use knorke_gpu::{ColorTarget, Load};
use knorke_host::{Host, HostConfig};
use knorke_input::{Button, InputEvent, Scancode};

fn main() -> Result<(), String> {
    let limit_nanos = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .map(|s| (s * 1e9) as u64);
    let host = Host::new(&HostConfig {
        title: "knorke clear".into(),
        ..HostConfig::default()
    })
    .map_err(|d| format!("{}: {}", d.code, d.message))?;
    let mut total = 0u64;
    loop {
        let mut frame = host.begin_frame().map_err(|d| d.message)?;
        let mut quit = frame.quit();
        for ev in frame.input() {
            if !matches!(ev, InputEvent::MouseMotion { .. }) {
                println!("{ev:?}");
            }
            quit |= *ev
                == InputEvent::Button {
                    button: Button::Key(Scancode::ESCAPE),
                    down: true,
                };
        }
        total += frame.elapsed_nanos();
        if let Some(target) = frame.target() {
            // Linear light; the sRGB swapchain encodes it once on store. A dark teal reads
            // as (0.0, 0.5, 0.55) in sRGB, so a wrong double conversion shows as too bright.
            let pass = frame
                .commands()
                .render_pass(&[ColorTarget::new(target, Load::Clear([0.0, 0.2, 0.25, 1.0]))])
                .map_err(|d| d.message)?;
            drop(pass);
        }
        host.end_frame(frame).map_err(|d| d.message)?;
        if quit || limit_nanos.is_some_and(|l| total >= l) {
            return Ok(());
        }
    }
}
