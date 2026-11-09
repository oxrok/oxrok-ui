//! Channel enable grid, optionally sectioned by probe
//! profile groups.

use super::Button;
use crate::ui::colors::channel_color;
use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::{SANS_MEDIUM, Text, family};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

pub struct ChipItem {
    /// Device channel index (picks the fallback color).
    pub index: u16,
    /// Text on the chip: the index, or a name such as `CLK1`.
    pub short: String,
    /// Tooltip: the row name, plus the probe description.
    pub name: String,
    /// Accessible label, unique across grids (e.g. `D3`).
    pub label: String,
    pub enabled: bool,
    pub toggleable: bool,
    /// Wire color from a probe profile; `None` = the fallback palette.
    pub color: Option<Color32>,
    /// Marker color: a bar along the chip's top edge.
    pub marker: Option<Color32>,
    /// Low contrast against the theme: draw an outline.
    pub outline: bool,
}

impl ChipItem {
    /// A plain chip: index label, fallback color, no marker.
    pub fn plain(index: u16, label: String, name: String, enabled: bool) -> Self {
        Self {
            index,
            short: index.to_string(),
            name,
            label,
            enabled,
            toggleable: true,
            color: None,
            marker: None,
            outline: false,
        }
    }
}

/// A section of the grid (one probe-profile group).
pub struct ChipGroup {
    /// `None`: no heading (a grid without a profile).
    pub name: Option<String>,
    pub marker: Option<Color32>,
    pub marker_outline: bool,
    pub items: Vec<ChipItem>,
}

#[derive(Default, Debug, PartialEq)]
pub struct GridResponse {
    /// Index of the toggled chip, counting across all groups in order.
    pub toggled: Option<usize>,
    pub all: bool,
    pub clear: bool,
    /// The header's extra button clicked, by index (see
    /// [`channel_grid_section`]).
    pub extra: Option<usize>,
}

const CHIP_H: f32 = 24.0;
const ROW_GAP: f32 = 8.0;
const MIN_COL_GAP: f32 = 4.0;
const MARKER_H: f32 = 3.0;

/// The section header (title + All / Clear) and one ungrouped chip grid.
pub fn channel_grid(ui: &mut Ui, title: &str, items: Vec<ChipItem>) -> GridResponse {
    channel_grid_groups(ui, title, vec![ChipGroup { name: None, marker: None, marker_outline: false, items }])
}

/// The section header and one sub-grid per group, each with a heading line
/// (marker swatch + group name) when the group has a name.
pub fn channel_grid_groups(ui: &mut Ui, title: &str, groups: Vec<ChipGroup>) -> GridResponse {
    channel_grid_section(ui, title, groups, &[])
}

/// [`channel_grid_groups`] with extra buttons left of All, in order
/// (*Group…*, *Probe Colors…*); [`GridResponse::extra`] says which was
/// clicked.
pub fn channel_grid_section(ui: &mut Ui, title: &str, groups: Vec<ChipGroup>, extra: &[&str]) -> GridResponse {
    let mut out = GridResponse::default();
    let theme = ui.la_theme();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title).font(FontId::new(15.0, family(SANS_MEDIUM))).color(theme.color(Token::Ink1)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            out.clear = ui.add(Button::tertiary(crate::l10n::tr("Clear"))).clicked();
            out.all = ui.add(Button::tertiary(crate::l10n::tr("All"))).clicked();
            for (k, label) in extra.iter().enumerate().rev() {
                if ui.add(Button::tertiary(label)).clicked() {
                    out.extra = Some(k);
                }
            }
        });
    });
    ui.add_space(8.0);

    let width = ui.available_width();
    // 8 per row when 8 chips with 4 pt gaps fit, else 4.
    let per_row = if 8.0 * CHIP_H + 7.0 * MIN_COL_GAP <= width { 8 } else { 4 };
    let chip_w = ((width - (per_row - 1) as f32 * MIN_COL_GAP) / per_row as f32).min(CHIP_H * 1.6);
    let gap = if per_row > 1 { (width - per_row as f32 * chip_w) / (per_row - 1) as f32 } else { 0.0 };
    let enabled_ui = ui.is_enabled();
    let mut flat = 0;

    for (gi, group) in groups.iter().enumerate() {
        if let Some(name) = &group.name {
            if gi > 0 {
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                if let Some(m) = group.marker {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 5.0, m);
                    if group.marker_outline {
                        ui.painter().circle_stroke(r.center(), 5.0, Stroke::new(1.0, theme.color(Token::Seam)));
                    }
                }
                ui.label(
                    egui::RichText::new(name).text_style(Text::CaptionMed.style()).color(theme.color(Token::Ink3)),
                );
            });
            ui.add_space(4.0);
        }
        // Lay out cells: wide labels span two columns.
        let mut cells = Vec::new();
        let (mut row, mut col) = (0usize, 0usize);
        for item in &group.items {
            let span = if item.short.chars().count() > 3 { 2 } else { 1 };
            if col + span > per_row {
                row += 1;
                col = 0;
            }
            cells.push((row, col, span));
            col += span;
        }
        let rows = cells.last().map_or(0, |c| c.0 + 1);
        let height = rows as f32 * CHIP_H + rows.saturating_sub(1) as f32 * ROW_GAP;
        let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());

        for (item, &(r, c, span)) in group.items.iter().zip(&cells) {
            let w = chip_w * span as f32 + gap * (span - 1) as f32;
            let rect = Rect::from_min_size(
                pos2(area.left() + c as f32 * (chip_w + gap), area.top() + r as f32 * (CHIP_H + ROW_GAP)),
                vec2(w, CHIP_H),
            );
            let id = ui.scope_id().with(("chip", title, item.index));
            let sense = if item.toggleable { Sense::click() } else { Sense::hover() };
            let resp = ui.interact(rect, id, sense);
            // After the tooltip, which would otherwise become the label.
            let resp = resp.on_hover_text(&item.name);
            resp.widget_info(|| {
                WidgetInfo::selected(Role::CheckBox, enabled_ui && item.toggleable, item.enabled, &item.label)
            });
            if resp.clicked() {
                out.toggled = Some(flat);
            }
            if !item.toggleable {
                resp.clone().on_hover_cursor(egui::CursorIcon::NotAllowed);
            }
            flat += 1;

            let color = item.color.unwrap_or_else(|| channel_color(&theme, item.index));
            let hovered = enabled_ui && resp.hovered();
            // With for colors that vanish on the theme (a
            // black wire on dark): the state must not depend on the color,
            // so enabled gets a bright ring and disabled a dim one.
            let (fill, text, border, border_w) = match (item.enabled, item.outline) {
                (true, false) => (color, theme.color(Token::InkOnSwatch), color, 1.0),
                (false, false) => (Color32::TRANSPARENT, color, theme.color(Token::Seam), 1.0),
                (true, true) => (color, contrasting_text(color), theme.color(Token::Ink2), 1.5),
                (false, true) => (Color32::TRANSPARENT, theme.color(Token::Ink3), theme.color(Token::Seam), 1.0),
            };
            // A dark wire color needs white text even though `on-color` is black.
            let text = if item.enabled && !item.outline { contrasting_text_on(fill, text) } else { text };
            let border = if hovered { theme.color(Token::RingHover) } else { border };
            let p = ui.painter();
            p.rect(rect, CornerRadius::same(4), fill, Stroke::new(border_w, border), StrokeKind::Inside);
            if let Some(m) = item.marker {
                let bar = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(rect.width() - 2.0, MARKER_H));
                p.rect_filled(bar, CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, m);
            }
            p.text(rect.center() + vec2(0.0, 1.0), Align2::CENTER_CENTER, &item.short, Text::Prose.font_id(), text);
        }
        if gi + 1 < groups.len() {
            ui.add_space(4.0);
        }
    }
    out
}

/// Relative luminance in 0–1 (sRGB approximation, enough to pick text color).
fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

/// Black or white, whichever reads on `fill`.
fn contrasting_text(fill: Color32) -> Color32 {
    if luminance(fill) < 0.5 { Color32::WHITE } else { Color32::BLACK }
}

/// `text` unless `fill` is too dark for it.
fn contrasting_text_on(fill: Color32, text: Color32) -> Color32 {
    if luminance(fill) < 0.3 { Color32::WHITE } else { text }
}
