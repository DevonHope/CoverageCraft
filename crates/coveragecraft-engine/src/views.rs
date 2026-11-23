//! The maps: each is worked out from every router's signal at every cell (computed once), so
//! switching between them costs nothing and the probe and the maps always agree.

use crate::materials::Band;
use crate::model::{Cell, SCALE};
use crate::radio::{Channel, HEARD_DBM, View, default_width, link_rate};

/// One row of a map's key: what a colour means and the share (0–100) of the area it describes
/// (the floor, or for the leakage map the area outside it).
#[derive(Clone, Debug, PartialEq)]
pub struct LegendRow {
    pub label: String,
    pub colour: [u8; 3],
    pub share: f64,
}

/// What each cell gets from the routers, derived from their signals there.
#[derive(Clone, Debug, Default)]
pub struct Derived {
    /// The strongest and second-strongest signal (dBm); NaN where there is none.
    pub best: Vec<f32>,
    pub second: Vec<f32>,
    /// The router giving the strongest signal; -1 where none does.
    pub serving: Vec<i16>,
    /// How many other routers on a channel overlapping the serving one's are heard at
    /// [`HEARD_DBM`] or more; -1 where the serving router has no channel set.
    pub overlap: Vec<i16>,
    /// The link rate the strongest signal allows (Mbit/s); 0 below the lowest rate, NaN where
    /// there is no signal.
    pub rate: Vec<f32>,
}

/// Derive what each cell gets from `signals` (`cell * routers + router`, dBm).
pub fn derive(
    signals: &[f32],
    routers: usize,
    channels: &[Option<Channel>],
    band: Band,
    streams: u8,
) -> Derived {
    let cells = signals.len().checked_div(routers).unwrap_or(0);
    let mut d = Derived {
        best: vec![f32::NAN; cells],
        second: vec![f32::NAN; cells],
        serving: vec![-1; cells],
        overlap: vec![-1; cells],
        rate: vec![f32::NAN; cells],
    };
    let mut heard: Vec<(f32, usize)> = Vec::with_capacity(routers);
    for i in 0..cells {
        heard.clear();
        if let Some(row) = signals.get(i * routers..(i + 1) * routers) {
            heard.extend(
                row.iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_finite())
                    .map(|(k, v)| (*v, k)),
            );
        }
        heard.sort_by(|a, b| b.0.total_cmp(&a.0));
        let Some(&(best, k)) = heard.first() else {
            continue;
        };
        let serving_channel = channels.get(k).copied().flatten();
        let width = serving_channel.map_or(default_width(band), |c| c.width);
        let overlap = serving_channel.map_or(-1, |own| {
            heard
                .iter()
                .skip(1)
                .filter(|(v, j)| {
                    f64::from(*v) >= HEARD_DBM
                        && channels
                            .get(*j)
                            .copied()
                            .flatten()
                            .is_some_and(|c| c.overlaps(own, band))
                })
                .count()
                .min(i16::MAX as usize) as i16
        });
        let rate = link_rate(f64::from(best), width, streams).map_or(0.0, |(_, r)| r as f32);
        for (slot, v) in [
            (d.best.get_mut(i), best),
            (d.second.get_mut(i), heard.get(1).map_or(f32::NAN, |h| h.0)),
            (d.rate.get_mut(i), rate),
        ] {
            if let Some(slot) = slot {
                *slot = v;
            }
        }
        if let Some(s) = d.serving.get_mut(i) {
            *s = i16::try_from(k).unwrap_or(-1);
        }
        if let Some(o) = d.overlap.get_mut(i) {
            *o = overlap;
        }
    }
    d
}

/// Colours for the serving-router map, one a router in turn (data colours, not the theme's).
pub const ROUTER_COLOURS: [[u8; 3]; 8] = [
    [66, 133, 214],
    [230, 140, 40],
    [60, 170, 100],
    [200, 70, 160],
    [150, 110, 220],
    [40, 170, 190],
    [210, 180, 40],
    [170, 90, 60],
];

const GREY: [u8; 3] = [120, 120, 130];
const WALL: [u8; 4] = [40, 46, 52, 235];

/// The link-rate map's steps (Mbit/s), coloured like the signal scale's.
const RATE_STEPS: [f64; 7] = [1000.0, 600.0, 300.0, 150.0, 50.0, 10.0, 0.05];

fn scale_colour(i: usize) -> [u8; 3] {
    SCALE.get(i).map_or(GREY, |(_, c)| *c)
}

fn dbm(x: f64) -> String {
    // A true minus sign, as the window's other figures use.
    format!("{x:.0}").replace('-', "−") + " dBm"
}

/// A cumulative key over `values` of the cells `counted` selects: for each threshold the share
/// at it or above, and a last row for the rest.
fn stepped(
    values: &[f32],
    counted: &dyn Fn(usize) -> bool,
    steps: &[f64],
    label: &dyn Fn(f64) -> String,
    rest: &str,
) -> Vec<LegendRow> {
    let picked: Vec<f64> = (0..values.len())
        .filter(|i| counted(*i))
        .map(|i| values.get(i).map_or(f64::NAN, |v| f64::from(*v)))
        .collect();
    let n = picked.len().max(1) as f64;
    let share = |keep: &dyn Fn(f64) -> bool| {
        (picked.iter().filter(|v| keep(**v)).count() as f64 / n * 1000.0).round() / 10.0
    };
    let mut rows: Vec<LegendRow> = steps
        .iter()
        .enumerate()
        .map(|(i, t)| LegendRow {
            label: label(*t),
            colour: scale_colour(i),
            share: share(&|v| v >= *t),
        })
        .collect();
    let last = steps.last().copied().unwrap_or(f64::NEG_INFINITY);
    rows.push(LegendRow {
        label: rest.to_string(),
        colour: GREY,
        share: share(&|v| v.is_nan() || v < last),
    });
    rows
}

fn step_index(steps: &[f64], v: f64) -> usize {
    steps.iter().position(|t| v >= *t).unwrap_or(steps.len())
}

/// The picture of `view` (straight RGBA8, a pixel a cell, first row lowest) and its key.
pub fn picture(
    view: View,
    d: &Derived,
    cells: &[Cell],
    names: &[String],
) -> (Vec<u8>, Vec<LegendRow>) {
    let floor = |i: usize| cells.get(i) == Some(&Cell::Floor);
    let outside = |i: usize| {
        cells.get(i) == Some(&Cell::Outside) && d.best.get(i).is_some_and(|v| v.is_finite())
    };
    let signal_steps: Vec<f64> = SCALE
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| t.is_finite())
        .collect();
    let mut out = vec![0u8; cells.len() * 4];
    let colour_of = |i: usize| -> Option<([u8; 3], u8)> {
        let f = |v: Option<&f32>| v.map_or(f64::NAN, |v| f64::from(*v));
        match view {
            View::Signal | View::Second | View::Leakage => {
                let v = f(if view == View::Second {
                    d.second.get(i)
                } else {
                    d.best.get(i)
                });
                if view == View::Leakage && floor(i) {
                    return Some(([90, 96, 104], 120));
                }
                let shown = if view == View::Leakage {
                    outside(i)
                } else {
                    floor(i)
                };
                shown.then(|| (scale_colour(step_index(&signal_steps, v)), 210))
            }
            View::Rate => floor(i).then(|| {
                let v = f(d.rate.get(i));
                (scale_colour(step_index(&RATE_STEPS, v)), 210)
            }),
            View::Overlap => floor(i).then(|| {
                let c = match d.overlap.get(i).copied().unwrap_or(-1) {
                    0 => scale_colour(0),
                    1 => scale_colour(3),
                    2 => scale_colour(5),
                    x if x >= 3 => scale_colour(6),
                    _ => GREY,
                };
                (c, 210)
            }),
            View::Serving => floor(i).then(|| {
                let k = d.serving.get(i).copied().unwrap_or(-1);
                let weak = f(d.best.get(i)) < HEARD_DBM || k < 0;
                let c = if weak {
                    GREY
                } else {
                    ROUTER_COLOURS
                        .get(usize::try_from(k).unwrap_or(0) % ROUTER_COLOURS.len())
                        .copied()
                        .unwrap_or(GREY)
                };
                (c, 210)
            }),
        }
    };
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let rgba = if cells.get(i) == Some(&Cell::Wall) {
            WALL
        } else if let Some(([r, g, b], a)) = colour_of(i) {
            [r, g, b, a]
        } else {
            [0, 0, 0, 0]
        };
        px.copy_from_slice(&rgba);
    }
    let legend = match view {
        View::Signal => stepped(
            &d.best,
            &floor,
            &signal_steps,
            &|t| format!("≥ {}", dbm(t)),
            &format!("< {}", dbm(-80.0)),
        ),
        View::Second => stepped(
            &d.second,
            &floor,
            &signal_steps,
            &|t| format!("≥ {}", dbm(t)),
            "Weaker, or no second router",
        ),
        View::Leakage => stepped(
            &d.best,
            &outside,
            &signal_steps,
            &|t| format!("≥ {}", dbm(t)),
            &format!("< {}", dbm(-80.0)),
        ),
        View::Rate => stepped(
            &d.rate,
            &floor,
            &RATE_STEPS,
            &|t| {
                if t < 1.0 {
                    "Any link".to_string()
                } else {
                    format!("≥ {t:.0} Mbit/s")
                }
            },
            "No link",
        ),
        View::Overlap => {
            let n = (0..cells.len()).filter(|i| floor(*i)).count().max(1) as f64;
            let share = |keep: &dyn Fn(i16) -> bool| {
                ((0..cells.len())
                    .filter(|i| floor(*i) && keep(d.overlap.get(*i).copied().unwrap_or(-1)))
                    .count() as f64
                    / n
                    * 1000.0)
                    .round()
                    / 10.0
            };
            vec![
                LegendRow {
                    label: "None".into(),
                    colour: scale_colour(0),
                    share: share(&|x| x == 0),
                },
                LegendRow {
                    label: "1".into(),
                    colour: scale_colour(3),
                    share: share(&|x| x == 1),
                },
                LegendRow {
                    label: "2".into(),
                    colour: scale_colour(5),
                    share: share(&|x| x == 2),
                },
                LegendRow {
                    label: "3 or more".into(),
                    colour: scale_colour(6),
                    share: share(&|x| x >= 3),
                },
                LegendRow {
                    label: "No channel set".into(),
                    colour: GREY,
                    share: share(&|x| x < 0),
                },
            ]
        }
        View::Serving => {
            let n = (0..cells.len()).filter(|i| floor(*i)).count().max(1) as f64;
            let usable = |i: usize| d.best.get(i).is_some_and(|v| f64::from(*v) >= HEARD_DBM);
            let mut rows: Vec<LegendRow> = names
                .iter()
                .enumerate()
                .map(|(k, name)| LegendRow {
                    label: name.clone(),
                    colour: ROUTER_COLOURS
                        .get(k % ROUTER_COLOURS.len())
                        .copied()
                        .unwrap_or(GREY),
                    share: ((0..cells.len())
                        .filter(|i| {
                            floor(*i)
                                && usable(*i)
                                && d.serving.get(*i).copied() == i16::try_from(k).ok()
                        })
                        .count() as f64
                        / n
                        * 1000.0)
                        .round()
                        / 10.0,
                })
                .collect();
            rows.push(LegendRow {
                label: "No usable signal".into(),
                colour: GREY,
                share: ((0..cells.len())
                    .filter(|i| floor(*i) && !usable(*i))
                    .count() as f64
                    / n
                    * 1000.0)
                    .round()
                    / 10.0,
            });
            rows
        }
    };
    (out, legend)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two cells, three routers: router 0 and 2 on channel 36, router 1 on 52.
    #[test]
    fn each_map_comes_from_the_same_signals() {
        let ch = |n| Channel::new(Band::G5, n, 20);
        let channels = [ch(36), ch(52), ch(36)];
        let signals = [
            -50.0, -60.0,
            -70.0, // cell 0: router 0 serves; router 2 shares its channel, heard
            -90.0, -40.0, -85.0, // cell 1: router 1 serves; nobody else on 52
        ];
        let d = derive(&signals, 3, &channels, Band::G5, 2);
        assert_eq!(d.best, vec![-50.0, -40.0]);
        assert_eq!(d.second, vec![-60.0, -85.0]);
        assert_eq!(d.serving, vec![0, 1]);
        assert_eq!(d.overlap, vec![1, 0]);
        // −50 dBm at 20 MHz is MCS 9: 234 × 6⅔ × 2 ÷ 13.6 = 229.4 Mbit/s.
        assert_eq!(d.rate.first().copied(), Some(229.4));
        let cells = [Cell::Floor, Cell::Floor];
        let names = ["A".to_string(), "B".to_string(), "C".to_string()];
        let (rgba, legend) = picture(View::Serving, &d, &cells, &names);
        assert_eq!(rgba.len(), 8);
        assert_eq!(
            legend.iter().map(|r| r.share).collect::<Vec<_>>(),
            vec![50.0, 50.0, 0.0, 0.0]
        );
        let (_, overlap) = picture(View::Overlap, &d, &cells, &names);
        assert_eq!(overlap.first().map(|r| r.share), Some(50.0));
        // Without channels, interference isn't guessed.
        let none = derive(&signals, 3, &[None, None, None], Band::G5, 2);
        assert_eq!(none.overlap, vec![-1, -1]);
    }
}
