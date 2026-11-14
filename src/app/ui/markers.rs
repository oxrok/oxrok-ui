//! Timing markers on the graph and the Time Zero menu.

use super::graph::Layout;
use crate::app::LaApp;
use crate::core::animator::{Anchor, JUMP_DURATION, ViewRequest};
use crate::core::format::{time_full, wall_clock};
use crate::core::markers::{MARKER_PALETTE, MarkerKind, MarkerRef, SNAP_PT, TimeOrigin, snap_x};
use crate::core::{CaptureTime, ChannelKind, ViewState};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::{SANS_MEDIUM, Text, family};
use crate::ui::widgets::{menu_check_item, menu_item, menu_separator, menu_title, popover_frame};
use egui::{
    Align2, Color32, CornerRadius, FontId, Popup, Pos2, Rect, Response, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2,
};

/// Head sizes.
const HEAD_W: f32 = 17.0;
const HEAD_H: f32 = 16.0;
/// Line hit width for dragging.
const LINE_HIT: f32 = 7.0;
/// Edit popup width.
const EDIT_W: f32 = 250.0;
/// The Timing Markers panel id.
pub(crate) const MARKERS_PANEL: &str = "markers";

/// A marker being placed.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Placing {
    pub id: u32,
    pub pair: bool,
    /// 0: placing (side A of) the item; 1: placing side B.
    pub stage: u8,
    /// The side being placed (pairs swap sides when B passes A).
    pub side: Option<u8>,
    /// Skip the click that started placement.
    pub fresh: bool,
}

pub(crate) fn c32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub(crate) fn marker_color(color: usize) -> Color32 {
    c32(MARKER_PALETTE[color % MARKER_PALETTE.len()])
}

impl LaApp {
    // ---- placement ----

    /// *Add Marker Pair* / *Add Single Marker*: needs data and no recording.
    pub(crate) fn begin_marker_placement(&mut self, pair: bool) {
        self.cancel_marker_placement();
        let Some(s) = self.session.as_mut() else { return };
        if !s.has_data() || s.state.is_busy() {
            return;
        }
        let t = s.view.state().center_time();
        let id = if pair { s.markers.add_pair(t, t) } else { s.markers.add_single(t) };
        s.markers.select_only(id);
        self.ui.placing = Some(Placing { id, pair, stage: 0, side: pair.then_some(0), fresh: true });
    }

    pub(crate) fn cancel_marker_placement(&mut self) {
        if let Some(p) = self.ui.placing.take()
            && let Some(s) = self.session.as_mut()
        {
            s.markers.remove(p.id);
        }
    }

    /// The time under `p`, snapped to the edges of the digital
    /// pulse under the pointer, else the pointer rounded up to a pixel.
    pub(crate) fn snap_time(&self, p: Pos2, l: &Layout, view: &ViewState) -> CaptureTime {
        let px = (p.x - l.plot_rows.left()) as f64;
        let edge = (|| {
            let s = self.session.as_ref()?;
            let g =
                self.ui.row_geometry.iter().find(|g| g.cell.contains(p) && g.channel.kind == ChannelKind::Digital)?;
            let cap = s.capture.as_ref()?.read();
            let rate = cap.sample_rate()?;
            let ch = cap.channel(g.channel)?;
            let sample = view.px_to_time(px).to_sample_floor(rate).max(0) as u64;
            let prev = ch.prev_transition(sample + 1).map(|e| CaptureTime::from_sample(e, rate));
            let next = ch.next_transition(sample).map(|e| CaptureTime::from_sample(e, rate));
            let x = snap_x(px, prev.map(|t| view.time_to_px(t)), next.map(|t| view.time_to_px(t)), SNAP_PT);
            [prev, next].into_iter().flatten().find(|t| view.time_to_px(*t) == x)
        })();
        let t = edge.unwrap_or_else(|| view.px_to_time(px.ceil()));
        t.clamp(view.left_time, view.right_time())
    }

    /// Placement each frame: the line follows the pointer; clicks fix it.
    fn placement(&mut self, ui: &Ui, l: &Layout, view: &ViewState, plot: &Response) {
        let Some(mut p) = self.ui.placing else { return };
        let pointer = self.plot_pointer(ui);
        if let Some(pos) = pointer.filter(|pos| l.plot_rows.contains(*pos)) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            let t = self.snap_time(pos, l, view);
            let s = self.session.as_mut().unwrap();
            if p.pair && p.stage == 0 {
                s.markers.set_time(MarkerRef { id: p.id, side: Some(0) }, t);
                s.markers.set_time(MarkerRef { id: p.id, side: Some(1) }, t);
            } else {
                let r = s.markers.set_time(MarkerRef { id: p.id, side: p.side }, t);
                p.side = r.side;
            }
        }
        let clicked_outside = ui.input(|i| i.pointer.any_click())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !l.plot_rows.contains(pos));
        if p.fresh {
            p.fresh = false;
        } else if plot.clicked() {
            if p.pair && p.stage == 0 {
                p.stage = 1;
                p.side = Some(1);
            } else {
                self.ui.placing = None;
                return; // finalized, still selected (step 5)
            }
        } else if clicked_outside {
            self.ui.placing = Some(p);
            self.cancel_marker_placement();
            return;
        }
        self.ui.placing = Some(p);
    }

    // ---- drawing and interaction ----

    pub(crate) fn markers_ui(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState, plot: &Response) {
        let Some(s) = self.session.as_ref() else { return };
        if !s.has_data() {
            return;
        }
        let theme = ui.la_theme();
        self.placement(ui, l, view, plot);
        self.time_zero_menu(ui, l);

        let s = self.session.as_ref().unwrap();
        let x_of = |t: CaptureTime| l.plot_rows.left() + view.time_to_px(t) as f32;
        let placing = self.ui.placing.map(|p| p.id);
        let lines: Vec<(MarkerRef, String, usize, bool, CaptureTime)> = s
            .markers
            .lines()
            .map(|(r, item)| (r, item.side_name(r.side), item.color, item.selected, item.time(r.side)))
            .collect();
        let pairs: Vec<(usize, CaptureTime, CaptureTime)> = s
            .markers
            .items
            .iter()
            .filter_map(|i| match i.kind {
                MarkerKind::Pair { t, .. } => Some((i.color, t[0], t[1])),
                MarkerKind::Single { .. } => None,
            })
            .collect();

        // Pair spans just under the heads.
        let painter = ui.painter_at(l.ruler);
        for (color, a, b) in &pairs {
            let (xa, xb) = (x_of(*a).max(l.ruler.left()), x_of(*b).min(l.ruler.right()));
            if xb > xa {
                let bar = Rect::from_x_y_ranges(xa..=xb, l.ruler.top() + 21.0..=l.ruler.top() + 24.0);
                painter.rect_filled(bar, 0.0, marker_color(*color).gamma_multiply(0.3));
            }
        }

        let mut clicked: Option<(MarkerRef, egui::Modifiers, bool)> = None;
        let mut menus: Vec<(MarkerRef, Response)> = Vec::new();
        let mut drag: Option<(MarkerRef, Pos2)> = None;
        let mut goto: Option<CaptureTime> = None;
        let mut open_panel = false;
        let mut offscreen = [0.0f32; 2];
        for (r, name, color, selected, t) in &lines {
            let x = x_of(*t);
            let temp = placing == Some(r.id);
            let base = marker_color(*color).gamma_multiply(if temp { 0.6 } else { 1.0 });
            if x < l.plot_rows.left() || x > l.plot_rows.right() {
                // An arrow chip at the ruler edge.
                let left = x < l.plot_rows.left();
                let font = FontId::new(10.0, family(SANS_MEDIUM));
                // The arrow is drawn (the UI font has no ◂ / ▸).
                let text = name.clone();
                let w = ui.painter().layout_no_wrap(text.clone(), font.clone(), Color32::WHITE).size().x + 18.0;
                let k = usize::from(!left);
                let cx = if left { l.ruler.left() + offscreen[k] } else { l.ruler.right() - offscreen[k] - w };
                offscreen[k] += w + 2.0;
                let chip = Rect::from_min_size(pos2(cx, l.ruler.top() + 4.0), vec2(w, 14.0));
                let resp = ui.interact(chip, ui.make_persistent_id(("marker_chip", *r)), Sense::click());
                resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Go to {}").replacen("{}", name, 1)));
                painter.rect_filled(chip, 3.0, base);
                edge_arrow(&painter, chip, left, Color32::WHITE);
                let at = chip.center() + vec2(if left { 5.0 } else { -5.0 }, 0.0);
                painter.text(at, Align2::CENTER_CENTER, text, font, Color32::WHITE);
                if resp.clicked() {
                    goto = Some(*t);
                }
                continue;
            }
            // The line over every row.
            let w = if *selected { 2.0 } else { 1.0 };
            ui.painter_at(l.plot_rows)
                .line_segment([pos2(x, l.rows.top()), pos2(x, l.rows.bottom())], Stroke::new(w, base));
            ui.painter_at(l.ruler)
                .line_segment([pos2(x, l.ruler.top()), pos2(x, l.ruler.bottom())], Stroke::new(w, base));
            if temp {
                continue;
            }
            // The head in the ruler.
            let head = if r.side.is_some() {
                Rect::from_min_size(pos2(x - HEAD_W / 2.0, l.ruler.top() + 4.0), vec2(HEAD_W, HEAD_H))
            } else {
                Rect::from_min_size(pos2(x, l.ruler.top() + 4.0), vec2(HEAD_W, HEAD_H - 2.0))
            };
            let dx = (l.ruler.left() - head.left()).max(0.0) + (l.ruler.right() - head.right()).min(0.0);
            let head = head.translate(vec2(dx, 0.0));
            let resp = ui.interact(head, ui.make_persistent_id(("marker_head", *r)), Sense::click_and_drag());
            resp.widget_info(|| WidgetInfo::selected(Role::Button, true, *selected, name));
            let fill = if resp.hovered() { base.lerp_to_gamma(theme.color(Token::Ink1), 0.3) } else { base };
            let radius =
                if r.side.is_some() { CornerRadius::same(4) } else { CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 } };
            painter.rect_filled(head, radius, fill);
            let label = self.session.as_ref().unwrap().markers.get(r.id).map(|i| i.label.clone()).unwrap_or_default();
            painter.text(
                head.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::new(10.0, family(SANS_MEDIUM)),
                Color32::WHITE,
            );
            let summary = self.marker_summary(*r);
            let resp = resp.on_hover_ui(|ui| {
                for line in &summary {
                    ui.label(egui::RichText::new(line).text_style(Text::Prose.style()));
                }
            });
            // The line itself, for dragging and menus.
            let hit = Rect::from_x_y_ranges(x - LINE_HIT / 2.0..=x + LINE_HIT / 2.0, l.plot_rows.y_range());
            let line = ui.interact(hit, ui.make_persistent_id(("marker_line", *r)), Sense::click_and_drag());
            line.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{name} line")));
            if self.ui.placing.is_none() && (line.hovered() || line.dragged()) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }
            for resp in [resp, line] {
                if resp.double_clicked() {
                    open_panel = true;
                } else if resp.clicked() {
                    let mods = ui.input(|i| i.modifiers);
                    let on_head = resp.rect == head;
                    clicked = Some((*r, mods, on_head));
                }
                if resp.dragged()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    drag = Some((*r, p));
                }
                if resp.drag_stopped() {
                    self.ui.marker_drag = None;
                }
                if resp.secondary_clicked() {
                    let s = self.session.as_mut().unwrap();
                    if !s.markers.get(r.id).is_some_and(|i| i.selected) {
                        s.markers.select_only(r.id);
                    }
                }
                menus.push((*r, resp));
            }
        }

        // Drag, with snapping; a pair keeps A ≤ B.
        if let Some((r, p)) = drag {
            let current = self.ui.marker_drag.filter(|d| d.id == r.id).unwrap_or(r);
            let t = self.snap_time(p, l, view);
            let s = self.session.as_mut().unwrap();
            let moved = s.markers.set_time(current, t);
            // A T0 on that side follows it when the sides swap.
            if moved != current && s.origin == TimeOrigin::Marker(current) {
                s.origin = TimeOrigin::Marker(moved);
            }
            self.ui.marker_drag = Some(moved);
        }
        if let Some((r, mods, on_head)) = clicked {
            self.click_marker(r.id, mods);
            if on_head && !mods.command && !mods.shift {
                let x = x_of(self.session.as_ref().unwrap().markers.time(r).unwrap_or(CaptureTime::ZERO));
                self.ui.marker_edit = Some((r.id, pos2(x - EDIT_W / 2.0, l.ruler.bottom() + 4.0)));
            }
        }
        if open_panel {
            self.settings.inspector.tab = MARKERS_PANEL.into();
            self.settings.inspector.open = true;
        }
        if let Some(t) = goto {
            self.center_on(t);
        }
        for (r, resp) in menus {
            let t = self.session.as_ref().and_then(|s| s.markers.time(r)).unwrap_or(CaptureTime::ZERO);
            let pos = pos2(x_of(t) - EDIT_W / 2.0, l.ruler.bottom() + 4.0);
            self.marker_menu(ui, &resp, r, pos);
        }
        self.marker_edit_popup(ui);
    }

    /// Time, Δ and 1/Δ for pairs, and the note.
    fn marker_summary(&self, r: MarkerRef) -> Vec<String> {
        let Some(s) = self.session.as_ref() else { return Vec::new() };
        let Some(item) = s.markers.get(r.id) else { return Vec::new() };
        let mut out = vec![self.time_label(item.time(r.side))];
        if let MarkerKind::Pair { t, .. } = item.kind {
            let d = t[1] - t[0];
            out.push(format!("Δ {}", time_full(d)));
            if d > CaptureTime::ZERO {
                out.push(format!("{:.2} Hz", 1.0 / d.as_secs_f64()));
            }
        }
        if let Some(n) = &item.note {
            out.push(n.clone());
        }
        out
    }

    /// A time as shown everywhere, relative to T0 or as wall clock.
    pub(crate) fn time_label(&self, t: CaptureTime) -> String {
        let Some(s) = self.session.as_ref() else { return time_full(t) };
        if s.origin == TimeOrigin::WallClock
            && let Some(start) = s.capture.as_ref().and_then(|c| c.read().header.as_ref().map(|h| h.start))
        {
            return wall_clock(&start, t, self.settings.time_labels.format());
        }
        time_full(t - s.t0())
    }

    /// On the graph: click selects (others unchanged), Ctrl/Cmd
    /// toggles, Shift selects the id range from the last click.
    pub(crate) fn click_marker(&mut self, id: u32, mods: egui::Modifiers) {
        let last = self.ui.last_marker_click;
        let Some(s) = self.session.as_mut() else { return };
        if mods.shift
            && let Some(a) = last
        {
            s.markers.select_range(a, id);
        } else if let Some(item) = s.markers.get_mut(id) {
            item.selected = if mods.command { !item.selected } else { true };
        }
        self.ui.last_marker_click = Some(id);
    }

    /// Centers the view on `t` (300 ms).
    pub(crate) fn center_on(&mut self, t: CaptureTime) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        s.follow = false;
        s.view.request(ViewRequest::MoveToTime { time: t, anchor: Anchor::Center, duration: JUMP_DURATION }, now);
    }

    /// Fits `[a, b]` with 5 % padding on each side.
    pub(crate) fn zoom_on(&mut self, a: CaptureTime, b: CaptureTime) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let pad = CaptureTime::from_secs_f64((b - a).as_secs_f64() * 0.05);
        if b <= a {
            return;
        }
        s.follow = false;
        s.view.request(ViewRequest::MoveToRange { t0: a - pad, t1: b + pad, duration: JUMP_DURATION }, now);
    }

    /// head/line (and panel kebab) menu; *Edit* opens the popup at
    /// `edit_pos`.
    pub(crate) fn marker_menu(&mut self, ui: &Ui, resp: &Response, r: MarkerRef, edit_pos: Pos2) {
        let popup = Popup::context_menu(resp);
        self.marker_menu_in(ui, popup, r, edit_pos);
    }

    pub(crate) fn marker_menu_in(&mut self, ui: &Ui, popup: Popup<'_>, r: MarkerRef, edit_pos: Pos2) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(item) = s.markers.get(r.id).cloned() else { return };
        let is_t0 = s.origin == TimeOrigin::Marker(r);
        let two_singles = s.markers.selected_singles().len() == 2;
        // Delete Data relative to this marker (or pair).
        let name = item.side_name(r.side);
        let at = item.time(r.side);
        let trim_items: Vec<super::trim::TrimItem> = if item.is_pair() {
            let (a, b) = item.span();
            vec![
                (tr("Before This Marker"), Some(at), None, name.clone()),
                (tr("After This Marker"), None, Some(at), name.clone()),
                (
                    tr("Outside Pair"),
                    Some(a),
                    Some(b),
                    tr("from {} to {}").replacen("{}", &item.side_name(Some(0)), 1).replacen(
                        "{}",
                        &item.side_name(Some(1)),
                        1,
                    ),
                ),
            ]
        } else {
            vec![
                (tr("Before Marker"), Some(at), None, name.clone()),
                (tr("After Marker"), None, Some(at), name.clone()),
            ]
        };
        let mut trim = None;
        let mut act = None;
        popup.frame(popover_frame(ui)).width(220.0).show(|ui| {
            if menu_item(ui, tr("Edit"), None).clicked() {
                act = Some("edit");
            }
            menu_separator(ui);
            if menu_item(ui, tr("Go to Marker"), None).clicked() {
                act = Some("goto");
            }
            if menu_check_item(ui, tr("Set as t₀"), is_t0, None).clicked() {
                act = Some("t0");
            }
            if item.is_pair() {
                if menu_item(ui, tr("Go to Pair"), None).clicked() {
                    act = Some("goto_pair");
                }
                if menu_item(ui, tr("Zoom on Pair"), None).clicked() {
                    act = Some("zoom_pair");
                }
                if menu_item(ui, tr("Ungroup Pair"), None).clicked() {
                    act = Some("ungroup");
                }
            } else {
                ui.add_enabled_ui(two_singles, |ui| {
                    if menu_item(ui, tr("Create Pair"), None).clicked() {
                        act = Some("pair");
                    }
                });
            }
            if let Some(k) = self.delete_data_submenu(ui, &trim_items) {
                trim = Some(k);
            }
            if menu_item(ui, tr("Delete"), Some("Del")).clicked() {
                act = Some("delete");
            }
            if menu_item(ui, tr("Delete All"), None).clicked() {
                act = Some("delete_all");
            }
        });
        if let Some(k) = trim {
            self.request_trim(&trim_items[k]);
            return;
        }
        let Some(a) = act else { return };
        let (ta, tb) = item.span();
        match a {
            "edit" => self.ui.marker_edit = Some((r.id, edit_pos)),
            "goto" => self.center_on(item.time(r.side)),
            "goto_pair" => self.center_on(ta + CaptureTime::from_secs_f64((tb - ta).as_secs_f64() / 2.0)),
            "zoom_pair" => self.zoom_on(ta, tb),
            "t0" => {
                let s = self.session.as_mut().unwrap();
                s.origin = if is_t0 { TimeOrigin::CaptureStart } else { TimeOrigin::Marker(r) };
            }
            "ungroup" => {
                self.session.as_mut().unwrap().markers.ungroup(r.id);
                self.fix_origin();
            }
            "pair" => {
                let s = self.session.as_mut().unwrap();
                if let [x, y] = s.markers.selected_singles()[..] {
                    s.markers.merge_into_pair(x, y);
                }
                self.fix_origin();
            }
            "delete" => {
                self.session.as_mut().unwrap().markers.remove(r.id);
                self.fix_origin();
            }
            "delete_all" => {
                let n = self.session.as_ref().map_or(0, |s| s.markers.lines().count());
                self.ui.confirm = Some(super::dialogs::Confirm::delete_all_markers(n));
            }
            _ => {}
        }
    }

    /// A T0 whose marker is gone falls back to the mode default.
    pub(crate) fn fix_origin(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        if let TimeOrigin::Marker(r) = s.origin
            && s.markers.time(r).is_none()
        {
            s.origin = s.origin.for_capture_start(false);
        }
    }

    /// Note, color and delete, under the head.
    fn marker_edit_popup(&mut self, ui: &Ui) {
        let Some((id, pos)) = self.ui.marker_edit else { return };
        let Some(item) = self.session.as_ref().and_then(|s| s.markers.get(id).cloned()) else {
            self.ui.marker_edit = None;
            return;
        };
        let theme = ui.la_theme();
        let mut note = item.note.clone().unwrap_or_default();
        let mut color = item.color;
        let mut delete = false;
        let mut close = false;
        let area = egui::Area::new(ui.make_persistent_id(("marker_edit", id)))
            .order(egui::Order::Foreground)
            .default_pos(pos)
            .movable(true)
            .show(ui.ctx(), |ui| {
                popover_frame(ui).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(EDIT_W - 24.0);
                    ui.label(
                        egui::RichText::new(item.name()).text_style(Text::Lead.style()).color(theme.color(Token::Ink1)),
                    );
                    ui.add_space(6.0);
                    // Enter closes; Shift+Enter is a newline.
                    let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut note)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .hint_text(tr("Comment here…")),
                    );
                    super::label_text_edit(&r, &note, tr("Marker note"));
                    if !ui.memory(|m| m.has_focus(r.id)) && ui.ctx().memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    if enter {
                        close = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (k, rgb) in MARKER_PALETTE.iter().enumerate() {
                            let h = if k == color { 20.0 } else { 16.0 };
                            let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::click());
                            resp.widget_info(|| {
                                WidgetInfo::selected(
                                    Role::Button,
                                    true,
                                    k == color,
                                    tr("Color {}").replacen("{}", &(k + 1).to_string(), 1),
                                )
                            });
                            let g = Rect::from_min_size(pos2(rect.left(), rect.bottom() - h), vec2(16.0, h));
                            icons::pennant(ui.painter(), g, c32(*rgb));
                            if resp.clicked() {
                                color = k;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Delete marker")));
                            let c = if resp.hovered() { Token::Alert } else { Token::Ink3 };
                            icons::trash(ui.painter(), rect, theme.color(c));
                            if resp.on_hover_text(tr("Delete")).clicked() {
                                delete = true;
                            }
                        });
                    });
                });
            });
        let outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) || outside {
            close = true;
        }
        let s = self.session.as_mut().unwrap();
        if delete {
            s.markers.remove(id);
            self.ui.marker_edit = None;
            self.fix_origin();
            return;
        }
        if let Some(i) = s.markers.get_mut(id) {
            i.note = (!note.trim().is_empty()).then_some(note);
            i.color = color;
        }
        if close {
            self.ui.marker_edit = None;
        }
    }

    /// right-click on the time ruler.
    fn time_zero_menu(&mut self, ui: &Ui, l: &Layout) {
        let resp = ui.interact(l.ruler, ui.make_persistent_id("ruler"), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, tr("Time ruler")));
        let Some(s) = self.session.as_ref() else { return };
        let origin = s.origin;
        let lines: Vec<(MarkerRef, String)> = s.markers.lines().map(|(r, i)| (r, i.side_name(r.side))).collect();
        let triggered = s.snapshot.as_ref().is_some_and(|sn| sn.mode == crate::core::capture::CaptureMode::Triggered);
        // Delete Data at the right-click point on the ruler.
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.ui.ruler_menu_x = p.x - l.plot_rows.left();
        }
        let v = s.view.state();
        let t = v.px_to_time(self.ui.ruler_menu_x as f64);
        let t0 = s.origin.resolve(&s.markers, s.trigger);
        let point = tr("this point ({})").replacen("{}", &crate::core::format::time_full(t - t0), 1);
        let trim_items = super::trim::point_items(t, &point, Some((v.left_time, v.right_time())));
        let mut trim = None;
        let mut pick = None;
        let mut jump = false;
        let mut time_settings = false;
        Popup::context_menu(&resp).frame(popover_frame(ui)).width(240.0).show(|ui| {
            menu_title(ui, tr("Set Time Relative To"));
            let start = menu_check_item(ui, tr("Start of Capture"), origin == TimeOrigin::CaptureStart, None);
            if start.on_hover_text(tr("t₀ will be set to the beginning of the capture.")).clicked() {
                pick = Some(TimeOrigin::CaptureStart);
            }
            ui.add_enabled_ui(triggered, |ui| {
                let r = menu_check_item(ui, tr("Trigger"), origin == TimeOrigin::Trigger, None);
                if r.on_hover_text(tr("t₀ at the trigger; the capture start until a trigger is found.")).clicked() {
                    pick = Some(TimeOrigin::Trigger);
                }
            });
            ui.add_enabled_ui(!lines.is_empty(), |ui| {
                let on_marker = matches!(origin, TimeOrigin::Marker(_));
                super::rowctl::submenu_checked(ui, tr("Timing Marker"), on_marker, 160.0, |ui| {
                    for (r, name) in &lines {
                        if menu_check_item(ui, name, origin == TimeOrigin::Marker(*r), None).clicked() {
                            pick = Some(TimeOrigin::Marker(*r));
                        }
                    }
                });
            });
            let wall = menu_check_item(ui, tr("Wall Clock Time"), origin == TimeOrigin::WallClock, None);
            if wall.on_hover_text(tr("Labels show the date and time (ISO and UTC options in Settings).")).clicked() {
                pick = Some(TimeOrigin::WallClock);
            }
            menu_separator(ui);
            if let Some(k) = self.delete_data_submenu(ui, &trim_items) {
                trim = Some(k);
            }
            if self.can_trim() {
                menu_separator(ui);
            }
            if menu_item(ui, tr("Jump to Time"), Some("G")).clicked() {
                jump = true;
            }
            menu_separator(ui);
            if menu_item(ui, tr("Time Settings…"), None).clicked() {
                time_settings = true;
            }
        });
        if time_settings {
            self.open_settings(super::settings::Section::TimeLabels);
        }
        if let Some(o) = pick
            && let Some(s) = self.session.as_mut()
        {
            s.origin = o;
        }
        if jump {
            self.open_jump();
        }
        if let Some(k) = trim {
            self.request_trim(&trim_items[k]);
        }
    }
}

/// Digital trigger color, hsl(55 100 % 73 %).
const TRIGGER_COLOR: Color32 = Color32::from_rgb(0xFF, 0xF1, 0x75);

impl LaApp {
    /// The trigger line, its "T" flag, the off-screen chip and
    /// the flag's menu.
    pub(crate) fn trigger_ui(&mut self, ui: &mut Ui, l: &Layout, view: &ViewState) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(t) = s.trigger else { return };
        let is_t0 = s.origin == TimeOrigin::Trigger;
        let x = l.plot_rows.left() + view.time_to_px(t) as f32;
        let painter = ui.painter_at(l.ruler);
        if x < l.plot_rows.left() || x > l.plot_rows.right() {
            // An arrow chip at the ruler edge.
            let left = x < l.plot_rows.left();
            let text = "T";
            let font = FontId::new(10.0, family(SANS_MEDIUM));
            let w = 24.0;
            let cx = if left { l.ruler.left() } else { l.ruler.right() - w };
            let chip = Rect::from_min_size(pos2(cx, l.ruler.bottom() - 18.0), vec2(w, 14.0));
            let resp = ui.interact(chip, ui.make_persistent_id("trigger_chip"), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Go to trigger")));
            painter.rect_filled(chip, 3.0, TRIGGER_COLOR);
            edge_arrow(&painter, chip, left, Color32::BLACK);
            let at = chip.center() + vec2(if left { 4.0 } else { -4.0 }, 0.0);
            painter.text(at, Align2::CENTER_CENTER, text, font, Color32::BLACK);
            if resp.on_hover_text(tr("Trigger")).clicked() {
                self.center_on(t);
            }
            return;
        }
        ui.painter_at(l.plot_rows)
            .line_segment([pos2(x, l.rows.top()), pos2(x, l.rows.bottom())], Stroke::new(1.0, TRIGGER_COLOR));
        painter.line_segment([pos2(x, l.ruler.top()), pos2(x, l.ruler.bottom())], Stroke::new(1.0, TRIGGER_COLOR));
        // A 12 pt flag marked "T", at the ruler's bottom so marker
        // heads keep the top.
        let flag = Rect::from_min_size(pos2(x, l.ruler.bottom() - 18.0), vec2(12.0, 14.0));
        let resp = ui.interact(flag, ui.make_persistent_id("trigger_flag"), Sense::click());
        // Not "Trigger": that is the Inspector's tab.
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tr("Trigger point")));
        painter.rect_filled(flag, CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 }, TRIGGER_COLOR);
        painter.text(flag.center(), Align2::CENTER_CENTER, "T", FontId::new(10.0, family(SANS_MEDIUM)), Color32::BLACK);
        let resp = resp.on_hover_text(tr("Trigger"));
        let mut set = false;
        // Delete Data before / after the trigger.
        let trim_items: Vec<super::trim::TrimItem> = vec![
            (tr("Before Trigger"), Some(t), None, tr("the trigger").to_owned()),
            (tr("After Trigger"), None, Some(t), tr("the trigger").to_owned()),
        ];
        let mut trim = None;
        Popup::context_menu(&resp).frame(popover_frame(ui)).width(200.0).show(|ui| {
            ui.add_enabled_ui(!is_t0, |ui| {
                if menu_check_item(ui, tr("Set as t₀"), is_t0, None).clicked() {
                    set = true;
                }
            });
            if let Some(k) = self.delete_data_submenu(ui, &trim_items) {
                trim = Some(k);
            }
        });
        if let Some(k) = trim {
            self.request_trim(&trim_items[k]);
        }
        if set && let Some(s) = self.session.as_mut() {
            s.origin = TimeOrigin::Trigger;
        }
    }

    /// `J`: centers the trigger.
    pub(crate) fn center_trigger(&mut self) {
        if let Some(t) = self.session.as_ref().and_then(|s| s.trigger) {
            self.center_on(t);
        }
    }
}

/// A small arrow at the chip's outer end.
fn edge_arrow(p: &egui::Painter, chip: Rect, left: bool, c: Color32) {
    let (x, d) = if left { (chip.left() + 4.0, 1.0) } else { (chip.right() - 4.0, -1.0) };
    let y = chip.center().y;
    let pts = vec![pos2(x, y), pos2(x + 4.0 * d, y - 3.5), pos2(x + 4.0 * d, y + 3.5)];
    p.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
}
