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

use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

impl SnapshotValue for Cell {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.x);
        w.put(&self.y);
        w.put(&self.level);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Cell::new(r.get()?, r.get()?, r.get()?))
    }
}

impl SnapshotValue for HexLayout {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => HexLayout::OddR,
            1 => HexLayout::EvenR,
            2 => HexLayout::OddQ,
            3 => HexLayout::EvenQ,
            _ => {
                return Err(SnapshotError::Invalid(
                    "knorke.grid: hex layout out of range",
                ));
            }
        })
    }
}

impl SnapshotValue for Sine {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Sine::Fine,
            1 => Sine::Coarse,
            _ => {
                return Err(SnapshotError::Invalid(
                    "knorke.grid: sine table out of range",
                ));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(Reader::new(&[9]).get::<HexLayout>().is_err());
        assert!(Reader::new(&[2]).get::<Sine>().is_err());
    }
}
