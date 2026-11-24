//! Snapshot support for the decision types of the crate root (ADR 0010, ADR 0012).
//!
//! A [`Decider`] is part of the state of a [`Mission`](crate::mission::Mission) and a
//! [`Squad`](crate::group::Squad): its options can be added at run time, and its current
//! choice, clock and memory steer the next decision.

use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer, by_fields, by_tag};

use crate::{
    Apply, Candidate, Combine, Consideration, Curve, Decider, Factor, MEMORY_LIMIT, OptionDef, Pick,
};

impl SnapshotValue for Curve {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Curve::from_u32(u32::from(r.get::<u8>()?)).ok_or(SnapshotError::Invalid("knorke.ai: curve"))
    }
}

impl SnapshotValue for Combine {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Combine::from_u32(u32::from(r.get::<u8>()?))
            .ok_or(SnapshotError::Invalid("knorke.ai: combine"))
    }
}

impl SnapshotValue for Apply {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Apply::from_u32(u32::from(r.get::<u8>()?)).ok_or(SnapshotError::Invalid("knorke.ai: apply"))
    }
}

// Plain values: tags and field order are pinned by `tests::value_bytes_are_pinned`.
by_fields! {
    Consideration { input, curve, min, max };
    Candidate { option, target };
    Factor { consideration, pull, apply };
}

by_tag! {
    Pick, SnapshotError::Invalid("knorke.ai: pick") => {
        0 => Best,
        1 => Weighted { top },
        2 => Uniform { top },
    };
}

impl SnapshotValue for OptionDef {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.weight);
        w.put(&self.rank);
        w.put(&self.cooldown);
        w.put(&self.combine);
        w.put(&self.factors);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut o = OptionDef {
            weight: 0,
            rank: 0,
            cooldown: 0,
            combine: None,
            factors: Vec::new(),
        };
        o.get_into(r)?;
        Ok(o)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.weight = r.get()?;
        self.rank = r.get()?;
        self.cooldown = r.get()?;
        self.combine = r.get()?;
        self.factors.get_into(r)
    }
}

/// The options with their considerations, the current choice with its bonus, the clock,
/// the window and the memory of past choices. Restore reuses the option and factor buffers
/// and rejects a memory longer than [`MEMORY_LIMIT`]. The scratch buffers are not saved.
impl SnapshotValue for Decider {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.options);
        w.put(&self.combine);
        w.put(&self.current);
        w.put(&self.now);
        w.put(&self.window);
        w.put_len(self.memory.len());
        for m in &self.memory {
            w.put(m);
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut d = Decider::new();
        d.get_into(r)?;
        Ok(d)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let result = (|| {
            self.options.get_into(r)?;
            self.combine = r.get()?;
            self.current = r.get()?;
            self.now = r.get()?;
            self.window = r.get()?;
            let len = r.get_len()?;
            if len > MEMORY_LIMIT {
                return Err(SnapshotError::Invalid("knorke.ai: decider memory"));
            }
            self.memory.clear();
            for _ in 0..len {
                self.memory.push_back(r.get()?);
            }
            Ok(())
        })();
        if result.is_err() {
            self.options.clear();
            self.current = None;
            self.memory.clear();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{INPUT_COUNT_OPTION, INPUT_SINCE_OPTION, ONE};

    fn save<T: SnapshotValue>(v: &T) -> Vec<u8> {
        let mut b = Vec::new();
        Writer::new(&mut b).put(v);
        b
    }

    fn decider() -> Decider {
        let mut d = Decider::new();
        let a = d.add_option(ONE);
        let b = d.add_option(ONE / 2);
        assert!(d.add_consideration(a, Consideration::new(0, Curve::Linear, 0, 100)));
        assert!(d.add_weighted(
            b,
            Consideration::new(INPUT_SINCE_OPTION, Curve::SCurve, 0, 50),
            ONE / 3,
            Apply::Multiply,
        ));
        assert!(d.add_consideration(
            b,
            Consideration::new(INPUT_COUNT_OPTION, Curve::Inverse, 0, 4)
        ));
        assert!(d.set_rank(b, 2));
        assert!(d.set_cooldown(a, 5));
        assert!(d.set_option_combine(b, Some(Combine::Minimum)));
        d.set_combine(Combine::GeometricMean);
        d.set_window(30);
        d
    }

    #[test]
    fn decider_round_trips_and_decides_alike() {
        let mut d = decider();
        let candidates = [
            Candidate::new(0, 1),
            Candidate::new(1, 1),
            Candidate::new(1, 2),
        ];
        let inputs = [40, 0, 70, 0, 20, 0];
        let mut scores = Vec::new();
        for now in 0..12 {
            d.set_now(now);
            if let Some(c) = d
                .decide(
                    &candidates,
                    &inputs,
                    2,
                    Pick::Weighted { top: 2 },
                    u64::from(now),
                    &mut scores,
                )
                .unwrap()
            {
                d.record(now, candidates[c.candidate]);
                d.set_current(Some(candidates[c.candidate]), ONE / 10);
            }
        }
        let bytes = save(&d);
        // Restore into a decider with other options: they are replaced.
        let mut back = Decider::new();
        back.add_option(1);
        back.get_into(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(save(&back), bytes);
        let mut other = Vec::new();
        for now in 12..40 {
            d.set_now(now);
            back.set_now(now);
            let seed = u64::from(now) * 7;
            let x = d
                .decide(
                    &candidates,
                    &inputs,
                    2,
                    Pick::Weighted { top: 2 },
                    seed,
                    &mut scores,
                )
                .unwrap();
            let y = back
                .decide(
                    &candidates,
                    &inputs,
                    2,
                    Pick::Weighted { top: 2 },
                    seed,
                    &mut other,
                )
                .unwrap();
            assert_eq!(x, y, "tick {now}");
            assert_eq!(scores, other);
            if let Some(c) = x {
                d.record(now, candidates[c.candidate]);
                back.record(now, candidates[c.candidate]);
            }
        }
        assert_eq!(save(&back), save(&d));
        for p in [
            Pick::Best,
            Pick::Weighted { top: 3 },
            Pick::Uniform { top: 0 },
        ] {
            assert_eq!(Reader::new(&save(&p)).get::<Pick>().unwrap(), p);
        }
    }

    #[test]
    fn decider_rejects_bad_data() {
        let d = decider();
        let good = save(&d);
        assert!(Reader::new(&good).get::<Decider>().is_ok());
        assert!(
            Reader::new(&good[..good.len() - 1])
                .get::<Decider>()
                .is_err()
        );
        // The memory length is the last u32 (empty memory): beyond the limit.
        let mut long = good.clone();
        let at = long.len() - 4;
        long[at..].copy_from_slice(&(MEMORY_LIMIT as u32 + 1).to_le_bytes());
        let mut back = decider();
        assert_eq!(
            back.get_into(&mut Reader::new(&long)),
            Err(SnapshotError::Invalid("knorke.ai: decider memory"))
        );
        assert_eq!(
            back.option_count(),
            0,
            "a failed restore leaves no half state"
        );
        // A curve tag out of range (first factor's curve: options len 4, weight, rank,
        // cooldown 12, combine 1 (None), factors len 4, input 4 → offset 25).
        let mut bad = good.clone();
        bad[25] = 99;
        assert!(Reader::new(&bad).get::<Decider>().is_err());
        assert!(Reader::new(&[7]).get::<Combine>().is_err());
        assert!(Reader::new(&[2]).get::<Apply>().is_err());
        assert!(Reader::new(&[3]).get::<Pick>().is_err());
    }

    /// Exact bytes of the plain decision values (see `pinned_bytes`).
    #[test]
    fn value_bytes_are_pinned() {
        let c = Consideration::new(3, Curve::Linear, -1, 100);
        assert_bytes!(
            c => "0300000000ffffffff64000000",
            Candidate { option: 1, target: 2 } => "0100000002000000",
            Pick::Best => "00",
            Pick::Weighted { top: 3 } => "0103000000",
            Pick::Uniform { top: 4 } => "0204000000",
            Factor { consideration: c, pull: 5, apply: Apply::Multiply } => "0300000000ffffffff640000000500000001",
        );
    }
}
