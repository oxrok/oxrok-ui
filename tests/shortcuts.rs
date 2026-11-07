#![cfg(feature = "mock")]
//! Keyboard Shortcuts dialog.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::LaApp;
use oxrok_ui::backend::MockBackend;

fn harness() -> Harness<'static, LaApp> {
    Harness::builder().with_size([1280.0, 800.0]).build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)))
}

fn type_question_mark(h: &mut Harness<'static, LaApp>) {
    h.event(egui::Event::Text("?".into()));
    h.run();
}

#[test]
fn question_mark_toggles_the_dialog() {
    let mut h = harness();
    h.run();
    assert!(h.query_by_label("Keyboard Shortcuts").is_none());
    type_question_mark(&mut h);
    h.get_by_label("Keyboard Shortcuts");
    h.get_by_label("Open Command Palette");
    h.get_by_label("Start/Stop Recording");
    type_question_mark(&mut h);
    assert!(h.query_by_label("Keyboard Shortcuts").is_none());
}

#[test]
fn escape_and_close_dismiss_the_dialog() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("Keyboard Shortcuts").is_none());
    type_question_mark(&mut h);
    h.get_by_label("Close").click();
    h.run();
    assert!(h.query_by_label("Keyboard Shortcuts").is_none());
}

/// Other shortcuts don't fire behind the dialog.
#[test]
fn shortcuts_are_blocked_while_open() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run();
    assert!(h.query_by_label("Search commands").is_none(), "no palette behind the dialog");
}

fn search(h: &mut Harness<'static, LaApp>, text: &str) {
    h.get_by_label("Search shortcuts").type_text(text);
    h.run();
}

#[test]
fn search_filters_by_label_and_category() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    search(&mut h, "zoom out");
    h.get_by_label("X Zoom Out");
    assert!(h.query_by_label("X Zoom In").is_none());
    assert!(h.query_by_label("Start/Stop Recording").is_none());
    // The Capture section is hidden; Signal Navigation stays.
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Label, "Capture").is_none());
    h.get_by_label("Signal Navigation");
}

#[test]
fn search_matches_key_names() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    search(&mut h, "shift m");
    h.get_by_label("Add Timing Markers Pair");
    assert!(h.query_by_label("Add Marker").is_none());
}

#[test]
fn search_without_results_says_so() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    search(&mut h, "xyzzy");
    h.get_by_label("No shortcuts match");
}

#[test]
fn escape_clears_the_search_then_closes() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    search(&mut h, "palette");
    assert!(h.query_by_label("Jump to Time").is_none());
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_label("Keyboard Shortcuts");
    h.get_by_label("Jump to Time");
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("Keyboard Shortcuts").is_none());
}

#[test]
fn search_is_cleared_when_reopened() {
    let mut h = harness();
    h.run();
    type_question_mark(&mut h);
    search(&mut h, "palette");
    h.get_by_label("Close").click();
    h.run();
    type_question_mark(&mut h);
    h.get_by_label("Jump to Time");
}
