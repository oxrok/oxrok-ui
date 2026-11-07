//! Device Settings panel rows: the logic threshold,
//! generic device options and the command line.

use super::hint;
use crate::app::LaApp;
use crate::app::worker::WorkerCmd;
use crate::backend::{
    ConfValue, DeviceOption, OptionGroup, OptionKind, OptionScope, ThresholdCaps, ThresholdChoice, format_value,
};
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{BinarySwitch, Button, Dropdown, Segment, number_input, segmented, text_input};
use egui::{Align, Layout, Rect, Role, Sense, Stroke, Ui, WidgetInfo, vec2};
use std::time::Duration;

/// read-only rows refresh this often while shown.
const REFRESH: Duration = Duration::from_secs(2);
/// More scopes than this use a dropdown.
const MAX_SEGMENTS: usize = 4;
/// Share of the row width for the control.
const CONTROL_SHARE: f32 = 0.55;
/// Custom threshold resolution.
const THRESHOLD_RESOLUTION: f64 = 0.01;

/// Label for a threshold without the device's list.
fn choice_label(c: &ThresholdChoice, caps: Option<&ThresholdCaps>) -> String {
    match c {
        ThresholdChoice::Preset(id) => caps
            .and_then(|t| t.options.iter().find(|o| &o.id == id))
            .map_or_else(|| id.replace(':', "–"), |o| o.label.clone()),
        ThresholdChoice::Custom(v) => format!("{} V", trim(*v)),
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Inside the panel's disabled area. Returns a new choice.
pub(super) fn threshold_ui(
    ui: &mut Ui,
    caps: Option<&ThresholdCaps>,
    current: Option<&ThresholdChoice>,
) -> Option<ThresholdChoice> {
    let Some(t) = caps else {
        // A file's panel: what the capture used.
        if let Some(c) = current {
            ui.add_space(12.0);
            hint(ui, &format!("Logic threshold: {}", choice_label(c, None)));
        }
        return None;
    };
    ui.add_space(12.0);
    let mut out = None;
    let custom_value = |r: &crate::backend::NumRange| match current {
        Some(ThresholdChoice::Custom(v)) => *v,
        _ => r.snap(1.5),
    };
    let input = |ui: &mut Ui, label: &str, r: &crate::backend::NumRange, v: f64| {
        let mut v = v;
        let step = r.step.max(THRESHOLD_RESOLUTION);
        number_input(ui, label, &mut v, "V", Some((r.min, r.max, step))).then_some(ThresholdChoice::Custom(v))
    };
    if t.options.is_empty() {
        // A voltage range only.
        let Some(r) = &t.custom else { return None };
        hint(ui, "Logic threshold");
        ui.add_space(6.0);
        return input(ui, "Logic threshold", r, custom_value(r));
    }
    hint(ui, "Logic threshold");
    ui.add_space(6.0);
    let mut labels: Vec<String> = t.options.iter().map(|o| o.label.clone()).collect();
    if t.custom.is_some() {
        labels.push("Custom…".into());
    }
    let mut idx = match current {
        Some(ThresholdChoice::Preset(id)) => t.options.iter().position(|o| &o.id == id).unwrap_or(0),
        Some(ThresholdChoice::Custom(_)) if t.custom.is_some() => t.options.len(),
        _ => 0,
    };
    if ui.add(Dropdown::new("Logic threshold", &mut idx, &labels)).changed() {
        out = Some(match t.options.get(idx) {
            Some(o) => ThresholdChoice::Preset(o.id.clone()),
            None => ThresholdChoice::Custom(custom_value(t.custom.as_ref().unwrap())),
        });
    }
    let shown_custom = matches!(out.as_ref().or(current), Some(ThresholdChoice::Custom(_)));
    if shown_custom && let Some(r) = &t.custom {
        ui.add_space(8.0);
        let v = match out.as_ref().or(current) {
            Some(ThresholdChoice::Custom(v)) => *v,
            _ => custom_value(r),
        };
        if let Some(c) = input(ui, "Custom threshold", r, v) {
            out = Some(c);
        }
    }
    out
}

/// A row: label (with the description as tooltip) left, control right.
fn option_row(ui: &mut Ui, name: &str, description: &str, control: impl FnOnce(&mut Ui)) {
    let theme = ui.la_theme();
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        let w = ui.available_width();
        ui.label(egui::RichText::new(name).color(theme.color(Token::Ink2)));
        if !description.is_empty() && description != name {
            ui.add_space(2.0);
            super::panels::help_icon(ui, description);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.allocate_ui_with_layout(vec2(w * CONTROL_SHARE, 40.0), Layout::right_to_left(Align::Center), control);
        });
    });
}

/// A collapsible header row; returns whether it was clicked.
fn disclosure(ui: &mut Ui, id: &str, label: &str, open: bool, right: impl FnOnce(&mut Ui)) -> bool {
    let theme = ui.la_theme();
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() * 0.7, 32.0), Sense::hover());
        let resp = ui.interact(r, ui.make_persistent_id(("disclosure", id)), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::Button, true, open, label));
        let color = theme.color(if resp.hovered() { Token::Ink1 } else { Token::Ink2 });
        let tri = Rect::from_center_size(egui::pos2(r.left() + 6.0, r.center().y), vec2(8.0, 8.0));
        let pts = if open {
            vec![tri.left_top(), tri.right_top(), egui::pos2(tri.center().x, tri.bottom())]
        } else {
            vec![tri.left_top(), egui::pos2(tri.right(), tri.center().y), tri.left_bottom()]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        ui.painter().text(
            egui::pos2(r.left() + 18.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            Text::Headline.font_id(),
            color,
        );
        clicked = resp.clicked();
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    clicked
}

/// An edit made in the options section.
enum Edit {
    Set(OptionScope, Box<DeviceOption>, ConfValue),
    Reset,
}

/// One control for `o`, given its value. Returns a new value.
fn control(ui: &mut Ui, o: &DeviceOption, value: Option<&ConfValue>, enabled: bool) -> Option<ConfValue> {
    let theme = ui.la_theme();
    let text = |ui: &mut Ui, s: String| {
        ui.label(egui::RichText::new(s).color(theme.color(Token::Ink3)));
        None
    };
    if !o.writable {
        return text(ui, value.map_or_else(|| "–".into(), |v| format_value(v, o.unit)));
    }
    ui.add_enabled_ui(enabled, |ui| match (&o.kind, value) {
        (OptionKind::Bool, v) => {
            let mut on = matches!(v, Some(ConfValue::Bool(true)));
            ui.add(BinarySwitch::new(&mut on, &o.name)).changed().then_some(ConfValue::Bool(on))
        }
        (OptionKind::Choice(choices), v) => {
            let labels: Vec<String> = choices.iter().map(|c| format_value(c, o.unit)).collect();
            let mut idx = v.and_then(|v| choices.iter().position(|c| c == v)).unwrap_or(0);
            ui.add(Dropdown::new(&o.name, &mut idx, &labels)).changed().then(|| choices[idx].clone())
        }
        (OptionKind::Range(r), v) => {
            let template = v.cloned().unwrap_or(ConfValue::Float(r.min));
            let mut f = template.as_f64().unwrap_or(r.min);
            let unit = o.unit.map_or("", |u| u.symbol());
            number_input(ui, &o.name, &mut f, unit, Some((r.min, r.max, r.step))).then(|| template.with_f64(f))
        }
        (OptionKind::Free, Some(ConfValue::Str(s))) => {
            let mut s = s.clone();
            text_input(ui, &o.name, &mut s).then_some(ConfValue::Str(s))
        }
        (OptionKind::Free, Some(v)) if v.as_f64().is_some() => {
            let mut f = v.as_f64().unwrap();
            let unit = o.unit.map_or("", |u| u.symbol());
            number_input(ui, &o.name, &mut f, unit, None).then(|| v.with_f64(f))
        }
        (OptionKind::Free, v) => text(ui, v.map_or_else(|| "–".into(), |v| format_value(v, o.unit))),
    })
    .inner
}

impl LaApp {
    /// "▸ Device options (n)".
    pub(super) fn device_options_section(&mut self, ui: &mut Ui, busy: bool) {
        let Some(s) = self.session.as_ref() else { return };
        let read_only = s.is_read_only();
        let groups: Vec<OptionGroup> = match &s.caps {
            Some(c) => c.options.clone(),
            // A file has no driver: the snapshot's values, read-only.
            None if read_only => s
                .settings
                .device_options
                .iter()
                .map(|(scope, values)| OptionGroup {
                    scope: scope.clone(),
                    options: values
                        .iter()
                        .map(|(id, v)| DeviceOption {
                            id: id.clone(),
                            name: crate::core::device::title_case(id),
                            description: String::new(),
                            kind: OptionKind::Free,
                            unit: crate::backend::Unit::for_key(id),
                            value: Some(v.clone()),
                            default: None,
                            writable: false,
                        })
                        .collect(),
                })
                .collect(),
            None => Vec::new(),
        };
        if groups.is_empty() {
            return;
        }
        let n = s.changed_options();
        let label = if n == 0 { "Device options".to_owned() } else { format!("Device options ({n})") };
        super::inspector::divider(ui);
        let open = self.ui.device_options_open;
        if disclosure(ui, "device_options", &label, open, |_| {}) {
            self.ui.device_options_open = !open;
        }
        if !self.ui.device_options_open {
            return;
        }
        ui.add_space(8.0);
        let mut scope_idx = self.ui.device_options_scope.min(groups.len() - 1);
        if groups.len() > 1 {
            let labels: Vec<&str> = groups.iter().map(|g| g.scope.label()).collect();
            if groups.len() > MAX_SEGMENTS {
                ui.add(Dropdown::new("Channel group", &mut scope_idx, &labels));
            } else {
                let segs: Vec<Segment> =
                    labels.iter().map(|l| Segment { label: l, enabled: true, tooltip: "" }).collect();
                segmented(ui, "device_option_scope", &segs, &mut scope_idx);
            }
            ui.add_space(8.0);
        }
        self.ui.device_options_scope = scope_idx;
        let group = &groups[scope_idx];
        let mut edits = Vec::new();
        let mut needs_refresh = false;
        for o in &group.options {
            let value = if read_only || !o.writable { o.value.clone() } else { s.option_value(&group.scope, o) };
            needs_refresh |= !o.writable && !read_only;
            option_row(ui, &o.name, &o.description, |ui| {
                if let Some(v) = control(ui, o, value.as_ref(), !busy && !read_only) {
                    edits.push(Edit::Set(group.scope.clone(), Box::new(o.clone()), v));
                }
            });
        }
        if !read_only {
            ui.add_space(4.0);
            if ui.add_enabled(!busy && n > 0, Button::tertiary("Reset to defaults")).clicked() {
                edits.push(Edit::Reset);
            }
        }
        // read-only values refresh while shown.
        let now = self.now();
        if needs_refresh && !busy {
            let due = self.ui.options_read_at.is_none_or(|t| now >= t + REFRESH);
            if due {
                self.ui.options_read_at = Some(now);
                self.worker.send(WorkerCmd::ReadOptions { session: s.id });
            }
            ui.ctx().request_repaint_after(REFRESH);
        }
        if edits.is_empty() {
            return;
        }
        let s = self.session.as_mut().unwrap();
        for e in edits {
            match e {
                Edit::Set(scope, o, v) => s.set_option(&scope, &o, v),
                Edit::Reset => s.settings.device_options.clear(),
            }
        }
        self.mark_settings_dirty();
    }

    /// "▸ Command line", outside the disabled area: copying works
    /// during a capture and in a file's panel.
    pub(super) fn command_line_section(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_ref() else { return };
        let cmd = s.cli_command();
        super::inspector::divider(ui);
        let open = self.settings.inspector.command_line_open;
        let mut copy = false;
        let toggled = disclosure(ui, "command_line", "Command line", open, |ui| {
            if open {
                copy = ui.add(Button::tertiary("Copy")).on_hover_text("Copy the oxrok command").clicked();
            }
        });
        if toggled {
            self.settings.inspector.command_line_open = !open;
            self.mark_settings_dirty();
        }
        if open {
            let theme = ui.la_theme();
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(theme.color(Token::Card))
                .corner_radius(3.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Breaks at 80 columns, or the box's width when narrower.
                    let font = egui::FontId::monospace(12.0);
                    let glyph = ui.fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
                    let cols = ((ui.available_width() / glyph) as usize).clamp(20, crate::app::cli::WRAP);
                    let text = cmd.wrapped(cols);
                    let r = ui.add(
                        egui::Label::new(egui::RichText::new(&text).font(font).color(theme.color(Token::Ink2))).wrap(),
                    );
                    r.widget_info(|| WidgetInfo::labeled(Role::Label, true, cmd.one_line()));
                });
        }
        if copy {
            self.copy_cli_command(ui.ctx());
        }
    }

    /// Copy, *Capture → Copy oxrok Command* and the palette.
    pub(crate) fn copy_cli_command(&mut self, ctx: &egui::Context) {
        let Some(s) = self.session.as_ref() else { return };
        ctx.copy_text(s.cli_command().one_line());
        self.toast(Toast::new(ToastKind::Success, "oxrok command copied"));
    }

    /// Edit from the panel.
    pub(super) fn set_threshold(&mut self, t: ThresholdChoice) {
        if let Some(s) = self.session.as_mut() {
            s.settings.logic_threshold = Some(t);
            self.mark_settings_dirty();
        }
    }
}
