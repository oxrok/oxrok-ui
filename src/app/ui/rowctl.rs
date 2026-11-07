//! Row controls (09): the label column splitter, inline rename
//! fit to window and Auto Layout, reordering
//! hiding and the row context menu.

use super::graph::Layout;
use super::{Rename, label_text_edit};
use crate::app::LaApp;
use crate::app::session::{ANALOG_ROW_HEIGHT, DIGITAL_ROW_HEIGHT};
use crate::core::animator::{JUMP_DURATION, ViewRequest};
use crate::core::bus::{BusFormat, Endianness};
use crate::core::layout::{FitRow, fit_rows};
use crate::core::{ChannelId, ChannelKind, ViewState};
use crate::ui::Token;
use crate::ui::colors::channel_color;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_check_item, menu_item, menu_separator, popover_frame};
use egui::containers::menu::{MenuConfig, SubMenu};
use egui::{Key, Popup, PopupCloseBehavior, Rect, Response, Role, Sense, Ui, WidgetInfo, pos2, vec2};

/// Limits: the handle width is the minimum (collapsed).
pub(crate) const LABEL_MIN: f32 = 53.0;
pub(crate) const LABEL_MAX: f32 = LABEL_MIN + 240.0;
/// Splitter handle width.
const SPLITTER_W: f32 = 9.0;
/// The per-row color choices (the palette's names).
const COLOR_NAMES: [&str; 8] = ["Gray", "Teal", "Coral", "Gold", "Sky", "Lime", "Magenta", "Indigo"];

/// Whether the label column is at its minimum (names hidden).
pub(crate) fn collapsed(label_w: f32) -> bool {
    label_w <= LABEL_MIN + 0.5
}

/// A context menu choice, applied after the menu is drawn.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    MatchYZoom(ChannelId),
    YFit(ChannelId),
    XyFit(ChannelId),
    YFitAll,
    MatchHeight(ChannelId),
    ResetHeight(ChannelId),
    FitAll,
    ToggleAutoLayout,
    Hide(ChannelId),
    ShowAll,
    ToggleHidden(ChannelId),
    Color(ChannelId, Option<[u8; 3]>),
    ToggleNeutral,
    TraceWidth(crate::ui::waveform::TraceWidth),
    ResetNames,
    /// *Settings…* on the Waveform section.
    Settings,
    /// A Delete Data item.
    Trim(super::trim::TrimItem),
    /// Signal groups.
    GroupChannels(ChannelId),
    EditGroup(u16),
    BusFormat(u16, BusFormat),
    BusEndianness(u16, Endianness),
    BusSigned(u16),
    RemoveGroup(u16),
    WireStatus,
}

impl LaApp {
    // ---- label column ----

    /// The splitter on the label column's right edge. Resizing keeps the
    /// waveform under the pointer in place.
    pub(crate) fn label_splitter(&mut self, ui: &mut Ui, l: &Layout) {
        let x = l.rows.left() + l.label_w;
        let r = Rect::from_min_max(
            pos2(x - SPLITTER_W / 2.0, l.label_header.top()),
            pos2(x + SPLITTER_W / 2.0, l.rows.bottom()),
        );
        let resp = ui.interact(r, ui.make_persistent_id("label_splitter"), Sense::drag());
        resp.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, "Resize label column"));
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.dragged() && resp.drag_delta().x != 0.0 {
            let max = LABEL_MAX.min(l.graph.width() - 50.0).max(LABEL_MIN);
            let old = self.settings.label_width.clamp(LABEL_MIN, max);
            let new = (old + resp.drag_delta().x).clamp(LABEL_MIN, max);
            if new != old {
                self.settings.label_width = new;
                self.mark_settings_dirty();
                // The plot's left edge moves by `d`: shift the view with it.
                let now = self.now();
                if let Some(s) = self.session.as_mut() {
                    let v = s.view.state();
                    let shifted = v.with_time_at(v.left_time, -(new - old) as f64);
                    let v = ViewState { width_px: v.width_px - (new - old) as f64, ..shifted };
                    s.view.request(ViewRequest::Immediate(v), now);
                }
            }
        }
    }

    // ---- rename ----

    pub(crate) fn start_rename(&mut self, id: ChannelId) {
        let Some(row) = self.session.as_ref().and_then(|s| s.row(id)) else { return };
        self.ui.rename = Some(Rename { channel: id, text: row.name.clone(), focus: true });
    }

    /// The inline editor over the row's name area. Enter or blur commits,
    /// Escape reverts, Tab / Shift+Tab commit and move to the next /
    /// previous visible row.
    pub(crate) fn rename_editor(&mut self, ui: &mut Ui, name_rects: &[(ChannelId, Rect)]) {
        let Some(mut ed) = self.ui.rename.take() else { return };
        let Some(&(_, rect)) = name_rects.iter().find(|(c, _)| *c == ed.channel) else {
            // Scrolled away or hidden: commit.
            if let Some(s) = self.session.as_mut() {
                s.rename_row(ed.channel, &ed.text);
            }
            return;
        };
        let ident = self.session.as_ref().and_then(|s| s.row(ed.channel)).map(|r| r.ident.clone()).unwrap_or_default();
        let theme = ui.la_theme();
        let (tab, shift) = ui.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift));
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        let edit_rect = Rect::from_center_size(rect.center(), vec2(rect.width(), 20.0f32.min(rect.height())));
        let r = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut ed.text)
                .font(Text::Lead.font_id())
                .text_color(theme.color(Token::Ink1))
                .margin(egui::Margin::symmetric(2, 1))
                .lock_focus(true),
        );
        label_text_edit(&r, &ed.text, &format!("{ident} name"));
        if ed.focus {
            r.request_focus();
            ed.focus = false;
        }
        let done = r.lost_focus() || (r.has_focus() && tab);
        if !done {
            self.ui.rename = Some(ed);
            return;
        }
        if escape {
            return; // revert
        }
        if let Some(s) = self.session.as_mut() {
            s.rename_row(ed.channel, &ed.text);
        }
        if tab {
            let i = name_rects.iter().position(|(c, _)| *c == ed.channel).unwrap_or(0);
            let next = if shift { i.checked_sub(1) } else { Some(i + 1) };
            if let Some(&(id, _)) = next.and_then(|n| name_rects.get(n)) {
                // The new editor takes focus next frame.
                ui.memory_mut(|m| m.surrender_focus(r.id));
                self.start_rename(id);
            }
        }
    }

    /// *Fit All Rows to Window Height* for a row stack `available` tall.
    pub(crate) fn fit_rows_to(&mut self, available: f32, window_height: f32) {
        // Analyzer lanes come on top of the rows' own heights.
        let lanes = self.lane_map();
        let lane_h = |c: ChannelId| lanes.get(&c).map_or(0.0, |v| v.len() as f32 * super::bubbles::LANE_H);
        let Some(s) = self.session.as_mut() else { return };
        let rows: Vec<(ChannelId, FitRow)> = s
            .visible_rows()
            .map(|r| {
                let fit = match r.channel.kind {
                    ChannelKind::Digital | ChannelKind::Bus => {
                        FitRow::Digital { min: r.min_height(), lanes: lane_h(r.channel) }
                    }
                    ChannelKind::Analog => FitRow::Analog { min: r.min_height() },
                };
                (r.channel, fit)
            })
            .collect();
        let fits: Vec<FitRow> = rows.iter().map(|r| r.1).collect();
        for ((id, _), h) in rows.iter().zip(fit_rows(&fits, available, window_height)) {
            s.set_row_height(*id, h);
        }
    }

    /// re-fits when the viewport height or the visible rows change.
    pub(crate) fn auto_layout(&mut self, ui: &Ui, l: &Layout) {
        if !self.settings.waveform.auto_layout {
            self.ui.auto_layout_for = None;
            return;
        }
        let Some(s) = self.session.as_ref() else { return };
        let key = (l.rows.height(), s.visible_rows().map(|r| r.channel).collect::<Vec<_>>());
        if self.ui.auto_layout_for.as_ref() != Some(&key) {
            self.fit_rows_to(l.rows.height(), ui.ctx().content_rect().height());
            self.ui.auto_layout_for = Some(key);
        }
    }

    // ---- context menu ----

    /// The row menu for `context_row`. It is anchored on the plot (which
    /// never goes away) at the right-click position, so hiding the row it
    /// was opened on keeps it open.
    pub(crate) fn row_context_menu(&mut self, ui: &Ui, plot: &Response, l: &Layout) {
        let open_now = self.ui.open_row_menu_at.take();
        if let Some(p) = open_now {
            self.ui.row_menu_pos = p;
        }
        let Some(id) = self.ui.context_row else { return };
        let Some(s) = self.session.as_ref() else { return };
        let Some(row) = s.row(id) else { return };
        let theme = ui.la_theme();
        let has_data = s.has_data();
        let analog = id.kind == ChannelKind::Analog;
        let auto = self.settings.waveform.auto_layout;
        let neutral = !self.settings.waveform.color_waveforms;
        let trace_width = self.settings.waveform.trace_width;
        let any_hidden = s.enabled_rows().any(|r| r.hidden);
        let rows: Vec<(ChannelId, String, bool)> =
            s.enabled_rows().map(|r| (r.channel, format!("{}. {}", r.ident, r.name), !r.hidden)).collect();
        let own_color = row.color;
        // The group this row is, or is in.
        let bus = (id.is_bus()).then(|| s.bus(id.index).cloned()).flatten();
        let member = s.bus_of(id).is_some();
        let wire = (self.command_checked("view.wire_status") == Some(true), self.command_available("view.wire_status"));
        let palette: Vec<[u8; 3]> = (0..8)
            .map(|i| {
                let c = channel_color(&theme, i);
                [c.r(), c.g(), c.b()]
            })
            .collect();
        // #6: Delete Data at the right-click point (plot cell only).
        let trim_items = {
            let v = s.view.state();
            let at = self.ui.row_menu_pos.x - l.plot_rows.left();
            let in_plot = l.plot_rows.contains(self.ui.row_menu_pos) && !id.is_bus();
            let t = v.px_to_time(at as f64);
            let origin = s.origin.resolve(&s.markers, s.trigger);
            let point = format!("this point ({})", crate::core::format::time_full(t - origin));
            if in_plot { super::trim::point_items(t, &point, Some((v.left_time, v.right_time()))) } else { Vec::new() }
        };
        let mut act = None;
        let set = |act: &mut Option<Action>, a: Action| *act = Some(a);
        let cmd = if open_now.is_some() {
            Some(egui::SetOpenCommand::Bool(true))
        } else if plot.clicked() {
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            None
        };
        let popup = Popup::menu(plot)
            .id(ui.make_persistent_id("row_menu"))
            .at_position(self.ui.row_menu_pos)
            .open_memory(cmd)
            .frame(popover_frame(ui))
            .width(260.0);
        popup.show(|ui| {
            if analog {
                submenu(ui, "Zoom and Resize", 300.0, None, |ui| {
                    if menu_item(ui, "Match the Y-zoom across all analog channels", None).clicked() {
                        set(&mut act, Action::MatchYZoom(id));
                    }
                    ui.add_enabled_ui(has_data, |ui| {
                        if menu_item(ui, "Y Fit", Some("Y")).clicked() {
                            set(&mut act, Action::YFit(id));
                        }
                        if menu_item(ui, "XY Fit", None).clicked() {
                            set(&mut act, Action::XyFit(id));
                        }
                        if menu_item(ui, "Y Fit All Channels", Some("Shift+Y")).clicked() {
                            set(&mut act, Action::YFitAll);
                        }
                    });
                    menu_separator(ui);
                    if menu_item(ui, "Resize All Analog Channels to Match", None).clicked() {
                        set(&mut act, Action::MatchHeight(id));
                    }
                    if menu_item(ui, "Reset Channel Height", None).clicked() {
                        set(&mut act, Action::ResetHeight(id));
                    }
                    if menu_item(ui, "Fit all Rows to Window Height", None).clicked() {
                        set(&mut act, Action::FitAll);
                    }
                    if menu_check_item(ui, "Auto Layout", auto, None).clicked() {
                        set(&mut act, Action::ToggleAutoLayout);
                    }
                });
            } else {
                submenu(ui, "Resize", 300.0, None, |ui| {
                    if menu_item(ui, "Resize All Digital Channels to Match", None).clicked() {
                        set(&mut act, Action::MatchHeight(id));
                    }
                    if menu_item(ui, "Fit all Rows to Window Height", None).clicked() {
                        set(&mut act, Action::FitAll);
                    }
                    if menu_check_item(ui, "Auto Layout", auto, None).clicked() {
                        set(&mut act, Action::ToggleAutoLayout);
                    }
                });
                if !id.is_bus() && !member && menu_item(ui, "Group Channels…", None).clicked() {
                    set(&mut act, Action::GroupChannels(id));
                }
            }
            // #3: group rows.
            if let Some(b) = &bus {
                menu_separator(ui);
                if menu_item(ui, "Edit Group…", None).clicked() {
                    set(&mut act, Action::EditGroup(b.id));
                }
                submenu(ui, "Value Format", 220.0, None, |ui| {
                    for f in BusFormat::ALL {
                        if menu_check_item(ui, f.label(), b.format == f, None).clicked() {
                            set(&mut act, Action::BusFormat(b.id, f));
                        }
                    }
                    menu_separator(ui);
                    // A No Data group has no value to order or sign.
                    ui.add_enabled_ui(b.format.has_value(), |ui| {
                        for e in Endianness::ALL {
                            if menu_check_item(ui, e.label(), b.endianness == e, None).clicked() {
                                set(&mut act, Action::BusEndianness(b.id, e));
                            }
                        }
                        menu_separator(ui);
                        if menu_check_item(ui, "Signed", b.signed, None).clicked() {
                            set(&mut act, Action::BusSigned(b.id));
                        }
                    });
                });
                if menu_item(ui, "Remove Group", None).clicked() {
                    set(&mut act, Action::RemoveGroup(b.id));
                }
            }
            menu_separator(ui);
            if !trim_items.is_empty()
                && let Some(k) = self.delete_data_submenu(ui, &trim_items)
            {
                set(&mut act, Action::Trim(trim_items[k].clone()));
            }
            if menu_item(ui, if bus.is_some() { "Hide Group" } else { "Hide Channel" }, None).clicked() {
                set(&mut act, Action::Hide(id));
            }
            // Toggling keeps the submenu open.
            let keep_open = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
            submenu(ui, "Show/Hide Channels", 220.0, Some(keep_open), |ui| {
                ui.add_enabled_ui(any_hidden, |ui| {
                    if menu_item(ui, "Show All Channels", None).clicked() {
                        set(&mut act, Action::ShowAll);
                    }
                });
                menu_separator(ui);
                for (c, label, shown) in &rows {
                    if menu_check_item(ui, label, *shown, None).clicked() {
                        set(&mut act, Action::ToggleHidden(*c));
                    }
                }
            });
            submenu(ui, "Appearance", 220.0, None, |ui| {
                // The row's own color, and the neutral traces toggle.
                submenu(ui, "Channel Color", 180.0, None, |ui| {
                    if menu_check_item(ui, "Default", own_color.is_none(), None).clicked() {
                        set(&mut act, Action::Color(id, None));
                    }
                    for (name, rgb) in COLOR_NAMES.iter().zip(&palette) {
                        let r = menu_check_item(ui, name, own_color == Some(*rgb), None);
                        let sw =
                            Rect::from_center_size(pos2(r.rect.right() - 20.0, r.rect.center().y), vec2(12.0, 12.0));
                        ui.painter().rect_filled(sw, 2.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        if r.clicked() {
                            set(&mut act, Action::Color(id, Some(*rgb)));
                        }
                    }
                });
                if menu_check_item(ui, "Toggle Traces Color", neutral, None).clicked() {
                    set(&mut act, Action::ToggleNeutral);
                }
                submenu(ui, "Trace Width", 160.0, None, |ui| {
                    for w in crate::ui::waveform::TraceWidth::ALL {
                        if menu_check_item(ui, w.label(), trace_width == w, None).clicked() {
                            set(&mut act, Action::TraceWidth(w));
                        }
                    }
                });
            });
            // #9: digital and group rows.
            if !analog {
                let r = ui.add_enabled_ui(wire.1.is_ok(), |ui| menu_check_item(ui, "Wire Status", wire.0, None)).inner;
                let r = match wire.1 {
                    Err(why) => r.on_disabled_hover_text(why),
                    Ok(()) => r,
                };
                if r.clicked() {
                    set(&mut act, Action::WireStatus);
                }
            }
            menu_separator(ui);
            if menu_item(ui, "Reset Channel Names", None).clicked() {
                set(&mut act, Action::ResetNames);
            }
            if menu_item(ui, "Settings…", None).clicked() {
                set(&mut act, Action::Settings);
            }
        });
        if let Some(a) = act {
            self.apply_row_action(a, ui, l);
        }
    }

    fn apply_row_action(&mut self, a: Action, ui: &Ui, l: &Layout) {
        let window_h = ui.ctx().content_rect().height();
        match a {
            Action::YFit(id) => self.y_fit(id),
            Action::XyFit(id) => {
                let before = self.session.as_ref().and_then(|s| s.row(id)).and_then(|r| r.analog);
                self.y_fit(id);
                let after = self.session.as_ref().and_then(|s| s.row(id)).and_then(|r| r.analog);
                if let (Some(b), Some(a)) = (before, after) {
                    let w = self.session.as_ref().map_or(1.0, |s| s.view.state().width_px);
                    self.zoom_view(a.units_per_px / b.units_per_px, w / 2.0, JUMP_DURATION);
                }
            }
            Action::Trim(item) => self.request_trim(&item),
            Action::GroupChannels(id) => self.open_group_dialog(id),
            Action::WireStatus => self.dispatch("view.wire_status"),
            Action::EditGroup(b) => self.open_edit_group(b),
            Action::Settings => self.open_settings(super::settings::Section::Waveform),
            Action::YFitAll => self.y_fit_all(),
            Action::FitAll => self.fit_rows_to(l.rows.height(), window_h),
            Action::ToggleAutoLayout => {
                self.settings.waveform.auto_layout = !self.settings.waveform.auto_layout;
                self.ui.auto_layout_for = None;
                self.mark_settings_dirty();
            }
            Action::ToggleNeutral => {
                self.settings.waveform.color_waveforms = !self.settings.waveform.color_waveforms;
                self.mark_settings_dirty();
            }
            Action::TraceWidth(w) => {
                self.settings.waveform.trace_width = w;
                self.mark_settings_dirty();
            }
            _ => {
                let Some(s) = self.session.as_mut() else { return };
                match a {
                    Action::MatchYZoom(id) => {
                        let Some(src) = s.row(id).and_then(|r| r.analog) else { return };
                        let ids: Vec<ChannelId> = s
                            .rows
                            .iter()
                            .filter(|r| r.channel.kind == ChannelKind::Analog && r.channel != id)
                            .map(|r| r.channel)
                            .collect();
                        for other in ids {
                            if let Some(r) = s.row_mut(other) {
                                let h = r.analog_inner.unwrap_or(100.0) as f64;
                                let v = r.analog.unwrap_or(src);
                                r.analog = Some(
                                    crate::core::analog::AnalogView {
                                        units_per_px: src.units_per_px,
                                        center_value: src.center_value,
                                        ..v
                                    }
                                    .clamped(h),
                                );
                            }
                        }
                    }
                    Action::MatchHeight(id) => {
                        let Some(h) = s.row(id).map(|r| r.height) else { return };
                        let ids: Vec<ChannelId> =
                            s.rows.iter().filter(|r| r.channel.kind == id.kind).map(|r| r.channel).collect();
                        for other in ids {
                            s.set_row_height(other, h);
                        }
                        self.settings.waveform.auto_layout = false;
                    }
                    Action::ResetHeight(id) => {
                        let h = if id.kind == ChannelKind::Analog { ANALOG_ROW_HEIGHT } else { DIGITAL_ROW_HEIGHT };
                        s.set_row_height(id, h);
                        self.settings.waveform.auto_layout = false;
                    }
                    Action::Hide(id) => {
                        if let Some(r) = s.row_mut(id) {
                            r.hidden = true;
                        }
                    }
                    Action::ShowAll => {
                        for r in &mut s.rows {
                            r.hidden = false;
                        }
                    }
                    Action::ToggleHidden(id) => {
                        if let Some(r) = s.row_mut(id) {
                            r.hidden = !r.hidden;
                        }
                    }
                    Action::Color(id, c) => {
                        if let Some(r) = s.row_mut(id) {
                            r.color = c;
                        }
                    }
                    Action::ResetNames => s.reset_names(),
                    Action::BusFormat(b, f) => {
                        if let Some(b) = s.bus_mut(b) {
                            b.format = f;
                        }
                    }
                    Action::BusEndianness(b, e) => {
                        if let Some(b) = s.bus_mut(b) {
                            b.endianness = e;
                        }
                    }
                    Action::BusSigned(b) => {
                        if let Some(b) = s.bus_mut(b) {
                            b.signed = !b.signed;
                        }
                    }
                    Action::RemoveGroup(b) => s.remove_bus(b),
                    _ => {}
                }
                self.mark_settings_dirty();
            }
        }
    }

    // ---- reorder ----

    /// Ends a label drag: commits the drop position when released over the
    /// row stack, otherwise cancels.
    pub(crate) fn finish_reorder(&mut self, ui: &Ui, l: &Layout) {
        let Some(ro) = self.ui.reorder else { return };
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.ui.reorder = None;
            return;
        }
        if ui.input(|i| i.pointer.any_down()) {
            return;
        }
        self.ui.reorder = None;
        let inside = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| l.rows.contains(p));
        if ro.active
            && inside
            && let Some(s) = self.session.as_mut()
        {
            s.move_row(ro.channel, ro.drop_before);
        }
    }
}

/// A submenu entry drawn as a [`menu_item`] with a ▸, opening a
/// `width`-wide submenu.
pub(crate) fn submenu(ui: &mut Ui, text: &str, width: f32, config: Option<MenuConfig>, content: impl FnOnce(&mut Ui)) {
    submenu_impl(ui, text, None, width, config, content);
}

/// [`submenu`] with a check mark (*Timing Marker*).
pub(crate) fn submenu_checked(ui: &mut Ui, text: &str, checked: bool, width: f32, content: impl FnOnce(&mut Ui)) {
    submenu_impl(ui, text, Some(checked), width, None, content);
}

fn submenu_impl(
    ui: &mut Ui,
    text: &str,
    checked: Option<bool>,
    width: f32,
    config: Option<MenuConfig>,
    content: impl FnOnce(&mut Ui),
) {
    let theme = ui.la_theme();
    let resp = match checked {
        Some(c) => menu_check_item(ui, text, c, None),
        None => menu_item(ui, text, None),
    };
    let c = if resp.hovered() { theme.color(Token::InkOnFlare) } else { theme.color(Token::Ink3) };
    let tip = pos2(resp.rect.right() - 14.0, resp.rect.center().y);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip + vec2(-4.0, -4.0), tip + vec2(0.0, 0.0), tip + vec2(-4.0, 4.0)],
        c,
        egui::Stroke::NONE,
    ));
    let mut sub = SubMenu::new();
    if let Some(cfg) = config {
        sub = sub.config(cfg);
    }
    sub.show(ui, &resp, |ui| {
        ui.set_width(width);
        content(ui);
    });
}
