#![cfg(feature = "mock")]
//! Settings, device nicknames and the UI size keys.

use egui::{Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::settings::ThemePref;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::MockBackend;
use oxrok_ui::core::ManualClock;
use std::path::PathBuf;
use std::sync::Arc;

struct Rig {
    h: Harness<'static, LaApp>,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self::with_file(None)
    }

    /// With a settings file holding `content` (when given).
    fn with_file(content: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        if let Some(c) = content {
            std::fs::write(&path, c).unwrap();
        }
        let profiles = dir.path().join("probe-profiles");
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::seeded(1)),
                    clock: Arc::new(ManualClock::new()),
                    dialogs: Arc::new(oxrok_ui::app::dialogs::FixedDialogs(None)),
                    settings_path: Some(path.clone()),
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: Some(profiles.clone()),
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        Rig { h, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn open_demo(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.h.run_steps(4);
    }

    fn settings(&mut self) {
        self.h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        self.h.run();
        self.h.get_by_label("Search settings");
    }

    fn section(&self) -> String {
        let selected = self
            .h
            .query_all_by_role(egui::accesskit::Role::Tab)
            .find(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
            .expect("a selected section");
        selected.accesskit_node().label().unwrap_or_default()
    }

    fn pref(&self) -> ThemePref {
        self.h.state().settings().appearance.theme
    }
}

/// `Ctrl/Cmd+,` opens Appearance; Theme switches to Light and back
/// to System; UI scale goes to 1.2 and *Reset* restores 1.0.
#[test]
fn appearance() {
    let mut r = Rig::new();
    r.h.run();
    r.settings();
    assert_eq!(r.section(), "Appearance");
    r.h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Light").click();
    r.h.run();
    assert_eq!(r.pref(), ThemePref::Light);
    assert_eq!(r.h.state().theme(), oxrok_ui::ui::theme::ThemeKind::Light);
    r.h.snapshot("settings_appearance_light");
    r.h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "System").click();
    r.h.run();
    assert_eq!(r.pref(), ThemePref::System);

    r.h.get_by_label("Increase UI scale").click();
    r.h.run();
    r.h.get_by_label("Increase UI scale").click();
    r.h.run_steps(4);
    assert!((r.h.state().settings().appearance.ui_scale - 1.2).abs() < 1e-4);
    assert!((r.h.ctx.zoom_factor() - 1.2).abs() < 1e-4, "text grows");
    r.h.get_by_label("Reset").click();
    r.h.run_steps(4);
    assert!((r.h.ctx.zoom_factor() - 1.0).abs() < 1e-4);
    // Saved at once (debounced).
    r.h.key_press(Key::Escape);
    r.h.run();
    assert!(r.h.query_by_label("Search settings").is_none(), "Escape closes");
}

/// Search selects Files & Folders; the row menu and the Time Zero
/// menu open their sections.
#[test]
fn search_and_entry_points() {
    let mut r = Rig::new();
    r.open_demo();
    r.settings();
    r.h.get_by_label("Search settings").focus();
    r.h.run();
    r.h.get_by_label("Search settings").type_text("export folder");
    r.h.run();
    assert_eq!(r.section(), "Files & Folders");
    assert!(r.h.query_by_label("Appearance").is_none(), "only matching sections are listed");
    r.h.get_by_label("Search settings").type_text("zzz");
    r.h.run();
    r.h.get_by_label("No settings match");
    r.h.key_press(Key::Escape);
    r.h.run();

    // The row context menu's Settings… opens Waveform.
    r.h.get_by_role_and_label(egui::accesskit::Role::Label, "D0").click_secondary();
    r.h.run();
    r.h.get_by_label("Settings…").click();
    r.h.run();
    assert_eq!(r.section(), "Waveform");
    r.h.key_press(Key::Escape);
    r.h.run();

    // The Time Zero menu's Time Settings… opens Time Labels (it needs data).
    r.h.key_press(Key::Space);
    // Wait for the backend to report Recording: under load, five frames can
    // pass while still Starting, and the second Space would cancel instead
    // of stop.
    let started = std::time::Instant::now();
    while r.h.state().session().unwrap().state != oxrok_ui::core::capture::CaptureState::Recording {
        assert!(started.elapsed().as_secs() < 60, "capture did not start");
        r.h.step();
    }
    r.h.key_press(Key::Space);
    let t0 = std::time::Instant::now();
    loop {
        let state = r.h.state().session().unwrap().state;
        if state == oxrok_ui::core::capture::CaptureState::Finished {
            break;
        }
        assert!(t0.elapsed().as_secs() < 60, "capture never finished, last state {state:?}");
        r.h.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    r.h.run_steps(3);
    let d0 = r.h.get_by_role_and_label(egui::accesskit::Role::Label, "D0").rect();
    let ruler = egui::pos2(d0.right() + 300.0, d0.top() - 20.0);
    r.h.hover_at(ruler);
    r.h.run();
    for pressed in [true, false] {
        r.h.event(egui::Event::PointerButton {
            pos: ruler,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    r.h.run();
    r.h.get_by_label("Time Settings…").click();
    r.h.run();
    assert_eq!(r.section(), "Time Labels");
}

/// *Reset all settings…* confirms, then resets the theme and scale;
/// nicknames and probe profiles stay.
#[test]
fn reset_all() {
    let mut r = Rig::new();
    r.h.run();
    std::fs::create_dir_all(r.path("probe-profiles")).unwrap();
    std::fs::write(r.path("probe-profiles/mine.json"), "{}").unwrap();
    {
        let s = r.h.state_mut().settings_mut();
        s.appearance.theme = ThemePref::Light;
        s.appearance.ui_scale = 1.4;
        s.device_nicknames.insert("demo".into(), "Bench".into());
    }
    r.settings();
    r.h.get_by_label("Reset all settings…").click();
    r.h.run();
    r.h.get_by_label("Reset all settings to defaults? Your capture files and probe profiles are not affected.");
    // The confirmation's Reset (below the UI scale's Reset link).
    r.h.query_all_by_label("Reset").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    r.h.run_steps(4);
    let s = r.h.state().settings();
    assert_eq!(s.appearance.theme, ThemePref::System);
    assert_eq!(s.appearance.ui_scale, 1.0);
    assert_eq!(s.device_nicknames["demo"], "Bench");
    assert!(r.path("probe-profiles/mine.json").exists());
    r.h.get_by_label("Search settings");
}

/// A `schema_version` 1 or unparsable file is backed up, and the app
/// starts with defaults and a warning.
#[test]
fn old_settings_are_reset() {
    for content in [r#"{"schema_version": 1, "theme": "light"}"#, "{ not json"] {
        let mut r = Rig::with_file(Some(content));
        r.h.run();
        assert_eq!(r.pref(), ThemePref::System);
        let toast = r.h.state().toasts().iter().find(|t| t.id == "settings-reset").expect("warning toast");
        assert!(toast.body.starts_with("Settings were reset."), "{}", toast.body);
        assert!(toast.actions.iter().any(|a| a.starts_with("Show in")));
        let backups: Vec<_> = std::fs::read_dir(r.dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.bak-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), content);
    }
}

/// A nickname shows wherever the device name does, and
/// *Use default name* removes it.
#[test]
fn device_nickname() {
    let mut r = Rig::new();
    r.open_demo();
    // The Inspector starts closed: open it on the Device tab.
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    r.h.run_steps(3);
    r.h.get_by_label("Device Information").click();
    r.h.run();
    let field = r.h.get_by_label("Device name");
    field.focus();
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.get_by_label("Device name").type_text("Bench LA");
    r.h.key_press(Key::Enter);
    r.h.run_steps(3);
    let id = r.h.state().session().unwrap().device.id.0.clone();
    assert_eq!(r.h.state().settings().device_nicknames.get(&id).map(String::as_str), Some("Bench LA"));
    r.h.get_by_label("Use default name").click();
    r.h.run_steps(3);
    assert!(r.h.state().settings().device_nicknames.is_empty());
    r.h.key_press(Key::Escape);
    r.h.run();
    // Shown in the window title, the panel header and the Sessions popover.
    r.h.state_mut().settings_mut().device_nicknames.insert(id, "Bench LA".into());
    r.h.run_steps(3);
    assert!(r.h.state().window_title().contains("[Bench LA - Demo]"), "{}", r.h.state().window_title());
    assert!(r.h.query_all_by_label("[DEMO] Bench LA").count() >= 1, "the panel header");
}

/// `Ctrl/Cmd+=`, `-` and `0` change the UI size, within limits.
#[test]
fn ui_size_keys() {
    let mut r = Rig::new();
    r.h.run();
    let scale = |r: &Rig| r.h.state().settings().appearance.ui_scale;
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    r.h.run();
    assert!((scale(&r) - 1.1).abs() < 1e-4);
    for _ in 0..20 {
        r.h.key_press_modifiers(Modifiers::COMMAND, Key::Minus);
        r.h.run();
    }
    assert!((scale(&r) - 0.3).abs() < 1e-4, "not below 0.3");
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    r.h.run();
    assert_eq!(scale(&r), 1.0);
}
