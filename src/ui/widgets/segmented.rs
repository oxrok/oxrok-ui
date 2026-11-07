//! Large segmented control.

use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::{SANS_MEDIUM, family};
use egui::{Align2, CornerRadius, FontId, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

const H: f32 = 46.0;

pub struct Segment<'a> {
    pub label: &'a str,
    pub enabled: bool,
    pub tooltip: &'a str,
}

/// An icon drawn in its bounding square (see `crate::ui::icons`).
pub type IconFn = fn(&egui::Painter, Rect, egui::Color32);

const ICON: f32 = 14.0;
const ICON_GAP: f32 = 6.0;

/// Returns `true` when the selection changed.
pub fn segmented(ui: &mut Ui, id_salt: &str, segments: &[Segment<'_>], selected: &mut usize) -> bool {
    segmented_impl(ui, id_salt, segments, None, selected)
}

/// [`segmented`] with an icon before each label. The label
/// font shrinks, 15 pt down to 12 pt, when icon and label don't fit.
pub fn segmented_icons(
    ui: &mut Ui,
    id_salt: &str,
    segments: &[Segment<'_>],
    icons: &[IconFn],
    selected: &mut usize,
) -> bool {
    segmented_impl(ui, id_salt, segments, Some(icons), selected)
}

fn segmented_impl(
    ui: &mut Ui,
    id_salt: &str,
    segments: &[Segment<'_>],
    icons: Option<&[IconFn]>,
    selected: &mut usize,
) -> bool {
    let theme = ui.la_theme();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, H), Sense::hover());
    let n = segments.len().max(1) as f32;
    let seg_w = rect.width() / n;
    let enabled_ui = ui.is_enabled();
    let mut changed = false;
    // With icons, one label size for every segment: the largest that fits
    // them all, 15 pt down to 12 pt.
    let font = |size: f32| FontId::new(size, family(SANS_MEDIUM));
    let mut size = 15.0;
    if icons.is_some() {
        let room = seg_w - 12.0 - ICON - ICON_GAP;
        let widest = |size: f32| {
            segments
                .iter()
                .map(|s| ui.painter().layout_no_wrap(s.label.into(), font(size), egui::Color32::WHITE).size().x)
                .fold(0.0, f32::max)
        };
        while size > 12.0 && widest(size) > room {
            size -= 0.5;
        }
    }
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, theme.color(Token::Seam)),
        StrokeKind::Inside,
    );
    for (i, seg) in segments.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * seg_w, rect.top()), vec2(seg_w, H));
        let on = *selected == i;
        let ok = enabled_ui && seg.enabled;
        let resp =
            ui.interact(r, ui.make_persistent_id((id_salt, i)), if ok { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, ok, on, seg.label));
        let resp = if seg.tooltip.is_empty() { resp } else { resp.on_hover_text(seg.tooltip) };
        let p = ui.painter();
        if on {
            p.rect_filled(r.shrink(1.0), CornerRadius::same(2), theme.color(Token::Slate));
        } else if ok && resp.hovered() {
            p.rect_filled(r.shrink(1.0), CornerRadius::same(2), theme.color(Token::Lift));
        }
        if i > 0 {
            p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
        }
        let color = if on {
            theme.color(Token::InkOnSlate)
        } else if ok {
            theme.color(Token::Ink3)
        } else {
            theme.color(Token::Ink5)
        };
        match icons.and_then(|icons| icons.get(i)) {
            Some(icon) => {
                let galley = p.layout_no_wrap(seg.label.into(), font(size), color);
                let w = ICON + ICON_GAP + galley.size().x;
                let left = r.center().x - w / 2.0;
                icon(p, Rect::from_min_size(pos2(left, r.center().y - ICON / 2.0), vec2(ICON, ICON)), color);
                let at = pos2(left + ICON + ICON_GAP, r.center().y - galley.size().y / 2.0);
                p.galley(at, galley, color);
            }
            None => {
                p.text(r.center(), Align2::CENTER_CENTER, seg.label, font(size), color);
            }
        }
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}
