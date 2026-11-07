//! Dropdown selector.

use super::{paint_check, paint_chevron};
use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{
    Color32, CornerRadius, Event, Frame, Key, Margin, Popup, PopupCloseBehavior, Rect, RectAlign, Response, Role,
    ScrollArea, Sense, Stroke, StrokeKind, Ui, Widget, WidgetInfo, pos2, vec2,
};

const HEIGHT: f32 = 40.0;
const ROW_HEIGHT: f32 = 44.0;
const PAD_X: f32 = 12.0;
const CHECK_COLUMN: f32 = 24.0;
const GAP: f32 = 2.0;
/// The list's vertical padding (the frame's inner margin, top and bottom).
const LIST_PAD: f32 = 8.0;
/// Room kept between the list and the window edge.
const EDGE: f32 = 8.0;

/// A full-width dropdown over `options`. `label` is the accessible name.
///
/// Keyboard while open: ↑/↓ move, Enter selects, Escape closes, typing a
/// letter jumps to the next option starting with it.
#[must_use = "add it with `ui.add(…)`"]
pub struct Dropdown<'a, T: AsRef<str>> {
    label: &'a str,
    selected: &'a mut usize,
    options: &'a [T],
}

impl<'a, T: AsRef<str>> Dropdown<'a, T> {
    pub fn new(label: &'a str, selected: &'a mut usize, options: &'a [T]) -> Self {
        Self { label, selected, options }
    }
}

impl<T: AsRef<str>> Widget for Dropdown<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        use Token::*;
        let enabled = ui.is_enabled();
        let width = ui.available_width();
        let (rect, mut response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click());
        let value = self.options.get(*self.selected).map(|o| o.as_ref().to_owned()).unwrap_or_default();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(Role::ComboBox, enabled, self.label);
            info.current_text_value = Some(value.clone());
            info
        });

        let popup_id = Popup::default_response_id(&response);
        let open = Popup::is_id_open(ui.ctx(), popup_id);
        let t = ui.la_theme();
        let c = |k| t.color(k);

        if ui.is_rect_visible(rect) {
            let alpha = if enabled { 1.0 } else { 0.5 };
            let border = if enabled && (response.hovered() || open) { c(RingFocus) } else { c(Seam) };
            let p = ui.painter();
            p.rect(
                rect,
                CornerRadius::same(4),
                c(Deck).gamma_multiply(alpha),
                Stroke::new(1.0, border.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            if enabled {
                let galley = p.layout_no_wrap(value.clone(), Text::Prose.font_id(), c(Ink2));
                p.galley(
                    pos2(rect.left() + PAD_X, rect.center().y - galley.size().y / 2.0),
                    galley,
                    Color32::PLACEHOLDER,
                );
            }
            let chevron = pos2(rect.right() - PAD_X - 5.0, rect.center().y);
            paint_chevron(p, chevron, 5.0, open, c(Ink3).gamma_multiply(alpha));
        }

        let highlight_id = popup_id.with("highlight");
        let revealed_id = popup_id.with("revealed");
        let n = self.options.len();
        let mut hi = ui.ctx().data(|d| d.get_temp::<usize>(highlight_id)).unwrap_or(*self.selected);
        let mut commit = None;
        let mut moved = false;
        if open {
            ui.input(|i| {
                for e in &i.events {
                    match e {
                        Event::Key { key: Key::ArrowDown, pressed: true, .. } if n > 0 => {
                            hi = (hi + 1).min(n - 1);
                            moved = true;
                        }
                        Event::Key { key: Key::ArrowUp, pressed: true, .. } => {
                            hi = hi.saturating_sub(1);
                            moved = true;
                        }
                        Event::Key { key: Key::Enter, pressed: true, .. } => commit = Some(hi),
                        Event::Text(text) => {
                            let typed = text.to_lowercase();
                            if let Some(j) = (1..=n)
                                .map(|d| (hi + d) % n)
                                .find(|&j| self.options[j].as_ref().to_lowercase().starts_with(&typed))
                            {
                                hi = j;
                                moved = true;
                            }
                        }
                        _ => {}
                    }
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(highlight_id, hi));
        } else {
            ui.ctx().data_mut(|d| {
                d.remove::<usize>(highlight_id);
                d.remove::<bool>(revealed_id);
            });
        }

        // A list taller than the room below opens on the side with more room,
        // limited to it, and scrolls.
        let screen = ui.ctx().content_rect();
        let below = screen.bottom() - rect.bottom() - GAP - EDGE;
        let above = rect.top() - screen.top() - GAP - EDGE;
        let full = n as f32 * ROW_HEIGHT + LIST_PAD;
        let (align, room) = if full <= below || below >= above {
            (RectAlign::BOTTOM_START, below)
        } else {
            (RectAlign::TOP_START, above)
        };
        let max_list = (room - LIST_PAD).max(ROW_HEIGHT);

        // `show` must run every frame: it opens the popup on click.
        let frame = Frame::new()
            .fill(c(Card))
            .stroke(Stroke::new(1.0, c(Seam)))
            .corner_radius(CornerRadius::same(4))
            .shadow(t.shallow_shadow())
            .inner_margin(Margin::symmetric(0, 4));
        Popup::from_toggle_button_response(&response)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .width(width)
            .gap(GAP)
            .align(align)
            .align_alternatives(&[])
            .frame(frame)
            .show(|ui| {
                // Bring the selected row into view once per opening, then
                // follow the keyboard highlight.
                let reveal = !ui.ctx().data(|d| d.get_temp::<bool>(revealed_id).unwrap_or(false));
                ui.ctx().data_mut(|d| d.insert_temp(revealed_id, true));
                ScrollArea::vertical().max_height(max_list).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (j, option) in self.options.iter().enumerate() {
                        let row = option_row(ui, option.as_ref(), j == *self.selected, open && j == hi);
                        if row.clicked() {
                            commit = Some(j);
                        }
                        if (reveal && j == *self.selected) || (moved && j == hi) {
                            row.scroll_to_me(None);
                        }
                    }
                });
            });
        if let Some(j) = commit.filter(|&j| j < n) {
            if j != *self.selected {
                *self.selected = j;
                response.mark_changed();
            }
            Popup::close_id(ui.ctx(), popup_id);
        }
        response
    }
}

fn option_row(ui: &mut Ui, text: &str, selected: bool, highlighted: bool) -> Response {
    use Token::*;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::ListBoxOption, true, selected, text));
    if ui.is_rect_visible(rect) {
        let t = ui.la_theme();
        let p = ui.painter();
        if selected || highlighted || response.hovered() {
            p.rect_filled(rect, CornerRadius::ZERO, t.color(Lift));
        }
        if selected {
            let check = Rect::from_center_size(pos2(rect.left() + PAD_X + 6.0, rect.center().y), vec2(12.0, 12.0));
            paint_check(p, check, t.color(Ink1), 1.5);
        }
        let galley = p.layout_no_wrap(text.to_owned(), Text::Prose.font_id(), t.color(Ink2));
        let pos = pos2(rect.left() + PAD_X + CHECK_COLUMN, rect.center().y - galley.size().y / 2.0);
        p.galley(pos, galley, Color32::PLACEHOLDER);
    }
    response
}
