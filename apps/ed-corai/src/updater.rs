//! The update banner: checks GitHub at start, installs a new release on request (both programs)
//! and restarts the window.

use std::sync::{Arc, Mutex};

/// Where the update stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Quiet,
    Available {
        version: String,
        page: String,
        can_install: bool,
    },
    Installing {
        version: String,
    },
    Installed {
        version: String,
    },
    Failed(String),
}

/// The phase a release check leads to (`check` is `corai_update::check_for_updates()`).
pub fn phase_from_check(check: &serde_json::Value) -> Phase {
    if check["updateAvailable"] != true {
        return Phase::Quiet;
    }
    Phase::Available {
        version: check["latest"]["version"]
            .as_str()
            .unwrap_or_default()
            .into(),
        page: check["latest"]["url"].as_str().unwrap_or_default().into(),
        can_install: check["canAutoInstall"] == true,
    }
}

#[derive(Debug, Clone)]
pub struct Updater {
    phase: Arc<Mutex<Phase>>,
}

impl Updater {
    /// Starts the release check in the background (skipped with `CORAI_UPDATE_CHECK=0`).
    pub fn start(ctx: &egui::Context) -> Self {
        let phase = Arc::new(Mutex::new(Phase::Quiet));
        if std::env::var("CORAI_UPDATE_CHECK").as_deref() != Ok("0") {
            let (phase, ctx) = (Arc::clone(&phase), ctx.clone());
            std::thread::spawn(move || {
                let next = phase_from_check(&corai_update::check_for_updates());
                if let Ok(mut p) = phase.lock() {
                    *p = next;
                }
                ctx.request_repaint();
            });
        }
        Self { phase }
    }

    fn get(&self) -> Phase {
        self.phase.lock().map(|p| p.clone()).unwrap_or(Phase::Quiet)
    }

    fn set(&self, next: Phase) {
        if let Ok(mut p) = self.phase.lock() {
            *p = next;
        }
    }

    fn install(&self, version: String, ctx: &egui::Context) {
        self.set(Phase::Installing {
            version: version.clone(),
        });
        let (this, ctx) = (self.clone(), ctx.clone());
        std::thread::spawn(move || {
            let tag = format!("v{version}");
            this.set(match corai_update::install_update(Some(&tag)) {
                Ok(_) => Phase::Installed { version },
                Err(e) => Phase::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    /// Draws the banner when there is something to say.
    pub fn banner(&self, ui: &mut egui::Ui) {
        let phase = self.get();
        if phase == Phase::Quiet {
            return;
        }
        egui::Panel::top("update").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| match &phase {
                Phase::Quiet => {}
                Phase::Available { version, page, can_install } => {
                    ui.label(format!("ED Corai {version} is available."));
                    if *can_install && ui.button("Install").clicked() {
                        self.install(version.clone(), ui.ctx());
                    }
                    ui.hyperlink_to(if *can_install { "What's new" } else { "Download" }, page);
                }
                Phase::Installing { version } => {
                    ui.spinner();
                    ui.label(format!("Installing ED Corai {version}…"));
                }
                Phase::Installed { version } => {
                    ui.label(format!(
                        "ED Corai {version} is installed. Restart your AI app's ED Corai server too (Claude Code: /mcp)."
                    ));
                    if ui.button("Restart window").clicked() {
                        restart(ui.ctx());
                    }
                }
                Phase::Failed(e) => {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 110), format!("Update failed: {e}"));
                }
            });
        });
    }
}

/// Starts the new window (it takes over the announcement) and closes this one.
fn restart(ctx: &egui::Context) {
    let Some(exe) = corai_update::program_dir().map(|d| d.join(corai_update::window_name())) else {
        return;
    };
    if std::process::Command::new(exe)
        .arg("--replace")
        .spawn()
        .is_ok()
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_newer_release_offers_itself() {
        assert_eq!(
            phase_from_check(&json!({ "updateAvailable": false })),
            Phase::Quiet
        );
        assert_eq!(
            phase_from_check(&json!({ "message": "Could not check" })),
            Phase::Quiet
        );
        assert_eq!(
            phase_from_check(&json!({ "updateAvailable": true, "canAutoInstall": true,
                "latest": { "version": "0.3.0", "url": "https://github.com/kay-dutch/ed-corai/releases/tag/v0.3.0" } })),
            Phase::Available {
                version: "0.3.0".into(),
                page: "https://github.com/kay-dutch/ed-corai/releases/tag/v0.3.0".into(),
                can_install: true
            }
        );
    }
}
