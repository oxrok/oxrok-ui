//! End to end through oxrok: open the `demo` driver from the start
//! screen, record ~0.3 s in Continuous mode, stop.
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
        assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn record_from_oxrok_demo() {
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
            },
        )
    });
    h.run();
    let demo = h.state().devices().unwrap().iter().find(|d| d.driver == "demo").expect("demo device").display_name();
    h.get_by_label(&demo).click();
    until(&mut h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
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
