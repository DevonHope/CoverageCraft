//! Channel suggestions: which channel each router should use in a band so that, over as much of
//! the floor as possible, no router on an overlapping channel is heard where another serves.
//!
//! The score is the Overlap map's own rule: on each floor cell the serving router is the
//! strongest, and any other router heard at [`HEARD_DBM`] or more on a channel overlapping the
//! serving one's takes its airtime. Who serves and who is heard don't depend on channels, so they
//! are worked out once per cell ([`table`]); each assignment is then scored against that.
//!
//! Candidates are the band's channels at each router's own width whose every 20 MHz channel is
//! allowed ([`allowed`]: a conservative default, the caller's list instead). The search tries
//! every assignment when there are few, otherwise assigns greedily and improves one router at a
//! time; ties go to fewer changes, then to lower channels, so the same input gives the same
//! answer.

use std::collections::BTreeMap;

use crate::materials::Band;
use crate::radio::{Channel, HEARD_DBM, numbers};

/// What a floor's cells say about who serves and who is heard, grouped by kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// (serving router, other routers heard at [`HEARD_DBM`] or more) → how many cells.
    rows: BTreeMap<(usize, Vec<usize>), usize>,
    /// The floor's cells, the share's denominator (cells no router reaches included).
    pub cells: usize,
}

/// The table for one floor: `signals` is `cell * routers + router` (dBm, NaN where not worked
/// out), `floor` says which cells are floor.
pub fn table(signals: &[f32], routers: usize, floor: &dyn Fn(usize) -> bool) -> Table {
    let mut t = Table::default();
    let cells = signals.len().checked_div(routers).unwrap_or(0);
    let mut heard: Vec<(f32, usize)> = Vec::with_capacity(routers);
    for i in (0..cells).filter(|i| floor(*i)) {
        t.cells += 1;
        heard.clear();
        if let Some(row) = signals.get(i * routers..(i + 1) * routers) {
            heard.extend(
                row.iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_finite())
                    .map(|(k, v)| (*v, k)),
            );
        }
        // As the maps sort them: strongest first, the lower index first among equals.
        heard.sort_by(|a, b| b.0.total_cmp(&a.0));
        let Some(&(_, serving)) = heard.first() else {
            continue;
        };
        let mut others: Vec<usize> = heard
            .iter()
            .skip(1)
            .filter(|(v, _)| f64::from(*v) >= HEARD_DBM)
            .map(|(_, k)| *k)
            .collect();
        others.sort_unstable();
        *t.rows.entry((serving, others)).or_insert(0) += 1;
    }
    t
}

/// How an assignment does on `tables`: the cells with no overlapping router heard (where the
/// serving one has a channel), and the overlapping routers heard summed over cells.
pub fn score(tables: &[Table], channels: &[Option<Channel>], band: Band) -> (usize, usize) {
    let (mut clear, mut heard) = (0, 0);
    for t in tables {
        for ((serving, others), n) in &t.rows {
            let Some(own) = channels.get(*serving).copied().flatten() else {
                continue;
            };
            let k = others
                .iter()
                .filter(|j| {
                    channels
                        .get(**j)
                        .copied()
                        .flatten()
                        .is_some_and(|c| c.overlaps(own, band))
                })
                .count();
            if k == 0 {
                clear += n;
            }
            heard += k * n;
        }
    }
    (clear, heard)
}

/// The 20 MHz channels allowed by default: 1, 6 and 11 on 2.4 GHz; on 5 GHz UNII-1 and UNII-3
/// (36–48, 149–165), with the DFS channels (52–64, 100–144) when `dfs`, never UNII-4 (169–177,
/// not allowed in many countries); every 6 GHz channel.
pub fn allowed(band: Band, dfs: bool) -> Vec<u16> {
    match band {
        Band::G2_4 => vec![1, 6, 11],
        Band::G5 => numbers(band)
            .into_iter()
            .filter(|n| {
                (36..=48).contains(n)
                    || (149..=165).contains(n)
                    || (dfs && ((52..=64).contains(n) || (100..=144).contains(n)))
            })
            .collect(),
        Band::G6 => numbers(band),
    }
}

/// A 6 GHz preferred scanning channel: 5, 21, 37 … 229, the ones clients look on first.
fn psc(n: u16) -> bool {
    n >= 5 && (n - 5).is_multiple_of(16)
}

/// The channels of `width` in `band` that cover only `allowed` 20 MHz channels, one per block of
/// spectrum (its lowest number; on 6 GHz its preferred scanning channel), lowest first.
pub fn candidates(band: Band, width: u16, allowed: &[u16]) -> Vec<Channel> {
    let mut out: Vec<Channel> = Vec::new();
    for n in numbers(band) {
        if band == Band::G6 && !psc(n) {
            continue;
        }
        let Some(c) = Channel::new(band, n, width) else {
            continue;
        };
        let covers = c.covers(band);
        if covers.is_empty() || !covers.iter().all(|x| allowed.contains(x)) {
            continue;
        }
        if out.iter().any(|o| o.spectrum(band) == c.spectrum(band)) {
            continue;
        }
        out.push(c);
    }
    out
}

/// Above this many assignments the search is greedy plus improvement instead of every one.
const EXHAUSTIVE: usize = 20_000;

/// The order assignments are judged in: more clear cells, then fewer overlapping routers heard,
/// then fewer changes from `current`.
fn key(
    tables: &[Table],
    a: &[Option<Channel>],
    current: &[Option<Channel>],
    band: Band,
) -> (std::cmp::Reverse<usize>, usize, usize) {
    let (clear, heard) = score(tables, a, band);
    let changes = a.iter().zip(current).filter(|(x, y)| x != y).count();
    (std::cmp::Reverse(clear), heard, changes)
}

/// The channels for every router: `current` for the ones not in `free`; for each free router
/// `free[i]`, one of `options[i]`. The best by [`key`]; `current` when nothing beats it.
pub fn suggest(
    tables: &[Table],
    band: Band,
    current: &[Option<Channel>],
    free: &[usize],
    options: &[Vec<Channel>],
) -> Vec<Option<Channel>> {
    let mut best = current.to_vec();
    let mut best_key = key(tables, &best, current, band);
    let mut consider = |a: &[Option<Channel>], best: &mut Vec<Option<Channel>>| {
        let k = key(tables, a, current, band);
        if k < best_key {
            best_key = k;
            *best = a.to_vec();
        }
    };
    let total = options
        .iter()
        .try_fold(1usize, |p, o| p.checked_mul(o.len().max(1)));
    let mut a = current.to_vec();
    if total.is_some_and(|t| t <= EXHAUSTIVE) {
        // Every assignment, in order (the first router's choice changing slowest).
        let mut idx = vec![0usize; free.len()];
        loop {
            for ((r, i), o) in free.iter().zip(&idx).zip(options) {
                if let Some(slot) = a.get_mut(*r) {
                    *slot = o.get(*i).copied();
                }
            }
            consider(&a, &mut best);
            // The next index, last router first.
            let mut k = free.len();
            loop {
                if k == 0 {
                    return best;
                }
                k -= 1;
                let len = options.get(k).map_or(0, Vec::len);
                if let Some(i) = idx.get_mut(k) {
                    *i += 1;
                    if *i < len {
                        break;
                    }
                    *i = 0;
                }
            }
        }
    }
    // Greedy, the busiest router first, each picking the best given those before it…
    for (r, _) in free.iter().zip(options) {
        if let Some(slot) = a.get_mut(*r) {
            *slot = None;
        }
    }
    let load = |r: usize| {
        tables
            .iter()
            .flat_map(|t| t.rows.iter())
            .filter(|((s, _), _)| *s == r)
            .map(|(_, n)| *n)
            .sum::<usize>()
    };
    let mut order: Vec<usize> = (0..free.len()).collect();
    order.sort_by_key(|i| (std::cmp::Reverse(free.get(*i).map_or(0, |r| load(*r))), *i));
    let pick = |a: &mut Vec<Option<Channel>>, i: usize| -> bool {
        let (Some(r), Some(opts)) = (free.get(i).copied(), options.get(i)) else {
            return false;
        };
        let was = a.get(r).copied().flatten();
        let mut top = (key(tables, a, current, band), was);
        for c in opts {
            if let Some(slot) = a.get_mut(r) {
                *slot = Some(*c);
            }
            let k = key(tables, a, current, band);
            if k < top.0 {
                top = (k, Some(*c));
            }
        }
        if let Some(slot) = a.get_mut(r) {
            *slot = top.1;
        }
        top.1 != was
    };
    for i in &order {
        pick(&mut a, *i);
    }
    // … then one router at a time while that helps.
    for _ in 0..100 {
        let mut moved = false;
        for i in 0..free.len() {
            moved |= pick(&mut a, i);
        }
        if !moved {
            break;
        }
    }
    consider(&a, &mut best);
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(band: Band, n: u16, w: u16) -> Option<Channel> {
        Channel::new(band, n, w)
    }

    #[test]
    fn candidates_follow_the_channel_plan() {
        let b = Band::G2_4;
        let c: Vec<u16> = candidates(b, 20, &allowed(b, false))
            .iter()
            .map(|c| c.number)
            .collect();
        assert_eq!(c, vec![1, 6, 11]);
        let b = Band::G5;
        let c: Vec<u16> = candidates(b, 80, &allowed(b, false))
            .iter()
            .map(|c| c.number)
            .collect();
        assert_eq!(c, vec![36, 149]);
        let c: Vec<u16> = candidates(b, 80, &allowed(b, true))
            .iter()
            .map(|c| c.number)
            .collect();
        assert_eq!(c, vec![36, 52, 100, 116, 132, 149]);
        let c: Vec<u16> = candidates(b, 20, &allowed(b, false))
            .iter()
            .map(|c| c.number)
            .collect();
        assert_eq!(c, vec![36, 40, 44, 48, 149, 153, 157, 161, 165]);
        let c: Vec<u16> = candidates(b, 160, &allowed(b, false))
            .iter()
            .map(|c| c.number)
            .collect();
        assert!(c.is_empty(), "{c:?}");
        let b = Band::G6;
        let c: Vec<u16> = candidates(b, 80, &allowed(b, false))
            .iter()
            .map(|c| c.number)
            .collect();
        assert_eq!(c.first(), Some(&5));
        assert_eq!(c.len(), 14);
        assert!(c.iter().all(|n| psc(*n)));
    }

    /// Three routers in a row, each heard where its neighbours serve: 1, 6 and 11 clear every
    /// cell; the middle one heard by both ends needs its own channel.
    #[test]
    fn neighbours_get_different_channels() {
        let b = Band::G2_4;
        let mut t = Table::default();
        t.rows.insert((0, vec![1]), 10);
        t.rows.insert((1, vec![0, 2]), 10);
        t.rows.insert((2, vec![1]), 10);
        t.cells = 30;
        let opts = vec![candidates(b, 20, &allowed(b, false)); 3];
        let got = suggest(&[t.clone()], b, &[None, None, None], &[0, 1, 2], &opts);
        assert_eq!(score(&[t.clone()], &got, b), (30, 0));
        assert_ne!(got[0], got[1]);
        assert_ne!(got[1], got[2]);
        // The same input, the same answer; and a kept router stays.
        assert_eq!(
            suggest(&[t.clone()], b, &[None, None, None], &[0, 1, 2], &opts),
            got
        );
        let kept = suggest(
            &[t.clone()],
            b,
            &[ch(b, 6, 20), None, None],
            &[1, 2],
            &opts[1..],
        );
        assert_eq!(kept[0], ch(b, 6, 20));
        assert_eq!(score(&[t], &kept, b), (30, 0));
    }

    /// Four routers all hearing each other with three channels: the exhaustive search finds the
    /// best, and the greedy search (forced by many options) does as well here.
    #[test]
    fn more_routers_than_channels() {
        let b = Band::G2_4;
        let mut t = Table::default();
        for r in 0..4usize {
            let others: Vec<usize> = (0..4).filter(|o| *o != r).collect();
            t.rows.insert((r, others), 10 + r);
        }
        t.cells = 46;
        let opts = vec![candidates(b, 20, &allowed(b, false)); 4];
        let got = suggest(&[t.clone()], b, &[None; 4], &[0, 1, 2, 3], &opts);
        // Two share a channel: the two serving least (0 and 1, 10 and 11 cells).
        let (clear, _) = score(&[t.clone()], &got, b);
        assert_eq!(clear, 12 + 13);
        assert_eq!(got[0], got[1]);
        // Brute force over all 81 agrees.
        let mut best = 0;
        for i in 0..81usize {
            let a: Vec<Option<Channel>> = (0..4)
                .map(|k| opts[0].get(i / 3usize.pow(k) % 3).copied())
                .collect();
            best = best.max(score(&[t.clone()], &a, b).0);
        }
        assert_eq!(clear, best);
        // A 2.4 GHz width can't widen; 20 MHz in the 2.4 band everywhere.
        assert!(got.iter().all(|c| c.is_some_and(|c| c.width == 20)));
    }

    /// Too many assignments to try them all (15 channels, 4 routers): the greedy search with
    /// improvement still clears every cell when there are enough channels.
    #[test]
    fn many_options_use_the_greedy_search() {
        let b = Band::G6;
        let mut t = Table::default();
        for r in 0..4usize {
            let others: Vec<usize> = (0..4).filter(|o| *o != r).collect();
            t.rows.insert((r, others), 5);
        }
        t.cells = 20;
        let opts = vec![candidates(b, 20, &allowed(b, false)); 4];
        assert!(opts[0].len().pow(4) > EXHAUSTIVE);
        let got = suggest(&[t.clone()], b, &[None; 4], &[0, 1, 2, 3], &opts);
        assert_eq!(score(&[t.clone()], &got, b), (20, 0));
        assert_eq!(suggest(&[t], b, &[None; 4], &[0, 1, 2, 3], &opts), got);
    }

    #[test]
    fn the_table_follows_the_maps() {
        // Two routers, three cells: 0 serves and hears 1; 1 serves, 0 too weak; nothing.
        let s = [-50.0, -70.0, -90.0, -60.0, f32::NAN, f32::NAN];
        let t = table(&s, 2, &|_| true);
        assert_eq!(t.cells, 3);
        assert_eq!(t.rows.get(&(0, vec![1])), Some(&1));
        assert_eq!(t.rows.get(&(1, vec![])), Some(&1));
    }
}
