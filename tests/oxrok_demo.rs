//! End to end through oxrok: open the `demo` driver from the start
//! screen, record ~0.3 s in Continuous mode, stop; and Trigger mode with
//! the demo's own trigger.
//!
//! `cargo test -p la-app --features oxrok --test oxrok_demo`

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::core::MonotonicClock;
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::oxrok::OxrokBackend;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        if t0.elapsed() > Duration::from_secs(30) {
            let toasts: Vec<String> =
                h.state().toasts().iter().map(|t| format!("{:?}: {}", t.heading, t.body)).collect();
            let state = h.state().session().map(|s| s.state);
            panic!("timed out waiting for {what}; state {state:?}, toasts: {toasts:?}");
        }
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The app on the oxrok backend, with the demo device opened.
fn open_demo() -> Harness<'static, LaApp> {
    let backend = OxrokBackend::with_drivers(Some(vec!["demo".into()]));
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(backend),
                clock: Arc::new(MonotonicClock::new()),
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
    let demo = h.state().devices().unwrap().iter().find(|d| d.driver == "demo").expect("demo device").display_name();
    h.get_by_label(&demo).click();
    until(&mut h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    h
}

#[test]
fn record_from_oxrok_demo() {
    let mut h = open_demo();
    let rate = h.state().session().unwrap().settings.sample_rate;
    assert!(rate > 0 && rate <= 10_000_000, " default rate, got {rate}");

    h.key_press(Key::Space);
    until(&mut h, "recording", |a| a.session().unwrap().state == CaptureState::Recording);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(300) {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.key_press(Key::Space);
    until(&mut h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);

    let s = h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let (begin, end) = cap.sample_range().expect("data");
    assert!(end - begin > rate / 100, "at least 10 ms of samples ({} samples)", end - begin);
    let d0 = cap.channel(oxrok_ui::core::ChannelId::digital(0)).unwrap();
    assert!(d0.transition_count() > 0, "demo pattern toggles D0");
    // Analog through SR_DF_ANALOG (A0/A1 are enabled by default).
    let a0 = cap.analog(oxrok_ui::core::ChannelId::analog(0)).expect("A0 data");
    let (lo, hi) = a0.min_max(begin, end).unwrap();
    assert!(hi > lo, "demo A0 varies");
    eprintln!(
        "oxrok demo: {} samples at {rate} S/s, D0 {} transitions, A0 {lo:.2}…{hi:.2} V",
        end - begin,
        d0.transition_count()
    );
}

/// Trigger mode on a streaming device whose own trigger takes the app's:
/// the device waits for a pulse, and the capture is the stretch around
/// it. The demo's counter pattern has channel `k` high for `2^k` samples.
#[test]
fn pulse_trigger_waits_in_the_device() {
    use oxrok_ui::backend::{ConfValue, OptionScope};
    use oxrok_ui::core::ChannelId;
    use oxrok_ui::core::capture::CaptureMode;
    use oxrok_ui::core::trigger::TriggerEvent;
    let (d3, d5) = (ChannelId::digital(3), ChannelId::digital(5));
    let rate = 1_000_000;
    let mut h = open_demo();
    {
        let s = h.state_mut().session_mut().unwrap();
        assert!(s.caps.as_ref().unwrap().supports_continuous, "the demo streams");
        assert!(s.caps.as_ref().unwrap().hw_trigger.as_ref().is_some_and(|t| t.pulse), "with pulse widths");
        s.settings.sample_rate = rate;
        s.settings
            .device_options
            .entry(OptionScope::Group("Logic".into()))
            .or_default()
            .insert("pattern".into(), ConfValue::Str("incremental".into()));
        s.settings.mode = CaptureMode::Triggered;
        // The app starts with D0, D1 and the analog channels.
        s.settings.set_enabled(d3, true);
        s.settings.set_enabled(d5, true);
        let t = &mut s.settings.trigger;
        t.channel = Some(d3);
        t.event = TriggerEvent::PositivePulse;
        // D3's pulses are 8 us; none is 9 to 15 us wide.
        (t.pulse_min, t.pulse_max) = (9e-6, 15e-6);
        t.linked = vec![(d5, true)];
        t.pre_trigger_seconds = 0.01;
        t.post_trigger_seconds = 0.05;
    }
    h.run();
    let cli = h.state().session().unwrap().cli_command().one_line();
    assert!(cli.contains(r#"-t "D3=f>8<16,D5=1""#) && cli.contains("captureratio=17"), "{cli}");

    // No such pulse: the device keeps waiting, and Stop leaves nothing.
    h.key_press(Key::Space);
    until(&mut h, "recording", |a| a.session().unwrap().state == CaptureState::Recording);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(300) {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    {
        let s = h.state().session().unwrap();
        assert_eq!(s.state, CaptureState::Recording);
        assert!(!s.has_data(), "nothing arrives before the trigger");
    }
    h.key_press(Key::Space);
    until(&mut h, "stopped", |a| !a.session().unwrap().state.is_busy());
    assert_eq!(h.state().session().unwrap().state, CaptureState::Empty);

    // 6 to 10 us: the capture ends by itself, after the post-trigger time.
    {
        let t = &mut h.state_mut().session_mut().unwrap().settings.trigger;
        (t.pulse_min, t.pulse_max) = (6e-6, 10e-6);
    }
    h.key_press(Key::Space);
    until(&mut h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);
    let s = h.state().session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let t = cap.trigger.expect("the trigger");
    assert_eq!(cap.hw_trigger, Some(t), "the device's trigger sample");
    let (begin, end) = cap.sample_range().unwrap();
    assert!(end - t >= 50_000, "the post-trigger time: {} samples", end - t);
    // The stretch around the trigger, not everything since Start.
    assert!(end - begin < 100_000, "{begin}..{end}");
    let level = |ch, at| cap.channel(ch).unwrap().level_at(at).unwrap();
    assert!(t - begin > 9, "the pulse is in the data ({begin}, trigger at {t})");
    assert!(!level(d3, t) && !level(d3, t - 9), "D3 falls at the trigger, after a pulse");
    assert!((1..=8).all(|back| level(d3, t - back)), "8 samples wide");
    assert!(level(d5, t), "D5 high at the trigger");
}
