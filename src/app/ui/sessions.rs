//! The session switcher: the session button in the bottom bar, the
//! Sessions popover and the session menu.

use crate::app::LaApp;
use crate::app::session::Session;
use crate::app::sessions::{CREATE_BLOCKED, SWITCH_BLOCKED};
use crate::app::worker::SessionId;
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_item, menu_separator, menu_title, popover_frame};
use egui::{Align2, Color32, Id, Key, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

/// Sizes.
const POPOVER_W: f32 = 360.0;
const ROW_H: f32 = 48.0;
const NAME_MAX_W: f32 = 220.0;
/// Typing filters the rows above this many sessions.
const FILTER_ABOVE: usize = 8;
/// A click activates at once, but the popover stays this long so a second
/// click can make it a double-click (rename).
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(300);

/// What the user did in the switcher this frame.
enum Act {
    Activate(SessionId),
    Rename(SessionId),
    CommitRename(SessionId, String),
    Close(SessionId),
    CloseOthers(SessionId),
    Save(SessionId, bool),
    Move(SessionId, Option<SessionId>),
    New,
}

/// The unsaved dot, or the origin glyph, at `c`.
fn origin_mark(p: &egui::Painter, s: &Session, c: egui::Pos2, color: Color32) {
    if s.is_unsaved() {
        p.circle_filled(c, 3.0, color);
    } else if s.opened_from.is_some() {
        icons::file(p, Rect::from_center_size(c, vec2(14.0, 14.0)), color);
    }
}

fn origin_tip(s: &Session) -> Option<&'static str> {
    if s.is_unsaved() { Some(tr("Not saved")) } else { None }
}

fn has_mark(s: &Session) -> bool {
    s.is_unsaved() || s.opened_from.is_some()
}

/// Elides `text` to `max` points in `font`.
pub(crate) fn elide(ui: &Ui, text: &str, font: egui::FontId, max: f32) -> String {
    let width = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), font.clone(), Color32::PLACEHOLDER).size().x;
    if width(text) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 0 && width(&format!("{}…", chars[..n].iter().collect::<String>())) > max {
        n -= 1;
    }
    format!("{}…", chars[..n].iter().collect::<String>())
}

impl LaApp {
    /// The session button, right of the Devices menu.
    pub(crate) fn session_button(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_ref() else {
            self.ui.sessions_open = false;
            return;
        };
        let theme = ui.la_theme();
        let n = self.session_count();
        let font = Text::Prose.font_id();
        let name = elide(ui, &s.name, font.clone(), NAME_MAX_W);
        let galley = ui.painter().layout_no_wrap(name.clone(), font, Color32::PLACEHOLDER);
        let mark_w = if has_mark(s) { 20.0 } else { 0.0 };
        let pill = (n > 1).then(|| n.to_string());
        let pill_w = pill.as_ref().map_or(0.0, |t| 8.0 * t.len() as f32 + 12.0);
        let w = 16.0 + mark_w + galley.size().x + 8.0 + 10.0 + pill_w + 16.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, ui.available_height()), Sense::click());
        let label = if n > 1 { format!("{} ({n} sessions)", s.name) } else { s.name.clone() };
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
        let open = self.ui.sessions_open;
        let p = ui.painter();
        if open {
            p.rect_filled(rect, 0.0, theme.color(Token::Mark));
        }
        p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
        let color = if open {
            theme.color(Token::InkOnFlare)
        } else if resp.hovered() {
            theme.color(Token::Ink1)
        } else {
            theme.color(Token::Ink3)
        };
        let mut x = rect.left() + 16.0;
        if mark_w > 0.0 {
            let mc = if s.is_unsaved() && !open { theme.color(Token::Ink2) } else { color };
            origin_mark(p, s, pos2(x + 7.0, rect.center().y), mc);
            x += mark_w;
        }
        p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley.clone(), color);
        x += galley.size().x + 8.0;
        let c = pos2(x + 4.0, rect.center().y);
        let dy = if open { -2.0 } else { 2.0 };
        p.line(vec![c + vec2(-4.0, -dy), c + vec2(0.0, dy), c + vec2(4.0, -dy)], Stroke::new(1.5, color));
        x += 10.0 + 4.0;
        if let Some(t) = pill {
            let r = Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(pill_w - 4.0, 16.0));
            p.rect_filled(r, 8.0, theme.color(Token::Ink4).gamma_multiply(0.35));
            p.text(r.center(), Align2::CENTER_CENTER, t, Text::CaptionMed.font_id(), color);
        }
        let tip = origin_tip(s).map(str::to_owned);
        let full_name = (name != s.name).then(|| s.name.clone());
        let active = s.id;
        let resp = match (tip, full_name) {
            (Some(t), _) => resp.on_hover_text(t),
            (None, Some(n)) => resp.on_hover_text(n),
            _ => resp,
        };
        let mut act = None;
        // right-clicking the button opens the active session's menu.
        resp.context_menu(|ui| self.session_menu(ui, active, &mut act));
        if resp.clicked() {
            self.toggle_sessions_popover();
        }
        self.sessions_popover(ui, rect, &mut act);
        if let Some(a) = act {
            self.apply_session_act(a);
        }
    }

    /// Inline rename in the popover (also *Rename Session*).
    pub(crate) fn start_session_rename(&mut self, id: SessionId) {
        self.ui.sessions_close_at = None;
        let name = self.session_by_id(id).map(|s| s.name.clone()).unwrap_or_default();
        self.ui.sessions_open = true;
        self.ui.session_rename = Some((id, name));
    }

    /// `Ctrl/Cmd+Shift+T` and the button.
    pub(crate) fn toggle_sessions_popover(&mut self) {
        self.ui.sessions_open = !self.ui.sessions_open;
        self.ui.sessions_filter.clear();
        self.ui.sessions_highlight = None;
        self.ui.session_rename = None;
        self.ui.sessions_flash_until = None;
        self.ui.sessions_close_at = None;
    }

    /// The popover, anchored above the button.
    fn sessions_popover(&mut self, ui: &mut Ui, button: Rect, act: &mut Option<Act>) {
        let now = self.now();
        let flashing = self.ui.sessions_flash_until.is_some_and(|t| now < t);
        if !flashing {
            self.ui.sessions_flash_until = None;
        } else if let Some(t) = self.ui.sessions_flash_until {
            ui.ctx().request_repaint_after(t.saturating_sub(now));
        }
        if let Some(t) = self.ui.sessions_close_at {
            if now >= t {
                self.ui.sessions_close_at = None;
                self.ui.sessions_open = false;
            } else {
                ui.ctx().request_repaint_after(t - now);
            }
        }
        if !self.ui.sessions_open && !flashing {
            return;
        }
        let ctx = ui.ctx().clone();
        let theme = ui.la_theme();
        let switch_blocked = self.switch_blocked();
        let create_blocked = self.create_blocked();
        let active = self.active_id();
        let filter = self.ui.sessions_filter.to_lowercase();
        let list: Vec<(SessionId, String, String, bool)> = self
            .sessions()
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
            .map(|s| (s.id, s.name.clone(), self.session_state_text(s), has_mark(s)))
            .collect();
        // Keyboard: ↑/↓, Enter, Escape; typing filters long lists.
        if self.ui.sessions_open && self.ui.session_rename.is_none() {
            let many = self.session_count() > FILTER_ABOVE;
            let (up, down, enter, esc, typed, back) = ctx.input_mut(|i| {
                let typed: String = if many {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Text(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect()
                } else {
                    String::new()
                };
                (
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, Key::Enter),
                    i.consume_key(egui::Modifiers::NONE, Key::Escape),
                    typed,
                    many && i.consume_key(egui::Modifiers::NONE, Key::Backspace),
                )
            });
            let cur = self.ui.sessions_highlight.or_else(|| list.iter().position(|r| Some(r.0) == active));
            let last = list.len().saturating_sub(1);
            if up {
                self.ui.sessions_highlight = Some(cur.map_or(last, |c| c.saturating_sub(1)));
            }
            if down {
                self.ui.sessions_highlight = Some(cur.map_or(0, |c| (c + 1).min(last)));
            }
            if enter && let Some(r) = cur.and_then(|c| list.get(c)) {
                *act = Some(Act::Activate(r.0));
            }
            if !typed.is_empty() {
                self.ui.sessions_filter.push_str(&typed);
                self.ui.sessions_highlight = Some(0);
            }
            if back {
                self.ui.sessions_filter.pop();
            }
            if esc {
                self.ui.sessions_open = false;
                return;
            }
        }
        let highlight = self.ui.sessions_highlight;
        let area = egui::Area::new(Id::unique("sessions_popover"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(button.left_top() - vec2(0.0, 4.0))
            .show(&ctx, |ui| {
                popover_frame(ui).show(ui, |ui| {
                    ui.set_width(POPOVER_W);
                    menu_title(ui, tr("Sessions"));
                    if !self.ui.sessions_filter.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(tr("Filter: {}").replacen("{}", &self.ui.sessions_filter, 1))
                                    .text_style(Text::CaptionMed.style())
                                    .color(theme.color(Token::Ink4)),
                            );
                        });
                    }
                    let mut rects: Vec<(SessionId, Rect)> = Vec::new();
                    for (k, (id, name, state, mark)) in list.iter().enumerate() {
                        let is_active = Some(*id) == active;
                        let blocked = !is_active && switch_blocked.is_some();
                        let (rect, resp) = ui.allocate_exact_size(vec2(POPOVER_W, ROW_H), Sense::click_and_drag());
                        rects.push((*id, rect));
                        resp.widget_info(|| WidgetInfo::selected(Role::Button, !blocked, is_active, name));
                        let p = ui.painter().clone();
                        if is_active {
                            p.rect_filled(rect, 0.0, theme.color(Token::Mark).gamma_multiply(0.25));
                            p.rect_filled(
                                Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                                0.0,
                                theme.color(Token::RingHot),
                            );
                        } else if (resp.hovered() && !blocked) || highlight == Some(k) {
                            p.rect_filled(rect, 0.0, theme.color(Token::Lift));
                        }
                        let fade = if blocked { 0.6 } else { 1.0 };
                        let ink = |t: Token| theme.color(t).gamma_multiply(fade);
                        let x = rect.left() + 16.0;
                        if *mark && let Some(s) = self.session_by_id(*id) {
                            origin_mark(&p, s, pos2(x + 7.0, rect.top() + 16.0), ink(Token::Ink2));
                        }
                        let tx = x + 22.0;
                        let renaming = self.ui.session_rename.as_ref().filter(|r| r.0 == *id).map(|r| r.1.clone());
                        if let Some(mut text) = renaming {
                            let r = Rect::from_min_size(pos2(tx, rect.top() + 4.0), vec2(POPOVER_W - 80.0, 22.0));
                            let te = ui.put(
                                r,
                                egui::TextEdit::singleline(&mut text)
                                    .font(Text::Prose.font_id())
                                    .id(Id::unique(("sess-rename", id))),
                            );
                            super::label_text_edit(&te, &text, tr("Session name"));
                            if !te.has_focus() && !te.lost_focus() {
                                te.request_focus();
                            }
                            let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                            if esc {
                                self.ui.session_rename = None;
                            } else if te.lost_focus() || enter {
                                *act = Some(Act::CommitRename(*id, text));
                            } else {
                                self.ui.session_rename = Some((*id, text));
                            }
                        } else {
                            let shown = elide(ui, name, Text::Prose.font_id(), POPOVER_W - 80.0);
                            p.text(
                                pos2(tx, rect.top() + 16.0),
                                Align2::LEFT_CENTER,
                                shown,
                                Text::Prose.font_id(),
                                ink(Token::Ink1),
                            );
                        }
                        // Ellipsized like the name, clear of the ✕; the full
                        // text on hover.
                        let shown = elide(ui, state, Text::CaptionMed.font_id(), POPOVER_W - 80.0);
                        let sr = p.text(
                            pos2(tx, rect.top() + 34.0),
                            Align2::LEFT_CENTER,
                            &shown,
                            Text::CaptionMed.font_id(),
                            ink(Token::Ink4),
                        );
                        let sresp = ui.interact(sr, Id::unique(("sess-state", id)), Sense::hover());
                        sresp.widget_info(|| WidgetInfo::labeled(Role::Label, true, state));
                        if shown != *state {
                            sresp.on_hover_text(state);
                        }
                        // ✕ on hover.
                        if resp.hovered() || resp.contains_pointer() {
                            let c = pos2(rect.right() - 20.0, rect.center().y);
                            let close = ui.interact(
                                Rect::from_center_size(c, vec2(16.0, 16.0)),
                                Id::unique(("sess-close", id)),
                                Sense::click(),
                            );
                            close.widget_info(|| {
                                WidgetInfo::labeled(Role::Button, true, tr("Close {}").replacen("{}", name, 1))
                            });
                            let bg = if close.hovered() { Token::Seam } else { Token::Lift };
                            ui.painter().circle_filled(c, 8.0, theme.color(bg));
                            icons::close(
                                ui.painter(),
                                Rect::from_center_size(c, vec2(8.0, 8.0)),
                                theme.color(Token::Ink3),
                            );
                            if close.clicked() {
                                *act = Some(Act::Close(*id));
                                continue;
                            }
                        }
                        let resp = if blocked { resp.on_hover_text(tr(SWITCH_BLOCKED)) } else { resp };
                        if resp.double_clicked() {
                            *act = Some(Act::Rename(*id));
                        } else if resp.clicked() && !blocked {
                            *act = Some(Act::Activate(*id));
                        }
                        if resp.drag_started() {
                            self.ui.session_drag = Some(*id);
                        }
                        let id = *id;
                        resp.context_menu(|ui| self.session_menu(ui, id, act));
                    }
                    // Drag to reorder, with the insertion line.
                    if let Some(dragged) = self.ui.session_drag {
                        let pointer = ui.input(|i| i.pointer.interact_pos());
                        let before = pointer.map(|p| rects.iter().find(|(_, r)| p.y < r.center().y).map(|(id, _)| *id));
                        if let Some(before) = before {
                            let y = match before.and_then(|b| rects.iter().find(|(id, _)| *id == b)) {
                                Some((_, r)) => r.top(),
                                None => rects.last().map_or(0.0, |(_, r)| r.bottom()),
                            };
                            ui.painter().hline(
                                rects.first().map_or(0.0, |(_, r)| r.left())
                                    ..=rects.first().map_or(0.0, |(_, r)| r.right()),
                                y,
                                Stroke::new(2.0, theme.color(Token::RingHot)),
                            );
                            if ui.input(|i| i.pointer.any_released()) {
                                self.ui.session_drag = None;
                                if before != Some(dragged) {
                                    *act = Some(Act::Move(dragged, before));
                                }
                            }
                        }
                        if !ui.input(|i| i.pointer.any_down()) {
                            self.ui.session_drag = None;
                        }
                    }
                    menu_separator(ui);
                    let (rect, resp) = ui.allocate_exact_size(vec2(POPOVER_W, 32.0), Sense::click());
                    let enabled = create_blocked.is_none();
                    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, tr("New Session")));
                    let c = theme.color(Token::FlareLink).gamma_multiply(if enabled { 1.0 } else { 0.6 });
                    if resp.hovered() && enabled {
                        ui.painter().rect_filled(rect, 0.0, theme.color(Token::Lift));
                    }
                    icons::plus(
                        ui.painter(),
                        Rect::from_center_size(pos2(rect.left() + 23.0, rect.center().y), vec2(12.0, 12.0)),
                        c,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 38.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        tr("New Session"),
                        Text::Prose.font_id(),
                        c,
                    );
                    let resp = if enabled { resp } else { resp.on_hover_text(tr(CREATE_BLOCKED)) };
                    if resp.clicked() && enabled {
                        *act = Some(Act::New);
                    }
                });
            });
        // Click outside closes (but not the button's own click, or a menu).
        let outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| !area.response.rect.contains(p) && !button.contains(p))
            && !egui::Popup::is_any_open(&ctx);
        if outside {
            self.ui.sessions_open = false;
            self.ui.session_rename = None;
            self.ui.sessions_flash_until = None;
        }
    }

    /// The session menu, for session `id`.
    fn session_menu(&mut self, ui: &mut Ui, id: SessionId, act: &mut Option<Act>) {
        let Some(s) = self.session_by_id(id) else { return };
        let can_save = s.state == crate::core::capture::CaptureState::Finished && s.has_data() && !self.ui.saving;
        let can_close = self.close_refused(id);
        let others = self.session_count() > 1;
        ui.set_min_width(220.0);
        if menu_item(ui, tr("Rename Session"), None).clicked() {
            *act = Some(Act::Rename(id));
            ui.close();
        }
        menu_separator(ui);
        let save_tip = tr("Record a capture before saving.");
        let r = ui.add_enabled_ui(can_save, |ui| menu_item(ui, tr("Save Capture"), Some(&shortcut("S")))).inner;
        if r.on_disabled_hover_text(save_tip).clicked() {
            *act = Some(Act::Save(id, false));
            ui.close();
        }
        let r =
            ui.add_enabled_ui(can_save, |ui| menu_item(ui, tr("Save Capture As…"), Some(&shortcut("Shift+S")))).inner;
        if r.on_disabled_hover_text(save_tip).clicked() {
            *act = Some(Act::Save(id, true));
            ui.close();
        }
        menu_separator(ui);
        let r = ui.add_enabled_ui(can_close.is_none(), |ui| menu_item(ui, tr("Close Session"), Some(&shortcut("W"))));
        if r.inner.on_disabled_hover_text(tr(can_close.unwrap_or_default())).clicked() {
            *act = Some(Act::Close(id));
            ui.close();
        }
        let r = ui.add_enabled_ui(others, |ui| menu_item(ui, tr("Close Other Sessions"), None));
        if r.inner.on_disabled_hover_text(tr("There are no other sessions.")).clicked() {
            *act = Some(Act::CloseOthers(id));
            ui.close();
        }
    }

    fn apply_session_act(&mut self, a: Act) {
        match a {
            Act::Activate(id) => {
                if self.activate(id) {
                    self.ui.sessions_close_at = Some(self.now() + DOUBLE_CLICK);
                }
            }
            Act::Rename(id) => self.start_session_rename(id),
            Act::CommitRename(id, name) => {
                self.rename_session(id, &name);
                self.ui.session_rename = None;
            }
            Act::Close(id) => self.request_close(id),
            Act::CloseOthers(id) => self.close_others(id),
            Act::Save(id, save_as) => self.save_session(id, save_as, None),
            Act::Move(id, before) => self.move_session(id, before),
            Act::New => {
                self.ui.sessions_open = false;
                self.new_session();
            }
        }
    }
}

/// `Ctrl+X` or `⌘X` for a menu's shortcut column.
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{}", key.replace("Shift+", "⇧")) } else { format!("Ctrl+{key}") }
}
