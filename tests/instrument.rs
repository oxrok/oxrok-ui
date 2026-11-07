#![cfg(feature = "mock")]
//! Logic threshold, device options,
//! Command line.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::ScriptedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{ConfValue, MockBackend, MockOptions, OptionScope, ThresholdChoice};
use oxrok_ui::core::ManualClock;
use oxrok_ui::core::capture::CaptureState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Rig {
    h: Harness<'static, LaApp>,
    dialogs: Arc<ScriptedDialogs>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let dialogs = Arc::new(ScriptedDialogs { folder: Some(dir.path().to_owned()), ..Default::default() });
        let d = dialogs.clone();
        let h = Harness::builder().with_size([1280.0, 1000.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(MockOptions { thresholds: true, ..MockOptions::default() })),
                    clock: Arc::new(ManualClock::new()),
                    dialogs: d,
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        Rig { h, dialogs, _dir: dir }
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(2);
    }

    fn open(&mut self, device: &str) {
        self.h.run();
        self.h.get_by_label(device).click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
        // The Inspector starts closed: open it on the Device tab.
        self.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
        self.h.run_steps(3);
    }

    /// Records briefly (Continuous: start, then stop).
    fn record(&mut self) {
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }

    fn click(&mut self, role: Role, label: &str) {
        self.h.get_by_role_and_label(role, label).scroll_to_me();
        // Let the scroll settle before clicking.
        self.h.run_steps(20);
        self.h.get_by_role_and_label(role, label).click();
        self.h.run_steps(3);
    }

    fn choose(&mut self, combo: &str, option: &str) {
        self.click(Role::ComboBox, combo);
        self.h.get_by_role_and_label(Role::ListBoxOption, option).click();
        self.h.run_steps(2);
    }

    fn set_text(&mut self, label: &str, text: &str) {
        self.h.get_by_role_and_label(Role::TextInput, label).scroll_to_me();
        self.h.run_steps(20);
        self.h.get_by_role_and_label(Role::TextInput, label).focus();
        self.h.run();
        self.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        self.h.key_press(Key::Backspace);
        self.h.get_by_role_and_label(Role::TextInput, label).type_text(text);
        self.h.key_press(Key::Enter);
        self.h.run_steps(2);
    }

    fn has_toast(&self, text: &str) -> bool {
        self.h.state().toasts().iter().any(|t| t.body.contains(text))
    }

    fn copied(&self) -> Option<String> {
        self.h
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .next_back()
    }

    fn cli(&self) -> String {
        self.h.state().session().unwrap().cli_command().one_line()
    }
}

/// The selector lists the mock's thresholds; Custom… reveals the
/// voltage input; the capture's snapshot carries the value.
#[test]
fn logic_threshold() {
    let mut r = Rig::new();
    r.open("Mock 16 digital + 4 analog");
    let combo = r.h.get_by_role_and_label(Role::ComboBox, "Logic threshold");
    assert_eq!(combo.accesskit_node().value().as_deref(), Some("3.3+ Volts"), "the device's current threshold");
    r.click(Role::ComboBox, "Logic threshold");
    for o in ["1.8 Volts", "3.3+ Volts", "Custom…"] {
        r.h.get_by_role_and_label(Role::ListBoxOption, o);
    }
    r.h.get_by_role_and_label(Role::ListBoxOption, "Custom…").click();
    r.h.run_steps(2);
    r.set_text("Custom threshold", "1.234");
    let s = r.h.state().session().unwrap();
    assert_eq!(s.settings.logic_threshold, Some(ThresholdChoice::Custom(1.23)), "0.01 V resolution");
    r.record();
    let snap = r.h.state().session().unwrap().snapshot.clone().unwrap();
    assert_eq!(snap.logic_threshold, Some(ThresholdChoice::Custom(1.23)));
    assert!(r.cli().contains("logic_threshold_custom=1.23"), "{}", r.cli());

    r.choose("Logic threshold", "1.8 Volts");
    assert_eq!(r.h.state().session().unwrap().settings.logic_threshold, Some(ThresholdChoice::Preset("1.8".into())));
    assert!(r.h.query_by_role_and_label(Role::TextInput, "Custom threshold").is_none());
}

/// The mock's options, the header count, the snapshot, a refused
/// value, *Reset to defaults*, and the opened file's disabled panel.
#[test]
fn device_options() {
    let mut r = Rig::new();
    r.open("Mock 16 digital + 4 analog");
    r.click(Role::Button, "Device options");
    // Device scope, plus the Analog group.
    r.h.get_by_role_and_label(Role::RadioButton, "Device");
    r.h.get_by_role_and_label(Role::RadioButton, "Analog");
    r.h.get_by_role_and_label(Role::Switch, "External clock");
    r.h.get_by_role_and_label(Role::ComboBox, "Pattern mode");
    r.h.get_by_role_and_label(Role::TextInput, "Average samples");
    r.h.get_by_label("active");
    r.click(Role::Switch, "External clock");
    r.h.run_steps(10);
    r.choose("Pattern mode", "Sine");
    r.h.get_by_role_and_label(Role::Button, "Device options (2)").scroll_to_me();
    r.h.run();
    r.h.snapshot("device_options");

    // A range input clamps on commit.
    r.set_text("Average samples", "5000");
    let dev = |r: &Rig| r.h.state().session().unwrap().settings.device_options.get(&OptionScope::Device).cloned();
    assert_eq!(dev(&r).unwrap()["avg_samples"], ConfValue::UInt(1024));
    r.record();
    let snap = r.h.state().session().unwrap().snapshot.clone().unwrap();
    let o = &snap.device_options[&OptionScope::Device];
    assert_eq!(o["external_clock"], ConfValue::Bool(true));
    assert_eq!(o["pattern"], ConfValue::Str("Sine".into()));

    // The driver refuses a value: it reverts with a toast.
    r.h.state_mut()
        .session_mut()
        .unwrap()
        .settings
        .device_options
        .get_mut(&OptionScope::Device)
        .unwrap()
        .insert("avg_samples".into(), ConfValue::UInt(5000));
    r.record();
    assert!(r.has_toast("didn't accept Average samples = 5000"));
    assert_eq!(dev(&r).unwrap().get("avg_samples"), Some(&ConfValue::UInt(1024)), "the value the device kept");

    // The file keeps what the device accepted, shown read-only.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    r.until("saved", |a| !a.session().unwrap().is_unsaved());
    let path: PathBuf = r.h.state().session().unwrap().file.clone().unwrap();
    *r.dialogs.open.lock() = vec![path.clone()];
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    r.h.run_steps(3);
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    r.until("opened", move |a| a.session().is_some_and(|s| s.opened_from.as_deref() == Some(&*path)));
    let opened = r.h.state().session().unwrap().settings.device_options.clone();
    assert_eq!(opened[&OptionScope::Device]["pattern"], ConfValue::Str("Sine".into()));
    assert!(r.h.query_by_role_and_label(Role::Switch, "External clock").is_none(), "no live controls");
    r.h.get_by_label("Sine");
    r.h.get_by_label("Pattern");
}

/// *Reset to defaults*.
#[test]
fn reset_to_defaults() {
    let mut r = Rig::new();
    r.open("Mock 16 digital + 4 analog");
    r.click(Role::Button, "Device options");
    r.click(Role::Switch, "External clock");
    r.h.run_steps(10);
    // Collapsing keeps the count in the header.
    r.click(Role::Button, "Device options (1)");
    r.h.run_steps(2);
    assert!(r.h.query_by_role_and_label(Role::Switch, "External clock").is_none());
    r.click(Role::Button, "Device options (1)");
    r.h.run_steps(2);
    r.click(Role::Button, "Reset to defaults");
    r.h.run_steps(2);
    assert!(r.h.state().session().unwrap().settings.device_options.is_empty());
    r.h.get_by_role_and_label(Role::Button, "Device options");
}

/// The command line for the defaults, Trigger mode, a renamed row,
/// and *Copy*.
#[test]
fn command_line() {
    let mut r = Rig::new();
    r.open("Mock 8 digital");
    r.click(Role::Button, "Command line");
    let expected = r#"oxrok -d demo -C D0,D1 -c samplerate=8m --continuous -o "Session 0.sr""#;
    assert_eq!(r.cli(), expected);
    r.h.get_by_label(expected).scroll_to_me();
    r.h.run_steps(20);
    r.h.snapshot("command_line");

    {
        let s = r.h.state_mut().session_mut().unwrap();
        s.settings.mode = oxrok_ui::core::capture::CaptureMode::Triggered;
        s.settings.trigger.channel = Some(oxrok_ui::core::ChannelId::digital(1));
        s.settings.trigger.event = oxrok_ui::core::trigger::TriggerEvent::Falling;
        s.settings.trigger.post_trigger_seconds = 0.5;
    }
    r.h.run();
    assert_eq!(r.cli(), r#"oxrok -d demo -C D0,D1 -c samplerate=8m -t D1=f --time 500 -o "Session 0.sr""#);
    r.h.state_mut().session_mut().unwrap().rename_row(oxrok_ui::core::ChannelId::digital(0), "SCL");
    r.h.run();
    assert!(r.cli().contains("-C D0=SCL,D1 "), "{}", r.cli());
    // The trigger names its channel by the alias -C gave it.
    r.h.state_mut().session_mut().unwrap().rename_row(oxrok_ui::core::ChannelId::digital(1), "SDA");
    r.h.run();
    assert!(r.cli().contains("-C D0=SCL,D1=SDA ") && r.cli().contains("-t SDA=f "), "{}", r.cli());

    r.h.get_by_role_and_label(Role::Button, "Copy").scroll_to_me();
    r.h.run_steps(20);
    r.h.get_by_role_and_label(Role::Button, "Copy").click();
    r.h.step();
    let copied = r.copied().expect("copied");
    assert_eq!(copied, r.cli());
    assert_eq!(copied.lines().count(), 1);
    // The same from the palette.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    r.h.run();
    r.h.get_by_label("Search commands").type_text("oxrok");
    r.h.run();
    // `key_press` runs its own frames; send the press and step once.
    r.h.event(egui::Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    r.h.step();
    assert_eq!(r.copied().as_deref(), Some(r.cli().as_str()));
}
