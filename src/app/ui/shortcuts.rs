//! Keyboard Shortcuts dialog, toggled with `?`.
//!
//! The rows come from the command registry.

use crate::app::LaApp;
use crate::app::commands::{self, Category};
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::Button;
use egui::{Context, Frame, Id, Margin, Modal, ScrollArea, Sense, Ui, UiBuilder, vec2};

/// A row: the label and its alternatives, each a sequence of key caps.
type Row = (String, Vec<Vec<String>>);

/// Commands shown as one row, as in ("Previous / Next Edge").
const PAIRS: &[(&str, &str, &str)] = &[
    ("nav.prev_edge", "nav.next_edge", "Previous / Next Edge (hovered row)"),
    ("nav.prev_result", "nav.next_result", "Previous / Next Result (hovered row)"),
    ("nav.start", "nav.end", "Go to Start / End"),
    ("nav.pan_left", "nav.pan_right", "Pan Left / Right"),
    ("nav.page_left", "nav.page_right", "Pan Page Left / Right"),
    ("nav.rows_up", "nav.rows_down", "Scroll Rows"),
    ("analog.y_zoom_in", "analog.y_zoom_out", "Y Zoom In / Out"),
    ("session.next", "session.prev", "Next / Previous Session"),
    ("edit.undo", "edit.redo", "Undo / Redo"),
    ("history.back", "history.forward", "Previous / Next View"),
];

/// A command's alternatives on this platform: its keys, then its labels.
fn alternatives(c: &commands::Command) -> Vec<Vec<String>> {
    let mac = super::commands::MAC;
    let mut v: Vec<Vec<String>> =
        c.shown_keys(mac).map(|k| k.caps(mac).into_iter().map(str::to_owned).collect()).collect();
    v.extend(c.key_labels.iter().map(|l| commands::label_caps(l, mac)));
    v
}

/// Sections in order, from the registry (order within each).
fn sections() -> Vec<(&'static str, Vec<Row>)> {
    let r = commands::registry();
    Category::ALL
        .iter()
        .map(|cat| {
            let mut rows: Vec<Row> = Vec::new();
            for c in r.iter().filter(|c| c.category == *cat && !c.hidden_in_shortcuts) {
                if PAIRS.iter().any(|p| p.1 == c.id) {
                    continue;
                }
                let mut alts = alternatives(c);
                let mut label = match c.id {
                    "session.goto" => crate::l10n::tr("Go to Session 1…9").to_owned(),
                    _ => crate::l10n::tr(c.label).to_owned(),
                };
                if let Some((_, second, l)) = PAIRS.iter().find(|p| p.0 == c.id) {
                    label = crate::l10n::tr(l).to_owned();
                    if let Some(s) = commands::get(second) {
                        alts.extend(alternatives(s));
                    }
                }
                if !alts.is_empty() {
                    rows.push((label, alts));
                }
            }
            (cat.label(), rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// Every space-separated word of `query` appears, ignoring case,
/// in the category, the label or the keys.
fn matches(query: &str, category: &str, (label, alternatives): &Row) -> bool {
    let mut hay = format!("{category} {label}");
    for k in alternatives.iter().flat_map(|a| a.iter()) {
        hay.push(' ');
        hay.push_str(commands::search_names(k));
    }
    let hay = hay.to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// Key cap: 16 pt tall, at least 16 pt wide, 4 pt padding, 11 pt text.
fn key_cap(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), Text::CaptionMed.font_id(), theme.color(Token::Ink2));
    let (rect, _) = ui.allocate_exact_size(vec2((galley.size().x + 8.0).max(16.0), 16.0), Sense::hover());
    ui.painter().rect(
        rect,
        3.0,
        theme.color(Token::Field),
        egui::Stroke::new(1.0, theme.color(Token::Seam)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, egui::Color32::PLACEHOLDER);
}

/// The chips for one command, right-aligned: alternatives separated by "OR".
fn chips(ui: &mut Ui, alternatives: &[Vec<String>]) {
    let theme = ui.la_theme();
    ui.spacing_mut().item_spacing.x = 2.0;
    // Right to left: the last alternative is placed first.
    for (i, keys) in alternatives.iter().rev().enumerate() {
        if i > 0 {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(crate::l10n::tr("OR"))
                    .text_style(Text::Caption.style())
                    .color(theme.color(Token::Ink4)),
            );
            ui.add_space(6.0);
        }
        for k in keys.iter().rev() {
            key_cap(ui, k);
        }
    }
}

impl LaApp {
    /// About 600 pt wide, a scrolling body of category sections.
    pub(crate) fn shortcuts_dialog(&mut self, ctx: &Context) {
        let theme = ctx.la_theme();
        // Leave room for the title, the footer and the window margins.
        let max_body = (ctx.content_rect().height() - 340.0).max(160.0);
        let mut close = false;
        let mut filter = self.ui.shortcuts_filter.clone();
        let resp = Modal::new(Id::unique("keyboard_shortcuts"))
            .frame(super::dialogs::modal_frame(ctx).inner_margin(Margin::ZERO))
            .show(ctx, |ui| {
                ui.set_width(600.0);
                Frame::new().inner_margin(Margin::symmetric(32, 24)).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(crate::l10n::tr("Keyboard Shortcuts"))
                            .text_style(Text::Display.style())
                            .color(theme.color(Token::Ink1)),
                    );
                    ui.add_space(16.0);
                    // Escape clears a search before it closes the dialog.
                    if !filter.is_empty() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        filter.clear();
                    }
                    let before = filter.clone();
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut filter)
                            .hint_text(crate::l10n::tr("Search shortcuts"))
                            .desired_width(ui.available_width()),
                    );
                    super::label_text_edit(&edit, &filter, crate::l10n::tr("Search shortcuts"));
                    if ui.memory(|m| m.focused().is_none()) {
                        edit.request_focus();
                    }
                    // `?` keeps toggling the dialog while the field has focus.
                    if before.is_empty() && filter == "?" {
                        close = true;
                    }
                });
                ui.separator();
                let shown: Vec<(&str, Vec<Row>)> = sections()
                    .into_iter()
                    .map(|(title, rows)| (title, rows.into_iter().filter(|r| matches(&filter, title, r)).collect()))
                    .filter(|(_, rows): &(&str, Vec<Row>)| !rows.is_empty())
                    .collect();
                // A fixed height, so the dialog doesn't jump while filtering.
                ScrollArea::vertical()
                    .min_scrolled_height(max_body)
                    .max_height(max_body)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if shown.is_empty() {
                            Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(crate::l10n::tr("No shortcuts match"))
                                        .text_style(Text::Prose.style())
                                        .color(theme.color(Token::Ink3)),
                                );
                            });
                        }
                        for (title, rows) in shown {
                            Frame::new().inner_margin(Margin { left: 32, right: 32, top: 20, bottom: 8 }).show(
                                ui,
                                |ui| {
                                    let font = egui::FontId::new(17.0, Text::Headline.font_id().family);
                                    ui.label(egui::RichText::new(title).font(font).color(theme.color(Token::Ink1)));
                                },
                            );
                            for (i, (label, alternatives)) in rows.iter().enumerate() {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                                if i % 2 == 0 {
                                    ui.painter().rect_filled(rect, 0.0, theme.color(Token::Canvas));
                                }
                                let layout = egui::Layout::left_to_right(egui::Align::Center);
                                let mut row = ui
                                    .new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(32.0, 0.0))).layout(layout));
                                row.label(
                                    egui::RichText::new(label.as_str())
                                        .text_style(Text::Prose.style())
                                        .color(theme.color(Token::Ink2)),
                                );
                                row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    chips(ui, alternatives)
                                });
                            }
                        }
                        ui.add_space(16.0);
                    });
                ui.separator();
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(Button::primary(crate::l10n::tr("Close"))).clicked() {
                            close = true;
                        }
                    });
                });
            });
        self.ui.shortcuts_filter = filter;
        if close || resp.should_close() {
            self.ui.shortcuts_open = false;
        }
    }
}
