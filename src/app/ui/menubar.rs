//! The menus as a model, and the in-window menu bar that draws it,
//! 24 pt above the main area. The native macOS menu (`macmenu`)
//! draws the same model. Every item runs a registry command; items of
//! features that ship later are left out.

use super::commands::MAC;
use super::rowctl::submenu;
use crate::app::LaApp;
use crate::app::commands;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_check_item, menu_item, menu_separator};
use egui::{Frame, Panel, Stroke, Ui};

/// The menu bar's height.
pub(crate) const MENU_BAR_HEIGHT: f32 = 24.0;
const MENU_W: f32 = 260.0;

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    /// A registry command, with the label the menu shows.
    Cmd {
        id: &'static str,
        label: String,
    },
    /// A sub-command of `parent`, ✓ when it's the current choice.
    Child {
        parent: &'static str,
        key: String,
        label: String,
        checked: Option<bool>,
    },
    /// An action that isn't a command: Clear Recent, Quit, a device.
    Special {
        id: String,
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        tip: Option<String>,
    },
    Sub {
        label: String,
        items: Vec<Node>,
    },
    Sep,
}

/// What a menu click asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    Run(&'static str),
    Child(&'static str, String),
    Special(String),
}

/// Special ids.
pub(crate) const CLEAR_RECENT: &str = "clear_recent";
pub(crate) const QUIT: &str = "quit";
const DEVICE: &str = "device:";

fn cmd(id: &'static str, label: &'static str) -> Node {
    Node::Cmd { id, label: crate::l10n::tr(label).to_owned() }
}

impl LaApp {
    /// The menus in order, titles and entries. `native` leaves out
    /// what the macOS app menu holds instead.
    pub(crate) fn menu_model(&self, native: bool) -> Vec<(&'static str, Vec<Node>)> {
        let label = |id: &'static str| {
            commands::get(id).map(|c| match id {
                // The label reflects the state (Capture menu).
                "capture.toggle" => self.command_label(c),
                _ => crate::l10n::tr(c.label).to_owned(),
            })
        };
        let c = |id: &'static str| Node::Cmd { id, label: label(id).unwrap_or_default() };
        let children = |id: &'static str, title: &'static str, current: Option<&str>| Node::Sub {
            label: crate::l10n::tr(title).to_owned(),
            items: self
                .command_children(id)
                .into_iter()
                .map(|(key, label)| {
                    let checked = current.map(|cur| cur == key);
                    Node::Child { parent: id, key, label, checked }
                })
                .collect(),
        };

        let mut recent: Vec<Node> = self
            .command_children("file.open_recent")
            .into_iter()
            .map(|(key, label)| Node::Child { parent: "file.open_recent", key, label, checked: None })
            .collect();
        if !recent.is_empty() {
            recent.push(Node::Sep);
            recent.push(Node::Special {
                id: CLEAR_RECENT.into(),
                label: crate::l10n::tr("Clear Recent").into(),
                shortcut: None,
                enabled: true,
                tip: None,
            });
        }
        let mut file = vec![
            c("session.new"),
            c("file.open"),
            Node::Sub { label: crate::l10n::tr("Open Recent").into(), items: recent },
            Node::Sep,
            c("file.save"),
            c("file.save_as"),
            Node::Sep,
            c("file.export"),
            Node::Sep,
            c("session.close"),
        ];
        if !native {
            // On the native menu these are in the app menu.
            file.push(Node::Sep);
            file.push(cmd("settings.open", "Settings…"));
            file.push(Node::Special {
                id: QUIT.into(),
                label: crate::l10n::tr("Quit").into(),
                shortcut: Some(if MAC { "⌘Q" } else { "Ctrl+Q" }.into()),
                enabled: true,
                tip: None,
            });
        }
        let edit = vec![
            c("edit.undo"),
            c("edit.redo"),
            Node::Sep,
            cmd("marker.select_all", "Select All"),
            Node::Sep,
            c("session.rename"),
            Node::Sep,
            Node::Sub {
                label: crate::l10n::tr("Delete Data").into(),
                items: vec![
                    cmd("data.delete_off_screen", "Off Screen"),
                    cmd("data.delete_before_view", "Before the View"),
                    cmd("data.delete_after_view", "After the View"),
                ],
            },
        ];
        let view = vec![
            cmd("nav.zoom_in", "Zoom In"),
            cmd("nav.zoom_out", "Zoom Out"),
            c("nav.fit"),
            Node::Sep,
            cmd("nav.jump", "Jump to Time…"),
            cmd("nav.center_trigger", "Center on Trigger"),
            Node::Sep,
            c("history.back"),
            c("history.forward"),
            Node::Sep,
            Node::Sub {
                label: crate::l10n::tr("Inspector").into(),
                items: vec![
                    cmd("inspector.toggle", "Show/Hide Inspector"),
                    Node::Sep,
                    cmd("panel.device", "Device"),
                    cmd("panel.analyzers", "Analyzers"),
                    cmd("panel.measurements", "Measurements"),
                    cmd("panel.markers", "Markers"),
                    Node::Sep,
                    cmd("inspector.detach", "Detach Inspector"),
                ],
            },
            Node::Sep,
            cmd("panel.notes", "Notes"),
            cmd("view.traces_color", "Color Waveforms by Channel"),
            cmd("view.wire_status", "Wire Status"),
            children("view.trace_width", "Trace Width", Some(self.settings.waveform.trace_width.label())),
            Node::Sep,
            children("view.theme", "Theme", Some(self.settings.appearance.theme.label())),
        ];
        let blocked = self.create_blocked();
        let devices: Vec<Node> = self
            .devices
            .clone()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, d)| Node::Special {
                id: format!("{DEVICE}{i}"),
                label: format!("New Session on {}", self.device_label(d)),
                shortcut: None,
                enabled: blocked.is_none(),
                tip: blocked.map(str::to_owned),
            })
            .collect();
        // The help row closes the Devices list.
        let mut devices = devices;
        devices.extend([Node::Sep, c("help.connection")]);
        let capture = vec![
            c("capture.toggle"),
            c("capture.repeat"),
            Node::Sep,
            c("capture.copy_cli"),
            Node::Sep,
            Node::Sub { label: "Devices".into(), items: devices },
        ];
        let analyze = vec![
            c("analyzer.add"),
            c("analyzer.pause"),
            Node::Sep,
            c("marker.add"),
            cmd("marker.add_pair", "Add Marker Pair"),
            c("range.add"),
            Node::Sep,
            c("table.find"),
        ];
        let help = vec![
            cmd("help.shortcuts", "Keyboard Shortcuts"),
            Node::Sep,
            c("help.onboarding"),
            c("help.connection"),
            Node::Sep,
            c("help.about"),
        ];
        vec![
            (crate::l10n::tr("File"), file),
            (crate::l10n::tr("Edit"), edit),
            (crate::l10n::tr("View"), view),
            (crate::l10n::tr("Capture"), capture),
            (crate::l10n::tr("Analyze"), analyze),
            (crate::l10n::tr("Help"), help),
        ]
    }

    /// Runs a menu click (either menu bar).
    pub(crate) fn apply_menu_act(&mut self, a: Act) {
        match a {
            Act::Run(id) => self.dispatch(id),
            Act::Child(id, key) => self.dispatch_child(id, &key),
            Act::Special(s) if s == CLEAR_RECENT => {
                self.settings.files.recent_files.clear();
                self.mark_settings_dirty();
            }
            // Through the quit prompt.
            Act::Special(s) if s == QUIT => {
                self.ui.ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close)
            }
            Act::Special(s) => {
                let device = s
                    .strip_prefix(DEVICE)
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| self.devices.as_ref()?.get(i).cloned());
                if let Some(d) = device {
                    let width = self.session.as_ref().map_or(800.0, |s| s.view.state().width_px);
                    self.open_session(&d, width);
                }
            }
        }
    }

    /// The in-window menu bar.
    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let model = self.menu_model(false);
        let mut out = None;
        Panel::top("menu_bar")
            .exact_size(MENU_BAR_HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(theme.color(Token::Lift)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter()
                    .line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
                ui.spacing_mut().button_padding = egui::vec2(10.0, 2.0);
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.add_space(4.0);
                    for (title, nodes) in &model {
                        let text = egui::RichText::new(*title).text_style(Text::Prose.style());
                        egui::containers::menu::MenuButton::new(text).ui(ui, |ui| {
                            ui.set_width(MENU_W);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            self.draw_nodes(ui, nodes, &mut out);
                        });
                    }
                });
            });
        if let Some(a) = out {
            self.apply_menu_act(a);
        }
    }

    fn draw_nodes(&self, ui: &mut Ui, nodes: &[Node], out: &mut Option<Act>) {
        for n in nodes {
            match n {
                Node::Sep => menu_separator(ui),
                Node::Cmd { id, label } => {
                    let Some(c) = commands::get(id) else { continue };
                    let available = self.command_available(id);
                    let key = c.primary_key(MAC);
                    let r = ui.add_enabled_ui(available.is_ok(), |ui| match self.command_checked(id) {
                        Some(on) => menu_check_item(ui, label, on, key.as_deref()),
                        None => menu_item(ui, label, key.as_deref()),
                    });
                    let r = match available {
                        Err(why) => r.inner.on_disabled_hover_text(why),
                        Ok(()) => r.inner,
                    };
                    if r.clicked() {
                        *out = Some(Act::Run(id));
                        ui.close();
                    }
                }
                Node::Child { parent, key, label, checked } => {
                    let r = match checked {
                        Some(on) => menu_check_item(ui, label, *on, None),
                        None => menu_item(ui, label, None),
                    };
                    let r = if *parent == "file.open_recent" { r.on_hover_text(key) } else { r };
                    if r.clicked() {
                        *out = Some(Act::Child(parent, key.clone()));
                        ui.close();
                    }
                }
                Node::Special { id, label, shortcut, enabled, tip } => {
                    let r = ui.add_enabled_ui(*enabled, |ui| menu_item(ui, label, shortcut.as_deref())).inner;
                    let r = match tip {
                        Some(t) => r.on_disabled_hover_text(t),
                        None => r,
                    };
                    if r.clicked() {
                        *out = Some(Act::Special(id.clone()));
                        ui.close();
                    }
                }
                Node::Sub { label, items } => {
                    let width = if label == "Open Recent" { 320.0 } else { 240.0 };
                    ui.add_enabled_ui(sub_enabled(self, items), |ui| {
                        submenu(ui, label, width, None, |ui| self.draw_nodes(ui, items, out));
                    });
                }
            }
        }
    }
}

/// A submenu is enabled when something in it can run.
pub(crate) fn sub_enabled(app: &LaApp, items: &[Node]) -> bool {
    items.iter().any(|n| match n {
        Node::Cmd { id, .. } => app.command_available(id).is_ok(),
        Node::Child { .. } => true,
        Node::Special { enabled, .. } => *enabled,
        Node::Sub { items, .. } => sub_enabled(app, items),
        Node::Sep => false,
    })
}
