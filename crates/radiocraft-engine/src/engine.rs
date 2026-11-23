//! The engine: RadioCraft's state and every action on it as a command. The window and the CLI
//! both run commands through [`Engine::execute`], so anything a panel can do, a script can do.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::devices::{self, Device, EIRP_CAP_DBM};
use crate::dxf::read_dxf;
use crate::materials::{Band, Material, RF_PRESETS};
use crate::mesh::{Up, read_mesh};
use crate::model::{self, Cell, Emitter, Grid, Model, Scratch, WallMap};
use crate::plan::{FloorPlan, SourceKind};

/// The largest plan or project file read, in bytes.
const MAX_FILE_BYTES: u64 = 512 << 20;
/// How far a router may be from the origin (mm): 100 km.
const MAX_COORD: f64 = 1.0e8;
/// The most routers a plan has.
const MAX_ROUTERS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{command}: {message}")]
    Bad { command: String, message: String },
    #[error("unknown command `{0}` (commands.list lists them)")]
    Unknown(String),
}

fn bad(command: &str, message: impl Into<String>) -> EngineError {
    EngineError::Bad {
        command: command.to_string(),
        message: message.into(),
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// How a router's power is given.
#[derive(Clone, Debug, PartialEq)]
pub enum Power {
    /// A device by id.
    Device(String),
    /// EIRP per band (dBm); `None` where it has no radio.
    Eirp([Option<f64>; 3]),
}

/// A router or access point on the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Router {
    pub id: u64,
    pub name: String,
    /// Plan x, y (mm).
    pub at: [f64; 2],
    /// Height above the floor (mm).
    pub height: f64,
    pub power: Power,
}

/// The analysis settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub band: Band,
    /// The grid's cell size (mm).
    pub grid_step: f64,
    /// The receiving device's height (mm).
    pub client_height: f64,
    pub exponent: f64,
    /// The height a model is cut at (mm).
    pub slice_height: f64,
    pub cap_eirp: bool,
    /// Openings narrower than this (mm) close the floor.
    pub close_gaps: f64,
    pub up: Up,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            band: Band::G5,
            grid_step: 100.0,
            client_height: 1000.0,
            exponent: 2.2,
            slice_height: 1000.0,
            cap_eirp: true,
            close_gaps: 1000.0,
            up: Up::Z,
        }
    }
}

/// What the analysis found for the shown band.
#[derive(Clone, Debug)]
pub struct Coverage {
    pub band: Band,
    pub grid: Grid,
    pub cells: Vec<Cell>,
    pub values: Vec<f32>,
    /// Straight RGBA8, a pixel a cell, the first row the grid's lowest.
    pub rgba: Arc<Vec<u8>>,
    pub stats: Option<model::Stats>,
    /// Where the floor came from: "enclosed" or "open".
    pub floor: &'static str,
    pub walls: Arc<WallMap>,
    /// For each step of [`model::SCALE`], the share of the floor (0–100) at its level or better;
    /// for the last, open step, the share below the one before it.
    pub floor_share: Vec<f64>,
}

/// RadioCraft's state.
pub struct Engine {
    plan: Option<FloorPlan>,
    routers: Vec<Router>,
    next_router: u64,
    settings: Settings,
    devices: Vec<Device>,
    devices_file: Option<String>,
    walls: Option<Arc<WallMap>>,
    coverage: Option<Arc<Coverage>>,
    /// Changes whenever anything shown changes.
    revision: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            plan: None,
            routers: Vec::new(),
            next_router: 1,
            settings: Settings::default(),
            devices: devices::generic(),
            devices_file: None,
            walls: None,
            coverage: None,
            revision: 0,
        }
    }
}

/// The commands, with a line on what each takes.
pub const COMMANDS: &[(&str, &str)] = &[
    ("commands.list", "{} → the commands"),
    (
        "coverage.at",
        "{at: [x, y]} → the signal there in each band, and from which router",
    ),
    (
        "coverage.compute",
        "{values?: bool} → the shown band's coverage figures (values: every cell's signal)",
    ),
    ("devices.list", "{} → the devices a router can name"),
    (
        "devices.load",
        "{path} → load a device file (a JSON list of {id, name?, eirp_dbm}); null path unloads it",
    ),
    (
        "export.png",
        "{path} → write the heatmap seen from above, north up, walls outlined, routers marked",
    ),
    (
        "material.set",
        "{source, kind?: layer|object, material: auto|ignore|preset id|{\"2.4\",\"5\",\"6\": dB}}",
    ),
    (
        "plan.info",
        "{} → the plan's sources (layers or objects) with their materials, its box and notes",
    ),
    (
        "plan.open",
        "{path, slice_height?, up?: z|y} → read a DXF plan or cut an OBJ/STL model",
    ),
    (
        "project.open",
        "{path} → open a RadioCraft project (.rcraft)",
    ),
    (
        "project.save",
        "{path} → save the project: the plan's path, materials, routers and settings",
    ),
    (
        "router.add",
        "{at: [x, y], height?, name?, device? | eirp_dbm?: dBm | {\"2.4\",\"5\",\"6\"}} → {id}",
    ),
    ("router.list", "{} → the routers"),
    ("router.move", "{id, at: [x, y]}"),
    ("router.remove", "{id}"),
    ("router.set", "{id, name?, height?, device? | eirp_dbm?}"),
    (
        "settings.set",
        "{band?: 2.4|5|6, grid_step?, client_height?, exponent?, slice_height?, cap_eirp?, close_gaps?, up?}",
    ),
];

fn point(command: &str, p: &Value, key: &str) -> Result<[f64; 2]> {
    let a = p
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(command, format!("`{key}` is [x, y] in mm")))?;
    match a.as_slice() {
        [x, y] => {
            let (x, y) = (x.as_f64(), y.as_f64());
            match (x, y) {
                (Some(x), Some(y))
                    if x.is_finite()
                        && y.is_finite()
                        && x.abs() <= MAX_COORD
                        && y.abs() <= MAX_COORD =>
                {
                    Ok([x, y])
                }
                _ => Err(bad(
                    command,
                    format!("`{key}` must be finite numbers in mm"),
                )),
            }
        }
        _ => Err(bad(command, format!("`{key}` is [x, y] in mm"))),
    }
}

fn ranged(command: &str, p: &Value, key: &str, lo: f64, hi: f64) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (lo..=hi).contains(&x) => Ok(Some(x)),
            _ => Err(bad(
                command,
                format!("`{key}` must be a number from {lo} to {hi}"),
            )),
        },
    }
}

fn eirp_param(command: &str, v: &Value) -> Result<[Option<f64>; 3]> {
    let check = |x: f64| -> Result<f64> {
        if x.is_finite() && (-30.0..=60.0).contains(&x) {
            Ok(x)
        } else {
            Err(bad(command, "an EIRP is -30 to 60 dBm"))
        }
    };
    match v {
        Value::Number(n) => {
            let x = check(n.as_f64().unwrap_or(f64::NAN))?;
            Ok([Some(x); 3])
        }
        Value::Object(m) => {
            let mut out = [None; 3];
            for (k, v) in m {
                let band = Band::ALL
                    .into_iter()
                    .find(|b| b.name() == k)
                    .ok_or_else(|| bad(command, "`eirp_dbm` keys are \"2.4\", \"5\" and \"6\""))?;
                if let Some(slot) = out.get_mut(band.index()) {
                    *slot = Some(check(v.as_f64().unwrap_or(f64::NAN))?);
                }
            }
            Ok(out)
        }
        _ => Err(bad(
            command,
            "`eirp_dbm` is dBm, or {\"2.4\", \"5\", \"6\": dBm}",
        )),
    }
}

fn band_map(values: &[Option<f64>; 3]) -> Value {
    let mut m = serde_json::Map::new();
    for b in Band::ALL {
        if let Some(Some(v)) = values.get(b.index()) {
            m.insert(b.name().to_string(), json!(v));
        }
    }
    Value::Object(m)
}

fn read_file(command: &str, path: &str) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| bad(command, format!("{path}: {e}")))?;
    if !meta.is_file() {
        return Err(bad(command, format!("{path}: not a file")));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(bad(
            command,
            format!("{path}: larger than {} MB", MAX_FILE_BYTES >> 20),
        ));
    }
    std::fs::read(path).map_err(|e| bad(command, format!("{path}: {e}")))
}

impl Engine {
    pub fn new() -> Engine {
        Engine::default()
    }

    pub fn plan(&self) -> Option<&FloorPlan> {
        self.plan.as_ref()
    }

    pub fn routers(&self) -> &[Router] {
        &self.routers
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self, walls_too: bool) {
        self.revision = self.revision.wrapping_add(1);
        self.coverage = None;
        if walls_too {
            self.walls = None;
        }
    }

    /// Run `command` with `params`.
    pub fn execute(&mut self, command: &str, params: &Value) -> Result<Value> {
        let p = if params.is_null() {
            &Value::Object(Default::default())
        } else {
            params
        };
        if !p.is_object() {
            return Err(bad(command, "parameters must be an object"));
        }
        match command {
            "commands.list" => Ok(json!(COMMANDS.iter().map(|(id, doc)| json!({"id": id, "params": doc})).collect::<Vec<_>>())),
            "plan.open" => self.plan_open(command, p),
            "plan.info" => self.plan_info(command),
            "material.set" => self.material_set(command, p),
            "router.add" => self.router_add(command, p),
            "router.move" => self.router_move(command, p),
            "router.set" => self.router_set(command, p),
            "router.remove" => self.router_remove(command, p),
            "router.list" => Ok(json!(self.routers.iter().map(|r| self.router_json(r)).collect::<Vec<_>>())),
            "settings.set" => self.settings_set(command, p),
            "coverage.compute" => self.coverage_compute(command, p),
            "coverage.at" => self.coverage_at(command, p),
            "devices.load" => self.devices_load(command, p),
            "devices.list" => Ok(json!(
                self.devices
                    .iter()
                    .map(|d| json!({"id": d.id, "name": d.name, "eirp_dbm": band_map(&d.eirp_dbm), "source": if d.from_file { "file" } else { "generic" }}))
                    .collect::<Vec<_>>()
            )),
            "export.png" => self.export_png(command, p),
            "project.save" => self.project_save(command, p),
            "project.open" => self.project_open(command, p),
            other => Err(EngineError::Unknown(other.to_string())),
        }
    }

    fn plan_open(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is the plan's file"))?
            .to_string();
        if let Some(h) = ranged(command, p, "slice_height", -MAX_COORD, MAX_COORD)? {
            self.settings.slice_height = h;
        }
        if let Some(up) = p.get("up").and_then(Value::as_str) {
            self.settings.up = match up {
                "z" | "Z" => Up::Z,
                "y" | "Y" => Up::Y,
                _ => return Err(bad(command, "`up` is z or y")),
            };
        }
        let plan = self.read_plan(command, &path)?;
        // Keep the materials of sources the new plan still has.
        let mut plan = plan;
        if let Some(old) = &self.plan {
            for s in &mut plan.sources {
                if let Some(o) = old
                    .sources
                    .iter()
                    .find(|o| o.name == s.name && o.kind == s.kind)
                {
                    s.material = o.material.clone();
                }
            }
        }
        self.plan = Some(plan);
        self.changed(true);
        self.plan_info(command)
    }

    fn read_plan(&self, command: &str, path: &str) -> Result<FloorPlan> {
        let bytes = read_file(command, path)?;
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".dxf") {
            read_dxf(&bytes, path).map_err(|e| bad(command, format!("{path}: {e}")))
        } else if lower.ends_with(".obj") || lower.ends_with(".stl") {
            read_mesh(&bytes, path, self.settings.slice_height, self.settings.up)
                .map_err(|e| bad(command, format!("{path}: {e}")))
        } else {
            Err(bad(
                command,
                format!("{path}: RadioCraft reads DXF plans and OBJ or STL models"),
            ))
        }
    }

    fn plan_info(&self, command: &str) -> Result<Value> {
        let plan = self
            .plan
            .as_ref()
            .ok_or_else(|| bad(command, "no plan is open (plan.open)"))?;
        let counts = plan.shape_counts();
        Ok(json!({
            "path": plan.path,
            "sources": plan.sources.iter().enumerate().map(|(i, s)| json!({
                "name": s.name,
                "kind": s.kind.name(),
                "material": s.material.to_json(),
                "shapes": counts.get(&i).copied().unwrap_or(0),
            })).collect::<Vec<_>>(),
            "bounds": plan.bounds(),
            "notes": plan.notes,
            "presets": RF_PRESETS.iter().map(|p| p.id).collect::<Vec<_>>(),
        }))
    }

    fn material_set(&mut self, command: &str, p: &Value) -> Result<Value> {
        let name = p
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`source` is a layer or object name"))?;
        let kind = match p.get("kind").and_then(Value::as_str) {
            None => None,
            Some("layer") => Some(SourceKind::Layer),
            Some("object") => Some(SourceKind::Object),
            Some(_) => return Err(bad(command, "`kind` is layer or object")),
        };
        let material = p
            .get("material")
            .and_then(Material::from_json)
            .ok_or_else(|| bad(command, "`material` is auto, ignore, a preset id or {\"2.4\", \"5\", \"6\": dB} (0–100)"))?;
        let plan = self
            .plan
            .as_mut()
            .ok_or_else(|| bad(command, "no plan is open (plan.open)"))?;
        let mut hits = 0;
        for s in plan
            .sources
            .iter_mut()
            .filter(|s| s.name == name && kind.is_none_or(|k| k == s.kind))
        {
            s.material = material.clone();
            hits += 1;
        }
        if hits == 0 {
            return Err(bad(
                command,
                format!("the plan has no layer or object `{name}`"),
            ));
        }
        self.changed(true);
        Ok(json!({"source": name, "material": material.to_json()}))
    }

    fn power(&self, command: &str, p: &Value) -> Result<Option<Power>> {
        match (p.get("device"), p.get("eirp_dbm")) {
            (Some(d), Some(e)) if !d.is_null() && !e.is_null() => {
                Err(bad(command, "give `device` or `eirp_dbm`, not both"))
            }
            (Some(Value::String(id)), _) => {
                let d = self
                    .devices
                    .iter()
                    .find(|d| d.id.eq_ignore_ascii_case(id.trim()))
                    .ok_or_else(|| bad(command, format!("no device `{id}` (devices.list)")))?;
                Ok(Some(Power::Device(d.id.clone())))
            }
            (_, Some(e)) if !e.is_null() => Ok(Some(Power::Eirp(eirp_param(command, e)?))),
            (Some(d), _) if !d.is_null() => Err(bad(command, "`device` is a device id")),
            _ => Ok(None),
        }
    }

    fn router(&mut self, command: &str, p: &Value) -> Result<usize> {
        let id = p
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad(command, "`id` is a router's id"))?;
        self.routers
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| bad(command, format!("no router {id}")))
    }

    fn router_add(&mut self, command: &str, p: &Value) -> Result<Value> {
        if self.routers.len() >= MAX_ROUTERS {
            return Err(bad(command, format!("at most {MAX_ROUTERS} routers")));
        }
        let at = point(command, p, "at")?;
        let height = ranged(command, p, "height", -MAX_COORD, MAX_COORD)?
            .unwrap_or(self.settings.client_height);
        let power = self.power(command, p)?.unwrap_or_else(|| {
            Power::Device(
                self.devices
                    .first()
                    .map(|d| d.id.clone())
                    .unwrap_or_default(),
            )
        });
        let id = self.next_router;
        self.next_router += 1;
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty())
            .map_or_else(|| format!("Router {id}"), str::to_string);
        self.routers.push(Router {
            id,
            name,
            at,
            height,
            power,
        });
        self.changed(false);
        Ok(json!({"id": id}))
    }

    fn router_move(&mut self, command: &str, p: &Value) -> Result<Value> {
        let i = self.router(command, p)?;
        let at = point(command, p, "at")?;
        if let Some(r) = self.routers.get_mut(i) {
            r.at = at;
        }
        self.changed(false);
        Ok(json!({"id": p.get("id"), "at": at}))
    }

    fn router_set(&mut self, command: &str, p: &Value) -> Result<Value> {
        let i = self.router(command, p)?;
        let height = ranged(command, p, "height", -MAX_COORD, MAX_COORD)?;
        let power = self.power(command, p)?;
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string);
        if let Some(r) = self.routers.get_mut(i) {
            if let Some(h) = height {
                r.height = h;
            }
            if let Some(power) = power {
                r.power = power;
            }
            if let Some(name) = name {
                r.name = name;
            }
        }
        self.changed(false);
        Ok(self
            .routers
            .get(i)
            .map(|r| self.router_json(r))
            .unwrap_or(Value::Null))
    }

    fn router_remove(&mut self, command: &str, p: &Value) -> Result<Value> {
        let i = self.router(command, p)?;
        self.routers.remove(i);
        self.changed(false);
        Ok(json!({"removed": p.get("id")}))
    }

    fn router_json(&self, r: &Router) -> Value {
        let (device, eirp) = match &r.power {
            Power::Device(id) => (Some(id.clone()), self.eirp(r)),
            Power::Eirp(e) => (None, self.cap(*e)),
        };
        json!({"id": r.id, "name": r.name, "at": r.at, "height": r.height, "device": device, "eirp_dbm": band_map(&eirp)})
    }

    fn cap(&self, eirp: [Option<f64>; 3]) -> [Option<f64>; 3] {
        let mut e = eirp;
        if self.settings.cap_eirp {
            for (v, limit) in e.iter_mut().zip(EIRP_CAP_DBM) {
                *v = v.map(|v| v.min(limit));
            }
        }
        e
    }

    /// A router's EIRP per band, after the cap.
    pub fn eirp(&self, r: &Router) -> [Option<f64>; 3] {
        match &r.power {
            Power::Device(id) => self.cap(
                self.devices
                    .iter()
                    .find(|d| d.id == *id)
                    .map_or([None; 3], |d| d.eirp_dbm),
            ),
            Power::Eirp(e) => self.cap(*e),
        }
    }

    fn settings_set(&mut self, command: &str, p: &Value) -> Result<Value> {
        let mut s = self.settings.clone();
        if let Some(b) = p.get("band") {
            let name = b
                .as_str()
                .map(str::to_string)
                .or_else(|| b.as_f64().map(|x| x.to_string()))
                .unwrap_or_default();
            s.band = Band::parse(&name).ok_or_else(|| bad(command, "`band` is 2.4, 5 or 6"))?;
        }
        if let Some(v) = ranged(command, p, "grid_step", 10.0, 2000.0)? {
            s.grid_step = v;
        }
        if let Some(v) = ranged(command, p, "client_height", -MAX_COORD, MAX_COORD)? {
            s.client_height = v;
        }
        if let Some(v) = ranged(command, p, "exponent", 1.5, 6.0)? {
            s.exponent = v;
        }
        if let Some(v) = ranged(command, p, "close_gaps", 0.0, 5000.0)? {
            s.close_gaps = v;
        }
        if let Some(v) = p.get("cap_eirp") {
            s.cap_eirp = v
                .as_bool()
                .ok_or_else(|| bad(command, "`cap_eirp` is true or false"))?;
        }
        if let Some(up) = p.get("up").and_then(Value::as_str) {
            s.up = match up {
                "z" | "Z" => Up::Z,
                "y" | "Y" => Up::Y,
                _ => return Err(bad(command, "`up` is z or y")),
            };
        }
        let new_slice = ranged(command, p, "slice_height", -MAX_COORD, MAX_COORD)?;
        let recut = new_slice.is_some_and(|h| h != s.slice_height) || s.up != self.settings.up;
        if let Some(h) = new_slice {
            s.slice_height = h;
        }
        self.settings = s;
        // A model is cut again at the new height; a DXF plan has no height.
        if recut && let Some(path) = self.plan.as_ref().map(|p| p.path.clone()) {
            let lower = path.to_ascii_lowercase();
            if lower.ends_with(".obj") || lower.ends_with(".stl") {
                self.plan_open(command, &json!({"path": path}))?;
            }
        }
        self.changed(true);
        Ok(json!({
            "band": self.settings.band.name(),
            "grid_step": self.settings.grid_step,
            "client_height": self.settings.client_height,
            "exponent": self.settings.exponent,
            "slice_height": self.settings.slice_height,
            "cap_eirp": self.settings.cap_eirp,
            "close_gaps": self.settings.close_gaps,
        }))
    }

    fn emitters(&self) -> Vec<Emitter> {
        self.routers
            .iter()
            .map(|r| Emitter {
                name: r.name.clone(),
                at: r.at,
                height: r.height,
                eirp_dbm: self.eirp(r),
            })
            .collect()
    }

    fn model(&self) -> Model {
        Model {
            exponent: self.settings.exponent,
            client_height: self.settings.client_height,
        }
    }

    /// The walls of the open plan, built once until a material or the plan changes.
    pub fn walls(&mut self) -> Option<Arc<WallMap>> {
        if self.walls.is_none() {
            self.walls = self.plan.as_ref().map(|p| Arc::new(p.walls()));
        }
        self.walls.clone()
    }

    /// The coverage of the shown band, worked out once until something changes. `None` without a
    /// plan or with nothing to analyse.
    pub fn coverage(&mut self) -> Option<Arc<Coverage>> {
        if let Some(c) = &self.coverage {
            return Some(c.clone());
        }
        let walls = self.walls()?;
        let bounds = walls.bounds()?;
        let s = self.settings.clone();
        let margin = 0.5 * s.close_gaps + 2.0 * s.grid_step;
        let grid = Grid::covering(
            [
                bounds[0] - margin,
                bounds[1] - margin,
                bounds[2] + margin,
                bounds[3] + margin,
            ],
            s.grid_step,
        )?;
        let solid = model::wall_cells(&walls, &grid);
        let enclosed = model::enclosed(&walls, &grid, &solid, s.close_gaps);
        let floor = if enclosed.iter().any(|x| *x) {
            "enclosed"
        } else {
            "open"
        };
        let cells: Vec<Cell> = solid
            .iter()
            .zip(&enclosed)
            .map(|(w, e)| {
                if *w {
                    Cell::Wall
                } else if *e || floor == "open" {
                    Cell::Floor
                } else {
                    Cell::Outside
                }
            })
            .collect();
        let emitters = self.emitters();
        let values = if emitters.iter().any(|e| e.eirp(s.band).is_some()) {
            model::coverage(&walls, &emitters, &self.model(), s.band, &grid, &cells)
        } else {
            vec![f32::NAN; grid.len()]
        };
        let rgba = Arc::new(model::rgba(&values, &cells, &grid));
        let stats = model::stats(&values, &cells);
        let floor_share = floor_share(&values, &cells);
        let c = Arc::new(Coverage {
            band: s.band,
            grid,
            cells,
            values,
            rgba,
            stats,
            floor,
            walls,
            floor_share,
        });
        self.coverage = Some(c.clone());
        Some(c)
    }

    fn coverage_compute(&mut self, command: &str, p: &Value) -> Result<Value> {
        if self.plan.is_none() {
            return Err(bad(command, "no plan is open (plan.open)"));
        }
        let c = self.coverage().ok_or_else(|| bad(command, "the plan has no walls to analyse: every layer or object is ignored, or none is closed"))?;
        let round = |v: f64| (v * 10.0).round() / 10.0;
        let cell_m2 = c.grid.step * c.grid.step / 1e6;
        let mut out = json!({
            "band": c.band.name(),
            "routers": self.routers.len(),
            "floor": c.floor,
            "floor_m2": round(c.cells.iter().filter(|x| **x == Cell::Floor).count() as f64 * cell_m2),
            "grid": {"origin": c.grid.origin, "step": c.grid.step, "columns": c.grid.columns, "rows": c.grid.rows},
            "walls": c.walls.regions.len(),
        });
        if let Some(st) = &c.stats {
            out["pct_at_least_60"] = json!(round(st.good * 100.0));
            out["pct_at_least_67"] = json!(round(st.fair * 100.0));
            out["p5_dbm"] = json!(round(st.p5));
            out["median_dbm"] = json!(round(st.median));
            out["min_dbm"] = json!(round(st.min));
            out["max_dbm"] = json!(round(st.max));
        } else {
            out["note"] = json!("no router has a radio in this band: add a router (router.add)");
        }
        if p.get("values").and_then(Value::as_bool).unwrap_or(false) {
            out["values"] = json!(
                c.values
                    .iter()
                    .map(|v| v.is_finite().then(|| (v * 10.0).round() / 10.0))
                    .collect::<Vec<_>>()
            );
        }
        Ok(out)
    }

    fn coverage_at(&mut self, command: &str, p: &Value) -> Result<Value> {
        let at = point(command, p, "at")?;
        let walls = self
            .walls()
            .ok_or_else(|| bad(command, "no plan is open (plan.open)"))?;
        let emitters = self.emitters();
        let model = self.model();
        let mut scratch = Scratch::default();
        let mut out = serde_json::Map::new();
        for band in Band::ALL {
            if let Some((v, i)) = model::best_at(&walls, &emitters, &model, band, at, &mut scratch)
            {
                let from = emitters.get(i);
                out.insert(
                    band.name().to_string(),
                    json!({
                        "rssi_dbm": (v * 10.0).round() / 10.0,
                        "from": from.map(|e| e.name.clone()),
                        "walls_db": from.map(|e| (walls.path_loss(e.at, at, band, &mut scratch) * 10.0).round() / 10.0),
                    }),
                );
            }
        }
        Ok(json!({"at": at, "bands": out}))
    }

    fn devices_load(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = match p.get("path") {
            Some(Value::String(s)) => Some(s.clone()),
            None | Some(Value::Null) => None,
            Some(_) => return Err(bad(command, "`path` is a device file, or null")),
        };
        self.devices = devices::load(path.as_deref()).map_err(|e| bad(command, e))?;
        self.devices_file = path;
        self.changed(false);
        Ok(
            json!({"devices": self.devices.len(), "from_file": self.devices.iter().filter(|d| d.from_file).count()}),
        )
    }

    fn export_png(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is where the PNG goes"))?;
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err(bad(command, "`path` must end in .png"));
        }
        let c = self
            .coverage()
            .ok_or_else(|| bad(command, "nothing to export: open a plan (plan.open)"))?;
        let emitters = self.emitters();
        let bytes = crate::picture::png(&c, &emitters).map_err(|e| bad(command, e))?;
        std::fs::write(path, bytes).map_err(|e| bad(command, format!("{path}: {e}")))?;
        Ok(json!({"path": path}))
    }

    fn project_save(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is the project file"))?;
        let plan = self.plan.as_ref();
        let project = json!({
            "format": "radiocraft-project/1",
            "plan": plan.map(|p| p.path.clone()),
            "materials": plan.map(|p| p.sources.iter().map(|s| json!({"source": s.name, "kind": s.kind.name(), "material": s.material.to_json()})).collect::<Vec<_>>()),
            "routers": self.routers.iter().map(|r| {
                let power = match &r.power { Power::Device(id) => json!({"device": id}), Power::Eirp(e) => json!({"eirp_dbm": band_map(e)}) };
                json!({"name": r.name, "at": r.at, "height": r.height, "power": power})
            }).collect::<Vec<_>>(),
            "settings": {
                "band": self.settings.band.name(), "grid_step": self.settings.grid_step, "client_height": self.settings.client_height,
                "exponent": self.settings.exponent, "slice_height": self.settings.slice_height, "cap_eirp": self.settings.cap_eirp,
                "close_gaps": self.settings.close_gaps, "up": if self.settings.up == Up::Y { "y" } else { "z" },
            },
            "devices_file": self.devices_file,
        });
        let text =
            serde_json::to_string_pretty(&project).map_err(|e| bad(command, e.to_string()))?;
        std::fs::write(path, text).map_err(|e| bad(command, format!("{path}: {e}")))?;
        Ok(json!({"path": path}))
    }

    fn project_open(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is the project file"))?;
        let bytes = read_file(command, path)?;
        let project: Value =
            serde_json::from_slice(&bytes).map_err(|e| bad(command, format!("{path}: {e}")))?;
        if project.get("format").and_then(Value::as_str) != Some("radiocraft-project/1") {
            return Err(bad(command, format!("{path}: not a RadioCraft project")));
        }
        let mut fresh = Engine::default();
        if let Some(df) = project.get("devices_file").and_then(Value::as_str) {
            fresh.execute("devices.load", &json!({"path": df}))?;
        }
        if let Some(s) = project.get("settings") {
            fresh.execute("settings.set", s)?;
        }
        if let Some(plan) = project.get("plan").and_then(Value::as_str) {
            fresh.execute("plan.open", &json!({"path": plan}))?;
            for m in project
                .get("materials")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // A source the plan no longer has is skipped.
                let _ = fresh.execute("material.set", m);
            }
        }
        for r in project
            .get("routers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut params =
                json!({"at": r.get("at"), "height": r.get("height"), "name": r.get("name")});
            if let Some(power) = r.get("power").and_then(Value::as_object) {
                for (k, v) in power {
                    params[k] = v.clone();
                }
            }
            fresh.execute("router.add", &params)?;
        }
        *self = fresh;
        self.revision = self.revision.wrapping_add(1);
        Ok(json!({"path": path, "routers": self.routers.len()}))
    }
}

/// For each step of the scale, the share of the floor (0–100) at its level or better; for the
/// last, open step, the share below the one before it.
fn floor_share(values: &[f32], cells: &[Cell]) -> Vec<f64> {
    let floor: Vec<f64> = values
        .iter()
        .zip(cells)
        .filter(|(v, c)| **c == Cell::Floor && v.is_finite())
        .map(|(v, _)| f64::from(*v))
        .collect();
    let n = floor.len().max(1) as f64;
    let share = |keep: &dyn Fn(f64) -> bool| {
        (floor.iter().filter(|v| keep(**v)).count() as f64 / n * 1000.0).round() / 10.0
    };
    let scale = &model::SCALE;
    scale
        .iter()
        .enumerate()
        .map(|(i, (from, _))| {
            if from.is_finite() {
                share(&|v| v >= *from)
            } else {
                let above = scale
                    .get(i.wrapping_sub(1))
                    .map_or(f64::NEG_INFINITY, |(t, _)| *t);
                share(&|v| v < above)
            }
        })
        .collect()
}
