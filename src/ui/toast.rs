//! Toast notifications.

use crate::ui::icons;
use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{Align2, Area, Context, CornerRadius, Frame, Id, Margin, Order, RichText, Sense, Stroke, vec2};
use std::time::Duration;

/// Default lifetime.
pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(4);
/// Maximum visible toasts.
pub const MAX_VISIBLE: usize = 5;
const WIDTH: f32 = 320.0;
/// Enter animation.
const SLIDE_S: f32 = 0.2;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Showing a toast with a visible id updates it in place.
    pub id: String,
    pub kind: ToastKind,
    pub heading: Option<String>,
    pub body: String,
    /// `None` = persistent.
    pub lifetime: Option<Duration>,
    /// Up to two text actions. Stored in English and translated when
    /// drawn, so handlers keep matching one stable label.
    pub actions: Vec<&'static str>,
    /// A progress bar, 0–1.
    pub progress: Option<f32>,
}

impl Toast {
    pub fn new(kind: ToastKind, body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            id: body.clone(),
            kind,
            heading: None,
            body,
            lifetime: Some(DEFAULT_LIFETIME),
            actions: Vec::new(),
            progress: None,
        }
    }

    /// A custom duration.
    pub fn lifetime(mut self, d: Duration) -> Self {
        self.lifetime = Some(d);
        self
    }

    /// No auto-dismiss.
    pub fn persistent(mut self) -> Self {
        self.lifetime = None;
        self
    }

    /// Adds an action (at most two are shown).
    pub fn action(mut self, label: &'static str) -> Self {
        self.actions.push(label);
        self
    }

    /// Shows a progress bar; progress toasts don't auto-dismiss.
    pub fn progress(mut self, fraction: f32) -> Self {
        self.progress = Some(fraction.clamp(0.0, 1.0));
        self.lifetime = None;
        self
    }

    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.heading = Some(heading.into());
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[derive(Default, Debug)]
pub struct Toasts {
    items: Vec<(Toast, Duration)>,
    /// The "+N more" pill was clicked.
    expanded: bool,
    last_frame: Option<Duration>,
}

impl Toasts {
    /// Shows `toast` at time `now` (or updates the one with the same id).
    pub fn show(&mut self, toast: Toast, now: Duration) {
        let deadline = toast.lifetime.map_or(Duration::MAX, |l| now + l);
        match self.items.iter_mut().find(|(t, _)| t.id == toast.id) {
            Some(slot) => *slot = (toast, deadline),
            None => self.items.push((toast, deadline)),
        }
    }

    pub fn dismiss(&mut self, id: &str) {
        self.items.retain(|(t, _)| t.id != id);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter().map(|(t, _)| t)
    }

    /// Draws the stack at the bottom-left, `bottom_offset` above the window
    /// bottom. Returns the actions clicked, as `(toast id, label)`.
    pub fn ui(&mut self, ctx: &Context, now: Duration, bottom_offset: f32) -> Vec<(String, String)> {
        let dt = self.last_frame.map_or(Duration::ZERO, |l| now.saturating_sub(l));
        self.last_frame = Some(now);
        self.items.retain(|(_, deadline)| *deadline > now);
        if self.items.len() <= MAX_VISIBLE {
            self.expanded = false;
        }
        let mut clicked = Vec::new();
        if let Some(next) = self.items.iter().map(|(_, d)| *d).filter(|d| *d != Duration::MAX).min() {
            ctx.request_repaint_after(next - now);
        }
        let theme = ctx.la_theme();
        let mut dismissed = None;
        let hidden = if self.expanded { 0 } else { self.items.len().saturating_sub(MAX_VISIBLE) };
        let start = hidden;
        let mut expand = false;
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        Area::new(Id::unique("crate::ui::toasts"))
            .order(Order::Foreground)
            .anchor(Align2::LEFT_BOTTOM, vec2(16.0, -bottom_offset))
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                // Older toasts collapse into a pill above the stack.
                if hidden > 0 {
                    let text = format!("+{hidden} more");
                    let galley =
                        ui.painter().layout_no_wrap(text.clone(), Text::CaptionMed.font_id(), theme.color(Token::Ink2));
                    let (r, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 8.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &text));
                    ui.painter().rect(
                        r,
                        CornerRadius::same(u8::MAX),
                        theme.color(Token::Field),
                        Stroke::new(1.0, theme.color(Token::Seam)),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().galley(r.center() - galley.size() / 2.0, galley, egui::Color32::PLACEHOLDER);
                    if resp.clicked() {
                        expand = true;
                    }
                    ui.add_space(8.0);
                }
                for (toast, deadline) in &mut self.items[start..] {
                    // Slide in from the left.
                    let shown = ctx.animate_bool_with_time(Id::unique(("toast", &toast.id)), true, SLIDE_S);
                    let mut frame_ui = ui.new_child(egui::UiBuilder::new().max_rect(
                        ui.available_rect_before_wrap().translate(vec2(-(1.0 - shown) * (WIDTH + 24.0), 0.0)),
                    ));
                    let height = {
                        let ui = &mut frame_ui;
                        let frame = Frame::new()
                            .fill(theme.color(Token::Field))
                            .stroke(Stroke::new(1.0, theme.color(Token::Seam)))
                            .corner_radius(CornerRadius::same(4))
                            .shadow(theme.shallow_shadow())
                            .inner_margin(Margin::same(16));
                        let resp = frame
                            .show(ui, |ui| {
                                ui.set_width(WIDTH - 32.0);
                                ui.horizontal_top(|ui| {
                                    let (icon, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                    let p = ui.painter();
                                    match toast.kind {
                                        ToastKind::Info => icons::info(p, icon, theme.color(Token::Mark)),
                                        ToastKind::Success => icons::success(p, icon, theme.color(Token::Good)),
                                        ToastKind::Warning => icons::warning(p, icon, theme.color(Token::Caution)),
                                        ToastKind::Error => icons::error(p, icon, theme.color(Token::Alert)),
                                    }
                                    ui.vertical(|ui| {
                                        ui.set_width(WIDTH - 32.0 - 20.0 - 16.0 - 16.0);
                                        if let Some(h) = &toast.heading {
                                            ui.label(
                                                RichText::new(h)
                                                    .text_style(Text::Lead.style())
                                                    .color(theme.color(Token::Ink1)),
                                            );
                                        }
                                        ui.label(
                                            RichText::new(&toast.body)
                                                .text_style(Text::Prose.style())
                                                .color(theme.color(Token::Ink2)),
                                        );
                                        if let Some(p) = toast.progress {
                                            ui.add_space(6.0);
                                            let (r, _) =
                                                ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                                            ui.painter().rect_filled(r, 2.0, theme.color(Token::Hair));
                                            let f = egui::Rect::from_min_size(r.min, vec2(r.width() * p, 4.0));
                                            ui.painter().rect_filled(f, 2.0, theme.color(Token::Mark));
                                        }
                                        // Text actions, right-aligned.
                                        if !toast.actions.is_empty() {
                                            // A fixed-height row: a bare right-to-left layout
                                            // would take all the height left in the window.
                                            let row = vec2(ui.available_width(), 22.0);
                                            let layout = egui::Layout::right_to_left(egui::Align::Center);
                                            ui.allocate_ui_with_layout(row, layout, |ui| {
                                                for a in toast.actions.iter().take(2).rev() {
                                                    let r = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(crate::l10n::tr(a))
                                                                .color(theme.color(Token::FlareLink)),
                                                        )
                                                        .frame(false),
                                                    );
                                                    if r.clicked() {
                                                        clicked.push((toast.id.clone(), (*a).to_owned()));
                                                    }
                                                }
                                            });
                                        }
                                    });
                                    let (close, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                                    resp.widget_info(|| {
                                        egui::WidgetInfo::labeled(egui::Role::Button, true, crate::l10n::tr("Dismiss"))
                                    });
                                    let c = if resp.hovered() { Token::Ink1 } else { Token::Ink3 };
                                    icons::close(ui.painter(), close, theme.color(c));
                                    if resp.on_hover_text(crate::l10n::tr("Dismiss")).clicked() {
                                        dismissed = Some(toast.id.clone());
                                    }
                                });
                            })
                            .response;
                        // Hovering pauses the timer; Escape dismisses it.
                        let hovered = ctx.pointer_hover_pos().is_some_and(|p| resp.rect.contains(p));
                        if hovered && *deadline != Duration::MAX {
                            *deadline += dt;
                        }
                        if hovered && escape {
                            dismissed = Some(toast.id.clone());
                        }
                        ui.min_rect().height()
                    };
                    ui.add_space(height + 8.0);
                }
            });
        if expand {
            self.expanded = true;
        }
        if let Some(id) = dismissed {
            self.dismiss(&id);
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_updates_in_place_and_expires() {
        let mut t = Toasts::default();
        t.show(Toast::new(ToastKind::Info, "a").id("x"), Duration::ZERO);
        t.show(Toast::new(ToastKind::Error, "b").id("x"), Duration::from_secs(1));
        assert_eq!(t.iter().count(), 1);
        assert_eq!(t.iter().next().unwrap().body, "b");
        t.items.retain(|(_, d)| *d > Duration::from_secs(6));
        assert_eq!(t.iter().count(), 0);
    }

    #[test]
    fn progress_is_persistent() {
        let p = Toast::new(ToastKind::Info, "x").progress(0.5).action("Cancel");
        assert_eq!((p.lifetime, p.actions.len()), (None, 1));
    }
}
