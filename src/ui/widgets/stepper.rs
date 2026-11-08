//! Stepper `[−][ value unit][+]`.

use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{Align2, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

const H: f32 = 40.0;
const FIELD_W: f32 = 110.0;

/// Returns `-1`, `0` or `+1` for the step pressed. `label` is the accessible name.
pub fn stepper(ui: &mut Ui, label: &str, value_text: &str, can_dec: bool, can_inc: bool) -> i32 {
    let theme = ui.la_theme();
    let total = vec2(H * 2.0 + FIELD_W, H);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    let p = ui.painter();
    let border = theme.color(Token::Seam);
    p.rect(rect, CornerRadius::same(4), theme.color(Token::Deck), Stroke::new(1.0, border), StrokeKind::Inside);
    let minus = Rect::from_min_size(rect.min, vec2(H, H));
    let plus = Rect::from_min_size(pos2(rect.right() - H, rect.top()), vec2(H, H));
    for x in [minus.right(), plus.left()] {
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, border));
    }
    p.text(rect.center(), Align2::CENTER_CENTER, value_text, Text::Prose.font_id(), theme.color(Token::Ink2));

    let mut step = 0;
    let enabled = ui.is_enabled();
    for (r, sign, ok, name) in [(minus, -1, can_dec, "Decrease"), (plus, 1, can_inc, "Increase")] {
        let resp: Response =
            ui.interact(r, ui.scope_id().with((label, name)), if ok { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| {
            WidgetInfo::labeled(Role::Button, enabled && ok, format!("{} {}", crate::l10n::tr(name), label))
        });
        let c = if !ok || !enabled {
            theme.color(Token::Ink5)
        } else if resp.hovered() {
            theme.color(Token::Ink1)
        } else {
            theme.color(Token::Ink3)
        };
        let ctr = r.center();
        ui.painter().line_segment([ctr - vec2(6.0, 0.0), ctr + vec2(6.0, 0.0)], Stroke::new(1.5, c));
        if sign > 0 {
            ui.painter().line_segment([ctr - vec2(0.0, 6.0), ctr + vec2(0.0, 6.0)], Stroke::new(1.5, c));
        }
        if resp.clicked() {
            step = sign;
        }
    }
    step
}
