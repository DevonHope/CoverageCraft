//! The fit to measured points in the right panel: what a calibration found, before it is used
//! (the walls' losses it would change, the error before, after and on points it wasn't fitted
//! to), and the calibrated wall losses in use, with Reset.

use egui::RichText;
use radiocraft_engine::materials::Band;
use serde_json::{Value, json};

use super::{App, preset_label};
use crate::i18n::tr;
use crate::theme::TOKENS;

impl App {
    /// A fit worked out by Calibrate…: shown until applied, discarded, or the design changes.
    pub(super) fn calibration_section(&mut self, ui: &mut egui::Ui) {
        if self
            .calibration
            .as_ref()
            .is_some_and(|(rev, _)| *rev != self.engine.revision())
        {
            self.calibration = None;
        }
        let Some((_, fit)) = self.calibration.clone() else {
            return;
        };
        let band = self.engine.settings().band.name();
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Calibration found")).strong());
        egui::Grid::new("calibration_fit").num_columns(2).show(ui, |ui| {
            ui.label(tr("Path-loss exponent"));
            ui.label(format!(
                "{:.2} {} {:.2}",
                self.engine.settings().exponent,
                tr("to"),
                fit["exponent"].as_f64().unwrap_or(0.0)
            ));
            ui.end_row();
            if let Some(o) = fit["offset_db"][band].as_f64() {
                ui.label(tr("Offset"));
                ui.label(format!("{o:+.1} dB"));
                ui.end_row();
            }
            for w in fit["walls"].as_array().into_iter().flatten() {
                if w["band"] != band {
                    continue;
                }
                let name = tr(preset_label(w["material"].as_str().unwrap_or_default()));
                let (prior, value) = (
                    w["prior_db"].as_f64().unwrap_or(0.0),
                    w["fitted_db"].as_f64().unwrap_or(0.0),
                );
                let points = w["points"].as_u64().unwrap_or(0);
                ui.label(name);
                let text = match w["status"].as_str() {
                    Some("kept") => format!("{prior:.1} dB ({})", tr("kept")),
                    Some("clamped") => format!("{prior:.1} {} {value:.1} dB ({})", tr("to"), tr("its limit")),
                    _ => format!("{prior:.1} {} {value:.1} dB", tr("to")),
                };
                let tip = if w["status"] == "kept" {
                    format!("{points} {}", tr("points measured through it: too few, or all alike, to fit it"))
                } else {
                    format!("{points} {}", tr("points measured through it"))
                };
                ui.label(text).on_hover_text(tip);
                ui.end_row();
            }
            let rms = |k: &str| fit[k][band]["rms_db"].as_f64();
            if let (Some(b), Some(a), Some(h)) = (rms("before"), rms("after"), rms("held_out")) {
                ui.label(tr("RMS error"));
                ui.label(format!("{b:.1} {} {a:.1} dB", tr("to")));
                ui.end_row();
                ui.label(tr("On points left out"))
                    .on_hover_text(tr(
                        "Each point predicted by a fit made without it: how well the fit will do at places not measured",
                    ));
                ui.label(format!("{h:.1} dB"));
                ui.end_row();
            }
        });
        let worse = fit["note"].is_string();
        if worse {
            ui.add(
                egui::Label::new(
                    RichText::new(tr(
                        "On points left out of the fit it does worse than the model as it is: better not to apply it",
                    ))
                    .color(TOKENS.error),
                )
                .wrap(),
            );
        }
        ui.horizontal(|ui| {
            let apply = ui.button(if worse {
                tr("Apply anyway")
            } else {
                tr("Apply")
            });
            if apply.clicked()
                && self
                    .run("survey.calibrate", json!({"apply": true}))
                    .is_some()
            {
                self.calibration = None;
                self.status = Some((tr("Calibration applied; Undo takes it back"), false));
            }
            if ui.button(tr("Discard")).clicked() {
                self.calibration = None;
            }
        });
    }

    /// The calibrated wall losses in use, each preset's per band, with Reset.
    pub(super) fn wall_losses_section(&mut self, ui: &mut egui::Ui) {
        let losses = self.engine.settings().loss_db.clone();
        if losses.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Calibrated wall losses")).strong())
            .on_hover_text(tr(
                "Fitted to your measured points, in place of the materials' usual losses in this project",
            ));
        let mut reset: Option<Value> = None;
        egui::Grid::new("wall_losses")
            .num_columns(3)
            .show(ui, |ui| {
                for (id, bands) in &losses {
                    let shown: Vec<String> = Band::ALL
                        .iter()
                        .filter_map(|b| {
                            bands
                                .get(b.index())
                                .copied()
                                .flatten()
                                .map(|v| format!("{} GHz {v:.1} dB", b.name()))
                        })
                        .collect();
                    ui.label(tr(preset_label(id)));
                    ui.label(RichText::new(shown.join(" · ")).color(TOKENS.text_dim));
                    if ui.small_button(tr("Reset")).clicked() {
                        reset = Some(json!({"loss_db": {id.as_str(): null}}));
                    }
                    ui.end_row();
                }
            });
        if ui.small_button(tr("Reset all")).clicked() {
            reset = Some(json!({"loss_db": {}}));
        }
        if let Some(p) = reset {
            self.run("settings.set", p);
        }
    }
}
