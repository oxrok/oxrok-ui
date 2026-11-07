//! The Settings dialog: a navigation column of sections with a
//! search field, one section's items at a time, changes applied at once.

use super::dialogs::modal_frame;
use crate::app::settings::{RECENT_FILES, RecordOnOpened, RelativeZoom, ThemePref, UI_SCALE_MAX, UI_SCALE_MIN};
use crate::app::{AppEvent, LaApp};
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::waveform::TraceWidth;
use crate::ui::widgets::{Button, Checkbox, Dropdown, Segment, segmented, stepper};
use egui::{Context, Frame, Id, Margin, Modal, Rect, Role, Sense, Stroke, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use std::path::PathBuf;

/// Sizes.
const WIDTH: f32 = 720.0;
const NAV_W: f32 = 170.0;
const NAV_ROW: f32 = 36.0;
const BODY_H: f32 = 440.0;
const LABEL_W: f32 = 200.0;

/// Sections, in navigation order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Appearance,
    Waveform,
    TimeLabels,
    Capture,
    Files,
    Decoders,
    History,
    Related,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Appearance,
        Section::Waveform,
        Section::TimeLabels,
        Section::Capture,
        Section::Files,
        Section::Decoders,
        Section::History,
        Section::Related,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Waveform => "Waveform",
            Section::TimeLabels => "Time Labels",
            Section::Capture => "Capture",
            Section::Files => "Files & Folders",
            Section::Decoders => "Decoders",
            Section::History => "Data & History",
            Section::Related => "Related",
        }
    }

    /// The item labels, for search.
    fn items(self) -> &'static [&'static str] {
        match self {
            Section::Appearance => &["Theme", "UI scale", "Show tooltips"],
            Section::Waveform => &[
                "Color waveforms by channel",
                "Trace width",
                "Auto layout",
                "Show interactive measurement",
                "Zoom relative to",
                "Wire status interval",
            ],
            Section::TimeLabels => &["Use UTC timezone", "Use ISO 8601 format"],
            Section::Capture => &[
                "Center on trigger after capture",
                "Ask before replacing an unsaved capture",
                "When recording on an opened file",
            ],
            Section::Files => &["Rename the session to the saved file name", "Default export folder"],
            Section::Decoders => &["Additional decoder directory"],
            Section::History => &["Measurement metrics", "Recent files", "Recent commands"],
            Section::Related => &["Keyboard Shortcuts", "Probe Colors"],
        }
    }

    /// The name or an item has every word of `query`.
    fn matches(self, query: &str) -> bool {
        let hay = std::iter::once(self.label()).chain(self.items().iter().copied()).collect::<Vec<_>>().join(" ");
        let hay = hay.to_lowercase();
        query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
    }
}

/// Something to run once the dialog has drawn (it borrows the settings).
type Deferred = Box<dyn FnOnce(&mut LaApp)>;

/// A folder the dialog's *Browse* buttons pick.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Folder {
    Export,
    Decoders,
}

/// The open dialog.
#[derive(Clone, Debug)]
pub(crate) struct SettingsDialog {
    pub section: Section,
    pub search: String,
}

/// Control heights, so a row's label is centered on its control.
const SEGMENTED_H: f32 = 46.0;
const FIELD_H: f32 = 40.0;

/// A labelled row: the label on the left, centered on a control of
/// height `h`, the control in the rest.
fn row(ui: &mut Ui, label: &str, h: f32, add: impl FnOnce(&mut Ui)) {
    let theme = ui.la_theme();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, h), Sense::hover());
        ui.painter().text(
            pos2(r.left(), r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            Text::Prose.font_id(),
            theme.color(Token::Ink2),
        );
        add(ui);
    });
    ui.add_space(10.0);
}

fn check(ui: &mut Ui, on: &mut bool, label: &str, tip: Option<&str>) {
    let r = ui.add(Checkbox::new(on, label));
    if let Some(t) = tip {
        r.on_hover_text(t);
    }
    ui.add_space(10.0);
}

/// A segmented choice of `options`; returns the new index when it changed.
fn choice(ui: &mut Ui, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    let segs: Vec<Segment> = options.iter().map(|l| Segment { label: l, enabled: true, tooltip: "" }).collect();
    let mut sel = selected;
    let w = (options.len() as f32 * 90.0).min(ui.available_width());
    let changed = ui.allocate_ui(vec2(w, SEGMENTED_H), |ui| segmented(ui, id, &segs, &mut sel)).inner;
    changed.then_some(sel)
}

/// A path row: the path (or `placeholder`) read-only, Browse, and Clear
/// when set. Returns (browse, clear).
fn path_row(ui: &mut Ui, label: &str, path: Option<&PathBuf>, placeholder: &str) -> (bool, bool) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(label).text_style(Text::Prose.style()).color(theme.color(Token::Ink2)));
    ui.add_space(6.0);
    let mut out = (false, false);
    ui.horizontal(|ui| {
        let mut text = path.map_or_else(|| placeholder.to_owned(), |p| p.display().to_string());
        let field = egui::TextEdit::singleline(&mut text)
            .interactive(false)
            .desired_width(ui.available_width() - if path.is_some() { 170.0 } else { 90.0 });
        let r = ui.add(field);
        super::label_text_edit(&r, &text, label);
        out.0 = ui.add(Button::secondary("Browse")).clicked();
        if path.is_some() {
            out.1 = ui.add(Button::tertiary("Clear")).clicked();
        }
    });
    ui.add_space(14.0);
    out
}

impl LaApp {
    /// Open the dialog on `section`.
    pub(crate) fn open_settings(&mut self, section: Section) {
        self.ui.settings_dialog = Some(SettingsDialog { section, search: String::new() });
    }

    fn pick_settings_folder(&self, which: Folder, start: Option<PathBuf>) {
        let (dialogs, tx, ctx) = (self.dialogs.clone(), self.events_tx.clone(), self.ui.ctx.clone());
        std::thread::spawn(move || {
            let picked = dialogs.pick_folder(start.as_deref());
            let _ = tx.send(AppEvent::SettingsFolder(which, picked));
            ctx.request_repaint();
        });
    }

    pub(crate) fn on_settings_folder(&mut self, which: Folder, picked: Option<PathBuf>) {
        let Some(p) = picked else { return };
        match which {
            Folder::Export => self.settings.files.export_dir = Some(p),
            Folder::Decoders => self.settings.decoders.extra_dir = Some(p),
        }
        self.mark_settings_dirty();
    }

    pub(crate) fn settings_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.ui.settings_dialog.take() else { return };
        let theme = ctx.la_theme();
        let before = self.settings.clone();
        let mut close = false;
        let mut after: Vec<Deferred> = Vec::new();
        let shown: Vec<Section> = Section::ALL.iter().copied().filter(|s| s.matches(&d.search)).collect();
        // The first match is selected while searching.
        if !d.search.trim().is_empty()
            && !shown.contains(&d.section)
            && let Some(&first) = shown.first()
        {
            d.section = first;
        }
        let resp = Modal::new(Id::unique("settings_dialog")).frame(modal_frame(ctx).inner_margin(Margin::ZERO)).show(
            ctx,
            |ui| {
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing.y = 0.0;
                // Title bar.
                Frame::new().inner_margin(Margin { left: 32, right: 16, top: 20, bottom: 16 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Settings")
                                .text_style(Text::Display.style())
                                .color(theme.color(Token::Ink1)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::ranges::icon_button(ui, "Close settings", crate::ui::icons::close).clicked() {
                                close = true;
                            }
                        });
                    });
                });
                let (body, _) = ui.allocate_exact_size(vec2(WIDTH, BODY_H), Sense::hover());
                let p = ui.painter();
                p.hline(body.x_range(), body.top(), Stroke::new(1.0, theme.color(Token::Seam)));
                let nav = Rect::from_min_size(body.min, vec2(NAV_W, BODY_H));
                p.rect_filled(nav, 0.0, theme.color(Token::Lift));
                p.vline(nav.right(), nav.y_range(), Stroke::new(1.0, theme.color(Token::Seam)));
                // Navigation: the search field, then one button per section.
                let mut nav_ui = ui.new_child(UiBuilder::new().max_rect(nav.shrink2(vec2(0.0, 12.0))));
                nav_ui.spacing_mut().item_spacing.y = 0.0;
                nav_ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut d.search)
                            .hint_text("Search settings")
                            .desired_width(NAV_W - 24.0),
                    );
                    super::label_text_edit(&r, &d.search, "Search settings");
                });
                nav_ui.add_space(12.0);
                for s in &shown {
                    let (r, resp) = nav_ui.allocate_exact_size(vec2(NAV_W, NAV_ROW), Sense::click());
                    let on = *s == d.section;
                    resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, s.label()));
                    let p = nav_ui.painter();
                    if on {
                        p.rect_filled(r, 0.0, theme.color(Token::Field));
                        p.rect_filled(
                            Rect::from_min_size(r.min, vec2(2.0, r.height())),
                            0.0,
                            theme.color(Token::RingHot),
                        );
                    } else if resp.hovered() {
                        p.rect_filled(r, 0.0, theme.color(Token::Card));
                    }
                    let c = if on { Token::Ink1 } else { Token::Ink2 };
                    p.text(
                        pos2(r.left() + 16.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        s.label(),
                        Text::Prose.font_id(),
                        theme.color(c),
                    );
                    if resp.clicked() {
                        d.section = *s;
                    }
                }
                // Content: the selected section only.
                let content = Rect::from_min_max(pos2(nav.right() + 1.0, body.top()), body.max).shrink(32.0);
                let mut c_ui =
                    ui.new_child(UiBuilder::new().max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut c_ui, |ui| {
                    if shown.is_empty() {
                        ui.label(
                            egui::RichText::new("No settings match")
                                .text_style(Text::Prose.style())
                                .color(theme.color(Token::Ink3)),
                        );
                        return;
                    }
                    ui.label(
                        egui::RichText::new(d.section.label())
                            .font(egui::FontId::new(
                                13.0,
                                crate::ui::typography::family(crate::ui::typography::SANS_BOLD),
                            ))
                            .color(theme.color(Token::Ink1)),
                    );
                    ui.add_space(16.0);
                    self.section_items(ui, d.section, &mut after);
                });
                // Footer.
                let (foot, _) = ui.allocate_exact_size(vec2(WIDTH, 1.0), Sense::hover());
                ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, theme.color(Token::Seam)));
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(Button::secondary("Reset all settings…")).clicked() {
                            after.push(Box::new(|app: &mut LaApp| {
                                app.ui.confirm = Some(super::dialogs::Confirm::reset_all_settings());
                            }));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(Button::primary("Close")).clicked() {
                                close = true;
                            }
                        });
                    });
                });
            },
        );
        // Escape or the backdrop close it, unless a confirmation is on top.
        if resp.should_close() && self.ui.confirm.is_none() {
            close = true;
        }
        if !close {
            self.ui.settings_dialog = Some(d);
        }
        for f in after {
            f(self);
        }
        // Changes apply at once and are saved.
        if self.settings != before {
            if self.settings.waveform.auto_layout != before.waveform.auto_layout {
                self.ui.auto_layout_for = None;
            }
            self.mark_settings_dirty();
        }
    }

    /// One section's items (table).
    fn section_items(&mut self, ui: &mut Ui, section: Section, after: &mut Vec<Deferred>) {
        let theme = ui.la_theme();
        let s = &mut self.settings;
        match section {
            Section::Appearance => {
                row(ui, "Theme", SEGMENTED_H, |ui| {
                    let i = ThemePref::ALL.iter().position(|t| *t == s.appearance.theme).unwrap_or(2);
                    if let Some(k) = choice(ui, "set-theme", &["Light", "Dark", "System"], i) {
                        s.appearance.theme = ThemePref::ALL[k];
                    }
                });
                row(ui, "UI scale", FIELD_H, |ui| {
                    let v = s.appearance.ui_scale;
                    let text = format!("{v:.1}");
                    let step = ui
                        .allocate_ui(vec2(140.0, FIELD_H), |ui| {
                            stepper(ui, "UI scale", &text, v > UI_SCALE_MIN + 1e-3, v < UI_SCALE_MAX - 1e-3)
                        })
                        .inner;
                    if step != 0 {
                        after.push(Box::new(move |app: &mut LaApp| app.step_ui_scale(Some(step))));
                    }
                    ui.add_space(12.0);
                    if (v - 1.0).abs() > 1e-3 && ui.add(Button::tertiary("Reset")).clicked() {
                        after.push(Box::new(|app: &mut LaApp| app.step_ui_scale(None)));
                    }
                });
                check(ui, &mut s.appearance.show_tooltips, "Show tooltips", None);
            }
            Section::Waveform => {
                check(
                    ui,
                    &mut s.waveform.color_waveforms,
                    "Color waveforms by channel",
                    Some("Channel colors follow the resistor color code. Off: every trace in one neutral color."),
                );
                row(ui, "Trace width", SEGMENTED_H, |ui| {
                    let i = TraceWidth::ALL.iter().position(|w| *w == s.waveform.trace_width).unwrap_or(1);
                    if let Some(k) = choice(ui, "set-trace", &["Thin", "Normal", "Bold"], i) {
                        s.waveform.trace_width = TraceWidth::ALL[k];
                    }
                });
                check(ui, &mut s.waveform.auto_layout, "Auto layout", None);
                check(ui, &mut s.waveform.show_interactive_measurement, "Show interactive measurement", None);
                row(ui, "Zoom relative to", SEGMENTED_H, |ui| {
                    let i = usize::from(s.waveform.zoom_relative_to == RelativeZoom::Mouse);
                    if let Some(k) = choice(ui, "set-zoom-rel", &["Center", "Mouse"], i) {
                        s.waveform.zoom_relative_to = if k == 1 { RelativeZoom::Mouse } else { RelativeZoom::Center };
                    }
                });
                row(ui, "Wire status interval", FIELD_H, |ui| {
                    let mut v = s.waveform.wire_status_interval_ms as f64;
                    let changed = ui
                        .allocate_ui(vec2(140.0, FIELD_H), |ui| {
                            crate::ui::widgets::number_input(
                                ui,
                                "Wire status interval",
                                &mut v,
                                "ms",
                                Some((20.0, 1000.0, 10.0)),
                            )
                        })
                        .inner;
                    if changed {
                        s.waveform.wire_status_interval_ms = v.round().clamp(20.0, 1000.0) as u32;
                    }
                });
            }
            Section::TimeLabels => {
                check(ui, &mut s.time_labels.utc, "Use UTC timezone", None);
                let now = crate::core::WallClock::now();
                let ex = |iso| {
                    crate::core::format::wall_clock(
                        &now,
                        crate::core::CaptureTime::ZERO,
                        crate::core::format::WallClockFormat { iso, utc: s.time_labels.utc },
                    )
                };
                let tip = format!("ISO 8601: {}\nDefault: {}", ex(true), ex(false));
                check(ui, &mut s.time_labels.iso8601, "Use ISO 8601 format", Some(&tip));
            }
            Section::Capture => {
                check(ui, &mut s.capture.center_on_trigger, "Center on trigger after capture", None);
                check(ui, &mut s.capture.ask_replace_unsaved, "Ask before replacing an unsaved capture", None);
                check(
                    ui,
                    &mut s.capture.pause_analyzers_on_repeat,
                    "Pause analyzers during Repeat",
                    Some("No decoding while a Repeat run goes on; the kept round is decoded when it ends."),
                );
                row(ui, "When recording on an opened file", FIELD_H, |ui| {
                    let opts = ["Ask", "New session", "Replace data"];
                    let all = [RecordOnOpened::Ask, RecordOnOpened::NewSession, RecordOnOpened::Replace];
                    let mut i = all.iter().position(|r| *r == s.capture.record_on_opened).unwrap_or(0);
                    ui.allocate_ui(vec2(200.0, FIELD_H), |ui| {
                        ui.add(Dropdown::new("When recording on an opened file", &mut i, &opts))
                    });
                    s.capture.record_on_opened = all[i];
                });
            }
            Section::Files => {
                check(ui, &mut s.files.rename_on_save, "Rename the session to the saved file name", None);
                let docs = dirs::document_dir().map(|p| p.display().to_string()).unwrap_or_default();
                let (browse, clear) = path_row(ui, "Default export folder", s.files.export_dir.as_ref(), &docs);
                if browse {
                    let start = s.files.export_dir.clone();
                    after.push(Box::new(move |app: &mut LaApp| app.pick_settings_folder(Folder::Export, start)));
                }
                if clear {
                    s.files.export_dir = None;
                }
            }
            Section::Decoders => {
                let (browse, clear) =
                    path_row(ui, "Additional decoder directory", s.decoders.extra_dir.as_ref(), "None");
                if browse {
                    let start = s.decoders.extra_dir.clone();
                    after.push(Box::new(move |app: &mut LaApp| app.pick_settings_folder(Folder::Decoders, start)));
                }
                if clear {
                    s.decoders.extra_dir = None;
                }
                ui.horizontal(|ui| {
                    let (g, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    crate::ui::icons::warning(ui.painter(), g, theme.color(Token::Caution));
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Restart required. Decoders are loaded only at application start.")
                            .text_style(Text::Prose.style())
                            .color(theme.color(Token::Ink3)),
                    );
                });
            }
            Section::History => {
                row(ui, "Measurement metrics", FIELD_H, |ui| {
                    let default = crate::app::settings::MeasurementMetrics::default();
                    let r = ui.add_enabled(s.measurement_metrics != default, Button::secondary("Reset to defaults"));
                    if r.clicked() {
                        s.measurement_metrics = default;
                    }
                });
                let n = s.files.recent_files.iter().filter(|p| p.exists()).count();
                row(ui, "Recent files", FIELD_H, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{n} of {RECENT_FILES}"))
                            .text_style(Text::Prose.style())
                            .color(theme.color(Token::Ink3)),
                    );
                    ui.add_space(12.0);
                    if ui.add_enabled(n > 0, Button::secondary("Clear")).clicked() {
                        s.files.recent_files.clear();
                    }
                });
                row(ui, "Recent commands", FIELD_H, |ui| {
                    let any = !s.history.recent_commands.is_empty();
                    if ui.add_enabled(any, Button::secondary("Reset")).clicked() {
                        s.history.recent_commands.clear();
                    }
                });
            }
            Section::Related => {
                if ui.add(Button::tertiary("Keyboard Shortcuts")).clicked() {
                    after.push(Box::new(|app: &mut LaApp| {
                        app.ui.settings_dialog = None;
                        app.ui.shortcuts_open = true;
                        app.ui.shortcuts_filter.clear();
                    }));
                }
                ui.add_space(8.0);
                let probe = self.session.is_some();
                let r = ui.add_enabled(probe, Button::tertiary("Probe Colors…"));
                if r.on_disabled_hover_text("Open a device first.").clicked() {
                    after.push(Box::new(|app: &mut LaApp| {
                        app.ui.settings_dialog = None;
                        app.ui.open_probe_editor = true;
                    }));
                }
            }
        }
    }
}
