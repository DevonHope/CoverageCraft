//! Radio channels and what they mean at a point: which channels share spectrum, which routers a
//! device hears on its own channel, and the link rate a signal level allows.
//!
//! The numbers come from the IEEE 802.11 standard (public channel plans, the receiver minimum
//! input sensitivity table, the OFDM symbol structure), not from any other planner.

use serde_json::{Value, json};

use crate::materials::Band;

/// A channel: its primary 20 MHz channel number and its width (MHz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub number: u16,
    pub width: u16,
}

/// The widths a band allows (MHz). 2.4 GHz is planned at 20 MHz only: wider channels there
/// overlap every other network.
pub fn widths(band: Band) -> &'static [u16] {
    match band {
        Band::G2_4 => &[20],
        Band::G5 | Band::G6 => &[20, 40, 80, 160],
    }
}

/// The width a band's channel has when none is given (MHz).
pub fn default_width(band: Band) -> u16 {
    match band {
        Band::G2_4 => 20,
        Band::G5 | Band::G6 => 80,
    }
}

/// The 20 MHz channel numbers of the 5 GHz band (UNII-1 to UNII-4).
const CH5: [u16; 28] = [
    36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165, 169, 173, 177,
];

/// The 20 MHz channels a channel of `width` with primary `number` covers, as the band's channel
/// plan bonds them; `None` when that isn't a channel the band has.
fn block(band: Band, number: u16, width: u16) -> Option<(u16, u16)> {
    if !widths(band).contains(&width) {
        return None;
    }
    let span = width / 5; // channel numbers per block (4 per 20 MHz)
    match band {
        Band::G2_4 => (1..=13).contains(&number).then_some((number, number)),
        Band::G5 => {
            if !CH5.contains(&number) {
                return None;
            }
            // Blocks are aligned: 36, 52, 100, 116, 132, 149, 165 start the 80 MHz blocks, and
            // 36, 100, 149 the 160 MHz ones; 40 MHz blocks pair neighbours from each start.
            let starts: &[u16] = match width {
                20 => return Some((number, number)),
                40 => &[
                    36, 44, 52, 60, 100, 108, 116, 124, 132, 140, 149, 157, 165, 173,
                ],
                80 => &[36, 52, 100, 116, 132, 149, 165],
                _ => &[36, 100, 149],
            };
            starts
                .iter()
                .find(|s| (**s..**s + span).contains(&number))
                .map(|s| (*s, *s + span - 4))
        }
        Band::G6 => {
            // 6 GHz: channels 1, 5, 9 … 233; blocks start at 1 and every `span` numbers.
            if !(1..=233).contains(&number) || number % 4 != 1 {
                return None;
            }
            let start = (number - 1) / span * span + 1;
            let end = start + span - 4;
            (end <= 233).then_some((start, end))
        }
    }
}

/// Every 20 MHz channel number `band` has.
pub fn numbers(band: Band) -> Vec<u16> {
    match band {
        Band::G2_4 => (1..=13).collect(),
        Band::G5 => CH5.to_vec(),
        Band::G6 => (1..=233).step_by(4).collect(),
    }
}

impl Channel {
    /// The 20 MHz channels it covers.
    pub fn covers(self, band: Band) -> Vec<u16> {
        match block(band, self.number, self.width) {
            Some((lo, hi)) => numbers(band)
                .into_iter()
                .filter(|n| (lo..=hi).contains(n))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The channel, when `band` has it.
    pub fn new(band: Band, number: u16, width: u16) -> Option<Channel> {
        block(band, number, width).map(|_| Channel { number, width })
    }

    /// The spectrum it occupies, in MHz.
    pub fn spectrum(self, band: Band) -> Option<(f64, f64)> {
        let (lo, hi) = block(band, self.number, self.width)?;
        let centre = |n: u16| match band {
            Band::G2_4 => 2407.0 + 5.0 * f64::from(n),
            Band::G5 => 5000.0 + 5.0 * f64::from(n),
            Band::G6 => 5950.0 + 5.0 * f64::from(n),
        };
        // A 2.4 GHz channel is counted 22 MHz wide (its DSSS mask), so channels fewer than five
        // apart overlap, as the 2.4 GHz channel plan intends: 1, 6 and 11 don't.
        let half = if band == Band::G2_4 { 11.0 } else { 10.0 };
        Some((centre(lo) - half, centre(hi) + half))
    }

    /// Whether two channels share any spectrum.
    pub fn overlaps(self, other: Channel, band: Band) -> bool {
        match (self.spectrum(band), other.spectrum(band)) {
            (Some((a0, a1)), Some((b0, b1))) => a0 < b1 && b0 < a1,
            _ => false,
        }
    }

    pub fn to_json(self) -> Value {
        json!({"channel": self.number, "width": self.width})
    }

    /// A channel from `5`, `"36"` or `{"channel": 36, "width": 80}`; a bare number is 20 MHz.
    pub fn from_json(band: Band, v: &Value) -> Option<Channel> {
        let (number, width) = match v {
            Value::Number(n) => (n.as_u64()?, 20),
            Value::String(s) => (s.trim().parse().ok()?, 20),
            Value::Object(m) => (
                m.get("channel")?.as_u64()?,
                m.get("width")
                    .map_or(Some(u64::from(default_width(band))), Value::as_u64)?,
            ),
            _ => return None,
        };
        Channel::new(
            band,
            u16::try_from(number).ok()?,
            u16::try_from(width).ok()?,
        )
    }
}

/// The weakest signal (dBm) on a 20 MHz channel that makes a device defer: the 802.11 OFDM
/// PHY's clear-channel-assessment level for a valid signal. A router on an overlapping channel
/// heard at this level or above takes airtime from the serving one.
pub const HEARD_DBM: f64 = -82.0;

/// The receiver minimum input sensitivity at 20 MHz for MCS 0 to 9 (dBm), from the IEEE 802.11
/// standard's VHT table (802.11-2020, Table 21-25), used here for HE (802.11ax) as well; MCS 10
/// and 11 are left out until checked against the HE table. Each doubling of the width needs
/// 3 dB more.
const SENSITIVITY_20: [f64; 10] = [
    -82.0, -79.0, -77.0, -74.0, -70.0, -66.0, -65.0, -64.0, -59.0, -57.0,
];

/// Coded bits per data subcarrier for MCS 0 to 9: BPSK 1/2, QPSK 1/2, QPSK 3/4, 16-QAM 1/2,
/// 16-QAM 3/4, 64-QAM 2/3, 3/4, 5/6, 256-QAM 3/4, 5/6.
const BITS: [f64; 10] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 4.5, 5.0, 6.0, 20.0 / 3.0];

/// Data subcarriers of an HE (802.11ax) channel of each width.
fn data_subcarriers(width: u16) -> f64 {
    match width {
        20 => 234.0,
        40 => 468.0,
        80 => 980.0,
        _ => 1960.0,
    }
}

/// The best MCS a signal of `rssi` dBm on a channel `width` MHz wide allows, and its PHY rate
/// (Mbit/s) for `streams` spatial streams: data subcarriers × coded bits × streams ÷ the
/// 13.6 µs HE symbol (0.8 µs guard). `None` below MCS 0.
///
/// It is the rate with nothing else on the air: no interference, no other clients.
pub fn link_rate(rssi: f64, width: u16, streams: u8) -> Option<(u8, f64)> {
    let extra = 3.0 * (f64::from(width.max(20)) / 20.0).log2();
    let mcs = SENSITIVITY_20.iter().rposition(|s| rssi >= s + extra)?;
    let bits = BITS.get(mcs)?;
    let rate = data_subcarriers(width) * bits * f64::from(streams) / 13.6;
    Some((u8::try_from(mcs).ok()?, (rate * 10.0).round() / 10.0))
}

/// What the map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The strongest signal.
    Signal,
    /// The second-strongest signal: where a device has another router to move to.
    Second,
    /// Which router serves each point.
    Serving,
    /// How many other routers on an overlapping channel a device hears.
    Overlap,
    /// The link rate the strongest signal allows.
    Rate,
    /// Signal reaching past the floor's outline.
    Leakage,
}

impl View {
    pub const ALL: [View; 6] = [
        View::Signal,
        View::Second,
        View::Serving,
        View::Overlap,
        View::Rate,
        View::Leakage,
    ];

    pub fn id(self) -> &'static str {
        match self {
            View::Signal => "signal",
            View::Second => "second",
            View::Serving => "serving",
            View::Overlap => "overlap",
            View::Rate => "rate",
            View::Leakage => "leakage",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        View::ALL.into_iter().find(|v| v.id() == s.trim())
    }

    /// Its name in the window and in reports.
    pub fn label(self) -> &'static str {
        match self {
            View::Signal => "Signal",
            View::Second => "Second-strongest signal",
            View::Serving => "Serving router",
            View::Overlap => "Same-channel routers heard",
            View::Rate => "Estimated link rate",
            View::Leakage => "Signal outside the walls",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_plans() {
        let ch = |b, n, w| Channel::new(b, n, w);
        // 2.4 GHz: 1, 6 and 11 don't overlap; 1 and 5 do.
        let (one, five, six, eleven) = (
            ch(Band::G2_4, 1, 20).unwrap(),
            ch(Band::G2_4, 5, 20).unwrap(),
            ch(Band::G2_4, 6, 20).unwrap(),
            ch(Band::G2_4, 11, 20).unwrap(),
        );
        assert!(!one.overlaps(six, Band::G2_4) && !six.overlaps(eleven, Band::G2_4));
        assert!(one.overlaps(five, Band::G2_4) && five.overlaps(six, Band::G2_4));
        assert!(ch(Band::G2_4, 14, 20).is_none() && ch(Band::G2_4, 6, 40).is_none());
        // 5 GHz: 36/80 covers 36–48 and so overlaps 44/20, not 52/20.
        let wide = ch(Band::G5, 36, 80).unwrap();
        assert!(wide.overlaps(ch(Band::G5, 44, 20).unwrap(), Band::G5));
        assert!(!wide.overlaps(ch(Band::G5, 52, 20).unwrap(), Band::G5));
        assert_eq!(wide.spectrum(Band::G5), Some((5170.0, 5250.0)));
        // A primary anywhere in the block is the same block.
        assert!(
            ch(Band::G5, 48, 80)
                .unwrap()
                .overlaps(ch(Band::G5, 36, 20).unwrap(), Band::G5)
        );
        assert!(ch(Band::G5, 37, 20).is_none() && ch(Band::G5, 144, 160).is_none());
        assert!(ch(Band::G5, 165, 80).is_some() && ch(Band::G5, 181, 20).is_none());
        // 6 GHz: 1 + 4k; 37/160 covers 33–61.
        let six160 = ch(Band::G6, 37, 160).unwrap();
        assert_eq!(six160.spectrum(Band::G6), Some((6105.0, 6265.0)));
        assert!(ch(Band::G6, 3, 20).is_none() && ch(Band::G6, 229, 160).is_none());
        assert!(
            Channel::from_json(Band::G5, &json!({"channel": 149})).is_some_and(|c| c.width == 80)
        );
        assert!(Channel::from_json(Band::G5, &json!(36)).is_some_and(|c| c.width == 20));
        assert!(Channel::from_json(Band::G5, &json!({"channel": 36, "width": 30})).is_none());
    }

    #[test]
    fn link_rates_follow_the_sensitivity_table() {
        // 1 stream at 20 MHz, MCS 0: 234 × 0.5 ÷ 13.6 µs = 8.6 Mbit/s.
        assert_eq!(link_rate(-82.0, 20, 1), Some((0, 8.6)));
        assert_eq!(link_rate(-82.1, 20, 1), None);
        // MCS 9 at 80 MHz needs −57 − 6 = −51 dBm: 980 × 6⅔ × 2 ÷ 13.6 = 960.8 Mbit/s.
        assert_eq!(link_rate(-51.0, 80, 2), Some((9, 960.8)));
        assert_eq!(link_rate(-51.5, 80, 2).map(|r| r.0), Some(8));
        assert_eq!(View::parse("rate"), Some(View::Rate));
    }
}
