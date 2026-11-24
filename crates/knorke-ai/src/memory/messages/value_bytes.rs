//! Exact snapshot bytes of `Posted` (see `knorke_types::assert_bytes!`): a swapped field
//! order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    let posted = Posted {
        msg: Message {
            kind: 1,
            subject: 2,
            pos: [3, 4, 5],
            seen_at: 6,
            strength: 7,
            data: 8,
            sender: 9,
            keep_if_sender_gone: 1,
        },
        at: 10,
        delay: 11,
    };
    knorke_types::assert_bytes!(
        posted => "010000000200000003000000040000000500000006000000070000000800000009000000010a0000000b000000"
    );
}
