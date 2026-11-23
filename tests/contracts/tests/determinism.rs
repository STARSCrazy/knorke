//! Contract: the fingerprint simulation gives the frozen checksum on every platform and
//! with every executor (ADR 0011). CI runs this on Windows, Linux, macOS (ARM) and WASM.
//!
//! If this fails on one platform only, simulation code uses a platform-dependent operation
//! (see clippy.toml). If it fails everywhere after an intended change, update `GOLDEN` and
//! say so in the commit message – every recorded replay becomes invalid.

use knorke_contract_tests::fingerprint::run;
use knorke_types::{Checksum, Serial};

const TICKS: u64 = 600;
const GOLDEN: Checksum = Checksum(0x8E13_81FE_2691_7217);

#[test]
fn fingerprint_is_frozen() {
    let got = run(Serial, TICKS);
    println!("fingerprint = {:#018X}", got.0);
    assert_eq!(
        got, GOLDEN,
        "determinism fingerprint changed: {:#018X}",
        got.0
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn fingerprint_is_executor_independent() {
    use knorke_exec::{Rayon, ScopedThreads};
    let reference = run(Serial, TICKS);
    assert_eq!(run(Rayon::with_threads(3).unwrap(), TICKS), reference);
    assert_eq!(run(ScopedThreads::available(), TICKS), reference);
}
