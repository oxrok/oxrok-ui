#![cfg(feature = "mock")]
//! Navigation: fling, the zoom tool and edge navigation.

use egui::accesskit::Toggled;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{CaptureTime, ManualClock, ViewState};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;
/// The label column: 105 pt of label and the 22 pt of one trigger icon.
const LABEL_W: f32 = 105.0 + 22.0;
const INSPECTOR_W: f32 = 300.0;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

/// `animation_speed` 0 makes view changes immediate; 1 runs them on the
/// manual clock.
fn rig(animation_speed: f64) -> Rig {
    let clock = ManualClock::new();
    let c = clock.clone();
    // 60 fps steps: a double-click must fit in egui's 0.3 s.
    let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(MockBackend::new(options)),
                clock: Arc::new(c),
                dialogs: Arc::new(FixedDialogs(None)),
                settings_path: None,
                animation_speed,
                scan_sync: true,
                profiles_dir: None,
                extra_builtin_profiles: Vec::new(),
                mock_decoders: cfg!(feature = "mock"),
            },
        )
    });
    Rig { h, clock }
}

impl Rig {
    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.step();
    }

    fn view(&self) -> ViewState {
        self.h.state().session().unwrap().view.state()
    }

    /// Opens the 16-channel mock and records `secs` of data, then fits it.
    fn with_capture(animation_speed: f64, secs: u64) -> Self {
        let mut r = rig(animation_speed);
        r.h.run();
        r.h.get_by_label("Mock 16 digital + 4 analog").click();
        r.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
        // The Inspector starts closed: open it, so the plot starts past it.
        r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
        r.h.run_steps(3);
        r.h.key_press(Key::Space);
        r.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        r.clock.advance(Duration::from_secs(secs));
        let want = secs * RATE;
        r.until("data", move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        r.h.key_press(Key::Space);
        r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
        r.h.key_press(Key::F);
        r.settle();
        r
    }

    /// Lets animations finish on the manual clock.
    fn settle(&mut self) {
        for _ in 0..5 {
            self.clock.advance(Duration::from_millis(200));
            self.h.run_steps(2);
        }
    }

    fn press(&mut self, at: Pos2, modifiers: Modifiers) {
        self.h.hover_at(at);
        self.h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
        self.h.run_steps(2);
    }

    /// A drag in `steps` moves of `step`, `dt` apart on the clock.
    fn drag(&mut self, from: Pos2, step: egui::Vec2, steps: usize, dt: Duration, modifiers: Modifiers) -> Pos2 {
        if modifiers != Modifiers::NONE {
            self.h.event(Event::ModifiersChanged(modifiers));
        }
        self.press(from, modifiers);
        let mut at = from;
        for _ in 0..steps {
            at += step;
            self.clock.advance(dt);
            self.h.hover_at(at);
            self.h.run_steps(2);
        }
        self.h.drop_at(at);
        self.h.run_steps(2);
        if modifiers != Modifiers::NONE {
            self.h.event(Event::ModifiersChanged(Modifiers::NONE));
            self.h.run_steps(2);
        }
        at
    }

    fn plot_point(&self, row: &str) -> Pos2 {
        let label = self.h.get_by_role_and_label(egui::Role::Label, row).rect();
        pos2(INSPECTOR_W + LABEL_W + 400.0, label.center().y)
    }
}

/// A fling continues after release and decays.
#[test]
fn fling_continues_and_decays() {
    let mut r = Rig::with_capture(1.0, 1);
    for _ in 0..4 {
        r.h.key_press(Key::Equals);
        r.h.run_steps(2);
    }
    r.settle();
    let before = r.view();
    let from = r.plot_point("D0");
    // 6 × 20 pt every 10 ms: 2000 pt/s to the right.
    r.drag(from, vec2(20.0, 0.0), 6, Duration::from_millis(10), Modifiers::NONE);
    let released = before.time_to_px(r.view().left_time);
    r.clock.advance(Duration::from_millis(30));
    r.h.run_steps(2);
    let mid = before.time_to_px(r.view().left_time);
    r.settle();
    let end = before.time_to_px(r.view().left_time);
    assert!((released + 120.0).abs() < 2.0, "1:1 while dragging: {released}");
    // Fling v0 = −1000 pt/s, τ = 100 ms: about 100 pt more, most of it early.
    assert!(mid < released - 10.0, "still moving after release: {mid}");
    assert!((end - released + 100.0).abs() < 10.0, "decayed after ≈ v0·τ: {}", end - released);
}

/// Ctrl/Cmd+Shift+drag horizontally zooms to the dragged range.
#[test]
fn marquee_zooms_to_the_dragged_range() {
    let mut r = Rig::with_capture(0.0, 1);
    let from = r.plot_point("D0");
    let t0 = r.view().px_to_time((from.x - INSPECTOR_W - LABEL_W) as f64);
    let t1 = r.view().px_to_time((from.x + 200.0 - INSPECTOR_W - LABEL_W) as f64);
    r.drag(from, vec2(40.0, 3.0), 5, Duration::from_millis(10), Modifiers::COMMAND | Modifiers::SHIFT);
    let v = r.view();
    assert!(v.left_time.secs_since(t0).abs() < 1e-9 && v.right_time().secs_since(t1).abs() < 1e-9, "{v:?}");
}

/// Releasing the modifiers before the button cancels the marquee.
#[test]
fn marquee_cancelled_by_releasing_modifiers() {
    let mut r = Rig::with_capture(0.0, 1);
    let before = r.view();
    let from = r.plot_point("D0");
    r.h.event(Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT));
    r.press(from, Modifiers::COMMAND | Modifiers::SHIFT);
    r.h.hover_at(from + vec2(150.0, 0.0));
    r.h.run_steps(2);
    r.h.event(Event::ModifiersChanged(Modifiers::NONE));
    r.h.run_steps(2);
    r.h.drop_at(from + vec2(200.0, 0.0));
    r.h.run_steps(2);
    assert_eq!(r.view().time_per_px, before.time_per_px, "no zoom");
}

/// The armed zoom tool turns the next drag into a range zoom,
/// then disarms; Y mode on an analog row zooms its value axis.
#[test]
fn zoom_tool_arms_once_and_y_mode_zooms_analog() {
    let mut r = Rig::with_capture(0.0, 1);
    r.h.get_by_label("Zoom Tool Options").click();
    r.h.run();
    r.h.get_by_label("Y Range Zoom").click();
    r.h.run();
    assert_eq!(r.h.state().settings().marquee_mode, oxrok_ui::app::settings::MarqueeMode::Y);
    let tool = r.h.get_by_role_and_label(egui::Role::Button, "Zoom Tool");
    assert_eq!(tool.accesskit_node().toggled(), Some(Toggled::True), "armed");

    let a0 = oxrok_ui::core::ChannelId::analog(0);
    let range = |r: &Rig| {
        let s = r.h.state().session().unwrap();
        let row = s.row(a0).unwrap();
        row.analog.unwrap().visible_range(row.analog_inner.unwrap() as f64)
    };
    let before = range(&r);
    let label = r.h.get_by_role_and_label(egui::Role::Label, "A0").rect();
    let from = pos2(INSPECTOR_W + LABEL_W + 300.0, label.top() + 20.0);
    r.drag(from, vec2(2.0, 10.0), 4, Duration::from_millis(10), Modifiers::NONE);
    let after = range(&r);
    assert!(after.1 - after.0 < (before.1 - before.0) / 2.0, "zoomed in: {before:?} → {after:?}");
    let tool = r.h.get_by_role_and_label(egui::Role::Button, "Zoom Tool");
    assert_eq!(tool.accesskit_node().toggled(), Some(Toggled::False), "disarmed after one use");
}

/// double-click zooms ×0.3 about the click.
#[test]
fn double_click_zooms_about_the_click() {
    let mut r = Rig::with_capture(0.0, 1);
    let at = r.plot_point("D0");
    let before = r.view();
    let t = before.px_to_time((at.x - INSPECTOR_W - LABEL_W) as f64);
    for _ in 0..2 {
        r.h.hover_at(at);
        r.h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        r.h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        r.h.run_steps(2);
    }
    r.h.run();
    let v = r.view();
    assert!((v.time_per_px / before.time_per_px - 0.3).abs() < 1e-9, "{}", v.time_per_px / before.time_per_px);
    assert!((v.time_to_px(t) - (at.x - INSPECTOR_W - LABEL_W) as f64).abs() < 0.01, "the clicked time stays put");
}

/// Typing `250us` sets the time scale; Recent Zooms lists it.
#[test]
fn zoom_controller_and_recent_zooms() {
    let mut r = Rig::with_capture(0.0, 1);
    r.h.get_by_label("1 s").click();
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.event(Event::Text("250us".into()));
    r.h.key_press(Key::Enter);
    r.h.run();
    assert!((r.view().time_scale() - 250e-6).abs() < 1e-12, "{}", r.view().time_scale());
    r.h.get_by_label("250 µs").click();
    r.h.run();
    r.h.get_by_label("RECENT ZOOMS");
    // The recent item, besides the button that opened the popover.
    assert_eq!(r.h.query_all_by_label("250 µs").count(), 2);
    assert_eq!(r.h.state().settings().history.recent_zooms, [250e-6]);
}

/// Ctrl/Cmd+J, `10ms`, Enter centers on T0 + 10 ms; out-of-range
/// input shows the error text and keeps the view.
#[test]
fn jump_to_time() {
    let mut r = Rig::with_capture(0.0, 1);
    for _ in 0..6 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.key_press(Key::G);
    r.h.run_steps(3);
    r.h.get_by_label("Type a value between: 0 s to 1 s");
    r.h.event(Event::Text("10ms".into()));
    r.h.key_press(Key::Enter);
    r.h.run_steps(3);
    let center = r.view().center_time();
    assert!((center.secs_since(CaptureTime::ZERO) - 0.010).abs() < 1e-9, "{center:?}");

    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.event(Event::Text("5s".into()));
    r.h.key_press(Key::Enter);
    r.h.run_steps(3);
    r.h.get_by_label("Value must be between 0 s and 1 s.");
    assert!((r.view().center_time().secs_since(CaptureTime::ZERO) - 0.010).abs() < 1e-9);
    // Escape closes; the text is remembered.
    r.h.key_press(Key::Escape);
    r.h.run_steps(3);
    assert!(r.h.query_by_label("Jump to Time").is_none());
    assert_eq!(r.h.state().session().unwrap().jump_text, "5s");
}

/// Back and forward restore the previous zoom.
#[test]
fn view_history() {
    let mut r = Rig::with_capture(0.0, 1);
    // The fitted view becomes the first entry after the debounce.
    r.clock.advance(Duration::from_millis(400));
    r.h.run_steps(2);
    let fitted = r.view();
    for _ in 0..3 {
        r.h.key_press(Key::Equals);
        r.h.run_steps(2);
    }
    r.clock.advance(Duration::from_millis(400));
    r.h.run_steps(2);
    let zoomed = r.view();
    assert!(zoomed.time_per_px < fitted.time_per_px);

    r.h.get_by_label("Previous view").click();
    r.h.run_steps(3);
    assert!((r.view().time_per_px / fitted.time_per_px - 1.0).abs() < 1e-9, "back to the fit");
    r.h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    r.h.run_steps(3);
    assert!((r.view().time_per_px / zoomed.time_per_px - 1.0).abs() < 1e-9, "forward to the zoom");
    let next = r.h.get_by_role_and_label(egui::Role::Button, "Next view");
    assert!(next.accesskit_node().is_disabled(), "nothing further");
}

/// Edge buttons on the hovered row, and `.`.
#[test]
fn edge_navigation() {
    let mut r = Rig::with_capture(0.0, 1);
    // Zoom far in so edges lie outside the view.
    for _ in 0..30 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.key_press(Key::Home);
    r.h.run();
    let at = r.plot_point("D1");
    r.h.hover_at(at);
    r.h.run();
    assert!(r.h.query_by_label("Previous Edge").is_none(), "nothing before the data start");
    let v = r.view();
    r.h.get_by_label("Next Edge").click();
    r.h.run();
    let after = r.view();
    assert!(after.left_time > v.left_time, "moved to a later edge");
    // The edge sits at 25 % of the width: D1 has a transition there.
    let s = r.h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let ch = cap.channel(oxrok_ui::core::ChannelId::digital(1)).unwrap();
    let quarter = after.px_to_time(after.width_px * 0.25).to_sample_floor(RATE) as u64;
    assert!(ch.next_transition(quarter.saturating_sub(1)).is_some_and(|e| e.abs_diff(quarter) <= 1));
    drop(cap);

    // `.` (typed) centers the next edge after the center.
    let center = r.view().center_time().to_sample_floor(RATE) as u64;
    let expected = ch_next(&r, 1, center);
    r.h.hover_at(at);
    r.h.event(egui::Event::Text(".".into()));
    r.h.run();
    let c = r.view().center_time().to_sample_floor(RATE) as u64;
    assert!(c.abs_diff(expected) <= 1, "{c} vs {expected}");
}

fn ch_next(r: &Rig, d: u16, from: u64) -> u64 {
    let s = r.h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    cap.channel(oxrok_ui::core::ChannelId::digital(d)).unwrap().next_transition(from).unwrap()
}

/// Shift+→ pans a whole page, `→` a third.
#[test]
fn shift_arrow_pans_a_page() {
    let mut r = Rig::with_capture(0.0, 1);
    for _ in 0..4 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.key_press(Key::Home);
    r.h.run();
    let v = r.view();
    r.h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    r.h.run();
    let moved = r.view().left_time.as_secs_f64() - v.left_time.as_secs_f64();
    let width = v.time_scale();
    assert!((moved - width).abs() < width * 0.01, "{moved} vs {width}");
}

/// Alt+2 opens the Inspector on the Analyzers tab; Alt+4
/// switches to the Markers tab; again it stays open on that tab.
#[test]
fn alt_number_opens_panels() {
    let mut r = Rig::with_capture(0.0, 1);
    let tab = |r: &Rig| r.h.state().settings().inspector.tab.clone();
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num2);
    r.h.run();
    assert_eq!(tab(&r), "analyzers");
    assert!(r.h.state().settings().inspector.open);
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num4);
    r.h.run();
    assert_eq!(tab(&r), "markers");
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num4);
    r.h.run();
    assert!(r.h.state().settings().inspector.open, "the tab key opens, it doesn't toggle");
    assert_eq!(tab(&r), "markers");
}

/// PageDown and the wheel over the labels scroll the rows.
#[test]
fn rows_scroll_with_page_keys_and_label_wheel() {
    let mut r = Rig::with_capture(0.0, 1);
    // The row label in the label column (the device panel has a D0 chip too);
    // rows scrolled out of the view aren't drawn.
    let row_top = |r: &Rig, name: &str| {
        r.h.get_all_by_label(name)
            .map(|n| n.rect())
            .find(|rc| (rc.left() >= INSPECTOR_W) && (rc.left() < INSPECTOR_W + LABEL_W))
            .map(|rc| rc.top())
    };
    // A short window, so the rows overflow.
    r.h.set_size(vec2(1280.0, 300.0));
    r.h.run_steps(3);
    let before = row_top(&r, "D0").expect("D0 visible at the top");
    r.h.key_press(Key::PageDown);
    r.h.run_steps(3);
    assert!(row_top(&r, "D0").is_none_or(|t| t < before), "PageDown scrolls down");
    r.h.key_press(Key::PageUp);
    r.h.run_steps(3);
    assert_eq!(row_top(&r, "D0"), Some(before));
    // The wheel over the label column.
    let at = pos2(INSPECTOR_W + LABEL_W / 2.0, before + 40.0);
    r.h.hover_at(at);
    r.h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -120.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    });
    r.h.run_steps(3);
    assert!(row_top(&r, "D0").is_none_or(|t| t < before), "the wheel over the labels scrolls");
    // The view didn't zoom.
}
