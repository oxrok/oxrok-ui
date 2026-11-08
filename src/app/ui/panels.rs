//! The Timing Markers and Measurements Inspector tabs.

use super::markers::{c32, marker_color};
use super::ranges::{icon_button, metric_text, metrics_tsv};
use crate::app::LaApp;
use crate::core::format::si_sig;
use crate::core::markers::{MarkerKind, MarkerRef};
use crate::core::ranges::RANGE_COLORS;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_item, popover_frame};
use egui::{Align2, Color32, FontId, Popup, Rect, Response, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

/// Row height.
const ROW_H: f32 = 36.0;

/// What a row was asked to do.
enum RowEvent {
    Click(egui::Modifiers),
    DoubleClick,
    Goto,
    GotoDouble,
}

/// A panel list row: background states, the text, and the hover actions
/// ("go to" arrow and `⋮`). Returns the row and kebab responses.
fn list_row(
    ui: &mut Ui,
    id: egui::Id,
    indent: f32,
    selected: bool,
    swatch: Option<(Color32, bool)>,
    text: &str,
    note: Option<&str>,
) -> (Response, Option<RowEvent>, Option<Response>) {
    let theme = ui.la_theme();
    let h = if note.is_some() { ROW_H + 16.0 } else { ROW_H };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, text));
    let hovered = ui.rect_contains_pointer(rect);
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, theme.color(Token::Card));
    } else if hovered {
        p.rect_filled(rect, 0.0, theme.color(Token::Lift));
    }
    let mut x = rect.left() + 16.0 + indent;
    let cy = rect.top() + ROW_H / 2.0;
    if let Some((c, pennant)) = swatch {
        let r = Rect::from_center_size(pos2(x + 5.0, cy), vec2(10.0, 10.0));
        if pennant {
            icons::pennant(p, r.expand(2.0), c);
        } else {
            p.rect_filled(r, 1.0, c);
        }
        x += 18.0;
    }
    p.text(pos2(x, cy), Align2::LEFT_CENTER, text, Text::Prose.font_id(), theme.color(Token::Ink2));
    if let Some(n) = note {
        p.text(
            pos2(x, cy + 16.0),
            Align2::LEFT_CENTER,
            n,
            FontId::new(12.0, Text::Prose.font_id().family),
            theme.color(Token::Ink3),
        );
    }
    let mut event = None;
    let mut kebab = None;
    if hovered {
        // Hover actions, right-aligned.
        let k = Rect::from_center_size(pos2(rect.right() - 16.0, cy), vec2(18.0, 18.0));
        let g = Rect::from_center_size(pos2(rect.right() - 40.0, cy), vec2(18.0, 18.0));
        let kr = ui.interact(k, id.with("kebab"), Sense::click());
        kr.widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::l10n::tr("More")));
        icons::kebab(ui.painter(), k, theme.color(Token::Ink2));
        let gr = ui.interact(g, id.with("goto"), Sense::click());
        gr.widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::l10n::tr("Go to {}").replacen("{}", text, 1)));
        icons::goto_arrow(ui.painter(), g, theme.color(Token::Ink2));
        if gr.double_clicked() {
            event = Some(RowEvent::GotoDouble);
        } else if gr.clicked() {
            event = Some(RowEvent::Goto);
        }
        kebab = Some(kr);
    }
    if event.is_none() {
        if resp.double_clicked() {
            event = Some(RowEvent::DoubleClick);
        } else if resp.clicked() {
            event = Some(RowEvent::Click(ui.input(|i| i.modifiers)));
        }
    }
    (resp, event, kebab)
}

/// The `(?)` help icon of panel headers.
pub(crate) fn help_icon(ui: &mut Ui, tip: &str) {
    let theme = ui.la_theme();
    let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().circle_stroke(r.center(), 6.5, Stroke::new(1.0, theme.color(Token::Ink3)));
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        "?",
        FontId::new(9.0, Text::Prose.font_id().family),
        theme.color(Token::Ink3),
    );
    resp.on_hover_text(tip);
}

/// An empty-state illustration and text.
fn empty_state(ui: &mut Ui, text: &str, markers: bool) {
    let theme = ui.la_theme();
    ui.add_space(40.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::hover());
    let c = theme.color(Token::Ink4);
    let mid = r.center();
    let wave = vec![
        mid + vec2(-60.0, 15.0),
        mid + vec2(-30.0, 15.0),
        mid + vec2(-30.0, -5.0),
        mid + vec2(0.0, -5.0),
        mid + vec2(0.0, 15.0),
        mid + vec2(30.0, 15.0),
        mid + vec2(30.0, -5.0),
        mid + vec2(60.0, -5.0),
    ];
    ui.painter().line(wave, Stroke::new(1.5, c));
    if markers {
        for dx in [-40.0, 20.0, 45.0] {
            icons::pennant(ui.painter(), Rect::from_min_size(mid + vec2(dx, -30.0), vec2(14.0, 40.0)), c);
        }
    } else {
        let band = Rect::from_min_max(mid + vec2(-30.0, -20.0), mid + vec2(30.0, 20.0));
        ui.painter().rect_filled(band, 0.0, theme.color(Token::Mark).gamma_multiply(0.2));
    }
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(text).color(theme.color(Token::Ink3)));
    });
}

impl LaApp {
    /// Header actions: the `+` add menu.
    pub(crate) fn markers_panel_actions(&mut self, ui: &mut Ui) {
        let add = icon_button(ui, crate::l10n::tr("Add Timing Marker"), icons::plus)
            .on_hover_text(crate::l10n::tr("Add Timing Markers"));
        let mut pick = None;
        Popup::menu(&add).frame(popover_frame(ui)).width(240.0).show(|ui| {
            if menu_item(ui, crate::l10n::tr("Add Marker Pair"), Some("Shift+M")).clicked() {
                pick = Some(true);
            }
            if menu_item(ui, crate::l10n::tr("Add Single Marker"), Some("M")).clicked() {
                pick = Some(false);
            }
        });
        if let Some(pair) = pick {
            self.begin_marker_placement(pair);
        }
    }

    /// The list.
    pub(crate) fn markers_panel(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_ref() else { return };
        if s.markers.items.is_empty() {
            empty_state(ui, crate::l10n::tr("Use the + button to place timing markers"), true);
            return;
        }
        let items = s.markers.items.clone();
        let placing = self.ui.placing.map(|p| p.id);
        let mut events: Vec<(MarkerRef, RowEvent)> = Vec::new();
        let mut kebabs: Vec<(MarkerRef, Response)> = Vec::new();
        let mut toggle = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        for item in items.iter().filter(|i| Some(i.id) != placing) {
            let id = ui.make_persistent_id(("marker_row", item.id));
            let r = MarkerRef { id: item.id, side: item.is_pair().then_some(0) };
            match item.kind {
                MarkerKind::Single { t } => {
                    let text = format!("{} = {}", item.name(), self.time_label(t));
                    let (_, ev, k) = list_row(
                        ui,
                        id,
                        0.0,
                        item.selected,
                        Some((marker_color(item.color), false)),
                        &text,
                        item.note.as_deref(),
                    );
                    events.extend(ev.map(|e| (r, e)));
                    kebabs.extend(k.map(|k| (r, k)));
                }
                MarkerKind::Pair { t, expanded } => {
                    let d = (t[1] - t[0]).as_secs_f64();
                    let hz = if d > 0.0 { format!(" ({})", si_sig(1.0 / d, "Hz", 4)) } else { String::new() };
                    let text = format!("{} → Δ{}{hz}", item.name(), si_sig(d, "s", 7));
                    // The disclosure triangle sits left of the pennant.
                    let (resp, ev, k) = list_row(
                        ui,
                        id,
                        14.0,
                        item.selected,
                        Some((marker_color(item.color), true)),
                        &text,
                        item.note.as_deref(),
                    );
                    let tri = Rect::from_center_size(
                        pos2(resp.rect.left() + 18.0, resp.rect.top() + ROW_H / 2.0),
                        vec2(12.0, 12.0),
                    );
                    let tr = ui.interact(tri, id.with("disclosure"), Sense::click());
                    tr.widget_info(|| {
                        WidgetInfo::selected(
                            Role::Button,
                            true,
                            expanded,
                            crate::l10n::tr("Expand {}").replacen("{}", &item.name(), 1),
                        )
                    });
                    let c = ui.la_theme().color(Token::Ink3);
                    let pts = if expanded {
                        vec![
                            tri.left_top() + vec2(2.0, 3.0),
                            tri.right_top() + vec2(-2.0, 3.0),
                            tri.center_bottom() - vec2(0.0, 2.0),
                        ]
                    } else {
                        vec![
                            tri.left_top() + vec2(3.0, 2.0),
                            tri.left_bottom() + vec2(3.0, -2.0),
                            tri.right_center() - vec2(2.0, 0.0),
                        ]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
                    if tr.clicked() {
                        toggle = Some(item.id);
                    } else {
                        events.extend(ev.map(|e| (r, e)));
                    }
                    kebabs.extend(k.map(|k| (r, k)));
                    if expanded {
                        for side in [0u8, 1] {
                            let sr = MarkerRef { id: item.id, side: Some(side) };
                            let text = format!(
                                "{}-{} = {}",
                                item.label,
                                if side == 0 { 'A' } else { 'B' },
                                self.time_label(t[side as usize])
                            );
                            let (_, ev, k) = list_row(
                                ui,
                                id.with(side),
                                32.0,
                                item.selected,
                                Some((marker_color(item.color), false)),
                                &text,
                                None,
                            );
                            events.extend(ev.map(|e| (sr, e)));
                            kebabs.extend(k.map(|k| (sr, k)));
                        }
                    }
                }
            }
        }
        if let Some(id) = toggle
            && let Some(MarkerKind::Pair { expanded, .. }) =
                self.session.as_mut().and_then(|s| s.markers.get_mut(id)).map(|i| &mut i.kind)
        {
            *expanded = !*expanded;
        }
        for (r, ev) in events {
            let Some(item) = self.session.as_ref().and_then(|s| s.markers.get(r.id).cloned()) else { continue };
            let (a, b) = item.span();
            match ev {
                // In the Inspector: a click replaces the selection.
                RowEvent::Click(m) => {
                    let s = self.session.as_mut().unwrap();
                    if m.shift
                        && let Some(last) = self.ui.last_marker_click
                    {
                        s.markers.select_range(last, r.id);
                    } else if m.command {
                        if let Some(i) = s.markers.get_mut(r.id) {
                            i.selected = !i.selected;
                        }
                    } else {
                        s.markers.select_only(r.id);
                    }
                    self.ui.last_marker_click = Some(r.id);
                }
                RowEvent::DoubleClick | RowEvent::Goto => self.center_on(item.time(r.side)),
                RowEvent::GotoDouble => {
                    if item.is_pair() {
                        self.zoom_on(a, b);
                    } else {
                        self.center_on(a);
                    }
                }
            }
        }
        for (r, k) in kebabs {
            let pos = k.rect.left_bottom() - vec2(250.0, 0.0);
            self.marker_menu_in(ui, Popup::menu(&k), r, pos);
        }
    }

    /// Header actions.
    pub(crate) fn ranges_panel_actions(&mut self, ui: &mut Ui) {
        let add = icon_button(ui, crate::l10n::tr("Add Measurement"), icons::plus);
        let mut pick = false;
        Popup::menu(&add).frame(popover_frame(ui)).width(240.0).show(|ui| {
            let r = menu_item(ui, crate::l10n::tr("Add Measurement"), Some("R"));
            if r.on_hover_text(crate::l10n::tr(
                "Drag across a channel, or click its start and then its end. Shift+drag works any time.",
            ))
            .clicked()
            {
                pick = true;
            }
        });
        if pick {
            self.ui.range_place = None;
            self.begin_range_placement();
        }
    }

    /// The list.
    pub(crate) fn ranges_panel(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_ref() else { return };
        if s.ranges.items.is_empty() {
            empty_state(ui, crate::l10n::tr("Use the + button to add a measurement"), false);
            return;
        }
        let items = s.ranges.items.clone();
        let several = items.iter().filter(|r| r.selected).count() > 1;
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut act: Option<(u32, &str)> = None;
        for r in &items {
            let values = self.range_metrics(r.id);
            let dt = values.first().and_then(|v| v.1).unwrap_or(0.0);
            let text = format!("{}   Δ{}", r.name, si_sig(dt, "s", 7));
            let id = ui.make_persistent_id(("range_row", r.id));
            let (resp, ev, k) = list_row(
                ui,
                id,
                14.0,
                r.selected,
                Some((c32(RANGE_COLORS[r.color % RANGE_COLORS.len()]), false)),
                &text,
                None,
            );
            let tri =
                Rect::from_center_size(pos2(resp.rect.left() + 18.0, resp.rect.top() + ROW_H / 2.0), vec2(12.0, 12.0));
            let tr = ui.interact(tri, id.with("disclosure"), Sense::click());
            tr.widget_info(|| {
                WidgetInfo::selected(
                    Role::Button,
                    true,
                    r.expanded,
                    crate::l10n::tr("Expand {}").replacen("{}", &r.name, 1),
                )
            });
            let c = ui.la_theme().color(Token::Ink3);
            let pts = if r.expanded {
                vec![
                    tri.left_top() + vec2(2.0, 3.0),
                    tri.right_top() + vec2(-2.0, 3.0),
                    tri.center_bottom() - vec2(0.0, 2.0),
                ]
            } else {
                vec![
                    tri.left_top() + vec2(3.0, 2.0),
                    tri.left_bottom() + vec2(3.0, -2.0),
                    tri.right_center() - vec2(2.0, 0.0),
                ]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
            if tr.clicked() {
                act = Some((r.id, "toggle"));
            } else if let Some(ev) = ev {
                act = Some((
                    r.id,
                    match ev {
                        RowEvent::Click(m) if m.command => "toggle_select",
                        RowEvent::Click(_) => "select",
                        RowEvent::Goto => "goto",
                        RowEvent::DoubleClick | RowEvent::GotoDouble => "zoom",
                    },
                ));
            }
            if r.expanded {
                let theme = ui.la_theme();
                for (m, v) in values.iter().skip(1) {
                    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                    resp.on_hover_text(m.tooltip());
                    let text = format!("{} = {}", m.plain_label(), metric_text(*m, *v));
                    ui.painter().text(
                        rr.left_center() + vec2(64.0, 0.0),
                        Align2::LEFT_CENTER,
                        &text,
                        Text::Prose.font_id(),
                        theme.color(Token::Ink3),
                    );
                }
            }
            if let Some(k) = k {
                Popup::menu(&k).frame(popover_frame(ui)).width(220.0).show(|ui| {
                    if menu_item(ui, crate::l10n::tr("Go to Range"), None).clicked() {
                        act = Some((r.id, "goto"));
                    }
                    if menu_item(ui, crate::l10n::tr("Zoom on Range"), None).clicked() {
                        act = Some((r.id, "zoom"));
                    }
                    if menu_item(ui, crate::l10n::tr("Copy"), None).clicked() {
                        act = Some((r.id, "copy"));
                    }
                    if menu_item(ui, crate::l10n::tr("Export Range…"), None).clicked() {
                        act = Some((r.id, "export"));
                    }
                    let del = if several && r.selected {
                        crate::l10n::tr("Delete Selected")
                    } else {
                        crate::l10n::tr("Delete")
                    };
                    if menu_item(ui, del, None).clicked() {
                        act = Some((r.id, "delete"));
                    }
                    if menu_item(ui, crate::l10n::tr("Delete All"), None).clicked() {
                        act = Some((r.id, "delete_all"));
                    }
                });
            }
        }
        let Some((id, a)) = act else { return };
        let Some(r) = self.session.as_ref().and_then(|s| s.ranges.get(id).cloned()) else { return };
        match a {
            "toggle" => {
                if let Some(x) = self.session.as_mut().unwrap().ranges.get_mut(id) {
                    x.expanded = !x.expanded;
                }
            }
            "export" => self.open_export_for_range(id),
            "select" => self.session.as_mut().unwrap().ranges.select_only(id),
            "toggle_select" => {
                if let Some(x) = self.session.as_mut().unwrap().ranges.get_mut(id) {
                    x.selected = !x.selected;
                }
            }
            "goto" => {
                self.center_on(r.t.0 + crate::core::CaptureTime::from_secs_f64((r.t.1 - r.t.0).as_secs_f64() / 2.0))
            }
            "zoom" => self.zoom_on(r.t.0, r.t.1),
            "copy" => {
                let values = self.range_metrics(id);
                ui.ctx().copy_text(metrics_tsv(&values));
            }
            "delete" => {
                let removed = if several && r.selected {
                    self.session
                        .as_ref()
                        .map(|s| s.ranges.items.iter().filter(|x| x.selected).cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                } else {
                    self.session.as_ref().and_then(|s| s.ranges.get(id)).cloned().into_iter().collect()
                };
                let s = self.session.as_mut().unwrap();
                if several && r.selected {
                    s.ranges.remove_selected();
                } else {
                    s.ranges.remove(id);
                }
                self.record_range_delete(removed);
            }
            "delete_all" => {
                let n = self.session.as_ref().map_or(0, |s| s.ranges.items.len());
                self.ui.confirm = Some(super::dialogs::Confirm::delete_all_ranges(n));
            }
            _ => {}
        }
    }
}
