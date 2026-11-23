//! Viewpoints: which way a plan is looked at to make the map.
//!
//! - A 3D model (OBJ, STL, a SolidCraft design) is cut by a plane facing the viewer: from the top
//!   (the usual floor plan), the bottom, the front, back, left or right, or a view saved in the
//!   design. The map's x and y are the view's right and up; the cut is at a depth toward the
//!   viewer (the height, from the top).
//! - A drawing (DXF from CADCraft) is already a plan; its named views pick the part of it that is
//!   the map.
//!
//! Directions are in a right-handed, Z-up world, millimetres. A saved camera's yaw, pitch and
//! roll are read as SolidCraft documents them: the eye is back = (−sin yaw · cos pitch,
//! −cos yaw · cos pitch, sin pitch) from the target, right = (cos yaw, −sin yaw, 0), and
//! up = back × right, turned by the roll.

use serde_json::{Value, json};

/// A view's axes: the map's x (right) and y (up), and toward the viewer. Orthonormal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub toward: [f64; 3],
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = dot(a, a).sqrt();
    (n.is_finite() && n > 1e-12).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

impl Basis {
    pub const TOP: Basis = Basis {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        toward: [0.0, 0.0, 1.0],
    };

    /// A standard view by id: top, bottom, front, back, left or right.
    pub fn standard(id: &str) -> Option<Basis> {
        let (right, up, toward) = match id {
            "top" => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            "bottom" => ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]),
            "front" => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
            "back" => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            "right" => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            "left" => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]),
            _ => return None,
        };
        Some(Basis { right, up, toward })
    }

    /// The axes of a camera given by yaw, pitch and roll (radians), as SolidCraft saves views.
    pub fn from_camera(yaw: f64, pitch: f64, roll: f64) -> Option<Basis> {
        if ![yaw, pitch, roll].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (cp, sp) = (pitch.cos(), pitch.sin());
        let toward = unit([-yaw.sin() * cp, -yaw.cos() * cp, sp])?;
        let right = unit([yaw.cos(), -yaw.sin(), 0.0])?;
        let up = unit(cross(toward, right))?;
        let (s, c) = roll.sin_cos();
        let r = unit([
            right[0] * c + up[0] * s,
            right[1] * c + up[1] * s,
            right[2] * c + up[2] * s,
        ])?;
        let u = unit([
            up[0] * c - right[0] * s,
            up[1] * c - right[1] * s,
            up[2] * c - right[2] * s,
        ])?;
        Some(Basis {
            right: r,
            up: u,
            toward,
        })
    }

    /// A world point in this view: map x, map y, depth toward the viewer.
    pub fn to_view(&self, p: [f64; 3]) -> [f64; 3] {
        [dot(p, self.right), dot(p, self.up), dot(p, self.toward)]
    }

    /// A point in this view back in the world.
    pub fn to_world(&self, v: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = v;
        [0, 1, 2].map(|i| {
            self.right.get(i).copied().unwrap_or(0.0) * x
                + self.up.get(i).copied().unwrap_or(0.0) * y
                + self.toward.get(i).copied().unwrap_or(0.0) * z
        })
    }
}

/// The standard views, by id and name.
pub const STANDARD: [(&str, &str); 6] = [
    ("top", "Top"),
    ("bottom", "Bottom"),
    ("front", "Front"),
    ("back", "Back"),
    ("left", "Left"),
    ("right", "Right"),
];

/// A viewpoint a plan offers.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanView {
    /// `top`, `front`… or `view:<name>` for a saved or named one.
    pub id: String,
    pub name: String,
    /// For a model: the axes it is cut along.
    pub basis: Option<Basis>,
    /// For a drawing: the part of it that is the map ([min x, min y, max x, max y], mm).
    pub window: Option<[f64; 4]>,
}

impl PlanView {
    pub fn to_json(&self) -> Value {
        let kind = if self.window.is_some() {
            "window"
        } else if self.id.starts_with("view:") {
            "saved"
        } else {
            "standard"
        };
        json!({"id": self.id, "name": self.name, "kind": kind, "window": self.window})
    }
}

/// The standard views of a model.
pub fn model_views() -> Vec<PlanView> {
    STANDARD
        .iter()
        .filter_map(|(id, name)| {
            Basis::standard(id).map(|b| PlanView {
                id: (*id).to_string(),
                name: (*name).to_string(),
                basis: Some(b),
                window: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn saved_cameras_match_the_standard_views() {
        use std::f64::consts::FRAC_PI_2;
        let pairs = [
            ("front", 0.0, 0.0),
            ("right", -FRAC_PI_2, 0.0),
            ("left", FRAC_PI_2, 0.0),
            ("back", std::f64::consts::PI, 0.0),
            ("top", 0.0, FRAC_PI_2),
        ];
        for (id, yaw, pitch) in pairs {
            let s = Basis::standard(id).unwrap();
            let c = Basis::from_camera(yaw, pitch, 0.0).unwrap();
            assert!(
                close(s.right, c.right) && close(s.up, c.up) && close(s.toward, c.toward),
                "{id}: {c:?}"
            );
        }
        // There and back.
        let b = Basis::from_camera(0.4, 0.3, 0.1).unwrap();
        let p = [120.0, -40.0, 900.0];
        assert!(close(b.to_world(b.to_view(p)), p));
        assert!(Basis::from_camera(f64::NAN, 0.0, 0.0).is_none());
        assert_eq!(model_views().len(), 6);
    }
}
