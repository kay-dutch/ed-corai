//! The Coriolis station: a hexagon that turns while ED Corai is working and comes to rest upright.

use std::f32::consts::TAU;

/// One sixth of a turn: the hexagon looks the same after it, so it rests on a multiple of it.
const SIXTH: f32 = TAU / 6.0;
/// Full speed while busy, radians per second (a turn in six seconds).
const BUSY_SPEED: f32 = TAU / 6.0;
/// How fast it spins up, radians per second squared.
const SPIN_UP: f32 = BUSY_SPEED * 1.5;
/// Slowest approach to the resting angle, so it never crawls for ever.
const MIN_SETTLE_SPEED: f32 = 0.05;

/// Rotation state. Turns one way only, so the station never visibly backs up.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spin {
    pub angle: f32,
    pub speed: f32,
}

impl Spin {
    /// Advances by `dt` seconds: spins up while `busy`, otherwise slows into the next resting angle.
    pub fn tick(&mut self, busy: bool, dt: f32) {
        let dt = dt.clamp(0.0, 0.1); // a stalled frame must not fling it round
        if busy {
            self.speed = (self.speed + SPIN_UP * dt).min(BUSY_SPEED);
            self.angle = (self.angle + self.speed * dt) % TAU;
            return;
        }
        if self.speed == 0.0 {
            return;
        }
        let rest = (self.angle / SIXTH).ceil() * SIXTH;
        let remaining = rest - self.angle;
        // Ease out: speed proportional to what is left, never faster than it was going.
        self.speed = self.speed.min(remaining * 3.0).max(MIN_SETTLE_SPEED);
        let step = self.speed * dt;
        if step >= remaining {
            self.angle = rest % TAU;
            self.speed = 0.0;
        } else {
            self.angle += step;
        }
    }

    /// Whether it still moves (and frames must keep coming).
    pub fn moving(&self) -> bool {
        self.speed > 0.0
    }
}

/// The six corners of a hexagon of `radius` around `center`, turned by `angle`; flat side on top
/// at rest, as a station seen face-on.
pub fn hexagon(center: egui::Pos2, radius: f32, angle: f32) -> Vec<egui::Pos2> {
    (0..6)
        .map(|i| {
            let a = angle + i as f32 * SIXTH;
            center + radius * egui::vec2(a.cos(), a.sin())
        })
        .collect()
}

/// ED's HUD orange.
pub const ORANGE: egui::Color32 = egui::Color32::from_rgb(255, 113, 0);

/// Paints the station into a square of `size` points. `glow` (0..1) lights the docking slot.
pub fn paint(ui: &mut egui::Ui, spin: &Spin, glow: f32, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let c = rect.center();
    let r = size * 0.45;
    let dim = egui::Color32::from_rgba_unmultiplied(255, 113, 0, 70);

    // Body: outer hull, an inner ring for depth, struts joining their corners.
    let outer = hexagon(c, r, spin.angle);
    let inner = hexagon(c, r * 0.62, spin.angle);
    p.add(egui::Shape::convex_polygon(
        outer.clone(),
        egui::Color32::from_rgb(28, 22, 18),
        egui::Stroke::new(2.0, ORANGE),
    ));
    p.add(egui::Shape::closed_line(
        inner.clone(),
        egui::Stroke::new(1.0, dim),
    ));
    for (a, b) in outer.iter().zip(&inner) {
        p.line_segment([*a, *b], egui::Stroke::new(1.0, dim));
    }

    // The docking slot: a letterbox that turns with the station.
    let (w, h) = (r * 0.5, r * 0.16);
    let (sin, cos) = spin.angle.sin_cos();
    let corner = |x: f32, y: f32| c + egui::vec2(x * cos - y * sin, x * sin + y * cos);
    let slot = vec![
        corner(-w / 2.0, -h / 2.0),
        corner(w / 2.0, -h / 2.0),
        corner(w / 2.0, h / 2.0),
        corner(-w / 2.0, h / 2.0),
    ];
    let light = egui::Color32::from_rgb(
        (40.0 + 140.0 * glow) as u8,
        (60.0 + 160.0 * glow) as u8,
        (70.0 + 185.0 * glow) as u8,
    );
    p.add(egui::Shape::convex_polygon(
        slot,
        light,
        egui::Stroke::new(1.0, ORANGE),
    ));

    // Navigation lights on the corners.
    for v in &outer {
        p.circle_filled(*v, 2.5, ORANGE);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(spin: &mut Spin, busy: bool, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            spin.tick(busy, 1.0 / 60.0);
        }
    }

    #[test]
    fn it_turns_while_busy_and_rests_upright_afterwards() {
        let mut spin = Spin::default();
        run(&mut spin, false, 1.0);
        assert_eq!(spin, Spin::default(), "idle and at rest: nothing moves");
        run(&mut spin, true, 2.3);
        assert!(spin.moving());
        assert!((spin.speed - BUSY_SPEED).abs() < 1e-4, "reaches full speed");
        let before = spin.angle;
        let mut last = spin.angle;
        for _ in 0..600 {
            spin.tick(false, 1.0 / 60.0);
            // Forward step, across the wrap at a full turn; a step back shows up as nearly a turn.
            let step = (spin.angle - last).rem_euclid(TAU);
            assert!(step < 0.1, "never turns backwards or jumps: step {step}");
            last = spin.angle;
        }
        assert!(!spin.moving(), "settles within ten seconds");
        let sixths = spin.angle / SIXTH;
        assert!(
            (sixths - sixths.round()).abs() < 1e-4,
            "rests on a flat side: {sixths}"
        );
        assert!(spin.angle != before);
    }

    #[test]
    fn a_long_frame_does_not_jump() {
        let mut spin = Spin::default();
        spin.tick(true, 5.0);
        assert!(spin.angle <= BUSY_SPEED * 0.1 + 1e-6);
    }

    #[test]
    fn the_hexagon_has_its_corners_on_the_circle() {
        let c = egui::pos2(10.0, 10.0);
        let pts = hexagon(c, 5.0, 0.3);
        assert_eq!(pts.len(), 6);
        for p in pts {
            assert!(((p - c).length() - 5.0).abs() < 1e-4);
        }
    }
}
