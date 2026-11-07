#![cfg(feature = "mock")]
//! Shell tests: start screen, panels, title (03).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::LaApp;
use oxrok_ui::backend::MockBackend;

fn harness() -> Harness<'static, LaApp> {
    Harness::builder().with_size([1280.0, 800.0]).build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)))
}

#[test]
fn start_screen_cards() {
    let mut h = harness();
    h.run();
    // One list of what was found, demo devices like any other.
    h.get_by_label("Devices");
    h.get_by_label("Mock 8 digital");
    h.get_by_label("Mock 16 digital + 4 analog");
    assert!(h.query_by_label("Try a demo device").is_none());
    // Only demo devices → the Devices menu says Disconnected.
    h.get_by_label("Disconnected");
    assert_eq!(h.state().devices().unwrap().len(), 2);
}

/// The Devices menu lists the mock devices with *New Session*
/// buttons; the active session's device has the check badge.
#[test]
fn devices_menu() {
    let mut h = harness();
    h.run();
    h.get_by_label("Disconnected").click();
    h.run();
    h.get_by_label("DEMO DEVICES");
    h.get_by_label("[DEMO] Mock 8 digital");
    assert_eq!(h.get_all_by_label("New Session").count(), 2);
    assert!(h.query_by_label("Active session's device").is_none(), "no session yet");
    // New Session on the second device: it becomes the active one.
    h.get_all_by_label("New Session").nth(1).unwrap().click();
    h.run_steps(4);
    assert_eq!(h.state().session().unwrap().device.model, "Mock 16 digital + 4 analog");
    h.get_by_label("Disconnected").click();
    h.run();
    let badge = h.get_by_label("Active session's device").rect();
    // The popover's row (the panel header shows the name too).
    let near = h
        .get_all_by_label("[DEMO] Mock 16 digital + 4 analog")
        .any(|n| (n.rect().center().y - badge.center().y).abs() < 20.0 && n.rect().left() > badge.right());
    assert!(near, "the badge is on that device's row");
}

#[test]
fn f12_toggles_gallery() {
    let mut h = harness();
    h.run();
    h.key_press(egui::Key::F12);
    h.run();
    h.get_by_label("Save");
}

#[test]
fn start_screen_snapshot() {
    let mut h = harness();
    h.run();
    h.snapshot("start_screen_dark");
}

/// The start screen lists the five newest files. A
/// short window scrolls the page instead of dropping any.
#[test]
fn recent_files_on_the_start_screen() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..5)
        .map(|i| {
            let p = dir.path().join(format!("capture {i}.oxrok"));
            std::fs::write(&p, b"x").unwrap();
            p
        })
        .collect();
    for size in [[1280.0, 800.0], [1000.0, 560.0], [1000.0, 640.0]] {
        let mut h =
            Harness::builder().with_size(size).build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)));
        h.state_mut().settings_mut().files.recent_files = files.clone();
        h.run();
        let shown: Vec<_> = files
            .iter()
            .filter_map(|p| h.query_by_label(&p.file_name().unwrap().to_string_lossy()).map(|n| n.rect()))
            .collect();
        assert_eq!(shown.len(), 5, "at {size:?}");
    }
}
