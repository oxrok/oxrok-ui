#![cfg(feature = "mock")]
//! Exit criteria: vertical, theme snapshots (subset), plus Y Fit and analog CSV export.

use egui::{Event, Key, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{ChannelId, ManualClock};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

fn rig(export_dir: Option<PathBuf>) -> Rig {
    let clock = ManualClock::new();
    let c = clock.clone();
    let h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(MockBackend::new(options)),
                clock: Arc::new(c),
                dialogs: Arc::new(FixedDialogs(export_dir)),
                settings_path: None,
                animation_speed: 0.0,
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
            if t0.elapsed() > Duration::from_secs(30) {
                let toasts: Vec<String> =
                    self.h.state().toasts().iter().map(|t| format!("{:?}: {}", t.heading, t.body)).collect();
                panic!("timed out waiting for {what}; toasts: {toasts:?}");
            }
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.step();
    }

    fn open_demo(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    }

    fn samples(&self) -> u64 {
        let s = self.h.state().session().unwrap();
        s.capture.as_ref().and_then(|c| c.read().sample_range()).map_or(0, |(b, e)| e - b)
    }

    fn capture(&mut self, secs: u64) {
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.clock.advance(Duration::from_secs(secs));
        self.until("data", move |a| {
            a.session()
                .unwrap()
                .capture
                .as_ref()
                .and_then(|c| c.read().sample_range())
                .is_some_and(|r| r.1 >= secs * RATE)
        });
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
        self.h.key_press(Key::F);
        self.h.run();
    }

    /// Center of a row's plot cell, by row label.
    fn row_point(&self, label: &str) -> Pos2 {
        let l = self.h.get_by_role_and_label(egui::Role::Label, label).rect();
        Pos2::new(l.right() + 300.0, l.center().y)
    }

    fn analog_view(&self, id: ChannelId) -> oxrok_ui::core::analog::AnalogView {
        self.h.state().session().unwrap().row(id).unwrap().analog.unwrap()
    }
}

#[test]
fn timer_mode_stops_by_itself() {
    let mut r = rig(None);
    r.open_demo();
    // The Inspector starts closed: open it on the Device tab.
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::RadioButton, "Timed").click();
    r.h.run();
    // Type 0.5 s into the timer field (default 1 s).
    r.h.get_by_role_and_label(egui::Role::TextInput, "Stop recording after").click();
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.event(Event::Text("500ms".into()));
    r.h.key_press(Key::Enter);
    r.h.run();
    assert_eq!(r.h.state().session().unwrap().settings.timer_seconds, 0.5);

    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
    r.h.get_by_label("Recording...");
    r.h.get_by_label("0:01");
    r.h.get_by_label("Memory");

    // The capture ends on its own once 0.5 s of data has been recorded.
    r.clock.advance(Duration::from_secs(1));
    r.until("timer stop", |a| a.session().unwrap().state == CaptureState::Finished);
    assert_eq!(r.samples(), RATE / 2);
    assert!(r.h.query_by_label("Recording...").is_none());
}

#[test]
fn vertical_zoom_about_pointer() {
    let mut r = rig(None);
    r.open_demo();
    r.capture(1);
    let p = r.row_point("A0") + vec2(0.0, -20.0);
    r.h.hover_at(p);
    r.h.run();
    let before = r.analog_view(ChannelId::analog(0));
    let inner_top = r.h.get_by_role_and_label(egui::Role::Label, "A0").rect().top();
    let inner_h = r.h.state().session().unwrap().row(ChannelId::analog(0)).unwrap().height - 2.0;
    let y = (p.y - inner_top) as f64;
    let v = before.y_to_value(y, inner_h as f64);
    r.h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::COMMAND,
        source: MouseWheelSource::Wheel,
    });
    r.h.run();
    let after = r.analog_view(ChannelId::analog(0));
    assert!(after.units_per_px < before.units_per_px, "zoomed in vertically");
    assert!((after.value_to_y(v, inner_h as f64) - y).abs() < 0.01, "value under the pointer stays put");
    // The time axis didn't change.
    let s = r.h.state().session().unwrap();
    assert!((s.view.state().time_scale() - 1.0).abs() < 1e-9);
}

#[test]
fn y_fit_fits_visible_data() {
    let mut r = rig(None);
    r.open_demo();
    r.capture(1);
    r.h.get_all_by_label("Y Fit").next().unwrap().click();
    r.h.run();
    let v = r.analog_view(ChannelId::analog(0));
    let inner_h = r.h.state().session().unwrap().row(ChannelId::analog(0)).unwrap().height as f64 - 2.0;
    let (lo, hi) = v.visible_range(inner_h);
    // A0 is ±2.5 V; fit adds 10 % of the span top and bottom.
    assert!((hi - 3.0).abs() < 0.01 && (lo + 3.0).abs() < 0.01, "{lo}..{hi}");
}

#[test]
fn digital_hover_measurement() {
    let mut r = rig(None);
    r.open_demo();
    r.capture(1);
    // Zoom in so the clock's pulses are wide.
    for _ in 0..9 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.hover_at(r.row_point("D0"));
    r.h.run();
    r.h.get_by_label("Duty: 50.00 %");
    r.h.get_by_label("Freq: 1 kHz");
    r.h.snapshot("d0_hover");
}

/// Nothing behind a dialog or the palette reacts to the
/// pointer; the hover measurement comes back once they close.
#[test]
fn no_hover_behind_dialogs() {
    let mut r = rig(None);
    r.open_demo();
    r.capture(1);
    for _ in 0..9 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    let at = r.row_point("D0");
    for (mods, key) in [(Modifiers::COMMAND, Key::Comma), (Modifiers::COMMAND, Key::K)] {
        r.h.key_press_modifiers(mods, key);
        r.h.run();
        r.h.hover_at(at);
        r.h.run();
        assert!(r.h.query_by_label_contains("Duty:").is_none(), "{key:?}: no hover measurement behind it");
        r.h.key_press(Key::Escape);
        r.h.run();
    }
    r.h.hover_at(at + vec2(1.0, 0.0));
    r.h.run();
    r.h.get_by_label("Duty: 50.00 %");
}

#[test]
fn analog_csv_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = rig(Some(dir.path().to_path_buf()));
    r.open_demo();
    r.capture(1);
    r.h.state_mut().settings_mut().last_dirs.export = Some(dir.path().to_path_buf());
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    r.h.run();
    r.h.get_by_label("✓ Export").click();
    r.until("export", |a| a.toasts().iter().any(|t| t.heading.as_deref() == Some("Export complete")));
    let path = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.to_string_lossy().ends_with(" analog.csv"))
        .expect("analog.csv written");
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("Time [s],A0,A1"));
    for (i, line) in lines.take(1000).enumerate() {
        let mut f = line.split(',');
        let t: f64 = f.next().unwrap().parse().unwrap();
        assert_eq!((t * RATE as f64).round() as u64, i as u64);
        for ch in 0..2u16 {
            let v: f32 = f.next().unwrap().parse().unwrap();
            let want = oxrok_ui::backend::mock::analog_value(ch, i as u64, RATE);
            assert!((v - want).abs() < 1e-4 * want.abs().max(1.0), "row {i} ch {ch}: {v} vs {want}");
        }
    }
    let digital = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.path().to_string_lossy().ends_with(" digital.csv"));
    assert!(digital, "the digital channels go to their own file");
}

/// Subset: start screen and a session with data, dark and light.
#[test]
fn theme_snapshots() {
    let mut results = egui_kittest::SnapshotResults::new();
    let mut r = rig(None);
    r.h.run();
    results.add(r.h.try_snapshot("theme_start_dark"));
    r.h.state_mut().set_theme(oxrok_ui::ui::theme::ThemeKind::Light);
    r.h.run();
    assert_eq!(r.h.state().theme(), oxrok_ui::ui::theme::ThemeKind::Light);
    results.add(r.h.try_snapshot("theme_start_light"));

    r.open_demo();
    r.capture(1);
    r.h.remove_cursor();
    r.h.run();
    results.add(r.h.try_snapshot("theme_session_light"));
    r.h.state_mut().set_theme(oxrok_ui::ui::theme::ThemeKind::Dark);
    r.h.run();
    results.add(r.h.try_snapshot("theme_session_dark"));
}
