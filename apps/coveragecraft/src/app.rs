//! The window: a plan view with the coverage map in the middle, the layers and objects with their
//! materials on the left, the routers and the coverage figures on the right. Every change goes
//! through an engine command, so the CLI can do anything the window does.

use coveragecraft_engine::materials::{Band, RF_PRESETS};
use coveragecraft_engine::radio::{self, View as Map};
use coveragecraft_engine::{Engine, Power};
use egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use serde_json::{Value, json};

use crate::i18n::tr;
use crate::theme::{TOKENS, wall_colour};

#[path = "calibration.rs"]
mod calibration;
#[path = "channels_ui.rs"]
mod channels_ui;
#[path = "header.rs"]
mod header;

/// How far an arrow key moves the selected router (mm).
const KEY_STEP_MM: f64 = 100.0;

/// What the plan view shows: the plan point at its centre and its scale.
struct View {
    centre: [f64; 2],
    /// Plan millimetres per screen point.
    mm_per_pt: f64,
    /// The plan it was fitted to, and the size of the view then: until the user pans or zooms,
    /// it fits again when the view's size changes (a panel growing, the window resizing).
    fitted_to: Option<(String, Vec2)>,
    /// The user has panned or zoomed since the fit.
    moved: bool,
}

pub struct App {
    engine: Engine,
    view: View,
    texture: Option<(u64, TextureHandle)>,
    status: Option<(String, bool)>,
    /// The router being dragged.
    dragging: Option<u64>,
    selected: Option<u64>,
    /// The next click on the plan adds a router.
    adding: bool,
    /// The slice height being typed, committed when it changes.
    slice: f64,
    /// `--screenshot`: where to save a picture of the window, and the frames drawn so far.
    screenshot: Option<(String, u32)>,
    /// The figures and the survey comparison, kept until the engine changes (they cost a pass
    /// over the floor and over every measured point).
    figures_cache: Option<(u64, Value, Option<Value>)>,
    /// The picture, once the window has sent it.
    shot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
    /// Home (the start page) is showing.
    home: bool,
    /// The About window is open.
    about: bool,
    logo: Option<TextureHandle>,
    /// The menus in the macOS menu bar; `None` draws them in the window.
    pub native: Option<craft_menu::NativeMenu>,
    /// Room kept at the bar's leading edge for the macOS window buttons (pt).
    leading: f32,
    /// The window buttons the app draws (Windows, Linux): the desktop's family and layout.
    window_buttons: Option<(craft_window::policy::CaptionFamily, craft_window::Layout)>,
    /// Whether the window is movable from the bar now (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    movable: Option<bool>,
    /// The plan point under the pointer.
    pointer: Option<[f64; 2]>,
    /// A SolidCraft design being exported in the background, before it opens.
    pending: Option<Pending>,
    /// The floor name being typed: the floor and the text so far.
    floor_name: Option<(usize, String)>,
    /// A calibration worked out but not applied: the engine's revision it was made at, and the
    /// fit.
    calibration: Option<(u64, Value)>,
    /// Channels suggested but not applied: the engine's revision then, and the suggestion.
    channel_plan: Option<(u64, Value)>,
    /// Whether channel suggestions may use 5 GHz DFS channels.
    channel_dfs: bool,
}

/// A file waiting for SolidCraft to export its design.
struct Pending {
    /// What to open once the export is in: the design, or a project whose plan it is.
    open: String,
    /// The design SolidCraft is exporting.
    design: String,
    done: std::sync::mpsc::Receiver<Result<coveragecraft_engine::sc::Converted, String>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    started: std::time::Instant,
    thread: std::thread::JoinHandle<()>,
}

impl Drop for App {
    /// Quitting while SolidCraft exports: stop it and let its temporary folder go, rather than
    /// leave it running on its own. It checks for the stop every 50 ms; this waits up to 2 s.
    fn drop(&mut self) {
        if let Some(p) = self.pending.take() {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !p.thread.is_finished() && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

/// The width of a name in the layers and objects list.
const NAME_WIDTH: f32 = 150.0;

impl App {
    pub fn new(
        path: Option<String>,
        script: Option<String>,
        screenshot: Option<String>,
        leading: f32,
        window_buttons: Option<(craft_window::policy::CaptionFamily, craft_window::Layout)>,
    ) -> App {
        let mut app = App {
            engine: Engine::new(),
            view: View {
                centre: [0.0, 0.0],
                mm_per_pt: 20.0,
                fitted_to: None,
                moved: false,
            },
            texture: None,
            status: None,
            dragging: None,
            selected: None,
            adding: false,
            slice: 1000.0,
            screenshot: screenshot.map(|p| (p, 0)),
            figures_cache: None,
            shot: Default::default(),
            home: true,
            about: false,
            logo: None,
            native: None,
            leading,
            window_buttons,
            movable: None,
            pointer: None,
            pending: None,
            floor_name: None,
            calibration: None,
            channel_plan: None,
            channel_dfs: false,
        };
        if let Some(path) = path {
            app.open(&path);
        }
        if let Some(script) = script {
            app.script(&script);
        }
        app
    }

    /// Run a command; an error goes to the status line.
    fn run(&mut self, command: &str, params: Value) -> Option<Value> {
        match self.engine.execute(command, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.status = Some((e.to_string(), true));
                None
            }
        }
    }

    /// Run a script of commands (`{"commands": [{"command", "params"}...]}`), as the CLI does.
    fn script(&mut self, path: &str) {
        let script = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        match script {
            Ok(v) => {
                for step in v
                    .get("commands")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let command = step
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let params = step.get("params").cloned().unwrap_or(json!({}));
                    let not_applied = !params
                        .get("apply")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let preview = command == "survey.calibrate" && not_applied;
                    let channels = command == "channel.suggest" && not_applied;
                    let Some(r) = self.run(&command, params) else {
                        break;
                    };
                    // A calibration or channels not applied are shown, as their buttons show them.
                    if preview {
                        self.calibration = Some((self.engine.revision(), r));
                    } else if channels {
                        self.channel_plan = Some((self.engine.revision(), r));
                    }
                }
                self.slice = self.engine.settings().slice_height;
                self.selected = self.engine.routers().first().map(|r| r.id);
                self.home = self.engine.plan().is_none();
            }
            Err(e) => self.status = Some((format!("{path}: {e}"), true)),
        }
    }

    /// `--screenshot`: once the window has settled, ask for a picture of it; save it when it
    /// comes, then close.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        // A design still being exported isn't shown yet: wait for it.
        if self.pending.is_some() {
            ctx.request_repaint();
            return;
        }
        let Some((path, frames)) = self.screenshot.as_mut() else {
            return;
        };
        *frames += 1;
        if *frames == 8 {
            let slot = self.shot.clone();
            let repaint = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |image| {
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(image);
                    }
                    repaint.request_repaint();
                }),
            ));
        }
        let image = self.shot.lock().ok().and_then(|mut s| s.take());
        if let Some(image) = image {
            let path = path.clone();
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let mut bytes = Vec::new();
            let written = (|| -> Result<(), String> {
                let mut e = png::Encoder::new(
                    &mut bytes,
                    u32::try_from(w).map_err(|e| e.to_string())?,
                    u32::try_from(h).map_err(|e| e.to_string())?,
                );
                e.set_color(png::ColorType::Rgba);
                e.set_depth(png::BitDepth::Eight);
                e.write_header()
                    .map_err(|e| e.to_string())?
                    .write_image_data(&rgba)
                    .map_err(|e| e.to_string())
            })();
            match written.and_then(|()| std::fs::write(&path, &bytes).map_err(|e| e.to_string())) {
                Ok(()) => eprintln!("coveragecraft: saved {path}"),
                Err(e) => eprintln!("coveragecraft: {path}: {e}"),
            }
            self.screenshot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }

    /// Open a plan, model or project. A SolidCraft design (or a project of one) is exported by
    /// SolidCraft in the background first, so the window stays free; it opens when that's done.
    fn open(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        let design = if lower.ends_with(".sc") {
            Some(path.to_string())
        } else if is_project(&lower) {
            std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|p| p["plan"].as_str().map(str::to_string))
                .filter(|plan| plan.to_ascii_lowercase().ends_with(".sc"))
        } else {
            None
        };
        if let Some(design) = design {
            self.export_in_background(path, design);
            return;
        }
        self.open_now(path);
    }

    fn export_in_background(&mut self, open: &str, design: String) {
        if let Some(p) = self.pending.take() {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (flag, file) = (cancel.clone(), design.clone());
        let spawned = std::thread::Builder::new()
            .name("solidcraft-export".into())
            .spawn(move || {
                let _ = tx.send(coveragecraft_engine::sc::convert_with(
                    std::path::Path::new(&file),
                    &flag,
                ));
            });
        match spawned {
            Ok(thread) => {
                self.pending = Some(Pending {
                    open: open.to_string(),
                    design,
                    done: rx,
                    cancel,
                    started: std::time::Instant::now(),
                    thread,
                });
                self.status = None;
            }
            // No thread: export here, as before.
            Err(_) => self.open_now(open),
        }
    }

    /// Each frame: open the design once its export is in.
    fn poll_pending(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pending else { return };
        match p.done.try_recv() {
            Ok(result) => {
                let Some(p) = self.pending.take() else { return };
                match result {
                    Ok(converted) => {
                        if let Err(e) = self.engine.prime_sc(&p.design, converted) {
                            self.status = Some((e.to_string(), true));
                            return;
                        }
                        self.open_now(&p.open);
                    }
                    Err(e) if e == coveragecraft_engine::sc::CANCELLED => {
                        self.status = Some((tr("Opening cancelled"), false));
                    }
                    Err(e) => self.status = Some((format!("{}: {e}", p.design), true)),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = Some((tr("SolidCraft's export stopped without a result"), true));
            }
        }
    }

    fn open_now(&mut self, path: &str) {
        let command = if is_project(&path.to_ascii_lowercase()) {
            "project.open"
        } else {
            "plan.open"
        };
        if self.run(command, json!({"path": path})).is_some() {
            self.status = Some((format!("{} {path}", tr("Opened")), false));
            self.view.fitted_to = None;
            self.view.moved = false;
            self.selected = None;
            self.slice = self.engine.settings().slice_height;
            self.home = false;
        }
    }

    fn saved(&mut self, path: &std::path::Path) {
        self.status = Some((format!("{} {}", tr("Saved"), path.to_string_lossy()), false));
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                tr("Plans, models and projects"),
                &["dxf", "dwg", "sc", "obj", "stl", "coveragecraft", "rcraft"],
            )
            .pick_file()
        {
            self.open(&path.to_string_lossy());
        }
    }

    fn save_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(tr("CoverageCraft project"), &["coveragecraft"])
            .set_file_name("plan.coveragecraft")
            .save_file()
            && self
                .run("project.save", json!({"path": path.to_string_lossy()}))
                .is_some()
        {
            self.saved(&path);
        }
    }

    fn export_png_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name("coverage.png")
            .save_file()
            && self
                .run("export.png", json!({"path": path.to_string_lossy()}))
                .is_some()
        {
            self.saved(&path);
        }
    }

    fn report_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("HTML", &["html"])
            .set_file_name("coverage-report.html")
            .save_file()
            && self
                .run("report.export", json!({"path": path.to_string_lossy()}))
                .is_some()
        {
            self.saved(&path);
        }
    }

    fn measurements_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(tr("Measurements"), &["csv", "json"])
            .pick_file()
            && let Some(r) = self.run("survey.import", json!({"path": path.to_string_lossy()}))
        {
            self.status = Some((
                format!("{} {}", r["points"], tr("measured points read")),
                false,
            ));
        }
    }

    fn devices_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
            && let Some(r) = self.run("devices.load", json!({"path": path.to_string_lossy()}))
        {
            self.status = Some((
                format!("{} {}", r["from_file"], tr("devices loaded")),
                false,
            ));
        }
    }

    /// Add a router like the selected one (or the last) where it helps most.
    fn suggest_spot(&mut self) {
        let like = self
            .engine
            .routers()
            .iter()
            .find(|r| Some(r.id) == self.selected)
            .or_else(|| self.engine.routers().last())
            .cloned();
        let mut params = json!({"add": true});
        if let Some(r) = like {
            params["height"] = json!(r.height);
            match &r.power {
                Power::Device(id) => params["device"] = json!(id),
                Power::Eirp(_) => {
                    params["eirp_dbm"] = json!(
                        self.engine
                            .eirp(&r)
                            .iter()
                            .zip(Band::ALL)
                            .filter_map(|(v, b)| v.map(|v| (b.name().to_string(), json!(v))))
                            .collect::<serde_json::Map<_, _>>()
                    )
                }
            }
        }
        if let Some(v) = self.run("router.suggest", params)
            && let Some(spot) = v["spots"].get(0)
        {
            self.selected = spot["id"].as_u64();
            self.status = Some((
                format!(
                    "{} {}% {}",
                    tr("Suggested spot added:"),
                    spot["pct_at_least_67"],
                    tr("of the floor at −67 dBm or better")
                ),
                false,
            ));
        }
    }

    /// The menus' shortcuts, when the menus are in the window (macOS's menu bar handles them).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        if self.native.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let command = if pressed(Modifiers::COMMAND, Key::O) {
            "file.open"
        } else if pressed(Modifiers::COMMAND, Key::S) {
            "file.save"
        } else if pressed(shift_cmd, Key::E) {
            "file.export_report"
        } else if pressed(Modifiers::COMMAND, Key::Num0) {
            "view.fit"
        } else if pressed(shift_cmd, Key::Z) {
            "edit.redo"
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            "edit.undo"
        } else {
            return;
        };
        self.menu_command(ctx, &craft_menu::CommandRef::new(command));
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        self.floor_section(ui);
        ui.heading(tr("Layers and objects"));
        let Some(plan) = self.engine.plan() else {
            ui.label(tr(
                "Open a DXF plan from CADCraft, or an OBJ or STL model from SolidCraft.",
            ));
            return;
        };
        let counts = plan.shape_counts();
        let rows: Vec<(String, String, String, usize)> = plan
            .sources
            .iter()
            .enumerate()
            .filter(|(i, _)| counts.get(i).copied().unwrap_or(0) > 0)
            .map(|(i, s)| {
                (
                    s.name.clone(),
                    s.kind.name().to_string(),
                    s.material.id().to_string(),
                    counts.get(&i).copied().unwrap_or(0),
                )
            })
            .collect();
        let mut choices: Vec<(String, String)> = vec![
            ("auto".into(), tr("Guess from thickness")),
            ("ignore".into(), tr("Not a wall")),
        ];
        choices.extend(
            RF_PRESETS
                .iter()
                .map(|p| (p.id.to_string(), tr(preset_label(p.id)))),
        );
        let mut change = None;
        egui::ScrollArea::vertical()
            .id_salt("sources")
            .show(ui, |ui| {
                egui::Grid::new("sources_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (name, kind, material, shapes) in &rows {
                            // Long names (a mesh's object ids) are cut short so the plan keeps its room;
                            // the whole name shows on hover.
                            ui.allocate_ui(
                                egui::vec2(NAME_WIDTH, ui.spacing().interact_size.y),
                                |ui| {
                                    ui.add(egui::Label::new(name).truncate()).on_hover_text(
                                        format!("{name}\n{kind}, {shapes} {}", tr("shapes")),
                                    );
                                },
                            );
                            let shown = choices
                                .iter()
                                .find(|(id, _)| id == material)
                                .map_or_else(|| tr("Custom"), |(_, label)| label.clone());
                            egui::ComboBox::from_id_salt(format!("material {kind} {name}"))
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    for (id, label) in &choices {
                                        if ui.selectable_label(id == material, label).clicked()
                                            && id != material
                                        {
                                            change = Some((name.clone(), kind.clone(), id.clone()));
                                        }
                                    }
                                });
                            ui.end_row();
                        }
                    });
            });
        if let Some((name, kind, id)) = change {
            self.run(
                "material.set",
                json!({"source": name, "kind": kind, "material": id}),
            );
        }
    }

    fn routers_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Routers"));
        let adding = ui
            .selectable_label(self.adding, tr("Add router"))
            .on_hover_text(tr("Then click the plan where it goes"));
        if adding.clicked() {
            self.adding = !self.adding;
        }
        ui.horizontal(|ui| {
            if ui
                .button(tr("Suggest a spot"))
                .on_hover_text(tr("Add a router where it raises the floor at −67 dBm or better the most, like the selected one"))
                .clicked()
            {
                self.suggest_spot();
            }
            if ui
                .add_enabled(self.engine.routers().len() > 1, egui::Button::new(tr("Suggest channels…")))
                .on_hover_text(tr(
                    "Channels for the shown band that keep routers on the same channel apart, shown before they are set",
                ))
                .clicked()
            {
                self.suggest_channels();
            }
        });
        self.channels_section(ui);
        let devices: Vec<(String, String)> = self
            .engine
            .devices()
            .iter()
            .map(|d| (d.id.clone(), d.name.clone()))
            .collect();
        let routers = self.engine.routers().to_vec();
        let mut commands: Vec<(&'static str, Value)> = Vec::new();
        for r in &routers {
            ui.separator();
            ui.horizontal(|ui| {
                let selected = self.selected == Some(r.id);
                if ui.selectable_label(selected, &r.name).clicked() {
                    self.selected = Some(r.id);
                }
                if ui.small_button(tr("Remove")).clicked() {
                    commands.push(("router.remove", json!({"id": r.id})));
                }
            });
            let current = match &r.power {
                Power::Device(id) => devices
                    .iter()
                    .find(|(d, _)| d == id)
                    .map_or_else(|| id.clone(), |(_, n)| n.clone()),
                Power::Eirp(_) => tr("Typed power"),
            };
            egui::ComboBox::from_id_salt(format!("device {}", r.id))
                .selected_text(current)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (id, name) in &devices {
                        if ui
                            .selectable_label(matches!(&r.power, Power::Device(d) if d == id), name)
                            .clicked()
                        {
                            commands.push(("router.set", json!({"id": r.id, "device": id})));
                        }
                    }
                });
            ui.horizontal(|ui| {
                // From the top a router's third coordinate is its height; from a side, its depth
                // toward the viewer.
                let top = self.engine.settings().viewpoint == "top"
                    || !self.engine.plan().is_some_and(|p| {
                        let l = p.path.to_ascii_lowercase();
                        l.ends_with(".obj") || l.ends_with(".stl") || l.ends_with(".sc")
                    });
                ui.label(if top { tr("Height") } else { tr("Depth") });
                let mut h = r.height;
                let resp = ui.add(
                    egui::DragValue::new(&mut h)
                        .range(-100_000.0..=100_000.0)
                        .suffix(" mm")
                        .speed(10.0),
                );
                if resp.changed() {
                    commands.push(("router.set", json!({"id": r.id, "height": h})));
                }
            });
            // The channel in the shown band: its number and width; the engine refuses a pair
            // the band's channel plan doesn't have.
            let band = self.engine.settings().band;
            let current = r.channels.get(band.index()).copied().flatten();
            // The router's floor, when there are several.
            let floors: Vec<String> = self
                .engine
                .floors()
                .iter()
                .map(|f| f.name.clone())
                .collect();
            if floors.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(tr("Floor"));
                    egui::ComboBox::from_id_salt(format!("router floor {}", r.id))
                        .selected_text(floors.get(r.floor).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (i, name) in floors.iter().enumerate() {
                                if ui.selectable_label(i == r.floor, name).clicked() && i != r.floor
                                {
                                    commands.push(("router.set", json!({"id": r.id, "floor": i})));
                                }
                            }
                        });
                });
            }
            ui.horizontal(|ui| {
                ui.label(tr("Channel"));
                let text = current.map_or_else(|| tr("Not set"), |c| c.number.to_string());
                egui::ComboBox::from_id_salt(format!("channel {}", r.id))
                    .selected_text(text)
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), tr("Not set")).clicked() {
                            commands.push(("router.set", json!({"id": r.id, "channels": {band.name(): null}})));
                        }
                        for n in channel_numbers(band) {
                            let width = current.map_or(radio::default_width(band), |c| c.width);
                            let width = if radio::Channel::new(band, n, width).is_some() { width } else { 20 };
                            if ui.selectable_label(current.is_some_and(|c| c.number == n), n.to_string()).clicked() {
                                commands.push(("router.set", json!({"id": r.id, "channels": {band.name(): {"channel": n, "width": width}}})));
                            }
                        }
                    });
                if let Some(c) = current {
                    egui::ComboBox::from_id_salt(format!("width {}", r.id))
                        .selected_text(format!("{} MHz", c.width))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for w in radio::widths(band) {
                                if ui.selectable_label(c.width == *w, format!("{w} MHz")).clicked() {
                                    commands.push(("router.set", json!({"id": r.id, "channels": {band.name(): {"channel": c.number, "width": w}}})));
                                }
                            }
                        });
                }
            });
            let eirp = self.engine.eirp(r);
            let text: Vec<String> = Band::ALL
                .iter()
                .filter_map(|b| {
                    eirp.get(b.index())
                        .copied()
                        .flatten()
                        .map(|v| format!("{} GHz {v:.1} dBm", b.name()))
                })
                .collect();
            ui.label(
                egui::RichText::new(text.join(" · "))
                    .color(TOKENS.text_dim)
                    .small(),
            );
        }
        for (c, p) in commands {
            self.run(c, p);
        }
        ui.separator();
        self.figures(ui);
    }

    fn figures(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Coverage"));
        let Some(c) = self.engine.coverage() else {
            ui.label(tr("No walls to analyse yet."));
            return;
        };
        if let Some(st) = &c.stats {
            egui::Grid::new("figures").num_columns(2).show(ui, |ui| {
                let cell_m2 = c.grid.step * c.grid.step / 1e6;
                let floor = c
                    .cells
                    .iter()
                    .filter(|x| **x == coveragecraft_engine::model::Cell::Floor)
                    .count() as f64
                    * cell_m2;
                for (label, value) in [
                    (tr("Floor"), format!("{floor:.1} m²")),
                    (
                        tr("−60 dBm or better (fast)"),
                        format!("{:.0}%", st.good * 100.0),
                    ),
                    (
                        tr("−67 dBm or better (video calls)"),
                        format!("{:.0}%", st.fair * 100.0),
                    ),
                    (tr("Weakest 5%"), format!("{:.0} dBm", st.p5)),
                    (tr("Median"), format!("{:.0} dBm", st.median)),
                ] {
                    ui.label(label);
                    ui.label(egui::RichText::new(value).monospace());
                    ui.end_row();
                }
            });
        } else {
            ui.label(tr("Add a router with a radio in this band."));
        }
        // The figures only the engine's report has: roaming, link rate, channels, leakage.
        let revision = self.engine.revision();
        if self
            .figures_cache
            .as_ref()
            .is_none_or(|(r, _, _)| *r != revision)
        {
            let figures = self
                .engine
                .execute("coverage.compute", &json!({}))
                .unwrap_or(Value::Null);
            let survey = (!self.engine.survey().is_empty())
                .then(|| self.engine.execute("survey.compare", &json!({})).ok())
                .flatten();
            self.figures_cache = Some((revision, figures, survey));
        }
        let (figures, survey) = self
            .figures_cache
            .as_ref()
            .map(|(_, f, s)| (f.clone(), s.clone()))
            .unwrap_or((Value::Null, None));
        if let f @ Value::Object(_) = figures {
            egui::Grid::new("more_figures")
                .num_columns(2)
                .show(ui, |ui| {
                    let mut row = |label: String, value: Option<String>| {
                        if let Some(value) = value {
                            ui.label(label);
                            ui.label(egui::RichText::new(value).monospace());
                            ui.end_row();
                        }
                    };
                    let pct = |k: &str| f[k].as_f64().map(|v| format!("{v:.0}%"));
                    row(
                        tr("Roaming: 2nd router ≥ −67 dBm"),
                        pct("pct_second_at_least_67"),
                    );
                    row(
                        tr("Median link rate"),
                        f["median_rate_mbps"]
                            .as_f64()
                            .map(|v| format!("{v:.0} Mbit/s")),
                    );
                    row(
                        tr("No overlapping channel heard"),
                        pct("pct_no_channel_overlap"),
                    );
                    row(
                        tr("Outside the walls ≥ −67 dBm"),
                        f["leakage"]["m2_at_least_67"]
                            .as_f64()
                            .map(|v| format!("{v:.1} m²")),
                    );
                });
        }
        // Measured points: how far the prediction is from them, and a fit to them.
        if !self.engine.survey().is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr("Measurements")).strong());
            let band = self.engine.settings().band;
            if let Some(v) = survey {
                let b = &v["bands"][band.name()];
                if let (Some(n), Some(mean), Some(rms)) = (
                    b["points"].as_u64(),
                    b["mean_error_db"].as_f64(),
                    b["rms_db"].as_f64(),
                ) {
                    ui.label(format!(
                        "{n} {} · {} {mean:+.1} dB · RMS {rms:.1} dB",
                        tr("points"),
                        tr("measured − predicted")
                    ));
                } else {
                    ui.label(tr("None in this band"));
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .button(tr("Calibrate…"))
                    .on_hover_text(tr(
                        "Fit the path-loss exponent, each band's offset and the losses of walls enough points are measured through, and show the fit before using it",
                    ))
                    .clicked()
                    && let Some(r) = self.run("survey.calibrate", json!({}))
                {
                    self.calibration = Some((self.engine.revision(), r));
                }
                if ui.button(tr("Clear")).clicked() {
                    self.run("survey.clear", json!({}));
                }
            });
            self.calibration_section(ui);
        }
        self.wall_losses_section(ui);
        ui.add_space(6.0);
        let key = match c.view {
            Map::Serving | Map::Overlap => tr("Key: share of the floor"),
            Map::Leakage => {
                tr("Key: share of the area outside the walls, within 3 m of the plan's edge")
            }
            _ => tr("Key: share of the floor at each level or better"),
        };
        ui.label(egui::RichText::new(key).color(TOKENS.text_dim).small());
        if c.view == Map::Rate {
            ui.label(
                egui::RichText::new(tr(
                    "Estimated from the signal alone, with nothing else on the air",
                ))
                .color(TOKENS.text_dim)
                .small(),
            );
        }
        for row in &c.legend {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                let [r, g, b] = row.colour;
                ui.painter()
                    .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                ui.label(egui::RichText::new(tr(&row.label)).monospace());
                ui.label(
                    egui::RichText::new(format!("{:.0}%", row.share))
                        .monospace()
                        .color(TOKENS.text_dim),
                );
            });
        }
    }

    fn plan_view(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Canvas,
                true,
                tr("Floor plan with the coverage map"),
            )
        });
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, TOKENS.outside);
        let Some(path) = self.engine.plan().map(|p| p.path.clone()) else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr("Open a plan to start"),
                FontId::proportional(18.0),
                TOKENS.text_dim,
            );
            return;
        };
        let coverage = self.engine.coverage();
        // Fit the view to a newly opened plan.
        let stale = match &self.view.fitted_to {
            Some((p, size)) => *p != path || (!self.view.moved && *size != rect.size()),
            None => true,
        };
        if stale && let Some(b) = self.engine.plan().and_then(|p| p.bounds()) {
            let (w, h) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
            self.view.centre = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
            self.view.mm_per_pt = (w / f64::from(rect.width().max(1.0)))
                .max(h / f64::from(rect.height().max(1.0)))
                * 1.1;
            self.view.fitted_to = Some((path, rect.size()));
        }
        let to_screen = |v: &View, p: [f64; 2]| -> Pos2 {
            Pos2::new(
                rect.center().x + ((p[0] - v.centre[0]) / v.mm_per_pt) as f32,
                rect.center().y - ((p[1] - v.centre[1]) / v.mm_per_pt) as f32,
            )
        };
        let to_plan = |v: &View, s: Pos2| -> [f64; 2] {
            [
                v.centre[0] + f64::from(s.x - rect.center().x) * v.mm_per_pt,
                v.centre[1] - f64::from(s.y - rect.center().y) * v.mm_per_pt,
            ]
        };
        // Zoom about the pointer.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0);
            if scroll != 0.0
                && let Some(pointer) = response.hover_pos()
            {
                let before = to_plan(&self.view, pointer);
                self.view.moved = true;
                self.view.mm_per_pt =
                    (self.view.mm_per_pt * f64::from((-scroll / 300.0).exp())).clamp(0.05, 2000.0);
                let after = to_plan(&self.view, pointer);
                self.view.centre = [
                    self.view.centre[0] + before[0] - after[0],
                    self.view.centre[1] + before[1] - after[1],
                ];
            }
        }
        if let Some(c) = &coverage {
            // The heatmap: its first row is the grid's lowest; an image's first row is its top.
            let revision = self.engine.revision();
            if self.texture.as_ref().is_none_or(|(r, _)| *r != revision) {
                let (w, h) = (c.grid.columns, c.grid.rows);
                let mut flipped = Vec::with_capacity(c.rgba.len());
                for row in (0..h).rev() {
                    if let Some(chunk) = c.rgba.get(row * w * 4..(row + 1) * w * 4) {
                        flipped.extend_from_slice(chunk);
                    }
                }
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &flipped);
                self.texture = Some((
                    revision,
                    ui.ctx()
                        .load_texture("coverage", image, TextureOptions::NEAREST),
                ));
            }
            if let Some((_, tex)) = &self.texture {
                let g = &c.grid;
                let top_left = to_screen(
                    &self.view,
                    [g.origin[0], g.origin[1] + g.rows as f64 * g.step],
                );
                let bottom_right = to_screen(
                    &self.view,
                    [g.origin[0] + g.columns as f64 * g.step, g.origin[1]],
                );
                let tint = Color32::from_white_alpha(TOKENS.heatmap_alpha);
                painter.image(
                    tex.id(),
                    Rect::from_two_pos(top_left, bottom_right),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    tint,
                );
            }
            for region in &c.walls.regions {
                let preset = c
                    .walls
                    .materials
                    .get(region.material)
                    .and_then(|(_, l)| coveragecraft_engine::materials::rf_preset_of(l))
                    .map(|p| p.id);
                let colour = wall_colour(preset, region.thick.is_some());
                for ring in &region.rings {
                    let mut points: Vec<Pos2> =
                        ring.iter().map(|p| to_screen(&self.view, *p)).collect();
                    if let Some(first) = points.first().copied() {
                        points.push(first);
                    }
                    painter.add(egui::Shape::line(points, Stroke::new(1.2, colour)));
                }
            }
        }
        // Measured points in the shown band: a dot in the measured signal's colour.
        let band = self.engine.settings().band;
        for pt in self.engine.survey().iter().filter(|pt| pt.band == band) {
            let [r, g, b] = coveragecraft_engine::model::colour(pt.rssi_dbm);
            painter.circle(
                to_screen(&self.view, pt.at),
                4.0,
                Color32::from_rgb(r, g, b),
                Stroke::new(1.5, TOKENS.text),
            );
        }
        // Routers.
        let pick = |v: &View, pos: Pos2, routers: &[coveragecraft_engine::Router]| -> Option<u64> {
            routers
                .iter()
                .map(|r| (r.id, to_screen(v, r.at).distance(pos)))
                .filter(|(_, d)| *d <= TOKENS.router_radius + 6.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(id, _)| id)
        };
        let all = self.engine.routers().to_vec();
        let active = self.engine.active_floor();
        let floor_name = |i: usize| {
            self.engine
                .floors()
                .get(i)
                .map_or_else(String::new, |f| f.name.clone())
        };
        // Routers on other floors: faded, named with their floor, not picked or dragged here.
        for r in all.iter().filter(|r| r.floor != active) {
            let p = to_screen(&self.view, r.at);
            painter.circle(
                p,
                TOKENS.router_radius * 0.8,
                TOKENS.router_fill.gamma_multiply(0.35),
                Stroke::new(2.0, TOKENS.accent.gamma_multiply(0.45)),
            );
            painter.text(
                p + Vec2::new(TOKENS.router_radius + 4.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{} · {}", r.name, floor_name(r.floor)),
                FontId::proportional(12.0),
                TOKENS.text_dim,
            );
        }
        let routers: Vec<_> = all.iter().filter(|r| r.floor == active).cloned().collect();
        for r in &routers {
            let p = to_screen(&self.view, r.at);
            let ring = if self.selected == Some(r.id) {
                TOKENS.selection
            } else {
                TOKENS.accent
            };
            painter.circle(
                p,
                TOKENS.router_radius,
                TOKENS.router_fill,
                Stroke::new(3.0, ring),
            );
            painter.text(
                p + Vec2::new(TOKENS.router_radius + 4.0, 0.0),
                Align2::LEFT_CENTER,
                &r.name,
                FontId::proportional(13.0),
                TOKENS.text,
            );
        }
        // Clicks, drags and keys.
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.dragging = pick(&self.view, pos, &routers);
            // A new drag is a step of its own, even of the router moved just before.
            let _ = self.engine.execute("edit.checkpoint", &json!({}));
            if self.dragging.is_some() {
                self.selected = self.dragging;
            }
        }
        if response.dragged() {
            match (self.dragging, response.interact_pointer_pos()) {
                (Some(id), Some(pos)) => {
                    let at = to_plan(&self.view, pos);
                    self.run("router.move", json!({"id": id, "at": at}));
                }
                _ => {
                    let d = response.drag_delta();
                    self.view.moved = true;
                    self.view.centre = [
                        self.view.centre[0] - f64::from(d.x) * self.view.mm_per_pt,
                        self.view.centre[1] + f64::from(d.y) * self.view.mm_per_pt,
                    ];
                }
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if self.adding {
                let at = to_plan(&self.view, pos);
                let height = self.engine.routers().last().map_or(2400.0, |r| r.height);
                if let Some(r) = self.run("router.add", json!({"at": at, "height": height})) {
                    self.selected = r["id"].as_u64();
                }
                self.adding = false;
            } else {
                self.selected = pick(&self.view, pos, &routers);
            }
            response.request_focus();
        }
        self.pointer = response.hover_pos().map(|p| to_plan(&self.view, p));
        if let Some(pos) = response.hover_pos()
            && !self.adding
        {
            // The signal under the pointer.
            let at = to_plan(&self.view, pos);
            if let Some(v) = self.engine.execute("coverage.at", &json!({"at": at})).ok()
                && let Some(band) = v["bands"].get(self.engine.settings().band.name())
            {
                let mut text = probe_text(band, self.engine.routers().len());
                // From another floor: say which.
                if let Some(f) = band["floor"].as_u64().and_then(|f| usize::try_from(f).ok())
                    && f != self.engine.active_floor()
                    && let Some(floor) = self.engine.floors().get(f)
                {
                    text = format!("{text}\n{} {}", tr("strongest from"), floor.name);
                }
                if !text.is_empty() {
                    let galley = painter.layout_no_wrap(text, FontId::monospace(12.0), TOKENS.text);
                    let at = pos + Vec2::new(14.0, -14.0 - galley.size().y);
                    let rect = Rect::from_min_size(at, galley.size()).expand(4.0);
                    painter.rect_filled(rect, 3.0, TOKENS.tooltip);
                    painter.galley(at, galley, TOKENS.text);
                }
            }
        }
        if response.has_focus() || response.hovered() {
            self.keys(ui);
        }
    }

    /// Keyboard: arrows move the selected router, Delete removes it, Escape stops adding.
    fn keys(&mut self, ui: &egui::Ui) {
        let (left, right, up, down, delete, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            self.adding = false;
        }
        let Some(id) = self.selected else { return };
        let Some(r) = self.engine.routers().iter().find(|r| r.id == id).cloned() else {
            return;
        };
        let (dx, dy) = (
            f64::from(i8::from(right) - i8::from(left)),
            f64::from(i8::from(up) - i8::from(down)),
        );
        if dx != 0.0 || dy != 0.0 {
            self.run(
                "router.move",
                json!({"id": id, "at": [r.at[0] + dx * KEY_STEP_MM, r.at[1] + dy * KEY_STEP_MM]}),
            );
        }
        if delete {
            self.run("router.remove", json!({"id": id}));
            self.selected = None;
        }
    }
}

/// The label of a preset, in our own words.
/// A project file: `.coveragecraft`, or `.rcraft` from when the app was called RadioCraft.
fn is_project(lower: &str) -> bool {
    lower.ends_with(".coveragecraft") || lower.ends_with(".rcraft")
}

fn preset_label(id: &str) -> &'static str {
    match id {
        "concrete" => "Concrete",
        "drywall" => "Drywall (steel stud)",
        "interior-glass" => "Interior glass",
        "low-e-glass" => "Low-E glass",
        "metal" => "Metal",
        "none" => "No loss",
        "wood-door" => "Wood door",
        _ => "Other",
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(menu) = &mut self.native {
            menu.raw_input(raw_input);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_screenshot(&ctx);
        self.poll_pending(&ctx);
        // The macOS menu bar: what was chosen since the last frame, then the menus' state.
        if self.native.is_some() {
            let gate = craft_menu::Gate {
                modal: self.about,
                recording_shortcut: false,
                text_editing: ctx.egui_wants_keyboard_input(),
            };
            let clicks = self.native.as_mut().map(|m| {
                m.set_gate(gate);
                m.take_clicks(&ctx)
            });
            for c in clicks.unwrap_or_default() {
                self.menu_command(&ctx, &c);
            }
            let state = self.menu_state();
            let bar = self.menu_model();
            if let Some(m) = self.native.as_mut() {
                let _ = m.sync_with(state, || {
                    craft_menu::mac_layout(&bar, &craft_menu::LayoutOptions::new(header::APP_NAME))
                });
            }
        }
        self.shortcuts(&ctx);
        egui::Panel::top("app_bar")
            .exact_size(header::BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(TOKENS.bar)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| self.app_bar(ui));
        let show_plan = !self.home && self.engine.plan().is_some();
        if show_plan {
            egui::Panel::top("tools").show(ui, |ui| self.tool_bar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if show_plan {
            egui::Panel::left("sources")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| self.sources_panel(ui));
            egui::Panel::right("routers")
                .resizable(true)
                .default_size(300.0)
                .max_size(380.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("routers")
                        .show(ui, |ui| self.routers_panel(ui));
                });
            egui::CentralPanel::default().show(ui, |ui| self.plan_view(ui));
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.start_page(ui));
        }
        self.about_window(&ctx);
        if self.window_buttons.is_some() {
            crate::captions::resize_zones(ui);
        }
        #[cfg(target_os = "macos")]
        craft_window::title_bar::sync_movable(&ctx, &mut self.movable);
    }
}

/// The 20 MHz channel numbers offered for `band`, as its channel plan has them.
fn channel_numbers(band: Band) -> Vec<u16> {
    (1..=233)
        .filter(|n| radio::Channel::new(band, *n, 20).is_some())
        .collect()
}

/// What the pointer's probe says about a point: the strongest routers and their channels, the
/// link rate, and the routers on an overlapping channel heard there.
fn probe_text(band: &Value, routers: usize) -> String {
    let Some(rssi) = band["rssi_dbm"].as_f64() else {
        return String::new();
    };
    let channel = |c: &Value| {
        c["channel"]
            .as_u64()
            .map_or_else(String::new, |n| format!(" · ch {n}/{}", c["width"]))
    };
    let mut lines = Vec::new();
    if routers > 1 {
        for r in band["routers"].as_array().into_iter().flatten() {
            lines.push(format!(
                "{:.0} dBm  {}{}",
                r["rssi_dbm"].as_f64().unwrap_or(f64::NAN),
                r["name"].as_str().unwrap_or_default(),
                channel(&r["channel"]),
            ));
        }
    } else {
        lines.push(format!("{rssi:.0} dBm"));
    }
    if let Some(rate) = band["rate_mbps"].as_f64() {
        lines.push(if rate > 0.0 {
            format!("{} {rate:.0} Mbit/s (MCS {})", tr("link"), band["mcs"])
        } else {
            tr("too weak to connect")
        });
    }
    if let Some(n) = band["channel_overlap"].as_u64()
        && n > 0
    {
        let names: Vec<&str> = band["interferers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["name"].as_str())
            .collect();
        lines.push(format!("{}: {}", tr("same channel"), names.join(", ")));
    }
    lines.join("\n")
}
