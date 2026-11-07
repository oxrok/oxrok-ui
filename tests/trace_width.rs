#![cfg(feature = "mock")]
//! The Trace Width setting, from the row context menu.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::ManualClock;
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::ui::waveform::TraceWidth;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn trace_width_from_the_row_menu() {
    let clock = ManualClock::new();
    let c = clock.clone();
    let mut h = Harness::builder()
        .with_size([1000.0, 500.0])
        .with_pixels_per_point(2.0)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
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
    let t0 = Instant::now();
    while !h.state().session().is_some_and(|s| s.caps.is_some()) {
        assert!(t0.elapsed() < Duration::from_secs(10));
        h.step();
    }
    {
        let s = &mut h.state_mut().session_mut().unwrap().settings;
        s.mode = oxrok_ui::core::capture::CaptureMode::Timed;
        s.timer_seconds = 0.01;
    }
    h.key_press(Key::Space);
    let t0 = Instant::now();
    while h.state().session().unwrap().state != CaptureState::Finished {
        assert!(t0.elapsed() < Duration::from_secs(10));
        clock.advance(Duration::from_millis(5));
        h.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    h.run_steps(3);
    assert_eq!(h.state().settings().waveform.trace_width, TraceWidth::Normal, "the default");
    let shots = std::env::var_os("LA_SHOT");
    for w in [TraceWidth::Thin, TraceWidth::Bold, TraceWidth::Normal] {
        // Right-click the D0 row label.
        let at = h.get_by_role_and_label(egui::Role::Label, "D0").rect().center();
        h.hover_at(at);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(4);
        h.get_by_label("Appearance").click();
        h.run_steps(4);
        h.get_by_label("Trace Width").click();
        h.run_steps(4);
        h.get_by_label(w.label()).click();
        h.run_steps(3);
        assert_eq!(h.state().settings().waveform.trace_width, w);
        if let Some(dir) = &shots {
            h.key_press(Key::Escape);
            h.remove_cursor();
            h.run_steps(3);
            h.render().unwrap().save(std::path::Path::new(dir).join(format!("trace_{}.png", w.label()))).unwrap();
        }
    }
}
