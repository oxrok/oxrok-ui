//! Reusable widgets.
//!
//! Every interactive widget sets an accessible label so `egui_kittest` can
//! find it.

pub mod buttons;
pub mod chips;
pub mod menu;
pub mod segmented;
pub mod select;
pub mod stepper;
pub mod time_input;
pub mod toggles;

pub use buttons::{Button, ButtonKind};
pub use chips::{ChipGroup, ChipItem, GridResponse, channel_grid, channel_grid_groups, channel_grid_section};
pub use menu::{menu_check_item, menu_item, menu_separator, menu_title, popover_frame, shortcut_chip};
pub use segmented::{IconFn, Segment, segmented, segmented_icons};
pub use select::Dropdown;
pub use stepper::stepper;
pub use time_input::{number_input, parse_si, parsed_input, text_input, time_input};
pub use toggles::{BinarySwitch, Checkbox};

use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};

/// A ✓ drawn as two strokes inside `rect`.
pub(crate) fn paint_check(painter: &Painter, rect: Rect, color: Color32, width: f32) {
    let p = |x: f32, y: f32| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    painter.line(vec![p(0.18, 0.52), p(0.42, 0.75), p(0.82, 0.28)], Stroke::new(width, color));
}

/// A ✕ drawn as two strokes inside `rect`.
pub(crate) fn paint_cross(painter: &Painter, rect: Rect, color: Color32, width: f32) {
    let stroke = Stroke::new(width, color);
    painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
}

/// A chevron centered at `c`, pointing down (or up when `up`).
pub(crate) fn paint_chevron(painter: &Painter, c: Pos2, half_width: f32, up: bool, color: Color32) {
    let dy = if up { -half_width / 2.0 } else { half_width / 2.0 };
    painter.line(
        vec![pos2(c.x - half_width, c.y - dy), pos2(c.x, c.y + dy), pos2(c.x + half_width, c.y - dy)],
        Stroke::new(1.5, color),
    );
}
