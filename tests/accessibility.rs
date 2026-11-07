#![cfg(feature = "mock")]
//! Every interactive widget has an accessible label, on the main screens and dialogs.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use oxrok_ui::app::LaApp;
use oxrok_ui::backend::MockBackend;

const INTERACTIVE: [Role; 10] = [
    Role::Button,
    Role::CheckBox,
    Role::RadioButton,
    Role::Switch,
    Role::TextInput,
    Role::ComboBox,
    Role::MenuItem,
    Role::Slider,
    Role::Link,
    Role::Tab,
];

/// The interactive nodes without a label or value.
fn unlabeled(h: &Harness<'static, LaApp>, screen: &str) -> Vec<String> {
    if std::env::var("A11Y_DEBUG").is_ok() {
        let all: Vec<_> = h.query_all(By::new()).collect();
        let inter = all.iter().filter(|n| INTERACTIVE.contains(&n.accesskit_node().role())).count();
        eprintln!("{screen}: {} nodes, {inter} interactive", all.len());
    }
    h.query_all(By::new())
        .filter_map(|n| {
            let a = n.accesskit_node();
            let role = a.role();
            let named =
                a.label().is_some_and(|l| !l.trim().is_empty()) || a.value().is_some_and(|v| !v.trim().is_empty());
            (INTERACTIVE.contains(&role) && !named).then(|| format!("{screen}: {role:?} at {:?}", n.rect()))
        })
        .collect()
}

#[test]
fn every_interactive_widget_is_labeled() {
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)));
    h.run();
    let mut bad = unlabeled(&h, "start screen");
    h.get_by_label("Mock 16 digital + 4 analog").click();
    h.run_steps(10);
    // The Inspector starts closed: open it, so its tabs are checked too.
    h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    h.run_steps(3);
    bad.extend(unlabeled(&h, "device settings"));
    h.key_press(Key::Space);
    h.run_steps(5);
    h.key_press(Key::Space);
    h.run_steps(10);
    bad.extend(unlabeled(&h, "after a capture"));
    for (label, screen) in
        [("Analyzers", "analyzers tab"), ("Measurements", "measurements tab"), ("Markers", "markers tab")]
    {
        h.get_by_role_and_label(Role::Button, label).click();
        h.run_steps(5);
        bad.extend(unlabeled(&h, screen));
    }
    // The analyzer settings dialog, from the panel's decoder list.
    h.get_by_role_and_label(Role::Button, "Analyzers").click();
    h.run_steps(5);
    h.get_by_role_and_label(Role::MenuItem, "Async Serial").click();
    h.run_steps(10);
    bad.extend(unlabeled(&h, "analyzer dialog"));
    h.key_press(Key::Escape);
    h.run_steps(5);
    h.event(egui::Event::Text("?".into()));
    h.run_steps(5);
    bad.extend(unlabeled(&h, "keyboard shortcuts"));
    h.key_press(Key::Escape);
    h.run_steps(5);
    for (mods, key, screen) in [
        (Modifiers::COMMAND, Key::E, "export dialog"),
        (Modifiers::COMMAND, Key::Comma, "settings dialog"),
        (Modifiers::COMMAND, Key::K, "command palette"),
        (Modifiers::COMMAND | Modifiers::SHIFT, Key::O, "welcome wizard"),
    ] {
        h.key_press_modifiers(mods, key);
        h.run_steps(5);
        bad.extend(unlabeled(&h, screen));
        h.key_press(Key::Escape);
        h.run_steps(5);
    }
    for (menu, item, screen) in [("Help", "Device Connection Help…", "connection help"), ("Help", "About", "about")] {
        h.get_by_label(menu).click();
        h.run();
        h.get_by_label(item).click();
        h.run_steps(5);
        bad.extend(unlabeled(&h, screen));
        h.key_press(Key::Escape);
        h.run_steps(5);
    }
    assert!(bad.is_empty(), "unlabeled widgets:\n{}", bad.join("\n"));
}
