//! Exact snapshot bytes of the plain planner values (see `knorke_types::assert_bytes!`): a
//! swapped tag or field order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    knorke_types::assert_bytes!(
        HexHeuristic { layout: HexLayout::OddR, step_cost: 3 } => "0003000000",
        HexHeuristic { layout: HexLayout::EvenQ, step_cost: 3 } => "0303000000",
        DStarStatus::Idle => "00",
        DStarStatus::Running => "01",
        DStarStatus::Found => "02",
        DStarStatus::Unreachable => "03",
        Record { node: 0x0102_0304u32, g: 5, rhs: 6, queued: None } => "040302010500000000000000060000000000000000",
        Record { node: 0x0102_0304u32, g: 5, rhs: 6, queued: Some((7, 8)) } => "04030201050000000000000006000000000000000107000000000000000800000000000000",
        Entry { key: (1, 2), index: 3 } => "0100000000000000020000000000000003000000",
    );
}
