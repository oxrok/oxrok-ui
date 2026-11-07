//! The Data section of the Analyzers panel (13): the data table and the
//! terminal.

use super::markers::c32;
use crate::app::LaApp;
use crate::core::CaptureTime;
use crate::core::format::si_sig;
use crate::decode::{Radix, all_rows};
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use egui::{Align2, Color32, FontId, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};
use std::collections::HashMap;
use std::time::Duration;

/// Geometry.
const ROW_H: f32 = 28.0;
const HEADER_H: f32 = 36.0;
/// Search debounce.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// Outline duration.
const FLASH: Duration = Duration::from_secs(2);
/// Copy limit.
const COPY_MAX: usize = 10_000;
/// Scrollback.
const TERMINAL_LINES: usize = 10_000;
/// Columns after the color square, with default widths.
const COLUMNS: [(&str, f32); 5] =
    [("Type", 90.0), ("Start", 100.0), ("Duration", 80.0), ("Value", 70.0), ("Text", 160.0)];

/// A line icon painter.
type Icon = fn(&egui::Painter, Rect, Color32);

/// Which view the Data section shows.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum DataMode {
    #[default]
    Table,
    Terminal,
}

/// One table row: an analyzer and one of its frames.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowRef {
    pub analyzer: u32,
    pub frame: u32,
    pub start: u64,
}

/// The merged and filtered rows, kept up to date incrementally.
#[derive(Clone, Debug, Default)]
pub(crate) struct TableCache {
    /// Analyzer id → frames already merged.
    seen: HashMap<u32, usize>,
    /// The analyzers (order, table flag, rows, radix) it was built for.
    key: Vec<(u32, bool, Vec<String>, Radix)>,
    pub rows: Vec<RowRef>,
    /// Positions in `rows` matching the applied query.
    pub filtered: Vec<u32>,
    /// The applied query.
    pub query: String,
    /// The query as typed, and when it last changed.
    pub typed: String,
    pub typed_at: Option<Duration>,
    /// A selected range of visible positions, and where it started.
    pub selected: Option<(usize, usize)>,
    pub anchor: Option<usize>,
    /// The row to outline, and since when.
    pub flash: Option<(RowRef, Duration)>,
    pub scroll_to: Option<usize>,
    /// Searchable columns the user unchecked (Type, Value, Text).
    pub unsearched: [bool; 3],
    /// The query must be re-applied (the searchable columns changed).
    pub stale: bool,
    /// The horizontal scroll offset, shared by the header and cells.
    pub h_scroll: f32,
    /// The row count last frame, and whether the user scrolled away
    /// from the bottom (then new rows don't pull the view down).
    pub last_len: usize,
    pub detached: bool,
}

/// The searchable columns: their index in [`COLUMNS`].
const SEARCHABLE: [usize; 3] = [0, 3, 4];

/// The Export Analyzer Data dialog.
#[derive(Clone, Debug)]
pub(crate) struct TableExport {
    /// Every analyzer: id, name, checked.
    pub analyzers: Vec<(u32, String, bool)>,
    pub apply_search: bool,
    pub selected_only: bool,
}

/// The character ranges of `text` matching the lowercase query
/// `q`; the whole text when it equals one of the radix `forms`.
pub(crate) fn match_ranges(text: &str, q: &str, forms: &[String]) -> Vec<std::ops::Range<usize>> {
    if q.is_empty() {
        return Vec::new();
    }
    let n = text.chars().count();
    if forms.iter().any(|f| f == text) {
        return std::iter::once(0..n).collect();
    }
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let ql: Vec<char> = q.chars().collect();
    if lower.len() != n {
        // Lowercasing changed the length: mark the whole cell.
        return if text.to_lowercase().contains(q) { std::iter::once(0..n).collect() } else { Vec::new() };
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + ql.len() <= lower.len() {
        if lower[i..i + ql.len()] == ql[..] {
            out.push(i..i + ql.len());
            i += ql.len();
        } else {
            i += 1;
        }
    }
    out
}

/// A row's cells.
pub(crate) struct Cells {
    pub kind: String,
    pub start: String,
    pub duration: String,
    pub value: String,
    pub text: String,
}

impl Cells {
    fn all(&self) -> [&str; 5] {
        [&self.kind, &self.start, &self.duration, &self.value, &self.text]
    }
}

/// A numeric query in the forms an analyzer's radix shows, so
/// `0x6C` finds `l` with Ascii and `108` with Dec.
pub(crate) fn query_forms(q: &str, radix: Radix) -> Vec<String> {
    let q = q.trim();
    let v = if let Some(h) = q.strip_prefix("0x").or_else(|| q.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else if let Some(b) = q.strip_prefix("0b") {
        u64::from_str_radix(b, 2).ok()
    } else {
        None
    };
    let Some(v) = v.filter(|v| *v < 256) else { return Vec::new() };
    vec![match radix {
        Radix::Hex | Radix::Native => format!("{v:02X}"),
        Radix::Dec => v.to_string(),
        Radix::Bin => format!("{v:08b}"),
        Radix::Ascii => match v {
            0x20..=0x7E => (v as u8 as char).to_string(),
            _ => format!("[{v:02X}]"),
        },
    }]
}

/// The byte of a UART data frame, back from its formatted text.
pub(crate) fn uart_byte(text: &str, radix: Radix) -> Option<u8> {
    match radix {
        Radix::Ascii => {
            if let Some(h) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
                u8::from_str_radix(h, 16).ok()
            } else {
                let mut c = text.chars();
                let ch = c.next()?;
                (c.next().is_none() && ch.is_ascii()).then_some(ch as u8)
            }
        }
        Radix::Hex => u8::from_str_radix(text.trim_start_matches("0x"), 16).ok(),
        // The decoder's default format (UART: hex).
        Radix::Native => u8::from_str_radix(text, 16).ok(),
        Radix::Dec => text.parse().ok(),
        Radix::Bin => u8::from_str_radix(text, 2).ok(),
    }
}

/// Terminal text of one byte: CR, LF and CR+LF as newlines (the
/// caller merges CR+LF), other control characters in caret notation.
fn terminal_char(b: u8) -> String {
    match b {
        b'\n' | b'\r' => "\n".into(),
        0x00..=0x1F => format!("^{}", (b + 0x40) as char),
        0x7F => "^?".into(),
        0x20..=0x7E => (b as char).to_string(),
        _ => char::from(b).to_string(),
    }
}

impl LaApp {
    fn table_key(&self) -> Vec<(u32, bool, Vec<String>, Radix)> {
        self.session.as_ref().map_or(Vec::new(), |s| {
            s.analyzers.iter().map(|a| (a.id, a.show_in_table, a.rows_visible.clone(), a.radix)).collect()
        })
    }

    /// 024: merges new frames; rebuilds when analyzers change.
    fn refresh_table(&mut self) {
        let key = self.table_key();
        let list = self.decoder_list();
        let Some(s) = self.session.as_ref() else { return };
        let c = &mut self.ui.table;
        let reset = s.analyzers.iter().any(|a| a.out.frames.read().len() < c.seen.get(&a.id).copied().unwrap_or(0));
        if c.key != key || reset {
            c.key = key;
            c.seen.clear();
            c.rows.clear();
            c.filtered.clear();
            c.query.clear();
            c.selected = None;
        }
        let mut new: Vec<RowRef> = Vec::new();
        for a in s.analyzers.iter().filter(|a| a.show_in_table) {
            let Some(info) = list.iter().find(|d| d.id == a.decoder) else { continue };
            let rows = all_rows(info);
            let f = a.out.frames.read();
            let from = c.seen.get(&a.id).copied().unwrap_or(0);
            for id in from..f.len() {
                let fr = f.get(id as u32).unwrap();
                if rows.get(fr.row).is_some_and(|r| a.rows_visible.contains(&r.0)) {
                    new.push(RowRef { analyzer: a.id, frame: id as u32, start: fr.start });
                }
            }
            c.seen.insert(a.id, f.len());
        }
        if new.is_empty() {
            return;
        }
        let order: HashMap<u32, usize> = s.analyzers.iter().enumerate().map(|(k, a)| (a.id, k)).collect();
        let key_of = |r: &RowRef| (r.start, order.get(&r.analyzer).copied().unwrap_or(0), r.frame);
        new.sort_by_key(key_of);
        let appended = c.rows.last().is_none_or(|l| key_of(l) <= key_of(&new[0]));
        let first_new = c.rows.len();
        c.rows.extend(new);
        if !appended {
            c.rows.sort_by_key(key_of);
        }
        // Keep an applied query's index current.
        if !self.ui.table.query.is_empty() {
            let q = self.ui.table.query.clone();
            let from = if appended { first_new } else { 0 };
            let matches: Vec<u32> =
                (from..self.ui.table.rows.len()).filter(|&k| self.row_matches(k, &q)).map(|k| k as u32).collect();
            if appended {
                self.ui.table.filtered.extend(matches);
            } else {
                self.ui.table.filtered = matches;
            }
        }
    }

    /// The cells of a row.
    pub(crate) fn cells(&self, r: RowRef) -> Option<Cells> {
        let s = self.session.as_ref()?;
        let a = s.analyzers.iter().find(|a| a.id == r.analyzer)?;
        let list = self.decoder_list();
        let info = list.iter().find(|d| d.id == a.decoder)?;
        let f = a.out.frames.read();
        let fr = f.get(r.frame)?;
        let rate = s.capture.as_ref()?.read().sample_rate()?;
        let t = |n: u64| CaptureTime::from_sample(n, rate);
        Some(Cells {
            kind: info.annotations.get(fr.class).map_or(String::new(), |c| c.0.clone()),
            start: si_sig((t(fr.start) - s.t0()).as_secs_f64(), "s", 5),
            duration: si_sig((t(fr.end) - t(fr.start)).as_secs_f64(), "s", 5),
            value: fr.shortest().to_owned(),
            text: fr.longest().to_owned(),
        })
    }

    /// case-insensitive substring on the searchable cells (not
    /// Start or Duration), plus the radix forms of a numeric query.
    fn row_matches(&self, k: usize, q: &str) -> bool {
        let Some(&r) = self.ui.table.rows.get(k) else { return false };
        let Some(c) = self.cells(r) else { return false };
        let ql = q.to_lowercase();
        let radix = self
            .session
            .as_ref()
            .and_then(|s| s.analyzers.iter().find(|a| a.id == r.analyzer))
            .map_or(Radix::Hex, |a| a.radix);
        let forms = query_forms(q, radix);
        let hidden = self.ui.table.unsearched;
        [&c.kind, &c.value, &c.text]
            .iter()
            .zip(hidden)
            .filter(|(_, hidden)| !hidden)
            .any(|(cell, _)| cell.to_lowercase().contains(&ql) || forms.iter().any(|f| *cell == f))
    }

    /// Applies the typed query once it has settled.
    fn apply_search(&mut self) {
        let now = self.now();
        let c = &self.ui.table;
        let settled = c.typed != c.query && c.typed_at.is_none_or(|t| now.saturating_sub(t) >= SEARCH_DEBOUNCE);
        if !settled && !c.stale {
            return;
        }
        let q = if c.stale && !settled { c.query.clone() } else { c.typed.clone() };
        let filtered: Vec<u32> = if q.is_empty() {
            Vec::new()
        } else {
            (0..c.rows.len()).filter(|&k| self.row_matches(k, &q)).map(|k| k as u32).collect()
        };
        let c = &mut self.ui.table;
        c.filtered = filtered;
        c.query = q;
        c.stale = false;
        c.selected = None;
    }

    /// The number of visible rows (all, or the matches).
    pub(crate) fn table_len(&self) -> usize {
        let c = &self.ui.table;
        if c.query.is_empty() { c.rows.len() } else { c.filtered.len() }
    }

    pub(crate) fn table_row(&self, k: usize) -> Option<RowRef> {
        let c = &self.ui.table;
        if c.query.is_empty() {
            c.rows.get(k).copied()
        } else {
            c.filtered.get(k).and_then(|&i| c.rows.get(i as usize).copied())
        }
    }

    /// A frame's sample span.
    fn frame_span(&self, r: RowRef) -> Option<(CaptureTime, CaptureTime)> {
        let s = self.session.as_ref()?;
        let a = s.analyzers.iter().find(|a| a.id == r.analyzer)?;
        let rate = s.capture.as_ref()?.read().sample_rate()?;
        let f = a.out.frames.read();
        let fr = f.get(r.frame)?;
        Some((CaptureTime::from_sample(fr.start, rate), CaptureTime::from_sample(fr.end, rate)))
    }

    /// Ctrl/Cmd+click on a bubble: the frame's row in the table.
    pub(crate) fn reveal_in_table(&mut self, analyzer: u32, frame: u32) {
        self.settings.inspector.tab = super::analyzers::ANALYZERS_PANEL.into();
        self.settings.inspector.open = true;
        self.ui.data_mode = DataMode::Table;
        self.refresh_table();
        let c = &self.ui.table;
        let Some(pos) = c.rows.iter().position(|r| r.analyzer == analyzer && r.frame == frame) else {
            self.toast(Toast::new(ToastKind::Info, "This result isn't in the table yet; it's still being processed"));
            return;
        };
        let visible = if c.query.is_empty() { Some(pos) } else { c.filtered.iter().position(|&i| i as usize == pos) };
        let Some(k) = visible else {
            self.toast(Toast::new(ToastKind::Info, "This result doesn't match the current table search"));
            return;
        };
        let r = c.rows[pos];
        let now = self.now();
        let c = &mut self.ui.table;
        c.selected = Some((k, k));
        c.anchor = Some(k);
        c.flash = Some((r, now));
        c.scroll_to = Some(k);
    }

    /// The selected rows as CSV (visible columns, header line).
    fn copy_selection(&mut self, ui: &Ui) {
        let Some((a, b)) = self.ui.table.selected else { return };
        let busy = self.session.as_ref().is_some_and(|s| {
            s.analyzers.iter().any(|x| *x.out.status.lock() == crate::decode::runner::Status::Running)
        });
        if busy {
            let t = Toast::new(ToastKind::Info, "Copy will be available after processing completes")
                .lifetime(Duration::from_secs(5));
            self.toast(t);
            return;
        }
        let quote = |s: &str| {
            if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
        };
        let mut out = vec![COLUMNS.iter().map(|c| c.0).collect::<Vec<_>>().join(",")];
        for k in a..=b.min(a + COPY_MAX - 1) {
            if let Some(c) = self.table_row(k).and_then(|r| self.cells(r)) {
                out.push(c.all().iter().map(|s| quote(s)).collect::<Vec<_>>().join(","));
            }
        }
        ui.ctx().copy_text(out.join("\n"));
    }

    // ---- the section ----

    pub(crate) fn data_section(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        if self.session.as_ref().is_none_or(|s| s.analyzers.is_empty()) {
            return;
        }
        self.refresh_table();
        self.apply_search();
        ui.add_space(8.0);
        let top = ui.cursor().left_top();
        ui.painter()
            .line_segment([top, top + vec2(ui.available_width(), 0.0)], Stroke::new(1.0, theme.color(Token::Seam)));
        ui.horizontal(|ui| {
            ui.set_height(40.0);
            ui.add_space(16.0);
            ui.label(egui::RichText::new("Data").size(17.0).color(theme.color(Token::Ink1)));
            let tip = if self.ui.data_mode == DataMode::Terminal {
                "Results are not in time order when more than one analyzer streams to the terminal."
            } else {
                "The decoded results of every analyzer, in time order."
            };
            super::panels::help_icon(ui, tip);
            // Aggregated progress.
            let s = self.session.as_ref().unwrap();
            let running: Vec<f32> = s
                .analyzers
                .iter()
                .filter(|a| *a.out.status.lock() == crate::decode::runner::Status::Running)
                .map(|a| *a.out.progress.lock())
                .collect();
            if !running.is_empty() {
                let p = running.iter().sum::<f32>() / running.len() as f32;
                let (r, _) = ui.allocate_exact_size(vec2(60.0, 4.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, theme.color(Token::Hair));
                ui.painter().rect_filled(
                    Rect::from_min_size(r.min, vec2(60.0 * p, 4.0)),
                    2.0,
                    theme.color(Token::Good),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                let modes: [(DataMode, &str, Icon); 2] =
                    [(DataMode::Terminal, "Terminal", icons::terminal), (DataMode::Table, "Data Table", icons::grid)];
                for (mode, label, icon) in modes {
                    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    let on = self.ui.data_mode == mode;
                    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, on, label));
                    let c = if on { Token::Ink1 } else { Token::Ink4 };
                    icon(ui.painter(), r.shrink(5.0), theme.color(c));
                    if resp.on_hover_text(label).clicked() {
                        self.ui.data_mode = mode;
                    }
                }
            });
        });
        match self.ui.data_mode {
            DataMode::Table => self.table_ui(ui),
            DataMode::Terminal => self.terminal_ui(ui),
        }
    }

    // ---- the table ----

    fn table_ui(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let now = self.now();
        // The toolbar: search, "N matches", Filter, Export Table.
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.add_space(16.0);
            let mut typed = self.ui.table.typed.clone();
            let matches = (!self.ui.table.query.is_empty()).then_some(self.ui.table.filtered.len());
            let matches_w = if matches.is_some() { 84.0 } else { 0.0 };
            let r = ui.add(
                egui::TextEdit::singleline(&mut typed)
                    .hint_text("Search results…")
                    .desired_width(ui.available_width() - 16.0 - 2.0 * 32.0 - matches_w),
            );
            super::label_text_edit(&r, &typed, "Search table");
            if std::mem::take(&mut self.ui.table_focus) {
                r.request_focus();
            }
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                typed.clear();
            }
            if typed != self.ui.table.typed {
                self.ui.table.typed = typed;
                self.ui.table.typed_at = Some(now);
                ui.ctx().request_repaint_after(SEARCH_DEBOUNCE);
            }
            if let Some(n) = matches {
                let text = if n == 1 { "1 match".to_owned() } else { format!("{n} matches") };
                ui.label(egui::RichText::new(text).color(theme.color(Token::Ink4)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                self.export_table_button(ui);
                self.filter_button(ui);
            });
        });
        ui.add_space(6.0);
        // The header and the cells share one horizontal offset. The
        // Inspector's scroll scrolls the rows vertically with the
        // rest of the tab, so the panel header scrolls away like the Device
        // tab's (culls the rows the same way `show_rows` did).
        let total_w = 18.0 + COLUMNS.iter().map(|c| c.1).sum::<f32>();
        let width = ui.available_width();
        let max_h = (total_w - width).max(0.0);
        let n = self.table_len();
        let (hr, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
        let (sbr, sb) =
            ui.allocate_exact_size(vec2(width, if max_h > 0.0 { 8.0 } else { 0.0 }), Sense::click_and_drag());
        let (rows, _) = ui.allocate_exact_size(vec2(width, n as f32 * ROW_H), Sense::hover());
        let mut h = self.ui.table.h_scroll.clamp(0.0, max_h);
        if max_h > 0.0 {
            let thumb_w = (width * (width / total_w)).clamp(24.0, width);
            // A trackpad's horizontal swipe (or Shift+wheel) over the table.
            if ui.rect_contains_pointer(hr.union(rows)) {
                let dx = ui.input(|i| i.smooth_scroll_delta.x);
                if dx != 0.0 {
                    h = (h - dx).clamp(0.0, max_h);
                }
            }
            if sb.dragged() && width > thumb_w {
                h = (h + sb.drag_delta().x / (width - thumb_w) * max_h).clamp(0.0, max_h);
            } else if sb.clicked()
                && let Some(p) = sb.interact_pointer_pos()
                && width > thumb_w
            {
                h = ((p.x - sbr.left() - thumb_w / 2.0) / (width - thumb_w)).clamp(0.0, 1.0) * max_h;
            }
        }
        self.ui.table.h_scroll = h;
        let hp = ui.painter().with_clip_rect(hr);
        hp.rect_filled(hr, 0.0, theme.color(Token::Card));
        let mut x = hr.left() + 18.0 - h;
        for (name, w) in COLUMNS {
            let font = FontId::new(13.0, Text::Lead.font_id().family);
            hp.text(pos2(x + 6.0, hr.center().y), Align2::LEFT_CENTER, name, font, theme.color(Token::Ink1));
            x += w;
        }
        // The horizontal scrollbar, under the header row.
        if max_h > 0.0 {
            let track = sbr.shrink2(vec2(0.0, 2.0));
            ui.painter().rect_filled(track, 0.0, theme.color(Token::Hair));
            let thumb_w = (width * (width / total_w)).clamp(24.0, width);
            let thumb = Rect::from_min_size(
                pos2(sbr.left() + (width - thumb_w) * (h / max_h), track.top()),
                vec2(thumb_w, track.height()),
            );
            let on = sb.hovered() || sb.dragged();
            ui.painter().rect_filled(thumb, 2.0, theme.color(if on { Token::Ink4 } else { Token::Seam }));
        }
        let flash = self.ui.table.flash.filter(|(_, t)| now.saturating_sub(*t) < FLASH);
        if flash.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let selected = self.ui.table.selected;
        let query = self.ui.table.query.to_lowercase();
        let raw_query = self.ui.table.query.clone();
        let unsearched = self.ui.table.unsearched;
        let radixes: HashMap<u32, Radix> =
            self.session.as_ref().map_or(HashMap::new(), |s| s.analyzers.iter().map(|a| (a.id, a.radix)).collect());
        let radix_of = |id: u32| radixes.get(&id).copied().unwrap_or(Radix::Hex);
        let mut acts: Vec<(usize, Act)> = Vec::new();
        // Only the rows inside the panel's viewport are laid out.
        let visible = ui.clip_rect().intersect(rows);
        let (first, last) = if visible.is_positive() {
            let first = (((visible.top() - rows.top()) / ROW_H).floor().max(0.0) as usize).min(n);
            let last = ((((visible.bottom() - rows.top()) / ROW_H).ceil() as usize) + 1).clamp(first, n);
            (first, last)
        } else {
            (0, 0)
        };
        let scroll_to = self.ui.table.scroll_to.take();
        // New rows follow the bottom only while it is in view.
        let stick =
            scroll_to.is_none() && self.ui.table.last_len > 0 && !self.ui.table.detached && n > self.ui.table.last_len;
        for k in first..last {
            let Some(r) = self.table_row(k) else { continue };
            let rect = Rect::from_min_size(pos2(rows.left(), rows.top() + k as f32 * ROW_H), vec2(width, ROW_H));
            let resp = ui.interact(rect, ui.make_persistent_id(("table_row", k)), Sense::click());
            let sel = selected.is_some_and(|(a, b)| (a..=b).contains(&k));
            let bg = match (sel, k % 2) {
                (true, _) => Token::Lift,
                (false, 0) => Token::Deck,
                _ => Token::Card,
            };
            let p = ui.painter();
            p.rect_filled(rect, 0.0, theme.color(bg));
            let Some(cells) = self.cells(r) else {
                let at = rect.left_center() + vec2(24.0 - h, 0.0);
                p.text(at, Align2::LEFT_CENTER, "Loading", Text::Prose.font_id(), theme.color(Token::Ink4));
                continue;
            };
            let color = self
                .session
                .as_ref()
                .and_then(|s| s.analyzers.iter().find(|a| a.id == r.analyzer))
                .map_or([128; 3], |a| a.color);
            p.rect_filled(
                Rect::from_center_size(pos2(rect.left() + 9.0 - h, rect.center().y), vec2(6.0, 6.0)),
                0.0,
                c32(color),
            );
            let mut x = rect.left() + 18.0 - h;
            for (col, ((_, w), t)) in COLUMNS.iter().zip(cells.all()).enumerate() {
                let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, ROW_H));
                let galley = p.layout_no_wrap(t.to_owned(), Text::Prose.font_id(), theme.color(Token::Ink2));
                let at = pos2(x + 6.0, rect.center().y - galley.size().y / 2.0);
                // The matching substrings, and a left border on
                // cells with a match; only searchable columns.
                let searchable = SEARCHABLE.iter().position(|&c| c == col).is_some_and(|i| !unsearched[i]);
                let ranges = if searchable {
                    match_ranges(t, &query, &query_forms(&raw_query, radix_of(r.analyzer)))
                } else {
                    Vec::new()
                };
                let clip = p.with_clip_rect(cell);
                for m in &ranges {
                    let x0 = galley.pos_from_cursor(egui::text::CCursor::new(m.start)).min.x;
                    let x1 = galley.pos_from_cursor(egui::text::CCursor::new(m.end)).min.x;
                    let hl = Rect::from_x_y_ranges(at.x + x0..=at.x + x1, rect.top() + 5.0..=rect.bottom() - 5.0);
                    clip.rect_filled(hl, 2.0, theme.color(Token::Caution).gamma_multiply(0.4));
                }
                if !ranges.is_empty() {
                    p.line_segment(
                        [cell.left_top(), cell.left_bottom()],
                        Stroke::new(1.0, theme.color(Token::Caution)),
                    );
                }
                clip.galley(at, galley, theme.color(Token::Ink2));
                x += w;
            }
            if flash.is_some_and(|(f, _)| f == r) {
                p.rect_stroke(
                    rect.shrink(1.0),
                    0.0,
                    Stroke::new(2.0, theme.color(Token::Flare)),
                    egui::StrokeKind::Inside,
                );
            }
            let label = format!("{} {} {}", cells.kind, cells.value, cells.start);
            resp.widget_info(|| WidgetInfo::selected(Role::Row, true, sel, &label));
            // The go-to arrow, pinned to the visible right edge.
            if ui.rect_contains_pointer(rect) {
                let g = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(18.0, 18.0));
                let gr = ui.interact(g, ui.make_persistent_id(("table_goto", r.analyzer, r.frame)), Sense::click());
                gr.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Go to data"));
                icons::goto_arrow(ui.painter(), g, theme.color(Token::Ink2));
                let gr = gr.on_hover_text("Go to data / Double click to zoom");
                if gr.double_clicked() {
                    acts.push((k, Act::Zoom));
                } else if gr.clicked() {
                    acts.push((k, Act::Goto));
                }
            }
            if resp.clicked() {
                acts.push((k, if ui.input(|i| i.modifiers.shift) { Act::Extend } else { Act::Select }));
            }
        }
        // Reveal a row, or keep the newest rows in view at the bottom.
        if let Some(k) = scroll_to {
            let r = Rect::from_min_size(pos2(rows.left(), rows.top() + k as f32 * ROW_H), vec2(width, ROW_H));
            ui.scroll_to_rect(r, Some(egui::Align::Center));
        } else if stick && n > 0 {
            ui.scroll_to_rect(rows, Some(egui::Align::BOTTOM));
        }
        self.ui.table.last_len = n;
        self.ui.table.detached = !(visible.is_positive() && visible.bottom() >= rows.bottom() - 1.0);
        // Ctrl/Cmd+C.
        let copy = ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
                || i.events.iter().any(|e| matches!(e, egui::Event::Copy))
        });
        if copy && ui.memory(|m| m.focused().is_none()) {
            self.copy_selection(ui);
        }
        for (k, act) in acts {
            let Some(r) = self.table_row(k) else { continue };
            match act {
                Act::Select => {
                    self.ui.table.selected = Some((k, k));
                    self.ui.table.anchor = Some(k);
                }
                Act::Extend => {
                    let a = self.ui.table.anchor.unwrap_or(k);
                    self.ui.table.selected = Some((a.min(k), a.max(k)));
                }
                Act::Goto | Act::Zoom => {
                    let Some((a, b)) = self.frame_span(r) else { continue };
                    self.ui.table.flash = Some((r, self.now()));
                    if act == Act::Zoom {
                        self.zoom_on(a, b);
                    } else {
                        self.center_on(a + CaptureTime::from_secs_f64((b - a).as_secs_f64() / 2.0));
                    }
                }
            }
        }
    }

    /// The Filter button, with a count badge, and its popover.
    fn filter_button(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        let hidden: Vec<String> = s.analyzers.iter().filter(|a| !a.show_in_table).map(|a| a.name.clone()).collect();
        let unsearched: Vec<&str> = SEARCHABLE
            .iter()
            .zip(self.ui.table.unsearched)
            .filter(|(_, off)| *off)
            .map(|(&c, _)| COLUMNS[c].0)
            .collect();
        let count = hidden.len() + unsearched.len();
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Filter"));
        let c = if resp.hovered() || count > 0 { Token::Ink1 } else { Token::Ink3 };
        icons::funnel(ui.painter(), r.shrink(6.0), theme.color(c));
        if count > 0 {
            let b = pos2(r.right() - 5.0, r.top() + 6.0);
            ui.painter().circle_filled(b, 6.0, theme.color(Token::Mark));
            ui.painter().text(
                b,
                Align2::CENTER_CENTER,
                count.to_string(),
                FontId::proportional(9.0),
                theme.color(Token::InkOnFlare),
            );
        }
        let tip = if count == 0 {
            "Filter".to_owned()
        } else {
            let mut parts = Vec::new();
            if !hidden.is_empty() {
                parts.push(format!("Hidden: {}", hidden.join(", ")));
            }
            if !unsearched.is_empty() {
                parts.push(format!("Not searched: {}", unsearched.join(", ")));
            }
            parts.join("\n")
        };
        let resp = resp.on_hover_text(tip);
        let mut show: Vec<(u32, String, bool)> =
            s.analyzers.iter().map(|a| (a.id, a.name.clone(), a.show_in_table)).collect();
        let mut cols = self.ui.table.unsearched.map(|off| !off);
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(crate::ui::widgets::popover_frame(ui))
            .width(360.0)
            .show(|ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.columns(2, |cols_ui| {
                        let head = |ui: &mut Ui, title: &str| {
                            ui.label(
                                egui::RichText::new(title)
                                    .text_style(Text::Lead.style())
                                    .color(theme.color(Token::Ink1)),
                            );
                        };
                        let all_none = |ui: &mut Ui| -> Option<bool> {
                            let mut out = None;
                            ui.horizontal(|ui| {
                                if ui.add(crate::ui::widgets::Button::tertiary("All")).clicked() {
                                    out = Some(true);
                                }
                                if ui.add(crate::ui::widgets::Button::tertiary("None")).clicked() {
                                    out = Some(false);
                                }
                            });
                            out
                        };
                        let ui = &mut cols_ui[0];
                        head(ui, "Show analyzers");
                        if let Some(v) = all_none(ui) {
                            show.iter_mut().for_each(|a| a.2 = v);
                        }
                        for (_, name, on) in &mut show {
                            ui.add(crate::ui::widgets::Checkbox::new(on, name.as_str()));
                        }
                        let ui = &mut cols_ui[1];
                        head(ui, "Search in columns");
                        if let Some(v) = all_none(ui) {
                            cols = [v; 3];
                        }
                        for (name, _) in &COLUMNS[1..3] {
                            ui.add_enabled(false, crate::ui::widgets::Checkbox::new(&mut false, name));
                        }
                        for (i, &c) in SEARCHABLE.iter().enumerate() {
                            ui.add(crate::ui::widgets::Checkbox::new(&mut cols[i], COLUMNS[c].0));
                        }
                    });
                });
            });
        if let Some(s) = self.session.as_mut() {
            for (id, _, on) in show {
                if let Some(a) = s.analyzers.iter_mut().find(|a| a.id == id) {
                    a.show_in_table = on;
                }
            }
        }
        let unsearched = cols.map(|on| !on);
        if unsearched != self.ui.table.unsearched {
            self.ui.table.unsearched = unsearched;
            self.ui.table.stale = true;
        }
    }

    /// Export Table, once every analyzer has finished.
    fn export_table_button(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        let done = !s.state.is_busy()
            && s.analyzers.iter().all(|a| *a.out.status.lock() == crate::decode::runner::Status::Complete);
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), if done { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, done, "Export Table"));
        let c = if !done {
            Token::Ink5
        } else if resp.hovered() {
            Token::Ink1
        } else {
            Token::Ink3
        };
        icons::export(ui.painter(), r.shrink(6.0), theme.color(c));
        let tip = if done { "Export Table" } else { "Available when decoding finishes" };
        if resp.on_hover_text(tip).clicked() {
            self.open_table_export(None);
        }
    }

    /// The dialog, with every analyzer checked, or only `only`.
    pub(crate) fn open_table_export(&mut self, only: Option<u32>) {
        let Some(s) = self.session.as_ref() else { return };
        let analyzers = s.analyzers.iter().map(|a| (a.id, a.name.clone(), only.is_none_or(|o| o == a.id))).collect();
        self.ui.table_export = Some(TableExport { analyzers, apply_search: true, selected_only: false });
    }

    /// The Export Analyzer Data dialog.
    pub(crate) fn table_export_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.ui.table_export.take() else { return };
        let theme = ctx.la_theme();
        let searching = !self.ui.table.query.is_empty();
        let has_selection = self.ui.table.selected.is_some();
        let (mut close, mut save) = (false, false);
        let resp = egui::Modal::new(egui::Id::unique("table_export")).frame(super::dialogs::modal_frame(ctx)).show(
            ctx,
            |ui| {
                ui.set_width(520.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Export Analyzer Data")
                            .text_style(Text::Display.style())
                            .color(theme.color(Token::Ink1)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::ranges::icon_button(ui, "Close", icons::close).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Analyzers").color(theme.color(Token::Ink3)));
                    ui.add_space(12.0);
                    for (_, name, on) in &mut d.analyzers {
                        ui.add(crate::ui::widgets::Checkbox::new(on, name.as_str()));
                    }
                });
                ui.add_space(10.0);
                if searching {
                    ui.add(crate::ui::widgets::Checkbox::new(&mut d.apply_search, "Apply table search"));
                }
                if has_selection {
                    ui.add(crate::ui::widgets::Checkbox::new(&mut d.selected_only, "Selected rows only"));
                }
                ui.add_space(20.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let any = d.analyzers.iter().any(|a| a.2);
                    if ui.add_enabled(any, crate::ui::widgets::Button::primary("Save As…")).clicked() {
                        save = true;
                    }
                    if ui.add(crate::ui::widgets::Button::secondary("Cancel")).clicked() {
                        close = true;
                    }
                });
            },
        );
        if save {
            let rows = self.table_export_rows(&d, searching, has_selection);
            let (dialogs, tx, start) =
                (self.dialogs.clone(), self.events_tx.clone(), self.settings.last_dirs.export.clone());
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let filters = [("CSV".to_owned(), vec!["csv".to_owned()])];
                let path = dialogs.save_file("Export Analyzer Data", start.as_deref(), "analyzer-data.csv", &filters);
                let _ = tx.send(crate::app::AppEvent::TableExportFile(rows, path));
                ctx.request_repaint();
            });
        } else if !close && !resp.should_close() {
            self.ui.table_export = Some(d);
        }
    }

    /// The CSV lines, header first, in table order.
    fn table_export_rows(&mut self, d: &TableExport, searching: bool, has_selection: bool) -> Vec<String> {
        self.refresh_table();
        let quote = |s: &str| {
            if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
        };
        let wanted: std::collections::HashSet<u32> = d.analyzers.iter().filter(|a| a.2).map(|a| a.0).collect();
        let c = &self.ui.table;
        let positions: Vec<usize> = if d.selected_only && has_selection {
            let (a, b) = c.selected.unwrap();
            (a..=b).collect()
        } else if d.apply_search && searching {
            (0..c.filtered.len()).collect()
        } else {
            (0..c.rows.len()).collect()
        };
        let use_filtered = !c.query.is_empty() && (d.selected_only && has_selection || d.apply_search && searching);
        let rows: Vec<RowRef> = positions
            .into_iter()
            .filter_map(|k| {
                if use_filtered { c.filtered.get(k).and_then(|&i| c.rows.get(i as usize)) } else { c.rows.get(k) }
                    .copied()
            })
            .filter(|r| wanted.contains(&r.analyzer))
            .collect();
        let mut out = vec![COLUMNS.iter().map(|c| c.0).collect::<Vec<_>>().join(",")];
        for r in rows {
            if let Some(c) = self.cells(r) {
                out.push(c.all().iter().map(|s| quote(s)).collect::<Vec<_>>().join(","));
            }
        }
        out
    }

    /// Writes the rows off the UI thread; "Export complete".
    pub(crate) fn write_table_export(&mut self, lines: Vec<String>, path: std::path::PathBuf) {
        if let Some(dir) = path.parent() {
            self.settings.last_dirs.export = Some(dir.to_path_buf());
            self.mark_settings_dirty();
        }
        let n = lines.len().saturating_sub(1) as u64;
        let tx = self.events_tx.clone();
        let text = lines.join("\n") + "\n";
        self.ui.exporting += 1;
        std::thread::spawn(move || {
            let r = std::fs::write(&path, text).map(|_| (path, n)).map_err(|e| e.to_string());
            let _ = tx.send(crate::app::AppEvent::ExportDone(r));
        });
    }

    // ---- the terminal ----

    /// The terminal's text: per streaming analyzer, its data frames after
    /// the last Clear, interleaved by start; as `(color, text)` runs.
    fn terminal_runs(&self) -> Vec<([u8; 3], String)> {
        let Some(s) = self.session.as_ref() else { return Vec::new() };
        let list = self.decoder_list();
        let mut items: Vec<(u64, [u8; 3], String)> = Vec::new();
        for a in s.analyzers.iter().filter(|a| a.to_terminal) {
            let Some(info) = list.iter().find(|d| d.id == a.decoder) else { continue };
            let rows = all_rows(info);
            let f = a.out.frames.read();
            let from = self.ui.terminal_cleared.get(&a.id).copied().unwrap_or(0);
            let serial = a.decoder == "uart";
            let mut prev_cr = false;
            for id in from..f.len() {
                let fr = f.get(id as u32).unwrap();
                let row = rows.get(fr.row).map_or("", |r| r.0.as_str());
                let class = info.annotations.get(fr.class).map_or("", |c| c.0.as_str());
                if serial {
                    // serial-like: the characters of the data frames.
                    if !class.ends_with("-data") {
                        continue;
                    }
                    let Some(b) = uart_byte(fr.longest(), a.radix) else { continue };
                    // CR+LF is one newline.
                    if b == b'\n' && prev_cr {
                        prev_cr = false;
                        continue;
                    }
                    prev_cr = b == b'\r';
                    items.push((fr.start, a.color, terminal_char(b)));
                } else if a.rows_visible.iter().any(|v| v == row) {
                    items.push((fr.start, a.color, format!("{}\n", fr.longest())));
                }
            }
        }
        items.sort_by_key(|i| i.0);
        // Merge runs of one color.
        let mut runs: Vec<([u8; 3], String)> = Vec::new();
        for (_, c, t) in items {
            match runs.last_mut() {
                Some((lc, s)) if *lc == c => s.push_str(&t),
                _ => runs.push((c, t)),
            }
        }
        // The last 10 000 lines.
        let lines: usize = runs.iter().map(|r| r.1.matches('\n').count()).sum();
        let mut drop = lines.saturating_sub(TERMINAL_LINES);
        while drop > 0 && !runs.is_empty() {
            let s = &mut runs[0].1;
            match s.find('\n') {
                Some(i) => {
                    s.drain(..=i);
                    drop -= 1;
                }
                None => {
                    runs.remove(0);
                }
            }
        }
        runs
    }

    fn terminal_ui(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let runs = self.terminal_runs();
        let light = self.theme() == crate::ui::theme::ThemeKind::Light;
        let mut job = egui::text::LayoutJob::default();
        let font = FontId::monospace(12.0);
        for (c, t) in &runs {
            // Darkened 40 % in the light theme.
            let color = if light { c32(*c).gamma_multiply(0.6).to_opaque() } else { c32(*c) };
            job.append(t, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
        }
        let text: String = runs.iter().map(|r| r.1.as_str()).collect();
        // On a console fill, in the panel's scroll like the table.
        let min = (ui.available_size() - vec2(20.0, 20.0)).max(vec2(0.0, 0.0));
        let at = egui::Frame::new()
            .fill(theme.color(Token::Canvas))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_min_size(min);
                let r = ui.add(egui::Label::new(job).selectable(true).wrap());
                r.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("Terminal: {text}")));
                let lines = runs.len();
                let visible = ui.clip_rect().intersect(r.rect);
                // Follow the newest lines while the bottom is in view.
                if lines > self.ui.terminal_len && !self.ui.terminal_detached {
                    ui.scroll_to_rect(r.rect, Some(egui::Align::BOTTOM));
                }
                self.ui.terminal_len = lines;
                self.ui.terminal_detached = !(visible.is_positive() && visible.bottom() >= r.rect.bottom() - 1.0);
                visible
            })
            .inner;
        // Clear on hover, at the visible top-right.
        if at.is_positive() && ui.rect_contains_pointer(at) {
            let b = Rect::from_center_size(pos2(at.right() - 24.0, at.top() + 24.0), vec2(20.0, 20.0));
            let resp = ui.interact(b, ui.make_persistent_id("terminal_clear"), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Clear Terminal"));
            icons::trash(ui.painter(), b, theme.color(Token::Ink2));
            if resp.on_hover_text("Clear Terminal").clicked() {
                let counts: Vec<(u32, usize)> = self
                    .session
                    .as_ref()
                    .map_or(Vec::new(), |s| s.analyzers.iter().map(|a| (a.id, a.out.frames.read().len())).collect());
                self.ui.terminal_cleared.extend(counts);
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Act {
    Select,
    Extend,
    Goto,
    Zoom,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_forms_and_bytes() {
        assert_eq!(query_forms("0x6C", Radix::Ascii), ["l"]);
        assert_eq!(query_forms("0x6c", Radix::Dec), ["108"]);
        assert_eq!(query_forms("0b1", Radix::Hex), ["01"]);
        assert!(query_forms("l", Radix::Ascii).is_empty());
        assert_eq!(uart_byte("[0D]", Radix::Ascii), Some(13));
        assert_eq!(uart_byte("H", Radix::Ascii), Some(b'H'));
        assert_eq!(uart_byte("48", Radix::Hex), Some(0x48));
        assert_eq!(terminal_char(7), "^G");
        assert_eq!(terminal_char(b'\r'), "\n");
    }
}
