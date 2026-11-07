//! How long a long RLE capture takes to appear in the app: 200 MS/s for
//! 5 s (10⁹ samples) on CLK1, Timer mode. Run on demand:
//! `make test-hil ARGS="--ignored app_rle"`.
//!
//! Its own test binary: one libsigrok context per process.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::core::MonotonicClock;
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::hil::bench::DRIVER;
use oxrok_ui::hil::ch::CLK1;
use oxrok_ui::oxrok::OxrokBackend;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
#[ignore = "takes ~10 s of bench time"]
fn app_rle_200msps_5s() {
    if !oxrok_ui::hil::enabled() {
        return;
    }
    let backend = OxrokBackend::with_drivers(Some(vec![DRIVER.into()]));
    let mut h = Harness::builder().with_size([1280.0, 1000.0]).build_eframe(move |cc| {
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
    h.get_by_role_and_label(egui::Role::Button, "Intronix LogicPort").click();
    let t0 = Instant::now();
    while h.state().session().is_none_or(|s| s.caps.is_none()) {
        assert!(t0.elapsed() < Duration::from_secs(30));
        h.step();
    }
    let rate = 200_000_000u64;
    {
        let s = h.state_mut().session_mut().unwrap();
        s.settings.sample_rate = rate;
        s.settings.mode = CaptureMode::Timed;
        s.settings.timer_seconds = 5.0;
        s.settings.rle = true;
        s.settings.enabled = vec![CLK1];
    }
    h.key_press(Key::Space);
    let t0 = Instant::now();
    let mut first_data = None;
    loop {
        // Frames as a real UI would draw them while data arrives.
        h.step();
        std::thread::sleep(Duration::from_millis(16));
        let s = h.state().session().unwrap();
        let n = s.capture.as_ref().and_then(|c| c.read().sample_range()).map_or(0, |(b, e)| e - b);
        if n > 0 && first_data.is_none() {
            first_data = Some(t0.elapsed());
        }
        if s.state == CaptureState::Finished {
            eprintln!(
                "{n} samples: first data after {:.1} s, finished after {:.1} s",
                first_data.unwrap_or_default().as_secs_f64(),
                t0.elapsed().as_secs_f64()
            );
            assert!(n as f64 >= 5.0 * rate as f64 * 0.99, "{n}");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "timed out");
    }
}
