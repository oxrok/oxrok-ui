//! Navigation tools: range zoom, the zoom tool,
//! view history, jump to time and edge navigation.

use super::graph::{CAPTURE_ITEM_W, Layout, TOOLBAR_H};
use super::{Marquee, cmd_key, label_text_edit};
use crate::app::LaApp;
use crate::app::settings::{MarqueeMode, RelativeZoom};
use crate::core::analog::AnalogView;
use crate::core::animator::{Anchor, FLING_TAU, JUMP_DURATION, ViewRequest};
use crate::core::format::{TimeUnit, parse_time, time_short};
use crate::core::history::ViewEntry;
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Segment, menu_check_item, menu_separator, menu_title, popover_frame, segmented};
use egui::{
    Color32, CornerRadius, Key, Popup, PopupCloseBehavior, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2,
    vec2,
};
use std::time::Duration;

/// Edge button size.
const EDGE_BUTTON: f32 = 20.0;
/// Where an edge lands, as a fraction of the width from the near side.
const EDGE_LANDING: f64 = 0.25;
/// The zoom tool's chevron part.
const CHEVRON_W: f32 = 18.0;
/// Input width.
const JUMP_W: f32 = 300.0;

impl LaApp {
    // ---- fling ----

    /// Starts a decaying pan at `velocity_px_s` (positive: later in time).
    pub(crate) fn fling(&mut self, velocity_px_s: f64) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if !s.has_data() {
            return;
        }
        s.follow = false;
        s.view.request(ViewRequest::Fling { velocity_px_s, tau: FLING_TAU }, now);
    }

    // ---- range zoom ----

    pub(crate) fn apply_marquee(&mut self, m: Marquee, plot: Rect) {
        let clamp = |p: egui::Pos2| pos2(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom()));
        let (a, b) = (clamp(m.start), clamp(m.current));
        let d = (b - a).abs();
        if d.x < 1.0 && d.y < 1.0 {
            return;
        }
        let mode = m.mode();
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if matches!(mode, MarqueeMode::X | MarqueeMode::XY) && d.x >= 1.0 && s.has_data() {
            let v = s.view.state();
            let t0 = v.px_to_time((a.x.min(b.x) - plot.left()) as f64);
            let t1 = v.px_to_time((a.x.max(b.x) - plot.left()) as f64);
            s.follow = false;
            s.view.request(ViewRequest::MoveToRange { t0, t1, duration: Duration::ZERO }, now);
        }
        if matches!(mode, MarqueeMode::Y | MarqueeMode::XY)
            && let Some(g) = m.row.filter(|g| g.is_analog())
        {
            let h = g.inner.height() as f64;
            let y = |p: egui::Pos2| (p.y.clamp(g.inner.top(), g.inner.bottom()) - g.inner.top()) as f64;
            let (y0, y1) = (y(a).min(y(b)), y(a).max(y(b)));
            if y1 - y0 >= 1.0
                && let Some(v) = s.analog_view(g.channel, h)
            {
                let (top, bottom) = (v.y_to_value(y0, h), v.y_to_value(y1, h));
                let zoomed =
                    AnalogView { units_per_px: (top - bottom) / h, center_value: (top + bottom) / 2.0, ..v }.clamped(h);
                s.set_analog_view(g.channel, zoomed);
            }
        }
    }

    /// The translucent rectangle while dragging.
    pub(crate) fn paint_marquee(&self, ui: &Ui, l: &Layout) {
        let Some(m) = self.ui.marquee else { return };
        let theme = ui.la_theme();
        let plot = l.plot_rows;
        let (a, b) = (m.start, m.current);
        let x = (a.x.min(b.x).max(plot.left()), a.x.max(b.x).min(plot.right()));
        let row = m.row.map_or(plot, |g| g.inner.intersect(plot));
        let rect = match m.mode() {
            // X spans the full height, Y the full width.
            MarqueeMode::X => Rect::from_x_y_ranges(x.0..=x.1, plot.y_range()),
            MarqueeMode::Y => {
                Rect::from_x_y_ranges(plot.x_range(), a.y.min(b.y).max(row.top())..=a.y.max(b.y).min(row.bottom()))
            }
            MarqueeMode::XY => Rect::from_x_y_ranges(x.0..=x.1, a.y.min(b.y)..=a.y.max(b.y)).intersect(plot),
        };
        let c = theme.color(Token::Mark);
        ui.painter().rect(rect, 0.0, c.gamma_multiply(0.18), Stroke::new(1.0, c), StrokeKind::Inside);
    }

    // ----: zoom tool and view history ----

    pub(crate) fn secondary_tools(&mut self, ui: &mut Ui, l: &Layout) {
        let Some(s) = self.session.as_ref() else { return };
        let (can_back, can_forward) = (s.history.can_go_back(), s.history.can_go_forward());
        let theme = ui.la_theme();
        // Right of the capture group, which is centered over the plot.
        let left = super::graph::capture_group(l).right() + 12.0;
        let group = Rect::from_min_size(pos2(left, l.toolbar.top()), vec2(CAPTURE_ITEM_W * 3.0 + CHEVRON_W, TOOLBAR_H));
        if group.right() > l.toolbar.right() - 8.0 {
            return; // May be hidden when narrow.
        }
        let radius = CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 };
        let p = ui.painter();
        p.add(theme.box_shadow().as_shape(group, radius));
        p.rect_filled(group, radius, theme.color(Token::Card));

        let mut x = group.left();
        let mut item = |w: f32| {
            let r = Rect::from_min_size(pos2(x, group.top()), vec2(w, TOOLBAR_H));
            x += w;
            r
        };
        let mode = self.settings.marquee_mode;
        let armed = self.ui.zoom_tool_armed;
        let main = item(CAPTURE_ITEM_W);
        let resp = tool_button(ui, main, tr("Zoom Tool"), true, armed, |p, r, c| match mode {
            MarqueeMode::X => icons::zoom_x(p, r, c),
            MarqueeMode::Y => icons::zoom_y(p, r, c),
            MarqueeMode::XY => icons::zoom_xy(p, r, c),
        });
        if resp.on_hover_text(tr("{} (click, then drag on the graph)").replacen("{}", mode_name(mode), 1)).clicked() {
            self.ui.zoom_tool_armed = !armed;
        }
        let chevron = item(CHEVRON_W);
        let resp = tool_button(ui, chevron, tr("Zoom Tool Options"), true, false, icons::chevron_down);
        let mut pick = None;
        let mut relative = self.settings.waveform.zoom_relative_to;
        Popup::from_toggle_button_response(&resp)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .frame(popover_frame(ui))
            .width(300.0)
            .show(|ui| {
                let cmd = cmd_key();
                for (m, hint) in [
                    (MarqueeMode::X, format!("{cmd}+Shift+Drag Horizontal")),
                    (MarqueeMode::Y, format!("{cmd}+Shift+Drag Vertical")),
                    (MarqueeMode::XY, format!("{cmd}+Drag")),
                ] {
                    if menu_check_item(ui, mode_name(m), m == mode, Some(&hint)).clicked() {
                        pick = Some(m);
                    }
                }
                menu_separator(ui);
                menu_title(ui, tr("Zoom relative to…"));
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.set_width(276.0);
                    let segments = [
                        Segment { label: tr("Center"), enabled: true, tooltip: tr("Zoom about the view center") },
                        Segment { label: tr("Mouse"), enabled: true, tooltip: tr("Zoom about the pointer") },
                    ];
                    let mut idx = usize::from(relative == RelativeZoom::Mouse);
                    if segmented(ui, "relative_zoom", &segments, &mut idx) {
                        relative = if idx == 1 { RelativeZoom::Mouse } else { RelativeZoom::Center };
                    }
                });
                ui.add_space(4.0);
            });
        if let Some(m) = pick {
            self.settings.marquee_mode = m;
            self.ui.zoom_tool_armed = true;
            self.mark_settings_dirty();
            Popup::close_all(ui.ctx());
        }
        if relative != self.settings.waveform.zoom_relative_to {
            self.settings.waveform.zoom_relative_to = relative;
            self.mark_settings_dirty();
        }
        let back = item(CAPTURE_ITEM_W);
        let resp = tool_button(ui, back, tr("Previous view"), can_back, false, icons::chevron_left);
        if resp.on_hover_text(tr("Previous view ({})").replacen("{}", &history_keys(false), 1)).clicked() {
            self.history_step(false);
        }
        let forward = item(CAPTURE_ITEM_W);
        let resp = tool_button(ui, forward, tr("Next view"), can_forward, false, icons::chevron_right);
        if resp.on_hover_text(tr("Next view ({})").replacen("{}", &history_keys(true), 1)).clicked() {
            self.history_step(true);
        }
    }

    // ---- view history ----

    /// The entry for what is on screen now.
    fn view_entry(&self) -> Option<ViewEntry> {
        let s = self.session.as_ref()?;
        let analog = s
            .visible_rows()
            .filter(|r| r.channel.kind == ChannelKind::Analog)
            .filter_map(|r| r.analog.map(|a| (r.channel, a)))
            .collect();
        Some(ViewEntry { view: s.view.state(), analog })
    }

    /// Feeds the history once per frame, while not recording.
    pub(crate) fn observe_history(&mut self) {
        let now = self.now();
        let Some(entry) = self.view_entry() else { return };
        let Some(s) = self.session.as_mut() else { return };
        if s.state.is_busy() || !s.has_data() {
            return;
        }
        s.history.observe(entry, now);
    }

    /// Previous (`forward` = false) / Next.
    pub(crate) fn history_step(&mut self, forward: bool) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let entry = if forward { s.history.forward() } else { s.history.back() };
        let Some(e) = entry else { return };
        s.follow = false;
        let width = s.view.state().width_px;
        let v = ViewState { width_px: width, ..e.view };
        s.view.request(ViewRequest::MoveToRange { t0: v.left_time, t1: v.right_time(), duration: JUMP_DURATION }, now);
        // Rows matched by id; ones that are gone are skipped.
        for (id, a) in e.analog {
            s.set_analog_view(id, a);
        }
    }

    // ---- jump to time ----

    pub(crate) fn open_jump(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.has_data()) {
            self.ui.jump_open = true;
            self.ui.jump_error = false;
        }
    }

    pub(crate) fn jump_to_time(&mut self, ui: &mut Ui, l: &Layout) {
        if !self.ui.jump_open {
            return;
        }
        let Some(s) = self.session.as_ref() else { return };
        let Some(b) = s.bounds() else {
            self.ui.jump_open = false;
            return;
        };
        let theme = ui.la_theme();
        // Relative to T0, the capture start for now (10).
        let (begin, end) = (b.begin.secs_since(CaptureTime::ZERO), b.end.secs_since(CaptureTime::ZERO));
        let (lo, hi) = (time_short(begin), time_short(end));
        let mut text = s.jump_text.clone();
        let opening = ui.memory(|m| m.focused().is_none()) && self.ui.jump_open;
        let pos = pos2(l.plot_rows.center().x - JUMP_W / 2.0, l.ruler.bottom() + 8.0);
        let mut submit = false;
        let mut close = false;
        let area = egui::Area::new(ui.make_persistent_id("jump_to_time"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                popover_frame(ui).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(JUMP_W - 24.0);
                    ui.label(
                        egui::RichText::new(tr("Jump to Time"))
                            .text_style(Text::Lead.style())
                            .color(theme.color(Token::Ink1)),
                    );
                    ui.add_space(6.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(f32::INFINITY)
                            .hint_text(tr("e.g. 10ms"))
                            .font(Text::Prose.font_id()),
                    );
                    label_text_edit(&r, &text, tr("Jump to Time"));
                    if opening {
                        r.request_focus();
                    }
                    let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    if r.lost_focus() && enter {
                        submit = true;
                        r.request_focus();
                    }
                    if esc {
                        close = true;
                    }
                    ui.add_space(6.0);
                    let (msg, color) = if self.ui.jump_error {
                        (
                            tr("Value must be between {} and {}.").replacen("{}", &lo, 1).replacen("{}", &hi, 1),
                            Token::Alert,
                        )
                    } else {
                        (
                            tr("Type a value between: {} to {}").replacen("{}", &lo, 1).replacen("{}", &hi, 1),
                            Token::Ink3,
                        )
                    };
                    ui.label(egui::RichText::new(msg).text_style(Text::CaptionMed.style()).color(theme.color(color)));
                });
            });
        // A click outside closes.
        let clicked_outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
        if let Some(s) = self.session.as_mut() {
            s.jump_text = text.clone();
        }
        if close || clicked_outside {
            self.ui.jump_open = false;
            self.ui.jump_error = false;
            return;
        }
        if submit {
            // 1 ns resolution.
            let value = parse_time(&text, TimeUnit::S).ok().map(|t| (t * 1e9).round() / 1e9);
            match value.filter(|t| (begin..=end).contains(t)) {
                Some(t) => {
                    self.ui.jump_error = false;
                    let now = self.now();
                    let s = self.session.as_mut().unwrap();
                    s.follow = false;
                    let time = CaptureTime::ZERO + CaptureTime::from_secs_f64(t);
                    s.view.request(
                        ViewRequest::MoveToTime { time, anchor: Anchor::Center, duration: JUMP_DURATION },
                        now,
                    );
                }
                None => self.ui.jump_error = true,
            }
        }
    }

    // ---- / edge navigation ----

    /// The transition nearest `from` in the given direction on a digital row.
    fn edge(&self, id: ChannelId, from: CaptureTime, next: bool) -> Option<CaptureTime> {
        let s = self.session.as_ref()?;
        let cap = s.capture.as_ref()?.read();
        let rate = cap.sample_rate()?;
        let ch = cap.channel(id)?;
        let x = from.to_sample_floor(rate).max(0) as u64;
        let sample = if next { ch.next_transition(x) } else { ch.prev_transition(x) }?;
        Some(CaptureTime::from_sample(sample, rate))
    }

    /// Centers the nearest edge before/after the view center.
    pub(crate) fn center_edge(&mut self, id: ChannelId, next: bool) {
        let Some(s) = self.session.as_ref() else { return };
        let v = s.view.target();
        let Some(t) = self.edge(id, v.center_time(), next) else { return };
        let now = self.now();
        let s = self.session.as_mut().unwrap();
        s.follow = false;
        s.view.request(ViewRequest::MoveToTime { time: t, anchor: Anchor::Center, duration: JUMP_DURATION }, now);
    }

    /// The edge buttons on the hovered digital row. Next puts the
    /// first edge past the right side at 25 % from the left; Previous puts
    /// the last edge before the left side at 25 % from the right.
    pub(crate) fn edge_buttons(&mut self, ui: &mut Ui, cell: Rect, id: ChannelId) {
        let Some(s) = self.session.as_ref() else { return };
        if s.follow && s.state.is_busy() {
            return;
        }
        let v = s.view.state();
        let mut go = None;
        for next in [false, true] {
            let from = if next { v.right_time() } else { v.left_time };
            let Some(t) = self.edge(id, from, next) else { continue };
            let x = if next { cell.right() - 6.0 - EDGE_BUTTON } else { cell.left() + 6.0 };
            let size = EDGE_BUTTON.min(cell.height() - 2.0);
            let r = Rect::from_min_size(pos2(x, cell.center().y - size / 2.0), vec2(size, size));
            let label = if next { tr("Next Edge") } else { tr("Previous Edge") };
            let resp = ui.interact(r, ui.make_persistent_id(("edge", id, next)), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
            let theme = ui.la_theme();
            let p = ui.painter();
            let fill = if resp.hovered() { Token::Lift } else { Token::Card };
            p.rect(r, 3.0, theme.color(fill), Stroke::new(1.0, theme.color(Token::Seam)), StrokeKind::Inside);
            icons::jump_edge(p, r.shrink(3.0), next, theme.color(Token::Ink2));
            let key = if next { "." } else { "," };
            if resp.on_hover_text(format!("{label} ({key})")).clicked() {
                go = Some((t, next));
            }
        }
        if let Some((t, next)) = go {
            let x = v.width_px * if next { EDGE_LANDING } else { 1.0 - EDGE_LANDING };
            let target = v.with_time_at(t, x);
            let now = self.now();
            let s = self.session.as_mut().unwrap();
            s.follow = false;
            let req =
                ViewRequest::MoveToRange { t0: target.left_time, t1: target.right_time(), duration: JUMP_DURATION };
            s.view.request(req, now);
        }
    }
}

fn mode_name(m: MarqueeMode) -> &'static str {
    match m {
        MarqueeMode::X => tr("X Range Zoom"),
        MarqueeMode::Y => tr("Y Range Zoom"),
        MarqueeMode::XY => tr("XY Range Zoom"),
    }
}

/// Keys as shown in tooltips.
pub(crate) fn history_keys(forward: bool) -> String {
    match (cfg!(target_os = "macos"), forward) {
        (true, false) => "Cmd+[".into(),
        (true, true) => "Cmd+]".into(),
        (false, false) => "Alt+←".into(),
        (false, true) => "Alt+→".into(),
    }
}

impl LaApp {
    /// *Add Range Measurement* (toggle) and *Add Timing Marker*
    /// (flag + chevron menu), disabled while recording or without data.
    pub(crate) fn annotation_tools(&mut self, ui: &mut Ui, group: Rect) {
        let Some(s) = self.session.as_ref() else { return };
        let enabled = s.has_data() && !s.state.is_busy();
        let tip = |t: &str| {
            if enabled {
                t.to_owned()
            } else if s.state.is_busy() {
                tr("Disabled while recording").into()
            } else {
                tr("{} (needs data)").replacen("{}", t, 1)
            }
        };
        let (range_tip, marker_tip) = (tip(tr("Add Measurement")), tip(tr("Add Timing Marker")));
        // After the capture button, the Repeat toggle and the mode dropdown.
        let x0 = group.left() + CAPTURE_ITEM_W * 2.0 + 3.0 + super::graph::MODE_ITEM_W;
        // Add Analyzer: the picker popover below it.
        let analyzer = Rect::from_min_size(pos2(x0, group.top()), vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let recording = s.state.is_busy();
        let resp = tool_button(ui, analyzer, tr("Add Analyzer"), !recording, false, icons::loupe);
        let resp = resp.on_hover_text(if recording { tr("Disabled while recording") } else { tr("Add Analyzer") });
        // Analyze → Add Analyzer… and the palette open the same picker.
        if std::mem::take(&mut self.ui.open_picker) && !recording {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
        }
        self.analyzer_picker(ui, &resp);
        let Some(s) = self.session.as_ref() else { return };
        let enabled = s.has_data() && !s.state.is_busy();
        let range = Rect::from_min_size(pos2(analyzer.right(), group.top()), vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let placing = self.ui.range_place.is_some();
        let resp = tool_button(ui, range, tr("Add Measurement"), enabled, placing, icons::tape_measure);
        let cmd = cmd_key();
        if resp.on_hover_text(format!("{range_tip} ({cmd}+G, Shift+Drag)")).clicked() {
            self.begin_range_placement();
        }
        let marker = Rect::from_min_size(pos2(range.right(), group.top()), vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let resp = tool_button(ui, marker, tr("Add Timing Marker"), enabled, self.ui.placing.is_some(), icons::flag);
        let chevron =
            Rect::from_min_size(pos2(marker.right(), group.top()), vec2(super::graph::MARKER_CHEVRON_W, TOOLBAR_H));
        let menu = tool_button(ui, chevron, tr("Timing Marker Options"), enabled, false, icons::chevron_down);
        if resp.on_hover_text(format!("{marker_tip} ({cmd}+T)")).clicked() {
            self.begin_marker_placement(true);
        }
        let mut pick = None;
        Popup::menu(&menu).frame(popover_frame(ui)).width(260.0).show(|ui| {
            if crate::ui::widgets::menu_item(ui, tr("Add Marker Pair"), Some("Shift+M")).clicked() {
                pick = Some(true);
            }
            if crate::ui::widgets::menu_item(ui, tr("Add Single Marker"), Some("M")).clicked() {
                pick = Some(false);
            }
        });
        if let Some(pair) = pick {
            self.begin_marker_placement(pair);
        }
    }
}

/// A toolbar item with an icon, a 1 pt right border, and the
/// selected (armed) state.
pub(crate) fn tool_button(
    ui: &mut Ui,
    r: Rect,
    label: &str,
    enabled: bool,
    selected: bool,
    icon: impl Fn(&egui::Painter, Rect, Color32),
) -> egui::Response {
    let theme = ui.la_theme();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(r, ui.make_persistent_id(("tool", label)), sense);
    resp.widget_info(|| WidgetInfo::selected(Role::Button, enabled, selected, label));
    let p = ui.painter();
    let color = if !enabled {
        theme.color(Token::Ink5)
    } else if selected {
        p.rect_filled(r, 0.0, theme.color(Token::Mark));
        theme.color(Token::InkOnFlare)
    } else if resp.hovered() {
        p.rect_filled(r, 0.0, theme.color(Token::Lift));
        theme.color(Token::Ink1)
    } else {
        theme.color(Token::Ink3)
    };
    p.line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
    let size = 16.0f32.min(r.width() - 2.0);
    icon(p, Rect::from_center_size(r.center(), vec2(size, size)), color);
    if !enabled {
        resp.clone().on_hover_cursor(egui::CursorIcon::NotAllowed);
    }
    resp
}
