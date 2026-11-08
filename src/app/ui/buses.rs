//! Signal groups: the bus row's value bubbles and the
//! *Group Channels…* / *Edit Group…* dialog.

use super::RowGeom;
use crate::app::LaApp;
use crate::app::capture::CaptureData;
use crate::core::bus::{Bus, BusFormat, Endianness, format_value};
use crate::core::format::time_short;
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState};
use crate::l10n::tr;
use crate::store::{ColumnClass, DigitalChannel};
use crate::ui::Token;
use crate::ui::theme::{Theme, ThemeExt};
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Checkbox, Dropdown};
use egui::{Align2, Context, CornerRadius, FontId, Id, Modal, Painter, Pos2, Rect, Role, Sense, Stroke, StrokeKind};
use egui::{Ui, WidgetInfo, pos2, vec2};

/// Bubbles as tall as analyzer bubbles, or the row.
const BUBBLE_H: f32 = 16.0;
/// Text width estimate per character.
const CHAR_PX: f32 = 6.5;

/// The dialog's state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BusDialog {
    /// The group being edited; `None` creates one.
    pub editing: Option<u16>,
    /// The checked channels in wire order (wire 0 first).
    pub selected: Vec<ChannelId>,
    pub name: String,
    /// The name was typed: it no longer follows the selection.
    pub name_typed: bool,
    pub format: BusFormat,
    pub endianness: Endianness,
    pub signed: bool,
}

/// A stable run of columns `[c0, c1)` and its value, or a dense run.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Run {
    Stable { c0: usize, c1: usize, raw: u64 },
    Dense { c0: usize, c1: usize },
}

/// The view's columns split into stable runs (no member changes
/// inside) and dense runs (two or more columns with changes in a row).
/// A single column with changes separates two stable runs.
fn runs(members: &[&DigitalChannel], bus: &Bus, first: f64, spc: f64, n: usize) -> Vec<Run> {
    let classes: Vec<Vec<ColumnClass>> = members.iter().map(|m| m.classify_columns(first, spc, n)).collect();
    let busy = |c: usize| classes.iter().any(|k| k[c] == ColumnClass::Transition);
    let empty = |c: usize| classes.iter().any(|k| matches!(k[c], ColumnClass::NoData | ColumnClass::PartialNoData));
    let mut out = Vec::new();
    let mut c = 0;
    while c < n {
        if empty(c) {
            c += 1;
        } else if busy(c) {
            let c0 = c;
            while c < n && busy(c) && !empty(c) {
                c += 1;
            }
            if c - c0 >= 2 {
                out.push(Run::Dense { c0, c1: c });
            }
        } else {
            let c0 = c;
            while c < n && !busy(c) && !empty(c) {
                c += 1;
            }
            // The value inside the run: at its first column's middle sample.
            let s = (first + (c0 as f64 + 0.5) * spc).max(0.0) as u64;
            let levels = members.iter().map(|m| m.level_at(s).unwrap_or(false));
            out.push(Run::Stable { c0, c1: c, raw: bus.raw(levels) });
        }
    }
    out
}

/// Tooltip: the exact segment around sample `s`, `[start, end)`.
fn segment_at(members: &[&DigitalChannel], s: u64) -> (u64, Option<u64>) {
    let start = members.iter().filter_map(|m| m.prev_transition(s + 1)).max();
    let begin = members.iter().filter_map(|m| m.data_range().map(|r| r.0)).max().unwrap_or(0);
    let end = members.iter().filter_map(|m| m.next_transition(s + 1)).min();
    (start.unwrap_or(begin).max(begin), end)
}

impl BusDialog {
    /// A newly checked channel goes after the last chip with a
    /// lower index, so checking channels one by one gives index order.
    fn check(&mut self, c: ChannelId) {
        if self.selected.contains(&c) {
            return;
        }
        let at = self.selected.iter().rposition(|x| *x < c).map_or(0, |k| k + 1);
        self.selected.insert(at, c);
    }
}

/// Wire order strip: `chips` as draggable chips. A finished drag
/// returns `(from, to)`: remove at `from`, insert at `to`.
fn order_strip(ui: &mut Ui, chips: &[(ChannelId, String, egui::Color32)], theme: &Theme) -> Option<(usize, usize)> {
    let font = FontId::new(13.0, Text::Prose.font_id().family);
    let mut rects = Vec::with_capacity(chips.len());
    let mut dragging: Option<(usize, Pos2)> = None;
    let mut dropped: Option<(usize, Pos2)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.set_min_height(24.0);
        for (k, (_, ident, color)) in chips.iter().enumerate() {
            let galley = ui.painter().layout_no_wrap(ident.clone(), font.clone(), theme.color(Token::Ink1));
            let size = vec2(galley.size().x + 20.0, 24.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::drag());
            let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
            resp.widget_info(|| {
                WidgetInfo::labeled(
                    Role::Button,
                    true,
                    tr("Wire {}: {}").replacen("{}", &k.to_string(), 1).replacen("{}", ident, 1),
                )
            });
            let held = resp.dragged();
            let p = ui.painter();
            let fill = if held { theme.color(Token::Card) } else { theme.color(Token::Deck) };
            p.rect(rect, CornerRadius::same(4), fill, Stroke::new(1.0, *color), StrokeKind::Inside);
            p.galley(rect.center() - galley.size() / 2.0, galley, theme.color(Token::Ink1));
            if let Some(at) = resp.interact_pointer_pos() {
                if held {
                    dragging = Some((k, at));
                }
                if resp.drag_stopped() {
                    dropped = Some((k, at));
                }
            }
            rects.push(rect);
        }
    });
    // Where a chip released at `at` lands: before the first other chip
    // whose center is after `at` in reading order.
    let target = |from: usize, at: Pos2| -> (usize, f32, Rect) {
        let others: Vec<(usize, Rect)> = rects.iter().copied().enumerate().filter(|(k, _)| *k != from).collect();
        let before = |r: &Rect| at.y < r.top() || (at.y <= r.bottom() && at.x < r.center().x);
        match others.iter().position(|(_, r)| before(r)) {
            Some(i) => (i, others[i].1.left() - 3.0, others[i].1),
            None => {
                let last = others.last().map_or(rects[from], |(_, r)| *r);
                (others.len(), last.right() + 3.0, last)
            }
        }
    };
    if let Some((from, at)) = dragging {
        let (_, x, r) = target(from, at);
        ui.painter().vline(x, r.y_range(), Stroke::new(2.0, theme.color(Token::Flare)));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let (from, at) = dropped?;
    let (to, _, _) = target(from, at);
    (to != from).then_some((from, to))
}

impl LaApp {
    /// The bus row's value bubbles over `g`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_bus(
        &self,
        ui: &Ui,
        p: &Painter,
        g: &RowGeom,
        view: &ViewState,
        rate: u64,
        data: Option<&CaptureData>,
        hovered: Option<Pos2>,
        theme: &Theme,
    ) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(bus) = s.bus(g.channel.index).filter(|b| b.format.has_value()) else { return };
        let Some(data) = data else { return };
        let order = s.wire_order(bus.id);
        let members: Option<Vec<&DigitalChannel>> = order.iter().map(|c| data.channel(*c)).collect();
        let Some(members) = members.filter(|m| !m.is_empty()) else { return };
        let Some(row) = s.row(g.channel) else { return };
        let color = super::probe::row_look(theme, s, row).swatch;
        let ppp = ui.ctx().pixels_per_point();
        let n = (g.cell.width() * ppp).ceil() as usize;
        let first = view.left_time.to_sample_f64(rate);
        let spc = view.time_per_px * rate as f64 / ppp as f64;
        let h = BUBBLE_H.min(g.inner.height().max(6.0));
        let y = g.inner.center().y - h / 2.0;
        let x = |c: usize| g.cell.left() + c as f32 / ppp;
        let font = FontId::new(12.0, Text::Prose.font_id().family);
        let ink = theme.color(Token::InkOnSwatch);
        let bits = members.len();
        for run in runs(&members, bus, first, spc, n) {
            match run {
                Run::Dense { c0, c1 } => {
                    let r = Rect::from_x_y_ranges(x(c0)..=x(c1), y..=y + h);
                    p.rect_filled(r, 0.0, color.gamma_multiply(0.5));
                }
                Run::Stable { c0, c1, raw } => {
                    let r = Rect::from_x_y_ranges(x(c0) + 0.5..=x(c1) - 0.5, y..=y + h);
                    if r.width() < 3.0 {
                        // A 1 pt tick.
                        p.line_segment([pos2(r.left(), y), pos2(r.left(), y + h)], Stroke::new(1.0, color));
                        continue;
                    }
                    let over = hovered.is_some_and(|q| r.contains(q));
                    let alpha = if over { 1.0 } else { 0.85 };
                    p.rect(
                        r,
                        CornerRadius::same(2),
                        color.gamma_multiply(alpha),
                        Stroke::new(1.0, color.gamma_multiply(0.6)),
                        StrokeKind::Inside,
                    );
                    let text = bus.format(raw, bits);
                    let fits = text.chars().count() as f32 * CHAR_PX <= r.width() - 6.0;
                    // Text centered in the visible part.
                    let vis = r.intersect(g.cell);
                    if fits && vis.width() > 0.0 {
                        p.with_clip_rect(vis).text(vis.center(), Align2::CENTER_CENTER, &text, font.clone(), ink);
                        let id = ui.make_persistent_id(("bus_bubble", g.channel, c0));
                        let resp = ui.interact(vis.intersect(p.clip_rect()), id, Sense::hover());
                        resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
                    }
                    if over {
                        let s0 = (first + (c0 as f64 + 0.5) * spc).max(0.0) as u64;
                        let (a, b) = segment_at(&members, s0);
                        let secs = |n: u64| n as f64 / rate as f64;
                        let at = time_short(CaptureTime::from_sample(a, rate).as_secs_f64() - s.t0().as_secs_f64());
                        let dur = b.map_or("—".to_owned(), |b| time_short(secs(b - a)));
                        let lines = [
                            format_value(raw, bits, BusFormat::Hex, false),
                            format_value(raw, bits, BusFormat::Decimal, bus.signed),
                            format_value(raw, bits, BusFormat::Binary, false),
                            tr("at {}, for {}").replacen("{}", &at, 1).replacen("{}", &dur, 1),
                        ];
                        let id = ui.make_persistent_id(("bus_tip", g.channel));
                        ui.interact(r.intersect(p.clip_rect()), id, Sense::hover()).on_hover_ui(|ui| {
                            for l in &lines {
                                ui.label(l);
                            }
                        });
                    }
                }
            }
        }
    }

    /// *Group Channels…*, with `pre` checked.
    pub(crate) fn open_group_dialog(&mut self, pre: ChannelId) {
        let selected: Vec<ChannelId> = Some(pre).into_iter().filter(|c| c.kind == ChannelKind::Digital).collect();
        let mut d = BusDialog {
            editing: None,
            selected,
            name: String::new(),
            name_typed: false,
            format: BusFormat::Hex,
            endianness: Endianness::MsbFirst,
            signed: false,
        };
        d.name = self.default_bus_name(&d);
        self.ui.bus_dialog = Some(d);
    }

    /// *Edit Group…*.
    pub(crate) fn open_edit_group(&mut self, index: u16) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(b) = s.bus(index) else { return };
        let name = s.row(b.row()).map(|r| r.name.clone()).unwrap_or_default();
        self.ui.bus_dialog = Some(BusDialog {
            editing: Some(index),
            selected: s.wire_order(index),
            name_typed: name != s.bus_default_name(index),
            name,
            format: b.format,
            endianness: b.endianness,
            signed: b.signed,
        });
    }

    /// `‹first id›–‹last id›` of the selection, in wire order.
    fn default_bus_name(&self, d: &BusDialog) -> String {
        let Some(s) = self.session.as_ref() else { return String::new() };
        let ident = |c: Option<&ChannelId>| c.and_then(|c| s.row(*c)).map(|r| r.ident.clone());
        match (ident(d.selected.first()), ident(d.selected.last())) {
            (Some(a), Some(b)) => crate::core::bus::default_name(&a, &b),
            _ => String::new(),
        }
    }

    pub(crate) fn bus_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.ui.bus_dialog.take() else { return };
        let Some(s) = self.session.as_ref() else { return };
        let theme = ctx.la_theme();
        // The device's enabled digital channels; those in another group
        // are shown disabled with its name.
        let channels: Vec<(ChannelId, String, Option<String>)> = s
            .rows
            .iter()
            .filter(|r| r.channel.kind == ChannelKind::Digital && s.settings.is_enabled(r.channel))
            .map(|r| {
                let other = s
                    .bus_of(r.channel)
                    .filter(|b| Some(b.id) != d.editing)
                    .and_then(|b| s.row(b.row()))
                    .map(|br| format!("{} {}", br.ident, br.name));
                (r.channel, r.display(), other)
            })
            .collect::<Vec<_>>();
        let mut channels = channels;
        channels.sort_by_key(|c| c.0);
        let title = if d.editing.is_some() { tr("Edit Group") } else { tr("Group Channels") };
        let (mut close, mut done) = (false, false);
        let before = d.selected.clone();
        let resp = Modal::new(Id::unique("bus_dialog")).frame(super::dialogs::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            super::help::title_row(ui, title, &mut close);
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tr("Channels")).color(theme.color(Token::Ink3)));
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                egui::Grid::new("bus_channels").num_columns(4).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                    for (k, (c, label, other)) in channels.iter().enumerate() {
                        let mut on = d.selected.contains(c);
                        let text = match other {
                            Some(g) => tr("{} (in {})").replacen("{}", label, 1).replacen("{}", g, 1),
                            None => label.clone(),
                        };
                        let r = ui.add_enabled(other.is_none(), Checkbox::new(&mut on, &text));
                        if r.changed() {
                            if on {
                                d.check(*c);
                            } else {
                                d.selected.retain(|x| x != c);
                            }
                        }
                        if k % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
            });
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tr("Wire order")).color(theme.color(Token::Ink3)));
            ui.add_space(4.0);
            let chips: Vec<(ChannelId, String, egui::Color32)> = d
                .selected
                .iter()
                .filter_map(|c| s.row(*c).map(|r| (*c, r.ident.clone(), super::probe::row_look(&theme, s, r).ink)))
                .collect();
            if let Some((from, to)) = order_strip(ui, &chips, &theme) {
                let c = d.selected.remove(from);
                d.selected.insert(to, c);
            }
            let msb = match d.endianness {
                Endianness::MsbFirst => tr("most"),
                Endianness::LsbFirst => tr("least"),
            };
            let hint = match d.selected.len() {
                0 => tr("Check one or more channels.").to_owned(),
                1 => tr("Check more channels to combine them into one value.").to_owned(),
                _ => tr("Drag to reorder. The first channel is the {} significant bit.").replacen("{}", msb, 1),
            };
            ui.label(egui::RichText::new(hint).small().color(theme.color(Token::Ink3)));
            ui.add_space(12.0);
            let edit =
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text(tr("Name")).desired_width(f32::INFINITY));
            super::label_text_edit(&edit, &d.name, tr("Name"));
            if edit.changed() {
                d.name_typed = true;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let names: Vec<&str> = BusFormat::ALL.iter().map(|f| f.label()).collect();
                let mut i = BusFormat::ALL.iter().position(|f| *f == d.format).unwrap_or(0);
                ui.allocate_ui(vec2(140.0, 32.0), |ui| ui.add(Dropdown::new(tr("Format"), &mut i, &names)));
                d.format = BusFormat::ALL[i];
                let names: Vec<&str> = Endianness::ALL.iter().map(|e| e.label()).collect();
                let mut i = Endianness::ALL.iter().position(|e| *e == d.endianness).unwrap_or(0);
                // A No Data group has no value to order or sign.
                let valued = d.format.has_value();
                ui.add_enabled_ui(valued, |ui| {
                    ui.allocate_ui(vec2(200.0, 32.0), |ui| ui.add(Dropdown::new(tr("Endianness"), &mut i, &names)));
                });
                d.endianness = Endianness::ALL[i];
            });
            ui.add_enabled(d.format.has_value(), Checkbox::new(&mut d.signed, tr("Signed")));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                let ok = !d.selected.is_empty();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if d.editing.is_some() { tr("Save") } else { tr("Create") };
                    if ui.add_enabled(ok, Button::primary(label)).clicked() {
                        done = true;
                    }
                    if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if resp.should_close() {
            close = true;
        }
        if d.selected != before && !d.name_typed {
            d.name = self.default_bus_name(&d);
        }
        if done {
            let members = d.selected.clone();
            if let Some(s) = self.session.as_mut() {
                match d.editing {
                    Some(id) => s.update_bus(id, &members, &d.name, d.format, d.endianness, d.signed),
                    None => {
                        s.create_bus(&members, &d.name, d.format, d.endianness, d.signed);
                    }
                }
            }
            return;
        }
        if !close {
            self.ui.bus_dialog = Some(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(transitions: &[u64], end: u64) -> DigitalChannel {
        let mut c = DigitalChannel::new();
        c.start(0, false);
        for &t in transitions {
            c.push_transition(t);
        }
        c.extend_to(end);
        c
    }

    #[test]
    fn stable_and_dense_runs() {
        let bus = Bus {
            id: 0,
            members: vec![ChannelId::digital(0), ChannelId::digital(1)],
            format: BusFormat::Hex,
            endianness: Endianness::MsbFirst,
            signed: false,
            collapsed: false,
        };
        // D0 rises at 100; D1 toggles every sample from 200 to 260.
        let d0 = ch(&[100], 1000);
        let d1 = ch(&(200..260).collect::<Vec<_>>(), 1000);
        // 10 samples per column, 100 columns.
        let r = runs(&[&d0, &d1], &bus, 0.0, 10.0, 100);
        assert_eq!(r[0], Run::Stable { c0: 0, c1: 10, raw: 0b00 });
        assert_eq!(r[1], Run::Stable { c0: 11, c1: 20, raw: 0b10 });
        assert!(matches!(r[2], Run::Dense { c0: 20, c1: 26 }));
        assert_eq!(r[3], Run::Stable { c0: 26, c1: 100, raw: 0b10 });
        assert_eq!(segment_at(&[&d0, &d1], 50), (0, Some(100)));
        assert_eq!(segment_at(&[&d0, &d1], 150), (100, Some(200)));
    }
}
