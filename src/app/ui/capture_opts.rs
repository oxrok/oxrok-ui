//! Capture options in Device Settings: the trigger editor.

use crate::core::ChannelId;
use crate::core::trigger::{DigitalTrigger, TriggerEvent};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Dropdown, time_input};
use egui::{Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

/// An enabled digital channel as the editors list it: `01.  D1 name`.
pub(crate) type ChannelItem = (ChannelId, String);

fn title(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).text_style(Text::Lead.style()).color(theme.color(Token::Ink1)));
}

/// 's event glyph: rising/falling edge or a pulse.
fn event_glyph(p: &egui::Painter, r: Rect, e: TriggerEvent, c: egui::Color32) {
    let s = Stroke::new(1.5, c);
    let (lo, hi) = (r.bottom() - 3.0, r.top() + 3.0);
    let x = |f: f32| r.left() + r.width() * f;
    let pts = match e {
        TriggerEvent::Rising => vec![pos2(x(0.1), lo), pos2(x(0.5), lo), pos2(x(0.5), hi), pos2(x(0.9), hi)],
        TriggerEvent::Falling => vec![pos2(x(0.1), hi), pos2(x(0.5), hi), pos2(x(0.5), lo), pos2(x(0.9), lo)],
        TriggerEvent::PositivePulse => {
            vec![
                pos2(x(0.05), lo),
                pos2(x(0.3), lo),
                pos2(x(0.3), hi),
                pos2(x(0.7), hi),
                pos2(x(0.7), lo),
                pos2(x(0.95), lo),
            ]
        }
        TriggerEvent::NegativePulse => {
            vec![
                pos2(x(0.05), hi),
                pos2(x(0.3), hi),
                pos2(x(0.3), lo),
                pos2(x(0.7), lo),
                pos2(x(0.7), hi),
                pos2(x(0.95), hi),
            ]
        }
    };
    p.line(pts, s);
}

/// What a Trigger capture keeps from before the trigger.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PreTrigger {
    /// Everything: the device streams and the app finds the trigger.
    Everything,
    /// The pre-trigger time, in the memory of a buffered device.
    DeviceMemory,
    /// The pre-trigger time: a streaming device waits for the trigger
    /// itself.
    DeviceTrigger,
}

/// Returns whether the trigger changed.
pub(crate) fn trigger_editor(
    ui: &mut Ui,
    t: &mut DigitalTrigger,
    channels: &[ChannelItem],
    open: &mut bool,
    pre_trigger: PreTrigger,
) -> bool {
    let theme = ui.la_theme();
    let before = t.clone();
    let enabled: Vec<ChannelId> = channels.iter().map(|c| c.0).collect();
    ui.horizontal(|ui| {
        title(ui, tr("Pattern and Channel"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(Button::tertiary(tr("Reset"))).on_hover_text(tr("Restore the default trigger")).clicked() {
                *t = DigitalTrigger::reset(&enabled);
            }
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let w = ui.available_width();
        // The event dropdown with its glyph.
        let labels: Vec<&str> = TriggerEvent::ALL.iter().map(|e| e.label()).collect();
        let mut k = TriggerEvent::ALL.iter().position(|e| *e == t.event).unwrap_or(0);
        ui.allocate_ui(vec2(w * 0.4, 40.0), |ui| {
            if ui.add(Dropdown::new(tr("Trigger event"), &mut k, &labels)).changed() {
                t.event = TriggerEvent::ALL[k];
            }
        });
        let mut names: Vec<String> = channels.iter().map(|c| c.1.clone()).collect();
        let mut sel = t.channel.and_then(|c| channels.iter().position(|x| x.0 == c)).unwrap_or(usize::MAX);
        if sel == usize::MAX {
            names.push(tr("Select Channel").into());
            sel = names.len() - 1;
        }
        ui.allocate_ui(vec2(ui.available_width(), 40.0), |ui| {
            if ui.add(Dropdown::new(tr("Trigger channel"), &mut sel, &names)).changed()
                && let Some(c) = channels.get(sel)
            {
                t.channel = Some(c.0);
                t.linked.retain(|(l, _)| *l != c.0);
            }
        });
    });
    let (r, _) = ui.allocate_exact_size(vec2(40.0, 18.0), Sense::hover());
    event_glyph(ui.painter(), r, t.event, theme.color(Token::Ink2));
    // Pulse width range.
    if t.event.is_pulse() {
        ui.add_space(10.0);
        title(ui, tr("Pulse width range"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let half = (ui.available_width() - 24.0) / 2.0;
            ui.allocate_ui(vec2(half, 40.0), |ui| {
                if time_input(ui, tr("Pulse width minimum"), &mut t.pulse_min, 0.0) && t.pulse_min > t.pulse_max {
                    t.pulse_max = t.pulse_min;
                }
            });
            ui.label("–");
            ui.allocate_ui(vec2(half, 40.0), |ui| {
                if time_input(ui, tr("Pulse width maximum"), &mut t.pulse_max, 0.0) && t.pulse_max < t.pulse_min {
                    t.pulse_min = t.pulse_max;
                }
            });
        });
    }
    // Other channel conditions.
    let others: Vec<&ChannelItem> = channels.iter().filter(|c| Some(c.0) != t.channel).collect();
    if !others.is_empty() {
        ui.add_space(10.0);
        let n = t.linked.len();
        let label = if n > 0 {
            tr("Other channel conditions ({})").replacen("{}", &n.to_string(), 1)
        } else {
            tr("Other channel conditions").to_owned()
        };
        // A disclosure triangle (drawn: the UI font has no ▸).
        let head = ui.add(egui::Button::new(format!("     {label}")).frame(false));
        head.widget_info(|| WidgetInfo::selected(Role::Button, true, *open, tr("Other channel conditions")));
        let c = head.rect.left_center() + vec2(8.0, 0.0);
        let pts = if *open {
            vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
        } else {
            vec![c + vec2(-2.0, -4.0), c + vec2(-2.0, 4.0), c + vec2(3.0, 0.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, theme.color(Token::Ink3), Stroke::NONE));
        if head.clicked() {
            *open = !*open;
        }
        if *open {
            for (id, name) in others {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name).color(theme.color(Token::Ink2)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cur = t.linked.iter().find(|(c, _)| c == id).map(|x| x.1);
                        for (level, text) in [(true, tr("Hi")), (false, tr("Lo"))] {
                            let on = cur == Some(level);
                            let r = ui.add(Button::tertiary(text).selected(on));
                            r.widget_info(|| WidgetInfo::selected(Role::Button, true, on, format!("{name} {text}")));
                            if r.clicked() {
                                t.linked.retain(|(c, _)| c != id);
                                if !on {
                                    t.linked.push((*id, level));
                                }
                            }
                        }
                    });
                });
            }
        }
    }
    // A device that waits for the trigger keeps only this much before
    // it; one the app scans keeps it all.
    let help = match pre_trigger {
        PreTrigger::Everything => None,
        PreTrigger::DeviceMemory => Some(tr(
            "This device records into its own memory, so it keeps only this much data before the trigger. Long durations use more device memory; with RLE the result can be longer.",
        )),
        PreTrigger::DeviceTrigger => {
            Some(tr("The device waits for this trigger itself, so the capture keeps only this much data before it."))
        }
    };
    if let Some(help) = help {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            title(ui, tr("Capture duration before trigger"));
            super::panels::help_icon(ui, help);
        });
        ui.add_space(6.0);
        time_input(ui, tr("Capture duration before trigger"), &mut t.pre_trigger_seconds, 0.0);
    }
    ui.add_space(10.0);
    title(ui, tr("Capture duration after trigger"));
    ui.add_space(6.0);
    time_input(ui, tr("Capture duration after trigger"), &mut t.post_trigger_seconds, 0.0);
    *t != before
}
