//! Widget tests with `egui_kittest`: locate by accessible label,
//! drive input, check state; gallery snapshots at 1× and 2×.

use egui::Role;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, HarnessBuilder};
use oxrok_ui::core::{Clock, ManualClock};
use oxrok_ui::ui::gallery::{GalleryState, gallery};
use oxrok_ui::ui::{Theme, clock};
use std::sync::Arc;
use std::time::Duration;

fn builder() -> HarnessBuilder<GalleryState> {
    Harness::builder().with_size([640.0, 480.0]).with_context_setup(|ctx| oxrok_ui::ui::install(ctx, Theme::dark()))
}

fn harness() -> Harness<'static, GalleryState> {
    builder().build_ui_state(|ui, state: &mut GalleryState| gallery(ui, state), GalleryState::default())
}

#[test]
fn primary_button_clicks_by_label() {
    let mut h = harness();
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    assert_eq!(h.state().primary_clicks, 1);
}

#[test]
fn disabled_button_ignores_clicks() {
    let h = harness();
    let node = h.get_by_role_and_label(Role::Button, "Disabled");
    assert!(node.accesskit_node().is_disabled());
}

#[test]
fn tertiary_button_reports_selection() {
    let mut h = harness();
    h.get_by_role_and_label(Role::Button, "All").click();
    h.run();
    assert!(h.state().tertiary_selected);
}

#[test]
fn checkbox_toggles() {
    let mut h = harness();
    let cb = h.get_by_role_and_label(Role::CheckBox, "Glitch filter");
    assert_eq!(cb.accesskit_node().toggled(), Some(Toggled::False));
    cb.click();
    h.run();
    assert!(h.state().checked);
    let cb = h.get_by_role_and_label(Role::CheckBox, "Glitch filter");
    assert_eq!(cb.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn switch_toggles() {
    let mut h = harness();
    h.get_by_role_and_label(Role::Switch, "Trim after capture").click();
    h.run();
    assert!(h.state().switch_on);
}

#[test]
fn dropdown_selects_with_mouse() {
    let mut h = harness();
    h.get_by_role_and_label(Role::ComboBox, "Sample rate").click();
    h.run();
    h.get_by_role_and_label(Role::ListBoxOption, "24 MS/s").click();
    h.run();
    assert_eq!(h.state().choice, 2);
    // The popup closed.
    assert!(h.query_by_role_and_label(Role::ListBoxOption, "24 MS/s").is_none());
}

#[test]
fn dropdown_selects_with_keyboard() {
    let mut h = harness();
    h.get_by_role_and_label(Role::ComboBox, "Sample rate").click();
    h.run();
    h.key_press(egui::Key::ArrowDown);
    h.run();
    h.key_press(egui::Key::ArrowDown);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().choice, 2);
}

#[test]
fn injected_clock_is_shared_with_the_ui() {
    let manual = ManualClock::new();
    let h = harness();
    clock::set_clock(&h.ctx, Arc::new(manual.clone()));
    manual.advance(Duration::from_millis(1500));
    assert_eq!(clock::now(&h.ctx), Duration::from_millis(1500));
    assert_eq!(clock::clock(&h.ctx).now(), manual.now());
}

#[test]
fn gallery_snapshots() {
    let mut results = egui_kittest::SnapshotResults::new();
    for ppp in [1.0, 2.0] {
        let mut h = builder()
            .with_pixels_per_point(ppp)
            .build_ui_state(|ui, state: &mut GalleryState| gallery(ui, state), GalleryState::default());
        h.run();
        results.add(h.try_snapshot(format!("gallery_dark_{ppp}x")));
    }
}

#[test]
fn dropdown_open_snapshot() {
    let mut h = harness();
    h.get_by_role_and_label(Role::ComboBox, "Sample rate").click();
    h.run();
    h.snapshot("dropdown_open_dark");
}

/// A list longer than the window stays inside it and scrolls: the keyboard
/// highlight is brought into view, and the last option can be clicked.
#[test]
fn long_dropdown_scrolls_within_the_window() {
    let options: Vec<String> = (0..40).map(|i| format!("Option {i}")).collect();
    let mut h = Harness::builder()
        .with_size([320.0, 300.0])
        .with_context_setup(|ctx| oxrok_ui::ui::install(ctx, Theme::dark()))
        .build_ui_state(
            move |ui, choice: &mut usize| {
                ui.add(oxrok_ui::ui::widgets::Dropdown::new("Channel", choice, &options));
            },
            0usize,
        );
    h.get_by_role_and_label(Role::ComboBox, "Channel").click();
    h.run();
    let first = h.get_by_role_and_label(Role::ListBoxOption, "Option 0").rect();
    assert!(first.top() >= 0.0 && first.bottom() <= 300.0, "{first:?}");
    for _ in 0..39 {
        h.key_press(egui::Key::ArrowDown);
    }
    h.run();
    let last = h.get_by_role_and_label(Role::ListBoxOption, "Option 39").rect();
    assert!(last.top() >= 0.0 && last.bottom() <= 300.0, "{last:?}");
    h.get_by_role_and_label(Role::ListBoxOption, "Option 39").click();
    h.run();
    assert_eq!(*h.state(), 39);
}
