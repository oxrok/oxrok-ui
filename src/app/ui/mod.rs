//! The screens of the app shell (03, 05, 06, 07, 15).

pub(crate) mod analyzers;
mod bottom;
mod bubbles;
pub(crate) mod buses;
mod capture_opts;
pub(crate) mod commands;
pub(crate) mod datatable;
mod device_opts;
pub(crate) mod dialogs;
pub(crate) mod export;
pub(crate) mod files;
mod graph;
pub(crate) mod help;
pub(crate) mod import;
pub(crate) mod inspector;
#[cfg(all(target_os = "macos", feature = "macos-menu"))]
pub(crate) mod macmenu;
pub(crate) mod markers;
mod menubar;
mod nav;
mod onboarding;
pub(crate) mod palette;
mod panels;
pub(crate) mod probe;
pub(crate) mod ranges;
mod rowctl;
mod rows;
mod sessions;
pub(crate) mod settings;
mod shortcuts;
mod start;
pub(crate) mod trim;

use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use egui::{Rect, Response, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

/// The analog row under the pointer (for `F` and `Ctrl/Cmd+↑/↓`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct HoverRow {
    pub channel: crate::core::ChannelId,
    /// Pointer y from the top of the inner area.
    pub y: f32,
    pub inner_height: f32,
}

/// Where a row was drawn in the last frame (input runs before painting).
/// A `Rect` field for state that derives `Default` (`Rect::NOTHING`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct DefaultRect(pub Rect);

impl Default for DefaultRect {
    fn default() -> Self {
        Self(Rect::NOTHING)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct RowGeom {
    pub channel: crate::core::ChannelId,
    pub cell: Rect,
    pub inner: Rect,
}

impl RowGeom {
    /// The voltage ruler (scale labels) at the left of an analog plot.
    pub fn ruler(&self) -> Rect {
        Rect::from_min_max(self.cell.min, pos2(self.cell.left() + 48.0, self.cell.bottom()))
    }

    pub fn is_analog(&self) -> bool {
        self.channel.kind == crate::core::ChannelKind::Analog
    }
}

/// An active drag on the plot.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct DragState {
    /// Analog row the drag started on.
    pub analog: Option<RowGeom>,
    /// Started on the voltage ruler: no horizontal panning.
    pub on_ruler: bool,
    pub total: egui::Vec2,
    pub vertical: bool,
}

/// How a range zoom picks its axes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum MarqueeKind {
    /// Ctrl/Cmd+Shift: the axis with the larger drag distance.
    Single,
    Fixed(crate::app::settings::MarqueeMode),
}

/// A range zoom being dragged.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Marquee {
    pub start: egui::Pos2,
    pub current: egui::Pos2,
    pub kind: MarqueeKind,
    /// Started with the keyboard modifiers: releasing them cancels.
    pub by_modifiers: bool,
    /// The row the drag started on (Y needs an analog row).
    pub row: Option<RowGeom>,
}

impl Marquee {
    /// The axes this marquee zooms right now.
    pub fn mode(&self) -> crate::app::settings::MarqueeMode {
        use crate::app::settings::MarqueeMode;
        match self.kind {
            MarqueeKind::Fixed(m) => m,
            MarqueeKind::Single => {
                let d = self.current - self.start;
                if d.x.abs() >= d.y.abs() { MarqueeMode::X } else { MarqueeMode::Y }
            }
        }
    }
}

/// The row whose name is being edited.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rename {
    pub channel: crate::core::ChannelId,
    pub text: String,
    /// Focus is requested on the first frame only.
    pub focus: bool,
}

/// A row being dragged by its label.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Reorder {
    pub channel: crate::core::ChannelId,
    /// Pointer y minus the row's top at the press.
    pub grab: f32,
    pub pointer_y: f32,
    /// Moved far enough to be a reorder.
    pub active: bool,
    /// Where it would land: before this row, or last.
    pub drop_before: Option<crate::core::ChannelId>,
}

/// `Cmd` on macOS, `Ctrl` elsewhere, for shortcut hints.
pub(crate) fn cmd_key() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }
}

/// Footer button: 1 pt left border, 16 pt padding, 13 pt text.
pub(crate) fn footer_button(
    ui: &mut Ui,
    text: &str,
    highlighted: bool,
    icon: Option<fn(&egui::Painter, Rect, egui::Color32)>,
) -> Response {
    let theme = ui.la_theme();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), Text::Prose.font_id(), egui::Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let w = galley.size().x + 32.0 + icon_w + 14.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ui.available_height()), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, text));
    let p = ui.painter();
    if highlighted {
        p.rect_filled(rect, 0.0, theme.color(Token::Mark));
    }
    p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
    let color = if highlighted {
        theme.color(Token::InkOnFlare)
    } else if resp.hovered() {
        theme.color(Token::Ink1)
    } else {
        theme.color(Token::Ink3)
    };
    let mut x = rect.left() + 16.0;
    if let Some(icon) = icon {
        icon(p, Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0)), color);
        x += icon_w;
    }
    p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
    let chevron = pos2(rect.right() - 16.0 - 3.0, rect.center().y);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let dy = if open { -2.0 } else { 2.0 };
    p.line(vec![chevron + vec2(-4.0, -dy), chevron + vec2(0.0, dy), chevron + vec2(4.0, -dy)], Stroke::new(1.5, color));
    resp
}

/// Gives a text field an accessible name (kittest's accessibility check).
pub(crate) fn label_text_edit(resp: &Response, text: &str, label: &str) {
    resp.widget_info(|| {
        let mut info = WidgetInfo::text_edit(true, text, text, "");
        info.label = Some(label.to_owned());
        info
    });
}

/// A section label inside panels and dialogs.
pub(crate) fn section_title(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).text_style(Text::Headline.style()).color(theme.color(Token::Ink1)));
}

/// Secondary text.
pub(crate) fn hint(ui: &mut Ui, text: &str) -> Response {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).color(theme.color(Token::Ink3)))
}
