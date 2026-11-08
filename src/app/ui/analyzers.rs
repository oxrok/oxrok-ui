//! Protocol analyzers: the picker, the settings dialog and the
//! Analyzers panel list.

use super::markers::c32;
use super::ranges::icon_button;
use crate::app::LaApp;
use crate::app::analyzers::{Analyzer, validate_channels};
use crate::core::ChannelId;
use crate::core::capture::CaptureMode;
use crate::decode::runner::Status;
use crate::decode::{ANALYZER_PALETTE, DecOption, DecoderInfo, OptionValue, Radix, SUGGESTED, default_visible_rows};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Checkbox, Dropdown, menu_check_item, menu_item, menu_separator, popover_frame};
use egui::{
    Align2, Color32, Context, Frame, Key, Margin, Modal, Popup, PopupCloseBehavior, Rect, Response, Role, Sense,
    Stroke, Ui, WidgetInfo, pos2, vec2,
};
use std::sync::Arc;

/// Panel id.
pub(crate) const ANALYZERS_PANEL: &str = "analyzers";
/// Row height.
const ROW_H: f32 = 36.0;

/// An option field in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OptField {
    /// Index into the option's values.
    Choice(usize),
    /// A typed value (int, float or string).
    Text(String),
    /// yes/no values as a checkbox.
    Check(bool),
}

/// The settings dialog.
#[derive(Clone, Debug)]
pub(crate) struct AnalyzerDialog {
    /// The analyzer being edited, or `None` for a new one.
    pub editing: Option<u32>,
    pub decoder: String,
    /// Role, label, required, the chosen channel.
    pub channels: Vec<(String, String, bool, Option<ChannelId>)>,
    pub options: Vec<(DecOption, OptField)>,
    pub show_in_table: bool,
    pub to_terminal: bool,
    /// Banner text and the invalid field ids.
    pub error: Option<String>,
    pub bad: Vec<String>,
    /// Opened from the Inspector, so it shows beside a detached one.
    pub from_inspector: bool,
}

/// The dialog's width.
const DIALOG_W: f32 = 540.0;

/// The analyzer dialog's parts: the fields scroll between the other two.
#[derive(Copy, Clone)]
enum Part {
    Header,
    Fields,
    Footer,
}

/// The analyzer dialog's own window, while it has one.
#[derive(Clone)]
pub(crate) struct DialogWindow {
    /// Fixed while open (a changing builder would move the window).
    pub builder: egui::ViewportBuilder,
    /// Moved next to the Inspector, once its size was known.
    pub placed: bool,
}

/// The dialog window's size for content of `want`: at least
/// [`DIALOG_W`] wide, and no taller than 85 % of the screen or the
/// Inspector window (at least 400 pt), so its buttons stay on screen.
fn fit_dialog(
    want: egui::Vec2,
    monitor: Option<egui::Vec2>,
    inspector: Option<crate::app::settings::WindowRect>,
) -> egui::Vec2 {
    let mut max = monitor.map_or(egui::Vec2::splat(f32::INFINITY), |m| m * vec2(0.9, 0.85));
    if let Some(r) = inspector {
        max.y = max.y.min(r.h.max(400.0));
    }
    vec2(want.x.max(DIALOG_W).min(max.x), want.y.min(max.y)).ceil()
}

/// Where the dialog window of `size` opens: over the
/// Inspector window `r`, sharing the edge that faces the main window
/// (which is on screen, unlike a centered dialog's overhang), centered
/// vertically but not above the Inspector's top.
fn place_dialog(size: egui::Vec2, r: crate::app::settings::WindowRect, main: Option<egui::Rect>) -> egui::Pos2 {
    let right_of_main = main.is_some_and(|m| r.x + r.w / 2.0 > m.center().x);
    let x = if right_of_main { r.x + r.w - size.x } else { r.x };
    egui::pos2(x, r.y + ((r.h - size.y) / 2.0).max(0.0))
}

fn is_yes_no(o: &DecOption) -> bool {
    let s: Vec<&str> =
        o.values.iter().filter_map(|v| if let OptionValue::Str(s) = v { Some(s.as_str()) } else { None }).collect();
    s.len() == 2 && s.contains(&"yes") && s.contains(&"no")
}

fn value_text(v: &OptionValue) -> String {
    match v {
        OptionValue::Int(i) => i.to_string(),
        OptionValue::Float(f) => f.to_string(),
        OptionValue::Str(s) => s.clone(),
    }
}

fn field_for(o: &DecOption, current: Option<&OptionValue>) -> OptField {
    let v = current.unwrap_or(&o.default);
    if is_yes_no(o) {
        OptField::Check(matches!(v, OptionValue::Str(s) if s == "yes"))
    } else if !o.values.is_empty() {
        OptField::Choice(o.values.iter().position(|x| x == v).unwrap_or(0))
    } else {
        OptField::Text(value_text(v))
    }
}

impl AnalyzerDialog {
    pub fn new(info: &DecoderInfo, existing: Option<&Analyzer>) -> Self {
        let chan = |role: &str| existing.and_then(|a| a.channels.iter().find(|c| c.0 == role).map(|c| c.1));
        let channels = info
            .channels
            .iter()
            .map(|c| (c.id.clone(), c.name.clone(), true, chan(&c.id)))
            .chain(info.opt_channels.iter().map(|c| (c.id.clone(), c.name.clone(), false, chan(&c.id))))
            .collect();
        let options = info
            .options
            // The radix sets `format`, so it isn't a field here.
            .iter()
            .filter(|o| o.id != "format")
            .map(|o| (o.clone(), field_for(o, existing.and_then(|a| a.options.get(&o.id)))))
            .collect();
        Self {
            editing: existing.map(|a| a.id),
            decoder: info.id.clone(),
            channels,
            options,
            show_in_table: existing.is_none_or(|a| a.show_in_table),
            to_terminal: existing.map_or(info.id == "uart", |a| a.to_terminal),
            error: None,
            bad: Vec::new(),
            from_inspector: false,
        }
    }

    /// The options as values, or the first error and its field.
    fn values(&self) -> Result<std::collections::BTreeMap<String, OptionValue>, (String, String)> {
        let mut out = std::collections::BTreeMap::new();
        for (o, f) in &self.options {
            let v = match f {
                OptField::Choice(k) => o.values.get(*k).cloned().unwrap_or_else(|| o.default.clone()),
                OptField::Check(b) => OptionValue::Str(if *b { "yes" } else { "no" }.into()),
                OptField::Text(t) => {
                    let t = t.trim();
                    match &o.default {
                        OptionValue::Int(_) if t.is_empty() => {
                            return Err((tr("Required").into(), o.id.clone()));
                        }
                        OptionValue::Int(_) => match t.parse::<i64>() {
                            Ok(i) => OptionValue::Int(i),
                            Err(_) => return Err((tr("Must be an integer").into(), o.id.clone())),
                        },
                        OptionValue::Float(_) => match t.parse::<f64>() {
                            Ok(x) => OptionValue::Float(x),
                            Err(_) => return Err((tr("Must be a number").into(), o.id.clone())),
                        },
                        OptionValue::Str(_) => OptionValue::Str(t.to_owned()),
                    }
                }
            };
            if v != o.default {
                out.insert(o.id.clone(), v);
            }
        }
        Ok(out)
    }
}

/// Decoders taking logic input that match `filter` (long name,
/// id, display name or tags): the suggested ones first, then the rest by
/// long name.
pub(crate) fn picker_items<'a>(list: &'a [DecoderInfo], filter: &str) -> Vec<&'a DecoderInfo> {
    let q = filter.to_lowercase();
    let matches = |d: &DecoderInfo| {
        d.inputs.iter().any(|i| i == "logic")
            && (q.is_empty()
                || d.longname.to_lowercase().contains(&q)
                || d.id.contains(&q)
                || crate::decode::display_name(d).to_lowercase().contains(&q)
                || d.tags.iter().any(|t| t.to_lowercase().contains(&q)))
    };
    let mut items: Vec<&DecoderInfo> =
        SUGGESTED.iter().filter_map(|id| list.iter().find(|d| d.id == *id)).filter(|d| matches(d)).collect();
    let mut rest: Vec<&DecoderInfo> =
        list.iter().filter(|d| !SUGGESTED.contains(&d.id.as_str()) && matches(d)).collect();
    rest.sort_by_key(|d| d.longname.to_lowercase());
    items.extend(rest);
    items
}

impl LaApp {
    /// The decoders once loaded (empty while loading or on failure).
    pub(crate) fn decoder_list(&self) -> Arc<Vec<DecoderInfo>> {
        self.decode.decoders().and_then(Result::ok).unwrap_or_default()
    }

    pub(crate) fn decoder_info(&self, id: &str) -> Option<DecoderInfo> {
        self.decoder_list().iter().find(|d| d.id == id).cloned()
    }

    /// No edits while recording.
    fn analyzers_locked(&mut self) -> bool {
        if self.session.as_ref().is_some_and(|s| s.state.is_busy()) {
            self.toast(Toast::new(ToastKind::Warning, tr("Analyzers can't be edited while recording")));
            return true;
        }
        false
    }

    pub(crate) fn open_analyzer_dialog(&mut self, decoder: &str, existing: Option<u32>) {
        if self.analyzers_locked() {
            return;
        }
        let Some(info) = self.decoder_info(decoder) else { return };
        let a = existing.and_then(|id| self.session.as_ref()?.analyzers.iter().find(|a| a.id == id).cloned());
        let mut d = AnalyzerDialog::new(&info, a.as_ref());
        d.from_inspector = self.ui.in_inspector;
        self.ui.analyzer_dialog = Some(d);
    }

    // ---- picker ----

    pub(crate) fn analyzer_picker(&mut self, ui: &Ui, anchor: &Response) {
        let theme = ui.la_theme();
        let list = self.decoder_list();
        let loading = self.decode.decoders().is_none();
        let mut pick = None;
        let filter = self.ui.picker_filter.clone();
        let mut new_filter = filter.clone();
        let mut highlight = self.ui.picker_highlight;
        let was_open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(anchor));
        Popup::menu(anchor)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .frame(popover_frame(ui))
            .width(280.0)
            .show(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    icons::zoom_x(ui.painter(), r, theme.color(Token::Ink3));
                    let edit = ui
                        .add(egui::TextEdit::singleline(&mut new_filter).hint_text(tr("Filter")).desired_width(220.0));
                    super::label_text_edit(&edit, &new_filter, tr("Filter analyzers"));
                    if !was_open || !ui.memory(|m| m.has_focus(edit.id)) && ui.memory(|m| m.focused().is_none()) {
                        edit.request_focus();
                    }
                });
                menu_separator(ui);
                if loading {
                    ui.label(
                        egui::RichText::new(format!("  {}", tr("Loading decoders…"))).color(theme.color(Token::Ink3)),
                    );
                    return;
                }
                let items = picker_items(&list, &new_filter);
                if items.is_empty() {
                    ui.label(egui::RichText::new("  No matching analyzers").color(theme.color(Token::Ink3)));
                    return;
                }
                let (up, down, enter) = ui
                    .input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter)));
                if down {
                    highlight = (highlight + 1).min(items.len() - 1);
                }
                if up {
                    highlight = highlight.saturating_sub(1);
                }
                highlight = highlight.min(items.len() - 1);
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for (k, d) in items.iter().enumerate() {
                        let r = menu_item(ui, &crate::decode::display_name(d), None);
                        if k == highlight && !r.hovered() {
                            ui.painter().rect_filled(r.rect, 0.0, theme.color(Token::Flare).gamma_multiply(0.25));
                            if up || down {
                                r.scroll_to_me(None);
                            }
                        }
                        if r.hovered() {
                            highlight = k;
                        }
                        if r.clicked() {
                            pick = Some(d.id.clone());
                        }
                    }
                });
                if enter {
                    pick = items.get(highlight).map(|d| d.id.clone());
                }
            });
        if new_filter != filter {
            highlight = 0;
        }
        self.ui.picker_filter = new_filter;
        self.ui.picker_highlight = highlight;
        if let Some(id) = pick {
            Popup::close_all(ui.ctx());
            self.ui.picker_filter.clear();
            self.open_analyzer_dialog(&id, None);
        }
    }

    // ---- settings dialog ----

    pub(crate) fn analyzer_dialog_ui(&mut self, ctx: &Context) {
        if self.ui.analyzer_dialog.is_none() {
            self.ui.analyzer_window = None;
            return;
        }
        let Some(mut d) = self.ui.analyzer_dialog.take() else { return };
        let Some(info) = self.decoder_info(&d.decoder) else { return };
        let Some(s) = self.session.as_ref() else { return };
        let theme = ctx.la_theme();
        // Enabled channels first, then the disabled ones.
        let rows: Vec<(ChannelId, String, bool)> = s
            .rows
            .iter()
            .filter(|r| r.channel.kind == crate::core::ChannelKind::Digital)
            .map(|r| (r.channel, format!("{:02}.  {}", r.channel.index, r.display()), s.settings.is_enabled(r.channel)))
            .collect();
        let mut chan_items: Vec<(Option<ChannelId>, String)> = Vec::new();
        for enabled in [true, false] {
            for (c, label, e) in rows.iter().filter(|r| r.2 == enabled) {
                let l = if *e { label.clone() } else { tr("{}  (disabled)").replacen("{}", label, 1) };
                chan_items.push((Some(*c), l));
            }
        }
        let (mut save, mut cancel, mut reset) = (false, false, false);
        let title = crate::decode::display_name(&info);
        // Beside a detached Inspector it was opened from, a
        // window of its own; else a modal in the main window.
        let own_window = self.dialog_beside_inspector(d.from_inspector);
        // One part of the dialog; `Fields` returns its content size.
        let mut part = |ui: &mut Ui, p: Part, max_h: f32| match p {
            Part::Header => {
                Frame::new().inner_margin(Margin::symmetric(32, 24)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&title)
                                .text_style(Text::Display.style())
                                .color(theme.color(Token::Ink1)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, tr("Close"), icons::close).clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
                ui.separator();
                egui::Vec2::ZERO
            }
            Part::Fields => {
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .show(ui, |ui| {
                        Frame::new().inner_margin(Margin::symmetric(32, 16)).show(ui, |ui| {
                            egui::Grid::new("analyzer_fields")
                                .num_columns(2)
                                .min_col_width(170.0)
                                .spacing(vec2(16.0, 8.0))
                                .show(ui, |ui| {
                                    for (role, name, required, chosen) in &mut d.channels {
                                        let label = if *required { format!("{name} *") } else { name.clone() };
                                        ui.label(egui::RichText::new(&label).color(theme.color(Token::Ink2)));
                                        let mut items: Vec<String> = Vec::new();
                                        let mut values: Vec<Option<ChannelId>> = Vec::new();
                                        if !*required {
                                            items.push(tr("None").into());
                                            values.push(None);
                                        }
                                        for (c, l) in &chan_items {
                                            items.push(l.clone());
                                            values.push(*c);
                                        }
                                        let mut k = values.iter().position(|v| v == chosen).unwrap_or(0);
                                        if *required && chosen.is_none() {
                                            k = usize::MAX;
                                        }
                                        let mut sel = k.min(items.len());
                                        let shown: Vec<String> = if k == usize::MAX {
                                            let mut v = items.clone();
                                            v.push(tr("Select a channel").into());
                                            sel = v.len() - 1;
                                            v
                                        } else {
                                            items.clone()
                                        };
                                        let bad = d.bad.contains(role);
                                        let r = ui.scope(|ui| {
                                            if bad {
                                                ui.visuals_mut().widgets.inactive.bg_stroke =
                                                    Stroke::new(1.0, theme.color(Token::Alert));
                                            }
                                            ui.add_sized(
                                                vec2(300.0, 40.0),
                                                Dropdown::new(&format!("{name} channel"), &mut sel, &shown),
                                            )
                                        });
                                        if bad {
                                            ui.painter().rect_stroke(
                                                r.inner.rect,
                                                3.0,
                                                Stroke::new(1.0, theme.color(Token::Alert)),
                                                egui::StrokeKind::Inside,
                                            );
                                        }
                                        if sel < values.len() {
                                            *chosen = values[sel];
                                        }
                                        ui.end_row();
                                    }
                                    for (o, f) in &mut d.options {
                                        ui.label(egui::RichText::new(&o.desc).color(theme.color(Token::Ink2)));
                                        let bad = d.bad.contains(&o.id);
                                        match f {
                                            OptField::Choice(k) => {
                                                let items: Vec<String> = o
                                                    .values
                                                    .iter()
                                                    .map(|v| {
                                                        let t = value_text(v);
                                                        if *v == o.default {
                                                            tr("{} (Standard)").replacen("{}", &t, 1)
                                                        } else {
                                                            t
                                                        }
                                                    })
                                                    .collect();
                                                ui.add_sized(vec2(300.0, 40.0), Dropdown::new(&o.desc, k, &items));
                                            }
                                            OptField::Text(t) => {
                                                let r = ui.add_sized(vec2(300.0, 32.0), egui::TextEdit::singleline(t));
                                                super::label_text_edit(&r, t, &o.desc);
                                                if bad {
                                                    ui.painter().rect_stroke(
                                                        r.rect,
                                                        3.0,
                                                        Stroke::new(1.0, theme.color(Token::Alert)),
                                                        egui::StrokeKind::Outside,
                                                    );
                                                }
                                            }
                                            OptField::Check(b) => {
                                                ui.add(Checkbox::new(b, &o.desc));
                                            }
                                        }
                                        ui.end_row();
                                    }
                                    // Common rows.
                                    ui.label("");
                                    ui.add(Checkbox::new(&mut d.show_in_table, tr("Show in protocol results table")));
                                    ui.end_row();
                                    ui.label("");
                                    ui.add(Checkbox::new(&mut d.to_terminal, tr("Stream to terminal")));
                                    ui.end_row();
                                });
                            if let Some(url) = &info.doc_url {
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                    icons::info(ui.painter(), r, theme.color(Token::Mark));
                                    ui.hyperlink_to(tr("Learn more about this decoder"), url);
                                });
                            }
                        });
                    })
                    .content_size
            }
            Part::Footer => {
                // Banner.
                if let Some(e) = &d.error {
                    Frame::new().fill(theme.color(Token::Alert)).inner_margin(Margin::symmetric(16, 10)).show(
                        ui,
                        |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new(format!("!  {e}")).color(Color32::WHITE));
                        },
                    );
                }
                ui.separator();
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(Button::secondary(tr("Reset"))).clicked() {
                            reset = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(Button::primary(tr("Save"))).clicked() {
                                save = true;
                            }
                            ui.add_space(8.0);
                            if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
                egui::Vec2::ZERO
            }
        };
        let close = if own_window {
            if self.ui.analyzer_window.is_none() {
                self.ui.analyzer_window = Some(DialogWindow {
                    builder: self.dialog_window_builder(&title, vec2(DIALOG_W, 560.0)),
                    placed: false,
                });
            }
            let builder = self.ui.analyzer_window.as_ref().map(|w| w.builder.clone()).unwrap_or_default();
            let inspector = self.settings.inspector.window;
            let fill = ctx.la_theme().color(Token::Card);
            let (close, fit) =
                ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("analyzer_dialog"), builder, |ui, class| {
                    let close = ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                    if class == egui::ViewportClass::EmbeddedWindow {
                        ui.set_width(DIALOG_W);
                        part(ui, Part::Header, 0.0);
                        part(ui, Part::Fields, 480.0);
                        part(ui, Part::Footer, 0.0);
                        return (close, None);
                    }
                    // Header and buttons fixed, the fields scroll between them.
                    let panel = |p: egui::Panel| p.frame(Frame::new().fill(fill)).show_separator_line(false);
                    let top = panel(egui::Panel::top("analyzer_header")).show(ui, |ui| part(ui, Part::Header, 0.0));
                    let bottom =
                        panel(egui::Panel::bottom("analyzer_footer")).show(ui, |ui| part(ui, Part::Footer, 0.0));
                    let fields = egui::CentralPanel::default()
                        .frame(Frame::new().fill(fill))
                        .show(ui, |ui| part(ui, Part::Fields, ui.available_height()))
                        .inner;
                    let want = vec2(fields.x, top.response.rect.height() + bottom.response.rect.height() + fields.y);
                    (close, Some((want, ui.ctx().input(|i| (i.viewport().inner_rect, i.viewport().monitor_size)))))
                });
            if let (Some((want, (inner, monitor))), Some(w)) = (fit, self.ui.analyzer_window.as_mut()) {
                // Fit the window to its content, within the screen and next
                // to the Inspector.
                let size = fit_dialog(want, monitor, inspector);
                if inner.is_some_and(|r| (r.width() - size.x).abs() > 1.0 || (r.height() - size.y).abs() > 1.0) {
                    ctx.send_viewport_cmd_to(
                        egui::ViewportId::from_hash_of("analyzer_dialog"),
                        egui::ViewportCommand::InnerSize(size),
                    );
                }
                if !w.placed
                    && let Some(r) = inspector
                {
                    w.placed = true;
                    let main = ctx.input_for(egui::ViewportId::ROOT, |i| i.viewport().outer_rect);
                    ctx.send_viewport_cmd_to(
                        egui::ViewportId::from_hash_of("analyzer_dialog"),
                        egui::ViewportCommand::OuterPosition(place_dialog(size, r, main)),
                    );
                }
            }
            close
        } else {
            self.ui.analyzer_window = None;
            let resp = Modal::new(egui::Id::unique("analyzer_dialog"))
                .frame(super::dialogs::modal_frame(ctx).inner_margin(Margin::ZERO))
                .show(ctx, |ui| {
                    ui.set_width(DIALOG_W);
                    part(ui, Part::Header, 0.0);
                    part(ui, Part::Fields, ctx.content_rect().height() * 0.6);
                    part(ui, Part::Footer, 0.0);
                });
            resp.should_close() || ctx.input(|i| i.key_pressed(Key::Escape))
        };
        if close {
            cancel = true;
        }
        if cancel {
            return;
        }
        if reset {
            let (editing, from_inspector) = (d.editing, d.from_inspector);
            d = AnalyzerDialog::new(&info, None);
            d.editing = editing;
            d.from_inspector = from_inspector;
        }
        if save {
            let chosen: Vec<(String, Option<ChannelId>)> = d.channels.iter().map(|c| (c.0.clone(), c.3)).collect();
            if let Err(e) = validate_channels(&info, &chosen) {
                d.error = Some(tr(e).into());
                d.bad = d.channels.iter().filter(|c| c.2 && c.3.is_none()).map(|c| c.0.clone()).collect();
                self.ui.analyzer_dialog = Some(d);
                return;
            }
            match d.values() {
                Err((e, field)) => {
                    d.error = Some(e);
                    d.bad = vec![field];
                    self.ui.analyzer_dialog = Some(d);
                }
                Ok(options) => self.save_analyzer(&info, d, options),
            }
            return;
        }
        self.ui.analyzer_dialog = Some(d);
    }

    /// Create or update, (re)start decoding, open the panel.
    fn save_analyzer(
        &mut self,
        info: &DecoderInfo,
        d: AnalyzerDialog,
        options: std::collections::BTreeMap<String, OptionValue>,
    ) {
        let channels: Vec<(String, ChannelId)> =
            d.channels.iter().filter_map(|c| c.3.map(|ch| (c.0.clone(), ch))).collect();
        let Some(s) = self.session.as_mut() else { return };
        // A disabled channel chosen is enabled, with a toast.
        let newly: Vec<ChannelId> = channels.iter().map(|c| c.1).filter(|c| !s.settings.is_enabled(*c)).collect();
        for c in &newly {
            s.settings.enabled.push(*c);
        }
        let id = d.editing.unwrap_or_else(|| s.analyzers.iter().map(|a| a.id + 1).max().unwrap_or(0));
        let a = match s.analyzers.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.channels = channels;
                a.options = options;
                a.show_in_table = d.show_in_table;
                a.to_terminal = d.to_terminal;
                a.clone()
            }
            None => {
                let a = Analyzer {
                    id,
                    decoder: info.id.clone(),
                    name: crate::decode::display_name(info),
                    color: ANALYZER_PALETTE[s.analyzers.len() % ANALYZER_PALETTE.len()],
                    channels,
                    options,
                    radix: crate::decode::default_radix(info),
                    show_in_table: d.show_in_table,
                    to_terminal: d.to_terminal,
                    rows_visible: default_visible_rows(info),
                    out: Default::default(),
                };
                s.analyzers.push(a.clone());
                a
            }
        };
        self.decode.set(s.id, a.id, a.task(info), a.out.clone());
        if !newly.is_empty() {
            let names: Vec<String> = newly.iter().map(|c| c.short_name()).collect();
            self.toast(Toast::new(
                ToastKind::Info,
                tr("Enabled {} for {}").replacen("{}", &names.join(", "), 1).replacen("{}", &a.name, 1),
            ));
        }
        self.settings.inspector.tab = ANALYZERS_PANEL.into();
        self.settings.inspector.open = true;
        self.mark_settings_dirty();
    }

    /// Re-sends an analyzer to the decoder thread (radix, restart).
    pub(crate) fn restart_analyzer(&mut self, id: u32) {
        let Some(a) = self.session.as_ref().and_then(|s| s.analyzers.iter().find(|a| a.id == id).cloned()) else {
            return;
        };
        let Some(sid) = self.active_id() else { return };
        if let Some(info) = self.decoder_info(&a.decoder) {
            self.decode.set(sid, a.id, a.task(&info), a.out.clone());
        }
    }

    // ---- panel ----

    pub(crate) fn analyzers_panel_actions(&mut self, ui: &mut Ui) {
        // The ⏸ toggle, left of `+` (the layout runs right to left).
        let paused = self.ui.analyzers_paused;
        let tip = if paused { tr("Resume analyzers") } else { tr("Pause analyzers") };
        let add = icon_button(ui, tr("Add Analyzer"), icons::plus).on_hover_text(tr("Add Analyzer"));
        let theme = ui.la_theme();
        let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::Button, true, paused, tr("Pause analyzers")));
        if paused {
            ui.painter().rect_filled(rect, 3.0, theme.color(Token::Mark));
        }
        let c = if paused {
            Token::InkOnFlare
        } else if resp.hovered() {
            Token::Ink1
        } else {
            Token::Ink3
        };
        icons::pause(ui.painter(), rect.shrink(4.0), theme.color(c));
        if resp.on_hover_text(tip).clicked() {
            self.dispatch("analyzer.pause");
        }
        self.analyzer_picker(ui, &add);
    }

    /// The analyzer list (or the suggested chips), above the data section.
    pub(crate) fn analyzers_list(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        if s.analyzers.is_empty() {
            // Every decoder (suggested first), filterable.
            ui.add_space(12.0);
            let list = self.decoder_list();
            let mut pick: Option<String> = None;
            if self.decode.decoders().is_none() {
                ui.label(
                    egui::RichText::new(format!("    {}", tr("Loading decoders…"))).color(theme.color(Token::Ink3)),
                );
                return;
            }
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.ui.panel_filter)
                        .hint_text(tr("Filter"))
                        .desired_width(ui.available_width() - 16.0),
                );
                super::label_text_edit(&r, &self.ui.panel_filter, tr("Filter decoders"));
            });
            ui.add_space(6.0);
            let items = picker_items(&list, &self.ui.panel_filter);
            if items.is_empty() {
                ui.label(
                    egui::RichText::new(format!("    {}", tr("No matching analyzers"))).color(theme.color(Token::Ink3)),
                );
            }
            egui::ScrollArea::vertical().id_salt("all_decoders").auto_shrink([false, false]).show(ui, |ui| {
                for d in items {
                    let r = menu_item(ui, &crate::decode::display_name(d), None).on_hover_text(&d.desc);
                    if r.clicked() {
                        pick = Some(d.id.clone());
                    }
                }
            });
            if let Some(id) = pick {
                self.open_analyzer_dialog(&id, None);
            }
            return;
        }
        let items: Vec<(u32, String, [u8; 3], Status, f32, String)> = s
            .analyzers
            .iter()
            .map(|a| {
                (a.id, a.name.clone(), a.color, a.out.status.lock().clone(), *a.out.progress.lock(), a.decoder.clone())
            })
            .collect();
        let recording = s.state.is_busy();
        let paused = self.analyzers_paused(s);
        let mut act: Option<(u32, Act)> = None;
        for (id, name, color, status, progress, decoder) in items {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
            let hovered = ui.rect_contains_pointer(rect);
            let p = ui.painter();
            if hovered {
                p.rect_filled(rect, 0.0, theme.color(Token::Lift));
            }
            let sq = Rect::from_center_size(pos2(rect.left() + 21.0, rect.center().y), vec2(10.0, 10.0));
            p.rect_filled(sq, 2.0, c32(color));
            p.text(
                pos2(rect.left() + 36.0, rect.center().y),
                Align2::LEFT_CENTER,
                &name,
                Text::Prose.font_id(),
                theme.color(Token::Ink2),
            );
            // Status.
            let st = Rect::from_center_size(pos2(rect.right() - 44.0, rect.center().y), vec2(16.0, 16.0));
            let (tip, label) = match &status {
                // A gray bar where decoding stopped.
                Status::Running | Status::Idle if paused => {
                    let bar = Rect::from_min_size(
                        pos2(rect.left() + 36.0, rect.bottom() - 6.0),
                        vec2(rect.width() - 100.0, 2.0),
                    );
                    p.rect_filled(bar, 1.0, theme.color(Token::Hair));
                    p.rect_filled(
                        Rect::from_min_size(bar.min, vec2(bar.width() * progress, 2.0)),
                        1.0,
                        theme.color(Token::Ink4),
                    );
                    (tr("Paused at {} %").replacen("{}", &format!("{:.0}", progress * 100.0), 1), "paused")
                }
                Status::Running => {
                    let bar = Rect::from_min_size(
                        pos2(rect.left() + 36.0, rect.bottom() - 6.0),
                        vec2(rect.width() - 100.0, 2.0),
                    );
                    p.rect_filled(bar, 1.0, theme.color(Token::Hair));
                    p.rect_filled(
                        Rect::from_min_size(bar.min, vec2(bar.width() * progress, 2.0)),
                        1.0,
                        theme.color(Token::Good),
                    );
                    (format!("{:.0} %", progress * 100.0), "decoding")
                }
                Status::Complete => {
                    icons::success(p, st, theme.color(Token::Good));
                    (tr("Complete").to_owned(), "complete")
                }
                Status::Error(e) => {
                    icons::error(p, st, theme.color(Token::Alert));
                    (e.clone(), "error")
                }
                Status::Idle => (tr("Waiting for data").to_owned(), "idle"),
            };
            let sr = ui.interact(st, ui.make_persistent_id(("an_status", id)), Sense::hover());
            sr.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("{name} {label}")));
            sr.on_hover_text(&tip);
            if resp.double_clicked() {
                act = Some((id, Act::Edit));
            }
            let kebab = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(18.0, 18.0));
            let k = ui.interact(kebab, ui.make_persistent_id(("an_kebab", id)), Sense::click());
            k.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("{} menu").replacen("{}", &name, 1)));
            if hovered {
                icons::kebab(ui.painter(), kebab, theme.color(Token::Ink2));
            }
            let a = self.session.as_ref().unwrap().analyzers.iter().find(|a| a.id == id).cloned().unwrap();
            let info = self.decoder_info(&decoder);
            for popup in [Popup::menu(&k), Popup::context_menu(&resp)] {
                popup.frame(popover_frame(ui)).width(240.0).show(|ui| {
                    ui.add_enabled_ui(!recording, |ui| {
                        if menu_item(ui, tr("Edit…"), None)
                            .on_disabled_hover_text(tr("Analyzers can't be edited while recording"))
                            .clicked()
                        {
                            act = Some((id, Act::Edit));
                        }
                        if menu_item(ui, tr("Restart"), None).clicked() {
                            act = Some((id, Act::Restart));
                        }
                    });
                    if menu_item(ui, tr("Delete"), None).clicked() {
                        act = Some((id, Act::Delete));
                    }
                    menu_separator(ui);
                    if menu_check_item(ui, tr("Stream to Terminal"), a.to_terminal, None).clicked() {
                        act = Some((id, Act::Terminal));
                    }
                    if menu_check_item(ui, tr("Show in Data Table"), a.show_in_table, None).clicked() {
                        act = Some((id, Act::Table));
                    }
                    menu_separator(ui);
                    for r in Radix::ALL {
                        let support = info.as_ref().map(crate::decode::radix_support);
                        if super::bubbles::radix_item(ui, support, r, a.radix == r).clicked() {
                            act = Some((id, Act::Radix(r)));
                        }
                    }
                    if let Some(info) = &info {
                        menu_separator(ui);
                        super::rowctl::submenu_checked(ui, tr("Annotation Rows"), false, 220.0, |ui| {
                            for (rid, desc) in crate::decode::all_rows(info) {
                                if menu_check_item(ui, &desc, a.rows_visible.contains(&rid), None).clicked() {
                                    act = Some((id, Act::Row(rid.clone())));
                                }
                            }
                        });
                    }
                    menu_separator(ui);
                    if menu_item(ui, tr("Export to TXT/CSV…"), None).clicked() {
                        act = Some((id, Act::Export));
                    }
                    super::rowctl::submenu_checked(ui, tr("Select Color"), false, 200.0, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.set_width(180.0);
                            for c in ANALYZER_PALETTE {
                                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Color")));
                                ui.painter().rect_filled(r, 3.0, c32(c));
                                if resp.clicked() {
                                    act = Some((id, Act::Color(c)));
                                }
                            }
                        });
                    });
                });
            }
        }
        let Some((id, a)) = act else { return };
        match a {
            Act::Edit => {
                let dec =
                    self.session.as_ref().unwrap().analyzers.iter().find(|x| x.id == id).map(|x| x.decoder.clone());
                if let Some(dec) = dec {
                    self.open_analyzer_dialog(&dec, Some(id));
                }
            }
            Act::Restart => self.restart_analyzer(id),
            Act::Export => self.open_table_export(Some(id)),
            Act::Delete => {
                // Undoable for 5 s.
                let s = self.session.as_mut().unwrap();
                if let Some(k) = s.analyzers.iter().position(|x| x.id == id) {
                    let gone = s.analyzers.remove(k);
                    self.decode.remove(s.id, id);
                    let toast = Toast::new(ToastKind::Info, tr("Analyzer deleted"))
                        .id("analyzer-deleted")
                        .action("Undo")
                        .lifetime(std::time::Duration::from_secs(5));
                    self.ui.deleted_analyzer = Some((k, gone));
                    self.toast(toast);
                }
            }
            _ => {
                let s = self.session.as_mut().unwrap();
                let Some(x) = s.analyzers.iter_mut().find(|x| x.id == id) else { return };
                let restart = match a {
                    Act::Terminal => {
                        x.to_terminal = !x.to_terminal;
                        false
                    }
                    Act::Table => {
                        x.show_in_table = !x.show_in_table;
                        false
                    }
                    Act::Radix(r) => {
                        x.radix = r;
                        true
                    }
                    Act::Row(rid) => {
                        if let Some(k) = x.rows_visible.iter().position(|v| *v == rid) {
                            x.rows_visible.remove(k);
                        } else {
                            x.rows_visible.push(rid);
                        }
                        false
                    }
                    Act::Color(c) => {
                        x.color = c;
                        false
                    }
                    _ => false,
                };
                if restart {
                    self.restart_analyzer(id);
                }
            }
        }
    }

    /// The toast's Undo.
    pub(crate) fn undo_delete_analyzer(&mut self) {
        let Some((k, a)) = self.ui.deleted_analyzer.take() else { return };
        let Some(s) = self.session.as_mut() else { return };
        let k = k.min(s.analyzers.len());
        s.analyzers.insert(k, a.clone());
        self.restart_analyzer(a.id);
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Edit,
    Restart,
    Export,
    Delete,
    Terminal,
    Table,
    Radix(Radix),
    Row(String),
    Color([u8; 3]),
}

// ---- the Analyzer Trigger card ----

impl LaApp {
    pub(crate) fn analyzer_trigger_card(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        if s.analyzers.is_empty() {
            return;
        }
        let sid = s.id;
        let busy = s.state.is_busy();
        let mode = s.settings.mode;
        let streams = s.caps.as_ref().is_none_or(|c| c.supports_continuous);
        let names: Vec<String> = s.analyzers.iter().map(|a| a.name.clone()).collect();
        let mut at = s.analyzer_trigger.clone();
        let mut idx = at.analyzer.and_then(|id| s.analyzers.iter().position(|a| a.id == id)).unwrap_or(0);
        let radix = s.analyzers.get(idx).map_or(Radix::Hex, |a| a.radix);
        let status = if !at.enabled {
            tr("Off").to_owned()
        } else if let (Some(t), true) = (s.trigger, s.trigger_scan.last.is_some()) {
            tr("Last trigger at {}").replacen("{}", &crate::core::format::si_sig(t.as_secs_f64(), "s", 4), 1)
        } else {
            tr("Armed").to_owned()
        };
        // The whole card is off while Timed or Trigger records.
        let card_off = busy && mode != CaptureMode::Continuous;
        let switch_block = if !streams {
            Some(tr("Needs a device that can stream (Continuous mode)"))
        } else if at.query.is_empty() && !at.enabled {
            Some(tr("Enter a value to match"))
        } else {
            None
        };
        let mut open = self.ui.analyzer_trigger_open;
        let before = at.clone();
        let top = ui.cursor().left_top();
        ui.painter()
            .line_segment([top, top + vec2(ui.available_width(), 0.0)], Stroke::new(1.0, theme.color(Token::Seam)));
        let card = ui.add_enabled_ui(!card_off, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(40.0);
                ui.add_space(12.0);
                let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                resp.widget_info(|| WidgetInfo::selected(Role::Button, true, open, tr("Analyzer Trigger details")));
                let chev = if open { icons::chevron_down } else { icons::chevron_right };
                chev(ui.painter(), r.shrink(3.0), theme.color(Token::Ink3));
                if resp.clicked() {
                    open = !open;
                }
                ui.label(egui::RichText::new(tr("Analyzer Trigger")).color(theme.color(Token::Ink1)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(16.0);
                    let r = ui
                        .add_enabled_ui(switch_block.is_none(), |ui| {
                            ui.add(crate::ui::widgets::BinarySwitch::new(&mut at.enabled, tr("Analyzer Trigger")))
                        })
                        .inner;
                    if let Some(why) = switch_block {
                        r.on_disabled_hover_text(why);
                    }
                });
            });
            if open || at.enabled {
                egui::Frame::new().inner_margin(egui::Margin { left: 40, right: 16, top: 0, bottom: 12 }).show(
                    ui,
                    |ui| {
                        egui::Grid::new("analyzer_trigger").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            let label = |ui: &mut Ui, t: &str| {
                                ui.label(egui::RichText::new(t).color(theme.color(Token::Ink3)));
                            };
                            label(ui, tr("Analyzer"));
                            ui.add(Dropdown::new(tr("Trigger analyzer"), &mut idx, &names));
                            ui.end_row();
                            label(ui, tr("Match"));
                            let r = ui
                                .add(
                                    egui::TextEdit::singleline(&mut at.query)
                                        .hint_text(tr("Value [{}]").replacen("{}", radix_short(radix), 1))
                                        .desired_width(ui.available_width()),
                                )
                                .on_hover_text(tr("Triggers when a result's text contains this, ignoring case."));
                            super::label_text_edit(&r, &at.query, tr("Trigger match"));
                            ui.end_row();
                        });
                        ui.label(egui::RichText::new(&status).color(theme.color(Token::Ink4)));
                    },
                );
            }
        });
        if card_off {
            card.response.on_disabled_hover_text(tr("Available in Continuous mode"));
        }
        self.ui.analyzer_trigger_open = open;
        let Some(s) = self.session.as_mut() else { return };
        at.analyzer = s.analyzers.get(idx).map(|a| a.id);
        if at == before && s.analyzer_trigger.analyzer.is_some() {
            return;
        }
        let turned_on = at.enabled && !before.enabled;
        s.analyzer_trigger = at;
        if turned_on {
            self.ui.analyzer_trigger_open = true;
        }
        // Enabling outside Continuous switches to it, with Undo.
        if turned_on && !busy && mode != CaptureMode::Continuous {
            s.settings.mode = CaptureMode::Continuous;
            self.ui.analyzer_trigger_undo = Some((sid, mode));
            self.toast(
                Toast::new(ToastKind::Info, tr("Switched to Continuous mode for the Analyzer Trigger."))
                    .id("analyzer-trigger-mode")
                    .action("Undo"),
            );
        }
        // A change while recording watches the frames decoded from now on.
        let s = self.session.as_mut().unwrap();
        if busy {
            s.arm_analyzer_trigger_at(true);
        }
    }
}

/// The radix in the Match placeholder.
fn radix_short(r: Radix) -> &'static str {
    match r {
        Radix::Native => tr("Native"),
        Radix::Hex => tr("Hex"),
        Radix::Dec => tr("Dec"),
        Radix::Bin => tr("Bin"),
        Radix::Ascii => tr("Ascii"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::settings::WindowRect;

    /// The dialog window fits its content, but stays on screen
    /// and no taller than the Inspector window.
    #[test]
    fn dialog_window_fits_content_within_limits() {
        let monitor = Some(vec2(1512.0, 945.0));
        let inspector = Some(WindowRect { x: 0.0, y: 40.0, w: 300.0, h: 700.0 });
        assert_eq!(fit_dialog(vec2(600.0, 500.0), monitor, inspector), vec2(600.0, 500.0));
        assert_eq!(fit_dialog(vec2(500.0, 500.0), monitor, inspector), vec2(DIALOG_W, 500.0), "at least 540 wide");
        assert_eq!(fit_dialog(vec2(600.0, 1200.0), monitor, inspector), vec2(600.0, 700.0), "the Inspector's height");
        assert_eq!(fit_dialog(vec2(600.0, 1200.0), monitor, None), vec2(600.0, (945.0f32 * 0.85).ceil()));
        // Inspector left of the main window: left edges line up; right of it: right edges.
        let r = WindowRect { x: 0.0, y: 40.0, w: 300.0, h: 700.0 };
        let main = Some(egui::Rect::from_min_size(egui::pos2(200.0, 0.0), vec2(1280.0, 800.0)));
        assert_eq!(place_dialog(vec2(600.0, 500.0), r, main), egui::pos2(0.0, 140.0));
        let r = WindowRect { x: 1600.0, ..r };
        assert_eq!(place_dialog(vec2(600.0, 500.0), r, main), egui::pos2(1300.0, 140.0));
        let short = Some(WindowRect { x: 0.0, y: 40.0, w: 300.0, h: 300.0 });
        assert_eq!(fit_dialog(vec2(600.0, 1200.0), monitor, short).y, 400.0, "at least 400 tall");
    }
}
