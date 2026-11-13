//! Device Connection Help and About.

use crate::app::firmware::{Job, JobKind, human_size, unpacker_hint};
use crate::app::{APP_NAME, AppEvent, LaApp};
use crate::backend::{DeviceInfo, MissingFirmware, fetch};
use crate::l10n::tr;
use crate::ui::ThemeExt;
use crate::ui::icons;
use crate::ui::theme::Token;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Segment, segmented};
use egui::{Align, Context, Frame, Id, Layout, Margin, Modal, ScrollArea, Sense, Ui, pos2, vec2};
use std::path::PathBuf;
use std::sync::Arc;

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
    /// The missing file that brought it up.
    pub file: String,
    /// Where the file comes from, when the firmware registry knows.
    pub package: Option<&'static fetch::Package>,
    /// The files and folders to install from.
    pub inputs: Vec<PathBuf>,
    /// One of `inputs` is an archive only an external unpacker opens.
    pub inputs_packed: bool,
    /// unar, 7-Zip or bsdtar is installed. Looked for when something
    /// needs one, and again while none is there.
    pub unpacker: bool,
    /// When it was last looked for, in egui's time.
    pub unpacker_checked: f64,
    /// The download or import, running or finished.
    pub job: Option<Arc<Job>>,
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
            Some(m) => Card { check: Check::Warn, title: tr("Firmware"), text: missing_text(m) },
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
                    ui.label(egui::RichText::new(missing_text(m)).color(theme.color(Token::Ink3)));
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

/// "X is missing (needed by D)", or without the driver when it isn't known.
fn missing_text(m: &MissingFirmware) -> String {
    if m.driver.is_empty() {
        tr("{} is missing.").replacen("{}", &m.file, 1)
    } else {
        tr("{} is missing (needed by {})").replacen("{}", &m.file, 1).replacen("{}", &m.driver, 1)
    }
}

impl LaApp {
    pub(crate) fn open_install_firmware(&mut self, file: &str) {
        self.close_install_firmware();
        self.ui.install_firmware = Some(InstallFirmware {
            file: file.to_owned(),
            package: fetch::package_for(file),
            inputs: Vec::new(),
            inputs_packed: false,
            unpacker: false,
            unpacker_checked: f64::NEG_INFINITY,
            job: None,
        });
    }

    /// Closes the dialog; a download or import still running is cancelled,
    /// and installs nothing.
    fn close_install_firmware(&mut self) {
        if let Some(job) = self.ui.install_firmware.take().and_then(|d| d.job) {
            job.cancel();
        }
    }

    /// Files or folders to install from: dropped on the dialog, or picked.
    pub(crate) fn on_firmware_input(&mut self, paths: Vec<PathBuf>) {
        let Some(d) = self.ui.install_firmware.as_mut() else { return };
        for p in paths {
            if !d.inputs.contains(&p) {
                d.inputs.push(p);
            }
        }
        d.inputs_packed = d.inputs.iter().any(|p| fetch::file_needs_unpacker(p));
    }

    pub(crate) fn install_firmware_dialog(&mut self, ctx: &Context) {
        /// A button's height: text beside one is centred on it.
        const ROW_H: f32 = 36.0;
        const UNPACKER_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);
        let Some(mut d) = self.ui.install_firmware.take() else { return };
        let theme = ctx.la_theme();
        let install_dir = self.firmware.install_dir.clone();
        let running = d.job.as_ref().filter(|j| !j.finished()).map(|j| j.kind);
        // While an archive needs an unpacker and none is there, look
        // again: the user may be installing one.
        if !d.unpacker && (d.inputs_packed || d.package.is_some_and(fetch::Package::needs_unpacker)) {
            let now = ctx.input(|i| i.time);
            if now - d.unpacker_checked >= UNPACKER_RECHECK.as_secs_f64() {
                d.unpacker = self.backend.firmware_unpacker();
                d.unpacker_checked = now;
            }
            ctx.request_repaint_after(UNPACKER_RECHECK);
        }
        let (mut close, mut cancel, mut rescan) = (false, false, false);
        // Browse for a folder (true) or files; the job to start.
        let (mut browse, mut start) = (None, None);
        let resp = Modal::new(Id::unique("install_firmware")).frame(super::dialogs::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(560.0);
            title_row(ui, tr("Install firmware"), &mut close);
            ui.label(
                egui::RichText::new(tr("{} is missing.").replacen("{}", &d.file, 1)).color(theme.color(Token::Ink4)),
            );
            ui.add_space(14.0);
            let heading = |ui: &mut Ui, text: &str| {
                ui.label(egui::RichText::new(text).text_style(Text::Lead.style()));
            };
            let dim = |ui: &mut Ui, text: &str| {
                ui.label(egui::RichText::new(text).color(theme.color(Token::Ink3)));
            };
            let caution = |ui: &mut Ui, text: &str| {
                ui.label(egui::RichText::new(format!("! {text}")).color(theme.color(Token::Caution)));
            };
            let no_unpacker = tr("Unpacking this needs unar, 7-Zip or bsdtar, and none is installed.");
            let how_to_get_one = tr(unpacker_hint(std::env::consts::OS));
            if let (Some(pkg), Some(dir)) = (d.package, &install_dir) {
                heading(ui, pkg.title);
                if let Some(note) = pkg.note {
                    dim(ui, note);
                }
                ui.add_space(10.0);
                if pkg.downloadable() {
                    heading(ui, tr("Download"));
                    let (size, hosts) = (human_size(pkg.download_size()), pkg.hosts().join(", "));
                    dim(ui, &tr("{} from {}").replacen("{}", &size, 1).replacen("{}", &hosts, 1));
                    // Without one the download would only fail at the end.
                    let can_unpack = d.unpacker || !pkg.needs_unpacker();
                    if !can_unpack {
                        caution(ui, no_unpacker);
                        dim(ui, how_to_get_one);
                        dim(ui, tr("Or download it yourself, unpack it and use the folder below:"));
                        for dl in pkg.downloads {
                            if let Some(url) = dl.urls.first() {
                                ui.hyperlink_to(format!("{} ↗", dl.name), *url);
                            }
                        }
                        ui.add_space(4.0);
                    }
                    ui.horizontal(|ui| {
                        ui.set_min_height(ROW_H);
                        if running == Some(JobKind::Download) {
                            let (done, total) = d.job.as_ref().and_then(|j| j.downloaded()).unwrap_or((0, 1));
                            let text = format!("{} / {}", human_size(done), human_size(total));
                            ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_width(380.0).text(text));
                            if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                                cancel = true;
                            }
                        } else if ui
                            .add_enabled(running.is_none() && can_unpack, Button::primary(tr("Download and install")))
                            .clicked()
                        {
                            start = Some(JobKind::Download);
                        }
                    });
                    ui.add_space(10.0);
                    heading(ui, tr("Or use files you have"));
                } else {
                    heading(ui, tr("Use files you have"));
                }
                match pkg.manual {
                    Some(m) => {
                        dim(ui, &tr("You need {}.").replacen("{}", m.what, 1));
                        if let Some(url) = m.url {
                            ui.hyperlink_to(format!("{url} ↗"), url);
                        }
                        for line in m.steps.lines().map(str::trim).filter(|l| !l.is_empty()) {
                            ui.label(egui::RichText::new(line).size(11.0).color(theme.color(Token::Ink4)));
                        }
                    }
                    None => dim(ui, tr("The firmware files, or an archive or a folder with them.")),
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    Frame::new()
                        .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
                        .corner_radius(4)
                        .inner_margin(Margin::symmetric(8, 10))
                        .show(ui, |ui| {
                            ui.set_width(340.0);
                            if d.inputs.is_empty() {
                                ui.label(
                                    egui::RichText::new(tr("Drop files or a folder here"))
                                        .color(theme.color(Token::Ink4)),
                                );
                            }
                            for p in &d.inputs {
                                ui.add(egui::Label::new(p.display().to_string()).truncate());
                            }
                        });
                    if ui.add_enabled(running.is_none(), Button::secondary(tr("Files…"))).clicked() {
                        browse = Some(false);
                    }
                    if ui.add_enabled(running.is_none(), Button::secondary(tr("Folder…"))).clicked() {
                        browse = Some(true);
                    }
                });
                if d.inputs_packed && !d.unpacker {
                    caution(ui, no_unpacker);
                    dim(ui, how_to_get_one);
                    dim(ui, tr("Or unpack it yourself and use the folder."));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = tr("Install from these files");
                    // The download, when there is one, is the main way.
                    let button = if pkg.downloadable() { Button::secondary(label) } else { Button::primary(label) };
                    if ui.add_enabled(running.is_none() && !d.inputs.is_empty(), button).clicked() {
                        start = Some(JobKind::Import);
                    }
                    if !d.inputs.is_empty() && running.is_none() && ui.link(tr("Clear")).clicked() {
                        d.inputs.clear();
                        d.inputs_packed = false;
                    }
                    if running == Some(JobKind::Import) {
                        ui.add(egui::Spinner::new());
                        if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                            cancel = true;
                        }
                    }
                });
                if let Some(job) = &d.job {
                    let log = job.log();
                    if !log.is_empty() {
                        ui.add_space(6.0);
                        Frame::new()
                            .stroke(egui::Stroke::new(1.0, theme.color(Token::Seam)))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ScrollArea::vertical().max_height(120.0).stick_to_bottom(true).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(egui::RichText::new(&log).monospace().size(11.0));
                                });
                            });
                    }
                    job.with_result(|result| match result {
                        Ok(installed) => {
                            let n = installed.files.len();
                            let dir = dir.display().to_string();
                            let text = if n == 1 {
                                tr("✓ Installed 1 file to {}").replacen("{}", &dir, 1)
                            } else {
                                tr("✓ Installed {} files to {}")
                                    .replacen("{}", &n.to_string(), 1)
                                    .replacen("{}", &dir, 1)
                            };
                            ui.label(egui::RichText::new(text).color(theme.color(Token::Good)));
                            for w in &installed.warnings {
                                caution(ui, w);
                            }
                            ui.horizontal(|ui| {
                                ui.set_min_height(ROW_H);
                                dim(ui, tr("Replug the device if it isn't found."));
                                if ui.add(Button::secondary(tr("Rescan"))).clicked() {
                                    rescan = true;
                                }
                            });
                        }
                        Err(fetch::Error::Cancelled) => dim(ui, tr("Cancelled. Nothing was installed.")),
                        Err(e) => {
                            ui.label(egui::RichText::new(format!("! {e}")).color(theme.color(Token::Alert)));
                            if ui.link(tr("Copy log")).clicked() {
                                ui.ctx().copy_text(format!("{log}\n{e}"));
                            }
                        }
                    });
                }
            } else {
                // A file the registry doesn't know: only the vendor has it.
                dim(ui, tr("No source is known for this file. Get it from the device's vendor and put it in one of:"));
                for (p, exists) in &self.firmware.paths {
                    let mark = if *exists { "✓" } else { "○" };
                    ui.label(
                        egui::RichText::new(format!("{mark} {}", p.display()))
                            .size(11.0)
                            .color(theme.color(Token::Ink4)),
                    );
                }
            }
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(Button::primary(tr("Close"))).clicked() {
                    close = true;
                }
            });
        });
        if let Some(folder) = browse {
            let (dialogs, tx, ctx2) = (self.dialogs.clone(), self.events_tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let paths = if folder {
                    dialogs.pick_folder(None).into_iter().collect()
                } else {
                    dialogs.open_files(tr("Choose the files"), None, &[])
                };
                let _ = tx.send(AppEvent::FirmwareInput(paths));
                ctx2.request_repaint();
            });
        }
        if let (Some(kind), Some(pkg), Some(dir)) = (start, d.package, install_dir) {
            let total = if kind == JobKind::Download { pkg.download_size() } else { 0 };
            let ctx2 = ctx.clone();
            let job = Arc::new(Job::new(kind, total, move || ctx2.request_repaint()));
            d.job = Some(job.clone());
            let (backend, inputs) = (self.backend.clone(), d.inputs.clone());
            std::thread::spawn(move || {
                let result = match kind {
                    JobKind::Download => backend.fetch_firmware(pkg, &dir, &*job),
                    JobKind::Import => backend.import_firmware(pkg, &inputs, &dir, &*job),
                };
                job.finish(result);
            });
        }
        if cancel && let Some(job) = &d.job {
            job.cancel();
        }
        if rescan {
            self.rescan_devices();
        }
        self.ui.install_firmware = Some(d);
        if close || resp.should_close() {
            self.close_install_firmware();
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
