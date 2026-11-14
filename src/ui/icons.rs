//! Original monochrome line icons drawn as vector paths.
//! Each takes the icon's bounding square.

use egui::epaint::PathShape;
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2, vec2};

const STROKE: f32 = 1.5;

fn p(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + x * r.width(), r.top() + y * r.height())
}

/// ▶ filled triangle.
pub fn play(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![p(r, 0.25, 0.15), p(r, 0.85, 0.5), p(r, 0.25, 0.85)];
    painter.add(PathShape::convex_polygon(pts, color, Stroke::NONE));
}

/// Repeat: a frame, open at its lower right corner, with a
/// ▶ there.
pub fn repeat(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let frame = vec![p(r, 0.42, 0.72), p(r, 0.05, 0.72), p(r, 0.05, 0.1), p(r, 0.8, 0.1), p(r, 0.8, 0.38)];
    painter.add(PathShape::line(frame, s));
    let tri = vec![p(r, 0.52, 0.48), p(r, 1.0, 0.74), p(r, 0.52, 1.0)];
    painter.add(PathShape::convex_polygon(tri, color, Stroke::NONE));
}

/// Trigger mode: a rising edge with an up arrow on it.
pub fn trigger_edge(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let edge = vec![p(r, 0.0, 0.88), p(r, 0.45, 0.88), p(r, 0.45, 0.12), p(r, 1.0, 0.12)];
    painter.add(PathShape::line(edge, s));
    let head = vec![p(r, 0.45, 0.3), p(r, 0.68, 0.58), p(r, 0.22, 0.58)];
    painter.add(PathShape::convex_polygon(head, color, Stroke::NONE));
}

/// Timed mode: a stopwatch.
pub fn stopwatch(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let c = p(r, 0.5, 0.57);
    let rad = r.width() * 0.4;
    painter.circle_stroke(c, rad, s);
    painter.line_segment([p(r, 0.5, 0.0), p(r, 0.5, 0.17)], s);
    painter.line_segment([p(r, 0.36, 0.0), p(r, 0.64, 0.0)], s);
    painter.line_segment([c, c + vec2(rad * 0.45, -rad * 0.55)], s);
}

/// Continuous mode: ∞.
pub fn infinity(painter: &Painter, r: Rect, color: Color32) {
    // Lemniscate of Bernoulli: round lobes like the glyph's.
    let pts: Vec<Pos2> = (0..48)
        .map(|i| {
            let t = i as f32 / 48.0 * std::f32::consts::TAU;
            let d = 1.0 + t.sin() * t.sin();
            p(r, 0.5 + 0.44 * t.cos() / d, 0.5 + 0.62 * t.sin() * t.cos() / d)
        })
        .collect();
    painter.add(PathShape::closed_line(pts, Stroke::new(STROKE, color)));
}

/// A funnel (Filter).
pub fn funnel(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![p(r, 0.05, 0.1), p(r, 0.95, 0.1), p(r, 0.6, 0.52), p(r, 0.6, 0.92), p(r, 0.4, 0.8), p(r, 0.4, 0.52)];
    painter.add(PathShape::closed_line(pts, Stroke::new(STROKE, color)));
}

/// ■ filled square.
/// Two bars (Pause).
pub fn pause(painter: &Painter, r: Rect, color: Color32) {
    for x in [0.22, 0.62] {
        painter.rect_filled(Rect::from_min_max(p(r, x, 0.1), p(r, x + 0.16, 0.9)), 1.0, color);
    }
}

pub fn stop(painter: &Painter, r: Rect, color: Color32) {
    painter.rect_filled(Rect::from_center_size(r.center(), r.size() * 0.6), 1.0, color);
}

/// ✕
pub fn close(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.25, 0.25), p(r, 0.75, 0.75)], s);
    painter.line_segment([p(r, 0.75, 0.25), p(r, 0.25, 0.75)], s);
}

/// ⓘ
pub fn info(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.circle_stroke(r.center(), r.width() * 0.42, s);
    painter.line_segment([p(r, 0.5, 0.45), p(r, 0.5, 0.72)], s);
    painter.circle_filled(p(r, 0.5, 0.3), STROKE * 0.7, color);
}

/// ✓ in a circle.
pub fn success(painter: &Painter, r: Rect, color: Color32) {
    painter.circle_stroke(r.center(), r.width() * 0.42, Stroke::new(STROKE, color));
    painter.line(vec![p(r, 0.3, 0.52), p(r, 0.45, 0.66), p(r, 0.72, 0.36)], Stroke::new(STROKE, color));
}

/// ! in a triangle.
pub fn warning(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.add(PathShape::closed_line(vec![p(r, 0.5, 0.1), p(r, 0.92, 0.88), p(r, 0.08, 0.88)], s));
    painter.line_segment([p(r, 0.5, 0.38), p(r, 0.5, 0.62)], s);
    painter.circle_filled(p(r, 0.5, 0.75), STROKE * 0.7, color);
}

/// ! in a circle (error).
pub fn error(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.circle_stroke(r.center(), r.width() * 0.42, s);
    painter.line_segment([p(r, 0.5, 0.28), p(r, 0.5, 0.56)], s);
    painter.circle_filled(p(r, 0.5, 0.7), STROKE * 0.7, color);
}

/// A generic logic analyzer: a box with probe pins and a cable stub.
pub fn device(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let body = Rect::from_min_max(p(r, 0.12, 0.25), p(r, 0.88, 0.75));
    painter.rect_stroke(body, 3.0, s, egui::StrokeKind::Middle);
    for i in 0..6 {
        let x = 0.22 + i as f32 * 0.112;
        painter.line_segment([p(r, x, 0.75), p(r, x, 0.9)], s);
    }
    painter.line_segment([p(r, 0.5, 0.25), p(r, 0.5, 0.08)], s);
    painter.circle_filled(p(r, 0.24, 0.42), STROKE, color);
}

/// ◂ small left-pointing triangle.
pub fn left_arrow(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let h = size / 2.0;
    let pts = vec![center + vec2(h * 0.6, -h), center + vec2(h * 0.6, h), center + vec2(-h * 0.6, 0.0)];
    painter.add(PathShape::convex_polygon(pts, color, Stroke::NONE));
}

/// A digital pulse glyph (used in sample-rate rows and the start screen).
pub fn pulse(painter: &Painter, r: Rect, color: Color32) {
    let pts =
        vec![p(r, 0.05, 0.75), p(r, 0.3, 0.75), p(r, 0.3, 0.25), p(r, 0.7, 0.25), p(r, 0.7, 0.75), p(r, 0.95, 0.75)];
    painter.line(pts, Stroke::new(STROKE, color));
}

/// A magnifier (zoom tool).
fn magnifier(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.circle_stroke(p(r, 0.42, 0.42), r.width() * 0.26, s);
    painter.line_segment([p(r, 0.61, 0.61), p(r, 0.88, 0.88)], s);
}

/// Range zoom along time: a magnifier over a horizontal span.
pub fn zoom_x(painter: &Painter, r: Rect, color: Color32) {
    magnifier(painter, r, color);
    let s = Stroke::new(1.0, color);
    painter.line_segment([p(r, 0.28, 0.42), p(r, 0.56, 0.42)], s);
}

/// Range zoom along value: a magnifier over a vertical span.
pub fn zoom_y(painter: &Painter, r: Rect, color: Color32) {
    magnifier(painter, r, color);
    let s = Stroke::new(1.0, color);
    painter.line_segment([p(r, 0.42, 0.28), p(r, 0.42, 0.56)], s);
}

/// Range zoom in both axes: a magnifier over a small box.
pub fn zoom_xy(painter: &Painter, r: Rect, color: Color32) {
    magnifier(painter, r, color);
    let s = Stroke::new(1.0, color);
    painter.rect_stroke(Rect::from_min_max(p(r, 0.32, 0.32), p(r, 0.52, 0.52)), 0.0, s, egui::StrokeKind::Middle);
}

/// ‹ chevron (previous view).
pub fn chevron_left(painter: &Painter, r: Rect, color: Color32) {
    painter.line(vec![p(r, 0.62, 0.22), p(r, 0.36, 0.5), p(r, 0.62, 0.78)], Stroke::new(STROKE, color));
}

/// › chevron (next view).
pub fn chevron_right(painter: &Painter, r: Rect, color: Color32) {
    painter.line(vec![p(r, 0.38, 0.22), p(r, 0.64, 0.5), p(r, 0.38, 0.78)], Stroke::new(STROKE, color));
}

/// ⌄ small chevron (split-button menus).
pub fn chevron_down(painter: &Painter, r: Rect, color: Color32) {
    painter.line(vec![p(r, 0.25, 0.4), p(r, 0.5, 0.62), p(r, 0.75, 0.4)], Stroke::new(STROKE, color));
}

/// Jump to an edge: an arrow into a vertical bar; `next` points right.
pub fn jump_edge(painter: &Painter, r: Rect, next: bool, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let (x0, x1, bar) = if next { (0.2, 0.62, 0.78) } else { (0.8, 0.38, 0.22) };
    painter.line_segment([p(r, x0, 0.5), p(r, x1, 0.5)], s);
    let k = if next { -0.16 } else { 0.16 };
    painter.line(vec![p(r, x1 + k, 0.34), p(r, x1, 0.5), p(r, x1 + k, 0.66)], s);
    painter.line_segment([p(r, bar, 0.22), p(r, bar, 0.78)], s);
}

/// ✓ check mark (checkable menu items).
pub fn check(painter: &Painter, r: Rect, color: Color32) {
    painter.line(vec![p(r, 0.2, 0.52), p(r, 0.42, 0.74), p(r, 0.8, 0.28)], Stroke::new(STROKE, color));
}

/// A flag on a pole (Add Timing Marker).
pub fn flag(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.25, 0.12), p(r, 0.25, 0.9)], s);
    let pts = vec![p(r, 0.25, 0.14), p(r, 0.82, 0.3), p(r, 0.25, 0.48)];
    painter.add(PathShape::convex_polygon(pts, Color32::TRANSPARENT, s));
}

/// A filled pennant (marker swatches).
pub fn pennant(painter: &Painter, r: Rect, color: Color32) {
    painter.line_segment([p(r, 0.2, 0.05), p(r, 0.2, 0.95)], Stroke::new(STROKE, color));
    let pts = vec![p(r, 0.2, 0.05), p(r, 0.9, 0.3), p(r, 0.2, 0.55)];
    painter.add(PathShape::convex_polygon(pts, color, Stroke::NONE));
}

/// A trash can.
pub fn trash(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.18, 0.25), p(r, 0.82, 0.25)], s);
    painter.line_segment([p(r, 0.4, 0.12), p(r, 0.6, 0.12)], s);
    painter.line(vec![p(r, 0.26, 0.25), p(r, 0.32, 0.88), p(r, 0.68, 0.88), p(r, 0.74, 0.25)], s);
    painter.line_segment([p(r, 0.44, 0.4), p(r, 0.44, 0.75)], Stroke::new(1.0, color));
    painter.line_segment([p(r, 0.56, 0.4), p(r, 0.56, 0.75)], Stroke::new(1.0, color));
}

/// Two overlapping sheets (Copy).
pub fn copy(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.rect_stroke(Rect::from_min_max(p(r, 0.35, 0.3), p(r, 0.85, 0.88)), 1.5, s, egui::StrokeKind::Middle);
    painter.line(vec![p(r, 0.2, 0.7), p(r, 0.2, 0.15), p(r, 0.65, 0.15)], s);
}

/// A file with a folded corner.
pub fn file(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(
        vec![p(r, 0.62, 0.1), p(r, 0.22, 0.1), p(r, 0.22, 0.9), p(r, 0.78, 0.9), p(r, 0.78, 0.26), p(r, 0.62, 0.1)],
        s,
    );
    painter.line(vec![p(r, 0.62, 0.1), p(r, 0.62, 0.26), p(r, 0.78, 0.26)], s);
}

/// ⌃ a chevron pointing up.
pub fn chevron_up(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(vec![p(r, 0.25, 0.62), p(r, 0.5, 0.38), p(r, 0.75, 0.62)], s);
}

/// Notes: three lines and a pencil.
pub fn notes(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    for (y, x1) in [(0.25, 0.8), (0.45, 0.8), (0.65, 0.45)] {
        painter.line_segment([p(r, 0.15, y), p(r, x1, y)], s);
    }
    // The pencil, from the lower right up and to the left.
    painter.line_segment([p(r, 0.88, 0.55), p(r, 0.6, 0.88)], s);
    painter.line_segment([p(r, 0.6, 0.88), p(r, 0.55, 0.93)], s);
}

/// Export: a tray with an arrow pointing down into it.
pub fn export(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(vec![p(r, 0.15, 0.6), p(r, 0.15, 0.85), p(r, 0.85, 0.85), p(r, 0.85, 0.6)], s);
    painter.line_segment([p(r, 0.5, 0.12), p(r, 0.5, 0.65)], s);
    painter.line(vec![p(r, 0.3, 0.45), p(r, 0.5, 0.65), p(r, 0.7, 0.45)], s);
}

/// +
pub fn plus(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.5, 0.2), p(r, 0.5, 0.8)], s);
    painter.line_segment([p(r, 0.2, 0.5), p(r, 0.8, 0.5)], s);
}

/// ⋮ kebab.
pub fn kebab(painter: &Painter, r: Rect, color: Color32) {
    for y in [0.25, 0.5, 0.75] {
        painter.circle_filled(p(r, 0.5, y), STROKE * 0.9, color);
    }
}

/// → "go to" arrow.
pub fn goto_arrow(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.15, 0.5), p(r, 0.8, 0.5)], s);
    painter.line(vec![p(r, 0.55, 0.28), p(r, 0.8, 0.5), p(r, 0.55, 0.72)], s);
}

/// A loupe over a small pulse (protocol analyzers).
pub fn loupe(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.circle_stroke(p(r, 0.42, 0.42), r.width() * 0.3, s);
    painter.line_segment([p(r, 0.64, 0.64), p(r, 0.9, 0.9)], Stroke::new(STROKE * 1.8, color));
    let pulse =
        vec![p(r, 0.22, 0.5), p(r, 0.33, 0.5), p(r, 0.33, 0.34), p(r, 0.51, 0.34), p(r, 0.51, 0.5), p(r, 0.62, 0.5)];
    painter.line(pulse, Stroke::new(1.2, color));
}

/// A tape measure: a case with its tape pulled out (measurements).
pub fn tape_measure(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    let case = Rect::from_min_max(p(r, 0.04, 0.16), p(r, 0.66, 0.78));
    painter.rect_stroke(case, case.width() * 0.22, s, egui::StrokeKind::Middle);
    painter.circle_stroke(case.center(), r.width() * 0.1, s);
    // The tape leaves the case at the bottom, with ticks and an end hook.
    let (top, bottom) = (0.62, 0.78);
    painter.line_segment([p(r, 0.66, top), p(r, 0.96, top)], s);
    painter.line_segment([p(r, 0.96, 0.54), p(r, 0.96, bottom)], s);
    for x in [0.74, 0.82, 0.9] {
        painter.line_segment([p(r, x, top), p(r, x, 0.7)], Stroke::new(1.0, color));
    }
}

/// A table grid (data table).
pub fn grid(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(1.2, color);
    painter.rect_stroke(Rect::from_min_max(p(r, 0.15, 0.2), p(r, 0.85, 0.8)), 1.5, s, egui::StrokeKind::Middle);
    painter.line_segment([p(r, 0.15, 0.4), p(r, 0.85, 0.4)], s);
    painter.line_segment([p(r, 0.15, 0.6), p(r, 0.85, 0.6)], s);
    painter.line_segment([p(r, 0.4, 0.2), p(r, 0.4, 0.8)], s);
}

/// `>_` (terminal).
pub fn terminal(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(vec![p(r, 0.2, 0.3), p(r, 0.42, 0.5), p(r, 0.2, 0.7)], s);
    painter.line_segment([p(r, 0.5, 0.72), p(r, 0.8, 0.72)], s);
}

/// A panel glyph: a rectangle with a vertical divider.
pub fn panel(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.rect_stroke(Rect::from_min_max(p(r, 0.15, 0.2), p(r, 0.85, 0.8)), 1.5, s, egui::StrokeKind::Middle);
    painter.line_segment([p(r, 0.45, 0.2), p(r, 0.45, 0.8)], s);
}

/// Pop-out: an open box with an arrow leaving its top-right corner.
pub fn pop_out(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(vec![p(r, 0.45, 0.2), p(r, 0.2, 0.2), p(r, 0.2, 0.8), p(r, 0.8, 0.8), p(r, 0.8, 0.55)], s);
    painter.line_segment([p(r, 0.45, 0.55), p(r, 0.85, 0.15)], s);
    painter.line(vec![p(r, 0.6, 0.15), p(r, 0.85, 0.15), p(r, 0.85, 0.4)], s);
}

/// Dock: the panel glyph with an arrow into its left pane.
pub fn dock(painter: &Painter, r: Rect, color: Color32) {
    panel(painter, r, color);
    let s = Stroke::new(STROKE, color);
    painter.line_segment([p(r, 0.75, 0.5), p(r, 0.25, 0.5)], s);
    painter.line(vec![p(r, 0.36, 0.38), p(r, 0.24, 0.5), p(r, 0.36, 0.62)], s);
}

// ---- trigger conditions (a channel row's trigger icon) ----

/// No condition: a small dot.
pub fn trig_none(painter: &Painter, r: Rect, color: Color32) {
    painter.circle_filled(r.center(), 1.5, color);
}

/// A rising edge.
pub fn trig_rising(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![p(r, 0.08, 0.85), p(r, 0.5, 0.85), p(r, 0.5, 0.15), p(r, 0.92, 0.15)];
    painter.line(pts, Stroke::new(STROKE, color));
}

/// A falling edge.
pub fn trig_falling(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![p(r, 0.08, 0.15), p(r, 0.5, 0.15), p(r, 0.5, 0.85), p(r, 0.92, 0.85)];
    painter.line(pts, Stroke::new(STROKE, color));
}

/// Either edge: both crossing.
pub fn trig_either(painter: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(STROKE, color);
    painter.line(vec![p(r, 0.05, 0.85), p(r, 0.3, 0.85), p(r, 0.7, 0.15), p(r, 0.95, 0.15)], s);
    painter.line(vec![p(r, 0.05, 0.15), p(r, 0.3, 0.15), p(r, 0.7, 0.85), p(r, 0.95, 0.85)], s);
}

/// High: a line at the top, over a faint base.
pub fn trig_high(painter: &Painter, r: Rect, color: Color32) {
    painter.line_segment([p(r, 0.1, 0.2), p(r, 0.9, 0.2)], Stroke::new(STROKE + 0.5, color));
    painter.line_segment([p(r, 0.1, 0.85), p(r, 0.9, 0.85)], Stroke::new(1.0, color.gamma_multiply(0.35)));
}

/// Low: a line at the bottom, under a faint top.
pub fn trig_low(painter: &Painter, r: Rect, color: Color32) {
    painter.line_segment([p(r, 0.1, 0.85), p(r, 0.9, 0.85)], Stroke::new(STROKE + 0.5, color));
    painter.line_segment([p(r, 0.1, 0.2), p(r, 0.9, 0.2)], Stroke::new(1.0, color.gamma_multiply(0.35)));
}

/// A high pulse, at the falling edge that ends it.
pub fn trig_pulse_high(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![
        p(r, 0.02, 0.85),
        p(r, 0.25, 0.85),
        p(r, 0.25, 0.15),
        p(r, 0.72, 0.15),
        p(r, 0.72, 0.85),
        p(r, 0.98, 0.85),
    ];
    painter.line(pts, Stroke::new(STROKE, color));
}

/// A low pulse, at the rising edge that ends it.
pub fn trig_pulse_low(painter: &Painter, r: Rect, color: Color32) {
    let pts = vec![
        p(r, 0.02, 0.15),
        p(r, 0.25, 0.15),
        p(r, 0.25, 0.85),
        p(r, 0.72, 0.85),
        p(r, 0.72, 0.15),
        p(r, 0.98, 0.15),
    ];
    painter.line(pts, Stroke::new(STROKE, color));
}

/// A channel of a value range: a bus segment.
pub fn trig_range(painter: &Painter, r: Rect, color: Color32) {
    let pts =
        vec![p(r, 0.05, 0.5), p(r, 0.22, 0.2), p(r, 0.78, 0.2), p(r, 0.95, 0.5), p(r, 0.78, 0.8), p(r, 0.22, 0.8)];
    painter.add(PathShape::closed_line(pts, Stroke::new(STROKE, color)));
}
