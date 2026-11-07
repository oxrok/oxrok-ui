//! The welcome wizard: theme, device access, three things to
//! know. The guided tour is deferred, so *Finish* only closes.

use crate::app::settings::ThemePref;
use crate::app::{APP_NAME, LaApp};
use crate::ui::ThemeExt;
use crate::ui::icons;
use crate::ui::theme::{ThemeKind, Token};
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, shortcut_chip};
use egui::{
    Align, Color32, Context, CornerRadius, Frame, Id, Layout, Margin, Modal, Rect, Role, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, pos2, vec2,
};

const STEPS: u8 = 3;
const CARD: egui::Vec2 = vec2(130.0, 90.0);

impl LaApp {
    /// The wizard opens by itself until it has been closed once.
    pub fn open_onboarding_on_first_run(&mut self) {
        if !self.settings.onboarding_done {
            self.ui.onboarding = Some(0);
        }
    }

    /// *Help → Start Onboarding*.
    pub(crate) fn start_onboarding(&mut self) {
        self.ui.onboarding = Some(0);
    }

    /// Skip, ✕, Escape and Finish: never again uninvited.
    fn close_onboarding(&mut self) {
        self.ui.onboarding = None;
        if !self.settings.onboarding_done {
            self.settings.onboarding_done = true;
            self.mark_settings_dirty();
        }
    }

    pub(crate) fn onboarding_ui(&mut self, ctx: &Context) {
        let Some(step) = self.ui.onboarding else { return };
        let theme = ctx.la_theme();
        let mut next_step = Some(step);
        let mut close = false;
        let resp = Modal::new(Id::unique("onboarding"))
            .frame(super::dialogs::modal_frame(ctx).inner_margin(Margin::ZERO))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                Frame::new().inner_margin(Margin { left: 32, right: 24, top: 24, bottom: 16 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("Welcome to {APP_NAME}"))
                                .text_style(Text::Display.style())
                                .color(theme.color(Token::Ink1)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if super::ranges::icon_button(ui, "Close", icons::close).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    // The step indicator: three dots and "Step N of 3".
                    ui.horizontal(|ui| {
                        for i in 0..STEPS {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            let c = if i == step { Token::Mark } else { Token::Seam };
                            ui.painter().circle_filled(r.center(), 3.0, theme.color(c));
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("Step {} of {STEPS}", step + 1))
                                .color(theme.color(Token::Ink4)),
                        );
                    });
                });
                ui.separator();
                Frame::new().inner_margin(Margin::symmetric(32, 20)).show(ui, |ui| {
                    ui.set_min_height(240.0);
                    match step {
                        0 => self.onboarding_theme(ui),
                        1 => self.onboarding_devices(ui),
                        _ => onboarding_tips(ui),
                    }
                });
                ui.separator();
                Frame::new().inner_margin(Margin::symmetric(24, 16)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(Button::tertiary("Skip")).clicked() {
                            close = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let last = step + 1 == STEPS;
                            if ui.add(Button::primary(if last { "Finish" } else { "Next" })).clicked() {
                                if last {
                                    close = true;
                                } else {
                                    next_step = Some(step + 1);
                                }
                            }
                            if step > 0 && ui.add(Button::secondary("Previous")).clicked() {
                                next_step = Some(step - 1);
                            }
                        });
                    });
                });
            });
        if close || resp.should_close() {
            self.close_onboarding();
        } else {
            self.ui.onboarding = next_step;
        }
    }

    /// Step 1: three theme cards; a click applies the theme live.
    fn onboarding_theme(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        section(ui, "Choose a theme");
        ui.add_space(20.0);
        let current = self.settings.appearance.theme;
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            for pref in ThemePref::ALL {
                ui.vertical(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(CARD, Sense::click());
                    let on = pref == current;
                    resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, on, pref.label()));
                    paint_thumbnail(ui, rect, pref);
                    let (w, c) = if on {
                        (2.0, theme.color(Token::Mark))
                    } else if resp.hovered() {
                        (1.0, theme.color(Token::RingHover))
                    } else {
                        (1.0, theme.color(Token::Seam))
                    };
                    ui.painter().rect_stroke(rect, 4.0, Stroke::new(w, c), StrokeKind::Inside);
                    ui.add_space(6.0);
                    ui.allocate_ui(vec2(CARD.x, 20.0), |ui| {
                        ui.centered_and_justified(|ui| {
                            let text = if on { format!("{} ✓", pref.label()) } else { pref.label().to_owned() };
                            ui.label(egui::RichText::new(text).color(theme.color(Token::Ink2)));
                        });
                    });
                    if resp.clicked() {
                        picked = Some(pref);
                    }
                });
            }
        });
        if let Some(pref) = picked.filter(|p| *p != current) {
            self.settings.appearance.theme = pref;
            self.mark_settings_dirty();
        }
    }

    /// Step 2: what the platform needs, and whether a device is there.
    fn onboarding_devices(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        section(ui, "Device access");
        ui.add_space(12.0);
        let body = |ui: &mut Ui, text: &str| {
            ui.label(egui::RichText::new(text).color(theme.color(Token::Ink2)));
        };
        if cfg!(target_os = "linux") {
            let (ok, text) = if super::help::udev_rules_installed() {
                (true, "udev rules: installed ✓")
            } else {
                (false, "udev rules: not found !")
            };
            let color = if ok { Token::Good } else { Token::Caution };
            ui.label(egui::RichText::new(text).color(theme.color(color)));
            ui.add_space(4.0);
            body(ui, "Replug the device after installing.");
        } else if cfg!(target_os = "windows") {
            body(ui, "Most devices need the WinUSB driver.");
            ui.hyperlink_to("Zadig instructions ↗", "https://sigrok.org/wiki/Windows");
        } else {
            body(ui, "No driver is needed. Firmware for some devices is loaded on first use.");
        }
        ui.add_space(8.0);
        if ui.link("Device Connection Help…").clicked() {
            self.open_connection_help();
        }
        ui.add_space(16.0);
        let found = self.devices.as_ref().map(|d| d.iter().filter(|d| !d.is_demo).count());
        let status = match found {
            None => "Looking for devices…".to_owned(),
            Some(0) => "No device found yet. You can continue and connect later.".to_owned(),
            Some(1) => "1 device found".to_owned(),
            Some(n) => format!("{n} devices found"),
        };
        let color = if found.is_some_and(|n| n > 0) { Token::Good } else { Token::Ink3 };
        ui.label(egui::RichText::new(status).color(theme.color(color)));
    }
}

/// Step 3: `Space`, `?`, and dropping a file.
fn onboarding_tips(ui: &mut Ui) {
    let theme = ui.la_theme();
    section(ui, "Three things to know");
    ui.add_space(16.0);
    let text = |ui: &mut Ui, t: &str| {
        ui.label(egui::RichText::new(t).color(theme.color(Token::Ink2)));
    };
    let lead = |ui: &mut Ui, f: &dyn Fn(&mut Ui)| {
        ui.allocate_ui_with_layout(vec2(64.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(64.0);
            f(ui);
        });
    };
    ui.spacing_mut().item_spacing.y = 14.0;
    ui.horizontal(|ui| {
        lead(ui, &|ui| shortcut_chip(ui, "Space"));
        text(ui, "starts and stops recording");
    });
    ui.horizontal(|ui| {
        lead(ui, &|ui| shortcut_chip(ui, "?"));
        text(ui, "shows every keyboard shortcut");
    });
    ui.horizontal(|ui| {
        lead(ui, &|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::file(ui.painter(), r, ui.la_theme().color(Token::Ink3));
        });
        text(ui, "drop a capture file anywhere to open it");
    });
}

fn section(ui: &mut Ui, title: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(title).text_style(Text::Subhead.style()).color(theme.color(Token::Ink1)));
}

/// A sine wave on each theme's background; *System* is split diagonally.
fn paint_thumbnail(ui: &Ui, rect: Rect, pref: ThemePref) {
    let (light, dark) = (ThemeKind::Light.theme(), ThemeKind::Dark.theme());
    let p = ui.painter().with_clip_rect(rect);
    let bg = |t: &crate::ui::theme::Theme| t.color(Token::Deck);
    let wave_color = |t: &crate::ui::theme::Theme| t.color(Token::Mark);
    let wave = |color: Color32| {
        let pts: Vec<_> = (0..=60)
            .map(|i| {
                let x = i as f32 / 60.0;
                let y = 0.5 - 0.25 * (x * std::f32::consts::TAU * 2.0).sin();
                pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height())
            })
            .collect();
        egui::Shape::line(pts, Stroke::new(2.0, color))
    };
    match pref {
        ThemePref::Light => {
            p.rect_filled(rect, CornerRadius::same(4), bg(&light));
            p.add(wave(wave_color(&light)));
        }
        ThemePref::Dark => {
            p.rect_filled(rect, CornerRadius::same(4), bg(&dark));
            p.add(wave(wave_color(&dark)));
        }
        ThemePref::System => {
            p.rect_filled(rect, CornerRadius::same(4), bg(&light));
            let lower = vec![rect.right_top(), rect.right_bottom(), rect.left_bottom()];
            p.add(egui::Shape::convex_polygon(lower, bg(&dark), Stroke::NONE));
            p.add(wave(wave_color(&light)));
        }
    }
}
