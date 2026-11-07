#![cfg(feature = "mock")]
//! The welcome wizard:.

use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::LaApp;
use oxrok_ui::app::settings::ThemePref;
use oxrok_ui::backend::MockBackend;

fn harness() -> Harness<'static, LaApp> {
    Harness::builder().with_size([1280.0, 800.0]).build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)))
}

fn first_run() -> Harness<'static, LaApp> {
    let mut h = harness();
    // The test constructor pins Dark; a first launch has the default.
    h.state_mut().settings_mut().appearance.theme = ThemePref::System;
    h.state_mut().open_onboarding_on_first_run();
    h.run();
    h
}

fn selected(h: &Harness<'static, LaApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::RadioButton, label).accesskit_node().toggled() == Some(Toggled::True)
}

/// The theme step with System selected; Light applies at once;
/// Next, Next, Finish ends on the start screen, and the wizard doesn't
/// come back.
#[test]
fn first_launch() {
    let mut h = first_run();
    h.get_by_label("Welcome to Oxrok UI");
    h.get_by_label("Step 1 of 3");
    assert!(selected(&h, "System"));
    h.get_by_role_and_label(Role::RadioButton, "Light").click();
    h.run();
    assert_eq!(h.state().settings().appearance.theme, ThemePref::Light);
    assert!(selected(&h, "Light"));
    h.snapshot("onboarding_light");

    h.get_by_label("Next").click();
    h.run();
    h.get_by_label("Step 2 of 3");
    h.get_by_label("Device access");
    h.get_by_label("Next").click();
    h.run();
    h.get_by_label("Step 3 of 3");
    h.get_by_label("starts and stops recording");
    h.snapshot("onboarding_tips");
    // Previous goes back a step.
    h.get_by_label("Previous").click();
    h.run();
    h.get_by_label("Step 2 of 3");
    h.get_by_label("Next").click();
    h.run();
    h.get_by_label("Finish").click();
    h.run();
    assert!(h.query_by_label("Welcome to Oxrok UI").is_none());
    assert!(h.state().settings().onboarding_done);
    h.get_by_label("Load a Capture File");

    h.state_mut().open_onboarding_on_first_run();
    h.run();
    assert!(h.query_by_label("Welcome to Oxrok UI").is_none(), "only once");
}

/// Escape and Skip close it for good; Help → Start Onboarding
/// reopens it.
#[test]
fn escape_skip_and_reopen() {
    let mut h = first_run();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("Welcome to Oxrok UI").is_none());
    assert!(h.state().settings().onboarding_done);

    h.get_by_label("Help").click();
    h.run();
    h.get_by_label("Start Onboarding").click();
    h.run();
    h.get_by_label("Step 1 of 3");
    h.get_by_label("Skip").click();
    h.run();
    assert!(h.query_by_label("Welcome to Oxrok UI").is_none());

    // CmdOrCtrl+Shift+O reopens it too.
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::O);
    h.run();
    h.get_by_label("Step 1 of 3");
}

/// Step 2 counts real devices only: the mock's are all demo devices.
#[test]
fn device_step_counts_real_devices() {
    let mut h = first_run();
    h.get_by_label("Next").click();
    h.run();
    h.get_by_label("No device found yet. You can continue and connect later.");
}
