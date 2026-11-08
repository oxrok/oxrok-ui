//! The native macOS menu bar (build option `macos-menu`): the
//! menu model of `menubar` as an `NSMenu` through `muda`, with the macOS
//! app and Window menus.

use super::menubar::{Act, Node, QUIT, sub_enabled};
use crate::app::commands::{self, Combo, KeySpec};
use crate::app::{APP_NAME, LaApp};
use crossbeam_channel::{Receiver, unbounded};
use egui::{Event, Key, Modifiers};
use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

/// With a text field focused, these go to the field.
const TEXT_FIELD_COMMANDS: [&str; 3] = ["edit.undo", "edit.redo", "marker.select_all"];

/// The installed menu and what it was built from.
pub(crate) struct NativeMenu {
    _menu: Menu,
    signature: String,
    events: Receiver<String>,
}

/// The native key equivalent for a combo: only bindings with ⌘.
pub(crate) fn accelerator_text(c: &Combo) -> Option<String> {
    if !c.cmd || c.hidden {
        return None;
    }
    let KeySpec::Key(k) = c.key else { return None };
    let code = match k {
        Key::Num0
        | Key::Num1
        | Key::Num2
        | Key::Num3
        | Key::Num4
        | Key::Num5
        | Key::Num6
        | Key::Num7
        | Key::Num8
        | Key::Num9 => format!("Digit{}", k.name()),
        Key::Comma => "Comma".into(),
        Key::OpenBracket => "BracketLeft".into(),
        Key::CloseBracket => "BracketRight".into(),
        Key::Equals => "Equal".into(),
        Key::Minus => "Minus".into(),
        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => format!("{k:?}"),
        k if k.name().len() == 1 && k.name().chars().all(|ch| ch.is_ascii_alphabetic()) => format!("Key{}", k.name()),
        k => k.name().to_owned(),
    };
    let mut s = String::from("Cmd+");
    if c.ctrl {
        s.push_str("Ctrl+");
    }
    if c.alt {
        s.push_str("Alt+");
    }
    if c.shift {
        s.push_str("Shift+");
    }
    s.push_str(&code);
    Some(s)
}

fn accelerator(id: &str) -> Option<Accelerator> {
    let c = commands::get(id)?;
    c.keys.iter().filter(|k| k.cmd && !k.hidden).find_map(accelerator_text)?.parse().ok()
}

/// Item ids: `cmd:‹id›`, `child:‹parent›␟‹key›`, `sp:‹id›`.
fn node_id(n: &Node) -> Option<String> {
    match n {
        Node::Cmd { id, .. } => Some(format!("cmd:{id}")),
        Node::Child { parent, key, .. } => Some(format!("child:{parent}\u{1f}{key}")),
        Node::Special { id, .. } => Some(format!("sp:{id}")),
        _ => None,
    }
}

pub(crate) fn parse_id(id: &str) -> Option<Act> {
    if let Some(c) = id.strip_prefix("cmd:") {
        return commands::get(c).map(|c| Act::Run(c.id));
    }
    if let Some(rest) = id.strip_prefix("child:") {
        let (parent, key) = rest.split_once('\u{1f}')?;
        return commands::get(parent).map(|c| Act::Child(c.id, key.to_owned()));
    }
    id.strip_prefix("sp:").map(|s| Act::Special(s.to_owned()))
}

impl LaApp {
    /// What the native menu shows now; it's rebuilt when this changes.
    fn menu_signature(&self, model: &[(&'static str, &'static str, Vec<Node>)]) -> String {
        fn walk(app: &LaApp, nodes: &[Node], out: &mut String) {
            for n in nodes {
                out.push_str(&format!("{n:?}"));
                match n {
                    Node::Cmd { id, .. } => {
                        out.push_str(&format!("{:?}{:?}", app.command_available(id).is_ok(), app.command_checked(id)))
                    }
                    Node::Sub { items, .. } => {
                        out.push_str(&format!("{}", sub_enabled(app, items)));
                        walk(app, items, out);
                    }
                    _ => {}
                }
            }
        }
        let mut s = String::new();
        for (_id, title, nodes) in model {
            s.push_str(title);
            walk(self, nodes, &mut s);
        }
        s
    }

    /// Installs the native menu (production only; never in UI tests).
    pub(crate) fn install_native_menu(&mut self) {
        // Sessions aren't window tabs: keep AppKit from adding Show Tab
        // Bar / Show All Tabs to the View menu.
        if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
            objc2_app_kit::NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
        }
        let (tx, rx) = unbounded();
        let ctx = self.ui.ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e.id.0);
            ctx.request_repaint();
        }));
        self.native_menu = Some(NativeMenu { _menu: Menu::new(), signature: String::new(), events: rx });
        self.sync_native_menu();
    }

    /// Every frame: run clicked items, then rebuild the menu if it changed.
    pub(crate) fn native_menu_frame(&mut self, ctx: &egui::Context) {
        let Some(m) = &self.native_menu else { return };
        let clicked: Vec<String> = m.events.try_iter().collect();
        for id in clicked {
            let Some(act) = parse_id(&id) else { continue };
            // A modal dialog keeps menu items from running.
            if !self.menus_allowed(ctx) && act != Act::Special(QUIT.into()) {
                continue;
            }
            if let Act::Run(cid) = act
                && TEXT_FIELD_COMMANDS.contains(&cid)
                && ctx.egui_wants_keyboard_input()
            {
                self.forward_to_text_field(cid);
                continue;
            }
            self.apply_menu_act(act);
        }
        self.sync_native_menu();
    }

    /// ⌘A / ⌘Z / ⇧⌘Z for the focused text field, as key presses.
    fn forward_to_text_field(&mut self, id: &str) {
        let (key, shift) = match id {
            "edit.undo" => (Key::Z, false),
            "edit.redo" => (Key::Z, true),
            _ => (Key::A, false),
        };
        let mut modifiers = Modifiers::MAC_CMD | Modifiers::COMMAND;
        if shift {
            modifiers |= Modifiers::SHIFT;
        }
        for pressed in [true, false] {
            self.ui.forward_events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        }
        self.ui.ctx.request_repaint();
    }

    fn sync_native_menu(&mut self) {
        let model = self.menu_model(true);
        let sig = self.menu_signature(&model);
        if self.native_menu.as_ref().is_some_and(|m| m.signature == sig) {
            return;
        }
        let menu = Menu::new();
        // The app menu.
        let quit = MenuItem::with_id("sp:quit", format!("Quit {APP_NAME}"), true, "Cmd+KeyQ".parse().ok());
        let settings = MenuItem::with_id("cmd:settings.open", "Settings…", true, accelerator("settings.open"));
        let app = Submenu::with_items(
            APP_NAME,
            true,
            &[
                &settings,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &quit,
            ],
        );
        if let Ok(app) = app {
            let _ = menu.append(&app);
        }
        let window = Submenu::with_items(
            "Window",
            true,
            &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(Some("Zoom"))],
        );
        for (id, title, nodes) in &model {
            if *id == "help"
                && let Ok(w) = &window
            {
                let _ = menu.append(w);
                w.set_as_windows_menu_for_nsapp();
            }
            let sub = Submenu::new(*title, true);
            self.append_nodes(&sub, nodes);
            let _ = menu.append(&sub);
            if *id == "help" {
                sub.set_as_help_menu_for_nsapp();
            }
        }
        menu.init_for_nsapp();
        if let Some(m) = self.native_menu.as_mut() {
            m._menu = menu;
            m.signature = sig;
        }
    }

    fn append_nodes(&self, sub: &Submenu, nodes: &[Node]) {
        for n in nodes {
            let id = node_id(n).unwrap_or_default();
            let item: Box<dyn IsMenuItem> = match n {
                Node::Sep => Box::new(PredefinedMenuItem::separator()),
                Node::Cmd { id: cid, label } => {
                    let enabled = self.command_available(cid).is_ok();
                    match self.command_checked(cid) {
                        Some(on) => Box::new(CheckMenuItem::with_id(id, label, enabled, on, accelerator(cid))),
                        None => Box::new(MenuItem::with_id(id, label, enabled, accelerator(cid))),
                    }
                }
                Node::Child { label, checked: Some(on), .. } => {
                    Box::new(CheckMenuItem::with_id(id, label, true, *on, None))
                }
                Node::Child { label, .. } => Box::new(MenuItem::with_id(id, label, true, None)),
                Node::Special { label, enabled, .. } => Box::new(MenuItem::with_id(id, label, *enabled, None)),
                Node::Sub { label, items } => {
                    let s = Submenu::new(label, sub_enabled(self, items));
                    self.append_nodes(&s, items);
                    Box::new(s)
                }
            };
            let _ = sub.append(item.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::commands::{k, t};

    #[test]
    fn accelerators_only_for_cmd_bindings() {
        assert_eq!(accelerator_text(&k(Key::S).cmd().shift()).as_deref(), Some("Cmd+Shift+KeyS"));
        assert_eq!(accelerator_text(&k(Key::Num1).cmd()).as_deref(), Some("Cmd+Digit1"));
        assert_eq!(accelerator_text(&k(Key::OpenBracket).cmd().shift()).as_deref(), Some("Cmd+Shift+BracketLeft"));
        assert_eq!(accelerator_text(&k(Key::ArrowUp).cmd()).as_deref(), Some("Cmd+ArrowUp"));
        assert_eq!(accelerator_text(&k(Key::Space)), None, "plain keys stay with the app");
        assert_eq!(accelerator_text(&t("?")), None);
        for c in commands::registry() {
            for combo in &c.keys {
                if let Some(text) = accelerator_text(combo) {
                    assert!(text.parse::<Accelerator>().is_ok(), "{}: {text}", c.id);
                }
            }
        }
    }

    #[test]
    fn ids_round_trip() {
        let n = Node::Child { parent: "view.theme", key: "Light".into(), label: "Light".into(), checked: None };
        assert_eq!(parse_id(&node_id(&n).unwrap()), Some(Act::Child("view.theme", "Light".into())));
        assert_eq!(parse_id("cmd:file.save"), Some(Act::Run("file.save")));
        assert_eq!(parse_id("sp:quit"), Some(Act::Special("quit".into())));
        assert_eq!(parse_id("cmd:nope"), None);
    }
}
