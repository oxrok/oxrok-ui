//! The graph area (07): toolbar, time ruler, digital rows, scrollbar and
//! recording overlays, plus wheel/drag navigation (08).

use super::{DragState, HoverRow, Marquee, MarqueeKind};
use crate::app::LaApp;
use crate::app::settings::{MarqueeMode, RelativeZoom};
use crate::core::ChannelKind;
use crate::core::animator::{Anchor, JUMP_DURATION, ViewRequest, WHEEL_ZOOM_DURATION};
use crate::core::capture::{CaptureMode, CaptureState};
use crate::core::nav::{WheelAction, WheelInput};
use crate::core::{CaptureTime, ViewState};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::waveform::{RULER_HEIGHT, paint_time_ruler, scrollbar_thumb, ticks_for};
use crate::ui::widgets::shortcut_chip;
use egui::{
    CornerRadius, CursorIcon, Event, MouseWheelSource, MouseWheelUnit, Pos2, Rect, Role, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, WidgetInfo, pos2, vec2,
};
use std::time::Duration;

pub(crate) const TOOLBAR_H: f32 = 30.0;
/// The marker item's chevron part.
pub(crate) const MARKER_CHEVRON_W: f32 = 16.0;

/// The capture group, centered over the plot.
pub(crate) fn capture_group(l: &Layout) -> Rect {
    let w = CAPTURE_ITEM_W * 5.0 + 3.0 + MODE_ITEM_W + MARKER_CHEVRON_W;
    Rect::from_min_size(pos2(l.plot_rows.center().x - w / 2.0, l.toolbar.top()), vec2(w, TOOLBAR_H))
}
pub(crate) const CAPTURE_ITEM_W: f32 = 48.0;
/// The capture mode dropdown: the mode's icon and a chevron.
pub(crate) const MODE_ITEM_W: f32 = CAPTURE_ITEM_W + MARKER_CHEVRON_W;
const SCROLLBAR_H: f32 = 10.0;
/// double-click zoom factor.
const DOUBLE_CLICK_ZOOM: f64 = 0.3;
/// Points per wheel line when scrolling rows with Space held.
const LINE_SCROLL: f32 = 40.0;

/// Areas of the graph for one frame.
pub(crate) struct Layout {
    pub graph: Rect,
    pub toolbar: Rect,
    pub label_header: Rect,
    pub ruler: Rect,
    pub rows: Rect,
    pub plot_rows: Rect,
    pub scrollbar: Rect,
    pub label_w: f32,
    /// The read-only banner under the toolbar, for loaded files.
    pub banner: Option<Rect>,
}

impl Layout {
    fn new(rect: Rect, label_width: f32, banner: bool) -> Self {
        // 53 pt (collapsed) to 53 + 240 pt; the plot keeps ≥ 50 pt.
        let max = super::rowctl::LABEL_MAX.min(rect.width() - 50.0).max(super::rowctl::LABEL_MIN);
        let label_w = label_width.clamp(super::rowctl::LABEL_MIN, max);
        let plot_left = rect.left() + label_w;
        let toolbar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + TOOLBAR_H));
        let banner = banner.then(|| {
            Rect::from_min_max(
                pos2(rect.left(), toolbar.bottom()),
                pos2(rect.right(), toolbar.bottom() + super::files::BANNER_H),
            )
        });
        let top = banner.map_or(toolbar.bottom(), |b| b.bottom());
        let ruler = Rect::from_min_max(pos2(plot_left, top), pos2(rect.right(), top + RULER_HEIGHT));
        let label_header = Rect::from_min_max(pos2(rect.left(), top), pos2(plot_left, ruler.bottom()));
        let scrollbar = Rect::from_min_max(pos2(plot_left, rect.bottom() - SCROLLBAR_H), rect.max);
        let rows = Rect::from_min_max(pos2(rect.left(), ruler.bottom()), pos2(rect.right(), scrollbar.top()));
        let plot_rows = Rect::from_min_max(pos2(plot_left, rows.top()), rows.max);
        Self { graph: rect, toolbar, label_header, ruler, rows, plot_rows, scrollbar, label_w, banner }
    }
}

impl LaApp {
    pub(crate) fn graph(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        self.ui.graph_rect = super::DefaultRect(rect);
        let read_only = self.session.as_ref().is_some_and(|s| s.is_read_only());
        let banner = read_only && !self.session.as_ref().is_some_and(|s| s.banner_closed);
        let l = Layout::new(rect, self.settings.label_width, banner);
        self.auto_layout(ui, &l);
        self.update_view(l.plot_rows.width() as f64);
        let plot = self.graph_input(ui, &l);
        let Some(view) = self.update_view(l.plot_rows.width() as f64) else { return };
        self.paint_rows(ui, &l, &view);
        self.label_splitter(ui, &l);
        // On the plot cell: the row under the right-click.
        if plot.secondary_clicked()
            && let Some(at) = plot.interact_pointer_pos()
            && let Some(g) = self.ui.row_geometry.iter().find(|g| g.cell.contains(at))
        {
            self.ui.context_row = Some(g.channel);
            self.ui.open_row_menu_at = Some(at);
        }
        self.row_context_menu(ui, &plot, &l);
        self.ranges_ui(ui, &l, &view);
        self.paint_marquee(ui, &l);
        self.paint_ruler(ui, &l, &view);
        self.markers_ui(ui, &l, &view, &plot);
        self.trigger_ui(ui, &l, &view);
        // A click on empty plot space deselects.
        if plot.clicked()
            && self.ui.placing.is_none()
            && !ui.input(|i| i.modifiers.command || i.modifiers.shift)
            && let Some(s) = self.session.as_mut()
        {
            s.markers.deselect_all();
            for r in &mut s.ranges.items {
                r.selected = false;
            }
        }
        self.toolbar(ui, &l);
        if let Some(b) = l.banner {
            self.file_banner(ui, b);
        } else if read_only {
            self.file_banner_icon(ui, &l);
        }
        self.secondary_tools(ui, &l);
        self.jump_to_time(ui, &l);
        self.scrollbar(ui, &l, &view);
        self.overlays(ui, &l);
    }

    // ---- input ----

    fn graph_input(&mut self, ui: &mut Ui, l: &Layout) -> egui::Response {
        let resp = ui.interact(l.plot_rows, ui.make_persistent_id("plot"), Sense::click_and_drag());
        let now = self.now();
        // Pressing on the graph cancels any animation.
        if resp.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.any_pressed())
            && let Some(s) = self.session.as_mut()
        {
            s.view.cancel(now);
        }
        let geometry = self.ui.row_geometry.clone();
        let row_at = |p: egui::Pos2| geometry.iter().copied().find(|g| g.cell.contains(p));
        let (mods, escape) = ui.input(|i| (i.modifiers, i.key_pressed(egui::Key::Escape)));
        if escape {
            // Escape cancels a marquee and disarms the tool.
            self.ui.marquee = None;
            self.ui.zoom_tool_armed = false;
        }
        if (self.ui.zoom_tool_armed && resp.hovered()) || self.ui.marquee.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        // double-click zooms in ×0.3 about the clicked time.
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.zoom_view(DOUBLE_CLICK_ZOOM, (p.x - l.plot_rows.left()) as f64, WHEEL_ZOOM_DURATION);
        }
        // Range placement takes the plot's clicks and drags.
        let view_now = self.session.as_ref().map(|s| s.view.state());
        if let Some(v) = view_now
            && self.range_input(ui, &resp, l, &v)
        {
            return resp;
        }
        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
            // Ctrl/Cmd+Shift = one axis, Ctrl/Cmd = XY, or the armed tool.
            let kind = if mods.command && mods.shift {
                Some((MarqueeKind::Single, true))
            } else if mods.command {
                Some((MarqueeKind::Fixed(MarqueeMode::XY), true))
            } else if self.ui.zoom_tool_armed {
                Some((MarqueeKind::Fixed(self.settings.marquee_mode), false))
            } else {
                None
            };
            if let (Some((kind, by_modifiers)), Some(p)) = (kind, origin) {
                self.ui.zoom_tool_armed = false;
                self.ui.marquee = Some(Marquee { start: p, current: p, kind, by_modifiers, row: row_at(p) });
            } else {
                // Horizontal pans 1:1; on analog rows a steep
                // drag (> 5 px, > 60°) pans the value axis from then on.
                let start = origin.and_then(row_at);
                let analog = start.filter(|g| g.is_analog());
                let on_ruler = analog.zip(origin).is_some_and(|(g, p)| g.ruler().contains(p));
                self.ui.drag = Some(DragState { analog, on_ruler, total: egui::Vec2::ZERO, vertical: false });
                self.ui.velocity.clear();
                if let Some(p) = resp.interact_pointer_pos() {
                    self.ui.velocity.push(now, p.x as f64);
                }
            }
        }
        if let Some(m) = self.ui.marquee.as_mut() {
            if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                m.current = p;
            }
            // A modifier marquee ends when the modifiers go before the button.
            if m.by_modifiers && !mods.command {
                self.ui.marquee = None;
            } else if resp.drag_stopped() {
                let m = *m;
                self.ui.marquee = None;
                self.apply_marquee(m, l.plot_rows);
            }
        } else if resp.dragged()
            && let Some(mut d) = self.ui.drag
        {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let delta = resp.drag_delta();
            d.total += delta;
            if let Some(p) = resp.interact_pointer_pos() {
                self.ui.velocity.push(now, p.x as f64);
            }
            let ppp = ui.ctx().pixels_per_point();
            if !d.vertical && d.analog.is_some() {
                let steep = d.total.y.abs() > d.total.x.abs() * 60f32.to_radians().tan();
                d.vertical = d.total.y.abs() * ppp > 5.0 && steep;
            }
            if d.vertical
                && let Some(g) = d.analog
                && let Some(s) = self.session.as_mut()
                && let Some(v) = s.analog_view(g.channel, g.inner.height() as f64)
            {
                s.set_analog_view(g.channel, v.pan(delta.y as f64, g.inner.height() as f64));
            }
            if !d.on_ruler && delta.x != 0.0 {
                self.pan_view(-delta.x as f64, Duration::ZERO);
            }
            self.ui.drag = Some(d);
        }
        if resp.drag_stopped()
            && let Some(d) = self.ui.drag.take()
        {
            // Fling with half the release velocity.
            let v = self.ui.velocity.velocity(now);
            if v.abs() > 0.01 && !d.vertical && !d.on_ruler {
                self.fling(-v / 2.0);
            }
        }

        // The wheel over the label column scrolls the rows.
        let labels = Rect::from_x_y_ranges(l.rows.left()..=l.plot_rows.left(), l.rows.y_range());
        if ui.rect_contains_pointer(labels) && !egui::Popup::is_any_open(ui.ctx()) && self.ui.palette.is_none() {
            let dy: f32 = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        Event::MouseWheel { unit, delta, .. } => {
                            Some(delta.y * if *unit == MouseWheelUnit::Point { 1.0 } else { LINE_SCROLL })
                        }
                        _ => None,
                    })
                    .sum()
            });
            self.ui.rows_scroll -= dy;
            return resp;
        }
        // Not while the palette is open.
        let over = ui.rect_contains_pointer(l.plot_rows.union(l.ruler))
            && !egui::Popup::is_any_open(ui.ctx())
            && self.ui.palette.is_none();
        if !over {
            return resp;
        }
        // The zoom fix-point is the pointer or the view center.
        let center_x = l.plot_rows.width() as f64 / 2.0;
        let pointer_x = match self.settings.waveform.zoom_relative_to {
            RelativeZoom::Mouse => {
                ui.input(|i| i.pointer.hover_pos()).map_or(center_x, |p| (p.x - l.plot_rows.left()) as f64)
            }
            RelativeZoom::Center => center_x,
        };
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let analog_row = pointer.and_then(row_at).filter(|g| g.is_analog());
        let events = ui.input(|i| i.events.clone());
        for e in events {
            match e {
                Event::MouseWheel { unit, delta, modifiers, source, .. } => {
                    // Notch units × 120, positive dy = zoom in. A notched
                    // wheel is discrete; trust the platform's source when it says.
                    let k = match unit {
                        MouseWheelUnit::Line => 120.0,
                        MouseWheelUnit::Page => 360.0,
                        MouseWheelUnit::Point => 1.0,
                    };
                    let discrete = match source {
                        MouseWheelSource::Wheel => true,
                        MouseWheelSource::Trackpad => false,
                        _ => unit != MouseWheelUnit::Point,
                    };
                    // Ctrl/Cmd, or the pointer on the voltage ruler,
                    // zooms an analog row vertically (Shift = fine).
                    if let (Some(g), Some(p)) = (analog_row, pointer)
                        && (modifiers.command || g.ruler().contains(p))
                    {
                        let factor = crate::core::analog::vertical_zoom_factor(delta.y as f64 * k, modifiers.shift);
                        self.ui.hover_row = Some(HoverRow {
                            channel: g.channel,
                            y: p.y - g.inner.top(),
                            inner_height: g.inner.height(),
                        });
                        let y = match self.settings.waveform.zoom_relative_to {
                            RelativeZoom::Mouse => p.y - g.inner.top(),
                            RelativeZoom::Center => g.inner.height() / 2.0,
                        };
                        self.zoom_analog(g.channel, factor, y);
                        continue;
                    }
                    let mut dx = delta.x as f64 * k;
                    if discrete && cfg!(target_os = "macos") {
                        dx = -dx;
                    }
                    let input = WheelInput { dx, dy: delta.y as f64 * k, discrete, command: modifiers.command };
                    match self.ui.wheel.classify(input, now) {
                        WheelAction::HorizontalZoom { factor, discrete } => {
                            let d = if discrete { WHEEL_ZOOM_DURATION } else { Duration::ZERO };
                            self.zoom_view(factor, pointer_x, d);
                        }
                        WheelAction::HorizontalPan { dx_px } => self.pan_view(dx_px, Duration::ZERO),
                        WheelAction::Ignore => {}
                    }
                }
                // Pinch: treat as a non-discrete zoom.
                Event::Zoom(f) if f > 0.0 => self.zoom_view(1.0 / f as f64, pointer_x, Duration::ZERO),
                _ => {}
            }
        }
        resp
    }

    // ---- painting ----

    fn paint_ruler(&self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let theme = ui.la_theme();
        let p = ui.painter();
        p.rect_filled(l.label_header, 0.0, theme.color(Token::Deck));
        p.line_segment(
            [l.label_header.right_top(), l.label_header.right_bottom()],
            Stroke::new(1.0, theme.color(Token::Seam)),
        );
        p.rect_filled(l.ruler, 0.0, theme.color(Token::Canvas));
        // Labels relative to T0; the date at T0.
        let mut ticks = ticks_for(view, self.session.as_ref().map_or(CaptureTime::ZERO, |s| s.t0()));
        if let Some(s) = self.session.as_ref()
            && s.origin == crate::core::markers::TimeOrigin::WallClock
            && let Some(start) = s.capture.as_ref().and_then(|c| c.read().header.as_ref().map(|h| h.start))
        {
            let date = crate::core::format::wall_clock(&start, CaptureTime::ZERO, self.settings.time_labels.format());
            for t in ticks.ticks.iter_mut().chain(ticks.offscreen_major.as_mut()) {
                if t.major && t.value == CaptureTime::ZERO {
                    t.label = date.clone();
                }
            }
        }
        paint_time_ruler(p, l.ruler, &ticks, &theme);
        let bottom = Rect::from_min_max(pos2(l.label_header.left(), l.ruler.bottom() - 1.0), l.ruler.right_bottom());
        p.rect_filled(bottom, 0.0, theme.color(Token::Seam));
    }

    /// The toolbar strip with the capture group centered over the plot.
    fn toolbar(&mut self, ui: &mut Ui, l: &Layout) {
        let theme = ui.la_theme();
        let ctx = ui.ctx().clone();
        let p = ui.painter();
        p.rect_filled(l.toolbar, 0.0, theme.color(Token::Canvas));
        p.line_segment([l.toolbar.left_bottom(), l.toolbar.right_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));

        let Some(s) = self.session.as_ref() else { return };
        // Capture, range measurement, timing marker (+ menu chevron).
        let group = capture_group(l);
        let item = Rect::from_min_size(group.min, vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let radius = CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 };
        p.add(theme.box_shadow().as_shape(group, radius));
        p.rect_filled(group, radius, theme.color(Token::Card));
        // A 3 pt right border separates the capture button from the rest.
        p.rect_filled(Rect::from_min_size(item.right_top(), vec2(3.0, TOOLBAR_H)), 0.0, theme.color(Token::Seam));

        // A Repeat run is one recording, between rounds too.
        let run = s.repeat.as_ref().is_some_and(|r| !r.user_stop) && s.state.is_busy();
        let recording = run || matches!(s.state, CaptureState::Starting | CaptureState::Recording);
        let label = if recording {
            tr("Stop")
        } else if s.device.is_demo {
            tr("Start Demo")
        } else {
            tr("Start")
        };
        let enabled = run || s.state != CaptureState::Stopping;
        let resp = ui.interact(
            item,
            ui.make_persistent_id("capture_button"),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
        let pressed = resp.is_pointer_button_down_on();
        let p = ui.painter();
        if pressed {
            p.rect_filled(item, radius, theme.color(Token::Mark));
        } else if resp.hovered() {
            p.rect_filled(item, radius, theme.color(Token::Lift));
        }
        let icon_color = if pressed { theme.color(Token::InkOnFlare) } else { theme.color(Token::Mark) };
        let icon_rect = Rect::from_center_size(item.center(), vec2(14.0, 14.0));
        if recording {
            icons::stop(p, icon_rect, icon_color);
        } else {
            icons::play(p, icon_rect, icon_color);
        }
        // The Repeat mark while a run goes on.
        if run {
            let mark = Rect::from_min_size(pos2(item.right() - 12.0, item.bottom() - 12.0), vec2(9.0, 9.0));
            icons::repeat(p, mark, icon_color);
        }
        // A red badge when recording isn't possible; the tooltip
        // gives the reason.
        let reason = self.capture_blocked();
        if reason.is_some() {
            let c = pos2(item.right() - 7.0, item.top() + 7.0);
            ui.painter().circle_filled(c, 4.0, theme.color(Token::Alert));
        }
        // The read-only case offers a new session in the tooltip.
        let offer_new = self.session.as_ref().is_some_and(|s| s.is_read_only() && !s.state.is_busy());
        let mut new_session = false;
        let resp = resp.on_hover_ui(|ui| match &reason {
            Some(r) => {
                ui.label(r);
                if offer_new {
                    ui.add_space(6.0);
                    new_session = ui.add(crate::ui::widgets::Button::primary(tr("New Session"))).clicked();
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(label);
                    shortcut_chip(ui, "Space");
                });
            }
        });
        self.capture_status(ui, l);
        if new_session {
            self.new_session();
        } else if resp.clicked() {
            self.toggle_capture(&ctx);
        }
        self.repeat_button(ui, item);
        self.capture_mode_button(ui, item);
        self.annotation_tools(ui, group);
        self.inspector_toggle_button(ui, l);
    }

    /// The Inspector toggle, at the left end of the toolbar.
    fn inspector_toggle_button(&mut self, ui: &mut Ui, l: &Layout) {
        let has_session = self.session.is_some();
        let r = Rect::from_min_size(pos2(l.toolbar.left(), l.toolbar.top()), vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let on = has_session && self.settings.inspector.open;
        let resp = super::nav::tool_button(ui, r, tr("Inspector"), has_session, on, icons::panel);
        let tip = if has_session { tr("Inspector (⌘\\)") } else { tr("Inspector (needs a session)") };
        if resp.on_hover_text(tip).clicked() {
            self.toggle_inspector();
        }
    }

    /// The Repeat toggle, right of the capture button.
    fn repeat_button(&mut self, ui: &mut Ui, capture: Rect) {
        let Some(s) = self.session.as_ref() else { return };
        let r = Rect::from_min_size(pos2(capture.right() + 3.0, capture.top()), vec2(CAPTURE_ITEM_W, TOOLBAR_H));
        let available = self.command_available("capture.repeat");
        let on = s.settings.repeat;
        let resp = super::nav::tool_button(ui, r, tr("Repeat"), available.is_ok(), on, icons::repeat);
        let tip = match available {
            Err(why) => why,
            Ok(()) if on => tr("Repeat is on: each capture re-arms when it ends"),
            Ok(()) => tr("Repeat: wait for the next trigger each time a capture ends"),
        };
        if resp.on_hover_text(tip).clicked() {
            self.dispatch("capture.repeat");
        }
    }

    /// The capture mode dropdown (Trigger, Timed, Continuous), right of
    /// the Repeat toggle.
    fn capture_mode_button(&mut self, ui: &mut Ui, capture: Rect) {
        let Some(s) = self.session.as_ref() else { return };
        let left = capture.right() + 3.0 + CAPTURE_ITEM_W;
        let r = Rect::from_min_size(pos2(left, capture.top()), vec2(MODE_ITEM_W, TOOLBAR_H));
        let mode = s.settings.mode;
        let enabled = s.caps.is_some() && !s.state.is_busy() && !s.is_read_only();
        let streams = s.caps.as_ref().is_some_and(|c| c.supports_continuous);
        let any_digital =
            s.rows.iter().any(|r| r.channel.kind == ChannelKind::Digital && s.settings.is_enabled(r.channel));
        let icon = |p: &egui::Painter, _: Rect, c| {
            let i = Rect::from_center_size(pos2(r.left() + CAPTURE_ITEM_W / 2.0, r.center().y), vec2(16.0, 16.0));
            mode_icon(mode)(p, i, c);
            let ch = Rect::from_center_size(
                pos2(r.right() - MARKER_CHEVRON_W / 2.0 - 2.0, r.center().y),
                vec2(MARKER_CHEVRON_W - 4.0, MARKER_CHEVRON_W - 4.0),
            );
            icons::chevron_down(p, ch, c);
        };
        let resp = super::nav::tool_button(ui, r, tr("Capture Mode"), enabled, false, icon);
        let tip = if s.state.is_busy() {
            tr("Disabled while recording").to_owned()
        } else {
            tr("Capture mode: {}").replacen("{}", mode_name(mode), 1)
        };
        let resp = resp.on_hover_text(tip);
        let mut pick = None;
        egui::Popup::menu(&resp).frame(crate::ui::widgets::popover_frame(ui)).width(240.0).show(|ui| {
            for (m, ok, tip) in [
                (
                    CaptureMode::Triggered,
                    any_digital,
                    if any_digital {
                        tr("Record continuously until a trigger condition is found")
                    } else {
                        tr("Enable a digital channel first.")
                    },
                ),
                (CaptureMode::Timed, true, tr("Capture for a fixed duration")),
                (
                    CaptureMode::Continuous,
                    streams,
                    if streams {
                        tr("Continuous recording that discards the oldest data as new data arrives")
                    } else {
                        tr("This device records into its own memory and can't stream. Use Trigger or Timed.")
                    },
                ),
            ] {
                let item =
                    ui.add_enabled_ui(ok, |ui| crate::ui::widgets::menu_check_item(ui, mode_name(m), m == mode, None));
                let item = item.inner;
                let item = if ok { item.on_hover_text(tip) } else { item.on_disabled_hover_text(tip) };
                if item.clicked() {
                    pick = Some(m);
                }
            }
        });
        if let Some(m) = pick
            && m != mode
        {
            self.session.as_mut().unwrap().settings.mode = m;
            self.mark_settings_dirty();
        }
    }

    /// Horizontal scrollbar.
    fn scrollbar(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let theme = ui.la_theme();
        ui.painter().rect_filled(l.scrollbar, 0.0, theme.color(Token::Deck));
        let Some(s) = self.session.as_ref() else { return };
        let Some(b) = s.bounds() else { return };
        let following = s.follow && s.state.is_busy();
        let track_w = l.scrollbar.width();
        let Some((x, w)) = scrollbar_thumb(view, b.begin, b.end, track_w, following) else { return };
        let thumb =
            Rect::from_min_size(pos2(l.scrollbar.left() + x, l.scrollbar.top() + 2.0), vec2(w, SCROLLBAR_H - 4.0));
        let track = ui.interact(l.scrollbar, ui.make_persistent_id("hscroll_track"), Sense::click());
        let t = ui.interact(thumb, ui.make_persistent_id("hscroll_thumb"), Sense::drag());
        let travel_secs = b.duration_secs() - view.time_scale();
        if t.dragged() && track_w > w && travel_secs > 0.0 {
            // Dragging the thumb pans immediately.
            let dt = t.drag_delta().x as f64 / (track_w - w) as f64 * travel_secs;
            self.pan_view(dt / view.time_per_px, Duration::ZERO);
        } else if track.clicked()
            && let Some(pos) = track.interact_pointer_pos()
        {
            // Clicking the track centers the thumb on the click (animated).
            let frac = ((pos.x - l.scrollbar.left() - w / 2.0) / (track_w - w).max(1.0)).clamp(0.0, 1.0) as f64;
            let left = b.begin + CaptureTime::from_secs_f64(frac * travel_secs.max(0.0));
            let now = self.now();
            let s = self.session.as_mut().unwrap();
            s.follow = false;
            s.view.request(ViewRequest::MoveToTime { time: left, anchor: Anchor::Left, duration: JUMP_DURATION }, now);
        }
        let c = if t.hovered() || t.dragged() { Token::Ink4 } else { Token::Seam };
        ui.painter().rect_filled(thumb, 3.0, theme.color(c));
    }

    /// Live chip, progress card.
    fn overlays(&mut self, ui: &mut Ui, l: &Layout) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        let busy = s.state.is_busy();

        if busy {
            let follow = s.follow;
            // "Live Trigger" once the Analyzer Trigger has fired.
            let live = if s.follows_analyzer_trigger() { tr("Live Trigger") } else { tr("Live") };
            let text_galley =
                ui.painter().layout_no_wrap(live.into(), Text::CaptionMed.font_id(), theme.color(Token::Ink1));
            let size = vec2(text_galley.size().x + 30.0, 22.0);
            let chip = Rect::from_min_size(pos2(l.plot_rows.left() + 8.0, l.plot_rows.bottom() - 8.0 - size.y), size);
            let resp = ui.interact(chip, ui.make_persistent_id("live_chip"), Sense::click());
            resp.widget_info(|| WidgetInfo::selected(Role::Button, true, follow, live));
            let p = ui.painter();
            p.rect(
                chip,
                CornerRadius::same(u8::MAX),
                theme.color(Token::Field),
                Stroke::new(1.0, theme.color(Token::Seam)),
                StrokeKind::Inside,
            );
            let dot = if follow { theme.color(Token::Alert) } else { theme.color(Token::Ink4) };
            p.circle_filled(pos2(chip.left() + 12.0, chip.center().y), 3.0, dot);
            p.galley(
                pos2(chip.left() + 20.0, chip.center().y - text_galley.size().y / 2.0),
                text_galley,
                egui::Color32::PLACEHOLDER,
            );
            let tip = if follow { tr("Click to freeze the auto-pan.") } else { tr("Click to follow the newest data.") };
            if resp.on_hover_text(tip).clicked() {
                let s = self.session.as_mut().unwrap();
                s.follow = !s.follow;
            }
            self.progress_card(ui, l);
        }
    }

    fn progress_card(&mut self, ui: &mut Ui, l: &Layout) {
        // Not for devices that record into their own memory.
        let buffered = self.session.as_ref().and_then(|s| s.caps.as_ref()).is_some_and(|c| !c.supports_continuous);
        if self.ui.progress_minimized || buffered {
            return;
        }
        let theme = ui.la_theme();
        let Some(p) = self.progress() else { return };
        let round = self.session.as_ref().and_then(|s| s.repeat_round());
        let backlog = matches!(p.trigger, Some(TriggerProgressUi::Searching { backlog }) if backlog > 0.5);
        let h = if p.timer.is_some() || p.trigger.is_some() { 112.0 } else { 64.0 } + if backlog { 20.0 } else { 0.0 };
        let size = vec2(260.0, h);
        let card =
            Rect::from_min_size(Pos2::new(l.rows.right() - 8.0 - size.x, l.scrollbar.top() - 8.0 - size.y), size);
        let painter = ui.painter();
        painter.add(theme.box_shadow().as_shape(card, 3));
        painter.rect(
            card,
            3.0,
            theme.color(Token::Card),
            Stroke::new(1.0, theme.color(Token::Seam)),
            StrokeKind::Inside,
        );
        let bar = |ui: &mut Ui, frac: f32, color: egui::Color32, under: Option<egui::Color32>| {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, theme.color(Token::Hair));
            if let Some(u) = under {
                ui.painter().rect_filled(r, 2.0, u);
            }
            let fill = Rect::from_min_size(r.min, vec2(r.width() * frac.clamp(0.01, 1.0), r.height()));
            ui.painter().rect_filled(fill, 2.0, color);
        };
        ui.scope_builder(UiBuilder::new().max_rect(card.shrink(16.0)), |ui| {
            let row = |ui: &mut Ui, title: &str, value: &str| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).text_style(Text::Lead.style()).color(theme.color(Token::Ink1)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new(value).color(theme.color(Token::Ink2)));
                    });
                });
            };
            let suffix = round.map_or(String::new(), |n| tr(" · round {}").replacen("{}", &n.to_string(), 1));
            let title = |t: &str| format!("{t}{suffix}");
            if let Some((frac, remaining)) = p.timer {
                row(ui, &title(tr("Recording...")), &remaining);
                ui.add_space(6.0);
                // Processed over collected; both are the same without a backlog.
                bar(ui, frac, theme.color(Token::Good), Some(theme.color(Token::Good).gamma_multiply(0.3)));
                ui.add_space(14.0);
            }
            match &p.trigger {
                Some(TriggerProgressUi::Searching { backlog }) => {
                    row(ui, &title(tr("Waiting for trigger...")), "");
                    ui.add_space(6.0);
                    // Indeterminate: a sliding stripe.
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, theme.color(Token::Hair));
                    let t = ui.input(|i| i.time) as f32;
                    let x = r.left() + (t * 0.8).fract() * (r.width() + 60.0) - 60.0;
                    let stripe = Rect::from_x_y_ranges(x.max(r.left())..=(x + 60.0).min(r.right()), r.y_range());
                    ui.painter().rect_filled(stripe, 2.0, theme.color(Token::Good));
                    ui.ctx().request_repaint();
                    if *backlog > 0.5 {
                        ui.add_space(6.0);
                        ui.label(tr("Trigger Backlog: {} s.").replacen("{}", &format!("{backlog:.1}"), 1));
                    }
                    ui.add_space(14.0);
                }
                Some(TriggerProgressUi::Triggered { frac, remaining, .. }) => {
                    row(ui, &title(tr("Recording...")), remaining);
                    ui.add_space(6.0);
                    bar(ui, *frac, theme.color(Token::Good), None);
                    ui.add_space(14.0);
                }
                None => {}
            }
            row(ui, tr("Memory"), &format!("{:.1} MB", p.used_mb));
            ui.add_space(6.0);
            bar(ui, p.memory, theme.color(Token::Caution), None);
        });
        // ✕ minimizes the card into the rail.
        let close = Rect::from_min_size(pos2(card.right() - 22.0, card.top() + 8.0), vec2(14.0, 14.0));
        let resp = ui.interact(close, ui.make_persistent_id("progress_minimize"), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Minimize")));
        let c = if resp.hovered() { Token::Ink1 } else { Token::Ink3 };
        icons::close(ui.painter(), close, theme.color(c));
        if resp.on_hover_text(tr("Minimize")).clicked() {
            self.ui.progress_minimized = true;
        }
    }
}

/// Progress figures.
pub(crate) struct Progress {
    /// Timer mode: processed fraction and remaining time `m:ss`.
    pub timer: Option<(f32, String)>,
    /// Trigger mode: searching (with the scan backlog, seconds), or the
    /// post-trigger fraction and time left.
    pub trigger: Option<TriggerProgressUi>,
    pub used_mb: f64,
    /// Used / buffer.
    pub memory: f32,
}

/// Trigger mode state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TriggerProgressUi {
    Searching { backlog: f64 },
    Triggered { frac: f32, remaining: String, since: f64 },
}

impl LaApp {
    pub(crate) fn progress(&self) -> Option<Progress> {
        let s = self.session.as_ref()?;
        if !s.state.is_busy() {
            return None;
        }
        let snap = s.snapshot.as_ref()?;
        // The recording round, plus the round kept on screen.
        let kept_mb = s.previous.as_ref().and_then(|p| {
            let c = if s.previous_shown { s.capture.as_ref() } else { p.capture.as_ref() }?;
            Some(c.read().memory_bytes() as f64 / 1e6)
        });
        let (used_mb, processed) = s.live_capture().map_or((0.0, 0.0), |c| {
            let d = c.read();
            let secs = d.sample_range().zip(d.sample_rate()).map_or(0.0, |((b, e), r)| (e - b) as f64 / r as f64);
            // The capture, the round kept on screen, and the
            // analyzers' results.
            let decoders = s.analyzer_memory_bytes() as f64 / 1e6;
            (d.memory_bytes() as f64 / 1e6 + kept_mb.unwrap_or(0.0) + decoders, secs)
        });
        let timer = (snap.mode == crate::core::capture::CaptureMode::Timed).then(|| {
            let remaining = (snap.timer_seconds - processed).max(0.0).ceil() as u64;
            ((processed / snap.timer_seconds) as f32, format!("{}:{:02}", remaining / 60, remaining % 60))
        });
        // Trigger mode.
        // Before any data (a buffered device delivers it at the end) the
        // trigger is still awaited.
        let trigger = snap.trigger.as_ref().map(|t| {
            let waiting = TriggerProgressUi::Searching { backlog: 0.0 };
            let Some(d) = s.live_capture().map(|c| c.read()) else { return waiting };
            let (Some(rate), Some((_, end))) = (d.sample_rate(), d.sample_range()) else { return waiting };
            match d.trigger {
                None => {
                    TriggerProgressUi::Searching { backlog: end.saturating_sub(d.trigger_scan) as f64 / rate as f64 }
                }
                Some(tr) => {
                    let since = end.saturating_sub(tr) as f64 / rate as f64;
                    let left = (t.post_trigger_seconds - since).max(0.0);
                    let frac = if t.post_trigger_seconds > 0.0 { (since / t.post_trigger_seconds) as f32 } else { 1.0 };
                    let secs = left.ceil() as u64;
                    TriggerProgressUi::Triggered { frac, remaining: format!("{}:{:02}", secs / 60, secs % 60), since }
                }
            }
        });
        Some(Progress { timer, trigger, used_mb, memory: (used_mb / (snap.buffer_mb as f64).max(1.0)) as f32 })
    }
}

/// Why an imported file's session can't record.
fn mode_name(m: CaptureMode) -> &'static str {
    match m {
        CaptureMode::Triggered => tr("Trigger"),
        CaptureMode::Timed => tr("Timed"),
        CaptureMode::Continuous => tr("Continuous"),
    }
}

fn mode_icon(m: CaptureMode) -> fn(&egui::Painter, Rect, egui::Color32) {
    match m {
        CaptureMode::Triggered => icons::trigger_edge,
        CaptureMode::Timed => icons::stopwatch,
        CaptureMode::Continuous => icons::infinity,
    }
}

pub(crate) fn import_cannot_record(s: &crate::app::session::Session) -> String {
    let format = s.device.display_name();
    if format.is_empty() {
        tr("An imported file has no device to record with. Start a new session on a device.").to_owned()
    } else {
        tr("An imported {} file has no device to record with. Start a new session on a device.")
            .replacen("{}", &format, 1)
    }
}

impl LaApp {
    /// Why recording isn't possible right now, if it isn't.
    pub fn capture_blocked(&self) -> Option<String> {
        let s = self.session.as_ref()?;
        if s.state.is_busy() {
            return None;
        }
        if s.open_error.is_some() {
            return Some(tr("The device was disconnected. Reconnect it or start a new session.").into());
        }
        if s.is_import() {
            return Some(import_cannot_record(s));
        }
        // An opened file records when its kind of device is here.
        if s.is_read_only() {
            return self.matching_device().is_none().then(|| {
                tr("Connect a {} to record with these settings.").replacen("{}", &s.device.display_name(), 1)
            });
        }
        if s.caps.is_none() {
            return Some(tr("Switching device…").into());
        }
        if s.settings.enabled.is_empty() {
            return Some(tr("Enable at least one channel to record.").into());
        }
        let triggered = s.settings.mode == crate::core::capture::CaptureMode::Triggered;
        (triggered && !s.settings.trigger.is_valid(&s.settings.enabled))
            .then(|| tr("The trigger setup is invalid.").into())
    }

    /// Timer or trigger status above the label column.
    fn capture_status(&self, ui: &Ui, l: &Layout) {
        let Some(p) = self.progress() else { return };
        let text = if let Some((_, remaining)) = &p.timer {
            tr("Timer {}").replacen("{}", remaining, 1)
        } else {
            match &p.trigger {
                Some(TriggerProgressUi::Searching { .. }) => {
                    // An animated ellipsis.
                    let n = (ui.input(|i| i.time) * 3.0) as usize % 4;
                    tr("Waiting{}").replacen("{}", &".".repeat(n), 1)
                }
                Some(TriggerProgressUi::Triggered { since, .. }) => {
                    tr("Triggered +{}").replacen("{}", &crate::core::format::time_short(*since), 1)
                }
                None => return,
            }
        };
        let text = match self.session.as_ref().and_then(|s| s.repeat_round()) {
            Some(n) => format!("{text} · #{n}"),
            None => text,
        };
        let theme = ui.la_theme();
        let area = Rect::from_min_max(l.toolbar.min, pos2(l.toolbar.left() + l.label_w, l.toolbar.bottom()));
        let resp = ui.interact(area, ui.make_persistent_id("capture_status"), Sense::hover());
        resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
        ui.painter().text(
            area.left_center() + vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &text,
            Text::CaptionMed.font_id(),
            theme.color(Token::Ink2),
        );
    }
}
