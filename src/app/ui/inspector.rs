//! The Inspector: a left-docked panel, or a window of its
//! own, with a tab strip over the selected tab's content, and the Device
//! tab (05).

use super::{hint, section_title};
use crate::app::LaApp;
use crate::core::capture::{BUFFER_MB_MIN, BUFFER_MB_STEP, CaptureMode, TIMER_SECONDS_MIN};
use crate::core::format::si;
use crate::core::{ChannelId, ChannelKind};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{BinarySwitch, Dropdown, GridResponse, IconFn, channel_grid_section, stepper, time_input};
use egui::{Frame, Margin, Panel, Rect, Role, ScrollArea, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

/// Panel header height.
const PANEL_HEADER_H: f32 = 76.0;
/// Tab strip segment height.
const TAB_H: f32 = 40.0;
/// Width of the Detach / Attach button at the strip's right end.
const DETACH_W: f32 = 28.0;
/// The detached window's default and minimum heights.
const WINDOW_H: f32 = 700.0;
const WINDOW_MIN_H: f32 = 300.0;
pub(crate) const DEVICE_PANEL: &str = "device";
pub(crate) const NOTES_PANEL: &str = "notes";

/// Rates with up to 4 decimals, trailing zeros stripped.
pub fn rate_label(rate: u64) -> String {
    si(rate as f64, "S/s", Some(4))
}

/// `3 GB`, `2.5 GB`.
pub fn buffer_label(mb: u32) -> String {
    let gb = mb as f64 / 1024.0;
    let s = format!("{gb:.2}");
    format!("{} GB", s.trim_end_matches('0').trim_end_matches('.'))
}

impl LaApp {
    /// The Inspector, a tab strip over the selected tab's
    /// content, docked on the left or in its own window. Hidden when
    /// `inspector.open` is false or there is no session (the start screen,
    pub(crate) fn inspector(&mut self, ui: &mut Ui) {
        if !self.settings.inspector.open || self.session.is_none() {
            self.ui.inspector_window = None;
            return;
        }
        if self.settings.inspector.detached {
            self.inspector_window(ui.ctx());
            return;
        }
        self.ui.inspector_window = None;
        let resp = Panel::left("inspector")
            .resizable(true)
            .min_size(crate::app::settings::INSPECTOR_MIN_WIDTH)
            .default_size(self.settings.inspector.width)
            .frame(Frame::new().fill(ui.la_theme().color(Token::Deck)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                // Docked (it pushes the graph), so a 1 pt right border, no shadow.
                ui.painter().line_segment(
                    [r.right_top(), r.right_bottom()],
                    Stroke::new(1.0, ui.la_theme().color(Token::Seam)),
                );
                self.inspector_body(ui);
            });
        let w = resp.response.rect.width();
        if (w - self.settings.inspector.width).abs() > 0.5 {
            self.settings.inspector.width = w;
            self.mark_settings_dirty();
        }
    }

    /// The tab strip and the selected tab, wherever the Inspector is.
    fn inspector_body(&mut self, ui: &mut Ui) {
        self.ui.in_inspector = true;
        // Notes is no longer a tab; a persisted `"notes"` falls
        // back to Device.
        if self.settings.inspector.tab == NOTES_PANEL {
            self.settings.inspector.tab = DEVICE_PANEL.into();
            self.mark_settings_dirty();
        }
        self.inspector_tab_strip(ui, true);
        let tab = self.inspector_tab().to_owned();
        self.inspector_content(ui, &tab);
        self.ui.in_inspector = false;
    }

    /// Whether a dialog opened from the Inspector goes in a window
    /// of its own, next to the detached Inspector.
    pub(crate) fn dialog_beside_inspector(&self, from_inspector: bool) -> bool {
        from_inspector && self.settings.inspector.detached && self.settings.inspector.open && self.session.is_some()
    }

    /// A fixed builder for a dialog window of `size` points, centered
    /// on the detached Inspector's last known position.
    pub(crate) fn dialog_window_builder(&self, title: &str, size: egui::Vec2) -> egui::ViewportBuilder {
        let b = egui::ViewportBuilder::default().with_title(title).with_inner_size(size).with_resizable(false);
        match self.settings.inspector.window {
            Some(r) => b.with_position([r.x + (r.w - size.x) / 2.0, r.y + 80.0]),
            None => b,
        }
    }

    /// The detached Inspector, in a native window when the
    /// platform has them, else floating inside the main one.
    fn inspector_window(&mut self, ctx: &egui::Context) {
        let builder = self.ui.inspector_window.get_or_insert_with(|| {
            let s = &self.settings.inspector;
            let b = egui::ViewportBuilder::default()
                .with_title(tr("Inspector"))
                .with_min_inner_size([crate::app::settings::INSPECTOR_MIN_WIDTH, WINDOW_MIN_H]);
            match s.window {
                Some(r) => b.with_position([r.x, r.y]).with_inner_size([r.w, r.h]),
                None => b.with_inner_size([s.width, WINDOW_H]),
            }
        });
        let builder = builder.clone();
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("inspector"), builder, |ui, class| {
            if class == egui::ViewportClass::EmbeddedWindow {
                self.inspector_body(ui);
                return;
            }
            egui::CentralPanel::default().frame(Frame::new().fill(ui.la_theme().color(Token::Deck))).show(ui, |ui| {
                self.inspector_body(ui);
            });
            let ctx = ui.ctx().clone();
            // The main window's keys, and its Cmd+Q / Cmd+M.
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
            if cfg!(target_os = "macos") && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::M)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            self.handle_command_keys(&ctx);
            let (close, outer, inner) = ctx.input(|i| {
                let v = i.viewport();
                (v.close_requested(), v.outer_rect, v.inner_rect)
            });
            if close {
                // Closing the window attaches the Inspector, hidden.
                self.settings.inspector.detached = false;
                self.settings.inspector.open = false;
                self.mark_settings_dirty();
                return;
            }
            if let (Some(o), Some(i)) = (outer, inner) {
                let r = crate::app::settings::WindowRect { x: o.left(), y: o.top(), w: i.width(), h: i.height() };
                let moved = self.settings.inspector.window.is_none_or(|w| {
                    (w.x - r.x).abs() > 0.5
                        || (w.y - r.y).abs() > 0.5
                        || (w.w - r.w).abs() > 0.5
                        || (w.h - r.h).abs() > 0.5
                });
                if moved {
                    self.settings.inspector.window = Some(r);
                    self.mark_settings_dirty();
                }
            }
        });
    }

    /// Whether the active session has a device to show: an
    /// imported file has none, so no Device tab.
    fn has_device_tab(&self) -> bool {
        !self.session.as_ref().is_some_and(|s| s.is_import())
    }

    /// The tab shown: the chosen one, or Analyzers in place of a
    /// hidden Device tab. The choice is kept for the next device session.
    pub(crate) fn inspector_tab(&self) -> &str {
        let tab = self.settings.inspector.tab.as_str();
        if tab == DEVICE_PANEL && !self.has_device_tab() { super::analyzers::ANALYZERS_PANEL } else { tab }
    }

    /// The Inspector's tabs, in tab order, with their Alt+n number.
    /// Each has an icon, a short label (drawn in the strip) and a full
    /// label (accessibility).
    fn inspector_tabs(&self) -> Vec<(usize, &'static str, &'static str, &'static str, IconFn)> {
        let all: [(&'static str, &'static str, &'static str, IconFn); 4] = [
            (DEVICE_PANEL, "Device", "Device", icons::device),
            (super::analyzers::ANALYZERS_PANEL, "Analyzers", "Analyze", icons::loupe),
            (super::ranges::RANGES_PANEL, "Measurements", "Measure", icons::tape_measure),
            (super::markers::MARKERS_PANEL, "Markers", "Markers", icons::flag),
        ];
        let device = self.has_device_tab();
        all.into_iter()
            .enumerate()
            .filter(|(_, t)| device || t.0 != DEVICE_PANEL)
            .map(|(i, (id, full, short, icon))| (i + 1, id, full, short, icon))
            .collect()
    }

    /// The tab strip, a single row of equal-width icon+label segments.
    fn inspector_tab_strip(&mut self, ui: &mut Ui, has_session: bool) {
        let theme = ui.la_theme();
        let tabs = self.inspector_tabs();
        let n = tabs.len();
        let font = egui::FontId::new(9.5, Text::Micro.font_id().family);
        let strip_w = ui.available_width();
        let (strip, _) = ui.allocate_exact_size(vec2(strip_w, TAB_H), Sense::hover());
        let seg_w = (strip_w - DETACH_W) / n as f32;
        let shown = self.inspector_tab().to_owned();
        for (i, (key, id, full, short, icon)) in tabs.iter().enumerate() {
            let x = strip.left() + i as f32 * seg_w;
            let rect = Rect::from_min_size(pos2(x, strip.top()), vec2(seg_w, TAB_H));
            let selected = has_session && shown == *id;
            let resp = ui.interact(
                rect,
                ui.make_persistent_id(("inspector_tab", *id)),
                if has_session { Sense::click() } else { Sense::hover() },
            );
            resp.widget_info(|| WidgetInfo::selected(Role::Button, has_session, selected, tr(full)));
            let resp = resp.on_hover_text(format!("{} (Alt+{key})", tr(full)));
            if selected {
                ui.painter().rect_filled(rect, 4.0, theme.color(Token::Flare));
            }
            let color = if selected {
                theme.color(Token::InkOnFlare)
            } else if resp.hovered() {
                theme.color(Token::Ink1)
            } else {
                theme.color(Token::Ink3)
            };
            let c = color.gamma_multiply(if has_session { 1.0 } else { 0.4 });
            // Icon above the short label, both centered.
            let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 13.0), vec2(15.0, 15.0));
            icon(ui.painter(), icon_rect, c);
            ui.painter().text(
                pos2(rect.center().x, rect.center().y + 7.0),
                egui::Align2::CENTER_CENTER,
                tr(short),
                font.clone(),
                c,
            );
            if resp.clicked() && has_session && !selected {
                self.settings.inspector.tab = (*id).to_owned();
                self.mark_settings_dirty();
            }
        }
        self.detach_button(ui, Rect::from_min_max(pos2(strip.right() - DETACH_W, strip.top()), strip.max));
    }

    /// The Detach / Attach button at the strip's right end.
    fn detach_button(&mut self, ui: &mut Ui, rect: Rect) {
        let theme = ui.la_theme();
        let detached = self.settings.inspector.detached;
        let label = tr(if detached { "Attach Inspector" } else { "Detach Inspector" });
        let resp = ui.interact(rect, ui.make_persistent_id("inspector_detach"), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
        let resp = resp.on_hover_text(format!("{label} (Alt+Shift+I)"));
        let color = theme.color(if resp.hovered() { Token::Ink1 } else { Token::Ink3 });
        let icon_rect = Rect::from_center_size(rect.center(), vec2(15.0, 15.0));
        if detached {
            icons::dock(ui.painter(), icon_rect, color);
        } else {
            icons::pop_out(ui.painter(), icon_rect, color);
        }
        if resp.clicked() {
            self.set_inspector_detached(!detached);
        }
    }

    /// The selected tab's header + content.
    fn inspector_content(&mut self, ui: &mut Ui, tab: &str) {
        let theme = ui.la_theme();
        let (title, help) = match tab {
            DEVICE_PANEL => (tr("Device Settings"), None),
            super::analyzers::ANALYZERS_PANEL => (tr("Analyzers"), None),
            super::ranges::RANGES_PANEL => (
                tr("Measurements"),
                Some(tr("Measure a time range on a channel: drag across it with the measurement tool or Shift.")),
            ),
            super::markers::MARKERS_PANEL => {
                (tr("Timing Markers"), Some(tr("Mark points in time; pairs show the time between their two markers.")))
            }
            _ => return,
        };
        // The header scrolls with the content. Each tab remembers
        // its own scroll offset.
        ScrollArea::vertical().id_salt(tab).auto_shrink([false, false]).show(ui, |ui| {
            let header = ui
                .horizontal(|ui| {
                    ui.set_height(PANEL_HEADER_H);
                    ui.add_space(16.0);
                    if tab == DEVICE_PANEL {
                        self.device_header(ui);
                    } else {
                        ui.label(
                            egui::RichText::new(title)
                                .text_style(Text::Display.style())
                                .color(theme.color(Token::Ink1)),
                        );
                    }
                    if let Some(h) = help {
                        ui.add_space(4.0);
                        super::panels::help_icon(ui, h);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        match tab {
                            super::markers::MARKERS_PANEL => self.markers_panel_actions(ui),
                            super::analyzers::ANALYZERS_PANEL => self.analyzers_panel_actions(ui),
                            super::ranges::RANGES_PANEL => self.ranges_panel_actions(ui),
                            _ => {}
                        }
                    });
                })
                .response
                .rect;
            ui.painter().line_segment(
                [header.left_bottom(), header.right_bottom()],
                Stroke::new(1.0, theme.color(Token::Seam)),
            );
            match tab {
                DEVICE_PANEL => {
                    Frame::new().inner_margin(Margin::symmetric(16, 20)).show(ui, |ui| self.device_panel(ui));
                }
                super::markers::MARKERS_PANEL => self.markers_panel(ui),
                super::analyzers::ANALYZERS_PANEL => {
                    self.analyzers_list(ui);
                    self.analyzer_trigger_card(ui);
                    self.data_section(ui);
                }
                super::ranges::RANGES_PANEL => self.ranges_panel(ui),
                _ => {}
            }
        });
    }

    /// A borderless text area filling the panel, scrolling when
    /// the notes are longer than the visible area.
    pub(crate) fn notes_panel(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_mut() else { return };
        Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let rows = s.notes.lines().count().max(1);
                let r = ui.add(
                    egui::TextEdit::multiline(&mut s.notes)
                        .frame(Frame::NONE)
                        .font(egui::FontId::new(14.0, Text::Prose.font_id().family))
                        .hint_text(tr("Click here to write a note about this capture"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(rows),
                );
                super::label_text_edit(&r, &s.notes, tr("Capture notes"));
            });
        });
    }

    /// The Device Settings panel's header: the glyph, the
    /// name on one line (ellipsized) and ⓘ right after it.
    fn device_header(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        let device_label = self.device_label(&s.device);
        let ink = theme.color(Token::Ink1);
        let (g, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        icons::device(ui.painter(), g, ink);
        ui.add_space(12.0);
        let max = (ui.available_width() - 16.0 - 26.0).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(device_label.clone(), Text::Display.font_id(), ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max);
        let galley = ui.painter().layout_job(job);
        let elided = galley.elided;
        let (rect, name) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, galley.size().y), Sense::hover());
        name.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &device_label));
        if elided {
            name.on_hover_text(&device_label);
        }
        let icon =
            Rect::from_center_size(egui::pos2(rect.left() + galley.size().x + 14.0, rect.center().y), vec2(20.0, 20.0));
        ui.painter().galley(rect.left_top(), galley, ink);
        let resp = ui.interact(icon, ui.make_persistent_id("device_info_icon"), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Device Information")));
        let c = if resp.hovered() { Token::Ink1 } else { Token::Ink3 };
        icons::info(ui.painter(), icon.shrink(2.0), theme.color(c));
        if resp.on_hover_text(tr("Device Information")).clicked() {
            self.ui.device_info_open = true;
        }
    }

    fn device_panel(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let s = self.session.as_ref().unwrap();
        if let Some(err) = &s.open_error {
            ui.label(egui::RichText::new(err).color(theme.color(Token::Alert)));
            return;
        }
        // A loaded file shows its settings, read-only.
        let read_only = s.is_read_only();
        if s.caps.is_none() && !read_only {
            hint(ui, tr("Opening device…"));
            return;
        }
        let busy = s.state.is_busy() || read_only;
        // Chip sections follow the probe profile's groups.
        let (digital, digital_order) = super::probe::chip_groups(&theme, s, ChannelKind::Digital);
        let (analog, analog_order) = super::probe::chip_groups(&theme, s, ChannelKind::Analog);
        let digital_order: Vec<ChannelId> = digital_order.iter().map(|r| r.channel).collect();
        let analog_order: Vec<ChannelId> = analog_order.iter().map(|r| r.channel).collect();
        let rates = if read_only { vec![s.settings.sample_rate] } else { s.rates() };
        let rate_labels: Vec<String> = rates.iter().map(|&r| rate_label(r)).collect();
        let mut rate_idx = rates.iter().position(|&r| r == s.settings.sample_rate).unwrap_or(0);
        let rate_text = rate_label(s.settings.sample_rate);
        let buffer_mb = s.settings.buffer_mb;
        let mode_idx = match s.settings.mode {
            CaptureMode::Triggered => 0,
            CaptureMode::Timed => 1,
            CaptureMode::Continuous => 2,
        };
        let mut timer = s.settings.timer_seconds;
        let mut rle = s.settings.rle;
        // 14 on a copy, written back if edited.
        let mut trig = s.settings.trigger.clone();
        let digital_items: Vec<super::capture_opts::ChannelItem> = s
            .rows
            .iter()
            .filter(|r| r.channel.kind == ChannelKind::Digital && s.settings.is_enabled(r.channel))
            .map(|r| (r.channel, format!("{:02}.  {}", r.channel.index, r.display())))
            .collect();
        let mut conditions_open = self.ui.trigger_conditions_open;
        let streams = s.caps.as_ref().is_some_and(|c| c.supports_continuous);
        let has_rle = s.caps.as_ref().is_some_and(|c| c.rle);
        let thresholds = s.caps.as_ref().and_then(|c| c.thresholds.clone());
        let threshold = s.settings.logic_threshold.clone();

        // The whole area is disabled while capturing.
        let area = ui.add_enabled_ui(!busy, |ui| {
            let mut out = PanelEdits::default();
            // The mode itself is picked in the toolbar.
            if mode_idx == 1 {
                ui.add_space(12.0);
                hint(ui, tr("Stop recording after"));
                ui.add_space(6.0);
                out.timer_changed = time_input(ui, tr("Stop recording after"), &mut timer, TIMER_SECONDS_MIN);
            }
            if mode_idx == 0 {
                ui.add_space(14.0);
                out.trigger_changed =
                    super::capture_opts::trigger_editor(ui, &mut trig, &digital_items, &mut conditions_open, !streams);
            }
            // Continuous has no settings of its own.
            if mode_idx != 2 {
                ui.add_space(20.0);
                divider(ui);
            }
            if !digital_order.is_empty() {
                out.digital = channel_grid_section(ui, tr("Digital"), digital, &[tr("Probe Colors…")]);
                ui.add_space(12.0);
                out.rate_changed = ui.add(Dropdown::new(tr("Sample rate"), &mut rate_idx, &rate_labels)).changed();
                out.threshold = super::device_opts::threshold_ui(ui, thresholds.as_ref(), threshold.as_ref());
                ui.add_space(20.0);
            }
            if !analog_order.is_empty() {
                out.analog = channel_grid_section(ui, tr("Analog"), analog, &[tr("Probe Colors…")]);
                ui.add_space(12.0);
                // Libsigrok shares one rate; show it read-only here.
                hint(ui, &tr("{} (shared with digital)").replacen("{}", &rate_text, 1));
                ui.add_space(20.0);
            }
            if has_rle {
                divider(ui);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("Compression (RLE)")).color(theme.color(Token::Ink2)))
                        .on_hover_text(tr("The device compresses idle stretches in its own memory, so captures can last much longer. Busy signals still fill the memory sooner."));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        out.rle_changed = ui.add(BinarySwitch::new(&mut rle, tr("Compression (RLE)"))).changed();
                    });
                });
                ui.add_space(20.0);
            }
            divider(ui);
            section_title(ui, tr("Memory buffer size"));
            ui.add_space(8.0);
            out.step = stepper(ui, tr("Memory buffer size"), &buffer_label(buffer_mb), buffer_mb > BUFFER_MB_MIN, true);
            ui.add_space(20.0);
            out
        });
        let edits = area.inner;
        if busy
            && ui.interact(area.response.rect, ui.make_persistent_id("device_area_blocker"), Sense::click()).clicked()
        {
            let msg = if read_only {
                tr(
                    "These are the settings the capture was recorded with. Press Record to capture with them; the banner chooses where.",
                )
            } else {
                tr("Settings lock while recording.")
            };
            self.toast(Toast::new(ToastKind::Info, msg).id("settings-disabled"));
        }

        // Probe Colors… in either header.
        if edits.digital.extra.is_some() || edits.analog.extra.is_some() {
            self.ui.open_probe_editor = true;
        }
        let s = self.session.as_mut().unwrap();
        let mut changed = false;
        for (grid, ids, kind) in [
            (&edits.digital, &digital_order, ChannelKind::Digital),
            (&edits.analog, &analog_order, ChannelKind::Analog),
        ] {
            if let Some(i) = grid.toggled {
                let on = !s.settings.is_enabled(ids[i]);
                s.settings.set_enabled(ids[i], on);
                changed = true;
            }
            if grid.all {
                for id in ids.iter() {
                    s.settings.set_enabled(*id, true);
                }
                changed = true;
            }
            if grid.clear {
                s.settings.enabled.retain(|c| c.kind != kind);
                changed = true;
            }
        }
        if edits.rate_changed
            && let Some(&r) = rates.get(rate_idx)
        {
            s.settings.sample_rate = r;
            changed = true;
        }
        if edits.trigger_changed {
            s.settings.trigger = trig;
            changed = true;
        }
        // The trigger follows the enabled channels; with
        // no digital channel left, Trigger mode falls back.
        let enabled = s.settings.enabled.clone();
        let before = s.settings.trigger.clone();
        s.settings.trigger.follow_enables(&enabled);
        changed |= s.settings.trigger != before;
        let any_digital = enabled.iter().any(|c| c.kind == ChannelKind::Digital);
        if s.settings.mode == CaptureMode::Triggered && !any_digital {
            let streams = s.caps.as_ref().is_some_and(|c| c.supports_continuous);
            s.settings.mode = if streams { CaptureMode::Continuous } else { CaptureMode::Timed };
            changed = true;
        }
        self.ui.trigger_conditions_open = conditions_open;
        let s = self.session.as_mut().unwrap();
        if edits.rle_changed {
            s.settings.rle = rle;
            changed = true;
        }
        if edits.timer_changed {
            s.settings.timer_seconds = timer;
            changed = true;
        }
        if edits.step != 0 {
            let mb = s.settings.buffer_mb as i64 + edits.step as i64 * BUFFER_MB_STEP as i64;
            s.settings.buffer_mb = mb.max(BUFFER_MB_MIN as i64) as u32;
            changed = true;
        }
        if changed {
            self.mark_settings_dirty();
        }
        if let Some(t) = edits.threshold {
            self.set_threshold(t);
        }
        self.device_options_section(ui, busy);
        self.command_line_section(ui);
    }
}

#[derive(Default)]
struct PanelEdits {
    digital: GridResponse,
    analog: GridResponse,
    rate_changed: bool,
    timer_changed: bool,
    rle_changed: bool,
    trigger_changed: bool,
    threshold: Option<crate::backend::ThresholdChoice>,
    step: i32,
}

pub(super) fn divider(ui: &mut Ui) {
    let theme = ui.la_theme();
    let (div, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(div, 0.0, theme.color(Token::Hair));
    ui.add_space(20.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(rate_label(1_562_500), "1.5625 MS/s");
        assert_eq!(rate_label(500_000_000), "500 MS/s");
        assert_eq!(rate_label(10_000), "10 kS/s");
        assert_eq!(buffer_label(4096), "4 GB");
        assert_eq!(buffer_label(2560), "2.5 GB");
        assert_eq!(buffer_label(512), "0.5 GB");
    }
}
