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
use crate::survey::{self, Point};
use crate::viewpoint::{Basis, PlanView};
use crate::views::{self, Derived, LegendRow};

/// The largest plan or project file read, in bytes.
const MAX_FILE_BYTES: u64 = 512 << 20;
/// How far a router may be from the origin (mm): 100 km.
const MAX_COORD: f64 = 1.0e8;
/// The most routers a plan has.
const MAX_ROUTERS: usize = 64;
/// How far past the plan's outer edge (its bounding box) the leakage map looks (mm).
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
    /// Added to every signal in each band (dB): a calibration to measurements.
    pub offset_db: [f64; 3],
    /// Which way the plan is looked at: `top`, `front`… or `view:<name>` (a model's saved view,
    /// a drawing's named view).
    pub viewpoint: String,
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
            offset_db: [0.0; 3],
            viewpoint: "top".into(),
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
    /// Measured points.
    survey: Vec<Point>,
    /// The last SolidCraft design exported: its path, modification time, size and export.
    sc_cache: Option<(
        String,
        Option<std::time::SystemTime>,
        u64,
        Arc<crate::sc::Converted>,
    )>,
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
            survey: Vec::new(),
            sc_cache: None,
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
        "{path, slice_height?, up?: z|y, viewpoint?} → read a DXF plan (CADCraft), cut an OBJ/STL model or a SolidCraft design (.sc, through SolidCraft's solidcraft-cli) along the viewpoint",
    ),
    (
        "project.new",
        "{} → start again: no plan, routers or measurements (loaded devices stay)",
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
        "{at: [x, y], height?, name?, device? | eirp_dbm?: dBm | {\"2.4\",\"5\",\"6\"}, channels?: {\"2.4\": 6, \"5\": {channel: 36, width: 80}, ...} (a bare number is 20 MHz; no width: 20 MHz on 2.4, 80 on 5 and 6)} → {id}",
    ),
    ("router.list", "{} → the routers"),
    (
        "router.suggest",
        "{count?: 1-8, device? | eirp_dbm?, height?, channels?, add?: bool} → spots for more routers in the shown band, each raising the floor at −67 dBm or better the most; add: place them",
    ),
    ("router.move", "{id, at: [x, y]}"),
    ("router.remove", "{id}"),
    (
        "router.set",
        "{id, name?, height?, device? | eirp_dbm?, channels?: {band: channel | {channel, width} | null}}",
    ),
    (
        "survey.add",
        "{at: [x, y], band, rssi_dbm, router?} → add one measured point (router: the one it was measured from; none: the strongest)",
    ),
    (
        "survey.calibrate",
        "{apply?: bool} → fit the path-loss exponent and a per-band offset to the measured points; the error before and after; apply: use them",
    ),
    ("survey.clear", "{} → forget the measured points"),
    (
        "survey.compare",
        "{points?: bool} → measured against predicted for each band: mean error, RMS, largest (points: each one)",
    ),
    (
        "survey.import",
        "{path: *.csv | *.json} → read measured points (CSV columns x, y, band, rssi_dbm, router?; mm and dBm), replacing any",
    ),
    (
        "settings.set",
        "{band?: 2.4|5|6, view?: signal|second|serving|overlap|rate|leakage, viewpoint?: top|bottom|front|back|left|right|view:<name> (plan.info lists the plan's), client_streams?: 1-4, offset_db?: {band: dB}, grid_step?, client_height?, exponent?, slice_height?, cap_eirp?, close_gaps?, up?}",
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

    pub fn survey(&self) -> &[Point] {
        &self.survey
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
            "router.suggest" => self.router_suggest(command, p),
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
            "project.new" => {
                let devices = (self.devices.clone(), self.devices_file.clone());
                let revision = self.revision;
                *self = Engine::default();
                (self.devices, self.devices_file) = devices;
                self.revision = revision.wrapping_add(1);
                Ok(json!({}))
            }
            "report.export" => self.report_export(command, p),
            "survey.import" => self.survey_import(command, p),
            "survey.add" => {
                let point = survey::from_json(p).map_err(|e| bad(command, e))?;
                if self.survey.len() >= survey::MAX_POINTS {
                    return Err(bad(command, format!("at most {} points", survey::MAX_POINTS)));
                }
                self.survey.push(point);
                self.revision = self.revision.wrapping_add(1);
                Ok(json!({"points": self.survey.len()}))
            }
            "survey.clear" => {
                self.survey.clear();
                self.revision = self.revision.wrapping_add(1);
                Ok(json!({"points": 0}))
            }
            "survey.compare" => self.survey_compare(command, p),
            "survey.calibrate" => self.survey_calibrate(command, p),
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
        if let Some(v) = p.get("viewpoint").and_then(Value::as_str) {
            self.settings.viewpoint = v.trim().to_string();
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

    fn read_plan(&mut self, command: &str, path: &str) -> Result<FloorPlan> {
        let lower = path.to_ascii_lowercase();
        let wanted = self.settings.viewpoint.clone();
        // The viewpoint asked for, among those the file offers; the top when it has none such.
        let pick = |views: &[PlanView]| -> (String, Basis) {
            views
                .iter()
                .find(|v| v.id == wanted)
                .or_else(|| views.iter().find(|v| v.id == "top"))
                .map_or(("top".to_string(), Basis::TOP), |v| {
                    (v.id.clone(), v.basis.unwrap_or(Basis::TOP))
                })
        };
        let mut plan = if lower.ends_with(".sc") {
            let converted = self.sc_converted(command, path)?;
            let mut views = crate::viewpoint::model_views();
            views.extend(converted.views.iter().cloned());
            let (_, basis) = pick(&views);
            let mut plan = read_mesh(
                &converted.obj,
                "design.obj",
                self.settings.slice_height,
                Up::Z,
                &basis,
            )
            .map_err(|e| bad(command, format!("{path}: {e}")))?;
            plan.path = path.to_string();
            plan.views = views;
            plan.notes.extend(converted.notes.iter().cloned());
            plan
        } else {
            let bytes = read_file(command, path)?;
            if lower.ends_with(".dxf") {
                read_dxf(&bytes, path).map_err(|e| bad(command, format!("{path}: {e}")))?
            } else if lower.ends_with(".obj") || lower.ends_with(".stl") {
                let (_, basis) = pick(&crate::viewpoint::model_views());
                read_mesh(
                    &bytes,
                    path,
                    self.settings.slice_height,
                    self.settings.up,
                    &basis,
                )
                .map_err(|e| bad(command, format!("{path}: {e}")))?
            } else {
                return Err(bad(
                    command,
                    format!(
                        "{path}: RadioCraft reads DXF plans (CADCraft), OBJ or STL models, and SolidCraft designs (.sc)"
                    ),
                ));
            }
        };
        let (id, _) = pick(&plan.views);
        if id != wanted {
            plan.notes.push(format!(
                "this plan has no viewpoint `{wanted}`; it is seen from the top"
            ));
        }
        self.settings.viewpoint = id;
        Ok(plan)
    }

    /// A SolidCraft design exported by SolidCraft, once per version of the file.
    fn sc_converted(&mut self, command: &str, path: &str) -> Result<Arc<crate::sc::Converted>> {
        let meta = std::fs::metadata(path).map_err(|e| bad(command, format!("{path}: {e}")))?;
        if !meta.is_file() {
            return Err(bad(command, format!("{path}: not a file")));
        }
        let (mtime, len) = (meta.modified().ok(), meta.len());
        if let Some((p, m, l, c)) = &self.sc_cache
            && p == path
            && *m == mtime
            && *l == len
        {
            return Ok(c.clone());
        }
        let converted = Arc::new(
            crate::sc::convert(std::path::Path::new(path))
                .map_err(|e| bad(command, format!("{path}: {e}")))?,
        );
        self.sc_cache = Some((path.to_string(), mtime, len, converted.clone()));
        Ok(converted)
    }

    /// Whether the open plan is a model (cut along the viewpoint) rather than a drawing.
    fn plan_is_model(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| {
            let l = p.path.to_ascii_lowercase();
            l.ends_with(".obj") || l.ends_with(".stl") || l.ends_with(".sc")
        })
    }

    /// The open plan's viewpoint.
    fn plan_view(&self) -> Option<&PlanView> {
        self.plan
            .as_ref()
            .and_then(|p| p.views.iter().find(|v| v.id == self.settings.viewpoint))
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
            "viewpoints": plan.views.iter().map(PlanView::to_json).collect::<Vec<_>>(),
            "viewpoint": self.settings.viewpoint,
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

    fn router_suggest(&mut self, command: &str, p: &Value) -> Result<Value> {
        let count = ranged(command, p, "count", 1.0, 8.0)?.unwrap_or(1.0);
        if count.fract() != 0.0 {
            return Err(bad(command, "`count` is 1 to 8"));
        }
        if self.routers.len() + count as usize > MAX_ROUTERS {
            return Err(bad(command, format!("at most {MAX_ROUTERS} routers")));
        }
        let height = ranged(command, p, "height", -MAX_COORD, MAX_COORD)?
            .or_else(|| self.routers.last().map(|r| r.height))
            .unwrap_or(2400.0);
        let power = self.power(command, p)?.unwrap_or_else(|| {
            Power::Device(
                self.devices
                    .first()
                    .map(|d| d.id.clone())
                    .unwrap_or_default(),
            )
        });
        let channels = match p.get("channels") {
            None | Some(Value::Null) => [None; 3],
            Some(v) => channels_param(command, v, [None; 3])?,
        };
        let add = p.get("add").and_then(Value::as_bool).unwrap_or(false);
        let probe = Router {
            id: 0,
            name: String::new(),
            at: [0.0, 0.0],
            height,
            power: power.clone(),
            channels,
        };
        let band = self.settings.band;
        let template = Emitter {
            name: "suggested".into(),
            at: [0.0, 0.0],
            height,
            eirp_dbm: self.eirp(&probe),
        };
        if template.eirp(band).is_none() {
            return Err(bad(
                command,
                format!("that device has no {} GHz radio", band.name()),
            ));
        }
        let f = self.field(false).ok_or_else(|| {
            bad(
                command,
                "nothing to analyse: open a plan with walls (plan.open)",
            )
        })?;
        let spots = crate::placement::suggest(
            &f.walls,
            &f.grid,
            &f.cells,
            &self.emitters(),
            &template,
            &self.model(),
            band,
            count as usize,
        );
        if spots.is_empty() {
            return Err(bad(command, "the plan has no floor to place a router on"));
        }
        let mut out = Vec::new();
        for (i, s) in spots.iter().enumerate() {
            let at = [s.at[0].round(), s.at[1].round()];
            let mut row = json!({"at": at, "pct_at_least_67": s.pct_at_least_67});
            if add {
                let mut params = json!({"at": at, "height": height, "name": format!("Suggested {}", i + 1), "channels": channels_json(&channels)});
                match &power {
                    Power::Device(id) => params["device"] = json!(id),
                    Power::Eirp(e) => params["eirp_dbm"] = band_map(e),
                }
                row["id"] = self.router_add(command, &params)?["id"].clone();
            }
            out.push(row);
        }
        Ok(json!({"band": band.name(), "spots": out}))
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
        if let Some(v) = p.get("offset_db").filter(|v| !v.is_null()) {
            let m = v
                .as_object()
                .ok_or_else(|| bad(command, "`offset_db` is {\"2.4\", \"5\", \"6\": dB}"))?;
            for (k, v) in m {
                let band = Band::parse(k)
                    .ok_or_else(|| bad(command, "`offset_db` keys are \"2.4\", \"5\" and \"6\""))?;
                let x = v
                    .as_f64()
                    .filter(|x| x.is_finite() && (-30.0..=30.0).contains(x))
                    .ok_or_else(|| bad(command, "an offset is -30 to 30 dB"))?;
                if let Some(slot) = s.offset_db.get_mut(band.index()) {
                    *slot = x;
                }
            }
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
        if let Some(v) = p.get("viewpoint").filter(|v| !v.is_null()) {
            let id = v.as_str().map(str::trim).unwrap_or_default().to_string();
            let known = match &self.plan {
                Some(plan) => plan.views.iter().any(|w| w.id == id),
                None => Basis::standard(&id).is_some() || id.starts_with("view:"),
            };
            if !known {
                return Err(bad(
                    command,
                    format!("no viewpoint `{id}` (plan.info lists the plan's viewpoints)"),
                ));
            }
            s.viewpoint = id;
        }
        let new_slice = ranged(command, p, "slice_height", -MAX_COORD, MAX_COORD)?;
        let new_view = s.viewpoint != self.settings.viewpoint;
        let recut =
            new_slice.is_some_and(|h| h != s.slice_height) || s.up != self.settings.up || new_view;
        // Routers stay where they are in the building when a model is seen from another side.
        let moved_view = (new_view && self.plan_is_model()).then(|| {
            let views = self
                .plan
                .as_ref()
                .map(|p| p.views.clone())
                .unwrap_or_default();
            let basis = |id: &str| views.iter().find(|v| v.id == id).and_then(|v| v.basis);
            (basis(&self.settings.viewpoint), basis(&s.viewpoint))
        });
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
        // A model is cut again at the new height or from the new side; a drawing has no height,
        // and its named views only frame it.
        if recut
            && self.plan_is_model()
            && let Some(path) = self.plan.as_ref().map(|p| p.path.clone())
        {
            self.plan_open(command, &json!({"path": path}))?;
        }
        if let Some((Some(from), Some(to))) = moved_view {
            for r in &mut self.routers {
                let v = to.to_view(from.to_world([r.at[0], r.at[1], r.height]));
                r.at = [v[0], v[1]];
                r.height = v[2];
            }
        }
        self.changed(true);
        let mut out = self.settings_json();
        if new_view && !self.survey.is_empty() && self.settings.viewpoint != "top" {
            out["note"] = json!(
                "measured points are positions on the floor plan; they don't follow a side view"
            );
        }
        Ok(out)
    }

    fn settings_json(&self) -> Value {
        let s = &self.settings;
        json!({
            "band": s.band.name(),
            "view": s.view.id(),
            "client_streams": s.client_streams,
            "offset_db": band_map(&s.offset_db.map(Some)),
            "viewpoint": s.viewpoint,
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
            offset_db: self.settings.offset_db,
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
        // A drawing's named view frames part of it: the map covers that part (the walls outside
        // it still count on the way).
        let bounds = match self.plan_view().and_then(|v| v.window) {
            Some(w) => w,
            None => walls.bounds()?,
        };
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
            // Out to LEAK_MM past the plan's outer edge, counting the usual grid's own margin.
            let k = ((LEAK_MM - margin) / s.grid_step).ceil().max(0.0) as usize;
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
            if !self.survey.is_empty() {
                let rows_m = self.survey_terms();
                let e = Self::errors_json(&rows_m);
                if let (Some(n), Some(mean), Some(rms)) = (
                    e[band.name()]["points"].as_u64(),
                    e[band.name()]["mean_error_db"].as_f64(),
                    e[band.name()]["rms_db"].as_f64(),
                ) {
                    rows.push((
                        "Measured points: measured − predicted".to_string(),
                        format!("{n} points, mean {mean:+.1} dB, RMS {rms:.1} dB"),
                    ));
                }
            }
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
                        "Outside the walls, within 3 m of the plan's edge, at −67 dBm or better"
                            .to_string(),
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
                        View::Leakage => "Share of the area outside the walls, within 3 m of the plan's edge, at each level or better (wall cavities and unenclosed spaces inside the plan count too)".to_string(),
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
                (
                    "Calibration offset".into(),
                    Band::ALL
                        .iter()
                        .map(|b| {
                            format!(
                                "{} GHz {:+.1} dB",
                                b.name(),
                                s.offset_db.get(b.index()).copied().unwrap_or(0.0)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ],
            bands,
        };
        let html = crate::report::html(&report);
        std::fs::write(&path, &html).map_err(|e| bad(command, format!("{path}: {e}")))?;
        Ok(
            json!({"path": path, "bands": report.bands.len(), "maps": report.bands.iter().map(|b| b.maps.len()).sum::<usize>(), "bytes": html.len()}),
        )
    }

    fn survey_import(&mut self, command: &str, p: &Value) -> Result<Value> {
        let path = p
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(command, "`path` is a CSV or JSON file of measured points"))?;
        let lower = path.to_ascii_lowercase();
        let json = lower.ends_with(".json");
        if !json && !lower.ends_with(".csv") {
            return Err(bad(command, "`path` must end in .csv or .json"));
        }
        let bytes = read_file(command, path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| bad(command, format!("{path}: not UTF-8 text")))?;
        self.survey =
            survey::parse(&text, json).map_err(|e| bad(command, format!("{path}: {e}")))?;
        self.revision = self.revision.wrapping_add(1);
        Ok(json!({"points": self.survey.len()}))
    }

    /// For each measured point: its band, the measured signal, the router it is compared with
    /// (the one it names, or the strongest predicted), and that router's predicted signal and
    /// fit terms. Points naming a router that isn't there, or with no router in their band, are
    /// left out.
    fn survey_terms(&mut self) -> Vec<(usize, f64, f64, survey::Term)> {
        let Some(walls) = self.walls() else {
            return Vec::new();
        };
        let emitters = self.emitters();
        let model = self.model();
        let mut scratch = Scratch::default();
        let mut out = Vec::new();
        for (i, pt) in self.survey.iter().enumerate() {
            let pick = match &pt.router {
                Some(name) => emitters.iter().find(|e| e.name == *name).and_then(|e| {
                    model::signal_at(&walls, e, &model, pt.band, pt.at, &mut scratch)
                        .map(|v| (v, e))
                }),
                None => emitters
                    .iter()
                    .filter_map(|e| {
                        model::signal_at(&walls, e, &model, pt.band, pt.at, &mut scratch)
                            .map(|v| (v, e))
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0)),
            };
            let Some((predicted, e)) = pick else { continue };
            let Some(eirp) = e.eirp(pt.band) else {
                continue;
            };
            let dz = e.height - model.client_height;
            let d = (((pt.at[0] - e.at[0]).powi(2) + (pt.at[1] - e.at[1]).powi(2) + dz * dz)
                .sqrt()
                / 1000.0)
                .max(model::MIN_DISTANCE_M);
            let rest = pt.rssi_dbm - eirp
                + model::fspl_1m(pt.band)
                + walls.path_loss(e.at, pt.at, pt.band, &mut scratch);
            out.push((
                i,
                pt.rssi_dbm,
                predicted,
                survey::Term {
                    band: pt.band,
                    log_d: d.log10(),
                    rest,
                },
            ));
        }
        out
    }

    fn errors_json(rows: &[(usize, f64, f64, survey::Term)]) -> Value {
        let round = |v: f64| (v * 10.0).round() / 10.0;
        let mut m = serde_json::Map::new();
        for band in Band::ALL {
            let diffs: Vec<f64> = rows
                .iter()
                .filter(|r| r.3.band == band)
                .map(|r| r.1 - r.2)
                .collect();
            if let Some(e) = survey::errors(&diffs) {
                m.insert(
                    band.name().to_string(),
                    json!({"points": e.count, "mean_error_db": round(e.mean), "rms_db": round(e.rms), "max_abs_db": round(e.max_abs)}),
                );
            }
        }
        Value::Object(m)
    }

    fn survey_compare(&mut self, command: &str, p: &Value) -> Result<Value> {
        if self.survey.is_empty() {
            return Err(bad(
                command,
                "no measured points (survey.import or survey.add)",
            ));
        }
        if self.plan.is_none() {
            return Err(bad(command, "no plan is open (plan.open)"));
        }
        let rows = self.survey_terms();
        let round = |v: f64| (v * 10.0).round() / 10.0;
        let mut out = json!({
            "points": self.survey.len(),
            "compared": rows.len(),
            "bands": Self::errors_json(&rows),
        });
        if p.get("points").and_then(Value::as_bool).unwrap_or(false) {
            out["each"] = json!(rows.iter().map(|(i, measured, predicted, _)| json!({
                "index": i, "at": self.survey.get(*i).map(|s| s.at), "band": self.survey.get(*i).map(|s| s.band.name()),
                "measured_dbm": measured, "predicted_dbm": round(*predicted), "error_db": round(measured - predicted),
            })).collect::<Vec<_>>());
        }
        Ok(out)
    }

    fn survey_calibrate(&mut self, command: &str, p: &Value) -> Result<Value> {
        if self.survey.is_empty() {
            return Err(bad(
                command,
                "no measured points (survey.import or survey.add)",
            ));
        }
        let before = self.survey_terms();
        if before.is_empty() {
            return Err(bad(
                command,
                "no measured point can be compared: add routers in their bands, or name routers that are there",
            ));
        }
        let terms: Vec<survey::Term> = before
            .iter()
            .map(|r| survey::Term {
                band: r.3.band,
                log_d: r.3.log_d,
                rest: r.3.rest,
            })
            .collect();
        let (exponent, offsets) = survey::fit(&terms, self.settings.exponent, 1.5, 6.0);
        let saved = self.settings.clone();
        self.settings.exponent = exponent;
        for (slot, o) in self.settings.offset_db.iter_mut().zip(offsets) {
            if let Some(o) = o {
                *slot = o.clamp(-30.0, 30.0);
            }
        }
        self.changed(false);
        let after = self.survey_terms();
        let fitted = self.settings.clone();
        let apply = p.get("apply").and_then(Value::as_bool).unwrap_or(false);
        if !apply {
            self.settings = saved;
            self.changed(false);
        }
        let round = |v: f64| (v * 100.0).round() / 100.0;
        Ok(json!({
            "exponent": round(fitted.exponent),
            "offset_db": band_map(&offsets.map(|o| o.map(|v| round(v.clamp(-30.0, 30.0))))),
            "before": Self::errors_json(&before),
            "after": Self::errors_json(&after),
            "applied": apply,
        }))
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
            "survey": self.survey.iter().map(|s| json!({"at": s.at, "band": s.band.name(), "rssi_dbm": s.rssi_dbm, "router": s.router})).collect::<Vec<_>>(),
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
        for s in project
            .get("survey")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            fresh.execute("survey.add", s)?;
        }
        *self = fresh;
        self.revision = self.revision.wrapping_add(1);
        Ok(json!({"path": path, "routers": self.routers.len()}))
    }
}
