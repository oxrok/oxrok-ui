//! Waveform painting (07): time ruler, grid and digital rows.
//!
//! Geometry is computed in physical pixels and converted to points so that
//! 1 px lines stay crisp.

use crate::core::CaptureTime;
use crate::core::ticks::{TimeTicks, time_ticks};
use crate::core::view::ViewState;
use crate::store::{ColumnClass, merge_runs};
use crate::ui::icons;
use crate::ui::theme::{Theme, Token};
use crate::ui::typography::Text;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

pub const RULER_HEIGHT: f32 = 30.0;
/// Vertical gutter of digital rows (at most).
pub const DIGITAL_GUTTER: f32 = 8.0;
/// H/L tags (and arrows) need this much inner height.
pub const ROOMY_INNER_HEIGHT: f32 = 30.0;

/// Snaps `x` (points) to the center of a physical pixel.
pub fn snap(x: f32, ppp: f32) -> f32 {
    ((x * ppp).floor() + 0.5) / ppp
}

/// Ticks for the plot whose x = 0 is `view.left_time`.
pub fn ticks_for(view: &ViewState, origin: CaptureTime) -> TimeTicks {
    time_ticks(view.left_time, origin, view.time_per_px, view.width_px)
}

fn vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    for (pos, color) in
        [(rect.left_top(), top), (rect.right_top(), top), (rect.right_bottom(), bottom), (rect.left_bottom(), bottom)]
    {
        mesh.vertices.push(Vertex { pos, uv: WHITE_UV, color });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// The time ruler over `rect` (x = 0 at `rect.left`).
pub fn paint_time_ruler(painter: &Painter, rect: Rect, ticks: &TimeTicks, theme: &Theme) {
    let painter = painter.with_clip_rect(rect);
    let ppp = painter.pixels_per_point();
    vertical_gradient(
        &painter,
        Rect::from_min_size(rect.min, vec2(rect.width(), 20.0)),
        Color32::from_black_alpha(20),
        Color32::TRANSPARENT,
    );
    let text = theme.color(Token::Ink1);
    let tick_color = theme.color(Token::Seam);
    let font = Text::CaptionMed.font_id();
    for t in &ticks.ticks {
        let x = snap(rect.left() + t.x as f32, ppp);
        let len = if t.major { 8.0 } else { 4.0 };
        painter
            .line_segment([pos2(x, rect.bottom() - len), pos2(x, rect.bottom())], Stroke::new(1.0 / ppp, tick_color));
        let y = if t.major { rect.top() + 3.0 } else { rect.top() + 16.0 };
        painter.text(pos2(x + 2.0, y), Align2::LEFT_TOP, &t.label, font.clone(), text);
    }
    if let Some(off) = &ticks.offscreen_major {
        // Synthetic major at x = 0 with a "◂".
        let y = rect.top() + 3.0;
        let bg = Rect::from_min_size(pos2(rect.left(), y - 1.0), vec2(10.0, 14.0));
        icons::left_arrow(&painter, bg.center() + vec2(1.0, 0.0), 7.0, text);
        painter.text(pos2(rect.left() + 11.0, y), Align2::LEFT_TOP, &off.label, font, text);
    }
}

/// Dashed vertical grid lines at the tick positions.
pub fn paint_grid(painter: &Painter, plot: Rect, ticks: &TimeTicks, theme: &Theme) {
    let ppp = painter.pixels_per_point();
    let (major, minor) = (theme.color(Token::Hair), theme.color(Token::Seam));
    for t in &ticks.ticks {
        let x = snap(plot.left() + t.x as f32, ppp);
        if x < plot.left() || x > plot.right() {
            continue;
        }
        let stroke = Stroke::new(1.0 / ppp, if t.major { major } else { minor });
        painter.extend(Shape::dashed_line(&[pos2(x, plot.top()), pos2(x, plot.bottom())], stroke, 2.0, 2.0));
    }
}

/// Trace line width (17 `trace_width`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceWidth {
    /// One physical pixel (1.5 px on displays with DPR < 1.5).
    Thin,
    /// 1 pt: 2 physical pixels on a 2× display.
    #[default]
    Normal,
    /// 2 pt.
    Bold,
}

impl TraceWidth {
    pub const ALL: [TraceWidth; 3] = [TraceWidth::Thin, TraceWidth::Normal, TraceWidth::Bold];

    pub fn label(self) -> &'static str {
        match self {
            TraceWidth::Thin => "Thin",
            TraceWidth::Normal => "Normal",
            TraceWidth::Bold => "Bold",
        }
    }

    /// The width in points, a whole number of physical pixels.
    /// On a 1× display Thin and Normal are both one pixel.
    pub fn points(self, ppp: f32) -> f32 {
        let px = match self {
            TraceWidth::Thin => 1.0,
            TraceWidth::Normal => ppp,
            TraceWidth::Bold => 2.0 * ppp,
        };
        px.round().max(1.0) / ppp
    }

    /// Analog traces: never thinner than 's minimum.
    fn analog_points(self, ppp: f32) -> f32 {
        self.points(ppp).max(trace_px(ppp))
    }
}

/// Geometry of a digital row's drawable area.
#[derive(Copy, Clone, Debug)]
pub struct DigitalLines {
    pub high_y: f32,
    pub low_y: f32,
}

impl DigitalLines {
    /// The gutter is `min(8 pt, 15 % of the inner height)`, so compact rows
    /// still show a clear high and low.
    pub fn new(inner: Rect) -> Self {
        let g = DIGITAL_GUTTER.min((inner.height() * 0.15).floor());
        Self { high_y: inner.top() + g, low_y: inner.bottom() - g - 1.0 }
    }
}

/// Draws a digital trace from per-physical-column classes.
/// Column `c` spans physical pixels `[c, c+1)` from `inner.left()`.
pub fn paint_digital_trace(
    painter: &Painter,
    inner: Rect,
    classes: &[ColumnClass],
    color: Color32,
    width: TraceWidth,
    theme: &Theme,
) {
    let ppp = painter.pixels_per_point();
    let px = 1.0 / ppp;
    // Lines `w` thick; edges widen to the right by the same amount.
    let w = width.points(ppp);
    let lines = DigitalLines::new(inner);
    // Snap level lines to physical rows.
    let high = (lines.high_y * ppp).floor() / ppp;
    let low = (lines.low_y * ppp).floor() / ppp;
    let col_x = |c: usize| (inner.left() * ppp).round() / ppp + c as f32 * px;
    let mut shapes = Vec::with_capacity(64);
    let no_data_bar = theme.color(Token::Ink4).gamma_multiply(0.2);
    let no_data_tint = theme.color(Token::Ink4).gamma_multiply(0.04);
    for run in merge_runs(classes) {
        let (x0, x1) = (col_x(run.start), col_x(run.end));
        let fill = |y0: f32, y1: f32, c: Color32| Shape::rect_filled(Rect::from_x_y_ranges(x0..=x1, y0..=y1), 0.0, c);
        let line =
            |y0: f32, y1: f32| Shape::rect_filled(Rect::from_x_y_ranges(x0..=x1 + (w - px), y0..=y1), 0.0, color);
        match run.class {
            ColumnClass::Low => shapes.push(line(low, low + w)),
            ColumnClass::High => shapes.push(line(high, high + w)),
            ColumnClass::Transition => shapes.push(line(high, low + w)),
            ColumnClass::NoData | ColumnClass::PartialNoData => {
                shapes.push(fill(inner.top(), inner.bottom(), no_data_tint));
                shapes.push(fill(inner.top(), inner.top() + 2.0, no_data_bar));
                if run.class == ColumnClass::PartialNoData {
                    let mid = inner.center().y;
                    shapes.push(fill(mid - 1.0, mid + 1.0, no_data_bar));
                }
            }
        }
    }
    painter.extend(shapes);

    // H / L labels at the left edge, when there is room.
    if inner.height() < ROOMY_INNER_HEIGHT {
        return;
    }
    let font = egui::FontId::new(9.0, crate::ui::typography::family(crate::ui::typography::SANS));
    let c = theme.color(Token::Ink3);
    painter.text(pos2(inner.left() + 2.0, high - 1.0), Align2::LEFT_BOTTOM, "H", font.clone(), c);
    painter.text(pos2(inner.left() + 2.0, low + 2.0), Align2::LEFT_TOP, "L", font, c);
}

/// How a row's channel looks.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RowLook {
    /// The channel's color as a swatch (wire color or fallback palette).
    pub swatch: Color32,
    /// Color for traces and text: `swatch`, or a neutral one when it lacks contrast.
    pub ink: Color32,
    pub marker: Option<Color32>,
    /// Swatches need an outline (low contrast).
    pub outline: bool,
}

/// The label cell. The 5 pt stripe is the swatch color; a marker
/// caps its top 8 pt. `name` is omitted when it equals the identifier.
/// An empty `identifier` draws the name alone, in the row's ink.
#[allow(clippy::too_many_arguments)]
pub fn paint_row_label(
    painter: &Painter,
    rect: Rect,
    indent: f32,
    // Room kept free at the right (live state).
    reserve: f32,
    identifier: &str,
    name: Option<&str>,
    look: &RowLook,
    theme: &Theme,
) {
    painter.rect_filled(rect, 0.0, theme.color(Token::Deck));
    let stripe = Rect::from_min_size(rect.min, vec2(5.0, rect.height() - 2.0));
    painter.rect_filled(stripe, 0.0, look.swatch);
    if look.outline {
        painter.rect_stroke(stripe, 0.0, Stroke::new(1.0, theme.color(Token::Seam)), egui::StrokeKind::Inside);
    }
    if let Some(m) = look.marker {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(5.0, 8.0)), 0.0, m);
    }
    let border = theme.color(Token::Seam);
    painter.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, border));
    painter.rect_filled(Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 2.0..=rect.bottom()), 0.0, border);

    let baseline = rect.top() + 20.0f32.min(rect.height() - 5.0);
    let id_galley = painter.layout_no_wrap(identifier.to_owned(), Text::ValueS.font_id(), look.ink);
    let id_pos = pos2(rect.left() + 12.0 + indent, baseline - id_galley.size().y);
    let name_only = identifier.is_empty();
    let name_x = if name_only { id_pos.x } else { id_pos.x + id_galley.size().x + 8.0 };
    painter.galley(id_pos, id_galley, look.ink);
    let max_w = (rect.right() - 6.0 - reserve - name_x).max(0.0);
    if let Some(name) = name
        && max_w > 8.0
    {
        let mut job = egui::text::LayoutJob::simple_singleline(
            name.to_owned(),
            Text::Lead.font_id(),
            if name_only { look.ink } else { theme.color(Token::Ink2) },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let galley = painter.layout_job(job);
        painter.galley(Pos2::new(name_x, baseline - galley.size().y + 1.0), galley, Color32::PLACEHOLDER);
    }
}

/// Thumb geometry: `(offset, width)` within a track of `track_w`
/// points, or `None` without data.
pub fn scrollbar_thumb(
    view: &ViewState,
    begin: CaptureTime,
    end: CaptureTime,
    track_w: f32,
    following: bool,
) -> Option<(f32, f32)> {
    let duration = end.secs_since(begin);
    if duration <= 0.0 || track_w <= 0.0 {
        return None;
    }
    let scale = view.time_scale();
    let w = (track_w * (scale / duration) as f32).clamp(25.0_f32.min(track_w), track_w);
    let travel = duration - scale;
    let frac = if following {
        1.0
    } else if travel > 0.0 {
        (view.left_time.secs_since(begin) / travel).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(((track_w - w) * frac as f32, w))
}

// ---- analog rows ----

use crate::core::analog::AnalogView;
use crate::core::ticks::ValueTicks;

/// Horizontal lines at the value ticks.
pub fn paint_value_grid(painter: &Painter, inner: Rect, view: &AnalogView, ticks: &ValueTicks, theme: &Theme) {
    let ppp = painter.pixels_per_point();
    let h = inner.height() as f64;
    for t in &ticks.ticks {
        let y = snap(inner.top() + view.value_to_y(t.value, h) as f32, ppp);
        if y < inner.top() || y > inner.bottom() {
            continue;
        }
        let pts = [pos2(inner.left(), y), pos2(inner.right(), y)];
        if t.value == 0.0 {
            painter.line_segment(pts, Stroke::new(1.0 / ppp, theme.color(Token::Hair)));
        } else {
            let c = if t.major { theme.color(Token::Hair) } else { theme.color(Token::Seam) };
            painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0 / ppp, c), 2.0, 2.0));
        }
    }
}

/// Scale labels flush left inside the plot, 8–10 pt scaled with
/// the row height (60…600 pt), each on an 80 % `background-0` chip.
pub fn paint_scale_labels(
    painter: &Painter,
    inner: Rect,
    view: &AnalogView,
    ticks: &ValueTicks,
    row_height: f32,
    theme: &Theme,
) {
    let size = 8.0 + 2.0 * ((row_height - 60.0) / 540.0).clamp(0.0, 1.0);
    let font = egui::FontId::new(size, crate::ui::typography::family(crate::ui::typography::SANS_MEDIUM));
    let h = inner.height() as f64;
    let chip = theme.color(Token::Canvas).gamma_multiply(0.8);
    for t in &ticks.ticks {
        let y = inner.top() + view.value_to_y(t.value, h) as f32;
        if y < inner.top() + size / 2.0 || y > inner.bottom() - size / 2.0 {
            continue;
        }
        let galley = painter.layout_no_wrap(t.label.clone(), font.clone(), theme.color(Token::Ink3));
        let r = Rect::from_min_size(pos2(inner.left() + 4.0, y - galley.size().y / 2.0), galley.size())
            .expand2(vec2(3.0, 1.0));
        painter.rect_filled(r, 2.0, chip);
        painter.galley(pos2(r.left() + 3.0, r.top() + 1.0), galley, Color32::PLACEHOLDER);
    }
}

/// Line width: 1 physical px, 1.5 px below 1.5× scaling.
fn trace_px(ppp: f32) -> f32 {
    if ppp < 1.5 { 1.5 / ppp } else { 1.0 / ppp }
}

/// Dense case: a min–max span per physical column, extended to
/// touch the previous column's span so there are no gaps.
pub fn paint_analog_spans(
    painter: &Painter,
    inner: Rect,
    cols: &[Option<(f32, f32)>],
    view: &AnalogView,
    color: Color32,
    width: TraceWidth,
) {
    let ppp = painter.pixels_per_point();
    let px = 1.0 / ppp;
    let w = width.analog_points(ppp);
    let h = inner.height() as f64;
    let y = |v: f32| inner.top() + view.value_to_y(v as f64, h) as f32;
    let x0 = (inner.left() * ppp).round() / ppp;
    let mut mesh = Mesh::default();
    let mut prev: Option<(f32, f32)> = None;
    for (c, col) in cols.iter().enumerate() {
        let Some((lo, hi)) = *col else {
            prev = None;
            continue;
        };
        let (mut top, mut bottom) = (y(hi), y(lo));
        if let Some((pt, pb)) = prev {
            top = top.min(pb);
            bottom = bottom.max(pt);
        }
        prev = Some((y(hi), y(lo)));
        // Snap to whole physical pixels so thin spans don't fade.
        let top = (top * ppp).floor() / ppp;
        let bottom = ((bottom + w) * ppp).ceil().max(top * ppp + w * ppp) / ppp;
        let x = x0 + c as f32 * px;
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, top), pos2(x + w.max(px), bottom)), color);
    }
    painter.add(Shape::mesh(mesh));
}

/// Sparse case: a polyline through `(x, value)` points, with a
/// 3 px dot at each sample when they are more than 8 px apart.
pub fn paint_analog_polyline(
    painter: &Painter,
    inner: Rect,
    points: &[(f32, f32)],
    view: &AnalogView,
    color: Color32,
    width: TraceWidth,
) {
    let ppp = painter.pixels_per_point();
    let h = inner.height() as f64;
    let pts: Vec<Pos2> =
        points.iter().map(|&(x, v)| pos2(x, inner.top() + view.value_to_y(v as f64, h) as f32)).collect();
    if pts.len() >= 2 {
        painter.add(Shape::line(pts.clone(), Stroke::new(width.analog_points(ppp), color)));
    }
    let spaced = pts.windows(2).all(|w| (w[1].x - w[0].x).abs() > 8.0);
    if spaced {
        for p in pts {
            painter.circle_filled(p, 1.5, color);
        }
    }
}

/// A small triangle at the top (`up`) or bottom edge; returns its rect.
pub fn paint_out_of_range(painter: &Painter, inner: Rect, x: f32, up: bool, color: Color32) -> Rect {
    let (w, h) = (10.0, 6.0);
    let (tip, base) =
        if up { (inner.top() + 2.0, inner.top() + 2.0 + h) } else { (inner.bottom() - 2.0, inner.bottom() - 2.0 - h) };
    let pts = vec![pos2(x, tip), pos2(x + w / 2.0, base), pos2(x - w / 2.0, base)];
    painter.add(egui::epaint::PathShape::convex_polygon(pts, color, Stroke::NONE));
    Rect::from_center_size(pos2(x, (tip + base) / 2.0), vec2(w + 6.0, h + 6.0))
}

// ---- hover measurement drawing ----

/// A horizontal double arrow from `x0` to `x1` at `y`.
pub fn paint_double_arrow(painter: &Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let s = Stroke::new(1.0, color);
    painter.line_segment([pos2(x0, y), pos2(x1, y)], s);
    for (x, dir) in [(x0, 1.0), (x1, -1.0)] {
        painter.line_segment([pos2(x, y), pos2(x + 5.0 * dir, y - 3.0)], s);
        painter.line_segment([pos2(x, y), pos2(x + 5.0 * dir, y + 3.0)], s);
    }
}

/// An 11 pt label on a `background-0` chip; `alpha` is the chip opacity.
pub fn paint_label_chip(painter: &Painter, anchor: Pos2, align: Align2, text: &str, alpha: f32, theme: &Theme) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), Text::Caption.font_id(), theme.color(Token::Ink1));
    let r = align.anchor_size(anchor, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, 2.0, theme.color(Token::Canvas).gamma_multiply(alpha));
    painter.galley(r.min + vec2(4.0, 2.0), galley, Color32::PLACEHOLDER);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_to_pixel_centers() {
        assert_eq!(snap(10.2, 1.0), 10.5);
        assert_eq!(snap(10.2, 2.0), 10.25);
    }

    #[test]
    fn thumb_geometry() {
        let b = CaptureTime::ZERO;
        let e = CaptureTime::from_secs_f64(10.0);
        // 1 s visible of 10 s on a 500 pt track → 50 pt thumb.
        let v = ViewState::new(CaptureTime::from_secs_f64(9.0), 1e-3, 1000.0);
        assert_eq!(scrollbar_thumb(&v, b, e, 500.0, false), Some((450.0, 50.0)));
        let v = ViewState::new(CaptureTime::ZERO, 1e-5, 1000.0);
        assert_eq!(scrollbar_thumb(&v, b, e, 500.0, false).unwrap().1, 25.0);
        assert_eq!(scrollbar_thumb(&v, b, b, 500.0, false), None);
        assert_eq!(scrollbar_thumb(&v, b, e, 500.0, true).unwrap().0, 475.0);
    }
}

#[cfg(test)]
mod trace_width_tests {
    use super::TraceWidth;

    /// Whole physical pixels; Thin is always one.
    #[test]
    fn widths_in_physical_pixels() {
        let px = |w: TraceWidth, ppp: f32| w.points(ppp) * ppp;
        assert_eq!(
            [px(TraceWidth::Thin, 1.0), px(TraceWidth::Normal, 1.0), px(TraceWidth::Bold, 1.0)],
            [1.0, 1.0, 2.0]
        );
        assert_eq!(
            [px(TraceWidth::Thin, 2.0), px(TraceWidth::Normal, 2.0), px(TraceWidth::Bold, 2.0)],
            [1.0, 2.0, 4.0]
        );
        assert_eq!(px(TraceWidth::Normal, 1.5), 2.0);
        assert_eq!(TraceWidth::Thin.analog_points(1.0), 1.5);
    }
}
