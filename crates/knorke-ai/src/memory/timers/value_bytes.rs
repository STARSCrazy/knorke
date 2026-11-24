//! Exact snapshot bytes of `Timer` (see `knorke_types::assert_bytes!`): a swapped field
//! order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    let timer = Timer {
        id: 1,
        since: 2,
        duration: 3,
    };
    knorke_types::assert_bytes!(timer => "010000000200000003000000");
}
