//! Exact snapshot bytes of `Alertness` (see `knorke_types::assert_bytes!`): a swapped field
//! order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    let alertness = Alertness {
        value: 0x0102_0304,
        last: 5,
        last_stimulus: 6,
        level: 7,
    };
    knorke_types::assert_bytes!(alertness => "04030201050000000600000007");
}
