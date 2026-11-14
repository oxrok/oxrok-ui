#![cfg(feature = "mock")]
//! Commands: the registry's keys, the command palette and the menu bar.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::ManualClock;
use oxrok_ui::core::capture::CaptureState;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

impl Rig {
    fn new() -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: Arc::new(oxrok_ui::app::dialogs::FixedDialogs(None)),
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        Rig { h, clock }
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(2);
    }

    fn open_demo(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    }

    fn record(&mut self, ms: u64) {
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.clock.advance(Duration::from_millis(ms));
        let want = ms * RATE / 1000;
        self.until("data", move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }

    fn palette(&mut self) {
        self.h.key_press_modifiers(Modifiers::COMMAND, Key::K);
        self.h.run();
        self.h.get_by_label("Search commands");
    }

    fn type_query(&mut self, q: &str) {
        self.h.get_by_label("Search commands").type_text(q);
        self.h.run();
    }

    /// The palette's rows (buttons below the field), in order.
    fn rows(&self) -> Vec<String> {
        let field = self.h.get_by_label("Search commands").rect();
        let mut v: Vec<(f32, String)> = self
            .h
            .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
            .filter(|n| n.rect().top() > field.bottom() && (n.rect().width() - 560.0).abs() < 1.0)
            .map(|n| (n.rect().top(), n.accesskit_node().label().unwrap_or_default()))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|x| x.1).collect()
    }

    fn time_scale(&self) -> f64 {
        self.h.state().session().unwrap().view.state().time_scale()
    }
}

/// `?` lists every category; Escape closes it.
#[test]
fn shortcuts_dialog_lists_all_categories() {
    let mut r = Rig::new();
    r.h.run();
    r.h.event(egui::Event::Text("?".into()));
    r.h.run();
    for c in
        ["Capture", "Signal Navigation", "Analog Signals", "Sessions", "Markers", "Measurements", "Inspector", "Misc"]
    {
        r.h.get_by_role_and_label(egui::accesskit::Role::Label, c);
    }
    r.h.get_by_label("Previous / Next View");
    r.h.key_press(Key::Escape);
    r.h.run();
    assert!(r.h.query_by_label("Keyboard Shortcuts").is_none());
}

/// `zoo` finds X Zoom In first, Enter zooms in, and it's recent next time.
#[test]
fn palette_runs_and_remembers() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(20);
    let before = r.time_scale();
    r.palette();
    r.type_query("zoo");
    assert_eq!(r.rows().first().map(String::as_str), Some("X Zoom In"), "{:?}", r.rows());
    r.h.key_press(Key::Enter);
    r.h.run();
    assert!(r.h.query_by_label("Search commands").is_none(), "running closes the palette");
    assert!((r.time_scale() - before / 2.0).abs() < before * 1e-6, "zoomed in");
    r.palette();
    r.h.get_by_label("Recent");
    assert_eq!(r.rows().first().map(String::as_str), Some("X Zoom In"));
    assert_eq!(r.h.state().settings().history.recent_commands, ["nav.zoom_in"]);
}

/// `Alt+3` opens the Inspector on the Analyzers tab; again it stays
/// open on that tab (the tab keys open, they don't toggle).
#[test]
fn alt_opens_panels() {
    let mut r = Rig::new();
    r.open_demo();
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num3);
    r.h.run();
    assert!(r.h.state().settings().inspector.open);
    assert_eq!(r.h.state().settings().inspector.tab.as_str(), "analyzers");
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num3);
    r.h.run();
    assert!(r.h.state().settings().inspector.open, "the tab key opens, it doesn't toggle");
    assert_eq!(r.h.state().settings().inspector.tab.as_str(), "analyzers");
}

/// *Detach Inspector* moves the Inspector out of the
/// main window (here, a floating window: kittest embeds viewports); its tab
/// keys still work, and *Attach Inspector* docks it again at its old width.
#[test]
fn detach_inspector() {
    let mut r = Rig::new();
    r.open_demo();
    r.h.key_press_modifiers(Modifiers::ALT, Key::I);
    r.h.run();
    let width = r.h.state().settings().inspector.width;
    let docked = r.h.get_by_label("Device").rect();
    r.h.get_by_label("Detach Inspector").click();
    r.h.run();
    let s = r.h.state().settings();
    assert!(s.inspector.open && s.inspector.detached);
    assert!(r.h.query_by_label("Detach Inspector").is_none());
    assert_ne!(r.h.get_by_label("Device").rect(), docked, "the tabs moved out of the left panel");
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num3);
    r.h.run();
    assert_eq!(r.h.state().settings().inspector.tab.as_str(), "analyzers");
    assert!(r.h.state().settings().inspector.detached, "a tab key keeps it detached");
    // Hiding keeps it detached; showing brings the window back.
    r.h.key_press_modifiers(Modifiers::ALT, Key::I);
    r.h.run();
    assert!(r.h.query_by_label("Attach Inspector").is_none());
    assert!(r.h.state().settings().inspector.detached);
    r.h.key_press_modifiers(Modifiers::ALT, Key::I);
    r.h.run();
    r.h.get_by_label("Attach Inspector").click();
    r.h.run();
    let s = r.h.state().settings();
    assert!(s.inspector.open && !s.inspector.detached);
    assert_eq!(s.inspector.width, width);
    assert_eq!(r.h.get_by_label("Device").rect(), docked, "docked where it was");
    // Alt+Shift+I toggles too.
    r.h.key_press_modifiers(Modifiers::ALT | Modifiers::SHIFT, Key::I);
    r.h.run();
    assert!(r.h.state().settings().inspector.detached);
}

/// Typing `r` in a text field doesn't start a capture.
#[test]
fn no_shortcuts_while_typing() {
    let mut r = Rig::new();
    r.open_demo();
    r.palette();
    r.type_query("r m");
    r.h.key_press(Key::Space);
    r.h.run();
    let s = r.h.state().session().unwrap();
    assert_eq!(s.state, CaptureState::Empty);
    assert!(!r.h.state().settings().inspector.open, "typing in the search field shouldn't open the Inspector");
}

/// Headers with an empty query; "theme dark" runs the sub-command;
/// choosing Theme shows the breadcrumb and Backspace returns.
#[test]
fn palette_headers_and_sub_commands() {
    let mut r = Rig::new();
    r.open_demo();
    // A recent command first, so both headers show.
    r.palette();
    r.type_query("traces color");
    r.h.key_press(Key::Enter);
    r.h.run();
    r.palette();
    r.h.get_by_label("Recent");
    r.h.get_by_label("All commands");
    assert_eq!(r.rows().first().map(String::as_str), Some("Toggle Traces Color: neutral"));
    r.h.snapshot("palette_dark");

    r.type_query("theme light");
    assert_eq!(r.rows().first().map(String::as_str), Some("Theme: Light"), "{:?}", r.rows());
    r.h.key_press(Key::Enter);
    r.h.run();
    assert_eq!(r.h.state().theme(), oxrok_ui::ui::theme::ThemeKind::Light);

    r.palette();
    r.type_query("theme");
    r.h.key_press(Key::Enter);
    r.h.run();
    r.h.get_by_label("‹ Theme");
    assert_eq!(r.rows(), ["‹ Theme", "Light", "Dark", "System"]);
    r.h.key_press(Key::Backspace);
    r.h.run();
    assert!(r.h.query_by_label("‹ Theme").is_none(), "back to the top level");
    r.h.get_by_label("All commands");
    r.h.key_press(Key::Escape);
    r.h.run();
    assert!(r.h.query_by_label("Search commands").is_none());
}

/// Commands that don't apply aren't listed; no match says so.
#[test]
fn palette_hides_what_does_not_apply() {
    let mut r = Rig::new();
    r.open_demo();
    r.palette();
    r.type_query("save capture");
    assert!(!r.rows().iter().any(|x| x.starts_with("Save Capture")), "nothing to save yet");
    r.h.get_by_label("Search commands").type_text("zzzz");
    r.h.run();
    r.h.get_by_label("No matching commands");
}

/// *Go to Session* lists the sessions.
#[test]
fn palette_goes_to_a_session() {
    let mut r = Rig::new();
    r.open_demo();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    r.until("second session", |a| a.session_count() == 2 && a.session().unwrap().caps.is_some());
    r.palette();
    r.type_query("go to session session 0");
    assert_eq!(r.rows().first().map(String::as_str), Some("Go to Session: Session 0"), "{:?}", r.rows());
    r.h.key_press(Key::Enter);
    r.h.run();
    assert_eq!(r.h.state().session().unwrap().name, "Session 0");
}

/// The menus run commands, show shortcuts and explain disabled items.
#[test]
fn menu_bar() {
    let mut r = Rig::new();
    r.open_demo();
    r.h.get_by_label("File").click();
    r.h.run();
    r.h.get_by_label("New Session");
    let save = r.h.get_by_label("Save Capture");
    assert!(save.accesskit_node().is_disabled());
    save.hover();
    r.h.run_steps(40);
    r.h.get_by_label("Record a capture first.");
    r.h.key_press(Key::Escape);
    r.h.run();

    // Capture → Start Recording reflects the state.
    r.h.get_by_label("Capture").click();
    r.h.run();
    r.h.get_by_label("Start Recording").click();
    r.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
    r.h.get_by_label("Capture").click();
    r.h.run_steps(3);
    r.h.get_by_label("Stop Recording");
    r.h.key_press(Key::Escape);
    r.clock.advance(Duration::from_millis(10));
    r.until("data", |a| a.session().unwrap().has_data());
    r.h.key_press(Key::Space);
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);

    // View → Theme ▸ Light.
    r.h.get_by_label("View").click();
    r.h.run();
    r.h.get_by_label("Theme").hover();
    r.h.run();
    r.h.snapshot("menu_view_dark");
    r.h.get_by_label("Light").click();
    r.h.run();
    assert_eq!(r.h.state().theme(), oxrok_ui::ui::theme::ThemeKind::Light);

    // View → Notes is checkable and opens the Notes bottom panel.
    r.h.get_by_label("View").click();
    r.h.run();
    r.h.get_by_role_and_label(egui::accesskit::Role::MenuItemCheckBox, "Notes").click();
    r.h.run();
    assert!(r.h.state().notes_panel_open());
}
