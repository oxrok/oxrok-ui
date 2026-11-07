//! Checkbox and binary switch.

use super::{paint_check, paint_cross};
use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{
    Color32, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Widget, WidgetInfo, pos2, vec2,
};

/// Disabled checkboxes and switches render at 40 % opacity.
const DISABLED_ALPHA: f32 = 0.4;
/// Knob animation.
const SWITCH_ANIMATION_S: f32 = 0.12;

#[must_use = "add it with `ui.add(…)`"]
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    label: &'a str,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool, label: &'a str) -> Self {
        Self { checked, label }
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        use Token::*;
        const BOX: f32 = 14.0;
        const GAP: f32 = 8.0;
        let galley = ui.painter().layout_no_wrap(self.label.to_owned(), Text::Prose.font_id(), Color32::PLACEHOLDER);
        let size = vec2(BOX + GAP + galley.size().x, BOX.max(galley.size().y));
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
        if response.clicked() {
            *self.checked = !*self.checked;
            response.mark_changed();
        }
        let (enabled, checked) = (ui.is_enabled(), *self.checked);
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, checked, self.label));

        if ui.is_rect_visible(rect) {
            let t = ui.la_theme();
            let alpha = if enabled { 1.0 } else { DISABLED_ALPHA };
            let c = |k| t.color(k).gamma_multiply(alpha);
            let b = Rect::from_min_size(pos2(rect.left(), rect.center().y - BOX / 2.0), vec2(BOX, BOX));
            let p = ui.painter();
            if checked {
                p.rect_filled(b, CornerRadius::same(2), c(Ink1));
                paint_check(p, b.shrink(1.5), c(Canvas), 1.6);
            } else {
                let border = if enabled && response.hovered() { c(RingHover) } else { c(Ink3) };
                p.rect_stroke(b, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
            }
            let pos = pos2(b.right() + GAP, rect.center().y - galley.size().y / 2.0);
            p.galley(pos, galley, c(Ink2));
        }
        response
    }
}

/// A 46 × 22 on/off switch. `label` is its accessible name; place any
/// visible label next to it.
#[must_use = "add it with `ui.add(…)`"]
pub struct BinarySwitch<'a> {
    on: &'a mut bool,
    label: &'a str,
}

impl<'a> BinarySwitch<'a> {
    pub fn new(on: &'a mut bool, label: &'a str) -> Self {
        Self { on, label }
    }
}

impl Widget for BinarySwitch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        use Token::*;
        const SIZE: egui::Vec2 = egui::Vec2::new(46.0, 22.0);
        const KNOB: f32 = 16.0;
        let (rect, mut response) = ui.allocate_exact_size(SIZE, Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let (enabled, on) = (ui.is_enabled(), *self.on);
        response.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, on, self.label));

        if ui.is_rect_visible(rect) {
            let t = ui.la_theme();
            let alpha = if enabled { 1.0 } else { DISABLED_ALPHA };
            let c = |k| t.color(k).gamma_multiply(alpha);
            let k = ui.ctx().animate_bool_with_time(response.id, on, SWITCH_ANIMATION_S);
            let lerp = |a: Color32, b: Color32| a.lerp_to_gamma(b, k);
            let fill = lerp(c(Field), c(Ink1));
            let border = if enabled && response.hovered() { c(RingHover) } else { lerp(c(Seam), c(Ink1)) };
            let p = ui.painter();
            p.rect(rect, CornerRadius::same(u8::MAX), fill, Stroke::new(1.0, border), StrokeKind::Inside);
            let inset = (SIZE.y - KNOB) / 2.0;
            let x = egui::lerp(rect.left() + inset + KNOB / 2.0..=rect.right() - inset - KNOB / 2.0, k);
            let center = pos2(x, rect.center().y);
            p.circle_filled(center, KNOB / 2.0, lerp(c(Ink4), c(Field)));
            if k < 0.5 {
                let cross = Rect::from_center_size(center, vec2(6.0, 6.0));
                paint_cross(p, cross, c(Field).gamma_multiply(1.0 - 2.0 * k), 1.5);
            }
        }
        response
    }
}
