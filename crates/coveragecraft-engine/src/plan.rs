//! A floor plan: the shapes read from a drawing or a model, grouped by where they came from (a
//! layer of a DXF, an object of a mesh file), and the materials those sources are given.

use std::collections::BTreeMap;

use crate::materials::{
    CONCRETE_FROM_MM, LossOverrides, Material, RfLoss, guess_material, heuristic_loss, preset_loss,
};
use crate::model::{WallMap, WallRegion};

/// What a source is in its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A DXF layer.
    Layer,
    /// An object (body) of a mesh file.
    Object,
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Layer => "layer",
            SourceKind::Object => "object",
        }
    }
}

/// A layer or an object, with the material its shapes are made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub name: String,
    pub kind: SourceKind,
    /// Other names that say what it is (a mesh object's material name).
    pub hints: Vec<String>,
    pub material: Material,
}

/// A shape of the plan, in plan millimetres.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Closed rings, inside by the even-odd rule: an outline and its holes, or a wall's two faces.
    Area(Vec<Vec<[f64; 2]>>),
    /// A line drawn for a thin element (a pane, a door leaf): it becomes a strip as wide as its
    /// material's usual thickness ([`stroke_width_mm`]).
    Stroke(Vec<[f64; 2]>),
}

/// A shape and the source it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanShape {
    pub source: usize,
    pub shape: Shape,
}

/// A floor plan read from a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FloorPlan {
    /// The file it was read from.
    pub path: String,
    pub sources: Vec<Source>,
    pub shapes: Vec<PlanShape>,
    /// What was left out or approximated while reading.
    pub notes: Vec<String>,
    /// The viewpoints it offers: a model's standard and saved views, a drawing's named views.
    pub views: Vec<crate::viewpoint::PlanView>,
}

/// How wide a drawn line counts, by its material (mm): a pane of glass, a door leaf, a wall.
pub fn stroke_width_mm(material: &Material) -> f64 {
    match material {
        Material::Preset("interior-glass" | "low-e-glass") => 10.0,
        Material::Preset("wood-door") => 40.0,
        Material::Preset("metal") => 20.0,
        Material::Preset("concrete") => 200.0,
        _ => 100.0,
    }
}

impl FloorPlan {
    /// Move every shape (and every view's frame) by `d` (mm): where the plan sits in the building.
    pub fn translate(&mut self, d: [f64; 2]) {
        if d == [0.0, 0.0] {
            return;
        }
        let shift = |p: &mut [f64; 2]| {
            p[0] += d[0];
            p[1] += d[1];
        };
        for s in &mut self.shapes {
            match &mut s.shape {
                Shape::Area(rings) => rings.iter_mut().flatten().for_each(shift),
                Shape::Stroke(points) => points.iter_mut().for_each(shift),
            }
        }
        for v in &mut self.views {
            if let Some(w) = v.window.as_mut() {
                *w = [w[0] + d[0], w[1] + d[1], w[2] + d[0], w[3] + d[1]];
            }
        }
    }

    /// The index of the source `name` of `kind`, added with a guessed material if new.
    pub fn source(&mut self, name: &str, kind: SourceKind, hints: &[&str]) -> usize {
        if let Some(i) = self
            .sources
            .iter()
            .position(|s| s.name == name && s.kind == kind)
        {
            return i;
        }
        let mut names = vec![name];
        names.extend_from_slice(hints);
        self.sources.push(Source {
            name: name.to_string(),
            kind,
            hints: hints.iter().map(|h| h.to_string()).collect(),
            material: guess_material(&names),
        });
        self.sources.len() - 1
    }

    /// The box round every shape whose source isn't ignored: [min x, min y, max x, max y].
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut b: Option<[f64; 4]> = None;
        for s in &self.shapes {
            if self
                .sources
                .get(s.source)
                .is_none_or(|src| src.material == Material::Ignore)
            {
                continue;
            }
            let points: Box<dyn Iterator<Item = &[f64; 2]>> = match &s.shape {
                Shape::Area(rings) => Box::new(rings.iter().flatten()),
                Shape::Stroke(points) => Box::new(points.iter()),
            };
            for p in points {
                let v = b.get_or_insert([p[0], p[1], p[0], p[1]]);
                *v = [
                    v[0].min(p[0]),
                    v[1].min(p[1]),
                    v[2].max(p[0]),
                    v[3].max(p[1]),
                ];
            }
        }
        b
    }

    /// The walls the plan makes with its sources' materials: ignored sources left out, strokes
    /// widened to strips, guessed walls split thin and thick where paths cross them.
    pub fn walls(&self) -> WallMap {
        self.walls_with(&LossOverrides::new())
    }

    /// As [`FloorPlan::walls`], with calibrated losses in place of the presets' own (guessed
    /// walls too).
    pub fn walls_with(&self, overrides: &LossOverrides) -> WallMap {
        let mut materials: Vec<(String, RfLoss)> = Vec::new();
        let mut index = |name: String, loss: RfLoss| -> usize {
            match materials.iter().position(|(n, l)| *n == name && *l == loss) {
                Some(i) => i,
                None => {
                    materials.push((name, loss));
                    materials.len() - 1
                }
            }
        };
        let (thin_id, _) = heuristic_loss(0.0);
        let (thick_id, _) = heuristic_loss(CONCRETE_FROM_MM);
        let (thin_loss, thick_loss) = (
            preset_loss(thin_id, overrides),
            preset_loss(thick_id, overrides),
        );
        let mut regions = Vec::new();
        for s in &self.shapes {
            let Some(src) = self.sources.get(s.source) else {
                continue;
            };
            if src.material == Material::Ignore {
                continue;
            }
            // Each stroke segment is a region of its own: the quads of one stroke overlap at its
            // corners, and rings of one region are inside by the even-odd rule.
            let parts: Vec<Vec<Vec<[f64; 2]>>> = match &s.shape {
                Shape::Area(rings) => vec![rings.clone()],
                Shape::Stroke(points) => strip(points, stroke_width_mm(&src.material))
                    .into_iter()
                    .map(|q| vec![q])
                    .collect(),
            };
            for rings in parts {
                let region = match &src.material {
                    Material::Ignore => None,
                    Material::Auto => {
                        let thin = index(format!("{thin_id} (guessed)"), thin_loss);
                        let thick = index(format!("{thick_id} (guessed)"), thick_loss);
                        WallRegion::new(rings, thin, src.name.clone())
                            .map(|r| r.guessed(thin, thick, CONCRETE_FROM_MM))
                    }
                    Material::Preset(id) => {
                        let loss = preset_loss(id, overrides);
                        WallRegion::new(rings, index((*id).to_string(), loss), src.name.clone())
                    }
                    Material::Custom(loss) => WallRegion::new(
                        rings,
                        index(format!("{} (custom)", src.name), *loss),
                        src.name.clone(),
                    ),
                };
                if let Some(r) = region {
                    regions.push(r);
                }
            }
        }
        WallMap::new(regions, materials)
    }

    /// How many shapes each source has, by source index.
    pub fn shape_counts(&self) -> BTreeMap<usize, usize> {
        let mut out = BTreeMap::new();
        for s in &self.shapes {
            *out.entry(s.source).or_insert(0) += 1;
        }
        out
    }
}

/// A polyline widened to `width`: one quadrilateral per segment.
pub fn strip(points: &[[f64; 2]], width: f64) -> Vec<Vec<[f64; 2]>> {
    let half = 0.5 * width;
    let mut out = Vec::new();
    for w in points.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else {
            continue;
        };
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len.is_nan() || len <= 1e-9 || !len.is_finite() {
            continue;
        }
        let (nx, ny) = (-dy / len * half, dx / len * half);
        out.push(vec![
            [a[0] + nx, a[1] + ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
            [a[0] - nx, a[1] - ny],
        ]);
    }
    out
}
