//! Device Connection Help and About.

use crate::app::{APP_NAME, AppEvent, LaApp};
use crate::backend::DeviceInfo;
use crate::l10n::tr;
use crate::ui::ThemeExt;
use crate::ui::icons;
use crate::ui::theme::Token;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Segment, segmented};
use egui::{Align, Context, Frame, Id, Layout, Margin, Modal, ScrollArea, Sense, Ui, pos2, vec2};

/// The recent "driver:conn" entries kept.
pub(crate) const RECENT_CONNS: usize = 5;

/// Dialog state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnectionHelp {
    /// The "driver:conn" field.
    pub conn: String,
    /// A connection scan is running.
    pub connecting: bool,
}

/// Install Firmware dialog state.
#[derive(Clone, Debug)]
pub(crate) struct InstallFirmware {
    pub family: crate::app::firmware::Family,
    /// The missing file that brought it up.
    pub file: String,
    pub input: Option<std::path::PathBuf>,
    pub log: Vec<String>,
    pub running: bool,
    pub result: Option<Result<Vec<std::path::PathBuf>, String>>,
    /// *Show manual steps*.
    pub manual: bool,
}

/// Dialog state.
#[derive(Clone, Debug, Default)]
pub(crate) struct About {
    /// 0 = License, 1 = Third-party notices.
    pub tab: usize,
}

/// A card's status glyph.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Check {
    Ok,
    Warn,
    Unknown,
}

impl Check {
    fn glyph(self) -> &'static str {
        match self {
            Check::Ok => "✓",
            Check::Warn => "!",
            Check::Unknown => "○",
        }
    }
}

/// One card: status, title, one line of text.
struct Card {
    check: Check,
    title: &'static str,
    text: String,
}

/// A `*sigrok*.rules` file in either udev rules directory.
pub(crate) fn udev_rules_installed() -> bool {
    ["/etc/udev/rules.d", "/usr/lib/udev/rules.d"].iter().any(|dir| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.contains("sigrok") && name.ends_with(".rules")
            })
        })
    })
}

/// "N devices found", counting physical devices only.
pub(crate) fn found_text(devices: Option<&[DeviceInfo]>) -> Option<String> {
    let n = devices?.iter().filter(|d| !d.is_demo).count();
    Some(match n {
        0 => tr("No device found").to_owned(),
        1 => tr("1 device found").to_owned(),
        n => tr("{} devices found").replacen("{}", &n.to_string(), 1),
    })
}

/// "driver:conn" or "driver:conn=…" → (driver, conn).
pub(crate) fn parse_conn(s: &str) -> Option<(String, String)> {
    let (driver, conn) = s.trim().split_once(':')?;
    let conn = conn.strip_prefix("conn=").unwrap_or(conn);
    (!driver.is_empty() && !conn.is_empty()).then(|| (driver.to_owned(), conn.to_owned()))
}

/// The build, as the About header shows it.
pub(crate) fn build_line() -> String {
    format!("built {} · {} · {} {}", env!("LA_BUILD_DATE"), env!("LA_BUILD_COMMIT"), os_name(), std::env::consts::ARCH)
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

/// The project URL the build sets, if any.
pub(crate) fn project_url() -> Option<&'static str> {
    Some(option_env!("LA_PROJECT_URL").filter(|u| !u.is_empty()).unwrap_or("https://github.com/oxrok/ui"))
}

const LICENSE: &str = include_str!("../../../LICENSE");
const NOTICES: &str = include_str!("../../../assets/third-party.txt");

impl LaApp {
    pub(crate) fn open_connection_help(&mut self) {
        self.ui.connection_help.get_or_insert_with(Default::default);
        self.rescan_devices();
    }

    /// Cards for this platform (Firmware comes with).
    fn connection_cards(&self) -> Vec<Card> {
        let mut cards = vec![Card {
            check: Check::Unknown,
            title: tr("Cable and port"),
            text: tr("Try another USB port or cable. Hubs without power often fail.").into(),
        }];
        if cfg!(target_os = "linux") {
            let ok = udev_rules_installed();
            cards.push(Card {
                check: if ok { Check::Ok } else { Check::Warn },
                title: tr("Permissions"),
                text: if ok {
                    tr("udev rules installed").into()
                } else {
                    tr("udev rules not found. Replug the device after installing.").into()
                },
            });
        }
        if cfg!(target_os = "windows") {
            cards.push(Card {
                check: Check::Unknown,
                title: tr("Driver"),
                text: tr("Most devices need the WinUSB driver. Install it with Zadig.").into(),
            });
        }
        cards
    }

    /// The Firmware card's status and text.
    fn firmware_card(&self) -> Card {
        match self.firmware.missing.first() {
            None => Card { check: Check::Ok, title: tr("Firmware"), text: tr("No firmware missing").into() },
            Some(m) => Card {
                check: Check::Warn,
                title: tr("Firmware"),
                text: tr("{} is missing (needed by {})").replacen("{}", &m.file, 1).replacen("{}", &m.driver, 1),
            },
        }
    }

    /// *Copy diagnostics*: versions, devices, every card.
    fn diagnostics(&self) -> String {
        let mut out = vec![format!("{APP_NAME} {} ({})", env!("CARGO_PKG_VERSION"), build_line())];
        if let Some(v) = self.backend.library_version() {
            out.push(v);
        }
        match &self.devices {
            Some(list) => {
                out.push(format!("Devices: {}", list.len()));
                for d in list {
                    out.push(format!("  {} — {} ({})", d.driver, d.display_name(), d.serial_or_conn));
                }
            }
            None => out.push("Devices: not scanned yet".into()),
        }
        for c in self.connection_cards().into_iter().chain([self.firmware_card()]) {
            out.push(format!("[{}] {}: {}", c.check.glyph(), c.title, c.text));
        }
        for (p, exists) in &self.firmware.paths {
            out.push(format!("  {} {}", if *exists { "✓" } else { "○" }, p.display()));
        }
        out.join("\n")
    }

    /// Device Connection Help.
    pub(crate) fn connection_help_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.ui.connection_help.take() else { return };
        let theme = ctx.la_theme();
        let status = found_text(self.devices.as_deref()).unwrap_or_else(|| tr("Looking for devices…").into());
        let scanning = self.scanning();
        let cards = self.connection_cards();
        let fw_card = self.firmware_card();
        let missing = self.firmware.missing.clone();
        let paths = self.firmware.paths.clone();
        let recent = self.settings.recent_connections.clone();
        let (mut close, mut rescan, mut copy, mut connect) = (false, false, false, false);
        let mut install = None;
        let resp = Modal::new(Id::unique("connection_help")).frame(super::dialogs::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(560.0);
            title_row(ui, tr("Device Connection Help"), &mut close);
            ui.add_space(16.0);
            // The status row, with Rescan.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&status).text_style(Text::Lead.style()));
                // One button, one size, so the dialog doesn't move.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if scanning { tr("Scanning…") } else { tr("Rescan") };
                    if ui.add_enabled(!scanning, Button::secondary(label).min_width(120.0)).clicked() {
                        rescan = true;
                    }
                });
            });
            ui.add_space(12.0);
            for c in &cards {
                card(ui, c, |_| {});
            }
            // Firmware: each missing file, then the search paths.
            card(ui, &fw_card, |ui| {
                for m in missing.iter().skip(1) {
                    ui.label(
                        egui::RichText::new(
                            tr("{} is missing (needed by {})").replacen("{}", &m.file, 1).replacen("{}", &m.driver, 1),
                        )
                        .color(theme.color(Token::Ink3)),
                    );
                }
                if let Some(m) = missing.first()
                    && ui.add(Button::secondary(tr("Install…"))).clicked()
                {
                    install = Some(m.file.clone());
                }
                for (p, exists) in &paths {
                    let mark = if *exists { "✓" } else { "○" };
                    ui.label(
                        egui::RichText::new(format!("{mark} {}", p.display()))
                            .size(11.0)
                            .color(theme.color(Token::Ink4)),
                    );
                }
            });
            // Serial and network instruments.
            card(
                ui,
                &Card {
                    check: Check::Unknown,
                    title: tr("Serial and network instruments"),
                    text: tr("Enter driver:conn, e.g. rigol-ds:conn=tcp-raw/192.168.1.20/5555").into(),
                },
                |ui| {
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut d.conn)
                                .hint_text("driver:conn")
                                .desired_width(ui.available_width() - 110.0),
                        );
                        super::label_text_edit(&edit, &d.conn, tr("Connection"));
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let ok = parse_conn(&d.conn).is_some() && !d.connecting;
                        if ui.add_enabled(ok, Button::primary(tr("Connect"))).clicked() || enter && ok {
                            connect = true;
                        }
                        if !recent.is_empty() {
                            let mut pick = None;
                            ui.menu_button("⌄", |ui| {
                                for r in &recent {
                                    if ui.button(r).clicked() {
                                        pick = Some(r.clone());
                                    }
                                }
                            })
                            .response
                            .on_hover_text(tr("Recent connections"));
                            if let Some(p) = pick {
                                d.conn = p;
                            }
                        }
                    });
                },
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.add(Button::secondary(tr("Copy diagnostics"))).clicked() {
                    copy = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::primary(tr("Close"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if rescan {
            self.rescan_devices();
        }
        if let Some(file) = install {
            self.open_install_firmware(&file);
        }
        if copy {
            ctx.copy_text(self.diagnostics());
            self.toast(Toast::new(ToastKind::Success, tr("Diagnostics copied")));
        }
        if connect && let Some((driver, conn)) = parse_conn(&d.conn) {
            d.connecting = true;
            let (backend, tx, ctx2, spec) = (self.backend.clone(), self.events_tx.clone(), ctx.clone(), d.conn.clone());
            std::thread::spawn(move || {
                let r = backend.scan_conn(&driver, &conn).map_err(|e| e.to_string());
                let _ = tx.send(AppEvent::Connected(spec, r));
                ctx2.request_repaint();
            });
        }
        if !close && !resp.should_close() {
            self.ui.connection_help = Some(d);
        }
    }

    /// A connection scan finished.
    pub(crate) fn on_connected(&mut self, spec: String, r: Result<Vec<DeviceInfo>, String>) {
        if let Some(d) = self.ui.connection_help.as_mut() {
            d.connecting = false;
        }
        match r {
            Ok(found) if !found.is_empty() => {
                let recent = &mut self.settings.recent_connections;
                recent.retain(|c| *c != spec);
                recent.insert(0, spec);
                recent.truncate(RECENT_CONNS);
                self.mark_settings_dirty();
                let n = found.len();
                let text = if n == 1 {
                    tr("Found 1 device").to_owned()
                } else {
                    tr("Found {} devices").replacen("{}", &n.to_string(), 1)
                };
                self.toast(Toast::new(ToastKind::Success, text));
                self.rescan_devices();
            }
            Ok(_) => self.toast(Toast::new(ToastKind::Warning, tr("No device found at {}").replacen("{}", &spec, 1))),
            Err(e) => self.toast(Toast::new(ToastKind::Error, e).heading(tr("Connect failed"))),
        }
    }

    /// "‹app› ‹version›", the build, and the library versions.
    fn version_info(&self) -> String {
        let decoders = self.decode.decoders().and_then(Result::ok).map_or(0, |d| d.len());
        let mut libs: Vec<String> = self.backend.library_version().into_iter().collect();
        libs.push(crate::app::decoder_library());
        libs.push(format!("{decoders} decoders"));
        format!("{APP_NAME} {}\n{}\n{}", env!("CARGO_PKG_VERSION"), build_line(), libs.join(" · "))
    }

    /// About.
    pub(crate) fn about_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.ui.about.take() else { return };
        let theme = ctx.la_theme();
        let info = self.version_info();
        let icon = self.app_icon(ctx);
        let mut close = false;
        let resp = Modal::new(Id::unique("about")).frame(super::dialogs::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            title_row(ui, &tr("About {}").replacen("{}", APP_NAME, 1), &mut close);
            ui.add_space(16.0);
            let mut lines = info.lines();
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&icon).fit_to_exact_size(vec2(48.0, 48.0)));
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(lines.next().unwrap_or_default())
                            .text_style(Text::Display.style())
                            .color(theme.color(Token::Ink1)),
                    );
                    ui.label(egui::RichText::new(lines.next().unwrap_or_default()).color(theme.color(Token::Ink4)));
                });
            });
            ui.add_space(10.0);
            ui.label(egui::RichText::new(lines.next().unwrap_or_default()).color(theme.color(Token::Ink2)));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if let Some(url) = project_url() {
                    ui.hyperlink_to(tr("Release Notes"), format!("{url}/releases"));
                    ui.label("·");
                    ui.hyperlink_to(tr("User Guide"), format!("{url}/wiki"));
                    ui.label("·");
                    ui.hyperlink_to(tr("Report Issue"), format!("{url}/issues"));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::tertiary(tr("Copy version info"))).clicked() {
                        ui.ctx().copy_text(info.clone());
                    }
                });
            });
            ui.add_space(12.0);
            let segs = [
                Segment { label: tr("License"), enabled: true, tooltip: "" },
                Segment { label: tr("Third-party notices"), enabled: true, tooltip: "" },
            ];
            segmented(ui, "about_tab", &segs, &mut d.tab);
            ui.add_space(8.0);
            Frame::new()
                .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
                .corner_radius(4)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ScrollArea::vertical().id_salt(("about_text", d.tab)).max_height(200.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let text = if d.tab == 0 { LICENSE } else { NOTICES };
                        ui.label(egui::RichText::new(text).monospace().size(11.0).color(theme.color(Token::Ink2)));
                    });
                });
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(Button::primary(tr("Close"))).clicked() {
                    close = true;
                }
            });
        });
        if !close && !resp.should_close() {
            self.ui.about = Some(d);
        }
    }

    /// The bundled icon as a texture, made once.
    fn app_icon(&mut self, ctx: &Context) -> egui::TextureHandle {
        self.ui
            .app_icon
            .get_or_insert_with(|| {
                let img = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png")).ok();
                let image = img.map_or_else(
                    || egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]),
                    |i| egui::ColorImage::from_rgba_unmultiplied([i.width as usize, i.height as usize], &i.rgba),
                );
                ctx.load_texture("app_icon", image, egui::TextureOptions::LINEAR)
            })
            .clone()
    }
}

// ---- Install Firmware ----

impl LaApp {
    pub(crate) fn open_install_firmware(&mut self, file: &str) {
        let family = crate::app::firmware::family_of(file);
        self.ui.install_firmware = Some(InstallFirmware {
            family,
            file: file.to_owned(),
            input: None,
            log: Vec::new(),
            running: false,
            result: None,
            manual: crate::app::firmware::script_path(&family).is_none() && !self.backend_extracts(),
        });
    }

    /// The backend can extract without a bundled script (the mock).
    fn backend_extracts(&self) -> bool {
        self.backend.extracts_without_script()
    }

    fn firmware_out_dir(&self) -> std::path::PathBuf {
        crate::app::firmware::user_firmware_dir(&self.firmware.paths)
    }

    pub(crate) fn on_firmware_input(&mut self, p: Option<std::path::PathBuf>) {
        if let (Some(d), Some(p)) = (self.ui.install_firmware.as_mut(), p) {
            d.input = Some(p);
        }
    }

    pub(crate) fn on_firmware_line(&mut self, line: String) {
        if let Some(d) = self.ui.install_firmware.as_mut() {
            d.log.push(line);
        }
    }

    pub(crate) fn on_firmware_done(&mut self, r: Result<Vec<std::path::PathBuf>, String>) {
        if let Some(d) = self.ui.install_firmware.as_mut() {
            d.running = false;
            d.result = Some(r);
        }
    }

    /// The equivalent terminal commands (*Show manual steps*).
    fn manual_steps(&self, d: &InstallFirmware) -> String {
        let out = self.firmware_out_dir();
        match d.family.script {
            Some(script) => format!(
                "git clone https://github.com/sigrokproject/sigrok-util\nmkdir -p \"{dir}\"\ncd \"{dir}\"\n{interp} /path/to/sigrok-util/firmware/{script} \"{input}\"",
                dir = out.display(),
                interp = if d.family.needs.contains(&"python3") { "python3" } else { "sh" },
                input = d.input.as_ref().map_or("/path/to/the/vendor/file".into(), |p| p.display().to_string()),
            ),
            None => format!(
                "See {} for where to get {} and put the files in\n{}",
                d.family.wiki,
                d.family.input.to_lowercase(),
                out.display()
            ),
        }
    }

    pub(crate) fn install_firmware_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.ui.install_firmware.take() else { return };
        let theme = ctx.la_theme();
        let out_dir = self.firmware_out_dir();
        let script = crate::app::firmware::script_path(&d.family);
        let can_extract = script.is_some() || self.backend_extracts();
        let missing_tools = crate::app::firmware::missing_tools(d.family.needs);
        let manual_text = self.manual_steps(&d);
        let (mut close, mut browse, mut extract, mut rescan) = (false, false, false, false);
        let resp = Modal::new(Id::unique("install_firmware")).frame(super::dialogs::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(560.0);
            title_row(ui, &tr("Install firmware for {}").replacen("{}", d.family.title, 1), &mut close);
            ui.label(
                egui::RichText::new(tr("{} is missing.").replacen("{}", &d.file, 1)).color(theme.color(Token::Ink4)),
            );
            ui.add_space(14.0);
            let step = |ui: &mut Ui, n: &str, title: &str| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(n).text_style(Text::Lead.style()).color(theme.color(Token::Mark)));
                    ui.label(egui::RichText::new(title).text_style(Text::Lead.style()));
                });
            };
            step(ui, "1", tr("Download the vendor software"));
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.label(egui::RichText::new(d.family.input).color(theme.color(Token::Ink3)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.hyperlink_to(tr("Device page ↗"), d.family.wiki);
                });
            });
            ui.add_space(10.0);
            if d.manual || !can_extract {
                ui.label(egui::RichText::new(tr("Manual steps")).text_style(Text::Lead.style()));
                if !can_extract {
                    ui.label(
                        egui::RichText::new(if d.family.script.is_some() {
                            tr("The extraction scripts aren't bundled with this build (make firmware-deps).")
                        } else {
                            tr("There is no extraction step for this device.")
                        })
                        .color(theme.color(Token::Ink4)),
                    );
                }
                Frame::new()
                    .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&manual_text).monospace().size(11.0));
                    });
                if ui.link(tr("Copy")).clicked() {
                    ui.ctx().copy_text(manual_text.clone());
                }
            } else {
                step(ui, "2", tr("Choose the downloaded file"));
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    let mut shown = d.input.as_ref().map_or(String::new(), |p| p.display().to_string());
                    ui.add_enabled(
                        false,
                        egui::TextEdit::singleline(&mut shown).hint_text(tr("Drop the file here")).desired_width(380.0),
                    );
                    if ui.add(Button::secondary(tr("Browse…"))).clicked() {
                        browse = true;
                    }
                });
                ui.add_space(10.0);
                step(ui, "3", tr("Extract"));
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    let ok = d.input.is_some() && !d.running && missing_tools.is_empty();
                    if ui.add_enabled(ok, Button::primary(tr("Extract firmware"))).clicked() {
                        extract = true;
                    }
                    if d.running {
                        ui.add(egui::Spinner::new());
                    }
                });
                for t in &missing_tools {
                    let hint = crate::app::firmware::install_hint(t);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.label(
                            egui::RichText::new(tr("! {} is needed: {}").replacen("{}", t, 1).replacen("{}", &hint, 1))
                                .color(theme.color(Token::Caution)),
                        );
                        if ui.link(tr("Copy command")).clicked() {
                            ui.ctx().copy_text(hint.clone());
                        }
                    });
                }
                if !d.log.is_empty() {
                    Frame::new()
                        .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            ScrollArea::vertical().max_height(120.0).stick_to_bottom(true).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(egui::RichText::new(d.log.join("\n")).monospace().size(11.0));
                            });
                        });
                }
                match &d.result {
                    Some(Ok(files)) => {
                        let n = files.len();
                        let dir = out_dir.display().to_string();
                        let text = if n == 1 {
                            tr("✓ Installed 1 file to {}").replacen("{}", &dir, 1)
                        } else {
                            tr("✓ Installed {} files to {}").replacen("{}", &n.to_string(), 1).replacen("{}", &dir, 1)
                        };
                        ui.label(egui::RichText::new(text).color(theme.color(Token::Good)));
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(tr("Replug the device if it isn't found."))
                                    .color(theme.color(Token::Ink3)),
                            );
                            if ui.add(Button::secondary(tr("Rescan"))).clicked() {
                                rescan = true;
                            }
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(egui::RichText::new(format!("! {e}")).color(theme.color(Token::Alert)));
                        if ui.link(tr("Copy log")).clicked() {
                            ui.ctx().copy_text(d.log.join("\n"));
                        }
                    }
                    None => {}
                }
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if can_extract {
                    let label = if d.manual { tr("Hide manual steps") } else { tr("Show manual steps") };
                    if ui.add(Button::secondary(label)).clicked() {
                        d.manual = !d.manual;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::primary(tr("Close"))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if browse {
            let (dialogs, tx, ctx2) = (self.dialogs.clone(), self.events_tx.clone(), ctx.clone());
            let folder = d.family.input.contains("folder");
            std::thread::spawn(move || {
                let p = if folder {
                    dialogs.pick_folder(None)
                } else {
                    dialogs.open_files(tr("Choose the downloaded file"), None, &[]).into_iter().next()
                };
                let _ = tx.send(AppEvent::FirmwareInput(p));
                ctx2.request_repaint();
            });
        }
        if extract && let Some(input) = d.input.clone() {
            d.running = true;
            d.log.clear();
            d.result = None;
            let script = script.clone().unwrap_or_default();
            let (backend, tx, ctx2, out) = (self.backend.clone(), self.events_tx.clone(), ctx.clone(), out_dir);
            std::thread::spawn(move || {
                let mut line = |l: String| {
                    let _ = tx.send(AppEvent::FirmwareLine(l));
                    ctx2.request_repaint();
                };
                let r = backend.extract_firmware(&script, &input, &out, &mut line).map_err(|e| e.to_string());
                let _ = tx.send(AppEvent::FirmwareDone(r));
                ctx2.request_repaint();
            });
        }
        if rescan {
            self.rescan_devices();
        }
        if !close && !resp.should_close() {
            self.ui.install_firmware = Some(d);
        }
    }
}

/// A dialog title with ✕ on the right.
pub(crate) fn title_row(ui: &mut Ui, title: &str, close: &mut bool) {
    let theme = ui.la_theme();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).text_style(Text::Display.style()).color(theme.color(Token::Ink1)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if super::ranges::icon_button(ui, tr("Close"), icons::close).clicked() {
                *close = true;
            }
        });
    });
}

/// A card with its status glyph, title, text, and extra content.
fn card(ui: &mut Ui, c: &Card, extra: impl FnOnce(&mut Ui)) {
    let theme = ui.la_theme();
    Frame::new()
        .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                let color = match c.check {
                    Check::Ok => Token::Good,
                    Check::Warn => Token::Caution,
                    Check::Unknown => Token::Ink4,
                };
                let center = r.center();
                ui.painter().circle_stroke(center, 8.0, egui::Stroke::new(1.5, theme.color(color)));
                // ○ is the bare circle; ✓ and ! sit inside it.
                if c.check != Check::Unknown {
                    ui.painter().text(
                        pos2(center.x, center.y),
                        egui::Align2::CENTER_CENTER,
                        c.check.glyph(),
                        egui::FontId::proportional(11.0),
                        theme.color(color),
                    );
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(c.title).color(theme.color(Token::Ink1)));
                    ui.label(egui::RichText::new(&c.text).color(theme.color(Token::Ink3)));
                    extra(ui);
                });
            });
        });
    ui.add_space(8.0);
}
