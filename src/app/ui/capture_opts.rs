//! Capture options in Device Settings: the trigger editor.

use crate::core::ChannelId;
use crate::core::trigger::DigitalTrigger;
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, time_input};
use egui::Ui;

/// An enabled digital channel as the editors list it: `01.  D1 name`.
pub(crate) type ChannelItem = (ChannelId, String);

fn title(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).text_style(Text::Lead.style()).color(theme.color(Token::Ink1)));
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

/// What the editor shows of the trigger besides its times.
pub(crate) struct TriggerInfo {
    /// A line per stage on offer: its letter and what it waits for.
    pub stages: Vec<(&'static str, String)>,
    /// What looks for the trigger, or why nothing can (`Err`).
    pub status: Result<&'static str, String>,
}

/// The trigger in words, a button to its details, and its times.
/// Returns whether the trigger changed, and whether the details were
/// asked for.
pub(crate) fn trigger_editor(
    ui: &mut Ui,
    t: &mut DigitalTrigger,
    channels: &[ChannelItem],
    info: &TriggerInfo,
    pre_trigger: PreTrigger,
) -> (bool, bool) {
    let theme = ui.la_theme();
    let before = t.clone();
    let enabled: Vec<ChannelId> = channels.iter().map(|c| c.0).collect();
    let mut details = false;
    ui.horizontal(|ui| {
        title(ui, tr("Trigger"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(Button::tertiary(tr("Reset"))).on_hover_text(tr("Restore the default trigger")).clicked() {
                *t = DigitalTrigger::reset(&enabled);
            }
        });
    });
    ui.add_space(8.0);
    for (letter, text) in &info.stages {
        ui.horizontal_wrapped(|ui| {
            if info.stages.len() > 1 {
                ui.label(egui::RichText::new(*letter).text_style(Text::ValueS.style()).color(theme.color(Token::Ink3)));
            }
            let (text, ink) =
                if text.is_empty() { (tr("No condition"), Token::Ink4) } else { (text.as_str(), Token::Ink2) };
            ui.label(egui::RichText::new(text).color(theme.color(ink)));
        });
    }
    ui.add_space(6.0);
    let (text, ink) = match &info.status {
        Ok(text) => (*text, Token::Ink3),
        Err(e) => (e.as_str(), Token::Alert),
    };
    ui.label(egui::RichText::new(text).color(theme.color(ink)));
    ui.add_space(8.0);
    if ui
        .add(Button::secondary(tr("Trigger Details…")))
        .on_hover_text(tr("Counts, durations, value ranges and how the stages combine"))
        .clicked()
    {
        details = true;
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
    (*t != before, details)
}
