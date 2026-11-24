// Ported from Sagax (https://github.com/STARSCrazy/sagax), revision fab00c0, crates/sagax-decide/src/memory/route.rs.
use knorke_grid::Pos;

/// How the distance between two places is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Metric {
    /// Straight line; [`distance`] returns the square (`dx² + dy²`).
    Euclid = 0,
    /// Sum of the axis distances (`|dx| + |dy|`).
    Manhattan = 1,
    /// Largest axis distance (`max(|dx|, |dy|)`).
    Chebyshev = 2,
    /// Hex grid in axial coordinates (`q = x`, `r = y`):
    /// `(|dq| + |dr| + |dq + dr|) / 2`. The sum is even for whole hex steps;
    /// with a divisor the axis differences are truncated first, and the
    /// final `/ 2` truncates an odd sum.
    Hex = 3,
}

/// Walking distance between two places, for the decisions that ask how far
/// a figure has to walk (see [`SquadRules::walk`](crate::group::SquadRules::walk)).
///
/// It knows the map; the decide crate does not. The facade's
/// `FlowDistance` is a ready one built on flow fields.
pub trait Distance {
    /// Walking distance from `from` to `to` in metric units (cells), or
    /// `None` when `to` cannot be reached from `from`.
    fn distance(&mut self, from: Pos, to: Pos) -> Option<u32>;
}

impl<T: Distance + ?Sized> Distance for &mut T {
    fn distance(&mut self, from: Pos, to: Pos) -> Option<u32> {
        (**self).distance(from, to)
    }
}

/// Which candidates get a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// The `k` nearest, nearest first; a tie keeps the earlier candidate first.
    Nearest(usize),
    /// Every candidate within this plain distance (in the metric, after the
    /// divisor), inclusive, in candidate order. Negative reaches nobody.
    InRange(i64),
    /// Every candidate, in order.
    All,
    /// This one id, if it is among the candidates.
    To(u32),
}

/// Distance for the axis differences `dx`, `dy` under `metric`, computed in
/// `i128` so nothing overflows. [`Metric::Euclid`] returns the square.
pub fn distance(metric: Metric, dx: i64, dy: i64) -> i128 {
    let (dx, dy) = (i128::from(dx), i128::from(dy));
    match metric {
        Metric::Euclid => dx * dx + dy * dy,
        Metric::Manhattan => dx.abs() + dy.abs(),
        Metric::Chebyshev => dx.abs().max(dy.abs()),
        Metric::Hex => (dx.abs() + dy.abs() + (dx + dy).abs()) / 2,
    }
}

/// Picks recipients from `candidates` `(id, x, y)` for a message sent from
/// `origin`, writing their ids to `out` (cleared first).
///
/// Each axis difference is divided (truncating toward zero) by `divisor`
/// first; a `divisor` of 0 or less counts as 1. The caller filters the
/// candidates (team, free, ...). For [`Route::InRange`] with
/// [`Metric::Euclid`] the squared distance is compared with `r²`.
pub fn route(
    rule: Route,
    origin: (i32, i32),
    candidates: &[(u32, i32, i32)],
    metric: Metric,
    divisor: i32,
    out: &mut Vec<u32>,
) {
    out.clear();
    let div = i64::from(divisor.max(1));
    let dist = |x: i32, y: i32| {
        let dx = (i64::from(x) - i64::from(origin.0)) / div;
        let dy = (i64::from(y) - i64::from(origin.1)) / div;
        distance(metric, dx, dy)
    };
    match rule {
        Route::Nearest(k) => {
            if k == 0 {
                return;
            }
            let mut best: Vec<(i128, u32)> = Vec::with_capacity(k.min(candidates.len()));
            for &(id, x, y) in candidates {
                let d = dist(x, y);
                // First slot with a strictly greater distance: ties stay ahead.
                let at = best.iter().position(|b| b.0 > d).unwrap_or(best.len());
                if at < k {
                    best.insert(at, (d, id));
                    best.truncate(k);
                }
            }
            out.extend(best.iter().map(|b| b.1));
        }
        Route::InRange(r) => {
            if r < 0 {
                return;
            }
            let limit = match metric {
                Metric::Euclid => i128::from(r) * i128::from(r),
                _ => i128::from(r),
            };
            out.extend(
                candidates
                    .iter()
                    .filter(|&&(_, x, y)| dist(x, y) <= limit)
                    .map(|c| c.0),
            );
        }
        Route::All => out.extend(candidates.iter().map(|c| c.0)),
        Route::To(id) => {
            if candidates.iter().any(|c| c.0 == id) {
                out.push(id);
            }
        }
    }
}

/// A routing rule, e.g. inside a host's command or order. `Nearest` saves its count as u64.
impl knorke_types::SnapshotValue for Route {
    fn put(&self, w: &mut knorke_types::Writer<'_>) {
        match *self {
            Route::Nearest(k) => {
                w.put(&0u8);
                w.put(&(k as u64));
            }
            Route::InRange(r) => {
                w.put(&1u8);
                w.put(&r);
            }
            Route::All => w.put(&2u8),
            Route::To(id) => {
                w.put(&3u8);
                w.put(&id);
            }
        }
    }
    fn get(r: &mut knorke_types::Reader<'_>) -> Result<Self, knorke_types::SnapshotError> {
        use knorke_types::SnapshotError;
        Ok(match r.get::<u8>()? {
            0 => Route::Nearest(
                usize::try_from(r.get::<u64>()?)
                    .map_err(|_| SnapshotError::Invalid("knorke.ai: route count"))?,
            ),
            1 => Route::InRange(r.get()?),
            2 => Route::All,
            3 => Route::To(r.get()?),
            _ => return Err(SnapshotError::Invalid("knorke.ai: route rule")),
        })
    }
}

// A plain value: the metric of a routing or walking rule.
// Tags pinned by `snapshot_tests::value_bytes_are_pinned`.
knorke_types::by_tag! {
    Metric, knorke_types::SnapshotError::Invalid("knorke.ai: metric") => {
        0 => Euclid,
        1 => Manhattan,
        2 => Chebyshev,
        3 => Hex,
    };
}
