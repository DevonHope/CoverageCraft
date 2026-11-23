//! Radio-frequency properties of materials: the loss of a wall each time a path crosses it.
//!
//! A material may say how much a Wi-Fi signal loses each time a straight path crosses a piece of
//! it: one loss in dB for each band (2.4, 5 and 6 GHz). The values are per crossing, not per
//! millimetre, the way multi-wall indoor models count walls. The presets are typical published
//! averages for building materials, rounded; real walls vary by several dB.

use serde::{Deserialize, Serialize};

/// The Wi-Fi bands the analysis knows, in the order [`RfLoss`] stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    /// 2.4 GHz.
    G2_4,
    /// 5 GHz.
    G5,
    /// 6 GHz.
    G6,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::G2_4, Band::G5, Band::G6];

    /// The band's name in parameters and replies: `"2.4"`, `"5"` or `"6"`.
    pub fn name(self) -> &'static str {
        match self {
            Band::G2_4 => "2.4",
            Band::G5 => "5",
            Band::G6 => "6",
        }
    }

    /// The band its name gives (`"2.4"`, `"5"`, `"6"`, with or without a `GHz` suffix).
    pub fn parse(s: &str) -> Option<Band> {
        let s = s.trim();
        let s = s
            .strip_suffix("GHz")
            .or_else(|| s.strip_suffix("ghz"))
            .unwrap_or(s)
            .trim();
        match s {
            "2.4" | "2" => Some(Band::G2_4),
            "5" => Some(Band::G5),
            "6" => Some(Band::G6),
            _ => None,
        }
    }

    /// The frequency the path loss is worked out at, in MHz: the middle of each band's channels.
    pub fn mhz(self) -> f64 {
        match self {
            Band::G2_4 => 2437.0,
            Band::G5 => 5500.0,
            Band::G6 => 6525.0,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Band::G2_4 => 0,
            Band::G5 => 1,
            Band::G6 => 2,
        }
    }
}

/// The largest loss one crossing may have, in dB: more than any building material, small enough
/// that sums stay finite.
pub const MAX_LOSS_DB: f64 = 100.0;

/// A material's loss each time a straight path crosses a piece of it, in dB, at 2.4, 5 and 6 GHz.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RfLoss(pub [f64; 3]);

impl RfLoss {
    /// The loss in `band`.
    pub fn at(&self, band: Band) -> f64 {
        self.0.get(band.index()).copied().unwrap_or(0.0)
    }

    /// Whether every loss is a number from 0 to [`MAX_LOSS_DB`].
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .all(|l| l.is_finite() && (0.0..=MAX_LOSS_DB).contains(l))
    }
}

/// A file's `rf_loss_db`: losses out of range, or anything else unreadable, read as none, so a
/// damaged file opens and the analysis guesses those walls instead.
pub fn valid_loss<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<RfLoss>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<RfLoss>(value)
        .ok()
        .filter(RfLoss::is_valid))
}

/// A named set of losses `material.create` and `material.edit` take by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RfPreset {
    /// The name parameters use, lower case with dashes.
    pub id: &'static str,
    pub loss: RfLoss,
}

/// The built-in RF presets.
pub const RF_PRESETS: &[RfPreset] = &[
    RfPreset {
        id: "concrete",
        loss: RfLoss([12.0, 20.0, 24.0]),
    },
    RfPreset {
        id: "drywall",
        loss: RfLoss([3.0, 4.0, 5.0]),
    },
    RfPreset {
        id: "interior-glass",
        loss: RfLoss([2.0, 3.0, 3.0]),
    },
    RfPreset {
        id: "low-e-glass",
        loss: RfLoss([10.0, 15.0, 18.0]),
    },
    RfPreset {
        id: "metal",
        loss: RfLoss([15.0, 20.0, 22.0]),
    },
    RfPreset {
        id: "none",
        loss: RfLoss([0.0, 0.0, 0.0]),
    },
    RfPreset {
        id: "wood-door",
        loss: RfLoss([3.0, 4.0, 5.0]),
    },
];

/// The preset `id` names (letter case, spaces and underscores don't matter).
pub fn rf_preset(id: &str) -> Option<&'static RfPreset> {
    let key: String = id
        .trim()
        .chars()
        .map(|c| {
            if c == '_' || c == ' ' {
                '-'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    RF_PRESETS.iter().find(|p| p.id == key)
}

/// The preset whose losses are exactly `loss`, if one is.
/// Calibrated losses in place of presets' own, per band (dB), by preset id: what fitting the
/// model to measured points found for the walls of this building.
pub type LossOverrides = std::collections::BTreeMap<String, [Option<f64>; 3]>;

/// A preset's loss, with any calibrated value in its place.
pub fn preset_loss(id: &str, overrides: &LossOverrides) -> RfLoss {
    let mut loss = rf_preset(id).map(|p| p.loss).unwrap_or(RfLoss([0.0; 3]));
    if let Some(o) = overrides.get(id) {
        for (slot, v) in loss.0.iter_mut().zip(o) {
            if let Some(v) = v {
                *slot = *v;
            }
        }
    }
    loss
}

/// The preset a wall's material name stands for (`concrete`, `concrete (guessed)`); none for a
/// custom loss.
pub fn preset_key(name: &str) -> Option<&'static str> {
    let base = name.strip_suffix(" (guessed)").unwrap_or(name);
    RF_PRESETS.iter().find(|p| p.id == base).map(|p| p.id)
}

pub fn rf_preset_of(loss: &RfLoss) -> Option<&'static RfPreset> {
    RF_PRESETS.iter().find(|p| p.loss == *loss)
}

/// What a layer or an object of a plan is made of, as far as the signal goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Material {
    /// Guessed where each path crosses it: drywall when thinner than [`CONCRETE_FROM_MM`], else
    /// concrete.
    Auto,
    /// Left out: not a wall (furniture, notes, dimensions).
    Ignore,
    /// One of [`RF_PRESETS`], by id.
    Preset(&'static str),
    /// Losses typed by the user.
    Custom(RfLoss),
}

impl Material {
    /// The id parameters and project files use: `auto`, `ignore`, a preset id, or `custom`.
    pub fn id(&self) -> &str {
        match self {
            Material::Auto => "auto",
            Material::Ignore => "ignore",
            Material::Preset(id) => id,
            Material::Custom(_) => "custom",
        }
    }

    /// The material `id` names: `auto`, `ignore` or a preset (loose spelling).
    pub fn parse(id: &str) -> Option<Material> {
        match id.trim().to_ascii_lowercase().as_str() {
            "auto" | "guess" => Some(Material::Auto),
            "ignore" | "none-ignore" | "skip" => Some(Material::Ignore),
            other => rf_preset(other).map(|p| Material::Preset(p.id)),
        }
    }

    /// The material as JSON: its id, or the losses by band for a custom one.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Material::Custom(l) => {
                serde_json::json!({"2.4": l.at(Band::G2_4), "5": l.at(Band::G5), "6": l.at(Band::G6)})
            }
            other => serde_json::Value::String(other.id().to_string()),
        }
    }

    /// The material `v` gives: an id, or losses by band (`{"2.4", "5", "6"}`, 0–100 dB).
    pub fn from_json(v: &serde_json::Value) -> Option<Material> {
        match v {
            serde_json::Value::String(s) => Material::parse(s),
            serde_json::Value::Object(m) => {
                let mut loss = [0.0; 3];
                for band in Band::ALL {
                    let db = m.get(band.name()).and_then(serde_json::Value::as_f64)?;
                    *loss.get_mut(band.index())? = db;
                }
                let loss = RfLoss(loss);
                (m.len() == 3 && loss.is_valid()).then_some(Material::Custom(loss))
            }
            _ => None,
        }
    }
}

/// The material a layer or object probably is, from its name (and its material name in a mesh
/// file): the names architects and modellers commonly use. Anything unmatched is [`Material::Auto`].
///
/// Names are split into words; a keyword matches a word that starts with it ("glaz" matches
/// "glazing"), except keywords of three letters or fewer, which match whole words ("bed" matches
/// "bed" and "beds", not "bedroom").
pub fn guess_material(names: &[&str]) -> Material {
    let text = names.join(" ").to_ascii_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |keys: &[&str]| {
        keys.iter().any(|k| {
            words.iter().any(|w| {
                if k.len() <= 3 {
                    *w == *k || w.strip_suffix('s') == Some(*k)
                } else {
                    w.starts_with(k)
                }
            })
        })
    };
    let window = has(&["window", "glaz", "glass", "pane", "slider"]);
    if has(&[
        "furn", "sofa", "couch", "chair", "table", "bed", "desk", "plant", "cushion", "lamp",
        "rug", "stool", "bench", "anno", "dim", "dims", "text", "note", "title", "grid", "symb",
        "pillow", "decor", "pot", "faucet", "valve", "vanity", "towel", "shelf",
    ]) {
        Material::Ignore
    } else if (window && has(&["post", "stile", "frame", "mullion", "handle", "rail"]))
        || has(&[
            "fridge",
            "refrig",
            "freezer",
            "oven",
            "dishwasher",
            "washer",
            "dryer",
            "tv",
            "television",
            "metal",
            "steel",
            "alum",
            "mullion",
            "appliance",
            "elevator",
            "duct",
            "railing",
        ])
    {
        Material::Preset("metal")
    } else if has(&["low-e", "lowe"]) || text.contains("low-e") {
        Material::Preset("low-e-glass")
    } else if window || has(&["mirror", "screen"]) {
        Material::Preset("interior-glass")
    } else if has(&["door"]) {
        Material::Preset("wood-door")
    } else if has(&["conc", "column", "shaft", "core", "masonry", "brick"]) {
        Material::Preset("concrete")
    } else if has(&["drywall", "gyp", "partition", "stud"]) {
        Material::Preset("drywall")
    } else {
        Material::Auto
    }
}

/// A wall at least this thick (mm) without an RF material counts as concrete, a thinner one as
/// drywall ([`heuristic_loss`]). Between a 4⅞″ steel-stud partition (124 mm) and a 6″ concrete
/// wall (152 mm).
pub const CONCRETE_FROM_MM: f64 = 140.0;

/// What a wall `thickness_mm` thick and without an RF material loses: drywall below
/// [`CONCRETE_FROM_MM`], concrete from there on.
pub fn heuristic_loss(thickness_mm: f64) -> (&'static str, RfLoss) {
    let id = if thickness_mm >= CONCRETE_FROM_MM {
        "concrete"
    } else {
        "drywall"
    };
    (
        id,
        rf_preset(id).map(|p| p.loss).unwrap_or(RfLoss([0.0; 3])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_sorted_valid_and_found_by_loose_names() {
        assert!(RF_PRESETS.windows(2).all(|w| w[0].id < w[1].id));
        assert!(RF_PRESETS.iter().all(|p| p.loss.is_valid()));
        assert_eq!(rf_preset("Low_E glass").map(|p| p.id), Some("low-e-glass"));
        assert_eq!(
            rf_preset("concrete").map(|p| p.loss.at(Band::G6)),
            Some(24.0)
        );
        assert!(rf_preset("brick").is_none());
    }

    #[test]
    fn thickness_decides_drywall_or_concrete() {
        assert_eq!(heuristic_loss(92.0).0, "drywall");
        assert_eq!(heuristic_loss(139.9).0, "drywall");
        assert_eq!(heuristic_loss(140.0).0, "concrete");
        assert_eq!(heuristic_loss(203.0).1, RfLoss([12.0, 20.0, 24.0]));
    }

    #[test]
    fn names_suggest_materials() {
        assert_eq!(
            guess_material(&["A-GLAZ"]),
            Material::Preset("interior-glass")
        );
        assert_eq!(guess_material(&["A-FURN"]), Material::Ignore);
        assert_eq!(guess_material(&["Fridge door"]), Material::Preset("metal"));
        assert_eq!(guess_material(&["A-WALL"]), Material::Auto);
        // Whole words for short keywords: a bedroom's door isn't a bed.
        assert_eq!(
            guess_material(&["Bedroom door"]),
            Material::Preset("wood-door")
        );
        assert_eq!(
            guess_material(&["Bedroom glass wall"]),
            Material::Preset("interior-glass")
        );
        // A window's posts are its metal frame.
        assert_eq!(
            guess_material(&["Window frame post"]),
            Material::Preset("metal")
        );
        assert_eq!(guess_material(&["TV Screen"]), Material::Preset("metal"));
        assert_eq!(
            Material::from_json(&serde_json::json!({"2.4": 1, "5": 2, "6": 3})),
            Some(Material::Custom(RfLoss([1.0, 2.0, 3.0])))
        );
        assert_eq!(Material::from_json(&serde_json::json!({"2.4": 1})), None);
    }

    #[test]
    fn bands_parse_their_names() {
        for b in Band::ALL {
            assert_eq!(Band::parse(b.name()), Some(b));
        }
        assert_eq!(Band::parse("5 GHz"), Some(Band::G5));
        assert_eq!(Band::parse("60"), None);
        assert!(!RfLoss([1.0, f64::NAN, 2.0]).is_valid());
        assert!(!RfLoss([1.0, -1.0, 2.0]).is_valid());
    }

    #[test]
    fn a_damaged_file_loss_reads_as_none() {
        #[derive(Deserialize)]
        struct M {
            #[serde(default, deserialize_with = "valid_loss")]
            rf: Option<RfLoss>,
        }
        let read = |j: &str| serde_json::from_str::<M>(j).map(|m| m.rf).ok();
        assert_eq!(
            read(r#"{"rf":[1,2,3]}"#),
            Some(Some(RfLoss([1.0, 2.0, 3.0])))
        );
        assert_eq!(read(r#"{"rf":[1,2,300]}"#), Some(None));
        assert_eq!(read(r#"{"rf":"x"}"#), Some(None));
        assert_eq!(read(r#"{}"#), Some(None));
    }
}
