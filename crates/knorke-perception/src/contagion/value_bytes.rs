//! Exact snapshot bytes of the plain contagion values (see `knorke_types::assert_bytes!`):
//! a swapped tag or field order fails here even though it round-trips.

use super::*;

#[test]
fn value_bytes_are_pinned() {
    let mut config = ContagionConfig::new(Grid::Hex);
    config.count = 2;
    config.deeds[0] = DeedRule {
        bits: 0x0102_0304,
        window: 5,
    };
    config.deeds[1] = DeedRule { bits: 6, window: 7 };
    let slot = Slot {
        used: true,
        ally: 1,
        last_seen: 2,
        point: [3, -4, 5],
        bits: 6,
        live: 7,
        onset: [8, 9, 10, 11, 12, 13, 14, 15],
    };
    knorke_types::assert_bytes!(
        Grid::Square => "00",
        Grid::Hex => "01",
        DeedRule { bits: 0x0102_0304, window: 5 } => "0403020105000000",
        config => "010403020105000000060000000700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002",
        slot => "01010000000200000003000000fcffffff05000000060000000708000000090000000a0000000b0000000c0000000d0000000e0000000f000000",
    );
}
