//! Start screen.

use crate::app::LaApp;
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::icons;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::Button;
use egui::{Layout, Rect, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

const MAX_WIDTH: f32 = 1180.0;
const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
/// The oxrok wordmark in the header (316×100), and the header's height.
const LOGO_W: f32 = 140.0;
const LOGO_H: f32 = LOGO_W * 100.0 / 316.0;
const HEAD_H: f32 = LOGO_H + 6.0 + 18.0 + 2.0 + 18.0;
/// The Load a Capture File card without recent files, and a row.
const LOAD_H: f32 = 120.0;
const RECENT_ROW: f32 = 32.0;
const RECENT_SHOWN: usize = 5;
/// The Recent list's gap above it and its header row.
const RECENT_HEAD: f32 = 16.0 + 28.0;
/// The Devices card without its list: margins, title, illustration.
const CARD_HEAD: f32 = 24.0 + 36.0 + 112.0 + 8.0 + 24.0;
/// The illustration's height.
const ART_H: f32 = 80.0;
/// One device row, and the list's width.
const DEVICE_ROW: f32 = 40.0;
const LIST_W: f32 = 560.0;
/// Supported device families.
const FAMILIES: &str = "fx2lafw (Cypress FX2), DreamSourceLab, Kingst, …";

/// A bundled PNG as a texture.
fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let image = eframe::icon_data::from_png_bytes(bytes).ok().map_or_else(
        || egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]),
        |i| egui::ColorImage::from_rgba_unmultiplied([i.width as usize, i.height as usize], &i.rgba),
    );
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

impl LaApp {
    pub(crate) fn start_screen(&mut self, ui: &mut Ui) {
        let width_px = (ui.max_rect().width() - self.settings.label_width) as f64;
        let mut open = None;
        let mut help = false;
        let mut choose = false;
        let mut pick = None;
        // The page scrolls when the window is short, so the logo and version
        // stay, and the five recent files stay reachable.
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let theme = ui.la_theme();
            let view = ui.available_rect_before_wrap();
            let recent: Vec<std::path::PathBuf> =
                self.settings.files.recent_files.iter().filter(|p| p.exists()).take(RECENT_SHOWN).cloned().collect();
            let load_h = LOAD_H + if recent.is_empty() { 0.0 } else { RECENT_HEAD + RECENT_ROW * recent.len() as f32 };
            let devices = self.devices.clone().unwrap_or_default();
            let rows = if devices.is_empty() { 2 } else { devices.len().clamp(1, 8) };
            let card_h = CARD_HEAD + DEVICE_ROW * rows as f32;
            let content_h = view.height().max(HEAD_H + GAP + card_h + GAP + load_h + 2.0 * MARGIN);
            let (rect, _) = ui.allocate_exact_size(vec2(view.width(), content_h), egui::Sense::hover());
            let area = rect.shrink(MARGIN);
            let width = area.width().min(MAX_WIDTH);
            let left = area.center().x - width / 2.0;
            let total = HEAD_H + GAP + card_h + GAP + load_h;
            let top = area.top() + ((area.height() - total) / 2.0).max(0.0);
            let version = self.version_line();
            let logo = if ui.visuals().dark_mode {
                self.ui
                    .logo_dark
                    .get_or_insert_with(|| {
                        texture(ui.ctx(), "oxrok_logo_dark", include_bytes!("../../../assets/oxrok-lockup-dark.png"))
                    })
                    .clone()
            } else {
                self.ui
                    .logo_light
                    .get_or_insert_with(|| {
                        texture(ui.ctx(), "oxrok_logo_light", include_bytes!("../../../assets/oxrok-lockup-light.png"))
                    })
                    .clone()
            };
            let head = Rect::from_min_size(pos2(left, top), vec2(width, HEAD_H));
            ui.scope_builder(UiBuilder::new().max_rect(head).layout(Layout::top_down(egui::Align::Center)), |ui| {
                ui.add(egui::Image::new(&logo).fit_to_exact_size(vec2(LOGO_W, LOGO_H)));
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(version).text_style(Text::Caption.style()).color(theme.color(Token::Ink3)),
                );
                ui.add_space(2.0);
                if let Some(url) = super::help::project_url() {
                    let labels = [tr("Release Notes"), tr("User Guide"), tr("Report Issue")];
                    let width = |ui: &Ui, t: &str| {
                        ui.painter()
                            .layout_no_wrap(t.to_owned(), Text::Caption.font_id(), egui::Color32::WHITE)
                            .rect
                            .width()
                    };
                    let dot = width(ui, "·");
                    let total = labels.iter().map(|t| width(ui, t)).sum::<f32>()
                        + 2.0 * dot
                        + ui.spacing().item_spacing.x * 4.0;
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), egui::Sense::hover());
                    let links = Rect::from_min_size(pos2(row.center().x - total / 2.0, row.top()), vec2(total, 18.0));
                    ui.scope_builder(
                        UiBuilder::new().max_rect(links).layout(Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            let link = |ui: &mut Ui, label: &str, href: String| {
                                ui.add(egui::Hyperlink::from_label_and_url(
                                    egui::RichText::new(label).text_style(Text::Caption.style()),
                                    href,
                                ));
                            };
                            link(ui, tr("Release Notes"), format!("{url}/releases"));
                            dotted(ui, &theme);
                            link(ui, tr("User Guide"), format!("{url}/wiki"));
                            dotted(ui, &theme);
                            link(ui, tr("Report Issue"), format!("{url}/issues"));
                        },
                    );
                }
            });
            let card = Rect::from_min_size(pos2(left, top + HEAD_H + GAP), vec2(width, card_h));
            ui.painter().rect_stroke(card, 3.0, Stroke::new(1.0, theme.color(Token::Seam)), StrokeKind::Inside);

            // Devices: everything oxrok found, demo included, alike.
            ui.scope_builder(
                UiBuilder::new().max_rect(card.shrink(24.0)).layout(Layout::top_down(egui::Align::Center)),
                |ui| {
                    title(ui, tr("Devices"));
                    illustration(ui, ART_H / 0.4, icons::device);
                    if devices.is_empty() {
                        let text = if self.devices.is_none() {
                            tr("Scanning for devices…").to_owned()
                        } else {
                            tr("No device found. Supported devices include {}").replacen("{}", FAMILIES, 1)
                        };
                        ui.label(egui::RichText::new(text).color(theme.color(Token::Ink3)));
                        if self.devices.is_some() && ui.link(tr("Device Connection Help…")).clicked() {
                            help = true;
                        }
                        return;
                    }
                    ui.add_space(8.0);
                    let w = ui.available_width().min(LIST_W);
                    let left = ui.max_rect().center().x - w / 2.0;
                    let list =
                        Rect::from_min_max(pos2(left, ui.cursor().top()), pos2(left + w, ui.max_rect().bottom()));
                    ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for d in &devices {
                                if device_row(ui, &self.settings.device_name(d), &device_detail(d)).clicked() {
                                    open = Some(d.clone());
                                }
                            }
                        });
                    });
                },
            );

            // Load a capture file.
            let load = Rect::from_min_size(pos2(left, card.bottom() + GAP), vec2(width, load_h));
            let content = ui.scope_builder(
                UiBuilder::new().max_rect(load.shrink2(vec2(24.0, 16.0))).layout(Layout::top_down(egui::Align::Center)),
                |ui| {
                    title(ui, tr("Load a Capture File"));
                    ui.add_space(12.0);
                    let row = vec2(ui.available_width(), 40.0);
                    ui.allocate_ui_with_layout(row, Layout::left_to_right(egui::Align::Center), |ui| {
                        let text_w = 260.0;
                        let button_w = 270.0;
                        ui.add_space(((ui.available_width() - text_w - button_w) / 2.0).max(0.0));
                        ui.label(
                            egui::RichText::new(tr("↓ Drag and drop a file here or")).color(theme.color(Token::Ink3)),
                        );
                        if ui.add(Button::primary(tr("Choose a file from your computer"))).clicked() {
                            choose = true;
                        }
                    });
                    if !recent.is_empty() {
                        ui.add_space(16.0);
                        ui.with_layout(Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let (head, _) =
                                ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::hover());
                            ui.painter().text(
                                pos2(head.left(), head.center().y),
                                egui::Align2::LEFT_CENTER,
                                tr("Recent"),
                                Text::CaptionBold.font_id(),
                                theme.color(Token::Ink3),
                            );
                            for p in &recent {
                                if recent_row(ui, p).clicked() {
                                    pick = Some(p.clone());
                                }
                            }
                        });
                    }
                },
            );
            // The dashed border around what was drawn, so it always holds it.
            let bottom = (content.response.rect.bottom() + 16.0).max(load.bottom());
            let load = Rect::from_min_max(load.min, pos2(load.right(), bottom));
            let dashed = Stroke::new(1.0, theme.color(Token::Seam));
            for side in [
                [load.left_top(), load.right_top()],
                [load.right_top(), load.right_bottom()],
                [load.right_bottom(), load.left_bottom()],
                [load.left_bottom(), load.left_top()],
            ] {
                ui.painter().add(egui::Shape::dashed_line(&side, dashed, 6.0, 4.0));
            }
        });

        if let Some(d) = open {
            self.open_session(&d, width_px);
        }
        if help {
            self.open_connection_help();
        }
        if choose {
            self.open_capture();
        }
        if let Some(p) = pick {
            self.open_paths(vec![p]);
        }
    }

    /// The start screen's version line, as the About dialog has it.
    fn version_line(&self) -> String {
        let decoders = self.decode.decoders().and_then(Result::ok).map_or(0, |d| d.len());
        match self.backend.library_version() {
            Some(lib) => {
                let decoders = tr("{} decoders").replacen("{}", &decoders.to_string(), 1);
                format!("{} {} · {} · {decoders}", crate::app::APP_NAME, env!("CARGO_PKG_VERSION"), lib)
            }
            None => {
                let decoders = tr("{} decoders").replacen("{}", &decoders.to_string(), 1);
                format!("{} {} · {decoders}", crate::app::APP_NAME, env!("CARGO_PKG_VERSION"))
            }
        }
    }
}

/// The dot between the start screen's links.
fn dotted(ui: &mut Ui, theme: &crate::ui::theme::Theme) {
    ui.label(egui::RichText::new("·").text_style(Text::Caption.style()).color(theme.color(Token::Ink4)));
}

/// The driver and connection under a device's name.
fn device_detail(d: &crate::backend::DeviceInfo) -> String {
    let conn = d.conn.as_deref().unwrap_or(&d.serial_or_conn);
    if conn.is_empty() { d.driver.clone() } else { format!("{} · {conn}", d.driver) }
}

/// One device row: its name, then driver and connection.
fn device_row(ui: &mut Ui, name: &str, detail: &str) -> egui::Response {
    let theme = ui.la_theme();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), DEVICE_ROW), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3.0, theme.color(Token::Lift));
    }
    let y = rect.center().y;
    p.text(
        pos2(rect.left() + 12.0, y),
        egui::Align2::LEFT_CENTER,
        name,
        Text::Prose.font_id(),
        theme.color(Token::Ink1),
    );
    p.text(
        pos2(rect.right() - 12.0, y),
        egui::Align2::RIGHT_CENTER,
        detail,
        Text::Prose.font_id(),
        theme.color(Token::Ink3),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One Recent row: icon, file name, shortened folder, date.
fn recent_row(ui: &mut Ui, path: &std::path::Path) -> egui::Response {
    let theme = ui.la_theme();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), RECENT_ROW), egui::Sense::click());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3.0, theme.color(Token::Lift));
    }
    let folder = path.parent().map(shorten_home).unwrap_or_default();
    let date =
        std::fs::metadata(path).and_then(|m| m.modified()).map(crate::core::format::short_date).unwrap_or_default();
    let y = rect.center().y;
    let icon = Rect::from_center_size(pos2(rect.left() + 14.0, y), vec2(12.0, 14.0));
    p.rect_stroke(icon, 1.0, Stroke::new(1.0, theme.color(Token::Ink3)), StrokeKind::Inside);
    let font = Text::Prose.font_id();
    p.text(pos2(rect.left() + 30.0, y), egui::Align2::LEFT_CENTER, &name, font.clone(), theme.color(Token::Ink1));
    let folder_x = rect.left() + (rect.width() * 0.4).max(200.0);
    p.text(pos2(folder_x, y), egui::Align2::LEFT_CENTER, folder, font.clone(), theme.color(Token::Ink3));
    p.text(pos2(rect.right() - 8.0, y), egui::Align2::RIGHT_CENTER, date, font, theme.color(Token::Ink3));
    resp
}

/// `~/…` for folders under the home directory.
fn shorten_home(dir: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|h| dir.strip_prefix(&h).ok().map(|r| r.to_path_buf())) {
        Some(rel) if rel.as_os_str().is_empty() => "~".into(),
        Some(rel) => format!("~/{}", rel.display()),
        None => dir.display().to_string(),
    }
}

fn title(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).text_style(Text::Display.style()).color(theme.color(Token::Ink1)));
}

fn illustration(ui: &mut Ui, card_h: f32, draw: fn(&egui::Painter, Rect, egui::Color32)) {
    let theme = ui.la_theme();
    let h = (card_h * 0.4).clamp(60.0, 200.0);
    let (rect, _) = ui.allocate_exact_size(vec2(h * 1.4, h), egui::Sense::hover());
    let square = Rect::from_center_size(rect.center(), vec2(h * 0.8, h * 0.8));
    draw(ui.painter(), square, theme.color(Token::Ink1));
}
