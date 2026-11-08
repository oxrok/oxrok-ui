//! The command registry: every action that a key, a menu, the
//! palette or a toolbar button can run, with its default bindings.
//!
//! This module is data only. What a command does, whether it applies now
//! and its state live in `ui::commands` (`LaApp::dispatch`).

use egui::{Key, Modifiers};
use std::sync::OnceLock;

/// Sections, in the Keyboard Shortcuts dialog's order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Category {
    Capture,
    SignalNavigation,
    AnalogSignals,
    Sessions,
    Markers,
    Measurements,
    Panels,
    Misc,
    Data,
    ViewHistory,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Capture,
        Category::SignalNavigation,
        Category::AnalogSignals,
        Category::Sessions,
        Category::Markers,
        Category::Measurements,
        Category::Panels,
        Category::Misc,
        Category::Data,
        Category::ViewHistory,
    ];

    pub fn label(self) -> &'static str {
        let en = match self {
            Category::Capture => "Capture",
            Category::SignalNavigation => "Signal Navigation",
            Category::AnalogSignals => "Analog Signals",
            Category::Sessions => "Sessions",
            Category::Markers => "Markers",
            Category::Measurements => "Measurements",
            Category::Panels => "Inspector",
            Category::Misc => "Misc",
            Category::Data => "Data",
            Category::ViewHistory => "View History",
        };
        crate::l10n::tr(en)
    }
}

/// The key of a binding: a key, or a typed character (punctuation
/// matches the character, so it works on any layout).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeySpec {
    Key(Key),
    Text(&'static str),
}

/// One binding (a key with optional modifiers, no chords).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    /// CmdOrCtrl.
    pub cmd: bool,
    /// The Control key on every platform (`Ctrl+Tab`).
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: KeySpec,
    /// Only on macOS (`Cmd+Shift+]`, `Cmd+[`).
    pub mac_only: bool,
    /// Works, but isn't shown (`Shift+=` for `+`).
    pub hidden: bool,
}

const fn combo(key: KeySpec) -> Combo {
    Combo { cmd: false, ctrl: false, shift: false, alt: false, key, mac_only: false, hidden: false }
}

/// A plain key.
pub const fn k(key: Key) -> Combo {
    combo(KeySpec::Key(key))
}

/// A typed character.
pub const fn t(text: &'static str) -> Combo {
    combo(KeySpec::Text(text))
}

impl Combo {
    pub const fn cmd(mut self) -> Self {
        self.cmd = true;
        self
    }
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
    pub const fn mac(mut self) -> Self {
        self.mac_only = true;
        self
    }
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Whether a key press with `mods` is this combo (on `mac` or not).
    pub fn matches_key(&self, key: Key, mods: Modifiers, mac: bool) -> bool {
        if self.mac_only && !mac {
            return false;
        }
        let KeySpec::Key(k) = self.key else { return false };
        if k != key {
            return false;
        }
        // On Windows and Linux CmdOrCtrl is Ctrl, so the two are one key.
        let mods_ok = if mac {
            mods.command == self.cmd && mods.ctrl == self.ctrl
        } else {
            (mods.command || mods.ctrl) == (self.cmd || self.ctrl)
        };
        mods_ok && mods.shift == self.shift && mods.alt == self.alt
    }

    /// Whether typed text `text` with `mods` is this combo. Shift is part
    /// of the character, so it isn't compared.
    pub fn matches_text(&self, text: &str, mods: Modifiers) -> bool {
        matches!(self.key, KeySpec::Text(t) if t == text) && !mods.command && !mods.alt && !mods.ctrl
    }

    /// The key caps, with the platform's names.
    pub fn caps(&self, mac: bool) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.ctrl {
            v.push(if mac { "⌃" } else { "Ctrl" });
        }
        if self.cmd && !(self.ctrl && !mac) {
            v.push(if mac { "⌘" } else { "Ctrl" });
        }
        if self.alt {
            v.push(if mac { "⌥" } else { "Alt" });
        }
        if self.shift {
            v.push(if mac { "⇧" } else { "Shift" });
        }
        v.push(match self.key {
            KeySpec::Text(t) => t,
            KeySpec::Key(k) => key_name(k, mac),
        });
        v
    }

    /// One string for a menu's shortcut column: `⌘⇧S` or `Ctrl+Shift+S`.
    pub fn text(&self, mac: bool) -> String {
        let caps = self.caps(mac);
        if mac { caps.concat() } else { caps.join("+") }
    }
}

/// A key cap's name.
pub fn key_name(k: Key, mac: bool) -> &'static str {
    match k {
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Comma => ",",
        Key::Backspace if mac => "⌫",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::PageUp => "Page Up",
        Key::PageDown => "Page Down",
        Key::Escape => "Esc",
        k => k.name(),
    }
}

/// A display-only label (`CmdOrCtrl+Shift+Drag`) as key caps for `mac`.
pub fn label_caps(label: &str, mac: bool) -> Vec<String> {
    label
        .split('+')
        .map(|p| match p {
            "CmdOrCtrl" if mac => "⌘".to_owned(),
            "CmdOrCtrl" => "Ctrl".to_owned(),
            "Shift" if mac => "⇧".to_owned(),
            "Backspace" if mac => "⌫".to_owned(),
            p => p.to_owned(),
        })
        .collect()
}

/// Every name a key cap can be searched by.
pub fn search_names(cap: &str) -> &str {
    match cap {
        "⌘" => "⌘ cmd command ctrl control",
        "Ctrl" => "ctrl control cmd command",
        "⌃" => "⌃ ctrl control",
        "⇧" | "Shift" => "⇧ shift",
        "⌥" | "Alt" => "⌥ alt option",
        "⌫" | "Backspace" => "⌫ backspace",
        c => c,
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    /// The palette's second column.
    pub description: &'static str,
    pub category: Category,
    pub keys: Vec<Combo>,
    /// Display-only hints for mouse gestures, e.g. *Shift+Drag*.
    pub key_labels: &'static [&'static str],
    pub hidden_in_palette: bool,
    pub hidden_in_shortcuts: bool,
    /// pointer-focus: acts on the hovered row; tried first.
    pub pointer: bool,
    /// Has sub-commands.
    pub has_children: bool,
}

fn cmd(id: &'static str, label: &'static str, category: Category, description: &'static str) -> Command {
    Command {
        id,
        label,
        description,
        category,
        keys: Vec::new(),
        key_labels: &[],
        hidden_in_palette: false,
        hidden_in_shortcuts: false,
        pointer: false,
        has_children: false,
    }
}

impl Command {
    fn keys(mut self, keys: &[Combo]) -> Self {
        self.keys = keys.to_vec();
        self
    }
    fn labels(mut self, labels: &'static [&'static str]) -> Self {
        self.key_labels = labels;
        self
    }
    fn no_palette(mut self) -> Self {
        self.hidden_in_palette = true;
        self
    }
    fn no_dialog(mut self) -> Self {
        self.hidden_in_shortcuts = true;
        self
    }
    fn pointer(mut self) -> Self {
        self.pointer = true;
        self
    }
    fn children(mut self) -> Self {
        self.has_children = true;
        self
    }

    /// The bindings shown on this platform.
    pub fn shown_keys(&self, mac: bool) -> impl Iterator<Item = &Combo> {
        self.keys.iter().filter(move |c| !c.hidden && (mac || !c.mac_only))
    }

    /// The first shown binding, for menus.
    pub fn primary_key(&self, mac: bool) -> Option<String> {
        self.shown_keys(mac).next().map(|c| c.text(mac))
    }
}

/// The registry, in order (also the palette's *All commands* order).
pub fn registry() -> &'static [Command] {
    static R: OnceLock<Vec<Command>> = OnceLock::new();
    R.get_or_init(build)
}

pub fn get(id: &str) -> Option<&'static Command> {
    registry().iter().find(|c| c.id == id)
}

fn build() -> Vec<Command> {
    use Category::*;
    let mut v = vec![
        // ---- Capture ----
        cmd("capture.toggle", "Start/Stop Recording", Capture, "record from the device, or stop")
            .keys(&[k(Key::Space), k(Key::R).cmd()]),
        cmd("capture.repeat", "Repeat Capture", Capture, "wait for the next trigger each time a capture ends"),
        cmd("file.save", "Save Capture", Capture, "save the capture as a .oxrok file").keys(&[k(Key::S).cmd()]),
        cmd("file.save_as", "Save Capture As…", Capture, "save the capture to a new file")
            .keys(&[k(Key::S).cmd().shift()]),
        cmd("file.open", "Open Capture…", Capture, "open .oxrok, .sr and other capture files").keys(&[k(Key::O).cmd()]),
        cmd("file.open_recent", "Open Recent", Capture, "reopen a capture file").children(),
        cmd("file.export", "Export Data…", Capture, "write CSV, binary, VCD or sigrok files").keys(&[k(Key::E).cmd()]),
        cmd("capture.copy_cli", "Copy oxrok Command", Capture, "the command line for the current capture settings"),
        // ---- Signal Navigation ----
        cmd("nav.jump", "Jump to Time", SignalNavigation, "center the view on a time").keys(&[k(Key::G)]),
        cmd("nav.prev_edge", "Previous Edge", SignalNavigation, "the edge before the view center (hovered row)")
            .keys(&[t(",")])
            .pointer()
            .no_palette(),
        cmd("nav.next_edge", "Next Edge", SignalNavigation, "the edge after the view center (hovered row)")
            .keys(&[t(".")])
            .pointer()
            .no_palette(),
        cmd("nav.prev_result", "Previous Result", SignalNavigation, "the decoded result before (hovered row)")
            .keys(&[t("<")])
            .pointer()
            .no_palette(),
        cmd("nav.next_result", "Next Result", SignalNavigation, "the decoded result after (hovered row)")
            .keys(&[t(">")])
            .pointer()
            .no_palette(),
        cmd("nav.start", "Go to Start", SignalNavigation, "show the first data").keys(&[k(Key::Home)]),
        cmd("nav.end", "Go to End", SignalNavigation, "show the newest data").keys(&[k(Key::End)]),
        cmd("nav.zoom_in", "X Zoom In", SignalNavigation, "half the visible time").keys(&[
            k(Key::Equals),
            k(Key::Plus),
            k(Key::ArrowUp),
            k(Key::Equals).shift().hidden(),
            k(Key::Plus).shift().hidden(),
        ]),
        cmd("nav.zoom_out", "X Zoom Out", SignalNavigation, "twice the visible time")
            .keys(&[k(Key::Minus), k(Key::ArrowDown)]),
        cmd("nav.full_zoom_in", "Full Zoom In", SignalNavigation, "zoom in as far as it goes")
            .keys(&[k(Key::ArrowUp).shift()]),
        cmd("nav.fit", "Fit All Data", SignalNavigation, "show the whole capture")
            .keys(&[k(Key::F), k(Key::ArrowDown).shift()]),
        cmd("nav.pan_left", "Pan Left", SignalNavigation, "a third of the view earlier")
            .keys(&[k(Key::ArrowLeft)])
            .no_palette(),
        cmd("nav.pan_right", "Pan Right", SignalNavigation, "a third of the view later")
            .keys(&[k(Key::ArrowRight)])
            .no_palette(),
        cmd("nav.page_left", "Pan Page Left", SignalNavigation, "a whole view earlier")
            .keys(&[k(Key::ArrowLeft).shift()])
            .no_palette(),
        cmd("nav.page_right", "Pan Page Right", SignalNavigation, "a whole view later")
            .keys(&[k(Key::ArrowRight).shift()])
            .no_palette(),
        cmd("nav.rows_up", "Scroll Rows Up", SignalNavigation, "a page of rows up")
            .keys(&[k(Key::PageUp)])
            .labels(&["Scroll over the labels"])
            .no_palette(),
        cmd("nav.rows_down", "Scroll Rows Down", SignalNavigation, "a page of rows down")
            .keys(&[k(Key::PageDown)])
            .no_palette(),
        cmd("nav.center_trigger", "Center Current Trigger", SignalNavigation, "show the trigger in the middle")
            .keys(&[k(Key::T)]),
        cmd("nav.x_range_zoom", "X Range Zoom", SignalNavigation, "drag across a time range")
            .labels(&["CmdOrCtrl+Shift+Drag Horizontal"])
            .no_palette(),
        cmd("analyzer.add", "Add Analyzer…", Capture, "decode a protocol on a row"),
        cmd("analyzer.pause", "Pause Analyzers", Capture, "stop decoding until resumed"),
        // ---- Analog Signals ----
        cmd("analog.y_zoom_in", "Y Zoom In", AnalogSignals, "the hovered analog row")
            .keys(&[k(Key::ArrowUp).cmd()])
            .labels(&["CmdOrCtrl+Scroll"])
            .pointer()
            .no_palette(),
        cmd("analog.y_zoom_out", "Y Zoom Out", AnalogSignals, "the hovered analog row")
            .keys(&[k(Key::ArrowDown).cmd()])
            .pointer()
            .no_palette(),
        cmd("analog.y_fit", "Y Fit", AnalogSignals, "fit the hovered analog row to its visible data")
            .keys(&[k(Key::Y)])
            .pointer()
            .no_palette(),
        cmd("analog.xy_range_zoom", "XY Range Zoom", AnalogSignals, "drag a box")
            .labels(&["CmdOrCtrl+Drag"])
            .no_palette(),
        cmd("analog.y_range_zoom", "Y Range Zoom", AnalogSignals, "drag up or down")
            .labels(&["CmdOrCtrl+Shift+Drag Vertical"])
            .no_palette(),
        cmd("analog.y_fit_all", "Y Fit All", AnalogSignals, "fit every analog row to its visible data")
            .keys(&[k(Key::Y).shift()]),
        // ---- Sessions ----
        cmd("session.new", "New Session", Sessions, "another capture on the same device").keys(&[k(Key::T).cmd()]),
        cmd("session.close", "Close Session", Sessions, "close the active session").keys(&[k(Key::W).cmd()]),
        cmd("session.show", "Show Sessions", Sessions, "list the open sessions").keys(&[k(Key::T).cmd().shift()]),
        cmd("session.next", "Next Session", Sessions, "switch to the next session").keys(&[
            k(Key::Tab).ctrl(),
            k(Key::CloseBracket).cmd().shift().mac(),
            k(Key::CloseCurlyBracket).cmd().shift().mac().hidden(),
        ]),
        cmd("session.prev", "Previous Session", Sessions, "switch to the previous session").keys(&[
            k(Key::Tab).ctrl().shift(),
            k(Key::OpenBracket).cmd().shift().mac(),
            k(Key::OpenCurlyBracket).cmd().shift().mac().hidden(),
        ]),
        cmd("session.goto", "Go to Session", Sessions, "switch to a session by name")
            .labels(&["CmdOrCtrl+1 … 9"])
            .children(),
        cmd("session.rename", "Rename Session", Sessions, "rename the active session"),
        cmd("session.close_others", "Close Other Sessions", Sessions, "keep only the active session"),
        // ---- Markers ----
        cmd("marker.add", "Add Marker", Markers, "place a marker at the pointer").keys(&[k(Key::M)]),
        cmd("marker.add_pair", "Add Timing Markers Pair", Markers, "two linked markers").keys(&[k(Key::M).shift()]),
        cmd("marker.delete_selected", "Delete Selected Markers", Markers, "the selected markers and ranges")
            .keys(&[k(Key::Backspace), k(Key::Delete)])
            .no_palette(),
        cmd("marker.select_all", "Select All Markers", Markers, "in the open Timing Markers panel")
            .keys(&[k(Key::A).cmd()])
            .no_palette(),
        // ---- Measurements ----
        cmd("range.add", "Add Range Measurement", Measurements, "measure between two times")
            .keys(&[k(Key::R)])
            .labels(&["Shift+Drag"]),
        cmd("range.pulse", "Measure Pulse", Measurements, "wrap the pulse under the pointer in a range")
            .keys(&[k(Key::R).shift()])
            .pointer()
            .no_palette(),
        cmd("range.delete_selected", "Delete Selected Range Measurements", Measurements, "")
            .labels(&["Backspace", "Delete"])
            .no_palette(),
        cmd("range.select_all", "Select All Range Measurements", Measurements, "")
            .labels(&["CmdOrCtrl+A"])
            .no_palette(),
        // ---- Inspector tabs ----
        cmd("inspector.toggle", "Toggle Inspector", Panels, "show or hide the Inspector")
            .keys(&[k(Key::I).alt(), k(Key::Backslash).cmd()]),
        cmd("panel.device", "Device tab", Panels, "open the Inspector on the Device tab").keys(&[k(Key::Num1).alt()]),
        cmd("panel.analyzers", "Analyzers tab", Panels, "open the Inspector on the Analyzers tab")
            .keys(&[k(Key::Num2).alt()]),
        cmd("panel.measurements", "Measurements tab", Panels, "open the Inspector on the Measurements tab")
            .keys(&[k(Key::Num3).alt()]),
        cmd("panel.markers", "Markers tab", Panels, "open the Inspector on the Markers tab")
            .keys(&[k(Key::Num4).alt()]),
        cmd("panel.notes", "Notes", Panels, "show or hide the Notes panel").keys(&[k(Key::N).cmd()]),
        cmd("inspector.detach", "Detach Inspector", Panels, "move the Inspector to a window of its own, or back")
            .keys(&[k(Key::I).alt().shift()]),
        // ---- Misc ----
        cmd("help.shortcuts", "Toggle Keyboard Shortcuts Menu", Misc, "every key binding").keys(&[t("?")]),
        cmd("help.connection", "Device Connection Help…", Misc, "when a device isn't found"),
        cmd("help.about", "About", Misc, "version, libraries and licenses"),
        cmd("help.onboarding", "Start Onboarding", Misc, "the welcome wizard: theme, device access, tips").keys(&[k(
            Key::O,
        )
        .cmd()
        .shift()]),
        cmd("table.find", "Find in Data Table", Misc, "search the decoded data").keys(&[k(Key::F).cmd()]),
        cmd("view.traces_color", "Toggle Traces Color", Misc, "color waveforms by channel, or neutral"),
        cmd("view.wire_status", "Wire Status", Misc, "show the live pin levels in the row labels"),
        cmd("view.trace_width", "Trace Width", Misc, "thin, normal or bold traces").children(),
        cmd("view.theme", "Theme", Misc, "light, dark or like the system").children(),
        cmd("settings.open", "Open Settings", Misc, "theme, UI scale, folders and more").keys(&[k(Key::Comma).cmd()]),
        cmd("view.ui_bigger", "Increase UI Size", Misc, "make text and controls bigger").keys(&[
            k(Key::Equals).cmd(),
            k(Key::Plus).cmd().hidden(),
            k(Key::Equals).cmd().shift().hidden(),
            k(Key::Plus).cmd().shift().hidden(),
        ]),
        cmd("view.ui_smaller", "Decrease UI Size", Misc, "make text and controls smaller").keys(&[k(Key::Minus).cmd()]),
        cmd("view.ui_reset", "Reset UI Size", Misc, "back to 100 %").keys(&[k(Key::Num0).cmd()]),
        cmd("palette.open", "Open Command Palette", Misc, "run any command by name")
            .keys(&[k(Key::K).cmd(), k(Key::P).cmd().shift()])
            .no_palette(),
        cmd("palette.reset_recent", "Reset Recent Commands List", Misc, "forget the palette's recent commands"),
        cmd("edit.undo", "Undo", Misc, "undo adding or deleting range measurements")
            .keys(&[k(Key::Z).cmd()])
            .no_palette(),
        cmd("edit.redo", "Redo", Misc, "redo adding or deleting range measurements")
            .keys(&[k(Key::Z).cmd().shift(), k(Key::Y).cmd()])
            .no_palette(),
        // ---- Data ----
        cmd("data.delete_off_screen", "Delete Data Off Screen", Data, "keep only the visible range"),
        cmd("data.delete_before_view", "Delete Data Before the View", Data, "delete what lies left of the view"),
        cmd("data.delete_after_view", "Delete Data After the View", Data, "delete what lies right of the view"),
        // ---- View History ----
        cmd("history.back", "Previous View", ViewHistory, "back to the previous zoom and position")
            .keys(&[k(Key::ArrowLeft).alt(), k(Key::OpenBracket).cmd().mac()]),
        cmd("history.forward", "Next View", ViewHistory, "forward again")
            .keys(&[k(Key::ArrowRight).alt(), k(Key::CloseBracket).cmd().mac()]),
    ];
    // Ctrl/Cmd+1…9: one hidden command each; the dialog shows
    // *Go to Session*'s label instead.
    const DIGITS: [Key; 9] =
        [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    const IDS: [&str; 9] = [
        "session.goto_1",
        "session.goto_2",
        "session.goto_3",
        "session.goto_4",
        "session.goto_5",
        "session.goto_6",
        "session.goto_7",
        "session.goto_8",
        "session.goto_9",
    ];
    for (key, id) in DIGITS.into_iter().zip(IDS) {
        v.push(cmd(id, "Go to Session", Category::Sessions, "").keys(&[k(key).cmd()]).no_palette().no_dialog());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_key_is_bound_once() {
        let r = registry();
        let mut ids: Vec<_> = r.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate command ids");
        // No two global commands share a binding.
        let mut seen: Vec<(Combo, &str)> = Vec::new();
        for c in r.iter().filter(|c| !c.pointer) {
            for combo in &c.keys {
                let clash = seen.iter().find(|(s, _)| s == combo);
                assert!(clash.is_none(), "{} and {} share {combo:?}", c.id, clash.map_or("", |x| x.1));
                seen.push((*combo, c.id));
            }
        }
    }

    #[test]
    fn combos_match_per_platform() {
        let save = k(Key::S).cmd();
        // What egui reports for ⌘ and Ctrl on macOS, and for Ctrl elsewhere.
        let mac_cmd = Modifiers { mac_cmd: true, command: true, ..Default::default() };
        let mac_ctrl = Modifiers { ctrl: true, ..Default::default() };
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        assert!(save.matches_key(Key::S, mac_cmd, true));
        assert!(!save.matches_key(Key::S, mac_ctrl, true), "Ctrl isn't ⌘ on macOS");
        assert!(save.matches_key(Key::S, ctrl, false));
        let tab = k(Key::Tab).ctrl();
        assert!(tab.matches_key(Key::Tab, mac_ctrl, true));
        assert!(!tab.matches_key(Key::Tab, mac_cmd, true));
        assert!(tab.matches_key(Key::Tab, ctrl, false));
        assert!(!k(Key::M).matches_key(Key::M, Modifiers::SHIFT, true));
        assert!(t("?").matches_text("?", Modifiers::SHIFT));
        assert_eq!(k(Key::S).cmd().shift().text(true), "⌘⇧S");
        assert_eq!(k(Key::S).cmd().shift().text(false), "Ctrl+Shift+S");
        assert_eq!(k(Key::Tab).ctrl().text(false), "Ctrl+Tab");
    }
}
