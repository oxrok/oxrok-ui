#![cfg(feature = "mock")]
//! Trigger mode, session reset and the capture button's badge.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::core::markers::TimeOrigin;
use oxrok_ui::core::trigger::Match;
use oxrok_ui::core::{CaptureTime, ChannelId, ManualClock};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Picks a capture mode from the toolbar dropdown.
fn pick_mode(h: &mut Harness<'_, LaApp>, mode: &str) {
    h.get_by_role_and_label(egui::Role::Button, "Capture Mode").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::Role::MenuItemCheckBox, mode).click();
    h.run_steps(3);
}

const RATE: u64 = 8_000_000;
const D0: ChannelId = ChannelId::digital(0);

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

impl Rig {
    fn new(opts: MockOptions) -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..opts.clone() };
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
        let mut r = Rig { h, clock };
        r.h.run();
        r.h.get_by_label("Mock 16 digital + 4 analog").click();
        r.until(|a| a.session().is_some_and(|s| s.caps.is_some()));
        // The Inspector starts closed: open it on the Device tab.
        r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
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

    fn state(&self) -> CaptureState {
        self.h.state().session().unwrap().state
    }

    /// Records until the capture ends by itself, advancing the clock.
    fn record_until_done(&mut self) {
        self.h.key_press(Key::Space);
        self.until(|a| a.session().unwrap().state == CaptureState::Recording);
        let t0 = Instant::now();
        while self.state() != CaptureState::Finished {
            assert!(t0.elapsed() < Duration::from_secs(30), "never stopped");
            self.clock.advance(Duration::from_millis(20));
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(3);
    }

    /// Scrolls a control of the Device Settings panel into view and clicks it.
    fn click_in_panel(&mut self, role: egui::Role, label: &str) {
        self.h.get_by_role_and_label(role, label).scroll_to_me();
        self.h.run_steps(3);
        self.h.get_by_role_and_label(role, label).click();
        self.h.run_steps(2);
    }

    fn type_time(&mut self, label: &str, text: &str) {
        self.click_in_panel(egui::Role::TextInput, label);
        self.h.run_steps(2);
        self.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        self.h.event(Event::Text(text.into()));
        self.h.key_press(Key::Enter);
        self.h.run_steps(3);
    }
}

/// Trigger on D0's rising edge, post 0.1 s: T0 is the trigger, the
/// T flag is drawn and the capture stops by itself.
#[test]
fn trigger_mode() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(4);
    let s = r.h.state().session().unwrap();
    assert_eq!(s.settings.mode, CaptureMode::Triggered);
    assert_eq!(s.settings.trigger.get(0, D0), Some(Match::Rising), "the lowest enabled channel");
    r.type_time("Capture duration after trigger", "100ms");
    assert!((r.h.state().session().unwrap().settings.trigger.post_trigger_seconds - 0.1).abs() < 1e-12);
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    // The mock's D0 starts high and falls at 0.5 ms: the first rise is at 1 ms.
    let t = s.trigger.expect("trigger found");
    assert_eq!(t, CaptureTime::from_sample(RATE / 1000, RATE));
    assert_eq!(s.origin, TimeOrigin::Trigger);
    assert_eq!(s.t0(), t, "the ruler counts from the trigger");
    let (_, end) = s.capture.as_ref().unwrap().read().sample_range().unwrap();
    let post = end - RATE / 1000;
    assert!((RATE / 10..RATE / 10 + RATE / 5).contains(&post), "stopped about 0.1 s after: {post}");
    let s = r.h.state().session().unwrap();
    // The flag: centered after the capture.
    r.h.get_by_role_and_label(egui::Role::Button, "Trigger");
    let center = s.view.state().center_time();
    assert!((center.secs_since(t)).abs() < 1e-9, "centered on the trigger");
}

/// An invalid trigger shows the badge's reason and refuses to start.
#[test]
fn invalid_trigger_badge_and_toast() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    {
        let s = r.h.state_mut().session_mut().unwrap();
        s.settings.trigger = oxrok_ui::core::trigger::DigitalTrigger::pulse(D0, true, 0.5, 0.1);
    }
    r.h.run_steps(2);
    let why = "No whole number of samples fits the duration at this sample rate.";
    assert_eq!(r.h.state().capture_blocked().as_deref(), Some(why));
    r.h.key_press(Key::Space);
    r.h.run_steps(3);
    assert_eq!(r.state(), CaptureState::Empty);
    assert!(r.h.state().toasts().iter().any(|t| t.body == why));
    // Nothing to wait for is no trigger either.
    r.h.state_mut().session_mut().unwrap().settings.trigger = Default::default();
    r.h.run_steps(2);
    assert_eq!(r.h.state().capture_blocked().as_deref(), Some("Set a trigger condition on a channel first."));
}

/// Starting again keeps analyzers and names, clears the markers.
#[test]
fn session_reset_keeps_configuration() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Timed");
    r.h.run_steps(2);
    r.type_time("Stop recording after", "100ms");
    r.record_until_done();
    {
        let s = r.h.state_mut().session_mut().unwrap();
        s.rename_row(D0, "CLK");
        s.markers.add_single(CaptureTime::from_secs_f64(0.01));
    }
    r.h.get_by_role_and_label(egui::Role::Button, "Analyzers").click();
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    r.h.run_steps(10);
    r.h.get_by_label("RX channel").click();
    r.h.run_steps(3);
    r.h.get_by_label("01.  D1").click();
    r.h.run_steps(3);
    r.h.get_by_label("Save").click();
    r.h.run_steps(5);
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    assert_eq!(s.analyzers.len(), 1, "analyzers kept");
    assert_eq!(s.row(D0).unwrap().name, "CLK", "names kept");
    assert!(s.markers.items.is_empty(), "markers cleared");
}

/// A start failure toasts and returns to Empty.
#[test]
fn start_failure() {
    let mut r = Rig::new(MockOptions { fail_start: true, ..MockOptions::default() });
    r.h.key_press(Key::Space);
    r.until(|a| a.toasts().iter().any(|t| t.heading.as_deref() == Some("Could not begin the capture.")));
    assert_eq!(r.state(), CaptureState::Empty);
}

/// Buffered devices get "Capture duration before trigger";
/// streaming devices don't (they keep everything before the trigger).
#[test]
fn pre_trigger_duration_on_buffered_devices() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    assert!(r.h.query_by_label("Capture duration before trigger").is_none());

    let mut r = Rig::new(MockOptions { buffered: Some(1 << 30), ..MockOptions::default() });
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    r.type_time("Capture duration before trigger", "20ms");
    r.type_time("Capture duration after trigger", "80ms");
    let t = &r.h.state().session().unwrap().settings.trigger;
    assert!((t.pre_trigger_seconds - 0.02).abs() < 1e-12);
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    // The mock has no hardware trigger, so its buffer starts at once; the
    // software trigger (1 ms) plus 80 ms ends it before the 100 ms limit.
    let (b, e) = s.capture.as_ref().unwrap().read().sample_range().unwrap();
    assert_eq!(s.trigger, Some(CaptureTime::from_sample(RATE / 1000, RATE)));
    assert!((RATE * 81 / 1000..=RATE / 10).contains(&(e - b)), "{} samples", e - b);
}

/// Devices that record into their own memory show no progress
/// card; the toolbar status gives the progress.
#[test]
fn no_progress_card_on_buffered_devices() {
    let mut r = Rig::new(MockOptions { buffered: Some(1 << 30), ..MockOptions::default() });
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    r.h.key_press(Key::Space);
    r.until(|a| a.session().unwrap().state == CaptureState::Recording);
    r.h.run_steps(3);
    assert!(r.h.query_by_label("Memory").is_none(), "no progress card");
    let status = r.h.query_by_label_contains("Waiting").is_some() || r.h.query_by_label_contains("Triggered").is_some();
    assert!(status, "the toolbar status");
}
