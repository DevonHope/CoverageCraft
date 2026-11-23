//! Where to put routers: a greedy search over spots on the floor. Each new router goes where it
//! raises the share of the floor at −67 dBm or better the most (then the weakest signals the
//! most), given the routers already placed. Deterministic: the same plan gives the same spots.
//!
//! It works on a sample of the floor (about every 0.5 m, at most [`MAX_SAMPLES`] points) and a
//! coarser set of candidate spots (at most [`MAX_CANDIDATES`]), so it stays well under a second
//! on a flat.

use crate::materials::Band;
use crate::model::{Cell, Emitter, FAIR_DBM, Grid, Model, Scratch, WallMap, signal_at};

/// The most floor points a spot is judged on.
pub const MAX_SAMPLES: usize = 600;
/// The most spots tried for each router.
pub const MAX_CANDIDATES: usize = 160;

/// The floor cells on a lattice of every `k`-th row and column, thinned until at most `max`.
fn lattice(grid: &Grid, cells: &[Cell], spacing_mm: f64, max: usize) -> Vec<[f64; 2]> {
    let mut k = ((spacing_mm / grid.step).ceil() as usize).max(1);
    loop {
        let pts: Vec<[f64; 2]> = (0..grid.rows)
            .step_by(k)
            .flat_map(|row| {
                (0..grid.columns)
                    .step_by(k)
                    .map(move |column| (row, column))
            })
            .filter(|(row, column)| cells.get(row * grid.columns + column) == Some(&Cell::Floor))
            .map(|(row, column)| grid.centre(column, row))
            .collect();
        if pts.len() <= max || k > grid.columns.max(grid.rows) {
            return pts;
        }
        k += 1;
    }
}

/// What a set of signals over the samples scores: the samples at −67 dBm or better, then the sum
/// of the signals (each capped at −40 dBm, so the weak places count most).
fn score(best: &[f64]) -> (usize, f64) {
    let good = best.iter().filter(|v| **v >= FAIR_DBM).count();
    let sum = best.iter().map(|v| v.clamp(-110.0, -40.0)).sum();
    (good, sum)
}

/// A spot tried: its score, where it is, and the best signal at each sample with it.
type Tried = ((usize, f64), [f64; 2], Vec<f64>);

/// A spot found for a new router, and the share of the sampled floor (0–100) at −67 dBm or
/// better with it and those before it.
pub struct Spot {
    pub at: [f64; 2],
    pub pct_at_least_67: f64,
}

/// Spots for `count` more routers like `template` (its position is ignored), given `existing`.
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    walls: &WallMap,
    grid: &Grid,
    cells: &[Cell],
    existing: &[Emitter],
    template: &Emitter,
    model: &Model,
    band: Band,
    count: usize,
) -> Vec<Spot> {
    let samples = lattice(grid, cells, 500.0, MAX_SAMPLES);
    let candidates = lattice(grid, cells, 750.0, MAX_CANDIDATES);
    if samples.is_empty() || candidates.is_empty() {
        return Vec::new();
    }
    let mut scratch = Scratch::default();
    let mut best: Vec<f64> = samples
        .iter()
        .map(|p| {
            existing
                .iter()
                .filter_map(|e| signal_at(walls, e, model, band, *p, &mut scratch))
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();
    let mut spots = Vec::new();
    for _ in 0..count {
        let mut chosen: Option<Tried> = None;
        for c in &candidates {
            let e = Emitter {
                at: *c,
                ..template.clone()
            };
            let with: Vec<f64> = samples
                .iter()
                .zip(&best)
                .map(|(p, b)| {
                    signal_at(walls, &e, model, band, *p, &mut scratch).map_or(*b, |v| v.max(*b))
                })
                .collect();
            let s = score(&with);
            if chosen
                .as_ref()
                .is_none_or(|(cs, _, _)| s.0 > cs.0 || (s.0 == cs.0 && s.1 > cs.1))
            {
                chosen = Some((s, *c, with));
            }
        }
        let Some(((good, _), at, with)) = chosen else {
            break;
        };
        best = with;
        spots.push(Spot {
            at,
            pct_at_least_67: (good as f64 / samples.len() as f64 * 1000.0).round() / 10.0,
        });
    }
    spots
}
