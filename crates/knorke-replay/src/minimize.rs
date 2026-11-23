use crate::ReplayLog;
use alloc::vec::Vec;

/// Shrinks a failing replay to a small set of commands that still makes `fails` return
/// `true` (delta debugging, Zeller's ddmin on command level).
///
/// `fails` typically restores the start state, replays the candidate and checks an
/// invariant. Candidates keep the tick structure and start state but carry no checksums
/// or keyframes. The result is 1-minimal: removing any single remaining command makes the
/// failure disappear. Cost: O(n²) calls of `fails` in the worst case, usually far fewer.
///
/// Candidates may contain command sequences the game would never produce (e.g. a "use"
/// without the preceding "pick up"). The simulation must tolerate that – validating
/// commands at the sync point is good practice anyway.
pub fn minimize<C: Clone>(
    log: &ReplayLog<C>,
    mut fails: impl FnMut(&ReplayLog<C>) -> bool,
) -> ReplayLog<C> {
    let mut keep: Vec<usize> = (0..log.command_count()).collect();
    let mut mask = alloc::vec![false; log.command_count()];
    let mut build = |subset: &[usize]| {
        mask.iter_mut().for_each(|m| *m = false);
        subset.iter().for_each(|&i| mask[i] = true);
        log.filtered(|i| mask[i])
    };

    if fails(&build(&[])) {
        return build(&[]);
    }
    let mut n = 2usize;
    while keep.len() >= 2 {
        let chunk = keep.len().div_ceil(n);
        let part =
            |i: usize| -> Vec<usize> { keep.iter().copied().skip(i * chunk).take(chunk).collect() };
        let rest = |i: usize| -> Vec<usize> {
            keep.iter()
                .enumerate()
                .filter(|(k, _)| k / chunk != i)
                .map(|(_, &v)| v)
                .collect()
        };
        // Try each chunk alone, then each complement.
        if let Some(smaller) = (0..n)
            .map(part)
            .find(|p| !p.is_empty() && p.len() < keep.len() && fails(&build(p)))
        {
            keep = smaller;
            n = 2;
        } else if let Some(smaller) = (0..n)
            .map(rest)
            .find(|r| r.len() < keep.len() && fails(&build(r)))
        {
            keep = smaller;
            n = (n - 1).max(2);
        } else if n >= keep.len() {
            break;
        } else {
            n = (n * 2).min(keep.len());
        }
    }
    build(&keep)
}
