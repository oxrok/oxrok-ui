//! Export Raw Data dialog (033): ready to export as it
//! opens, the time ranges (all, visible, between markers, a range
//! measurement), CSV / Binary / VCD, the size estimate, and the replace
//! confirmation. The write runs on a background thread.

use super::dialogs::modal_frame;
use super::label_text_edit;
use crate::app::settings::{ExportFormat, ExportRange};
use crate::app::{AppEvent, LaApp};
use crate::core::capture::CaptureState;
use crate::core::{CaptureTime, ChannelId, ChannelKind};
use crate::engine::export::{
    AnalogCsvColumn, CsvColumn, Estimate, ExportFormatKind, SampleRange, TimeColumn, estimate, write_analog_bin,
    write_analog_csv, write_digital_bin, write_digital_csv, write_vcd,
};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{
    BinarySwitch, Button, Checkbox, ChipItem, Dropdown, GridResponse, Segment, channel_grid, segmented,
};
use egui::{Context, Id, Modal, Sense, vec2};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

/// Estimates above this turn into a warning.
const LARGE_EXPORT: u64 = 1 << 30;

/// A named instant offered as an export start or end (`T0`, `P1-A`, …).
type Position = (String, CaptureTime);

pub(crate) struct ExportDialog {
    pub folder: PathBuf,
    pub file_name: String,
    pub included: Vec<ChannelId>,
    pub picking: bool,
    /// Analog Downsample Ratio, as typed.
    pub ratio: String,
    pub format: ExportFormat,
    pub range: ExportRange,
    pub start: usize,
    pub end: usize,
    /// Index into the session's ranges.
    pub measurement: usize,
    pub iso: bool,
    pub always_replace: bool,
    /// The files that exist, waiting for "Replace".
    pub confirm: Option<Vec<PathBuf>>,
}

impl ExportDialog {
    /// A positive integer, or `None` while the text is invalid.
    fn ratio(&self) -> Option<u64> {
        self.ratio.trim().parse::<u64>().ok().filter(|&r| r > 0)
    }

    fn has(&self, kind: ChannelKind) -> bool {
        self.included.iter().any(|c| c.kind == kind)
    }

    /// The files this export writes, in order.
    fn files(&self) -> Vec<PathBuf> {
        let base = self.file_name.trim();
        let at = |suffix: &str| self.folder.join(format!("{base}{suffix}"));
        match self.format {
            ExportFormat::Csv => {
                let mut f = Vec::new();
                if self.has(ChannelKind::Digital) {
                    f.push(at(" digital.csv"));
                }
                if self.has(ChannelKind::Analog) {
                    f.push(at(" analog.csv"));
                }
                f
            }
            ExportFormat::Binary => self.included.iter().map(|c| at(&format!(" {}.bin", c.short_name()))).collect(),
            ExportFormat::Vcd => vec![at(".vcd")],
            ExportFormat::Sigrok => vec![at(".sr")],
        }
    }
}

fn format_label(f: ExportFormat) -> &'static str {
    match f {
        ExportFormat::Csv => "CSV",
        ExportFormat::Binary => tr("Binary"),
        ExportFormat::Vcd => "VCD",
        ExportFormat::Sigrok => "sigrok .sr",
    }
}

fn format_description(f: ExportFormat) -> &'static str {
    match f {
        ExportFormat::Csv => {
            tr("CSV opens in other tools (e.g. spreadsheets) but can't be reloaded here — use File → Save for that.")
        }
        ExportFormat::Binary => tr(
            "One file per channel with a small header: transition times for digital channels, samples for analog ones.",
        ),
        ExportFormat::Vcd => tr("Value Change Dump (IEEE 1364), readable by most waveform viewers and HDL simulators."),
        ExportFormat::Sigrok => tr("A sigrok session file (.sr), readable by PulseView and oxrok."),
    }
}

const FORMATS: [ExportFormat; 4] = [ExportFormat::Csv, ExportFormat::Binary, ExportFormat::Vcd, ExportFormat::Sigrok];
const RANGES: [ExportRange; 4] =
    [ExportRange::All, ExportRange::Visible, ExportRange::Markers, ExportRange::Measurement];

fn range_label(r: ExportRange) -> &'static str {
    match r {
        ExportRange::All => tr("All Time"),
        ExportRange::Visible => tr("Visible Screen"),
        ExportRange::Markers => tr("Between Timing Markers"),
        ExportRange::Measurement => tr("Range Measurement"),
    }
}

/// The default folder (Export Location): the last one used, else
/// the *Default export folder* setting, else Documents.
fn default_folder(app: &LaApp) -> PathBuf {
    app.settings
        .last_dirs
        .export
        .clone()
        .or_else(|| app.settings.files.export_dir.clone())
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `‹tab name› YYYY-MM-DD HH-MM`, without characters file systems reject.
fn default_file_name(tab: &str) -> String {
    let clean: String =
        tab.chars().map(|c| if r#"/\:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    format!("{} {}", clean.trim(), crate::core::format::file_stamp(std::time::SystemTime::now()))
}

impl LaApp {
    /// `Ctrl/Cmd+E`. Only for finished captures (`can_export`).
    pub(crate) fn open_export_dialog(&mut self) {
        self.open_export(None);
    }

    /// *Export Range…*: the dialog with that range measurement.
    pub(crate) fn open_export_for_range(&mut self, range_id: u32) {
        self.open_export(Some(range_id));
    }

    fn open_export(&mut self, range_id: Option<u32>) {
        let Some(s) = self.session.as_ref() else { return };
        if s.state != CaptureState::Finished || !s.has_data() {
            self.toast(Toast::new(ToastKind::Info, tr("Record a capture before exporting.")).id("export-nothing"));
            return;
        }
        // The visible rows' channels, among those with data (FIL-).
        let captured = s.snapshot.as_ref().map(|c| c.enabled.clone()).unwrap_or_default();
        let visible: Vec<ChannelId> = s.visible_rows().map(|r| r.channel).filter(|c| captured.contains(c)).collect();
        let included = self.ui.export_included.clone().unwrap_or(if visible.is_empty() { captured } else { visible });
        let prefs = self.settings.export.clone();
        let measurement = range_id.and_then(|id| s.ranges.items.iter().position(|r| r.id == id));
        let range = match measurement {
            Some(_) => ExportRange::Measurement,
            None => prefs.range,
        };
        let positions = self.export_positions().len();
        let d = ExportDialog {
            folder: default_folder(self),
            file_name: default_file_name(&s.name),
            included,
            picking: false,
            ratio: prefs.downsample.max(1).to_string(),
            format: prefs.format,
            range,
            start: 0,
            end: positions.saturating_sub(1),
            measurement: measurement.unwrap_or(0),
            iso: prefs.iso8601,
            always_replace: false,
            confirm: None,
        };
        self.ui.export = Some(d);
    }

    /// *Between Timing Markers*: `T0` and every marker position, by time.
    fn export_positions(&self) -> Vec<Position> {
        let Some(s) = self.session.as_ref() else { return Vec::new() };
        let mut v: Vec<Position> = vec![("T0".into(), s.origin.resolve(&s.markers, s.trigger))];
        for (r, item) in s.markers.lines() {
            v.push((item.side_name(r.side), item.time(r.side)));
        }
        v.sort_by_key(|p| p.1);
        v
    }

    /// The samples to export for the dialog's time range.
    fn export_range(&self, d: &ExportDialog) -> Option<SampleRange> {
        let s = self.session.as_ref()?;
        let cap = s.capture.as_ref()?.read();
        let rate = cap.sample_rate()?;
        let (b, e) = cap.sample_range()?;
        let sample = |t: CaptureTime| t.to_sample_floor(rate).clamp(b as i128, e as i128) as u64;
        let (a, z) = match d.range {
            ExportRange::All => (b, e),
            ExportRange::Visible => {
                let v = s.view.state();
                (sample(v.left_time), sample(v.right_time()))
            }
            ExportRange::Markers => {
                let p = self.export_positions();
                let (t0, t1) = (p.get(d.start)?.1, p.get(d.end)?.1);
                (sample(t0.min(t1)), sample(t0.max(t1)))
            }
            ExportRange::Measurement => {
                let r = s.ranges.items.get(d.measurement)?;
                (sample(r.t.0.min(r.t.1)), sample(r.t.0.max(r.t.1)))
            }
        };
        (z > a).then_some((a, z))
    }

    fn export_estimate(&self, d: &ExportDialog, range: SampleRange) -> Option<Estimate> {
        let s = self.session.as_ref()?;
        let cap = s.capture.as_ref()?.read();
        let digital: Vec<_> = d.included.iter().filter_map(|c| cap.channel(*c)).collect();
        let analog_n = d.included.iter().filter(|c| c.kind == ChannelKind::Analog).count() as u64;
        let ratio = d.ratio().unwrap_or(1);
        let analog_samples = if analog_n > 0 { (range.1 - range.0).div_ceil(ratio) } else { 0 };
        let kind = match d.format {
            ExportFormat::Csv => ExportFormatKind::Csv,
            ExportFormat::Binary | ExportFormat::Sigrok => ExportFormatKind::Binary,
            ExportFormat::Vcd => ExportFormatKind::Vcd,
        };
        Some(estimate(kind, &digital, analog_samples, analog_n, range))
    }

    pub(crate) fn export_dialog(&mut self, ctx: &Context) {
        let theme = ctx.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        let Some(d) = self.ui.export.as_ref() else { return };
        if d.confirm.is_some() {
            self.export_replace_dialog(ctx);
            return;
        }
        let captured = s.snapshot.as_ref().map(|c| c.enabled.clone()).unwrap_or_default();
        let of_kind = |k: ChannelKind| captured.iter().copied().filter(|c| c.kind == k).collect::<Vec<_>>();
        let (cap_d, cap_a) = (of_kind(ChannelKind::Digital), of_kind(ChannelKind::Analog));
        let chips = |ids: &[ChannelId]| -> Vec<ChipItem> {
            ids.iter()
                .map(|id| match s.row(*id) {
                    Some(r) => super::probe::chip(&theme, s, r, d.included.contains(id)),
                    None => ChipItem::plain(id.index, id.short_name(), id.short_name(), d.included.contains(id)),
                })
                .collect()
        };
        let (items_d, items_a) = (chips(&cap_d), chips(&cap_a));
        let positions = self.export_positions();
        let position_labels: Vec<String> = positions.iter().map(|p| p.0.clone()).collect();
        let ranges: Vec<String> = s
            .ranges
            .items
            .iter()
            .map(|r| {
                format!("{} · {}", r.name, crate::core::format::si_sig((r.t.1 - r.t.0).as_secs_f64().abs(), "s", 3))
            })
            .collect();
        // Range types on offer (markers need two positions, ranges one range).
        let offered: Vec<ExportRange> = RANGES
            .into_iter()
            .filter(|r| match r {
                ExportRange::Markers => positions.len() >= 2,
                ExportRange::Measurement => !ranges.is_empty(),
                _ => true,
            })
            .collect();
        let range_labels: Vec<&str> = offered.iter().map(|r| range_label(*r)).collect();
        let span = self.export_range(d);
        let span_text = span.and_then(|(a, z)| {
            let rate = s.capture.as_ref()?.read().sample_rate()?;
            let origin = s.origin.resolve(&s.markers, s.trigger);
            let (ta, tz) = (CaptureTime::from_sample(a, rate) - origin, CaptureTime::from_sample(z, rate) - origin);
            let dur = crate::core::format::si_sig((tz - ta).as_secs_f64(), "s", 4);
            Some(
                tr("{}, from {} to {}")
                    .replacen("{}", &dur, 1)
                    .replacen("{}", &crate::core::format::time_full(ta), 1)
                    .replacen("{}", &crate::core::format::time_full(tz), 1),
            )
        });
        let est = span.and_then(|r| self.export_estimate(d, r));
        let has_a = d.has(ChannelKind::Analog);
        let files = d.files();
        let rate = s.snapshot.as_ref().map_or(0, |c| c.sample_rate);
        let ratio = d.ratio();
        let sigrok_ok = self.sigrok_export_available();
        let can_export = !d.included.is_empty()
            && !d.file_name.trim().is_empty()
            && span.is_some()
            && (!has_a || ratio.is_some())
            && (d.format != ExportFormat::Sigrok || sigrok_ok);
        let mut ratio_text = d.ratio.clone();
        let mut file_name = d.file_name.clone();
        let mut format_idx = FORMATS.iter().position(|f| *f == d.format).unwrap_or(0);
        let mut range_idx = offered.iter().position(|r| *r == d.range).unwrap_or(0);
        let (mut start, mut end, mut measurement, mut iso, mut always) =
            (d.start, d.end, d.measurement, d.iso, d.always_replace);
        let (folder_text, picking, n_included) = (d.folder.display().to_string(), d.picking, d.included.len());
        let format = d.format;
        let (mut browse, mut export, mut cancel, mut reset) = (false, false, false, false);
        let mut grid_d = GridResponse::default();
        let mut grid_a = GridResponse::default();

        let resp = Modal::new(Id::unique("export_dialog")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(540.0);
            let c = |t| theme.color(t);
            let muted = |t: &str| egui::RichText::new(t).color(c(Token::Ink3));
            ui.label(
                egui::RichText::new(tr("Export Raw Data")).text_style(Text::Display.style()).color(c(Token::Ink1)),
            );
            ui.add_space(20.0);
            // The body scrolls when the dialog would pass 90 % of
            // the window; the footer stays visible.
            let max_body = (ui.ctx().content_rect().height() * 0.9 - 300.0).max(200.0);
            egui::ScrollArea::vertical().max_height(max_body).auto_shrink([false, true]).show(ui, |ui| {
                // Options box: only rows that apply (FIL-).
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, c(Token::Seam)))
                    .corner_radius(4)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        egui::Grid::new("export_options").num_columns(2).spacing([24.0, 12.0]).show(ui, |ui| {
                            ui.label(muted(tr("Time Range"))).on_hover_text(tr("Which part of the capture to export"));
                            ui.vertical(|ui| {
                                ui.set_width(300.0);
                                ui.add(Dropdown::new(tr("Time Range"), &mut range_idx, &range_labels));
                            });
                            ui.end_row();
                            match offered.get(range_idx) {
                                Some(ExportRange::Markers) => {
                                    ui.label(muted(tr("Export Start")));
                                    ui.vertical(|ui| {
                                        ui.set_width(300.0);
                                        ui.add(Dropdown::new(tr("Export Start"), &mut start, &position_labels));
                                    });
                                    ui.end_row();
                                    ui.label(muted(tr("Export End")));
                                    ui.vertical(|ui| {
                                        ui.set_width(300.0);
                                        ui.add(Dropdown::new(tr("Export End"), &mut end, &position_labels));
                                    });
                                    ui.end_row();
                                }
                                Some(ExportRange::Measurement) => {
                                    ui.label(muted(tr("Range")));
                                    ui.vertical(|ui| {
                                        ui.set_width(300.0);
                                        ui.add(Dropdown::new(tr("Range"), &mut measurement, &ranges));
                                    });
                                    ui.end_row();
                                }
                                _ => {}
                            }
                            ui.label("");
                            ui.label(muted(span_text.as_deref().unwrap_or(tr("The range holds no samples."))));
                            ui.end_row();
                            if has_a {
                                ui.label(muted(tr("Analog Downsample Ratio")))
                                    .on_hover_text(tr("Keep every Nth analog sample"));
                                ui.vertical(|ui| {
                                    let r = ui.add(egui::TextEdit::singleline(&mut ratio_text).desired_width(80.0));
                                    label_text_edit(&r, &ratio_text, tr("Analog Downsample Ratio"));
                                    let new_rate = ratio.map_or_else(
                                        || "—".to_owned(),
                                        |r| crate::core::format::sample_rate(rate as f64 / r as f64),
                                    );
                                    ui.label(muted(&tr("New analog sample rate: {}").replacen("{}", &new_rate, 1)));
                                });
                                ui.end_row();
                            }
                            ui.label(muted(tr("Export Format")));
                            ui.vertical(|ui| {
                                let segments: Vec<Segment> = FORMATS
                                    .iter()
                                    .map(|f| Segment {
                                        label: format_label(*f),
                                        enabled: *f != ExportFormat::Sigrok || sigrok_ok,
                                        tooltip: if *f == ExportFormat::Sigrok && !sigrok_ok {
                                            tr("Needs the app built with libsigrok")
                                        } else {
                                            format_description(*f)
                                        },
                                    })
                                    .collect();
                                segmented(ui, "export_format", &segments, &mut format_idx);
                                ui.add_space(4.0);
                                ui.add(egui::Label::new(muted(format_description(FORMATS[format_idx]))).wrap());
                            });
                            ui.end_row();
                            if FORMATS[format_idx] == ExportFormat::Csv {
                                ui.label(muted(tr("Use ISO8601 Timestamps")))
                                    .on_hover_text(tr("Absolute wall-clock times (UTC) instead of seconds from T0"));
                                ui.add(BinarySwitch::new(&mut iso, tr("Use ISO8601 Timestamps")));
                                ui.end_row();
                            }
                        });
                    });
                // A fixed-height row: a bare right-to-left layout takes all
                // the height below it.
                let row = vec2(ui.available_width(), 30.0);
                ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Button::tertiary(tr("Reset to default"))).clicked() {
                        reset = true;
                    }
                });
                ui.add_space(12.0);

                // Export location.
                ui.label(
                    egui::RichText::new(tr("Export Location")).text_style(Text::Subhead.style()).color(c(Token::Ink2)),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(muted(tr("Folder")));
                    let mut path = folder_text.clone();
                    ui.add(
                        egui::TextEdit::singleline(&mut path)
                            .interactive(false)
                            .desired_width(ui.available_width() - 110.0),
                    );
                    let label = if picking { tr("Browsing…") } else { tr("Browse") };
                    if ui.add_enabled(!picking, Button::secondary(label)).clicked() {
                        browse = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(muted(tr("File name")));
                    let r = ui.add(egui::TextEdit::singleline(&mut file_name).desired_width(ui.available_width()));
                    label_text_edit(&r, &file_name, tr("File name"));
                });
                ui.add_space(4.0);
                ui.add(Checkbox::new(&mut always, tr("Always Replace")));
                ui.add_space(16.0);

                // Included channels.
                ui.label(
                    egui::RichText::new(
                        tr("Included Channels  {} of {}").replacen("{}", &n_included.to_string(), 1).replacen(
                            "{}",
                            &captured.len().to_string(),
                            1,
                        ),
                    )
                    .text_style(Text::Subhead.style())
                    .color(c(Token::Ink2)),
                );
                ui.add_space(8.0);
                if !items_d.is_empty() {
                    grid_d = channel_grid(ui, tr("Digital"), items_d);
                    ui.add_space(12.0);
                }
                if !items_a.is_empty() {
                    grid_a = channel_grid(ui, tr("Analog"), items_a);
                }
                ui.add_space(8.0);
            });
            ui.add_space(12.0);

            // Footer.
            let (div, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(div, 0.0, c(Token::Hair));
            ui.add_space(12.0);
            let n = files.len();
            let fmt = format_label(format);
            let text = if n == 1 {
                tr("Export will write 1 {} file:").replacen("{}", fmt, 1)
            } else {
                tr("Export will write {} {} files:").replacen("{}", &n.to_string(), 1).replacen("{}", fmt, 1)
            };
            ui.label(muted(&text));
            let mono = egui::FontId::new(12.0, crate::ui::typography::family(crate::ui::typography::MONO));
            // File names only: the folder is shown above.
            for p in files.iter().take(6) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(egui::RichText::new(name).font(mono.clone()).color(c(Token::Ink2)));
            }
            if files.len() > 6 {
                ui.label(muted(&tr("… and {} more").replacen("{}", &(files.len() - 6).to_string(), 1)));
            }
            if let Some(e) = est {
                let text = tr("≈ {}rows · ~{}")
                    .replacen("{}", &crate::core::format::si_sig(e.rows as f64, "", 2), 1)
                    .replacen("{}", &crate::core::format::si_sig(e.bytes as f64, "B", 2), 1);
                if e.bytes > LARGE_EXPORT {
                    ui.label(
                        egui::RichText::new(tr("{} — This is a large file.").replacen("{}", &text, 1))
                            .color(c(Token::Caution)),
                    );
                } else {
                    ui.label(muted(&text));
                }
            }
            ui.add_space(16.0);
            let row = vec2(ui.available_width(), 40.0);
            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(can_export, Button::primary(tr("✓ Export"))).clicked() {
                    export = true;
                }
                if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                    cancel = true;
                }
            });
            if can_export && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                export = true;
            }
        });

        let d = self.ui.export.as_mut().unwrap();
        d.ratio = ratio_text;
        d.file_name = file_name;
        d.format = FORMATS[format_idx];
        d.range = offered.get(range_idx).copied().unwrap_or(ExportRange::All);
        (d.start, d.end, d.measurement, d.iso, d.always_replace) = (start, end, measurement, iso, always);
        for (grid, ids) in [(&grid_d, &cap_d), (&grid_a, &cap_a)] {
            if let Some(i) = grid.toggled {
                let id = ids[i];
                if d.included.contains(&id) {
                    d.included.retain(|c| *c != id);
                } else {
                    d.included.push(id);
                    d.included.sort();
                }
            }
            if grid.all {
                d.included.retain(|c| !ids.contains(c));
                d.included.extend(ids.iter().copied());
                d.included.sort();
            }
            if grid.clear {
                d.included.retain(|c| !ids.contains(c));
            }
        }
        if reset {
            let defaults = crate::app::settings::ExportPrefs::default();
            d.format = defaults.format;
            d.range = defaults.range;
            d.ratio = defaults.downsample.to_string();
            d.iso = defaults.iso8601;
        }
        if browse {
            d.picking = true;
            let (dialogs, tx, start, ctx) =
                (self.dialogs.clone(), self.events_tx.clone(), Some(d.folder.clone()), ctx.clone());
            std::thread::spawn(move || {
                let picked = dialogs.pick_folder(start.as_deref());
                let _ = tx.send(AppEvent::FolderPicked(picked));
                ctx.request_repaint();
            });
        }
        if export {
            let existing: Vec<PathBuf> = d.files().into_iter().filter(|p| p.exists()).collect();
            if !existing.is_empty() && !d.always_replace {
                d.confirm = Some(existing);
            } else {
                self.finish_export(ctx);
            }
        } else if cancel || (resp.should_close() && !d.picking) {
            self.ui.export = None;
        }
    }

    /// "Replace Files".
    fn export_replace_dialog(&mut self, ctx: &Context) {
        let theme = ctx.la_theme();
        let Some(existing) = self.ui.export.as_ref().and_then(|d| d.confirm.clone()) else { return };
        let folder = existing.first().and_then(|p| p.parent()).map(|p| p.display().to_string()).unwrap_or_default();
        let (mut replace, mut cancel) = (false, false);
        let resp = Modal::new(Id::unique("export_replace")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(
                egui::RichText::new(tr("Replace Files"))
                    .text_style(Text::Display.style())
                    .color(theme.color(Token::Ink1)),
            );
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(tr("These files already exist in {}:").replacen("{}", &folder, 1))
                    .color(theme.color(Token::Ink2)),
            );
            ui.add_space(8.0);
            for p in &existing {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(egui::RichText::new(format!("  {name}")).color(theme.color(Token::Ink2)));
            }
            ui.add_space(20.0);
            let row = vec2(ui.available_width(), 40.0);
            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(Button::destructive(tr("Replace"))).clicked() {
                    replace = true;
                }
                ui.add_space(8.0);
                if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                    cancel = true;
                }
            });
        });
        if replace {
            self.finish_export(ctx);
        } else if (cancel || resp.should_close())
            && let Some(d) = self.ui.export.as_mut()
        {
            d.confirm = None;
        }
    }

    /// Remembers the choices, closes the dialog and runs the export.
    fn finish_export(&mut self, ctx: &Context) {
        let Some(d) = self.ui.export.take() else { return };
        self.settings.export = crate::app::settings::ExportPrefs {
            format: d.format,
            range: d.range,
            downsample: d.ratio().unwrap_or(1),
            iso8601: d.iso,
        };
        self.settings.last_dirs.export = Some(d.folder.clone());
        self.ui.export_included = Some(d.included.clone());
        self.settings_dirty_at.get_or_insert(self.clock.now());
        if let Some(range) = self.export_range(&d) {
            self.run_export(ctx, d, range);
        }
    }

    fn run_export(&mut self, ctx: &Context, d: ExportDialog, range: SampleRange) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(cap) = s.capture.clone() else { return };
        if d.format == ExportFormat::Sigrok {
            self.export_sigrok(ctx, &d, range);
            return;
        }
        let ratio = d.ratio().unwrap_or(1);
        let named = |kind: ChannelKind| -> Vec<(ChannelId, String)> {
            d.included
                .iter()
                .filter(|c| c.kind == kind)
                .map(|id| (*id, s.row(*id).map_or_else(|| id.short_name(), |r| r.name.clone())))
                .collect()
        };
        let (digital, analog) = (named(ChannelKind::Digital), named(ChannelKind::Analog));
        let origin = s.origin.resolve(&s.markers, s.trigger);
        let (format, iso, files) = (d.format, d.iso, d.files());
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        self.ui.exporting += 1;
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || {
                let cancel = AtomicBool::new(false);
                let result = (|| {
                    let data = cap.read();
                    let rate = data.sample_rate().ok_or("no sample rate")?;
                    let start = data.header.as_ref().map(|h| h.start).unwrap_or_default();
                    let time = if iso { TimeColumn::Iso(start) } else { TimeColumn::Relative(origin) };
                    let create =
                        |path: &PathBuf| std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()));
                    let err = |e: crate::engine::export::ExportError| e.to_string();
                    let dcols: Vec<CsvColumn<'_>> = digital
                        .iter()
                        .filter_map(|(id, name)| data.channel(*id).map(|ch| CsvColumn { name, data: ch }))
                        .collect();
                    let acols: Vec<AnalogCsvColumn<'_>> = analog
                        .iter()
                        .filter_map(|(id, name)| data.analog(*id).map(|ch| AnalogCsvColumn { name, data: ch }))
                        .collect();
                    let mut rows = 0u64;
                    let mut next = files.iter();
                    match format {
                        ExportFormat::Csv => {
                            if !dcols.is_empty() {
                                let p = next.next().ok_or("no file")?;
                                rows +=
                                    write_digital_csv(create(p)?, &dcols, rate, time, range, &cancel).map_err(err)?;
                            }
                            if !acols.is_empty() {
                                let p = next.next().ok_or("no file")?;
                                rows += write_analog_csv(create(p)?, &acols, rate, time, range, ratio, &cancel)
                                    .map_err(err)?;
                            }
                        }
                        ExportFormat::Binary => {
                            // Files follow `included` order: digital then analog by id.
                            for (id, _) in digital.iter().chain(&analog) {
                                let p = files
                                    .iter()
                                    .find(|f| f.to_string_lossy().ends_with(&format!(" {}.bin", id.short_name())))
                                    .ok_or("no file")?;
                                rows += match id.kind {
                                    ChannelKind::Digital => {
                                        let ch = data.channel(*id).ok_or("no data")?;
                                        write_digital_bin(create(p)?, ch, rate, origin, range, &cancel).map_err(err)?
                                    }
                                    ChannelKind::Analog => {
                                        let ch = data.analog(*id).ok_or("no data")?;
                                        write_analog_bin(create(p)?, ch, rate, origin, range, ratio, &cancel)
                                            .map_err(err)?
                                    }
                                    ChannelKind::Bus => 0,
                                };
                            }
                        }
                        ExportFormat::Vcd => {
                            let p = next.next().ok_or("no file")?;
                            rows += write_vcd(create(p)?, &dcols, &acols, rate, range, ratio, &cancel).map_err(err)?;
                        }
                        ExportFormat::Sigrok => unreachable!("handled above"),
                    }
                    let last = files.last().cloned().ok_or("no file")?;
                    Ok::<_, String>((last, rows))
                })();
                let _ = tx.send(AppEvent::ExportDone(result));
                ctx.request_repaint();
            })
            .expect("spawn export thread");
    }

    /// Sigrok `.sr` export needs libsigrok.
    pub(crate) fn sigrok_export_available(&self) -> bool {
        self.backend.can_save_sr()
    }

    fn export_sigrok(&mut self, ctx: &Context, d: &ExportDialog, range: SampleRange) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(cap) = s.capture.clone() else { return };
        let named = |kind: ChannelKind| -> Vec<(ChannelId, String)> {
            d.included
                .iter()
                .filter(|c| c.kind == kind)
                .map(|id| (*id, s.row(*id).map_or_else(|| id.short_name(), |r| r.name.clone())))
                .collect()
        };
        let source =
            CaptureExport { cap, range, digital: named(ChannelKind::Digital), analog: named(ChannelKind::Analog) };
        let Some(path) = d.files().into_iter().next() else { return };
        let (backend, tx, ctx) = (self.backend.clone(), self.events_tx.clone(), ctx.clone());
        self.ui.exporting += 1;
        std::thread::Builder::new()
            .name("export-sr".into())
            .spawn(move || {
                let samples = range.1 - range.0;
                let r = backend
                    .save_sr_file(&path, &source, Box::new(|_| {}))
                    .map(|()| (path.clone(), samples))
                    .map_err(|e| format!("{}: {e}", path.display()));
                let _ = tx.send(AppEvent::ExportDone(r));
                ctx.request_repaint();
            })
            .expect("spawn sr export thread");
    }
}

/// The capture as libsigrok's export reads it.
struct CaptureExport {
    cap: crate::app::capture::SharedCapture,
    range: SampleRange,
    digital: Vec<(ChannelId, String)>,
    analog: Vec<(ChannelId, String)>,
}

impl crate::backend::ExportSource for CaptureExport {
    fn sample_rate(&self) -> u64 {
        self.cap.read().sample_rate().unwrap_or(0)
    }

    fn range(&self) -> (u64, u64) {
        self.range
    }

    fn digital(&self) -> Vec<(ChannelId, String)> {
        self.digital.clone()
    }

    fn analog(&self) -> Vec<(ChannelId, String)> {
        self.analog.clone()
    }

    /// Fills each channel's bit run by run between its transitions.
    fn logic(&self, first: u64, n: usize, unit_size: usize, out: &mut Vec<u8>) {
        out.clear();
        out.resize(n * unit_size, 0);
        let data = self.cap.read();
        let end = first + n as u64;
        for (k, (id, _)) in self.digital.iter().enumerate() {
            let Some(ch) = data.channel(*id) else { continue };
            let (byte, bit) = (k / 8, 1u8 << (k % 8));
            let mut level = ch.level_at(first).unwrap_or(false);
            let mut fill = |from: u64, to: u64, on: bool| {
                if on {
                    for s in from..to {
                        out[(s - first) as usize * unit_size + byte] |= bit;
                    }
                }
            };
            let mut at = first;
            for t in ch.transitions_from(first + 1).take_while(|&t| t < end) {
                fill(at, t, level);
                level = !level;
                at = t;
            }
            fill(at, end, level);
        }
    }

    fn analog_samples(&self, id: ChannelId, first: u64, n: usize, out: &mut Vec<f32>) {
        let data = self.cap.read();
        out.clear();
        if let Some(ch) = data.analog(id) {
            out.extend(ch.samples(first, first + n as u64).map(|(_, v)| v));
        }
        out.resize(n, f32::NAN);
    }
}
