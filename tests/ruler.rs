#![cfg(feature = "mock")]
//! Timing markers, T0, range measurements and confirmations.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::markers::{MarkerRef, TimeOrigin};
use oxrok_ui::core::{CaptureTime, ChannelId, ManualClock};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;
/// The label column: 105 pt of label and the 22 pt of one trigger icon.
const LABEL_W: f32 = 105.0 + 22.0;
const INSPECTOR_W: f32 = 300.0;
const D0: ChannelId = ChannelId::digital(0);

struct Rig {
    h: Harness<'static, LaApp>,
}

impl Rig {
    /// The 16-channel mock with 1 s recorded, zoomed so D0's 1 kHz clock
    /// shows a few periods.
    fn new() -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let mut h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: Arc::new(FixedDialogs(None)),
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        h.run();
        h.get_by_label("Mock 16 digital + 4 analog").click();
        let mut r = Rig { h };
        r.until(|a| a.session().is_some_and(|s| s.caps.is_some()));
        // The Inspector starts closed: open it, so the plot starts past it.
        r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
        r.h.run_steps(3);
        r.h.key_press(Key::Space);
        r.until(|a| a.session().unwrap().state == CaptureState::Recording);
        clock.advance(Duration::from_secs(1));
        r.until(|a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= RATE)
        });
        r.h.key_press(Key::Space);
        r.until(|a| a.session().unwrap().state == CaptureState::Finished);
        r.h.key_press(Key::F);
        r.h.run_steps(2);
        for _ in 0..6 {
            r.h.key_press(Key::Equals);
            r.h.run_steps(1);
        }
        r.h.key_press(Key::Home);
        r.h.run_steps(3);
        r
    }

    fn until(&mut self, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(2);
    }

    fn d0_y(&self) -> f32 {
        self.h.get_by_role_and_label(egui::Role::Label, "D0").rect().center().y
    }

    fn x_of(&self, t: CaptureTime) -> f32 {
        INSPECTOR_W + LABEL_W + self.h.state().session().unwrap().view.state().time_to_px(t) as f32
    }

    /// The first D0 transition at or after pixel `x` of the plot.
    fn edge_after(&self, x: f32) -> CaptureTime {
        let s = self.h.state().session().unwrap();
        let t = s.view.state().px_to_time((x - INSPECTOR_W - LABEL_W) as f64);
        let cap = s.capture.as_ref().unwrap().read();
        let e = cap.channel(D0).unwrap().next_transition(t.to_sample_floor(RATE) as u64).unwrap();
        CaptureTime::from_sample(e, RATE)
    }

    fn click(&mut self, at: Pos2, button: PointerButton) {
        self.h.hover_at(at);
        self.h.run_steps(1);
        for pressed in [true, false] {
            self.h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
        }
        self.h.run_steps(3);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.h.hover_at(from);
        self.h.event(Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        self.h.run_steps(2);
        for k in 1..=6 {
            self.h.hover_at(from + (to - from) * (k as f32 / 6.0));
            self.h.run_steps(2);
        }
        self.h.drop_at(to);
        self.h.run_steps(3);
    }

    fn pair(&mut self, xa: f32, xb: f32) -> u32 {
        let y = self.d0_y() + 200.0;
        self.h.key_press_modifiers(Modifiers::SHIFT, Key::M);
        self.h.run_steps(2);
        self.click(pos2(INSPECTOR_W + xa, y), PointerButton::Primary);
        self.click(pos2(INSPECTOR_W + xb, y), PointerButton::Primary);
        self.h.state().session().unwrap().markers.items.last().unwrap().id
    }
}

/// Ctrl/Cmd+T, click, click: `P0`, listed as `P0 → Δ… (… Hz)` with
/// `0-A` and `0-B` below.
#[test]
fn marker_pair_and_panel() {
    let mut r = Rig::new();
    let id = r.pair(400.0, 700.0);
    let s = r.h.state().session().unwrap();
    let item = s.markers.get(id).unwrap();
    assert_eq!(item.name(), "P0");
    let (a, b) = item.span();
    assert!(a < b);
    r.h.get_by_role_and_label(egui::accesskit::Role::Button, "Markers").click();
    r.h.run_steps(3);
    let row = r.h.get(By::new().label_contains("P0 → Δ"));
    let n = row.accesskit_node();
    let text = n.label().or(n.value()).unwrap_or_default().to_owned();
    assert!(text.ends_with("Hz)"), "{text}");
    r.h.get(By::new().label_contains("0-A = "));
    r.h.get(By::new().label_contains("0-B = "));
    // Heads in the ruler.
    r.h.get_by_label("P0-A");
    r.h.get_by_label("P0-B");
}

/// Placing near an edge snaps to it.
#[test]
fn placement_snaps_to_edges() {
    let mut r = Rig::new();
    let edge = r.edge_after(500.0);
    let x = r.x_of(edge);
    r.h.key_press(Key::M);
    r.h.run_steps(2);
    let y = r.d0_y();
    r.click(pos2(x + 5.0, y), PointerButton::Primary);
    let s = r.h.state().session().unwrap();
    let m = s.markers.items.last().unwrap();
    assert_eq!(m.name(), "T0");
    assert_eq!(m.time(None), edge, "snapped to the transition");
    assert!(r.h.state().session().unwrap().markers.items.iter().all(|i| i.id == m.id), "no temporary left");
}

/// The ruler's Time Zero menu → Timing Marker → P0-A puts 0 there.
#[test]
fn time_zero_on_a_marker() {
    let mut r = Rig::new();
    let id = r.pair(400.0, 700.0);
    let a = r.h.state().session().unwrap().markers.get(id).unwrap().time(Some(0));
    let ruler = pos2(INSPECTOR_W + LABEL_W + 200.0, r.d0_y() - 30.0);
    r.click(ruler, PointerButton::Secondary);
    r.h.get_by_label("Timing Marker").click();
    r.h.run_steps(4);
    r.h.get_by_role_and_label(egui::Role::MenuItemCheckBox, "P0-A").click();
    r.h.run_steps(4);
    let s = r.h.state().session().unwrap();
    assert_eq!(s.origin, TimeOrigin::Marker(MarkerRef { id, side: Some(0) }));
    assert_eq!(s.t0(), a);
}

/// Ctrl/Cmd+G and drag on D0: `M0` with the clock's metrics; Copy
/// produces TSV.
#[test]
fn range_measurement() {
    let mut r = Rig::new();
    r.h.key_press(Key::R);
    r.h.run_steps(2);
    let y = r.d0_y();
    r.drag(pos2(INSPECTOR_W + LABEL_W + 120.0, y), pos2(INSPECTOR_W + LABEL_W + 1000.0, y));
    let s = r.h.state().session().unwrap();
    let m = s.ranges.items.first().expect("range created");
    assert_eq!((m.name.as_str(), m.channel), ("M0", D0));
    // The popup opened with the metric grid.
    let f = r.h.get(By::new().label_contains("f mean = "));
    let n = f.accesskit_node();
    let text = n.label().or(n.value()).unwrap_or_default().to_owned();
    assert_eq!(text, "f mean = 1 kHz", "exactly 1 kHz; trailing zeros dropped");
    r.h.get(By::new().label_contains("N rising = "));
    r.h.get(By::new().label_contains("ΔT = "));
    r.h.get_by_label("Copy").click();
    r.h.step();
    let copied: Vec<String> =
        r.h.output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
    let tsv = copied.last().expect("copied");
    assert!(tsv.starts_with("ΔT\t"), "{tsv}");
    assert!(tsv.lines().any(|l| l.starts_with("f mean\t1000") && l.ends_with("\tHz")), "{tsv}");
    r.h.run_steps(2);
    r.h.get_by_label("Copied!");
}

/// Delete All asks for confirmation; Cancel keeps the markers.
#[test]
fn delete_all_confirms() {
    let mut r = Rig::new();
    r.pair(400.0, 700.0);
    let head = r.h.get_by_label("P0-A").rect().center();
    r.click(head, PointerButton::Secondary);
    r.h.get_by_label("Delete All").click();
    r.h.run_steps(6);
    // The count, pairs as two markers.
    r.h.get_by_label("Delete all 2 timing markers?");
    r.h.get_by_label("Cancel").click();
    r.h.run_steps(6);
    assert_eq!(r.h.state().session().unwrap().markers.items.len(), 1, "kept");
    let head = r.h.get_by_label("P0-A").rect().center();
    r.click(head, PointerButton::Secondary);
    r.h.get_by_label("Delete All").click();
    r.h.run_steps(6);
    r.h.get_by_role_and_label(egui::Role::Button, "Delete All").click();
    r.h.run_steps(6);
    assert!(r.h.state().session().unwrap().markers.items.is_empty());
}

/// Delete removes the selection; `M` `D` does too.
#[test]
fn delete_keys_and_chord() {
    let mut r = Rig::new();
    r.pair(300.0, 500.0);
    r.pair(600.0, 800.0);
    assert_eq!(r.h.state().session().unwrap().markers.items.len(), 2);
    // Finalized items stay selected; select only P1.
    let head = r.h.get_by_label("P1-A").rect().center();
    r.click(pos2(INSPECTOR_W + LABEL_W + 50.0, r.d0_y() + 200.0), PointerButton::Primary);
    r.click(head, PointerButton::Primary);
    r.h.key_press(Key::Escape);
    r.h.key_press(Key::Delete);
    r.h.run_steps(2);
    assert_eq!(r.h.state().session().unwrap().markers.items.len(), 1);
    r.h.get_by_label("P0-A").click();
    r.h.run_steps(2);
    r.h.key_press(Key::Escape);
    r.h.run_steps(2);
    r.h.key_press(Key::Backspace);
    r.h.run_steps(2);
    assert!(r.h.state().session().unwrap().markers.items.is_empty());
}

/// Dragging a head moves that side, and B past A swaps.
#[test]
fn drag_a_head() {
    let mut r = Rig::new();
    let id = r.pair(400.0, 700.0);
    let b = r.h.get_by_label("P0-B").rect().center();
    r.drag(b, b - vec2(500.0, 0.0));
    let s = r.h.state().session().unwrap();
    let (a, bb) = s.markers.get(id).unwrap().span();
    assert!(a < bb && r.x_of(a) < INSPECTOR_W + 300.0, "B moved left of A and swapped");
}

/// `Shift+R` wraps the pulse under the pointer in a range.
#[test]
fn quick_add_pulse() {
    let mut r = Rig::new();
    let y = r.d0_y();
    r.h.hover_at(pos2(INSPECTOR_W + LABEL_W + 400.0, y));
    r.h.run_steps(2);
    r.h.key_press_modifiers(Modifiers::SHIFT, Key::R);
    r.h.run_steps(2);
    let s = r.h.state().session().unwrap();
    let m = s.ranges.items.first().expect("a range was created");
    assert_eq!(m.channel, D0);
    assert!(m.selected, "the new range is selected");
    assert!(m.t.1 > m.t.0, "it spans the pulse");
}

/// `Ctrl/Cmd+Z` undoes a delete; `Ctrl/Cmd+Shift+Z` redoes it.
#[test]
fn range_undo_redo() {
    let mut r = Rig::new();
    r.h.key_press(Key::R);
    r.h.run_steps(2);
    let y = r.d0_y();
    r.drag(pos2(INSPECTOR_W + LABEL_W + 120.0, y), pos2(INSPECTOR_W + LABEL_W + 1000.0, y));
    assert_eq!(r.h.state().session().unwrap().ranges.items.len(), 1);
    // Delete it from the popup.
    r.h.get_by_label("Delete measurement").click();
    r.h.run_steps(2);
    assert!(r.h.state().session().unwrap().ranges.items.is_empty(), "deleted");
    // Undo restores it.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    r.h.run_steps(2);
    assert_eq!(r.h.state().session().unwrap().ranges.items.len(), 1, "undo restored it");
    // Redo removes it again.
    r.h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    r.h.run_steps(2);
    assert!(r.h.state().session().unwrap().ranges.items.is_empty(), "redo removed it");
}

/// A range on an analog row, zoomed in far enough to draw samples, then
/// recording again: the new capture's analog data is empty at first (this
/// crashed: "range end index … out of range for slice of length 0").
#[test]
fn rerecord_with_analog_range_zoomed_in() {
    let mut r = Rig::new();
    let a0 = r.h.get_by_role_and_label(egui::Role::Label, "A0").rect().center().y;
    r.h.key_press(Key::R);
    r.h.run_steps(2);
    r.drag(pos2(INSPECTOR_W + LABEL_W + 300.0, a0), pos2(INSPECTOR_W + LABEL_W + 600.0, a0));
    assert_eq!(r.h.state().session().unwrap().ranges.items.len(), 1);
    for _ in 0..20 {
        r.h.key_press(Key::Equals);
        r.h.run_steps(1);
    }
    r.h.key_press(Key::End);
    r.h.run_steps(2);
    r.h.key_press(Key::Space);
    for _ in 0..30 {
        r.h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    r.h.key_press(Key::Space);
    r.h.run_steps(5);
}

/// Ruler menu → Delete Data → Before This Point. The confirmation
/// names what goes; afterwards the data begins there and the marker
/// before it is gone.
#[test]
fn delete_data_before_this_point() {
    let mut r = Rig::new();
    let single = r.h.state_mut().session_mut().unwrap().markers.add_single(CaptureTime::ZERO);
    r.h.state_mut().session_mut().unwrap().markers.get_mut(single).unwrap().label = "T9".into();
    let x = INSPECTOR_W + LABEL_W + 300.0;
    let pair = r.pair(LABEL_W + 500.0, LABEL_W + 700.0);
    let ruler = pos2(x, r.d0_y() - 30.0);
    r.click(ruler, PointerButton::Secondary);
    r.h.get_by_label("Delete Data").click();
    r.h.run_steps(4);
    r.h.get_by_label("Before This Point").click();
    r.h.run_steps(4);
    // Concrete wording and the action on the button.
    let body: Vec<String> =
        r.h.query_all(By::new().role(egui::Role::Label))
            .filter_map(|n| {
                let a = n.accesskit_node();
                a.label().or(a.value()).map(|l| l.to_owned())
            })
            .filter(|l| l.starts_with("Delete ") || l.contains("timing marker"))
            .collect();
    assert!(body.iter().any(|l| l.contains("of data before this point")), "{body:?}");
    assert!(body.iter().any(|l| l.contains("1 timing marker outside the kept data are removed")), "{body:?}");
    let button =
        r.h.query_all(By::new().role(egui::Role::Button))
            .filter_map(|n| n.accesskit_node().label().map(|l| l.to_owned()))
            .find(|l| l.starts_with("Delete ") && l.ends_with('s'))
            .expect("a Delete ‹duration› button");
    r.h.get_by_role_and_label(egui::Role::Button, &button).click();
    r.h.run_steps(4);
    let s = r.h.state().session().unwrap();
    let (begin, _) = s.capture.as_ref().unwrap().read().time_range().unwrap();
    assert!(begin > CaptureTime::ZERO, "the data now begins at the point");
    assert!(s.markers.get(single).is_none(), "the marker before the point is gone");
    assert!(s.markers.get(pair).is_some(), "the pair after it stays");
    assert!(s.is_unsaved());
}
