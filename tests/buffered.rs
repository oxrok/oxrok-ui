#![cfg(feature = "mock")]
//! Buffered devices (), with the
//! mock simulating a 2048-sample device memory.

use egui::Key;
use egui::accesskit::Toggled;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::ManualClock;
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEPTH: u64 = 2048;
const RATE: u64 = 8_000_000;

fn rig() -> (Harness<'static, LaApp>, ManualClock) {
    rig_with(None)
}

fn rig_with(rle_max_rate: Option<u64>) -> (Harness<'static, LaApp>, ManualClock) {
    let clock = ManualClock::new();
    let c = clock.clone();
    let h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        let options = MockOptions {
            streaming: Streaming::RealTime(Arc::new(c.clone())),
            buffered: Some(DEPTH),
            rle_max_rate,
            ..MockOptions::default()
        };
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
    (h, clock)
}

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    // Not `run`: the UI keeps repainting while recording.
    h.run_steps(3);
}

fn open(h: &mut Harness<'_, LaApp>) {
    h.run();
    h.get_by_label("Mock 8 digital").click();
    until(h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    // The Inspector starts closed: open it on the Device tab.
    h.key_press_modifiers(egui::Modifiers::ALT, Key::Num1);
    h.run_steps(3);
}

fn samples(h: &Harness<'_, LaApp>) -> u64 {
    let s = h.state().session().unwrap();
    s.capture.as_ref().and_then(|c| c.read().sample_range()).map_or(0, |(b, e)| e - b)
}

/// Records one Timed capture (default 1 s) and waits for the device to end it.
fn record(h: &mut Harness<'_, LaApp>, clock: &ManualClock) {
    h.key_press(Key::Space);
    until(h, "recording", |a| a.session().unwrap().state == CaptureState::Recording);
    clock.advance(Duration::from_secs(2));
    until(h, "finished by the device", |a| a.session().unwrap().state == CaptureState::Finished);
}

#[test]
fn buffered_device_defaults_to_timer_with_compression() {
    let (mut h, _) = rig();
    open(&mut h);
    assert_eq!(h.state().session().unwrap().settings.mode, CaptureMode::Timed);
    h.get_by_role_and_label(egui::Role::Button, "Capture Mode").click();
    h.run_steps(3);
    let looping = h.get_by_role_and_label(egui::Role::MenuItemCheckBox, "Continuous");
    assert!(looping.accesskit_node().is_disabled(), "Continuous needs a streaming device");
    let rle = h.get_by_role_and_label(egui::Role::Switch, "Compression (RLE)");
    assert_eq!(rle.accesskit_node().toggled(), Some(Toggled::True), "on by default");
}

#[test]
fn without_compression_the_memory_fills_and_the_user_is_told() {
    let (mut h, clock) = rig();
    open(&mut h);
    h.get_by_role_and_label(egui::Role::Switch, "Compression (RLE)").click();
    h.run();
    assert!(!h.state().session().unwrap().settings.rle);
    record(&mut h, &clock);
    assert_eq!(samples(&h), DEPTH);
    let t = h.state().toasts().iter().find(|t| t.heading.as_deref() == Some("Capture ended early")).expect(" toast");
    assert!(t.body.contains("2,048 samples") && t.body.contains("Compression (RLE) was off."), "{}", t.body);
}

#[test]
fn with_compression_the_whole_duration_is_recorded() {
    let (mut h, clock) = rig();
    open(&mut h);
    record(&mut h, &clock);
    assert_eq!(samples(&h), RATE, "1 s at 8 MS/s");
    assert!(h.state().toasts().iter().all(|t| t.heading.as_deref() != Some("Capture ended early")));
}

/// A device may round the sample rate asked for. The capture is sized
/// for the rate it took, so the timer is the time recorded.
#[test]
fn the_timer_counts_at_the_rate_the_device_took() {
    let (mut h, clock) = rig();
    open(&mut h);
    // The mock has 4 and 8 MS/s, and takes 7 MS/s as 8.
    h.state_mut().session_mut().unwrap().settings.sample_rate = 7_000_000;
    record(&mut h, &clock);
    assert!(h.state().toasts().iter().any(|t| t.body.starts_with("Sample rate adjusted to")));
    assert_eq!(samples(&h), RATE, "1 s at 8 MS/s");
    assert!(h.state().toasts().iter().all(|t| t.heading.as_deref() != Some("Capture ended early")));
}

#[test]
fn streaming_devices_keep_looping_and_have_no_switch() {
    let clock = ManualClock::new();
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::new_for_test(
            cc,
            MockBackend::new(MockOptions { streaming: Streaming::RealTime(Arc::new(clock)), ..MockOptions::default() }),
        )
    });
    open(&mut h);
    assert_eq!(h.state().session().unwrap().settings.mode, CaptureMode::Continuous);
    h.get_by_role_and_label(egui::Role::Button, "Capture Mode").click();
    h.run_steps(3);
    assert!(!h.get_by_role_and_label(egui::Role::MenuItemCheckBox, "Continuous").accesskit_node().is_disabled());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_role_and_label(egui::Role::Switch, "Compression (RLE)").is_none());
}

#[test]
fn a_refused_compression_start_hints_at_the_switch() {
    let (mut h, _) = rig_with(Some(RATE / 2));
    open(&mut h);
    h.key_press(Key::Space);
    until(&mut h, "start failed", |a| {
        a.toasts().iter().any(|t| t.heading.as_deref() == Some("Could not begin the capture."))
    });
    let t = h.state().toasts().iter().find(|t| t.heading.as_deref() == Some("Could not begin the capture.")).unwrap();
    assert!(t.body.contains("invalid argument") && t.body.contains("Try disabling Compression (RLE)"), "{}", t.body);
}
