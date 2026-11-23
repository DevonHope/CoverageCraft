//! Suggested channels in the routers panel: what `channel.suggest` would set, router by router,
//! and the share of the floor with no same-channel router heard before and after, shown before
//! anything changes; Apply sets them in one undoable step.

use egui::RichText;
use serde_json::{Value, json};

use super::App;
use crate::i18n::tr;
use crate::theme::TOKENS;

/// A channel as the panel says it: `36 (80 MHz)`, or "not set".
fn channel_text(v: &Value) -> String {
    match (v["channel"].as_u64(), v["width"].as_u64()) {
        (Some(c), Some(w)) => format!("{c} ({w} MHz)"),
        _ => tr("not set"),
    }
}

impl App {
    /// Work out channels for the shown band and show them.
    pub(super) fn suggest_channels(&mut self) {
        let dfs = self.channel_dfs;
        if let Some(r) = self.run("channel.suggest", json!({"dfs": dfs})) {
            self.channel_plan = Some((self.engine.revision(), r));
        }
        self.home = false;
    }

    /// The suggestion, until applied, discarded, or the design changes.
    pub(super) fn channels_section(&mut self, ui: &mut egui::Ui) {
        if self
            .channel_plan
            .as_ref()
            .is_some_and(|(rev, _)| *rev != self.engine.revision())
        {
            self.channel_plan = None;
        }
        let Some((_, plan)) = self.channel_plan.clone() else {
            return;
        };
        ui.separator();
        ui.label(
            RichText::new(format!(
                "{} ({} GHz)",
                tr("Suggested channels"),
                plan["band"].as_str().unwrap_or_default()
            ))
            .strong(),
        );
        egui::Grid::new("channel_plan")
            .num_columns(2)
            .show(ui, |ui| {
                for r in plan["routers"].as_array().into_iter().flatten() {
                    ui.label(r["name"].as_str().unwrap_or_default());
                    let (from, to) = (channel_text(&r["from"]), channel_text(&r["to"]));
                    if r["changed"].as_bool().unwrap_or(false) {
                        ui.label(format!("{from} {} {to}", tr("to")));
                    } else {
                        ui.label(
                            RichText::new(format!("{to} ({})", tr("stays"))).color(TOKENS.text_dim),
                        );
                    }
                    ui.end_row();
                }
            });
        if let (Some(b), Some(a)) = (
            plan["before_pct_no_channel_overlap"].as_f64(),
            plan["after_pct_no_channel_overlap"].as_f64(),
        ) {
            ui.label(format!(
                "{}: {b:.0}% {} {a:.0}%",
                tr("Floor with no same-channel router heard"),
                tr("to")
            ))
            .on_hover_text(tr(
                "Where a device hears no other router on a channel overlapping its own router's, at −82 dBm or more; every floor counts in the top view, only the shown one from a side",
            ));
        }
        let mut notes: Vec<String> = Vec::new();
        if let (Some(r), Some(c)) = (
            plan["shortage"]["routers"].as_u64(),
            plan["shortage"]["channels"].as_u64(),
        ) {
            notes.push(format!(
                "{r} {} {c} {}",
                tr("routers and as few as"),
                tr("channels at their width: routers that hear each other may have to share one. Narrower channels give more."),
            ));
        }
        if let Some(names) = plan["no_channel"].as_array() {
            let names: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
            notes.push(format!(
                "{}: {}",
                tr("No allowed channel at its width for"),
                names.join(", ")
            ));
        }
        for n in notes {
            ui.add(egui::Label::new(RichText::new(n).color(TOKENS.text_dim).small()).wrap());
        }
        if plan["band"] == "5" {
            let mut dfs = self.channel_dfs;
            if ui
                .checkbox(&mut dfs, tr("Allow DFS channels (52–144)"))
                .on_hover_text(tr(
                    "More 5 GHz channels; a router must leave one when it detects radar, and not every country or router allows them",
                ))
                .changed()
            {
                self.channel_dfs = dfs;
                self.suggest_channels();
                return;
            }
        }
        ui.horizontal(|ui| {
            let changes = plan["routers"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["changed"] == true));
            if ui
                .add_enabled(changes, egui::Button::new(tr("Apply")))
                .clicked()
                && self
                    .run(
                        "channel.suggest",
                        json!({"band": plan["band"], "dfs": self.channel_dfs, "apply": true}),
                    )
                    .is_some()
            {
                self.channel_plan = None;
                self.status = Some((tr("Channels set; Undo takes them back"), false));
            }
            if ui.button(tr("Discard")).clicked() {
                self.channel_plan = None;
            }
        });
    }
}
