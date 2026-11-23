//! Measured signal: points read from a file (or typed), compared with the prediction, and a
//! calibration that fits the model to them.
//!
//! The calibration fits one path-loss exponent for all bands and one offset (dB) per band, by
//! least squares: with each point's prediction split into its distance term and the rest, the
//! measured-minus-rest residual is a straight line in log10(distance) per band, sharing a slope.

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
pub struct Term {
    pub band: Band,
    pub log_d: f64,
    pub rest: f64,
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
                    },
                    Term {
                        band: Band::G2_4,
                        log_d: *x + 0.1,
                        rest: -30.0 * (x + 0.1) + 2.0,
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
}
