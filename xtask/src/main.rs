//! Repository automation. Run `cargo xtask help`.

mod deps;
mod shaders;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("deps") => deps::run(),
        Some("standalone") => standalone(),
        Some("shaders") => shaders::run(&args[1..]),
        _ => {
            eprintln!(
                "usage: cargo xtask <task>\n\n\
                 tasks:\n  \
                 deps        check forbidden dependency edges (ADR 0001)\n  \
                 standalone  build every library crate on its own with default features off
                   shaders     build shader packs (.kpak) from compiled blobs + reflection JSON"
            );
            ExitCode::from(2)
        }
    }
}

/// Builds each workspace library crate in isolation with `--no-default-features`, so a crate
/// that only compiles thanks to features unified in from its neighbours is caught.
fn standalone() -> ExitCode {
    let meta = match deps::metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = Vec::new();
    for name in deps::workspace_libraries(&meta) {
        eprintln!("== {name}");
        let status = std::process::Command::new(env!("CARGO"))
            .args([
                "check",
                "--package",
                &name,
                "--no-default-features",
                "--lib",
            ])
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            failed.push(name);
        }
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("standalone build failed for: {}", failed.join(", "));
        ExitCode::FAILURE
    }
}
