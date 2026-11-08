//! Modal dialogs: Device Information, Export and
//! confirmations.

use crate::app::LaApp;
use crate::backend::Rates;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::Button;
use egui::{Context, Frame, Id, Margin, Modal};

pub(crate) fn modal_frame(ctx: &Context) -> Frame {
    let theme = ctx.la_theme();
    Frame::new()
        .fill(theme.color(Token::Card))
        .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
        .corner_radius(4)
        .shadow(theme.box_shadow())
        .inner_margin(Margin::same(32))
}

/// What a confirmed dialog does.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmAction {
    DeleteAllMarkers,
    DeleteAllRanges,
    ResetAllSettings,
}

/// A standard confirmation dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Confirm {
    pub title: &'static str,
    pub body: String,
    pub confirm: &'static str,
    pub action: ConfirmAction,
}

impl Confirm {
    /// Says how many go; pairs count as two markers.
    pub fn delete_all_markers(n: usize) -> Self {
        Self {
            title: "Delete Timing Markers",
            body: format!("Delete all {n} timing {}?", if n == 1 { "marker" } else { "markers" }),
            confirm: "Delete All",
            action: ConfirmAction::DeleteAllMarkers,
        }
    }

    /// what's reset and what isn't.
    pub fn reset_all_settings() -> Self {
        Self {
            title: "Reset All Settings",
            body: "Reset all settings to defaults? Your capture files and probe profiles are not affected.".into(),
            confirm: "Reset",
            action: ConfirmAction::ResetAllSettings,
        }
    }

    pub fn delete_all_ranges(n: usize) -> Self {
        Self {
            title: "Delete Measurements",
            body: format!("Delete all {n} {}?", if n == 1 { "measurement" } else { "measurements" }),
            confirm: "Delete All",
            action: ConfirmAction::DeleteAllRanges,
        }
    }
}

impl LaApp {
    /// Blocking (the backdrop doesn't close it), Escape
    /// cancels, Enter confirms; the destructive button is rightmost.
    fn confirm_dialog(&mut self, ctx: &Context) {
        let Some(c) = self.ui.confirm.clone() else { return };
        let theme = ctx.la_theme();
        let mut result = None;
        Modal::new(Id::unique("confirm_dialog")).frame(modal_frame(ctx).inner_margin(Margin::ZERO)).show(ctx, |ui| {
            ui.set_width(420.0);
            Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                ui.label(
                    egui::RichText::new(c.title).text_style(Text::Display.style()).color(theme.color(Token::Ink1)),
                );
            });
            ui.separator();
            Frame::new().inner_margin(Margin::symmetric(32, 20)).show(ui, |ui| {
                ui.label(egui::RichText::new(&c.body).text_style(Text::Prose.style()).color(theme.color(Token::Ink2)));
            });
            ui.separator();
            Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Button::destructive(c.confirm)).clicked() {
                        result = Some(true);
                    }
                    ui.add_space(8.0);
                    if ui.add(Button::secondary("Cancel")).clicked() {
                        result = Some(false);
                    }
                });
            });
            let (esc, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
            if esc {
                result = Some(false);
            } else if enter {
                result = Some(true);
            }
        });
        let Some(ok) = result else { return };
        self.ui.confirm = None;
        if !ok {
            return;
        }
        match c.action {
            ConfirmAction::DeleteAllMarkers => {
                if let Some(s) = self.session.as_mut() {
                    s.markers.items.clear();
                    s.origin = s.origin.for_capture_start(false);
                }
            }
            ConfirmAction::ResetAllSettings => {
                self.settings.reset_all();
                crate::l10n::set(self.settings.language);
                self.ui.auto_layout_for = None;
                self.mark_settings_dirty();
            }
            ConfirmAction::DeleteAllRanges => {
                let removed = self.session.as_ref().map(|s| s.ranges.items.clone()).unwrap_or_default();
                if let Some(s) = self.session.as_mut() {
                    s.ranges.items.clear();
                }
                self.record_range_delete(removed);
            }
        }
    }

    pub(crate) fn dialogs_ui(&mut self, ctx: &Context) {
        // The Settings dialog first, so its confirmation draws on top.
        self.settings_dialog(ctx);
        self.confirm_dialog(ctx);
        self.analyzer_dialog_ui(ctx);
        if self.ui.device_info_open {
            self.device_info_dialog(ctx);
        }
        if self.ui.export.is_some() {
            self.export_dialog(ctx);
        }
        if std::mem::take(&mut self.ui.open_probe_editor) {
            self.open_probe_editor();
        }
        if self.ui.probe_editor.is_some() {
            self.probe_editor_ui(ctx);
        }
        if self.ui.shortcuts_open {
            self.shortcuts_dialog(ctx);
        }
        self.table_export_dialog(ctx);
        self.connection_help_dialog(ctx);
        self.bus_dialog(ctx);
        self.install_firmware_dialog(ctx);
        self.about_dialog(ctx);
        self.trim_dialog(ctx);
        self.import_dialog(ctx);
    }

    fn device_info_dialog(&mut self, ctx: &Context) {
        let Some(s) = self.session.as_ref() else {
            self.ui.device_info_open = false;
            return;
        };
        let theme = ctx.la_theme();
        let device = s.device.clone();
        let d = &device;
        let rates = s.rates();
        let rate_range = match (rates.last(), rates.first()) {
            (Some(lo), Some(hi)) => {
                format!("{} – {}", super::inspector::rate_label(*lo), super::inspector::rate_label(*hi))
            }
            _ => "unknown".into(),
        };
        let caps = s.caps.as_ref();
        let rows: Vec<(&str, String)> = vec![
            ("Driver", d.driver.clone()),
            ("Vendor", d.vendor.clone()),
            ("Model", d.model.clone()),
            ("Version", d.version.clone()),
            ("Serial / connection", d.serial_or_conn.clone()),
            ("Digital channels", d.digital_channels().count().to_string()),
            ("Analog channels", d.analog_channels().count().to_string()),
            (
                "Sample rates",
                if matches!(caps.map(|c| &c.samplerates), Some(Rates::Range { .. })) {
                    format!("{rate_range} (range)")
                } else {
                    rate_range
                },
            ),
            ("Hardware trigger", if caps.is_some_and(|c| c.hw_trigger.is_some()) { "Yes".into() } else { "No".into() }),
        ];
        let title = self.device_label(d);
        // The Name field: the nickname, or vendor + model.
        let (key, default_name) = (d.id.0.clone(), d.display_name());
        let has_nickname = self.settings.device_nicknames.contains_key(&key);
        let mut name = self.ui.device_name_edit.take().unwrap_or_else(|| self.settings.device_name(d));
        let mut commit = false;
        let mut use_default = false;
        let mut close = false;
        let resp = Modal::new(Id::unique("device_info")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(egui::RichText::new(&title).text_style(Text::Display.style()).color(theme.color(Token::Ink1)));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name").color(theme.color(Token::Ink3)));
                ui.add_space(12.0);
                let w = ui.available_width() - if has_nickname { 130.0 } else { 0.0 };
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(w));
                super::label_text_edit(&r, &name, "Device name");
                commit = r.lost_focus();
                if has_nickname && ui.add(Button::tertiary("Use default name")).clicked() {
                    use_default = true;
                }
            });
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Details").text_style(Text::Subhead.style()).color(theme.color(Token::Ink2)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Button::tertiary("Copy")).clicked() {
                        let text: String = rows.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
                        ui.ctx().copy_text(text);
                    }
                });
            });
            ui.add_space(8.0);
            egui::Grid::new("device_info_grid").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
                for (k, v) in &rows {
                    ui.label(egui::RichText::new(*k).color(theme.color(Token::Ink3)));
                    ui.label(egui::RichText::new(v).color(theme.color(Token::Ink2)));
                    ui.end_row();
                }
            });
            ui.add_space(24.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(Button::primary("Close")).clicked() {
                    close = true;
                }
            });
        });
        // Enter or blur commits; an empty or default name clears the nickname.
        if use_default {
            name = default_name.clone();
            commit = true;
        }
        if commit || close || resp.should_close() {
            let trimmed = name.trim();
            let before = self.settings.device_nicknames.get(&key).cloned();
            if trimmed.is_empty() || trimmed == default_name {
                self.settings.device_nicknames.remove(&key);
            } else {
                self.settings.device_nicknames.insert(key.clone(), trimmed.to_owned());
            }
            if self.settings.device_nicknames.get(&key) != before.as_ref() {
                self.mark_settings_dirty();
            }
            name = self.settings.device_nicknames.get(&key).cloned().unwrap_or_else(|| default_name.clone());
        }
        if close || resp.should_close() {
            self.ui.device_info_open = false;
        } else {
            self.ui.device_name_edit = Some(name);
        }
    }
}
