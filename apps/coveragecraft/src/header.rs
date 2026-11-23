//! The header the Craft apps share, built on craft-libs' `craft-menu` (the menus: in the macOS
//! menu bar, or drawn in the window elsewhere) and `craft-window` (the app's bar is the title bar):
//!
//! - the app bar: the logo and name, Home, the menus (off macOS), the open plan's tab, and on the
//!   right the version and Help;
//! - a toolbar for what the map shows: band, map, viewpoint, cut height, fit;
//! - a status bar: the last message, the pointer's position, the scale, the viewpoint.
//!
//! Every menu item is a command id; [`App::menu_command`] runs it, through the engine's commands or
//! the same dialogs the panels use.

use coveragecraft_engine::materials::Band;
use coveragecraft_engine::radio::View as Map;
use craft_menu::{Chord, CommandRef, ItemKind, LayoutOptions, MenuBar, MenuBarUi, Row, Target};
use egui::{Align, Layout, RichText, TextureOptions};
use serde_json::{Value, json};

use super::App;
use crate::i18n::tr;
use crate::theme::TOKENS;

/// The app bar's height (pt): room for the macOS window buttons, centred 14 pt from the top.
pub(super) const BAR_HEIGHT: f32 = 30.0;

/// The app's name in menus and the About window.
pub const APP_NAME: &str = "CoverageCraft";

fn row(
    path: &[&str],
    command: CommandRef,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    kind: ItemKind,
) -> Row {
    Row {
        path: path.iter().map(|p| (*p).to_string()).collect(),
        command,
        label: label.to_string(),
        shortcut: shortcut.and_then(|s| Chord::parse(s).ok()),
        enabled,
        kind,
    }
}

fn separator(path: &[&str]) -> Row {
    row(
        path,
        CommandRef::new("separator"),
        "---",
        None,
        true,
        ItemKind::Plain,
    )
}

impl App {
    /// The menus, from the app's state.
    pub(crate) fn menu_model(&self) -> MenuBar {
        let has_plan = self.engine.plan().is_some();
        let s = self.engine.settings();
        let plain = ItemKind::Plain;
        let mut rows = vec![
            row(
                &["File"],
                CommandRef::new("file.open"),
                "Open…",
                Some("Cmd+O"),
                true,
                plain,
            ),
            row(
                &["File"],
                CommandRef::new("file.save"),
                "Save Project…",
                Some("Cmd+S"),
                has_plan,
                plain,
            ),
            row(
                &["File"],
                CommandRef::new("file.close"),
                "Close Plan",
                None,
                has_plan,
                plain,
            ),
            separator(&["File"]),
            row(
                &["File"],
                CommandRef::new("file.measurements"),
                "Import Measurements…",
                None,
                has_plan,
                plain,
            ),
            row(
                &["File"],
                CommandRef::new("file.devices"),
                "Load Device File…",
                None,
                true,
                plain,
            ),
            separator(&["File"]),
            row(
                &["File"],
                CommandRef::new("file.export_png"),
                "Export Picture…",
                None,
                has_plan,
                plain,
            ),
            row(
                &["File"],
                CommandRef::new("file.export_report"),
                "Export Report…",
                Some("Cmd+Shift+E"),
                has_plan,
                plain,
            ),
        ];
        let undo = self.engine.undo_label().map(|l| format!("Undo {l}"));
        let redo = self.engine.redo_label().map(|l| format!("Redo {l}"));
        rows.extend([
            row(
                &["Edit"],
                CommandRef::new("edit.undo"),
                undo.as_deref().unwrap_or("Undo"),
                Some("Cmd+Z"),
                undo.is_some(),
                plain,
            ),
            row(
                &["Edit"],
                CommandRef::new("edit.redo"),
                redo.as_deref().unwrap_or("Redo"),
                Some("Cmd+Shift+Z"),
                redo.is_some(),
                plain,
            ),
        ]);
        for b in Band::ALL {
            rows.push(row(
                &["View", "Band"],
                CommandRef::with_params("view.band", json!({"band": b.name()})),
                &format!("{} GHz", b.name()),
                None,
                true,
                ItemKind::Radio(s.band == b),
            ));
        }
        for m in Map::ALL {
            rows.push(row(
                &["View", "Map"],
                CommandRef::with_params("view.map", json!({"view": m.id()})),
                m.label(),
                None,
                has_plan,
                ItemKind::Radio(s.view == m),
            ));
        }
        for v in self
            .engine
            .plan()
            .map(|p| p.views.clone())
            .unwrap_or_default()
        {
            rows.push(row(
                &["View", "Viewpoint"],
                CommandRef::with_params("view.viewpoint", json!({"viewpoint": v.id})),
                &v.name,
                None,
                true,
                ItemKind::Radio(s.viewpoint == v.id),
            ));
        }
        for (i, f) in self.engine.floors().iter().enumerate() {
            rows.push(row(
                &["View", "Floor"],
                CommandRef::with_params("view.floor", json!({"index": i})),
                &f.name,
                None,
                true,
                ItemKind::Radio(i == self.engine.active_floor()),
            ));
        }
        rows.push(separator(&["View", "Floor"]));
        rows.push(row(
            &["View", "Floor"],
            CommandRef::new("floor.add"),
            "Add Floor",
            None,
            has_plan,
            plain,
        ));
        rows.extend([
            separator(&["View"]),
            row(
                &["View"],
                CommandRef::new("view.fit"),
                "Fit Plan",
                Some("Cmd+0"),
                has_plan,
                plain,
            ),
            row(
                &["View"],
                CommandRef::new("app.home"),
                "Home",
                None,
                true,
                plain,
            ),
            row(
                &["Routers"],
                CommandRef::new("routers.add"),
                "Add Router",
                None,
                has_plan,
                ItemKind::Check(self.adding),
            ),
            row(
                &["Routers"],
                CommandRef::new("routers.suggest"),
                "Suggest a Spot",
                None,
                has_plan,
                plain,
            ),
            row(
                &["Routers"],
                CommandRef::new("routers.remove"),
                "Remove Selected Router",
                None,
                self.selected.is_some(),
                plain,
            ),
            row(
                &["Help"],
                CommandRef::new("help.about"),
                "About CoverageCraft",
                None,
                true,
                plain,
            ),
        ]);
        let mut bar = craft_menu::from_items(&["File", "Edit", "View", "Routers", "Help"], &rows);
        bar.tag_menu("File", craft_menu::MenuRole::File);
        bar.tag_menu("Edit", craft_menu::MenuRole::Edit);
        bar.tag_menu("View", craft_menu::MenuRole::View);
        bar.tag_menu("Help", craft_menu::MenuRole::Help);
        bar.tag_item(&CommandRef::new("help.about"), craft_menu::ItemRole::About);
        bar
    }

    /// A state key for the menus: they're laid out again when it changes.
    pub(super) fn menu_state(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let s = self.engine.settings();
        (
            self.engine.plan().map(|p| (p.path.clone(), p.views.len())),
            s.band.name(),
            s.view.id(),
            &s.viewpoint,
        )
            .hash(&mut h);
        (
            self.adding,
            self.selected,
            self.engine.undo_label(),
            self.engine.redo_label(),
        )
            .hash(&mut h);
        (
            self.engine.active_floor(),
            self.engine
                .floors()
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        )
            .hash(&mut h);
        h.finish()
    }

    /// Run a menu item's command.
    pub(super) fn menu_command(&mut self, ctx: &egui::Context, c: &CommandRef) {
        let param = |k: &str| c.params.get(k).cloned().unwrap_or(Value::Null);
        match c.id.as_str() {
            "file.open" => self.open_dialog(),
            "file.save" => self.save_dialog(),
            "file.close" => {
                if self.run("project.new", json!({})).is_some() {
                    self.home = true;
                    self.selected = None;
                }
            }
            "file.measurements" => self.measurements_dialog(),
            "file.devices" => self.devices_dialog(),
            "file.export_png" => self.export_png_dialog(),
            "file.export_report" => self.report_dialog(),
            "view.band" => {
                self.run("settings.set", json!({"band": param("band")}));
            }
            "view.map" => {
                self.run("settings.set", json!({"view": param("view")}));
            }
            "view.viewpoint" => {
                self.set_viewpoint(&param("viewpoint"));
            }
            "view.fit" => self.view.fitted_to = None,
            "app.home" => self.home = true,
            "routers.add" => {
                self.adding = !self.adding;
                self.home = false;
            }
            "routers.suggest" => self.suggest_spot(),
            "routers.remove" => {
                if let Some(id) = self.selected.take() {
                    self.run("router.remove", json!({"id": id}));
                }
            }
            "help.about" => self.about = true,
            "view.floor" => self.select_floor(&param("index")),
            "floor.add" => self.add_floor(),
            "edit.undo" | "edit.redo" => {
                if let Some(r) = self.run(&c.id, json!({})) {
                    let what = r["undone"]
                        .as_str()
                        .map(|l| format!("{} {l}", tr("Undid")))
                        .or_else(|| r["redone"].as_str().map(|l| format!("{} {l}", tr("Redid"))));
                    self.status = what.map(|t| (t, false));
                    // A router that's gone isn't selected; the cut may have changed.
                    if self
                        .selected
                        .is_some_and(|id| !self.engine.routers().iter().any(|r| r.id == id))
                    {
                        self.selected = None;
                    }
                    self.slice = self.engine.settings().slice_height;
                }
            }
            _ => {}
        }
        ctx.request_repaint();
    }

    pub(super) fn set_viewpoint(&mut self, id: &Value) {
        if let Some(r) = self.run("settings.set", json!({"viewpoint": id})) {
            self.view.fitted_to = None;
            self.view.moved = false;
            self.slice = self.engine.settings().slice_height;
            if let Some(note) = r["note"].as_str() {
                self.status = Some((note.to_string(), false));
            }
        }
    }

    /// The app bar: logo, Home, menus (off macOS), the plan's tab, version and Help. It is the
    /// window's title bar: a drag on its empty part moves the window.
    pub(super) fn app_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        craft_window::title_bar::empty_area(ui, rect);
        let mut commands: Vec<CommandRef> = Vec::new();
        // The window buttons the app draws itself (Windows, Linux), at the window's very edges.
        let (left, right) = self
            .window_buttons
            .as_ref()
            .map_or((0.0, 0.0), |(family, layout)| {
                crate::captions::reserved(*family, layout)
            });
        ui.horizontal_centered(|ui| {
            ui.add_space(self.leading + left);
            let logo = self.logo.get_or_insert_with(|| {
                ctx.load_texture(
                    "logo",
                    crate::logo_image(),
                    TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
                )
            });
            ui.add(egui::Image::new((logo.id(), egui::vec2(18.0, 18.0))));
            ui.label(RichText::new(APP_NAME).strong());
            if ui
                .selectable_label(self.home, tr("Home"))
                .on_hover_text(tr("Open a plan, or go back to the open one"))
                .clicked()
            {
                self.home = !self.home || self.engine.plan().is_none();
            }
            // Undo and redo, named for what they'd do.
            let undo = self.engine.undo_label().map(str::to_string);
            let redo = self.engine.redo_label().map(str::to_string);
            if ui
                .add_enabled(undo.is_some(), egui::Button::new(tr("Undo")).small())
                .on_hover_text(
                    undo.map_or_else(|| tr("Nothing to undo"), |l| format!("{} {l}", tr("Undo"))),
                )
                .clicked()
            {
                commands.push(CommandRef::new("edit.undo"));
            }
            if ui
                .add_enabled(redo.is_some(), egui::Button::new(tr("Redo")).small())
                .on_hover_text(
                    redo.map_or_else(|| tr("Nothing to redo"), |l| format!("{} {l}", tr("Redo"))),
                )
                .clicked()
            {
                commands.push(CommandRef::new("edit.redo"));
            }
            ui.separator();
            // Off macOS (or when the system menu bar refused), the menus live here.
            if self.native.is_none() {
                let laid_out = craft_menu::layout(
                    &self.menu_model(),
                    Target::current(),
                    &LayoutOptions::new(APP_NAME),
                )
                .bar;
                let label = |s: &'static str| tr(s);
                commands.extend(
                    MenuBarUi::new(&laid_out)
                        .label(&label)
                        .shrink_to_fit(360.0)
                        .show(ui),
                );
                ui.separator();
            }
            // The open plan's tab.
            if let Some(name) = self.engine.plan().map(|p| {
                std::path::Path::new(&p.path)
                    .file_name()
                    .map_or_else(|| p.path.clone(), |n| n.to_string_lossy().to_string())
            }) {
                let frame = egui::Frame::new()
                    .fill(if self.home {
                        TOKENS.tooltip
                    } else {
                        TOKENS.tab
                    })
                    .corner_radius(egui::CornerRadius::same(5))
                    .inner_margin(egui::Margin::symmetric(8, 2));
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Label::new(RichText::new(&name))
                                    .sense(egui::Sense::click())
                                    .truncate(),
                            )
                            .clicked()
                        {
                            self.home = false;
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text(tr("Close the plan"))
                            .clicked()
                        {
                            commands.push(CommandRef::new("file.close"));
                        }
                    });
                });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(right);
                if ui
                    .button("?")
                    .on_hover_text(tr("About CoverageCraft"))
                    .clicked()
                {
                    self.about = true;
                }
                ui.label(
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .color(TOKENS.text_dim)
                        .small(),
                );
            });
        });
        if let Some((family, layout)) = &self.window_buttons {
            let edges = ctx.content_rect();
            let bar = egui::Rect::from_x_y_ranges(edges.x_range(), rect.y_range());
            crate::captions::buttons(ui, bar, *family, layout);
        }
        for c in commands {
            self.menu_command(&ctx, &c);
        }
    }

    /// What the map shows: band, map, viewpoint, cut height, fit.
    pub(super) fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            // The floor shown, and a new one.
            let floors: Vec<String> = self.engine.floors().iter().map(|f| f.name.clone()).collect();
            let active = self.engine.active_floor();
            ui.label(RichText::new(tr("Floor")).color(TOKENS.text_dim));
            let mut pick = None;
            egui::ComboBox::from_id_salt("floor")
                .selected_text(floors.get(active).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (i, name) in floors.iter().enumerate() {
                        if ui.selectable_label(i == active, name).clicked() && i != active {
                            pick = Some(i);
                        }
                    }
                });
            if let Some(i) = pick {
                self.select_floor(&json!(i));
            }
            if ui
                .button(tr("Add floor"))
                .on_hover_text(tr("A floor above the highest; open its plan next"))
                .clicked()
            {
                self.add_floor();
            }
            ui.separator();
            ui.label(RichText::new(tr("Band")).color(TOKENS.text_dim));
            let band = self.engine.settings().band;
            for b in Band::ALL {
                if ui.selectable_label(band == b, format!("{} GHz", b.name())).clicked() && b != band {
                    self.run("settings.set", json!({"band": b.name()}));
                }
            }
            ui.separator();
            ui.label(RichText::new(tr("Map")).color(TOKENS.text_dim));
            let shown = self.engine.settings().view;
            egui::ComboBox::from_id_salt("map").selected_text(tr(shown.label())).show_ui(ui, |ui| {
                for m in Map::ALL {
                    if ui.selectable_label(m == shown, tr(m.label())).clicked() && m != shown {
                        self.run("settings.set", json!({"view": m.id()}));
                    }
                }
            });
            let views = self.engine.plan().map(|p| p.views.clone()).unwrap_or_default();
            if !views.is_empty() {
                ui.separator();
                ui.label(RichText::new(tr("Viewpoint")).color(TOKENS.text_dim));
                let current = self.engine.settings().viewpoint.clone();
                let shown = views.iter().find(|v| v.id == current).map_or(current.clone(), |v| v.name.clone());
                let mut pick = None;
                egui::ComboBox::from_id_salt("viewpoint").selected_text(tr(&shown)).show_ui(ui, |ui| {
                    for v in &views {
                        if ui.selectable_label(v.id == current, tr(&v.name)).clicked() && v.id != current {
                            pick = Some(v.id.clone());
                        }
                    }
                });
                if let Some(id) = pick {
                    self.set_viewpoint(&json!(id));
                }
            }
            // A model is cut: from the top at a height, from a side at a depth toward the viewer.
            let model = self.engine.plan().is_some_and(|p| {
                let l = p.path.to_ascii_lowercase();
                l.ends_with(".obj") || l.ends_with(".stl") || l.ends_with(".sc")
            });
            if model {
                ui.separator();
                let top = self.engine.settings().viewpoint == "top";
                ui.label(RichText::new(if top { tr("Cut height") } else { tr("Cut depth") }).color(TOKENS.text_dim))
                    .on_hover_text(tr("Where the model is cut to make the plan: from the top, the height above the floor; from a side, the distance toward you"));
                let r = ui.add(egui::DragValue::new(&mut self.slice).range(-100_000.0..=100_000.0).suffix(" mm").speed(10.0));
                if (r.drag_stopped() || r.lost_focus()) && self.slice != self.engine.settings().slice_height {
                    self.run("settings.set", json!({"slice_height": self.slice}));
                }
            }
            if self.engine.plan().is_some() {
                ui.separator();
                if ui.button(tr("Fit")).on_hover_text(tr("Fit the plan to the view")).clicked() {
                    self.view.fitted_to = None;
                    self.view.moved = false;
                }
            }
        });
    }

    /// The last message, the pointer's position, the scale and the viewpoint.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        // A SolidCraft design on its way: what, how long, and a way out.
        if let Some(p) = &self.pending {
            let name = std::path::Path::new(&p.design)
                .file_name()
                .map_or_else(|| p.design.clone(), |n| n.to_string_lossy().to_string());
            let secs = p.started.elapsed().as_secs();
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "{} {name} {} ({secs} s)",
                    tr("Opening"),
                    tr("with SolidCraft…")
                ));
                cancel = ui.small_button(tr("Cancel")).clicked();
            });
            if cancel {
                p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
        ui.horizontal(|ui| {
            let (text, error) = self.status.clone().unwrap_or_else(|| {
                (
                    tr("Drag a router to move it; scroll to zoom; drag the plan to pan."),
                    false,
                )
            });
            ui.add(
                egui::Label::new(RichText::new(text).color(if error {
                    TOKENS.error
                } else {
                    TOKENS.text_dim
                }))
                .truncate(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.engine.plan().is_none() {
                    return;
                }
                let dim = |s: String| RichText::new(s).color(TOKENS.text_dim).monospace();
                ui.label(dim(format!("{} GHz", self.engine.settings().band.name())));
                ui.separator();
                ui.label(dim(format!("1 pt = {:.0} mm", self.view.mm_per_pt)));
                if let Some([x, y]) = self.pointer {
                    ui.separator();
                    ui.label(dim(format!("{:.2} m, {:.2} m", x / 1000.0, y / 1000.0)));
                }
            });
        });
    }

    /// Home: open a plan or a project.
    pub(super) fn start_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(24.0));
            let logo = self.logo.get_or_insert_with(|| {
                ctx.load_texture(
                    "logo",
                    crate::logo_image(),
                    TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
                )
            });
            ui.add(egui::Image::new((logo.id(), egui::vec2(112.0, 112.0))));
            ui.add_space(8.0);
            ui.label(RichText::new(APP_NAME).size(28.0).strong());
            ui.label(
                RichText::new(tr(
                    "Plan Wi-Fi coverage on the floor plans and models you already have.",
                ))
                .color(TOKENS.text_dim),
            );
            ui.add_space(20.0);
            let busy = self.pending.is_some();
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(RichText::new(tr("Open a plan or model…")).size(16.0))
                        .min_size(egui::vec2(260.0, 36.0)),
                )
                .clicked()
            {
                self.open_dialog();
            }
            if busy {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                    ui.spinner();
                    ui.label(
                        RichText::new(tr("SolidCraft is exporting the design…"))
                            .color(TOKENS.text_dim),
                    );
                });
            }
            if let Some(name) = self.engine.plan().map(|p| p.path.clone()) {
                ui.add_space(6.0);
                let name = std::path::Path::new(&name)
                    .file_name()
                    .map_or(name.clone(), |n| n.to_string_lossy().to_string());
                if ui.button(format!("{} {name}", tr("Back to"))).clicked() {
                    self.home = false;
                }
            }
            ui.add_space(14.0);
            for line in [
                tr("CADCraft drawings (DXF and DWG), with their named views"),
                tr("SolidCraft designs (.sc), seen from any side or a saved view"),
                tr("OBJ and STL models, and CoverageCraft projects (.coveragecraft)"),
            ] {
                ui.label(RichText::new(line).color(TOKENS.text_dim).small());
            }
        });
    }

    /// About CoverageCraft.
    pub(super) fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.about;
        egui::Window::new(tr("About CoverageCraft"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.add(egui::Image::new((logo.id(), egui::vec2(72.0, 72.0))));
                    }
                    ui.label(RichText::new(APP_NAME).size(20.0).strong());
                    ui.label(format!("{} {}", tr("Version"), env!("CARGO_PKG_VERSION")));
                    ui.label(
                        RichText::new(tr("Wi-Fi coverage planning for the Craft suite. Beta."))
                            .color(TOKENS.text_dim),
                    );
                    ui.label(
                        RichText::new("MIT OR Apache-2.0")
                            .color(TOKENS.text_dim)
                            .small(),
                    );
                });
            });
        self.about = open;
    }
}

impl App {
    pub(super) fn select_floor(&mut self, index: &Value) {
        if self.run("floor.select", json!({"index": index})).is_some() {
            self.selected = None;
            self.slice = self.engine.settings().slice_height;
            self.view.fitted_to = None;
            self.view.moved = false;
        }
    }

    pub(super) fn add_floor(&mut self) {
        if let Some(r) = self.run("floor.add", json!({})) {
            self.selected = None;
            self.slice = self.engine.settings().slice_height;
            let index = r["index"]
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .unwrap_or(0);
            let name = self
                .engine
                .floors()
                .get(index)
                .map(|f| f.name.clone())
                .unwrap_or_default();
            self.status = Some((
                format!(
                    "{name}: {}",
                    tr("open its plan (File › Open), or keep it as a floor without walls")
                ),
                false,
            ));
        }
    }

    /// The shown floor: name, elevation, offset, slab; remove. Shown once there are two.
    pub(super) fn floor_section(&mut self, ui: &mut egui::Ui) {
        use coveragecraft_engine::materials::{Material, RF_PRESETS};
        let floors = self.engine.floors().to_vec();
        if floors.len() < 2 {
            return;
        }
        let i = self.engine.active_floor();
        let Some(f) = floors.get(i) else { return };
        ui.heading(tr("Floor"));
        let mut commands: Vec<Value> = Vec::new();
        egui::Grid::new("floor_grid").num_columns(2).show(ui, |ui| {
            ui.label(tr("Name"));
            if self.floor_name.as_ref().is_none_or(|(k, _)| *k != i) {
                self.floor_name = Some((i, f.name.clone()));
            }
            if let Some((_, buf)) = self.floor_name.as_mut() {
                let r = ui.add(egui::TextEdit::singleline(buf).desired_width(150.0));
                if r.lost_focus() && !buf.trim().is_empty() && *buf != f.name {
                    commands.push(json!({"index": i, "name": buf.trim()}));
                }
            }
            ui.end_row();
            ui.label(tr("Elevation"));
            let mut m = f.elevation / 1000.0;
            if ui
                .add(egui::DragValue::new(&mut m).speed(0.05).suffix(" m").range(-1000.0..=1000.0))
                .changed()
            {
                commands.push(json!({"index": i, "elevation": m * 1000.0}));
            }
            ui.end_row();
            ui.label(tr("Offset")).on_hover_text(tr(
                "Where this floor's plan sits in the building, so plans drawn at different origins line up",
            ));
            let mut o = f.offset;
            let changed = ui
                .horizontal(|ui| {
                    let x = ui
                        .add(egui::DragValue::new(&mut o[0]).speed(10.0).prefix("x ").suffix(" mm"))
                        .changed();
                    let y = ui
                        .add(egui::DragValue::new(&mut o[1]).speed(10.0).prefix("y ").suffix(" mm"))
                        .changed();
                    x || y
                })
                .inner;
            if changed {
                commands.push(json!({"index": i, "offset": o}));
            }
            ui.end_row();
            ui.label(tr("Slab below"));
            let shown = match &f.slab {
                Material::Preset(id) => tr(super::preset_label(id)),
                Material::Ignore => tr("None (an opening)"),
                _ => tr("Custom"),
            };
            egui::ComboBox::from_id_salt("slab").selected_text(shown).show_ui(ui, |ui| {
                for p in RF_PRESETS {
                    if ui
                        .selectable_label(f.slab == Material::Preset(p.id), tr(super::preset_label(p.id)))
                        .clicked()
                    {
                        commands.push(json!({"index": i, "slab": p.id}));
                    }
                }
                if ui
                    .selectable_label(f.slab == Material::Ignore, tr("None (an opening)"))
                    .clicked()
                {
                    commands.push(json!({"index": i, "slab": "ignore"}));
                }
            });
            ui.end_row();
        });
        if ui
            .small_button(tr("Remove this floor"))
            .on_hover_text(tr(
                "With its routers and measured points; Undo brings them back",
            ))
            .clicked()
        {
            if self.run("floor.remove", json!({"index": i})).is_some() {
                self.floor_name = None;
                self.selected = None;
                self.slice = self.engine.settings().slice_height;
                self.view.fitted_to = None;
            }
            return;
        }
        for c in commands {
            if self.run("floor.set", c).is_some() {
                self.floor_name = None;
            }
        }
        ui.separator();
    }
}
