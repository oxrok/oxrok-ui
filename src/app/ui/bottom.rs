//! Bottom bar: Devices menu, the session button
//! and zoom controller.

use super::footer_button;
use crate::app::LaApp;
use crate::core::animator::{JUMP_DURATION, ViewRequest, WHEEL_ZOOM_DURATION};
use crate::core::format::{TimeUnit, parse_time, si};
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, menu_item, menu_separator, menu_title, popover_frame};
use egui::{
    Frame, Key, Layout, Panel, Popup, PopupCloseBehavior, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2,
};

/// Label: two decimals above 1 s, else SI with 0 decimals.
pub fn zoom_label(time_scale: f64) -> String {
    if time_scale > 1.0 { format!("{time_scale:.2} s") } else { si(time_scale, "s", Some(0)) }
}

impl LaApp {
    pub(crate) fn bottom_bar(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        Panel::bottom("bottom_bar")
            .exact_size(crate::app::BOTTOM_BAR_HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(theme.color(Token::Lift)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, theme.color(Token::Seam)));
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    ui.set_height(crate::app::BOTTOM_BAR_HEIGHT);
                    self.progress_minimized_bars(ui);
                    self.devices_menu(ui);
                    self.session_button(ui);
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        self.zoom_controller(ui);
                        self.notes_button(ui);
                    });
                });
            });
    }

    /// The minimized capture progress, two stacked bars at the left
    /// end of the bottom bar.
    fn progress_minimized_bars(&mut self, ui: &mut Ui) {
        if !self.ui.progress_minimized {
            return;
        }
        let Some(p) = self.progress() else { return };
        let theme = ui.la_theme();
        let (bars, _) = ui.allocate_exact_size(vec2(60.0, 8.0), Sense::hover());
        let progress = p.timer.map_or(0.0, |t| t.0);
        for (i, (frac, token)) in [(progress, Token::Good), (p.memory, Token::Caution)].into_iter().enumerate() {
            let b = Rect::from_min_size(bars.min + vec2(0.0, 4.0 * i as f32), vec2(bars.width(), 4.0));
            ui.painter().rect_filled(b, 0.0, theme.color(Token::Hair));
            let f = Rect::from_min_size(b.min, vec2(bars.width() * frac.clamp(0.0, 1.0), 4.0));
            ui.painter().rect_filled(f, 0.0, theme.color(token));
        }
        let restore =
            ui.interact(bars.expand2(vec2(0.0, 4.0)), ui.make_persistent_id("progress_restore"), Sense::click());
        restore.widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::l10n::tr("Show capture progress")));
        if restore.on_hover_text(crate::l10n::tr("Show capture progress")).clicked() {
            self.ui.progress_minimized = false;
        }
    }

    fn devices_menu(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let devices = self.devices.clone().unwrap_or_default();
        let physical = devices.iter().filter(|d| !d.is_demo).count();
        let label = match physical {
            0 => crate::l10n::tr("Disconnected").to_owned(),
            1 => crate::l10n::tr("1 Device").to_owned(),
            n => crate::l10n::tr("{} Devices").replacen("{}", &n.to_string(), 1),
        };
        let icon: Option<fn(&egui::Painter, Rect, egui::Color32)> =
            (physical > 0).then_some(icons::device as fn(&_, _, _));
        let resp = footer_button(ui, &label, false, icon);
        // New Session without a device opens this menu.
        if std::mem::take(&mut self.ui.open_devices_menu) {
            Popup::open_id(ui.ctx(), Popup::default_response_id(&resp));
        }
        let active = self.session.as_ref().map(|s| s.device.id.clone());
        let width_px = (ui.ctx().content_rect().width() - self.settings.label_width) as f64;
        let mut open = None;
        let mut help = false;
        let mut install = None;
        let missing = self.firmware.missing.first().cloned();
        // A warning dot on the button while a device needs firmware.
        if missing.is_some() {
            let c = pos2(resp.rect.right() - 8.0, resp.rect.top() + 8.0);
            ui.painter().circle_filled(c, 3.5, theme.color(Token::Caution));
        }
        self.ui.devices_menu_open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&resp));
        Popup::from_toggle_button_response(&resp)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .frame(popover_frame(ui))
            .width(340.0)
            .show(|ui| {
                menu_title(
                    ui,
                    if physical == 0 { crate::l10n::tr("Demo Devices") } else { crate::l10n::tr("Connected Devices") },
                );
                for d in &devices {
                    ui.horizontal(|ui| {
                        ui.set_height(56.0);
                        ui.add_space(12.0);
                        let (g, glyph) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                        icons::device(ui.painter(), g, theme.color(Token::Ink2));
                        if active.as_ref() == Some(&d.id) {
                            glyph.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Image,
                                    true,
                                    crate::l10n::tr("Active session's device"),
                                )
                            });
                            // Check badge: this device belongs to the active session.
                            let c = pos2(g.right() - 2.0, g.bottom() - 4.0);
                            ui.painter().circle_filled(c, 6.0, theme.color(Token::Mark));
                            crate::ui::icons::success(
                                ui.painter(),
                                Rect::from_center_size(c, vec2(10.0, 10.0)),
                                theme.color(Token::InkOnFlare),
                            );
                        }
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(self.device_label(d))
                                .text_style(Text::Prose.style())
                                .color(theme.color(Token::Ink2)),
                        );
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(12.0);
                            if ui.add(Button::tertiary(crate::l10n::tr("New Session"))).clicked() {
                                open = Some(d.clone());
                            }
                        });
                    });
                }
                // A device needs firmware.
                if let Some(m) = &missing {
                    crate::ui::widgets::menu_separator(ui);
                    ui.horizontal(|ui| {
                        ui.set_height(40.0);
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(crate::l10n::tr("A device needs firmware"))
                                .text_style(Text::Prose.style())
                                .color(theme.color(Token::Caution)),
                        );
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(12.0);
                            if ui.add(Button::tertiary(crate::l10n::tr("Install…"))).clicked() {
                                install = Some(m.file.clone());
                            }
                        });
                    });
                }
                // The last row.
                crate::ui::widgets::menu_separator(ui);
                if crate::ui::widgets::menu_item(ui, crate::l10n::tr("Device Connection Help ›"), None).clicked() {
                    help = true;
                }
            });
        if help {
            Popup::close_all(ui.ctx());
            self.open_connection_help();
        }
        if let Some(file) = install {
            Popup::close_all(ui.ctx());
            self.open_install_firmware(&file);
        }
        if let Some(d) = open {
            Popup::close_all(ui.ctx());
            self.open_session(&d, width_px);
        }
    }

    fn zoom_controller(&mut self, ui: &mut Ui) {
        let Some(scale) = self.session.as_ref().map(|s| s.view.state().time_scale()) else { return };
        let label = zoom_label(scale);
        let resp = footer_button(ui, &label, false, None);
        let popup_id = Popup::default_response_id(&resp);
        let was_open = Popup::is_id_open(ui.ctx(), popup_id);
        if resp.clicked() && !was_open {
            self.ui.zoom_text = label.clone();
        }
        let mut commit = None;
        let mut zoom = None;
        let mut recent = None;
        let recents = self.settings.history.recent_zooms.clone();
        Popup::from_toggle_button_response(&resp)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .frame(popover_frame(ui))
            .width(220.0)
            .show(|ui| {
                menu_title(ui, crate::l10n::tr("Time Range"));
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let edit = egui::TextEdit::singleline(&mut self.ui.zoom_text)
                        .desired_width(ui.available_width() - 12.0)
                        .font(Text::Prose.font_id());
                    let r = ui.add(edit);
                    super::label_text_edit(&r, &self.ui.zoom_text, crate::l10n::tr("Time Range"));
                    if !was_open {
                        r.request_focus();
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        commit = Some(self.ui.zoom_text.clone());
                    }
                });
                if !recents.is_empty() {
                    ui.add_space(4.0);
                    menu_title(ui, crate::l10n::tr("Recent Zooms"));
                    for z in &recents {
                        if menu_item(ui, &zoom_label(*z), None).clicked() {
                            recent = Some(*z);
                        }
                    }
                }
                menu_separator(ui);
                if menu_item(ui, crate::l10n::tr("Zoom In"), Some("=")).clicked() {
                    zoom = Some(0.5);
                }
                if menu_item(ui, crate::l10n::tr("Zoom Out"), Some("-")).clicked() {
                    zoom = Some(2.0);
                }
            });
        let width = self.session.as_ref().map_or(1.0, |s| s.view.state().width_px);
        if let Some(f) = zoom {
            self.zoom_view(f, width / 2.0, WHEEL_ZOOM_DURATION);
        }
        if let Some(text) = commit {
            // Invalid input silently reverts.
            if let Ok(t) = parse_time(&text, TimeUnit::S)
                && t > 0.0
            {
                self.set_time_scale(t);
                self.settings.add_recent_zoom(t);
                self.mark_settings_dirty();
            }
            Popup::close_id(ui.ctx(), popup_id);
        }
        if let Some(t) = recent {
            self.set_time_scale(t);
            self.settings.add_recent_zoom(t);
            self.mark_settings_dirty();
            Popup::close_id(ui.ctx(), popup_id);
        }
    }

    /// The Notes icon in the bottom bar; toggles the Notes
    /// bottom panel. A dot when the session has notes.
    fn notes_button(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_ref() else { return };
        let theme = ui.la_theme();
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, ui.available_height()), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::l10n::tr("Notes")));
        let open = self.ui.notes_panel_open;
        let p = ui.painter();
        if open {
            p.rect_filled(rect, 0.0, theme.color(Token::Mark));
        }
        let color = if open {
            theme.color(Token::InkOnFlare)
        } else if resp.hovered() {
            theme.color(Token::Ink1)
        } else {
            theme.color(Token::Ink3)
        };
        icons::notes(p, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), color);
        if !s.notes.trim().is_empty() {
            p.circle_filled(pos2(rect.right() - 8.0, rect.top() + 8.0), 3.0, theme.color(Token::Ink2));
        }
        if resp.on_hover_text(crate::l10n::tr("Notes (Alt+5)")).clicked() {
            self.toggle_notes_panel();
        }
    }

    /// The Notes bottom panel, above the bottom bar. Hidden
    /// without a session.
    pub(crate) fn notes_panel_bottom(&mut self, ui: &mut Ui) {
        if !self.ui.notes_panel_open || self.session.is_none() {
            return;
        }
        let theme = ui.la_theme();
        // ⅓ of the window height by default.
        let win_h = ui.ctx().input(|i| i.viewport().inner_rect).map_or(800.0, |r| r.height());
        let default_h = (win_h / 3.0).max(80.0);
        Panel::bottom("notes_panel")
            .resizable(true)
            .min_size(80.0)
            .default_size(default_h)
            .frame(Frame::new().fill(theme.color(Token::Deck)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, theme.color(Token::Seam)));
                // Header: title (with the tooltip) + a close button.
                ui.horizontal(|ui| {
                    ui.set_height(30.0);
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(crate::l10n::tr("Notes"))
                            .text_style(Text::Headline.style())
                            .color(theme.color(Token::Ink1)),
                    )
                    .on_hover_text(crate::l10n::tr("Free-form notes saved with the capture"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let (close_rect, close_resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                        close_resp
                            .widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::l10n::tr("Close Notes")));
                        let color = theme.color(if close_resp.hovered() { Token::Ink1 } else { Token::Ink3 });
                        icons::close(
                            ui.painter(),
                            Rect::from_center_size(close_rect.center(), vec2(14.0, 14.0)),
                            color,
                        );
                        if close_resp.on_hover_text(crate::l10n::tr("Close Notes")).clicked() {
                            self.ui.notes_panel_open = false;
                        }
                    });
                });
                // Content: the notes text area.
                self.notes_panel(ui);
            });
    }

    /// Animate the time scale about the center, within the limits.
    fn set_time_scale(&mut self, scale: f64) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let v = s.view.target();
        let mut factor = scale / v.time_scale();
        if let Some(b) = s.bounds() {
            factor = crate::core::nav::clamp_zoom_factor(&v, factor, &b, s.state.is_busy());
        }
        let fix_px = if s.follow && s.state.is_busy() {
            v.width_px * crate::core::nav::FOLLOW_FRACTION
        } else {
            v.width_px / 2.0
        };
        s.view.request(ViewRequest::Zoom { fix_px, factor, duration: JUMP_DURATION }, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_labels() {
        assert_eq!(zoom_label(12.5), "12.50 s");
        assert_eq!(zoom_label(0.016), "16 ms");
        assert_eq!(zoom_label(250e-6), "250 µs");
        assert_eq!(zoom_label(1.0), "1 s");
    }
}
