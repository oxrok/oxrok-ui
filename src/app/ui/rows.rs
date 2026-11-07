//! Row painting (09): digital and analog rows, labels, the Y Fit
//! button, out-of-range markers and the hover readouts.

use super::graph::Layout;
use super::{HoverRow, RowGeom};
use crate::app::LaApp;
use crate::core::analog::AnalogView;
use crate::core::format::{si, time_short};
use crate::core::ticks::value_ticks;
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState};
use crate::engine::measure::{measure_analog, measure_digital};
use crate::store::{AnalogChannel, DigitalChannel};
use crate::ui::Token;
use crate::ui::theme::{Theme, ThemeExt};
use crate::ui::typography::Text;
use crate::ui::waveform::{
    DigitalLines, paint_analog_polyline, paint_analog_spans, paint_digital_trace, paint_double_arrow, paint_grid,
    paint_label_chip, paint_out_of_range, paint_row_label, paint_scale_labels, paint_value_grid, ticks_for,
};
use egui::{Align2, Pos2, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};
use std::collections::HashMap;

/// Digital rows pad the inner area by 8 pt on top and 8 pt + a 2 pt
/// border below; analog rows only lose the border. Digital padding is
/// `min(8 pt, 12 % of the row)` so compact rows keep their trace.
const DIGITAL_PAD: f32 = 8.0;
const BORDER: f32 = 2.0;
/// Resize handle height.
const RESIZE_HANDLE: f32 = 9.0;
/// Vertical scrollbar width.
const VSCROLL_W: f32 = 8.0;
const YFIT_BUTTON: f32 = 32.0;
/// Label indents of a group row (room for ▾) and its members.
const BUS_INDENT: f32 = 14.0;
const MEMBER_INDENT: f32 = 12.0;
/// Arrows shorter than this are omitted.
const MIN_ARROW_PX: f32 = 3.0;

fn inner_rect(cell: Rect, kind: ChannelKind) -> Rect {
    match kind {
        ChannelKind::Digital | ChannelKind::Bus => {
            let pad = DIGITAL_PAD.min((cell.height() * 0.12).floor());
            Rect::from_min_max(pos2(cell.left(), cell.top() + pad), pos2(cell.right(), cell.bottom() - pad - BORDER))
        }
        ChannelKind::Analog => Rect::from_min_max(cell.min, pos2(cell.right(), cell.bottom() - BORDER)),
    }
}

/// Plot x (points) → fractional sample position.
fn x_to_sample(view: &ViewState, cell: Rect, x: f32, rate: u64) -> f64 {
    view.px_to_time((x - cell.left()) as f64).to_sample_f64(rate)
}

fn sample_to_x(view: &ViewState, cell: Rect, sample: u64, rate: u64) -> f32 {
    cell.left() + view.time_to_px(CaptureTime::from_sample(sample, rate)) as f32
}

impl LaApp {
    pub(crate) fn paint_rows(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let theme = ui.la_theme();
        // Reset each frame; `digital_hover` sets it while hovering.
        self.ui.pulse_under.set(None);
        // Bubble lanes add to a row's height, above the trace.
        let lanes = self.lane_map();
        let Some(s) = self.session.as_ref() else { return };
        let lanes_h = |c: ChannelId| lanes.get(&c).map_or(0.0, |v| v.len() as f32 * super::bubbles::LANE_H);
        let h = |r: &crate::app::session::Row| r.height + lanes_h(r.channel);
        let total: f32 = s.visible_rows().map(h).sum();
        let max_scroll = (total - l.rows.height()).max(0.0);
        self.ui.rows_scroll = self.ui.rows_scroll.clamp(0.0, max_scroll);
        let scroll = self.ui.rows_scroll;
        // PageUp / PageDown scroll 90 % of the row area.
        self.ui.rows_page = l.rows.height() * 0.9;

        // Layout pass: geometry for every visible row, and the analog views
        // (created on first layout).
        let mut geoms = Vec::new();
        let mut y = l.rows.top() - scroll;
        // While reordering, the other rows open a gap where the
        // dragged one would land (animated), and it floats at the pointer.
        let reorder = self.ui.reorder.filter(|r| r.active);
        // A group is dragged with its (visible) members.
        let block: Vec<(ChannelId, f32)> = match reorder {
            Some(ro) => s
                .visible_rows()
                .filter(|r| {
                    r.channel == ro.channel
                        || ro.channel.is_bus() && s.bus_of(r.channel).is_some_and(|b| b.row() == ro.channel)
                })
                .map(|r| (r.channel, h(r)))
                .collect(),
            None => Vec::new(),
        };
        let in_block = |c: ChannelId| block.iter().any(|r| r.0 == c);
        let block_h: f32 = block.iter().map(|r| r.1).sum();
        let dragged = block.first().copied();
        let mut drop_before = None;
        let mut gap_at = None;
        if let (Some(ro), Some(_)) = (reorder, dragged) {
            let mut yy = l.rows.top() - scroll;
            let mut idx = 0;
            for row in s.visible_rows().filter(|r| !in_block(r.channel)) {
                if yy + h(row) / 2.0 < ro.pointer_y - ro.grab + block_h / 2.0 {
                    idx += 1;
                } else if drop_before.is_none() {
                    drop_before = Some(row.channel);
                }
                yy += h(row);
            }
            gap_at = Some(idx);
        }
        let ctx = ui.ctx().clone();
        let mut i = 0;
        for row in s.visible_rows() {
            if in_block(row.channel) {
                continue;
            }
            if gap_at == Some(i) {
                y += block_h;
            }
            i += 1;
            let t = if reorder.is_some() { 0.2 } else { 0.0 };
            let top = ctx.animate_value_with_time(ui.make_persistent_id(("row_y", row.channel)), y, t);
            let r = Rect::from_min_max(pos2(l.rows.left(), top), pos2(l.rows.right(), top + h(row)));
            y += h(row);
            let cell = Rect::from_min_max(pos2(r.left() + l.label_w, r.top()), r.max);
            let trace = Rect::from_min_max(pos2(cell.left(), cell.top() + lanes_h(row.channel)), cell.max);
            geoms.push((RowGeom { channel: row.channel, cell, inner: inner_rect(trace, row.channel.kind) }, r));
        }
        if let Some(ro) = reorder {
            y += block_h;
            let mut top = ro.pointer_y - ro.grab;
            for &(c, hh) in &block {
                ctx.animate_value_with_time(ui.make_persistent_id(("row_y", c)), top, 0.0);
                let r = Rect::from_min_max(pos2(l.rows.left(), top), pos2(l.rows.right(), top + hh));
                let cell = Rect::from_min_max(pos2(r.left() + l.label_w, r.top()), r.max);
                let trace = Rect::from_min_max(pos2(cell.left(), cell.top() + lanes_h(c)), cell.max);
                geoms.push((RowGeom { channel: c, cell, inner: inner_rect(trace, c.kind) }, r));
                top += hh;
            }
        }
        if let Some(ro) = self.ui.reorder.as_mut() {
            ro.drop_before = drop_before;
        }
        let filler_top = y;
        let s = self.session.as_mut().unwrap();
        let mut views: HashMap<ChannelId, AnalogView> = HashMap::new();
        self.ui.row_inner_height.clear();
        for (g, _) in &geoms {
            if g.is_analog() {
                let h = g.inner.height();
                self.ui.row_inner_height.insert(g.channel, h);
                if let Some(v) = s.analog_view(g.channel, h as f64) {
                    // Keep the view valid if the row height changed.
                    let v = v.clamped(h as f64);
                    s.set_analog_view(g.channel, v);
                    views.insert(g.channel, v);
                }
            }
        }
        self.ui.row_geometry = geoms.iter().map(|(g, _)| *g).collect();

        let s = self.session.as_ref().unwrap();
        let painter = ui.painter_at(l.rows);
        let ppp = ui.ctx().pixels_per_point();
        let ticks = ticks_for(view, s.t0());
        let data = s.capture.as_ref().map(|c| c.read());
        let rate = data.as_ref().and_then(|d| d.sample_rate());
        let pointer = self.plot_pointer(ui).filter(|p| l.plot_rows.contains(*p));
        let dragging = self.ui.drag.is_some() || self.ui.marquee.is_some() || reorder.is_some();
        let measure = self.settings.waveform.show_interactive_measurement;
        let neutral = !self.settings.waveform.color_waveforms;
        let collapsed = super::rowctl::collapsed(l.label_w);
        let mut name_rects = Vec::new();
        let mut label_resps = Vec::new();
        let mut start_rename = None;
        let mut start_reorder = None;
        let mut fit = None;
        let mut resize: Option<(ChannelId, f32, bool)> = None;
        let mut hover = None;
        let mut hover_digital = None;
        let mut bubble_acts = Vec::new();
        let mut handles = Vec::new();
        let mut toggle_bus = None;
        let mut triangles = Vec::new();
        let mut indicators: Vec<(ChannelId, Rect, String)> = Vec::new();
        let any_pointer = self.plot_pointer(ui).filter(|p| l.rows.contains(*p));

        for (g, r) in &geoms {
            if r.bottom() < l.rows.top() || r.top() > l.rows.bottom() {
                continue;
            }
            let row = s.row(g.channel).unwrap();
            let label = Rect::from_min_max(r.min, pos2(g.cell.left(), r.bottom()));
            let look = super::probe::row_look(&theme, s, row);
            // Toggle Traces Color: one neutral trace color.
            let color = if neutral { theme.color(Token::Ink2) } else { look.ink };
            if reorder.is_some() && in_block(row.channel) {
                // The floating row's drop shadow.
                painter.add(theme.box_shadow().as_shape(*r, 0));
            }
            let clip = painter.with_clip_rect(g.cell.intersect(l.rows));
            paint_grid(&clip, g.cell, &ticks, &theme);
            let hovered = pointer.filter(|p| g.cell.contains(*p) && !dragging);
            if g.channel.kind == ChannelKind::Digital && any_pointer.is_some_and(|p| r.contains(p)) {
                hover_digital = Some((g.channel, g.cell.intersect(l.rows), hovered.is_some()));
            }
            match g.channel.kind {
                ChannelKind::Digital => {
                    let ch = data.as_ref().and_then(|d| d.channel(g.channel));
                    if let (Some(ch), Some(rate)) = (ch, rate) {
                        let ncols = (g.cell.width() * ppp).ceil() as usize;
                        let first = view.left_time.to_sample_f64(rate);
                        let spc = view.time_per_px * rate as f64 / ppp as f64;
                        let cls = ch.classify_columns(first, spc, ncols);
                        paint_digital_trace(&clip, g.inner, &cls, color, self.settings.waveform.trace_width, &theme);
                        // The analyzers' bubble lanes above the trace.
                        if let Some(ls) = lanes.get(&g.channel) {
                            let area =
                                Rect::from_min_max(g.cell.min, pos2(g.cell.right(), g.cell.top() + lanes_h(g.channel)));
                            self.paint_lanes(ui, &painter, area, ls, view, rate, &theme, &mut bubble_acts);
                        }
                        if let Some(p) = hovered.filter(|_| measure) {
                            // Clipped to the row stack, not the cell: the width
                            // label sits above the row's top gutter.
                            self.digital_hover(ui, &painter, g, ch, view, rate, p, &theme);
                        }
                    }
                }
                ChannelKind::Bus => {
                    // Value bubbles.
                    if let Some(rate) = rate {
                        self.paint_bus(ui, &clip, g, view, rate, data.as_deref(), hovered, &theme);
                    }
                }
                ChannelKind::Analog => {
                    let Some(av) = views.get(&g.channel) else { continue };
                    let h = g.inner.height() as f64;
                    let (bottom, top) = av.visible_range(h);
                    let vticks = value_ticks(bottom, top, h.max(1.0), "V");
                    paint_value_grid(&clip, g.inner, av, &vticks, &theme);
                    let ch = data.as_ref().and_then(|d| d.analog(g.channel));
                    if let (Some(ch), Some(rate)) = (ch, rate) {
                        paint_analog(&clip, g.inner, ch, view, av, rate, color, self.settings.waveform.trace_width);
                        // out-of-range markers.
                        let a = view.left_time.to_sample_floor(rate).max(0) as u64;
                        let b = view.right_time().to_sample_floor(rate).max(0) as u64 + 1;
                        if let Some((lo, hi)) = ch.min_max(a, b) {
                            let x = g.inner.right() - YFIT_BUTTON - 24.0;
                            for (out, up) in [(hi as f64 > top, true), ((lo as f64) < bottom, false)] {
                                if out {
                                    let rr = paint_out_of_range(&clip, g.inner, x, up, color);
                                    let id = ui.make_persistent_id(("out_of_range", g.channel, up));
                                    if ui.interact(rr, id, Sense::click()).on_hover_text("Y Fit").clicked() {
                                        fit = Some(g.channel);
                                    }
                                }
                            }
                        }
                        // Y Fit button.
                        let btn = Rect::from_min_size(
                            pos2(g.inner.right() - YFIT_BUTTON - 4.0, g.inner.top() + 4.0),
                            vec2(YFIT_BUTTON, YFIT_BUTTON),
                        );
                        let resp = ui.interact(
                            btn.intersect(l.rows),
                            ui.make_persistent_id(("y_fit", g.channel)),
                            Sense::click(),
                        );
                        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Y Fit"));
                        let btn_hover = resp.hovered();
                        paint_yfit_button(&clip, btn, btn_hover, &theme);
                        if resp.on_hover_text("Y Fit").clicked() {
                            fit = Some(g.channel);
                        }
                        if let Some(p) = hovered.filter(|_| !btn_hover && measure) {
                            analog_hover(&clip, g, ch, av, view, rate, p, color, &theme);
                        }
                    }
                    paint_scale_labels(&clip, g.inner, av, &vticks, row.height, &theme);
                    if let Some(p) = pointer.filter(|p| g.cell.contains(*p)) {
                        hover = Some(HoverRow {
                            channel: g.channel,
                            y: p.y - g.inner.top(),
                            inner_height: g.inner.height(),
                        });
                    }
                }
            }
            painter.line_segment(
                [g.cell.left_bottom(), g.cell.right_bottom()],
                Stroke::new(1.0 / ppp, theme.color(Token::Hair)),
            );
            // Collapsed labels show the identifier only; the
            // name is hidden while its editor is open.
            let editing = self.ui.rename.as_ref().is_some_and(|e| e.channel == row.channel);
            let name = (row.name != row.ident && !collapsed && !editing).then_some(row.name.as_str());
            // A group with a name of its own shows just the name.
            let name_only = row.channel.is_bus() && !collapsed && row.name != s.row_default_name(row);
            let shown_ident = if name_only { String::new() } else { row.identifier() };
            // Lane subtitles above the label of the trace part.
            let lh = lanes_h(row.channel);
            if let Some(ls) = lanes.get(&row.channel) {
                let top = Rect::from_min_max(label.min, pos2(label.right(), label.top() + lh));
                Self::paint_lane_labels(&painter, top, ls, look.swatch, &theme);
            }
            let trace_label = Rect::from_min_max(pos2(label.left(), label.top() + lh), label.max);
            // A group's disclosure triangle; its members indented
            // along a guide line in the group's color.
            let group = s.bus_of(row.channel);
            let indent = if row.channel.is_bus() {
                BUS_INDENT
            } else if group.is_some() {
                MEMBER_INDENT
            } else {
                0.0
            };
            // The live level (a dot, or H / L when collapsed)
            // or a group's live value, at the label's right.
            let live: Option<(String, Option<bool>, bool)> = s.pins.as_ref().and_then(|p| {
                if row.channel.is_bus() {
                    s.live_bus_value(row.channel.index).map(|v| (v, None, p.stale))
                } else {
                    p.level(row.channel).map(|l| (if l { "high" } else { "low" }.to_owned(), Some(l), p.stale))
                }
            });
            let live_font = Text::ValueS.font_id();
            let live_w = match &live {
                Some((v, None, _)) => painter.layout_no_wrap(v.clone(), live_font.clone(), color).size().x + 8.0,
                Some(_) => 14.0,
                None => 0.0,
            };
            paint_row_label(&painter, trace_label, indent, live_w, &shown_ident, name, &look, &theme);
            if let Some((text, level, stale)) = &live {
                let ink = if *stale { look.ink.gamma_multiply(0.4) } else { look.ink };
                let cy = trace_label.top() + 20.0f32.min(trace_label.height() - 5.0) - 6.0;
                let x = label.right() - 8.0;
                let spot = match level {
                    Some(high) if collapsed => {
                        let t = if *high { "H" } else { "L" };
                        painter.text(pos2(x, cy), Align2::RIGHT_CENTER, t, live_font.clone(), ink)
                    }
                    Some(high) => {
                        let c = pos2(x - 4.0, cy);
                        if *high {
                            painter.circle_filled(c, 3.5, ink);
                        } else {
                            painter.circle_stroke(c, 3.0, Stroke::new(1.2, ink));
                        }
                        Rect::from_center_size(c, vec2(10.0, 10.0))
                    }
                    None => painter.text(pos2(x, cy), Align2::RIGHT_CENTER, text, live_font.clone(), ink),
                };
                let what = if *stale {
                    format!("{}: {text} (stale)", row.identifier())
                } else {
                    format!("{}: {text}", row.identifier())
                };
                indicators.push((row.channel, spot.expand(2.0), what));
            }
            if let Some(b) = group
                && let Some(br) = s.row(b.row())
            {
                let gc = super::probe::row_look(&theme, s, br).swatch;
                let x = label.left() + 14.0;
                painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, gc));
            }
            if let Some(b) = row.channel.is_bus().then(|| s.bus(row.channel.index)).flatten() {
                let c = pos2(label.left() + 17.0, trace_label.top() + 20.0f32.min(trace_label.height() - 5.0) - 6.0);
                // Its click area is made after the label's (below), so
                // the label's drag doesn't take it.
                triangles.push((b.id, c, b.collapsed));
            }
            // Accessible row label, e.g. "D3" or "D3 SPI CLK"; the tooltip
            // describes the probe lead. It is also the drag handle
            // for reordering and opens the context menu.
            let label_vis = label.intersect(l.rows);
            let mut resp =
                ui.interact(label_vis, ui.make_persistent_id(("row_label", row.channel)), Sense::click_and_drag());
            if let Some(d) = super::probe::describe(s, row) {
                resp = resp.on_hover_text(format!("{} · {d}", row.identifier()));
            }
            let text = if collapsed { row.identifier() } else { row.display() };
            resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
            // The name area (right of the identifier) edits on click.
            let id_w = if name_only {
                -8.0
            } else {
                painter.layout_no_wrap(row.identifier(), Text::ValueS.font_id(), color).size().x
            };
            let name_rect = Rect::from_min_max(
                pos2(label.left() + 12.0 + indent + id_w + 4.0, label.top() + 1.0),
                pos2(label.right() - 4.0, label.bottom() - 3.0),
            );
            let on_name = !collapsed && name_rect.width() > 12.0;
            if on_name {
                name_rects.push((row.channel, name_rect));
                let n = ui.interact(
                    name_rect.intersect(l.rows),
                    ui.make_persistent_id(("row_name", row.channel)),
                    Sense::click(),
                );
                n.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Rename {}", row.identifier())));
                if n.hovered() && !editing {
                    painter.rect_stroke(
                        name_rect,
                        2.0,
                        Stroke::new(1.0, theme.color(Token::Seam)),
                        egui::StrokeKind::Inside,
                    );
                }
                if n.clicked() {
                    start_rename = Some(row.channel);
                }
                label_resps.push((row.channel, n));
            }
            // A drag that didn't start on the name reorders.
            if resp.drag_started() {
                let origin = ui.input(|i| i.pointer.press_origin());
                if !(on_name && origin.is_some_and(|o| name_rect.contains(o))) {
                    let o = origin.unwrap_or(label.center());
                    start_reorder = Some(super::Reorder {
                        channel: row.channel,
                        grab: o.y - r.top(),
                        pointer_y: o.y,
                        active: true,
                        drop_before: None,
                    });
                }
            }
            label_resps.push((row.channel, resp));
            // A 9 pt handle on the label's bottom border resizes the
            // row. Created after every label (below), so the next row's label
            // drag doesn't take the lower half of the handle.
            handles.push((
                row.channel,
                row.identifier(),
                row.height,
                row.snapped(row.height),
                label.left(),
                label.right(),
                r.bottom(),
            ));
        }
        // The live state's tooltip.
        for (id, r, what) in indicators {
            let resp = ui.interact(r.intersect(l.rows), ui.make_persistent_id(("live_pin", id)), Sense::hover());
            resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, &what));
            resp.on_hover_text(&what);
        }
        // Disclosure triangles.
        for (id, c, collapsed) in triangles {
            let tri = Rect::from_center_size(c, vec2(16.0, 16.0));
            let resp = ui.interact(tri.intersect(l.rows), ui.make_persistent_id(("bus_toggle", id)), Sense::click());
            let label = if collapsed { "Expand group" } else { "Collapse group" };
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
            let ink = if resp.hovered() { theme.color(Token::Ink1) } else { theme.color(Token::Ink3) };
            let pts = if collapsed {
                vec![c + vec2(-2.5, -4.0), c + vec2(-2.5, 4.0), c + vec2(3.5, 0.0)]
            } else {
                vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.5)]
            };
            painter.add(egui::Shape::convex_polygon(pts, ink, Stroke::NONE));
            if resp.clicked() {
                toggle_bus = Some(id);
            }
        }
        for (channel, ident, height, snapped, left, right, bottom) in handles {
            let handle =
                Rect::from_min_max(pos2(left, bottom - RESIZE_HANDLE / 2.0), pos2(right, bottom + RESIZE_HANDLE / 2.0));
            let h =
                ui.interact(handle.intersect(l.rows), ui.make_persistent_id(("row_resize", channel)), Sense::drag());
            h.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, format!("Resize {ident}")));
            if h.hovered() || h.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if h.dragged() {
                resize = Some((channel, height + h.drag_delta().y, false));
            }
            if h.drag_stopped() {
                resize = Some((channel, snapped, true));
            }
        }
        drop(data);
        if let Some(rate) = rate {
            self.apply_bubble_acts(ui, bubble_acts, rate);
        }
        if let Some(ro) = start_reorder {
            self.ui.reorder = Some(ro);
        }
        if let Some(ro) = self.ui.reorder.as_mut()
            && let Some(p) = ui.input(|i| i.pointer.latest_pos())
        {
            ro.pointer_y = p.y;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        self.finish_reorder(ui, l);
        if let Some(id) = start_rename {
            self.start_rename(id);
        }
        self.rename_editor(ui, &name_rects);
        // Label right-clicks open the row menu, anchored on the plot
        // so it survives the row being hidden from it.
        for (id, resp) in &label_resps {
            if resp.secondary_clicked() {
                self.ui.context_row = Some(*id);
                self.ui.open_row_menu_at = resp.interact_pointer_pos();
            }
        }
        self.ui.hover_row = hover;
        self.ui.hover_digital = hover_digital.map(|h| h.0);
        // Edge buttons on the hovered digital row's plot.
        if let Some((id, cell, true)) = hover_digital
            && self.session.as_ref().is_some_and(|s| s.has_data())
        {
            self.edge_buttons(ui, cell, id);
        }

        // The label column background extends below the last row.
        if filler_top < l.rows.bottom() {
            let filler = Rect::from_min_max(
                pos2(l.rows.left(), filler_top.max(l.rows.top())),
                pos2(l.rows.left() + l.label_w, l.rows.bottom()),
            );
            painter.rect_filled(filler, 0.0, theme.color(Token::Deck));
            painter
                .line_segment([filler.right_top(), filler.right_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
        }

        // Vertical scrollbar when the rows overflow, and a one-time
        // hint about scrolling them.
        if max_scroll > 0.0 && !self.settings.hints_shown.iter().any(|h| h == "rows_scroll") {
            self.settings.hints_shown.push("rows_scroll".into());
            self.mark_settings_dirty();
            let toast = crate::ui::toast::Toast::new(
                crate::ui::toast::ToastKind::Info,
                "Use the mouse wheel over the channel names, or Page Up / Page Down, to scroll through the rows.",
            )
            .heading("More rows than fit");
            self.toast(toast);
        }
        if max_scroll > 0.0 {
            let track = Rect::from_min_max(pos2(l.rows.right() - VSCROLL_W, l.rows.top()), l.rows.max);
            let h = (track.height() * track.height() / total).max(25.0);
            let top = track.top() + (track.height() - h) * (scroll / max_scroll);
            let thumb = Rect::from_min_size(pos2(track.left(), top), vec2(VSCROLL_W, h));
            let resp = ui.interact(thumb, ui.make_persistent_id("vscroll"), Sense::drag());
            if resp.dragged() {
                self.ui.rows_scroll += resp.drag_delta().y * total / track.height();
            }
            let c = if resp.hovered() || resp.dragged() { Token::Ink4 } else { Token::Seam };
            painter.rect_filled(thumb, 4.0, theme.color(c).gamma_multiply(0.6));
        }

        if let Some((id, height, _)) = resize
            && let Some(s) = self.session.as_mut()
        {
            s.set_row_height(id, height);
            // A manual resize turns Auto Layout off.
            if self.settings.waveform.auto_layout {
                self.settings.waveform.auto_layout = false;
                self.mark_settings_dirty();
            }
        }
        if let Some(id) = fit {
            self.y_fit(id);
        }
        if let Some(b) = toggle_bus.and_then(|b| self.session.as_mut()?.bus_mut(b)) {
            b.collapsed = !b.collapsed;
        }
    }

    /// Width and period arrows plus the info box.
    #[allow(clippy::too_many_arguments)]
    fn digital_hover(
        &self,
        ui: &mut Ui,
        p: &egui::Painter,
        g: &RowGeom,
        ch: &DigitalChannel,
        view: &ViewState,
        rate: u64,
        at: Pos2,
        theme: &Theme,
    ) {
        let sample = x_to_sample(view, g.cell, at.x, rate);
        if sample < 0.0 {
            return;
        }
        let Some(pulse) = measure_digital(ch, sample.floor() as u64) else { return };
        // Remember the pulse so `Shift+R` can wrap it.
        self.ui.pulse_under.set(pulse.e1.map(|e1| (g.channel, pulse.e0, e1)));
        let fg = theme.color(Token::Ink1);
        let lines = DigitalLines::new(g.inner);
        let x0 = sample_to_x(view, g.cell, pulse.e0, rate);
        let dashed = Stroke::new(1.0, fg);
        p.extend(egui::Shape::dashed_line(&[pos2(x0, g.inner.top()), pos2(x0, g.inner.bottom())], dashed, 3.0, 3.0));
        let secs = |n: u64| n as f64 / rate as f64;
        // Arrows need a roomy row; compact rows get the info box only.
        let roomy = g.inner.height() >= crate::ui::waveform::ROOMY_INNER_HEIGHT;
        if let (true, Some(e1), Some(w)) = (roomy, pulse.e1, pulse.width()) {
            let x1 = sample_to_x(view, g.cell, e1, rate);
            if x1 - x0 >= MIN_ARROW_PX {
                let y = lines.high_y - 6.0;
                paint_double_arrow(p, x0, x1, y, fg);
                // Centered on the arrow so it fits in the top row too
                // (puts it above; there is no room above row 1).
                paint_label_chip(p, pos2((x0 + x1) / 2.0, y), Align2::CENTER_CENTER, &time_short(secs(w)), 1.0, theme);
            }
        }
        if let (true, Some(e2), Some(period)) = (roomy, pulse.e2, pulse.period()) {
            let x2 = sample_to_x(view, g.cell, e2, rate);
            if x2 - x0 >= MIN_ARROW_PX {
                let y = lines.low_y + 6.0;
                paint_double_arrow(p, x0, x2, y, fg);
                paint_label_chip(
                    p,
                    pos2((x0 + x2) / 2.0, y),
                    Align2::CENTER_CENTER,
                    &time_short(secs(period)),
                    1.0,
                    theme,
                );
            }
        }
        let (Some(w), Some(period)) = (pulse.width(), pulse.period()) else { return };
        let lines_text = [
            format!("Duty: {:.2} %", pulse.duty_percent().unwrap_or(0.0)),
            format!("Freq: {}", si(1.0 / secs(period), "Hz", Some(3))),
            format!("width⁻¹: {}", si(1.0 / secs(w), "Hz", Some(3))),
        ];
        // Info box: 30 pt from the right edge, or the left one when the
        // pointer is in the right part of the plot.
        let size = vec2(150.0, 62.0);
        let right_side = at.x < g.cell.center().x + g.cell.width() * 0.25;
        let x = if right_side { g.cell.right() - 30.0 - size.x } else { g.cell.left() + 30.0 };
        // Centered on the row, but kept inside the row stack (compact rows
        // are smaller than the box).
        let area = p.clip_rect();
        let top = (g.cell.top() + (g.cell.height() - size.y) / 2.0)
            .clamp(area.top(), (area.bottom() - size.y).max(area.top()));
        let rect = Rect::from_min_size(pos2(x, top), size);
        // Its own foreground layer: compact rows are smaller than the box,
        // and the rows below must not draw over it.
        egui::Area::new(ui.make_persistent_id("pulse_info"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme.color(Token::Card))
                    .stroke(Stroke::new(1.0, theme.color(Token::Seam)))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(size.x - 16.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for t in &lines_text {
                            ui.label(
                                egui::RichText::new(t).text_style(Text::Value.style()).color(theme.color(Token::Ink2)),
                            );
                        }
                    });
            });
    }
}

/// Spans when samples are denser than pixels, a polyline otherwise.
#[allow(clippy::too_many_arguments)]
fn paint_analog(
    p: &egui::Painter,
    inner: Rect,
    ch: &AnalogChannel,
    view: &ViewState,
    av: &AnalogView,
    rate: u64,
    color: egui::Color32,
    width: crate::ui::waveform::TraceWidth,
) {
    let ppp = p.pixels_per_point();
    let ncols = (inner.width() * ppp).ceil() as usize;
    let first = view.left_time.to_sample_f64(rate);
    let spc = view.time_per_px * rate as f64 / ppp as f64;
    if spc >= 1.0 {
        paint_analog_spans(p, inner, &ch.column_min_max(first, spc, ncols), av, color, width);
    } else {
        let a = first.floor().max(0.0) as u64;
        let b = (first + ncols as f64 * spc).ceil() as u64 + 1;
        let x0 = (inner.left() * ppp).round() / ppp;
        let pts: Vec<(f32, f32)> =
            ch.samples(a, b + 1).map(|(s, v)| (x0 + ((s as f64 - first) / spc) as f32 / ppp, v)).collect();
        paint_analog_polyline(p, inner, &pts, av, color, width);
    }
}

/// Dashed cursor line, a dot on the trace and the value label.
#[allow(clippy::too_many_arguments)]
fn analog_hover(
    p: &egui::Painter,
    g: &RowGeom,
    ch: &AnalogChannel,
    av: &AnalogView,
    view: &ViewState,
    rate: u64,
    at: Pos2,
    color: egui::Color32,
    theme: &Theme,
) {
    let line = Stroke::new(1.0, theme.color(Token::Ink3));
    p.extend(egui::Shape::dashed_line(&[pos2(at.x, g.inner.top()), pos2(at.x, g.inner.bottom())], line, 3.0, 3.0));
    let Some(v) = measure_analog(ch, x_to_sample(view, g.cell, at.x, rate)) else { return };
    let y = g.inner.top() + av.value_to_y(v as f64, g.inner.height() as f64) as f32;
    if y < g.inner.top() || y > g.inner.bottom() {
        return;
    }
    p.circle_filled(pos2(at.x, y), 2.5, color);
    paint_label_chip(p, pos2(at.x - 8.0, y), Align2::RIGHT_CENTER, &si(v as f64, "V", Some(3)), 0.85, theme);
}

/// Glyph: two vertical arrows toward a bar.
fn paint_yfit_button(p: &egui::Painter, r: Rect, hovered: bool, theme: &Theme) {
    if hovered {
        p.rect_filled(r, 3.0, theme.color(Token::Lift));
    }
    let c = if hovered { theme.color(Token::Ink1) } else { theme.color(Token::Ink3) };
    let s = Stroke::new(1.5, c);
    let ctr = r.center();
    p.line_segment([ctr + vec2(-7.0, 0.0), ctr + vec2(7.0, 0.0)], s);
    for dir in [-1.0, 1.0] {
        let tip = ctr + vec2(0.0, 3.0 * dir);
        let tail = ctr + vec2(0.0, 10.0 * dir);
        p.line_segment([tail, tip], s);
        p.line_segment([tip, tip + vec2(-3.0, 3.0 * dir)], s);
        p.line_segment([tip, tip + vec2(3.0, 3.0 * dir)], s);
    }
}
