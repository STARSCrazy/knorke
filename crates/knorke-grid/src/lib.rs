//! Grid and hex space of the Knorke Engine (ADR 0012).
//!
//! The places every grid-based module agrees on: [`Pos`] (three integers), [`Cell`]
//! (column, row, level), [`HexLayout`] with axial coordinates, and the integer heading
//! tables [`sine`]. Navigation, perception and AI all address the world through these
//! types; none of them needs the others to do so, which is why the types live in their own
//! crate and not in `knorke-navigation` (and not in `knorke-types`, which holds no
//! coordinate types, ADR 0001 §6).
//!
//! Origin: `sagax-core` of Sagax (revision fab00c0), without its trace sinks (now
//! `knorke_observe::Sink`) and its seed mix (now `knorke_types::Seed::derive`).

#![no_std]

#[cfg(test)]
extern crate alloc;

mod cell;
mod heading;
mod hex;

pub use cell::{Cell, Pos};
pub use heading::{Sine, sine};
pub use hex::{HexLayout, axial_step};

use knorke_types::SnapshotError;

// Field order and tags pinned by `tests::snapshot_values_round_trip`.
knorke_types::by_fields! {
    Cell { x, y, level };
}

knorke_types::by_tag! {
    HexLayout, SnapshotError::Invalid("knorke.grid: hex layout out of range") => {
        0 => OddR,
        1 => EvenR,
        2 => OddQ,
        3 => EvenQ,
    };
    Sine, SnapshotError::Invalid("knorke.grid: sine table out of range") => {
        0 => Fine,
        1 => Coarse,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_types::{Reader, Writer};

    #[test]
    fn snapshot_values_round_trip() {
        let mut bytes = alloc::vec::Vec::new();
        let mut w = Writer::new(&mut bytes);
        w.put(&Cell::new(-3, 7, 2));
        w.put(&HexLayout::EvenQ);
        w.put(&Sine::Coarse);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.get::<Cell>(), Ok(Cell::new(-3, 7, 2)));
        assert_eq!(r.get::<HexLayout>(), Ok(HexLayout::EvenQ));
        assert_eq!(r.get::<Sine>(), Ok(Sine::Coarse));
        // Pinned bytes: cell fields in order (little-endian), then the enum tags.
        assert_eq!(
            bytes,
            [253, 255, 255, 255, 7, 0, 0, 0, 2, 0, 0, 0, 3, 1][..]
        );
        let mut tags = alloc::vec::Vec::new();
        let mut w = Writer::new(&mut tags);
        for l in [
            HexLayout::OddR,
            HexLayout::EvenR,
            HexLayout::OddQ,
            HexLayout::EvenQ,
        ] {
            w.put(&l);
        }
        for s in [Sine::Fine, Sine::Coarse] {
            w.put(&s);
        }
        assert_eq!(tags, [0, 1, 2, 3, 0, 1]);
        assert!(Reader::new(&[9]).get::<HexLayout>().is_err());
        assert!(Reader::new(&[2]).get::<Sine>().is_err());
    }
}
