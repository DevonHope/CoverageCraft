//! Floor plans from 3D models (SolidCraft's OBJ and STL exports), read with `craft-mesh` and cut
//! by a horizontal plane.
//!
//! - Each object (an OBJ `o`, one SolidCraft body) is a source; its name and its material name
//!   suggest what it is made of.
//! - The cut follows each object's own triangles: every triangle that crosses the plane gives a
//!   segment between two of its edges, and segments sharing an edge join into rings. Open chains
//!   (a mesh with holes) are left out. One area per object, inside by the even-odd rule.
//! - Coordinates are taken as millimetres, Z up (SolidCraft's convention); `up: y` reads a Y-up
//!   file.

use std::collections::HashMap;

use craft_mesh::{Limits, Scene};

use crate::plan::{FloorPlan, PlanShape, Shape, SourceKind};

/// Objects narrower than this where they're cut (mm), and not named as walls, are left out.
const SMALL_MM: f64 = 300.0;

#[derive(Debug, thiserror::Error)]
pub enum MeshImportError {
    #[error("{0}")]
    Mesh(#[from] craft_mesh::Error),
    #[error("not an OBJ or STL file (by its name): {0}")]
    Format(String),
}

/// Which way is up in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Up {
    Z,
    Y,
}

/// The floor plan a mesh file's bytes give, cut at `height` (mm).
pub fn read_mesh(
    bytes: &[u8],
    path: &str,
    height: f64,
    up: Up,
) -> Result<FloorPlan, MeshImportError> {
    let lower = path.to_ascii_lowercase();
    let limits = Limits::default();
    let scene: Scene = if lower.ends_with(".obj") {
        // Material libraries beside the file aren't read: the material names in the OBJ suffice.
        let mut resolve = |_: &str| -> Option<Vec<u8>> { None };
        craft_mesh::obj::read(bytes, &limits, &mut resolve)?
    } else if lower.ends_with(".stl") {
        craft_mesh::stl::read(bytes, &limits)?
    } else {
        return Err(MeshImportError::Format(path.to_string()));
    };
    let mut plan = FloorPlan {
        path: path.to_string(),
        ..FloorPlan::default()
    };
    plan.notes.extend(scene.warnings.iter().take(20).cloned());
    for (index, mesh) in scene.meshes.iter().enumerate() {
        let name = if mesh.name.trim().is_empty() {
            format!("Object {}", index + 1)
        } else {
            mesh.name.clone()
        };
        let materials: Vec<String> = mesh
            .parts
            .iter()
            .filter_map(|p| {
                p.material
                    .and_then(|m| scene.materials.get(m))
                    .map(|m| m.name.clone())
            })
            .filter(|n| !n.is_empty())
            .collect();
        let hints: Vec<&str> = materials.iter().map(String::as_str).collect();
        // Planar coordinates and height of each vertex, by the file's up axis.
        // The mesh's own position pool: corners that share a position share a vertex, whatever
        // their normals, so neighbouring triangles share edges and the cut joins into rings.
        let points: Vec<[f64; 3]> = mesh
            .positions
            .iter()
            .map(|p| match up {
                Up::Z => [p[0], p[1], p[2]],
                Up::Y => [p[0], -p[2], p[1]],
            })
            .collect();
        let triangles: Vec<[u32; 3]> = mesh
            .parts
            .iter()
            .flat_map(|part| part.triangles_in(&mesh.positions))
            .map(|t| t.map(|c| c.position))
            .collect();
        if let Some(rings) = slice(&points, &triangles, height) {
            let source = plan.source(&name, SourceKind::Object, &hints);
            // An object its name doesn't place, smaller than this across where it's cut, isn't a
            // wall: a tap, a plant, a lamp.
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in rings.iter().flatten() {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            if let Some(s) = plan.sources.get_mut(source)
                && s.material == crate::materials::Material::Auto
                && (hi[0] - lo[0]).max(hi[1] - lo[1]) < SMALL_MM
            {
                s.material = crate::materials::Material::Ignore;
            }
            plan.shapes.push(PlanShape {
                source,
                shape: Shape::Area(rings),
            });
        }
    }
    Ok(plan)
}

/// The rings a horizontal plane at `height` cuts out of a triangle mesh; `None` when it doesn't
/// cross it.
pub fn slice(
    points: &[[f64; 3]],
    triangles: &[[u32; 3]],
    height: f64,
) -> Option<Vec<Vec<[f64; 2]>>> {
    let above = |i: u32| points.get(i as usize).map(|p| p[2] > height);
    let crossing = |a: u32, b: u32| -> Option<[f64; 2]> {
        let (pa, pb) = (
            points.get(a.min(b) as usize)?,
            points.get(a.max(b) as usize)?,
        );
        let t = (height - pa[2]) / (pb[2] - pa[2]);
        Some([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t])
    };
    // Each segment joins two edges, keyed by their vertices (lower first).
    let mut links: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    let mut spots: HashMap<(u32, u32), [f64; 2]> = HashMap::new();
    for t in triangles {
        let mut ends = Vec::with_capacity(2);
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if above(a)? != above(b)? {
                let key = (a.min(b), a.max(b));
                if let std::collections::hash_map::Entry::Vacant(v) = spots.entry(key) {
                    v.insert(crossing(a, b)?);
                }
                ends.push(key);
            }
        }
        if let [a, b] = ends.as_slice() {
            links.entry(*a).or_default().push(*b);
            links.entry(*b).or_default().push(*a);
        }
    }
    let mut used: HashMap<(u32, u32), bool> = HashMap::new();
    let mut rings = Vec::new();
    let mut starts: Vec<(u32, u32)> = links.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        if used.get(&start).copied().unwrap_or(false) {
            continue;
        }
        let mut ring = Vec::new();
        let (mut at, mut previous) = (start, None);
        let closed = loop {
            used.insert(at, true);
            ring.push(*spots.get(&at)?);
            let next = links
                .get(&at)?
                .iter()
                .copied()
                .find(|n| Some(*n) != previous && !used.get(n).copied().unwrap_or(false));
            match next {
                Some(n) => {
                    previous = Some(at);
                    at = n;
                }
                None => break links.get(&at)?.contains(&start) && ring.len() >= 3,
            }
        };
        if closed {
            rings.push(ring);
        }
    }
    (!rings.is_empty()).then_some(rings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box 0..10 × 0..4 × 0..3 as 12 triangles.
    fn cube() -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
        let p = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
            [0.0, 0.0, 3.0],
            [10.0, 0.0, 3.0],
            [10.0, 4.0, 3.0],
            [0.0, 4.0, 3.0],
        ];
        let t = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        (p, t)
    }

    #[test]
    fn a_box_cut_through_its_middle_is_one_closed_rectangle() {
        let (p, t) = cube();
        let rings = slice(&p, &t, 1.5).unwrap();
        assert_eq!(rings.len(), 1);
        let ring = &rings[0];
        let (xs, ys): (Vec<f64>, Vec<f64>) = ring.iter().map(|q| (q[0], q[1])).unzip();
        assert!(
            xs.iter().all(|x| (0.0..=10.0).contains(x))
                && ys.iter().all(|y| (0.0..=4.0).contains(y))
        );
        assert!(xs.contains(&0.0) && xs.contains(&10.0));
        // Above or below it, nothing.
        assert!(slice(&p, &t, 5.0).is_none());
        assert!(slice(&p, &t, -1.0).is_none());
    }
}
