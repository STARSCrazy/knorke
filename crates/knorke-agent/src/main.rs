//! `knorke`: the agent plane as a command line. One JSON object on stdout per call.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (code, answer) = knorke_agent::run(&args);
    println!("{answer:#}");
    ExitCode::from(code as u8)
}
