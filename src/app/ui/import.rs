//! Import Options dialog: one row per libsigrok input-module
//! option, typed by the module's default, with validation, Reset, Cancel
//! and Import; a Format dropdown when the file's format isn't certain.

use super::dialogs::modal_frame;
use crate::app::LaApp;
use crate::backend::{ImportFormat, InputOption, InputOptionKind, OptionValue};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{BinarySwitch, Button, Dropdown};
use egui::{Context, Frame, Id, Margin, Modal, vec2};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(crate) struct ImportDialog {
    pub path: PathBuf,
    /// Index into the app's input formats.
    pub format: usize,
    /// The format wasn't detected, so the user picks it.
    pub choose_format: bool,
    /// Option values as typed, by option id.
    pub values: BTreeMap<String, String>,
}

/// An option value as the text field shows it.
fn text_of(v: &OptionValue) -> String {
    match v {
        OptionValue::Int(i) => i.to_string(),
        OptionValue::Float(f) => f.to_string(),
        OptionValue::Str(s) => s.clone(),
    }
}

/// Parses a typed value for `o`; `None` when invalid (validation).
pub(crate) fn parse_value(o: &InputOption, text: &str) -> Option<OptionValue> {
    let t = text.trim();
    match o.kind {
        InputOptionKind::Int | InputOptionKind::Bool => {
            // Sample rates accept SI suffixes ("1M", "500k").
            let v = if o.id == "samplerate" { parse_rate(t)? } else { t.parse::<i64>().ok()? };
            (v >= 0 && !(o.required() && v == 0)).then_some(OptionValue::Int(v))
        }
        InputOptionKind::Float => t.parse::<f64>().ok().map(OptionValue::Float),
        InputOptionKind::Str => Some(OptionValue::Str(t.to_owned())),
    }
}

/// `1000000`, `1M`, `1 MHz`, `500k` → Hz.
fn parse_rate(t: &str) -> Option<i64> {
    let t = t.trim().trim_end_matches("Hz").trim_end_matches("hz").trim();
    let (num, mult) = match t.chars().last()? {
        'k' | 'K' => (&t[..t.len() - 1], 1e3),
        'M' => (&t[..t.len() - 1], 1e6),
        'G' => (&t[..t.len() - 1], 1e9),
        _ => (t, 1.0),
    };
    let v = num.trim().parse::<f64>().ok()? * mult;
    (v.is_finite() && v >= 0.0).then_some(v.round() as i64)
}

impl LaApp {
    /// Opens the dialog for `path` with `module` selected.
    pub(crate) fn open_import_dialog(&mut self, path: PathBuf, module: &str, choose_format: bool) {
        let Some(format) = self.input_formats.iter().position(|f| f.id == module) else { return };
        let mut d = ImportDialog { path, format, choose_format, values: BTreeMap::new() };
        self.fill_import_values(&mut d);
        self.ui.import = Some(d);
    }

    /// The module's defaults, overridden by the last values used.
    fn fill_import_values(&self, d: &mut ImportDialog) {
        let f = &self.input_formats[d.format];
        let saved = self.settings.import_options.get(&f.id);
        d.values = f
            .options
            .iter()
            .map(|o| {
                let v = saved.and_then(|s| s.get(&o.id).cloned()).unwrap_or_else(|| text_of(&o.default));
                (o.id.clone(), v)
            })
            .collect();
    }

    pub(crate) fn import_dialog(&mut self, ctx: &Context) {
        let Some(d) = self.ui.import.as_mut() else { return };
        let theme = ctx.la_theme();
        let formats = self.input_formats.clone();
        let name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut format_idx = d.format;
        // libsigrok's descriptions read better than its short names.
        let format_labels: Vec<String> = formats.iter().map(|f| f.description.clone()).collect();
        let options = formats[d.format].options.clone();
        let invalid: Vec<String> =
            options.iter().filter(|o| parse_value(o, &d.values[&o.id]).is_none()).map(|o| o.id.clone()).collect();
        let (mut import, mut cancel, mut reset) = (false, false, false);
        let resp = Modal::new(Id::unique("import_options")).frame(modal_frame(ctx).inner_margin(Margin::ZERO)).show(
            ctx,
            |ui| {
                ui.set_width(520.0);
                Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(tr("Import {}").replacen("{}", &name, 1))
                            .text_style(Text::Display.style())
                            .color(theme.color(Token::Ink1)),
                    );
                });
                ui.separator();
                Frame::new().inner_margin(Margin::symmetric(32, 20)).show(ui, |ui| {
                    egui::Grid::new("import_options_grid").num_columns(2).spacing([24.0, 14.0]).show(ui, |ui| {
                        let label = |ui: &mut egui::Ui, text: &str, help: &str| {
                            let r = ui.label(egui::RichText::new(text).color(theme.color(Token::Ink3)));
                            if !help.is_empty() {
                                r.on_hover_text(help);
                            }
                        };
                        if d.choose_format {
                            label(ui, tr("Format"), tr("This file's format couldn't be recognized; pick it here."));
                            ui.vertical(|ui| {
                                ui.set_width(260.0);
                                ui.add(Dropdown::new(tr("Format"), &mut format_idx, &format_labels));
                            });
                            ui.end_row();
                        }
                        for o in &options {
                            label(ui, &o.name, &o.description);
                            let value = d.values.get_mut(&o.id).unwrap();
                            let bad = invalid.contains(&o.id);
                            match o.kind {
                                InputOptionKind::Bool => {
                                    let mut on = value.trim() == "1" || value.trim() == "true";
                                    if ui.add(BinarySwitch::new(&mut on, &o.name)).changed() {
                                        *value = if on { "1".into() } else { "0".into() };
                                    }
                                }
                                _ if !o.values.is_empty() => {
                                    let labels: Vec<String> = o.values.iter().map(text_of).collect();
                                    let mut idx = labels.iter().position(|l| l == value).unwrap_or(0);
                                    ui.vertical(|ui| {
                                        ui.set_width(260.0);
                                        if ui.add(Dropdown::new(&o.name, &mut idx, &labels)).changed() {
                                            *value = labels[idx].clone();
                                        }
                                    });
                                }
                                _ => {
                                    let stroke = if bad { theme.color(Token::Alert) } else { theme.color(Token::Seam) };
                                    ui.scope(|ui| {
                                        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, stroke);
                                        let r = ui.add(egui::TextEdit::singleline(value).desired_width(260.0));
                                        super::label_text_edit(&r, value, &o.name);
                                    });
                                }
                            }
                            ui.end_row();
                        }
                        if options.is_empty() {
                            ui.label(
                                egui::RichText::new(tr("This format has no options.")).color(theme.color(Token::Ink3)),
                            );
                            ui.end_row();
                        }
                    });
                    if !invalid.is_empty() {
                        ui.add_space(12.0);
                        let names: Vec<&str> =
                            options.iter().filter(|o| invalid.contains(&o.id)).map(|o| o.name.as_str()).collect();
                        ui.label(
                            egui::RichText::new(tr("! Enter a valid value for: {}").replacen(
                                "{}",
                                &names.join(", "),
                                1,
                            ))
                            .color(theme.color(Token::Alert)),
                        );
                    }
                });
                ui.separator();
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    let row = vec2(ui.available_width(), 40.0);
                    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if ui.add(Button::secondary(tr("Reset"))).clicked() {
                            reset = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(invalid.is_empty(), Button::primary(tr("Import"))).clicked() {
                                import = true;
                            }
                            ui.add_space(8.0);
                            if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
                if invalid.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    import = true;
                }
            },
        );
        let format_changed = format_idx != d.format;
        if format_changed {
            d.format = format_idx;
        }
        if format_changed || reset {
            let mut d = self.ui.import.take().unwrap();
            if reset {
                self.settings.import_options.remove(&formats[d.format].id);
            }
            self.fill_import_values(&mut d);
            self.ui.import = Some(d);
            return;
        }
        if import {
            let d = self.ui.import.take().unwrap();
            let f = &formats[d.format];
            let parsed: BTreeMap<String, OptionValue> =
                f.options.iter().filter_map(|o| parse_value(o, &d.values[&o.id]).map(|v| (o.id.clone(), v))).collect();
            self.settings.import_options.insert(f.id.clone(), d.values.clone());
            self.settings_dirty_at.get_or_insert(self.clock.now());
            self.start_import(d.path, ImportFormat::Module(f.id.clone()), parsed);
        } else if cancel || resp.should_close() {
            self.ui.import = None;
            // The next queued file.
            self.open_next_queued();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_with_suffixes() {
        assert_eq!(parse_rate("1000000"), Some(1_000_000));
        assert_eq!(parse_rate("1M"), Some(1_000_000));
        assert_eq!(parse_rate("1 MHz"), Some(1_000_000));
        assert_eq!(parse_rate("2.5k"), Some(2500));
        assert_eq!(parse_rate("x"), None);
    }

    #[test]
    fn a_required_sample_rate_rejects_zero() {
        let o = InputOption {
            id: "samplerate".into(),
            name: "Sample rate".into(),
            description: String::new(),
            kind: InputOptionKind::Int,
            default: OptionValue::Int(0),
            values: Vec::new(),
        };
        assert_eq!(parse_value(&o, "0"), None);
        assert_eq!(parse_value(&o, "8M"), Some(OptionValue::Int(8_000_000)));
    }
}
