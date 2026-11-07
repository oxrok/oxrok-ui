#![cfg(feature = "mock")]
//! Probe profiles: matching, display, and the editor.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::profiles::ProfileSource;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::MockBackend;
use oxrok_ui::core::ManualClock;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A user profile for "Mock 8 digital": two marked groups, D6/D7 unlisted.
const MOCK8: &str = r##"{
  "schema_version": 1,
  "name": "Mock 8 leads",
  "match": { "driver": "demo", "model": "Mock 8 digital" },
  "groups": [
    { "name": "Bus A", "marker": "blue", "channels": ["D0", "D1", "D2", "D3"],
      "wires": ["black", "brown", "red", "orange"] },
    { "name": "Bus B", "marker": "yellow",
      "channels": [ { "channel": "D4", "wire": "#336699", "label": "SDA" },
                    { "channel": "D5", "wire": "white", "marker": "green" } ] }
  ]
}"##;

fn harness(dir: &Path, builtins: Vec<String>) -> Harness<'static, LaApp> {
    let dir = dir.to_path_buf();
    Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(MockBackend::seeded(1)),
                clock: Arc::new(ManualClock::new()),
                dialogs: Arc::new(FixedDialogs(None)),
                settings_path: None,
                animation_speed: 0.0,
                scan_sync: true,
                profiles_dir: Some(dir),
                extra_builtin_profiles: builtins,
            },
        )
    })
}

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        assert!(t0.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

fn open(h: &mut Harness<'_, LaApp>, device: &str) {
    h.run();
    h.get_by_label(device).click();
    until(h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    // The Inspector starts closed: open it on the Device tab.
    h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    h.run_steps(3);
}

/// Opens the editor and waits for the modal to settle: it slides into place
/// over a few frames while its content sizes itself.
fn open_editor(h: &mut Harness<'_, LaApp>) {
    h.get_all_by_label("Probe Colors…").next().unwrap().click();
    h.run_steps(10);
}

fn probe(h: &Harness<'_, LaApp>) -> Option<(oxrok_ui::core::probe::ProbeProfile, ProfileSource)> {
    h.state().session().unwrap().probe.clone()
}

fn type_into(h: &mut Harness<'_, LaApp>, label: &str, text: &str) {
    h.get_by_role_and_label(egui::Role::TextInput, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(text.into()));
    h.run();
}

#[test]
fn user_profile_applies_groups_markers_and_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mock8.json"), MOCK8).unwrap();
    let mut h = harness(dir.path(), vec![]);
    open(&mut h, "Mock 8 digital");
    let (p, src) = probe(&h).expect("profile");
    assert_eq!(src, ProfileSource::User);
    assert_eq!(p.channel("D1").unwrap().wire.as_ref().unwrap().spec, "brown");
    // Grid sections, unlisted channels under Other.
    for heading in ["Bus A", "Bus B", "Other"] {
        h.get_by_label(heading);
    }
    // The profile label becomes the default row name.
    let s = h.state().session().unwrap();
    assert_eq!(s.row(oxrok_ui::core::ChannelId::digital(4)).unwrap().display(), "D4 SDA");
    h.snapshot("probe_groups_dark");
}

#[test]
fn editor_saves_a_wire_color_and_applies_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mock8.json"), MOCK8).unwrap();
    let mut h = harness(dir.path(), vec![]);
    open(&mut h, "Mock 8 digital");
    open_editor(&mut h);
    h.get_by_label("D1 wire").click();
    h.run();
    h.get_by_label("green").click();
    h.run();
    h.remove_cursor();
    h.run();
    h.snapshot("probe_editor_dark");
    h.get_by_label("Save").click();
    h.run();
    let (p, _) = probe(&h).unwrap();
    assert_eq!(p.channel("D1").unwrap().wire.as_ref().unwrap().spec, "green");
    let saved = std::fs::read_to_string(dir.path().join("mock8.json")).unwrap();
    assert!(saved.contains("\"channel\": \"D1\",\n          \"wire\": \"green\""), "{saved}");
    assert!(h.state().toasts().iter().any(|t| t.body == "Probe colors saved"));
}

#[test]
fn hex_color_and_marker_override() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mock8.json"), MOCK8).unwrap();
    let mut h = harness(dir.path(), vec![]);
    open(&mut h, "Mock 8 digital");
    open_editor(&mut h);
    h.get_by_label("D0 marker").click();
    h.run();
    type_into(&mut h, "Hex color", "#A1B2C3");
    h.get_by_label("Apply").click();
    h.run();
    h.get_by_label("Save").click();
    h.run();
    let (p, _) = probe(&h).unwrap();
    let d0 = p.channel("D0").unwrap();
    assert_eq!(p.marker(d0).unwrap().rgb, [0xA1, 0xB2, 0xC3], "own marker beats the group's blue");
}

#[test]
fn create_a_profile_for_a_device_without_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path(), vec![]);
    open(&mut h, "Mock 16 digital + 4 analog");
    assert!(probe(&h).is_none());
    open_editor(&mut h);
    type_into(&mut h, "Group 1 name", "Low byte");
    h.get_by_label("Save").click();
    h.run();
    let (p, src) = probe(&h).expect("created");
    assert_eq!(src, ProfileSource::User);
    let names: Vec<&str> = p.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Low byte", "D8–D15", "A0–A3"]);
    let file = dir.path().join("demo-mock-16-digital---4-analog.json");
    assert!(file.exists(), "{:?}", std::fs::read_dir(dir.path()).unwrap().collect::<Vec<_>>());
}

#[test]
fn editing_a_builtin_makes_a_user_copy_that_can_be_reset() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path(), vec![MOCK8.to_owned()]);
    open(&mut h, "Mock 8 digital");
    assert_eq!(probe(&h).unwrap().1, ProfileSource::BuiltIn);
    assert!(h.query_by_label("Reset to built-in").is_none());

    open_editor(&mut h);
    type_into(&mut h, "Group 2 name", "I2C");
    h.get_by_label("Save").click();
    h.run();
    let (p, src) = probe(&h).unwrap();
    assert_eq!(src, ProfileSource::UserOverride);
    assert_eq!(p.groups[1].name, "I2C");

    open_editor(&mut h);
    h.get_by_label("Reset to built-in").click();
    h.run();
    let (p, src) = probe(&h).unwrap();
    assert_eq!(src, ProfileSource::BuiltIn);
    assert_eq!(p.groups[1].name, "Bus B");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "user copy deleted");
}

#[test]
fn broken_user_profile_warns() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), r#"{"schema_version":1,"name":"x","match":{"driver":"mock"},"groups":[{"name":"g","channels":["D0"],"wires":["plaid"]}]}"#).unwrap();
    let mut h = harness(dir.path(), vec![]);
    h.run();
    let t = h.state().toasts().iter().find(|t| t.heading.as_deref() == Some("Probe profile skipped")).expect("warning");
    assert!(t.body.contains("broken.json") && t.body.contains("plaid"), "{}", t.body);
}
