//! Floors: a building's plans stacked, each with where it starts (its elevation), where its plan
//! sits in the building (an offset, so plans drawn at different origins line up), the slab under
//! it and where its model is cut. Routers and measured points belong to a floor.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::materials::{Material, rf_preset};
use crate::plan::FloorPlan;

/// The most floors a project has.
pub const MAX_FLOORS: usize = 32;

/// A floor of the building.
#[derive(Clone, Debug)]
pub struct Floor {
    pub name: String,
    /// Where it starts above the building's zero (mm).
    pub elevation: f64,
    /// Where its plan's origin is in the building (mm).
    pub offset: [f64; 2],
    /// The slab under it: what a signal loses passing it. Concrete unless set.
    pub slab: Material,
    /// Where its model is cut, in the model file's own coordinates (mm).
    pub cut: f64,
    /// Its plan, while another floor is the one shown (the shown one's is the engine's).
    pub(crate) plan: Option<Arc<FloorPlan>>,
}

impl Floor {
    pub fn new(name: impl Into<String>, elevation: f64, cut: f64) -> Floor {
        Floor {
            name: name.into(),
            elevation,
            offset: [0.0, 0.0],
            slab: Material::Preset("concrete"),
            cut,
            plan: None,
        }
    }

    /// The same floor: its settings equal and the same plan (by identity or content).
    pub fn same(&self, o: &Floor) -> bool {
        let plans = match (&self.plan, &o.plan) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            (None, None) => true,
            _ => false,
        };
        plans
            && self.name == o.name
            && self.elevation == o.elevation
            && self.offset == o.offset
            && self.slab == o.slab
            && self.cut == o.cut
    }

    pub fn to_json(&self, index: usize, active: bool, plan_path: Option<&str>) -> Value {
        json!({
            "index": index,
            "name": self.name,
            "elevation": self.elevation,
            "offset": self.offset,
            "slab": self.slab.to_json(),
            "cut": self.cut,
            "plan": plan_path,
            "active": active,
        })
    }
}

/// What a slab of `m` loses (dB at 2.4, 5, 6 GHz): a preset's or custom values; nothing for
/// "not a wall" (an opening, an atrium).
pub fn slab_db(m: &Material) -> [f64; 3] {
    match m {
        Material::Preset(id) => rf_preset(id).map_or([0.0; 3], |p| p.loss.0),
        Material::Custom(l) => l.0,
        Material::Ignore | Material::Auto => [0.0; 3],
    }
}
