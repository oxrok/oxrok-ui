//! Delete Data: the trim request from the row, Time Zero, marker
//! and trigger menus, the confirmation that says what goes, and
//! the trim itself.

use super::dialogs::modal_frame;
use crate::app::LaApp;
use crate::core::CaptureTime;
use crate::core::capture::CaptureState;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, menu_item};
use egui::{Context, Frame, Id, Margin, Modal, Ui};

/// A pending trim: keep `[keep.0, keep.1]`, described as `what` (e.g.
/// "from P1-A to P1-B").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrimRequest {
    pub keep: (CaptureTime, CaptureTime),
    pub what: String,
}

/// One *Delete Data* item: its label and what it keeps (`None` = the data
/// edge on that side).
pub(crate) type TrimItem = (&'static str, Option<CaptureTime>, Option<CaptureTime>, String);

/// *Before This Point*, *After This Point* and, with
/// the visible range, *Off Screen*.
pub(crate) fn point_items(t: CaptureTime, point: &str, visible: Option<(CaptureTime, CaptureTime)>) -> Vec<TrimItem> {
    let mut v = vec![
        ("Before This Point", Some(t), None, point.to_owned()),
        ("After This Point", None, Some(t), point.to_owned()),
    ];
    if let Some((a, b)) = visible {
        v.push(("Off Screen", Some(a), Some(b), "the visible range".to_owned()));
    }
    v
}

fn dur(t: CaptureTime) -> String {
    crate::core::format::si_sig(t.as_secs_f64().abs(), "s", 4)
}

impl LaApp {
    /// Offered for finished captures whose data can change.
    pub(crate) fn can_trim(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.state == CaptureState::Finished && s.has_data() && !s.is_read_only())
    }

    /// The *Delete Data* submenu with `items`; returns the chosen one.
    pub(crate) fn delete_data_submenu(&self, ui: &mut Ui, items: &[TrimItem]) -> Option<usize> {
        if !self.can_trim() {
            return None;
        }
        let mut pick = None;
        super::rowctl::submenu(ui, "Delete Data", 200.0, None, |ui| {
            for (k, (label, ..)) in items.iter().enumerate() {
                if menu_item(ui, label, None).clicked() {
                    pick = Some(k);
                }
            }
        });
        pick
    }

    /// Opens the confirmation for a *Delete Data* item.
    pub(crate) fn request_trim(&mut self, item: &TrimItem) {
        let Some(s) = self.session.as_ref() else { return };
        let Some((b, e)) = s.capture.as_ref().and_then(|c| c.read().time_range()) else { return };
        let (_, start, end, what) = item;
        let keep = (start.unwrap_or(b).clamp(b, e), end.unwrap_or(e).clamp(b, e));
        if keep.1 <= keep.0 {
            return;
        }
        self.ui.trim = Some(TrimRequest { keep, what: what.clone() });
    }

    /// Confirmation: what is kept, what goes, and what else changes.
    pub(crate) fn trim_dialog(&mut self, ctx: &Context) {
        let Some(req) = self.ui.trim.clone() else { return };
        let Some(s) = self.session.as_ref() else {
            self.ui.trim = None;
            return;
        };
        let Some((b, e)) = s.capture.as_ref().and_then(|c| c.read().time_range()) else {
            self.ui.trim = None;
            return;
        };
        let (k0, k1) = req.keep;
        let (before, after) = (k0 - b, e - k1);
        let deleted = before + after;
        let lead = match (before > CaptureTime::ZERO, after > CaptureTime::ZERO) {
            (true, true) => {
                format!("Keep {} ({}) and delete {} of data before and after it.", dur(k1 - k0), req.what, dur(deleted))
            }
            (true, false) => format!("Delete {} of data before {}.", dur(before), req.what),
            _ => format!("Delete {} of data after {}.", dur(after), req.what),
        };
        let mut consequences = Vec::new();
        if !s.analyzers.is_empty() {
            consequences.push("Analyzers restart.".to_owned());
        }
        let markers_gone = s.markers.items.iter().filter(|i| i.span().0 < k0 || i.span().1 > k1).count();
        let ranges_gone = s.ranges.items.iter().filter(|r| r.t.0 < k0 || r.t.1 > k1).count();
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match (markers_gone, ranges_gone) {
            (0, 0) => {}
            (m, 0) => consequences
                .push(format!("{} outside the kept data are removed.", plural(m, "timing marker", "timing markers"))),
            (0, r) => consequences
                .push(format!("{} outside the kept data are removed.", plural(r, "measurement", "measurements"))),
            (m, r) => consequences.push(format!(
                "{} and {} outside the kept data are removed.",
                plural(m, "timing marker", "timing markers"),
                plural(r, "measurement", "measurements")
            )),
        }
        let t0 = s.origin.resolve(&s.markers, s.trigger);
        let at_start = s.origin == crate::core::markers::TimeOrigin::CaptureStart;
        if !at_start && (t0 < k0 || t0 > k1) {
            consequences.push("T0 moves to the start of the capture.".to_owned());
        }
        // The samples exist nowhere else.
        let unsaved = !s.samples_saved;
        let label = format!("Delete {}", dur(deleted));
        let theme = ctx.la_theme();
        let (mut confirm, mut cancel, mut save_copy) = (false, false, false);
        let resp =
            Modal::new(Id::unique("trim_dialog")).frame(modal_frame(ctx).inner_margin(Margin::ZERO)).show(ctx, |ui| {
                ui.set_width(460.0);
                Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Delete Data")
                            .text_style(Text::Display.style())
                            .color(theme.color(Token::Ink1)),
                    );
                });
                ui.separator();
                Frame::new().inner_margin(Margin::symmetric(32, 20)).show(ui, |ui| {
                    let body = |ui: &mut Ui, t: &str| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(t).text_style(Text::Prose.style()).color(theme.color(Token::Ink2)),
                            )
                            .wrap(),
                        );
                    };
                    body(ui, &lead);
                    if !consequences.is_empty() {
                        ui.add_space(10.0);
                        body(ui, &consequences.join(" "));
                    }
                    if unsaved {
                        ui.add_space(10.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new("⚠ The deleted data can't be recovered.")
                                    .color(theme.color(Token::Caution)),
                            );
                            let link = ui.add(
                                egui::Button::new(
                                    egui::RichText::new("Save a Copy First…").color(theme.color(Token::FlareLink)),
                                )
                                .frame(false),
                            );
                            if link.clicked() {
                                save_copy = true;
                            }
                        });
                    }
                });
                ui.separator();
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    let row = egui::vec2(ui.available_width(), 40.0);
                    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(Button::destructive(&label)).clicked() {
                            confirm = true;
                        }
                        ui.add_space(8.0);
                        if ui.add(Button::secondary("Cancel")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
            });
        if save_copy {
            // Save As; the dialog stays open.
            self.save_capture(true, None);
            return;
        }
        if confirm {
            self.ui.trim = None;
            self.apply_trim(req.keep);
        } else if cancel || resp.should_close() {
            self.ui.trim = None;
        }
    }

    /// Keeps `[keep.0, keep.1]` of every channel, drops markers and
    /// ranges outside, restarts the analyzers and fixes T0.
    pub(crate) fn apply_trim(&mut self, keep: (CaptureTime, CaptureTime)) {
        let Some(s) = self.session.as_mut() else { return };
        let Some(cap) = s.capture.clone() else { return };
        {
            let mut d = cap.write();
            let Some(rate) = d.sample_rate() else { return };
            let first = keep.0.to_sample_floor(rate).max(0) as u64;
            let end = keep.1.to_sample_floor(rate).max(0) as u64 + 1;
            if let Some(ingest) = d.ingest.as_mut() {
                for ch in ingest.channels_mut() {
                    ch.trim_before(first);
                    ch.truncate(end);
                }
            }
            for (_, a) in &mut d.analog {
                a.trim_before(first);
                a.truncate(end);
            }
            d.trimmed = true;
            if d.trigger.is_some_and(|t| t < first || t >= end) {
                d.trigger = None;
            }
        }
        if let Some((b, e)) = cap.read().time_range() {
            s.markers.trim(b, e);
            s.ranges.trim(b, e);
            if s.trigger.is_some_and(|t| t < b || t > e) {
                s.trigger = None;
            }
        }
        s.samples_saved = false;
        let source = crate::app::analyzers::CaptureSource { capture: cap, done: s.capture_done.clone() };
        self.decode.set_source(s.id, Some(std::sync::Arc::new(source)));
        self.fix_origin();
        // T0 falls back when it left the data.
        if let Some(s) = self.session.as_mut() {
            let t0 = s.origin.resolve(&s.markers, s.trigger);
            if s.capture.as_ref().and_then(|c| c.read().time_range()).is_some_and(|(b, e)| t0 < b || t0 > e) {
                s.origin = s.origin.for_capture_start(false);
            }
        }
        self.fit_view();
    }
}
