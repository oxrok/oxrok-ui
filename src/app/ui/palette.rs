//! The command palette: `Ctrl/Cmd+K`, fuzzy search over the
//! registry, recent commands and sub-commands.

use super::dialogs::modal_frame;
use crate::app::LaApp;
use crate::app::commands::{self, Command};
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::{SANS_BOLD, Text, family};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Id, Key, Margin, Modifiers, Rect, Role, Sense, Ui, WidgetInfo, pos2, vec2};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

/// Sizes.
const WIDTH: f32 = 560.0;
const TOP: f32 = 100.0;
const ROW_H: f32 = 44.0;
const HEADER_H: f32 = 28.0;
const VISIBLE_ROWS: f32 = 10.0;
/// At most this many results.
const MAX_RESULTS: usize = 30;

/// The open palette.
#[derive(Clone, Debug, Default)]
pub(crate) struct Palette {
    pub query: String,
    pub highlight: usize,
    /// The command whose sub-commands are listed.
    pub parent: Option<&'static str>,
    /// Focus the field on the first frame.
    opened: bool,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Header(&'static str),
    Command {
        id: &'static str,
        label: String,
        matched: Vec<usize>,
    },
    /// A sub-command of `parent`.
    Child {
        parent: &'static str,
        key: String,
        label: String,
        matched: Vec<usize>,
    },
    Nothing,
}

impl Entry {
    fn runnable(&self) -> bool {
        matches!(self, Entry::Command { .. } | Entry::Child { .. })
    }
}

/// Fuzzy match: the label first, then the description (its matches
/// rank lower and aren't highlighted).
fn score(m: &SkimMatcherV2, label: &str, description: &str, query: &str) -> Option<(i64, Vec<usize>)> {
    if let Some((s, idx)) = m.fuzzy_indices(label, query) {
        return Some((s * 2, idx));
    }
    m.fuzzy_match(&format!("{label} {description}"), query).map(|s| (s, Vec::new()))
}

impl LaApp {
    /// `Ctrl/Cmd+K`.
    pub(crate) fn open_palette(&mut self) {
        self.ui.palette = Some(Palette::default());
    }

    /// The commands the palette can list now.
    fn palette_commands(&self) -> Vec<&'static Command> {
        commands::registry()
            .iter()
            .filter(|c| !c.hidden_in_palette && self.command_available(c.id).is_ok())
            .filter(|c| !c.has_children || !self.command_children(c.id).is_empty())
            .collect()
    }

    /// The list for the palette's state.
    fn palette_entries(&self, p: &Palette) -> Vec<Entry> {
        let m = SkimMatcherV2::default();
        let q = p.query.trim();
        if let Some(parent) = p.parent {
            let mut v: Vec<(i64, Entry)> = self
                .command_children(parent)
                .into_iter()
                .filter_map(|(key, label)| {
                    let (s, matched) = if q.is_empty() { (0, Vec::new()) } else { m.fuzzy_indices(&label, q)? };
                    Some((s, Entry::Child { parent, key, label, matched }))
                })
                .collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.0));
            let v: Vec<Entry> = v.into_iter().map(|e| e.1).take(MAX_RESULTS).collect();
            return if v.is_empty() { vec![Entry::Nothing] } else { v };
        }
        let cmds = self.palette_commands();
        let entry = |c: &Command| Entry::Command { id: c.id, label: self.command_label(c), matched: Vec::new() };
        if q.is_empty() {
            let recent: Vec<&Command> = self
                .settings
                .history
                .recent_commands
                .iter()
                .filter_map(|id| cmds.iter().find(|c| c.id == id).copied())
                .collect();
            let mut v = Vec::new();
            if !recent.is_empty() {
                v.push(Entry::Header("Recent"));
                v.extend(recent.iter().map(|c| entry(c)));
            }
            v.push(Entry::Header("All commands"));
            v.extend(cmds.iter().filter(|c| !recent.iter().any(|r| r.id == c.id)).map(|c| entry(c)));
            v.truncate(MAX_RESULTS);
            return v;
        }
        let mut scored: Vec<(i64, Entry)> = Vec::new();
        for c in &cmds {
            let label = self.command_label(c);
            if let Some((s, matched)) = score(&m, &label, c.description, q) {
                scored.push((s, Entry::Command { id: c.id, label, matched }));
            }
            // A child's search text is "‹parent› ‹child›".
            if c.has_children {
                for (key, child) in self.command_children(c.id) {
                    if let Some(s) = m.fuzzy_match(&format!("{} {child}", c.label), q) {
                        let label = format!("{}: {child}", c.label);
                        scored.push((s, Entry::Child { parent: c.id, key, label, matched: Vec::new() }));
                    }
                }
            }
        }
        // Best first; ties keep the registry order.
        scored.sort_by_key(|e| std::cmp::Reverse(e.0));
        let v: Vec<Entry> = scored.into_iter().map(|e| e.1).take(MAX_RESULTS).collect();
        if v.is_empty() { vec![Entry::Nothing] } else { v }
    }

    pub(crate) fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.ui.palette.take() else { return };
        let entries = self.palette_entries(&p);
        let runnable: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.runnable()).map(|(i, _)| i).collect();
        p.highlight = p.highlight.min(runnable.len().saturating_sub(1));
        // Keys, before the text field sees them.
        let (up, down, enter, shift_enter, esc, back) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::SHIFT, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
                p.query.is_empty() && p.parent.is_some() && i.consume_key(Modifiers::NONE, Key::Backspace),
            )
        });
        if esc {
            return;
        }
        if back {
            p.parent = None;
            p.highlight = 0;
        }
        if up {
            p.highlight = p.highlight.saturating_sub(1);
        }
        if down {
            p.highlight = (p.highlight + 1).min(runnable.len().saturating_sub(1));
        }
        let mut chosen: Option<(Entry, bool)> = None;
        if (enter || shift_enter)
            && let Some(&i) = runnable.get(p.highlight)
        {
            chosen = Some((entries[i].clone(), shift_enter));
        }
        let theme = ctx.la_theme();
        let graph = self.ui.graph_rect.0;
        let center_x =
            if graph.is_finite() && graph.width() > 0.0 { graph.center().x } else { ctx.content_rect().center().x };
        let mut back_clicked = false;
        let area = egui::Area::new(Id::unique("command_palette"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(pos2(center_x, ctx.content_rect().top() + TOP))
            .show(ctx, |ui| {
                modal_frame(ctx).inner_margin(Margin::ZERO).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // The search field with its ⌕ glyph.
                    ui.horizontal(|ui| {
                        ui.set_height(44.0);
                        ui.add_space(16.0);
                        let (g, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        let c = theme.color(Token::Ink3);
                        ui.painter().circle_stroke(g.center() - vec2(2.0, 2.0), 5.0, egui::Stroke::new(1.5, c));
                        ui.painter().line_segment(
                            [g.center() + vec2(2.0, 2.0), g.center() + vec2(6.5, 6.5)],
                            egui::Stroke::new(1.5, c),
                        );
                        ui.add_space(8.0);
                        let before = p.query.clone();
                        let edit = egui::TextEdit::singleline(&mut p.query)
                            .hint_text("Search commands…")
                            .frame(egui::Frame::NONE)
                            .font(Text::Prose.font_id())
                            .desired_width(ui.available_width() - 16.0)
                            .id(Id::unique("palette_query"));
                        let r = ui.add(edit);
                        super::label_text_edit(&r, &p.query, "Search commands");
                        if !p.opened || !r.has_focus() {
                            r.request_focus();
                            p.opened = true;
                        }
                        if p.query != before {
                            p.highlight = 0;
                        }
                    });
                    ui.painter().hline(
                        ui.min_rect().x_range(),
                        ui.min_rect().bottom(),
                        egui::Stroke::new(1.0, theme.color(Token::Seam)),
                    );
                    // Breadcrumb.
                    if let Some(parent) = p.parent.and_then(commands::get) {
                        let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, HEADER_H), Sense::click());
                        let text = format!("‹ {}", parent.label);
                        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &text));
                        let c = if resp.hovered() { Token::Ink1 } else { Token::Ink3 };
                        ui.painter().text(
                            pos2(rect.left() + 16.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &text,
                            Text::CaptionMed.font_id(),
                            theme.color(c),
                        );
                        back_clicked = resp.clicked();
                    }
                    // Up to 10 rows high, scrolling beyond.
                    let content: f32 =
                        entries.iter().map(|e| if matches!(e, Entry::Header(_)) { HEADER_H } else { ROW_H }).sum();
                    let height = content.min(ROW_H * VISIBLE_ROWS);
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(height)
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (i, e) in entries.iter().enumerate() {
                                let k = runnable.iter().position(|&r| r == i);
                                if let Some(pick) = self.palette_row(ui, e, k == Some(p.highlight)) {
                                    chosen = Some((pick, false));
                                }
                            }
                        });
                    ui.add_space(4.0);
                });
            });
        if back_clicked {
            p.parent = None;
            p.highlight = 0;
            p.query.clear();
        }
        // A click outside closes.
        let outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos));
        if outside {
            return;
        }
        match chosen {
            Some((Entry::Command { id, .. }, keep)) => {
                let c = commands::get(id);
                if c.is_some_and(|c| c.has_children) {
                    // Show the children; nothing runs yet.
                    p.parent = Some(id);
                    p.query.clear();
                    p.highlight = 0;
                    self.ui.palette = Some(p);
                    return;
                }
                self.settings.add_recent_command(id);
                self.mark_settings_dirty();
                // Shift+Enter keeps it open for state toggles.
                let toggle = self.command_checked(id).is_some() && c.is_some_and(|c| c.keys.is_empty());
                if keep && toggle {
                    self.ui.palette = Some(p);
                }
                self.dispatch(id);
            }
            Some((Entry::Child { parent, key, .. }, _)) => {
                self.settings.add_recent_command(parent);
                self.mark_settings_dirty();
                self.dispatch_child(parent, &key);
            }
            _ => self.ui.palette = Some(p),
        }
    }

    /// One 44 pt row (or a 28 pt header); returns the entry when clicked.
    fn palette_row(&self, ui: &mut Ui, e: &Entry, highlighted: bool) -> Option<Entry> {
        let theme = ui.la_theme();
        match e {
            Entry::Header(h) => {
                let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, HEADER_H), Sense::hover());
                resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, *h));
                ui.painter().text(
                    pos2(rect.left() + 16.0, rect.center().y + 2.0),
                    Align2::LEFT_CENTER,
                    *h,
                    egui::FontId::new(11.0, family(crate::ui::typography::SANS_MEDIUM)),
                    theme.color(Token::Ink4),
                );
                None
            }
            Entry::Nothing => {
                let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, ROW_H), Sense::hover());
                resp.widget_info(|| WidgetInfo::labeled(Role::Label, false, "No matching commands"));
                ui.painter().text(
                    pos2(rect.left() + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "No matching commands",
                    Text::Prose.font_id(),
                    theme.color(Token::Ink4),
                );
                None
            }
            Entry::Command { id, label, matched } => {
                let c = commands::get(id)?;
                let keys: Vec<String> = c.primary_key(super::commands::MAC).into_iter().collect();
                self.row_body(ui, label, matched, c.description, &keys, c.has_children, highlighted).then(|| e.clone())
            }
            Entry::Child { label, matched, .. } => {
                self.row_body(ui, label, matched, "", &[], false, highlighted).then(|| e.clone())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row_body(
        &self,
        ui: &mut Ui,
        label: &str,
        matched: &[usize],
        description: &str,
        keys: &[String],
        more: bool,
        highlighted: bool,
    ) -> bool {
        let theme = ui.la_theme();
        let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, ROW_H), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::Button, true, highlighted, label));
        if highlighted {
            ui.scroll_to_rect(rect, None);
        }
        let painter = ui.painter_at(rect);
        if highlighted || resp.hovered() {
            painter.rect_filled(rect, 0.0, theme.color(Token::Lift));
        }
        // The label, with matched characters in bold.
        let mut job = LayoutJob::default();
        for (k, ch) in label.chars().enumerate() {
            let bold = matched.contains(&k);
            let font = if bold { egui::FontId::new(13.0, family(SANS_BOLD)) } else { Text::Prose.font_id() };
            job.append(
                &ch.to_string(),
                0.0,
                TextFormat { font_id: font, color: theme.color(Token::Ink1), ..Default::default() },
            );
        }
        let galley = ui.painter().layout_job(job);
        let x = rect.left() + 16.0;
        let label_w = galley.size().x;
        painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
        // The right side: shortcut chips or ›.
        let mut right = rect.right() - 16.0;
        if more {
            painter.text(
                pos2(right, rect.center().y),
                Align2::RIGHT_CENTER,
                "›",
                Text::Headline.font_id(),
                theme.color(Token::Ink3),
            );
            right -= 20.0;
        }
        for key in keys.iter().rev() {
            let g = painter.layout_no_wrap(key.clone(), Text::CaptionMed.font_id(), theme.color(Token::Ink2));
            let chip = Rect::from_min_size(
                pos2(right - g.size().x - 10.0, rect.center().y - 10.0),
                vec2(g.size().x + 10.0, 20.0),
            );
            painter.rect(
                chip,
                3.0,
                theme.color(Token::Field),
                egui::Stroke::new(1.0, theme.color(Token::Seam)),
                egui::StrokeKind::Inside,
            );
            painter.galley(chip.center() - g.size() / 2.0, g, Color32::PLACEHOLDER);
            right = chip.left() - 6.0;
        }
        // The description, ellipsized in what's left.
        if !description.is_empty() {
            let left = x + label_w + 16.0;
            let room = right - 12.0 - left;
            if room > 24.0 {
                let text = super::sessions::elide(ui, description, Text::Prose.font_id(), room);
                painter.text(
                    pos2(left, rect.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    Text::Prose.font_id(),
                    theme.color(Token::Ink4),
                );
            }
        }
        resp.clicked()
    }
}
