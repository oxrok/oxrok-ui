//! Range measurements on the graph: placement, overlay, edge
//! drags, the hover grid and the popup.

use super::RowGeom;
use super::graph::Layout;
use super::markers::c32;
use crate::app::LaApp;
use crate::core::format::{count, si_sig};
use crate::core::markers::{SNAP_PT, snap_x};
use crate::core::ranges::{Metric, RANGE_COLORS};
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState};
use crate::engine::measure::{analog_metrics, digital_metrics, metric_value};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_check_item, popover_frame};
use egui::{
    Color32, FontId, Popup, PopupCloseBehavior, Pos2, Rect, Response, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2,
};
use std::time::Duration;

/// The Measurements panel id.
pub(crate) const RANGES_PANEL: &str = "measurements";
/// Edge grab distance.
const EDGE_GRAB: f32 = 5.0;
/// A press-drag shorter than this is a click.
const MIN_DRAG: f32 = 2.0;
/// Popup width.
const POPUP_W: f32 = 420.0;
/// "Copied!" stays this long.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// A range being placed.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct RangePlace {
    /// The row and time where it began.
    pub begin: Option<(RowGeom, CaptureTime)>,
    /// The current end, following the pointer.
    pub end: Option<CaptureTime>,
    /// Begun by a press-drag (release ends it) rather than a click.
    pub dragging: bool,
    /// One-off Shift+drag placement.
    pub by_shift: bool,
    /// Pointer x where it began.
    pub begin_x: f32,
}

/// Metric values of one range, cached by its channel and bounds.
pub(crate) type MetricCache =
    std::collections::HashMap<u32, ((ChannelId, CaptureTime, CaptureTime), Vec<(Metric, Option<f64>)>)>;

/// Value text: 7 significant digits with SI units; counts as
/// integers; `N/A` when it can't be computed.
pub(crate) fn metric_text(m: Metric, v: Option<f64>) -> String {
    match v {
        None => tr("N/A").into(),
        Some(v) => match m {
            Metric::NRising | Metric::NFalling => count(v as u64),
            Metric::Duty => format!("{} %", si_sig(v, "", 7).trim_end()),
            _ => si_sig(v, m.unit(), 7),
        },
    }
}

/// Copy: `name<TAB>value<TAB>unit` lines.
pub(crate) fn metrics_tsv(values: &[(Metric, Option<f64>)]) -> String {
    values
        .iter()
        .map(|(m, v)| format!("{}\t{}\t{}", m.plain_label(), v.map_or(tr("N/A").into(), |v| format!("{v}")), m.unit()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A rich-text label, e.g. italic *f* with subscript `mean`.
pub(crate) fn metric_label(ui: &Ui, m: Metric) -> egui::text::LayoutJob {
    let theme = ui.la_theme();
    let (base, sub, italic) = m.label();
    let color = theme.color(Token::Ink3);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        base,
        0.0,
        egui::TextFormat { font_id: Text::Prose.font_id(), color, italics: italic, ..Default::default() },
    );
    if !sub.is_empty() {
        job.append(
            sub,
            0.0,
            egui::TextFormat {
                font_id: FontId::new(9.0, Text::Prose.font_id().family),
                color,
                valign: egui::Align::BOTTOM,
                ..Default::default()
            },
        );
    }
    job
}

impl LaApp {
    // ---- placement ----

    /// Toggles placement (toolbar button, Ctrl/Cmd+G, panel `+`).
    pub(crate) fn begin_range_placement(&mut self) {
        if self.ui.range_place.take().is_some() {
            return;
        }
        let Some(s) = self.session.as_ref() else { return };
        if s.has_data() && !s.state.is_busy() {
            self.ui.range_place = Some(RangePlace::default());
        }
    }

    /// The stored interval of a channel.
    fn data_span(&self, id: ChannelId) -> Option<(CaptureTime, CaptureTime)> {
        let s = self.session.as_ref()?;
        let cap = s.capture.as_ref()?.read();
        let rate = cap.sample_rate()?;
        let (b, e) = match id.kind {
            ChannelKind::Digital => cap.channel(id)?.data_range()?,
            ChannelKind::Analog => cap.analog(id)?.data_range()?,
            ChannelKind::Bus => return None,
        };
        Some((CaptureTime::from_sample(b, rate), CaptureTime::from_sample(e, rate)))
    }

    /// The snapped time at `x` on `row` (the end stays on the start row).
    fn row_time(&self, row: &RowGeom, x: f32, l: &Layout, view: &ViewState) -> CaptureTime {
        self.snap_time(pos2(x, row.cell.center().y), l, view)
    }

    /// The digital edge `p` snaps to, if any (for the highlight).
    fn snap_edge(&self, p: Pos2, l: &Layout, view: &ViewState) -> Option<CaptureTime> {
        let px = (p.x - l.plot_rows.left()) as f64;
        let s = self.session.as_ref()?;
        let g = self.ui.row_geometry.iter().find(|g| g.cell.contains(p) && g.channel.kind == ChannelKind::Digital)?;
        let cap = s.capture.as_ref()?.read();
        let rate = cap.sample_rate()?;
        let ch = cap.channel(g.channel)?;
        let sample = view.px_to_time(px).to_sample_floor(rate).max(0) as u64;
        let prev = ch.prev_transition(sample + 1).map(|e| CaptureTime::from_sample(e, rate));
        let next = ch.next_transition(sample).map(|e| CaptureTime::from_sample(e, rate));
        let x = snap_x(px, prev.map(|t| view.time_to_px(t)), next.map(|t| view.time_to_px(t)), SNAP_PT);
        [prev, next].into_iter().flatten().find(|t| view.time_to_px(*t) == x)
    }

    /// Placement input on the plot; returns true when it used the input
    /// (no panning then).
    pub(crate) fn range_input(&mut self, ui: &Ui, plot: &Response, l: &Layout, view: &ViewState) -> bool {
        let mods = ui.input(|i| i.modifiers);
        let origin = ui.input(|i| i.pointer.press_origin());
        let row_at = |p: Pos2| self.ui.row_geometry.iter().copied().find(|g| g.cell.contains(p));
        // Shift+drag is a one-off placement.
        if self.ui.range_place.is_none() && plot.drag_started() && mods.shift && !mods.command {
            let s = self.session.as_ref();
            if s.is_some_and(|s| s.has_data() && !s.state.is_busy()) {
                self.ui.range_place = Some(RangePlace { by_shift: true, ..Default::default() });
            }
        }
        let Some(mut rp) = self.ui.range_place else { return false };
        let pointer = self.plot_pointer(ui);
        if let Some(p) = pointer.filter(|p| l.plot_rows.contains(*p)) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if let Some((row, _)) = rp.begin {
                let (lo, hi) = self.data_span(row.channel).unwrap_or((CaptureTime::ZERO, CaptureTime::ZERO));
                rp.end = Some(self.row_time(&row, p.x, l, view).clamp(lo, hi));
            }
        }
        // Begin: a press-drag start or a first click, inside recorded data.
        let start_at = if plot.drag_started() {
            origin
        } else if plot.clicked() && rp.begin.is_none() {
            plot.interact_pointer_pos()
        } else {
            None
        };
        if rp.begin.is_none()
            && let Some(p) = start_at
            && let Some(row) = row_at(p)
        {
            let t = self.row_time(&row, p.x, l, view);
            if self.data_span(row.channel).is_some_and(|(lo, hi)| t >= lo && t <= hi) {
                rp.begin = Some((row, t));
                rp.end = Some(t);
                rp.begin_x = p.x;
                rp.dragging = plot.drag_started();
            }
            self.ui.range_place = Some(rp);
            return true;
        }
        let finish = if rp.dragging { plot.drag_stopped() } else { plot.clicked() && rp.begin.is_some() };
        if finish && let (Some((row, t0)), Some(t1)) = (rp.begin, rp.end) {
            let x = ui.input(|i| i.pointer.latest_pos()).map_or(rp.begin_x, |p| p.x);
            let dragged = (x - rp.begin_x).abs() > MIN_DRAG;
            if rp.dragging && !dragged {
                // A short press: treat it as the first click.
                rp.dragging = false;
                self.ui.range_place = Some(rp);
                return true;
            }
            self.ui.range_place = None;
            if t1 != t0 {
                let id = self.session.as_mut().unwrap().ranges.add(row.channel, t0, t1);
                self.session.as_mut().unwrap().ranges.select_only(id);
                self.ui.range_popup = Some(id);
                self.record_range_add(id);
            }
            return true;
        }
        if rp.by_shift && rp.begin.is_none() && !mods.shift {
            self.ui.range_place = None;
            return false;
        }
        self.ui.range_place = Some(rp);
        true
    }

    // ---- metrics ----

    /// The visible metrics of a range, cached until it changes.
    pub(crate) fn range_metrics(&mut self, id: u32) -> Vec<(Metric, Option<f64>)> {
        let Some(s) = self.session.as_ref() else { return Vec::new() };
        let Some(r) = s.ranges.get(id) else { return Vec::new() };
        let key = (r.channel, r.t.0, r.t.1);
        let visible = match r.channel.kind {
            ChannelKind::Digital | ChannelKind::Bus => &self.settings.measurement_metrics.digital,
            ChannelKind::Analog => &self.settings.measurement_metrics.analog,
        };
        let all = match self.ui.metric_cache.get(&id) {
            Some((k, v)) if *k == key => v.clone(),
            _ => {
                let v = compute_metrics(s, r.channel, r.t.0, r.t.1);
                self.ui.metric_cache.insert(id, (key, v.clone()));
                v
            }
        };
        // ΔT always first, then the visible ones in catalogue order.
        all.into_iter().filter(|(m, _)| *m == Metric::Dt || visible.contains(m)).collect()
    }

    // ---- overlay and edges ----

    pub(crate) fn ranges_ui(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let Some(s) = self.session.as_ref() else { return };
        if !s.has_data() {
            return;
        }
        let theme = ui.la_theme();
        let x_of = |t: CaptureTime| l.plot_rows.left() + view.time_to_px(t) as f32;
        let geoms = self.ui.row_geometry.clone();
        let painter = ui.painter_at(l.plot_rows);
        let items: Vec<(u32, String, ChannelId, CaptureTime, CaptureTime, usize, bool)> = s
            .ranges
            .items
            .iter()
            .map(|r| (r.id, r.name.clone(), r.channel, r.t.0, r.t.1, r.color, r.selected))
            .collect();
        let mut clicked = None;
        let mut edge_drag = None;
        let mut hovered = None;
        for (id, name, ch, t0, t1, color, selected) in items {
            let Some(g) = geoms.iter().find(|g| g.channel == ch) else { continue };
            let (x0, x1) = (x_of(t0), x_of(t1));
            if x1 < l.plot_rows.left() || x0 > l.plot_rows.right() {
                continue;
            }
            let rect = Rect::from_x_y_ranges(x0..=x1, g.inner.y_range());
            let c = c32(RANGE_COLORS[color % RANGE_COLORS.len()]);
            painter.rect_filled(rect, 0.0, c.gamma_multiply(if selected { 0.25 } else { 0.15 }));
            let edge = Stroke::new(1.0, c.gamma_multiply(if selected { 0.5 } else { 0.4 }));
            for x in [x0, x1] {
                painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], edge);
            }
            if selected {
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(2.0, theme.color(Token::RingHot)),
                    egui::StrokeKind::Outside,
                );
            }
            // The name chip, when it fits.
            let galley = painter.layout_no_wrap(
                name.clone(),
                FontId::new(11.0, Text::Prose.font_id().family),
                theme.color(Token::Ink1),
            );
            let chip = Rect::from_center_size(
                pos2(rect.center().x, rect.top() + galley.size().y / 2.0 + 2.0),
                galley.size() + vec2(8.0, 2.0),
            );
            if chip.width() < rect.width() {
                painter.rect_filled(chip, 2.0, theme.color(Token::Card));
                painter.galley(chip.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
            }
            // Body: click opens the popup, hover shows the metrics.
            let body =
                ui.interact(rect.intersect(l.plot_rows), ui.make_persistent_id(("range_body", id)), Sense::click());
            body.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &name));
            if body.clicked() {
                clicked = Some(id);
            }
            if body.hovered() && self.ui.range_popup != Some(id) {
                hovered = Some((id, body.clone()));
            }
            // Edges.
            for (end, x) in [(false, x0), (true, x1)] {
                let hit = Rect::from_x_y_ranges(x - EDGE_GRAB..=x + EDGE_GRAB, g.inner.y_range());
                let e = ui.interact(hit, ui.make_persistent_id(("range_edge", id, end)), Sense::drag());
                e.widget_info(|| {
                    WidgetInfo::labeled(
                        Role::Splitter,
                        true,
                        format!("{name} {}", if end { tr("end") } else { tr("begin") }),
                    )
                });
                if e.hovered() || e.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                }
                if e.dragged()
                    && let Some(p) = e.interact_pointer_pos()
                {
                    edge_drag = Some((id, end, *g, p.x));
                }
            }
        }
        if let Some((id, end, g, x)) = edge_drag {
            let (lo, hi) = self.data_span(g.channel).unwrap_or((CaptureTime::ZERO, CaptureTime::ZERO));
            let t = self.row_time(&g, x, l, view).clamp(lo, hi);
            let current = self.ui.range_edge.filter(|e| e.0 == id).map_or(end, |e| e.1);
            {
                let s = self.session.as_mut().unwrap();
                let now_end = s.ranges.set_edge(id, current, t);
                self.ui.range_edge = Some((id, now_end));
            }
            // Highlight the edge the pointer snapped to.
            if let Some(p) = ui.input(|i| i.pointer.interact_pos())
                && let Some(edge) = self.snap_edge(p, l, view)
            {
                let x = x_of(edge);
                painter.line_segment(
                    [pos2(x, g.inner.top()), pos2(x, g.inner.bottom())],
                    Stroke::new(1.0, theme.color(Token::RingHot)),
                );
            }
        } else if !ui.input(|i| i.pointer.any_down()) {
            self.ui.range_edge = None;
        }
        if let Some(id) = clicked {
            self.session.as_mut().unwrap().ranges.select_only(id);
            self.ui.range_popup = Some(id);
        }
        if let Some((id, resp)) = hovered {
            let values = self.range_metrics(id);
            resp.on_hover_ui(|ui| metric_grid(ui, &values));
        }
        // The range being placed, the guide line, and the live readout.
        if let Some(rp) = self.ui.range_place {
            let pointer = self.plot_pointer(ui).filter(|p| l.plot_rows.contains(*p));
            match (rp.begin, rp.end) {
                (Some((row, t0)), Some(t1)) => {
                    let (a, b) = (x_of(t0.min(t1)), x_of(t0.max(t1)));
                    let rect = Rect::from_x_y_ranges(a..=b, row.inner.y_range());
                    let c = theme.color(Token::Mark);
                    painter.rect_filled(rect, 0.0, c.gamma_multiply(0.15));
                    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, c.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                    // The live ΔT, centered above the rectangle.
                    paint_time_chip(
                        &painter,
                        pos2((a + b) / 2.0, rect.top()),
                        &si_sig((t1 - t0).as_secs_f64(), "s", 6),
                        theme.as_ref(),
                    );
                    // Highlight the edge the pointer snapped to.
                    if let Some(p) = pointer
                        && let Some(edge) = self.snap_edge(p, l, view)
                    {
                        let x = x_of(edge);
                        painter.line_segment(
                            [pos2(x, row.inner.top()), pos2(x, row.inner.bottom())],
                            Stroke::new(1.0, theme.color(Token::RingHot)),
                        );
                        paint_time_chip(
                            &painter,
                            pos2(x, row.inner.top()),
                            &si_sig(edge.as_secs_f64(), "s", 6),
                            theme.as_ref(),
                        );
                    }
                }
                _ => {
                    if let Some(p) = pointer
                        && let Some(g) = geoms.iter().find(|g| g.cell.contains(p))
                    {
                        let x = x_of(self.row_time(g, p.x, l, view));
                        painter.line_segment(
                            [pos2(x, g.inner.top()), pos2(x, g.inner.bottom())],
                            Stroke::new(1.0, theme.color(Token::Mark)),
                        );
                        // Highlight the snapped edge.
                        if let Some(edge) = self.snap_edge(p, l, view) {
                            let x = x_of(edge);
                            painter.line_segment(
                                [pos2(x, g.inner.top()), pos2(x, g.inner.bottom())],
                                Stroke::new(1.0, theme.color(Token::RingHot)),
                            );
                        }
                    }
                }
            }
        }
        self.range_popup(ui, l, view);
    }

    // ---- popup ----

    fn range_popup(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let Some(id) = self.ui.range_popup else { return };
        let Some(r) = self.session.as_ref().and_then(|s| s.ranges.get(id).cloned()) else {
            self.ui.range_popup = None;
            return;
        };
        let theme = ui.la_theme();
        let values = self.range_metrics(id);
        let row = self.ui.row_geometry.iter().find(|g| g.channel == r.channel).copied();
        let x = l.plot_rows.left() + view.time_to_px(r.t.0) as f32;
        let x1 = l.plot_rows.left() + view.time_to_px(r.t.1) as f32;
        let below = row.map_or(l.plot_rows.top(), |g| g.cell.bottom());
        let default = pos2(
            ((x + x1) / 2.0 - POPUP_W / 2.0)
                .clamp(l.plot_rows.left(), (l.plot_rows.right() - POPUP_W).max(l.plot_rows.left())),
            below + 4.0,
        );
        let pos = r.popup_offset.map_or(default, |(dx, dy)| default + vec2(dx, dy));
        let copied = self.ui.copied_at.is_some_and(|t| self.now().saturating_sub(t) < COPIED_FOR);
        let mut name = r.name.clone();
        let mut comment = r.comment.clone();
        let mut color = r.color;
        let (mut delete, mut copy, mut export) = (false, false, false);
        let mut toggle = None;
        let kind = r.channel.kind;
        let visible = match kind {
            ChannelKind::Digital | ChannelKind::Bus => self.settings.measurement_metrics.digital.clone(),
            ChannelKind::Analog => self.settings.measurement_metrics.analog.clone(),
        };
        let area = egui::Area::new(ui.make_persistent_id(("range_popup", id)))
            .order(egui::Order::Foreground)
            .current_pos(pos)
            .movable(true)
            .show(ui.ctx(), |ui| {
                popover_frame(ui).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                    ui.set_width(POPUP_W - 28.0);
                    ui.horizontal(|ui| {
                        // The name is editable; Enter or blur commits.
                        let name_resp = ui.add(
                            egui::TextEdit::singleline(&mut name)
                                .font(Text::Lead.font_id())
                                .desired_width(140.0)
                                .hint_text(format!("M{}", r.id)),
                        );
                        super::label_text_edit(&name_resp, &name, tr("Range name"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, tr("Export this range's data"), icons::export)
                                .on_hover_text(tr("Export this range's data"))
                                .clicked()
                            {
                                export = true;
                            }
                            if icon_button(ui, tr("Copy"), icons::copy).on_hover_text(tr("Copy")).clicked() {
                                copy = true;
                            }
                            if copied {
                                ui.label(egui::RichText::new(tr("Copied!")).color(theme.color(Token::Good)));
                            }
                            let add = icon_button(ui, tr("Add or Remove Measurements"), icons::plus)
                                .on_hover_text(tr("Add or Remove Measurements"));
                            Popup::menu(&add)
                                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                                .frame(popover_frame(ui))
                                .width(240.0)
                                .show(|ui| {
                                    let all: &[Metric] =
                                        if kind == ChannelKind::Digital { &Metric::DIGITAL } else { &Metric::ANALOG };
                                    for m in all.iter().filter(|m| **m != Metric::Dt) {
                                        if menu_check_item(ui, &m.plain_label(), visible.contains(m), None)
                                            .on_hover_text(m.tooltip())
                                            .clicked()
                                        {
                                            toggle = Some(*m);
                                        }
                                    }
                                });
                        });
                    });
                    ui.add_space(8.0);
                    metric_grid(ui, &values);
                    ui.add_space(10.0);
                    let c = ui.add(
                        egui::TextEdit::multiline(&mut comment)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .hint_text(tr("Comments ...")),
                    );
                    super::label_text_edit(&c, &comment, tr("Range comment"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (k, rgb) in RANGE_COLORS.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                            resp.widget_info(|| {
                                WidgetInfo::selected(
                                    Role::Button,
                                    true,
                                    k == color,
                                    tr("Range color {}").replacen("{}", &(k + 1).to_string(), 1),
                                )
                            });
                            ui.painter().rect_filled(rect, 3.0, c32(*rgb));
                            if k == color {
                                icons::check(ui.painter(), rect.shrink(3.0), Color32::WHITE);
                            }
                            if resp.clicked() {
                                color = k;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, tr("Delete measurement"), icons::trash)
                                .on_hover_text(tr("Delete"))
                                .clicked()
                            {
                                delete = true;
                            }
                        });
                    });
                });
            });
        // Remember a dragged popup's offset.
        let moved = area.response.rect.min - default;
        let outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p))
            && !Popup::is_any_open(ui.ctx());
        let close = outside || ui.input(|i| i.key_pressed(egui::Key::Escape));
        if copy {
            ui.ctx().copy_text(metrics_tsv(&values));
            self.ui.copied_at = Some(self.now());
        }
        if export {
            self.ui.range_popup = None;
            self.open_export_for_range(id);
            return;
        }
        if let Some(m) = toggle {
            let list = match kind {
                ChannelKind::Digital | ChannelKind::Bus => &mut self.settings.measurement_metrics.digital,
                ChannelKind::Analog => &mut self.settings.measurement_metrics.analog,
            };
            if let Some(i) = list.iter().position(|x| *x == m) {
                list.remove(i);
            } else {
                list.push(m);
            }
            self.mark_settings_dirty();
        }
        if delete {
            let removed = self.session.as_ref().and_then(|s| s.ranges.get(id)).cloned();
            self.session.as_mut().unwrap().ranges.remove(id);
            if let Some(r) = removed {
                self.record_range_delete(vec![r]);
            }
            self.ui.range_popup = None;
            return;
        }
        let s = self.session.as_mut().unwrap();
        if let Some(r) = s.ranges.get_mut(id) {
            // An empty name reverts to `M<id>`.
            let name = name.trim();
            r.name = if name.is_empty() { format!("M{}", r.id) } else { name.to_owned() };
            r.comment = comment;
            r.color = color;
            if moved.length() > 0.5 {
                r.popup_offset = Some((moved.x, moved.y));
            }
        }
        if close {
            self.ui.range_popup = None;
        }
    }
}

/// The metric values of `[t0, t1)` on `ch`, every metric of its type.
fn compute_metrics(
    s: &crate::app::session::Session,
    ch: ChannelId,
    t0: CaptureTime,
    t1: CaptureTime,
) -> Vec<(Metric, Option<f64>)> {
    let dt = (t1 - t0).as_secs_f64();
    let Some(cap) = s.capture.as_ref() else { return Vec::new() };
    let cap = cap.read();
    let Some(rate) = cap.sample_rate() else { return Vec::new() };
    let s0 = t0.to_sample_floor(rate).max(0) as u64;
    let s1 = t1.to_sample_floor(rate).max(0) as u64;
    match ch.kind {
        ChannelKind::Bus => Vec::new(),
        ChannelKind::Digital => {
            let d = cap.channel(ch).and_then(|c| digital_metrics(c, s0, s1, rate));
            Metric::DIGITAL
                .iter()
                .map(|m| {
                    (*m, if d.is_none() && *m != Metric::Dt { None } else { metric_value(*m, dt, d.as_ref(), None) })
                })
                .collect()
        }
        ChannelKind::Analog => {
            let a = cap.analog(ch).and_then(|c| analog_metrics(c, s0, s1, rate));
            Metric::ANALOG.iter().map(|m| (*m, metric_value(*m, dt, None, a.as_ref()))).collect()
        }
    }
}

/// Two columns of `label value` pairs.
pub(crate) fn metric_grid(ui: &mut Ui, values: &[(Metric, Option<f64>)]) {
    let theme = ui.la_theme();
    egui::Grid::new(ui.next_auto_id()).num_columns(4).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for (i, (m, v)) in values.iter().enumerate() {
            let job = metric_label(ui, *m);
            ui.label(job).on_hover_text(m.tooltip());
            let text = metric_text(*m, *v);
            let color = if v.is_some() { Token::Ink1 } else { Token::Ink4 };
            let r = ui.label(egui::RichText::new(&text).text_style(Text::Value.style()).color(theme.color(color)));
            r.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("{} = {text}", m.plain_label())));
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

/// A time label on a `background-20` chip, centered on `center`.
fn paint_time_chip(painter: &egui::Painter, center: Pos2, text: &str, theme: &crate::ui::Theme) {
    let galley = painter.layout_no_wrap(
        text.to_string(),
        FontId::new(11.0, Text::Prose.font_id().family),
        theme.color(Token::Ink1),
    );
    let chip = Rect::from_center_size(center, galley.size() + vec2(8.0, 2.0));
    painter.rect_filled(chip, 2.0, theme.color(Token::Card));
    painter.galley(chip.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
}

/// A small icon button with an accessible label.
pub(crate) fn icon_button(ui: &mut Ui, label: &str, icon: fn(&egui::Painter, Rect, Color32)) -> Response {
    let theme = ui.la_theme();
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    let c = if resp.hovered() { Token::Ink1 } else { Token::Ink3 };
    icon(ui.painter(), rect.shrink(2.0), theme.color(c));
    resp
}
