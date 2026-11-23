//! The window: a plan view with the coverage map in the middle, the layers and objects with their
//! materials on the left, the routers and the coverage figures on the right. Every change goes
//! through an engine command, so the CLI can do anything the window does.

use egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use radiocraft_engine::materials::{Band, RF_PRESETS};
use radiocraft_engine::model::SCALE;
use radiocraft_engine::{Engine, Power};
use serde_json::{Value, json};

use crate::i18n::tr;
use crate::theme::{TOKENS, wall_colour};

/// How far an arrow key moves the selected router (mm).
const KEY_STEP_MM: f64 = 100.0;

/// What the plan view shows: the plan point at its centre and its scale.
struct View {
    centre: [f64; 2],
    /// Plan millimetres per screen point.
    mm_per_pt: f64,
    /// Whether it was fitted to the plan shown.
    fitted_to: Option<String>,
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
    /// The picture, once the window has sent it.
    shot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
}

/// The width of a name in the layers and objects list.
const NAME_WIDTH: f32 = 150.0;

impl App {
    pub fn new(path: Option<String>, script: Option<String>, screenshot: Option<String>) -> App {
        let mut app = App {
            engine: Engine::new(),
            view: View {
                centre: [0.0, 0.0],
                mm_per_pt: 20.0,
                fitted_to: None,
            },
            texture: None,
            status: None,
            dragging: None,
            selected: None,
            adding: false,
            slice: 1000.0,
            screenshot: screenshot.map(|p| (p, 0)),
            shot: Default::default(),
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
                    if self.run(&command, params).is_none() {
                        break;
                    }
                }
                self.slice = self.engine.settings().slice_height;
                self.selected = self.engine.routers().first().map(|r| r.id);
            }
            Err(e) => self.status = Some((format!("{path}: {e}"), true)),
        }
    }

    /// `--screenshot`: once the window has settled, ask for a picture of it; save it when it
    /// comes, then close.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
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
                Ok(()) => eprintln!("radiocraft: saved {path}"),
                Err(e) => eprintln!("radiocraft: {path}: {e}"),
            }
            self.screenshot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }

    fn open(&mut self, path: &str) {
        let command = if path.to_ascii_lowercase().ends_with(".rcraft") {
            "project.open"
        } else {
            "plan.open"
        };
        if self.run(command, json!({"path": path})).is_some() {
            self.status = Some((format!("{} {path}", tr("Opened")), false));
            self.view.fitted_to = None;
            self.selected = None;
            self.slice = self.engine.settings().slice_height;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(tr("Open plan…"))
                .on_hover_text(tr(
                    "A DXF plan, an OBJ or STL model, or a RadioCraft project",
                ))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(tr("Plans and projects"), &["dxf", "obj", "stl", "rcraft"])
                    .pick_file()
            {
                self.open(&path.to_string_lossy());
            }
            if ui.button(tr("Save project…")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(tr("RadioCraft project"), &["rcraft"])
                    .set_file_name("plan.rcraft")
                    .save_file()
                && self
                    .run("project.save", json!({"path": path.to_string_lossy()}))
                    .is_some()
            {
                self.status = Some((format!("{} {}", tr("Saved"), path.to_string_lossy()), false));
            }
            if ui.button(tr("Export picture…")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG", &["png"])
                    .set_file_name("coverage.png")
                    .save_file()
                && self
                    .run("export.png", json!({"path": path.to_string_lossy()}))
                    .is_some()
            {
                self.status = Some((format!("{} {}", tr("Saved"), path.to_string_lossy()), false));
            }
            if ui
                .button(tr("Device file…"))
                .on_hover_text(tr("A JSON list of devices: {id, name, eirp_dbm}"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                && let Some(r) = self.run("devices.load", json!({"path": path.to_string_lossy()}))
            {
                self.status = Some((
                    format!("{} {}", r["from_file"], tr("devices loaded")),
                    false,
                ));
            }
            ui.separator();
            ui.label(tr("Band"));
            let band = self.engine.settings().band;
            for b in Band::ALL {
                if ui
                    .selectable_label(band == b, format!("{} GHz", b.name()))
                    .clicked()
                    && b != band
                {
                    self.run("settings.set", json!({"band": b.name()}));
                }
            }
            ui.separator();
            ui.label(tr("Slice height"));
            let r = ui.add(
                egui::DragValue::new(&mut self.slice)
                    .range(0.0..=10_000.0)
                    .suffix(" mm")
                    .speed(10.0),
            );
            if (r.drag_stopped() || r.lost_focus())
                && self.slice != self.engine.settings().slice_height
            {
                self.run("settings.set", json!({"slice_height": self.slice}));
            }
        });
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui) {
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
                ui.label(tr("Height"));
                let mut h = r.height;
                let resp = ui.add(
                    egui::DragValue::new(&mut h)
                        .range(0.0..=10_000.0)
                        .suffix(" mm")
                        .speed(10.0),
                );
                if resp.changed() {
                    commands.push(("router.set", json!({"id": r.id, "height": h})));
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
                    .filter(|x| **x == radiocraft_engine::model::Cell::Floor)
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
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(tr("Key: share of the floor at each level or better"))
                .color(TOKENS.text_dim)
                .small(),
        );
        for (i, (from, [r, g, b])) in SCALE.iter().enumerate() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, Color32::from_rgb(*r, *g, *b));
                let label = if from.is_finite() {
                    format!("≥ {from:.0} dBm")
                } else {
                    format!(
                        "< {:.0} dBm",
                        SCALE.get(i.wrapping_sub(1)).map_or(-80.0, |(t, _)| *t)
                    )
                };
                ui.label(egui::RichText::new(label).monospace());
                if let Some(share) = c.floor_share.get(i) {
                    ui.label(
                        egui::RichText::new(format!("{share:.0}%"))
                            .monospace()
                            .color(TOKENS.text_dim),
                    );
                }
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
        if self.view.fitted_to.as_deref() != Some(path.as_str())
            && let Some(b) = self.engine.plan().and_then(|p| p.bounds())
        {
            let (w, h) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
            self.view.centre = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
            self.view.mm_per_pt = (w / f64::from(rect.width().max(1.0)))
                .max(h / f64::from(rect.height().max(1.0)))
                * 1.1;
            self.view.fitted_to = Some(path);
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
                    .and_then(|(_, l)| radiocraft_engine::materials::rf_preset_of(l))
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
        // Routers.
        let pick = |v: &View, pos: Pos2, routers: &[radiocraft_engine::Router]| -> Option<u64> {
            routers
                .iter()
                .map(|r| (r.id, to_screen(v, r.at).distance(pos)))
                .filter(|(_, d)| *d <= TOKENS.router_radius + 6.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(id, _)| id)
        };
        let routers = self.engine.routers().to_vec();
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
        if let Some(pos) = response.hover_pos()
            && !self.adding
        {
            // The signal under the pointer.
            let at = to_plan(&self.view, pos);
            if let Some(v) = self.engine.execute("coverage.at", &json!({"at": at})).ok()
                && let Some(band) = v["bands"].get(self.engine.settings().band.name())
                && let Some(rssi) = band["rssi_dbm"].as_f64()
            {
                // The signal and the router it comes from, when there's more than one.
                let text = match band["from"].as_str() {
                    Some(from) if self.engine.routers().len() > 1 => {
                        format!("{rssi:.0} dBm · {from}")
                    }
                    _ => format!("{rssi:.0} dBm"),
                };
                painter.text(
                    pos + Vec2::new(12.0, -12.0),
                    Align2::LEFT_BOTTOM,
                    text,
                    FontId::monospace(12.0),
                    TOKENS.text,
                );
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_screenshot(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            let (text, error) = self.status.clone().unwrap_or_else(|| {
                (
                    tr("Drag a router to move it; scroll to zoom; drag the plan to pan."),
                    false,
                )
            });
            ui.label(egui::RichText::new(text).color(if error {
                TOKENS.error
            } else {
                TOKENS.text_dim
            }));
        });
        egui::Panel::left("sources")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| self.sources_panel(ui));
        egui::Panel::right("routers")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| self.routers_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.plan_view(ui));
    }
}
