//! Buttons: primary, secondary, round tertiary.

use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{Color32, CornerRadius, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo, vec2};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    /// A destructive primary action: `negative` fill, white text.
    Destructive,
    Secondary,
    TertiaryRound,
}

/// A text button. Disable it with `ui.add_enabled(false, …)`.
#[must_use = "add it with `ui.add(…)`"]
pub struct Button<'a> {
    text: &'a str,
    kind: ButtonKind,
    selected: bool,
    min_width: f32,
}

impl<'a> Button<'a> {
    pub fn primary(text: &'a str) -> Self {
        Self { text, kind: ButtonKind::Primary, selected: false, min_width: 0.0 }
    }

    pub fn destructive(text: &'a str) -> Self {
        Self { text, kind: ButtonKind::Destructive, selected: false, min_width: 0.0 }
    }

    pub fn secondary(text: &'a str) -> Self {
        Self { text, kind: ButtonKind::Secondary, selected: false, min_width: 0.0 }
    }

    pub fn tertiary(text: &'a str) -> Self {
        Self { text, kind: ButtonKind::TertiaryRound, selected: false, min_width: 0.0 }
    }

    /// At least this wide, e.g. so a label change doesn't resize it.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// Selected/active look (secondary and tertiary only).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

struct Look {
    fill: Color32,
    stroke: Option<Color32>,
    text: Color32,
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        use Token::*;
        let (text_style, padding, radius) = match self.kind {
            ButtonKind::Primary | ButtonKind::Destructive | ButtonKind::Secondary => {
                (Text::Lead, vec2(16.0, 10.0), CornerRadius::same(4))
            }
            ButtonKind::TertiaryRound => (Text::CaptionMed, vec2(8.0, 3.0), CornerRadius::same(u8::MAX)),
        };
        let galley = ui.painter().layout_no_wrap(self.text.to_owned(), text_style.font_id(), Color32::PLACEHOLDER);
        let size = galley.size() + 2.0 * padding;
        let size = vec2(size.x.max(self.min_width), size.y);
        let (rect, response) = ui.allocate_exact_size(size.max(Vec2::splat(0.0)), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, self.selected, self.text));

        if ui.is_rect_visible(rect) {
            let t = ui.la_theme();
            let c = |k| t.color(k);
            let hovered = enabled && response.hovered();
            let mut look = match self.kind {
                ButtonKind::Primary => Look {
                    fill: if hovered { c(Slate).gamma_multiply(0.8) } else { c(Slate) },
                    stroke: None,
                    text: c(InkOnSlate),
                },
                ButtonKind::Destructive => Look {
                    fill: if hovered { c(Alert).gamma_multiply(0.85) } else { c(Alert) },
                    stroke: None,
                    text: Color32::WHITE,
                },
                ButtonKind::Secondary if self.selected => {
                    Look { fill: c(Slate), stroke: Some(c(Slate)), text: c(Deck) }
                }
                ButtonKind::Secondary => Look {
                    fill: c(Deck),
                    stroke: Some(if hovered { c(RingFocus) } else { c(Seam) }),
                    text: if enabled { c(Ink2) } else { c(Ink4) },
                },
                ButtonKind::TertiaryRound if self.selected => {
                    Look { fill: c(Slate), stroke: Some(c(Slate)), text: c(InkOnSlate) }
                }
                ButtonKind::TertiaryRound => {
                    Look { fill: c(Field), stroke: Some(if hovered { c(RingHot) } else { c(Seam) }), text: c(Ink2) }
                }
            };
            if !enabled && self.kind != ButtonKind::Secondary {
                look.fill = look.fill.gamma_multiply(0.3);
                look.text = look.text.gamma_multiply(0.3);
                look.stroke = look.stroke.map(|s| s.gamma_multiply(0.3));
            }
            let stroke = look.stroke.map_or(Stroke::NONE, |c| Stroke::new(1.0, c));
            ui.painter().rect(rect, radius, look.fill, stroke, StrokeKind::Inside);
            let pos = rect.center() - galley.size() / 2.0;
            ui.painter().galley(pos, galley, look.text);
        }
        response
    }
}
