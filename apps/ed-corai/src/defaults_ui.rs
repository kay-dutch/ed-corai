//! "Defaults for your AI app": how the player flies, used by every ED Corai server for settings a
//! request does not mention. Saved as soon as one changes.

use corai_activity::defaults::{Defaults, defaults_path};

const PIPS: [f64; 9] = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0];
const UNSET: &str = "assistant decides";

#[derive(Debug)]
pub struct DefaultsUi {
    defaults: Defaults,
    error: Option<String>,
}

impl DefaultsUi {
    pub fn load() -> Self {
        Self {
            defaults: defaults_path()
                .map(|p| Defaults::load(&p))
                .unwrap_or_default(),
            error: None,
        }
    }

    fn save(&mut self) {
        self.error = match defaults_path() {
            None => Some("No user folder to save in.".into()),
            Some(p) => self
                .defaults
                .save(&p)
                .err()
                .map(|e| format!("Not saved: {e}")),
        };
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        egui::CollapsingHeader::new("Defaults for your AI app")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Used when a request does not say otherwise.")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                egui::Grid::new("defaults").num_columns(2).show(ui, |ui| {
                    ui.label("SYS pips in combat");
                    changed |= pips(ui, "sys", &mut self.defaults.sys_pips);
                    ui.end_row();
                    ui.label("WEP pips in combat");
                    changed |= pips(ui, "wep", &mut self.defaults.wep_pips);
                    ui.end_row();
                    ui.label("Incoming damage");
                    changed |= choice(
                        ui,
                        "damage",
                        &mut self.defaults.damage_preset,
                        &[("even", "even mix"), ("pve", "NPC (thermal-heavy)")],
                    );
                    ui.end_row();
                    ui.label("Engineering");
                    changed |= choice(
                        ui,
                        "engineering",
                        &mut self.defaults.engineering,
                        &[("full", "G5 + experimentals"), ("none", "stock modules")],
                    );
                    ui.end_row();
                });
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 110), e);
                }
            });
        if changed {
            self.save();
        }
    }
}

fn pips(ui: &mut egui::Ui, id: &str, value: &mut Option<f64>) -> bool {
    let shown = value.map_or_else(|| UNSET.to_string(), |v| v.to_string());
    let before = *value;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, UNSET);
            for p in PIPS {
                ui.selectable_value(value, Some(p), p.to_string());
            }
        });
    *value != before
}

fn choice(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut Option<String>,
    options: &[(&str, &str)],
) -> bool {
    let shown = value
        .as_deref()
        .and_then(|v| options.iter().find(|(k, _)| *k == v))
        .map_or(UNSET, |(_, label)| label);
    let before = value.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, UNSET);
            for (key, label) in options {
                ui.selectable_value(value, Some((*key).to_string()), *label);
            }
        });
    *value != before
}
