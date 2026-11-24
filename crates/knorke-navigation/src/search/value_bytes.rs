//! Exact snapshot bytes of the plain search values (see `knorke_types::assert_bytes!`): a
//! swapped tag or field order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    knorke_types::assert_bytes!(
        SearchStatus::Idle => "00",
        SearchStatus::Running => "01",
        SearchStatus::Found => "02",
        SearchStatus::Exhausted => "03",
        Record { node: 0x0102_0304u32, cost: 5, estimate: 6, parent: 7, closed: true } => "0403020105000000060000000700000001",
        Open { key: 0x0102_0304_0506_0708, index: 9 } => "080706050403020109000000",
    );
}
