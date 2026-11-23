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
use crate::radio::{Channel, HEARD_DBM, View, default_width, link_rate};
use crate::views::{self, Derived, LegendRow};

/// The largest plan or project file read, in bytes.
const MAX_FILE_BYTES: u64 = 512 << 20;
/// How far a router may be from the origin (mm): 100 km.
const MAX_COORD: f64 = 1.0e8;
/// The most routers a plan has.
const MAX_ROUTERS: usize = 64;
/// How far past the floor's outline the leakage map looks (mm).
const LEAK_MM: f64 = 3000.0;
/// The most signals (cells × routers) kept at once: 64 MB.
const MAX_SIGNALS: usize = 16_000_000;

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
    /// Its channel in each band (2.4, 5, 6 GHz); `None` where it isn't set.
    pub channels: [Option<Channel>; 3],
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
    /// What the map shows.
    pub view: View,
    /// The receiving device's spatial streams, for the link rate.
    pub client_streams: u8,
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
            view: View::Signal,
            client_streams: 2,
        }
    }
}

/// Every router's signal over the grid for the shown band, and what each cell gets from them:
/// worked out once, then every map is drawn from it.
#[derive(Clone, Debug)]
pub struct Field {
    pub band: Band,
    pub grid: Grid,
    pub cells: Vec<Cell>,
    /// Where the floor came from: "enclosed" or "open".
    pub floor: &'static str,
    pub walls: Arc<WallMap>,
    /// `cell * routers + router` (dBm); NaN where not worked out.
    pub signals: Vec<f32>,
    pub names: Vec<String>,
    pub channels: Vec<Option<Channel>>,
    pub derived: Derived,
    /// Whether cells outside the floor were worked out too (for the leakage map).
    pub outside: bool,
}

/// What the analysis found for the shown band, drawn as the shown map.
#[derive(Clone, Debug)]
pub struct Coverage {
    pub band: Band,
    pub view: View,
    pub grid: Grid,
    pub cells: Vec<Cell>,
    /// The strongest signal at each cell (dBm); NaN where not worked out.
    pub values: Vec<f32>,
    /// The shown map: straight RGBA8, a pixel a cell, the first row the grid's lowest.
    pub rgba: Arc<Vec<u8>>,
    /// The shown map's key.
    pub legend: Vec<LegendRow>,
    pub stats: Option<model::Stats>,
    /// Where the floor came from: "enclosed" or "open".
    pub floor: &'static str,
    pub walls: Arc<WallMap>,
    pub field: Arc<Field>,
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
    field: Option<Arc<Field>>,
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
            field: None,
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
        "{at: [x, y]} → in each band: the strongest signal and its router, the three strongest routers with their channels, the link rate, and the routers on an overlapping channel heard there",
    ),
    (
        "coverage.compute",
        "{values?: bool, leakage?: bool} → the shown band's figures: coverage, second-router (roaming) cover, link rate, channel overlap; leakage: the area outside the walls with signal (values: every cell's signal)",
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
        "report.export",
        "{path: *.html, title?, notes?} → a self-contained HTML report: every band's figures and maps, the routers, the walls and the settings",
    ),
    (
        "router.add",
        "{at: [x, y], height?, name?, device? | eirp_dbm?: dBm | {\"2.4\",\"5\",\"6\"}, channels?: {\"2.4\": 6, \"5\": {channel: 36, width: 80}, ...}} → {id}",
    ),
    ("router.list", "{} → the routers"),
    ("router.move", "{id, at: [x, y]}"),
    ("router.remove", "{id}"),
    (
        "router.set",
        "{id, name?, height?, device? | eirp_dbm?, channels?: {band: channel | {channel, width} | null}}",
    ),
    (
        "settings.set",
        "{band?: 2.4|5|6, view?: signal|second|serving|overlap|rate|leakage, client_streams?: 1-4, grid_step?, client_height?, exponent?, slice_height?, cap_eirp?, close_gaps?, up?}",
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

fn channels_param(
    command: &str,
    v: &Value,
    mut current: [Option<Channel>; 3],
) -> Result<[Option<Channel>; 3]> {
    let m = v.as_object().ok_or_else(|| {
        bad(
            command,
            "`channels` is {\"2.4\": 6, \"5\": {\"channel\": 36, \"width\": 80}, ...}",
        )
    })?;
    for (k, v) in m {
        let band = Band::parse(k)
            .ok_or_else(|| bad(command, "`channels` keys are \"2.4\", \"5\" and \"6\""))?;
        let slot = current
            .get_mut(band.index())
            .ok_or_else(|| bad(command, "no such band"))?;
        *slot = if v.is_null() {
            None
        } else {
            Some(Channel::from_json(band, v).ok_or_else(|| {
                bad(
                    command,
                    format!(
                        "`channels.{k}`: {v} isn't a {k} GHz channel (widths: {:?} MHz)",
                        crate::radio::widths(band)
                    ),
                )
            })?)
        };
    }
    Ok(current)
}

fn channels_json(c: &[Option<Channel>; 3]) -> Value {
    let mut m = serde_json::Map::new();
    for b in Band::ALL {
        if let Some(Some(ch)) = c.get(b.index()) {
            m.insert(b.name().to_string(), ch.to_json());
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
        self.field = None;
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
            "report.export" => self.report_export(command, p),
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
        let channels = match p.get("channels") {
            None | Some(Value::Null) => [None; 3],
            Some(v) => channels_param(command, v, [None; 3])?,
        };
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
            channels,
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
        let channels = match (p.get("channels"), self.routers.get(i)) {
            (Some(v), Some(r)) if !v.is_null() => Some(channels_param(command, v, r.channels)?),
            _ => None,
        };
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
            if let Some(c) = channels {
                r.channels = c;
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
        json!({"id": r.id, "name": r.name, "at": r.at, "height": r.height, "device": device, "eirp_dbm": band_map(&eirp), "channels": channels_json(&r.channels)})
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
        if let Some(v) = p.get("view").filter(|v| !v.is_null()) {
            s.view = v.as_str().and_then(View::parse).ok_or_else(|| {
                bad(
                    command,
                    "`view` is signal, second, serving, overlap, rate or leakage",
                )
            })?;
        }
        if let Some(v) = ranged(command, p, "client_streams", 1.0, 4.0)? {
            if v.fract() != 0.0 {
                return Err(bad(command, "`client_streams` is 1, 2, 3 or 4"));
            }
            s.client_streams = v as u8;
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
        // Another map of the same signals is only drawn again.
        let only_view = Settings {
            view: self.settings.view,
            ..s.clone()
        } == self.settings;
        self.settings = s;
        if only_view {
            self.revision = self.revision.wrapping_add(1);
            self.coverage = None;
            return Ok(self.settings_json());
        }
        // A model is cut again at the new height; a DXF plan has no height.
        if recut && let Some(path) = self.plan.as_ref().map(|p| p.path.clone()) {
            let lower = path.to_ascii_lowercase();
            if lower.ends_with(".obj") || lower.ends_with(".stl") {
                self.plan_open(command, &json!({"path": path}))?;
            }
        }
        self.changed(true);
        Ok(self.settings_json())
    }

    fn settings_json(&self) -> Value {
        let s = &self.settings;
        json!({
            "band": s.band.name(),
            "view": s.view.id(),
            "client_streams": s.client_streams,
            "grid_step": s.grid_step,
            "client_height": s.client_height,
            "exponent": s.exponent,
            "slice_height": s.slice_height,
            "cap_eirp": s.cap_eirp,
            "close_gaps": s.close_gaps,
            "up": if s.up == Up::Y { "y" } else { "z" },
        })
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

    /// Every router's signal over the grid for the shown band, worked out once until something
    /// changes; `outside` also works out the cells around the floor (for the leakage map).
    fn field(&mut self, outside: bool) -> Option<Arc<Field>> {
        if let Some(f) = &self.field
            && (f.outside || !outside)
        {
            return Some(f.clone());
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
        // The leakage map pads the same grid with whole cells, all outside the floor, so the
        // floor's cells, and its figures, are the same in every map.
        let (grid, cells) = if outside {
            let k = (LEAK_MM / s.grid_step).ceil() as usize;
            let big = Grid {
                origin: [
                    grid.origin[0] - k as f64 * grid.step,
                    grid.origin[1] - k as f64 * grid.step,
                ],
                step: grid.step,
                columns: grid.columns + 2 * k,
                rows: grid.rows + 2 * k,
            };
            let mut padded = vec![Cell::Outside; big.len()];
            for row in 0..grid.rows {
                for column in 0..grid.columns {
                    if let (Some(slot), Some(c)) = (
                        padded.get_mut((row + k) * big.columns + column + k),
                        cells.get(row * grid.columns + column),
                    ) {
                        *slot = *c;
                    }
                }
            }
            if big.len() > model::MAX_CELLS {
                return None;
            }
            (big, padded)
        } else {
            (grid, cells)
        };
        if grid.len().saturating_mul(self.routers.len().max(1)) > MAX_SIGNALS {
            return None;
        }
        let emitters = self.emitters();
        let keep = |i: usize| match cells.get(i) {
            Some(Cell::Floor) => true,
            Some(Cell::Outside) => outside,
            _ => false,
        };
        let signals = model::signals(&walls, &emitters, &self.model(), s.band, &grid, &keep);
        let channels: Vec<Option<Channel>> = self
            .routers
            .iter()
            .map(|r| r.channels.get(s.band.index()).copied().flatten())
            .collect();
        let derived = views::derive(
            &signals,
            emitters.len(),
            &channels,
            s.band,
            s.client_streams,
        );
        let derived = if emitters.is_empty() {
            Derived {
                best: vec![f32::NAN; grid.len()],
                second: vec![f32::NAN; grid.len()],
                serving: vec![-1; grid.len()],
                overlap: vec![-1; grid.len()],
                rate: vec![f32::NAN; grid.len()],
            }
        } else {
            derived
        };
        let f = Arc::new(Field {
            band: s.band,
            grid,
            cells,
            floor,
            walls,
            signals,
            names: emitters.iter().map(|e| e.name.clone()).collect(),
            channels,
            derived,
            outside,
        });
        self.field = Some(f.clone());
        Some(f)
    }

    /// The shown band's coverage drawn as the shown map, worked out once until something
    /// changes. `None` without a plan or with nothing to analyse.
    pub fn coverage(&mut self) -> Option<Arc<Coverage>> {
        if let Some(c) = &self.coverage {
            return Some(c.clone());
        }
        let view = self.settings.view;
        let f = self.field(view == View::Leakage)?;
        let (rgba, legend) = views::picture(view, &f.derived, &f.cells, &f.names);
        let stats = model::stats(&f.derived.best, &f.cells);
        let c = Arc::new(Coverage {
            band: f.band,
            view,
            grid: f.grid.clone(),
            cells: f.cells.clone(),
            values: f.derived.best.clone(),
            rgba: Arc::new(rgba),
            legend,
            stats,
            floor: f.floor,
            walls: f.walls.clone(),
            field: f,
        });
        self.coverage = Some(c.clone());
        Some(c)
    }

    fn coverage_compute(&mut self, command: &str, p: &Value) -> Result<Value> {
        if self.plan.is_none() {
            return Err(bad(command, "no plan is open (plan.open)"));
        }
        let c = self.coverage().ok_or_else(|| bad(command, "nothing to analyse: every layer or object is ignored or none is closed, or the grid is too fine for the plan and its routers (raise grid_step)"))?;
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
            let d = &c.field.derived;
            let on_floor: Vec<usize> = (0..c.cells.len())
                .filter(|i| c.cells.get(*i) == Some(&Cell::Floor))
                .collect();
            let n = on_floor.len().max(1) as f64;
            let pct = |keep: &dyn Fn(usize) -> bool| {
                round(on_floor.iter().filter(|i| keep(**i)).count() as f64 / n * 100.0)
            };
            if self.routers.len() > 1 {
                out["pct_second_at_least_67"] = json!(pct(&|i| d
                    .second
                    .get(i)
                    .is_some_and(|v| f64::from(*v) >= model::FAIR_DBM)));
            }
            let mut rates: Vec<f64> = on_floor
                .iter()
                .filter_map(|i| d.rate.get(*i).map(|r| f64::from(*r)))
                .filter(|r| r.is_finite())
                .collect();
            rates.sort_by(f64::total_cmp);
            if let Some(m) = rates.get(rates.len() / 2) {
                out["median_rate_mbps"] = json!(round(*m));
            }
            if c.field.channels.iter().any(Option::is_some) {
                out["pct_no_channel_overlap"] =
                    json!(pct(&|i| d.overlap.get(i).copied() == Some(0)));
                out["pct_channel_unset"] = json!(pct(&|i| d.overlap.get(i).copied() == Some(-1)));
            }
        } else {
            out["note"] = json!("no router has a radio in this band: add a router (router.add)");
        }
        if p.get("leakage").and_then(Value::as_bool).unwrap_or(false) || c.view == View::Leakage {
            let f = self
                .field(true)
                .ok_or_else(|| bad(command, "nothing to analyse"))?;
            let outside: Vec<usize> = (0..f.cells.len())
                .filter(|i| f.cells.get(*i) == Some(&Cell::Outside))
                .collect();
            let m2 = |t: f64| {
                round(
                    outside
                        .iter()
                        .filter(|i| f.derived.best.get(**i).is_some_and(|v| f64::from(*v) >= t))
                        .count() as f64
                        * cell_m2,
                )
            };
            out["leakage"] = if f.floor == "open" {
                json!({"note": "the floor isn't enclosed, so nothing is outside it"})
            } else {
                json!({"within_mm": LEAK_MM, "m2_at_least_67": m2(model::FAIR_DBM), "m2_at_least_75": m2(-75.0)})
            };
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
        let round = |v: f64| (v * 10.0).round() / 10.0;
        let mut scratch = Scratch::default();
        let mut out = serde_json::Map::new();
        for band in Band::ALL {
            // The same per-router signals the maps are drawn from.
            let mut heard: Vec<(f64, usize)> = emitters
                .iter()
                .enumerate()
                .filter_map(|(k, e)| {
                    model::signal_at(&walls, e, &model, band, at, &mut scratch).map(|v| (v, k))
                })
                .collect();
            heard.sort_by(|a, b| b.0.total_cmp(&a.0));
            let Some(&(best, k)) = heard.first() else {
                continue;
            };
            let channel = |j: usize| {
                self.routers
                    .get(j)
                    .and_then(|r| r.channels.get(band.index()).copied().flatten())
            };
            let width = channel(k).map_or(default_width(band), |c| c.width);
            let rate = link_rate(best, width, self.settings.client_streams);
            let from = emitters.get(k);
            let interferers: Vec<Value> = match channel(k) {
                Some(own) => heard
                    .iter()
                    .skip(1)
                    .filter(|(v, j)| *v >= HEARD_DBM && channel(*j).is_some_and(|c| c.overlaps(own, band)))
                    .map(|(v, j)| json!({"name": emitters.get(*j).map(|e| e.name.clone()), "rssi_dbm": round(*v), "channel": channel(*j).map(Channel::to_json)}))
                    .collect(),
                None => Vec::new(),
            };
            out.insert(
                band.name().to_string(),
                json!({
                    "rssi_dbm": round(best),
                    "from": from.map(|e| e.name.clone()),
                    "walls_db": from.map(|e| round(walls.path_loss(e.at, at, band, &mut scratch))),
                    "routers": heard.iter().take(3).map(|(v, j)| json!({
                        "name": emitters.get(*j).map(|e| e.name.clone()),
                        "rssi_dbm": round(*v),
                        "channel": channel(*j).map(Channel::to_json),
                    })).collect::<Vec<_>>(),
                    "mcs": rate.map(|r| r.0),
                    "rate_mbps": rate.map_or(0.0, |r| r.1),
                    "channel_overlap": channel(k).map(|_| interferers.len()),
                    "interferers": interferers,
                }),
            );
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

    fn report_export(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is where the report goes"))?
            .to_string();
        let lower = path.to_ascii_lowercase();
        if !(lower.ends_with(".html") || lower.ends_with(".htm")) {
            return Err(bad(command, "`path` must end in .html"));
        }
        let text = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| t.chars().take(20_000).collect::<String>())
        };
        let plan = self
            .plan
            .as_ref()
            .ok_or_else(|| bad(command, "no plan is open (plan.open)"))?;
        // Only the file's name: a report is often shared, and folders say who made it.
        let plan_name = std::path::Path::new(&plan.path)
            .file_name()
            .map_or_else(|| plan.path.clone(), |n| n.to_string_lossy().to_string());
        let counts = plan.shape_counts();
        let materials: Vec<[String; 3]> = plan
            .sources
            .iter()
            .enumerate()
            .filter(|(i, _)| counts.get(i).copied().unwrap_or(0) > 0)
            .map(|(_, s)| {
                let m = match &s.material {
                    Material::Auto => "Guessed from thickness".to_string(),
                    Material::Ignore => "Not a wall".to_string(),
                    Material::Preset(id) => id.to_string(),
                    Material::Custom(l) => format!("{:?} dB", l.0),
                };
                [s.name.clone(), s.kind.name().to_string(), m]
            })
            .collect();
        let m = |mm: f64| format!("{:.2}", mm / 1000.0);
        let routers: Vec<[String; 6]> = self
            .routers
            .iter()
            .map(|r| {
                let device = match &r.power {
                    Power::Device(id) => self
                        .devices
                        .iter()
                        .find(|d| d.id == *id)
                        .map_or_else(|| id.clone(), |d| d.name.clone()),
                    Power::Eirp(_) => "Typed power".to_string(),
                };
                let eirp = self.eirp(r);
                let per_band = |f: &dyn Fn(Band) -> Option<String>| {
                    Band::ALL
                        .iter()
                        .filter_map(|b| f(*b).map(|v| format!("{} GHz {v}", b.name())))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                [
                    r.name.clone(),
                    format!("{}, {}", m(r.at[0]), m(r.at[1])),
                    m(r.height),
                    device,
                    per_band(&|b| {
                        eirp.get(b.index())
                            .copied()
                            .flatten()
                            .map(|v| format!("{v:.1} dBm"))
                    }),
                    per_band(&|b| {
                        r.channels
                            .get(b.index())
                            .copied()
                            .flatten()
                            .map(|c| format!("{} ({} MHz)", c.number, c.width))
                    }),
                ]
            })
            .collect();
        let saved = self.settings.clone();
        let emitters = self.emitters();
        let mut bands = Vec::new();
        for band in Band::ALL {
            if !emitters.iter().any(|e| e.eirp(band).is_some()) {
                continue;
            }
            self.settings.band = band;
            self.settings.view = View::Signal;
            self.changed(false);
            // The leakage figure (and its map) come last: they need the wider grid, and the other
            // maps are drawn on the usual one.
            let figures = match self.coverage_compute(command, &json!({})) {
                Ok(f) => f,
                Err(e) => {
                    self.settings = saved;
                    self.changed(false);
                    return Err(e);
                }
            };
            let pct = |k: &str| figures[k].as_f64().map(|v| format!("{v:.0}%"));
            let dbm = |k: &str| figures[k].as_f64().map(|v| format!("{v:.0} dBm"));
            let mut rows: Vec<(String, String)> = [
                (
                    "Floor",
                    figures["floor_m2"].as_f64().map(|v| format!("{v:.1} m²")),
                ),
                ("−60 dBm or better (fast)", pct("pct_at_least_60")),
                ("−67 dBm or better (video calls)", pct("pct_at_least_67")),
                ("Weakest 5%", dbm("p5_dbm")),
                ("Median", dbm("median_dbm")),
                (
                    "Roaming: a second router at −67 dBm or better",
                    pct("pct_second_at_least_67"),
                ),
                (
                    "Median link rate",
                    figures["median_rate_mbps"]
                        .as_f64()
                        .map(|v| format!("{v:.0} Mbit/s")),
                ),
                (
                    "No overlapping channel heard",
                    pct("pct_no_channel_overlap"),
                ),
            ]
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
            .collect();
            let several = self.routers.len() > 1;
            let channels = self
                .routers
                .iter()
                .any(|r| r.channels.get(band.index()).copied().flatten().is_some());
            let mut maps = Vec::new();
            for view in View::ALL {
                let wanted = match view {
                    View::Second | View::Serving => several,
                    View::Overlap => channels,
                    View::Leakage => figures["floor"] == "enclosed",
                    View::Signal | View::Rate => true,
                };
                if !wanted {
                    continue;
                }
                self.settings.view = view;
                self.coverage = None;
                if view == View::Leakage
                    && let Ok(f) = self.coverage_compute(command, &json!({"leakage": true}))
                    && let Some(v) = f["leakage"]["m2_at_least_67"].as_f64()
                {
                    rows.push((
                        "Outside the walls (within 3 m) at −67 dBm or better".to_string(),
                        format!("{v:.1} m²"),
                    ));
                }
                let Some(c) = self.coverage() else { continue };
                let png = match crate::picture::png(&c, &emitters) {
                    Ok(png) => png,
                    Err(e) => {
                        self.settings = saved;
                        self.changed(false);
                        return Err(bad(command, e));
                    }
                };
                maps.push(crate::report::Map {
                    title: view.label().to_string(),
                    png,
                    key_note: match view {
                        View::Serving | View::Overlap => "Share of the floor".to_string(),
                        View::Leakage => "Share of the area within 3 m outside the walls, at each level or better".to_string(),
                        View::Rate => "Share of the floor at each rate or better; the most the signal allows with nothing else on the air".to_string(),
                        _ => "Share of the floor at each level or better".to_string(),
                    },
                    legend: c.legend.clone(),
                });
            }
            bands.push(crate::report::BandSection {
                band: band.name().to_string(),
                figures: rows,
                maps,
            });
        }
        self.settings = saved;
        self.changed(false);
        let s = &self.settings;
        let report = crate::report::Report {
            title: text("title").unwrap_or_else(|| "Wi-Fi coverage".to_string()),
            notes: text("notes"),
            plan: plan_name,
            routers,
            materials,
            settings: vec![
                ("Device height".into(), format!("{} m", m(s.client_height))),
                ("Path-loss exponent".into(), format!("{}", s.exponent)),
                ("Grid".into(), format!("{} mm", s.grid_step)),
                (
                    "Device spatial streams".into(),
                    s.client_streams.to_string(),
                ),
                (
                    "Power capped at the usual limits".into(),
                    if s.cap_eirp { "yes" } else { "no" }.into(),
                ),
                (
                    "Openings counted as closed below".into(),
                    format!("{} m", m(s.close_gaps)),
                ),
                ("Model cut at".into(), format!("{} m", m(s.slice_height))),
            ],
            bands,
        };
        let html = crate::report::html(&report);
        std::fs::write(&path, &html).map_err(|e| bad(command, format!("{path}: {e}")))?;
        Ok(
            json!({"path": path, "bands": report.bands.len(), "maps": report.bands.iter().map(|b| b.maps.len()).sum::<usize>(), "bytes": html.len()}),
        )
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
                json!({"name": r.name, "at": r.at, "height": r.height, "power": power, "channels": channels_json(&r.channels)})
            }).collect::<Vec<_>>(),
            // Fields added after the first beta (view, client_streams, channels) are optional
            // when read, so older projects open unchanged: the format name stays.
            "settings": self.settings_json(),
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
            let mut params = json!({"at": r.get("at"), "height": r.get("height"), "name": r.get("name"), "channels": r.get("channels")});
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
