//! ED Corai's desktop window: shows what the MCP servers started by your AI apps are working on.
//! The station turns while a tool runs. The window only watches; the AI apps work the same
//! whether it is open or not.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod defaults_ui;
mod icon;
mod ship;
mod station;
mod updater;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use activity::{Activity, duration_label, tool_label};
use corai_activity::{Event, Listener};
use station::{ORANGE, Spin};

const README: &str = "https://github.com/kay-dutch/ed-corai#configuration";

struct App {
    activity: Arc<Mutex<Activity>>,
    ship: Arc<Mutex<Option<ship::CurrentShip>>>,
    updater: updater::Updater,
    defaults: defaults_ui::DefaultsUi,
    spin: Spin,
    glow: f32,
    last_frame: Instant,
    _listener: Option<Listener>,
    listen_error: Option<String>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let activity = Arc::new(Mutex::new(Activity::default()));
        let (listener, listen_error) = match corai_activity::discovery_path() {
            None => (
                None,
                Some("No user folder to announce the window in.".to_string()),
            ),
            Some(path) => {
                let (shared, ctx) = (Arc::clone(&activity), ctx.clone());
                match Listener::start(&path, move |event| {
                    if event == Event::FocusRequested {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    } else if let Ok(mut a) = shared.lock() {
                        a.apply(event, Instant::now());
                    }
                    ctx.request_repaint();
                }) {
                    Ok(l) => (Some(l), None),
                    Err(e) => (None, Some(format!("Cannot listen for AI apps: {e}"))),
                }
            }
        };
        let ship = Arc::new(Mutex::new(None));
        watch_journal(Arc::clone(&ship), ctx.clone());
        Self {
            activity,
            ship,
            updater: updater::Updater::start(ctx),
            defaults: defaults_ui::DefaultsUi::load(),
            spin: Spin::default(),
            glow: 0.0,
            last_frame: Instant::now(),
            _listener: listener,
            listen_error,
        }
    }
}

/// Polls the journal every few seconds and keeps `ship` current.
fn watch_journal(ship: Arc<Mutex<Option<ship::CurrentShip>>>, ctx: egui::Context) {
    let _ = std::thread::Builder::new()
        .name("journal".into())
        .spawn(move || {
            let mut watcher = ship::Watcher::default();
            loop {
                if let Some(dir) = corai_journal::journal_dir()
                    && let Some(now) = watcher.poll(&dir)
                {
                    if let Ok(mut s) = ship.lock() {
                        *s = now;
                    }
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        });
}

/// "just now", "5 min ago", "2 h ago".
fn ago(at: SystemTime) -> String {
    let s = at.elapsed().unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        let Ok(act) = self.activity.lock() else {
            return;
        };
        let busy = act.busy();
        self.spin.tick(busy, dt);
        let target = if busy { 1.0 } else { 0.0 };
        self.glow += (target - self.glow) * (dt * 4.0).min(1.0);
        if self.spin.moving() || busy || (self.glow - target).abs() > 0.01 {
            ui.ctx().request_repaint();
        } else {
            // Keeps "5 min ago" current.
            ui.ctx().request_repaint_after(Duration::from_secs(20));
        }
        let weak = ui.visuals().weak_text_color();

        self.updater.banner(ui);
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("ED Corai {}", env!("CARGO_PKG_VERSION")))
                        .color(weak),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to("Set up an AI app", README);
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                station::paint(ui, &self.spin, self.glow, 150.0);
                ui.add_space(8.0);
                if let Some(e) = &self.listen_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 110), e);
                }
                match act.jobs.values().next() {
                    Some(job) => {
                        ui.label(egui::RichText::new(tool_label(&job.tool)).strong().color(ORANGE).size(16.0));
                        if !job.detail.is_empty() {
                            ui.label(&job.detail);
                        }
                        let more = act.jobs.len() - 1;
                        let extra = if more > 0 { format!(" · {more} more running") } else { String::new() };
                        ui.label(
                            egui::RichText::new(format!("{} · {}{extra}", job.client, duration_label(job.started.elapsed())))
                                .color(weak),
                        );
                    }
                    None => {
                        ui.label(egui::RichText::new("Standing by").strong().size(16.0));
                        let names = act.client_names();
                        ui.label(egui::RichText::new(if names.is_empty() {
                            "No AI app connected. Ask your assistant for a build and it appears here.".to_string()
                        } else {
                            format!("Connected: {}", names.join(", "))
                        })
                        .color(weak));
                    }
                }
                if let Some(ship) = self.ship.lock().ok().and_then(|s| s.clone()) {
                    ui.add_space(6.0);
                    let mut text = format!("Flying: {}", ship.model);
                    if !ship.name.is_empty() {
                        text += &format!(" \u{201c}{}\u{201d}", ship.name);
                    }
                    if !ship.ident.is_empty() {
                        text += &format!(" ({})", ship.ident);
                    }
                    ui.label(text);
                    if let Some(link) = &ship.link {
                        ui.hyperlink_to("Open in Coriolis", link);
                    }
                }
            });
            ui.add_space(10.0);
            self.defaults.show(ui);
            ui.separator();
            if act.history.is_empty() {
                return;
            }
            ui.label(egui::RichText::new("Recent").strong());
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for done in &act.history {
                    ui.horizontal_wrapped(|ui| {
                        let (mark, colour) = if done.ok {
                            ("✔", egui::Color32::from_rgb(120, 220, 140))
                        } else {
                            ("✖", egui::Color32::from_rgb(255, 110, 110))
                        };
                        ui.colored_label(colour, mark);
                        ui.label(egui::RichText::new(tool_label(&done.tool)).strong());
                        if !done.detail.is_empty() {
                            ui.label(&done.detail);
                        }
                        if let Some(link) = &done.link {
                            ui.hyperlink_to("Coriolis", link);
                        }
                    });
                    ui.label(
                        egui::RichText::new(format!("{} · {} · {}", done.client, duration_label(done.took), ago(done.at)))
                            .small()
                            .color(weak),
                    );
                    ui.add_space(4.0);
                }
            });
        });
    }
}

fn main() -> eframe::Result {
    corai_update::cleanup_previous();
    // One window: a second start brings the first to the front instead. After an update the new
    // window starts with --replace while the old one closes.
    let replacing = std::env::args().any(|a| a == "--replace");
    if !replacing
        && corai_activity::discovery_path()
            .is_some_and(|p| corai_activity::focus_running_window(&p))
    {
        return Ok(());
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("ED Corai {}", env!("CARGO_PKG_VERSION")))
        .with_inner_size([440.0, 600.0])
        .with_min_inner_size([340.0, 420.0])
        .with_icon(Arc::new(egui::IconData {
            rgba: icon::rgba(128),
            width: 128,
            height: 128,
        }));
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "ED Corai",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    )
}
