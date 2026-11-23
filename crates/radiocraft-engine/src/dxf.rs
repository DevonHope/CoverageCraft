//! Floor plans from DXF drawings (CADCraft's own format), read with `craft-dxf`.
//!
//! - Each layer is a source; its name suggests a material (`A-WALL`, `A-GLAZ`, `A-DOOR`...).
//! - Closed polylines, circles and hatch boundaries are areas: a wall drawn as its outline, a
//!   column, a hatched wall. Lines, open polylines and arcs are strokes: a pane or a door leaf
//!   drawn as one line. A door's swing arc isn't a wall and is left out.
//! - Block inserts are expanded, nested ones too; an entity on layer `0` inside a block takes the
//!   insert's layer, as DXF has it.
//! - Coordinates are scaled to millimetres from the drawing's `$INSUNITS`; paper-space and
//!   invisible entities are left out.

use crate::viewpoint::{Basis, PlanView};
use std::collections::HashMap;

use craft_dxf::model::{
    Block, Document, Entity, EntityKind, Groups, HatchEdge, HatchPath, LwPolyline, Polyline,
};

use crate::materials::Material;
use crate::plan::{FloorPlan, PlanShape, Shape, SourceKind};

/// How deep block inserts are expanded.
const MAX_DEPTH: usize = 8;
/// The most shapes one drawing gives, and the most points one shape has.
const MAX_SHAPES: usize = 200_000;
const MAX_POINTS: usize = 100_000;
/// Arcs are followed in steps of at most this angle (radians).
const ARC_STEP: f64 = std::f64::consts::PI / 24.0;

#[derive(Debug, thiserror::Error)]
pub enum DxfImportError {
    #[error("{0}")]
    Dxf(#[from] craft_dxf::DxfError),
    #[error("the drawing has no model-space entities")]
    Empty,
}

/// A 2D affine map: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Clone, Copy, Debug)]
struct Affine([f64; 6]);

impl Affine {
    fn scale(s: f64) -> Affine {
        Affine([s, 0.0, 0.0, s, 0.0, 0.0])
    }

    fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }

    /// `self` after `other`: the map that applies `other` first.
    fn then_inner(&self, other: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }
}

/// Millimetres per drawing unit for a `$INSUNITS` code; unitless drawings are taken as mm.
fn mm_per_unit(code: i64) -> f64 {
    match code {
        1 => 25.4,
        2 => 304.8,
        3 => 1_609_344.0,
        5 => 10.0,
        6 => 1000.0,
        7 => 1_000_000.0,
        8 => 2.54e-5,
        9 => 0.0254,
        10 => 914.4,
        14 => 100.0,
        _ => 1.0,
    }
}

struct Reader<'a> {
    blocks: HashMap<&'a str, &'a Block<'a>>,
    plan: FloorPlan,
    /// Closed outlines (polylines, circles) by layer, joined into areas at the end
    /// ([`nest_rings`]).
    outlines: Vec<(String, Vec<[f64; 2]>)>,
}

/// The floor plan a DXF file's bytes hold.
pub fn read_dxf(bytes: &[u8], path: &str) -> Result<FloorPlan, DxfImportError> {
    let tags = craft_dxf::parse(bytes)?;
    let doc = Document::read(&tags);
    let units = doc
        .header
        .var("$INSUNITS")
        .and_then(|g| g.i(70))
        .unwrap_or(0);
    let scale = mm_per_unit(units);
    let mut reader = Reader {
        blocks: doc.blocks.iter().map(|b| (b.name, b)).collect(),
        plan: FloorPlan {
            path: path.to_string(),
            ..FloorPlan::default()
        },
        outlines: Vec::new(),
    };
    if (scale - 1.0).abs() > 1e-12 {
        reader.plan.notes.push(format!(
            "drawing units (INSUNITS {units}) scaled to millimetres ×{scale}"
        ));
    }
    let top = Affine::scale(scale);
    let mut any = false;
    for record in &doc.entities {
        let entity = Entity::read(record);
        if entity.common.paper_space {
            continue;
        }
        any = true;
        reader.entity(&entity, &top, None, 0);
    }
    if !any {
        return Err(DxfImportError::Empty);
    }
    // A wall drawn as its outer and inner outlines is one area with a hole: outlines of a layer
    // that lie inside one another join, by the even-odd rule.
    let mut by_layer: Vec<(String, Vec<Vec<[f64; 2]>>)> = Vec::new();
    for (layer, ring) in std::mem::take(&mut reader.outlines) {
        match by_layer.iter_mut().find(|(l, _)| *l == layer) {
            Some((_, rings)) => rings.push(ring),
            None => by_layer.push((layer, vec![ring])),
        }
    }
    for (layer, rings) in by_layer {
        for area in nest_rings(rings) {
            reader.push(&layer, Shape::Area(area));
        }
    }
    reader.plan.views = views(&doc, scale, &mut reader.plan.notes);
    Ok(reader.plan)
}

/// The drawing's viewpoints: the whole of it, then each named view of model space that looks
/// straight down, as the part of the drawing it frames.
fn views(doc: &Document<'_>, scale: f64, notes: &mut Vec<String>) -> Vec<PlanView> {
    let mut out = vec![PlanView {
        id: "top".into(),
        name: "Whole drawing".into(),
        basis: Some(Basis::TOP),
        window: None,
    }];
    for v in doc.views().into_iter().take(500) {
        let name = v.name.trim().to_string();
        if name.is_empty() || v.paper_space {
            continue;
        }
        let d = v.direction;
        let plan_view = d[2] > 0.0 && (d[0].hypot(d[1]) / d[2]).abs() < 1e-6;
        if !plan_view {
            notes.push(format!(
                "the named view \"{name}\" doesn't look straight down, so it isn't offered"
            ));
            continue;
        }
        if v.twist.abs() > 1e-9 {
            notes.push(format!(
                "the named view \"{name}\" is twisted; its frame is used without the twist"
            ));
        }
        let (cx, cy) = (v.target[0] + v.center[0], v.target[1] + v.center[1]);
        let (w, h) = (v.width, v.height);
        let window = [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0].map(|x| x * scale);
        if window.iter().all(|x| x.is_finite()) && w > 0.0 && h > 0.0 {
            out.push(PlanView {
                id: format!("view:{name}"),
                name,
                basis: Some(Basis::TOP),
                window: Some(window),
            });
        }
    }
    out
}

impl Reader<'_> {
    fn push(&mut self, layer: &str, shape: Shape) {
        if self.plan.shapes.len() >= MAX_SHAPES {
            if self
                .plan
                .notes
                .last()
                .is_none_or(|n| !n.starts_with("more than"))
            {
                self.plan.notes.push(format!(
                    "more than {MAX_SHAPES} shapes: the rest are left out"
                ));
            }
            return;
        }
        let source = self.plan.source(layer, SourceKind::Layer, &[]);
        self.plan.shapes.push(PlanShape { source, shape });
    }

    fn entity(&mut self, e: &Entity, map: &Affine, parent_layer: Option<&str>, depth: usize) {
        if !e.common.visible {
            return;
        }
        // Layer 0 inside a block takes the insert's layer.
        let layer = match (e.common.layer.as_str(), parent_layer) {
            ("0", Some(parent)) => parent.to_string(),
            (own, _) => own.to_string(),
        };
        match &e.kind {
            EntityKind::Line { start, end } => self.push(
                &layer,
                Shape::Stroke(vec![
                    map.apply([start[0], start[1]]),
                    map.apply([end[0], end[1]]),
                ]),
            ),
            EntityKind::LwPolyline(pl) => self.lwpolyline(&layer, pl, map),
            EntityKind::Polyline(pl) => self.polyline(&layer, pl, map),
            EntityKind::Circle { center, radius } => {
                let ring = arc_points(
                    [center[0], center[1]],
                    *radius,
                    0.0,
                    std::f64::consts::TAU,
                    true,
                );
                let ring: Vec<[f64; 2]> = ring.into_iter().map(|p| map.apply(p)).collect();
                if ring.len() >= 3 {
                    self.outlines.push((layer, ring));
                }
            }
            EntityKind::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                // A door's swing is drawn as an arc: not a wall.
                let source = self.plan.source(&layer, SourceKind::Layer, &[]);
                if self
                    .plan
                    .sources
                    .get(source)
                    .is_some_and(|s| s.material == Material::Preset("wood-door"))
                {
                    return;
                }
                let (a0, a1) = (start_angle.to_radians(), end_angle.to_radians());
                let sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
                let points = arc_points(
                    [center[0], center[1]],
                    *radius,
                    a0,
                    if sweep == 0.0 {
                        std::f64::consts::TAU
                    } else {
                        sweep
                    },
                    true,
                );
                self.push(
                    &layer,
                    Shape::Stroke(points.into_iter().map(|p| map.apply(p)).collect()),
                );
            }
            EntityKind::Hatch(h) => {
                let rings: Vec<Vec<[f64; 2]>> = h
                    .loops
                    .iter()
                    .filter_map(|l| {
                        let ring = hatch_ring(&l.path);
                        (ring.len() >= 3).then(|| ring.into_iter().map(|p| map.apply(p)).collect())
                    })
                    .collect();
                if !rings.is_empty() {
                    self.push(&layer, Shape::Area(rings));
                }
            }
            EntityKind::Insert(ins) => {
                if depth >= MAX_DEPTH {
                    return;
                }
                let Some(block) = self.blocks.get(ins.block.as_str()).copied() else {
                    return;
                };
                let base = Groups(block.tags).point(10);
                let (sx, sy) = (
                    if ins.scale[0] == 0.0 {
                        1.0
                    } else {
                        ins.scale[0]
                    },
                    if ins.scale[1] == 0.0 {
                        1.0
                    } else {
                        ins.scale[1]
                    },
                );
                let (s, c) = ins.rotation.to_radians().sin_cos();
                let columns = ins.columns.clamp(1, 64);
                let rows = ins.rows.clamp(1, 64);
                for row in 0..rows {
                    for column in 0..columns {
                        // Translate to the insert point, rotate, scale, then from the block's base.
                        let offset = [
                            column as f64 * ins.column_spacing,
                            row as f64 * ins.row_spacing,
                        ];
                        let local = Affine([c * sx, s * sx, -s * sy, c * sy, 0.0, 0.0]);
                        let [ox, oy] = local.apply(offset);
                        let [bx, by] = local.apply([base[0], base[1]]);
                        let place = Affine([
                            c * sx,
                            s * sx,
                            -s * sy,
                            c * sy,
                            ins.insert[0] + ox - bx,
                            ins.insert[1] + oy - by,
                        ]);
                        let inner = map.then_inner(&place);
                        for record in &block.entities {
                            let child = Entity::read(record);
                            self.entity(&child, &inner, Some(&layer), depth + 1);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn lwpolyline(&mut self, layer: &str, pl: &LwPolyline, map: &Affine) {
        let vertices: Vec<[f64; 3]> = pl.vertices.iter().map(|v| [v.x, v.y, v.bulge]).collect();
        let points = bulged(&vertices, pl.closed);
        self.polyline_shape(layer, points, pl.closed, map);
    }

    fn polyline(&mut self, layer: &str, pl: &Polyline, map: &Affine) {
        // 3D polylines and meshes aren't plan outlines.
        if pl.flags & (8 | 16 | 64) != 0 {
            return;
        }
        let closed = pl.flags & 1 != 0;
        let vertices: Vec<[f64; 3]> = pl
            .vertices
            .iter()
            .map(|v| [v.location[0], v.location[1], v.bulge])
            .collect();
        let points = bulged(&vertices, closed);
        self.polyline_shape(layer, points, closed, map);
    }

    fn polyline_shape(&mut self, layer: &str, points: Vec<[f64; 2]>, closed: bool, map: &Affine) {
        let points: Vec<[f64; 2]> = points
            .into_iter()
            .take(MAX_POINTS)
            .map(|p| map.apply(p))
            .collect();
        if closed && points.len() >= 3 {
            self.outlines.push((layer.to_string(), points));
        } else if points.len() >= 2 {
            self.push(layer, Shape::Stroke(points));
        }
    }
}

/// Whether `p` lies inside `ring`, by the even-odd rule.
fn inside(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let (Some(a), Some(b)) = (ring.get(i), ring.get((i + 1) % ring.len())) else {
            continue;
        };
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Rings grouped into areas: a ring that lies wholly inside another (all its points) joins it,
/// so an outer and an inner outline make a wall with a hole; rings that only overlap stay
/// apart, so walls drawn as overlapping rectangles don't cancel where they cross.
fn nest_rings(rings: Vec<Vec<[f64; 2]>>) -> Vec<Vec<Vec<[f64; 2]>>> {
    let n = rings.len();
    // Each ring's group: the outermost ring holding it.
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while let Some(&p) = parent.get(i) {
            if p == i {
                break;
            }
            i = p;
        }
        i
    }
    if n <= 2000 {
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let (Some(inner), Some(outer)) = (rings.get(i), rings.get(j)) else {
                    continue;
                };
                if inner.iter().all(|p| inside(outer, *p)) {
                    let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                    if ri != rj
                        && let Some(slot) = parent.get_mut(ri)
                    {
                        *slot = rj;
                    }
                }
            }
        }
    }
    let mut groups: Vec<(usize, Vec<Vec<[f64; 2]>>)> = Vec::new();
    for (i, ring) in rings.into_iter().enumerate() {
        let r = root(&mut parent, i);
        match groups.iter_mut().find(|(g, _)| *g == r) {
            Some((_, rs)) => rs.push(ring),
            None => groups.push((r, vec![ring])),
        }
    }
    groups.into_iter().map(|(_, rs)| rs).collect()
}

/// Points along an arc from angle `start` sweeping `sweep` (radians), counter-clockwise when
/// `ccw`, both ends included.
fn arc_points(center: [f64; 2], radius: f64, start: f64, sweep: f64, ccw: bool) -> Vec<[f64; 2]> {
    if !(radius.is_finite() && radius > 0.0 && sweep.is_finite()) {
        return Vec::new();
    }
    let n = ((sweep.abs() / ARC_STEP).ceil() as usize).clamp(1, 720);
    let dir = if ccw { 1.0 } else { -1.0 };
    (0..=n)
        .map(|k| {
            let t = start + dir * sweep * k as f64 / n as f64;
            [center[0] + radius * t.cos(), center[1] + radius * t.sin()]
        })
        .collect()
}

/// A polyline's points with its bulges (arcs between vertices) followed by straight steps.
fn bulged(vertices: &[[f64; 3]], closed: bool) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let n = vertices.len();
    for i in 0..n {
        let Some(v) = vertices.get(i) else { break };
        out.push([v[0], v[1]]);
        let next = if i + 1 < n {
            vertices.get(i + 1)
        } else if closed {
            vertices.first()
        } else {
            None
        };
        let Some(w) = next else { continue };
        let bulge = v[2];
        if bulge.abs() < 1e-9 || !bulge.is_finite() {
            continue;
        }
        // The arc from v to w: its sweep is 4·atan(bulge), positive counter-clockwise.
        let sweep = 4.0 * bulge.atan();
        let (dx, dy) = (w[0] - v[0], w[1] - v[1]);
        let chord = (dx * dx + dy * dy).sqrt();
        if chord < 1e-12 {
            continue;
        }
        let radius = chord / (2.0 * (sweep / 2.0).sin()).abs();
        // The centre lies off the chord's middle, on the side the bulge's sign says.
        let mid = [(v[0] + w[0]) / 2.0, (v[1] + w[1]) / 2.0];
        let h = radius * (sweep / 2.0).cos() * sweep.signum();
        let normal = [-dy / chord, dx / chord];
        let center = [mid[0] + normal[0] * h, mid[1] + normal[1] * h];
        let start = (v[1] - center[1]).atan2(v[0] - center[0]);
        let arc = arc_points(center, radius, start, sweep.abs(), sweep > 0.0);
        // Its inner points: the ends are the vertices.
        out.extend(arc.iter().skip(1).take(arc.len().saturating_sub(2)));
    }
    if closed && out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

/// A hatch boundary loop as a ring.
fn hatch_ring(path: &HatchPath) -> Vec<[f64; 2]> {
    match path {
        // A boundary loop is closed whatever its flag says.
        HatchPath::Polyline { vertices, .. } => bulged(vertices, true),
        HatchPath::Edges(edges) => {
            let mut out: Vec<[f64; 2]> = Vec::new();
            for edge in edges {
                let points = match edge {
                    HatchEdge::Line { start, end } => vec![*start, *end],
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        ccw,
                    } => {
                        let (a0, a1) = (start_angle.to_radians(), end_angle.to_radians());
                        let sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
                        let sweep = if sweep == 0.0 {
                            std::f64::consts::TAU
                        } else {
                            sweep
                        };
                        // A clockwise edge has its angles measured clockwise.
                        if *ccw {
                            arc_points(*center, *radius, a0, sweep, true)
                        } else {
                            arc_points(*center, *radius, -a0, sweep, false)
                        }
                    }
                    HatchEdge::Ellipse {
                        center,
                        major,
                        ratio,
                        start_angle,
                        end_angle,
                        ccw,
                    } => {
                        let (a0, a1) = (start_angle.to_radians(), end_angle.to_radians());
                        let sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
                        let sweep = if sweep == 0.0 {
                            std::f64::consts::TAU
                        } else {
                            sweep
                        };
                        let n = ((sweep / ARC_STEP).ceil() as usize).clamp(1, 720);
                        let minor = [-major[1] * ratio, major[0] * ratio];
                        let dir = if *ccw { 1.0 } else { -1.0 };
                        (0..=n)
                            .map(|k| {
                                let t = dir * (a0 + sweep * k as f64 / n as f64);
                                [
                                    center[0] + major[0] * t.cos() + minor[0] * t.sin(),
                                    center[1] + major[1] * t.cos() + minor[1] * t.sin(),
                                ]
                            })
                            .collect()
                    }
                    // A spline edge is followed through its fit points, else its control points.
                    HatchEdge::Spline { fit, control, .. } => {
                        if fit.len() >= 2 {
                            fit.clone()
                        } else {
                            control.clone()
                        }
                    }
                };
                for p in points {
                    if out
                        .last()
                        .is_none_or(|q| (q[0] - p[0]).abs() > 1e-9 || (q[1] - p[1]).abs() > 1e-9)
                    {
                        out.push(p);
                    }
                }
            }
            if out.len() > 1 && out.first() == out.last() {
                out.pop();
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, size: f64) -> Vec<[f64; 2]> {
        vec![
            [x0, y0],
            [x0 + size, y0],
            [x0 + size, y0 + size],
            [x0, y0 + size],
        ]
    }

    #[test]
    fn an_outline_inside_another_is_a_hole_and_overlapping_ones_stay_apart() {
        // A wall's outer and inner faces: one area of two rings.
        let nested = nest_rings(vec![square(0.0, 0.0, 10.0), square(1.0, 1.0, 8.0)]);
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].len(), 2);
        // Two walls drawn as overlapping rectangles: two areas.
        let overlapping = nest_rings(vec![square(0.0, 0.0, 10.0), square(8.0, 8.0, 10.0)]);
        assert_eq!(overlapping.len(), 2);
    }

    #[test]
    fn a_bulge_of_one_is_a_half_circle() {
        // From (0,0) to (2,0) with bulge 1: a half circle of radius 1 below the chord (clockwise
        // from the first to the second point would be negative).
        let pts = bulged(&[[0.0, 0.0, 1.0], [2.0, 0.0, 0.0]], false);
        assert!(pts.len() > 4);
        assert!(
            pts.iter()
                .all(|p| ((p[0] - 1.0).powi(2) + p[1].powi(2)).sqrt() - 1.0 < 1e-9)
        );
        assert!(
            pts.iter().any(|p| p[1] < -0.9),
            "a positive bulge turns counter-clockwise, below the chord: {pts:?}"
        );
    }
}
