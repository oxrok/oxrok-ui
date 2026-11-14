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
use oxrok_ui::core::trigger::{DigitalTrigger, Match};
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
    // The mock's D0 rises every 1 ms. The trigger is looked for after
    // the time to keep before it, 0.1 s: the rise at 100 ms.
    let t = s.trigger.expect("trigger found");
    assert_eq!(t, CaptureTime::from_sample(RATE / 10, RATE));
    assert_eq!(s.origin, TimeOrigin::Trigger);
    assert_eq!(s.t0(), t, "the ruler counts from the trigger");
    let (_, end) = s.capture.as_ref().unwrap().read().sample_range().unwrap();
    let post = end - RATE / 10;
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

/// A device may round the sample rate, and a pulse width may have no
/// whole sample in it at the rate it took: the capture does not begin
/// without its trigger, and the reason has the rate.
#[test]
fn a_trigger_the_applied_rate_cannot_take() {
    use oxrok_ui::backend::{HwTriggerCaps, TriggerMatch};
    // Two stages, which only the device's own trigger takes.
    let hw_trigger = Some(HwTriggerCaps {
        matches: vec![TriggerMatch::Rising, TriggerMatch::Falling],
        capture_ratio: true,
        pulse: true,
        duration: true,
        holdoff: true,
        ..HwTriggerCaps::default()
    });
    let mut r = Rig::new(MockOptions { hw_trigger, ..MockOptions::default() });
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    {
        let s = r.h.state_mut().session_mut().unwrap();
        // One sample at 3 MS/s, which the mock has not: it takes 2 or
        // 4 MS/s, and a third of a microsecond is no sample of either.
        s.settings.sample_rate = 3_000_000;
        let mut t = DigitalTrigger::pulse(D0, true, 1.0 / 3e6, 1.0 / 3e6);
        let [b, _] = DigitalTrigger::on(ChannelId::digital(1), Match::Rising).stages;
        t.stages[1] = b;
        s.settings.trigger = t;
    }
    r.h.run_steps(2);
    assert_eq!(r.h.state().capture_blocked(), None, "fine at the rate asked for");
    r.h.key_press(Key::Space);
    r.until(|a| a.toasts().iter().any(|t| t.heading.as_deref() == Some("Could not begin the capture.")));
    assert_eq!(r.state(), CaptureState::Empty);
    let toasts = r.h.state().toasts();
    let t = toasts.iter().find(|t| t.heading.as_deref() == Some("Could not begin the capture.")).unwrap();
    assert!(t.body.starts_with("No whole number of samples fits the duration at this sample rate."), "{}", t.body);
    assert!(t.body.contains("Sample rate adjusted to"), "{}", t.body);
    // Not the hints for what a device refuses.
    assert_eq!(t.body.lines().count(), 2, "{}", t.body);
}

/// "Capture duration before trigger" is kept before the trigger on a
/// streaming device and on a buffered one.
#[test]
fn pre_trigger_duration_on_buffered_devices() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    // A streaming device has it too: the trigger is looked for after it.
    assert!(r.h.query_all_by_label("Capture duration before trigger").next().is_some());

    let mut r = Rig::new(MockOptions { buffered: Some(1 << 30), ..MockOptions::default() });
    pick_mode(&mut r.h, "Trigger");
    r.h.run_steps(3);
    r.type_time("Capture duration before trigger", "20ms");
    r.type_time("Capture duration after trigger", "80ms");
    let t = &r.h.state().session().unwrap().settings.trigger;
    assert!((t.pre_trigger_seconds - 0.02).abs() < 1e-12);
    r.record_until_done();
    let s = r.h.state().session().unwrap();
    // The mock has no hardware trigger, so its buffer starts at once. The
    // app looks for the trigger after the 20 ms to keep before it: the
    // rise at 20 ms, and 80 ms more is the 100 ms limit.
    let (b, e) = s.capture.as_ref().unwrap().read().sample_range().unwrap();
    assert_eq!(s.trigger, Some(CaptureTime::from_sample(RATE / 50, RATE)));
    assert_eq!(e - b, RATE / 10, "20 ms before the trigger and 80 ms after");
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

/// A capture its analog channels may not fit the buffer for says so at
/// Start; one that fits does not.
#[test]
fn warns_when_the_buffer_is_small() {
    let mut r = Rig::new(MockOptions::default());
    pick_mode(&mut r.h, "Trigger");
    let warned = |r: &Rig| r.h.state().toasts().iter().any(|t| t.body.contains("may not fit the memory buffer"));
    // Two analog channels at 8 MS/s for 1.1 s: about 176 MB.
    r.h.state_mut().session_mut().unwrap().settings.buffer_mb = 256;
    r.h.key_press(Key::Space);
    r.until(|a| a.session().unwrap().state == CaptureState::Recording);
    assert!(!warned(&r));
    r.h.key_press(Key::Space);
    r.until(|a| !a.session().unwrap().state.is_busy());
    r.h.state_mut().session_mut().unwrap().settings.buffer_mb = 128;
    r.h.key_press(Key::Space);
    r.until(|a| a.session().unwrap().state == CaptureState::Recording);
    let toasts = r.h.state().toasts();
    let t = toasts.iter().find(|t| t.body.contains("may not fit the memory buffer")).expect("the warning");
    assert!(t.body.contains("about 0.17 GB, the buffer is 0.12 GB"), "{}", t.body);
}
