#![cfg(feature = "mock")]
//! Rows, fit to window and Auto Layout, row colors.

use egui::accesskit::Toggled;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::LaApp;
use oxrok_ui::backend::MockBackend;
use oxrok_ui::core::ChannelId;

const D0: ChannelId = ChannelId::digital(0);
const D1: ChannelId = ChannelId::digital(1);

fn open() -> Harness<'static, LaApp> {
    // 60 fps steps keep egui's animations (menus, row gaps) short.
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)));
    h.run();
    h.get_by_label("Mock 16 digital + 4 analog").click();
    h.run_steps(5);
    h
}

fn label_rect(h: &Harness<'_, LaApp>, text: &str) -> egui::Rect {
    h.get_by_role_and_label(egui::Role::Label, text).rect()
}

fn right_click(h: &mut Harness<'_, LaApp>, at: Pos2) {
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run_steps(4);
}

/// Right-clicks the row label `label`.
fn menu_on(h: &mut Harness<'_, LaApp>, label: &str) {
    let at = label_rect(h, label).center();
    right_click(h, at);
}

fn drag(h: &mut Harness<'_, LaApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.run_steps(2);
    }
    h.drop_at(to);
    h.run_steps(20);
}

fn row_order(h: &Harness<'_, LaApp>) -> Vec<ChannelId> {
    h.state().session().unwrap().visible_rows().map(|r| r.channel).collect()
}

fn menu_toggled(h: &Harness<'_, LaApp>, label: &str) -> Option<Toggled> {
    h.get_by_role_and_label(egui::Role::MenuItemCheckBox, label).accesskit_node().toggled()
}

/// Edit a row name inline; Tab commits and moves to the next row.
#[test]
fn rename_and_tab() {
    let mut h = open();
    h.get_by_label("Rename D0").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("SPI CLK".into()));
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
    let s = h.state().session().unwrap();
    assert_eq!(s.row(D0).unwrap().name, "SPI CLK");
    let editor = h.get_by_role_and_label(egui::Role::TextInput, "D1 name");
    assert!(editor.accesskit_node().is_focused(), "focus moved to D1's name");
    // Escape reverts; an empty name commits as the default.
    h.event(Event::Text("x".into()));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().session().unwrap().row(D1).unwrap().name, "D1");
    h.get_by_role_and_label(egui::Role::Label, "D0 SPI CLK");
    h.get_by_label("Rename D0").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session().unwrap().row(D0).unwrap().name, "D0");
}

/// Drag the label column edge to the minimum: identifiers only.
#[test]
fn label_column_collapses() {
    let mut h = open();
    h.get_by_label("Rename D0").click();
    h.run_steps(3);
    h.event(Event::Text("CS".into()));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.get_by_role_and_label(egui::Role::Label, "D0 D0CS");
    let edge = h.get_by_label("Resize label column").rect().center();
    let t_before = h.state().session().unwrap().view.state().px_to_time(400.0);
    drag(&mut h, edge, edge - vec2(200.0, 0.0));
    assert_eq!(h.state().settings().label_width, 53.0);
    h.get_by_role_and_label(egui::Role::Label, "D0");
    assert!(h.query_by_label("Rename D0").is_none(), "no name area when collapsed");
    // The waveform under a screen position doesn't jump.
    let v = h.state().session().unwrap().view.state();
    let moved = v.time_to_px(t_before) - (400.0 + (105.0 - 53.0));
    assert!(moved.abs() < 0.01, "{moved}");
}

/// Resizing snaps to 5 pt and turns Auto Layout off (menu check).
#[test]
fn resize_turns_auto_layout_off() {
    let mut h = open();
    menu_on(&mut h, "D0");
    h.get_by_label("Resize").click();
    h.run_steps(4);
    h.get_by_label("Auto Layout").click();
    h.run_steps(4);
    assert!(h.state().settings().waveform.auto_layout);
    // Auto Layout fits the rows to the window: analog rows get the room.
    let a0 = h.state().session().unwrap().row(ChannelId::analog(0)).unwrap().height;
    assert!(a0 > 200.0, "fit: {a0}");

    let r = label_rect(&h, "D0");
    let start = pos2(r.center().x, r.bottom() - 1.0);
    drag(&mut h, start, start + vec2(0.0, 22.0));
    let height = h.state().session().unwrap().row(D0).unwrap().height;
    assert_eq!(height % 5.0, 0.0, "snapped: {height}");
    assert!(!h.state().settings().waveform.auto_layout);
    menu_on(&mut h, "D0");
    h.get_by_label("Resize").click();
    h.run_steps(4);
    assert_eq!(menu_toggled(&h, "Auto Layout"), Some(Toggled::False));
}

/// Hide Channel; the Show/Hide submenu toggles it back and stays open.
#[test]
fn hide_and_show_without_closing_the_menu() {
    let mut h = open();
    menu_on(&mut h, "D0");
    h.get_by_label("Hide Channel").click();
    h.run_steps(4);
    assert!(h.query_by_role_and_label(egui::Role::Label, "D0").is_none());
    assert_eq!(row_order(&h)[0], D1);

    menu_on(&mut h, "D1");
    h.get_by_label("Show/Hide Channels").click();
    h.run_steps(4);
    assert_eq!(menu_toggled(&h, "D0. D0"), Some(Toggled::False));
    h.get_by_label("D0. D0").click();
    h.run_steps(4);
    assert_eq!(menu_toggled(&h, "D0. D0"), Some(Toggled::True), "still open, now checked");
    h.get_by_label("D1. D1").click();
    h.run_steps(4);
    assert_eq!(row_order(&h), [D0, ChannelId::analog(0), ChannelId::analog(1)]);
    h.get_by_label("Show All Channels").click();
    h.run_steps(4);
    assert_eq!(row_order(&h)[..2], [D0, D1]);
}

/// (in-session part; saving arrives in 0.5.0): drag a label to reorder.
#[test]
fn drag_label_to_reorder() {
    let mut h = open();
    let d0 = label_rect(&h, "D0");
    let a0 = label_rect(&h, "A0");
    // Grab D0 by its stripe (not the name) and drop it inside A0's lower half.
    let from = pos2(d0.left() + 3.0, d0.center().y);
    drag(&mut h, from, pos2(from.x, a0.center().y + 20.0));
    assert_eq!(row_order(&h), [D1, ChannelId::analog(0), D0, ChannelId::analog(1)]);
    // Escape cancels a drag in progress.
    let d1 = label_rect(&h, "D1");
    let from = pos2(d1.left() + 3.0, d1.center().y);
    h.hover_at(from);
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    for k in 1..=6 {
        h.hover_at(from + vec2(0.0, 30.0 * k as f32));
        h.run_steps(2);
    }
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.drop_at(from + vec2(0.0, 180.0));
    h.run_steps(10);
    assert_eq!(row_order(&h), [D1, ChannelId::analog(0), D0, ChannelId::analog(1)], "cancelled");
}

/// A row color overrides the palette; Toggle Traces Color.
#[test]
fn row_color_and_neutral_traces() {
    let mut h = open();
    menu_on(&mut h, "D1");
    h.get_by_label("Appearance").click();
    h.run_steps(4);
    h.get_by_label("Channel Color").click();
    h.run_steps(4);
    h.get_by_label("Teal").click();
    h.run_steps(4);
    let c = h.state().session().unwrap().row(D1).unwrap().color;
    assert_eq!(c, Some([0x4F, 0xC4, 0xB0]));
    menu_on(&mut h, "D1");
    h.get_by_label("Appearance").click();
    h.run_steps(4);
    h.get_by_label("Toggle Traces Color").click();
    h.run_steps(4);
    assert!(!h.state().settings().waveform.color_waveforms);
    // Reset Channel Names restores defaults.
    h.get_by_label("Rename D1").click();
    h.run_steps(3);
    h.event(Event::Text("MOSI".into()));
    h.key_press(Key::Enter);
    h.run_steps(3);
    menu_on(&mut h, "D1 D1MOSI");
    h.get_by_label("Reset Channel Names").click();
    h.run_steps(4);
    assert_eq!(h.state().session().unwrap().row(D1).unwrap().name, "D1");
}

/// From the menu, digital rows only: they share the height.
#[test]
fn fit_all_rows_digital_only() {
    let mut h = open();
    // The Inspector starts closed: open it to reach the channel chips.
    h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    h.run_steps(3);
    for chip in ["A0", "A1"] {
        h.get_by_role_and_label(egui::Role::CheckBox, chip).click();
        h.run();
    }
    menu_on(&mut h, "D0");
    h.get_by_label("Resize").click();
    h.run_steps(4);
    h.get_by_label("Fit all Rows to Window Height").click();
    h.run_steps(4);
    let s = h.state().session().unwrap();
    let (d0, d1) = (s.row(D0).unwrap().height, s.row(D1).unwrap().height);
    // max(200, 50 % of the 800 pt window) = 400 each, the stack is ~650 pt.
    assert!(d0 > 250.0 && d0 <= 400.0 && d1 >= d0, "{d0} {d1}");
}
