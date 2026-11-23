//! Measured signal: points read from a file (or typed), compared with the prediction, and a
//! calibration that fits the model to them.
//!
//! The calibration fits one path-loss exponent for all bands and one offset (dB) per band, by
//! least squares: with each point's prediction split into its distance term and the rest, the
//! measured-minus-rest residual is a straight line in log10(distance) per band, sharing a slope.
//!
//! It can fit the walls' losses too, per material and band, where enough points cross them: the
//! same least squares with a column per material (how many times each point's path crosses it),
//! each loss held toward its current value by a prior, and values that leave their range fixed
//! at its end while the rest are fitted again. Slabs and custom losses stay as they are.

use serde_json::Value;

use crate::materials::Band;

/// The most points a survey holds.
pub const MAX_POINTS: usize = 100_000;

/// One measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// Plan x, y (mm).
    pub at: [f64; 2],
    pub band: Band,
    pub rssi_dbm: f64,
    /// The router it was measured from, by name; `None` for the strongest.
    pub router: Option<String>,
    /// The floor it was measured on (by index); `None`: the one shown when it was read.
    pub floor: Option<usize>,
}

fn point(
    at: [f64; 2],
    band: &str,
    rssi: f64,
    router: Option<String>,
    floor: Option<usize>,
) -> Result<Point, String> {
    let band = Band::parse(band).ok_or_else(|| format!("band `{band}` is 2.4, 5 or 6"))?;
    if !(at.iter().all(|v| v.is_finite() && v.abs() <= 1.0e8)) {
        return Err("a position must be finite numbers in mm".into());
    }
    if !(rssi.is_finite() && (-120.0..=0.0).contains(&rssi)) {
        return Err(format!("a signal of {rssi} dBm is outside -120 to 0"));
    }
    Ok(Point {
        at,
        band,
        rssi_dbm: rssi,
        router: router.filter(|r| !r.trim().is_empty()),
        floor,
    })
}

/// A point from `{at: [x, y], band, rssi_dbm, router?}`.
pub fn from_json(v: &Value) -> Result<Point, String> {
    let at = v
        .get("at")
        .and_then(Value::as_array)
        .and_then(|a| match a.as_slice() {
            [x, y] => Some([x.as_f64()?, y.as_f64()?]),
            _ => None,
        })
        .ok_or("`at` is [x, y] in mm")?;
    let band = match v.get("band") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err("`band` is 2.4, 5 or 6".into()),
    };
    let rssi = v
        .get("rssi_dbm")
        .and_then(Value::as_f64)
        .ok_or("`rssi_dbm` is the measured signal")?;
    let router = v.get("router").and_then(Value::as_str).map(str::to_string);
    let floor = match v.get("floor") {
        None | Some(Value::Null) => None,
        Some(f) => Some(
            f.as_u64()
                .and_then(|f| usize::try_from(f).ok())
                .ok_or("`floor` is a floor's index")?,
        ),
    };
    point(at, &band, rssi, router, floor)
}

/// The points in a CSV file (a header naming `x`, `y`, `band`, `rssi_dbm` and optionally
/// `router`, in any order; mm and dBm) or a JSON list of points.
pub fn parse(text: &str, json: bool) -> Result<Vec<Point>, String> {
    let points = if json {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let list = v
            .as_array()
            .or_else(|| v.get("points").and_then(Value::as_array))
            .ok_or("a JSON survey is a list of points, or {\"points\": [...]}")?;
        if list.len() > MAX_POINTS {
            return Err(format!("more than {MAX_POINTS} points"));
        }
        list.iter()
            .enumerate()
            .map(|(i, p)| from_json(p).map_err(|e| format!("point {}: {e}", i + 1)))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header: Vec<String> = lines
            .next()
            .ok_or("the file is empty")?
            .split(',')
            .map(|h| h.trim().trim_matches('"').to_ascii_lowercase())
            .collect();
        let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
        let (x, y, band, rssi) = (
            col(&["x", "x_mm"]).ok_or("no `x` column")?,
            col(&["y", "y_mm"]).ok_or("no `y` column")?,
            col(&["band", "ghz"]).ok_or("no `band` column")?,
            col(&["rssi_dbm", "rssi", "dbm"]).ok_or("no `rssi_dbm` column")?,
        );
        let router = col(&["router", "ap"]);
        let floor = col(&["floor"]);
        let mut out = Vec::new();
        for (i, line) in lines.enumerate() {
            if out.len() >= MAX_POINTS {
                return Err(format!("more than {MAX_POINTS} points"));
            }
            let f: Vec<&str> = line
                .split(',')
                .map(|c| c.trim().trim_matches('"'))
                .collect();
            let num = |c: usize| -> Result<f64, String> {
                f.get(c).and_then(|v| v.parse::<f64>().ok()).ok_or(format!(
                    "line {}: column {} isn't a number",
                    i + 2,
                    c + 1
                ))
            };
            out.push(
                point(
                    [num(x)?, num(y)?],
                    f.get(band).copied().unwrap_or_default(),
                    num(rssi)?,
                    router.and_then(|c| f.get(c)).map(|s| s.to_string()),
                    match floor.and_then(|c| f.get(c)).filter(|s| !s.is_empty()) {
                        None => None,
                        Some(s) => Some(s.parse::<usize>().map_err(|_| {
                            format!("line {}: the floor isn't a floor's index", i + 2)
                        })?),
                    },
                )
                .map_err(|e| format!("line {}: {e}", i + 2))?,
            );
        }
        out
    };
    if points.is_empty() {
        return Err("no points".into());
    }
    Ok(points)
}

/// The figures of the differences measured − predicted (dB).
#[derive(Clone, Debug, PartialEq)]
pub struct Errors {
    pub count: usize,
    /// The mean difference: positive when the model predicts too little.
    pub mean: f64,
    pub rms: f64,
    pub max_abs: f64,
}

pub fn errors(diffs: &[f64]) -> Option<Errors> {
    if diffs.is_empty() {
        return None;
    }
    let n = diffs.len() as f64;
    Some(Errors {
        count: diffs.len(),
        mean: diffs.iter().sum::<f64>() / n,
        rms: (diffs.iter().map(|d| d * d).sum::<f64>() / n).sqrt(),
        max_abs: diffs.iter().fold(0.0, |m, d| m.max(d.abs())),
    })
}

/// One point's terms for the fit: its band, log10 of its distance (m), and the measured signal
/// minus every term of the prediction but the distance's and the offset.
/// `crossed`: the walls its path counts that a calibration can fit (a preset's id and its loss
/// now, once per crossing); their losses are in `rest` already.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub band: Band,
    pub log_d: f64,
    pub rest: f64,
    pub crossed: Vec<(&'static str, f64)>,
}

/// The exponent and per-band offsets that fit `terms` best, the exponent kept in `lo..=hi`;
/// `current` when the distances don't vary enough to fit an exponent (then only offsets).
pub fn fit(terms: &[Term], current: f64, lo: f64, hi: f64) -> (f64, [Option<f64>; 3]) {
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut means = [(0.0, 0.0, 0usize); 3];
    for t in terms {
        if let Some(m) = means.get_mut(t.band.index()) {
            m.0 += t.log_d;
            m.1 += t.rest;
            m.2 += 1;
        }
    }
    for m in &mut means {
        if m.2 > 0 {
            m.0 /= m.2 as f64;
            m.1 /= m.2 as f64;
        }
    }
    for t in terms {
        if let Some(m) = means.get(t.band.index()) {
            sxx += (t.log_d - m.0).powi(2);
            sxy += (t.log_d - m.0) * (t.rest - m.1);
        }
    }
    // rest = −10·n·log10(d) + offset: the slope is −10·n.
    let n = if sxx > 0.05 {
        (-sxy / sxx / 10.0).clamp(lo, hi)
    } else {
        current
    };
    let mut offsets = [None; 3];
    for (slot, m) in offsets.iter_mut().zip(means) {
        if m.2 > 0 {
            *slot = Some(m.1 + 10.0 * n * m.0);
        }
    }
    (n, offsets)
}

/// A material is fitted in a band only when at least this many points there cross it.
pub const MIN_POINTS: usize = 5;
/// How strongly a wall's loss is held to its current value: as much as this many points that
/// agree with it exactly. Higher keeps losses near the presets on thin evidence; lower lets a few
/// points (and their noise) move them further.
pub const PRIOR_WEIGHT: f64 = 2.0;
pub const EXPONENT_RANGE: (f64, f64) = (1.5, 6.0);
pub const OFFSET_RANGE: (f64, f64) = (-30.0, 30.0);
pub const LOSS_RANGE: (f64, f64) = (0.0, 60.0);
/// Below this spread of log10(distance) (summed squares) the exponent isn't fitted.
const MIN_SPREAD: f64 = 0.05;

/// What became of a wall's loss in a fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Fitted to the points.
    Fitted,
    /// Too few points cross it, or all cross it alike: kept as it was.
    Kept,
    /// Fitted, but held at the end of its range.
    Clamped,
}

impl Status {
    pub fn id(self) -> &'static str {
        match self {
            Status::Fitted => "fitted",
            Status::Kept => "kept",
            Status::Clamped => "clamped",
        }
    }
}

/// A material's loss in one band, as a fit found it.
#[derive(Clone, Debug, PartialEq)]
pub struct WallLoss {
    pub material: &'static str,
    pub band: Band,
    /// How many points' paths cross it.
    pub points: usize,
    /// Its loss before (dB).
    pub prior: f64,
    /// Its loss after (dB): the prior unless fitted.
    pub value: f64,
    pub status: Status,
}

/// The walls the points' paths cross, per material and band (each with its loss now), and
/// whether enough points cross each, differently enough, to fit it.
pub fn wall_candidates(terms: &[Term]) -> Vec<(WallLoss, bool)> {
    let mut out: Vec<(WallLoss, bool)> = Vec::new();
    for t in terms {
        for (k, loss) in &t.crossed {
            if !out
                .iter()
                .any(|(w, _)| w.material == *k && w.band == t.band)
            {
                out.push((
                    WallLoss {
                        material: k,
                        band: t.band,
                        points: 0,
                        prior: *loss,
                        value: *loss,
                        status: Status::Kept,
                    },
                    false,
                ));
            }
        }
    }
    for (w, ok) in &mut out {
        let counts: Vec<usize> = terms
            .iter()
            .filter(|t| t.band == w.band)
            .map(|t| t.crossed.iter().filter(|(k, _)| *k == w.material).count())
            .collect();
        w.points = counts.iter().filter(|c| **c > 0).count();
        let varies = counts.iter().min() != counts.iter().max();
        *ok = w.points >= MIN_POINTS && varies;
    }
    out.sort_by(|a, b| (a.0.material, a.0.band).cmp(&(b.0.material, b.0.band)));
    out
}

/// What a joint fit found.
#[derive(Clone, Debug, PartialEq)]
pub struct WallFit {
    pub exponent: f64,
    pub offsets: [Option<f64>; 3],
    /// The fitted walls' losses (the ones given).
    pub losses: Vec<WallLoss>,
    /// The exponent and offsets held at the end of their range (`exponent`, `offset 5`…).
    pub clamped: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Param {
    Exponent,
    Offset(Band),
    Loss(&'static str, Band),
}

impl Param {
    fn coef(self, t: &Term) -> f64 {
        match self {
            Param::Exponent => -10.0 * t.log_d,
            Param::Offset(b) => f64::from(u8::from(t.band == b)),
            Param::Loss(k, b) if t.band == b => {
                -(t.crossed.iter().filter(|(m, _)| *m == k).count() as f64)
            }
            Param::Loss(..) => 0.0,
        }
    }

    fn range(self) -> (f64, f64) {
        match self {
            Param::Exponent => EXPONENT_RANGE,
            Param::Offset(_) => OFFSET_RANGE,
            Param::Loss(..) => LOSS_RANGE,
        }
    }
}

/// The unknowns for `terms` (the losses in `walls` as `(material, band, prior)`), with the
/// exponent fixed at `current` when distances don't vary enough to fit it.
fn params(
    terms: &[&Term],
    current: f64,
    walls: &[(&'static str, Band, f64)],
) -> (Vec<Param>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut ps = vec![Param::Exponent];
    let mut priors = vec![None];
    let mut fixed = vec![None];
    let mut spread = 0.0;
    for b in Band::ALL {
        let ds: Vec<f64> = terms
            .iter()
            .filter(|t| t.band == b)
            .map(|t| t.log_d)
            .collect();
        if ds.is_empty() {
            continue;
        }
        let mean = ds.iter().sum::<f64>() / ds.len() as f64;
        spread += ds.iter().map(|d| (d - mean).powi(2)).sum::<f64>();
        ps.push(Param::Offset(b));
        priors.push(None);
        fixed.push(None);
    }
    if spread <= MIN_SPREAD
        && let Some(f) = fixed.first_mut()
    {
        *f = Some(current);
    }
    for (k, b, prior) in walls {
        if terms.iter().any(|t| t.band == *b) {
            ps.push(Param::Loss(k, *b));
            priors.push(Some(*prior));
            fixed.push(None);
        }
    }
    (ps, priors, fixed)
}

/// A point's measured-minus-known part: its `rest` without the losses being fitted.
fn known(t: &Term, ps: &[Param]) -> f64 {
    t.rest
        - t.crossed
            .iter()
            .filter(|(k, _)| ps.contains(&Param::Loss(k, t.band)))
            .map(|(_, l)| l)
            .sum::<f64>()
}

/// Least squares for the unknowns not fixed, each loss held to its prior by [`PRIOR_WEIGHT`].
fn solve(
    terms: &[&Term],
    ps: &[Param],
    priors: &[Option<f64>],
    fixed: &[Option<f64>],
) -> Option<Vec<f64>> {
    let free: Vec<usize> = (0..ps.len())
        .filter(|i| fixed.get(*i).is_some_and(Option::is_none))
        .collect();
    let m = free.len();
    let mut ata = vec![vec![0.0; m]; m];
    let mut atb = vec![0.0; m];
    for t in terms {
        let mut y = known(t, ps);
        for (p, f) in ps.iter().zip(fixed) {
            if let Some(v) = f {
                y -= p.coef(t) * v;
            }
        }
        let row: Vec<f64> = free
            .iter()
            .filter_map(|i| ps.get(*i))
            .map(|p| p.coef(t))
            .collect();
        for (r, a) in row.iter().zip(ata.iter_mut()) {
            for (c, cell) in row.iter().zip(a.iter_mut()) {
                *cell += r * c;
            }
        }
        for (r, b) in row.iter().zip(atb.iter_mut()) {
            *b += r * y;
        }
    }
    for (j, i) in free.iter().enumerate() {
        if let Some(Some(prior)) = priors.get(*i)
            && let (Some(cell), Some(b)) =
                (ata.get_mut(j).and_then(|r| r.get_mut(j)), atb.get_mut(j))
        {
            *cell += PRIOR_WEIGHT;
            *b += PRIOR_WEIGHT * prior;
        }
    }
    let x = gauss(ata, atb)?;
    let mut out: Vec<f64> = fixed.iter().map(|f| f.unwrap_or(0.0)).collect();
    for (i, v) in free.iter().zip(x) {
        if let Some(slot) = out.get_mut(*i) {
            *slot = v;
        }
    }
    Some(out)
}

/// Solve, then fix whatever left its range at the end it passed and solve again, until all are
/// in range. The values, and which were fixed by a range.
fn solve_in_range(
    terms: &[&Term],
    ps: &[Param],
    priors: &[Option<f64>],
    mut fixed: Vec<Option<f64>>,
) -> Option<(Vec<f64>, Vec<bool>)> {
    let mut clamped = vec![false; ps.len()];
    for _ in 0..=ps.len() {
        let x = solve(terms, ps, priors, &fixed)?;
        let mut again = false;
        for (i, (p, v)) in ps.iter().zip(&x).enumerate() {
            let (lo, hi) = p.range();
            if fixed.get(i).is_some_and(Option::is_none) && !(lo..=hi).contains(v) {
                if let Some(f) = fixed.get_mut(i) {
                    *f = Some(v.clamp(lo, hi));
                }
                if let Some(c) = clamped.get_mut(i) {
                    *c = true;
                }
                again = true;
            }
        }
        if !again {
            return Some((x, clamped));
        }
    }
    None
}

/// The exponent, offsets and the losses of `walls` (`(material, band, prior)`) that fit `terms`
/// best; none when the points can't settle them.
pub fn fit_walls(
    terms: &[Term],
    current: f64,
    walls: &[(&'static str, Band, f64)],
) -> Option<WallFit> {
    let all: Vec<&Term> = terms.iter().collect();
    let (ps, priors, fixed) = params(&all, current, walls);
    let (x, clamped) = solve_in_range(&all, &ps, &priors, fixed)?;
    let mut fit = WallFit {
        exponent: current,
        offsets: [None; 3],
        losses: Vec::new(),
        clamped: Vec::new(),
    };
    for ((p, v), c) in ps.iter().zip(&x).zip(&clamped) {
        match p {
            Param::Exponent => {
                fit.exponent = *v;
                if *c {
                    fit.clamped.push("exponent".into());
                }
            }
            Param::Offset(b) => {
                if let Some(slot) = fit.offsets.get_mut(b.index()) {
                    *slot = Some(*v);
                }
                if *c {
                    fit.clamped.push(format!("offset {}", b.name()));
                }
            }
            Param::Loss(k, b) => {
                let prior = walls
                    .iter()
                    .find(|w| w.0 == *k && w.1 == *b)
                    .map_or(*v, |w| w.2);
                fit.losses.push(WallLoss {
                    material: k,
                    band: *b,
                    points: 0,
                    prior,
                    value: *v,
                    status: if *c { Status::Clamped } else { Status::Fitted },
                });
            }
        }
    }
    Some(fit)
}

/// Each point's error (measured − predicted, dB) when the fit is made without it: the points
/// are split into `folds` groups and each group is predicted by a fit to the others. Points whose
/// band the others don't have are left out.
pub fn held_out(
    terms: &[Term],
    current: f64,
    walls: &[(&'static str, Band, f64)],
    folds: usize,
) -> Vec<(Band, f64)> {
    let folds = folds.max(2);
    let mut out = Vec::new();
    for f in 0..folds {
        let train: Vec<&Term> = terms
            .iter()
            .enumerate()
            .filter(|(i, _)| i % folds != f)
            .map(|(_, t)| t)
            .collect();
        let (ps, priors, fixed) = params(&train, current, walls);
        let Some((x, _)) = solve_in_range(&train, &ps, &priors, fixed) else {
            continue;
        };
        for t in terms
            .iter()
            .enumerate()
            .filter(|(i, _)| i % folds == f)
            .map(|(_, t)| t)
        {
            if !ps.contains(&Param::Offset(t.band)) {
                continue;
            }
            let predicted: f64 = ps.iter().zip(&x).map(|(p, v)| p.coef(t) * v).sum();
            out.push((t.band, known(t, &ps) - predicted));
        }
    }
    out
}

/// Gaussian elimination with partial pivoting; none when the system is singular.
fn gauss(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    let scale = a
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.get(i))
        .fold(0.0f64, |m, v| m.max(v.abs()));
    let tiny = (scale * 1e-12).max(1e-300);
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| {
            let at = |r: usize| a.get(r).and_then(|r| r.get(c)).map_or(0.0, |v| v.abs());
            at(i).total_cmp(&at(j))
        })?;
        a.swap(c, p);
        b.swap(c, p);
        let pivot_row = a.get(c)?.clone();
        let pivot = *pivot_row.get(c)?;
        if pivot.abs() < tiny || !pivot.is_finite() {
            return None;
        }
        let bc = *b.get(c)?;
        for r in c + 1..n {
            let row = a.get_mut(r)?;
            let f = row.get(c)? / pivot;
            for (cell, pv) in row.iter_mut().zip(&pivot_row).skip(c) {
                *cell -= f * pv;
            }
            *b.get_mut(r)? -= f * bc;
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let row = a.get(r)?;
        let s: f64 = row.iter().zip(&x).skip(r + 1).map(|(a, x)| a * x).sum();
        *x.get_mut(r)? = (b.get(r)? - s) / row.get(r)?;
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surveys_parse_and_fit() {
        let csv = "X,Y,Band,RSSI_dBm,Router\n1000,2000,5,-55.5,Hall\n\n3000,2000,2.4,-60,\n";
        let p = parse(csv, false).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].router.as_deref(), Some("Hall"));
        assert_eq!(p[1].router, None);
        assert!(parse("x,y,band\n1,2,5\n", false).is_err());
        assert!(parse("x,y,band,rssi\n1,2,5,40\n", false).is_err());
        assert!(parse("x,y,band,rssi\n1,nan,5,-40\n", false).is_err());
        assert!(
            parse(
                "[{\"at\": [1, 2], \"band\": \"7\", \"rssi_dbm\": -50}]",
                true
            )
            .is_err()
        );
        assert_eq!(
            parse(
                "{\"points\": [{\"at\": [1, 2], \"band\": 5, \"rssi_dbm\": -50}]}",
                true
            )
            .unwrap()
            .len(),
            1
        );
        // rest = −10·3·log d − 4 on 5 GHz and −10·3·log d + 2 on 2.4: n = 3, offsets −4 and 2.
        let terms: Vec<Term> = [0.2, 0.5, 0.8, 1.1]
            .iter()
            .flat_map(|x| {
                [
                    Term {
                        band: Band::G5,
                        log_d: *x,
                        rest: -30.0 * x - 4.0,
                        crossed: Vec::new(),
                    },
                    Term {
                        band: Band::G2_4,
                        log_d: *x + 0.1,
                        rest: -30.0 * (x + 0.1) + 2.0,
                        crossed: Vec::new(),
                    },
                ]
            })
            .collect();
        let (n, o) = fit(&terms, 2.2, 1.5, 6.0);
        assert!((n - 3.0).abs() < 1e-9, "{n}");
        assert!((o[1].unwrap() + 4.0).abs() < 1e-9 && (o[0].unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(o[2], None);
        let e = errors(&[1.0, -1.0, 3.0]).unwrap();
        assert!((e.mean - 1.0).abs() < 1e-12 && (e.rms - (11.0f64 / 3.0).sqrt()).abs() < 1e-12);
    }

    /// Walls from a known model come back: a material crossed by enough points, differently, is
    /// fitted with the exponent and offset; one crossed too little stays at its prior; values
    /// past a range are held at its end; the walls-off fit is the plain one.
    #[test]
    fn walls_fit() {
        let truth = |log_d: f64, concrete: usize, glass: usize| {
            -10.0 * 2.8 * log_d - 3.0 - 28.0 * concrete as f64 - 6.0 * glass as f64
        };
        let mut terms = Vec::new();
        for i in 0..40u32 {
            let log_d = 0.1 + 0.03 * f64::from(i);
            let concrete = (i % 3) as usize;
            let glass = usize::from(i == 7 || i == 20);
            let mut crossed = vec![("concrete", 23.0); concrete];
            crossed.extend(vec![("low-e-glass", 10.0); glass]);
            // `rest` carries the losses as the model has them now (concrete 23, glass 10).
            let rest = truth(log_d, concrete, glass) + 23.0 * concrete as f64 + 10.0 * glass as f64;
            terms.push(Term {
                band: Band::G5,
                log_d,
                rest,
                crossed,
            });
        }
        let cands = wall_candidates(&terms);
        let ok: Vec<_> = cands
            .iter()
            .filter(|c| c.1)
            .map(|c| (c.0.material, c.0.band, c.0.prior))
            .collect();
        assert_eq!(ok, vec![("concrete", Band::G5, 23.0)]);
        let glass = cands
            .iter()
            .find(|c| c.0.material == "low-e-glass")
            .unwrap();
        assert_eq!((glass.0.points, glass.1), (2, false));
        let wf = fit_walls(&terms, 2.2, &ok).unwrap();
        // The glass's 4 dB error lands in the fit (it isn't fitted), so allow a little.
        assert!((wf.exponent - 2.8).abs() < 0.1, "{wf:?}");
        let c = &wf.losses[0];
        assert!(
            (c.value - 28.0).abs() < 1.0 && c.status == Status::Fitted,
            "{wf:?}"
        );
        // A wall every point crosses alike is the offset's: not fitted.
        let same: Vec<Term> = terms
            .iter()
            .map(|t| Term {
                crossed: vec![("drywall", 3.0)],
                ..t.clone()
            })
            .collect();
        assert!(wall_candidates(&same).iter().all(|c| !c.1));
        // Pulled past 60 dB, a loss stops there and the rest are fitted again.
        let big: Vec<Term> = terms
            .iter()
            .map(|t| {
                let n = t.crossed.iter().filter(|c| c.0 == "concrete").count() as f64;
                Term {
                    rest: t.rest - 60.0 * n,
                    ..t.clone()
                }
            })
            .collect();
        let wf = fit_walls(&big, 2.2, &ok).unwrap();
        assert_eq!(
            (wf.losses[0].value, wf.losses[0].status),
            (60.0, Status::Clamped)
        );
        // Nothing to fit but the exponent and offsets: the same as the plain fit.
        let plain: Vec<Term> = terms
            .iter()
            .map(|t| Term {
                crossed: Vec::new(),
                ..t.clone()
            })
            .collect();
        let joint = fit_walls(&plain, 2.2, &[]).unwrap();
        let (n, o) = fit(&plain, 2.2, 1.5, 6.0);
        assert!(
            (joint.exponent - n).abs() < 1e-9
                && (joint.offsets[1].unwrap() - o[1].unwrap()).abs() < 1e-9
        );
        // Held out: exact data predicts itself.
        let errs = held_out(
            &plain[..20]
                .iter()
                .map(|t| Term {
                    rest: -25.0 * t.log_d - 1.0,
                    ..t.clone()
                })
                .collect::<Vec<_>>(),
            2.2,
            &[],
            5,
        );
        assert_eq!(errs.len(), 20);
        assert!(errs.iter().all(|e| e.1.abs() < 1e-9), "{errs:?}");
    }
}
