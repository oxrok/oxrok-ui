#![cfg(feature = "mock")]
//! Repeat: rounds, stopping a run.

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::animator::ViewRequest;
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::core::{CaptureTime, ManualClock, ViewState};
use oxrok_ui::decode::runner::Status;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

impl Rig {
    /// The 16 + 4 mock in Trigger mode on D0 rising, 10 ms post-trigger.
    fn new() -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
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
        let mut r = Rig { h, clock };
        r.h.run();
        r.h.get_by_label("Mock 16 digital + 4 analog").click();
        r.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
        // The Inspector starts closed: open it on the Device tab.
        r.h.key_press_modifiers(egui::Modifiers::ALT, Key::Num1);
        r.h.run_steps(3);
        r
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(2);
    }

    /// Advances the clock in `step_ms` steps until `cond`.
    fn run_until(&mut self, what: &str, step_ms: u64, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.clock.advance(Duration::from_millis(step_ms));
            self.h.step();

            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn repeat_button(&self) -> bool {
        !self.h.get_by_role_and_label(Role::Button, "Repeat").accesskit_node().is_disabled()
    }

    fn trigger_mode(&mut self) {
        self.h.get_by_role_and_label(Role::RadioButton, "Trigger").click();
        self.h.run_steps(3);
        self.h.state_mut().session_mut().unwrap().settings.trigger.post_trigger_seconds = 0.01;
        self.h.get_by_role_and_label(Role::Button, "Repeat").click();
        self.h.run_steps(3);
        assert!(self.h.state().session().unwrap().settings.repeat);
    }

    /// Advances the clock until `cond`, checking that the capture button
    /// says Stop on every frame.
    fn run_until_stop_shown(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.clock.advance(Duration::from_millis(2));
            self.h.step();
            let stop = self.h.query_by_role_and_label(Role::Button, "Stop");
            assert!(stop.is_some_and(|b| !b.accesskit_node().is_disabled()), "Stop shown and enabled ({what})");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// An Async Serial analyzer on D1, through the panel.
    fn add_uart(&mut self) {
        self.h.get_by_role_and_label(Role::Button, "Analyzers").click();
        self.h.run_steps(3);
        self.h.get_by_role_and_label(Role::MenuItem, "Async Serial").click();
        self.h.run_steps(10);
        self.h.get_by_label("RX channel").click();
        self.h.run_steps(3);
        self.h.get_by_label("01.  D1").click();
        self.h.run_steps(3);
        self.h.get_by_label("Save").click();
        self.h.run_steps(5);
        assert_eq!(self.h.state().session().unwrap().analyzers.len(), 1);
    }

    fn analyzer(&self) -> (Status, usize) {
        let a = &self.h.state().session().unwrap().analyzers[0];
        (a.out.status.lock().clone(), a.out.frames.read().order().len())
    }

    /// Steps frames with real time for the decoder thread, without
    /// advancing the capture clock.
    fn settle(&mut self, ms: u64) {
        for _ in 0..ms / 5 {
            self.h.step();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn round(&self) -> u32 {
        self.h.state().session().unwrap().repeat.as_ref().map_or(0, |r| r.round)
    }

    fn state(&self) -> CaptureState {
        self.h.state().session().unwrap().state
    }
}

/// Rounds follow each other; the status counts them; a marker and
/// the view stay across rounds; the capture button stays Stop; the toggle
/// is disabled in Continuous and Timed mode.
#[test]
fn repeat_rounds() {
    let mut r = Rig::new();
    assert_eq!(r.h.state().session().unwrap().settings.mode, CaptureMode::Continuous);
    assert!(!r.repeat_button(), "Continuous never ends by itself");
    r.h.get_by_role_and_label(Role::RadioButton, "Timed").click();
    r.h.run_steps(3);
    assert!(!r.repeat_button(), "Repeat is for Trigger mode");
    r.trigger_mode();
    assert!(r.repeat_button());

    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    assert_eq!(r.round(), 1);
    r.run_until_stop_shown("round 2", |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round == 2));
    r.run_until("round 2", 2, |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round == 2));
    // Some data, before round 2's trigger at 1 ms.
    r.clock.advance(Duration::from_micros(500));
    r.h.run_steps(3);
    r.h.get(By::new().label_contains("Waiting").label_contains("· #2"));
    // Round 1 is on screen while round 2 waits: it has its trigger.
    let t = CaptureTime::from_sample(RATE / 1000, RATE);
    r.h.state_mut().session_mut().unwrap().markers.add_single(t);
    // A view zoomed to 5 ms and panned away from the trigger stays there
    // (no re-centering).
    let left = CaptureTime::from_sample(RATE * 3 / 1000, RATE);
    {
        let now = r.h.state().now();
        let s = r.h.state_mut().session_mut().unwrap();
        let v = s.view.state();
        let tpp = 0.005 / v.width_px;
        s.view.request(ViewRequest::Immediate(ViewState { left_time: left, time_per_px: tpp, ..v }), now);
    }
    r.run_until("round 3", 2, |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round >= 3));
    let s = r.h.state().session().unwrap();
    assert_eq!(s.markers.items.len(), 1, "markers stay across rounds");
    assert_eq!(s.markers.items[0].span().0, t, "at the same time from the trigger");
    assert_eq!(s.view.state().left_time, left, "the view stays");
    assert!(s.state.is_busy());

    // Space ends the run.
    r.h.key_press(Key::Space);
    r.until("stopped", |a| a.session().unwrap().state == CaptureState::Finished);
    let s = r.h.state().session().unwrap();
    assert!(s.repeat.is_none());
    assert!(s.trigger.is_some(), "a complete round is kept");
}

/// Stopping while a round waits keeps the previous round; turning
/// Repeat off lets the current round finish and starts no other.
#[test]
fn stopping_a_run() {
    let mut r = Rig::new();
    r.trigger_mode();
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    // Round 2 has started but no time has passed: it waits for its trigger.
    r.run_until("round 2", 2, |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round == 2));
    let shown_end = |a: &LaApp| a.session().unwrap().capture.as_ref().unwrap().read().sample_range().unwrap().1;
    r.h.key_press(Key::Space);
    r.until("stopped", |a| a.session().unwrap().state == CaptureState::Finished);
    let s = r.h.state().session().unwrap();
    let t = s.trigger.expect("round 1's trigger");
    assert_eq!(t, CaptureTime::from_sample(RATE / 1000, RATE));
    let end = shown_end(r.h.state());
    assert!(end >= RATE / 1000 + RATE / 100, "round 1's data, through its post-trigger time: {end}");
    assert!(s.repeat.is_none());

    // Repeat off during round 2: it finishes, and no round 3 starts.
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    r.run_until("round 2", 2, |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round == 2));
    r.h.get_by_role_and_label(Role::Button, "Repeat").click();
    r.h.step();
    assert!(!r.h.state().session().unwrap().settings.repeat);
    r.run_until("round 2 done", 2, |a| a.session().unwrap().state == CaptureState::Finished);
    r.h.run_steps(5);
    assert_eq!(r.state(), CaptureState::Finished, "no round 3");
    assert!(r.h.state().session().unwrap().trigger.is_some());
}

/// The Stop button ends the run between rounds too.
#[test]
fn stop_between_rounds() {
    let mut r = Rig::new();
    r.trigger_mode();
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    // Round 1 ends by itself; while it stops, click Stop.
    r.run_until("round 1 stopping", 1, |a| a.session().unwrap().state == CaptureState::Stopping);
    r.h.get_by_role_and_label(Role::Button, "Stop").click();
    r.h.step();
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    r.h.run_steps(5);
    let s = r.h.state().session().unwrap();
    assert_eq!(s.state, CaptureState::Finished, "no round 2");
    assert!(s.repeat.is_none());
    assert!(s.trigger.is_some(), "round 1 is kept");
}

/// Pause Analyzers stops decoding and keeps the frames; resuming
/// completes it. *Pause analyzers during Repeat* decodes only the kept
/// round, once the run stops.
#[test]
fn pause_analyzers() {
    let mut r = Rig::new();
    r.trigger_mode();
    r.add_uart();
    r.h.get_by_role_and_label(Role::Button, "Repeat").click();
    r.h.run_steps(3);
    assert!(!r.h.state().session().unwrap().settings.repeat);
    // One Trigger capture, decoded while paused: nothing.
    r.h.get_by_role_and_label(Role::Button, "Pause analyzers").click();
    r.h.run_steps(2);
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    r.run_until("finished", 2, |a| a.session().unwrap().state == CaptureState::Finished);
    r.settle(100);
    assert_eq!(r.analyzer(), (Status::Idle, 0), "paused");
    r.h.get_by_role_and_label(Role::Button, "Pause analyzers").click();
    r.until("decoded", |a| *a.session().unwrap().analyzers[0].out.status.lock() == Status::Complete);
    assert!(r.analyzer().1 > 0, "frames once resumed");

    // The preference: a Repeat run decodes nothing until it stops.
    r.h.state_mut().settings_mut().capture.pause_analyzers_on_repeat = true;
    r.h.get_by_role_and_label(Role::Button, "Repeat").click();
    r.h.run_steps(3);
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state.is_busy());
    r.run_until("round 3", 2, |a| a.session().unwrap().repeat.as_ref().is_some_and(|x| x.round >= 3));
    r.settle(100);
    assert_eq!(r.analyzer(), (Status::Idle, 0), "paused during the run");
    r.h.get_by_role_and_label(Role::Button, "Stop").click();
    r.until("stopped", |a| a.session().unwrap().state == CaptureState::Finished);
    r.until("decoded", |a| *a.session().unwrap().analyzers[0].out.status.lock() == Status::Complete);
    assert!(r.analyzer().1 > 0, "the kept round is decoded");
}

/// A new session starts with Repeat off, though the device's
/// last settings and the session it copies had it on.
#[test]
fn new_sessions_start_without_repeat() {
    let mut r = Rig::new();
    r.trigger_mode();
    assert!(r.h.state().session().unwrap().settings.repeat);
    let first = r.h.state().session().unwrap().id;
    r.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::T);
    r.until("new session", |a| a.session().is_some_and(|s| s.id != first && s.caps.is_some()));
    let s = r.h.state().session().unwrap();
    assert_eq!(s.settings.mode, CaptureMode::Triggered, "the rest is copied");
    assert!(!s.settings.repeat);
}
