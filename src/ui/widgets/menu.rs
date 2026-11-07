//! Popover menu parts.

use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::{SANS_MEDIUM, Text, family};
use egui::{Align2, FontId, Response, Role, Sense, Ui, WidgetInfo, pos2, vec2};

const ITEM_H: f32 = 32.0;

/// Uppercase 10 pt menu title.
pub fn menu_title(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(text.to_uppercase())
                .font(FontId::new(10.0, family(SANS_MEDIUM)))
                .color(theme.color(Token::Ink3)),
        );
    });
    ui.add_space(4.0);
}

/// A menu item: hover fills `primary` with white text; optional right-hand shortcut.
pub fn menu_item(ui: &mut Ui, text: &str, shortcut: Option<&str>) -> Response {
    let theme = ui.la_theme();
    let w = ui.available_width().max(150.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ITEM_H), Sense::click());
    let enabled = ui.is_enabled();
    resp.widget_info(|| WidgetInfo::labeled(Role::MenuItem, enabled, text));
    let hovered = enabled && resp.hovered();
    let p = ui.painter();
    if hovered {
        p.rect_filled(rect, 0.0, theme.color(Token::Flare));
    }
    let color = if !enabled {
        theme.color(Token::Ink4)
    } else if hovered {
        theme.color(Token::InkOnFlare)
    } else {
        theme.color(Token::Ink2)
    };
    let font = FontId::new(12.0, family(crate::ui::typography::SANS));
    p.text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, text, font, color);
    if let Some(s) = shortcut {
        let c = if hovered { theme.color(Token::InkOnFlare) } else { theme.color(Token::Ink3) };
        p.text(pos2(rect.right() - 16.0, rect.center().y), Align2::RIGHT_CENTER, s, Text::CaptionMed.font_id(), c);
    }
    resp
}

/// A checkable menu item: a check mark in the left margin when `checked`.
pub fn menu_check_item(ui: &mut Ui, text: &str, checked: bool, shortcut: Option<&str>) -> Response {
    let theme = ui.la_theme();
    let resp = menu_item(ui, text, shortcut);
    let enabled = ui.is_enabled();
    resp.widget_info(|| WidgetInfo::selected(Role::MenuItemCheckBox, enabled, checked, text));
    if checked {
        let c = if enabled && resp.hovered() { theme.color(Token::InkOnFlare) } else { theme.color(Token::Ink2) };
        let r = egui::Rect::from_center_size(pos2(resp.rect.left() + 12.0, resp.rect.center().y), vec2(12.0, 12.0));
        crate::ui::icons::check(ui.painter(), r, c);
    }
    resp
}

/// 1 pt separator with 8 pt margins.
pub fn menu_separator(ui: &mut Ui) {
    let theme = ui.la_theme();
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme.color(Token::Seam));
    ui.add_space(8.0);
}

/// A popover frame.
pub fn popover_frame(ui: &Ui) -> egui::Frame {
    let theme = ui.la_theme();
    egui::Frame::new()
        .fill(theme.color(Token::Card))
        .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
        .corner_radius(4)
        .shadow(theme.shallow_shadow())
        .inner_margin(egui::Margin::symmetric(0, 8))
}

/// A shortcut chip such as `R` (tooltips, menus).
pub fn shortcut_chip(ui: &mut Ui, key: &str) {
    let theme = ui.la_theme();
    let galley = ui.painter().layout_no_wrap(key.to_owned(), Text::CaptionMed.font_id(), theme.color(Token::Ink2));
    let size = galley.size() + vec2(10.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        4.0,
        theme.color(Token::Field),
        egui::Stroke::new(1.0, theme.color(Token::Seam)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, egui::Color32::PLACEHOLDER);
}
